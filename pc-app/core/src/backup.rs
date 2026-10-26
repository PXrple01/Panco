//! Backups der Panco-Daten: Einstellungen, Wake-Word-Merkmale, Protokoll, Notizen (später auch
//! Gedächtnis, Makros, Spielzeit). Als normale Ordner – ohne Werkzeug wiederherstellbar.
//! Keine Sprachmodelle, kein Papierkorb, kein Audio.

use crate::files::{copy_recursive, within, Places};
use crate::time::to_local;
use std::path::{Path, PathBuf};

pub const PREFIX: &str = "Panco-Backup-";
/// Ein Backup pro Tag reicht.
pub const INTERVAL_SECS: i64 = 86_400;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct BackupInfo {
    pub name: String,
    pub path: PathBuf,
    /// Unix-Sekunden
    pub created: i64,
}

const README: &str = "Panco-Backup\r\n============\r\n\r\n\
Panco-Daten\\   Einstellungen (config.json), Wake-Word-Merkmale, Aktionsprotokoll\r\n\
Notizen\\       deine Notizen\r\n\r\n\
Wiederherstellen: Panco beenden, den Inhalt von \"Panco-Daten\" nach %APPDATA%\\Panco kopieren,\r\n\
die Notizen in deinen Notizen-Ordner. Danach Panco starten.\r\n\r\n\
Hinweis: Verschlüsselte Notizen (*.md.panco) lassen sich nur mit demselben Windows-Benutzerkonto\r\n\
öffnen, mit dem sie erstellt wurden (Windows-DPAPI).\r\n";

fn skip_in_data(name: &str) -> bool {
    matches!(name, "models") || name.ends_with(".part")
}

/// Legt ein neues Backup an und löscht alte über `keep` hinaus.
pub fn run(places: &Places, now: i64, offset: i32, keep: u32) -> std::io::Result<BackupInfo> {
    let d = to_local(now, offset);
    let base = format!("{PREFIX}{:04}-{:02}-{:02}_{:02}-{:02}", d.year, d.month, d.day, d.hour, d.minute);
    let mut dest = places.backup.join(&base);
    let mut n = 2;
    while dest.exists() {
        dest = places.backup.join(format!("{base}-{n}"));
        n += 1;
    }
    let tmp = dest.with_extension("tmp");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;

    // Panco-Daten (ohne Modelle; Papierkorb/Backups, falls sie darin liegen, auch nicht)
    if let Ok(rd) = std::fs::read_dir(&places.data) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let path = e.path();
            if skip_in_data(&name) || within(&path, &places.trash) || within(&path, &places.backup) {
                continue;
            }
            copy_recursive(&path, &tmp.join("Panco-Daten").join(&name))?;
        }
    }
    if places.notes.is_dir() && !within(&places.notes, &places.data) {
        copy_recursive(&places.notes, &tmp.join("Notizen"))?;
    }
    std::fs::write(tmp.join("LIESMICH.txt"), README)?;
    // Erst wenn alles kopiert ist, bekommt der Ordner seinen endgültigen Namen.
    std::fs::rename(&tmp, &dest)?;
    prune(&places.backup, keep.max(1));
    Ok(BackupInfo { name: dest.file_name().unwrap().to_string_lossy().into(), path: dest, created: now })
}

/// Alle Backups, neueste zuerst.
pub fn list(dir: &Path) -> Vec<BackupInfo> {
    let mut out: Vec<BackupInfo> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if !name.starts_with(PREFIX) || name.ends_with(".tmp") {
                        return None;
                    }
                    let created = e.metadata().ok()?.modified().map(crate::time::unix).unwrap_or(0);
                    Some(BackupInfo { name, path: e.path(), created })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

/// Entfernt die ältesten Backups (nur Ordner mit Panco-Präfix). Gibt die Anzahl zurück.
pub fn prune(dir: &Path, keep: u32) -> usize {
    list(dir).into_iter().skip(keep as usize).filter(|b| std::fs::remove_dir_all(&b.path).is_ok()).count()
}

/// Ist ein neues Backup fällig?
pub fn due(dir: &Path, now: i64) -> bool {
    list(dir).first().is_none_or(|b| now - b.created >= INTERVAL_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FileSettings;

    #[test]
    fn backup_copies_data_and_notes_and_prunes() {
        let root = std::env::temp_dir().join(format!("panco-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let s = |p: PathBuf| p.to_string_lossy().to_string();
        let files = FileSettings {
            allowed_folders: vec![],
            notes_dir: s(root.join("Notizen")),
            trash_dir: s(root.join("data/Papierkorb")),
            trash_days: 30,
            backup_enabled: true,
            backup_dir: s(root.join("Backups")),
            backup_keep: 2,
        };
        let places = Places::from_settings(&files, &root.join("data"));
        std::fs::create_dir_all(root.join("data/models")).unwrap();
        std::fs::create_dir_all(root.join("data/wakeword")).unwrap();
        std::fs::create_dir_all(root.join("data/Papierkorb/1")).unwrap();
        std::fs::create_dir_all(root.join("Notizen")).unwrap();
        std::fs::write(root.join("data/config.json"), "{}").unwrap();
        std::fs::write(root.join("data/models/ggml-base.bin"), "gross").unwrap();
        std::fs::write(root.join("data/wakeword/panco.rpw"), "x").unwrap();
        std::fs::write(root.join("Notizen/Einkauf.md"), "# Einkauf").unwrap();

        assert!(due(&places.backup, 1_000_000));
        let b = run(&places, 1_793_017_800, 3600, 2).unwrap();
        assert_eq!(b.name, "Panco-Backup-2026-10-26_13-30");
        assert!(b.path.join("Panco-Daten/config.json").is_file());
        assert!(b.path.join("Panco-Daten/wakeword/panco.rpw").is_file());
        assert!(b.path.join("Notizen/Einkauf.md").is_file());
        assert!(b.path.join("LIESMICH.txt").is_file());
        assert!(!b.path.join("Panco-Daten/models").exists(), "Modelle werden nicht gesichert");
        assert!(!b.path.join("Panco-Daten/Papierkorb").exists(), "Papierkorb wird nicht gesichert");
        assert!(!due(&places.backup, crate::time::unix_now()));

        // Drei weitere → nur die zwei neuesten bleiben
        for i in 1..=3 {
            run(&places, 1_793_017_800 + i * 86_400, 3600, 2).unwrap();
        }
        let names: Vec<String> = list(&places.backup).into_iter().map(|b| b.name).collect();
        assert_eq!(names, vec!["Panco-Backup-2026-10-29_13-30", "Panco-Backup-2026-10-28_13-30"]);
        std::fs::remove_dir_all(root).ok();
    }
}

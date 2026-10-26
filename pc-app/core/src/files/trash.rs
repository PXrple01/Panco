//! Panco-Papierkorb: Panco löscht nie endgültig. "Löschen" verschiebt hierher, mit Metadaten zum
//! Wiederherstellen. Erst nach Ablauf der Frist wird ein Eintrag wirklich entfernt.
//!
//! Aufbau: `<papierkorb>/<id>/meta.json` + `<papierkorb>/<id>/item/<Originalname>`

use super::{copy_recursive, free_name, move_path, size_of};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrashKind {
    /// Vom Nutzer "gelöscht".
    Deleted,
    /// Sicherungskopie vor dem Überschreiben/Bearbeiten.
    Snapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrashEntry {
    pub id: String,
    pub original: PathBuf,
    pub name: String,
    /// Unix-Sekunden
    pub deleted_at: i64,
    pub is_dir: bool,
    pub size: u64,
    pub kind: TrashKind,
}

pub struct Trash {
    dir: PathBuf,
}

static SEQ: AtomicU32 = AtomicU32::new(0);

impl Trash {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn new_id() -> String {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        format!("{nanos}-{:04}", SEQ.fetch_add(1, Ordering::Relaxed) % 10_000)
    }

    fn store(&self, path: &Path, kind: TrashKind, copy: bool) -> std::io::Result<TrashEntry> {
        let meta = std::fs::symlink_metadata(path)?;
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Element".into());
        let id = Self::new_id();
        let slot = self.dir.join(&id);
        let target = slot.join("item").join(&name);
        let entry = TrashEntry {
            id,
            original: path.to_path_buf(),
            name,
            deleted_at: crate::time::unix_now(),
            is_dir: meta.is_dir(),
            size: size_of(path),
            kind,
        };
        std::fs::create_dir_all(slot.join("item"))?;
        // Metadaten zuerst schreiben: so ist ein Eintrag immer zuordenbar.
        std::fs::write(slot.join("meta.json"), serde_json::to_vec_pretty(&entry).unwrap_or_default())?;
        let result = if copy { copy_recursive(path, &target) } else { move_path(path, &target) };
        if let Err(e) = result {
            let _ = std::fs::remove_dir_all(&slot);
            return Err(e);
        }
        Ok(entry)
    }

    /// Verschiebt Datei/Ordner in den Papierkorb.
    pub fn put(&self, path: &Path) -> std::io::Result<TrashEntry> {
        self.store(path, TrashKind::Deleted, false)
    }

    /// Legt eine Sicherungskopie ab (Original bleibt).
    pub fn snapshot(&self, path: &Path) -> std::io::Result<TrashEntry> {
        self.store(path, TrashKind::Snapshot, true)
    }

    pub fn get(&self, id: &str) -> Option<TrashEntry> {
        // Nur einfache IDs zulassen (kein Pfad-Trick über die ID).
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit() || c == '-') {
            return None;
        }
        let raw = std::fs::read(self.dir.join(id).join("meta.json")).ok()?;
        serde_json::from_slice(&raw).ok()
    }

    /// Stellt einen Eintrag am ursprünglichen Ort wieder her (bei Namenskonflikt mit Zusatz).
    /// Bei Sicherungskopien wird der aktuelle Stand vorher selbst in den Papierkorb gelegt.
    pub fn restore(&self, id: &str) -> std::io::Result<PathBuf> {
        let entry = self.get(id).ok_or_else(|| std::io::Error::other("Eintrag nicht gefunden"))?;
        let item = self.dir.join(id).join("item").join(&entry.name);
        let target = if entry.kind == TrashKind::Snapshot && entry.original.exists() {
            self.put(&entry.original)?;
            entry.original.clone()
        } else {
            free_name(&entry.original)
        };
        move_path(&item, &target)?;
        std::fs::remove_dir_all(self.dir.join(id))?;
        Ok(target)
    }

    /// Alle Einträge, neueste zuerst.
    pub fn list(&self) -> Vec<TrashEntry> {
        let mut out: Vec<TrashEntry> = std::fs::read_dir(&self.dir)
            .map(|rd| rd.flatten().filter_map(|e| self.get(&e.file_name().to_string_lossy())).collect())
            .unwrap_or_default();
        out.sort_by_key(|e| std::cmp::Reverse(e.deleted_at));
        out
    }

    /// Entfernt endgültig, was älter als die Frist ist. Gibt die Anzahl zurück.
    pub fn purge_older_than(&self, days: u32, now: i64) -> usize {
        let limit = now - days as i64 * 86_400;
        let mut n = 0;
        for e in self.list() {
            if e.deleted_at < limit && std::fs::remove_dir_all(self.dir.join(&e.id)).is_ok() {
                n += 1;
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("panco-trash-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn put_restore_and_conflict() {
        let root = tmp("a");
        let trash = Trash::new(root.join("trash"));
        let f = root.join("docs/Rechnung.pdf");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, b"pdf").unwrap();

        let e = trash.put(&f).unwrap();
        assert!(!f.exists());
        assert_eq!(trash.list().len(), 1);
        assert_eq!(e.size, 3);

        // Inzwischen gibt es eine neue Datei gleichen Namens → Wiederherstellen mit Zusatz
        std::fs::write(&f, b"neu").unwrap();
        let restored = trash.restore(&e.id).unwrap();
        assert_eq!(restored, root.join("docs/Rechnung (2).pdf"));
        assert_eq!(std::fs::read(&restored).unwrap(), b"pdf");
        assert!(trash.list().is_empty());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn folders_snapshots_and_purge() {
        let root = tmp("b");
        let trash = Trash::new(root.join("trash"));
        let dir = root.join("Projekt");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a.txt"), b"x").unwrap();
        let e = trash.put(&dir).unwrap();
        assert!(e.is_dir && !dir.exists());
        trash.restore(&e.id).unwrap();
        assert_eq!(std::fs::read(dir.join("sub/a.txt")).unwrap(), b"x");

        // Sicherungskopie: Original bleibt, Wiederherstellen ersetzt den neuen Stand
        let file = dir.join("sub/a.txt");
        let snap = trash.snapshot(&file).unwrap();
        std::fs::write(&file, b"geaendert").unwrap();
        trash.restore(&snap.id).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"x");
        assert_eq!(trash.list().len(), 1, "geänderter Stand liegt jetzt im Papierkorb");

        // Frist: nichts Neues wird entfernt, alles Alte schon
        assert_eq!(trash.purge_older_than(30, crate::time::unix_now()), 0);
        assert_eq!(trash.purge_older_than(30, crate::time::unix_now() + 31 * 86_400), 1);
        assert!(trash.list().is_empty());
        assert!(trash.get("../../etc").is_none());
        std::fs::remove_dir_all(root).ok();
    }
}

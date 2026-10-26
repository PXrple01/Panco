//! Dateizugriff mit Leitplanken: Zonen (freigegeben / sonstige / System / Panco-intern),
//! Panco-Papierkorb, Dateisuche. Alle Pfade werden normalisiert, bevor entschieden wird.

pub mod search;
pub mod trash;

use crate::catalog::Tier;
use crate::config::{expand_env, FileSettings};
use std::path::{Component, Path, PathBuf};

/// Wo liegt ein Pfad – und wie streng muss Panco sein?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// In einem freigegebenen Ordner.
    Allowed,
    /// Woanders auf der Platte.
    Other,
    /// Windows, Programme, Laufwerkswurzeln.
    System,
    /// Panco-Papierkorb, Backups, Panco-Daten – verwaltet Panco nur selbst.
    Protected,
}

/// Alle wichtigen Orte, aus der Konfig aufgelöst.
#[derive(Debug, Clone)]
pub struct Places {
    pub allowed: Vec<PathBuf>,
    pub notes: PathBuf,
    pub trash: PathBuf,
    pub backup: PathBuf,
    pub data: PathBuf,
    pub trash_days: u32,
}

impl Places {
    pub fn from_settings(f: &FileSettings, data: &Path) -> Self {
        let p = |s: &str| normalize(Path::new(&expand_env(s)));
        let mut allowed: Vec<PathBuf> =
            f.allowed_folders.iter().filter(|s| !s.trim().is_empty()).map(|s| p(s)).collect();
        let notes = p(&f.notes_dir);
        if !allowed.iter().any(|a| within(&notes, a)) {
            allowed.push(notes.clone());
        }
        Self {
            allowed,
            notes,
            trash: p(&f.trash_dir),
            backup: p(&f.backup_dir),
            data: normalize(data),
            trash_days: f.trash_days,
        }
    }

    pub fn zone(&self, path: &Path) -> Zone {
        let path = real(path);
        if [&self.trash, &self.backup, &self.data].iter().any(|p| within(&path, p)) {
            Zone::Protected
        } else if self.allowed.iter().any(|a| within(&path, a)) {
            Zone::Allowed
        } else if is_system(&path) {
            Zone::System
        } else {
            Zone::Other
        }
    }

    /// Stufe für eine schreibende Aktion an `path`; `min` ist die Mindeststufe der Aktion.
    pub fn write_tier(&self, path: &Path, min: Tier) -> Result<Tier, String> {
        match self.zone(path) {
            Zone::Allowed => Ok(min),
            Zone::Other => Ok(min.max(Tier::Confirm)),
            Zone::System => Ok(Tier::PcOnly),
            Zone::Protected => Err("Diesen Ordner verwaltet Panco selbst.".into()),
        }
    }

    /// Lesen/Öffnen: in freigegebenen Ordnern sofort, sonst mit Rückfrage.
    pub fn read_tier(&self, path: &Path) -> Tier {
        match self.zone(path) {
            Zone::Allowed => Tier::Instant,
            _ => Tier::Confirm,
        }
    }

    /// Sprechbare Ordnernamen ("Dokumente", "Downloads" …) → Pfad.
    pub fn known_folder(&self, name: &str) -> Option<PathBuf> {
        let n = crate::normalize::name(name);
        let n = n.trim_start_matches("meine ").trim_start_matches("den ").trim_start_matches("die ").trim();
        let dir = match n {
            "dokumente" | "dokument" | "documents" | "eigene dateien" => dirs::document_dir(),
            "downloads" | "download" => dirs::download_dir(),
            "desktop" | "schreibtisch" => dirs::desktop_dir(),
            "bilder" | "fotos" | "pictures" => dirs::picture_dir(),
            "musik" | "music" => dirs::audio_dir(),
            "videos" | "video" => dirs::video_dir(),
            "notizen" => Some(self.notes.clone()),
            _ => None,
        };
        dir.map(|d| normalize(&d))
    }
}

/// Lexikalisch normalisieren: `.` und `..` auflösen, ohne das Dateisystem zu fragen.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Echte Lage (Symlinks/Junctions auflösen), sonst lexikalisch.
fn real(p: &Path) -> PathBuf {
    // Für noch nicht existierende Ziele den nächsten existierenden Elternordner auflösen.
    let mut probe = normalize(p);
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(c) = std::fs::canonicalize(&probe) {
            let mut c = strip_verbatim(c);
            for r in rest.iter().rev() {
                c.push(r);
            }
            return c;
        }
        match (probe.file_name().map(|f| f.to_os_string()), probe.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                probe = parent.to_path_buf();
            }
            _ => return normalize(p),
        }
    }
}

/// Windows liefert `\\?\C:\…` – für Vergleiche ohne Präfix.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => p,
    }
}

fn key(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/');
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

/// Liegt `child` in `parent` (oder ist es)?
pub fn within(child: &Path, parent: &Path) -> bool {
    let (c, p) = (key(child), key(parent));
    !p.is_empty() && (c == p || c.starts_with(&format!("{p}/")))
}

fn is_system(p: &Path) -> bool {
    // Laufwerkswurzel selbst ("C:\") oder "/"
    if p.parent().is_none() || p.parent().is_some_and(|pp| pp.as_os_str().is_empty()) {
        return true;
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in ["WINDIR", "ProgramFiles", "ProgramFiles(x86)", "ProgramData", "ProgramW6432"] {
            if let Ok(v) = std::env::var(var) {
                roots.push(PathBuf::from(v));
            }
        }
        if let Some(local) = dirs::data_local_dir() {
            roots.push(local.join("Microsoft"));
        }
    } else {
        for r in ["/usr", "/etc", "/bin", "/sbin", "/boot", "/lib", "/lib64", "/sys", "/proc", "/dev", "/var"] {
            roots.push(PathBuf::from(r));
        }
    }
    roots.iter().any(|r| within(p, r))
}

/// Dateitypen, die beim Öffnen Code ausführen würden – öffnet Panco nie per Datei-Befehl.
const ACTIVE_EXT: [&str; 34] = [
    "exe",
    "com",
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "psd1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "ws",
    "msi",
    "msp",
    "mst",
    "lnk",
    "url",
    "website",
    "scr",
    "pif",
    "hta",
    "cpl",
    "jar",
    "reg",
    "inf",
    "application",
    "appref-ms",
    "gadget",
    "msc",
    "settingcontent-ms",
    "library-ms",
    "scf",
];

pub fn is_active_file(p: &Path) -> bool {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).is_some_and(|e| ACTIVE_EXT.contains(&e.as_str()))
}

/// Gültiger Datei-/Ordnername unter Windows (ohne Pfadtrenner, ohne reservierte Namen).
pub fn valid_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() || n.len() > 200 {
        return Err("Name leer oder zu lang".into());
    }
    if n.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control()) {
        return Err("Name enthält unerlaubte Zeichen".into());
    }
    if n.ends_with('.') || n == "." || n == ".." {
        return Err("Name darf nicht mit Punkt enden".into());
    }
    let stem = n.split('.').next().unwrap_or("").to_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"];
    let numbered = (1..=9).flat_map(|i| [format!("COM{i}"), format!("LPT{i}")]).any(|r| r == stem);
    if reserved.contains(&stem.as_str()) || numbered {
        return Err("Name ist unter Windows reserviert".into());
    }
    Ok(n.to_string())
}

/// Größe einer Datei oder eines Ordners (rekursiv, begrenzt).
pub fn size_of(p: &Path) -> u64 {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => {
            std::fs::read_dir(p).map(|rd| rd.flatten().take(10_000).map(|e| size_of(&e.path())).sum()).unwrap_or(0)
        }
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// Kopiert Dateien/Ordner rekursiv (für Papierkorb über Laufwerksgrenzen und Backups).
pub fn copy_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_recursive(&e.path(), &to.join(e.file_name()))?;
        }
    } else if meta.is_file() {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to)?;
    }
    // Symlinks werden bewusst nicht mitkopiert.
    Ok(())
}

/// Verschieben; über Laufwerksgrenzen als Kopieren + Entfernen des Originals.
pub fn move_path(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_recursive(from, to)?;
            if from.is_dir() {
                std::fs::remove_dir_all(from)
            } else {
                std::fs::remove_file(from)
            }
        }
    }
}

/// Freier Zielname: "Bericht.pdf" → "Bericht (2).pdf", falls schon vorhanden.
pub fn free_name(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..1000)
        .map(|i| path.with_file_name(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or_else(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places(root: &Path) -> Places {
        let s = |p: PathBuf| p.to_string_lossy().to_string();
        let f = FileSettings {
            allowed_folders: vec![s(root.join("docs"))],
            notes_dir: s(root.join("docs/Notizen")),
            trash_dir: s(root.join("trash")),
            trash_days: 30,
            backup_enabled: true,
            backup_dir: s(root.join("backup")),
            backup_keep: 3,
        };
        Places::from_settings(&f, &root.join("data"))
    }

    #[test]
    fn zones_and_traversal() {
        let root = std::env::temp_dir().join(format!("panco-zones-{}", std::process::id()));
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let pl = places(&root);
        assert_eq!(pl.zone(&root.join("docs/a.txt")), Zone::Allowed);
        assert_eq!(pl.zone(&root.join("docs/neu/ordner")), Zone::Allowed);
        assert_eq!(pl.zone(&root.join("other/a.txt")), Zone::Other);
        // ".." darf nicht aus dem freigegebenen Ordner herausführen
        assert_eq!(pl.zone(&root.join("docs/../other/a.txt")), Zone::Other);
        assert_eq!(pl.zone(&root.join("trash/x")), Zone::Protected);
        assert_eq!(pl.zone(&root.join("data/config.json")), Zone::Protected);
        assert_eq!(pl.write_tier(&root.join("docs/a"), Tier::Instant), Ok(Tier::Instant));
        assert_eq!(pl.write_tier(&root.join("other/a"), Tier::Instant), Ok(Tier::Confirm));
        assert!(pl.write_tier(&root.join("trash/a"), Tier::Instant).is_err());
        if cfg!(unix) {
            assert_eq!(pl.zone(Path::new("/etc/passwd")), Zone::System);
            assert_eq!(pl.write_tier(Path::new("/usr/bin/x"), Tier::Instant), Ok(Tier::PcOnly));
        }
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn active_files_and_names() {
        assert!(is_active_file(Path::new("C:/x/setup.EXE")));
        assert!(is_active_file(Path::new("a.lnk")));
        assert!(!is_active_file(Path::new("Rechnung.pdf")));
        assert!(valid_name("Steuer 2025").is_ok());
        assert!(valid_name("a/b").is_err());
        assert!(valid_name("CON").is_err());
        assert!(valid_name("com3.txt").is_err());
        assert!(valid_name("x.").is_err());
    }
}

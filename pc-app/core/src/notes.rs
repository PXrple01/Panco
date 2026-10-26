//! Notizen als Markdown-Dateien im Notizen-Ordner.
//! Sensible Titel (Passwörter, PIN, Zugangsdaten …) werden verschlüsselt gespeichert
//! (`<Titel>.md.panco`, DPAPI = an dein Windows-Konto gebunden) und nie vorgelesen.

use crate::files::trash::Trash;
use crate::platform::Platform;
use crate::undo::UndoOp;
use std::path::{Path, PathBuf};

const ENC_MAGIC: &[u8] = b"PANCO-DPAPI1\n";
const ENC_EXT: &str = ".md.panco";

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub title: String,
    pub path: PathBuf,
    pub sensitive: bool,
    /// Unix-Sekunden
    pub modified: i64,
}

/// Wörter, bei denen eine Notiz automatisch verschlüsselt wird.
const SENSITIVE: [&str; 22] = [
    "passwort",
    "passwoert",
    "password",
    "kennwort",
    "zugangsdaten",
    "zugang",
    "login",
    "pin",
    "tan",
    "puk",
    "geheim",
    "schluessel",
    "iban",
    "bank",
    "kreditkarte",
    "konto",
    "seed",
    "recovery",
    "2fa",
    "totp",
    "wlan",
    "secret",
];

pub fn is_sensitive_title(title: &str) -> bool {
    let n = crate::normalize::name(title);
    n.split(' ').any(|w| SENSITIVE.iter().any(|s| w == *s || (s.len() >= 4 && w.starts_with(s))))
}

/// Titel → Dateiname (ohne Endung); unerlaubte Zeichen werden ersetzt.
fn safe_stem(title: &str) -> String {
    let s: String =
        title
            .trim()
            .chars()
            .map(|c| {
                if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
                    '-'
                } else {
                    c
                }
            })
            .take(80)
            .collect();
    let s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() {
        "Notiz".into()
    } else {
        s
    }
}

fn note_from_path(path: PathBuf) -> Option<Note> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let (title, sensitive) = if let Some(t) = name.strip_suffix(ENC_EXT) {
        (t.to_string(), true)
    } else {
        let t = name.strip_suffix(".md")?;
        (t.to_string(), false)
    };
    let modified = std::fs::metadata(&path).ok()?.modified().map(crate::time::unix).unwrap_or(0);
    Some(Note { title, path, sensitive, modified })
}

/// Alle Notizen, zuletzt geändert zuerst.
pub fn list(dir: &Path) -> Vec<Note> {
    let mut notes: Vec<Note> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().filter_map(|e| note_from_path(e.path())).collect())
        .unwrap_or_default();
    notes.sort_by_key(|n| std::cmp::Reverse(n.modified));
    notes
}

/// Notiz per (ungefährem) Titel finden.
pub fn find(dir: &Path, title: &str) -> Option<Note> {
    let notes = list(dir);
    match crate::fuzzy::resolve(title, &notes, |n| vec![n.title.as_str()]) {
        crate::fuzzy::Resolved::Sure(n) | crate::fuzzy::Resolved::Unsure(n) => Some(n.clone()),
        crate::fuzzy::Resolved::None => None,
    }
}

fn encode(pf: &dyn Platform, text: &str, sensitive: bool) -> Result<Vec<u8>, String> {
    if !sensitive {
        return Ok(text.as_bytes().to_vec());
    }
    let blob = pf.protect(text.as_bytes()).map_err(|e| format!("Verschlüsselung nicht verfügbar ({e})"))?;
    let mut out = ENC_MAGIC.to_vec();
    out.extend(blob);
    Ok(out)
}

/// Vollständiger Inhalt (entschlüsselt). Nur für Panco intern und das Einstellungsfenster am PC.
pub fn read(pf: &dyn Platform, note: &Note) -> Result<String, String> {
    let raw = std::fs::read(&note.path).map_err(|e| e.to_string())?;
    if !note.sensitive {
        return Ok(String::from_utf8_lossy(&raw).to_string());
    }
    let blob = raw.strip_prefix(ENC_MAGIC).ok_or("Notiz ist beschädigt")?;
    let plain = pf.unprotect(blob).map_err(|e| format!("Entschlüsseln nicht möglich ({e})"))?;
    Ok(String::from_utf8_lossy(&plain).to_string())
}

/// Text ohne die Überschriftszeile.
pub fn body(content: &str) -> String {
    let mut lines = content.lines();
    let first = lines.clone().next().unwrap_or("");
    if first.starts_with("# ") {
        lines.next();
    }
    lines.collect::<Vec<_>>().join("\n").trim().to_string()
}

/// Neue Notiz anlegen. `secret` erzwingt Verschlüsselung (zusätzlich zu sensiblen Titeln).
pub fn create(pf: &dyn Platform, dir: &Path, title: &str, text: &str, secret: bool) -> Result<Note, String> {
    let title = safe_stem(title);
    let sensitive = secret || is_sensitive_title(&title);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let plain_path = dir.join(format!("{title}.md"));
    let enc_path = dir.join(format!("{title}{ENC_EXT}"));
    if plain_path.exists() || enc_path.exists() {
        return Err(format!("Die Notiz „{title}“ gibt es schon."));
    }
    let mut content = format!("# {title}\n\n");
    if !text.trim().is_empty() {
        content.push_str(&format!("- {}\n", text.trim()));
    }
    let path = if sensitive { enc_path } else { plain_path };
    std::fs::write(&path, encode(pf, &content, sensitive)?).map_err(|e| e.to_string())?;
    note_from_path(path).ok_or_else(|| "Notiz nicht lesbar".into())
}

/// Ganze Notiz überschreiben (Einstellungsfenster). Vorher Sicherungskopie in den Papierkorb.
pub fn save(pf: &dyn Platform, note: &Note, content: &str, trash: &Trash) -> Result<UndoOp, String> {
    let snap = trash.snapshot(&note.path).map_err(|e| e.to_string())?;
    std::fs::write(&note.path, encode(pf, content, note.sensitive)?).map_err(|e| e.to_string())?;
    Ok(UndoOp::Restore { trash_id: snap.id, name: snap.name })
}

/// Zeile anhängen. Liefert, wie es rückgängig geht.
pub fn append(pf: &dyn Platform, note: &Note, text: &str, trash: &Trash) -> Result<UndoOp, String> {
    let line = format!("- {}\n", text.trim());
    if note.sensitive {
        let content = read(pf, note)?;
        let sep = if content.ends_with('\n') { "" } else { "\n" };
        return save(pf, note, &format!("{content}{sep}{line}"), trash);
    }
    use std::io::Write;
    let len = std::fs::metadata(&note.path).map_err(|e| e.to_string())?.len();
    let needs_newline = len > 0 && std::fs::read(&note.path).map(|b| b.last() != Some(&b'\n')).unwrap_or(false);
    let mut f = std::fs::OpenOptions::new().append(true).open(&note.path).map_err(|e| e.to_string())?;
    if needs_newline {
        f.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    f.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    Ok(UndoOp::Truncate { path: note.path.clone(), len })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::mock::MockPlatform;

    #[test]
    fn sensitive_titles() {
        assert!(is_sensitive_title("Passwörter"));
        assert!(is_sensitive_title("WLAN Zugang"));
        assert!(is_sensitive_title("PIN Handy"));
        assert!(is_sensitive_title("Bankdaten"));
        assert!(!is_sensitive_title("Einkaufsliste"));
        assert!(!is_sensitive_title("Pinguine"), "kein Treffer nur wegen Wortanfang bei kurzen Wörtern");
    }

    #[test]
    fn create_append_encrypt() {
        let dir = std::env::temp_dir().join(format!("panco-notes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pf = MockPlatform::default();
        let trash = Trash::new(dir.join(".trash"));

        let n = create(&pf, &dir, "Einkauf", "Milch", false).unwrap();
        append(&pf, &n, "Eier", &trash).unwrap();
        assert_eq!(body(&read(&pf, &n).unwrap()), "- Milch\n- Eier");

        let s = create(&pf, &dir, "Passwörter", "", false).unwrap();
        assert!(s.sensitive && s.path.to_string_lossy().ends_with(".md.panco"));
        let undo = append(&pf, &s, "Steam: hunter2", &trash).unwrap();
        let raw = std::fs::read(&s.path).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("hunter2"), "Klartext darf nicht auf der Platte stehen");
        assert_eq!(body(&read(&pf, &s).unwrap()), "- Steam: hunter2");
        // Rückgängig stellt den verschlüsselten Stand vorher wieder her
        undo.apply(&pf, &trash).unwrap();
        assert_eq!(body(&read(&pf, &s).unwrap()), "");

        assert_eq!(find(&dir, "einkaufsliste").map(|n| n.title), Some("Einkauf".into()));
        assert!(create(&pf, &dir, "Einkauf", "", false).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}

//! Aktionsprotokoll: eine JSON-Zeile pro Ereignis in `%APPDATA%\Panco\logs\actions-JJJJ-MM.jsonl`.
//! Nur lokal. Enthält nie Audio, nur Zeit, Quelle, erkannten Text, Aktion und Ergebnis.

use serde::Serialize;
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Debug, Serialize, Clone)]
pub struct Entry {
    pub ts: String,
    /// "text", "voice", "remote:<Gerät>", "claude"
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<u8>,
    /// done | pending | confirmed | cancelled | expired | denied | failed | not_understood | disabled
    pub outcome: String,
    pub message: String,
}

pub struct ActionLog {
    dir: Option<PathBuf>,
    /// Letzte Einträge im Speicher (für Diagnose/Tests), begrenzt.
    recent: Mutex<Vec<Entry>>,
}

const RECENT_MAX: usize = 50;

impl ActionLog {
    /// `dir = None` → nur im Speicher (Tests).
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self { dir, recent: Mutex::new(Vec::new()) }
    }

    pub fn now() -> String {
        humantime::format_rfc3339_seconds(SystemTime::now()).to_string()
    }

    pub fn write(&self, entry: Entry) {
        if let Some(dir) = &self.dir {
            let month = &entry.ts[..7.min(entry.ts.len())];
            let path = dir.join(format!("actions-{month}.jsonl"));
            let line = serde_json::to_string(&entry).unwrap_or_default();
            let res = std::fs::create_dir_all(dir)
                .and_then(|_| std::fs::OpenOptions::new().create(true).append(true).open(&path))
                .and_then(|mut f| writeln!(f, "{line}"));
            if let Err(e) = res {
                eprintln!("[panco] Protokoll nicht schreibbar: {e}");
            }
        }
        let mut recent = self.recent.lock().unwrap();
        if recent.len() >= RECENT_MAX {
            recent.remove(0);
        }
        recent.push(entry);
    }

    pub fn recent(&self) -> Vec<Entry> {
        self.recent.lock().unwrap().clone()
    }

    pub fn dir(&self) -> Option<&PathBuf> {
        self.dir.as_ref()
    }
}

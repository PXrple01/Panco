//! Aktionsprotokoll: eine JSON-Zeile pro Ereignis in `%APPDATA%\Panco\logs\actions-JJJJ-MM.jsonl`.
//! Nur lokal. Enthält nie Audio, nur Zeit, Quelle, erkannten Text, Aktion und Ergebnis – und bei
//! umkehrbaren Aktionen, wie sie rückgängig gemacht werden.

use crate::undo::UndoOp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Entry {
    /// Eindeutige ID (für "Rückgängig").
    #[serde(default)]
    pub id: String,
    pub ts: String,
    /// "text", "voice", "remote:<Gerät>", "claude"
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<u8>,
    /// done | pending | confirmed | cancelled | expired | denied | failed | not_understood | disabled
    pub outcome: String,
    pub message: String,
    /// Wie diese Aktion rückgängig gemacht wird (falls möglich).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo: Option<UndoOp>,
    /// Dieser Eintrag macht den Eintrag mit dieser ID rückgängig.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_of: Option<String>,
}

pub struct ActionLog {
    dir: Option<PathBuf>,
    /// Letzte Einträge im Speicher (Tests ohne Ordner, schneller Zugriff), begrenzt.
    recent: Mutex<Vec<Entry>>,
    seq: AtomicU64,
}

const RECENT_MAX: usize = 300;

impl ActionLog {
    /// `dir = None` → nur im Speicher (Tests).
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self { dir, recent: Mutex::new(Vec::new()), seq: AtomicU64::new(0) }
    }

    pub fn now() -> String {
        humantime::format_rfc3339_seconds(SystemTime::now()).to_string()
    }

    /// Schreibt einen Eintrag (vergibt die ID) und gibt sie zurück.
    pub fn write(&self, mut entry: Entry) -> String {
        if entry.id.is_empty() {
            let millis = SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
            entry.id = format!("{millis}-{}", self.seq.fetch_add(1, Ordering::Relaxed));
        }
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
        let id = entry.id.clone();
        let mut recent = self.recent.lock().unwrap();
        if recent.len() >= RECENT_MAX {
            recent.remove(0);
        }
        recent.push(entry);
        id
    }

    /// Einträge dieser Sitzung (älteste zuerst).
    pub fn recent(&self) -> Vec<Entry> {
        self.recent.lock().unwrap().clone()
    }

    pub fn dir(&self) -> Option<&PathBuf> {
        self.dir.as_ref()
    }

    /// Die letzten `limit` Einträge aus den Protokolldateien (neueste zuerst).
    pub fn read_recent(&self, limit: usize) -> Vec<Entry> {
        let Some(dir) = &self.dir else {
            return self.recent().into_iter().rev().take(limit).collect();
        };
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("actions-")))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        let mut out = Vec::new();
        for f in files.iter().rev() {
            let Ok(raw) = std::fs::read_to_string(f) else { continue };
            for line in raw.lines().rev() {
                if let Ok(e) = serde_json::from_str::<Entry>(line) {
                    out.push(e);
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
        out
    }

    pub fn find(&self, id: &str) -> Option<Entry> {
        self.read_recent(2000).into_iter().find(|e| e.id == id)
    }

    /// Wurde dieser Eintrag schon rückgängig gemacht?
    pub fn is_undone(&self, id: &str) -> bool {
        self.read_recent(2000).iter().any(|e| e.undo_of.as_deref() == Some(id))
    }

    /// Letzte noch nicht rückgängig gemachte, umkehrbare Aktion (für "Panco, rückgängig").
    pub fn last_undoable(&self) -> Option<Entry> {
        let entries = self.read_recent(500);
        let undone: Vec<&str> = entries.iter().filter_map(|e| e.undo_of.as_deref()).collect();
        entries.iter().find(|e| e.undo.is_some() && !undone.contains(&e.id.as_str())).cloned()
    }
}

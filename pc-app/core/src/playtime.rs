//! Spiel-Erkennung (Auswertung) und Spielzeit. Was gerade läuft, meldet die App (Steam-Registry,
//! ggf. Prozessliste); hier wird daraus "Spiel gestartet/beendet" und die Spielzeit pro Tag.
//!
//! Gespeichert in `<Panco-Daten>/spielzeit.json` (wird mit gesichert): Spiel → Datum → Sekunden.

use crate::time::to_local;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Höchstens so viel Zeit wird zwischen zwei Beobachtungen angerechnet (Standby, Ruhezustand …).
const MAX_STEP: i64 = 120;
/// So oft wird spätestens gespeichert.
const FLUSH_EVERY: i64 = 300;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Running {
    pub name: String,
    /// Unix-Sekunden
    pub since: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Started(String),
    Stopped { name: String, secs: u64 },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Playtime {
    /// Spiel → "JJJJ-MM-TT" → Sekunden
    pub days: BTreeMap<String, BTreeMap<String, u64>>,
}

fn day_key(unix: i64, offset: i32) -> String {
    let d = to_local(unix, offset);
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

impl Playtime {
    /// Summe pro Spiel ab dem Tag `from` (einschließlich), absteigend sortiert.
    pub fn since_day(&self, from: &str) -> Vec<(String, u64)> {
        let mut v: Vec<(String, u64)> = self
            .days
            .iter()
            .map(|(game, days)| (game.clone(), days.range(from.to_string()..).map(|(_, s)| s).sum()))
            .filter(|(_, s): &(String, u64)| *s > 0)
            .collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        v
    }

    pub fn total(&self, game: &str) -> u64 {
        self.days.get(game).map(|d| d.values().sum()).unwrap_or(0)
    }

    pub fn games(&self) -> Vec<&str> {
        self.days.keys().map(String::as_str).collect()
    }
}

pub struct Tracker {
    file: PathBuf,
    pub data: Playtime,
    pub current: Option<Running>,
    last_seen: i64,
    last_flush: i64,
}

impl Tracker {
    pub fn load(file: &Path) -> Self {
        let data = std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { file: file.to_path_buf(), data, current: None, last_seen: 0, last_flush: 0 }
    }

    /// Neue Beobachtung: `running` = Name des laufenden Spiels (oder keins).
    pub fn observe(&mut self, now: i64, offset: i32, running: Option<&str>) -> Vec<Change> {
        // Zeit seit der letzten Beobachtung dem bisherigen Spiel gutschreiben.
        if let Some(cur) = &self.current {
            let step = (now - self.last_seen).clamp(0, MAX_STEP) as u64;
            *self.data.days.entry(cur.name.clone()).or_default().entry(day_key(now, offset)).or_default() += step;
        }
        self.last_seen = now;
        let mut changes = Vec::new();
        if self.current.as_ref().map(|c| c.name.as_str()) != running {
            if let Some(c) = self.current.take() {
                changes.push(Change::Stopped { name: c.name, secs: (now - c.since).max(0) as u64 });
            }
            if let Some(n) = running {
                self.current = Some(Running { name: n.to_string(), since: now });
                changes.push(Change::Started(n.to_string()));
            }
            self.flush(now);
        }
        if now - self.last_flush >= FLUSH_EVERY {
            self.flush(now);
        }
        changes
    }

    pub fn flush(&mut self, now: i64) {
        self.last_flush = now;
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.file.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(&self.data).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }

    pub fn today_key(now: i64, offset: i32) -> String {
        day_key(now, offset)
    }

    /// Montag dieser Woche
    pub fn week_start_key(now: i64, offset: i32) -> String {
        let wd = to_local(now, offset).weekday as i64;
        day_key(now - wd * 86_400, offset)
    }
}

/// "2 Std. 15 Min." / "25 Min." / "unter einer Minute"
pub fn duration_de(secs: u64) -> String {
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    match (h, m) {
        (0, 0) => "unter einer Minute".into(),
        (0, m) => format!("{m} Min."),
        (h, 0) => format!("{h} Std."),
        (h, m) => format!("{h} Std. {m} Min."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_sessions_and_days() {
        let file = std::env::temp_dir().join(format!("panco-spielzeit-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let mut t = Tracker::load(&file);
        let start = 1_793_017_800; // Mo 26.10.2026 13:30 MEZ
        assert_eq!(t.observe(start, 3600, None), vec![]);
        assert_eq!(t.observe(start + 10, 3600, Some("Deadlock")), vec![Change::Started("Deadlock".into())]);
        for i in 1..=60 {
            t.observe(start + 10 + i * 12, 3600, Some("Deadlock"));
        }
        // Standby: große Lücke zählt höchstens 2 Minuten
        t.observe(start + 10 + 720 + 5000, 3600, Some("Deadlock"));
        let stop = t.observe(start + 10 + 720 + 5012, 3600, None);
        assert!(matches!(&stop[..], [Change::Stopped { name, .. }] if name == "Deadlock"));
        assert_eq!(t.data.total("Deadlock"), 720 + 120 + 12);
        assert_eq!(t.data.since_day("2026-10-26"), vec![("Deadlock".into(), 852)]);
        // gespeichert und wieder ladbar
        assert_eq!(Tracker::load(&file).data, t.data);
        assert_eq!(Tracker::week_start_key(start, 3600), "2026-10-26");
        assert_eq!(duration_de(8100), "2 Std. 15 Min.");
        assert_eq!(duration_de(1500), "25 Min.");
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn switching_games() {
        let mut t = Tracker::load(&std::env::temp_dir().join("panco-spielzeit-nicht-da.json"));
        t.file = std::env::temp_dir().join(format!("panco-spielzeit-w-{}.json", std::process::id()));
        t.observe(100, 0, Some("A"));
        let ch = t.observe(160, 0, Some("B"));
        assert!(matches!(&ch[..], [Change::Stopped { .. }, Change::Started(b)] if b == "B"));
        assert_eq!(t.current.as_ref().unwrap().name, "B");
        assert_eq!(t.data.total("A"), 60);
        let _ = std::fs::remove_file(&t.file);
    }
}

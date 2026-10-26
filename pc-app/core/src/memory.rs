//! Gedächtnis (Merksätze + Regeln) und eigene Befehle (Makros). Beides besteht nur aus Text und
//! Whitelist-Aktionen – Schritte entstehen über den normalen Parser, nie als freie Befehle.
//!
//! - `gedaechtnis.json`: Merksätze ("Mein Lieblingsspiel ist Deadlock") und Regeln
//!   ("Wenn ich Deadlock starte, starte auch Discord")
//! - `befehle.json`: eigene Befehle ("Feierabend" = Discord schließen, Lautstärke 30, PC sperren)
//!
//! Beide Dateien liegen im Panco-Datenordner und werden mit gesichert.

use crate::catalog;
use crate::parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Diese Aktionen dürfen nicht in Regeln/Befehlen stecken (keine Verschachtelung, keine Schleifen).
const NOT_AS_STEP: [&str; 6] =
    ["panco.undo", "panco.repeat", "memory.remember", "memory.forget", "memory.rule", "macro.run"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Step {
    pub action: String,
    pub params: Value,
    /// So wie gesagt/geschrieben ("starte Discord") – für Anzeige und Bearbeitung.
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fact {
    pub id: String,
    pub text: String,
    pub created: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Ein bestimmtes Spiel startet (erkannt über Steam/Prozessliste) – `None` = jedes Spiel.
    GameStart { game: Option<String> },
    /// Panco startet dieses Programm ("starte Discord").
    AppStart { target: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub id: String,
    pub when: Trigger,
    pub steps: Vec<Step>,
    pub created: i64,
}

impl Rule {
    pub fn describe(&self) -> String {
        let when = match &self.when {
            Trigger::GameStart { game: Some(g) } => format!("Wenn {g} startet"),
            Trigger::GameStart { game: None } => "Wenn ein Spiel startet".into(),
            Trigger::AppStart { target } => format!("Wenn ich {} starte", crate::actions::display_name(target)),
        };
        format!("{when}: {}", self.steps.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(", "))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Memory {
    pub facts: Vec<Fact>,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Macro {
    pub id: String,
    /// Name = Auslöser ("Feierabend" → "Panco, Feierabend")
    pub name: String,
    /// Weitere Formulierungen, die denselben Befehl auslösen
    #[serde(default)]
    pub phrases: Vec<String>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Macros {
    pub list: Vec<Macro>,
}

fn load<T: Default + for<'de> Deserialize<'de>>(path: &Path) -> T {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn memory_path(data: &Path) -> PathBuf {
    data.join("gedaechtnis.json")
}

pub fn macros_path(data: &Path) -> PathBuf {
    data.join("befehle.json")
}

impl Memory {
    pub fn load(data: &Path) -> Self {
        load(&memory_path(data))
    }
    pub fn save(&self, data: &Path) -> Result<(), String> {
        save(&memory_path(data), self)
    }
}

impl Macros {
    pub fn load(data: &Path) -> Self {
        load(&macros_path(data))
    }
    pub fn save(&self, data: &Path) -> Result<(), String> {
        save(&macros_path(data), self)
    }

    /// Eigener Befehl zu diesem (normalisierten) Satz? Name oder eine der Formulierungen, genau.
    pub fn find(&self, normalized: &str) -> Option<&Macro> {
        let n = normalized.trim();
        self.list.iter().find(|m| std::iter::once(&m.name).chain(&m.phrases).any(|p| crate::normalize::command(p) == n))
    }
}

pub fn new_id() -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    format!("{ms}-{}", SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 1000)
}

/// "schließe Discord, Lautstärke 30 und dann sperr den PC" → Schritte. Jeder Teil muss ein
/// verständlicher Panco-Befehl sein; `max_tier` begrenzt die Stufe (Regeln: nur Stufe 1).
pub fn parse_steps(text: &str, max_tier: u8) -> Result<Vec<Step>, String> {
    let cleaned = text.replace(" und dann ", ",").replace(" danach ", ",").replace(" dann ", ",").replace(" und ", ",");
    let mut steps = Vec::new();
    for part in cleaned.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        // Füllwörter von Regeln ("starte auch Discord", "mach immer Spotify leiser")
        let part = part
            .split_whitespace()
            .filter(|w| !matches!(w.to_lowercase().as_str(), "auch" | "immer" | "bitte" | "automatisch"))
            .collect::<Vec<_>>()
            .join(" ");
        let intent = parser::parse_text(&part).ok_or_else(|| format!("„{part}“ verstehe ich nicht"))?;
        if NOT_AS_STEP.contains(&intent.action) {
            return Err(format!("„{part}“ geht nicht als Schritt"));
        }
        let spec = catalog::get(intent.action).ok_or_else(|| format!("„{part}“ ist nicht erlaubt"))?;
        if spec.tier as u8 > max_tier {
            return Err(format!("„{part}“ braucht eine Bestätigung und kann nicht automatisch laufen"));
        }
        steps.push(Step { action: intent.action.to_string(), params: intent.params, text: part });
    }
    if steps.is_empty() {
        return Err("keine Schritte erkannt".into());
    }
    if steps.len() > 10 {
        return Err("höchstens 10 Schritte".into());
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_from_text() {
        let s = parse_steps("schließe Discord, Lautstärke 30 und dann sperr den PC", 3).unwrap();
        let actions: Vec<&str> = s.iter().map(|s| s.action.as_str()).collect();
        assert_eq!(actions, vec!["app.close", "volume.set", "system.lock"]);
        assert_eq!(parse_steps("starte auch Discord", 1).unwrap()[0].action, "app.launch");
        // Regeln: nichts, was eine Rückfrage braucht
        assert!(parse_steps("fahr den PC herunter", 1).unwrap_err().contains("Bestätigung"));
        assert!(parse_steps("blablub", 3).unwrap_err().contains("verstehe ich nicht"));
        assert!(parse_steps("mach das rückgängig", 3).is_err());
    }

    #[test]
    fn macro_lookup_and_files() {
        let dir = std::env::temp_dir().join(format!("panco-memory-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut m = Macros::default();
        m.list.push(Macro {
            id: new_id(),
            name: "Feierabend".into(),
            phrases: vec!["Schluss für heute".into()],
            steps: parse_steps("Lautstärke 20", 3).unwrap(),
        });
        m.save(&dir).unwrap();
        let back = Macros::load(&dir);
        assert_eq!(back, m);
        assert!(back.find("feierabend").is_some());
        assert!(back.find("schluss für heute").is_some());
        assert!(back.find("feierabend machen").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}

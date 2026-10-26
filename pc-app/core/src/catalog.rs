//! Aktionskatalog aus `shared/schema/actions.json`: Whitelist, Mindeststufe, Remote-Freigabe.

use serde::Deserialize;
use std::sync::LazyLock;

/// Sicherheitsstufe einer Aktion (siehe CLAUDE.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "u8")]
pub enum Tier {
    /// Sofort ausführen.
    Instant = 1,
    /// Kurze Bestätigung (Sprache oder Overlay).
    Confirm = 2,
    /// Nur ausdrückliche Bestätigung per Klick am PC, nie remote.
    PcOnly = 3,
}

impl TryFrom<u8> for Tier {
    type Error = String;
    fn try_from(v: u8) -> Result<Self, String> {
        match v {
            1 => Ok(Tier::Instant),
            2 => Ok(Tier::Confirm),
            3 => Ok(Tier::PcOnly),
            _ => Err(format!("ungültige Stufe {v}")),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActionSpec {
    pub id: String,
    pub tier: Tier,
    pub remote: bool,
    pub title: String,
    pub description: String,
    pub params: serde_json::Value,
    /// Inhalte (Text-Parameter, erkannter Satz, Antwort) nicht ins Protokoll schreiben.
    #[serde(default)]
    pub redact: bool,
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    actions: Vec<ActionSpec>,
}

pub const CATALOG_JSON: &str = include_str!("../../../shared/schema/actions.json");

static CATALOG: LazyLock<Vec<ActionSpec>> =
    LazyLock::new(|| serde_json::from_str::<CatalogFile>(CATALOG_JSON).expect("actions.json ist gültig").actions);

pub fn all() -> &'static [ActionSpec] {
    &CATALOG
}

pub fn get(id: &str) -> Option<&'static ActionSpec> {
    CATALOG.iter().find(|a| a.id == id)
}

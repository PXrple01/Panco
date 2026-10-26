//! Whitelist-Aktionen. Jede Aktion ist ein eigenes Modul und muss in `shared/schema/actions.json`
//! stehen (ein Test prüft das).
//!
//! Neue Aktion hinzufügen:
//! 1. Eintrag in `actions.json` (id, tier, remote, params-Schema)
//! 2. Modul hier anlegen, `Action` implementieren, in `registry()` eintragen
//! 3. Optional: Parser-Regel in `parser.rs`, Sätze in `shared/texts/de.json`

use crate::catalog::Tier;
use crate::config::Config;
use crate::engine::Source;
use crate::platform::{Platform, PlatformError};
use crate::protocol::Mood;
use crate::steam::Game;
use serde::de::DeserializeOwned;
use serde_json::Value;

mod close;
mod focus;
mod launch;
mod media;
mod screen;
mod system;
mod volume;
mod web;

/// Alles, was eine Aktion beim Vorbereiten wissen darf.
pub struct Ctx<'a> {
    pub platform: &'a dyn Platform,
    pub config: &'a Config,
    pub games: &'a [Game],
    pub source: &'a Source,
}

/// Ergebnis einer ausgeführten Aktion.
#[derive(Debug)]
pub struct Done {
    pub text: String,
    pub mood: Mood,
}

impl Done {
    pub fn ok(text: String) -> Self {
        Done { text, mood: Mood::Success }
    }
}

/// Die eigentliche Ausführung – läuft sofort (Stufe 1) oder erst nach Bestätigung.
pub type Job = Box<dyn FnOnce(&dyn Platform) -> Result<Done, ActionError> + Send>;

pub enum Plan {
    /// Parameter geprüft, Ziel aufgelöst. `tier` kann über der Katalog-Mindeststufe liegen.
    Ready { tier: Tier, what: String, job: Job },
    /// Unsicher ("Meinst du Deadlock?"): bei Ja wird die Aktion mit `params` neu vorbereitet.
    Clarify { question: String, params: Value },
}

impl Plan {
    pub fn instant(
        what: impl Into<String>,
        job: impl FnOnce(&dyn Platform) -> Result<Done, ActionError> + Send + 'static,
    ) -> Self {
        Plan::Ready { tier: Tier::Instant, what: what.into(), job: Box::new(job) }
    }
    pub fn confirm(
        what: impl Into<String>,
        job: impl FnOnce(&dyn Platform) -> Result<Done, ActionError> + Send + 'static,
    ) -> Self {
        Plan::Ready { tier: Tier::Confirm, what: what.into(), job: Box::new(job) }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    #[error("ungültige Parameter: {0}")]
    InvalidParams(String),
    /// Ziel nicht gefunden; der Text ist die fertige Antwort.
    #[error("{0}")]
    NotFound(String),
    #[error("auf diesem System nicht verfügbar")]
    Unsupported,
    #[error("{0}")]
    Failed(String),
}

impl From<PlatformError> for ActionError {
    fn from(e: PlatformError) -> Self {
        match e {
            PlatformError::Unsupported => ActionError::Unsupported,
            PlatformError::Failed(m) => ActionError::Failed(m),
        }
    }
}

pub trait Action: Send + Sync {
    fn id(&self) -> &'static str;
    /// Prüft Parameter und löst Ziele auf – führt noch nichts aus.
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError>;
}

/// Parameter streng typisiert einlesen (unbekannte Felder → Fehler).
pub(crate) fn params<T: DeserializeOwned>(v: &Value) -> Result<T, ActionError> {
    let v = if v.is_null() { Value::Object(Default::default()) } else { v.clone() };
    serde_json::from_value(v).map_err(|e| ActionError::InvalidParams(e.to_string()))
}

/// Für Aktionen ohne Parameter: nur `{}` oder `null` erlaubt.
pub(crate) fn no_params(params: &Value) -> Result<(), ActionError> {
    match params {
        Value::Null => Ok(()),
        Value::Object(m) if m.is_empty() => Ok(()),
        _ => Err(ActionError::InvalidParams("keine Parameter erwartet".into())),
    }
}

/// Text-Parameter: getrimmt, nicht leer, Länge begrenzt, keine Steuerzeichen.
pub(crate) fn clean_text(s: &str, max: usize) -> Result<String, ActionError> {
    let s = s.trim();
    if s.is_empty() || s.chars().count() > max || s.chars().any(char::is_control) {
        return Err(ActionError::InvalidParams("Text leer, zu lang oder mit Steuerzeichen".into()));
    }
    Ok(s.to_string())
}

/// "discord" → "Discord"
pub(crate) fn display_name(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Alle Aktionen. Die Reihenfolge ist egal, die id zählt.
pub fn registry() -> Vec<Box<dyn Action>> {
    vec![
        Box::new(launch::AppLaunch),
        Box::new(close::AppClose),
        Box::new(focus::WindowFocus),
        Box::new(volume::VolumeSet),
        Box::new(volume::VolumeChange),
        Box::new(volume::VolumeMute),
        Box::new(media::MediaControl),
        Box::new(web::WebSearch),
        Box::new(web::WebOpen),
        Box::new(screen::Screenshot),
        Box::new(system::Lock),
        Box::new(system::Power),
    ]
}

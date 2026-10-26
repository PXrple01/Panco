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
use crate::files::Places;
use crate::log::ActionLog;
use crate::platform::{Platform, PlatformError};
use crate::protocol::{Choice, Mood};
use crate::steam::Game;
use crate::undo::UndoOp;
use serde::de::DeserializeOwned;
use serde_json::Value;

mod close;
mod files;
mod focus;
mod launch;
mod media;
mod notes;
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
    /// Freigegebene Ordner, Papierkorb, Notizen …
    pub places: &'a Places,
    pub log: &'a ActionLog,
    /// Unix-Sekunden
    pub now: i64,
}

/// Ergebnis einer ausgeführten Aktion.
#[derive(Debug)]
pub struct Done {
    pub text: String,
    pub mood: Mood,
    /// Wie sich die Aktion rückgängig machen lässt (landet im Protokoll).
    pub undo: Option<UndoOp>,
    /// Diese Aktion hat den Protokolleintrag mit dieser ID rückgängig gemacht.
    pub undo_of: Option<String>,
    pub choices: Vec<Choice>,
}

impl Done {
    pub fn new(text: impl Into<String>, mood: Mood) -> Self {
        Done { text: text.into(), mood, undo: None, undo_of: None, choices: Vec::new() }
    }
    pub fn ok(text: impl Into<String>) -> Self {
        Self::new(text, Mood::Success)
    }
    pub fn undo(mut self, op: UndoOp) -> Self {
        self.undo = Some(op);
        self
    }
    pub fn choices(mut self, c: Vec<Choice>) -> Self {
        self.choices = c;
        self
    }
}

/// Die eigentliche Ausführung – läuft sofort (Stufe 1) oder erst nach Bestätigung.
pub type Job = Box<dyn FnOnce(&dyn Platform) -> Result<Done, ActionError> + Send>;

pub enum Plan {
    /// Parameter geprüft, Ziel aufgelöst. `tier` kann über der Katalog-Mindeststufe liegen.
    Ready { tier: Tier, what: String, job: Job },
    /// Rückfrage ("Meinst du Deadlock?" / "Gefunden: X. Öffnen?"): bei Ja wird `action` (oder dieselbe
    /// Aktion) mit `params` neu vorbereitet. `choices` = Alternativen zum Anklicken.
    Clarify { question: String, params: Value, action: Option<&'static str>, choices: Vec<Choice> },
}

impl Plan {
    pub fn clarify(question: impl Into<String>, params: Value) -> Self {
        Plan::Clarify { question: question.into(), params, action: None, choices: Vec::new() }
    }
    pub fn ready(
        tier: Tier,
        what: impl Into<String>,
        job: impl FnOnce(&dyn Platform) -> Result<Done, ActionError> + Send + 'static,
    ) -> Self {
        Plan::Ready { tier, what: what.into(), job: Box::new(job) }
    }
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
        Box::new(files::FileSearch),
        Box::new(files::FileOpen),
        Box::new(files::FileDelete),
        Box::new(files::FileMove),
        Box::new(files::FileRename),
        Box::new(files::FolderCreate),
        Box::new(files::TrashRestore),
        Box::new(files::Undo),
        Box::new(notes::NoteCreate),
        Box::new(notes::NoteAppend),
        Box::new(notes::NoteRead),
        Box::new(notes::NoteSearch),
    ]
}

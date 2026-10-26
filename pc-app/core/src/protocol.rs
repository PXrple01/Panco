//! Rust-Spiegel von `shared/schema/messages.schema.json`.
//! Ein Test (tests/schema.rs) prüft, dass serialisierte Nachrichten dem Schema entsprechen.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientMessage {
    Command { id: String, text: String },
    Invoke { id: String, action: String, params: Value },
    Confirm { confirm_id: String, accept: bool },
}

/// Auswahlmöglichkeit unter einer Antwort (z. B. Suchtreffer): ein Klick ruft die Aktion auf.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub action: String,
    pub params: Value,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mood {
    Success,
    Game,
    Confused,
    Error,
    Info,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AvatarState {
    Idle,
    Alert,
    Listening,
    Thinking,
    Sleeping,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MicState {
    Off,
    Wake,
    Recording,
    Muted,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Reply {
        id: String,
        ok: bool,
        text: String,
        mood: Mood,
        #[serde(skip_serializing_if = "Option::is_none")]
        action: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        choices: Vec<Choice>,
        /// Vollständiges Ergebnis für Claude (z. B. Dateiinhalt) – wird nie an Oberflächen geschickt.
        #[serde(skip)]
        data: Option<String>,
        /// Bild für Claude (PNG) – wird nie an Oberflächen geschickt.
        #[serde(skip)]
        image: Option<Vec<u8>>,
    },
    ConfirmRequest {
        id: String,
        confirm_id: String,
        text: String,
        tier: u8,
        expires_in_ms: u64,
        pc_only: bool,
        /// Vorschau zum Prüfen (Diff, Skript)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        choices: Vec<Choice>,
    },
    /// Claude-Antwort, die gerade entsteht.
    Stream {
        id: String,
        text: String,
        done: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        activity: Option<String>,
    },
    State {
        avatar: AvatarState,
        mic: MicState,
        enabled: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        level: Option<f32>,
    },
}

impl ServerMessage {
    pub fn reply(id: &str, ok: bool, text: impl Into<String>, mood: Mood, action: Option<&str>) -> Self {
        ServerMessage::Reply {
            id: id.to_string(),
            ok,
            text: text.into(),
            mood,
            action: action.map(str::to_string),
            choices: Vec::new(),
            data: None,
            image: None,
        }
    }

    /// Hängt Auswahlmöglichkeiten an eine Antwort oder Rückfrage.
    pub fn with_choices(mut self, new: Vec<Choice>) -> Self {
        match &mut self {
            ServerMessage::Reply { choices, .. } | ServerMessage::ConfirmRequest { choices, .. } => *choices = new,
            ServerMessage::Stream { .. } | ServerMessage::State { .. } => {}
        }
        self
    }

    pub fn choices(&self) -> &[Choice] {
        match self {
            ServerMessage::Reply { choices, .. } | ServerMessage::ConfirmRequest { choices, .. } => choices,
            ServerMessage::Stream { .. } | ServerMessage::State { .. } => &[],
        }
    }

    /// Text, den Panco sagen/anzeigen soll (für TTS und Tests).
    pub fn text(&self) -> &str {
        match self {
            ServerMessage::Reply { text, .. }
            | ServerMessage::ConfirmRequest { text, .. }
            | ServerMessage::Stream { text, .. } => text,
            ServerMessage::State { .. } => "",
        }
    }
}

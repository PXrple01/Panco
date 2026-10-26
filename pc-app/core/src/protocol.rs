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
    },
    ConfirmRequest {
        id: String,
        confirm_id: String,
        text: String,
        tier: u8,
        expires_in_ms: u64,
        pc_only: bool,
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
        ServerMessage::Reply { id: id.to_string(), ok, text: text.into(), mood, action: action.map(str::to_string) }
    }

    /// Text, den Panco sagen/anzeigen soll (für TTS und Tests).
    pub fn text(&self) -> &str {
        match self {
            ServerMessage::Reply { text, .. } | ServerMessage::ConfirmRequest { text, .. } => text,
            ServerMessage::State { .. } => "",
        }
    }
}

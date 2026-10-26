//! Befehle, die das Overlay aufrufen darf. Alles läuft über die Engine (Whitelist + Stufen).

use crate::overlay;
use panco_core::protocol::{ClientMessage, ServerMessage};
use panco_core::Source;
use serde::Serialize;
use tauri::AppHandle;

/// Overlay → Engine. Eingaben im Overlay gelten als "am PC" (Quelle Text, Bestätigung per Klick).
#[tauri::command]
pub async fn send(app: AppHandle, msg: ClientMessage) -> Result<ServerMessage, String> {
    let engine = crate::engine(&app);
    tauri::async_runtime::spawn_blocking(move || engine.handle(msg, Source::Text)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn hide_overlay(app: AppHandle) {
    overlay::hide(&app);
}

#[derive(Serialize)]
pub struct UiSettings {
    reduce_motion: bool,
    show_avatar: bool,
    auto_hide_ms: u64,
    enabled: bool,
    /// Aktueller Avatar-/Mikrofonzustand (das Overlay wird oft erst nach einem Ereignis erzeugt).
    state: ServerMessage,
}

#[tauri::command]
pub fn ui_settings(app: AppHandle) -> UiSettings {
    let engine = crate::engine(&app);
    let o = engine.config().overlay;
    UiSettings {
        reduce_motion: o.reduce_motion,
        show_avatar: o.show_avatar,
        auto_hide_ms: o.auto_hide_ms,
        enabled: engine.enabled(),
        state: crate::state_message(&app),
    }
}

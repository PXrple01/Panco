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
    /// Antwort/Rückfrage und erkannter Text, die eintrafen, während das Overlay noch lud.
    pending: Option<ServerMessage>,
    heard: Option<String>,
}

#[tauri::command]
pub fn ui_settings(app: AppHandle) -> UiSettings {
    let engine = crate::engine(&app);
    let o = engine.config().overlay;
    let (pending, heard) = overlay::take_early();
    UiSettings {
        reduce_motion: o.reduce_motion,
        show_avatar: o.show_avatar,
        auto_hide_ms: o.auto_hide_ms,
        enabled: engine.enabled(),
        state: crate::state_message(&app),
        pending,
        heard,
    }
}

/// Overlay-Höhe an den Inhalt anpassen (z. B. Trefferliste). Begrenzt, damit es nie den Bildschirm füllt.
#[tauri::command]
pub fn overlay_height(app: AppHandle, height: f64) {
    if let Some(w) = tauri::Manager::get_webview_window(&app, overlay::LABEL) {
        let h = height.clamp(150.0, 560.0);
        let _ = w.set_size(tauri::LogicalSize::new(overlay::WIDTH, h));
    }
}

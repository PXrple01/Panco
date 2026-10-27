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
    tauri::async_runtime::spawn_blocking(move || {
        if let ClientMessage::Command { id, text } = &msg {
            // Was Panco nicht selbst versteht, geht (wenn eingeschaltet) an Claude.
            if let Some(reply) = crate::assistant::route(&app, id, text, Source::Text) {
                return reply;
            }
        }
        engine.handle(msg, Source::Text)
    })
    .await
    .map_err(|e| e.to_string())
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
    /// Mini-Panco (per Sprache geöffnet) statt voller Karte
    compact: bool,
    /// Game Mode: Avatar schläft, keine Animation
    game_mode: bool,
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
        compact: overlay::is_compact(),
        game_mode: engine.current_game().is_some(),
        state: crate::state_message(&app),
        pending,
        heard,
    }
}

/// Overlay-Größe an den Inhalt anpassen (CSS-Pixel bei Größe 1.0; Rust rechnet Größe und Ecke ein).
#[tauri::command]
pub fn overlay_size(app: AppHandle, width: f64, height: f64) {
    overlay::resize(&app, width, height);
}

/// Am Avatar ziehen verschiebt das Overlay.
#[tauri::command]
pub fn overlay_drag(app: AppHandle) {
    overlay::drag(&app);
}

/// ✕ / Esc: versehentlich geöffnet? Zuhören, Vorlesen und eine offene Rückfrage abbrechen, ausblenden.
#[tauri::command]
pub async fn overlay_dismiss(app: AppHandle) {
    crate::voice::cancel(&app);
    let engine = crate::engine(&app);
    if engine.has_pending() {
        engine.confirm(None, false, panco_core::engine::ConfirmVia::PcClick);
    }
    overlay::hide(&app);
}

/// Eigene Titelleiste (Einstellungen, Ersteinrichtung): ziehen, minimieren, maximieren, schließen.
#[tauri::command]
pub fn window_action(window: tauri::WebviewWindow, action: String) -> Result<(), String> {
    let r = match action.as_str() {
        "drag" => window.start_dragging(),
        "min" => window.minimize(),
        "max" => match window.is_maximized() {
            Ok(true) => window.unmaximize(),
            _ => window.maximize(),
        },
        "close" => window.close(),
        _ => return Err("unbekannte Fensteraktion".into()),
    };
    r.map_err(|e| e.to_string())
}

//! Overlay-Fenster: rahmenlos, halbtransparent, oben mittig, immer im Vordergrund.
//! Wird bei jedem Einblenden neu erzeugt und beim Ausblenden zerstört (spart RAM im Leerlauf).

use panco_core::protocol::ServerMessage;
use std::sync::Mutex;
use tauri::utils::config::WindowEffectsConfig;
use tauri::window::Effect;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "overlay";
pub const WIDTH: f64 = 560.0;
const HEIGHT: f64 = 210.0;

/// Was eintrifft, während das Overlay noch lädt: Der WebView-Kaltstart dauert unter Last (Whisper
/// rechnet parallel) oft länger als die Erkennung eines kurzen Befehls. Das Overlay holt es beim Start ab.
struct Early {
    ready: bool,
    msg: Option<ServerMessage>,
    heard: Option<String>,
}

static EARLY: Mutex<Early> = Mutex::new(Early { ready: false, msg: None, heard: None });

fn reset_early() {
    *EARLY.lock().unwrap() = Early { ready: false, msg: None, heard: None };
}

/// Vom Overlay beim Start aufgerufen: ab jetzt direkt senden; liefert Verpasstes (Antwort, Erkanntes).
pub fn take_early() -> (Option<ServerMessage>, Option<String>) {
    let mut e = EARLY.lock().unwrap();
    e.ready = true;
    (e.msg.take(), e.heard.take())
}

pub fn is_open(app: &AppHandle) -> bool {
    app.get_webview_window(LABEL).is_some()
}

pub fn toggle(app: &AppHandle) {
    if is_open(app) {
        hide(app);
    } else {
        show(app, true);
    }
}

/// Zeigt das Overlay. `focus = false` für Sprachbefehle, damit ein laufendes Spiel den Fokus behält.
pub fn show(app: &AppHandle, focus: bool) {
    if let Some(w) = app.get_webview_window(LABEL) {
        if focus {
            let _ = w.set_focus();
        }
        return;
    }
    // Fenster nicht im Event-Handler selbst bauen (WebView2 kann dort blockieren) – eigener Task.
    reset_early();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = build(&app, focus) {
            eprintln!("[panco] Overlay konnte nicht erstellt werden: {e}");
        }
    });
}

fn build(app: &AppHandle, focus: bool) -> tauri::Result<()> {
    let cfg = crate::engine(app).config().overlay;
    let mut builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Panco")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .focused(focus);

    // Oben mittig auf dem Hauptbildschirm (wie eine Befehlspalette).
    if let Some(m) = app.primary_monitor()? {
        let scale = m.scale_factor();
        let size = m.size().to_logical::<f64>(scale);
        let pos = m.position().to_logical::<f64>(scale);
        builder = builder.position(pos.x + (size.width - WIDTH) / 2.0, pos.y + size.height * 0.16);
    } else {
        builder = builder.center();
    }

    if cfg.acrylic {
        builder = builder.effects(WindowEffectsConfig {
            effects: vec![Effect::Acrylic],
            state: None,
            radius: Some(22.0),
            color: None,
            interactive: false,
        });
    }
    builder.build()?;
    Ok(())
}

/// Blendet aus und gibt WebView2 frei.
pub fn hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.destroy();
    }
    reset_early();
}

/// Nachricht an das Overlay schicken, falls es offen ist (sonst nichts tun).
/// Lädt es noch, wird die letzte Antwort/Rückfrage aufgehoben; den Zustand fragt es beim Start selbst ab.
pub fn emit(app: &AppHandle, msg: &ServerMessage) {
    if !is_open(app) {
        return;
    }
    let mut early = EARLY.lock().unwrap();
    if !early.ready {
        if !matches!(msg, ServerMessage::State { .. }) {
            early.msg = Some(msg.clone());
        }
        return;
    }
    drop(early);
    let _ = app.emit_to(LABEL, "panco://message", msg);
}

/// Erkannten Text zur Kontrolle im Overlay zeigen.
pub fn heard(app: &AppHandle, text: &str) {
    if !is_open(app) {
        return;
    }
    let mut early = EARLY.lock().unwrap();
    if !early.ready {
        early.heard = Some(text.to_string());
        return;
    }
    drop(early);
    let _ = app.emit_to(LABEL, "panco://heard", text);
}

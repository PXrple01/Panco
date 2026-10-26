//! Overlay-Fenster: rahmenlos, halbtransparent, immer im Vordergrund – auf dem gewählten Bildschirm,
//! an der gewählten Ecke (oder dort, wohin man es gezogen hat), in der gewählten Größe.
//! Zwei Formen: Mini-Panco (bei "Panco, …": kleines Icon + erkannter Text) und die volle Karte mit
//! Eingabezeile (Strg+Leertaste). Wird bei jedem Einblenden neu erzeugt und beim Ausblenden
//! zerstört (spart RAM im Leerlauf).

use panco_core::config::{config_path, OverlayAnchor, OverlayConfig};
use panco_core::protocol::ServerMessage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::utils::config::WindowEffectsConfig;
use tauri::window::{Effect, Monitor};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};

pub const LABEL: &str = "overlay";
/// Startgrößen (logische Pixel bei Größe 1.0); danach folgt das Fenster dem Inhalt.
const FULL: (f64, f64) = (560.0, 150.0);
const MINI: (f64, f64) = (260.0, 72.0);
/// Abstand zum Bildschirmrand (logische Pixel)
const MARGIN: f64 = 16.0;

/// Mini-Panco oder volle Karte (für `ui_settings`).
static COMPACT: AtomicBool = AtomicBool::new(false);
/// Einblenden angefordert, Fenster evtl. noch im Aufbau: Nachrichten werden bis dahin aufgehoben.
static OPENING: AtomicBool = AtomicBool::new(false);
/// Wohin Panco das Fenster zuletzt selbst gesetzt hat – weicht die Position ab, hat man es gezogen.
static PLACED: Mutex<Option<PhysicalPosition<i32>>> = Mutex::new(None);

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

pub fn is_compact() -> bool {
    COMPACT.load(Ordering::Relaxed)
}

pub fn toggle(app: &AppHandle) {
    match app.get_webview_window(LABEL) {
        // Mini-Panco ist offen → zur vollen Karte mit Eingabezeile
        Some(_) if is_compact() => show(app, true),
        Some(_) => hide(app),
        None => show(app, true),
    }
}

/// Zeigt das Overlay. `focus = false` für Sprachbefehle: Mini-Panco (falls eingestellt), und ein
/// laufendes Spiel behält den Fokus.
pub fn show(app: &AppHandle, focus: bool) {
    if let Some(w) = app.get_webview_window(LABEL) {
        if focus {
            if is_compact() {
                COMPACT.store(false, Ordering::Relaxed);
                let _ = app.emit_to(LABEL, "panco://mode", "full");
            }
            let _ = w.set_focus();
        }
        return;
    }
    let compact = !focus && crate::engine(app).config().overlay.compact;
    COMPACT.store(compact, Ordering::Relaxed);
    // Fenster nicht im Event-Handler selbst bauen (WebView2 kann dort blockieren) – eigener Task.
    reset_early();
    OPENING.store(true, Ordering::Relaxed);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = build(&app, focus, compact) {
            eprintln!("[panco] Overlay konnte nicht erstellt werden: {e}");
        }
    });
}

fn build(app: &AppHandle, focus: bool, compact: bool) -> tauri::Result<()> {
    let cfg = crate::engine(app).config().overlay;
    let scale = cfg.scale.clamp(0.75, 1.5);
    let (w, h) = if compact { MINI } else { FULL };
    let size = LogicalSize::new(w * scale, h * scale);
    let monitor = target_monitor(app, &cfg.monitor);
    let mut builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Panco")
        .inner_size(size.width, size.height)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .focused(focus);
    // Ungefähr schon beim Erzeugen an die richtige Stelle (ohne Sprung), genau gleich danach.
    let target = monitor.as_ref().map(|m| {
        let sf = m.scale_factor();
        place(&cfg, m, PhysicalSize::new((size.width * sf) as u32, (size.height * sf) as u32))
    });
    if let (Some(p), Some(m)) = (target, &monitor) {
        builder = builder.position(p.x as f64 / m.scale_factor(), p.y as f64 / m.scale_factor());
    }
    // Glas-Effekt nur für die volle Karte (die runde Mini-Form hätte sonst eckiges Glas dahinter).
    if cfg.acrylic && !compact {
        builder = builder.effects(WindowEffectsConfig {
            effects: vec![Effect::Acrylic],
            state: None,
            radius: Some(22.0),
            color: None,
            interactive: false,
        });
    }
    let window = builder.build()?;
    if let Some(p) = target {
        let _ = window.set_position(p);
        *PLACED.lock().unwrap() = Some(p);
    }
    let _ = window.set_size(size);
    if (scale - 1.0).abs() > 0.01 {
        let _ = window.set_zoom(scale);
    }
    Ok(())
}

/// Gewählter Bildschirm (per Name) oder der Hauptbildschirm.
fn target_monitor(app: &AppHandle, name: &str) -> Option<Monitor> {
    let all = app.available_monitors().unwrap_or_default();
    all.iter()
        .find(|m| !name.is_empty() && m.name().is_some_and(|n| n == name))
        .cloned()
        .or_else(|| app.primary_monitor().ok().flatten())
        .or_else(|| all.into_iter().next())
}

/// Position (physische Pixel) für ein Fenster dieser Größe auf dem Bildschirm – innerhalb der
/// Arbeitsfläche (ohne Taskleiste).
fn place(cfg: &OverlayConfig, m: &Monitor, size: PhysicalSize<u32>) -> PhysicalPosition<i32> {
    let area = m.work_area();
    let margin = (MARGIN * m.scale_factor()) as i32;
    let free_w = (area.size.width as i32 - size.width as i32).max(0);
    let free_h = (area.size.height as i32 - size.height as i32).max(0);
    let (start, mid, end) = (margin.min(free_w), free_w / 2, (free_w - margin).max(0));
    let (top, center, bottom) = (margin.min(free_h), free_h / 2, (free_h - margin).max(0));
    let (x, y) = match cfg.anchor {
        OverlayAnchor::TopLeft => (start, top),
        OverlayAnchor::TopCenter => (mid, top),
        OverlayAnchor::TopRight => (end, top),
        OverlayAnchor::BottomLeft => (start, bottom),
        OverlayAnchor::BottomCenter => (mid, bottom),
        OverlayAnchor::BottomRight => (end, bottom),
        OverlayAnchor::Center => (mid, center),
        OverlayAnchor::Custom => {
            let [fx, fy] = cfg.custom.unwrap_or([1.0, 0.0]);
            ((fx.clamp(0.0, 1.0) * free_w as f64) as i32, (fy.clamp(0.0, 1.0) * free_h as f64) as i32)
        }
    };
    PhysicalPosition::new(area.position.x + x, area.position.y + y)
}

/// Fenster an den Inhalt anpassen (`width`/`height` in CSS-Pixeln bei Größe 1.0). Hat man Panco
/// gezogen, bleibt er dort; sonst bleibt er an seiner Ecke (wächst z. B. unten nach oben).
pub fn resize(app: &AppHandle, width: f64, height: f64) {
    let Some(w) = app.get_webview_window(LABEL) else { return };
    let cfg = crate::engine(app).config().overlay;
    let scale = cfg.scale.clamp(0.75, 1.5);
    let size = LogicalSize::new(width.clamp(80.0, 700.0) * scale, height.clamp(56.0, 600.0) * scale);
    let _ = w.set_size(size);
    let moved = user_moved(&w);
    if let (false, Ok(Some(m))) = (moved, w.current_monitor()) {
        let p = place(&cfg, &m, size.to_physical(m.scale_factor()));
        let _ = w.set_position(p);
        *PLACED.lock().unwrap() = Some(p);
    }
}

fn user_moved(w: &tauri::WebviewWindow) -> bool {
    match (w.outer_position(), *PLACED.lock().unwrap()) {
        (Ok(now), Some(placed)) => (now.x - placed.x).abs() > 3 || (now.y - placed.y).abs() > 3,
        _ => false,
    }
}

/// Ziehen am Avatar: das Betriebssystem verschiebt das Fenster.
pub fn drag(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.start_dragging();
    }
}

/// Gezogene Position merken: Bildschirm + Anteil der freien Fläche (übersteht Auflösungswechsel).
fn remember_position(app: &AppHandle, w: &tauri::WebviewWindow) {
    if !user_moved(w) {
        return;
    }
    let (Ok(pos), Ok(size)) = (w.outer_position(), w.outer_size()) else { return };
    let cx = pos.x as f64 + size.width as f64 / 2.0;
    let cy = pos.y as f64 + size.height as f64 / 2.0;
    let Ok(Some(m)) = app.monitor_from_point(cx, cy).or_else(|_| w.current_monitor()) else { return };
    let area = m.work_area();
    let free_w = (area.size.width as i32 - size.width as i32).max(1) as f64;
    let free_h = (area.size.height as i32 - size.height as i32).max(1) as f64;
    let fx = ((pos.x - area.position.x) as f64 / free_w).clamp(0.0, 1.0);
    let fy = ((pos.y - area.position.y) as f64 / free_h).clamp(0.0, 1.0);
    let engine = crate::engine(app);
    let mut cfg = engine.config();
    cfg.overlay.anchor = OverlayAnchor::Custom;
    cfg.overlay.custom = Some([(fx * 1000.0).round() / 1000.0, (fy * 1000.0).round() / 1000.0]);
    cfg.overlay.monitor = m.name().cloned().unwrap_or_default();
    if cfg.save(&config_path()).is_ok() {
        engine.set_config(cfg);
    }
}

/// Blendet aus und gibt WebView2 frei (eine gezogene Position wird vorher gemerkt).
pub fn hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        remember_position(app, &w);
        let _ = w.destroy();
    }
    *PLACED.lock().unwrap() = None;
    OPENING.store(false, Ordering::Relaxed);
    reset_early();
}

/// Offen oder gerade im Aufbau?
fn wanted(app: &AppHandle) -> bool {
    OPENING.load(Ordering::Relaxed) || is_open(app)
}

/// Auswahl der Bildschirme für die Einstellungen.
#[derive(serde::Serialize)]
pub struct MonitorInfo {
    name: String,
    label: String,
}

pub fn monitors(app: &AppHandle) -> Vec<MonitorInfo> {
    let primary = app.primary_monitor().ok().flatten().and_then(|m| m.name().cloned());
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let name = m.name().cloned().unwrap_or_default();
            let main = if primary.as_ref() == Some(&name) { ", Hauptbildschirm" } else { "" };
            let label = format!("Bildschirm {} ({}×{}{main})", i + 1, m.size().width, m.size().height);
            MonitorInfo { name, label }
        })
        .collect()
}

/// Nachricht an das Overlay schicken, falls es offen ist (sonst nichts tun).
/// Lädt es noch, wird die letzte Antwort/Rückfrage aufgehoben; den Zustand fragt es beim Start selbst ab.
pub fn emit(app: &AppHandle, msg: &ServerMessage) {
    if !wanted(app) {
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
    if !wanted(app) {
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

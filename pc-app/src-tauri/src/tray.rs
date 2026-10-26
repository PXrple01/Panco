//! Tray-Icon mit Menü. Das Icon zeigt den Zustand: normal, lauscht auf "Panco", nimmt auf,
//! stumm, Not-Aus – man sieht also immer, ob das Mikrofon offen ist.

use crate::{overlay, settings, voice};
use panco_core::config::{config_path, data_dir, VoiceMode};
use panco_core::protocol::MicState;
use std::sync::atomic::Ordering;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

const TRAY_ID: &str = "panco";
const ICON_NORMAL: &[u8] = include_bytes!("../icons/tray.png");
const ICON_OFF: &[u8] = include_bytes!("../icons/tray-off.png");
const ICON_LISTEN: &[u8] = include_bytes!("../icons/tray-listen.png");
const ICON_REC: &[u8] = include_bytes!("../icons/tray-rec.png");
const ICON_MUTED: &[u8] = include_bytes!("../icons/tray-muted.png");

/// Menüeinträge, deren Zustand sich ändert.
struct TrayItems {
    kill: CheckMenuItem<Wry>,
    mute: CheckMenuItem<Wry>,
    mode_wake: CheckMenuItem<Wry>,
    mode_ptt: CheckMenuItem<Wry>,
    mode_off: CheckMenuItem<Wry>,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let none = None::<&str>;
    let kill = CheckMenuItem::with_id(app, "kill", "Not-Aus", true, false, none)?;
    let mute = CheckMenuItem::with_id(app, "mute", "Mikrofon stumm", true, false, none)?;
    let mode_wake = CheckMenuItem::with_id(app, "mode_wake", "Wake-Word „Panco“ (offenes Mikro)", true, false, none)?;
    let mode_ptt = CheckMenuItem::with_id(app, "mode_ptt", "Nur Push-to-Talk", true, false, none)?;
    let mode_off = CheckMenuItem::with_id(app, "mode_off", "Aus", true, false, none)?;
    let voice_menu = Submenu::with_items(app, "Sprachsteuerung", true, &[&mode_wake, &mode_ptt, &mode_off])?;

    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", "Panco öffnen", true, none)?,
            &PredefinedMenuItem::separator(app)?,
            &mute,
            &voice_menu,
            &kill,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "settings", "Einstellungen & Diagnose …", true, none)?,
            &MenuItem::with_id(app, "rescan", "Steam-Spiele neu einlesen", true, none)?,
            &MenuItem::with_id(app, "config", "Konfigurationsdatei öffnen", true, none)?,
            &MenuItem::with_id(app, "reload", "Konfiguration neu laden", true, none)?,
            &MenuItem::with_id(app, "log", "Protokoll öffnen", true, none)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Beenden", true, none)?,
        ],
    )?;
    app.manage(TrayItems { kill, mute, mode_wake, mode_ptt, mode_off });

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(ICON_NORMAL)?)
        .tooltip("Panco")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => overlay::show(app, true),
            "kill" => {
                let on = !crate::engine(app).enabled();
                crate::set_enabled(app, on);
            }
            "mute" => voice::toggle_muted(app),
            "mode_wake" => set_mode(app, VoiceMode::Wake),
            "mode_ptt" => set_mode(app, VoiceMode::PushToTalk),
            "mode_off" => set_mode(app, VoiceMode::Off),
            "settings" => settings::open(app),
            "rescan" => {
                let app = app.clone();
                std::thread::spawn(move || {
                    let n = crate::engine(&app).rescan_games();
                    voice::apply(&app); // neue Spielnamen ins Whisper-Vokabular
                    eprintln!("[panco] {n} Steam-Spiele gefunden");
                });
            }
            "config" => open_file(app, &config_path().to_string_lossy()),
            "reload" => crate::reload_config(app),
            "log" => open_file(app, &data_dir().join("logs").to_string_lossy()),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                let app = tray.app_handle();
                if crate::engine(app).enabled() {
                    overlay::toggle(app);
                }
            }
        })
        .build(app)?;
    refresh(app);
    Ok(())
}

fn set_mode(app: &AppHandle, mode: VoiceMode) {
    let engine = crate::engine(app);
    let mut cfg = engine.config();
    cfg.voice.mode = mode;
    if let Err(e) = cfg.save(&config_path()) {
        eprintln!("[panco] Konfig nicht gespeichert: {e}");
    }
    engine.set_config(cfg);
    voice::apply(app);
}

/// Icon, Tooltip und Haken an den aktuellen Zustand anpassen.
pub fn refresh(app: &AppHandle) {
    let engine = crate::engine(app);
    let enabled = engine.enabled();
    let mode = engine.config().voice.mode;
    let (muted, mic) = match app.try_state::<voice::Voice>() {
        Some(v) => (v.muted.load(Ordering::Relaxed), *v.mic.lock().unwrap()),
        None => (false, MicState::Off),
    };

    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.kill.set_checked(!enabled);
        let _ = items.mute.set_checked(muted);
        let _ = items.mode_wake.set_checked(mode == VoiceMode::Wake);
        let _ = items.mode_ptt.set_checked(mode == VoiceMode::PushToTalk);
        let _ = items.mode_off.set_checked(mode == VoiceMode::Off);
    }
    let (icon, tip) = if !enabled {
        (ICON_OFF, "Panco – Not-Aus aktiv")
    } else if muted {
        (ICON_MUTED, "Panco – Mikrofon stumm")
    } else {
        match mic {
            MicState::Recording => (ICON_REC, "Panco – hört zu (Aufnahme)"),
            MicState::Wake => (ICON_LISTEN, "Panco – wartet auf „Panco“ (Mikro offen)"),
            _ if mode == VoiceMode::Wake && !voice::wakeword_ready() => {
                (ICON_NORMAL, "Panco – Wake-Word noch nicht eingesprochen (Einstellungen)")
            }
            _ => (ICON_NORMAL, "Panco – Mikro aus"),
        }
    };
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(img) = Image::from_bytes(icon) {
            let _ = tray.set_icon(Some(img));
        }
        let _ = tray.set_tooltip(Some(tip));
    }
}

/// Datei/Ordner mit dem Standardprogramm öffnen (über die Plattform-Schicht, keine Shell).
fn open_file(app: &AppHandle, path: &str) {
    let pf = crate::engine(app).platform().clone();
    if pf.open(path, &[]).is_err() && cfg!(windows) {
        let _ = pf.open("notepad.exe", &[path.to_string()]);
    }
}

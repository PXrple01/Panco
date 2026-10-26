//! Panco PC-App: Tray, globale Hotkeys, Overlay auf Abruf.
//!
//! Ressourcen-Prinzip: Im Leerlauf existiert KEIN Fenster (kein WebView2-Prozess).
//! Das Overlay wird per Hotkey erzeugt und beim Ausblenden wieder zerstört.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hotkeys;
mod ipc;
mod overlay;
mod tray;

use panco_core::config::{config_path, data_dir, Config};
use panco_core::log::ActionLog;
use panco_core::protocol::{AvatarState, MicState, ServerMessage};
use panco_core::{platform, Engine};
use std::sync::Arc;
use tauri::{AppHandle, Manager, RunEvent};

/// Gemeinsamer App-Zustand.
pub struct Panco {
    pub engine: Arc<Engine>,
}

pub fn engine(app: &AppHandle) -> Arc<Engine> {
    app.state::<Panco>().engine.clone()
}

/// Not-Aus umschalten: Engine, Tray und Overlay synchron halten.
pub fn set_enabled(app: &AppHandle, on: bool) {
    engine(app).set_enabled(on);
    tray::refresh(app);
    if !on {
        overlay::hide(app);
    }
    overlay::emit(app, &state_message(app));
}

pub fn state_message(app: &AppHandle) -> ServerMessage {
    ServerMessage::State { avatar: AvatarState::Idle, mic: MicState::Off, enabled: engine(app).enabled(), level: None }
}

/// Konfig neu laden (Tray-Menü) und Hotkeys neu registrieren.
pub fn reload_config(app: &AppHandle) {
    let (cfg, err) = Config::load_or_create(&config_path());
    if let Some(e) = err {
        eprintln!("[panco] {e}");
    }
    engine(app).set_config(cfg);
    hotkeys::register_all(app);
}

fn main() {
    let (config, cfg_err) = Config::load_or_create(&config_path());
    if let Some(e) = cfg_err {
        eprintln!("[panco] {e}");
    }
    let log = ActionLog::new(Some(data_dir().join("logs")));
    let engine = Arc::new(Engine::new(platform::native(), config, log));

    let app = tauri::Builder::default()
        // Zweiter Start öffnet nur das Overlay der laufenden Instanz.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| overlay::show(app, true)))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(Panco { engine })
        .invoke_handler(tauri::generate_handler![ipc::send, ipc::hide_overlay, ipc::ui_settings])
        .setup(|app| {
            tray::create(app.handle())?;
            hotkeys::register_all(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Panco konnte nicht starten");

    app.run(|_, event| {
        // Ohne Fenster weiterlaufen; nur "Beenden" im Tray (exit mit Code) beendet wirklich.
        if let RunEvent::ExitRequested { api, code: None, .. } = event {
            api.prevent_exit();
        }
    });
}

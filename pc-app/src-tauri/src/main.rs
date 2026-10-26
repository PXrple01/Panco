//! Panco PC-App: Tray, globale Hotkeys, Overlay auf Abruf.
//!
//! Ressourcen-Prinzip: Im Leerlauf existiert KEIN Fenster (kein WebView2-Prozess).
//! Das Overlay wird per Hotkey erzeugt und beim Ausblenden wieder zerstört.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod crash;
mod data;
mod hotkeys;
mod ipc;
mod maintenance;
mod overlay;
mod settings;
mod setup;
mod system;
mod tray;
mod voice;

use panco_core::config::{config_path, data_dir, Config};
use panco_core::log::ActionLog;
use panco_core::protocol::ServerMessage;
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
    voice::apply(app); // Not-Aus schließt sofort das Mikrofon
    tray::refresh(app);
    if !on {
        overlay::hide(app);
    }
    overlay::emit(app, &state_message(app));
}

pub fn state_message(app: &AppHandle) -> ServerMessage {
    let v = voice::get(app);
    let (avatar, mic) = (*v.avatar.lock().unwrap(), *v.mic.lock().unwrap());
    ServerMessage::State { avatar, mic, enabled: engine(app).enabled(), level: None }
}

/// Konfig neu laden (Tray-Menü) und Hotkeys neu registrieren.
pub fn reload_config(app: &AppHandle) {
    let (cfg, err) = Config::load_or_create(&config_path());
    if let Some(e) = err {
        eprintln!("[panco] {e}");
    }
    engine(app).set_config(cfg);
    hotkeys::register_all(app);
    voice::apply(app);
}

/// Startet panco-waechter neben dieser panco.exe. `false` = nicht vorhanden → ohne Wächter weiter.
fn start_watchdog(args: &[String]) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let name = if cfg!(windows) { "panco-waechter.exe" } else { "panco-waechter" };
    let watchdog = exe.with_file_name(name);
    if !watchdog.is_file() {
        crash::note("panco-waechter fehlt – Panco läuft ohne Neustart-Schutz.");
        return false;
    }
    let pass = args.iter().filter(|a| *a == "--autostart");
    std::process::Command::new(watchdog)
        .arg("--log")
        .arg(data_dir().join("logs").join("absturz.log"))
        .arg("--")
        .arg(&exe)
        .arg("--app")
        .args(pass)
        .spawn()
        .is_ok()
}

fn main() {
    crash::install(data_dir().join("logs"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (config, cfg_err) = Config::load_or_create(&config_path());
    if let Some(e) = cfg_err {
        eprintln!("[panco] {e}");
    }
    // Ohne "--app" übernimmt der winzige Wächter (panco-waechter.exe): Er startet Panco als
    // Kindprozess und startet es nach einem Absturz neu. Dieser Start-Prozess endet sofort.
    if !args.iter().any(|a| a == "--app") && config.system.watchdog && start_watchdog(&args) {
        return;
    }
    let autostart = config.system.autostart;
    let log = ActionLog::new(Some(data_dir().join("logs")));
    let engine = Arc::new(Engine::new(platform::native(), config, log));

    let app = tauri::Builder::default()
        // Zweiter Start öffnet nur das Overlay der laufenden Instanz.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| overlay::show(app, true)))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .manage(Panco { engine })
        .manage(settings::CpuSampler::default())
        .invoke_handler(tauri::generate_handler![
            ipc::send,
            ipc::hide_overlay,
            ipc::ui_settings,
            ipc::overlay_size,
            ipc::overlay_drag,
            ipc::overlay_dismiss,
            ipc::window_action,
            settings::status,
            settings::appearance,
            settings::set_overlay,
            settings::overlay_preview,
            settings::tts_preview,
            settings::set_voice,
            settings::set_muted,
            settings::download_model,
            settings::train_start,
            settings::train_cancel,
            system::system_status,
            system::set_system,
            setup::setup_state,
            setup::setup_save,
            setup::setup_finish,
            data::files_settings,
            data::files_save,
            data::backup_now,
            data::backups,
            data::open_place,
            data::notes_list,
            data::note_get,
            data::note_put,
            data::note_new,
            data::log_entries,
            data::undo,
            data::trash_entries,
            data::trash_restore,
        ])
        .setup(move |app| {
            voice::start(app.handle());
            tray::create(app.handle())?;
            hotkeys::register_all(app.handle());
            maintenance::start(app.handle());
            // Autostart-Eintrag aktuell halten (zeigt immer auf diese panco.exe).
            if let Err(e) = autostart::sync(autostart) {
                crash::note(&format!("Autostart nicht einstellbar: {e}"));
            }
            // Erster Start: Ordner für Papierkorb, Backup und Notizen wählen lassen.
            if !crate::engine(app.handle()).config().setup.done {
                setup::open(app.handle());
            }
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

//! Spiel-Erkennung im Hintergrund: Ein schlafender Thread wacht auf, wenn Steam "RunningAppID"
//! ändert (Registry-Benachrichtigung, kein Polling) – spätestens alle 12 s, um auch Spiele außerhalb
//! von Steam an ihrem Exe-Namen zu erkennen (nur wenn welche eingetragen sind).
//!
//! Game Mode = ein Spiel läuft: Avatar schläft, Backups warten bis nach dem Spiel, Regeln für den
//! Spielstart werden ausgeführt.

use panco_core::platform;
use panco_core::playtime::Change;
use std::time::Duration;
use tauri::AppHandle;

const CHECK_EVERY: Duration = Duration::from_secs(12);

pub fn start(app: &AppHandle) {
    let app = app.clone();
    let _ = std::thread::Builder::new().name("panco-spiele".into()).spawn(move || loop {
        let engine = crate::engine(&app);
        let steam = platform::steam_running_app();
        let procs = if engine.config().game_processes.is_empty() {
            Vec::new()
        } else {
            engine.platform().processes().unwrap_or_default()
        };
        let running = engine.detect_game(steam, &procs);
        for change in engine.observe_game(running.as_deref()) {
            on_change(&app, &change);
        }
        platform::wait_steam_change(CHECK_EVERY);
    });
}

fn on_change(app: &AppHandle, change: &Change) {
    crate::tray::refresh(app);
    crate::overlay::emit(app, &crate::state_message(app));
    match change {
        Change::Started(name) => {
            crate::engine(app).on_game_started(name);
        }
        // Nach dem Spiel: aufgeschobenes Backup nachholen.
        Change::Stopped { .. } => crate::maintenance::poke(app),
    }
}

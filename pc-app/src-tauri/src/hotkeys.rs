//! Globale Hotkeys (aus config.json): Overlay ein/aus, Not-Aus.

use crate::overlay;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub fn register_all(app: &AppHandle) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let keys = crate::engine(app).config().hotkeys;

    bind(app, &keys.overlay, |app| {
        // Bei Not-Aus reagiert Panco nur noch auf den Not-Aus-Hotkey.
        if crate::engine(app).enabled() {
            overlay::toggle(app);
        }
    });
    bind(app, &keys.kill_switch, |app| {
        let on = !crate::engine(app).enabled();
        crate::set_enabled(app, on);
    });
}

fn bind(app: &AppHandle, keys: &str, action: fn(&AppHandle)) {
    let shortcut: Shortcut = match keys.parse() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[panco] Hotkey '{keys}' ungültig: {e}");
            return;
        }
    };
    let res = app.global_shortcut().on_shortcut(shortcut, move |app, _, event| {
        if event.state == ShortcutState::Pressed {
            action(app);
        }
    });
    if let Err(e) = res {
        eprintln!("[panco] Hotkey '{keys}' ist schon belegt: {e}");
    }
}

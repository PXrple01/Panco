//! Globale Hotkeys (aus config.json): Overlay, Not-Aus, Push-to-Talk (halten), Mikro stumm.

use crate::{overlay, voice};
use panco_voice::Ctl;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub fn register_all(app: &AppHandle) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let keys = crate::engine(app).config().hotkeys;

    bind(app, &keys.overlay, |app, pressed| {
        // Bei Not-Aus reagiert Panco nur noch auf den Not-Aus-Hotkey.
        if pressed && crate::engine(app).enabled() {
            overlay::toggle(app);
        }
    });
    bind(app, &keys.kill_switch, |app, pressed| {
        if pressed {
            let on = !crate::engine(app).enabled();
            crate::set_enabled(app, on);
        }
    });
    // Push-to-Talk: Mikro nur, solange die Taste gehalten wird.
    bind(app, &keys.push_to_talk, |app, pressed| {
        if crate::engine(app).enabled() {
            voice::get(app).svc.send(Ctl::Ptt(pressed));
        }
    });
    bind(app, &keys.mic_mute, |app, pressed| {
        if pressed {
            voice::toggle_muted(app);
        }
    });
}

fn bind(app: &AppHandle, keys: &str, action: fn(&AppHandle, bool)) {
    let shortcut: Shortcut = match keys.parse() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[panco] Hotkey '{keys}' ungültig: {e}");
            return;
        }
    };
    let res = app.global_shortcut().on_shortcut(shortcut, move |app, _, event| {
        action(app, event.state == ShortcutState::Pressed);
    });
    if let Err(e) = res {
        eprintln!("[panco] Hotkey '{keys}' ist schon belegt: {e}");
    }
}

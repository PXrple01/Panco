//! Tray-Icon mit Menü. Das Icon zeigt den Zustand (normal / Not-Aus).

use crate::overlay;
use panco_core::config::{config_path, data_dir};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

const TRAY_ID: &str = "panco";
const ICON_NORMAL: &[u8] = include_bytes!("../icons/tray.png");
const ICON_OFF: &[u8] = include_bytes!("../icons/tray-off.png");

/// Menüeinträge, deren Zustand sich ändert.
struct TrayItems {
    kill: CheckMenuItem<Wry>,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let kill = CheckMenuItem::with_id(app, "kill", "Not-Aus", true, false, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", "Panco öffnen", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &kill,
            &MenuItem::with_id(app, "rescan", "Steam-Spiele neu einlesen", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "settings", "Einstellungen bearbeiten", true, None::<&str>)?,
            &MenuItem::with_id(app, "reload", "Einstellungen neu laden", true, None::<&str>)?,
            &MenuItem::with_id(app, "log", "Protokoll öffnen", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)?,
        ],
    )?;
    app.manage(TrayItems { kill });

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
            "rescan" => {
                let engine = crate::engine(app);
                std::thread::spawn(move || {
                    let n = engine.rescan_games();
                    eprintln!("[panco] {n} Steam-Spiele gefunden");
                });
            }
            "settings" => open_file(app, &config_path().to_string_lossy()),
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

/// Icon, Tooltip und Haken an den aktuellen Zustand anpassen.
pub fn refresh(app: &AppHandle) {
    let enabled = crate::engine(app).enabled();
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.kill.set_checked(!enabled);
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let icon = if enabled { ICON_NORMAL } else { ICON_OFF };
        if let Ok(img) = Image::from_bytes(icon) {
            let _ = tray.set_icon(Some(img));
        }
        let _ = tray.set_tooltip(Some(if enabled { "Panco" } else { "Panco – Not-Aus aktiv" }));
    }
}

/// Datei/Ordner mit dem Standardprogramm öffnen (über die Plattform-Schicht, keine Shell).
fn open_file(app: &AppHandle, path: &str) {
    let pf = crate::engine(app).platform().clone();
    if pf.open(path, &[]).is_err() && cfg!(windows) {
        let _ = pf.open("notepad.exe", &[path.to_string()]);
    }
}

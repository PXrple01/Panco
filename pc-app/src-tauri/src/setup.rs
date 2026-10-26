//! Ersteinrichtung beim ersten Start: Papierkorb-, Backup- und Notizen-Ordner, freigegebene Ordner,
//! Wohnort. Danach geht es mit Sprachmodell und Wake-Word im Einstellungsfenster weiter.

use panco_core::config::{config_path, expand_env, FileSettings};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "setup";

pub fn open(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.set_focus();
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let built = WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::App("setup.html".into()))
            .title("Panco – Ersteinrichtung")
            .inner_size(720.0, 760.0)
            .min_inner_size(520.0, 560.0)
            .decorations(false)
            .shadow(true)
            .background_color(tauri::window::Color(8, 6, 14, 255))
            .center()
            .build();
        if let Err(e) = built {
            eprintln!("[panco] Ersteinrichtung: {e}");
        }
    });
}

#[derive(Serialize, Deserialize)]
pub struct SetupData {
    files: FileSettings,
    city: String,
    done: bool,
}

#[tauri::command]
pub fn setup_state(app: AppHandle) -> SetupData {
    let cfg = crate::engine(&app).config();
    SetupData { files: cfg.files, city: cfg.setup.city, done: cfg.setup.done }
}

/// Prüft die gewählten Ordner, legt sie an und speichert.
pub fn validate(files: &FileSettings) -> Result<(), String> {
    let abs = |label: &str, s: &str| -> Result<(), String> {
        if s.trim().is_empty() || !Path::new(&expand_env(s)).is_absolute() {
            return Err(format!("{label}: bitte einen vollständigen Ordnerpfad wählen."));
        }
        Ok(())
    };
    abs("Papierkorb", &files.trash_dir)?;
    abs("Backup", &files.backup_dir)?;
    abs("Notizen", &files.notes_dir)?;
    for f in &files.allowed_folders {
        abs("Freigegebener Ordner", f)?;
    }
    if !(1..=365).contains(&files.trash_days) {
        return Err("Aufbewahrungsfrist: 1 bis 365 Tage.".into());
    }
    if !(1..=60).contains(&files.backup_keep) {
        return Err("Backups behalten: 1 bis 60.".into());
    }
    for dir in [&files.trash_dir, &files.backup_dir, &files.notes_dir] {
        std::fs::create_dir_all(expand_env(dir)).map_err(|e| format!("{dir}: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn setup_save(app: AppHandle, files: FileSettings, city: String) -> Result<(), String> {
    validate(&files)?;
    let engine = crate::engine(&app);
    let mut cfg = engine.config();
    cfg.files = files;
    cfg.setup.city = city.trim().chars().take(80).collect();
    cfg.setup.done = true;
    cfg.save(&config_path()).map_err(|e| e.to_string())?;
    engine.set_config(cfg);
    engine.log_system("setup", "Ersteinrichtung abgeschlossen");
    crate::maintenance::poke(&app); // erstes Backup gleich anlegen
    Ok(())
}

/// Am Ende: weiter zu Sprachmodell + Wake-Word.
#[tauri::command]
pub fn setup_finish(app: AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.destroy();
    }
    crate::settings::open(&app);
}

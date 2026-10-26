//! Reiter "Schutz & Start": Autostart, Wächter, Aktivierungs-Schutz und Absturzprotokoll.

use crate::{autostart, voice};
use panco_core::config::{config_path, data_dir, SystemSettings};
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::AppHandle;

#[derive(Serialize)]
pub struct Protection {
    /// Punktzahl der letzten Wake-Word-Erkennung
    last_wake: Option<f32>,
    rejected_speakers: u64,
    rejected_voice: u64,
    loopback_active: bool,
    /// Durchschnittliche Rechenzeit der Schutzprüfung pro Wake-Word (ms)
    guard_ms: Option<f64>,
    profile_ready: bool,
}

#[derive(Serialize)]
pub struct SystemStatus {
    system: SystemSettings,
    autostart_active: bool,
    /// Läuft Panco gerade unter dem Wächter?
    watched: bool,
    protection: Protection,
    /// Letzte Zeilen aus logs/absturz.log
    crash_log: Vec<String>,
}

#[tauri::command]
pub fn system_status(app: AppHandle) -> SystemStatus {
    let stats = voice::get(&app).svc.stats().clone();
    let runs = stats.guard_runs.load(Ordering::Relaxed);
    let last_wake = *stats.last_wake.lock().unwrap();
    let log = std::fs::read_to_string(data_dir().join("logs").join("absturz.log")).unwrap_or_default();
    let mut crash_log: Vec<String> = log.lines().rev().take(8).map(str::to_string).collect();
    crash_log.reverse();
    SystemStatus {
        system: crate::engine(&app).config().system,
        autostart_active: autostart::is_enabled(),
        watched: std::env::args().any(|a| a == "--app"),
        protection: Protection {
            last_wake,
            rejected_speakers: stats.rejected_speakers.load(Ordering::Relaxed),
            rejected_voice: stats.rejected_voice.load(Ordering::Relaxed),
            loopback_active: stats.loopback_active.load(Ordering::Relaxed),
            guard_ms: (runs > 0).then(|| stats.guard_ns.load(Ordering::Relaxed) as f64 / runs as f64 / 1e6),
            profile_ready: voice::voice_profile_path().is_file(),
        },
        crash_log,
    }
}

#[tauri::command]
pub fn set_system(app: AppHandle, system: SystemSettings) -> Result<(), String> {
    autostart::sync(system.autostart)?;
    let engine = crate::engine(&app);
    let mut cfg = engine.config();
    cfg.system = system;
    cfg.save(&config_path()).map_err(|e| e.to_string())?;
    engine.set_config(cfg);
    Ok(())
}

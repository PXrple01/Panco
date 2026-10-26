//! Fenster "Einstellungen & Diagnose" – nur auf Abruf erzeugt, beim Schließen zerstört.
//! Zeigt RAM/CPU von Panco und der Spracherkennung, Modelle, Wake-Word-Training.

use crate::voice;
use panco_core::config::{config_path, models_dir, Hotkeys, VoiceSettings};
use panco_core::platform::ProcStats;
use panco_voice::model;
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "settings";

pub fn open(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.set_focus();
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let built = WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::App("settings.html".into()))
            .title("Panco – Einstellungen & Diagnose")
            .inner_size(560.0, 760.0)
            .min_inner_size(460.0, 520.0)
            .center()
            .build();
        if let Err(e) = built {
            eprintln!("[panco] Einstellungsfenster: {e}");
        }
    });
}

/// Letzte CPU-Messung, um Prozent aus der Differenz zu berechnen.
#[derive(Default)]
pub struct CpuSampler(Mutex<Option<(Instant, f64)>>);

#[derive(Serialize)]
pub struct ModelRow {
    id: &'static str,
    label: &'static str,
    installed: bool,
}

#[derive(Serialize)]
pub struct LastStt {
    text: String,
    engine: &'static str,
    audio_ms: u64,
    load_ms: u64,
    stt_ms: u64,
}

#[derive(Serialize)]
pub struct Status {
    process: Option<ProcStats>,
    cpu_percent: Option<f64>,
    enabled: bool,
    muted: bool,
    mic: panco_core::protocol::MicState,
    wake_cpu_percent: f64,
    whisper_loaded: bool,
    wakeword_ready: bool,
    last: Option<LastStt>,
    models: Vec<ModelRow>,
    voice: VoiceSettings,
    games: usize,
    config_path: String,
    hotkeys: Hotkeys,
}

#[tauri::command]
pub fn status(app: AppHandle) -> Status {
    let engine = crate::engine(&app);
    let v = voice::get(&app);
    let stats = v.svc.stats();
    let process = engine.platform().process_stats().ok();

    let cpu_percent = process.as_ref().and_then(|p| {
        let sampler = app.state::<CpuSampler>();
        let mut last = sampler.0.lock().unwrap();
        let now = Instant::now();
        let pct = last.map(|(t, cpu)| (p.cpu_seconds - cpu) / now.duration_since(t).as_secs_f64().max(0.001) * 100.0);
        *last = Some((now, p.cpu_seconds));
        pct
    });

    let last = stats.last.lock().unwrap().clone().map(|t| LastStt {
        text: t.text,
        engine: t.engine,
        audio_ms: t.audio_ms,
        load_ms: t.load_ms,
        stt_ms: t.stt_ms,
    });
    let dir = models_dir();
    let mic = *v.mic.lock().unwrap();
    let status = Status {
        process,
        cpu_percent,
        enabled: engine.enabled(),
        muted: v.muted.load(Ordering::Relaxed),
        mic,
        wake_cpu_percent: stats.wake_cpu_percent(),
        whisper_loaded: stats.whisper_loaded.load(Ordering::Relaxed),
        wakeword_ready: voice::wakeword_ready(),
        last,
        models: model::MODELS
            .iter()
            .map(|m| ModelRow { id: m.id, label: m.label, installed: dir.join(m.file).is_file() })
            .collect(),
        voice: engine.config().voice,
        games: engine.games().len(),
        config_path: config_path().to_string_lossy().into(),
        hotkeys: engine.config().hotkeys,
    };
    status
}

/// Spracheinstellungen speichern und sofort anwenden.
#[tauri::command]
pub fn set_voice(app: AppHandle, settings: VoiceSettings) -> Result<(), String> {
    if model::info(&settings.model).is_none() {
        return Err("unbekanntes Modell".into());
    }
    let engine = crate::engine(&app);
    let mut cfg = engine.config();
    cfg.voice = settings;
    cfg.save(&config_path()).map_err(|e| e.to_string())?;
    engine.set_config(cfg);
    voice::apply(&app);
    Ok(())
}

#[tauri::command]
pub fn set_muted(app: AppHandle, muted: bool) {
    voice::set_muted(&app, muted);
}

#[derive(Clone, Serialize)]
struct Progress {
    id: String,
    done: u64,
    total: u64,
    finished: bool,
    error: Option<String>,
}

/// Modell herunterladen – nur auf Klick, mit Prüfsumme. Fortschritt per Event.
#[tauri::command]
pub fn download_model(app: AppHandle, id: String) -> Result<(), String> {
    model::info(&id).ok_or("unbekanntes Modell")?;
    std::thread::spawn(move || {
        let mut last_emit = Instant::now();
        let result = model::download(&id, &models_dir(), |done, total| {
            if last_emit.elapsed().as_millis() > 200 {
                last_emit = Instant::now();
                let _ = app.emit_to(
                    LABEL,
                    "panco://download",
                    Progress { id: id.clone(), done, total, finished: false, error: None },
                );
            }
        });
        let error = result.err();
        let _ = app.emit_to(LABEL, "panco://download", Progress { id, done: 0, total: 0, finished: true, error });
    });
    Ok(())
}

#[tauri::command]
pub fn train_start(app: AppHandle) {
    voice::get(&app).svc.send(panco_voice::Ctl::TrainStart);
}

#[tauri::command]
pub fn train_cancel(app: AppHandle) {
    voice::get(&app).svc.send(panco_voice::Ctl::TrainCancel);
}

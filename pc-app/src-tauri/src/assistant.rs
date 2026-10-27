//! Claude-Modus in der App: Weiterleitung (was Panco nicht selbst versteht), Zwischenstände ins
//! Overlay, Sätze in die Sprachausgabe, Rückfragen zum Klicken – und der Einstellungsreiter.

use crate::{overlay, voice};
use panco_core::claude::{self, Assistant, Sink};
use panco_core::config::config_path;
use panco_core::platform::HttpRequest;
use panco_core::protocol::{Mood, ServerMessage};
use panco_core::Source;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Manager};

pub fn get(app: &AppHandle) -> Arc<Assistant> {
    app.state::<Arc<Assistant>>().inner().clone()
}

/// ✕/Esc/Not-Aus: laufende Claude-Antwort abbrechen.
pub fn cancel(app: &AppHandle) {
    if let Some(a) = app.try_state::<Arc<Assistant>>() {
        a.cancel();
    }
}

/// Geht dieser Satz an Claude? Liefert dann die fertige Antwort (Zwischenstände, Vorlesen und
/// Rückfragen sind da schon passiert); `None` = Panco erledigt ihn selbst.
pub fn route(app: &AppHandle, id: &str, text: &str, source: Source) -> Option<ServerMessage> {
    let engine = crate::engine(app);
    let norm = panco_core::normalize::command(text);
    let norm = norm.trim_start_matches("panco ").trim();
    if matches!(norm, "neues gespräch" | "neues thema" | "neue unterhaltung" | "vergiss das gespräch" | "fang neu an")
    {
        get(app).reset();
        return Some(ServerMessage::reply(
            id,
            true,
            panco_core::texts::t("claude_new_conversation", &[]),
            Mood::Info,
            None,
        ));
    }
    // "Frag Claude: …" erzwingt Claude, sonst nur, was der Parser nicht versteht.
    let forced = ["frag claude", "frage claude", "claude"]
        .iter()
        .find_map(|p| norm.strip_prefix(p).filter(|r| r.is_empty() || r.starts_with(' ')).map(|_| ()));
    if forced.is_none() && engine.understands(text, &source) {
        return None;
    }
    if !claude::ready(&engine) {
        return None;
    }
    let question = if forced.is_some() {
        let lower = text.to_lowercase();
        let cut = lower.find("claude").map(|i| i + "claude".len()).unwrap_or(0);
        text[cut..].trim_start_matches([':', ',', ' ']).to_string()
    } else {
        text.to_string()
    };
    let speak = engine.config().voice.speak_replies;
    let sink = AppSink { app: app.clone(), id: id.to_string(), speak, state: Mutex::new((String::new(), None)) };
    voice::set_avatar(app, panco_core::protocol::AvatarState::Thinking);
    let msg = get(app).ask(&engine, id, &question, &sink);
    voice::set_avatar(app, panco_core::protocol::AvatarState::Idle);
    // Fehler und Abbruch wurden nicht gestreamt – die sagt Panco jetzt.
    if let ServerMessage::Reply { ok: false, text, .. } = &msg {
        if speak {
            voice::queue(app, text);
        }
    }
    Some(msg)
}

struct AppSink {
    app: AppHandle,
    id: String,
    speak: bool,
    /// bisheriger Text, letzte Anzeige (für eine ruhige Anzeige höchstens ~15× pro Sekunde)
    state: Mutex<(String, Option<Instant>)>,
}

impl AppSink {
    fn show(&self, activity: Option<&str>) {
        let text = self.state.lock().unwrap().0.clone();
        overlay::emit(
            &self.app,
            &ServerMessage::Stream {
                id: self.id.clone(),
                text: text.chars().take(4000).collect(),
                done: false,
                activity: activity.map(str::to_string),
            },
        );
    }
}

impl Sink for AppSink {
    fn text(&self, so_far: &str) {
        let due = {
            let mut s = self.state.lock().unwrap();
            s.0 = so_far.to_string();
            let due = s.1.is_none_or(|t| t.elapsed().as_millis() >= 66);
            if due {
                s.1 = Some(Instant::now());
            }
            due
        };
        if due {
            self.show(None);
        }
    }
    fn sentence(&self, sentence: &str) {
        if self.speak {
            voice::queue(&self.app, sentence);
        }
    }
    fn activity(&self, what: Option<&str>) {
        // Sichtbar, solange Panco liest (Bildschirm, Zwischenablage, Dateien, Skripte).
        if what.is_some() && !overlay::is_open(&self.app) {
            overlay::show(&self.app, false);
        }
        self.show(what);
    }
    fn question(&self, msg: &ServerMessage) {
        overlay::show(&self.app, false);
        overlay::emit(&self.app, msg);
        if self.speak {
            voice::queue(&self.app, msg.text());
        }
    }
}

// ---------------------------------------------------------------- Einstellungen

#[derive(Serialize)]
pub struct ScriptRow {
    id: String,
    name: String,
    purpose: String,
    script: String,
}

#[derive(Serialize)]
pub struct ClaudeState {
    enabled: bool,
    has_key: bool,
    model: String,
    effort: String,
    allow_screen_images: bool,
    models: Vec<(String, String)>,
    history: Vec<claude::HistoryItem>,
    scripts: Vec<ScriptRow>,
    sandbox: bool,
}

#[tauri::command]
pub fn claude_state(app: AppHandle) -> ClaudeState {
    let engine = crate::engine(&app);
    let cfg = engine.config().claude;
    let pf = engine.platform().clone();
    ClaudeState {
        enabled: cfg.enabled,
        has_key: claude::has_key(pf.as_ref()),
        model: cfg.model,
        effort: cfg.effort,
        allow_screen_images: cfg.allow_screen_images,
        models: claude::MODELS.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        history: get(&app).history(),
        scripts: panco_core::actions::shell::SavedScripts::load(&engine.data_dir())
            .list
            .into_iter()
            .map(|s| ScriptRow { id: s.id, name: s.name, purpose: s.purpose, script: s.script })
            .collect(),
        sandbox: pf.sandbox_available(),
    }
}

/// Key in den Windows-Tresor (leer = löschen). Er steht nie in der Konfiguration oder im Protokoll.
#[tauri::command]
pub fn claude_set_key(app: AppHandle, key: String) -> Result<(), String> {
    let key = key.trim();
    let engine = crate::engine(&app);
    if key.is_empty() {
        engine.platform().secret_set(claude::SECRET, None).map_err(|e| e.to_string())?;
        engine.log_system("done", "Anthropic-Key entfernt");
        return Ok(());
    }
    if !key.starts_with("sk-ant-") || key.len() < 30 || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("Das sieht nicht nach einem Anthropic-Key aus (er beginnt mit „sk-ant-“).".into());
    }
    engine.platform().secret_set(claude::SECRET, Some(key)).map_err(|e| e.to_string())?;
    engine.log_system("done", "Anthropic-Key im Windows-Tresor hinterlegt");
    Ok(())
}

#[tauri::command]
pub fn claude_settings(
    app: AppHandle,
    enabled: bool,
    model: String,
    effort: String,
    allow_screen_images: bool,
) -> Result<(), String> {
    if !claude::MODELS.iter().any(|(m, _)| *m == model) {
        return Err("unbekanntes Modell".into());
    }
    if !["low", "medium", "high"].contains(&effort.as_str()) {
        return Err("unbekannte Denktiefe".into());
    }
    let engine = crate::engine(&app);
    if enabled && !claude::has_key(engine.platform().as_ref()) {
        return Err("Erst den Anthropic-Key hinterlegen.".into());
    }
    let mut cfg = engine.config();
    cfg.claude.enabled = enabled;
    cfg.claude.model = model;
    cfg.claude.effort = effort;
    cfg.claude.allow_screen_images = allow_screen_images;
    cfg.save(&config_path()).map_err(|e| e.to_string())?;
    engine.set_config(cfg);
    Ok(())
}

#[tauri::command]
pub fn claude_reset(app: AppHandle) {
    get(&app).reset();
}

/// Verbindung prüfen, ohne etwas zu kosten: Modell-Liste abrufen.
#[tauri::command]
pub async fn claude_test(app: AppHandle) -> Result<String, String> {
    let engine = crate::engine(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let pf = engine.platform().clone();
        let key = pf.secret_get(claude::SECRET).ok().flatten().ok_or("Kein Key hinterlegt.")?;
        let req = HttpRequest {
            method: "GET",
            url: "https://api.anthropic.com/v1/models?limit=1".into(),
            headers: vec![("x-api-key".into(), key), ("anthropic-version".into(), "2026-10-21".into())],
            body: None,
        };
        match pf.http(&req).map_err(|e| e.to_string())? {
            (200, _) => Ok("Verbindung klappt – der Key ist gültig.".to_string()),
            (401 | 403, _) => Err("Der Key wird nicht akzeptiert.".to_string()),
            (s, _) => Err(format!("Anthropic antwortet mit HTTP {s}.")),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Gespeichertes Skript samt eigenem Befehl löschen (Kopie in den Papierkorb).
#[tauri::command]
pub fn script_remove(app: AppHandle, id: String) -> Result<(), String> {
    use panco_core::actions::shell::{saved_path, SavedScripts};
    let engine = crate::engine(&app);
    let data = engine.data_dir();
    let mut all = SavedScripts::load(&data);
    let before = all.list.len();
    all.list.retain(|s| s.id != id);
    if all.list.len() == before {
        return Err("Skript nicht gefunden".into());
    }
    engine.trash().snapshot(&saved_path(&data)).map_err(|e| e.to_string())?;
    all.save(&data)?;
    let mut macros = panco_core::memory::Macros::load(&data);
    macros.list.retain(|m| !m.steps.iter().any(|s| s.action == "shell.saved" && s.params["id"] == id.as_str()));
    macros.save(&data)?;
    engine.log_system("done", "Gespeichertes Skript gelöscht (Kopie im Papierkorb)");
    Ok(())
}

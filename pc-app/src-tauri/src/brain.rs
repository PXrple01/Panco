//! Reiter "Befehle & Gedächtnis": eigene Befehle bearbeiten, Merksätze und Regeln verwalten,
//! Rückfragen pro Aktion abschalten. Alles läuft über dieselben Prüfungen wie per Sprache.

use crate::overlay;
use panco_core::catalog::{self, Tier};
use panco_core::config::config_path;
use panco_core::memory::{self, Macro, Macros, Memory};
use panco_core::Source;
use serde::Serialize;
use tauri::AppHandle;

#[derive(Serialize)]
pub struct MacroRow {
    id: String,
    name: String,
    phrases: Vec<String>,
    steps: Vec<String>,
    /// Höchste Stufe der Schritte (2/3 = fragt einmal nach)
    tier: u8,
}

#[derive(Serialize)]
pub struct Item {
    id: String,
    text: String,
}

#[derive(Serialize)]
pub struct Confirmable {
    id: String,
    title: String,
    skip: bool,
}

#[derive(Serialize)]
pub struct BrainState {
    macros: Vec<MacroRow>,
    facts: Vec<Item>,
    rules: Vec<Item>,
    confirmable: Vec<Confirmable>,
    quiet_secs: u64,
}

fn tier_of(steps: &[memory::Step]) -> u8 {
    steps.iter().filter_map(|s| catalog::get(&s.action)).map(|s| s.tier as u8).max().unwrap_or(1)
}

#[tauri::command]
pub fn brain_state(app: AppHandle) -> BrainState {
    let engine = crate::engine(&app);
    let data = engine.data_dir();
    let mem = Memory::load(&data);
    let skip = engine.config().skip_confirm;
    BrainState {
        macros: Macros::load(&data)
            .list
            .into_iter()
            .map(|m| MacroRow {
                tier: tier_of(&m.steps),
                steps: m.steps.into_iter().map(|s| s.text).collect(),
                id: m.id,
                name: m.name,
                phrases: m.phrases,
            })
            .collect(),
        facts: mem.facts.into_iter().rev().map(|f| Item { id: f.id, text: f.text }).collect(),
        rules: mem.rules.iter().map(|r| Item { id: r.id.clone(), text: r.describe() }).collect(),
        // Nur Stufe 2 lässt sich abschalten – Stufe 3 fragt immer.
        confirmable: catalog::all()
            .iter()
            .filter(|a| a.tier == Tier::Confirm)
            .map(|a| Confirmable { id: a.id.clone(), title: a.title.clone(), skip: skip.contains(&a.id) })
            .collect(),
        quiet_secs: engine.quiet_secs(),
    }
}

/// Eigenen Befehl anlegen oder ändern. Schritte: eine Zeile pro Befehl.
#[tauri::command]
pub fn macro_save(
    app: AppHandle,
    id: Option<String>,
    name: String,
    phrases: Vec<String>,
    steps: String,
) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.chars().count() < 2 || name.chars().count() > 40 {
        return Err("Der Name braucht 2 bis 40 Zeichen.".into());
    }
    if panco_core::parser::parse_text(&name).is_some() {
        return Err(format!("„{name}“ ist schon ein Panco-Befehl – nimm einen anderen Namen."));
    }
    let steps = memory::parse_steps(&steps.replace('\n', ","), 3)?;
    let phrases: Vec<String> =
        phrases.into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).take(5).collect();
    let engine = crate::engine(&app);
    let data = engine.data_dir();
    let mut all = Macros::load(&data);
    let norm = panco_core::normalize::command(&name);
    if all.list.iter().any(|m| panco_core::normalize::command(&m.name) == norm && Some(&m.id) != id.as_ref()) {
        return Err(format!("Einen Befehl „{name}“ gibt es schon."));
    }
    let entry = Macro { id: id.clone().unwrap_or_else(memory::new_id), name: name.clone(), phrases, steps };
    match all.list.iter_mut().find(|m| Some(&m.id) == id.as_ref()) {
        Some(m) => *m = entry,
        None => all.list.push(entry),
    }
    all.save(&data)?;
    engine.log_system("done", &format!("Eigener Befehl „{name}“ gespeichert (Einstellungen)"));
    crate::voice::apply(&app); // Namen in den Vokabular-Hinweis für Whisper
    Ok(())
}

#[tauri::command]
pub fn macro_remove(app: AppHandle, id: String) -> Result<(), String> {
    let engine = crate::engine(&app);
    let data = engine.data_dir();
    let mut all = Macros::load(&data);
    let before = all.list.len();
    all.list.retain(|m| m.id != id);
    if all.list.len() == before {
        return Err("Befehl nicht gefunden".into());
    }
    engine.trash().snapshot(&memory::macros_path(&data)).map_err(|e| e.to_string())?;
    all.save(&data)?;
    engine.log_system("done", "Eigener Befehl gelöscht (Kopie im Papierkorb)");
    Ok(())
}

/// Ausprobieren: läuft wie gesprochen; Rückfragen erscheinen im Overlay.
#[tauri::command]
pub async fn macro_try(app: AppHandle, name: String) {
    let engine = crate::engine(&app);
    let msg = tauri::async_runtime::spawn_blocking(move || engine.command("einst", &name, Source::Text)).await;
    if let Ok(msg) = msg {
        overlay::show(&app, true);
        overlay::emit(&app, &msg);
    }
}

#[tauri::command]
pub fn fact_add(app: AppHandle, text: String) -> Result<String, String> {
    let msg = crate::engine(&app).invoke("einst", "memory.remember", serde_json::json!({ "text": text }), Source::Text);
    match msg {
        panco_core::protocol::ServerMessage::Reply { ok: true, text, .. } => Ok(text),
        other => Err(other.text().to_string()),
    }
}

/// Merksatz oder Regel löschen (vorher Kopie in den Papierkorb).
#[tauri::command]
pub fn memory_remove(app: AppHandle, id: String) -> Result<(), String> {
    let engine = crate::engine(&app);
    let data = engine.data_dir();
    let mut mem = Memory::load(&data);
    let before = mem.facts.len() + mem.rules.len();
    mem.facts.retain(|f| f.id != id);
    mem.rules.retain(|r| r.id != id);
    if mem.facts.len() + mem.rules.len() == before {
        return Err("Eintrag nicht gefunden".into());
    }
    engine.trash().snapshot(&memory::memory_path(&data)).map_err(|e| e.to_string())?;
    mem.save(&data)?;
    engine.log_system("done", "Gedächtnis-Eintrag gelöscht (Kopie im Papierkorb)");
    Ok(())
}

#[tauri::command]
pub fn set_skip_confirm(app: AppHandle, actions: Vec<String>) -> Result<(), String> {
    let actions: Vec<String> =
        actions.into_iter().filter(|a| catalog::get(a).is_some_and(|s| s.tier == Tier::Confirm)).collect();
    let engine = crate::engine(&app);
    let mut cfg = engine.config();
    cfg.skip_confirm = actions;
    cfg.save(&config_path()).map_err(|e| e.to_string())?;
    engine.set_config(cfg);
    Ok(())
}

#[tauri::command]
pub fn set_quiet(app: AppHandle, minutes: u64) {
    crate::engine(&app).set_quiet(minutes.min(120));
}

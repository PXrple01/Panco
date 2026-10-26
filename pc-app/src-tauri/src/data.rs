//! Befehle für die Einstellungs-Reiter "Ordner & Backup", "Notizen", "Protokoll", "Papierkorb".
//! Ändernde Schritte laufen über die Engine (Whitelist, Stufen, Protokoll); Quelle ist "am PC".

use panco_core::backup::{self, BackupInfo};
use panco_core::config::{config_path, FileSettings};
use panco_core::notes;
use panco_core::protocol::ServerMessage;
use panco_core::Source;
use serde::Serialize;
use serde_json::json;
use tauri::AppHandle;

async fn run_blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())
}

// ---------- Ordner & Backup ----------

#[tauri::command]
pub fn files_settings(app: AppHandle) -> FileSettings {
    crate::engine(&app).config().files
}

#[tauri::command]
pub fn files_save(app: AppHandle, files: FileSettings) -> Result<(), String> {
    crate::setup::validate(&files)?;
    let engine = crate::engine(&app);
    let mut cfg = engine.config();
    cfg.files = files;
    cfg.save(&config_path()).map_err(|e| e.to_string())?;
    engine.set_config(cfg);
    engine.log_system("settings", "Ordner-Einstellungen geändert");
    Ok(())
}

#[tauri::command]
pub async fn backup_now(app: AppHandle) -> Result<Vec<String>, String> {
    let engine = crate::engine(&app);
    run_blocking(move || engine.maintenance(true)).await
}

#[tauri::command]
pub fn backups(app: AppHandle) -> Vec<BackupInfo> {
    backup::list(&crate::engine(&app).places().backup)
}

/// Öffnet einen der Panco-Ordner im Explorer (nur diese drei, kein beliebiger Pfad).
#[tauri::command]
pub fn open_place(app: AppHandle, which: String) -> Result<(), String> {
    let engine = crate::engine(&app);
    let places = engine.places();
    let dir = match which.as_str() {
        "trash" => places.trash,
        "backup" => places.backup,
        "notes" => places.notes,
        _ => return Err("unbekannter Ordner".into()),
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    engine.platform().open(&dir.to_string_lossy(), &[]).map_err(|e| e.to_string())
}

// ---------- Notizen (geschützte nur hier, am PC, sichtbar) ----------

#[derive(Serialize)]
pub struct NoteRow {
    title: String,
    sensitive: bool,
    modified: i64,
}

#[tauri::command]
pub fn notes_list(app: AppHandle) -> Vec<NoteRow> {
    notes::list(&crate::engine(&app).places().notes)
        .into_iter()
        .map(|n| NoteRow { title: n.title, sensitive: n.sensitive, modified: n.modified })
        .collect()
}

#[tauri::command]
pub fn note_get(app: AppHandle, title: String) -> Result<String, String> {
    let engine = crate::engine(&app);
    let note =
        notes::list(&engine.places().notes).into_iter().find(|n| n.title == title).ok_or("Notiz nicht gefunden")?;
    if note.sensitive {
        engine.log_system("note_viewed", &format!("Geschützte Notiz „{}“ am PC angezeigt", note.title));
    }
    notes::read(engine.platform().as_ref(), &note)
}

#[tauri::command]
pub fn note_put(app: AppHandle, title: String, content: String) -> Result<(), String> {
    if content.len() > 200_000 {
        return Err("Notiz zu lang".into());
    }
    let engine = crate::engine(&app);
    let note =
        notes::list(&engine.places().notes).into_iter().find(|n| n.title == title).ok_or("Notiz nicht gefunden")?;
    let undo = notes::save(engine.platform().as_ref(), &note, &content, &engine.trash())?;
    engine.log().write(panco_core::log::Entry {
        ts: panco_core::log::ActionLog::now(),
        source: "text".into(),
        action: Some("note.edit".into()),
        params: Some(json!({ "title": note.title })),
        outcome: "done".into(),
        message: "Notiz im Einstellungsfenster bearbeitet".into(),
        undo: Some(undo),
        ..Default::default()
    });
    Ok(())
}

#[tauri::command]
pub async fn note_new(app: AppHandle, title: String, secret: bool) -> Result<ServerMessage, String> {
    let engine = crate::engine(&app);
    run_blocking(move || engine.invoke("s", "note.create", json!({ "title": title, "secret": secret }), Source::Text))
        .await
}

// ---------- Protokoll mit Rückgängig ----------

#[derive(Serialize)]
pub struct LogRow {
    id: String,
    ts: String,
    source: String,
    text: Option<String>,
    action: Option<String>,
    /// Lesbarer Name der Aktion ("Programm beenden")
    title: Option<String>,
    outcome: String,
    message: String,
    /// Was "Rückgängig" tun würde (nur wenn möglich und noch nicht geschehen).
    undo: Option<String>,
    undone: bool,
}

#[tauri::command]
pub async fn log_entries(app: AppHandle, limit: usize) -> Result<Vec<LogRow>, String> {
    let engine = crate::engine(&app);
    run_blocking(move || {
        let entries = engine.log().read_recent(limit.clamp(10, 1000));
        let undone: Vec<String> = entries.iter().filter_map(|e| e.undo_of.clone()).collect();
        entries
            .into_iter()
            .map(|e| {
                let is_undone = undone.contains(&e.id);
                LogRow {
                    undo: e.undo.as_ref().filter(|_| !is_undone).map(|u| u.describe()),
                    undone: is_undone,
                    id: e.id,
                    ts: e.ts,
                    source: e.source,
                    text: e.text,
                    title: e.action.as_deref().and_then(panco_core::catalog::get).map(|s| s.title.clone()),
                    action: e.action,
                    outcome: e.outcome,
                    message: e.message,
                }
            })
            .collect()
    })
    .await
}

#[tauri::command]
pub async fn undo(app: AppHandle, id: String) -> Result<ServerMessage, String> {
    let engine = crate::engine(&app);
    run_blocking(move || engine.invoke("s", "panco.undo", json!({ "id": id }), Source::Text)).await
}

// ---------- Papierkorb ----------

#[derive(Serialize)]
pub struct TrashRow {
    id: String,
    name: String,
    original: String,
    deleted_at: i64,
    size: u64,
    is_dir: bool,
    snapshot: bool,
    days_left: i64,
}

#[tauri::command]
pub fn trash_entries(app: AppHandle) -> Vec<TrashRow> {
    let engine = crate::engine(&app);
    let days = engine.places().trash_days as i64;
    let now = panco_core::time::unix_now();
    engine
        .trash()
        .list()
        .into_iter()
        .map(|e| TrashRow {
            days_left: (days - (now - e.deleted_at) / 86_400).max(0),
            snapshot: e.kind == panco_core::files::trash::TrashKind::Snapshot,
            original: e.original.to_string_lossy().into(),
            id: e.id,
            name: e.name,
            deleted_at: e.deleted_at,
            size: e.size,
            is_dir: e.is_dir,
        })
        .collect()
}

#[tauri::command]
pub async fn trash_restore(app: AppHandle, id: String) -> Result<ServerMessage, String> {
    let engine = crate::engine(&app);
    run_blocking(move || engine.invoke("s", "trash.restore", json!({ "id": id }), Source::Text)).await
}

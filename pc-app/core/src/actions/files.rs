//! Datei-Aktionen: suchen, öffnen, löschen (→ Panco-Papierkorb), verschieben, umbenennen,
//! Ordner anlegen, wiederherstellen. Stufen kommen aus den Ordner-Zonen (files::Places).

use super::*;
use crate::files::search::{self, Hit, Kind, Limits};
use crate::files::trash::Trash;
use crate::files::{is_active_file, move_path, normalize, valid_name, within};
use crate::texts::t;
use crate::undo::UndoOp;
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};

pub struct FileSearch;
pub struct FileOpen;
pub struct FileDelete;
pub struct FileMove;
pub struct FileRename;
pub struct FolderCreate;
pub struct TrashRestore;

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
}

fn folder_of(p: &Path) -> String {
    p.parent().map(name_of).unwrap_or_default()
}

/// "Rechnung.pdf (Downloads, gestern 14:02)"
fn describe(ctx: &Ctx, path: &Path, modified: Option<i64>) -> String {
    let when = modified
        .or_else(|| std::fs::metadata(path).ok()?.modified().ok().map(crate::time::unix))
        .map(|m| format!(", {}", crate::time::relative_de(m, ctx.now, ctx.platform.utc_offset_secs())))
        .unwrap_or_default();
    format!("{} ({}{when})", name_of(path), folder_of(path))
}

fn open_choice(ctx: &Ctx, h: &Hit) -> Choice {
    let when = crate::time::relative_de(h.modified, ctx.now, ctx.platform.utc_offset_secs());
    Choice {
        label: h.name.clone(),
        detail: Some(format!("{} · {when}", folder_of(&h.path))),
        action: "file.open".into(),
        params: json!({ "path": h.path }),
    }
}

/// Suche in den freigegebenen Ordnern (Papierkorb, Backups, Panco-Daten ausgenommen).
fn find(ctx: &Ctx, query: &str, want_dir: bool) -> (search::Query, Vec<Hit>) {
    let mut q = search::parse_query(query, ctx.now, ctx.platform.utc_offset_secs());
    if want_dir {
        q.kind = Some(Kind::Folder);
    }
    let pl = ctx.places;
    let exclude = [pl.trash.clone(), pl.backup.clone(), pl.data.clone()];
    let hits = search::search(&pl.allowed, &exclude, &q, ctx.now, &Limits::default());
    (q, hits)
}

enum Found {
    One(PathBuf),
    /// Wahrscheinlich das hier – nachfragen, Alternativen anbieten.
    Unsure(PathBuf, Vec<Hit>),
}

/// Pfad oder gesprochener Name → Datei/Ordner.
fn resolve(ctx: &Ctx, spec: &str, want_dir: bool) -> Result<Found, ActionError> {
    let spec = clean_text(spec, 400)?;
    let as_path = Path::new(&spec);
    if as_path.is_absolute() {
        let p = normalize(as_path);
        return if p.exists() {
            Ok(Found::One(p))
        } else {
            Err(ActionError::NotFound(t("file_not_found", &[("name", &name_of(&p))])))
        };
    }
    let (q, hits) = find(ctx, &spec, want_dir);
    match hits.as_slice() {
        [] => Err(ActionError::NotFound(t("file_not_found", &[("name", &q.label)]))),
        [only] => Ok(Found::One(only.path.clone())),
        [top, second, ..] if q.latest || top.score - second.score >= 0.3 => Ok(Found::One(top.path.clone())),
        [top, ..] => Ok(Found::Unsure(top.path.clone(), hits.iter().take(5).cloned().collect())),
    }
}

/// Bei unsicherem Treffer: "Meinst du X?" mit Alternativen für dieselbe Aktion.
fn clarify_same(ctx: &Ctx, action: &'static str, top: &Path, hits: &[Hit], extra: Value) -> Plan {
    let with = |p: &Path| {
        let mut v = extra.clone();
        v["path"] = json!(p);
        v
    };
    Plan::Clarify {
        question: t("did_you_mean", &[("name", &describe(ctx, top, None))]),
        params: with(top),
        action: Some(action),
        choices: hits
            .iter()
            .map(|h| Choice { action: action.into(), params: with(&h.path), ..open_choice(ctx, h) })
            .collect(),
    }
}

fn io_err(e: std::io::Error) -> ActionError {
    ActionError::Failed(e.to_string())
}

/// Ordnerangabe ("Dokumente", Pfad, Name eines Unterordners) → Ordner.
fn resolve_dir(ctx: &Ctx, spec: &str) -> Result<PathBuf, ActionError> {
    if let Some(known) = ctx.places.known_folder(spec) {
        return Ok(known);
    }
    match resolve(ctx, spec, true)? {
        Found::One(p) | Found::Unsure(p, _) if p.is_dir() => Ok(p),
        _ => Err(ActionError::NotFound(t("folder_not_found", &[("name", spec)]))),
    }
}

/// Wurzeln (freigegebene Ordner, Benutzerordner) werden nie gelöscht/verschoben.
fn is_root_folder(ctx: &Ctx, p: &Path) -> bool {
    let home = dirs::home_dir().map(|h| normalize(&h));
    ctx.places.allowed.iter().any(|a| normalize(a) == normalize(p))
        || home.is_some_and(|h| h == normalize(p))
        || p.parent().is_none()
}

// ---------- file.search ----------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchP {
    query: String,
    #[serde(default)]
    open: bool,
}

impl Action for FileSearch {
    fn id(&self) -> &'static str {
        "file.search"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: SearchP = super::params(params)?;
        let query = clean_text(&p.query, 200)?;
        let (q, hits) = find(ctx, &query, false);
        let Some(top) = hits.first().cloned() else {
            let text = t("file_search_none", &[("query", &q.label)]);
            return Ok(Plan::instant("Dateisuche", move |_| Ok(Done::new(text, Mood::Confused))));
        };
        let mut choices: Vec<Choice> = hits.iter().take(5).map(|h| open_choice(ctx, h)).collect();
        choices.push(Choice {
            label: t("reveal_label", &[]),
            detail: Some(top.name.clone()),
            action: "file.open".into(),
            params: json!({ "path": top.path, "reveal": true }),
        });
        let clear = hits.len() == 1 || q.latest || hits[0].score - hits[1].score >= 0.3;
        if p.open && clear {
            // "Öffne meine letzte Rechnung" – eindeutig: direkt öffnen, Rest als Auswahl.
            return FileOpen.prepare(&json!({ "path": top.path }), ctx).map(|plan| with_choices(plan, choices));
        }
        Ok(Plan::Clarify {
            question: t("file_search_found", &[("what", &describe(ctx, &top.path, Some(top.modified)))]),
            params: json!({ "path": top.path }),
            action: Some("file.open"),
            choices,
        })
    }
}

/// Auswahl an eine fertige Ausführung hängen.
fn with_choices(plan: Plan, choices: Vec<Choice>) -> Plan {
    match plan {
        Plan::Ready { tier, what, job } => Plan::Ready {
            tier,
            what,
            job: Box::new(move |pf| job(pf).map(|d| if d.choices.is_empty() { d.choices(choices) } else { d })),
        },
        other => other,
    }
}

// ---------- file.open ----------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenP {
    path: String,
    #[serde(default)]
    reveal: bool,
}

impl Action for FileOpen {
    fn id(&self) -> &'static str {
        "file.open"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: OpenP = super::params(params)?;
        let path = match resolve(ctx, &p.path, false)? {
            Found::One(p) => p,
            Found::Unsure(top, hits) => return Ok(clarify_same(ctx, "file.open", &top, &hits, json!({}))),
        };
        let name = name_of(&path);
        if p.reveal {
            return Ok(Plan::instant(format!("{name} im Ordner zeigen"), move |pf| {
                pf.reveal(&path)?;
                Ok(Done::new(t("revealed", &[("name", &name)]), Mood::Info))
            }));
        }
        // Programme/Skripte/Verknüpfungen öffnet Panco nicht per Datei-Befehl (würde Code ausführen).
        if path.is_file() && is_active_file(&path) {
            return Err(ActionError::Failed(t("no_active_files", &[("name", &name)])));
        }
        let tier = ctx.places.read_tier(&path);
        Ok(Plan::ready(tier, format!("{name} öffnen"), move |pf| {
            pf.open(&path.to_string_lossy(), &[])?;
            Ok(Done::new(t("file_opened", &[("name", &name)]), Mood::Success))
        }))
    }
}

// ---------- file.delete ----------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathP {
    path: String,
}

impl Action for FileDelete {
    fn id(&self) -> &'static str {
        "file.delete"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: PathP = super::params(params)?;
        let path = match resolve(ctx, &p.path, false)? {
            Found::One(p) => p,
            Found::Unsure(top, hits) => return Ok(clarify_same(ctx, "file.delete", &top, &hits, json!({}))),
        };
        if is_root_folder(ctx, &path) {
            return Err(ActionError::Failed(t("protected_folder", &[])));
        }
        let tier = ctx.places.write_tier(&path, Tier::Confirm).map_err(ActionError::Failed)?;
        let trash = Trash::new(ctx.places.trash.clone());
        let days = ctx.places.trash_days.to_string();
        let what = t("what_delete", &[("what", &describe(ctx, &path, None))]);
        Ok(Plan::ready(tier, what, move |_| {
            let entry = trash.put(&path).map_err(io_err)?;
            Ok(Done::ok(t("deleted", &[("name", &entry.name), ("days", &days)]))
                .undo(UndoOp::Restore { trash_id: entry.id, name: entry.name }))
        }))
    }
}

// ---------- file.move ----------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveP {
    path: String,
    to: String,
}

impl Action for FileMove {
    fn id(&self) -> &'static str {
        "file.move"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: MoveP = super::params(params)?;
        let src = match resolve(ctx, &p.path, false)? {
            Found::One(p) => p,
            Found::Unsure(top, hits) => {
                return Ok(clarify_same(ctx, "file.move", &top, &hits, json!({ "to": p.to })));
            }
        };
        if is_root_folder(ctx, &src) {
            return Err(ActionError::Failed(t("protected_folder", &[])));
        }
        let dest_dir = resolve_dir(ctx, &p.to)?;
        if within(&dest_dir, &src) {
            return Err(ActionError::Failed(t("move_into_itself", &[])));
        }
        let target = dest_dir.join(src.file_name().unwrap_or_default());
        if target.exists() {
            return Err(ActionError::Failed(t("already_exists", &[("name", &describe(ctx, &target, None))])));
        }
        let pl = ctx.places;
        let tier = pl
            .write_tier(&src, Tier::Confirm)
            .and_then(|a| pl.write_tier(&target, Tier::Confirm).map(|b| a.max(b)))
            .map_err(ActionError::Failed)?;
        let (name, dest_name) = (name_of(&src), name_of(&dest_dir));
        let what = t("what_move", &[("name", &name), ("to", &dest_name)]);
        Ok(Plan::ready(tier, what, move |_| {
            move_path(&src, &target).map_err(io_err)?;
            Ok(Done::ok(t("moved", &[("name", &name), ("to", &dest_name)]))
                .undo(UndoOp::MoveBack { from: target, to: src }))
        }))
    }
}

// ---------- file.rename ----------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameP {
    path: String,
    name: String,
}

impl Action for FileRename {
    fn id(&self) -> &'static str {
        "file.rename"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: RenameP = super::params(params)?;
        let src = match resolve(ctx, &p.path, false)? {
            Found::One(p) => p,
            Found::Unsure(top, hits) => {
                return Ok(clarify_same(ctx, "file.rename", &top, &hits, json!({ "name": p.name })));
            }
        };
        if is_root_folder(ctx, &src) {
            return Err(ActionError::Failed(t("protected_folder", &[])));
        }
        let mut new_name = valid_name(&p.name).map_err(ActionError::InvalidParams)?;
        // Endung behalten, wenn keine neue genannt wurde ("benenne X in Steuer 2025 um").
        if src.is_file() && Path::new(&new_name).extension().is_none() {
            if let Some(ext) = src.extension() {
                new_name = format!("{new_name}.{}", ext.to_string_lossy());
            }
        }
        let target = src.with_file_name(&new_name);
        if target.exists() {
            return Err(ActionError::Failed(t("already_exists", &[("name", &new_name)])));
        }
        let tier = ctx.places.write_tier(&src, Tier::Confirm).map_err(ActionError::Failed)?;
        let old = name_of(&src);
        let what = t("what_rename", &[("name", &old), ("new", &new_name)]);
        Ok(Plan::ready(tier, what, move |_| {
            std::fs::rename(&src, &target).map_err(io_err)?;
            Ok(Done::ok(t("renamed", &[("name", &old), ("new", &new_name)]))
                .undo(UndoOp::MoveBack { from: target, to: src }))
        }))
    }
}

// ---------- folder.create ----------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateP {
    name: String,
    #[serde(default)]
    parent: Option<String>,
}

impl Action for FolderCreate {
    fn id(&self) -> &'static str {
        "folder.create"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: CreateP = super::params(params)?;
        let name = valid_name(&p.name).map_err(ActionError::InvalidParams)?;
        let parent = match p.parent.as_deref().filter(|s| !s.trim().is_empty()) {
            Some(spec) => resolve_dir(ctx, spec)?,
            None => ctx.places.known_folder("dokumente").unwrap_or_else(|| ctx.places.notes.clone()),
        };
        let target = parent.join(&name);
        if target.exists() {
            return Err(ActionError::Failed(t("already_exists", &[("name", &name)])));
        }
        let tier = ctx.places.write_tier(&target, Tier::Instant).map_err(ActionError::Failed)?;
        let parent_name = name_of(&parent);
        let what = t("what_mkdir", &[("name", &name), ("in", &parent_name)]);
        Ok(Plan::ready(tier, what, move |_| {
            std::fs::create_dir_all(&target).map_err(io_err)?;
            Ok(Done::ok(t("mkdir_done", &[("name", &name), ("in", &parent_name)])).undo(UndoOp::Trash { path: target }))
        }))
    }
}

// ---------- trash.restore ----------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreP {
    id: String,
}

impl Action for TrashRestore {
    fn id(&self) -> &'static str {
        "trash.restore"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: RestoreP = super::params(params)?;
        let trash = Trash::new(ctx.places.trash.clone());
        let entry = trash.get(&p.id).ok_or_else(|| ActionError::NotFound(t("trash_missing", &[])))?;
        let tier = ctx.places.write_tier(&entry.original, Tier::Instant).map_err(ActionError::Failed)?;
        let name = entry.name.clone();
        Ok(Plan::ready(tier, format!("{name} wiederherstellen"), move |_| {
            let at = trash.restore(&p.id).map_err(io_err)?;
            Ok(Done::ok(t("restored", &[("name", &name), ("folder", &folder_of(&at))]))
                .undo(UndoOp::Trash { path: at }))
        }))
    }
}

// ---------- panco.undo ----------

pub struct Undo;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UndoP {
    #[serde(default)]
    id: Option<String>,
}

impl Action for Undo {
    fn id(&self) -> &'static str {
        "panco.undo"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: UndoP = super::params(params)?;
        let entry = match p.id.as_deref() {
            Some(id) => ctx.log.find(id).ok_or_else(|| ActionError::NotFound(t("undo_nothing", &[])))?,
            None => ctx.log.last_undoable().ok_or_else(|| ActionError::NotFound(t("undo_nothing", &[])))?,
        };
        let op = entry.undo.clone().ok_or_else(|| ActionError::NotFound(t("undo_impossible", &[])))?;
        if ctx.log.is_undone(&entry.id) {
            return Err(ActionError::NotFound(t("undo_already", &[])));
        }
        let trash = Trash::new(ctx.places.trash.clone());
        let id = entry.id.clone();
        Ok(Plan::instant(op.describe(), move |pf| {
            let text = op.apply(pf, &trash).map_err(ActionError::Failed)?;
            let mut done = Done::ok(text);
            done.undo_of = Some(id);
            Ok(done)
        }))
    }
}

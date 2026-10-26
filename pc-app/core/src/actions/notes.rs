//! note.create / note.append / note.read / note.search
//! Inhalte werden im Protokoll geschwärzt (`redact` im Katalog). Geschützte Notizen liest Panco
//! nie vor und gibt sie nie heraus – nur das Einstellungsfenster am PC zeigt sie an.

use super::*;
use crate::files::trash::Trash;
use crate::notes::{self, Note};
use crate::texts::t;
use crate::undo::UndoOp;
use serde::Deserialize;
use serde_json::json;

pub struct NoteCreate;
pub struct NoteAppend;
pub struct NoteRead;
pub struct NoteSearch;

/// Höchstlänge einer vorgelesenen/angezeigten Antwort (Protokoll-Schema: 300 Zeichen).
const READ_MAX: usize = 260;

fn err(e: String) -> ActionError {
    ActionError::Failed(e)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateP {
    title: String,
    #[serde(default)]
    text: Option<String>,
    /// Ausdrücklich verschlüsseln ("geheime Notiz").
    #[serde(default)]
    secret: bool,
}

impl Action for NoteCreate {
    fn id(&self) -> &'static str {
        "note.create"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: CreateP = super::params(params)?;
        let title = clean_text(&p.title, 80)?;
        let text = p.text.as_deref().map(|s| clean_text(s, 2000)).transpose()?.unwrap_or_default();
        let dir = ctx.places.notes.clone();
        let trash = Trash::new(ctx.places.trash.clone());
        // Gibt es die Notiz schon und kommt Text mit? Dann anhängen statt Fehler.
        if let Some(existing) = notes::list(&dir).into_iter().find(|n| n.title.eq_ignore_ascii_case(&title)) {
            if !text.is_empty() {
                return Ok(append_plan(existing, text, trash));
            }
            return Err(ActionError::Failed(t("note_exists", &[("title", &existing.title)])));
        }
        Ok(Plan::instant(format!("Notiz {title} anlegen"), move |pf| {
            let note = notes::create(pf, &dir, &title, &text, p.secret).map_err(err)?;
            let key = if note.sensitive { "note_created_secret" } else { "note_created" };
            Ok(Done::ok(t(key, &[("title", &note.title)])).undo(UndoOp::Trash { path: note.path }))
        }))
    }
}

fn append_plan(note: Note, text: String, trash: Trash) -> Plan {
    Plan::instant(format!("in Notiz {} schreiben", note.title), move |pf| {
        let undo = notes::append(pf, &note, &text, &trash).map_err(err)?;
        Ok(Done::ok(t("note_appended", &[("title", &note.title)])).undo(undo))
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppendP {
    text: String,
    #[serde(default)]
    title: Option<String>,
}

impl Action for NoteAppend {
    fn id(&self) -> &'static str {
        "note.append"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: AppendP = super::params(params)?;
        let text = clean_text(&p.text, 2000)?;
        let dir = ctx.places.notes.clone();
        let trash = Trash::new(ctx.places.trash.clone());
        let note = match p.title.as_deref().filter(|s| !s.trim().is_empty()) {
            Some(title) => notes::find(&dir, title)
                .ok_or_else(|| ActionError::NotFound(t("note_not_found", &[("title", title)])))?,
            // "Notiz: …" ohne Titel → in die zuletzt benutzte Notiz, sonst neue Schnellnotiz
            None => match notes::list(&dir).into_iter().next() {
                Some(n) => n,
                None => {
                    return NoteCreate.prepare(&json!({ "title": t("quick_note_title", &[]), "text": text }), ctx);
                }
            },
        };
        Ok(append_plan(note, text, trash))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadP {
    #[serde(default)]
    title: Option<String>,
}

impl Action for NoteRead {
    fn id(&self) -> &'static str {
        "note.read"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: ReadP = super::params(params)?;
        let dir = ctx.places.notes.clone();
        let note = match p.title.as_deref().filter(|s| !s.trim().is_empty()) {
            Some(title) => notes::find(&dir, title)
                .ok_or_else(|| ActionError::NotFound(t("note_not_found", &[("title", title)])))?,
            None => notes::list(&dir).into_iter().next().ok_or_else(|| ActionError::NotFound(t("no_notes", &[])))?,
        };
        if note.sensitive {
            // Nie vorlesen, nie an iPhone/Claude – nur im Einstellungsfenster am PC.
            return Err(ActionError::Failed(t("note_protected", &[("title", &note.title)])));
        }
        Ok(Plan::instant(format!("Notiz {} vorlesen", note.title), move |pf| {
            let body = notes::body(&notes::read(pf, &note).map_err(err)?);
            let body =
                if body.is_empty() { t("note_empty", &[]) } else { body.replace("\n- ", ", ").replace("- ", "") };
            let max = READ_MAX.saturating_sub(note.title.chars().count()).max(40);
            let short: String = body.chars().take(max).collect();
            let more = if body.chars().count() > max { " …" } else { "" };
            Ok(Done::new(format!("{}: {short}{more}", note.title), Mood::Info))
        }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchP {
    #[serde(default)]
    query: Option<String>,
}

impl Action for NoteSearch {
    fn id(&self) -> &'static str {
        "note.search"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: SearchP = super::params(params)?;
        let query = p.query.as_deref().map(|q| clean_text(q, 200)).transpose()?.unwrap_or_default();
        let dir = ctx.places.notes.clone();
        Ok(Plan::instant("Notizen durchsuchen", move |pf| {
            let needle = crate::normalize::name(&query);
            let found: Vec<Note> = notes::list(&dir)
                .into_iter()
                .filter(|n| {
                    needle.is_empty()
                        || crate::normalize::name(&n.title).contains(&needle)
                        // Inhalte nur bei ungeschützten Notizen durchsuchen
                        || (!n.sensitive
                            && notes::read(pf, n).map(|c| crate::normalize::name(&c).contains(&needle)).unwrap_or(false))
                })
                .take(8)
                .collect();
            if found.is_empty() {
                return Ok(Done::new(t("notes_none", &[("query", &query)]), Mood::Confused));
            }
            let names: Vec<String> = found.iter().map(|n| n.title.clone()).collect();
            let choices = found
                .iter()
                .map(|n| Choice {
                    label: if n.sensitive { format!("🔒 {}", n.title) } else { n.title.clone() },
                    detail: None,
                    action: "note.read".into(),
                    params: json!({ "title": n.title }),
                })
                .collect();
            Ok(Done::new(t("notes_found", &[("n", &found.len().to_string()), ("list", &names.join(", "))]), Mood::Info)
                .choices(choices))
        }))
    }
}

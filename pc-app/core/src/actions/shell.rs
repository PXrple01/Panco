//! Geprüfte Shell (Etappe 7): shell.run, shell.test, shell.save, shell.saved.
//! Die Einstufung macht `crate::shell`; hier passiert, was daraus folgt:
//! - nur lesen → sofort
//! - ändern in freigegebenen Ordnern → sofort, vorher Kopie in den Papierkorb (rückgängig machbar)
//! - riskant → Stufe 3: Rückfrage am PC mit Beschreibung + Skript, vorher Wiederherstellungspunkt,
//!   optional Testlauf in der Windows-Sandbox
//! - verboten → nie

use super::*;
use crate::shell::{self, Analysis, Class};
use crate::texts::t;
use crate::undo::UndoOp;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

pub struct ShellRun;
pub struct ShellTest;
pub struct ShellSave;
pub struct ShellSaved;

/// Größer als das wird nicht gesichert → dann gilt das Skript als riskant.
const MAX_BACKUP: u64 = 500 << 20;
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

/// Zuletzt geprüfte Skripte (für den Sandbox-Testlauf per Klick) und das letzte erfolgreiche.
static RECENT: Mutex<VecDeque<(String, String, String)>> = Mutex::new(VecDeque::new());
static LAST_OK: Mutex<Option<(String, String)>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SavedScript {
    pub id: String,
    pub name: String,
    pub purpose: String,
    pub script: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SavedScripts {
    pub list: Vec<SavedScript>,
}

pub fn saved_path(data: &Path) -> PathBuf {
    data.join("skripte.json")
}

impl SavedScripts {
    pub fn load(data: &Path) -> Self {
        std::fs::read(saved_path(data)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self, data: &Path) -> Result<(), String> {
        std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
        std::fs::write(saved_path(data), serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
}

/// Arbeitsordner für Skripte: der erste freigegebene Ordner (relative Pfade landen dort).
fn work_dir(ctx: &Ctx) -> PathBuf {
    ctx.places.allowed.first().cloned().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."))
}

/// Panco-Analyse + Gegenprobe mit PowerShells Parser (unter Windows).
pub fn check(script: &str, ctx: &Ctx) -> Analysis {
    let mut a = shell::analyze(script, ctx.places, &work_dir(ctx));
    match ctx.platform.powershell_parse(script) {
        Ok(parsed) => shell::cross_check(&mut a, &parsed),
        Err(PlatformError::Unsupported) => {}
        Err(e) => {
            let mut b = a.clone();
            shell::cross_check(&mut b, &crate::platform::ParsedScript { errors: 1, commands: vec![], dynamic: 0 });
            b.reasons.push(format!("Gegenprobe nicht möglich ({e})"));
            a = b;
        }
    }
    a
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunP {
    script: String,
    purpose: String,
}

impl Action for ShellRun {
    fn id(&self) -> &'static str {
        "shell.run"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: RunP = super::params(params)?;
        let purpose = clean_text(&p.purpose, 200)?;
        plan_script(p.script, purpose, ctx, true)
    }
}

/// Plan für ein Skript (von Claude oder gespeichert).
fn plan_script(script: String, purpose: String, ctx: &Ctx, offer_save: bool) -> Result<Plan, ActionError> {
    let mut a = check(&script, ctx);
    if ctx.tainted && a.class != Class::ReadOnly {
        return Err(ActionError::NotFound(t("shell_tainted", &[])));
    }
    if a.class == Class::Forbidden {
        return Err(ActionError::NotFound(t("shell_forbidden", &[("why", &a.describe())])));
    }
    // Sicherungskopien nur in vernünftiger Größe – sonst lieber nachfragen.
    let existing: u64 = a.writes.iter().filter(|w| w.exists()).map(|w| crate::files::size_of(w)).sum();
    if existing > MAX_BACKUP && a.class == Class::ChangeAllowed {
        a.class = Class::Risky;
        a.reasons.push(t("shell_too_big_backup", &[]));
    }
    let id = crate::memory::new_id();
    {
        let mut recent = RECENT.lock().unwrap();
        recent.push_back((id.clone(), script.clone(), purpose.clone()));
        while recent.len() > 5 {
            recent.pop_front();
        }
    }
    let cwd = work_dir(ctx);
    let dir = ctx.data_dir.join("skripte");
    let trash = crate::files::trash::Trash::new(ctx.places.trash.clone());
    let writes = a.writes.clone();
    let risky = a.class == Class::Risky;
    let class = a.class;
    let shown = purpose.clone();
    let shown_script = script.clone();
    let job = move |pf: &dyn crate::platform::Platform| -> Result<Done, ActionError> {
        let mut notes = Vec::new();
        if risky {
            match pf.restore_point(&format!("Panco: {purpose}")) {
                Ok(()) => notes.push(t("shell_restore_point", &[])),
                Err(e) => notes.push(t("shell_restore_point_failed", &[("why", &e.to_string())])),
            }
        }
        // Vorher sichern: vorhandene Ziele kopieren, neue merken (zum Wegräumen beim Rückgängig).
        let mut undo = Vec::new();
        let mut new_paths = Vec::new();
        for w in &writes {
            if w.exists() {
                let e = trash.snapshot(w).map_err(|e| ActionError::Failed(format!("Sicherung fehlgeschlagen: {e}")))?;
                undo.push(UndoOp::Restore { trash_id: e.id, name: e.name });
            } else {
                new_paths.push(w.clone());
            }
        }
        let out = run_script(pf, &dir, &cwd, &script)?;
        for p in new_paths.into_iter().filter(|p| p.exists()) {
            undo.push(UndoOp::Trash { path: p });
        }
        let mut data = format!("Exit-Code {}\n{}", out.code, out.stdout.chars().take(20_000).collect::<String>());
        if !out.stderr.trim().is_empty() {
            data.push_str(&format!("\nFehlerausgabe:\n{}", out.stderr.chars().take(4000).collect::<String>()));
        }
        if !notes.is_empty() {
            data.push_str(&format!("\n{}", notes.join(" ")));
        }
        let text = if out.timed_out {
            t("shell_timeout", &[])
        } else if out.code != 0 {
            t("shell_failed", &[("code", &out.code.to_string())])
        } else {
            *LAST_OK.lock().unwrap() = Some((script.clone(), purpose.clone()));
            if offer_save {
                data.push_str(&format!(
                    "\n(Hinweis für dich: Der Nutzer kann das Skript mit „Speicher das als Befehl <Name>“ speichern. {})",
                    t("shell_offer_save", &[])
                ));
            }
            let first = out.stdout.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            if first.is_empty() {
                t("shell_done", &[])
            } else {
                format!("{} {}", t("shell_done", &[]), first.chars().take(150).collect::<String>())
            }
        };
        let mut done = Done::new(text, if out.code == 0 { Mood::Success } else { Mood::Error }).data(data);
        if !undo.is_empty() {
            done = done.undo(if undo.len() == 1 { undo.pop().unwrap() } else { UndoOp::Batch { ops: undo } });
        }
        if out.code != 0 || out.timed_out {
            return Err(ActionError::Failed(done.data.unwrap_or_default().chars().take(300).collect()));
        }
        Ok(done)
    };
    Ok(match class {
        Class::ReadOnly | Class::ChangeAllowed => Plan::instant(format!("Skript: {shown}"), job),
        _ => {
            let what = t("shell_what", &[("purpose", &shown), ("why", &a.describe())]);
            let detail = format!(
                "{shown_script}\n\n— Panco hat geprüft: {} —\n{}\nVorher legt Panco einen Wiederherstellungspunkt an (wenn Windows es erlaubt).",
                a.class.label(),
                a.reasons.iter().map(|r| format!("• {r}")).collect::<Vec<_>>().join("\n")
            );
            let mut plan = Plan::ready(Tier::PcOnly, what, job).with_detail(detail);
            if ctx.platform.sandbox_available() {
                plan = plan.with_choices(vec![crate::protocol::Choice {
                    label: "Erst in der Sandbox testen".into(),
                    detail: Some("zeigt, was sich ändern würde".into()),
                    action: "shell.test".into(),
                    params: serde_json::json!({ "id": id }),
                }]);
            }
            plan
        }
    })
}

/// Skript als Datei (UTF-8 mit BOM, damit Windows PowerShell 5.1 Umlaute richtig liest) ablegen,
/// ausführen, wieder löschen.
fn run_script(
    pf: &dyn crate::platform::Platform,
    dir: &Path,
    cwd: &Path,
    script: &str,
) -> Result<crate::platform::ScriptOutput, ActionError> {
    std::fs::create_dir_all(dir).map_err(|e| ActionError::Failed(e.to_string()))?;
    let file = dir.join(format!("lauf-{}.ps1", crate::memory::new_id()));
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(script.as_bytes());
    std::fs::write(&file, bytes).map_err(|e| ActionError::Failed(e.to_string()))?;
    let out = pf.powershell(&file, cwd, RUN_TIMEOUT);
    let _ = std::fs::remove_file(&file);
    Ok(out?)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdP {
    id: String,
}

impl Action for ShellTest {
    fn id(&self) -> &'static str {
        "shell.test"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: IdP = super::params(params)?;
        let script = RECENT
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _, _)| *id == p.id)
            .map(|(_, s, _)| s.clone())
            .ok_or_else(|| ActionError::NotFound(t("shell_test_unknown", &[])))?;
        if !ctx.platform.sandbox_available() {
            return Err(ActionError::NotFound(t("shell_test_unavailable", &[])));
        }
        Ok(Plan::instant(t("shell_test_running", &[]), move |pf| {
            let report = pf.sandbox_run(&script, Duration::from_secs(300))?;
            let first: String = report.lines().take(3).collect::<Vec<_>>().join(" ");
            Ok(Done::new(format!("Testlauf fertig: {}", first.chars().take(250).collect::<String>()), Mood::Info)
                .data(report))
        }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NameP {
    name: String,
}

impl Action for ShellSave {
    fn id(&self) -> &'static str {
        "shell.save"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: NameP = super::params(params)?;
        let name = clean_text(&p.name, 40)?;
        let (script, purpose) =
            LAST_OK.lock().unwrap().clone().ok_or_else(|| ActionError::NotFound(t("shell_nothing_to_save", &[])))?;
        if crate::parser::parse_text(&name).is_some() {
            return Err(ActionError::NotFound(t("macro_name_taken", &[("name", &name)])));
        }
        let data = ctx.data_dir.to_path_buf();
        Ok(Plan::instant(format!("Skript als „{name}“ speichern"), move |_| {
            let mut macros = crate::memory::Macros::load(&data);
            if macros.find(&crate::normalize::command(&name)).is_some() {
                return Err(ActionError::NotFound(t("macro_name_taken", &[("name", &name)])));
            }
            let mut saved = SavedScripts::load(&data);
            let id = crate::memory::new_id();
            saved.list.push(SavedScript { id: id.clone(), name: name.clone(), purpose: purpose.clone(), script });
            saved.save(&data).map_err(ActionError::Failed)?;
            let m = crate::memory::Macro {
                id: crate::memory::new_id(),
                name: name.clone(),
                phrases: Vec::new(),
                steps: vec![crate::memory::Step {
                    action: "shell.saved".into(),
                    params: serde_json::json!({ "id": id }),
                    text: purpose,
                }],
            };
            let macro_id = m.id.clone();
            macros.list.push(m);
            macros.save(&data).map_err(ActionError::Failed)?;
            Ok(Done::ok(t("shell_saved", &[("name", &name)]))
                .undo(UndoOp::MemoryRemove { file: crate::memory::macros_path(&data), id: macro_id }))
        }))
    }
}

impl Action for ShellSaved {
    fn id(&self) -> &'static str {
        "shell.saved"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: IdP = super::params(params)?;
        let s = SavedScripts::load(ctx.data_dir)
            .list
            .into_iter()
            .find(|s| s.id == p.id)
            .ok_or_else(|| ActionError::NotFound("Dieses gespeicherte Skript gibt es nicht mehr.".into()))?;
        // Jedes Mal neu prüfen – Ordner-Freigaben können sich geändert haben.
        plan_script(s.script, s.purpose, ctx, false)
    }
}

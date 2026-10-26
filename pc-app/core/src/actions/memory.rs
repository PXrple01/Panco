//! Gedächtnis und eigene Befehle:
//! memory.remember / memory.list / memory.forget / memory.rule – Merksätze und Vorlieben-Regeln
//! macro.create / macro.run / macro.delete – eigene Befehle aus Whitelist-Schritten

use super::*;
use crate::files::trash::Trash;
use crate::memory::{self, Fact, Macro, Macros, Memory, Rule, Trigger};
use crate::texts::t;
use crate::undo::UndoOp;
use serde::Deserialize;

pub struct Remember;
pub struct List;
pub struct Forget;
pub struct RuleAdd;
pub struct MacroCreate;
pub struct MacroRun;
pub struct MacroDelete;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextP {
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ForgetP {
    text: Option<String>,
    all: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleP {
    when: String,
    then: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MacroP {
    name: String,
    steps: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NameP {
    name: String,
}

fn not_found(text: String) -> ActionError {
    ActionError::NotFound(text)
}

impl Action for Remember {
    fn id(&self) -> &'static str {
        "memory.remember"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: TextP = super::params(params)?;
        let text = clean_text(&p.text, 300)?;
        // Passwörter & Co. gehören in eine geschützte Notiz, nicht ins Gedächtnis (das bekommt Claude).
        if crate::notes::is_sensitive_title(&text) {
            return Err(not_found(t("memory_sensitive", &[])));
        }
        let data = ctx.places.data.clone();
        let now = ctx.now;
        Ok(Plan::instant("Merken", move |_| {
            let mut m = Memory::load(&data);
            let id = memory::new_id();
            m.facts.push(Fact { id: id.clone(), text: text.clone(), created: now });
            m.save(&data).map_err(ActionError::Failed)?;
            Ok(Done::new(t("memory_saved", &[("text", &text)]), Mood::Success)
                .undo(UndoOp::MemoryRemove { file: memory::memory_path(&data), id }))
        }))
    }
}

impl Action for List {
    fn id(&self) -> &'static str {
        "memory.list"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        no_params(params)?;
        let m = Memory::load(&ctx.places.data);
        let macros = Macros::load(&ctx.places.data);
        let mut parts = Vec::new();
        if !m.facts.is_empty() {
            let facts: Vec<&str> = m.facts.iter().rev().take(5).map(|f| f.text.as_str()).collect();
            parts.push(t("memory_list_facts", &[("list", &facts.join("; "))]));
        }
        if !m.rules.is_empty() {
            let rules: Vec<String> = m.rules.iter().take(4).map(Rule::describe).collect();
            parts.push(t("memory_list_rules", &[("list", &rules.join("; "))]));
        }
        if !macros.list.is_empty() {
            let names: Vec<&str> = macros.list.iter().map(|m| m.name.as_str()).collect();
            parts.push(t("memory_list_macros", &[("list", &names.join(", "))]));
        }
        let text = if parts.is_empty() { t("memory_empty", &[]) } else { parts.join(" ") };
        Ok(Plan::instant("Gedächtnis zeigen", move |_| Ok(Done::new(text, Mood::Info))))
    }
}

impl Action for Forget {
    fn id(&self) -> &'static str {
        "memory.forget"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: ForgetP = super::params(params)?;
        let data = ctx.places.data.clone();
        let trash = ctx.places.trash.clone();
        let m = Memory::load(&data);
        if p.all == Some(true) {
            if m.facts.is_empty() && m.rules.is_empty() {
                return Err(not_found(t("memory_empty", &[])));
            }
            let what = t(
                "memory_forget_all_q",
                &[("facts", &m.facts.len().to_string()), ("rules", &m.rules.len().to_string())],
            );
            return Ok(Plan::confirm(what, move |_| {
                // Vorher eine Kopie in den Panco-Papierkorb – rückgängig machbar.
                let snap = Trash::new(trash)
                    .snapshot(&memory::memory_path(&data))
                    .map_err(|e| ActionError::Failed(e.to_string()))?;
                Memory::default().save(&data).map_err(ActionError::Failed)?;
                Ok(Done::new(t("memory_forgot_all", &[]), Mood::Info)
                    .undo(UndoOp::Restore { trash_id: snap.id, name: snap.name }))
            }));
        }
        let query = clean_text(p.text.as_deref().unwrap_or(""), 200)?;
        // "dem Lieblingsspiel" → "Lieblingsspiel"
        let mut words: Vec<&str> = query.split_whitespace().collect();
        while words.len() > 1
            && ["der", "die", "das", "dem", "den", "mein", "meine", "meinem", "meinen", "meiner"]
                .contains(&words[0].to_lowercase().as_str())
        {
            words.remove(0);
        }
        let query = words.join(" ");
        // Merksätze und Regeln gemeinsam unscharf durchsuchen
        let items: Vec<(String, String)> = m
            .facts
            .iter()
            .map(|f| (f.id.clone(), f.text.clone()))
            .chain(m.rules.iter().map(|r| (r.id.clone(), r.describe())))
            .collect();
        let hit = match crate::fuzzy::resolve(&query, &items, |i| vec![i.1.as_str()]) {
            crate::fuzzy::Resolved::Sure(i) | crate::fuzzy::Resolved::Unsure(i) => Some(i.clone()),
            crate::fuzzy::Resolved::None => {
                let q = crate::normalize::name(&query);
                items.iter().find(|i| crate::normalize::name(&i.1).contains(&q)).cloned()
            }
        };
        let (id, label) = hit.ok_or_else(|| not_found(t("memory_forget_none", &[("text", &query)])))?;
        Ok(Plan::instant("Vergessen", move |_| {
            let snap = Trash::new(trash)
                .snapshot(&memory::memory_path(&data))
                .map_err(|e| ActionError::Failed(e.to_string()))?;
            let mut m = Memory::load(&data);
            m.facts.retain(|f| f.id != id);
            m.rules.retain(|r| r.id != id);
            m.save(&data).map_err(ActionError::Failed)?;
            Ok(Done::new(t("memory_forgot", &[("text", &label)]), Mood::Info)
                .undo(UndoOp::Restore { trash_id: snap.id, name: snap.name }))
        }))
    }
}

impl Action for RuleAdd {
    fn id(&self) -> &'static str {
        "memory.rule"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: RuleP = super::params(params)?;
        let when = clean_text(&p.when, 80)?;
        let then = clean_text(&p.then, 300)?;
        // Regeln laufen von selbst – deshalb nur Schritte, die keine Rückfrage brauchen.
        let steps = memory::parse_steps(&then, 1).map_err(|e| not_found(t("rule_invalid", &[("why", &e)])))?;
        let w = crate::normalize::name(&when);
        let trigger = if ["ein spiel", "irgendein spiel", "ein game", "spiel", "spiele", "zocke"].contains(&w.as_str())
        {
            Trigger::GameStart { game: None }
        } else {
            match crate::fuzzy::resolve(&when, ctx.games, |g| vec![g.name.as_str()]) {
                crate::fuzzy::Resolved::Sure(g) => Trigger::GameStart { game: Some(g.name.clone()) },
                _ => Trigger::AppStart { target: w },
            }
        };
        let rule = Rule { id: memory::new_id(), when: trigger, steps, created: ctx.now };
        let data = ctx.places.data.clone();
        Ok(Plan::instant("Regel merken", move |_| {
            let mut m = Memory::load(&data);
            let text = t("rule_saved", &[("rule", &rule.describe())]);
            let id = rule.id.clone();
            m.rules.push(rule);
            m.save(&data).map_err(ActionError::Failed)?;
            Ok(Done::new(text, Mood::Success).undo(UndoOp::MemoryRemove { file: memory::memory_path(&data), id }))
        }))
    }
}

impl Action for MacroCreate {
    fn id(&self) -> &'static str {
        "macro.create"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: MacroP = super::params(params)?;
        let name = clean_text(&p.name, 40)?;
        if name.chars().count() < 2 {
            return Err(ActionError::InvalidParams("Name zu kurz".into()));
        }
        // Ein eigener Befehl darf keinen eingebauten überdecken ("lauter" bleibt "lauter").
        if crate::parser::parse_text(&name).is_some() {
            return Err(not_found(t("macro_name_taken", &[("name", &name)])));
        }
        let steps = memory::parse_steps(&p.steps, 3).map_err(|e| not_found(t("macro_invalid", &[("why", &e)])))?;
        let data = ctx.places.data.clone();
        let list = steps.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(", ");
        let text = t("macro_saved", &[("name", &name), ("list", &list)]);
        Ok(Plan::instant(format!("Befehl „{name}“ anlegen"), move |_| {
            let mut all = Macros::load(&data);
            let norm = crate::normalize::command(&name);
            all.list.retain(|m| crate::normalize::command(&m.name) != norm); // gleichnamigen ersetzen
            let id = memory::new_id();
            all.list.push(Macro { id: id.clone(), name, phrases: Vec::new(), steps });
            all.save(&data).map_err(ActionError::Failed)?;
            Ok(Done::new(text, Mood::Success).undo(UndoOp::MemoryRemove { file: memory::macros_path(&data), id }))
        }))
    }
}

impl Action for MacroRun {
    fn id(&self) -> &'static str {
        "macro.run"
    }
    /// Alle Schritte vorbereiten; der Befehl hat die höchste Stufe seiner Schritte. Braucht einer eine
    /// Bestätigung, wird der ganze Ablauf einmal gezeigt und bestätigt.
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: NameP = super::params(params)?;
        let all = Macros::load(&ctx.places.data);
        let m = all
            .find(&crate::normalize::command(&p.name))
            .cloned()
            .ok_or_else(|| not_found(t("macro_unknown", &[("name", &p.name)])))?;
        let registry = super::registry();
        let mut tier = Tier::Instant;
        let mut whats = Vec::new();
        let mut jobs = Vec::new();
        for (i, step) in m.steps.iter().enumerate() {
            let spec = crate::catalog::get(&step.action)
                .ok_or_else(|| not_found(t("macro_step_bad", &[("n", &(i + 1).to_string())])))?;
            if ctx.source.is_remote() && !spec.remote {
                return Err(not_found(t("remote_forbidden", &[])));
            }
            let handler = registry.iter().find(|a| a.id() == step.action).ok_or(ActionError::Unsupported)?;
            match handler.prepare(&step.params, ctx)? {
                Plan::Ready { tier: st, what, job, .. } => {
                    tier = tier.max(st).max(spec.tier);
                    whats.push(what);
                    jobs.push(job);
                }
                Plan::Clarify { .. } => {
                    return Err(not_found(t(
                        "macro_step_unclear",
                        &[("n", &(i + 1).to_string()), ("step", &step.text)],
                    )))
                }
            }
        }
        let what = t("macro_what", &[("name", &m.name), ("list", &whats.join(", "))]);
        Ok(Plan::ready(tier, what, move |pf| {
            let mut texts = Vec::new();
            let mut undos = Vec::new();
            for (i, job) in jobs.into_iter().enumerate() {
                match job(pf) {
                    Ok(done) => {
                        texts.push(done.text);
                        undos.extend(done.undo);
                    }
                    Err(e) => {
                        texts.push(t("macro_step_failed", &[("n", &(i + 1).to_string()), ("why", &e.to_string())]));
                        break;
                    }
                }
            }
            let done = Done::new(texts.join(" "), Mood::Success);
            // Rückgängig nur, wenn genau ein Schritt umkehrbar war (sonst wäre es unklar).
            Ok(if undos.len() == 1 { done.undo(undos.remove(0)) } else { done })
        }))
    }
}

impl Action for MacroDelete {
    fn id(&self) -> &'static str {
        "macro.delete"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: NameP = super::params(params)?;
        let data = ctx.places.data.clone();
        let trash = ctx.places.trash.clone();
        let all = Macros::load(&data);
        let m = all
            .find(&crate::normalize::command(&p.name))
            .cloned()
            .ok_or_else(|| not_found(t("macro_unknown", &[("name", &p.name)])))?;
        Ok(Plan::instant(format!("Befehl „{}“ löschen", m.name), move |_| {
            let snap = Trash::new(trash)
                .snapshot(&memory::macros_path(&data))
                .map_err(|e| ActionError::Failed(e.to_string()))?;
            let mut all = Macros::load(&data);
            all.list.retain(|x| x.id != m.id);
            all.save(&data).map_err(ActionError::Failed)?;
            Ok(Done::new(t("macro_deleted", &[("name", &m.name)]), Mood::Info)
                .undo(UndoOp::Restore { trash_id: snap.id, name: snap.name }))
        }))
    }
}

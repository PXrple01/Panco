//! Engine: verbindet Parser, Whitelist, Stufen, Bestätigungen, Rückgängig und Protokoll.
//!
//! Sicherheitsregeln (siehe CLAUDE.md), alle hier durchgesetzt:
//! - Not-Aus aus → nichts wird ausgeführt.
//! - Nur Aktionen aus dem Katalog; Remote nur, wenn `remote: true`.
//! - Stufe 1 sofort; Stufe 2 Bestätigung (Sprache/Overlay); Stufe 3 nur Klick am PC, nie remote.
//! - Anfragen vom iPhone oder von Claude: Stufe 2/3 immer per Klick am PC bestätigen.
//! - Jede Entscheidung landet im Protokoll; sensible Inhalte (`redact`) nur geschwärzt.

use crate::actions::{self, Action, ActionError, Ctx, Done, Job, Plan};
use crate::catalog::{self, ActionSpec, Tier};
use crate::config::Config;
use crate::files::trash::Trash;
use crate::files::Places;
use crate::log::{ActionLog, Entry};
use crate::normalize;
use crate::parser;
use crate::platform::Platform;
use crate::protocol::{Choice, ClientMessage, Mood, ServerMessage};
use crate::steam::{self, Game};
use crate::texts::t;
use crate::undo::UndoOp;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Woher ein Befehl kommt.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// Getippt im Overlay oder geklickt im Einstellungsfenster am PC.
    Text,
    /// Gesprochen am PC (Wake-Word oder Push-to-Talk).
    Voice,
    /// Vom gekoppelten iPhone.
    Remote { device: String },
    /// Tool-Aufruf von Claude (Etappe 7).
    Claude,
}

impl Source {
    pub fn label(&self) -> String {
        match self {
            Source::Text => "text".into(),
            Source::Voice => "voice".into(),
            Source::Remote { device } => format!("remote:{device}"),
            Source::Claude => "claude".into(),
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Source::Remote { .. })
    }

    /// Kommt die Anfrage nicht direkt vom Menschen am PC?
    pub fn is_indirect(&self) -> bool {
        matches!(self, Source::Remote { .. } | Source::Claude)
    }
}

/// Wie eine Bestätigung eingegangen ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmVia {
    /// "Ja" gesagt oder getippt am PC.
    Spoken,
    /// Klick auf den Bestätigen-Knopf im Overlay am PC.
    PcClick,
    /// Antwort vom iPhone.
    Remote,
}

enum PendingKind {
    Run {
        job: Job,
    },
    /// Rückfrage – bei Ja wird `action` mit diesen Parametern neu vorbereitet.
    Clarify {
        action: String,
        params: Value,
    },
}

struct Pending {
    confirm_id: String,
    meta: Meta,
    pc_only: bool,
    created: Instant,
    kind: PendingKind,
}

/// Was zu einer Anfrage für Antwort und Protokoll gebraucht wird.
struct Meta {
    request_id: String,
    source: Source,
    text: Option<String>,
    action: String,
    params: Value,
    tier: u8,
}

pub struct Engine {
    platform: Arc<dyn Platform>,
    config: RwLock<Config>,
    data_dir: RwLock<PathBuf>,
    games: RwLock<Option<Vec<Game>>>,
    log: ActionLog,
    enabled: AtomicBool,
    pending: Mutex<Option<Pending>>,
    registry: Vec<Box<dyn Action>>,
    counter: AtomicU64,
}

impl Engine {
    pub fn new(platform: Arc<dyn Platform>, config: Config, log: ActionLog) -> Self {
        Self {
            platform,
            config: RwLock::new(config),
            data_dir: RwLock::new(crate::config::data_dir()),
            games: RwLock::new(None),
            log,
            enabled: AtomicBool::new(true),
            pending: Mutex::new(None),
            registry: actions::registry(),
            counter: AtomicU64::new(0),
        }
    }

    // ---------- Zustand ----------

    /// Not-Aus: `false` stoppt alles sofort und verwirft offene Bestätigungen.
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
        if !on {
            self.pending.lock().unwrap().take();
        }
        let msg = if on { "Not-Aus aufgehoben" } else { "Not-Aus aktiviert" };
        self.log_plain(&Source::Text, None, if on { "enabled" } else { "disabled" }, msg);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn config(&self) -> Config {
        self.config.read().unwrap().clone()
    }

    pub fn set_config(&self, config: Config) {
        *self.config.write().unwrap() = config;
    }

    /// Panco-Datenordner (Standard `%APPDATA%\Panco`; Tests setzen einen eigenen).
    pub fn set_data_dir(&self, dir: PathBuf) {
        *self.data_dir.write().unwrap() = dir;
    }

    pub fn data_dir(&self) -> PathBuf {
        self.data_dir.read().unwrap().clone()
    }

    pub fn places(&self) -> Places {
        Places::from_settings(&self.config.read().unwrap().files, &self.data_dir())
    }

    pub fn trash(&self) -> Trash {
        Trash::new(self.places().trash)
    }

    pub fn log(&self) -> &ActionLog {
        &self.log
    }

    pub fn platform(&self) -> &Arc<dyn Platform> {
        &self.platform
    }

    /// Steam-Bibliothek (lazy beim ersten Bedarf eingelesen).
    pub fn games(&self) -> Vec<Game> {
        if self.games.read().unwrap().is_none() {
            self.rescan_games();
        }
        self.games.read().unwrap().clone().unwrap_or_default()
    }

    pub fn rescan_games(&self) -> usize {
        let root = steam::find_root(self.config.read().unwrap().steam_path.as_deref());
        let games = root.map(|r| steam::scan(&r)).unwrap_or_default();
        let n = games.len();
        *self.games.write().unwrap() = Some(games);
        n
    }

    /// Für Tests und spätere Caches: Spieleliste direkt setzen.
    pub fn set_games(&self, games: Vec<Game>) {
        *self.games.write().unwrap() = Some(games);
    }

    // ---------- Eingänge ----------

    pub fn handle(&self, msg: ClientMessage, source: Source) -> ServerMessage {
        match msg {
            ClientMessage::Command { id, text } => self.command(&id, &text, source),
            ClientMessage::Invoke { id, action, params } => self.invoke(&id, &action, params, source),
            ClientMessage::Confirm { confirm_id, accept } => {
                let via = match source {
                    Source::Remote { .. } => ConfirmVia::Remote,
                    Source::Text => ConfirmVia::PcClick,
                    Source::Voice => ConfirmVia::Spoken,
                    // Claude kann nichts bestätigen.
                    Source::Claude => return self.reply_fail("-", t("pc_confirm_needed", &[]), Mood::Error),
                };
                self.confirm(Some(&confirm_id), accept, via)
            }
        }
    }

    /// Freier Text (getippt/gesprochen/vom iPhone).
    pub fn command(&self, id: &str, text: &str, source: Source) -> ServerMessage {
        if !self.enabled() {
            self.log_plain(&source, Some(text), "disabled", "Not-Aus aktiv");
            return self.reply_fail(id, t("disabled", &[]), Mood::Error);
        }
        let norm = normalize::command(text);

        // Antwort auf eine offene Frage?
        if let Some(accept) = parser::yes_no(&norm) {
            if self.pending.lock().unwrap().is_some() {
                let via = if source.is_remote() { ConfirmVia::Remote } else { ConfirmVia::Spoken };
                return self.confirm(None, accept, via);
            }
            if accept {
                return self.reply_fail(id, t("nothing_to_confirm", &[]), Mood::Confused);
            }
        }

        let intent = match source {
            // Gesprochen: Reste des Wake-Words vorne werden toleriert.
            Source::Voice => parser::parse_text_lenient(text),
            _ => parser::parse_text(text),
        };
        match intent {
            Some(intent) => self.run(id, intent.action, intent.params, source, Some(text.to_string())),
            None => {
                self.log_plain(&source, Some(text), "not_understood", "");
                self.reply_fail(id, t("not_understood", &[]), Mood::Confused)
            }
        }
    }

    /// Direkter Aktionsaufruf (Kacheln, Auswahlknöpfe, Einstellungen, später Claude-Tools).
    pub fn invoke(&self, id: &str, action: &str, params: Value, source: Source) -> ServerMessage {
        if !self.enabled() {
            self.log_plain(&source, None, "disabled", "Not-Aus aktiv");
            return self.reply_fail(id, t("disabled", &[]), Mood::Error);
        }
        self.run(id, action, params, source, None)
    }

    /// Wartet eine Rückfrage auf Antwort?
    pub fn has_pending(&self) -> bool {
        self.pending.lock().unwrap().is_some()
    }

    /// Bestätigung/Ablehnung. `confirm_id = None` → die aktuell offene Frage (Sprachantwort).
    pub fn confirm(&self, confirm_id: Option<&str>, accept: bool, via: ConfirmVia) -> ServerMessage {
        let mut slot = self.pending.lock().unwrap();
        let matches = slot.as_ref().is_some_and(|p| confirm_id.is_none_or(|c| c == p.confirm_id));
        if !matches {
            drop(slot);
            return self.reply_fail(confirm_id.unwrap_or("-"), t("nothing_to_confirm", &[]), Mood::Confused);
        }
        let p = slot.take().unwrap();

        let timeout = Duration::from_secs(self.config.read().unwrap().confirm_timeout_secs);
        if p.created.elapsed() > timeout {
            drop(slot);
            self.log_meta(&p.meta, "expired", "", None, None);
            return self.reply_fail(&p.meta.request_id, t("confirm_expired", &[]), Mood::Info);
        }

        // Darf diese Antwortart diese Frage beantworten?
        let allowed = match via {
            ConfirmVia::PcClick => true,
            ConfirmVia::Spoken => !p.pc_only,
            ConfirmVia::Remote => matches!(p.kind, PendingKind::Clarify { .. }) && p.meta.source.is_remote(),
        };
        if !allowed {
            let id = p.meta.request_id.clone();
            *slot = Some(p); // Frage bleibt offen
            return self.reply_fail(&id, t("pc_confirm_needed", &[]), Mood::Info);
        }
        drop(slot);

        if !self.enabled() {
            return self.reply_fail(&p.meta.request_id, t("disabled", &[]), Mood::Error);
        }
        if !accept {
            self.log_meta(&p.meta, "cancelled", "", None, None);
            let action = p.meta.action.clone();
            return ServerMessage::reply(
                &p.meta.request_id,
                true,
                t("confirm_cancelled", &[]),
                Mood::Info,
                Some(&action),
            );
        }
        match p.kind {
            PendingKind::Run { job } => {
                let result = job(self.platform.as_ref());
                self.finish(p.meta, result, "confirmed")
            }
            PendingKind::Clarify { action, params } => {
                self.run(&p.meta.request_id, &action, params, p.meta.source, p.meta.text)
            }
        }
    }

    // ---------- Kern ----------

    fn run(&self, id: &str, action: &str, params: Value, source: Source, text: Option<String>) -> ServerMessage {
        let mut meta = Meta { request_id: id.into(), source, text, action: action.into(), params, tier: 1 };
        let Some(spec) = catalog::get(action) else {
            self.log_meta(&meta, "denied", "nicht in der Whitelist", None, None);
            return self.reply_fail(id, t("not_understood", &[]), Mood::Confused);
        };
        meta.tier = spec.tier as u8;
        if meta.source.is_remote() && !spec.remote {
            self.log_meta(&meta, "denied", "remote gesperrt", None, None);
            return self.reply_fail(id, t("remote_forbidden", &[]), Mood::Error);
        }
        let Some(handler) = self.registry.iter().find(|a| a.id() == action) else {
            return self.reply_fail(id, t("unsupported", &[]), Mood::Error);
        };

        let mut plan = self.prepare(handler.as_ref(), &meta.params, &meta.source);
        // Spiel nicht gefunden? Einmal die Steam-Bibliothek neu einlesen (neu installiert?).
        if action == "app.launch" && matches!(plan, Err(ActionError::NotFound(_))) {
            self.rescan_games();
            plan = self.prepare(handler.as_ref(), &meta.params, &meta.source);
        }

        match plan {
            Err(e) => self.finish(meta, Err(e), "failed"),
            Ok(Plan::Clarify { question, params: new_params, action: next, choices }) => {
                let confirm_id = self.next_id();
                meta.tier = 1;
                self.log_meta(&meta, "pending", &question, None, None);
                let next = next.map(str::to_string).unwrap_or_else(|| action.to_string());
                *self.pending.lock().unwrap() = Some(Pending {
                    confirm_id: confirm_id.clone(),
                    meta,
                    pc_only: false,
                    created: Instant::now(),
                    kind: PendingKind::Clarify { action: next, params: new_params },
                });
                self.confirm_request(id, confirm_id, question, 1, false).with_choices(choices)
            }
            Ok(Plan::Ready { tier, what, job }) => {
                let tier = tier.max(spec.tier);
                meta.tier = tier as u8;
                if tier == Tier::PcOnly && meta.source.is_remote() {
                    self.log_meta(&meta, "denied", "Stufe 3 remote gesperrt", None, None);
                    return self.reply_fail(id, t("remote_forbidden", &[]), Mood::Error);
                }
                if tier == Tier::Instant {
                    let result = job(self.platform.as_ref());
                    return self.finish(meta, result, "done");
                }
                let pc_only = tier == Tier::PcOnly || meta.source.is_indirect();
                let question = t(if pc_only { "confirm_ask_pc" } else { "confirm_ask" }, &[("what", &what)]);
                let confirm_id = self.next_id();
                self.log_meta(&meta, "pending", &what, None, None);
                *self.pending.lock().unwrap() = Some(Pending {
                    confirm_id: confirm_id.clone(),
                    meta,
                    pc_only,
                    created: Instant::now(),
                    kind: PendingKind::Run { job },
                });
                self.confirm_request(id, confirm_id, question, tier as u8, pc_only)
            }
        }
    }

    fn prepare(&self, handler: &dyn Action, params: &Value, source: &Source) -> Result<Plan, ActionError> {
        let games = self.games();
        let places = self.places();
        let config = self.config.read().unwrap();
        let ctx = Ctx {
            platform: self.platform.as_ref(),
            config: &config,
            games: &games,
            source,
            places: &places,
            log: &self.log,
            now: crate::time::unix_now(),
        };
        handler.prepare(params, &ctx)
    }

    fn finish(&self, m: Meta, result: Result<Done, ActionError>, ok_outcome: &str) -> ServerMessage {
        match result {
            Ok(done) => {
                self.log_meta(&m, ok_outcome, &done.text, done.undo.clone(), done.undo_of.clone());
                ServerMessage::reply(&m.request_id, true, short(&done.text), done.mood, Some(&m.action))
                    .with_choices(done.choices)
            }
            Err(e) => {
                let (text, mood) = match &e {
                    ActionError::NotFound(msg) => (msg.clone(), Mood::Confused),
                    ActionError::Unsupported => (t("unsupported", &[]), Mood::Error),
                    ActionError::InvalidParams(r) | ActionError::Failed(r) => {
                        (t("error", &[("reason", r)]), Mood::Error)
                    }
                };
                self.log_meta(&m, "failed", &e.to_string(), None, None);
                ServerMessage::reply(&m.request_id, false, text, mood, Some(&m.action))
            }
        }
    }

    // ---------- Hilfen ----------

    fn next_id(&self) -> String {
        format!("c{}", self.counter.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn confirm_request(&self, id: &str, confirm_id: String, text: String, tier: u8, pc_only: bool) -> ServerMessage {
        let secs = self.config.read().unwrap().confirm_timeout_secs;
        ServerMessage::ConfirmRequest {
            id: id.into(),
            confirm_id,
            text: short(&text),
            tier,
            expires_in_ms: secs * 1000,
            pc_only,
            choices: Vec::<Choice>::new(),
        }
    }

    fn reply_fail(&self, id: &str, text: String, mood: Mood) -> ServerMessage {
        ServerMessage::reply(id, false, text, mood, None)
    }

    /// Hintergrund-Ereignis protokollieren (Backup, Papierkorb geleert …).
    pub fn log_system(&self, outcome: &str, msg: &str) {
        self.log.write(Entry {
            ts: ActionLog::now(),
            source: "panco".into(),
            outcome: outcome.into(),
            message: msg.into(),
            ..Default::default()
        });
    }

    /// Wartung: abgelaufene Papierkorb-Einträge entfernen, fälliges Backup anlegen.
    /// Gibt eine kurze Zusammenfassung zurück (für Protokoll/Diagnose).
    pub fn maintenance(&self, force_backup: bool) -> Vec<String> {
        let places = self.places();
        let files = self.config.read().unwrap().files.clone();
        let now = crate::time::unix_now();
        let mut report = Vec::new();
        let purged = self.trash().purge_older_than(places.trash_days, now);
        if purged > 0 {
            let msg = format!("{purged} Einträge nach {} Tagen aus dem Papierkorb entfernt", places.trash_days);
            self.log_system("trash_purged", &msg);
            report.push(msg);
        }
        if files.backup_enabled && (force_backup || crate::backup::due(&places.backup, now)) {
            match crate::backup::run(&places, now, self.platform.utc_offset_secs(), files.backup_keep) {
                Ok(b) => {
                    let msg = format!("Backup {} angelegt", b.name);
                    self.log_system("backup", &msg);
                    report.push(msg);
                }
                Err(e) => {
                    let msg = format!("Backup fehlgeschlagen: {e}");
                    self.log_system("backup_failed", &msg);
                    report.push(msg);
                }
            }
        }
        report
    }

    fn log_plain(&self, source: &Source, text: Option<&str>, outcome: &str, msg: &str) {
        self.log.write(Entry {
            ts: ActionLog::now(),
            source: source.label(),
            text: text.map(str::to_string),
            outcome: outcome.into(),
            message: msg.into(),
            ..Default::default()
        });
    }

    fn log_meta(&self, m: &Meta, outcome: &str, msg: &str, undo: Option<UndoOp>, undo_of: Option<String>) {
        let spec = catalog::get(&m.action);
        let redact = spec.is_some_and(|s| s.redact);
        let (text, params, message) = if redact {
            (m.text.as_ref().map(|_| "[ausgeblendet]".to_string()), Some(redact_params(spec.unwrap(), &m.params)), {
                if outcome == "done" || outcome == "confirmed" {
                    "[ausgeblendet]".to_string()
                } else {
                    msg.to_string()
                }
            })
        } else {
            (m.text.clone(), Some(m.params.clone()), msg.to_string())
        };
        self.log.write(Entry {
            ts: ActionLog::now(),
            source: m.source.label(),
            text,
            action: spec.map(|s| s.id.clone()).or_else(|| Some(m.action.clone())),
            params,
            tier: Some(m.tier),
            outcome: outcome.into(),
            message,
            undo,
            undo_of,
            ..Default::default()
        });
    }
}

/// Antworten sind höchstens 300 Zeichen lang (Protokoll-Schema, Sprechblase).
fn short(text: &str) -> String {
    if text.chars().count() <= 300 {
        text.to_string()
    } else {
        text.chars().take(298).collect::<String>() + " …"
    }
}

/// Text-Parameter durch ihre Länge ersetzen; Titel bleiben lesbar (zum Wiederfinden).
fn redact_params(_spec: &ActionSpec, params: &Value) -> Value {
    match params {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let v = match (k.as_str(), v) {
                        ("title" | "query", _) => v.clone(),
                        (_, Value::String(s)) => Value::String(format!("[{} Zeichen]", s.chars().count())),
                        _ => v.clone(),
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

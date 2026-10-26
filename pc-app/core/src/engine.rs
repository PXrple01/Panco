//! Engine: verbindet Parser, Whitelist, Stufen, Bestätigungen und Protokoll.
//!
//! Sicherheitsregeln (siehe CLAUDE.md), alle hier durchgesetzt:
//! - Not-Aus aus → nichts wird ausgeführt.
//! - Nur Aktionen aus dem Katalog; Remote nur, wenn `remote: true`.
//! - Stufe 1 sofort; Stufe 2 Bestätigung (Sprache/Overlay); Stufe 3 nur Klick am PC, nie remote.
//! - Anfragen vom iPhone oder von Claude: Stufe 2/3 immer per Klick am PC bestätigen.
//! - Jede Entscheidung landet im Protokoll.

use crate::actions::{self, Action, ActionError, Ctx, Job, Plan};
use crate::catalog::{self, Tier};
use crate::config::Config;
use crate::log::{ActionLog, Entry};
use crate::normalize;
use crate::parser;
use crate::platform::Platform;
use crate::protocol::{ClientMessage, Mood, ServerMessage};
use crate::steam::{self, Game};
use crate::texts::t;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Woher ein Befehl kommt.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// Getippt im Overlay am PC.
    Text,
    /// Gesprochen am PC (Wake-Word oder Push-to-Talk).
    Voice,
    /// Vom gekoppelten iPhone.
    Remote { device: String },
    /// Tool-Aufruf von Claude (Etappe 6).
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

    fn is_remote(&self) -> bool {
        matches!(self, Source::Remote { .. })
    }

    /// Kommt die Anfrage nicht direkt vom Menschen am PC?
    fn is_indirect(&self) -> bool {
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
    /// "Meinst du …?" – bei Ja wird die Aktion mit diesen Parametern neu vorbereitet.
    Clarify {
        params: Value,
    },
}

struct Pending {
    confirm_id: String,
    request_id: String,
    source: Source,
    text: Option<String>,
    action: String,
    params: Value,
    tier: u8,
    pc_only: bool,
    created: Instant,
    kind: PendingKind,
}

pub struct Engine {
    platform: Arc<dyn Platform>,
    config: RwLock<Config>,
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
        self.log_simple(
            &Source::Text,
            None,
            if on { "enabled" } else { "disabled" },
            if on { "Not-Aus aufgehoben" } else { "Not-Aus aktiviert" },
        );
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
            self.log_simple(&source, Some(text), "disabled", "Not-Aus aktiv");
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

        match parser::parse(&norm) {
            Some(intent) => self.run(id, intent.action, intent.params, source, Some(text.to_string())),
            None => {
                self.log_simple(&source, Some(text), "not_understood", "");
                self.reply_fail(id, t("not_understood", &[]), Mood::Confused)
            }
        }
    }

    /// Direkter Aktionsaufruf (Kacheln, später Claude-Tools).
    pub fn invoke(&self, id: &str, action: &str, params: Value, source: Source) -> ServerMessage {
        if !self.enabled() {
            self.log_simple(&source, None, "disabled", "Not-Aus aktiv");
            return self.reply_fail(id, t("disabled", &[]), Mood::Error);
        }
        self.run(id, action, params, source, None)
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
            self.log_pending(&p, "expired", "");
            return self.reply_fail(&p.request_id, t("confirm_expired", &[]), Mood::Info);
        }

        // Darf diese Antwortart diese Frage beantworten?
        let allowed = match via {
            ConfirmVia::PcClick => true,
            ConfirmVia::Spoken => !p.pc_only,
            ConfirmVia::Remote => matches!(p.kind, PendingKind::Clarify { .. }) && p.source.is_remote(),
        };
        if !allowed {
            let id = p.request_id.clone();
            *slot = Some(p); // Frage bleibt offen
            return self.reply_fail(&id, t("pc_confirm_needed", &[]), Mood::Info);
        }
        drop(slot);

        if !self.enabled() {
            return self.reply_fail(&p.request_id, t("disabled", &[]), Mood::Error);
        }
        if !accept {
            self.log_pending(&p, "cancelled", "");
            return ServerMessage::reply(&p.request_id, true, t("confirm_cancelled", &[]), Mood::Info, Some(&p.action));
        }
        match p.kind {
            PendingKind::Run { job } => {
                let result = job(self.platform.as_ref());
                let request = PendingMeta {
                    request_id: p.request_id,
                    source: p.source,
                    text: p.text,
                    action: p.action,
                    params: p.params,
                    tier: p.tier,
                };
                self.finish(request, result, "confirmed")
            }
            PendingKind::Clarify { params } => self.run(&p.request_id, &p.action, params, p.source, p.text),
        }
    }

    // ---------- Kern ----------

    fn run(&self, id: &str, action: &str, params: Value, source: Source, text: Option<String>) -> ServerMessage {
        let Some(spec) = catalog::get(action) else {
            self.log_entry(
                &source,
                text.as_deref(),
                Some(action),
                Some(&params),
                None,
                "denied",
                "nicht in der Whitelist",
            );
            return self.reply_fail(id, t("not_understood", &[]), Mood::Confused);
        };
        if source.is_remote() && !spec.remote {
            self.log_entry(
                &source,
                text.as_deref(),
                Some(action),
                Some(&params),
                Some(spec.tier as u8),
                "denied",
                "remote gesperrt",
            );
            return self.reply_fail(id, t("remote_forbidden", &[]), Mood::Error);
        }
        let Some(handler) = self.registry.iter().find(|a| a.id() == action) else {
            return self.reply_fail(id, t("unsupported", &[]), Mood::Error);
        };

        let mut plan = self.prepare(handler.as_ref(), &params, &source);
        // Spiel nicht gefunden? Einmal die Steam-Bibliothek neu einlesen (neu installiert?).
        if action == "app.launch" && matches!(plan, Err(ActionError::NotFound(_))) {
            self.rescan_games();
            plan = self.prepare(handler.as_ref(), &params, &source);
        }

        match plan {
            Err(e) => {
                let meta = PendingMeta {
                    request_id: id.into(),
                    source,
                    text,
                    action: action.into(),
                    params,
                    tier: spec.tier as u8,
                };
                self.finish(meta, Err(e), "failed")
            }
            Ok(Plan::Clarify { question, params: new_params }) => {
                let confirm_id = self.next_id();
                self.log_entry(&source, text.as_deref(), Some(action), Some(&params), Some(1), "pending", &question);
                *self.pending.lock().unwrap() = Some(Pending {
                    confirm_id: confirm_id.clone(),
                    request_id: id.into(),
                    source,
                    text,
                    action: action.into(),
                    params,
                    tier: 1,
                    pc_only: false,
                    created: Instant::now(),
                    kind: PendingKind::Clarify { params: new_params },
                });
                self.confirm_request(id, confirm_id, question, 1, false)
            }
            Ok(Plan::Ready { tier, what, job }) => {
                let tier = tier.max(spec.tier);
                if tier == Tier::PcOnly && source.is_remote() {
                    self.log_entry(
                        &source,
                        text.as_deref(),
                        Some(action),
                        Some(&params),
                        Some(3),
                        "denied",
                        "Stufe 3 remote gesperrt",
                    );
                    return self.reply_fail(id, t("remote_forbidden", &[]), Mood::Error);
                }
                if tier == Tier::Instant {
                    let result = job(self.platform.as_ref());
                    let meta =
                        PendingMeta { request_id: id.into(), source, text, action: action.into(), params, tier: 1 };
                    return self.finish(meta, result, "done");
                }
                let pc_only = tier == Tier::PcOnly || source.is_indirect();
                let question = t(if pc_only { "confirm_ask_pc" } else { "confirm_ask" }, &[("what", &what)]);
                let confirm_id = self.next_id();
                self.log_entry(
                    &source,
                    text.as_deref(),
                    Some(action),
                    Some(&params),
                    Some(tier as u8),
                    "pending",
                    &what,
                );
                *self.pending.lock().unwrap() = Some(Pending {
                    confirm_id: confirm_id.clone(),
                    request_id: id.into(),
                    source,
                    text,
                    action: action.into(),
                    params,
                    tier: tier as u8,
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
        let config = self.config.read().unwrap();
        let ctx = Ctx { platform: self.platform.as_ref(), config: &config, games: &games, source };
        handler.prepare(params, &ctx)
    }

    fn finish(&self, m: PendingMeta, result: Result<actions::Done, ActionError>, ok_outcome: &str) -> ServerMessage {
        match result {
            Ok(done) => {
                self.log_entry(
                    &m.source,
                    m.text.as_deref(),
                    Some(&m.action),
                    Some(&m.params),
                    Some(m.tier),
                    ok_outcome,
                    &done.text,
                );
                ServerMessage::reply(&m.request_id, true, done.text, done.mood, Some(&m.action))
            }
            Err(e) => {
                let (text, mood) = match &e {
                    ActionError::NotFound(msg) => (msg.clone(), Mood::Confused),
                    ActionError::Unsupported => (t("unsupported", &[]), Mood::Error),
                    ActionError::InvalidParams(r) | ActionError::Failed(r) => {
                        (t("error", &[("reason", r)]), Mood::Error)
                    }
                };
                self.log_entry(
                    &m.source,
                    m.text.as_deref(),
                    Some(&m.action),
                    Some(&m.params),
                    Some(m.tier),
                    "failed",
                    &e.to_string(),
                );
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
        ServerMessage::ConfirmRequest { id: id.into(), confirm_id, text, tier, expires_in_ms: secs * 1000, pc_only }
    }

    fn reply_fail(&self, id: &str, text: String, mood: Mood) -> ServerMessage {
        ServerMessage::reply(id, false, text, mood, None)
    }

    fn log_pending(&self, p: &Pending, outcome: &str, msg: &str) {
        self.log_entry(&p.source, p.text.as_deref(), Some(&p.action), Some(&p.params), Some(p.tier), outcome, msg);
    }

    fn log_simple(&self, source: &Source, text: Option<&str>, outcome: &str, msg: &str) {
        self.log_entry(source, text, None, None, None, outcome, msg);
    }

    #[allow(clippy::too_many_arguments)]
    fn log_entry(
        &self,
        source: &Source,
        text: Option<&str>,
        action: Option<&str>,
        params: Option<&Value>,
        tier: Option<u8>,
        outcome: &str,
        msg: &str,
    ) {
        self.log.write(Entry {
            ts: ActionLog::now(),
            source: source.label(),
            text: text.map(str::to_string),
            action: action.map(str::to_string),
            params: params.cloned(),
            tier,
            outcome: outcome.into(),
            message: msg.into(),
        });
    }
}

/// Was nach dem Ausführen für Antwort und Protokoll gebraucht wird.
struct PendingMeta {
    request_id: String,
    source: Source,
    text: Option<String>,
    action: String,
    params: Value,
    tier: u8,
}

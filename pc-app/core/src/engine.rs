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
    /// Eine selbst eingerichtete Regel ("Wenn ich Deadlock starte, …") – nur Stufe 1.
    Rule,
}

impl Source {
    pub fn label(&self) -> String {
        match self {
            Source::Text => "text".into(),
            Source::Voice => "voice".into(),
            Source::Remote { device } => format!("remote:{device}"),
            Source::Claude => "claude".into(),
            Source::Rule => "regel".into(),
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
    /// Claude wartet auf diese Entscheidung (Ergebnis wird zusätzlich hierhin geschickt).
    waiter: Option<std::sync::mpsc::Sender<ServerMessage>>,
}

/// Diese Aktionen bringen fremde Inhalte (Bildschirm, Dateien, Zwischenablage …) in eine
/// Claude-Anfrage. Danach dürfen Skripte in derselben Anfrage nur noch lesen.
const TAINTING: [&str; 7] =
    ["screen.read", "screen.look", "clipboard.read", "file.read", "file.search", "note.read", "note.search"];

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
    /// Spielzeit + laufendes Spiel (beim ersten Bedarf aus spielzeit.json geladen)
    play: Mutex<Option<crate::playtime::Tracker>>,
    /// Letzte erfolgreiche Aktion (für "nochmal" und "schließ es")
    last: Mutex<Option<(String, Value)>>,
    /// "10 Minuten nicht mehr fragen": bis dahin laufen Stufe-2-Befehle vom PC ohne Rückfrage.
    quiet_until: Mutex<Option<Instant>>,
    /// In der laufenden Claude-Anfrage wurden fremde Inhalte gelesen (siehe `TAINTING`).
    tainted: AtomicBool,
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
            play: Mutex::new(None),
            last: Mutex::new(None),
            quiet_until: Mutex::new(None),
            tainted: AtomicBool::new(false),
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

    // ---------- Spiele ----------

    fn with_play<R>(&self, f: impl FnOnce(&mut crate::playtime::Tracker) -> R) -> R {
        let mut slot = self.play.lock().unwrap();
        let tracker =
            slot.get_or_insert_with(|| crate::playtime::Tracker::load(&self.data_dir().join("spielzeit.json")));
        f(tracker)
    }

    /// Welches Spiel läuft? Steam meldet die App-ID; andere Spiele über bekannte Exe-Namen.
    pub fn detect_game(&self, steam_app: Option<u32>, processes: &[String]) -> Option<String> {
        if let Some(id) = steam_app {
            let name = self.games().into_iter().find(|g| g.app_id == id).map(|g| g.name);
            return Some(name.unwrap_or_else(|| format!("Steam-Spiel {id}")));
        }
        let known = &self.config.read().unwrap().game_processes;
        processes.iter().find_map(|p| known.iter().find(|(exe, _)| exe.eq_ignore_ascii_case(p)).map(|(_, n)| n.clone()))
    }

    /// Neue Beobachtung der Spiel-Erkennung; Start/Ende landen im Protokoll.
    pub fn observe_game(&self, running: Option<&str>) -> Vec<crate::playtime::Change> {
        let offset = self.platform.utc_offset_secs();
        let changes = self.with_play(|t| t.observe(crate::time::unix_now(), offset, running));
        for c in &changes {
            let msg = match c {
                crate::playtime::Change::Started(n) => format!("{n} gestartet – Game Mode an"),
                crate::playtime::Change::Stopped { name, secs } => {
                    format!("{name} beendet nach {} – Game Mode aus", crate::playtime::duration_de(*secs))
                }
            };
            self.log_system("game", &msg);
        }
        changes
    }

    /// Ein Spiel wurde gestartet: Regeln mit Auslöser "Spielstart" ausführen.
    pub fn on_game_started(&self, name: &str) -> Vec<String> {
        let memory = crate::memory::Memory::load(&self.data_dir());
        let rules: Vec<_> = memory
            .rules
            .into_iter()
            .filter(|r| matches!(&r.when, crate::memory::Trigger::GameStart { game } if game.as_deref().is_none_or(|g| g == name)))
            .collect();
        rules.iter().flat_map(|r| self.run_rule(r)).collect()
    }

    /// Schritte einer Regel ausführen (Quelle "regel": nur Stufe 1, jede einzeln protokolliert).
    fn run_rule(&self, rule: &crate::memory::Rule) -> Vec<String> {
        let mut texts = Vec::new();
        for step in &rule.steps {
            let id = self.next_id();
            let msg = self.run(&id, &step.action, step.params.clone(), Source::Rule, Some(rule.describe()));
            if let ServerMessage::Reply { ok: true, text, .. } = msg {
                texts.push(text);
            }
        }
        texts
    }

    /// Läuft gerade ein Spiel (= Game Mode)?
    pub fn current_game(&self) -> Option<crate::playtime::Running> {
        self.play.lock().unwrap().as_ref().and_then(|t| t.current.clone())
    }

    /// Spielzeit sichern (z. B. beim Beenden von Panco).
    pub fn flush_playtime(&self) {
        if let Some(t) = self.play.lock().unwrap().as_mut() {
            t.flush(crate::time::unix_now());
        }
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
                    Source::Claude | Source::Rule => {
                        return self.reply_fail("-", t("pc_confirm_needed", &[]), Mood::Error)
                    }
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

        // "Ja, und frag die nächsten 10 Minuten nicht mehr"
        if self.pending.lock().unwrap().is_some() && !source.is_indirect() && parser::wants_quiet(&norm) {
            self.set_quiet(10);
            let via = if source.is_remote() { ConfirmVia::Remote } else { ConfirmVia::Spoken };
            return self.confirm(None, true, via);
        }

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

        // Eigener Befehl? ("Panco, Feierabend")
        if let Some(m) = crate::memory::Macros::load(&self.data_dir()).find(&norm) {
            let name = m.name.clone();
            return self.run(id, "macro.run", serde_json::json!({ "name": name }), source, Some(text.to_string()));
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
        let mut p = slot.take().unwrap();

        // Darf diese Antwortart diese Frage beantworten?
        let timeout = Duration::from_secs(self.config.read().unwrap().confirm_timeout_secs);
        let allowed = match via {
            ConfirmVia::PcClick => true,
            ConfirmVia::Spoken => !p.pc_only,
            ConfirmVia::Remote => matches!(p.kind, PendingKind::Clarify { .. }) && p.meta.source.is_remote(),
        };
        if !allowed && p.created.elapsed() <= timeout {
            let id = p.meta.request_id.clone();
            *slot = Some(p); // Frage bleibt offen
            return self.reply_fail(&id, t("pc_confirm_needed", &[]), Mood::Info);
        }
        drop(slot);

        let waiter = p.waiter.take();
        let result = if p.created.elapsed() > timeout {
            self.log_meta(&p.meta, "expired", "", None, None);
            self.reply_fail(&p.meta.request_id, t("confirm_expired", &[]), Mood::Info)
        } else if !self.enabled() {
            self.reply_fail(&p.meta.request_id, t("disabled", &[]), Mood::Error)
        } else if !accept {
            self.log_meta(&p.meta, "cancelled", "", None, None);
            let action = p.meta.action.clone();
            ServerMessage::reply(&p.meta.request_id, true, t("confirm_cancelled", &[]), Mood::Info, Some(&action))
        } else {
            match p.kind {
                PendingKind::Run { job } => {
                    let result = job(self.platform.as_ref());
                    self.finish(p.meta, result, "confirmed")
                }
                PendingKind::Clarify { action, params } => {
                    self.run(&p.meta.request_id, &action, params, p.meta.source, p.meta.text)
                }
            }
        };
        if let Some(w) = waiter {
            let _ = w.send(result.clone());
        }
        result
    }

    // ---------- Claude ----------

    /// Neue Claude-Anfrage beginnt: noch nichts Fremdes gelesen.
    pub fn begin_claude_turn(&self) {
        self.tainted.store(false, Ordering::SeqCst);
    }

    /// Hat Claude in dieser Anfrage schon fremde Inhalte gelesen?
    pub fn claude_tainted(&self) -> bool {
        self.tainted.load(Ordering::SeqCst)
    }

    /// Tool-Aufruf von Claude. Braucht er eine Bestätigung, wird `on_question` mit der Rückfrage
    /// aufgerufen (Overlay zeigt sie) und blockierend auf den Klick am PC gewartet – höchstens
    /// `timeout`. Ohne Antwort gilt sie als abgelehnt.
    pub fn claude_invoke(
        &self,
        id: &str,
        action: &str,
        params: Value,
        on_question: &mut dyn FnMut(&ServerMessage),
        timeout: Duration,
    ) -> ServerMessage {
        if !self.enabled() {
            return self.reply_fail(id, t("disabled", &[]), Mood::Error);
        }
        let msg = self.run(id, action, params, Source::Claude, None);
        let ServerMessage::ConfirmRequest { confirm_id, .. } = &msg else { return msg };
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let mut slot = self.pending.lock().unwrap();
            match slot.as_mut() {
                Some(p) if &p.confirm_id == confirm_id => p.waiter = Some(tx),
                _ => return self.reply_fail(id, t("confirm_expired", &[]), Mood::Info),
            }
        }
        on_question(&msg);
        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(_) => {
                // Zeit abgelaufen oder Frage ersetzt/Not-Aus: offene Frage verwerfen.
                let mut slot = self.pending.lock().unwrap();
                if slot.as_ref().is_some_and(|p| &p.confirm_id == confirm_id) {
                    let p = slot.take().unwrap();
                    drop(slot);
                    self.log_meta(&p.meta, "expired", "", None, None);
                }
                self.reply_fail(id, t("confirm_expired", &[]), Mood::Info)
            }
        }
    }

    // ---------- Kern ----------

    fn run(&self, id: &str, action: &str, mut params: Value, source: Source, text: Option<String>) -> ServerMessage {
        // "Frag mich 10 Minuten nicht mehr" / "Frag wieder nach" (nur direkt am PC)
        if action == "panco.quiet" {
            if source.is_indirect() || source == Source::Rule {
                return self.reply_fail(id, t("pc_confirm_needed", &[]), Mood::Error);
            }
            let minutes = params.get("minutes").and_then(Value::as_u64).unwrap_or(10).min(120);
            self.set_quiet(minutes);
            let text = if minutes == 0 { t("quiet_off", &[]) } else { t("quiet_on", &[("min", &minutes.to_string())]) };
            self.log_plain(&source, text.as_str().into(), "done", &text);
            return ServerMessage::reply(id, true, text, Mood::Info, None);
        }
        // "nochmal": die letzte erfolgreiche Aktion wiederholen (mit denselben Regeln wie beim ersten Mal).
        if action == "panco.repeat" {
            let last = self.last.lock().unwrap().clone();
            return match last {
                Some((a, p)) => self.run(id, &a, p, source, text),
                None => self.reply_fail(id, t("repeat_nothing", &[]), Mood::Confused),
            };
        }
        self.fill_pronouns(&mut params);
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
            // Claude bekommt Rückfragen als Antwort (mit den Möglichkeiten) und fragt selbst nach.
            Ok(Plan::Clarify { question, choices, .. }) if meta.source == Source::Claude => {
                let options: Vec<String> = choices.iter().map(|c| c.label.clone()).collect();
                let data = if options.is_empty() {
                    question.clone()
                } else {
                    format!("{question} Möglich: {}", options.join("; "))
                };
                self.log_meta(&meta, "unclear", &question, None, None);
                let mut reply = ServerMessage::reply(id, false, question, Mood::Confused, Some(action));
                if let ServerMessage::Reply { data: d, .. } = &mut reply {
                    *d = Some(data);
                }
                reply
            }
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
                    waiter: None,
                });
                self.confirm_request(id, confirm_id, question, 1, false, None).with_choices(choices)
            }
            Ok(Plan::Ready { tier, what, job, detail, choices }) => {
                let tier = tier.max(spec.tier);
                meta.tier = tier as u8;
                if tier != Tier::Instant && meta.source == Source::Rule {
                    self.log_meta(&meta, "denied", "Regeln dürfen nur Stufe 1", None, None);
                    return self.reply_fail(id, t("rule_only_instant", &[]), Mood::Error);
                }
                if tier == Tier::PcOnly && meta.source.is_remote() {
                    self.log_meta(&meta, "denied", "Stufe 3 remote gesperrt", None, None);
                    return self.reply_fail(id, t("remote_forbidden", &[]), Mood::Error);
                }
                // Ohne Rückfrage: Stufe 1 immer; Stufe 2 nur direkt am PC, wenn so eingestellt oder
                // gerade "nicht mehr fragen" gilt. Stufe 3, iPhone, Claude und Regeln fragen immer.
                let at_pc = matches!(meta.source, Source::Text | Source::Voice);
                let skip = tier == Tier::Confirm
                    && at_pc
                    && (self.quiet() || self.config.read().unwrap().skip_confirm.iter().any(|a| a == action));
                if tier == Tier::Instant || skip {
                    let result = job(self.platform.as_ref());
                    return self.finish(meta, result, if skip { "done_unasked" } else { "done" });
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
                    waiter: None,
                });
                self.confirm_request(id, confirm_id, question, tier as u8, pc_only, detail).with_choices(choices)
            }
        }
    }

    fn prepare(&self, handler: &dyn Action, params: &Value, source: &Source) -> Result<Plan, ActionError> {
        let games = self.games();
        let places = self.places();
        let data_dir = self.data_dir();
        self.with_play(|_| ()); // Spielzeit laden, bevor die Konfiguration gesperrt wird
        let play = self.play.lock().unwrap();
        let config = self.config.read().unwrap();
        let ctx = Ctx {
            platform: self.platform.as_ref(),
            config: &config,
            games: &games,
            source,
            places: &places,
            log: &self.log,
            now: crate::time::unix_now(),
            play: play.as_ref(),
            tainted: *source == Source::Claude && self.claude_tainted(),
            data_dir: &data_dir,
        };
        handler.prepare(params, &ctx)
    }

    /// Rückfragen (Stufe 2, nur am PC) für `minutes` Minuten aussetzen; 0 = wieder fragen.
    pub fn set_quiet(&self, minutes: u64) {
        *self.quiet_until.lock().unwrap() = (minutes > 0).then(|| Instant::now() + Duration::from_secs(minutes * 60));
    }

    /// Restzeit von "nicht mehr fragen" in Sekunden (0 = aus).
    pub fn quiet_secs(&self) -> u64 {
        self.quiet_until.lock().unwrap().map_or(0, |u| u.saturating_duration_since(Instant::now()).as_secs())
    }

    fn quiet(&self) -> bool {
        self.quiet_secs() > 0
    }

    /// "schließ es", "öffne das": Fürwort durch das letzte Ziel ersetzen.
    fn fill_pronouns(&self, params: &mut Value) {
        const PRONOUNS: [&str; 7] = ["es", "das", "ihn", "sie", "den", "die", "dieses"];
        let Some(obj) = params.as_object_mut() else { return };
        for key in ["target", "app"] {
            let is_pronoun = obj
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|v| PRONOUNS.contains(&v.trim().to_lowercase().as_str()));
            if is_pronoun {
                if let Some(last) = self.last_target() {
                    obj.insert(key.into(), Value::String(last));
                }
            }
        }
    }

    fn last_target(&self) -> Option<String> {
        let last = self.last.lock().unwrap();
        let (_, p) = last.as_ref()?;
        ["target", "app"].iter().find_map(|k| p.get(*k).and_then(Value::as_str).map(str::to_string))
    }

    fn finish(&self, m: Meta, result: Result<Done, ActionError>, ok_outcome: &str) -> ServerMessage {
        match result {
            Ok(mut done) => {
                // Für "nochmal"/"schließ es" merken (nur echte Aktionen, keine Auskünfte)
                const NOT_REPEATABLE: [&str; 6] =
                    ["panco.undo", "panco.help", "info.time", "info.playtime", "memory.list", "memory.forget"];
                if !NOT_REPEATABLE.contains(&m.action.as_str()) && !m.action.starts_with("macro.c") {
                    *self.last.lock().unwrap() = Some((m.action.clone(), m.params.clone()));
                }
                // Regeln "Wenn ich X starte, …" (Regeln lösen keine Regeln aus – keine Schleifen)
                if m.action == "app.launch" && m.source != Source::Rule {
                    let target =
                        m.params.get("target").and_then(Value::as_str).map(normalize::name).unwrap_or_default();
                    let memory = crate::memory::Memory::load(&self.data_dir());
                    for rule in memory
                        .rules
                        .iter()
                        .filter(|r| matches!(&r.when, crate::memory::Trigger::AppStart { target: t } if *t == target))
                    {
                        let ran = self.run_rule(rule);
                        if !ran.is_empty() {
                            done.text = format!("{} {}", done.text, t("rule_ran", &[("text", &ran.join(" "))]));
                        }
                    }
                }
                if m.source == Source::Claude && TAINTING.contains(&m.action.as_str()) {
                    self.tainted.store(true, Ordering::SeqCst);
                }
                self.log_meta(&m, ok_outcome, &done.text, done.undo.clone(), done.undo_of.clone());
                let mut reply =
                    ServerMessage::reply(&m.request_id, true, short(&done.text), done.mood, Some(&m.action))
                        .with_choices(done.choices);
                if let ServerMessage::Reply { data, image, .. } = &mut reply {
                    *data = done.data.or_else(|| (done.text.chars().count() > 300).then(|| done.text.clone()));
                    *image = done.image;
                }
                reply
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

    fn confirm_request(
        &self,
        id: &str,
        confirm_id: String,
        text: String,
        tier: u8,
        pc_only: bool,
        detail: Option<String>,
    ) -> ServerMessage {
        let secs = self.config.read().unwrap().confirm_timeout_secs;
        ServerMessage::ConfirmRequest {
            id: id.into(),
            confirm_id,
            text: short(&text),
            tier,
            expires_in_ms: secs * 1000,
            pc_only,
            detail: detail.map(|d| d.chars().take(20_000).collect()),
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

    /// Claude-Anfrage protokollieren (erkannter Text + Anfang der Antwort; Tool-Aufrufe stehen einzeln).
    pub fn log_claude(&self, text: &str, answer: &str) {
        let short: String = answer.chars().take(200).collect();
        self.log_plain(&Source::Claude, Some(text), "claude", &short);
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
        // Während eines Spiels kein Backup (Festplatte schonen) – es folgt nach dem Spiel.
        let gaming = self.current_game().is_some();
        if gaming && !force_backup && files.backup_enabled && crate::backup::due(&places.backup, now) {
            report.push("Backup wartet bis nach dem Spiel".into());
        }
        if files.backup_enabled && (force_backup || (!gaming && crate::backup::due(&places.backup, now))) {
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

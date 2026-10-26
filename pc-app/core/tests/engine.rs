//! Engine-Tests mit MockPlatform: Whitelist, Stufen, Bestätigungen, Not-Aus, Protokoll.

use panco_core::config::Config;
use panco_core::engine::{ConfirmVia, Engine, Source};
use panco_core::log::ActionLog;
use panco_core::platform::mock::MockPlatform;
use panco_core::platform::WindowInfo;
use panco_core::protocol::{ClientMessage, Mood, ServerMessage};
use panco_core::steam::Game;
use serde_json::json;
use std::sync::Arc;

fn setup() -> (Engine, Arc<MockPlatform>) {
    setup_with(Config::default(), None)
}

fn setup_with(config: Config, log_dir: Option<std::path::PathBuf>) -> (Engine, Arc<MockPlatform>) {
    let pf = Arc::new(MockPlatform::default());
    *pf.volume.lock().unwrap() = 50;
    let engine = Engine::new(pf.clone(), config, ActionLog::new(log_dir));
    engine.set_games(vec![
        Game { app_id: 1422450, name: "Deadlock".into() },
        Game { app_id: 730, name: "Counter-Strike 2".into() },
        Game { app_id: 292030, name: "The Witcher 3: Wild Hunt".into() },
    ]);
    (engine, pf)
}

fn say(e: &Engine, text: &str) -> ServerMessage {
    e.command("r1", text, Source::Voice)
}

fn confirm_id(msg: &ServerMessage) -> String {
    match msg {
        ServerMessage::ConfirmRequest { confirm_id, .. } => confirm_id.clone(),
        other => panic!("Bestätigungsfrage erwartet, bekam {other:?}"),
    }
}

#[test]
fn launches_steam_game() {
    let (e, pf) = setup();
    let r = say(&e, "Panco, öffne Deadlock");
    assert_eq!(r, ServerMessage::reply("r1", true, "Deadlock startet.", Mood::Game, Some("app.launch")));
    assert_eq!(pf.take_calls(), vec!["open steam://rungameid/1422450"]);
}

#[test]
fn launches_game_alias_and_app_alias() {
    let (e, pf) = setup();
    say(&e, "starte cs");
    say(&e, "öffne Discord");
    assert_eq!(pf.take_calls(), vec!["open steam://rungameid/730", "open discord://"]);
}

#[test]
fn unknown_target_is_not_found() {
    let (e, pf) = setup();
    let r = say(&e, "starte Minecraft");
    assert_eq!(r.text(), "Minecraft finde ich nicht.");
    assert!(pf.take_calls().is_empty());
}

#[test]
fn unsure_match_asks_back() {
    let (e, pf) = setup();
    let q = say(&e, "starte witscher");
    assert_eq!(q.text(), "Meinst du The Witcher 3: Wild Hunt?");
    assert!(pf.take_calls().is_empty());
    let r = say(&e, "ja");
    assert_eq!(r.text(), "The Witcher 3: Wild Hunt startet.");
    assert_eq!(pf.take_calls(), vec!["open steam://rungameid/292030"]);
}

#[test]
fn voice_ignores_leftover_first_word() {
    let (e, pf) = setup();
    say(&e, "Personality, öffne Deadlock");
    assert_eq!(pf.take_calls(), vec!["open steam://rungameid/1422450"]);
    // Getippt gilt das nicht – da ist der Text genau so gemeint.
    let r = e.command("r2", "quatsch öffne Deadlock", Source::Text);
    assert_eq!(r.text(), "Das kann ich noch nicht. Frag mich „Was kannst du?“");
}

#[test]
fn tier1_runs_immediately() {
    let (e, pf) = setup();
    assert_eq!(say(&e, "Lautstärke 30").text(), "Lautstärke 30.");
    assert_eq!(say(&e, "lauter").text(), "Lautstärke 40.");
    say(&e, "nächstes Lied");
    say(&e, "such deadlock build guides");
    assert_eq!(
        pf.take_calls(),
        vec!["volume 30", "volume 40", "media Next", "open https://www.google.com/search?q=deadlock+build+guides"]
    );
}

#[test]
fn tier2_needs_confirmation_and_voice_yes_works() {
    let (e, pf) = setup();
    let q = say(&e, "fahr den PC herunter");
    assert!(matches!(q, ServerMessage::ConfirmRequest { tier: 2, pc_only: false, .. }));
    assert_eq!(q.text(), "Soll ich den PC herunterfahren? Sag ja oder nein.");
    assert!(pf.take_calls().is_empty(), "darf vor Bestätigung nichts tun");
    say(&e, "ja");
    assert_eq!(pf.take_calls(), vec!["power Shutdown"]);
}

#[test]
fn tier2_no_cancels() {
    let (e, pf) = setup();
    say(&e, "starte den PC neu");
    assert_eq!(say(&e, "nein").text(), "Okay, lasse ich.");
    assert_eq!(say(&e, "ja").text(), "Es gibt gerade nichts zu bestätigen.");
    assert!(pf.take_calls().is_empty());
}

#[test]
fn close_app_is_tier2() {
    let (e, pf) = setup();
    *pf.windows.lock().unwrap() =
        vec![WindowInfo { id: 7, pid: 70, title: "#general - Discord".into(), process: "Discord.exe".into() }];
    let q = say(&e, "schließe Discord");
    assert_eq!(q.text(), "Soll ich Discord beenden? Sag ja oder nein.");
    say(&e, "ja");
    assert_eq!(pf.take_calls(), vec!["close Discord.exe"]);
    assert_eq!(say(&e, "schließe Spotify").text(), "Spotify läuft gerade nicht.");
}

#[test]
fn remote_tier2_must_be_clicked_on_pc() {
    let (e, pf) = setup();
    let remote = Source::Remote { device: "iPhone".into() };
    let q = e.invoke("r9", "system.power", json!({"mode": "shutdown"}), remote.clone());
    assert!(matches!(q, ServerMessage::ConfirmRequest { pc_only: true, .. }));
    let cid = confirm_id(&q);

    // iPhone darf nicht selbst bestätigen, Sprache am PC auch nicht.
    let r = e.handle(ClientMessage::Confirm { confirm_id: cid.clone(), accept: true }, remote);
    assert_eq!(r.text(), "Das musst du am PC per Klick bestätigen.");
    assert_eq!(say(&e, "ja").text(), "Das musst du am PC per Klick bestätigen.");
    assert!(pf.take_calls().is_empty());

    // Klick im Overlay am PC.
    e.handle(ClientMessage::Confirm { confirm_id: cid, accept: true }, Source::Text);
    assert_eq!(pf.take_calls(), vec!["power Shutdown"]);
}

#[test]
fn claude_tier2_must_be_clicked_on_pc() {
    let (e, pf) = setup();
    let q = e.invoke("r1", "system.power", json!({"mode": "restart"}), Source::Claude);
    assert!(matches!(q, ServerMessage::ConfirmRequest { pc_only: true, .. }));
    assert_eq!(e.confirm(None, true, ConfirmVia::Spoken).text(), "Das musst du am PC per Klick bestätigen.");
    assert!(pf.take_calls().is_empty());
    e.confirm(None, true, ConfirmVia::PcClick);
    assert_eq!(pf.take_calls(), vec!["power Restart"]);
}

#[test]
fn whitelist_and_param_validation() {
    let (e, pf) = setup();
    let r = e.invoke("r1", "shell.exec", json!({"cmd": "del C:\\"}), Source::Text);
    assert!(matches!(r, ServerMessage::Reply { ok: false, .. }));
    let r = e.invoke("r1", "volume.set", json!({"level": 500}), Source::Text);
    assert!(matches!(r, ServerMessage::Reply { ok: false, .. }));
    let r = e.invoke("r1", "volume.set", json!({"level": 20, "extra": 1}), Source::Text);
    assert!(matches!(r, ServerMessage::Reply { ok: false, .. }));
    let r = e.invoke("r1", "web.open", json!({"url": "file:///C:/Windows/System32"}), Source::Text);
    assert!(matches!(r, ServerMessage::Reply { ok: false, .. }));
    assert!(pf.take_calls().is_empty());
}

#[test]
fn kill_switch_blocks_everything() {
    let (e, pf) = setup();
    say(&e, "fahr den PC herunter");
    e.set_enabled(false);
    assert_eq!(say(&e, "öffne Deadlock").text(), "Ich bin gerade ausgeschaltet.");
    assert_eq!(
        e.invoke("r1", "volume.set", json!({"level": 10}), Source::Text).text(),
        "Ich bin gerade ausgeschaltet."
    );
    e.set_enabled(true);
    // Offene Frage wurde beim Not-Aus verworfen.
    assert_eq!(say(&e, "ja").text(), "Es gibt gerade nichts zu bestätigen.");
    assert!(pf.take_calls().is_empty());
}

#[test]
fn confirmation_expires() {
    let cfg = Config { confirm_timeout_secs: 0, ..Config::default() };
    let (e, pf) = setup_with(cfg, None);
    say(&e, "fahr den PC herunter");
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert_eq!(say(&e, "ja").text(), "Zu spät, ich lasse es.");
    assert!(pf.take_calls().is_empty());
}

#[test]
fn not_understood() {
    let (e, _) = setup();
    let r = say(&e, "Wie wird das Wetter?");
    assert_eq!(
        r,
        ServerMessage::reply("r1", false, "Das kann ich noch nicht. Frag mich „Was kannst du?“", Mood::Confused, None)
    );
}

#[test]
fn everything_is_logged_to_jsonl() {
    let dir = std::env::temp_dir().join(format!("panco-log-{}", std::process::id()));
    let (e, _) = setup_with(Config::default(), Some(dir.clone()));
    say(&e, "öffne Deadlock");
    say(&e, "fahr den pc herunter");
    say(&e, "nein");
    say(&e, "quatsch");

    let file = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
    let lines: Vec<serde_json::Value> =
        std::fs::read_to_string(&file).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let outcomes: Vec<&str> = lines.iter().map(|l| l["outcome"].as_str().unwrap()).collect();
    assert_eq!(outcomes, vec!["done", "pending", "cancelled", "not_understood"]);
    assert_eq!(lines[0]["source"], "voice");
    assert_eq!(lines[0]["text"], "öffne Deadlock");
    assert_eq!(lines[0]["action"], "app.launch");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn time_date_and_help() {
    let (e, _) = setup();
    let r = say(&e, "Wie viel Uhr ist es gerade?");
    assert!(r.text().starts_with("Es ist ") && r.text().ends_with(" Uhr."), "{}", r.text());
    assert!(say(&e, "Welcher Tag ist heute?").text().starts_with("Heute ist "));
    assert!(say(&e, "Was kannst du?").text().starts_with("Ich kann "));
    assert!(say(&e, "Lies mir vor, was im Editor steht").text().contains("Claude"));
}

//! Gedächtnis, Regeln, eigene Befehle und Kontext ("nochmal", "schließ es") über die Engine.

use panco_core::config::{Config, FileSettings};
use panco_core::engine::{Engine, Source};
use panco_core::log::ActionLog;
use panco_core::platform::mock::MockPlatform;
use panco_core::platform::AppVolume;
use panco_core::protocol::ServerMessage;
use panco_core::steam::Game;
use std::path::PathBuf;
use std::sync::Arc;

struct Env {
    root: PathBuf,
    e: Engine,
    pf: Arc<MockPlatform>,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn env(name: &str) -> Env {
    let root = std::env::temp_dir().join(format!("panco-mem-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let s = |p: PathBuf| p.to_string_lossy().to_string();
    let files = FileSettings {
        allowed_folders: vec![s(root.join("docs"))],
        notes_dir: s(root.join("docs/Notizen")),
        trash_dir: s(root.join("Papierkorb")),
        trash_days: 30,
        backup_enabled: false,
        backup_dir: s(root.join("Backups")),
        backup_keep: 3,
    };
    let pf = Arc::new(MockPlatform::default());
    *pf.volume.lock().unwrap() = 50;
    *pf.apps.lock().unwrap() = vec![AppVolume { process: "Spotify.exe".into(), level: 80, muted: false }];
    let e = Engine::new(pf.clone(), Config { files, ..Config::default() }, ActionLog::new(Some(root.join("logs"))));
    e.set_data_dir(root.join("data"));
    e.set_games(vec![Game { app_id: 1422450, name: "Deadlock".into() }]);
    Env { root, e, pf }
}

fn say(e: &Engine, text: &str) -> ServerMessage {
    e.command("r", text, Source::Voice)
}

#[test]
fn remember_list_forget_and_undo() {
    let env = env("facts");
    let e = &env.e;
    assert_eq!(
        say(e, "Merk dir, mein Lieblingsspiel ist Deadlock").text(),
        "Gemerkt: mein Lieblingsspiel ist Deadlock"
    );
    assert!(say(e, "Was weißt du über mich?").text().contains("Lieblingsspiel ist Deadlock"));
    // Passwörter nie ins Gedächtnis
    let r = say(e, "Merk dir mein Passwort ist hunter2");
    assert!(r.text().contains("geschützte Notizen"));
    assert!(!say(e, "Was weißt du über mich?").text().contains("hunter2"));
    // Vergessen + rückgängig
    let r = say(e, "Vergiss das mit dem Lieblingsspiel");
    assert!(r.text().starts_with("Vergessen"), "{r:?}");
    assert!(say(e, "Was weißt du über mich?").text().contains("noch nichts gemerkt"));
    say(e, "mach das rückgängig");
    assert!(say(e, "Was weißt du über mich?").text().contains("Lieblingsspiel"));
}

#[test]
fn rules_run_after_launch_and_game_start() {
    let env = env("rules");
    let e = &env.e;
    let r = say(e, "Wenn ich Deadlock starte, starte auch Discord");
    assert_eq!(r.text(), "Okay. Wenn Deadlock startet: starte Discord.");
    // Spiel-Regeln laufen, wenn die Spiel-Erkennung das Spiel sieht (egal wie es gestartet wurde)
    let ran = e.on_game_started("Deadlock");
    assert_eq!(ran.len(), 1);
    assert!(env.pf.take_calls().iter().any(|c| c == "open discord://"));
    // Programm-Regel: direkt nach "starte …"
    say(e, "Wenn ich Discord starte, stell Spotify auf 20 Prozent");
    let r = say(e, "starte Discord");
    assert!(r.text().contains("Regel: Spotify ist jetzt bei 20 %."), "{}", r.text());
    // Regeln dürfen nichts mit Rückfrage tun
    let r = say(e, "Wenn ich ein Spiel starte, fahr den PC herunter");
    assert!(r.text().contains("braucht eine Bestätigung"), "{}", r.text());
    // "Wenn ein Spiel startet" gilt für jedes Spiel
    say(e, "Wenn ich ein Spiel starte, mach Spotify leiser");
    e.on_game_started("Counter-Strike 2");
    assert_eq!(env.pf.apps.lock().unwrap()[0].level, 5, "20 - 15");
}

#[test]
fn macros_with_confirmation_and_context() {
    let env = env("macros");
    let e = &env.e;
    let r = say(e, "Neuer Befehl Feierabend: Lautstärke 30 und sperr den PC");
    assert!(r.text().starts_with("Neuer Befehl „Feierabend“"), "{}", r.text());
    env.pf.take_calls();
    say(e, "Panco, Feierabend");
    assert_eq!(env.pf.take_calls(), vec!["volume 30", "lock"]);

    // Höchste Stufe zählt: einmal alle Schritte zeigen und bestätigen
    say(e, "Neuer Befehl Schluss: Lautstärke 10 und fahr den PC herunter");
    let q = say(e, "Schluss");
    assert!(matches!(q, ServerMessage::ConfirmRequest { tier: 2, .. }), "{q:?}");
    assert!(q.text().contains("Lautstärke auf 10") && q.text().contains("herunterfahren"), "{}", q.text());
    assert!(env.pf.take_calls().is_empty(), "vor dem Ja passiert nichts");
    say(e, "ja");
    assert_eq!(env.pf.take_calls(), vec!["volume 10", "power Shutdown"]);

    // Eingebaute Befehle lassen sich nicht überschreiben
    assert!(say(e, "Neuer Befehl lauter: Lautstärke 100").text().contains("schon ein Panco-Befehl"));
    // Löschen + rückgängig
    say(e, "Lösche den Befehl Feierabend");
    assert!(say(e, "Feierabend").text().contains("kann ich noch nicht"));
    say(e, "rückgängig");
    env.pf.take_calls();
    say(e, "Feierabend");
    assert_eq!(env.pf.take_calls(), vec!["volume 30", "lock"]);

    // Kontext: "nochmal" und "schließ es"
    say(e, "starte Discord");
    say(e, "nochmal");
    assert_eq!(env.pf.take_calls(), vec!["open discord://", "open discord://"]);
    *env.pf.windows.lock().unwrap() = vec![panco_core::platform::WindowInfo {
        id: 7,
        pid: 70,
        title: "Discord".into(),
        process: "Discord.exe".into(),
    }];
    let q = say(e, "schließ es");
    assert!(q.text().contains("Discord"), "{}", q.text());
    say(e, "ja");
    assert_eq!(env.pf.take_calls(), vec!["close Discord.exe"]);
}

#[test]
fn confirmations_can_be_skipped_only_at_the_pc() {
    let env = env("quiet");
    let e = &env.e;
    let pc_only = |m: &ServerMessage| matches!(m, ServerMessage::ConfirmRequest { .. });
    // Standard: fragt nach
    assert!(pc_only(&say(e, "fahr den PC herunter")));
    say(e, "nein");
    // "Frag mich 10 Minuten nicht mehr" → läuft sofort, aber nur vom PC aus
    assert!(say(e, "Frag mich 10 Minuten nicht mehr").text().contains("10 Minuten"));
    assert!(e.quiet_secs() > 590);
    assert!(matches!(say(e, "fahr den PC herunter"), ServerMessage::Reply { ok: true, .. }));
    assert_eq!(env.pf.take_calls(), vec!["power Shutdown"]);
    let remote = e.command("r", "fahr den PC herunter", Source::Remote { device: "iPhone".into() });
    assert!(pc_only(&remote), "iPhone fragt immer");
    e.confirm(None, false, panco_core::engine::ConfirmVia::PcClick);
    assert!(
        pc_only(&e.invoke("r", "system.power", serde_json::json!({"mode": "restart"}), Source::Claude)),
        "Claude fragt immer"
    );
    e.confirm(None, false, panco_core::engine::ConfirmVia::PcClick);
    say(e, "frag wieder nach");
    assert!(pc_only(&say(e, "fahr den PC herunter")));
    // "Ja, und frag 10 Minuten nicht mehr" beantwortet die Frage und setzt die Pause
    assert!(matches!(say(e, "Ja, und frag die nächsten 10 Minuten nicht mehr"), ServerMessage::Reply { ok: true, .. }));
    assert!(e.quiet_secs() > 0);
    say(e, "frag wieder nach");

    // Pro Aktion abschaltbar (Einstellungen)
    let mut cfg = e.config();
    cfg.skip_confirm = vec!["system.power".into()];
    e.set_config(cfg);
    assert!(matches!(say(e, "starte den PC neu"), ServerMessage::Reply { ok: true, .. }));
    assert!(pc_only(&e.command("r", "starte den PC neu", Source::Remote { device: "iPhone".into() })));
}

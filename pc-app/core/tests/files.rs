//! Dateien, Papierkorb, Rückgängig und Notizen über die Engine – mit echten Dateien in einem Temp-Ordner.

use panco_core::config::{Config, FileSettings};
use panco_core::engine::{ConfirmVia, Engine, Source};
use panco_core::log::ActionLog;
use panco_core::platform::mock::MockPlatform;
use panco_core::protocol::ServerMessage;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Env {
    root: PathBuf,
    engine: Engine,
    pf: Arc<MockPlatform>,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn env(name: &str) -> Env {
    let root = std::env::temp_dir().join(format!("panco-files-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let s = |p: &Path| p.to_string_lossy().to_string();
    let docs = root.join("Dokumente");
    let downloads = root.join("Downloads");
    for (rel, days) in [
        ("Downloads/rechnung-telekom-okt.pdf", 3),
        ("Downloads/Rechnung_Telekom_Sept.pdf", 33),
        ("Dokumente/Bewerbung.pdf", 40),
        ("Dokumente/Urlaub.jpg", 1),
        ("Dokumente/setup.exe", 5),
        ("Andere/geheim.txt", 2),
    ] {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        let f = fs::File::create(&p).unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86_400)).unwrap();
    }
    fs::create_dir_all(docs.join("Steuer")).unwrap();
    let files = FileSettings {
        allowed_folders: vec![s(&docs), s(&downloads)],
        notes_dir: s(&docs.join("Panco-Notizen")),
        trash_dir: s(&root.join("Papierkorb")),
        trash_days: 30,
        backup_enabled: false,
        backup_dir: s(&root.join("Backups")),
        backup_keep: 3,
    };
    let pf = Arc::new(MockPlatform::default());
    *pf.volume.lock().unwrap() = 50;
    let engine =
        Engine::new(pf.clone(), Config { files, ..Config::default() }, ActionLog::new(Some(root.join("logs"))));
    engine.set_data_dir(root.join("data"));
    engine.set_games(vec![]);
    Env { root, engine, pf }
}

fn say(e: &Engine, text: &str) -> ServerMessage {
    e.command("r1", text, Source::Voice)
}

fn tier(m: &ServerMessage) -> Option<u8> {
    match m {
        ServerMessage::ConfirmRequest { tier, .. } => Some(*tier),
        _ => None,
    }
}

#[test]
fn search_finds_latest_invoice_and_opens_on_yes() {
    let env = env("search");
    let q = say(&env.engine, "Panco, finde meine letzte Rechnung");
    assert_eq!(tier(&q), Some(1), "Rückfrage, kein Ausführen: {q:?}");
    assert!(q.text().starts_with("Gefunden: rechnung-telekom-okt.pdf (Downloads, vor 3 Tagen)"), "{}", q.text());
    // Auswahl: beide Rechnungen + "Im Ordner zeigen"
    let labels: Vec<&str> = q.choices().iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, vec!["rechnung-telekom-okt.pdf", "Rechnung_Telekom_Sept.pdf", "Im Ordner zeigen"]);
    say(&env.engine, "ja");
    let calls = env.pf.take_calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("open ") && calls[0].ends_with("rechnung-telekom-okt.pdf"), "{calls:?}");
}

#[test]
fn open_my_latest_invoice_opens_directly() {
    let env = env("open");
    let r = say(&env.engine, "öffne meine letzte Rechnung");
    assert!(matches!(r, ServerMessage::Reply { ok: true, .. }), "{r:?}");
    assert!(env.pf.take_calls()[0].ends_with("rechnung-telekom-okt.pdf"));
    assert!(!r.choices().is_empty(), "Alternativen werden angeboten");
}

#[test]
fn delete_goes_to_trash_and_undo_restores() {
    let env = env("delete");
    let file = env.root.join("Downloads/Rechnung_Telekom_Sept.pdf");
    let q = say(&env.engine, "lösche die Rechnung Telekom Sept");
    assert_eq!(tier(&q), Some(2), "Löschen braucht Bestätigung: {q:?}");
    assert!(file.exists());
    let r = say(&env.engine, "ja");
    assert_eq!(r.text(), "Rechnung_Telekom_Sept.pdf liegt im Papierkorb (30 Tage).");
    assert!(!file.exists());
    assert_eq!(env.engine.trash().list().len(), 1);

    assert_eq!(say(&env.engine, "mach das rückgängig").text(), "Rechnung_Telekom_Sept.pdf ist wieder da (Downloads).");
    assert!(file.exists());
    assert!(env.engine.trash().list().is_empty());
    assert_eq!(say(&env.engine, "rückgängig").text(), "Da gibt es nichts rückgängig zu machen.");
}

#[test]
fn move_and_rename_with_undo() {
    let env = env("move");
    let q = say(&env.engine, "verschiebe die Bewerbung nach Steuer");
    assert_eq!(tier(&q), Some(2));
    assert_eq!(say(&env.engine, "ja").text(), "Bewerbung.pdf ist jetzt in Steuer.");
    assert!(env.root.join("Dokumente/Steuer/Bewerbung.pdf").exists());

    let q = say(&env.engine, "Benenne die Datei Urlaub in Sommer 2026 um");
    assert_eq!(q.text(), "Soll ich Urlaub.jpg in Sommer 2026.jpg umbenennen? Sag ja oder nein.");
    say(&env.engine, "ja");
    assert!(env.root.join("Dokumente/Sommer 2026.jpg").exists(), "Endung bleibt erhalten");

    // Rückgängig in umgekehrter Reihenfolge
    say(&env.engine, "rückgängig");
    assert!(env.root.join("Dokumente/Urlaub.jpg").exists());
    say(&env.engine, "rückgängig");
    assert!(env.root.join("Dokumente/Bewerbung.pdf").exists());
}

#[test]
fn zones_protect_system_trash_and_programs() {
    let env = env("zones");
    let e = &env.engine;
    // Außerhalb der freigegebenen Ordner: Öffnen nur mit Rückfrage
    let other = env.root.join("Andere/geheim.txt");
    assert_eq!(tier(&e.invoke("r", "file.open", json!({"path": other}), Source::Text)), Some(2));
    // Programme werden nicht per Datei-Befehl gestartet
    let exe = env.root.join("Dokumente/setup.exe");
    let r = e.invoke("r", "file.open", json!({"path": exe}), Source::Text);
    assert!(matches!(r, ServerMessage::Reply { ok: false, .. }), "{r:?}");
    // Panco-Papierkorb und freigegebene Wurzelordner sind tabu
    fs::create_dir_all(env.root.join("Papierkorb/x")).unwrap();
    let r = e.invoke("r", "file.delete", json!({"path": env.root.join("Papierkorb/x")}), Source::Text);
    assert!(matches!(r, ServerMessage::Reply { ok: false, .. }));
    let r = e.invoke("r", "file.delete", json!({"path": env.root.join("Dokumente")}), Source::Text);
    assert!(matches!(r, ServerMessage::Reply { ok: false, .. }));
    if cfg!(unix) {
        let r = e.invoke("r", "file.delete", json!({"path": "/etc/hostname"}), Source::Text);
        assert_eq!(tier(&r), Some(3), "Systemdateien: nur ausdrücklich am PC");
        let r =
            e.invoke("r", "file.delete", json!({"path": "/etc/hostname"}), Source::Remote { device: "iPhone".into() });
        assert!(matches!(r, ServerMessage::Reply { ok: false, .. }), "Stufe 3 nie remote");
    }
    // Vom iPhone: Löschen nur per Klick am PC
    let r = e.invoke("r", "file.delete", json!({"path": "Bewerbung"}), Source::Remote { device: "iPhone".into() });
    assert!(matches!(r, ServerMessage::ConfirmRequest { pc_only: true, .. }), "{r:?}");
    assert!(env.pf.take_calls().is_empty());
}

#[test]
fn folder_create_and_restore_from_trash() {
    let env = env("mkdir");
    let docs = env.root.join("Dokumente");
    let r = env.engine.invoke("r", "folder.create", json!({"name": "Projekte", "parent": docs}), Source::Text);
    assert_eq!(r.text(), "Ordner Projekte liegt in Dokumente.");
    assert!(docs.join("Projekte").is_dir());
    let r = env.engine.invoke("r", "folder.create", json!({"name": "a/b", "parent": docs}), Source::Text);
    assert!(matches!(r, ServerMessage::Reply { ok: false, .. }), "ungültige Namen werden abgelehnt");

    // Rückgängig legt den neuen Ordner in den Papierkorb, Wiederherstellen holt ihn zurück
    say(&env.engine, "rückgängig");
    assert!(!docs.join("Projekte").exists());
    let id = env.engine.trash().list()[0].id.clone();
    env.engine.invoke("r", "trash.restore", json!({"id": id}), Source::Text);
    assert!(docs.join("Projekte").is_dir());
}

#[test]
fn notes_encrypted_never_read_and_redacted_in_log() {
    let env = env("notes");
    let e = &env.engine;
    assert_eq!(
        say(e, "Erstelle eine neue Notiz und nenne sie Passwörter").text(),
        "Notiz „Passwörter“ ist angelegt und verschlüsselt."
    );
    assert_eq!(say(e, "Notiz: Steam hunter2").text(), "Steht in „Passwörter“.");
    let r = say(e, "Lies die Notiz Passwörter vor");
    assert!(matches!(r, ServerMessage::Reply { ok: false, .. }));
    assert!(!r.text().contains("hunter2"));

    say(e, "Neue Notiz Einkauf: Milch");
    say(e, "Füge Brot zur Notiz Einkauf hinzu");
    assert_eq!(say(e, "Lies die Notiz Einkauf vor").text(), "Einkauf: Milch, Brot");
    let found = say(e, "Welche Notizen habe ich?");
    assert_eq!(found.choices().len(), 2);
    assert!(found.choices().iter().any(|c| c.label == "🔒 Passwörter"));

    // Klartext steht weder im Protokoll noch in der verschlüsselten Datei
    let logs: String =
        fs::read_dir(env.root.join("logs")).unwrap().map(|f| fs::read_to_string(f.unwrap().path()).unwrap()).collect();
    assert!(!logs.contains("hunter2") && !logs.contains("Milch") && !logs.contains("Brot"), "{logs}");
    let enc = fs::read(env.root.join("Dokumente/Panco-Notizen/Passwörter.md.panco")).unwrap();
    assert!(!String::from_utf8_lossy(&enc).contains("hunter2"));
}

#[test]
fn volume_undo_and_confirm_via_click_still_works() {
    let env = env("volume");
    say(&env.engine, "Lautstärke 20");
    assert_eq!(*env.pf.volume.lock().unwrap(), 20);
    assert_eq!(say(&env.engine, "mach das rückgängig").text(), "Lautstärke wieder 50.");
    assert_eq!(*env.pf.volume.lock().unwrap(), 50);
    // Zweites Rückgängig findet nichts mehr (der Undo-Eintrag selbst ist nicht umkehrbar)
    assert_eq!(say(&env.engine, "rückgängig").text(), "Da gibt es nichts rückgängig zu machen.");
    // Overlay-Klick auf einen Suchtreffer (invoke aus einer Auswahl)
    let q = say(&env.engine, "finde meine Bewerbung");
    let choice = q.choices()[0].clone();
    let r = env.engine.invoke("r2", &choice.action, choice.params, Source::Text);
    assert!(matches!(r, ServerMessage::Reply { ok: true, .. }));
    // offene Rückfrage bleibt bestehen, bis sie beantwortet wird – "nein" schließt sie
    assert_eq!(env.engine.confirm(None, false, ConfirmVia::PcClick).text(), "Okay, lasse ich.");
}

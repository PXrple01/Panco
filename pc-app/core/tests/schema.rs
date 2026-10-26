//! Hält Rust und `/shared/schema` synchron.

use panco_core::actions::registry;
use panco_core::catalog;
use panco_core::protocol::{AvatarState, ClientMessage, MicState, Mood, ServerMessage};
use serde_json::json;

#[test]
fn catalog_and_registry_match() {
    let mut cat: Vec<&str> = catalog::all().iter().map(|a| a.id.as_str()).collect();
    let mut reg: Vec<&str> = registry().iter().map(|a| a.id()).collect();
    cat.sort();
    reg.sort();
    assert_eq!(cat, reg, "actions.json und actions::registry() müssen dieselben Aktionen enthalten");
}

#[test]
fn catalog_param_schemas_are_valid() {
    for a in catalog::all() {
        assert!(jsonschema::validator_for(&a.params).is_ok(), "Parameter-Schema von {} ist ungültig", a.id);
        assert!(!a.description.is_empty() && !a.title.is_empty());
    }
}

#[test]
fn messages_match_shared_schema() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../../shared/schema/messages.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).expect("messages.schema.json ist gültig");

    let server = [
        ServerMessage::reply("1", true, "Deadlock startet.", Mood::Game, Some("app.launch")),
        ServerMessage::reply("1", false, "Nö.", Mood::Confused, None),
        ServerMessage::ConfirmRequest {
            id: "1".into(),
            confirm_id: "c1".into(),
            text: "Soll ich?".into(),
            tier: 2,
            expires_in_ms: 30000,
            pc_only: true,
            detail: Some("- alt\n+ neu".into()),
            choices: vec![],
        },
        ServerMessage::Stream {
            id: "3".into(),
            text: "Das ist".into(),
            done: false,
            activity: Some("liest den Bildschirm".into()),
        },
        ServerMessage::reply("2", true, "Gefunden.", Mood::Info, Some("file.search")).with_choices(vec![
            panco_core::protocol::Choice {
                label: "Rechnung.pdf".into(),
                detail: Some("Downloads · gestern".into()),
                action: "file.open".into(),
                params: json!({"path": "C:/x/Rechnung.pdf"}),
            },
        ]),
        ServerMessage::State {
            avatar: AvatarState::Listening,
            mic: MicState::Recording,
            enabled: true,
            level: Some(0.4),
        },
    ];
    for m in server {
        let v = serde_json::to_value(&m).unwrap();
        assert!(validator.is_valid(&v), "ServerMessage passt nicht zum Schema: {v}");
    }

    let client = [
        json!({"type": "command", "id": "1", "text": "öffne deadlock"}),
        json!({"type": "invoke", "id": "2", "action": "volume.set", "params": {"level": 3}}),
        json!({"type": "confirm", "confirm_id": "c1", "accept": false}),
    ];
    for v in client {
        assert!(validator.is_valid(&v), "Beispiel passt nicht zum Schema: {v}");
        serde_json::from_value::<ClientMessage>(v).expect("Rust versteht die Nachricht");
    }
    // Unbekannte Felder werden abgelehnt (Schema und Rust).
    let bad = json!({"type": "command", "id": "1", "text": "x", "shell": "rm"});
    assert!(!validator.is_valid(&bad));
    assert!(serde_json::from_value::<ClientMessage>(bad).is_err());
}

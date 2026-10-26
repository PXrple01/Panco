//! Spotify über die Engine – das Netzwerk ist simuliert (MockPlatform::http_replies).
//! Ein Test für alles, weil das Zugriffstoken prozessweit im Speicher liegt.

use panco_core::config::Config;
use panco_core::engine::{Engine, Source};
use panco_core::log::ActionLog;
use panco_core::platform::mock::MockPlatform;
use panco_core::spotify;
use std::sync::Arc;

fn reply(pf: &MockPlatform, status: u16, body: &str) {
    pf.http_replies.lock().unwrap().push_back((status, body.into()));
}

#[test]
fn search_play_now_and_errors() {
    let root = std::env::temp_dir().join(format!("panco-spotify-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let pf = Arc::new(MockPlatform::default());
    let cfg = Config { spotify_client_id: "abc123".into(), ..Config::default() };
    let e = Engine::new(pf.clone(), cfg, ActionLog::new(Some(root.join("logs"))));
    e.set_data_dir(root.join("data"));
    let say = |t: &str| e.command("r", t, Source::Voice).text().to_string();

    // Nicht verbunden → kein einziger Netzwerkzugriff
    assert!(say("spiel Lo-Fi Beats auf Spotify").contains("noch nicht verbunden"));
    assert!(pf.http_requests.lock().unwrap().is_empty());

    // Anmeldung (Code → Tokens), Refresh-Token landet im Tresor
    reply(&pf, 200, r#"{"access_token":"A1","refresh_token":"R1","expires_in":3600}"#);
    spotify::exchange(pf.as_ref(), "abc123", "CODE", "VERIFIER").unwrap();
    assert_eq!(pf.secrets.lock().unwrap().get(spotify::SECRET).map(String::as_str), Some("R1"));
    let body = pf.http_requests.lock().unwrap()[0].body.clone().unwrap();
    assert!(body.contains("code_verifier=VERIFIER") && body.contains("client_id=abc123"));

    // Suchen + abspielen
    reply(&pf, 200, r#"{"tracks":{"items":[{"uri":"spotify:track:1","name":"Snowman","artists":[{"name":"WYS"}]}]}}"#);
    reply(&pf, 204, "");
    assert_eq!(say("spiel Lo-Fi Beats auf Spotify"), "Läuft: Snowman – WYS.");
    {
        let reqs = pf.http_requests.lock().unwrap();
        assert!(reqs[1].url.contains("/search?q=lo%20fi%20beats&type=track"));
        assert!(reqs[1].headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer A1"));
        assert_eq!(reqs[2].method, "PUT");
        assert!(reqs[2].body.as_deref().unwrap().contains("spotify:track:1"));
    }

    // Playlist → context_uri
    reply(&pf, 200, r#"{"playlists":{"items":[{"uri":"spotify:playlist:9","name":"Gaming Mix"}]}}"#);
    reply(&pf, 204, "");
    assert_eq!(say("spiel die Playlist Gaming Mix auf Spotify"), "Läuft: Gaming Mix.");
    assert!(pf.http_requests.lock().unwrap()[4].body.as_deref().unwrap().contains("context_uri"));

    // Fehler verständlich erklärt
    reply(&pf, 200, r#"{"tracks":{"items":[]}}"#);
    assert!(say("spiel asdfgh auf Spotify").contains("nichts gefunden"));
    reply(&pf, 200, r#"{"tracks":{"items":[{"uri":"spotify:track:2","name":"X"}]}}"#);
    reply(
        &pf,
        403,
        r#"{"error":{"status":403,"message":"Player command failed: Premium required","reason":"PREMIUM_REQUIRED"}}"#,
    );
    assert!(say("spiel X auf Spotify").contains("Premium"));
    reply(&pf, 200, r#"{"tracks":{"items":[{"uri":"spotify:track:2","name":"X"}]}}"#);
    reply(&pf, 404, r#"{"error":{"status":404,"reason":"NO_ACTIVE_DEVICE"}}"#);
    assert!(say("spiel X auf Spotify").contains("keinem Gerät"));

    // Was läuft gerade?
    reply(&pf, 200, r#"{"is_playing":true,"item":{"name":"Snowman","artists":[{"name":"WYS"}]}}"#);
    assert_eq!(say("Was läuft gerade?"), "Gerade läuft „Snowman“ von WYS.");
    reply(&pf, 204, "");
    assert_eq!(say("Was läuft gerade?"), "Auf Spotify läuft gerade nichts.");

    // Abgelaufenes Token (401) → beim nächsten Mal per Refresh-Token erneuert
    reply(&pf, 401, r#"{"error":{"status":401}}"#);
    assert!(say("Was läuft gerade?").contains("nicht verbunden"));
    reply(&pf, 200, r#"{"access_token":"A2","expires_in":3600}"#);
    reply(&pf, 204, "");
    assert_eq!(say("Was läuft gerade?"), "Auf Spotify läuft gerade nichts.");
    {
        let reqs = pf.http_requests.lock().unwrap();
        let n = reqs.len();
        assert!(reqs[n - 2].body.as_deref().unwrap().contains("grant_type=refresh_token&refresh_token=R1"));
        assert!(reqs[n - 1].headers.iter().any(|(_, v)| v == "Bearer A2"));
    }
    // Refresh-Token behalten, wenn Spotify kein neues schickt
    assert_eq!(pf.secrets.lock().unwrap().get(spotify::SECRET).map(String::as_str), Some("R1"));

    // Trennen → Tresor leer, wieder offline
    spotify::disconnect(pf.as_ref());
    assert!(pf.secrets.lock().unwrap().is_empty());
    let before = pf.http_requests.lock().unwrap().len();
    assert!(say("Was läuft gerade?").contains("noch nicht verbunden"));
    assert_eq!(pf.http_requests.lock().unwrap().len(), before);

    // Protokoll: Quelle + Aktion, keine Tokens
    let log =
        std::fs::read_to_string(std::fs::read_dir(root.join("logs")).unwrap().next().unwrap().unwrap().path()).unwrap();
    assert!(log.contains("spotify.play"));
    assert!(!log.contains("A1") && !log.contains("R1"));
    let _ = std::fs::remove_dir_all(root);
}

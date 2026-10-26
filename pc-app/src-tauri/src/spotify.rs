//! Spotify verbinden (optional): Browser-Anmeldung per PKCE, Rückruf an 127.0.0.1:8977.
//!
//! Der kleine Empfänger läuft nur während der Anmeldung (höchstens 3 Minuten), hört nur auf
//! 127.0.0.1 und nimmt genau einen gültigen Rückruf an. Das Refresh-Token geht in den Windows-Tresor.

use panco_core::config::config_path;
use panco_core::spotify;
use serde::Serialize;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

const PORT: u16 = 8977;
const WAIT: Duration = Duration::from_secs(180);

#[derive(Serialize)]
pub struct SpotifyStatus {
    client_id: String,
    connected: bool,
}

#[tauri::command]
pub fn spotify_status(app: AppHandle) -> SpotifyStatus {
    let engine = crate::engine(&app);
    SpotifyStatus {
        client_id: engine.config().spotify_client_id,
        connected: spotify::is_connected(engine.platform().as_ref()),
    }
}

#[tauri::command]
pub fn spotify_disconnect(app: AppHandle) {
    let engine = crate::engine(&app);
    spotify::disconnect(engine.platform().as_ref());
    engine.log_system("done", "Spotify getrennt (Anmeldung aus dem Tresor gelöscht)");
}

#[tauri::command]
pub async fn spotify_connect(app: AppHandle, client_id: String) -> Result<(), String> {
    let client_id = client_id.trim().to_string();
    if client_id.len() != 32 || !client_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("Die Client-ID hat 32 Zeichen (0–9, a–f) – du findest sie im Spotify-Entwicklerportal.".into());
    }
    let engine = crate::engine(&app);
    let mut cfg = engine.config();
    if cfg.spotify_client_id != client_id {
        cfg.spotify_client_id = client_id.clone();
        cfg.save(&config_path()).map_err(|e| e.to_string())?;
        engine.set_config(cfg);
    }
    tauri::async_runtime::spawn_blocking(move || {
        let pf = engine.platform().clone();
        let p = spotify::pkce(pf.as_ref())?;
        let listener = TcpListener::bind(("127.0.0.1", PORT))
            .map_err(|_| format!("Port {PORT} ist belegt – läuft die Anmeldung schon in einem anderen Fenster?"))?;
        pf.open(&spotify::authorize_url(&client_id, &p), &[]).map_err(|e| e.to_string())?;
        let code = wait_for_code(listener, &p.state)?;
        spotify::exchange(pf.as_ref(), &client_id, &code, &p.verifier).map_err(|e| match e {
            spotify::SpotifyError::Other(m) => m,
            _ => "Spotify hat die Anmeldung abgelehnt – stimmt die Redirect-URI im Entwicklerportal?".into(),
        })?;
        engine.log_system("done", "Spotify verbunden");
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Wartet blockierend auf den Rückruf. Nach `WAIT` weckt ein Hilfsthread das `accept` mit einer
/// eigenen Verbindung auf – kein Abfragen in einer Schleife.
fn wait_for_code(listener: TcpListener, state: &str) -> Result<String, String> {
    let done = Arc::new(AtomicBool::new(false));
    let timed_out = Arc::new(AtomicBool::new(false));
    {
        let (done, timed_out) = (done.clone(), timed_out.clone());
        std::thread::spawn(move || {
            std::thread::park_timeout(WAIT);
            if !done.load(Ordering::SeqCst) {
                timed_out.store(true, Ordering::SeqCst);
                let _ = TcpStream::connect(("127.0.0.1", PORT));
            }
        });
    }
    let result = loop {
        let Ok((mut stream, _)) = listener.accept() else { continue };
        if timed_out.load(Ordering::SeqCst) {
            break Err("Keine Antwort von Spotify innerhalb von 3 Minuten – bitte nochmal verbinden.".to_string());
        }
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]);
        let path = head.lines().next().and_then(|l| l.split(' ').nth(1)).unwrap_or("");
        let Some(query) = path.strip_prefix("/callback?") else {
            respond(&mut stream, "404 Not Found", "");
            continue;
        };
        let get = |key: &str| {
            query.split('&').find_map(|kv| kv.strip_prefix(key).and_then(|v| v.strip_prefix('='))).map(percent_decode)
        };
        if get("state").as_deref() != Some(state) {
            respond(&mut stream, "400 Bad Request", "Diese Anmeldung gehört nicht zu Panco.");
            continue;
        }
        if let Some(code) = get("code") {
            respond(&mut stream, "200 OK", "Spotify ist mit Panco verbunden. Du kannst dieses Fenster schließen.");
            break Ok(code);
        }
        respond(&mut stream, "200 OK", "Anmeldung abgebrochen.");
        break Err("Anmeldung bei Spotify abgebrochen.".to_string());
    };
    done.store(true, Ordering::SeqCst);
    result
}

fn respond(stream: &mut TcpStream, status: &str, text: &str) {
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>Panco</title><body style=\"background:#000;color:#e9e2ff;font:18px system-ui;display:grid;place-items:center;height:90vh\"><p>{text}</p>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        match b[i] {
            b'%' if i + 2 < b.len() => {
                if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                    out.push(h << 4 | l);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

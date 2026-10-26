//! Spotify Web-API (optional): "Spiel Lo-Fi Beats auf Spotify", "Was läuft gerade?".
//!
//! - Anmeldung per PKCE (kein App-Geheimnis nötig): eigene Client-ID aus dem Spotify-Entwicklerportal,
//!   Weiterleitung auf http://127.0.0.1:8977/callback. Das Refresh-Token liegt nur im Windows-Tresor.
//! - Verbindungen nur, wenn du etwas fragst – kein Hintergrund-Abruf.
//! - Abspielen per Web-API verlangt Spotify Premium; Pause/Weiter gehen immer über die Medientasten.

use crate::platform::{HttpRequest, Platform};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Mutex;

pub const SECRET: &str = "Panco/Spotify";
pub const REDIRECT: &str = "http://127.0.0.1:8977/callback";
const SCOPES: &str = "user-read-currently-playing user-read-playback-state user-modify-playback-state";

/// Zugriffstoken (1 h gültig) nur im Speicher.
static ACCESS: Mutex<Option<(String, i64)>> = Mutex::new(None);

#[derive(Debug, PartialEq)]
pub enum SpotifyError {
    NotConnected,
    Premium,
    NoDevice,
    NothingFound(String),
    Other(String),
}

fn b64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (*b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

pub fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

pub fn pkce(pf: &dyn Platform) -> Result<Pkce, String> {
    let verifier = b64url(&pf.random_bytes(48).map_err(|e| e.to_string())?);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let state = b64url(&pf.random_bytes(12).map_err(|e| e.to_string())?);
    Ok(Pkce { verifier, challenge, state })
}

pub fn authorize_url(client_id: &str, p: &Pkce) -> String {
    format!(
        "https://accounts.spotify.com/authorize?client_id={}&response_type=code&redirect_uri={}&code_challenge_method=S256&code_challenge={}&state={}&scope={}",
        url_encode(client_id),
        url_encode(REDIRECT),
        p.challenge,
        p.state,
        url_encode(SCOPES)
    )
}

fn token_request(pf: &dyn Platform, form: &str) -> Result<Value, SpotifyError> {
    let req = HttpRequest {
        method: "POST",
        url: "https://accounts.spotify.com/api/token".into(),
        headers: vec![("Content-Type".into(), "application/x-www-form-urlencoded".into())],
        body: Some(form.into()),
    };
    let (status, text) = pf.http(&req).map_err(|e| SpotifyError::Other(e.to_string()))?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status != 200 {
        // abgelaufene/entzogene Anmeldung → neu verbinden
        return Err(if status == 400 {
            SpotifyError::NotConnected
        } else {
            SpotifyError::Other(format!("Anmeldung: HTTP {status}"))
        });
    }
    Ok(v)
}

fn store(pf: &dyn Platform, v: &Value) -> Result<String, SpotifyError> {
    let access = v["access_token"].as_str().ok_or_else(|| SpotifyError::Other("kein Zugriffstoken".into()))?;
    let expires = crate::time::unix_now() + v["expires_in"].as_i64().unwrap_or(3600) - 60;
    if let Some(refresh) = v["refresh_token"].as_str() {
        pf.secret_set(SECRET, Some(refresh)).map_err(|e| SpotifyError::Other(e.to_string()))?;
    }
    *ACCESS.lock().unwrap() = Some((access.to_string(), expires));
    Ok(access.to_string())
}

/// Nach der Anmeldung im Browser: Code gegen Tokens tauschen (Refresh-Token → Tresor).
pub fn exchange(pf: &dyn Platform, client_id: &str, code: &str, verifier: &str) -> Result<(), SpotifyError> {
    let form = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
        url_encode(code),
        url_encode(REDIRECT),
        url_encode(client_id),
        url_encode(verifier)
    );
    store(pf, &token_request(pf, &form)?).map(|_| ())
}

pub fn is_connected(pf: &dyn Platform) -> bool {
    pf.secret_get(SECRET).ok().flatten().is_some()
}

pub fn disconnect(pf: &dyn Platform) {
    let _ = pf.secret_set(SECRET, None);
    *ACCESS.lock().unwrap() = None;
}

fn access_token(pf: &dyn Platform, client_id: &str) -> Result<String, SpotifyError> {
    if let Some((token, until)) = ACCESS.lock().unwrap().clone() {
        if until > crate::time::unix_now() {
            return Ok(token);
        }
    }
    let refresh = pf.secret_get(SECRET).ok().flatten().ok_or(SpotifyError::NotConnected)?;
    if client_id.is_empty() {
        return Err(SpotifyError::NotConnected);
    }
    let form =
        format!("grant_type=refresh_token&refresh_token={}&client_id={}", url_encode(&refresh), url_encode(client_id));
    store(pf, &token_request(pf, &form)?)
}

fn api(
    pf: &dyn Platform,
    client_id: &str,
    method: &'static str,
    path: &str,
    body: Option<Value>,
) -> Result<(u16, Value), SpotifyError> {
    let token = access_token(pf, client_id)?;
    let mut headers = vec![("Authorization".into(), format!("Bearer {token}"))];
    if body.is_some() {
        headers.push(("Content-Type".into(), "application/json".into()));
    }
    let req = HttpRequest {
        method,
        url: format!("https://api.spotify.com/v1{path}"),
        headers,
        body: body.map(|b| b.to_string()),
    };
    let (status, text) = pf.http(&req).map_err(|e| SpotifyError::Other(e.to_string()))?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    match status {
        401 => {
            *ACCESS.lock().unwrap() = None;
            Err(SpotifyError::NotConnected)
        }
        403 if text.contains("PREMIUM") || text.contains("Premium") => Err(SpotifyError::Premium),
        404 if text.contains("NO_ACTIVE_DEVICE") || path.starts_with("/me/player/play") => Err(SpotifyError::NoDevice),
        s if s >= 400 => Err(SpotifyError::Other(format!("Spotify antwortet mit HTTP {s}"))),
        s => Ok((s, v)),
    }
}

/// Suchen und abspielen. `kind`: track (Standard), playlist, artist, album. Liefert "Titel – Künstler".
pub fn play(pf: &dyn Platform, client_id: &str, query: &str, kind: &str) -> Result<String, SpotifyError> {
    let kind = match kind {
        "playlist" | "artist" | "album" => kind,
        _ => "track",
    };
    let (_, found) = api(pf, client_id, "GET", &format!("/search?q={}&type={kind}&limit=1", url_encode(query)), None)?;
    let item =
        found[format!("{kind}s")]["items"].get(0).cloned().ok_or_else(|| SpotifyError::NothingFound(query.into()))?;
    let uri = item["uri"].as_str().ok_or_else(|| SpotifyError::NothingFound(query.into()))?.to_string();
    let name = item["name"].as_str().unwrap_or(query).to_string();
    let label = match item["artists"].get(0).and_then(|a| a["name"].as_str()) {
        Some(artist) if kind != "artist" => format!("{name} – {artist}"),
        _ => name,
    };
    let body =
        if kind == "track" { serde_json::json!({ "uris": [uri] }) } else { serde_json::json!({ "context_uri": uri }) };
    api(pf, client_id, "PUT", "/me/player/play", Some(body))?;
    Ok(label)
}

/// Was läuft gerade? (Titel, Künstler, spielt?) – `None`, wenn nichts läuft.
pub fn now_playing(pf: &dyn Platform, client_id: &str) -> Result<Option<(String, String, bool)>, SpotifyError> {
    let (status, v) = api(pf, client_id, "GET", "/me/player/currently-playing", None)?;
    if status == 204 || v["item"].is_null() {
        return Ok(None);
    }
    let title = v["item"]["name"].as_str().unwrap_or("?").to_string();
    let artist = v["item"]["artists"].get(0).and_then(|a| a["name"].as_str()).unwrap_or("").to_string();
    Ok(Some((title, artist, v["is_playing"].as_bool().unwrap_or(false))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding() {
        // RFC 7636, Anhang B
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(b64url(&Sha256::digest(verifier.as_bytes())), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(url_encode("Lo-Fi Beats & Co"), "Lo-Fi%20Beats%20%26%20Co");
        assert_eq!(b64url(b"f"), "Zg");
    }
}

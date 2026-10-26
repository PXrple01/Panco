//! web.search / web.open – Standardbrowser, nur https.

use super::*;
use crate::texts::t;
use serde::Deserialize;

pub struct WebSearch;
pub struct WebOpen;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchP {
    query: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenP {
    url: String,
}

/// Prozent-Kodierung für Query-Strings (RFC 3986, Leerzeichen als +).
pub(crate) fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Nur https, keine Leer-/Steuerzeichen, kein Benutzer@Host-Trick.
pub(crate) fn validate_https(url: &str) -> Result<String, ActionError> {
    let url = url.trim();
    let rest = url.strip_prefix("https://").ok_or_else(|| ActionError::InvalidParams("nur https-Adressen".into()))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let valid_host = !host.is_empty()
        && !host.contains('@')
        && host.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | ':'));
    if url.len() > 500 || url.chars().any(|c| c.is_whitespace() || c.is_control()) || !valid_host {
        return Err(ActionError::InvalidParams("ungültige Adresse".into()));
    }
    Ok(url.to_string())
}

impl Action for WebSearch {
    fn id(&self) -> &'static str {
        "web.search"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: SearchP = super::params(params)?;
        let query = clean_text(&p.query, 200)?;
        let url = validate_https(&ctx.config.search_url.replace("{q}", &url_encode(&query)))?;
        Ok(Plan::instant(format!("nach {query} suchen"), move |pf| {
            pf.open(&url, &[])?;
            Ok(Done { text: t("web_search", &[("query", &query)]), mood: Mood::Info })
        }))
    }
}

impl Action for WebOpen {
    fn id(&self) -> &'static str {
        "web.open"
    }
    fn prepare(&self, params: &Value, _: &Ctx) -> Result<Plan, ActionError> {
        let p: OpenP = super::params(params)?;
        let url = validate_https(&p.url)?;
        Ok(Plan::instant(format!("{url} öffnen"), move |pf| {
            pf.open(&url, &[])?;
            Ok(Done::ok(t("web_open", &[])))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes() {
        assert_eq!(url_encode("deadlock build guides"), "deadlock+build+guides");
        assert_eq!(url_encode("ä&b"), "%C3%A4%26b");
    }

    #[test]
    fn https_only() {
        assert!(validate_https("https://www.twitch.tv/x?y=1").is_ok());
        assert!(validate_https("http://example.com").is_err());
        assert!(validate_https("file:///C:/Windows").is_err());
        assert!(validate_https("javascript:alert(1)").is_err());
        assert!(validate_https("https://evil.com@good.com").is_err());
        assert!(validate_https("https://a.com/ b").is_err());
    }
}

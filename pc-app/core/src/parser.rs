//! Regelbasierter Parser: schnell, offline, deterministisch.
//! Eingabe ist bereits mit `normalize::command` bereinigt (klein, ohne "Panco", ohne Füllwörter).
//!
//! Neue Regel: in `RULES` an passender Stelle einfügen (Reihenfolge = Priorität).

use crate::normalize;
use regex::{Captures, Regex};
use serde_json::{json, Value};
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq)]
pub struct Intent {
    pub action: &'static str,
    pub params: Value,
}

fn intent(action: &'static str, params: Value) -> Option<Intent> {
    Some(Intent { action, params })
}

type Build = fn(&Captures) -> Option<Intent>;

struct Rule {
    re: Regex,
    build: Build,
}

fn rule(pattern: &str, build: Build) -> Rule {
    Rule { re: Regex::new(pattern).expect("Parser-Regel ist gültig"), build }
}

/// Artikel/Possessiva vor Zielen entfernen: "den browser" → "browser".
fn target(raw: &str) -> String {
    // Leerzeichen anhängen, damit ein alleinstehender Artikel ("starte den") ganz verschwindet.
    let padded = format!("{} ", raw.trim());
    let mut s = padded.as_str();
    for prefix in
        ["den ", "die ", "das ", "dem ", "der ", "mein ", "meine ", "meinen ", "ein ", "eine ", "einen ", "mir "]
    {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest;
        }
    }
    s.trim().to_string()
}

fn cap<'a>(c: &'a Captures, i: usize) -> &'a str {
    c.get(i).map_or("", |m| m.as_str())
}

const PC: &str = r"(?:den |die |das )?(?:pc|computer|rechner|kiste|laptop)";

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        // --- System: Energie (vor "starte X", damit "starte den pc neu" nicht als Spiel gilt) ---
        rule(&format!(r"^(?:{PC} )?(?:herunterfahren|runterfahren|ausschalten|ausmachen)$"), |_| {
            intent("system.power", json!({"mode": "shutdown"}))
        }),
        rule(&format!(r"^(?:fahr|fahre|schalt|schalte|mach|mache) {PC} (?:herunter|runter|aus)$"), |_| {
            intent("system.power", json!({"mode": "shutdown"}))
        }),
        rule(r"^(?:fahr|fahre) (?:herunter|runter)$", |_| intent("system.power", json!({"mode": "shutdown"}))),
        rule(&format!(r"^(?:(?:{PC} )?(?:neu starten|neustarten|neustart)|starte? {PC} neu|reboot)$"), |_| {
            intent("system.power", json!({"mode": "restart"}))
        }),
        rule(&format!(r"^(?:{PC} (?:in den )?)?(?:ruhemodus|standby|schlafmodus|energiesparmodus)$"), |_| {
            intent("system.power", json!({"mode": "sleep"}))
        }),
        rule(
            &format!(
                r"^(?:schick|schicke|versetz|versetze|setz|setze) {PC} in den (?:ruhemodus|standby|schlafmodus|energiesparmodus)$|^(?:leg|lege) {PC} schlafen$"
            ),
            |_| intent("system.power", json!({"mode": "sleep"})),
        ),
        // --- Sperren / Screenshot ---
        rule(
            &format!(
                r"^(?:(?:{PC}|(?:den )?bildschirm) sperren|sperr|sperre|sperr(?:e)? {PC}|sperr(?:e)? (?:den )?bildschirm|bildschirmsperre|pc sperren)$"
            ),
            |_| intent("system.lock", json!({})),
        ),
        rule(r"(?:screenshot|screen shot|bildschirmfoto|bildschirmaufnahme)", |_| {
            intent("screen.screenshot", json!({}))
        }),
        // --- Ton ---
        rule(r"^(?:ton aus|stumm|stummschalten|schalt(?:e)? (?:den ton |alles )?stumm|mute|lautlos)$", |_| {
            intent("volume.mute", json!({"mute": true}))
        }),
        rule(r"^(?:ton (?:wieder )?an|stumm aus|unmute|ton einschalten|laut schalten)$", |_| {
            intent("volume.mute", json!({"mute": false}))
        }),
        rule(
            r"^(?:(?:stell|stelle|mach|mache|setz|setze|dreh|drehe) )?(?:die |den )?(?:lautstärke|volume|ton|sound)(?: auf| zu)? (\S+)(?: prozent| %)?$",
            |c| {
                let n = normalize::number(cap(c, 1))?;
                (n <= 100).then(|| Intent { action: "volume.set", params: json!({"level": n}) })
            },
        ),
        rule(
            r"^(?:(?:mach|mache|dreh|drehe|stell|stelle) )?(?:(?:es|den ton|die musik|die lautstärke|alles|das) )?(?:(viel|deutlich|etwas|ein bisschen|ein wenig|bisschen) )?(lauter|leiser)(?: machen| drehen| stellen)?$",
            |c| {
                let step = match cap(c, 1) {
                    "viel" | "deutlich" => 20,
                    "" => 10,
                    _ => 5,
                };
                let delta = if cap(c, 2) == "lauter" { step } else { -step };
                intent("volume.change", json!({"delta": delta}))
            },
        ),
        rule(r"^(?:lautstärke|ton) (hoch|rauf|runter)$", |c| {
            intent("volume.change", json!({"delta": if cap(c, 1) == "runter" { -10 } else { 10 }}))
        }),
        // --- Medien ---
        rule(
            r"^(?:pause|pausieren|pausiere|play|abspielen|weiter|weiterspielen|fortsetzen|spiel(?:e)? weiter|musik (?:an|aus|weiter|pausieren)|(?:spiel|spiele) (?:die )?musik(?: ab| weiter)?|pausier(?:e)? (?:die )?musik|mach(?:e)? (?:die )?musik (?:an|aus))$",
            |_| intent("media.control", json!({"command": "play_pause"})),
        ),
        rule(
            r"^(?:nächster|nächstes|nächste|skip|skippen|überspringen|überspring(?:e)?)(?: (?:song|lied|titel|track|das lied|den song))?$|^(?:spring|springe|geh|gehe) zum nächsten(?: \S+)?$",
            |_| intent("media.control", json!({"command": "next"})),
        ),
        rule(
            r"^(?:vorheriger|vorheriges|vorherige|letzter|letztes|letzte)(?: (?:song|lied|titel|track))?$|^(?:zurück|ein (?:lied|song|titel) zurück)$",
            |_| intent("media.control", json!({"command": "previous"})),
        ),
        rule(r"^(?:stopp|stop|stoppe die musik|musik stoppen)$", |_| {
            intent("media.control", json!({"command": "stop"}))
        }),
        // --- Web ---
        rule(r"^(?:öffne|öffnen|geh auf|gehe auf) (?:die )?(?:webseite|website|seite|url) (.+)$", |c| {
            let site = cap(c, 1).trim();
            let looks_like_domain = !site.contains(' ') && site.contains('.');
            if looks_like_domain {
                let host = site.trim_start_matches("https://").trim_start_matches("http://");
                intent("web.open", json!({"url": format!("https://{host}")}))
            } else {
                intent("web.search", json!({"query": site}))
            }
        }),
        rule(
            r"^(?:such|suche|google|googel|googele|recherchier|recherchiere)(?: (?:mir|im internet|im web|online|bei google|nach))* (.+?)(?: (?:im internet|im web|online|bei google|raus))?$",
            |c| intent("web.search", json!({"query": cap(c, 1)})),
        ),
        // --- Fenster ---
        rule(
            r"^(?:wechsel|wechsle|wechsele|wechseln|geh|gehe|spring|springe|schalt|schalte) (?:zu|zum|zur|in|auf) (.+)$",
            |c| intent("window.focus", json!({"target": target(cap(c, 1))})),
        ),
        rule(
            r"^(?:hol|hole) (.+?) (?:nach vorne|in den vordergrund)$|^(.+?) (?:nach vorne|in den vordergrund)$",
            |c| {
                let t = if c.get(1).is_some() { cap(c, 1) } else { cap(c, 2) };
                intent("window.focus", json!({"target": target(t)}))
            },
        ),
        // --- Programme beenden (Stufe 2) ---
        rule(r"^(?:schließ|schließe|schliess|schliesse|beend|beende|kill|kille) (.+)$", |c| {
            intent("app.close", json!({"target": target(cap(c, 1))}))
        }),
        rule(r"^(?:mach|mache) (.+?) zu$|^(.+?) (?:schließen|beenden|zumachen)$", |c| {
            let t = if c.get(1).is_some() { cap(c, 1) } else { cap(c, 2) };
            intent("app.close", json!({"target": target(t)}))
        }),
        // --- Starten (zuletzt: sehr allgemein) ---
        rule(
            r"^(?:öffne|öffnen|starte|start|lade|lad|spiel|spiele|spielen|zock|zocke|zocken|mach|mache|ruf|rufe|lass uns|ich will|ich möchte|ich würde gern|ich würde gerne) (.+?)(?: (?:auf|an|starten|öffnen|spielen|zocken|aufmachen|auf machen))?$",
            |c| intent("app.launch", json!({"target": target(cap(c, 1))})),
        ),
        rule(r"^(.+?) (?:starten|öffnen|spielen|zocken)$", |c| {
            intent("app.launch", json!({"target": target(cap(c, 1))}))
        }),
    ]
});

/// Versteht einen bereits normalisierten Befehl. `None` = nicht verstanden.
pub fn parse(normalized: &str) -> Option<Intent> {
    if normalized.is_empty() {
        return None;
    }
    RULES.iter().find_map(|r| r.re.captures(normalized).and_then(|c| (r.build)(&c))).filter(|i| {
        // Leere Ziele ("starte den") nicht durchlassen.
        !i.params.as_object().is_some_and(|m| m.values().any(|v| v.as_str() == Some("")))
    })
}

/// Ja/Nein-Antwort auf eine Rückfrage erkennen.
pub fn yes_no(normalized: &str) -> Option<bool> {
    const YES: [&str; 14] = [
        "ja",
        "jawohl",
        "jep",
        "jo",
        "yes",
        "ok",
        "okay",
        "klar",
        "genau",
        "mach",
        "mach das",
        "mach es",
        "bestätigt",
        "los",
    ];
    const NO: [&str; 11] =
        ["nein", "nee", "ne", "nö", "no", "abbrechen", "abbruch", "lass es", "lieber nicht", "doch nicht", "nicht"];
    let s = normalized.trim_end_matches('.');
    let s = s.strip_suffix(" danke").unwrap_or(s);
    if YES.contains(&s) || s.starts_with("ja ") {
        Some(true)
    } else if NO.contains(&s) || s.starts_with("nein ") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> Option<(&'static str, Value)> {
        parse(&normalize::command(text)).map(|i| (i.action, i.params))
    }

    #[test]
    fn launch() {
        assert_eq!(p("Panco, öffne Deadlock"), Some(("app.launch", json!({"target": "deadlock"}))));
        assert_eq!(p("Starte bitte Counter Strike"), Some(("app.launch", json!({"target": "counter strike"}))));
        assert_eq!(p("lass uns elden ring spielen"), Some(("app.launch", json!({"target": "elden ring"}))));
        assert_eq!(p("ich will Rocket League zocken"), Some(("app.launch", json!({"target": "rocket league"}))));
        assert_eq!(p("mach den Browser auf"), Some(("app.launch", json!({"target": "browser"}))));
        assert_eq!(p("Discord starten"), Some(("app.launch", json!({"target": "discord"}))));
        assert_eq!(p("starte den"), None);
    }

    #[test]
    fn power_is_not_launch() {
        assert_eq!(p("starte den PC neu"), Some(("system.power", json!({"mode": "restart"}))));
        assert_eq!(p("fahr den Computer herunter"), Some(("system.power", json!({"mode": "shutdown"}))));
        assert_eq!(p("PC herunterfahren"), Some(("system.power", json!({"mode": "shutdown"}))));
        assert_eq!(p("Ruhemodus"), Some(("system.power", json!({"mode": "sleep"}))));
        assert_eq!(p("schick den pc in den ruhemodus"), Some(("system.power", json!({"mode": "sleep"}))));
        assert_eq!(p("neustart"), Some(("system.power", json!({"mode": "restart"}))));
    }

    #[test]
    fn volume() {
        assert_eq!(p("Lautstärke 30"), Some(("volume.set", json!({"level": 30}))));
        assert_eq!(p("Lautstärke auf dreißig Prozent"), Some(("volume.set", json!({"level": 30}))));
        assert_eq!(p("stell die Lautstärke auf 45%"), Some(("volume.set", json!({"level": 45}))));
        assert_eq!(p("Lautstärke 300"), None);
        assert_eq!(p("lauter"), Some(("volume.change", json!({"delta": 10}))));
        assert_eq!(p("mach etwas leiser"), Some(("volume.change", json!({"delta": -5}))));
        assert_eq!(p("viel lauter"), Some(("volume.change", json!({"delta": 20}))));
        assert_eq!(p("Ton aus"), Some(("volume.mute", json!({"mute": true}))));
        assert_eq!(p("ton wieder an"), Some(("volume.mute", json!({"mute": false}))));
    }

    #[test]
    fn media() {
        assert_eq!(p("Pause"), Some(("media.control", json!({"command": "play_pause"}))));
        assert_eq!(p("spiel weiter"), Some(("media.control", json!({"command": "play_pause"}))));
        assert_eq!(p("nächstes Lied"), Some(("media.control", json!({"command": "next"}))));
        assert_eq!(p("skip"), Some(("media.control", json!({"command": "next"}))));
        assert_eq!(p("vorheriger Titel"), Some(("media.control", json!({"command": "previous"}))));
    }

    #[test]
    fn web() {
        assert_eq!(p("such Deadlock Build Guides"), Some(("web.search", json!({"query": "deadlock build guides"}))));
        assert_eq!(p("google mir elden ring bosse"), Some(("web.search", json!({"query": "elden ring bosse"}))));
        assert_eq!(p("öffne die Webseite twitch.tv"), Some(("web.open", json!({"url": "https://twitch.tv"}))));
    }

    #[test]
    fn windows_and_close() {
        assert_eq!(p("wechsel zu Discord"), Some(("window.focus", json!({"target": "discord"}))));
        assert_eq!(p("hol Spotify nach vorne"), Some(("window.focus", json!({"target": "spotify"}))));
        assert_eq!(p("schließe Discord"), Some(("app.close", json!({"target": "discord"}))));
        assert_eq!(p("mach den Browser zu"), Some(("app.close", json!({"target": "browser"}))));
        assert_eq!(p("Spotify beenden"), Some(("app.close", json!({"target": "spotify"}))));
    }

    #[test]
    fn misc() {
        assert_eq!(p("PC sperren"), Some(("system.lock", json!({}))));
        assert_eq!(p("sperr den Bildschirm"), Some(("system.lock", json!({}))));
        assert_eq!(p("mach einen Screenshot"), Some(("screen.screenshot", json!({}))));
        assert_eq!(p("blablub"), None);
        assert_eq!(p(""), None);
    }

    #[test]
    fn answers() {
        assert_eq!(yes_no("ja"), Some(true));
        assert_eq!(yes_no("ja bitte"), Some(true));
        assert_eq!(yes_no(&normalize::command("Okay.")), Some(true));
        assert_eq!(yes_no("nein danke"), Some(false));
        assert_eq!(yes_no("deadlock"), None);
    }
}

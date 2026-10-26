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
/// Programmname (1–2 Wörter) für Lautstärke pro App.
const APP: &str = r"([a-zäöüß0-9][a-zäöüß0-9.\-]*(?: [a-zäöüß0-9.\-]+)??)";

/// Wörter, die kein Programm sind ("mach es leiser" ist die Gesamtlautstärke).
fn app_name(raw: &str) -> Option<String> {
    let t = target(raw);
    let no = [
        "es",
        "das",
        "ton",
        "alles",
        "lautstärke",
        "lautstärk",
        "volume",
        "sound",
        "musik",
        "die musik",
        "den ton",
        "mich",
        "dich",
        "etwas",
        "viel",
        "bisschen",
        "ein bisschen",
        "deutlich",
        "ganz",
        "wieder",
        "",
    ];
    (!no.contains(&t.as_str()) && !t.starts_with("lautstärke")).then_some(t)
}

fn app_step(word: &str) -> i64 {
    match word {
        "viel" | "deutlich" => 25,
        "" => 15,
        _ => 8,
    }
}

const UNDO: &str = r"r(?:ü|u|ö|ue|oe)ckg(?:ä|a|ae)ngig";

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
        rule(
            r"^(?:(?:mach|mache|nimm|nehme)(?: mir)? )?(?:einen |ein )?(?:screenshot|screen shot|bildschirmfoto|bildschirmaufnahme)(?: machen)?$",
            |_| intent("screen.screenshot", json!({})),
        ),
        // --- Rückgängig ---
        rule(
            // "röckgängig", "macht das …": typische Whisper-Varianten
            &format!(
                r"^(?:(?:mach|mache|macht) (?:das |es |den letzten schritt )?{UNDO}|{UNDO}(?: machen)?|undo|(?:hol|hole|stell|stelle) (?:das|es) wieder (?:her|zurück))$"
            ),
            |_| intent("panco.undo", json!({})),
        ),
        // --- Uhrzeit, Datum, Hilfe ---
        rule(
            r"^(?:wie spät|wie viel uhr|wieviel uhr|was ist die uhrzeit)(?: ist es| haben wir| isses)?(?: gerade| grad| eigentlich)?$|^(?:sag mir |nenn mir )?(?:die )?uhrzeit$",
            |_| intent("info.time", json!({"what": "time"})),
        ),
        rule(
            r"^(?:welcher tag ist (?:heute|es)|welchen tag haben wir(?: heute)?|was (?:ist|haben wir) heute für ein (?:tag|datum)|welches datum (?:ist )?(?:heute|haben wir(?: heute)?)|der wievielte ist heute|(?:sag mir )?(?:das )?(?:heutige )?datum)$",
            |_| intent("info.time", json!({"what": "date"})),
        ),
        rule(
            r"^(?:was kannst du(?: alles)?(?: machen| tun)?|hilfe|was kann ich (?:dich )?fragen|wie funktionierst du|was kann panco)$",
            |_| intent("panco.help", json!({})),
        ),
        // "Lies mir vor, was im Editor steht" – lokal per Texterkennung (nur auf Anfrage).
        rule(
            r"^(?:(?:lies|lese|les|läs|lees)(?: mir)?(?: vor)? was (?:im|in dem|auf dem|in meinem|am|da im) (editor|fenster|bildschirm|screen|monitor|browser|chat)|was steht (?:da|hier|im|in dem|auf dem) ?(editor|fenster|bildschirm|screen|monitor|browser|chat)?)\b.*$",
            |c| {
                let w = c.get(1).or(c.get(2)).map_or("", |m| m.as_str());
                let target = if matches!(w, "bildschirm" | "screen" | "monitor") { "screen" } else { "window" };
                intent("screen.read", json!({"target": target}))
            },
        ),
        // --- Gedächtnis zeigen, wiederholen ---
        rule(
            r"^(?:was weißt du(?: alles)?(?: über mich| von mir)?|was hast du dir (?:alles )?gemerkt|was merkst du dir|zeig(?:e)? (?:mir )?(?:dein|das) gedächtnis|welche (?:befehle|regeln) (?:hast du|habe ich|kennst du|gibt es))$",
            |_| intent("memory.list", json!({})),
        ),
        rule(
            r"^(?:frag(?:e)? (?:mich )?(?:die nächsten |für )?(\S+) minuten nicht(?: mehr)?(?: nach)?|keine rückfragen(?: für| die nächsten)? (\S+) minuten)$",
            |c| {
                let n = normalize::number(if c.get(1).is_some() { cap(c, 1) } else { cap(c, 2) })?;
                intent("panco.quiet", json!({"minutes": n.min(120)}))
            },
        ),
        rule(r"^(?:frag(?:e)? (?:mich )?(?:wieder|ab jetzt wieder)(?: nach)?|rückfragen (?:wieder )?an)$", |_| {
            intent("panco.quiet", json!({"minutes": 0}))
        }),
        rule(
            r"^(?:nochmal|noch mal|noch einmal|nochmal bitte|wiederhol(?:e)?(?: das)?|mach das (?:nochmal|noch mal|noch einmal)|das gleiche (?:nochmal|noch mal))$",
            |_| intent("panco.repeat", json!({})),
        ),
        // --- Lesen auf Anfrage (lokal, ohne Claude) ---
        rule(
            r"^(?:lies|lese|lies mir) (?:mir )?(?:den |das )?(bildschirm|fenster|text auf dem bildschirm)(?: vor)?$|^was steht (?:auf dem |im )(bildschirm|fenster)$",
            |c| {
                let what =
                    if c.get(1).or(c.get(2)).is_some_and(|m| m.as_str() == "bildschirm") { "screen" } else { "window" };
                intent("screen.read", json!({"target": what}))
            },
        ),
        rule(
            r"^(?:was (?:steht|ist) in (?:der|meiner) zwischenablage|lies (?:mir )?(?:die )?zwischenablage(?: vor)?)$",
            |_| intent("clipboard.read", json!({})),
        ),
        rule(
            r"^(?:welches|was für ein) (?:fenster|programm) (?:ist|habe ich) (?:gerade )?(?:vorne|offen|aktiv)$",
            |_| intent("window.active", json!({})),
        ),
        // --- Spotify (Web-API, optional) ---
        rule(
            r"^(?:was (?:für ein (?:lied|song)(?: läuft| ist das)?|läuft)(?: gerade| grad)?(?: (?:auf|bei) spotify)?|welches lied (?:ist das|läuft(?: gerade)?)|wie heißt (?:das|dieses) lied)$",
            |_| intent("spotify.now", json!({})),
        ),
        rule(
            r"^(?:spiel|spiele|spielt|play|mach) (?:mir )?(?:(?:die )?(playlist)|(?:das )?(album)|(?:musik|lieder|songs) von|(?:den )?(?:song|titel|track)|das lied)? ?(.+?) (?:auf|bei|in|über|mit) spotify(?: ab| an)?$|^spotify (?:spiel|spiele|play) (?:mir )?(.+)$",
            |c| {
                let query = if c.get(3).is_some() { cap(c, 3) } else { cap(c, 4) };
                let text = c.get(0).map_or("", |m| m.as_str());
                let kind = if c.get(1).is_some() {
                    "playlist"
                } else if c.get(2).is_some() {
                    "album"
                } else if text.contains(" von ") && !text.contains("song") && !text.contains("lied ") {
                    "artist"
                } else {
                    "track"
                };
                intent("spotify.play", json!({"query": query.trim(), "kind": kind}))
            },
        ),
        // --- Spielzeit ---
        rule(
            r"^(?:was|welches spiel) (?:spiele|zocke|spiel) ich (?:gerade|grad|da)?$|^läuft (?:gerade )?(?:ein|irgendein) spiel$",
            |_| intent("info.playtime", json!({"period": "now"})),
        ),
        rule(
            r"^(?:wie (?:lange|viel)|wieviel) (?:habe|hab) ich (heute|diese woche|die woche|in dieser woche|insgesamt|überhaupt) (?:schon )?(?:gespielt|gezockt)$|^(?:meine )?spielzeit(?: (heute|diese woche|insgesamt))?$",
            |c| {
                let when = if c.get(1).is_some() { cap(c, 1) } else { cap(c, 2) };
                let period = match when {
                    "heute" | "" => "today",
                    "insgesamt" | "überhaupt" => "total",
                    _ => "week",
                };
                intent("info.playtime", json!({"period": period}))
            },
        ),
        rule(
            r"^(?:wie (?:lange|viel)|wieviel) (?:habe|hab) ich (?:schon |insgesamt )?(.+?) (?:gespielt|gezockt)$",
            |c| intent("info.playtime", json!({"period": "total", "game": target(cap(c, 1))})),
        ),
        // --- Lautstärke pro App (vor der Gesamtlautstärke) ---
        rule(
            &format!(
                r"^(?:(?:mach|mache|dreh|drehe|stell|stelle) )?(?:(?:den ton|die lautstärke) von )?{APP} (?:(viel|deutlich|etwas|ein bisschen|ein wenig|bisschen) )?(lauter|leiser)(?: machen| drehen| stellen)?$"
            ),
            |c| {
                let step = app_step(cap(c, 2));
                let delta = if cap(c, 3) == "lauter" { step } else { -step };
                intent("volume.app", json!({"app": app_name(cap(c, 1))?, "delta": delta}))
            },
        ),
        rule(
            &format!(
                r"^(?:(?:stell|stelle|mach|mache|setz|setze|dreh|drehe) )?(?:(?:den ton|die lautstärke) von )?{APP}(?: lautstärke)? (?:auf|zu) (\S+)(?: prozent| %)?$"
            ),
            |c| {
                let n = normalize::number(cap(c, 2))?;
                (n <= 100).then_some(())?;
                intent("volume.app", json!({"app": app_name(cap(c, 1))?, "level": n}))
            },
        ),
        rule(
            &format!(
                r"^(?:(?:mach|mache|schalt|schalte) )?{APP} (?:stumm|lautlos)(?: schalten| machen)?$|^(?:mute|stummschalten) {APP}$"
            ),
            |c| {
                let app = if c.get(1).is_some() { cap(c, 1) } else { cap(c, 2) };
                intent("volume.app", json!({"app": app_name(app)?, "mute": true}))
            },
        ),
        rule(
            &format!(
                r"^(?:(?:mach|mache|schalt|schalte) )?(?:den ton von )?{APP} (?:wieder )?(?:laut|hörbar)(?: schalten| machen)?$"
            ),
            |c| intent("volume.app", json!({"app": app_name(cap(c, 1))?, "mute": false})),
        ),
        // --- Ton ---
        rule(r"^(?:ton aus|stumm|stummschalten|schalt(?:e)? (?:den ton |alles )?stumm|mute|lautlos)$", |_| {
            intent("volume.mute", json!({"mute": true}))
        }),
        rule(r"^(?:ton (?:wieder )?an|stumm aus|unmute|ton einschalten|laut schalten)$", |_| {
            intent("volume.mute", json!({"mute": false}))
        }),
        rule(
            r"^(?:(?:stell|stelle|mach|mache|setz|setze|dreh|drehe) )?(?:die |den )?(?:lautstärke|lautstärk|volume|ton|sound)(?: auf| zu)? (\S+)(?: prozent| %)?$",
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
        rule(r"^(?:lautstärke|lautstärk|ton) (hoch|rauf|runter)$", |c| {
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
        // --- Dateien (vor Web-Suche und "öffne X") ---
        rule(r"^(?:finde|findet|finden|find|wo ist|wo sind|wo liegt|wo liegen)(?: mir)? (.+)$", |c| {
            intent("file.search", json!({"query": cap(c, 1)}))
        }),
        rule(
            r"^(?:such|suche|sucht|zeig|zeige)(?: mir)? (?:nach )?((?:meine[nrms]?|der datei|die datei|den ordner|die dateien|das dokument|datei|dateien|ordner|dokument) .+)$",
            |c| intent("file.search", json!({"query": cap(c, 1)})),
        ),
        rule(
            r"^(?:öffne|öffnen|mach|mache) ((?:meine[nrms]?|die datei|das dokument|den ordner) .+?)(?: auf)?$",
            |c| intent("file.search", json!({"query": cap(c, 1), "open": true})),
        ),
        rule(r"^(?:lösch|lösche|löschen|entfern|entferne) (.+)$", |c| {
            intent("file.delete", json!({"path": cap(c, 1)}))
        }),
        rule(
            r"^(?:verschieb|verschiebe|schieb|schiebe|pack|packe) (.+?) (?:nach|in|zu|auf) (?:den |die |das |meine |meinen |ordner )*(.+?)(?: rein)?$",
            |c| intent("file.move", json!({"path": cap(c, 1), "to": cap(c, 2)})),
        ),
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
            r"^(?:such|suche|sucht|google|googel|googele|recherchier|recherchiere)(?: (?:mir|im internet|im web|online|bei google|nach))* (.+?)(?: (?:im internet|im web|online|bei google|raus))?$",
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

/// Regeln auf dem Originaltext – für Inhalte, deren Schreibweise zählt (Notiztitel/-text, neue Namen).
static ORIGINAL_RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    const NOTE_VERB: &str = r"(?:erstell|erstelle|leg|lege|mach|mache|schreib|schreibe|notier|notiere)";
    vec![
        // --- Gedächtnis, Regeln, eigene Befehle ---
        // "Wenn ich Deadlock starte, starte auch Discord" / "Merk dir: wenn ich ein Spiel starte, …"
        rule(
            r"(?i)^(?:(?:bitte\s+)?merk(?:e)?\s+dir\s*[,:]?\s*(?:dass\s+)?)?(?:immer\s+)?wenn\s+ich\s+(.+?)\s+(?:starte|öffne|spiele|zocke|anmache|anwerfe)\s*,?\s*(?:dann\s+)?(.+)$",
            |c| intent("memory.rule", json!({"when": cap(c, 1).trim(), "then": cap(c, 2).trim()})),
        ),
        rule(r"(?i)^(?:bitte\s+)?merk(?:e)?\s+dir(?:\s+bitte)?\s*[,:]?\s*(?:dass\s+)?(.+)$", |c| {
            intent("memory.remember", json!({"text": cap(c, 1).trim()}))
        }),
        rule(r"(?i)^vergiss\s+(?:bitte\s+)?alles$", |_| intent("memory.forget", json!({"all": true}))),
        rule(
            r"(?i)^vergiss(?:\s+bitte)?(?:\s*,?\s*dass|\s+das\s+mit|\s+die\s+regel|\s+den\s+merksatz)?\s+(.+)$",
            |c| intent("memory.forget", json!({"text": cap(c, 1).trim()})),
        ),
        // "Neuer Befehl Feierabend: schließe Discord, Lautstärke 30 und sperr den PC"
        rule(
            r#"(?i)^(?:(?:erstell|erstelle|leg|lege|mach|mache)\s+(?:mir\s+)?(?:einen\s+)?(?:neuen\s+)?|neuer\s+|neues\s+)(?:befehl|kommando|makro)\s+(?:namens\s+|mit\s+dem\s+namen\s+)?["„]?([^:,"“]+?)["“]?(?:\s+an)?\s*[:,]\s*(.+)$"#,
            |c| intent("macro.create", json!({"name": cap(c, 1).trim(), "steps": cap(c, 2).trim()})),
        ),
        rule(
            r#"(?i)^(?:lösch|lösche|entfern|entferne)\s+(?:den\s+|meinen\s+)?(?:eigenen\s+)?(?:befehl|makro)\s+["„]?(.+?)["“]?$"#,
            |c| intent("macro.delete", json!({"name": cap(c, 1).trim()})),
        ),
        // Erfolgreiches Skript als eigenen Befehl speichern: "Speicher das als Befehl Aufräumen"
        rule(
            r#"(?i)^speicher(?:e)?\s+(?:das|dieses\s+skript|das\s+skript)\s+(?:bitte\s+)?als\s+(?:(?:neuen\s+)?befehl\s+)?["„]?([^"“]+?)["“]?\s*(?:ab)?$"#,
            |c| intent("shell.save", json!({"name": cap(c, 1).trim()})),
        ),
        // "Erstelle eine neue Notiz und nenne sie Passwörter", "Neue Notiz Einkauf: Milch"
        rule(
            &format!(
                r"(?i)^(?:{NOTE_VERB}\s+(?:mir\s+)?(?:eine\s+)?|eine\s+)?neue\s+(geheime\s+)?notiz(?:\s+an)?(?:\s*,?\s*(?:und\s+)?(?:nenn|nenne)\s+sie|\s+mit\s+dem\s+(?:titel|namen)|\s+namens|\s+mit\s+titel|\s+zum\s+thema|\s+über)?\s*[,:]?\s*(.*)$"
            ),
            |c| note_create(c.get(1).is_some(), cap(c, 2)),
        ),
        rule(
            &format!(
                r"(?i)^{NOTE_VERB}\s+(?:mir\s+)?(?:eine\s+)?(geheime\s+)?notiz(?:\s+an)?(?:\s*,?\s*(?:und\s+)?(?:nenn|nenne)\s+sie|\s+mit\s+dem\s+(?:titel|namen)|\s+namens|\s+mit\s+titel|\s+zum\s+thema|\s+über)?\s*[,:]?\s*(.*)$"
            ),
            |c| note_create(c.get(1).is_some(), cap(c, 2)),
        ),
        // "Notiz: Milch kaufen"
        rule(r"(?i)^notiz\s*[:,]\s*(.+)$", |c| intent("note.append", json!({"text": cap(c, 1)}))),
        // "Füge zur Notiz Einkauf hinzu: Brot", "Schreib in die Notiz Einkauf: Brot"
        rule(
            r"(?i)^(?:füg|füge|schreib|schreibe|trag|trage|setz|setze)\s+(?:zur|zu\s+der|in\s+die|auf\s+die)\s+notiz\s+(.+?)(?:\s+hinzu)?\s*[:,]\s*(.+?)(?:\s+hinzu)?$",
            |c| intent("note.append", json!({"title": cap(c, 1), "text": cap(c, 2)})),
        ),
        // "Füge Brot zur Notiz Einkauf hinzu"
        rule(
            r"(?i)^(?:füg|füge|schreib|schreibe|setz|setze)\s+(.+?)\s+(?:zur|zu\s+der|auf\s+die)\s+notiz\s+(.+?)(?:\s+hinzu)?$",
            |c| intent("note.append", json!({"title": cap(c, 2), "text": cap(c, 1)})),
        ),
        // "Lies die Notiz Einkauf vor", "Lies meine letzte Notiz vor"
        rule(
            r"(?i)^(?:lies|lese|les)\s+(?:mir\s+)?(?:die\s+|meine\s+)?(?:letzte\s+|neueste\s+)?notiz\s*(.*?)(?:\s+vor)?$",
            |c| {
                let title = cap(c, 1).trim();
                let title = if title.eq_ignore_ascii_case("vor") { "" } else { title };
                intent("note.read", if title.is_empty() { json!({}) } else { json!({"title": title}) })
            },
        ),
        // "Such in meinen Notizen nach Steam", "Welche Notizen habe ich?"
        rule(r"(?i)^(?:such|suche|finde)\s+(?:in\s+)?(?:meinen\s+|den\s+)?notizen\s+(?:nach\s+)?(.+)$", |c| {
            intent("note.search", json!({"query": cap(c, 1)}))
        }),
        rule(r"(?i)^(?:welche\s+notizen.*|(?:zeig|zeige)(?:\s+mir)?\s+(?:meine|alle)\s+notizen)$", |_| {
            intent("note.search", json!({}))
        }),
        // "Benenne die Rechnung Telekom in Telekom Oktober um"
        rule(
            r"(?i)^(?:benenn|benenne)\s+(?:die\s+datei\s+|den\s+ordner\s+|das\s+dokument\s+)?(.+?)\s+(?:in|zu|nach)\s+(.+?)\s+um$",
            |c| intent("file.rename", json!({"path": cap(c, 1), "name": cap(c, 2)})),
        ),
        // "Erstelle einen Ordner Steuer 2025 auf dem Desktop"
        rule(
            r"(?i)^(?:erstell|erstelle|leg|lege|mach|mache)\s+(?:mir\s+)?(?:einen\s+)?(?:neuen\s+)?ordner(?:\s+an)?(?:\s+(?:namens|mit\s+dem\s+namen|der))?\s+(.+?)(?:\s+(?:in|auf|im|unter)\s+(?:den\s+|dem\s+|meinen\s+|meine\s+|die\s+)?(dokumente|dokumenten|downloads|desktop|schreibtisch|bilder|bildern|musik|videos|notizen))?(?:\s+an)?$",
            |c| {
                // "Dokumenten"/"Bildern" (Dativ) → "dokumente"/"bilder"
                let parent = match cap(c, 2).to_lowercase().as_str() {
                    "dokumenten" => "dokumente".to_string(),
                    "bildern" => "bilder".to_string(),
                    other => other.to_string(),
                };
                let mut params = json!({"name": cap(c, 1)});
                if !parent.is_empty() {
                    params["parent"] = json!(parent);
                }
                intent("folder.create", params)
            },
        ),
    ]
});

fn note_create(secret: bool, rest: &str) -> Option<Intent> {
    // "Einkauf: Milch, Eier" → Titel + erster Text. Whisper schreibt statt ":" oft ",".
    let (title, text) = match rest.split_once(':').or_else(|| rest.split_once(',')) {
        Some((t, x)) => (t.trim(), x.trim()),
        None => (rest.trim(), ""),
    };
    let title = match title.chars().next() {
        None => crate::texts::t("quick_note_title", &[]),
        Some(c) => c.to_uppercase().chain(title.chars().skip(1)).collect(),
    };
    let mut p = json!({"title": title});
    if !text.is_empty() {
        p["text"] = json!(text);
    }
    if secret {
        p["secret"] = json!(true);
    }
    intent("note.create", p)
}

/// Anrede/Wake-Word vorne und Satzzeichen hinten entfernen – Groß-/Kleinschreibung bleibt.
fn strip_greeting(text: &str) -> String {
    let mut s = text.trim().trim_end_matches(['.', '!', '?']).trim().to_string();
    while let Some(first) = s.split_whitespace().next() {
        let word = first.trim_end_matches([',', '.', '!', ':']).to_lowercase();
        let greeting =
            matches!(word.as_str(), "hey" | "hallo" | "ok" | "okay" | "bitte") || normalize::is_wake_word(&word);
        if s.split_whitespace().count() > 1 && greeting {
            s = s[first.len()..].trim_start_matches([',', ' ']).trim_start().to_string();
        } else {
            break;
        }
    }
    s
}

/// Versteht einen Originaltext (getippt oder transkribiert). `None` = nicht verstanden.
pub fn parse_text(original: &str) -> Option<Intent> {
    let stripped = strip_greeting(original);
    ORIGINAL_RULES
        .iter()
        .find_map(|r| r.re.captures(&stripped).and_then(|c| (r.build)(&c)))
        .or_else(|| parse(&normalize::command(original)))
}

/// Gesprochen: Reste des Wake-Words ("…co, öffne X") stehen manchmal noch vorne – dann ohne erstes Wort.
pub fn parse_text_lenient(original: &str) -> Option<Intent> {
    parse_text(original).or_else(|| {
        let stripped = strip_greeting(original);
        stripped.split_once(char::is_whitespace).and_then(|(_, rest)| parse_text(rest))
    })
}

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

/// Antwort auf eine Rückfrage aus dem Erkannten lösen. Hört das Mikro Pancos eigene Frage aus den
/// Lautsprechern mit, steht sie im Text – dann zählt nur, was danach kam. `None` = nur Echo.
pub fn answer_text(text: &str, asked: Option<&str>) -> Option<String> {
    use normalize::command;
    if yes_no(&command(text)).is_some() {
        return Some(text.to_string());
    }
    let Some(asked) = asked else { return Some(text.to_string()) };
    let spoken: Vec<String> = command(asked).split(' ').map(str::to_string).collect();
    let is_echo = |part: &str| {
        let words: Vec<String> = command(part).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
        let known = words.iter().filter(|w| spoken.contains(w)).count();
        !words.is_empty() && known * 10 >= words.len() * 6
    };
    // Letzter Satz ist eine Antwort, davor nur Echo: "Soll ich … löschen? Sag ja oder nein. Ja."
    let sentences: Vec<&str> = text.split_inclusive(['.', '!', '?']).map(str::trim).filter(|s| !s.is_empty()).collect();
    if let Some((last, before)) = sentences.split_last() {
        if yes_no(&command(last)).is_some() && before.iter().all(|s| is_echo(s)) {
            return Some(last.to_string());
        }
    }
    if is_echo(text) {
        None
    } else {
        Some(text.to_string())
    }
}

/// "Ja, und frag die nächsten 10 Minuten nicht mehr" – Zustimmung + Rückfragen kurz aussetzen.
pub fn wants_quiet(normalized: &str) -> bool {
    let n = normalized;
    (n.starts_with("ja") || n.starts_with("okay") || n.starts_with("ok"))
        && (n.contains("nicht mehr fragen") || n.contains("frag nicht mehr") || n.contains("minuten nicht"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_of_own_question_is_ignored() {
        let asked = Some("Soll ich Rechnung.pdf in den Papierkorb legen? Sag ja oder nein.");
        assert_eq!(answer_text("Ja.", asked).as_deref(), Some("Ja."));
        assert_eq!(answer_text("Soll ich Rechnung PDF in den Papierkorb legen?", asked), None);
        assert_eq!(answer_text("Sag ja oder nein.", asked), None);
        assert_eq!(
            answer_text("Soll ich Rechnung PDF in den Papierkorb legen? Sag ja oder nein. Nein.", asked).as_deref(),
            Some("Nein.")
        );
        // Etwas anderes als die Frage: normal als Befehl weitergeben
        assert_eq!(answer_text("Öffne lieber Discord", asked).as_deref(), Some("Öffne lieber Discord"));
    }

    fn p(text: &str) -> Option<(&'static str, Value)> {
        parse_text(text).map(|i| (i.action, i.params))
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
        assert_eq!(p("Paanco, Lautstärk auf 30 Prozent."), Some(("volume.set", json!({"level": 30}))));
        assert_eq!(p("Lautstärke 300"), None);
        assert_eq!(p("lauter"), Some(("volume.change", json!({"delta": 10}))));
        assert_eq!(p("mach etwas leiser"), Some(("volume.change", json!({"delta": -5}))));
        assert_eq!(p("viel lauter"), Some(("volume.change", json!({"delta": 20}))));
        assert_eq!(p("Ton aus"), Some(("volume.mute", json!({"mute": true}))));
        // Lautstärke pro App
        assert_eq!(p("Panco, Spotify leiser"), Some(("volume.app", json!({"app": "spotify", "delta": -15}))));
        assert_eq!(p("mach Discord viel lauter"), Some(("volume.app", json!({"app": "discord", "delta": 25}))));
        assert_eq!(p("stell Spotify auf 30 Prozent"), Some(("volume.app", json!({"app": "spotify", "level": 30}))));
        assert_eq!(
            p("die Lautstärke von Discord auf zwanzig"),
            Some(("volume.app", json!({"app": "discord", "level": 20})))
        );
        assert_eq!(p("mach Discord stumm"), Some(("volume.app", json!({"app": "discord", "mute": true}))));
        assert_eq!(p("Discord wieder laut"), Some(("volume.app", json!({"app": "discord", "mute": false}))));
        assert_eq!(p("mach es leiser"), Some(("volume.change", json!({"delta": -10}))));
        assert_eq!(p("die Musik leiser"), Some(("volume.change", json!({"delta": -10}))));
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
        assert_eq!(
            p("Hanco, sucht Deadlock Build Guides."),
            Some(("web.search", json!({"query": "deadlock build guides"})))
        );
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
    fn files() {
        assert_eq!(
            p("Panco, finde meine letzte Rechnung"),
            Some(("file.search", json!({"query": "meine letzte rechnung"})))
        );
        // Whisper konjugiert gern falsch
        assert_eq!(
            p("Paanco findet meine letzte Rechnung."),
            Some(("file.search", json!({"query": "meine letzte rechnung"})))
        );
        assert_eq!(p("wo ist meine Bewerbung?"), Some(("file.search", json!({"query": "meine bewerbung"}))));
        assert_eq!(
            p("zeig mir meine Screenshots von gestern"),
            Some(("file.search", json!({"query": "meine screenshots von gestern"})))
        );
        assert_eq!(p("such die Datei Steuer"), Some(("file.search", json!({"query": "die datei steuer"}))));
        assert_eq!(p("such deadlock build guides"), Some(("web.search", json!({"query": "deadlock build guides"}))));
        assert_eq!(
            p("Öffne meine letzte Rechnung"),
            Some(("file.search", json!({"query": "meine letzte rechnung", "open": true})))
        );
        assert_eq!(
            p("lösche die Datei Rechnung Telekom"),
            Some(("file.delete", json!({"path": "die datei rechnung telekom"})))
        );
        assert_eq!(
            p("verschiebe meine letzte Rechnung nach Dokumente"),
            Some(("file.move", json!({"path": "meine letzte rechnung", "to": "dokumente"})))
        );
        assert_eq!(
            p("Benenne die Rechnung Telekom in Telekom Oktober um."),
            Some(("file.rename", json!({"path": "die Rechnung Telekom", "name": "Telekom Oktober"})))
        );
        assert_eq!(
            p("Erstelle einen Ordner Steuer 2025 auf dem Desktop"),
            Some(("folder.create", json!({"name": "Steuer 2025", "parent": "desktop"})))
        );
        assert_eq!(p("Leg einen neuen Ordner Projekte an"), Some(("folder.create", json!({"name": "Projekte"}))));
        assert_eq!(p("mach einen Screenshot"), Some(("screen.screenshot", json!({}))));
        assert_eq!(p("mach das rückgängig"), Some(("panco.undo", json!({}))));
        assert_eq!(p("Wie viel Uhr ist es gerade?"), Some(("info.time", json!({"what": "time"}))));
        assert_eq!(p("Wie lange habe ich heute gespielt?"), Some(("info.playtime", json!({"period": "today"}))));
        assert_eq!(p("wie lange hab ich diese Woche gezockt"), Some(("info.playtime", json!({"period": "week"}))));
        assert_eq!(
            p("Wie lange habe ich schon Deadlock gespielt?"),
            Some(("info.playtime", json!({"period": "total", "game": "deadlock"})))
        );
        assert_eq!(p("Was spiele ich gerade?"), Some(("info.playtime", json!({"period": "now"}))));
        assert_eq!(p("Panco, wie spät ist es?"), Some(("info.time", json!({"what": "time"}))));
        assert_eq!(p("Welcher Tag ist heute?"), Some(("info.time", json!({"what": "date"}))));
        assert_eq!(p("Was kannst du alles?"), Some(("panco.help", json!({}))));
        assert_eq!(p("Lese mir vor, was in dem Editor steht."), Some(("screen.read", json!({"target": "window"}))));
        assert_eq!(p("Läs mir vor, was in dem Editor steht."), Some(("screen.read", json!({"target": "window"}))));
        assert_eq!(p("Panko, macht das Röckgängig."), Some(("panco.undo", json!({}))));
        assert_eq!(p("Rückgängig!"), Some(("panco.undo", json!({}))));
    }

    #[test]
    fn notes() {
        assert_eq!(
            p("Panco, erstelle eine neue Notiz und nenne sie Passwörter."),
            Some(("note.create", json!({"title": "Passwörter"})))
        );
        assert_eq!(
            p("Neue Notiz Einkauf: Milch, Eier"),
            Some(("note.create", json!({"title": "Einkauf", "text": "Milch, Eier"})))
        );
        // So transkribiert Whisper den Doppelpunkt
        assert_eq!(
            p("Panco, neue Notiz einkauf, Milch und Brot."),
            Some(("note.create", json!({"title": "Einkauf", "text": "Milch und Brot"})))
        );
        assert_eq!(
            p("Mach mir eine geheime Notiz Tresor"),
            Some(("note.create", json!({"title": "Tresor", "secret": true})))
        );
        assert_eq!(p("Notiz: Milch kaufen."), Some(("note.append", json!({"text": "Milch kaufen"}))));
        assert_eq!(
            p("Füge zur Notiz Einkauf hinzu: Brot"),
            Some(("note.append", json!({"title": "Einkauf", "text": "Brot"})))
        );
        assert_eq!(
            p("Füge Brot zur Notiz Einkauf hinzu"),
            Some(("note.append", json!({"title": "Einkauf", "text": "Brot"})))
        );
        assert_eq!(p("Lies die Notiz Einkauf vor"), Some(("note.read", json!({"title": "Einkauf"}))));
        assert_eq!(p("lies meine letzte Notiz vor"), Some(("note.read", json!({}))));
        assert_eq!(p("Welche Notizen habe ich?"), Some(("note.search", json!({}))));
        assert_eq!(p("Such in meinen Notizen nach Steam"), Some(("note.search", json!({"query": "Steam"}))));
    }

    #[test]
    fn memory_and_macros() {
        assert_eq!(
            p("Panco, merk dir, dass mein Lieblingsspiel Deadlock ist"),
            Some(("memory.remember", json!({"text": "mein Lieblingsspiel Deadlock ist"})))
        );
        assert_eq!(
            p("Wenn ich Deadlock starte, starte auch Discord."),
            Some(("memory.rule", json!({"when": "Deadlock", "then": "starte auch Discord"})))
        );
        assert_eq!(
            p("Merk dir: wenn ich ein Spiel starte, stell Spotify auf 20 Prozent"),
            Some(("memory.rule", json!({"when": "ein Spiel", "then": "stell Spotify auf 20 Prozent"})))
        );
        assert_eq!(p("Was weißt du über mich?"), Some(("memory.list", json!({}))));
        assert_eq!(p("Vergiss alles"), Some(("memory.forget", json!({"all": true}))));
        assert_eq!(p("Vergiss das mit Discord"), Some(("memory.forget", json!({"text": "Discord"}))));
        assert_eq!(
            p("Neuer Befehl Feierabend: schließe Discord, Lautstärke 30 und sperr den PC"),
            Some((
                "macro.create",
                json!({"name": "Feierabend", "steps": "schließe Discord, Lautstärke 30 und sperr den PC"})
            ))
        );
        assert_eq!(
            p("Erstelle einen Befehl „Zocken“: starte Discord und Spotify leiser"),
            Some(("macro.create", json!({"name": "Zocken", "steps": "starte Discord und Spotify leiser"})))
        );
        assert_eq!(p("Lösche den Befehl Feierabend"), Some(("macro.delete", json!({"name": "Feierabend"}))));
        assert_eq!(p("nochmal"), Some(("panco.repeat", json!({}))));
        assert_eq!(p("Speicher das als Befehl Aufräumen"), Some(("shell.save", json!({"name": "Aufräumen"}))));
        assert_eq!(p("Lies mir den Bildschirm vor"), Some(("screen.read", json!({"target": "screen"}))));
        assert_eq!(p("Was steht im Fenster?"), Some(("screen.read", json!({"target": "window"}))));
        assert_eq!(p("Was steht in der Zwischenablage?"), Some(("clipboard.read", json!({}))));
        assert_eq!(p("Welches Fenster ist gerade vorne?"), Some(("window.active", json!({}))));
        assert_eq!(
            p("Spiel Lo-Fi Beats auf Spotify"),
            Some(("spotify.play", json!({"query": "lo fi beats", "kind": "track"})))
        );
        assert_eq!(
            p("spiel die Playlist Gaming Mix auf Spotify"),
            Some(("spotify.play", json!({"query": "gaming mix", "kind": "playlist"})))
        );
        assert_eq!(
            p("spiel Musik von Daft Punk auf Spotify"),
            Some(("spotify.play", json!({"query": "daft punk", "kind": "artist"})))
        );
        assert_eq!(p("Was läuft gerade?"), Some(("spotify.now", json!({}))));
        assert_eq!(p("was für ein Lied läuft gerade auf Spotify"), Some(("spotify.now", json!({}))));
        assert_eq!(p("spiel deadlock"), Some(("app.launch", json!({"target": "deadlock"}))));
        assert_eq!(p("Frag mich 10 Minuten nicht mehr"), Some(("panco.quiet", json!({"minutes": 10}))));
        assert_eq!(p("frag wieder nach"), Some(("panco.quiet", json!({"minutes": 0}))));
        assert!(wants_quiet(&normalize::command("Ja, und frag die nächsten 10 Minuten nicht mehr")));
        assert!(!wants_quiet("ja"));
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

//! Text-Normalisierung für Parser und unscharfen Vergleich.

/// Für den Parser: klein, ohne Satzzeichen, ohne Anrede "Panco", Füllwörter raus.
pub fn command(text: &str) -> String {
    let lower = text.to_lowercase();
    let cleaned: String = lower
        .chars()
        .map(|c| if matches!(c, ',' | '!' | '?' | '"' | '„' | '“' | ';' | '-' | '–') { ' ' } else { c })
        .collect();
    // Punkte am Wortende entfernen ("Lautstärke. Auf 30.") – Punkte im Wort (twitch.tv) bleiben.
    let mut words: Vec<&str> =
        cleaned.split_whitespace().map(|w| w.trim_end_matches(['.', ':'])).filter(|w| !w.is_empty()).collect();
    // Anrede am Anfang: "hey panco", "Hanco", "Pango" … (ein einzelnes "okay" bleibt als Antwort stehen)
    while words.len() > 1 && (matches!(words[0], "hey" | "hallo" | "ok" | "okay") || is_wake_word(words[0])) {
        words.remove(0);
    }
    words
        .into_iter()
        .filter(|w| !matches!(*w, "bitte" | "mal" | "doch" | "kurz" | "eben" | "jetzt"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Klingt wie "Panco"? Whisper schreibt das Wake-Word oft als "Hanco", "Pango", "Panko", "Paanco" …
pub fn is_wake_word(word: &str) -> bool {
    let w = name(word);
    let len = w.chars().count();
    // Lautmuster P/B/H/F/V + a + n + g/k/c + o/u ("paangu", "hanco") oder sehr ähnlich geschrieben.
    static PATTERN: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^[pbhfvw]a+n+[gkcq]+[oua]+$").unwrap());
    (4..=7).contains(&len)
        && (PATTERN.is_match(&w) || strsim::jaro_winkler(&w, "panco").max(strsim::jaro_winkler(&w, "panko")) >= 0.8)
}

/// Für Namensvergleiche: ASCII-nah, ohne Sonderzeichen, Ziffern bleiben.
/// "Counter-Strike® 2" → "counter strike 2", "Ärger" → "aerger"
pub fn name(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.to_lowercase().chars() {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            'é' | 'è' | 'ê' => out.push('e'),
            'á' | 'à' | 'â' => out.push('a'),
            c if c.is_alphanumeric() => out.push(c),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Deutsche Zahlwörter 0–100 (Whisper schreibt meist Ziffern, aber nicht immer).
pub fn number(word: &str) -> Option<u32> {
    if let Ok(n) = word.trim_end_matches('%').parse::<u32>() {
        return Some(n);
    }
    const ONES: [&str; 20] = [
        "null",
        "eins",
        "zwei",
        "drei",
        "vier",
        "fünf",
        "sechs",
        "sieben",
        "acht",
        "neun",
        "zehn",
        "elf",
        "zwölf",
        "dreizehn",
        "vierzehn",
        "fünfzehn",
        "sechzehn",
        "siebzehn",
        "achtzehn",
        "neunzehn",
    ];
    const TENS: [(&str, u32); 8] = [
        ("zwanzig", 20),
        ("dreißig", 30),
        ("vierzig", 40),
        ("fünfzig", 50),
        ("sechzig", 60),
        ("siebzig", 70),
        ("achtzig", 80),
        ("neunzig", 90),
    ];
    let w = word.trim();
    if w == "ein" || w == "eine" {
        return Some(1);
    }
    if w == "hundert" || w == "einhundert" {
        return Some(100);
    }
    if let Some(i) = ONES.iter().position(|o| *o == w) {
        return Some(i as u32);
    }
    // "fünfundzwanzig" = fünf + und + zwanzig
    for (t, v) in TENS {
        if w == t {
            return Some(v);
        }
        if let Some(prefix) = w.strip_suffix(t).and_then(|p| p.strip_suffix("und")) {
            let ones = if prefix == "ein" {
                Some(1)
            } else {
                ONES[1..10].iter().position(|o| *o == prefix).map(|i| i as u32 + 1)
            };
            return ones.map(|o| v + o);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_wake_word_and_fillers() {
        assert_eq!(command("Panco, öffne bitte mal Deadlock."), "öffne deadlock");
        assert_eq!(command("Hey Panko! Lautstärke 30"), "lautstärke 30");
        assert_eq!(command("öffne twitch.tv"), "öffne twitch.tv");
        assert_eq!(command("Hanco, Öffne Deadlock."), "öffne deadlock");
        assert_eq!(command("Paangu, Fahr den PC herunter."), "fahr den pc herunter");
        assert_eq!(command("Pango. Lautstärke 30"), "lautstärke 30");
        assert_eq!(command("Panko"), "panko");
        assert_eq!(command("Paanco, Lautstärk auf 30 Prozent."), "lautstärk auf 30 prozent");
    }

    #[test]
    fn names() {
        assert_eq!(name("Counter-Strike® 2"), "counter strike 2");
        assert_eq!(name("Tom Clancy's Rainbow Six® Siege"), "tom clancy s rainbow six siege");
        assert_eq!(name("Spaß"), "spass");
    }

    #[test]
    fn numbers() {
        assert_eq!(number("30"), Some(30));
        assert_eq!(number("30%"), Some(30));
        assert_eq!(number("dreißig"), Some(30));
        assert_eq!(number("fünfundzwanzig"), Some(25));
        assert_eq!(number("einundzwanzig"), Some(21));
        assert_eq!(number("hundert"), Some(100));
        assert_eq!(number("zwölf"), Some(12));
        assert_eq!(number("deadlock"), None);
    }
}

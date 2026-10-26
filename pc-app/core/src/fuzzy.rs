//! Unscharfes Namens-Matching ("dead lock", "deadlok" → "Deadlock").

use crate::normalize;
use strsim::jaro_winkler;

/// Ab diesem Wert wird direkt ausgeführt.
pub const SURE: f64 = 0.88;
/// Ab diesem Wert fragt Panco "Meinst du …?".
pub const MAYBE: f64 = 0.72;

const STOPWORDS: [&str; 6] = ["the", "der", "die", "das", "of", "a"];

fn tokens(s: &str) -> Vec<&str> {
    s.split(' ').filter(|t| !t.is_empty() && !STOPWORDS.contains(t)).collect()
}

/// Ähnlichkeit 0..=1 zwischen gesprochenem Namen und Kandidat.
pub fn score(query: &str, candidate: &str) -> f64 {
    let q = normalize::name(query);
    let c = normalize::name(candidate);
    if q.is_empty() || c.is_empty() {
        return 0.0;
    }
    let qs: String = q.split(' ').collect();
    let cs: String = c.split(' ').collect();
    if q == c || qs == cs {
        return 1.0;
    }
    let qt = tokens(&q);
    let ct = tokens(&c);
    let extra = ct.len().saturating_sub(qt.len()) as f64;
    let mut best = jaro_winkler(&qs, &cs) * 0.97;

    // Alle gesprochenen Wörter kommen im Namen vor ("witcher" ⊂ "The Witcher 3 Wild Hunt").
    let meaningful = qt.iter().any(|t| t.len() >= 3 || t.chars().all(|c| c.is_ascii_digit()));
    if meaningful && !qt.is_empty() {
        if qt.iter().all(|t| ct.contains(t)) {
            let prefix_bonus = if c.starts_with(&q) { 0.02 } else { 0.0 };
            best = best.max(0.92 + prefix_bonus - 0.01 * extra);
        } else if qt.iter().all(|t| ct.iter().any(|w| jaro_winkler(t, w) >= 0.9)) {
            // Wörter fast gleich (Tippfehler/Transkriptionsfehler pro Wort).
            best = best.max(0.86 - 0.01 * extra);
        }
    }
    best.clamp(0.0, 1.0)
}

/// Ergebnis einer Namensauflösung.
#[derive(Debug, PartialEq)]
pub enum Resolved<T> {
    /// Eindeutig – ausführen.
    Sure(T),
    /// Wahrscheinlich – nachfragen.
    Unsure(T),
    None,
}

/// Bester Treffer über alle Kandidaten; jeder Kandidat kann mehrere Namen haben.
pub fn resolve<'a, T>(query: &str, items: &'a [T], names: impl Fn(&'a T) -> Vec<&'a str>) -> Resolved<&'a T> {
    let mut scored: Vec<(&T, f64)> = items
        .iter()
        .map(|it| {
            let s = names(it).into_iter().map(|n| score(query, n)).fold(0.0, f64::max);
            (it, s)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    match scored.as_slice() {
        [] => Resolved::None,
        [(top, s), rest @ ..] => {
            let close_second = rest.first().is_some_and(|(_, s2)| s - s2 < 0.015);
            if *s >= SURE && !close_second {
                Resolved::Sure(*top)
            } else if *s >= MAYBE {
                Resolved::Unsure(*top)
            } else {
                Resolved::None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAMES: [&str; 8] = [
        "Deadlock",
        "Counter-Strike 2",
        "The Witcher 3: Wild Hunt",
        "Baldur's Gate 3",
        "Elden Ring",
        "Dead by Daylight",
        "Rocket League",
        "Apex Legends",
    ];

    fn r(q: &str) -> Resolved<&'static str> {
        resolve(q, &GAMES, |g| vec![*g]).map_ref()
    }

    impl Resolved<&&'static str> {
        fn map_ref(self) -> Resolved<&'static str> {
            match self {
                Resolved::Sure(x) => Resolved::Sure(*x),
                Resolved::Unsure(x) => Resolved::Unsure(*x),
                Resolved::None => Resolved::None,
            }
        }
    }

    #[test]
    fn exact_and_spacing() {
        assert_eq!(r("deadlock"), Resolved::Sure("Deadlock"));
        assert_eq!(r("dead lock"), Resolved::Sure("Deadlock"));
        assert_eq!(r("Elden Ring"), Resolved::Sure("Elden Ring"));
    }

    #[test]
    fn typos_and_partial_names() {
        assert_eq!(r("deadlok"), Resolved::Sure("Deadlock"));
        assert_eq!(r("witcher"), Resolved::Sure("The Witcher 3: Wild Hunt"));
        assert_eq!(r("baldurs gate"), Resolved::Sure("Baldur's Gate 3"));
        assert_eq!(r("rocket league"), Resolved::Sure("Rocket League"));
        assert_eq!(r("apex"), Resolved::Sure("Apex Legends"));
    }

    #[test]
    fn unknown_is_none() {
        assert_eq!(r("minecraft"), Resolved::None);
        assert_eq!(r("zz"), Resolved::None);
    }
}

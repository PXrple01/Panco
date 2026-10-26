//! Panco-Sätze aus `shared/texts/de.json` (zur Compile-Zeit eingebettet).

use std::collections::HashMap;
use std::sync::LazyLock;

static TEXTS: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    let raw: HashMap<String, serde_json::Value> =
        serde_json::from_str(include_str!("../../../shared/texts/de.json")).expect("de.json ist gültiges JSON");
    raw.into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string()))).collect()
});

/// Liefert einen Satz und ersetzt `{name}`-Platzhalter.
/// Fehlende Schlüssel geben den Schlüssel selbst zurück (fällt in Tests sofort auf).
pub fn t(key: &str, vars: &[(&str, &str)]) -> String {
    let mut s = TEXTS.get(key).cloned().unwrap_or_else(|| key.to_string());
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_placeholders() {
        assert_eq!(t("launch_game", &[("name", "Deadlock")]), "Deadlock startet.");
        assert_eq!(t("gibt_es_nicht", &[]), "gibt_es_nicht");
    }
}

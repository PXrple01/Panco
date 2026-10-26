//! Dateisuche per Sprache: "finde meine letzte Rechnung", "Screenshots von gestern",
//! "meine Bewerbung als PDF". Durchsucht die freigegebenen Ordner (begrenzt in Tiefe, Anzahl und
//! Zeit) und bewertet Treffer nach Name, Ordner, Dateityp und Alter.

use crate::normalize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pdf,
    Image,
    Video,
    Audio,
    Sheet,
    Slides,
    Document,
    Folder,
}

impl Kind {
    fn matches(self, path: &Path, is_dir: bool) -> bool {
        if self == Kind::Folder {
            return is_dir;
        }
        if is_dir {
            return false;
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let list: &[&str] = match self {
            Kind::Pdf => &["pdf"],
            Kind::Image => &["png", "jpg", "jpeg", "gif", "webp", "bmp", "heic", "tif", "tiff"],
            Kind::Video => &["mp4", "mkv", "mov", "avi", "webm", "wmv"],
            Kind::Audio => &["mp3", "wav", "flac", "m4a", "ogg", "aac", "wma"],
            Kind::Sheet => &["xlsx", "xls", "csv", "ods"],
            Kind::Slides => &["pptx", "ppt", "odp", "key"],
            Kind::Document => &["pdf", "docx", "doc", "odt", "rtf", "txt", "md"],
            Kind::Folder => &[],
        };
        list.contains(&ext.as_str())
    }
}

/// Ein gesuchter Begriff mit Synonymen: starke zählen voll, schwache weniger
/// ("Rechnung": invoice stark, Beleg/Quittung schwach).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Group {
    pub strong: Vec<String>,
    pub weak: Vec<String>,
}

/// Ausgewertete Anfrage.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Query {
    /// Jede Gruppe muss passen; innerhalb einer Gruppe reicht ein Synonym.
    pub groups: Vec<Group>,
    pub kind: Option<Kind>,
    /// "letzte", "neueste": nach Datum sortieren.
    pub latest: bool,
    /// Zeitraum in Unix-Sekunden.
    pub since: Option<i64>,
    pub until: Option<i64>,
    /// Was der Nutzer gesucht hat (für Antworten).
    pub label: String,
}

const STOP: [&str; 34] = [
    "meine", "meinen", "meiner", "meinem", "mein", "die", "der", "das", "den", "dem", "des", "ein", "eine", "einen",
    "von", "vom", "zu", "zur", "zum", "mir", "alle", "datei", "dateien", "als", "im", "in", "auf", "mit", "nach",
    "fuer", "fur", "bitte", "wo", "ist",
];

/// Synonyme häufiger Dokumentarten (deutsch/englisch): (stark, schwach).
const SYNONYMS: [(&[&str], &[&str]); 14] = [
    (&["rechnung", "invoice", "faktura"], &["beleg", "quittung", "receipt", "bill"]),
    (&["vertrag", "vertraege", "contract"], &[]),
    (&["lebenslauf", "cv", "resume"], &["vita"]),
    (&["kontoauszug", "kontoauszuege"], &["statement", "auszug"]),
    (&["steuer", "steuern", "steuererklaerung", "tax", "elster"], &[]),
    (&["bewerbung", "application"], &["anschreiben"]),
    (&["versicherung", "insurance"], &["police"]),
    (&["zeugnis", "zertifikat", "certificate"], &[]),
    (&["ticket", "fahrkarte", "boardingpass", "eintrittskarte"], &[]),
    (&["gehalt", "lohn", "gehaltsabrechnung", "lohnabrechnung", "payslip"], &[]),
    (&["screenshot", "bildschirmfoto"], &["screen"]),
    (&["foto", "photo", "bild"], &["img", "image"]),
    (&["notiz", "note", "notes"], &[]),
    (&["praesentation", "presentation", "folien"], &[]),
];

fn group_from(g: &(&[&str], &[&str])) -> Group {
    Group { strong: g.0.iter().map(|s| s.to_string()).collect(), weak: g.1.iter().map(|s| s.to_string()).collect() }
}

/// Mehrzahl/Fälle grob abschneiden, damit "rechnungen" auch "rechnung" findet.
fn stem(w: &str) -> String {
    for suf in ["ungen", "en", "n", "e", "s"] {
        if w.len() > 5 && w.ends_with(suf) && !w.ends_with("ung") {
            return w[..w.len() - suf.len()].to_string();
        }
    }
    w.to_string()
}

/// Wertet die Anfrage aus. `now`/`offset` für "heute", "gestern" usw.
pub fn parse_query(text: &str, now: i64, offset: i32) -> Query {
    let mut q = Query::default();
    let mut t = format!(" {} ", normalize::name(text));
    let day = |back: i64| crate::time::local_midnight(now, offset, back);
    // Zeiträume zuerst (enthalten Wörter wie "letzte")
    let periods: [(&str, i64, Option<i64>); 6] = [
        (" von gestern ", day(1), Some(day(0))),
        (" gestern ", day(1), Some(day(0))),
        (" von heute ", day(0), None),
        (" heute ", day(0), None),
        (" letzte woche ", day(14), Some(day(7))),
        (" diese woche ", day(7), None),
    ];
    for (phrase, since, until) in periods {
        if t.contains(phrase) {
            q.since = Some(since);
            q.until = until;
            t = t.replace(phrase, " ");
            break;
        }
    }
    for (phrase, since) in [(" letzten monat ", day(60)), (" diesen monat ", day(31))] {
        if t.contains(phrase) {
            q.since = Some(since);
            t = t.replace(phrase, " ");
        }
    }
    let mut label = Vec::new();
    for w in t.split_whitespace() {
        let kind = match w {
            "pdf" | "pdfs" => Some(Kind::Pdf),
            "bild" | "bilder" | "foto" | "fotos" => Some(Kind::Image),
            "screenshot" | "screenshots" | "bildschirmfoto" | "bildschirmfotos" => {
                q.kind = Some(Kind::Image);
                q.groups.push(group_from(&SYNONYMS[10]));
                label.push(w.to_string());
                continue;
            }
            "video" | "videos" => Some(Kind::Video),
            "musik" | "song" | "songs" | "lied" | "lieder" => Some(Kind::Audio),
            "tabelle" | "tabellen" | "excel" => Some(Kind::Sheet),
            "praesentation" | "praesentationen" | "powerpoint" => Some(Kind::Slides),
            "dokument" | "dokumente" | "word" | "text" => Some(Kind::Document),
            "ordner" => Some(Kind::Folder),
            _ => None,
        };
        if let Some(k) = kind {
            q.kind = Some(k);
            label.push(w.to_string());
            continue;
        }
        if matches!(
            w,
            "letzte"
                | "letzten"
                | "letztes"
                | "letzter"
                | "neueste"
                | "neuesten"
                | "neuestes"
                | "aktuelle"
                | "aktuellste"
                | "juengste"
        ) {
            q.latest = true;
            continue;
        }
        if STOP.contains(&w) || w.len() < 2 {
            continue;
        }
        label.push(w.to_string());
        let s = stem(w);
        let group = SYNONYMS
            .iter()
            .find(|g| g.0.iter().chain(g.1.iter()).any(|syn| *syn == s || *syn == w || stem(syn) == s))
            .map(group_from)
            .unwrap_or_else(|| Group { strong: vec![s], weak: vec![] });
        q.groups.push(group);
    }
    q.label = label.join(" ");
    q
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub modified: i64,
    pub size: u64,
    /// Wie gut Name/Ordner/Typ passen (ohne Alter).
    pub relevance: f64,
    /// Relevanz + kleiner Bonus für Neues.
    pub score: f64,
}

/// Grenzen, damit eine Suche nie den PC ausbremst.
pub struct Limits {
    pub max_depth: usize,
    pub max_entries: usize,
    pub max_time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_depth: 8, max_entries: 80_000, max_time: Duration::from_millis(2500) }
    }
}

fn skip_dir(name: &str) -> bool {
    name.starts_with('.')
        || name.starts_with('$')
        || matches!(name, "node_modules" | "target" | "__pycache__" | "AppData" | "venv" | ".venv" | "Panco-Backups")
}

/// Wie gut passt ein Name/Ordner zu einer Synonymgruppe? 0 = gar nicht.
fn group_score(group: &Group, name_tokens: &[String], folder_tokens: &[String]) -> f64 {
    let hit = |words: &[String], tokens: &[String]| {
        tokens.iter().any(|tok| {
            words.iter().any(|g| tok.contains(g.as_str()) || (g.len() >= 5 && strsim::jaro_winkler(tok, g) >= 0.92))
        })
    };
    if hit(&group.strong, name_tokens) {
        1.0
    } else if hit(&group.strong, folder_tokens) || hit(&group.weak, name_tokens) {
        0.6
    } else if hit(&group.weak, folder_tokens) {
        0.35
    } else {
        0.0
    }
}

fn tokens(s: &str) -> Vec<String> {
    normalize::name(s).split(' ').filter(|t| !t.is_empty()).map(str::to_string).collect()
}

/// Durchsucht `roots`; `exclude` (Papierkorb, Backups) wird übersprungen.
pub fn search(roots: &[PathBuf], exclude: &[PathBuf], q: &Query, now: i64, limits: &Limits) -> Vec<Hit> {
    let started = Instant::now();
    let mut hits = Vec::new();
    let mut seen = 0usize;
    let mut stack: Vec<(PathBuf, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    let mut visited_roots: Vec<PathBuf> = Vec::new();

    while let Some((dir, depth)) = stack.pop() {
        if started.elapsed() > limits.max_time || seen > limits.max_entries {
            break;
        }
        if exclude.iter().any(|e| super::within(&dir, e)) || visited_roots.iter().any(|v| v == &dir) {
            continue;
        }
        visited_roots.push(dir.clone());
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let folder_tokens: Vec<String> =
            dir.components().rev().take(3).flat_map(|c| tokens(&c.as_os_str().to_string_lossy())).collect();
        for entry in rd.flatten() {
            seen += 1;
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(meta) = entry.metadata() else { continue };
            let path = entry.path();
            let is_dir = meta.is_dir();
            if is_dir && depth < limits.max_depth && !skip_dir(&name) {
                stack.push((path.clone(), depth + 1));
            }
            if name.starts_with('.') || name.starts_with('~') || name.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }
            if let Some(k) = q.kind {
                if !k.matches(&path, is_dir) {
                    continue;
                }
            }
            let modified = meta.modified().map(crate::time::unix).unwrap_or(0);
            if q.since.is_some_and(|s| modified < s) || q.until.is_some_and(|u| modified >= u) {
                continue;
            }
            let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(name.clone());
            let name_tokens = tokens(&stem);
            let mut score = 0.0;
            let mut all = true;
            for g in &q.groups {
                let s = group_score(g, &name_tokens, &folder_tokens);
                if s == 0.0 {
                    all = false;
                    break;
                }
                score += s;
            }
            // Nur Zeitraum/Typ angegeben ("Screenshots von heute") → alles im Rahmen zählt.
            if !all || (q.groups.is_empty() && q.kind.is_none() && q.since.is_none()) {
                continue;
            }
            if is_dir && q.kind != Some(Kind::Folder) {
                score -= 0.2;
            }
            if !is_dir && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) && !q.groups.is_empty() {
                score += 0.1; // Rechnungen, Verträge … sind meist PDFs
            }
            let relevance = score;
            let age_days = ((now - modified).max(0) as f64) / 86_400.0;
            score += 0.3 * (-age_days / 60.0).exp();
            hits.push(Hit { path, name, is_dir, modified, size: meta.len(), relevance, score });
        }
    }
    // Ordner nur, wenn danach gefragt wurde oder keine Datei passt.
    if q.kind != Some(Kind::Folder) && hits.iter().any(|h| !h.is_dir) {
        hits.retain(|h| !h.is_dir);
    }
    if q.latest || (q.groups.is_empty() && q.since.is_some()) {
        // "Die letzte Rechnung": nur unter den passendsten Treffern nach Datum sortieren –
        // sonst gewinnt ein neuer "Beleg" gegen die echte Rechnung von gestern.
        let best = hits.iter().map(|h| h.relevance).fold(f64::MIN, f64::max);
        hits.retain(|h| h.relevance >= best - 0.3);
        hits.sort_by(|a, b| b.modified.cmp(&a.modified).then(b.score.total_cmp(&a.score)));
    } else {
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.modified.cmp(&a.modified)));
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File, FileTimes};
    use std::time::SystemTime;

    fn touch(root: &Path, rel: &str, days_ago: u64) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        let f = File::create(&p).unwrap();
        let t = SystemTime::now() - Duration::from_secs(days_ago * 86_400 + 60);
        f.set_times(FileTimes::new().set_modified(t)).unwrap();
    }

    fn setup(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("panco-search-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        touch(&root, "Downloads/Rechnung_Telekom_Sept.pdf", 35);
        touch(&root, "Downloads/rechnung-telekom-okt.pdf", 5);
        touch(&root, "Dokumente/Finanzen/Rechnungen/Stromabrechnung 2025.pdf", 2);
        touch(&root, "Dokumente/Invoice_4711.pdf", 12);
        touch(&root, "Dokumente/Lebenslauf.docx", 100);
        touch(&root, "Dokumente/Bewerbung Firma.pdf", 30);
        touch(&root, "Bilder/Screenshots/Screenshot 2026-10-25.png", 1);
        touch(&root, "Bilder/Urlaub.jpg", 0);
        touch(&root, "Dokumente/.git/rechnung.pdf", 0);
        touch(&root, "Dokumente/node_modules/rechnung.pdf", 0);
        root
    }

    fn names(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|h| h.name.as_str()).collect()
    }

    #[test]
    fn query_parsing() {
        let q = parse_query("meine letzte Rechnung", 0, 0);
        assert!(q.latest);
        assert_eq!(q.groups.len(), 1);
        assert!(q.groups[0].strong.contains(&"invoice".to_string()));
        assert!(q.groups[0].weak.contains(&"beleg".to_string()));
        let q = parse_query("Screenshots von gestern", 10 * 86_400, 0);
        assert_eq!(q.kind, Some(Kind::Image));
        assert_eq!((q.since, q.until), (Some(9 * 86_400), Some(10 * 86_400)));
        let q = parse_query("meine Bewerbung als PDF", 0, 0);
        assert_eq!(q.kind, Some(Kind::Pdf));
        assert_eq!(q.label, "bewerbung pdf");
    }

    #[test]
    fn finds_latest_invoice_with_synonyms() {
        let root = setup("latest");
        let now = crate::time::unix_now();
        let q = parse_query("meine letzte Rechnung", now, 3600);
        let hits = search(std::slice::from_ref(&root), &[], &q, now, &Limits::default());
        // Neueste zuerst; "Rechnungen"-Ordner und "Invoice" zählen mit; .git/node_modules nicht
        assert_eq!(
            names(&hits),
            vec![
                "Stromabrechnung 2025.pdf",
                "rechnung-telekom-okt.pdf",
                "Invoice_4711.pdf",
                "Rechnung_Telekom_Sept.pdf"
            ]
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn kinds_and_periods() {
        let root = setup("kinds");
        let now = crate::time::unix_now();
        let hits =
            search(std::slice::from_ref(&root), &[], &parse_query("screenshots", now, 0), now, &Limits::default());
        assert_eq!(names(&hits), vec!["Screenshot 2026-10-25.png"]);
        let hits =
            search(std::slice::from_ref(&root), &[], &parse_query("bilder von heute", now, 0), now, &Limits::default());
        assert_eq!(names(&hits), vec!["Urlaub.jpg"]);
        let hits =
            search(std::slice::from_ref(&root), &[], &parse_query("lebenslauf", now, 0), now, &Limits::default());
        assert_eq!(names(&hits), vec!["Lebenslauf.docx"]);
        let hits = search(
            std::slice::from_ref(&root),
            &[root.join("Downloads")],
            &parse_query("telekom", now, 0),
            now,
            &Limits::default(),
        );
        assert!(hits.is_empty(), "ausgeschlossene Ordner werden nicht durchsucht");
        fs::remove_dir_all(root).ok();
    }
}

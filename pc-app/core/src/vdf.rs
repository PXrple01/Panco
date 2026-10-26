//! Minimaler Parser für Valves KeyValues-Format (.vdf/.acf).
//!
//! ```text
//! "AppState" { "appid" "1422450"  "name" "Deadlock" }
//! ```
//! Nur Strings und verschachtelte Blöcke; Kommentare mit `//`.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Str(String),
    Map(BTreeMap<String, Node>),
}

impl Node {
    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            // Schlüssel sind in Valve-Dateien nicht immer gleich geschrieben ("AppID"/"appid").
            Node::Map(m) => m.get(key).or_else(|| m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)),
            Node::Str(_) => None,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Node::Str(s) => Some(s),
            Node::Map(_) => None,
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = (&String, &Node)> {
        let map = match self {
            Node::Map(m) => Some(m),
            Node::Str(_) => None,
        };
        map.into_iter().flat_map(|m| m.iter())
    }
}

/// Parst eine ganze Datei als Map der Top-Level-Schlüssel.
pub fn parse(input: &str) -> Result<Node, String> {
    let mut tokens = Tokenizer { chars: input.chars().peekable() };
    let map = parse_map(&mut tokens, true)?;
    Ok(Node::Map(map))
}

enum Tok {
    Str(String),
    Open,
    Close,
}

struct Tokenizer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl Tokenizer<'_> {
    fn next(&mut self) -> Result<Option<Tok>, String> {
        loop {
            match self.chars.next() {
                None => return Ok(None),
                Some(c) if c.is_whitespace() => continue,
                Some('/') if self.chars.peek() == Some(&'/') => {
                    for c in self.chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                Some('{') => return Ok(Some(Tok::Open)),
                Some('}') => return Ok(Some(Tok::Close)),
                Some('"') => {
                    let mut s = String::new();
                    loop {
                        match self.chars.next() {
                            None => return Err("String nicht beendet".into()),
                            Some('"') => break,
                            Some('\\') => match self.chars.next() {
                                Some('n') => s.push('\n'),
                                Some('t') => s.push('\t'),
                                Some(c) => s.push(c),
                                None => return Err("Escape am Dateiende".into()),
                            },
                            Some(c) => s.push(c),
                        }
                    }
                    return Ok(Some(Tok::Str(s)));
                }
                Some(c) => {
                    // Unquotierte Tokens (selten): bis zum nächsten Leerzeichen/Klammer.
                    let mut s = String::from(c);
                    while let Some(&c) = self.chars.peek() {
                        if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                            break;
                        }
                        s.push(c);
                        self.chars.next();
                    }
                    return Ok(Some(Tok::Str(s)));
                }
            }
        }
    }
}

fn parse_map(t: &mut Tokenizer, top: bool) -> Result<BTreeMap<String, Node>, String> {
    let mut map = BTreeMap::new();
    loop {
        let key = match t.next()? {
            Some(Tok::Str(k)) => k,
            Some(Tok::Close) if !top => return Ok(map),
            None if top => return Ok(map),
            _ => return Err("Schlüssel erwartet".into()),
        };
        let value = match t.next()? {
            Some(Tok::Str(v)) => Node::Str(v),
            Some(Tok::Open) => Node::Map(parse_map(t, false)?),
            _ => return Err(format!("Wert für '{key}' erwartet")),
        };
        map.insert(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifest() {
        let acf = r#"
"AppState"
{
	"appid"		"1422450"
	"name"		"Deadlock"
	"StateFlags"		"4"
	"installdir"		"Deadlock"
	"UserConfig" { "language" "german" }
}"#;
        let root = parse(acf).unwrap();
        let app = root.get("AppState").unwrap();
        assert_eq!(app.str("appid"), Some("1422450"));
        assert_eq!(app.str("name"), Some("Deadlock"));
        assert_eq!(app.get("UserConfig").unwrap().str("language"), Some("german"));
    }

    #[test]
    fn escapes_and_comments() {
        let vdf = "// Kommentar\n\"libraryfolders\" { \"0\" { \"path\" \"C:\\\\Program Files (x86)\\\\Steam\" } }";
        let root = parse(vdf).unwrap();
        let path = root.get("libraryfolders").unwrap().get("0").unwrap().str("path");
        assert_eq!(path, Some("C:\\Program Files (x86)\\Steam"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("\"a\" {").is_err());
    }
}

//! Server-Sent Events der Messages-API zu fertigen Inhaltsblöcken zusammensetzen.
//!
//! Blöcke werden so aufbewahrt, wie sie kamen (inkl. Denk-Blöcken mit Signatur), damit sie
//! unverändert zurückgeschickt werden können – das Gespräch bleibt "append-only".

use serde_json::{json, Value};

#[derive(Debug, Default)]
pub struct StreamParser {
    blocks: Vec<Value>,
    partial: Vec<String>,
    /// tool_use-Blöcke, deren Eingabe kein gültiges JSON war: (Index, Rohtext)
    pub invalid: Vec<(usize, String)>,
    pub stop_reason: Option<String>,
    pub model: Option<String>,
    pub error: Option<String>,
    /// Bei HTTP-Fehlern: der Antworttext (JSON)
    raw: String,
}

/// Ein fertiger Antwort-Durchgang.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub content: Vec<Value>,
    pub stop_reason: String,
    pub model: String,
    pub invalid: Vec<(usize, String)>,
}

impl StreamParser {
    /// Eine Zeile verarbeiten; liefert neuen Text (für Anzeige/Sprache), falls welcher kam.
    pub fn line(&mut self, line: &str) -> Option<String> {
        let Some(data) = line.strip_prefix("data:") else {
            if !line.starts_with("event:") && !line.is_empty() {
                self.raw.push_str(line);
            }
            return None;
        };
        let v: Value = match serde_json::from_str(data.trim()) {
            Ok(v) => v,
            Err(_) => return None,
        };
        match v["type"].as_str().unwrap_or("") {
            "message_start" => {
                self.model = v["message"]["model"].as_str().map(str::to_string);
            }
            "content_block_start" => {
                let i = v["index"].as_u64().unwrap_or(self.blocks.len() as u64) as usize;
                while self.blocks.len() <= i {
                    self.blocks.push(Value::Null);
                    self.partial.push(String::new());
                }
                self.blocks[i] = v["content_block"].clone();
                if let Some(t) = self.blocks[i]["text"].as_str().filter(|t| !t.is_empty()) {
                    return Some(t.to_string());
                }
            }
            "content_block_delta" => {
                let i = v["index"].as_u64().unwrap_or(0) as usize;
                let Some(block) = self.blocks.get_mut(i) else { return None };
                let d = &v["delta"];
                match d["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let add = d["text"].as_str().unwrap_or("");
                        let old = block["text"].as_str().unwrap_or("").to_string();
                        block["text"] = Value::String(old + add);
                        return Some(add.to_string());
                    }
                    "input_json_delta" => self.partial[i].push_str(d["partial_json"].as_str().unwrap_or("")),
                    "thinking_delta" => {
                        let old = block["thinking"].as_str().unwrap_or("").to_string();
                        block["thinking"] = Value::String(old + d["thinking"].as_str().unwrap_or(""));
                    }
                    "signature_delta" => {
                        block["signature"] = d["signature"].clone();
                    }
                    "citations_delta" => {
                        if !block["citations"].is_array() {
                            block["citations"] = json!([]);
                        }
                        if let Some(arr) = block["citations"].as_array_mut() {
                            arr.push(d["citation"].clone());
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let i = v["index"].as_u64().unwrap_or(0) as usize;
                if let Some(block) = self.blocks.get_mut(i) {
                    if block["type"] == "tool_use" || block["type"] == "server_tool_use" {
                        let raw = std::mem::take(&mut self.partial[i]);
                        if !raw.trim().is_empty() {
                            // Streng parsen – nie "reparieren" (eager_input_streaming ist an).
                            match serde_json::from_str::<Value>(&raw) {
                                Ok(input) if input.is_object() => block["input"] = input,
                                _ => {
                                    block["input"] = json!({});
                                    self.invalid.push((i, raw));
                                }
                            }
                        }
                    }
                }
            }
            "message_delta" => {
                if let Some(r) = v["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(r.to_string());
                }
            }
            "error" => {
                self.error = Some(error_text(&v));
            }
            _ => {}
        }
        None
    }

    /// Fehlertext bei HTTP-Status ≠ 200 (der Körper kam als normale Zeilen).
    pub fn http_error(&self) -> String {
        serde_json::from_str::<Value>(&self.raw)
            .map(|v| error_text(&v))
            .unwrap_or_else(|_| self.raw.chars().take(200).collect())
    }

    pub fn finish(self) -> Result<Turn, String> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let stop_reason = self.stop_reason.ok_or("Antwort unvollständig (Verbindung abgebrochen)")?;
        let content: Vec<Value> = self.blocks.into_iter().filter(|b| !b.is_null()).collect();
        Ok(Turn {
            content: echo_safe(content),
            stop_reason,
            model: self.model.unwrap_or_default(),
            invalid: self.invalid,
        })
    }
}

fn error_text(v: &Value) -> String {
    let e = &v["error"];
    let kind = e["type"].as_str().unwrap_or("");
    let msg = e["message"].as_str().unwrap_or("");
    format!("{kind}: {msg}")
}

/// Nach einem Modellwechsel mitten in der Antwort (server-seitiger Fallback) dürfen Denk- und
/// Tool-Blöcke vor dem letzten `fallback`-Block nicht zurückgeschickt werden.
fn echo_safe(content: Vec<Value>) -> Vec<Value> {
    let Some(last) = content.iter().rposition(|b| b["type"] == "fallback") else { return content };
    content.into_iter().enumerate().filter(|(i, b)| *i >= last || b["type"] == "text").map(|(_, b)| b).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(lines: &[&str]) -> (StreamParser, String) {
        let mut p = StreamParser::default();
        let mut text = String::new();
        for l in lines {
            if let Some(t) = p.line(l) {
                text.push_str(&t);
            }
        }
        (p, text)
    }

    #[test]
    fn text_thinking_and_tool_use() {
        let (p, text) = feed(&[
            "event: message_start",
            r#"data: {"type":"message_start","message":{"id":"msg_1","model":"claude-opus-5-5"}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig=="}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Ich schau "}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"nach."}}"#,
            r#"data: {"type":"content_block_stop","index":1}"#,
            r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"screen_read","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"target\": \"win"}}"#,
            r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"dow\"}"}}"#,
            r#"data: {"type":"content_block_stop","index":2}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":20}}"#,
            r#"data: {"type":"message_stop"}"#,
        ]);
        assert_eq!(text, "Ich schau nach.");
        let turn = p.finish().unwrap();
        assert_eq!(turn.stop_reason, "tool_use");
        assert_eq!(turn.model, "claude-opus-5-5");
        assert_eq!(turn.content[0], json!({"type":"thinking","thinking":"","signature":"sig=="}));
        assert_eq!(turn.content[2]["input"], json!({"target":"window"}));
        assert!(turn.invalid.is_empty());
    }

    #[test]
    fn invalid_tool_json_and_errors() {
        let (p, _) = feed(&[
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t","name":"x","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"a\": \"b"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#,
        ]);
        let turn = p.finish().unwrap();
        assert_eq!(turn.invalid, vec![(0, "{\"a\": \"b".to_string())]);

        let (p, _) = feed(&[r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#]);
        assert_eq!(p.finish().unwrap_err(), "overloaded_error: Overloaded");

        let (p, _) =
            feed(&[r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#]);
        assert_eq!(p.http_error(), "authentication_error: invalid x-api-key");

        let (p, _) =
            feed(&[r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#]);
        assert!(p.finish().is_err(), "ohne stop_reason unvollständig");
    }

    #[test]
    fn fallback_blocks_are_echo_safe() {
        let content = vec![
            json!({"type":"thinking","thinking":"","signature":"a"}),
            json!({"type":"text","text":"Teil"}),
            json!({"type":"fallback","from":{"model":"x"},"to":{"model":"y"}}),
            json!({"type":"text","text":"Rest"}),
        ];
        let out = echo_safe(content);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["type"], "text");
    }
}

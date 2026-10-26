//! Lesen auf ausdrückliche Anfrage: Bildschirm (lokale Texterkennung zuerst), Bild an Claude,
//! aktives Fenster, Zwischenablage, Textdateien. Nie im Hintergrund – jede Nutzung steht im
//! Protokoll, und während des Lesens zeigt das Overlay, was Panco tut.

use super::*;
use crate::files::Zone;
use crate::platform::CaptureTarget;
use crate::texts::t;
use serde::Deserialize;
use std::path::PathBuf;

pub struct ScreenRead;
pub struct ScreenLook;
pub struct WindowActive;
pub struct ClipboardRead;
pub struct FileRead;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TargetP {
    target: Option<String>,
}

fn target(p: &TargetP) -> Result<CaptureTarget, ActionError> {
    match p.target.as_deref() {
        None | Some("window") => Ok(CaptureTarget::ActiveWindow),
        Some("screen") => Ok(CaptureTarget::Screen),
        _ => Err(ActionError::InvalidParams("target: window oder screen".into())),
    }
}

/// Für die Sprechblase: höchstens ~200 Zeichen, in einer Zeile.
fn preview(text: &str) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 200 {
        one.chars().take(197).collect::<String>() + " …"
    } else {
        one
    }
}

impl Action for ScreenRead {
    fn id(&self) -> &'static str {
        "screen.read"
    }
    fn prepare(&self, params: &Value, _ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: TargetP = super::params(params)?;
        let target = target(&p)?;
        Ok(Plan::instant(t("screen_reading", &[]), move |pf| {
            let img = pf.capture(target).map_err(|e| match e {
                PlatformError::Unsupported => ActionError::NotFound(t("screen_unavailable", &[])),
                other => other.into(),
            })?;
            let text = pf.ocr(&img)?;
            drop(img); // Bild sofort weg
            if text.trim().is_empty() {
                return Ok(Done::new(t("screen_empty", &[]), Mood::Info));
            }
            Ok(Done::new(t("screen_text", &[("text", &preview(&text))]), Mood::Info).data(text))
        }))
    }
}

impl Action for ScreenLook {
    fn id(&self) -> &'static str {
        "screen.look"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: TargetP = super::params(params)?;
        let target = target(&p)?;
        // Bilder gehen nur an Claude, nur im Claude-Modus und nur für diese Anfrage.
        if *ctx.source != Source::Claude {
            return Err(ActionError::NotFound(t("screen_look_claude_only", &[])));
        }
        if !ctx.config.claude.allow_screen_images {
            return Err(ActionError::NotFound(t("screen_look_off", &[])));
        }
        Ok(Plan::instant(t("screen_reading", &[]), move |pf| {
            let img = pf.capture(target).map_err(|e| match e {
                PlatformError::Unsupported => ActionError::NotFound(t("screen_unavailable", &[])),
                other => other.into(),
            })?;
            let small = crate::image::downscale(&img, crate::image::MAX_SIDE);
            drop(img);
            let png = crate::image::to_png(&small).map_err(ActionError::Failed)?;
            Ok(Done::new(t("screen_look_done", &[]), Mood::Info).image(png))
        }))
    }
}

impl Action for WindowActive {
    fn id(&self) -> &'static str {
        "window.active"
    }
    fn prepare(&self, params: &Value, _ctx: &Ctx) -> Result<Plan, ActionError> {
        no_params(params)?;
        Ok(Plan::instant("Aktives Fenster ansehen", |pf| {
            Ok(match pf.active_window()? {
                Some(w) => {
                    let process = w.process.trim_end_matches(".exe").to_string();
                    let text = t("window_active", &[("title", &w.title), ("process", &process)]);
                    Done::new(text.clone(), Mood::Info).data(format!("Titel: {}\nProgramm: {}", w.title, w.process))
                }
                None => Done::new(t("window_none", &[]), Mood::Info),
            })
        }))
    }
}

impl Action for ClipboardRead {
    fn id(&self) -> &'static str {
        "clipboard.read"
    }
    fn prepare(&self, params: &Value, _ctx: &Ctx) -> Result<Plan, ActionError> {
        no_params(params)?;
        Ok(Plan::instant("Zwischenablage lesen", |pf| {
            Ok(match pf.clipboard_text()?.filter(|s| !s.trim().is_empty()) {
                Some(text) => {
                    let text: String = text.chars().take(20_000).collect();
                    Done::new(t("clipboard_text", &[("text", &preview(&text))]), Mood::Info).data(text)
                }
                None => Done::new(t("clipboard_empty", &[]), Mood::Info),
            })
        }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileReadP {
    path: String,
    max_chars: Option<usize>,
}

/// Pfad aus einem Parameter: `~` und `%VAR%` auflösen, muss absolut sein.
pub(crate) fn user_path(raw: &str) -> Result<PathBuf, ActionError> {
    let raw = raw.trim();
    let expanded = if let Some(rest) = raw.strip_prefix('~') {
        dirs::home_dir().map(|h| format!("{}{rest}", h.display())).unwrap_or_else(|| raw.to_string())
    } else {
        crate::config::expand_env(raw)
    };
    let p = PathBuf::from(&expanded);
    if !p.is_absolute() {
        return Err(ActionError::InvalidParams("Bitte den vollständigen Pfad angeben".into()));
    }
    Ok(crate::files::normalize(&p))
}

pub(crate) fn file_label(p: &std::path::Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
}

/// Höchstens 2 MB, keine Binärdateien, keine geschützten Notizen.
pub(crate) fn read_text(path: &std::path::Path) -> Result<String, ActionError> {
    let name = file_label(path);
    if path.to_string_lossy().ends_with(".panco") {
        return Err(ActionError::NotFound(t("file_read_protected", &[])));
    }
    let meta = std::fs::metadata(path).map_err(|_| ActionError::NotFound(format!("„{name}“ gibt es nicht.")))?;
    if !meta.is_file() {
        return Err(ActionError::NotFound(format!("„{name}“ ist keine Datei.")));
    }
    if meta.len() > 2 << 20 {
        return Err(ActionError::NotFound(t("file_read_too_big", &[("name", &name)])));
    }
    let bytes = std::fs::read(path).map_err(|e| ActionError::Failed(e.to_string()))?;
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return Err(ActionError::NotFound(t("file_read_binary", &[("name", &name)])));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

impl Action for FileRead {
    fn id(&self) -> &'static str {
        "file.read"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: FileReadP = super::params(params)?;
        let path = user_path(&p.path)?;
        let tier = match ctx.places.zone(&path) {
            Zone::Protected => return Err(ActionError::NotFound("Diesen Ordner verwaltet Panco selbst.".into())),
            _ => ctx.places.read_tier(&path),
        };
        let max = p.max_chars.unwrap_or(50_000).clamp(100, 200_000);
        let name = file_label(&path);
        Ok(Plan::ready(tier, format!("„{name}“ lesen"), move |_| {
            let text = read_text(&path)?;
            let lines = text.lines().count();
            let total = text.chars().count();
            let mut data: String = text.chars().take(max).collect();
            if total > max {
                data.push_str(&format!("\n[… gekürzt: {max} von {total} Zeichen]"));
            }
            Ok(Done::new(t("file_read_done", &[("name", &name), ("lines", &lines.to_string())]), Mood::Info).data(data))
        }))
    }
}

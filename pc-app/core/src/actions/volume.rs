//! volume.set / volume.change / volume.mute – Gesamtlautstärke.
//! volume.app – Lautstärke eines Programms im Windows-Lautstärkemixer ("Spotify leiser").

use super::*;
use crate::texts::t;
use crate::undo::UndoOp;
use serde::Deserialize;

pub struct VolumeSet;
pub struct VolumeChange;
pub struct VolumeMute;
pub struct VolumeApp;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppP {
    app: String,
    level: Option<i64>,
    delta: Option<i64>,
    mute: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetP {
    level: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeP {
    delta: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MuteP {
    mute: Option<bool>,
}

impl Action for VolumeSet {
    fn id(&self) -> &'static str {
        "volume.set"
    }
    fn prepare(&self, params: &Value, _: &Ctx) -> Result<Plan, ActionError> {
        let p: SetP = super::params(params)?;
        if !(0..=100).contains(&p.level) {
            return Err(ActionError::InvalidParams("level muss 0–100 sein".into()));
        }
        let level = p.level as u8;
        Ok(Plan::instant(format!("Lautstärke auf {level}"), move |pf| {
            let before = pf.volume().ok();
            pf.set_volume(level)?;
            let done = Done::new(t("volume_set", &[("level", &level.to_string())]), Mood::Info);
            Ok(match before {
                Some(b) if b != level => done.undo(UndoOp::Volume { level: b }),
                _ => done,
            })
        }))
    }
}

impl Action for VolumeChange {
    fn id(&self) -> &'static str {
        "volume.change"
    }
    fn prepare(&self, params: &Value, _: &Ctx) -> Result<Plan, ActionError> {
        let p: ChangeP = super::params(params)?;
        if !(-100..=100).contains(&p.delta) || p.delta == 0 {
            return Err(ActionError::InvalidParams("delta muss -100–100 und ungleich 0 sein".into()));
        }
        let delta = p.delta;
        Ok(Plan::instant(if delta > 0 { "lauter" } else { "leiser" }, move |pf| {
            let before = pf.volume()?;
            let level = (before as i64 + delta).clamp(0, 100) as u8;
            pf.set_volume(level)?;
            Ok(Done::new(t("volume_set", &[("level", &level.to_string())]), Mood::Info)
                .undo(UndoOp::Volume { level: before }))
        }))
    }
}

impl Action for VolumeMute {
    fn id(&self) -> &'static str {
        "volume.mute"
    }
    fn prepare(&self, params: &Value, _: &Ctx) -> Result<Plan, ActionError> {
        let p: MuteP = super::params(params)?;
        Ok(Plan::instant("Ton umschalten", move |pf| {
            let mute = match p.mute {
                Some(m) => m,
                None => !pf.muted()?,
            };
            pf.set_muted(mute)?;
            Ok(Done::new(t(if mute { "volume_mute" } else { "volume_unmute" }, &[]), Mood::Info)
                .undo(UndoOp::Mute { muted: !mute }))
        }))
    }
}

/// "spotify" → "Spotify.exe": erst über die bekannten Programme (Aliase mit Prozessname),
/// sonst unscharf über die Programme, die gerade Ton ausgeben können.
pub(crate) fn resolve_app(name: &str, ctx: &Ctx, playing: &[crate::platform::AppVolume]) -> Option<String> {
    let wanted = crate::normalize::name(name);
    for alias in &ctx.config.apps {
        if let Some(process) = &alias.process {
            if alias.names.iter().any(|n| crate::normalize::name(n) == wanted) {
                if let Some(p) = playing.iter().find(|p| p.process.eq_ignore_ascii_case(process)) {
                    return Some(p.process.clone());
                }
            }
        }
    }
    let stem = |p: &str| crate::normalize::name(p.trim_end_matches(".exe").trim_end_matches(".EXE"));
    match crate::fuzzy::resolve(&wanted, playing, |p| vec![p.process.as_str()]) {
        crate::fuzzy::Resolved::Sure(p) => Some(p.process.clone()),
        _ => {
            playing.iter().find(|p| stem(&p.process).contains(&wanted) && wanted.len() >= 3).map(|p| p.process.clone())
        }
    }
}

fn app_label(process: &str) -> String {
    display_name(process.trim_end_matches(".exe").trim_end_matches(".EXE"))
}

impl Action for VolumeApp {
    fn id(&self) -> &'static str {
        "volume.app"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: AppP = super::params(params)?;
        let app = clean_text(&p.app, 60)?;
        let chosen = [p.level.is_some(), p.delta.is_some(), p.mute.is_some()].iter().filter(|b| **b).count();
        if chosen != 1 {
            return Err(ActionError::InvalidParams("genau eins von level, delta, mute".into()));
        }
        if p.level.is_some_and(|l| !(0..=100).contains(&l))
            || p.delta.is_some_and(|d| d == 0 || !(-100..=100).contains(&d))
        {
            return Err(ActionError::InvalidParams("level 0–100, delta -100–100".into()));
        }
        let playing = ctx.platform.app_volumes()?;
        let process = resolve_app(&app, ctx, &playing)
            .ok_or_else(|| ActionError::NotFound(t("app_not_playing", &[("app", &display_name(&app))])))?;
        let before = playing.iter().find(|a| a.process == process).cloned().expect("aufgelöst aus der Liste");
        let label = app_label(&process);
        Ok(Plan::instant(format!("Lautstärke von {label}"), move |pf| {
            let undo = UndoOp::AppVolume { process: process.clone(), level: before.level, muted: before.muted };
            let text = if let Some(mute) = p.mute {
                pf.set_app_volume(&process, None, Some(mute))?;
                t(if mute { "app_muted" } else { "app_unmuted" }, &[("app", &label)])
            } else {
                let level = p.level.unwrap_or_else(|| before.level as i64 + p.delta.unwrap_or(0)).clamp(0, 100) as u8;
                // Lauter machen hebt auch Stumm auf.
                pf.set_app_volume(&process, Some(level), before.muted.then_some(false))?;
                t("app_volume_set", &[("app", &label), ("level", &level.to_string())])
            };
            Ok(Done::new(text, Mood::Info).undo(undo))
        }))
    }
}

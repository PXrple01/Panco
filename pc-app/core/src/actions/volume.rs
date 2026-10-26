//! volume.set / volume.change / volume.mute – Gesamtlautstärke.
//! (Lautstärke pro App, z. B. Spotify, folgt in Etappe 5.)

use super::*;
use crate::texts::t;
use crate::undo::UndoOp;
use serde::Deserialize;

pub struct VolumeSet;
pub struct VolumeChange;
pub struct VolumeMute;

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

//! system.lock (Stufe 1) und system.power (Stufe 2: herunterfahren, neu starten, Ruhemodus).

use super::*;
use crate::platform::PowerMode;
use crate::texts::t;
use serde::Deserialize;

pub struct Lock;
pub struct Power;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PowerP {
    mode: Mode,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Shutdown,
    Restart,
    Sleep,
}

impl Action for Lock {
    fn id(&self) -> &'static str {
        "system.lock"
    }
    fn prepare(&self, params: &Value, _: &Ctx) -> Result<Plan, ActionError> {
        no_params(params)?;
        Ok(Plan::instant("PC sperren", |pf| {
            pf.lock()?;
            Ok(Done { text: t("lock", &[]), mood: Mood::Info })
        }))
    }
}

impl Action for Power {
    fn id(&self) -> &'static str {
        "system.power"
    }
    fn prepare(&self, params: &Value, _: &Ctx) -> Result<Plan, ActionError> {
        let p: PowerP = super::params(params)?;
        let (mode, what, done) = match p.mode {
            Mode::Shutdown => (PowerMode::Shutdown, "what_shutdown", "power_shutdown"),
            Mode::Restart => (PowerMode::Restart, "what_restart", "power_restart"),
            Mode::Sleep => (PowerMode::Sleep, "what_sleep", "power_sleep"),
        };
        Ok(Plan::confirm(t(what, &[]), move |pf| {
            pf.power(mode)?;
            Ok(Done { text: t(done, &[]), mood: Mood::Info })
        }))
    }
}

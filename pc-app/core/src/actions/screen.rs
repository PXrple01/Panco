//! screen.screenshot – Windows-eigener Screenshot (Win+Druck → Bilder\Screenshots).

use super::*;
use crate::texts::t;

pub struct Screenshot;

impl Action for Screenshot {
    fn id(&self) -> &'static str {
        "screen.screenshot"
    }
    fn prepare(&self, params: &Value, _: &Ctx) -> Result<Plan, ActionError> {
        no_params(params)?;
        Ok(Plan::instant("Screenshot machen", |pf| {
            pf.screenshot()?;
            Ok(Done { text: t("screenshot", &[]), mood: Mood::Info })
        }))
    }
}

//! window.focus – Fenster nach Titel oder Programmname in den Vordergrund holen.

use super::close::exe_stem;
use super::*;
use crate::fuzzy::{self, Resolved};
use crate::texts::t;
use serde::Deserialize;

pub struct WindowFocus;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct P {
    target: String,
}

impl Action for WindowFocus {
    fn id(&self) -> &'static str {
        "window.focus"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: P = super::params(params)?;
        let query = clean_text(&p.target, 120)?;
        let windows = ctx.platform.windows()?;
        let names = |w: &crate::platform::WindowInfo| -> Vec<String> {
            let mut n = vec![w.title.clone(), exe_stem(&w.process).to_string()];
            if let Some(a) = ctx
                .config
                .apps
                .iter()
                .find(|a| a.process.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(&w.process)))
            {
                n.extend(a.names.iter().cloned());
            }
            n
        };
        let named: Vec<(u64, String, Vec<String>)> =
            windows.iter().map(|w| (w.id, exe_stem(&w.process).to_string(), names(w))).collect();

        match fuzzy::resolve(&query, &named, |(_, _, n)| n.iter().map(String::as_str).collect()) {
            Resolved::Sure((id, proc_name, _)) | Resolved::Unsure((id, proc_name, _)) => {
                // Fokus wechseln ist harmlos – auch bei unsicherem Treffer direkt ausführen.
                let (id, name) = (*id, display_name(proc_name));
                Ok(Plan::instant(format!("zu {name} wechseln"), move |pf| {
                    pf.focus_window(id)?;
                    Ok(Done::new(t("focus_window", &[("name", &name)]), Mood::Info))
                }))
            }
            Resolved::None => Err(ActionError::NotFound(t("not_found", &[("name", &display_name(&query))]))),
        }
    }
}

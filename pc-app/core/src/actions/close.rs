//! app.close – Programm sanft schließen (Stufe 2, braucht Bestätigung).

use super::*;
use crate::fuzzy::{self, Resolved};
use crate::texts::t;
use serde::Deserialize;

pub struct AppClose;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct P {
    target: String,
}

/// Ein laufender Prozess mit allen Namen, unter denen man ihn ansprechen kann.
struct Running {
    process: String,
    display: String,
    names: Vec<String>,
}

pub(super) fn exe_stem(process: &str) -> &str {
    process.strip_suffix(".exe").or_else(|| process.strip_suffix(".EXE")).unwrap_or(process)
}

impl Action for AppClose {
    fn id(&self) -> &'static str {
        "app.close"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: P = super::params(params)?;
        let query = clean_text(&p.target, 120)?;
        let windows = ctx.platform.windows()?;

        let mut running: Vec<Running> = Vec::new();
        for w in windows.iter().filter(|w| !w.process.is_empty()) {
            let alias = ctx
                .config
                .apps
                .iter()
                .find(|a| a.process.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(&w.process)));
            let entry = match running.iter_mut().position(|r| r.process.eq_ignore_ascii_case(&w.process)) {
                Some(i) => &mut running[i],
                None => {
                    running.push(Running {
                        process: w.process.clone(),
                        display: alias
                            .map(|a| display_name(&a.names[0]))
                            .unwrap_or_else(|| exe_stem(&w.process).to_string()),
                        names: vec![exe_stem(&w.process).to_string()],
                    });
                    if let Some(a) = alias {
                        running.last_mut().unwrap().names.extend(a.names.iter().cloned());
                    }
                    running.last_mut().unwrap()
                }
            };
            entry.names.push(w.title.clone());
        }
        // Panco selbst nie per Sprachbefehl schließen (dafür gibt es Beenden im Tray).
        running.retain(|r| !exe_stem(&r.process).eq_ignore_ascii_case("panco"));

        match fuzzy::resolve(&query, &running, |r| r.names.iter().map(String::as_str).collect()) {
            Resolved::Sure(r) => {
                let (process, name) = (r.process.clone(), r.display.clone());
                Ok(Plan::confirm(t("what_close", &[("name", &name)]), move |pf| {
                    pf.close_process(&process)?;
                    Ok(Done::ok(t("close_app", &[("name", &name)])))
                }))
            }
            Resolved::Unsure(r) => Ok(Plan::Clarify {
                question: t("did_you_mean", &[("name", &r.display)]),
                params: serde_json::json!({ "target": exe_stem(&r.process) }),
            }),
            Resolved::None => Err(ActionError::NotFound(t("not_running", &[("name", &display_name(&query))]))),
        }
    }
}

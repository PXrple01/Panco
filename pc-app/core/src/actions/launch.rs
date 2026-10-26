//! app.launch – Steam-Spiel (steam://rungameid) oder Alias aus der Konfig öffnen.

use super::*;
use crate::config::{expand_env, AppAlias};
use crate::fuzzy::{self, Resolved};
use crate::normalize;
use crate::texts::t;
use serde::Deserialize;

pub struct AppLaunch;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct P {
    target: String,
}

enum Target<'a> {
    Game(&'a Game),
    App(&'a AppAlias),
}

impl Target<'_> {
    fn name(&self) -> String {
        match self {
            Target::Game(g) => g.name.clone(),
            Target::App(a) => display_name(&a.names[0]),
        }
    }
}

impl Action for AppLaunch {
    fn id(&self) -> &'static str {
        "app.launch"
    }

    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: P = super::params(params)?;
        let query = clean_text(&p.target, 120)?;

        // Kandidaten: Spiele (mit Kurznamen aus game_aliases) und Programme aus der Konfig.
        let mut items: Vec<(Target, Vec<&str>)> = Vec::new();
        for g in ctx.games {
            let mut names = vec![g.name.as_str()];
            for (alias, full) in &ctx.config.game_aliases {
                if normalize::name(full) == normalize::name(&g.name) {
                    names.push(alias.as_str());
                }
            }
            items.push((Target::Game(g), names));
        }
        for a in ctx.config.apps.iter().filter(|a| !a.names.is_empty()) {
            items.push((Target::App(a), a.names.iter().map(String::as_str).collect()));
        }

        match fuzzy::resolve(&query, &items, |(_, names)| names.clone()) {
            Resolved::Sure((target, _)) => Ok(plan_for(target)),
            Resolved::Unsure((target, _)) => {
                let name = target.name();
                Ok(Plan::clarify(t("did_you_mean", &[("name", &name)]), serde_json::json!({ "target": name })))
            }
            Resolved::None => Err(ActionError::NotFound(t("not_found", &[("name", &display_name(&query))]))),
        }
    }
}

fn plan_for(target: &Target) -> Plan {
    let name = target.name();
    match target {
        Target::Game(g) => {
            let uri = format!("steam://rungameid/{}", g.app_id);
            Plan::instant(format!("{name} starten"), move |pf| {
                pf.open(&uri, &[])?;
                Ok(Done::new(t("launch_game", &[("name", &name)]), Mood::Game))
            })
        }
        Target::App(a) => {
            let open = expand_env(&a.open);
            let args = a.args.clone();
            Plan::instant(format!("{name} öffnen"), move |pf| {
                pf.open(&open, &args)?;
                Ok(Done::ok(t("launch_app", &[("name", &name)])))
            })
        }
    }
}

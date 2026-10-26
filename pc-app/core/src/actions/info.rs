//! info.time – Uhrzeit und Datum; info.playtime – Spielzeit; panco.help – was Panco kann.

use super::*;
use crate::texts::t;
use serde::Deserialize;

const WEEKDAYS: [&str; 7] = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"];
const MONTHS: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];

pub struct InfoTime;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TimeP {
    /// "time" (Standard) oder "date"
    what: Option<String>,
}

impl Action for InfoTime {
    fn id(&self) -> &'static str {
        "info.time"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: TimeP = super::params(params)?;
        let d = crate::time::to_local(ctx.now, ctx.platform.utc_offset_secs());
        let text = match p.what.as_deref().unwrap_or("time") {
            "time" => t("time_now", &[("time", &format!("{}:{:02}", d.hour, d.minute))]),
            "date" => t(
                "date_now",
                &[
                    ("weekday", WEEKDAYS[d.weekday as usize % 7]),
                    ("day", &d.day.to_string()),
                    ("month", MONTHS[(d.month as usize + 11) % 12]),
                    ("year", &d.year.to_string()),
                ],
            ),
            _ => return Err(ActionError::InvalidParams("what: time oder date".into())),
        };
        Ok(Plan::instant("Uhrzeit/Datum sagen", move |_| Ok(Done::new(text, Mood::Info))))
    }
}

pub struct Help;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HelpP {
    /// "screen" = Frage nach Bildschirm/Fenster-Inhalt (kommt mit dem Claude-Assistenten)
    topic: Option<String>,
}

impl Action for Help {
    fn id(&self) -> &'static str {
        "panco.help"
    }
    fn prepare(&self, params: &Value, _: &Ctx) -> Result<Plan, ActionError> {
        let p: HelpP = super::params(params)?;
        let text = match p.topic.as_deref() {
            None => t("help", &[]),
            Some("screen") => t("help_screen", &[]),
            Some(_) => return Err(ActionError::InvalidParams("unbekanntes Thema".into())),
        };
        Ok(Plan::instant("Hilfe", move |_| Ok(Done::new(text, Mood::Info))))
    }
}

pub struct Playtime;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayP {
    /// now | today | week | total
    period: String,
    game: Option<String>,
}

fn list(items: &[(String, u64)]) -> String {
    items
        .iter()
        .take(4)
        .map(|(g, s)| format!("{g} {}", crate::playtime::duration_de(*s)))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Action for Playtime {
    fn id(&self) -> &'static str {
        "info.playtime"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        use crate::playtime::{duration_de, Tracker};
        let p: PlayP = super::params(params)?;
        let empty = crate::playtime::Playtime::default();
        let data = ctx.play.map(|t| &t.data).unwrap_or(&empty);
        let current = ctx.play.and_then(|t| t.current.as_ref());
        let offset = ctx.platform.utc_offset_secs();
        let text = match (p.period.as_str(), p.game.as_deref()) {
            ("now", _) => match current {
                Some(c) => {
                    t("playing_now", &[("game", &c.name), ("time", &duration_de((ctx.now - c.since).max(0) as u64))])
                }
                None => t("playing_none", &[]),
            },
            (_, Some(game)) => {
                let names = data.games();
                let name = match crate::fuzzy::resolve(game, &names, |n| vec![*n]) {
                    crate::fuzzy::Resolved::Sure(n) | crate::fuzzy::Resolved::Unsure(n) => n.to_string(),
                    crate::fuzzy::Resolved::None => {
                        return Err(ActionError::NotFound(t("playtime_unknown", &[("game", &display_name(game))])))
                    }
                };
                let week = data.since_day(&Tracker::week_start_key(ctx.now, offset));
                let w = week.iter().find(|(g, _)| *g == name).map_or(0, |x| x.1);
                t(
                    "playtime_game",
                    &[("game", &name), ("week", &duration_de(w)), ("total", &duration_de(data.total(&name)))],
                )
            }
            (period @ ("today" | "week"), None) => {
                let from = if period == "today" {
                    Tracker::today_key(ctx.now, offset)
                } else {
                    Tracker::week_start_key(ctx.now, offset)
                };
                let items = data.since_day(&from);
                let when = if period == "today" { "Heute" } else { "Diese Woche" };
                if items.is_empty() {
                    t("playtime_nothing", &[("when", when)])
                } else {
                    let sum: u64 = items.iter().map(|x| x.1).sum();
                    t("playtime_sum", &[("when", when), ("time", &duration_de(sum)), ("list", &list(&items))])
                }
            }
            ("total", None) => {
                let items = data.since_day("0000");
                if items.is_empty() {
                    t("playtime_nothing", &[("when", "Bisher")])
                } else {
                    let sum: u64 = items.iter().map(|x| x.1).sum();
                    t("playtime_sum", &[("when", "Insgesamt"), ("time", &duration_de(sum)), ("list", &list(&items))])
                }
            }
            _ => return Err(ActionError::InvalidParams("period: now, today, week oder total".into())),
        };
        Ok(Plan::instant("Spielzeit sagen", move |_| Ok(Done::new(text, Mood::Info))))
    }
}

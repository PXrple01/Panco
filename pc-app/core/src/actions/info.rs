//! info.time – Uhrzeit und Datum; panco.help – was Panco kann (und was noch nicht).

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

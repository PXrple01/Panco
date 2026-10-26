//! media.control – Medientasten (wirkt auf Spotify, Browser, alles mit Mediensteuerung).

use super::*;
use crate::platform::MediaKey;
use crate::texts::t;
use serde::Deserialize;

pub struct MediaControl;

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Command {
    PlayPause,
    Next,
    Previous,
    Stop,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct P {
    command: Command,
}

impl Action for MediaControl {
    fn id(&self) -> &'static str {
        "media.control"
    }
    fn prepare(&self, params: &Value, _: &Ctx) -> Result<Plan, ActionError> {
        let p: P = super::params(params)?;
        let (key, text) = match p.command {
            Command::PlayPause => (MediaKey::PlayPause, "media_play_pause"),
            Command::Next => (MediaKey::Next, "media_next"),
            Command::Previous => (MediaKey::Previous, "media_previous"),
            Command::Stop => (MediaKey::Stop, "media_stop"),
        };
        Ok(Plan::instant(t(text, &[]), move |pf| {
            pf.media_key(key)?;
            Ok(Done::new(t(text, &[]), Mood::Info))
        }))
    }
}

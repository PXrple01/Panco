//! spotify.play / spotify.now – Spotify Web-API (optional, nur wenn verbunden).

use super::*;
use crate::spotify::{self, SpotifyError};
use crate::texts::t;
use serde::Deserialize;

pub struct Play;
pub struct Now;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayP {
    query: String,
    /// track (Standard), playlist, artist, album
    kind: Option<String>,
}

fn explain(e: SpotifyError) -> ActionError {
    ActionError::NotFound(match e {
        SpotifyError::NotConnected => t("spotify_not_connected", &[]),
        SpotifyError::Premium => t("spotify_premium", &[]),
        SpotifyError::NoDevice => t("spotify_no_device", &[]),
        SpotifyError::NothingFound(q) => t("spotify_nothing", &[("query", &q)]),
        SpotifyError::Other(m) => t("error", &[("reason", &m)]),
    })
}

/// Ohne Client-ID oder Anmeldung gar nicht erst ins Internet.
fn client(ctx: &Ctx) -> Result<String, ActionError> {
    let id = ctx.config.spotify_client_id.trim().to_string();
    if id.is_empty() || !spotify::is_connected(ctx.platform) {
        return Err(explain(SpotifyError::NotConnected));
    }
    Ok(id)
}

impl Action for Play {
    fn id(&self) -> &'static str {
        "spotify.play"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        let p: PlayP = super::params(params)?;
        let query = clean_text(&p.query, 120)?;
        let kind = p.kind.unwrap_or_else(|| "track".into());
        if !["track", "playlist", "artist", "album"].contains(&kind.as_str()) {
            return Err(ActionError::InvalidParams("kind: track, playlist, artist oder album".into()));
        }
        let id = client(ctx)?;
        Ok(Plan::instant(format!("„{query}“ auf Spotify abspielen"), move |pf| {
            let label = spotify::play(pf, &id, &query, &kind).map_err(explain)?;
            Ok(Done::new(t("spotify_playing", &[("what", &label)]), Mood::Success))
        }))
    }
}

impl Action for Now {
    fn id(&self) -> &'static str {
        "spotify.now"
    }
    fn prepare(&self, params: &Value, ctx: &Ctx) -> Result<Plan, ActionError> {
        no_params(params)?;
        let id = client(ctx)?;
        Ok(Plan::instant("Gerade laufenden Titel sagen", move |pf| {
            let text = match spotify::now_playing(pf, &id).map_err(explain)? {
                Some((title, artist, true)) => t("spotify_now", &[("title", &title), ("artist", &artist)]),
                Some((title, artist, false)) => t("spotify_paused", &[("title", &title), ("artist", &artist)]),
                None => t("spotify_silent", &[]),
            };
            Ok(Done::new(text, Mood::Info))
        }))
    }
}

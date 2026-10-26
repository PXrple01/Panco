//! panco-core – das plattformunabhängige "Gehirn" von Panco.
//!
//! Ablauf eines Befehls:
//! ```text
//! Text ─▶ parser::parse ─▶ Intent{action, params} ─▶ Engine
//!          Engine: Not-Aus? ─▶ Katalog (Whitelist, Remote?) ─▶ Action::prepare (Parameter prüfen,
//!          Ziel auflösen) ─▶ Stufe 1: ausführen │ Stufe 2/3: Bestätigung ─▶ Log
//! ```
//! Windows-spezifisches steckt ausschließlich in `platform::windows`.

pub mod actions;
pub mod catalog;
pub mod config;
pub mod engine;
pub mod fuzzy;
pub mod log;
pub mod normalize;
pub mod parser;
pub mod platform;
pub mod protocol;
pub mod steam;
pub mod texts;
pub mod vdf;

pub use engine::{Engine, Source};

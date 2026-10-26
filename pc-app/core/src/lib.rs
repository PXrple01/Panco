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
pub mod backup;
pub mod catalog;
pub mod claude;
pub mod config;
pub mod engine;
pub mod files;
pub mod fuzzy;
pub mod image;
pub mod log;
pub mod memory;
pub mod normalize;
pub mod notes;
pub mod parser;
pub mod platform;
pub mod playtime;
pub mod protocol;
pub mod shell;
pub mod spotify;
pub mod steam;
pub mod texts;
pub mod time;
pub mod undo;
pub mod vdf;

pub use engine::{Engine, Source};

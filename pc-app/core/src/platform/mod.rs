//! Betriebssystem-Schnittstelle. Aktionen sprechen nur über dieses Trait mit dem System –
//! so bleibt die Whitelist an einer Stelle und alles ist ohne Windows testbar (MockPlatform).

use std::path::PathBuf;
use std::sync::Arc;

pub mod mock;
#[cfg(not(windows))]
mod other;
#[cfg(windows)]
mod windows;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PlatformError {
    #[error("auf diesem System nicht verfügbar")]
    Unsupported,
    #[error("{0}")]
    Failed(String),
}

pub type PResult<T> = Result<T, PlatformError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKey {
    PlayPause,
    Next,
    Previous,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerMode {
    Shutdown,
    Restart,
    Sleep,
}

/// Ressourcen von Panco selbst und seinen Kindprozessen (WebView2).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ProcStats {
    /// Privater Arbeitssatz von Panco in MB (wie im Task-Manager).
    pub own_mb: f64,
    /// Summe der Kindprozesse (Overlay-WebView) in MB.
    pub children_mb: f64,
    /// CPU-Zeit aller Prozesse zusammen in Sekunden (für Prozent: Differenz / Zeit).
    pub cpu_seconds: f64,
    pub children: Vec<String>,
}

/// Sichtbares Top-Level-Fenster.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowInfo {
    pub id: u64,
    pub pid: u32,
    pub title: String,
    /// Dateiname der Exe, z. B. "Discord.exe".
    pub process: String,
}

pub trait Platform: Send + Sync {
    /// Öffnet einen Pfad, eine URI (steam://, discord://) oder https-URL mit festen Argumenten.
    /// Keine Shell: Windows ShellExecute bzw. xdg-open, Argumente werden nie interpretiert.
    fn open(&self, target: &str, args: &[String]) -> PResult<()>;
    fn volume(&self) -> PResult<u8>;
    fn set_volume(&self, level: u8) -> PResult<()>;
    fn muted(&self) -> PResult<bool>;
    fn set_muted(&self, mute: bool) -> PResult<()>;
    fn media_key(&self, key: MediaKey) -> PResult<()>;
    fn screenshot(&self) -> PResult<()>;
    fn lock(&self) -> PResult<()>;
    fn power(&self, mode: PowerMode) -> PResult<()>;
    fn windows(&self) -> PResult<Vec<WindowInfo>>;
    fn focus_window(&self, id: u64) -> PResult<()>;
    /// Schließt ein Programm sanft (WM_CLOSE an alle seine Fenster), kein Kill.
    fn close_process(&self, process: &str) -> PResult<usize>;
    /// RAM/CPU von Panco und seinen Kindprozessen (nur für das Diagnosefenster).
    fn process_stats(&self) -> PResult<ProcStats>;
    /// Datei im Explorer anzeigen (Ordner öffnen, Datei markiert).
    fn reveal(&self, path: &std::path::Path) -> PResult<()>;
    /// Verschlüsseln, gebunden an das Windows-Benutzerkonto (DPAPI). Für sensible Notizen.
    fn protect(&self, data: &[u8]) -> PResult<Vec<u8>>;
    fn unprotect(&self, data: &[u8]) -> PResult<Vec<u8>>;
    /// Versatz der lokalen Zeit zu UTC in Sekunden (inkl. Sommerzeit).
    fn utc_offset_secs(&self) -> i32;
}

/// Die echte Plattform des laufenden Systems.
pub fn native() -> Arc<dyn Platform> {
    #[cfg(windows)]
    return Arc::new(windows::WindowsPlatform);
    #[cfg(not(windows))]
    return Arc::new(other::OtherPlatform);
}

/// Steam-Installationspfad aus der Registry (nur Windows).
pub fn steam_path_from_registry() -> Option<PathBuf> {
    #[cfg(windows)]
    return windows::steam_path();
    #[cfg(not(windows))]
    return None;
}

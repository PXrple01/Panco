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

/// Programm mit eigener Lautstärke im Windows-Lautstärkemixer.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AppVolume {
    /// Exe-Name, z. B. "Spotify.exe"
    pub process: String,
    pub level: u8,
    pub muted: bool,
}

/// HTTP-Anfrage (nur für Funktionen, die der Nutzer eingeschaltet hat, z. B. Spotify).
#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
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
    /// Programme, die gerade Ton ausgeben können (Lautstärkemixer).
    fn app_volumes(&self) -> PResult<Vec<AppVolume>>;
    /// Lautstärke (0–100) und/oder Stumm für alle Audio-Sitzungen dieses Programms. Liefert die Anzahl.
    fn set_app_volume(&self, process: &str, level: Option<u8>, mute: Option<bool>) -> PResult<usize>;
    /// Exe-Namen aller laufenden Programme (Spiel-Erkennung).
    fn processes(&self) -> PResult<Vec<String>>;
    /// HTTPS-Anfrage → (Status, Antworttext).
    fn http(&self, req: &HttpRequest) -> PResult<(u16, String)> {
        http_send(req)
    }
    /// Geheimnis aus dem Betriebssystem-Tresor (Windows-Anmeldeinformationsverwaltung).
    fn secret_get(&self, key: &str) -> PResult<Option<String>>;
    /// Geheimnis speichern (`None` = löschen).
    fn secret_set(&self, key: &str, value: Option<&str>) -> PResult<()>;
    /// Kryptografisch sichere Zufallsbytes.
    fn random_bytes(&self, n: usize) -> PResult<Vec<u8>>;
}

/// HTTPS mit den Zertifikaten des Systems, 15 s Zeitlimit, Antwort höchstens 1 MB.
pub fn http_send(req: &HttpRequest) -> PResult<(u16, String)> {
    let fail = |e: ureq::Error| PlatformError::Failed(format!("Verbindung fehlgeschlagen: {e}"));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .into();
    let mut builder = ureq::http::Request::builder().method(req.method).uri(&req.url);
    for (k, v) in &req.headers {
        builder = builder.header(k, v);
    }
    let request =
        builder.body(req.body.clone().unwrap_or_default()).map_err(|e| PlatformError::Failed(e.to_string()))?;
    let mut resp = agent.run(request).map_err(fail)?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().with_config().limit(1 << 20).read_to_string().unwrap_or_default();
    Ok((status, text))
}

/// Die echte Plattform des laufenden Systems.
pub fn native() -> Arc<dyn Platform> {
    #[cfg(windows)]
    return Arc::new(windows::WindowsPlatform);
    #[cfg(not(windows))]
    return Arc::new(other::OtherPlatform);
}

/// Laufendes Steam-Spiel (App-ID aus `HKCU\Software\Valve\Steam\RunningAppID`), nur Windows.
pub fn steam_running_app() -> Option<u32> {
    #[cfg(windows)]
    return windows::steam_running_app();
    #[cfg(not(windows))]
    return None;
}

/// Wartet, bis Steam einen Wert ändert (Spiel startet/endet) oder `timeout` abläuft – blockierend,
/// ohne Polling (Windows: RegNotifyChangeKeyValue). Sonst: schläft einfach `timeout`.
pub fn wait_steam_change(timeout: std::time::Duration) {
    #[cfg(windows)]
    windows::wait_steam_change(timeout);
    #[cfg(not(windows))]
    std::thread::sleep(timeout);
}

/// Steam-Installationspfad aus der Registry (nur Windows).
pub fn steam_path_from_registry() -> Option<PathBuf> {
    #[cfg(windows)]
    return windows::steam_path();
    #[cfg(not(windows))]
    return None;
}

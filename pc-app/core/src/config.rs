//! Einstellungen in `%APPDATA%\Panco\config.json` (Linux: `~/.config/Panco`).
//! Fehlende Felder bekommen Standardwerte; eine kaputte Datei wird nie überschrieben.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub hotkeys: Hotkeys,
    pub overlay: OverlayConfig,
    pub voice: VoiceSettings,
    pub files: FileSettings,
    pub setup: SetupSettings,
    pub system: SystemSettings,
    /// Spotify Web-API (optional): eigene Client-ID aus dem Spotify-Entwicklerportal. Leer = aus.
    pub spotify_client_id: String,
    /// Programme, Webseiten und URIs, die Panco per Name öffnen darf.
    pub apps: Vec<AppAlias>,
    /// Kurznamen für Spiele, z. B. "cs" → "Counter-Strike 2".
    pub game_aliases: BTreeMap<String, String>,
    /// Spiele außerhalb von Steam, erkannt am Exe-Namen (Prozessliste alle 12 s): Exe → Name.
    pub game_processes: BTreeMap<String, String>,
    /// Steam-Ordner; leer = automatisch (Registry) finden.
    pub steam_path: Option<String>,
    /// Such-URL, `{q}` wird durch die Suchanfrage ersetzt.
    pub search_url: String,
    /// Wie lange eine Bestätigungsfrage gültig ist.
    pub confirm_timeout_secs: u64,
    /// Aktionen der Stufe 2, die ohne Rückfrage laufen – nur für Befehle direkt am PC
    /// (getippt/gesprochen), nie für iPhone, Claude oder Regeln. Stufe 3 fragt immer.
    pub skip_confirm: Vec<String>,
}

/// Start mit Windows und Absturz-Wächter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SystemSettings {
    /// Mit Windows starten (Eintrag unter HKCU\…\Run)
    pub autostart: bool,
    /// Kleiner Wächter-Prozess startet Panco nach einem Absturz neu
    pub watchdog: bool,
}

impl Default for SystemSettings {
    fn default() -> Self {
        Self { autostart: true, watchdog: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub overlay: String,
    pub kill_switch: String,
    /// Halten = sprechen (funktioniert in jedem Sprachmodus).
    pub push_to_talk: String,
    pub mic_mute: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceMode {
    /// Offenes Mikro, lokales Wake-Word "Panco" (Standard).
    Wake,
    /// Mikro nur, solange die Push-to-Talk-Taste gehalten wird.
    PushToTalk,
    /// Keine Sprachsteuerung.
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SttChoice {
    /// whisper.cpp, lokal
    Whisper,
    /// Windows-Spracherkennung (nur Befehlsliste, offline) – zum Vergleich
    Windows,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceSettings {
    pub mode: VoiceMode,
    pub engine: SttChoice,
    /// Whisper-Modell: "tiny", "base" oder "small"
    pub model: String,
    /// So lange bleibt Whisper nach dem letzten Befehl geladen.
    pub keep_model_secs: u64,
    /// Empfindlichkeit des Wake-Words (0.3 = empfindlich … 0.7 = streng)
    pub wake_threshold: f32,
    /// Antworten per Windows-Sprachausgabe vorlesen
    pub speak_replies: bool,
    /// Overlay bei Sprachbefehlen einblenden (ohne Fokus zu stehlen)
    pub show_overlay: bool,
    /// Windows-Stimme (Anzeigename, z. B. "Microsoft Katja"); leer = automatisch eine deutsche Frauenstimme
    pub tts_voice: String,
    /// Tonhöhe 0.5 … 2.0 (1.0 = normal). Panco klingt etwas höher, passend zum Avatar.
    pub tts_pitch: f32,
    /// Sprechtempo 0.5 … 2.0 (1.0 = normal)
    pub tts_rate: f32,
    /// Natürliche Stimme (Piper, lokal): "kerstin", "ramona", "eva", "thorsten"; leer = Windows-Stimme
    pub natural_voice: String,
    /// Tonhöhe der natürlichen Stimme 0.7 … 1.6
    pub natural_pitch: f32,
    /// Tempo der natürlichen Stimme 0.5 … 2.0
    pub natural_rate: f32,
    /// So lange bleibt die natürliche Stimme nach dem Sprechen geladen.
    pub natural_keep_secs: u64,
    /// Bei Rückfragen schon zuhören, während Panco noch spricht
    pub barge_in: bool,
    /// "Panco" aus Lautsprechern/Stream ignorieren (Systemton-Abgleich)
    pub loopback_check: bool,
    /// Nur auf die eigene Stimme reagieren (einfaches Stimmprofil aus dem Einsprechen)
    pub speaker_check: bool,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            mode: VoiceMode::Wake,
            engine: SttChoice::Whisper,
            model: "base".into(),
            keep_model_secs: 30,
            wake_threshold: 0.5,
            speak_replies: true,
            show_overlay: true,
            tts_voice: String::new(),
            tts_pitch: 1.4,
            tts_rate: 1.1,
            natural_voice: String::new(),
            natural_pitch: 1.15,
            natural_rate: 1.05,
            natural_keep_secs: 60,
            barge_in: true,
            loopback_check: true,
            speaker_check: false,
        }
    }
}

/// Ordner, Papierkorb, Notizen, Backup. Pfade dürfen `%VAR%` enthalten.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FileSettings {
    /// Hier arbeitet Panco ohne Nachfrage (lesen, öffnen, anlegen).
    pub allowed_folders: Vec<String>,
    pub notes_dir: String,
    /// Panco-Papierkorb: "Gelöschtes" landet hier und wird erst nach `trash_days` entfernt.
    pub trash_dir: String,
    pub trash_days: u32,
    pub backup_enabled: bool,
    pub backup_dir: String,
    /// So viele Backups (Tage) bleiben erhalten.
    pub backup_keep: u32,
}

/// Ersteinrichtung (wird beim ersten Start abgefragt).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SetupSettings {
    pub done: bool,
    /// Wohnort fürs Wetter (Etappe 8), optional.
    pub city: String,
}

fn dir_or_home(d: Option<PathBuf>, fallback: &str) -> PathBuf {
    d.unwrap_or_else(|| dirs::home_dir().unwrap_or_else(std::env::temp_dir).join(fallback))
}

impl Default for FileSettings {
    fn default() -> Self {
        let documents = dir_or_home(dirs::document_dir(), "Documents");
        let home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
        let notes = documents.join("Panco-Notizen");
        let s = |p: PathBuf| p.to_string_lossy().to_string();
        Self {
            allowed_folders: vec![
                s(documents.clone()),
                s(dir_or_home(dirs::download_dir(), "Downloads")),
                s(dir_or_home(dirs::desktop_dir(), "Desktop")),
                s(notes.clone()),
            ],
            notes_dir: s(notes),
            trash_dir: s(home.join("Panco").join("Papierkorb")),
            trash_days: 30,
            backup_enabled: true,
            backup_dir: s(documents.join("Panco-Backups")),
            backup_keep: 7,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayConfig {
    /// Nach erfolgreichem Befehl ausblenden (0 = nie).
    pub auto_hide_ms: u64,
    pub reduce_motion: bool,
    pub show_avatar: bool,
    /// Windows-Acrylic-Hintergrund (Glas-Effekt).
    pub acrylic: bool,
    /// Bei "Panco, …" nur ein kleines Icon mit dem erkannten Text (statt der großen Karte).
    pub compact: bool,
    /// Bildschirm (Name, wie Windows ihn meldet); leer = Hauptbildschirm.
    pub monitor: String,
    pub anchor: OverlayAnchor,
    /// Selbst verschobene Position: Anteil (0…1) der freien Fläche des Bildschirms, links/oben.
    pub custom: Option<[f64; 2]>,
    /// Größe 0.75 … 1.5
    pub scale: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayAnchor {
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
    Center,
    /// Per Ziehen verschoben (siehe `custom`)
    Custom,
}

/// Ein Programm/Ziel, das Panco öffnen darf. `open` ist ein Pfad (Umgebungsvariablen wie
/// `%LOCALAPPDATA%` erlaubt), eine URI (`discord://`, `ms-settings:`) oder eine https-URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppAlias {
    pub names: Vec<String>,
    pub open: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Prozessname zum Beenden/Fokussieren, z. B. "Discord.exe".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            overlay: "Ctrl+Space".into(),
            kill_switch: "Ctrl+Alt+End".into(),
            push_to_talk: "Ctrl+Alt+Space".into(),
            mic_mute: "Ctrl+Alt+M".into(),
        }
    }
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            auto_hide_ms: 1600,
            reduce_motion: false,
            show_avatar: true,
            acrylic: true,
            compact: true,
            monitor: String::new(),
            anchor: OverlayAnchor::TopRight,
            custom: None,
            scale: 1.0,
        }
    }
}

fn app(names: &[&str], open: &str, args: &[&str], process: Option<&str>) -> AppAlias {
    AppAlias {
        names: names.iter().map(|s| s.to_string()).collect(),
        open: open.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        process: process.map(str::to_string),
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkeys: Hotkeys::default(),
            overlay: OverlayConfig::default(),
            voice: VoiceSettings::default(),
            files: FileSettings::default(),
            setup: SetupSettings::default(),
            system: SystemSettings::default(),
            skip_confirm: Vec::new(),
            spotify_client_id: String::new(),
            game_processes: [
                ("VALORANT-Win64-Shipping.exe", "Valorant"),
                ("League of Legends.exe", "League of Legends"),
                ("FortniteClient-Win64-Shipping.exe", "Fortnite"),
                ("RobloxPlayerBeta.exe", "Roblox"),
                ("Minecraft.Windows.exe", "Minecraft"),
            ]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect(),
            apps: vec![
                app(&["discord"], "discord://", &[], Some("Discord.exe")),
                app(&["spotify"], "spotify:", &[], Some("Spotify.exe")),
                app(&["steam"], "steam://open/main", &[], Some("steam.exe")),
                app(
                    &["obs", "obs studio"],
                    "%ProgramFiles%\\obs-studio\\bin\\64bit\\obs64.exe",
                    &[],
                    Some("obs64.exe"),
                ),
                app(&["browser", "internet"], "https://www.google.com", &[], None),
                app(&["youtube"], "https://www.youtube.com", &[], None),
                app(&["twitch"], "https://www.twitch.tv", &[], None),
                app(&["explorer", "dateien", "datei explorer"], "%WINDIR%\\explorer.exe", &[], Some("explorer.exe")),
                app(&["rechner", "taschenrechner"], "calc.exe", &[], Some("CalculatorApp.exe")),
                app(&["editor", "notepad"], "notepad.exe", &[], Some("Notepad.exe")),
                app(&["task manager", "taskmanager"], "taskmgr.exe", &[], Some("Taskmgr.exe")),
                app(&["einstellungen", "windows einstellungen"], "ms-settings:", &[], None),
            ],
            game_aliases: BTreeMap::from([
                ("cs".into(), "Counter-Strike 2".into()),
                ("counter strike".into(), "Counter-Strike 2".into()),
            ]),
            steam_path: None,
            search_url: "https://www.google.com/search?q={q}".into(),
            confirm_timeout_secs: 30,
        }
    }
}

/// Panco-Datenordner (`%APPDATA%\Panco`).
pub fn data_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("Panco")
}

pub fn models_dir() -> PathBuf {
    data_dir().join("models")
}

/// Natürliche Stimmen (Piper-Laufzeit + Stimmdateien).
pub fn voices_dir() -> PathBuf {
    data_dir().join("stimmen")
}

/// Wake-Word-Merkmale (keine Audiodaten).
pub fn wakeword_path() -> PathBuf {
    data_dir().join("wakeword").join("panco.rpw")
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

impl Config {
    /// Lädt die Konfig. Gibt bei Fehlern Standardwerte plus Fehlermeldung zurück.
    pub fn load_or_create(path: &Path) -> (Config, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(raw) => match serde_json::from_str(&raw) {
                Ok(cfg) => (cfg, None),
                Err(e) => (Config::default(), Some(format!("config.json ist fehlerhaft ({e}), nutze Standardwerte"))),
            },
            Err(_) => {
                let cfg = Config::default();
                let err = cfg.save(path).err().map(|e| format!("config.json nicht schreibbar: {e}"));
                (cfg, err)
            }
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).expect("Config ist serialisierbar");
        std::fs::write(path, json)
    }
}

/// Ersetzt `%VAR%` durch Umgebungsvariablen (unbekannte bleiben stehen).
pub fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => out.push_str(&rest[start..start + end + 2]),
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_env_vars() {
        std::env::set_var("PANCO_TEST_VAR", "C:\\X");
        assert_eq!(expand_env("%PANCO_TEST_VAR%\\a.exe"), "C:\\X\\a.exe");
        assert_eq!(expand_env("%NOPE_PANCO%\\a"), "%NOPE_PANCO%\\a");
        assert_eq!(expand_env("100%"), "100%");
    }

    #[test]
    fn partial_config_gets_defaults() {
        let cfg: Config = serde_json::from_str(r#"{"search_url":"https://duckduckgo.com/?q={q}"}"#).unwrap();
        assert_eq!(cfg.hotkeys.overlay, "Ctrl+Space");
        assert!(cfg.search_url.contains("duckduckgo"));
    }
}

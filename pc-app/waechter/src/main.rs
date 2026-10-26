//! panco-waechter: winziger Absturz-Wächter (nur Standardbibliothek, keine Fenster, kein Audio).
//! Startet Panco und **wartet blockierend auf dessen Ende** – kein Polling, ~0 % CPU, ~1 MB RAM.
//!
//! Aufruf (macht panco.exe selbst): `panco-waechter --log <absturz.log> -- <panco.exe> --app [...]`
//!
//! - Normales Beenden (Tray → Beenden, Code 0) → Wächter endet auch.
//! - Bewusst beendet (Task-Manager „Task beenden“ = Code 1, unter Linux SIGTERM/SIGKILL) → kein Neustart.
//! - Absturz (Panik, Zugriffsverletzung …) → Neustart nach kurzer Pause; bei wiederholten Abstürzen
//!   werden die Pausen länger, nach 4 Abstürzen in 10 Minuten gibt der Wächter auf (statt Endlosschleife).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const WINDOW: Duration = Duration::from_secs(600);
const MAX_CRASHES: usize = 4;

/// Wie ist Panco zu Ende gegangen?
#[derive(Debug, PartialEq, Eq)]
enum Ending {
    Normal,
    /// Von außen beendet (Task-Manager, kill) – das war Absicht.
    Stopped,
    Crashed(String),
}

fn classify(status: &ExitStatus) -> Ending {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            // 2 = SIGINT, 9 = SIGKILL, 15 = SIGTERM
            return if matches!(sig, 2 | 9 | 15) { Ending::Stopped } else { Ending::Crashed(format!("Signal {sig}")) };
        }
    }
    match status.code() {
        Some(0) => Ending::Normal,
        Some(1) => Ending::Stopped,
        Some(c) => Ending::Crashed(format!("Code 0x{:08X}", c as u32)),
        None => Ending::Stopped,
    }
}

/// Pause vor dem nächsten Start; `None` = aufgeben.
fn pause_after(recent_crashes: usize) -> Option<Duration> {
    match recent_crashes {
        0 | 1 => Some(Duration::from_secs(1)),
        2 => Some(Duration::from_secs(5)),
        3 => Some(Duration::from_secs(30)),
        _ => None,
    }
}

/// Unix-Sekunden → "2026-10-26T17:29:05Z" (Kalender nach Howard Hinnant, ohne Zusatzbibliothek).
fn iso(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let t = secs % 86_400;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", t / 3600, t % 3600 / 60, t % 60)
}

fn log_to(file: &Path, text: &str) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let _ = writeln!(f, "{} Wächter – {text}", iso(secs));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(l), Some(sep)) = (args.iter().position(|a| a == "--log"), args.iter().position(|a| a == "--")) else {
        eprintln!("Aufruf: panco-waechter --log <datei> -- <programm> [argumente]");
        std::process::exit(2);
    };
    let (Some(file), Some(exe)) = (args.get(l + 1).map(PathBuf::from), args.get(sep + 1)) else {
        std::process::exit(2);
    };
    run(Path::new(exe), &args[sep + 2..], |t| log_to(&file, t));
}

/// Läuft als Wächter, bis Panco normal beendet wird. Kehrt nicht zurück.
fn run(exe: &Path, app_args: &[String], log: impl Fn(&str)) -> ! {
    let mut crashes: Vec<Instant> = Vec::new();
    loop {
        // `status()` wartet im Betriebssystem auf das Prozessende (WaitForSingleObject / waitpid).
        let status = Command::new(exe).args(app_args).status();
        let ending = match status {
            Ok(s) => classify(&s),
            Err(e) => {
                log(&format!("Panco ließ sich nicht starten: {e}"));
                std::process::exit(1);
            }
        };
        match ending {
            Ending::Normal => std::process::exit(0),
            Ending::Stopped => {
                log("Panco wurde von außen beendet – kein Neustart.");
                std::process::exit(0);
            }
            Ending::Crashed(why) => {
                crashes.retain(|t| t.elapsed() < WINDOW);
                crashes.push(Instant::now());
                match pause_after(crashes.len()) {
                    Some(pause) => {
                        log(&format!("Panco unerwartet beendet ({why}) – Neustart in {} s.", pause.as_secs()));
                        std::thread::sleep(pause);
                    }
                    None => {
                        log(&format!(
                            "Wächter: {MAX_CRASHES} Abstürze in 10 Minuten ({why}) – kein weiterer Neustart. Bitte absturz.log an Claude schicken."
                        ));
                        std::process::exit(1);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_time() {
        assert_eq!(iso(1_793_035_745), "2026-10-26T17:29:05Z");
        assert_eq!(iso(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn backoff_and_give_up() {
        assert_eq!(pause_after(1), Some(Duration::from_secs(1)));
        assert_eq!(pause_after(3), Some(Duration::from_secs(30)));
        assert_eq!(pause_after(MAX_CRASHES), None);
    }

    #[cfg(unix)]
    #[test]
    fn exit_kinds() {
        let run = |cmd: &str| Command::new("sh").args(["-c", cmd]).status().unwrap();
        assert_eq!(classify(&run("exit 0")), Ending::Normal);
        assert_eq!(classify(&run("exit 1")), Ending::Stopped);
        assert!(matches!(classify(&run("kill -SEGV $$")), Ending::Crashed(_)));
        assert_eq!(classify(&run("kill -TERM $$")), Ending::Stopped);
        assert!(matches!(classify(&run("exit 3")), Ending::Crashed(_)));
    }
}

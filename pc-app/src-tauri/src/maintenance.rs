//! Wartung im Hintergrund: Papierkorb-Frist und tägliches Backup.
//! Ein schlafender Thread, der alle 6 Stunden (oder auf Anstoß) kurz prüft – kein Polling.

use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};

const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);

pub struct Maintenance(Mutex<Sender<()>>);

pub fn start(app: &AppHandle) {
    let (tx, rx) = channel::<()>();
    let engine = crate::engine(app);
    std::thread::Builder::new()
        .name("panco-wartung".into())
        .spawn(move || loop {
            // Vor der Ersteinrichtung (Ordner noch nicht gewählt) nichts anlegen.
            if engine.config().setup.done {
                for line in engine.maintenance(false) {
                    eprintln!("[panco] {line}");
                }
            }
            match rx.recv_timeout(CHECK_EVERY) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        })
        .expect("Wartungs-Thread startet");
    app.manage(Maintenance(Mutex::new(tx)));
}

/// Sofort prüfen (z. B. nach der Ersteinrichtung).
pub fn poke(app: &AppHandle) {
    if let Some(m) = app.try_state::<Maintenance>() {
        let _ = m.0.lock().unwrap().send(());
    }
}

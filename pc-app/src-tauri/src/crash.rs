//! Absturzprotokoll: Beendet sich Panco unerwartet, steht in `logs/absturz.log`, wo und warum –
//! Rust-Fehler (Panik) und unter Windows auch Abstürze in nativem Code (Ausnahmecode + Adresse).
//! Keine Audio- oder Inhaltsdaten, nur technische Angaben; nichts verlässt den PC.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

static FILE: OnceLock<PathBuf> = OnceLock::new();

/// Zeile ins Absturzprotokoll (auch für Meldungen des Wächters).
pub fn note(text: &str) {
    write(text);
}

fn write(text: &str) {
    let Some(path) = FILE.get() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let now = panco_core::log::ActionLog::now();
        let _ = writeln!(f, "{now} Panco {} – {text}", env!("CARGO_PKG_VERSION"));
    }
}

pub fn install(dir: PathBuf) {
    let _ = FILE.set(dir.join("absturz.log"));
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        write(&format!("Panik im Thread „{thread}“: {info}"));
        default(info);
    }));
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter;
        SetUnhandledExceptionFilter(Some(on_exception));
    }
}

#[cfg(windows)]
unsafe extern "system" fn on_exception(
    info: *const windows::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS,
) -> i32 {
    let (code, addr) = unsafe {
        info.as_ref()
            .and_then(|i| i.ExceptionRecord.as_ref())
            .map(|r| (r.ExceptionCode.0 as u32, r.ExceptionAddress as usize))
            .unwrap_or_default()
    };
    write(&format!("Absturz: Ausnahme 0x{code:08X} an Adresse 0x{addr:X}"));
    0 // EXCEPTION_CONTINUE_SEARCH: Windows beendet den Prozess wie gewohnt
}

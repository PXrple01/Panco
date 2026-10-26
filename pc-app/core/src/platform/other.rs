//! Nicht-Windows (nur für Entwicklung): Öffnen per xdg-open/open, Rest nicht unterstützt.

use super::*;
use std::process::{Command, Stdio};

pub struct OtherPlatform;

impl Platform for OtherPlatform {
    fn open(&self, target: &str, args: &[String]) -> PResult<()> {
        // Programm mit Argumenten direkt starten (kein Shell-Interpreter), sonst xdg-open.
        let mut cmd = if args.is_empty() {
            let mut c = Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" });
            c.arg(target);
            c
        } else {
            let mut c = Command::new(target);
            c.args(args);
            c
        };
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        cmd.spawn().map(|_| ()).map_err(|e| PlatformError::Failed(e.to_string()))
    }
    fn volume(&self) -> PResult<u8> {
        Err(PlatformError::Unsupported)
    }
    fn set_volume(&self, _: u8) -> PResult<()> {
        Err(PlatformError::Unsupported)
    }
    fn muted(&self) -> PResult<bool> {
        Err(PlatformError::Unsupported)
    }
    fn set_muted(&self, _: bool) -> PResult<()> {
        Err(PlatformError::Unsupported)
    }
    fn media_key(&self, _: MediaKey) -> PResult<()> {
        Err(PlatformError::Unsupported)
    }
    fn screenshot(&self) -> PResult<()> {
        Err(PlatformError::Unsupported)
    }
    fn lock(&self) -> PResult<()> {
        Err(PlatformError::Unsupported)
    }
    fn power(&self, _: PowerMode) -> PResult<()> {
        Err(PlatformError::Unsupported)
    }
    fn windows(&self) -> PResult<Vec<WindowInfo>> {
        Ok(Vec::new())
    }
    fn focus_window(&self, _: u64) -> PResult<()> {
        Err(PlatformError::Unsupported)
    }
    fn close_process(&self, _: &str) -> PResult<usize> {
        Err(PlatformError::Unsupported)
    }
}

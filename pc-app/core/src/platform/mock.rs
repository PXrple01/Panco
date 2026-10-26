//! Test-Plattform: führt nichts aus, merkt sich jeden Aufruf.

use super::*;
use std::sync::Mutex;

#[derive(Default)]
pub struct MockPlatform {
    pub calls: Mutex<Vec<String>>,
    pub volume: Mutex<u8>,
    pub muted: Mutex<bool>,
    pub windows: Mutex<Vec<WindowInfo>>,
}

impl MockPlatform {
    fn record(&self, s: String) {
        self.calls.lock().unwrap().push(s);
    }

    pub fn take_calls(&self) -> Vec<String> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }
}

impl Platform for MockPlatform {
    fn open(&self, target: &str, args: &[String]) -> PResult<()> {
        self.record(if args.is_empty() { format!("open {target}") } else { format!("open {target} {args:?}") });
        Ok(())
    }
    fn volume(&self) -> PResult<u8> {
        Ok(*self.volume.lock().unwrap())
    }
    fn set_volume(&self, level: u8) -> PResult<()> {
        *self.volume.lock().unwrap() = level;
        self.record(format!("volume {level}"));
        Ok(())
    }
    fn muted(&self) -> PResult<bool> {
        Ok(*self.muted.lock().unwrap())
    }
    fn set_muted(&self, mute: bool) -> PResult<()> {
        *self.muted.lock().unwrap() = mute;
        self.record(format!("mute {mute}"));
        Ok(())
    }
    fn media_key(&self, key: MediaKey) -> PResult<()> {
        self.record(format!("media {key:?}"));
        Ok(())
    }
    fn screenshot(&self) -> PResult<()> {
        self.record("screenshot".into());
        Ok(())
    }
    fn lock(&self) -> PResult<()> {
        self.record("lock".into());
        Ok(())
    }
    fn power(&self, mode: PowerMode) -> PResult<()> {
        self.record(format!("power {mode:?}"));
        Ok(())
    }
    fn windows(&self) -> PResult<Vec<WindowInfo>> {
        Ok(self.windows.lock().unwrap().clone())
    }
    fn focus_window(&self, id: u64) -> PResult<()> {
        self.record(format!("focus {id}"));
        Ok(())
    }
    fn close_process(&self, process: &str) -> PResult<usize> {
        self.record(format!("close {process}"));
        Ok(1)
    }
    fn process_stats(&self) -> PResult<ProcStats> {
        Ok(ProcStats::default())
    }
    fn reveal(&self, path: &std::path::Path) -> PResult<()> {
        self.record(format!("reveal {}", path.display()));
        Ok(())
    }
    /// Test-"Verschlüsselung": umkehrbar, aber nicht lesbar (reicht für Tests).
    fn protect(&self, data: &[u8]) -> PResult<Vec<u8>> {
        let mut out = b"MOCK".to_vec();
        out.extend(data.iter().map(|b| b ^ 0x5A));
        Ok(out)
    }
    fn unprotect(&self, data: &[u8]) -> PResult<Vec<u8>> {
        let body = data.strip_prefix(b"MOCK").ok_or_else(|| PlatformError::Failed("kein Mock-Blob".into()))?;
        Ok(body.iter().map(|b| b ^ 0x5A).collect())
    }
    fn utc_offset_secs(&self) -> i32 {
        3600
    }
}

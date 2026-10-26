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
}

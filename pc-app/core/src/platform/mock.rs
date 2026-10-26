//! Test-Plattform: führt nichts aus, merkt sich jeden Aufruf.

use super::*;
use std::sync::Mutex;

#[derive(Default)]
pub struct MockPlatform {
    pub calls: Mutex<Vec<String>>,
    pub volume: Mutex<u8>,
    pub muted: Mutex<bool>,
    pub windows: Mutex<Vec<WindowInfo>>,
    pub apps: Mutex<Vec<AppVolume>>,
    pub procs: Mutex<Vec<String>>,
    /// Vorbereitete HTTP-Antworten (der Reihe nach) und gestellte Anfragen
    pub http_replies: Mutex<std::collections::VecDeque<(u16, String)>>,
    pub http_requests: Mutex<Vec<HttpRequest>>,
    pub secrets: Mutex<std::collections::HashMap<String, String>>,
    /// Bildschirm: erkannter Text (OCR) und ob überhaupt fotografiert werden kann
    pub screen_text: Mutex<Option<String>>,
    pub clipboard: Mutex<Option<String>>,
    pub active: Mutex<Option<WindowInfo>>,
    /// Vorbereitete PowerShell-Ergebnisse; ausgeführte Skripte (Inhalt) werden gemerkt
    pub ps_results: Mutex<std::collections::VecDeque<ScriptOutput>>,
    pub ps_scripts: Mutex<Vec<String>>,
    pub restore_points: Mutex<Vec<String>>,
    pub restore_fails: Mutex<bool>,
    pub sandbox: Mutex<Option<String>>,
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
    fn app_volumes(&self) -> PResult<Vec<AppVolume>> {
        Ok(self.apps.lock().unwrap().clone())
    }
    fn set_app_volume(&self, process: &str, level: Option<u8>, mute: Option<bool>) -> PResult<usize> {
        let mut apps = self.apps.lock().unwrap();
        let mut n = 0;
        for a in apps.iter_mut().filter(|a| a.process.eq_ignore_ascii_case(process)) {
            if let Some(l) = level {
                a.level = l;
            }
            if let Some(m) = mute {
                a.muted = m;
            }
            n += 1;
        }
        drop(apps);
        self.record(format!("app-volume {process} {level:?} {mute:?}"));
        Ok(n)
    }
    fn processes(&self) -> PResult<Vec<String>> {
        Ok(self.procs.lock().unwrap().clone())
    }
    fn http(&self, req: &HttpRequest) -> PResult<(u16, String)> {
        self.http_requests.lock().unwrap().push(req.clone());
        self.http_replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| PlatformError::Failed("keine Antwort vorbereitet".into()))
    }
    fn secret_get(&self, key: &str) -> PResult<Option<String>> {
        Ok(self.secrets.lock().unwrap().get(key).cloned())
    }
    fn secret_set(&self, key: &str, value: Option<&str>) -> PResult<()> {
        let mut s = self.secrets.lock().unwrap();
        match value {
            Some(v) => s.insert(key.into(), v.into()),
            None => s.remove(key),
        };
        Ok(())
    }
    /// Vorbereitete Antwort zeilenweise ausliefern (wie ein SSE-Stream).
    fn http_stream(&self, req: &HttpRequest, on_line: &mut dyn FnMut(&str) -> bool) -> PResult<u16> {
        let (status, body) = self.http(req)?;
        for line in body.lines() {
            if !on_line(line) {
                break;
            }
        }
        Ok(status)
    }
    fn capture(&self, target: CaptureTarget) -> PResult<Image> {
        self.record(format!("capture {target:?}"));
        if self.screen_text.lock().unwrap().is_none() {
            return Err(PlatformError::Unsupported);
        }
        Ok(Image { width: 4, height: 2, rgba: vec![200; 4 * 2 * 4] })
    }
    fn ocr(&self, _image: &Image) -> PResult<String> {
        self.screen_text.lock().unwrap().clone().ok_or(PlatformError::Unsupported)
    }
    fn clipboard_text(&self) -> PResult<Option<String>> {
        Ok(self.clipboard.lock().unwrap().clone())
    }
    fn active_window(&self) -> PResult<Option<WindowInfo>> {
        Ok(self.active.lock().unwrap().clone())
    }
    fn powershell(
        &self,
        script: &std::path::Path,
        _cwd: &std::path::Path,
        _t: std::time::Duration,
    ) -> PResult<ScriptOutput> {
        self.ps_scripts.lock().unwrap().push(std::fs::read_to_string(script).unwrap_or_default());
        Ok(self.ps_results.lock().unwrap().pop_front().unwrap_or_default())
    }
    fn restore_point(&self, description: &str) -> PResult<()> {
        if *self.restore_fails.lock().unwrap() {
            return Err(PlatformError::Failed("Adminrechte nötig".into()));
        }
        self.restore_points.lock().unwrap().push(description.into());
        Ok(())
    }
    fn sandbox_available(&self) -> bool {
        self.sandbox.lock().unwrap().is_some()
    }
    fn sandbox_run(&self, script: &str, _t: std::time::Duration) -> PResult<String> {
        self.record(format!("sandbox {} Zeichen", script.len()));
        self.sandbox.lock().unwrap().clone().ok_or(PlatformError::Unsupported)
    }
    fn random_bytes(&self, n: usize) -> PResult<Vec<u8>> {
        Ok((0..n).map(|i| (i * 37 % 256) as u8).collect())
    }
}

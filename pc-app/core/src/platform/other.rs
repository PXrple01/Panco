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
    fn process_stats(&self) -> PResult<ProcStats> {
        linux_stats().ok_or(PlatformError::Unsupported)
    }
    fn reveal(&self, path: &std::path::Path) -> PResult<()> {
        let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
        self.open(&dir.to_string_lossy(), &[])
    }
    fn protect(&self, _: &[u8]) -> PResult<Vec<u8>> {
        Err(PlatformError::Unsupported)
    }
    fn unprotect(&self, _: &[u8]) -> PResult<Vec<u8>> {
        Err(PlatformError::Unsupported)
    }
    fn utc_offset_secs(&self) -> i32 {
        0
    }
    fn app_volumes(&self) -> PResult<Vec<AppVolume>> {
        Err(PlatformError::Unsupported)
    }
    fn set_app_volume(&self, _: &str, _: Option<u8>, _: Option<bool>) -> PResult<usize> {
        Err(PlatformError::Unsupported)
    }
    /// /proc/*/comm (Linux, nur für die Entwicklung).
    fn processes(&self) -> PResult<Vec<String>> {
        let dir = std::fs::read_dir("/proc").map_err(|_| PlatformError::Unsupported)?;
        Ok(dir
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()))
            .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
            .map(|s| s.trim().to_string())
            .collect())
    }
    /// Kein Tresor unter Linux (nur Entwicklung) – Geheimnisse werden nicht gespeichert.
    fn secret_get(&self, _: &str) -> PResult<Option<String>> {
        Ok(None)
    }
    fn secret_set(&self, _: &str, _: Option<&str>) -> PResult<()> {
        Err(PlatformError::Unsupported)
    }
    fn random_bytes(&self, n: usize) -> PResult<Vec<u8>> {
        use std::io::Read;
        let mut buf = vec![0u8; n];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut buf))
            .map_err(|e| PlatformError::Failed(e.to_string()))?;
        Ok(buf)
    }
}

/// Linux (Entwicklung): /proc lesen. USS = privater Speicher.
fn linux_stats() -> Option<ProcStats> {
    let me = std::process::id();
    let mut parents: Vec<(u32, u32, String, f64)> = Vec::new(); // pid, ppid, name, cpu
    let ticks = 100.0; // CLK_TCK ist unter Linux praktisch immer 100
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
        let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else { continue };
        let name = stat[open + 1..close].to_string();
        let f: Vec<&str> = stat[close + 2..].split(' ').collect();
        let ppid = f.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        let cpu = f.get(11).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0)
            + f.get(12).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        parents.push((pid, ppid, name, cpu / ticks));
    }
    let uss = |pid: u32| -> f64 {
        std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup"))
            .map(|s| {
                s.lines()
                    .filter(|l| l.starts_with("Private_"))
                    .filter_map(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
                    .sum::<f64>()
                    / 1024.0
            })
            .unwrap_or(0.0)
    };
    let mut stats = ProcStats { own_mb: uss(me), ..Default::default() };
    let mut tree = vec![me];
    let mut i = 0;
    while i < tree.len() {
        let parent = tree[i];
        for (pid, ppid, name, cpu) in &parents {
            if *ppid == parent && !tree.contains(pid) {
                tree.push(*pid);
                stats.children_mb += uss(*pid);
                stats.children.push(name.clone());
            }
            if *pid == parent {
                stats.cpu_seconds += cpu;
            }
        }
        i += 1;
    }
    Some(stats)
}

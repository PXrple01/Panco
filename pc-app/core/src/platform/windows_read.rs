//! Windows: Bildschirm lesen (GDI + Windows-OCR), Zwischenablage, aktives Fenster und die
//! geprüfte Shell (PowerShell ausführen/parsen, Wiederherstellungspunkt, Windows-Sandbox).
//! Bilder bleiben im Speicher; Skripte liegen nur für die Dauer des Laufs auf der Platte.

use super::*;
use ::windows::core::PCWSTR;
use ::windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use ::windows::Media::Ocr::OcrEngine;
use ::windows::Storage::Streams::DataWriter;
use ::windows::Win32::Foundation::{HGLOBAL, HWND, RECT};
use ::windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use ::windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, GetMonitorInfoW,
    MonitorFromWindow, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS,
    MONITORINFO, MONITOR_DEFAULTTONEAREST, SRCCOPY,
};
use ::windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
};
use ::windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use ::windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CF_UNICODETEXT: u32 = 13;

fn fail(e: impl std::fmt::Display) -> PlatformError {
    PlatformError::Failed(e.to_string())
}

/// Bereich des Vordergrundfensters bzw. seines Bildschirms (physische Pixel).
fn capture_rect(target: CaptureTarget) -> PResult<RECT> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return Err(fail("kein Fenster im Vordergrund"));
    }
    match target {
        CaptureTarget::ActiveWindow => {
            let mut r = RECT::default();
            unsafe {
                DwmGetWindowAttribute(
                    hwnd,
                    DWMWA_EXTENDED_FRAME_BOUNDS,
                    &mut r as *mut RECT as *mut _,
                    std::mem::size_of::<RECT>() as u32,
                )
                .map_err(fail)?;
            }
            Ok(r)
        }
        CaptureTarget::Screen => {
            let mon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !unsafe { GetMonitorInfoW(mon, &mut info) }.as_bool() {
                return Err(fail("Bildschirm nicht gefunden"));
            }
            Ok(info.rcMonitor)
        }
    }
}

/// Bildschirmbereich per GDI kopieren → BGRA (von oben nach unten).
fn grab(r: RECT) -> PResult<(u32, u32, Vec<u8>)> {
    let (w, h) = ((r.right - r.left).clamp(1, 8192), (r.bottom - r.top).clamp(1, 8192));
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        let ok = BitBlt(mem, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY | CAPTUREBLT);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // von oben nach unten
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(mem, bmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        ok.map_err(fail)?;
        if lines == 0 {
            return Err(fail("Bildschirm konnte nicht gelesen werden"));
        }
        Ok((w as u32, h as u32, buf))
    }
}

pub(super) fn capture(target: CaptureTarget) -> PResult<Image> {
    let (width, height, mut px) = grab(capture_rect(target)?)?;
    for p in px.chunks_exact_mut(4) {
        p.swap(0, 2); // BGRA → RGBA
        p[3] = 255;
    }
    Ok(Image { width, height, rgba: px })
}

/// Windows-Texterkennung (Sprache aus dem Benutzerprofil), komplett lokal.
pub(super) fn ocr(img: &Image) -> PResult<String> {
    super::windows::com_init();
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|_| fail("keine Texterkennung für deine Sprache installiert"))?;
    let max = OcrEngine::MaxImageDimension().unwrap_or(2600);
    let img = crate::image::downscale(img, max);
    let mut bgra = img.rgba.clone();
    for p in bgra.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    let writer = DataWriter::new().map_err(fail)?;
    writer.WriteBytes(&bgra).map_err(fail)?;
    let buffer = writer.DetachBuffer().map_err(fail)?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        img.width as i32,
        img.height as i32,
        BitmapAlphaMode::Premultiplied,
    )
    .map_err(fail)?;
    let result = engine.RecognizeAsync(&bitmap).map_err(fail)?.join().map_err(fail)?;
    let lines = result.Lines().map_err(fail)?;
    let mut out = Vec::new();
    for i in 0..lines.Size().unwrap_or(0) {
        if let Ok(line) = lines.GetAt(i) {
            out.push(line.Text().map(|t| t.to_string()).unwrap_or_default());
        }
    }
    Ok(out.join("\n"))
}

pub(super) fn clipboard_text() -> PResult<Option<String>> {
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
            return Ok(None);
        }
        // Ein anderes Programm kann die Zwischenablage kurz belegen.
        let mut opened = false;
        for _ in 0..5 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !opened {
            return Err(fail("Zwischenablage ist belegt"));
        }
        let result = (|| {
            let h = GetClipboardData(CF_UNICODETEXT).map_err(fail)?;
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if p.is_null() {
                return Ok(None);
            }
            let text = PCWSTR(p).to_string().unwrap_or_default();
            let _ = GlobalUnlock(g);
            Ok(Some(text))
        })();
        let _ = CloseClipboard();
        result
    }
}

pub(super) fn active_window() -> PResult<Option<WindowInfo>> {
    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.0.is_null() {
            return Ok(None);
        }
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut buf) as usize;
        let title = String::from_utf16_lossy(&buf[..n]);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let process = super::windows::process_name(pid).unwrap_or_default();
        Ok(Some(WindowInfo { id: hwnd.0 as u64, pid, title, process }))
    }
}

// ---------------------------------------------------------------- PowerShell

fn powershell_exe() -> std::path::PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    Path::new(&root).join("System32\\WindowsPowerShell\\v1.0\\powershell.exe")
}

/// Pfad als PowerShell-Text in einfachen Anführungszeichen.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Läuft mit Zeitlimit; Ausgaben werden parallel gelesen (höchstens 1 MB je Strom).
fn run(mut cmd: Command, timeout: Duration) -> PResult<ScriptOutput> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).creation_flags(CREATE_NO_WINDOW);
    let mut child = cmd.spawn().map_err(|e| fail(format!("PowerShell startet nicht: {e}")))?;
    let read = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(r) = r {
                let _ = r.take(1 << 20).read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let out = read(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let err = read(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    // Nur solange ein Skript läuft (höchstens `timeout`) – kein Dauer-Polling.
    let start = Instant::now();
    let (code, timed_out) = loop {
        if let Some(status) = child.try_wait().map_err(fail)? {
            break (status.code().unwrap_or(-1), false);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break (-1, true);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    Ok(ScriptOutput { code, stdout: out.join().unwrap_or_default(), stderr: err.join().unwrap_or_default(), timed_out })
}

fn base_command() -> Command {
    let mut c = Command::new(powershell_exe());
    c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass"]);
    c
}

pub(super) fn powershell(script: &Path, cwd: &Path, timeout: Duration) -> PResult<ScriptOutput> {
    let mut c = base_command();
    // UTF-8-Ausgabe, dann genau diese Skriptdatei (fester Pfad) – nichts sonst.
    c.arg("-Command").arg(format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8; & {}; if ($?) {{ exit 0 }} else {{ exit 1 }}",
        ps_quote(&script.to_string_lossy())
    ));
    c.current_dir(cwd);
    run(c, timeout)
}

fn temp_dir() -> PResult<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("panco-ps-{}-{}", std::process::id(), crate::memory::new_id()));
    std::fs::create_dir_all(&dir).map_err(fail)?;
    Ok(dir)
}

pub(super) fn powershell_parse(script: &str) -> PResult<ParsedScript> {
    let dir = temp_dir()?;
    let user = dir.join("skript.ps1");
    let parser = dir.join("parse.ps1");
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(script.as_bytes());
    std::fs::write(&user, bytes).map_err(fail)?;
    std::fs::write(&parser, crate::shell::PARSE_SCRIPT).map_err(fail)?;
    let mut c = base_command();
    c.arg("-File").arg(&parser).arg(&user);
    let out = run(c, Duration::from_secs(30));
    let _ = std::fs::remove_dir_all(&dir);
    let out = out?;
    let v: serde_json::Value = serde_json::from_str(out.stdout.trim())
        .map_err(|_| fail(format!("Parser-Ausgabe unlesbar: {}", out.stderr.lines().next().unwrap_or(""))))?;
    // PowerShell schreibt einzelne Werte manchmal ohne Liste
    let commands = match &v["commands"] {
        serde_json::Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        serde_json::Value::String(s) => vec![s.clone()],
        _ => Vec::new(),
    };
    Ok(ParsedScript {
        errors: v["errors"].as_u64().unwrap_or(1) as usize,
        commands,
        dynamic: v["dynamic"].as_u64().unwrap_or(0) as usize,
    })
}

pub(super) fn restore_point(description: &str) -> PResult<()> {
    let desc: String = description.chars().filter(|c| !c.is_control()).take(60).collect();
    let mut c = base_command();
    c.arg("-Command").arg(format!(
        "Checkpoint-Computer -Description {} -RestorePointType MODIFY_SETTINGS -ErrorAction Stop",
        ps_quote(&desc)
    ));
    let out = run(c, Duration::from_secs(180))?;
    if out.code == 0 {
        return Ok(());
    }
    let msg = out.stderr.to_lowercase();
    Err(fail(if msg.contains("zugriff") || msg.contains("access") || msg.contains("administrat") {
        "dafür braucht Panco Adminrechte".to_string()
    } else if msg.contains("deaktiviert") || msg.contains("disabled") {
        "der Computerschutz ist ausgeschaltet".to_string()
    } else {
        out.stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("unbekannter Fehler").trim().to_string()
    }))
}

fn sandbox_exe() -> std::path::PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    Path::new(&root).join("System32\\WindowsSandbox.exe")
}

pub(super) fn sandbox_available() -> bool {
    sandbox_exe().is_file()
}

/// Läuft in der Sandbox: Dateien vorher/nachher vergleichen, Skript ausführen, Bericht schreiben,
/// Sandbox herunterfahren.
const SANDBOX_RUNNER: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$roots = @("$env:USERPROFILE", 'C:\ProgramData', 'C:\Program Files', 'C:\Program Files (x86)')
function Snap { $h = @{}; Get-ChildItem $roots -Recurse -Force -File | ForEach-Object { $h[$_.FullName] = $_.LastWriteTimeUtc.Ticks }; $h }
function Keys { $k = @{}; foreach ($r in 'HKCU:\Software', 'HKLM:\SOFTWARE') { Get-ChildItem $r -Recurse -Depth 1 | ForEach-Object { $k[$_.Name] = 1 } }; $k }
$before = Snap; $regBefore = Keys
$log = & powershell -NoProfile -ExecutionPolicy Bypass -File C:\Panco\skript.ps1 *>&1 | Out-String
$after = Snap; $regAfter = Keys
$neu = @($after.Keys | Where-Object { -not $before.ContainsKey($_) -and $_ -notlike 'C:\Panco\*' })
$geaendert = @($after.Keys | Where-Object { $before.ContainsKey($_) -and $before[$_] -ne $after[$_] })
$weg = @($before.Keys | Where-Object { -not $after.ContainsKey($_) })
$regNeu = @($regAfter.Keys | Where-Object { -not $regBefore.ContainsKey($_) })
$r = @("Neue Dateien: $($neu.Count), geänderte: $($geaendert.Count), gelöschte: $($weg.Count), neue Registry-Schlüssel: $($regNeu.Count)")
$r += ($neu | Select-Object -First 40 | ForEach-Object { "+ $_" })
$r += ($geaendert | Select-Object -First 40 | ForEach-Object { "~ $_" })
$r += ($weg | Select-Object -First 40 | ForEach-Object { "- $_" })
$r += ($regNeu | Select-Object -First 20 | ForEach-Object { "+ Registry: $_" })
$r += '--- Ausgabe des Skripts ---'
$r += $log
Set-Content -Path C:\Panco\bericht.tmp -Value $r -Encoding UTF8
Rename-Item C:\Panco\bericht.tmp bericht.txt
shutdown /s /t 0
"#;

pub(super) fn sandbox_run(script: &str, timeout: Duration) -> PResult<String> {
    let dir = temp_dir()?;
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(script.as_bytes());
    std::fs::write(dir.join("skript.ps1"), bytes).map_err(fail)?;
    let mut runner = vec![0xEF, 0xBB, 0xBF];
    runner.extend_from_slice(SANDBOX_RUNNER.as_bytes());
    std::fs::write(dir.join("runner.ps1"), runner).map_err(fail)?;
    let wsb = dir.join("panco.wsb");
    let host = dir.to_string_lossy().replace('&', "&amp;").replace('<', "&lt;");
    std::fs::write(
        &wsb,
        format!(
            "<Configuration><MappedFolders><MappedFolder><HostFolder>{host}</HostFolder>\
<SandboxFolder>C:\\Panco</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder></MappedFolders>\
<LogonCommand><Command>powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File C:\\Panco\\runner.ps1</Command></LogonCommand>\
<ClipboardRedirection>Disable</ClipboardRedirection><PrinterRedirection>Disable</PrinterRedirection></Configuration>"
        ),
    )
    .map_err(fail)?;
    Command::new(sandbox_exe()).arg(&wsb).spawn().map_err(|e| fail(format!("Windows-Sandbox startet nicht: {e}")))?;
    // Nur während des Testlaufs: auf den Bericht warten (höchstens `timeout`).
    let report = dir.join("bericht.txt");
    let start = Instant::now();
    let result = loop {
        if report.is_file() {
            break std::fs::read_to_string(&report).map_err(fail);
        }
        if start.elapsed() > timeout {
            break Err(fail("der Testlauf hat zu lange gedauert"));
        }
        std::thread::sleep(Duration::from_secs(1));
    };
    let _ = std::fs::remove_dir_all(&dir);
    result.map(|r| r.trim_start_matches('\u{feff}').to_string())
}

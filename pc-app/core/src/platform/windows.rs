//! Windows-Implementierung über Win32/COM (windows-rs). Keine Shell, keine Hilfsprozesse.

use super::*;
use ::windows::core::{w, BOOL, HSTRING, PCWSTR, PWSTR};
use ::windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LUID, WPARAM};
use ::windows::Win32::Foundation::{LocalFree, HLOCAL};
use ::windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use ::windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use ::windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use ::windows::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED, SE_SHUTDOWN_NAME,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use ::windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
use ::windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use ::windows::Win32::System::Power::SetSuspendState;
use ::windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
};
use ::windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
use ::windows::Win32::System::Shutdown::{
    ExitWindowsEx, LockWorkStation, EWX_POWEROFF, EWX_REBOOT, EWX_SHUTDOWN, SHTDN_REASON_FLAG_PLANNED,
    SHTDN_REASON_MAJOR_OTHER,
};
use ::windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetProcessTimes, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};
use ::windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_LWIN, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_MEDIA_STOP, VK_MENU, VK_SNAPSHOT,
};
use ::windows::Win32::UI::Shell::ShellExecuteW;
use ::windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindow, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, PostMessageW, SetForegroundWindow, ShowWindow, GWL_EXSTYLE, GW_OWNER, SW_RESTORE, SW_SHOWNORMAL,
    WM_CLOSE, WS_EX_TOOLWINDOW,
};

pub struct WindowsPlatform;

fn fail(e: impl std::fmt::Display) -> PlatformError {
    PlatformError::Failed(e.to_string())
}

/// COM pro Thread initialisieren (mehrfach aufrufen ist harmlos).
fn com_init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn endpoint_volume() -> PResult<IAudioEndpointVolume> {
    com_init();
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(fail)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).map_err(fail)?;
        device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).map_err(fail)
    }
}

fn key_input(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_EXTENDEDKEY;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

/// Drückt die Tasten nacheinander und lässt sie in umgekehrter Reihenfolge los.
fn press(keys: &[VIRTUAL_KEY]) -> PResult<()> {
    let mut inputs: Vec<INPUT> = keys.iter().map(|k| key_input(*k, false)).collect();
    inputs.extend(keys.iter().rev().map(|k| key_input(*k, true)));
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(fail("Tastendruck wurde blockiert"))
    }
}

fn quote_arg(a: &str) -> String {
    if a.is_empty() || a.contains([' ', '\t', '"']) {
        format!("\"{}\"", a.replace('"', "\\\""))
    } else {
        a.to_string()
    }
}

fn process_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(handle);
        ok.ok()?;
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit('\\').next().map(str::to_string)
    }
}

unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = &mut *(lparam.0 as *mut Vec<WindowInfo>);
    // Nur echte App-Fenster: sichtbar, mit Titel, nicht im Besitz eines anderen Fensters, kein Tool-Fenster.
    if !IsWindowVisible(hwnd).as_bool() || GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
        return true.into();
    }
    if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
        return true.into();
    }
    let len = GetWindowTextLengthW(hwnd);
    if len == 0 {
        return true.into();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let n = GetWindowTextW(hwnd, &mut buf);
    let title = String::from_utf16_lossy(&buf[..n as usize]);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    let process = process_name(pid).unwrap_or_default();
    out.push(WindowInfo { id: hwnd.0 as usize as u64, pid, title, process });
    true.into()
}

impl Platform for WindowsPlatform {
    fn open(&self, target: &str, args: &[String]) -> PResult<()> {
        let params = args.iter().map(|a| quote_arg(a)).collect::<Vec<_>>().join(" ");
        // Bei Programmpfaden das Programmverzeichnis als Arbeitsordner (z. B. für OBS nötig).
        let dir = if target.contains('\\') && target.to_lowercase().ends_with(".exe") {
            std::path::Path::new(target).parent().map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };
        let target_h = HSTRING::from(target);
        let params_h = HSTRING::from(params.as_str());
        let dir_h = dir.as_deref().map(HSTRING::from);
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &target_h,
                if params.is_empty() { PCWSTR::null() } else { PCWSTR(params_h.as_ptr()) },
                dir_h.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecute meldet Erfolg mit einem Wert > 32.
        if result.0 as usize > 32 {
            Ok(())
        } else {
            Err(fail(format!("konnte '{target}' nicht öffnen (Code {})", result.0 as usize)))
        }
    }

    fn volume(&self) -> PResult<u8> {
        let v = unsafe { endpoint_volume()?.GetMasterVolumeLevelScalar().map_err(fail)? };
        Ok((v * 100.0).round() as u8)
    }

    fn set_volume(&self, level: u8) -> PResult<()> {
        let ep = endpoint_volume()?;
        unsafe {
            ep.SetMasterVolumeLevelScalar(level.min(100) as f32 / 100.0, std::ptr::null()).map_err(fail)?;
            if level > 0 {
                ep.SetMute(false, std::ptr::null()).map_err(fail)?;
            }
        }
        Ok(())
    }

    fn muted(&self) -> PResult<bool> {
        Ok(unsafe { endpoint_volume()?.GetMute().map_err(fail)?.as_bool() })
    }

    fn set_muted(&self, mute: bool) -> PResult<()> {
        unsafe { endpoint_volume()?.SetMute(mute, std::ptr::null()).map_err(fail) }
    }

    fn media_key(&self, key: MediaKey) -> PResult<()> {
        press(&[match key {
            MediaKey::PlayPause => VK_MEDIA_PLAY_PAUSE,
            MediaKey::Next => VK_MEDIA_NEXT_TRACK,
            MediaKey::Previous => VK_MEDIA_PREV_TRACK,
            MediaKey::Stop => VK_MEDIA_STOP,
        }])
    }

    fn screenshot(&self) -> PResult<()> {
        // Win+Druck: Windows speichert selbst unter Bilder\Screenshots.
        press(&[VK_LWIN, VK_SNAPSHOT])
    }

    fn lock(&self) -> PResult<()> {
        unsafe { LockWorkStation().map_err(fail) }
    }

    fn power(&self, mode: PowerMode) -> PResult<()> {
        unsafe {
            if mode == PowerMode::Sleep {
                return if SetSuspendState(false, false, false) {
                    Ok(())
                } else {
                    Err(fail("Ruhemodus fehlgeschlagen"))
                };
            }
            // Herunterfahren braucht das Shutdown-Privileg im eigenen Token.
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token).map_err(fail)?;
            let mut luid = LUID::default();
            let lookup = LookupPrivilegeValueW(None, SE_SHUTDOWN_NAME, &mut luid);
            let privs = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }],
            };
            let adjust = lookup.and_then(|_| AdjustTokenPrivileges(token, false, Some(&privs), 0, None, None));
            let _ = CloseHandle(token);
            adjust.map_err(fail)?;
            let flags = match mode {
                PowerMode::Restart => EWX_REBOOT,
                _ => EWX_SHUTDOWN | EWX_POWEROFF,
            };
            ExitWindowsEx(flags, SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_FLAG_PLANNED).map_err(fail)
        }
    }

    fn windows(&self) -> PResult<Vec<WindowInfo>> {
        let mut list: Vec<WindowInfo> = Vec::new();
        unsafe {
            EnumWindows(Some(collect_window), LPARAM(&mut list as *mut _ as isize)).map_err(fail)?;
        }
        Ok(list)
    }

    fn focus_window(&self, id: u64) -> PResult<()> {
        let hwnd = HWND(id as usize as *mut core::ffi::c_void);
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            // Windows erlaubt Fokuswechsel nur nach einer Eingabe – ein Alt-Tipp schaltet das frei.
            let _ = press(&[VK_MENU]);
            if SetForegroundWindow(hwnd).as_bool() {
                Ok(())
            } else {
                Err(fail("Fenster ließ sich nicht nach vorne holen"))
            }
        }
    }

    fn process_stats(&self) -> PResult<ProcStats> {
        stats()
    }

    fn reveal(&self, path: &std::path::Path) -> PResult<()> {
        let params = HSTRING::from(format!("/select,\"{}\"", path.display()));
        let result =
            unsafe { ShellExecuteW(None, w!("open"), w!("explorer.exe"), &params, PCWSTR::null(), SW_SHOWNORMAL) };
        if result.0 as usize > 32 {
            Ok(())
        } else {
            Err(fail("Explorer ließ sich nicht öffnen"))
        }
    }

    fn protect(&self, data: &[u8]) -> PResult<Vec<u8>> {
        dpapi(data, true)
    }

    fn unprotect(&self, data: &[u8]) -> PResult<Vec<u8>> {
        dpapi(data, false)
    }

    fn utc_offset_secs(&self) -> i32 {
        let mut tz = TIME_ZONE_INFORMATION::default();
        let mode = unsafe { GetTimeZoneInformation(&mut tz) };
        // UTC = lokal + Bias (Minuten); 2 = Sommerzeit aktiv
        let bias = tz.Bias + if mode == 2 { tz.DaylightBias } else { tz.StandardBias };
        -bias * 60
    }

    fn close_process(&self, process: &str) -> PResult<usize> {
        let targets: Vec<WindowInfo> =
            self.windows()?.into_iter().filter(|w| w.process.eq_ignore_ascii_case(process)).collect();
        for w in &targets {
            unsafe {
                let _ =
                    PostMessageW(Some(HWND(w.id as usize as *mut core::ffi::c_void)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        Ok(targets.len())
    }
}

/// DPAPI: an das Windows-Benutzerkonto gebunden, ohne eigenes Passwort.
fn dpapi(data: &[u8], encrypt: bool) -> PResult<Vec<u8>> {
    const ENTROPY: &[u8] = b"Panco-Notizen-v1";
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let entropy = CRYPT_INTEGER_BLOB { cbData: ENTROPY.len() as u32, pbData: ENTROPY.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        let res = if encrypt {
            CryptProtectData(&input, w!("Panco"), Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&input, None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        res.map_err(|e| fail(format!("Verschlüsselung: {e}")))?;
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut core::ffi::c_void)));
        Ok(bytes)
    }
}

/// Privater Arbeitssatz (MB) und CPU-Zeit (s) eines Prozesses.
fn proc_usage(pid: u32) -> Option<(f64, f64)> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
        let mut mem = PROCESS_MEMORY_COUNTERS_EX2 {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
            ..Default::default()
        };
        let mem_ok = GetProcessMemoryInfo(h, &mut mem as *mut _ as *mut PROCESS_MEMORY_COUNTERS, mem.cb).is_ok();
        let (mut c, mut e, mut k, mut u) = Default::default();
        let times_ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u).is_ok();
        let _ = CloseHandle(h);
        let ft = |f: ::windows::Win32::Foundation::FILETIME| {
            ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 1e7
        };
        Some((
            if mem_ok { mem.PrivateWorkingSetSize as f64 / 1_048_576.0 } else { 0.0 },
            if times_ok { ft(k) + ft(u) } else { 0.0 },
        ))
    }
}

/// Alle Prozesse als (pid, ppid, Exe-Name).
fn process_list() -> Vec<(u32, u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
            out.push((e.th32ProcessID, e.th32ParentProcessID, String::from_utf16_lossy(&e.szExeFile[..len])));
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

pub(super) fn stats() -> PResult<ProcStats> {
    let me = unsafe { GetCurrentProcessId() };
    let (own_mb, own_cpu) = proc_usage(me).ok_or_else(|| fail("Prozessdaten nicht lesbar"))?;
    let mut stats = ProcStats { own_mb, cpu_seconds: own_cpu, ..Default::default() };
    // WebView2: Browser-Prozess ist Kind von Panco, Renderer usw. sind dessen Kinder.
    let all = process_list();
    let mut tree = vec![me];
    let mut i = 0;
    while i < tree.len() {
        let parent = tree[i];
        for (pid, ppid, name) in &all {
            if *ppid == parent && *pid != parent && !tree.contains(pid) {
                tree.push(*pid);
                if let Some((mb, cpu)) = proc_usage(*pid) {
                    stats.children_mb += mb;
                    stats.cpu_seconds += cpu;
                }
                stats.children.push(name.clone());
            }
        }
        i += 1;
    }
    Ok(stats)
}

/// `HKCU\Software\Valve\Steam\SteamPath`
pub fn steam_path() -> Option<PathBuf> {
    let mut buf = [0u16; 520];
    let mut size = (buf.len() * 2) as u32;
    let res = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Valve\\Steam"),
            w!("SteamPath"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if res.is_err() {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1);
    let s = String::from_utf16_lossy(&buf[..len]).replace('/', "\\");
    (!s.is_empty()).then(|| PathBuf::from(s))
}

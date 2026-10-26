//! Mit Windows starten: ein Eintrag unter `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
//! (nur für dieses Benutzerkonto, keine Adminrechte). Er zeigt immer auf die aktuelle panco.exe –
//! wird der Ordner verschoben, korrigiert Panco den Eintrag beim nächsten Start.

/// Eintrag setzen oder entfernen.
pub fn sync(enabled: bool) -> Result<(), String> {
    imp::sync(enabled)
}

pub fn is_enabled() -> bool {
    imp::current().is_some()
}

#[cfg(windows)]
mod imp {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
    };

    const KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const NAME: PCWSTR = w!("Panco");

    fn command() -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        Ok(format!("\"{}\" --autostart", exe.display()))
    }

    pub fn current() -> Option<String> {
        let mut buf = [0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let ok = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                KEY,
                NAME,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut len),
            )
        };
        ok.is_ok().then(|| String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]))
    }

    pub fn sync(enabled: bool) -> Result<(), String> {
        if !enabled {
            if current().is_some() {
                unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, KEY, NAME) }.ok().map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        let cmd = command()?;
        if current().as_deref() == Some(cmd.as_str()) {
            return Ok(());
        }
        let wide = HSTRING::from(cmd.as_str());
        let bytes = ((wide.len() + 1) * 2) as u32;
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, KEY, NAME, REG_SZ.0, Some(wide.as_ptr().cast()), bytes) }
            .ok()
            .map_err(|e| e.to_string())
    }
}

/// Linux (Entwicklung): kein Autostart.
#[cfg(not(windows))]
mod imp {
    pub fn current() -> Option<String> {
        None
    }
    pub fn sync(_: bool) -> Result<(), String> {
        Ok(())
    }
}

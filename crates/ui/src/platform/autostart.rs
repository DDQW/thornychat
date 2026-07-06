//! Autostart registration via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`,
//! toggled from Settings. The app supports a `--minimized` launch flag so
//! autostart doesn't pop a window on login.

use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

const RUN_KEY_PATH: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "ThornyChat";
/// Value name autostart was registered under before the rename to
/// ThornyChat — see `migrate_legacy_value`.
const LEGACY_VALUE_NAME: &str = "Synapse";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn to_io_err(error: windows::core::Error) -> std::io::Error {
    std::io::Error::other(error.to_string())
}

/// Reads whether autostart is currently registered — a real registry read,
/// not the toggle's last-known UI state, so a value removed by hand (or by
/// an uninstaller) shows correctly whenever Settings opens.
pub fn is_enabled() -> bool {
    unsafe {
        let path = wide(RUN_KEY_PATH);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), 0, KEY_QUERY_VALUE, &mut hkey).is_err()
        {
            return false;
        }
        let name = wide(VALUE_NAME);
        let found = RegQueryValueExW(hkey, PCWSTR(name.as_ptr()), None, None, None, None).is_ok();
        let _ = RegCloseKey(hkey);
        found
    }
}

/// One-time cleanup after the rename from "Synapse": a Run value written
/// under the old name points at the old synapse.exe, so login would launch
/// a stale binary (or nothing at all once it's deleted). If the legacy
/// value exists, re-register autostart under the new name aimed at the
/// current exe, then delete the old value. The legacy value doubles as the
/// migration marker: it's only removed once the new registration succeeded,
/// so a failure here retries on the next launch.
pub fn migrate_legacy_value() {
    let legacy_exists = unsafe {
        let path = wide(RUN_KEY_PATH);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), 0, KEY_QUERY_VALUE, &mut hkey)
            .is_err()
        {
            return;
        }
        let name = wide(LEGACY_VALUE_NAME);
        let found = RegQueryValueExW(hkey, PCWSTR(name.as_ptr()), None, None, None, None).is_ok();
        let _ = RegCloseKey(hkey);
        found
    };
    if !legacy_exists || set_enabled(true).is_err() {
        return;
    }
    unsafe {
        let path = wide(RUN_KEY_PATH);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), 0, KEY_SET_VALUE, &mut hkey)
            .is_err()
        {
            return;
        }
        let name = wide(LEGACY_VALUE_NAME);
        let _ = RegDeleteValueW(hkey, PCWSTR(name.as_ptr()));
        let _ = RegCloseKey(hkey);
    }
}

/// Adds or removes the Run-key value, pointed at the current executable with
/// `--minimized` so a login-time launch doesn't pop the window over whatever
/// the user is doing.
pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
    // Resolve the fallible exe path BEFORE opening the key: a `?` bail here
    // while the key handle is held would leak it (RegCloseKey below is only
    // reached by falling through `result`). `command` is kept alive for the
    // whole `unsafe` block so the raw-slice borrow into it stays valid.
    let command = if enabled {
        let exe = std::env::current_exe()?;
        Some(wide(&format!("\"{}\" --minimized", exe.display())))
    } else {
        None
    };

    unsafe {
        let path = wide(RUN_KEY_PATH);
        let mut hkey = HKEY::default();
        RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), 0, KEY_SET_VALUE, &mut hkey)
            .ok()
            .map_err(to_io_err)?;
        let name = wide(VALUE_NAME);

        let result = if let Some(command) = &command {
            let bytes = std::slice::from_raw_parts(command.as_ptr().cast::<u8>(), command.len() * 2);
            RegSetValueExW(hkey, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes)).ok().map_err(to_io_err)
        } else {
            let outcome = RegDeleteValueW(hkey, PCWSTR(name.as_ptr()));
            // Already absent is the desired end state either way.
            if outcome.is_ok() || outcome == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                outcome.ok().map_err(to_io_err)
            }
        };

        let _ = RegCloseKey(hkey);
        result
    }
}

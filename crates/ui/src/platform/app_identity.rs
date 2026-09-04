//! Windows app identity — the AppUserModelID (AUMID) every toast is keyed
//! off, plus the local "installer" that registers it.
//!
//! Win11 will not display a toast from a plain unpackaged exe. The shell
//! resolves the notifier's AUMID to *something it knows about*, and for an
//! unpackaged Win32 app the only thing that counts is a Start Menu shortcut
//! carrying the `System.AppUserModel.ID` property. No shortcut, no toast —
//! `ToastNotifier::Show` returns success and nothing appears, which is the
//! failure mode this module exists to prevent.
//!
//! So the "installer" is deliberately small: one shortcut in the user's
//! Start Menu plus two HKCU keys. No MSIX, no signing certificate, no
//! admin rights — an MSIX (sparse or full) would give real package identity
//! but needs a trusted cert installed on the machine, which is the wrong
//! trade for a dev box. Run it once per machine:
//!
//! ```text
//! cargo xtask install-dev     # register
//! cargo xtask toast-test      # verify a toast actually appears
//! cargo xtask uninstall-dev   # remove
//! ```
//!
//! What gets written:
//!   - `%APPDATA%\..\Start Menu\Programs\ThornyChat (Dev).lnk`, targeting the
//!     current exe, with the AUMID and toast-activator CLSID properties set.
//!   - `HKCU\Software\Classes\AppUserModelId\<AUMID>` — display name and
//!     activator, what Win11 shows as the toast's app header.
//!   - `HKCU\Software\Classes\CLSID\<activator>\LocalServer32` — where toast
//!     activations (button clicks, inline reply) will be delivered once the
//!     COM callback lands. Registered now so packaging is settled: changing
//!     the AUMID later orphans every pinned taskbar entry and notification
//!     setting the user has accumulated under the old one.
//!
//! The activator CLSID is registered ahead of its implementation, so until
//! `INotificationActivationCallback` exists Windows launches the exe with
//! `-ToastActivated` on a toast click; `crates/app/src/main.rs` recognizes
//! that and exits rather than opening a second window.

use std::io;
use std::path::{Path, PathBuf};

use windows::core::{Interface, GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, PROPERTYKEY};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::System::Variant::{VT_CLSID, VT_LPWSTR};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    SHChangeNotify, SHGetKnownFolderPath, SetCurrentProcessExplicitAppUserModelID, IShellLinkW,
    FOLDERID_Programs, KF_FLAG_DEFAULT, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, ShellLink,
};

/// The one identity string the whole app is known by: toast notifier id,
/// taskbar grouping key, and the value stamped into the Start Menu shortcut.
/// `Vendor.Product` is the documented convention. Treat as frozen — see the
/// module docs on why changing it is not free.
pub const AUMID: &str = "Woelki.ThornyChat";

/// COM class the shell activates when a toast (or one of its buttons) is
/// clicked. Ours alone; generated once, never regenerated.
pub const TOAST_ACTIVATOR_CLSID: GUID = GUID::from_u128(0x8f5d2a93_6c41_4e7b_9d0a_3b27e1c4f86d);

/// "(Dev)" in the name so a future real installer's shortcut can sit beside
/// this one without either clobbering the other.
const SHORTCUT_FILE_NAME: &str = "ThornyChat (Dev).lnk";
const DISPLAY_NAME: &str = "ThornyChat";

/// `System.AppUserModel.ID` and `System.AppUserModel.ToastActivatorCLSID`.
/// Both live in the same property set; propsys.h spells them out, but the
/// `windows` crate doesn't re-export shell PKEYs, so they're written here.
const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};
const PKEY_TOAST_ACTIVATOR_CLSID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 26,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn to_io_err(error: windows::core::Error) -> io::Error {
    io::Error::other(error.to_string())
}

/// Tells the shell this process *is* the AUMID above, rather than letting it
/// guess one from the exe path. Two effects: the window groups under (and can
/// be pinned to) the same taskbar button as the registered shortcut, and
/// toasts raised from this process are attributed to the same identity.
///
/// Must run before the first window exists, so call it early in `main`.
/// Failure is not fatal — toasts still work, since the notifier names the
/// AUMID explicitly — so this only logs.
pub fn set_process_aumid() {
    let id = wide(AUMID);
    if let Err(error) = unsafe { SetCurrentProcessExplicitAppUserModelID(PCWSTR(id.as_ptr())) } {
        tracing::warn!(%error, "failed to set process AppUserModelID");
    }
}

/// `%APPDATA%\Microsoft\Windows\Start Menu\Programs\ThornyChat (Dev).lnk`,
/// resolved through the known-folder API rather than assembled from
/// `%APPDATA%` — the Start Menu is relocatable.
pub fn shortcut_path() -> io::Result<PathBuf> {
    let programs = unsafe {
        let raw = SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None).map_err(to_io_err)?;
        take_co_string(raw)
    };
    Ok(PathBuf::from(programs).join(SHORTCUT_FILE_NAME))
}

/// Whether this machine has been through `install`. Checks the file itself,
/// so a shortcut deleted by hand reads as uninstalled.
pub fn is_installed() -> bool {
    shortcut_path().map(|path| path.exists()).unwrap_or(false)
}

/// Registers the currently running exe for toasts. Idempotent: re-running it
/// overwrites the shortcut, which is how you re-point the registration after
/// moving or rebuilding the binary elsewhere.
pub fn install() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let path = shortcut_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let _com = ComGuard::new();
    write_shortcut(&exe, &path)?;

    let activator = clsid_string(&TOAST_ACTIVATOR_CLSID);
    let identity_key = format!("Software\\Classes\\AppUserModelId\\{AUMID}");
    set_string_value(&identity_key, "DisplayName", DISPLAY_NAME)?;
    set_string_value(&identity_key, "CustomActivator", &activator)?;
    // Quoted: the shell appends its own `-ToastActivated` argument, and an
    // unquoted path with spaces would split at the first one.
    set_string_value(
        &format!("Software\\Classes\\CLSID\\{activator}\\LocalServer32"),
        "",
        &format!("\"{}\"", exe.display()),
    )?;

    // Nudge the shell to re-read the Start Menu now; without it the first
    // toast can be swallowed until the folder is scanned on its own schedule.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };

    Ok(path)
}

/// Removes everything `install` wrote. Anything already gone counts as
/// success — the goal is the end state, not the bookkeeping.
pub fn uninstall() -> io::Result<()> {
    let path = shortcut_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    delete_key(&format!("Software\\Classes\\AppUserModelId\\{AUMID}"))?;
    delete_key(&format!("Software\\Classes\\CLSID\\{}", clsid_string(&TOAST_ACTIVATOR_CLSID)))?;

    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    Ok(())
}

/// Creates the .lnk and stamps the two AppUserModel properties onto it.
/// The properties are the entire point — a shortcut without them registers
/// nothing as far as the notification platform is concerned.
fn write_shortcut(exe: &Path, path: &Path) -> io::Result<()> {
    unsafe {
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(to_io_err)?;

        let exe_wide = wide(&exe.display().to_string());
        link.SetPath(PCWSTR(exe_wide.as_ptr())).map_err(to_io_err)?;
        // The icon Windows draws on the toast comes from here, and the exe
        // already carries one (embedded by crates/app/build.rs).
        link.SetIconLocation(PCWSTR(exe_wide.as_ptr()), 0).map_err(to_io_err)?;
        if let Some(dir) = exe.parent() {
            let dir_wide = wide(&dir.display().to_string());
            link.SetWorkingDirectory(PCWSTR(dir_wide.as_ptr())).map_err(to_io_err)?;
        }
        let description = wide("ThornyChat (local dev build)");
        link.SetDescription(PCWSTR(description.as_ptr())).map_err(to_io_err)?;

        let store: IPropertyStore = link.cast().map_err(to_io_err)?;
        // Buffers outlive both SetValue calls: SetValue copies the value, but
        // only while these are still alive.
        let mut aumid = wide(AUMID);
        let mut clsid = TOAST_ACTIVATOR_CLSID;
        store.SetValue(&PKEY_APP_USER_MODEL_ID, &string_propvariant(&mut aumid)).map_err(to_io_err)?;
        store
            .SetValue(&PKEY_TOAST_ACTIVATOR_CLSID, &clsid_propvariant(&mut clsid))
            .map_err(to_io_err)?;
        store.Commit().map_err(to_io_err)?;

        let file: IPersistFile = link.cast().map_err(to_io_err)?;
        let target = wide(&path.display().to_string());
        file.Save(PCWSTR(target.as_ptr()), true).map_err(to_io_err)?;
    }
    Ok(())
}

/// `VT_LPWSTR` variant borrowing `buffer`. Constructed by hand because the
/// `windows` crate exposes `PROPVARIANT` as a raw union with no conversions.
/// It has no `Drop`, so nothing here tries to free the borrowed buffer — the
/// caller keeps owning it.
fn string_propvariant(buffer: &mut [u16]) -> PROPVARIANT {
    let mut value = PROPVARIANT::default();
    unsafe {
        let inner = &mut *value.Anonymous.Anonymous;
        inner.vt = VT_LPWSTR;
        inner.Anonymous.pwszVal = PWSTR(buffer.as_mut_ptr());
    }
    value
}

/// `VT_CLSID` counterpart, likewise borrowing the caller's GUID.
fn clsid_propvariant(guid: &mut GUID) -> PROPVARIANT {
    let mut value = PROPVARIANT::default();
    unsafe {
        let inner = &mut *value.Anonymous.Anonymous;
        inner.vt = VT_CLSID;
        inner.Anonymous.puuid = guid;
    }
    value
}

/// Registry-shaped GUID: braced, uppercase, hyphenated. The shell matches
/// `CustomActivator` against the `CLSID\{...}` key name as a string, so the
/// formatting has to be exactly this.
fn clsid_string(guid: &GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7],
    )
}

/// Writes one REG_SZ under HKCU, creating the key path as needed. An empty
/// `name` writes the key's default value.
fn set_string_value(subkey: &str, name: &str, value: &str) -> io::Result<()> {
    unsafe {
        let path = wide(subkey);
        let mut hkey = HKEY::default();
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        )
        .ok()
        .map_err(to_io_err)?;

        // The NUL is part of the written data for REG_SZ, so `wide`'s
        // terminator is intentionally included in the byte count.
        let data = wide(value);
        let bytes = std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2);
        let name = wide(name);
        let result = RegSetValueExW(hkey, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes))
            .ok()
            .map_err(to_io_err);

        let _ = RegCloseKey(hkey);
        result
    }
}

/// Deletes a key and everything under it; already-absent is success.
fn delete_key(subkey: &str) -> io::Result<()> {
    unsafe {
        let path = wide(subkey);
        let outcome = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()));
        if outcome.is_ok() || outcome == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            outcome.ok().map_err(to_io_err)
        }
    }
}

/// Reads a shell-allocated string and frees it with the allocator that
/// produced it.
unsafe fn take_co_string(raw: PWSTR) -> String {
    let owned = unsafe { raw.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(raw.0.cast())) };
    owned
}

/// Balances `CoInitializeEx` on whichever thread runs the installer. The app's
/// main thread is already an STA by the time anything here could run from the
/// UI, in which case `CoInitializeEx` returns `S_FALSE` and still needs the
/// matching uninitialize; `RPC_E_CHANGED_MODE` means our call did nothing and
/// must not be balanced.
struct ComGuard(bool);

impl ComGuard {
    fn new() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        ComGuard(hr.is_ok())
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clsid_string_matches_registry_form() {
        assert_eq!(
            clsid_string(&TOAST_ACTIVATOR_CLSID),
            "{8F5D2A93-6C41-4E7B-9D0A-3B27E1C4F86D}"
        );
    }

    #[test]
    fn shortcut_lands_in_the_start_menu() {
        let path = shortcut_path().expect("known folder lookup failed");
        assert!(path.ends_with(SHORTCUT_FILE_NAME));
        assert!(path.to_string_lossy().contains("Start Menu"));
    }
}

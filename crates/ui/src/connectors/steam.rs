//! Steam connector. Steam keeps `HKCU\Software\Valve\Steam\RunningAppID` (the
//! AppID of the game currently running, `0` when none) plus a per-app
//! `…\Apps\<id>\Name`. That's a clean, local, no-login signal for exactly which
//! game is up — no process scanning needed.

use windows::Win32::System::Registry::HKEY_CURRENT_USER;

use super::reg::Key;

/// The currently-running Steam game's display name, or `None` when Steam isn't
/// running a game (or isn't installed).
pub fn running_game() -> Option<String> {
    let steam = Key::open(HKEY_CURRENT_USER, r"Software\Valve\Steam")?;
    let app_id = steam.dword("RunningAppID")?;
    if app_id == 0 {
        return None;
    }
    // Prefer the human name; fall back to the bare AppID so a just-installed
    // game whose `Name` hasn't populated yet still announces *something*.
    let name = Key::open(HKEY_CURRENT_USER, &format!(r"Software\Valve\Steam\Apps\{app_id}"))
        .and_then(|apps| apps.string("Name"))
        .unwrap_or_else(|| format!("app {app_id}"));
    Some(name)
}

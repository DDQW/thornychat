//! GOG connector. GOG installs register under
//! `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<gameId>` with `gameName` and `exe`
//! (a full path). There's no "running game" key like Steam's, so we build an
//! exe→name map from that list and match it against running processes.

use std::collections::HashSet;

use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

use super::reg::Key;

const GOG_GAMES_KEY: &str = r"SOFTWARE\WOW6432Node\GOG.com\Games";

/// `(lowercased exe base name, display name)` for every installed GOG game.
pub fn installed_games() -> Vec<(String, String)> {
    let Some(root) = Key::open(HKEY_LOCAL_MACHINE, GOG_GAMES_KEY) else {
        return Vec::new();
    };
    let mut games = Vec::new();
    for id in root.subkey_names() {
        let Some(game) = Key::open(HKEY_LOCAL_MACHINE, &format!(r"{GOG_GAMES_KEY}\{id}")) else {
            continue;
        };
        // `exe` is a full path; reduce to a lowercased base name to match
        // `process::running_exe_names`.
        if let (Some(name), Some(exe)) =
            (game.string("gameName"), game.string("exe").and_then(|p| super::exe_base(&p)))
        {
            games.push((exe, name));
        }
    }
    games
}

/// Display name of the running GOG game, if any installed game's exe is in the
/// running set.
pub fn running_game(running: &HashSet<String>) -> Option<String> {
    super::match_running(installed_games(), running)
}

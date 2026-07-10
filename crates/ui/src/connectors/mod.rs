//! Game-launcher activity connectors. A poll tick (see `subscriptions.rs`)
//! calls [`detect_active_game`] off-thread; `update.rs` diffs the result
//! against the last known game and posts an `m.emote` (`* you plays …`) into
//! the room you're currently viewing when it changes. Windows-only — every
//! source reads a Windows registry key, manifest, or process list.
//!
//! Only games this phase; a `media.rs` "now playing" source (Spotify/YouTube/
//! … via the Windows media-transport API) slots in the same shape later.

mod epic;
mod gog;
mod process;
mod reg;
mod steam;

/// A game detected as currently running, and which launcher saw it. `source`
/// is carried for future per-launcher formatting/emoji; the emote body
/// currently uses only `name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveGame {
    pub name: String,
    pub source: GameSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameSource {
    Steam,
    Gog,
    Epic,
}

/// The single game to announce right now, honoring the enabled connectors in
/// precedence order (Steam → GOG → Epic — a user plays one at a time, so the
/// first hit wins). Runs registry/process reads, so call it off the UI thread
/// (`update.rs` wraps it in `spawn_blocking`). `None` when nothing enabled is
/// running.
pub fn detect_active_game(cfg: &crate::connectors_config::ConnectorsConfig) -> Option<ActiveGame> {
    if cfg.steam_enabled {
        if let Some(name) = steam::running_game() {
            return Some(ActiveGame { name, source: GameSource::Steam });
        }
    }

    // GOG and Epic share one process snapshot — only taken if one of them is
    // on (it's the only non-trivial cost in the idle-but-enabled case).
    if cfg.gog_enabled || cfg.epic_enabled {
        let running = process::running_exe_names();
        if cfg.gog_enabled {
            if let Some(name) = gog::running_game(&running) {
                return Some(ActiveGame { name, source: GameSource::Gog });
            }
        }
        if cfg.epic_enabled {
            if let Some(name) = epic::running_game(&running) {
                return Some(ActiveGame { name, source: GameSource::Epic });
            }
        }
    }

    None
}

/// Lowercased base file name of a Windows path, e.g.
/// `C:\Games\Foo\foo.exe` → `foo.exe`. Shared by the GOG and Epic matchers.
pub(crate) fn exe_base(path: &str) -> Option<String> {
    let base = path.rsplit(['\\', '/']).next()?.trim();
    (!base.is_empty()).then(|| base.to_lowercase())
}

/// First installed game whose exe base name is in the running-process set —
/// the shared GOG/Epic match step, kept pure so it's unit-testable without a
/// registry or filesystem.
pub(crate) fn match_running(
    games: Vec<(String, String)>,
    running: &std::collections::HashSet<String>,
) -> Option<String> {
    games.into_iter().find(|(exe, _)| running.contains(exe)).map(|(_, name)| name)
}

/// The emote body to post for a game-state transition, or `None` when the
/// change shouldn't be announced. A start or switch always announces
/// (`plays X`); a stop announces `stopped playing X` only when `announce_stop`
/// is on. Pure — the sending lives in `update::apply_connector_change`.
pub fn emote_body(
    previous: Option<&ActiveGame>,
    current: Option<&ActiveGame>,
    announce_stop: bool,
) -> Option<String> {
    if previous == current {
        return None;
    }
    match current {
        Some(game) => Some(format!("plays {}", game.name)),
        None => {
            let prev = previous?;
            announce_stop.then(|| format!("stopped playing {}", prev.name))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(name: &str) -> ActiveGame {
        ActiveGame { name: name.to_string(), source: GameSource::Steam }
    }

    #[test]
    fn exe_base_reduces_and_lowercases() {
        assert_eq!(exe_base(r"C:\Games\Foo\Foo.exe").as_deref(), Some("foo.exe"));
        assert_eq!(exe_base("bar.exe").as_deref(), Some("bar.exe"));
        assert_eq!(exe_base("Sub/Dir/Baz.EXE").as_deref(), Some("baz.exe"));
        assert_eq!(exe_base(r"C:\Games\trailing\"), None);
        assert_eq!(exe_base(""), None);
    }

    #[test]
    fn match_running_finds_first_running_exe() {
        let games = vec![
            ("foo.exe".to_string(), "Foo".to_string()),
            ("bar.exe".to_string(), "Bar".to_string()),
        ];
        let running: std::collections::HashSet<String> =
            ["explorer.exe".to_string(), "bar.exe".to_string()].into_iter().collect();
        assert_eq!(match_running(games, &running).as_deref(), Some("Bar"));
    }

    #[test]
    fn match_running_none_when_nothing_matches() {
        let games = vec![("foo.exe".to_string(), "Foo".to_string())];
        let running: std::collections::HashSet<String> =
            ["explorer.exe".to_string()].into_iter().collect();
        assert_eq!(match_running(games, &running), None);
    }

    #[test]
    fn emote_body_announces_start_and_switch() {
        assert_eq!(emote_body(None, Some(&game("Foo")), false).as_deref(), Some("plays Foo"));
        assert_eq!(
            emote_body(Some(&game("Foo")), Some(&game("Bar")), false).as_deref(),
            Some("plays Bar")
        );
    }

    #[test]
    fn emote_body_stop_is_gated_by_announce_stop() {
        assert_eq!(emote_body(Some(&game("Foo")), None, false), None);
        assert_eq!(
            emote_body(Some(&game("Foo")), None, true).as_deref(),
            Some("stopped playing Foo")
        );
    }

    #[test]
    fn emote_body_no_change_is_silent() {
        assert_eq!(emote_body(Some(&game("Foo")), Some(&game("Foo")), true), None);
        assert_eq!(emote_body(None, None, true), None);
    }
}

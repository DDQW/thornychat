//! Epic connector. The Epic launcher writes one JSON manifest per installed
//! game under `%PROGRAMDATA%\Epic\EpicGamesLauncher\Data\Manifests\*.item`, each
//! with a `DisplayName` and a `LaunchExecutable`. No "running game" signal, so
//! — like GOG — we build an exe→name map and match running processes.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::Deserialize;

/// Just the two fields we need out of an Epic `.item` manifest.
#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "DisplayName")]
    display_name: String,
    #[serde(rename = "LaunchExecutable")]
    launch_executable: String,
}

fn manifests_dir() -> PathBuf {
    let program_data =
        std::env::var("PROGRAMDATA").unwrap_or_else(|_| r"C:\ProgramData".to_string());
    PathBuf::from(program_data).join(r"Epic\EpicGamesLauncher\Data\Manifests")
}

/// `(lowercased exe base name, display name)` for every installed Epic game.
pub fn installed_games() -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(manifests_dir()) else {
        return Vec::new();
    };
    let mut games = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("item") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else { continue };
        if let Some(game) = parse_manifest(&contents) {
            games.push(game);
        }
    }
    games
}

/// Parse one Epic `.item` manifest into `(lowercased exe base, display name)`.
/// `None` for a non-manifest JSON or a non-launchable entry (empty
/// `DisplayName`/`LaunchExecutable` — Epic writes some of those for tools/DLC).
fn parse_manifest(contents: &str) -> Option<(String, String)> {
    let manifest: Manifest = serde_json::from_str(contents).ok()?;
    let exe = super::exe_base(&manifest.launch_executable)?;
    (!manifest.display_name.is_empty()).then_some((exe, manifest.display_name))
}

/// Display name of the running Epic game, if any installed game's exe is in the
/// running set.
pub fn running_game(running: &HashSet<String>) -> Option<String> {
    super::match_running(installed_games(), running)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_display_name_and_exe_base() {
        let json = r#"{
            "DisplayName": "Rocket League",
            "LaunchExecutable": "Binaries/Win64/RocketLeague.exe",
            "InstallLocation": "C:/Games/rocketleague"
        }"#;
        assert_eq!(
            parse_manifest(json),
            Some(("rocketleague.exe".to_string(), "Rocket League".to_string()))
        );
    }

    #[test]
    fn rejects_manifest_without_launch_exe() {
        let json = r#"{ "DisplayName": "Some DLC", "LaunchExecutable": "" }"#;
        assert_eq!(parse_manifest(json), None);
    }

    #[test]
    fn rejects_non_manifest_json() {
        assert_eq!(parse_manifest("not json"), None);
        assert_eq!(parse_manifest("{}"), None);
    }
}

//! Game-activity connectors: polls the enabled launchers and, when the game
//! you're playing changes, tells the page to post an `m.emote`
//! (`* you plays Half-Life`) into the room you're looking at. The poll lives
//! here because it reads the registry and the process list; the *posting*
//! stays with the page, which is the only place that knows which room is open.

use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::commands::SettingsStore;
use crate::config::connectors::MIN_POLL_INTERVAL_SECS;
use crate::connectors::{detect_active_game, emote_body, ActiveGame};

/// Event name the page listens for; the payload is the emote body.
pub const EMOTE_EVENT: &str = "connector-emote";

/// How often to look at the settings again while every connector is off, so
/// turning one on starts polling within seconds without a restart.
const IDLE_RECHECK: Duration = Duration::from_secs(10);

pub fn spawn(app: AppHandle, settings: SettingsStore) {
    tauri::async_runtime::spawn(async move {
        let mut previous: Option<ActiveGame> = None;
        loop {
            let config = settings.0.lock().expect("settings lock poisoned").connectors;
            if !config.any_enabled() {
                // Forget the game so re-enabling announces it afresh.
                previous = None;
                tokio::time::sleep(IDLE_RECHECK).await;
                continue;
            }

            // Registry and process-list reads: off the async threads.
            let current = tauri::async_runtime::spawn_blocking(move || detect_active_game(&config))
                .await
                .unwrap_or(None);
            if let Some(body) = emote_body(previous.as_ref(), current.as_ref(), config.announce_stop) {
                tracing::info!(%body, "game activity changed");
                if let Err(error) = app.emit(EMOTE_EVENT, body) {
                    tracing::warn!(%error, "could not deliver the game-activity emote");
                }
            }
            previous = current;

            tokio::time::sleep(Duration::from_secs(config.poll_interval_secs.max(MIN_POLL_INTERVAL_SECS))).await;
        }
    });
}

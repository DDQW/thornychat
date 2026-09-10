//! Read-side of the account's ignore list (`m.ignored_user_list`): pushes
//! the current set of ignored users to the UI, both at startup and whenever
//! it changes — including changes made from another device, since the list
//! is global account data that arrives through sync like any other.
//!
//! The write-side (ignore/unignore) lives in `sync.rs`'s command handlers.
//!
//! Nothing here filters anything. Ignoring is enforced by the *homeserver*:
//! it stops sending that user's events in `/sync` entirely (Matrix spec,
//! "Ignoring Users"), and matrix-sdk's event cache clears its stored rooms
//! on every ignore-list change so the already-cached history goes too. The
//! UI needs this list only to label the menu entry and offer the undo.

use matrix_sdk::ruma::events::ignored_user_list::IgnoredUserListEventContent;
use matrix_sdk::Client;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::events::ClientEvent;

/// Reads the ignore list straight from account data. Used for the startup
/// snapshot only: the SDK's observable starts empty and is first filled by
/// a sync response, so waiting on it alone would show "nobody is ignored"
/// until the first sync carrying the account data lands.
async fn snapshot(client: &Client) -> Vec<String> {
    let raw = match client.account().account_data::<IgnoredUserListEventContent>().await {
        Ok(Some(raw)) => raw,
        // No list yet (nobody has ever been ignored on this account) is the
        // same answer as an empty one.
        Ok(None) => return Vec::new(),
        Err(error) => {
            tracing::warn!(%error, "couldn't read the ignored-user list; assuming empty");
            return Vec::new();
        }
    };
    match raw.deserialize() {
        Ok(content) => content.ignored_users.keys().map(|user| user.to_string()).collect(),
        Err(error) => {
            tracing::warn!(%error, "couldn't parse the ignored-user list; assuming empty");
            Vec::new()
        }
    }
}

/// Spawns the ignore-list watcher. Detached for the process lifetime like
/// the other watchers — it ends on its own once `event_tx` is gone.
pub fn spawn_watcher(
    client: Client,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Subscribe BEFORE the snapshot, for the same reason `push.rs` does:
        // the first sync's account data can land during the read, and a
        // change published with no subscriber attached is simply lost.
        let mut changes = client.subscribe_to_ignore_user_list_changes();
        if event_tx.send(ClientEvent::IgnoredUsersUpdated(snapshot(&client).await)).is_err() {
            return;
        }
        // The observable carries the whole list, so each tick is already a
        // full snapshot — no re-read needed.
        while let Some(users) = changes.next().await {
            if event_tx.send(ClientEvent::IgnoredUsersUpdated(users)).is_err() {
                break;
            }
        }
    })
}

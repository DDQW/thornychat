use matrix_sdk::Client;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

use crate::commands::ClientCommand;
use crate::error::CoreResult;
use crate::events::ClientEvent;
use crate::session::{self, RestoredSession};
use crate::store::AppPaths;
use crate::sync;

/// Everything `app::main` needs to hand off to the UI layer once a session
/// exists: the command sender for `update()` to push through, and the join
/// handle for graceful shutdown.
pub struct RunningClient {
    pub client: Client,
    pub cmd_tx: mpsc::UnboundedSender<ClientCommand>,
    pub worker_handle: JoinHandle<()>,
}

/// Restores a saved session and starts the sync worker if one exists.
/// Returns `None` if interactive login is required.
pub async fn try_start(
    profile: &str,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) -> CoreResult<Option<RunningClient>> {
    let paths = AppPaths::for_profile(profile)?;
    let Some(RestoredSession { client, .. }) = session::restore_or_none(&paths).await? else {
        return Ok(None);
    };

    spawn_cache_eviction(&paths);
    let (cmd_tx, worker_handle) = sync::spawn(client.clone(), event_tx, paths);
    Ok(Some(RunningClient { client, cmd_tx, worker_handle }))
}

pub async fn start_with_password(
    profile: &str,
    homeserver: &str,
    username: &str,
    password: Zeroizing<String>,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) -> CoreResult<RunningClient> {
    let paths = AppPaths::for_profile(profile)?;
    let RestoredSession { client, .. } =
        session::login_password(&paths, homeserver, username, password).await?;

    spawn_cache_eviction(&paths);
    let (cmd_tx, worker_handle) = sync::spawn(client.clone(), event_tx, paths);
    Ok(RunningClient { client, cmd_tx, worker_handle })
}

pub async fn start_with_sso(
    profile: &str,
    homeserver: &str,
    identity_provider_id: Option<&str>,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) -> CoreResult<RunningClient> {
    let paths = AppPaths::for_profile(profile)?;
    let RestoredSession { client, .. } =
        session::login_sso(&paths, homeserver, identity_provider_id).await?;

    spawn_cache_eviction(&paths);
    let (cmd_tx, worker_handle) = sync::spawn(client.clone(), event_tx, paths);
    Ok(RunningClient { client, cmd_tx, worker_handle })
}

/// Fire-and-forget startup sweep keeping the media/emoji disk caches under
/// their size caps (see `media::evict_cache_dir`). Spawned wherever a
/// client actually starts — a fresh install with no session has nothing to
/// sweep, so the login screen path skips it.
fn spawn_cache_eviction(paths: &AppPaths) {
    let media_dir = paths.media_cache_dir();
    let emoji_dir = paths.emoji_cache_dir();
    // spawn_blocking: the sweep is a synchronous directory walk.
    tokio::task::spawn_blocking(move || {
        crate::media::evict_cache_dir(&media_dir, crate::media::MEDIA_CACHE_CAP_BYTES);
        crate::media::evict_cache_dir(&emoji_dir, crate::media::EMOJI_CACHE_CAP_BYTES);
    });
}

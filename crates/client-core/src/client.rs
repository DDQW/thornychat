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

    let (cmd_tx, worker_handle) = sync::spawn(client.clone(), event_tx, paths.media_cache_dir());
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

    let (cmd_tx, worker_handle) = sync::spawn(client.clone(), event_tx, paths.media_cache_dir());
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

    let (cmd_tx, worker_handle) = sync::spawn(client.clone(), event_tx, paths.media_cache_dir());
    Ok(RunningClient { client, cmd_tx, worker_handle })
}

pub async fn logout(profile: &str, running: RunningClient) -> CoreResult<()> {
    let paths = AppPaths::for_profile(profile)?;
    session::logout(&paths, &running.client).await?;
    running.worker_handle.abort();
    Ok(())
}

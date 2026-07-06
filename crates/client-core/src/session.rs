//! Login (password + SSO) and session persistence.
//!
//! Session secrets (access/refresh token) are stored in the Windows
//! Credential Manager via `keyring`, keyed by homeserver+user. The
//! non-secret session metadata (user id, device id, homeserver) is cached
//! alongside it as JSON so `restore_or_login` doesn't need a network round
//! trip just to know *whether* a saved session exists.
//!
use std::path::Path;

use matrix_sdk::{
    authentication::matrix::MatrixSession,
    config::SyncSettings,
    ruma::{
        api::client::session::get_login_types::v3::LoginType, OwnedDeviceId, OwnedUserId,
    },
    Client, SessionTokens,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{CoreError, CoreResult};
use crate::store::AppPaths;

const KEYRING_SERVICE: &str = "ThornyChat Matrix Client";
/// Service name tokens were stored under before the rename to ThornyChat;
/// still read (and migrated forward) so an existing login survives.
const LEGACY_KEYRING_SERVICE: &str = "Synapse Matrix Client";
const DEVICE_DISPLAY_NAME: &str = "ThornyChat (Windows)";

/// Non-secret session metadata cached on disk so we know whether a saved
/// login exists without touching the keyring or network.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionMeta {
    homeserver: String,
    user_id: String,
    device_id: String,
}

pub struct RestoredSession {
    pub client: Client,
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
}

/// What a homeserver actually supports, discovered via `GET
/// /_matrix/client/v3/login` before showing the user any login form —
/// mirrors how Element decides whether to show a password field, an SSO
/// button, or both (many homeservers, e.g. ones delegating auth to a forum
/// or SSO provider, only support `m.login.sso` and disable password login
/// entirely).
#[derive(Debug, Clone, Default)]
pub struct LoginFlows {
    pub supports_password: bool,
    pub supports_sso: bool,
    /// Specific identity providers the server advertises (e.g. "GitLab",
    /// "Forum Account"). Empty even when `supports_sso` is true if the
    /// server exposes a single generic SSO flow with no named providers.
    pub sso_providers: Vec<SsoIdentityProvider>,
}

#[derive(Debug, Clone)]
pub struct SsoIdentityProvider {
    pub id: String,
    pub name: String,
}

/// Queries `homeserver` for its supported login flows without persisting
/// anything or touching the on-disk store — used to drive the login
/// screen's homeserver-first step before showing password/SSO options.
pub async fn discover_login_flows(homeserver: &str) -> CoreResult<LoginFlows> {
    let client = Client::builder()
        .server_name_or_homeserver_url(homeserver)
        .build()
        .await?;

    let response = client
        .matrix_auth()
        .get_login_types()
        .await
        .map_err(|e| CoreError::Other(format!("could not reach homeserver: {e}")))?;

    let mut flows = LoginFlows::default();
    for flow in response.flows {
        match flow {
            LoginType::Password(_) => flows.supports_password = true,
            LoginType::Sso(sso) => {
                flows.supports_sso = true;
                flows.sso_providers.extend(
                    sso.identity_providers
                        .into_iter()
                        .map(|idp| SsoIdentityProvider { id: idp.id, name: idp.name }),
                );
            }
            _ => {}
        }
    }

    if !flows.supports_password && !flows.supports_sso {
        return Err(CoreError::Other(
            "this homeserver doesn't support password or SSO login".into(),
        ));
    }

    Ok(flows)
}

/// Attempts to restore a previously saved session; falls back to `None` if
/// no session is cached, letting the caller drive an interactive login.
pub async fn try_restore(paths: &AppPaths) -> CoreResult<Option<RestoredSession>> {
    let meta_path = paths.root.join("session.json");
    if !meta_path.exists() {
        // No session — any leftover store is orphaned (its crypto account is
        // bound to a dead device and would fail a fresh login with
        // MismatchedAccount). This also retries discards that failed while a
        // live Client still held the sqlite files open (Windows sharing
        // violation, e.g. during logout): at startup no client exists yet,
        // so the removal can actually succeed.
        discard_state_store(paths);
        return Ok(None);
    }

    let meta: SessionMeta = match serde_json::from_slice(&std::fs::read(&meta_path)?) {
        Ok(meta) => meta,
        Err(error) => {
            // A truncated/corrupt session.json (crash mid-write, disk issues)
            // would otherwise surface a cryptic serde error on every launch,
            // permanently. Treat it as "no saved session".
            tracing::warn!(%error, "corrupt session.json, falling back to interactive login");
            let _ = std::fs::remove_file(&meta_path);
            discard_state_store(paths);
            return Ok(None);
        }
    };

    let entry = keyring::Entry::new(KEYRING_SERVICE, &meta.user_id)?;
    let tokens_json = match entry.get_password() {
        Ok(pw) => pw,
        Err(keyring::Error::NoEntry) => match migrate_legacy_keyring_entry(&entry, &meta.user_id)
        {
            Some(pw) => pw,
            None => {
                // Tokens are gone: the store's crypto account is bound to the dead
                // device id, and a fresh login on top of it fails with
                // MismatchedAccount (orphaning a new server-side device per retry).
                let _ = std::fs::remove_file(&meta_path);
                discard_state_store(paths);
                return Ok(None);
            }
        },
        Err(e) => return Err(e.into()),
    };
    let tokens: SessionTokens = match serde_json::from_str(&tokens_json) {
        Ok(tokens) => tokens,
        Err(error) => {
            tracing::warn!(%error, "corrupt token entry in credential manager, falling back to interactive login");
            let _ = entry.delete_credential();
            let _ = std::fs::remove_file(&meta_path);
            discard_state_store(paths);
            return Ok(None);
        }
    };

    // From here on, failures are store problems (meta.homeserver is a full
    // URL, so build_client does no network discovery; restore_session is
    // local store activation): a corrupt sqlite store or one bound to a
    // different device (MismatchedAccount). Propagating them would wedge
    // startup permanently — interactive login builds on the SAME store and
    // fails identically. Self-heal like the corruption paths above: drop
    // the session and store, fall back to a clean interactive login. The
    // keyring entry is left in place (overwritten by the next login).
    let client = match build_client(&meta.homeserver, &paths.state_store_dir()).await {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(%error, "state store unusable, falling back to interactive login");
            let _ = std::fs::remove_file(&meta_path);
            discard_state_store(paths);
            return Ok(None);
        }
    };

    let session = MatrixSession {
        meta: matrix_sdk::SessionMeta {
            user_id: meta.user_id.parse().map_err(|_| {
                CoreError::Other("corrupt session metadata: invalid user id".into())
            })?,
            device_id: meta.device_id.as_str().into(),
        },
        tokens,
    };

    if let Err(error) = client.restore_session(session).await {
        tracing::warn!(%error, "session restore failed against the state store, falling back to interactive login");
        // Drop the client (and its open store handles) before discarding —
        // Windows can't delete files another handle has open. If the
        // removal still fails, the no-session branch at the top retries it
        // on the next launch.
        drop(client);
        let _ = std::fs::remove_file(&meta_path);
        discard_state_store(paths);
        return Ok(None);
    }

    let user_id = meta
        .user_id
        .parse()
        .map_err(|_| CoreError::Other("corrupt session metadata: invalid user id".into()))?;
    let device_id: OwnedDeviceId = meta.device_id.as_str().into();

    Ok(Some(RestoredSession { client, user_id, device_id }))
}

/// Interactive password login. `homeserver` may be a bare server name
/// (matrix-sdk resolves `.well-known` automatically) or a full URL.
pub async fn login_password(
    paths: &AppPaths,
    homeserver: &str,
    username: &str,
    password: Zeroizing<String>,
) -> CoreResult<RestoredSession> {
    let client = build_client(homeserver, &paths.state_store_dir()).await?;

    client
        .matrix_auth()
        .login_username(username, password.as_str())
        .initial_device_display_name(DEVICE_DISPLAY_NAME)
        .send()
        .await
        .map_err(|e| CoreError::LoginFailed(e.to_string()))?;

    persist_session(paths, &client).await?;
    let session = client
        .matrix_auth()
        .session()
        .ok_or_else(|| CoreError::Other("login succeeded but no session was created".into()))?;

    Ok(RestoredSession {
        user_id: session.meta.user_id,
        device_id: session.meta.device_id,
        client,
    })
}

/// SSO login via the system browser. `matrix-sdk`'s `login_sso` builder runs
/// its own local loopback HTTP server internally to catch the redirect
/// callback; we only need to hand it a closure that opens the SSO URL.
/// `identity_provider_id` selects a specific provider when the homeserver
/// advertises more than one (e.g. picking "Forum Account" out of several
/// options); pass `None` when there's only a single generic SSO flow.
pub async fn login_sso(
    paths: &AppPaths,
    homeserver: &str,
    identity_provider_id: Option<&str>,
) -> CoreResult<RestoredSession> {
    let client = build_client(homeserver, &paths.state_store_dir()).await?;

    let mut builder = client
        .matrix_auth()
        .login_sso(|sso_url| async move { open::that(sso_url).map_err(matrix_sdk::Error::Io) })
        .initial_device_display_name(DEVICE_DISPLAY_NAME);
    if let Some(idp) = identity_provider_id {
        builder = builder.identity_provider_id(idp);
    }
    builder.send().await.map_err(|e| CoreError::LoginFailed(e.to_string()))?;

    persist_session(paths, &client).await?;
    let session = client
        .matrix_auth()
        .session()
        .ok_or_else(|| CoreError::Other("SSO login succeeded but no session was created".into()))?;

    Ok(RestoredSession {
        user_id: session.meta.user_id,
        device_id: session.meta.device_id,
        client,
    })
}

pub async fn logout(paths: &AppPaths, client: &Client) -> CoreResult<()> {
    let _ = client.matrix_auth().logout().await;

    if let Some(session) = client.matrix_auth().session() {
        let entry = keyring::Entry::new(KEYRING_SERVICE, session.meta.user_id.as_str())?;
        let _ = entry.delete_credential();
    }

    // Best-effort (no `?`): a failed session.json removal must not skip the
    // store discard below — try_restore self-heals either leftover at the
    // next launch anyway.
    let meta_path = paths.root.join("session.json");
    if meta_path.exists() {
        if let Err(error) = std::fs::remove_file(&meta_path) {
            tracing::warn!(%error, "failed to remove session.json during logout");
        }
    }

    // The device was just deleted server-side, so the store's crypto account
    // is unusable — and left in place it would break the next login with
    // MismatchedAccount (the new login gets a fresh device id). NOTE: while
    // the process still holds Client clones (worker + detached forwarders),
    // sqlite's open handles make this removal fail on Windows — the
    // no-session branch in try_restore retries it at next startup, before
    // any client exists.
    discard_state_store(paths);

    Ok(())
}

/// Tokens saved before the rename to ThornyChat live under the old keyring
/// service name. On a miss under the new name, copy them across (deleting
/// the old entry only once the copy is confirmed written, so a failed write
/// retries next launch instead of losing the session).
fn migrate_legacy_keyring_entry(new_entry: &keyring::Entry, user_id: &str) -> Option<String> {
    let old_entry = keyring::Entry::new(LEGACY_KEYRING_SERVICE, user_id).ok()?;
    let tokens = old_entry.get_password().ok()?;
    if new_entry.set_password(&tokens).is_ok() {
        let _ = old_entry.delete_credential();
        tracing::info!("migrated session tokens from legacy Synapse keyring entry");
    }
    Some(tokens)
}

/// Removes the on-disk state/crypto store. Called whenever the saved session
/// is gone or unusable: the store's crypto account is bound to the old device
/// id, and building a new login on top of it fails with `MismatchedAccount`.
fn discard_state_store(paths: &AppPaths) {
    let store_dir = paths.state_store_dir();
    if store_dir.exists() {
        if let Err(error) = std::fs::remove_dir_all(&store_dir) {
            tracing::warn!(%error, "failed to remove stale state store");
        }
    }
}

async fn build_client(homeserver: &str, store_dir: &Path) -> CoreResult<Client> {
    let client = Client::builder()
        .server_name_or_homeserver_url(homeserver)
        .sqlite_store(store_dir, None)
        .build()
        .await?;
    Ok(client)
}

async fn persist_session(paths: &AppPaths, client: &Client) -> CoreResult<()> {
    let session = client
        .matrix_auth()
        .session()
        .ok_or_else(|| CoreError::Other("no session to persist after login".into()))?;

    let meta = SessionMeta {
        homeserver: client.homeserver().to_string(),
        user_id: session.meta.user_id.to_string(),
        device_id: session.meta.device_id.to_string(),
    };
    std::fs::write(paths.root.join("session.json"), serde_json::to_vec(&meta)?)?;

    let entry = keyring::Entry::new(KEYRING_SERVICE, &meta.user_id)?;
    entry.set_password(&serde_json::to_string(&session.tokens)?)?;

    Ok(())
}

/// Convenience used by the sync worker bootstrap: try a saved session first,
/// signalling to the caller (via `Ok(None)`) that interactive login is needed.
pub async fn restore_or_none(paths: &AppPaths) -> CoreResult<Option<RestoredSession>> {
    try_restore(paths).await
}

pub fn default_sync_settings() -> SyncSettings {
    SyncSettings::default()
}

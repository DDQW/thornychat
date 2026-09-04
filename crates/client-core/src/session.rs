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
    cross_process_lock::CrossProcessLockConfig,
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

    // A *store* failure here can only be self-healed by starting clean:
    // interactive login builds on the SAME store and would fail identically,
    // wedging startup forever. So a corrupt sqlite store still discards.
    //
    // Everything else must not. `build_client` does reach the network even
    // when handed a full homeserver URL — the comment that used to sit here
    // claimed otherwise, and on 2026-08-24 a `error sending request` at launch
    // took that branch and deleted the session, the cache, and this device's
    // E2EE identity. Launching before the network is up (autostart at boot, or
    // straight after a resume) is exactly when that happens, so the cost of
    // guessing wrong here is high and recurring.
    let client = match build_client(&meta.homeserver, &paths.state_store_dir()).await {
        Ok(client) => client,
        Err(error) if error.is_unusable_store() => {
            tracing::warn!(%error, "state store unusable, falling back to interactive login");
            let _ = std::fs::remove_file(&meta_path);
            discard_state_store(paths);
            return Ok(None);
        }
        Err(error) => {
            // Session left untouched: the caller retries, and a later attempt
            // (or the next launch) restores it.
            tracing::warn!(
                %error,
                "could not build the client; keeping the saved session for a retry"
            );
            return Err(error);
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
        let error = CoreError::from(error);
        // The mirror image of the build step above, and a blacklist rather
        // than a whitelist because `matrix_sdk::Error` is broad: almost
        // everything here really is a local problem that only a clean store
        // fixes (a crypto account bound to a dead device id, `MismatchedAccount`,
        // a half-written store). A transport error is the one thing that
        // certainly isn't, and must never cost the session.
        if error.is_transient_transport() {
            tracing::warn!(
                %error,
                "could not reach the homeserver while restoring; keeping the saved session"
            );
            return Err(error);
        }
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
    let mut client = build_client(homeserver, &paths.state_store_dir()).await?;

    if let Err(error) = client
        .matrix_auth()
        .login_username(username, password.as_str())
        .initial_device_display_name(DEVICE_DISPLAY_NAME)
        .send()
        .await
    {
        if !matches!(error, matrix_sdk::Error::CryptoStoreError(_)) {
            return Err(CoreError::LoginFailed(error.to_string()));
        }
        // The store is bound to a different device's crypto account — most
        // likely a same-process `logout` couldn't discard it (Windows kept
        // it open via other live `Client` clones; see `logout`'s doc
        // comment). Drop this client to release its handles, discard the
        // stale store, and retry once against a clean one.
        tracing::warn!(%error, "stale crypto store left from an earlier logout; discarding and retrying login");
        drop(client);
        discard_state_store(paths);
        client = build_client(homeserver, &paths.state_store_dir()).await?;
        client
            .matrix_auth()
            .login_username(username, password.as_str())
            .initial_device_display_name(DEVICE_DISPLAY_NAME)
            .send()
            .await
            .map_err(|e| CoreError::LoginFailed(e.to_string()))?;
    }

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
    let mut client = build_client(homeserver, &paths.state_store_dir()).await?;

    let sso_login = |client: &Client| {
        let mut builder = client
            .matrix_auth()
            .login_sso(|sso_url| async move { open::that(sso_url).map_err(matrix_sdk::Error::Io) })
            .initial_device_display_name(DEVICE_DISPLAY_NAME);
        if let Some(idp) = identity_provider_id {
            builder = builder.identity_provider_id(idp);
        }
        builder.send()
    };

    if let Err(error) = sso_login(&client).await {
        if !matches!(error, matrix_sdk::Error::CryptoStoreError(_)) {
            return Err(CoreError::LoginFailed(error.to_string()));
        }
        // Same self-heal as `login_password` — see its comment. Note this
        // means re-opening the browser for a second SSO round trip; that's
        // still far better than a hard failure with no way to recover.
        tracing::warn!(%error, "stale crypto store left from an earlier logout; discarding and retrying login");
        drop(client);
        discard_state_store(paths);
        client = build_client(homeserver, &paths.state_store_dir()).await?;
        sso_login(&client).await.map_err(|e| CoreError::LoginFailed(e.to_string()))?;
    }

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

/// Builds the client with the cross-process store lock **disabled**.
///
/// This is the single biggest reduction in idle disk writes available here, and
/// it costs nothing behaviourally.
///
/// `ClientBuilder` defaults to `CrossProcessLockConfig::MultiProcess`
/// (`matrix-sdk/src/client/builder/mod.rs`), which arms a lease-renewal task
/// with `LEASE_DURATION_MS = 500` and `EXTEND_LEASE_EVERY_MS = 50`
/// (`matrix-sdk-common/src/cross_process_lock.rs`). That is a committed
/// `INSERT … ON CONFLICT DO UPDATE` on the event-cache store's `lease_locks`
/// table **twenty times a second, forever**, whether or not anything happened.
/// On top of it, the event-cache lock is acquired *before* the "nothing
/// changed" early-return in `handle_timeline_inner`, so every room in every
/// sync response pays another one even when its update is empty.
///
/// Measured here: the app wrote 129.9 KB/s and 64.6 write-ops/s while
/// completely idle, with the event-cache WAL touched in 100% of two-second
/// sampling windows and pinned at SQLite's 1000-page checkpoint threshold.
///
/// `SingleProcess` makes `try_lock_once` a no-op that touches no database and
/// spawns no renew task. That is the honest description of this app: one
/// instance owns a profile at a time, and nothing here calls
/// `Encryption::enable_cross_process_store_lock`, so no code path depends on
/// the multi-process behaviour.
///
/// **Do not set this back to `MultiProcess` without a reason.** Its only
/// purpose is letting a second process share these databases safely — if that
/// ever becomes a goal (a background notification helper, say), this is the
/// line that has to change first, and the write cost comes back with it.
///
/// Note what is deliberately *not* done: the state store stays on disk. It
/// looks like a cache but is not one — `client.rooms()` reads an in-memory map
/// whose only cold-start seed is `load_rooms()` from that store, and the
/// sidebar's names, heroes and member counts are all persisted `RoomInfo`
/// fields. Running it from memory leaves the app with unnamed rooms and no
/// spaces, which is exactly what happened when it was tried. It is also only
/// ~7% of the write traffic, so there is nothing to gain.
async fn build_client(homeserver: &str, store_dir: &Path) -> CoreResult<Client> {
    let client = Client::builder()
        .server_name_or_homeserver_url(homeserver)
        .cross_process_store_config(CrossProcessLockConfig::SingleProcess)
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

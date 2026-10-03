//! The seam between the webview and `client-core`.
//!
//! `client-core` already speaks only in plain serializable commands and
//! events, so this layer is thin: the frontend calls [`dispatch`] with a
//! `ClientCommand` and listens on one [`Channel`] for `ClientEvent`s. Login and
//! session restore are separate commands because they *create* the running
//! client that everything else talks to.
//!
//! Requests are correlated by the `request_id` the frontend puts in each
//! command (`crypto.randomUUID()`), which comes back on `CommandSucceeded` /
//! `CommandFailed`; nothing here needs to track them.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use client_core::{ClientCommand, ClientEvent, RunningClient};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

pub struct Bridge {
    pub profile: String,
    /// Where `tcmedia` serves Matrix media from (and caches it to).
    pub media_dir: PathBuf,
    /// The profile's data directory (session metadata, per-account state).
    pub profile_dir: PathBuf,
    /// Twemoji SVGs and proxied card images.
    pub emoji_dir: PathBuf,
    /// Lanczos-upscaled copies for the lightbox (see `upscale.rs`).
    pub upscale_dir: PathBuf,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
    running: Mutex<Option<Arc<RunningClient>>>,
    sink: Mutex<Option<Channel<ClientEvent>>>,
}

impl Bridge {
    /// Builds the bridge and starts the task that forwards worker events to
    /// the webview. Must be called from inside the async runtime.
    pub fn new(profile: String, app: AppHandle) -> Arc<Self> {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (media_dir, emoji_dir, profile_dir) = match client_core::store::AppPaths::for_profile(&profile) {
            Ok(paths) => (paths.media_cache_dir(), paths.emoji_cache_dir(), paths.root.clone()),
            Err(error) => {
                tracing::error!(%error, "could not resolve the profile directories");
                let fallback = std::env::temp_dir().join("thornychat-cache");
                (fallback.join("media"), fallback.join("emoji"), fallback.join("profile"))
            }
        };
        // A sibling of the media cache, not inside it: that one's size cap
        // counts only its own files, and these have a cap of their own.
        let upscale_dir = media_dir.with_file_name("upscale-cache");
        let bridge = Arc::new(Self {
            profile,
            media_dir,
            profile_dir,
            emoji_dir,
            upscale_dir,
            event_tx,
            running: Mutex::new(None),
            sink: Mutex::new(None),
        });
        tauri::async_runtime::spawn(pump(bridge.clone(), event_rx, app));
        bridge
    }

    /// The running client, if a session is active.
    pub fn running(&self) -> Option<Arc<RunningClient>> {
        self.running.lock().expect("bridge lock poisoned").clone()
    }

    fn adopt(&self, running: RunningClient) -> SessionInfo {
        let info = SessionInfo::of(&running);
        *self.running.lock().expect("bridge lock poisoned") = Some(Arc::new(running));
        info
    }

    /// Closes the command channel so the sync worker winds down (it leaves any
    /// call this device is in on the way out) and waits up to `grace` for it.
    pub async fn shutdown(&self, grace: std::time::Duration) {
        let Some(running) = self.running.lock().expect("bridge lock poisoned").take() else { return };
        // Anyone still holding a clone (a media request in flight) keeps the
        // channel open; the worker then ends with the process instead.
        let Ok(RunningClient { client: _client, cmd_tx, worker_handle }) = Arc::try_unwrap(running) else {
            return;
        };
        drop(cmd_tx);
        if tokio::time::timeout(grace, worker_handle).await.is_err() {
            tracing::warn!("the sync worker did not stop in time; exiting anyway");
        }
    }

    fn send(&self, command: ClientCommand) -> Result<(), String> {
        let running = self.running().ok_or("not signed in")?;
        running.cmd_tx.send(command).map_err(|_| "the sync worker has stopped".to_string())
    }
}

/// Forwards every worker event to the webview, with the few side effects that
/// belong to the shell rather than the page.
async fn pump(bridge: Arc<Bridge>, mut events: mpsc::UnboundedReceiver<ClientEvent>, app: AppHandle) {
    while let Some(event) = events.recv().await {
        match &event {
            ClientEvent::LoggedOut => {
                *bridge.running.lock().expect("bridge lock poisoned") = None;
            }
            // The page streams media through the `tcmedia` protocol; the bytes
            // variant exists for the old in-process UI and would serialize as
            // a giant JSON array.
            ClientEvent::MediaFetched { .. } => continue,
            ClientEvent::Notification(notification) => notify(&app, notification),
            _ => {}
        }
        let sink = bridge.sink.lock().expect("bridge lock poisoned").clone();
        if let Some(sink) = sink {
            if let Err(error) = sink.send(event) {
                tracing::debug!(%error, "could not deliver an event to the webview");
            }
        }
    }
}

/// A toast for a push-rule hit — skipped while the window has focus, since
/// the user is already looking at the app. Clicking it opens the room.
fn notify(app: &AppHandle, notification: &client_core::events::NotificationEvent) {
    let focused = app.get_webview_window("main").and_then(|window| window.is_focused().ok()).unwrap_or(false);
    if focused {
        return;
    }
    crate::toast::show(app, notification.room_id.clone(), notification.title.clone(), notification.body.clone());
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub user_id: String,
    pub device_id: Option<String>,
    pub homeserver: String,
}

impl SessionInfo {
    fn of(running: &RunningClient) -> Self {
        Self {
            user_id: running.client.user_id().map(ToString::to_string).unwrap_or_default(),
            device_id: running.client.device_id().map(ToString::to_string),
            homeserver: running.client.homeserver().to_string(),
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RestoreOutcome {
    /// No saved session; show the login screen.
    None,
    Started { session: SessionInfo },
    /// Restoring failed. `retryable` means the homeserver was unreachable —
    /// the session is still on disk, untouched, so a later attempt can still
    /// succeed.
    Failed { message: String, retryable: bool },
}

#[derive(Serialize)]
pub struct LoginFlowsDto {
    pub supports_password: bool,
    pub supports_sso: bool,
    pub sso_providers: Vec<SsoProviderDto>,
}

#[derive(Serialize)]
pub struct SsoProviderDto {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct OwnProfile {
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[tauri::command]
pub fn attach_events(bridge: State<'_, Arc<Bridge>>, on_event: Channel<ClientEvent>) {
    *bridge.sink.lock().expect("bridge lock poisoned") = Some(on_event);
}

#[tauri::command]
pub async fn session_restore(bridge: State<'_, Arc<Bridge>>) -> Result<RestoreOutcome, String> {
    // A page reload (dev only) finds the session already running: report it
    // rather than opening the sqlite store a second time.
    if let Some(running) = bridge.running() {
        return Ok(RestoreOutcome::Started { session: SessionInfo::of(&running) });
    }
    match client_core::try_start(&bridge.profile, bridge.event_tx.clone()).await {
        Ok(Some(running)) => Ok(RestoreOutcome::Started { session: bridge.adopt(running) }),
        Ok(None) => Ok(RestoreOutcome::None),
        Err(error) => {
            tracing::warn!(%error, "could not restore the saved session");
            Ok(RestoreOutcome::Failed {
                retryable: error.is_transient_transport(),
                message: error.to_string(),
            })
        }
    }
}

#[tauri::command]
pub async fn discover_homeserver(homeserver: String) -> Result<LoginFlowsDto, String> {
    let flows = client_core::session::discover_login_flows(&homeserver)
        .await
        .map_err(|error| error.to_string())?;
    Ok(LoginFlowsDto {
        supports_password: flows.supports_password,
        supports_sso: flows.supports_sso,
        sso_providers: flows
            .sso_providers
            .into_iter()
            .map(|provider| SsoProviderDto { id: provider.id, name: provider.name })
            .collect(),
    })
}

#[tauri::command]
pub async fn login_password(
    bridge: State<'_, Arc<Bridge>>,
    homeserver: String,
    username: String,
    password: String,
) -> Result<SessionInfo, String> {
    ensure_signed_out(&bridge)?;
    let running = client_core::start_with_password(
        &bridge.profile,
        &homeserver,
        &username,
        Zeroizing::new(password),
        bridge.event_tx.clone(),
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(bridge.adopt(running))
}

#[tauri::command]
pub async fn login_sso(
    bridge: State<'_, Arc<Bridge>>,
    homeserver: String,
    identity_provider_id: Option<String>,
) -> Result<SessionInfo, String> {
    ensure_signed_out(&bridge)?;
    let running = client_core::start_with_sso(
        &bridge.profile,
        &homeserver,
        identity_provider_id.as_deref(),
        bridge.event_tx.clone(),
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(bridge.adopt(running))
}

fn ensure_signed_out(bridge: &Bridge) -> Result<(), String> {
    if bridge.running().is_some() {
        return Err("already signed in".to_string());
    }
    Ok(())
}

/// Sends one command to the sync worker. Commands are deliberately not logged
/// with `{:?}`: `EnableRecovery` / `RestoreFromBackup` carry secrets.
#[tauri::command]
pub fn dispatch(bridge: State<'_, Arc<Bridge>>, command: ClientCommand) -> Result<(), String> {
    if tracing::enabled!(tracing::Level::DEBUG) {
        tracing::debug!(command = command_name(&command), "dispatch");
    }
    bridge.send(command)
}

fn command_name(command: &ClientCommand) -> String {
    serde_json::to_value(command)
        .ok()
        .and_then(|value| value.get("type").and_then(|name| name.as_str().map(str::to_owned)))
        .unwrap_or_default()
}

#[tauri::command]
pub async fn own_profile(bridge: State<'_, Arc<Bridge>>) -> Result<OwnProfile, String> {
    let running = bridge.running().ok_or("not signed in")?;
    let account = running.client.account();
    let display_name = account.get_display_name().await.map_err(|error| error.to_string())?;
    let avatar_url = account.get_avatar_url().await.map_err(|error| error.to_string())?;
    Ok(OwnProfile { display_name, avatar_url: avatar_url.map(|url| url.to_string()) })
}

/// Uploads a file as a message. The bytes arrive as the raw IPC body (no JSON
/// array of numbers), with the rest of the request in percent-encoded headers
/// because a raw body has nowhere else to put them.
#[tauri::command]
pub fn send_attachment(
    bridge: State<'_, Arc<Bridge>>,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("attachment bytes must be sent as a raw body".into());
    };
    let headers = request.headers();
    let header = |name: &str| -> Option<String> {
        let value = headers.get(name)?.to_str().ok()?;
        Some(percent_decode(value))
    };
    let room_id = header("x-room-id").ok_or("missing room id")?;
    let filename = header("x-filename").ok_or("missing filename")?;
    let request_id = header("x-request-id")
        .and_then(|id| id.parse().ok())
        .ok_or("missing or invalid request id")?;
    let mentioned_user_ids: Vec<String> = header("x-mentions")
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    bridge.send(ClientCommand::SendAttachment {
        room_id,
        filename,
        bytes: bytes.clone(),
        mime: header("x-mime").filter(|mime| !mime.is_empty()).unwrap_or_else(|| "application/octet-stream".into()),
        caption: header("x-caption").filter(|caption| !caption.is_empty()),
        mentioned_user_ids,
        reply_to_event_id: header("x-reply-to").filter(|id| !id.is_empty()),
        request_id,
    })
}

/// Decodes the `encodeURIComponent` output the frontend puts in headers.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            // Slice the bytes, not the str: a multi-byte character right after
            // the `%` would put a str slice off a char boundary.
            let escaped = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            if let Some(byte) = escaped {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding_matches_encode_uri_component() {
        assert_eq!(percent_decode("plain.txt"), "plain.txt");
        assert_eq!(percent_decode("a%20b%2Fc"), "a b/c");
        // "ü" and an emoji, as encodeURIComponent writes them (UTF-8 bytes).
        assert_eq!(percent_decode("%C3%BCber%20%F0%9F%98%80"), "über 😀");
        assert_eq!(percent_decode("100%25"), "100%");
    }

    #[test]
    fn malformed_escapes_pass_through_instead_of_panicking() {
        assert_eq!(percent_decode("%"), "%");
        assert_eq!(percent_decode("%4"), "%4");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode(""), "");
        // A multi-byte character straight after the `%` must not panic.
        assert_eq!(percent_decode("%aé"), "%aé");
        assert_eq!(percent_decode("%é"), "%é");
    }

    #[test]
    fn command_names_come_from_the_wire_tag() {
        assert_eq!(command_name(&ClientCommand::Logout), "Logout");
        let secret = ClientCommand::RestoreFromBackup {
            recovery_key: Zeroizing::new("hunter2".into()),
            request_id: uuid::Uuid::nil(),
        };
        assert_eq!(command_name(&secret), "RestoreFromBackup");
    }
}

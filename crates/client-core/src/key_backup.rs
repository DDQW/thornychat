//! Cross-signing bootstrap and key backup/recovery.
//!
//! Cross-signing bootstrap runs once automatically after login/restore.
//! Homeservers commonly require interactive auth (UIAA) to upload the new
//! cross-signing keys — rather than special-casing password vs SSO vs
//! anything else, this uses the UIAA **fallback web page**, which handles
//! whatever auth stage the server demands uniformly: open it in the
//! system browser, let the user complete it there, then acknowledge with
//! `AuthData::FallbackAcknowledgement` and retry.
//!
//! Recovery (the "recovery key" / secret-storage system) lets a new device
//! recover room keys for encrypted history, or sets itself up for the
//! first time. All passphrase/recovery-key strings are `Zeroizing` from
//! capture through to consumption and never logged.

use matrix_sdk::encryption::recovery::{EnableProgress, RecoveryState};
use matrix_sdk::ruma::api::client::uiaa::{AuthData, FallbackAcknowledgement, UiaaInfo};
use matrix_sdk::Client;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use zeroize::Zeroizing;

use crate::commands::RequestId;
use crate::events::{ClientEvent, RecoveryEnableStage};

/// Attempts to bootstrap cross-signing if this account doesn't have it yet.
/// Returns the pending UIAA session id if the homeserver demands
/// interactive auth to complete it (the caller should stash this and hand
/// it to [`retry_with_fallback`] once the user has completed the fallback
/// web page); returns `None` if bootstrap already succeeded or hard-failed
/// (both cases already reported via `event_tx`).
pub async fn bootstrap_cross_signing(
    client: &Client,
    event_tx: &mpsc::UnboundedSender<ClientEvent>,
) -> Option<String> {
    match client.encryption().bootstrap_cross_signing_if_needed(None).await {
        Ok(()) => {
            let _ = event_tx.send(ClientEvent::CrossSigningBootstrapDone);
            None
        }
        Err(error) => match error.as_uiaa_response().and_then(|info| fallback_url_and_session(client, info)) {
            Some((url, session)) => {
                let _ = event_tx.send(ClientEvent::CrossSigningBootstrapNeedsFallback { url });
                Some(session)
            }
            None => {
                let _ = event_tx.send(ClientEvent::CrossSigningBootstrapFailed { reason: error.to_string() });
                None
            }
        },
    }
}

/// Completes a bootstrap that was waiting on the UIAA fallback page, using
/// the session id captured from the original failure. Like
/// [`bootstrap_cross_signing`], returns a pending UIAA session id if the
/// server *still* demands auth (user clicked retry before completing the
/// fallback page, or the session expired) — the caller must stash it so the
/// flow can be retried again instead of dead-ending until app restart.
pub async fn retry_with_fallback(
    client: &Client,
    session: String,
    event_tx: &mpsc::UnboundedSender<ClientEvent>,
) -> Option<String> {
    let auth_data = AuthData::FallbackAcknowledgement(FallbackAcknowledgement::new(session));
    match client.encryption().bootstrap_cross_signing(Some(auth_data)).await {
        Ok(()) => {
            let _ = event_tx.send(ClientEvent::CrossSigningBootstrapDone);
            None
        }
        Err(error) => match error.as_uiaa_response().and_then(|info| fallback_url_and_session(client, info)) {
            Some((url, new_session)) => {
                let _ = event_tx.send(ClientEvent::CrossSigningBootstrapNeedsFallback { url });
                Some(new_session)
            }
            None => {
                let _ = event_tx.send(ClientEvent::CrossSigningBootstrapFailed { reason: error.to_string() });
                None
            }
        },
    }
}

fn fallback_url_and_session(client: &Client, info: &UiaaInfo) -> Option<(String, String)> {
    let session = info.session.clone()?;
    let stage = info.flows.first()?.stages.first()?;
    let url = format!(
        "{}_matrix/client/v3/auth/{}/fallback/web?session={session}",
        client.homeserver(),
        stage.as_str(),
    );
    Some((url, session))
}

/// Checked once after login/restore: tells the UI whether recovery needs
/// setting up for the first time, or needs this device's recovery key to
/// unlock already-configured secret storage.
pub async fn check_recovery_state(client: &Client, event_tx: &mpsc::UnboundedSender<ClientEvent>) {
    // The recovery state starts as `Unknown` and is only settled by the
    // SDK's detached e2ee setup task (spawned at login/restore, does its own
    // network round trips). A one-shot read racing that task would see
    // `Unknown`, emit nothing, and the user would never be prompted for
    // their recovery key — wait for it to settle first.
    client.encryption().wait_for_e2ee_initialization_tasks().await;
    match client.encryption().recovery().state() {
        RecoveryState::Disabled => {
            let _ = event_tx.send(ClientEvent::RecoverySetupNeeded);
        }
        RecoveryState::Incomplete => {
            let _ = event_tx.send(ClientEvent::KeyBackupNeedsRecovery);
        }
        RecoveryState::Enabled | RecoveryState::Unknown => {}
    }
}

/// Generates a fresh recovery key and enables secret storage + key backup.
/// The generated key is emitted via `ClientEvent::RecoveryEnabled` exactly
/// once — the UI must show it to the user with an explicit "I've saved it"
/// confirmation, since it can never be recovered again if lost.
pub async fn enable_recovery(
    client: &Client,
    passphrase: Option<Zeroizing<String>>,
    event_tx: &mpsc::UnboundedSender<ClientEvent>,
    request_id: RequestId,
) {
    let recovery = client.encryption().recovery();
    let enable = recovery.enable().wait_for_backups_to_upload();
    let enable = match &passphrase {
        Some(p) => enable.with_passphrase(p.as_str()),
        None => enable,
    };

    let mut progress_stream = enable.subscribe_to_progress();
    let progress_tx = event_tx.clone();
    let progress_task = tokio::spawn(async move {
        while let Some(Ok(progress)) = progress_stream.next().await {
            let stage = match progress {
                EnableProgress::Starting => Some(RecoveryEnableStage::Starting),
                EnableProgress::CreatingBackup => Some(RecoveryEnableStage::CreatingBackup),
                EnableProgress::CreatingRecoveryKey => Some(RecoveryEnableStage::CreatingRecoveryKey),
                EnableProgress::BackingUp(_) => Some(RecoveryEnableStage::BackingUp),
                EnableProgress::Done { .. } | EnableProgress::RoomKeyUploadError => None,
            };
            if let Some(stage) = stage {
                let _ = progress_tx.send(ClientEvent::RecoveryEnableProgress(stage));
            }
        }
    });

    match enable.await {
        Ok(recovery_key) => {
            let _ = event_tx.send(ClientEvent::RecoveryEnabled { recovery_key });
            let _ = event_tx.send(ClientEvent::CommandSucceeded { request_id });
        }
        Err(error) => {
            let _ = event_tx.send(ClientEvent::RecoveryEnableFailed { reason: error.to_string() });
            let _ = event_tx.send(ClientEvent::CommandFailed { request_id, error: error.to_string() });
        }
    }
    progress_task.abort();
}

/// Restores room keys on this device using an existing recovery key (or
/// passphrase — matrix-sdk accepts either as the same string).
pub async fn restore_from_backup(
    client: &Client,
    recovery_key: Zeroizing<String>,
    event_tx: &mpsc::UnboundedSender<ClientEvent>,
    request_id: RequestId,
) {
    match client.encryption().recovery().recover(recovery_key.as_str()).await {
        Ok(()) => {
            let _ = event_tx.send(ClientEvent::KeyBackupRestored);
            let _ = event_tx.send(ClientEvent::CommandSucceeded { request_id });
        }
        Err(error) => {
            let _ = event_tx.send(ClientEvent::KeyBackupFailed { reason: error.to_string() });
            let _ = event_tx.send(ClientEvent::CommandFailed { request_id, error: error.to_string() });
        }
    }
}

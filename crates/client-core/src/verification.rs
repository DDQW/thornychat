//! SAS device verification: a single active flow at a time, driving
//! `matrix_sdk::encryption::verification::{VerificationRequest, SasVerification}`
//! and forwarding transitions as `ClientEvent::VerificationStateChanged`.
//!
//! Both the request phase and the SAS phase are modeled as their own loop
//! racing the SDK's state-change stream against an `action_rx` channel the
//! worker forwards user actions (accept/confirm/reject/cancel) through —
//! see the project plan's "E2EE verification & key backup flow" section for
//! the transition contract this implements.

use matrix_sdk::encryption::verification::{
    SasState as SdkSasState, SasVerification, VerificationRequest, VerificationRequestState,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

use crate::events::{ClientEvent, SasState};

#[derive(Debug, Clone)]
pub enum VerificationAction {
    Accept,
    ConfirmMatch,
    RejectMatch,
    Cancel,
}

/// Owns the background task driving one verification flow. Dropping it
/// (e.g. when a new incoming request replaces it, or the worker shuts down)
/// aborts the task, which in turn drops the `VerificationRequest`/
/// `SasVerification` handles it was holding.
pub struct VerificationSession {
    action_tx: mpsc::UnboundedSender<VerificationAction>,
    task: JoinHandle<()>,
}

impl VerificationSession {
    pub fn send(&self, action: VerificationAction) {
        let _ = self.action_tx.send(action);
    }

    /// Whether this flow's background task is still running. A finished
    /// (done/cancelled) session is treated as free for a new incoming
    /// request to claim, even if the slot hasn't been explicitly cleared.
    pub fn is_active(&self) -> bool {
        !self.task.is_finished()
    }
}

impl Drop for VerificationSession {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn spawn(
    request: VerificationRequest,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) -> VerificationSession {
    let (action_tx, action_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(run(request, action_rx, event_tx));
    VerificationSession { action_tx, task }
}

/// What handling one `VerificationRequestState` decided: keep waiting,
/// hand over to the SAS phase, or end the flow.
enum RequestOutcome {
    Continue,
    Sas(SasVerification),
    Finished,
}

async fn handle_request_state(
    state: VerificationRequestState,
    request: &VerificationRequest,
    event_tx: &mpsc::UnboundedSender<ClientEvent>,
) -> RequestOutcome {
    match state {
        VerificationRequestState::Created { .. } => {
            send(event_tx, SasState::RequestSent);
            RequestOutcome::Continue
        }
        VerificationRequestState::Requested { .. } => {
            send(
                event_tx,
                SasState::RequestReceived { from_user_id: request.other_user_id().to_string() },
            );
            RequestOutcome::Continue
        }
        VerificationRequestState::Ready { .. } => {
            send(event_tx, SasState::Ready);
            // Either side may call start_sas(); if the other side already
            // did, this is a harmless no-op and we'll pick up their SAS via
            // Transitioned below.
            match request.start_sas().await {
                Ok(Some(sas)) => RequestOutcome::Sas(sas),
                _ => RequestOutcome::Continue,
            }
        }
        VerificationRequestState::Transitioned { verification } => match verification.sas() {
            Some(sas) => RequestOutcome::Sas(sas),
            None => RequestOutcome::Continue,
        },
        VerificationRequestState::Done => {
            send(event_tx, SasState::Done);
            RequestOutcome::Finished
        }
        VerificationRequestState::Cancelled(info) => {
            send(event_tx, SasState::Cancelled { reason: info.reason().to_string() });
            RequestOutcome::Finished
        }
    }
}

async fn run(
    request: VerificationRequest,
    mut action_rx: mpsc::UnboundedReceiver<VerificationAction>,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) {
    let mut request_stream = request.changes();

    // `changes()` only yields transitions that happen *after* subscribing
    // (eyeball `subscribe()` semantics), and the request is born with a
    // meaningful state before this task starts — incoming = `Requested`,
    // outgoing = `Created`. Handle the current state first, or the initial
    // accept/decline prompt (and RequestSent status) is never emitted.
    let initial = handle_request_state(request.state(), &request, &event_tx).await;

    let sas = match initial {
        RequestOutcome::Sas(sas) => Some(sas),
        RequestOutcome::Finished => return,
        RequestOutcome::Continue => loop {
            tokio::select! {
                action = action_rx.recv() => {
                    match action {
                        Some(VerificationAction::Accept) => {
                            let _ = request.accept().await;
                        }
                        Some(VerificationAction::Cancel) | None => {
                            let _ = request.cancel().await;
                            return;
                        }
                        _ => {}
                    }
                }
                state = request_stream.next() => {
                    let Some(state) = state else { return };
                    match handle_request_state(state, &request, &event_tx).await {
                        RequestOutcome::Sas(sas) => break Some(sas),
                        RequestOutcome::Finished => return,
                        RequestOutcome::Continue => {}
                    }
                }
            }
        },
    };

    let Some(sas) = sas else { return };
    run_sas(sas, action_rx, event_tx).await;
}

async fn run_sas(
    sas: SasVerification,
    mut action_rx: mpsc::UnboundedReceiver<VerificationAction>,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) {
    let mut sas_stream = sas.changes();

    // A SAS handed over via `Transitioned` (the other side sent
    // m.key.verification.start) is born in `Started`, and the SDK does NOT
    // auto-accept — without an accept() no keys are ever exchanged and both
    // sides hang until timeout. Our own start_sas() yields `Created`, where
    // accepting is the other side's job.
    if matches!(sas.state(), SdkSasState::Started { .. }) && !sas.we_started() {
        let _ = sas.accept().await;
    }

    loop {
        tokio::select! {
            action = action_rx.recv() => {
                match action {
                    Some(VerificationAction::ConfirmMatch) => {
                        let _ = sas.confirm().await;
                    }
                    Some(VerificationAction::RejectMatch) => {
                        let _ = sas.mismatch().await;
                        return;
                    }
                    Some(VerificationAction::Cancel) | None => {
                        let _ = sas.cancel().await;
                        return;
                    }
                    Some(VerificationAction::Accept) => {
                        // No-op unless the SAS is in an acceptable state.
                        let _ = sas.accept().await;
                    }
                }
            }
            state = sas_stream.next() => {
                let Some(state) = state else { return };
                match state {
                    SdkSasState::KeysExchanged { emojis, .. } => {
                        if let Some(short_auth_string) = emojis {
                            let pairs = short_auth_string
                                .emojis
                                .iter()
                                .map(|e| (e.symbol.to_string(), e.description.to_string()))
                                .collect();
                            send(&event_tx, SasState::EmojisReady(pairs));
                        }
                    }
                    SdkSasState::Confirmed => {
                        send(&event_tx, SasState::WaitingForOtherPartyConfirmation);
                    }
                    SdkSasState::Done { .. } => {
                        send(&event_tx, SasState::Done);
                        return;
                    }
                    SdkSasState::Cancelled(info) => {
                        send(&event_tx, SasState::Cancelled { reason: info.reason().to_string() });
                        return;
                    }
                    SdkSasState::Started { .. } => {
                        // Both-sides-started tiebreak: the SDK can replace
                        // our SAS with the remote one mid-flow — accept it.
                        if !sas.we_started() {
                            let _ = sas.accept().await;
                        }
                    }
                    SdkSasState::Created { .. } | SdkSasState::Accepted { .. } => {}
                }
            }
        }
    }
}

fn send(event_tx: &mpsc::UnboundedSender<ClientEvent>, state: SasState) {
    let _ = event_tx.send(ClientEvent::VerificationStateChanged(state));
}

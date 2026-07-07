//! MatrixRTC call signaling (MSC3401/MSC4143): active-call detection with
//! participant rosters, and join/leave by publishing this device's call
//! membership state event. Phase 5, deliberately scoped to signaling only —
//! matrix-sdk 0.13 ships the MatrixRTC event types and keeps per-room
//! membership state fresh (sliding sync's default `required_state` includes
//! `m.call.member` with state key `"*"`, unlike the emoji-pack/power-tag
//! custom state), but has no media stack. Actual audio needs a client for
//! the call's LiveKit focus — that lives behind [`webrtc_session`] later;
//! memberships published here are real either way (Element Call users see
//! this device in the call).

pub mod webrtc_session;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use matrix_sdk::deserialized_responses::RawSyncOrStrippedState;
use matrix_sdk::ruma::api::client::delayed_events::{
    delayed_state_event, update_delayed_event, DelayParameters,
};
use matrix_sdk::ruma::api::client::discovery::discover_homeserver::{self, RtcFocusInfo};
use matrix_sdk::ruma::events::call::member::{
    ActiveFocus, ActiveLivekitFocus, Application, CallApplicationContent, CallMemberEventContent,
    CallMemberStateKey, CallScope, Focus, LivekitFocus,
};
use matrix_sdk::ruma::events::call::notify::{ApplicationType, CallNotifyEventContent, NotifyType};
use matrix_sdk::ruma::events::{Mentions, SyncStateEvent};
use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, OwnedRoomId, RoomId};
use matrix_sdk::{Client, Room, RoomState};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use crate::commands::RequestId;
use crate::events::{CallParticipant, CallState, ClientEvent};

/// How long the server waits after our last heartbeat before it publishes
/// the scheduled "left" membership on its own (MSC4140) — the crash
/// cleanup that keeps this device from haunting a call forever. Matches
/// Element Call's 8s.
const DELAYED_LEAVE_TIMEOUT: Duration = Duration::from_secs(8);

/// A call this device has joined: the machinery keeping its server-side
/// delayed leave from firing while we're still in it. Dropping this stops
/// the heartbeats (and thereby lets the delayed leave fire, if one was
/// scheduled).
struct JoinedCall {
    delay_id: Option<String>,
    heartbeat: Option<JoinHandle<()>>,
}

impl Drop for JoinedCall {
    fn drop(&mut self) {
        if let Some(task) = &self.heartbeat {
            task.abort();
        }
    }
}

/// Owns the signaling side of calls: a sync event handler keeping the UI's
/// per-room call state fresh, plus join/leave (each emitting the usual
/// `CommandSucceeded`/`CommandFailed` correlation and an optimistic
/// `CallStateUpdated` so the banner flips without waiting for the sync
/// echo).
pub struct CallManager {
    client: Client,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
    joined: Mutex<HashMap<OwnedRoomId, JoinedCall>>,
}

impl CallManager {
    /// Registers the live-update handler, sweeps rooms for calls already in
    /// progress (state restored from the store before the handler existed),
    /// and returns the manager the command loop drives.
    pub fn spawn(client: Client, event_tx: mpsc::UnboundedSender<ClientEvent>) -> Arc<Self> {
        let manager = Arc::new(Self {
            client: client.clone(),
            event_tx: event_tx.clone(),
            joined: Mutex::new(HashMap::new()),
        });

        let handler_tx = event_tx.clone();
        // Weak: the handler registry lives on the Client, which the manager
        // owns — a strong Arc here would be a reference cycle.
        let weak = Arc::downgrade(&manager);
        client.add_event_handler(
            move |_ev: SyncStateEvent<CallMemberEventContent>, room: Room| {
                let event_tx = handler_tx.clone();
                let weak = weak.clone();
                async move {
                    let mut state = snapshot(&room).await;
                    // Our just-published membership isn't in the state store
                    // until the sync echo lands — without this, another
                    // participant's m.call.member event arriving in that
                    // window flips the banner back to "Join" and re-arms the
                    // button (a click then double-joins and orphans the
                    // first delayed leave). Only `joined` is OR'd in; the
                    // roster stays store truth and self-heals on echo.
                    if !state.joined {
                        if let Some(manager) = weak.upgrade() {
                            state.joined =
                                manager.joined.lock().await.contains_key(room.room_id());
                        }
                    }
                    let _ = event_tx.send(ClientEvent::CallStateUpdated(state));
                }
            },
        );

        tokio::spawn(async move {
            for room in client.rooms() {
                if room.state() == RoomState::Joined && room.has_active_room_call() {
                    let _ = event_tx.send(ClientEvent::CallStateUpdated(snapshot(&room).await));
                }
            }
        });

        manager
    }

    pub async fn join(&self, room_id: String, request_id: RequestId) {
        match self.try_join(&room_id).await {
            Ok(state) => {
                let _ = self.event_tx.send(ClientEvent::CallStateUpdated(state));
                let _ = self.event_tx.send(ClientEvent::CommandSucceeded { request_id });
            }
            Err(error) => {
                let _ = self.event_tx.send(ClientEvent::CommandFailed { request_id, error });
            }
        }
    }

    pub async fn leave(&self, room_id: String, request_id: RequestId) {
        match self.try_leave(&room_id).await {
            Ok(state) => {
                let _ = self.event_tx.send(ClientEvent::CallStateUpdated(state));
                let _ = self.event_tx.send(ClientEvent::CommandSucceeded { request_id });
            }
            Err(error) => {
                let _ = self.event_tx.send(ClientEvent::CommandFailed { request_id, error });
            }
        }
    }

    /// Best-effort leave of every call this device is in — logout/shutdown
    /// hygiene so other clients don't show us lingering in one.
    pub async fn leave_all(&self) {
        let room_ids: Vec<OwnedRoomId> = self.joined.lock().await.keys().cloned().collect();
        for room_id in room_ids {
            if let Err(error) = self.try_leave(room_id.as_str()).await {
                tracing::warn!(%room_id, %error, "failed to leave call during shutdown");
            }
        }
    }

    async fn try_join(&self, room_id: &str) -> Result<CallState, String> {
        let (room, user_id, device_id) = self.room_and_identity(room_id)?;

        let mut before = snapshot(&room).await;
        // The store lags our own membership until the sync echo — consult
        // the local join bookkeeping too, or a click inside that window
        // double-joins and orphans the first entry's delayed leave.
        if !before.joined && self.joined.lock().await.contains_key(room.room_id()) {
            before.joined = true;
        }
        if before.joined {
            // Already in (e.g. double-click, or state raced ahead) — treat
            // as success rather than churning the membership event.
            return Ok(before);
        }

        // Join the existing room call when there is one (same `call_id`,
        // same preferred foci — MatrixRTC converges everyone on the oldest
        // membership's foci); otherwise start one, advertising the
        // homeserver's MSC4143 foci so Element Call participants have an
        // SFU to converge on.
        let starting_fresh = before.participants.is_empty();
        let (call_id, mut foci) = existing_call_parameters(&room).await.unwrap_or_default();
        if foci.is_empty() {
            foci = well_known_foci(&self.client, room.room_id()).await;
        }

        let state_key =
            CallMemberStateKey::new(user_id.clone(), Some(device_id.clone()), false);

        // Crash cleanup first: with the delayed leave scheduled before the
        // join is published, a crash between the two can't strand a ghost
        // membership.
        let delay_id = schedule_delayed_leave(&room, &state_key).await;
        let heartbeat =
            delay_id.clone().map(|id| spawn_heartbeat(self.client.clone(), id));

        let content = CallMemberEventContent::new(
            Application::Call(CallApplicationContent::new(call_id.clone(), CallScope::Room)),
            device_id.clone(),
            ActiveFocus::Livekit(ActiveLivekitFocus::new()),
            foci,
            None,
        );
        if let Err(error) = room.send_state_event_for_key(&state_key, content).await {
            if let Some(task) = &heartbeat {
                task.abort();
            }
            // The already-scheduled delayed leave fires on its own and
            // publishes an empty membership — a no-op, since we never joined.
            return Err(error.to_string());
        }

        // Replacing a previous entry (re-join raced ahead of the sync echo):
        // its heartbeat stops on drop, but its server-side delayed leave
        // stays scheduled and would fire ~8s later, wiping the membership we
        // just published — cancel it explicitly. (`as_ref` because
        // JoinedCall has a manual Drop, so fields can't be moved out.)
        let displaced = self
            .joined
            .lock()
            .await
            .insert(room.room_id().to_owned(), JoinedCall { delay_id, heartbeat });
        if let Some(old_delay_id) = displaced.as_ref().and_then(|j| j.delay_id.clone()) {
            let request = update_delayed_event::unstable::Request::new(
                old_delay_id,
                update_delayed_event::unstable::UpdateAction::Cancel,
            );
            if let Err(error) = self.client.send(request).await {
                tracing::warn!(
                    %error,
                    "couldn't cancel a superseded delayed leave; it may briefly kick this membership"
                );
            }
        }

        // We started this call — ring/notify the room (MSC4075), otherwise
        // other clients show nothing until someone happens to open the room.
        if starting_fresh {
            let notify_type = if room.direct_targets_length() != 0 {
                NotifyType::Ring
            } else {
                NotifyType::Notify
            };
            let mut mentions = Mentions::new();
            mentions.room = true;
            let notify =
                CallNotifyEventContent::new(call_id, ApplicationType::Call, notify_type, mentions);
            if let Err(error) = room.send(notify).await {
                tracing::warn!(%error, "call started but m.call.notify failed to send");
            }
        }

        // Optimistic: the authoritative roster comes back through sync, but
        // the banner should flip the instant the join round trip succeeds.
        let mut state = before;
        state.joined = true;
        state.participants.push(CallParticipant {
            user_id: user_id.to_string(),
            device_id: device_id.to_string(),
        });
        Ok(state)
    }

    async fn try_leave(&self, room_id: &str) -> Result<CallState, String> {
        let (room, user_id, device_id) = self.room_and_identity(room_id)?;

        // Stop heartbeating before the explicit leave. Works even with no
        // entry — e.g. clearing a membership a previous crash of this
        // device left behind, which the banner still (correctly) shows as
        // "joined".
        let joined = self.joined.lock().await.remove(room.room_id());

        let state_key =
            CallMemberStateKey::new(user_id.clone(), Some(device_id.clone()), false);
        room.send_state_event_for_key(&state_key, CallMemberEventContent::new_empty(None))
            .await
            .map_err(|e| e.to_string())?;

        // The scheduled delayed leave is now redundant; cancelling it is
        // pure politeness (if this fails it fires once and re-publishes the
        // same empty membership — harmless).
        if let Some(delay_id) = joined.as_ref().and_then(|j| j.delay_id.clone()) {
            let request = update_delayed_event::unstable::Request::new(
                delay_id,
                update_delayed_event::unstable::UpdateAction::Cancel,
            );
            if let Err(error) = self.client.send(request).await {
                tracing::debug!(%error, "couldn't cancel the scheduled delayed leave");
            }
        }

        // Optimistic mirror of `try_join`: drop ourselves from the roster
        // rather than waiting for the sync echo.
        let mut state = snapshot(&room).await;
        state.joined = false;
        state
            .participants
            .retain(|p| !(p.user_id == user_id.as_str() && p.device_id == device_id.as_str()));
        Ok(state)
    }

    /// [`snapshot`] plus the local join bookkeeping — the state store lags
    /// our own membership until the sync echo lands, so a raw snapshot taken
    /// in that window (e.g. on room open) would show "Join" while we're in
    /// the call.
    pub async fn snapshot_for(&self, room: &Room) -> CallState {
        let mut state = snapshot(room).await;
        if !state.joined {
            state.joined = self.joined.lock().await.contains_key(room.room_id());
        }
        state
    }

    fn room_and_identity(
        &self,
        room_id: &str,
    ) -> Result<(Room, matrix_sdk::ruma::OwnedUserId, matrix_sdk::ruma::OwnedDeviceId), String>
    {
        let room_id = RoomId::parse(room_id).map_err(|_| "invalid room id".to_string())?;
        let room = self.client.get_room(&room_id).ok_or_else(|| "unknown room".to_string())?;
        let user_id =
            self.client.user_id().ok_or_else(|| "not logged in".to_string())?.to_owned();
        let device_id = self
            .client
            .device_id()
            .ok_or_else(|| "session has no device id".to_string())?
            .to_owned();
        Ok((room, user_id, device_id))
    }
}

/// Current call state for a room, read from the local state store. Room
/// calls only (application `m.call`, scope `m.room`); participants are
/// ordered oldest membership first, matching how MatrixRTC clients pick
/// the focus.
pub async fn snapshot(room: &Room) -> CallState {
    let mut with_ts: Vec<(MilliSecondsSinceUnixEpoch, CallParticipant)> = Vec::new();
    match room.get_state_events_static::<CallMemberEventContent>().await {
        Ok(events) => {
            for event in events {
                let RawSyncOrStrippedState::Sync(raw) = event else { continue };
                let Ok(SyncStateEvent::Original(ev)) = raw.deserialize() else { continue };
                for membership in ev.content.active_memberships(Some(ev.origin_server_ts)) {
                    if !membership.is_room_call() {
                        continue;
                    }
                    with_ts.push((
                        membership.created_ts().unwrap_or(ev.origin_server_ts),
                        CallParticipant {
                            user_id: ev.state_key.user_id().to_string(),
                            device_id: membership.device_id().to_string(),
                        },
                    ));
                }
            }
        }
        Err(error) => {
            tracing::warn!(room_id = %room.room_id(), %error, "failed to read call member state");
        }
    }
    with_ts.sort_by_key(|(ts, _)| *ts);
    let participants: Vec<CallParticipant> = with_ts.into_iter().map(|(_, p)| p).collect();

    let client = room.client();
    let joined = match (client.user_id(), client.device_id()) {
        (Some(user_id), Some(device_id)) => participants
            .iter()
            .any(|p| p.user_id == user_id.as_str() && p.device_id == device_id.as_str()),
        _ => false,
    };

    CallState { room_id: room.room_id().to_string(), participants, joined }
}

/// The `call_id` and preferred foci of the room's active call, taken from
/// its oldest membership.
async fn existing_call_parameters(room: &Room) -> Option<(String, Vec<Focus>)> {
    let events = room.get_state_events_static::<CallMemberEventContent>().await.ok()?;
    let mut oldest: Option<(MilliSecondsSinceUnixEpoch, String, Vec<Focus>)> = None;
    for event in events {
        let RawSyncOrStrippedState::Sync(raw) = event else { continue };
        let Ok(SyncStateEvent::Original(ev)) = raw.deserialize() else { continue };
        for membership in ev.content.active_memberships(Some(ev.origin_server_ts)) {
            let Application::Call(call) = membership.application() else { continue };
            if call.scope != CallScope::Room {
                continue;
            }
            let ts = membership.created_ts().unwrap_or(ev.origin_server_ts);
            if oldest.as_ref().is_none_or(|(oldest_ts, ..)| ts < *oldest_ts) {
                oldest = Some((ts, call.call_id.clone(), membership.foci_preferred().clone()));
            }
        }
    }
    oldest.map(|(_, call_id, foci)| (call_id, foci))
}

/// Foci the homeserver advertises in `.well-known/matrix/client`
/// (`org.matrix.msc4143.rtc_foci`) — used when starting a fresh call so
/// joining clients have an SFU to converge on. Empty when the server
/// doesn't advertise any (the call still works as pure signaling).
async fn well_known_foci(client: &Client, room_id: &RoomId) -> Vec<Focus> {
    match client.send(discover_homeserver::Request::new()).await {
        Ok(response) => response
            .rtc_foci
            .into_iter()
            .filter_map(|info| match info {
                RtcFocusInfo::LiveKit(livekit) => Some(Focus::Livekit(LivekitFocus::new(
                    // Element Call's convention: the room id names the
                    // LiveKit session on the SFU.
                    room_id.to_string(),
                    livekit.service_url,
                ))),
                _ => None,
            })
            .collect(),
        Err(error) => {
            tracing::debug!(%error, "no .well-known rtc_foci; starting the call without preferred foci");
            Vec::new()
        }
    }
}

/// Schedules the server-side "left" membership (MSC4140). `None` when the
/// homeserver doesn't support delayed events — everything still works, a
/// crash just leaves a membership behind until it's cleared manually.
async fn schedule_delayed_leave(room: &Room, state_key: &CallMemberStateKey) -> Option<String> {
    let request = delayed_state_event::unstable::Request::new(
        room.room_id().to_owned(),
        state_key.as_ref().to_owned(),
        DelayParameters::Timeout { timeout: DELAYED_LEAVE_TIMEOUT },
        &CallMemberEventContent::new_empty(None),
    )
    .ok()?;
    match room.client().send(request).await {
        Ok(response) => Some(response.delay_id),
        Err(error) => {
            tracing::info!(%error, "homeserver rejected the delayed leave; joining without crash cleanup");
            None
        }
    }
}

/// Keeps the scheduled leave from firing while we're in the call:
/// restarts its timeout at half the delay interval until aborted.
fn spawn_heartbeat(client: Client, delay_id: String) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(DELAYED_LEAVE_TIMEOUT / 2);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await; // completes immediately
        loop {
            ticker.tick().await;
            let request = update_delayed_event::unstable::Request::new(
                delay_id.clone(),
                update_delayed_event::unstable::UpdateAction::Restart,
            );
            match client.send(request).await {
                Ok(_) => {}
                Err(error) => {
                    let gone = error
                        .as_client_api_error()
                        .is_some_and(|e| e.status_code.as_u16() == 404);
                    if gone {
                        // The delay already fired (missed heartbeats) or was
                        // cancelled. With the local joined bookkeeping OR'd
                        // into snapshots, a fired leave does NOT flip the UI
                        // — the banner stays "In call" until the user clicks
                        // Leave, which republishes the empty membership
                        // (idempotent) and reconciles.
                        tracing::info!("delayed leave no longer exists; stopping heartbeat");
                        break;
                    }
                    // Transient failure: keep trying — worst case the leave
                    // fires and sync tells everyone the truth.
                    tracing::warn!(%error, "delayed-leave heartbeat failed");
                }
            }
        }
    })
}

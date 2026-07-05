//! Bridges the tokio-owned sync worker (see `client_core::sync`) into
//! iced's `Subscription` system. This is the concrete implementation of the
//! architecture described in the project plan: the worker is spawned once,
//! inside the subscription's producer, and its `ClientEvent` stream is
//! forwarded into `Message::Client` for the lifetime of the session.

use iced::futures::SinkExt;
use iced::Subscription;
use matrix_sdk::Client;
use tokio::sync::mpsc;

use crate::message::{Message, OpaqueCmdSender};
use crate::state::App;

pub fn subscription(app: &App) -> Subscription<Message> {
    let client_events = match &app.client {
        Some(client) => client_events(client.clone(), app.profile.clone()),
        None => Subscription::none(),
    };
    Subscription::batch([client_events, iced::event::listen_with(window_events)])
}

/// Window-level events the app cares about: resizes keep the native video
/// overlay (see `video_player`) glued to the iced-drawn frame.
fn window_events(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Window(iced::window::Event::Resized(size)) => {
            Some(Message::WindowResized(size))
        }
        _ => None,
    }
}

fn client_events(client: Client, profile: String) -> Subscription<Message> {
    Subscription::run_with_id(
        "matrix-sync-worker",
        iced::stream::channel(64, move |mut output| {
            let client = client.clone();
            let profile = profile.clone();
            async move {
                let cache_dir = match client_core::store::AppPaths::for_profile(&profile) {
                    Ok(paths) => paths.media_cache_dir(),
                    Err(error) => {
                        tracing::error!(%error, "failed to resolve media cache directory");
                        return;
                    }
                };

                let (event_tx, mut event_rx) = mpsc::unbounded_channel();
                let (cmd_tx, worker_handle) = client_core::sync::spawn(client, event_tx, cache_dir);

                // The worker runs for the lifetime of the process (or until
                // it stops itself on Logout); detach the handle rather than
                // awaiting it here.
                drop(worker_handle);

                if output.send(Message::WorkerStarted(OpaqueCmdSender(cmd_tx))).await.is_err() {
                    return;
                }

                while let Some(event) = event_rx.recv().await {
                    if output.send(Message::Client(event)).await.is_err() {
                        break;
                    }
                }
            }
        }),
    )
}

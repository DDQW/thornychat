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
    // Only subscribed while a Settings-panel resize drag is actually in
    // progress — tracking every mouse move for the app's whole lifetime
    // would be wasteful when nothing is being dragged.
    let settings_resize = if app.settings_resize_drag.is_some() {
        iced::event::listen_with(settings_resize_events)
    } else {
        Subscription::none()
    };
    Subscription::batch([client_events, iced::event::listen_with(window_events), settings_resize])
}

/// Window-level events the app cares about: resizes invalidate the
/// timeline's scroll-anchor geometry (and, via the update wrapper's stage
/// probe, reglue the inline video player), Escape dismisses the image
/// lightbox, Ctrl+V probes the clipboard for files/images to attach
/// (`clipboard_paste`), and dropped files stage as attachment chips.
fn window_events(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Window(iced::window::Event::Resized(size)) => {
            Some(Message::WindowResized(size))
        }
        iced::Event::Window(iced::window::Event::FileDropped(path)) => {
            Some(Message::FileDropped(path))
        }
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            ..
        }) => Some(Message::EscapePressed),
        // Deliberately ignores the capture status: a focused text_input
        // consumes Ctrl+V for its own *text* paste, but files/images on the
        // clipboard aren't text — those are handled app-side, and the
        // update-side "text wins" rule keeps one Ctrl+V from double-acting.
        // `!alt()` exempts AltGr combos (reported as Ctrl+Alt on Windows),
        // which are characters being typed, not paste chords.
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character(c),
            modifiers,
            ..
        }) if modifiers.command() && !modifiers.alt() && c.eq_ignore_ascii_case("v") => {
            Some(Message::PasteClipboard)
        }
        _ => None,
    }
}

/// Cursor tracking for an active Settings-panel resize drag (see
/// `state::ResizeDrag`). The grip's own `mouse_area` only fires while the
/// cursor stays over that tiny widget, which a drag immediately leaves, so
/// the actual tracking has to come from a window-level subscription instead.
fn settings_resize_events(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
            Some(Message::SettingsResizeDragged(position))
        }
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
            Some(Message::SettingsResizeReleased)
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

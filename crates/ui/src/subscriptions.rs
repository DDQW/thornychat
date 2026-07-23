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
    // Browser-style middle-click autoscroll: while it's active, a ~60fps timer
    // drives the glide and a window-level listener ends it on the next click,
    // wheel tick, or key press. Both are off entirely otherwise — no idle
    // timer, no extra event listening.
    let autoscroll = if app.timeline.autoscroll.is_some() {
        Subscription::batch([
            iced::time::every(std::time::Duration::from_millis(16))
                .map(|_| Message::AutoscrollTick),
            iced::event::listen_with(autoscroll_cancel_events),
        ])
    } else {
        Subscription::none()
    };
    // While a video plays inline, its WebView2 child window seizes Win32
    // keyboard focus the moment it's clicked, and winit won't reclaim it on
    // an ordinary click of the app surface — so any press that reaches iced
    // (i.e. landed off the video) hands focus back to the app window,
    // keeping the composer typeable. Off entirely when nothing is playing.
    let video_focus = if app.timeline.inline_video.is_some() {
        iced::event::listen_with(video_focus_events)
    } else {
        Subscription::none()
    };
    // Poll the enabled game-launcher connectors on a timer and announce what
    // you're playing when it changes (see `crate::connectors`). Off entirely
    // when no launcher is enabled — a fresh install does no polling.
    let connectors = if app.connectors.any_enabled() {
        let secs =
            app.connectors.poll_interval_secs.max(crate::connectors_config::MIN_POLL_INTERVAL_SECS);
        iced::time::every(std::time::Duration::from_secs(secs)).map(|_| Message::PollConnectors)
    } else {
        Subscription::none()
    };
    Subscription::batch([
        client_events,
        iced::event::listen_with(window_events),
        settings_resize,
        autoscroll,
        video_focus,
        connectors,
    ])
}

/// A press that reaches iced while a video plays landed on the app surface
/// (presses on the video's own child HWND never reach winit) — the user is
/// done with the video for now, so keyboard focus returns to the app window
/// (see `video_player::reclaim_focus`). Only mouse presses can serve as the
/// trigger: once the webview holds focus, key events don't reach iced at all.
fn video_focus_events(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_)) => Some(Message::ReclaimAppFocus),
        _ => None,
    }
}

/// Ends an active middle-click autoscroll on the first click, wheel tick, or
/// key press — the browser convention. The middle button is deliberately
/// excluded: it toggles autoscroll off through the message list's own
/// `mouse_area`, and catching it here too would race that toggle (the two
/// handlers could re-enable each other depending on delivery order).
fn autoscroll_cancel_events(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(button))
            if button != iced::mouse::Button::Middle =>
        {
            Some(Message::AutoscrollEnd)
        }
        iced::Event::Mouse(iced::mouse::Event::WheelScrolled { .. })
        | iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { .. }) => {
            Some(Message::AutoscrollEnd)
        }
        _ => None,
    }
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
        iced::Event::Window(iced::window::Event::Moved(position)) => {
            Some(Message::WindowMoved(position))
        }
        // Window-global cursor position, so a right-click menu can open at the
        // pointer: `mouse_area::on_right_press` carries no coordinates and
        // `on_move` only reports widget-local ones.
        iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
            Some(Message::CursorMoved(position))
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
    use std::hash::{Hash, Hasher};

    struct SyncData {
        client: Client,
        profile: String,
    }
    // One sync worker per profile. iced 0.14 dedupes subscriptions by hashing
    // `data` plus the builder fn pointer (0.13 used the "matrix-sync-worker"
    // id string); the client handle isn't hashable and must not factor into
    // that identity, so hash only a stable tag and the profile.
    impl Hash for SyncData {
        fn hash<H: Hasher>(&self, state: &mut H) {
            "matrix-sync-worker".hash(state);
            self.profile.hash(state);
        }
    }

    Subscription::run_with(SyncData { client, profile }, |data| {
        let client = data.client.clone();
        let profile = data.profile.clone();
        iced::stream::channel(64, move |mut output: iced::futures::channel::mpsc::Sender<Message>| {
            async move {
                let paths = match client_core::store::AppPaths::for_profile(&profile) {
                    Ok(paths) => paths,
                    Err(error) => {
                        tracing::error!(%error, "failed to resolve app data directory");
                        return;
                    }
                };

                let (event_tx, mut event_rx) = mpsc::unbounded_channel();
                let (cmd_tx, worker_handle) = client_core::sync::spawn(client, event_tx, paths);

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
        })
    })
}

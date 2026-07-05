use client_core::events::ClientEvent;
use client_core::ClientCommand;
use iced::Task;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::message::Message;
use crate::screens;
use crate::state::App;

pub fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::RestoreResult(Ok(Some(opaque_client))) => {
            app.adopt_client(opaque_client.0);
            Task::none()
        }
        Message::RestoreResult(Ok(None)) => {
            // No saved session; stay on the login screen.
            Task::none()
        }
        Message::RestoreResult(Err(reason)) => {
            app.login.status = screens::login::Status::Error(reason);
            Task::none()
        }

        Message::LoginResult(Ok(opaque_client)) => {
            app.adopt_client(opaque_client.0);
            Task::none()
        }
        Message::LoginResult(Err(reason)) => {
            app.login.status = screens::login::Status::Error(reason);
            Task::none()
        }

        Message::WorkerStarted(sender) => {
            app.cmd_tx = Some(sender.0);
            Task::none()
        }

        Message::Client(event) => dispatch_client_event(app, event),

        Message::Login(msg) => {
            let profile = app.profile.clone();
            let (task, effect) = screens::login::update(&mut app.login, msg);
            let task = task.map(Message::Login);

            let login_task = match effect {
                screens::login::Effect::None => Task::none(),
                screens::login::Effect::Discover { homeserver } => perform_discover(homeserver),
                screens::login::Effect::AttemptPasswordLogin { homeserver, username, password } => {
                    perform_password_login(profile, homeserver, username, password)
                }
                screens::login::Effect::AttemptSsoLogin { homeserver, identity_provider_id } => {
                    perform_sso_login(profile, homeserver, identity_provider_id)
                }
            };

            Task::batch([task, login_task])
        }

        Message::RoomList(screens::room_list::Message::RoomClicked(room_id)) => {
            select_room(app, room_id)
        }
        Message::RoomList(screens::room_list::Message::FilterChanged(filter)) => {
            app.room_list.filter = filter;
            Task::none()
        }

        Message::Timeline(msg) => {
            let (task, effect) = screens::timeline::update(&mut app.timeline, msg);
            let task = task.map(Message::Timeline);
            let effect_task = apply_timeline_effect(app, effect);
            Task::batch([task, effect_task])
        }

        Message::Verification(msg) => {
            let effect = screens::verification::update(&mut app.verification, msg);
            apply_verification_effect(app, effect)
        }

        Message::EmojiSvgFetched(emoji, result) => {
            app.media.emoji_pending.remove(&emoji);
            match result {
                Ok(bytes) => {
                    app.media.emoji.insert(emoji, iced::widget::svg::Handle::from_memory(bytes));
                }
                // Negative-cache the failure — bogus reaction keys 404 on
                // the CDN and would otherwise refetch on every sync tick.
                Err(_) => {
                    app.media.emoji_failed.insert(emoji);
                }
            }
            Task::none()
        }

        Message::GifDecoded(url, result) => {
            app.media.pending_urls.remove(&url);
            match result {
                Ok(frames) => {
                    app.media.mxc_gifs.insert(url, frames);
                }
                // Corrupt GIF: fall back to the (first-frame-only) raster path.
                Err(raster) => {
                    app.media.images.insert(url, raster);
                }
            }
            Task::none()
        }

        Message::TweetFetched(url, Ok(tweet)) => {
            let image_urls = tweet.all_image_urls();
            app.tweet_previews.insert(url, Some(tweet));
            ensure_web_images(app, image_urls)
        }
        Message::TweetFetched(url, Err(reason)) => {
            tracing::debug!(%url, %reason, "fxtwitter lookup failed; falling back to OG preview");
            // Keep the None entry (don't retry) and fall back to the
            // homeserver's OpenGraph card for this URL.
            if !app.url_previews.contains_key(&url) {
                app.url_previews.insert(url.clone(), None);
                send_cmd(app, ClientCommand::FetchUrlPreview { url });
            }
            Task::none()
        }
        Message::SteamFetched(url, Ok(app_data)) => {
            let image_urls: Vec<String> = app_data.header_image.clone().into_iter().collect();
            app.steam_previews.insert(url, Some(app_data));
            ensure_web_images(app, image_urls)
        }
        Message::SteamFetched(url, Err(reason)) => {
            tracing::debug!(%url, %reason, "steam appdetails lookup failed; falling back to OG preview");
            // Keep the None entry (don't retry) and fall back to the
            // homeserver's OpenGraph card for this URL.
            if !app.url_previews.contains_key(&url) {
                app.url_previews.insert(url.clone(), None);
                send_cmd(app, ClientCommand::FetchUrlPreview { url });
            }
            Task::none()
        }
        Message::WebImageFetched(url, result) => {
            app.media.web_pending.remove(&url);
            if let Ok(bytes) = result {
                app.media.web_images.insert(url, iced::widget::image::Handle::from_bytes(bytes));
            }
            Task::none()
        }

        Message::AttachmentPickedFor { room_id, filename, bytes } => {
            if app.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                // Same room still open — route through the composer's normal
                // AttachmentPicked path (mime sniffing, in-flight guard).
                update(
                    app,
                    Message::Timeline(screens::timeline::Message::Composer(
                        screens::timeline::composer::Message::AttachmentPicked(Ok((
                            filename, bytes,
                        ))),
                    )),
                )
            } else {
                tracing::info!("attachment picked for a room no longer open; dropped");
                app.timeline.composer.error =
                    Some("Attachment cancelled — the room changed while picking a file".into());
                Task::none()
            }
        }

        Message::CloseZoom => {
            app.zoomed_image = None;
            Task::none()
        }

        Message::VideoPlayerOpened { window, scale, result } => {
            match &mut app.video_player {
                Some(player) => {
                    // A WindowResized that landed while the webview was still
                    // building recorded a fresher size — keep that one and
                    // re-sync the native bounds to it, instead of clobbering
                    // it with the stale open-time capture (maximize emits no
                    // further resize, so it would stay wrong indefinitely).
                    let effective = *player.window.get_or_insert(window);
                    player.scale = scale;
                    if let Err(reason) = result {
                        tracing::warn!(%reason, "embedded video player failed to start");
                        player.error = Some(reason);
                        return Task::none();
                    }
                    if effective != window {
                        let rect = crate::video_player::video_rect(effective);
                        return iced::window::get_latest().then(move |id| match id {
                            Some(id) => iced::window::run_with_handle(id, move |_handle| {
                                crate::video_player::set_bounds(rect, scale);
                            })
                            .map(|_| Message::Noop),
                            None => Task::none(),
                        });
                    }
                    Task::none()
                }
                // Overlay was dismissed before the webview finished
                // building — tear the orphan down.
                None => close_native_player(),
            }
        }
        Message::WindowResized(size) => {
            // Text rewraps to the new width — every row's on-screen position
            // changes legitimately, so the scroll anchor's stored geometry
            // is meaningless until the next scroll re-learns it.
            app.timeline.scroll_anchor = None;
            if let Some(player) = &mut app.video_player {
                player.window = Some(size);
                if player.error.is_none() {
                    let rect = crate::video_player::video_rect(size);
                    let scale = player.scale;
                    return iced::window::get_latest().then(move |id| match id {
                        Some(id) => iced::window::run_with_handle(id, move |_handle| {
                            crate::video_player::set_bounds(rect, scale);
                        })
                        .map(|_| Message::Noop),
                        None => Task::none(),
                    });
                }
            }
            Task::none()
        }
        Message::CloseVideoPlayer => {
            app.video_player = None;
            close_native_player()
        }
        Message::OpenVideoInBrowser => {
            if let Some(player) = app.video_player.take() {
                let _ = open::that(player.video.watch_url());
            }
            close_native_player()
        }

        Message::ToggleDarkMode => {
            app.dark_mode = !app.dark_mode;
            Task::none()
        }
        Message::ToggleKeywordPanel => {
            app.show_keyword_panel = !app.show_keyword_panel;
            Task::none()
        }
        Message::KeywordDraftChanged(draft) => {
            app.keyword_draft = draft;
            Task::none()
        }
        Message::AddKeywordClicked => {
            let keyword = app.keyword_draft.trim().to_string();
            if !keyword.is_empty() {
                app.keyword_draft.clear();
                send_cmd(app, ClientCommand::AddKeywordHighlight { keyword, request_id: Uuid::new_v4() });
            }
            Task::none()
        }
        Message::RemoveKeywordClicked(keyword) => {
            send_cmd(app, ClientCommand::RemoveKeywordHighlight { keyword, request_id: Uuid::new_v4() });
            Task::none()
        }

        Message::Settings(_) | Message::Noop => Task::none(),
    }
}

fn apply_verification_effect(app: &mut App, effect: screens::verification::Effect) -> Task<Message> {
    use screens::verification::Effect;
    match effect {
        Effect::None => {}
        Effect::OpenUrl(url) => {
            let _ = open::that(url);
        }
        Effect::RetryCrossSigningBootstrap => {
            send_cmd(app, ClientCommand::RetryCrossSigningBootstrap);
        }
        Effect::EnableRecovery { passphrase } => {
            let request_id = Uuid::new_v4();
            send_cmd(
                app,
                ClientCommand::EnableRecovery { passphrase: passphrase.map(Zeroizing::new), request_id },
            );
        }
        Effect::RestoreFromBackup { recovery_key } => {
            let request_id = Uuid::new_v4();
            send_cmd(
                app,
                ClientCommand::RestoreFromBackup { recovery_key: Zeroizing::new(recovery_key), request_id },
            );
        }
        Effect::StartVerification { user_id } => {
            send_cmd(app, ClientCommand::StartVerification { user_id });
        }
        Effect::AcceptVerificationRequest => send_cmd(app, ClientCommand::AcceptVerificationRequest),
        Effect::ConfirmSasMatch => send_cmd(app, ClientCommand::ConfirmSasMatch),
        Effect::RejectSasMatch => send_cmd(app, ClientCommand::RejectSasMatch),
        Effect::VerificationCancel => send_cmd(app, ClientCommand::VerificationCancel),
    }
    Task::none()
}

fn send_cmd(app: &App, cmd: ClientCommand) {
    if let Some(tx) = &app.cmd_tx {
        let _ = tx.send(cmd);
    }
}

fn apply_timeline_effect(app: &mut App, effect: screens::timeline::Effect) -> Task<Message> {
    match effect {
        screens::timeline::Effect::None => Task::none(),
        screens::timeline::Effect::Composer(composer_effect) => apply_composer_effect(app, composer_effect),
        screens::timeline::Effect::Edit { event_id, new_body } => {
            send_edit(app, event_id, new_body);
            Task::none()
        }
        screens::timeline::Effect::Redact { event_id } => {
            send_redact(app, event_id);
            Task::none()
        }
        screens::timeline::Effect::ToggleReaction { event_id, key } => {
            let usage_task = record_emoji_use(app, key.clone());
            toggle_reaction(app, event_id, key);
            usage_task
        }
        screens::timeline::Effect::EnsureEmojiFetched(emojis) => ensure_emoji_fetched(app, emojis),
        screens::timeline::Effect::SetNotificationMode(mode) => {
            if let Some(room_id) = app.timeline.room_id.clone() {
                let request_id = Uuid::new_v4();
                let cmd = match mode {
                    Some(mode) => ClientCommand::SetRoomNotificationMode { room_id, mode, request_id },
                    None => ClientCommand::ClearRoomNotificationMode { room_id, request_id },
                };
                send_cmd(app, cmd);
            }
            Task::none()
        }
        screens::timeline::Effect::PaginateBackwards => {
            if let Some(room_id) = app.timeline.room_id.clone() {
                let request_id = Uuid::new_v4();
                app.timeline.pending_paginate_request = Some(request_id);
                send_cmd(app, ClientCommand::PaginateBackwards { room_id, request_id });
            } else {
                app.timeline.loading_older = false;
            }
            Task::none()
        }
        screens::timeline::Effect::MaybeMarkRead => {
            if app.timeline.at_bottom {
                mark_open_room_read(app);
            }
            Task::none()
        }
        screens::timeline::Effect::ZoomImage(url) => {
            app.zoomed_image = Some(url);
            Task::none()
        }
        screens::timeline::Effect::OpenDirectMessage(user_id) => {
            send_cmd(app, ClientCommand::OpenDirectMessage { user_id, request_id: Uuid::new_v4() });
            Task::none()
        }
        screens::timeline::Effect::PlayVideo { video, title } => {
            app.video_player = Some(crate::state::VideoPlayer {
                video: video.clone(),
                title,
                window: None,
                scale: 1.0,
                error: None,
            });
            open_native_player(&app.profile, video)
        }
        screens::timeline::Effect::JoinCall => {
            send_call_command(app, true);
            Task::none()
        }
        screens::timeline::Effect::LeaveCall => {
            send_call_command(app, false);
            Task::none()
        }
        screens::timeline::Effect::DismissCallError => {
            app.call.error = None;
            Task::none()
        }
    }
}

/// Fires JoinCall/LeaveCall for the open room, remembering the request so
/// the banner can show progress and surface a failure.
fn send_call_command(app: &mut App, join: bool) {
    let Some(room_id) = app.timeline.room_id.clone() else { return };
    if app.call.pending.is_some() {
        // One join/leave in flight at a time — the banner's button is
        // disabled, but the header's start-call button could still race.
        return;
    }
    let request_id = Uuid::new_v4();
    app.call.pending = Some((request_id, room_id.clone(), join));
    app.call.error = None;
    let cmd = if join {
        ClientCommand::JoinCall { room_id, request_id }
    } else {
        ClientCommand::LeaveCall { room_id, request_id }
    };
    send_cmd(app, cmd);
}

/// Builds the native webview on the event-loop thread: window id → size →
/// scale factor → `run_with_handle` (which is where WebView2 creation must
/// happen). The geometry captured along the way rides back on
/// [`Message::VideoPlayerOpened`] so the overlay draws to the same rect.
fn open_native_player(profile: &str, video: crate::video_player::EmbedVideo) -> Task<Message> {
    // WebView2 refuses to put its profile data next to the exe; park it
    // with the rest of this profile's on-disk state.
    let data_dir = client_core::store::AppPaths::for_profile(profile)
        .ok()
        .map(|paths| paths.root.join("webview-data"));

    iced::window::get_latest().then(move |maybe_id| {
        let Some(id) = maybe_id else {
            return Task::none();
        };
        let video = video.clone();
        let data_dir = data_dir.clone();
        iced::window::get_size(id).then(move |size| {
            let video = video.clone();
            let data_dir = data_dir.clone();
            iced::window::get_scale_factor(id).then(move |scale| {
                let video = video.clone();
                let data_dir = data_dir.clone();
                let rect = crate::video_player::video_rect(size);
                iced::window::run_with_handle(id, move |handle| {
                    crate::video_player::open(&handle, &video, rect, scale, data_dir)
                })
                .map(move |result| Message::VideoPlayerOpened { window: size, scale, result })
            })
        })
    })
}

/// Drops the native webview (if any) on its owning thread. Safe to fire
/// even when nothing is open.
fn close_native_player() -> Task<Message> {
    iced::window::get_latest().then(|maybe_id| match maybe_id {
        Some(id) => iced::window::run_with_handle(id, |_handle| {
            crate::video_player::close();
        })
        .map(|_| Message::Noop),
        None => Task::none(),
    })
}

fn mark_open_room_read(app: &mut App) {
    if let Some(room_id) = app.timeline.room_id.clone() {
        send_cmd(app, ClientCommand::MarkRoomRead { room_id });
        // The user is caught up as of this instant — hide the unread
        // divider locally rather than waiting for the server's fully-read
        // echo (which observably never arrives on this homeserver).
        app.timeline.suppress_unread_divider = true;
    }
}

fn apply_composer_effect(app: &mut App, effect: screens::timeline::composer::Effect) -> Task<Message> {
    use screens::timeline::composer::Effect as ComposerEffect;
    match effect {
        ComposerEffect::None => Task::none(),
        ComposerEffect::Send { body, mentioned_user_ids, reply_to_event_id } => {
            send_message(app, body, mentioned_user_ids, reply_to_event_id);
            Task::none()
        }
        ComposerEffect::PickAttachment => match app.timeline.room_id.clone() {
            Some(room_id) => pick_attachment_task(room_id),
            None => Task::none(),
        },
        ComposerEffect::SendAttachment { filename, bytes, mime } => {
            send_attachment(app, filename, bytes, mime);
            Task::none()
        }
        ComposerEffect::Typing(typing) => {
            set_typing(app, typing);
            Task::none()
        }
        ComposerEffect::EnsureEmojiFetched(emojis) => ensure_emoji_fetched(app, emojis),
        ComposerEffect::EmojiUsed(key) => record_emoji_use(app, key),
    }
}

/// Bumps the usage count behind the picker's "Frequently used" section and
/// persists the history. The map update is synchronous; the file write is
/// not — a create+write through Defender's real-time scan is milliseconds
/// (a dropped frame per emoji click if done on the update thread).
fn record_emoji_use(app: &mut App, key: String) -> Task<Message> {
    *app.emoji_usage.entry(key).or_insert(0) += 1;
    let Some(path) = crate::state::emoji_usage_path(&app.profile) else { return Task::none() };
    let Ok(contents) = serde_json::to_string(&app.emoji_usage) else { return Task::none() };
    Task::future(async move {
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(path, contents).await;
        Message::Noop
    })
}

/// Issues an async Twemoji-SVG fetch for every emoji in `emojis` that isn't
/// already cached or in flight. Pure UI-side work (not routed through
/// `client_core`) since Twemoji assets aren't Matrix data.
fn ensure_emoji_fetched(app: &mut App, emojis: Vec<String>) -> Task<Message> {
    // Resolved lazily, once per batch: `AppPaths::for_profile` does a
    // blocking create_dir_all plus a known-folder lookup, and opening the
    // emoji picker requests ~1800 emoji in one call — doing it per emoji
    // meant ~1800 identical filesystem round trips.
    let mut cache_dir: Option<std::path::PathBuf> = None;
    let mut tasks = Vec::new();
    for emoji in emojis {
        // Check-and-mark inside the loop: `is_emoji_known` consults
        // emoji_pending, so marking here also dedups repeats WITHIN this
        // batch (20 messages all reacted with "👍" must spawn one fetch,
        // not 20).
        if app.media.is_emoji_known(&emoji) {
            continue;
        }
        let dir = match &cache_dir {
            Some(dir) => dir.clone(),
            None => match client_core::store::AppPaths::for_profile(&app.profile) {
                Ok(paths) => {
                    let dir = paths.emoji_cache_dir();
                    cache_dir = Some(dir.clone());
                    dir
                }
                Err(error) => {
                    tracing::warn!(%error, "cannot resolve emoji cache dir");
                    return Task::batch(tasks);
                }
            },
        };
        app.media.emoji_pending.insert(emoji.clone());
        tasks.push(fetch_emoji_svg_task(dir, emoji));
    }
    Task::batch(tasks)
}

fn fetch_emoji_svg_task(cache_dir: std::path::PathBuf, emoji: String) -> Task<Message> {
    let emoji_for_message = emoji.clone();
    Task::perform(
        async move { crate::twemoji::fetch(&cache_dir, &emoji).await.map_err(|e| e.to_string()) },
        move |result| Message::EmojiSvgFetched(emoji_for_message.clone(), result),
    )
}

fn send_message(
    app: &mut App,
    body: String,
    mentioned_user_ids: Vec<String>,
    reply_to_event_id: Option<String>,
) {
    let Some(room_id) = app.timeline.room_id.clone() else { return };
    let Some(cmd_tx) = &app.cmd_tx else { return };
    let request_id = Uuid::new_v4();
    app.timeline.composer.pending_request = Some(request_id);
    let _ = cmd_tx.send(ClientCommand::SendMessage {
        room_id,
        body,
        mentioned_user_ids,
        reply_to_event_id,
        request_id,
    });
}

fn send_attachment(app: &mut App, filename: String, bytes: Vec<u8>, mime: String) {
    let Some(room_id) = app.timeline.room_id.clone() else { return };
    let Some(cmd_tx) = &app.cmd_tx else { return };
    let request_id = Uuid::new_v4();
    // NOT pending_request: sharing the text-send slot would make the
    // attachment's CommandSucceeded wipe an unsent draft via SendSucceeded.
    app.timeline.composer.pending_attachment_request = Some(request_id);
    let _ = cmd_tx.send(ClientCommand::SendAttachment { room_id, filename, bytes, mime, request_id });
}

fn send_edit(app: &mut App, event_id: String, new_body: String) {
    let Some(room_id) = app.timeline.room_id.clone() else { return };
    let Some(cmd_tx) = &app.cmd_tx else { return };
    let request_id = Uuid::new_v4();
    app.timeline.pending_edit_request = Some(request_id);
    let _ = cmd_tx.send(ClientCommand::EditMessage { room_id, event_id, new_body, request_id });
}

fn send_redact(app: &mut App, event_id: String) {
    let Some(room_id) = app.timeline.room_id.clone() else { return };
    let Some(cmd_tx) = &app.cmd_tx else { return };
    let request_id = Uuid::new_v4();
    app.timeline.pending_redact_request = Some(request_id);
    let _ = cmd_tx.send(ClientCommand::RedactEvent { room_id, event_id, reason: None, request_id });
}

fn set_typing(app: &App, typing: bool) {
    let Some(room_id) = app.timeline.room_id.clone() else { return };
    let Some(cmd_tx) = &app.cmd_tx else { return };
    let _ = cmd_tx.send(ClientCommand::SetTyping { room_id, typing });
}

fn toggle_reaction(app: &App, event_id: String, key: String) {
    let Some(room_id) = app.timeline.room_id.clone() else { return };
    let Some(cmd_tx) = &app.cmd_tx else { return };
    let request_id = Uuid::new_v4();
    let _ = cmd_tx.send(ClientCommand::ToggleReaction { room_id, event_id, key, request_id });
}

/// Issues `ClientCommand::FetchMedia` for every mxc URL referenced by
/// `urls` that isn't already cached or in flight.
fn ensure_media_fetched(app: &mut App, urls: impl IntoIterator<Item = String>) {
    let Some(cmd_tx) = &app.cmd_tx else { return };
    for url in urls {
        if app.media.is_known(&url) {
            continue;
        }
        let request_id = Uuid::new_v4();
        app.media.pending.insert(request_id, url.clone());
        app.media.pending_urls.insert(url.clone());
        let _ = cmd_tx.send(ClientCommand::FetchMedia { mxc_url: url, request_id });
    }
}

/// Custom-emoji reactions carry their `mxc://` URL directly as the
/// reaction key (MSC2545) — those need fetching as media too, not just
/// image-message content, or `resolve_reaction_visual`'s `images.get(key)`
/// lookup never has anything to find. Sender avatars ride along the same
/// path.
fn image_urls_in_timeline(
    items: &[client_core::events::TimelineItem],
    media: &crate::media_cache::State,
) -> Vec<String> {
    // Filter against the cache BEFORE cloning: after the first snapshot,
    // nearly every URL here is already known, and this runs per sync tick.
    items
        .iter()
        .flat_map(|item| {
            let content_url = match &item.content {
                client_core::events::TimelineItemContent::Image { url, .. } => Some(url.as_str()),
                _ => None,
            };
            let reaction_urls =
                item.reactions.iter().map(|r| r.key.as_str()).filter(|k| k.starts_with("mxc://"));
            content_url
                .into_iter()
                .chain(reaction_urls)
                .chain(item.sender_avatar_url.as_deref())
                .chain(item.in_reply_to.as_ref().and_then(|r| r.image_url.as_deref()))
        })
        .filter(|url| !media.is_known(url))
        .map(str::to_owned)
        .collect()
}

fn image_urls_in_packs(packs: &[client_core::events::EmojiPack]) -> Vec<String> {
    packs.iter().flat_map(|p| p.emojis.iter().map(|e| e.mxc_url.clone())).collect()
}

/// Lowercased shortcode → emoji; first pack wins on collisions (the
/// personal pack is fetched first, matching the old first-match scan).
fn build_shortcode_index(
    packs: &[client_core::events::EmojiPack],
) -> std::collections::HashMap<String, client_core::events::CustomEmoji> {
    let mut index = std::collections::HashMap::new();
    for emoji in packs.iter().flat_map(|p| &p.emojis) {
        index.entry(emoji.shortcode.to_ascii_lowercase()).or_insert_with(|| emoji.clone());
    }
    index
}

/// Requests a preview for each text message's first URL, once per unique
/// URL for the app's lifetime. Tweet links go to the FxTwitter API (rich
/// card: author, photos, engagement), Steam store links go to the
/// storefront appdetails API (capsule art, platforms, live pricing);
/// everything else goes through the homeserver's OpenGraph proxy.
/// Also returns each item's first URL (index-parallel to `items`) so the
/// caller can store it in timeline State for view() — the linkify scan
/// already happens here once per snapshot, and view() must not repeat it
/// per URL-bearing message per frame.
fn request_url_previews(
    app: &mut App,
    items: &[client_core::events::TimelineItem],
) -> (Task<Message>, Vec<Option<String>>) {
    let first_urls: Vec<Option<String>> = items
        .iter()
        .map(|item| match &item.content {
            client_core::events::TimelineItemContent::Text(body) => {
                screens::timeline::first_url_in(body)
            }
            _ => None,
        })
        .collect();

    let mut tasks = Vec::new();
    for url in first_urls.iter().flatten() {
        if let Some(api_url) = crate::tweets::tweet_api_url(url) {
            if !app.tweet_previews.contains_key(url) {
                app.tweet_previews.insert(url.clone(), None);
                let url_for_message = url.clone();
                tasks.push(Task::perform(
                    async move { crate::tweets::fetch(api_url).await.map_err(|e| e.to_string()) },
                    move |result| Message::TweetFetched(url_for_message.clone(), result),
                ));
            }
        } else if let Some(api_url) = crate::steam::steam_api_url(url) {
            if !app.steam_previews.contains_key(url) {
                app.steam_previews.insert(url.clone(), None);
                let url_for_message = url.clone();
                tasks.push(Task::perform(
                    async move { crate::steam::fetch(api_url).await.map_err(|e| e.to_string()) },
                    move |result| Message::SteamFetched(url_for_message.clone(), result),
                ));
            }
        } else if !app.url_previews.contains_key(url) {
            app.url_previews.insert(url.clone(), None);
            send_cmd(app, ClientCommand::FetchUrlPreview { url: url.clone() });
        }
    }
    (Task::batch(tasks), first_urls)
}

/// Kicks off fetches for plain-HTTPS images (tweet avatars/photos) not yet
/// cached or in flight.
fn ensure_web_images(app: &mut App, urls: impl IntoIterator<Item = String>) -> Task<Message> {
    let mut tasks = Vec::new();
    for url in urls {
        if app.media.is_web_image_known(&url) {
            continue;
        }
        app.media.web_pending.insert(url.clone());
        let url_for_message = url.clone();
        tasks.push(Task::perform(
            async move { crate::tweets::fetch_image(url).await.map_err(|e| e.to_string()) },
            move |result| Message::WebImageFetched(url_for_message.clone(), result),
        ));
    }
    Task::batch(tasks)
}

/// Reaction keys that are neither an `mxc://` URL nor a known custom-pack
/// shortcode are assumed to be plain unicode emoji, candidates for a
/// Twemoji fetch (see `resolve_reaction_visual`'s same fallback order).
fn unicode_reaction_keys(
    items: &[client_core::events::TimelineItem],
    packs: &[client_core::events::EmojiPack],
    media: &crate::media_cache::State,
) -> Vec<String> {
    // One pass over the packs instead of a rescan per reaction key — this
    // runs on every TimelineUpdated (i.e. every sync tick of the open room).
    // Filter on borrowed strs and clone last (same shape as
    // image_urls_in_timeline above): after warmup nearly every key is
    // already cached and would be cloned only to be dropped.
    let shortcodes: std::collections::HashSet<&str> =
        packs.iter().flat_map(|p| &p.emojis).map(|e| e.shortcode.as_str()).collect();
    items
        .iter()
        .flat_map(|item| item.reactions.iter().map(|r| r.key.as_str()))
        .filter(|key| !key.starts_with("mxc://"))
        .filter(|key| !shortcodes.contains(key.trim_matches(':')))
        .filter(|key| !media.is_emoji_known(key))
        .map(str::to_owned)
        .collect()
}

fn pick_attachment_task(room_id: String) -> Task<Message> {
    Task::perform(
        async {
            match rfd::AsyncFileDialog::new().pick_file().await {
                Some(file) => {
                    let filename = file.file_name();
                    let bytes = file.read().await;
                    Some((filename, bytes))
                }
                None => None,
            }
        },
        move |result| match result {
            Some((filename, bytes)) => {
                Message::AttachmentPickedFor { room_id: room_id.clone(), filename, bytes }
            }
            // Cancelling the dialog is a normal action, not an error to pin
            // above the composer.
            None => Message::Noop,
        },
    )
}

/// Switches the open room: tells the sync worker to close the previous
/// room's timeline (if any) and open the newly selected one, and clears the
/// timeline pane immediately so stale messages don't flash while the new
/// room's snapshot is in flight.
fn select_room(app: &mut App, room_id: String) -> Task<Message> {
    let previous = app.room_list.selected_room_id.replace(room_id.clone());

    app.timeline.room_id = Some(room_id.clone());
    app.timeline.items.clear();
    app.timeline.first_urls.clear();
    app.timeline.composer = screens::timeline::composer::State::default();
    // Kept in lockstep with composer.member_candidates (just cleared above).
    app.timeline.member_index.clear();
    app.timeline.editing = None;
    app.timeline.confirm_delete = None;
    app.timeline.typing_users.clear();
    app.timeline.action_error = None;
    app.timeline.reacting_to = None;
    app.timeline.search_open = false;
    app.timeline.search_query.clear();
    app.timeline.search_matches.clear();
    app.timeline.reached_start = false;
    app.timeline.loading_older = false;
    app.timeline.pending_paginate_request = None;
    // Stale edit/redact requests would otherwise match a late CommandFailed
    // from the previous room and surface its error banner in this one.
    app.timeline.pending_edit_request = None;
    app.timeline.pending_redact_request = None;
    // The scrollable is bottom-anchored, so a freshly opened room starts at
    // the newest message. `at_bottom` alone is just a state flag though —
    // the scrollable itself keeps whatever raw scroll position it had from
    // the previous room (it's the same widget id) unless explicitly moved,
    // so without the `scroll_to` below a freshly opened room could render
    // scrolled to wherever you happened to leave the last one.
    app.timeline.at_bottom = true;
    app.timeline.highlighted_event_id = None;
    app.timeline.scroll_anchor = None;
    app.timeline.last_content_height = 0.0;
    app.timeline.last_from_bottom = 0.0;
    app.timeline.last_seen_newest = None;
    app.timeline.suppress_unread_divider = false;
    app.timeline.power_tags.clear();
    app.zoomed_image = None;

    let reset_scroll = iced::widget::scrollable::scroll_to(
        screens::timeline::timeline_scroll_id(),
        iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: 0.0 },
    );

    let Some(cmd_tx) = &app.cmd_tx else { return reset_scroll };

    if let Some(previous_room_id) = previous {
        if previous_room_id != room_id {
            let _ = cmd_tx.send(ClientCommand::CloseRoom { room_id: previous_room_id });
        }
    }
    let _ = cmd_tx.send(ClientCommand::OpenRoom { room_id });
    reset_scroll
}

/// A single `ClientEvent` can affect multiple screens at once (unread
/// counts, tray badge, open timeline). Fanning it out here in one explicit
/// function keeps that "god object" surface named and contained, rather
/// than smeared across the root `match`.
fn dispatch_client_event(app: &mut App, event: ClientEvent) -> Task<Message> {
    match event {
        ClientEvent::SyncStateChanged(state) => {
            app.sync_state = state;
        }
        ClientEvent::LoggedOut => {
            app.client = None;
            app.cmd_tx = None;
            app.own_user_id = None;
            // Session-scoped state must not leak into the next login: the
            // old account's rooms/timeline would linger, media requests
            // in flight on the dead worker would block refetching those
            // URLs forever, and verification/notification state would be
            // flat-out wrong for the next account.
            app.room_list = Default::default();
            app.timeline = Default::default();
            app.verification = Default::default();
            app.call = Default::default();
            app.media.pending.clear();
            app.media.pending_urls.clear();
            app.notification_modes.clear();
            app.keyword_highlights.clear();
            app.emoji_packs.clear();
            app.emoji_shortcode_index.clear();
            app.zoomed_image = None;
            app.route = crate::state::Route::Login;
            if app.video_player.take().is_some() {
                return close_native_player();
            }
        }
        ClientEvent::DirectMessageReady { room_id } => {
            return select_room(app, room_id);
        }
        ClientEvent::PowerLevelTagsUpdated { room_id, tags } => {
            if app.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                app.timeline.power_tags = tags;
            }
        }
        ClientEvent::RoomListUpdated(mut rooms) => {
            let avatar_urls: Vec<String> = rooms
                .iter()
                .filter_map(|r| r.avatar_url.as_deref())
                .filter(|url| !app.media.is_known(url))
                .map(str::to_owned)
                .collect();
            // Same root cause as the timeline divider: this count comes
            // from the SDK, which only clears it once the server echoes
            // our receipt back — an echo that doesn't arrive on this
            // homeserver. The currently-open room, once locally confirmed
            // read, shouldn't show a stale badge for content the user is
            // looking at right now (screenshot caught exactly this: "1"
            // sitting on the open room). Scoped to just that room, not a
            // general per-room override — clearing every room's badge
            // this way needs tracking read state across switches, a
            // bigger feature than this bug warranted.
            if app.timeline.suppress_unread_divider {
                if let Some(open_id) = &app.timeline.room_id {
                    if let Some(room) = rooms.iter_mut().find(|r| &r.room_id == open_id) {
                        room.unread_count = 0;
                    }
                }
            }
            app.room_list.rooms = rooms;
            ensure_media_fetched(app, avatar_urls);
        }
        ClientEvent::TimelineUpdated { room_id, items } => {
            if app.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                let urls = image_urls_in_timeline(&items, &app.media);
                let candidate_emojis =
                    unicode_reaction_keys(&items, &app.emoji_packs, &app.media);

                // A sync gap makes the SDK clear and re-seed the timeline:
                // the previous window is gone wholesale (its first event no
                // longer exists in the new snapshot — appends, prepends,
                // edits and receipts all keep it). Should be rare now that
                // the open room is subscribed (see `OpenRoom` in the sync
                // worker), but still happens after sleep/reconnect or a
                // >20-event burst. The old scroll offset points into a list
                // that no longer exists; land at the live edge predictably
                // instead of wherever the clamp happens to fall.
                let reset = {
                    let old_first = app.timeline.items.iter().find_map(|i| i.event_id.as_deref());
                    match old_first {
                        Some(old_first) => {
                            !items.iter().any(|i| i.event_id.as_deref() == Some(old_first))
                        }
                        None => false,
                    }
                };

                let (preview_task, first_urls) = request_url_previews(app, &items);
                app.timeline.items = items;
                // Index-parallel to items — set only here and cleared in
                // select_room, so the two can't drift apart.
                app.timeline.first_urls = first_urls;
                // Membership set is index-based; a new snapshot invalidates
                // it (no-op when no search is active).
                screens::timeline::recompute_search_matches(&mut app.timeline);
                let reset_task = if reset {
                    tracing::info!(
                        len = app.timeline.items.len(),
                        "timeline window reset by a sync gap — snapping to live edge"
                    );
                    app.timeline.scroll_anchor = None;
                    app.timeline.at_bottom = true;
                    iced::widget::scrollable::scroll_to(
                        screens::timeline::timeline_scroll_id(),
                        iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: 0.0 },
                    )
                } else {
                    Task::none()
                };
                ensure_media_fetched(app, urls);
                let emoji_task = ensure_emoji_fetched(app, candidate_emojis);
                // Diagnostic: the receiving end of the pipeline — a new
                // tail event proves snapshots reach the UI, and `at_bottom`
                // at that instant is the sole gate for marking read.
                let newest_changed = {
                    let newest =
                        app.timeline.items.iter().rev().find_map(|i| i.event_id.as_deref());
                    newest != app.timeline.last_seen_newest.as_deref()
                };
                if newest_changed {
                    app.timeline.last_seen_newest = app
                        .timeline
                        .items
                        .iter()
                        .rev()
                        .find_map(|i| i.event_id.clone());
                    tracing::debug!(
                        at_bottom = app.timeline.at_bottom,
                        len = app.timeline.items.len(),
                        "timeline snapshot applied (new tail event)"
                    );
                    // A genuinely new message while scrolled up: the unread
                    // boundary is meaningful again.
                    if !app.timeline.at_bottom {
                        app.timeline.suppress_unread_divider = false;
                    }
                }
                // IRC-style: while the newest message is in view, anything
                // that just arrived is immediately read (focus irrelevant).
                // Scrolling up is the only thing that holds messages unread.
                // `mark_as_read` is idempotent, so re-firing on non-message
                // updates (reactions, receipts) is a cheap no-op.
                if app.timeline.at_bottom {
                    mark_open_room_read(app);
                }
                return Task::batch([preview_task, reset_task, emoji_task]);
            }
        }
        ClientEvent::RoomMembersUpdated { room_id, members } => {
            if app.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                let mut members = members;
                // Sorted once here so member_panel doesn't re-sort every
                // group on every view call (groups are filled in iteration
                // order, so pre-sorted input keeps them sorted).
                members.sort_by_cached_key(|m| m.display_name.to_lowercase());
                app.timeline.member_index =
                    members.iter().enumerate().map(|(i, m)| (m.user_id.clone(), i)).collect();
                app.timeline.composer.member_candidates_lower =
                    members.iter().map(|m| m.display_name.to_lowercase()).collect();
                app.timeline.composer.member_candidates = members;
            }
        }
        ClientEvent::TypingUpdated { room_id, user_ids } => {
            if app.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                app.timeline.typing_users = user_ids;
            }
        }
        ClientEvent::CommandSucceeded { request_id } => {
            if app.timeline.composer.pending_request == Some(request_id) {
                let (_, effect) = screens::timeline::composer::update(
                    &mut app.timeline.composer,
                    screens::timeline::composer::Message::SendSucceeded,
                );
                return apply_composer_effect(app, effect);
            } else if app.timeline.composer.pending_attachment_request == Some(request_id) {
                // Attachment done — clear only the upload tracking; the
                // typed draft (if any) is untouched.
                app.timeline.composer.pending_attachment_request = None;
                app.timeline.composer.error = None;
            } else if app.timeline.pending_edit_request == Some(request_id) {
                app.timeline.editing = None;
                app.timeline.pending_edit_request = None;
            } else if app.timeline.pending_redact_request == Some(request_id) {
                app.timeline.confirm_delete = None;
                app.timeline.pending_redact_request = None;
            } else if app.timeline.pending_paginate_request == Some(request_id) {
                app.timeline.pending_paginate_request = None;
                app.timeline.loading_older = false;
            } else if app.call.pending.as_ref().is_some_and(|(id, ..)| *id == request_id) {
                // The optimistic CallStateUpdated already flipped the
                // banner; this just re-arms the buttons.
                app.call.pending = None;
            }
        }
        ClientEvent::CommandFailed { request_id, error } => {
            if app.timeline.composer.pending_request == Some(request_id) {
                let (_, effect) = screens::timeline::composer::update(
                    &mut app.timeline.composer,
                    screens::timeline::composer::Message::SendFailed(error),
                );
                return apply_composer_effect(app, effect);
            } else if app.timeline.composer.pending_attachment_request == Some(request_id) {
                app.timeline.composer.pending_attachment_request = None;
                app.timeline.composer.error = Some(error);
            } else if app.timeline.pending_edit_request == Some(request_id) {
                app.timeline.pending_edit_request = None;
                app.timeline.action_error = Some(error);
            } else if app.timeline.pending_redact_request == Some(request_id) {
                app.timeline.pending_redact_request = None;
                app.timeline.action_error = Some(error);
            } else if app.timeline.pending_paginate_request == Some(request_id) {
                app.timeline.pending_paginate_request = None;
                app.timeline.loading_older = false;
                app.timeline.action_error = Some(error);
            } else if app.call.pending.as_ref().is_some_and(|(id, ..)| *id == request_id) {
                // Keep the room id so the error renders only in the room
                // whose join/leave actually failed.
                if let Some((_, room_id, _)) = app.call.pending.take() {
                    app.call.error = Some((room_id, error));
                }
            }
        }
        ClientEvent::CrossSigningBootstrapNeedsFallback { url } => {
            app.verification.cross_signing_fallback_url = Some(url);
        }
        ClientEvent::CrossSigningBootstrapDone => {
            app.verification.cross_signing_fallback_url = None;
            app.verification.cross_signing_error = None;
        }
        ClientEvent::CrossSigningBootstrapFailed { reason } => {
            app.verification.cross_signing_fallback_url = None;
            app.verification.cross_signing_error = Some(reason);
        }
        ClientEvent::VerificationStateChanged(sas_state) => {
            app.verification.sas = Some(sas_state);
        }
        ClientEvent::KeyBackupNeedsRecovery => {
            app.verification.recovery_needs_key = true;
        }
        ClientEvent::RecoverySetupNeeded => {
            app.verification.recovery_setup_needed = true;
        }
        ClientEvent::RecoveryEnableProgress(stage) => {
            app.verification.recovery_enable_stage = Some(stage);
        }
        ClientEvent::RecoveryEnabled { recovery_key } => {
            app.verification.recovery_setup_needed = false;
            app.verification.recovery_enable_stage = None;
            app.verification.recovery_key_to_confirm = Some(recovery_key);
        }
        ClientEvent::RecoveryEnableFailed { reason } => {
            app.verification.recovery_enable_stage = None;
            app.verification.recovery_error = Some(reason);
        }
        ClientEvent::KeyBackupRestored => {
            app.verification.recovery_needs_key = false;
            app.verification.recovery_key_input.clear();
        }
        ClientEvent::KeyBackupFailed { reason } => {
            app.verification.recovery_error = Some(reason);
        }
        ClientEvent::CustomEmojiPacksUpdated(packs) => {
            let urls = image_urls_in_packs(&packs);
            app.emoji_packs = packs;
            app.emoji_shortcode_index = build_shortcode_index(&app.emoji_packs);
            ensure_media_fetched(app, urls);
        }
        ClientEvent::MediaFetched { request_id, bytes } => {
            if let Some(url) = app.media.pending.remove(&request_id) {
                app.media.pending_urls.remove(&url);
                if crate::media_cache::looks_like_svg(&bytes) {
                    app.media.mxc_svgs.insert(url, iced::widget::svg::Handle::from_memory(bytes));
                } else if crate::media_cache::looks_like_gif(&bytes) {
                    // Frame decode is CPU-heavy (full RGBA per frame; a chat
                    // GIF is easily 100ms+) — run it off the update thread.
                    // The url stays in pending_urls until GifDecoded lands so
                    // it isn't re-requested mid-decode.
                    app.media.pending_urls.insert(url.clone());
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                iced_gif::Frames::from_bytes(bytes.clone())
                                    .map(std::sync::Arc::new)
                                    .map_err(|_| iced::widget::image::Handle::from_bytes(bytes))
                            })
                            .await
                            .unwrap_or_else(|_| {
                                Err(iced::widget::image::Handle::from_bytes(Vec::new()))
                            })
                        },
                        move |result| Message::GifDecoded(url.clone(), result),
                    );
                } else {
                    app.media.images.insert(url, iced::widget::image::Handle::from_bytes(bytes));
                }
            }
        }
        ClientEvent::MediaFetchFailed { request_id, .. } => {
            // Negative-cache the URL — without this, the very next timeline
            // or room-list update re-requests it, once per sync tick, forever.
            if let Some(url) = app.media.pending.remove(&request_id) {
                app.media.pending_urls.remove(&url);
                app.media.failed_mxc.insert(url);
            }
        }
        ClientEvent::UrlPreviewFetched(preview) => {
            if let Some(image_mxc) = preview.image_mxc.clone() {
                ensure_media_fetched(app, [image_mxc]);
            }
            app.url_previews.insert(preview.url.clone(), Some(preview));
        }
        ClientEvent::UrlPreviewFailed { url } => {
            // Cached as "asked, nothing to show" so it isn't re-requested.
            app.url_previews.insert(url, None);
        }
        ClientEvent::TimelineStartReached { room_id } => {
            if app.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                app.timeline.reached_start = true;
                app.timeline.loading_older = false;
            }
        }
        ClientEvent::RoomNotificationModesUpdated(modes) => {
            app.notification_modes = modes.into_iter().collect();
        }
        ClientEvent::RoomNotificationModeChanged { room_id, mode } => {
            app.notification_modes.insert(room_id, mode);
        }
        ClientEvent::RoomNotificationModeCleared { room_id } => {
            app.notification_modes.remove(&room_id);
        }
        ClientEvent::KeywordHighlightsUpdated(keywords) => {
            app.keyword_highlights = keywords;
        }
        ClientEvent::CallStateUpdated(call_state) => {
            app.call.calls.insert(call_state.room_id.clone(), call_state);
        }
        _ => {}
    }
    Task::none()
}

fn perform_password_login(
    profile: String,
    homeserver: String,
    username: String,
    password: Zeroizing<String>,
) -> Task<Message> {
    Task::perform(
        async move {
            let paths = client_core::store::AppPaths::for_profile(&profile)
                .map_err(|e| e.to_string())?;
            client_core::session::login_password(&paths, &homeserver, &username, password)
                .await
                .map(|restored| crate::message::OpaqueClient(restored.client))
                .map_err(|e| e.to_string())
        },
        Message::LoginResult,
    )
}

fn perform_sso_login(
    profile: String,
    homeserver: String,
    identity_provider_id: Option<String>,
) -> Task<Message> {
    Task::perform(
        async move {
            let paths = client_core::store::AppPaths::for_profile(&profile)
                .map_err(|e| e.to_string())?;
            client_core::session::login_sso(&paths, &homeserver, identity_provider_id.as_deref())
                .await
                .map(|restored| crate::message::OpaqueClient(restored.client))
                .map_err(|e| e.to_string())
        },
        Message::LoginResult,
    )
}

fn perform_discover(homeserver: String) -> Task<Message> {
    Task::perform(
        async move { client_core::session::discover_login_flows(&homeserver).await },
        |result| match result.map_err(|e| e.to_string()) {
            Ok(flows) => Message::Login(screens::login::Message::FlowsDiscovered(flows)),
            Err(reason) => Message::Login(screens::login::Message::DiscoverFailed(reason)),
        },
    )
}

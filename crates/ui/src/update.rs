use client_core::events::ClientEvent;
use client_core::ClientCommand;
use iced::Task;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::message::Message;
use crate::screens;
use crate::state::App;

pub fn update(app: &mut App, message: Message) -> Task<Message> {
    // Glue for the inline video player: while one is playing, any message
    // at all may have reflowed the timeline (scrolls, arriving messages,
    // images finishing decode, panels toggling...), so re-probe where the
    // stage container sits after every update and let the probe handler
    // resync the native webview. Probe results themselves are exempt —
    // they'd re-trigger forever; `inline_video_bounds` re-issues probes
    // itself in the one case that needs it (a missed stage), bounded by
    // its miss limit.
    let is_probe_result = matches!(message, Message::InlineVideoBounds(_));
    let task = update_inner(app, message);
    if !is_probe_result && app.timeline.inline_video.is_some() {
        return Task::batch([
            task,
            crate::video_player::stage_bounds_probe().map(Message::InlineVideoBounds),
        ]);
    }
    task
}

fn update_inner(app: &mut App, message: Message) -> Task<Message> {
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
        Message::RoomList(screens::room_list::Message::SpaceClicked(space_id)) => {
            let name = app
                .room_list
                .rooms
                .iter()
                .find(|r| r.room_id == space_id)
                .map(|r| r.name.clone())
                .unwrap_or_else(|| space_id.clone());
            let request_id = Uuid::new_v4();
            app.space_explorer =
                Some(screens::space_explorer::State::open(space_id.clone(), name, request_id));
            send_cmd(app, ClientCommand::FetchSpaceHierarchy { space_id, from: None, request_id });
            Task::none()
        }
        Message::RoomList(screens::room_list::Message::RoomRightClicked(room_id)) => {
            if let Some(room) = app.room_list.rooms.iter().find(|r| r.room_id == room_id) {
                app.pending_room_action = Some(crate::state::RoomActionPrompt {
                    room_id: room.room_id.clone(),
                    room_name: room.name.clone(),
                    is_dm: room.is_dm,
                    rename_draft: room.name.clone(),
                });
            }
            Task::none()
        }
        Message::RoomList(screens::room_list::Message::NewRoomClicked) => {
            // A solo room, unencrypted (a personal scratch/test space — no
            // other members, so E2EE would only add key overhead). Opens on
            // the RoomCreated echo.
            send_cmd(app, ClientCommand::CreateRoom { encrypted: false, request_id: Uuid::new_v4() });
            Task::none()
        }
        Message::RoomList(screens::room_list::Message::NewDirectMessageClicked) => {
            // Open the user-search overlay fresh (no stale results from a
            // previous open).
            app.dm_search = Some(screens::dm_search::State::default());
            Task::none()
        }

        Message::ConfirmLeaveRoom(room_id) => {
            app.pending_room_action = None;
            send_cmd(app, ClientCommand::LeaveRoom { room_id: room_id.clone(), request_id: Uuid::new_v4() });
            forget_open_room(app, &room_id)
        }
        Message::ConfirmForgetRoom(room_id) => {
            app.pending_room_action = None;
            send_cmd(app, ClientCommand::ForgetRoom { room_id: room_id.clone(), request_id: Uuid::new_v4() });
            forget_open_room(app, &room_id)
        }
        Message::RoomRenameDraftChanged(name) => {
            if let Some(prompt) = app.pending_room_action.as_mut() {
                prompt.rename_draft = name;
            }
            Task::none()
        }
        Message::ConfirmRoomRename(room_id) => {
            // Empty/whitespace name would just clear the display name back to
            // the fallback — treat it as "no change" and only dismiss.
            let name = app
                .pending_room_action
                .as_ref()
                .map(|p| p.rename_draft.trim().to_string())
                .unwrap_or_default();
            app.pending_room_action = None;
            if !name.is_empty() {
                send_cmd(app, ClientCommand::SetRoomName { room_id, name, request_id: Uuid::new_v4() });
            }
            Task::none()
        }
        Message::CancelRoomAction => {
            app.pending_room_action = None;
            Task::none()
        }

        Message::Timeline(msg) => {
            let (task, effect) = screens::timeline::update(
                &mut app.timeline,
                msg,
                &app.spellcheck,
                app.chat.show_membership_events,
            );
            let task = task.map(Message::Timeline);
            let effect_task = apply_timeline_effect(app, effect);
            Task::batch([task, effect_task])
        }

        Message::Verification(msg) => {
            let effect = screens::verification::update(&mut app.verification, msg);
            apply_verification_effect(app, effect)
        }

        Message::SpaceExplorer(msg) => update_space_explorer(app, msg),
        Message::DmSearch(msg) => update_dm_search(app, msg),

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
                    // Cross-reference against "emoji pack entry" (ground truth
                    // shortcode/url) and any later "widget slot reused" warning
                    // — this line is the decoded identity actually stored under
                    // `url` in the cache.
                    tracing::info!(
                        url = %url,
                        gif_id = frames.id(),
                        frames = frames.frame_count(),
                        size = ?frames.first_frame_size(),
                        "gif decoded"
                    );
                    app.media.mxc_gifs.insert(url, frames);
                }
                // Corrupt GIF: fall back to the (first-frame-only) raster path.
                Err(raster) => {
                    tracing::warn!(url = %url, "gif decode failed, falling back to static raster");
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

        Message::PasteClipboard => {
            // A paste belongs to the surface being looked at: with Settings
            // or the space explorer overlaying the shell (each has its own
            // text inputs), or before login, don't aim it at the room
            // behind. Text pastes are unaffected either way — the clipboard
            // probe ignores them (see `clipboard_paste::read`).
            if app.route != crate::state::Route::Main
                || app.show_settings
                || app.space_explorer.is_some()
            {
                return Task::none();
            }
            match app.timeline.room_id.clone() {
                Some(room_id) => paste_clipboard_task(room_id),
                None => Task::none(),
            }
        }
        Message::FileDropped(path) => {
            // Same surface rule as paste: a drop targets the open room's
            // composer only while the main shell is actually showing it.
            if app.route != crate::state::Route::Main
                || app.show_settings
                || app.space_explorer.is_some()
            {
                return Task::none();
            }
            match app.timeline.room_id.clone() {
                Some(room_id) => dropped_file_task(room_id, path),
                None => Task::none(),
            }
        }
        Message::AttachmentsReadFor { room_id, files, failed } => {
            if app.timeline.room_id.as_deref() != Some(room_id.as_str()) {
                tracing::info!("attachments read for a room no longer open; dropped");
                app.timeline.composer.error =
                    Some("Attachment cancelled — the room changed while reading the files".into());
                return Task::none();
            }
            if failed > 0 {
                let plural = if failed == 1 { "" } else { "s" };
                app.timeline.composer.error = Some(format!(
                    "{failed} item{plural} couldn't be read (folders can't be attached)"
                ));
            }
            // Stage each file through the composer's normal AttachmentPicked
            // path (mime sniffing, duplicate skip). Nothing sends yet —
            // Enter/Send dispatches the batch.
            let staged_any = !files.is_empty();
            let mut tasks: Vec<Task<Message>> = files
                .into_iter()
                .map(|(filename, bytes)| {
                    update_inner(
                        app,
                        Message::Timeline(screens::timeline::Message::Composer(
                            screens::timeline::composer::Message::AttachmentPicked(Ok((
                                filename, bytes,
                            ))),
                        )),
                    )
                })
                .collect();
            if staged_any {
                // Focus the input so "paste → type a caption → Enter" flows
                // without an extra click (Enter submits via on_submit).
                tasks.push(iced::widget::operation::focus(
                    screens::timeline::composer::input_id(),
                ));
            }
            Task::batch(tasks)
        }

        Message::CloseZoom => {
            app.zoomed_image = None;
            Task::none()
        }
        Message::DownloadZoomedImage => {
            // Re-fetch the original bytes (fast — the SDK has them cached on
            // disk from displaying the image) rather than reconstructing them
            // from a decoded handle; the save dialog opens when they land in
            // the `MediaFetched` handler below. Works the same for raster,
            // GIF, and SVG.
            if let Some(url) = app.zoomed_image.clone() {
                let request_id = Uuid::new_v4();
                app.media.download_requests.insert(request_id);
                send_cmd(app, ClientCommand::FetchMedia { mxc_url: url, request_id });
            }
            Task::none()
        }
        Message::EscapePressed => {
            // Escape also cancels an active autoscroll — but the gated
            // subscription that ends it on any key press already handles that;
            // clearing here too is just belt-and-braces (and harmless when
            // idle). The lightbox is the real target: a press on the image
            // itself pans rather than closing, so it needs a keyboard exit
            // too. Everything else keeps its explicit close affordances.
            app.timeline.autoscroll = None;
            app.zoomed_image = None;
            Task::none()
        }

        Message::InlineVideoScale(scale) => {
            if let Some(inline) = app.timeline.inline_video.as_mut() {
                inline.scale = Some(scale);
            }
            // The wrapper's probe follows this message; once bounds land
            // too, the webview opens.
            Task::none()
        }
        Message::InlineVideoOpened(result) => {
            if let Err(reason) = result {
                tracing::warn!(%reason, "inline video player failed to start");
                if let Some(inline) = app.timeline.inline_video.as_mut() {
                    inline.error = Some(reason);
                }
                // Belt and braces — `open` cleans up after itself on error.
                return close_native_player();
            }
            Task::none()
        }
        Message::InlineVideoBounds(probed) => inline_video_bounds(app, probed),
        Message::ReclaimAppFocus => {
            // The user clicked the app off the video — return Win32 keyboard
            // focus to the app window so the composer types again (the
            // embedded webview grabs it on click; see `reclaim_focus`).
            iced::window::latest().then(|maybe_id| match maybe_id {
                Some(id) => iced::window::run(id, |handle| {
                    crate::video_player::reclaim_focus(handle);
                })
                .map(|_| Message::Noop),
                None => Task::none(),
            })
        }
        Message::WindowResized(_) => {
            // Text rewraps to the new width — every row's on-screen position
            // changes legitimately, so the scroll anchor's stored geometry
            // is meaningless until the next scroll re-learns it. (The inline
            // video player reglues itself through the probe this message
            // triggers, like any other reflow.)
            app.timeline.scroll_anchor = None;
            Task::none()
        }
        Message::CursorMoved(position) => {
            // Cheap: just remember where the pointer is, so a right-click menu
            // can open there (the press event itself carries no coordinates),
            // and so an active autoscroll knows how far the cursor has drifted
            // from its anchor.
            app.cursor_position = position;
            Task::none()
        }
        Message::AutoscrollTick => autoscroll_tick(app),
        Message::AutoscrollEnd => {
            app.timeline.autoscroll = None;
            Task::none()
        }

        Message::ToggleSettings => {
            app.show_settings = !app.show_settings;
            // Re-read the autostart registry state each time the panel opens
            // so the toggle reflects reality even if an uninstaller or the
            // user cleared the Run key since launch. Only the General tab is
            // rebuilt — leaving the Appearance tab's in-progress hex drafts
            // (and the active tab) untouched.
            if app.show_settings {
                app.settings.general = screens::settings::general::State::new();
            }
            Task::none()
        }
        Message::SettingsResizeStarted => {
            app.settings_resize_drag = Some(crate::state::ResizeDrag {
                size_at_start: app.settings_panel_size,
                anchor: None,
            });
            Task::none()
        }
        Message::SettingsResizeDragged(cursor) => {
            let mut new_size = None;
            if let Some(drag) = &mut app.settings_resize_drag {
                match drag.anchor {
                    None => drag.anchor = Some(cursor),
                    Some(anchor) => {
                        new_size = Some(iced::Size {
                            width: (drag.size_at_start.width + (cursor.x - anchor.x))
                                .max(crate::state::MIN_SETTINGS_WIDTH),
                            height: (drag.size_at_start.height + (cursor.y - anchor.y))
                                .max(crate::state::MIN_SETTINGS_HEIGHT),
                        });
                    }
                }
            }
            if let Some(size) = new_size {
                app.settings_panel_size = size;
            }
            Task::none()
        }
        Message::SettingsResizeReleased => {
            app.settings_resize_drag = None;
            Task::none()
        }

        Message::Settings(msg) => {
            let (task, effect) = screens::settings::update(
                &mut app.settings,
                &mut app.theme,
                &mut app.privacy,
                &mut app.encryption,
                &mut app.spellcheck,
                &mut app.chat,
                &app.profile,
                msg,
            );
            // Settings is the only place `theme` changes; rebuild the cached
            // iced::Theme here so the per-frame `.theme()` closure just clones
            // it instead of regenerating the palette every update cycle.
            app.built_theme = app.theme.to_iced_theme();
            let task = task.map(Message::Settings);
            let effect_task = apply_settings_effect(app, effect);
            Task::batch([task, effect_task])
        }

        Message::Noop => Task::none(),
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
            // Blank input = "verify this device" (the form's placeholder
            // promises it): map it to the account's own user id, which the
            // worker treats as self-verification.
            let user_id = if user_id.is_empty() {
                match &app.own_user_id {
                    Some(id) => id.clone(),
                    None => return Task::none(),
                }
            } else {
                user_id
            };
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

/// Space-explorer overlay actions. Handled at the app level (no screen
/// `update` of its own) because everything it does — fetching hierarchy
/// pages, joining, opening a room — goes through the command channel or
/// `select_room`.
fn update_space_explorer(app: &mut App, msg: screens::space_explorer::Message) -> Task<Message> {
    use screens::space_explorer::Message as Msg;
    match msg {
        Msg::Close => {
            app.space_explorer = None;
            Task::none()
        }
        Msg::Back => {
            if let Some(explorer) = &mut app.space_explorer {
                explorer.stack.pop();
                if explorer.stack.is_empty() {
                    app.space_explorer = None;
                }
            }
            Task::none()
        }
        Msg::EnterSpace { space_id, name } => {
            let request_id = Uuid::new_v4();
            let Some(explorer) = &mut app.space_explorer else { return Task::none() };
            explorer.stack.push(screens::space_explorer::Level::loading(
                space_id.clone(),
                name,
                request_id,
            ));
            send_cmd(app, ClientCommand::FetchSpaceHierarchy { space_id, from: None, request_id });
            Task::none()
        }
        Msg::Join { room_id, via } => {
            let request_id = Uuid::new_v4();
            let Some(explorer) = &mut app.space_explorer else { return Task::none() };
            // One join at a time — rows render their button inert while
            // one is pending, but a stale press could still race in.
            if explorer.pending_join.is_some() {
                return Task::none();
            }
            explorer.pending_join = Some((request_id, room_id.clone()));
            explorer.join_error = None;
            send_cmd(app, ClientCommand::JoinRoom { room_id_or_alias: room_id, via, request_id });
            Task::none()
        }
        Msg::Open(room_id) => {
            app.space_explorer = None;
            select_room(app, room_id)
        }
        Msg::LoadMore => {
            let Some(explorer) = &mut app.space_explorer else { return Task::none() };
            let Some(level) = explorer.stack.last_mut() else { return Task::none() };
            if level.pending_request.is_some() {
                return Task::none();
            }
            let Some(from) = level.next_batch.clone() else { return Task::none() };
            let request_id = Uuid::new_v4();
            level.pending_request = Some(request_id);
            let space_id = level.space_id.clone();
            send_cmd(
                app,
                ClientCommand::FetchSpaceHierarchy { space_id, from: Some(from), request_id },
            );
            Task::none()
        }
        Msg::Retry => {
            let Some(explorer) = &mut app.space_explorer else { return Task::none() };
            let Some(level) = explorer.stack.last_mut() else { return Task::none() };
            // Back to a fresh page 1 — a failed load-more can't resume from
            // its token (the failure may have invalidated it server-side).
            let request_id = Uuid::new_v4();
            level.children.clear();
            level.next_batch = None;
            level.error = None;
            level.pending_request = Some(request_id);
            let space_id = level.space_id.clone();
            send_cmd(app, ClientCommand::FetchSpaceHierarchy { space_id, from: None, request_id });
            Task::none()
        }
    }
}

fn update_dm_search(app: &mut App, msg: screens::dm_search::Message) -> Task<Message> {
    use screens::dm_search::Message as Msg;
    match msg {
        Msg::Close => {
            app.dm_search = None;
            Task::none()
        }
        Msg::QueryChanged(query) => {
            let Some(state) = app.dm_search.as_mut() else { return Task::none() };
            state.query = query;
            state.error = None;
            state.generation += 1;
            let generation = state.generation;
            // Empty box: clear immediately, don't hit the server.
            if state.query.trim().is_empty() {
                state.results.clear();
                state.pending = false;
                state.last_request_id = None;
                return Task::none();
            }
            // Debounce: only the newest keystroke's timer still matches
            // `generation` when it fires (see `dm_search` module docs).
            Task::future(async move {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                Message::DmSearch(Msg::Debounced(generation))
            })
        }
        Msg::Debounced(generation) => {
            let Some(state) = app.dm_search.as_mut() else { return Task::none() };
            // Superseded by a newer keystroke, or the box was cleared.
            if generation != state.generation || state.query.trim().is_empty() {
                return Task::none();
            }
            let request_id = Uuid::new_v4();
            state.last_request_id = Some(request_id);
            state.pending = true;
            let query = state.query.clone();
            send_cmd(app, ClientCommand::SearchUsers { query, request_id });
            Task::none()
        }
        Msg::ResultClicked(user_id) => {
            // Close the overlay now: the DirectMessageReady echo will
            // `select_room`, so leaving it open would switch the room out
            // from under it (same as the space explorer's Open).
            app.dm_search = None;
            send_cmd(
                app,
                ClientCommand::OpenDirectMessage {
                    user_id,
                    encrypted: app.encryption.encrypt_direct_messages,
                    request_id: Uuid::new_v4(),
                },
            );
            Task::none()
        }
    }
}

fn apply_settings_effect(app: &mut App, effect: screens::settings::Effect) -> Task<Message> {
    match effect {
        screens::settings::Effect::None => {}
        screens::settings::Effect::Logout => send_cmd(app, ClientCommand::Logout),
        screens::settings::Effect::SetDefaultNotificationMode { scope, mode } => {
            send_cmd(
                app,
                ClientCommand::SetDefaultNotificationMode { scope, mode, request_id: Uuid::new_v4() },
            );
        }
        // The Security tab hosts the verification module's recovery UI; run
        // its message through that module's own update + effect handler.
        screens::settings::Effect::Verification(msg) => {
            let effect = screens::verification::update(&mut app.verification, msg);
            return apply_verification_effect(app, effect);
        }
    }
    Task::none()
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
            send_cmd(
                app,
                ClientCommand::OpenDirectMessage {
                    user_id,
                    encrypted: app.encryption.encrypt_direct_messages,
                    request_id: Uuid::new_v4(),
                },
            );
            Task::none()
        }
        screens::timeline::Effect::CreateRoomWith(user_id) => {
            send_cmd(
                app,
                ClientCommand::CreateRoomWith {
                    user_id,
                    encrypted: app.encryption.encrypt_rooms,
                    request_id: Uuid::new_v4(),
                },
            );
            Task::none()
        }
        screens::timeline::Effect::PlayVideo { event_id, video, title } => {
            app.timeline.inline_video = Some(screens::timeline::InlineVideo {
                event_id,
                video,
                title,
                error: None,
                scale: None,
                live: false,
                synced: None,
                misses: 0,
            });
            // The webview is created once both the scale factor (fetched
            // here) and the stage's first bounds probe (fired by the update
            // wrapper for this very message) have landed. Tear down any
            // previous player first — one video at a time.
            Task::batch([
                close_native_player(),
                iced::window::latest().then(|maybe_id| match maybe_id {
                    Some(id) => {
                        iced::window::scale_factor(id).map(Message::InlineVideoScale)
                    }
                    None => Task::none(),
                }),
            ])
        }
        screens::timeline::Effect::StopVideo => {
            app.timeline.inline_video = None;
            close_native_player()
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
        screens::timeline::Effect::ToggleAutoscroll => {
            // Toggle: a second middle-click over the chat turns it back off.
            // On activation, anchor at the live cursor — the middle-press
            // itself carries no coordinates, but `cursor_position` was set by
            // the moves that brought the pointer here.
            app.timeline.autoscroll =
                if app.timeline.autoscroll.is_some() { None } else { Some(app.cursor_position) };
            Task::none()
        }
    }
}

/// One frame of middle-click autoscroll: scroll the timeline by a step
/// proportional to how far the cursor sits from the anchor point, past a small
/// dead zone. The scrollable measures its offset from the bottom (newer
/// content = smaller offset), so a cursor *below* the anchor scrolls toward
/// newer messages — hence the sign flip when handing the step to `scroll_by`.
/// Called ~60×/s while `autoscroll` is set; a no-op inside the dead zone.
fn autoscroll_tick(app: &App) -> Task<Message> {
    /// Radius around the anchor where the view holds still, so a click that
    /// barely nudges the pointer doesn't start creeping.
    const DEAD_ZONE: f32 = 14.0;
    /// Per-frame pixels of scroll per pixel of overshoot past the dead zone.
    const SPEED: f32 = 0.14;
    /// Cap on per-frame scroll, so flinging the pointer to a screen edge
    /// glides fast but stays legible rather than teleporting.
    const MAX_STEP: f32 = 42.0;

    let Some(origin) = app.timeline.autoscroll else {
        return Task::none();
    };
    let dy = app.cursor_position.y - origin.y;
    if dy.abs() <= DEAD_ZONE {
        return Task::none();
    }
    let step = ((dy.abs() - DEAD_ZONE) * SPEED).min(MAX_STEP) * dy.signum();
    iced::widget::operation::scroll_by(
        screens::timeline::timeline_scroll_id(),
        iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: -step },
    )
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

/// How many consecutive failed stage probes before playback stops. The
/// first layout of a fresh placeholder can legitimately miss once or
/// twice; a sustained run means the card left the tree for good (message
/// redacted, filtered out by search, timeline reset).
const INLINE_VIDEO_MISS_LIMIT: u32 = 30;

/// Applies a resolved stage-geometry probe (see
/// `video_player::stage_bounds_probe`): creates the webview once geometry
/// and scale are first known, then keeps regluing it as the stage moves.
fn inline_video_bounds(
    app: &mut App,
    probed: Option<(iced::Rectangle, Option<iced::Rectangle>)>,
) -> Task<Message> {
    // Every iced overlay draws *under* the native webview, so while one is
    // open the video hides (audio keeps playing) instead of covering it.
    let overlay_open = app.show_settings
        || app.zoomed_image.is_some()
        || app.space_explorer.is_some()
        || app.pending_room_action.is_some();

    let Some(inline) = app.timeline.inline_video.as_mut() else {
        // Stopped while the probe was in flight; the close already ran.
        return Task::none();
    };

    match probed {
        Some((full, visible)) => {
            inline.misses = 0;
            let visible = if overlay_open { None } else { visible };
            if inline.live {
                // Nothing moved since the last sync → no task. This is the
                // loop's resting state: a sync's completion message triggers
                // exactly one more probe, which lands here and goes quiet.
                if inline.synced == Some((full, visible)) {
                    return Task::none();
                }
                inline.synced = Some((full, visible));
                let scale = inline.scale.unwrap_or(1.0);
                return iced::window::latest().then(move |maybe_id| match maybe_id {
                    Some(id) => iced::window::run(id, move |_handle| {
                        crate::video_player::sync_bounds(full, visible, scale);
                    })
                    .map(|_| Message::Noop),
                    None => Task::none(),
                });
            }
            if inline.error.is_some() {
                // The stage is showing the failure fallback; nothing to glue.
                return Task::none();
            }
            let Some(scale) = inline.scale else {
                // Bounds beat the scale-factor fetch; its arrival re-probes.
                return Task::none();
            };
            inline.live = true;
            inline.synced = Some((full, visible));
            open_inline_player(&app.profile, inline.video.clone(), full, visible, scale)
        }
        None => {
            inline.misses += 1;
            if inline.misses > INLINE_VIDEO_MISS_LIMIT {
                app.timeline.inline_video = None;
                return close_native_player();
            }
            // Re-probe straight away: before the webview exists this is
            // how the open path polls for the placeholder's first layout,
            // and after, how a vanished card counts up to the limit above
            // instead of leaving hidden audio playing forever. Bounded by
            // the miss limit — the update wrapper doesn't re-probe on
            // probe results, so this is the only repeat path.
            let hide = if inline.live {
                // The native player no longer reflects any known geometry;
                // whatever probe next finds the stage must sync it.
                inline.synced = None;
                iced::window::latest().then(|maybe_id| match maybe_id {
                    Some(id) => iced::window::run(id, |_handle| {
                        crate::video_player::hide();
                    })
                    .map(|_| Message::Noop),
                    None => Task::none(),
                })
            } else {
                Task::none()
            };
            Task::batch([
                hide,
                crate::video_player::stage_bounds_probe().map(Message::InlineVideoBounds),
            ])
        }
    }
}

/// Builds the native webview over the probed stage geometry, on the
/// event-loop thread (where WebView2 creation must happen).
fn open_inline_player(
    profile: &str,
    video: crate::video_player::EmbedVideo,
    full: iced::Rectangle,
    visible: Option<iced::Rectangle>,
    scale: f32,
) -> Task<Message> {
    // WebView2 refuses to put its profile data next to the exe; park it
    // with the rest of this profile's on-disk state.
    let data_dir = client_core::store::AppPaths::for_profile(profile)
        .ok()
        .map(|paths| paths.root.join("webview-data"));

    iced::window::latest().then(move |maybe_id| {
        let Some(id) = maybe_id else {
            return Task::none();
        };
        let video = video.clone();
        let data_dir = data_dir.clone();
        iced::window::run(id, move |handle| {
            crate::video_player::open(handle, &video, full, visible, scale, data_dir)
        })
        .map(Message::InlineVideoOpened)
    })
}

/// Drops the native webview (if any) on its owning thread. Safe to fire
/// even when nothing is open.
fn close_native_player() -> Task<Message> {
    iced::window::latest().then(|maybe_id| match maybe_id {
        Some(id) => iced::window::run(id, |_handle| {
            crate::video_player::close();
        })
        .map(|_| Message::Noop),
        None => Task::none(),
    })
}

fn mark_open_room_read(app: &mut App) {
    if let Some(room_id) = app.timeline.room_id.clone() {
        // Privacy: when read receipts are off, the worker sends a private
        // receipt instead of the federated public one — your unread state
        // still clears locally, but others don't see what you've read.
        send_cmd(
            app,
            ClientCommand::MarkRoomRead {
                room_id,
                public_receipt: app.privacy.send_read_receipts,
            },
        );
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
        ComposerEffect::SendAttachment {
            filename,
            bytes,
            mime,
            caption,
            mentioned_user_ids,
            reply_to_event_id,
        } => {
            send_attachment(app, filename, bytes, mime, caption, mentioned_user_ids, reply_to_event_id);
            Task::none()
        }
        ComposerEffect::Typing(typing) => {
            set_typing(app, typing);
            Task::none()
        }
        ComposerEffect::EnsureEmojiFetched(emojis) => ensure_emoji_fetched(app, emojis),
        ComposerEffect::EnsureStickersFetched => {
            // Pack sticker images are already fetched on pack load; the
            // collected ones (loaded from disk) may not be yet.
            let urls: Vec<String> =
                app.sticker_collection.iter().map(|s| s.url.clone()).collect();
            ensure_media_fetched(app, urls);
            Task::none()
        }
        ComposerEffect::SendSticker { url, body, width, height } => {
            send_sticker(app, url, body, width, height);
            Task::none()
        }
        ComposerEffect::EmojiUsed(key) => record_emoji_use(app, key),
        ComposerEffect::OpenContextMenu => {
            // Anchor the edit menu at the pointer — the composer can't see the
            // window-global cursor, so it's snapshotted here.
            app.timeline.composer.context_menu = Some(app.cursor_position);
            Task::none()
        }
        ComposerEffect::ClipboardEdit(edit) => {
            // Right-click Cut/Copy: replay the native chord at the focused
            // input (which owns the selection app code can't see).
            crate::synthetic_input::edit(edit);
            Task::none()
        }
        ComposerEffect::PasteFromClipboard => match app.timeline.room_id.clone() {
            Some(room_id) => paste_menu_task(room_id),
            None => Task::none(),
        },
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

fn send_attachment(
    app: &mut App,
    filename: String,
    bytes: Vec<u8>,
    mime: String,
    caption: Option<String>,
    mentioned_user_ids: Vec<String>,
    reply_to_event_id: Option<String>,
) {
    let Some(room_id) = app.timeline.room_id.clone() else { return };
    let Some(cmd_tx) = &app.cmd_tx else { return };
    let request_id = Uuid::new_v4();
    // NOT pending_request: sharing the text-send slot would make the
    // attachment's CommandSucceeded wipe an unsent draft via SendSucceeded.
    app.timeline.composer.pending_attachment_request = Some(request_id);
    let _ = cmd_tx.send(ClientCommand::SendAttachment {
        room_id,
        filename,
        bytes,
        mime,
        caption,
        mentioned_user_ids,
        reply_to_event_id,
        request_id,
    });
}

fn send_sticker(app: &mut App, url: String, body: String, width: Option<u32>, height: Option<u32>) {
    let Some(room_id) = app.timeline.room_id.clone() else { return };
    let Some(cmd_tx) = &app.cmd_tx else { return };
    let request_id = Uuid::new_v4();
    // Separate slot, like attachments: a sticker send must never trigger the
    // text-draft reset a `SendSucceeded` runs. `mimetype` is left to the
    // homeserver — the media is already hosted and clients sniff it.
    app.timeline.composer.pending_sticker_request = Some(request_id);
    let _ = cmd_tx.send(ClientCommand::SendSticker {
        room_id,
        url,
        body,
        width,
        height,
        mimetype: None,
        request_id,
    });
}

/// Adds any `m.sticker` items in a fresh timeline snapshot to the
/// grow-with-use collection — newly-seen ones prepended (most-recent first),
/// deduped by url, capped at [`crate::state::MAX_COLLECTED_STICKERS`].
/// Returns whether the collection changed, so the caller only rewrites the
/// on-disk file when it did (snapshots arrive on every sync tick).
fn harvest_stickers(app: &mut App, items: &[client_core::events::TimelineItem]) -> bool {
    use client_core::events::TimelineItemContent;
    let mut fresh: Vec<crate::state::CollectedSticker> = Vec::new();
    for item in items {
        if let TimelineItemContent::Sticker { url, body, width, height } = &item.content {
            let known = app.sticker_collection.iter().any(|s| &s.url == url)
                || fresh.iter().any(|s| &s.url == url);
            if !known {
                fresh.push(crate::state::CollectedSticker {
                    url: url.clone(),
                    body: body.clone(),
                    width: *width,
                    height: *height,
                });
            }
        }
    }
    if fresh.is_empty() {
        return false;
    }
    // Items run oldest→newest; reverse so the newest lands at the front.
    fresh.reverse();
    for sticker in fresh {
        app.sticker_collection.insert(0, sticker);
    }
    app.sticker_collection.truncate(crate::state::MAX_COLLECTED_STICKERS);
    true
}

/// Persists the sticker collection off-thread (same fire-and-forget shape as
/// [`record_emoji_use`]).
fn persist_sticker_collection(app: &App) -> Task<Message> {
    let Some(path) = crate::state::sticker_collection_path(&app.profile) else {
        return Task::none();
    };
    let Ok(contents) = serde_json::to_string(&app.sticker_collection) else {
        return Task::none();
    };
    Task::future(async move {
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(path, contents).await;
        Message::Noop
    })
}

/// Records the open room as this profile's "last room" (fire-and-forget, same
/// shape as [`record_emoji_use`]) so the next launch can reopen it.
fn persist_last_room(profile: &str, room_id: &str) -> Task<Message> {
    let Some(path) = crate::state::last_room_path(profile) else { return Task::none() };
    let contents = room_id.to_string();
    Task::future(async move {
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(path, contents).await;
        Message::Noop
    })
}

/// Clears the persisted "last room" (fire-and-forget) — used when the user
/// leaves or forgets the room that's currently open, so the next launch
/// doesn't try to reopen a room they deliberately walked away from.
fn clear_last_room(profile: &str) -> Task<Message> {
    let Some(path) = crate::state::last_room_path(profile) else { return Task::none() };
    Task::future(async move {
        let _ = tokio::fs::remove_file(path).await;
        Message::Noop
    })
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
    // Privacy: with typing notifications disabled we broadcast nothing at all
    // — not even a "stopped typing" — so the setting is a clean off switch.
    // (The rare stuck-indicator case, toggling this off mid-compose, self-
    // heals via the server-side typing timeout and on message send.)
    if !app.privacy.send_typing_notifications {
        return;
    }
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
                client_core::events::TimelineItemContent::Image { url, .. }
                | client_core::events::TimelineItemContent::Sticker { url, .. } => Some(url.as_str()),
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

    // Privacy: link previews contact the homeserver's OG proxy and, for
    // tweets/Steam links, third-party APIs directly (leaking your IP and what
    // you're reading). When disabled we still compute `first_urls` — so link
    // rendering and a later opt-in both work — but fetch nothing.
    if !app.privacy.enable_link_previews {
        return (Task::none(), first_urls);
    }

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

/// Unicode emoji typed directly into message text (as opposed to sent as a
/// reaction) need the same Twemoji fetch — otherwise `render_text_body`
/// only ever shows the font-fallback glyph for them, never the SVG every
/// other emoji surface in the app renders.
fn unicode_body_emojis(
    items: &[client_core::events::TimelineItem],
    media: &crate::media_cache::State,
) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match &item.content {
            client_core::events::TimelineItemContent::Text(body) => Some(body.as_str()),
            _ => None,
        })
        .flat_map(screens::timeline::unicode_emojis_in)
        .filter(|emoji| !media.is_emoji_known(emoji))
        .map(str::to_owned)
        .collect()
}

/// Probes the clipboard off-thread (see `clipboard_paste::read`) and reads
/// any pasted files' bytes, resolving to `AttachmentsReadFor` (staging).
/// Bound to the room open at Ctrl+V time for the same reason the file
/// dialog is.
fn paste_clipboard_task(room_id: String) -> Task<Message> {
    Task::perform(
        async move {
            let pasted = tokio::task::spawn_blocking(crate::clipboard_paste::read)
                .await
                .unwrap_or(crate::clipboard_paste::Pasted::None);
            match pasted {
                crate::clipboard_paste::Pasted::None => (Vec::new(), 0),
                crate::clipboard_paste::Pasted::Image { filename, bytes } => {
                    (vec![(filename, bytes)], 0)
                }
                crate::clipboard_paste::Pasted::Files(paths) => read_pasted_paths(paths).await,
            }
        },
        move |(files, failed)| {
            if files.is_empty() && failed == 0 {
                // Nothing attachable — the everyday text paste. Stay silent.
                Message::Noop
            } else {
                Message::AttachmentsReadFor { room_id: room_id.clone(), files, failed }
            }
        },
    )
}

/// Reads the bytes behind clipboard-pasted file paths, returning the readable
/// `(filename, bytes)` pairs plus a count of the ones that failed (folders,
/// permission errors). Shared by the Ctrl+V and right-click-Paste flows.
async fn read_pasted_paths(
    paths: Vec<std::path::PathBuf>,
) -> (Vec<(String, Vec<u8>)>, usize) {
    let mut files = Vec::new();
    let mut failed = 0usize;
    for path in paths {
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let filename = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "attachment".to_string());
                files.push((filename, bytes));
            }
            Err(error) => {
                // Folders land here too (fs::read refuses them).
                tracing::warn!(
                    %error,
                    path = %path.display(),
                    "pasted path not readable; skipped"
                );
                failed += 1;
            }
        }
    }
    (files, failed)
}

/// Where a right-click paste's clipboard content is headed once read.
enum PasteRoute {
    Text(String),
    Attach { files: Vec<(String, Vec<u8>)>, failed: usize },
    Nothing,
}

/// Right-click **Paste**: probes the clipboard off-thread — text included,
/// unlike Ctrl+V (see `clipboard_paste::read_for_menu`) — and routes it. Text
/// is appended to the draft via `InsertText` (the menu can't lean on the
/// focused widget to paste text the way Ctrl+V does); files/image stage as
/// attachments, matching Ctrl+V. Bound to the room open at click time.
fn paste_menu_task(room_id: String) -> Task<Message> {
    use crate::clipboard_paste::PastedForMenu;
    Task::perform(
        async move {
            let pasted = tokio::task::spawn_blocking(crate::clipboard_paste::read_for_menu)
                .await
                .unwrap_or(PastedForMenu::None);
            match pasted {
                PastedForMenu::Text(text) => PasteRoute::Text(text),
                PastedForMenu::Image { filename, bytes } => {
                    PasteRoute::Attach { files: vec![(filename, bytes)], failed: 0 }
                }
                PastedForMenu::Files(paths) => {
                    let (files, failed) = read_pasted_paths(paths).await;
                    PasteRoute::Attach { files, failed }
                }
                PastedForMenu::None => PasteRoute::Nothing,
            }
        },
        move |route| match route {
            PasteRoute::Text(text) => Message::Timeline(screens::timeline::Message::Composer(
                screens::timeline::composer::Message::InsertText(text),
            )),
            PasteRoute::Attach { files, failed } => {
                if files.is_empty() && failed == 0 {
                    Message::Noop
                } else {
                    Message::AttachmentsReadFor { room_id: room_id.clone(), files, failed }
                }
            }
            PasteRoute::Nothing => Message::Noop,
        },
    )
}

/// Reads one drag-and-dropped file and resolves to `AttachmentsReadFor`
/// (staging), bound to the room open at drop time like the paste and picker
/// paths. One task per file — the OS delivers a multi-file drop as a burst
/// of `FileDropped` events.
fn dropped_file_task(room_id: String, path: std::path::PathBuf) -> Task<Message> {
    Task::perform(
        async move {
            match tokio::fs::read(&path).await {
                Ok(bytes) => {
                    let filename = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "attachment".to_string());
                    (vec![(filename, bytes)], 0)
                }
                Err(error) => {
                    // Folders land here too (fs::read refuses them).
                    tracing::warn!(
                        %error,
                        path = %path.display(),
                        "dropped path not readable; skipped"
                    );
                    (Vec::new(), 1)
                }
            }
        },
        move |(files, failed)| Message::AttachmentsReadFor {
            room_id: room_id.clone(),
            files,
            failed,
        },
    )
}

fn pick_attachment_task(room_id: String) -> Task<Message> {
    Task::perform(
        async {
            // Multi-select: files stage as chips and send together on
            // Enter, so picking several at once is the natural unit.
            match rfd::AsyncFileDialog::new().pick_files().await {
                Some(picked) => {
                    let mut files = Vec::new();
                    for file in picked {
                        let filename = file.file_name();
                        let bytes = file.read().await;
                        files.push((filename, bytes));
                    }
                    files
                }
                None => Vec::new(),
            }
        },
        move |files| {
            if files.is_empty() {
                // Cancelling the dialog is a normal action, not an error to
                // pin above the composer.
                Message::Noop
            } else {
                Message::AttachmentsReadFor { room_id: room_id.clone(), files, failed: 0 }
            }
        },
    )
}

/// Switches the open room: tells the sync worker to close the previous
/// room's timeline (if any) and open the newly selected one, and clears the
/// timeline pane immediately so stale messages don't flash while the new
/// room's snapshot is in flight.
fn select_room(app: &mut App, room_id: String) -> Task<Message> {
    // Clicking the already-open room must be a no-op — the reset below
    // would wipe a typed-but-unsent draft and reload the whole timeline.
    if app.room_list.selected_room_id.as_deref() == Some(room_id.as_str()) {
        return Task::none();
    }
    // Any genuine room open — click, new DM, restored session — cancels a
    // still-pending safe-state restore so it can never fire on top of it.
    app.pending_restore_room = None;
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
    // A jump deferred in the previous room must not fire in this one.
    app.timeline.pending_jump = None;
    app.timeline.scroll_anchor = None;
    // An autoscroll anchor belongs to the room being left; its scroll target
    // is meaningless in the new one.
    app.timeline.autoscroll = None;
    app.timeline.last_content_height = 0.0;
    app.timeline.last_from_bottom = 0.0;
    app.timeline.last_seen_newest = None;
    app.timeline.suppress_unread_divider = false;
    app.timeline.power_tags.clear();
    // Roster selection / message highlight / open member menu are per-room.
    app.timeline.selected_member = None;
    app.timeline.highlighted_member = None;
    app.timeline.member_menu = None;
    app.zoomed_image = None;
    // An inline video belongs to a message in the room being left — its
    // card is gone from the new timeline, so stop it (the miss-counting
    // fallback would get there too, just slower and with lingering audio).
    let close_video = if app.timeline.inline_video.take().is_some() {
        close_native_player()
    } else {
        Task::none()
    };

    let reset_scroll = Task::batch([
        close_video,
        // Remember this room as the one to reopen next launch (safe-state).
        persist_last_room(&app.profile, &room_id),
        iced::widget::operation::scroll_to(
            screens::timeline::timeline_scroll_id(),
            iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: 0.0 },
        ),
    ]);

    let Some(cmd_tx) = &app.cmd_tx else { return reset_scroll };

    if let Some(previous_room_id) = previous {
        if previous_room_id != room_id {
            let _ = cmd_tx.send(ClientCommand::CloseRoom { room_id: previous_room_id });
        }
    }
    let _ = cmd_tx.send(ClientCommand::OpenRoom { room_id });
    reset_scroll
}

/// When a room the user just left/forgot is the one on screen, clear the
/// timeline pane and deselect it so the "Select a room" placeholder shows
/// (the sidebar entry itself drops on the next `RoomListUpdated`). No-op when
/// a different room was open.
fn forget_open_room(app: &mut App, room_id: &str) -> Task<Message> {
    if app.room_list.selected_room_id.as_deref() != Some(room_id) {
        return Task::none();
    }
    app.room_list.selected_room_id = None;
    app.timeline.room_id = None;
    app.timeline.items.clear();
    app.timeline.first_urls.clear();
    app.timeline.composer = screens::timeline::composer::State::default();
    app.timeline.member_index.clear();
    app.timeline.selected_member = None;
    app.timeline.highlighted_member = None;
    app.timeline.member_menu = None;
    app.timeline.autoscroll = None;
    send_cmd(app, ClientCommand::CloseRoom { room_id: room_id.to_string() });
    // Don't let the next launch reopen a room the user just left.
    let clear = clear_last_room(&app.profile);
    if app.timeline.inline_video.take().is_some() {
        return Task::batch([clear, close_native_player()]);
    }
    clear
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
            app.emoji_packs.clear();
            app.emoji_shortcode_index.clear();
            app.zoomed_image = None;
            app.space_explorer = None;
            app.route = crate::state::Route::Login;
            // The timeline reset above dropped any inline-video state; tear
            // down the native webview too (no-op when none is live).
            return close_native_player();
        }
        ClientEvent::DirectMessageReady { room_id } => {
            return select_room(app, room_id);
        }
        ClientEvent::RoomCreated { room_id } => {
            return select_room(app, room_id);
        }
        ClientEvent::UserSearchResults { request_id, results, limited } => {
            // Only accept the newest in-flight search's results — a slower
            // earlier response must not overwrite a faster later one.
            if app.dm_search.as_ref().and_then(|s| s.last_request_id) == Some(request_id) {
                let avatar_urls: Vec<String> =
                    results.iter().filter_map(|r| r.avatar_url.clone()).collect();
                if let Some(state) = app.dm_search.as_mut() {
                    state.results = results;
                    state.limited = limited;
                    state.pending = false;
                    state.error = None;
                }
                ensure_media_fetched(app, avatar_urls);
            }
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

            // Safe-state: reopen the room we were last in on this launch, the
            // moment the sync brings it into the list (sliding sync streams
            // rooms in gradually, so it may not be in the first update). Only
            // when nothing's been opened yet, the worker channel is up
            // (`select_room` needs it to subscribe the room), and the target
            // is a real, non-space room still in the list — a space has no
            // timeline, and a room left elsewhere simply never reappears.
            // Consuming it here means it fires exactly once.
            if let Some(target) = app.pending_restore_room.clone() {
                let restorable = app.room_list.selected_room_id.is_none()
                    && app.cmd_tx.is_some()
                    && app.room_list.rooms.iter().any(|r| r.room_id == target && !r.is_space);
                if restorable {
                    app.pending_restore_room = None;
                    tracing::info!(room_id = %target, "safe-state: reopening last room");
                    return select_room(app, target);
                }
            }
        }
        ClientEvent::TimelineUpdated { room_id, items } => {
            if app.timeline.room_id.as_deref() == Some(room_id.as_str()) {
                let urls = image_urls_in_timeline(&items, &app.media);
                let mut candidate_emojis =
                    unicode_reaction_keys(&items, &app.emoji_packs, &app.media);
                candidate_emojis.extend(unicode_body_emojis(&items, &app.media));

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

                // Grow the sticker collection with anything new in this
                // snapshot (before `items` is moved into state below).
                let stickers_changed = harvest_stickers(app, &items);
                let (preview_task, first_urls) = request_url_previews(app, &items);
                app.timeline.items = items;
                // Index-parallel to items — set only here and cleared in
                // select_room, so the two can't drift apart.
                app.timeline.first_urls = first_urls;
                // Membership set is index-based; a new snapshot invalidates
                // it (no-op when no search is active).
                screens::timeline::recompute_search_matches(
                    &mut app.timeline,
                    app.chat.show_membership_events,
                );
                let reset_task = if reset {
                    tracing::info!(
                        len = app.timeline.items.len(),
                        "timeline window reset by a sync gap — snapping to live edge"
                    );
                    app.timeline.scroll_anchor = None;
                    app.timeline.at_bottom = true;
                    iced::widget::operation::scroll_to(
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
                let sticker_task =
                    if stickers_changed { persist_sticker_collection(app) } else { Task::none() };
                return Task::batch([preview_task, reset_task, emoji_task, sticker_task]);
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
                let avatar_urls: Vec<String> = members
                    .iter()
                    .filter_map(|m| m.avatar_url.as_deref())
                    .filter(|url| !app.media.is_known(url))
                    .map(str::to_owned)
                    .collect();
                app.timeline.composer.member_candidates = members;
                ensure_media_fetched(app, avatar_urls);
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
                    &app.spellcheck,
                );
                return apply_composer_effect(app, effect);
            } else if app.timeline.composer.pending_attachment_request == Some(request_id) {
                // Attachment landed: drop its chip (the front of the staged
                // list) and, if this Enter-batch has more files waiting,
                // start the next — bare, the caption already rode out on
                // the first file. A typed draft is untouched.
                let composer = &mut app.timeline.composer;
                composer.pending_attachment_request = None;
                composer.error = None;
                if !composer.staged_attachments.is_empty() {
                    composer.staged_attachments.remove(0);
                }
                composer.sending_remaining = composer.sending_remaining.saturating_sub(1);
                if composer.staged_attachments.is_empty() {
                    composer.sending_remaining = 0;
                }
                let had_caption = composer
                    .carried
                    .take()
                    .is_some_and(|carried| !carried.body.trim().is_empty());
                let next = if composer.sending_remaining > 0 {
                    composer.staged_attachments.first().map(|staged| {
                        (staged.filename.clone(), staged.bytes.clone(), staged.mime.clone())
                    })
                } else {
                    None
                };
                if let Some((filename, bytes, mime)) = next {
                    send_attachment(app, filename, bytes, mime, None, Vec::new(), None);
                }
                if had_caption {
                    // The caption went out with the file — stop advertising
                    // "is typing" for text that's already posted (the text
                    // path does the same via SendSucceeded).
                    set_typing(app, false);
                }
            } else if app.timeline.composer.pending_sticker_request == Some(request_id) {
                // Sticker posted — the echo lands in the timeline (and gets
                // harvested); nothing here touches the text draft.
                app.timeline.composer.pending_sticker_request = None;
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
            } else if app
                .space_explorer
                .as_mut()
                .is_some_and(|explorer| explorer.handle_success(request_id))
            {
                // A join finished — the explorer flipped that row to
                // "Joined"; the sidebar picks the room up on the next
                // room-list emission.
            } else if app
                .dm_search
                .as_mut()
                .is_some_and(|search| search.handle_success(request_id))
            {
                // A user search succeeded; its results arrive separately via
                // UserSearchResults. Nothing else to do here.
            }
        }
        ClientEvent::CommandFailed { request_id, error } => {
            if app.timeline.composer.pending_request == Some(request_id) {
                let (_, effect) = screens::timeline::composer::update(
                    &mut app.timeline.composer,
                    screens::timeline::composer::Message::SendFailed(error),
                    &app.spellcheck,
                );
                return apply_composer_effect(app, effect);
            } else if app.timeline.composer.pending_attachment_request == Some(request_id) {
                let composer = &mut app.timeline.composer;
                composer.pending_attachment_request = None;
                composer.error = Some(error);
                // Stop the batch: the failed file stays staged (front chip)
                // so Enter retries it; nothing behind it is silently
                // skipped or sent out of order.
                composer.sending_remaining = 0;
                // The caption never left — put the draft back rather than
                // losing it, unless the user typed something new meanwhile.
                if let Some(carried) = composer.carried.take() {
                    if composer.body.is_empty() {
                        composer.body = carried.body;
                        composer.mentioned = carried.mentioned;
                    }
                    if composer.replying_to.is_none() {
                        composer.replying_to = carried.replying_to;
                    }
                }
            } else if app.timeline.composer.pending_sticker_request == Some(request_id) {
                app.timeline.composer.pending_sticker_request = None;
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
            } else if app
                .space_explorer
                .as_mut()
                .is_some_and(|explorer| explorer.handle_failure(request_id, &error))
            {
                // Routed into the explorer (failed page fetch → level error
                // with Retry; failed join → error line in that room's row).
            } else if app
                .dm_search
                .as_mut()
                .is_some_and(|search| search.handle_failure(request_id, &error))
            {
                // Surfaced in the DM-search overlay's error line.
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
            // A failure card from an earlier attempt shouldn't outlive the
            // success it was retried into.
            app.verification.recovery_error = None;
        }
        ClientEvent::RecoveryEnableFailed { reason } => {
            app.verification.recovery_enable_stage = None;
            app.verification.recovery_error = Some(reason);
        }
        ClientEvent::KeyBackupRestored => {
            app.verification.recovery_needs_key = false;
            app.verification.recovery_key_input.clear();
            // Typo-then-correct-key: the stale failure card must not stay
            // up next to the success state.
            app.verification.recovery_error = None;
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
            // Lightbox Download button: these bytes are destined for a file,
            // not the display caches. Intercept before the display path (the
            // request_id was never put in `pending`, so it wouldn't match
            // below anyway).
            if app.media.download_requests.remove(&request_id) {
                let suggested =
                    format!("thornychat-image.{}", crate::media_cache::image_extension(&bytes));
                return Task::future(async move {
                    if let Some(handle) =
                        rfd::AsyncFileDialog::new().set_file_name(&suggested).save_file().await
                    {
                        if let Err(error) = handle.write(&bytes).await {
                            tracing::warn!(%error, "saving downloaded image failed");
                        }
                    }
                    Message::Noop
                });
            }
            if let Some(url) = app.media.pending.remove(&request_id) {
                app.media.pending_urls.remove(&url);
                // Logged for every branch below: url + byte length + content
                // fingerprint, so a "wrong image shown" report can be
                // diagnosed from the log file alone. Cross-reference against
                // "emoji pack entry" (what shortcode/pack this url belongs
                // to) and, for gifs, the "gif decoded" line that follows.
                let len = bytes.len();
                let fingerprint = crate::media_cache::fingerprint(&bytes);
                if crate::media_cache::looks_like_svg(&bytes) {
                    tracing::info!(url, len, fingerprint, kind = "svg", "media fetched");
                    app.media.mxc_svgs.insert(url, iced::widget::svg::Handle::from_memory(bytes));
                } else if crate::media_cache::looks_like_gif(&bytes) {
                    tracing::info!(url, len, fingerprint, kind = "gif", "media fetched, decoding");
                    // Frame decode is CPU-heavy (full RGBA per frame; a chat
                    // GIF is easily 100ms+) — run it off the update thread.
                    // The url stays in pending_urls until GifDecoded lands so
                    // it isn't re-requested mid-decode.
                    app.media.pending_urls.insert(url.clone());
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                crate::animated_image::Frames::from_bytes(bytes.clone())
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
                } else if crate::media_cache::looks_like_unsupported_container(&bytes) {
                    // Fetched fine, but our raster decoder can't read AVIF/HEIC —
                    // iced's renderer would otherwise swallow the decode error
                    // and leave a permanently blank, unexplained gap. Fail it
                    // the same way a failed fetch would, so the caller falls
                    // back to the initials avatar instead.
                    tracing::warn!(url, len, fingerprint, "media is AVIF/HEIC — no decoder compiled in, falling back");
                    app.media.failed_mxc.insert(url);
                } else {
                    tracing::info!(url, len, fingerprint, kind = "raster", "media fetched");
                    app.media.images.insert(url, iced::widget::image::Handle::from_bytes(bytes));
                }
            }
        }
        ClientEvent::MediaFetchFailed { request_id, reason } => {
            // A download re-fetch that failed: just drop the tracking entry so
            // it doesn't leak; nothing to blacklist (the display copy, if any,
            // is unaffected).
            if app.media.download_requests.remove(&request_id) {
                tracing::warn!(reason, "image download re-fetch failed");
                return Task::none();
            }
            // Negative-cache the URL — without this, the very next timeline
            // or room-list update re-requests it, once per sync tick, forever.
            if let Some(url) = app.media.pending.remove(&request_id) {
                app.media.pending_urls.remove(&url);
                tracing::warn!(url, reason, "media fetch failed, blacklisting for this session");
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
        ClientEvent::DefaultNotificationModesUpdated { direct_messages, group_chats } => {
            app.default_notification_modes = (direct_messages, group_chats);
        }
        ClientEvent::CallStateUpdated(call_state) => {
            app.call.calls.insert(call_state.room_id.clone(), call_state);
        }
        ClientEvent::SpaceHierarchyFetched { request_id, space_id, children, next_batch } => {
            let avatar_urls: Vec<String> = children
                .iter()
                .filter_map(|c| c.avatar_url.as_deref())
                .filter(|url| !app.media.is_known(url))
                .map(str::to_owned)
                .collect();
            if let Some(explorer) = &mut app.space_explorer {
                explorer.apply_page(request_id, &space_id, children, next_batch);
            }
            ensure_media_fetched(app, avatar_urls);
        }
        ClientEvent::SpaceChildrenFetched { space_id, children } => {
            app.room_list.space_children.insert(space_id, children);
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

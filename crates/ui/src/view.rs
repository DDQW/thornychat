use client_core::events::SyncState;
use iced::widget::{
    button, center, column, container, horizontal_space, mouse_area, opaque, row, scrollable, stack,
    text, text_input,
};
use iced::{Element, Length};

use crate::message::Message;
use crate::screens;
use crate::state::{App, Route};

pub fn view(app: &App) -> Element<'_, Message> {
    // `corner_radius` has no slot in iced's `Theme`, so it's synced through
    // a small global once per frame here, ahead of anything that reads it
    // (`ghost_button`/`overlay_button`/`selected_ghost_button`).
    crate::theme::sync_corner_radius(app.theme.corner_radius);

    match app.route {
        Route::Login => screens::login::view(&app.login).map(Message::Login),
        Route::Main => {
            // Always a stack, even with no overlay: swapping the root
            // widget type would reset the whole tree's state (scroll
            // positions, input focus) every time the lightbox opens.
            let mut layers = stack![main_shell(app)];
            if let Some(url) = &app.zoomed_image {
                layers = layers.push(lightbox(app, url));
            }
            if app.show_settings {
                layers = layers.push(settings_overlay(app));
            }
            if let Some(explorer) = &app.space_explorer {
                layers = layers.push(
                    screens::space_explorer::view(explorer, &app.media)
                        .map(Message::SpaceExplorer),
                );
            }
            if let Some(prompt) = &app.pending_room_action {
                layers = layers.push(room_action_overlay(prompt));
            }
            layers.into()
        }
    }
}

/// Fullscreen image viewer: dimmed backdrop, the image contain-fit in the
/// middle. The viewer is sized `Shrink`, not `Fill` — its hit-box then
/// matches the rendered picture exactly (same on-screen size either way,
/// `Shrink` just stops it from also claiming the letterboxed margin around
/// a picture whose aspect ratio doesn't match the window), so a click
/// anywhere in that translucent margin — like the ✕ or Escape — dismisses,
/// while a click on the picture itself is left for the viewer's own
/// zoom/pan handling.
fn lightbox<'a>(app: &'a App, url: &'a str) -> Element<'a, Message> {
    let visual: Element<'a, Message> = if let Some(frames) = app.media.mxc_gifs.get(url) {
        iced_gif::gif(frames).width(Length::Fill).height(Length::Fill).into()
    } else if let Some(handle) = app.media.images.get(url) {
        iced::widget::image::Viewer::new(handle.clone())
            .width(Length::Shrink)
            .height(Length::Shrink)
            // 1.0 = contain-fit; wheel-down stops there instead of
            // shrinking the image into a thumbnail.
            .min_scale(1.0)
            .max_scale(16.0)
            .scale_step(0.25)
            .into()
    } else if let Some(handle) = app.media.mxc_svgs.get(url) {
        iced::widget::svg(handle.clone()).width(Length::Fill).height(Length::Fill).into()
    } else {
        text("Loading image...").size(14).into()
    };

    let backdrop = mouse_area(
        center(container(visual).padding(40)).style(|_theme: &iced::Theme| {
            iced::widget::container::Style {
                background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.82).into()),
                ..iced::widget::container::Style::default()
            }
        }),
    )
    .on_press(Message::CloseZoom)
    .interaction(iced::mouse::Interaction::Pointer);

    // Floating ✕, top-right. Its container spans the whole layer but only
    // the button consumes clicks — everywhere else falls through to the
    // backdrop's close-on-click (same stacking trick as the settings grip).
    let close = container(
        button(text(crate::theme::icon::CLOSE).font(crate::theme::ICON_FONT).size(14))
            .on_press(Message::CloseZoom)
            .style(crate::theme::overlay_button)
            .padding([6, 10]),
    )
    .width(Length::Fill)
    .align_x(iced::Right)
    .padding(12);

    opaque(stack![backdrop, close])
}

/// Settings dialog: dimmed backdrop, centered panel with the tab strip and
/// the active tab's content, resizable via a grip in its bottom-right
/// corner. Same nesting as `lightbox` — the panel's own buttons/inputs sit
/// inside the backdrop's `mouse_area`, and iced only bubbles a click
/// through to the backdrop's `on_press` if nothing inside consumed it
/// first; the grip's own `mouse_area` consumes its press the same way.
fn settings_overlay(app: &App) -> Element<'_, Message> {
    let backdrop = |_theme: &iced::Theme| iced::widget::container::Style {
        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
        ..iced::widget::container::Style::default()
    };

    let header = row![
        text("Settings").size(16).font(crate::theme::SEMIBOLD_FONT).width(Length::Fill),
        button(text(crate::theme::icon::CLOSE).font(crate::theme::ICON_FONT).size(12))
            .on_press(Message::ToggleSettings)
            .style(crate::theme::ghost_button)
            .padding([4, 8]),
    ]
    .align_y(iced::Center);

    let account = screens::settings::general::AccountInfo {
        user_id: app.own_user_id.as_deref(),
        homeserver: app.client.as_ref().map(|client| client.homeserver().to_string()),
        device_id: app.client.as_ref().and_then(|client| client.device_id()).map(|id| id.to_string()),
    };
    let body = scrollable(
        screens::settings::view(
            &app.settings,
            &app.theme,
            &app.privacy,
            &app.encryption,
            &app.spellcheck,
            account,
            app.default_notification_modes,
            &app.verification,
        )
        .map(Message::Settings),
    )
    .style(crate::theme::thin_scrollbar);

    let size = app.settings_panel_size;
    let card = container(
        column![header, body]
            .spacing(16)
            .width(Length::Fixed(size.width))
            .height(Length::Fixed(size.height)),
    )
    .padding(20)
    .style(crate::theme::panel);

    let grip = mouse_area(
        container(iced::widget::Space::new(Length::Fixed(14.0), Length::Fixed(14.0))).style(
            |theme: &iced::Theme| iced::widget::container::Style {
                // Deliberately distinct from the card's own background
                // (`crate::theme::panel`) — the grip has to read as an
                // affordance, not blend into the panel it's resizing.
                background: Some(theme.extended_palette().background.strong.color.into()),
                border: iced::border::rounded(3),
                ..iced::widget::container::Style::default()
            },
        ),
    )
    .on_press(Message::SettingsResizeStarted)
    .interaction(iced::mouse::Interaction::ResizingDiagonallyDown);
    let grip_layer = container(grip)
        .width(Length::Fixed(size.width))
        .height(Length::Fixed(size.height))
        .align_x(iced::Right)
        .align_y(iced::Bottom)
        .padding(6);

    opaque(
        mouse_area(center(stack![card, grip_layer]).style(backdrop)).on_press(Message::ToggleSettings),
    )
}

/// Leave-or-forget confirmation for a sidebar room, raised on right-click.
/// Offers both actions (with a clear warning) plus Cancel — the modal itself
/// is the confirmation gate, so nothing destructive happens on the
/// right-click alone.
fn room_action_overlay(prompt: &crate::state::RoomActionPrompt) -> Element<'_, Message> {
    let backdrop = |_theme: &iced::Theme| iced::widget::container::Style {
        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
        ..iced::widget::container::Style::default()
    };

    let kind = if prompt.is_dm { "direct message" } else { "room" };
    let title = format!("Leave {}?", prompt.room_name);
    let explanation = format!(
        "Leave removes this {kind} from your list; you can rejoin if you're \
         invited again. Forget also deletes your local copy of its history. \
         Neither can be undone from here."
    );

    let room_id = prompt.room_id.clone();
    let buttons = row![
        button(text("Cancel").size(13))
            .on_press(Message::CancelRoomAction)
            .style(crate::theme::ghost_button)
            .padding([6, 12]),
        horizontal_space(),
        button(text("Leave").size(13))
            .on_press(Message::ConfirmLeaveRoom(room_id.clone()))
            .padding([6, 12]),
        button(text("Forget").size(13))
            .on_press(Message::ConfirmForgetRoom(room_id))
            .style(button::danger)
            .padding([6, 12]),
    ]
    .spacing(8)
    .align_y(iced::Center);

    let card = container(
        column![
            text(title).size(16).font(crate::theme::SEMIBOLD_FONT),
            text(explanation).size(12).style(text::secondary),
            buttons,
        ]
        .spacing(14)
        .width(Length::Fixed(380.0)),
    )
    .padding(20)
    .style(crate::theme::panel);

    // Backdrop click cancels; the card's own buttons consume their clicks
    // first (same nesting the other overlays rely on).
    opaque(mouse_area(center(card).style(backdrop)).on_press(Message::CancelRoomAction))
}

fn main_shell(app: &App) -> Element<'_, Message> {
    let status = match &app.sync_state {
        SyncState::Connecting => "Connecting...".to_string(),
        SyncState::Syncing => "Connected".to_string(),
        SyncState::Offline => "Offline".to_string(),
        SyncState::Error(e) => format!("Sync error: {e}"),
    };

    let sidebar =
        screens::room_list::view(&app.room_list, &app.notification_modes, &app.call, &app.media)
            .map(Message::RoomList);

    let selected_room = app
        .room_list
        .selected_room_id
        .as_deref()
        .and_then(|id| app.room_list.rooms.iter().find(|r| r.room_id == id));
    let notification_mode = app
        .room_list
        .selected_room_id
        .as_deref()
        .and_then(|id| app.notification_modes.get(id))
        .copied();

    let timeline = screens::timeline::view(
        &app.timeline,
        app.own_user_id.as_deref(),
        selected_room,
        notification_mode,
        &app.call,
        &app.emoji_usage,
        &app.media,
        &app.emoji_packs,
        &app.sticker_collection,
        &app.emoji_shortcode_index,
        &app.url_previews,
        &app.tweet_previews,
        &app.steam_previews,
    )
    .map(Message::Timeline);

    let security = screens::verification::view(&app.verification, app.own_user_id.as_deref())
        .map(Message::Verification);

    let keyword_slot =
        crate::theme::slot(app.show_keyword_panel.then(|| keyword_panel(app)));

    column![
        top_bar(app, status),
        security,
        keyword_slot,
        row![sidebar, timeline].width(Length::Fill).height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn top_bar<'a>(app: &'a App, status: String) -> Element<'a, Message> {
    let mut bar = row![text(status).size(12), horizontal_space()]
        .spacing(8)
        .align_y(iced::Center);

    if let Some(toggle) = screens::verification::verify_toggle(&app.verification) {
        bar = bar.push(toggle.map(Message::Verification));
    }
    bar = bar.push(
        button(crate::theme::icon_text(crate::theme::icon::KEYWORDS, 14))
            .on_press(Message::ToggleKeywordPanel)
            .style(crate::theme::ghost_button)
            .padding([4, 8]),
    );
    bar = bar.push(
        button(crate::theme::icon_text(crate::theme::icon::SETTINGS, 14))
            .on_press(Message::ToggleSettings)
            .style(crate::theme::ghost_button)
            .padding([4, 8]),
    );

    container(bar).padding([4, 8]).style(crate::theme::panel).into()
}

/// Account-wide keyword highlights (words that trigger a
/// mentions-and-keywords notification even without an @mention), toggled
/// from the top bar.
fn keyword_panel(app: &App) -> Element<'_, Message> {
    let mut chips = row![].spacing(6).align_y(iced::Center);
    if app.keyword_highlights.is_empty() {
        chips = chips.push(text("No highlight keywords yet.").size(12).style(text::secondary));
    }
    for keyword in &app.keyword_highlights {
        chips = chips.push(
            row![
                text(keyword.clone()).size(12),
                button(text("×").size(12))
                    .on_press(Message::RemoveKeywordClicked(keyword.clone()))
                    .style(crate::theme::ghost_button)
                    .padding([0, 4]),
            ]
            .spacing(2)
            .align_y(iced::Center),
        );
    }

    let add_row = row![
        text_input("Add keyword...", &app.keyword_draft)
            .on_input(Message::KeywordDraftChanged)
            .on_submit(Message::AddKeywordClicked)
            .padding(6)
            .size(13)
            .width(Length::Fixed(220.0)),
        button(text("Add").size(12)).on_press(Message::AddKeywordClicked).padding([6, 10]),
    ]
    .spacing(6)
    .align_y(iced::Center);

    container(
        column![
            text("Keyword highlights — you'll be notified when these words are mentioned")
                .size(12),
            chips,
            add_row,
        ]
        .spacing(6),
    )
    .padding([8, 12])
    .width(Length::Fill)
    .style(crate::theme::panel)
    .into()
}

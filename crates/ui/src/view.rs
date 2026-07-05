use client_core::events::SyncState;
use iced::widget::{
    button, center, column, container, horizontal_space, mouse_area, opaque, row, stack, text,
    text_input,
};
use iced::{Element, Length};

use crate::message::Message;
use crate::screens;
use crate::state::{App, Route};

pub fn view(app: &App) -> Element<'_, Message> {
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
            if let Some(player) = &app.video_player {
                layers = layers.push(video_overlay(player));
            }
            layers.into()
        }
    }
}

/// Fullscreen image viewer: dimmed backdrop, the image contain-fit in the
/// middle, click anywhere to dismiss.
fn lightbox<'a>(app: &'a App, url: &'a str) -> Element<'a, Message> {
    let visual: Element<'a, Message> = if let Some(frames) = app.media.mxc_gifs.get(url) {
        iced_gif::gif(frames).width(Length::Fill).height(Length::Fill).into()
    } else if let Some(handle) = app.media.images.get(url) {
        iced::widget::image(handle.clone())
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    } else if let Some(handle) = app.media.mxc_svgs.get(url) {
        iced::widget::svg(handle.clone()).width(Length::Fill).height(Length::Fill).into()
    } else {
        text("Loading image...").size(14).into()
    };

    opaque(
        mouse_area(
            center(container(visual).padding(40)).style(|_theme: &iced::Theme| {
                iced::widget::container::Style {
                    background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.82).into()),
                    ..iced::widget::container::Style::default()
                }
            }),
        )
        .on_press(Message::CloseZoom)
        .interaction(iced::mouse::Interaction::Pointer),
    )
}

/// In-app video player overlay: dimmed backdrop, header row (title,
/// "Watch on {platform}", close), and the video stage. The video itself is
/// a native WebView2 child window that composites *over* the iced surface
/// at exactly `video_player::video_rect` — iced draws everything around it,
/// and the black stage only peeks through while the player is loading (or
/// hosts the error fallback if the webview couldn't start).
fn video_overlay(player: &crate::state::VideoPlayer) -> Element<'_, Message> {
    let backdrop = |_theme: &iced::Theme| iced::widget::container::Style {
        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.82).into()),
        ..iced::widget::container::Style::default()
    };

    let Some(window) = player.window else {
        // Geometry unknown until the open task reports back (a frame or
        // two) — just dim the screen.
        return opaque(
            mouse_area(center(text("Starting player...").size(14)).style(backdrop))
                .on_press(Message::CloseVideoPlayer),
        );
    };

    let rect = crate::video_player::video_rect(window);
    let platform_label = player.video.platform.label();

    let header = row![
        text(player.title.as_deref().unwrap_or(platform_label))
            .size(14)
            .font(crate::theme::SEMIBOLD_FONT)
            .color(iced::Color::WHITE)
            .width(Length::Fill),
        button(text(format!("Watch on {platform_label}")).size(12))
            .on_press(Message::OpenVideoInBrowser)
            .style(crate::theme::overlay_button)
            .padding([4, 8]),
        button(text(crate::theme::icon::CLOSE).font(crate::theme::ICON_FONT).size(12))
            .on_press(Message::CloseVideoPlayer)
            .style(crate::theme::overlay_button)
            .padding([4, 8]),
    ]
    .spacing(8)
    .align_y(iced::Center)
    .width(Length::Fixed(rect.width))
    .height(Length::Fixed(crate::video_player::HEADER_HEIGHT));

    let stage = |_theme: &iced::Theme| iced::widget::container::Style {
        background: Some(iced::Color::BLACK.into()),
        ..iced::widget::container::Style::default()
    };
    let surface: Element<'_, Message> = if let Some(reason) = &player.error {
        container(
            column![
                text("The embedded player couldn't start.")
                    .size(14)
                    .color(iced::Color::WHITE),
                text(reason.clone()).size(12).color(iced::Color::from_rgb8(0xB0, 0xB0, 0xB0)),
                button(text("Watch in browser").size(13))
                    .on_press(Message::OpenVideoInBrowser)
                    .style(crate::theme::overlay_button)
                    .padding([6, 10]),
            ]
            .spacing(10)
            .align_x(iced::Center),
        )
        .center_x(Length::Fixed(rect.width))
        .center_y(Length::Fixed(rect.height))
        .style(stage)
        .into()
    } else {
        container(iced::widget::Space::new(rect.width, rect.height)).style(stage).into()
    };

    let block = column![header, surface].spacing(crate::video_player::HEADER_GAP);

    opaque(
        mouse_area(center(block).style(backdrop))
            .on_press(Message::CloseVideoPlayer)
            .interaction(iced::mouse::Interaction::Pointer),
    )
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
        button(text("Keywords").size(12))
            .on_press(Message::ToggleKeywordPanel)
            .style(crate::theme::ghost_button)
            .padding([4, 8]),
    );
    bar = bar.push(
        button(text(if app.dark_mode { "Light" } else { "Dark" }).size(12))
            .on_press(Message::ToggleDarkMode)
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

use iced::widget::{
    button, center, column, container, horizontal_space, mouse_area, opaque, row, scrollable, stack,
    text,
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
            if let Some(zoom) = &app.zoomed_image {
                layers = layers.push(lightbox(app, zoom));
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
/// middle at rest, scroll-wheel zoom growing it from there. The picture
/// itself swallows clicks (`swallow_click`) so pressing it never closes the
/// lightbox; the backdrop's own `on_press` only ever sees a click that
/// landed on translucent margin the picture doesn't cover — which, at a
/// high enough zoom, can be none at all, and that's fine: there's simply
/// nothing left to click that isn't the picture. The ✕ and Escape are the
/// close affordances that always work regardless of zoom.
fn lightbox<'a>(app: &'a App, zoom: &'a crate::state::ZoomedImage) -> Element<'a, Message> {
    let url = zoom.url.as_str();
    let scale = zoom.scale;

    let visual: Element<'a, Message> = if let Some(frames) = app.media.mxc_gifs.get(url) {
        swallow_click(iced_gif::gif(frames).width(Length::Fill).height(Length::Fill).into())
    } else if let Some(handle) = app.media.images.get(url) {
        match (zoom.width, zoom.height) {
            // Sender-declared dimensions: the image can grow with `scale`
            // past its contain-fit size — `responsive` hands back the
            // backdrop's actual padded pixel size, already excluding its
            // own margin, so the math here doesn't need to know the window
            // size or duplicate that padding.
            (Some(w), Some(h)) if w > 0 && h > 0 => {
                let handle = handle.clone();
                let (w, h) = (w as f32, h as f32);
                iced::widget::responsive(move |available| {
                    let fit = (available.width.max(1.0) / w).min(available.height.max(1.0) / h);
                    let final_w = w * fit * scale;
                    let final_h = h * fit * scale;

                    center(swallow_click(
                        iced::widget::image(handle.clone())
                            .width(Length::Fixed(final_w))
                            .height(Length::Fixed(final_h))
                            .into(),
                    ))
                    .into()
                })
                .into()
            }
            // No declared size to scale from — same contain-fit, non-zoomable
            // picture this was before zoom existed at all.
            _ => iced::widget::image(handle.clone()).width(Length::Fill).height(Length::Fill).into(),
        }
    } else if let Some(handle) = app.media.mxc_svgs.get(url) {
        swallow_click(iced::widget::svg(handle.clone()).width(Length::Fill).height(Length::Fill).into())
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
    .on_scroll(Message::LightboxZoomed)
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

/// Consumes a press so it never bubbles up to an ancestor `mouse_area`'s
/// `on_press` (see `MouseArea::on_event`: it returns early once its content
/// reports the event as captured) — what keeps a click on the lightbox's
/// picture from closing it.
fn swallow_click(content: Element<'_, Message>) -> Element<'_, Message> {
    mouse_area(content).on_press(Message::Noop).into()
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
    let sidebar = column![
        screens::room_list::view(&app.room_list, &app.notification_modes, &app.call, &app.media)
            .map(Message::RoomList),
        own_profile_bar(app),
    ]
    .width(Length::Fixed(240.0))
    .height(Length::Fill);

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

    let security = screens::verification::view(&app.verification).map(Message::Verification);

    column![
        security,
        row![sidebar, timeline].width(Length::Fill).height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Own account row pinned under the room list — click opens Settings
/// (relocated here from a top-bar gear icon, so there's no bar taking up
/// space above the room list at all).
fn own_profile_bar(app: &App) -> Element<'_, Message> {
    let label = app.own_user_id.as_deref().unwrap_or("Account");
    let label_row = row![
        crate::media_cache::avatar(&app.media, None, label, 24),
        text(label).size(13).width(Length::Fill),
    ]
    .spacing(8)
    .align_y(iced::Center);

    container(
        button(label_row)
            .on_press(Message::ToggleSettings)
            .width(Length::Fill)
            .padding([8, 12])
            .style(crate::theme::ghost_button),
    )
    .style(crate::theme::panel)
    .into()
}

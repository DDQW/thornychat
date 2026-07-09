use iced::widget::{
    button, center, column, container, mouse_area, opaque, row, scrollable, space, stack,
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
            if let Some(search) = &app.dm_search {
                layers = layers
                    .push(screens::dm_search::view(search, &app.media).map(Message::DmSearch));
            }
            if let Some(prompt) = &app.pending_room_action {
                layers = layers.push(room_action_overlay(prompt));
            }
            // The autoscroll anchor puck sits on top of everything, pinned at
            // the point where middle-click started the glide.
            if let Some(origin) = app.timeline.autoscroll {
                layers = layers.push(autoscroll_marker(origin));
            }
            layers.into()
        }
    }
}

/// Fullscreen image viewer: dimmed backdrop, the image contain-fit in the
/// middle at rest. Raster images use the custom [`LightboxImage`] widget: the
/// whole picture scales with the mouse wheel (growing past the viewport for
/// deep zoom, not cropped inside a fixed frame) and drags to pan, while a
/// press on the empty margin around it is left unhandled so it falls through
/// to the backdrop's close. That combination is the one thing iced's own
/// `Viewer` can't do — it either crops the zoom to a fixed frame or swallows
/// the margin click. GIFs and SVGs can't feed the widget, so they render
/// `Fill` and are simply click-anywhere-to-close (they don't zoom). Beyond
/// clicking the free space, the lightbox also closes on right-click, on a
/// double-click of the picture, on the ✕ in the top-right, and on Escape; the
/// Download button beside the ✕ saves the image.
fn lightbox<'a>(app: &'a App, url: &'a str) -> Element<'a, Message> {
    let visual: Element<'a, Message> = if let Some(frames) = app.media.mxc_gifs.get(url) {
        crate::animated_image::gif(frames)
            .debug_label(url)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    } else if let Some(handle) = app.media.images.get(url) {
        crate::widgets::lightbox_image::LightboxImage::new(handle.clone())
            .on_double_click(Message::CloseZoom)
            .into()
    } else if let Some(handle) = app.media.mxc_svgs.get(url) {
        iced::widget::svg(handle.clone()).width(Length::Fill).height(Length::Fill).into()
    } else {
        text("Loading image...").size(14).into()
    };

    // No padding: the widget gets the whole layer so a zoomed-in picture fills
    // it edge-to-edge. At rest the picture is contain-fit, so the letterbox
    // band around it is free space a click closes through. Right-click closes
    // anywhere (the widget ignores right presses, so they reach here).
    let backdrop = mouse_area(center(visual).style(|_theme: &iced::Theme| {
        iced::widget::container::Style {
            background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.82).into()),
            ..iced::widget::container::Style::default()
        }
    }))
    .on_press(Message::CloseZoom)
    .on_right_press(Message::CloseZoom)
    .interaction(iced::mouse::Interaction::Pointer);

    // Download + ✕, top-right. The container spans the layer but only the two
    // buttons consume clicks — everywhere else falls through to the backdrop's
    // close below (same stacking trick as the settings grip).
    let controls = container(
        row![
            button(text(crate::theme::icon::DOWNLOAD).font(crate::theme::ICON_FONT).size(14))
                .on_press(Message::DownloadZoomedImage)
                .style(crate::theme::overlay_button)
                .padding([6, 10]),
            button(text(crate::theme::icon::CLOSE).font(crate::theme::ICON_FONT).size(14))
                .on_press(Message::CloseZoom)
                .style(crate::theme::overlay_button)
                .padding([6, 10]),
        ]
        .spacing(8),
    )
    .width(Length::Fill)
    .align_x(iced::Right)
    .padding(12);

    opaque(stack![backdrop, controls])
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
            &app.chat,
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
        container(iced::widget::Space::new().width(Length::Fixed(14.0)).height(Length::Fixed(14.0)))
            .style(
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
    let explanation = format!(
        "Leave removes this {kind} from your list; you can rejoin if you're \
         invited again. Forget also deletes your local copy of its history. \
         Neither can be undone from here."
    );

    let room_id = prompt.room_id.clone();
    // Rename: an editable name committed as the room's `m.room.name` (Enter or
    // the button). Handy for labelling a scratch/self room "Notes", "Test", …
    let rename = row![
        text_input("Room name", &prompt.rename_draft)
            .on_input(Message::RoomRenameDraftChanged)
            .on_submit(Message::ConfirmRoomRename(room_id.clone()))
            .padding(6),
        button(text("Rename").size(13))
            .on_press(Message::ConfirmRoomRename(room_id.clone()))
            .padding([6, 12]),
    ]
    .spacing(8)
    .align_y(iced::Center);

    let buttons = row![
        button(text("Cancel").size(13))
            .on_press(Message::CancelRoomAction)
            .style(crate::theme::ghost_button)
            .padding([6, 12]),
        space::horizontal(),
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
            text(prompt.room_name.clone()).size(16).font(crate::theme::SEMIBOLD_FONT),
            rename,
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

/// The little anchor puck shown at the point where middle-click autoscroll
/// began — a fixed reference the pointer scrolls relative to, the same idea as
/// a browser's autoscroll origin glyph. A ring with a center dot, drawn purely
/// from containers so no font glyph can go missing. Deliberately *not* wrapped
/// in `mouse_area`/`opaque`: it's an inert top layer, so the click that ends
/// autoscroll (and every other click) falls straight through to the shell
/// below. `origin` is in window-global coordinates, which line up with this
/// outermost stack's own origin, so a top/left inset positions it directly.
fn autoscroll_marker(origin: iced::Point) -> Element<'static, Message> {
    const RING: f32 = 24.0;
    let dot = container(iced::widget::Space::new().width(Length::Fixed(6.0)).height(Length::Fixed(6.0)))
        .style(
        |theme: &iced::Theme| iced::widget::container::Style {
            background: Some(theme.extended_palette().primary.base.color.into()),
            border: iced::border::rounded(3),
            ..iced::widget::container::Style::default()
        },
    );
    let ring = container(dot)
        .center_x(Length::Fixed(RING))
        .center_y(Length::Fixed(RING))
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            iced::widget::container::Style {
                background: Some(palette.background.base.color.into()),
                border: iced::Border {
                    color: palette.primary.base.color,
                    width: 1.5,
                    radius: (RING / 2.0).into(),
                },
                ..iced::widget::container::Style::default()
            }
        });
    container(ring)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: (origin.y - RING / 2.0).max(0.0),
            left: (origin.x - RING / 2.0).max(0.0),
            right: 0.0,
            bottom: 0.0,
        })
        .into()
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
        app.chat.show_membership_events,
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

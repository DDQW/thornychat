//! Win11-approximated `iced::Theme`. This is a palette approximation, not a
//! pixel-match — iced doesn't composite with DWM backdrop materials
//! (Mica/acrylic), so this only tunes flat colors to sit close to Windows'
//! light/dark surface and accent conventions.

use iced::border;
use iced::theme::{Custom, Palette};
use iced::widget::{button, container};
use iced::{Color, Element, Font, Theme};

/// iced's default font family (Fira Sans, falling back to whatever generic
/// sans-serif fontdb finds) has no emoji glyphs, so unicode emoji render as
/// tofu boxes. `fontdb` does index all installed system fonts though
/// (confirmed: `cosmic_text::FontSystem::new_with_fonts` calls
/// `db.load_system_fonts()` before adding iced's bundled fonts) — Windows
/// ships "Segoe UI Emoji" by default, so requesting it by name for
/// emoji-only text (reaction pills, emoji pickers) renders real glyphs
/// without needing to bundle a font ourselves.
pub const EMOJI_FONT: Font = Font::with_name("Segoe UI Emoji");

/// Heavier weight for names/titles (ships with Windows; fontdb resolves it
/// from the installed system fonts, falling back silently if absent).
pub const SEMIBOLD_FONT: Font = Font::with_name("Segoe UI Semibold");

/// Windows' system icon font (Windows 11; outline-style UI glyphs at
/// documented PUA codepoints — e.g. U+E76E Emoji2, U+E97A Reply, U+E70F
/// Edit, U+E74D Delete). The right source for monochrome action icons on a
/// Windows-first app; iced's bundled text font has no such glyphs.
pub const ICON_FONT: Font = Font::with_name("Segoe Fluent Icons");

pub mod icon {
    pub const REACT: &str = "\u{E76E}";
    pub const REPLY: &str = "\u{E97A}";
    pub const EDIT: &str = "\u{E70F}";
    pub const DELETE: &str = "\u{E74D}";
    pub const PLAY: &str = "\u{E768}";
    pub const CLOSE: &str = "\u{E8BB}";
    /// U+E717 Phone — call banner / start-call button.
    pub const CALL: &str = "\u{E717}";
}

/// Builds a theme using the given accent color (ideally read from
/// `windows::UI::ViewManagement::UISettings::GetColorValue` at startup so
/// the app matches the user's actual system accent — Phase 7 polish; a
/// static fallback accent is used until then).
pub fn windows_theme(dark: bool, accent: Color) -> Theme {
    let palette = if dark {
        Palette {
            background: Color::from_rgb8(0x20, 0x20, 0x20),
            text: Color::from_rgb8(0xF3, 0xF3, 0xF3),
            primary: accent,
            success: Color::from_rgb8(0x0F, 0x7B, 0x0F),
            danger: Color::from_rgb8(0xC4, 0x2B, 0x1C),
        }
    } else {
        Palette {
            background: Color::from_rgb8(0xF3, 0xF3, 0xF3),
            text: Color::from_rgb8(0x20, 0x20, 0x20),
            primary: accent,
            success: Color::from_rgb8(0x0F, 0x7B, 0x0F),
            danger: Color::from_rgb8(0xC4, 0x2B, 0x1C),
        }
    };

    Theme::Custom(std::sync::Arc::new(Custom::new(
        if dark { "Synapse Dark".into() } else { "Synapse Light".into() },
        palette,
    )))
}

pub fn default_accent() -> Color {
    // Windows' default accent blue; overridden by the real system accent in
    // Phase 7 via `windows::UI::ViewManagement::UISettings`.
    Color::from_rgb8(0x00, 0x78, 0xD4)
}

/// Quiet button: no chrome at rest, soft rounded highlight on hover.
/// Replaces iced's `button::primary` default (a solid accent-blue slab)
/// everywhere a control shouldn't shout — room rows, toolbars, pickers.
pub fn ghost_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let base = button::Style {
        text_color: palette.background.base.text,
        border: border::rounded(6),
        ..button::Style::default()
    };
    match status {
        button::Status::Hovered | button::Status::Pressed => button::Style {
            background: Some(palette.background.weak.color.into()),
            ..base
        },
        button::Status::Disabled => button::Style {
            text_color: palette.background.base.text.scale_alpha(0.4),
            ..base
        },
        button::Status::Active => base,
    }
}

/// Like [`ghost_button`] but always white-on-dark — for controls drawn over
/// the dimmed lightbox/player backdrop, where the light theme's normal text
/// color would vanish.
pub fn overlay_button(_theme: &Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        text_color: Color::WHITE,
        border: border::rounded(6),
        ..button::Style::default()
    };
    match status {
        button::Status::Hovered | button::Status::Pressed => button::Style {
            background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.12).into()),
            ..base
        },
        button::Status::Disabled => button::Style {
            text_color: Color::WHITE.scale_alpha(0.4),
            ..base
        },
        button::Status::Active => base,
    }
}

/// Like [`ghost_button`] but accent-tinted at rest — for the selected item
/// in a list (e.g. the open room's row).
pub fn selected_ghost_button(theme: &Theme, _status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    button::Style {
        text_color: palette.primary.weak.text,
        background: Some(palette.primary.weak.color.into()),
        border: border::rounded(6),
        ..button::Style::default()
    }
}

/// Fully-rounded accent badge (unread counts).
pub fn pill_badge(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.primary.strong.color.into()),
        text_color: Some(palette.primary.strong.text),
        border: border::rounded(999),
        ..container::Style::default()
    }
}

/// Slightly-recessed panel background (sidebar, header bar, pickers).
pub fn panel(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        ..container::Style::default()
    }
}

/// Hyperlink-style button: accent-colored text, no chrome, blends into a
/// text row.
pub fn link_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let color = match status {
        button::Status::Hovered | button::Status::Pressed => palette.primary.base.color,
        _ => palette.primary.strong.color,
    };
    button::Style { text_color: color, ..button::Style::default() }
}

/// Slim rounded scrollbar (iced's default is a bare gray square floating
/// in space — it reads as a mystery button, not a scrollbar).
pub fn thin_scrollbar(
    theme: &Theme,
    status: iced::widget::scrollable::Status,
) -> iced::widget::scrollable::Style {
    use iced::widget::scrollable::{Rail, Scroller, Status, Style};
    let palette = theme.extended_palette();
    let engaged = matches!(status, Status::Hovered { .. } | Status::Dragged { .. });
    let rail = Rail {
        background: Some(palette.background.weak.color.scale_alpha(0.5).into()),
        border: border::rounded(4),
        scroller: Scroller {
            color: if engaged {
                palette.background.strong.color
            } else {
                palette.background.strong.color.scale_alpha(0.7)
            },
            border: border::rounded(4),
        },
    };
    Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
    }
}

/// A layout slot that exists whether or not it has content. iced diffs
/// widget state by tree position, so conditionally inserting an element
/// (typing indicator, error line, banner) shifts every later sibling and
/// wipes their state — most painfully, a focused `text_input` loses focus
/// whenever the element above it blinks in or out. Keeping a constant
/// `container` node in the slot makes the change purely internal.
pub fn slot<'a, M: 'a>(content: Option<Element<'a, M>>) -> Element<'a, M> {
    match content {
        Some(content) => container(content).into(),
        None => container(iced::widget::Space::new(0, 0)).into(),
    }
}

//! Win11-approximated `iced::Theme`. This is a palette approximation, not a
//! pixel-match — iced doesn't composite with DWM backdrop materials
//! (Mica/acrylic), so this only tunes flat colors to sit close to Windows'
//! light/dark surface and accent conventions.

use std::sync::atomic::{AtomicU32, Ordering};

use iced::border;
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
    /// U+E713 Settings (gear) — opens the appearance/settings overlay.
    pub const SETTINGS: &str = "\u{E713}";
}

/// Corner radius for `ghost_button`/`overlay_button`/`selected_ghost_button`,
/// synced once per frame from `view()` via `sync_corner_radius`. iced's
/// `Palette`/`Extended` have no "radius" slot to piggyback on the way colors
/// do, and these functions are referenced as bare function pointers
/// (`.style(theme::ghost_button)`) at many call sites across the app — a
/// signature change would mean touching every one of those for a single
/// float, so this is synced through a small global instead.
static CORNER_RADIUS_BITS: AtomicU32 = AtomicU32::new(6.0f32.to_bits());

pub fn sync_corner_radius(radius: f32) {
    CORNER_RADIUS_BITS.store(radius.to_bits(), Ordering::Relaxed);
}

fn corner_radius() -> f32 {
    f32::from_bits(CORNER_RADIUS_BITS.load(Ordering::Relaxed))
}

/// Quiet button: no chrome at rest, soft rounded highlight on hover.
/// Replaces iced's `button::primary` default (a solid accent-blue slab)
/// everywhere a control shouldn't shout — room rows, toolbars, pickers.
pub fn ghost_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let base = button::Style {
        text_color: palette.background.base.text,
        border: border::rounded(corner_radius()),
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
        border: border::rounded(corner_radius()),
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
        border: border::rounded(corner_radius()),
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

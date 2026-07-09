//! Theme editor: the 9 color roles (hex input + live swatch), font family,
//! UI scale, and corner radius, plus preset/import/export/reset. Every
//! committed change mutates the live `ThemeConfig` directly (so the rest of
//! the app repaints with it immediately) and autosaves to disk.

use std::collections::HashMap;

use iced::widget::{
    button, column, container, row, scrollable, slider, space, text, text_input,
};
use iced::{Element, Length, Task};

use crate::theme_config::{ThemeColor, ThemeConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorRole {
    Background,
    Surface,
    SurfaceStrong,
    Text,
    MutedText,
    Accent,
    AccentText,
    Success,
    Danger,
}

impl ColorRole {
    pub const ALL: [ColorRole; 9] = [
        ColorRole::Background,
        ColorRole::Surface,
        ColorRole::SurfaceStrong,
        ColorRole::Text,
        ColorRole::MutedText,
        ColorRole::Accent,
        ColorRole::AccentText,
        ColorRole::Success,
        ColorRole::Danger,
    ];

    fn label(self) -> &'static str {
        match self {
            ColorRole::Background => "Background",
            ColorRole::Surface => "Surface (panels)",
            ColorRole::SurfaceStrong => "Surface (hover/selected)",
            ColorRole::Text => "Text",
            ColorRole::MutedText => "Muted text",
            ColorRole::Accent => "Accent",
            ColorRole::AccentText => "Text on accent",
            ColorRole::Success => "Success",
            ColorRole::Danger => "Danger",
        }
    }

    fn get(self, theme: &ThemeConfig) -> ThemeColor {
        match self {
            ColorRole::Background => theme.background,
            ColorRole::Surface => theme.surface,
            ColorRole::SurfaceStrong => theme.surface_strong,
            ColorRole::Text => theme.text,
            ColorRole::MutedText => theme.muted_text,
            ColorRole::Accent => theme.accent,
            ColorRole::AccentText => theme.accent_text,
            ColorRole::Success => theme.success,
            ColorRole::Danger => theme.danger,
        }
    }

    fn set(self, theme: &mut ThemeConfig, color: ThemeColor) {
        match self {
            ColorRole::Background => theme.background = color,
            ColorRole::Surface => theme.surface = color,
            ColorRole::SurfaceStrong => theme.surface_strong = color,
            ColorRole::Text => theme.text = color,
            ColorRole::MutedText => theme.muted_text = color,
            ColorRole::Accent => theme.accent = color,
            ColorRole::AccentText => theme.accent_text = color,
            ColorRole::Success => theme.success = color,
            ColorRole::Danger => theme.danger = color,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    ThornyChatDark,
    ThornyChatLight,
}

impl Preset {
    pub const ALL: [Preset; 2] = [Preset::ThornyChatDark, Preset::ThornyChatLight];

    fn theme(self) -> ThemeConfig {
        match self {
            Preset::ThornyChatDark => ThemeConfig::thornychat_dark(),
            Preset::ThornyChatLight => ThemeConfig::thornychat_light(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Preset::ThornyChatDark => "ThornyChat Dark",
            Preset::ThornyChatLight => "ThornyChat Light",
        }
    }
}

#[derive(Debug, Clone)]
pub struct State {
    /// Raw text currently in each hex field, kept separate from the
    /// committed color so a mid-edit invalid string (e.g. "#2") isn't
    /// clobbered every keystroke.
    hex_drafts: HashMap<ColorRole, String>,
    font_family_draft: String,
    import_error: Option<String>,
}

impl State {
    pub fn synced_from(theme: &ThemeConfig) -> Self {
        Self {
            hex_drafts: ColorRole::ALL.into_iter().map(|role| (role, role.get(theme).to_hex())).collect(),
            font_family_draft: theme.font_family.clone().unwrap_or_default(),
            import_error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    HexChanged(ColorRole, String),
    PresetSelected(Preset),
    FontFamilyChanged(String),
    UiScaleChanged(f32),
    CornerRadiusChanged(f32),
    ImportClicked,
    Imported(Result<ThemeConfig, String>),
    ExportClicked,
    /// Fire-and-forget task (autosave, or a resolved/cancelled file dialog)
    /// completed; nothing to do.
    Saved,
}

pub fn update(state: &mut State, theme: &mut ThemeConfig, message: Message) -> Task<Message> {
    match message {
        Message::HexChanged(role, text) => {
            let parsed = ThemeColor::parse_hex(&text);
            state.hex_drafts.insert(role, text);
            match parsed {
                Some(color) => {
                    role.set(theme, color);
                    save_task(theme)
                }
                None => Task::none(),
            }
        }
        Message::PresetSelected(preset) => {
            *theme = preset.theme();
            *state = State::synced_from(theme);
            save_task(theme)
        }
        Message::FontFamilyChanged(text) => {
            let trimmed = text.trim();
            theme.font_family = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };
            state.font_family_draft = text;
            save_task(theme)
        }
        Message::UiScaleChanged(value) => {
            theme.ui_scale = value;
            save_task(theme)
        }
        Message::CornerRadiusChanged(value) => {
            theme.corner_radius = value;
            save_task(theme)
        }
        Message::ImportClicked => {
            state.import_error = None;
            Task::future(async {
                let Some(handle) =
                    rfd::AsyncFileDialog::new().add_filter("ThornyChat Theme", &["json"]).pick_file().await
                else {
                    return Message::Saved;
                };
                let bytes = handle.read().await;
                match serde_json::from_slice::<ThemeConfig>(&bytes) {
                    Ok(imported) => Message::Imported(Ok(imported)),
                    Err(err) => Message::Imported(Err(err.to_string())),
                }
            })
        }
        Message::Imported(Ok(imported)) => {
            // Clamp ui_scale/corner_radius from an arbitrary picked file —
            // an out-of-range or NaN scale would otherwise brick the window.
            *theme = imported.sanitized();
            *state = State::synced_from(theme);
            save_task(theme)
        }
        Message::Imported(Err(reason)) => {
            state.import_error = Some(format!("Couldn't import theme: {reason}"));
            Task::none()
        }
        Message::ExportClicked => {
            let Some(contents) = theme.to_json_pretty() else { return Task::none() };
            let suggested_name = format!("{}.json", theme.name.to_lowercase().replace(' ', "-"));
            Task::future(async move {
                let handle = rfd::AsyncFileDialog::new()
                    .set_file_name(&suggested_name)
                    .add_filter("ThornyChat Theme", &["json"])
                    .save_file()
                    .await;
                if let Some(handle) = handle {
                    let _ = handle.write(contents.as_bytes()).await;
                }
                Message::Saved
            })
        }
        Message::Saved => Task::none(),
    }
}

fn save_task(theme: &ThemeConfig) -> Task<Message> {
    let (Some(path), Some(contents)) = (ThemeConfig::theme_path(), theme.to_json_pretty()) else {
        return Task::none();
    };
    Task::future(async move {
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(path, contents).await;
        Message::Saved
    })
}

pub fn view<'a>(state: &'a State, theme: &'a ThemeConfig) -> Element<'a, Message> {
    let mut presets = row![].spacing(8);
    for preset in Preset::ALL {
        let active = preset.label() == theme.name;
        let style =
            if active { crate::theme::selected_ghost_button } else { crate::theme::ghost_button };
        presets = presets.push(
            button(text(preset.label()).size(13))
                .on_press(Message::PresetSelected(preset))
                .style(style)
                .padding([6, 10]),
        );
    }

    // Fill width with a right-hand padding gutter: each row right-aligns its
    // swatch to the padded edge, so the overlay scrollbar rides in the gutter
    // instead of sitting on top of the color-preview boxes.
    let mut color_rows = column![].spacing(6).width(Length::Fill).padding(
        iced::Padding { top: 0.0, right: 14.0, bottom: 0.0, left: 0.0 },
    );
    for role in ColorRole::ALL {
        color_rows = color_rows.push(color_row(role, state, theme));
    }
    let colors = scrollable(color_rows)
        .style(crate::theme::thin_scrollbar)
        .height(Length::Fixed(260.0));

    let font_row = row![
        text("Font family (applies after restart)").size(12).width(Length::Fixed(220.0)),
        text_input("Default", &state.font_family_draft)
            .on_input(Message::FontFamilyChanged)
            .size(13)
            .padding(6)
            .width(Length::Fixed(200.0)),
    ]
    .spacing(8)
    .align_y(iced::Center);

    let scale_row = row![
        text(format!("UI scale: {:.0}%", theme.ui_scale * 100.0)).size(12).width(Length::Fixed(220.0)),
        slider(0.8..=1.5, theme.ui_scale, Message::UiScaleChanged).step(0.05).width(Length::Fixed(200.0)),
    ]
    .spacing(8)
    .align_y(iced::Center);

    let radius_row = row![
        text(format!("Corner radius: {:.0}px", theme.corner_radius)).size(12).width(Length::Fixed(220.0)),
        slider(0.0..=16.0, theme.corner_radius, Message::CornerRadiusChanged)
            .step(1.0)
            .width(Length::Fixed(200.0)),
    ]
    .spacing(8)
    .align_y(iced::Center);

    let actions = row![
        button(text("Import Theme...").size(12))
            .on_press(Message::ImportClicked)
            .style(crate::theme::ghost_button)
            .padding([6, 10]),
        button(text("Export Theme...").size(12))
            .on_press(Message::ExportClicked)
            .style(crate::theme::ghost_button)
            .padding([6, 10]),
        button(text("Reset to Default").size(12))
            .on_press(Message::PresetSelected(Preset::ThornyChatDark))
            .style(crate::theme::ghost_button)
            .padding([6, 10]),
    ]
    .spacing(8);

    let error_line = crate::theme::slot(state.import_error.as_ref().map(|err| {
        text(err.clone()).size(12).color(theme.danger.color()).into()
    }));

    column![presets, colors, font_row, scale_row, radius_row, actions, error_line]
        .spacing(14)
        .into()
}

fn color_row<'a>(role: ColorRole, state: &'a State, theme: &'a ThemeConfig) -> Element<'a, Message> {
    let draft = state.hex_drafts.get(&role).map(String::as_str).unwrap_or_default();
    let parsed = ThemeColor::parse_hex(draft);
    let is_valid = parsed.is_some();
    let swatch_color = parsed.unwrap_or_else(|| role.get(theme)).color();

    let swatch = container(
        iced::widget::Space::new().width(Length::Fixed(20.0)).height(Length::Fixed(20.0)),
    )
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(swatch_color.into()),
            border: iced::border::rounded(4),
            ..container::Style::default()
        });

    row![
        text(role.label()).size(12).width(Length::Fixed(170.0)),
        text_input("#RRGGBB", draft)
            .on_input(move |value| Message::HexChanged(role, value))
            .size(13)
            .padding(6)
            .width(Length::Fixed(120.0))
            .style(move |theme: &iced::Theme, status: text_input::Status| {
                let mut style = text_input::default(theme, status);
                if !is_valid {
                    style.border.color = iced::Color::from_rgb8(0xC4, 0x2B, 0x1C);
                    style.border.width = 2.0;
                }
                style
            }),
        space::horizontal(),
        swatch,
    ]
    .spacing(8)
    .align_y(iced::Center)
    .width(Length::Fill)
    .into()
}

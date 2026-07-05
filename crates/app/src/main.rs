//! Binary entrypoint: bootstraps an explicit multi-thread tokio runtime
//! (entered before iced starts, so iced's `Task`s and our own
//! `tokio::spawn` calls share one runtime), then hands off to iced's
//! functional `application` builder.
//!
//! NOTE: `iced::application(...).run_with(...)` targets the 0.13/0.14-era
//! builder API. If this doesn't match the installed iced version, this is
//! the first place to reconcile against that version's docs/examples.

mod logging;

fn main() -> iced::Result {
    let profile = std::env::args().nth(1).unwrap_or_else(|| "default".to_string());

    let _log_guard = logging::init(&profile);
    tracing::info!(%profile, "starting Synapse");

    let runtime = tokio::runtime::Runtime::new().expect("failed to build tokio runtime");
    let _runtime_guard = runtime.enter();

    let boot_profile = profile.clone();

    // Loaded synchronously, before the window opens — the only exception to
    // this app's usual "defer I/O to async tasks" rule, because the font
    // family has to reach `.default_font()` below, which iced only accepts
    // as a static builder-time setting (not a reactive per-frame closure
    // like `.theme()`/`.scale_factor()`).
    let theme = ui::theme_config::ThemeConfig::load_or_default();
    let default_font = match &theme.font_family {
        Some(name) => iced::Font::with_name(Box::leak(name.clone().into_boxed_str())),
        None => iced::Font::DEFAULT,
    };

    iced::application("Synapse", ui::update, ui::view)
        .subscription(ui::subscription)
        .theme(|state: &ui::App| state.theme.to_iced_theme())
        .scale_factor(|state: &ui::App| state.theme.ui_scale as f64)
        .default_font(default_font)
        .run_with(move || ui::boot(boot_profile.clone(), theme))
}

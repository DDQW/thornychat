// Detach from the console on Windows GUI (release) builds so double-clicking
// the exe doesn't spawn a terminal window alongside it. Debug builds keep the
// console so `cargo run` from a terminal still shows live stderr logs; release
// logs always go to the rotating `thornychat.log` file regardless.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `--minimized` (used by autostart, see `ui::platform::autostart`) is a
    // flag, not the positional profile name, so it has to be filtered out
    // before picking the first remaining arg as the profile.
    let minimized = args.iter().any(|arg| arg == "--minimized");
    let profile =
        args.iter().find(|arg| !arg.starts_with("--")).cloned().unwrap_or_else(|| "default".to_string());

    let _log_guard = logging::init(&profile);
    tracing::info!(%profile, minimized, "starting ThornyChat");

    // An autostart Run value written before the rename to ThornyChat points
    // at the old synapse.exe; re-register it under the new name if present.
    ui::platform::autostart::migrate_legacy_value();

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

    iced::application("ThornyChat", ui::update, ui::view)
        .subscription(ui::subscription)
        // Clone the pre-built theme (an Arc bump) rather than regenerating
        // the extended palette every update cycle.
        .theme(|state: &ui::App| state.built_theme.clone())
        // Belt-and-suspenders clamp: ThemeConfig::sanitized already bounds
        // ui_scale on load/import, but never feed a non-finite factor to
        // iced (it would divide the viewport into a degenerate size).
        .scale_factor(|state: &ui::App| {
            let scale = state.theme.ui_scale;
            if scale.is_finite() { scale.clamp(0.8, 1.5) as f64 } else { 1.0 }
        })
        .default_font(default_font)
        .run_with(move || ui::boot(boot_profile.clone(), theme, minimized))
}

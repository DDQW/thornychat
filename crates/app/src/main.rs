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

    iced::application("Synapse", ui::update, ui::view)
        .subscription(ui::subscription)
        .theme(|state: &ui::App| ui::theme::windows_theme(state.dark_mode, ui::theme::default_accent()))
        .run_with(move || ui::boot(boot_profile.clone()))
}

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

    // Remembered window geometry: synchronous for the same reason as the
    // theme — it feeds `window::Settings` below, a builder-time setting.
    let window = ui::window_config::WindowConfig::load_or_default();

    // Runtime window/taskbar icon, decoded from the embedded PNG. `.ok()`:
    // a corrupt asset falls back to iced's default icon rather than
    // aborting launch. (The exe icon Explorer shows is separate — embedded
    // from app.rc by build.rs.)
    let window_icon =
        iced::window::icon::from_file_data(include_bytes!("../../../assets/icon-256.png"), None).ok();

    // 0.14 moved the boot closure to `application`'s first argument (was the
    // trailing `.run_with(...)`) and the window title to `.title(...)`. The
    // boot fn is `Fn`, not `FnOnce`, so it clones the theme per call rather
    // than moving it.
    iced::application(
        move || ui::boot(profile.clone(), theme.clone(), minimized),
        ui::update,
        ui::view,
    )
    .title("ThornyChat")
    .subscription(ui::subscription)
        .window(iced::window::Settings {
            icon: window_icon,
            size: window.size(),
            position: window.position(),
            maximized: window.maximized,
            ..Default::default()
        })
        // Clone the pre-built theme (an Arc bump) rather than regenerating
        // the extended palette every update cycle.
        .theme(|state: &ui::App| state.built_theme.clone())
        // Belt-and-suspenders clamp: ThemeConfig::sanitized already bounds
        // ui_scale on load/import, but never feed a non-finite factor to
        // iced (it would divide the viewport into a degenerate size).
        .scale_factor(|state: &ui::App| {
            let scale = state.theme.ui_scale;
            if scale.is_finite() { scale.clamp(0.8, 1.5) } else { 1.0 }
        })
        .default_font(default_font)
        // cosmic-text's glyph-fallback walk (default font -> per-script
        // Windows-named fallback, e.g. Han -> "Microsoft YaHei UI" -> every
        // other installed font as a last resort) only ever searches fonts
        // fontdb actually found on this machine. Trimmed Windows images
        // (Server Core, some IoT/LTSC/N SKUs) ship without the CJK fonts
        // that fallback list expects, so untranslated room/message/link-
        // preview text — anything we don't control the script of — renders
        // as tofu boxes there. Bundling one CJK-complete font sidesteps that
        // dependency entirely: fontdb indexes it alongside the system fonts,
        // so it's in the pool cosmic-text's last-resort scan can land on
        // (see assets/fonts/NotoSansCJK-LICENSE.txt, SIL OFL 1.1). Regular
        // weight only — this is a fallback net, not a UI font, so it's
        // never requested by name/weight, only reached when nothing else on
        // the system has the glyph.
        //
        // CAVEAT that bit us once already: the walk above only runs for text
        // shaped with `Shaping::Advanced`. iced's default `Shaping::Basic`
        // does NO fallback at all — with it, this bundled font (and every
        // installed CJK font) is unreachable and remote text still tofus.
        // That's why server-authored strings render through
        // `ui::theme::remote_text` instead of plain `text`.
        .font(include_bytes!("../../../assets/fonts/NotoSansCJKsc-Regular.otf").as_slice())
        .run()
}

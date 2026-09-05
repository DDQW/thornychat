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

mod log_limit;
mod logging;
mod render_watchdog;

/// How long the process may take to tear down after the event loop returns
/// before it is killed outright.
///
/// Observed 2026-09-04: an instance closed normally, then sat at one thread
/// with 136 MB and 723 handles for over an hour, ignoring `taskkill /F`. It
/// only went away when something finally released it. For an app that is meant
/// to run for days and to restart *itself* after a render hang, leaving undead
/// husks behind is not acceptable — a second copy cannot start cleanly and the
/// memory never comes back.
///
/// Five seconds is far more than an honest shutdown needs (the log appender's
/// own flush grace is 400 ms) and short enough that a wedged exit is over
/// before anyone reaches for Task Manager.
const SHUTDOWN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// Arms a thread that hard-exits the process if teardown stalls past
/// [`SHUTDOWN_DEADLINE`].
///
/// `std::process::exit` skips remaining destructors, which is the point: by
/// the time this fires, some `Drop` is not coming back. The thread is detached
/// and does nothing at all on a healthy exit, because the process is already
/// gone before the sleep elapses.
///
/// Honest about its limits: this rescues a teardown blocked in *user* space —
/// a driver `Drop`, a socket flush, a join that never completes. A thread
/// wedged inside a kernel call cannot be killed from user space at all, which
/// is exactly why `taskkill /F` failed on the instance above. That case needs
/// the render watchdog to act earlier, before the surface teardown is reached.
fn arm_shutdown_deadline() {
    let spawned = std::thread::Builder::new()
        .name("shutdown-deadline".into())
        .spawn(|| {
            std::thread::sleep(SHUTDOWN_DEADLINE);
            // Deliberately not `tracing`: the log guard is being dropped
            // around now, and a wedged writer is one of the things this
            // exists to escape.
            std::process::exit(0);
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not arm the shutdown deadline");
    }
}

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Toast-identity commands (`cargo xtask install-dev` and friends) and
    // shell-driven toast activations both run to completion without ever
    // opening a window.
    if let Some(code) = handle_identity_command(&args) {
        std::process::exit(code);
    }

    // Claims the registered AUMID for this process, so the window groups
    // under the same taskbar button as the shortcut the installer wrote.
    // Before any window exists, per `SetCurrentProcessExplicitAppUserModelID`.
    ui::platform::app_identity::set_process_aumid();

    // `--minimized` (used by autostart, see `ui::platform::autostart`) is a
    // flag, not the positional profile name, so it has to be filtered out
    // before picking the first remaining arg as the profile.
    let minimized = args.iter().any(|arg| arg == "--minimized");
    // Set before any window exists: the render watchdog consults it to decide
    // whether restarting again would just start a respawn loop.
    ui::platform::relaunch::set_restarted_after_hang(
        args.iter().any(|arg| arg == ui::platform::relaunch::RESTARTED_FLAG),
    );
    let profile =
        args.iter().find(|arg| !arg.starts_with("--")).cloned().unwrap_or_else(|| "default".to_string());

    let _log_guard = logging::init(&profile);
    tracing::info!(%profile, minimized, "starting ThornyChat");

    // An autostart Run value written before the rename to ThornyChat points
    // at the old synapse.exe; re-register it under the new name if present.
    ui::platform::autostart::migrate_legacy_value();

    // Remembered window geometry is loaded again by `ui::boot` (it opens the
    // window now); this copy exists for `apply_gpu_preference`, which has to
    // run here — it writes a process-wide env var, which is only sound while
    // this is still the only thread, and the runtime below is what ends that.
    let window = ui::window_config::WindowConfig::load_or_default();
    window.apply_gpu_preference();

    // Default worker count (one per core). A four-worker build was measured
    // against this one — same profile, equal downtime, 9x180s samples each —
    // and changed nothing: 159-167 threads either way, idle CPU inside the
    // noise band. The idle work is spread thin across many threads rather than
    // concentrated, so constraining the pool has no upside to trade against
    // the throughput it could cost during a media burst.
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

    // Runtime window/taskbar icon, decoded from the embedded PNG. `.ok()`:
    // a corrupt asset falls back to iced's default icon rather than
    // aborting launch. (The exe icon Explorer shows is separate — embedded
    // from app.rc by build.rs.)
    //
    // Decoded here rather than through `iced::window::icon::from_file_data`,
    // which is gated behind iced's `image` feature — the one that would force
    // image's full default codec set back into the build (see the workspace
    // manifest). `from_rgba` lives in iced_core and carries no such gate.
    let window_icon = image::load_from_memory_with_format(
        include_bytes!("../../../assets/icon-256.png"),
        image::ImageFormat::Png,
    )
    .ok()
    .and_then(|decoded| {
        let pixels = decoded.into_rgba8();
        let (width, height) = pixels.dimensions();
        iced::window::icon::from_rgba(pixels.into_raw(), width, height).ok()
    });

    // 0.14 moved the boot closure to `application`'s first argument (was the
    // trailing `.run_with(...)`) and the window title to `.title(...)`. The
    // boot fn is `Fn`, not `FnOnce`, so it clones the theme per call rather
    // than moving it.
    let result = iced::daemon(
        move || ui::boot(profile.clone(), theme.clone(), minimized, window_icon.clone()),
        ui::update,
        view,
    )
    .title(title)
    .subscription(ui::subscription)
        // A daemon rather than an `application` for one reason: an
        // `application` exits when its last window closes, and this app
        // deliberately spends the machine's standby with *no* window — that is
        // what makes `iced_winit` drop the compositor, and with it the wgpu
        // device that would otherwise come back dead (see
        // `ui::platform::power` and `docs/iced-surface-error-other-hang.md`).
        // The window itself is opened by `ui::boot`, since a daemon starts
        // with none, and its geometry/icon settings moved there with it.
        //
        // The other half of that trade: nothing exits this process on its own
        // any more. `ui::update` answers the user's close request with
        // `iced::exit()`, and a window that goes away without being asked is
        // reopened rather than left headless.
        //
        // Clone the pre-built theme (an Arc bump) rather than regenerating
        // the extended palette every update cycle.
        .theme(|state: &ui::App, _window| state.built_theme.clone())
        // Belt-and-suspenders clamp: ThemeConfig::sanitized already bounds
        // ui_scale on load/import, but never feed a non-finite factor to
        // iced (it would divide the viewport into a degenerate size).
        .scale_factor(|state: &ui::App, _window| {
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
        .run();

    // Past this point the window is gone and nothing is left worth waiting
    // for: what remains is the tokio runtime join, the wgpu device teardown,
    // and the log flush. Any of those hanging strands a husk of a process, so
    // put a ceiling on all of them at once.
    arm_shutdown_deadline();
    result
}

/// The daemon's view, which is the app's one window rendered whatever id it
/// happens to have.
///
/// A named function rather than a closure on purpose: `ViewFn` is implemented
/// for every lifetime, and a closure written inline infers a single one, which
/// the compiler then refuses ("implementation of `ViewFn` is not general
/// enough").
fn view(app: &ui::App, _window: iced::window::Id) -> iced::Element<'_, ui::Message> {
    ui::view(app)
}

/// Same for every window: this app only ever has one, and it is the app.
fn title(_app: &ui::App, _window: iced::window::Id) -> String {
    "ThornyChat".to_string()
}

/// Handles the one-shot toast-identity commands, returning the exit code to
/// leave with — or `None` when this is an ordinary launch and `main` should
/// carry on. See `ui::platform::app_identity` for what registration writes.
///
/// Output goes through `writeln!` rather than `println!` on purpose: release
/// builds are GUI-subsystem, so a double-clicked exe has no stdout at all and
/// `println!` would panic on the failed write. Run from a console (directly or
/// via `cargo xtask install-dev`, which inherits its handles) the text lands in
/// the terminal as normal.
fn handle_identity_command(args: &[String]) -> Option<i32> {
    use std::io::Write;

    use ui::platform::{app_identity, notifications};

    // The shell launches the registered LocalServer32 with this when a toast
    // is clicked. There's no COM activation callback yet, so acknowledge it by
    // doing nothing — quietly exiting beats opening a second window, which is
    // what falling through to a normal launch would do.
    if args.iter().any(|arg| arg.eq_ignore_ascii_case("-ToastActivated")) {
        return Some(0);
    }

    let command = args.iter().find(|arg| {
        matches!(arg.as_str(), "--install-dev" | "--uninstall-dev" | "--toast-test")
    })?;

    let mut out = std::io::stdout();
    match command.as_str() {
        "--install-dev" => match app_identity::install() {
            Ok(path) => {
                let _ = writeln!(out, "Registered {} for Windows notifications.", app_identity::AUMID);
                let _ = writeln!(out, "  shortcut: {}", path.display());
                let _ = writeln!(out, "  exe:      {}", std::env::current_exe().unwrap_or_default().display());
                Some(0)
            }
            Err(error) => {
                let _ = writeln!(std::io::stderr(), "error: registration failed: {error}");
                Some(1)
            }
        },
        "--uninstall-dev" => match app_identity::uninstall() {
            Ok(()) => {
                let _ = writeln!(out, "Removed the {} notification registration.", app_identity::AUMID);
                Some(0)
            }
            Err(error) => {
                let _ = writeln!(std::io::stderr(), "error: removal failed: {error}");
                Some(1)
            }
        },
        _ => {
            if !app_identity::is_installed() {
                let _ = writeln!(
                    std::io::stderr(),
                    "error: not registered — run `cargo xtask install-dev` first."
                );
                return Some(1);
            }
            match notifications::show("ThornyChat", "Toast notifications are working.") {
                Ok(()) => {
                    // Show() hands off asynchronously; give the shell a moment
                    // to pick the toast up before the process (and its COM
                    // apartment) goes away underneath it.
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    let _ = writeln!(out, "Test toast sent as {}.", app_identity::AUMID);
                    let _ = writeln!(
                        out,
                        "Nothing on screen? Check Settings > System > Notifications, and that \
                         focus assist / do not disturb is off."
                    );
                    Some(0)
                }
                Err(error) => {
                    let _ = writeln!(std::io::stderr(), "error: toast failed: {error}");
                    Some(1)
                }
            }
        }
    }
}

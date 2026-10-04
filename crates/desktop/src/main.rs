// Detach from the console on Windows GUI (release) builds so double-clicking
// the exe doesn't spawn a terminal window alongside it. Debug builds keep the
// console so `cargo tauri dev` still shows live stderr logs; release logs
// always go to the rotating `thornychat.log` file regardless.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! ThornyChat: a Tauri shell around `client-core`. The Rust side owns the
//! Matrix session, the media cache and the Windows integration; the page
//! (`frontend/`) is Svelte and talks to it through the bridge in [`bridge`].

mod activity;
mod autostart;
mod bridge;
mod commands;
mod config;
mod connectors;
mod diagnostics;
mod embeds;
mod log_limit;
mod logging;
mod media;
mod spellcheck;
mod toast;
mod tray;
mod upscale;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

use bridge::Bridge;
use commands::SettingsStore;

/// How long the process may take to tear down after the event loop returns
/// before it is killed outright.
///
/// Observed 2026-09-04 (on the iced build): an instance closed normally, then
/// sat at one thread with 136 MB and 723 handles for over an hour, ignoring
/// `taskkill /F`. For an app that is meant to run for days, leaving undead
/// husks behind is not acceptable — a second copy cannot start cleanly and the
/// memory never comes back. Five seconds is far more than an honest shutdown
/// needs and short enough that a wedged exit is over before anyone reaches for
/// Task Manager.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);

/// How long exit waits for the sync worker to wind down (it leaves any call
/// this device is in) before giving up on it.
const WORKER_GRACE: Duration = Duration::from_secs(2);

/// Arms a thread that hard-exits the process if teardown stalls past
/// [`SHUTDOWN_DEADLINE`]. `std::process::exit` skips remaining destructors,
/// which is the point: by the time this fires, some `Drop` is not coming back.
fn arm_shutdown_deadline() {
    let spawned = std::thread::Builder::new().name("shutdown-deadline".into()).spawn(|| {
        std::thread::sleep(SHUTDOWN_DEADLINE);
        // Deliberately not `tracing`: the log guard is being dropped around
        // now, and a wedged writer is one of the things this exists to escape.
        std::process::exit(0);
    });
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not arm the shutdown deadline");
    }
}

/// The page is served from the app origin (or the Vite dev server in
/// development). Anything else — a link that slipped past the click handler —
/// must not replace the IPC-enabled page.
fn is_app_origin(url: &tauri::Url) -> bool {
    match url.scheme() {
        "tauri" | "ipc" => true,
        "http" | "https" => {
            url.host_str() == Some("tauri.localhost")
                || (cfg!(debug_assertions) && url.host_str() == Some("localhost") && url.port() == Some(1420))
        }
        _ => false,
    }
}

/// Freezes `Object.prototype` against prototype pollution — in the app's own
/// document only. Tauri's `freezePrototype` setting is off because on Windows
/// wry injects every initialization script into every frame, cross-origin
/// iframes included, and YouTube's embedded player throws on a frozen
/// prototype (it assigns `toString` on its own objects) and stays black.
const FREEZE_PROTOTYPE_SCRIPT: &str = "if (window === window.top) Object.freeze(Object.prototype);";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `--minimized` (used by autostart) is a flag, not the positional profile
    // name, so it has to be filtered out before picking the first remaining
    // arg as the profile.
    let minimized = args.iter().any(|arg| arg == "--minimized");
    let profile = args.iter().find(|arg| !arg.starts_with("--")).cloned().unwrap_or_else(|| "default".to_string());

    let _log_guard = logging::init(&profile);
    tracing::info!(%profile, minimized, "starting ThornyChat");

    // An autostart Run value written before the rename to ThornyChat points
    // at the old synapse.exe; re-register it under the new name if present.
    autostart::migrate_legacy_value();

    let settings = SettingsStore(Arc::new(Mutex::new(config::Settings::load())));
    let initial_scale = settings.0.lock().expect("settings lock poisoned").theme.ui_scale;
    let shutting_down = Arc::new(AtomicBool::new(false));

    let mut builder = tauri::Builder::default();
    // One instance per profile name: a second launch of the default profile
    // just raises the first. Other profiles are separate accounts, meant to
    // run side by side.
    if profile == "default" {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            commands::show_main_window(app);
        }));
    }

    let app = builder
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .register_asynchronous_uri_scheme_protocol(media::SCHEME, media::handler::<tauri::Wry>)
        .manage(settings.clone())
        .setup(move |app| {
            let handle = app.handle().clone();
            app.manage(Bridge::new(profile.clone(), handle.clone()));

            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("ThornyChat")
                .inner_size(1100.0, 760.0)
                .min_inner_size(640.0, 480.0)
                // The page handles file drops itself (attach by dropping onto
                // the composer); Tauri's own drag-drop handler would swallow
                // the HTML5 events on Windows.
                .disable_drag_drop_handler()
                // Serve the page (and the `tcmedia` scheme) as https://*.localhost
                // rather than http://: YouTube's embedded player refuses to
                // play for an embedder without a real https origin (error 153).
                .use_https_scheme(true)
                .initialization_script(FREEZE_PROTOTYPE_SCRIPT)
                .on_navigation(|url| {
                    let allowed = is_app_origin(url);
                    if !allowed {
                        tracing::warn!(%url, "blocked a navigation away from the app page");
                    }
                    allowed
                })
                // An embedded player's logo or "Watch on YouTube" asks for a
                // new window, which WebView2 would otherwise drop without a
                // trace. Open it in the system browser instead.
                .on_new_window(|url, _features| {
                    if let Err(error) = commands::open_external(url.to_string()) {
                        tracing::warn!(%url, %error, "refused a new-window request");
                    }
                    tauri::webview::NewWindowResponse::Deny
                })
                .build()?;
            commands::apply_zoom(&handle, initial_scale);
            if minimized {
                let _ = window.minimize();
            }

            if let Err(error) = tray::setup(&handle) {
                tracing::warn!(%error, "could not create the tray icon");
            }
            activity::spawn(handle, settings.clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bridge::attach_events,
            bridge::session_restore,
            bridge::discover_homeserver,
            bridge::login_password,
            bridge::login_sso,
            bridge::dispatch,
            bridge::own_profile,
            bridge::send_attachment,
            commands::app_info,
            commands::get_settings,
            commands::set_settings,
            commands::theme_presets,
            commands::fetch_tweet,
            commands::fetch_steam_app,
            commands::open_external,
            commands::save_media,
            commands::autostart_enabled,
            commands::set_autostart,
            commands::read_log,
            commands::clear_logs,
            commands::load_profile_file,
            commands::save_profile_file,
            commands::quit_app,
            spellcheck::spell_status,
            spellcheck::spell_check,
            spellcheck::spell_suggest,
            spellcheck::spell_correction,
            spellcheck::spell_add,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the ThornyChat application");

    app.run(move |app_handle, event| match event {
        // Closing the window quits the app, but not before the sync worker has
        // had a moment to leave any call this device is in.
        RunEvent::ExitRequested { api, .. } => {
            if shutting_down.swap(true, Ordering::SeqCst) {
                return;
            }
            api.prevent_exit();
            let app_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(bridge) = app_handle.try_state::<Arc<Bridge>>() {
                    bridge.shutdown(WORKER_GRACE).await;
                }
                app_handle.exit(0);
            });
        }
        // Past this point the window is gone and nothing is left worth waiting
        // for: what remains is the runtime join, the webview teardown and the
        // log flush. Any of those hanging strands a husk of a process, so put
        // a ceiling on all of them at once.
        RunEvent::Exit => arm_shutdown_deadline(),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> tauri::Url {
        text.parse().unwrap()
    }

    #[test]
    fn the_app_origin_is_navigable() {
        assert!(is_app_origin(&url("http://tauri.localhost/")));
        assert!(is_app_origin(&url("https://tauri.localhost/index.html")));
        assert!(is_app_origin(&url("tauri://localhost/")));
    }

    #[test]
    fn anything_else_is_not() {
        for bad in [
            "https://example.com/",
            "http://tauri.localhost.evil.example/",
            "file:///C:/Windows/win.ini",
            "http://127.0.0.1:1420/",
            "javascript:alert(1)",
            "http://tcmedia.localhost/mxc/a/b",
        ] {
            assert!(!is_app_origin(&url(bad)), "{bad}");
        }
    }
}

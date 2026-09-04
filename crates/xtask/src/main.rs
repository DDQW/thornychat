//! The release build script, run as `cargo xtask`. Builds the three release
//! binaries, each in its own target/ subdirectory so they don't overwrite
//! each other:
//!
//!   target/x86_64-pc-windows-msvc/release/thornychat.exe   (generic)
//!   target/znver4/x86_64-pc-windows-msvc/release/thornychat.exe
//!   target/znver5/x86_64-pc-windows-msvc/release/thornychat.exe
//!
//! Why these three (see README.md "Building" for the fuller rationale):
//!   generic - baseline x86-64: runs on any 64-bit CPU, the variant to hand
//!             to someone else or ship as a download.
//!   znver4  - AVX-512 (incl. IFMA) + DDR5, on Zen 4's double-pumped 256-bit
//!             units; this dev machine's native CPU.
//!   znver5  - Zen 5: a native full-width 512-bit AVX-512 datapath plus wider
//!             dispatch - the newest generation.
//!
//! Never ship a znverN (or target-cpu=native) binary to unknown hardware - a
//! CPU without those instructions dies with an illegal-instruction fault. The
//! generic build is the one that's safe everywhere.
//!
//! This is the standard release-build approach for ThornyChat - prefer
//! `cargo xtask` over a bare `cargo build --release` (which produces only the
//! generic variant).
//!
//! It also carries the local dev installer for Win11 toast notifications,
//! which need an AUMID registration this repo can't get from a bare exe (see
//! `ui::platform::app_identity`):
//!
//!   cargo xtask install-dev [--debug]   register this machine for toasts
//!   cargo xtask uninstall-dev           undo that
//!   cargo xtask toast-test [--debug]    fire one toast to prove it works
//!
//! Those three only ever *run* an already-built exe - none of them build.

use std::env;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::Path;
use std::process::{exit, Command};

const EXE_SUBPATH: &str = "x86_64-pc-windows-msvc/release/thornychat.exe";
const VARIANTS: [&str; 2] = ["znver4", "znver5"];

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // Bare `cargo xtask` stays the three-variant release build.
        None => build_all(&root),
        Some("install-dev") => identity(&root, "--install-dev", &args[1..]),
        Some("uninstall-dev") => identity(&root, "--uninstall-dev", &args[1..]),
        Some("toast-test") => identity(&root, "--toast-test", &args[1..]),
        Some(other) => {
            eprintln!("error: unknown command `{other}`");
            eprintln!();
            eprintln!("usage:");
            eprintln!("  cargo xtask                        build the three release variants");
            eprintln!("  cargo xtask install-dev [--debug]  register this build for Win11 toasts");
            eprintln!("  cargo xtask uninstall-dev          remove that registration");
            eprintln!("  cargo xtask toast-test [--debug]   fire a test toast");
            exit(2);
        }
    }
}

/// Runs one of the app's toast-identity commands against an already-built exe
/// (see `ui::platform::app_identity`). Deliberately doesn't build anything: it
/// registers the binary you have, and says so plainly when there isn't one,
/// rather than kicking off a multi-minute release build as a side effect.
///
/// `--debug` targets `target/<triple>/debug/thornychat.exe` instead, for when
/// the binary you actually run is a `cargo run` build. The registration points
/// at whichever exe it was run from, so installing from one and then running
/// the other means re-running install-dev.
fn identity(root: &Path, flag: &str, rest: &[String]) {
    let profile = if rest.iter().any(|a| a == "--debug") { "debug" } else { "release" };
    let exe = root.join("target").join("x86_64-pc-windows-msvc").join(profile).join("thornychat.exe");
    if !exe.exists() {
        eprintln!("error: {} not found.", exe.display());
        eprintln!(
            "       build it first ({}), then re-run this command.",
            if profile == "debug" { "cargo build" } else { "cargo xtask" }
        );
        exit(1);
    }

    // Inheriting stdio is what makes the child's output visible: the release
    // exe is GUI-subsystem and never gets a console of its own, but it writes
    // to the handles this console process passes down.
    let status = Command::new(&exe).arg(flag).status().unwrap_or_else(|e| {
        eprintln!("error: failed to run {}: {e}", exe.display());
        exit(1);
    });
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
}

fn build_all(root: &Path) {
    // The linker can't overwrite a running exe - catch that up front instead
    // of failing minutes into a build. Opening for write trips the same file
    // lock the linker would; a copy running from some other directory doesn't
    // hold these paths and correctly doesn't block.
    let outputs: Vec<_> = std::iter::once(root.join("target").join(EXE_SUBPATH))
        .chain(VARIANTS.iter().map(|v| root.join("target").join(v).join(EXE_SUBPATH)))
        .collect();
    for exe in &outputs {
        match OpenOptions::new().write(true).open(exe) {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!(
                    "error: {} can't be overwritten ({e}) - close the running app first.",
                    exe.display()
                );
                exit(1);
            }
        }
    }

    // Generic first: no target-cpu flag, default target dir - identical to
    // what a plain `cargo build --release` produces.
    build("generic (baseline x86-64)", root, None);
    for v in VARIANTS {
        build(v, root, Some(v));
    }

    println!();
    println!("Built binaries:");
    println!("  target/{EXE_SUBPATH}   (generic)");
    for v in VARIANTS {
        println!("  target/{v}/{EXE_SUBPATH}");
    }
}

fn build(label: &str, root: &Path, target_cpu: Option<&str>) {
    println!("=== Building {label} ===");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.args(["build", "--release"])
        .current_dir(root)
        // Scrub inherited env so a RUSTFLAGS/CARGO_TARGET_DIR from the parent
        // shell can't leak into the generic build.
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR");
    if let Some(cpu) = target_cpu {
        cmd.env("RUSTFLAGS", format!("-C target-cpu={cpu}"))
            .env("CARGO_TARGET_DIR", format!("target/{cpu}"));
    }
    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("error: failed to spawn cargo: {e}");
        exit(1);
    });
    if !status.success() {
        eprintln!("error: {label} build failed");
        exit(1);
    }
}

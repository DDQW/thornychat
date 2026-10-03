//! The build script, run as `cargo xtask`.
//!
//! ThornyChat is a Tauri app: the Rust shell embeds a built copy of the Svelte
//! frontend (`frontend/dist`), so every release build starts by building that.
//!
//!   cargo xtask                 build the frontend, then the three release variants
//!   cargo xtask installer       build the frontend, then the NSIS installer (generic CPU)
//!   cargo xtask frontend        build only the frontend
//!   cargo xtask bindings        regenerate the TypeScript types from client-core
//!
//! The three variants each get their own target/ subdirectory so they don't
//! overwrite each other:
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
//! generic build is the one that's safe everywhere. The CPU level only matters
//! to matrix-sdk's crypto; the webview does the rest.
//!
//! A bare `cargo build --release` is NOT a substitute: without the
//! `custom-protocol` feature the exe loads the dev server instead of its
//! embedded frontend. These commands pass it for you.

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
        None => {
            build_frontend(&root);
            build_all(&root);
        }
        Some("installer") => {
            build_frontend(&root);
            installer(&root);
        }
        Some("frontend") => build_frontend(&root),
        Some("bindings") => bindings(&root),
        Some(other) => {
            eprintln!("error: unknown command `{other}`");
            eprintln!();
            eprintln!("usage:");
            eprintln!("  cargo xtask             build the frontend and the three release variants");
            eprintln!("  cargo xtask installer   build the frontend and the NSIS installer");
            eprintln!("  cargo xtask frontend    build only the frontend");
            eprintln!("  cargo xtask bindings    regenerate frontend/src/lib/bindings from client-core");
            exit(2);
        }
    }
}

/// `npm` is a `.cmd` shim on Windows, which `Command` won't find by bare name.
fn npm() -> Command {
    Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" })
}

fn run(label: &str, cmd: &mut Command) {
    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("error: failed to run {label}: {e}");
        exit(1);
    });
    if !status.success() {
        eprintln!("error: {label} failed");
        exit(1);
    }
}

fn build_frontend(root: &Path) {
    println!("=== Building the frontend ===");
    let frontend = root.join("frontend");
    // A fresh checkout has no node_modules; `ci` installs exactly what the
    // lockfile says.
    if !frontend.join("node_modules").exists() {
        run("npm ci", npm().args(["ci"]).current_dir(&frontend));
    }
    run("npm run build", npm().args(["run", "build"]).current_dir(&frontend));
}

/// Regenerates the TypeScript types the frontend imports. They are written by
/// ts-rs tests (see `.cargo/config.toml` for the output directory); after a new
/// type appears, add it to `frontend/src/lib/bindings/index.ts` by hand.
fn bindings(root: &Path) {
    println!("=== Regenerating TypeScript bindings ===");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    run(
        "cargo test (ts export)",
        Command::new(cargo).args(["test", "-p", "client-core", "--features", "ts"]).current_dir(root),
    );
}

fn installer(root: &Path) {
    println!("=== Building the installer (generic) ===");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    run(
        "cargo tauri build",
        Command::new(cargo)
            .args(["tauri", "build", "--bundles", "nsis"])
            .current_dir(root.join("crates/desktop"))
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_TARGET_DIR"),
    );
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

    // Generic first: no target-cpu flag, default target dir.
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
    cmd.args(["build", "--release", "-p", "thornychat", "--features", "custom-protocol"])
        .current_dir(root)
        // Scrub inherited env so a RUSTFLAGS/CARGO_TARGET_DIR from the parent
        // shell can't leak into the generic build.
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR");
    if let Some(cpu) = target_cpu {
        cmd.env("RUSTFLAGS", format!("-C target-cpu={cpu}")).env("CARGO_TARGET_DIR", format!("target/{cpu}"));
    }
    run(&format!("{label} build"), &mut cmd);
}

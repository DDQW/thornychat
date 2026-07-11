//! Embeds the exe icon (Explorer/taskbar) into the Windows binary. The .rc
//! source is one generated line in OUT_DIR rather than a tracked file.
//! `manifest_optional` keeps the build alive on toolchains without a resource
//! compiler (rc.exe / llvm-rc) — the exe just ships without an embedded icon
//! there.

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=../../assets/thornychat.ico");
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("crate dir has no workspace root")
        .join("assets")
        .join("thornychat.ico");
    // The rc lives in OUT_DIR, so the icon path must be absolute. No
    // canonicalize() — its \\?\ prefix breaks rc.exe; double the backslashes
    // for the RC string literal instead.
    let rc = format!(
        "1 ICON \"{}\"\n",
        icon.display().to_string().replace('\\', "\\\\")
    );
    let rc_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("app.rc");
    fs::write(&rc_path, rc).expect("failed to write app.rc");
    embed_resource::compile(&rc_path, embed_resource::NONE)
        .manifest_optional()
        .expect("failed to compile app.rc");
}

//! Embeds app.rc (the exe icon) into the Windows binary. `manifest_optional`
//! keeps the build alive on toolchains without a resource compiler
//! (rc.exe / llvm-rc) — the exe just ships without an embedded icon there.

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=../../assets/thornychat.ico");
    embed_resource::compile("app.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("failed to compile app.rc");
}

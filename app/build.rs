//! Embeds the Windows manifest: modern control styles and per-monitor DPI for the settings window.
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_manifest(new_manifest("kb16-mixer")).expect("unable to embed manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}

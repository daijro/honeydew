use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_app_font)");
    println!("cargo::rustc-check-cfg=cfg(plugin_host)");
    // Third-party plugins (VST3, VST2, CLAP) are hosted on the desktop only: not on the web, not on Android.
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if arch != "wasm32" && os != "android" {
        println!("cargo::rustc-cfg=plugin_host");
    }
    println!("cargo::rerun-if-changed=assets/fonts");
    if Path::new("assets/fonts/OpenSans_SemiCondensed-SemiBold.ttf").exists() {
        println!("cargo::rustc-cfg=has_app_font");
    }
}

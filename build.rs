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
    // `desktop`: the desktop sound modes, the MCP token and multi-core mixing (rayon, getrandom). `x11`: plugin editor
    // windows are X11 ones (x11rb). `native_midi`: midir has a MIDI input backend (its ALSA one is Linux only). Each
    // set matches its dependencies' target in Cargo.toml.
    for (name, systems) in [
        ("desktop", &["linux", "freebsd", "macos", "windows"][..]),
        ("x11", &["linux", "freebsd"]),
        ("native_midi", &["linux", "macos", "windows"]),
    ] {
        println!("cargo::rustc-check-cfg=cfg({name})");
        if systems.contains(&os.as_str()) {
            println!("cargo::rustc-cfg={name}");
        }
    }
    println!("cargo::rerun-if-changed=assets/fonts");
    if Path::new("assets/fonts/OpenSans_SemiCondensed-SemiBold.ttf").exists() {
        println!("cargo::rustc-cfg=has_app_font");
    }
}

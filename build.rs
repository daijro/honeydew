use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_app_font)");
    println!("cargo::rustc-check-cfg=cfg(plugin_host)");
    // Third-party plugins (VST3, VST2, CLAP) are hosted on the desktop only: not on the web, Android or iOS.
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if arch != "wasm32" && os != "android" && os != "ios" {
        println!("cargo::rustc-cfg=plugin_host");
    }
    if os == "ios" || os == "macos" {
        build_auv3(&os);
    }
    // `desktop`: the desktop sound modes, the MCP token and multi-core mixing (rayon, getrandom). `x11`: plugin editor
    // windows are X11 ones (x11rb). `native_midi`: midir has a MIDI input backend (its ALSA one is Linux only). Each
    // set matches its dependencies' target in Cargo.toml. `mobile`: the phones (Android, iOS), which have no file
    // dialogs, file manager or desktop sound library.
    for (name, systems) in [
        ("desktop", &["linux", "freebsd", "macos", "windows"][..]),
        ("x11", &["linux", "freebsd"]),
        ("native_midi", &["linux", "macos", "windows"]),
        ("mobile", &["android", "ios"]),
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

/// Compile the Apple Audio Unit bridge with the selected Xcode SDK; no new Cargo dependencies.
fn build_auv3(os: &str) {
    use std::process::Command;
    let target = std::env::var("TARGET").expect("TARGET");
    let simulator = os == "ios" && (target.ends_with("-sim") || target.starts_with("x86_64"));
    let sdk = if os == "macos" {
        "macosx"
    } else if simulator {
        "iphonesimulator"
    } else {
        "iphoneos"
    };
    let arch = if target.starts_with("aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    let triple = if os == "macos" {
        let minimum = std::env::var("MACOSX_DEPLOYMENT_TARGET")
            .unwrap_or_else(|_| if arch == "arm64" { "11.0" } else { "10.13" }.into());
        format!("{arch}-apple-macos{minimum}")
    } else {
        let minimum = std::env::var("IPHONEOS_DEPLOYMENT_TARGET").unwrap_or_else(|_| "14.0".into());
        format!(
            "{arch}-apple-ios{minimum}{}",
            if simulator { "-simulator" } else { "" }
        )
    };
    let sdk_path = Command::new("xcrun")
        .args(["--sdk", sdk, "--show-sdk-path"])
        .output()
        .expect("Xcode SDK required for AUv3");
    assert!(sdk_path.status.success(), "Xcode SDK required for AUv3");
    let sdk_path = String::from_utf8(sdk_path.stdout).expect("SDK path");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let object = out.join("auv3.o");
    let status = Command::new("xcrun")
        .args([
            "--sdk",
            sdk,
            "clang",
            "-target",
            &triple,
            "-isysroot",
            sdk_path.trim(),
            "-fobjc-arc",
            "-fblocks",
            "-O2",
            "-Wall",
            "-Wextra",
            "-c",
            "src/plugins/auv3/bridge.m",
            "-o",
        ])
        .arg(&object)
        .status()
        .expect("clang");
    assert!(status.success(), "AUv3 bridge compilation failed");
    let status = Command::new("xcrun")
        .args(["--sdk", sdk, "ar", "crs"])
        .arg(out.join("libhoneydew_auv3.a"))
        .arg(&object)
        .status()
        .expect("ar");
    assert!(status.success(), "AUv3 bridge archive failed");
    println!("cargo::rustc-link-search=native={}", out.display());
    println!("cargo::rustc-link-lib=static=honeydew_auv3");
    for framework in ["AVFoundation", "AudioToolbox", "CoreAudioKit", "Foundation"] {
        println!("cargo::rustc-link-lib=framework={framework}");
    }
    println!(
        "cargo::rustc-link-lib=framework={}",
        if os == "macos" { "AppKit" } else { "UIKit" }
    );
    println!("cargo::rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    println!("cargo::rerun-if-changed=src/plugins/auv3/bridge.m");
    println!("cargo::rerun-if-changed=src/plugins/auv3/bridge.h");
    println!("cargo::rerun-if-env-changed=IPHONEOS_DEPLOYMENT_TARGET");
    println!("cargo::rerun-if-env-changed=DEVELOPER_DIR");
}

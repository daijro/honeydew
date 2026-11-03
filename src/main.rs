//! Native entry point. The web build starts from `lib.rs` instead.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    // The plugin scanner: this program loading one plugin file in a process of its own (`plugins::scan`).
    #[cfg(plugin_host)]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().map(String::as_str) == Some(honeydew::plugins::scan::ARG) {
            honeydew::plugins::scan::child_main(&args[1..]);
        }
    }
    let icon = include_bytes!("../assets/branding/honeydew-256.png");
    let options = eframe::NativeOptions {
        // A landscape phone shape: about 36.6 U x 16.9 U at U = 32.
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1170.0, 540.0])
            // Wayland ignores the window icon: the compositor takes the `Icon=` of the desktop entry named after the
            // app_id (`assets/linux/honeydew.desktop`, installed by `tools/install-desktop-entry.sh`). On X11 the
            // app_id is also the WM_CLASS, which the entry's `StartupWMClass` matches.
            .with_app_id("honeydew")
            // X11, Windows and macOS show this one.
            .with_icon(eframe::icon_data::from_png_bytes(icon).expect("our icon is a PNG")),
        ..Default::default()
    };
    let result = eframe::run_native(
        honeydew::APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(honeydew::app::HoneydewApp::new(cc)))),
    );
    #[cfg(target_os = "windows")]
    if let Err(error) = &result {
        rfd::MessageDialog::new()
            .set_title(format!("{} could not start", honeydew::APP_NAME))
            .set_description(error.to_string())
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
    result
}

// The wasm32 bin target still has to compile; the real web entry point is in `lib.rs`.
#[cfg(target_arch = "wasm32")]
fn main() {}

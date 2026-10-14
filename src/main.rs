//! Native entry point. The web build starts from `lib.rs` instead.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
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
    eframe::run_native(
        honeydew::APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(honeydew::app::HoneydewApp::new(cc)))),
    )
}

// The wasm32 bin target still has to compile; the real web entry point is in `lib.rs`.
#[cfg(target_arch = "wasm32")]
fn main() {}

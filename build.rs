use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_app_font)");
    println!("cargo::rerun-if-changed=assets/fonts");
    if Path::new("assets/fonts/OpenSans_SemiCondensed-SemiBold.ttf").exists() {
        println!("cargo::rustc-cfg=has_app_font");
    }
}

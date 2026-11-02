//! `honeydew-import`: the Original Content importer as a command. It unpacks a user-provided `.apk`
//! or `.obb` into the Original Content folder, as the app does with one dropped there
//! ([`honeydew::project_files::import_into`]): the same guards, markers and never-overwrite rules. It reads only the
//! files it is given and writes only below the folder; it downloads, decrypts and sends nothing. Desktop only.
//!
//! ```sh
//! cargo run --release --bin honeydew-import -- path/to/your.apk [--to <folder>]
//! ```

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    use std::path::PathBuf;
    use std::process::ExitCode;

    use honeydew::project_files::{
        CONTENT_FOLDER, Counts, import_into, import_notice, import_target_allowed,
    };

    const USAGE: &str = "Usage: honeydew-import <archive.apk|archive.obb>... [--to <folder>]\n\
        The folder is the app's Original Content folder unless --to names another.";
    let (mut to, mut archives) = (None, Vec::new());
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--to") => {
                let Some(dir) = args.next() else {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                };
                to = Some(PathBuf::from(dir));
            }
            Some("-h" | "--help") => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => archives.push(PathBuf::from(arg)),
        }
    }
    if archives.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let songs = honeydew::ui::main_menu::songs_dir();
    let user = songs.as_deref().and_then(|s| s.parent());
    let Some(to) = to.or_else(|| Some(user?.join(CONTENT_FOLDER))) else {
        eprintln!("No user folder here: name one with --to.");
        return ExitCode::from(2);
    };
    // Imported content stays where the project export's guard sees it.
    if user.is_some_and(|user| !import_target_allowed(user, &to)) {
        eprintln!(
            "{}: inside the user folder, --to must be its {CONTENT_FOLDER} folder.",
            to.display()
        );
        return ExitCode::from(2);
    }
    println!("{}\n", import_notice().join("\n\n"));
    let mut failed = false;
    for archive in &archives {
        let mut counts = Counts::default();
        let done = import_into(&to, archive, &mut counts);
        let Counts { written, skipped } = counts;
        println!(
            "{}: {written} written, {skipped} skipped",
            archive.display()
        );
        if let Err(e) = done {
            eprintln!("{}: {e}", archive.display());
            failed = true;
        }
    }
    println!("Into {}", to.display());
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

// The wasm32 bin target still has to compile, as `main.rs`'s; there is no content folder on the web.
#[cfg(target_arch = "wasm32")]
fn main() {}

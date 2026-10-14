//! Honeydew: an open-source app inspired by a mobile DAW's workflow.

#[cfg(target_os = "android")]
mod android;
mod android_text;
pub mod app;
pub mod audio_bridge;
pub mod audio_engine;
pub mod audio_link;
pub mod content;
pub mod flm;
pub mod fonts;
pub mod history;
pub mod icons;
pub mod keymap;
pub mod metrics;
pub mod midi;
pub mod panels;
pub mod params;
pub mod project_files;
pub mod recording;
pub mod render;
pub mod sample_cache;
pub mod settings;
pub mod sf2;
pub mod smf;
pub mod song;
pub mod theme;
pub mod ui;
pub mod zip;

/// Display name, used for the window title, and the name of the app's data and settings folders.
pub const APP_NAME: &str = "Honeydew";

/// The app's former name: its data and settings folders, and its web storage keys, were named after it.
pub const LEGACY_APP_NAME: &str = "Studio";

/// The folder `new`, named after the app, which `old`, named after its former name, becomes the first time: when `new`
/// is missing (or an empty folder) and `old` is the app's folder ([`is_legacy_app_dir`]), `old` is renamed to `new`
/// (one rename in the same parent, nothing copied). Another program's folder of that name is left alone. When the
/// rename fails, `old` stays in use, so nothing is lost, unless another instance has just renamed it.
#[cfg(not(target_arch = "wasm32"))]
pub fn migrate_legacy_dir(old: &std::path::Path, new: &std::path::Path) -> std::path::PathBuf {
    if !(old.is_dir() && is_legacy_app_dir(old)) {
        return new.to_owned();
    }
    // An empty new folder gives way: `remove_dir` removes only an empty one, never a link.
    if std::fs::symlink_metadata(new).is_ok_and(|m| m.is_dir()) {
        let _ = std::fs::remove_dir(new);
    }
    if new.exists() {
        eprintln!(
            "{} and {} both exist: using {}",
            old.display(),
            new.display(),
            new.display()
        );
        return new.to_owned();
    }
    match std::fs::rename(old, new) {
        Ok(()) => {
            eprintln!("renamed {} to {}", old.display(), new.display());
            new.to_owned()
        }
        // Another instance starting at the same time renamed it first.
        Err(_) if !old.is_dir() => new.to_owned(),
        Err(err) => {
            eprintln!(
                "kept {}, not renamed to {}: {err}",
                old.display(),
                new.display()
            );
            old.to_owned()
        }
    }
}

/// `dir`, a folder of the app's former name, is provably the app's, not another program's of the same name: as the data
/// folder it holds the songs folder or `Original Content` (on Windows in its `data`; a `Recently Deleted` alone is
/// too common a name); as the config folder, a `storage.json` holding a key of the former name
/// ([`settings::is_legacy_key`]). One check for both, as on Windows they are one folder.
#[cfg(not(target_arch = "wasm32"))]
pub fn is_legacy_app_dir(dir: &std::path::Path) -> bool {
    let folders = [ui::main_menu::SONGS_FOLDER, project_files::CONTENT_FOLDER];
    let data = |d: std::path::PathBuf| folders.iter().any(|f| d.join(f).is_dir());
    let settings = || {
        let json = std::fs::read_to_string(dir.join("storage.json")).ok();
        json.and_then(|j| {
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&j).ok()
        })
        .is_some_and(|map| map.keys().any(|k| settings::is_legacy_key(k)))
    };
    data(dir.to_owned()) || data(dir.join("data")) || settings()
}

/// [`migrate_legacy_dir`] at most once per process for each pair of folders: later calls get the first one's answer.
/// On Windows the settings and the data share one folder, and a second try that renamed it after a failed first one
/// would leave the settings, opened in the old folder, behind.
#[cfg(not(target_arch = "wasm32"))]
pub fn migrate_legacy_dir_once(old: &std::path::Path, new: &std::path::Path) -> std::path::PathBuf {
    use std::path::PathBuf;
    static DONE: std::sync::Mutex<std::collections::BTreeMap<(PathBuf, PathBuf), PathBuf>> =
        std::sync::Mutex::new(std::collections::BTreeMap::new());
    let mut done = DONE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    (done.entry((old.to_owned(), new.to_owned())))
        .or_insert_with(|| migrate_legacy_dir(old, new))
        .clone()
}

/// Web entry point: runs the app on the `<canvas id="the_canvas_id">` of `index.html`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    wasm_bindgen_futures::spawn_local(async {
        if let Err(err) = run_web().await {
            web_sys::console::error_1(&err);
        }
    });
}

#[cfg(target_arch = "wasm32")]
use eframe::egui::{Key, Modifiers};

#[cfg(target_arch = "wasm32")]
async fn run_web() -> Result<(), wasm_bindgen::JsValue> {
    use wasm_bindgen::{JsCast as _, JsValue};

    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("no document"))?;
    let canvas = document
        .get_element_by_id("the_canvas_id")
        .ok_or_else(|| JsValue::from_str("canvas #the_canvas_id not found"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| JsValue::from_str("#the_canvas_id is not a <canvas>"))?;

    // The bundled content's index, before the app's first `content::factory()`.
    content::web::start().await;
    let runner = eframe::WebRunner::new();
    runner
        .start(
            canvas.clone(),
            eframe::WebOptions::default(),
            Box::new(|cc| Ok(Box::new(app::HoneydewApp::new(cc)))),
        )
        .await?;
    // eframe leaves most keys their browser action too (Ctrl+= zooms the page, Ctrl+F finds, Ctrl+K and F6 go to the
    // address bar): a key the keymap binds is the app's alone (`keymap.rs`, "Web").
    runner.add_event_listener(
        &canvas,
        "keydown",
        |event: web_sys::KeyboardEvent, runner| {
            if bound_key(&event, runner.egui_ctx()).is_some() {
                event.prevent_default();
            }
        },
    )?;
    // While a text field has the keyboard its keys go to eframe's text agent, not the canvas: there only a bound
    // Ctrl/Cmd key is kept from the browser, and not Ctrl+A or Z, which edit the text.
    let capture = web_sys::AddEventListenerOptions::new();
    capture.set_capture(true);
    runner.add_event_listener_ex(
        &document,
        "keydown",
        &capture,
        |event: web_sys::KeyboardEvent, runner, _| {
            let text_safe =
                |(at, mods): (Key, Modifiers)| mods.command && !matches!(at, Key::A | Key::Z);
            if bound_key(&event, runner.egui_ctx()).is_some_and(text_safe) {
                event.prevent_default();
            }
        },
    )?;
    // Closing or reloading the page with unsaved changes asks first. The desktop's close asks the Quit
    // question whatever is saved; the web has no Quit question, so it guards unsaved work here.
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    runner.add_event_listener(
        &window,
        "beforeunload",
        |event: web_sys::BeforeUnloadEvent, runner| {
            if runner.app_mut::<app::HoneydewApp>().unsaved() {
                event.prevent_default();
                // Older browsers ask only when a return value is set.
                event.set_return_value("unsaved");
            }
        },
    )?;
    // A hidden page runs no frame (the browser stops requestAnimationFrame) to see its focus loss: its notes stop
    // here, or they would sound until it shows again.
    runner.add_event_listener(
        &document.clone(),
        "visibilitychange",
        move |_: JsValue, runner| {
            if document.hidden() {
                runner.app_mut::<app::HoneydewApp>().on_hidden();
            }
        },
    )
}

/// The keymap key ([`keymap::key_at`]) and the modifiers of browser key `event`, when some action binds them
/// ([`keymap::Keymap::binds`]).
#[cfg(target_arch = "wasm32")]
fn bound_key(
    event: &web_sys::KeyboardEvent,
    ctx: &eframe::egui::Context,
) -> Option<(Key, Modifiers)> {
    let mods = Modifiers {
        alt: event.alt_key(),
        ctrl: event.ctrl_key(),
        shift: event.shift_key(),
        mac_cmd: event.meta_key(),
        command: event.ctrl_key() || event.meta_key(),
    };
    let at = keymap::key_at(Key::from_name(&event.key())?, None);
    keymap::for_os(ctx.os())
        .binds(at, mods)
        .then_some((at, mods))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{migrate_legacy_dir, migrate_legacy_dir_once};

    /// A fresh temp folder for `name`, removed on drop.
    struct Temp(std::path::PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("br-migrate-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Makes `old` a data folder of the app's former name: it holds the songs folder.
    fn ours(old: &std::path::Path) {
        std::fs::create_dir_all(old.join("My Songs")).unwrap();
    }

    /// A folder of the former name that is not provably the app's (another program's `studio`, its own `storage.json`
    /// without the former settings key) is left alone, and the new folder is used, even an empty one.
    #[test]
    fn another_programs_folder_of_the_former_name_is_left_alone() {
        let tmp = Temp::new("foreign");
        let (old, new) = (tmp.0.join("studio"), tmp.0.join("honeydew"));
        std::fs::create_dir_all(old.join("data").join("cache")).unwrap();
        std::fs::write(old.join("theirs.txt"), b"theirs").unwrap();
        std::fs::write(old.join("storage.json"), r#"{"other.settings":"x"}"#).unwrap();
        assert_eq!(migrate_legacy_dir(&old, &new), new);
        assert_eq!(std::fs::read(old.join("theirs.txt")).unwrap(), b"theirs");
        assert!(!new.exists());
        std::fs::create_dir_all(&new).unwrap();
        assert_eq!(migrate_legacy_dir(&old, &new), new);
        assert!(new.is_dir() && old.join("theirs.txt").is_file());
        // Damaged JSON is no proof either, nor a `Recently Deleted` alone: any program may have one.
        std::fs::write(old.join("storage.json"), "{").unwrap();
        std::fs::create_dir_all(old.join("Recently Deleted")).unwrap();
        assert_eq!(migrate_legacy_dir(&old, &new), new);
        assert!(old.join("theirs.txt").is_file());
    }

    /// The app's folder of its former name is known by what it holds: as the data folder, the songs folder or the
    /// original content (on Windows in its `data`); as the config folder, a `storage.json` holding a key of the former
    /// name.
    #[test]
    fn a_folder_of_the_former_name_holding_the_apps_files_migrates() {
        let markers = ["My Songs", "Original Content", "data/My Songs"];
        for (i, marker) in markers.into_iter().enumerate() {
            let tmp = Temp::new(&format!("ours-{i}"));
            let (old, new) = (tmp.0.join("studio"), tmp.0.join("honeydew"));
            std::fs::create_dir_all(old.join(marker)).unwrap();
            assert_eq!(migrate_legacy_dir(&old, &new), new, "{marker}");
            assert!(!old.exists() && new.join(marker).is_dir(), "{marker}");
        }
        let tmp = Temp::new("ours-settings");
        let (old, new) = (tmp.0.join("Studio"), tmp.0.join("Honeydew"));
        std::fs::create_dir_all(&old).unwrap();
        let json = serde_json::json!({ "studio.settings": "{}" }).to_string();
        std::fs::write(old.join("storage.json"), &json).unwrap();
        assert_eq!(migrate_legacy_dir(&old, &new), new);
        assert_eq!(
            std::fs::read_to_string(new.join("storage.json")).unwrap(),
            json
        );
        // Any key of the former name will do: only the routing preferences were ever stored.
        let tmp = Temp::new("ours-routing");
        let (old, new) = (tmp.0.join("Studio"), tmp.0.join("Honeydew"));
        std::fs::create_dir_all(&old).unwrap();
        let json = serde_json::json!({ "studio.routingPrefs": "{}" }).to_string();
        std::fs::write(old.join("storage.json"), &json).unwrap();
        assert_eq!(migrate_legacy_dir(&old, &new), new);
        assert!(!old.exists() && new.join("storage.json").is_file());
    }

    #[test]
    fn the_legacy_folder_is_renamed_when_the_new_one_is_missing() {
        let tmp = Temp::new("renamed");
        let (old, new) = (tmp.0.join("studio"), tmp.0.join("honeydew"));
        std::fs::create_dir_all(old.join("My Songs")).unwrap();
        std::fs::write(old.join("My Songs").join("A.flm"), b"song").unwrap();
        assert_eq!(migrate_legacy_dir(&old, &new), new);
        assert!(!old.exists());
        assert_eq!(
            std::fs::read(new.join("My Songs").join("A.flm")).unwrap(),
            b"song"
        );
    }

    #[test]
    fn an_existing_new_folder_is_used_and_the_legacy_one_left_alone() {
        let tmp = Temp::new("both");
        let (old, new) = (tmp.0.join("studio"), tmp.0.join("honeydew"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("old.txt"), b"old").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("new.txt"), b"new").unwrap();
        assert_eq!(migrate_legacy_dir(&old, &new), new);
        assert_eq!(std::fs::read(old.join("old.txt")).unwrap(), b"old");
        assert_eq!(std::fs::read(new.join("new.txt")).unwrap(), b"new");
        assert!(!new.join("old.txt").exists());
    }

    #[test]
    fn without_a_legacy_folder_the_new_one_is_used_and_not_made() {
        let tmp = Temp::new("fresh");
        let (old, new) = (tmp.0.join("studio"), tmp.0.join("honeydew"));
        assert_eq!(migrate_legacy_dir(&old, &new), new);
        assert!(!new.exists() && !old.exists());
    }

    #[test]
    fn a_failed_rename_keeps_the_legacy_folder_in_use() {
        let tmp = Temp::new("failed");
        // The new folder's parent is missing, so the rename fails.
        let (old, new) = (tmp.0.join("studio"), tmp.0.join("missing").join("honeydew"));
        ours(&old);
        std::fs::write(old.join("keep.txt"), b"keep").unwrap();
        assert_eq!(migrate_legacy_dir(&old, &new), old);
        assert_eq!(std::fs::read(old.join("keep.txt")).unwrap(), b"keep");
        assert!(!new.exists());
    }

    #[test]
    fn an_empty_new_folder_gives_way_to_the_legacy_one() {
        let tmp = Temp::new("empty-new");
        let (old, new) = (tmp.0.join("studio"), tmp.0.join("honeydew"));
        ours(&old);
        std::fs::write(old.join("keep.txt"), b"keep").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        assert_eq!(migrate_legacy_dir(&old, &new), new);
        assert!(!old.exists());
        assert_eq!(std::fs::read(new.join("keep.txt")).unwrap(), b"keep");
    }

    #[test]
    fn instances_starting_together_all_use_the_renamed_folder() {
        for round in 0..20 {
            let tmp = Temp::new(&format!("race-{round}"));
            let (old, new) = (tmp.0.join("studio"), tmp.0.join("honeydew"));
            ours(&old);
            std::fs::write(old.join("keep.txt"), b"keep").unwrap();
            let barrier = std::sync::Barrier::new(8);
            let results: Vec<_> = std::thread::scope(|s| {
                let racers: Vec<_> = (0..8)
                    .map(|_| {
                        s.spawn(|| {
                            barrier.wait();
                            migrate_legacy_dir(&old, &new)
                        })
                    })
                    .collect();
                racers.into_iter().map(|r| r.join().unwrap()).collect()
            });
            assert!(!old.exists(), "round {round}");
            assert!(
                results.iter().all(|r| *r == new),
                "round {round}: {results:?}"
            );
            assert_eq!(std::fs::read(new.join("keep.txt")).unwrap(), b"keep");
        }
    }

    #[test]
    fn a_pair_is_migrated_once_per_process() {
        let tmp = Temp::new("once");
        let (old, new) = (tmp.0.join("studio"), tmp.0.join("missing").join("honeydew"));
        ours(&old);
        // The first try fails (no parent) and keeps the old folder; a second, which could rename it now, does not run.
        assert_eq!(migrate_legacy_dir_once(&old, &new), old);
        std::fs::create_dir_all(new.parent().unwrap()).unwrap();
        assert_eq!(migrate_legacy_dir_once(&old, &new), old);
        assert!(old.is_dir() && !new.exists());
    }
}

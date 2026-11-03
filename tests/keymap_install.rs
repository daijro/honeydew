//! `keymap::install` makes the merged table what `for_os` returns. Its own test binary: the table is
//! installed once per process.

use eframe::egui::{Context, Event, Key, Modifiers, RawInput, os::OperatingSystem};
use honeydew::keymap;

#[test]
fn the_installed_table_is_the_one_for_os_returns_for_this_platform() {
    let os = if cfg!(any(target_os = "macos", target_os = "ios")) {
        OperatingSystem::Mac
    } else {
        OperatingSystem::Windows
    };
    // The primary modifier is `ctrl` in the Windows table; in the Mac table `ctrl` is the physical Ctrl, so the
    // user's file says `cmd`.
    let (primary, held) = if os.is_mac() {
        ("cmd", Modifiers::MAC_CMD | Modifiers::COMMAND)
    } else {
        ("ctrl", Modifiers::CTRL | Modifiers::COMMAND)
    };
    let dir = std::env::temp_dir().join(format!("honeydew-keymap-install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("keymap.json"),
        format!(
            r#"{{"modifiers":[],"actions":[{{"key":"{primary}+q","action":"mine"}},{{"key":"{primary}+s","action":""}}]}}"#
        ),
    )
    .unwrap();
    // Before: the built-in map.
    let press = |action: &str, key: Key| {
        let ctx = Context::default();
        ctx.set_os(os);
        let event = Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: held,
        };
        let mut fired = false;
        let input = RawInput {
            events: vec![event],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            fired = ctx.input_mut(|i| keymap::for_os(os).consume_press(i, action))
        });
        fired
    };
    assert!(press("saveProject", Key::S));
    assert!(!press("mine", Key::Q));
    keymap::install(None, Some(&dir));
    // After: the user's file added `mine` and removed the primary modifier + S.
    assert!(press("mine", Key::Q));
    assert!(!press("saveProject", Key::S));
    assert!(press("undo", Key::Z));
    // Once: a second install changes nothing.
    keymap::install(None, None);
    assert!(press("mine", Key::Q));
    let _ = std::fs::remove_dir_all(&dir);
}

//! Keyboard shortcut maps: the JSON files in `assets/content/keymaps/` (ours), parsed into bindings that the app asks
//! by action ID, and merged at start with the files of the Original Content folder and the user's ([`load`],
//! [`install`]).
//!
//! Details:
//! - **Merge order**: our built-in map for the platform, then the content folder's
//!   `Keyboard Shortcuts/<platform>.json`, then its `gamepad.json`, then `keymap.json` in the user folder (the one
//!   `@L@` names). The built-in map is always the base, so the app has shortcuts without Original Content; the content
//!   folder's own file adds to it (a combination may then list an action twice: duplicates are kept) or unbinds.
//! - **Platform file.** `mac.json` on a Mac or iOS, `windows.json` elsewhere (no Android build).
//! - **Modifier names** are the five modifier keys (`ctrl shift alt win cmd`) and the gamepad buttons; a plain key as
//!   a modifier is "Invalid modifier". The declared set is the union across the files read so far.
//! - **Gamepad.** An entry that names a gamepad button is skipped (there is no gamepad input); a file's skipped
//!   entries are told in one log line, with their count.
//! - **A bad file or entry** never fails the load: it is logged (the lines [`load`] returns) and skipped. A file is
//!   read up to 1 MiB; one cut inside its JSON is bad. A folder in a file's place is not read; a link is not read
//!   and says so. A file with no `modifiers` list is logged, and its entries still merge ([`Keymap::parse`], which is
//!   strict, refuses it).
//! - The app asks by action ID ([`Keymap::consume_press`], [`pressed`]): the global actions at the root after the
//!   screens (`app/keys.rs`), the editing and zoom actions in the editor grids (`ui/edit_keys.rs`), `leaveFolder` and
//!   `startSearch` in the browsers (`ui/search_bar.rs`), `open` on a focused control (`ui/focus.rs`). The
//!   held-modifier ones (`fineTweak`, `editorSelect`) are asked with [`held`]. The rest are parsed and kept.
//! - `win` never fires: egui does not report it.
//! - A binding of a modifier alone (`"ctrl+"`, the held `fineTweak`) is kept but is not a key press, so
//!   [`Keymap::consume_press`] never fires it.
//! - The primary modifier is egui's [`Modifiers::COMMAND`], so Windows' Ctrl and the Mac's Cmd work on whatever the
//!   host reports. `cmd` is always that key. `ctrl` is too, unless the first file read (the built-in map) declares
//!   `cmd`, as the mac map does: there `ctrl` is the physical Ctrl.
//! - Like egui's `consume_key`, a binding ignores extra Shift and Alt: ask for the most specific action first.
//! - **Web.** A key some action binds is kept from the browser ([`Keymap::binds`], `lib.rs`), but Ctrl+C, V and X;
//!   while a text field has the keyboard, only a bound Ctrl/Cmd key is, and not Ctrl+A or Z either. The browser keeps
//!   a few anyway: Chrome's Ctrl+Shift+T (reopen tab) never reaches `showTempo`. Ctrl+Shift+− arrives as
//!   `_`, which egui does not map, so `fineDecrement` and `zoomOutVertical` cannot be typed there.

use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;

use eframe::egui::{Context, Event, InputState, Key, Modifiers, os::OperatingSystem};
use serde_json::Value;

const WINDOWS: &str = include_str!("../assets/content/keymaps/windows.json");
const MAC: &str = include_str!("../assets/content/keymaps/mac.json");

/// How much of each file is read.
const MAX_BYTES: u64 = 1 << 20;

/// The map [`install`] made at start, with whether it is the Mac's.
static INSTALLED: OnceLock<(bool, Keymap)> = OnceLock::new();

/// The map for `os`: the merged one [`install`] made, when it is for the same kind of platform; else the built-in
/// one: Mac and iOS use `mac.json`, everything else `windows.json`.
pub fn for_os(os: OperatingSystem) -> &'static Keymap {
    #[cfg(test)]
    if let Some(map) = TEST_MAP.get() {
        return map;
    }
    static WINDOWS_MAP: OnceLock<Keymap> = OnceLock::new();
    static MAC_MAP: OnceLock<Keymap> = OnceLock::new();
    if let Some((mac, map)) = INSTALLED.get()
        && *mac == os.is_mac()
    {
        return map;
    }
    if os.is_mac() {
        MAC_MAP.get_or_init(|| Keymap::parse(MAC).expect("mac.json"))
    } else {
        WINDOWS_MAP.get_or_init(|| Keymap::parse(WINDOWS).expect("windows.json"))
    }
}

#[cfg(test)]
thread_local! {
    /// A test's own table, which [`for_os`] returns on the test's thread ([`tests::use_map`]).
    static TEST_MAP: std::cell::Cell<Option<&'static Keymap>> = const { std::cell::Cell::new(None) };
}

/// Merges the files ([`load`]) for the platform this is built for, prints what it logged, and makes the result what
/// [`for_os`] returns. Once, at start: later calls change nothing.
pub fn install(shortcuts: Option<&Path>, user: Option<&Path>) {
    if INSTALLED.get().is_some() {
        return;
    }
    let os = if cfg!(any(target_os = "macos", target_os = "ios")) {
        OperatingSystem::Mac
    } else {
        OperatingSystem::Windows
    };
    let (map, log) = load(os, shortcuts, user);
    for line in log {
        eprintln!("keymap: {line}");
    }
    let _ = INSTALLED.set((os.is_mac(), map));
}

/// The merged table for `os`: the built-in map, then `shortcuts` (the content folder's `Keyboard Shortcuts`)
/// `/<platform>.json` and `/gamepad.json`, then `<user>/keymap.json`. A file that is missing or a folder is skipped; a
/// link is skipped with a "link not read" line; one that is bad is skipped with a line in the log, and so is each bad
/// entry (the entries that name a gamepad button share one line per file).
pub fn load(
    os: OperatingSystem,
    shortcuts: Option<&Path>,
    user: Option<&Path>,
) -> (Keymap, Vec<String>) {
    let (platform, built_in) = if os.is_mac() {
        ("mac.json", MAC)
    } else {
        ("windows.json", WINDOWS)
    };
    let mut merger = Merger::default();
    merger.file("built-in", built_in.as_bytes());
    let files = [
        (shortcuts, platform),
        (shortcuts, "gamepad.json"),
        (user, "keymap.json"),
    ];
    for (dir, name) in files {
        let Some(path) = dir.map(|dir| dir.join(name)) else {
            continue;
        };
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() => {}
            Ok(m) if m.is_symlink() => {
                merger.log.push(format!("{name}: link not read"));
                continue;
            }
            _ => continue,
        }
        let mut bytes = Vec::new();
        let read = std::fs::File::open(&path)
            .and_then(|file| file.take(MAX_BYTES).read_to_end(&mut bytes));
        match read {
            Ok(_) => merger.file(name, &bytes),
            Err(e) => merger.log.push(format!("{name}: {e}")),
        }
    }
    (merger.map, merger.log)
}

/// The modifier tokens a file may declare.
const MODIFIERS: [&str; 5] = ["ctrl", "shift", "alt", "win", "cmd"];

/// The gamepad buttons, after `gamepad_`.
const GAMEPAD: [&str; 20] = [
    "a", "b", "x", "y", "up", "down", "left", "right", "lb", "rb", "lt", "rt", "lsb", "rsb", "lsx",
    "lsy", "rsx", "rsy", "menu", "options",
];

/// A keymap key token as egui's key (the Android token list): letters, digits, `- = [ ] ; ' \ , . /`, the
/// ISO key's `` ` ``, `escape backspace tab enter space delete up down left right home end` and `f1` … `f10`.
fn key_token(token: &str) -> Option<Key> {
    const NAMED: [&str; 12] = [
        "space",
        "tab",
        "enter",
        "escape",
        "backspace",
        "delete",
        "left",
        "right",
        "up",
        "down",
        "home",
        "end",
    ];
    let f = token.strip_prefix('f').and_then(|n| n.parse::<u8>().ok());
    let name = match token.as_bytes() {
        _ if NAMED.contains(&token) => token[..1].to_ascii_uppercase() + &token[1..],
        // `f01` parses too, and egui knows no `F01`.
        _ if f.is_some_and(|n| (1..=10).contains(&n)) => token.to_ascii_uppercase(),
        [c] if c.is_ascii_lowercase() || c.is_ascii_digit() || br"-=[];'\,./`".contains(c) => {
            token.to_ascii_uppercase()
        }
        _ => return None,
    };
    Key::from_name(&name)
}

#[derive(Clone, Debug, PartialEq)]
struct Binding {
    action: String,
    mods: Modifiers,
    /// `None`: the modifier alone.
    key: Option<Key>,
    win: bool,
    /// The combination as a tooltip shows it ([`Keymap::shortcut`]).
    label: String,
}

/// What a combination is, for the rules that remove: its modifiers, whether `win` is held, its key.
type Combo = (Modifiers, bool, Option<Key>);

impl Binding {
    fn combo(&self) -> Combo {
        (self.mods, self.win, self.key)
    }

    /// It fires on key `at` ([`key_at`]) pressed with `mods`: extra Shift and Alt are ignored, as egui's
    /// `consume_key` does. A held-modifier binding and one with `win` never do.
    fn matches(&self, at: Key, mods: Modifiers) -> bool {
        !self.win && self.key == Some(at) && mods.matches_logically(self.mods)
    }
}

#[derive(Debug, Default)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Keymap {
    /// A map from the text of one keymap file. `key` is `+`-joined tokens: modifiers the file declares, in any order,
    /// then one key token, or an empty one (`"ctrl+"`) for the modifier alone. Strict: anything [`load`] would log
    /// is an error here, which names it.
    pub fn parse(json: &str) -> Result<Self, String> {
        let mut merger = Merger::default();
        merger.file("keymap", json.as_bytes());
        match merger.log.into_iter().next() {
            Some(error) => Err(error),
            None => Ok(merger.map),
        }
    }

    /// Whether a key of `action` is held down (a held `Preview`). (egui keeps the held keys by their logical key.)
    pub fn down(&self, input: &InputState, action: &str) -> bool {
        (self.bindings.iter())
            .filter(|b| b.action == action && !b.win)
            .any(|b| b.key.is_some_and(|key| input.key_down(key)))
    }

    /// Whether `action`'s key was pressed this frame, consuming every press of its bindings ([`Binding::matches`]: by
    /// key position, extra Shift and Alt ignored). An auto-repeat (a press of a key already held) is consumed without
    /// firing.
    pub fn consume_press(&self, input: &mut InputState, action: &str) -> bool {
        let mut fired = false;
        input.events.retain(|e| match *e {
            Event::Key {
                key,
                physical_key,
                pressed: true,
                repeat,
                modifiers,
            } if (self.bindings.iter())
                .any(|b| b.action == action && b.matches(key_at(key, physical_key), modifiers)) =>
            {
                fired |= !repeat;
                false
            }
            _ => true,
        });
        fired
    }

    /// Whether `action` is a held-modifier binding (`"ctrl+"`: `fineTweak`, `editorSelect`) whose modifiers `mods`
    /// hold.
    pub fn held(&self, mods: Modifiers, action: &str) -> bool {
        let on = |b: &Binding| {
            b.action == action
                && b.key.is_none()
                && !b.win
                && b.mods != Modifiers::NONE
                && mods.contains(b.mods)
        };
        self.bindings.iter().any(on)
    }

    /// Whether some action binds key `at` (a key position: [`key_at`]) pressed alone, which takes it from the
    /// typing-keyboard notes (notes are not keymap actions).
    pub fn binds_plain(&self, at: Key) -> bool {
        (self.bindings.iter()).any(|b| b.matches(at, Modifiers::NONE))
    }

    /// The combination of `action`'s first key binding as a tooltip shows it: the file's modifier tokens in the order
    /// its `modifiers` list declares them, then the key token, joined by `+` and upper-cased (`CTRL+K`).
    pub fn shortcut(&self, action: &str) -> Option<&str> {
        (self.bindings.iter())
            .find(|b| b.action == action && b.key.is_some())
            .map(|b| b.label.as_str())
    }

    /// Whether some action binds key `at` ([`key_at`]) pressed with `mods` ([`Binding::matches`]): the web build keeps
    /// the browser's own action from it (`lib.rs`). Not Ctrl+C, V or X: the browser's clipboard events need theirs.
    pub fn binds(&self, at: Key, mods: Modifiers) -> bool {
        let clipboard = mods.command && matches!(at, Key::C | Key::V | Key::X);
        !clipboard && (self.bindings.iter()).any(|b| b.matches(at, mods))
    }
}

/// The key a key event names for the keymap: its physical key, else (a platform that reports none) its logical one.
/// The keymap tokens are hardware keys with US legends, so Shift+`=` is still `=`, and on AZERTY the key labelled A
/// is `q`. The web reports no physical key, and Shift+`=` as a `+`: that is the `=` key.
pub fn key_at(key: Key, physical_key: Option<Key>) -> Key {
    physical_key.unwrap_or(match key {
        Key::Plus => Key::Equals,
        key => key,
    })
}

/// [`Keymap::consume_press`] on `ctx`'s input with the map for its platform ([`for_os`]).
pub fn pressed(ctx: &Context, action: &str) -> bool {
    let keys = for_os(ctx.os());
    ctx.input_mut(|i| keys.consume_press(i, action))
}

/// [`Keymap::held`] for the modifiers held now, with the map for `ctx`'s platform.
pub fn held(ctx: &Context, action: &str) -> bool {
    let keys = for_os(ctx.os());
    ctx.input(|i| keys.held(i.modifiers, action))
}

/// A table being built from files, in order.
#[derive(Default)]
struct Merger {
    map: Keymap,
    /// The modifiers declared by the files so far.
    declared: Vec<String>,
    /// Whether the first file declares `cmd`: `ctrl` is then the physical Ctrl.
    cmd_primary: Option<bool>,
    /// What was skipped, and why.
    log: Vec<String>,
}

impl Merger {
    /// Merges one file's `bytes`; `name` heads each log line.
    fn file(&mut self, name: &str, bytes: &[u8]) {
        let from = self.log.len();
        self.entries(bytes);
        for line in &mut self.log[from..] {
            *line = format!("{name}: {line}");
        }
    }

    fn entries(&mut self, bytes: &[u8]) {
        let object = match serde_json::from_slice(bytes) {
            Ok(Value::Object(object)) => object,
            Ok(_) => return self.log.push("not a JSON object".into()),
            Err(e) => return self.log.push(e.to_string()),
        };
        let modifiers = object.get("modifiers").and_then(Value::as_array);
        if modifiers.is_none() {
            self.log.push("no `modifiers` list".into());
        }
        for m in modifiers.into_iter().flatten() {
            match m.as_str() {
                Some(m) if MODIFIERS.contains(&m) || gamepad(m) => {
                    if !self.declared.iter().any(|d| d == m) {
                        self.declared.push(m.into());
                    }
                }
                _ => self.log.push(format!("Invalid modifier {m}")),
            }
        }
        self.cmd_primary
            .get_or_insert_with(|| self.declared.iter().any(|m| m == "cmd"));
        let Some(actions) = object.get("actions").and_then(Value::as_array) else {
            return self.log.push("no `actions` list".into());
        };
        // The entries that name a gamepad button: counted, and told in one line (a gamepad file is mostly these).
        let (mut gamepad_entries, mut first_gamepad) = (0, "");
        for (i, entry) in actions.iter().enumerate() {
            let text = |field| entry.get(field).and_then(Value::as_str);
            let (Some(key), Some(action)) = (text("key"), text("action")) else {
                self.log
                    .push(format!("entry {i} needs a string `key` and `action`"));
                continue;
            };
            if key.split('+').any(gamepad) {
                gamepad_entries += 1;
                if first_gamepad.is_empty() {
                    first_gamepad = key;
                }
                continue;
            }
            match self.combo(key) {
                Ok(combo) => self.apply(combo, action, self.label(key)),
                Err(e) => self.log.push(e),
            }
        }
        if gamepad_entries > 0 {
            self.log.push(format!(
                "entries with gamepad buttons skipped (no gamepad input): {gamepad_entries}, first `{first_gamepad}`"
            ));
        }
    }

    /// The combination a `key` names.
    fn combo(&self, key: &str) -> Result<Combo, String> {
        let mut tokens: Vec<&str> = key.split('+').collect();
        let last = tokens.pop().unwrap_or_default();
        let (mut mods, mut win) = (Modifiers::NONE, false);
        for t in &tokens {
            if !self.declared.iter().any(|m| m == t) {
                return Err(format!("`{key}`: unknown modifier `{t}`"));
            }
            match *t {
                "shift" => mods |= Modifiers::SHIFT,
                "alt" => mods |= Modifiers::ALT,
                "win" => win = true,
                "cmd" => mods |= Modifiers::COMMAND,
                "ctrl" if self.cmd_primary == Some(true) => mods |= Modifiers::CTRL,
                _ => mods |= Modifiers::COMMAND,
            }
        }
        let key = if last.is_empty() && !tokens.is_empty() {
            None
        } else {
            let known = key_token(last);
            Some(known.ok_or_else(|| format!("`{key}`: unknown key token `{last}`"))?)
        };
        Ok((mods, win, key))
    }

    /// A valid `key` as [`Keymap::shortcut`] shows it.
    fn label(&self, key: &str) -> String {
        let mut tokens: Vec<&str> = key.split('+').collect();
        let last = tokens.pop().unwrap_or_default();
        tokens.sort_by_key(|t| self.declared.iter().position(|m| m == t));
        tokens.push(last);
        tokens.join("+").to_uppercase()
    }

    /// `""` removes every binding of the combination, `-name` that one action from it, any other action is appended.
    fn apply(&mut self, combo @ (mods, win, key): Combo, action: &str, label: String) {
        let bindings = &mut self.map.bindings;
        match action.strip_prefix('-') {
            _ if action.is_empty() => bindings.retain(|b| b.combo() != combo),
            Some(name) => bindings.retain(|b| b.combo() != combo || b.action != name),
            None => bindings.push(Binding {
                action: action.into(),
                mods,
                key,
                win,
                label,
            }),
        }
    }
}

/// `name` is a gamepad button, `gamepad_a` ... `gamepad_options`.
fn gamepad(name: &str) -> bool {
    name.strip_prefix("gamepad_")
        .is_some_and(|button| GAMEPAD.contains(&button))
}

#[cfg(test)]
pub(crate) mod tests {
    use eframe::egui::{Context, Event, RawInput};

    use std::path::PathBuf;

    use super::*;
    use crate::project_files::content::tests::{Temp, put};

    fn windows() -> &'static Keymap {
        for_os(OperatingSystem::Windows)
    }

    /// Makes `map` what [`for_os`] returns on this thread, as a user's merged table would be, until the guard drops.
    /// libtest runs each test on its own thread, so the map is that test's alone.
    #[must_use]
    pub(crate) fn use_map(map: Keymap) -> impl Drop {
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                TEST_MAP.set(None);
            }
        }
        TEST_MAP.set(Some(Box::leak(Box::new(map))));
        Guard
    }

    fn mac() -> &'static Keymap {
        for_os(OperatingSystem::Mac)
    }

    /// Whether `action` fires on one press of `key` with `modifiers`, in a context that reports `os`.
    pub(crate) fn fires(
        map: &Keymap,
        os: OperatingSystem,
        action: &str,
        key: Key,
        modifiers: Modifiers,
    ) -> bool {
        let ctx = Context::default();
        ctx.set_os(os);
        let input = RawInput {
            events: vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        };
        let mut fired = false;
        let _ = ctx.run(input, |ctx| {
            fired = ctx.input_mut(|i| map.consume_press(i, action))
        });
        fired
    }

    fn only(json_key: &str, action: &str) -> Result<Keymap, String> {
        Keymap::parse(&format!(
            r#"{{"modifiers":["ctrl","shift","alt","win"],"actions":[{{"key":"{json_key}","action":"{action}"}}]}}"#
        ))
    }

    #[test]
    fn both_maps_parse() {
        for (name, json) in [("windows", WINDOWS), ("mac", MAC)] {
            let map = Keymap::parse(json).unwrap_or_else(|e| panic!("{name}.json: {e}"));
            assert!(map.bindings.len() > 30, "{name}.json");
        }
    }

    #[test]
    fn os_picks_the_map() {
        use OperatingSystem::*;
        assert!(std::ptr::eq(for_os(Mac), for_os(IOS)));
        for os in [Windows, Nix, Android, Unknown] {
            assert!(std::ptr::eq(for_os(os), windows()));
        }
        assert!(!std::ptr::eq(mac(), windows()));
        // Only the mac map binds Cmd+, to the settings, and it reads Cmd where the Windows map reads Ctrl.
        let cmd = Modifiers::MAC_CMD | Modifiers::COMMAND;
        let ctrl = Modifiers::CTRL | Modifiers::COMMAND;
        assert!(fires(mac(), Mac, "showMainSettings", Key::Comma, cmd));
        assert!(!fires(
            windows(),
            Windows,
            "showMainSettings",
            Key::Comma,
            ctrl
        ));
        assert!(fires(mac(), Mac, "saveProject", Key::S, cmd));
        assert!(fires(windows(), Windows, "saveProject", Key::S, ctrl));
        // The Mac's Ctrl alone is not Cmd.
        assert!(!fires(mac(), Mac, "saveProject", Key::S, Modifiers::CTRL));
    }

    /// The keymap's action IDs, expanded (`note_{-4..-1,1..4}{,#,b}`, `chord_1..8`, ...).
    fn action_ids() -> Vec<String> {
        let mut ids: Vec<String> = "play rewind armRecord saveProject undo redo copy cut paste delete selectAll \
            startSearch fineTweak editorSelect showMainBrowser showMainSettings showRack showMainMixer showMiniMixer \
            showTempo showTransport showHints showGraphEditor showContextMenu showRackModuleInternalParams \
            newChannel newRackModule replaceRackModule enableRackModule disableRackModule maximizeRackModule \
            minimizeRackModule browsePresets toggleArpeggiator arpGateAdd arpGateSub arpModeX arpModeY arpRateX \
            arpRateY toggleBeatSnap setBeatSnapMode triplets toggleKeyboardPanel resizeKeyboardPanel kbdSettings \
            enterKbdFocus kbdFocusPlay kbdFocusRecord kbdFocusRewind permTransposeUp permTransposeDown \
            tempTransposeUp tempTransposeDown pitchBend modWheelX modWheelY velocity chordGlissandoUp \
            chordGlissandoDown repeatRateX repeatRateY joyXnotes_1 joyXnotes_2 joyYnotes_1 joyYnotes_2 clearNotes \
            focusUp focusDown focusLeft focusRight focusNext focusPrev focusHorizontal focusVertical focusEditor \
            focusEditorTimeline scrollHorizontal scrollVertical pianoScrollHorizontal pianoZoom zoomInHorizontal \
            zoomInVertical zoomOutHorizontal zoomOutVertical marqueeHorizontal marqueeVertical startMarquee nextTab \
            prevTab increment decrement fineIncrement fineDecrement open close selectFile leaveFolder backspace \
            blinkCursor newSongInBrowser newSongFromTemplateInBrowser saveSongInBrowser quickSaveSongInBrowser \
            prevBar nextBar lineStart lineEnd wordLeft wordRight selectLeft selectRight selectToLineStart \
            selectToLineEnd selectWordLeft selectWordRight"
            .split_whitespace()
            .map(String::from)
            .collect();
        for n in (-4..=-1).chain(1..=4) {
            for suffix in ["", "#", "b"] {
                ids.push(format!("note_{n}{suffix}"));
            }
        }
        for n in 1..=8 {
            ids.extend([
                format!("chord_{n}"),
                format!("drumpad_{n}"),
                format!("repeatPad_{n}"),
            ]);
        }
        ids
    }

    #[test]
    fn every_action_is_a_known_id() {
        let ids = action_ids();
        for (name, json) in [("windows", WINDOWS), ("mac", MAC)] {
            for b in &Keymap::parse(json).unwrap().bindings {
                assert!(
                    ids.contains(&b.action),
                    "{name}.json: unknown action `{}`",
                    b.action
                );
            }
        }
    }

    #[test]
    fn there_are_no_duplicate_key_action_pairs() {
        for (name, json) in [("windows", WINDOWS), ("mac", MAC)] {
            let bindings = Keymap::parse(json).unwrap().bindings;
            for (i, b) in bindings.iter().enumerate() {
                assert!(
                    !bindings[..i].contains(b),
                    "{name}.json: `{}` twice",
                    b.action
                );
            }
        }
    }

    #[test]
    fn the_wired_actions_are_in_both_maps() {
        let wired = "saveProject undo redo play rewind armRecord showMainBrowser showMainSettings showRack \
            showMainMixer showTempo showTransport copy cut paste delete selectAll leaveFolder zoomInHorizontal \
            zoomInVertical zoomOutHorizontal zoomOutVertical close open fineTweak editorSelect startSearch";
        for map in [windows(), mac()] {
            for action in wired.split_whitespace() {
                assert!(map.bindings.iter().any(|b| b.action == action), "{action}");
            }
        }
    }

    #[test]
    fn windows_redo_is_alt_ctrl_z_and_shift_ctrl_z() {
        let ctrl = Modifiers::CTRL | Modifiers::COMMAND;
        for extra in [Modifiers::ALT, Modifiers::SHIFT] {
            assert!(fires(
                windows(),
                OperatingSystem::Windows,
                "redo",
                Key::Z,
                ctrl | extra
            ));
        }
        assert!(!fires(
            windows(),
            OperatingSystem::Windows,
            "redo",
            Key::Z,
            ctrl
        ));
        assert!(fires(
            windows(),
            OperatingSystem::Windows,
            "undo",
            Key::Z,
            ctrl
        ));
    }

    #[test]
    fn modifiers_come_in_any_order_and_every_key_token_parses() {
        let a = only("shift+ctrl+z", "redo").unwrap();
        let b = only("ctrl+shift+z", "redo").unwrap();
        assert_eq!(a.bindings, b.bindings);
        assert_eq!(a.bindings[0].mods, Modifiers::COMMAND | Modifiers::SHIFT);
        let tokens =
            "a z m space tab enter escape backspace delete left right up down , - = f6 f8 f9 f10";
        for t in tokens.split(' ') {
            assert!(only(t, "play").is_ok(), "{t}");
        }
        assert_eq!(only("f10", "play").unwrap().bindings[0].key, Some(Key::F10));
        assert_eq!(
            only("left", "play").unwrap().bindings[0].key,
            Some(Key::ArrowLeft)
        );
        assert_eq!(
            only("=", "play").unwrap().bindings[0].key,
            Some(Key::Equals)
        );
    }

    #[test]
    fn every_android_key_token_parses_to_its_key() {
        // The Android key tokens: letters, digits, the US punctuation keys, the ISO key, the named keys and f1 … f10.
        let keys = [
            ("2", Key::Num2),
            ("0", Key::Num0),
            (";", Key::Semicolon),
            (".", Key::Period),
            ("/", Key::Slash),
            ("[", Key::OpenBracket),
            ("]", Key::CloseBracket),
            ("'", Key::Quote),
            ("\\\\", Key::Backslash),
            (",", Key::Comma),
            ("`", Key::Backtick),
            ("f1", Key::F1),
            ("f7", Key::F7),
            ("escape", Key::Escape),
            ("delete", Key::Delete),
            ("home", Key::Home),
            ("end", Key::End),
            ("[", Key::OpenBracket),
        ];
        for (token, key) in keys {
            let map = only(&format!("ctrl+{token}"), "play").unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(map.bindings[0].key, Some(key), "{token}");
        }
        for bad in ["f0", "f11", "f01", "F1", "A", "+", "ab", "numpad1", "pgup"] {
            assert!(only(&format!("ctrl+{bad}"), "play").is_err(), "{bad}");
        }
    }

    #[test]
    fn a_users_digit_and_punctuation_bindings_fire() {
        let user = file(
            "",
            &[entry("2", "play"), entry("ctrl+;", "showTempo")].join(","),
        );
        let (tmp, _) = folders("km-digits", &[], Some(&user));
        let (map, log) = load(OperatingSystem::Windows, None, Some(&tmp.0));
        assert!(log.is_empty(), "{log:?}");
        let os = OperatingSystem::Windows;
        assert!(fires(&map, os, "play", Key::Num2, Modifiers::NONE));
        assert!(fires(
            &map,
            os,
            "showTempo",
            Key::Semicolon,
            Modifiers::COMMAND
        ));
        // A plain key binding does not fire with Ctrl held.
        assert!(!fires(&map, os, "play", Key::Num2, Modifiers::COMMAND));
    }

    /// Frames on one context, one per event (pressed and never released): whether `consume_press` fired `action` in
    /// each, and the events each left in the input.
    fn presses(map: &Keymap, action: &str, events: Vec<Event>) -> Vec<(bool, usize)> {
        let ctx = Context::default();
        let mut out = Vec::new();
        for event in events {
            let input = RawInput {
                events: vec![event],
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                let fired = ctx.input_mut(|i| map.consume_press(i, action));
                out.push((fired, ctx.input(|i| i.events.len())));
            });
        }
        out
    }

    #[test]
    fn a_press_matches_by_its_physical_key_too_and_a_repeat_is_spent_without_firing() {
        let key = |key, physical_key| Event::Key {
            key,
            physical_key,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND | Modifiers::SHIFT,
        };
        // Shift+Ctrl+= reports `+` as its logical key on a US layout; the key is still `=`.
        let shifted = key(Key::Plus, Some(Key::Equals));
        let action = "zoomInVertical";
        assert_eq!(
            presses(windows(), action, vec![shifted.clone()]),
            [(true, 0)]
        );
        // The web reports no physical key: a logical `+` is that key too.
        assert_eq!(
            presses(windows(), action, vec![key(Key::Plus, None)]),
            [(true, 0)]
        );
        // Held down, the key repeats (egui marks a press of a held key so): the repeats are eaten without firing.
        let held = vec![shifted.clone(), shifted.clone(), shifted];
        assert_eq!(
            presses(windows(), action, held),
            [(true, 0), (false, 0), (false, 0)]
        );
    }

    #[test]
    fn a_key_is_its_position_so_on_azerty_ctrl_a_is_the_key_at_q() {
        // Tokens are scan codes with US legends. AZERTY's A key sits where US-QWERTY has Q.
        let key = |key, physical_key| Event::Key {
            key,
            physical_key,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        };
        let action = "selectAll";
        assert_eq!(
            presses(windows(), action, vec![key(Key::A, Some(Key::Q))]),
            [(false, 1)]
        );
        assert_eq!(
            presses(windows(), action, vec![key(Key::Q, Some(Key::A))]),
            [(true, 0)]
        );
        // Typed notes ask by position too (`app/typed.rs`): Space is bound alone, wherever it is.
        assert!(windows().binds_plain(key_at(Key::A, Some(Key::Space))));
        assert!(!windows().binds_plain(key_at(Key::Space, Some(Key::A))));
    }

    #[test]
    fn on_the_web_a_bound_key_is_kept_from_the_browser_but_ctrl_c_v_and_x() {
        // `lib.rs` stops the browser's own action for these: Ctrl+= zooms the page, Ctrl+F finds, Ctrl+K and F6 go to
        // the address bar. Ctrl+C, V and X keep theirs, or the clipboard events never come.
        let map = windows();
        let ctrl = Modifiers::COMMAND | Modifiers::CTRL;
        for key in [Key::Equals, Key::Minus, Key::F, Key::K, Key::S, Key::Z] {
            assert!(map.binds(key, ctrl), "{key:?}");
        }
        assert!(map.binds(Key::F6, Modifiers::NONE));
        // Shift+Ctrl+= is a `+` with no physical key there: the `=` key.
        assert!(map.binds(key_at(Key::Plus, None), ctrl | Modifiers::SHIFT));
        for key in [Key::C, Key::V, Key::X] {
            assert!(!map.binds(key, ctrl), "{key:?}");
        }
        // Not bound: refresh, Ctrl+R, a plain F.
        assert!(!map.binds(Key::F5, Modifiers::NONE));
        assert!(!map.binds(Key::R, ctrl));
        assert!(!map.binds(Key::F, Modifiers::NONE));
    }

    #[test]
    fn a_modifier_alone_binding_is_held_while_its_modifiers_are() {
        let map = windows();
        for action in ["fineTweak", "editorSelect"] {
            assert!(map.held(Modifiers::COMMAND, action), "{action}");
            assert!(
                map.held(Modifiers::COMMAND | Modifiers::SHIFT, action),
                "{action}"
            );
            assert!(!map.held(Modifiers::SHIFT, action), "{action}");
            assert!(!map.held(Modifiers::NONE, action), "{action}");
        }
        // A key binding is no held one.
        assert!(!map.held(Modifiers::COMMAND, "saveProject"));
        // Unbound by the user: never held.
        let user = file("", &entry("ctrl+", "-fineTweak"));
        let (tmp, _) = folders("km-held", &[], Some(&user));
        let (map, _) = load(OperatingSystem::Windows, None, Some(&tmp.0));
        assert!(!map.held(Modifiers::COMMAND, "fineTweak"));
        assert!(map.held(Modifiers::COMMAND, "editorSelect"));
    }

    #[test]
    fn a_shortcut_shows_its_modifiers_in_the_declared_order() {
        assert_eq!(windows().shortcut("showTransport"), Some("CTRL+K"));
        assert_eq!(windows().shortcut("showTempo"), Some("CTRL+SHIFT+T"));
        assert_eq!(mac().shortcut("showTempo"), Some("SHIFT+CMD+T"));
        assert_eq!(windows().shortcut("showRack"), Some("F6"));
        // The first key binding; a held modifier alone is none.
        assert_eq!(windows().shortcut("redo"), Some("CTRL+ALT+Z"));
        assert_eq!(windows().shortcut("fineTweak"), None);
    }

    #[test]
    fn a_trailing_plus_binds_the_modifier_alone() {
        let map = only("ctrl+", "fineTweak").unwrap();
        assert_eq!(
            map.bindings,
            [Binding {
                action: "fineTweak".into(),
                mods: Modifiers::COMMAND,
                key: None,
                win: false,
                label: "CTRL+".into()
            }]
        );
        // It is a held state, not a press: no key fires it.
        assert!(!fires(
            &map,
            OperatingSystem::Windows,
            "fineTweak",
            Key::A,
            Modifiers::COMMAND
        ));
    }

    #[test]
    fn the_primary_modifier_is_command_and_ctrl_in_the_mac_map_is_ctrl() {
        let json =
            r#"{"modifiers":["ctrl","cmd"],"actions":[{"key":"ctrl+cmd+x","action":"cut"}]}"#;
        let mods = Keymap::parse(json).unwrap().bindings[0].mods;
        assert_eq!(mods, Modifiers::CTRL | Modifiers::COMMAND);
        let json = r#"{"modifiers":["ctrl"],"actions":[{"key":"ctrl+x","action":"cut"}]}"#;
        assert_eq!(
            Keymap::parse(json).unwrap().bindings[0].mods,
            Modifiers::COMMAND
        );
    }

    #[test]
    fn win_never_fires() {
        let map = only("win+x", "cut").unwrap();
        assert!(map.bindings[0].win);
        for mods in [Modifiers::NONE, Modifiers::COMMAND, Modifiers::ALT] {
            assert!(!fires(&map, OperatingSystem::Windows, "cut", Key::X, mods));
        }
    }

    #[test]
    fn parse_errors_name_the_token() {
        assert!(Keymap::parse("{").is_err());
        assert!(Keymap::parse("[]").is_err());
        assert!(Keymap::parse(r#"{"modifiers":["ctrl"]}"#).is_err());
        let unknown_key = only("ctrl+q9", "play").unwrap_err();
        assert!(unknown_key.contains("q9"), "{unknown_key}");
        // The gamepad file's buttons are not keys.
        assert!(only("gamepad_a", "play").unwrap_err().contains("gamepad_a"));
        // A modifier the file does not declare.
        let undeclared =
            Keymap::parse(r#"{"modifiers":["ctrl"],"actions":[{"key":"cmd+z","action":"undo"}]}"#);
        assert!(undeclared.unwrap_err().contains("cmd"));
        let undeclared = Keymap::parse(r#"{"modifiers":["hyper"],"actions":[]}"#);
        assert!(undeclared.unwrap_err().contains("hyper"));
        // No key, and no trailing `+` to say so; and nothing at all.
        assert!(only("ctrl", "play").unwrap_err().contains("ctrl"));
        assert!(only("shift+ctrl", "play").is_err());
        assert!(only("", "play").is_err());
        assert!(only("+", "play").is_err());
        // Upper case is not a token.
        assert!(only("ctrl+Z", "undo").is_err());
    }

    // ---- Merged table ----

    const SHORTCUTS: &str = "Keyboard Shortcuts";

    /// The actions bound to the keymap combination `key`, in table order.
    fn bound(map: &Keymap, key: &str) -> Vec<String> {
        let probe = &only(key, "probe").unwrap().bindings[0];
        let same = |b: &&Binding| (b.mods, b.win, b.key) == (probe.mods, probe.win, probe.key);
        let found = map.bindings.iter().filter(same);
        found.map(|b| b.action.clone()).collect()
    }

    /// A user folder and a `Keyboard Shortcuts` folder in it, holding the files given (name, text).
    fn folders(name: &str, shortcuts: &[(&str, &str)], user: Option<&str>) -> (Temp, PathBuf) {
        let tmp = Temp::new(name);
        let dir = tmp.0.join(SHORTCUTS);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in shortcuts {
            put(&dir.join(file), text.as_bytes());
        }
        if let Some(text) = user {
            put(&tmp.0.join("keymap.json"), text.as_bytes());
        }
        (tmp, dir)
    }

    fn file(modifiers: &str, actions: &str) -> String {
        format!(r#"{{"modifiers":[{modifiers}],"actions":[{actions}]}}"#)
    }

    fn entry(key: &str, action: &str) -> String {
        format!(r#"{{"key":"{key}","action":"{action}"}}"#)
    }

    #[test]
    fn the_files_merge_in_order_built_in_platform_gamepad_user() {
        let platform = file(r#""ctrl""#, &entry("ctrl+q", "fromPlatform"));
        let gamepad = file(
            r#""gamepad_lb","gamepad_rb""#,
            &[
                entry("gamepad_lb+gamepad_a", "play"),
                entry("ctrl+q", "fromGamepad"),
            ]
            .join(","),
        );
        let user = file("", &entry("ctrl+q", "fromUser"));
        let (tmp, dir) = folders(
            "km-order",
            &[("windows.json", &platform), ("gamepad.json", &gamepad)],
            Some(&user),
        );
        let (map, log) = load(OperatingSystem::Windows, Some(&dir), Some(&tmp.0));
        assert_eq!(
            bound(&map, "ctrl+q"),
            ["fromPlatform", "fromGamepad", "fromUser"]
        );
        // The built-in map is the base.
        assert_eq!(bound(&map, "ctrl+z"), ["undo"]);
        // The gamepad button is skipped with a log line, not an error.
        assert_eq!(log.len(), 1, "{log:?}");
        assert!(log[0].contains("gamepad.json") && log[0].contains("gamepad_lb+gamepad_a"));
    }

    #[test]
    fn the_gamepad_entries_of_a_file_are_one_log_line_with_their_count() {
        let entries: Vec<String> = ["gamepad_a", "gamepad_lb+gamepad_x", "ctrl+gamepad_up"]
            .iter()
            .map(|key| entry(key, "play"))
            .chain([entry("ctrl+q", "kept")])
            .collect();
        let gamepad = file(r#""gamepad_lb""#, &entries.join(","));
        let (_tmp, dir) = folders("km-gamepad", &[("gamepad.json", &gamepad)], None);
        let (map, log) = load(OperatingSystem::Windows, Some(&dir), None);
        assert_eq!(bound(&map, "ctrl+q"), ["kept"]);
        assert_eq!(log.len(), 1, "{log:?}");
        assert!(
            log[0].starts_with("gamepad.json:") && log[0].contains(": 3,"),
            "{log:?}"
        );
        assert!(log[0].contains("`gamepad_a`"), "{log:?}");
        // A name that only looks like a button is an unknown key token, a separate line.
        let (_, log) = load_one(&file("", &entry("gamepad_zz", "play")));
        assert!(log.len() == 1 && log[0].contains("gamepad_zz"), "{log:?}");
    }

    #[test]
    fn a_file_without_a_modifiers_list_is_logged_and_still_merges() {
        for bad in [
            r#"{"actions":[{"key":"q","action":"late"}]}"#,
            r#"{"modifiers":"ctrl","actions":[{"key":"q","action":"late"}]}"#,
            r#"{"modifiers":null,"actions":[{"key":"q","action":"late"}]}"#,
        ] {
            let (map, log) = load_one(bad);
            assert_eq!(log, ["one.json: no `modifiers` list"], "{bad}");
            // The entry (a plain key) still binds.
            assert_eq!(map.bindings.len(), 1, "{bad}");
            // Strict: refused.
            let error = Keymap::parse(bad).unwrap_err();
            assert!(error.contains("no `modifiers` list"), "{error}");
        }
        // An empty list is a list.
        assert!(load_one(r#"{"modifiers":[],"actions":[]}"#).1.is_empty());
    }

    #[test]
    fn the_platform_file_is_mac_json_on_a_mac_and_windows_json_elsewhere() {
        let mac = file(r#""cmd""#, &entry("cmd+q", "fromMac"));
        let win = file(r#""ctrl""#, &entry("ctrl+q", "fromWindows"));
        let (_tmp, dir) = folders("km-os", &[("mac.json", &mac), ("windows.json", &win)], None);
        let (map, log) = load(OperatingSystem::Mac, Some(&dir), None);
        assert_eq!(bound_mac(&map, "cmd+q"), ["fromMac"]);
        assert!(log.is_empty(), "{log:?}");
        let (map, _) = load(OperatingSystem::Nix, Some(&dir), None);
        assert_eq!(bound(&map, "ctrl+q"), ["fromWindows"]);
    }

    #[test]
    fn an_empty_action_unbinds_the_combination() {
        let user = file("", &[entry("ctrl+z", ""), entry("ctrl+j", "")].join(","));
        let (tmp, _) = folders("km-unbind", &[], Some(&user));
        let (map, log) = load(OperatingSystem::Windows, None, Some(&tmp.0));
        assert!(bound(&map, "ctrl+z").is_empty());
        // The neighbours stay.
        assert_eq!(bound(&map, "shift+ctrl+z"), ["redo"]);
        assert_eq!(bound(&map, "ctrl+s"), ["saveProject"]);
        // Unbinding what is not bound is no error.
        assert!(log.is_empty(), "{log:?}");
    }

    #[test]
    fn a_minus_name_removes_that_one_action_only() {
        // `saveProject` is also bound elsewhere: `-saveProject` on Ctrl+S leaves that binding.
        let platform = file(
            "",
            &[entry("ctrl+s", "extra"), entry("ctrl+e", "saveProject")].join(","),
        );
        let user = file(
            "",
            &[
                entry("ctrl+s", "-saveProject"),
                entry("ctrl+z", "-notBound"),
            ]
            .join(","),
        );
        let (tmp, dir) = folders("km-minus", &[("windows.json", &platform)], Some(&user));
        let (map, log) = load(OperatingSystem::Windows, Some(&dir), Some(&tmp.0));
        assert_eq!(bound(&map, "ctrl+s"), ["extra"]);
        assert_eq!(bound(&map, "ctrl+e"), ["saveProject"]);
        assert_eq!(bound(&map, "ctrl+z"), ["undo"]);
        assert!(log.is_empty(), "{log:?}");
    }

    #[test]
    fn in_the_mac_table_the_users_primary_modifier_is_cmd_and_ctrl_is_the_physical_ctrl() {
        let user = file("", &entry("cmd+s", ""));
        let (tmp, _) = folders("km-mac-user", &[], Some(&user));
        let (map, log) = load(OperatingSystem::Mac, None, Some(&tmp.0));
        assert!(bound_mac(&map, "cmd+s").is_empty());
        assert!(log.is_empty(), "{log:?}");
        // `ctrl+s` is another key there: it unbinds nothing.
        let user = file("", &entry("ctrl+s", ""));
        let (tmp, _) = folders("km-mac-user2", &[], Some(&user));
        let (map, _) = load(OperatingSystem::Mac, None, Some(&tmp.0));
        assert_eq!(bound_mac(&map, "cmd+s"), ["saveProject"]);
    }

    #[test]
    fn an_action_is_appended_in_file_order_with_duplicates_kept() {
        let user = file(
            "",
            &[
                entry("ctrl+z", "undo"),
                entry("ctrl+z", "extra"),
                entry("ctrl+z", "undo"),
            ]
            .join(","),
        );
        let (tmp, _) = folders("km-append", &[], Some(&user));
        let (map, _) = load(OperatingSystem::Windows, None, Some(&tmp.0));
        assert_eq!(bound(&map, "ctrl+z"), ["undo", "undo", "extra", "undo"]);
    }

    #[test]
    fn the_order_of_modifiers_does_not_matter_when_unbinding() {
        let user = file("", &entry("ctrl+alt+z", ""));
        let (tmp, _) = folders("km-order-mods", &[], Some(&user));
        let (map, _) = load(OperatingSystem::Windows, None, Some(&tmp.0));
        assert!(bound(&map, "alt+ctrl+z").is_empty());
    }

    #[test]
    fn an_invalid_modifier_and_a_bad_entry_are_logged_and_skipped() {
        let user = format!(
            r#"{{"modifiers":["ctrl","hyper",5],"actions":[
                {},
                {{"key":"ctrl+y"}},
                {{"action":"noKey"}},
                {{"key":1,"action":"x"}},
                {{"key":"ctrl+e","action":2}},
                "text",
                {},
                {}
            ]}}"#,
            entry("hyper+x", "usesHyper"),
            entry("ctrl+q9", "badKey"),
            entry("ctrl+e", "fine"),
        );
        let (tmp, _) = folders("km-bad", &[], Some(&user));
        let (map, log) = load(OperatingSystem::Windows, None, Some(&tmp.0));
        assert_eq!(bound(&map, "ctrl+e"), ["fine"]);
        for action in ["usesHyper", "badKey", "noKey", "x"] {
            assert!(!map.bindings.iter().any(|b| b.action == action), "{action}");
        }
        let has = |text: &str| log.iter().filter(|l| l.contains(text)).count();
        assert_eq!(has("Invalid modifier"), 2, "{log:?}");
        assert!(has("hyper") >= 2, "{log:?}");
        assert_eq!(has("needs a string"), 5, "{log:?}");
        assert_eq!(has("q9"), 1, "{log:?}");
        assert_eq!(log.len(), 2 + 5 + 2, "{log:?}");
        // The valid part of the file and the rest of the table survive.
        assert_eq!(bound(&map, "ctrl+z"), ["undo"]);
    }

    #[test]
    fn modifiers_are_the_union_across_the_files() {
        // Windows' own file declares no `cmd`; the platform file does, and the user file may then use it. On a
        // table where Ctrl is the primary modifier `cmd` is the same key.
        let platform = file(r#""cmd""#, "");
        let user = file(
            "",
            &[entry("cmd+k", "extra"), entry("ctrl+z", "")].join(","),
        );
        let (tmp, dir) = folders("km-union", &[("windows.json", &platform)], Some(&user));
        let (map, log) = load(OperatingSystem::Windows, Some(&dir), Some(&tmp.0));
        assert_eq!(bound(&map, "ctrl+k"), ["showTransport", "extra"]);
        // Only the first file (the built-in map) says whether `ctrl` is the physical Ctrl: the platform file's `cmd`
        // does not change what `ctrl` means, so the user's `ctrl+z` still unbinds undo.
        assert!(bound(&map, "ctrl+z").is_empty());
        assert!(log.is_empty(), "{log:?}");
        // Without that declaration `cmd` is an unknown modifier (and the `ctrl+z` entry still works).
        let (tmp, _) = folders("km-union2", &[], Some(&user));
        let (map, log) = load(OperatingSystem::Windows, None, Some(&tmp.0));
        assert_eq!(bound(&map, "ctrl+k"), ["showTransport"]);
        assert!(bound(&map, "ctrl+z").is_empty());
        assert!(log.iter().any(|l| l.contains("cmd+k")), "{log:?}");
    }

    #[test]
    fn a_file_is_read_up_to_one_mib() {
        let open = r#"{"modifiers":[],"actions":["#;
        let item = entry("ctrl+q", "late");
        let close = "]}";
        let at = |pad: usize| format!("{open}{}{item}{close}", " ".repeat(pad));
        // The entry ends inside the first MiB: read.
        let fits = at(MAX_FILE - open.len() - item.len() - close.len());
        assert_eq!(fits.len(), MAX_FILE);
        // One byte more and the file is cut inside the entry: the file is bad JSON, skipped and logged.
        let cut = at(MAX_FILE - open.len() - item.len() - close.len() + 1);
        // A long tail of blanks after the document is simply not read.
        let tail = format!("{open}{item}{close}{}", " ".repeat(2 * MAX_FILE));
        for (name, text, bound_to_q) in [
            ("fits", &fits, true),
            ("cut", &cut, false),
            ("tail", &tail, true),
        ] {
            let (tmp, _) = folders(&format!("km-cap-{name}"), &[], Some(text));
            let (map, log) = load(OperatingSystem::Windows, None, Some(&tmp.0));
            assert_eq!(bound(&map, "ctrl+q") == ["late"], bound_to_q, "{name}");
            assert_eq!(log.is_empty(), bound_to_q, "{name}: {log:?}");
        }
    }

    #[test]
    fn no_content_folder_leaves_the_built_in_map_alone() {
        for os in [OperatingSystem::Windows, OperatingSystem::Mac] {
            let built_in = &for_os(os).bindings;
            let (map, log) = load(os, None, None);
            assert_eq!(&map.bindings, built_in);
            assert!(log.is_empty(), "{log:?}");
            // Folders with no files in them are no different.
            let (tmp, dir) = folders("km-none", &[], None);
            let (map, log) = load(os, Some(&dir), Some(&tmp.0));
            assert_eq!(&map.bindings, built_in);
            assert!(log.is_empty(), "{log:?}");
        }
    }

    #[test]
    fn a_broken_file_is_skipped_whole_and_the_others_still_load() {
        let user = file("", &entry("ctrl+q", "fromUser"));
        let (tmp, dir) = folders(
            "km-broken",
            &[("windows.json", "{"), ("gamepad.json", "[1]")],
            Some(&user),
        );
        let (map, log) = load(OperatingSystem::Windows, Some(&dir), Some(&tmp.0));
        assert_eq!(bound(&map, "ctrl+q"), ["fromUser"]);
        assert_eq!(bound(&map, "ctrl+z"), ["undo"]);
        assert_eq!(log.len(), 2, "{log:?}");
        // A file with no `actions` list adds nothing but is not an error for the table.
        let (_, log) = load_one(r#"{"modifiers":["ctrl"]}"#);
        assert_eq!(log.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_or_a_folder_is_not_read() {
        let outside = Temp::new("km-link-target");
        put(
            &outside.0.join("real.json"),
            file("", &entry("ctrl+q", "viaLink")).as_bytes(),
        );
        let tmp = Temp::new("km-link");
        std::os::unix::fs::symlink(outside.0.join("real.json"), tmp.0.join("keymap.json")).unwrap();
        let (map, log) = load(OperatingSystem::Windows, None, Some(&tmp.0));
        assert!(bound(&map, "ctrl+q").is_empty());
        assert_eq!(log, ["keymap.json: link not read"]);
        // A link in the shortcuts folder too, with the file's own name.
        let shortcuts = Temp::new("km-link-shortcuts");
        std::os::unix::fs::symlink(
            outside.0.join("real.json"),
            shortcuts.0.join("gamepad.json"),
        )
        .unwrap();
        let (_, log) = load(OperatingSystem::Windows, Some(&shortcuts.0), None);
        assert_eq!(log, ["gamepad.json: link not read"]);
        let tmp = Temp::new("km-dir");
        std::fs::create_dir(tmp.0.join("keymap.json")).unwrap();
        let (map, log) = load(OperatingSystem::Windows, None, Some(&tmp.0));
        assert_eq!(&map.bindings, &for_os(OperatingSystem::Windows).bindings);
        assert!(log.is_empty(), "{log:?}");
    }

    /// [`bound`] for a Mac table: `cmd` is declared there.
    fn bound_mac(map: &Keymap, key: &str) -> Vec<String> {
        let json = file(r#""ctrl","shift","alt","win","cmd""#, &entry(key, "probe"));
        let probe = &Keymap::parse(&json).unwrap().bindings[0];
        let found = map
            .bindings
            .iter()
            .filter(|b| (b.mods, b.win, b.key) == (probe.mods, probe.win, probe.key));
        found.map(|b| b.action.clone()).collect()
    }

    /// One file merged onto nothing, with its log.
    fn load_one(json: &str) -> (Keymap, Vec<String>) {
        let mut merger = Merger::default();
        merger.file("one.json", json.as_bytes());
        (merger.map, merger.log)
    }

    const MAX_FILE: usize = 1 << 20;

    /// `consume_press` fires on a fresh press only; a press of a key already held (egui marks it a repeat) is spent
    /// without firing.
    #[test]
    fn consume_press_spends_an_auto_repeat_without_firing() {
        let ctx = Context::default();
        let tab = Event::Key {
            key: Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let run = || {
            let input = RawInput {
                events: vec![tab.clone()],
                ..Default::default()
            };
            let (mut fired, mut left) = (false, true);
            let _ = ctx.run(input, |ctx| {
                ctx.input_mut(|i| {
                    fired = windows().consume_press(i, "focusNext");
                    left = !i.events.is_empty();
                })
            });
            (fired, left)
        };
        assert_eq!(run(), (true, false));
        // Still held: the next press is a repeat.
        assert_eq!(run(), (false, false));
    }
}

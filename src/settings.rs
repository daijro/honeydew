//! The app settings (the main menu's SETTINGS) and where they are kept.
//!
//! [`Settings`] holds every SETTINGS row that has a value of its own. The rows the app has no use for yet (MIDI driver,
//! Bluetooth, nearby sharing, cloud backup) are shown disabled in their default state and have no field. The settings
//! are stored as one JSON string under [`STORAGE_KEY`] in an [`eframe::Storage`], and the JSON field names are the
//! preference keys.
//!
//! eframe has a native storage only with its `persistence` feature, which brings in two more crates (`ron`, `home`).
//! [`storage`] is the dependency-free stand-in: a JSON file natively, the browser's local storage on the
//! web. The app owns it, loads the settings at startup and saves them when the settings tab reports a change, the same
//! way on both platforms.

use serde::{Deserialize, Serialize};

use crate::metrics::{DEFAULT_UI_SCALE, UI_SCALE_FACTORS, UI_SCALE_RANGE};
use crate::midi::device::ANY;
use crate::theme::builtin_themes;
use crate::ui::automation::value::ValueSnap;
use crate::ui::keyboard_settings::KeyboardMode;
use crate::ui::main_menu::Sort;
use crate::ui::modules_browser::ModuleOrdering;
use crate::ui::save_dialog::ExportFormat;
use crate::ui::soundfont_picker::PatchOrdering;
use serde_json::{Map, Value};

/// The [`eframe::Storage`] key the settings are stored under, prefixed because the web's local storage is shared by
/// everything on the page's origin.
pub const STORAGE_KEY: &str = "honeydew.settings";

/// The keys the app stored its values under before its rename, each with the key that replaces it.
const LEGACY_KEYS: [(&str, &str); 2] = [
    ("studio.settings", STORAGE_KEY),
    (
        "studio.routingPrefs",
        crate::recording::routing::STORAGE_KEY,
    ),
];

/// `key` is one the app stored a value under before its rename ([`LEGACY_KEYS`]): a config folder of the former name
/// holding it is the app's ([`crate::is_legacy_app_dir`]).
pub fn is_legacy_key(key: &str) -> bool {
    LEGACY_KEYS.iter().any(|&(old, _)| old == key)
}

/// Copies each value stored under a key of the app's former name ([`LEGACY_KEYS`]) to its new key, where that holds
/// nothing yet. The old key keeps its value.
pub fn migrate_legacy_keys(storage: &mut dyn eframe::Storage) {
    for (old, new) in LEGACY_KEYS {
        if storage.get_string(new).is_none()
            && let Some(value) = storage.get_string(old)
        {
            storage.set_string(new, value);
        }
    }
}

/// The `Sound mode:` items in menu order, with their stored ids.
///
/// FL Studio Mobile exposes buffer profiles on mobile builds, but its desktop builds expose the
/// driver modes `Standard` and `Multitrack` instead.
#[cfg(desktop)]
pub const SOUND_MODES: [(u8, &str); 2] = [(0, "STANDARD"), (1, "MULTITRACK")];
#[cfg(not(desktop))]
pub const SOUND_MODES: [(u8, &str); 1] = [(3, "NATIVE")];

/// Buffer-size slider positions. Zero keeps the audio device's native/default buffer;
/// positive values are targets that the backend clamps to the device's supported range.
pub const BUFFER_SIZES: [u32; 7] = [0, 64, 128, 256, 512, 1024, 2048];
pub const MCP_DEFAULT_PORT: u16 = 57831;

#[cfg(desktop)]
pub fn mcp_token_path() -> Option<std::path::PathBuf> {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join("Library/Application Support"))?;
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(std::path::PathBuf::from)?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let app_dir = "honeydew";
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let app_dir = "Honeydew";
    Some(base.join(app_dir).join("control-token"))
}

#[cfg(desktop)]
pub fn load_mcp_token() -> Option<String> {
    let token = std::fs::read_to_string(mcp_token_path()?)
        .ok()?
        .trim()
        .to_owned();
    (!token.is_empty()).then_some(token)
}

#[cfg(desktop)]
pub fn save_mcp_token(token: &str) -> std::io::Result<()> {
    let path = mcp_token_path()
        .ok_or_else(|| std::io::Error::other("could not determine config directory"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(desktop))]
pub fn load_mcp_token() -> Option<String> {
    None
}

#[cfg(not(desktop))]
pub fn save_mcp_token(_token: &str) -> std::io::Result<()> {
    Err(std::io::Error::other("MCP token storage is desktop-only"))
}

#[cfg(desktop)]
const DEFAULT_SOUND_MODE: u8 = 0;
#[cfg(not(desktop))]
const DEFAULT_SOUND_MODE: u8 = 3;

/// Every SETTINGS value the app keeps. For the defaults see [`Settings::default`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `Input device:` (`MidiDevice`): the menu label of the chosen MIDI input, `<no midi input>`, `Any available` or
    /// a device name (`MidiChoice::label`; read it back with `MidiChoice::from_label`). A name stays when its device
    /// is not connected: the choice reconnects when the device returns.
    #[serde(rename = "MidiDevice")]
    pub midi_device: String,
    /// `Multi core processing`: parallel independent desktop channel racks.
    #[serde(rename = "multithreaded")]
    pub multi_core: bool,
    #[serde(rename = "mcpEnabled")]
    pub mcp_enabled: bool,
    #[serde(rename = "mcpPort")]
    pub mcp_port: u16,
    #[serde(skip)]
    pub mcp_token: String,
    /// The selected output buffer target: an index into [`BUFFER_SIZES`]. Zero keeps the device default.
    #[serde(rename = "lowlatency")]
    pub low_latency: u8,
    /// `Enable background audio (iOS Only)`. Stored for the iOS build; disabled elsewhere.
    #[serde(rename = "bgAudio")]
    pub background_audio: bool,
    /// `Sound mode:`, a stored id from [`SOUND_MODES`], selecting the platform's audio mode.
    #[serde(rename = "soundModeAndr")]
    pub sound_mode: u8,
    /// `Enable audio recording latency compensation`: the toggle as stored, `None` while unset. Its meaning is
    /// inverted: compensation is on exactly when the toggle is OFF, and an unset key shows the toggle ON but
    /// compensates ([`crate::recording::recorder::compensates`]).
    #[serde(rename = "reclatency")]
    pub latency_compensation: Option<bool>,
    /// `Enable multitrack recording`. Stored only.
    #[serde(rename = "multitrackrec")]
    pub multitrack_recording: bool,
    /// `Enable count-in when recording` (default on).
    #[serde(rename = "reccount")]
    pub count_in: bool,
    /// `Metronome when recording` (our own key; default on): REC's take clicks the beats. Off, it records silently; a
    /// count-in still clicks.
    #[serde(rename = "recMetronome")]
    pub rec_metronome: bool,
    /// `Metronome level` in [0, 1], shown as 0–100 %. Stored only.
    #[serde(rename = "mtmlevel")]
    pub metronome_level: f32,
    /// `Theme:`, a built-in theme id.
    pub theme: u32,
    /// `Scaling:`, the UI scale in percent, kept in [`UI_SCALE_RANGE`]. A missing one is the older `uiscale` index's
    /// factor ([`UI_SCALE_FACTORS`]), once.
    #[serde(rename = "uiscalefactor")]
    pub ui_scale: u32,
    /// `Animation` is `Full`: panels, menus, fades and eases animate; off, they jump to their end.
    #[serde(rename = "enableAnimations")]
    pub animations: bool,
    /// `Animation` is not `Minimal`: scrolls coast after a fling.
    #[serde(rename = "enableInertia")]
    pub inertia: bool,
    /// `Enable ghost notes` (read by the piano roll).
    #[serde(rename = "ghostnotes")]
    pub ghost_notes: bool,
    /// `Show key names on keyboard and piano roll`.
    #[serde(rename = "showKeyNames")]
    pub key_names: bool,
    /// `Set middle C to C4` (off: middle C is labelled C5).
    #[serde(rename = "middlec4")]
    pub middle_c_is_c4: bool,
    /// `Show range labels on keyboard`.
    #[serde(rename = "showRangeIndexLabels")]
    pub range_labels: bool,
    /// `Continuous scrolling during playback`.
    #[serde(rename = "continuousScroll")]
    pub continuous_scroll: bool,
    /// `Show hidden tracks` (our own key): the playlist shows the tracks `Hide track` hid, dimmed. Off by default.
    #[serde(rename = "showHiddenTracks")]
    pub show_hidden_tracks: bool,
    /// `Show CPU and RAM meters`.
    #[serde(rename = "showHardwareMeters")]
    pub hardware_meters: bool,
    /// The format `Render` last exported, which PROJECT `Quick Render` renders: an audio format's stored id
    /// ([`ExportFormat::id`]; a hidden setting).
    #[serde(rename = "lastAudioFileFormat")]
    pub last_audio_format: u8,
    /// The keyboard panel's `lockpads`: the drum pads cannot be rearranged. On by default; the keyboard settings
    /// strip's `Lock` toggles it.
    #[serde(rename = "lockpads")]
    pub lock_pads: bool,
    /// The keyboard panel's `showTwoRows`: the piano shows a second row, an octave up, over the main one. Off by
    /// default.
    #[serde(rename = "showTwoRows")]
    pub show_two_rows: bool,
    /// The keyboard panel's `preferredKbdSizeCells`, in U: the height the panel animates to. 5 U by default; turning
    /// two rows on raises it to make room for them. A load without it takes the former `preferredKbdSize` (U too).
    #[serde(rename = "preferredKbdSizeCells")]
    pub preferred_kbd_size: f32,
    /// The keyboard panel's `recentKbdSizeCells`, in U: the last height a hand resize left above the bar alone, which
    /// the tap toggle opens the panel to again. 5 U by default. A load without it takes the former `recentKbdSize`.
    #[serde(rename = "recentKbdSizeCells")]
    pub recent_kbd_size: f32,
    /// The keyboard panel's `showWheels`: the strip's icon 12 shows the pitch and mod wheels left of the keys. Off by
    /// default, and the strip's `Reset` turns it off.
    #[serde(rename = "showWheels")]
    pub show_wheels: bool,
    /// The keyboard panel's `keyboardKind` (global, not the song's): the `Keyboard` choice, 0 `Classic`, 1 `Scale`,
    /// 2 `Chords`, 3 `Strum`.
    #[serde(rename = "keyboardKind")]
    pub keyboard_kind: u8,
    /// The demo-song set last put into the songs folder: a start copies the missing ones while it is below
    /// [`DEMO_SONGS_VERSION`] ([`crate::content::restore_demo_songs`]), and `Restore demo songs` sets it too. A load
    /// without it takes the former `demoSongsRestored` as the current set.
    #[serde(rename = "demoSongsVersion")]
    pub demo_songs_version: i32,
    /// The What's New notes version last shown at a start, "" before any.
    #[serde(rename = "whatsNewSeen")]
    pub whats_new_seen: String,
    /// The SoundFont patch picker's `Sort` (`SoundFontPatchOrdering`, a hidden setting): the id of the
    /// [`PatchOrdering`] its menu last picked, −1 (missing) until then, which is `Default` (file order).
    #[serde(rename = "SoundFontPatchOrdering")]
    pub sound_font_patch_ordering: i32,
    /// The Modules picker's ordering (`ModuleBrowserOrdering`, a hidden setting): the id of the [`ModuleOrdering`] its
    /// `Sort` menu last picked, −1 (missing) until then; one that names no ordering opens the picker in `Category`.
    #[serde(rename = "ModuleBrowserOrdering")]
    pub module_browser_ordering: i32,
    /// The SONGS page's `Sort` (`SongBrowserOrdering`, a hidden setting): a [`Sort`]'s stored id, `A-Z` by default.
    #[serde(rename = "SongBrowserOrdering")]
    pub song_browser_ordering: u8,
    /// The first-import notice's `Import` was chosen (our own key): archives dropped into the Original Content folder
    /// import without asking ([`crate::project_files::ProjectFiles::import_accepted`]).
    #[serde(rename = "importNoticeAccepted")]
    pub import_notice_accepted: bool,
    /// The plugin folders the user added in SETTINGS (our own key), searched besides the standard ones
    /// (`plugins::scan::default_folders`). Desktop only.
    #[serde(rename = "pluginFolders")]
    pub plugin_folders: Vec<String>,
    /// The shipped sound release (`sounds/release.txt`) the desktop's missing-sounds question was last shown for (our
    /// own key), "" before any: it shows again only for a newer one (`app/sounds.rs`).
    #[serde(rename = "soundsNoticeSeen")]
    pub sounds_notice_seen: String,
    /// The automation editor's value snap (our own key): on, and its grid; on with `Rows` by default.
    #[serde(rename = "automationValueSnap")]
    pub automation_value_snap: ValueSnap,
    /// The Save dialog's MIDI `Apply note effects` (our own key; #31): a MIDI export writes the notes as the channels'
    /// note effects play them. On by default.
    #[serde(rename = "midiApplyNoteEffects")]
    pub midi_note_effects: bool,
    /// The piano roll graph lane's value snap, `Vel`'s and `Pan`'s (our own keys; #32): off by default.
    #[serde(rename = "pianoRollVelocitySnap")]
    pub velocity_snap: ValueSnap,
    #[serde(rename = "pianoRollPanSnap")]
    pub pan_snap: ValueSnap,
}

impl Default for Settings {
    /// The fresh-install state: ghost notes, range labels, the count-in, the recording's metronome and the CPU and RAM
    /// meters on, the latency toggle unset (shown ON, compensating), the other toggles off, `NATIVE`, `Classic Gray`
    /// (the fallback theme), `100 %` and `Animation` `Full`. The metronome level is half way; the drum pads are locked.
    fn default() -> Self {
        Self {
            midi_device: ANY.to_owned(),
            multi_core: false,
            mcp_enabled: false,
            mcp_port: MCP_DEFAULT_PORT,
            mcp_token: String::new(),
            low_latency: 0,
            background_audio: false,
            sound_mode: DEFAULT_SOUND_MODE,
            latency_compensation: None,
            multitrack_recording: false,
            count_in: true,
            rec_metronome: true,
            metronome_level: 0.5,
            theme: 0,
            ui_scale: DEFAULT_UI_SCALE,
            animations: true,
            inertia: true,
            ghost_notes: true,
            key_names: false,
            middle_c_is_c4: false,
            range_labels: true,
            continuous_scroll: false,
            show_hidden_tracks: false,
            hardware_meters: true,
            lock_pads: true,
            last_audio_format: ExportFormat::Wav.id(),
            show_two_rows: false,
            preferred_kbd_size: crate::ui::keyboard_settings::PANEL_H,
            recent_kbd_size: crate::ui::keyboard_settings::PANEL_H,
            show_wheels: false,
            keyboard_kind: 0,
            demo_songs_version: 0,
            whats_new_seen: String::new(),
            sound_font_patch_ordering: -1,
            module_browser_ordering: -1,
            song_browser_ordering: Sort::AZ.id(),
            import_notice_accepted: false,
            plugin_folders: Vec::new(),
            sounds_notice_seen: String::new(),
            automation_value_snap: ValueSnap::default(),
            midi_note_effects: true,
            velocity_snap: ValueSnap::OFF,
            pan_snap: ValueSnap::OFF,
        }
    }
}

impl Settings {
    /// The settings stored in `storage`. Anything missing or unreadable is the default, field by field, and so is an
    /// out-of-range value: an unknown theme is Classic Gray, an unknown scale 100 %.
    pub fn load(storage: &dyn eframe::Storage) -> Self {
        let mut fields: Map<String, Value> = storage
            .get_string(STORAGE_KEY)
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        // `uiscalefactor` replaces the `uiscale` index: a missing (or zero) one takes the index's factor, an unknown
        // index 100 %.
        if fields
            .get("uiscalefactor")
            .is_none_or(|v| v.as_u64().is_none_or(|p| p < 1))
        {
            let index = fields.get("uiscale").and_then(Value::as_u64);
            let factor = index.and_then(|i| UI_SCALE_FACTORS.get(i as usize).copied());
            if let Some(factor) = factor {
                fields.insert(
                    "uiscalefactor".into(),
                    ((factor * 100.0).round() as u32).into(),
                );
            } else {
                fields.remove("uiscalefactor");
            }
        }
        migrate_keys(&mut fields);
        // Older versions stored this setting as a boolean toggle. Preserve an enabled toggle as the
        // smallest target when loading those settings into the buffer-size slider.
        if let Some(enabled) = fields.get("lowlatency").and_then(Value::as_bool) {
            fields.insert("lowlatency".into(), u64::from(enabled).into());
        }
        // Field by field: one unreadable value (wrong type, out of its type's range) is dropped on its own instead of
        // failing the whole object.
        fields.retain(|k, v| {
            let one = Map::from_iter([(k.clone(), v.clone())]);
            serde_json::from_value::<Self>(Value::Object(one)).is_ok()
        });
        let mut s: Self = serde_json::from_value(Value::Object(fields)).unwrap_or_default();
        let d = Self::default();
        if s.theme as usize >= builtin_themes().len() {
            s.theme = d.theme;
        }
        s.ui_scale = s
            .ui_scale
            .clamp(*UI_SCALE_RANGE.start(), *UI_SCALE_RANGE.end());
        s.low_latency = s.low_latency.min((BUFFER_SIZES.len() - 1) as u8);
        if !SOUND_MODES.iter().any(|&(id, _)| id == s.sound_mode) {
            s.sound_mode = d.sound_mode;
        }
        if !(0.0..=1.0).contains(&s.metronome_level) {
            s.metronome_level = d.metronome_level;
        }
        // A a height must be a size, which the panel then keeps within its range.
        for (h, default) in [
            (&mut s.preferred_kbd_size, d.preferred_kbd_size),
            (&mut s.recent_kbd_size, d.recent_kbd_size),
        ] {
            if !(h.is_finite() && *h >= 0.0) {
                *h = default;
            }
        }
        if !ExportFormat::from_id(s.last_audio_format).is_some_and(ExportFormat::is_audio) {
            s.last_audio_format = d.last_audio_format;
        }
        if usize::from(s.keyboard_kind) >= KeyboardMode::ALL.len() {
            s.keyboard_kind = d.keyboard_kind;
        }
        if Sort::from_id(s.song_browser_ordering).is_none() {
            s.song_browser_ordering = d.song_browser_ordering;
        }
        // No device is called "": it would never connect.
        if s.midi_device.is_empty() {
            s.midi_device = d.midi_device;
        }
        s
    }

    /// The `Keyboard` choice the `keyboardKind` setting names (an unknown one is `Classic`).
    pub fn keyboard_kind(&self) -> KeyboardMode {
        (KeyboardMode::ALL.get(usize::from(self.keyboard_kind)))
            .copied()
            .unwrap_or_default()
    }

    /// The Modules picker's ordering at an opening: the one `ModuleBrowserOrdering` names, else `Category`.
    pub fn module_ordering(&self) -> ModuleOrdering {
        ModuleOrdering::from_id(self.module_browser_ordering).unwrap_or_default()
    }

    /// The SoundFont patch picker's ordering at an opening: the one `SoundFontPatchOrdering` names, else `Default`.
    pub fn patch_ordering(&self) -> PatchOrdering {
        PatchOrdering::from_id(self.sound_font_patch_ordering)
    }

    /// The SONGS page's order the `SongBrowserOrdering` setting names (an unknown one is `A-Z`).
    pub fn song_ordering(&self) -> Sort {
        Sort::from_id(self.song_browser_ordering).unwrap_or_default()
    }

    /// Writes the settings to `storage`. The caller flushes it.
    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        let json = serde_json::to_string(self).expect("plain data always serialises");
        storage.set_string(STORAGE_KEY, json);
    }
}

/// The demo-song set this build puts into the songs folder ([`Settings::demo_songs_version`]).
pub const DEMO_SONGS_VERSION: i32 = 2;

/// Moves the values of the preference keys that newer ones replaced to those, where they hold nothing yet: the panel
/// heights move to the `…Cells` keys (ours were in U already), and `demoSongsRestored` becomes the current demo-song
/// set. The former keys are read no more and go with the next save.
fn migrate_keys(fields: &mut Map<String, Value>) {
    for (old, new) in [
        ("preferredKbdSize", "preferredKbdSizeCells"),
        ("recentKbdSize", "recentKbdSizeCells"),
    ] {
        if let Some(value) = fields.remove(old)
            && !fields.contains_key(new)
        {
            fields.insert(new.into(), value);
        }
    }
    if fields.remove("demoSongsRestored") == Some(Value::Bool(true))
        && !fields.contains_key("demoSongsVersion")
    {
        fields.insert("demoSongsVersion".into(), DEMO_SONGS_VERSION.into());
    }
}

/// The key a settings store answers (with anything) while its last flush failed: what it could not write waits in
/// memory, and [`eframe::Storage::flush`] returns nothing to say so.
pub const UNFLUSHED_KEY: &str = "honeydew.unflushed";

/// `storage`'s last flush failed ([`UNFLUSHED_KEY`]): the app tries again.
pub fn unflushed(storage: &dyn eframe::Storage) -> bool {
    storage.get_string(UNFLUSHED_KEY).is_some()
}

/// The app's settings store: a JSON file natively, local storage on the web. `None` when there is nowhere to keep
/// a file (no `XDG_CONFIG_HOME`, `APPDATA` or `HOME`); the settings then last for the session. What the app kept under
/// its former name carries over: the folder ([`open_storage`]) and the keys ([`migrate_legacy_keys`]).
pub fn storage() -> Option<Box<dyn eframe::Storage>> {
    #[cfg(target_arch = "wasm32")]
    {
        let mut storage = WebStorage::default();
        migrate_legacy_keys(&mut storage);
        Some(Box::new(storage))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        #[cfg(target_os = "android")]
        let dir = crate::android::data_dir();
        #[cfg(not(target_os = "android"))]
        let dir = config_dir(|key| std::env::var_os(key));
        dir.map(|dir| open_storage(&dir))
    }
}

/// The settings file in the app's folder in the config folder `dir`, which the folder of the app's former name becomes
/// the first time ([`crate::migrate_legacy_dir_once`]), with the values of the former keys carried over.
#[cfg(not(target_arch = "wasm32"))]
fn open_storage(dir: &std::path::Path) -> Box<dyn eframe::Storage> {
    let app = crate::migrate_legacy_dir_once(
        &dir.join(crate::LEGACY_APP_NAME),
        &dir.join(crate::APP_NAME),
    );
    let mut storage = FileStorage::open(app.join("storage.json"));
    migrate_legacy_keys(&mut storage);
    Box::new(storage)
}

/// The user's config directory, from the environment variables `var` reads: `XDG_CONFIG_HOME`, else `APPDATA`, else
/// `HOME/.config`. Unset, empty and relative values are skipped (the XDG spec ignores a relative `XDG_CONFIG_HOME`).
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn config_dir(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<std::path::PathBuf> {
    // ponytail: ~/.config on macOS too, rather than ~/Library/Application Support.
    let path = |key| {
        var(key)
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    path("XDG_CONFIG_HOME")
        .or_else(|| path("APPDATA"))
        .or_else(|| Some(path("HOME")?.join(".config")))
}

/// The browser's local storage, through eframe's helpers. Writes stay in memory until [`flush`](Self::flush), as in
/// [`FileStorage`], so a slider drag doesn't write local storage every frame.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct WebStorage(std::collections::BTreeMap<String, String>);

#[cfg(target_arch = "wasm32")]
impl eframe::Storage for WebStorage {
    /// [`UNFLUSHED_KEY`] while a value waits for the next flush.
    fn get_string(&self, key: &str) -> Option<String> {
        if key == UNFLUSHED_KEY {
            return (!self.0.is_empty()).then(String::new);
        }
        let unflushed = self.0.get(key).cloned();
        unflushed.or_else(|| eframe::web::storage::local_storage_get(key))
    }

    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }

    /// Stores each value with `setItem`, whose failure (the quota full, storage disabled) is logged and keeps the
    /// value for the next flush ([`write_unsaved`]).
    fn flush(&mut self) {
        let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten());
        write_unsaved(&mut self.0, |key, value| {
            let stored = match &storage {
                Some(storage) => storage.set_item(key, value),
                None => Err("no local storage".into()),
            };
            stored
                .map_err(|err| web_sys::console::error_2(&format!("{key} not saved:").into(), &err))
                .is_ok()
        });
    }
}

/// Writes each of `unsaved` with `set`, which says whether it was stored. What was not (the web's quota full) stays
/// unsaved: it reads back from memory, and the next flush tries it again.
#[cfg(any(test, target_arch = "wasm32"))]
fn write_unsaved(
    unsaved: &mut std::collections::BTreeMap<String, String>,
    mut set: impl FnMut(&str, &str) -> bool,
) {
    unsaved.retain(|key, value| !set(key, value));
}

/// A storage kept as one JSON object of strings in a file. Writes stay in memory until [`flush`](Self::flush).
#[cfg(not(target_arch = "wasm32"))]
pub struct FileStorage {
    path: std::path::PathBuf,
    map: std::collections::BTreeMap<String, String>,
    /// The last flush failed ([`UNFLUSHED_KEY`]).
    unflushed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileStorage {
    /// The storage in the file at `path`. A missing or damaged file reads as empty; the first flush creates or
    /// replaces it.
    pub fn open(path: std::path::PathBuf) -> Self {
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self {
            path,
            map,
            unflushed: false,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl eframe::Storage for FileStorage {
    /// [`UNFLUSHED_KEY`] while the last flush failed.
    fn get_string(&self, key: &str) -> Option<String> {
        if key == UNFLUSHED_KEY {
            return self.unflushed.then(String::new);
        }
        self.map.get(key).cloned()
    }

    fn set_string(&mut self, key: &str, value: String) {
        self.map.insert(key.to_owned(), value);
    }

    /// Writes the file whole or not at all ([`crate::project_files::write_atomic`]: a new temp file, synced, renamed
    /// over the old one). A failure is logged, and the values stay in memory for the next flush ([`UNFLUSHED_KEY`]).
    fn flush(&mut self) {
        let json = serde_json::to_string_pretty(&self.map).expect("strings always serialise");
        let result = (self.path.parent())
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| crate::project_files::write_atomic(&self.path, json.as_bytes()));
        self.unflushed = result.is_err();
        if let Err(err) = result {
            eprintln!("settings not saved to {}: {err}", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// An in-memory storage.
    #[derive(Default)]
    struct Mem(HashMap<String, String>);

    impl eframe::Storage for Mem {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }

        fn flush(&mut self) {}
    }

    fn stored(json: &str) -> Mem {
        let mut mem = Mem::default();
        mem.0.insert(STORAGE_KEY.into(), json.into());
        mem
    }

    /// Every field away from its default.
    fn changed() -> Settings {
        let d = Settings::default();
        Settings {
            midi_device: "Keys 49".into(),
            multi_core: !d.multi_core,
            mcp_enabled: !d.mcp_enabled,
            mcp_port: 57999,
            mcp_token: String::new(),
            low_latency: if d.low_latency == 0 { 1 } else { 0 },
            background_audio: !d.background_audio,
            sound_mode: SOUND_MODES
                .get(1)
                .map(|&(id, _)| id)
                .unwrap_or(d.sound_mode),
            latency_compensation: Some(false),
            multitrack_recording: !d.multitrack_recording,
            count_in: !d.count_in,
            rec_metronome: !d.rec_metronome,
            metronome_level: 0.25,
            theme: 6,
            ui_scale: 145,
            animations: false,
            inertia: false,
            ghost_notes: !d.ghost_notes,
            key_names: !d.key_names,
            middle_c_is_c4: !d.middle_c_is_c4,
            range_labels: !d.range_labels,
            continuous_scroll: !d.continuous_scroll,
            show_hidden_tracks: !d.show_hidden_tracks,
            hardware_meters: !d.hardware_meters,
            lock_pads: !d.lock_pads,
            last_audio_format: 5,
            show_two_rows: !d.show_two_rows,
            preferred_kbd_size: 7.5,
            recent_kbd_size: 9.25,
            show_wheels: !d.show_wheels,
            keyboard_kind: 3,
            demo_songs_version: 2,
            whats_new_seen: "5.0".into(),
            sound_font_patch_ordering: 1,
            module_browser_ordering: 1,
            song_browser_ordering: 2,
            import_notice_accepted: !d.import_notice_accepted,
            plugin_folders: vec!["/plugins".into()],
            sounds_notice_seen: "0.1.4".into(),
            automation_value_snap: ValueSnap {
                on: false,
                grid: crate::ui::automation::value::ValueGrid::Percent(10),
            },
            midi_note_effects: !d.midi_note_effects,
            velocity_snap: ValueSnap {
                on: true,
                grid: crate::ui::automation::value::ValueGrid::Steps,
            },
            pan_snap: ValueSnap {
                on: true,
                grid: crate::ui::automation::value::ValueGrid::Percent(25),
            },
        }
    }

    #[test]
    fn defaults_are_the_device_state() {
        let d = Settings::default();
        // The rows that default on, and the count-in.
        assert!(
            d.ghost_notes && d.range_labels && d.hardware_meters && d.count_in && d.rec_metronome
        );
        // The latency toggle is unset: shown ON, and it compensates.
        assert_eq!(d.latency_compensation, None);
        // The other toggles are off.
        assert!(!d.multi_core && d.low_latency == 0 && !d.background_audio);
        assert!(!d.multitrack_recording);
        assert!(!d.key_names && !d.middle_c_is_c4 && !d.continuous_scroll);
        // `NATIVE`, `Classic Gray`, `100%`.
        assert_eq!(d.sound_mode, DEFAULT_SOUND_MODE);
        assert_eq!(d.theme, 0);
        assert_eq!(d.ui_scale, 100);
        // `Animation`: `Full`.
        assert!(d.animations && d.inertia);
        assert_eq!(d.metronome_level, 0.5);
        // The last export format: WAV.
        assert_eq!(d.last_audio_format, 2);
        // One keyboard row, a 5 U panel.
        assert!(!d.show_two_rows);
        assert_eq!(d.preferred_kbd_size, 5.0);
        // Both panel sizes default to 5 U.
        assert_eq!(d.recent_kbd_size, 5.0);
        assert!(!d.show_wheels);
        assert_eq!(
            (d.keyboard_kind, d.keyboard_kind()),
            (0, KeyboardMode::Classic)
        );
        // The drum pads are locked.
        assert!(d.lock_pads);
        // The demo songs are not out until the first run puts them there.
        assert_eq!(d.demo_songs_version, 0);
        assert_eq!(
            (d.whats_new_seen.as_str(), d.sound_font_patch_ordering),
            ("", -1)
        );
        // The first import asks first.
        assert!(!d.import_notice_accepted);
    }

    #[test]
    fn settings_survive_a_save_and_load() {
        let mut mem = Mem::default();
        let s = changed();
        assert_ne!(s, Settings::default());
        s.save(&mut mem);
        assert_eq!(Settings::load(&mem), s);
    }

    #[test]
    fn nothing_stored_or_garbage_loads_the_defaults() {
        assert_eq!(Settings::load(&Mem::default()), Settings::default());
        assert_eq!(Settings::load(&stored("not json")), Settings::default());
        assert_eq!(Settings::load(&stored("[1, 2]")), Settings::default());
    }

    #[test]
    fn the_latency_toggle_stores_true_or_false_and_reads_unset_as_none() {
        // The stored TRUE/FALSE, and no key at all for a fresh install.
        for (json, want) in [
            (r#"{"reclatency": true}"#, Some(true)),
            (r#"{"reclatency": false}"#, Some(false)),
            ("{}", None),
        ] {
            let s = Settings::load(&stored(json));
            assert_eq!(s.latency_compensation, want, "{json}");
            let mut mem = Mem::default();
            s.save(&mut mem);
            assert_eq!(Settings::load(&mem).latency_compensation, want, "{json}");
        }
    }

    #[test]
    fn missing_fields_are_defaults_and_unknown_ones_are_ignored() {
        let s = Settings::load(&stored(r#"{"middlec4": true, "someday": 7}"#));
        assert_eq!(
            s,
            Settings {
                middle_c_is_c4: true,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn one_unreadable_field_does_not_take_the_others_with_it() {
        let s = Settings::load(&stored(
            r#"{"soundModeAndr":300,"uiscale":-1,"ghostnotes":false}"#,
        ));
        assert_eq!(
            s,
            Settings {
                ghost_notes: false,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn out_of_range_values_load_as_defaults() {
        let s = Settings::load(&stored(
            r#"{"theme": 9, "uiscale": 9, "soundModeAndr": 4, "mtmlevel": 1.5, "ghostnotes": false}"#,
        ));
        assert_eq!(
            s,
            Settings {
                ghost_notes: false,
                ..Settings::default()
            }
        );
        // A keyboard height that is no size: 5 U.
        for json in [
            r#"{"preferredKbdSizeCells": -1, "recentKbdSizeCells": -2}"#,
            r#"{"preferredKbdSizeCells": "6", "recentKbdSizeCells": "7"}"#,
        ] {
            let s = Settings::load(&stored(json));
            assert_eq!(
                (s.preferred_kbd_size, s.recent_kbd_size),
                (5.0, 5.0),
                "{json}"
            );
        }
        // A kind past `Strum` is `Classic`; `Strum` stays.
        for (json, want) in [(r#"{"keyboardKind": 4}"#, 0), (r#"{"keyboardKind": 3}"#, 3)] {
            assert_eq!(Settings::load(&stored(json)).keyboard_kind, want, "{json}");
        }
        // The last valid ones stay.
        let s = Settings::load(&stored(
            r#"{"theme": 8, "uiscale": 8, "soundModeAndr": 0, "mtmlevel": 1.0}"#,
        ));
        assert_eq!(
            (s.theme, s.ui_scale, s.sound_mode, s.metronome_level),
            (8, 60, 0, 1.0)
        );
        // `uiscalefactor` is kept in 60–200 %; a missing or zero one takes the old `uiscale` index's factor.
        for (json, want) in [
            (r#"{"uiscalefactor": 250}"#, 200),
            (r#"{"uiscalefactor": 30, "uiscale": 2}"#, 60),
            (r#"{"uiscalefactor": 0, "uiscale": 2}"#, 125),
            (r#"{"uiscalefactor": 145, "uiscale": 2}"#, 145),
            (r#"{"uiscale": 6}"#, 175),
        ] {
            assert_eq!(Settings::load(&stored(json)).ui_scale, want, "{json}");
        }
        // The last export format is an audio format's id: FLM (0), MIDI (1) and no format (4) are WAV; FLAC (5) stays.
        for (id, want) in [(0, 2), (1, 2), (4, 2), (9, 2), (5, 5), (3, 3)] {
            let s = Settings::load(&stored(&format!(r#"{{"lastAudioFileFormat": {id}}}"#)));
            assert_eq!(s.last_audio_format, want, "{id}");
        }
    }

    #[test]
    fn the_keys_newer_ones_replaced_are_read_once_into_those() {
        // The panel heights (ours in U already) keep their values; a set demo flag is the current set. A save writes
        // only the new keys.
        let mut mem = stored(
            r#"{"preferredKbdSize": 7.5, "recentKbdSize": 6.25, "demoSongsRestored": true}"#,
        );
        let s = Settings::load(&mem);
        assert_eq!((s.preferred_kbd_size, s.recent_kbd_size), (7.5, 6.25));
        assert_eq!(s.demo_songs_version, DEMO_SONGS_VERSION);
        s.save(&mut mem);
        for old in ["KbdSize\"", "demoSongsRestored"] {
            assert!(!mem.0[STORAGE_KEY].contains(old), "{old}");
        }
        // A new key wins over its old one.
        let s = Settings::load(&stored(
            r#"{"preferredKbdSize": 7.5, "preferredKbdSizeCells": 6.0, "demoSongsRestored": false}"#,
        ));
        assert_eq!((s.preferred_kbd_size, s.demo_songs_version), (6.0, 0));
    }

    #[test]
    fn the_json_uses_the_original_preference_keys() {
        let mut mem = Mem::default();
        Settings::default().save(&mut mem);
        let json: serde_json::Value = serde_json::from_str(&mem.0[STORAGE_KEY]).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "MidiDevice",
                "ModuleBrowserOrdering",
                "SongBrowserOrdering",
                "SoundFontPatchOrdering",
                "automationValueSnap",
                "bgAudio",
                "continuousScroll",
                "demoSongsVersion",
                "enableAnimations",
                "enableInertia",
                "ghostnotes",
                "importNoticeAccepted",
                "keyboardKind",
                "lastAudioFileFormat",
                "lockpads",
                "lowlatency",
                "mcpEnabled",
                "mcpPort",
                "middlec4",
                "midiApplyNoteEffects",
                "mtmlevel",
                "multithreaded",
                "multitrackrec",
                "pianoRollPanSnap",
                "pianoRollVelocitySnap",
                "pluginFolders",
                "preferredKbdSizeCells",
                "recMetronome",
                "reccount",
                "recentKbdSizeCells",
                "reclatency",
                "showHardwareMeters",
                "showHiddenTracks",
                "showKeyNames",
                "showRangeIndexLabels",
                "showTwoRows",
                "showWheels",
                "soundModeAndr",
                "soundsNoticeSeen",
                "theme",
                "uiscalefactor",
                "whatsNewSeen"
            ]
        );
    }

    #[test]
    fn the_modules_sort_is_stored_under_module_browser_ordering_by_its_id() {
        use crate::ui::modules_browser::ModuleOrdering::{self, AZ, Category, TopLevel};
        // Missing is −1, which names no ordering, so the Modules picker opens in Category; a stored id (A-Z 1, Top
        // Level 2, Category 3) opens in that ordering.
        let d = Settings::default();
        assert_eq!(ModuleOrdering::from_id(d.module_browser_ordering), None);
        assert_eq!(d.module_ordering(), Category);
        let mut mem = Mem::default();
        Settings {
            module_browser_ordering: AZ.id(),
            ..d
        }
        .save(&mut mem);
        let json: serde_json::Value = serde_json::from_str(&mem.0[STORAGE_KEY]).unwrap();
        assert_eq!(json["ModuleBrowserOrdering"], 1);
        assert_eq!(Settings::load(&mem).module_ordering(), AZ);
        for (id, want) in [
            (-1, Category),
            (0, Category),
            (2, TopLevel),
            (3, Category),
            (7, Category),
        ] {
            let s = Settings::load(&stored(&format!(r#"{{"ModuleBrowserOrdering": {id}}}"#)));
            assert_eq!(s.module_ordering(), want, "{id}");
        }
    }

    #[test]
    fn the_soundfont_patch_sort_is_stored_under_sound_font_patch_ordering_by_its_id() {
        // Missing is −1, `Default` (file order); `Default` is stored as 0, `A-Z` as 1; any other id is `Default`.
        let d = Settings::default();
        assert_eq!(d.patch_ordering(), PatchOrdering::Default);
        let mut mem = Mem::default();
        Settings {
            sound_font_patch_ordering: PatchOrdering::AZ.id(),
            ..d
        }
        .save(&mut mem);
        let json: serde_json::Value = serde_json::from_str(&mem.0[STORAGE_KEY]).unwrap();
        assert_eq!(json["SoundFontPatchOrdering"], 1);
        assert_eq!(Settings::load(&mem).patch_ordering(), PatchOrdering::AZ);
        for (id, want) in [
            (-1, PatchOrdering::Default),
            (0, PatchOrdering::Default),
            (7, PatchOrdering::Default),
        ] {
            let s = Settings::load(&stored(&format!(r#"{{"SoundFontPatchOrdering": {id}}}"#)));
            assert_eq!(s.patch_ordering(), want, "{id}");
        }
        assert_eq!(PatchOrdering::Default.id(), 0);
    }

    #[test]
    fn the_songs_sort_is_stored_under_song_browser_ordering_a_z_by_default() {
        use crate::ui::main_menu::Sort;
        // Stored by its place in `Sort`'s menu.
        assert_eq!(Settings::default().song_ordering(), Sort::AZ);
        let mut mem = Mem::default();
        Settings {
            song_browser_ordering: Sort::Recent.id(),
            ..Settings::default()
        }
        .save(&mut mem);
        let json: serde_json::Value = serde_json::from_str(&mem.0[STORAGE_KEY]).unwrap();
        assert_eq!(json["SongBrowserOrdering"], 2);
        assert_eq!(Settings::load(&mem).song_ordering(), Sort::Recent);
        // An ordering past `Recent` is `A-Z`.
        let s = Settings::load(&stored(r#"{"SongBrowserOrdering": 3}"#));
        assert_eq!(s.song_browser_ordering, 0);
    }

    #[test]
    fn the_midi_device_defaults_to_any_and_is_stored_under_midi_device() {
        use crate::midi::device::MidiChoice;
        let d = Settings::default();
        assert_eq!(d.midi_device, "Any available");
        assert_eq!(MidiChoice::from_label(&d.midi_device), MidiChoice::Any);
        let mut mem = Mem::default();
        for label in ["<no midi input>", "Keys 49"] {
            Settings {
                midi_device: label.into(),
                ..Settings::default()
            }
            .save(&mut mem);
            let json: serde_json::Value = serde_json::from_str(&mem.0[STORAGE_KEY]).unwrap();
            assert_eq!(json["MidiDevice"], label);
            assert_eq!(Settings::load(&mem).midi_device, label);
        }
    }

    #[test]
    fn a_stored_device_name_survives_a_load_and_a_save_when_the_device_is_gone() {
        // Nothing here knows which devices exist: the name is kept so that the device reconnects when it returns.
        let mut mem = stored(r#"{"MidiDevice":"Gone Keys 49","ghostnotes":false}"#);
        let s = Settings::load(&mem);
        assert_eq!(s.midi_device, "Gone Keys 49");
        s.save(&mut mem);
        assert_eq!(Settings::load(&mem).midi_device, "Gone Keys 49");
    }

    #[test]
    fn an_empty_or_unreadable_device_name_loads_as_any_available() {
        for json in [
            r#"{"MidiDevice":""}"#,
            r#"{"MidiDevice":5}"#,
            r#"{"MidiDevice":null}"#,
            r#"{}"#,
        ] {
            assert_eq!(Settings::load(&stored(json)), Settings::default(), "{json}");
        }
        // One bad field does not take the device with it.
        let s = Settings::load(&stored(r#"{"MidiDevice":"Keys 49","theme":"x"}"#));
        assert_eq!(s.midi_device, "Keys 49");
    }

    #[test]
    fn values_under_the_former_keys_move_to_the_new_ones_and_stay_under_the_old() {
        use crate::recording::routing;
        use eframe::Storage as _;
        let mut mem = Mem::default();
        mem.set_string("studio.settings", r#"{"MidiDevice":"Keys 49"}"#.into());
        mem.set_string("studio.routingPrefs", "{}".into());
        migrate_legacy_keys(&mut mem);
        assert_eq!(Settings::load(&mem).midi_device, "Keys 49");
        assert_eq!(mem.get_string(routing::STORAGE_KEY).as_deref(), Some("{}"));
        assert!(mem.0.contains_key("studio.settings") && mem.0.contains_key("studio.routingPrefs"));
        // A value under the new key wins, and nothing is made from nothing.
        let mut mem = stored(r#"{"MidiDevice":"New"}"#);
        mem.set_string("studio.settings", r#"{"MidiDevice":"Old"}"#.into());
        migrate_legacy_keys(&mut mem);
        assert_eq!(Settings::load(&mem).midi_device, "New");
        assert_eq!(mem.get_string(routing::STORAGE_KEY), None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_settings_of_the_former_name_open_from_the_renamed_folder() {
        let dir = std::env::temp_dir().join(format!("br-open-storage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let old = dir.join("Studio");
        std::fs::create_dir_all(&old).unwrap();
        let file = serde_json::json!({ "studio.settings": r#"{"MidiDevice":"Keys 49"}"# });
        std::fs::write(old.join("storage.json"), file.to_string()).unwrap();
        let storage = open_storage(&dir);
        assert_eq!(Settings::load(&*storage).midi_device, "Keys 49");
        assert!(dir.join("Honeydew").join("storage.json").is_file());
        assert!(!old.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_config_dir_skips_unset_empty_and_relative_variables() {
        use std::path::PathBuf;
        let dir = |vars: &[(&str, &str)]| {
            let vars: Vec<(String, String)> =
                vars.iter().map(|&(k, v)| (k.into(), v.into())).collect();
            config_dir(|key| vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.into()))
        };
        assert_eq!(dir(&[]), None);
        assert_eq!(
            dir(&[("HOME", "/home/a")]),
            Some(PathBuf::from("/home/a/.config"))
        );
        assert_eq!(
            dir(&[("XDG_CONFIG_HOME", "/cfg"), ("HOME", "/home/a")]),
            Some(PathBuf::from("/cfg"))
        );
        // The XDG spec: a relative XDG_CONFIG_HOME is invalid and ignored.
        for xdg in ["", "relative/cfg"] {
            assert_eq!(
                dir(&[("XDG_CONFIG_HOME", xdg), ("HOME", "/home/a")]),
                Some(PathBuf::from("/home/a/.config")),
                "{xdg:?}"
            );
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_file_storage_keeps_values_across_opens_once_flushed() {
        use eframe::Storage as _;
        let dir =
            std::env::temp_dir().join(format!("honeydew-settings-test-{}", std::process::id()));
        let path = dir.join("nested").join("storage.json");
        let _ = std::fs::remove_dir_all(&dir);

        let mut file = FileStorage::open(path.clone());
        assert_eq!(file.get_string(STORAGE_KEY), None);
        changed().save(&mut file);
        assert_eq!(Settings::load(&file), changed());
        // Not on disk until flushed.
        assert_eq!(
            Settings::load(&FileStorage::open(path.clone())),
            Settings::default()
        );
        file.flush();
        assert_eq!(Settings::load(&FileStorage::open(path.clone())), changed());
        assert!(
            !path.with_file_name(".storage.json.tmp").exists(),
            "the temp file is left behind"
        );

        // A damaged file reads as empty, and the next flush replaces it.
        std::fs::write(&path, "{").unwrap();
        let mut file = FileStorage::open(path.clone());
        assert_eq!(file.get_string(STORAGE_KEY), None);
        file.set_string("k", "v".into());
        file.flush();
        assert_eq!(
            FileStorage::open(path).get_string("k").as_deref(),
            Some("v")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The settings file goes through the shared atomic writer, whose temp file is made new: a link left at a temp name
    /// (by a crash or another program) is never written through.
    #[cfg(unix)]
    #[test]
    fn a_flush_never_writes_through_a_link_at_a_temp_name() {
        use eframe::Storage as _;
        let tmp = crate::project_files::content::tests::Temp::new("fd4-settings-link");
        let path = tmp.0.join("storage.json");
        let victim = tmp.0.join("victim.txt");
        std::fs::write(&victim, b"mine").unwrap();
        for name in ["storage.json.tmp", ".storage.json.tmp"] {
            std::os::unix::fs::symlink(&victim, tmp.0.join(name)).unwrap();
        }
        let mut file = FileStorage::open(path.clone());
        file.set_string("k", "v".into());
        file.flush();
        assert_eq!(std::fs::read(&victim).unwrap(), b"mine");
        assert!(!std::fs::symlink_metadata(&path).unwrap().is_symlink());
        assert_eq!(
            FileStorage::open(path).get_string("k").as_deref(),
            Some("v")
        );
    }

    /// What the web's local storage refuses (its quota full) stays unsaved, to read back and to try again at the next
    /// flush; what it stored is done.
    #[test]
    fn a_value_the_storage_refuses_stays_for_the_next_flush() {
        let mut unsaved: std::collections::BTreeMap<String, String> = [("a", "1"), ("b", "2")]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .into();
        write_unsaved(&mut unsaved, |key, _| key == "a");
        assert_eq!(unsaved, [("b".to_owned(), "2".to_owned())].into());
        write_unsaved(&mut unsaved, |_, _| true);
        assert!(unsaved.is_empty());
    }

    /// A flush that fails says so ([`unflushed`]), for the app to try again, until one works.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_file_storage_says_its_flush_failed_until_one_works() {
        use eframe::Storage as _;
        let tmp = crate::project_files::content::tests::Temp::new("fo5-settings-flush");
        // The settings' folder is a file: nothing can be written there.
        let folder = tmp.0.join("app");
        std::fs::write(&folder, b"").unwrap();
        let mut file = FileStorage::open(folder.join("storage.json"));
        file.set_string("k", "v".into());
        assert!(!unflushed(&file));
        file.flush();
        assert!(unflushed(&file));
        std::fs::remove_file(&folder).unwrap();
        file.flush();
        assert!(!unflushed(&file));
        let stored = FileStorage::open(folder.join("storage.json"));
        assert_eq!(stored.get_string("k").as_deref(), Some("v"));
    }
}

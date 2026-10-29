use std::sync::Arc;

use eframe::egui;
use honeydew_audio::device::{AudioCommand, OutputSnapshot};

use crate::audio_engine::LiveInput;
use crate::audio_link::{AudioLink, AudioSink, Handover, buffer_mode, open_device_with_mode};
use crate::fonts;
use crate::history::{self, History, Step};
use crate::metrics::{FontSize, Metrics};
use crate::project_files::{ProjectFiles, Replace};
use crate::recording::input::{InputChoice, InputLink};
use crate::recording::recorder::{Recorder, Take, compensates, latency, recording_channels};
use crate::recording::routing::{Routing, input_channel};
use crate::sample_cache::{self, SampleCache};
use crate::settings::{self, Settings};
use crate::song::chords::ChordSet;
use crate::song::scales::{CHROMATIC, SCALE_MASKS};
use crate::song::{AutomationTarget, Channel, ChannelKind, LaneKind, Song};
use crate::theme::{Theme, theme_by_id};
use crate::ui::UiCtx;
use crate::ui::add_channel_menu::AddChannelPick;
use crate::ui::chord_keyboard::{ChordInput, Note, TIMING_MS};
use crate::ui::clips::{LiveTake, song_end_beats};
use crate::ui::focus;
use crate::ui::keyboard::KeyInput;
use crate::ui::keyboard_settings::KeyboardMode;
use crate::ui::main_screen::{MainScreen, ScreenActions, ScreenInput, track_rows};
use crate::ui::performance;
use crate::ui::piano_roll::notes::project_scale;
use crate::ui::piano_roll::{PianoRoll, PianoRollAction};
use crate::ui::preview::{self, ClipKey};
use crate::ui::sample_row::{self, RowAction};
use crate::ui::split;
use crate::ui::transport_extras::Load;
use crate::ui::views::main_menu::{QUIT_TAG, quit_question};
use crate::ui::views::{self, AppCtx, Nav, Navigation, Overlay, Slots, SongEdit, View};
use crate::ui::{panel_resize, wheels};

mod audio_edit;
pub(crate) mod auto_record;
pub(crate) mod automation;
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
mod control;
mod demo_restore;
mod export;
mod input;
mod keys;
mod midi;
mod note_record;
mod panel;
mod plugins;
mod tracks;
mod typed;
// A song load on the web waits for its bundled sounds.
mod bundle_wait;
// Live editing in a Playground session (web only).
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) mod live;
// Audio files dropped on the app.
mod file_drop;

/// How long a flash (`Saved!`) shows.
const FLASH_SECS: f64 = 1.5;
/// A gap between user actions this long or longer is not editing time (META @256).
const IDLE_SECS: f64 = 600.0;

/// What an input that fails to open shows.
const MIC_PERMISSION: &str = "Please grant microphone permission to enable audio recording.";
/// The tags of that message and of a recording's file error.
const INPUT_TAG: &str = "audio input";
const RECORD_TAG: &str = "recording";
/// What a song that loads with audio files missing shows, and its tag.
const MISSING_AUDIO: &str = "Some audio files were missing";
const MISSING_TAG: &str = "missing audio";

/// The piano roll's key column velocity: MIDI 100, normalised /127 as `NoteOn` takes it (song notes keep bytes, a/255).
/// The keyboard plays at its fingers' depth instead (`ui::keyboard`).
pub(crate) const KEY_VELOCITY: f32 = 100.0 / 127.0;

/// How long a tap on the keyboard or a drum pad sounds at least, in seconds. The engine drains the commands that
/// reached it before it renders each buffer, so a NoteOff that came with its NoteOn would make a zero-length, silent
/// note.
pub(crate) const TAP_GATE: f64 = 0.05;

/// The web starts the audio on the primary release, native builds on the press. Touch on the web grants the
/// user gesture only on the release, but eframe runs a frame on the press too (egui cannot tell a touch from a
/// mouse there), and a start on that press "succeeds" with a suspended audio context that never resumes.
const START_ON_RELEASE: bool = cfg!(target_arch = "wasm32");

/// This frame has the click that may start or reopen the audio: its release when `on_release`, else its press.
/// One attempt per click.
fn audio_gesture(input: &egui::InputState, on_release: bool) -> bool {
    if on_release {
        input.pointer.primary_released()
    } else {
        input.pointer.primary_pressed()
    }
}

pub struct HoneydewApp {
    /// The SETTINGS values: the theme, UI scale, keyboard labels and meters follow them.
    settings: Settings,
    /// What `storage` holds: the settings as last saved.
    settings_saved: Settings,
    /// Saved settings not yet written to disk: they are, once no pointer button is down.
    settings_unflushed: bool,
    /// After a failed flush: the time (`egui::InputState::time`) to try again at, a second later.
    settings_retry_at: Option<f64>,
    /// Where the settings are kept (`settings::storage`); `None` keeps them for the session only.
    storage: Option<Box<dyn eframe::Storage>>,
    /// The theme `settings.theme` names.
    theme: Theme,
    song: Song,
    /// This frame's `U` in device pixels, which a song new to the app takes its view zooms per U at
    /// ([`Song::zooms_to_units`]).
    u_px: f32,
    main_screen: MainScreen,
    /// The view shown and the overlays open.
    nav: Navigation,
    /// The views' and overlays' own state.
    slots: Slots,
    /// An edit the next frame reports from inside the screens, as a slot would.
    #[cfg(test)]
    test_edit: Option<SongEdit>,
    /// Started by the first click (the web needs a user gesture to start audio), see [`START_ON_RELEASE`].
    audio: AudioLink,
    /// The channel the keyboard plays. The master (0) plays nothing; it is only selected while it is alone.
    selected_channel: usize,
    /// The keyboard notes sounding now, as (channel, key).
    notes: Vec<(usize, u8)>,
    /// The live keys Play's all-notes-off cut while held, as (channel, key): dark until let go
    /// ([`Self::cut_live_keys`]).
    cut: Vec<(usize, u8)>,
    /// By channel, the keys the engine sounds at its input, as of this frame's snapshot. By channel index: after
    /// an edit that moves channels, until the rebuilt engine's first render, a channel may read the keys of the one that
    /// had its index (a frame or two of stale 0.2 lights, which the new engine clears).
    engine_keys: Vec<u128>,
    /// When the gate of a tapped note ends (egui input time, in seconds), while it sounds: see [`TAP_GATE`].
    tap_off: Option<f64>,
    /// `nextBar` (true) or `prevBar` was pressed: the frame seeks to that bar ([`keys::bar_jump`]).
    bar_jump: Option<bool>,
    /// The web page was hidden since the last frame ([`Self::on_hidden`]): the next frame counts as a focus loss.
    hidden: bool,
    /// Debug view of the whole icon set, toggled with F2.
    #[cfg(debug_assertions)]
    show_icon_sheet: bool,
    /// Open / save: the song's file and the requests to the platform's file dialogs.
    files: ProjectFiles,
    /// Undo / redo: the song after each completed edit.
    history: History,
    /// A param edit's undo step is open until no pointer is down.
    param_gesture: bool,
    /// A sample-row slider's ongoing edit waits for its release, or the pointer's lift once its row went.
    row_drag: bool,
    /// Channel params moved that the undo history does not have yet ([`History::sync`]).
    params_moved: bool,
    /// The quit question was answered `Yes`: the window's close request that follows is let through.
    quitting: bool,
    /// The flash shown (`Saved!`), and when it showed.
    flash: Option<(String, f64)>,
    /// The view the last frame started with: a roll that goes ends its new note's sound ([`Self::left_view`]).
    shown: View,
    /// Preview (the editors' transport toggle): the clip the engine plays alone while it is on.
    preview: Option<ClipKey>,
    /// The loop the engine has ([`Self::engine_loop`]), as built or last sent.
    sent_loop: Option<(u32, u32)>,
    /// The song length in beats at the last rebuild ([`Self::reestimate`]).
    song_length: Option<f64>,
    /// The current lane last sent as `LiveLane` ([`Self::live_lane`]): channel, and whether it is a notes lane.
    sent_lane: Option<(usize, bool)>,
    /// The song's channel ids as of the last re-key ([`Self::rekey`]): what every index the app keeps refers to.
    channel_ids: Vec<i32>,
    /// The track menu's question waiting for its answer.
    asked: Option<tracks::Asked>,
    /// The recent control: the channel id and the target of the last bound control pressed. In memory only; a new song
    /// clears it.
    recent: Option<(i32, AutomationTarget)>,
    /// The recent control's press goes on: the pointer has stayed down since it.
    recent_held: bool,
    /// `Delete automation?` waiting for its answer: the channel's id and what goes on the answer.
    delete_asked: Option<(i32, automation::Pending)>,
    /// The params this frame's screens show at the value their automation has set ([`Self::show_automation`]).
    automated: Vec<automation::Automated>,
    /// REC's live automation recording: the recent control's last value and the clips being recorded.
    auto_record: auto_record::AutoRecord,
    /// REC's live notes and drum hits ([`note_record`]).
    note_record: note_record::NoteRecord,
    /// The audio input: one device, open while something needs it ([`Self::open_input`]).
    input: InputLink,
    /// The input failed to open since what needs it last changed ([`Self::input_need`]) or an `Audio Record` channel
    /// was added: not tried again until then.
    input_failed: bool,
    /// The audio recording under way, from Play (after its count-in) to Stop.
    recorder: Option<Recorder>,
    /// The recording under way has shown its file error.
    record_failed: bool,
    /// Each channel's `INPUT` pick, kept in the storage.
    routing: Routing,
    /// What the input was last wanted for, the device and whether REC is armed: another need tries a failed open
    /// again ([`Self::open_input`]).
    input_need: Option<(Option<String>, bool)>,
    /// The latency calibration under way, and this session's result in input frames.
    calibration: Option<input::Calibrating>,
    latency: Option<u32>,
    /// This frame's egui time, and the last user action's: see [`Self::count_time`].
    now: f64,
    last_action: Option<f64>,
    /// The audio editor's region op waiting to be saved (`app/audio_edit.rs`).
    audio_edit: Option<audio_edit::Pending>,
    /// The audio export rendering behind the busy screen (`app/export.rs`).
    export: Option<crate::project_files::export::Job>,
    /// `Restore demo songs` behind its loading screen (`app/demo_restore.rs`).
    demo_restore: Option<demo_restore::DemoRestore>,
    /// `Burn Notes to Track` behind its loading screen: the channel's id, and whether the screen has had its frame
    /// (`app/tracks.rs`).
    burn: Option<(i32, bool)>,
    /// MIDI input: the device link and the router (`app/midi.rs`).
    midi: midi::Midi,
    /// The song just loaded has audio files missing: [`MISSING_AUDIO`] shows once no other message does.
    missing_audio: bool,
    /// The web: the bundled sounds the song just loaded waits for (`app/bundle_wait.rs`).
    song_download: Option<crate::content::web::Ticket>,
    /// The MOD wheel's value (channel param 8, MIDI CC 1) per channel id: the song keeps no value for it (`RPRM` holds
    /// params 0–5), so the wheel's lives here for the session, 0 until moved.
    mod_wheel: std::collections::HashMap<i32, f32>,
    /// The chord keyboard's notes sounding now, as (channel, key): each goes off on the channel it went on, wherever
    /// that channel moved since ([`Self::rekey`]).
    chord_on: Vec<(usize, u8)>,
    /// The typing keyboard's held keys and notes (`app/typed.rs`).
    typed: typed::Typed,
    /// The live notes turned on in this frame's live keys so far, as (channel, key): a note-off of one waits for its tap
    /// gate.
    fresh: Vec<(usize, u8)>,
    /// Live note-offs waiting for their tap gate, as (when it ends, egui time; channel, key): [`Self::key_off`].
    gated: Vec<(f64, usize, u8)>,
    /// Audio files dropped on the app, decoding (`app/file_drop.rs`).
    importing: Option<file_drop::Importing>,
    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    control: Option<control::Bridge>,
}

impl Default for HoneydewApp {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            settings_saved: Settings::default(),
            settings_unflushed: false,
            settings_retry_at: None,
            storage: None,
            theme: theme_by_id(Settings::default().theme),
            song: Song::default(),
            u_px: Metrics::from_pixels_per_point(1.0, crate::metrics::DEFAULT_UI_SCALE).u_px(),
            main_screen: MainScreen::default(),
            nav: Navigation::default(),
            slots: Slots::default(),
            #[cfg(test)]
            test_edit: None,
            audio: AudioLink::default(),
            selected_channel: 0,
            notes: Vec::new(),
            cut: Vec::new(),
            engine_keys: Vec::new(),
            tap_off: None,
            bar_jump: None,
            hidden: false,
            #[cfg(debug_assertions)]
            show_icon_sheet: false,
            files: ProjectFiles::default(),
            history: History::new(&Song::default()),
            shown: View::Playlist,
            param_gesture: false,
            row_drag: false,
            params_moved: false,
            quitting: false,
            flash: None,
            preview: None,
            sent_loop: None,
            song_length: None,
            sent_lane: None,
            channel_ids: tracks::channel_ids(&Song::default()),
            asked: None,
            recent: None,
            recent_held: false,
            delete_asked: None,
            automated: Vec::new(),
            auto_record: Default::default(),
            note_record: Default::default(),
            // Tests never open a device: a rig that records hands in a fake opener.
            #[cfg(not(test))]
            input: InputLink::default(),
            #[cfg(test)]
            input: InputLink::new(|_, _| Err("no input device in tests".into())),
            input_failed: false,
            recorder: None,
            record_failed: false,
            routing: Routing::default(),
            input_need: None,
            calibration: None,
            latency: None,
            now: 0.0,
            last_action: None,
            audio_edit: None,
            export: None,
            demo_restore: None,
            burn: None,
            midi: Default::default(),
            missing_audio: false,
            song_download: None,
            mod_wheel: Default::default(),
            chord_on: Vec::new(),
            typed: Default::default(),
            fresh: Vec::new(),
            gated: Vec::new(),
            importing: None,
            #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
            control: None,
        }
    }
}

impl HoneydewApp {
    fn mcp_status(&self) -> &'static str {
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            if !self.settings.mcp_enabled {
                "Disabled"
            } else if self.control.is_some() {
                "Listening"
            } else {
                "Error"
            }
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            "Disabled"
        }
    }

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_context(&cc.egui_ctx);
        let storage = settings::storage();
        let fresh =
            (storage.as_deref()).is_none_or(|s| s.get_string(settings::STORAGE_KEY).is_none());
        let mut app = Self::with_storage(storage);
        app.whats_new_at_start(fresh);
        app.midi.link = crate::midi::device::MidiLink::platform(cc.egui_ctx.clone());
        app.start_plugins(&cc.egui_ctx);
        #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
        {
            app.control = control::Bridge::start(
                cc.egui_ctx.clone(),
                app.settings.mcp_enabled,
                app.settings.mcp_port,
            );
        }
        app.extract_demo_songs(cfg!(target_arch = "wasm32"));
        // Once per start, before the purge (whose end every move waits for): a folder of the songs folder named as a row
        // beside it (`My Songs/Demo Songs`) gets a free numbered name, so that it is listed, not hidden.
        if let Err(err) = crate::ui::main_menu::rename_shadowed_folders(&mut *app.files.library) {
            eprintln!("{err}");
        }
        // Once per start, off the UI thread: the bin's expired files. The web has no bin ages.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(trash) = app.files.library.trash() {
            crate::ui::main_menu::library::purge_trash_at_start(trash);
        }
        app
    }

    /// The app with the settings `storage` holds, kept there as they change.
    fn with_storage(storage: Option<Box<dyn eframe::Storage>>) -> Self {
        let mut settings = storage
            .as_deref()
            .map_or_else(Settings::default, Settings::load);
        // The MCP token lives in its own file (desktop only: elsewhere there is none).
        if let Some(token) = settings::load_mcp_token() {
            settings.mcp_token = token;
        }
        let mut app = Self {
            theme: theme_by_id(settings.theme),
            routing: storage.as_deref().map(Routing::load).unwrap_or_default(),
            settings_saved: settings.clone(),
            settings,
            storage,
            ..Self::default()
        };
        app.audio.set_multi_core(app.settings.multi_core);
        // The song the app starts with is created now (MET`created`).
        if let Some(song) = app.files.replace(Replace::New) {
            app.history = History::new(&song);
            app.song = song;
        }
        app
    }

    /// What's New, once per notes version ([`views::whats_new::due`]): it opens by itself on phones and tablets after an
    /// upgrade; a `fresh` install (no stored settings) only takes the version.
    fn whats_new_at_start(&mut self, fresh: bool) {
        if !views::whats_new::due(&self.settings.whats_new_seen) {
            return;
        }
        if !fresh && cfg!(target_os = "android") {
            self.nav.overlays.push(Overlay::WhatsNew);
        }
        self.settings.whats_new_seen = views::whats_new::NOTES_VERSION.into();
    }

    /// The first run puts our demo songs into the songs folder, once per set (`demoSongsVersion`), replacing no copy
    /// of the user's; a failure (no songs folder) is logged and tried again at the next start. `every_start`: the library
    /// lasts a session (the web's), so they go into it at every start. The settings keep the flag at the end of the
    /// first frame.
    fn extract_demo_songs(&mut self, every_start: bool) {
        if !every_start && self.settings.demo_songs_version >= settings::DEMO_SONGS_VERSION {
            return;
        }
        match crate::content::restore_demo_songs(&mut *self.files.library, false) {
            Ok(()) => self.settings.demo_songs_version = settings::DEMO_SONGS_VERSION,
            Err(e) => eprintln!("{}: {e}", crate::content::DEMO_FOLDER),
        }
    }

    /// Applies and keeps what the frame changed in the settings: the theme and the metronome level at once (the UI
    /// scale is read every frame), the storage in memory at once, and the disk only while no pointer button is down, so that a slider
    /// drag writes the file once, at its end.
    fn keep_settings(&mut self, ctx: &egui::Context) {
        if self.settings != self.settings_saved {
            if self.theme.id != self.settings.theme {
                self.theme = theme_by_id(self.settings.theme);
            }
            // The click level, heard at once (the engine keeps one level for REC's clicks and the flag's).
            let level = self.settings.metronome_level;
            if level != self.settings_saved.metronome_level {
                let on = self.audio.metronome_flag();
                self.audio.send(AudioCommand::MetronomeFlag { on, level });
            }
            if self.settings.low_latency != self.settings_saved.low_latency {
                self.audio
                    .set_low_latency(&self.song, self.settings.low_latency);
            }
            if self.settings.sound_mode != self.settings_saved.sound_mode {
                self.audio
                    .set_sound_mode(&self.song, self.settings.sound_mode);
            }
            if self.settings.multi_core != self.settings_saved.multi_core {
                self.audio.set_multi_core(self.settings.multi_core);
            }
            #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
            if self.settings.mcp_enabled != self.settings_saved.mcp_enabled
                || self.settings.mcp_port != self.settings_saved.mcp_port
            {
                self.control = None;
                if self.settings.mcp_enabled {
                    self.control = control::Bridge::start(
                        ctx.clone(),
                        self.settings.mcp_enabled,
                        self.settings.mcp_port,
                    );
                }
            }
            if let Some(storage) = self.storage.as_deref_mut() {
                self.settings.save(storage);
            }
            self.settings_saved = self.settings.clone();
            self.settings_unflushed = true;
            ctx.request_repaint();
        }
        let now = ctx.input(|i| i.time);
        let due = self.settings_retry_at.is_none_or(|at| now >= at);
        if self.settings_unflushed && due && !ctx.input(|i| i.pointer.any_down()) {
            if let Some(storage) = self.storage.as_deref_mut() {
                storage.flush();
            }
            // A failed flush is tried again a second later at the soonest (each try writes a file), and at exit.
            // ponytail: no repaint is asked for the retry; it waits for the next frame.
            self.settings_unflushed = self.storage.as_deref().is_some_and(settings::unflushed);
            self.settings_retry_at = self.settings_unflushed.then_some(now + 1.0);
        }
    }

    /// The app with `song` open, as an opened project is: saved as it is, the start of the undo history, and the
    /// first channel after the master selected (for render harnesses).
    pub fn with_song(song: Song) -> Self {
        let mut app = Self::default();
        app.files.saved();
        app.history = History::new(&song);
        app.selected_channel = usize::from(song.channels.len() > 1);
        app.channel_ids = tracks::channel_ids(&song);
        app.song = song;
        app
    }

    /// Switches the view or the overlays, as the screens do. The bottom editor follows the playlist's current clip
    /// ([`Slots::follow`]): a clip's `Edit` makes it its track's current clip and the track the current one (its channel
    /// takes the keyboard); a channel's `Edit` is its main lane's current clip's
    /// ([`views::main_clip`]), else nothing.
    pub fn navigate(&mut self, nav: Nav) {
        let nav = match nav {
            Nav::Edit(channel) => match views::main_clip(&self.song, channel) {
                Some(key) => Nav::EditClip(key),
                None => return,
            },
            nav => nav,
        };
        if let Nav::EditClip(key) = nav
            && key.clip(&self.song).is_some()
        {
            let keys = &mut self.selected_channel;
            (self.main_screen.clips).make_current(&mut self.song, keys, key.into());
            self.slots.open(key, &self.song);
        }
        self.nav.apply(nav, &self.song);
    }

    /// The one funnel for song edits, from the screens ([`AppCtx::song_edited`]) and the app alike. `tick` is
    /// the transport position before the edit.
    /// - ChannelParam (a channel param, a rack card's settings or size): the audio has it already (`SetParam`). It sets
    ///   The song-changed flag and saves no undo step: the undo history takes it ([`History::sync`]) once no pointer is
    ///   down, and before an undo or redo.
    /// - Param: the audio has it already or never needs it. The undo history notes a gesture in progress, which the end
    ///   of the frame closes once no pointer is down. Each one sets the song-changed flag; so does every undo step saved
    ///   (Final, Quiet, Resync), and a rack or param change with none ([`Self::record`]). RackState, RackRebuild and
    ///   Untracked do not set it (a pick sets it itself); one that closes an open gesture into a step does.
    /// - Ongoing (mid-drag): sets the song-changed flag; the undo history notes a gesture in progress, which the drag's
    ///   final edit closes into one step (a drag its view or overlay took away ends there: [`Self::left_view`]). A
    ///   tempo drag's tempo goes to the engine live ([`AudioLink::set_tempo`]).
    /// - RackState: no undo step and no rebuild; every undo and redo step keeps it ([`History::rack_state`]).
    /// - RackRebuild, Untracked (a pick): as RackState, and a rebuilt engine.
    /// - Final: the undo history records the step, but for the rack and params (the rack and presets are not covered by
    ///   undo: a module's insert, delete, move or paste, Clear Rack), and the audio gets a rebuilt engine
    ///   ([`Self::rebuild_as`]) that takes over the playing one's sound and transport, cutting only what the edit
    ///   destroys ([`Handover::Edit`]).
    /// - Every edit but Replaced first re-keys what the app keeps by channel index, if the channels moved
    ///   ([`Self::rekey`]).
    /// - Replaced (a whole new song): starts a new undo history, turns Preview off, resets the navigation, the slots'
    ///   state but the clipboard, the clip selection and the timeline scroll (they belong to the old song), selects
    ///   the first channel after the master, stops the transport and rebuilds.
    fn song_edited(&mut self, edit: SongEdit, tick: u64) {
        if edit != SongEdit::Replaced {
            self.rekey(&[]);
            self.count_time();
        }
        // A stronger edit ends the gesture its own way; a channel param's leaves it.
        if edit != SongEdit::ChannelParam {
            self.param_gesture = edit == SongEdit::Param;
        }
        match edit {
            SongEdit::ChannelParam => {
                self.files.song_changed();
                self.params_moved = true;
            }
            SongEdit::Param => {
                self.files.song_changed();
                self.history.edited(&self.song, false);
            }
            SongEdit::Ongoing => {
                self.files.song_changed();
                self.history.edited(&self.song, false);
                // A tempo drag: the engine takes the tempo live.
                self.audio.set_tempo(self.song.tempo_bpm);
            }
            SongEdit::RackState | SongEdit::RackRebuild | SongEdit::Untracked => {
                // It closed an open gesture into a step: a saved undo step.
                if self.history.rack_state(&self.song) {
                    self.files.song_changed();
                }
                if edit != SongEdit::RackState {
                    self.rebuild_as(tick, Handover::Edit);
                }
            }
            SongEdit::Final => {
                self.record();
                self.rebuild_as(tick, Handover::Edit);
            }
            SongEdit::Quiet => {
                self.record();
                self.rebuild_as(tick, Handover::Quiet);
            }
            // A re-sync whatever changed, as an undo's.
            SongEdit::Resync => {
                self.record();
                self.rebuild_as(tick, Handover::Undo);
            }
            SongEdit::Replaced => {
                // A new song counts from its own first action, and no slider drag of the old one goes on.
                (self.last_action, self.row_drag) = (None, false);
                // Its view zooms per U, for its history and engine too.
                self.song.zooms_to_units(f64::from(self.u_px));
                self.history = History::new(&self.song);
                (self.channel_ids, self.asked) = (tracks::channel_ids(&self.song), None);
                (self.recent, self.delete_asked) = (None, None);
                // The MOD wheel's values were the old song's channels'.
                self.mod_wheel.clear();
                // Loading a song turns Preview off.
                self.preview = None;
                self.nav = Navigation::default();
                // Every note sounding stops in the old engine, which a failed build keeps: the pads' before the
                // slots that keep them go, and the keyboard's.
                self.stop_notes();
                // The clipboard is the app's, not the song's; so are the drum mixer's folds, the step view's and the
                // free grid's (only a new run starts them collapsed).
                // A rack's `Set Range` waiting for a key ends: `AudioLink` would give its learn to the new song's
                // module with the same ids.
                if let Some(end) = self.slots.rack.end_learn() {
                    self.audio.send(end.command());
                }
                let editor = &self.slots.drum_editor;
                let folds = (editor.step.fold, editor.grid.fold);
                // The performance monitor is the app's, not the song's: a song load changes nothing of it (visibility,
                // idle, history).
                let performance = std::mem::take(&mut self.slots.performance);
                // So is the chord editor's open state, which KEYB does not keep (a load does not close it).
                let chord_editor = self.slots.keyboard_settings.editor;
                self.slots = Slots {
                    clipboard: std::mem::take(&mut self.slots.clipboard),
                    performance,
                    ..Slots::default()
                };
                self.slots.keyboard_settings.editor = chord_editor;
                let editor = &mut self.slots.drum_editor;
                (editor.step.fold, editor.grid.fold) = folds;
                // The clip selection, the timeline's scroll and the ruler's gesture belong to the old song too.
                self.main_screen.clips = Default::default();
                self.main_screen.cancel_hold();
                self.main_screen.region = Default::default();
                self.main_screen.scroll_x = 0.0;
                // Its row height comes from its stored view (HEAD@296 zoom y) on the next frame.
                self.main_screen.row_u = None;
                self.main_screen.forget_keys_views(|_| true);
                self.selected_channel = usize::from(self.song.channels.len() > 1);
                // The new engine starts stopped; the stop is for an old engine that a failed build leaves. Any stop
                // disarms REC; what it recorded belongs to the old song's history.
                self.main_screen.playing = false;
                self.main_screen.armed = false;
                self.auto_record.stop();
                self.note_record.forget();
                // A recording's files stay, closed as they are (they load); their clips were the old song's.
                self.recorder = None;
                self.audio.send(AudioCommand::Stop);
                // A new cache: the old song's samples are freed (the web's memory) and the missing ones read again.
                // Real-time safe: the playing engine holds its own `Arc`s, and a retired engine is dropped on this
                // thread.
                self.audio.samples = SampleCache::default();
                // The old song's plugins go; the new one's start with its engine.
                crate::plugins::song_replaced();
                // A fresh engine and the missing check; on the web once the bundled sounds are in (`app/bundle_wait.rs`).
                self.song_loaded();
            }
        }
    }

    /// A whole new song ([`SongEdit::Replaced`]), which stops every live note: a finger still on the piano or on the
    /// chord editor's button holds nothing on it until it lifts and presses again.
    fn song_replaced(&mut self, ctx: &egui::Context) {
        self.song_edited(SongEdit::Replaced, 0);
        crate::ui::keyboard::let_go(ctx);
        crate::ui::chord_editor::let_go(ctx);
    }

    /// Undo or redo: the song becomes the one restored, keeping its title (the file it is saved as) and its play loop
    /// (no edit), and the audio gets it as an edit's. An active drag is cancelled, or its next frame would apply its
    /// edit again. It sets the song-changed flag.
    fn step(&mut self, ctx: &egui::Context, step: Step, tick: u64) {
        #[cfg(any(target_arch = "wasm32", test))]
        if live::undo(matches!(step, Step::Redo)) {
            return;
        }
        // What the step keeps.
        self.params_moved = false;
        self.history.sync(&self.song);
        let restored = match step {
            Step::Undo => self.history.undo(),
            Step::Redo => self.history.redo(),
        };
        let Some(mut song) = restored else {
            return;
        };
        // The title, play loop, save stamp and view state are no edits: the current ones stay.
        song.keep_unedited(&self.song);
        // Channels and tracks the step brought back or took away move what is kept for the others.
        let shifts = tracks::lane_shifts(&self.song, &song);
        // Each track's current clip is view state: it stays, wherever the step moved the track.
        tracks::carry_current_clips(&self.song, &mut song, &shifts);
        let before = std::mem::replace(&mut self.song, song);
        self.files.song_changed();
        self.count_time();
        ctx.stop_dragging();
        // The undo ends a sample-row slider's drag too: nothing is left for its lift to close.
        self.row_drag = false;
        self.rekey(&shifts);
        // A channel the step took away keeps no keyboard view: a new one may take its id. One a delete took away keeps
        // its view, for the undo that brings it back.
        let ids = &self.channel_ids;
        let gone = |id: i32| before.channels.iter().any(|c| c.id == id) && !ids.contains(&id);
        self.main_screen.forget_keys_views(gone);
        // The selected channel may be gone.
        self.selected_channel = self.selected_channel.min(self.song.channels.len() - 1);
        self.rebuild_as(tick, Handover::Undo);
    }

    /// A completed edit's undo step: the song-changed flag for it, or for a rack or param change with none
    /// ([`History::sync`]).
    fn record(&mut self) {
        if self.history.sync(&self.song) | self.history.edited(&self.song, true) {
            self.files.song_changed();
        }
    }

    /// A user action (an edit, an undo or redo) at [`Self::now`]: the META editing time (@256) gains the time since the
    /// last one, unless that is [`IDLE_SECS`] or more, the time is negative (not counted, or hidden) or the song is
    /// locked (META@776 = 1 also stops time counting). It is no edit itself ([`Song::keep_unedited`]), nor an unsaved
    /// change: it sets no song-changed flag.
    fn count_time(&mut self) {
        let meta = &mut self.song.file.meta;
        if let Some(last) = self.last_action.replace(self.now)
            && (0.0..IDLE_SECS).contains(&(self.now - last))
            && meta.editing_seconds >= 0.0
            && !meta.locked
        {
            meta.editing_seconds += self.now - last;
        }
    }

    /// Hands the audio a fresh engine built from what it plays ([`Self::rebuild_as`]).
    fn rebuild(&mut self, tick: u64) {
        self.rebuild_as(tick, Handover::Fresh);
    }

    /// Hands the audio an engine rebuilt from what it plays: the song, or while Preview is on the previewed clip alone
    /// ([`preview::solo`]), at `tick`, playing if it was; a song edit's (`how`) may take over the playing engine
    /// ([`AudioLink::replace_song_keeping`]). A previewed clip that is gone turns Preview off.
    fn rebuild_as(&mut self, tick: u64, how: Handover) {
        let seek = self.reestimate(tick, how);
        let solo = self.solo();
        if solo.is_none() {
            self.preview = None;
        }
        // A take's clip stays out of playback while it records.
        let song = solo.or_else(|| self.note_record.without_take(&self.song));
        let song = song.as_ref().unwrap_or(&self.song);
        let read = sample_cache::reader(&self.files);
        // A new engine reads its loop from that song.
        // The samples of the clips Preview mutes stay cached: Preview off decodes nothing.
        let playing = self.main_screen.playing;
        if (self.audio).replace_song_keeping(song, &self.song, playing, tick, &read, how) {
            self.sent_loop = self.engine_loop();
        }
        drop(read);
        if let Some(start) = seek {
            self.audio.send(AudioCommand::Seek(start));
        }
        // A new engine starts from each channel's first lane: the current one goes again, right after it.
        self.sent_lane = None;
        self.live_lane();
    }

    /// Song length re-estimate: an edit that changes the song length (the latest clip end rounded up to whole bars,
    /// [`crate::render::song_length_beats`]) clips the song loop to the new end, its end only when not recording; a
    /// loop left empty is none. The engine wraps a position past the end itself. Playing at `tick` outside the
    /// non-empty loop the engine plays, the loop start to seek to (a re-sync). A fresh engine (a song opened)
    /// clips nothing; nor does an edit while Preview is on, as the loop holds Preview's markers then
    /// ([`Self::preview_on`]), which may lie past the song end; and an undo does not bring a clipped loop back, as no
    /// step holds the loop ([`Song::keep_unedited`]).
    fn reestimate(&mut self, tick: u64, how: Handover) -> Option<u64> {
        let length = crate::render::song_length_beats(&self.song);
        let last = self.song_length.replace(length);
        if how == Handover::Fresh || last.is_none_or(|last| last == length) {
            return None;
        }
        let end = (length * f64::from(crate::song::PPQ)) as u32;
        let keep_end = self.recording();
        if self.preview.is_none() {
            self.song.play_loop = (self.song.play_loop)
                .map(|(start, stop)| (start.min(end), if keep_end { stop } else { stop.min(end) }))
                .filter(|(start, stop)| start < stop);
        }
        let (start, stop) = self.engine_loop().filter(|(start, stop)| start < stop)?;
        let outside = !(u64::from(start)..u64::from(stop)).contains(&tick);
        (self.main_screen.playing && outside).then_some(start.into())
    }

    /// Before the frame and after its navigation: a view other than the one last recorded ends what that one left
    /// under way, however the view went (the X, the mixer button or F9, `Select clip`, a nav between frames), for it
    /// will not see the release: a piano roll's new note within its gate, its key column's key (its press stale for every
    /// roll) and its drag ([`PianoRoll::hide`]: the dragged key stops, and a move or paint that reported edits is a final edit, a lane
    /// touch that changed nothing a re-sync, returned for the caller to make); the automation and audio editors' drags end with no edit (their `end_touch`). The drum
    /// editor's end with its popups (`views::drum_editor::after_chrome`). The playlist goes with the mixer: its clip
    /// drag ends where it left the clips, a final edit ([`Clips::end_drag`]). So does a tempo dial drag whose editor
    /// went another way than its own Close (the `showTempo` toggle, [`views::tempo_editor::closed`]).
    ///
    /// [`Clips::end_drag`]: crate::ui::clips::Clips::end_drag
    fn left_view(&mut self, ctx: &egui::Context) -> Option<SongEdit> {
        let tempo = !self.nav.overlays.contains(&Overlay::TempoEditor)
            && views::tempo_editor::closed(&mut self.slots.tempo_editor, self.song.tempo_bpm);
        let mut edit = tempo.then_some(SongEdit::Final);
        let drawn = std::mem::replace(&mut self.shown, self.nav.view);
        if drawn == self.nav.view {
            return edit;
        }
        let slots = &mut self.slots;
        match drawn {
            View::PianoRoll(channel) => {
                // The key column's press is stale, so no roll shown under it (this one again
                // or another channel's) plays it until it lifts and presses again.
                crate::ui::let_go_press(ctx, crate::ui::piano_roll::keys_id());
                let roll = slots.piano_roll.rolls.get_mut(&channel);
                for action in roll.map(PianoRoll::hide).into_iter().flatten() {
                    match action {
                        PianoRollAction::Note {
                            key,
                            velocity: None,
                        } => self.audio.send(AudioCommand::NoteOff { channel, key }),
                        PianoRollAction::SongEdited { final_edit: true } => {
                            edit = edit.max(Some(SongEdit::Final));
                        }
                        // A lane touch that changed nothing re-syncs, as its release would.
                        PianoRollAction::Resync => edit = edit.max(Some(SongEdit::Resync)),
                        _ => {}
                    }
                }
            }
            View::AutomationEditor(_) => slots.automation_editor.editor.end_touch(),
            View::AudioEditor(_) => slots.audio_editor.editor.end_touch(),
            View::Playlist | View::Mixer | View::DrumEditor(_) => {}
        }
        let clips = &mut self.main_screen.clips;
        if self.nav.view == View::Mixer
            && clips.end_drag(&mut self.song, &mut self.audio.samples, &self.files)
        {
            edit = edit.max(Some(SongEdit::Final));
        }
        edit
    }

    /// Tells the engine, when it changes and after every new engine (a rebuild, a song load, an audio start), whether
    /// The current lane of the channel live keys play on is a notes lane, which routes them: an open editor's lane (the
    /// piano roll's is one; the drum, automation and audio editors' are not), else the playlist's selected clip's on
    /// The selected channel, else that channel's first lane.
    fn live_lane(&mut self) {
        let lane = match self.nav.view {
            View::PianoRoll(channel) => Some((channel, true)),
            View::DrumEditor(channel)
            | View::AutomationEditor(channel)
            | View::AudioEditor(channel) => Some((channel, false)),
            View::Playlist | View::Mixer => {
                let channel = self.selected_channel;
                let current = self.current_track().filter(|&(c, _)| c == channel);
                let lanes = self.song.channels.get(channel).map(|c| &c.lanes);
                let lane = lanes.and_then(|lanes| lanes.get(current.map_or(0, |(_, lane)| lane)));
                lane.map(|lane| (channel, lane.kind == LaneKind::Notes))
            }
        };
        if let Some((channel, notes)) = lane.filter(|_| lane != self.sent_lane) {
            self.sent_lane = lane;
            self.audio.send(AudioCommand::LiveLane { channel, notes });
        }
    }

    /// The loop the engine plays: Preview's ([`preview::engine_loop`]) while it is on, else the song's play loop.
    fn engine_loop(&self) -> Option<(u32, u32)> {
        match self.preview.and_then(|key| key.clip(&self.song)) {
            Some(clip) => preview::engine_loop(clip, self.song.play_loop),
            None => self.song.play_loop,
        }
    }

    /// Preview after the screens: the transport bar's button turns it on for the current clip or off. It plays the
    /// current track's current clip whatever the view shows: it moves with it (changing the clip or channel keeps
    /// Preview on and moves it to the new clip), and goes off without one (the editor collapsed for want of
    /// it, taking Preview's button; the X keeps the clip, and Preview on it). While it plays, the engine loops its loop
    /// ([`preview::looped`]: each play-loop marker inside the clip's span, else the span's; from tick 0 on, and none
    /// wholly before the song start, s₀ ≤ −2L), and without Preview the song's play loop. A loop that changes without a
    /// rebuild (a ruler) goes to the engine as `SetLoop`. The engine wraps a playhead at or past the end; Preview jumps
    /// one before its loop to the loop's start.
    fn preview(&mut self, ctx: &egui::Context, clicked: bool, current: Option<ClipKey>, tick: u64) {
        match (clicked, self.preview, current) {
            (true, Some(_), _) | (false, Some(_), None) => self.preview_off(ctx, tick),
            (true, None, Some(key)) => self.preview_on(ctx, key, tick),
            (false, Some(on), Some(key)) if on != key => self.preview_on(ctx, key, tick),
            _ => {
                let looped = self.engine_loop();
                if looped != self.sent_loop && self.audio.running() {
                    self.sent_loop = looped;
                    let ticks = looped.map(|(start, end)| (start.into(), end.into()));
                    self.audio.send(AudioCommand::SetLoop(ticks));
                }
                let playing = self.preview.is_some() && self.main_screen.playing;
                if let Some((start, _)) = looped.filter(|_| playing)
                    && tick < u64::from(start)
                {
                    self.audio.send(AudioCommand::Seek(start.into()));
                }
            }
        }
    }

    /// Turns Preview on for `key`, or moves it there: the song's play loop is saved clamped into the clip
    /// ([`preview::clamped_loop`]), every note stops, the engine gets the clip alone, and a playhead outside its loop
    /// ([`preview::looped`]) jumps to the loop's start, clamped to ≥ 0.
    /// The saved play loop is no undo step, as no play-loop change is ([`Song::keep_unedited`]); the app makes
    /// it, so it sets no song-changed flag either, as a ruler's loop edit does not. Turning Preview off does not
    /// restore it.
    fn preview_on(&mut self, ctx: &egui::Context, key: ClipKey, tick: u64) {
        let clip = key.clip(&self.song);
        let Some((play_loop, (s, e))) = clip.and_then(|c| {
            let play_loop = preview::clamped_loop(c, self.song.play_loop);
            Some((play_loop, preview::looped(c, play_loop)?))
        }) else {
            return;
        };
        self.song.play_loop = play_loop;
        // All notes off on every channel, the handover's: the live keys' notes are over.
        self.live_notes_silenced(ctx);
        self.preview = Some(key);
        let at = if (s..e).contains(&(tick as i64)) {
            tick
        } else {
            s.max(0) as u64
        };
        // The engine taking over keeps its position: one outside the loop jumps.
        let seek = (at != tick).then_some(at);
        self.rebuild_as(at, Handover::Preview { seek });
    }

    /// Open / save , after the screens: the shortcuts while no Save dialog or message is open, the
    /// newer-version answer, finished file requests, and the overlays they ask for. An opened song replaces the
    /// current one.
    fn project_files(&mut self, ctx: &egui::Context) {
        let busy = self.nav.overlays.iter().any(|o| {
            matches!(
                o,
                Overlay::SaveDialog { .. }
                    | Overlay::PresetDialog { .. }
                    | Overlay::Message { .. }
                    | Overlay::SampleName { .. }
                    | Overlay::Busy { .. }
                    | Overlay::MidiImport
            )
        });
        let answer = self
            .files
            .waiting()
            .and_then(|tag| self.nav.take_choice(tag));
        self.files.songs_folder = self.slots.main_menu.songs_folder().map(Into::into);
        // The first-import notice's `Import`, kept in the settings.
        self.files.import_accepted |= self.settings.import_notice_accepted;
        if let Some(song) = self.files.update(ctx, &mut self.song, !busy, answer) {
            self.song = song;
            self.song_replaced(ctx);
        }
        self.settings.import_notice_accepted = self.files.import_accepted;
        // A card (and an instrument's channel) named after the preset just saved from it: rack state, with no undo step
        // and nothing for the audio (the rack is not covered by undo), and a song change.
        // `Locate samples…` loaded samples into the song: an edit, and its audio is built again.
        if self.files.take_relinked() {
            self.files.song_changed();
            self.song_edited(SongEdit::Final, 0);
            ctx.request_repaint();
        }
        if self.files.take_preset_named() {
            self.files.song_changed();
            self.song_edited(SongEdit::RackState, 0);
            ctx.request_repaint();
        }
        while let Some(overlay) = self.files.take_overlay() {
            self.navigate(Nav::Open(overlay));
        }
        // After the load's own message (one shows at a time).
        let message = |o: &Overlay| matches!(o, Overlay::Message { .. });
        if self.missing_audio && !self.nav.overlays.iter().any(message) {
            self.missing_audio = false;
            self.navigate(Nav::Open(Overlay::Message {
                tag: MISSING_TAG,
                lines: vec![MISSING_AUDIO.into()],
                confirm: None,
                extra: None,
            }));
        }
    }

    /// Quit: the window's close button asks [`quit_question`], holding the close back; `Yes` closes the window. (The
    /// web has no window to close.)
    fn quit(&mut self, ctx: &egui::Context) {
        if let Some((QUIT_TAG, yes)) = self.nav.answer() {
            self.nav.take_answer(QUIT_TAG);
            if yes {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if !self.quitting && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.navigate(quit_question());
            ctx.request_repaint();
        }
    }

    /// Shows the flash [`ProjectFiles`] asks for over everything, centred, in the inverted popup style, for
    /// [`FLASH_SECS`]. Its text may have several lines (`\n`), each centred, and wraps inside the screen.
    fn flash(&mut self, ctx: &egui::Context, m: &Metrics) {
        let now = ctx.input(|i| i.time);
        if let Some(text) = self.files.take_flash() {
            self.flash = Some((text, now));
        }
        let Some((text, since)) = &self.flash else {
            return;
        };
        let left = FLASH_SECS - (now - since);
        if left <= 0.0 {
            self.flash = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
        let colors = &self.theme.colors;
        let (fill, ink) = (colors.text.to_color32(), colors.background.to_color32());
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("flash"),
        ));
        let pad = m.u(1.0);
        let screen = ctx.content_rect();
        let mut job = egui::text::LayoutJob::simple(
            text.clone(),
            m.font_id(FontSize::Pt12),
            ink,
            (screen.width() - m.u(2.0) - pad).max(pad),
        );
        job.halign = egui::Align::Center;
        let galley = painter.layout_job(job);
        let rect =
            egui::Rect::from_center_size(screen.center(), galley.size() + egui::Vec2::splat(pad));
        painter.rect_filled(rect, 0.0, fill);
        // A centred galley is anchored at its middle, so it is placed by its top centre.
        painter.galley(
            egui::pos2(rect.center().x, rect.top() + pad / 2.0),
            galley,
            ink,
        );
    }

    /// What the engine plays while Preview is on: the previewed clip alone ([`preview::solo`]).
    fn solo(&self) -> Option<Song> {
        self.preview.and_then(|key| preview::solo(&self.song, key))
    }

    /// How touches play the keyboard. The keys are disabled while no channel plays (the master alone);
    /// key-lock is on while the project scale (the song's key lock) is not Chromatic, and every channel the keyboard
    /// plays supports it.
    ///
    /// What the keys light: the selected channel's notes on, from the keyboard, typed keys and MIDI (T = 1), and the
    /// keys the engine sounds at its input (T = 0.2: song playback, previews). The channel's key range spans
    /// its instrument modules' `ADD1` ranges, and is every key without one.
    fn key_input(&self) -> KeyInput {
        let (root, mask) = project_scale(&self.song);
        let lock = (mask != SCALE_MASKS[CHROMATIC]).then_some((root, mask));
        let channel = self.selected_channel;
        let notes = self.sounding(channel);
        let modules = (self.song.channels.get(channel)).map_or(&[][..], |c| &c.rack.modules);
        let range = (modules.iter())
            .filter(|m| crate::ui::rack::instrument(m.type_id).is_some())
            .map(|m| (m.file.key_low.max(0) as u8, m.file.key_high.max(0) as u8))
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
        KeyInput {
            enabled: channel > 0,
            lock,
            all_white: lock.is_some() && self.settings.keyboard_kind() == KeyboardMode::Scale,
            notes,
            sounding: self.engine_keys.get(channel).copied().unwrap_or(0),
            range,
        }
    }

    /// The keys sounding on `channel` from the keyboard, typed notes and MIDI, bit k for key k.
    fn sounding(&self, channel: usize) -> u128 {
        let touched = (self.notes.iter()).filter(|n| n.0 == channel).map(|n| n.1);
        // MIDI keys light the key they play under key-lock.
        let midi = (self.midi.snapped.iter())
            .filter(|s| s.0 == channel)
            .map(|s| s.2);
        (touched.chain(self.typed.notes(channel)).chain(midi))
            .filter(|&k| k < 128 && !self.cut.contains(&(channel, k)))
            .fold(0, |bits, k| bits | 1_u128 << k)
    }

    /// Play start cut every live key held: each goes dark until let go, and its release then sends a note-off with no
    /// effect; a take this Play starts records none of them (Play start clears the live-held marks). Not the
    /// keys whose press is in the Play's frame: their notes wait for that Play ([`AudioLink::hold_notes`]), and sound,
    /// light and record after it.
    fn cut_live_keys(&mut self) {
        let waiting: Vec<_> = self.audio.waiting_presses().collect();
        for channel in 0..self.song.channels.len() {
            let lit = self.sounding(channel);
            let keys = (0..128)
                .filter(|&k| lit >> k & 1 == 1)
                .map(|k| (channel, k));
            self.cut.extend(keys.filter(|key| !waiting.contains(key)));
        }
        let cut = &self.cut;
        (self.note_record.held).retain(|h| !cut.contains(&(h.0, h.1)));
    }

    /// The keyboard is a piano: the current channel's first lane is an automation lane, or the chosen kind (the
    /// `keyboardKind` setting) is `Classic` or `Scale`; else the chord keyboard shows. (Pads and the sample row replace
    /// The keys on their own.)
    fn piano_keys(&self) -> bool {
        let first = (self.song.channels.get(self.selected_channel)).and_then(|c| c.lanes.first());
        first.is_some_and(|l| l.kind == LaneKind::Automation)
            || !self.settings.keyboard_kind().chords()
    }

    /// Makes `keys` (key, velocity) the notes sounding on the selected channel: NoteOff for each sounding note not
    /// in them, then NoteOn at its velocity for each key not sounding yet.
    fn play_key(&mut self, keys: &[(u8, f32)]) {
        let channel = Some(self.selected_channel).filter(|&c| c > 0);
        let notes: Vec<(usize, u8, f32)> = (channel.iter())
            .flat_map(|&c| keys.iter().map(move |&(k, v)| (c, k, v)))
            .collect();
        let off =
            (self.notes.iter()).filter(|&&(c, k)| !notes.iter().any(|n| (n.0, n.1) == (c, k)));
        for (channel, key) in off.copied().collect::<Vec<_>>() {
            self.key_off(channel, key);
        }
        for &(channel, key, velocity) in &notes {
            if !self.notes.contains(&(channel, key)) {
                self.key_on(channel, key, velocity);
            }
        }
        self.notes = notes.into_iter().map(|(c, k, _)| (c, k)).collect();
    }

    /// A live key's note-on, from the keyboard, typed keys or MIDI: their one way to the engine. A note-off of the key
    /// still waiting for its tap gate goes first, or it would end the new note.
    fn key_on(&mut self, channel: usize, key: u8, velocity: f32) {
        self.cut.retain(|&cut| cut != (channel, key));
        if let Some(i) = self.gated.iter().position(|g| (g.1, g.2) == (channel, key)) {
            self.gated.remove(i);
            self.audio.send(AudioCommand::NoteOff { channel, key });
        }
        self.fresh.push((channel, key));
        self.audio.send(AudioCommand::NoteOn {
            channel,
            key,
            velocity,
        });
        // A rack card's `Set Range` / `Set First Note` learns from every key played, MIDI included.
        self.slots.rack.key_played(channel, key);
        self.live_key((channel, key, Some(velocity)));
    }

    /// A live key's note-off, [`Self::key_on`]'s other half. One in its note-on's frame (a chord cell or a typed key
    /// tapped within a frame, a MIDI tap in one batch) waits for the tap gate, as a tapped piano key does: the engine
    /// drains both before it renders, which would make a silent note ([`Self::gate_offs`]).
    fn key_off(&mut self, channel: usize, key: u8) {
        self.cut.retain(|&cut| cut != (channel, key));
        if self.fresh.contains(&(channel, key)) {
            self.gated.push((self.now + TAP_GATE, channel, key));
        } else {
            self.audio.send(AudioCommand::NoteOff { channel, key });
        }
        self.live_key((channel, key, None));
    }

    /// The gated note-offs whose tap gate has ended go, or `all` of them (a hidden web page runs no frame to end a
    /// gate). Returns when the next one ends.
    fn gate_offs(&mut self, all: bool) -> Option<f64> {
        let now = self.now;
        let (due, wait) = std::mem::take(&mut self.gated)
            .into_iter()
            .partition(|g: &(f64, usize, u8)| all || g.0 <= now);
        self.gated = wait;
        for (_, channel, key) in due {
            self.audio.send(AudioCommand::NoteOff { channel, key });
        }
        self.gated.iter().map(|g| g.0).reduce(f64::min)
    }

    /// Every channel's notes were silenced by Preview going on or off (its handover's all notes off on every channel):
    /// The live keys forget theirs, sending nothing but their key-ups to REC, and a finger or key still down holds
    /// nothing until it lifts, so no note comes back and no key stays lit. So do the drum pads
    /// ([`views::drum_editor::silenced`]).
    fn live_notes_silenced(&mut self, ctx: &egui::Context) {
        views::drum_editor::silenced(&mut self.slots.drum_editor);
        let mut gone = std::mem::take(&mut self.notes);
        gone.append(&mut self.chord_on);
        gone.extend(self.typed.rekey(&|_| None));
        gone.extend(self.midi.snapped.drain(..).map(|(c, _, key)| (c, key)));
        self.midi.router.rekey(&|_| None);
        (self.gated, self.tap_off, self.cut) = Default::default();
        self.main_screen.chords.release_all();
        crate::ui::keyboard::let_go(ctx);
        crate::ui::chord_editor::let_go(ctx);
        for (channel, key) in gone {
            self.live_key((channel, key, None));
        }
    }

    /// Preview goes off, by its button, for want of a clip, or as a recording starts: the whole song again, with the
    /// handover's all notes off on every channel ([`Self::live_notes_silenced`]).
    fn preview_off(&mut self, ctx: &egui::Context, tick: u64) {
        self.preview = None;
        self.live_notes_silenced(ctx);
        self.rebuild_as(tick, Handover::Preview { seek: None });
    }

    /// What the chord keyboard plays from: `set`, and the song's `KEYB`.
    fn chord_input<'a>(&self, set: &'a ChordSet, enabled: bool) -> ChordInput<'a> {
        let k = &self.song.file.keyboard;
        let state = &self.slots.keyboard_settings;
        ChordInput {
            set,
            strum: self.settings.keyboard_kind() == KeyboardMode::Strum,
            shift: k.octave_shift.clamp(-2, 2),
            velocity: k.velocity,
            timing_ms: k.timing.clamp(-TIMING_MS, TIMING_MS),
            hold: k.hold,
            editor: state.editor,
            enabled,
        }
    }

    /// One of the chord keyboard's notes: a note-on on the current channel (none on the master), and a note-off on the
    /// channel its note went on.
    fn chord_note(&mut self, note: Note) {
        match note {
            Note::On(key, velocity) => {
                let channel = self.selected_channel;
                if channel > 0 {
                    self.chord_on.push((channel, key));
                    self.key_on(channel, key, velocity);
                }
            }
            Note::Off(key) => {
                if let Some(i) = self.chord_on.iter().position(|&(_, k)| k == key) {
                    let (channel, _) = self.chord_on.remove(i);
                    self.key_off(channel, key);
                }
            }
        }
    }

    /// Keeps the keyboard panel's new `sizes` (points) as its `preferredKbdSize` and `recentKbdSize` settings, in U.
    pub(super) fn set_panel_sizes(&mut self, sizes: panel_resize::Sizes, m: &Metrics) {
        let u = m.u(1.0);
        self.settings.preferred_kbd_size = sizes.preferred / u;
        if let Some(recent) = sizes.recent {
            self.settings.recent_kbd_size = recent / u;
        }
    }

    /// The current channel's MOD and PITCH values, which the wheels show.
    fn wheel_values(&self) -> [f32; 2] {
        let ch = self.song.channels.get(self.selected_channel);
        let pitch = ch.map_or(0.5, |c| c.params.pitch_bend());
        let cc1 = ch.and_then(|c| self.mod_wheel.get(&c.id)).copied();
        [cc1.unwrap_or(0.0), pitch]
    }

    /// The value of `target` on `channel`: the song's, or for MIDI CC 1 (channel param 8) the MOD wheel's, which the
    /// song does not keep; the master's tempo param is the song's tempo ([`crate::song::tempo_value`]).
    pub(super) fn param_value(&self, channel: usize, target: AutomationTarget) -> Option<f32> {
        if self.song.is_tempo(channel, target) {
            return Some(crate::song::tempo_value(self.song.tempo_bpm));
        }
        let ch = self.song.channels.get(channel)?;
        ch.param(target).or_else(|| {
            let cc1 = AutomationTarget {
                instance_id: 0,
                param: wheels::MOD,
            };
            (target == cc1).then(|| self.mod_wheel.get(&ch.id).copied().unwrap_or(0.0))
        })
    }

    /// The pitch and mod wheels' frame, on the current channel: the one pressed becomes the recent control, and each
    /// move goes live. PITCH is the channel's pitch wheel (param 4), a channel param edit; MOD is MIDI CC 1 (param 8),
    /// kept in [`Self::mod_wheel`], and also goes through the CC path, its value unquantised: every param linked to CC
    /// 1 follows it, as a linked controller's would.
    fn wheels(&mut self, wheels: wheels::Wheels, tick: u64) {
        let channel = self.selected_channel;
        let Some(id) = (self.song.channels.get(channel))
            .filter(|_| channel > 0)
            .map(|c| c.id)
        else {
            return;
        };
        if let Some(param) = wheels.pressed {
            self.recent = Some((
                id,
                AutomationTarget {
                    instance_id: 0,
                    param,
                },
            ));
            self.recent_held = true;
        }
        let mut edited = false;
        for (param, value) in wheels.moves {
            if param == wheels::PITCH {
                self.song.channels[channel].params.0[param as usize] = value;
                edited = true;
            } else {
                self.mod_wheel.insert(id, value);
                edited |= self.mod_wheel_cc(value);
            }
            self.audio.send(AudioCommand::SetParam {
                channel,
                instance_id: 0,
                index: param,
                value,
            });
        }
        if edited {
            self.song_edited(SongEdit::ChannelParam, tick);
        }
    }

    /// The recent control's change this frame, as (channel, target, old value, new value) ([`AutoRecord::moved`]).
    ///
    /// [`AutoRecord::moved`]: auto_record::AutoRecord::moved
    fn control_moved(&mut self) -> Option<(usize, AutomationTarget, f32, f32)> {
        let (channel, target) = self.recent_index()?;
        let ch = &self.song.channels[channel];
        let now = (self.param_value(channel, target)).map(|v| (ch.id, target, v));
        let old = self.auto_record.moved(now)?;
        Some((channel, target, old, now?.2))
    }

    /// REC is armed and the transport plays: a take runs, from the end of any count-in.
    fn recording(&self) -> bool {
        self.main_screen.armed && self.main_screen.playing
    }

    /// REC after the transport: any stop disarms, and the engine's metronome follows the arm (it sounds while
    /// armed and rolling). While a take runs and no count-in does, the frame's change `moved` is a point at song tick
    /// `tick` ([`AutoRecord::record`]), an ongoing edit; a take that ends (Stop, or REC disarmed) is one undo step.
    /// Notes and the pads' hits record in [`Self::note_record`].
    ///
    /// [`AutoRecord::record`]: auto_record::AutoRecord::record
    fn auto_record(
        &mut self,
        moved: Option<(usize, AutomationTarget, f32, f32)>,
        was_playing: bool,
        tick: u64,
    ) {
        if was_playing && !self.main_screen.playing {
            self.main_screen.armed = false;
        }
        // The arm goes out now; a disarm after the frame's takes ([`Self::send_disarm`]).
        if self.main_screen.armed && !self.auto_record.armed {
            self.send_arm(true);
        }
        if !self.recording() {
            // Its undo step closes with the notes' take, in one step ([`Self::record_notes`]).
            let stopped = self.auto_record.stop();
            self.auto_record.ended |= stopped;
            return;
        }
        let Some((channel, target, old, new)) = moved.filter(|_| !self.audio.counting_in()) else {
            return;
        };
        let current = self.current_track().filter(|&(c, _)| c == channel);
        let song = &mut self.song;
        let current = current.map(|(_, lane)| lane);
        let Some(added) =
            (self.auto_record).record(song, channel, target, (old, new), tick, current)
        else {
            return;
        };
        if let Some(lane) = added {
            // ponytail: a song copy per lane added (`lane_shifts` compares two songs); one per take and target at most.
            let mut before = self.song.clone();
            before.channels[channel].lanes.remove(lane);
            self.rekey(&tracks::lane_shifts(&before, &self.song));
        }
        self.song_edited(SongEdit::Ongoing, tick);
    }

    /// The engine's metronome follows REC's arm, and so does its song-end wrap: a recording does not wrap there. The
    /// tempo dialog's metronome flag clicks too, on its own command (`AudioCommand::MetronomeFlag`).
    fn send_arm(&mut self, on: bool) {
        self.auto_record.armed = on;
        let level = self.settings.metronome_level;
        self.audio.send(AudioCommand::Metronome { on, level });
    }

    /// REC disarmed reaches the engine after the frame's takes (`audio_record`, `record_notes`) left their clips: the
    /// engine wraps at the song end once disarmed, so the take's longer song goes out first.
    fn send_disarm(&mut self) {
        if !self.main_screen.armed && self.auto_record.armed {
            self.send_arm(false);
        }
    }

    /// Audio recording after REC and the transport, at the playhead `tick` after the frame's seek. The input follows
    /// REC ([`Self::open_input`]). A recording, automation or audio, starts with its take (`was_recording`: one ran in
    /// The last frame) and turns Preview off (one the frame's Play starts turned it off already, before the live
    /// notes); the audio one then takes the input every frame, and at its end leaves its clips with no undo step
    /// ([`History::amend`]). A take that reaches the longest length that loads back ([`Recorder::full`]) ends the
    /// recording in its own frame, with a message. An input of another width or rate (another tap,
    /// [`Self::open_input`]) ends it in its frame too, and REC armed starts the next on the new input.
    fn audio_record(&mut self, ctx: &egui::Context, tick: u64, was_recording: bool) {
        self.open_input(tick);
        if self.recording() && !was_recording {
            if self.preview.is_some() {
                self.preview_off(ctx, tick);
            }
            self.arm_recording();
        }
        let recording = self.recording();
        let Some(recorder) = &mut self.recorder else {
            return;
        };
        let retapped =
            (self.audio.input.as_ref()).is_some_and(|i| !Arc::ptr_eq(&i.tap, recorder.tap()));
        if recording && !retapped {
            let tempo = self.audio.engine_tempo(&self.song);
            let polled = recorder.poll(&self.song, tempo);
            if !recorder.full() {
                return self.record_error(polled);
            }
            // A take at the longest length that loads back ends the recording in this frame: REC disarms (the
            // transport plays on), the message shows, and the stop below leaves the clips. Left to the next frame, a
            // click on REC that is still down could arm it again first, and a new recorder would replace this one.
            let minutes = recorder.limit_minutes();
            self.main_screen.armed = false;
            self.navigate(Nav::Open(Overlay::Message {
                tag: RECORD_TAG,
                lines: vec![format!(
                    "Recording stopped. A take can be at most {minutes} minutes long."
                )],
                confirm: None,
                extra: None,
            }));
            self.record_error(polled);
        }
        let recorder = self.recorder.take().expect("a recording");
        let before = self.song.clone();
        let stopped = recorder.stop(&mut self.song);
        if self.song != before {
            self.rekey(&tracks::tracks_added(&before, &self.song));
            self.history.amend(&before, &self.song);
            // A take sets the song-changed flag: its placement is no loss without the question.
            self.files.song_changed();
            self.rebuild_as(tick, Handover::Edit);
        }
        self.record_error(stopped);
        if retapped && recording {
            self.arm_recording();
        }
    }

    /// Arms the recording that starts now, unless one is armed: its recorder arms the engine's tap.
    fn arm_recording(&mut self) {
        if self.recorder.is_none() {
            self.record_failed = false;
            self.recorder = self.new_recorder();
        }
    }

    /// The recorder of a recording starting now: the channels it takes ([`recording_channels`]), each from its stored
    /// input ports, else `Input 1`, into the recordings folder, from the engine's tap of the open input; each take
    /// drops L frames when the setting asks for it ([`compensates`]): this session's calibration, else the engine's
    /// buffer size ([`latency`]). `None` when nothing records: no input open, no channel to take, or no recordings
    /// folder (the web, in-memory files). A folder that fails shows the file error.
    fn new_recorder(&mut self) -> Option<Recorder> {
        let ring = self.input.ring()?.clone();
        let tap = (self.audio.input.as_ref())
            .filter(|i| Arc::ptr_eq(&i.ring, &ring))?
            .tap
            .clone();
        let multitrack = self.settings.multitrack_recording;
        let channels = recording_channels(
            &self.song,
            self.selected_channel,
            multitrack,
            ring.channels(),
        );
        let ports = self.input_ports();
        let source = |c: usize| {
            let id = self.song.channels[c].id;
            let stored = ports.iter().find(|p| p.0 == id).copied();
            let (_, first, second) = stored.unwrap_or((id, 0, None));
            let device = 0;
            (
                id,
                InputChoice {
                    device,
                    first,
                    second,
                },
            )
        };
        let sources: Vec<_> = channels.iter().map(|&c| source(c)).collect();
        if sources.is_empty() {
            return None;
        }
        let dir = match self.files.recordings_dir()? {
            Ok(dir) => dir,
            Err(err) => {
                self.record_error(Err(err));
                return None;
            }
        };
        let skip = match compensates(self.settings.latency_compensation) {
            true => latency(self.latency),
            false => 0,
        };
        Some(Recorder::new(tap, &self.song, &sources, dir, skip))
    }

    /// A recording's file error, shown once per recording as `Failed to save file` with the error under it. What was
    /// written stays, and the recording goes on.
    fn record_error(&mut self, result: std::io::Result<()>) {
        if let Err(err) = result
            && !std::mem::replace(&mut self.record_failed, true)
        {
            self.navigate(Nav::Open(Overlay::Message {
                tag: RECORD_TAG,
                lines: vec!["Failed to save file".into(), err.to_string()],
                confirm: None,
                extra: None,
            }));
        }
    }

    /// The input follows what needs it: open while REC is armed in a song with an Audio Record channel (only
    /// they record), while such a channel monitors, while the rack shows one, or while the calibration runs; closed
    /// otherwise. It opens on the input channel's device ([`input_channel`]: the default one without a stored input;
    /// The calibration's is always the default one) at the output's rate. The engines monitor the open ring with each
    /// channel's stored ports, and record it: one channel monitoring, the engine is rebuilt when they change, and
    /// with REC armed whenever the engine playing was not built with it. An open that fails is not tried again until
    /// what needs it changes (REC armed again, another device) or an `Audio Record` channel is added
    /// ([`Self::input_failed`]); for REC it shows the permission message (natively: the web has no input).
    fn open_input(&mut self, tick: u64) {
        let records = |c: &Channel| c.kind == ChannelKind::AudioRecord;
        let channels = &self.song.channels;
        let rack = self.rack_channel();
        let calibrating = matches!(self.calibration, Some(input::Calibrating::Running(_)));
        let armed = self.main_screen.armed && channels.iter().any(records);
        let monitors = channels
            .iter()
            .any(|c| records(c) && c.file.input_monitoring);
        let wanted = armed
            || monitors
            || rack.and_then(|c| channels.get(c)).is_some_and(records)
            || calibrating;
        // The calibration listens to the first input device.
        let device = input_channel(&self.song, rack, self.selected_channel)
            .filter(|_| !calibrating)
            .and_then(|c| self.routing.get(&self.song, &self.song.channels[c]))
            .map(|route| route.device.clone());
        let need = wanted.then(|| (device.clone(), armed));
        if need != self.input_need {
            self.input_failed = false;
            self.input_need = need;
        }
        if !wanted {
            self.input.close();
        } else if let Some(rate) = self.audio.sample_rate().filter(|_| !self.input_failed)
            && self.input.open(device.as_deref(), rate).is_err()
        {
            self.input_failed = true;
            if armed && !cfg!(target_arch = "wasm32") {
                self.navigate(Nav::Open(Overlay::Message {
                    tag: INPUT_TAG,
                    lines: vec![if cfg!(target_os = "android") {
                        "Audio recording is not available in this Android preview.".into()
                    } else {
                        MIC_PERMISSION.into()
                    }],
                    confirm: None,
                    extra: None,
                }));
            }
        }
        let ring = self.input.ring().cloned();
        let ports = self.input_ports();
        let built = (self.audio.input.as_ref()).map(|i| (Arc::as_ptr(&i.ring), &i.ports));
        let changed = ring.as_ref().map(|r| (Arc::as_ptr(r), &ports)) != built;
        if changed {
            // An input of the same width and rate keeps its tap (all a tap is made of), so a take records on through a
            // port pick (the engine's take goes on), a reopen or another device (the new ring's engine starts a take on
            // it, the recorder a clip there). Another width or rate gets a new tap, which ends the recording
            // ([`Self::audio_record`]).
            let tap = (self.audio.input.as_ref())
                .filter(|i| {
                    let format = (i.ring.channels(), i.ring.rate());
                    ring.as_ref()
                        .is_some_and(|r| (r.channels(), r.rate()) == format)
                })
                .map(|i| i.tap.clone());
            self.audio.input = ring.map(|ring| match tap {
                Some(tap) => LiveInput { ring, ports, tap },
                None => LiveInput::new(ring, ports),
            });
        }
        // The monitors' new ports or ring, a running measurement's new ring (its first engine the calibration builds),
        // and an armed REC need an engine built with the open input: an edit's engine takes over (the transport, the
        // sound, a running take and the measurement go on).
        let measuring = matches!(self.calibration, Some(input::Calibrating::Running(Some(_))));
        let unrecorded = armed && self.audio.input.is_some() && !self.audio.records();
        if (changed && (monitors || measuring)) || (unrecorded && self.audio.running()) {
            self.rebuild_as(tick, Handover::Edit);
        }
    }

    /// The performance monitor's shared data (`ui::performance`), brought up to date once per drawn main-UI frame from
    /// The audio snapshot and the song, and not while a loading screen is up; the transport's CPU meter and tooltips
    /// read it too. The graph moves on its own while the song plays; the bar eases and the peak falls for a while
    /// after, which frames must show if a view of them is up.
    fn update_performance(&mut self, ctx: &egui::Context, snapshot: Option<&OutputSnapshot>) {
        let loading = (self.nav.overlays.iter()).any(|o| matches!(o, Overlay::Busy { .. }));
        let live = snapshot.filter(|s| !s.stream_failed);
        let reading = performance::Reading {
            load: live.map_or(0.0, |s| s.cpu_load),
            playing: self.main_screen.playing,
            tick: snapshot.map_or(0, |s| s.transport_tick),
            song_beats: song_end_beats(&self.song),
            underruns: live.map_or(0, |s| s.underruns),
        };
        let memory = self.main_screen.ram_text();
        let memory = if memory.is_empty() || memory.starts_with("--") {
            "-".to_owned()
        } else {
            memory.to_owned()
        };
        let p = &mut self.slots.performance;
        if !loading {
            p.update(self.now, reading);
        }
        let (plugin_lock_misses, recovered_waits) = crate::plugins::lock_stats();
        p.set_plugin_lock_stats(plugin_lock_misses, recovered_waits);
        p.app_memory = memory;
        self.main_screen.meter = Load {
            peak: p.peak,
            smoothed: p.smoothed,
        };
        let page = (self.nav.overlays.iter())
            .any(|o| matches!(o, Overlay::MainMenu(views::MenuTab::Project)));
        if page && p.moving() {
            ctx.request_repaint();
        }
    }

    /// The takes of the recording under way, for the playlist's live bars: each clip being recorded, notes, hits or
    /// automation, from its start to the playhead `tick` (an automation clip the playhead is not past since a loop wrap
    /// shows as itself); and each audio take on its track, from its start to the playhead while it runs, else to as far
    /// as its frames reach. A take bound for a track still to be made shows once the recording stops (the
    /// playlist has no row for it yet).
    fn live_takes(&self, tick: u64) -> Vec<LiveTake> {
        let channels = &self.song.channels;
        let auto = (self.auto_record.clips()).filter_map(|(id, lane, clip)| {
            let channel = channels.iter().position(|c| c.id == id)?;
            Some((channel, lane, clip))
        });
        let clips = (self.note_record.clip(&self.song).into_iter()).chain(auto);
        let mut takes: Vec<LiveTake> = clips
            .filter_map(|(channel, lane, id)| {
                let lane_clips = &channels[channel].lanes.get(lane)?.clips;
                let start = lane_clips.iter().find(|c| c.id == id)?.start_tick;
                (tick >= u64::from(start)).then_some(LiveTake {
                    channel,
                    lane,
                    start: f64::from(start),
                    end: tick as f64,
                    clip: Some(id),
                })
            })
            .collect();
        let Some(recorder) = &self.recorder else {
            return takes;
        };
        let take = |t: &Take| {
            let channel = self
                .song
                .channels
                .iter()
                .position(|c| c.id == t.channel_id)?;
            let ch = &self.song.channels[channel];
            let made = t.lane < tracks::new_track(ch) && ch.lanes[t.lane].kind == LaneKind::Audio;
            let start = t.start_tick as f64;
            // A running take's bar ends at the playhead, the engine's song position, with no latency offset. Right
            // after a loop wrap the playhead is behind it until the recorder sees the next take: that frame it ends
            // where its frames reach.
            let end = match t.running() && tick as f64 >= start {
                true => tick as f64,
                false => start + t.ticks,
            };
            made.then_some(LiveTake {
                channel,
                lane: t.lane,
                start,
                end,
                clip: None,
            })
        };
        takes.extend(recorder.takes().iter().filter_map(take));
        takes
    }

    /// The web page was hidden. It runs no frame (the browser stops requestAnimationFrame) to see its focus loss, so
    /// `lib.rs` calls this from `visibilitychange`: every note stops now (the shown roll's too,
    /// [`PianoRoll::silence`]), and the next frame counts as a focus loss, so a press held over the hide holds nothing
    /// until its release (`ui::primary_down_live`). Not left to eframe's own focus loss: it queues none while the
    /// canvas is unfocused, as after a first touch before any release.
    pub fn on_hidden(&mut self) {
        self.stop_notes();
        // The roll shown: what it sounds, as at its hide.
        if let View::PianoRoll(channel) = self.shown
            && let Some(roll) = self.slots.piano_roll.rolls.get_mut(&channel)
        {
            for key in roll.silence() {
                self.audio.send(AudioCommand::NoteOff { channel, key });
            }
        }
        // Not in `stop_notes`, which a song load runs too: a load keeps the typed notes.
        for (channel, key, _) in self.typed.lose_focus() {
            self.key_off(channel, key);
        }
        for overlay in &self.nav.overlays {
            if let Overlay::PresetPicker { channel, .. } = *overlay {
                let picker = &mut self.slots.preset_picker;
                views::preset_picker::stop_notes(picker, &mut self.audio, channel);
            }
        }
        self.hidden = true;
    }

    /// The song has changes its file does not hold: the web page asks before it is closed or reloaded (`lib.rs`'s
    /// `beforeunload`).
    pub fn unsaved(&self) -> bool {
        self.files.modified()
    }

    /// Stops every note the keyboard, the pads and MIDI sound: held, or tapped within its gate.
    fn stop_notes(&mut self) {
        views::drum_editor::stop_notes(&mut self.slots.drum_editor, &mut self.audio);
        self.tap_off = None;
        self.play_key(&[]);
        self.release_midi();
        for note in self.main_screen.chords.release_all() {
            self.chord_note(note);
        }
        self.gate_offs(true);
    }

    /// Adds a channel and selects it, a final edit. `tick` is the transport position before the edit. `Rack` opens
    /// the rack preset picker instead, whose pick adds the channel ([`views::preset_picker`]).
    fn add_channel(&mut self, kind: ChannelKind, tick: u64) {
        if kind == ChannelKind::Rack {
            return self.navigate(rack_channel_picker(None));
        }
        // At the end: [`Self::insert_channel`], which a Drums channel's rule goes through.
        self.insert_channel(self.song.channels.len(), kind, tick);
    }

    #[cfg(debug_assertions)]
    fn icon_sheet(&mut self, ctx: &egui::Context, metrics: &Metrics) {
        if ctx.input(|i| i.key_pressed(egui::Key::F2)) {
            self.show_icon_sheet = !self.show_icon_sheet;
        }
        if self.show_icon_sheet {
            let colors = &self.theme.colors;
            crate::icons::draw_debug_sheet(
                &ctx.debug_painter(),
                ctx.content_rect(),
                colors.text.to_color32(),
                colors.background.to_color32(),
                metrics,
            );
        }
    }
}

/// Opens the rack preset picker for a new Rack channel, at the end or at `at` (`Insert Channel`).
/// The panel's sizes after the `toggleKeyboardPanel` shortcut on a screen `screen_h` points tall, the collapsed height
/// `c`: the tap toggle's height ([`panel_resize::toggled`], through the release snap), also the recent height when it
/// is over the bar alone.
fn toggle_snapped(settings: &Settings, c: f32, screen_h: f32, m: &Metrics) -> panel_resize::Sizes {
    let u = m.u(1.0);
    let (preferred, recent) = (
        settings.preferred_kbd_size * u,
        settings.recent_kbd_size * u,
    );
    let h = panel_resize::toggled(preferred, recent, c, screen_h, m);
    panel_resize::Sizes::settled(h, c)
}

fn rack_channel_picker(at: Option<usize>) -> Nav {
    let target = crate::ui::rack::PresetTarget::NewChannel(at);
    Nav::Open(Overlay::PresetPicker { channel: 0, target })
}

/// One-time egui setup: the app font, and no built-in keyboard zoom (the UI scale replaces it).
fn setup_context(ctx: &egui::Context) {
    fonts::install(ctx);
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
}

impl eframe::App for HoneydewApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "android")]
        crate::android_text::begin_frame(ctx);
        let sound_mode = self.settings.sound_mode;
        let low_latency = self.settings.low_latency;
        self.frame(ctx, |song, samples| {
            open_device_with_mode(song, samples, buffer_mode(sound_mode, low_latency))
        });
        #[cfg(target_os = "android")]
        crate::android_text::end_frame(ctx);
    }

    /// The settings not flushed yet (a drag's, or a failed flush's) get a last try (native only: the web has no exit).
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // The plugins' editors close before their windows go.
        crate::plugins::song_replaced();
        if self.settings_unflushed
            && let Some(storage) = self.storage.as_deref_mut()
        {
            storage.flush();
        }
    }

    /// For `WebRunner::app_mut` (`lib.rs`).
    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl HoneydewApp {
    /// One frame. `open` starts the audio output for the song; it is called on a click (its press natively,
    /// its release on the web) while the audio is not running.
    pub fn frame(
        &mut self,
        ctx: &egui::Context,
        open: impl FnOnce(&Song, &SampleCache) -> Option<Box<dyn AudioSink>>,
    ) {
        // Before anything reads the input: the latches see the hide as the focus loss it is.
        let hidden = std::mem::take(&mut self.hidden);
        if hidden {
            ctx.input_mut(|i| i.events.push(egui::Event::WindowFocused(false)));
        }
        self.now = ctx.input(|i| i.time);
        // A song new to the app keeps its view zooms per U: a file without the zoom-units flag has them per device
        // pixel, converted at this frame's U, so a save writes the current layout. A replaced song is converted as it
        // comes in (`SongEdit::Replaced`); the one the app starts with on its first frame, where its undo history
        // starts.
        self.u_px = Metrics::new(ctx, self.settings.ui_scale).u_px();
        if !self.song.file.zooms_per_u() {
            self.song.zooms_to_units(f64::from(self.u_px));
            self.history = History::new(&self.song);
        }
        // The open rack follows the current channel ([`Self::rack_follows`]).
        let was_current = self.current_channel();
        // The focus keys and the focused control's, before anything else reads the keys (`ui::focus`).
        focus::begin_frame(ctx);
        // The snapshot first: a stream that failed while egui was idle (no frames) must count as not running for
        // this click. The start is before the screen, so a press that lands on a key is heard natively.
        let mut snapshot = self.audio.poll();
        if !self.audio.running() && ctx.input(|i| audio_gesture(i, START_ON_RELEASE)) {
            self.audio
                .set_low_latency_preference(self.settings.low_latency);
            self.audio
                .set_sound_mode_preference(self.settings.sound_mode);
            let solo = self.solo();
            let song = solo.as_ref().unwrap_or(&self.song);
            let read = sample_cache::reader(&self.files);
            let playing = self.main_screen.playing;
            (self.audio).start_keeping(song, &self.song, playing, &read, open);
            snapshot = self.audio.poll();
            drop(read);
            // Before the starting press's key: the new engine starts from each channel's first lane.
            self.sent_lane = None;
            self.live_lane();
        }
        // Last frame's leftovers go before this frame's commands.
        self.audio.flush();
        // What a view gone between frames left under way ends first, ahead of the frame's held live notes.
        if let Some(edit) = self.left_view(ctx) {
            self.song_edited(edit, snapshot.as_ref().map_or(0, |s| s.transport_tick));
        }
        // This frame's live notes wait for the transport's Play or Stop below, which goes out first: while REC is armed
        // and the transport stopped as the frame starts, a press starts playback; and presses queued with a Play start
        // (its tap, Space) go out after its reset. A Stop or an arming in this frame starts nothing.
        let start_armed = self.main_screen.armed && !self.main_screen.playing;
        self.audio.hold_notes(true);
        self.midi_input(
            ctx,
            hidden,
            snapshot.as_ref().map_or(0, |s| s.transport_tick),
        );
        self.plugins_frame(ctx, snapshot.as_ref().map_or(0, |s| s.transport_tick));
        let was_playing = self.main_screen.playing;
        let was_recording = self.recording();
        // A slice of the export under way, which stops the transport (`app/export.rs`).
        self.export_slice(ctx);
        // The latency calibration's test stops the transport and holds it.
        if matches!(self.calibration, Some(input::Calibrating::Running(_))) {
            self.main_screen.playing = false;
        }
        self.main_screen.takes = self.live_takes(snapshot.as_ref().map_or(0, |s| s.transport_tick));
        self.update_performance(ctx, snapshot.as_ref());
        // The playhead the editors draw this frame, smooth between the snapshot's block steps, never past where the
        // engine wraps: its loop's end, else the song length (not while REC is armed: a recording plays on).
        let bpm = self.audio.engine_tempo(&self.song);
        let end = (self.sent_loop.filter(|(start, end)| end > start))
            .map(|(_, end)| u64::from(end))
            .or_else(|| {
                let beats = crate::render::song_length_beats(&self.song);
                (!self.main_screen.armed).then(|| (beats * f64::from(crate::song::PPQ)) as u64)
            });
        (self.audio).advance_playhead(self.now, self.main_screen.playing, bpm, end);
        // While the song plays, its automation moves the controls, at the drawn playhead; not in Preview, where the
        // engine plays one clip alone, nor under the main menu or a Save dialog, which save and render the song itself.
        let saving = (self.nav.overlays.iter())
            .any(|o| matches!(o, Overlay::MainMenu(_) | Overlay::SaveDialog { .. }));
        let shows = self.main_screen.playing && self.preview.is_none() && !saving;
        self.recent_held &= ctx.input(|i| i.pointer.any_down());
        let mut shown_bpm = self.song.tempo_bpm;
        if let Some(s) = snapshot.as_ref().filter(|_| shows) {
            let tick = s.transport_tick + self.audio.playhead_lead();
            self.show_automation(tick);
            // The BPM button shows the tempo the master's tempo automation has set.
            shown_bpm = self.automated_tempo(tick).unwrap_or(shown_bpm);
        }
        self.engine_keys = snapshot
            .as_ref()
            .map_or_else(Vec::new, |s| s.sounding.clone());
        // Once per frame; screens take it by reference.
        let metrics = Metrics::new(ctx, self.settings.ui_scale);
        crate::ui::anim::set_mode(ctx, self.settings.animations, self.settings.inertia);
        // As the frame starts; the settings tab edits `self.settings` meanwhile.
        let shown = self.settings.clone();
        // The roll's rows tint for the live notes and the engine's keys alike: each channel's, for the roll to take its
        // own as it draws, after the editor follows the playlist.
        let lit: Vec<u128> = (0..self.song.channels.len())
            .map(|c| self.sounding(c) | self.engine_keys.get(c).copied().unwrap_or(0))
            .collect();
        let cx = UiCtx {
            theme: &self.theme,
            m: &metrics,
        };
        let tracks = track_rows(&self.song);
        let answer = self.nav.answer();
        // The piano's scroll and zoom are the current channel's.
        (self.main_screen).keys_for(self.song.channels.get(self.selected_channel).map(|c| c.id));
        let (key_input, piano) = (self.key_input(), self.piano_keys());
        // As of the last frame's editor state; an editor that moved to another clip moves Preview after the screens.
        let previewing = (self.preview)
            .is_some_and(|k| preview::current(self.nav.view, &self.slots, &self.song) == Some(k));
        let mcp_status = self.mcp_status();
        self.slots.ctrl_menu.recent = self.ctrl_recent();
        // The sample row's clip, found after the view.
        let mut row_clip = None;
        // SETTINGS' `Input device:` dropdown opened this frame.
        let mut midi_rescan = false;
        let wheels = self.wheel_values();
        // The chord keyboard's set, transposed to the key, for the frame.
        let chord_set = views::keyboard_settings::chord_set(&self.song);
        let scale_name = crate::ui::main_screen::scale_name(&self.song);
        let strip_n = crate::ui::keyboard_settings::strip_controls(
            views::keyboard_settings::notes_channel(&self.song, self.selected_channel),
            views::drum_editor::pads_channel(&self.song, self.selected_channel).is_some(),
            self.settings.keyboard_kind(),
        );
        let chords = (!piano).then(|| self.chord_input(&chord_set, key_input.enabled));
        let (mut actions, mut navs, overlay_navs, edit) = egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(self.theme.colors.background.to_color32()))
            .show(ctx, |ui| {
                // The keyboard panel animates to its preferred height (`preferredKbdSize`).
                let screen = ui.max_rect();
                // The hand resize before the layout, so the panel follows the finger in this frame. The rack never
                // covers the bar; any other overlay but the strip covers everything.
                let over = |o: &Overlay| !matches!(o, Overlay::Rack(_) | Overlay::KeyboardSettings);
                let covered = self.nav.overlays.iter().any(over);
                let grabbed = self
                    .main_screen
                    .grab(ui, &metrics, covered, screen.height());
                let (mut regions, strip) = panel::panel_layout(
                    ui.ctx(),
                    screen,
                    grabbed,
                    strip_n,
                    &mut self.settings,
                    &mut self.main_screen,
                    &mut self.nav.overlays,
                    &metrics,
                );
                // The rack slides in and out a step per frame; the view is laid out left of it.
                let racked = (self.nav.overlays.iter()).any(|o| matches!(o, Overlay::Rack(_)));
                let width = regions.area.width() * metrics.ppp();
                let open = self.slots.rack.slide(racked, width, ui.ctx());
                if open > 0.0 && open < 1.0 {
                    ui.ctx().request_repaint();
                }
                regions = regions.with_rack(open, &metrics);
                // The keyboard settings strip takes its height off the top of the keys.
                regions = regions.with_strip(strip, &metrics);
                let input = ScreenInput {
                    view: self.nav.view,
                    tracks: &tracks,
                    selected: self.selected_channel,
                    settings: &shown,
                    audio: snapshot.as_ref(),
                    bpm: shown_bpm,
                    history: (self.history.can_undo(), self.history.can_redo()),
                    preview: self.preview,
                    can_preview: false,
                    scale_name: scale_name.as_deref(),
                    sample_row: None,
                    pads: false,
                    keyboard: key_input,
                    piano,
                    covered: views::covered(&self.nav.overlays, &regions, ui),
                    wheels,
                    chords,
                };
                // Keep the Settings MCP status in sync with the bridge that is actually running. The bridge is
                // restarted after settings changes in `keep_settings`, so the next frame reflects bind failures too.
                self.slots.settings_tab.set_mcp_status(mcp_status);
                let mut app = AppCtx {
                    cx,
                    regions,
                    song: &mut self.song,
                    audio: &mut self.audio,
                    selected: &mut self.selected_channel,
                    snapshot: snapshot.as_ref(),
                    playing: self.main_screen.playing,
                    armed: self.main_screen.armed,
                    previewing,
                    middle_c_is_c4: shown.middle_c_is_c4,
                    settings: &mut self.settings,
                    ghost_notes: shown.ghost_notes,
                    files: &mut self.files,
                    edit: None,
                    answer,
                    covered: input.covered,
                    playhead: None,
                    seek: None,
                    midi_devices: self.midi.link.ports(),
                    midi_rescan: false,
                };
                // The view, then the transport and keyboard over it, then the overlays bottom to top: what is
                // drawn later takes the input first.
                let mut actions = ScreenActions {
                    panel_sizes: grabbed,
                    ..ScreenActions::default()
                };
                let mut navs = Vec::new();
                // A clip editor opens under the playlist (`ui::split`); the mixer replaces both.
                let editor = !matches!(self.nav.view, View::Playlist | View::Mixer);
                let view_rect = app.regions.editor;
                let split = &mut self.main_screen.split;
                let step = crate::ui::anim::step(ui.ctx(), crate::ui::anim::PANEL_KEEP);
                if split.step(editor, view_rect.height() * metrics.ppp(), step) {
                    ui.ctx().request_repaint();
                }
                let split = *split;
                // The mixer, the playlist and the bottom editor are focus containers (`showMainMixer`, `focusEditor`).
                // The one holding the focus takes the editing and zoom keys while nothing covers the view (the strip
                // and the rack do not, as for the pointer; the focused control first, so its Escape deselects before
                // either closes); with the focus at the root, the open editor, else the playlist, so copy
                // and paste work before any tap (`ui/edit_keys.rs`).
                let ctx = ui.ctx().clone();
                let (free, root) = (!covered, focus::focused(&ctx).is_none());
                // Says whether container `id`, drawn next, holds the keys; gives its id.
                let keys = |id: &str, at_root: bool| {
                    let held = focus::within(&ctx, egui::Id::new(id)) || root && at_root;
                    crate::ui::edit_keys::set_focus(&ctx, free && held);
                    egui::Id::new(id)
                };
                if self.nav.view == View::Mixer {
                    focus::container(ui, keys(focus::MIXER, true), view_rect, |ui| {
                        navs.extend(self.slots.view(View::Mixer, ui, &mut app));
                    });
                } else {
                    let (top, bottom, shown) = split.layout(view_rect, &metrics);
                    let top = if editor { top } else { view_rect };
                    app.regions.editor = top;
                    // The editors may fill the window. A playlist under its ruler's height is 0 (`ui::split`) and not
                    // drawn: nothing of it takes touches or the focus over the editor.
                    if top.height() > 0.0 {
                        focus::container(ui, keys(focus::PLAYLIST, !editor), top, |ui| {
                            self.main_screen.playlist(ui, &mut app, input, &mut actions);
                        });
                    }
                    // The bottom editor shows the current track's current clip, and follows it: a tap, `Edit`, a double
                    // tap or `Select clip` re-targets it, after the playlist's frame. It collapses whenever the current
                    // track has no current clip.
                    let current = self.main_screen.clips.current_clip(&app);
                    let followed = || current.and_then(|r| self.slots.follow(r.into(), app.song));
                    match editor.then(followed) {
                        None => {}
                        // At once, so that no frame shows the editor's chrome (Undo, Preview) without it.
                        Some(None) => {
                            self.nav.view = View::Playlist;
                            navs.push(Nav::Collapse);
                        }
                        Some(Some(view)) => {
                            // Followed to another view, which draws before `left_view`: the key column's press is
                            // stale there at once.
                            if view != self.shown {
                                crate::ui::let_go_press(ui.ctx(), crate::ui::piano_roll::keys_id());
                            }
                            self.nav.view = view;
                            if let View::PianoRoll(channel) = view {
                                self.slots.piano_roll.sounding =
                                    lit.get(channel).copied().unwrap_or(0);
                            }
                            if let View::AutomationEditor(channel) = view {
                                let editor = &mut self.slots.automation_editor;
                                let key = editor.current(app.song, channel);
                                editor.init = key.and_then(|k| {
                                    automation::automated_init(&self.automated, app.song, k)
                                });
                            }
                            app.regions.editor = bottom;
                            focus::container(ui, keys(focus::EDITOR, true), shown, |ui| {
                                let clip = ui.clip_rect();
                                ui.set_clip_rect(clip.intersect(shown));
                                navs.extend(self.slots.view(view, ui, &mut app));
                                ui.set_clip_rect(clip);
                                // The colour band and its close ribbon over the editor, the shadow over the playlist.
                                // The ribbon's tap: the playlist alone again, the clip kept; a focus in the editor goes
                                // back to the playlist.
                                let channel = app.song.channels.get(*app.selected);
                                let color = channel.map_or(cx.theme.colors.menu_fill, |c| {
                                    crate::theme::channel_color(c.hue, cx.theme)
                                });
                                split::band(
                                    ui.painter(),
                                    view_rect,
                                    shown.top(),
                                    color.to_color32(),
                                    &metrics,
                                );
                                split::shadow(ui.painter(), top, &metrics);
                                let split = &mut self.main_screen.split;
                                let screen = app.regions.screen.right();
                                if split::ribbon(
                                    ui,
                                    split,
                                    (view_rect, shown.top()),
                                    screen,
                                    color,
                                    &cx,
                                ) {
                                    navs.push(Nav::Collapse);
                                    if focus::within(ui.ctx(), egui::Id::new(focus::EDITOR)) {
                                        focus::move_to(ui.ctx(), egui::Id::new(focus::PLAYLIST));
                                    }
                                }
                            });
                        }
                    }
                }
                // Escape belongs to the topmost popup or overlay: a popup the view shows over the overlays has taken
                // it (`popup::show`); otherwise it is held back from the overlays below the topmost one.
                let escape = !self.nav.overlays.is_empty() && views::take_escape(ui);
                // After the view, which makes its editor's state: SOLO CLIP shows while the current track has a
                // current clip, in every view.
                let current = (self.main_screen.clips).clip_for(app.song, *app.selected);
                row_clip = sample_row::current(app.song, *app.selected, &self.slots.audio_editor);
                let row_slot = |key: ClipKey| key.clip(app.song)?.file.sample.as_deref();
                let input = ScreenInput {
                    // As the view left it: the editor followed the playlist, or collapsed.
                    view: self.nav.view,
                    can_preview: current.is_some(),
                    sample_row: row_clip.map(|key| sample_row::Row::of(row_slot(key))),
                    pads: views::drum_editor::pads_channel(app.song, *app.selected).is_some(),
                    ..input
                };
                // A tap on an editor's playhead knob plays (`AppCtx::play`): before the transport's buttons, which may
                // stop it this frame.
                self.main_screen.playing |= app.playing;
                self.main_screen
                    .chrome(ui, &cx, input, &regions, &mut actions);
                views::drum_editor::after_chrome(&mut self.slots.drum_editor, ui, &mut app);
                // The corner buttons go over the rack and under the other overlays.
                let hue = app.song.channels.get(input.selected).map_or(0.0, |c| c.hue);
                let ink = crate::theme::channel_color(hue, cx.theme).to_color32();
                let corner = |ui: &mut egui::Ui| {
                    let (selected, screen) = ((input.selected, ink), &mut self.main_screen);
                    screen.corner_buttons(ui, &cx, selected, &regions, &mut actions);
                };
                let overlay_navs =
                    (self.slots).overlays(&mut self.nav.overlays, ui, &mut app, escape, corner);
                #[cfg(test)]
                match self.test_edit.take() {
                    Some(SongEdit::Replaced) => app.song_replaced(),
                    Some(SongEdit::Param) => app.param_edited(),
                    Some(SongEdit::ChannelParam) => app.channel_param_edited(),
                    Some(edit) => app.song_edited(edit == SongEdit::Final),
                    None => {}
                }
                navs.extend(actions.nav.take());
                actions.playhead = app.playhead;
                actions.seek = app.seek.or(actions.seek);
                midi_rescan = app.midi_rescan;
                (actions, navs, overlay_navs, app.edit)
            })
            .inner;
        // Before anything else reads the song: its own values, for the edits, the undo steps and the engines.
        self.hide_automation(edit == Some(SongEdit::Replaced));
        if midi_rescan {
            self.midi_rescan();
        }
        // The transport bar's `CPU` / `RAM` labels' tap: the menu is opening on PROJECT; its Performance block scrolls
        // into view as it draws.
        if std::mem::take(&mut self.main_screen.performance_tap) {
            self.slots.main_menu.show_performance();
        }
        // As drawn: the channel indices the bound controls' touches name.
        let add_automation = self.take_touches();
        let tick = snapshot.as_ref().map_or(0, |s| s.transport_tick);
        #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
        self.control_frame(tick);
        // The chord editor's edit: the set in use with the new chord, marked edited, an undo step.
        if let Some((strip, chord)) = actions.chord_edit.take() {
            let mut set = views::keyboard_settings::chord_set(&self.song);
            if let Some(c) = set.chords.get_mut(strip) {
                *c = chord;
                let k = &mut self.song.file.keyboard;
                (k.chord_json, k.set_state) = (set.to_json(), crate::song::SET_EDITED);
                self.song_edited(SongEdit::Param, tick);
            }
        }
        // The chord keyboard's notes, on the current channel's live input.
        for note in std::mem::take(&mut actions.chord_notes) {
            self.chord_note(note);
        }
        // The wheels, before the recent control's change is read: their press makes them recent.
        self.wheels(std::mem::take(&mut actions.wheels), tick);
        // REC: the recent control's change this frame, or a live key's press (the pads' this frame, the keys' since the
        // last check); while armed either starts a stopped transport.
        let moved = self.control_moved();
        for key in views::drum_editor::take_hits(&mut self.slots.drum_editor) {
            self.live_key(key);
        }
        // The keymap's global actions, after the screens' own keys (`app/keys.rs`).
        self.global_keys(ctx, &mut actions.rewind, &mut navs);
        // After the screens: a focused text field keeps its own Ctrl+Z.
        let step = actions.step.or_else(|| history::shortcut(ctx));
        // The header's channel, then the keyboard's and the typed keys: every press of the frame can start playback
        // in this frame (its note goes out right after that Play).
        if let Some(index) = actions.header {
            self.selected_channel = index;
        }
        // The keyboard holds no key over a focus loss (`ui::fingers`): its notes stop now, not at the next click.
        let held = actions.keys.held;
        let now = ctx.input(|i| i.time);
        if !actions.keys.tapped.is_empty() && held.is_empty() {
            self.play_key(&actions.keys.tapped);
            self.tap_off = Some(now + TAP_GATE);
        }
        // A hidden web page runs no frame to end a gate: the focus-loss frame stops every note at once.
        let lost = ctx.input(crate::ui::focus_lost);
        match self.tap_off {
            // A tapped note sounds until its gate ends, another key is held, or the focus goes.
            Some(off) if held.is_empty() && now < off && !lost => {
                ctx.request_repaint_after_secs((off - now) as f32);
            }
            _ => {
                self.tap_off = None;
                self.play_key(&held);
            }
        }
        // After the screens, which took the presses their shortcuts and text fields own (`app/typed.rs`).
        self.typed_notes(ctx, tick);
        // After every live key of the frame: the note-offs whose tap gate ended, all of them on a focus loss. The frame's
        // notes are on from here.
        if let Some(next) = self.gate_offs(lost) {
            ctx.request_repaint_after_secs((next - self.now) as f32);
        }
        self.fresh.clear();
        let pressed = std::mem::take(&mut self.note_record.start);
        if (moved.is_some() || pressed) && start_armed {
            self.main_screen.playing = true;
        }
        // A tap on the step grid's playhead knob plays.
        if std::mem::take(&mut self.slots.drum_editor.step.play) {
            self.main_screen.playing = true;
        }
        // The transport: a ruler tap seeks, or stops a playing transport; REV stops and goes back to the play-start
        // point, or from there to the song start.
        let screen = &mut self.main_screen;
        let mut seek = actions.seek;
        if let Some(next) = self.bar_jump.take() {
            seek = Some(keys::bar_jump(&self.song, tick, next));
        }
        // A ruler tap that stops returns to the play-start point, as REV's first press.
        let stop_tap = actions.playhead.is_some() && was_playing;
        if let Some(at) = actions.playhead.filter(|_| !was_playing) {
            seek = Some(at);
        }
        // REV at the play-start point goes to beat 0 and resets the sound, after the Stop and the seek.
        let reset_sound = actions.rewind && tick == screen.play_start;
        if actions.rewind || stop_tap {
            screen.playing = false;
            seek = Some(if reset_sound { 0 } else { screen.play_start });
        }
        let (playing, armed) = (screen.playing, screen.armed);
        if playing != was_playing {
            // Play while armed counts in first, with `Enable count-in when recording` on.
            if playing && armed && self.settings.count_in {
                self.audio
                    .send(AudioCommand::CountIn(auto_record::COUNT_IN_BEATS));
            }
            // Play starts from the song position, the play-start point; Stop stays where it is.
            if playing {
                screen.play_start = tick;
            }
            // A recording arms the engine's tap before its Play: its take starts at the Play position.
            if playing && armed {
                self.arm_recording();
            }
            self.audio.send(if playing {
                AudioCommand::Play
            } else {
                AudioCommand::Stop
            });
        }
        let screen = &mut self.main_screen;
        // Every seek moves the play-start point, and one while playing turns recording off. The rest of the frame
        // works at the new position.
        let jumped = seek.is_some();
        let tick = seek.map_or(tick, |at| {
            self.audio.send(AudioCommand::Seek(at));
            screen.play_start = at;
            screen.armed &= !playing;
            at
        });
        if reset_sound {
            self.audio.send(AudioCommand::ResetSound);
        }
        // Play cuts the live keys held and stops the pickers' sample; then the frame's live notes go out.
        if playing && !was_playing {
            self.cut_live_keys();
            self.slots.stop_samples();
            // A recording this Play starts turns Preview off (`audio_record`) before the frame's live notes go out: its
            // handover silences none of them, and the key that starts it keeps sounding. A a Preview tap in
            // this frame changes nothing, the recording turning Preview off.
            if self.recording() {
                actions.preview = false;
                if self.preview.take().is_some() {
                    self.rebuild_as(tick, Handover::Preview { seek: None });
                }
            }
        }
        self.audio.hold_notes(false);
        self.rack_input();
        self.auto_record(moved, was_playing, tick);
        self.audio_record(ctx, tick, was_recording);
        self.calibrate(ctx, snapshot.as_ref().and_then(|s| s.ping), tick);
        // After every live key of the frame, and the channel the header picked.
        self.record_notes(jumped, tick);
        self.send_disarm();
        if let Some(pick) = actions.add {
            match pick {
                AddChannelPick::Kind(kind) => self.add_channel(kind, tick),
                AddChannelPick::Plugin(uid) => {
                    self.insert_plugin_channel(self.song.channels.len(), uid, tick)
                }
            }
        }
        if let Some((at, pick)) = actions.insert {
            match pick {
                AddChannelPick::Kind(kind) => self.insert_channel(at, kind, tick),
                AddChannelPick::Plugin(uid) => self.insert_plugin_channel(at, uid, tick),
            }
        }
        if let Some((channel, lane, op)) = actions.track {
            self.track_op(channel, lane, op, tick);
        }
        // A track header's drop: the screen showed its drag live.
        if let Some(order) = actions.reorder {
            self.set_playlist(&order, tick);
        }
        // What the frame changed shows from the next frame on, which egui does not run by itself; so do the keys it
        // turned on or off.
        if !(overlay_navs.is_empty() && navs.is_empty() && edit.is_none() && step.is_none())
            || self.key_input().notes != key_input.notes
        {
            ctx.request_repaint();
        }
        // Top down, so that closing one leaves the indices below it valid.
        for (index, nav) in overlay_navs.into_iter().rev() {
            self.nav.apply_from(index, nav, &self.song);
        }
        for nav in navs {
            self.navigate(nav);
        }
        // The frame's edit takes in what ending a drag the view took away made.
        let edit = edit.max(self.left_view(ctx));
        // The sample row: its picker, or an edit of its clip's slot, ongoing while a slider drags.
        match (actions.sample_row, row_clip) {
            (Some(RowAction::Pick), Some(key)) => {
                self.navigate(Nav::Open(Overlay::ClipSamplePicker(key)));
            }
            (Some(RowAction::Set { ctl, v, done }), Some(key)) => {
                if sample_row::apply(&mut self.song, key, ctl, v) || done {
                    self.row_drag = !done;
                    let edit = if done {
                        SongEdit::Final
                    } else {
                        SongEdit::Ongoing
                    };
                    self.song_edited(edit, tick);
                }
            }
            // A slider drag whose row went before its release (another track current, the panel hidden) ends at the
            // pointer's lift, as its release would.
            _ if self.row_drag && !ctx.input(|i| i.pointer.any_down()) => {
                self.row_drag = false;
                self.song_edited(SongEdit::Final, tick);
            }
            _ => {}
        }
        self.answered(tick);
        // After the navigation: a replaced song resets it.
        match edit {
            Some(SongEdit::Replaced) => self.song_replaced(ctx),
            Some(edit) => self.song_edited(edit, tick),
            None => {}
        }
        // After its step: a playlist delete asks whether what it emptied goes too (`app/tracks.rs`).
        self.ask_emptied();
        // Removing a drum row clears the undo history, the removal's step too.
        if std::mem::take(&mut self.slots.drum_editor.row_removed) {
            self.history = History::new(&self.song);
        }
        // After the edit, which re-keys the selection as it was: the channel a rack preset added.
        if let Some(index) = self.slots.preset_picker.select.take() {
            self.selected_channel = index;
        }
        // After the edit: the lane ops of the recent control and of `Delete automation?` (`app/automation.rs`).
        self.automation(add_automation, tick);
        // After the edit: the audio editor's region op, its save and the `Processing...` screen (`app/audio_edit.rs`).
        self.audio_edit(ctx, &metrics, tick);
        // After this frame's edit: an undo mid-drag reverts the drag.
        if let Some(step) = step {
            self.step(ctx, step, tick);
            // What the step restored is no move of the recent control.
            self.control_moved();
        }
        // The current track's current clip, whatever the view shows, after this frame's edits, undo and track change.
        let current = (self.main_screen.clips).clip_for(&self.song, self.selected_channel);
        self.preview(ctx, actions.preview, current.map(ClipKey::from), tick);
        self.live_lane();
        // Channel params moved go into the undo history once no pointer is down and a burst of linked MIDI CC moves has
        // ended: once per gesture, not per frame, and before the channel may go.
        // ponytail: a channel another input deletes while its knob is still held goes with its pre-drag values, which
        // its delete's undo brings back; sync before the delete's step if that shows.
        if self.params_moved && !ctx.input(|i| i.pointer.any_down()) && self.midi_quiet(ctx) {
            self.params_moved = false;
            self.history.sync(&self.song);
        }
        // A param gesture ends with no pointer down (as the rack's note-effect rebuild does); an undo or redo during
        // it has closed it already, and then this records nothing. A recording keeps it open: the take is one undo step
        // at its end.
        if self.param_gesture && !ctx.input(|i| i.pointer.any_down()) && !self.recording() {
            self.param_gesture = false;
            self.history.edited(&self.song, true);
        }
        // The keyboard panel's sizes: a hand resize or the tap toggle (the `toggleKeyboardPanel` shortcut is a global
        // key, `app/keys.rs`).
        if let Some(sizes) = actions.panel_sizes {
            self.set_panel_sizes(sizes, &metrics);
        }
        self.project_files(ctx);
        // A save took the params moved: the undo steps take them now, or the next edit would count them as unsaved.
        if self.params_moved && !self.files.modified() {
            self.params_moved = false;
            self.history.sync(&self.song);
        }
        // `Import MIDI Tracks`: its file once read, and the dialog's `Import` (`app/tracks.rs`).
        self.midi_import(ctx, tick);
        self.export_started(ctx);
        self.demo_restore(ctx);
        self.burn_notes(ctx, tick);
        self.bundle_wait(ctx, tick);
        self.file_drop(ctx, &metrics, tick);
        self.quit(ctx);
        self.flash(ctx, &metrics);
        if self.current_channel() != was_current {
            self.rack_follows(ctx);
        }
        // After every edit of the frame.
        #[cfg(any(target_arch = "wasm32", test))]
        self.live_frame(ctx, tick);
        // The focus cursor, over everything.
        focus::end_frame(ctx, &self.theme);
        // The tooltip of the control under the pointer, once every control has named its own.
        let cx = UiCtx {
            theme: &self.theme,
            m: &metrics,
        };
        crate::ui::tooltip::show(ctx, &cx);
        self.keep_settings(ctx);
        // The playhead follows the audio; otherwise egui repaints only on input.
        if playing && snapshot.is_some_and(|s| !s.stream_failed) {
            ctx.request_repaint();
        }
        #[cfg(debug_assertions)]
        self.icon_sheet(ctx, &metrics);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_link::testing::{Recorder, warnings};
    use crate::metrics::DEFAULT_UI_SCALE;
    use crate::song::SynthKind;
    use crate::ui::keyboard_settings::PANEL_H;
    use crate::ui::main_screen::{Regions, TRANSPORT_H};
    use crate::ui::modules_browser::InsertAt;
    use crate::ui::views::{MenuTab, Overlay};
    use eframe::egui::{Pos2, pos2, vec2};

    mod audio_edit;
    mod automation;
    mod automation_editor;
    mod chrome;
    mod demo_songs;
    mod drumsets;
    mod editor_follow;
    mod editor_wiring;
    mod export;
    mod factory_templates;
    mod floating_row;
    mod instruments_picker;
    mod keyboard_state;
    mod keymap_actions;
    mod live_edits;
    mod midi;
    mod module_samples;
    mod nav_gaps;
    mod new_channel;
    mod new_song;
    mod new_template;
    mod peer_live;
    mod performance_monitor;
    mod picker_rules;
    mod preview;
    mod project_info;
    mod rack_input;
    mod rack_wiring;
    mod recording;
    mod sample_row;
    mod songs_archive;
    mod soundfont_wiring;
    mod track_menu;
    mod transport;
    mod typed;
    // The browsers' search bar.
    mod search_bar;
    // The song-changed flag.
    mod song_changed;
    // Live keys held while the channels move.
    mod held_keys_rekey;
    // A drag whose editor stops drawing.
    mod hidden_gestures;
    // The open rack follows the current channel.
    mod rack_follow;
    // The first-import notice.
    mod import_notice;
    // An item drag's release shows no floating row; light floating-row bubbles.
    mod drag_release;
    // A song load on the web waits for its bundled sounds.
    mod bundle_wait;
    // Audio files dropped on the app.
    mod file_drop;

    impl HoneydewApp {
        /// A Drums channel as the model makes it (the default kit, no picker open), added at the end and selected:
        /// The fixture of tests not about the add-channel flow (that is [`Self::add_channel`]).
        fn add_default_drums(&mut self, tick: u64) {
            self.add_bare_channel(ChannelKind::Drums, tick);
        }

        /// A channel of `kind` as the model makes it (no clip, no editor or picker open), added at the end and
        /// selected: the fixture of tests not about the add-channel flow, which also adds a clip and opens its editor
        /// ([`Self::insert_channel`]).
        fn add_bare_channel(&mut self, kind: ChannelKind, tick: u64) {
            self.insert_bare_channel(self.song.channels.len(), kind, tick);
        }

        /// [`Self::add_bare_channel`] at index `at`.
        fn insert_bare_channel(&mut self, at: usize, kind: ChannelKind, tick: u64) {
            let at = self.song.insert_channel(at, kind).expect("an id");
            self.song_edited(SongEdit::Final, tick);
            self.selected_channel = at;
        }
    }

    /// The app in an offscreen context (1170 x 540 points, 1 pixel per point, UI scale 100 %: U = 32), with
    /// a [`Recorder`] for the audio output.
    struct Rig {
        ctx: egui::Context,
        app: HoneydewApp,
        rec: Recorder,
        time: f64,
        /// How often the app tried to open the audio output.
        opens: usize,
        /// Opening the output fails.
        no_device: bool,
        /// What the last frame put out: its shapes and repaint request.
        out: egui::FullOutput,
        /// The next frame brings the window's close request (its close button).
        close_request: bool,
    }

    const U: f32 = 32.0;
    /// Every ruler's height (the playlist's and the clip editors'), 1.65 U cut to whole pixels, and so the playlist's track
    /// rows' top.
    const RULER: f32 = 52.0;

    /// `x` to a thousandth of a point, for a play box's playhead x (its centre less half its width).
    fn r3(x: f32) -> f32 {
        (x * 1000.0).round() / 1000.0
    }
    /// The bottom panel at its default height ([`PANEL_H`] U): its top (the content area's bottom), the transport bar's
    /// centre line under it, and the keys' top under the 1.75 U bar (y 380, 408, 436); the keys run to the screen's
    /// bottom, 540. Checked against the layout by `the_rigs_panel_constants_are_the_layouts`.
    const PANEL_TOP: f32 = 540.0 - PANEL_H * U;
    const BAR_Y: f32 = PANEL_TOP + TRANSPORT_H * U / 2.0;
    const KEYS_TOP: f32 = PANEL_TOP + TRANSPORT_H * U;
    /// E4 and F4, low on their white keys, 115/135 of the keys' height down: velocity 108 (MIDI).
    const LOW: f32 = KEYS_TOP + 115.0 / 135.0 * (540.0 - KEYS_TOP);
    const E4: Pos2 = pos2(5.0 * U, LOW);
    const F4: Pos2 = pos2(7.0 * U, LOW);
    /// In the transport bar, between its left buttons and REC: a click there only starts the audio, held past a tap
    /// ([`Rig::click`]; a tap there toggles the panel). (The ruler is no place for it: a tap there sets the playhead.)
    const OUTSIDE: Pos2 = pos2(300.0, BAR_Y);
    /// Centres of the play button and the REV button.
    const PLAY: Pos2 = pos2(585.0, BAR_Y);
    const REV: Pos2 = pos2(585.0 - 1.75 * U, BAR_Y);

    /// The centre of drum pad `index` in the keys area, where a current Drums channel has its pads (in their rect,
    /// 0.35 U under the keys' top).
    fn pad_centre(index: usize) -> Pos2 {
        let band = egui::Rect::from_min_max(pos2(0.0, KEYS_TOP), pos2(1170.0, 540.0));
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
        let area = crate::ui::keyboard_settings::pads_rect(band, &m);
        crate::ui::drums::pad_grid(area, 1170.0, U).centres[index]
    }

    #[test]
    fn the_rigs_panel_constants_are_the_layouts() {
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
        let r = Regions::new(
            egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0)),
            &m,
        );
        assert_eq!(m.u(1.0), U);
        assert_eq!(r.area.bottom(), PANEL_TOP);
        assert_eq!(
            (r.transport.top(), r.transport.center().y),
            (PANEL_TOP, BAR_Y)
        );
        assert_eq!((r.keyboard.top(), r.keyboard.bottom()), (KEYS_TOP, 540.0));
        assert_eq!(PLAY, r.transport.center());
    }

    fn button_event(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    /// The header of track `index` (rows are 2 U tall below the ruler).
    fn header(index: usize) -> Pos2 {
        pos2(0.75 * U, RULER + (index as f32 + 0.5) * 2.0 * U)
    }

    /// `files` with an EMPTY factory in place of [`crate::content::factory`]: the real one grows (templates, presets) and
    /// tests that are not about it must not depend on what it holds.
    fn without_factory(mut files: ProjectFiles) -> ProjectFiles {
        files.factory = Box::leak(Box::new(crate::content::Factory::new(vec![])));
        files
    }

    impl Rig {
        fn new() -> Self {
            // Never the developer's songs folder or the OS file dialogs.
            let files = without_factory(ProjectFiles::with_provider(Box::new(MemFiles::default())));
            let mut rig = Self {
                ctx: egui::Context::default(),
                app: HoneydewApp {
                    files,
                    ..HoneydewApp::default()
                },
                rec: Recorder::default(),
                time: 0.0,
                opens: 0,
                no_device: false,
                out: egui::FullOutput::default(),
                close_request: false,
            };
            // Lets egui learn the widget rects before the first press.
            rig.frame(vec![]);
            // Fixed-coordinate input fixtures start at MIDI 48; startup centering has its own test.
            rig.app.main_screen.keys_view = crate::ui::keyboard::KeyView::default();
            rig.frame(vec![]);
            rig.pin_playlist();
            rig
        }

        /// Two fingers: the first holds `at`, the anchor; the second, on the screen's inner side of it (so on the
        /// editor too), spreads from 100 to 150 points away both ways, so each axis whose touch zone holds `at` zooms
        /// in 1.5 times about it.
        pub(crate) fn two_finger_zoom(&mut self, at: Pos2) {
            use egui::TouchPhase::{End, Move, Start};
            let touch = |id, phase, pos| egui::Event::Touch {
                device_id: egui::TouchDeviceId(1),
                id: egui::TouchId(id),
                phase,
                pos,
                force: None,
            };
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let inward = vec2(
                if at.x > 585.0 { -100.0 } else { 100.0 },
                if at.y > 200.0 { -100.0 } else { 100.0 },
            );
            let (from, to) = (at + inward, at + 1.5 * inward);
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![touch(1, Start, at), button(true)]);
            self.frame(vec![touch(2, Start, from)]);
            self.frame(vec![touch(2, Move, to)]);
            self.frame(vec![
                touch(2, End, to),
                touch(1, End, at),
                button(false),
                egui::Event::PointerGone,
            ]);
            self.frame(vec![]);
        }

        /// The playlist at 1.2 U a beat and 2 U rows from the song start, where the tests place their taps: the view a
        /// song opens at is its own (HEAD@296), and this takes the open's place.
        pub(crate) fn pin_playlist(&mut self) {
            let screen = &mut self.app.main_screen;
            (screen.zoom, screen.scroll_x, screen.scroll_y) = (0.0, 0.0, 0.0);
            screen.row_u = Some(2.0);
        }

        /// A rig whose audio has started, with a MiniSynth channel (1) selected and an empty log.
        fn with_synth() -> Self {
            let mut rig = Self::new();
            rig.app
                .add_bare_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
            rig.click(OUTSIDE);
            rig.rec.0.borrow_mut().log.clear();
            rig
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 0.1;
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0))),
                events,
                time: Some(self.time),
                ..Default::default()
            };
            if std::mem::take(&mut self.close_request) {
                input
                    .viewports
                    .entry(egui::ViewportId::ROOT)
                    .or_default()
                    .events = vec![egui::ViewportEvent::Close];
            }
            let Self {
                ctx,
                app,
                rec,
                opens,
                no_device,
                out,
                ..
            } = self;
            *out = ctx.run(input, |ctx| {
                app.frame(ctx, |_, _| {
                    *opens += 1;
                    (!*no_device).then(|| Box::new(rec.clone()) as Box<dyn AudioSink>)
                });
            });
        }

        fn button(&mut self, pos: Pos2, pressed: bool) {
            self.frame(vec![button_event(pos, pressed)]);
        }

        fn press(&mut self, pos: Pos2) {
            self.frame(vec![egui::Event::PointerMoved(pos)]);
            self.button(pos, true);
        }

        fn drag_to(&mut self, pos: Pos2) {
            self.frame(vec![egui::Event::PointerMoved(pos)]);
        }

        fn release(&mut self, pos: Pos2) {
            self.button(pos, false);
        }

        fn click(&mut self, pos: Pos2) {
            self.press(pos);
            // A tap on the bar's empty part toggles the panel: the rig's click there, which only starts the audio, is
            // held past a tap.
            if pos == OUTSIDE {
                for _ in 0..3 {
                    self.frame(vec![]);
                }
            }
            self.release(pos);
        }

        /// The commands sent since the last call.
        fn take(&mut self) -> Vec<String> {
            std::mem::take(&mut self.rec.0.borrow_mut().log)
        }

        /// Waits until the first `first` (`usize::MAX`: all) `STR1` renders streaming in, in the order they started,
        /// have finished on the worker, which none of this thread's held gates may hold, then lets them land; returns
        /// the log of its frames. A frame first starts the renders a rebuild left waiting. Joined, not timed: a loaded
        /// machine only makes the worker slower, and each test's worker hold is its own thread's.
        fn landed(&mut self, first: usize) -> Vec<String> {
            self.frame(vec![]);
            self.app.slots.drum_editor.wait_stretches(first);
            self.frame(vec![]);
            self.frame(vec![]);
            self.take()
        }

        /// Applies undo while a modal message, dialog or popup shows, then advances a frame. Ctrl+Z waits under
        /// A modal, but [`HoneydewApp::step`] can receive undo from other callers.
        fn undo_under_modal(&mut self) {
            let ctx = self.ctx.clone();
            self.app.step(&ctx, Step::Undo, 0);
            self.frame(vec![]);
        }

        /// The next frame comes at egui time `t`.
        fn next_frame_at(&mut self, t: f64) {
            self.time = t - 0.1;
        }

        /// A note tapped at egui time `tapped` sounds for [`TAP_GATE`] exactly, then `note_off` goes out. Frames
        /// without input within the gate ask for the frame that ends it, so the note never hangs (egui asks for
        /// the frame after an input by itself, so only the second one after the tap shows the app's ask).
        fn assert_gate(&mut self, tapped: f64, note_off: &str) {
            const EPS: f64 = 1e-3;
            for t in [TAP_GATE / 4.0, TAP_GATE / 2.0, TAP_GATE - EPS] {
                self.next_frame_at(tapped + t);
                self.frame(vec![]);
                assert_eq!(self.take(), Vec::<String>::new(), "at {t}");
                let delay = self.out.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
                assert!(
                    delay.as_secs_f64() <= TAP_GATE - t + EPS,
                    "at {t}: {delay:?}"
                );
            }
            self.next_frame_at(tapped + TAP_GATE + EPS);
            self.frame(vec![]);
            assert_eq!(self.take(), [note_off]);
        }
    }

    #[test]
    fn the_rig_keeps_its_songs_in_memory() {
        // The default files are the real songs folder and the OS dialogs.
        let rig = Rig::new();
        assert_eq!(rig.app.files.library.root(), Some(Path::new("")));
    }

    /// The app tells the engine the selected channel's current lane. On a Drums channel whose second lane holds notes,
    /// selecting a clip on that lane sends the notes lane, and a live key then takes the notes-lane path: the MiniSynth
    /// above the octave-up Transpose gets the key as played.
    #[test]
    fn a_live_key_on_a_selected_notes_lane_takes_the_notes_lane_path() {
        use crate::audio_engine::SongAudioEngine;
        use crate::ui::clips::ClipRef;
        use honeydew_audio::device::AudioRenderer;
        let mut rig = Rig::new();
        rig.app.add_default_drums(0);
        let channel = &mut rig.app.song.channels[1];
        channel.rack.add(0);
        let transpose = channel.rack.add(48);
        channel.rack.get_mut(transpose).unwrap().params[1] = 5.0 / 8.0;
        let lane = channel.add_lane(LaneKind::Notes);
        let id = channel.lanes[lane].add_clip(0, 4.0).id;
        rig.click(OUTSIDE);
        rig.app.song_edited(SongEdit::Final, 0);
        rig.frame(vec![]);
        let sent = |rig: &Rig| rig.rec.0.borrow().lanes.last().copied();
        assert_eq!(sent(&rig), Some((1, false)), "the drum lane is current");
        rig.app.main_screen.clips.selected = Some(ClipRef {
            channel: 1,
            lane,
            id,
        });
        rig.frame(vec![]);
        assert_eq!(sent(&rig), Some((1, true)));
        rig.app.audio.send(AudioCommand::NoteOn {
            channel: 1,
            key: 60,
            velocity: 1.0,
        });
        let render = |engine: &mut SongAudioEngine| {
            let mut output = vec![0.0; 4096 * 2];
            engine.render(&mut output, 2);
            output
        };
        let played = render(rig.rec.0.borrow_mut().engine.as_mut().unwrap());
        let reference = |notes: bool| {
            let samples = &rig.app.audio.samples;
            let mut engine =
                SongAudioEngine::with_samples(&rig.app.song, 48_000.0, samples).unwrap();
            engine.set_live_lane(1, notes);
            engine.note_on(1, 60, 1.0);
            render(&mut engine)
        };
        assert!(played.iter().any(|sample| sample.abs() > 0.01));
        assert!(played == reference(true));
        assert!(played != reference(false));
    }

    /// Every new engine starts from each channel's first lane: a rebuild sends the current lane again, right after
    /// its `ReplaceSong`, and the new engine routes the next live key by it.
    #[test]
    fn a_rebuild_sends_the_current_lane_again() {
        use crate::audio_engine::SongAudioEngine;
        use crate::ui::clips::ClipRef;
        use honeydew_audio::device::AudioRenderer;
        let mut rig = Rig::new();
        rig.app.add_default_drums(0);
        let channel = &mut rig.app.song.channels[1];
        channel.rack.add(0);
        let transpose = channel.rack.add(48);
        channel.rack.get_mut(transpose).unwrap().params[1] = 5.0 / 8.0;
        let lane = channel.add_lane(LaneKind::Notes);
        let id = channel.lanes[lane].add_clip(0, 4.0).id;
        rig.click(OUTSIDE);
        rig.app.main_screen.clips.selected = Some(ClipRef {
            channel: 1,
            lane,
            id,
        });
        rig.frame(vec![]);
        rig.rec.0.borrow_mut().all.clear();
        rig.app.song_edited(SongEdit::Final, 0);
        let all = std::mem::take(&mut rig.rec.0.borrow_mut().all);
        let replaced = all
            .iter()
            .position(|c| c.starts_with("ReplaceSong"))
            .expect("a rebuild");
        assert_eq!(
            all[replaced + 1..].first().map(String::as_str),
            Some("LiveLane 1 true"),
            "{all:?}"
        );
        rig.app.audio.send(AudioCommand::NoteOn {
            channel: 1,
            key: 60,
            velocity: 1.0,
        });
        let mut played = vec![0.0; 4096 * 2];
        (rig.rec.0.borrow_mut().engine.as_mut().unwrap()).render(&mut played, 2);
        let samples = &rig.app.audio.samples;
        let mut notes_lane =
            SongAudioEngine::with_samples(&rig.app.song, 48_000.0, samples).unwrap();
        notes_lane.set_live_lane(1, true);
        notes_lane.note_on(1, 60, 1.0);
        let mut expected = vec![0.0; 4096 * 2];
        notes_lane.render(&mut expected, 2);
        assert!(played == expected);
    }

    /// The press that starts the audio plays its key on the current lane's route: the lane goes out before its
    /// `NoteOn` (here an automation lane, so not the first lane's notes route).
    #[test]
    fn the_press_that_starts_audio_plays_on_the_current_lane() {
        use crate::ui::clips::ClipRef;
        let mut rig = Rig::new();
        rig.app
            .add_bare_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
        let lane = rig.app.song.channels[1].add_automation_lane(AutomationTarget {
            instance_id: 0,
            param: 1,
        });
        let id = lane.add_clip(0, 4.0).id;
        rig.app.main_screen.clips.selected = Some(ClipRef {
            channel: 1,
            lane: 1,
            id,
        });
        rig.frame(vec![]);
        rig.press(E4);
        assert_eq!(rig.opens, 1);
        let all = std::mem::take(&mut rig.rec.0.borrow_mut().all);
        let lane = all.iter().position(|c| c == "LiveLane 1 false");
        let key = all.iter().position(|c| c == "NoteOn 1 52 108");
        assert!(lane.is_some() && key.is_some() && lane < key, "{all:?}");
    }

    #[test]
    fn the_first_primary_press_starts_audio_and_plays_the_key_it_lands_on() {
        let mut rig = Rig::new();
        rig.app
            .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        assert_eq!(rig.opens, 0, "audio started without a press");
        rig.press(E4);
        assert_eq!(rig.opens, 1);
        assert_eq!(rig.take(), ["NoteOn 1 52 108"]);
        rig.release(E4);
        rig.click(F4);
        assert_eq!(rig.opens, 1, "audio started twice");
    }

    #[test]
    fn a_failed_start_retries_on_the_next_click_only() {
        let mut rig = Rig::new();
        rig.no_device = true;
        rig.click(OUTSIDE);
        for _ in 0..5 {
            rig.frame(vec![]);
        }
        assert_eq!(rig.opens, 1);
        // The UI keeps working without audio.
        rig.click(PLAY);
        assert!(rig.app.main_screen.playing);
        assert_eq!(rig.opens, 2);
        rig.no_device = false;
        rig.click(OUTSIDE);
        assert_eq!(rig.opens, 3);
        // The transport was left playing while audio was off.
        assert_eq!(rig.take(), ["Play"]);
    }

    #[test]
    fn the_audio_gesture_is_the_press_natively_and_the_release_on_the_web() {
        const { assert!(!START_ON_RELEASE, "the tests run natively") };
        let ctx = egui::Context::default();
        let gestures = |pressed: bool| {
            let mut both = (false, false);
            let raw = egui::RawInput {
                events: vec![button_event(OUTSIDE, pressed)],
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                both = ctx.input(|i| (audio_gesture(i, false), audio_gesture(i, true)));
            });
            both
        };
        assert_eq!(gestures(true), (true, false));
        assert_eq!(gestures(false), (false, true));
    }

    #[test]
    fn a_click_tries_to_start_the_audio_once() {
        let mut rig = Rig::new();
        rig.no_device = true;
        rig.press(OUTSIDE);
        assert_eq!(rig.opens, 1);
        rig.no_device = false;
        rig.release(OUTSIDE);
        assert_eq!(rig.opens, 1, "the release tried too");
        // A press and its release in the same frame try once.
        rig.frame(vec![
            button_event(OUTSIDE, true),
            button_event(OUTSIDE, false),
        ]);
        assert_eq!(rig.opens, 2);
        assert!(rig.app.audio.has_output());
        rig.click(OUTSIDE);
        assert_eq!(rig.opens, 2, "audio started twice");
    }

    #[test]
    fn a_stream_that_failed_while_idle_is_reopened_by_the_next_single_click() {
        let mut rig = Rig::with_synth();
        // No frame runs between the failure and the press: egui does not repaint while idle.
        rig.rec.0.borrow_mut().failed = true;
        rig.button(OUTSIDE, true);
        assert_eq!(rig.rec.0.borrow().reopens, 1);
        rig.button(OUTSIDE, false);
        assert_eq!(rig.rec.0.borrow().reopens, 1);
        assert_eq!(rig.take(), ["Reopen 2", "Stop"]);
    }

    #[test]
    fn a_failed_stream_warns_once_drops_commands_and_reopens_on_the_next_click() {
        let mut rig = Rig::with_synth();
        // A command the full queue handed back is waiting when the stream fails.
        rig.rec.0.borrow_mut().room = Some(0);
        rig.app.add_default_drums(0);
        // The new engine, and its current lane right after it.
        assert_eq!(rig.app.audio.pending(), 2);
        let before = warnings();
        rig.rec.0.borrow_mut().failed = true;
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        assert_eq!(warnings() - before, 1, "one warning, not one per frame");
        // Nothing waits or queues for the dead stream.
        assert_eq!(rig.app.audio.pending(), 0);
        rig.app.add_default_drums(0);
        rig.app.play_key(&[(60, 1.0)]);
        assert_eq!(rig.app.audio.pending(), 0);
        rig.rec.0.borrow_mut().room = None;
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new());
        assert_eq!(rig.rec.0.borrow().reopens, 0, "reopened without a click");

        // The next click reopens the output with the current song (master and three channels), and the
        // transport takes the play button's state.
        rig.click(OUTSIDE);
        assert_eq!(rig.rec.0.borrow().reopens, 1);
        assert_eq!(
            rig.opens, 1,
            "a failed output is reopened, not started again"
        );
        assert_eq!(rig.take(), ["Reopen 4", "Stop"]);
        // The new Drums channel (3) is current: its pads replace the keyboard.
        rig.press(pad_centre(0));
        assert_eq!(rig.take(), ["NoteOn 3 48 127"]);
        assert_eq!(warnings() - before, 1);
    }

    #[test]
    fn a_failed_reopen_keeps_the_output_warns_and_retries_on_the_next_click() {
        let mut rig = Rig::with_synth();
        rig.click(PLAY);
        rig.rec.0.borrow_mut().failed = true;
        rig.rec.0.borrow_mut().no_device = true;
        rig.frame(vec![]);
        let before = warnings();
        rig.click(OUTSIDE);
        // One attempt per click, and its failure warns.
        assert_eq!(rig.rec.0.borrow().reopens, 1);
        assert_eq!(warnings() - before, 1);
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        assert_eq!(rig.rec.0.borrow().reopens, 1, "retried without a click");
        assert_eq!(warnings() - before, 1);
        assert!(rig.app.audio.has_output(), "the output was dropped");

        rig.rec.0.borrow_mut().no_device = false;
        rig.click(OUTSIDE);
        assert_eq!(rig.rec.0.borrow().reopens, 2);
        assert_eq!(rig.opens, 1);
        assert_eq!(rig.take(), ["Play", "Reopen 2", "Play"]);
    }

    #[test]
    fn press_slide_and_release_play_exact_notes_on_the_selected_channel() {
        let mut rig = Rig::with_synth();
        rig.press(E4);
        rig.frame(vec![]);
        rig.drag_to(F4);
        rig.release(F4);
        assert_eq!(
            rig.take(),
            [
                "NoteOn 1 52 108",
                "NoteOff 1 52",
                "NoteOn 1 53 108",
                "NoteOff 1 53"
            ]
        );
    }

    #[test]
    fn each_finger_sounds_its_own_key_at_its_own_depth_on_the_selected_channel() {
        let mut rig = Rig::with_synth();
        let touch = |id, phase, pos| egui::Event::Touch {
            device_id: egui::TouchDeviceId(0),
            id: egui::TouchId(id),
            phase,
            pos,
            force: None,
        };
        use egui::TouchPhase::{End, Move, Start};
        // G4 half way down: velocity 0.5 (MIDI 64).
        let g4 = pos2(9.0 * U, (KEYS_TOP + 540.0) / 2.0);
        rig.frame(vec![touch(1, Start, E4)]);
        rig.frame(vec![touch(2, Start, g4)]);
        // The first finger slides onto F4: E4 stops, F4 sounds at E4's velocity; G4 sounds on.
        rig.frame(vec![touch(1, Move, F4)]);
        rig.frame(vec![touch(2, End, g4)]);
        rig.frame(vec![touch(1, End, F4)]);
        assert_eq!(
            rig.take(),
            [
                "NoteOn 1 52 108",
                "NoteOn 1 55 64",
                "NoteOff 1 52",
                "NoteOn 1 53 108",
                "NoteOff 1 55",
                "NoteOff 1 53"
            ]
        );
    }

    #[test]
    fn a_tap_within_one_frame_sounds_until_its_gate_ends() {
        let mut rig = Rig::with_synth();
        let tap = [button_event(E4, true), button_event(E4, false)];
        // A mouse tap, and a touch whose pointer goes with the lift.
        for events in [
            vec![tap[0].clone(), tap[1].clone()],
            vec![tap[0].clone(), tap[1].clone(), egui::Event::PointerGone],
        ] {
            rig.frame(vec![egui::Event::PointerMoved(E4)]);
            rig.frame(events);
            let tapped = rig.time;
            assert_eq!(rig.take(), ["NoteOn 1 52 108"]);
            rig.assert_gate(tapped, "NoteOff 1 52");
        }
        // A key pressed within the gate takes over at once.
        rig.frame(vec![egui::Event::PointerMoved(E4)]);
        rig.frame(tap.to_vec());
        rig.time += TAP_GATE / 2.0 - 0.1;
        rig.frame(vec![egui::Event::PointerMoved(F4), button_event(F4, true)]);
        rig.release(F4);
        assert_eq!(
            rig.take(),
            [
                "NoteOn 1 52 108",
                "NoteOff 1 52",
                "NoteOn 1 53 108",
                "NoteOff 1 53"
            ]
        );
    }

    #[test]
    fn leaving_the_keyboard_and_releasing_outside_stops_the_note() {
        let mut rig = Rig::with_synth();
        rig.press(E4);
        rig.drag_to(OUTSIDE);
        rig.release(OUTSIDE);
        rig.frame(vec![]);
        assert_eq!(rig.take(), ["NoteOn 1 52 108", "NoteOff 1 52"]);
    }

    #[test]
    fn losing_focus_mid_hold_stops_the_note_and_the_stale_press_plays_nothing() {
        let mut rig = Rig::with_synth();
        rig.press(E4);
        rig.frame(vec![egui::Event::WindowFocused(false)]);
        assert_eq!(rig.take(), ["NoteOn 1 52 108", "NoteOff 1 52"]);
        // The release was lost with the focus: egui still reports the button down.
        rig.frame(vec![egui::Event::WindowFocused(true)]);
        rig.drag_to(F4);
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new());
        // A real release, then a new press plays again.
        rig.release(F4);
        rig.press(E4);
        assert_eq!(rig.take(), ["NoteOn 1 52 108"]);
    }

    #[test]
    fn a_fresh_press_after_the_focus_returns_plays_at_once() {
        let mut rig = Rig::with_synth();
        rig.press(E4);
        rig.frame(vec![egui::Event::WindowFocused(false)]);
        rig.frame(vec![egui::Event::WindowFocused(true)]);
        rig.take();
        // The release was lost with the focus, so egui still has the button down; the next press is a new one.
        rig.press(F4);
        assert_eq!(rig.take(), ["NoteOn 1 53 108"]);
        rig.release(F4);
        assert_eq!(rig.take(), ["NoteOff 1 53"]);
    }

    #[test]
    fn a_focus_loss_stops_a_tapped_note_within_its_gate_at_once() {
        let mut rig = Rig::with_synth();
        rig.frame(vec![egui::Event::PointerMoved(E4)]);
        rig.frame(vec![button_event(E4, true), button_event(E4, false)]);
        assert_eq!(rig.take(), ["NoteOn 1 52 108"]);
        // A hidden web page runs no frame to end the gate, so the focus-loss frame ends it.
        rig.time += TAP_GATE / 4.0 - 0.1;
        rig.frame(vec![egui::Event::WindowFocused(false)]);
        assert_eq!(rig.take(), ["NoteOff 1 52"]);
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new());
    }

    #[test]
    fn play_toggles_play_and_stop_and_rev_stops_and_rewinds() {
        let mut rig = Rig::with_synth();
        rig.click(PLAY);
        assert!(rig.app.main_screen.playing);
        rig.click(PLAY);
        assert!(!rig.app.main_screen.playing);
        rig.click(PLAY);
        rig.click(REV);
        assert!(!rig.app.main_screen.playing);
        // The transport is at the play-start point (the rig reports tick 0): REV resets the sound there.
        assert_eq!(
            rig.take(),
            ["Play", "Stop", "Play", "Stop", "Seek 0", "ResetSound"]
        );
    }

    #[test]
    fn commands_the_full_queue_hands_back_go_first_and_in_order() {
        let mut rig = Rig::with_synth();
        rig.rec.0.borrow_mut().room = Some(0);
        rig.press(E4);
        rig.drag_to(F4);
        assert_eq!(rig.take(), Vec::<String>::new());
        assert_eq!(rig.app.audio.pending(), 3);
        // Room for one: the oldest goes, the rest keeps waiting.
        rig.rec.0.borrow_mut().room = Some(1);
        rig.frame(vec![]);
        assert_eq!(rig.take(), ["NoteOn 1 52 108"]);
        // The rest goes before the release's NoteOff.
        rig.rec.0.borrow_mut().room = None;
        rig.release(F4);
        assert_eq!(
            rig.take(),
            ["NoteOff 1 52", "NoteOn 1 53 108", "NoteOff 1 53"]
        );
        assert_eq!(rig.app.audio.pending(), 0);
    }

    #[test]
    fn adding_a_channel_replaces_the_song_and_resumes_playback_where_it_was() {
        let mut rig = Rig::with_synth();
        // Stopped: the new engine keeps the position.
        rig.app.add_channel(ChannelKind::Drums, 700);
        assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
        // The new Drums channel's drumset picker: closed, loading nothing.
        let picker = Overlay::PresetPicker {
            channel: 2,
            target: crate::ui::rack::PresetTarget::Drumset,
        };
        assert_eq!(rig.app.nav.overlays, [picker]);
        rig.frame(escape());
        rig.frame(vec![]);
        assert!(rig.app.nav.overlays.is_empty());

        rig.click(PLAY);
        rig.take();
        rig.app.add_channel(ChannelKind::Drums, 700);
        assert_eq!(rig.take(), ["ReplaceSong adopting no-resync", "Play"]);
    }

    #[test]
    fn picking_from_the_add_popup_while_playing_resumes_at_the_frame_tick() {
        let mut rig = Rig::with_synth();
        rig.click(PLAY);
        rig.rec.0.borrow_mut().tick = 333;
        let center = |rig: &Rig, id: egui::Id| rig.ctx.read_response(id).unwrap().rect.center();
        rig.click(center(&rig, egui::Id::new("add_channel_button")));
        rig.frame(vec![]);
        // `Synth:` column, first item: MiniSynth.
        let mini_synth = egui::Id::new("add_channel_popup").with((1_usize, 1_usize));
        rig.click(center(&rig, mini_synth));
        assert_eq!(rig.app.song.channels.len(), 3);
        assert_eq!(rig.app.selected_channel, 2);
        assert_eq!(
            rig.take(),
            ["Play", "ReplaceSong adopting no-resync", "Play"]
        );
    }

    #[test]
    fn a_new_channel_is_selected_and_a_header_click_selects_the_master_too() {
        let mut rig = Rig::new();
        assert_eq!(rig.app.selected_channel, 0);
        rig.app
            .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
        assert_eq!(rig.app.selected_channel, 1);
        rig.app.add_channel(ChannelKind::Drums, 0);
        assert_eq!(rig.app.selected_channel, 2);
        // Its drumset picker, closed first.
        let picker = Overlay::PresetPicker {
            channel: 2,
            target: crate::ui::rack::PresetTarget::Drumset,
        };
        assert_eq!(rig.app.nav.overlays, [picker]);
        rig.frame(escape());
        rig.frame(vec![]);
        // Its drum editor, open under the picker, collapsed for the playlist's headers.
        assert_eq!(rig.app.nav.view, View::DrumEditor(2));
        rig.app.navigate(Nav::Collapse);
        rig.frame(vec![]);
        rig.click(header(1));
        assert_eq!(rig.app.selected_channel, 1);
        rig.press(E4);
        assert_eq!(rig.take(), ["NoteOn 1 52 108"]);
        rig.release(E4);
        rig.take();
        // The master becomes current, and its keys play nothing.
        rig.click(header(0));
        assert_eq!(rig.app.selected_channel, 0);
        rig.press(E4);
        assert!(rig.take().is_empty());
    }

    #[test]
    fn with_only_the_master_the_keyboard_plays_nothing() {
        let mut rig = Rig::new();
        rig.click(OUTSIDE);
        rig.press(E4);
        rig.drag_to(F4);
        rig.release(F4);
        assert_eq!(rig.opens, 1);
        assert_eq!(rig.take(), Vec::<String>::new());
    }

    /// Transport button centres: mixer, keyboard settings, CTRL and close.
    const MIXER: Pos2 = pos2(0.75 * U, BAR_Y);
    const KEYBOARD_SETTINGS: Pos2 = pos2(2.25 * U, BAR_Y);
    const CTRL: Pos2 = pos2(585.0 + 2.0 * 1.75 * U, BAR_Y);
    const CLOSE: Pos2 = pos2(1170.0 - 0.75 * U, BAR_Y);
    /// Inside the logo quarter-disc in the top-right corner, and the rack tab on the right edge (centred on the
    /// track rows, from the 1.5 U ruler to the panel).
    const LOGO: Pos2 = pos2(1150.0, 20.0);
    const TAB_Y: f32 = (1.5 * U + PANEL_TOP) / 2.0;
    const EDGE_TAB: Pos2 = pos2(1160.0, TAB_Y);

    /// The lane of track `index`, right of its header.
    fn lane(index: usize) -> Pos2 {
        header(index) + vec2(10.0 * U, 0.0)
    }

    impl Rig {
        /// A rig with Drums (1) and MiniSynth (2), MiniSynth selected.
        fn with_drums_and_synth() -> Self {
            let mut rig = Self::new();
            rig.app.add_default_drums(0);
            rig.app
                .add_bare_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
            rig.frame(vec![]);
            rig
        }

        fn overlays(&self) -> Vec<Overlay> {
            self.app.nav.overlays.clone()
        }

        /// A wheel of `dy` points over `at`, and the frames that smooth it out.
        fn wheel(&mut self, at: Pos2, dy: f32) {
            let wheel = egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, dy),
                modifiers: Default::default(),
            };
            self.frame(vec![egui::Event::PointerMoved(at), wheel]);
            for _ in 0..60 {
                self.frame(vec![]);
            }
        }

        /// Clicks the centre of the widget `id` as laid out last frame.
        fn click_id(&mut self, id: egui::Id) {
            let rect = self
                .ctx
                .read_response(id)
                .expect("the widget was drawn")
                .rect;
            self.click(rect.center());
        }
    }

    #[test]
    fn the_logo_opens_the_main_menu_whose_tabs_switch_and_whose_close_x_closes_it() {
        let mut rig = Rig::new();
        rig.click(LOGO);
        assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Songs)]);
        rig.frame(vec![]);
        rig.click_id(egui::Id::new("main_menu").with(("tab", 2_usize)));
        assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Settings)]);
        rig.click_id(egui::Id::new("main_menu").with("close"));
        assert_eq!(rig.overlays(), []);
    }

    #[test]
    fn the_transport_switches_to_the_mixer_and_back_and_toggles_its_panels() {
        let mut rig = Rig::new();
        rig.click(MIXER);
        assert_eq!(rig.app.nav.view, View::Mixer);
        rig.click(MIXER);
        assert_eq!(rig.app.nav.view, View::Playlist);
        rig.click(KEYBOARD_SETTINGS);
        assert_eq!(rig.overlays(), [Overlay::KeyboardSettings]);
        rig.click(CTRL);
        assert_eq!(
            rig.overlays(),
            [Overlay::KeyboardSettings, Overlay::CtrlMenu]
        );
        // The CTRL menu is a modal popup, shown from its second frame (egui sizes it first): a tap on CTRL
        // closes it through its backdrop, which goes with the repaint the closing asks for.
        rig.frame(vec![]);
        rig.click(CTRL);
        rig.frame(vec![]);
        rig.click(KEYBOARD_SETTINGS);
        assert_eq!(rig.overlays(), []);
        // Switching the view closes the overlays; the close X hides the panel and keeps the mixer.
        rig.click(KEYBOARD_SETTINGS);
        rig.click(MIXER);
        assert_eq!((rig.app.nav.view, rig.overlays()), (View::Mixer, vec![]));
        rig.click(CLOSE);
        assert_eq!(rig.app.nav.view, View::Mixer);
        assert!(rig.app.main_screen.panel_hidden);
    }

    #[test]
    fn a_clips_edit_button_opens_its_channels_editor_and_the_close_x_goes_back() {
        let mut rig = Rig::with_drums_and_synth();
        let edit = egui::Id::new(("clip_button", 3_usize));
        let plus = egui::Id::new("clip_insert").with(0);
        // A tap on the empty drum lane makes its track current, the next shows the insert menu, whose `New Clip` adds a
        // clip and selects it; its Edit opens the drum editor.
        rig.click(lane(1));
        rig.click(lane(1));
        rig.frame(vec![]);
        rig.click_id(plus);
        rig.frame(vec![]);
        rig.click_id(edit);
        assert_eq!(rig.app.nav.view, View::DrumEditor(1));
        rig.slide_split();
        rig.click_id(egui::Id::new("editor_close"));
        assert_eq!(rig.app.nav.view, View::Playlist);
        // On the synth lane: the first tap clears the selection, the second makes the track current, the third shows
        // the insert menu.
        for _ in 0..3 {
            rig.click(lane(2));
        }
        rig.frame(vec![]);
        rig.click_id(plus);
        rig.frame(vec![]);
        rig.click_id(edit);
        assert_eq!(rig.app.nav.view, View::PianoRoll(2));
        rig.slide_split();
        rig.click_id(egui::Id::new("editor_close"));
        assert_eq!(rig.app.nav.view, View::Playlist);
        // A clip edit reaches the song.
        assert_eq!(rig.app.song.channels[2].lanes[0].clips.len(), 1);
    }

    #[test]
    fn a_replaced_song_clears_the_clip_selection_and_the_timeline_scroll() {
        let mut rig = Rig::with_drums_and_synth();
        rig.app.song.channels[1].lanes[0].add_clip(0, 16.0);
        rig.frame(vec![]);
        rig.click(lane(1));
        assert!(rig.app.main_screen.clips.selected.is_some());
        rig.app.main_screen.scroll_x = 2.0;
        // The same shape of song, so the old selection would still find a clip.
        let mut song = rig.app.song.clone();
        song.title = "Opened".into();
        rig.app.song = song;
        rig.app.song_edited(SongEdit::Replaced, 0);
        assert_eq!(rig.app.main_screen.clips.selected, None);
        assert_eq!(rig.app.main_screen.scroll_x, 0.0);
    }

    #[test]
    fn a_header_tap_selects_the_track_and_the_edge_tab_opens_the_selected_ones_rack() {
        // A track header tap makes the track current, and opens no rack.
        let mut rig = Rig::with_drums_and_synth();
        rig.click(header(1));
        assert_eq!(rig.app.selected_channel, 1);
        assert_eq!(rig.overlays(), []);
        // The master is selected too: the edge tab opens its rack, where the master effects go.
        rig.click(header(0));
        assert_eq!(rig.app.selected_channel, 0);
        rig.click(EDGE_TAB);
        assert_eq!(rig.overlays(), [Overlay::Rack(0)]);
        rig.slide_rack();
        rig.click(rack_wiring::collapse_tab());
        assert_eq!(rig.overlays(), []);
    }

    #[test]
    fn overlays_without_a_trigger_yet_close_with_their_x_or_ok() {
        let mut rig = Rig::new();
        for (overlay, close) in [
            (
                Overlay::SaveDialog {
                    name: "Song".into(),
                },
                egui::Id::new("save_dialog").with("close"),
            ),
            (
                Overlay::Message {
                    tag: "damaged",
                    lines: vec!["The file is damaged.".into(), "It was not opened.".into()],
                    confirm: None,
                    extra: None,
                },
                egui::Id::new("message").with((0_usize, 2_usize)),
            ),
        ] {
            rig.app.navigate(Nav::Open(overlay.clone()));
            rig.frame(vec![]);
            assert_eq!(rig.overlays(), [overlay]);
            rig.click_id(close);
            assert_eq!(rig.overlays(), [], "{close:?}");
        }
    }

    #[test]
    fn the_main_menu_takes_the_clicks_on_it_and_a_click_beside_it_only_closes_it() {
        let mut rig = Rig::with_synth();
        rig.click(LOGO);
        rig.frame(vec![]);
        // The main menu covers the right of the screen (20 U), keyboard included.
        rig.click(pos2(900.0, 520.0));
        assert_eq!(rig.take(), Vec::<String>::new());
        assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Songs)]);
        // A click left of it closes it and reaches nothing below.
        rig.click(E4);
        assert_eq!(rig.overlays(), []);
        assert_eq!(rig.take(), Vec::<String>::new());
        rig.click(E4);
        rig.click(REV);
        assert_eq!(
            rig.take(),
            ["NoteOn 1 52 108", "NoteOff 1 52", "Seek 0", "ResetSound"]
        );
    }

    /// The drum view has its pads in the keyboard's place: see `the_pads_replace_the_keyboard_…`.
    #[test]
    fn the_transport_and_keyboard_work_in_every_view() {
        for view in [View::Mixer, View::PianoRoll(1)] {
            let mut rig = Rig::with_synth();
            rig.app.navigate(Nav::Show(view));
            rig.frame(vec![]);
            rig.click(PLAY);
            rig.click(E4);
            assert_eq!(
                rig.take(),
                ["Play", "NoteOn 1 52 108", "NoteOff 1 52"],
                "{view:?}"
            );
        }
    }

    #[test]
    fn each_overlays_x_closes_that_overlay_only() {
        let browser = Overlay::ModulesBrowser {
            channel: 1,
            at: InsertAt::End,
        };
        let browser_x = egui::Id::new("modules_browser").with("close");
        for (stack, close, left) in [
            (
                vec![Overlay::Rack(1), Overlay::KeyboardSettings],
                Err(rack_wiring::collapse_tab()),
                vec![Overlay::KeyboardSettings],
            ),
            // The browser is modal: the rack beside it (the 18 U browser covers the notch) is a click outside it,
            // which closes the browser.
            (
                vec![Overlay::Rack(1), browser.clone()],
                Err(pos2(800.0, 200.0)),
                vec![Overlay::Rack(1)],
            ),
            (
                vec![Overlay::Rack(1), browser.clone()],
                Ok(browser_x),
                vec![Overlay::Rack(1)],
            ),
        ] {
            let mut rig = Rig::with_synth();
            for overlay in stack {
                rig.app.navigate(Nav::Open(overlay));
            }
            // Past the rack's and the browser's slide-in.
            rig.slide_rack();
            match close {
                Ok(id) => rig.click_id(id),
                Err(at) => rig.click(at),
            }
            assert_eq!(rig.overlays(), left, "{close:?}");
        }
    }

    /// A tap of Escape: its press and release (a press of a held key would be an auto-repeat, which closes nothing).
    fn escape() -> Vec<egui::Event> {
        key_tap(egui::Key::Escape, egui::Modifiers::NONE)
    }

    #[test]
    fn escape_goes_to_the_topmost_overlay_only() {
        let message = Overlay::Message {
            tag: "saved",
            lines: vec!["Saved.".into()],
            confirm: None,
            extra: None,
        };
        let mut rig = Rig::with_synth();
        rig.app.navigate(Nav::Open(message.clone()));
        rig.app.navigate(Nav::Open(Overlay::Rack(1)));
        rig.frame(vec![]);
        // The rack on top ignores Escape, and the message below never sees it.
        rig.frame(escape());
        assert_eq!(rig.overlays(), [message.clone(), Overlay::Rack(1)]);
        rig.app.navigate(Nav::Open(message.clone()));
        rig.frame(vec![]);
        rig.frame(escape());
        assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    }

    #[test]
    fn the_close_x_on_the_playlist_only_hides_the_panel() {
        let mut rig = Rig::with_synth();
        rig.app.navigate(Nav::Open(Overlay::Rack(1)));
        rig.app.navigate(Nav::Open(Overlay::KeyboardSettings));
        rig.frame(vec![]);
        rig.click(CLOSE);
        assert!(rig.app.main_screen.panel_hidden);
        let overlays = vec![Overlay::Rack(1), Overlay::KeyboardSettings];
        assert_eq!(
            (rig.app.nav.view, rig.overlays()),
            (View::Playlist, overlays)
        );
    }

    #[test]
    fn a_view_or_panel_of_a_channel_that_is_gone_falls_back() {
        for view in [View::PianoRoll(9), View::DrumEditor(9)] {
            let mut rig = Rig::with_synth();
            rig.app.navigate(Nav::Show(view));
            rig.frame(vec![]);
            assert_eq!(rig.app.nav.view, View::Playlist, "{view:?}");
        }
        for overlay in [
            Overlay::Rack(9),
            Overlay::ModulesBrowser {
                channel: 9,
                at: InsertAt::End,
            },
        ] {
            let mut rig = Rig::with_synth();
            rig.app.navigate(Nav::Open(Overlay::KeyboardSettings));
            rig.app.navigate(Nav::Open(overlay.clone()));
            rig.frame(vec![]);
            assert_eq!(rig.overlays(), [Overlay::KeyboardSettings], "{overlay:?}");
        }
    }

    #[test]
    fn a_message_returns_its_answer_to_the_app() {
        let ask = || {
            Nav::Open(Overlay::Message {
                tag: "newer",
                lines: vec!["A newer version wrote this file.".into(), "Open it?".into()],
                confirm: Some(("Yes".into(), "Cancel".into())),
                extra: None,
            })
        };
        let mut rig = Rig::with_synth();
        rig.app.navigate(Nav::Open(Overlay::Rack(1)));
        rig.app.navigate(ask());
        rig.frame(vec![]);
        // Two lines, then Yes and Cancel, one per row.
        rig.click_id(egui::Id::new("message").with((0_usize, 2_usize)));
        assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
        assert_eq!(rig.app.nav.take_answer("newer"), Some(true));
        rig.app.navigate(ask());
        rig.frame(vec![]);
        rig.click_id(egui::Id::new("message").with((0_usize, 3_usize)));
        assert_eq!(rig.app.nav.take_answer("newer"), Some(false));
        // Escape answers no.
        rig.app.navigate(ask());
        rig.frame(vec![]);
        rig.frame(escape());
        assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
        assert_eq!(rig.app.nav.take_answer("newer"), Some(false));
    }

    #[test]
    fn ongoing_and_param_edits_rebuild_nothing_and_the_final_one_rebuilds() {
        let mut rig = Rig::with_synth();
        rig.click(PLAY);
        rig.take();
        rig.app.song_edited(SongEdit::Ongoing, 0);
        rig.app.song_edited(SongEdit::Param, 0);
        assert_eq!(rig.take(), Vec::<String>::new());
        rig.app.song_edited(SongEdit::Final, 700);
        assert_eq!(rig.take(), ["ReplaceSong adopting no-resync", "Play"]);
    }

    #[test]
    fn an_edit_while_stopped_keeps_the_playhead_and_play_starts_there() {
        use honeydew_audio::device::AudioRenderer;
        // Stopped at bar 3, in a four-bar song (bar 3 is past a shorter one's end, where Play wraps).
        const BAR_3: u64 = 2 * 4 * 128;
        let mut rig = Rig::with_synth();
        rig.app.song.channels[1].lanes[0].add_clip(0, 16.0);
        rig.rec.0.borrow_mut().tick = BAR_3;
        // The recorder's first engine, stopped there.
        rig.app.rebuild(BAR_3);
        rig.take();
        // A structural edit: its engine takes over the stopped one at its position.
        rig.app.song.channels[1].rack.add(32);
        rig.app.test_edit = Some(SongEdit::Final);
        rig.frame(vec![]);
        assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
        rig.click(PLAY);
        assert_eq!(rig.take(), ["Play"]);
        let mut rec = rig.rec.0.borrow_mut();
        let engine = rec.engine.as_mut().expect("the edit's engine");
        engine.render(&mut [0.0; 512], 2);
        let tick = engine.transport_tick();
        assert!((BAR_3..BAR_3 + 8).contains(&tick), "playing from {tick}");
    }

    #[test]
    fn a_replaced_song_resets_the_navigation_and_stops_the_transport() {
        let mut rig = Rig::with_synth();
        rig.app.add_default_drums(0);
        rig.click(PLAY);
        rig.edit(2);
        rig.app.navigate(Nav::Open(Overlay::Rack(2)));
        rig.frame(vec![]);
        rig.take();
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        rig.app.song = song;
        rig.app.song_edited(SongEdit::Replaced, 0);
        assert_eq!((rig.app.nav.view, rig.overlays()), (View::Playlist, vec![]));
        assert_eq!(rig.app.selected_channel, 1);
        assert!(!rig.app.main_screen.playing);
        assert_eq!(rig.take(), ["Stop", "ReplaceSong"]);
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new());
        // A master-only song selects the master.
        rig.app.song = Song::default();
        rig.app.song_edited(SongEdit::Replaced, 0);
        assert_eq!(rig.app.selected_channel, 0);
    }

    #[test]
    fn an_edit_reported_during_a_frame_reaches_the_funnel() {
        let mut rig = Rig::with_synth();
        rig.app.test_edit = Some(SongEdit::Ongoing);
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new());
        rig.app.test_edit = Some(SongEdit::Final);
        rig.frame(vec![]);
        assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    }

    #[test]
    fn a_song_replaced_in_the_frame_that_opens_a_rack_leaves_no_overlays() {
        let mut rig = Rig::with_synth();
        rig.press(EDGE_TAB);
        // The release opens the selected channel's rack, in the frame that replaces the song.
        rig.app.test_edit = Some(SongEdit::Replaced);
        rig.release(EDGE_TAB);
        assert_eq!(rig.overlays(), []);
    }

    #[test]
    fn a_frame_that_navigates_asks_for_the_next_frame() {
        let mut rig = Rig::new();
        rig.press(LOGO);
        // The release opens the main menu after the frame is drawn; only a repaint shows it.
        rig.release(LOGO);
        assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Songs)]);
        // The causes of a pass's requests show after the next pass.
        rig.frame(vec![]);
        let causes = rig.ctx.repaint_causes();
        assert!(
            causes.iter().any(|c| c.file.ends_with("app.rs")),
            "{causes:?}"
        );
    }

    #[test]
    fn the_built_in_keyboard_zoom_is_off() {
        let ctx = egui::Context::default();
        assert!(ctx.options(|o| o.zoom_with_keyboard), "egui's default");
        setup_context(&ctx);
        assert!(!ctx.options(|o| o.zoom_with_keyboard));
    }

    // ---- Open / save on the overlays ----

    use crate::project_files::mem::MemFiles;
    use crate::project_files::tests::{FRESH, synthetic_codec, synthetic_file, with_stamp};
    use crate::project_files::{DISCARD_TAG, FORMAT_VERSION, MESSAGE_TAG, ProjectFiles, UNSAVED};
    use crate::ui::save_dialog;

    /// [`Rig::with_synth`], with an in-memory provider whose open dialog picks `path`, and a handle on it. The
    /// song counts as saved.
    fn rig_with_file(path: &str, bytes: &[u8]) -> (Rig, MemFiles) {
        let mut rig = Rig::with_synth();
        let mem = MemFiles::with_file(path, bytes);
        rig.app.files = without_factory(ProjectFiles::with_provider(Box::new(mem.clone())));
        rig.app.files.codec = synthetic_codec();
        rig.app.files.saved();
        (rig, mem)
    }

    /// A rig whose audio has started, with a fresh 4.10.19 song (`tests/fixtures`) opened through the real codec from
    /// `/songs/Fresh.flm`, its log empty, and a handle on the files. Its samples are original content, which tests do
    /// not have: the missing-audio message is closed.
    fn fresh_song() -> (Rig, MemFiles) {
        let mut rig = Rig::new();
        let mem = MemFiles::with_file("/songs/Fresh.flm", FRESH);
        rig.app.files = ProjectFiles::with_provider(Box::new(mem.clone()));
        rig.click(OUTSIDE);
        command(&mut rig, egui::Key::O);
        rig.app.navigate(Nav::Close);
        rig.frame(vec![]);
        rig.take();
        (rig, mem)
    }

    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn command(rig: &mut Rig, key: egui::Key) {
        rig.frame(key_tap(key, egui::Modifiers::COMMAND));
    }

    /// A press of `key` with `modifiers` and its release: a second press without a release in between would be an
    /// auto-repeat, which the keymap's actions ignore (`Keymap::consume_press`).
    fn key_tap(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        let release = egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        };
        vec![key_event(key, modifiers), release]
    }

    /// The message's item at `row` (its lines come first).
    fn message_item(row: usize) -> egui::Id {
        egui::Id::new("message").with((0_usize, row))
    }

    #[test]
    fn opening_a_project_replaces_the_song_selects_its_first_channel_and_rebuilds_audio() {
        let (mut rig, _) = rig_with_file("/songs/Beat.flm", &synthetic_file(2, FORMAT_VERSION, ""));
        rig.click(PLAY);
        rig.app.navigate(Nav::Open(Overlay::Rack(1)));
        rig.take();
        command(&mut rig, egui::Key::O);
        assert_eq!(rig.app.song.title, "Beat");
        assert_eq!(rig.app.song.channels.len(), 3);
        assert_eq!(rig.app.selected_channel, 1);
        assert_eq!(rig.overlays(), []);
        // The new engine starts stopped, and so does the transport.
        assert!(!rig.app.main_screen.playing);
        assert_eq!(rig.take(), ["Stop", "ReplaceSong"]);
    }

    #[test]
    fn the_owners_song_opens_with_its_channels_clips_and_rack() {
        let mut rig = Rig::new();
        let mem = MemFiles::with_file("/songs/Fresh.flm", FRESH);
        rig.app.files = ProjectFiles::with_provider(Box::new(mem));
        rig.click(OUTSIDE);
        rig.take();
        command(&mut rig, egui::Key::O);
        assert_eq!(rig.take(), ["Stop", "ReplaceSong"]);
        // No notices; its samples are original content, which tests do not have.
        let missing = |o: &[Overlay]| {
            matches!(
                o,
                [Overlay::Message {
                    tag: MISSING_TAG,
                    ..
                }]
            )
        };
        assert!(missing(&rig.overlays()), "{:?}", rig.overlays());
        rig.app.navigate(Nav::Close);
        let song = &rig.app.song;
        assert_eq!(song.title, "Fresh");
        let channels: Vec<_> = song
            .channels
            .iter()
            .map(|c| (c.name.as_str(), c.kind))
            .collect();
        assert_eq!(
            channels,
            [
                ("MASTER", ChannelKind::Master),
                ("Close Grand", ChannelKind::Instrument),
                ("Drums 2", ChannelKind::Drums)
            ]
        );
        let clips: Vec<Vec<usize>> = (song.channels.iter())
            .map(|c| c.lanes.iter().map(|l| l.clips.len()).collect())
            .collect();
        assert_eq!(clips, [vec![0], vec![1, 1], vec![1]]);
        let rack: Vec<u16> = song.channels[1]
            .rack
            .modules
            .iter()
            .map(|m| m.type_id)
            .collect();
        assert_eq!(rack, [1, 9], "DirectWave and Filter");
        assert_eq!(
            song.channels[2].drum_kit.as_ref().map(|k| k.slots.len()),
            Some(11)
        );
        assert_eq!(rig.app.selected_channel, 1);
        rig.frame(vec![]);
        // Its stored view's rows (HEAD@296 zoom y: 1/170 px, 2 U where it was saved) clamp to 100/24 U at U = 32:
        // Drums 2's track shows once the wheel over the grid scrolls the list down.
        assert_eq!(rig.app.main_screen.row_u, Some(100.0 / 24.0));
        assert!(rig.painted("Close Grand"));
        rig.wheel(pos2(600.0, 300.0), -400.0);
        assert!(rig.painted("Drums 2"));
        rig.app.navigate(Nav::Open(Overlay::Rack(1)));
        rig.frame(vec![]);
        rig.frame(vec![]);
        for name in [1, 9].map(crate::ui::rack::module_name) {
            assert!(rig.painted(name), "{name}");
        }
        // Showing it changes nothing: no unsaved changes.
        rig.edit(2);
        rig.slide_split();
        rig.frame(vec![]);
        assert!(!rig.app.files.modified());
    }

    /// A song whose first pad plays `My Samples/Gone.wav`, which no test has.
    fn gone_song() -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        let kit = song.channels[1].drum_kit.as_mut().unwrap();
        kit.slots[0].file.path = Some(crate::flm::chunk::ContentPath::Shorthand {
            root: crate::flm::chunk::ContentRoot::User,
            relative: "My Samples/Gone.wav".into(),
        });
        song
    }

    /// A song with missing audio loads, and the user is told once per load, whether the audio runs or not, after the
    /// load's own message.
    #[test]
    fn a_song_with_missing_audio_loads_and_says_so_once_per_load() {
        let missing = || Overlay::Message {
            tag: MISSING_TAG,
            lines: vec!["Some audio files were missing".into()],
            confirm: None,
            extra: None,
        };
        for started in [false, true] {
            let mut rig = Rig::new();
            let mem = MemFiles::with_file("/songs/Gone.flm", b"10LF");
            rig.app.files = ProjectFiles::with_provider(Box::new(mem));
            rig.app.files.codec.read = |_| Ok((gone_song(), Vec::new()));
            if started {
                rig.click(OUTSIDE);
            }
            command(&mut rig, egui::Key::O);
            assert_eq!(rig.app.song.title, "Gone");
            assert_eq!(rig.overlays(), [missing()], "audio started: {started}");
            // Once per load: an edit that rebuilds does not say it again; the next load does.
            rig.app.navigate(Nav::Close);
            rig.app.song_edited(SongEdit::Final, 0);
            rig.frame(vec![]);
            assert_eq!(rig.overlays(), []);
            command(&mut rig, egui::Key::O);
            assert_eq!(rig.overlays(), [missing()]);
        }
        // After the load's corrupted-data message, which shows first.
        let (mut rig, _) = rig_with_file("/songs/Gone.flm", b"10LF");
        rig.app.files.codec.read = |_| {
            Ok((
                gone_song(),
                vec!["Missing rack for Channel #3 skipped".into()],
            ))
        };
        command(&mut rig, egui::Key::O);
        assert!(matches!(
            &rig.overlays()[..],
            [Overlay::Message {
                tag: MESSAGE_TAG,
                ..
            }]
        ));
        rig.app.navigate(Nav::Close);
        rig.frame(vec![]);
        assert_eq!(rig.overlays(), [missing()]);
        // A song whose samples are all there says nothing.
        let (mut rig, _) = rig_with_file("/songs/Beat.flm", &synthetic_file(2, FORMAT_VERSION, ""));
        command(&mut rig, egui::Key::O);
        assert_eq!(rig.overlays(), []);
    }

    #[test]
    fn a_project_with_only_the_master_selects_the_master() {
        let (mut rig, _) =
            rig_with_file("/songs/Empty.flm", &synthetic_file(0, FORMAT_VERSION, ""));
        command(&mut rig, egui::Key::O);
        assert_eq!(rig.app.song.channels.len(), 1);
        assert_eq!(rig.app.selected_channel, 0);
        assert_eq!(rig.take(), ["Stop", "ReplaceSong"]);
    }

    #[test]
    fn an_app_made_with_a_song_counts_it_as_saved() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        let mut app = HoneydewApp::with_song(song.clone());
        // New and Open replace it without asking about unsaved changes.
        assert!(!app.files.modified());
        // An undo goes back to it, not to an empty song.
        app.add_default_drums(0);
        assert_eq!(app.history.undo(), Some(song));
    }

    #[test]
    fn a_corrupt_project_shows_the_message_and_leaves_the_song_and_audio_alone() {
        let (mut rig, _) = rig_with_file("/songs/Bad.flm", b"JUNK");
        let before = rig.app.song.clone();
        command(&mut rig, egui::Key::O);
        assert_eq!(rig.app.song, before);
        assert_eq!(rig.app.selected_channel, 1);
        assert!(matches!(
            rig.overlays()[..],
            [Overlay::Message {
                tag: MESSAGE_TAG,
                ..
            }]
        ));
        assert_eq!(rig.take(), Vec::<String>::new());
        // The message is modal: the keyboard under it does not play.
        rig.frame(vec![]);
        rig.press(E4);
        rig.release(E4);
        assert_eq!(rig.take(), Vec::<String>::new());
    }

    #[test]
    fn command_o_opens_and_command_s_saves_without_asking() {
        let (mut rig, mem) =
            rig_with_file("/songs/Beat.flm", &synthetic_file(1, FORMAT_VERSION, ""));
        // A plain O is not the shortcut.
        rig.frame(vec![key_event(egui::Key::O, egui::Modifiers::NONE)]);
        assert_eq!(rig.app.song.title, "");
        command(&mut rig, egui::Key::O);
        assert_eq!(rig.app.song.title, "Beat");
        rig.app.add_default_drums(0);
        command(&mut rig, egui::Key::S);
        assert_eq!(rig.overlays(), [], "it asked");
        assert_eq!(
            mem.0.borrow().files[std::path::Path::new("/songs/Beat.flm")],
            synthetic_file(2, FORMAT_VERSION, "")
        );
    }

    #[test]
    fn the_message_shows_over_the_old_song_blocks_the_shortcuts_and_ok_closes_it() {
        let (mut rig, _) = rig_with_file("/songs/Bad.flm", b"JUNK");
        command(&mut rig, egui::Key::O);
        rig.frame(vec![]);
        // While it shows, the shortcuts do nothing: the untitled song would ask for a name.
        command(&mut rig, egui::Key::S);
        assert_eq!(rig.overlays().len(), 1);
        rig.click_id(message_item(2));
        assert_eq!(rig.overlays(), []);
    }

    #[test]
    fn yes_loads_a_project_from_a_newer_version_and_cancel_does_not() {
        let (mut rig, _) = rig_with_file("/songs/New.flm", &synthetic_file(1, "9.0", ""));
        command(&mut rig, egui::Key::O);
        rig.frame(vec![]);
        // Three lines, then Yes and Cancel.
        rig.click_id(message_item(4));
        assert_eq!(rig.overlays(), []);
        rig.frame(vec![]);
        assert_eq!(rig.app.song.channels.len(), 2, "Cancel loaded");

        command(&mut rig, egui::Key::O);
        rig.frame(vec![]);
        rig.click_id(message_item(3));
        rig.frame(vec![]);
        assert_eq!(rig.app.song.title, "New");
        assert_eq!(rig.app.song.channels.len(), 2);
        assert_eq!(rig.overlays(), []);
    }

    #[test]
    fn command_s_on_an_untitled_song_shows_the_dialog_and_save_writes_the_file() {
        let (mut rig, mem) = rig_with_file("/songs/Beat.flm", b"");
        mem.0.borrow_mut().save_dir = Some("/home/me".into());
        command(&mut rig, egui::Key::S);
        assert_eq!(
            rig.overlays(),
            [Overlay::SaveDialog {
                name: "New Song".into()
            }]
        );
        rig.frame(vec![]);
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
        rig.click(save_dialog::layout(screen, &m).save.center());
        assert_eq!(rig.overlays(), []);
        assert_eq!(rig.app.song.title, "New Song");
        assert_eq!(
            mem.0.borrow().files[std::path::Path::new("/home/me/New Song.flm")],
            synthetic_file(1, FORMAT_VERSION, "")
        );
    }

    #[test]
    fn a_bad_name_keeps_the_dialog_under_the_message_and_escape_closes_it() {
        let (mut rig, mem) = rig_with_file("/songs/Beat.flm", b"");
        command(&mut rig, egui::Key::S);
        let Some(Overlay::SaveDialog { name }) = rig.app.nav.overlays.last_mut() else {
            panic!("no dialog");
        };
        *name = "a:b".into();
        rig.frame(vec![]);
        rig.frame(vec![key_event(egui::Key::Enter, egui::Modifiers::NONE)]);
        let top = rig.overlays();
        assert!(matches!(
            top[..],
            [Overlay::SaveDialog { .. }, Overlay::Message { .. }]
        ));
        assert!(mem.0.borrow().saves.is_empty());
        rig.frame(vec![]);
        rig.click_id(message_item(1));
        assert!(matches!(rig.overlays()[..], [Overlay::SaveDialog { .. }]));
        rig.frame(vec![]);
        rig.frame(escape());
        assert_eq!(rig.overlays(), []);
    }

    // ---- Settings ----

    use crate::settings::STORAGE_KEY;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    /// A settings storage that counts its writes and flushes; clones share it.
    #[derive(Clone, Default)]
    struct Spy(Rc<RefCell<SpyState>>);

    #[derive(Default)]
    struct SpyState {
        map: HashMap<String, String>,
        sets: usize,
        flushes: usize,
    }

    impl eframe::Storage for Spy {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.borrow().map.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            let mut spy = self.0.borrow_mut();
            spy.sets += 1;
            spy.map.insert(key.to_owned(), value);
        }

        fn flush(&mut self) {
            self.0.borrow_mut().flushes += 1;
        }
    }

    impl Spy {
        fn counts(&self) -> (usize, usize) {
            let spy = self.0.borrow();
            (spy.sets, spy.flushes)
        }
    }

    impl Rig {
        /// One frame with `events` and no audio, returning what it painted and asked the window for.
        fn output(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            self.time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0))),
                events,
                time: Some(self.time),
                ..Default::default()
            };
            let Self { ctx, app, .. } = self;
            ctx.run(input, |ctx| app.frame(ctx, |_, _| None))
        }

        /// The texts the next frame paints.
        fn texts(&mut self) -> Vec<String> {
            self.output(vec![])
                .shapes
                .into_iter()
                .filter_map(|c| match c.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect()
        }

        /// Whether the next frame paints the whole screen in `theme`'s background first.
        fn background_is(&mut self, theme: u32) -> bool {
            let fill = theme_by_id(theme).colors.background.to_color32();
            let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
            self.output(vec![]).shapes.iter().any(
                |c| matches!(&c.shape, egui::Shape::Rect(r) if r.rect == screen && r.fill == fill),
            )
        }
    }

    #[test]
    fn the_settings_are_loaded_at_startup_and_drive_the_first_frame() {
        let spy = Spy::default();
        spy.0.borrow_mut().map.insert(
            STORAGE_KEY.into(),
            r#"{"theme": 6, "showHardwareMeters": false}"#.into(),
        );
        let mut rig = Rig::new();
        rig.app = HoneydewApp::with_storage(Some(Box::new(spy.clone())));
        assert!(rig.background_is(6));
        assert!(!rig.texts().contains(&"CPU".to_owned()));
        // Loading is not a change: nothing is written back.
        assert_eq!(spy.counts(), (0, 0));
    }

    #[test]
    fn the_theme_setting_repaints_the_app_in_its_colours() {
        let mut rig = Rig::new();
        assert!(rig.background_is(0));
        rig.app.settings.theme = 6;
        rig.frame(vec![]);
        assert!(rig.background_is(6));
        assert!(!rig.background_is(0));
    }

    #[test]
    fn the_scaling_setting_resizes_the_ui() {
        let mut rig = Rig::new();
        let logo = |rig: &Rig| {
            rig.ctx
                .read_response(egui::Id::new("app_logo"))
                .unwrap()
                .rect
        };
        let before = logo(&rig).width();
        rig.app.settings.ui_scale = 150;
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert!((logo(&rig).width() - before * 1.5).abs() < 1e-3);
    }

    #[test]
    fn middle_c_and_key_names_change_the_keyboard_labels_but_range_labels_do_not() {
        // Every root key (C under Chromatic) shows its octave, only for a channel that supports key-lock. With the
        // master alone there is none, so no labels at all.
        let has = |texts: &[String], t: &str| texts.iter().any(|x| x == t);
        let mut rig = Rig::new();
        let texts = rig.texts();
        assert!(!has(&texts, "C4") && !has(&texts, "C5"), "{texts:?}");
        // Defaults: middle C (MIDI 60) is C5, no key names.
        let mut rig = Rig::with_synth();
        let texts = rig.texts();
        assert!(has(&texts, "C4") && has(&texts, "C5"), "{texts:?}");
        assert!(!has(&texts, "C3") && !has(&texts, "D"));
        rig.app.settings.middle_c_is_c4 = true;
        let texts = rig.texts();
        assert!(has(&texts, "C3") && has(&texts, "C4"), "{texts:?}");
        rig.app.settings.key_names = true;
        let texts = rig.texts();
        for name in ["C3", "D", "E", "F", "G", "A", "B"] {
            assert!(has(&texts, name), "{name}: {texts:?}");
        }
        // The roots keep their octave whatever the range labels (which draw range borders).
        rig.app.settings.range_labels = false;
        let texts = rig.texts();
        assert!(has(&texts, "C3") && has(&texts, "D"), "{texts:?}");
        assert!(!has(&texts, "C"), "{texts:?}");
        rig.app.settings.key_names = false;
        let texts = rig.texts();
        assert!(has(&texts, "C3") && !has(&texts, "D"), "{texts:?}");
    }

    #[test]
    fn the_meters_setting_shows_or_hides_the_cpu_and_ram_readouts() {
        let mut rig = Rig::new();
        let readout = |texts: Vec<String>| texts.iter().any(|t| t.ends_with(" MB"));
        assert!(readout(rig.texts()));
        rig.app.settings.hardware_meters = false;
        assert!(!readout(rig.texts()));
    }

    #[test]
    fn a_tap_in_the_settings_tab_changes_saves_and_flushes_the_settings_once() {
        let spy = Spy::default();
        let mut rig = Rig::new();
        rig.app.storage = Some(Box::new(spy.clone()));
        rig.app
            .navigate(Nav::Open(Overlay::MainMenu(MenuTab::Settings)));
        rig.frame(vec![]);
        rig.frame(vec![]);
        // `Multi core processing`, the settings tab's row 5.
        rig.click_id(egui::Id::new("settings_tab").with(5_usize));
        assert!(rig.app.settings.multi_core);
        assert_eq!(spy.counts(), (1, 1));
        let stored = Settings::load(&spy);
        assert_eq!(stored, rig.app.settings);
        rig.frame(vec![]);
        assert_eq!(spy.counts(), (1, 1), "saved again with no change");
    }

    #[test]
    fn settings_changed_mid_drag_are_flushed_once_the_button_is_up() {
        let spy = Spy::default();
        let mut rig = Rig::new();
        rig.app.storage = Some(Box::new(spy.clone()));
        rig.press(OUTSIDE);
        for level in [0.8, 0.9] {
            rig.app.settings.metronome_level = level;
            rig.frame(vec![]);
        }
        // Kept in memory each time, not written to disk while the button is down.
        assert_eq!(spy.counts(), (2, 0));
        rig.release(OUTSIDE);
        assert_eq!(spy.counts(), (2, 1));
        assert_eq!(Settings::load(&spy).metronome_level, 0.9);
    }

    /// A flush the storage says failed ([`crate::settings::unflushed`]) is tried again each frame until
    /// one works, and once more as the app exits.
    #[test]
    fn a_failed_settings_flush_is_tried_again_until_it_works_and_at_exit() {
        let spy = Spy::default();
        let mut rig = Rig::new();
        rig.app.storage = Some(Box::new(spy.clone()));
        let failing = |on: bool| {
            let map = &mut spy.0.borrow_mut().map;
            match on {
                true => map.insert(crate::settings::UNFLUSHED_KEY.into(), String::new()),
                false => map.remove(crate::settings::UNFLUSHED_KEY),
            };
        };
        failing(true);
        rig.app.settings.metronome_level = 0.8;
        rig.frame(vec![]);
        assert_eq!(spy.counts(), (1, 1));
        rig.frame(vec![]);
        assert_eq!(spy.counts(), (1, 1), "not again within a second");
        rig.time += 1.0;
        rig.frame(vec![]);
        assert_eq!(spy.counts(), (1, 2), "tried again");
        eframe::App::on_exit(&mut rig.app, None);
        assert_eq!(spy.counts(), (1, 3), "and at exit");
        failing(false);
        rig.time += 1.0;
        rig.frame(vec![]);
        assert_eq!(spy.counts(), (1, 4));
        rig.time += 1.0;
        rig.frame(vec![]);
        eframe::App::on_exit(&mut rig.app, None);
        assert_eq!(spy.counts(), (1, 4), "done once it worked");
        // The rack's `INPUT` pick, kept in the same storage at once, is tried again the same way.
        failing(true);
        let route = crate::recording::routing::Route {
            device: "Mic".into(),
            first: 0,
            second: None,
        };
        rig.app.slots.rack.route = Some((0, route));
        rig.frame(vec![]);
        assert_eq!(spy.counts(), (2, 5));
        rig.frame(vec![]);
        assert_eq!(spy.counts(), (2, 5));
        rig.time += 1.0;
        rig.frame(vec![]);
        assert_eq!(spy.counts(), (2, 6), "the pick tried again");
    }

    // ---- The main menu ----

    use std::path::{Path, PathBuf};

    fn songs_id(part: impl std::hash::Hash) -> egui::Id {
        egui::Id::new("main_menu_songs").with(part)
    }

    fn project_id(part: impl std::hash::Hash) -> egui::Id {
        egui::Id::new("main_menu_project").with(part)
    }

    /// The SONGS row of the library's song `name`.
    fn row(name: &str) -> egui::Id {
        songs_id(("row", PathBuf::from(name)))
    }

    impl Rig {
        fn open_menu(&mut self, tab: MenuTab) {
            self.app.navigate(Nav::Open(Overlay::MainMenu(tab)));
            self.frame(vec![]);
            self.frame(vec![]);
        }

        /// The widget `id` was drawn in the last frame.
        fn drawn(&self, id: egui::Id) -> bool {
            self.ctx.read_response(id).is_some()
        }
    }

    #[test]
    fn songs_new_starts_an_empty_untitled_song() {
        let (mut rig, mem) =
            rig_with_file("/songs/Beat.flm", &synthetic_file(2, FORMAT_VERSION, ""));
        command(&mut rig, egui::Key::O);
        rig.click(PLAY);
        rig.open_menu(MenuTab::Songs);
        rig.take();
        rig.click_id(songs_id(("big", 0_usize)));
        // The Song options dialog's defaults make the empty song: the new song at the dialog's 140 BPM, not the startup
        // song's 120.
        rig.click_id(egui::Id::new(crate::ui::song_options::ID).with("ok"));
        let mut song = rig.app.song.clone();
        assert_eq!(song.tempo_bpm, 140.0);
        song.tempo_bpm = Song::default().tempo_bpm;
        // Its view, zooms per U from the moment it comes in, is no part of the comparison.
        assert!(song.file.zooms_per_u());
        (song.file.playlist_view, song.file.head_extra) =
            (Song::default().file.playlist_view, vec![]);
        assert!(crate::project_files::tests::new_now(&song));
        assert_eq!(rig.overlays(), []);
        assert!(!rig.app.main_screen.playing);
        assert_eq!(rig.take(), ["Stop", "ReplaceSong"]);
        // Untitled: saving asks for a name instead of writing over Beat.flm.
        command(&mut rig, egui::Key::S);
        assert!(matches!(rig.overlays()[..], [Overlay::SaveDialog { .. }]));
        assert_eq!(mem.0.borrow().saves, []);
    }

    #[test]
    fn songs_save_quick_saves_and_the_library_keeps_the_saved_song() {
        let (mut rig, mem) =
            rig_with_file("/songs/Beat.flm", &synthetic_file(2, FORMAT_VERSION, ""));
        command(&mut rig, egui::Key::O);
        rig.app.add_default_drums(0);
        rig.open_menu(MenuTab::Songs);
        rig.click_id(songs_id(("big", 1_usize)));
        assert_eq!(
            rig.overlays(),
            [Overlay::MainMenu(MenuTab::Songs)],
            "it asked"
        );
        let three = synthetic_file(3, FORMAT_VERSION, "");
        assert_eq!(mem.0.borrow().files[Path::new("/songs/Beat.flm")], three);
        let library = &rig.app.files.library;
        assert_eq!(library.read(Path::new("Beat.flm")).unwrap(), three);
    }

    #[test]
    fn songs_save_names_an_untitled_song_and_the_open_list_shows_it() {
        let (mut rig, mem) = rig_with_file("/songs/Beat.flm", b"");
        mem.0.borrow_mut().save_dir = Some("/home/me".into());
        rig.open_menu(MenuTab::Songs);
        assert!(!rig.drawn(row("New Song.flm")));
        rig.click_id(songs_id(("big", 1_usize)));
        assert!(matches!(
            rig.overlays()[..],
            [Overlay::MainMenu(_), Overlay::SaveDialog { .. }]
        ));
        rig.frame(vec![]);
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
        rig.click(save_dialog::layout(screen, &m).save.center());
        assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Songs)]);
        assert!(
            mem.0
                .borrow()
                .files
                .contains_key(Path::new("/home/me/New Song.flm"))
        );
        rig.frame(vec![]);
        assert!(rig.drawn(row("New Song.flm")));
    }

    #[test]
    fn a_song_in_the_list_opens_through_the_library() {
        let (mut rig, _) = rig_with_file("/songs/Beat.flm", b"");
        let groove = synthetic_file(3, FORMAT_VERSION, "");
        rig.app.files.library.store("Groove.flm", &groove).unwrap();
        rig.click(PLAY);
        rig.open_menu(MenuTab::Songs);
        rig.take();
        rig.click_id(row("Groove.flm"));
        assert_eq!(rig.app.song.title, "Groove");
        assert_eq!(rig.app.song.channels.len(), 4);
        assert_eq!(rig.app.selected_channel, 1);
        assert_eq!(rig.overlays(), []);
        assert_eq!(rig.take(), ["Stop", "ReplaceSong"]);
    }

    #[test]
    fn a_damaged_song_in_the_list_shows_the_message_and_keeps_the_song() {
        let (mut rig, _) = rig_with_file("/songs/Beat.flm", b"");
        rig.app.files.library.store("Bad.flm", b"JUNK").unwrap();
        rig.open_menu(MenuTab::Songs);
        rig.take();
        let before = rig.app.song.clone();
        rig.click_id(row("Bad.flm"));
        assert_eq!(rig.app.song, before);
        assert!(matches!(
            rig.overlays()[..],
            [
                Overlay::MainMenu(_),
                Overlay::Message {
                    tag: MESSAGE_TAG,
                    ..
                }
            ]
        ));
        assert_eq!(rig.take(), Vec::<String>::new());
    }

    #[test]
    fn songs_import_asks_the_file_picker() {
        let (mut rig, _) = rig_with_file("/songs/Beat.flm", &synthetic_file(1, FORMAT_VERSION, ""));
        rig.open_menu(MenuTab::Songs);
        rig.take();
        rig.click_id(songs_id(("big", 2_usize)));
        assert_eq!(rig.app.song.title, "Beat");
        assert_eq!(rig.overlays(), []);
        assert_eq!(rig.take(), ["Stop", "ReplaceSong"]);
    }

    #[test]
    fn the_songs_folder_is_read_again_when_the_menu_opens() {
        let (mut rig, _) = rig_with_file("/songs/Beat.flm", b"");
        rig.open_menu(MenuTab::Songs);
        rig.app.files.library.store("Late.flm", b"x").unwrap();
        rig.frame(vec![]);
        assert!(!rig.drawn(row("Late.flm")), "read every frame");
        rig.click_id(egui::Id::new("main_menu").with("close"));
        assert_eq!(rig.overlays(), []);
        rig.frame(vec![]);
        rig.open_menu(MenuTab::Songs);
        assert!(rig.drawn(row("Late.flm")));
    }

    /// The SONGS ≡ menu's item at `row`.
    fn songs_option(rig: &mut Rig, row: usize) {
        rig.click_id(songs_id(("side", 4_usize)));
        rig.frame(vec![]);
        rig.click_id(songs_id("options").with((0_usize, row)));
        rig.frame(vec![]);
    }

    #[test]
    fn songs_delete_permanently_asks_yes_or_no_and_a_failed_action_shows_its_message() {
        let (mut rig, _) = rig_with_file("/songs/Beat.flm", b"");
        rig.app.files.library.store("Groove.flm", b"g").unwrap();
        rig.open_menu(MenuTab::Songs);
        rig.click_id(songs_id(("side", 3_usize)));
        rig.click_id(row("Groove.flm"));
        // Select all, Create folder, Delete.
        songs_option(&mut rig, 2);
        let trash = rig.app.files.library.trash().unwrap();
        rig.click_id(songs_id(("row", trash.clone())));
        songs_option(&mut rig, 0);
        // Select all, Restore, Delete permanently.
        songs_option(&mut rig, 2);
        let asked = |rig: &Rig| {
            matches!(&rig.overlays()[..], [Overlay::MainMenu(_), Overlay::Message { lines, .. }]
                if lines == &["Delete 1 file?"])
        };
        assert!(asked(&rig));
        let groove = trash.join("Groove.flm");
        rig.click_id(message_item(2));
        assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Songs)], "No");
        assert!(rig.app.files.library.exists(&groove));
        songs_option(&mut rig, 0);
        songs_option(&mut rig, 2);
        assert!(asked(&rig));
        rig.click_id(message_item(1));
        rig.frame(vec![]);
        assert!(!rig.app.files.library.exists(&groove), "Yes");
        rig.frame(vec![]);
        assert!(
            !rig.drawn(songs_id(("row", groove))),
            "the list is read again"
        );

        // A copy whose song is gone meanwhile: Paste shows why it failed.
        rig.click_id(songs_id(("side", 0_usize)));
        rig.app.files.library.store("Gone.flm", b"x").unwrap();
        rig.app.navigate(Nav::Close);
        rig.open_menu(MenuTab::Songs);
        rig.click_id(row("Gone.flm"));
        // Select all, Create folder, Delete, Copy.
        songs_option(&mut rig, 3);
        rig.app.files.library.remove(Path::new("Gone.flm")).unwrap();
        // Select all, Create folder, Delete, Copy, Cut, Paste.
        songs_option(&mut rig, 5);
        assert!(matches!(
            &rig.overlays()[..],
            [Overlay::MainMenu(_), Overlay::Message { lines, confirm: None, .. }]
                if lines[0].starts_with("Gone.flm: ")
        ));
    }

    /// In SONGS select mode: renames the row `name` (its field holds `stem`) to `new`.
    fn rename_in_songs(rig: &mut Rig, name: &str, stem: &str, new: &str) {
        rig.click_id(row(name));
        // Select all, Create folder, Delete, Copy, Cut, Paste, Export, Rename.
        songs_option(rig, 7);
        for _ in stem.chars() {
            rig.frame(vec![key_event(egui::Key::Backspace, egui::Modifiers::NONE)]);
        }
        rig.frame(vec![egui::Event::Text(new.into())]);
        rig.frame(vec![key_event(egui::Key::Enter, egui::Modifiers::NONE)]);
        // The field closes: the rows move up, which `click_id` sees a frame later.
        rig.frame(vec![]);
        rig.frame(vec![]);
    }

    /// A regression case: the open song's file renamed, its old name given to another song, then a quick save.
    #[test]
    fn a_quick_save_after_songs_renamed_the_songs_file_never_overwrites_another_song() {
        let (mut rig, mem) = rig_with_file("/songs/X.flm", b"");
        let (beat, groove) = (
            synthetic_file(1, FORMAT_VERSION, ""),
            synthetic_file(3, FORMAT_VERSION, ""),
        );
        rig.app.files.library.store("Beat.flm", &beat).unwrap();
        rig.app.files.library.store("Groove.flm", &groove).unwrap();
        rig.open_menu(MenuTab::Songs);
        rig.click_id(row("Beat.flm"));
        assert_eq!(rig.app.song.title, "Beat");
        rig.open_menu(MenuTab::Songs);
        rig.click_id(songs_id(("side", 3_usize)));
        rename_in_songs(&mut rig, "Beat.flm", "Beat", "Old");
        assert_eq!(rig.app.song.title, "Old", "the title follows its file");
        assert!(!rig.app.files.modified());
        rename_in_songs(&mut rig, "Groove.flm", "Groove", "Beat");
        command(&mut rig, egui::Key::S);
        let library = &rig.app.files.library;
        assert_eq!(library.read(Path::new("Beat.flm")).unwrap(), groove);
        assert_eq!(library.read(Path::new("Old.flm")).unwrap(), beat);
        let last = mem.0.borrow().saves.last().unwrap().clone();
        assert_eq!(last, (PathBuf::from("Old.flm"), false));
    }

    #[test]
    fn a_song_moved_into_a_folder_saves_there_and_one_deleted_asks_where() {
        let (mut rig, mem) = rig_with_file("/songs/X.flm", b"");
        let (beat, other) = (
            synthetic_file(1, FORMAT_VERSION, ""),
            synthetic_file(2, FORMAT_VERSION, ""),
        );
        rig.app.files.library.store("Beat.flm", &beat).unwrap();
        rig.app.files.library.make_folder(Path::new("F")).unwrap();
        rig.open_menu(MenuTab::Songs);
        rig.click_id(row("Beat.flm"));
        // Cut, into F, Paste.
        rig.open_menu(MenuTab::Songs);
        rig.click_id(songs_id(("side", 3_usize)));
        rig.click_id(row("Beat.flm"));
        songs_option(&mut rig, 4);
        // Out of select mode, a folder opens.
        rig.click_id(songs_id(("side", 3_usize)));
        rig.click_id(row("F"));
        rig.frame(vec![]);
        // Select all, Create folder, Paste.
        songs_option(&mut rig, 2);
        let moved = Path::new("F/Beat.flm");
        assert!(rig.app.files.library.exists(moved));
        // Another song takes the old name: the save goes to the moved file, not over it, and makes no copy.
        rig.app.files.library.store("Beat.flm", &other).unwrap();
        command(&mut rig, egui::Key::S);
        assert_eq!(mem.0.borrow().saves.last().unwrap().0, moved);
        let library = &rig.app.files.library;
        assert_eq!(library.read(Path::new("Beat.flm")).unwrap(), other);
        assert_eq!(library.read(moved).unwrap(), beat);
        // Deleted (into Recently Deleted): the next save asks for a name and a place.
        rig.click_id(songs_id(("side", 3_usize)));
        rig.click_id(songs_id(("row", PathBuf::from("F/Beat.flm"))));
        // Select all, Create folder, Delete.
        songs_option(&mut rig, 2);
        let saves = mem.0.borrow().saves.len();
        command(&mut rig, egui::Key::S);
        assert!(matches!(
            rig.overlays()[..],
            [Overlay::MainMenu(_), Overlay::SaveDialog { .. }]
        ));
        assert_eq!(mem.0.borrow().saves.len(), saves);
    }

    #[test]
    fn project_tempo_steps_and_title_edits_go_through_the_funnel() {
        let mut rig = Rig::with_synth();
        rig.open_menu(MenuTab::Project);
        rig.take();
        rig.click_id(project_id(("tempo", 2_usize)));
        rig.click_id(project_id(("tempo", 2_usize)));
        rig.click_id(project_id(("tempo", 0_usize)));
        assert_eq!(rig.app.song.tempo_bpm, 121.0);
        // A tempo step without audio clips changes only the tempo: the engine plays on.
        let stepped = ["ReplaceSong adopting no-resync"; 3];
        assert_eq!(rig.take(), stepped);
        // The title is no undo step and the audio never reads it: typing it rebuilds nothing, and it is an unsaved
        // change.
        rig.click_id(project_id("Title:"));
        rig.frame(vec![egui::Event::Text("Groove".into())]);
        assert_eq!(rig.app.song.file.meta.display_title, "Groove");
        rig.frame(vec![key_event(egui::Key::Enter, egui::Modifiers::NONE)]);
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new());
        assert_eq!(rig.app.song.file.meta.display_title, "Groove");
        assert!(rig.app.files.modified());
    }

    #[test]
    fn the_project_title_is_the_display_title_and_never_renames_the_file() {
        // HEAD@8: the song title is the file name, and quick save names the file after it; PROJECT `Title:` is
        // META@520, the display title. Typing it is no undo step.
        let (mut rig, mem) = fresh_song();
        rig.open_menu(MenuTab::Project);
        rig.click_id(project_id("Title:"));
        rig.frame(vec![egui::Event::Text("Groove".into())]);
        rig.frame(vec![key_event(egui::Key::Enter, egui::Modifiers::NONE)]);
        rig.frame(vec![]);
        let song = &rig.app.song;
        assert_eq!(
            (song.title.as_str(), song.file.meta.display_title.as_str()),
            ("Fresh", "Groove")
        );
        assert!(!rig.app.history.can_undo());
        command(&mut rig, egui::Key::S);
        let (path, _) = mem.0.borrow().saves.last().unwrap().clone();
        assert_eq!(path.file_name().unwrap(), "Fresh.flm");
        let written = crate::flm::read(&mem.0.borrow().files[&path]).unwrap();
        assert_eq!(written.file.meta.display_title, "Groove");
        assert_eq!(rig.app.song.title, "Fresh");
    }

    #[test]
    fn project_shows_the_songs_meta_whose_texts_are_unsaved_changes_its_time_reset_none_and_no_undo_steps()
     {
        let mut rig = Rig::new();
        let meta = &mut rig.app.song.file.meta;
        meta.created = "2026-10-10 23-55-03".into();
        meta.modified = "2026-10-11 00:02:20".into();
        meta.editing_seconds = 13.0 * 60.0 + 2.7;
        meta.author = "Me".into();
        meta.info_url = "example.com".into();
        rig.app.files.saved();
        rig.open_menu(MenuTab::Project);
        for text in [
            "13m 2s",
            "10 October 2026 23:55:03",
            "11 October 2026 00:02:20",
            "Me",
        ] {
            assert!(rig.painted(text), "{text}");
        }
        // The link is below the fold.
        let content = crate::ui::main_menu::shell_layout(
            egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0)),
            &Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE),
        )
        .content;
        let scroll = |rig: &mut Rig, dy: f32| {
            let wheel = egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, dy),
                modifiers: Default::default(),
            };
            rig.frame(vec![egui::Event::PointerMoved(content.center()), wheel]);
            for _ in 0..60 {
                rig.frame(vec![]);
            }
        };
        scroll(&mut rig, -5000.0);
        assert!(rig.painted("example.com"));
        // Typed into the song's META: an unsaved change, as the title is, and nothing for the audio or the history.
        for (field, typed) in [("Author:", "!"), ("Online link:", "/a")] {
            rig.click_id(project_id(field));
            rig.frame(vec![egui::Event::Text(typed.into())]);
            rig.frame(vec![key_event(egui::Key::Enter, egui::Modifiers::NONE)]);
            rig.frame(vec![]);
        }
        let meta = &rig.app.song.file.meta;
        assert_eq!(
            (meta.author.as_str(), meta.info_url.as_str()),
            ("Me!", "example.com/a")
        );
        assert!(rig.app.files.modified());
        assert!(!rig.app.history.can_undo());
        assert_eq!(rig.take(), Vec::<String>::new());
        // `Reset` asks, then zeroes the time, which no undo brings back. It sets no song-changed flag.
        rig.app.files.saved();
        scroll(&mut rig, 5000.0);
        rig.click_id(project_id("Reset"));
        rig.frame(vec![]);
        assert_eq!(asking(&rig), Some("main_menu_reset_time"));
        rig.click_id(message_item(2));
        rig.frame(vec![]);
        assert_eq!(rig.app.song.file.meta.editing_seconds, 0.0);
        assert!(rig.painted("0m 0s"));
        assert!(!rig.app.files.modified());
        assert!(!rig.app.history.can_undo());
    }

    #[test]
    fn editing_time_counts_the_gaps_between_actions_under_ten_minutes() {
        // META @256: each user action adds the time since the last one, unless that is 600 s or more; a negative time
        // or a locked song (META@776) is not counted.
        let mut rig = Rig::new();
        let act = |rig: &mut Rig, at: f64| {
            rig.next_frame_at(at);
            rig.app.song.tempo_bpm += 1.0;
            rig.app.test_edit = Some(SongEdit::Final);
            rig.frame(vec![]);
            rig.app.song.file.meta.editing_seconds
        };
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        assert_eq!(act(&mut rig, 100.0), 0.0, "nothing before the first action");
        assert!(close(act(&mut rig, 130.0), 30.0));
        assert!(close(act(&mut rig, 730.0), 30.0), "a 600 s gap");
        assert!(close(act(&mut rig, 735.5), 35.5));
        // A param edit and an undo are actions too.
        rig.next_frame_at(740.0);
        rig.app.test_edit = Some(SongEdit::Param);
        rig.frame(vec![]);
        assert!(close(rig.app.song.file.meta.editing_seconds, 40.0));
        rig.next_frame_at(742.0);
        rig.frame(z(egui::Modifiers::NONE));
        assert!(close(rig.app.song.file.meta.editing_seconds, 42.0));
        // Not counted: a negative time, a locked song.
        rig.app.song.file.meta.editing_seconds = -1.0;
        assert_eq!(act(&mut rig, 743.0), -1.0);
        let meta = &mut rig.app.song.file.meta;
        (meta.editing_seconds, meta.locked) = (5.0, true);
        assert_eq!(act(&mut rig, 744.0), 5.0);
        // A new song counts from its own first action.
        rig.app.song = Song::default();
        rig.next_frame_at(745.0);
        rig.app.test_edit = Some(SongEdit::Replaced);
        rig.frame(vec![]);
        assert_eq!(act(&mut rig, 746.0), 0.0);
        assert!(close(act(&mut rig, 750.0), 4.0));
    }

    #[test]
    fn counted_time_is_no_unsaved_change_and_an_undo_or_redo_is() {
        // Only the song-changed flag counts as modified. A saved undo step or a param value change sets it; counting
        // time does not; an undo or redo does.
        let mut rig = Rig::new();
        rig.app.files.saved();
        let at = |rig: &mut Rig, t: f64, events: Vec<egui::Event>, edit: Option<SongEdit>| {
            rig.next_frame_at(t);
            rig.app.test_edit = edit;
            rig.frame(events);
        };
        // An edit that changes nothing saves no undo step: counted, but no change.
        at(&mut rig, 10.0, vec![], Some(SongEdit::Final));
        assert!(!rig.app.files.modified());
        // A param report is a value change.
        at(&mut rig, 15.0, vec![], Some(SongEdit::Param));
        assert_eq!(rig.app.song.file.meta.editing_seconds, 5.0);
        assert!(rig.app.files.modified());
        // An edit, then (saved in between) its undo and its redo.
        rig.app.files.saved();
        rig.app.song.tempo_bpm = 99.0;
        at(&mut rig, 20.0, vec![], Some(SongEdit::Final));
        assert!(rig.app.files.modified());
        for (t, keys) in [
            (30.0, egui::Modifiers::NONE),
            (31.0, egui::Modifiers::SHIFT),
        ] {
            rig.app.files.saved();
            at(&mut rig, t, z(keys), None);
            assert!(rig.app.files.modified(), "{keys:?}");
        }
        assert_eq!(rig.app.song.file.meta.editing_seconds, 21.0);
        // Saved, counting on is no change, nor is the time's reset.
        rig.app.files.saved();
        at(&mut rig, 32.0, vec![], Some(SongEdit::Final));
        rig.app.song.file.meta.editing_seconds = 0.0;
        at(&mut rig, 33.0, vec![], Some(SongEdit::Final));
        assert!(!rig.app.files.modified());
    }

    #[test]
    fn the_app_starts_with_a_song_created_now() {
        let app = HoneydewApp::with_storage(None);
        let created = &app.song.file.meta.created;
        assert!(
            crate::ui::main_menu::meta_secs(created).is_some(),
            "{created}"
        );
        assert!(!app.files.modified());
    }

    #[test]
    fn project_length_is_where_the_last_clip_ends_at_the_song_tempo() {
        // it always read `0m 0s`. 8 beats at 120 BPM end at 4 s.
        let mut rig = Rig::with_synth();
        rig.app.song.tempo_bpm = 120.0;
        rig.app.song.channels[1].lanes[0]
            .clips
            .push(crate::song::Clip {
                id: 1,
                start_tick: 4 * crate::song::PPQ,
                visible_len_beats: 4.0,
                loop_len_beats: 4.0,
                offset_beats: 0.0,
                events: Vec::new(),
                muted: false,
                file: Default::default(),
            });
        rig.open_menu(MenuTab::Project);
        assert!(rig.painted("0m 4s"));
    }

    #[test]
    fn project_save_new_always_asks_for_a_name() {
        let (mut rig, _) = rig_with_file("/songs/Beat.flm", &synthetic_file(1, FORMAT_VERSION, ""));
        command(&mut rig, egui::Key::O);
        rig.open_menu(MenuTab::Project);
        rig.click_id(project_id("Save New"));
        assert_eq!(
            rig.overlays(),
            [
                Overlay::MainMenu(MenuTab::Project),
                Overlay::SaveDialog {
                    name: "Beat".into()
                }
            ]
        );
    }

    #[test]
    fn hide_and_lock_each_ask_once_and_set_the_songs_meta() {
        // `Hide forever` / Cancel sets META@256 = −1, `Lock forever` / Cancel sets META@776. Neither is an undo step,
        // and neither sets the song-changed flag.
        let mut rig = Rig::new();
        rig.app.song.file.meta.editing_seconds = 5.0;
        rig.app.files.saved();
        rig.open_menu(MenuTab::Project);
        let choices = |rig: &Rig| match &rig.overlays()[..] {
            [_, Overlay::Message { confirm, .. }] => confirm.clone(),
            _ => None,
        };
        let pair = |yes: &str| Some((yes.to_owned(), "Cancel".to_owned()));
        // Three lines, then the two choices: rows 3 and 4.
        let hide = project_id("Hide");
        rig.click_id(hide);
        assert_eq!(asking(&rig), Some("main_menu_hide_time"));
        assert_eq!(choices(&rig), pair("Hide forever"));
        choose(&mut rig, 4);
        rig.frame(vec![]);
        assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Project)]);
        assert_eq!(rig.app.song.file.meta.editing_seconds, 5.0);
        rig.click_id(hide);
        choose(&mut rig, 3);
        rig.frame(vec![]);
        assert_eq!(rig.app.song.file.meta.editing_seconds, -1.0);
        assert!(rig.painted("Hidden") && !rig.painted("Hide"));
        // The lock button is below the fold.
        let content = crate::ui::main_menu::shell_layout(
            egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0)),
            &Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE),
        )
        .content;
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -5000.0),
            modifiers: Default::default(),
        };
        rig.frame(vec![egui::Event::PointerMoved(content.center()), wheel]);
        for _ in 0..60 {
            rig.frame(vec![]);
        }
        let lock = project_id("lock");
        rig.click_id(lock);
        assert_eq!(asking(&rig), Some("main_menu_lock"));
        assert_eq!(choices(&rig), pair("Lock forever"));
        choose(&mut rig, 4);
        rig.frame(vec![]);
        assert!(!rig.app.song.file.meta.locked);
        rig.click_id(lock);
        choose(&mut rig, 3);
        rig.frame(vec![]);
        assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Project)]);
        assert!(rig.app.song.file.meta.locked);
        assert!(!rig.app.files.modified());
        assert!(!rig.app.history.can_undo());
        // Locked and hidden: the time rows and the lock button are gone.
        assert!(!rig.painted("Hidden") && !rig.painted("Lock metadata forever"));
    }

    fn closes(out: &egui::FullOutput) -> bool {
        sends(out, &egui::ViewportCommand::Close)
    }

    fn sends(out: &egui::FullOutput, command: &egui::ViewportCommand) -> bool {
        out.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(command)
    }

    /// The tag of the message shown over the main menu.
    fn asking(rig: &Rig) -> Option<&'static str> {
        match &rig.overlays()[..] {
            [Overlay::MainMenu(_), Overlay::Message { tag, .. }] => Some(*tag),
            _ => None,
        }
    }

    /// Picks the shown one-line question's choice at `row`: its first choice is on row 1.
    fn choose(rig: &mut Rig, row: usize) {
        rig.frame(vec![]);
        rig.click_id(message_item(row));
    }

    /// Whether the frame after next paints `texts` in this order (a message's lines, wrapped or not, then its
    /// choices left to right). A message shows from its second frame.
    fn shows_in_order(rig: &mut Rig, texts: &[&str]) -> bool {
        rig.frame(vec![]);
        let shown = rig.texts().join(" ");
        let mut rest = shown.as_str();
        texts.iter().all(|text| match rest.find(text) {
            Some(at) => {
                rest = &rest[at + text.len()..];
                true
            }
            None => false,
        })
    }

    // ---- Quit, and the unsaved-changes question ----

    #[test]
    fn closing_the_window_asks_exit_and_closes_on_yes() {
        let mut rig = Rig::new();
        rig.app.add_default_drums(0);
        rig.close_request = true;
        rig.frame(vec![]);
        assert!(sends(&rig.out, &egui::ViewportCommand::CancelClose));
        assert!(matches!(
            rig.overlays()[..],
            [Overlay::Message {
                tag: "main_menu_quit",
                ..
            }]
        ));
        let exit = format!("Exit {}?", crate::APP_NAME);
        assert!(shows_in_order(&mut rig, &[&exit, "Yes", "Cancel"]));
        choose(&mut rig, 2);
        assert!(
            !closes(&rig.out) && !closes(&rig.output(vec![])),
            "Cancel closed it"
        );
        assert_eq!(rig.overlays(), []);
        // Asked again; Yes closes, and the close request that follows is let through.
        rig.close_request = true;
        rig.frame(vec![]);
        assert!(sends(&rig.out, &egui::ViewportCommand::CancelClose));
        choose(&mut rig, 1);
        assert!(closes(&rig.out));
        rig.close_request = true;
        rig.frame(vec![]);
        assert!(!sends(&rig.out, &egui::ViewportCommand::CancelClose));
    }

    /// BYO-b: an import that ends while another screen asks something waits for the answer.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_failed_import_waits_for_the_quit_question() {
        use crate::project_files::content::tests::{Temp, put};
        let tmp = Temp::new("import-quit");
        put(
            &tmp.0
                .join(crate::project_files::CONTENT_FOLDER)
                .join("Gen.obb"),
            b"not a container",
        );
        let mut rig = Rig::new();
        rig.app.files = ProjectFiles::on_disk(&tmp.0);
        rig.app.settings.import_notice_accepted = true;
        rig.close_request = true;
        rig.frame(vec![]);
        rig.app.files.import_archives();
        let quit = |rig: &Rig| {
            matches!(
                rig.overlays()[..],
                [Overlay::Message {
                    tag: "main_menu_quit",
                    ..
                }]
            )
        };
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            rig.frame(vec![]);
            assert!(quit(&rig), "{:?}", rig.overlays());
        }
        choose(&mut rig, 2);
        for _ in 0..10 {
            rig.frame(vec![]);
        }
        assert!(
            matches!(
                rig.overlays()[..],
                [Overlay::Message {
                    tag: MESSAGE_TAG,
                    ..
                }]
            ),
            "{:?}",
            rig.overlays()
        );
    }

    /// The actions the unsaved-changes question guards.
    #[derive(Clone, Copy, Debug)]
    enum Guarded {
        /// SONGS `New`.
        New,
        /// The SONGS row `Groove.flm`.
        Row,
        /// SONGS `Import`, which picks `/songs/Beat.flm`.
        Import,
        /// `Ctrl+O`, which picks the same.
        CtrlO,
    }

    const GUARDED: [Guarded; 4] = [Guarded::New, Guarded::Row, Guarded::Import, Guarded::CtrlO];

    impl Guarded {
        fn ask(self, rig: &mut Rig) {
            let id = match self {
                Guarded::CtrlO => return command(rig, egui::Key::O),
                Guarded::New => songs_id(("big", 0_usize)),
                Guarded::Row => row("Groove.flm"),
                Guarded::Import => songs_id(("big", 2_usize)),
            };
            rig.open_menu(MenuTab::Songs);
            rig.click_id(id);
            // `New` asks for the song options first.
            if let Guarded::New = self {
                rig.click_id(egui::Id::new(crate::ui::song_options::ID).with("ok"));
            }
        }

        /// The number of channels of the song it opens.
        fn channels(self) -> usize {
            match self {
                Guarded::New => 1,
                Guarded::Row => 4,
                Guarded::Import | Guarded::CtrlO => 2,
            }
        }
    }

    /// `/songs/Beat.flm` (one drum channel) open and edited to three channels, `Groove.flm` (three drum channels)
    /// in the library, and no command logged.
    fn edited_rig() -> (Rig, MemFiles) {
        let beat = synthetic_file(1, FORMAT_VERSION, "");
        let (mut rig, mem) = rig_with_file("/songs/Beat.flm", &beat);
        let groove = synthetic_file(3, FORMAT_VERSION, "");
        rig.app.files.library.store("Groove.flm", &groove).unwrap();
        command(&mut rig, egui::Key::O);
        // The playlist opens it, writing its clamped view zooms back (HEAD@296).
        rig.frame(vec![]);
        rig.app.add_default_drums(0);
        rig.take();
        (rig, mem)
    }

    /// The overlays but the main menu.
    fn shown(rig: &Rig) -> Vec<Overlay> {
        let mut overlays = rig.overlays();
        overlays.retain(|o| !matches!(o, Overlay::MainMenu(_)));
        overlays
    }

    #[test]
    fn each_guarded_action_asks_save_discard_or_cancel_and_cancel_abandons_it() {
        for action in GUARDED {
            let (mut rig, mem) = edited_rig();
            let edited = rig.app.song.clone();
            action.ask(&mut rig);
            assert!(
                matches!(
                    shown(&rig)[..],
                    [Overlay::Message {
                        tag: DISCARD_TAG,
                        ..
                    }]
                ),
                "{action:?}"
            );
            assert!(
                shows_in_order(&mut rig, &[UNSAVED, "Save", "Discard changes", "Cancel"]),
                "{action:?}"
            );
            choose(&mut rig, 3);
            rig.frame(vec![]);
            assert_eq!(rig.app.song, edited, "{action:?}");
            assert_eq!(shown(&rig), [], "{action:?}");
            assert_eq!(rig.take(), Vec::<String>::new(), "{action:?}");
            assert_eq!(mem.0.borrow().saves, [], "{action:?}");
        }
    }

    #[test]
    fn discard_changes_runs_the_guarded_action() {
        for action in GUARDED {
            let (mut rig, mem) = edited_rig();
            action.ask(&mut rig);
            choose(&mut rig, 2);
            rig.frame(vec![]);
            assert_eq!(rig.app.song.channels.len(), action.channels(), "{action:?}");
            assert_eq!(rig.overlays(), [], "{action:?}");
            assert_eq!(rig.take(), ["Stop", "ReplaceSong"], "{action:?}");
            assert_eq!(mem.0.borrow().saves, [], "{action:?}");
        }
    }

    #[test]
    fn save_writes_the_song_without_asking_then_runs_the_guarded_action_and_flashes_saved() {
        for action in GUARDED {
            let (mut rig, mem) = edited_rig();
            action.ask(&mut rig);
            choose(&mut rig, 1);
            rig.frame(vec![]);
            // A quick save to the song's own file: no Save dialog, even had it none.
            let path = PathBuf::from("/songs/Beat.flm");
            assert_eq!(mem.0.borrow().saves, [(path.clone(), false)], "{action:?}");
            let two_drums = synthetic_file(2, FORMAT_VERSION, "");
            assert_eq!(mem.0.borrow().files[&path], two_drums, "{action:?}");
            // Then the action, on the saved file for Import and Ctrl+O.
            let channels = match action {
                Guarded::Import | Guarded::CtrlO => 3,
                _ => action.channels(),
            };
            assert_eq!(rig.app.song.channels.len(), channels, "{action:?}");
            assert_eq!(rig.overlays(), [], "{action:?}");
            assert_eq!(rig.take(), ["Stop", "ReplaceSong"], "{action:?}");
            assert!(!rig.app.files.modified(), "{action:?}");
            assert!(rig.texts().iter().any(|t| t == "Saved!"), "{action:?}");
        }
    }

    #[test]
    fn saved_flashes_for_a_second_and_a_half() {
        let (mut rig, _) = edited_rig();
        Guarded::New.ask(&mut rig);
        choose(&mut rig, 1);
        // The frame the save was made in; `texts` paints the next one, 0.1 s on.
        let saved = rig.time;
        rig.time = saved + 1.39 - 0.1;
        assert!(
            rig.texts().iter().any(|t| t == "Saved!"),
            "gone before 1.5 s"
        );
        rig.time = saved + 1.51 - 0.1;
        assert!(
            !rig.texts().iter().any(|t| t == "Saved!"),
            "still there after 1.5 s"
        );
    }

    /// What the next frame paints for the flash `text`: its box, the text's rect and its galley.
    fn painted_flash(
        rig: &mut Rig,
        text: &str,
    ) -> (egui::Rect, egui::Rect, std::sync::Arc<egui::Galley>) {
        rig.app.files.flash(text);
        let shapes = rig.output(vec![]).shapes;
        let (pos, galley) = shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == text => Some((t.pos, t.galley.clone())),
                _ => None,
            })
            .expect("the flash is painted");
        let text_rect = galley.rect.translate(pos.to_vec2());
        let fill = rig.app.theme.colors.text.to_color32();
        let boxed = shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == fill && r.rect.contains_rect(text_rect) => {
                    Some(r.rect)
                }
                _ => None,
            })
            .expect("the flash is on its box");
        (boxed, text_rect, galley)
    }

    #[test]
    fn a_flash_of_two_lines_is_centred_on_the_screen_with_both_lines_centred() {
        let mut rig = Rig::new();
        let (boxed, text, galley) = painted_flash(&mut rig, "Render complete\nSaved to: /x/y.wav");
        assert_eq!(galley.rows.len(), 2);
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
        assert!(
            (boxed.center() - screen.center()).length() <= 1.0,
            "{boxed:?}"
        );
        assert!(
            (text.center() - boxed.center()).length() <= 1.0,
            "{text:?} in {boxed:?}"
        );
        // The shorter first line is centred over the second, not flush left.
        let [first, second] = [0, 1].map(|i| galley.rows[i].rect().center().x);
        assert!((first - second).abs() <= 1.0, "{first} vs {second}");
        // One line is still one line, and the box grows with the lines.
        let (one, _, galley) = painted_flash(&mut rig, "Saved!");
        assert_eq!(galley.rows.len(), 1);
        assert!(one.height() < boxed.height());
    }

    #[test]
    fn a_long_flash_of_words_wraps_between_words() {
        let mut rig = Rig::new();
        let text = format!("Render complete\nSaved to: {}", "mixdown ".repeat(80));
        let (boxed, _, galley) = painted_flash(&mut rig, text.trim_end());
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
        assert!(galley.rows.len() > 2, "{} rows", galley.rows.len());
        assert!(screen.contains_rect(boxed), "{boxed:?}");
        for row in &galley.rows {
            let line: String = row.glyphs.iter().map(|g| g.chr).collect();
            for word in line.split_whitespace() {
                assert!(
                    ["Render", "complete", "Saved", "to:", "mixdown"].contains(&word),
                    "broken word {word:?} in {line:?}"
                );
            }
        }
    }

    #[test]
    fn a_flash_with_a_long_path_wraps_inside_the_screen() {
        let mut rig = Rig::new();
        // A path with no space in it: egui breaks the over-long word itself.
        let path = format!("/home/user/My Tracks/{}/Mix.wav", "x".repeat(300));
        let (boxed, _, galley) =
            painted_flash(&mut rig, &format!("Render complete\nSaved to: {path}"));
        let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
        assert!(galley.rows.len() > 2, "{} rows", galley.rows.len());
        assert!(screen.contains_rect(boxed), "{boxed:?}");
    }

    #[test]
    fn a_failed_save_shows_its_message_and_drops_the_guarded_action() {
        for action in GUARDED {
            let (mut rig, mem) = edited_rig();
            mem.0.borrow_mut().write_error = Some("disk full".into());
            let edited = rig.app.song.clone();
            action.ask(&mut rig);
            choose(&mut rig, 1);
            rig.frame(vec![]);
            assert_eq!(
                shown(&rig),
                [Overlay::Message {
                    tag: MESSAGE_TAG,
                    lines: vec!["Failed to save file".into(), "disk full".into()],
                    confirm: None,
                    extra: None,
                }],
                "{action:?}"
            );
            // `OK`, and nothing more happens, not even once the next save works: Ctrl+S only saves.
            choose(&mut rig, 2);
            rig.frame(vec![]);
            assert_eq!(shown(&rig), [], "{action:?}");
            mem.0.borrow_mut().write_error = None;
            command(&mut rig, egui::Key::S);
            rig.frame(vec![]);
            assert_eq!(mem.0.borrow().saves.len(), 2, "{action:?}");
            let song = &rig.app.song;
            assert_eq!(*song, with_stamp(&edited, song), "{action:?}");
            assert_eq!(shown(&rig), [], "{action:?}");
            assert_eq!(rig.take(), Vec::<String>::new(), "{action:?}");
            assert!(!rig.texts().iter().any(|t| t == "Saved!"), "{action:?}");
        }
    }

    #[test]
    fn an_unmodified_song_runs_the_guarded_action_at_once() {
        for action in GUARDED {
            let (mut rig, mem) = edited_rig();
            command(&mut rig, egui::Key::S);
            rig.take();
            action.ask(&mut rig);
            assert_eq!(rig.overlays(), [], "{action:?}");
            let channels = match action {
                Guarded::Import | Guarded::CtrlO => 3,
                _ => action.channels(),
            };
            assert_eq!(rig.app.song.channels.len(), channels, "{action:?}");
            assert_eq!(rig.take(), ["Stop", "ReplaceSong"], "{action:?}");
            assert_eq!(mem.0.borrow().saves.len(), 1, "{action:?}: Ctrl+S only");
        }
    }

    // ---- the mixer view, the transport extras and undo / redo ----

    use crate::song::ChannelParams;
    use crate::ui::keyboard_settings::{SCALES, scale_to_stored};

    /// The BPM button; in the tempo dialog (over the whole screen) the dial's centre, the screen's.
    const BPM: Pos2 = pos2(585.0 + 1.75 * U, BAR_Y);
    const TEMPO_DIAL: Pos2 = pos2(585.0, 270.0);

    /// On the dial, 1 U out: 3, 6, 9 and 12 o'clock for `quarter` 0 to 3 (and on round), so that each next one is a
    /// quarter turn clockwise, +0.25 BPM.
    fn dial(quarter: usize) -> Pos2 {
        TEMPO_DIAL + [vec2(U, 0.0), vec2(0.0, U), vec2(-U, 0.0), vec2(0.0, -U)][quarter % 4]
    }
    /// In the keyboard settings strip (from the keys' band top, 3 U under the 1.75 U button bar, the keys below it; six
    /// 5 U cells from x 105): the key spinner's `v` and the scale spinner's `^`.
    const KEY_DOWN: Pos2 = pos2(345.0, KEYS_TOP + 79.0);
    const SCALE_UP: Pos2 = pos2(505.0, KEYS_TOP + 33.0);

    /// Ctrl/Cmd+Z with `extra` held.
    fn z(extra: egui::Modifiers) -> Vec<egui::Event> {
        key_tap(egui::Key::Z, egui::Modifiers::COMMAND | extra)
    }

    fn ink() -> egui::Color32 {
        crate::theme::theme_by_id(0).colors.text.to_color32()
    }

    impl Rig {
        /// The colour the last frame painted `text` in, if it painted it.
        fn color_of(&self, text: &str) -> Option<egui::Color32> {
            self.out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(t.galley.job.sections[0].format.color)
                }
                _ => None,
            })
        }

        fn painted(&self, text: &str) -> bool {
            self.color_of(text).is_some()
        }

        /// Clicks the transport's text button `label`.
        fn click_text_button(&mut self, label: &str) {
            self.click_id(egui::Id::new(("transport", label)));
        }
    }

    #[test]
    fn the_mixer_writes_channel_params_and_sends_them_to_the_channel_strip() {
        let mut rig = Rig::with_drums_and_synth();
        rig.click(OUTSIDE);
        rig.app.navigate(Nav::Show(View::Mixer));
        rig.frame(vec![]);
        rig.take();
        // Channel 1's fader (its strip spans x 128..256, the fader y 24..181), dragged down by 1 U.
        let fader = pos2(192.0, 100.0);
        rig.press(fader);
        rig.drag_to(fader + vec2(0.0, U));
        rig.release(fader + vec2(0.0, U));
        let level = rig.app.song.channels[1].params.0[0];
        assert!(level < ChannelParams::default().level(), "{level}");
        let params = std::mem::take(&mut rig.rec.0.borrow_mut().params);
        assert_eq!(params.last(), Some(&(1, 0, 0, level)));
        assert!(
            params
                .iter()
                .all(|&(c, i, idx, _)| (c, i, idx) == (1, 0, 0))
        );
        // Live: no engine rebuild.
        let log = rig.take();
        assert!(
            !log.is_empty() && log.iter().all(|c| c.starts_with("SetParam ")),
            "{log:?}"
        );
        // A strip's bottom cell (1.5 U over the panel) selects its channel for the bottom panel (the Drums channel's
        // pads); the master's selects the master.
        rig.click(pos2(64.0, PANEL_TOP - 0.75 * U));
        assert_eq!(rig.app.selected_channel, 0);
        rig.click(pos2(192.0, PANEL_TOP - 0.75 * U));
        assert_eq!(rig.app.selected_channel, 1);
        rig.press(pad_centre(0));
        rig.release(pad_centre(0));
        assert_eq!(rig.take(), ["NoteOn 1 48 127", "NoteOff 1 48"]);
    }

    #[test]
    fn a_tempo_drag_is_one_edit_rebuilt_once_and_undone_at_once() {
        let mut rig = Rig::with_synth();
        rig.frame(vec![]);
        assert!(rig.painted("120"));
        rig.click(BPM);
        assert_eq!(rig.overlays(), [Overlay::TempoEditor]);
        rig.frame(vec![]);
        // A full turn clockwise: +1 BPM.
        rig.press(dial(0));
        for quarter in 1..=4 {
            rig.drag_to(dial(quarter));
        }
        assert!((rig.app.song.tempo_bpm - 121.0).abs() < 1e-9);
        // The engine takes the tempo live, nothing rebuilt mid-drag.
        let live = [
            "SetTempo 120.250",
            "SetTempo 120.500",
            "SetTempo 120.750",
            "SetTempo 121.000",
        ];
        assert_eq!(rig.take(), live);
        rig.release(dial(4));
        assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
        rig.frame(vec![]);
        assert!(rig.painted("121"), "the transport shows the new tempo");
        // The dialog is modal: undo waits until it closes (the BPM button under it is swallowed too).
        rig.frame(z(egui::Modifiers::NONE));
        assert!((rig.app.song.tempo_bpm - 121.0).abs() < 1e-9);
        rig.click(BPM);
        assert_eq!(rig.overlays(), [Overlay::TempoEditor]);
        rig.frame(escape());
        assert_eq!(rig.overlays(), []);
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!(rig.app.song.tempo_bpm, 120.0);
        assert_eq!(rig.take(), ["ReplaceSong adopting"]);
        // One entry: the next undo is the edit before the drag, adding the synth.
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!(rig.app.song.channels.len(), 1);
        for _ in 0..2 {
            rig.frame(z(egui::Modifiers::SHIFT));
        }
        assert!((rig.app.song.tempo_bpm - 121.0).abs() < 1e-9);
        let redone = ["ReplaceSong adopting"; 3];
        assert_eq!(rig.take(), redone);
    }

    #[test]
    fn the_tempo_dialog_swallows_a_touch_on_its_disc_and_closes_on_one_off_it() {
        let mut rig = Rig::with_synth();
        rig.click(BPM);
        rig.frame(vec![]);
        assert_eq!(rig.overlays(), [Overlay::TempoEditor]);
        // On its 7.5 U disc, over the tracks and the transport, off the dial and the buttons: nothing happens, to the
        // dialog or to what is under it.
        for at in [
            pos2(TEMPO_DIAL.x, TEMPO_DIAL.y - 6.0 * U),
            pos2(TEMPO_DIAL.x, TEMPO_DIAL.y + 7.0 * U),
            BPM,
        ] {
            rig.click(at);
            assert_eq!(rig.overlays(), [Overlay::TempoEditor], "{at:?}");
            assert_eq!(rig.take(), Vec::<String>::new());
        }
        // 7.6 U left: it closes, and reaches nothing below.
        rig.click(pos2(TEMPO_DIAL.x - 7.6 * U, TEMPO_DIAL.y));
        assert_eq!(rig.overlays(), []);
        assert_eq!(rig.take(), Vec::<String>::new());
    }

    #[test]
    fn escape_mid_drag_closes_the_tempo_editor_and_ends_the_gesture() {
        let mut rig = Rig::with_synth();
        rig.click(BPM);
        rig.frame(vec![]);
        rig.press(dial(0));
        rig.drag_to(dial(1));
        rig.frame(escape());
        assert_eq!(rig.overlays(), []);
        assert_eq!(rig.app.song.tempo_bpm, 120.25);
        let ended = ["SetTempo 120.250", "ReplaceSong adopting no-resync"];
        assert_eq!(rig.take(), ended);
        rig.release(dial(1));
        assert_eq!(rig.take(), Vec::<String>::new());
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!(rig.app.song.tempo_bpm, 120.0);
    }

    #[test]
    fn ctrl_z_mid_drag_waits_under_the_modal_tempo_editor() {
        // The tempo editor is modal: Ctrl+Z mid-drag does nothing, and the drag ends as one edit.
        let mut rig = Rig::with_synth();
        rig.click(BPM);
        rig.frame(vec![]);
        rig.press(dial(0));
        rig.drag_to(dial(1));
        rig.frame(z(egui::Modifiers::NONE));
        assert!((rig.app.song.tempo_bpm - 120.25).abs() < 1e-9);
        assert_eq!(rig.take(), ["SetTempo 120.250"]);
        rig.drag_to(dial(2));
        rig.release(dial(2));
        assert!((rig.app.song.tempo_bpm - 120.5).abs() < 1e-9);
        let ended = ["SetTempo 120.500", "ReplaceSong adopting no-resync"];
        assert_eq!(rig.take(), ended);
        // Closed, one undo takes the whole drag back.
        rig.frame(escape());
        assert_eq!(rig.overlays(), []);
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!(rig.app.song.tempo_bpm, 120.0);
        assert_eq!(rig.app.song.channels.len(), 2);
    }

    #[test]
    fn the_undo_and_redo_buttons_step_through_the_history() {
        let mut rig = Rig::with_synth();
        rig.app.add_default_drums(0);
        rig.take();
        rig.frame(vec![]);
        rig.click_text_button("Undo");
        assert_eq!(rig.app.song.channels.len(), 2);
        // The selected channel went with it.
        assert_eq!(rig.app.selected_channel, 1);
        assert_eq!(rig.take(), ["ReplaceSong adopting"]);
        rig.frame(vec![]);
        rig.click_text_button("Redo");
        assert_eq!(rig.app.song.channels.len(), 3);
        assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    }

    #[test]
    fn a_narrow_window_takes_two_transport_rows_and_keeps_every_button() {
        let mut rig = Rig::with_synth();
        rig.app.song.channels[1].lanes[0].add_clip(0, 4.0);
        rig.edit(1);
        // At 100 % the 1170-point window is 36.6 U wide: one row. At 150 % it is 24.4 U, under the 27.25 U the right
        // group needs with the meters: two rows, the right group under the centre one.
        for (scale, rows) in [(1.0, 1), (1.5, 2)] {
            rig.app.settings.ui_scale = (scale * 100.0) as u32;
            rig.frame(vec![]);
            rig.frame(vec![]);
            assert_eq!(rig.app.main_screen.rows, rows, "{scale}");
            let rect = |label: &str| {
                let id = egui::Id::new(("transport", label));
                rig.ctx.read_response(id).unwrap().rect
            };
            let (control, undo, solo) = (rect("CONTROL"), rect("Undo"), rect("SOLO CLIP"));
            if rows == 2 {
                assert!(
                    undo.top() >= control.bottom() && solo.right() == undo.left(),
                    "{scale}"
                );
            } else {
                assert!(
                    undo.top() == control.top() && solo.left() > control.right(),
                    "{scale}"
                );
            }
            // CONTROL still takes its taps.
            rig.click_id(egui::Id::new(("transport", "CONTROL")));
            assert_eq!(rig.overlays(), [Overlay::CtrlMenu], "{scale}");
            rig.app.nav.overlays.clear();
        }
    }

    #[test]
    fn ctrl_z_undoes_and_ctrl_shift_z_or_ctrl_alt_z_redoes_but_not_in_a_text_field() {
        use egui::Modifiers;
        let mut rig = Rig::with_synth();
        rig.click(PLAY);
        rig.rec.0.borrow_mut().tick = 333;
        rig.take();
        rig.app.song.tempo_bpm = 99.0;
        rig.app.test_edit = Some(SongEdit::Final);
        rig.frame(vec![]);
        assert_eq!(rig.take(), ["ReplaceSong adopting no-resync", "Play"]);
        // An undo hands over the restored song as an edit does, and it always re-syncs.
        rig.frame(z(Modifiers::NONE));
        assert_eq!(rig.app.song.tempo_bpm, 120.0);
        assert_eq!(rig.take(), ["ReplaceSong adopting", "Play"]);
        for (extra, bpm) in [
            (Modifiers::SHIFT, 99.0),
            (Modifiers::NONE, 120.0),
            (Modifiers::ALT, 99.0),
        ] {
            rig.frame(z(extra));
            assert_eq!(rig.app.song.tempo_bpm, bpm, "{extra:?}");
        }
        // A plain Z is no shortcut.
        rig.frame(vec![key_event(egui::Key::Z, Modifiers::NONE)]);
        assert_eq!(rig.app.song.tempo_bpm, 99.0);
        // In the Save dialog's name field, Ctrl+Z is the field's own.
        rig.app.navigate(Nav::Open(Overlay::SaveDialog {
            name: "Song".into(),
        }));
        rig.frame(vec![]);
        rig.frame(z(Modifiers::NONE));
        assert_eq!(rig.app.song.tempo_bpm, 99.0);
    }

    #[test]
    fn a_new_song_starts_a_new_history_and_undo_keeps_the_file_name() {
        let mut rig = Rig::with_synth();
        rig.app.song.tempo_bpm = 99.0;
        rig.app.song_edited(SongEdit::Final, 0);
        // A save names the song after its file; an undo does not take that back.
        rig.app.song.title = "Beat".into();
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!(rig.app.song.tempo_bpm, 120.0);
        assert_eq!(rig.app.song.title, "Beat");
        rig.frame(z(egui::Modifiers::SHIFT));
        assert_eq!(rig.app.song.tempo_bpm, 99.0);
        // A loaded song has nothing to undo. (Its view zooms count per U, and the playlist writes the view it opened
        // at back into it, HEAD@296.)
        rig.app.song = Song::default();
        rig.app.song_edited(SongEdit::Replaced, 0);
        rig.take();
        rig.frame(z(egui::Modifiers::NONE));
        let mut new = Song::default();
        new.file.head_extra = vec![1];
        new.file.playlist_view = rig.app.song.file.playlist_view;
        assert_eq!(rig.app.song, new);
        assert_eq!(rig.take(), Vec::<String>::new());
    }

    #[test]
    fn the_ctrl_menu_lists_its_six_items_disabled_and_a_tap_outside_closes_it() {
        let mut rig = Rig::with_synth();
        rig.click(CTRL);
        // egui sizes the popup on its first frame and shows it on the next.
        rig.frame(vec![]);
        rig.frame(vec![]);
        let dim = crate::theme::theme_by_id(0)
            .colors
            .background
            .to_color32()
            .gamma_multiply(0.3);
        for item in [
            "Link to MIDI CC",
            "Link to Mod Wheel",
            "Unlink from MIDI CC",
            "Add automation track",
            "Link to recent control",
            "Fine tune control",
        ] {
            assert_eq!(rig.color_of(item), Some(dim), "{item}");
        }
        // Standing on the CTRL button (x 669..725, on the panel's top), six rows of 1.25 U: a disabled item does nothing.
        rig.click(pos2(697.0, PANEL_TOP - 2.5 * 40.0));
        assert_eq!(rig.overlays(), [Overlay::CtrlMenu]);
        // A tap outside closes it and reaches nothing below.
        rig.click(E4);
        assert_eq!(rig.overlays(), []);
        assert_eq!(rig.take(), Vec::<String>::new());
    }

    #[test]
    fn keyboard_settings_show_the_songs_key_lock_and_its_picks_are_undo_steps_without_a_rebuild() {
        let mut rig = Rig::with_synth();
        // A loaded song locked to D and to stored scale 40, past the id list, which reads as Chromatic.
        rig.app.song.key_root = 2;
        rig.app.song.key_scale = 40;
        rig.app.song_edited(SongEdit::Replaced, 0);
        rig.click(KEYBOARD_SETTINGS);
        assert_eq!(rig.overlays(), [Overlay::KeyboardSettings]);
        rig.frame(vec![]);
        rig.take();
        assert!(rig.painted("D") && rig.painted("Chromatic"));
        // Showing it rewrites nothing.
        assert_eq!((rig.app.song.key_root, rig.app.song.key_scale), (2, 40));
        assert!(!rig.app.history.can_undo());
        // `v` on the key: D# / Eb, an undo step that leaves the scale alone. The key lock changes no audio: no
        // rebuild.
        rig.click(KEY_DOWN);
        assert_eq!((rig.app.song.key_root, rig.app.song.key_scale), (3, 40));
        assert_eq!(rig.take(), Vec::<String>::new());
        // `^` on the scale: the one before Chromatic in the picker, stored by its own id.
        rig.click(SCALE_UP);
        assert_eq!(SCALES[19], "Locrian");
        assert_eq!(rig.app.song.key_scale, scale_to_stored(19));
        assert_eq!(rig.take(), Vec::<String>::new());
        // Undo steps back through both, one pick at a time, and the panel follows the song.
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!((rig.app.song.key_root, rig.app.song.key_scale), (3, 40));
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!((rig.app.song.key_root, rig.app.song.key_scale), (2, 40));
        assert!(!rig.app.history.can_undo());
        rig.frame(vec![]);
        assert!(rig.painted("D") && rig.painted("Chromatic"));
        rig.take();
        // The strip covers the top 3 U of the keyboard band: a press on it plays nothing, and the keys start under it.
        // (Held past a tap: a tap on its gaps toggles the panel's size.)
        let strip_bottom = KEYS_TOP + 3.0 * U;
        for y in [KEYS_TOP + 48.0, strip_bottom - 1.0] {
            rig.press(pos2(900.0, y));
            for _ in 0..3 {
                rig.frame(vec![]);
            }
            rig.release(pos2(900.0, y));
            assert_eq!(rig.take(), Vec::<String>::new(), "y {y}");
        }
        assert_eq!(rig.app.settings.preferred_kbd_size, PANEL_H);
        // (A key's top edge plays its least velocity: KP's velocity by depth.)
        rig.click(pos2(E4.x, strip_bottom + 1.0));
        let played = rig.take();
        assert!(
            played.len() == 2
                && played[0].starts_with("NoteOn 1 52 ")
                && played[1] == "NoteOff 1 52",
            "{played:?}"
        );
        // Escape closes an open spinner popup first, then the panel.
        rig.click(pos2(KEY_DOWN.x, KEYS_TOP + 60.0));
        rig.frame(vec![]);
        assert!(rig.painted("G# / Ab"), "the key popup is open");
        rig.frame(escape());
        assert_eq!(rig.overlays(), [Overlay::KeyboardSettings]);
        rig.frame(vec![]);
        assert!(!rig.painted("G# / Ab"), "the key popup closed");
        rig.frame(escape());
        assert_eq!(rig.overlays(), []);
    }

    #[test]
    fn the_ram_readout_shows_the_app_memory_and_asks_for_a_frame_a_second() {
        let mut rig = Rig::new();
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        let memory = rig.out.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text().ends_with(" MB") => {
                Some(t.galley.text().to_owned())
            }
            _ => None,
        });
        assert!(
            memory.as_ref().is_some_and(|m| m != "0 MB" && m != "-- MB"),
            "{memory:?}"
        );
        // A second, less egui's predicted frame time.
        let delay = rig.out.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        assert!(
            delay.as_secs_f32() > 0.9 && delay.as_secs_f32() <= 1.0,
            "{delay:?}"
        );
    }

    #[test]
    fn hiding_the_meters_hides_the_cpu_and_ram_readouts_and_stops_their_frame_a_second() {
        let mut rig = Rig::new();
        rig.app.settings.hardware_meters = false;
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        assert!(!rig.texts().iter().any(|t| t.ends_with(" MB")));
        let delay = rig.out.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        assert!(delay.as_secs_f32() > 1.0, "{delay:?}");
    }

    // ---- Channel param controls save no undo step, without a rebuild ----

    /// Channel 1's strip in the mixer (x 128..256): its fader, and its Mute pill (the 1.75 U mute/solo row over the
    /// 1.5 U bottom cell, which sits on the panel's top).
    const FADER_1: Pos2 = pos2(192.0, 100.0);
    const MUTE_1: Pos2 = pos2(160.0, PANEL_TOP - (1.5 + 1.75 / 2.0) * U);

    /// [`Rig::with_drums_and_synth`] in the mixer, its audio started, with an empty log. Its two undo steps are the
    /// channel adds.
    fn mixer_rig() -> Rig {
        let mut rig = Rig::with_drums_and_synth();
        rig.click(OUTSIDE);
        rig.app.navigate(Nav::Show(View::Mixer));
        rig.frame(vec![]);
        rig.take();
        rig
    }

    /// Channel 1's param `index` (0 Level, 1 Pan, 2 Mute).
    fn param(rig: &Rig, index: usize) -> f32 {
        rig.app.song.channels[1].params.0[index]
    }

    #[test]
    fn a_fader_drag_saves_no_undo_step_and_no_undo_or_redo_puts_its_value_back() {
        let mut rig = mixer_rig();
        rig.app.files.saved();
        let default = ChannelParams::default().level();
        rig.press(FADER_1);
        rig.drag_to(FADER_1 + vec2(0.0, 0.5 * U));
        assert!(rig.app.files.modified(), "at its first change");
        rig.drag_to(FADER_1 + vec2(0.0, U));
        rig.release(FADER_1 + vec2(0.0, U));
        let dragged = param(&rig, 0);
        assert!(dragged < default, "{dragged}");
        // Live during the drag and at its end: no rebuild.
        let log = rig.take();
        assert!(
            !log.is_empty() && log.iter().all(|c| c.starts_with("SetParam ")),
            "{log:?}"
        );
        // The undos are the two channel adds; the redo of channel 1's brings it back with the fader where it is.
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!((rig.app.song.channels.len(), param(&rig, 0)), (2, dragged));
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!(rig.app.song.channels.len(), 1);
        rig.frame(z(egui::Modifiers::SHIFT));
        assert_eq!((rig.app.song.channels.len(), param(&rig, 0)), (2, dragged));
    }

    #[test]
    fn a_released_fader_keeps_its_value_through_its_channels_delete_and_the_undo() {
        // The frame that ends the gesture gives the undo history the value, before the channel may go: the delete's own
        // sync no longer finds it.
        let mut rig = mixer_rig();
        rig.press(FADER_1);
        rig.drag_to(FADER_1 + vec2(0.0, U));
        rig.release(FADER_1 + vec2(0.0, U));
        rig.frame(vec![]);
        let dragged = param(&rig, 0);
        assert_ne!(dragged, ChannelParams::default().level());
        rig.app.song.delete_channel(1);
        rig.app.song_edited(SongEdit::Final, 0);
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!((rig.app.song.channels.len(), param(&rig, 0)), (3, dragged));
    }

    #[test]
    fn a_tap_saves_no_undo_step() {
        let mut rig = mixer_rig();
        rig.drag_to(MUTE_1);
        rig.frame(vec![
            button_event(MUTE_1, true),
            button_event(MUTE_1, false),
        ]);
        assert_eq!(param(&rig, 2), 1.0);
        assert_eq!(rig.take().len(), 1);
        // The next undo is the MiniSynth add, and Mute stays on.
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!((rig.app.song.channels.len(), param(&rig, 2)), (2, 1.0));
    }

    #[test]
    fn a_param_edit_with_no_pointer_down_saves_no_undo_step() {
        // As a key or the wheel would make it: no pointer is ever down.
        let mut rig = mixer_rig();
        for pan in [0.25, 0.75] {
            rig.app.song.channels[1].params.0[1] = pan;
            rig.app.test_edit = Some(SongEdit::ChannelParam);
            rig.frame(vec![]);
        }
        assert_eq!(rig.take(), Vec::<String>::new());
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!((rig.app.song.channels.len(), param(&rig, 1)), (2, 0.75));
    }

    #[test]
    fn an_undo_mid_fader_drag_undoes_the_step_before_and_keeps_the_drag() {
        let mut rig = mixer_rig();
        let default = ChannelParams::default().level();
        rig.press(FADER_1);
        rig.drag_to(FADER_1 + vec2(0.0, U));
        let dragged = param(&rig, 0);
        assert!(dragged < default, "{dragged}");
        rig.take();
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!((rig.app.song.channels.len(), param(&rig, 0)), (2, dragged));
        assert_eq!(rig.take(), ["ReplaceSong adopting"]);
        // The undo cancelled the drag: its rest does nothing.
        rig.drag_to(FADER_1 + vec2(0.0, 2.0 * U));
        rig.release(FADER_1 + vec2(0.0, 2.0 * U));
        assert_eq!(param(&rig, 0), dragged);
        rig.frame(z(egui::Modifiers::SHIFT));
        assert_eq!((rig.app.song.channels.len(), param(&rig, 0)), (3, dragged));
    }
}

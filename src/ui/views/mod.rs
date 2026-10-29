//! Navigation: the view in the editor area above the transport bar, the overlays stacked over the screen, and the
//! [`Nav`] actions that switch them.
//!
//! Each view and overlay is a slot in its own file: a `State` the app keeps in [`Slots`] and a `show` the app
//! calls every frame with an [`AppCtx`], returning the navigation it asks for. The playlist view is
//! `MainScreen::playlist`.
//!
//! A slot that edits the song reports it with [`AppCtx::song_edited`] (or [`AppCtx::song_replaced`]), never by
//! rebuilding the audio itself; a channel param's value sent as `SetParam` reports [`AppCtx::channel_param_edited`]. A
//! view whose channel no longer exists returns `Nav::Show(View::Playlist)`; an overlay whose channel is gone returns
//! `Nav::Close`.

use std::mem::discriminant;

use eframe::egui::{self};
use honeydew_audio::device::OutputSnapshot;

use crate::audio_link::AudioLink;
use crate::project_files::ProjectFiles;
use crate::settings::Settings;
use crate::song::{LaneKind, ModuleInstance, Song};
use crate::ui::UiCtx;
use crate::ui::main_screen::Regions;
use crate::ui::modules_browser::InsertAt;
use crate::ui::performance::Performance;
use crate::ui::piano_roll::PianoRoll;
use crate::ui::popup::{self, PopupItem};
use crate::ui::preview::ClipKey;

pub mod audio_editor;
pub mod automation_editor;
pub mod channel_settings;
pub mod ctrl_menu;
pub mod drum_editor;
pub mod keyboard_settings;
pub mod main_menu;
pub mod message;
pub mod midi_import;
pub mod mixer;
pub mod modules_browser;
pub mod piano_roll;
pub mod preset_picker;
pub mod rack;
pub mod sample_picker;
pub mod save_dialog;
pub mod settings_tab;
pub mod soundfont_picker;
pub mod tempo_editor;
mod wrap;
// The fine tune dialog.
pub mod fine_tune;
// The `Sound credits` screen.
pub mod credits;
// The What's New dialog.
pub mod whats_new;
// `Add MIDI CC track…`'s picker.
pub mod cc_picker;

/// What fills the editor area, between the top of the screen and the transport bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Playlist,
    /// The piano roll of a channel's note lane.
    PianoRoll(usize),
    /// The step sequencer of a channel's drum lane.
    DrumEditor(usize),
    Mixer,
    /// The automation editor of a channel's automation lane, on the clip it holds for the channel.
    AutomationEditor(usize),
    /// The audio clip editor of a channel's audio lane, on the clip it holds for the channel.
    AudioEditor(usize),
}

/// The main menu's tabs, in strip order: the menu's own.
pub use crate::ui::main_menu::Tab as MenuTab;

/// A panel, dialog or menu over the view. Several can be open; the topmost takes input first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlay {
    /// The main menu (logo corner), on a tab.
    MainMenu(MenuTab),
    /// The Save dialog, with its `File Name` field.
    SaveDialog { name: String },
    /// The keyboard settings panel (transport icon 48).
    KeyboardSettings,
    /// The `•••` CTRL menu.
    CtrlMenu,
    /// A channel's rack.
    Rack(usize),
    /// The Modules browser, adding to `channel`'s rack at `at`.
    ModulesBrowser { channel: usize, at: InsertAt },
    /// A message in the popup style: header `lines`, then `OK`, or the two choices of `confirm`
    /// (`(yes, no)` labels, e.g. `("Yes", "Cancel")`) with the `extra` choice above them, if any (`Save` in the
    /// unsaved-changes question; it needs `confirm`). `tag` names the question: its asker reads the answer with
    /// [`Navigation::take_answer`], or [`Navigation::take_choice`] when it has an extra choice. One message is open
    /// at a time; a new one replaces it.
    Message {
        tag: &'static str,
        lines: Vec<String>,
        confirm: Option<(String, String)>,
        extra: Option<String>,
    },
    /// The tempo editor (the transport's BPM button).
    TempoEditor,
    /// A channel's `Channel Settings` dialog (the track menu's), with its name field's text.
    ChannelSettings { channel: usize, name: String },
    /// The Save dialog of a preset (`Save preset`, `Save Rack preset`, `Save Drumset`), with its `File Name` field.
    /// What it saves is [`ProjectFiles`]'s.
    PresetDialog {
        name: String,
        kind: crate::project_files::PresetKind,
    },
    /// The preset picker, loading into `target` on `channel` (ignored for a new channel).
    PresetPicker {
        channel: usize,
        target: crate::ui::rack::PresetTarget,
    },
    /// The SoundFont picker of the SoundFont Player with instance id `module` on `channel` (its `Patch` spinner).
    SoundFontPicker { channel: usize, module: i32 },
    /// The sample picker, for `channel`'s drum pad `pad` (`Load Sample`), or for a new pad (`None`, the `+`).
    SamplePicker { channel: usize, pad: Option<u8> },
    /// The sample picker for the sampler slot of an audio clip (the sample row's icon button).
    ClipSamplePicker(ClipKey),
    /// The sample picker a tap on an empty cell of the current audio track opens: its first pick makes an audio clip on
    /// `channel`'s lane `lane` at song tick `tick`.
    TrackSamplePicker {
        channel: usize,
        lane: usize,
        tick: u32,
    },
    /// The audio editor's `New sample name` dialog (its save's `Save new`), with its `File Name` field.
    SampleName { name: String },
    /// The loading screen of an export under way, with its title ([`busy`]). The app keeps the title current and
    /// closes it.
    Busy { title: String },
    /// The sample picker for the module with instance id `module` on `channel`: the file its `CSTM` `PATH` names (a 3x
    /// Osc's custom waveform).
    ModuleSamplePicker { channel: usize, module: i32 },
    /// `Link to MIDI CC`'s prompt ([`LEARN_CC_PROMPT`], `Cancel`): while it is the topmost overlay the app waits for
    /// the next MIDI CC, which it links to the recent control.
    LearnCc,
    /// The fine tune dialog on param `target` of channel `channel` (`Fine tune control`).
    FineTune {
        channel: usize,
        target: crate::song::AutomationTarget,
    },
    /// The MIDI import dialog of `Import MIDI Tracks`. What it imports, and into which channel, is
    /// [`Slots::midi_import`]'s.
    MidiImport,
    /// The `Sound credits` screen (SETTINGS' `Sound credits`): the bundled sounds' sources and the app's own credits.
    Credits,
    /// The What's New dialog (Help → `What's new`, or by itself once per notes version).
    WhatsNew,
    /// `Add MIDI CC track…`'s picker of the MIDI CCs, for a lane on this channel.
    CcPicker(usize),
}

/// `Link to MIDI CC`'s prompt.
pub const LEARN_CC_PROMPT: &str =
    "Tweak a control on your MIDI controller to assign it to the parameter";

/// A navigation request, returned by views, overlays and the playlist screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nav {
    /// Shows a view and closes every overlay. Showing the view already shown goes back to the playlist, so the
    /// mixer button toggles the mixer.
    Show(View),
    /// [`Self::EditClip`] on a channel's main lane's clip ([`main_clip`]): the piano roll for a note lane, the drum
    /// editor for a drum lane, the audio editor for an audio lane. A main lane with no clip, and the other channels (the
    /// master and the sends, whose main lane is automation: a clip's `Edit` opens that), have no editor to open, and
    /// nothing happens.
    Edit(usize),
    /// Shows the editor of the clip's lane, on that clip (a playlist clip's `Edit`). The app makes it the playlist's
    /// current clip, which the bottom editor follows ([`Slots::follow`]), as it navigates.
    EditClip(ClipKey),
    /// Opens an overlay on top, closing any other overlay of the same kind.
    Open(Overlay),
    /// Closes the topmost overlay if it is of this kind, else opens it.
    Toggle(Overlay),
    /// From an overlay: closes that overlay, wherever it sits in the stack. From a view or the playlist screen:
    /// closes the topmost overlay.
    Close,
    /// The choice of the message tagged so, `true` for its yes. Closes the message; its asker takes the answer
    /// with [`Navigation::take_answer`].
    Answer(&'static str, bool),
    /// The message tagged so was answered with its extra choice. Closes the message; its asker takes the answer
    /// with [`Navigation::take_choice`].
    Extra(&'static str),
    /// From an overlay: closes that overlay and opens this one on top in its place, as [`Self::Open`] does. From a view
    /// or the playlist screen: the same for the topmost overlay.
    Swap(Overlay),
    /// The bottom editor collapses (its shared close X, or no current clip on the current track): the playlist alone,
    /// every overlay kept.
    Collapse,
}

/// A message's answer: its yes, its no (also Escape, a click outside, or the message closed unanswered), or its
/// extra choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Extra,
}

/// A yes / no answer.
impl From<bool> for Answer {
    fn from(yes: bool) -> Self {
        if yes { Self::Yes } else { Self::No }
    }
}

/// The view shown and the overlays open.
#[derive(Debug, Default)]
pub struct Navigation {
    pub view: View,
    /// Bottom to top: the last one is drawn last and takes input first.
    pub overlays: Vec<Overlay>,
    /// The last message's tag and answer, until its asker takes it; a new message clears it.
    answer: Option<(&'static str, Answer)>,
}

impl Navigation {
    /// Applies `nav`; `song` tells which editor a channel has.
    pub fn apply(&mut self, nav: Nav, song: &Song) {
        match nav {
            Nav::Show(view) => self.show(if view == self.view {
                View::Playlist
            } else {
                view
            }),
            Nav::Edit(channel) => {
                if let Some(key) = main_clip(song, channel) {
                    self.apply(Nav::EditClip(key), song);
                }
            }
            Nav::EditClip(key) => {
                if let Some(view) = editor(song, key.channel, key.lane) {
                    self.show(view);
                }
            }
            Nav::Open(overlay) => {
                if matches!(overlay, Overlay::Message { .. }) {
                    self.answer = None;
                }
                self.overlays
                    .retain(|o| discriminant(o) != discriminant(&overlay));
                self.overlays.push(overlay);
            }
            Nav::Toggle(overlay) => {
                if self
                    .overlays
                    .last()
                    .is_some_and(|top| discriminant(top) == discriminant(&overlay))
                {
                    self.overlays.pop();
                } else {
                    self.apply(Nav::Open(overlay), song);
                }
            }
            Nav::Close => {
                self.overlays.pop();
            }
            Nav::Answer(tag, yes) => self.answered(tag, yes.into()),
            Nav::Extra(tag) => self.answered(tag, Answer::Extra),
            Nav::Swap(overlay) => {
                self.overlays.pop();
                self.apply(Nav::Open(overlay), song);
            }
            Nav::Collapse => self.view = View::Playlist,
        }
    }

    fn answered(&mut self, tag: &'static str, answer: Answer) {
        self.overlays
            .retain(|o| !matches!(o, Overlay::Message { .. }));
        self.answer = Some((tag, answer));
    }

    /// The answer to the message tagged `tag`, for its asker to poll while it waits: `None` while the message
    /// is open, then its answer once. A message that is neither open nor answered (replaced by another, or
    /// closed by a view switch) reads as no.
    pub fn take_choice(&mut self, tag: &str) -> Option<Answer> {
        match self.answer {
            Some((t, answer)) if t == tag => {
                self.answer = None;
                Some(answer)
            }
            _ if self
                .overlays
                .iter()
                .any(|o| matches!(o, Overlay::Message { tag: t, .. } if *t == tag)) =>
            {
                None
            }
            _ => Some(Answer::No),
        }
    }

    /// [`Self::take_choice`] for a yes / no question: `true` for its yes.
    pub fn take_answer(&mut self, tag: &str) -> Option<bool> {
        self.take_choice(tag).map(|a| a == Answer::Yes)
    }

    /// The stored answer, left in place, `true` for a yes: what [`AppCtx::answer`] shows the slots.
    pub fn answer(&self) -> Option<(&'static str, bool)> {
        self.answer.map(|(tag, a)| (tag, a == Answer::Yes))
    }

    /// Applies `nav` returned by the overlay at `index` in [`Self::overlays`]: its `Close` closes that overlay, and its
    /// `Swap` too before it opens the other.
    pub fn apply_from(&mut self, index: usize, nav: Nav, song: &Song) {
        if matches!(nav, Nav::Close | Nav::Swap(_)) && index < self.overlays.len() {
            self.overlays.remove(index);
        }
        match nav {
            Nav::Close => {}
            Nav::Swap(overlay) => self.apply(Nav::Open(overlay), song),
            nav => self.apply(nav, song),
        }
    }

    fn show(&mut self, view: View) {
        self.view = view;
        self.overlays.clear();
    }
}

/// How a frame changed the song, from the weakest to the strongest. See [`AppCtx::song_edited`] and
/// [`AppCtx::param_edited`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SongEdit {
    /// A channel param's value from a control bound to it, which the audio already has (`SetParam`), or a rack card's
    /// settings or size: the song-changed flag, no undo step and no rebuild (the rack and presets are not covered by
    /// undo). No undo or redo puts an old one back ([`History::sync`]).
    ///
    /// [`History::sync`]: crate::history::History::sync
    ChannelParam,
    /// An edit the audio already has or never needs (a chord set, a track's zip): an undo step only, no rebuild.
    Param,
    /// Rack state the audio already has or takes at its next build, with no undo step and no rebuild: the drum strips'
    /// controls. Every step keeps it ([`History::rack_state`]).
    ///
    /// [`History::rack_state`]: crate::history::History::rack_state
    RackState,
    /// Mid-gesture (a drag): the song changed, the audio is not rebuilt yet.
    Ongoing,
    /// Rack state the audio reads only at build (a drum strip's `Shift`, `AUX`, a stretch): as [`Self::RackState`],
    /// and a new engine.
    RackRebuild,
    /// A completed edit that makes no undo step (a preset or sample pick): a new engine; every step keeps
    /// it, as [`Self::RackState`].
    Untracked,
    /// A completed edit that never re-syncs a playing transport (a tap's new note or hit, the step sequencer's Clear
    /// and Fill), else as [`Self::Final`].
    Quiet,
    /// A completed edit: the end of a drag, a menu action, a tap.
    Final,
    /// A completed edit that re-syncs a playing transport whatever it changed: a graph-lane touch that changed nothing
    /// (it undoes its own snapshot, and that undo re-syncs), the piano roll's Quantize, Humanize and Snap to
    /// scale. An undo step only if the song changed, as [`Self::Final`].
    Resync,
    /// A whole new song (open, new song).
    Replaced,
}

/// What the app lends a view or overlay for one frame.
pub struct AppCtx<'a> {
    pub cx: UiCtx<'a>,
    pub regions: Regions,
    /// Edit it, then report the edit with [`Self::song_edited`].
    pub song: &'a mut Song,
    /// Commands to the audio: notes and `SetParam` (then report [`Self::channel_param_edited`]). Song edits go through
    /// [`Self::song_edited`] instead.
    pub audio: &'a mut AudioLink,
    /// The channel the keyboard plays. The master (0) is only selected while it is alone.
    pub selected: &'a mut usize,
    /// This frame's output snapshot, if there is an output: transport tick, CPU load, meters.
    pub snapshot: Option<&'a OutputSnapshot>,
    /// The transport is playing; [`Self::play`] sets it, which the app applies after the view.
    pub playing: bool,
    /// REC is armed: the editors' playhead discs show icon 5 (record) in place of 3 (play).
    pub armed: bool,
    /// Preview plays the clip the shown editor edits.
    pub previewing: bool,
    /// Setting "Set middle C to C4".
    pub middle_c_is_c4: bool,
    /// Setting "Enable ghost notes" (the piano roll).
    pub ghost_notes: bool,
    /// The SETTINGS values (ghost notes, key names …). The settings tab edits them; the app applies and keeps
    /// what changed after the frame.
    pub settings: &'a mut Settings,
    /// Open / save: `open()`, `save(song)`, the Save dialog's `dialog_save`. What they show (the Save dialog,
    /// messages) opens after the frame.
    pub files: &'a mut ProjectFiles,
    /// The strongest edit reported this frame; the app applies it after the frame.
    pub edit: Option<SongEdit>,
    /// The last message's tag and answer ([`Navigation::answer`]), until another message opens. A slot that asked
    /// acts on the answer to its tag once (it remembers what it asked).
    pub answer: Option<(&'static str, bool)>,
    /// The pointer is over an overlay above the one drawing ([`covered`]): the wheel, zoom and pinch are not its.
    pub covered: bool,
    /// A ruler tap's song tick this frame ([`Self::set_playhead`]); the app applies it after the frame.
    pub playhead: Option<u64>,
    /// A playhead knob's drop this frame: the song tick to seek to, playing or stopped, as the playlist's knob does. The
    /// app applies it after the frame.
    pub seek: Option<u64>,
    /// The MIDI input ports at the app's last scan (`MidiLink::ports`), for SETTINGS' `Input device:` menu. The app
    /// reads the choice from `settings.midi_device` (`MidiChoice::from_label`).
    pub midi_devices: &'a [String],
    /// The `Input device:` dropdown opened this frame: the app lists the ports again once (`MidiLink::rescan`). The
    /// tap is a user gesture, which the web needs to open MIDI for the first time.
    pub midi_rescan: bool,
}

impl AppCtx<'_> {
    /// Reports an edit of [`Self::song`]. The app's one funnel for song edits applies it after the frame: an
    /// ongoing edit (`final_edit` false, mid-drag) only marks the song dirty; a final one hands the audio a
    /// rebuilt engine, resuming where the transport was if it is playing, and is where the undo history records
    /// a step.
    pub fn song_edited(&mut self, final_edit: bool) {
        let edit = if final_edit {
            SongEdit::Final
        } else {
            SongEdit::Ongoing
        };
        self.edit = self.edit.max(Some(edit));
    }

    /// Reports a completed edit of [`Self::song`] that never re-syncs a playing transport ([`SongEdit::Quiet`]). A final
    /// edit in the same frame takes it in.
    pub fn quiet_edited(&mut self) {
        self.edit = self.edit.max(Some(SongEdit::Quiet));
    }

    /// Reports an edit that re-syncs whatever it changed ([`SongEdit::Resync`]).
    pub fn resync_edited(&mut self) {
        self.edit = self.edit.max(Some(SongEdit::Resync));
    }

    /// Reports a preset pick's edit of [`Self::song`]: a new engine, and no undo step. A final edit in the same frame
    /// takes it in.
    pub fn pick_loaded(&mut self) {
        self.edit = self.edit.max(Some(SongEdit::Untracked));
    }

    /// Reports that [`Self::song`] was replaced by a whole new song. After the frame the app resets the
    /// navigation, every slot's state but the clipboard, the playlist's clip selection and the timeline scroll (they
    /// belong to the old song), selects the first channel after the master, stops the transport and rebuilds the audio.
    pub fn song_replaced(&mut self) {
        self.edit = Some(SongEdit::Replaced);
    }

    /// Reports a channel param's new value in [`Self::song`] from a control bound to it (a knob, fader or toggle), which
    /// the slot also sent as `SetParam`, or a rack card's new settings or size: it sets the song-changed flag at once,
    /// with no undo step and no engine rebuild ([`SongEdit::ChannelParam`]). Call it on each frame that changes one. A
    /// stronger edit in the same frame keeps it out of its undo step ([`crate::history::History::edited`]).
    pub fn channel_param_edited(&mut self) {
        self.files.song_changed();
        self.edit = self.edit.max(Some(SongEdit::ChannelParam));
    }

    /// Reports an edit of [`Self::song`] that the audio already has or never needs (a chord set, the key lock). Call it
    /// on each frame that changes it.
    ///
    /// It only makes an undo step, with no engine rebuild. The step stays open while a pointer is down, so a drag's
    /// edits make one step; the first frame with no pointer down closes it: a drag's release, a tap's own frame, at
    /// once for a key or the wheel. A stronger edit in the same frame takes it in (a final edit makes it that step).
    /// It sets the song-changed flag at once, which that stronger edit may not.
    pub fn param_edited(&mut self) {
        self.files.song_changed();
        self.edit = self.edit.max(Some(SongEdit::Param));
    }

    /// Reports an edit of rack state in [`Self::song`] that makes no undo step and needs no new engine
    /// ([`SongEdit::RackState`]): a drum strip's control, whose value the audio already has. A stronger edit in the
    /// same frame takes it in.
    pub fn rack_state_edited(&mut self) {
        self.edit = self.edit.max(Some(SongEdit::RackState));
    }

    /// Reports an edit of rack state in [`Self::song`] that the audio reads only at build ([`SongEdit::RackRebuild`]): a
    /// new engine, and no undo step. A stronger edit in the same frame takes it in.
    pub fn rack_state_rebuilt(&mut self) {
        self.edit = self.edit.max(Some(SongEdit::RackRebuild));
    }

    /// A tap on a ruler: the playhead goes to song tick `tick`, or, while the transport plays, it
    /// stops instead. The app applies it after the frame; every editor's playhead is the transport's.
    pub fn set_playhead(&mut self, tick: u64) {
        self.playhead = Some(tick);
    }

    /// A tap on an editor's playhead knob: the transport plays. The app applies it after the view.
    pub fn play(&mut self) {
        self.playing = true;
    }
}

/// What every view and overlay keeps between frames.
#[derive(Default)]
pub struct Slots {
    pub piano_roll: piano_roll::State,
    pub drum_editor: drum_editor::State,
    pub mixer: mixer::State,
    pub main_menu: main_menu::State,
    pub settings_tab: settings_tab::State,
    pub keyboard_settings: keyboard_settings::State,
    pub ctrl_menu: ctrl_menu::State,
    pub rack: rack::State,
    pub modules_browser: modules_browser::State,
    pub tempo_editor: tempo_editor::State,
    pub preset_picker: preset_picker::State,
    pub soundfont_picker: soundfont_picker::State,
    /// The app clipboard: the modules a rack card's `Copy` or the rack's `Copy Rack` put there, top to bottom, for the
    /// Modules browser's `Paste`; empty until the first copy. Not saved; the app keeps it when another song opens.
    pub clipboard: Vec<ModuleInstance>,
    pub sample_picker: sample_picker::State,
    pub automation_editor: automation_editor::State,
    pub audio_editor: audio_editor::State,
    /// The song's Save dialog and the render options.
    pub export: save_dialog::State,
    /// The fine tune dialog.
    pub fine_tune: fine_tune::State,
    /// The MIDI import dialog's file and choices.
    pub midi_import: midi_import::State,
    /// The performance monitor's data, which the PROJECT page shows.
    pub performance: Performance,
    /// The `Sound credits` screen's lines, made when it first shows.
    pub credits: credits::State,
    /// The What's New dialog's scroll.
    pub whats_new: whats_new::State,
    /// The MIDI CC picker's panel and pick.
    pub cc_picker: cc_picker::State,
}

impl Slots {
    /// Play start stopped the preview player: an open picker's sample strip stops too.
    pub fn stop_samples(&mut self) {
        self.sample_picker.stop_sample();
        self.preset_picker.stop_sample();
    }

    /// Points the editor of `key`'s lane at that clip, if it exists: the piano roll keeps its snap and zoom, the drum
    /// editor its views (a switch of channel resets the rest, as a visit does); the automation and audio editors hold
    /// it for the channel.
    pub fn open(&mut self, key: ClipKey, song: &Song) {
        let Some(lane) = (song.channels.get(key.channel)).and_then(|c| c.lanes.get(key.lane))
        else {
            return;
        };
        if !lane.clips.iter().any(|c| c.id == key.id) {
            return;
        }
        match lane.kind {
            LaneKind::Notes => (self.piano_roll.rolls.entry(key.channel))
                .or_insert_with(|| PianoRoll::new(key.channel, key.lane, Some(key.id)))
                .open(key.lane, key.id),
            LaneKind::Drums => {
                drum_editor::open(&mut self.drum_editor, key.channel, key.lane, key.id)
            }
            LaneKind::Automation => {
                (self.automation_editor.clips).insert(key.channel, (key.lane, key.id));
            }
            LaneKind::Audio => {
                (self.audio_editor.clips).insert(key.channel, (key.lane, key.id));
            }
        }
    }

    /// Points the bottom editor at `key`, the current track's current clip ([`Self::open`]), and gives its view: the
    /// editor of the clip's lane. `None` once the clip is gone.
    pub fn follow(&mut self, key: ClipKey, song: &Song) -> Option<View> {
        key.clip(song)?;
        self.open(key, song);
        editor(song, key.channel, key.lane)
    }

    /// The clip `channel`'s piano roll edits (its lane and id), when it holds one: where a Slicer's AUTO writes first
    /// ([`crate::audio_engine::slicer::write_auto_clip`]).
    fn roll_clip(&self, channel: usize) -> Option<(usize, u32)> {
        let roll = self.piano_roll.rolls.get(&channel)?;
        Some((roll.lane, roll.clip_id?))
    }

    /// Draws `view` in the editor area; the playlist is drawn by the playlist screen instead.
    pub fn view(&mut self, view: View, ui: &mut egui::Ui, app: &mut AppCtx) -> Option<Nav> {
        match view {
            View::Playlist => None,
            View::PianoRoll(channel) => piano_roll::show(&mut self.piano_roll, ui, app, channel),
            View::DrumEditor(channel) => drum_editor::show(&mut self.drum_editor, ui, app, channel),
            View::Mixer => mixer::show(&mut self.mixer, ui, app),
            View::AutomationEditor(channel) => {
                automation_editor::show(&mut self.automation_editor, ui, app, channel)
            }
            View::AudioEditor(channel) => {
                audio_editor::show(&mut self.audio_editor, ui, app, channel)
            }
        }
    }

    /// Draws `overlays` bottom to top, over the view and the transport, and `corner` (the playlist screen's corner
    /// buttons) over the racks and under every other overlay; with no rack open, a rack still sliding out goes under
    /// them all ([`rack::show_closing`]). `escape`: the Escape press [`take_escape`] held back, handed to the topmost
    /// overlay only. Returns each overlay's request with its index, for [`Navigation::apply_from`].
    pub fn overlays(
        &mut self,
        overlays: &mut [Overlay],
        ui: &mut egui::Ui,
        app: &mut AppCtx,
        escape: bool,
        corner: impl FnOnce(&mut egui::Ui),
    ) -> Vec<(usize, Nav)> {
        // Every new DW Sampler loads Close Grand, landing whatever is open.
        let tag = crate::app::automation::DELETE_AUTOMATION_TAG;
        let asking =
            (overlays.iter()).any(|o| matches!(o, Overlay::Message { tag: t, .. } if *t == tag));
        let (state, rack) = (&mut self.preset_picker, &mut self.rack);
        preset_picker::land_close_grands(state, rack, ui.ctx(), app, asking);
        let top = overlays.len().saturating_sub(1);
        let above = (overlays.iter())
            .rposition(|o| matches!(o, Overlay::Rack(_)))
            .map_or(0, |i| i + 1);
        if above == 0 {
            rack::show_closing(&mut self.rack, ui, app, &mut self.clipboard);
        }
        let mut corner = Some(corner);
        let mut navs = Vec::new();
        for i in 0..overlays.len() {
            app.covered = covered(&overlays[i + 1..], &app.regions, ui);
            let overlay = &mut overlays[i];
            if i == above
                && let Some(corner) = corner.take()
            {
                corner(ui);
            }
            if i == top && escape {
                crate::ui::hand_close(ui.ctx());
            }
            navs.extend(self.overlay(overlay, ui, app, i == top).map(|nav| (i, nav)));
        }
        if let Some(corner) = corner {
            corner(ui);
        }
        navs
    }

    /// `top`: `overlay` is the topmost one.
    fn overlay(
        &mut self,
        overlay: &mut Overlay,
        ui: &mut egui::Ui,
        app: &mut AppCtx,
        top: bool,
    ) -> Option<Nav> {
        match overlay {
            Overlay::MainMenu(tab) => main_menu::show(
                &mut self.main_menu,
                &mut self.settings_tab,
                &mut self.performance,
                ui,
                app,
                tab,
            ),
            // Modal in its own layers: hidden while a message shows over it.
            Overlay::SaveDialog { name } => {
                top.then(|| save_dialog::show(&mut self.export, ui, app, name))?
            }
            Overlay::KeyboardSettings => {
                keyboard_settings::show(&mut self.keyboard_settings, ui, app)
            }
            Overlay::CtrlMenu => ctrl_menu::show(&mut self.ctrl_menu, ui, app),
            // A focus container, for `showRack`.
            Overlay::Rack(channel) => {
                let held = self.roll_clip(*channel);
                let (id, rect) = (egui::Id::new(crate::ui::focus::RACK), app.regions.rack);
                crate::ui::focus::container(ui, id, rect, |ui| {
                    rack::show(&mut self.rack, ui, app, *channel, &mut self.clipboard, held)
                })
            }
            Overlay::ModulesBrowser { channel, at } => modules_browser::show(
                &mut self.modules_browser,
                &mut self.preset_picker,
                ui,
                app,
                *channel,
                *at,
                &self.clipboard,
            ),
            Overlay::Message {
                tag,
                lines,
                confirm,
                extra,
            } => message::show(ui, app, tag, lines, confirm.as_ref(), extra.as_deref()),
            Overlay::TempoEditor => tempo_editor::show(&mut self.tempo_editor, ui, app),
            Overlay::ChannelSettings { channel, name } => {
                channel_settings::show(ui, app, *channel, name)
            }
            // Modal like the Save dialog.
            Overlay::PresetDialog { name, kind } => {
                top.then(|| save_dialog::show_preset(ui, app, name, *kind))?
            }
            Overlay::PresetPicker { channel, target } => {
                let held = self.roll_clip(*channel);
                let (state, rack) = (&mut self.preset_picker, &mut self.rack);
                preset_picker::show(state, rack, ui, app, *channel, *target, held)
            }
            Overlay::SoundFontPicker { channel, module } => {
                let state = &mut self.soundfont_picker;
                soundfont_picker::show(state, &mut self.rack, ui, app, *channel, *module)
            }
            Overlay::SamplePicker { channel, pad } => {
                let (state, drums) = (&mut self.sample_picker, &mut self.drum_editor);
                sample_picker::show(state, drums, ui, app, *channel, pad)
            }
            Overlay::TrackSamplePicker {
                channel,
                lane,
                tick,
            } => sample_picker::show_track(
                &mut self.sample_picker,
                ui,
                app,
                (*channel, *lane, *tick),
            ),
            Overlay::ClipSamplePicker(key) => {
                sample_picker::show_clip(&mut self.sample_picker, ui, app, *key)
            }
            // Modal like the Save dialog.
            Overlay::SampleName { name } => {
                top.then(|| save_dialog::show_sample(ui, app, name, &mut self.audio_editor))?
            }
            Overlay::Busy { title } => {
                busy(ui.ctx(), &app.cx, title);
                None
            }
            Overlay::ModuleSamplePicker { channel, module } => {
                let held = self.roll_clip(*channel);
                sample_picker::show_module(
                    &mut self.sample_picker,
                    ui,
                    app,
                    *channel,
                    *module,
                    held,
                )
            }
            // Modal like the Save dialog: hidden, and not learning, while a message shows over it.
            Overlay::LearnCc => top.then(|| learn_cc(ui, app))?,
            Overlay::FineTune { channel, target } => {
                fine_tune::show(&mut self.fine_tune, ui, app, *channel, *target)
            }
            Overlay::MidiImport => midi_import::show(&mut self.midi_import, ui, app),
            // Modal like the Save dialog: hidden while a message shows over it, which draws in the same layer.
            Overlay::Credits => top.then(|| credits::show(&mut self.credits, ui, app))?,
            // Modal like the Save dialog: hidden while a message shows over it.
            Overlay::WhatsNew => top.then(|| whats_new::show(&mut self.whats_new, ui, app))?,
            Overlay::CcPicker(channel) => cc_picker::show(&mut self.cc_picker, ui, app, *channel),
        }
    }
}

/// A loading screen: `title` centred on the background over the whole screen, which takes every click. The export's
/// `Overlay::Busy` and the audio editor's `Processing...` show it.
pub fn busy(ctx: &egui::Context, cx: &UiCtx, title: &str) {
    busy_screen(ctx, cx, title, false);
}

/// [`busy`] with an abort button, the close X in a 1.5 U cell 0.25 U in from the top-right corner: `text` on a
/// `background` disc that only shows under the mouse. Returns whether it was clicked.
pub fn busy_abort(ctx: &egui::Context, cx: &UiCtx, title: &str) -> bool {
    busy_screen(ctx, cx, title, true)
}

fn busy_screen(ctx: &egui::Context, cx: &UiCtx, title: &str, abort: bool) -> bool {
    let screen = ctx.content_rect();
    let mut aborted = false;
    let c = &cx.theme.colors;
    egui::Area::new(egui::Id::new("busy"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .fade_in(false)
        .sense(egui::Sense::click_and_drag())
        .show(ctx, |ui| {
            ui.allocate_exact_size(screen.size(), egui::Sense::hover());
            // The layer's own painter: the area's first frame (its sizing pass) paints nothing through its `Ui`.
            let painter = ctx.layer_painter(ui.layer_id());
            // Past the screen's edges, which the feathering would leave half covered.
            painter.rect_filled(screen.expand(1.0), 0.0, c.background.to_color32());
            let font = cx.m.font_id(crate::metrics::FontSize::Pt12);
            let ink = c.text.to_color32();
            // Each line centred (an export's stem shows two), the block centred on the screen.
            let mut job = egui::text::LayoutJob::simple(title.to_owned(), font, ink, f32::INFINITY);
            job.halign = egui::Align::Center;
            let galley = painter.layout_job(job);
            let at = screen.center() - egui::vec2(0.0, galley.size().y / 2.0);
            painter.galley(at, galley, ink);
            if abort {
                let (m, inset) = (cx.m, cx.m.u(0.25));
                let cell = egui::Rect::from_min_size(
                    egui::pos2(screen.right() - inset - m.u(1.5), screen.top() + inset),
                    egui::Vec2::splat(m.u(1.5)),
                );
                let id = egui::Id::new("busy abort");
                let bg = c.background.to_color32();
                let x =
                    crate::ui::buttons::filled_circle_button(ui, id, cell.center(), m.u(1.2), bg);
                let close = crate::icons::IconId::Close;
                crate::icons::draw_icon(&painter, cell, close, ink, 0.7, m);
                aborted = x.clicked();
            }
        });
    aborted
}

/// `Link to MIDI CC`'s prompt in the popup style, centred on the screen: [`LEARN_CC_PROMPT`], then `Cancel`. `Cancel`,
/// Escape or a click outside closes it; the CC the app waits for closes it too.
fn learn_cc(ui: &egui::Ui, app: &AppCtx) -> Option<Nav> {
    let items = [
        PopupItem::header(0, 0, LEARN_CC_PROMPT),
        PopupItem::new((), 0, 1, "Cancel"),
    ];
    let (screen, id) = (app.regions.screen, egui::Id::new("learn cc"));
    let result = popup::show(ui.ctx(), id, screen.center(), screen, &items, &app.cx);
    result.close.then_some(Nav::Close)
}

/// The pointer is over one of `overlays`, which keeps the wheel, zoom and pinch from what lies under them. Most draw in
/// the views' layer, where egui lets them through: the rack covers its panel, the keyboard settings nothing that scrolls
/// (the strip under the transport bar), and the rest the screen (the main menu, the popups and the tempo dialog, which
/// is modal over the whole screen though its band is 3 U).
pub fn covered(overlays: &[Overlay], regions: &Regions, ui: &egui::Ui) -> bool {
    let Some(at) = ui.input(|i| i.pointer.latest_pos()) else {
        return false;
    };
    overlays.iter().any(|o| match o {
        Overlay::Rack(_) => regions.rack.contains(at),
        Overlay::KeyboardSettings => false,
        _ => true,
    })
}

/// Takes this frame's `close` press (Escape by default) out of the input, so that the lower overlays never see it. Call
/// it after the view, whose open popups take it first. Returns whether there was one, for [`Slots::overlays`], which
/// hands it to the topmost overlay ([`crate::ui::hand_close`]). An auto-repeat is taken too, and is no press.
pub fn take_escape(ui: &egui::Ui) -> bool {
    crate::keymap::pressed(ui.ctx(), "close")
}

/// The clip a channel's `Edit` opens, which names none: its main lane's (lane 0) current clip (`DESc`@24), else its
/// first. `None` for a lane with no clip, and for an automation main lane (the master's and the sends', whose clips
/// only their own `Edit` opens).
pub fn main_clip(song: &Song, channel: usize) -> Option<ClipKey> {
    let lane =
        (song.channels.get(channel)?.lanes.first()).filter(|l| l.kind != LaneKind::Automation)?;
    let has = |id: &u32| lane.clips.iter().any(|c| c.id == *id);
    let id = (Some(lane.file.current_clip_id).filter(has)).or(lane.clips.first().map(|c| c.id))?;
    Some(ClipKey {
        channel,
        lane: 0,
        id,
    })
}

/// The editor of `channel`'s lane `lane`, if it has one.
fn editor(song: &Song, channel: usize, lane: usize) -> Option<View> {
    match song.channels.get(channel)?.lanes.get(lane)?.kind {
        LaneKind::Notes => Some(View::PianoRoll(channel)),
        LaneKind::Drums => Some(View::DrumEditor(channel)),
        LaneKind::Automation => Some(View::AutomationEditor(channel)),
        LaneKind::Audio => Some(View::AudioEditor(channel)),
    }
}

/// The clip `held` names on `channel` (its lane and clip id), while that lane is of `kind` and has it: the current clip
/// of the automation and audio editors.
fn held_clip(
    song: &Song,
    channel: usize,
    held: Option<&(usize, u32)>,
    kind: LaneKind,
) -> Option<ClipKey> {
    let &(lane, id) = held?;
    let key = ClipKey { channel, lane, id };
    let l = song.channels.get(channel)?.lanes.get(lane)?;
    (l.kind == kind && key.clip(song).is_some()).then_some(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{ChannelKind, SynthKind};

    use Overlay::*;

    /// Master (0), Drums (1), MiniSynth (2), Audio Clip (3), Effect Track (4); a clip on each instrument's main lane.
    fn song() -> Song {
        let mut song = Song::default();
        for kind in [
            ChannelKind::Drums,
            ChannelKind::Synth(SynthKind::MiniSynth),
            ChannelKind::AudioClip,
            ChannelKind::EffectTrack,
        ] {
            song.add_channel(kind);
        }
        for channel in 1..4 {
            song.channels[channel].lanes[0].add_clip(0, 4.0);
        }
        song
    }

    fn with(view: View, overlays: &[Overlay]) -> Navigation {
        Navigation {
            view,
            overlays: overlays.to_vec(),
            answer: None,
        }
    }

    fn after(mut nav: Navigation, action: Nav) -> (View, Vec<Overlay>) {
        nav.apply(action, &song());
        (nav.view, nav.overlays)
    }

    #[test]
    fn edit_shows_the_editor_of_the_channels_main_lane_and_closes_the_overlays() {
        let edit = |channel| after(with(View::Mixer, &[Rack(1)]), Nav::Edit(channel));
        assert_eq!(edit(1), (View::DrumEditor(1), vec![]));
        assert_eq!(edit(2), (View::PianoRoll(2), vec![]));
        assert_eq!(edit(3), (View::AudioEditor(3), vec![]));
        // The master, a send (their main lanes are automation, which only a clip's `Edit` opens) and a missing channel
        // have no editor; nor has a main lane with no clip.
        for channel in [0, 4, 9] {
            assert_eq!(edit(channel), (View::Mixer, vec![Rack(1)]), "{channel}");
        }
        let mut nav = with(View::Mixer, &[Rack(1)]);
        let mut empty = song();
        empty.channels[2].lanes[0].clips.clear();
        nav.apply(Nav::Edit(2), &empty);
        assert_eq!((nav.view, nav.overlays), (View::Mixer, vec![Rack(1)]));
        // Editing the channel already in its editor stays there.
        let nav = with(View::PianoRoll(2), &[]);
        assert_eq!(after(nav, Nav::Edit(2)).0, View::PianoRoll(2));
        // A clip's: the editor of its lane (the app points it at the clip); a lane that is gone has none.
        let clip = |lane| {
            let key = ClipKey {
                channel: 1,
                lane,
                id: 1,
            };
            after(with(View::Mixer, &[]), Nav::EditClip(key)).0
        };
        assert_eq!(clip(0), View::DrumEditor(1));
        assert_eq!(clip(1), View::Mixer);
    }

    #[test]
    fn edit_opens_the_main_lanes_current_clip_else_its_first() {
        let mut song = song();
        let lane = &mut song.channels[2].lanes[0];
        lane.add_clip(512, 4.0);
        lane.add_clip(1024, 4.0);
        let key = |id| ClipKey {
            channel: 2,
            lane: 0,
            id,
        };
        assert_eq!(main_clip(&song, 2), Some(key(1)));
        song.channels[2].lanes[0].file.current_clip_id = 3;
        assert_eq!(main_clip(&song, 2), Some(key(3)));
        // One that is gone plays no part.
        song.channels[2].lanes[0].file.current_clip_id = 9;
        assert_eq!(main_clip(&song, 2), Some(key(1)));
        // An automation main lane (the master's) opens nothing, even with a clip.
        song.channels[0].lanes[0].add_clip(0, 4.0);
        assert_eq!(main_clip(&song, 0), None);
    }

    #[test]
    fn a_drum_lane_of_either_grid_opens_the_drum_editor() {
        // The drum editor follows the current lane, flag 0 or 1 (its mode is `drum_editor::State::mode`).
        let mut song = song();
        let drums = &mut song.channels[1];
        let notes = drums.add_lane(LaneKind::Drums);
        drums.lanes[notes].grid = crate::song::DrumGrid::Notes;
        assert_eq!(editor(&song, 1, 0), Some(View::DrumEditor(1)));
        assert_eq!(editor(&song, 1, notes), Some(View::DrumEditor(1)));
    }

    #[test]
    fn an_automation_or_audio_clips_edit_opens_its_editor_and_an_audio_record_channel_the_audio_one()
     {
        let mut song = song();
        song.add_channel(ChannelKind::AudioRecord);
        let (synth, master) = (&mut song.channels[2], 0);
        let level = crate::song::AutomationTarget {
            instance_id: 0,
            param: 0,
        };
        synth.add_automation_lane(level);
        let clip = |channel, lane| {
            let mut nav = with(View::Playlist, &[KeyboardSettings]);
            let key = ClipKey {
                channel,
                lane,
                id: 1,
            };
            nav.apply(Nav::EditClip(key), &song);
            (nav.view, nav.overlays)
        };
        assert_eq!(clip(2, 1), (View::AutomationEditor(2), vec![]));
        assert_eq!(clip(master, 0), (View::AutomationEditor(0), vec![]));
        assert_eq!(clip(3, 0), (View::AudioEditor(3), vec![]));
        let mut nav = with(View::Playlist, &[]);
        nav.apply(Nav::Edit(5), &song);
        assert_eq!(nav.view, View::Playlist, "no clip to edit");
        song.channels[5].lanes[0].add_clip(0, 4.0);
        nav.apply(Nav::Edit(5), &song);
        assert_eq!(nav.view, View::AudioEditor(5));
    }

    #[test]
    fn show_switches_the_view_closes_the_overlays_and_the_same_view_goes_back_to_the_playlist() {
        let nav = with(View::Playlist, &[MainMenu(MenuTab::Songs), CtrlMenu]);
        assert_eq!(after(nav, Nav::Show(View::Mixer)), (View::Mixer, vec![]));
        let nav = with(View::Mixer, &[KeyboardSettings]);
        assert_eq!(after(nav, Nav::Show(View::Mixer)), (View::Playlist, vec![]));
        // The editors' close X.
        let nav = with(View::DrumEditor(1), &[]);
        assert_eq!(after(nav, Nav::Show(View::Playlist)).0, View::Playlist);
        let nav = with(View::Playlist, &[]);
        assert_eq!(after(nav, Nav::Show(View::Playlist)).0, View::Playlist);
    }

    #[test]
    fn open_stacks_one_overlay_per_kind_and_close_pops_the_top() {
        let browser = ModulesBrowser {
            channel: 1,
            at: InsertAt::Above(2),
        };
        let nav = with(View::Mixer, &[Rack(1)]);
        assert_eq!(
            after(nav, Nav::Open(browser.clone())),
            (View::Mixer, vec![Rack(1), browser.clone()])
        );
        // Another channel's rack replaces the open one, and moves to the top.
        let nav = with(View::Playlist, &[Rack(1), KeyboardSettings]);
        assert_eq!(
            after(nav, Nav::Open(Rack(2))).1,
            [KeyboardSettings, Rack(2)]
        );
        let nav = with(View::Playlist, &[Rack(1), browser]);
        assert_eq!(after(nav, Nav::Close).1, [Rack(1)]);
        assert_eq!(after(Navigation::default(), Nav::Close).1, []);
    }

    #[test]
    fn toggle_closes_the_top_overlay_of_its_kind_or_opens_it() {
        let menu = |tab| MainMenu(tab);
        // The logo closes the menu on whichever tab it shows.
        let nav = with(View::Playlist, &[menu(MenuTab::Settings)]);
        assert_eq!(after(nav, Nav::Toggle(menu(MenuTab::Songs))).1, []);
        let nav = with(View::Playlist, &[]);
        assert_eq!(
            after(nav, Nav::Toggle(menu(MenuTab::Songs))).1,
            [menu(MenuTab::Songs)]
        );
        // Not on top: brought up rather than closed.
        let nav = with(View::Playlist, &[CtrlMenu, Rack(1)]);
        assert_eq!(after(nav, Nav::Toggle(CtrlMenu)).1, [Rack(1), CtrlMenu]);
        let nav = with(View::Playlist, &[Rack(1), CtrlMenu]);
        assert_eq!(after(nav, Nav::Toggle(CtrlMenu)).1, [Rack(1)]);
    }

    #[test]
    fn an_overlays_close_closes_that_overlay_wherever_it_sits() {
        let mut nav = with(View::Playlist, &[Rack(1), KeyboardSettings, CtrlMenu]);
        nav.apply_from(1, Nav::Close, &song());
        assert_eq!(nav.overlays, [Rack(1), CtrlMenu]);
        nav.apply_from(0, Nav::Close, &song());
        assert_eq!(nav.overlays, [CtrlMenu]);
        // Anything else applies as usual; a stale index closes nothing.
        nav.apply_from(0, Nav::Open(Rack(2)), &song());
        assert_eq!(nav.overlays, [CtrlMenu, Rack(2)]);
        nav.apply_from(5, Nav::Close, &song());
        assert_eq!(nav.overlays, [CtrlMenu, Rack(2)]);
    }

    #[test]
    fn a_swap_closes_the_asking_overlay_and_opens_the_other_on_top() {
        let at = InsertAt::End;
        let browser = ModulesBrowser { channel: 1, at };
        let target = crate::ui::rack::PresetTarget::ModulesRack(at);
        let picker = PresetPicker { channel: 1, target };
        let mut nav = with(View::Playlist, &[Rack(1), browser, CtrlMenu]);
        nav.apply_from(1, Nav::Swap(picker.clone()), &song());
        assert_eq!(nav.overlays, [Rack(1), CtrlMenu, picker]);
        // From a view: the topmost goes.
        nav.apply(Nav::Swap(KeyboardSettings), &song());
        assert_eq!(nav.overlays, [Rack(1), CtrlMenu, KeyboardSettings]);
    }

    fn ask(tag: &'static str) -> Nav {
        Nav::Open(Message {
            tag,
            lines: vec!["Replace it?".into()],
            confirm: Some(("Yes".into(), "Cancel".into())),
            extra: None,
        })
    }

    #[test]
    fn an_answer_closes_the_message_and_goes_to_its_asker_only() {
        let mut nav = with(View::Playlist, &[Rack(1)]);
        nav.apply(ask("overwrite"), &song());
        assert_eq!(nav.take_answer("overwrite"), None, "still open");
        nav.apply(Nav::Answer("overwrite", true), &song());
        assert_eq!(nav.overlays, [Rack(1)]);
        assert_eq!(nav.take_answer("newer"), Some(false), "never asked");
        assert_eq!(nav.take_answer("overwrite"), Some(true));
    }

    #[test]
    fn a_message_with_an_extra_choice_tells_its_asker_which_choice_was_picked() {
        let mut nav = with(View::Playlist, &[]);
        for (pick, answer) in [
            (Nav::Extra("unsaved"), Answer::Extra),
            (Nav::Answer("unsaved", true), Answer::Yes),
            (Nav::Answer("unsaved", false), Answer::No),
        ] {
            nav.apply(ask("unsaved"), &song());
            assert_eq!(nav.take_choice("unsaved"), None, "still open");
            nav.apply(pick, &song());
            assert_eq!(nav.overlays, []);
            assert_eq!(nav.take_choice("unsaved"), Some(answer));
        }
        assert_eq!(nav.take_choice("unsaved"), Some(Answer::No), "never asked");
        // A yes / no asker reads the extra choice as no.
        nav.apply(ask("unsaved"), &song());
        nav.apply(Nav::Extra("unsaved"), &song());
        assert_eq!(nav.answer(), Some(("unsaved", false)));
        assert_eq!(nav.take_answer("unsaved"), Some(false));
    }

    #[test]
    fn a_second_message_replaces_the_first_whose_asker_reads_no() {
        let mut nav = with(View::Playlist, &[]);
        nav.apply(ask("overwrite"), &song());
        nav.apply(ask("newer"), &song());
        assert_eq!(nav.take_answer("overwrite"), Some(false));
        assert_eq!(nav.take_answer("newer"), None);
        nav.apply(Nav::Answer("newer", true), &song());
        assert_eq!(nav.take_answer("newer"), Some(true));
    }

    #[test]
    fn a_stale_answer_never_reaches_a_new_asker() {
        let mut nav = with(View::Playlist, &[]);
        nav.apply(ask("overwrite"), &song());
        nav.apply(Nav::Answer("overwrite", true), &song());
        // Nobody took it; the same question is asked again.
        nav.apply(ask("overwrite"), &song());
        assert_eq!(nav.take_answer("overwrite"), None, "answered before shown");
    }

    #[test]
    fn a_confirm_dropped_by_a_view_switch_reads_as_no() {
        for switch in [Nav::Show(View::Mixer), Nav::Edit(2)] {
            let mut nav = with(View::Playlist, &[]);
            nav.apply(ask("overwrite"), &song());
            nav.apply(switch.clone(), &song());
            assert_eq!(nav.take_answer("overwrite"), Some(false), "{switch:?}");
        }
    }

    #[test]
    fn the_strongest_edit_of_a_frame_wins() {
        let theme = crate::theme::theme_by_id(0);
        let m = crate::metrics::Metrics::from_pixels_per_point(1.0, 100);
        let (mut song, mut audio, mut selected) = (song(), AudioLink::default(), 1);
        let mut files =
            ProjectFiles::with_provider(Box::new(crate::project_files::mem::MemFiles::default()));
        let mut app = AppCtx {
            cx: UiCtx {
                theme: &theme,
                m: &m,
            },
            regions: Regions::new(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1170.0, 540.0)),
                &m,
            ),
            song: &mut song,
            audio: &mut audio,
            selected: &mut selected,
            snapshot: None,
            playing: false,
            armed: false,
            previewing: false,
            middle_c_is_c4: false,
            settings: &mut Settings::default(),
            ghost_notes: true,
            files: &mut files,
            edit: None,
            answer: None,
            covered: false,
            playhead: None,
            seek: None,
            midi_devices: &[],
            midi_rescan: false,
        };
        app.rack_state_edited();
        assert!(!app.files.modified());
        // A param edit sets the song-changed flag at once, which a stronger edit in its frame
        // (here a drum strip's rack state) would not.
        app.param_edited();
        assert!(app.files.modified());
        app.edit = None;
        app.param_edited();
        assert_eq!(app.edit, Some(SongEdit::Param));
        app.song_edited(false);
        app.param_edited();
        assert_eq!(app.edit, Some(SongEdit::Ongoing));
        app.song_edited(true);
        app.song_edited(false);
        assert_eq!(app.edit, Some(SongEdit::Final));
        app.song_replaced();
        app.song_edited(true);
        assert_eq!(app.edit, Some(SongEdit::Replaced));
    }

    #[test]
    fn tab_labels_follow_the_strip_order() {
        let labels = MenuTab::ALL.map(MenuTab::label);
        // No SHOP and no SYNC tab.
        assert_eq!(labels, ["SONGS", "PROJECT", "SETTINGS"]);
    }
}

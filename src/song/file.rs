//! Project-file fields that playback and editing do not use, kept so a song saves back as it was read. Defaults are
//! The new-song values; the few placeholders that [`super::Song::stamp_save`] fills at save time say so.

use crate::flm::chunk::ContentPath;

/// A chunk this model does not know, kept per container and written back after the known children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawChunk {
    pub id: [u8; 4],
    pub payload: Vec<u8>,
}

/// Bytes kept only to write a record back exactly as it was read. Not song state: any two compare equal, so a
/// song that holds them equals one that does not. The writer uses them only while they still decode to the model.
#[derive(Clone, Debug, Default)]
pub struct Raw<T>(pub Option<T>);

impl<T> PartialEq for Raw<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// The built-in chord set: the diatonic triads of C major, our own set named "Major". The JSON layout is the
/// `.chords` one (4-space indent, keys in alphabetical order).
pub const DEFAULT_CHORD_JSON: &str = r#"{
    "chords": [
        {
            "baseNote": 0,
            "extensions": [],
            "octave": 3,
            "quality": 0
        },
        {
            "baseNote": 2,
            "extensions": [],
            "octave": 3,
            "quality": 1
        },
        {
            "baseNote": 4,
            "extensions": [],
            "octave": 3,
            "quality": 1
        },
        {
            "baseNote": 5,
            "extensions": [],
            "octave": 3,
            "quality": 0
        },
        {
            "baseNote": 7,
            "extensions": [],
            "octave": 3,
            "quality": 0
        },
        {
            "baseNote": 9,
            "extensions": [],
            "octave": 3,
            "quality": 1
        },
        {
            "baseNote": 11,
            "extensions": [],
            "octave": 3,
            "quality": 3
        },
        {
            "baseNote": 0,
            "extensions": [],
            "octave": 4,
            "quality": 0
        }
    ],
    "rootNote": 0
}"#;

/// `HEAD` fields beyond the song model, plus `META`, `KEYB`, the `TDIV` tail and unknown top-level chunks.
#[derive(Clone, Debug, PartialEq)]
pub struct SongFileState {
    /// @358: the saving app's version. [`super::Song::stamp_save`] sets ours ([`super::FORMAT_VERSION`], or
    /// [`super::PIXEL_ZOOM_VERSION`] while the zooms are per pixel).
    pub version: String,
    /// @0: the selected channel's index.
    pub selected_channel: i32,
    /// @4: the last send-bus number given out (≥ every send's `bus_id`); a new send gets one more.
    pub last_send_bus: i32,
    /// @288: the playhead at save, beats. Ignored on load.
    pub playhead_beats: f64,
    /// @296: the playlist view (scroll x, scroll y, zoom x, zoom y). The zooms are beats and rows a U, or a device
    /// pixel while [`Self::zooms_per_u`] is false.
    pub playlist_view: [f64; 4],
    /// @328: zero until [`super::Song::stamp_save`] gives it a random one (zero loads as random).
    pub guid: [u8; 16],
    /// @352: show sharps (else flats).
    pub show_sharps: bool,
    /// @353: notes that start before a clip window but overlap it play.
    pub play_truncated_notes: bool,
    /// @354: the next channel id (ids are never reused). Always > every channel id, ≤ 4095.
    pub next_channel_id: i32,
    /// Bytes after the version string: first the zoom-units flag ([`Self::zooms_per_u`]), absent in older files.
    pub head_extra: Vec<u8>,
    /// `TDIV` bytes after the two kept in `Song::time_sig`.
    pub time_sig_extra: Vec<u8>,
    pub meta: MetaState,
    pub keyboard: KeyboardState,
    pub unknown_chunks: Vec<RawChunk>,
}

impl Default for SongFileState {
    fn default() -> Self {
        Self {
            version: super::PIXEL_ZOOM_VERSION.into(),
            selected_channel: 0,
            last_send_bus: 0,
            playhead_beats: 0.0,
            playlist_view: [0.0, 0.0, 0.01, 0.02],
            guid: [0; 16],
            show_sharps: true,
            play_truncated_notes: true,
            next_channel_id: 1,
            head_extra: Vec::new(),
            time_sig_extra: Vec::new(),
            meta: MetaState::default(),
            keyboard: KeyboardState::default(),
            unknown_chunks: Vec::new(),
        }
    }
}

impl SongFileState {
    /// The zoom-units flag is 1: the view zooms (the playlist's, each clip's `ZOOM`, each channel's `CHHD`@1040) count
    /// per U. Otherwise (a file from before the flag, a new song) they count per device pixel, until
    /// [`super::Song::zooms_to_units`].
    pub fn zooms_per_u(&self) -> bool {
        self.head_extra.first() == Some(&1)
    }
}

/// `META` (3081 bytes).
#[derive(Clone, Debug, PartialEq)]
pub struct MetaState {
    /// @0, `YYYY-MM-DD HH-MM-SS`. Placeholder `"?"` (the loader's default for an absent META) until
    /// [`super::Song::stamp_save`] sets it.
    pub created: String,
    /// @256: active editing time, seconds (negative disables counting).
    pub editing_seconds: f64,
    /// @264.
    pub author: String,
    /// @520.
    pub display_title: String,
    /// @776, the metadata lock: 1 locks Title / Author / Online link and the time rows forever, and stops time
    /// counting. Written on every save, so it survives Save, Save New, quick save and copies, and templates keep it.
    /// New empty song = 0; demos ship with 1. Set by PROJECT `Lock metadata forever`.
    pub locked: bool,
    /// @777.
    pub info_url: String,
    /// @2825, `YYYY-MM-DD HH:MM:SS`, set by [`super::Song::stamp_save`]. Absent → "Never / demo project".
    pub modified: String,
    /// Bytes after @3081.
    pub extra: Vec<u8>,
}

impl Default for MetaState {
    fn default() -> Self {
        Self {
            created: "?".into(),
            editing_seconds: 0.0,
            author: String::new(),
            display_title: String::new(),
            locked: false,
            info_url: String::new(),
            modified: "Never / demo project".into(),
            extra: Vec::new(),
        }
    }
}

/// The `KEYB` chord-set states.
pub const SET_DEFAULT: u32 = 0;
pub const SET_LOADED: u32 = 1;
pub const SET_EDITED: u32 = 2;

/// `KEYB`: the chord keyboard.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyboardState {
    /// The chord set, `.chords` JSON. Default [`DEFAULT_CHORD_JSON`].
    pub chord_json: String,
    /// Vertical mode: 0 Chords, 1 Strum.
    pub vertical_mode: u32,
    /// The chord set's state: [`SET_DEFAULT`] (the untouched default, `Default: <name>`),
    /// [`SET_LOADED`] (loaded or saved, `<name>`) or [`SET_EDITED`] (`* <name>`).
    pub set_state: u32,
    pub chord_set_name: String,
    pub octave_shift: i32,
    /// [1/127, 1].
    pub velocity: f32,
    /// Strum timing, signed.
    pub timing: f32,
    pub hold: bool,
    /// Bytes after `hold`.
    pub extra: Vec<u8>,
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self {
            chord_json: DEFAULT_CHORD_JSON.into(),
            vertical_mode: 0,
            set_state: SET_DEFAULT,
            chord_set_name: "Major".into(),
            octave_shift: 0,
            velocity: 0.5,
            timing: 0.0,
            hold: false,
            extra: Vec::new(),
        }
    }
}

/// One `SEND` (24 bytes). The channel's `output_bus` is derived from its sends (RHED[12]).
#[derive(Clone, Debug, PartialEq)]
pub struct SendState {
    /// @0: the target channel id (master 0).
    pub target_channel_id: i32,
    /// @4.
    pub enabled: f32,
    /// @8: the level as linear gain, derived from `level` by the app (kept as read).
    pub linear_gain: f32,
    /// @12.
    pub enabled_init: f32,
    /// @16.
    pub level_init: f32,
    /// @20: normalized, 0.8531 = 0 dB.
    pub level: f32,
    /// Bytes after @24.
    pub extra: Vec<u8>,
}

impl Default for SendState {
    fn default() -> Self {
        Self {
            target_channel_id: 0,
            enabled: 1.0,
            linear_gain: 1.0,
            enabled_init: 1.0,
            level_init: 0.8531,
            level: 0.8531,
            extra: Vec::new(),
        }
    }
}

/// `CHHD` fields beyond the channel model, its sends, and the rack/channel tails.
///
/// Derived on write: the playlist row (@1028) from the channel's place in [`super::Song::playlist`] (the master's from
/// [`super::Song::master_below`]); the base name (vstr32 #1)
/// `MASTER` or `<kind label> <id>`, or the one read; the custom name (vstr32 #3) `Channel::name` when it
/// differs from the auto name (the preset name, else the base name), or the one read while it is still the name.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelFileState {
    /// @1040, @1048: per-channel view. @1040 is a zoom, in the song's zoom units ([`SongFileState::zooms_per_u`]).
    pub view: [f64; 2],
    /// @1056: input monitoring.
    pub input_monitoring: bool,
    /// @1060.
    pub record_armed: bool,
    /// @1064: random for a channel the song makes (a clone gets its own); zero on the master of a new song until
    /// [`super::Song::stamp_save`] gives it one (zero loads as random).
    pub guid: [u8; 16],
    /// vstr32 #2: the instrument preset loaded into the channel, else "".
    pub preset_name: String,
    /// The `SEND`s, in file order; every non-master channel has at least one (default: to the master).
    pub sends: Vec<SendState>,
    /// The base and custom names as read (`CHHD` vstr32 #1 and #3), written back for the channel they were read
    /// with; a new or cloned channel has none.
    pub names: Raw<(String, String)>,
    /// The channel's place among the file's racks of its kind (the mixer order), kept so a save writes them back in
    /// that order ([`super::Song::rack_order`]). A clone keeps its source's, so it lands right after it (equal places go
    /// by id); other new channels have none and go after, by id.
    pub rack_order: Raw<usize>,
    /// The raw `CCLK` payload (n × 3 × i32), written only when non-empty.
    pub cc_links: Vec<u8>,
    /// `RHED` bytes after 16.
    pub rack_header_extra: Vec<u8>,
    /// `RPRM` bytes after 24.
    pub params_extra: Vec<u8>,
    pub rack_unknown_chunks: Vec<RawChunk>,
    pub channel_unknown_chunks: Vec<RawChunk>,
    /// `CHHD` bytes after the three names.
    pub header_extra: Vec<u8>,
    /// Honeydew's own `HDMi`: the live MIDI an instrument or drum channel takes.
    pub midi_input: MidiInput,
    /// Honeydew's own `HDMs`: the channels the notes and MIDI CCs it plays also go to, in the order added.
    pub midi_sends: Vec<MidiSend>,
    /// Honeydew's own `HDMd`: the targets of its rack's Modulators, one each. Kept here, outside the rack, so that each
    /// target edit is an undo step (no undo step covers the rack).
    pub modulations: Vec<Modulation>,
}

/// A Modulator's target (Honeydew's own, `honeydew_audio::modulator`): the param its signal moves, any automatable
/// param of any channel. Saved in an `HDMd`, which FL Studio Mobile skips. One whose Modulator or target is gone does
/// nothing (ids are never reused).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modulation {
    /// The Modulator's instance id in the channel's rack.
    pub modulator: i32,
    /// The target's channel id (`CHHD`@1080).
    pub channel: i32,
    pub target: super::AutomationTarget,
}

/// A MIDI send (Honeydew's own): every note the channel's instruments get, past its note effects, and its MIDI CCs and
/// Pitch Bend, go to another channel's note input too. Saved in an `HDMs`, which FL Studio Mobile skips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiSend {
    /// The target channel id (`CHHD`@1080).
    pub target_channel_id: i32,
    pub enabled: bool,
}

/// Which live MIDI a channel takes (Honeydew's own; FL Studio Mobile takes it all on the current channel). `None` is
/// All. Saved as an `HDMi` in the `CHNL`, only when set, which FL Studio Mobile skips as an unknown chunk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MidiInput {
    /// A device by the name the MIDI layer reports (`midi::device::device_name`); while it is missing nothing
    /// matches.
    pub device: Option<String>,
    /// A MIDI channel, 0…15 (shown as 1…16).
    pub channel: Option<u8>,
}

impl MidiInput {
    /// Whether a message from `device` on MIDI channel `channel` gets through.
    pub fn passes(&self, device: &str, channel: u8) -> bool {
        self.device.as_deref().is_none_or(|d| d == device)
            && self.channel.is_none_or(|c| c == channel)
    }
}

impl Default for ChannelFileState {
    fn default() -> Self {
        Self {
            view: [0.0; 2],
            input_monitoring: false,
            record_armed: false,
            guid: [0; 16],
            preset_name: String::new(),
            sends: Vec::new(),
            names: Raw::default(),
            rack_order: Raw::default(),
            cc_links: Vec::new(),
            rack_header_extra: Vec::new(),
            params_extra: Vec::new(),
            rack_unknown_chunks: Vec::new(),
            channel_unknown_chunks: Vec::new(),
            header_extra: Vec::new(),
            midi_input: MidiInput::default(),
            midi_sends: Vec::new(),
            modulations: Vec::new(),
        }
    }
}

/// `RMOd` chunks beyond the module model. `SMPR` is always written with 3 values (older 2-value ones load
/// with the default Wet Pan).
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleFileState {
    /// `HEAD` @4 (written 0).
    pub head_reserved: u32,
    /// `HEAD` @8 (written 1).
    pub head_enabled: u8,
    /// `DESC` @0: the preset category/folder, `/`-separated.
    pub category: String,
    /// `DESC` @256: the preset name.
    pub preset_name: String,
    /// `DESC` @512: the preset path relative to the user root ("" = `u32 0`).
    pub preset_path: String,
    /// `PRST` #1: the preset source path (absolute, or relative to the content root).
    pub source_path: String,
    /// `PRST` #2: `Category/Name`.
    pub provenance: String,
    /// `ADD1` @1, @2 (the loader swaps an inverted range and clamps a negative low).
    pub key_low: i8,
    pub key_high: i8,
    /// The `CSTM` blob, `None` when absent. Only some module types carry one.
    pub custom_data: Option<Vec<u8>>,
    pub unknown_chunks: Vec<RawChunk>,
    /// `HEAD` bytes after 9.
    pub head_extra: Vec<u8>,
    /// `DESC` bytes after the preset path.
    pub desc_extra: Vec<u8>,
    /// `ADD1` bytes after 3.
    pub add1_extra: Vec<u8>,
    /// The DW Sampler's Attack (p1) and Release (p2) init values, which a save writes as its `PATh` floats, when they
    /// are not `params[1]` and `params[2]`: a `PATh` whose program did not load keeps its floats here. `None`: the
    /// params are the init values (a program load or pick sets both, and a control moved while not recording sets its
    /// init value).
    pub program_init: Option<[f32; 2]>,
    /// The DW Sampler's Attack (p1) and Release (p2) values its program set as their default, which a double tap and
    /// `Reset` set. `None`: no program set them, so the registry's. Not in the file: a program load sets it again.
    pub program_reset: Option<[f32; 2]>,
    /// A plugin module's plugin and state ([`super::plugin`]); `None` on every other module.
    pub plugin: Option<Box<super::plugin::PluginRef>>,
}

impl Default for ModuleFileState {
    fn default() -> Self {
        Self {
            head_reserved: 0,
            head_enabled: 1,
            category: "Default".into(),
            preset_name: "Default".into(),
            preset_path: String::new(),
            source_path: String::new(),
            provenance: "Default/Default".into(),
            key_low: 0,
            key_high: 127,
            custom_data: None,
            unknown_chunks: Vec::new(),
            head_extra: Vec::new(),
            desc_extra: Vec::new(),
            add1_extra: Vec::new(),
            program_init: None,
            program_reset: None,
            plugin: None,
        }
    }
}

/// `DESc` fields beyond the lane model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaneFileState {
    /// @1 of a lane that is no drum lane (a drum lane's is `Lane::grid`): kept as read, 0 on a new lane (the audio
    /// template has 1).
    pub flag: u8,
    /// @24: the current clip id (0 = none).
    pub current_clip_id: u32,
    /// @1052: the custom label ("" = the auto name in `Lane::label`).
    pub custom_label: String,
    pub unknown_chunks: Vec<RawChunk>,
    /// `DESc` bytes after the custom label.
    pub desc_extra: Vec<u8>,
}

/// `CLIP` fields beyond the clip model.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipFileState {
    /// `CLHd` @28: audio lanes' source length, beats (0 elsewhere). A visible length ≤ 0 falls back to it.
    pub source_len_beats: f64,
    /// `ZOOM`: the editor view (scroll x, scroll y, zoom x, zoom y), the zooms in the song's zoom units
    /// ([`SongFileState::zooms_per_u`]). Placeholder default: the drums value (audio and automation clips are usually
    /// `0, 127, 1/96, 2`).
    pub zoom: [f64; 4],
    /// `CLSm`: an audio clip's sample (boxed: most clips have none). Its id is given on load.
    pub sample: Option<Box<super::SamplerSlot>>,
    pub unknown_chunks: Vec<RawChunk>,
    /// `CLHd` bytes after 37.
    pub header_extra: Vec<u8>,
    /// The `EVN2` payload as read, kept only when re-encoding the events would not give it back.
    pub raw_events: Raw<Vec<u8>>,
    /// `HDSh` (Honeydew's own): on an automation clip, the shape of each segment that is not the straight line, by the
    /// tick of its point (sorted, one per tick: the last point at a tick starts the segment that has a length).
    pub shapes: Vec<(u32, honeydew_audio::automation::Shape)>,
}

impl Default for ClipFileState {
    fn default() -> Self {
        Self {
            source_len_beats: 0.0,
            zoom: [0.0, 0.0, 1.0 / 192.0, 1.0 / 96.0],
            sample: None,
            unknown_chunks: Vec::new(),
            header_extra: Vec::new(),
            raw_events: Raw::default(),
            shapes: Vec::new(),
        }
    }
}

/// `RSMP` fields beyond the kit model.
#[derive(Clone, Debug, PartialEq)]
pub struct KitFileState {
    /// The last slot id given out (≥ every slot's id); a new slot gets one more ([`super::DrumKit::insert_slot`]).
    pub last_slot_id: u32,
    /// `PADS` (pad → slot map; the identity in every file).
    pub pads: [u8; 22],
    pub unknown_chunks: Vec<RawChunk>,
}

impl Default for KitFileState {
    fn default() -> Self {
        Self {
            last_slot_id: 0,
            pads: std::array::from_fn(|index| index as u8),
            unknown_chunks: Vec::new(),
        }
    }
}

/// Sampler-slot fields beyond the slot model. The `MAIN` pad index (@4) is not stored: a kit slot's is its
/// position in the kit, an audio clip's is −1.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotFileState {
    /// `LINk` @0: unique in the kit, ≥ 1 ([`super::DrumKit::insert_slot`]). Given on load for audio clips.
    pub id: u32,
    /// `MAIN` @1036.
    pub reverse: bool,
    /// `MAIN` @1037, beats.
    pub beat_offset: f64,
    /// `MAIN` @1045 (0.5 = none).
    pub beat_offset_control: f32,
    /// `MAIN` @1050: voices overlap on retrigger.
    pub overlapping_voices: bool,
    /// `MAIN` @1051: the custom pad name, "" = not renamed.
    pub custom_pad_name: String,
    /// `LINk` @4: the legacy output bus (−1 = channel output).
    pub output_bus: i32,
    /// `LINk` @8: the pad's MIDI note, 0xFFFF = none (see `SamplerSlot::midi_note`).
    pub midi_key: u16,
    /// `LINk` @10: the send target channel id (−1 = channel output).
    pub send_target: i32,
    /// `STR1`: mode, stretch ratio, pitch in semitones.
    pub stretch_mode: u8,
    pub stretch_ratio: f64,
    pub pitch_semitones: f64,
    /// `STRC` / `STR1` as read, with the (mode, ratio, semitones) they gave; written back while that is unchanged.
    pub raw_strc: Raw<Vec<u8>>,
    pub raw_str1: Raw<Vec<u8>>,
    pub raw_stretch_state: Raw<(u8, f64, f64)>,
    /// `PTH1` (v1 `PATH`): the sample, an app- or user-root shorthand; `None` = no sample.
    pub path: Option<ContentPath>,
    /// `PRST` #1: the source path. (`ModuleFileState::preset_path` is the module's `DESC` path; this is PRST's.)
    pub preset_path: String,
    /// `PRST` #2: `Category/Name`.
    pub provenance: String,
    pub unknown_chunks: Vec<RawChunk>,
    /// `MAIN` bytes after the custom pad name.
    pub main_extra: Vec<u8>,
    /// `LINk` bytes after 14.
    pub link_extra: Vec<u8>,
}

impl Default for SlotFileState {
    fn default() -> Self {
        Self {
            id: 0,
            reverse: false,
            beat_offset: 0.0,
            beat_offset_control: 0.5,
            overlapping_voices: true,
            custom_pad_name: String::new(),
            output_bus: -1,
            midi_key: u16::MAX,
            send_target: -1,
            stretch_mode: 0,
            stretch_ratio: 1.0,
            pitch_semitones: 0.0,
            raw_strc: Raw::default(),
            raw_str1: Raw::default(),
            raw_stretch_state: Raw::default(),
            path: None,
            preset_path: String::new(),
            provenance: String::new(),
            unknown_chunks: Vec::new(),
            main_extra: Vec::new(),
            link_extra: Vec::new(),
        }
    }
}

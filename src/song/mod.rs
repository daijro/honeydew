//! The song model, shaped after the project file: song header, channels with their rack (modules, `RPRM` channel
//! params) and sequencer lanes (clips, events). Pure data shared by the UI and the audio side: no UI types, no interior
//! mutability, everything `Clone + Debug + PartialEq`.

mod channel;
pub mod chords;
mod drum_kit;
mod file;
mod lane;
pub mod plugin;
pub mod points;
mod rack;
pub mod scales;
mod tempo;

pub use channel::{Channel, ChannelParams};
pub use drum_kit::{DrumKit, SamplerSlot};
pub use file::{
    ChannelFileState, ClipFileState, DEFAULT_CHORD_JSON, KeyboardState, KitFileState,
    LaneFileState, MetaState, MidiSend, Modulation, ModuleFileState, Raw, RawChunk, SET_DEFAULT,
    SET_EDITED, SET_LOADED, SendState, SlotFileState, SongFileState,
};
pub use lane::MAX_PLAYS_A_PIXEL;
pub use lane::{AutomationTarget, Clip, DrumGrid, Event, Lane, LaneKind};
pub use rack::{CommonParams, ModuleInstance, Rack};
pub use tempo::{
    AUTOMATION_TEMPO_RANGE, TEMPO_PARAM, TEMPO_RANGE, TEMPO_TARGET, automated_tempo, tempo_value,
};

/// Ticks per beat of event positions and clip start ticks.
pub const PPQ: u32 = 128;

/// Channel ids run below this: HEAD@354, the next id, must stay under 4096, so the last id given out is 4094.
pub const CHANNEL_ID_LIMIT: i32 = 4095;

/// The format version a save writes: the layout with the zoom-units flag and the view zooms per U
/// ([`SongFileState::zooms_per_u`]).
pub const FORMAT_VERSION: &str = "5.0.1";

/// The version of a song whose view zooms are still per device pixel (the new-song default): the last layout without
/// the zoom-units flag, which a save writes for it instead.
pub const PIXEL_ZOOM_VERSION: &str = "4.10.19";

/// Instrument modules offered in the add-channel popup's `Synth:` column, in menu order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynthKind {
    MiniSynth,
    DwSampler,
    GmSynth,
    TransistorBass,
    SuperSaw,
    Slicer,
    ThreeXOsc,
    SoundFontPlayer,
}

impl SynthKind {
    pub const ALL: [Self; 8] = [
        Self::MiniSynth,
        Self::DwSampler,
        Self::GmSynth,
        Self::TransistorBass,
        Self::SuperSaw,
        Self::Slicer,
        Self::ThreeXOsc,
        Self::SoundFontPlayer,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::MiniSynth => "MiniSynth",
            Self::DwSampler => "DW Sampler",
            Self::GmSynth => "GMSynth",
            Self::TransistorBass => "Transistor Bass",
            Self::SuperSaw => "SuperSaw",
            Self::Slicer => "Slicer",
            Self::ThreeXOsc => "3x Osc",
            Self::SoundFontPlayer => "SoundFont Player",
        }
    }

    /// The instrument's module type id.
    pub fn type_id(self) -> u16 {
        match self {
            Self::MiniSynth => 0,
            Self::DwSampler => 1,
            Self::GmSynth => 2,
            Self::TransistorBass => 19,
            Self::SuperSaw => 23,
            Self::Slicer => 33,
            Self::ThreeXOsc => 39,
            Self::SoundFontPlayer => 52,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    Master,
    Drums,
    Instrument,
    AudioClip,
    /// The send (aux bus) channel, file channel kind 0x080: owns a bus id that other channels can output to.
    EffectTrack,
    AudioRecord,
    Rack,
    Synth(SynthKind),
}

impl ChannelKind {
    /// The kinds of the add-channel popup's `Channel:` column, in menu order.
    pub const ADDABLE: [Self; 6] = [
        Self::Drums,
        Self::Instrument,
        Self::AudioClip,
        Self::EffectTrack,
        Self::AudioRecord,
        Self::Rack,
    ];

    /// The add-channel menu label, which is also the default channel name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Master => "MASTER",
            Self::Drums => "Drums",
            Self::Instrument => "Instrument",
            Self::AudioClip => "Audio Clip",
            Self::EffectTrack => "Effect Track (AUX)",
            Self::AudioRecord => "Audio Record",
            Self::Rack => "Rack",
            Self::Synth(synth) => synth.label(),
        }
    }

    /// The kind word of the base name `<Kind> <channel ID>` (`CHHD`): the label, but `Effect` for a send, which
    /// is also how a new send is named.
    pub fn base_label(self) -> &'static str {
        match self {
            Self::EffectTrack => "Effect",
            _ => self.label(),
        }
    }

    /// The `RACK`/`CHNL` channel kind: 0x000 master, 0x080 send, 0x100 instrument/audio.
    pub fn file_kind(self) -> u32 {
        match self {
            Self::Master => 0x000,
            Self::EffectTrack => 0x080,
            _ => 0x100,
        }
    }

    /// An instrument or drum channel: one live notes play on.
    pub fn plays_notes(self) -> bool {
        matches!(self, Self::Drums | Self::Instrument | Self::Synth(_))
    }
}

/// Hue of the first channel added after the master; the n-th added channel is `fract(HUE_START + n × HUE_STEP)`.
const HUE_START: f64 = 0.6;
const HUE_STEP: f64 = 0.618034;

/// `TDIV`, its two bytes in file order. A file may hold 0 in either; the accessors count 0 as 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSig {
    /// Byte 0, the beat subdivision d: steps per beat. The timeline grid (1/d beat), snapping, the drum step grid
    /// and the drum Swing.
    pub subdivision: u8,
    /// Byte 1, b: beats per bar. Bar lines, the metronome accent and every bar length (one-bar clips, b·2ᵏ loops).
    pub beats_per_bar: u8,
}

impl Default for TimeSig {
    fn default() -> Self {
        Self {
            subdivision: 4,
            beats_per_bar: 4,
        }
    }
}

impl TimeSig {
    /// Beats per bar, at least 1.
    pub fn bar_beats(self) -> u32 {
        u32::from(self.beats_per_bar.max(1))
    }

    /// Steps per beat, at least 1.
    pub fn steps_per_beat(self) -> u32 {
        u32::from(self.subdivision.max(1))
    }

    /// A step's length in ticks, [`PPQ`]/d (fractional unless d divides it).
    pub fn step_ticks(self) -> f64 {
        f64::from(PPQ) / f64::from(self.steps_per_beat())
    }

    /// The tick step `step` starts at: round(step·PPQ/d).
    pub fn step_start(self, step: usize) -> u32 {
        let d = u64::from(self.steps_per_beat());
        u32::try_from((2 * step as u64 * u64::from(PPQ) + d) / (2 * d)).unwrap_or(u32::MAX)
    }

    /// The step tick `pos` lies in: the last one starting at or before it.
    pub fn step_of(self, pos: u32) -> usize {
        // A step start is never a half tick from step·PPQ/d (d < 256), so this inverts `step_start` while steps are
        // at least a tick (d ≤ 128).
        ((2 * u64::from(pos) + 1) * u64::from(self.steps_per_beat()) / (2 * u64::from(PPQ)))
            as usize
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Song {
    /// `HEAD` song title.
    pub title: String,
    pub tempo_bpm: f64,
    /// `TDIV`: the subdivision and the beats per bar.
    pub time_sig: TimeSig,
    /// In playlist order (a file's `CHHD` playlist rows). The master channel is always `channels[0]`. A file keeps
    /// them in kind order (master, sends, the rest) and the reader restores this order from the rows.
    pub channels: Vec<Channel>,
    /// `HEAD` @344: the key-lock root note, 0 = C.
    pub key_root: i32,
    /// `HEAD` @348: the key-lock scale, an id in the stored scale list ([`scales::scale_from_stored`]).
    pub key_scale: i32,
    /// `HEAD` @272 / @280: the song's play loop (the rulers' time range), as song ticks
    /// `(start, end)`, start < end. `None`: no loop (the file's start = end). The file holds beats.
    pub play_loop: Option<(u32, u32)>,
    /// What only the project file needs (`HEAD` view state, `META`, `KEYB`, unknown chunks).
    pub file: SongFileState,
    /// The master's place in the playlist ([`Self::playlist`]): right below the channel with this id; first when `None`
    /// or when no channel has it. A file has it in the master's row (`CHHD` @1028).
    pub master_below: Option<i32>,
}

impl Default for Song {
    fn default() -> Self {
        Self {
            title: String::new(),
            tempo_bpm: 120.0,
            time_sig: TimeSig::default(),
            channels: vec![Channel::master()],
            key_root: 0,
            key_scale: 0,
            play_loop: None,
            file: SongFileState::default(),
            master_below: None,
        }
    }
}

impl Song {
    /// Sets the key-lock scale (stored id `id`). While the chord set is still the untouched default, `KEYB` takes the
    /// new scale's default (`Major` or `Minor`), whoever changes the scale: the keyboard settings, or the piano roll's
    /// scale guess and `Disable`.
    pub fn set_key_scale(&mut self, id: i32) {
        self.key_scale = id;
        let keyboard = &mut self.file.keyboard;
        if keyboard.set_state == SET_DEFAULT {
            let (json, name) = chords::default_keyb(id);
            (keyboard.chord_json, keyboard.chord_set_name) = (json, name.into());
        }
    }

    pub fn master(&self) -> &Channel {
        &self.channels[0]
    }

    pub fn channel_mut(&mut self, index: usize) -> Option<&mut Channel> {
        self.channels.get_mut(index)
    }

    /// The module `instance_id` in the rack of channel `channel`.
    pub fn find_module(&self, channel: usize, instance_id: i32) -> Option<&ModuleInstance> {
        self.channels.get(channel)?.rack.get(instance_id)
    }

    /// Appends a channel of `kind`, see [`Self::insert_channel`].
    pub fn add_channel(&mut self, kind: ChannelKind) {
        self.insert_channel(self.channels.len(), kind);
    }

    /// Inserts a channel at `at` (the playlist's `Insert Channel`) and returns its index. It is named after `kind`,
    /// or `<label> N` for the smallest free `N >= 2` if the name is taken (a send `Effect <channel ID>`, an Audio
    /// Record channel `Record <ID>`), and gets the next channel id. A send gets the next free bus id (≥ 1); a synth
    /// channel gets its instrument module; every new channel outputs to the master (one `SEND`). `None`, and no
    /// channel, once the ids are used up ([`CHANNEL_ID_LIMIT`]).
    pub fn insert_channel(&mut self, at: usize, kind: ChannelKind) -> Option<usize> {
        let id = self.next_id()?;
        let name = match kind {
            ChannelKind::EffectTrack => format!("{} {id}", kind.base_label()),
            // `Record <ID>`: unique, as ids are never reused.
            ChannelKind::AudioRecord => format!("Record {id}"),
            _ => self.free_name(kind.label()),
        };
        // `channels.len()` is the number of channels added so far, plus one, as the master is not counted.
        let n = self.channels.len() as f64;
        let hue = (HUE_START + n * HUE_STEP).fract() as f32;
        let mut channel = Channel::new(name, kind, hue);
        channel.id = id;
        channel.file.guid = random_guid(id);
        channel.file.sends.push(SendState::default());
        if kind == ChannelKind::EffectTrack {
            channel.bus_id = self.next_bus();
        }
        if let ChannelKind::Synth(synth) = kind {
            channel.rack.add(synth.type_id());
        }
        // Below the master, which stays first.
        let at = at.clamp(1.min(self.channels.len()), self.channels.len());
        self.channels.insert(at, channel);
        Some(at)
    }

    /// Copies channel `index` right below it (`Clone Channel`) and returns the copy's index: everything but its id,
    /// its name (the source's, numbered as [`Self::insert_channel`] numbers a taken name), its GUID (a new one) and,
    /// for a send, its bus, which is a new one that nothing outputs to. Not the master, and nothing once the ids are
    /// used up: then `index` is returned.
    pub fn clone_channel(&mut self, index: usize) -> usize {
        if index == 0 || index >= self.channels.len() {
            return index;
        }
        let Some(id) = self.next_id() else {
            return index;
        };
        let mut copy = self.channels[index].clone();
        copy.name = self.free_name(&copy.name);
        copy.file.guid = random_guid(id);
        // Its names are its own: the base name embeds the id.
        copy.file.names = Raw::default();
        copy.id = id;
        if copy.bus_id >= 1 {
            copy.bus_id = self.next_bus();
        }
        // Right below its source, above a master shown there.
        if self.master_below == Some(self.channels[index].id) {
            self.master_below = Some(copy.id);
        }
        self.channels.insert(index + 1, copy);
        index + 1
    }

    /// Removes channel `index` (`Delete Channel`): the channels' `SEND`s to it go to the master (dropped where one to
    /// the master exists), the channels that output to its bus or sent to it output where their `SEND`s now say
    /// ([`output_bus`]), and their Enable Send / Send Level automation lanes for it (params 0x1000 / 0x2000 + its id)
    /// go, and so do the pad sends to it (`LINk` @10, back to the channel output), the sidechains it keyed, the MIDI
    /// sends to it and the Modulator targets on it. The file's selected channel (`HEAD` @0, an index) stays on its channel, or in range when that is the
    /// one deleted. Never the master.
    pub fn delete_channel(&mut self, index: usize) {
        if index == 0 || index >= self.channels.len() {
            return;
        }
        let gone = self.channels.remove(index);
        // The master keeps its place, below the channel that was above the deleted one.
        if self.master_below == Some(gone.id) {
            self.master_below = (index > 1).then(|| self.channels[index - 1].id);
        }
        let selected = &mut self.file.selected_channel;
        if *selected > index as i32 {
            *selected -= 1;
        }
        *selected = (*selected).min(self.channels.len() as i32 - 1);
        let buses = self.send_buses();
        for c in &mut self.channels {
            for module in &mut c.rack.modules {
                module.sidechain.take_if(|&mut id| id == gone.id);
            }
            for slot in c.drum_kit.iter_mut().flat_map(|kit| &mut kit.slots) {
                if slot.file.send_target == gone.id {
                    slot.file.send_target = -1;
                }
            }
            let routed = (gone.bus_id >= 1 && c.output_bus == gone.bus_id)
                || c.file.sends.iter().any(|s| s.target_channel_id == gone.id);
            let send_params = [0x1000, 0x2000].map(|base| base + gone.id);
            c.lanes.retain(|l| {
                l.target
                    .is_none_or(|t| t.instance_id != 0 || !send_params.contains(&t.param))
            });
            c.file.midi_sends.retain(|s| s.target_channel_id != gone.id);
            c.file.modulations.retain(|m| m.channel != gone.id);
            let sends = &mut c.file.sends;
            let to_master = sends.iter().any(|s| s.target_channel_id == 0);
            if to_master {
                sends.retain(|s| s.target_channel_id != gone.id);
            } else if let Some(send) = sends.iter_mut().find(|s| s.target_channel_id == gone.id) {
                send.target_channel_id = 0;
            }
            // Its output follows its SENDs, as a save and load would set it.
            if routed {
                c.output_bus = output_bus(&c.file.sends, &buses);
            }
        }
    }

    /// The channels' indices in rack order: the file's `RACK` order, which is the rack and mixer order. By kind
    /// (master, sends, the rest); within a kind, read channels in the order they were read
    /// ([`ChannelFileState::rack_order`]), then the others by id, as made. A playlist move changes none of it.
    pub fn rack_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.channels.len()).collect();
        order.sort_by_key(|&i| {
            let c = &self.channels[i];
            let read = c.file.rack_order.0.unwrap_or(usize::MAX);
            (c.kind.file_kind(), read, c.id)
        });
        order
    }

    /// The channels' indices in playlist order (a file's `CHHD` @1028 rows): index order, with the master right below
    /// the channel [`Self::master_below`] names.
    pub fn playlist(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (1..self.channels.len()).collect();
        let at = (self.master_below)
            .and_then(|id| order.iter().position(|&c| self.channels[c].id == id))
            .map_or(0, |p| p + 1);
        order.insert(at, 0);
        order
    }

    /// Puts the channels in playlist order `order`, their indices each once (a track header's drag): the others take
    /// that order, and the master, which stays `channels[0]`, its place in it ([`Self::master_below`]). Sends, routing
    /// and automation name channels by id, so nothing else changes but the file's selected channel (`HEAD` @0, an
    /// index), which stays on its channel. False, and no change, for anything but a new order.
    pub fn set_playlist(&mut self, order: &[usize]) -> bool {
        let n = self.channels.len();
        let mut seen = vec![false; n];
        let once = |c: &usize| *c < n && !std::mem::replace(&mut seen[*c], true);
        if order.len() != n || !order.iter().all(once) || order == self.playlist() {
            return false;
        }
        let master = order.iter().position(|&c| c == 0).unwrap_or(0);
        self.master_below = (master > 0).then(|| self.channels[order[master - 1]].id);
        // By new index, the old.
        let old: Vec<usize> = (std::iter::once(0))
            .chain(order.iter().copied().filter(|&c| c > 0))
            .collect();
        let selected = usize::try_from(self.file.selected_channel).ok();
        if let Some(at) = selected.and_then(|s| old.iter().position(|&c| c == s)) {
            self.file.selected_channel = at as i32;
        }
        let mut channels: Vec<Option<Channel>> = std::mem::take(&mut self.channels)
            .into_iter()
            .map(Some)
            .collect();
        self.channels = old.iter().filter_map(|&c| channels[c].take()).collect();
        true
    }

    /// The auto name of a lane for `target` on channel `channel` ([`Channel::automation_label`]), where a send param
    /// (`0x1000` / `0x2000` + the target's id) is named after its target channel:
    /// `Enable Send (to <name>)` / `Send Level (to <name>)` ([`ChannelParams::send_name`]).
    pub fn automation_label(&self, channel: usize, target: AutomationTarget) -> Option<String> {
        let ch = self.channels.get(channel)?;
        let index = usize::try_from(target.param).ok();
        match index.filter(|_| target.instance_id == 0) {
            Some(index @ 0x1000..=0x2FFF) => {
                let id = (index & 0xFFF) as i32;
                let to = self.channels.iter().find(|c| c.id == id)?;
                ChannelParams::send_name(index, &to.name)
            }
            _ => ch.automation_label(target),
        }
    }

    /// The send channels' (id, bus), for [`output_bus`].
    pub fn send_buses(&self) -> Vec<(i32, i32)> {
        (self.channels.iter())
            .filter(|c| c.kind == ChannelKind::EffectTrack)
            .map(|c| (c.id, c.bus_id))
            .collect()
    }

    /// Prepares the song for a save at `now` (`YYYY-MM-DD HH:MM:SS`, local time): the `META` modified date, the created
    /// date if it has none (with dashes in the time), the version of its layout ([`FORMAT_VERSION`] once its zooms are
    /// per U, [`Self::zooms_to_units`]), and a random GUID for the song and for every channel that has none (kept ones
    /// stay). The writer itself is deterministic. The app's other view state (HEAD @0
    /// selected channel, @288 playhead) is set here too when it is kept; none is yet.
    pub fn stamp_save(&mut self, now: &str) {
        let meta = &mut self.file.meta;
        meta.modified = now.to_owned();
        if meta.created == MetaState::default().created {
            meta.created = match now.split_once(' ') {
                Some((date, time)) => format!("{date} {}", time.replace(':', "-")),
                None => now.to_owned(),
            };
        }
        self.file.version = match self.file.zooms_per_u() {
            true => FORMAT_VERSION,
            false => PIXEL_ZOOM_VERSION,
        }
        .to_owned();
        if self.file.guid == [0; 16] {
            self.file.guid = random_guid(-1);
        }
        for c in &mut self.channels {
            if c.file.guid == [0; 16] {
                c.file.guid = random_guid(c.id);
            }
        }
    }

    /// Makes the view zooms count per U, at `u_px` device pixels a U, if they count per device pixel (a file without
    /// the zoom-units flag, a new song): the playlist's always, each clip's `ZOOM` zooms and each channel's
    /// `CHHD`@1040 where above 0. Then sets the flag. A song new to the app takes it once, at the U it opens with.
    pub fn zooms_to_units(&mut self, u_px: f64) {
        if self.file.zooms_per_u() {
            return;
        }
        self.file.playlist_view[2] *= u_px;
        self.file.playlist_view[3] *= u_px;
        for c in &mut self.channels {
            for zoom in std::iter::once(&mut c.file.view[0]).chain(
                (c.lanes.iter_mut().flat_map(|l| &mut l.clips))
                    .flat_map(|clip| &mut clip.file.zoom[2..]),
            ) {
                if *zoom > 0.0 {
                    *zoom *= u_px;
                }
            }
        }
        match self.file.head_extra.first_mut() {
            Some(flag) => *flag = 1,
            None => self.file.head_extra.push(1),
        }
    }

    /// Takes from `current` what is not an edit, onto a song that undo or redo restored (or that a history compares):
    /// the title (a save sets it), the META display title, author and link (the PROJECT tab types them), the play loop
    /// (the rulers set it), the save stamp ([`Self::stamp_save`]: version, song GUID, created and modified dates), the
    /// view state a file keeps (HEAD @0, @288, @296, META editing time; per channel by id, CHHD @1040/@1048 and its
    /// GUID), and the META@776 lock (PROJECT confirms it "forever"). So none of them is ever an undo step. (Each track's
    /// current clip is view state too: the app carries it across a step wherever the step moved the track,
    /// `app/tracks.rs` `carry_current_clips`.)
    pub fn keep_unedited(&mut self, current: &Song) {
        self.title.clone_from(&current.title);
        (self.file.meta.display_title).clone_from(&current.file.meta.display_title);
        self.file.meta.author.clone_from(&current.file.meta.author);
        self.file
            .meta
            .info_url
            .clone_from(&current.file.meta.info_url);
        self.play_loop = current.play_loop;
        let (file, from) = (&mut self.file, &current.file);
        file.version.clone_from(&from.version);
        file.selected_channel = from.selected_channel;
        file.playhead_beats = from.playhead_beats;
        file.playlist_view = from.playlist_view;
        file.guid = from.guid;
        file.meta.created.clone_from(&from.meta.created);
        file.meta.modified.clone_from(&from.meta.modified);
        file.meta.editing_seconds = from.meta.editing_seconds;
        file.meta.locked = from.meta.locked;
        for c in &mut self.channels {
            if let Some(from) = current.channels.iter().find(|f| f.id == c.id) {
                c.file.guid = from.file.guid;
                c.file.view = from.file.view;
            }
        }
    }

    /// Takes each track's current clip (DESc@24) from `current`: view state, which a tap sets (`ui/clips.rs`), and so
    /// never an edit or an unsaved change. Channels match by id, tracks by index and kind: for a comparison, where a song
    /// whose tracks moved differs anyway.
    pub fn keep_current_clips(&mut self, current: &Song) {
        for c in &mut self.channels {
            let Some(from) = current.channels.iter().find(|f| f.id == c.id) else {
                continue;
            };
            for (lane, from) in c.lanes.iter_mut().zip(&from.lanes) {
                if lane.kind == from.kind {
                    lane.file.current_clip_id = from.file.current_clip_id;
                }
            }
        }
    }

    /// `base`, or `<base> N` for the smallest free `N >= 2` if a channel has that name.
    pub(crate) fn free_name(&self, base: &str) -> String {
        let taken = |name: &str| self.channels.iter().any(|c| c.name == name);
        if !taken(base) {
            return base.to_owned();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|name| !taken(name))
            .expect("an unbounded range has a free number")
    }

    /// Takes the next channel id: the song's counter (HEAD@354), or more if a channel has that id already.
    /// `None` at [`CHANNEL_ID_LIMIT`] (the app then says so).
    fn next_id(&mut self) -> Option<i32> {
        let max = self.channels.iter().map(|c| c.id).max().unwrap_or(0);
        let id = self.file.next_channel_id.max(max + 1);
        (id < CHANNEL_ID_LIMIT).then(|| {
            self.file.next_channel_id = id + 1;
            id
        })
    }

    /// Takes the next send bus: one more than the song's counter (HEAD@4) and every bus, at least 1.
    fn next_bus(&mut self) -> i32 {
        let max = self.channels.iter().map(|c| c.bus_id).max().unwrap_or(0);
        self.file.last_send_bus = self.file.last_send_bus.max(max).max(0) + 1;
        self.file.last_send_bus
    }

    /// New random GUIDs for the song and every channel: a song made from a template is a song of its own. Nothing in a
    /// song refers to a GUID (sends go by channel id), so nothing else changes.
    pub fn new_guids(&mut self) {
        self.file.guid = random_guid(-1);
        for c in &mut self.channels {
            c.file.guid = random_guid(c.id);
        }
    }
}

/// RHED[12]: the bus of the first `SEND` target that is a send channel (`buses`: their id and bus, see
/// [`Song::send_buses`]), else −1, the master. `Channel::output_bus` follows this rule.
pub fn output_bus(sends: &[SendState], buses: &[(i32, i32)]) -> i32 {
    (sends.iter())
        .find_map(|send| buses.iter().find(|(id, _)| *id == send.target_channel_id))
        .map_or(-1, |&(_, bus)| bus)
}

/// 16 random bytes for a GUID (`salt` tells apart the ones made at once). Native: std's OS-seeded `RandomState`. On
/// the web its keys are not random (wasm32-unknown-unknown has no OS source), so `Math.random()` is mixed in.
fn random_guid(salt: i32) -> [u8; 16] {
    use std::hash::{BuildHasher, RandomState};
    let state = RandomState::new();
    let mut guid = [0; 16];
    for (n, half) in guid.as_chunks_mut::<8>().0.iter_mut().enumerate() {
        half.copy_from_slice(&state.hash_one((salt, n, entropy())).to_le_bytes());
    }
    guid
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = Math, js_name = random)]
    fn math_random() -> f64;
}

/// Random bits the platform's `RandomState` lacks: 53 bits of `Math.random()` on the web, nothing natively.
fn entropy() -> u64 {
    #[cfg(target_arch = "wasm32")]
    return (math_random() * (1_u64 << 53) as f64) as u64;
    #[cfg(not(target_arch = "wasm32"))]
    0
}

#[cfg(test)]
mod tests;

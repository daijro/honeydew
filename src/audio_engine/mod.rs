mod audio_clips;

pub use audio_clips::{fit_audio_clip, source_beats};
mod drums;
mod generators;
mod input;
mod live;
mod merge;
mod prepare;
mod preview;
mod routing;
pub(crate) mod samples;
pub(crate) mod settings;
pub(crate) mod slicer;
mod slides;

pub use input::{LiveInput, MONITOR_MAX_LAG_FRAMES, RecordTap, TakeStart};
pub(crate) use prepare::burned_notes;

use std::sync::Arc;

use crate::audio_bridge::{SongAudioBridge, SongAudioError};
use crate::sample_cache::SampleCache;
use crate::settings::Settings;
use crate::song::{PPQ, Song};
use honeydew_audio::analysis;
use honeydew_audio::automation::AutomationTarget;
use honeydew_audio::device::{AudioRenderer, ModuleKey, PingReport};
use honeydew_audio::instrument::{Instrument, InstrumentModule};
use honeydew_audio::metronome::{self, Metronome};
use honeydew_audio::mixer::{MixError, Mixer, OwnedStereoBuffer};
use honeydew_audio::note_effects::{LiveNoteEvent, NoteEffectError, NoteEffectTrace};
use honeydew_audio::playback::AudioPlayback;
use honeydew_audio::sample::SampleData;
use honeydew_audio::{MeterSnapshot, ProcessContext};
use input::Monitor;
use prepare::{Action, KIT, ParamTimeline, ScheduledAction, beat_frame};

pub use slicer::{auto_clip as slicer_auto_clip, settle_first_notes};

const BLOCK_FRAMES: usize = 1024;
/// Channel param 4: Pitch Bend.
const PITCH_BEND: usize = 4;
/// Channel param 7 + 64: MIDI CC #64, the sustain pedal.
const SUSTAIN_PEDAL: usize = 71;

/// The strip param that channel param `index` drives: MIDI CC #7 (14) sets the Level, CC #8 (15) and CC #10 (17) the
/// Pan. Changes are aliased where they enter (live `SetParam` and prepared automation), so the later of the Level and
/// CC #7 wins, on seek too.
pub(crate) fn strip_param(instance_id: i32, index: i32) -> i32 {
    match (instance_id, index) {
        (0, 14) => 0,
        (0, 15 | 17) => 1,
        _ => index,
    }
}

/// Channel param `index` of instance `instance_id` is a MIDI CC (7 + n, the sustain pedal's too), which Stop and
/// re-sync step 1 keep.
fn midi_cc(instance_id: i32, index: i32) -> bool {
    instance_id == 0 && (7..=133).contains(&index)
}

#[derive(Debug)]
pub enum EngineError {
    InvalidTiming,
    Song(SongAudioError),
    Mixer(MixError),
    Notes(NoteEffectError),
    /// The clips' loop passes would expand to more event plays than the file reader's memory budget allows.
    TooManyEvents,
}

pub struct SongAudioEngine {
    audio: AudioPlayback,
    mixer: Mixer,
    /// Per channel, its instruments in rack order, by instance id.
    instruments: Vec<Vec<(i32, InstrumentModule)>>,
    /// Per channel, its pad sampler; `None` without drum slots.
    kits: Vec<Option<drums::Kit>>,
    /// Per channel, its audio clips' slots; `None` without audio clips.
    clips: Vec<Option<audio_clips::Clips>>,
    /// Per channel, its live keys' path; `None` where nothing plays a note.
    live: Vec<Option<live::Live>>,
    /// The live notes one channel sends at a time.
    live_out: Vec<LiveNoteEvent>,
    buffers: Vec<OwnedStereoBuffer>,
    actions: Vec<ScheduledAction>,
    /// Each param's value before automation, the saved one or the last sent live ([`AudioRenderer::set_param`]), by
    /// param.
    initial: Vec<(usize, i32, i32, f32)>,
    /// Each `initial` param's value as last set ([`Self::apply_param`]): what the module holding it has now. An engine
    /// built after an edit sets a module it takes over only the params that change ([`merge`]).
    applied: Vec<f32>,
    /// The modules' saved settings, which an engine built after an edit gives the modules it takes over too.
    settings: Vec<(usize, i32, u32, f32)>,
    param_timelines: Vec<ParamTimeline>,
    action_index: usize,
    channel_controls: Vec<[f32; 4]>,
    /// Per channel, instrument (as in `instruments`) and key, the note-offs held by the sustain pedal.
    sustained: Vec<Vec<[u8; 128]>>,
    /// Per channel and key, the note-offs the sustain pedal holds for its pad sampler.
    kit_sustained: Vec<[u8; 128]>,
    /// The channel ids and the solo links of the song's sends and pad targets.
    routing: routing::Routing,
    /// What the sequencer holds, as the actions played so far left it ([`hold`]): per channel, instrument (as in
    /// `instruments`, then the pads' notes) and key, the note-ons not yet matched by a note-off; per channel and pad,
    /// the hits not yet released; per channel, the keys at its input. A re-sync releases what no note covers.
    held: Vec<Vec<[u8; 128]>>,
    held_pads: Vec<Vec<u8>>,
    held_keys: Vec<u128>,
    /// Scratch for a re-sync ([`Self::resync`]), shaped as `held`: what covers the new beat ([`cover`]).
    cover: Vec<Vec<[u8; 128]>>,
    cover_pads: Vec<Vec<u8>>,
    cover_keys: Vec<u128>,
    /// The re-syncs that replayed the events for their cover: a seek or Mute change that found something held.
    cover_replays: u64,
    /// The cover after every [`CHECKPOINT_SPAN`] actions, so a re-sync replays at most that many ([`Checkpoints`]).
    checkpoints: Checkpoints,
    /// The actions the covers replayed, all told.
    cover_events: u64,
    /// Per channel, the keys sounding at its input ([`AudioRenderer::sounding`]).
    sounding: Vec<u128>,
    meter_keys: Vec<ModuleKey>,
    /// The note effects' traces (they run ahead of playback), by meter, and each meter's note effect analysis: empty
    /// for any other module.
    note_traces: Vec<(usize, NoteEffectTrace)>,
    note_slabs: Vec<Vec<f32>>,
    channel_meters: Vec<MeterSnapshot>,
    /// Each meter's peaks over the last render call, which actions split into segments.
    held_meters: Vec<MeterSnapshot>,
    sample_rate: f32,
    /// The live tempo: the build's, or a tempo drag's ([`AudioRenderer::set_tempo`]).
    tempo_bpm: f32,
    /// The tempo the prepared events, param timelines, loop and audio clip spans are timed at: the transport counts
    /// frames at this tempo, and a live tempo only changes how fast they go by ([`Self::speed`]).
    built_bpm: f32,
    /// The transport position, in frames at `built_bpm`.
    frame: u64,
    /// The position past `frame`, in [0, 1) of a frame, while the live tempo is not `built_bpm`.
    phase: f64,
    /// The loop range [start, end) in frames.
    loop_frames: Option<(u64, u64)>,
    /// The song length in beats (the latest clip end rounded up to whole bars, at least one), where a transport with no
    /// loop wraps to the start ([`Self::wrap_range`]). `None` for an export, which plays on into its tail
    /// ([`Self::play_past_song_end`]).
    song_beats: Option<f64>,
    playing: bool,
    /// The click voice: the metronome's beats and the count-in's.
    metronome: Metronome,
    /// The metronome clicks on the song's beats while the transport rolls. It follows REC's arm (`App::auto_record`,
    /// its only sender), so it also tells a recording, which does not wrap at the song end ([`Self::wrap_range`]).
    metronome_on: bool,
    /// The metronome flag (the tempo dialog's toggle): the beats click while it is on too, REC armed or not. It never
    /// stops the song-end wrap. An export's engine never gets it.
    metronome_flag: bool,
    /// The click level, from the `Metronome level` setting.
    metronome_level: f32,
    /// b: the accent sounds on beats with floor(beat) mod b = 0.
    bar_beats: u32,
    /// Beats the next Play counts in.
    count_in_beats: u32,
    /// The running count-in: its frames done, its length and its beats. The transport holds until it ends.
    count_in: Option<(u64, u64, u32)>,
    /// The index of the last beat clicked, the count-in's or the song's: a beat clicks only past it, so a retime that
    /// lands back on a beat clicked never clicks it again. A jump forgets it: Play, the count-in's end, a seek and a
    /// loop wrap.
    clicked_beat: Option<u64>,
    /// The latency measurement, with the number it was started with ([`input`]).
    ping: Option<(u32, crate::recording::calibrate::Ping)>,
    /// Frames output since the build.
    rendered: u64,
    monitor: Option<Monitor>,
    /// Built for a song edit: it takes over the engine it replaces ([`Self::adopting`]).
    adopt: bool,
    /// The edit's position re-sync runs as it takes over ([`Self::adopting_edit`]).
    resync: bool,
    /// The channels that get all notes off as it takes over, before the re-sync: a deleted lane's.
    silence: Vec<usize>,
    /// The pickers' sample preview while it plays ([`AudioRenderer::preview`]).
    preview: Option<preview::Preview>,
}

impl SongAudioEngine {
    /// The engine for `song` with no samples: every sample is missing (silent).
    pub fn new(song: &Song, sample_rate: f32) -> Result<Self, EngineError> {
        Self::with_samples(song, sample_rate, &SampleCache::default())
    }

    /// The engine for `song`, playing the samples `samples` has loaded for it ([`SampleCache::load`]). It takes `Arc`
    /// clones; nothing is decoded here or on the audio thread.
    pub fn with_samples(
        song: &Song,
        sample_rate: f32,
        samples: &SampleCache,
    ) -> Result<Self, EngineError> {
        Self::with_input(song, sample_rate, samples, None)
    }

    /// [`Self::with_samples`], monitoring `input` on `song`'s Audio Record channels that have monitoring on. A ring
    /// at another rate is not monitored.
    pub fn with_input(
        song: &Song,
        sample_rate: f32,
        samples: &SampleCache,
        input: Option<&LiveInput>,
    ) -> Result<Self, EngineError> {
        let tempo_bpm = song.tempo_bpm as f32;
        if !sample_rate.is_finite()
            || sample_rate <= 0.0
            || !tempo_bpm.is_finite()
            || tempo_bpm <= 0.0
        {
            return Err(EngineError::InvalidTiming);
        }
        let mut bridge = SongAudioBridge::new(song, sample_rate).map_err(EngineError::Song)?;
        for channel in 0..song.channels.len() {
            bridge
                .audio
                .process_channel(
                    channel,
                    &mut [],
                    &mut [],
                    ProcessContext {
                        sample_rate,
                        tempo_bpm,
                        start_beat: 0.0,
                        playing: false,
                    },
                )
                .map_err(|error| EngineError::Song(SongAudioError::Playback(error)))?;
        }
        let (mixes, routing) = routing::build(song);
        let mixer = Mixer::new(&mixes).map_err(EngineError::Mixer)?;
        let mut instruments = Vec::with_capacity(song.channels.len());
        let mut kits = Vec::with_capacity(song.channels.len());
        let mut clips = Vec::with_capacity(song.channels.len());
        let mut lives = Vec::with_capacity(song.channels.len());
        let mut meter_keys = Vec::new();
        // The sends' Enable and Level, as channel params 0x1000 / 0x2000 + the target's id.
        let mut initial: Vec<_> = (mixes.iter().enumerate())
            .flat_map(|(channel, mix)| mix.sends.iter().map(move |send| (channel, send)))
            .flat_map(|(channel, send)| {
                let id = routing.ids[send.target];
                [
                    (channel, 0, 0x1000 + id, send.enabled),
                    (channel, 0, 0x2000 + id, send.level),
                ]
            })
            .collect();
        for (channel_index, channel) in song.channels.iter().enumerate() {
            meter_keys.push(ModuleKey {
                channel: channel_index,
                instance_id: 0,
            });
            for (index, value) in channel.params.0.iter().copied().enumerate() {
                initial.push((channel_index, 0, index as i32, value));
            }
            let kit = drums::build(song, channel_index, sample_rate, samples);
            // Slot params live at instance −id, restored on seek like any other; so do the slots' meters.
            let slots = channel.drum_kit.iter().flat_map(|kit| &kit.slots);
            for slot in slots.filter(|slot| kit.is_some() && slot.file.id > 0) {
                let id = -(slot.file.id as i32);
                meter_keys.push(ModuleKey {
                    channel: channel_index,
                    instance_id: id,
                });
                for (index, value) in slot.params.iter().copied().enumerate() {
                    initial.push((channel_index, id, index as i32, value));
                }
            }
            kits.push(kit);
            clips.push(audio_clips::build(
                song,
                channel_index,
                sample_rate,
                tempo_bpm,
                samples,
            )?);
            // A CC lane's target rests at 0 before its first point: the song stores no value for it.
            for target in channel.lanes.iter().filter_map(|lane| lane.target) {
                let index = strip_param(0, target.param);
                let key = (channel_index, 0, index);
                if target.instance_id == 0 && !initial.iter().any(|&(c, i, p, _)| (c, i, p) == key)
                {
                    initial.push((channel_index, 0, index, 0.0));
                }
            }
            let mut modules = generators::build(
                channel,
                sample_rate,
                tempo_bpm,
                BLOCK_FRAMES,
                samples,
                song.time_sig,
            );
            // Each instrument runs after the effects above it.
            let mut effects = 0;
            for module in &channel.rack.modules {
                let id = module.instance_id;
                if bridge.audio.effect(channel_index, id).is_some() {
                    effects += 1;
                } else if let Some((_, instrument)) = modules.iter_mut().find(|(i, _)| *i == id) {
                    instrument.effects_above = effects;
                }
            }
            for module in &channel.rack.modules {
                let id = module.instance_id;
                // An Arpeggiator's or a Note Echo's panel shows its live state too; it has no params here.
                let traced = matches!(module.type_id, 47 | 51);
                if !modules.iter().any(|(instrument, _)| *instrument == id)
                    && bridge.audio.effect(channel_index, id).is_none()
                    && !traced
                {
                    continue;
                }
                meter_keys.push(ModuleKey {
                    channel: channel_index,
                    instance_id: id,
                });
                if traced {
                    continue;
                }
                for (index, value) in module.params.iter().copied().enumerate() {
                    initial.push((channel_index, id, index as i32, value));
                }
                for (index, value) in module.common.0.iter().copied().enumerate() {
                    initial.push((channel_index, id, -(index as i32) - 1, value));
                }
            }
            lives.push(live::build(
                song,
                channel_index,
                &modules,
                sample_rate,
                tempo_bpm,
            )?);
            instruments.push(modules);
        }
        // By param, as the timelines are: `restore_params` looks one up for each.
        initial.sort_by_key(|&(channel, instance_id, index, _)| (channel, instance_id, index));
        let mut traces = Vec::new();
        let actions = prepare::actions(
            song,
            &bridge,
            &instruments,
            sample_rate,
            tempo_bpm,
            &mut traces,
        )?;
        let mut note_slabs = vec![Vec::new(); meter_keys.len()];
        let note_traces: Vec<(usize, NoteEffectTrace)> = (traces.into_iter())
            .filter_map(|(channel, trace)| {
                let id = prepare::trace_id(&trace);
                let at = meter_keys
                    .iter()
                    .position(|k| (k.channel, k.instance_id) == (channel, id))?;
                note_slabs[at] = vec![0.0; analysis::note_effect_len(&trace)];
                Some((at, trace))
            })
            .collect();
        let param_timelines = prepare::timelines(&actions);
        let count = song.channels.len();
        let sustained = instruments
            .iter()
            .map(|modules| vec![[0; 128]; modules.len()])
            .collect();
        let held: Vec<_> = instruments
            .iter()
            .map(|modules| vec![[0; 128]; modules.len() + 1])
            .collect();
        let held_pads: Vec<_> = (kits.iter())
            .map(|kit| vec![0; kit.as_ref().map_or(0, |kit| kit.sampler.pads())])
            .collect();
        let settings = settings::saved(song);
        let checkpoints = Checkpoints::new(&actions, &instruments, &held, &held_pads);
        let mut engine = Self {
            audio: bridge.audio,
            mixer,
            instruments,
            kits,
            clips,
            live: lives,
            live_out: Vec::with_capacity(1024),
            buffers: (0..count)
                .map(|_| OwnedStereoBuffer {
                    left: vec![0.0; BLOCK_FRAMES],
                    right: vec![0.0; BLOCK_FRAMES],
                })
                .collect(),
            actions,
            // None set yet: the build's `restore_params` sets every one.
            applied: vec![f32::NAN; initial.len()],
            initial,
            settings,
            param_timelines,
            action_index: 0,
            channel_controls: song
                .channels
                .iter()
                .map(|channel| {
                    [
                        channel.params.0[0],
                        channel.params.0[1],
                        channel.params.0[2],
                        channel.params.0[3],
                    ]
                })
                .collect(),
            sustained,
            kit_sustained: vec![[0; 128]; count],
            routing,
            cover: held.clone(),
            cover_pads: held_pads.clone(),
            cover_keys: vec![0; count],
            cover_replays: 0,
            checkpoints,
            cover_events: 0,
            held,
            held_pads,
            held_keys: vec![0; count],
            sounding: vec![0; count],
            held_meters: vec![MeterSnapshot::default(); meter_keys.len()],
            meter_keys,
            note_traces,
            note_slabs,
            channel_meters: vec![MeterSnapshot::default(); count],
            sample_rate,
            tempo_bpm,
            built_bpm: tempo_bpm,
            frame: 0,
            phase: 0.0,
            loop_frames: None,
            song_beats: Some(crate::render::song_length_beats(song)),
            playing: false,
            metronome: Metronome::new(sample_rate),
            metronome_on: false,
            metronome_flag: false,
            metronome_level: metronome::level(Settings::default().metronome_level),
            bar_beats: song.time_sig.bar_beats(),
            count_in_beats: 0,
            count_in: None,
            clicked_beat: None,
            ping: None,
            rendered: 0,
            monitor: input.and_then(|input| Monitor::new(song, input, sample_rate)),
            adopt: false,
            resync: false,
            silence: Vec::new(),
            preview: None,
        };
        engine.sync_mixer_controls();
        engine.apply_settings();
        engine.restore_params(0, false);
        engine.set_loop(
            song.play_loop
                .map(|(start, end)| (start.into(), end.into())),
        );
        Ok(engine)
    }

    /// This engine, built for a song that differs from the playing one only in its clips (a Mute or Unmute,
    /// CLHd@36), takes over the engine it replaces instead of starting afresh, and re-syncs
    /// ([`AudioRenderer::take_over`]).
    pub fn adopting(self) -> Self {
        self.adopting_edit(true, Vec::new())
    }

    /// [`Self::adopting`] for a song edit: the position re-sync runs only if `resync`, after all notes off on each
    /// channel of `silence` (a deleted lane's).
    pub fn adopting_edit(mut self, resync: bool, silence: Vec<usize>) -> Self {
        (self.adopt, self.resync, self.silence) = (true, resync, silence);
        self
    }

    /// [`Self::adopting`] was called.
    pub fn adopts(&self) -> bool {
        self.adopt
    }

    /// An adopting engine re-syncs as it takes over.
    pub fn resyncs(&self) -> bool {
        self.resync
    }

    /// The channels an adopting engine silences as it takes over.
    pub fn silences(&self) -> &[usize] {
        &self.silence
    }

    /// Its song's channel ids (`CHHD`@1080), by channel index.
    pub fn channel_ids(&self) -> &[i32] {
        &self.routing.ids
    }

    /// Channel `channel`'s instrument `instance_id`.
    fn instrument(&mut self, channel: usize, instance_id: i32) -> Option<&mut InstrumentModule> {
        let modules = self.instruments.get_mut(channel)?;
        let (_, module) = modules.iter_mut().find(|(id, _)| *id == instance_id)?;
        Some(module)
    }

    /// The pad sampler of `channel`.
    fn kit(&mut self, channel: usize) -> Option<&mut honeydew_audio::drums::PadSampler> {
        let kit = self.kits.get_mut(channel)?.as_mut()?;
        Some(&mut kit.sampler)
    }

    /// Every instrument of `channel`.
    fn instruments_of(
        &mut self,
        channel: usize,
    ) -> impl Iterator<Item = &mut (dyn Instrument + 'static)> {
        let modules = self.instruments.get_mut(channel).into_iter().flatten();
        modules.map(|(_, module)| module.instrument.as_mut())
    }

    fn apply_param(&mut self, channel: usize, instance_id: i32, index: i32, value: f32) {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return;
        }
        if let Some(at) = position(&self.initial, (channel, instance_id, index)) {
            self.applied[at] = value;
        }
        if instance_id == 0
            && let Some((target, level)) = self.routing.send(index)
        {
            let _ = self.mixer.set_send(channel, target, level, value);
            return;
        }
        if instance_id < 0 {
            if let Some(Some(kit)) = self.kits.get_mut(channel) {
                kit.set_slot_param(instance_id.unsigned_abs(), index, value);
            }
            return;
        }
        if let Some(module) = self.instrument(channel, instance_id) {
            module.set_parameter(index, value);
            return;
        }
        let target = AutomationTarget {
            channel,
            instance_id,
            parameter_index: index,
        };
        if self.audio.set_target(target, value).is_err() {
            return;
        }
        if instance_id != 0 {
            // A param set may move another of its effect (Tremolo, ID 46: Max below Min lowers Min): `applied` takes
            // what the effect holds now, so a re-sync or an edit still sets the other back.
            if let (Some(effect), true) = (self.audio.effect(channel, instance_id), index >= 0) {
                let key = (channel, instance_id, 0);
                let start = (self.initial).partition_point(|&(c, i, p, _)| (c, i, p) < key);
                for at in start..self.initial.len() {
                    let (c, i, p, _) = self.initial[at];
                    match effect.parameter(p as usize) {
                        Some(held) if (c, i) == (channel, instance_id) => self.applied[at] = held,
                        _ => break,
                    }
                }
            }
            return;
        }
        match usize::try_from(index) {
            Ok(control @ 0..=3) => {
                self.channel_controls[channel][control] = value;
                self.sync_mixer_controls();
            }
            // The engine holds note-offs for the sustain pedal; instruments never see CC #64.
            Ok(SUSTAIN_PEDAL) => {
                if value <= 0.5 {
                    self.lift_pedal(channel);
                }
            }
            Ok(controller) => {
                if controller == PITCH_BEND
                    && let Some(kit) = self.kit(channel)
                {
                    kit.set_bend(value);
                }
                for instrument in self.instruments_of(channel) {
                    instrument.set_controller(controller, value);
                }
            }
            Err(_) => {}
        }
    }

    /// Mute and Solo are on above 0.5: a channel is out of the mix while its Mute is on, or while another channel's
    /// Solo is on and its own is not, unless a path of sends or pad targets joins it to a soloed channel. The master is
    /// never out under a solo.
    // solo keeps send and pad-target sources audible.
    fn sync_mixer_controls(&mut self) {
        let soloed = |controls: &[f32; 4]| controls[3] > 0.5;
        let solo = self.channel_controls.iter().any(soloed);
        for (index, controls) in self.channel_controls.iter().enumerate() {
            let linked =
                || (self.routing.linked[index].iter()).any(|&o| soloed(&self.channel_controls[o]));
            let mut level = controls[0];
            if controls[2] > 0.5 || (index != 0 && solo && !soloed(controls) && !linked()) {
                level = 0.0;
            }
            let _ = self.mixer.set_channel_controls(index, level, controls[1]);
        }
    }

    fn apply_actions(&mut self) {
        while self.action_index < self.actions.len()
            && self.actions[self.action_index].frame <= self.frame
        {
            let event = self.actions[self.action_index];
            match event.action {
                Action::NoteOn {
                    instance_id: KIT,
                    key,
                    velocity,
                    ..
                } => {
                    if let Some(kit) = self.kit(event.channel) {
                        kit.note_on(key, velocity);
                    }
                }
                Action::NoteOn {
                    instance_id,
                    key,
                    velocity,
                    pan,
                } => {
                    let module = self.instrument(event.channel, instance_id);
                    if let Some(module) = module.filter(|module| module.hears(key)) {
                        module.instrument.note_on(key, velocity, pan);
                    }
                }
                Action::NoteOff { instance_id, key } => {
                    self.key_up(event.channel, Some(instance_id), key);
                }
                Action::Hit {
                    pad,
                    velocity,
                    pan,
                    pitch,
                    ..
                } => {
                    if let Some(kit) = self.kit(event.channel) {
                        kit.hit(pad, velocity, pan, pitch);
                    }
                }
                Action::PadOff { pad } => {
                    if let Some(kit) = self.kit(event.channel) {
                        kit.release(pad);
                    }
                }
                Action::Param {
                    instance_id,
                    index,
                    value,
                } => self.apply_param(event.channel, instance_id, index, value),
                Action::Key { key, on } => set_key(&mut self.sounding[event.channel], key, on),
                Action::ClipHit { pad, offset } => {
                    if let Some(clips) = self.clips[event.channel].as_mut() {
                        clips.sampler.hit_at(pad, 1.0, 0.5, 1.0, offset);
                    }
                }
                Action::ClipFade { pad } => {
                    if let Some(clips) = self.clips[event.channel].as_mut() {
                        clips.sampler.fade(pad);
                    }
                }
                Action::ClipParam { pad, index, value } => {
                    if let Some(clips) = self.clips[event.channel].as_mut() {
                        clips.sampler.set_param(pad, index, value);
                    }
                }
                Action::StepCover { .. } => {}
                Action::Slide {
                    key,
                    pitch,
                    pan,
                    velocity,
                } => {
                    // A slide event is its slide note's, so the note's key meets each module's key range (and
                    // its key-learn) like a note's.
                    let modules = self
                        .instruments
                        .get_mut(event.channel)
                        .into_iter()
                        .flatten();
                    for (_, module) in modules.filter(|(_, module)| module.hears(key)) {
                        module.instrument.set_slide(pitch, pan, velocity);
                    }
                }
            }
            let instruments = &self.instruments;
            let (keys, pads, input) = (&mut self.held, &mut self.held_pads, &mut self.held_keys);
            hold(&event, instruments, keys, pads, input, true);
            self.action_index += 1;
        }
    }

    fn render_segment(&mut self, output: &mut [f32], channels: usize, frames: usize) {
        for buffer in &mut self.buffers {
            buffer.left.resize(frames, 0.0);
            buffer.right.resize(frames, 0.0);
            buffer.left.fill(0.0);
            buffer.right.fill(0.0);
        }
        let context = ProcessContext {
            sample_rate: self.sample_rate,
            tempo_bpm: self.tempo_bpm,
            start_beat: self.beat(),
            playing: self.rolling(),
        };
        // Pads, clips and input first, into their channel's (or a pad's send target's) buffer; then each channel's rack
        // runs on it in order, in place, every instrument adding at its own position.
        for kit in self.kits.iter_mut().flatten() {
            kit.render(&mut self.buffers);
        }
        for (clips, buffer) in self.clips.iter_mut().zip(&mut self.buffers) {
            if let Some(clips) = clips {
                clips.render(buffer);
            }
        }
        if let Some(monitor) = &mut self.monitor {
            monitor.add(&mut self.buffers, frames);
        }
        let (audio, instruments) = (&mut self.audio, &mut self.instruments);
        let rack = |index: usize, left: &mut [f32], right: &mut [f32]| {
            let mut done = 0;
            for (_, module) in &mut instruments[index] {
                audio.process_effects(index, done..module.effects_above, left, right, context)?;
                module.process(left, right, context);
                done = module.effects_above;
            }
            audio.process_effects(index, done..usize::MAX, left, right, context)
        };
        if self.mixer.mix(&mut self.buffers, rack).is_err() {
            return;
        }
        for (meter, buffer) in self.channel_meters.iter_mut().zip(&self.buffers) {
            meter.peak_left = buffer
                .left
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            meter.peak_right = buffer
                .right
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            meter.gain_reduction_db = 0.0;
        }
        // The click and the sample preview go out past the master and its meter.
        let master = &mut self.buffers[0];
        self.metronome.add(&mut master.left, &mut master.right);
        if let Some(preview) = &mut self.preview
            && !preview.add(&mut master.left, &mut master.right)
        {
            self.preview = None;
        }
        for frame in 0..frames {
            if channels == 1 {
                output[frame] = 0.5 * (master.left[frame] + master.right[frame]);
            } else {
                output[frame * channels] = master.left[frame];
                output[frame * channels + 1] = master.right[frame];
            }
        }
    }
}

impl AudioRenderer for SongAudioEngine {
    fn render(&mut self, output: &mut [f32], channels: usize) {
        output.fill(0.0);
        if channels == 0 || self.render_ping(output, channels) {
            return;
        }
        let total_frames = output.len() / channels;
        let mut cursor = 0;
        self.held_meters.fill(MeterSnapshot::default());
        while cursor < total_frames {
            if self.rolling() {
                self.wrap_at_loop_end();
                self.apply_actions();
            }
            self.live_due(cursor);
            let mut frames = (total_frames - cursor).min(BLOCK_FRAMES);
            if let Some(next) = self.live_next(cursor).filter(|next| *next > 0) {
                frames = next.min(frames as u64) as usize;
            }
            let (click, beat) = self.clicks();
            let count_in_end = self.count_in.map(|(done, end, _)| end - done);
            for stop in click.into_iter().chain(count_in_end) {
                frames = stop.min(frames as u64) as usize;
            }
            // The transport's stops, at the events' tempo: the next action, the loop or song end and the metronome's
            // next beat.
            if self.rolling() {
                let next = self.actions.get(self.action_index).map(|next| next.frame);
                let end = self.wrap_range().map(|(_, end)| end);
                for at in next.into_iter().chain(end).chain(beat) {
                    frames = self.frames_to(at).min(frames as u64) as usize;
                }
            }
            // Every pass renders a frame at least, so none can repeat: each stop is ahead of the frame (actions due and
            // the loop end were met above), and a count-in never stands at its end ([`Self::retime_count_in`]).
            let frames = frames.max(1);
            self.input_segment(cursor, frames, total_frames);
            self.render_segment(
                &mut output[cursor * channels..(cursor + frames) * channels],
                channels,
                frames,
            );
            self.hold_meters();
            cursor += frames;
            if let Some((done, end, _)) = &mut self.count_in {
                *done += frames as u64;
                if *done >= *end {
                    (self.count_in, self.clicked_beat) = (None, None);
                    self.resync();
                }
            } else if self.playing {
                self.advance(frames as u64);
                // A loop end on the call's last frame wraps now, not in the next call.
                self.wrap_at_loop_end();
            }
        }
        self.rendered += total_frames as u64;
        // The note effects' live state where the transport stands: their notes were worked out ahead.
        let beat = self.beat();
        let rolls = self.rolling();
        for (index, trace) in &self.note_traces {
            analysis::note_effect_state(trace, beat, rolls, &mut self.note_slabs[*index]);
        }
    }

    fn set_param(&mut self, channel: usize, instance_id: i32, index: i32, value: f32) {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return;
        }
        let index = strip_param(instance_id, index);
        self.apply_param(channel, instance_id, index, value);
        if let Some(at) = position(&self.initial, (channel, instance_id, index)) {
            self.initial[at].3 = value;
        }
    }

    /// MIDI's Pitch Bend or channel pressure: no initial value (the song holds none), so a re-sync and Stop put the
    /// saved one back. The on-screen pitch wheel writes the song, and comes as [`Self::set_param`].
    fn set_live_param(&mut self, channel: usize, index: i32, value: f32) {
        self.apply_param(channel, 0, strip_param(0, index), value);
    }

    /// Play start: the pickers' sample preview stops; every channel gets a full all-notes-off ([`Self::release_notes`]:
    /// Every key the sequencer, a live key or the sustain pedal alone holds is released and goes dark, the pedal itself
    /// untouched) and its audio clip slots fast-fade; then the position re-sync at the playhead, once any count-in has
    /// ended. A live press still queued (the key that starts playback) reaches the instruments after it, at the next
    /// block. Details:
    /// - the effects' transport reset (filter histories, a Leveller's smoothing) stays, so Stop then Play cuts the
    ///   reverb and delay tails at Play;
    /// - with a count-in, the preview stop, the all-notes-off and the effects reset happen at Play, the re-sync when it
    ///   ends; the keys played during the count-in sound on through it;
    /// - every channel's clip slots fade, as at Stop, not only those of a channel whose first lane is audio: a stopped
    ///   transport plays no other clip voice anyway.
    fn play(&mut self) {
        let beats = std::mem::take(&mut self.count_in_beats);
        if self.playing {
            return;
        }
        self.preview = None;
        self.release_notes(true);
        for clips in self.clips.iter_mut().flatten() {
            clips.fade_all();
        }
        if beats > 0 {
            let end = beat_frame(f64::from(beats), self.sample_rate, self.tempo_bpm);
            self.count_in = end.ok().filter(|end| *end > 0).map(|end| (0, end, beats));
        }
        (self.playing, self.clicked_beat) = (true, None);
        self.audio.reset_effects();
        if self.rolling() {
            self.resync();
        }
    }

    /// The transport's Stop: [`Self::halt`], then every param back to its value before automation, the saved or live
    /// one, but the MIDI CCs, which keep theirs, the sustain pedal's too (as re-sync step 1).
    fn stop(&mut self) {
        self.halt();
        for position in 0..self.initial.len() {
            let (channel, instance_id, index, value) = self.initial[position];
            if !midi_cc(instance_id, index) {
                self.apply_param(channel, instance_id, index, value);
            }
        }
    }

    /// While the transport rolls, the position re-sync at the new beat ([`Self::resync`]); stopped or counting in, it
    /// only moves the position: nothing happens to notes, params or audio until Play.
    fn seek(&mut self, tick: u64) {
        let beat = tick as f64 / f64::from(PPQ);
        let Ok(frame) = beat_frame(beat, self.sample_rate, self.built_bpm) else {
            return;
        };
        (self.frame, self.phase, self.clicked_beat) = (frame, 0.0, None);
        self.action_index = self.actions.partition_point(|event| event.frame < frame);
        if self.rolling() {
            self.resync();
        }
    }

    /// A live key: it sounds from the next rendered frame, routed by `channel`'s current lane ([`live`]).
    fn note_on(&mut self, channel: usize, key: u8, velocity: f32) {
        self.live_key_down(channel, key, velocity);
    }

    fn note_off(&mut self, channel: usize, key: u8) {
        self.live_key_up(channel, key);
    }

    /// An [`Self::adopting`] engine takes over `old` as it replaces it: a song edit, which never rebuilds the sound
    /// ([`Self::merge`]). Any engine takes the sample preview playing. Runs on the audio thread and allocates nothing;
    /// what `old` keeps leaves with it.
    fn take_over(&mut self, old: &mut Self) {
        self.preview = old.preview.take();
        self.take_input(old);
        if self.adopt {
            self.merge(old);
        }
    }

    fn adopts(&self) -> bool {
        self.adopt
    }

    /// A tempo change without a rebuild (a tempo drag): from the next frame the transport runs at `bpm`, at the same
    /// beat. The prepared events, param timelines, loop and audio clip spans stay timed at the build's tempo and keep
    /// their beats, as the transport's frames go by faster or slower; a running count-in keeps its beats; the
    /// instruments, the effects and the live keys' note effects take the tempo ([`live`]: an Arpeggiator's steps follow
    /// it, a free rate keeping its milliseconds, and Note Echo times what it makes from then on at it). During a drag
    /// The note effects' step interval is not recomputed: their prepared output is one of those compiled lists, so it
    /// keeps its beats, a Sync-off Arpeggiator's free rate included; the release's rebuild at the new tempo restores it
    /// ([`Self::merge`]). Note Echo's delays are beats either way. O(1) but for the instruments and the live notes in
    /// flight; allocates nothing.
    fn set_tempo(&mut self, bpm: f32) {
        if !bpm.is_finite() || bpm <= 0.0 || bpm == self.tempo_bpm {
            return;
        }
        let from = std::mem::replace(&mut self.tempo_bpm, bpm);
        self.count_in = self
            .count_in
            .map(|count_in| self.retime_count_in(count_in, from));
        for (_, module) in self.instruments.iter_mut().flatten() {
            module.instrument.set_tempo_bpm(bpm);
        }
        self.set_live_tempo(bpm);
    }

    /// The Arc it replaces (or ends) is never the last: the link keeps every previewed sample until the engine has let
    /// go of it ([`crate::audio_link::AudioLink::poll`]), so this is a count only.
    fn preview(&mut self, sample: Option<(Arc<SampleData>, usize)>) {
        let rate = self.sample_rate;
        self.preview = sample.map(|(data, from)| preview::Preview::new(data, from, rate));
    }

    fn set_live_lane(&mut self, channel: usize, notes: bool) {
        if let Some(Some(live)) = self.live.get_mut(channel) {
            live.notes_lane = notes;
        }
    }

    fn set_loop(&mut self, range: Option<(u64, u64)>) {
        let frame = |tick: u64| {
            beat_frame(
                tick as f64 / f64::from(PPQ),
                self.sample_rate,
                self.built_bpm,
            )
        };
        self.loop_frames = range
            .and_then(|(start, end)| frame(start).ok().zip(frame(end).ok()))
            .filter(|(start, end)| start < end);
    }

    fn all_notes_off(&mut self) {
        self.release_notes(false);
    }

    /// A sound reset (REV at the play-start point): every voice, pad, clip voice and effect silenced at once and their
    /// state cleared, the note-offs the sustain pedals hold and the live notes forgotten, nothing held or lit. Params
    /// and the transport are kept.
    fn reset_sound(&mut self) {
        self.release_notes(false);
        for (_, module) in self.instruments.iter_mut().flatten() {
            module.instrument.reset();
        }
        for kit in self.kits.iter_mut().flatten() {
            kit.sampler.reset();
        }
        for clips in self.clips.iter_mut().flatten() {
            clips.sampler.reset();
        }
        self.audio.reset_effects();
    }

    /// A module setting by its panel `#n` key ([`settings`]): an instrument's goes to `Instrument::set_setting`, an
    /// effect's to [`settings::apply`]; a drum slot's (instance −id) to its kit pad.
    fn set_setting(&mut self, channel: usize, instance_id: i32, key: u32, value: f32) {
        if !value.is_finite() {
            return;
        }
        if instance_id < 0 {
            if let Some(Some(kit)) = self.kits.get_mut(channel) {
                kit.set_slot_setting(instance_id.unsigned_abs(), key, value);
            }
        } else if let Some(module) = self.instrument(channel, instance_id) {
            module.set_setting(key, value);
        } else if let Some(effect) = self.audio.effect_mut(channel, instance_id) {
            settings::apply(effect, key, value);
        }
    }

    fn count_in(&mut self, beats: u32) {
        self.count_in_beats = beats;
    }

    /// `on` always applies; a level that is not finite keeps the last one.
    fn set_metronome(&mut self, on: bool, level: f32) {
        self.metronome_on = on;
        if level.is_finite() {
            self.metronome_level = metronome::level(level.clamp(0.0, 1.0));
        }
    }

    /// As [`Self::set_metronome`], for the flag.
    fn set_metronome_flag(&mut self, on: bool, level: f32) {
        self.metronome_flag = on;
        if level.is_finite() {
            self.metronome_level = metronome::level(level.clamp(0.0, 1.0));
        }
    }

    fn ping(&mut self, id: u32) {
        self.start_ping(id);
    }

    fn counting_in(&self) -> bool {
        self.count_in.is_some()
    }

    fn ping_report(&self) -> Option<PingReport> {
        self.ping_state()
    }

    fn set_buffer_frames(&mut self, frames: u32) {
        self.set_input_buffer(frames);
    }

    fn transport_tick(&self) -> u64 {
        ((self.frame as f64 + self.phase) * f64::from(self.built_bpm) * f64::from(PPQ)
            / (60.0 * f64::from(self.sample_rate))) as u64
    }

    fn meter_count(&self) -> usize {
        self.meter_keys.len()
    }

    fn meter_key(&self, index: usize) -> ModuleKey {
        self.meter_keys[index]
    }

    /// The peaks over the last render call.
    fn meter(&self, index: usize) -> MeterSnapshot {
        self.held_meters[index]
    }

    /// An effect's analysis after the last segment, or a note effect's after the last render call.
    /// Empty for a channel or an instrument.
    fn analysis(&self, index: usize) -> &[f32] {
        if !self.note_slabs[index].is_empty() {
            return &self.note_slabs[index];
        }
        let key = self.meter_keys[index];
        match key.instance_id {
            ..=0 => &[],
            id => (self.audio.effect(key.channel, id)).map_or(&[], |effect| effect.analysis()),
        }
    }

    /// Set by song playback (at the input, before the note effects: [`prepare::Action::Key`]) and live keys (the
    /// keyboard, previews); cleared by their note-offs, whatever the sustain pedal holds, by Stop and all notes off
    /// ([`Self::release_notes`]), and by the note-offs of a position re-sync (a seek or clip Mute while playing, the
    /// loop wrap: [`Self::release_uncovered`]), which reach only the keys the song held that no note covers: a covered
    /// key that sounds stays lit.
    fn sounding(&self, channel: usize) -> u128 {
        self.sounding.get(channel).copied().unwrap_or(0)
    }
}

impl SongAudioEngine {
    /// An export's engine ([`crate::render`]) plays on past the song end into its tail: no song-end wrap.
    pub fn play_past_song_end(&mut self) {
        self.song_beats = None;
    }

    /// Where a rolling transport wraps: the loop, else [0, song end), but for a recording (with no song loop it does
    /// not wrap at the song end) and an export.
    fn wrap_range(&self) -> Option<(u64, u64)> {
        let beats = self.song_beats.filter(|_| !self.metronome_on);
        let end = beats.and_then(|beats| beat_frame(beats, self.sample_rate, self.built_bpm).ok());
        let end = end.filter(|end| *end > 0);
        self.loop_frames.or(end.map(|end| (0, end)))
    }

    /// Stops where it is, every voice released, the params as the automation left them: an export's range end
    /// ([`crate::render`]), whose tail rings out on them. A live press queued with it plays after it, as at Play.
    pub fn halt(&mut self) {
        self.playing = false;
        self.count_in = None;
        self.release_notes(true);
        for clips in self.clips.iter_mut().flatten() {
            clips.fade_all();
        }
    }

    /// The frames at [`Self::built_bpm`] the transport moves per output frame: the live tempo over the events' tempo.
    fn speed(&self) -> f64 {
        f64::from(self.tempo_bpm) / f64::from(self.built_bpm)
    }

    /// Output frames until the transport reaches frame `at` (at the events' tempo); 0 once it has.
    fn frames_to(&self, at: u64) -> u64 {
        if self.tempo_bpm == self.built_bpm {
            return at.saturating_sub(self.frame);
        }
        let ahead = at as f64 - self.frame as f64 - self.phase;
        if ahead > 0.0 {
            (ahead / self.speed()).ceil() as u64
        } else {
            0
        }
    }

    /// Moves the transport on by `frames` output frames.
    fn advance(&mut self, frames: u64) {
        if self.tempo_bpm == self.built_bpm {
            self.frame = self.frame.saturating_add(frames);
            return;
        }
        let at = self.frame as f64 + self.phase + frames as f64 * self.speed();
        let whole = at.floor();
        (self.frame, self.phase) = (whole as u64, at - whole);
    }

    /// The song beat the transport is at.
    fn beat(&self) -> f64 {
        (self.frame as f64 + self.phase) * f64::from(self.built_bpm)
            / (60.0 * f64::from(self.sample_rate))
    }

    /// The re-sync's cover ([`cover`]) of the first `index` actions: the nearest checkpoint at or before it, and the
    /// actions after it.
    fn cover_to(&mut self, index: usize) {
        let from = self.checkpoints.restore(
            index,
            &mut self.cover,
            &mut self.cover_pads,
            &mut self.cover_keys,
        );
        for event in &self.actions[from..index] {
            let (keys, pads, input) = (&mut self.cover, &mut self.cover_pads, &mut self.cover_keys);
            hold(event, &self.instruments, keys, pads, input, false);
        }
        self.cover_events += (index - from) as u64;
    }

    /// A running count-in `(done, end, beats)` counted at `from_bpm`, at the live tempo: its beats kept, its end on its
    /// last beat (an end rounded at each retime drifts off the beats) and a frame to go at least, so it ends as
    /// `render` counts its last frame and never stands at its end (where the render loop would hang).
    fn retime_count_in(
        &self,
        (done, end, beats): (u64, u64, u32),
        from_bpm: f32,
    ) -> (u64, u64, u32) {
        let ratio = f64::from(from_bpm) / f64::from(self.tempo_bpm);
        let at = |frames: u64| (frames as f64 * ratio).round() as u64;
        let last = beat_frame(f64::from(beats), self.sample_rate, self.tempo_bpm);
        let end = last.unwrap_or(at(end)).max(1);
        (at(done).min(end - 1), end, beats)
    }

    /// The transport runs: playing, and no count-in holds it.
    fn rolling(&self) -> bool {
        self.playing && self.count_in.is_none()
    }

    /// The first beat starting at or after `frame`, counting frames at `bpm`: its index k and its frame,
    /// ceil(k·60·rate/bpm) as a note on beat k gets.
    fn beat_from(&self, frame: u64, bpm: f32) -> (u64, u64) {
        let start = |k: u64| beat_frame(k as f64, self.sample_rate, bpm).unwrap_or(u64::MAX);
        let beats = frame as f64 * f64::from(bpm) / (60.0 * f64::from(self.sample_rate));
        let mut k = beats as u64;
        while k > 0 && start(k - 1) >= frame {
            k -= 1;
        }
        while start(k) < frame {
            k += 1;
        }
        (k, start(k))
    }

    /// Starts the clicks due at this frame: a count-in beat, or a song beat while the metronome is on and the transport
    /// rolls (the accent when floor(beat) mod b = 0). The output frames to the count-in's next beat, or the transport's
    /// frame of the song's next beat (at the events' tempo, [`Self::built_bpm`]), if any.
    fn clicks(&mut self) -> (Option<u64>, Option<u64>) {
        let mut next = (None, None);
        // The count-in counts output frames at the live tempo.
        if let Some((done, ..)) = self.count_in {
            let (k, start) = self.beat_from(done, self.tempo_bpm);
            if start == done {
                self.click_beat(k);
            }
            next.0 = Some(self.beat_from(done + 1, self.tempo_bpm).1 - done);
        } else if (self.metronome_on || self.metronome_flag) && self.playing {
            // The beat the transport is on or last passed clicks as the transport reaches it, which lands within an
            // output frame of it (its frame is a stop); at a live tempo the transport's frames go by faster or slower.
            let (mut k, mut start) = self.beat_from(self.frame, self.built_bpm);
            if start > self.frame && k > 0 {
                k -= 1;
                start = beat_frame(k as f64, self.sample_rate, self.built_bpm).unwrap_or(u64::MAX);
            }
            let past = self.frame as f64 + self.phase - start as f64;
            if (0.0..self.speed()).contains(&past) {
                self.click_beat(k);
            }
            next.1 = Some(self.beat_from(self.frame + 1, self.built_bpm).1);
        }
        next
    }

    /// Clicks beat `k` (the accent when k mod b = 0) if it is past the last beat clicked ([`Self::clicked_beat`]).
    fn click_beat(&mut self, k: u64) {
        if self.clicked_beat.is_none_or(|last| k > last) {
            self.clicked_beat = Some(k);
            let accent = k.is_multiple_of(u64::from(self.bar_beats));
            self.metronome.click(accent, self.metronome_level);
        }
    }

    /// All notes off: every instrument, pad and clip voice released (a forced note-off: the sustain pedals hold none of
    /// it and forget what they held; the pedals stay as they are), the live notes forgotten, nothing held or lit.
    /// `queued`: a live press not yet sent goes out after it ([`Self::clear_live`]).
    fn release_notes(&mut self, queued: bool) {
        for (_, module) in self.instruments.iter_mut().flatten() {
            module.instrument.all_notes_off();
        }
        for kit in self.kits.iter_mut().flatten() {
            kit.sampler.all_notes_off();
        }
        for clips in self.clips.iter_mut().flatten() {
            clips.sampler.all_notes_off();
        }
        self.clear_sustained();
        // Every key goes dark, live and sequenced, and nothing is held any more: the cover of no events.
        self.sounding.fill(0);
        self.clear_live(queued);
        cover(
            &[],
            &self.instruments,
            &mut self.held,
            &mut self.held_pads,
            &mut self.held_keys,
        );
    }

    /// All notes off on `channel` only (a deleted lane's): every key it holds, from playback or live input, is
    /// released, the note-offs its sustain pedal holds are forgotten, and its audio clips' voices fast-fade.
    fn release_channel(&mut self, channel: usize) {
        if channel >= self.instruments.len() {
            return;
        }
        for (_, module) in &mut self.instruments[channel] {
            module.instrument.all_notes_off();
        }
        if let Some(kit) = &mut self.kits[channel] {
            kit.sampler.all_notes_off();
        }
        if let Some(clips) = &mut self.clips[channel] {
            clips.fade_all();
        }
        if let Some(live) = &mut self.live[channel] {
            live.clear();
        }
        let rows = (self.sustained[channel].iter_mut()).chain([&mut self.kit_sustained[channel]]);
        rows.chain(&mut self.held[channel])
            .for_each(|keys| *keys = [0; 128]);
        self.held_pads[channel].fill(0);
        (self.held_keys[channel], self.sounding[channel]) = (0, 0);
    }

    /// Forgets every note-off the sustain pedals hold.
    fn clear_sustained(&mut self) {
        let rows = self.sustained.iter_mut().flatten();
        rows.chain(&mut self.kit_sustained)
            .for_each(|keys| *keys = [0; 128]);
    }

    /// The position re-sync at the playhead, while the transport rolls: a seek, right after a loop wrap, at play start
    /// (once a count-in ends) and after a song edit that re-syncs (a clip Mute or Unmute, a note moved:
    /// [`AudioRenderer::take_over`]). A jump never re-triggers a note. The steps, on every channel:
    /// - 1: every param returns to its initial value (the saved or live one), but a MIDI CC (7–133, the pedal's too)
    ///   keeps its value and a live Pitch Bend or Aftertouch is no initial value;
    /// - 3: each key or pad the sequencer holds that no unmuted note covers gets its note-off
    ///   ([`Self::release_uncovered`]), meeting the instruments and the drum slots at their initial values and the
    ///   sustain pedal as it stands; a covered key that sounds sounds on untouched, and a covered key that does not
    ///   stays silent: a note starts only when the playhead crosses its start. Covered is what playback up to the beat
    ///   would hold ([`cover`]), so in Preview only the previewed clip covers (the engine plays it alone,
    ///   `ui::preview::solo`);
    /// - 4: every param takes its automation at the beat: the last point at or before it, past a clip's end its last
    ///   point ([`Self::restore_params`]);
    /// - 2 and 5: the audio clips' seek rule ([`Self::relaunch_clips`]): a channel whose first lane is audio cuts every
    ///   clip voice at once, then each unmuted clip over the playhead restarts at its offset.
    ///
    /// The scheduled end of a covered key: it ends at the next note-off the sequencer plays on it from the new beat,
    /// The covering note's end; the old note's own end is gone with the jump. steps 1 and 4 set each param
    /// once, to its final value (a param set twice in a frame acts twice: a Pitcher 2 Low Latency switch rebuilds),
    /// except what the note-offs read (instruments, drum slots), which takes its initial value before step 3 when
    /// anything is held ([`Self::note_off_params_to_initial`]). Allocates nothing. A re-sync that finds something held
    /// (a seek while playing, an edit) replays the events from the nearest checkpoint before the playhead, at most
    /// [`CHECKPOINT_SPAN`]; the loop wrap and play start hold nothing and skip it.
    fn resync(&mut self) {
        let frame = self.frame;
        self.action_index = self.actions.partition_point(|event| event.frame < frame);
        if self.holds_any() {
            self.note_off_params_to_initial();
            self.cover_replays += 1;
            self.cover_to(self.action_index);
            self.release_uncovered(true);
        }
        self.restore_params(frame, true);
        self.relaunch_clips();
    }

    /// Step 1 of the re-sync for what the note-offs of step 3 read: every instrument's params (a MiniSynth latches its
    /// release rates at the note-off; mono and lead modes) and every drum slot's (a pad's release) back to their
    /// initial value, before the automation of step 4 moves them. The sustain pedal is a MIDI CC, which step 1 keeps:
    /// The note-offs meet it as it stands. The other params (effects, the strip) are set once, at their final value
    /// ([`Self::restore_params`]).
    fn note_off_params_to_initial(&mut self) {
        for position in 0..self.initial.len() {
            let (channel, instance_id, index, value) = self.initial[position];
            let instrument = self.instruments[channel]
                .iter()
                .any(|(id, _)| *id == instance_id);
            if instrument || instance_id < 0 {
                self.apply_param(channel, instance_id, index, value);
            }
        }
    }

    /// The sequencer holds a key or a pad.
    fn holds_any(&self) -> bool {
        self.held_keys.iter().any(|&keys| keys != 0)
            || (self.held.iter().flatten()).any(|row| row.iter().any(|&count| count > 0))
            || self.held_pads.iter().flatten().any(|&count| count > 0)
    }

    /// All notes off from the sequencer: a forced note-off for each key and pad it holds (the loop wrap).
    fn release_held(&mut self) {
        let (keys, pads, input) = (&mut self.cover, &mut self.cover_pads, &mut self.cover_keys);
        cover(&[], &self.instruments, keys, pads, input);
        self.release_uncovered(false);
    }

    /// Each key and pad the sequencer holds that the re-sync's cover does not cover gets its note-off (through the
    /// sustain pedal, [`Self::key_up`]; the loop wrap's, `!now`, is forced) and goes dark; a covered one is held on,
    /// now by the note that covers it, whose note-off ends it. `now`: a re-sync at the playhead, where a note or held
    /// hit that starts exactly there covers its key too (start ≤ new beat): no note-off before it, and playback then
    /// plays it; its own note-on and note-off count it, so the old hold ends here. Caveat: an instrument whose note-off
    /// releases one press of a key (the DW Sampler in Mono, which goes back to the key's earlier press still held)
    /// keeps the old voice past that note's note-off, until the key's next release; in Poly a note-off releases every
    /// voice of its key.
    fn release_uncovered(&mut self, now: bool) {
        for channel in 0..self.held.len() {
            let mut uncovered = self.held_keys[channel] & !self.cover_keys[channel];
            for key in 0..=127 {
                let starts =
                    |action| matches!(action, Action::Key { key: k, on: true } if k == key);
                if uncovered >> key & 1 == 1 && now && self.starts_now(channel, starts) {
                    uncovered &= !(1 << key);
                }
            }
            self.sounding[channel] &= !uncovered;
            self.held_keys[channel] &= !uncovered;
            for row in 0..self.held[channel].len() {
                let id = self.instruments[channel]
                    .get(row)
                    .map_or(KIT, |(id, _)| *id);
                for key in 0..=127 {
                    let k = usize::from(key);
                    let (held, covered) = (self.held[channel][row][k], self.cover[channel][row][k]);
                    let starts = |action| match action {
                        Action::NoteOn {
                            instance_id,
                            key: k,
                            ..
                        } => instance_id == id && k == key,
                        _ => false,
                    };
                    if held > 0 && covered == 0 && !(now && self.starts_now(channel, starts)) {
                        for _ in 0..held {
                            match now {
                                true => self.key_up(channel, Some(id), key),
                                false => self.force_off(channel, id, key),
                            }
                        }
                    }
                    self.held[channel][row][k] = held.min(covered);
                }
            }
            for pad in 0..self.held_pads[channel].len() {
                let (held, covered) = (self.held_pads[channel][pad], self.cover_pads[channel][pad]);
                let starts = |action| {
                    matches!(action, Action::Hit { pad: p, held: true, .. }
                        | Action::StepCover { pad: p, on: true } if p == pad)
                };
                if held > 0
                    && covered == 0
                    && !(now && self.starts_now(channel, starts))
                    && let Some(kit) = self.kit(channel)
                {
                    kit.release(pad);
                }
                self.held_pads[channel][pad] = held.min(covered);
            }
        }
    }

    /// An action of `channel` exactly at the playhead, still to play, matches `of`.
    fn starts_now(&self, channel: usize, of: impl Fn(Action) -> bool) -> bool {
        let frame = self.frame;
        (self.actions[self.action_index..].iter())
            .take_while(|event| event.frame == frame)
            .any(|event| event.channel == channel && of(event.action))
    }

    /// The audio clips' seek rule at the playhead: a locate, Play or loop wrap while playing.
    fn relaunch_clips(&mut self) {
        let beat = self.beat();
        // Their samples and spans are rendered and timed at the build's tempo.
        for clips in self.clips.iter_mut().flatten() {
            clips.relaunch(self.frame, beat, self.built_bpm);
        }
    }

    /// Every param as it stands at `frame`, set once to its final value: its automation up to `frame`, else its saved
    /// or live value. Setting it twice in one frame would act twice (a Pitcher 2 Low Latency switch rebuilds). A param
    /// in `initial` is set only where the module holding it has another value (`applied`), as an edit's are: a loop
    /// wrap sets back only what its pass moved, not some 10 000 params. `resync`: re-sync step 1's scope, where a MIDI
    /// CC no automation sets keeps its value.
    fn restore_params(&mut self, frame: u64, resync: bool) {
        let key = |&(channel, instance_id, index, _): &(usize, i32, i32, f32)| {
            (channel, instance_id, index)
        };
        for position in 0..self.initial.len() {
            let (channel, instance_id, index, saved) = self.initial[position];
            let value = self.automated((channel, instance_id, index), frame);
            if resync && value.is_none() && midi_cc(instance_id, index) {
                continue;
            }
            let value = value.unwrap_or(saved);
            if self.applied[position] != value {
                self.apply_param(channel, instance_id, index, value);
            }
        }
        for position in 0..self.param_timelines.len() {
            let timeline = &self.param_timelines[position];
            let param = (timeline.channel, timeline.instance_id, timeline.index);
            if self.initial.binary_search_by_key(&param, key).is_err()
                && let Some(value) = self.automated(param, frame)
            {
                self.apply_param(param.0, param.1, param.2, value);
            }
        }
    }

    /// Param `(channel, instance_id, index)`'s automated value at `frame`: its timeline's last change up to it.
    fn automated(&self, param: (usize, i32, i32), frame: u64) -> Option<f32> {
        let position = (self.param_timelines)
            .binary_search_by_key(&param, |t| (t.channel, t.instance_id, t.index))
            .ok()?;
        let changes = &self.param_timelines[position].changes;
        let count = changes.partition_point(|(at, _)| *at <= frame);
        count.checked_sub(1).map(|last| changes[last].1)
    }

    /// Folds the segment just rendered into [`Self::held_meters`].
    fn hold_meters(&mut self) {
        for index in 0..self.meter_keys.len() {
            let live = self.live_meter(index);
            self.held_meters[index].hold(live);
        }
    }

    /// Meter `index` as the last segment left it.
    fn live_meter(&self, index: usize) -> MeterSnapshot {
        let key = self.meter_keys[index];
        let mut instruments = self.instruments[key.channel].iter();
        if key.instance_id == 0 {
            self.channel_meters[key.channel]
        } else if key.instance_id < 0 {
            let kit = self.kits[key.channel].as_ref();
            let [peak_left, peak_right] =
                kit.map_or([0.0; 2], |kit| kit.peak(key.instance_id.unsigned_abs()));
            MeterSnapshot {
                peak_left,
                peak_right,
                ..MeterSnapshot::default()
            }
        } else if let Some((_, module)) = instruments.find(|(id, _)| *id == key.instance_id) {
            module.meter()
        } else {
            self.audio
                .effect(key.channel, key.instance_id)
                .map(|effect| effect.meter())
                .unwrap_or_default()
        }
    }

    /// A playing transport at or past the loop end, or with no loop the song end ([`Self::wrap_range`]), goes on from
    /// its start (the song end's wrap is the loop's): a forced note-off for each key the sequencer holds (a pedal down
    /// holds none; tails ring on, nothing is reset; a live key sounds on), the audio clips' notes released, then the
    /// position re-sync at the start, which finds nothing left to release: a note spanning the start stays silent, one
    /// starting on it plays. At a live tempo the transport goes past the end by up to an output frame's worth of
    /// frames, which it carries on from the start; the actions it passes there play next. A transport further past the
    /// end (a seek, a loop moved before it) starts at the start.
    fn wrap_at_loop_end(&mut self) {
        let Some((start, end)) = self.wrap_range() else {
            return;
        };
        if self.frame < end {
            return;
        }
        let over = (self.frame - end) as f64 + self.phase;
        self.release_held();
        for clips in self.clips.iter_mut().flatten() {
            clips.sampler.all_notes_off();
        }
        (self.frame, self.phase, self.clicked_beat) = (start, 0.0, None);
        self.resync();
        if over < self.speed() {
            let at = start as f64 + over;
            (self.frame, self.phase) = (at as u64, at.fract());
        }
    }

    /// Every note-off of `channel`, sequenced or live, goes through here: to its instrument `instance_id` or its pad
    /// sampler ([`KIT`]), or to all of them for `None` (a live key). While its sustain pedal is down (CC #64 > 0.5) the
    /// note-off is held, counted per instrument (and for the pads) and key, until the pedal lifts. An instrument
    /// outside whose key range the key is, or that is learning one, never gets it.
    fn key_up(&mut self, channel: usize, instance_id: Option<i32>, key: u8) {
        let pedal = self.audio.channel_parameter(channel, SUSTAIN_PEDAL);
        let pedal = pedal.is_some_and(|pedal| pedal > 0.5);
        if instance_id.is_none_or(|id| id == KIT)
            && let (Some(Some(kit)), Some(held)) = (
                self.kits.get_mut(channel),
                self.kit_sustained.get_mut(channel),
            )
        {
            match held.get_mut(usize::from(key)) {
                Some(count) if pedal => *count = count.saturating_add(1),
                _ => kit.sampler.note_off(key),
            }
        }
        let (Some(modules), Some(sustained)) = (
            self.instruments.get_mut(channel),
            self.sustained.get_mut(channel),
        ) else {
            return;
        };
        for ((id, module), held) in modules.iter_mut().zip(sustained) {
            if instance_id.is_some_and(|target| target != *id) || !module.hears(key) {
                continue;
            }
            match held.get_mut(usize::from(key)) {
                Some(count) if pedal => *count = count.saturating_add(1),
                _ => module.instrument.note_off(key),
            }
        }
    }

    /// A forced note-off of `key` to `channel`'s instrument `instance_id` or its pads ([`KIT`]), which the sustain
    /// pedal never holds (the loop wrap's).
    fn force_off(&mut self, channel: usize, instance_id: i32, key: u8) {
        if instance_id == KIT {
            if let Some(kit) = self.kit(channel) {
                kit.note_off(key);
            }
        } else if let Some(module) = self.instrument(channel, instance_id)
            && module.hears(key)
        {
            module.instrument.note_off(key);
        }
    }

    /// The pedal of `channel` lifted: sends every held note-off to its instrument or the pads, as many times as each
    /// key was released, so an instrument that releases one voice per note-off releases them all.
    fn lift_pedal(&mut self, channel: usize) {
        if let (Some(Some(kit)), Some(held)) = (
            self.kits.get_mut(channel),
            self.kit_sustained.get_mut(channel),
        ) {
            for key in 0..=127 {
                if std::mem::take(&mut held[usize::from(key)]) > 0 {
                    kit.sampler.note_off(key);
                }
            }
        }
        let (Some(modules), Some(sustained)) = (
            self.instruments.get_mut(channel),
            self.sustained.get_mut(channel),
        ) else {
            return;
        };
        for ((_, module), held) in modules.iter_mut().zip(sustained) {
            for key in 0..=127 {
                for _ in 0..std::mem::take(&mut held[usize::from(key)]) {
                    module.instrument.note_off(key);
                }
            }
        }
    }
}

/// The actions between two of the cover's checkpoints: a re-sync replays at most this many.
const CHECKPOINT_SPAN: usize = 4096;

/// What [`cover`] leaves after every [`CHECKPOINT_SPAN`] actions, worked out at build (on the UI thread): checkpoint i
/// holds the cover of the first (i + 1)·[`CHECKPOINT_SPAN`] actions, flattened as the engine's `held`, `held_pads` and
/// `held_keys` are laid out.
#[derive(Default)]
struct Checkpoints {
    keys: Vec<[u8; 128]>,
    pads: Vec<u8>,
    input: Vec<u128>,
}

impl Checkpoints {
    /// The checkpoints of `actions`, for a cover shaped as `keys` and `pads` (one input row per channel).
    fn new(
        actions: &[ScheduledAction],
        instruments: &[Vec<(i32, InstrumentModule)>],
        keys: &[Vec<[u8; 128]>],
        pads: &[Vec<u8>],
    ) -> Self {
        let (mut keys, mut pads) = (keys.to_vec(), pads.to_vec());
        let mut input = vec![0; keys.len()];
        cover(&[], instruments, &mut keys, &mut pads, &mut input);
        let mut checkpoints = Self::default();
        for span in actions.as_chunks::<CHECKPOINT_SPAN>().0 {
            for event in span {
                hold(event, instruments, &mut keys, &mut pads, &mut input, false);
            }
            checkpoints.keys.extend(keys.iter().flatten());
            checkpoints.pads.extend(pads.iter().flatten());
            checkpoints.input.extend(&input);
        }
        checkpoints
    }

    /// Sets `keys`, `pads` and `input` to the cover at the nearest checkpoint at or before action `index` (none held
    /// before the first), and returns the actions it covers.
    fn restore(
        &self,
        index: usize,
        keys: &mut [Vec<[u8; 128]>],
        pads: &mut [Vec<u8>],
        input: &mut [u128],
    ) -> usize {
        let Some(at) = (index / CHECKPOINT_SPAN).checked_sub(1) else {
            keys.iter_mut().flatten().for_each(|row| *row = [0; 128]);
            pads.iter_mut().flatten().for_each(|count| *count = 0);
            input.fill(0);
            return 0;
        };
        let width = keys.iter().map(Vec::len).sum::<usize>();
        let rows = keys.iter_mut().flatten();
        rows.zip(&self.keys[at * width..])
            .for_each(|(row, saved)| *row = *saved);
        let width = pads.iter().map(Vec::len).sum::<usize>();
        let counts = pads.iter_mut().flatten();
        counts
            .zip(&self.pads[at * width..])
            .for_each(|(count, saved)| *count = *saved);
        input.copy_from_slice(&self.input[at * input.len()..(at + 1) * input.len()]);
        (at + 1) * CHECKPOINT_SPAN
    }
}

/// What `actions` (applied in order from nothing held) cover: [`hold`] as a re-sync reads it.
fn cover(
    actions: &[ScheduledAction],
    instruments: &[Vec<(i32, InstrumentModule)>],
    keys: &mut [Vec<[u8; 128]>],
    pads: &mut [Vec<u8>],
    input: &mut [u128],
) {
    keys.iter_mut().flatten().for_each(|row| *row = [0; 128]);
    pads.iter_mut().flatten().for_each(|count| *count = 0);
    input.fill(0);
    for event in actions {
        hold(event, instruments, keys, pads, input, false);
    }
}

/// What `event` leaves held on its channel: per instrument row (the position in `instruments`, then the pads' notes,
/// [`KIT`]) and key in `keys` the note-ons not yet matched by a note-off, per pad in `pads` the hits not yet released,
/// and in `input` the keys at the channel's input (the last event of each). `played`: the sequencer's own hold, where a
/// hit without a note-off (a step lane's, a length-0 or Repeat hit) holds its pad until something releases it.
/// Otherwise a re-sync's cover: a step lane's earlier hit covers its pad to its clip's end ([`Action::StepCover`]), and
/// other hits without a note-off cover nothing.
fn hold(
    event: &ScheduledAction,
    instruments: &[Vec<(i32, InstrumentModule)>],
    keys: &mut [Vec<[u8; 128]>],
    pads: &mut [Vec<u8>],
    input: &mut [u128],
    played: bool,
) {
    let channel = event.channel;
    let row = |id: i32| match id {
        KIT => Some(instruments[channel].len()),
        id => (instruments[channel].iter()).position(|(other, _)| *other == id),
    };
    let up: fn(u8) -> u8 = |n| n.saturating_add(1);
    let down: fn(u8) -> u8 = |n| n.saturating_sub(1);
    let at_least_one: fn(u8) -> u8 = |n| n.max(1);
    let (count, change) = match (event.action, played) {
        (Action::Key { key, on }, _) => {
            set_key(&mut input[channel], key, on);
            return;
        }
        (
            Action::NoteOn {
                instance_id, key, ..
            },
            _,
        ) => (
            row(instance_id).and_then(|r| keys[channel][r].get_mut(usize::from(key))),
            up,
        ),
        (Action::NoteOff { instance_id, key }, _) => (
            row(instance_id).and_then(|r| keys[channel][r].get_mut(usize::from(key))),
            down,
        ),
        (
            Action::Hit {
                pad, held: true, ..
            },
            _,
        ) => (pads[channel].get_mut(pad), up),
        (Action::Hit { pad, .. }, true) => (pads[channel].get_mut(pad), at_least_one),
        (Action::PadOff { pad }, _) => (pads[channel].get_mut(pad), down),
        (Action::StepCover { pad, on }, false) => {
            (pads[channel].get_mut(pad), if on { up } else { down })
        }
        _ => return,
    };
    if let Some(count) = count {
        *count = change(*count);
    }
}

/// Where param `key` (channel, instance id, index) is in `initial`, which is sorted by param.
fn position(initial: &[(usize, i32, i32, f32)], key: (usize, i32, i32)) -> Option<usize> {
    initial
        .binary_search_by_key(&key, |&(channel, instance_id, index, _)| {
            (channel, instance_id, index)
        })
        .ok()
}

/// Key `key` of `keys` (bit k = MIDI k) set (`on`) or cleared; a key past 127 is none.
fn set_key(keys: &mut u128, key: u8, on: bool) {
    let bit = 1_u128.checked_shl(u32::from(key)).unwrap_or(0);
    if on {
        *keys |= bit;
    } else {
        *keys &= !bit;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{ChannelKind, Event, SynthKind};

    /// The RT cost of a jump: the loop wrap releases what the sequencer holds before its re-sync, so that re-sync
    /// replays no events, however many wraps; a seek over a held note does replay them.
    #[test]
    fn only_a_re_sync_that_finds_something_held_replays_the_events() {
        let mut song = Song {
            tempo_bpm: 120.0,
            play_loop: Some((0, 2 * PPQ)),
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let clip = song.channels[1].lanes[0].add_clip(0, 4.0);
        for (beat, len_beats, key) in [(0.0, 1.5, 60), (0.5, 2.5, 64)] {
            clip.events.push(Event::Note {
                pos: (beat * f64::from(PPQ)) as u32,
                len_beats,
                key,
                velocity: Event::DEFAULT_VELOCITY,
                pan: Event::DEFAULT_PAN,
                slide: false,
            });
        }
        let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
        engine.play();
        let mut output = vec![0.0; 1000 * 2];
        // Five beats: two wraps, each over the held 64.
        for _ in 0..120 {
            engine.render(&mut output, 2);
        }
        assert!(engine.holds_any());
        assert_eq!(engine.cover_replays, 0);
        engine.seek(u64::from(PPQ));
        assert_eq!(engine.cover_replays, 1);
    }

    /// A re-sync's cover replays at most one checkpoint span of actions, and is what replaying every action up to the
    /// playhead would give. 5 000 notes are 20 000 actions (each note's key and instrument on and off): four
    /// checkpoints. A long low note holds through them all.
    #[test]
    fn a_re_sync_replays_at_most_one_checkpoint_span() {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let clip = song.channels[1].lanes[0].add_clip(0, 5000.0);
        clip.events.extend((0..5000).map(|beat| Event::Note {
            pos: beat * PPQ,
            len_beats: 0.5,
            key: 60 + (beat % 12) as u8,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        }));
        clip.events.insert(
            0,
            Event::Note {
                pos: 0,
                len_beats: 4999.0,
                key: 40,
                velocity: Event::DEFAULT_VELOCITY,
                pan: Event::DEFAULT_PAN,
                slide: false,
            },
        );
        let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
        assert!(engine.actions.len() > 4 * CHECKPOINT_SPAN);
        for index in [
            0,
            1,
            CHECKPOINT_SPAN - 1,
            CHECKPOINT_SPAN,
            3 * CHECKPOINT_SPAN + 7,
        ] {
            engine.cover_to(index);
            let mut whole = (
                engine.cover.clone(),
                engine.cover_pads.clone(),
                engine.cover_keys.clone(),
            );
            cover(
                &engine.actions[..index],
                &engine.instruments,
                &mut whole.0,
                &mut whole.1,
                &mut whole.2,
            );
            assert!(
                whole
                    == (
                        engine.cover.clone(),
                        engine.cover_pads.clone(),
                        engine.cover_keys.clone()
                    ),
                "{index}"
            );
        }
        engine.play();
        engine.render(&mut vec![0.0; 4800 * 2], 2);
        let before = engine.cover_events;
        engine.seek(4900 * u64::from(PPQ) + 32);
        assert_eq!(engine.cover_replays, 1);
        let replayed = engine.cover_events - before;
        assert!(
            (1..=CHECKPOINT_SPAN as u64).contains(&replayed),
            "{replayed}"
        );
        assert_eq!(
            engine.sounding(1),
            1 << 40,
            "the low note covers the playhead"
        );
    }

    /// A live tempo, slower or faster than the build's: the metronome clicks once on each beat, on its frame at that
    /// tempo (the click's first sample is its loudest, 0.631 × level).
    #[test]
    fn the_metronome_clicks_on_each_beat_at_a_live_tempo() {
        for bpm in [90.0, 180.0, 250.0] {
            let mut engine = SongAudioEngine::new(&Song::default(), 48_000.0).unwrap();
            engine.set_metronome(true, 1.0);
            engine.set_tempo(bpm);
            engine.play();
            let per_beat = 48_000.0 * 60.0 / f64::from(bpm);
            let mut output = vec![0.0; ((4.0 * per_beat) as usize + 10) * 2];
            engine.render(&mut output, 2);
            let onsets: Vec<_> = (output.iter().step_by(2).enumerate())
                .filter(|(_, s)| s.abs() >= 0.631 * 0.9995)
                .map(|(frame, _)| frame)
                .collect();
            assert_eq!(onsets.len(), 5, "{bpm}: {onsets:?}");
            for (k, frame) in onsets.into_iter().enumerate() {
                assert!(
                    (frame as f64 - k as f64 * per_beat).abs() <= 1.0,
                    "{bpm}: beat {k} at {frame}"
                );
            }
        }
    }

    /// An edit's engine takes the new song's initial params (a CC lane brings its target's rest at 0, and takes it away
    /// again). A value sent live stays the engine's only while the song holds it too (the app writes both); a song with
    /// another value (an undo) is heard.
    #[test]
    fn an_edit_taken_over_takes_the_new_songs_initial_params() {
        use crate::song::AutomationTarget;
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let saved = song.channels[1].params.0[0];
        let mut cc = song.clone();
        // MIDI CC #1, channel param 8.
        cc.channels[1].add_automation_lane(AutomationTarget {
            instance_id: 0,
            param: 8,
        });
        let initial = |engine: &SongAudioEngine, param: i32| {
            let entry = engine
                .initial
                .iter()
                .find(|e| (e.0, e.1, e.2) == (1, 0, param));
            entry.map(|e| e.3)
        };
        let mut old = SongAudioEngine::new(&song, 48_000.0).unwrap();
        old.set_param(1, 0, 0, 0.3);
        let mut with_cc = SongAudioEngine::new(&cc, 48_000.0).unwrap().adopting();
        with_cc.take_over(&mut old);
        assert_eq!(
            (initial(&with_cc, 8), initial(&with_cc, 0)),
            (Some(0.0), Some(saved))
        );
        with_cc.set_param(1, 0, 0, 0.3);
        song.channels[1].params.0[0] = 0.3;
        let mut without = SongAudioEngine::new(&song, 48_000.0).unwrap().adopting();
        without.take_over(&mut with_cc);
        assert_eq!(
            (initial(&without, 8), initial(&without, 0)),
            (None, Some(0.3))
        );
    }

    /// MiniSynth on channel 1 under note effect `type_id` (at the top of its rack), with a note on MIDI 60 for `beats`
    /// beats from beat 0; played until `found` holds after a render of 64 frames (at most 4 beats), then sought while
    /// playing to beat 20, where nothing covers. The engine after the seek.
    fn sought_once(
        type_id: u16,
        beats: f64,
        found: impl Fn(&SongAudioEngine) -> bool,
    ) -> SongAudioEngine {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        song.channels[1].rack.insert(0, type_id);
        song.channels[1].lanes[0]
            .add_clip(0, 8.0)
            .events
            .push(Event::Note {
                pos: 0,
                len_beats: beats,
                key: 60,
                velocity: Event::DEFAULT_VELOCITY,
                pan: Event::DEFAULT_PAN,
                slide: false,
            });
        let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
        engine.play();
        let mut output = vec![0.0; 64 * 2];
        let mut renders = 0;
        while !found(&engine) {
            engine.render(&mut output, 2);
            renders += 1;
            assert!(renders < 4 * 24_000 / 64, "never found");
        }
        engine.seek(20 * u64::from(PPQ));
        engine
    }

    /// No instrument row holds a key on channel 1.
    fn rows_free(engine: &SongAudioEngine) -> bool {
        engine.held[1]
            .iter()
            .all(|row| row.iter().all(|&count| count == 0))
    }

    /// An Arpeggiator between two steps: the sequencer holds the input key and no instrument key. The re-sync still
    /// releases it, and its key goes dark.
    #[test]
    fn a_re_sync_releases_an_input_key_an_arpeggiator_holds_between_steps() {
        let engine = sought_once(51, 4.0, |e| e.held_keys[1] != 0 && rows_free(e));
        assert_eq!((engine.held_keys[1], engine.sounding(1)), (0, 0));
    }

    /// At a live tempo the transport moves by fractions of a frame, and a loop wrap carries the part it went past the
    /// loop end: after any number of wraps the transport stands where the output frames at that speed put it. A note on
    /// The loop start still plays after each wrap.
    #[test]
    fn a_loop_wrap_at_a_live_tempo_keeps_the_overshoot() {
        let mut song = Song {
            tempo_bpm: 120.0,
            play_loop: Some((0, PPQ)),
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let clip = song.channels[1].lanes[0].add_clip(0, 4.0);
        clip.events.push(Event::Note {
            pos: 0,
            len_beats: 0.5,
            key: 60,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        });
        let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
        engine.play();
        engine.set_tempo(137.0);
        let speed = 137.0 / 120.0;
        let mut output = vec![0.0; 1000 * 2];
        for _ in 0..110 {
            engine.render(&mut output, 2);
        }
        // 110 000 output frames are 125 583.3 at the build's tempo: five wraps of the 24 000-frame loop.
        let expected = (110_000.0 * speed) % 24_000.0;
        let at = engine.frame as f64 + engine.phase;
        assert!((at - expected).abs() < 1e-6, "{at} vs {expected}");
        assert_eq!(engine.sounding(1), 1 << 60);
    }

    /// A Note Echo's tail: an echo holds an instrument key after the input key is off. The re-sync releases it.
    #[test]
    fn a_re_sync_releases_a_note_echo_tail() {
        let engine = sought_once(47, 0.25, |e| e.held_keys[1] == 0 && !rows_free(e));
        assert!(rows_free(&engine));
    }

    /// No pass of the render loop renders nothing, so none can repeat: a count-in standing at its end (which a tempo
    /// change no longer leaves) ends as the next frame renders instead of holding the audio thread.
    #[test]
    fn a_count_in_at_its_end_cannot_hold_the_render_loop() {
        let mut engine = SongAudioEngine::new(&Song::default(), 48_000.0).unwrap();
        engine.play();
        engine.count_in = Some((100, 100, 1));
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            engine.render(&mut [0.0; 512 * 2], 2);
            let _ = send.send(engine);
        });
        let timeout = std::time::Duration::from_secs(10);
        let engine = receive
            .recv_timeout(timeout)
            .expect("render never returned");
        assert!(!engine.counting_in());
    }

    /// A retime 1 frame after a beat (0.4 frame at 300 BPM), by a tempo drag or by an edit's engine built at 300 BPM
    /// taking over, never clicks that beat again, in a count-in or from the metronome: one click per beat.
    #[test]
    fn a_retime_just_after_a_beat_clicks_each_beat_once() {
        for (count_in, take_over) in [(4, false), (0, false), (4, true), (0, true)] {
            let case = format!("count-in {count_in}, take-over {take_over}");
            let mut engine = clicking(&Song::default());
            engine.count_in(count_in);
            engine.play();
            assert_eq!(clicks(&mut engine, 24_001), [0, 24_000], "{case}");
            if take_over {
                let mut faster = Song::default();
                faster.set_tempo(300.0);
                let mut new = SongAudioEngine::new(&faster, 48_000.0).unwrap();
                new = new.adopting_edit(false, Vec::new());
                new.take_over(&mut engine);
                engine = new;
            } else {
                engine.set_tempo(300.0);
            }
            // Beats 2 and 3 (of the count-in, or the song's), then the song's beat 0 or its beats 4 and 5.
            let after = clicks(&mut engine, 4 * 9_600 + 10);
            assert_eq!(after, [9_600, 19_200, 28_800, 38_400], "{case}");
        }
    }

    /// The frames, into `frames` rendered, where a click starts (its first sample is its loudest, 0.631 × level).
    fn clicks(engine: &mut SongAudioEngine, frames: usize) -> Vec<usize> {
        let mut output = vec![0.0; frames * 2];
        engine.render(&mut output, 2);
        (output.iter().step_by(2).enumerate())
            .filter(|(_, s)| s.abs() >= 0.631 * 0.9995)
            .map(|(frame, _)| frame)
            .collect()
    }

    /// An engine for `song` with the metronome on at full level.
    fn clicking(song: &Song) -> SongAudioEngine {
        let mut engine = SongAudioEngine::new(song, 48_000.0).unwrap();
        engine.set_metronome(true, 1.0);
        engine
    }

    /// A seek forgets the last beat clicked, so the beats behind it click again.
    #[test]
    fn a_seek_back_clicks_its_beats_again() {
        let mut engine = clicking(&Song::default());
        engine.play();
        clicks(&mut engine, 6 * 24_000 + 10);
        engine.seek(2 * u64::from(PPQ));
        assert_eq!(clicks(&mut engine, 24_001), [0, 24_000]);
    }

    /// A loop wrap forgets the last beat clicked, so the loop's beats click on every pass.
    #[test]
    fn a_loop_wrap_clicks_the_loops_beats_again() {
        let song = Song {
            play_loop: Some((0, 2 * PPQ)),
            ..Song::default()
        };
        let mut engine = clicking(&song);
        engine.play();
        let expected = [0, 24_000, 48_000, 72_000, 96_000];
        assert_eq!(clicks(&mut engine, 4 * 24_000 + 1), expected);
    }

    /// The metronome flag clicks the beats with REC off, and the transport still wraps at the song end (one bar).
    #[test]
    fn the_metronome_flag_clicks_without_rec_and_keeps_the_song_end_wrap() {
        let mut engine = SongAudioEngine::new(&Song::default(), 48_000.0).unwrap();
        engine.play();
        assert!(clicks(&mut engine, 24_000).is_empty());
        engine.set_metronome_flag(true, 1.0);
        // Beats 1 to 3, then beat 0 again past the wrap: 5 beats in, the transport is 1 beat into the bar.
        assert_eq!(clicks(&mut engine, 4 * 24_000), [0, 24_000, 48_000, 72_000]);
        assert_eq!(engine.transport_tick(), u64::from(PPQ));
    }

    /// Play forgets the last beat clicked, so a count-in after a Stop clicks its beats.
    #[test]
    fn a_count_in_after_a_pause_clicks() {
        let mut engine = clicking(&Song::default());
        engine.play();
        clicks(&mut engine, 5 * 24_000 + 10);
        engine.stop();
        engine.count_in(2);
        engine.play();
        assert_eq!(clicks(&mut engine, 2 * 24_000 - 10), [0, 24_000]);
    }

    /// A count-in retimed again and again keeps its end on its last beat at the live tempo, so its beats and the song's
    /// beat 0 click once each, a beat apart (each retime rounding the end on its own drifted it 2 frames past that beat
    /// at 120, 173, 223 then 70 BPM: the beat clicked, then the song's beat 0 2 frames on).
    #[test]
    fn a_count_in_retimed_again_and_again_clicks_once_at_its_end() {
        let mut engine = clicking(&Song::default());
        engine.count_in(4);
        engine.play();
        for bpm in [173.0, 223.0, 70.0] {
            clicks(&mut engine, 1_000);
            engine.set_tempo(bpm);
        }
        // Count-in beats 1 to 3, then the song's beat 0 as it ends.
        let onsets = clicks(&mut engine, 4 * 41_143);
        assert_eq!(onsets.len(), 4, "{onsets:?}");
        let per_beat = 48_000.0 * 60.0 / 70.0;
        for pair in onsets.windows(2) {
            let apart = (pair[1] - pair[0]) as f64;
            assert!((apart - per_beat).abs() <= 1.0, "{onsets:?}");
        }
    }

    /// Play start: a channel whose first lane is audio has every clip slot fast-faded (c = 0.01 per sample). Stopped,
    /// nothing starts a clip voice, so one is struck here by hand; with a count-in no re-sync cuts it at Play, and it
    /// is silent 1500 frames in.
    #[test]
    fn play_fast_fades_the_clip_slots_of_an_audio_channel() {
        use crate::flm::chunk::ContentPath;
        use crate::song::SamplerSlot;
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::AudioClip);
        let mut slot = SamplerSlot::default();
        slot.file.path = Some(ContentPath::Plain("dc".into()));
        song.channels[1].lanes[0].add_clip(0, 8.0).file.sample = Some(Box::new(slot));
        let mut cache = SampleCache::default();
        let dc = crate::sample_cache::tests::wav(&[16_000; 88_200]);
        cache.load(&song, &|_: &ContentPath| Some(dc.clone()));
        let mut engine = SongAudioEngine::with_samples(&song, 48_000.0, &cache).unwrap();
        let meter = (engine.meter_keys.iter())
            .position(|k| (k.channel, k.instance_id) == (1, 0))
            .unwrap();
        engine.clips[1]
            .as_mut()
            .unwrap()
            .sampler
            .hit_at(0, 1.0, 0.5, 1.0, 0.0);
        let mut output = vec![0.0; 1500 * 2];
        engine.render(&mut output, 2);
        assert!(engine.meter(meter).peak_left > 0.1, "the clip voice sounds");
        engine.count_in(4);
        engine.play();
        engine.render(&mut output, 2);
        engine.render(&mut output[..500 * 2], 2);
        assert!(engine.meter(meter).peak_left < 1e-5, "faded");
    }

    /// Song with a MiniSynth on channel 1 whose MIDI CC #1 (channel param 8) is automated to 1 over beats 2 to 3.
    fn cc_song() -> Song {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let lane = song.channels[1].add_automation_lane(crate::song::AutomationTarget {
            instance_id: 0,
            param: 8,
        });
        lane.add_clip(2 * PPQ, 1.0).events.push(Event::Automation {
            pos: 0,
            value: 1.0,
            control: true,
        });
        song
    }

    /// Channel 1's params `indices` as the engine holds them.
    fn params<const N: usize>(engine: &SongAudioEngine, indices: [usize; N]) -> [Option<f32>; N] {
        indices.map(|index| engine.audio.channel_parameter(1, index))
    }

    /// Played from beat 2.5, a live Level and pedal set, and MIDI's Pitch Bend and Aftertouch, then `jump`.
    fn jumped(jump: impl FnOnce(&mut SongAudioEngine)) -> SongAudioEngine {
        let mut engine = SongAudioEngine::new(&cc_song(), 48_000.0).unwrap();
        engine.seek(5 * u64::from(PPQ) / 2);
        engine.play();
        for (index, value) in [(0, 0.3), (71, 1.0)] {
            engine.set_param(1, 0, index, value);
        }
        for (index, value) in [(4, 1.0), (5, 0.7)] {
            engine.set_live_param(1, index, value);
        }
        assert_eq!(params(&engine, [8]), [Some(1.0)]);
        jump(&mut engine);
        engine
    }

    /// Stop: params 0–5 go back to their initial values, MIDI's live Pitch Bend and Aftertouch too (no song holds them;
    /// A live Level is the song's, as the app writes both); the MIDI CCs (7–133) keep theirs, the pedal's too.
    #[test]
    fn stop_resets_params_0_to_5_and_keeps_the_midi_ccs() {
        let saved = cc_song().channels[1].params.0;
        let engine = jumped(|engine| engine.stop());
        let expected = [
            Some(0.3),
            Some(saved[4]),
            Some(saved[5]),
            Some(1.0),
            Some(1.0),
        ];
        assert_eq!(params(&engine, [0, 4, 5, 8, 71]), expected);
    }

    /// Re-sync step 1: as Stop, a re-sync (a seek to beat 1, before the CC's clip) resets Pitch Bend and Aftertouch and
    /// keeps the CCs; an automation lane on a CC still sets it in step 4 (a seek past the clip).
    #[test]
    fn a_re_sync_resets_bend_and_aftertouch_and_keeps_a_cc() {
        let saved = cc_song().channels[1].params.0;
        let mut engine = jumped(|engine| engine.seek(u64::from(PPQ)));
        let expected = [
            Some(0.3),
            Some(saved[4]),
            Some(saved[5]),
            Some(1.0),
            Some(1.0),
        ];
        assert_eq!(params(&engine, [0, 4, 5, 8, 71]), expected);
        engine.set_param(1, 0, 8, 0.2);
        engine.seek(4 * u64::from(PPQ));
        assert_eq!(params(&engine, [8]), [Some(1.0)]);
    }

    /// The on-screen pitch wheel writes the song's bend (`SetParam`), so a seek, the loop wrap and Stop keep it at 0.9;
    /// MIDI's (`LiveParam`) goes back to the saved value at each.
    #[test]
    fn the_pitch_wheels_bend_holds_through_a_seek_a_wrap_and_stop_and_midis_resets() {
        let mut song = cc_song();
        song.play_loop = Some((0, PPQ));
        let saved = song.channels[1].params.0[4];
        for wheel in [true, false] {
            let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
            engine.play();
            let bend = |engine: &mut SongAudioEngine| match wheel {
                true => engine.set_param(1, 0, 4, 0.9),
                false => engine.set_live_param(1, 4, 0.9),
            };
            let expected = [Some(if wheel { 0.9 } else { saved })];
            bend(&mut engine);
            engine.seek(u64::from(PPQ) / 2);
            assert_eq!(params(&engine, [4]), expected, "seek, wheel {wheel}");
            bend(&mut engine);
            engine.render(&mut vec![0.0; 24_000 * 2], 2);
            assert_eq!(params(&engine, [4]), expected, "wrap, wheel {wheel}");
            bend(&mut engine);
            engine.stop();
            assert_eq!(params(&engine, [4]), expected, "stop, wheel {wheel}");
        }
    }

    /// A release of a key past 127 (no MIDI key) changes nothing and does not overflow the key bits.
    #[test]
    fn a_live_key_past_127_lifts_without_a_panic() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
        engine.note_on(1, 60, 1.0);
        engine.note_off(1, 200);
        engine.render(&mut vec![0.0; 64 * 2], 2);
        assert_eq!(engine.sounding(1), 1 << 60);
    }

    /// Play start stops the pickers' sample preview, which otherwise plays on past the master.
    #[test]
    fn play_stops_the_sample_preview() {
        use honeydew_audio::sample::SampleData;
        let data = Arc::new(SampleData {
            sample_rate: 48_000,
            left: vec![0.5; 48_000].into(),
            right: None,
            meta: Default::default(),
        });
        let mut engine = SongAudioEngine::new(&Song::default(), 48_000.0).unwrap();
        let mut output = vec![0.0; 256 * 2];
        engine.preview(Some((Arc::clone(&data), 0)));
        engine.render(&mut output, 2);
        assert!(output.iter().any(|s| s.abs() > 0.1), "the preview plays");
        engine.play();
        engine.render(&mut output, 2);
        assert!(output.iter().all(|s| *s == 0.0), "stopped at Play");
    }

    /// The MIDI CC range Stop and a re-sync keep starts at CC #0 (param 7) and ends at CC #127 (param 133): each, moved
    /// by its lane, keeps its value at Stop; channel pressure (param 5) does not.
    #[test]
    fn the_kept_cc_range_is_params_7_to_133() {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        for param in [5, 7, 133] {
            let lane = song.channels[1].add_automation_lane(crate::song::AutomationTarget {
                instance_id: 0,
                param,
            });
            lane.add_clip(0, 1.0).events.push(Event::Automation {
                pos: 0,
                value: 1.0,
                control: true,
            });
        }
        let saved = song.channels[1].params.0[5];
        let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
        engine.play();
        engine.render(&mut vec![0.0; 64 * 2], 2);
        assert_eq!(params(&engine, [5, 7, 133]), [Some(1.0); 3]);
        engine.stop();
        assert_eq!(
            params(&engine, [5, 7, 133]),
            [Some(saved), Some(1.0), Some(1.0)]
        );
    }

    /// A loop wrap's re-sync sets only the params whose value it changes (some 10 000 set again at every wrap were most
    /// of its block): the one its automation moved, of all the instrument's.
    #[test]
    fn a_loop_wrap_sets_only_the_params_it_changes() {
        use std::sync::Mutex;

        /// Records each param it is set, as (index, value).
        struct Probe(Arc<Mutex<Vec<(usize, f32)>>>);
        impl Instrument for Probe {
            fn set_parameter(&mut self, index: usize, value: f32) {
                self.0.lock().unwrap().push((index, value));
            }
            fn note_on(&mut self, _: u8, _: f32, _: f32) {}
            fn note_off(&mut self, _: u8) {}
            fn all_notes_off(&mut self) {}
            fn reset(&mut self) {}
            fn process(&mut self, _: &mut [f32], _: &mut [f32], _: ProcessContext) {}
        }
        let mut song = Song {
            tempo_bpm: 120.0,
            play_loop: Some((0, 2 * PPQ)),
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let synth = &song.channels[1].rack.modules[0];
        let (id, saved) = (synth.instance_id, synth.params[9]);
        let target = crate::song::AutomationTarget {
            instance_id: id,
            param: 9,
        };
        let lane = song.channels[1].add_automation_lane(target);
        lane.add_clip(PPQ, 1.0).events.push(Event::Automation {
            pos: 0,
            value: 0.7,
            control: true,
        });
        let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
        let set = Arc::new(Mutex::new(Vec::new()));
        engine.instruments[1][0].1.instrument = Box::new(Probe(set.clone()));
        engine.play();
        // To beat 1.5: the automation set 0.7 at beat 1.
        engine.render(&mut vec![0.0; 36_000 * 2], 2);
        set.lock().unwrap().clear();
        // Over the wrap at beat 2, to beat 0.25: back to the saved value, and nothing else.
        engine.render(&mut vec![0.0; 18_000 * 2], 2);
        assert_eq!(*set.lock().unwrap(), [(9, saved)]);
    }

    /// A param set that moves another of its effect (Tremolo, ID 46: Max below Min lowers Min) leaves what the effect
    /// holds known, so a re-sync (a loop wrap, a seek) still sets the other back. Max 0.8, Min 0.5; Max automated to
    /// 0.2 from beat 1 (Min follows to 0.2); the loop is beats 0 to 2.
    #[test]
    fn a_re_sync_restores_a_param_its_coupled_param_moved() {
        let mut song = Song {
            tempo_bpm: 120.0,
            play_loop: Some((0, 2 * PPQ)),
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let id = song.channels[1].rack.add(46);
        let tremolo = song.channels[1].rack.get_mut(id).unwrap();
        (tremolo.params[3], tremolo.params[4]) = (0.8, 0.5);
        let target = crate::song::AutomationTarget {
            instance_id: id,
            param: 3,
        };
        let lane = song.channels[1].add_automation_lane(target);
        lane.add_clip(PPQ, 1.0).events.push(Event::Automation {
            pos: 0,
            value: 0.2,
            control: true,
        });
        let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
        let limits = |engine: &SongAudioEngine| {
            let tremolo = engine.audio.effect(1, id).unwrap();
            (tremolo.parameter(3), tremolo.parameter(4))
        };
        engine.play();
        // To beat 1.5: both limits at 0.2.
        engine.render(&mut vec![0.0; 36_000 * 2], 2);
        assert_eq!(limits(&engine), (Some(0.2), Some(0.2)));
        // Over the wrap at beat 2, to beat 0.25.
        engine.render(&mut vec![0.0; 18_000 * 2], 2);
        assert_eq!(limits(&engine), (Some(0.8), Some(0.5)), "the wrap");
        engine.render(&mut vec![0.0; 36_000 * 2], 2);
        engine.seek(u64::from(PPQ) / 2);
        assert_eq!(limits(&engine), (Some(0.8), Some(0.5)), "a seek");
    }
}

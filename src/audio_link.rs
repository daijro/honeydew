//! The app's link to the audio output: the device sink, the commands the full audio
//! queue handed back, starting and reopening the stream, and whether the stream has failed.

use std::collections::VecDeque;
use std::sync::Arc;

use honeydew_audio::device::{
    AudioCommand, AudioOutput, AudioRenderer, BufferMode, OutputSnapshot,
};
use honeydew_audio::sample::SampleData;

use crate::audio_engine::{LiveInput, SongAudioEngine, settings, strip_param};
use crate::sample_cache::{Reader, SampleCache};
use crate::song::Song;

mod clock;
mod edits;

/// How [`AudioLink::replace_song_keeping`] hands the audio an engine for a song.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handover {
    /// A fresh engine at the tick: a new song, Preview switched on or off, another input.
    Fresh,
    /// A song edit: its engine takes over the playing one's sound, cutting only what the edit destroys.
    Edit,
    /// An undo or redo: as an edit, and it always re-syncs.
    Undo,
    /// An edit that runs no re-sync, though its song changes would (a tap's new note or hit, the step sequencer's Clear
    /// and Fill): as an edit, never re-syncing; a deleted lane's channel still gets all notes off.
    Quiet,
    /// Preview switched on, moved or off: the engine for the previewed clip alone, or the whole song again, takes over
    /// The playing one; every channel gets all notes off, then a re-sync runs, and effect tails ring on. `seek`: the
    /// playhead jumps there (into Preview's loop), else it stays.
    Preview { seek: Option<u64> },
}

/// Channel param `index` is a live controller the link keeps for an edit's engine: a MIDI CC (7 + n), which no song
/// holds and Stop and a re-sync keep. The MIDI input and the MOD wheel send them. Pitch Bend (4) and channel pressure
/// (5) go back to the song's at a re-sync and Stop; the engine taking over keeps them otherwise.
fn controller(index: i32) -> bool {
    (7..=133).contains(&index)
}

/// A command for the song renderer on the audio thread.
pub type Command = AudioCommand<SongAudioEngine>;

/// Where the app's audio commands go: the device output, or a recorder in tests.
pub trait AudioSink {
    /// Queues `cmd` for the audio thread, or hands it back when the queue is full.
    fn send(&mut self, cmd: Command) -> Result<(), Command>;
    /// Transport tick, CPU load, stream failure and meters; read once per frame.
    fn snapshot(&mut self) -> OutputSnapshot;
    /// For building a replacement [`SongAudioEngine`].
    fn sample_rate(&self) -> u32;
    /// Replaces a failed stream with one on the current output device playing `song` with `samples` and monitoring
    /// `input`, at the transport tick and Play/Stop state last sent. On failure the old output stays and can be
    /// reopened again.
    fn reopen(
        &mut self,
        song: &Song,
        samples: &SampleCache,
        input: Option<&LiveInput>,
        mode: BufferMode,
    ) -> Result<(), String>;
}

impl AudioSink for AudioOutput<SongAudioEngine> {
    fn send(&mut self, cmd: Command) -> Result<(), Command> {
        AudioOutput::send(self, cmd)
    }

    fn snapshot(&mut self) -> OutputSnapshot {
        AudioOutput::snapshot(self)
    }

    fn sample_rate(&self) -> u32 {
        AudioOutput::sample_rate(self)
    }

    fn reopen(
        &mut self,
        song: &Song,
        samples: &SampleCache,
        input: Option<&LiveInput>,
        mode: BufferMode,
    ) -> Result<(), String> {
        AudioOutput::reopen_with_mode(self, mode, |rate| {
            SongAudioEngine::with_input(song, rate as f32, samples, input)
        })
        .map_err(|err| format!("{err:?}"))
    }
}

/// Opens the default output device playing `song` with `samples`. On failure (no device, permission denied) it
/// logs one warning and the app runs without audio. It monitors nothing: an input opens at the output's rate, so
/// after it.
pub fn open_device(song: &Song, samples: &SampleCache) -> Option<Box<dyn AudioSink>> {
    open_device_with_latency(song, samples, false)
}

pub fn open_device_with_latency(
    song: &Song,
    samples: &SampleCache,
    low_latency: bool,
) -> Option<Box<dyn AudioSink>> {
    match AudioOutput::start_with_latency(low_latency, |rate| {
        SongAudioEngine::with_samples(song, rate as f32, samples)
    }) {
        Ok(output) => Some(Box::new(output)),
        Err(err) => {
            warn(&format!("audio is off: {err:?}"));
            None
        }
    }
}

pub fn open_device_with_mode(
    song: &Song,
    samples: &SampleCache,
    mode: BufferMode,
) -> Option<Box<dyn AudioSink>> {
    match AudioOutput::start_with_mode(mode, |rate| {
        SongAudioEngine::with_samples(song, rate as f32, samples)
    }) {
        Ok(output) => Some(Box::new(output)),
        Err(err) => {
            warn(&format!("audio is off: {err:?}"));
            None
        }
    }
}

pub fn buffer_mode(sound_mode: u8, low_latency: u8) -> BufferMode {
    let target = crate::settings::BUFFER_SIZES
        .get(usize::from(low_latency))
        .copied()
        .unwrap_or_else(|| *crate::settings::BUFFER_SIZES.last().unwrap());
    let requested = (target > 0).then_some(BufferMode::Fixed(target));
    // The explicit buffer target is independent of the desktop Sound Mode. Native keeps
    // the device default; a selected target is applied in either Standard or Multitrack.
    if cfg!(desktop) {
        return requested.unwrap_or(BufferMode::Native);
    }
    requested.unwrap_or(match sound_mode {
        2 => BufferMode::UltraSafe,
        0 => BufferMode::Safe,
        1 => BufferMode::Fast,
        _ => BufferMode::Native,
    })
}

#[cfg(test)]
thread_local! {
    /// How many warnings this thread logged, for the tests.
    static WARNINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One line on stderr, or in the browser console on the web.
fn warn(msg: &str) {
    #[cfg(test)]
    WARNINGS.with(|w| w.set(w.get() + 1));
    #[cfg(target_arch = "wasm32")]
    web_sys::console::warn_1(&msg.into());
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("{msg}");
}

/// The audio output and the commands waiting for room in its queue. Without an output, or while its stream is
/// failed, commands are dropped.
#[derive(Default)]
pub struct AudioLink {
    /// The output, from the first [`Self::start`] that opens one (the web needs a user gesture to start audio).
    sink: Option<Box<dyn AudioSink>>,
    /// The output's stream failed (its snapshot says so). Commands are dropped until a [`Self::start`] reopens it.
    stream_failed: bool,
    /// The selected output buffer target index; zero keeps the device default.
    low_latency: u8,
    /// Stored sound-mode id; 2=UltraSafe, 0=Safe, 1=Fast, 3=Native.
    sound_mode: u8,
    /// Commands the full audio queue handed back, oldest first. They go out before any newer command.
    pending: VecDeque<Command>,
    /// The live notes and controllers waiting for the transport's next Play or Stop, in order, while held
    /// ([`Self::hold_notes`]).
    waiting: Option<Vec<Command>>,
    /// The samples the engines play, loaded before each build. Its `missing()` lists what the current song plays and
    /// the cache could not load (a build with a part of the song, as Preview's, keeps the rest of its samples
    /// meanwhile); `load` frees the rest. Read it right after a load's build for the song's missing files.
    pub samples: SampleCache,
    /// The live input each engine build monitors and records. Set it when the input opens, closes or changes ports,
    /// then rebuild: a running engine keeps the input it was built with.
    pub input: Option<LiveInput>,
    /// The input the last engine built got ([`Self::records`]).
    built_input: Option<LiveInput>,
    /// The metronome as last sent (`AudioCommand::Metronome`); each new engine gets it again.
    metronome: Option<(bool, bool, f32)>,
    /// The metronome flag as last sent (`AudioCommand::MetronomeFlag`), after the metronome: a runtime flag, off at
    /// start, kept across songs and never saved ([`Self::metronome_flag`]).
    metronome_flag: Option<(bool, f32)>,
    /// Effective desktop multi-core setting; new renderers receive it too.
    multi_core: bool,
    /// The beats of the count-in last sent (`AudioCommand::CountIn`), armed or running, until Stop, `CountIn(0)` or
    /// a poll that sees it end: a new engine counts in again before it plays.
    count_in: Option<u32>,
    /// The last snapshot's `counting_in`.
    counting_in: bool,
    /// The last snapshot's transport tick.
    tick: u64,
    /// The tick the last poll saw when `Play` went out with [`Self::count_in`] set: the transport is held there until
    /// the count-in ends. A poll that finds it moved with no count-in on has seen it end, whether or not a poll ever
    /// saw it run (one shorter than a frame, or a snapshot that missed it).
    held: Option<u64>,
    /// Each `SetSetting` as last sent, as (channel, instance id, key, value), the channel an index into
    /// [`Self::channel_ids`]: each new engine gets the screen-only ones again ([`settings::screen_only`]).
    settings: Vec<(usize, i32, u32, f32)>,
    /// The channel ids of the song the last engine was built from.
    channel_ids: Vec<i32>,
    /// Each live controller as last sent ([`controller`]: the MIDI CCs, the MOD wheel's too), as (channel id, index,
    /// value): the song holds no such value, so an edit's engine gets it again ([`Self::keep_controllers`]). A live
    /// Pitch Bend and channel pressure (`LiveParam`) are the engine's to keep through an edit. The Level or Pan written
    /// after CC #7, #8 or #10 (which set them) drops those.
    controllers: Vec<(i32, i32, f32)>,
    /// The song the last engine was built from, to tell what an edit's engine re-syncs ([`edits::edit`]).
    // ponytail: a whole copy of the song, which `edits::edit` diffs with the next one (a digest could not tell what
    // changed); share the undo history's snapshot instead if copying large songs ever shows.
    song: Option<Song>,
    /// The tempo a drag set live since the last engine was built ([`Self::set_tempo`]).
    tempo: Option<f64>,
    /// Each sample a `Preview` sent, kept until the engine has let go of it ([`Self::poll`]), so the audio thread never
    /// frees one: the picker's strip may drop its own first.
    previews: Vec<Arc<SampleData>>,
    /// The playhead the editors draw ([`Self::advance_playhead`]), and how far it runs ahead of the last snapshot.
    clock: clock::PlayheadClock,
    lead: u64,
}

impl AudioLink {
    /// Prepare workers on this (UI) thread, then switch the running renderer at
    /// the next callback. No engine or audio stream restart is needed.
    pub fn set_multi_core(&mut self, on: bool) {
        let enabled = on && crate::audio_engine::prepare_multi_core();
        if self.multi_core != enabled {
            self.multi_core = enabled;
            self.send(AudioCommand::SetMultiCore(enabled));
        }
    }

    /// The song the last engine was built from.
    #[cfg(test)]
    pub(crate) fn built_song(&self) -> Option<&Song> {
        self.song.as_ref()
    }

    /// The tempo, BPM, the engine's playhead runs at: a tempo drag's, sent live ([`Self::set_tempo`]), else that of the
    /// song the last engine was built from. `song`'s when no engine was built.
    pub fn engine_tempo(&self, song: &Song) -> f64 {
        (self.tempo).unwrap_or_else(|| self.song.as_ref().unwrap_or(song).tempo_bpm)
    }

    /// A tempo drag: the running engine plays at `bpm` from its next block and its notes go on; nothing is rebuilt. The
    /// drag's end is an edit as any other.
    pub fn set_tempo(&mut self, bpm: f64) {
        let Some(built) = self.song.as_ref().filter(|_| self.running()) else {
            return;
        };
        if self.tempo.unwrap_or(built.tempo_bpm) != bpm {
            self.tempo = Some(bpm);
            self.send(AudioCommand::SetTempo(bpm as f32));
        }
    }

    /// The engine playing was built with [`Self::input`]'s ring and tap, so it records that ring into that tap.
    pub fn records(&self) -> bool {
        (self.input.as_ref().zip(self.built_input.as_ref()))
            .is_some_and(|(i, b)| Arc::ptr_eq(&i.ring, &b.ring) && Arc::ptr_eq(&i.tap, &b.tap))
    }

    /// There is an output and its stream works.
    pub fn running(&self) -> bool {
        self.sink.is_some() && !self.stream_failed
    }

    /// The running output's sample rate, Hz.
    pub fn sample_rate(&self) -> Option<u32> {
        let sink = self.sink.as_ref().filter(|_| self.running())?;
        Some(sink.sample_rate())
    }

    /// Sends `cmd` after any commands still pending; one the queue hands back waits for the next
    /// [`Self::flush`]. Without a running output, `cmd` is dropped. A fresh engine drops the pending engines, params
    /// and seeks it supersedes; one that takes over the playing engine keeps the params and seeks (it takes over what
    /// they leave) and replaces a waiting one that takes over too when only transport commands, or what goes out again
    /// after every engine, follow that one, so a queue that stays full without other commands holds one engine at most.
    // ponytail: only a fresh engine drops what it supersedes. While the queue stays full everything else piles up
    // (each small: a command boxes its engine): notes and Play/Stop, and, since every edit's engine takes over and
    // needs them, SetParam, Seek, SetSetting and SetTempo too (a knob or tempo drag sends one a frame), and a note or
    // param behind a waiting engine keeps one engine per edit. Coalesce those per target, and remap them by channel id
    // to merge their engines, if a stalled device matters.
    pub fn send(&mut self, cmd: Command) {
        // A live controller (a CC, pitch bend, pressure) waits with the notes, in input order: a note-off then the
        // sustain pedal down stays unsustained.
        let live = matches!(
            cmd,
            AudioCommand::NoteOn { .. }
                | AudioCommand::NoteOff { .. }
                | AudioCommand::LiveParam { .. }
        ) || matches!(cmd, AudioCommand::SetParam { instance_id: 0, index, .. } if controller(index));
        if let Some(waiting) = &mut self.waiting
            && live
        {
            waiting.push(cmd);
            return;
        }
        match cmd {
            AudioCommand::Metronome { on, click, level } => {
                self.metronome = Some((on, click, level))
            }
            AudioCommand::MetronomeFlag { on, level } => self.metronome_flag = Some((on, level)),
            AudioCommand::CountIn(beats) => {
                (self.count_in, self.held) = (Some(beats).filter(|b| *b > 0), None);
            }
            // A poll that sees no count-in after this has not seen one end (a new one may be on its way).
            AudioCommand::Stop => (self.count_in, self.counting_in) = (None, false),
            // ponytail: the tick the last poll saw, not a Seek sent since: a snapshot taken after that Seek and before
            // this Play reached the engine would read as the end. Both go out in one frame here, so the window is a
            // callback landing between two sends; an engine-side count-in serial in the snapshot would close it.
            AudioCommand::Play => self.held = self.count_in.map(|_| self.tick),
            // One clone a sample (the play button sends it again): two would keep each other past `poll`'s count.
            AudioCommand::Preview(Some((ref data, _)))
                if !self.previews.iter().any(|p| Arc::ptr_eq(p, data)) =>
            {
                self.previews.push(Arc::clone(data))
            }
            AudioCommand::SetParam {
                channel,
                instance_id: 0,
                index,
                value,
            } => {
                let id = self
                    .song
                    .as_ref()
                    .and_then(|s| Some(s.channels.get(channel)?.id));
                if let Some(id) = id {
                    if controller(index) {
                        match (self.controllers.iter_mut()).find(|c| (c.0, c.1) == (id, index)) {
                            Some(sent) => sent.2 = value,
                            None => self.controllers.push((id, index, value)),
                        }
                    } else {
                        // The Level or Pan that CC #7, #8 or #10 set, written again: the later write wins, so those CCs
                        // are no longer kept.
                        (self.controllers).retain(|c| c.0 != id || strip_param(0, c.1) != index);
                    }
                }
            }
            AudioCommand::SetSetting {
                channel,
                instance_id,
                key,
                value,
            } => {
                let at = (channel, instance_id, key);
                match self.settings.iter_mut().find(|s| (s.0, s.1, s.2) == at) {
                    Some(setting) => setting.3 = value,
                    None => self.settings.push((channel, instance_id, key, value)),
                }
            }
            _ => {}
        }
        if self.running() {
            let cmd = match cmd {
                // An engine that takes over the playing one needs what is still on its way to it, but gives way to
                // the next: the newest engine waiting, if it takes over too, is replaced in place by this one, which
                // takes on its re-sync and silenced channels (so a queue that stays full holds one such engine).
                // Only while no command naming one of its song's channels waits behind it (a note, a param, a
                // setting), which must reach that song's engine. The transport's name none, and what goes out again
                // after every engine ([`Self::resent`]) is dropped: it goes out again for this one.
                AudioCommand::ReplaceSong(engine) if engine.adopts() => {
                    let behind = |c: &Command| {
                        matches!(
                            c,
                            AudioCommand::Play | AudioCommand::Stop | AudioCommand::Seek(_)
                        ) || self.resent(c)
                    };
                    let at = self.pending.iter().rposition(|c| !behind(c));
                    match at.map(|at| (at, &mut self.pending[at])) {
                        Some((at, AudioCommand::ReplaceSong(waiting))) if waiting.adopts() => {
                            let resync = waiting.resyncs() || engine.resyncs();
                            // Its channels by id: the next edit may have added or deleted some.
                            let ids = waiting.channel_ids();
                            let index = |&at: &usize| {
                                let id = ids.get(at)?;
                                engine.channel_ids().iter().position(|other| other == id)
                            };
                            let mut silence: Vec<usize> =
                                waiting.silences().iter().filter_map(index).collect();
                            silence.extend(engine.silences());
                            silence.sort_unstable();
                            silence.dedup();
                            **waiting = (*engine).adopting_edit(resync, silence);
                            let tail = self.pending.split_off(at + 1);
                            let kept: Vec<_> =
                                tail.into_iter().filter(|c| !self.resent(c)).collect();
                            self.pending.extend(kept);
                            None
                        }
                        _ => Some(AudioCommand::ReplaceSong(engine)),
                    }
                }
                AudioCommand::ReplaceSong(engine) => {
                    self.pending.retain(|c| {
                        !matches!(
                            c,
                            AudioCommand::ReplaceSong(_)
                                | AudioCommand::SetParam { .. }
                                | AudioCommand::Seek(_)
                        )
                    });
                    Some(AudioCommand::ReplaceSong(engine))
                }
                cmd => Some(cmd),
            };
            self.pending.extend(cmd);
            self.flush();
        }
    }

    /// Sends the live notes held so far, then holds the next ones while `hold`, and the live controllers (CCs, pitch
    /// bend, pressure) with them in input order: the app holds each frame's until its transport command has gone out,
    /// for a press may start playback (REC armed, the transport stopped) or come with a Play start, and its note must
    /// reach the engine after that Play and its all-notes-off (the key that starts playback keeps sounding).
    pub fn hold_notes(&mut self, hold: bool) {
        for cmd in self.waiting.take().into_iter().flatten() {
            self.send(cmd);
        }
        self.waiting = hold.then(Vec::new);
    }

    /// The keys whose press waits for the next Play ([`Self::hold_notes`]), as (channel, key).
    pub fn waiting_presses(&self) -> impl Iterator<Item = (usize, u8)> + '_ {
        (self.waiting.iter().flatten()).filter_map(|cmd| match *cmd {
            AudioCommand::NoteOn { channel, key, .. } => Some((channel, key)),
            _ => None,
        })
    }

    /// `cmd`, keyed by the song [`Self::channel_ids`] holds, goes out again after every engine: the live lane (the
    /// app's) and a screen-only setting ([`Self::resend_settings`]), its module looked up in the newest song by id.
    fn resent(&self, cmd: &Command) -> bool {
        match *cmd {
            AudioCommand::LiveLane { .. } => true,
            AudioCommand::SetSetting {
                channel,
                instance_id,
                key,
                ..
            } => {
                let module = (self.channel_ids.get(channel)).and_then(|id| {
                    let song = self.song.as_ref()?;
                    song.channels
                        .iter()
                        .find(|c| c.id == *id)?
                        .rack
                        .get(instance_id)
                });
                module.is_some_and(|m| settings::screen_only(m.type_id, key))
            }
            _ => false,
        }
    }

    /// Retries the pending commands in order, stopping at the first one the queue hands back.
    pub fn flush(&mut self) {
        let Some(sink) = &mut self.sink else { return };
        while let Some(cmd) = self.pending.pop_front() {
            if let Err(cmd) = sink.send(cmd) {
                self.pending.push_front(cmd);
                break;
            }
        }
    }

    /// Opens the output with `open`, or reopens a failed stream, playing `song` with its samples (read with
    /// `read`); does nothing while running. `playing` is the play button's state, which the new transport takes. A
    /// failed attempt leaves the link as it was, to be tried again.
    pub fn start(
        &mut self,
        song: &Song,
        playing: bool,
        read: &Reader<'_>,
        open: impl FnOnce(&Song, &SampleCache) -> Option<Box<dyn AudioSink>>,
    ) {
        self.start_keeping(song, song, playing, read, open);
    }

    /// [`Self::start`] for an engine that plays `song`, a part of the song the app holds (`keep`): the samples of
    /// the rest stay cached ([`SampleCache::load_keeping`]).
    pub fn start_keeping(
        &mut self,
        song: &Song,
        keep: &Song,
        playing: bool,
        read: &Reader<'_>,
        open: impl FnOnce(&Song, &SampleCache) -> Option<Box<dyn AudioSink>>,
    ) {
        self.start_keeping_mode(
            song,
            keep,
            playing,
            read,
            |song, samples, _| open(song, samples),
            self.low_latency,
        );
    }

    /// Like [`Self::start_keeping`], with an explicit output buffer policy for the initial stream.
    pub fn start_keeping_mode(
        &mut self,
        song: &Song,
        keep: &Song,
        playing: bool,
        read: &Reader<'_>,
        open: impl FnOnce(&Song, &SampleCache, u8) -> Option<Box<dyn AudioSink>>,
        low_latency: u8,
    ) {
        self.low_latency = low_latency;
        if self.running() {
            return;
        }
        self.samples.load_live(song, keep, read);
        // The song's plugins run before an engine is built for it.
        crate::plugins::sync(keep);
        (self.song, self.tempo) = (Some(song.clone()), None);
        self.controllers.clear();
        let mode = self.buffer_mode();
        let Some(sink) = &mut self.sink else {
            self.sink = open(song, &self.samples, low_latency);
            // The arm before Play: a recording's engine does not wrap at the song end.
            self.resend_metronome();
            // The play button works without audio; the new engine starts stopped.
            if playing {
                self.send(AudioCommand::Play);
            }
            self.resend_settings(song);
            return;
        };
        match sink.reopen(song, &self.samples, self.input.as_ref(), mode) {
            Ok(()) => {
                self.stream_failed = false;
                self.built_input.clone_from(&self.input);
                self.resend_metronome();
                // The reopen restores the transport as last sent; the play button may have changed since.
                self.send(if playing {
                    AudioCommand::Play
                } else {
                    AudioCommand::Stop
                });
                self.resend_settings(song);
            }
            Err(err) => warn(&format!("the audio stream is still off: {err}")),
        }
    }

    /// Changes the output buffer policy, reopening the current stream while preserving transport state.
    pub fn set_low_latency(&mut self, song: &Song, low_latency: u8) {
        if self.low_latency == low_latency {
            return;
        }
        self.low_latency = low_latency;
        let mode = self.buffer_mode();
        let Some(sink) = &mut self.sink else { return };
        if let Err(err) = sink.reopen(song, &self.samples, self.input.as_ref(), mode) {
            warn(&format!("could not change audio latency mode: {err}"));
            self.stream_failed = true;
            return;
        }
        self.stream_failed = false;
    }

    /// Records the preference before the first stream exists; an active stream is changed by [`Self::set_low_latency`].
    pub fn set_low_latency_preference(&mut self, low_latency: u8) {
        self.low_latency = low_latency;
    }

    pub fn set_sound_mode(&mut self, song: &Song, sound_mode: u8) {
        if self.sound_mode == sound_mode {
            return;
        }
        self.sound_mode = sound_mode;
        let mode = self.buffer_mode();
        let Some(sink) = &mut self.sink else { return };
        if let Err(err) = sink.reopen(song, &self.samples, self.input.as_ref(), mode) {
            warn(&format!("could not change sound mode: {err}"));
            self.stream_failed = true;
        } else {
            self.stream_failed = false;
        }
    }

    pub fn set_sound_mode_preference(&mut self, sound_mode: u8) {
        self.sound_mode = sound_mode;
    }

    fn buffer_mode(&self) -> BufferMode {
        buffer_mode(self.sound_mode, self.low_latency)
    }

    /// The multicore choice, metronome and its flag for a fresh renderer.
    fn resend_metronome(&mut self) {
        if self.multi_core {
            self.send(AudioCommand::SetMultiCore(true));
        }
        if let Some((on, click, level)) = self.metronome {
            self.send(AudioCommand::Metronome { on, click, level });
        }
        if let Some((on, level)) = self.metronome_flag {
            self.send(AudioCommand::MetronomeFlag { on, level });
        }
    }

    /// The metronome flag is on: playback clicks the beats, REC armed or not (the tempo dialog's toggle).
    pub fn metronome_flag(&self) -> bool {
        self.metronome_flag.is_some_and(|(on, _)| on)
    }

    /// The screen-only settings as last sent, for a new engine built from `song`: re-keyed to its channels by channel
    /// id, so they follow a channel that moved. Those whose channel or module is gone, or whose module takes no such
    /// setting, are dropped, as are the other settings (the song's blobs hold them).
    fn resend_settings(&mut self, song: &Song) {
        let ids = song.channels.iter().map(|c| c.id).collect();
        let before = std::mem::replace(&mut self.channel_ids, ids);
        for (channel, instance_id, key, value) in std::mem::take(&mut self.settings) {
            let channel = match before.get(channel) {
                Some(id) => self.channel_ids.iter().position(|i| i == id),
                // Before a first build the indices are the song's own.
                None if before.is_empty() => Some(channel),
                None => None,
            };
            let module = channel.and_then(|c| song.channels.get(c)?.rack.get(instance_id));
            if let (Some(channel), Some(module)) = (channel, module)
                && settings::screen_only(module.type_id, key)
            {
                self.send(AudioCommand::SetSetting {
                    channel,
                    instance_id,
                    key,
                    value,
                });
            }
        }
    }

    /// The output's snapshot for this frame, if there is an output. A stream that has just failed logs one
    /// warning and drops the pending commands: the reopened output restores the transport. A count-in it sees end
    /// is forgotten.
    // ponytail: the snapshot's meters are a fresh Vec each frame (32 bytes a module, built in `crates/audio`), one
    // allocation per frame while playing; reuse a buffer there if a profile ever shows it.
    pub fn poll(&mut self) -> Option<OutputSnapshot> {
        // The previews the engine (and any command still queued) let go of are freed here, on this thread.
        self.previews.retain(|d| Arc::strong_count(d) > 1);
        let snapshot = self.sink.as_mut().map(|s| s.snapshot());
        let failed = snapshot.as_ref().is_some_and(|s| s.stream_failed);
        if failed && !self.stream_failed {
            warn("the audio stream failed; the next click restarts it");
            self.pending.clear();
        }
        self.stream_failed = failed;
        if let Some(s) = &snapshot {
            let moved = self.held.is_some_and(|h| h != s.transport_tick);
            if !s.counting_in && (self.counting_in || moved) {
                (self.count_in, self.held) = (None, None);
            }
            (self.counting_in, self.tick) = (s.counting_in, s.transport_tick);
        }
        snapshot
    }

    /// Once a frame after [`Self::poll`], at egui time `now`: steps the drawn playhead ([`clock::PlayheadClock`]) on
    /// from the last snapshot's tick, while the transport is `playing` at `bpm` on a running output and not counting in,
    /// never past `end`, the tick the engine wraps at (`None`: it plays on).
    pub fn advance_playhead(&mut self, now: f64, playing: bool, bpm: f64, end: Option<u64>) {
        let runs = playing && self.running() && !self.counting_in;
        let rate = runs.then_some(bpm / 60.0 * f64::from(crate::song::PPQ));
        let end = end.map_or(f64::INFINITY, |e| e as f64);
        self.lead = self.clock.advance(now, self.tick, rate, end) - self.tick;
    }

    /// How many ticks past this frame's snapshot the editors draw the playhead (while following): 0 unless playing. For
    /// drawing only (the line, the disc, follow playhead and the clip editors' folding); edits, seeks and recording use
    /// The snapshot's tick.
    pub fn playhead_lead(&self) -> u64 {
        self.lead
    }

    /// A count-in was sent and has not ended: armed for the next `Play`, or running. What the transport plays meanwhile
    /// is no take yet.
    pub fn counting_in(&self) -> bool {
        self.count_in.is_some()
    }

    /// Hands the audio an engine rebuilt from `song` after an edit, at `tick` (the transport position before the
    /// edit; Stop is a pause, so a stopped position is kept too), playing if `playing`. The new samples are read with
    /// `read` first. On failure the old engine keeps playing the old song. Without a running output nothing is built
    /// or read: a reopen builds from the song it is given. True when the new engine went out.
    pub fn replace_song(
        &mut self,
        song: &Song,
        playing: bool,
        tick: u64,
        read: &Reader<'_>,
    ) -> bool {
        self.replace_song_keeping(song, song, playing, tick, read, Handover::Edit)
    }

    /// [`Self::replace_song`] for an engine that plays `song`, a part of the song the app holds (`keep`): the samples
    /// of the rest stay cached ([`SampleCache::load_keeping`]), so that going back to the whole song decodes nothing.
    /// `how`: an edit's or undo's engine takes over the playing one, its transport included (no Seek), and re-syncs
    /// where `edits::edit` says; one after a failed build, or a fresh one, starts at the tick.
    pub fn replace_song_keeping(
        &mut self,
        song: &Song,
        keep: &Song,
        playing: bool,
        tick: u64,
        read: &Reader<'_>,
        how: Handover,
    ) -> bool {
        let rate = match &self.sink {
            Some(sink) if self.running() => sink.sample_rate(),
            _ => {
                // Nothing is built, but the screen-only settings follow the song's channels now: a setting sent before
                // The stream returns is keyed by this song.
                self.resend_settings(song);
                return false;
            }
        };
        self.samples.load_live(song, keep, read);
        crate::plugins::set_rate(rate);
        crate::plugins::sync(keep);
        let moved = self.tempo.is_some();
        let mut last = self.song.take().filter(|_| how != Handover::Fresh);
        let edit = last.as_mut().map(|last| match how {
            // Preview on or off gives every channel all notes off, then re-syncs.
            Handover::Preview { .. } => edits::Edit {
                resync: true,
                silence: (0..song.channels.len()).collect(),
            },
            _ => edits::edit(last, song, moved),
        });
        let built =
            SongAudioEngine::with_input(song, rate as f32, &self.samples, self.input.as_ref());
        if built.is_ok() {
            self.built_input.clone_from(&self.input);
        }
        match (built, edit, last) {
            (Ok(mut engine), Some(edit), Some(last)) => {
                engine.set_multi_core(self.multi_core);
                // The new engine takes over the playing one, its transport and state included. Play keeps the
                // output's own Play/Stop record.
                (self.song, self.tempo) = (Some(song.clone()), None);
                let resync = (edit.resync || how == Handover::Undo) && how != Handover::Quiet;
                // Re-sync step 1 puts params back to the saved or live one, so the live controllers stay through it
                // too.
                self.keep_controllers(&mut engine, &last, song);
                let engine = engine.adopting_edit(resync, edit.silence);
                self.send(AudioCommand::ReplaceSong(Box::new(engine)));
                if let Handover::Preview { seek: Some(tick) } = how {
                    self.send(AudioCommand::Seek(tick));
                }
                if playing {
                    self.send(AudioCommand::Play);
                }
                // Channels may have moved; a module built afresh lacks what the screen alone holds.
                self.resend_settings(song);
                true
            }
            (Ok(mut engine), _, _) => {
                engine.set_multi_core(self.multi_core);
                (self.song, self.tempo) = (Some(song.clone()), None);
                self.controllers.clear();
                self.send(AudioCommand::ReplaceSong(Box::new(engine)));
                // The arm before Play: a recording's engine does not wrap at the song end.
                self.resend_metronome();
                // The new engine starts stopped at tick 0.
                if playing || tick > 0 {
                    self.send(AudioCommand::Seek(tick));
                }
                // A count-in, armed or running, starts over on the new engine.
                if let Some(beats) = self.count_in {
                    self.send(AudioCommand::CountIn(beats));
                }
                if playing {
                    self.send(AudioCommand::Play);
                }
                self.resend_settings(song);
                true
            }
            (Err(err), _, _) => {
                warn(&format!("the song edit is not heard: {err:?}"));
                false
            }
        }
    }

    /// Gives `engine`, built for `song` to take over the engine built for `last`, each live controller as last sent
    /// ([`Self::controllers`]), before it goes out: an edit leaves a held MIDI sustain pedal or a CC where it was, and
    /// its re-sync, if any, keeps them (re-sync step 1 keeps the MIDI CCs, as a seek does). they are set on the
    /// new engine rather than sent after it, which would first lift the pedal and release what it holds. One whose
    /// channel is gone is forgotten (the song holds no CC value to change). A fresh engine and a reopen forget them
    /// all.
    fn keep_controllers(&mut self, engine: &mut SongAudioEngine, last: &Song, song: &Song) {
        self.controllers.retain(|&(id, index, value)| {
            let param = |song: &Song| {
                let channel = song.channels.iter().position(|c| c.id == id)?;
                let saved = song.channels[channel].params.0.get(index as usize).copied();
                Some((channel, saved))
            };
            match (param(last), param(song)) {
                (Some((_, before)), Some((channel, now))) if before == now => {
                    engine.set_param(channel, 0, index, value);
                    true
                }
                _ => false,
            }
        });
    }

    /// How many commands wait for room in the queue.
    #[cfg(test)]
    pub(crate) fn pending(&self) -> usize {
        self.pending.len()
    }

    /// There is an output, running or failed.
    #[cfg(test)]
    pub(crate) fn has_output(&self) -> bool {
        self.sink.is_some()
    }
}

/// The test double for the output, shared with the app's tests.
#[cfg(test)]
pub(crate) mod testing {
    use std::cell::RefCell;
    use std::rc::Rc;

    use honeydew_audio::device::AudioRenderer;

    use super::*;

    /// Warnings logged on this thread so far.
    pub(crate) fn warnings() -> usize {
        WARNINGS.with(|w| w.get())
    }

    /// A sink that records what it is sent. Clones share the record.
    #[derive(Clone, Default)]
    pub(crate) struct Recorder(pub(crate) Rc<RefCell<Recorded>>);

    #[derive(Default)]
    pub(crate) struct Recorded {
        pub(crate) log: Vec<String>,
        /// How many more commands the queue takes; `None` is unlimited.
        pub(crate) room: Option<usize>,
        pub(crate) tick: u64,
        /// The stream failed; a successful reopen clears it.
        pub(crate) failed: bool,
        /// Reopening fails.
        pub(crate) no_device: bool,
        /// Reopen attempts, failed ones included.
        pub(crate) reopens: usize,
        /// Each `SetParam` sent, as (channel, instance id, index, value).
        pub(crate) params: Vec<(usize, i32, i32, f32)>,
        /// Each `LiveLane` sent, as (channel, notes lane); kept out of `log`, which tests read command by command.
        pub(crate) lanes: Vec<(usize, bool)>,
        /// Every command sent, `LiveLane` included, in order.
        pub(crate) all: Vec<String>,
        /// The last engine sent, given the Play, Stop, Seek, SetLoop, the params, the live keys and lanes sent after it.
        pub(crate) engine: Option<SongAudioEngine>,
        /// The module meters each snapshot carries.
        pub(crate) meters: Vec<honeydew_audio::device::ModuleMeter>,
        /// The output's buffer size, which the tests' renders give the engine ([`AudioRenderer::set_buffer_frames`]).
        pub(crate) buffer_frames: u32,
        /// The output's sample rate, Hz; `None` is 48 kHz.
        pub(crate) sample_rate: Option<u32>,
    }

    impl Recorder {
        /// The commands sent since the last call.
        pub(crate) fn take(&self) -> Vec<String> {
            std::mem::take(&mut self.0.borrow_mut().log)
        }

        /// This recorder as the output a link opens.
        pub(crate) fn sink(&self) -> Option<Box<dyn AudioSink>> {
            Some(Box::new(self.clone()))
        }
    }

    fn describe(cmd: &Command) -> String {
        match cmd {
            AudioCommand::NoteOn {
                channel,
                key,
                velocity,
            } => format!("NoteOn {channel} {key} {:.0}", velocity * 127.0),
            AudioCommand::NoteOff { channel, key } => format!("NoteOff {channel} {key}"),
            AudioCommand::Play => "Play".into(),
            AudioCommand::Stop => "Stop".into(),
            AudioCommand::Seek(tick) => format!("Seek {tick}"),
            AudioCommand::ReplaceSong(engine) if engine.adopts() => {
                let mut text = String::from("ReplaceSong adopting");
                if !engine.resyncs() {
                    text += " no-resync";
                }
                if !engine.silences().is_empty() {
                    let channels: Vec<_> = engine.silences().iter().map(usize::to_string).collect();
                    text += &format!(" silencing {}", channels.join(","));
                }
                text
            }
            AudioCommand::ReplaceSong(_) => "ReplaceSong".into(),
            AudioCommand::SetParam {
                channel,
                instance_id,
                index,
                value,
            } => format!("SetParam {channel} {instance_id} {index} {value:.3}"),
            AudioCommand::SetLoop(Some((start, end))) => format!("SetLoop {start} {end}"),
            AudioCommand::SetLoop(None) => "SetLoop none".into(),
            AudioCommand::AllNotesOff => "AllNotesOff".into(),
            AudioCommand::ResetSound => "ResetSound".into(),
            AudioCommand::SetSetting {
                channel,
                instance_id,
                key,
                value,
            } => format!("SetSetting {channel} {instance_id} {key} {value:.3}"),
            AudioCommand::CountIn(beats) => format!("CountIn {beats}"),
            AudioCommand::Metronome { on, click, level } => {
                let silent = if *click { "" } else { " silent" };
                format!("Metronome {on} {level:.3}{silent}")
            }
            AudioCommand::MetronomeFlag { on, level } => format!("MetronomeFlag {on} {level:.3}"),
            AudioCommand::SetMultiCore(on) => format!("SetMultiCore {on}"),
            AudioCommand::Ping(id) => format!("Ping {id}"),
            AudioCommand::LiveLane { channel, notes } => format!("LiveLane {channel} {notes}"),
            AudioCommand::Preview(Some((data, from))) => {
                format!("Preview {} frames from {from}", data.frames())
            }
            AudioCommand::Preview(None) => "Preview stop".into(),
            AudioCommand::SetTempo(bpm) => format!("SetTempo {bpm:.3}"),
            AudioCommand::LiveParam {
                channel,
                index,
                value,
            } => format!("LiveParam {channel} {index} {value:.3}"),
        }
    }

    impl AudioSink for Recorder {
        fn send(&mut self, cmd: Command) -> Result<(), Command> {
            let mut r = self.0.borrow_mut();
            match &mut r.room {
                Some(0) => return Err(cmd),
                Some(room) => *room -= 1,
                None => {}
            }
            if let AudioCommand::SetParam {
                channel,
                instance_id,
                index,
                value,
            } = &cmd
            {
                r.params.push((*channel, *instance_id, *index, *value));
            }
            r.all.push(describe(&cmd));
            match cmd {
                AudioCommand::LiveLane { channel, notes } => r.lanes.push((channel, notes)),
                _ => r.log.push(describe(&cmd)),
            }
            match (cmd, &mut r.engine) {
                (AudioCommand::ReplaceSong(mut engine), slot) => {
                    if let Some(old) = slot.as_mut() {
                        engine.take_over(old);
                    }
                    *slot = Some(*engine);
                }
                (AudioCommand::Play, Some(engine)) => engine.play(),
                (AudioCommand::SetMultiCore(on), Some(engine)) => engine.set_multi_core(on),
                (AudioCommand::Stop, Some(engine)) => engine.stop(),
                (AudioCommand::Seek(tick), Some(engine)) => engine.seek(tick),
                (AudioCommand::ResetSound, Some(engine)) => engine.reset_sound(),
                (AudioCommand::SetLoop(range), Some(engine)) => engine.set_loop(range),
                (AudioCommand::CountIn(beats), Some(engine)) => engine.count_in(beats),
                (AudioCommand::Metronome { on, click, level }, Some(engine)) => {
                    engine.set_metronome(on, click, level)
                }
                (AudioCommand::MetronomeFlag { on, level }, Some(engine)) => {
                    engine.set_metronome_flag(on, level)
                }
                (AudioCommand::Ping(id), Some(engine)) => engine.ping(id),
                (
                    AudioCommand::NoteOn {
                        channel,
                        key,
                        velocity,
                    },
                    Some(engine),
                ) => engine.note_on(channel, key, velocity),
                (AudioCommand::NoteOff { channel, key }, Some(engine)) => {
                    engine.note_off(channel, key)
                }
                (
                    AudioCommand::SetSetting {
                        channel,
                        instance_id,
                        key,
                        value,
                    },
                    Some(engine),
                ) => engine.set_setting(channel, instance_id, key, value),
                (AudioCommand::LiveLane { channel, notes }, Some(engine)) => {
                    engine.set_live_lane(channel, notes)
                }
                (AudioCommand::SetTempo(bpm), Some(engine)) => engine.set_tempo(bpm),
                (
                    AudioCommand::LiveParam {
                        channel,
                        index,
                        value,
                    },
                    Some(engine),
                ) => engine.set_live_param(channel, index, value),
                (
                    AudioCommand::SetParam {
                        channel,
                        instance_id,
                        index,
                        value,
                    },
                    Some(engine),
                ) => engine.set_param(channel, instance_id, index, value),
                _ => {}
            }
            Ok(())
        }

        fn snapshot(&mut self) -> OutputSnapshot {
            let r = self.0.borrow();
            OutputSnapshot {
                transport_tick: r.tick,
                stream_failed: r.failed,
                module_meters: r.meters.clone(),
                counting_in: r.engine.as_ref().is_some_and(|e| e.counting_in()),
                ping: r.engine.as_ref().and_then(|e| e.ping_report()),
                // By channel, as the device publishes them.
                sounding: (r.engine.iter())
                    .flat_map(|e| (0..e.meter_count()).map(move |i| (e, e.meter_key(i))))
                    .filter(|(_, k)| k.instance_id == 0)
                    .map(|(e, k)| e.sounding(k.channel))
                    .collect(),
                ..OutputSnapshot::default()
            }
        }

        fn sample_rate(&self) -> u32 {
            self.0.borrow().sample_rate.unwrap_or(48_000)
        }

        fn reopen(
            &mut self,
            song: &Song,
            samples: &SampleCache,
            input: Option<&LiveInput>,
            _mode: BufferMode,
        ) -> Result<(), String> {
            let mut r = self.0.borrow_mut();
            r.reopens += 1;
            if r.no_device {
                return Err("no device".into());
            }
            r.failed = false;
            // As the device's: a fresh engine for `song`, at the tick (the link sends Play or Stop next).
            let rate = r.sample_rate.unwrap_or(48_000) as f32;
            if let Some(tick) = r.engine.as_ref().map(|old| old.transport_tick()) {
                let mut fresh = SongAudioEngine::with_input(song, rate, samples, input)
                    .map_err(|err| format!("{err:?}"))?;
                fresh.seek(tick);
                r.engine = Some(fresh);
            }
            r.log.push(format!("Reopen {}", song.channels.len()));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{Recorder, warnings};
    use super::*;
    use crate::flm::chunk::ContentPath;
    use crate::song::{ChannelKind, SynthKind};

    /// The link keeps MIDI CCs 0 to 127 (params 7 to 133), not Pitch Bend (4) or channel pressure (5), which a re-sync
    /// and Stop reset.
    #[test]
    fn the_kept_controllers_are_params_7_to_133() {
        let kept: Vec<i32> = (0..140).filter(|&index| controller(index)).collect();
        assert_eq!(kept, (7..=133).collect::<Vec<_>>());
    }

    fn note_on(key: u8) -> Command {
        AudioCommand::NoteOn {
            channel: 1,
            key,
            velocity: 100.0 / 127.0,
        }
    }

    /// A fresh engine for `song` at `tick`, as a new song, Preview or an input change hands over.
    fn fresh(link: &mut AudioLink, song: &Song, playing: bool, tick: u64) -> bool {
        link.replace_song_keeping(song, song, playing, tick, &|_| None, Handover::Fresh)
    }

    /// A link started on `rec` with the transport stopped, and an empty log.
    fn started(rec: &Recorder) -> AudioLink {
        let mut link = AudioLink::default();
        link.start(&Song::default(), false, &|_| None, |_, _| rec.sink());
        rec.take();
        link
    }

    #[test]
    fn multi_core_switches_live_and_is_resent_to_new_streams() {
        let rec = Recorder::default();
        let mut link = AudioLink::default();
        link.set_multi_core(true);
        if !link.multi_core {
            return; // A single-core or non-desktop host has no worker pool.
        }
        link.start(&Song::default(), false, &|_| None, |_, _| rec.sink());
        assert_eq!(rec.take(), ["SetMultiCore true"]);
        link.set_multi_core(false);
        assert_eq!(rec.take(), ["SetMultiCore false"]);
        link.set_multi_core(true);
        assert_eq!(rec.take(), ["SetMultiCore true"]);
        fresh(&mut link, &Song::default(), false, 0);
        assert_eq!(rec.take(), ["ReplaceSong", "SetMultiCore true"]);
    }

    #[test]
    fn without_an_output_commands_are_dropped_and_the_start_takes_the_play_state() {
        let mut link = AudioLink::default();
        link.send(AudioCommand::Play);
        assert!(link.pending.is_empty());
        assert!(!link.running());
        // Opening fails: the link stays without output.
        link.start(&Song::default(), true, &|_| None, |_, _| None);
        assert!(!link.running());

        let rec = Recorder::default();
        link.start(&Song::default(), true, &|_| None, |_, _| rec.sink());
        assert!(link.running());
        // The play button was on while audio was off.
        assert_eq!(rec.take(), ["Play"]);
        // A running link neither opens nor reopens.
        link.start(&Song::default(), true, &|_| None, |_, _| {
            panic!("opened twice")
        });
        assert_eq!(rec.0.borrow().reopens, 0);
        assert_eq!(rec.take(), Vec::<String>::new());
    }

    #[test]
    fn the_engine_tempo_is_that_of_the_song_the_last_engine_was_built_from() {
        // A tempo drag edits the song without a rebuild: the engine plays on at the tempo it was built at.
        let mut song = Song {
            tempo_bpm: 140.0,
            ..Song::default()
        };
        assert_eq!(
            AudioLink::default().engine_tempo(&song),
            140.0,
            "none: the song's"
        );
        // `started` builds the first engine from the default song, 120 BPM.
        let rec = Recorder::default();
        let mut link = started(&rec);
        assert_eq!(link.engine_tempo(&song), 120.0);
        assert!(link.replace_song(&song, false, 0, &|_| None));
        song.tempo_bpm = 70.0;
        assert_eq!(link.engine_tempo(&song), 140.0);
        assert!(link.replace_song(&song, false, 0, &|_| None));
        assert_eq!(link.engine_tempo(&song), 70.0);
    }

    fn preview_sample() -> std::sync::Arc<honeydew_audio::sample::SampleData> {
        std::sync::Arc::new(honeydew_audio::sample::SampleData {
            sample_rate: 48_000,
            left: vec![0.0; 8].into(),
            right: None,
            meta: Default::default(),
        })
    }

    #[test]
    fn a_previewed_sample_is_kept_until_the_engine_lets_go_of_it() {
        // The strip may drop its sample before the engine has stopped playing it; the link keeps it, so the audio
        // thread never frees it, and a poll lets go once the engine has.
        let rec = Recorder::default();
        let mut link = started(&rec);
        let data = preview_sample();
        let weak = std::sync::Arc::downgrade(&data);
        link.send(AudioCommand::Preview(Some((data, 0))));
        // The recorder (the engine here) has let go already; the link keeps it until a poll sees that.
        assert!(weak.upgrade().is_some());
        link.poll();
        assert!(weak.upgrade().is_none());
        // While the engine still holds it, a poll keeps the link's too.
        let data = preview_sample();
        let engine = data.clone();
        link.send(AudioCommand::Preview(Some((data, 0))));
        link.poll();
        assert_eq!(
            std::sync::Arc::strong_count(&engine),
            2,
            "the link still holds it"
        );
        let weak = std::sync::Arc::downgrade(&engine);
        drop(engine);
        link.poll();
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn a_sample_played_twice_is_still_let_go() {
        // The strip's play button (or a waveform release) sends the same Arc again; one link clone each, or two would
        // keep each other past the poll's count and leak the sample.
        let rec = Recorder::default();
        let mut link = started(&rec);
        let data = preview_sample();
        let weak = std::sync::Arc::downgrade(&data);
        link.send(AudioCommand::Preview(Some((data.clone(), 0))));
        link.send(AudioCommand::Preview(Some((data, 4))));
        link.poll();
        assert!(weak.upgrade().is_none(), "leaked");
        assert!(link.previews.is_empty());
    }

    #[test]
    fn commands_the_full_queue_hands_back_go_first_and_in_order() {
        let rec = Recorder::default();
        let mut link = started(&rec);
        rec.0.borrow_mut().room = Some(0);
        for key in [52, 53, 54] {
            link.send(note_on(key));
        }
        assert_eq!(link.pending.len(), 3);
        // Room for one: the oldest goes, the rest keeps waiting.
        rec.0.borrow_mut().room = Some(1);
        link.flush();
        assert_eq!(rec.take(), ["NoteOn 1 52 100"]);
        // The rest goes before a newer command.
        rec.0.borrow_mut().room = None;
        link.send(AudioCommand::NoteOff {
            channel: 1,
            key: 54,
        });
        assert_eq!(
            rec.take(),
            ["NoteOn 1 53 100", "NoteOn 1 54 100", "NoteOff 1 54"]
        );
        assert!(link.pending.is_empty());
    }

    #[test]
    fn a_queued_engine_supersedes_the_waiting_engines_params_and_seeks() {
        let rec = Recorder::default();
        let mut link = started(&rec);
        rec.0.borrow_mut().room = Some(0);
        let song = Song::default();
        for tick in [500, 600, 700] {
            fresh(&mut link, &song, true, tick);
            link.send(note_on(52));
            link.send(AudioCommand::SetParam {
                channel: 0,
                instance_id: 0,
                index: 0,
                value: 0.5,
            });
        }
        // Only the newest engine waits, with what came after it; notes and transport keep their order.
        rec.0.borrow_mut().room = None;
        link.flush();
        assert_eq!(
            rec.take(),
            [
                "Play",
                "NoteOn 1 52 100",
                "Play",
                "NoteOn 1 52 100",
                "ReplaceSong",
                "Seek 700",
                "Play",
                "NoteOn 1 52 100",
                "SetParam 0 0 0 0.500"
            ]
        );
    }

    #[test]
    fn a_failed_stream_warns_once_and_drops_pending_and_new_commands() {
        let rec = Recorder::default();
        let mut link = started(&rec);
        rec.0.borrow_mut().room = Some(0);
        link.send(note_on(52));
        assert_eq!(link.pending.len(), 1);
        let before = warnings();
        rec.0.borrow_mut().failed = true;
        for _ in 0..3 {
            assert!(link.poll().is_some_and(|s| s.stream_failed));
        }
        assert_eq!(warnings() - before, 1, "one warning, not one per poll");
        assert!(!link.running());
        assert!(link.pending.is_empty());
        link.send(note_on(53));
        link.replace_song(&Song::default(), true, 0, &|_| None);
        assert!(link.pending.is_empty());
        rec.0.borrow_mut().room = None;
        link.flush();
        assert_eq!(rec.take(), Vec::<String>::new());
    }

    #[test]
    fn a_reopen_takes_the_current_song_and_play_state_and_a_failed_one_keeps_the_output() {
        let rec = Recorder::default();
        let mut link = started(&rec);
        rec.0.borrow_mut().failed = true;
        link.poll();
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);

        rec.0.borrow_mut().no_device = true;
        let before = warnings();
        link.start(&song, true, &|_| None, |_, _| {
            panic!("a failed output is reopened, not opened")
        });
        assert_eq!(warnings() - before, 1);
        assert_eq!(rec.0.borrow().reopens, 1);
        assert!(link.poll().is_some(), "the output was dropped");
        assert!(!link.running());

        rec.0.borrow_mut().no_device = false;
        link.start(&song, true, &|_| None, |_, _| {
            panic!("a failed output is reopened, not opened")
        });
        assert!(link.running());
        assert_eq!(rec.take(), ["Reopen 2", "Play"]);
    }

    #[test]
    fn no_engine_is_built_for_a_failed_stream() {
        // A zero tempo makes the engine build fail, which warns.
        let song = Song {
            tempo_bpm: 0.0,
            ..Song::default()
        };
        let rec = Recorder::default();
        let mut link = started(&rec);
        let before = warnings();
        link.replace_song(&song, false, 0, &|_| None);
        assert_eq!(warnings() - before, 1, "a running link builds and warns");
        rec.0.borrow_mut().failed = true;
        link.poll();
        let before = warnings();
        link.replace_song(&song, false, 0, &|_| None);
        assert_eq!(warnings() - before, 0, "a failed link built an engine");
    }

    #[test]
    fn replacing_the_song_keeps_the_tick_and_resumes_only_while_playing() {
        let rec = Recorder::default();
        let mut link = started(&rec);
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        // Stop is a pause: the stopped position is kept for Play, Slice and the like.
        fresh(&mut link, &song, false, 700);
        assert_eq!(rec.take(), ["ReplaceSong", "Seek 700"]);
        // A new engine starts stopped at tick 0 already.
        fresh(&mut link, &song, false, 0);
        assert_eq!(rec.take(), ["ReplaceSong"]);
        fresh(&mut link, &song, true, 700);
        assert_eq!(rec.take(), ["ReplaceSong", "Seek 700", "Play"]);
        // Without an output there is nothing to rebuild.
        AudioLink::default().replace_song(&song, true, 700, &|_| None);
    }

    #[test]
    fn the_samples_are_decoded_before_each_build_and_once() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        let kick = ContentPath::Plain("kick.wav".into());
        song.channels[1].drum_kit.as_mut().unwrap().slots[0]
            .file
            .path = Some(kick);
        let slots = &song.channels[1].drum_kit.as_ref().unwrap().slots;
        let reads = std::cell::Cell::new(0);
        let read = |_: &ContentPath| {
            reads.set(reads.get() + 1);
            None
        };
        let mut link = AudioLink::default();
        // Nothing is built, so nothing is read, until there is an output.
        link.replace_song(&song, false, 0, &read);
        assert_eq!(reads.get(), 0);
        let rec = Recorder::default();
        link.start(&song, false, &read, |_, samples| {
            assert!(
                samples.slot(&slots[1]).is_some(),
                "the first engine has the kit"
            );
            rec.sink()
        });
        assert_eq!(reads.get(), 1);
        link.replace_song(&song, true, 0, &read);
        link.replace_song(&song, true, 0, &read);
        assert_eq!(reads.get(), 1, "a rebuild read the missing file again");
        assert_eq!(link.samples.missing().count(), 1);
    }

    #[test]
    fn an_engine_for_part_of_the_song_keeps_the_samples_of_the_rest() {
        use std::sync::Arc;

        use crate::flm::chunk::ContentRoot;
        use crate::sample_cache::{SampleKey, tests::wav};
        use crate::song::SamplerSlot;
        use crate::ui::preview::{ClipKey, solo};
        // Two audio clips; Preview plays the first alone (`solo`), the app holds the whole song.
        let user = |name: &str| ContentPath::Shorthand {
            root: ContentRoot::User,
            relative: name.into(),
        };
        let mut whole = Song::default();
        whole.add_channel(ChannelKind::AudioClip);
        let lane = &mut whole.channels[1].lanes[0];
        for (name, start) in [("a.wav", 0), ("b.wav", 1024)] {
            let mut slot = SamplerSlot::default();
            slot.file.path = Some(user(name));
            lane.add_clip(start, 4.0).file.sample = Some(Box::new(slot));
        }
        let clip = ClipKey {
            channel: 1,
            lane: 0,
            id: 1,
        };
        let part = solo(&whole, clip).expect("the clip");
        assert_eq!(part.channels[1].lanes[0].clips.len(), 1);
        let reads = std::cell::Cell::new(0);
        let read = |_: &ContentPath| {
            reads.set(reads.get() + 1);
            Some(wav(&[1, 2, 3, 4]))
        };
        let other = SampleKey::File(user("b.wav"));
        // A stopped link: `start` loads and opens nothing.
        let mut link = AudioLink::default();
        link.start(&whole, false, &read, |_, _| None);
        let before = link.samples.get(&other).expect("both clips load");
        link.start_keeping(&part, &whole, false, &read, |_, _| None);
        let kept = link.samples.get(&other).expect("the other clip's sample");
        assert!(Arc::ptr_eq(&before, &kept), "decoded again");
        // Without `keep` the engine's song is all the song there is: the other clip's sample goes.
        link.start(&part, false, &read, |_, _| None);
        assert!(link.samples.get(&other).is_none());
        // A running link: Preview on and off through `replace_song_keeping`.
        let rec = Recorder::default();
        let mut link = AudioLink::default();
        link.start(&whole, false, &read, |_, _| rec.sink());
        let before = link.samples.get(&other).expect("both clips load");
        assert!(link.replace_song_keeping(&part, &whole, false, 0, &read, Handover::Fresh));
        assert!(link.replace_song_keeping(&whole, &whole, false, 0, &read, Handover::Fresh));
        let after = link.samples.get(&other).expect("still cached");
        assert!(Arc::ptr_eq(&before, &after), "decoded again");
        assert_eq!(reads.get(), 4, "a.wav and b.wav were read once per link");
    }

    #[test]
    fn the_sample_rate_is_the_running_outputs() {
        assert_eq!(AudioLink::default().sample_rate(), None);
        let rec = Recorder::default();
        let mut link = started(&rec);
        assert_eq!(link.sample_rate(), Some(48_000));
        rec.0.borrow_mut().failed = true;
        link.poll();
        assert_eq!(link.sample_rate(), None);
    }

    #[test]
    fn every_new_engine_monitors_the_input_and_gets_the_metronome_again() {
        use honeydew_audio::device::AudioRenderer;
        use honeydew_audio::input::InputRing;
        use std::sync::Arc;

        let rec = Recorder::default();
        let mut link = started(&rec);
        let mut song = Song::default();
        song.add_channel(ChannelKind::AudioRecord);
        song.channels[1].file.input_monitoring = true;
        let ring = Arc::new(InputRing::new(1, 48_000, 48_000));
        link.input = Some(LiveInput::new(ring.clone(), Vec::new()));
        link.send(AudioCommand::Metronome {
            on: true,
            click: true,
            level: 0.5,
        });
        assert_eq!(rec.take(), ["Metronome true 0.500"]);
        fresh(&mut link, &song, false, 0);
        assert_eq!(rec.take(), ["ReplaceSong", "Metronome true 0.500"]);
        // The new engine monitors the ring: Audio Record, monitoring on, the strip at unity.
        ring.push(std::iter::repeat_n(0.5, 256));
        let mut output = [0.0; 512];
        rec.0
            .borrow_mut()
            .engine
            .as_mut()
            .unwrap()
            .render(&mut output, 2);
        assert!(output.iter().all(|s| (s - 0.5).abs() < 1e-3), "{output:?}");
        // The arm goes before Play (a recording's engine does not wrap at the song end), for a fresh engine, a reopened
        // stream's and a first one.
        fresh(&mut link, &song, true, 0);
        let played = ["ReplaceSong", "Metronome true 0.500", "Seek 0", "Play"];
        assert_eq!(rec.take(), played);
        rec.0.borrow_mut().failed = true;
        link.poll();
        link.start(&song, true, &|_| None, |_, _| {
            panic!("reopened, not opened")
        });
        assert_eq!(rec.take(), ["Reopen 2", "Metronome true 0.500", "Play"]);
        let mut fresh = AudioLink::default();
        fresh.send(AudioCommand::Metronome {
            on: false,
            click: true,
            level: 0.25,
        });
        fresh.start(&song, true, &|_| None, |_, _| rec.sink());
        assert_eq!(rec.take(), ["Metronome false 0.250", "Play"]);
    }

    #[test]
    fn a_rebuild_counts_in_again_until_the_count_in_ends_or_stops() {
        use honeydew_audio::device::AudioRenderer;

        let rec = Recorder::default();
        let mut link = started(&rec);
        // 140 BPM: the count-in length below.
        let song = Song {
            tempo_bpm: 140.0,
            ..Song::default()
        };
        // Fresh engines: one that takes over the playing engine keeps its count-in as it is.
        let rebuilt = |link: &mut AudioLink, playing| {
            link.replace_song_keeping(&song, &song, playing, 0, &|_| None, Handover::Fresh);
            rec.take()
        };
        assert_eq!(rebuilt(&mut link, false), ["ReplaceSong"]);
        // Armed, not yet playing: the new engine is armed too.
        link.send(AudioCommand::CountIn(3));
        rec.take();
        assert_eq!(rebuilt(&mut link, false), ["ReplaceSong", "CountIn 3"]);
        // Counting in: the new engine counts in again before it plays.
        link.send(AudioCommand::CountIn(4));
        link.send(AudioCommand::Play);
        rec.take();
        assert!(link.poll().unwrap().counting_in);
        let counting = ["ReplaceSong", "Seek 0", "CountIn 4", "Play"];
        assert_eq!(rebuilt(&mut link, true), counting);
        assert!(link.poll().unwrap().counting_in);
        // The count-in ends (4 beats at 140 BPM: 82 286 frames); once a poll sees it fall, a rebuild just plays.
        let mut output = vec![0.0; 2 * 82_286];
        rec.0
            .borrow_mut()
            .engine
            .as_mut()
            .unwrap()
            .render(&mut output, 2);
        assert!(!link.poll().unwrap().counting_in);
        assert_eq!(rebuilt(&mut link, true), ["ReplaceSong", "Seek 0", "Play"]);
        // Stop and CountIn(0) take an armed count-in back.
        for undo in [AudioCommand::Stop, AudioCommand::CountIn(0)] {
            link.send(AudioCommand::CountIn(2));
            link.send(undo);
            rec.take();
            assert_eq!(rebuilt(&mut link, false), ["ReplaceSong"]);
        }
    }

    #[test]
    fn a_count_in_no_poll_saw_ends_once_the_transport_moves_after_play() {
        let rec = Recorder::default();
        let mut link = started(&rec);
        let song = Song::default();
        rec.0.borrow_mut().tick = 640;
        link.poll();
        // Armed, then played: no engine runs it, so no snapshot ever shows it (a count-in shorter than a frame).
        link.send(AudioCommand::CountIn(4));
        assert!(link.counting_in(), "armed");
        // A seek before the Play: still held there.
        link.send(AudioCommand::Seek(640));
        link.send(AudioCommand::Play);
        rec.take();
        // The transport has not moved: the count-in may still be on its way.
        assert!(!link.poll().unwrap().counting_in);
        assert!(link.counting_in());
        // It moved with no count-in seen: the count-in is over, and a rebuild just plays.
        rec.0.borrow_mut().tick = 700;
        link.poll();
        assert!(!link.counting_in());
        fresh(&mut link, &song, true, 700);
        assert_eq!(rec.take(), ["ReplaceSong", "Seek 700", "Play"]);
        // An armed count-in with no Play yet stays, however the stopped transport moves (a ruler tap seeks).
        link.send(AudioCommand::Stop);
        link.send(AudioCommand::CountIn(2));
        rec.0.borrow_mut().tick = 0;
        link.poll();
        assert!(link.counting_in());
    }

    /// A song whose channel 1 is a MiniSynth (instance 2) with a Multiband (instance 3) and a Pitcher (instance 4).
    fn multiband_song() -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let rack = &mut song.channels[1].rack;
        assert_eq!((rack.add(36), rack.add(25)), (3, 4));
        song
    }

    fn solo(channel: usize, band: f32) -> Command {
        AudioCommand::SetSetting {
            channel,
            instance_id: 3,
            key: 38,
            value: band,
        }
    }

    #[test]
    fn every_new_engine_gets_the_screen_only_settings_again() {
        // A Multiband's soloed band (key 38) is on the screen and in the engine, not in the song.
        let rec = Recorder::default();
        let mut link = started(&rec);
        let song = multiband_song();
        link.replace_song(&song, false, 0, &|_| None);
        link.send(solo(1, 2.0));
        // A Pitcher note is in the song's blob: the new engine reads it there.
        link.send(AudioCommand::SetSetting {
            channel: 1,
            instance_id: 4,
            key: 9,
            value: 1.0,
        });
        rec.take();
        fresh(&mut link, &song, true, 0);
        let resent = ["ReplaceSong", "Seek 0", "Play", "SetSetting 1 3 38 2.000"];
        assert_eq!(rec.take(), resent);
        // An edit's engine gets them too: a module it builds afresh needs them, one it takes over has them already.
        link.replace_song(&song, true, 0, &|_| None);
        let resent = [
            "ReplaceSong adopting no-resync",
            "Play",
            "SetSetting 1 3 38 2.000",
        ];
        assert_eq!(rec.take(), resent);
        // The latest value; a reopened stream's engine gets it too.
        link.send(solo(1, 3.0));
        rec.take();
        rec.0.borrow_mut().failed = true;
        link.poll();
        link.start(&song, false, &|_| None, |_, _| panic!("reopened"));
        assert_eq!(rec.take(), ["Reopen 2", "Stop", "SetSetting 1 3 38 3.000"]);
        // Solo off goes again too.
        link.send(solo(1, 0.0));
        rec.take();
        link.replace_song(&song, false, 0, &|_| None);
        assert_eq!(
            rec.take(),
            ["ReplaceSong adopting no-resync", "SetSetting 1 3 38 0.000"]
        );
    }

    #[test]
    fn a_screen_only_setting_of_a_module_that_is_gone_is_dropped() {
        let rec = Recorder::default();
        let mut link = started(&rec);
        let mut song = multiband_song();
        link.replace_song(&song, false, 0, &|_| None);
        link.send(solo(1, 1.0));
        rec.take();
        // The Multiband deleted: nothing goes out, and nothing once it is back (its channel's delete undone).
        let before = song.clone();
        song.channels[1].rack.modules.retain(|m| m.instance_id != 3);
        link.replace_song(&song, false, 0, &|_| None);
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync"]);
        link.replace_song(&before, false, 0, &|_| None);
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync"]);
        // Its channel deleted.
        link.send(solo(1, 1.0));
        rec.take();
        song = before.clone();
        song.channels.remove(1);
        link.replace_song(&song, false, 0, &|_| None);
        assert_eq!(rec.take(), ["ReplaceSong adopting"]);
        // Another module type at its instance id is not a Multiband.
        link.replace_song(&before, false, 0, &|_| None);
        link.send(solo(1, 1.0));
        rec.take();
        song = before.clone();
        *song.channels[1].rack.get_mut(3).unwrap() = crate::song::ModuleInstance::new(25, 3);
        link.replace_song(&song, false, 0, &|_| None);
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync"]);
    }

    #[test]
    fn a_song_edit_while_the_stream_is_down_rekeys_the_screen_only_settings() {
        // A channel inserted while the stream is down moves the Multiband to channel 2; a solo sent then is keyed by
        // The new song, and the reopened engine gets it there.
        let rec = Recorder::default();
        let mut link = started(&rec);
        let mut song = multiband_song();
        link.replace_song(&song, false, 0, &|_| None);
        link.send(solo(1, 2.0));
        rec.0.borrow_mut().failed = true;
        link.poll();
        song.insert_channel(1, ChannelKind::Synth(SynthKind::MiniSynth));
        assert!(!link.replace_song(&song, false, 0, &|_| None));
        link.send(solo(2, 3.0));
        rec.take();
        link.start(&song, false, &|_| None, |_, _| panic!("reopened"));
        assert_eq!(rec.take(), ["Reopen 3", "Stop", "SetSetting 2 3 38 3.000"]);
    }

    #[test]
    fn a_screen_only_setting_follows_its_module_when_the_channels_move() {
        let rec = Recorder::default();
        let mut link = started(&rec);
        let mut song = multiband_song();
        link.replace_song(&song, false, 0, &|_| None);
        link.send(solo(1, 2.0));
        rec.take();
        // A channel inserted before it: the Multiband's channel is 2 now, in the new engine and in later sends.
        song.insert_channel(1, ChannelKind::Synth(SynthKind::MiniSynth));
        link.replace_song(&song, false, 0, &|_| None);
        let moved = ["ReplaceSong adopting no-resync", "SetSetting 2 3 38 2.000"];
        assert_eq!(rec.take(), moved);
        link.send(solo(2, 1.0));
        rec.take();
        // Moved to another slot of its rack: the same module.
        song.channels[2].rack.modules.swap(1, 2);
        link.replace_song(&song, false, 0, &|_| None);
        let swapped = ["ReplaceSong adopting no-resync", "SetSetting 2 3 38 1.000"];
        assert_eq!(rec.take(), swapped);
        // The channel before it deleted: back to 1.
        song.channels.remove(1);
        link.replace_song(&song, false, 0, &|_| None);
        assert_eq!(
            rec.take(),
            ["ReplaceSong adopting", "SetSetting 1 3 38 1.000"]
        );
    }

    /// Mute or Unmute (`CLHd`@36) re-syncs the playing engine rather than restarting it, so the new engine adopts the
    /// old one (its transport too: no Seek); while stopped Mute only sets the flag. So does any other edit, re-syncing
    /// only where it says.
    #[test]
    fn a_mute_or_unmute_hands_the_song_to_an_adopting_engine() {
        let rec = Recorder::default();
        let mut link = started(&rec);
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        song.channels[1].lanes[0].add_clip(0, 4.0);
        link.replace_song(&song, true, 700, &|_| None);
        rec.take();
        song.channels[1].lanes[0].clips[0].muted = true;
        link.replace_song(&song, true, 700, &|_| None);
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        song.channels[1].lanes[0].clips[0].muted = false;
        // The app counts the editing time on every edit: `META`, which no engine reads.
        song.file.meta.editing_seconds += 1.0;
        link.replace_song(&song, false, 700, &|_| None);
        assert_eq!(rec.take(), ["ReplaceSong adopting"]);
        // The same song again changes nothing; a structural edit with it (an effect added) re-syncs for the Mute.
        link.replace_song(&song, true, 700, &|_| None);
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
        song.channels[1].lanes[0].clips[0].muted = true;
        song.channels[1].rack.add(32);
        link.replace_song(&song, true, 700, &|_| None);
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
    }

    /// The left side of `engine`'s next `frames` frames.
    fn render(engine: &mut SongAudioEngine, frames: usize) -> Vec<f32> {
        let mut output = vec![0.0; frames * 2];
        engine.render(&mut output, 2);
        output.into_iter().step_by(2).collect()
    }

    /// [`render`] of the recorder's engine.
    fn rendered(rec: &Recorder, frames: usize) -> Vec<f32> {
        render(rec.0.borrow_mut().engine.as_mut().unwrap(), frames)
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    /// A sample rewritten under its path (the audio editor's `Replace`) is heard from the next edit's engine, never its
    /// old data: a re-sync (`SongEdit::Resync`'s handover) relaunches the clip with the new sample; an edit without one
    /// leaves the old clip voice behind.
    #[test]
    fn a_sample_rewritten_under_its_path_is_heard_and_its_old_data_stops() {
        use std::cell::Cell;

        use crate::sample_cache::{SampleKey, tests::wav};
        use crate::song::SamplerSlot;
        let path = ContentPath::Plain("a.wav".into());
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::AudioClip);
        let mut slot = SamplerSlot::default();
        slot.file.path = Some(path.clone());
        song.channels[1].lanes[0].add_clip(0, 16.0).file.sample = Some(Box::new(slot));
        let level = Cell::new(1000);
        let read = |_: &ContentPath| Some(wav(&vec![level.get(); 8 * 44_100]));
        let rec = Recorder::default();
        let mut link = started(&rec);
        assert!(link.replace_song(&song, true, 0, &read));
        let before = peak(&rendered(&rec, 24_000));
        assert!(before > 0.01, "a silent clip");
        // Eight times louder, then an edit that changes nothing in the song but re-syncs.
        level.set(8000);
        link.samples.forget(&SampleKey::File(path.clone()));
        rec.take();
        let resync = Handover::Undo;
        assert!(link.replace_song_keeping(&song, &song, true, 0, &read, resync));
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        let ratio = peak(&rendered(&rec, 4800)) / before;
        assert!((ratio - 8.0).abs() < 0.01, "{ratio}");
        // Rewritten again; an edit without a re-sync: the clip voice playing the old data stays with the old engine.
        level.set(2000);
        link.samples.forget(&SampleKey::File(path));
        assert!(link.replace_song(&song, true, 0, &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
        assert_eq!(peak(&rendered(&rec, 4800)), 0.0);
    }

    /// MIDI CC #7 sets the Level, CC #8 and #10 the Pan, and the later write wins: the Level or Pan knob moved after
    /// The CC is what an edit's engine plays, not the CC kept as a live controller (before: 30 dB down). With no knob
    /// moved, the CC's Level or Pan plays on through the edit; only [`AudioLink::keep_controllers`] keeps it (the
    /// engine's `keep_ccs` keeps CCs 7–133, not params 0 and 1).
    #[test]
    fn a_level_or_pan_knob_moved_after_its_cc_wins_through_an_edit() {
        use crate::song::{Event, PPQ};
        let note = |beat: u32, key: u8| Event::Note {
            pos: beat * PPQ,
            len_beats: 8.0,
            key,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        };
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        for _ in 0..2 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        for (channel, beat, key) in [(1, 0, 57), (2, 6, 64)] {
            let clip = song.channels[channel].lanes[0].add_clip(0, 16.0);
            clip.events.push(note(beat, key));
        }
        // Each CC, then its knob if moved: the Level, then the Pan hard left after CC #10 or #8 put it hard right.
        let knobs = [
            (14, 0, 0.3, Some(0.9)),
            (17, 1, 1.0, Some(0.0)),
            (15, 1, 1.0, Some(0.0)),
        ];
        // Then each CC alone: the Level, then the Pan hard left (the reference is the left side).
        let ccs_only = [(14, 0, 0.3, None), (17, 1, 0.0, None), (15, 1, 0.0, None)];
        for (cc, knob, sent, moved) in knobs.into_iter().chain(ccs_only) {
            let rec = Recorder::default();
            let mut link = started(&rec);
            assert!(link.replace_song(&song, true, 0, &|_| None));
            // An engine that gets the same writes and sees no edit.
            let mut heard = SongAudioEngine::new(&song, 48_000.0).unwrap();
            heard.play();
            render(&mut heard, 24_000);
            rendered(&rec, 24_000);
            let mut knobbed = song.clone();
            if let Some(moved) = moved {
                knobbed.channels[1].params.0[knob as usize] = moved;
            }
            let writes = [Some((cc, sent)), moved.map(|moved| (knob, moved))];
            for (index, value) in writes.into_iter().flatten() {
                link.send(AudioCommand::SetParam {
                    channel: 1,
                    instance_id: 0,
                    index,
                    value,
                });
                heard.set_param(1, 0, index, value);
            }
            // A graph-lane edit on the other channel (its note's velocity): no re-sync.
            let edited = &mut knobbed.channels[2].lanes[0].clips[0].events[0];
            if let Event::Note { velocity, .. } = edited {
                *velocity = 100;
            }
            rec.take();
            assert!(link.replace_song(&knobbed, true, 0, &|_| None));
            assert_eq!(
                rec.take(),
                ["ReplaceSong adopting no-resync", "Play"],
                "{cc}"
            );
            let (actual, expected) = (rendered(&rec, 24_000), render(&mut heard, 24_000));
            assert!(peak(&expected) > 0.01, "{cc}: a silent reference");
            for (frame, (a, e)) in actual.iter().zip(&expected).enumerate() {
                assert!((a - e).abs() <= 1e-5, "{cc}: frame {frame}: {a} vs {e}");
            }
        }
    }

    /// A waiting engine is replaced in place only while nothing that names one of its song's channels waits behind it.
    /// Four synths, the queue full: an edit, a key down on channel 3, channel 2 deleted (3 is now 2), then the key up
    /// on channel 2 reaches the voice the key down started.
    #[test]
    fn the_commands_behind_a_waiting_engine_stay_behind_it() {
        let mut song = Song::default();
        for _ in 0..4 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        let rec = Recorder::default();
        let mut link = started(&rec);
        assert!(link.replace_song(&song, true, 0, &|_| None));
        rec.0.borrow_mut().room = Some(0);
        let mut edited = song.clone();
        edited.channels[1].rack.add(32);
        assert!(link.replace_song(&edited, true, 0, &|_| None));
        link.send(AudioCommand::NoteOn {
            channel: 3,
            key: 60,
            velocity: 1.0,
        });
        edited.delete_channel(2);
        assert!(link.replace_song(&edited, true, 0, &|_| None));
        rec.0.borrow_mut().room = None;
        link.flush();
        let sounding = |channel| rec.0.borrow().engine.as_ref().unwrap().sounding(channel);
        assert_eq!((sounding(2), sounding(3)), (1 << 60, 0));
        link.send(AudioCommand::NoteOff {
            channel: 2,
            key: 60,
        });
        assert_eq!(sounding(2), 0);
    }

    /// Every command is small, an engine's too (it is boxed): the commands that pile up while the queue stays full are
    /// no engine's size each.
    #[test]
    fn a_command_is_small() {
        assert!(size_of::<Command>() <= 64, "{} bytes", size_of::<Command>());
    }

    /// What the app sends again after every engine (the live lane, a screen-only setting) does not pin a waiting
    /// engine: the next edit's replaces it in place and they go out again for it, so a queue that stays full holds one
    /// engine.
    #[test]
    fn a_live_lane_or_screen_only_setting_behind_a_waiting_engine_gives_way() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let synth = song.channels[1].rack.modules[0].instance_id;
        let rec = Recorder::default();
        let mut link = started(&rec);
        assert!(link.replace_song(&song, true, 0, &|_| None));
        rec.0.borrow_mut().room = Some(0);
        let mut edited = song.clone();
        edited.channels[1].rack.add(32);
        assert!(link.replace_song(&edited, true, 0, &|_| None));
        link.send(AudioCommand::LiveLane {
            channel: 1,
            notes: true,
        });
        link.send(AudioCommand::SetSetting {
            channel: 1,
            instance_id: synth,
            key: honeydew_audio::instrument::KEY_LEARN,
            value: 1.0,
        });
        // A channel added before it: the setting goes out again at its new index.
        edited.insert_channel(1, ChannelKind::Synth(SynthKind::MiniSynth));
        assert!(link.replace_song(&edited, true, 0, &|_| None));
        rec.0.borrow_mut().room = None;
        rec.take();
        link.flush();
        let learn = format!("SetSetting 2 {synth} {} 1.000", u32::MAX);
        let sent = [
            "ReplaceSong adopting no-resync",
            "Play",
            "Play",
            learn.as_str(),
        ];
        assert_eq!(rec.take(), sent);
    }
}

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering, fence};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SupportedBufferSize, SupportedStreamConfig, StreamConfig};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};

use crate::MeterSnapshot;
use crate::sample::SampleData;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleKey {
    pub channel: usize,
    pub instance_id: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModuleMeter {
    pub key: ModuleKey,
    pub value: MeterSnapshot,
    /// The module's analysis as last published ([`AudioRenderer::analysis`]); empty for a module without one.
    pub analysis: Vec<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputSnapshot {
    pub transport_tick: u64,
    pub cpu_load: f32,
    /// The callbacks since the stream started that took longer than their buffer lasts: each one an underrun.
    pub underruns: u64,
    pub module_meters: Vec<ModuleMeter>,
    pub stream_failed: bool,
    /// A count-in holds the transport ([`AudioRenderer::counting_in`]).
    pub counting_in: bool,
    /// The latency calibration's last measurement ([`AudioRenderer::ping_report`]).
    pub ping: Option<PingReport>,
    /// By channel, the keys sounding at its input as the last render left them ([`AudioRenderer::sounding`]).
    pub sounding: Vec<u128>,
}

/// Why a latency calibration failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PingFailure {
    /// Input louder than 0.1 in the second second of silence.
    Noise,
    /// No input louder than 0.2 within a second of tone.
    Timeout,
    /// No input to listen to.
    Unknown,
}

/// A latency measurement ([`AudioRenderer::ping`]): the number it was started with, and its result once it has one,
/// the round trip in frames or why there is none.
pub type PingReport = (u32, Option<Result<u32, PingFailure>>);

/// Output buffer policy. Native keeps the backend's default; the other modes trade latency for stability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferMode {
    UltraSafe,
    Safe,
    Fast,
    /// Requests a target frame count, clamped to the device's supported range.
    Fixed(u32),
    Native,
}

pub trait AudioRenderer: Send + 'static {
    fn render(&mut self, output: &mut [f32], channels: usize);
    /// Enables independent-channel processing if this renderer supports it.
    fn set_multi_core(&mut self, _on: bool) {}
    fn set_param(&mut self, channel: usize, instance_id: i32, index: i32, value: f32);
    fn play(&mut self);
    fn stop(&mut self);
    fn seek(&mut self, tick: u64);
    fn note_on(&mut self, channel: usize, key: u8, velocity: f32);
    fn note_off(&mut self, channel: usize, key: u8);
    /// Loops the transport over song ticks [start, end) while playing: a playhead reaching or past the end goes on
    /// from the start, sample-accurately. `None`, or an empty or inverted range, plays on unlooped.
    fn set_loop(&mut self, range: Option<(u64, u64)>);
    /// Releases every voice of every instrument; their releases ring on and the transport keeps going.
    fn all_notes_off(&mut self);
    /// A sound reset: every voice and effect silenced at once and its state cleared, params and transport kept (REV
    /// at the play-start point). The default does nothing.
    fn reset_sound(&mut self) {}
    /// Hands module `instance_id` of `channel` its `CSTM` setting `key`.
    fn set_setting(&mut self, channel: usize, instance_id: i32, key: u32, value: f32);
    /// Whether `channel`'s current lane is a notes lane, which routes its live keys.
    fn set_live_lane(&mut self, _channel: usize, _notes: bool) {}
    /// Called on the audio thread when this renderer replaces `old` (`ReplaceSong`), before `old` is retired: a
    /// renderer may take over what `old` is playing. The default takes nothing.
    fn take_over(&mut self, _old: &mut Self) {}
    /// Its [`Self::take_over`] carries over the keys sounding (an edit's engine): until it first renders, the output
    /// reads them from the renderer it replaces. The default carries nothing.
    fn adopts(&self) -> bool {
        false
    }
    /// The next Play first counts in `beats` beats of metronome with the transport held; 0 takes it back. Any Play
    /// takes it, so send it right before the Play it is for.
    fn count_in(&mut self, beats: u32);
    /// The metronome clicks on the song's beats while `on` and playing; `level` is the normalized `Metronome level`
    /// setting, which the count-in uses too.
    fn set_metronome(&mut self, on: bool, level: f32);
    /// The metronome flag (the tempo dialog's toggle): the song's beats click while it is on and the transport plays,
    /// whatever [`Self::set_metronome`] says; `level` as there. The default ignores it.
    fn set_metronome_flag(&mut self, _on: bool, _level: f32) {}
    /// Starts latency measurement number `id`: while it runs, the output
    /// is its test signal alone.
    fn ping(&mut self, id: u32);
    /// A count-in is holding the transport.
    fn counting_in(&self) -> bool;
    /// The last [`Self::ping`] and its result.
    fn ping_report(&self) -> Option<PingReport>;
    /// The stream's audio buffer size in frames: its first callback's, fixed for the stream's life (the latency L
    /// without a calibration). Given before every render call. The default
    /// ignores it.
    fn set_buffer_frames(&mut self, _frames: u32) {}
    fn transport_tick(&self) -> u64;
    fn meter_count(&self) -> usize;
    fn meter_key(&self, index: usize) -> ModuleKey;
    fn meter(&self, index: usize) -> MeterSnapshot;
    /// Meter `index`'s module's analysis now (`crate::analysis`), whose length is fixed while the renderer lives: the
    /// device sizes its slots from it once. Empty for a module without one.
    fn analysis(&self, _index: usize) -> &[f32] {
        &[]
    }
    /// Plays `sample` once from frame `from`, past the master, replacing any preview playing; `None` stops it (the
    /// pickers' sample preview). The default plays nothing.
    fn preview(&mut self, _sample: Option<(Arc<SampleData>, usize)>) {}
    /// The keys sounding at `channel`'s input (bit k = MIDI k): a note-on that reaches the channel before its note
    /// effects sets its key, the note-off clears it. The default: none.
    fn sounding(&self, _channel: usize) -> u128 {
        0
    }
    /// The tempo is now `bpm` from the next rendered frame, the song position kept in beats (a tempo drag): what plays
    /// goes on. The default ignores it.
    fn set_tempo(&mut self, _bpm: f32) {}
    /// Channel param `index` of `channel` set by a live controller no song holds (MIDI Pitch Bend, channel pressure):
    /// as [`Self::set_param`], but never the value a re-sync or Stop puts back. The default is `set_param`.
    fn set_live_param(&mut self, channel: usize, index: i32, value: f32) {
        self.set_param(channel, 0, index, value);
    }
}

pub enum AudioCommand<R> {
    SetMultiCore(bool),
    SetParam {
        channel: usize,
        instance_id: i32,
        index: i32,
        value: f32,
    },
    Play,
    Stop,
    Seek(u64),
    /// Boxed, so that every command is small however large the renderer; it moves to the audio thread and back to
    /// be dropped without an allocation or a free there.
    ReplaceSong(Box<R>),
    NoteOn {
        channel: usize,
        key: u8,
        velocity: f32,
    },
    NoteOff {
        channel: usize,
        key: u8,
    },
    /// [`AudioRenderer::set_loop`].
    SetLoop(Option<(u64, u64)>),
    /// [`AudioRenderer::all_notes_off`].
    AllNotesOff,
    /// [`AudioRenderer::reset_sound`].
    ResetSound,
    /// [`AudioRenderer::set_setting`].
    SetSetting {
        channel: usize,
        instance_id: i32,
        key: u32,
        value: f32,
    },
    /// [`AudioRenderer::count_in`].
    CountIn(u32),
    /// [`AudioRenderer::set_metronome`].
    Metronome {
        on: bool,
        level: f32,
    },
    /// [`AudioRenderer::set_metronome_flag`].
    MetronomeFlag {
        on: bool,
        level: f32,
    },
    /// [`AudioRenderer::ping`].
    Ping(u32),
    /// [`AudioRenderer::set_live_lane`].
    LiveLane {
        channel: usize,
        notes: bool,
    },
    /// [`AudioRenderer::preview`].
    Preview(Option<(Arc<SampleData>, usize)>),
    /// [`AudioRenderer::set_tempo`].
    SetTempo(f32),
    /// [`AudioRenderer::set_live_param`].
    LiveParam {
        channel: usize,
        index: i32,
        value: f32,
    },
}

enum CallbackCommand<R> {
    Control(AudioCommand<R>),
    Replace(Box<R>, Arc<MeterBank>),
}

struct MeterCell {
    key: ModuleKey,
    left: AtomicU32,
    right: AtomicU32,
    reduction: AtomicU32,
    input: AtomicU32,
    /// The analysis's f32 bits, and its seqlock version: odd while the audio thread writes them.
    analysis: Box<[AtomicU32]>,
    version: AtomicU32,
}

/// The meters, by channel (one past the meters' highest) the low and high halves of the keys sounding at its input
/// ([`AudioRenderer::sounding`]), and whether the audio thread has published those keys yet.
struct MeterBank(Vec<MeterCell>, Box<[[AtomicU64; 2]]>, AtomicBool);

/// Set on a magnitude held since the last [`take`] (a magnitude's sign bit is free).
const FRESH: u32 = 1 << 31;

/// Keeps the largest magnitude since the last [`take`]; f32 bit patterns of non-negative values order as the values,
/// and any fresh one is above the last value read.
fn hold(cell: &AtomicU32, value: f32) {
    cell.fetch_max(value.abs().to_bits() | FRESH, Ordering::AcqRel);
}

/// The largest magnitude held since the last read, which starts the next hold; with none held since, the last read's
/// again (a frame between two callbacks reads no 0, so a steady tone's meter does not flicker).
fn take(cell: &AtomicU32) -> f32 {
    f32::from_bits(cell.fetch_and(!FRESH, Ordering::AcqRel) & !FRESH)
}

impl MeterCell {
    /// The audio thread's write of `values` (a seqlock: one writer, which never waits). Extra values are dropped.
    fn publish(&self, values: &[f32]) {
        if self.analysis.is_empty() {
            return;
        }
        let version = self.version.load(Ordering::Relaxed);
        self.version
            .store(version.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        for (slot, value) in self.analysis.iter().zip(values) {
            slot.store(value.to_bits(), Ordering::Relaxed);
        }
        self.version
            .store(version.wrapping_add(2), Ordering::Release);
    }

    /// The last whole analysis published. A read the writer overlaps is retried, yielding after the first few tries
    /// (a writer the OS preempted mid-write needs the CPU back). After 1024 tries in a row, never in practice, the last
    /// read stands, at worst a mix of two consecutive analyses: a display glitch, not a hang.
    fn analysis(&self) -> Vec<f32> {
        let mut values = vec![0.0; self.analysis.len()];
        for attempt in 0..1024 {
            let before = self.version.load(Ordering::Acquire);
            for (value, slot) in values.iter_mut().zip(&self.analysis) {
                *value = f32::from_bits(slot.load(Ordering::Relaxed));
            }
            fence(Ordering::Acquire);
            if before.is_multiple_of(2) && self.version.load(Ordering::Relaxed) == before {
                break;
            }
            #[cfg(not(target_arch = "wasm32"))]
            if attempt >= 8 {
                std::thread::yield_now();
            }
            #[cfg(target_arch = "wasm32")]
            let _ = attempt;
        }
        values
    }
}

impl MeterBank {
    fn new<R: AudioRenderer>(renderer: &R) -> Self {
        let channels = (0..renderer.meter_count())
            .map(|index| renderer.meter_key(index).channel + 1)
            .max()
            .unwrap_or(0);
        Self(
            (0..renderer.meter_count())
                .map(|index| MeterCell {
                    key: renderer.meter_key(index),
                    left: AtomicU32::new(0),
                    right: AtomicU32::new(0),
                    reduction: AtomicU32::new(0),
                    input: AtomicU32::new(0),
                    // At rest until the first render: the new renderer's own values.
                    analysis: (renderer.analysis(index).iter())
                        .map(|v| AtomicU32::new(v.to_bits()))
                        .collect(),
                    version: AtomicU32::new(0),
                })
                .collect(),
            (0..channels).map(|_| Default::default()).collect(),
            AtomicBool::new(false),
        )
    }

    /// Each meter starting at `old`'s last values under its key (an edit's replace reads no 0 until the new
    /// renderer's first callback). On the app thread, before the bank is shared.
    fn seeded(self, old: &MeterBank) -> Self {
        for cell in &self.0 {
            let Some(from) = old.0.iter().find(|from| from.key == cell.key) else {
                continue;
            };
            let pairs = [
                (&cell.left, &from.left),
                (&cell.right, &from.right),
                (&cell.reduction, &from.reduction),
                (&cell.input, &from.input),
            ];
            for (to, from) in pairs {
                to.store(from.load(Ordering::Acquire) & !FRESH, Ordering::Relaxed);
            }
        }
        self
    }

    /// The audio thread's write of each channel's sounding keys. The halves are stored one after the other: a read
    /// between them mixes two renders' keys for one frame, a display glitch the next read mends.
    fn publish_sounding<R: AudioRenderer>(&self, renderer: &R) {
        for (channel, [low, high]) in self.1.iter().enumerate() {
            let keys = renderer.sounding(channel);
            low.store(keys as u64, Ordering::Release);
            high.store((keys >> 64) as u64, Ordering::Release);
        }
        self.2.store(true, Ordering::Release);
    }

    /// The audio thread has published the sounding keys at least once.
    fn published(&self) -> bool {
        self.2.load(Ordering::Acquire)
    }

    /// Each channel's sounding keys as last published; a read takes nothing.
    fn sounding(&self) -> Vec<u128> {
        (self.1.iter())
            .map(|[low, high]| {
                u128::from(low.load(Ordering::Acquire))
                    | u128::from(high.load(Ordering::Acquire)) << 64
            })
            .collect()
    }

    /// Every meter's peaks and gain reduction since the last read (item 18: a peak between two reads is not lost), or
    /// the last read's with no render since, and each analysis as last published.
    fn take(&self) -> Vec<ModuleMeter> {
        (self.0.iter())
            .map(|cell| ModuleMeter {
                key: cell.key,
                value: MeterSnapshot {
                    peak_left: take(&cell.left),
                    peak_right: take(&cell.right),
                    gain_reduction_db: take(&cell.reduction),
                    input_peak: take(&cell.input),
                },
                analysis: cell.analysis(),
            })
            .collect()
    }
}

#[derive(Default)]
struct Shared {
    tick: AtomicU64,
    cpu: AtomicU32,
    underruns: AtomicU64,
    failed: AtomicBool,
    counting_in: AtomicBool,
    /// The last [`PingReport`], packed ([`pack`]); 0 = none.
    ping: AtomicU64,
    /// [`AudioRenderer::set_buffer_frames`]: the first callback's frames, 0 before it.
    buffer_frames: AtomicU32,
}

/// A [`PingReport`] in one word, so a reader never sees half of one: the id above bit 32, then a state byte (1 running,
/// 2 done, 3 noise, 4 timeout, 5 unknown error), then the frames (< 2^24, under a second at any rate).
fn pack((id, result): PingReport) -> u64 {
    let (state, frames): (u8, u32) = match result {
        None => (1, 0),
        Some(Ok(frames)) => (2, frames.min(0xFF_FFFF)),
        Some(Err(PingFailure::Noise)) => (3, 0),
        Some(Err(PingFailure::Timeout)) => (4, 0),
        Some(Err(PingFailure::Unknown)) => (5, 0),
    };
    u64::from(id) << 32 | u64::from(state) << 24 | u64::from(frames)
}

fn unpack(word: u64) -> Option<PingReport> {
    let (id, frames) = ((word >> 32) as u32, (word & 0xFF_FFFF) as u32);
    let result = match (word >> 24) & 0xFF {
        1 => None,
        2 => Some(Ok(frames)),
        3 => Some(Err(PingFailure::Noise)),
        4 => Some(Err(PingFailure::Timeout)),
        5 => Some(Err(PingFailure::Unknown)),
        _ => return None,
    };
    Some((id, result))
}

impl Shared {
    /// The renderer's count-in and latency measurement after a render call.
    fn publish<R: AudioRenderer>(&self, renderer: &R) {
        self.counting_in
            .store(renderer.counting_in(), Ordering::Release);
        let ping = renderer.ping_report().map_or(0, pack);
        self.ping.store(ping, Ordering::Release);
    }

    fn counting_in(&self) -> bool {
        self.counting_in.load(Ordering::Acquire)
    }

    fn ping(&self) -> Option<PingReport> {
        unpack(self.ping.load(Ordering::Acquire))
    }
}

#[derive(Debug)]
pub enum StartError<E> {
    NoDevice,
    Config(cpal::DefaultStreamConfigError),
    Renderer(E),
    Build(cpal::BuildStreamError),
    Play(cpal::PlayStreamError),
    UnsupportedSampleFormat,
}

pub struct AudioOutput<R: AudioRenderer> {
    stream: Option<cpal::Stream>,
    sender: Sender<CallbackCommand<R>>,
    retired: Receiver<(Box<R>, Arc<MeterBank>)>,
    shared: Arc<Shared>,
    meters: Arc<MeterBank>,
    /// The last published bank an adopting renderer replaced: its sounding keys stand for the new bank's until that
    /// one first publishes (the keys are carried over, `AudioRenderer::adopts`).
    replaced: Option<Arc<MeterBank>>,
    sample_rate: u32,
    transport_playing: bool,
    buffer_mode: BufferMode,
}

impl<R: AudioRenderer> AudioOutput<R> {
    pub fn start<F, E>(make_renderer: F) -> Result<Self, StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        Self::start_with_mode(BufferMode::Native, make_renderer)
    }

    /// Starts the output, preferring the smallest buffer the device advertises when `low_latency` is on.
    pub fn start_with_latency<F, E>(low_latency: bool, make_renderer: F) -> Result<Self, StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        Self::start_with_mode(
            if low_latency { BufferMode::Fast } else { BufferMode::Native },
            make_renderer,
        )
    }

    /// Starts the output with an explicit buffer profile.
    pub fn start_with_mode<F, E>(mode: BufferMode, make_renderer: F) -> Result<Self, StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(StartError::NoDevice)?;
        let supported = device.default_output_config().map_err(StartError::Config)?;
        let sample_rate = supported.sample_rate().0;
        let channels = usize::from(supported.channels());
        let renderer = make_renderer(sample_rate).map_err(StartError::Renderer)?;
        let meters = Arc::new(MeterBank::new(&renderer));
        let shared = Arc::new(Shared {
            tick: AtomicU64::new(renderer.transport_tick()),
            ..Shared::default()
        });
        let (sender, receiver) = bounded(1024);
        let (retired_sender, retired) = bounded(1024);
        let state = CallbackState {
            renderer: Box::new(renderer),
            meters: meters.clone(),
            shared: shared.clone(),
            receiver,
            retired_sender,
            pending: None,
        };
        let error_shared = shared.clone();
        let error = move |_| {
            error_shared.failed.store(true, Ordering::Release);
        };
        let config = output_config(&device, &supported, mode);
        let stream = match supported.sample_format() {
            cpal::SampleFormat::I8 => {
                build_stream::<i8, _, _>(&device, &config, channels, state, error)
            }
            cpal::SampleFormat::I16 => {
                build_stream::<i16, _, _>(&device, &config, channels, state, error)
            }
            cpal::SampleFormat::I24 => {
                build_stream::<cpal::I24, _, _>(&device, &config, channels, state, error)
            }
            cpal::SampleFormat::I32 => {
                build_stream::<i32, _, _>(&device, &config, channels, state, error)
            }
            cpal::SampleFormat::I64 => {
                build_stream::<i64, _, _>(&device, &config, channels, state, error)
            }
            cpal::SampleFormat::U8 => {
                build_stream::<u8, _, _>(&device, &config, channels, state, error)
            }
            cpal::SampleFormat::U16 => {
                build_stream::<u16, _, _>(&device, &config, channels, state, error)
            }
            cpal::SampleFormat::U32 => {
                build_stream::<u32, _, _>(&device, &config, channels, state, error)
            }
            cpal::SampleFormat::U64 => {
                build_stream::<u64, _, _>(&device, &config, channels, state, error)
            }
            cpal::SampleFormat::F32 => {
                build_stream::<f32, _, _>(&device, &config, channels, state, error)
            }
            cpal::SampleFormat::F64 => {
                build_stream::<f64, _, _>(&device, &config, channels, state, error)
            }
            _ => return Err(StartError::UnsupportedSampleFormat),
        }
        .map_err(StartError::Build)?;
        stream.play().map_err(StartError::Play)?;
        Ok(Self {
            stream: Some(stream),
            sender,
            retired,
            shared,
            meters,
            replaced: None,
            sample_rate,
            transport_playing: false,
            buffer_mode: mode,
        })
    }

    pub fn reopen<F, E>(&mut self, make_renderer: F) -> Result<(), StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        self.reopen_with_mode(self.buffer_mode, make_renderer)
    }

    /// Reopens the output with the requested buffer policy, preserving transport state.
    pub fn reopen_with_latency<F, E>(&mut self, low_latency: bool, make_renderer: F) -> Result<(), StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        self.reopen_with_mode(
            if low_latency { BufferMode::Fast } else { BufferMode::Native },
            make_renderer,
        )
    }

    /// Reopens the output with the requested buffer profile, preserving transport state.
    pub fn reopen_with_mode<F, E>(&mut self, mode: BufferMode, make_renderer: F) -> Result<(), StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        let tick = self.shared.tick.load(Ordering::Acquire);
        let playing = self.transport_playing;
        drop(self.stream.take());
        self.shared.failed.store(true, Ordering::Release);
        let mut replacement = Self::start_with_mode(mode, make_renderer)?;
        let _ = replacement.send(AudioCommand::Seek(tick));
        if playing {
            let _ = replacement.send(AudioCommand::Play);
        }
        *self = replacement;
        Ok(())
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn send(&mut self, command: AudioCommand<R>) -> Result<(), AudioCommand<R>> {
        match command {
            AudioCommand::ReplaceSong(renderer) => {
                let meters = Arc::new(MeterBank::new(&*renderer).seeded(&self.meters));
                let adopts = renderer.adopts();
                match self
                    .sender
                    .try_send(CallbackCommand::Replace(renderer, meters.clone()))
                {
                    Ok(()) => {
                        let old = std::mem::replace(&mut self.meters, meters);
                        // Several replaces before a render: the last bank published stands.
                        if !adopts {
                            self.replaced = None;
                        } else if old.published() {
                            self.replaced = Some(old);
                        }
                        self.transport_playing = false;
                        Ok(())
                    }
                    Err(TrySendError::Full(CallbackCommand::Replace(renderer, _)))
                    | Err(TrySendError::Disconnected(CallbackCommand::Replace(renderer, _))) => {
                        Err(AudioCommand::ReplaceSong(renderer))
                    }
                    Err(_) => unreachable!(),
                }
            }
            other => {
                let playing = match &other {
                    AudioCommand::Play => Some(true),
                    AudioCommand::Stop => Some(false),
                    _ => None,
                };
                let result = self
                    .sender
                    .try_send(CallbackCommand::Control(other))
                    .map_err(|error| match error.into_inner() {
                        CallbackCommand::Control(command) => command,
                        CallbackCommand::Replace(_, _) => unreachable!(),
                    });
                if result.is_ok()
                    && let Some(playing) = playing
                {
                    self.transport_playing = playing;
                }
                result
            }
        }
    }

    /// The transport tick, CPU load and stream state now, and each meter's peaks since the last snapshot.
    pub fn snapshot(&mut self) -> OutputSnapshot {
        while self.retired.try_recv().is_ok() {}
        if self.meters.published() {
            self.replaced = None;
        }
        let sounding = self.replaced.as_ref().unwrap_or(&self.meters).sounding();
        OutputSnapshot {
            transport_tick: self.shared.tick.load(Ordering::Acquire),
            cpu_load: f32::from_bits(self.shared.cpu.load(Ordering::Acquire)),
            underruns: self.shared.underruns.load(Ordering::Acquire),
            stream_failed: self.shared.failed.load(Ordering::Acquire),
            module_meters: self.meters.take(),
            counting_in: self.shared.counting_in(),
            ping: self.shared.ping(),
            sounding,
        }
    }
}

/// Keeps the device's default format, rate and channel count, changing only the buffer size when the device exposes a
/// supported range for that exact format. `Unknown` deliberately keeps `Default`: some backends cannot validate a
/// requested fixed size until stream creation.
fn output_config(
    device: &cpal::Device,
    default: &SupportedStreamConfig,
    mode: BufferMode,
) -> StreamConfig {
    let mut config = default.config();
    if mode == BufferMode::Native {
        return config;
    }
    let Some((min, max)) = device
        .supported_output_configs()
        .ok()
        .into_iter()
        .flatten()
        .filter(|range| {
            range.channels() == default.channels()
                && range.sample_format() == default.sample_format()
                && range.min_sample_rate() <= default.sample_rate()
                && default.sample_rate() <= range.max_sample_rate()
        })
        .filter_map(|range| match *range.buffer_size() {
            SupportedBufferSize::Range { min, max } => Some((min, max)),
            SupportedBufferSize::Unknown => None,
        })
        .min_by_key(|(min, _)| *min)
    else {
        return config;
    };
    let frames = match mode {
        BufferMode::UltraSafe => max,
        BufferMode::Safe => 256.clamp(min, max),
        BufferMode::Fast => min,
        BufferMode::Fixed(target) => target.clamp(min, max),
        BufferMode::Native => return config,
    };
    config.buffer_size = BufferSize::Fixed(frames);
    config
}

struct CallbackState<R: AudioRenderer> {
    renderer: Box<R>,
    meters: Arc<MeterBank>,
    shared: Arc<Shared>,
    receiver: Receiver<CallbackCommand<R>>,
    retired_sender: Sender<(Box<R>, Arc<MeterBank>)>,
    pending: Option<(Box<R>, Arc<MeterBank>)>,
}

impl<R: AudioRenderer> CallbackState<R> {
    fn render(&mut self, output: &mut [f32], channels: usize, sample_rate: u32) {
        #[cfg(not(target_arch = "wasm32"))]
        let start = Instant::now();
        #[cfg(target_arch = "wasm32")]
        let _ = sample_rate;
        self.dispatch();
        let buffer = self.shared.buffer_frames.load(Ordering::Acquire);
        self.renderer.set_buffer_frames(buffer);
        output.fill(0.0);
        self.renderer.render(output, channels);
        self.shared
            .tick
            .store(self.renderer.transport_tick(), Ordering::Release);
        self.shared.publish(&*self.renderer);
        for (index, cell) in self.meters.0.iter().enumerate() {
            let meter = self.renderer.meter(index);
            hold(&cell.left, meter.peak_left);
            hold(&cell.right, meter.peak_right);
            hold(&cell.reduction, meter.gain_reduction_db);
            hold(&cell.input, meter.input_peak);
            cell.publish(self.renderer.analysis(index));
        }
        self.meters.publish_sounding(&*self.renderer);
        #[cfg(not(target_arch = "wasm32"))]
        if !output.is_empty() {
            let available = output.len() as f32 / channels as f32 / sample_rate as f32;
            let load = start.elapsed().as_secs_f32() / available;
            self.shared.cpu.store(load.to_bits(), Ordering::Release);
            // ponytail: a render slower than its buffer counts as the underrun it makes; the backends report no
            // xruns of their own to count instead.
            if load > 1.0 {
                self.shared.underruns.fetch_add(1, Ordering::Release);
            }
        }
    }

    fn dispatch(&mut self) {
        if self.pending.is_some() && self.retired_sender.is_full() {
            return;
        }
        if let Some((renderer, meters)) = self.pending.take() {
            self.replace(renderer, meters);
        }
        while let Ok(command) = self.receiver.try_recv() {
            match command {
                CallbackCommand::Replace(renderer, meters) => {
                    if self.retired_sender.is_full() {
                        self.pending = Some((renderer, meters));
                        break;
                    }
                    self.replace(renderer, meters);
                }
                CallbackCommand::Control(command) => match command {
                    AudioCommand::SetMultiCore(on) => self.renderer.set_multi_core(on),
                    AudioCommand::SetParam {
                        channel,
                        instance_id,
                        index,
                        value,
                    } => self.renderer.set_param(channel, instance_id, index, value),
                    AudioCommand::Play => self.renderer.play(),
                    AudioCommand::Stop => self.renderer.stop(),
                    AudioCommand::Seek(tick) => self.renderer.seek(tick),
                    AudioCommand::NoteOn {
                        channel,
                        key,
                        velocity,
                    } => self.renderer.note_on(channel, key, velocity),
                    AudioCommand::NoteOff { channel, key } => self.renderer.note_off(channel, key),
                    AudioCommand::SetLoop(range) => self.renderer.set_loop(range),
                    AudioCommand::AllNotesOff => self.renderer.all_notes_off(),
                    AudioCommand::ResetSound => self.renderer.reset_sound(),
                    AudioCommand::SetSetting {
                        channel,
                        instance_id,
                        key,
                        value,
                    } => self.renderer.set_setting(channel, instance_id, key, value),
                    AudioCommand::CountIn(beats) => self.renderer.count_in(beats),
                    AudioCommand::Metronome { on, level } => self.renderer.set_metronome(on, level),
                    AudioCommand::MetronomeFlag { on, level } => {
                        self.renderer.set_metronome_flag(on, level)
                    }
                    AudioCommand::Ping(id) => self.renderer.ping(id),
                    AudioCommand::LiveLane { channel, notes } => {
                        self.renderer.set_live_lane(channel, notes)
                    }
                    AudioCommand::Preview(sample) => self.renderer.preview(sample),
                    AudioCommand::SetTempo(bpm) => self.renderer.set_tempo(bpm),
                    AudioCommand::LiveParam {
                        channel,
                        index,
                        value,
                    } => self.renderer.set_live_param(channel, index, value),
                    AudioCommand::ReplaceSong(_) => unreachable!(),
                },
            }
        }
    }

    fn replace(&mut self, mut renderer: Box<R>, meters: Arc<MeterBank>) {
        renderer.take_over(&mut self.renderer);
        let old = std::mem::replace(&mut self.renderer, renderer);
        let old_meters = std::mem::replace(&mut self.meters, meters);
        let _ = self.retired_sender.try_send((old, old_meters));
    }
}

fn build_stream<T, R, E>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    mut state: CallbackState<R>,
    error: E,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32> + 'static,
    R: AudioRenderer,
    E: FnMut(cpal::StreamError) + Send + 'static,
{
    let sample_rate = config.sample_rate.0;
    let mut scratch = vec![0.0_f32; 1024 * channels];
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            // The first callback's size is the stream's buffer size.
            let frames = u32::try_from(data.len() / channels.max(1)).unwrap_or(u32::MAX);
            let _ = (state.shared.buffer_frames).compare_exchange(
                0,
                frames,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            for chunk in data.chunks_mut(scratch.len()) {
                let length = chunk.len();
                state.render(&mut scratch[..length], channels, sample_rate);
                for (output, sample) in chunk.iter_mut().zip(&scratch[..length]) {
                    *output = T::from_sample(if sample.is_finite() {
                        sample.clamp(-1.0, 1.0)
                    } else {
                        0.0
                    });
                }
            }
        },
        error,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestRenderer {
        tick: u64,
        playing: bool,
        note: bool,
        gain: f32,
        looped: Option<(u64, u64)>,
        setting: Option<(usize, i32, u32, f32)>,
        count_in: u32,
        metronome: (bool, f32),
        ping: Option<PingReport>,
        /// Takes over the transport of the renderer it replaces.
        adopts: bool,
        analysis: Vec<f32>,
        /// Channel 0's keys sounding.
        keys: u128,
        /// The buffer size it was last given.
        buffer: u32,
    }

    impl AudioRenderer for TestRenderer {
        fn render(&mut self, output: &mut [f32], _: usize) {
            output.fill(if self.note { self.gain } else { 0.0 });
            if self.playing {
                self.tick += output.len() as u64 / 2;
            }
        }

        fn set_param(&mut self, _: usize, _: i32, _: i32, value: f32) {
            self.gain = value;
        }

        fn play(&mut self) {
            self.playing = true;
        }

        fn stop(&mut self) {
            self.playing = false;
        }

        fn seek(&mut self, tick: u64) {
            self.tick = tick;
        }

        fn note_on(&mut self, _: usize, _: u8, _: f32) {
            self.note = true;
        }

        fn note_off(&mut self, _: usize, _: u8) {
            self.note = false;
        }

        fn set_loop(&mut self, range: Option<(u64, u64)>) {
            self.looped = range;
        }

        fn all_notes_off(&mut self) {
            self.note = false;
        }

        fn set_setting(&mut self, channel: usize, instance_id: i32, key: u32, value: f32) {
            self.setting = Some((channel, instance_id, key, value));
        }

        fn count_in(&mut self, beats: u32) {
            self.count_in = beats;
        }

        fn set_metronome(&mut self, on: bool, level: f32) {
            self.metronome = (on, level);
        }

        fn ping(&mut self, id: u32) {
            self.ping = Some((id, (self.tick > 0).then_some(Ok(7))));
        }

        fn counting_in(&self) -> bool {
            self.count_in > 0
        }

        fn ping_report(&self) -> Option<PingReport> {
            self.ping
        }

        fn transport_tick(&self) -> u64 {
            self.tick
        }

        fn meter_count(&self) -> usize {
            1
        }

        fn meter_key(&self, _: usize) -> ModuleKey {
            ModuleKey {
                channel: 0,
                instance_id: 2,
            }
        }

        fn meter(&self, _: usize) -> MeterSnapshot {
            MeterSnapshot {
                peak_left: self.gain,
                peak_right: self.gain,
                gain_reduction_db: if self.note { 12.0 * self.gain } else { 0.0 },
                input_peak: 2.0 * self.gain,
            }
        }

        fn take_over(&mut self, old: &mut Self) {
            if self.adopts {
                (self.tick, self.playing, self.keys) = (old.tick, old.playing, old.keys);
            }
        }

        fn adopts(&self) -> bool {
            self.adopts
        }

        fn analysis(&self, _: usize) -> &[f32] {
            &self.analysis
        }

        fn sounding(&self, channel: usize) -> u128 {
            if channel == 0 { self.keys } else { 0 }
        }

        fn set_buffer_frames(&mut self, frames: u32) {
            self.buffer = frames;
        }
    }

    fn renderer() -> TestRenderer {
        TestRenderer {
            tick: 0,
            playing: false,
            note: false,
            gain: 0.5,
            looped: None,
            setting: None,
            count_in: 0,
            metronome: (false, 0.0),
            ping: None,
            adopts: false,
            analysis: vec![-60.0, 0.5, 7.0],
            keys: 0,
            buffer: 0,
        }
    }

    /// Every render call first gives the renderer the stream's buffer size, as its first callback set it.
    #[test]
    fn each_render_gives_the_renderer_the_streams_buffer_size() {
        let (mut state, _, _) = callback();
        let mut output = [0.0; 16];
        state.render(&mut output, 2, 48_000);
        assert_eq!(state.renderer.buffer, 0, "before the first callback");
        state.shared.buffer_frames.store(512, Ordering::Release);
        state.render(&mut output, 2, 48_000);
        assert_eq!(state.renderer.buffer, 512);
    }

    /// The renderer a `ReplaceSong` brings takes over the one it replaces, on the audio thread, before that one is
    /// retired (Mute and Unmute re-sync the playing engine).
    #[test]
    fn a_replacing_renderer_takes_over_the_one_it_replaces() {
        let (mut state, sender, _) = callback();
        let play = CallbackCommand::Control(AudioCommand::Play);
        assert!(sender.try_send(play).is_ok());
        let mut output = [0.0; 16];
        state.render(&mut output, 2, 48_000);
        let adopting = TestRenderer {
            adopts: true,
            ..renderer()
        };
        let meters = Arc::new(MeterBank::new(&adopting));
        assert!(
            sender
                .try_send(CallbackCommand::Replace(Box::new(adopting), meters))
                .is_ok()
        );
        state.render(&mut output, 2, 48_000);
        assert!(state.renderer.playing);
        assert_eq!(state.renderer.tick, 16);
    }

    /// A callback state on [`renderer`] with its command queue and meters, no device.
    fn callback() -> (
        CallbackState<TestRenderer>,
        Sender<CallbackCommand<TestRenderer>>,
        Arc<MeterBank>,
    ) {
        let (sender, receiver) = bounded(8);
        let (retired_sender, _) = bounded(8);
        let initial = renderer();
        let meters = Arc::new(MeterBank::new(&initial));
        let state = CallbackState {
            renderer: Box::new(initial),
            meters: meters.clone(),
            shared: Arc::new(Shared::default()),
            receiver,
            retired_sender,
            pending: None,
        };
        (state, sender, meters)
    }

    /// The app's side of [`callback`]'s state, as [`AudioOutput::start`] makes it but with no device.
    fn output() -> (AudioOutput<TestRenderer>, CallbackState<TestRenderer>) {
        let (state, sender, meters) = callback();
        let output = AudioOutput {
            stream: None,
            sender,
            retired: bounded(8).1,
            shared: state.shared.clone(),
            meters,
            replaced: None,
            sample_rate: 48_000,
            transport_playing: false,
            buffer_mode: BufferMode::Native,
        };
        (output, state)
    }

    /// The keys sounding hold through a replace. An adopting renderer carries them over, so until the
    /// audio thread first renders it the snapshot reads the bank it replaced (not its own, still at rest); a fresh one
    /// has none.
    #[test]
    fn the_sounding_keys_hold_through_an_adopting_replace_and_clear_for_a_fresh_one() {
        let (mut output, mut state) = output();
        state.renderer.keys = 1 << 60;
        state.render(&mut [0.0; 2], 2, 48_000);
        assert_eq!(output.snapshot().sounding, [1 << 60]);
        let adopting = TestRenderer {
            adopts: true,
            ..renderer()
        };
        assert!(
            output
                .send(AudioCommand::ReplaceSong(Box::new(adopting)))
                .is_ok()
        );
        assert_eq!(output.snapshot().sounding, [1 << 60], "not rendered yet");
        state.render(&mut [0.0; 2], 2, 48_000);
        assert_eq!(output.snapshot().sounding, [1 << 60], "carried over");
        // From its first render on, the new renderer's own keys.
        state.renderer.keys = 1 << 64;
        state.render(&mut [0.0; 2], 2, 48_000);
        assert_eq!(output.snapshot().sounding, [1 << 64], "its own");
        assert!(
            output
                .send(AudioCommand::ReplaceSong(Box::new(renderer())))
                .is_ok()
        );
        assert_eq!(output.snapshot().sounding, [0], "not rendered yet");
        state.render(&mut [0.0; 2], 2, 48_000);
        assert_eq!(output.snapshot().sounding, [0]);
    }

    #[test]
    fn loop_all_notes_off_and_setting_commands_reach_the_renderer() {
        let (mut state, sender, _) = callback();
        for command in [
            AudioCommand::SetLoop(Some((128, 384))),
            AudioCommand::NoteOn {
                channel: 1,
                key: 60,
                velocity: 1.0,
            },
            AudioCommand::Play,
            AudioCommand::SetSetting {
                channel: 1,
                instance_id: 3,
                key: 0x4E4F_5445,
                value: 0.25,
            },
        ] {
            assert!(sender.try_send(CallbackCommand::Control(command)).is_ok());
        }
        let mut output = [0.0; 16];
        state.render(&mut output, 2, 48_000);
        assert_eq!(state.renderer.looped, Some((128, 384)));
        assert_eq!(state.renderer.setting, Some((1, 3, 0x4E4F_5445, 0.25)));
        assert!(state.renderer.note);
        // All notes off: the note stops, the transport plays on.
        assert!(
            sender
                .try_send(CallbackCommand::Control(AudioCommand::AllNotesOff))
                .is_ok()
        );
        state.render(&mut output, 2, 48_000);
        assert!(!state.renderer.note);
        assert!(state.renderer.playing);
        assert_eq!(state.renderer.tick, 16);
        assert!(
            sender
                .try_send(CallbackCommand::Control(AudioCommand::SetLoop(None)))
                .is_ok()
        );
        state.render(&mut output, 2, 48_000);
        assert_eq!(state.renderer.looped, None);
    }

    #[test]
    fn a_peak_between_two_snapshots_is_read_then_held_until_the_next_render() {
        let (mut state, sender, meters) = callback();
        let mut output = [0.0; 2];
        // One callback of one frame carries a spike (peak 0.9, GR 10.8 dB); the next is quieter.
        state.renderer.note = true;
        state.renderer.gain = 0.9;
        state.render(&mut output, 2, 48_000);
        assert!(
            sender
                .try_send(CallbackCommand::Control(AudioCommand::SetParam {
                    channel: 0,
                    instance_id: 2,
                    index: 0,
                    value: 0.1,
                }))
                .is_ok()
        );
        state.render(&mut output, 2, 48_000);
        let read = meters.take();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].key.instance_id, 2);
        let held = read[0].value;
        assert_eq!((held.peak_left, held.peak_right), (0.9, 0.9));
        assert!((held.gain_reduction_db - 10.8).abs() < 1e-5, "{held:?}");
        // The next read without a callback between reads it again; the next callback starts over.
        assert_eq!(meters.take()[0].value, held);
        state.render(&mut output, 2, 48_000);
        assert_eq!(meters.take()[0].value.peak_left, 0.1);
    }

    #[test]
    fn the_input_peak_is_held_between_snapshots_like_the_others() {
        let (mut state, sender, meters) = callback();
        let mut output = [0.0; 2];
        state.renderer.gain = 0.4;
        state.render(&mut output, 2, 48_000);
        assert!(
            sender
                .try_send(CallbackCommand::Control(AudioCommand::SetParam {
                    channel: 0,
                    instance_id: 2,
                    index: 0,
                    value: 0.1,
                }))
                .is_ok()
        );
        state.render(&mut output, 2, 48_000);
        assert_eq!(meters.take()[0].value.input_peak, 0.8);
        assert_eq!(meters.take()[0].value.input_peak, 0.8);
        state.render(&mut output, 2, 48_000);
        assert_eq!(meters.take()[0].value.input_peak, 0.2);
    }

    /// An edit's replace does not drop the meters to 0 until the new renderer's first callback; they read
    /// the old bank's last values under the same key, and that callback's own peaks from then on.
    #[test]
    fn a_replace_reads_the_old_meters_until_the_new_renderer_renders() {
        let (mut output, mut state) = output();
        state.renderer.note = true;
        state.render(&mut [0.0; 2], 2, 48_000);
        assert_eq!(output.snapshot().module_meters[0].value.peak_left, 0.5);
        let quieter = TestRenderer {
            note: true,
            gain: 0.25,
            ..renderer()
        };
        assert!(
            output
                .send(AudioCommand::ReplaceSong(Box::new(quieter)))
                .is_ok()
        );
        let seeded = output.snapshot().module_meters[0].value;
        assert_eq!((seeded.peak_left, seeded.input_peak), (0.5, 1.0));
        state.render(&mut [0.0; 2], 2, 48_000);
        assert_eq!(output.snapshot().module_meters[0].value.peak_left, 0.25);
    }

    /// A steady tone's meters must not flicker. The UI takes one snapshot a frame at its own rate and
    /// the device renders a block whenever it asks; a frame with no block since the last snapshot must read the last
    /// peak again, not 0, so a steady tone reads steady whichever is faster.
    #[test]
    fn a_steady_tone_reads_steady_at_any_ui_frame_rate() {
        for (block, hz) in [(256, 120), (1024, 120), (1024, 60), (4096, 240), (256, 30)] {
            let (mut output, mut state) = output();
            // A held note: peak 0.5 in every block.
            state.renderer.note = true;
            let mut buffer = vec![0.0; 2 * block];
            let frame = 48_000 / hz;
            let mut due = 0;
            let reads: Vec<f32> = (1..=hz)
                .map(|k| {
                    // Every callback due by the frame, then the frame's snapshot.
                    while due <= k * frame {
                        state.render(&mut buffer, 2, 48_000);
                        due += block;
                    }
                    let meter = output.snapshot().module_meters[0].value;
                    meter.peak_left.min(meter.peak_right)
                })
                .collect();
            assert!(
                reads.iter().all(|&p| p == 0.5),
                "{block}-frame blocks, {hz} Hz frames: {reads:?}"
            );
        }
    }

    #[test]
    fn an_analysis_rests_until_the_first_render_then_reads_as_last_published() {
        let (mut state, _, meters) = callback();
        // Before any render: the renderer's own rest values.
        assert_eq!(meters.take()[0].analysis, [-60.0, 0.5, 7.0]);
        let mut output = [0.0; 2];
        state.renderer.analysis = vec![-12.0, 0.25, 3.0];
        state.render(&mut output, 2, 48_000);
        // Not taken by a read: the next one sees it too.
        for _ in 0..2 {
            assert_eq!(meters.take()[0].analysis, [-12.0, 0.25, 3.0]);
        }
        // Each publish is whole: the version is even again and moved by 2.
        assert_eq!(meters.0[0].version.load(Ordering::Acquire), 2);
        // A renderer with more values than the slots it was sized for writes what fits.
        state.renderer.analysis = vec![1.0, 2.0, 3.0, 4.0];
        state.render(&mut output, 2, 48_000);
        assert_eq!(meters.take()[0].analysis, [1.0, 2.0, 3.0]);
    }

    /// Each channel's sounding keys reach the snapshot as the last render left them; a read takes nothing.
    #[test]
    fn the_sounding_keys_reach_the_snapshot_as_the_last_render_left_them() {
        let (mut state, _, meters) = callback();
        // The renderer's one meter is on channel 0: one channel, at rest before the first render.
        assert_eq!(meters.sounding(), [0]);
        state.renderer.keys = 1 << 60 | 1 << 100;
        state.render(&mut [0.0; 2], 2, 48_000);
        for _ in 0..2 {
            assert_eq!(meters.sounding(), [1 << 60 | 1 << 100]);
        }
        state.renderer.keys = 0;
        state.render(&mut [0.0; 2], 2, 48_000);
        assert_eq!(meters.sounding(), [0]);
    }

    #[test]
    fn a_reader_never_sees_a_half_written_analysis() {
        // The audio thread publishes slabs whose values are all equal, a callback apart (here 20 µs), while a reader
        // reads on: every read is uniform.
        let mut slab = renderer();
        slab.analysis = vec![0.0; 512];
        let bank = Arc::new(MeterBank::new(&slab));
        let writer = {
            let bank = bank.clone();
            std::thread::spawn(move || {
                for n in 0..2_000 {
                    bank.0[0].publish(&[n as f32; 512]);
                    std::thread::sleep(std::time::Duration::from_micros(20));
                }
            })
        };
        let mut reads = 0;
        while !writer.is_finished() || reads == 0 {
            let read = bank.0[0].analysis();
            assert!(read.iter().all(|v| *v == read[0]), "torn at {}", read[0]);
            reads += 1;
        }
        writer.join().unwrap();
        assert_eq!(bank.0[0].analysis(), [1_999.0; 512]);
    }

    #[test]
    fn commands_publish_audio_position_and_meters_without_device() {
        let (sender, receiver) = bounded(8);
        let (retired_sender, retired) = bounded(8);
        let initial = renderer();
        let meters = Arc::new(MeterBank::new(&initial));
        let shared = Arc::new(Shared::default());
        let mut state = CallbackState {
            renderer: Box::new(initial),
            meters: meters.clone(),
            shared: shared.clone(),
            receiver,
            retired_sender,
            pending: None,
        };
        assert!(
            sender
                .try_send(CallbackCommand::Control(AudioCommand::SetParam {
                    channel: 0,
                    instance_id: 2,
                    index: 0,
                    value: 0.8,
                }))
                .is_ok()
        );
        assert!(
            sender
                .try_send(CallbackCommand::Control(AudioCommand::NoteOn {
                    channel: 0,
                    key: 60,
                    velocity: 1.0,
                }))
                .is_ok()
        );
        assert!(
            sender
                .try_send(CallbackCommand::Control(AudioCommand::Play))
                .is_ok()
        );
        let mut output = [0.0; 16];
        state.render(&mut output, 2, 48_000);
        assert_eq!(output, [0.8; 16]);
        assert_eq!(shared.tick.load(Ordering::Acquire), 8);
        assert_eq!(take(&meters.0[0].left), 0.8);

        assert!(
            sender
                .try_send(CallbackCommand::Replace(
                    Box::new(renderer()),
                    Arc::new(MeterBank::new(&renderer()))
                ))
                .is_ok()
        );
        state.render(&mut output, 2, 48_000);
        assert_eq!(output, [0.0; 16]);
        assert!(retired.try_recv().is_ok());

        assert!(
            sender
                .try_send(CallbackCommand::Control(AudioCommand::Seek(128)))
                .is_ok()
        );
        assert!(
            sender
                .try_send(CallbackCommand::Control(AudioCommand::Play))
                .is_ok()
        );
        state.render(&mut output, 2, 48_000);
        assert_eq!(shared.tick.load(Ordering::Acquire), 136);
    }

    #[test]
    fn count_in_metronome_and_the_ping_reach_the_renderer_and_the_snapshot() {
        let (mut state, sender, _) = callback();
        let shared = state.shared.clone();
        let snapshot = || (shared.counting_in(), shared.ping());
        assert_eq!(snapshot(), (false, None));
        for command in [
            AudioCommand::CountIn(4),
            AudioCommand::Metronome {
                on: true,
                level: 0.25,
            },
            AudioCommand::Ping(3),
        ] {
            assert!(sender.try_send(CallbackCommand::Control(command)).is_ok());
        }
        let mut output = [0.0; 16];
        state.render(&mut output, 2, 48_000);
        assert_eq!(state.renderer.metronome, (true, 0.25));
        assert_eq!(snapshot(), (true, Some((3, None))));
        // A result; the count-in taken back.
        for command in [AudioCommand::CountIn(0), AudioCommand::Ping(4)] {
            assert!(sender.try_send(CallbackCommand::Control(command)).is_ok());
        }
        state.renderer.tick = 8;
        state.render(&mut output, 2, 48_000);
        assert_eq!(snapshot(), (false, Some((4, Some(Ok(7))))));
        // Every report packs into one word and back.
        for result in [
            None,
            Some(Ok(0)),
            Some(Ok(47_999)),
            Some(Err(PingFailure::Noise)),
            Some(Err(PingFailure::Timeout)),
            Some(Err(PingFailure::Unknown)),
        ] {
            assert_eq!(unpack(pack((u32::MAX, result))), Some((u32::MAX, result)));
        }
        assert_eq!(unpack(0), None);
    }
}

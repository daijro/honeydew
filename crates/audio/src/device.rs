use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering, fence};
use std::sync::{Arc, Mutex, PoisonError};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, StreamConfig, SupportedBufferSize, SupportedStreamConfig};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};

use crate::MeterSnapshot;
use crate::sample::SampleData;

/// A callback renders in sub-blocks of at most this many frames, and publishes each one's meters and analyses as a
/// visual frame of its own ([`MeterBank`]): the panels move several times a callback, whatever the buffer size.
pub const SUB_BLOCK: usize = 512;

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

/// A param a Modulator moves ([`AudioRenderer::modulated`]): its module, its param index and its value as the last render
/// left it, NaN while nothing moves it (the transport is stopped: it rests at its base).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Modulated {
    pub key: ModuleKey,
    pub index: i32,
    pub value: f32,
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
    /// The params the Modulators move, as the last render left them ([`AudioRenderer::modulated`]).
    pub modulated: Vec<Modulated>,
    /// The commands the audio thread has taken from the queue so far: those up to [`AudioOutput::sent`]'s count then
    /// have reached the renderer.
    pub taken: u64,
    /// The transport tick at the frame heard now, on the output clock as the meters ([`AudioOutput::snapshot_at`]):
    /// the drawn playhead's. Behind [`Self::transport_tick`] by the output's latency (most with Bluetooth).
    pub heard_tick: u64,
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
    /// REC's arm: while `on` a recording runs (no wrap at the song end) and, if `click`, the song's beats click while
    /// playing; `level` is the normalized `Metronome level` setting, which the count-in uses too.
    fn set_metronome(&mut self, on: bool, click: bool, level: f32);
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
    /// The params the Modulators move, their values as the last render left them; the length is fixed while the
    /// renderer lives (the device sizes its slots from it once). The default: none.
    fn modulated(&self) -> &[Modulated] {
        &[]
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
        click: bool,
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

/// The visual frames each meter keeps (and the transport, [`OutputSnapshot::heard_tick`]): 680 ms at 48 kHz, for the
/// output's latency (a Bluetooth headset's 200 ms or more, #92) and a callback beside it.
const SLOTS: usize = 64;

/// How fast the output clock may run ahead of the latest callback's: 1000 ppm, a device clock running fast.
const DRIFT: f64 = 1e-3;

/// One sub-block's visuals for a meter ([`MeterCell::frames`]), under a seqlock: one writer, the audio thread, which
/// never waits.
struct Visual {
    /// Odd while the audio thread writes it.
    version: AtomicU32,
    /// The stream frame its sub-block ended at; 0 before the first write.
    end: AtomicU64,
    /// The transport tick its sub-block ended at.
    tick: AtomicU64,
    /// Peak left, peak right, gain reduction and input peak over the sub-block, as f32 bits.
    meter: [AtomicU32; 4],
    /// The analysis as the sub-block left it, as f32 bits.
    analysis: Box<[AtomicU32]>,
}

impl Visual {
    fn new(analysis: usize) -> Self {
        Self {
            version: AtomicU32::new(0),
            end: AtomicU64::new(0),
            tick: AtomicU64::new(0),
            meter: Default::default(),
            analysis: (0..analysis).map(|_| AtomicU32::new(0)).collect(),
        }
    }

    /// The audio thread's write. Extra analysis values are dropped.
    fn write(&self, end: u64, tick: u64, meter: MeterSnapshot, analysis: &[f32]) {
        let version = self.version.load(Ordering::Relaxed);
        self.version
            .store(version.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        self.end.store(end, Ordering::Relaxed);
        self.tick.store(tick, Ordering::Relaxed);
        let values = [
            meter.peak_left,
            meter.peak_right,
            meter.gain_reduction_db,
            meter.input_peak,
        ];
        for (slot, value) in self.meter.iter().zip(values) {
            slot.store(value.to_bits(), Ordering::Relaxed);
        }
        for (slot, value) in self.analysis.iter().zip(analysis) {
            slot.store(value.to_bits(), Ordering::Relaxed);
        }
        self.version
            .store(version.wrapping_add(2), Ordering::Release);
    }

    /// Its end and what `read` takes from it, whole, or `None` before the first write. A read the writer overlaps is
    /// retried, yielding after the first few tries (a writer the OS preempted mid-write needs the CPU back). After 1024
    /// tries in a row, never in practice, the last read stands, at worst a mix of two sub-blocks: a display glitch, not
    /// a hang.
    fn read<T>(&self, read: impl Fn(&Self) -> T) -> Option<(u64, T)> {
        let mut attempt = 0;
        loop {
            let before = self.version.load(Ordering::Acquire);
            let (end, value) = (self.end.load(Ordering::Relaxed), read(self));
            fence(Ordering::Acquire);
            attempt += 1;
            if before.is_multiple_of(2) && self.version.load(Ordering::Relaxed) == before
                || attempt == 1024
            {
                return (end > 0).then_some((end, value));
            }
            #[cfg(not(target_arch = "wasm32"))]
            if attempt >= 8 {
                std::thread::yield_now();
            }
        }
    }

    fn meter(&self) -> MeterSnapshot {
        let [left, right, reduction, input] = self
            .meter
            .each_ref()
            .map(|v| f32::from_bits(v.load(Ordering::Relaxed)));
        MeterSnapshot {
            peak_left: left,
            peak_right: right,
            gain_reduction_db: reduction,
            input_peak: input,
        }
    }

    fn analysis(&self) -> Vec<f32> {
        (self.analysis.iter())
            .map(|v| f32::from_bits(v.load(Ordering::Relaxed)))
            .collect()
    }
}

/// The level meters' release (#41), in dB a second, whatever the sample rate, the buffer size and the UI's frame rate.
/// Honeydew change (#135): twice the earlier 20.9 dB/s at 48 kHz (19.2 at 44.1 kHz), so a meter shows the level sooner.
const RELEASE_DB_PER_S: f32 = 40.0;

/// A released level under −96 dB, the floor of the deepest meter scale (the Compressor's), is silence: a meter comes
/// to rest.
const RELEASE_FLOOR: f32 = 1.5849e-5;

/// `m` `frames` stream frames on at `rate` Hz: both peaks and the input peak fallen at the release
/// ([`RELEASE_DB_PER_S`]), 0 under [`RELEASE_FLOOR`], and the gain reduction as it was.
fn released(m: MeterSnapshot, frames: u64, rate: u32) -> MeterSnapshot {
    let k = 10f32.powf(-RELEASE_DB_PER_S / 20.0 * frames as f32 / rate.max(1) as f32);
    let fall = |peak: f32| match peak.abs() * k {
        p if p > RELEASE_FLOOR => p,
        _ => 0.0,
    };
    MeterSnapshot {
        peak_left: fall(m.peak_left),
        peak_right: fall(m.peak_right),
        input_peak: fall(m.input_peak),
        ..m
    }
}

/// Each value's larger magnitude.
fn louder(a: MeterSnapshot, b: MeterSnapshot) -> MeterSnapshot {
    let max = |a: f32, b: f32| a.abs().max(b.abs());
    MeterSnapshot {
        peak_left: max(a.peak_left, b.peak_left),
        peak_right: max(a.peak_right, b.peak_right),
        gain_reduction_db: max(a.gain_reduction_db, b.gain_reduction_db),
        input_peak: max(a.input_peak, b.input_peak),
    }
}

struct MeterCell {
    key: ModuleKey,
    /// The last [`SLOTS`] sub-blocks' visuals: the callback writes its n-th sub-block's in slot n % [`SLOTS`].
    frames: Box<[Visual]>,
    /// The analysis that reads before the first render: the renderer's own at rest.
    rest: Box<[f32]>,
}

/// The meters, by channel (one past the meters' highest) the low and high halves of the keys sounding at its input
/// ([`AudioRenderer::sounding`]), whether the audio thread has published those keys yet, the modulated params with
/// their values' bits ([`AudioRenderer::modulated`]), and the app's side of the meters' visual frames: per meter, the end
/// of the last sub-block it read and the meter it read then.
struct MeterBank(
    Vec<MeterCell>,
    Box<[[AtomicU64; 2]]>,
    AtomicBool,
    Box<[(Modulated, AtomicU32)]>,
    Mutex<Vec<(u64, MeterSnapshot)>>,
);

impl MeterBank {
    fn new<R: AudioRenderer>(renderer: &R) -> Self {
        let channels = (0..renderer.meter_count())
            .map(|index| renderer.meter_key(index).channel + 1)
            .max()
            .unwrap_or(0);
        Self(
            (0..renderer.meter_count())
                .map(|index| {
                    let rest: Box<[f32]> = renderer.analysis(index).into();
                    MeterCell {
                        key: renderer.meter_key(index),
                        frames: (0..SLOTS).map(|_| Visual::new(rest.len())).collect(),
                        rest,
                    }
                })
                .collect(),
            (0..channels).map(|_| Default::default()).collect(),
            AtomicBool::new(false),
            (renderer.modulated().iter())
                .map(|&m| (m, AtomicU32::new(f32::NAN.to_bits())))
                .collect(),
            Mutex::new(vec![(0, MeterSnapshot::default()); renderer.meter_count()]),
        )
    }

    /// Each meter starting at `old`'s last read under its key (an edit's replace reads no 0 until the new
    /// renderer's first callback). On the app thread, before the bank is shared.
    fn seeded(self, old: &MeterBank) -> Self {
        {
            let (from, mut to) = (old.reader(), self.reader());
            for (cell, seen) in self.0.iter().zip(to.iter_mut()) {
                if let Some(at) = old.0.iter().position(|from| from.key == cell.key) {
                    *seen = from[at];
                }
            }
        }
        // So does each modulated param, which would read as resting at its base until then.
        for (m, cell) in &self.3 {
            let same = |(o, _): &&(Modulated, AtomicU32)| (o.key, o.index) == (m.key, m.index);
            if let Some((_, from)) = old.3.iter().find(same) {
                cell.store(from.load(Ordering::Acquire), Ordering::Relaxed);
            }
        }
        self
    }

    fn reader(&self) -> std::sync::MutexGuard<'_, Vec<(u64, MeterSnapshot)>> {
        self.4.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The audio thread's write of a sub-block's visuals, ending at stream frame `end`, into `slot`.
    fn publish<R: AudioRenderer>(&self, renderer: &R, slot: usize, end: u64) {
        for (index, cell) in self.0.iter().enumerate() {
            cell.frames[slot].write(end, 0, renderer.meter(index), renderer.analysis(index));
        }
    }

    /// The audio thread's write of each channel's sounding keys, and of each modulated param's value. The halves are
    /// stored one after the other: a read between them mixes two renders' keys for one frame, a display glitch the next
    /// read mends.
    fn publish_sounding<R: AudioRenderer>(&self, renderer: &R) {
        for (channel, [low, high]) in self.1.iter().enumerate() {
            let keys = renderer.sounding(channel);
            low.store(keys as u64, Ordering::Release);
            high.store((keys >> 64) as u64, Ordering::Release);
        }
        self.2.store(true, Ordering::Release);
        for ((_, cell), m) in self.3.iter().zip(renderer.modulated()) {
            cell.store(m.value.to_bits(), Ordering::Release);
        }
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

    /// The meters as the app shows them at stream frame `upto` (the output clock, [`AudioOutput::snapshot_at`];
    /// `u64::MAX`: as last published), as of the last sub-block that ended by then: every meter's levels the loudest
    /// peak so far, each fallen at the release ([`released`]) since its sub-block ended (item 18: a peak between two
    /// reads is not lost), and its gain reduction the largest since the last read, or the last read's with none; and
    /// each analysis as that sub-block left it. The stream runs at `rate` Hz.
    fn take(&self, upto: u64, rate: u32) -> Vec<ModuleMeter> {
        let mut reader = self.reader();
        (self.0.iter().zip(reader.iter_mut()))
            .map(|(cell, (seen, last))| {
                let read: [_; SLOTS] =
                    std::array::from_fn(|slot| cell.frames[slot].read(Visual::meter));
                // The oldest frame and the newest by then, as (end, slot).
                let ends =
                    || (read.iter().enumerate()).filter_map(|(s, r)| Some((r.as_ref()?.0, s)));
                let oldest = ends().min();
                let newest = ends().filter(|&(end, _)| end <= upto).max();
                let now = newest.map_or(*seen, |(end, _)| end.max(*seen));
                let fresh = (read.iter().flatten()).filter(|(end, _)| *seen < *end && *end <= upto);
                if let Some(peaks) =
                    (fresh.map(|&(end, m)| released(m, now - end, rate))).reduce(louder)
                {
                    let held = MeterSnapshot {
                        gain_reduction_db: 0.0,
                        ..released(*last, now - *seen, rate)
                    };
                    *last = louder(peaks, held);
                }
                *seen = now;
                // ponytail: a callback longer than the ring beside the latency shows its oldest frame until the clock
                // reaches it; widen the ring if buffers over the latency's room in `SLOTS` ever matter.
                let analysis = (newest.or(oldest))
                    .and_then(|(_, slot)| cell.frames[slot].read(Visual::analysis))
                    .map_or_else(|| cell.rest.to_vec(), |(_, analysis)| analysis);
                ModuleMeter {
                    key: cell.key,
                    value: *last,
                    analysis,
                }
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
    /// The output clock, once a callback has set it: when stream frame 0 is heard at the callbacks' pace, ns since
    /// [`clock`]'s epoch ([`AudioOutput::snapshot_at`], [`CallbackState::callback`]).
    clocked: AtomicBool,
    origin: AtomicI64,
    /// The commands the audio thread has taken from the queue ([`OutputSnapshot::taken`]).
    taken: AtomicU64,
    /// Each sub-block's transport tick, as the meters' visual frames ([`OutputSnapshot::heard_tick`]).
    ticks: Box<[Visual]>,
}

/// Now, ns since the first call: the output clock's time base. `None` on the web, which has no clock to read here
/// (its visuals show as last published).
pub fn clock() -> Option<i64> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        Some(EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as i64)
    }
    #[cfg(target_arch = "wasm32")]
    None
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
    /// A stream's error report, on the thread cpal reports it on (the audio thread for some): an xrun counts as an
    /// underrun, and one that stops the stream fails it ([`stops`]), which the app then reopens. Atomics only.
    fn report(&self, error: &cpal::Error) {
        if error.kind() == cpal::ErrorKind::Xrun {
            self.underruns.fetch_add(1, Ordering::Release);
        }
        if stops(error) {
            self.failed.store(true, Ordering::Release);
        }
    }

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
    Config(cpal::Error),
    Renderer(E),
    Build(cpal::Error),
    Play(cpal::Error),
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
    /// The commands queued so far ([`OutputSnapshot::taken`]).
    sent: u64,
    /// The stream frame the last snapshot showed on the output clock: a clock that moves later (a callback later than
    /// any before it) holds the visuals there, never back.
    heard: u64,
}

impl<R: AudioRenderer> AudioOutput<R> {
    pub fn start<F, E>(make_renderer: F) -> Result<Self, StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        Self::start_with_mode(BufferMode::Native, make_renderer)
    }

    /// Starts the output, preferring the smallest buffer the device advertises when `low_latency` is on.
    pub fn start_with_latency<F, E>(
        low_latency: bool,
        make_renderer: F,
    ) -> Result<Self, StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        Self::start_with_mode(
            if low_latency {
                BufferMode::Fast
            } else {
                BufferMode::Native
            },
            make_renderer,
        )
    }

    /// Starts the output with an explicit buffer profile.
    pub fn start_with_mode<F, E>(mode: BufferMode, make_renderer: F) -> Result<Self, StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        Self::start_with_mode_at(mode, 0, make_renderer)
    }

    /// [`Self::start_with_mode`] at `rate` Hz (PR #87: a project's sample rate) where the device takes it in its
    /// default format and channel count; else, and for 0, at the device's default rate. The renderer is built at the
    /// rate the stream runs, so a fallback shows as [`Self::sample_rate`] differing from `rate`.
    ///
    /// 0 is the default rate as cpal 0.16 chose it: 44.1 kHz where the device takes it on ALSA, AAudio and the web,
    /// where cpal 0.18 prefers 48, so a song at the output's own rate plays and renders at the rate it did.
    pub fn start_with_mode_at<F, E>(
        mode: BufferMode,
        rate: u32,
        make_renderer: F,
    ) -> Result<Self, StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        let rate = match rate {
            0 if cfg!(any(
                target_os = "linux",
                target_os = "freebsd",
                target_os = "android",
                target_arch = "wasm32"
            )) =>
            {
                44_100
            }
            rate => rate,
        };
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(StartError::NoDevice)?;
        let default = device.default_output_config().map_err(StartError::Config)?;
        let takes = |r: &cpal::SupportedStreamConfigRange| {
            r.channels() == default.channels()
                && r.sample_format() == default.sample_format()
                && (r.min_sample_rate()..=r.max_sample_rate()).contains(&rate)
        };
        let asked = (rate > 0 && rate != default.sample_rate())
            .then(|| device.supported_output_configs().ok())
            .flatten()
            .and_then(|mut ranges| ranges.find(takes));
        let supported = match asked {
            Some(range) => range.with_sample_rate(rate),
            None => default,
        };
        let sample_rate = supported.sample_rate();
        let channels = usize::from(supported.channels());
        let renderer = make_renderer(sample_rate).map_err(StartError::Renderer)?;
        let (mut output, state) = Self::unstarted(renderer, sample_rate);
        output.buffer_mode = mode;
        let error_shared = output.shared.clone();
        let error = move |error: cpal::Error| error_shared.report(&error);
        let config = output_config(&device, &supported, mode);
        let stream = match supported.sample_format() {
            cpal::SampleFormat::I8 => {
                build_stream::<i8, _, _>(&device, config, channels, state, error)
            }
            cpal::SampleFormat::I16 => {
                build_stream::<i16, _, _>(&device, config, channels, state, error)
            }
            cpal::SampleFormat::I24 => {
                build_stream::<cpal::I24, _, _>(&device, config, channels, state, error)
            }
            cpal::SampleFormat::I32 => {
                build_stream::<i32, _, _>(&device, config, channels, state, error)
            }
            cpal::SampleFormat::I64 => {
                build_stream::<i64, _, _>(&device, config, channels, state, error)
            }
            cpal::SampleFormat::U8 => {
                build_stream::<u8, _, _>(&device, config, channels, state, error)
            }
            cpal::SampleFormat::U16 => {
                build_stream::<u16, _, _>(&device, config, channels, state, error)
            }
            cpal::SampleFormat::U32 => {
                build_stream::<u32, _, _>(&device, config, channels, state, error)
            }
            cpal::SampleFormat::U64 => {
                build_stream::<u64, _, _>(&device, config, channels, state, error)
            }
            cpal::SampleFormat::F32 => {
                build_stream::<f32, _, _>(&device, config, channels, state, error)
            }
            cpal::SampleFormat::F64 => {
                build_stream::<f64, _, _>(&device, config, channels, state, error)
            }
            _ => return Err(StartError::UnsupportedSampleFormat),
        }
        .map_err(StartError::Build)?;
        stream.play().map_err(StartError::Play)?;
        output.stream = Some(stream);
        Ok(output)
    }

    /// An output with no stream yet playing `renderer` at `sample_rate`, and the callback state a stream runs.
    fn unstarted(renderer: R, sample_rate: u32) -> (Self, CallbackState<R>) {
        // The clock's epoch, here rather than on the audio thread.
        clock();
        let meters = Arc::new(MeterBank::new(&renderer));
        let shared = Arc::new(Shared {
            tick: AtomicU64::new(renderer.transport_tick()),
            ticks: (0..SLOTS).map(|_| Visual::new(0)).collect(),
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
            taken: 0,
            frames: 0,
            visuals: 0,
            clock: None,
            load: 0.0,
        };
        let output = Self {
            stream: None,
            sender,
            retired,
            shared,
            meters,
            replaced: None,
            sample_rate,
            transport_playing: false,
            buffer_mode: BufferMode::Native,
            sent: 0,
            heard: 0,
        };
        (output, state)
    }

    /// An output on no device playing `renderer` at `sample_rate`, and the stream's callback for the caller to run: the
    /// whole audio-thread path, for tests and benchmarks.
    pub fn offline(renderer: R, sample_rate: u32) -> (Self, OfflineCallback<R>) {
        let (output, state) = Self::unstarted(renderer, sample_rate);
        let callback = OfflineCallback {
            state,
            scratch: vec![0.0; 2 * SUB_BLOCK],
            sample_rate,
        };
        (output, callback)
    }

    pub fn reopen<F, E>(&mut self, make_renderer: F) -> Result<(), StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        self.reopen_with_mode(self.buffer_mode, make_renderer)
    }

    /// Reopens the output with the requested buffer policy, preserving transport state.
    pub fn reopen_with_latency<F, E>(
        &mut self,
        low_latency: bool,
        make_renderer: F,
    ) -> Result<(), StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        self.reopen_with_mode(
            if low_latency {
                BufferMode::Fast
            } else {
                BufferMode::Native
            },
            make_renderer,
        )
    }

    /// Reopens the output with the requested buffer profile, preserving transport state.
    pub fn reopen_with_mode<F, E>(
        &mut self,
        mode: BufferMode,
        make_renderer: F,
    ) -> Result<(), StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        self.reopen_with_mode_at(mode, 0, make_renderer)
    }

    /// [`Self::reopen_with_mode`] at `rate` Hz, as [`Self::start_with_mode_at`] takes it (PR #87).
    pub fn reopen_with_mode_at<F, E>(
        &mut self,
        mode: BufferMode,
        rate: u32,
        make_renderer: F,
    ) -> Result<(), StartError<E>>
    where
        F: FnOnce(u32) -> Result<R, E>,
    {
        let tick = self.shared.tick.load(Ordering::Acquire);
        let playing = self.transport_playing;
        drop(self.stream.take());
        self.shared.failed.store(true, Ordering::Release);
        let mut replacement = Self::start_with_mode_at(mode, rate, make_renderer)?;
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

    /// The commands queued so far, ReplaceSong included: the audio thread has taken them all once the snapshot's
    /// [`OutputSnapshot::taken`] reaches this count.
    pub fn sent(&self) -> u64 {
        self.sent
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
                        self.sent += 1;
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
                if result.is_ok() {
                    self.sent += 1;
                    if let Some(playing) = playing {
                        self.transport_playing = playing;
                    }
                }
                result
            }
        }
    }

    /// The transport tick, CPU load and stream state now, and the meters as the app shows them now
    /// ([`Self::snapshot_at`]).
    pub fn snapshot(&mut self) -> OutputSnapshot {
        self.snapshot_at(clock())
    }

    /// [`Self::snapshot`] at `now`, ns on [`clock`]. The meters, analyses and [`OutputSnapshot::heard_tick`] play back
    /// on the output clock, as the audio is heard: a callback publishes its sub-blocks' visuals at once, and they show
    /// as its frames' time goes by (each sub-block once the clock has passed its end), from when its first frame is
    /// heard (the callbacks' clock, [`CallbackState::callback`]). The frame shown never goes back. Without a clock
    /// (`now` or the callbacks' `None`), as last published. The transport tick and the rest are always as last
    /// published.
    pub fn snapshot_at(&mut self, now: Option<i64>) -> OutputSnapshot {
        let rate = f64::from(self.sample_rate) / 1e9;
        let upto = match now {
            Some(now) if self.shared.clocked.load(Ordering::Acquire) => {
                let origin = self.shared.origin.load(Ordering::Acquire);
                let upto = ((now - origin).max(0) as f64 * rate).round() as u64;
                self.heard = self.heard.max(upto);
                self.heard
            }
            _ => u64::MAX,
        };
        while self.retired.try_recv().is_ok() {}
        if self.meters.published() {
            self.replaced = None;
        }
        let sounding = self.replaced.as_ref().unwrap_or(&self.meters).sounding();
        // Before the tick ([`CallbackState::render`]).
        let taken = self.shared.taken.load(Ordering::Acquire);
        let transport_tick = self.shared.tick.load(Ordering::Acquire);
        let ticks: Vec<_> = (self.shared.ticks.iter())
            .filter_map(|v| v.read(|v| v.tick.load(Ordering::Relaxed)))
            .collect();
        // The sub-block heard last, else the oldest kept (as the meters).
        let heard = ticks.iter().filter(|(end, _)| *end <= upto).max();
        OutputSnapshot {
            transport_tick,
            heard_tick: heard
                .or(ticks.iter().min())
                .map_or(transport_tick, |&(_, t)| t),
            cpu_load: f32::from_bits(self.shared.cpu.load(Ordering::Acquire)),
            underruns: self.shared.underruns.load(Ordering::Acquire),
            stream_failed: self.shared.failed.load(Ordering::Acquire),
            module_meters: self.meters.take(upto, self.sample_rate),
            modulated: (self.meters.3.iter())
                .map(|(m, cell)| Modulated {
                    value: f32::from_bits(cell.load(Ordering::Acquire)),
                    ..*m
                })
                .collect(),
            counting_in: self.shared.counting_in(),
            ping: self.shared.ping(),
            sounding,
            taken,
        }
    }
}

/// Keeps the device's default format, rate and channel count, changing only the buffer size when the device exposes a
/// supported range for that exact format. `Unknown` deliberately keeps `Default`: some backends cannot validate a
/// requested fixed size until stream creation. Android's AAudio says `Unknown` (its burst is known once a stream is
/// open) and fits a fixed size to the device itself: there a [`BufferMode::Fixed`] size goes as asked.
fn output_config(
    device: &cpal::Device,
    default: &SupportedStreamConfig,
    mode: BufferMode,
) -> StreamConfig {
    let mut config = default.config();
    // iOS keeps its audio session's buffer (`ios::start`): cpal's iOS host sets the session's own for a fixed size, and
    // it would stay after a later stream at the default.
    if mode == BufferMode::Native || cfg!(target_os = "ios") {
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
        if let (true, BufferMode::Fixed(frames)) = (cfg!(target_os = "android"), mode) {
            config.buffer_size = BufferSize::Fixed(frames);
        }
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

/// Whether a stream's error stops it. Since cpal 0.18 an xrun, a refused low-latency mode or real-time priority, and a
/// default device's stream following a new default come as errors too, and the stream plays on. On iOS so does a route
/// change cpal calls invalidating: the session's own category changes (the input's open and close) make them, and the
/// app reopens for the routes that matter itself (`ios::take_reopen`), as with cpal 0.16, which reported none.
pub(crate) fn stops(error: &cpal::Error) -> bool {
    use cpal::ErrorKind::{DeviceChanged, RealtimeDenied, StreamInvalidated, Xrun};
    match error.kind() {
        Xrun | RealtimeDenied | DeviceChanged => false,
        StreamInvalidated => cfg!(not(target_os = "ios")),
        _ => true,
    }
}

struct CallbackState<R: AudioRenderer> {
    renderer: Box<R>,
    meters: Arc<MeterBank>,
    shared: Arc<Shared>,
    receiver: Receiver<CallbackCommand<R>>,
    retired_sender: Sender<(Box<R>, Arc<MeterBank>)>,
    pending: Option<(Box<R>, Arc<MeterBank>)>,
    /// The commands taken from the queue so far ([`Shared::taken`]).
    taken: u64,
    /// The stream frames rendered so far, which stamp each sub-block's visuals with where it ends.
    frames: u64,
    /// The sub-blocks published so far: the next one's visuals go to slot `visuals % SLOTS`.
    visuals: usize,
    /// The output clock's origin ([`Shared::origin`]) and the callback that last set it, ns on [`clock`].
    clock: Option<(i64, i64)>,
    /// The load the snapshot reads, averaged (none on the web, which has no clock here).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    load: f32,
}

/// A stream's callback with no device ([`AudioOutput::offline`]).
pub struct OfflineCallback<R: AudioRenderer> {
    state: CallbackState<R>,
    scratch: Vec<f32>,
    sample_rate: u32,
}

impl<R: AudioRenderer> OfflineCallback<R> {
    /// One stereo callback into `output`, as the stream runs it, at `now` on [`clock`] (`None`: without a clock).
    /// Allocates nothing.
    pub fn run(&mut self, output: &mut [f32], now: Option<i64>) {
        let (frames, rate) = (output.len() / 2, self.sample_rate);
        (self.state).callback(frames, 2, rate, now, &mut self.scratch, |at, rendered| {
            output[2 * at..2 * at + rendered.len()].copy_from_slice(rendered)
        });
    }
}

/// The CPU load the snapshot reads is each callback's averaged over about this many seconds (a time-weighted
/// exponential average), so one slow callback is no spike.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const LOAD_AVERAGE_S: f32 = 0.5;

impl<R: AudioRenderer> CallbackState<R> {
    /// One device callback of `frames` frames, whose first frame is heard at `now` (ns on [`clock`]: the callback's
    /// start and the latency the host reports; `None` without a clock): rendered into `scratch` a sub-block at a time
    /// ([`SUB_BLOCK`] frames of `channels`; `scratch` holds one), each handed to `out` with its first frame. Every
    /// sub-block publishes its visuals; `now` sets the output clock they play back on. The CPU load is the callback's
    /// time over its buffer's, averaged ([`LOAD_AVERAGE_S`]).
    ///
    /// #95: the clock follows the callback latest for its frames, the one nearest its deadline. Callbacks that come in
    /// bursts (a Bluetooth output's) run early by up to a burst and set it no earlier: the visuals then move on at the
    /// audio's pace between bursts instead of jumping with each. It falls back only at [`DRIFT`], for a device clock
    /// that runs fast.
    fn callback(
        &mut self,
        frames: usize,
        channels: usize,
        sample_rate: u32,
        now: Option<i64>,
        scratch: &mut [f32],
        mut out: impl FnMut(usize, &[f32]),
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        let start = Instant::now();
        if let Some(now) = now {
            let ahead = self.frames as f64 * 1e9 / f64::from(sample_rate);
            let mut origin = now - ahead as i64;
            if let Some((last, at)) = self.clock {
                origin = origin.max(last - ((now - at).max(0) as f64 * DRIFT) as i64);
            }
            self.clock = Some((origin, now));
            self.shared.origin.store(origin, Ordering::Release);
            self.shared.clocked.store(true, Ordering::Release);
        }
        let mut at = 0;
        while at < frames {
            let length = (frames - at).min(SUB_BLOCK) * channels;
            self.render(&mut scratch[..length], channels);
            out(at, &scratch[..length]);
            at += SUB_BLOCK;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if frames > 0 {
            let load = start.elapsed().as_secs_f32() * sample_rate as f32 / frames as f32;
            let k = (frames as f32 / sample_rate as f32 / LOAD_AVERAGE_S).min(1.0);
            self.load += k * (load - self.load);
            self.shared
                .cpu
                .store(self.load.to_bits(), Ordering::Release);
            // ponytail: a callback slower than its buffer counts as the underrun it makes, on Windows only: WASAPI's
            // output reports no xruns, while the other backends' reports count ([`Shared::report`]) and a slow
            // callback's xrun would count twice. Count both everywhere if the reports ever miss some.
            if load > 1.0 && cfg!(target_os = "windows") {
                self.shared.underruns.fetch_add(1, Ordering::Release);
            }
        }
    }

    /// One sub-block: the commands queued, the render, and what it publishes.
    fn render(&mut self, output: &mut [f32], channels: usize) {
        self.dispatch();
        let buffer = self.shared.buffer_frames.load(Ordering::Acquire);
        self.renderer.set_buffer_frames(buffer);
        output.fill(0.0);
        self.renderer.render(output, channels);
        // The tick before the count of commands taken: a snapshot that reads a command as taken reads the tick after it.
        self.shared
            .tick
            .store(self.renderer.transport_tick(), Ordering::Release);
        self.shared.taken.store(self.taken, Ordering::Release);
        self.shared.publish(&*self.renderer);
        let frames = output.len() / channels.max(1);
        if frames > 0 {
            self.frames += frames as u64;
            let (slot, tick) = (self.visuals % SLOTS, self.renderer.transport_tick());
            (self.meters).publish(&*self.renderer, slot, self.frames);
            let rest = MeterSnapshot::default();
            self.shared.ticks[slot].write(self.frames, tick, rest, &[]);
            self.visuals = self.visuals.wrapping_add(1);
        }
        self.meters.publish_sounding(&*self.renderer);
    }

    fn dispatch(&mut self) {
        if self.pending.is_some() && self.retired_sender.is_full() {
            return;
        }
        if let Some((renderer, meters)) = self.pending.take() {
            self.replace(renderer, meters);
        }
        while let Ok(command) = self.receiver.try_recv() {
            self.taken += 1;
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
                    AudioCommand::Metronome { on, click, level } => {
                        self.renderer.set_metronome(on, click, level)
                    }
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
    config: cpal::StreamConfig,
    channels: usize,
    mut state: CallbackState<R>,
    error: E,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + cpal::FromSample<f32> + 'static,
    R: AudioRenderer,
    E: FnMut(cpal::Error) + Send + 'static,
{
    let sample_rate = config.sample_rate;
    let mut scratch = vec![0.0_f32; SUB_BLOCK * channels];
    device.build_output_stream(
        config,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            // The first callback's size is the stream's buffer size.
            let frames = data.len() / channels.max(1);
            let _ = (state.shared.buffer_frames).compare_exchange(
                0,
                u32::try_from(frames).unwrap_or(u32::MAX),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            let out = |at: usize, rendered: &[f32]| {
                for (output, sample) in data[at * channels..].iter_mut().zip(rendered) {
                    *output = T::from_sample(if sample.is_finite() {
                        sample.clamp(-1.0, 1.0)
                    } else {
                        0.0
                    });
                }
            };
            // When its first frame is heard: the latency the host reports, up to a second (#92: a Bluetooth headset's
            // is in it; AAudio's both times are on CLOCK_MONOTONIC since cpal 0.18).
            let stamp = info.timestamp();
            let latency =
                Some(stamp.playback.duration_since(stamp.callback)).filter(|l| l.as_secs() < 1);
            let heard = clock().map(|now| now + latency.map_or(0, |l| l.as_nanos() as i64));
            state.callback(frames, channels, sample_rate, heard, &mut scratch, out);
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
        /// Each render writes the frames rendered so far into `analysis[0]`.
        stamp: bool,
        frames: u64,
    }

    impl AudioRenderer for TestRenderer {
        fn render(&mut self, output: &mut [f32], _: usize) {
            output.fill(if self.note { self.gain } else { 0.0 });
            if self.playing {
                self.tick += output.len() as u64 / 2;
            }
            self.frames += output.len() as u64 / 2;
            if self.stamp {
                self.analysis[0] = self.frames as f32;
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

        fn set_metronome(&mut self, on: bool, _click: bool, level: f32) {
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

    impl CallbackState<TestRenderer> {
        /// A stereo device callback of `output` at `now`, as the stream's.
        fn run(&mut self, output: &mut [f32], now: Option<i64>) {
            let mut scratch = vec![0.0; 2 * SUB_BLOCK];
            self.callback(output.len() / 2, 2, 48_000, now, &mut scratch, |at, r| {
                output[2 * at..2 * at + r.len()].copy_from_slice(r)
            });
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
            stamp: false,
            frames: 0,
        }
    }

    /// A 4096-frame callback (85 ms at 48 kHz) still moves the panels on every frame of a 60 Hz UI: the callback
    /// publishes a visual frame per sub-block, and the app plays them back on the output clock one buffer late, so each
    /// UI frame shows the sub-block that ended last before its time, about a UI frame's worth of audio on from the last.
    #[test]
    fn a_large_callback_reaches_the_ui_as_evenly_timed_visual_frames() {
        let (mut output, mut state) = output();
        state.renderer.stamp = true;
        let (block, rate) = (4096_u64, 48_000.0);
        let ns = |frames: u64| (frames as f64 / rate * 1e9) as i64;
        let mut buffer = vec![0.0; 2 * block as usize];
        let (mut due, mut last) = (0, 0.0);
        for frame in 0..60 {
            // Every callback due by the UI frame, then its snapshot.
            let clock = (frame as f64 * rate / 60.0) as u64;
            while due <= clock {
                state.run(&mut buffer, Some(ns(due)));
                due += block;
            }
            let shown = output.snapshot_at(Some(ns(clock))).module_meters[0].analysis[0];
            if clock >= block {
                let behind = clock as f32 - shown;
                assert!(
                    (0.0..SUB_BLOCK as f32).contains(&behind) && shown > last,
                    "UI frame {frame} at {clock}: shows {shown}, after {last}"
                );
            }
            last = shown;
        }
    }

    /// #95: a Bluetooth output asks for its callbacks in bursts, now one, now several at once, at uneven times. The
    /// visuals still move on at every frame of a 60 Hz UI, by about a frame's worth of audio and never back, and the
    /// heard tick with them.
    #[test]
    fn callbacks_in_uneven_bursts_still_reach_the_ui_evenly() {
        let (mut output, mut state) = output();
        (state.renderer.stamp, state.renderer.playing) = (true, true);
        let (block, rate) = (960_u64, 48_000.0);
        let ns = |frames: u64| (frames as f64 / rate * 1e9) as i64;
        let mut buffer = vec![0.0; 2 * block as usize];
        // The device wakes after 1 to 5 blocks' time and asks for every block its buffer lacks: it holds 6.
        let wakes = [3, 1, 5, 2, 4, 1, 1, 5, 3, 2];
        let (mut due, mut wake, mut woke, mut last) = (0, 0, 0, 0.0);
        for frame in 0..180 {
            let clock = (frame as f64 * rate / 60.0) as u64;
            while woke <= clock {
                while due <= woke + 5 * block {
                    state.run(&mut buffer, Some(ns(woke)));
                    due += block;
                }
                woke += wakes[wake % wakes.len()] * block;
                wake += 1;
            }
            let snapshot = output.snapshot_at(Some(ns(clock)));
            let shown = snapshot.module_meters[0].analysis[0];
            // Playing from 0, the tick counts the frames.
            assert_eq!(snapshot.heard_tick as f32, shown, "UI frame {frame}");
            if frame >= 30 {
                let step = shown - last;
                assert!(
                    step > 0.0 && step <= 2.0 * SUB_BLOCK as f32,
                    "UI frame {frame} at {clock}: shows {shown}, after {last}"
                );
            }
            last = shown;
        }
    }

    /// cpal 0.18 reports an xrun, a refused low-latency mode and a default output followed to a new device as errors:
    /// the stream plays on, so none fails it; a device gone does.
    #[test]
    fn only_an_error_that_stops_the_stream_fails_it() {
        use cpal::ErrorKind::{
            BackendError, DeviceChanged, DeviceNotAvailable, RealtimeDenied, Xrun,
        };
        for kind in [Xrun, RealtimeDenied, DeviceChanged] {
            assert!(!stops(&cpal::Error::new(kind)), "{kind:?}");
        }
        for kind in [DeviceNotAvailable, BackendError] {
            assert!(stops(&cpal::Error::new(kind)), "{kind:?}");
        }
    }

    /// The xruns cpal reports count as underruns (#123: the Performance counter shows them); they leave the stream
    /// running. A device gone fails it.
    #[test]
    fn a_reported_xrun_counts_as_an_underrun_and_a_device_gone_fails_the_stream() {
        let shared = Shared::default();
        shared.report(&cpal::Error::new(cpal::ErrorKind::Xrun));
        shared.report(&cpal::Error::new(cpal::ErrorKind::Xrun));
        assert_eq!(shared.underruns.load(Ordering::Acquire), 2);
        assert!(!shared.failed.load(Ordering::Acquire));
        shared.report(&cpal::Error::new(cpal::ErrorKind::DeviceNotAvailable));
        assert!(shared.failed.load(Ordering::Acquire));
        assert_eq!(shared.underruns.load(Ordering::Acquire), 2);
    }

    /// Every render call first gives the renderer the stream's buffer size, as its first callback set it.
    #[test]
    fn each_render_gives_the_renderer_the_streams_buffer_size() {
        let (mut state, _, _) = callback();
        let mut output = [0.0; 16];
        state.render(&mut output, 2);
        assert_eq!(state.renderer.buffer, 0, "before the first callback");
        state.shared.buffer_frames.store(512, Ordering::Release);
        state.render(&mut output, 2);
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
        state.render(&mut output, 2);
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
        state.render(&mut output, 2);
        assert!(state.renderer.playing);
        assert_eq!(state.renderer.tick, 16);
    }

    /// A callback state on [`renderer`] with its command queue and meters, no device.
    fn callback() -> (
        CallbackState<TestRenderer>,
        Sender<CallbackCommand<TestRenderer>>,
        Arc<MeterBank>,
    ) {
        let (output, state) = output();
        (state, output.sender.clone(), output.meters.clone())
    }

    /// The app's side of [`callback`]'s state, as [`AudioOutput::start`] makes it but with no device.
    fn output() -> (AudioOutput<TestRenderer>, CallbackState<TestRenderer>) {
        AudioOutput::unstarted(renderer(), 48_000)
    }

    /// The keys sounding hold through a replace. An adopting renderer carries them over, so until the
    /// audio thread first renders it the snapshot reads the bank it replaced (not its own, still at rest); a fresh one
    /// has none.
    #[test]
    fn the_sounding_keys_hold_through_an_adopting_replace_and_clear_for_a_fresh_one() {
        let (mut output, mut state) = output();
        state.renderer.keys = 1 << 60;
        state.render(&mut [0.0; 2], 2);
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
        state.render(&mut [0.0; 2], 2);
        assert_eq!(output.snapshot().sounding, [1 << 60], "carried over");
        // From its first render on, the new renderer's own keys.
        state.renderer.keys = 1 << 64;
        state.render(&mut [0.0; 2], 2);
        assert_eq!(output.snapshot().sounding, [1 << 64], "its own");
        assert!(
            output
                .send(AudioCommand::ReplaceSong(Box::new(renderer())))
                .is_ok()
        );
        assert_eq!(output.snapshot().sounding, [0], "not rendered yet");
        state.render(&mut [0.0; 2], 2);
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
        state.render(&mut output, 2);
        assert_eq!(state.renderer.looped, Some((128, 384)));
        assert_eq!(state.renderer.setting, Some((1, 3, 0x4E4F_5445, 0.25)));
        assert!(state.renderer.note);
        // All notes off: the note stops, the transport plays on.
        assert!(
            sender
                .try_send(CallbackCommand::Control(AudioCommand::AllNotesOff))
                .is_ok()
        );
        state.render(&mut output, 2);
        assert!(!state.renderer.note);
        assert!(state.renderer.playing);
        assert_eq!(state.renderer.tick, 16);
        assert!(
            sender
                .try_send(CallbackCommand::Control(AudioCommand::SetLoop(None)))
                .is_ok()
        );
        state.render(&mut output, 2);
        assert_eq!(state.renderer.looped, None);
    }

    #[test]
    fn a_peak_between_two_snapshots_is_read_then_held_until_the_next_render() {
        let (mut state, sender, meters) = callback();
        let mut output = [0.0; 2];
        // One callback of one frame carries a spike (peak 0.9, GR 10.8 dB); the next is quieter.
        state.renderer.note = true;
        state.renderer.gain = 0.9;
        state.render(&mut output, 2);
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
        state.render(&mut output, 2);
        let read = meters.take(u64::MAX, 48_000);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].key.instance_id, 2);
        // The spike, a frame older than the read, has fallen a frame's release: 40 dB/s at 48 kHz.
        let held = read[0].value;
        let fallen = |frames: f32| 0.9 * 10f32.powf(-2.0 * frames / 48_000.0);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-6;
        assert!(near(held.peak_left, fallen(1.0)) && near(held.peak_right, fallen(1.0)));
        assert!((held.gain_reduction_db - 10.8).abs() < 1e-5, "{held:?}");
        // The next read without a callback between reads it again; after the next callback the spike, still louder
        // than its 0.1, goes on falling at the release.
        assert_eq!(meters.take(u64::MAX, 48_000)[0].value, held);
        state.render(&mut output, 2);
        let next = meters.take(u64::MAX, 48_000)[0].value.peak_left;
        assert!((next - fallen(2.0)).abs() < 1e-6, "{next}");
    }

    #[test]
    fn the_input_peak_is_held_between_snapshots_like_the_others() {
        let (mut state, sender, meters) = callback();
        let mut output = [0.0; 2];
        state.renderer.gain = 0.4;
        state.render(&mut output, 2);
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
        state.render(&mut output, 2);
        let fallen = |frames: f32| 0.8 * 10f32.powf(-2.0 * frames / 48_000.0);
        let first = meters.take(u64::MAX, 48_000)[0].value.input_peak;
        assert!((first - fallen(1.0)).abs() < 1e-6, "{first}");
        assert_eq!(meters.take(u64::MAX, 48_000)[0].value.input_peak, first);
        state.render(&mut output, 2);
        let next = meters.take(u64::MAX, 48_000)[0].value.input_peak;
        assert!((next - fallen(2.0)).abs() < 1e-6, "{next}");
    }

    /// An edit's replace does not drop the meters to 0 until the new renderer's first callback; they read
    /// the old bank's last values under the same key, which fall at the release from then on under louder peaks.
    #[test]
    fn a_replace_reads_the_old_meters_until_the_new_renderer_renders() {
        let (mut output, mut state) = output();
        state.renderer.note = true;
        state.render(&mut [0.0; 2], 2);
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
        state.render(&mut [0.0; 2], 2);
        let next = output.snapshot().module_meters[0].value.peak_left;
        assert!(
            (next - 0.5 * 10f32.powf(-2.0 / 48_000.0)).abs() < 1e-6,
            "{next}"
        );
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
                        state.render(&mut buffer, 2);
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

    /// The level meters fall at one release whatever the sample rate, the buffer size and the UI's frame rate (#41):
    /// 40 dB/s from each peak's sub-block on (Honeydew change, #135: the earlier ×0.95 per 1024 frames was about 20).
    /// A full-scale burst over the first 2048 frames reads −20.1 dB at frame 26112, half a second after it ends at
    /// 48 kHz, and −21.8 dB at 44.1 kHz, 0.55 s after.
    #[test]
    fn a_peak_falls_at_the_meter_release_whatever_the_rate_buffer_and_frame_rate() {
        let (burst, at) = (2048, 26_112);
        let runs = [(256, 60), (2048, 60), (256, 144), (2048, 24)].map(|(b, hz)| (48_000, b, hz));
        for (rate, block, hz) in runs
            .into_iter()
            .chain([(44_100, 256, 60), (44_100, 2048, 24)])
        {
            let ns = |frames: u64| (frames as f64 / f64::from(rate) * 1e9) as i64;
            let expected = -40.0 * (at - burst) as f32 / rate as f32;
            let (mut output, mut device) = AudioOutput::offline(renderer(), rate);
            device.state.renderer.gain = 1.0;
            let mut buffer = vec![0.0; 2 * block as usize];
            let mut due = 0;
            // The UI's frames, then one at `at`; every callback due by each first, the burst's at full scale.
            let frames = (0..)
                .map(|k| k * u64::from(rate) / hz)
                .take_while(|&c| c < at);
            let mut shown = 0.0;
            for clock in frames.chain([at]) {
                while due <= clock {
                    device.run(&mut buffer, Some(ns(due)));
                    due += block;
                    if due >= burst {
                        device.state.renderer.gain = 0.0;
                    }
                }
                shown = output.snapshot_at(Some(ns(clock))).module_meters[0]
                    .value
                    .peak_left;
            }
            let db = 20.0 * shown.log10();
            assert!(
                (db - expected).abs() < 0.01,
                "{rate} Hz, {block}-frame blocks, {hz} Hz frames: {db} dB, not {expected}"
            );
        }
    }

    #[test]
    fn an_analysis_rests_until_the_first_render_then_reads_as_last_published() {
        let (mut state, _, meters) = callback();
        // Before any render: the renderer's own rest values.
        assert_eq!(meters.take(u64::MAX, 48_000)[0].analysis, [-60.0, 0.5, 7.0]);
        let mut output = [0.0; 2];
        state.renderer.analysis = vec![-12.0, 0.25, 3.0];
        state.render(&mut output, 2);
        // Not taken by a read: the next one sees it too.
        for _ in 0..2 {
            assert_eq!(
                meters.take(u64::MAX, 48_000)[0].analysis,
                [-12.0, 0.25, 3.0]
            );
        }
        // Each publish is whole: the version is even again and moved by 2.
        assert_eq!(meters.0[0].frames[0].version.load(Ordering::Acquire), 2);
        // A renderer with more values than the slots it was sized for writes what fits.
        state.renderer.analysis = vec![1.0, 2.0, 3.0, 4.0];
        state.render(&mut output, 2);
        assert_eq!(meters.take(u64::MAX, 48_000)[0].analysis, [1.0, 2.0, 3.0]);
    }

    /// Each channel's sounding keys reach the snapshot as the last render left them; a read takes nothing.
    #[test]
    fn the_sounding_keys_reach_the_snapshot_as_the_last_render_left_them() {
        let (mut state, _, meters) = callback();
        // The renderer's one meter is on channel 0: one channel, at rest before the first render.
        assert_eq!(meters.sounding(), [0]);
        state.renderer.keys = 1 << 60 | 1 << 100;
        state.render(&mut [0.0; 2], 2);
        for _ in 0..2 {
            assert_eq!(meters.sounding(), [1 << 60 | 1 << 100]);
        }
        state.renderer.keys = 0;
        state.render(&mut [0.0; 2], 2);
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
                    let frame = &bank.0[0].frames[n % SLOTS];
                    frame.write(n as u64 + 1, 0, MeterSnapshot::default(), &[n as f32; 512]);
                    std::thread::sleep(std::time::Duration::from_micros(20));
                }
            })
        };
        let mut reads = 0;
        while !writer.is_finished() || reads == 0 {
            let read = bank.take(u64::MAX, 48_000).remove(0).analysis;
            assert!(read.iter().all(|v| *v == read[0]), "torn at {}", read[0]);
            reads += 1;
        }
        writer.join().unwrap();
        assert_eq!(bank.take(u64::MAX, 48_000)[0].analysis, [1_999.0; 512]);
    }

    #[test]
    fn commands_publish_audio_position_and_meters_without_device() {
        let (sender, receiver) = bounded(8);
        let (retired_sender, retired) = bounded(8);
        let initial = renderer();
        let meters = Arc::new(MeterBank::new(&initial));
        let shared = Arc::new(Shared {
            ticks: (0..SLOTS).map(|_| Visual::new(0)).collect(),
            ..Shared::default()
        });
        let mut state = CallbackState {
            renderer: Box::new(initial),
            meters: meters.clone(),
            shared: shared.clone(),
            receiver,
            retired_sender,
            pending: None,
            taken: 0,
            frames: 0,
            visuals: 0,
            clock: None,
            load: 0.0,
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
        state.render(&mut output, 2);
        assert_eq!(output, [0.8; 16]);
        assert_eq!(shared.tick.load(Ordering::Acquire), 8);
        assert_eq!(meters.take(u64::MAX, 48_000)[0].value.peak_left, 0.8);

        assert!(
            sender
                .try_send(CallbackCommand::Replace(
                    Box::new(renderer()),
                    Arc::new(MeterBank::new(&renderer()))
                ))
                .is_ok()
        );
        state.render(&mut output, 2);
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
        state.render(&mut output, 2);
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
                click: true,
                level: 0.25,
            },
            AudioCommand::Ping(3),
        ] {
            assert!(sender.try_send(CallbackCommand::Control(command)).is_ok());
        }
        let mut output = [0.0; 16];
        state.render(&mut output, 2);
        assert_eq!(state.renderer.metronome, (true, 0.25));
        assert_eq!(snapshot(), (true, Some((3, None))));
        // A result; the count-in taken back.
        for command in [AudioCommand::CountIn(0), AudioCommand::Ping(4)] {
            assert!(sender.try_send(CallbackCommand::Control(command)).is_ok());
        }
        state.renderer.tick = 8;
        state.render(&mut output, 2);
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

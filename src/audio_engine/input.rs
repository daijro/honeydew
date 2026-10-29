//! The engine's live input: each render call reads one input block, the frames captured as its output goes out, with
//! frame-aligned input and output blocks. From that block:
//! - monitoring adds the input to its Audio Record channels' racks, ahead of their effects (left is the first port,
//!   right the second port or the first);
//! - the recording ([`RecordTap`]) takes the raw input, every port, per rendered segment while the transport rolls and
//!   The app has armed it, with the skip counter and the take rules;
//! - the latency calibration ([`Ping`]) hears its first port while its test signal is the whole output.
//!
//! The engine reads the input through its own [`InputReader`]; the ring's `Arc` comes with each build and leaves with
//! the retired engine, on the app thread. A new engine takes over the reader, the running take and the calibration of
//! the one it replaces, so an edit mid-take neither drops input nor starts a take.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

use honeydew_audio::device::PingReport;
use honeydew_audio::input::{InputReader, InputRing};
use honeydew_audio::mixer::OwnedStereoBuffer;

use super::SongAudioEngine;
use crate::recording::calibrate::Ping;
use crate::song::{ChannelKind, PPQ, Song};

/// The input's backlog, in frames, past which a block read skips ahead to half of it. The input and output clocks
/// drift apart, and a late first block leaves a backlog too; this bounds the monitoring latency to about this plus one
/// block. It must exceed the input and output callback periods together, or the reads keep skipping: 2048 frames is
/// 43 ms at 48 kHz.
// ponytail: a fixed bound and a hard skip (one discontinuity per skip); a resampler would follow drift smoothly.
pub const MONITOR_MAX_LAG_FRAMES: u64 = 2048;

/// The most frames one input block holds. The device renders at most 1024 a call; a longer call (an offline caller's)
/// reads its input in blocks this long.
const MAX_BLOCK: usize = 8192;
/// Seconds of recorded frames the tap holds for the recorder, which drains it once per app frame.
const TAP_SECONDS: usize = 2;
/// Take starts the tap holds for the recorder.
const STARTS: usize = 64;
/// Two song positions closer than this, in beats, are one: a segment starting where the last one ended goes on with its
/// take. Far less than a frame at any tempo and rate; a tempo rebuild keeps the beat to about 1e-12.
const SAME_BEAT: f64 = 1e-6;

/// What an engine build takes from the open input device: its ring, each monitoring channel's ports by
/// [`crate::song::Channel::id`] as (first, second) (a monitoring channel without an entry takes `Input 1`), and where it
/// records.
#[derive(Clone)]
pub struct LiveInput {
    pub ring: Arc<InputRing>,
    pub ports: Vec<(i32, usize, Option<usize>)>,
    pub tap: Arc<RecordTap>,
}

impl LiveInput {
    /// `ring` and `ports`, with a new tap.
    pub fn new(ring: Arc<InputRing>, ports: Vec<(i32, usize, Option<usize>)>) -> Self {
        let tap = Arc::new(RecordTap::new(&ring));
        Self { ring, ports, tap }
    }
}

/// Where a take of the tap starts ([`RecordTap::start`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakeStart {
    /// The recording it belongs to ([`RecordTap::arm`]).
    pub session: u32,
    /// The frame of [`RecordTap::frames`] its frames start at.
    pub frame: u64,
    /// The song position it starts at, in ticks (the position at Play, else the block start after arming, a jump or a
    /// loop wrap; no snapping).
    pub tick: u64,
}

/// What the engine records, for the app's recorder: every input port's frames of each take, one take after the other,
/// and where each take starts. The app arms it; the audio thread fills it; neither waits for the other. A recorder that
/// falls more than [`TAP_SECONDS`] behind loses frames.
pub struct RecordTap {
    /// The recording armed, numbered by [`Self::arm`]; 0 for none.
    armed: AtomicU32,
    /// The last number [`Self::arm`] gave out.
    sessions: AtomicU32,
    /// The skip counter S each take starts with: L with compensation, else 0; [`Self::BUFFER`] for the engine's buffer
    /// size.
    skip: AtomicU32,
    frames: Arc<InputRing>,
    /// Take starts as (session, frame, tick); start `n` at slot `n % STARTS`.
    starts: Box<[[AtomicU64; 3]]>,
    started: AtomicU64,
}

impl RecordTap {
    /// The skip that stands for L = the engine's audio buffer size in frames, which the engine knows from its first
    /// callback (no calibration).
    pub const BUFFER: u32 = u32::MAX;

    /// A tap for `input`'s ports and rate.
    pub fn new(input: &InputRing) -> Self {
        let capacity = input.rate() as usize * TAP_SECONDS;
        Self {
            armed: AtomicU32::new(0),
            sessions: AtomicU32::new(0),
            skip: AtomicU32::new(0),
            frames: Arc::new(InputRing::new(
                input.channels(),
                input.rate(),
                capacity.max(1),
            )),
            starts: (0..STARTS).map(|_| Default::default()).collect(),
            started: AtomicU64::new(0),
        }
    }

    /// Arms a new recording whose takes each drop their first `skip` frames (the skip counter's start;
    /// [`Self::BUFFER`]: the engine's buffer size), and returns its number. The engine starts a take at its next
    /// rolling block.
    pub fn arm(&self, skip: u32) -> u32 {
        let session = self
            .sessions
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1)
            .max(1);
        self.skip.store(skip, Ordering::Relaxed);
        self.armed.store(session, Ordering::Release);
        session
    }

    /// Ends the recording: the engine takes no more.
    pub fn disarm(&self) {
        self.armed.store(0, Ordering::Release);
    }

    /// The takes' frames, every port of the input, back to back.
    pub fn frames(&self) -> &Arc<InputRing> {
        &self.frames
    }

    /// Take starts so far.
    pub fn started(&self) -> u64 {
        self.started.load(Ordering::Acquire)
    }

    /// Take start `n`, while the tap still holds it. Read [`Self::frames`]' count first: the frames a reader sees have
    /// their start in place.
    pub fn start(&self, n: u64) -> Option<TakeStart> {
        if n >= self.started() {
            return None;
        }
        let [session, frame, tick] = &self.starts[(n % STARTS as u64) as usize];
        let start = TakeStart {
            session: session.load(Ordering::Relaxed) as u32,
            frame: frame.load(Ordering::Relaxed),
            tick: tick.load(Ordering::Relaxed),
        };
        // Start n + STARTS overwrites this slot once start n + STARTS − 1 is out: until then, the read is whole.
        fence(Ordering::Acquire);
        (self.started.load(Ordering::Relaxed) < n + STARTS as u64).then_some(start)
    }

    /// The armed recording and its skip; 0 for none.
    #[cfg(test)]
    pub(crate) fn armed(&self) -> (u32, u32) {
        let session = self.armed.load(Ordering::Acquire);
        (session, self.skip.load(Ordering::Relaxed))
    }

    /// The audio thread's take start: before its frames.
    pub(crate) fn mark(&self, session: u32, tick: u64) {
        let n = self.started.load(Ordering::Relaxed);
        // A reader that sees any of these stores sees `started` at n or later after its fence ([`Self::start`]).
        fence(Ordering::Release);
        let [s, frame, t] = &self.starts[(n % STARTS as u64) as usize];
        s.store(u64::from(session), Ordering::Relaxed);
        frame.store(self.frames.written(), Ordering::Relaxed);
        t.store(tick, Ordering::Relaxed);
        self.started.store(n + 1, Ordering::Release);
    }
}

/// The take the engine records.
struct Take {
    session: u32,
    /// The skip counter S: frames still to drop.
    skip: usize,
    /// Where in this block the take started: its input is read from the block's start all the same (a quirk of the
    /// first, partial block). 0 from the next block on.
    from: usize,
    /// The song beat its last segment ended at.
    end: f64,
}

pub(super) struct Monitor {
    reader: InputReader,
    /// Every port of the device, in order: what a read takes.
    ports: Vec<usize>,
    /// The input block, every port, and where it starts in the render call and how long it is.
    scratch: Vec<f32>,
    block: (usize, usize),
    /// Where the segment being rendered starts in the block.
    at: usize,
    /// Each monitoring channel: its index, and the ports its left and right take.
    channels: Vec<(usize, usize, usize)>,
    tap: Arc<RecordTap>,
    /// The tap's armed recording and skip, read once a block: a take starts at a block start.
    armed: (u32, u32),
    /// The engine's audio buffer size in frames ([`SongAudioEngine::set_input_buffer`]).
    buffer: u32,
    take: Option<Take>,
}

impl Monitor {
    /// `song`'s input path for `input`. `None` for a ring at another rate (no resampling) or without ports.
    pub(super) fn new(song: &Song, input: &LiveInput, sample_rate: f32) -> Option<Self> {
        let width = input.ring.channels();
        if width == 0 || input.ring.rate() as f32 != sample_rate {
            return None;
        }
        let channels = (song.channels.iter().enumerate())
            .filter(|(_, c)| c.kind == ChannelKind::AudioRecord && c.file.input_monitoring)
            .map(|(index, channel)| {
                let ports = input.ports.iter().find(|(id, ..)| *id == channel.id);
                let (first, second) =
                    ports.map_or((0, None), |&(_, first, second)| (first, second));
                (index, first, second.unwrap_or(first))
            })
            .collect();
        Some(Self {
            reader: InputReader::new(input.ring.clone()),
            ports: (0..width).collect(),
            scratch: vec![0.0; MAX_BLOCK * width],
            block: (0, 0),
            at: 0,
            channels,
            tap: input.tap.clone(),
            armed: (0, 0),
            buffer: 0,
            take: None,
        })
    }

    /// Reads the input block of a `total`-frame render call from `cursor` on (at most [`MAX_BLOCK`] frames of it):
    /// what has come in, the rest silence when the input runs late. A backlog past the bound, beyond what the rest of
    /// the call takes, is skipped first. Never allocates.
    fn read_block(&mut self, cursor: usize, total: usize) {
        let (rest, frames) = (total - cursor, (total - cursor).min(MAX_BLOCK));
        let keep = MONITOR_MAX_LAG_FRAMES / 2 + rest as u64;
        if self.reader.available() > MONITOR_MAX_LAG_FRAMES + rest as u64 {
            while self.reader.available() > keep {
                let skip = (self.reader.available() - keep) as usize;
                let take = skip.min(self.scratch.len());
                if self.reader.read(&[0], &mut self.scratch[..take]) == 0 {
                    break;
                }
            }
        }
        let width = self.ports.len();
        let block = &mut self.scratch[..frames * width];
        let read = self.reader.read(&self.ports, block);
        block[read * width..].fill(0.0);
        self.block = (cursor, frames);
        let session = self.tap.armed.load(Ordering::Acquire);
        self.armed = (session, self.tap.skip.load(Ordering::Relaxed));
        if let Some(take) = &mut self.take {
            take.from = 0;
        }
    }

    /// Adds the segment's input frames to each monitoring channel's buffer.
    pub(super) fn add(&mut self, buffers: &mut [OwnedStereoBuffer], frames: usize) {
        if self.channels.is_empty() {
            return;
        }
        let width = self.ports.len();
        let input = &self.scratch[self.at * width..(self.at + frames) * width];
        let read_port = |port: usize, frame: &[f32]| frame.get(port).copied().unwrap_or(0.0);
        for &(channel, left, right) in &self.channels {
            let buffer = &mut buffers[channel];
            for (i, frame) in input.chunks_exact(width).enumerate() {
                buffer.left[i] += read_port(left, frame);
                buffer.right[i] += read_port(right, frame);
            }
        }
    }

    /// Records a rendered segment of `frames` frames. `rolling`: the song beat it starts at and the beats a frame,
    /// while the transport rolls. Nothing records unless the tap was armed at the block's start and the transport rolls
    /// (so a count-in writes nothing and leaves S alone). A segment that does not start where the last one ended (a
    /// seek, a loop wrap), a new recording, or the first rolling segment starts a take: its start is marked and S = the
    /// tap's skip (L, or the engine's buffer size for [`RecordTap::BUFFER`]). Then if S ≥ n, S −= n and nothing is
    /// written; else input frames [S, n) of the segment, indexed from where the take started in this block, are written
    /// and S = 0.
    fn record(&mut self, frames: usize, rolling: Option<(f64, f64)>) {
        let (session, skip) = self.armed;
        let Some((beat, per_frame)) = rolling.filter(|_| session != 0) else {
            self.take = None;
            return;
        };
        let goes_on = (self.take.as_ref())
            .is_some_and(|t| t.session == session && (beat - t.end).abs() < SAME_BEAT);
        if !goes_on {
            self.tap
                .mark(session, (beat * f64::from(PPQ)).round() as u64);
            let skip = if skip == RecordTap::BUFFER {
                self.buffer
            } else {
                skip
            };
            self.take = Some(Take {
                session,
                skip: skip as usize,
                from: self.at,
                end: beat,
            });
        }
        let Some(take) = &mut self.take else {
            return;
        };
        take.end = beat + frames as f64 * per_frame;
        if take.skip >= frames {
            take.skip -= frames;
            return;
        }
        let width = self.ports.len();
        let base = self.at - take.from;
        let input = &self.scratch[(base + take.skip) * width..(base + frames) * width];
        self.tap.frames.push(input.iter().copied());
        take.skip = 0;
    }
}

impl SongAudioEngine {
    /// The input of the render call's segment [`cursor`, `cursor` + `frames`) of `total`: its block is read at the
    /// call's start (or as a call longer than [`MAX_BLOCK`] needs the next), then the segment records. Before the
    /// segment renders, which monitors it.
    pub(super) fn input_segment(&mut self, cursor: usize, frames: usize, total: usize) {
        let per_frame = f64::from(self.tempo_bpm) / (60.0 * f64::from(self.sample_rate));
        let rolling = self.rolling().then(|| (self.beat(), per_frame));
        let Some(monitor) = &mut self.monitor else {
            return;
        };
        let (start, len) = monitor.block;
        if cursor == 0 || cursor + frames > start + len {
            monitor.read_block(cursor, total);
        }
        monitor.at = cursor - monitor.block.0;
        monitor.record(frames, rolling);
    }

    /// While a latency measurement runs, the render call is its test signal alone, on every output channel, against the
    /// first port of the call's input block: no song, no metronome, no effects. True when it rendered the call.
    /// Without an input it fails with the unknown error at once, and the song renders.
    pub(super) fn render_ping(&mut self, output: &mut [f32], channels: usize) -> bool {
        let Some((_, ping)) = self.ping.as_mut().filter(|(_, p)| p.result().is_none()) else {
            return false;
        };
        let Some(monitor) = &mut self.monitor else {
            ping.fail();
            return false;
        };
        let total = output.len() / channels;
        let width = monitor.ports.len();
        let mut cursor = 0;
        while cursor < total {
            let frames = (total - cursor).min(MAX_BLOCK);
            monitor.read_block(cursor, total);
            let out = &mut output[cursor * channels..(cursor + frames) * channels];
            for (frame, input) in out
                .chunks_exact_mut(channels)
                .zip(monitor.scratch.chunks_exact(width))
            {
                frame.fill(ping.sample(input[0]));
            }
            cursor += frames;
        }
        self.rendered += total as u64;
        true
    }

    /// The engine's audio buffer size in frames, fixed at its stream's first callback: L without a calibration.
    pub(super) fn set_input_buffer(&mut self, frames: u32) {
        if let Some(monitor) = &mut self.monitor {
            monitor.buffer = frames;
        }
    }

    /// Starts latency measurement `id` at the next render call.
    pub(super) fn start_ping(&mut self, id: u32) {
        self.ping = Some((id, Ping::new(self.sample_rate as u32)));
    }

    pub(super) fn ping_state(&self) -> Option<PingReport> {
        self.ping.map(|(id, ping)| (id, ping.result()))
    }

    /// Takes over `old`'s input reader (the same ring), its running take (the same tap) and its latency measurement.
    /// Swaps, so nothing is freed on the audio thread. The buffer size comes before every render call.
    pub(super) fn take_input(&mut self, old: &mut Self) {
        self.ping = old.ping.take();
        if let (Some(new), Some(old)) = (&mut self.monitor, &mut old.monitor)
            && Arc::ptr_eq(new.reader.ring(), old.reader.ring())
        {
            std::mem::swap(&mut new.reader, &mut old.reader);
            if Arc::ptr_eq(&new.tap, &old.tap) {
                new.take = old.take.take();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use honeydew_audio::device::{AudioRenderer, PingFailure};

    use super::*;
    use crate::sample_cache::SampleCache;

    /// 12 800 Hz at 60 BPM: 100 frames a tick, 12 800 a beat.
    const RATE: u32 = 12_800;
    const BEAT: u64 = PPQ as u64;

    /// An engine for a 60 BPM song with an Audio Record channel, on a mono input whose frame i reads i (exact in f32
    /// below 2^24), and what its tap recorded.
    struct Rig {
        engine: SongAudioEngine,
        ring: Arc<InputRing>,
        input: LiveInput,
        reader: InputReader,
        recorded: Vec<u64>,
        pushed: u64,
    }

    impl Rig {
        /// Looping over `ticks`, if any, REC armed: the app's metronome follows the arm, so a take plays on past the
        /// song end. It clicks at level 0.
        fn new(ticks: Option<(u64, u64)>) -> Self {
            let mut song = Song {
                tempo_bpm: 60.0,
                ..Song::default()
            };
            song.add_channel(ChannelKind::AudioRecord);
            let ring = Arc::new(InputRing::new(1, RATE, 1 << 16));
            let input = LiveInput::new(ring.clone(), Vec::new());
            let mut engine = Self::engine(&song, &input);
            engine.set_metronome(true, true, 0.0);
            Self {
                engine,
                reader: InputReader::new(input.tap.frames().clone()),
                ring,
                input,
                recorded: Vec::new(),
                pushed: 0,
            }
            .looping(ticks)
        }

        fn engine(song: &Song, input: &LiveInput) -> SongAudioEngine {
            let samples = SampleCache::default();
            SongAudioEngine::with_input(song, RATE as f32, &samples, Some(input)).unwrap()
        }

        fn looping(mut self, ticks: Option<(u64, u64)>) -> Self {
            self.engine.set_loop(ticks);
            self
        }

        /// One render call of `frames` frames, its input block (the next `frames` input frames) in first.
        fn block(&mut self, frames: usize) {
            let start = self.pushed;
            self.ring
                .push((start as u32..(start + frames as u64) as u32).map(|i| i as f32));
            self.pushed += frames as u64;
            let mut output = vec![0.0; 2 * frames];
            self.engine.render(&mut output, 2);
        }

        fn blocks(&mut self, count: usize, frames: usize) {
            for _ in 0..count {
                self.block(frames);
            }
        }

        /// The takes so far, each its start tick and the input frames it holds (by number), all of recording
        /// `session`.
        fn takes(&mut self, session: u32) -> Vec<(u64, Vec<u64>)> {
            let mut frames = vec![0.0; self.reader.available() as usize];
            let read = self.reader.read(&[0], &mut frames);
            self.recorded
                .extend(frames[..read].iter().map(|&x| x as u64));
            let tap = &self.input.tap;
            let starts: Vec<_> = (0..tap.started()).map(|n| tap.start(n).unwrap()).collect();
            assert!(starts.iter().all(|s| s.session == session), "{starts:?}");
            let ends = starts.iter().skip(1).map(|s| s.frame as usize);
            let ends = ends.chain([self.recorded.len()]);
            (starts.iter().zip(ends))
                .map(|(s, end)| (s.tick, self.recorded[s.frame as usize..end].to_vec()))
                .collect()
        }
    }

    /// The input frames of `ranges`, one after the other.
    fn frames(ranges: &[Range<u64>]) -> Vec<u64> {
        ranges.iter().flat_map(|r| r.clone()).collect()
    }

    fn span(range: Range<u64>) -> Vec<u64> {
        range.collect()
    }

    #[test]
    fn a_take_starts_at_the_play_position_and_drops_its_first_l_frames() {
        // L = 300 (a calibration's) and L = the engine's buffer size, 256 (no calibration): S ≥ n drops a whole block.
        for (skip, want) in [
            (300, 300..1024),
            (RecordTap::BUFFER, 256..1024),
            (0, 0..1024),
        ] {
            let mut rig = Rig::new(None);
            rig.engine.set_buffer_frames(256);
            rig.engine.seek(5 * BEAT);
            let session = rig.input.tap.arm(skip);
            rig.engine.play();
            rig.blocks(4, 256);
            // The clip starts at the play position; its frame count excludes the dropped frames.
            assert_eq!(
                rig.takes(session),
                [(5 * BEAT, frames(&[want]))],
                "L {skip}"
            );
        }
    }

    #[test]
    fn a_take_starts_at_the_song_position_rounded_to_a_tick() {
        // Armed while playing, a take starts at the next block start: 150 or 199 frames in are 1.5 and 1.99 ticks,
        // which round to 2; 140 frames in are 1.4 ticks, which round to 1.
        for (frames, tick) in [(150, 2), (199, 2), (140, 1)] {
            let mut rig = Rig::new(None);
            rig.engine.play();
            rig.block(frames);
            let session = rig.input.tap.arm(0);
            rig.block(100);
            let want = [(tick, span(frames as u64..frames as u64 + 100))];
            assert_eq!(rig.takes(session), want, "{frames} frames");
        }
    }

    #[test]
    fn a_count_in_ending_mid_block_records_from_the_input_blocks_start() {
        // A beat of count-in, 12 800 frames, ends 800 frames into the 13th block of 1 000. Nothing records before it, S
        // included; the take starts at the play position, and its segment [800, 1000) writes input [S, 200) of that
        // block: the first 200 − S samples come from 800 frames early, and input [200, 1000) of the block is lost.
        // Aligned from the next block on.
        for (skip, want) in [
            (150, frames(&[12_150..12_200, 13_000..14_000])),
            (0, frames(&[12_000..12_200, 13_000..14_000])),
            // S ≥ 200: the segment drops 200 of it, the next block the 100 left.
            (300, span(13_100..14_000)),
        ] {
            let mut rig = Rig::new(None);
            rig.engine.seek(5 * BEAT);
            let session = rig.input.tap.arm(skip);
            rig.engine.count_in(1);
            rig.engine.play();
            rig.blocks(12, 1_000);
            assert_eq!(rig.takes(session), [], "the count-in records nothing");
            rig.blocks(2, 1_000);
            assert_eq!(rig.takes(session), [(5 * BEAT, want)], "S {skip}");
        }
    }

    #[test]
    fn a_loop_wrap_starts_a_new_take_that_re_applies_l_from_the_input_blocks_start() {
        // The loop (0, 123) ticks is 12 300 frames: the wrap comes 300 frames into the 13th block of 1 000 (n1 = 300).
        // L = 150 < n1: the old take ends with input [12 150, 12 300), and the new one, indexed from the block start,
        // writes input [150, 700) of the block, so it starts with those same frames.
        let mut rig = Rig::new(Some((0, 123)));
        let session = rig.input.tap.arm(150);
        rig.engine.play();
        rig.blocks(14, 1_000);
        let takes = rig.takes(session);
        let want = [
            (0, span(150..12_300)),
            (0, frames(&[12_150..12_700, 13_000..14_000])),
        ];
        assert_eq!(takes, want);
        assert_eq!(takes[0].1[takes[0].1.len() - 150..], takes[1].1[..150]);
        // A wrap on a block boundary: the last L frames played before it are in neither take.
        let mut rig = Rig::new(Some((0, 120)));
        let session = rig.input.tap.arm(150);
        rig.engine.play();
        rig.blocks(13, 1_000);
        let want = [(0, span(150..12_000)), (0, span(12_150..13_000))];
        assert_eq!(rig.takes(session), want);
    }

    #[test]
    fn a_seek_starts_a_take_at_its_tick_with_a_full_first_block_and_l_again() {
        let mut rig = Rig::new(None);
        let session = rig.input.tap.arm(100);
        rig.engine.play();
        rig.blocks(4, 500);
        rig.engine.seek(10 * BEAT);
        rig.blocks(2, 500);
        let want = [(0, span(100..2_000)), (10 * BEAT, span(2_100..3_000))];
        assert_eq!(rig.takes(session), want);
    }

    #[test]
    fn arming_while_playing_starts_at_the_next_block_and_disarming_or_stop_ends_the_take() {
        let mut rig = Rig::new(None);
        rig.engine.play();
        rig.blocks(3, 1_000);
        // 3 000 frames are 30 ticks.
        let session = rig.input.tap.arm(0);
        rig.blocks(2, 1_000);
        rig.input.tap.disarm();
        rig.blocks(1, 1_000);
        assert_eq!(rig.takes(session), [(30, span(3_000..5_000))]);
        // Armed again by the same recording number at a stop and Play: a take of its own.
        let mut rig = Rig::new(None);
        let session = rig.input.tap.arm(0);
        rig.engine.play();
        rig.blocks(1, 1_000);
        rig.engine.stop();
        rig.blocks(1, 1_000);
        rig.engine.play();
        rig.blocks(1, 1_000);
        let want = [(0, span(0..1_000)), (10, span(2_000..3_000))];
        assert_eq!(rig.takes(session), want);
    }

    #[test]
    fn an_engine_that_takes_over_mid_take_records_on_in_the_same_take() {
        // An edit's engine (or a fresh one) on the same input: the reader, the take and its S go on.
        let mut rig = Rig::new(None);
        let session = rig.input.tap.arm(1_500);
        rig.engine.play();
        rig.blocks(1, 1_000);
        let mut song = Song {
            tempo_bpm: 60.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::AudioRecord);
        let mut engine = Rig::engine(&song, &rig.input).adopting_edit(false, Vec::new());
        engine.take_over(&mut rig.engine);
        rig.engine = engine;
        rig.blocks(2, 1_000);
        assert_eq!(rig.takes(session), [(0, span(1_500..3_000))]);
    }

    #[test]
    fn the_latency_ping_is_the_whole_output_and_hears_the_first_port_on_the_engines_clock() {
        // A stereo input whose first port hears the output 37 frames late at half level, and whose second port is loud
        // (not listened to). Blocks of 32 frames, so each block's input is the echo of output already rendered. The
        // metronome is on and the transport plays: the output is the test signal alone all the same.
        let ring = Arc::new(InputRing::new(2, RATE, 1 << 16));
        let input = LiveInput::new(ring.clone(), Vec::new());
        let mut engine = Rig::engine(&Song::default(), &input);
        engine.set_metronome(true, true, 1.0);
        engine.play();
        engine.ping(9);
        assert_eq!(engine.ping_report(), Some((9, None)));
        let mut sent: Vec<f32> = Vec::new();
        while engine.ping_report() == Some((9, None)) {
            let t0 = sent.len();
            let echo = |t: usize| if t >= 37 { 0.5 * sent[t - 37] } else { 0.0 };
            let block: Vec<f32> = (t0..t0 + 32).flat_map(|t| [echo(t), 0.9]).collect();
            ring.push(block.into_iter());
            let mut output = vec![0.0; 2 * 32];
            engine.render(&mut output, 2);
            sent.extend(output.as_chunks::<2>().0.iter().map(|f| {
                assert_eq!(f[0], f[1], "both channels");
                f[0]
            }));
        }
        assert_eq!(engine.ping_report(), Some((9, Some(Ok(37)))));
        // Two seconds of silence, then the tone from 1.
        let fs = RATE as usize;
        assert!(sent[..2 * fs].iter().all(|&y| y == 0.0));
        assert_eq!(sent[2 * fs], 1.0);
        // Without an input: the unknown error at once.
        let mut engine = SongAudioEngine::with_input(
            &Song::default(),
            RATE as f32,
            &SampleCache::default(),
            None,
        )
        .unwrap();
        engine.ping(2);
        engine.render(&mut [0.0; 64], 2);
        assert_eq!(
            engine.ping_report(),
            Some((2, Some(Err(PingFailure::Unknown))))
        );
    }
}

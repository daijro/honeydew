//! The offline renderer behind the Save dialog's render page and `Quick Render`.
//!
//! A render runs the song twice, so that its memory does not grow with the song. [`Render::new`] builds a
//! [`SongAudioEngine`] for a copy of the song at the engine rate, seeks to the range start and plays
//! (no loop, no metronome). Pass 1, [`Render::step`], renders 1024-frame blocks of the range and then,
//! unless the remainder is `Cut`, a 30 s tail, and keeps only what the processing pass (tail fade, the 16-bit
//! intermediate, remainder, trim, normalize) needs to know. [`Render::finish`] builds the engine again: pass 2,
//! [`Stream::step`], renders the same audio and hands it out processed, block by block, to the encoder. The app runs
//! both in time slices.

use crate::audio_engine::{EngineError, SongAudioEngine};
use crate::sample_cache::SampleCache;
use crate::song::{PPQ, Song};
use crate::ui::clips::song_end_beats;
use honeydew_audio::device::AudioRenderer;

/// The render range spinner, in spinner order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderRange {
    /// [0, Project Length].
    #[default]
    Full,
    /// `Full`, less the leading frames that are exactly 0 (with `Leave` only).
    TrimSilence,
    /// The song's play loop.
    Selection,
}

/// The loop remainder spinner, in spinner order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Remainder {
    /// The faded tail is kept up to its last audible frame.
    #[default]
    Leave,
    /// No tail: exactly the range.
    Cut,
    /// The faded tail is added onto the start; exactly the range.
    Wrap,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderSpec {
    pub range: RenderRange,
    pub remainder: Remainder,
    /// `Normalize level`: × 1/peak.
    pub normalize: bool,
    /// `High quality`. Off, the render passes through 16-bit (trunc(x·32767), clamped) before the processing pass.
    pub high_quality: bool,
    /// The engine sample rate, in Hz.
    pub rate: u32,
    /// A stem: this channel soloed, every other channel's Solo off (`Split mixer tracks`).
    pub solo: Option<usize>,
    /// The master channel's rack plays. Off (a stem with `Enable master effects` off), the song copy has none.
    pub master_effects: bool,
}

#[derive(Debug)]
pub enum RenderError {
    /// The engine could not be built for the song.
    Build(EngineError),
    /// No audible frame: `The song has no sound`.
    Silent,
    /// The range is longer than [`MAX_SECS`].
    TooLong,
    /// The web's archive of the stems could not be made (past zip's 4 GiB).
    Archive,
}

/// The engine renders in blocks of this many frames.
const BLOCK: usize = 1024;
/// −60 dBFS: a frame is audible when a sample on either channel is above it.
const AUDIBLE: f32 = 0.001;
/// The tail rendered after the range end, faded out over its length.
const TAIL_SECS: usize = 30;

/// The longest range rendered, in seconds (from when the whole render was held in memory).
pub const MAX_SECS: u32 = 30 * 60;

/// The song length estimate in beats: the latest clip end rounded up to whole bars, at least one bar.
pub fn song_length_beats(song: &Song) -> f64 {
    let bar = f64::from(song.time_sig.bar_beats());
    (song_end_beats(song) / bar).ceil().max(1.0) * bar
}

/// The range in song ticks: `Full` and `TrimSilence` = [0, the song length estimate]; `Selection` = the play loop,
/// `None` without one (the range spinner then skips it).
pub fn range_ticks(song: &Song, range: RenderRange) -> Option<(f64, f64)> {
    match range {
        RenderRange::Selection => song
            .play_loop
            .filter(|(start, end)| end > start)
            .map(|(start, end)| (f64::from(start), f64::from(end))),
        _ => Some((0.0, song_length_beats(song) * f64::from(PPQ))),
    }
}

/// [`range_ticks`], with `Selection` falling back to `Full` when there is no play loop.
fn ticks(song: &Song, range: RenderRange) -> (f64, f64) {
    range_ticks(song, range).unwrap_or_else(|| (0.0, song_length_beats(song) * f64::from(PPQ)))
}

/// `Estimated length: ` in seconds: the range's length at the song tempo.
pub fn estimated_secs(song: &Song, range: RenderRange) -> f64 {
    let (start, end) = ticks(song, range);
    (end - start) / f64::from(PPQ) * 60.0 / song.tempo_bpm
}

/// Pass 1 of a render: the engine renders the range and its tail, and only what the processing pass needs is kept: the
/// peak, the first frame with a sample ≠ 0, the last audible one and, for `Wrap`, the range's first frames with the
/// tail added on. [`Render::finish`] hands over to [`Stream`], pass 2.
pub struct Render {
    pass: Pass,
    /// The song copy and the samples the engine plays, for pass 2's.
    song: Song,
    samples: SampleCache,
    spec: RenderSpec,
    /// The range start, in ticks.
    start: f64,
    kept: Kept,
}

/// What pass 1 keeps of the frames it renders, and what pass 2 hands out from it.
struct Kept {
    range: usize,
    wrap: bool,
    /// The peak of every frame, the tail's too (also the part a wrap loses).
    peak: f32,
    /// The first frame with a sample ≠ 0.
    first: Option<usize>,
    /// The last frame above −60 dBFS; with `Wrap`, of those it leaves alone.
    last: Option<usize>,
    /// `Wrap`: the range's first `overlap` frames (as many as the tail has, up to the range's), the tail added on.
    head: [Vec<f32>; 2],
    overlap: usize,
}

impl Kept {
    fn new(range: usize, tail: usize, remainder: Remainder) -> Self {
        let wrap = remainder == Remainder::Wrap;
        let overlap = if wrap { range.min(tail) } else { 0 };
        Self {
            range,
            wrap,
            peak: 0.0,
            first: None,
            last: None,
            head: [Vec::with_capacity(overlap), Vec::with_capacity(overlap)],
            overlap,
        }
    }

    /// Frame `f` as pass 1 renders it, processed.
    fn frame(&mut self, f: usize, frame: &[f32]) {
        self.peak = frame.iter().fold(self.peak, |m, x| m.max(x.abs()));
        if self.first.is_none() && frame.iter().any(|&x| x != 0.0) {
            self.first = Some(f);
        }
        if f < self.overlap {
            self.head
                .iter_mut()
                .zip(frame)
                .for_each(|(h, &x)| h.push(x));
        } else if self.wrap && f >= self.range {
            // The tail lands on the range's start; what is past its length is lost.
            let t = f - self.range;
            if t < self.overlap {
                self.head
                    .iter_mut()
                    .zip(frame)
                    .for_each(|(h, &x)| h[t] += x);
            }
        } else if frame.iter().any(|x| x.abs() > AUDIBLE) {
            self.last = Some(f);
        }
    }

    /// The frames [start, end) the stream hands out: `Silent` when none is audible. `Leave` up to the last audible
    /// one (a selection at least its range), from the first with a sample ≠ 0 with `TrimSilence`; `Cut` and `Wrap` the
    /// range.
    fn bounds(&self, spec: &RenderSpec) -> Result<(usize, usize), RenderError> {
        let wrapped = (0..self.overlap).any(|i| audible(&self.head, i));
        if self.last.is_none() && !wrapped {
            return Err(RenderError::Silent);
        }
        Ok(match spec.remainder {
            Remainder::Leave => {
                let mut end = self.last.map_or(0, |i| i + 1);
                if spec.range == RenderRange::Selection {
                    end = end.max(self.range);
                }
                let start = match spec.range {
                    RenderRange::TrimSilence => self.first.map_or(end, |f| f.min(end)),
                    _ => 0,
                };
                (start, end)
            }
            Remainder::Cut | Remainder::Wrap => (0, self.range),
        })
    }
}

/// `audio` (`range` frames, then the tail) through the two passes' processing: what the stream would hand out. The
/// processing tests run it beside [`process`].
#[cfg(test)]
fn streamed(
    audio: [Vec<f32>; 2],
    range: usize,
    spec: &RenderSpec,
) -> Result<[Vec<f32>; 2], RenderError> {
    let tail = audio[0].len() - range;
    let mut kept = Kept::new(range, tail, spec.remainder);
    let frames: Vec<[f32; 2]> = (0..audio[0].len())
        .map(|f| [0, 1].map(|c| shape(audio[c][f], f, range, tail, spec.high_quality)))
        .collect();
    (frames.iter().enumerate()).for_each(|(f, frame)| kept.frame(f, frame));
    let (start, end) = kept.bounds(spec)?;
    let peak = spec.normalize.then_some(kept.peak);
    Ok([0, 1].map(|c| {
        let head = &kept.head[c];
        (start..end)
            .map(|f| output(head, f, frames[f][c], peak))
            .collect()
    }))
}

impl Render {
    pub fn new(
        song: &Song,
        samples: &SampleCache,
        mut spec: RenderSpec,
    ) -> Result<Self, RenderError> {
        if range_ticks(song, spec.range).is_none() {
            spec.range = RenderRange::Full;
        }
        let mut song = song.clone();
        if let Some(master) = song.channels.first_mut().filter(|_| !spec.master_effects) {
            master.rack.modules.clear();
        }
        if let Some(stem) = spec.solo {
            for (index, channel) in song.channels.iter_mut().enumerate() {
                channel.params.0[3] = if index == stem { 1.0 } else { 0.0 };
            }
        }
        let (start, end) = ticks(&song, spec.range);
        let engine = engine(&song, samples, spec.rate, start)?;
        // As the engine's seek maps ticks to frames.
        let frame = |tick: f64| {
            (tick / f64::from(PPQ) * 60.0 * f64::from(spec.rate as f32)
                / f64::from(song.tempo_bpm as f32))
            .ceil()
        };
        let range = frame(end) - frame(start);
        if range.is_nan() || range > f64::from(MAX_SECS) * f64::from(spec.rate) {
            return Err(RenderError::TooLong);
        }
        let range = range as usize;
        let tail = match spec.remainder {
            Remainder::Cut => 0,
            Remainder::Leave | Remainder::Wrap => TAIL_SECS * spec.rate as usize,
        };
        Ok(Self {
            pass: Pass::new(engine, range, tail, range + tail, spec.high_quality),
            song,
            samples: samples.clone(),
            spec,
            start,
            kept: Kept::new(range, tail, spec.remainder),
        })
    }

    /// Renders at least one more block, whole blocks while `max_frames` lasts. Whether pass 1 is over. The transport
    /// stops at the range end: the tail plays no song events, voices release and effects ring out.
    pub fn step(&mut self, max_frames: usize) -> bool {
        let mut budget = max_frames.max(1);
        while budget > 0
            && let Some((at, block)) = self.pass.next()
        {
            budget = budget.saturating_sub(block.len() / 2);
            for (frame, f) in block.chunks_exact(2).zip(at..) {
                self.kept.frame(f, frame);
            }
        }
        self.pass.ended()
    }

    /// Renders what is left of pass 1 and starts pass 2: `Silent` when no frame the stream would hand out is audible.
    /// `Leave` hands out the frames up to the last audible one (a selection at least its range), from the first with
    /// a sample ≠ 0 with `TrimSilence`; `Cut` and `Wrap` the range.
    pub fn finish(mut self) -> Result<Stream, RenderError> {
        self.step(usize::MAX);
        let Self {
            pass,
            song,
            samples,
            spec,
            start: tick,
            kept,
        } = self;
        let (start, end) = kept.bounds(&spec)?;
        let (range, tail) = (pass.range, pass.tail);
        // Pass 1's engine goes before pass 2's is built.
        drop(pass);
        let engine = engine(&song, &samples, spec.rate, tick)?;
        Ok(Stream {
            pass: Pass::new(engine, range, tail, end, spec.high_quality),
            start,
            head: kept.head,
            peak: spec.normalize.then_some(kept.peak),
            out: [Vec::with_capacity(2 * BLOCK), Vec::with_capacity(2 * BLOCK)],
            handed: 0,
        })
    }
}

/// Pass 2 of a render: the engine renders the same audio again, and each block goes through the processing pass as it
/// comes: frames [start, end) of it, `Wrap`'s first frames those pass 1 kept, divided by the peak with Normalize.
/// [`Stream::step`] hands them out [`BLOCK`] frames at a time from the first, as an encoder takes them.
pub struct Stream {
    pass: Pass,
    start: usize,
    head: [Vec<f32>; 2],
    /// Normalize: the peak pass 1 found.
    peak: Option<f32>,
    /// Processed frames not handed out yet, but for the first `handed`, which the last step handed out.
    out: [Vec<f32>; 2],
    handed: usize,
}

impl Stream {
    /// The frames the stream hands out in all.
    pub fn frames(&self) -> usize {
        self.pass.end - self.start
    }

    /// Renders one more block, and hands out the next [`BLOCK`] frames (the last ones fewer) once they are there,
    /// else `None`.
    pub fn step(&mut self) -> Option<[&[f32]; 2]> {
        for out in &mut self.out {
            out.drain(..self.handed);
        }
        self.handed = 0;
        let (start, end) = (self.start, self.pass.end);
        if self.out[0].len() < BLOCK
            && let Some((at, block)) = self.pass.next()
        {
            let frames = (block.chunks_exact(2).zip(at..)).filter(|&(_, f)| f >= start);
            for (frame, f) in frames.take_while(|&(_, f)| f < end) {
                for ((&x, head), out) in frame.iter().zip(&self.head).zip(&mut self.out) {
                    out.push(output(head, f, x, self.peak));
                }
            }
        }
        let ready = self.out[0].len() >= BLOCK || (self.pass.ended() && !self.out[0].is_empty());
        if !ready {
            return None;
        }
        self.handed = self.out[0].len().min(BLOCK);
        Some([&self.out[0][..self.handed], &self.out[1][..self.handed]])
    }

    /// Every frame is handed out.
    pub fn done(&self) -> bool {
        self.pass.ended() && self.out[0].len() == self.handed
    }
}

/// The engine for `song` (the render's copy) at `rate` Hz, seeked to tick `start` and playing straight through: no
/// loop (the engine arms the song's play loop), no metronome.
fn engine(
    song: &Song,
    samples: &SampleCache,
    rate: u32,
    start: f64,
) -> Result<SongAudioEngine, RenderError> {
    let mut engine =
        SongAudioEngine::with_samples(song, rate as f32, samples).map_err(RenderError::Build)?;
    engine.set_loop(None);
    engine.play_past_song_end();
    engine.seek(start as u64);
    engine.play();
    Ok(engine)
}

/// One run of the engine over the range and its tail, up to frame `end`, in whole [`BLOCK`]-frame blocks from the range
/// start and again from its end, where the transport halts. Each block comes faded in the tail and through the 16-bit
/// intermediate. The engine is deterministic, so two runs render the same bits as far as both go.
struct Pass {
    engine: SongAudioEngine,
    /// The interleaved block the engine renders into.
    block: Vec<f32>,
    /// Frames rendered.
    done: usize,
    range: usize,
    tail: usize,
    /// Frames to render: the last block may run past it.
    end: usize,
    high_quality: bool,
}

impl Pass {
    fn new(engine: SongAudioEngine, range: usize, tail: usize, end: usize, hq: bool) -> Self {
        Self {
            engine,
            block: vec![0.0; 2 * BLOCK],
            done: 0,
            range,
            tail,
            end,
            high_quality: hq,
        }
    }

    /// Renders the next block: its first frame and its interleaved samples, processed. `None` once `end` is reached.
    fn next(&mut self) -> Option<(usize, &[f32])> {
        let done = self.done;
        if self.ended() {
            return None;
        }
        if done == self.range {
            self.engine.halt();
        }
        let stop = if done < self.range {
            self.range
        } else {
            self.range + self.tail
        };
        let frames = (stop - done).min(BLOCK);
        let block = &mut self.block[..2 * frames];
        self.engine.render(block, 2);
        for (frame, f) in block.chunks_exact_mut(2).zip(done..) {
            for x in frame {
                *x = shape(*x, f, self.range, self.tail, self.high_quality);
            }
        }
        self.done += frames;
        Some((done, block))
    }

    fn ended(&self) -> bool {
        self.done >= self.end
    }
}

/// Sample `x` of frame `f`, processed: faded in the tail (gain 1 − t/30 s over its whole length), and through the
/// 16-bit intermediate unless `high_quality`.
fn shape(x: f32, f: usize, range: usize, tail: usize, high_quality: bool) -> f32 {
    let x = if f >= range {
        x * (1.0 - (f - range) as f32 / tail as f32)
    } else {
        x
    };
    if high_quality { x } else { pcm16(x) }
}

/// Sample `x` of frame `f` as the stream hands it out: `Wrap`'s kept frame (`head`) where there is one, divided by the
/// peak with Normalize.
fn output(head: &[f32], f: usize, x: f32, peak: Option<f32>) -> f32 {
    let x = head.get(f).copied().unwrap_or(x);
    peak.map_or(x, |peak| x / peak)
}

/// A sample of frame `i` is above −60 dBFS.
fn audible(audio: &[Vec<f32>; 2], i: usize) -> bool {
    audio.iter().any(|channel| channel[i].abs() > AUDIBLE)
}

/// `x` through the 16-bit intermediate: trunc(x·32767), clamped to ±32767 (NaN → 0), as [`WavWriter`] stores it,
/// kept as the `f32` that writes back as the same integer.
///
/// [`WavWriter`]: honeydew_audio::wav_writer::WavWriter
fn pcm16(x: f32) -> f32 {
    let pcm = |x: f32| (f64::from(x) * 32767.0).clamp(-32767.0, 32767.0) as i16;
    let q = pcm(x);
    let y = f32::from(q) / 32767.0;
    // y can round to just inside q, which the truncation would take one step towards 0.
    match pcm(y) == q {
        true => y,
        false if q > 0 => y.next_up(),
        false => y.next_down(),
    }
}

/// Today's single pass, kept as the tests' oracle: the whole render held as f32, then [`process`].
#[cfg(test)]
pub(crate) fn single_pass(
    song: &Song,
    samples: &SampleCache,
    spec: RenderSpec,
) -> Result<[Vec<f32>; 2], RenderError> {
    let render = Render::new(song, samples, spec)?;
    let (mut pass, spec) = (render.pass, render.spec);
    let (range, total) = (pass.range, pass.end);
    let mut audio = [Vec::new(), Vec::new()];
    while audio[0].len() < total {
        let done = audio[0].len();
        if done == range {
            pass.engine.halt();
        }
        let end = if done < range { range } else { total };
        let frames = (end - done).min(BLOCK);
        let block = &mut pass.block[..2 * frames];
        pass.engine.render(block, 2);
        let [left, right] = &mut audio;
        left.extend(block.iter().step_by(2));
        right.extend(block.iter().skip(1).step_by(2));
    }
    process(audio, range, &spec)
}

/// The oracle's processing pass over the rendered range (`range` frames) and its tail (the rest of `audio`).
#[cfg(test)]
fn process(
    mut audio: [Vec<f32>; 2],
    range: usize,
    spec: &RenderSpec,
) -> Result<[Vec<f32>; 2], RenderError> {
    for channel in &mut audio {
        // The tail fades out linearly over its whole length: gain 1 − t/30 s.
        let tail = channel.len() - range;
        for (t, x) in channel[range..].iter_mut().enumerate() {
            *x *= 1.0 - t as f32 / tail as f32;
        }
        if !spec.high_quality {
            channel.iter_mut().for_each(|x| *x = pcm16(*x));
        }
    }
    // `Normalize level`: the peak of the rendered audio including the tail, also the part a wrap loses.
    let peak = audio.iter().flatten().fold(0f32, |m, x| m.max(x.abs()));
    let frames = audio[0].len();
    match spec.remainder {
        Remainder::Leave => {
            let mut end = (0..frames)
                .rposition(|i| audible(&audio, i))
                .map_or(0, |i| i + 1);
            if spec.range == RenderRange::Selection {
                end = end.max(range);
            }
            let start = match spec.range {
                RenderRange::TrimSilence => (0..end)
                    .position(|i| audio.iter().any(|channel| channel[i] != 0.0))
                    .unwrap_or(end),
                _ => 0,
            };
            for channel in &mut audio {
                channel.truncate(end);
                channel.drain(..start);
            }
        }
        Remainder::Cut => {}
        Remainder::Wrap => {
            for channel in &mut audio {
                let (head, tail) = channel.split_at_mut(range);
                head.iter_mut().zip(tail).for_each(|(x, t)| *x += *t);
                channel.truncate(range);
            }
        }
    }
    let frames = audio[0].len();
    if !(0..frames).any(|i| audible(&audio, i)) {
        return Err(RenderError::Silent);
    }
    // An audible frame means a peak above 0.
    if spec.normalize {
        audio.iter_mut().flatten().for_each(|x| *x /= peak);
    }
    Ok(audio)
}

#[cfg(test)]
mod tests;

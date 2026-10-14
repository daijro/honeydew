//! The pad sampler: a Drums channel's pads, each
//! playing one sample through up to [`VOICES_PER_PAD`] voices. Audio clips reuse it, one pad per clip slot.
//! [`PadSampler::new`] runs on the app thread and [`stretch`] ([`Stretching`]) there or on a pick's worker; every other
//! method runs on the audio thread and never allocates.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::sample::{MAX_FRAMES, SampleData, SampleMeta, clamp_loop};
use crate::sample_edit::speed_ratio;

/// Slot param indices (`PRMS`).
pub const LEVEL: usize = 0;
pub const PAN: usize = 1;
pub const SPEED: usize = 2;
pub const MUTE: usize = 3;
pub const SOLO: usize = 4;
pub const LENGTH: usize = 5;
pub const EG_A: usize = 6;
pub const EG_D: usize = 7;
pub const EG_S: usize = 8;
pub const EG_R: usize = 9;
pub const SAMPLE_START: usize = 10;
/// The slot params' defaults (`PRMS`).
pub const DEFAULTS: [f32; 11] = [0.8531, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.5];

/// Voices per pad.
pub const VOICES_PER_PAD: usize = 32;
/// The fast fade of a choke and an Overlap-off retrigger: target 0 with this coefficient per sample.
const FAST_FADE: f32 = 0.01;
/// A voice heading for 0 ends below this envelope level.
const SILENT: f32 = 1e-4;

/// How many frames of a `STR1` re-render are ready at once, and how many a worker adds at a time after them
/// (the first 8192 frames are stretched at once and the rest streams in).
pub const STREAM_FRAMES: usize = 8192;

/// A sample that fills in while it plays: a slot's `STR1` re-render streaming in ([`Stretching`]). Its final length is
/// allocated up front, silent. One writer appends frames in order ([`Self::push`]) and publishes how many it has written
/// with Release; the audio thread reads that with Acquire ([`Self::filled`]) before it reads them. No lock, and nothing
/// allocates once it is made.
#[derive(Debug)]
pub struct Stream {
    pub sample_rate: u32,
    /// `f32` bits.
    left: Box<[AtomicU32]>,
    right: Option<Box<[AtomicU32]>>,
    filled: AtomicUsize,
}

impl Stream {
    /// `frames` silent frames at `sample_rate`, two channels with `stereo`, none written yet. Allocates.
    // ponytail: zeroed here, on the thread that makes it (the UI's for a `Stretch`), at memset speed for the final
    // length; allocate the frames past the first ones on the writer's first push if long stretches still lag the
    // release.
    pub fn new(sample_rate: u32, frames: usize, stereo: bool) -> Self {
        let silent = || (0..frames).map(|_| AtomicU32::new(0)).collect();
        Self {
            sample_rate,
            left: silent(),
            right: stereo.then(silent),
            filled: AtomicUsize::new(0),
        }
    }

    /// The final length.
    pub fn frames(&self) -> usize {
        self.left.len()
    }

    /// How many frames are written, from the first.
    pub fn filled(&self) -> usize {
        self.filled.load(Ordering::Acquire)
    }

    /// The bytes its frames take, all channels.
    pub fn byte_len(&self) -> usize {
        Self::bytes_for(self.frames(), self.right.is_some())
    }

    /// The bytes a stream of `frames` frames takes, two channels with `stereo`: what [`Self::byte_len`] will be, before
    /// it is made.
    pub fn bytes_for(frames: usize, stereo: bool) -> usize {
        4 * frames * (1 + usize::from(stereo))
    }

    /// Writes `left`, and `right` on a stereo stream, after the frames written so far (as far as the length goes), then
    /// publishes them. One writer at a time.
    pub fn push(&self, left: &[f32], right: Option<&[f32]>) {
        let at = self.filled.load(Ordering::Relaxed);
        let channels = std::iter::once((&self.left, left)).chain(self.right.as_ref().zip(right));
        for (to, from) in channels {
            for (to, x) in to[at..].iter().zip(from) {
                to.store(x.to_bits(), Ordering::Relaxed);
            }
        }
        let filled = (at + left.len()).min(self.frames());
        self.filled.store(filled, Ordering::Release);
    }
}

/// What a pad plays: a whole sample, or a re-render still streaming in.
#[derive(Clone, Debug)]
pub enum PadSample {
    Whole(Arc<SampleData>),
    Streaming(Arc<Stream>),
}

impl PadSample {
    /// `self` plays the data `old` did: the very same (not a copy), or the whole render of `old`, a stream, now that it
    /// has landed (its length and rate): a clip voice playing the stream plays on.
    pub fn same_data(&self, old: &PadSample) -> bool {
        match (self, old) {
            (Self::Whole(a), Self::Whole(b)) => Arc::ptr_eq(a, b),
            (Self::Streaming(a), Self::Streaming(b)) => Arc::ptr_eq(a, b),
            (Self::Whole(whole), Self::Streaming(stream)) => {
                (whole.frames(), whole.sample_rate) == (stream.frames(), stream.sample_rate)
            }
            (Self::Streaming(_), Self::Whole(_)) => false,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        match self {
            Self::Whole(sample) => sample.sample_rate,
            Self::Streaming(stream) => stream.sample_rate,
        }
    }

    /// Its whole length, streamed or not.
    pub fn frames(&self) -> usize {
        match self {
            Self::Whole(sample) => sample.frames(),
            Self::Streaming(stream) => stream.frames(),
        }
    }

    /// Left and right as voices read them (a mono sample's left twice); a stream's as far as it is written now.
    fn channels(&self) -> [Frames<'_>; 2] {
        match self {
            Self::Whole(sample) => {
                let left = Frames::Whole(&sample.left);
                [left, sample.right.as_deref().map_or(left, Frames::Whole)]
            }
            Self::Streaming(stream) => {
                let filled = stream.filled();
                let left = Frames::Filled(&stream.left[..filled]);
                let right = stream.right.as_deref();
                [left, right.map_or(left, |r| Frames::Filled(&r[..filled]))]
            }
        }
    }
}

/// One channel of a pad's sample as a voice reads it: whole, or the frames of a stream written so far.
#[derive(Clone, Copy)]
enum Frames<'a> {
    Whole(&'a [f32]),
    Filled(&'a [AtomicU32]),
}

impl Frames<'_> {
    /// Frame `i`; silence past the frames there are. A
    /// voice that reaches frames not yet streamed reads silence there, and plays on.
    fn get(self, i: usize) -> f32 {
        let frame = match self {
            Self::Whole(data) => data.get(i).copied(),
            Self::Filled(data) => data
                .get(i)
                .map(|x| f32::from_bits(x.load(Ordering::Relaxed))),
        };
        frame.unwrap_or(0.0)
    }
}

/// One pad as the sampler plays it: a slot's sample and settings.
#[derive(Clone, Debug)]
pub struct Pad {
    /// `None`: a silent pad (no sample, or a missing one). Hits on it still choke its cut group.
    pub sample: Option<PadSample>,
    /// The slot params, normalized, indexed by [`LEVEL`] … [`SAMPLE_START`].
    pub params: [f32; 11],
    /// The effective note that live notes play it with; `None` for no key.
    pub note: Option<u8>,
    /// `MAIN`@1036: the voices it launches play the sample backwards.
    pub reverse: bool,
    /// `MAIN`@1049: 0 none, else a hit or note-off here fast-fades the group's other pads.
    pub cut_group: u8,
    /// `MAIN`@1050: false, a retrigger fast-fades the pad's own voices first.
    pub overlap: bool,
}

impl Default for Pad {
    fn default() -> Self {
        Self {
            sample: None,
            params: DEFAULTS,
            note: None,
            reverse: false,
            cut_group: 0,
            overlap: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Stage {
    #[default]
    Attack,
    Decay,
    Release,
    Fade,
}

#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    active: bool,
    stage: Stage,
    env: f32,
    /// c_A, fixed at the voice's start.
    attack: f32,
    /// Source frames, in the reversed sample when it plays reversed; negative is silence still to come.
    pos: f64,
    /// The pad's reverse when the voice launched.
    reverse: bool,
    /// The hit's pitch ratio.
    pitch: f64,
    /// Velocity × the hit pan, left and right.
    gain: [f32; 2],
    /// Output frames since the start, for the Length gate.
    age: u64,
}

/// A kit's pads and their voices.
pub struct PadSampler {
    pads: Vec<Pad>,
    voices: Vec<Voice>,
    /// The channel Pitch Bend ratio 2^w.
    bend: f64,
    sample_rate: f32,
    /// Some pad's Solo is on: kept at each Solo write, so `process` does not rescan every pad for every pad.
    soloed: bool,
}

impl PadSampler {
    /// The sampler for `pads` at device rate `sample_rate`, with every voice allocated.
    pub fn new(pads: Vec<Pad>, sample_rate: f32) -> Self {
        Self {
            voices: vec![Voice::default(); pads.len() * VOICES_PER_PAD],
            soloed: pads.iter().any(|p| p.params[SOLO] > 0.5),
            pads,
            bend: 1.0,
            sample_rate,
        }
    }

    pub fn pads(&self) -> usize {
        self.pads.len()
    }

    /// Pad `pad`'s sample, `None` for a silent pad or one out of range.
    pub fn sample(&self, pad: usize) -> Option<&PadSample> {
        self.pads.get(pad)?.sample.as_ref()
    }

    /// Slot param `index` of `pad`; anything out of range is ignored. Speed, Level, Pan, Mute, Solo, D, S, R and Length
    /// act on the voices already playing from the next block.
    pub fn set_param(&mut self, pad: usize, index: usize, value: f32) {
        if value.is_finite()
            && (0.0..=1.0).contains(&value)
            && let Some(param) = self.pads.get_mut(pad).and_then(|p| p.params.get_mut(index))
        {
            *param = value;
            if index == SOLO {
                self.soloed = self.pads.iter().any(|p| p.params[SOLO] > 0.5);
            }
        }
    }

    /// `MAIN`@1036 of `pad` (the strip's `REV`): each voice takes it when it launches, so the voices already
    /// playing keep their direction. Nothing is re-rendered.
    pub fn set_reverse(&mut self, pad: usize, on: bool) {
        if let Some(p) = self.pads.get_mut(pad) {
            p.reverse = on;
        }
    }

    /// `MAIN`@1049 of `pad` (the strip's `Cut...`): 0 none, else the group its hits and note-offs choke.
    pub fn set_cut_group(&mut self, pad: usize, group: u8) {
        if let Some(p) = self.pads.get_mut(pad) {
            p.cut_group = group;
        }
    }

    /// `MAIN`@1050 of `pad` (the strip's `Poly`), read at each hit.
    pub fn set_overlap(&mut self, pad: usize, on: bool) {
        if let Some(p) = self.pads.get_mut(pad) {
            p.overlap = on;
        }
    }

    /// The channel's Pitch Bend v: drums apply 2^w, w = 4v − 2.
    pub fn set_bend(&mut self, v: f32) {
        if v.is_finite() {
            self.bend = f64::from(4.0 * v.clamp(0.0, 1.0) - 2.0).exp2();
        }
    }

    /// A hit on `pad`: `velocity` is linear amplitude, `pan` the hit pan 0..1 (0.5 centre), `pitch` a ratio
    /// ([`hit_pitch`]). It chokes the pad's cut group, then starts a voice.
    pub fn hit(&mut self, pad: usize, velocity: f32, pan: f32, pitch: f32) {
        self.hit_at(pad, velocity, pan, pitch, 0.0);
    }

    /// [`Self::hit`] from source time `offset` seconds, added to Sample Start and not scaled by Speed: an audio clip
    /// starts its slot at its content offset this way.
    pub fn hit_at(&mut self, pad: usize, velocity: f32, pan: f32, pitch: f32, offset: f64) {
        let valid = velocity.is_finite() && pan.is_finite() && pitch.is_finite() && pitch > 0.0;
        let Some(p) = self.pads.get(pad).filter(|_| valid && offset.is_finite()) else {
            return;
        };
        let (overlap, reverse, params) = (p.overlap, p.reverse, p.params);
        let Some(rate) = p.sample.as_ref().map(PadSample::sample_rate) else {
            self.choke(pad);
            return;
        };
        self.choke(pad);
        let voices = &mut self.voices[pad * VOICES_PER_PAD..][..VOICES_PER_PAD];
        if !overlap {
            voices.iter_mut().for_each(fade);
        }
        let slot = voices.iter().position(|v| !v.active).unwrap_or_else(|| {
            let furthest = |a: &(usize, &Voice), b: &(usize, &Voice)| a.1.pos.total_cmp(&b.1.pos);
            voices
                .iter()
                .enumerate()
                .max_by(furthest)
                .map_or(0, |(i, _)| i)
        });
        let pan = pan.clamp(0.0, 1.0);
        voices[slot] = Voice {
            active: true,
            stage: Stage::Attack,
            env: 0.0,
            attack: coefficient(params[EG_A], 500.0),
            pos: (sample_start_seconds(params[SAMPLE_START]) + offset) * f64::from(rate),
            reverse,
            pitch: f64::from(pitch),
            gain: [
                velocity * (2.0 - 2.0 * pan).min(1.0),
                velocity * (2.0 * pan).min(1.0),
            ],
            age: 0,
        };
    }

    /// A note-off on `pad` (a live key-up or a drum notes lane's): its voices release with c_R, and its cut group's
    /// other pads fast-fade.
    pub fn release(&mut self, pad: usize) {
        if pad >= self.pads.len() {
            return;
        }
        self.voices[pad * VOICES_PER_PAD..][..VOICES_PER_PAD]
            .iter_mut()
            .for_each(release);
        self.choke(pad);
    }

    /// A live or sequenced note: every pad whose effective note is `key` plays, pan and pitch centred.
    pub fn note_on(&mut self, key: u8, velocity: f32) {
        for pad in 0..self.pads.len() {
            if self.pads[pad].note == Some(key) {
                self.hit(pad, velocity, 0.5, 1.0);
            }
        }
    }

    /// Releases every pad whose effective note is `key`.
    pub fn note_off(&mut self, key: u8) {
        for pad in 0..self.pads.len() {
            if self.pads[pad].note == Some(key) {
                self.release(pad);
            }
        }
    }

    /// Fast-fades every voice of `pad` (c = 0.01 per sample), as a choke does: an audio clip's loop repetition and
    /// end.
    pub fn fade(&mut self, pad: usize) {
        if pad < self.pads.len() {
            self.voices[pad * VOICES_PER_PAD..][..VOICES_PER_PAD]
                .iter_mut()
                .for_each(fade);
        }
    }

    /// Releases every voice (Stop, AllNotesOff, the loop wrap); a fading voice keeps fading.
    pub fn all_notes_off(&mut self) {
        self.voices.iter_mut().for_each(release);
    }

    /// Silences every voice at once (a re-sync's cut of a channel whose first lane is audio); params kept.
    pub fn reset(&mut self) {
        self.voices.fill(Voice::default());
    }

    /// Takes over `old`'s sound (the sampler this one replaces after an edit): pad `p` gets the voices of `old`'s pad
    /// `map(p)`, sounding ones included. The voices play this sampler's
    /// pads from the frame they are at: a pad's new sample, its new output; the channel bend is the host's to set.
    /// Allocates nothing.
    pub fn adopt(&mut self, old: &PadSampler, map: impl Fn(usize) -> Option<usize>) {
        for pad in 0..self.pads.len() {
            if let Some(from) = map(pad).filter(|&from| from < old.pads.len()) {
                self.voices[pad * VOICES_PER_PAD..][..VOICES_PER_PAD]
                    .copy_from_slice(&old.voices[from * VOICES_PER_PAD..][..VOICES_PER_PAD]);
            }
        }
    }

    /// How many of `pad`'s voices sound.
    pub fn active_voices(&self, pad: usize) -> usize {
        let voices = self.voices.chunks(VOICES_PER_PAD).nth(pad);
        voices.map_or(0, |v| v.iter().filter(|v| v.active).count())
    }

    /// Renders `pad`'s voices for one block and adds them into `left` and `right` (equal lengths). Speed, bend,
    /// Level, Pan, Mute/Solo, D, S, R and Length are read once per block.
    pub fn process(&mut self, pad: usize, left: &mut [f32], right: &mut [f32]) {
        let soloed = self.soloed;
        let Some(p) = self.pads.get(pad) else { return };
        let Some(sample) = &p.sample else {
            // Nothing to play: the voices it has (taken over from a pad that had a sample) end rather than wait,
            // frozen, for a later edit to give it one back.
            self.voices[pad * VOICES_PER_PAD..][..VOICES_PER_PAD].fill(Voice::default());
            return;
        };
        let params = &p.params;
        let audible = params[MUTE] <= 0.5 && (!soloed || params[SOLO] > 0.5);
        let level = if audible {
            level_gain(params[LEVEL])
        } else {
            0.0
        };
        let pan = 2.0 * params[PAN] - 1.0;
        let out = [level * (1.0 - pan.max(0.0)), level * (1.0 + pan.min(0.0))];
        // Speed: exp2f(s/12), s = trunc(fma_f32(v, 48, −24)) whole semitones.
        let step = speed_ratio(params[SPEED]) * self.bend * f64::from(sample.sample_rate())
            / f64::from(self.sample_rate);
        let (decay, sustain, release) = (
            coefficient(params[EG_D], 50.0),
            params[EG_S],
            coefficient(params[EG_R], 50.0),
        );
        let frames = sample.frames();
        let gate =
            (params[LENGTH] > 0.0).then(|| (frames as f64 * f64::from(params[LENGTH])) as u64);
        let last = frames as f64 - 1.0;
        let data = sample.channels();
        let voices = &mut self.voices[pad * VOICES_PER_PAD..][..VOICES_PER_PAD];
        for voice in voices.iter_mut().filter(|v| v.active) {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                if voice.pos > last {
                    voice.active = false;
                    break;
                }
                if gate.is_some_and(|gate| voice.age >= gate)
                    && matches!(voice.stage, Stage::Attack | Stage::Decay)
                {
                    voice.stage = Stage::Release;
                }
                let (target, c) = match voice.stage {
                    Stage::Attack => (5.0, voice.attack),
                    Stage::Decay => (sustain, decay),
                    Stage::Release => (0.0, release),
                    Stage::Fade => (0.0, FAST_FADE),
                };
                voice.env += (target - voice.env) * c;
                if voice.stage == Stage::Attack && voice.env > 1.0 {
                    voice.env = 1.0;
                    voice.stage = Stage::Decay;
                } else if target == 0.0 && voice.env < SILENT {
                    voice.active = false;
                    break;
                }
                let (reverse, pos) = (voice.reverse, voice.pos);
                *l += read(data[0], frames, pos, reverse) * voice.env * voice.gain[0] * out[0];
                *r += read(data[1], frames, pos, reverse) * voice.env * voice.gain[1] * out[1];
                voice.pos += step * voice.pitch;
                voice.age += 1;
            }
        }
    }

    /// Fast-fades every other pad in `pad`'s cut group.
    fn choke(&mut self, pad: usize) {
        let group = self.pads[pad].cut_group;
        if group == 0 {
            return;
        }
        let pads = self.pads.iter().enumerate();
        for (other, _) in pads.filter(|(i, p)| *i != pad && p.cut_group == group) {
            self.voices[other * VOICES_PER_PAD..][..VOICES_PER_PAD]
                .iter_mut()
                .for_each(fade);
        }
    }
}

fn fade(voice: &mut Voice) {
    voice.stage = Stage::Fade;
}

fn release(voice: &mut Voice) {
    if voice.stage != Stage::Fade {
        voice.stage = Stage::Release;
    }
}

/// `data`, of `frames` frames, at source position `pos` by linear interpolation; silence before 0 and past the end.
/// Reversed, position 0 is the last frame.
fn read(data: Frames, frames: usize, pos: f64, reverse: bool) -> f32 {
    if pos < 0.0 {
        return 0.0;
    }
    let (i, t) = (pos as usize, pos.fract() as f32);
    let at = |k: usize| match frames.checked_sub(k + 1) {
        Some(back) => data.get(if reverse { back } else { k }),
        None => 0.0,
    };
    at(i) + (at(i + 1) - at(i)) * t
}

/// Slot Level v as gain: dB = 66·v^0.6 − 60; v = 0 plays at −60 dB, not silence.
pub fn level_gain(v: f32) -> f32 {
    10f32.powf((66.0 * v.powf(0.6) - 60.0) / 20.0)
}

/// Slot Sample Start v in seconds: the signed square ±0.5·u², u = 2v − 1.
fn sample_start_seconds(v: f32) -> f64 {
    let u = 2.0 * v - 1.0;
    f64::from(0.5 * u * u * u.signum())
}

/// An envelope coefficient per sample: 0.0002 / (scale·x⁴), x = 0.95v + 0.05, with scale 500
/// for A and 50 for D and R; 1 (instant) when v < 0.001 or scale·x⁴ < 0.001.
fn coefficient(v: f32, scale: f32) -> f32 {
    let denominator = scale * (0.95 * v + 0.05).powi(4);
    if v < 0.001 || denominator < 0.001 {
        1.0
    } else {
        0.0002 / denominator
    }
}

/// A drum hit's per-hit Pitch (`EVN2` value) as a ratio: 2^(2·value/65535 − 1), ±12 semitones, continuous.
pub fn hit_pitch(value: u16) -> f32 {
    (2.0 * f32::from(value) / 65535.0 - 1.0).exp2()
}

/// WSOLA grain length of the load-time stretch, in source frames. A standard waveform-similarity
/// overlap-add.
const GRAIN: usize = 1024;
/// How far, in frames, WSOLA may move a grain to line it up with the audio before it.
const SEEK: isize = 256;

/// A slot's sample with its `STR1` stretch and pitch rendered: the length
/// times `ratio` (output/source; 1 when the mode is off) and the pitch shifted by 2^(round(st)/12) without a length
/// change. Never on the audio thread: it allocates and costs about (2·[`SEEK`] + 1)·frames multiply-adds. Ratio
/// and semitones are clamped to the `STRC` normalized ranges, ×¼…×4 and ±24.
pub fn stretch(sample: &SampleData, ratio: f64, semitones: f64) -> SampleData {
    let (ratio, pitch) = factors(ratio, semitones);
    if ratio == 1.0 && pitch == 1.0 || sample.frames() == 0 {
        return sample.clone();
    }
    Stretching::new(sample, ratio, semitones).finish()
}

/// The length in frames of [`stretch`]'s render of `frames` source frames: before anything renders, for a byte budget.
pub fn stretch_frames(frames: usize, ratio: f64, semitones: f64) -> usize {
    match factors(ratio, semitones) {
        (1.0, 1.0) => frames,
        (ratio, _) => ((frames as f64 * ratio).round() as usize).min(MAX_FRAMES),
    }
}

/// `STR1`'s (ratio, pitch ratio) as [`stretch`] renders them.
fn factors(ratio: f64, semitones: f64) -> (f64, f64) {
    let ratio = if ratio.is_finite() {
        ratio.clamp(0.25, 4.0)
    } else {
        1.0
    };
    let pitch = if semitones.is_finite() {
        (semitones.clamp(-24.0, 24.0).round() / 12.0).exp2()
    } else {
        1.0
    };
    (ratio, pitch)
}

/// One channel of a source at a new pitch, resampled only as far as a render has needed it: at pitch 1 the source
/// itself (the same values as a resample, for finite frames).
struct Pitched {
    source: Arc<[f32]>,
    pitch: f64,
    /// The frames resampled so far (none at pitch 1).
    done: Vec<f32>,
}

impl Pitched {
    fn new(source: &Arc<[f32]>, pitch: f64) -> Self {
        Self {
            source: source.clone(),
            pitch,
            done: Vec::new(),
        }
    }

    /// Resamples its frames up to `upto` (at most its length, source frames / pitch).
    fn reach(&mut self, upto: usize) {
        if self.pitch == 1.0 {
            return;
        }
        let (source, pitch) = (&self.source, self.pitch);
        let upto = upto.min((source.len() as f64 / pitch) as usize);
        let at = |i: usize| read(Frames::Whole(source), source.len(), i as f64 * pitch, false);
        let from = self.done.len().min(upto);
        self.done.extend((from..upto).map(at));
    }

    /// Frame `i` (resampled by [`Self::reach`]); silence past the end.
    fn get(&self, i: usize) -> f32 {
        let data = if self.pitch == 1.0 {
            &self.source[..]
        } else {
            &self.done[..]
        };
        data.get(i).copied().unwrap_or(0.0)
    }
}

/// A [`stretch`] rendered in order into a [`Stream`], so that a pad plays it as it fills: the pitch by resampling,
/// then the length by WSOLA at ratio·pitch, its grains placed on the left channel
/// and shared by both. Making it and rendering the first frames cost the same whatever the source's length (the
/// stream's zeroing aside): the source resamples as far as the frames rendered need. Never on the audio thread.
pub struct Stretching {
    stream: Arc<Stream>,
    /// The source at the new pitch, left then right.
    source: (Pitched, Option<Pitched>),
    /// The length factor WSOLA stretches by: ratio·pitch.
    factor: f64,
    /// The source start of each output grain so far ([`Self::grains`]).
    starts: Vec<isize>,
    meta: SampleMeta,
}

impl Stretching {
    /// The render of `sample` at (`ratio`, `semitones`), its [`Stream`] allocated at the final length
    /// ([`stretch_frames`]) with nothing written; when that changes nothing (the source itself), all of it written at
    /// once.
    pub fn new(sample: &SampleData, ratio: f64, semitones: f64) -> Self {
        let frames = stretch_frames(sample.frames(), ratio, semitones);
        let (ratio, pitch) = factors(ratio, semitones);
        let stream = Arc::new(Stream::new(
            sample.sample_rate,
            frames,
            sample.right.is_some(),
        ));
        if ratio == 1.0 && pitch == 1.0 {
            stream.push(&sample.left, sample.right.as_deref());
        }
        // The loop and cues move with the audio, by the length actually rendered, and are fitted
        // to it as at load (a cue past the end is held to it).
        let scale = frames as f64 / sample.frames() as f64;
        let to = |x: u32| (f64::from(x) * scale).round() as u32;
        let mut meta = sample.meta.clone();
        meta.loop_frames = meta
            .loop_frames
            .map(|(start, end)| clamp_loop((to(start), to(end)), frames));
        for cue in &mut meta.cues {
            *cue = to(*cue).min(frames as u32);
        }
        let right = sample
            .right
            .as_ref()
            .map(|right| Pitched::new(right, pitch));
        Self {
            stream,
            source: (Pitched::new(&sample.left, pitch), right),
            factor: ratio * pitch,
            starts: Vec::with_capacity(frames / (GRAIN / 2) + 2),
            meta,
        }
    }

    /// The stream it renders into.
    pub fn stream(&self) -> &Arc<Stream> {
        &self.stream
    }

    /// Renders the frames up to `upto` (at most the length) not written yet, and publishes them.
    pub fn fill(&mut self, upto: usize) {
        let (from, upto) = (self.stream.filled(), upto.min(self.stream.frames()));
        if from >= upto {
            return;
        }
        let hop = GRAIN / 2;
        // Output grain j covers frames (j − 1)·hop up to (j + 1)·hop, Hann-windowed (periodic, so 50 % overlaps sum to
        // 1): frame o sums grains o/hop and o/hop + 1, in that order. Grains up to `last` read the source up to their
        // nominal start (j·hop/factor − hop) + SEEK + GRAIN.
        let last = (upto - 1) / hop + 1;
        let reach = ((last + 1) * hop) as f64 / self.factor;
        let reach = reach as usize + SEEK as usize + GRAIN + 2;
        self.source.0.reach(reach);
        if let Some(right) = &mut self.source.1 {
            right.reach(reach);
        }
        self.grains(last);
        let starts = &self.starts;
        let render = |data: &Pitched| -> Vec<f32> {
            let frame = |o: usize| {
                let mut sum = 0.0;
                for j in [o / hop, o / hop + 1] {
                    let n = o + hop - j * hop;
                    let source = usize::try_from(starts[j] + n as isize).ok();
                    let value = source.map_or(0.0, |i| data.get(i));
                    let window = (std::f32::consts::PI * n as f32 / GRAIN as f32)
                        .sin()
                        .powi(2);
                    sum += window * value;
                }
                sum
            };
            (from..upto).map(frame).collect()
        };
        let left = render(&self.source.0);
        let right = self.source.1.as_ref().map(render);
        self.stream.push(&left, right.as_deref());
    }

    /// The source starts of the grains up to `last`: each the nominal j·hop/factor − hop, moved by up to [`SEEK`] to
    /// best continue the grain before it.
    fn grains(&mut self, last: usize) {
        let hop = GRAIN / 2;
        let data = &self.source.0;
        let at = |i: isize| usize::try_from(i).map_or(0.0, |i| data.get(i));
        while self.starts.len() <= last {
            let j = self.starts.len();
            let nominal = ((j * hop) as f64 / self.factor) as isize - hop as isize;
            let Some(&previous) = self.starts.last() else {
                self.starts.push(nominal);
                continue;
            };
            let natural = previous + hop as isize;
            let score = |start: isize| -> f32 {
                (0..hop as isize)
                    .map(|n| at(start + n) * at(natural + n))
                    .sum()
            };
            let best = (nominal - SEEK..=nominal + SEEK)
                .map(|start| (start, score(start)))
                .fold((nominal, f32::MIN), |best, next| {
                    if next.1 > best.1 { next } else { best }
                });
            self.starts.push(best.0);
        }
    }

    /// The whole render: what is left of it rendered now, [`STREAM_FRAMES`] at a time, read back from the stream.
    pub fn finish(mut self) -> SampleData {
        while self.stream.filled() < self.stream.frames() {
            self.fill(self.stream.filled() + STREAM_FRAMES);
        }
        let read = |data: &[AtomicU32]| -> Arc<[f32]> {
            (data.iter())
                .map(|x| f32::from_bits(x.load(Ordering::Relaxed)))
                .collect()
        };
        let stream = &self.stream;
        SampleData {
            sample_rate: stream.sample_rate,
            left: read(&stream.left),
            right: stream.right.as_deref().map(read),
            meta: self.meta,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many source frames `render` has resampled, both channels.
    fn resampled(render: &Stretching) -> usize {
        let right = render.source.1.as_ref().map_or(0, |r| r.done.len());
        render.source.0.done.len() + right
    }

    #[test]
    fn the_first_frames_cost_the_same_whatever_the_source_length() {
        // A `Stretch` on a long loop resamples only what its first frames read (none at pitch 1, the
        // knob's), so its inline part does not grow with the source.
        let source = |seconds: usize| {
            let left: Arc<[f32]> = (0..seconds * 44_100)
                .map(|i| (i as f32 * 0.01).sin())
                .collect();
            SampleData {
                sample_rate: 44_100,
                right: Some(left.clone()),
                left,
                ..Default::default()
            }
        };
        let first = |sample: &SampleData, semitones: f64| {
            let mut render = Stretching::new(sample, 2.0, semitones);
            render.fill(STREAM_FRAMES);
            assert_eq!(render.stream.filled(), STREAM_FRAMES);
            resampled(&render)
        };
        let (short, long) = (source(1), source(60));
        assert_eq!((first(&short, 0.0), first(&long, 0.0)), (0, 0));
        let pitched = first(&short, 7.0);
        assert!(pitched > 0 && pitched < 4 * STREAM_FRAMES, "{pitched}");
        assert_eq!(first(&long, 7.0), pitched);
    }
}

//! MP3 writing: `Mp3Encoder` takes the render in
//! slices, as `FlacEncoder` does, and makes a stereo CBR MPEG Layer III file, 320 kbps with `High quality` on and
//! 192 kbps off, through rusty_mp3 (vendored in `vendor/rusty_mp3`, Apache-2.0). Mono input is duplicated.
//!
//! - **Gapless.** The codec delays the audio by 1057 frames (rusty_mp3's 528 plus a decoder's 529) and pads the end
//!   to a whole frame. The file opens with an Info frame that carries a LAME-style tag recording both (see
//!   [`lame_tag`]), so a decoder that reads it (symphonia, ffmpeg) trims them and plays exactly the input, in place,
//!   at the stream rate. The input is followed by silence as long as that delay ([`TAIL`]), so a decoder that ignores
//!   the tag still plays all of it, and the end a decoder trims stays under a frame.
//!
//! - **Rate.** Above 48 kHz the audio is halved (96 → 48 kHz), and again while that leaves 32 kHz or more
//!   (192 → 96 → 48, 176.4 → 88.2 → 44.1 kHz), each time through a windowed-sinc low-pass. A rate that is then none of
//!   the Layer III rates [`RATES`] is resampled linearly to the nearest one (50 kHz goes to 48, not 25).
//! - **Quality.** LAME's encoder quality (`-q 2` on, `-q 5` off) has no rusty_mp3 equivalent, so only the bitrate
//!   differs. Below 32 kHz the MPEG-2/2.5 bitrates stop at 160 kbps, and both settings snap to that.
//!
//! This file builds on its own, with no `crate::` paths: `tools/wasm-smoke` includes it by `#[path]`.

use std::f64::consts::{PI, TAU};

/// The Layer III sample rates written. Not 8 kHz: rusty_mp3 1.0.0 codes it wrongly above about 700 Hz (a 1 kHz tone
/// decodes at 9 dB SNR through symphonia and through rusty_mp3's own decoder alike), so 8 kHz goes to 11.025 kHz.
const RATES: [u32; 8] = [
    11_025, 12_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000,
];
/// Taps of the 2:1 low-pass: odd, so its centre tap lines the output up with the input (no delay).
const TAPS: usize = 97;
/// rusty_mp3's own delay in frames, as the LAME tag records it.
const DELAY: usize = 528;
/// The silence after the input, in frames out of the rate conversion: the codec's delay, rusty_mp3's [`DELAY`] and a
/// decoder's 529. Not more: the end a decoder trims then stays under a frame, as LAME's does (a whole trimmed frame
/// would have `sample::decode` fill the gap to its timestamp with silence).
const TAIL: usize = DELAY + 529;
/// The LAME tag's 9-byte encoder string: `LAME` for the family of tags that decoders read the delay and
/// padding from (symphonia takes `LAME`, `Lavf` and `Lavc`, and checks the tag's CRC for `LAME`), then `rmp3 ` for
/// rusty_mp3 instead of a LAME version number.
const ENCODER: &[u8; 9] = b"LAMErmp3 ";

/// One step of one channel's rate conversion, streaming.
enum Stage {
    /// 2:1 decimation through [`lowpass`]: the input not yet under a window, after `TAPS / 2` samples of leading
    /// silence.
    Halve(Vec<f32>),
    /// Linear interpolation, `step` input samples per output sample: the input not yet passed and the position of
    /// the next output in it.
    ///
    /// ponytail: no low-pass (images and folds from about -34 dB). Only 8 kHz and rates that halving cannot bring to
    /// a Layer III rate come here; a windowed-sinc resampler if a device at one of them matters.
    Linear { step: f64, at: f64, input: Vec<f32> },
}

impl Stage {
    fn run(&mut self, audio: &[f32], taps: &[f32; TAPS]) -> Vec<f32> {
        match self {
            Self::Halve(input) => {
                input.extend_from_slice(audio);
                let windows = input.windows(TAPS).step_by(2);
                let out = windows
                    .map(|w| w.iter().zip(taps).map(|(x, h)| x * h).sum())
                    .collect::<Vec<f32>>();
                input.drain(..2 * out.len());
                out
            }
            Self::Linear { step, at, input } => {
                input.extend_from_slice(audio);
                let mut out = Vec::new();
                while (*at as usize) + 1 < input.len() {
                    let (i, t) = (*at as usize, at.fract() as f32);
                    out.push(input[i] + (input[i + 1] - input[i]) * t);
                    *at += *step;
                }
                let passed = (*at as usize).min(input.len());
                input.drain(..passed);
                *at -= passed as f64;
                out
            }
        }
    }
}

/// The 2:1 low-pass: a Blackman-windowed sinc cut off at 0.23 of the input rate (flat to about 0.2, stopped from
/// about 0.26, so nothing folds below 0.24 of it: 23 kHz from 96 kHz), unit gain at DC.
fn lowpass() -> [f32; TAPS] {
    const CUT: f64 = 0.23;
    let m = (TAPS / 2) as f64;
    let mut taps = [0.0_f64; TAPS];
    for (i, tap) in taps.iter_mut().enumerate() {
        let x = i as f64 - m;
        let sinc = if x == 0.0 {
            2.0 * CUT
        } else {
            (TAU * CUT * x).sin() / (PI * x)
        };
        *tap = sinc * (0.42 + 0.5 * (PI * x / m).cos() + 0.08 * (TAU * x / m).cos());
    }
    let sum: f64 = taps.iter().sum();
    taps.map(|t| (t / sum) as f32)
}

/// A streaming stereo MP3 encoder.
pub struct Mp3Encoder {
    inner: rusty_mp3::Mp3Encoder,
    /// The stream's rate: a Layer III rate.
    rate: u32,
    /// Each channel's rate conversion, in order (none at a Layer III rate).
    stages: [Vec<Stage>; 2],
    taps: [f32; TAPS],
    /// The input's rate and the frames pushed at it.
    input_rate: u32,
    input: u64,
    /// The frames handed to rusty_mp3 (out of the rate conversion).
    encoded: usize,
}

impl Mp3Encoder {
    /// An encoder of audio at `rate` Hz: 320 kbps CBR with `high_quality`, else 192 kbps.
    ///
    /// # Panics
    /// On a zero `rate`.
    pub fn new(rate: u32, high_quality: bool) -> Self {
        assert!(rate > 0, "MP3 needs a sample rate");
        let (mut exact, mut halvings) = (f64::from(rate), 0);
        while exact >= 64_000.0 {
            (exact, halvings) = (exact / 2.0, halvings + 1);
        }
        let off = |r: u32| (f64::from(r) - exact).abs();
        let nearest = (RATES.into_iter())
            .min_by(|&a, &b| off(a).total_cmp(&off(b)))
            .expect("rates");
        let stages = [(); 2].map(|()| {
            let halve = (0..halvings).map(|_| Stage::Halve(vec![0.0; TAPS / 2]));
            let linear = (off(nearest) > 0.0).then(|| Stage::Linear {
                step: exact / f64::from(nearest),
                at: 0.0,
                input: Vec::new(),
            });
            halve.chain(linear).collect()
        });
        let config = rusty_mp3::Mp3EncoderConfig {
            bitrate_kbps: if high_quality { 320 } else { 192 },
            vbr_quality: None,
        };
        Self {
            inner: rusty_mp3::Mp3Encoder::new(config),
            rate: nearest,
            stages,
            taps: lowpass(),
            input_rate: rate,
            input: 0,
            encoded: 0,
        }
    }

    /// Appends one slice per channel: one (mono, duplicated) or two (left, right). The slices may be any length, but
    /// of equal length: if they differ, the shortest counts and the rest of the longer one is dropped.
    ///
    /// # Panics
    /// If there are not one or two slices.
    pub fn push<C: AsRef<[f32]>>(&mut self, channels: &[C]) {
        assert!(matches!(channels.len(), 1 | 2), "MP3 takes 1 or 2 channels");
        let len = channels.iter().map(|c| c.as_ref().len()).min().unwrap_or(0);
        self.input += len as u64;
        let [left, right] = [0, 1].map(|c| {
            let audio = &channels[c.min(channels.len() - 1)].as_ref()[..len];
            self.convert(c, audio.to_vec(), false)
        });
        self.encode(&left, &right);
    }

    /// The finished file: what the rate conversion holds back, the [`TAIL`] of silence after the input, then
    /// everything with the Info frame in front, tagged with the input's length at the stream rate.
    pub fn finish(mut self) -> Vec<u8> {
        let [left, right] = [0, 1].map(|c| self.convert(c, Vec::new(), true));
        self.encode(&left, &right);
        let scale = f64::from(self.rate) / f64::from(self.input_rate);
        let music = (self.input as f64 * scale).round() as usize;
        // The conversion's flush may already have added a few frames of silence after the input.
        let silence = vec![0.0; (music + TAIL).saturating_sub(self.encoded)];
        self.encode(&silence, &silence);
        self.inner.finish();
        let mut out = Vec::new();
        while let Ok(packet) = self.inner.next_packet() {
            out.extend(packet);
        }
        lame_tag(&mut out, music);
        out
    }

    /// Channel `c`'s `audio` through its rate conversion; `last` also lets out what the stages hold back.
    fn convert(&mut self, c: usize, mut audio: Vec<f32>, last: bool) -> Vec<f32> {
        for stage in &mut self.stages[c] {
            if last {
                audio.extend([0.0; TAPS / 2]);
            }
            audio = stage.run(&audio, &self.taps);
        }
        audio
    }

    fn encode(&mut self, left: &[f32], right: &[f32]) {
        self.encoded += left.len();
        let frames: Vec<f32> = (left.iter().zip(right))
            .flat_map(|(&l, &r)| [l, r])
            .collect();
        self.inner
            .push_pcm_f32(&frames, 2, self.rate)
            .expect("a Layer III rate");
    }
}

/// Makes the Info frame that opens `out` (rusty_mp3 writes it with frame and byte counts) gapless, as LAME writes it:
/// the frame count counts the audio frames only (the Xing convention), and the LAME extension after the counts holds
/// [`ENCODER`], the delay [`DELAY`] and the padding after the `music` frames, then the CRC of the frame up to there.
/// The extension's other fields stay zero (unknown).
fn lame_tag(out: &mut [u8], music: usize) {
    // After the header and stereo side info: 32 bytes in MPEG-1 frames of 1152 samples, 17 in MPEG-2/2.5 frames of 576.
    let (side_info, frame) = if (out[1] >> 3) & 3 == 3 {
        (32, 1152)
    } else {
        (17, 576)
    };
    let at = 4 + side_info;
    debug_assert_eq!(&out[at..at + 4], b"Info");
    let frames = u32::from_be_bytes(out[at + 8..at + 12].try_into().expect("4 bytes")) - 1;
    out[at + 8..at + 12].copy_from_slice(&frames.to_be_bytes());
    let lame = at + 16;
    out[lame..lame + 9].copy_from_slice(ENCODER);
    // 12 bits each. The padding (a decoder's 529 and the last frame's rest, `TAIL` having met the delay exactly) stays
    // under 529 + 1152.
    let padding = frames as usize * frame - DELAY - music;
    let trim = (DELAY << 12 | padding) as u32;
    out[lame + 21..lame + 24].copy_from_slice(&trim.to_be_bytes()[1..]);
    let crc = crc16_arc(&out[..lame + 34]);
    out[lame + 34..lame + 36].copy_from_slice(&crc.to_be_bytes());
}

/// CRC-16/ARC (x¹⁶+x¹⁵+x²+1 reflected, from 0): the LAME tag's.
fn crc16_arc(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0, |mut crc, &b| {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                crc >> 1 ^ 0xA001
            } else {
                crc >> 1
            };
        }
        crc
    })
}

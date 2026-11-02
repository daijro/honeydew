// Modified from rusty_mp3 1.0.0 (see Cargo.toml).
//! `rusty_mp3` — an in-house, pure-Rust MP3 (MPEG-1/2/2.5 Audio Layer III)
//! decoder **and** encoder. Zero dependencies, no C, no FFI.
//!
//! Why in-house: the robust Rust MP3 crate (Symphonia) is MPL-2.0, which trips
//! a no-copyleft license gate; the permissive one (puremp3) is incomplete, and
//! every existing Rust MP3 *encoder* option is an FFI binding to LAME. MP3's
//! patents expired in 2017 and Layer III is exhaustively documented, so we
//! built our own.
//!
//! ## Layout (the framework, built brick by brick)
//!
//! * shared: [`header`] (frame header), [`frame`] (side-info + types),
//!   [`bitio`] (MSB-first bit I/O), [`tables`] (ISO constant tables)
//! * [`decode`]: side-info → reservoir → Huffman → scalefactors → requantize →
//!   stereo → antialias → IMDCT → synthesis filterbank → PCM
//! * [`encode`]: analysis filterbank → MDCT → psychoacoustic model → two-loop
//!   quantizer → Huffman → bitstream
//!
//! ## Stream API
//!
//! [`Mp3Decoder`] and [`Mp3Encoder`] are the packet-level entry points:
//! push bytes/PCM in, pull frames/packets out. The pull calls follow FFmpeg's
//! EAGAIN/EOF drain protocol via [`Error::Again`] / [`Error::Eof`] — see
//! [`error`]. The frame-level engines ([`Mp3Decode`] / [`Mp3Encode`]) are also
//! public for callers that do their own framing.
#![allow(dead_code)] // A few stage helpers are wired for lab/diagnostic use only.

// Vendored (studio): modified from rusty_mp3 1.0.0. No `std::env` read outside the tests (the debug knobs that changed
// the encoder's output take their defaults), the `lab` module and the decoder's `MP3_CENSUS` drop hook are gone.

use std::collections::VecDeque;

pub mod bitio;
pub mod decode;
pub mod encode;
pub mod error;
pub mod frame;
pub mod header;
pub mod tables;

/// Prometheus telemetry hooks — samplers for the psychoacoustic model's
/// signal-independent curves (ATH, spreading, Bark), for offline formula
/// discovery by the private Prometheus refinery. Opt-in behind the
/// `prometheus-telemetry` feature; the production build is byte-identical
/// without it.
#[cfg(feature = "prometheus-telemetry")]
pub mod prometheus_telemetry {
    pub use crate::encode::psychoacoustic::prometheus::*;
}

pub use decode::Mp3Decode;
pub use encode::Mp3Encode;
pub use error::{Error, Result};
use header::FrameHeader;

/// One decoded chunk of PCM — the output of [`Mp3Decoder::next_frame`].
/// `samples` is interleaved f32 in `[-1, 1]`, `samples.len()` =
/// per-channel samples × `channels`.
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

/// Stream-level MP3 decoder: [`push`](Mp3Decoder::push) raw bytes (packets may
/// split/join frames — the decoder frame-syncs internally, skipping ID3 and
/// garbage), then drain decoded PCM with [`next_frame`](Mp3Decoder::next_frame).
#[derive(Default)]
pub struct Mp3Decoder {
    state: Mp3Decode,
    /// Accumulated bytes awaiting frame-sync (packets may split/join frames).
    buf: Vec<u8>,
    queue: VecDeque<DecodedAudio>,
    eof: bool,
}

impl Mp3Decoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed more compressed bytes; any whole frames they complete are decoded
    /// and queued for [`next_frame`](Mp3Decoder::next_frame).
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
        self.parse_frames();
    }

    /// Pull the next decoded frame. `Err(Again)` = feed more input;
    /// `Err(Eof)` = flushed and fully drained.
    ///
    /// # Errors
    /// [`Error::Again`] when more input is needed; [`Error::Eof`] once
    /// [`flush`](Self::flush) was called and every frame has been drained.
    pub fn next_frame(&mut self) -> Result<DecodedAudio> {
        if let Some(frame) = self.queue.pop_front() {
            return Ok(frame);
        }
        if self.eof {
            Err(Error::Eof)
        } else {
            Err(Error::Again)
        }
    }

    /// Signal end of input: once the queue drains, [`next_frame`](Mp3Decoder::next_frame)
    /// returns `Err(Eof)` instead of `Err(Again)`.
    pub fn flush(&mut self) {
        self.eof = true;
    }

    /// Frame-sync the buffer: for each complete frame, split header / side-info /
    /// main-data, decode it, and queue a [`DecodedAudio`]. Leaves a trailing partial
    /// frame in `buf` for the next push.
    fn parse_frames(&mut self) {
        let mut pos = 0;
        while pos + 4 <= self.buf.len() {
            // Sync = 11 set bits: 0xFF then top 3 bits of the next byte.
            if self.buf[pos] != 0xFF || self.buf[pos + 1] & 0xE0 != 0xE0 {
                pos += 1;
                continue;
            }
            let hb = [
                self.buf[pos],
                self.buf[pos + 1],
                self.buf[pos + 2],
                self.buf[pos + 3],
            ];
            let Ok(header) = FrameHeader::parse(hb) else {
                pos += 1;
                continue;
            };
            let frame_size = header.frame_size();
            if frame_size < 4 {
                pos += 1;
                continue;
            }
            if pos + frame_size > self.buf.len() {
                break; // incomplete frame — wait for more data
            }

            let crc = if header.crc_protected { 2 } else { 0 };
            let si_start = pos + 4 + crc;
            let si_len = header.side_info_len();
            let main_start = si_start + si_len;
            if main_start > pos + frame_size {
                pos += 1;
                continue;
            }
            // The decoder reads both parts straight out of the sync buffer (it
            // used to copy each into its own `Vec` first, two per frame).
            let side_info = &self.buf[si_start..main_start];
            let main_data = &self.buf[main_start..pos + frame_size];

            if let Ok(pcm) = self.state.decode_frame(&header, side_info, main_data) {
                let channels = header.channel_mode.channels().max(1);
                self.queue.push_back(DecodedAudio {
                    sample_rate: header.sample_rate,
                    channels: channels as u16,
                    samples: pcm,
                });
            }
            pos += frame_size;
        }
        self.buf.drain(0..pos);
    }
}

/// Decode a complete MP3 stream with a **two-stage pipeline** across two threads.
///
/// FFmpeg's MP3 decoder is single-threaded per stream, and MP3 resists the usual
/// frame-parallel trick because the bit reservoir makes each frame's main data
/// depend on its predecessors. But the decoder's STATE divides cleanly in two
/// and nothing is shared between the halves:
///
/// * **entropy** — frame sync, reservoir, Huffman, scalefactors, requantize,
///   joint stereo. Owns the reservoir. Serial across frames.
/// * **transform** — alias reduction, hybrid IMDCT, polyphase synthesis. Owns
///   the IMDCT overlap and the synthesis FIFO. Serial across granules.
///
/// Neither half can be parallelised internally, but they can overlap in TIME, so
/// wall-clock approaches `max(entropy, transform)` instead of their sum. Measured
/// on this decoder the two halves are ~50.7% and ~48.4% of the work — close to
/// the ideal split for a two-stage pipeline.
///
/// Output is identical to the serial path: the same code runs in the same order
/// on each half, only on different threads. `std::thread` only — this crate has
/// no dependencies and does not acquire one for this.
#[must_use]
pub fn decode_pipelined(bytes: &[u8]) -> Vec<DecodedAudio> {
    // No threads on `wasm32-unknown-unknown`: spawning traps, so a browser caller
    // reaching this function aborted the module. The two-stage split is a SPEED
    // optimisation whose output is identical to the serial path -- that equality is
    // a standing test (`pipelined_decode_matches_serial_exactly`) -- so where there
    // is no thread to give it, degrade instead of failing.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let mut dec = Mp3Decoder::new();
        dec.push(bytes);
        dec.flush();
        let mut out = Vec::new();
        while let Ok(f) = dec.next_frame() {
            out.push(f);
        }
        return out;
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        use std::sync::mpsc::sync_channel;

        // Bounded so a fast entropy stage cannot buffer the whole file's spectra
        // into memory ahead of the transform stage; this is the pipeline depth.
        let (tx, rx) = sync_channel::<(FrameHeader, Vec<decode::GranuleWork>)>(32);
        let mut out: Vec<DecodedAudio> = Vec::new();

        std::thread::scope(|scope| {
            scope.spawn(move || {
                let mut state = Mp3Decode::new();
                let mut pos = 0usize;
                while pos + 4 <= bytes.len() {
                    if bytes[pos] != 0xFF || bytes[pos + 1] & 0xE0 != 0xE0 {
                        pos += 1;
                        continue;
                    }
                    let hb = [bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]];
                    let Ok(header) = FrameHeader::parse(hb) else {
                        pos += 1;
                        continue;
                    };
                    let frame_size = header.frame_size();
                    if frame_size < 4 || pos + frame_size > bytes.len() {
                        if frame_size < 4 {
                            pos += 1;
                            continue;
                        }
                        break; // trailing partial frame
                    }
                    let crc = if header.crc_protected { 2 } else { 0 };
                    let si_start = pos + 4 + crc;
                    let main_start = si_start + header.side_info_len();
                    if main_start > pos + frame_size {
                        pos += 1;
                        continue;
                    }
                    let side = &bytes[si_start..main_start];
                    let main = &bytes[main_start..pos + frame_size];
                    if let Ok(work) = state.decode_frame_entropy(&header, side, main) {
                        if tx.send((header, work)).is_err() {
                            break; // receiver gone
                        }
                    }
                    pos += frame_size;
                }
            });

            // Transform stage runs on this thread, owning the overlap and the FIFO.
            let mut tstate = decode::TransformState::new();
            for (header, work) in rx {
                let channels = header.channel_mode.channels().max(1);
                let mut pcm =
                    Vec::with_capacity(work.len() * crate::frame::GRANULE_LINES * channels);
                for mut g in work {
                    tstate.granule_to_pcm(&g.side, &mut g.spectrum, channels, &mut pcm);
                }
                out.push(DecodedAudio {
                    sample_rate: header.sample_rate,
                    channels: channels as u16,
                    samples: pcm,
                });
            }
        });
        out
    }
}

/// The largest input magnitude the encoder accepts: 8x (+18 dB over) full scale.
/// Ordinary PCM, f32 overs included, is far inside it and passes through
/// untouched. Beyond it, input is clamped -- the rate loop was measured to fit
/// up to 4096x full scale and to fail by 65536x, which used to surface as a
/// panic in the reservoir assembly on hostile PCM (integer-valued "f32" samples,
/// random bit patterns).
pub const MAX_INPUT_AMPLITUDE: f32 = 8.0;

/// Admit one input sample: NaN becomes silence, everything else (infinities
/// included) is clamped to [`MAX_INPUT_AMPLITUDE`]. The identity on every
/// finite sample within range, so valid input encodes byte-for-byte as before.
#[inline]
fn sanitize_sample(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(-MAX_INPUT_AMPLITUDE, MAX_INPUT_AMPLITUDE)
    }
}

/// Configuration for [`Mp3Encoder`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Mp3EncoderConfig {
    /// CBR target in kbps, snapped to the nearest valid Layer III value for the
    /// MPEG version ([`snap_bitrate`]). `0` ⇒ default (128 for MPEG-1, 64 for
    /// MPEG-2/2.5).
    pub bitrate_kbps: u32,
    /// VBR **target average bitrate in kbps**. `Some` ⇒ VBR, `None` ⇒ CBR. To
    /// map an ffmpeg/LAME-style `-q:a` 0–9 quality index, use
    /// [`vbr_quality_index`].
    ///
    /// Was a peak-NMR target through 0.5.1; that path is gone (see
    /// [`vbr_quality_index`] for why).
    pub vbr_quality: Option<f32>,
}

/// Map an ffmpeg/LAME-style VBR quality index (`-q:a`, 0 = best … 9 = smallest)
/// to the peak-NMR target [`Mp3EncoderConfig::vbr_quality`] expects.
#[must_use]
pub fn vbr_quality_index(q: f32) -> f32 {
    // Returns a TARGET AVERAGE BITRATE in kbps, not a noise-to-mask ratio.
    //
    // Until 0.5.1 this returned an NMR target that drove a separate gain search.
    // That search was measured at 3.5 ODG behind LAME at matched bitrate, and
    // worse at 268 kbps than the CBR path was at 192 kbps -- the criterion was
    // dimensionless but unanchored, so it settled on a globally too-coarse
    // quantizer. Rate now drives quality through the SAME two-loop quantizer CBR
    // uses, which PEAQ puts within ~0.3 ODG of LAME.
    //
    // The ladder tracks LAME's own V0..V9 average rates, so `-q:a N` lands near
    // where users expect it to.
    const KBPS: [f32; 10] = [
        245.0, 225.0, 190.0, 175.0, 165.0, 130.0, 115.0, 100.0, 85.0, 65.0,
    ];
    let q = q.clamp(0.0, 9.0);
    let lo = q.floor() as usize;
    let hi = (lo + 1).min(9);
    let t = q - lo as f32;
    KBPS[lo] * (1.0 - t) + KBPS[hi] * t
}

/// Stream-level MP3 encoder: accumulates per-channel PCM, emits one MP3 frame per
/// 1152 samples per channel (576 for MPEG-2/2.5). MPEG-1/2/2.5, CBR or VBR,
/// psychoacoustic noise shaping; mono, stereo, or per-frame mid/side joint
/// stereo. Configured via [`Mp3EncoderConfig`].
///
/// Push PCM with [`push_pcm_f32`](Mp3Encoder::push_pcm_f32) /
/// [`push_pcm_s16`](Mp3Encoder::push_pcm_s16) (the first push fixes the header
/// from the sample rate + channel count), drain with
/// [`next_packet`](Mp3Encoder::next_packet), and call
/// [`finish`](Mp3Encoder::finish) at end of input (tail padding, reservoir
/// assembly, Xing/Info header).
#[derive(Default)]
pub struct Mp3Encoder {
    state: Mp3Encode,
    header: Option<FrameHeader>,
    /// Accumulated samples per channel awaiting a full frame.
    pcm: [Vec<f32>; 2],
    queue: VecDeque<Vec<u8>>,
    /// Audio frames emitted and their total byte length (for the Info header).
    total_frames: u32,
    total_bytes: usize,
    /// CBR target (kbps); 0 ⇒ default 128.
    cbr_kbps: u32,
    /// VBR quality target (peak NMR). `Some` ⇒ VBR, `None` ⇒ CBR.
    quality: Option<f32>,
    /// **3R1** — CBR reservoir-RD mode (buffer all frames, allocate bits across them by
    /// perceptual entropy, assemble with the bit reservoir at flush). Gated by
    /// `MP3_RESERVOIR`; `resv_gain` (`MP3_RESV_GAIN`, 0 ⇒ flat/byte-identical) is the knob.
    reservoir: bool,
    resv_gain: f32,
    /// **3R1 lookahead** (opt-in, `MP3_RESV_LOOKAHEAD=1`): buffer all frame PCM and allocate
    /// against the GLOBAL perceptual-entropy distribution (two-pass) instead of the default
    /// causal running average. Off by default — on representative-length content the causal
    /// EMA tracks the short-term (≤511 B) reservoir better; the buffer holds `(header, PCM)`.
    resv_lookahead: bool,
    resv_frames_pcm: Vec<(FrameHeader, Vec<Vec<f32>>)>,
    eof: bool,
}

/// Snap a requested bitrate (kbps) to the nearest valid Layer III value for the
/// MPEG version (the V1 and V2/2.5 bitrate tables differ).
#[must_use]
pub fn snap_bitrate(version: header::MpegVersion, kbps: u32) -> u32 {
    let v1 = [
        32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    let v2 = [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    let valid: &[u32] = if version == header::MpegVersion::V1 {
        &v1
    } else {
        &v2
    };
    *valid
        .iter()
        .min_by_key(|&&b| b.abs_diff(kbps))
        .unwrap_or(&128)
}

/// Build the frame header from the input's sample rate, channel count, and the
/// configured CBR bitrate (VBR overrides the bitrate per frame later).
///
/// # Errors
/// [`Error::Unsupported`] for a sample rate outside the nine MPEG Layer III rates.
pub fn encoder_header(sample_rate: u32, channels: u16, cbr_kbps: u32) -> Result<FrameHeader> {
    let version = match sample_rate {
        32000 | 44100 | 48000 => header::MpegVersion::V1,
        16000 | 22050 | 24000 => header::MpegVersion::V2,
        8000 | 11025 | 12000 => header::MpegVersion::V2_5,
        _ => {
            return Err(Error::unsupported(
                "mp3 encode: unsupported sample rate (need 8–48 kHz MPEG-1/2/2.5)",
            ))
        }
    };
    let channel_mode = if channels >= 2 {
        frame::ChannelMode::Stereo
    } else {
        frame::ChannelMode::Mono
    };
    // V2/2.5 default to a lower nominal bitrate to fit the smaller frame.
    let default_kbps = if version == header::MpegVersion::V1 {
        128
    } else {
        64
    };
    let bitrate_kbps = snap_bitrate(
        version,
        if cbr_kbps == 0 {
            default_kbps
        } else {
            cbr_kbps
        },
    );
    Ok(FrameHeader {
        version,
        crc_protected: false,
        bitrate_kbps,
        sample_rate,
        padding: false,
        channel_mode,
        copyright: false,
        original: true,
        emphasis: 0,
    })
}

impl Mp3Encoder {
    #[must_use]
    pub fn new(config: Mp3EncoderConfig) -> Self {
        Self {
            cbr_kbps: config.bitrate_kbps,
            quality: config.vbr_quality,
            ..Self::default()
        }
    }

    /// Push interleaved f32 PCM in `[-1, 1]`. The first push fixes the frame
    /// header (MPEG version from `sample_rate`, mono/stereo from `channels`);
    /// if a later push carries fewer channels than the header, the last input
    /// channel is replicated. `interleaved.len()` should be a multiple of
    /// `channels` (a trailing partial sample is ignored).
    ///
    /// # Errors
    /// [`Error::Unsupported`] when the FIRST push carries a sample rate outside
    /// the nine MPEG Layer III rates (it fixes the stream header).
    pub fn push_pcm_f32(
        &mut self,
        interleaved: &[f32],
        channels: u16,
        sample_rate: u32,
    ) -> Result<()> {
        let in_ch = (channels as usize).max(1);
        self.push_with(interleaved.len() / in_ch, channels, sample_rate, |i| {
            interleaved[i]
        })
    }

    /// Push interleaved signed-16-bit PCM. Converts `i16 / 32768.0` — the same
    /// convention the decoder uses — then follows
    /// [`push_pcm_f32`](Mp3Encoder::push_pcm_f32).
    ///
    /// # Errors
    /// [`Error::Unsupported`] when the FIRST push carries a sample rate outside
    /// the nine MPEG Layer III rates (it fixes the stream header).
    pub fn push_pcm_s16(
        &mut self,
        interleaved: &[i16],
        channels: u16,
        sample_rate: u32,
    ) -> Result<()> {
        let in_ch = (channels as usize).max(1);
        self.push_with(interleaved.len() / in_ch, channels, sample_rate, |i| {
            f32::from(interleaved[i]) / 32768.0
        })
    }

    /// Push interleaved **little-endian s16 bytes** — the layout a WAV `data`
    /// chunk or an FFmpeg-style `AudioFrame` plane already has. Same samples as
    /// [`push_pcm_s16`](Mp3Encoder::push_pcm_s16) on the decoded values, without
    /// the caller materialising an `i16` (and then an `f32`) copy of the input
    /// first. A trailing partial sample is ignored.
    ///
    /// # Errors
    /// [`Error::Unsupported`] when the FIRST push carries a sample rate outside
    /// the nine MPEG Layer III rates (it fixes the stream header).
    pub fn push_pcm_s16le(&mut self, bytes: &[u8], channels: u16, sample_rate: u32) -> Result<()> {
        let in_ch = (channels as usize).max(1);
        self.push_with(bytes.len() / 2 / in_ch, channels, sample_rate, |i| {
            f32::from(i16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]])) / 32768.0
        })
    }

    /// Push interleaved **little-endian f32 bytes**; the byte-level twin of
    /// [`push_pcm_f32`](Mp3Encoder::push_pcm_f32), as
    /// [`push_pcm_s16le`](Mp3Encoder::push_pcm_s16le) is of `push_pcm_s16`.
    ///
    /// # Errors
    /// [`Error::Unsupported`] when the FIRST push carries a sample rate outside
    /// the nine MPEG Layer III rates (it fixes the stream header).
    pub fn push_pcm_f32le(&mut self, bytes: &[u8], channels: u16, sample_rate: u32) -> Result<()> {
        let in_ch = (channels as usize).max(1);
        self.push_with(bytes.len() / 4 / in_ch, channels, sample_rate, |i| {
            let b = &bytes[4 * i..4 * i + 4];
            f32::from_le_bytes([b[0], b[1], b[2], b[3]])
        })
    }

    /// The one push path: `samples` per-channel sample instants, interleaved
    /// sample `i` read through `get`. Converts and deinterleaves STRAIGHT into
    /// the per-channel staging buffers, one frame at a time, encoding each frame
    /// as it fills — so the staging never holds more than one frame, whatever
    /// the caller hands over.
    ///
    /// It used to stage the whole push first. A demuxer that delivers a whole
    /// file as one frame (WAV does) therefore cost a file-sized `f32` copy per
    /// channel, grown by `push` with no reservation (each regrowth moving
    /// everything so far), and the s16 entry point added an `f32` copy of the
    /// whole input on top. Same samples, same order, same frames: byte-identical.
    fn push_with(
        &mut self,
        samples: usize,
        channels: u16,
        sample_rate: u32,
        get: impl Fn(usize) -> f32,
    ) -> Result<()> {
        if self.header.is_none() {
            self.header = Some(encoder_header(sample_rate, channels, self.cbr_kbps)?);
            // 3R1 reservoir RD — DEFAULT ON for CBR ≤ 256 kbps (2026-07-08). Measured
            // vs LAME on real music (guitar/piano, PEAQ): the bit reservoir closes the
            // whole quality gap — e.g. guitar@128k −0.59→+0.04 (LAME +0.06), piano@128k
            // −0.97→−0.13; +0.5–0.85 ODG across 96–256k, at parity with LAME. (It was
            // gated OFF and mis-judged "does nothing" by prom_qual001 — but that verdict
            // was measured on the SILENT encoder, before the s16-format fix.)
            // ⚠ 320 kbps EXCLUDED: the reservoir assembly produces valid-but-garbled
            // frames at the top bitrate (guitar@320k −0.85→−3.31) and the reservoir
            // barely helps there anyway (little cross-frame headroom to redistribute).
            // Known follow-up bug. `MP3_RESERVOIR=0` forces it off for A/B.
            // ⚠ MPEG-1 (V1) ONLY: the assembler hardcodes the 9-bit `main_data_begin`
            // (MAX_BEGIN=511); MPEG-2/2.5 use an 8-bit field (max 255) + 1 granule/frame,
            // so the reservoir would corrupt LSF streams. V2/2.5 keep the fixed path.
            // Cannot fail: the statement above this comment block assigned `Some`.
            let is_v1 = self.header.as_ref().unwrap().version == crate::header::MpegVersion::V1;
            // Vendored (studio): the `MP3_RESERVOIR` override is removed; the default.
            self.reservoir = self.quality.is_none() && is_v1 && self.cbr_kbps <= 256;
            // gain 0.2 = the swept corpus optimum (mean ODG +0.029 vs flat, no clip
            // regressed); MP3_RESV_GAIN overrides (0 ⇒ flat, for the neutrality check).
            // Vendored (studio): the `MP3_RESV_GAIN` override is removed; the default.
            self.resv_gain = 0.2;
            // CAUSAL by default; lookahead is opt-in (MP3_RESV_LOOKAHEAD=1). On a
            // representative 24 s corpus the causal lagging-EMA BEAT the global-mean
            // lookahead on the dynamic clip (vocal@128 −0.156 vs −0.205) — the 511 B
            // bank is a short-term smoother, so a LOCAL demand signal fits it better;
            // and causal is streaming-friendly (no full-file PCM buffer). See tune-quality.
            // Vendored (studio): the `MP3_RESV_LOOKAHEAD` override is removed; the default.
            self.resv_lookahead = false;
        }
        // Cannot fail: the block above either found `Some` or assigned it (an
        // unsupported rate returns `Err` from `encoder_header` before this).
        let header = self.header.as_ref().unwrap();
        let nch = header.channel_mode.channels();
        let spf = header.version.samples_per_frame();
        let in_ch = (channels as usize).max(1);
        let mut s = 0;
        while s < samples {
            if self.pcm[0].len() >= spf {
                self.drain_frames();
            }
            let n = (spf - self.pcm[0].len()).min(samples - s);
            // Deinterleave to per-channel f32; if the input has fewer channels
            // than the output, replicate the last one.
            for c in 0..nch {
                let ic = c.min(in_ch - 1);
                let dst = &mut self.pcm[c];
                dst.reserve(spf.saturating_sub(dst.len()));
                dst.extend((s..s + n).map(|k| sanitize_sample(get(k * in_ch + ic))));
            }
            s += n;
        }
        if self.pcm[0].len() >= spf {
            self.drain_frames();
        }
        Ok(())
    }

    /// Pull the next encoded MP3 packet (one frame, or the prepended Xing/Info
    /// frame after [`finish`](Mp3Encoder::finish)). `Err(Again)` = feed more
    /// PCM; `Err(Eof)` = finished and fully drained.
    ///
    /// # Errors
    /// [`Error::Again`] when more PCM is needed; [`Error::Eof`] once
    /// [`finish`](Self::finish) was called and every packet has been drained.
    pub fn next_packet(&mut self) -> Result<Vec<u8>> {
        if let Some(p) = self.queue.pop_front() {
            return Ok(p);
        }
        if self.eof {
            Err(Error::Eof)
        } else {
            Err(Error::Again)
        }
    }

    /// End of input: pad the PCM tail to a whole frame, assemble the bit
    /// reservoir stream (CBR V1), and prepend the Xing/Info header. After this,
    /// [`next_packet`](Mp3Encoder::next_packet) drains to `Err(Eof)`.
    pub fn finish(&mut self) {
        // Pad each channel's tail to a whole frame and encode it.
        if let Some(header) = self.header.clone() {
            let nch = header.channel_mode.channels();
            let spf = header.version.samples_per_frame();
            if (0..nch).any(|c| !self.pcm[c].is_empty()) {
                for c in 0..nch {
                    let padded = self.pcm[c].len().div_ceil(spf) * spf;
                    self.pcm[c].resize(padded, 0.0);
                }
                self.drain_frames();
            }
            // 3R1: all frames are banked — assemble the reservoir stream now and split
            // it back into fixed-size frame packets (B8 output is frame_size-aligned).
            if self.reservoir {
                if self.resv_lookahead {
                    let fsize = header.frame_size();
                    let frames = std::mem::take(&mut self.resv_frames_pcm);
                    let stream = self
                        .state
                        .encode_reservoir_lookahead(&frames, self.resv_gain);
                    for chunk in stream.chunks(fsize) {
                        self.queue.push_back(chunk.to_vec());
                    }
                } else {
                    // Assembled one packet per frame, so nothing is cut back out
                    // of a contiguous stream with a copy apiece.
                    self.queue.extend(self.state.finish_reservoir_frames());
                }
            }
            // Prepend the Xing/Info header now that the totals are known (counts
            // include the Info frame itself). Streaming consumers that drain before
            // finish won't get it first — that case wants two-pass.
            if self.total_frames > 0 {
                // The Info header's counters are 32-bit by format: saturate rather
                // than wrap on a > 4 GiB stream.
                let fsize = u32::try_from(header.frame_size()).unwrap_or(u32::MAX);
                let info = encode::bitstream::info_frame(
                    &header,
                    self.total_frames.saturating_add(1),
                    u32::try_from(self.total_bytes)
                        .unwrap_or(u32::MAX)
                        .saturating_add(fsize),
                    self.quality.is_some(),
                );
                self.queue.push_front(info);
            }
        }
        self.eof = true;
    }

    /// Emit a frame for each full 1152-sample-per-channel block accumulated.
    fn drain_frames(&mut self) {
        let Some(header) = self.header.clone() else {
            return;
        };
        let nch = header.channel_mode.channels();
        let spf = header.version.samples_per_frame();
        // Consume full frames via an advancing OFFSET, then remove the whole
        // consumed prefix ONCE at the end. Front-draining `spf` at a time shifts
        // the entire remaining tail every frame — O(n²) when a demuxer hands us
        // the whole file in one `send_frame` (the WAV path yields the `data`
        // chunk as a single packet). The offset walk is O(n). Byte-identical:
        // the same `[off..off+spf]` samples in the same order.
        let mut off = 0usize;
        while self.pcm[0].len() - off >= spf && (nch == 1 || self.pcm[1].len() - off >= spf) {
            // The frame is BORROWED from the staging buffers: the encoder only
            // reads it. (It used to be copied into a fresh `Vec<Vec<f32>>` per
            // frame -- three allocations and a frame of copying, every frame.)
            let block: [&[f32]; 2] = [
                &self.pcm[0][off..off + spf],
                &self.pcm[nch - 1][off..off + spf],
            ];
            let block = &block[..nch];
            if self.reservoir && self.resv_lookahead {
                // Lookahead: buffer PCM; analyse-all + allocate + assemble at
                // flush. This path must OWN the frame, so it alone copies.
                let owned = block.iter().map(|c| c.to_vec()).collect();
                self.resv_frames_pcm.push((header.clone(), owned));
                self.total_frames = self.total_frames.saturating_add(1);
                self.total_bytes += header.frame_size();
            } else if self.reservoir {
                // Causal: encode now, bank the raw frame; assemble (B8) at flush.
                self.state
                    .encode_frame_reservoir(&header, block, self.resv_gain);
                self.total_frames = self.total_frames.saturating_add(1);
                self.total_bytes += header.frame_size();
            } else if let Ok(bytes) = self.state.encode_frame(&header, block, self.quality) {
                self.total_frames = self.total_frames.saturating_add(1);
                self.total_bytes += bytes.len();
                self.queue.push_back(bytes);
            }
            off += spf;
        }
        // Drop all consumed samples in one shift (O(n) total, not O(n²)); the
        // sub-frame remainder stays at the front for the next call / flush pad.
        if off > 0 {
            for c in 0..nch {
                self.pcm[c].drain(0..off);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_mono(input: &[f32], sample_rate: u32) -> Vec<u8> {
        let mut enc = Mp3Encoder::default();
        enc.push_pcm_f32(input, 1, sample_rate).unwrap();
        enc.finish();
        let mut mp3 = Vec::new();
        while let Ok(p) = enc.next_packet() {
            mp3.extend_from_slice(&p);
        }
        mp3
    }

    /// Encode interleaved **S16** mono via the native s16 entry point.
    fn encode_mono_s16(input: &[i16], sample_rate: u32) -> Vec<u8> {
        let mut enc = Mp3Encoder::default();
        enc.push_pcm_s16(input, 1, sample_rate).unwrap();
        enc.finish();
        let mut mp3 = Vec::new();
        while let Ok(p) = enc.next_packet() {
            mp3.extend_from_slice(&p);
        }
        mp3
    }

    /// Hostile PCM -- NaN, infinities, integer-valued "f32" samples, random bit
    /// patterns -- must encode without panicking at every MPEG-1 rate and CBR
    /// bitrate (the reservoir path, where out-of-range input used to panic in the
    /// stream assembly), and in-range samples must be untouched by the input
    /// sanitizer.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn hostile_pcm_never_panics() {
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for kind in 0..4 {
            for &sr in &[32_000u32, 44_100, 48_000] {
                for &kbps in &[32u32, 128, 192] {
                    let pcm: Vec<f32> = (0..2 * 4 * 1152)
                        .map(|_| match kind {
                            0 => f32::from_bits(rnd() as u32),
                            1 => [f32::NAN, f32::INFINITY, f32::NEG_INFINITY][(rnd() % 3) as usize],
                            2 => (rnd() as i32) as f32,
                            _ => 65_536.0 * if rnd() & 1 == 0 { 1.0 } else { -1.0 },
                        })
                        .collect();
                    let mut enc = Mp3Encoder::new(Mp3EncoderConfig {
                        bitrate_kbps: kbps,
                        vbr_quality: None,
                    });
                    enc.push_pcm_f32(&pcm, 2, sr).unwrap();
                    enc.finish();
                    let mut bytes = 0;
                    while let Ok(p) = enc.next_packet() {
                        bytes += p.len();
                    }
                    assert!(bytes > 0, "kind {kind} @ {sr}/{kbps}k produced nothing");
                }
            }
        }
        for v in [0.0f32, -0.0, 1.0, -1.0, 0.5, 7.999, -8.0, f32::MIN_POSITIVE] {
            assert_eq!(
                sanitize_sample(v).to_bits(),
                v.to_bits(),
                "{v} must pass through"
            );
        }
        assert_eq!(sanitize_sample(f32::NAN), 0.0);
        assert_eq!(sanitize_sample(f32::INFINITY), MAX_INPUT_AMPLITUDE);
        assert_eq!(sanitize_sample(-1e9), -MAX_INPUT_AMPLITUDE);
    }

    /// Every push entry point, and every way of slicing the input across pushes,
    /// must produce the SAME bitstream: the byte-level `push_pcm_*le` twins equal
    /// their typed counterparts, and the frame-at-a-time staging (which encodes
    /// mid-push as each frame fills) equals one whole push. Chunk sizes are odd
    /// and straddle frame boundaries, including a chunk smaller than a sample
    /// frame's worth of channels. Stereo and an MPEG-2 rate (576-sample frames).
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn push_entry_points_and_chunkings_are_byte_identical() {
        fn drain(mut enc: Mp3Encoder) -> Vec<u8> {
            enc.finish();
            let mut mp3 = Vec::new();
            while let Ok(p) = enc.next_packet() {
                mp3.extend_from_slice(&p);
            }
            mp3
        }
        for &(sr, kbps) in &[(44_100u32, 192u32), (22_050, 64)] {
            let n = 7 * 1152 + 333; // per channel; not a whole number of frames
            let s16: Vec<i16> = (0..2 * n)
                .map(|i| {
                    let t = (i / 2) as f32 / sr as f32;
                    let f = if i % 2 == 0 { 440.0 } else { 660.0 };
                    ((2.0 * std::f32::consts::PI * f * t).sin() * 12000.0) as i16
                })
                .collect();
            let f32s: Vec<f32> = s16.iter().map(|&s| f32::from(s) / 32768.0).collect();
            let s16le: Vec<u8> = s16.iter().flat_map(|s| s.to_le_bytes()).collect();
            let f32le: Vec<u8> = f32s.iter().flat_map(|s| s.to_le_bytes()).collect();
            let cfg = || Mp3EncoderConfig {
                bitrate_kbps: kbps,
                vbr_quality: None,
            };

            let mut enc = Mp3Encoder::new(cfg());
            enc.push_pcm_s16(&s16, 2, sr).unwrap();
            let reference = drain(enc);
            assert!(reference.len() > 1000);

            let mut enc = Mp3Encoder::new(cfg());
            enc.push_pcm_s16le(&s16le, 2, sr).unwrap();
            assert_eq!(drain(enc), reference, "s16le whole @ {sr}");

            let mut enc = Mp3Encoder::new(cfg());
            enc.push_pcm_f32(&f32s, 2, sr).unwrap();
            assert_eq!(drain(enc), reference, "f32 whole @ {sr}");

            let mut enc = Mp3Encoder::new(cfg());
            enc.push_pcm_f32le(&f32le, 2, sr).unwrap();
            assert_eq!(drain(enc), reference, "f32le whole @ {sr}");

            for chunk in [1usize, 37, 1151, 1153, 2304, 5000] {
                let mut enc = Mp3Encoder::new(cfg());
                for c in s16.chunks(2 * chunk) {
                    enc.push_pcm_s16(c, 2, sr).unwrap();
                }
                assert_eq!(drain(enc), reference, "s16 chunks of {chunk} @ {sr}");
                let mut enc = Mp3Encoder::new(cfg());
                for c in f32le.chunks(8 * chunk) {
                    enc.push_pcm_f32le(c, 2, sr).unwrap();
                }
                assert_eq!(drain(enc), reference, "f32le chunks of {chunk} @ {sr}");
            }
        }
    }

    /// The native S16 path (`push_pcm_s16`) must produce audible output — the
    /// stream-level counterpart of the adapter's `s16_input_encodes_to_audible_output`
    /// regression (which guards the rff `af.format` byte-reinterpretation path).
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn s16_push_encodes_to_audible_output() {
        let sr = 44100u32;
        let n = sr as usize; // 1 s
        let s16: Vec<i16> = (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                (0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 32767.0) as i16
            })
            .collect();
        let out = decode_mono(&encode_mono_s16(&s16, sr));
        let rms = (out.iter().map(|x| x * x).sum::<f32>() / out.len().max(1) as f32).sqrt();
        assert!(
            rms > 0.1,
            "S16 input produced near-silent output (rms={rms})"
        );
    }

    /// Decode profiling driver (run explicitly): encode ~10 s of dense audio, then
    /// decode it while the per-stage profiler runs. `cargo test -p rusty_mp3
    /// --release profile_decode -- --ignored --nocapture`.
    #[test]
    #[ignore = "manual profiling run (prints the decode stage profile)"]
    fn profile_decode() {
        let sr = 44100u32;
        let n = 10 * sr as usize;
        let mut s = 0x9E37_79B9u32;
        let mut rng = || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        // Dense harmonic + noise → busy spectrum, realistic decode work per frame.
        let input: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                let mut v = 0f32;
                for k in 1..=24 {
                    v += (1.0 / k as f32)
                        * (2.0 * std::f32::consts::PI * 130.0 * k as f32 * t).sin();
                }
                (0.2 * v + 0.03 * rng()).clamp(-1.0, 1.0)
            })
            .collect();
        let mp3 = encode_mono(&input, sr);
        eprintln!("[profile] decoding {} KB of mp3 (~10 s)", mp3.len() / 1024);
        let out = decode_mono(&mp3);
        eprintln!("[profile] {} PCM samples out", out.len());
        crate::decode::prof::dump();
    }

    fn decode_mono(mp3: &[u8]) -> Vec<f32> {
        let mut dec = Mp3Decoder::default();
        dec.push(mp3);
        dec.flush();
        let mut out = Vec::new();
        while let Ok(af) = dec.next_frame() {
            out.extend_from_slice(&af.samples);
        }
        out
    }

    /// Best-aligned reconstruction SNR (dB) of `out` vs `reference`, searching the
    /// codec delay and skipping warm-up at both ends.
    fn best_snr(reference: &[f32], out: &[f32]) -> f64 {
        let (mut best, skip) = (f64::NEG_INFINITY, 2304usize);
        for delay in 0..3000 {
            let mut sig = 0f64;
            let mut err = 0f64;
            let mut n = 0;
            let mut i = skip;
            while i + delay < out.len() && i < reference.len() {
                let r = f64::from(reference[i]);
                sig += r * r;
                err += (r - f64::from(out[i + delay])).powi(2);
                n += 1;
                i += 1;
            }
            if n > 5000 && err > 0.0 {
                best = best.max(10.0 * (sig / err).log10());
            }
        }
        best
    }

    /// **R1 — stereo.** Two different tones in L and R survive independently.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn encode_decode_stereo() {
        let sr = 44100u32;
        let frames = 16;
        let n = frames * 1152;
        let pi2 = 2.0 * std::f32::consts::PI;
        // Interleaved L/R: L = 700 Hz, R = 3000 Hz.
        let mut interleaved = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f32 / sr as f32;
            interleaved.push(0.35 * (pi2 * 700.0 * t).sin()); // L
            interleaved.push(0.30 * (pi2 * 3000.0 * t).sin()); // R
        }

        let mut enc = Mp3Encoder::default();
        enc.push_pcm_f32(&interleaved, 2, sr).unwrap();
        enc.finish();
        let mut mp3 = Vec::new();
        while let Ok(p) = enc.next_packet() {
            mp3.extend_from_slice(&p);
        }
        assert!(!mp3.is_empty());
        if let Ok(path) = std::env::var("MP3_ENC_OUT") {
            std::fs::write(path, &mp3).expect("write MP3_ENC_OUT");
        }

        // Decode and split the interleaved stereo output back into L and R.
        let mut dec = Mp3Decoder::default();
        dec.push(&mp3);
        dec.flush();
        let (mut left, mut right) = (Vec::new(), Vec::new());
        while let Ok(af) = dec.next_frame() {
            assert_eq!(af.channels, 2, "decoded stream must be stereo");
            for fr in af.samples.chunks_exact(2) {
                left.push(fr[0]);
                right.push(fr[1]);
            }
        }

        let ref_l: Vec<f32> = (0..n)
            .map(|i| 0.35 * (pi2 * 700.0 * i as f32 / sr as f32).sin())
            .collect();
        let ref_r: Vec<f32> = (0..n)
            .map(|i| 0.30 * (pi2 * 3000.0 * i as f32 / sr as f32).sin())
            .collect();
        let snr_l = best_snr(&ref_l, &left);
        let snr_r = best_snr(&ref_r, &right);
        eprintln!("[R1] stereo SNR L {snr_l:.1} dB  R {snr_r:.1} dB");
        assert!(
            snr_l > 20.0 && snr_r > 20.0,
            "stereo channels too noisy: L {snr_l} R {snr_r}"
        );
    }

    /// **R5 — MPEG-2 / 2.5 (LSF).** Each low sample rate encodes as MPEG-2 (≥16 kHz)
    /// or MPEG-2.5 (<16 kHz) — 1 granule/frame, V2 bitrate + side-info — and
    /// round-trips through our decoder. FFmpeg validates the band tables out of band
    /// (`MP3_ENC_DIR`); all six rates decode to >69 dB there.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn encode_decode_mpeg2() {
        let dump_dir = std::env::var("MP3_ENC_DIR").ok();
        for &sr in &[22050u32, 24000, 16000, 12000, 11025, 8000] {
            let n = 24 * 576; // MPEG-2: 576 samples/frame
            let input: Vec<f32> = (0..n)
                .map(|i| 0.4 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sr as f32).sin())
                .collect();

            let mp3 = encode_mono(&input, sr);
            assert!(!mp3.is_empty());
            if let Some(dir) = &dump_dir {
                std::fs::write(format!("{dir}/v2_{sr}.mp3"), &mp3).expect("dump");
            }
            let h = header::FrameHeader::parse([mp3[0], mp3[1], mp3[2], mp3[3]]).unwrap();
            assert_ne!(
                h.version,
                header::MpegVersion::V1,
                "{sr}: must be MPEG-2/2.5"
            );
            assert_eq!(h.sample_rate, sr);

            let out = decode_mono(&mp3);
            assert!(out.len() > n / 2);
            let snr = best_snr(&input, &out);
            eprintln!("[R5] MPEG-2 {sr} Hz round-trip SNR {snr:.1} dB");
            assert!(
                snr > 30.0,
                "{sr}: MPEG-2 round-trip SNR too low: {snr:.1} dB"
            );
        }
    }

    /// **Q5 — block switching.** A castanet-like transient (silence then a sharp
    /// burst, repeating) drives the encoder into short blocks; the stream must
    /// carry window-switched frames and still reconstruct.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn block_switching_on_transients() {
        let sr = 44100u32;
        let frames = 20;
        let n = frames * 1152;
        let mut s = 0xBEEF_1234u32;
        // Repeating impulse-train: a sharp burst every ~1500 samples on silence.
        let input: Vec<f32> = (0..n)
            .map(|i| {
                if i % 1500 < 80 {
                    s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    0.7 * ((s >> 8) as f32 / (1u32 << 24) as f32 - 0.5)
                } else {
                    0.0
                }
            })
            .collect();

        let mp3 = encode_mono(&input, sr);
        assert!(!mp3.is_empty());
        if let Ok(path) = std::env::var("MP3_ENC_OUT") {
            std::fs::write(path, &mp3).expect("write MP3_ENC_OUT");
        }

        // Count window-switched frames: in the side info, the first granule's
        // `window_switching` bit. For MPEG-1 mono it's bit 8 of the per-granule
        // fields — simplest to detect by decoding and checking the stream carries
        // short blocks via the side-info parser.
        let si_len = header::FrameHeader::parse([mp3[0], mp3[1], mp3[2], mp3[3]])
            .map_or(17, |h| h.side_info_len());
        let mut switched = 0;
        let mut pos = 0;
        while pos + 4 <= mp3.len() {
            if mp3[pos] == 0xFF && mp3[pos + 1] & 0xE0 == 0xE0 {
                if let Ok(h) =
                    header::FrameHeader::parse([mp3[pos], mp3[pos + 1], mp3[pos + 2], mp3[pos + 3]])
                {
                    let si = &mp3[pos + 4..pos + 4 + si_len];
                    if let Ok(parsed) = decode::sideinfo::parse(&h, si) {
                        if (0..h.version.granules())
                            .any(|gr| parsed.granules[gr][0].window_switching)
                        {
                            switched += 1;
                        }
                    }
                    pos += h.frame_size();
                    continue;
                }
            }
            pos += 1;
        }
        eprintln!("[Q5] window-switched frames: {switched}/{frames}");
        assert!(
            switched >= 3,
            "transients must trigger block switching, got {switched}"
        );

        // And the switched stream still decodes frame-for-frame.
        let mut dec = Mp3Decoder::default();
        dec.push(&mp3);
        dec.flush();
        let mut decoded = 0;
        while dec.next_frame().is_ok() {
            decoded += 1;
        }
        assert!(decoded >= frames, "all frames must decode, got {decoded}");
    }

    /// **R4 — conformance corpus.** A broad set of signals each round-trips through
    /// encode → our decoder above a per-signal floor. With `MP3_ENC_DIR` set, the
    /// `.mp3`s are dumped for the out-of-band FFmpeg/LAME cross-check.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn conformance_corpus_round_trips() {
        let sr = 44100u32;
        let frames = 14;
        let n = frames * 1152;
        let pi2 = std::f32::consts::TAU;
        let tone = |f: f32| -> Vec<f32> {
            (0..n)
                .map(|i| 0.4 * (pi2 * f * i as f32 / sr as f32).sin())
                .collect()
        };
        let noise = |seed: u32, amp: f32| -> Vec<f32> {
            let mut s = seed;
            (0..n)
                .map(|_| {
                    s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    amp * ((s >> 8) as f32 / (1u32 << 24) as f32 - 0.5)
                })
                .collect()
        };
        let sweep: Vec<f32> = {
            let mut ph = 0f32;
            (0..n)
                .map(|i| {
                    let f = 80.0 + (16000.0 - 80.0) * (i as f32 / n as f32);
                    ph += pi2 * f / sr as f32;
                    0.4 * ph.sin()
                })
                .collect()
        };
        // (name, mono signal, SNR floor dB) — tones reconstruct cleanly, noise
        // is lossy at 128k so the floor is lower.
        let corpus: Vec<(&str, Vec<f32>, f64)> = vec![
            ("tone-250", tone(250.0), 40.0),
            ("tone-1k", tone(1000.0), 50.0),
            ("tone-4k", tone(4000.0), 45.0),
            ("tone-12k", tone(12000.0), 30.0),
            ("sweep", sweep, 15.0),
            ("noise", noise(0xC0FFEE, 0.4), 5.0),
            ("quiet-noise", noise(0x1234, 0.02), 3.0),
            (
                "two-tone",
                {
                    let a = tone(600.0);
                    let b = tone(5200.0);
                    a.iter().zip(&b).map(|(x, y)| 0.5 * (x + y)).collect()
                },
                25.0,
            ),
        ];

        let dump_dir = std::env::var("MP3_ENC_DIR").ok();
        for (name, sig, floor) in &corpus {
            let mp3 = encode_mono(sig, sr);
            assert!(!mp3.is_empty(), "{name}: no output");
            if let Some(dir) = &dump_dir {
                std::fs::write(format!("{dir}/{name}.mp3"), &mp3).expect("dump");
            }
            let out = decode_mono(&mp3);
            assert!(out.len() > n / 2, "{name}: too few samples");
            let snr = best_snr(sig, &out);
            eprintln!("[R4] {name:<12} SNR {snr:.1} dB (floor {floor})");
            assert!(snr >= *floor, "{name}: SNR {snr:.1} below floor {floor}");
        }
    }

    /// **R2 — VBR.** Quiet-then-loud content makes the per-frame bitrate vary, and
    /// the stream still decodes (in our decoder and FFmpeg).
    /// **VBR gate.** Three separate regressions, each of which shipped:
    ///
    /// 1. `-q:a` was INERT — every setting produced ~39 kbps, because the
    ///    masking thresholds (FFT power domain) were compared directly against
    ///    quantization noise (MDCT domain), scales ~10^4 apart. The search
    ///    saturated at the coarsest gain for 97.5% of granules.
    /// 2. The whole ladder targeted NMR >= 1.0 — noise AT or ABOVE the mask even
    ///    at the best setting.
    /// 3. Once (1) and (2) were fixed, quality demanded more bits than the
    ///    largest legal frame holds, and the overflow corrupted the reservoir
    ///    back-pointer: FFmpeg rejected the stream with "invalid new backstep".
    ///
    /// The old round-trip test could not catch (3) at all — it decoded with OUR
    /// decoder, which tolerates the bad back-pointer. So this asserts the
    /// PROPERTY that was violated (main data fits the frame it is written into)
    /// rather than trusting a self-round-trip.
    /// **Pipeline gate.** The two-stage threaded path must produce EXACTLY the
    /// serial path's samples — same count, same bits. It can, because the split
    /// is along a state boundary: neither half touches the other's state, so the
    /// same code runs in the same order, just on two threads.
    ///
    /// A threading change that merely produced "close" audio would be a silent
    /// corruption, so this compares bit patterns, not a tolerance.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn pipelined_decode_matches_serial_exactly() {
        // Encode a couple of seconds of real-ish content so the stream spans many
        // frames and exercises the reservoir across frame boundaries.
        let sr = 44_100u32;
        let n = 90 * 1152;
        let mut lcg: u32 = 0xA5A5_1234;
        let pcm: Vec<f32> = (0..n * 2)
            .map(|i| {
                lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let t = (i / 2) as f32 / sr as f32;
                let tone = (2.0 * std::f32::consts::PI * 220.0 * t).sin()
                    + 0.5 * (2.0 * std::f32::consts::PI * 1310.0 * t).sin();
                (0.3 * tone + 0.03 * ((lcg >> 9) as f32 / (1 << 23) as f32 - 0.5)).clamp(-1.0, 1.0)
            })
            .collect();
        let mut enc = Mp3Encoder::new(Mp3EncoderConfig {
            bitrate_kbps: 192,
            vbr_quality: None,
        });
        enc.push_pcm_f32(&pcm, 2, sr).unwrap();
        enc.finish();
        let mut mp3 = Vec::new();
        while let Ok(p) = enc.next_packet() {
            mp3.extend_from_slice(&p);
        }
        assert!(mp3.len() > 10_000, "test stream too short to be meaningful");

        let mut dec = Mp3Decoder::new();
        dec.push(&mp3);
        dec.flush();
        let mut serial: Vec<f32> = Vec::new();
        while let Ok(f) = dec.next_frame() {
            serial.extend_from_slice(&f.samples);
        }

        let piped: Vec<f32> = decode_pipelined(&mp3)
            .into_iter()
            .flat_map(|f| f.samples)
            .collect();

        assert_eq!(
            serial.len(),
            piped.len(),
            "pipelined produced a different sample count"
        );
        assert!(!serial.is_empty(), "decoded nothing");
        for (i, (a, b)) in serial.iter().zip(piped.iter()).enumerate() {
            assert_eq!(a.to_bits(), b.to_bits(), "pipelined diverged at sample {i}");
        }
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn vbr_ladder_is_live_conformant_and_ordered() {
        let sr = 44_100u32;
        let n = 8 * 1152;
        let mut lcg: u32 = 0x5EED_1234;
        let pcm: Vec<f32> = (0..n)
            .map(|i| {
                lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let t = i as f32 / sr as f32;
                let tone = (0..6)
                    .map(|k| {
                        (1.0 / (k + 1) as f32)
                            * (2.0 * std::f32::consts::PI * 220.0 * (k + 1) as f32 * t).sin()
                    })
                    .sum::<f32>();
                (0.25 * tone + 0.02 * ((lcg >> 9) as f32 / (1 << 23) as f32 - 0.5)).clamp(-1.0, 1.0)
            })
            .collect();

        let mut sizes = Vec::new();
        for q in [0.0f32, 3.0, 6.0, 9.0] {
            let mut enc = Mp3Encoder::new(Mp3EncoderConfig {
                bitrate_kbps: 0,
                vbr_quality: Some(vbr_quality_index(q)),
            });
            enc.push_pcm_f32(&pcm, 1, sr).unwrap();
            enc.finish();
            let mut mp3 = Vec::new();
            while let Ok(p) = enc.next_packet() {
                // CONFORMANCE: every emitted frame must physically contain its
                // own main data. A frame that overflows is what corrupted
                // `main_data_begin` and made FFmpeg reject the stream.
                assert!(!p.is_empty(), "q={q}: empty packet");
                mp3.extend_from_slice(&p);
            }
            assert!(mp3.len() > 1000, "q={q}: implausibly small output");
            sizes.push(mp3.len());
        }

        // LIVE: the knob must actually move the rate. The inert-knob bug had all
        // four within 1% of each other.
        let (best, worst) = (sizes[0], sizes[sizes.len() - 1]);
        assert!(
            best as f64 / worst as f64 > 1.5,
            "-q:a is inert: sizes {sizes:?} span less than 1.5x"
        );
        // ORDERED: higher q (lower quality) must never cost MORE bits.
        for w in sizes.windows(2) {
            assert!(w[1] <= w[0], "VBR ladder not monotonic: {sizes:?}");
        }
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn vbr_varies_bitrate_and_round_trips() {
        let sr = 44100u32;
        let pi2 = 2.0 * std::f32::consts::PI;
        // 10 quiet simple frames, then 10 loud broadband-noise frames.
        let mut input = Vec::new();
        let mut s = 0x1234_5678u32;
        for f in 0..20 {
            for i in 0..1152 {
                let t = (f * 1152 + i) as f32 / sr as f32;
                if f < 10 {
                    input.push(0.05 * (pi2 * 500.0 * t).sin()); // quiet tone → few bits
                } else {
                    s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    input.push(0.6 * ((s >> 8) as f32 / (1u32 << 24) as f32 - 0.5));
                    // loud noise
                }
            }
        }

        // Quality index 3 (ffmpeg `-q:a 3`) → VBR via the peak-NMR mapping.
        let mut enc = Mp3Encoder::new(Mp3EncoderConfig {
            bitrate_kbps: 0,
            vbr_quality: Some(vbr_quality_index(3.0)),
        });
        enc.push_pcm_f32(&input, 1, sr).unwrap();
        enc.finish();
        let mut mp3 = Vec::new();
        while let Ok(p) = enc.next_packet() {
            mp3.extend_from_slice(&p);
        }
        if let Ok(path) = std::env::var("MP3_ENC_OUT") {
            std::fs::write(path, &mp3).expect("write MP3_ENC_OUT");
        }

        // Walk the frames and collect their coded bitrates — VBR must use ≥2.
        let mut bitrates = std::collections::BTreeSet::new();
        let mut pos = 0;
        while pos + 4 <= mp3.len() {
            if mp3[pos] == 0xFF && mp3[pos + 1] & 0xE0 == 0xE0 {
                if let Ok(h) =
                    header::FrameHeader::parse([mp3[pos], mp3[pos + 1], mp3[pos + 2], mp3[pos + 3]])
                {
                    bitrates.insert(h.bitrate_kbps);
                    pos += h.frame_size();
                    continue;
                }
            }
            pos += 1;
        }
        eprintln!("[R2] VBR bitrates used: {bitrates:?}");
        assert!(
            bitrates.len() >= 2,
            "VBR must vary the bitrate, got {bitrates:?}"
        );

        // And it still decodes frame-for-frame.
        let mut dec = Mp3Decoder::default();
        dec.push(&mp3);
        dec.flush();
        let mut frames = 0;
        while dec.next_frame().is_ok() {
            frames += 1;
        }
        assert!(frames >= 20, "expected all frames to decode, got {frames}");
    }

    /// **R1+ — mid/side joint stereo.** Correlated channels (L ≈ R, slightly
    /// panned) trigger M/S; the stream must still reconstruct L and R.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn encode_decode_joint_stereo() {
        let sr = 44100u32;
        let n = 16 * 1152;
        let pi2 = 2.0 * std::f32::consts::PI;
        // Near-mono content with a small inter-channel difference → M/S wins.
        let mut interleaved = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f32 / sr as f32;
            let base = 0.4 * (pi2 * 900.0 * t).sin();
            interleaved.push(base); // L
            interleaved.push(base * 0.95 + 0.02 * (pi2 * 1500.0 * t).sin()); // R ≈ L
        }

        let mut enc = Mp3Encoder::default();
        enc.push_pcm_f32(&interleaved, 2, sr).unwrap();
        enc.finish();
        let mut mp3 = Vec::new();
        while let Ok(p) = enc.next_packet() {
            mp3.extend_from_slice(&p);
        }
        if let Ok(path) = std::env::var("MP3_ENC_OUT") {
            std::fs::write(path, &mp3).expect("write MP3_ENC_OUT");
        }

        // At least one frame must have chosen joint stereo (header mode 0b01).
        let joint = mp3
            .windows(2)
            .step_by(1)
            .any(|w| w[0] == 0xFF && (w[1] & 0xE0) == 0xE0 && (w[1] & 0x06) == 0x02);
        assert!(joint, "no joint-stereo frame emitted");

        let mut dec = Mp3Decoder::default();
        dec.push(&mp3);
        dec.flush();
        let (mut left, mut right) = (Vec::new(), Vec::new());
        while let Ok(af) = dec.next_frame() {
            for fr in af.samples.chunks_exact(2) {
                left.push(fr[0]);
                right.push(fr[1]);
            }
        }
        let ref_l: Vec<f32> = (0..n)
            .map(|i| 0.4 * (pi2 * 900.0 * i as f32 / sr as f32).sin())
            .collect();
        let ref_r: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                0.4 * (pi2 * 900.0 * t).sin() * 0.95 + 0.02 * (pi2 * 1500.0 * t).sin()
            })
            .collect();
        let snr_l = best_snr(&ref_l, &left);
        let snr_r = best_snr(&ref_r, &right);
        eprintln!("[R1+] joint-stereo SNR L {snr_l:.1} dB  R {snr_r:.1} dB");
        assert!(
            snr_l > 20.0 && snr_r > 20.0,
            "joint stereo too noisy: L {snr_l} R {snr_r}"
        );
    }

    /// **C4 — the pipeline gate.** A multi-tone signal (which exercises Q6's
    /// non-flat scalefactors) round-trips PCM → encoder → decoder → PCM well above
    /// the noise floor, and the `.mp3` decodes in FFmpeg (checked out-of-band; see
    /// docs/mp3-encoder-plan.md).
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn encode_decode_pipeline_multitone() {
        let sr = 44100u32;
        let n = 16 * 1152;
        let input: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                let pi2 = 2.0 * std::f32::consts::PI;
                0.3 * (pi2 * 600.0 * t).sin()
                    + 0.2 * (pi2 * 2300.0 * t).sin()
                    + 0.12 * (pi2 * 9000.0 * t).sin()
            })
            .collect();

        let mp3 = encode_mono(&input, sr);
        assert!(!mp3.is_empty(), "encoder produced no data");
        assert_eq!(mp3[0], 0xFF);
        assert_eq!(mp3[1] & 0xE0, 0xE0);
        if let Ok(path) = std::env::var("MP3_ENC_OUT") {
            std::fs::write(path, &mp3).expect("write MP3_ENC_OUT");
        }

        let out = decode_mono(&mp3);
        assert!(out.len() > n / 2, "decoder produced too few samples");

        let snr = best_snr(&input, &out);
        eprintln!("[C4] encode→decode multitone SNR {snr:.1} dB");
        assert!(snr > 20.0, "round-trip SNR too low: {snr:.1} dB");
    }

    /// Decode a real MP3 file (path in `MP3_REF`) and report structure. Validates
    /// frame-sync (skips ID3), header/side-info parsing, and main-data extraction
    /// on real data. Output is silent until D[]/codebooks are laid; this checks
    /// the *structure* (frame count, sample count, no panics, finite samples).
    #[test]
    fn decode_real_mp3_structure() {
        let Ok(path) = std::env::var("MP3_REF") else {
            return; // self-skip when not running the reference harness
        };
        let data = std::fs::read(&path).expect("read MP3_REF");
        let mut dec = Mp3Decoder::default();
        dec.push(&data);
        dec.flush();

        let mut frames = 0usize;
        let mut samples = 0usize;
        let mut pcm: Vec<u8> = Vec::new();
        while let Ok(af) = dec.next_frame() {
            assert_eq!(af.sample_rate, 44100);
            assert_eq!(af.channels, 1);
            for s in &af.samples {
                pcm.extend_from_slice(&s.to_le_bytes());
            }
            frames += 1;
            samples += af.samples.len() / af.channels as usize;
        }
        eprintln!("[MP3] decoded frames={frames} samples={samples}");
        assert!(frames > 0, "must decode at least one frame from real data");
        if let Ok(out) = std::env::var("MP3_OUT") {
            std::fs::write(out, &pcm).expect("write MP3_OUT");
        }
    }

    /// Encode a WAV file (`WAV_IN`, f32le or s16le mono) → mp3 at `ENC_OUT`, at `BR` kbps.
    /// Honors `MP3_RESERVOIR`/`MP3_RESV_GAIN`. Lets the quality gate run without the CLI.
    #[test]
    fn encode_wav_env() {
        let Ok(inp) = std::env::var("WAV_IN") else {
            return;
        };
        let d = std::fs::read(&inp).expect("read WAV_IN");
        // minimal WAV parse: fmt (rate, bits, format) + data chunk
        let fmt = d.windows(4).position(|w| w == b"fmt ").unwrap() + 8;
        let audio_fmt = u16::from_le_bytes([d[fmt], d[fmt + 1]]);
        let rate = u32::from_le_bytes([d[fmt + 4], d[fmt + 5], d[fmt + 6], d[fmt + 7]]);
        let bits = u16::from_le_bytes([d[fmt + 14], d[fmt + 15]]);
        let data = d.windows(4).position(|w| w == b"data").unwrap() + 8;
        let pcm: Vec<f32> = if audio_fmt == 3 || bits == 32 {
            d[data..]
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        } else {
            d[data..]
                .chunks_exact(2)
                .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0)
                .collect()
        };
        // Channel count from the fmt chunk (offset +2..+4); pcm is interleaved as read.
        let nch = u16::from_le_bytes([d[fmt + 2], d[fmt + 3]]).max(1);
        let mut enc = Mp3Encoder::new(Mp3EncoderConfig {
            bitrate_kbps: std::env::var("BR")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(128),
            vbr_quality: None,
        });
        enc.push_pcm_f32(&pcm, nch, rate).unwrap();
        enc.finish();
        let mut mp3 = Vec::new();
        while let Ok(p) = enc.next_packet() {
            mp3.extend_from_slice(&p);
        }
        std::fs::write(std::env::var("ENC_OUT").unwrap(), mp3).unwrap();
    }

    /// Encode a synthetic tone+noise signal at `ENC_RATE` Hz → mp3 at `ENC_OUT`.
    /// Used to check our V1/V2/V2.5 encoder output is valid (decodable by a neutral ref).
    #[test]
    fn encode_at_rate_to_file() {
        let Some(rate) = std::env::var("ENC_RATE")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            return;
        };
        let n = rate as usize * 2;
        let mut pcm = vec![0f32; n];
        let mut s = 0x0001_2345u32;
        for (i, v) in pcm.iter_mut().enumerate() {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
            let t = i as f32 / rate as f32;
            *v = 0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin() + 0.05 * noise;
        }
        let mp3 = encode_mono(&pcm, rate);
        std::fs::write(std::env::var("ENC_OUT").unwrap(), mp3).unwrap();
    }

    /// Rate-agnostic decode of `MP3_REF2` → f32le PCM at `MP3_OUT2` (any rate/channels).
    /// Used to verify LSF/MPEG-2.5 decode via the full pipeline without the CLI.
    #[test]
    fn decode_any_mp3_to_pcm() {
        let Ok(path) = std::env::var("MP3_REF2") else {
            return;
        };
        let data = std::fs::read(&path).expect("read MP3_REF2");
        let mut dec = Mp3Decoder::default();
        dec.push(&data);
        dec.flush();
        let mut pcm: Vec<u8> = Vec::new();
        while let Ok(af) = dec.next_frame() {
            for s in &af.samples {
                pcm.extend_from_slice(&s.to_le_bytes());
            }
        }
        assert!(!pcm.is_empty());
        if let Ok(out) = std::env::var("MP3_OUT2") {
            std::fs::write(out, &pcm).expect("write MP3_OUT2");
        }
    }

    #[test]
    fn header_parse_roundtrip() {
        // MPEG-1 Layer III, 128 kbps, 44.1 kHz, stereo, no CRC, no padding.
        let bytes = [0xFF, 0xFB, 0x90, 0x00];
        let h = header::FrameHeader::parse(bytes).unwrap();
        assert_eq!(h.version, header::MpegVersion::V1);
        assert_eq!(h.bitrate_kbps, 128);
        assert_eq!(h.sample_rate, 44100);
        assert_eq!(h.channel_mode, frame::ChannelMode::Stereo);
        assert!(!h.crc_protected && !h.padding);
        assert_eq!(h.frame_size(), 417);
        assert_eq!(h.to_bytes(), bytes, "header must round-trip bit-exactly");
    }

    #[test]
    fn header_rejects_non_layer3_and_bad_sync() {
        // Layer II (0b10 in the layer field): byte1 = 1111_1101.
        assert!(header::FrameHeader::parse([0xFF, 0xFD, 0x90, 0x00]).is_err());
        // Broken sync.
        assert!(header::FrameHeader::parse([0x00, 0x00, 0x00, 0x00]).is_err());
    }

    fn hdr(version: header::MpegVersion, mode: frame::ChannelMode) -> header::FrameHeader {
        header::FrameHeader {
            version,
            crc_protected: false,
            bitrate_kbps: 128,
            sample_rate: 44100,
            padding: false,
            channel_mode: mode,
            copyright: false,
            original: true,
            emphasis: 0,
        }
    }

    #[test]
    fn sideinfo_bit_accounting_all_layouts() {
        use frame::ChannelMode::{Mono, Stereo};
        use header::MpegVersion::{V1, V2};
        // (version, channel mode, expected side-info length in bytes)
        for (v, m, len) in [
            (V1, Stereo, 32),
            (V1, Mono, 17),
            (V2, Stereo, 17),
            (V2, Mono, 9),
        ] {
            let h = hdr(v, m);
            assert_eq!(h.side_info_len(), len);
            // An all-zero block parses cleanly; parse()'s debug_assert verifies
            // the field widths sum to exactly len*8 bits for this layout.
            let si = decode::sideinfo::parse(&h, &vec![0u8; len]).unwrap();
            assert_eq!(si.main_data_begin, 0);
        }
    }

    #[test]
    fn frame_size_matches_spec_example() {
        // MPEG-1 L3, 128 kbps, 44100 Hz, no padding → 417 bytes.
        let h = header::FrameHeader {
            version: header::MpegVersion::V1,
            crc_protected: false,
            bitrate_kbps: 128,
            sample_rate: 44100,
            padding: false,
            channel_mode: frame::ChannelMode::Stereo,
            copyright: false,
            original: true,
            emphasis: 0,
        };
        assert_eq!(h.frame_size(), 417);
        assert_eq!(h.side_info_len(), 32);
    }
}

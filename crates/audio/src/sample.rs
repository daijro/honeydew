//! Sample decoding (bytes in, samples out) and the one interpolated reader every sample player uses.
//!
//! `decode` runs on the app thread. It sniffs the magic and never trusts a file name: 3612 of the bundled `.wav`
//! assets are FLAC streams. Sniffing loads the same files as picking a reader by extension with a WAV → MP3 → FLAC →
//! OGG fallback would, given that each reader accepts only its own magic, after leading ID3v2 tags: `RIFF…WAVE`, then
//! `fLaC`, then `OggS` (Vorbis or FLAC inside), and MP3, which
//! has no reliable magic, takes everything else. A RIFF the WAV reader rejects goes on to MP3 (MP3 in WAV). WAV chunks,
//! FLAC's `FLSM` metadata block and Ogg pages (RFC 3533) are parsed by hand; FLAC, MP3 and Vorbis
//! packets go to symphonia. Samples stay f32 at the native rate, never normalised. Input is hostile: every length is
//! checked, symphonia never sees a metadata block or a setup header that would make it allocate from an unchecked
//! count, output is capped at a frame limit, and a panic in symphonia becomes an error where unwinding exists. `read`
//! and `step` are real-time safe.

use std::sync::Arc;

use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::formats::{FormatOptions, FormatReader, Packet};
use symphonia::core::io::MediaSourceStream;

/// Most frames per channel that `decode` accepts: 2^25, about 12.7 minutes at 44.1 kHz, 256 MiB as stereo `f32`.
// Bounds memory on hostile or huge files, on wasm too.
pub const MAX_FRAMES: usize = 1 << 25;

/// Decoded audio. `right` is `None` for mono; when present it has the same length as `left`. A file with more than
/// two channels keeps the first two.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SampleData {
    pub sample_rate: u32,
    pub left: Arc<[f32]>,
    pub right: Option<Arc<[f32]>>,
    pub meta: SampleMeta,
}

/// A file's tempo, loop and slice markers, values as stored: from a WAV's `smpl`, `acid` and `cue ` chunks, or
/// from a FLAC's `FLSM` block (`fLmL` and the `cue ` inside `fLsF`; plain, ID3-prefixed and Ogg FLAC alike). Other
/// formats carry none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SampleMeta {
    /// `smpl` MIDI unity note. FLAC has none: `fLmL`'s root note is ignored on load.
    pub unity_note: Option<u32>,
    /// The first loop's start and end frame: WAV `smpl` or FLAC `fLmL`, raw (the end is as stored, not +1) and fitted to
    /// the audio, so a player needs no check of its own: a start at or past the frame count is 0 and an end past it is
    /// the frame count. FLAC 0/0 is none (a WAV's is kept as stored: a loop that does not play, start = end).
    pub loop_frames: Option<(u32, u32)>,
    /// `acid` or `fLmL` number of beats.
    pub beats: Option<u32>,
    /// `acid` or `fLmL` tempo in BPM, when finite and positive.
    pub tempo: Option<f32>,
    /// `cue ` point positions (`dwPosition`) in frames: the slice markers.
    pub cues: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// No WAV, FLAC, Ogg Vorbis, Ogg FLAC or MPEG layer III stream, or a WAV other than PCM 8/16/24/32-bit and float
    /// 32-bit, plain or extensible (64-bit is rejected).
    Unsupported,
    /// A broken header or stream, or no audio at all.
    Malformed(&'static str),
    /// More frames than the limit.
    TooLarge,
}

impl SampleData {
    pub fn frames(&self) -> usize {
        self.left.len()
    }

    /// The sample at fractional frame `pos`, by cubic Hermite (Catmull-Rom) with neighbours clamped to the ends.
    /// Mono returns the same value twice. Outside `[0, frames)` it is silence.
    // Positions past either end read silence, so a player that overruns never
    // holds a DC level.
    pub fn read(&self, pos: f64) -> (f32, f32) {
        if !(pos >= 0.0 && pos < self.left.len() as f64) {
            return (0.0, 0.0);
        }
        let (i, t) = (pos as usize, pos.fract() as f32);
        let left = hermite(&self.left, i, t);
        (
            left,
            self.right
                .as_deref()
                .map_or(left, |right| hermite(right, i, t)),
        )
    }

    /// The read position's advance per output frame: source rate / device rate × 2^(semitones/12).
    pub fn step(&self, device_rate: f32, semitones: f32) -> f64 {
        f64::from(self.sample_rate) / f64::from(device_rate) * (f64::from(semitones) / 12.0).exp2()
    }
}

fn hermite(x: &[f32], i: usize, t: f32) -> f32 {
    let Some(last) = x.len().checked_sub(1) else {
        return 0.0;
    };
    let at = |k: usize| x[k.min(last)];
    let (p0, p1, p2, p3) = (at(i.saturating_sub(1)), at(i), at(i + 1), at(i + 2));
    let c1 = 0.5 * (p2 - p0);
    let c2 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c3 = 0.5 * (p3 - p0) + 1.5 * (p1 - p2);
    ((c3 * t + c2) * t + c1) * t + p1
}

pub fn decode(bytes: &[u8]) -> Result<SampleData, DecodeError> {
    decode_with_limit(bytes, MAX_FRAMES)
}

/// `decode` with a caller's frame limit instead of `MAX_FRAMES`.
pub fn decode_with_limit(bytes: &[u8], max_frames: usize) -> Result<SampleData, DecodeError> {
    let bytes = without_id3v2(bytes);
    let mp3 = || guarded("MP3 decoder panicked", || decode_mp3(bytes, max_frames));
    match (bytes.get(..4), bytes.get(8..12)) {
        // A RIFF the WAV reader rejects (MP3 in WAV is format tag 0x55) goes on to the MP3 reader. If that finds
        // nothing either, the WAV reader's error stands.
        (Some(b"RIFF"), Some(b"WAVE")) => match decode_wav(&bytes[12..], max_frames) {
            Err(error @ (DecodeError::Unsupported | DecodeError::Malformed(_))) => {
                mp3().map_err(|_| error)
            }
            result => result,
        },
        (Some(b"fLaC"), _) => guarded("FLAC decoder panicked", || decode_flac(bytes, max_frames)),
        (Some(b"OggS"), _) => guarded("Ogg decoder panicked", || decode_ogg(bytes, max_frames)),
        // MP3 has no magic, so it is the catch-all: its reader skips junk up to the first frame that a similar frame
        // follows.
        _ => mp3(),
    }
}

/// Third-party code on hostile bytes: a panic becomes an error where unwinding exists (not on wasm).
fn guarded(
    panicked: &'static str,
    decode: impl FnOnce() -> Result<SampleData, DecodeError> + std::panic::UnwindSafe,
) -> Result<SampleData, DecodeError> {
    std::panic::catch_unwind(decode).unwrap_or(Err(DecodeError::Malformed(panicked)))
}

/// Skips leading ID3v2 tags: a 10-byte header with a syncsafe size, then a 10-byte footer when flagged. A tag that
/// claims more than the file loses only its header, and the MP3 reader scans past the rest.
fn without_id3v2(mut bytes: &[u8]) -> &[u8] {
    while let Some(&[b'I', b'D', b'3', _, _, flags, a, b, c, d]) = bytes.get(..10) {
        let size = [a, b, c, d]
            .iter()
            .fold(0, |size, &byte| size << 7 | usize::from(byte & 0x7F));
        let footer = if flags & 0x10 != 0 { 10 } else { 0 };
        bytes = bytes.get(10 + size + footer..).unwrap_or(&bytes[10..]);
    }
    bytes
}

fn le_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// The RIFF chunks of `chunks` to its end, as (id, body): padded to even, and one cut short by the end keeps what is
/// there.
fn riff_chunks(mut chunks: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    std::iter::from_fn(move || {
        if chunks.len() < 8 {
            return None;
        }
        let size = le_u32(chunks, 4) as usize;
        let (id, rest) = (&chunks[..4], &chunks[8..]);
        chunks = rest
            .get(size.saturating_add(size & 1)..)
            .unwrap_or_default();
        Some((id, &rest[..size.min(rest.len())]))
    })
}

/// The positions of a `cue ` chunk's points (`dwPosition`, at +4 of each 24-byte point): as many as it claims and holds.
fn cue_points(body: &[u8]) -> Vec<u32> {
    let Some(points) = body.get(4..) else {
        return Vec::new();
    };
    let points = points.chunks_exact(24).take(le_u32(body, 0) as usize);
    points.map(|point| le_u32(point, 4)).collect()
}

/// A KSDATAFORMAT sub-format GUID, `0000xxxx-0000-0010-8000-00aa00389b71`, after its little-endian tag `xxxx`.
const KSDATAFORMAT_GUID_TAIL: [u8; 14] = [
    0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

/// Walks the chunks to the end of the bytes (the RIFF size is ignored). A chunk cut short by the end of the file,
/// `data` included, keeps what is there.
fn decode_wav(chunks: &[u8], max_frames: usize) -> Result<SampleData, DecodeError> {
    let (mut fmt, mut data, mut meta) = (None, None, SampleMeta::default());
    for (id, body) in riff_chunks(chunks) {
        match id {
            b"fmt " if fmt.is_none() => fmt = Some(body),
            b"data" if data.is_none() => data = Some(body),
            // First chunk of each kind wins.
            b"smpl" if body.len() >= 36 && meta.unity_note.is_none() => {
                meta.unity_note = Some(le_u32(body, 12));
                if le_u32(body, 28) > 0 && body.len() >= 52 {
                    meta.loop_frames = Some((le_u32(body, 44), le_u32(body, 48)));
                }
            }
            b"acid" if body.len() >= 24 && meta.beats.is_none() => {
                meta.beats = Some(le_u32(body, 12));
                meta.tempo =
                    Some(f32::from_bits(le_u32(body, 20))).filter(|t| t.is_finite() && *t > 0.0);
            }
            b"cue " if meta.cues.is_empty() => meta.cues = cue_points(body),
            _ => {}
        }
    }
    let fmt = fmt.ok_or(DecodeError::Malformed("no fmt chunk"))?;
    let data = data.ok_or(DecodeError::Malformed("no data chunk"))?;
    if fmt.len() < 16 {
        return Err(DecodeError::Malformed("short fmt chunk"));
    }
    let (tag, channels, sample_rate) =
        (le_u16(fmt, 0), usize::from(le_u16(fmt, 2)), le_u32(fmt, 4));
    // WAVE_FORMAT_EXTENSIBLE with a KSDATAFORMAT sub-format GUID reads
    // as the tag in its first two bytes, so PCM and float load and anything else is still unsupported. Many DAWs export
    // it. wValidBitsPerSample and the channel mask are ignored, as the container bits are all that is read.
    let tag = match tag {
        0xFFFE if fmt.get(26..40) == Some(&KSDATAFORMAT_GUID_TAIL) => le_u16(fmt, 24),
        tag => tag,
    };
    let (block_align, bits) = (usize::from(le_u16(fmt, 12)), le_u16(fmt, 14));
    // Float is 32-bit only.
    let float = match (tag, bits) {
        (1, 8 | 16 | 24 | 32) => false,
        (3, 32) => true,
        _ => return Err(DecodeError::Unsupported),
    };
    let width = usize::from(bits / 8);
    if channels == 0 || sample_rate == 0 {
        return Err(DecodeError::Malformed("no channels or no sample rate"));
    }
    if block_align != channels * width {
        return Err(DecodeError::Malformed(
            "block align is not channels × bits/8",
        ));
    }
    let stride = block_align;
    let frames = data.len() / stride;
    if frames == 0 {
        return Err(DecodeError::Malformed("no audio"));
    }
    if frames > max_frames {
        return Err(DecodeError::TooLarge);
    }
    // The loop is read raw and fitted to the frames that are there, so a player can rely on it.
    meta.loop_frames = meta.loop_frames.map(|l| clamp_loop(l, frames));
    let channel = |c: usize| -> Arc<[f32]> {
        let read = |frame: &[u8]| to_f32(float, &frame[c * width..(c + 1) * width]);
        data.chunks_exact(stride).map(read).collect()
    };
    Ok(SampleData {
        sample_rate,
        left: channel(0),
        right: (channels >= 2).then(|| channel(1)),
        meta,
    })
}

/// One little-endian sample. 8-bit is unsigned, value/128 − 1. Wider integers are signed and scaled by
/// 2^−(bits−1), read as the top bytes of an `i32`. Floats go through [`float_sample`].
fn to_f32(float: bool, b: &[u8]) -> f32 {
    let value = match (float, b.len()) {
        (true, _) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        (false, 1) => f32::from(b[0]) / 128.0 - 1.0,
        (false, n) => {
            let mut word = [0; 4];
            word[4 - n..].copy_from_slice(b);
            i32::from_le_bytes(word) as f32 / 2_147_483_648.0
        }
    };
    float_sample(value)
}

/// The largest float sample kept, about +24 dBFS.
// Float files may exceed 1.0, but a huge finite value (1.6e38 × a DW zone gain of 16 is
// inf) would poison every filter after it.
const MAX_FLOAT_SAMPLE: f32 = 16.0;

/// A stored float sample made safe: non-finite becomes silence, the rest is held to ±16.
pub(crate) fn float_sample(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-MAX_FLOAT_SAMPLE, MAX_FLOAT_SAMPLE)
    } else {
        0.0
    }
}

/// `fLaC`, STREAMINFO marked last, then the bytes after the last metadata block, and the `FLSM` data on the way.
/// symphonia sizes PICTURE and VORBIS_COMMENT buffers from unchecked lengths (a 58-byte file can ask for 4 GiB, which
/// aborts on wasm), and no other block is used.
fn flac_without_metadata(bytes: &[u8]) -> Option<(Vec<u8>, SampleMeta)> {
    let (mut at, mut streaminfo, mut meta) = (4, None, SampleMeta::default());
    while let Some(&[kind, a, b, c]) = bytes.get(at..at + 4) {
        let end = (at + 4).saturating_add(u32::from_be_bytes([0, a, b, c]) as usize);
        if kind & 0x7F == 0 && streaminfo.is_none() {
            streaminfo = Some(bytes.get(at + 1..end)?);
        }
        if let Some(block) = bytes.get(at..end) {
            read_flsm(block, &mut meta);
        }
        at = end;
        if kind & 0x80 != 0 {
            break;
        }
    }
    let mut out = b"fLaC\x80".to_vec();
    out.extend(streaminfo?);
    out.extend(bytes.get(at..).unwrap_or_default());
    Some((out, meta))
}

/// Reads a FLAC metadata block (its 4-byte header, then the data) into `meta` when it is an APPLICATION block with ID
/// `FLSM` (no other block is read). Its data is chunks of id[4], u32 LE length and payload, with no
/// padding, up to a zero length or a chunk that would overrun the block. `fLmL` (16 bytes): f32 tempo, u16 beats, u32
/// loop start and end, a root note (ignored) and a zero byte. `fLsF` holds RIFF chunks, of which only `cue ` is read.
/// The loop is as stored: `with_flsm` fits it to the audio.
// As for a WAV, the first chunk of each kind wins, across blocks too; a chunk of a
// kind the block does not read, or an `fLmL` of another length, is skipped and parsing goes on.
// "Already read" is keyed on what a chunk always sets: `meta.beats` for `fLmL` (the only FLAC source of beats; its
// tempo and loop may be none), so the first `fLmL` of the first FLSM block that has one wins whole. `fLsF` is keyed on
// `meta.cues`, so one without cue points lets a later one in.
fn read_flsm(block: &[u8], meta: &mut SampleMeta) {
    let [kind, a, b, c, data @ ..] = block else {
        return;
    };
    let length = u32::from_be_bytes([0, *a, *b, *c]) as usize;
    let Some(mut chunks) = data
        .get(..length)
        .and_then(|data| data.strip_prefix(b"FLSM"))
        .filter(|_| kind & 0x7F == 2)
    else {
        return;
    };
    while chunks.len() >= 8 {
        let length = le_u32(chunks, 4) as usize;
        let Some(payload) = chunks[8..].get(..length).filter(|p| !p.is_empty()) else {
            break;
        };
        match &chunks[..4] {
            b"fLmL" if length == 16 && meta.beats.is_none() => {
                meta.tempo =
                    Some(f32::from_bits(le_u32(payload, 0))).filter(|t| t.is_finite() && *t > 0.0);
                meta.beats = Some(u32::from(le_u16(payload, 4)));
                meta.loop_frames = Some((le_u32(payload, 6), le_u32(payload, 10)));
            }
            b"fLsF" if meta.cues.is_empty() => {
                let cue = riff_chunks(payload).find(|(id, _)| id == b"cue ");
                meta.cues = cue.map(|(_, body)| cue_points(body)).unwrap_or_default();
            }
            _ => {}
        }
        chunks = &chunks[8 + length..];
    }
}

/// `sample` with the `FLSM` `meta`, its loop fitted to the audio: a start at or past the frame count becomes 0, an
/// end past it becomes the frame count, and then 0/0 is no loop.
fn with_flsm(mut sample: SampleData, mut meta: SampleMeta) -> SampleData {
    let frames = sample.frames();
    meta.loop_frames = meta
        .loop_frames
        .map(|l| clamp_loop(l, frames))
        .filter(|&l| l != (0, 0));
    sample.meta = meta;
    sample
}

/// A loop's (start, end) as stored, fitted to `frames` (WAV `smpl` and FLAC `fLmL` alike): no +1 on the end.
pub(crate) fn clamp_loop((start, end): (u32, u32), frames: usize) -> (u32, u32) {
    let frames = u32::try_from(frames).unwrap_or(u32::MAX);
    (if start >= frames { 0 } else { start }, end.min(frames))
}

/// symphonia's fixed and LPC decoders slice `order` (up to 32) warm-up samples before checking the block holds them,
/// and panic on a shorter block.
const MIN_DECODED_BLOCK: u64 = 32;

fn decode_flac(bytes: &[u8], max_frames: usize) -> Result<SampleData, DecodeError> {
    use symphonia::default::{codecs::FlacDecoder, formats::FlacReader};
    const BAD: DecodeError = DecodeError::Malformed("bad FLAC stream");
    let (stream, meta) = flac_without_metadata(bytes).ok_or(BAD)?;
    decode_stream::<FlacReader, FlacDecoder>(
        stream,
        &FormatOptions::default(),
        MIN_DECODED_BLOCK,
        BAD,
        max_frames,
    )
    .map(|sample| with_flsm(sample, meta))
}

/// MPEG audio layer III, from the first run of `MP3_RUN` frames. Layers I and II are not built and are `Unsupported`.
// Gapless, so the encoder delay and padding a LAME/Xing tag records are trimmed and an
// MP3 sample starts on its beat; a file without the tag keeps them. MP3 has no timestamps: a frame whose header is
// destroyed is lost without a gap, while one that fails to decode becomes silence of its length.
fn decode_mp3(bytes: &[u8], max_frames: usize) -> Result<SampleData, DecodeError> {
    use symphonia::default::{codecs::MpaDecoder, formats::MpaReader};
    let gapless = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let start = mp3_start(bytes).ok_or(DecodeError::Unsupported)?;
    decode_stream::<MpaReader, MpaDecoder>(
        bytes[start..].to_vec(),
        &gapless,
        0,
        DecodeError::Unsupported,
        max_frames,
    )
}

/// Layer III frames in a row that MP3 decoding needs before it trusts a sync word, unless a shorter run ends the file.
// MP3 is the catch-all, and non-audio files often hold one or two header-like words
// but almost never four chained frames; real MP3s always start with them.
const MP3_RUN: usize = 4;

/// Where the first run of `MP3_RUN` MPEG layer III frames of one version, rate and channel mode starts, each frame
/// following the last, or a shorter run that ends the bytes. Layer I and II sync words are passed over.
fn mp3_start(bytes: &[u8]) -> Option<usize> {
    (0..bytes.len()).find(|&start| {
        let (mut at, mut kind) = (start, None);
        for _ in 0..MP3_RUN {
            if at == bytes.len() && kind.is_some() {
                return true;
            }
            let Some((len, frame)) = bytes.get(at..at + 4).and_then(layer3_frame) else {
                return false;
            };
            if *kind.get_or_insert(frame) != frame {
                return false;
            }
            at += len;
        }
        at <= bytes.len()
    })
}

/// An MPEG layer III frame header's frame length and kind (version, sample rate, mono), or `None`. A free-format frame
/// (bitrate index 0) is not one.
fn layer3_frame(header: &[u8]) -> Option<(usize, (u8, u8, bool))> {
    const KBPS: [[u16; 15]; 2] = [
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160], // MPEG-2 and 2.5
        [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ], // MPEG-1
    ];
    let &[0xFF, b, c, d] = header else {
        return None;
    };
    let (version, layer, bitrate, rate) = (b >> 3 & 3, b >> 1 & 3, c >> 4, c >> 2 & 3);
    // Sync, a version that exists (1 is reserved), layer III, and a bitrate and rate that exist.
    if b & 0xE0 != 0xE0 || version == 1 || layer != 1 || bitrate == 0 || bitrate == 15 || rate == 3
    {
        return None;
    }
    let mpeg1 = version == 3;
    let hz = [44_100, 48_000, 32_000][usize::from(rate)] >> [2, 0, 1, 0][usize::from(version)];
    let bits_per_second = 1000 * usize::from(KBPS[usize::from(mpeg1)][usize::from(bitrate)]);
    let len = if mpeg1 { 144 } else { 72 } * bits_per_second / hz + usize::from(c >> 1 & 1);
    Some((len, (version, rate, d >> 6 == 3)))
}

/// A symphonia reader and decoder over the stream (an owned copy: symphonia needs a `'static` source). `bad` is the
/// error when either won't start.
fn decode_stream<R: FormatReader, D: Decoder>(
    stream: Vec<u8>,
    options: &FormatOptions,
    min_dur: u64,
    bad: DecodeError,
    max_frames: usize,
) -> Result<SampleData, DecodeError> {
    let source = MediaSourceStream::new(Box::new(std::io::Cursor::new(stream)), Default::default());
    let mut reader = R::try_new(source, options).map_err(|_| bad)?;
    let params = reader.default_track().ok_or(bad)?.codec_params.clone();
    let sample_rate = params.sample_rate.filter(|&rate| rate > 0).ok_or(bad)?;
    let stereo = params
        .channels
        .is_some_and(|channels| channels.count() >= 2);
    let mut decoder = D::try_new(&params, &DecoderOptions::default()).map_err(|_| bad)?;
    let packets = std::iter::from_fn(|| reader.next_packet().ok());
    decode_packets(
        packets,
        &mut decoder,
        sample_rate,
        stereo,
        min_dur,
        usize::MAX,
        max_frames,
    )
}

/// Decodes `packets` into at most `max_frames` frames per channel, ending at frame `end`. Any reader error (EOF or
/// truncation) ends the stream, which keeps the frames decoded before it, as a cut WAV `data` chunk does.
// A packet that fails to decode, or one under `min_dur` frames (for FLAC only a
// final block under 1 ms in a valid file), becomes silence of its duration, and audio is padded with silence up to the
// timestamp of each packet that has audio, so a packet the reader dropped leaves its gap and later audio keeps its time.
// A stream whose first packet is not at 0 gains that much leading silence (still bounded by the frame limit). A packet
// with no frames (gapless trimming emptied an MP3's last one, but left its timestamp) adds nothing, and no padding: audio
// ends where the last packet that has any ends.
fn decode_packets(
    packets: impl Iterator<Item = Packet>,
    decoder: &mut dyn Decoder,
    sample_rate: u32,
    stereo: bool,
    min_dur: u64,
    end: usize,
    max_frames: usize,
) -> Result<SampleData, DecodeError> {
    use symphonia::core::audio::Signal;
    let (mut left, mut right, mut decoded_any) = (Vec::new(), Vec::new(), false);
    for packet in packets {
        let start = usize::try_from(packet.ts)
            .unwrap_or(usize::MAX)
            .max(left.len())
            .min(end);
        if start == end {
            break; // nothing after the end can add audio
        }
        let decoded = (packet.dur >= min_dur)
            .then(|| decoder.decode(&packet).ok())
            .flatten()
            .filter(|decoded| decoded.spec().channels.count() > 0);
        let frames = decoded
            .as_ref()
            .map_or(packet.dur as usize, |decoded| decoded.frames())
            .min(end - start);
        if frames == 0 {
            continue;
        }
        if start + frames > max_frames {
            return Err(DecodeError::TooLarge);
        }
        // Grow by doubling but never past the frame limit, and to exactly the need when that is more: silence padded
        // to a far position is one allocation of its size.
        for channel in [&mut left, &mut right]
            .into_iter()
            .take(1 + usize::from(stereo))
        {
            let capacity = channel.capacity();
            if capacity < start + frames {
                let target = (start + frames)
                    .max(capacity.saturating_mul(2))
                    .min(max_frames);
                channel.reserve_exact(target - channel.len());
            }
        }
        let silent_to = if decoded.is_some() {
            start
        } else {
            start + frames
        };
        left.resize(silent_to, 0.0);
        right.resize(if stereo { silent_to } else { 0 }, 0.0);
        let Some(decoded) = decoded else {
            continue;
        };
        decoded_any |= frames > 0;
        let channels = decoded.spec().channels.count();
        let mut buffer = decoded.make_equivalent::<f32>();
        decoded.convert(&mut buffer);
        // Non-finite samples (a damaged Vorbis codebook can decode to them) become silence, as in a float WAV.
        let finite = |&sample: &f32| if sample.is_finite() { sample } else { 0.0 };
        left.extend(buffer.chan(0)[..frames].iter().map(finite));
        if stereo {
            right.extend(
                buffer.chan(channels.min(2) - 1)[..frames]
                    .iter()
                    .map(finite),
            );
        }
    }
    if !decoded_any {
        return Err(DecodeError::Malformed("no audio"));
    }
    Ok(SampleData {
        sample_rate,
        left: left.into(),
        right: stereo.then(|| right.into()),
        meta: SampleMeta::default(),
    })
}

/// Ogg (RFC 3533) holding Vorbis or FLAC, demuxed here because symphonia's Ogg reader sizes Vorbis comments and FLAC
/// pictures with `vec![0; n]` from unchecked lengths and asserts on Vorbis setup headers. The first Vorbis or FLAC
/// logical stream is read as its pages arrive; comment, picture and other metadata packets are never handed over.
fn decode_ogg(bytes: &[u8], max_frames: usize) -> Result<SampleData, DecodeError> {
    use symphonia::core::codecs::{CODEC_TYPE_VORBIS, CodecParameters};
    use symphonia::default::codecs::VorbisDecoder;
    const BAD: DecodeError = DecodeError::Malformed("bad Ogg stream");
    let is_head =
        |packet: &[u8]| packet.starts_with(b"\x01vorbis") || packet.starts_with(b"\x7FFLAC");
    let head = ogg_pages(bytes)
        .find(|page| page.flags & 2 != 0 && is_head(page.body))
        .ok_or(DecodeError::Unsupported)?;
    let mut packets = ogg_packets(bytes, head.serial);
    if head.body.starts_with(b"\x7FFLAC") {
        // Ogg FLAC: 0x7F "FLAC", version and header count, then "fLaC" and STREAMINFO (51 bytes), then metadata
        // packets, then one frame per packet (sync byte 0xFF). The frames carry their own positions.
        let mut flac = head.body.get(9..51).ok_or(BAD)?.to_vec();
        flac[4] |= 0x80; // STREAMINFO is now the last metadata block
        let mut meta = SampleMeta::default();
        for (_, packet) in packets {
            if packet.first() == Some(&0xFF) {
                flac.extend(packet);
            } else {
                read_flsm(&packet, &mut meta); // a metadata block of its own
            }
        }
        return decode_flac(&flac, max_frames).map(|sample| with_flsm(sample, meta));
    }
    // The identification header comes first, the setup header third (Vorbis I §4.2.1); the comment is skipped.
    let mut next_header = |kind: &[u8]| {
        packets
            .find(|(_, packet)| packet.starts_with(kind))
            .map(|(_, packet)| packet)
            .ok_or(BAD)
    };
    let (ident, setup) = (next_header(b"\x01vorbis")?, next_header(b"\x05vorbis")?);
    if ident.len() < 30 || vorbis_setup_ok(&setup, &ident).is_none() {
        return Err(BAD);
    }
    let mut params = CodecParameters::new();
    params
        .for_codec(CODEC_TYPE_VORBIS)
        .with_extra_data([ident.as_slice(), &setup].concat().into_boxed_slice());
    let mut decoder =
        VorbisDecoder::try_new(&params, &DecoderOptions::default()).map_err(|_| BAD)?;
    // Audio packets have bit 0 clear (Vorbis I §4.3.1); header packets are odd.
    let audio = packets
        .filter(|(_, packet)| packet[0] & 1 == 0)
        .map(|(ts, packet)| Packet::new_from_boxed_slice(0, ts, 0, packet.into_boxed_slice()));
    // The last granule position of a page that ends a packet is the stream's length,
    // so the final block's tail is cut. Granule positions are taken as absolute, as FLAC frame positions are.
    let end = ogg_pages(bytes)
        .filter(|page| page.serial == head.serial && page.lacing.iter().any(|&lace| lace < 255))
        .map(|page| page.granule)
        .filter(|&granule| granule != u64::MAX)
        .last()
        .map_or(usize::MAX, |end| usize::try_from(end).unwrap_or(usize::MAX));
    decode_packets(
        audio,
        &mut decoder,
        le_u32(&ident, 12),
        ident[11] >= 2,
        0,
        end,
        max_frames,
    )
}

struct OggPage<'a> {
    flags: u8,
    granule: u64,
    serial: u32,
    sequence: u32,
    lacing: &'a [u8],
    body: &'a [u8],
}

/// The pages whose CRC holds, in order. A damaged page is skipped and the scan resumes at the next "OggS".
// The scan stops once the pages checked add up to four times the
// input, so a file packed with candidates that claim 64 KiB each costs linear time. Intact pages use at most one.
fn ogg_pages(bytes: &[u8]) -> impl Iterator<Item = OggPage<'_>> {
    let (mut at, mut budget) = (0, bytes.len().saturating_mul(4));
    std::iter::from_fn(move || {
        loop {
            let start = at + bytes.get(at..)?.windows(4).position(|w| w == b"OggS")?;
            at = start + 1;
            let Some(header) = bytes.get(start..start + 27) else {
                continue;
            };
            let segments = usize::from(header[26]);
            let Some(lacing) = bytes.get(start + 27..start + 27 + segments) else {
                continue;
            };
            let len = 27 + segments + lacing.iter().map(|&n| usize::from(n)).sum::<usize>();
            let Some(page) = bytes.get(start..start + len) else {
                continue;
            };
            if header[4] != 0 {
                continue;
            }
            budget = budget.checked_sub(len)?;
            if ogg_crc(page) != le_u32(header, 22) {
                continue;
            }
            at = start + len;
            return Some(OggPage {
                flags: header[5],
                granule: u64::from_le_bytes(header[6..14].try_into().unwrap_or_default()),
                serial: le_u32(header, 14),
                sequence: le_u32(header, 18),
                lacing,
                body: &page[27 + segments..],
            });
        }
    })
}

/// CRC-32 of an Ogg page: polynomial 0x04C11DB7, not reflected, with the CRC field read as zero.
fn ogg_crc(page: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0; 256];
        let mut byte = 0;
        while byte < 256 {
            let mut crc = (byte as u32) << 24;
            let mut bit = 0;
            while bit < 8 {
                crc = if crc & 0x8000_0000 != 0 {
                    (crc << 1) ^ 0x04C1_1DB7
                } else {
                    crc << 1
                };
                bit += 1;
            }
            table[byte] = crc;
            byte += 1;
        }
        table
    };
    page.iter().enumerate().fold(0, |crc, (at, &byte)| {
        let byte = if (22..26).contains(&at) { 0 } else { byte };
        (crc << 8) ^ TABLE[usize::from((crc >> 24) as u8 ^ byte)]
    })
}

/// The packets of logical stream `serial` as its pages arrive, each with the granule position of the last earlier page
/// that ended a packet (its audio starts there or later). Empty packets are skipped and a packet that lost a page is
/// dropped; only one page's packets are held at a time.
fn ogg_packets(bytes: &[u8], serial: u32) -> impl Iterator<Item = (u64, Vec<u8>)> + '_ {
    let mut pages = ogg_pages(bytes).filter(move |page| page.serial == serial);
    let (mut partial, mut next_sequence, mut granule) = (None, None, 0);
    let mut ready = std::collections::VecDeque::new();
    std::iter::from_fn(move || {
        loop {
            if let Some(packet) = ready.pop_front() {
                return Some(packet);
            }
            let page = pages.next()?;
            if page.flags & 1 == 0 {
                partial = Some(Vec::new());
            } else if next_sequence != Some(page.sequence) {
                partial = None; // the start of the continued packet is lost
            }
            next_sequence = Some(page.sequence.wrapping_add(1));
            let (mut body, mut ended) = (page.body, false);
            for &lace in page.lacing {
                let (segment, rest) = body.split_at(usize::from(lace));
                body = rest;
                if let Some(packet) = &mut partial {
                    packet.extend_from_slice(segment);
                }
                if lace < 255 {
                    if let Some(packet) = partial.replace(Vec::new())
                        && !packet.is_empty()
                    {
                        ready.push_back((granule, packet));
                    }
                    ended = true;
                }
            }
            if ended && page.granule != u64::MAX {
                granule = page.granule;
            }
        }
    })
}

/// Most codebook entries × dimensions one Vorbis setup header may hold in all.
// Symphonia sizes each codebook's tables from these counts unchecked
// (a 100-byte header can ask for terabytes, which aborts). Real setups hold at most about 63 000 (libvorbis q−1…10 at
// 8–96 kHz, mono to 5.1, and ffmpeg's own encoder); at 2^18 its per-entry tables stay near 1 MiB each.
const VORBIS_CODEBOOK_BUDGET: u64 = 1 << 18;

/// Most entries all of a Vorbis setup's codebook lookup tables may hold, 8 bytes each (16 MiB, 32 MiB while growing).
// Symphonia's table takes a block of up to 256 entries for each
// 8-bit level a codeword crosses, so its size follows the code's shape: 90 000 codewords can ask for 2.6 million.
// The real setups above reach about 123 000 by the bound `vorbis_setup_ok` uses (17× headroom).
const VORBIS_LOOKUP_BUDGET: u64 = 1 << 21;

/// Walks a Vorbis setup header (Vorbis I §4.2.4) and refuses what would make symphonia allocate past the budget or
/// panic. Codebooks (§3.2.1): zero dimensions or entries, used lengths over 32, an overfull code (its codeword builder
/// underflows on one at length 32), and type 1 lookups where its `lookup1_values` asserts or overflows. Floor 1
/// (§7.2): its renderer indexes a 256-entry dB table by value and its buffer by x, unchecked. Values stay in range
/// (§7.2.4) when no subclass book has more entries than the floor's range and the multiplier is not 3, whose first two
/// values are read with too many bits; every encoder measured uses books of exactly the range and multipliers 2 and 4.
/// The x range must fit the half block of every mode that uses the floor, and no x may repeat another or an implicit
/// end (0, 2^rangebits): the renderer divides by the gap. Floor 0 is refused: no current encoder writes it, and symphonia
/// reads its book index with more bits than its 16-entry book list holds. Residues: a class book past the last
/// codebook. Mappings: a type 2 residue over exactly two channels that do not start at channel 0 is refused (symphonia's
/// two-channel deinterleave takes the second index as relative to the first); libvorbis's 5.1 mapping, with two
/// submaps, is not one.
fn vorbis_setup_ok(setup: &[u8], ident: &[u8]) -> Option<()> {
    use symphonia::core::io::{BitReaderRtl, ReadBitsRtl};
    let mut bits = BitReaderRtl::new(setup.strip_prefix(b"\x05vorbis")?);
    macro_rules! read {
        ($n:expr) => {
            bits.read_bits_leq32($n).ok()?
        };
    }
    macro_rules! skip {
        ($n:expr) => {{
            let n = $n;
            bits.ignore_bits(n).ok()?
        }};
    }
    let (mut budget, mut lookup_budget) = (VORBIS_CODEBOOK_BUDGET, VORBIS_LOOKUP_BUDGET);
    let mut book_entries = Vec::new();
    for _ in 0..=read!(8) {
        let (sync, dimensions, entries) = (read!(24), read!(16), read!(24));
        book_entries.push(entries);
        budget = budget.checked_sub(u64::from(entries) * u64::from(dimensions))?;
        if sync != 0x56_4342 || dimensions == 0 || entries == 0 {
            return None;
        }
        // Kraft sums, in units of 2^-32, of the codewords longer than 0, 8, 16 and 24 bits, and the longest length.
        let (mut longer_than, mut longest) = ([0u64; 4], 0);
        let mut add = |length: u32, count: u32| {
            let weight = u64::from(count) << 32u32.checked_sub(length)?;
            for (level, sum) in longer_than.iter_mut().enumerate() {
                if length > 8 * level as u32 {
                    *sum += weight;
                }
            }
            longest = longest.max(length);
            Some(())
        };
        if read!(1) == 1 {
            // Length-ordered: a run of entries for each length, starting at a given length.
            let (mut entry, mut length) = (0, read!(5) + 1);
            while entry < entries {
                let run = read!(32 - (entries - entry).leading_zeros());
                if run > 0 {
                    add(length, run)?;
                }
                (entry, length) = (entry + run, length + 1);
            }
        } else {
            let sparse = read!(1) == 1;
            for _ in 0..entries {
                if !sparse || read!(1) == 1 {
                    add(read!(5) + 1, 1)?;
                }
            }
        }
        if longer_than[0] > 1 << 32 {
            return None; // an overfull code
        }
        // The lookup table has a block for the root and for each node at depth 8, 16 and 24 that longer codewords pass
        // through, of up to 2^min(8, longest − depth) entries. In a complete code (symphonia refuses others but a
        // one-entry code) the nodes at a depth number exactly the Kraft sum of the longer codewords there, rounded up.
        for (level, &sum) in longer_than.iter().enumerate() {
            let depth = 8 * level as u32;
            if depth < longest {
                let nodes = if level == 0 {
                    1
                } else {
                    sum.div_ceil(1 << (32 - depth))
                };
                lookup_budget = lookup_budget.checked_sub(nodes << (longest - depth).min(8))?;
            }
        }
        match read!(4) {
            0 => {}
            lookup @ (1 | 2) => {
                skip!(64); // minimum and delta values
                let value_bits = read!(4) + 1;
                skip!(1); // sequence flag
                let values = if lookup == 1 {
                    lookup1_values(entries, dimensions)?
                } else {
                    entries * dimensions
                };
                skip!(values.checked_mul(value_bits)?);
            }
            _ => return None,
        }
    }
    for _ in 0..=read!(6) {
        if read!(16) != 0 {
            return None; // time-domain transforms are placeholders
        }
    }
    // Each floor 1's x range, 2^rangebits.
    let mut floor_widths = Vec::new();
    for _ in 0..=read!(6) {
        match read!(16) {
            1 => {
                let mut partitions = [0; 32];
                let partitions = &mut partitions[..read!(5) as usize];
                for class in partitions.iter_mut() {
                    *class = read!(4) as usize;
                }
                let (mut dimensions, mut most_entries) = ([0; 16], 0);
                for class_dimensions in
                    &mut dimensions[..partitions.iter().max().map_or(0, |&c| c + 1)]
                {
                    *class_dimensions = read!(3) + 1;
                    let subclass_bits = read!(2);
                    skip!(if subclass_bits > 0 { 8 } else { 0 }); // master book
                    for _ in 0..1 << subclass_bits {
                        if let Some(book) = (read!(8) as usize).checked_sub(1) {
                            most_entries = most_entries.max(*book_entries.get(book)?);
                        }
                    }
                }
                let range = [256, 128, 0, 64][read!(2) as usize];
                if range == 0 || most_entries > range {
                    return None; // multiplier 3, or values past the range
                }
                let range_bits = read!(4);
                let mut xs = vec![0, 1 << range_bits];
                for &class in partitions.iter() {
                    for _ in 0..dimensions[class] {
                        let x = read!(range_bits);
                        if xs.contains(&x) {
                            return None;
                        }
                        xs.push(x);
                    }
                }
                floor_widths.push(1 << range_bits);
            }
            _ => return None,
        }
    }
    let mut residue_types = Vec::new();
    for _ in 0..=read!(6) {
        residue_types.push(read!(16));
        skip!(3 * 24); // begin, end, partition size
        let classifications = read!(6) + 1;
        if read!(8) as usize >= book_entries.len() {
            return None; // a class book symphonia indexes unchecked
        }
        let mut books = 0;
        for _ in 0..classifications {
            let low = read!(3);
            let high = if read!(1) == 1 { read!(5) } else { 0 };
            books += (high << 3 | low).count_ones();
        }
        skip!(8 * books);
    }
    let channels = u32::from(ident[11]);
    // Each mapping's floors, one per submap.
    let mut mapping_floors = Vec::new();
    for _ in 0..=read!(6) {
        if read!(16) != 0 {
            return None;
        }
        let submaps = if read!(1) == 1 { read!(4) + 1 } else { 1 };
        if read!(1) == 1 {
            let coupling_bits = 32 - channels.checked_sub(1)?.leading_zeros();
            skip!((read!(8) + 1) * 2 * coupling_bits);
        }
        skip!(2); // reserved
        let mut multiplex = vec![0; channels as usize];
        if submaps > 1 {
            for submap in &mut multiplex {
                *submap = read!(4);
            }
        }
        let mut floors = Vec::new();
        for submap in 0..submaps {
            skip!(8); // time
            floors.push(read!(8) as usize);
            let residue = read!(8) as usize;
            let members: Vec<usize> = (0..multiplex.len())
                .filter(|&channel| multiplex[channel] == submap)
                .collect();
            // Guards symphonia-codec-vorbis 0.5.5, `residue.rs`, `deinterleave_2`, whose two-channel branch (taken for a
            // type 2 residue over exactly two channels) is `channels.split_at_mut(ch0).1.split_at_mut(ch1)` and then
            // `b[0]` (lines 185-187): the second split is on the slice that starts at the first channel, but `ch1` is
            // an index into all channels. The two agree only when ch0 == 0. Otherwise it writes channel ch0 + ch1 (and
            // the real second channel is not written), or panics when `ch1` is past that slice's end or `b` is empty.
            // Every other case is safe: any other channel count takes the generic branch (`stride =>`, lines 201-212),
            // which indexes `channels[ch]` by absolute index from `residue_channels.iter()` (count 1 is stride 1;
            // count 0 never enters its loop; 3 or more stride by the count), and residue types 0 and 1 never call
            // `deinterleave_2` and index `channels[ch]` the same way (`read_residue_inner_type_0_1`). So a first
            // channel that is not 0 is fine with any count but two; libvorbis's 5.1 mapping has two submaps (channels
            // 0-4 as type 2, the LFE alone) and is not refused. `members` is the submap's `residue_channels` (`lib.rs`
            // sets a channel in the submap its multiplex value names; `common.rs` `BitSet256::iter` ascends).
            if *residue_types.get(residue)? == 2 && members.len() == 2 && members[0] != 0 {
                return None;
            }
        }
        mapping_floors.push(floors);
    }
    for _ in 0..=read!(6) {
        let block_exponent = if read!(1) == 1 {
            ident[28] >> 4
        } else {
            ident[28] & 15
        };
        skip!(32); // window and transform types
        for &floor in mapping_floors.get(read!(8) as usize)? {
            if *floor_widths.get(floor)? > 1u32 << block_exponent >> 1 {
                return None;
            }
        }
    }
    Some(())
}

/// symphonia's `lookup1_values` (Vorbis I §9.2.3, computed in `f32`), or `None` where its asserts would fail or
/// overflow.
fn lookup1_values(entries: u32, dimensions: u32) -> Option<u32> {
    let value = (entries as f32).powf(1.0 / dimensions as f32).floor() as u32;
    let fits = value.checked_pow(dimensions)? <= entries
        && value.checked_add(1)?.checked_pow(dimensions)? > entries;
    fits.then_some(value)
}

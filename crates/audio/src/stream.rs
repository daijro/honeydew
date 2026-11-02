//! Samples streamed from disk (issues #38 and #39 §8): a sample too big for the memory budget keeps only its heads in
//! memory, [`HEAD_SECONDS`] from each frame a voice can start at (its start, a region's `offset`, its loop start, a
//! reversed DW zone's end), and the rest is read from its file while it plays.
//!
//! A PCM WAV, a FLAC that says how long it is, and a `.dwp`'s embedded PCM or FLAC stream ([`Source`]); any other file
//! (MP3, Ogg, a bundled member, the web's copies) loads whole as before. A WAV is read at its frame's byte, a FLAC from
//! symphonia's seek, and both decode exactly as [`crate::sample::decode`] does.
//!
//! Each instrument that plays one keeps [`Rings`], [`STREAM_VOICES`] rings of [`RING_FRAMES`]: its voices below
//! [`STREAM_VOICES`] are the ones that may play a streamed sample, the others play samples in memory. A voice reads its
//! heads at once and its ring after them. One [`Streamer`] thread per engine fills the rings ahead of the voices; the
//! audio thread only loads and stores atomics (no lock, allocation or file read). A frame the ring does not hold yet is
//! silence for that voice, and an underrun is counted. Rings no streamer took (an offline render's) are filled on the reader's own
//! thread as it needs them: a render never misses a frame.
//!
//! A voice reads a segment: from a frame its heads hold, forward (or backward, a reversed DW zone), with the ring
//! taking over where the heads end. A loop's wrap starts a new segment at the loop start, whose head is in memory; a
//! loop its heads hold whole needs none.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::default::{codecs::FlacDecoder, formats::FlacReader};

use crate::sample::{
    MAX_FRAMES, MIN_DECODED_BLOCK, SampleData, SampleMeta, clamp_loop, fit_flsm, read_flsm, to_f32,
    wav_format,
};

/// The seconds of a streamed sample kept in memory from each frame a voice can start at: what a voice plays while the
/// streamer opens and seeks its file.
pub const HEAD_SECONDS: f64 = 0.5;
/// The frames each voice's ring holds ahead of it.
// ponytail: 16 384 frames (0.34 s at 48 kHz), 128 KiB a ring, 8 MiB for an instrument's [`STREAM_VOICES`]; raise it if
// slow storage underruns.
pub const RING_FRAMES: usize = 1 << 14;
/// The voices of an instrument that may play a streamed sample at once: the rings it keeps, whatever its polyphony. A
/// streamed note past them steals the oldest of them (the DW Sampler drops it, as it drops any note with no voice free).
// ponytail: 64 rings are 8 MiB an instrument (512 would be 64 MiB, too much on a phone), and 64 voices reading 48 kHz
// stereo 24-bit (18 MB/s in all) is about what a phone's storage sustains in 4096-frame random reads. Raise it with
// faster storage and the memory to spare.
pub const STREAM_VOICES: usize = 64;
/// The frames the streamer reads at a time.
const CHUNK: usize = 4096;
/// The longest FLAC block, past which a read seeks rather than decodes its way forward.
const MAX_BLOCK: u64 = 1 << 16;

/// Where a streamed sample's frames are in its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Interleaved little-endian PCM from byte `offset`, `width` bytes a sample (8-bit unsigned, 16, 24 or 32-bit
    /// signed, or `f32`), `channels` a frame.
    Pcm {
        offset: u64,
        channels: u16,
        width: u8,
        float: bool,
    },
    /// A FLAC stream whose frames are bytes `start..end`, with its STREAMINFO `info`; its first `skip` frames are not
    /// the sample's.
    Flac {
        start: u64,
        end: u64,
        skip: u64,
        info: [u8; 34],
    },
}

/// A sample file that can stream: where its frames are, and what [`crate::sample::decode`] would make of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub path: PathBuf,
    pub layout: Layout,
    pub rate: u32,
    pub stereo: bool,
    pub frames: usize,
    pub meta: SampleMeta,
}

impl Source {
    /// The sample file at `path` when it can stream: a PCM WAV or a FLAC whose STREAMINFO counts its frames, at most
    /// `MAX_FRAMES`. `None` for any other (MP3, Ogg, an ID3-tagged file, a broken one), which loads whole.
    pub fn open(path: &Path) -> Option<Self> {
        let mut file = File::open(path).ok()?;
        let mut magic = [0; 12];
        file.read_exact(&mut magic).ok()?;
        match (&magic[..4], &magic[8..]) {
            (b"RIFF", b"WAVE") => wav(file, path),
            (b"fLaC", _) => {
                let len = file.metadata().ok()?.len();
                Self::flac(path, 0, len, 0, usize::MAX)
            }
            _ => None,
        }
    }

    /// Interleaved PCM in `path` (a `.dwp`'s 0x205 record).
    pub fn pcm(path: &Path, layout: Layout, rate: u32, stereo: bool, frames: usize) -> Self {
        Self {
            path: path.to_owned(),
            layout,
            rate,
            stereo,
            frames,
            meta: SampleMeta::default(),
        }
    }

    /// The FLAC stream in bytes `offset..end` of `path`, its first `skip` frames left out and at most `limit` after them
    /// (a FLAC file, or a `.dwp`'s 0x206 record). Its metadata blocks are read here, never by symphonia, which sizes some
    /// from unchecked lengths ([`crate::sample`]).
    pub fn flac(path: &Path, offset: u64, end: u64, skip: u64, limit: usize) -> Option<Self> {
        let mut file = File::open(path).ok()?;
        file.seek(SeekFrom::Start(offset)).ok()?;
        if body(&mut file, 4)? != b"fLaC" {
            return None;
        }
        let (mut at, mut info, mut meta) = (offset + 4, None, SampleMeta::default());
        loop {
            let mut header = [0; 4];
            file.seek(SeekFrom::Start(at)).ok()?;
            file.read_exact(&mut header).ok()?;
            let len = u64::from(u32::from_be_bytes([0, header[1], header[2], header[3]]));
            let kind = header[0] & 0x7F;
            if kind == 0 && info.is_none() && len >= 34 {
                info = Some(<[u8; 34]>::try_from(&body(&mut file, 34)?[..]).ok()?);
            } else if kind == 2 && len <= 1 << 20 {
                let mut block = header.to_vec();
                block.extend(body(&mut file, len)?);
                read_flsm(&block, &mut meta);
            }
            at += 4 + len;
            if header[0] & 0x80 != 0 || at >= end {
                break;
            }
        }
        let info = info?;
        let word = u64::from_be_bytes(info[10..18].try_into().ok()?);
        let (rate, channels, total) = (
            (word >> 44) as u32,
            (word >> 41 & 7) + 1,
            word & 0xF_FFFF_FFFF,
        );
        let frames = usize::try_from(total.checked_sub(skip)?).ok()?.min(limit);
        if rate == 0 || frames == 0 || frames > MAX_FRAMES || at >= end {
            return None;
        }
        Some(Self {
            path: path.to_owned(),
            layout: Layout::Flac {
                start: at,
                end,
                skip,
                info,
            },
            rate,
            stereo: channels >= 2,
            frames,
            meta: fit_flsm(meta, usize::try_from(total).ok()?),
        })
    }

    /// The memory the whole sample takes decoded.
    pub fn whole_bytes(&self) -> usize {
        4 * self.frames * (1 + usize::from(self.stereo))
    }
}

/// `len` bytes of `file` from where it is.
fn body(file: &mut File, len: u64) -> Option<Vec<u8>> {
    let mut bytes = vec![0; usize::try_from(len).ok()?];
    file.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

/// A WAV's `fmt `, `data` and `smpl` chunks, read as [`crate::sample::decode`] reads them, `data` left on disk.
fn wav(mut file: File, path: &Path) -> Option<Source> {
    let len = file.metadata().ok()?.len();
    let (mut at, mut fmt, mut data, mut smpl) = (12, None, None, None::<Vec<u8>>);
    while at + 8 <= len {
        let mut header = [0; 8];
        file.seek(SeekFrom::Start(at)).ok()?;
        file.read_exact(&mut header).ok()?;
        let size = u64::from(u32::from_le_bytes([
            header[4], header[5], header[6], header[7],
        ]));
        let kept = size.min(len - at - 8);
        match &header[..4] {
            b"fmt " if fmt.is_none() => fmt = Some(body(&mut file, kept.min(64))?),
            b"data" if data.is_none() => data = Some((at + 8, kept)),
            b"smpl" if smpl.is_none() && kept >= 36 => smpl = Some(body(&mut file, kept.min(52))?),
            _ => {}
        }
        at += 8 + size + size % 2;
    }
    let (channels, rate, width, float) = wav_format(&fmt?).ok()?;
    let (offset, bytes) = data?;
    let frames = usize::try_from(bytes).ok()? / (channels * width);
    if frames == 0 || frames > MAX_FRAMES {
        return None;
    }
    let le = |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    let mut meta = SampleMeta::default();
    if let Some(smpl) = smpl {
        meta.unity_note = Some(le(&smpl, 12));
        if le(&smpl, 28) > 0 && smpl.len() >= 52 {
            meta.loop_frames = Some(clamp_loop((le(&smpl, 44), le(&smpl, 48)), frames));
        }
    }
    Some(Source {
        path: path.to_owned(),
        layout: Layout::Pcm {
            offset,
            channels: channels as u16,
            width: width as u8,
            float,
        },
        rate,
        stereo: channels >= 2,
        frames,
        meta,
    })
}

/// The frames a head holds at `rate`.
pub fn head_frames(rate: u32) -> usize {
    (HEAD_SECONDS * f64::from(rate)).ceil() as usize
}

/// The parts of a sample of `frames` at `rate` kept in memory: [`head_frames`] from each of `starts` (any order), merged
/// where they meet, in order.
pub fn head_spans(frames: usize, rate: u32, starts: &[usize]) -> Vec<(usize, usize)> {
    let len = head_frames(rate).max(1);
    let mut spans: Vec<_> = (starts.iter().filter(|&&s| s < frames))
        .map(|&s| (s, s.saturating_add(len).min(frames)))
        .collect();
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    merged
}

/// An open [`Source`], read anywhere: a PCM one at its frame's byte, a FLAC one where the last read ended or after a
/// seek.
pub struct Cursor {
    file: File,
    bytes: Vec<u8>,
    flac: Option<Flac>,
}

struct Flac {
    reader: FlacReader,
    decoder: FlacDecoder,
    /// The last packet decoded: its first frame in the stream, and its frames.
    at: u64,
    left: Vec<f32>,
    right: Vec<f32>,
}

/// A FLAC stream's STREAMINFO, then its frames from the file: what symphonia reads.
struct FlacBytes {
    file: File,
    header: Vec<u8>,
    /// The file's frames: `start..end`.
    start: u64,
    end: u64,
    pos: u64,
}

impl FlacBytes {
    fn len(&self) -> u64 {
        self.header.len() as u64 + self.end - self.start
    }
}

impl Read for FlacBytes {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let head = self.header.len() as u64;
        if self.pos < head {
            let from = &self.header[self.pos as usize..];
            let n = from.len().min(out.len());
            out[..n].copy_from_slice(&from[..n]);
            self.pos += n as u64;
            return Ok(n);
        }
        let left = self.len().saturating_sub(self.pos);
        let n = (out.len() as u64).min(left) as usize;
        self.file
            .seek(SeekFrom::Start(self.start + self.pos - head))?;
        let n = self.file.read(&mut out[..n])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for FlacBytes {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::End(by) => self.len().checked_add_signed(by),
            SeekFrom::Current(by) => self.pos.checked_add_signed(by),
        };
        self.pos = pos.ok_or_else(|| io::Error::other("seek before the start"))?;
        Ok(self.pos)
    }
}

impl MediaSource for FlacBytes {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len())
    }
}

impl Flac {
    /// The stream of `layout` in `file` from its start, its metadata blocks but STREAMINFO left out.
    fn open(file: &File, layout: &Layout) -> Option<Self> {
        let Layout::Flac {
            start, end, info, ..
        } = *layout
        else {
            return None;
        };
        let file = file.try_clone().ok()?;
        let mut header = b"fLaC\x80\0\0\x22".to_vec();
        header.extend(info);
        let bytes = FlacBytes {
            file,
            header,
            start,
            end,
            pos: 0,
        };
        let source = MediaSourceStream::new(Box::new(bytes), Default::default());
        let reader = FlacReader::try_new(source, &FormatOptions::default()).ok()?;
        let params = reader.default_track()?.codec_params.clone();
        let decoder = FlacDecoder::try_new(&params, &DecoderOptions::default()).ok()?;
        Some(Self {
            reader,
            decoder,
            at: 0,
            left: Vec::new(),
            right: Vec::new(),
        })
    }

    /// Decodes the next packet: frames at its time stamp, silence for one that is too short or fails to decode, as
    /// [`crate::sample::decode`] does. False at the end.
    fn next(&mut self, stereo: bool) -> bool {
        use symphonia::core::audio::Signal;
        let Ok(packet) = self.reader.next_packet() else {
            return false;
        };
        let decoded = (packet.dur >= MIN_DECODED_BLOCK)
            .then(|| self.decoder.decode(&packet).ok())
            .flatten()
            .filter(|decoded| decoded.spec().channels.count() > 0);
        self.at = packet.ts;
        self.left.clear();
        self.right.clear();
        let Some(decoded) = decoded else {
            let frames = usize::try_from(packet.dur).unwrap_or(0);
            self.left.resize(frames, 0.0);
            self.right.resize(frames, 0.0);
            return true;
        };
        let channels = decoded.spec().channels.count();
        let mut buffer = decoded.make_equivalent::<f32>();
        decoded.convert(&mut buffer);
        let finite = |&sample: &f32| if sample.is_finite() { sample } else { 0.0 };
        self.left.extend(buffer.chan(0).iter().map(finite));
        let right = if stereo { channels.min(2) - 1 } else { 0 };
        self.right.extend(buffer.chan(right).iter().map(finite));
        true
    }

    /// Seeks so that the next packets lead to frame `to`: the packet that holds it, or one before.
    fn seek(&mut self, to: u64) -> bool {
        for target in [to, to.saturating_sub(MAX_BLOCK), 0] {
            let seek = SeekTo::TimeStamp {
                ts: target,
                track_id: 0,
            };
            if let Ok(seeked) = self.reader.seek(SeekMode::Accurate, seek)
                && seeked.actual_ts <= to
            {
                self.decoder.reset();
                (self.at, self.left, self.right) = (seeked.actual_ts, Vec::new(), Vec::new());
                return true;
            }
        }
        false
    }
}

impl Cursor {
    pub fn open(source: &Source) -> Option<Self> {
        let file = File::open(&source.path).ok()?;
        let flac = match source.layout {
            Layout::Flac { .. } => Some(Flac::open(&file, &source.layout)?),
            Layout::Pcm { .. } => None,
        };
        Some(Self {
            file,
            bytes: Vec::new(),
            flac,
        })
    }

    /// Frames `from..` of `source` into `left` and `right` (as many as they hold; the right is the left for mono).
    /// Frames the file does not have are silence.
    pub fn read(&mut self, source: &Source, from: usize, left: &mut [f32], right: &mut [f32]) {
        left.fill(0.0);
        right.fill(0.0);
        let n = left
            .len()
            .min(right.len())
            .min(source.frames.saturating_sub(from));
        // Third-party code on hostile bytes: a panic leaves the rest silent.
        let read = std::panic::catch_unwind(AssertUnwindSafe(|| match source.layout {
            Layout::Pcm {
                offset,
                channels,
                width,
                float,
            } => self.read_pcm(
                offset,
                channels,
                width,
                float,
                from,
                &mut left[..n],
                &mut right[..n],
            ),
            Layout::Flac { skip, .. } => self.read_flac(
                source.stereo,
                skip + from as u64,
                &mut left[..n],
                &mut right[..n],
            ),
        }));
        if read.is_err() {
            self.flac = None;
        }
        if !source.stereo {
            right[..n].copy_from_slice(&left[..n]);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn read_pcm(
        &mut self,
        offset: u64,
        channels: u16,
        width: u8,
        float: bool,
        from: usize,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        let (width, stride) = (
            usize::from(width),
            usize::from(channels) * usize::from(width),
        );
        let at = offset + (from * stride) as u64;
        self.bytes.clear();
        if self.file.seek(SeekFrom::Start(at)).is_err() {
            return;
        }
        let want = (left.len() * stride) as u64;
        if (&mut self.file)
            .take(want)
            .read_to_end(&mut self.bytes)
            .is_err()
        {
            return;
        }
        let frames = self.bytes.chunks_exact(stride);
        for ((frame, l), r) in frames.zip(left.iter_mut()).zip(right.iter_mut()) {
            *l = to_f32(float, &frame[..width]);
            if channels >= 2 {
                *r = to_f32(float, &frame[width..2 * width]);
            }
        }
    }

    fn read_flac(&mut self, stereo: bool, from: u64, left: &mut [f32], right: &mut [f32]) {
        let Some(flac) = &mut self.flac else {
            return;
        };
        // `next`: the packet in hand followed one that ended at or before the frame wanted. One seek a read at most: then
        // it decodes its way on. `tries`: a hostile stream gives up.
        let (mut done, mut next, mut sought, mut tries) = (0, false, false, 0);
        while done < left.len() {
            let at = from + done as u64;
            let end = flac.at + flac.left.len() as u64;
            if (flac.at..end).contains(&at) {
                let skip = (at - flac.at) as usize;
                let n = (flac.left.len() - skip).min(left.len() - done);
                left[done..done + n].copy_from_slice(&flac.left[skip..skip + n]);
                right[done..done + n].copy_from_slice(&flac.right[skip..skip + n]);
                done += n;
                continue;
            }
            if next && at < flac.at {
                // A gap the stream leaves is silence, as the whole decode pads it.
                done += ((flac.at - at) as usize).min(left.len() - done);
                continue;
            }
            tries += 1;
            if tries > MAX_BLOCK {
                return;
            }
            // Behind the packet, or far past it: seek; else decode on to it.
            if !sought && (at < flac.at || at > end + MAX_BLOCK) {
                if !flac.seek(at) {
                    return;
                }
                sought = true;
            }
            if !flac.next(stereo) {
                return;
            }
            next = true;
        }
    }
}

/// A sample streamed from disk: its [`Source`], and its heads in memory.
pub struct Streamed {
    pub source: Source,
    /// The parts in memory, apart and in order: their first frame, then each channel's frames.
    heads: Vec<(usize, Vec<f32>, Option<Vec<f32>>)>,
}

impl std::fmt::Debug for Streamed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Streamed")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl Streamed {
    /// `source` with [`head_spans`] of `starts` read into memory now (blocking); `None` when its file does not open.
    pub fn new(source: Source, starts: &[usize]) -> Option<Self> {
        let mut cursor = Cursor::open(&source)?;
        let heads = head_spans(source.frames, source.rate, starts)
            .into_iter()
            .map(|(a, b)| {
                let (mut left, mut right) = (vec![0.0; b - a], vec![0.0; b - a]);
                cursor.read(&source, a, &mut left, &mut right);
                (a, left, source.stereo.then_some(right))
            })
            .collect();
        Some(Self { source, heads })
    }

    /// The whole sample, read from its file now (blocking): for a writer that needs every frame in memory (an FLP
    /// export's sampler).
    pub fn whole(&self) -> Option<SampleData> {
        let mut cursor = Cursor::open(&self.source)?;
        let frames = self.source.frames;
        let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
        cursor.read(&self.source, 0, &mut left, &mut right);
        Some(SampleData {
            sample_rate: self.source.rate,
            left: left.into(),
            right: self.source.stereo.then(|| right.into()),
            meta: self.source.meta.clone(),
        })
    }

    /// The memory its heads take.
    pub fn byte_len(&self) -> usize {
        let frames = |(_, l, r): &(usize, Vec<f32>, Option<Vec<f32>>)| {
            l.len() + r.as_ref().map_or(0, Vec::len)
        };
        4 * self.heads.iter().map(frames).sum::<usize>()
    }

    /// The head holding frame `f`: its frames.
    fn head(&self, f: usize) -> Option<(usize, usize)> {
        (self.heads.iter())
            .map(|(a, l, _)| (*a, a + l.len()))
            .find(|&(a, b)| (a..b).contains(&f))
    }

    /// Frame `f` from a head, `None` where no head holds it.
    fn ram(&self, f: usize) -> Option<(f32, f32)> {
        let (a, left, right) = self
            .heads
            .iter()
            .find(|(a, l, _)| (*a..a + l.len()).contains(&f))?;
        let l = left[f - a];
        Some((l, right.as_ref().map_or(l, |r| r[f - a])))
    }

    /// Frames `from..to` are all in one head: a loop over them never reads its ring.
    pub fn in_memory(&self, from: usize, to: usize) -> bool {
        self.head(from).is_some_and(|(_, end)| to <= end)
    }
}

/// A region's or zone's sample: whole in memory, or streamed from disk.
#[derive(Clone, Debug)]
pub enum Pcm {
    Ram(Arc<SampleData>),
    Disk(Arc<Streamed>),
}

impl From<Arc<SampleData>> for Pcm {
    fn from(data: Arc<SampleData>) -> Self {
        Self::Ram(data)
    }
}

impl Pcm {
    pub fn frames(&self) -> usize {
        match self {
            Self::Ram(data) => data.frames(),
            Self::Disk(s) => s.source.frames,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        match self {
            Self::Ram(data) => data.sample_rate,
            Self::Disk(s) => s.source.rate,
        }
    }

    pub fn stereo(&self) -> bool {
        match self {
            Self::Ram(data) => data.right.is_some(),
            Self::Disk(s) => s.source.stereo,
        }
    }

    pub fn meta(&self) -> &SampleMeta {
        match self {
            Self::Ram(data) => &data.meta,
            Self::Disk(s) => &s.source.meta,
        }
    }

    /// The memory it holds: the decoded PCM, or the heads.
    pub fn byte_len(&self) -> usize {
        match self {
            Self::Ram(data) => 4 * (data.left.len() + data.right.as_ref().map_or(0, |r| r.len())),
            Self::Disk(s) => s.byte_len(),
        }
    }

    /// Its identity, for sharing and for what an engine build hears.
    pub fn ptr(&self) -> *const () {
        match self {
            Self::Ram(data) => Arc::as_ptr(data).cast(),
            Self::Disk(s) => Arc::as_ptr(s).cast(),
        }
    }

    pub fn disk(&self) -> Option<Arc<Streamed>> {
        match self {
            Self::Ram(_) => None,
            Self::Disk(s) => Some(s.clone()),
        }
    }

    /// The whole sample in memory: its data, or a streamed one read from its file now (blocking).
    pub fn whole(&self) -> Option<Arc<SampleData>> {
        match self {
            Self::Ram(data) => Some(data.clone()),
            Self::Disk(s) => s.whole().map(Arc::new),
        }
    }
}

// A ring's words: an epoch (16 bits, a new one per segment) over a path index (48 bits: frames along the segment from
// its origin). A segment packs its epoch, the sample (20 bits, `IDLE` for none), its direction and its origin (27
// bits, past `MAX_FRAMES`).
const IDLE: u64 = (1 << 20) - 1;
const LOW: u64 = (1 << 48) - 1;

fn pack(epoch: u64, path: u64) -> u64 {
    epoch << 48 | path.min(LOW)
}

fn unpack(word: u64) -> (u64, u64) {
    (word >> 48, word & LOW)
}

#[derive(Clone, Copy)]
struct Segment {
    epoch: u64,
    sample: u64,
    reverse: bool,
    origin: usize,
}

impl Segment {
    fn word(self) -> u64 {
        self.epoch << 48 | u64::from(self.reverse) << 47 | self.sample << 27 | self.origin as u64
    }

    fn of(word: u64) -> Self {
        Self {
            epoch: word >> 48,
            reverse: word >> 47 & 1 == 1,
            sample: word >> 27 & IDLE,
            origin: (word & ((1 << 27) - 1)) as usize,
        }
    }

    /// Frame `f`'s index along the segment, `None` behind its origin.
    fn path(self, f: usize) -> Option<u64> {
        match self.reverse {
            false => f.checked_sub(self.origin),
            true => self.origin.checked_sub(f),
        }
        .map(|p| p as u64)
    }

    /// The frames along the segment of a sample of `frames`.
    fn len(self, frames: usize) -> u64 {
        match self.reverse {
            false => frames.saturating_sub(self.origin) as u64,
            true => (self.origin + 1).min(frames) as u64,
        }
    }
}

/// One voice's ring. The voice (the audio thread) writes `seg` and `need`; the filler writes `filled` and `data`.
struct Ring {
    seg: AtomicU64,
    /// The segment's epoch, and the first path index the voice still needs: the filler may overwrite what is before.
    need: AtomicU64,
    /// The segment's epoch, and the path index the ring holds up to (from `need` on).
    filled: AtomicU64,
    /// [`RING_FRAMES`] frames, left and right, as `f32` bits: path index `p` at `p % RING_FRAMES`.
    data: Box<[AtomicU32]>,
}

impl Ring {
    fn new() -> Self {
        Self {
            seg: AtomicU64::new(
                Segment {
                    epoch: 0,
                    sample: IDLE,
                    reverse: false,
                    origin: 0,
                }
                .word(),
            ),
            need: AtomicU64::new(0),
            filled: AtomicU64::new(0),
            data: (0..2 * RING_FRAMES).map(|_| AtomicU32::new(0)).collect(),
        }
    }
}

/// What fills one ring: the segment it fills, how far, and the open file.
#[derive(Default)]
struct Filler {
    epoch: Option<u64>,
    end: u64,
    cursor: Option<(u64, Option<Cursor>)>,
    left: Vec<f32>,
    right: Vec<f32>,
}

/// An instrument's streamed samples (by its own sample index) and a ring for each of its first [`STREAM_VOICES`] voices
/// ([`crate::stream`]).
pub struct Rings {
    samples: Box<[Option<Arc<Streamed>>]>,
    rings: Box<[Ring]>,
    fillers: Mutex<Box<[Filler]>>,
    /// A [`Streamer`] fills the rings; else each voice fills its own as it reads.
    live: AtomicBool,
    underruns: AtomicU64,
}

impl Rings {
    /// [`STREAM_VOICES`] rings over `samples` (`None` where one plays from memory); `None` when none streams. App
    /// thread: it allocates the rings.
    pub fn new(samples: Vec<Option<Arc<Streamed>>>) -> Option<Arc<Self>> {
        samples.iter().any(Option::is_some).then(|| {
            Arc::new(Self {
                samples: samples.into(),
                rings: (0..STREAM_VOICES).map(|_| Ring::new()).collect(),
                fillers: Mutex::new((0..STREAM_VOICES).map(|_| Filler::default()).collect()),
                live: AtomicBool::new(false),
                underruns: AtomicU64::new(0),
            })
        })
    }

    /// The voices it keeps a ring for: the first [`STREAM_VOICES`].
    pub fn voices(&self) -> usize {
        self.rings.len()
    }

    fn sample(&self, sample: u64) -> Option<&Streamed> {
        self.samples.get(sample as usize)?.as_deref()
    }

    /// Sample `sample` streams: a voice playing it needs a ring.
    pub fn streams(&self, sample: usize) -> bool {
        self.sample(sample as u64).is_some()
    }

    /// Voice `voice` reads `sample` (a streamed one, or any other: none) from frame `origin` on, backward with `reverse`.
    /// A voice past the rings reads none. Audio thread.
    pub fn start(&self, voice: usize, sample: usize, origin: usize, reverse: bool) {
        let Some(ring) = self.rings.get(voice) else {
            return;
        };
        let old = Segment::of(ring.seg.load(Ordering::Relaxed));
        let streamed = (sample < IDLE as usize)
            .then(|| self.sample(sample as u64))
            .flatten();
        let epoch = (old.epoch + 1) & 0xFFFF;
        let Some(streamed) = streamed else {
            if old.sample != IDLE {
                let idle = Segment {
                    epoch,
                    sample: IDLE,
                    reverse: false,
                    origin: 0,
                };
                ring.seg.store(idle.word(), Ordering::Relaxed);
                ring.need.store(pack(epoch, 0), Ordering::Release);
            }
            return;
        };
        let origin = origin.min(streamed.source.frames.saturating_sub(1));
        let seg = Segment {
            epoch,
            sample: sample as u64,
            reverse,
            origin,
        };
        // The ring starts where the head holding the origin ends.
        let skip = streamed.head(origin).map_or(0, |(a, b)| match reverse {
            false => b - origin,
            true => origin + 1 - a,
        });
        ring.seg.store(seg.word(), Ordering::Relaxed);
        ring.need.store(pack(epoch, skip as u64), Ordering::Release);
    }

    /// Voice `voice` reads nothing more: its ring is let go. Audio thread.
    pub fn stop(&self, voice: usize) {
        self.start(voice, IDLE as usize, 0, false);
    }

    /// Voice `voice` is at frame `at`: it needs nothing behind it (forward; ahead of the next frame, backward). Once a
    /// block, before it reads. Audio thread.
    pub fn advance(&self, voice: usize, at: usize) {
        let Some(ring) = self.rings.get(voice) else {
            return;
        };
        let seg = Segment::of(ring.seg.load(Ordering::Relaxed));
        let at = if seg.reverse { at + 1 } else { at };
        let (epoch, need) = unpack(ring.need.load(Ordering::Relaxed));
        if let Some(path) = seg
            .path(at)
            .filter(|&p| seg.sample != IDLE && epoch == seg.epoch && p > need)
        {
            ring.need.store(pack(epoch, path), Ordering::Release);
        }
    }

    /// Frame `f` of voice `voice`'s sample: from its heads or its ring; silence past the end. `None` when the ring does not
    /// hold it yet (an underrun). Audio thread: rings a streamer fills are only read; others are filled here first.
    pub fn frame(&self, voice: usize, f: usize) -> Option<(f32, f32)> {
        let ring = self.rings.get(voice)?;
        let seg = Segment::of(ring.seg.load(Ordering::Relaxed));
        let streamed = self.sample(seg.sample)?;
        if let Some(frame) = streamed.ram(f) {
            return Some(frame);
        }
        if f >= streamed.source.frames {
            return Some((0.0, 0.0));
        }
        let path = seg.path(f)?;
        loop {
            let (epoch, end) = unpack(ring.filled.load(Ordering::Acquire));
            let (_, need) = unpack(ring.need.load(Ordering::Relaxed));
            if epoch == seg.epoch && (need..end).contains(&path) {
                let at = 2 * (path as usize % RING_FRAMES);
                let bits = |i: usize| f32::from_bits(ring.data[i].load(Ordering::Relaxed));
                return Some((bits(at), bits(at + 1)));
            }
            if self.live.load(Ordering::Relaxed) || path < need {
                return None;
            }
            // An offline render: fill this ring now, from a frame before this one (an interpolation's other frame).
            ring.need.store(
                pack(seg.epoch, path.saturating_sub(1).max(need)),
                Ordering::Release,
            );
            let mut fillers = self.fillers.lock().unwrap_or_else(PoisonError::into_inner);
            if !self.fill_one(voice, &mut fillers[voice]) {
                let (epoch, end) = unpack(ring.filled.load(Ordering::Acquire));
                if epoch != seg.epoch || end <= path {
                    return None;
                }
            }
        }
    }

    /// Counts an underrun: a voice that read silence for lack of frames.
    pub fn underrun(&self) {
        self.underruns.fetch_add(1, Ordering::Relaxed);
    }

    /// The underruns so far.
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    /// Frames `from..to` of `sample` are in memory whole: a loop over them needs no new segment at its wrap.
    pub fn in_memory(&self, sample: usize, from: usize, to: usize) -> bool {
        self.sample(sample as u64)
            .is_none_or(|s| s.in_memory(from, to))
    }

    /// Fills each ring a chunk on: false when none wants more. The streamer's.
    fn fill(&self) -> bool {
        let mut fillers = self.fillers.lock().unwrap_or_else(PoisonError::into_inner);
        let mut more = false;
        for (voice, filler) in fillers.iter_mut().enumerate() {
            more |= self.fill_one(voice, filler);
        }
        more
    }

    /// Fills voice `voice`'s ring a chunk on; false when it wants no more now.
    fn fill_one(&self, voice: usize, filler: &mut Filler) -> bool {
        let ring = &self.rings[voice];
        let (epoch, need) = unpack(ring.need.load(Ordering::Acquire));
        let seg = Segment::of(ring.seg.load(Ordering::Relaxed));
        if seg.epoch != epoch {
            // The voice is starting a segment: the next pass.
            return false;
        }
        let Some(streamed) = self.sample(seg.sample) else {
            (filler.epoch, filler.cursor) = (None, None);
            return false;
        };
        if filler.epoch != Some(epoch) || need > filler.end {
            // A new segment, or a voice that ran past what was filled: start from what it needs.
            (filler.epoch, filler.end) = (Some(epoch), need);
            ring.filled.store(pack(epoch, need), Ordering::Release);
        }
        let limit = (need + RING_FRAMES as u64).min(seg.len(streamed.source.frames));
        if filler.end >= limit {
            return false;
        }
        let n = (CHUNK as u64).min(limit - filler.end) as usize;
        let first = match seg.reverse {
            false => seg.origin + filler.end as usize,
            true => seg.origin + 1 - filler.end as usize - n,
        };
        if filler.cursor.as_ref().is_none_or(|(s, _)| *s != seg.sample) {
            filler.cursor = Some((seg.sample, Cursor::open(&streamed.source)));
        }
        filler.left.resize(n, 0.0);
        filler.right.resize(n, 0.0);
        match filler.cursor.as_mut().and_then(|(_, c)| c.as_mut()) {
            Some(cursor) => {
                cursor.read(&streamed.source, first, &mut filler.left, &mut filler.right)
            }
            None => (filler.left.fill(0.0), filler.right.fill(0.0)).0,
        }
        for k in 0..n {
            let path = filler.end as usize + k;
            let from = if seg.reverse { n - 1 - k } else { k };
            let at = 2 * (path % RING_FRAMES);
            ring.data[at].store(filler.left[from].to_bits(), Ordering::Relaxed);
            ring.data[at + 1].store(filler.right[from].to_bits(), Ordering::Relaxed);
        }
        filler.end += n as u64;
        ring.filled
            .store(pack(epoch, filler.end), Ordering::Release);
        filler.end < limit
    }
}

/// The thread that fills the rings of an engine's instruments ([`Rings`]), while any handle to it lives.
#[derive(Clone)]
pub struct Streamer(Arc<Hub>);

struct Hub {
    rings: Mutex<Vec<Arc<Rings>>>,
    stalled: AtomicBool,
}

impl std::fmt::Debug for Streamer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Streamer")
    }
}

impl Streamer {
    /// Starts the thread. It ends once no handle is left.
    pub fn spawn() -> Self {
        let hub = Arc::new(Hub {
            rings: Mutex::new(Vec::new()),
            stalled: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&hub);
        let thread = std::thread::Builder::new().name("sample streamer".into());
        if let Err(err) = thread.spawn(move || run(weak)) {
            eprintln!("sample streamer: {err}");
        }
        Self(hub)
    }

    /// The streamer fills `rings` from now on, and their voices no longer fill them themselves. App thread.
    pub fn register(&self, rings: &Arc<Rings>) {
        rings.live.store(true, Ordering::Relaxed);
        let mut list = self.0.rings.lock().unwrap_or_else(PoisonError::into_inner);
        list.push(rings.clone());
    }

    /// A test hook: a stalled streamer fills nothing.
    pub fn stall(&self, stalled: bool) {
        self.0.stalled.store(stalled, Ordering::Relaxed);
    }
}

fn run(hub: Weak<Hub>) {
    loop {
        let Some(live) = hub.upgrade() else {
            return;
        };
        let rings = match live.stalled.load(Ordering::Relaxed) {
            true => Vec::new(),
            false => {
                let mut list = live.rings.lock().unwrap_or_else(PoisonError::into_inner);
                // An instrument that is gone holds its rings no longer.
                list.retain(|rings| Arc::strong_count(rings) > 1);
                list.clone()
            }
        };
        drop(live);
        while rings.iter().fold(false, |more, r| r.fill() | more) {}
        let pause = if rings.is_empty() { 20 } else { 2 };
        drop(rings);
        std::thread::sleep(Duration::from_millis(pause));
    }
}

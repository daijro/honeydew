//! DirectWave programs for the DW Sampler: the `.dwp` reader and the module's `CSTM` `10WD` blob. Bytes in, zones
//! out. Input is hostile: every length is checked, nothing
//! panics, and what is kept grows at most linearly with the input (embedded audio decodes under `MAX_FRAMES`).

use std::ops::Range;

use crate::sample::{MAX_FRAMES, SampleData, decode, decode_with_limit, float_sample};

/// Which keys and velocities a zone answers, and how it plays (0x1F4/0x1F7/0x1FD, `ZONE`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone {
    pub root: u8,
    pub key_low: u8,
    pub key_high: u8,
    pub vel_low: u8,
    pub vel_high: u8,
    /// Linear, applied only while Loop (p9) is On.
    pub gain: f32,
    /// Stored; playback never reads it.
    pub pan: f32,
    /// > 1 loops forward over `loop_start..loop_end` (frames); start = end takes the file's `smpl` loop.
    pub loop_mode: i32,
    pub loop_start: u32,
    pub loop_end: u32,
    /// The 0x1FD floats: #2 > 0 enables the Decay ramp (≤ 0 jumps to Sustain); #1 and #4 are a program's Attack and
    /// Release.
    pub floats: [f32; 4],
}

impl Default for Zone {
    /// Root 48, keys 0–128, velocities 0–127, gain 1, pan 0, no loop, floats 0, 1, 1, 0.1.
    fn default() -> Self {
        Self {
            root: 48,
            key_low: 0,
            key_high: 128,
            vel_low: 0,
            vel_high: 127,
            gain: 1.0,
            pan: 0.0,
            loop_mode: 0,
            loop_start: 0,
            loop_end: 0,
            floats: [0.0, 1.0, 1.0, 0.1],
        }
    }
}

impl Zone {
    /// A user sample: one zone over all keys, root 48 (the `smpl` unity note ignored), looping over the file's
    /// `smpl` loop when it has one.
    // Loop mode 2 with start = end, so the loop rule takes the file's `smpl` loop.
    pub fn user_sample() -> Self {
        Self {
            loop_mode: 2,
            ..Self::default()
        }
    }

    /// The Attack and Release a program load sets (value and default) when this is its last zone: floats #1 and #4.
    pub fn attack_release(&self) -> (f32, f32) {
        (self.floats[0], self.floats[3])
    }
}

/// 0x1F7: the embedded audio's shape.
#[derive(Clone, Copy, Debug)]
struct Format {
    frames: u64,
    channels: u32,
    rate: f32,
}

#[derive(Clone, Copy, Debug)]
enum Embedded<'a> {
    /// 0x205: interleaved PCM, 16-bit LE below 4 bytes per sample, else `f32`.
    Pcm(&'a [u8]),
    /// 0x206: a FLAC stream, with or without a leading `u64` length.
    Flac(&'a [u8]),
}

/// A `.dwp` zone as stored: its params, the file name its 0x1F6 path ends in, and any embedded audio (which wins
/// over the file).
#[derive(Clone, Debug)]
pub struct DwpZone<'a> {
    pub zone: Zone,
    pub file: Option<String>,
    audio: Option<Embedded<'a>>,
    format: Option<Format>,
    /// P/2 frames of embedded audio skipped at its start (an assumed meaning).
    lead_in: u32,
}

impl DwpZone<'_> {
    /// Plays embedded audio (0x205 or 0x206) rather than a file.
    pub fn embedded(&self) -> bool {
        self.audio.is_some()
    }

    /// The frames 0x1F7 says its audio holds, 0 when it says none: what decoding it takes.
    pub fn declared_frames(&self) -> u64 {
        self.format.map_or(0, |format| format.frames)
    }

    /// The embedded audio, decoded: the lead-in skipped, then at most 0x1F7's frames. `None` for a file zone or audio
    /// that does not decode. App thread.
    pub fn decode(&self) -> Option<SampleData> {
        self.decode_with_limit(MAX_FRAMES)
    }

    /// [`Self::decode`], `None` when the stream holds more than `max_frames` frames (lead-in included).
    pub fn decode_with_limit(&self, max_frames: usize) -> Option<SampleData> {
        let mut sample = match self.audio? {
            Embedded::Flac(bytes) => {
                let stream = if bytes.starts_with(b"fLaC") {
                    bytes
                } else {
                    bytes.get(8..)?
                };
                decode_with_limit(stream, max_frames).ok()?
            }
            Embedded::Pcm(bytes) => pcm(bytes, self.format?, self.lead_in, max_frames)?,
        };
        let start = (self.lead_in as usize).min(sample.frames());
        let end = self.format.map_or(sample.frames(), |f| {
            sample.frames().min(start.saturating_add(f.frames as usize))
        });
        if start == end {
            return None;
        }
        if (start, end) != (0, sample.frames()) {
            sample.left = sample.left[start..end].into();
            sample.right = sample.right.map(|right| right[start..end].into());
        }
        Some(sample)
    }
}

/// Interleaved PCM: the lead-in and 0x1F7's frames at most, `None` when that is more than `max_frames`.
fn pcm(bytes: &[u8], format: Format, lead_in: u32, max_frames: usize) -> Option<SampleData> {
    let channels = format.channels as usize;
    let wide = bytes.len() as u64 / (format.frames * channels as u64) >= 4;
    let width = if wide { 4 } else { 2 };
    let rate = format.rate.round();
    if !(1.0..=u32::MAX as f32).contains(&rate) {
        return None;
    }
    let wanted = u64::from(lead_in) + format.frames;
    let frames = bytes
        .chunks_exact(width * channels)
        .take(usize::try_from(wanted).unwrap_or(usize::MAX));
    if frames.len() > max_frames {
        return None;
    }
    let value = |b: &[u8]| {
        if wide {
            float_sample(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        } else {
            f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0
        }
    };
    let channel = |c: usize| -> std::sync::Arc<[f32]> {
        frames
            .clone()
            .map(|frame| value(&frame[c * width..]))
            .collect()
    };
    let left = channel(0);
    (!left.is_empty()).then(|| SampleData {
        sample_rate: rate as u32,
        right: (channels == 2).then(|| channel(1)),
        left,
        meta: Default::default(),
    })
}

fn word<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    bytes.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    word(bytes, at).map(u32::from_le_bytes)
}

/// An `f32` field, `None` when missing or not finite.
fn f32_at(bytes: &[u8], at: usize) -> Option<f32> {
    word(bytes, at)
        .map(f32::from_le_bytes)
        .filter(|v| v.is_finite())
}

/// `u32 tag, u64 len, data[len]` records; a length past the end is cut to what is there.
// An overlong record is clamped like a chunk.
fn records(mut bytes: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    std::iter::from_fn(move || {
        let tag = u32_at(bytes, 0)?;
        let len = u64::from_le_bytes(word(bytes, 4)?);
        let rest = &bytes[12..];
        let (data, next) =
            rest.split_at(usize::try_from(len).map_or(rest.len(), |len| len.min(rest.len())));
        bytes = next;
        Some((tag, data))
    })
}

/// The zones of a `.dwp` (`"DwPr"`, `u32` version, records): tag 1's tag-3 records, up to its tag 2. Tags 0x64 and
/// 0x66–0x6E are ignored. A zone whose 0x1F7 is bad, or that has neither a file name nor embedded audio, is left out.
/// `None` when the bytes are not a program.
// Any version is read (37 and 38 are known); the first tag-1 program counts.
pub fn read(bytes: &[u8]) -> Option<Vec<DwpZone<'_>>> {
    if !bytes.starts_with(b"DwPr") {
        return None;
    }
    let program = records(bytes.get(8..)?).find(|(tag, _)| *tag == 1);
    let records = program.into_iter().flat_map(|(_, data)| records(data));
    let zones = records
        .take_while(|(tag, _)| *tag != 2)
        .filter(|(tag, _)| *tag == 3);
    Some(zones.filter_map(|(_, data)| zone(data)).collect())
}

fn zone(data: &[u8]) -> Option<DwpZone<'_>> {
    let mut z = DwpZone {
        zone: Zone::default(),
        file: None,
        audio: None,
        format: None,
        lead_in: 0,
    };
    for (tag, d) in records(data) {
        match tag {
            0x1F4 => {
                let zone = &mut z.zone;
                for (field, at) in [
                    (&mut zone.root, 0),
                    (&mut zone.key_low, 1),
                    (&mut zone.key_high, 2),
                    (&mut zone.vel_low, 3),
                    (&mut zone.vel_high, 4),
                ] {
                    *field = d.get(at).copied().unwrap_or(*field);
                }
                zone.gain = f32_at(d, 9).unwrap_or(zone.gain);
                zone.pan = f32_at(d, 13).unwrap_or(zone.pan);
                z.lead_in = u32_at(d, 21).unwrap_or(0) / 2;
            }
            0x1F6 => {
                let path = d.split(|&b| b == 0).next().unwrap_or_default();
                let name = path
                    .rsplit(|&b| b == b'\\' || b == b'/')
                    .next()
                    .unwrap_or_default();
                z.file = Some(String::from_utf8_lossy(name).into_owned())
                    .filter(|name| !matches!(name.as_str(), "" | "." | ".."));
            }
            0x1F7 => {
                let format = Format {
                    frames: word(d, 0).map_or(0, u64::from_le_bytes),
                    channels: u32_at(d, 8).unwrap_or(0),
                    rate: f32_at(d, 16).unwrap_or(0.0),
                };
                if !(1..=u64::from(u32::MAX)).contains(&format.frames)
                    || !(1..=2).contains(&format.channels)
                {
                    return None;
                }
                z.format = Some(format);
                z.zone.loop_mode = word(d, 20).map_or(0, i32::from_le_bytes);
                z.zone.loop_start = u32_at(d, 24).unwrap_or(0);
                z.zone.loop_end = u32_at(d, 28).unwrap_or(0);
            }
            0x1FD => {
                for (i, value) in z.zone.floats.iter_mut().enumerate() {
                    *value = f32_at(d, 4 * i).unwrap_or(*value);
                }
            }
            0x205 => z.audio = Some(Embedded::Pcm(d)),
            0x206 => z.audio = Some(Embedded::Flac(d)),
            _ => {}
        }
    }
    (z.file.is_some() || z.audio.is_some()).then_some(z)
}

/// A `.dwp` that [`read`] takes back: `"DwPr"`, version 38, then tag 1 holding one tag-3 zone per entry (0x1F4, 0x1F6,
/// 0x1F7, 0x1FD) and tag 2. Each entry is the zone, its sample's file name, the sample's frames (≥ 1, or `read` drops
/// the zone) and whether it is stereo. The zones play files: no lead-in, no embedded audio, and 0x1F7 says 16-bit
/// 44100 Hz as factory files do (playback reads neither).
pub fn write(zones: &[(Zone, &str, u32, bool)]) -> Vec<u8> {
    let record = |tag: u32, data: &[u8]| {
        [
            &tag.to_le_bytes()[..],
            &(data.len() as u64).to_le_bytes(),
            data,
        ]
        .concat()
    };
    let mut program = Vec::new();
    for (z, file, frames, stereo) in zones {
        // Root, keys and velocities, the skipped and ignored bytes, gain, pan, the ignored word and P = 0.
        let keys = [
            &[
                z.root, z.key_low, z.key_high, z.vel_low, z.vel_high, 0, 0, 0, 0,
            ][..],
            &z.gain.to_le_bytes(),
            &z.pan.to_le_bytes(),
            &[0; 8],
        ]
        .concat();
        let format = [
            &u64::from(*frames).to_le_bytes()[..],
            &(1 + u32::from(*stereo)).to_le_bytes(),
            &16u32.to_le_bytes(),
            &44_100f32.to_le_bytes(),
            &z.loop_mode.to_le_bytes(),
            &z.loop_start.to_le_bytes(),
            &z.loop_end.to_le_bytes(),
            &[0; 8],
        ]
        .concat();
        let floats: Vec<u8> = z.floats.iter().flat_map(|f| f.to_le_bytes()).collect();
        let zone = [
            record(0x1F4, &keys),
            record(0x1F6, file.as_bytes()),
            record(0x1F7, &format),
            record(0x1FD, &floats),
        ]
        .concat();
        program.extend(record(3, &zone));
    }
    program.extend(record(2, &[]));
    [&b"DwPr"[..], &38u32.to_le_bytes(), &record(1, &program)].concat()
}

/// The `CSTM` `MIDI` settings: pitch-bend range in semitones (held to 1…12), and the flag that makes Attack at v = 0 take 1 ms
/// (≠ 0) instead of being instant (0). `Default` is a new module's 2 and 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Midi {
    pub pb_range: u8,
    pub attack_flag: u8,
}

impl Default for Midi {
    fn default() -> Self {
        Self {
            pb_range: 2,
            attack_flag: 0,
        }
    }
}

/// `PATh` (or legacy `PATH`): the program file to load zones from, through [`load_program`]'s loader.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgramRef<'a> {
    /// The shorthand path as stored.
    pub path: &'a [u8],
    /// The init values of Attack (p1) and Release (p2): applied after the program loads, replacing both the init and
    /// the current values. `None` unless both are there (≥ 8 bytes after the path, both finite); a legacy
    /// `PATH` has none.
    pub init: Option<(f32, f32)>,
    /// The whole chunk, header included, in the blob: what a save rewrites with the init values.
    pub chunk: Range<usize>,
}

/// The DW Sampler's program loader on a file's `bytes` (the same for a `PATh` and a pick): `None` when it
/// rejects them, else the Attack (p1) and Release (p2) it sets. In order it tries the old instrument format, then a
/// `.dwp`, which sets its last zone's floats #1 and #4 when it has zones, then a plain audio file, which sets nothing.
// The old instrument format's layout is unknown, so this loader never takes it.
pub fn load_program(bytes: &[u8]) -> Option<Option<(f32, f32)>> {
    match read(bytes) {
        Some(zones) => Some(zones.last().map(|zone| zone.zone.attack_release())),
        None => decode(bytes).ok().map(|_| None),
    }
}

/// The DW Sampler's `CSTM` as playback reads it: `"10WD"` + chunks. Its zones come from `program` or from
/// `zones`, never both.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cstm<'a> {
    pub program: Option<ProgramRef<'a>>,
    /// `ZONE` records: the `char[1024]` shorthand path as stored, and the zone.
    pub zones: Vec<(&'a [u8], Zone)>,
    pub midi: Midi,
}

impl<'a> Cstm<'a> {
    /// The blob's settings and zones. A blob without the `10WD` magic (or none at all, `&[]`) is a new module's.
    /// Otherwise the module is reset first (no zones, no path, MIDI 4 and 1), then the chunks load in order: a `MIDI`
    /// overrides the settings (a 1-byte one only the range); a `PATh` or legacy `PATH` is ignored once zones or a path
    /// are loaded, and a `ZONE` once a path is. A short `ZONE` (the loader rejects the song) is
    /// skipped, as is a path that is empty or cut short.
    pub fn read(blob: &'a [u8]) -> Self {
        let Some(mut chunks) = blob.strip_prefix(b"10WD") else {
            return Self::default();
        };
        let mut cstm = Self {
            midi: Midi {
                pb_range: 4,
                attack_flag: 1,
            },
            ..Self::default()
        };
        while let (Some(id), Some(size)) = (word::<4>(chunks, 0), u32_at(chunks, 4)) {
            let start = blob.len() - chunks.len();
            let rest = &chunks[8..];
            let (body, next) = rest.split_at((size as usize).min(rest.len()));
            chunks = next;
            // A path counts only while no zones and no path are loaded; a ZONE only while no path is.
            let free = cstm.program.is_none();
            match &id {
                b"PATh" | b"PATH" if free && cstm.zones.is_empty() => {
                    let len = u32_at(body, 0).map(|len| len as usize);
                    // An empty path loads nothing, as one cut short does.
                    if let Some(path) = len.and_then(|len| body.get(4..4usize.checked_add(len)?))
                        && !path.is_empty()
                    {
                        let at = 4 + path.len();
                        let floats = f32_at(body, at).zip(f32_at(body, at + 4));
                        cstm.program = Some(ProgramRef {
                            path,
                            init: floats.filter(|_| id == *b"PATh"),
                            chunk: start..blob.len() - chunks.len(),
                        });
                    }
                }
                b"ZONE" if free && body.len() >= 1065 => {
                    cstm.zones.push((&body[..1024], cstm_zone(&body[1024..])))
                }
                b"MIDI" => {
                    // Held to 1…12 like a live edit (`panels::cstm`), so a file sounds the same before and after one.
                    let range = body.first().map(|&range| range.clamp(1, 12));
                    cstm.midi.pb_range = range.unwrap_or(cstm.midi.pb_range);
                    cstm.midi.attack_flag = body.get(1).copied().unwrap_or(cstm.midi.attack_flag);
                }
                _ => {}
            }
        }
        cstm
    }
}

/// A `ZONE` after its path: root, key and velocity range, gain, pan, loop mode, start, end, then the four floats.
fn cstm_zone(d: &[u8]) -> Zone {
    let default = Zone::default();
    let float = |at: usize, or: f32| f32_at(d, at).unwrap_or(or);
    Zone {
        root: d[0],
        key_low: d[1],
        key_high: d[2],
        vel_low: d[3],
        vel_high: d[4],
        gain: float(5, default.gain),
        pan: float(9, 0.5),
        loop_mode: word(d, 13).map_or(0, i32::from_le_bytes),
        loop_start: u32_at(d, 17).unwrap_or(0),
        loop_end: u32_at(d, 21).unwrap_or(0),
        floats: std::array::from_fn(|i| float(25 + 4 * i, default.floats[i])),
    }
}

//! Standard MIDI file bytes: a reader for the files users open, which never panics and
//! allocates no more than the file and [`MAX_EVENTS`] allow, and a plain format-1 writer.

use crate::midi::MidiMsg;
use crate::midi::message::data_len;
use crate::project_files::MAX_FILE_BYTES;

/// More `MTrk` chunks than this fail to read; the export writes no more.
pub const MAX_TRACKS: usize = 256;
/// More events than this, over all tracks, fail to read; the export writes no more.
pub const MAX_EVENTS: usize = 2_000_000;
/// The largest delta time: a four-byte variable-length quantity.
pub const MAX_DELTA: u64 = 0x0FFF_FFFF;

/// One track event. The reader skips SysEx (`F0`, `F7`) and poly pressure (`An`), which [`MidiMsg`] lacks.
#[derive(Clone, Debug, PartialEq)]
pub enum SmfEvent {
    Voice(MidiMsg),
    /// `FF 03` (track name) or `FF 04` (instrument name), its bytes undecoded. The writer writes `FF 04`.
    Name(Vec<u8>),
    /// `FF 51 03`: µs per quarter note.
    Tempo(u32),
    /// Any other meta, `FF 51` with a length other than 3 included. `FF 2F` ends a track and is not kept.
    Other {
        kind: u8,
        data: Vec<u8>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Smf {
    /// Ticks per quarter note, ≥ 1.
    pub division: u16,
    /// The `MTrk` chunks in file order, each `(tick from the track start, event)` in time order.
    pub tracks: Vec<Vec<(u64, SmfEvent)>>,
}

const NOT_SMF: &str = "not a MIDI file";
const TRUNCATED: &str = "truncated";
const BAD_EVENT: &str = "invalid event";
const TOO_LARGE: &str = "too large";

/// Reads a file of format 0, 1 or 2 alike. A division below 1 (SMPTE included) reads as 96. Chunks other than `MTrk`
/// are skipped, and so are fewer than 8 bytes after the last chunk.
pub fn read(bytes: &[u8]) -> Result<Smf, &'static str> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(TOO_LARGE);
    }
    let mut rest = bytes;
    let (id, header) = chunk(&mut rest)?;
    if id != b"MThd" || header.len() < 6 {
        return Err(NOT_SMF);
    }
    let division = i16::from_be_bytes([header[4], header[5]]);
    let division = if division < 1 { 96 } else { division as u16 };
    let (mut tracks, mut events) = (Vec::new(), 0);
    while rest.len() >= 8 {
        let (id, body) = chunk(&mut rest)?;
        if id != b"MTrk" {
            continue;
        }
        if tracks.len() == MAX_TRACKS {
            return Err(TOO_LARGE);
        }
        tracks.push(track(body, &mut events)?);
    }
    Ok(Smf { division, tracks })
}

/// The next chunk's id and body.
fn chunk<'a>(rest: &mut &'a [u8]) -> Result<(&'a [u8], &'a [u8]), &'static str> {
    let id = take(rest, 4)?;
    let len = u32::from_be_bytes(take(rest, 4)?.try_into().expect("4 bytes"));
    Ok((id, take(rest, len as usize)?))
}

fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8], &'static str> {
    if n > b.len() {
        return Err(TRUNCATED);
    }
    let (head, tail) = b.split_at(n);
    *b = tail;
    Ok(head)
}

fn byte(b: &mut &[u8]) -> Result<u8, &'static str> {
    Ok(take(b, 1)?[0])
}

/// A variable-length quantity of at most four bytes.
fn vlq(b: &mut &[u8]) -> Result<u64, &'static str> {
    let mut v = 0;
    for _ in 0..4 {
        let x = byte(b)?;
        v = v << 7 | u64::from(x & 0x7F);
        if x < 0x80 {
            return Ok(v);
        }
    }
    Err(BAD_EVENT)
}

/// One `MTrk` body, adding its events to `count`. Running status survives metas and SysEx (lenient).
fn track(mut b: &[u8], count: &mut usize) -> Result<Vec<(u64, SmfEvent)>, &'static str> {
    let (mut out, mut tick, mut running) = (Vec::new(), 0, None);
    while !b.is_empty() {
        tick += vlq(&mut b)?;
        let first = byte(&mut b)?;
        let event = match first {
            0xFF => {
                let kind = byte(&mut b)?;
                let len = vlq(&mut b)? as usize;
                let data = take(&mut b, len)?;
                match (kind, data) {
                    (0x2F, _) => break,
                    (0x03 | 0x04, _) => SmfEvent::Name(data.to_vec()),
                    (0x51, &[a, b, c]) => SmfEvent::Tempo(u32::from_be_bytes([0, a, b, c])),
                    _ => SmfEvent::Other {
                        kind,
                        data: data.to_vec(),
                    },
                }
            }
            0xF0 | 0xF7 => {
                let len = vlq(&mut b)? as usize;
                take(&mut b, len)?;
                continue;
            }
            _ => {
                // A data byte first: running status, and it is the first data byte.
                let status = if first >= 0x80 {
                    first
                } else {
                    running.ok_or(BAD_EVENT)?
                };
                let n = data_len(status).ok_or(BAD_EVENT)?;
                running = Some(status);
                let mut msg = [status, first, 0];
                let have = usize::from(first < 0x80);
                msg[1 + have..=n].copy_from_slice(take(&mut b, n - have)?);
                match MidiMsg::parse(&msg[..=n]) {
                    Some(m) => SmfEvent::Voice(m),
                    None if status >> 4 == 0xA && msg[1..=n].iter().all(|&d| d < 0x80) => continue,
                    None => return Err(BAD_EVENT),
                }
            }
        };
        *count += 1;
        if *count > MAX_EVENTS {
            return Err(TOO_LARGE);
        }
        out.push((tick, event));
    }
    Ok(out)
}

/// Format 1: delta times, no running status, and `00 FF 2F 00` after each track's events. Ticks must not go back
/// (a tick before the last one is written at it), and a gap over [`MAX_DELTA`] is cut to it. Data bytes are masked
/// to 7 bits.
pub fn write(smf: &Smf) -> Vec<u8> {
    let mut out = b"MThd\0\0\0\x06\0\x01".to_vec();
    out.extend((smf.tracks.len().min(usize::from(u16::MAX)) as u16).to_be_bytes());
    out.extend(smf.division.to_be_bytes());
    for track in &smf.tracks {
        let mut body = Vec::new();
        let mut last = 0;
        for (tick, event) in track {
            let delta = tick.saturating_sub(last).min(MAX_DELTA);
            last += delta;
            put_vlq(&mut body, delta);
            match event {
                SmfEvent::Voice(msg) => put_voice(&mut body, *msg),
                SmfEvent::Name(text) => put_meta(&mut body, 0x04, text),
                SmfEvent::Tempo(us) => put_meta(&mut body, 0x51, &us.to_be_bytes()[1..]),
                SmfEvent::Other { kind, data } => put_meta(&mut body, *kind, data),
            }
        }
        body.extend([0x00, 0xFF, 0x2F, 0x00]);
        out.extend(b"MTrk");
        out.extend((body.len() as u32).to_be_bytes());
        out.extend(body);
    }
    out
}

fn put_vlq(out: &mut Vec<u8>, v: u64) {
    let mut shift = (1..10)
        .rev()
        .find(|&g| v >> (7 * g) != 0)
        .map_or(0, |g| 7 * g);
    while shift > 0 {
        out.push(0x80 | (v >> shift) as u8 & 0x7F);
        shift -= 7;
    }
    out.push(v as u8 & 0x7F);
}

fn put_meta(out: &mut Vec<u8>, kind: u8, data: &[u8]) {
    out.extend([0xFF, kind]);
    put_vlq(out, data.len() as u64);
    out.extend(data);
}

fn put_voice(out: &mut Vec<u8>, msg: MidiMsg) {
    let (kind, ch, data) = match msg {
        MidiMsg::NoteOff { ch, key, vel } => (0x80, ch, [key, vel]),
        MidiMsg::NoteOn { ch, key, vel } => (0x90, ch, [key, vel]),
        MidiMsg::Cc { ch, cc, value } => (0xB0, ch, [cc, value]),
        MidiMsg::Program { ch, program } => (0xC0, ch, [program, 0]),
        MidiMsg::ChannelPressure { ch, value } => (0xD0, ch, [value, 0]),
        MidiMsg::PitchBend { ch, value } => (0xE0, ch, [value as u8, (value >> 7) as u8]),
    };
    out.push(kind | ch & 0x0F);
    let n = data_len(kind).expect("a channel-voice status");
    out.extend(data[..n].iter().map(|d| d & 0x7F));
}

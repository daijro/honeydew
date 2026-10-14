use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::f64::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use honeydew_audio::sample::{
    DecodeError, MAX_FRAMES, SampleData, SampleMeta, decode, decode_with_limit,
};

/// A FLAC stream under a `.wav` name, as 3612 of the bundled samples are. Project-made:
/// `ffmpeg -f lavfi -i "aevalsrc=0.5*sin(2*PI*441*t)|0.5*sin(2*PI*882*t):s=44100:d=0.1" -c:a pcm_s16le -bitexact`
/// then the reference encoder, `flac -8 --no-padding --no-seektable` (libFLAC 1.5.0). 4410 frames, two FLAC frames.
const FLAC_SINE: &[u8] = include_bytes!("fixtures/sine_flac.wav");

/// The same tone, `lame --silent -b 128 --noreplaygain` (LAME 4.0) on the `-bitexact` WAV: an Info frame whose LAME
/// tag gives 1105 frames of delay and 245 of padding, then five audio frames (`MP3_SINE_FRAMES`).
const MP3_SINE: &[u8] = include_bytes!("fixtures/sine.mp3");
const MP3_SINE_FRAMES: [usize; 6] = [0, 417, 834, 1252, 1670, 2088];

/// `aevalsrc=0.5*sin(2*PI*441*t):s=22050:d=0.1` mono, `lame --silent -b 32 -m m --noreplaygain`: MPEG-2 frames too
/// small for a LAME tag, so nothing is trimmed.
const MP3_MONO_22K: &[u8] = include_bytes!("fixtures/sine_mono_22k.mp3");

/// The stereo tone by ffmpeg's libvorbis, `-q:a 2 -map_metadata -1 -fflags +bitexact -flags:a +bitexact
/// -page_duration 20000`: an ident page, a comment and setup page, then four audio pages ending at granule 4410
/// (`VORBIS_SINE_PAGES`).
const VORBIS_SINE: &[u8] = include_bytes!("fixtures/sine_vorbis.ogg");
const VORBIS_SINE_PAGES: [usize; 7] = [0, 58, 3945, 4267, 4374, 4524, 4670];

/// The stereo tone by `flac -s --ogg -8 --no-padding --no-seektable --serial-number=1`: Ogg FLAC with a
/// VORBIS_COMMENT header packet on its second page (79..151).
const OGG_FLAC_SINE: &[u8] = include_bytes!("fixtures/sine_flac.oga");

/// Records the largest single allocation in this test binary. On Linux a hostile 4 GiB `vec![0; n]` succeeds
/// lazily, so only its size shows it (on wasm it aborts). Per thread it also counts allocations and tracks live bytes,
/// for `measure`.
struct LargestAllocation;

static LARGEST_ALLOCATION: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy)]
struct ThreadAllocations {
    count: usize,
    live: isize,
    peak: isize,
    largest: usize,
}

thread_local! {
    static THREAD_ALLOCATIONS: Cell<ThreadAllocations> = const {
        Cell::new(ThreadAllocations { count: 0, live: 0, peak: 0, largest: 0 })
    };
}

/// Books `size` bytes (negative when freed) against this thread; `new` is the size of a new block, if one was made.
fn track(size: isize, new: Option<usize>) {
    LARGEST_ALLOCATION.fetch_max(new.unwrap_or(0), Ordering::Relaxed);
    let _ = THREAD_ALLOCATIONS.try_with(|stats| {
        let mut s = stats.get();
        if let Some(new) = new {
            s.count += 1;
            s.largest = s.largest.max(new);
        }
        s.live += size;
        s.peak = s.peak.max(s.live);
        stats.set(s);
    });
}

unsafe impl GlobalAlloc for LargestAllocation {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        track(layout.size() as isize, Some(layout.size()));
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        track(layout.size() as isize, Some(layout.size()));
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        track(new_size as isize - layout.size() as isize, Some(new_size));
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        track(-(layout.size() as isize), None);
        unsafe { System.dealloc(ptr, layout) }
    }
}

/// What `f` allocated on this thread: the number of allocations, the peak of live bytes above where it started, and the
/// largest single allocation.
fn measure<T>(f: impl FnOnce() -> T) -> (T, usize, usize, usize) {
    let before = THREAD_ALLOCATIONS.with(Cell::get);
    THREAD_ALLOCATIONS.with(|stats| {
        stats.set(ThreadAllocations {
            peak: before.live,
            largest: 0,
            ..before
        })
    });
    let result = f();
    let after = THREAD_ALLOCATIONS.with(Cell::get);
    let peak = (after.peak - before.live).max(0) as usize;
    (result, after.count - before.count, peak, after.largest)
}

#[global_allocator]
static ALLOCATOR: LargestAllocation = LargestAllocation;

/// The fixture's two FLAC frames: start, end (after the CRC-16) and header length (with the CRC-8).
const FLAC_SINE_FRAMES: [(usize, usize, usize); 2] = [(86, 3135, 6), (3135, 3477, 8)];

fn crc8(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |crc, &b| {
        (0..8).fold(crc ^ b, |c, _| {
            if c & 0x80 != 0 {
                (c << 1) ^ 0x07
            } else {
                c << 1
            }
        })
    })
}

fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0, |crc, &b| {
        (0..8).fold(crc ^ (u16::from(b) << 8), |c, _| {
            if c & 0x8000 != 0 {
                (c << 1) ^ 0x8005
            } else {
                c << 1
            }
        })
    })
}

/// A STREAMINFO block for 44.1 kHz mono 16-bit with fixed `block`-sample blocks.
fn streaminfo(last: bool, block: u16) -> Vec<u8> {
    let mut out = vec![if last { 0x80 } else { 0 }, 0, 0, 34];
    out.extend(block.to_be_bytes());
    out.extend(block.to_be_bytes());
    out.extend([0; 6]);
    out.extend(((44_100u64 << 44) | (15 << 36)).to_be_bytes());
    out.extend([0; 16]);
    out
}

/// A CRC-valid mono frame: fixed blocking, frame number `n` (< 128), an 8-bit block size and `sub` as the subframe.
fn flac_frame(n: u8, block: u8, sub: &[u8]) -> Vec<u8> {
    let mut frame = vec![0xFF, 0xF8, 0x60, 0x00, n, block - 1];
    frame.push(crc8(&frame));
    frame.extend(sub);
    frame.extend(crc16(&frame).to_be_bytes());
    frame
}

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
    out
}

fn fmt(tag: u16, channels: u16, rate: u32, bits: u16) -> Vec<u8> {
    let align = channels * bits.div_ceil(8);
    let mut body = tag.to_le_bytes().to_vec();
    body.extend(channels.to_le_bytes());
    body.extend(rate.to_le_bytes());
    body.extend((rate * u32::from(align)).to_le_bytes());
    body.extend(align.to_le_bytes());
    body.extend(bits.to_le_bytes());
    chunk(b"fmt ", &body)
}

/// `WAVE_FORMAT_EXTENSIBLE`: the real format tag is the first two bytes of the sub-format GUID.
fn fmt_extensible(sub_tag: u16, channels: u16, rate: u32, bits: u16) -> Vec<u8> {
    let mut body = fmt(0xFFFE, channels, rate, bits)[8..].to_vec();
    body.extend(22u16.to_le_bytes());
    body.extend(bits.to_le_bytes());
    body.extend(3u32.to_le_bytes());
    body.extend(sub_tag.to_le_bytes());
    body.extend([
        0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
    ]);
    chunk(b"fmt ", &body)
}

fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = chunks.concat();
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32 + 4).to_le_bytes());
    out.extend(b"WAVE");
    out.extend(body);
    out
}

fn wav(fmt_chunk: Vec<u8>, data: &[u8]) -> Vec<u8> {
    riff(&[fmt_chunk, chunk(b"data", data)])
}

fn smpl(unity_note: u32, first_loop: Option<(u32, u32)>) -> Vec<u8> {
    let mut body = vec![0; 36];
    body[12..16].copy_from_slice(&unity_note.to_le_bytes());
    if let Some((start, end)) = first_loop {
        body[28..32].copy_from_slice(&1u32.to_le_bytes());
        body.extend([0; 8]);
        body.extend(start.to_le_bytes());
        body.extend(end.to_le_bytes());
        body.extend([0; 8]);
    }
    chunk(b"smpl", &body)
}

fn acid(beats: u32, tempo: f32) -> Vec<u8> {
    let mut body = vec![0; 24];
    body[12..16].copy_from_slice(&beats.to_le_bytes());
    body[20..24].copy_from_slice(&tempo.to_le_bytes());
    chunk(b"acid", &body)
}

fn cue(positions: &[u32]) -> Vec<u8> {
    let mut body = (positions.len() as u32).to_le_bytes().to_vec();
    for (id, position) in positions.iter().enumerate() {
        body.extend((id as u32).to_le_bytes());
        body.extend(position.to_le_bytes());
        body.extend(b"data");
        body.extend([0; 8]);
        body.extend((position + 1_000_000).to_le_bytes()); // dwSampleOffset, not the marker
    }
    chunk(b"cue ", &body)
}

fn mono(sample: &SampleData) -> &[f32] {
    assert!(sample.right.is_none(), "expected mono");
    &sample.left
}

fn stereo(sample: &SampleData) -> (&[f32], &[f32]) {
    (
        &sample.left,
        sample.right.as_deref().expect("expected stereo"),
    )
}

#[test]
fn pcm8_mono_is_unsigned() {
    let sample = decode(&wav(fmt(1, 1, 22_050, 8), &[128, 255, 0, 192])).unwrap();
    assert_eq!(sample.sample_rate, 22_050);
    assert_eq!(mono(&sample), [0.0, 127.0 / 128.0, -1.0, 0.5]);
}

#[test]
fn pcm16_stereo() {
    let frames: [[i16; 2]; 3] = [[0, 32767], [16384, -16384], [-32768, 0]];
    let data: Vec<u8> = frames
        .iter()
        .flatten()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    let sample = decode(&wav(fmt(1, 2, 44_100, 16), &data)).unwrap();
    assert_eq!(sample.sample_rate, 44_100);
    assert_eq!(sample.frames(), 3);
    let (left, right) = stereo(&sample);
    assert_eq!(left, [0.0, 0.5, -1.0]);
    assert_eq!(right, [32767.0 / 32768.0, -0.5, 0.0]);
}

#[test]
fn pcm24_mono() {
    let data = [
        0x00, 0x00, 0x40, 0x00, 0x00, 0x80, 0xFF, 0xFF, 0x7F, 0x00, 0x00, 0xC0,
    ];
    let sample = decode(&wav(fmt(1, 1, 48_000, 24), &data)).unwrap();
    assert_eq!(mono(&sample), [0.5, -1.0, 8_388_607.0 / 8_388_608.0, -0.5]);
}

#[test]
fn pcm24_stereo() {
    let data = [
        0x00, 0x00, 0x40, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x80, 0x00, 0x00, 0x20,
    ];
    let sample = decode(&wav(fmt(1, 2, 48_000, 24), &data)).unwrap();
    let (left, right) = stereo(&sample);
    assert_eq!(left, [0.5, -1.0]);
    assert_eq!(right, [-0.5, 0.25]);
    // Deviation (2026-10-14): WAVE_FORMAT_EXTENSIBLE with the PCM sub-format reads as tag 1.
    assert_eq!(
        decode(&wav(fmt_extensible(1, 2, 48_000, 24), &data)),
        Ok(sample)
    );
}

#[test]
fn pcm32_int_stereo() {
    let frames: [[i32; 2]; 2] = [[i32::MIN, 1 << 30], [-(1 << 29), 0]];
    let data: Vec<u8> = frames
        .iter()
        .flatten()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    let sample = decode(&wav(fmt(1, 2, 96_000, 32), &data)).unwrap();
    let (left, right) = stereo(&sample);
    assert_eq!(left, [-1.0, -0.25]);
    assert_eq!(right, [0.5, 0.0]);
}

#[test]
fn float32_mono_and_stereo_with_non_finite_values_silenced() {
    let values = [0.25f32, -1.5, f32::NAN, f32::INFINITY];
    let data: Vec<u8> = values.iter().flat_map(|s| s.to_le_bytes()).collect();
    let sample = decode(&wav(fmt(3, 1, 44_100, 32), &data)).unwrap();
    assert_eq!(mono(&sample), [0.25, -1.5, 0.0, 0.0]);
    let sample = decode(&wav(fmt(3, 2, 44_100, 32), &data)).unwrap();
    let (left, right) = stereo(&sample);
    assert_eq!(left, [0.25, 0.0]);
    assert_eq!(right, [-1.5, 0.0]);
}

#[test]
fn huge_finite_floats_are_held_to_16() {
    // 1.6e38 times a DW zone gain of 16 would overflow to inf and poison every filter after it.
    let values = [1.6e38f32, -1e30, 16.5, -2.0];
    let data: Vec<u8> = values.iter().flat_map(|s| s.to_le_bytes()).collect();
    let sample = decode(&wav(fmt(3, 1, 44_100, 32), &data)).unwrap();
    assert_eq!(mono(&sample), [16.0, -16.0, 16.0, -2.0]);
}

#[test]
fn smpl_acid_and_cue_chunks_are_read_as_stored() {
    // 4000 frames: the loop (100, 4000) fits, so it is as stored.
    let data = [0u8; 8000];
    let bytes = riff(&[
        fmt(1, 1, 44_100, 16),
        smpl(57, Some((100, 4000))),
        acid(8, 128.5),
        cue(&[0, 2205, 6615]),
        smpl(1, Some((1, 2))),
        acid(1, 60.0),
        cue(&[1]),
    ]);
    let bytes = [bytes, chunk(b"data", &data)].concat();
    let meta = decode(&bytes).unwrap().meta;
    assert_eq!(
        meta,
        SampleMeta {
            unity_note: Some(57),
            loop_frames: Some((100, 4000)),
            beats: Some(8),
            tempo: Some(128.5),
            cues: vec![0, 2205, 6615],
        }
    );
    let bytes = riff(&[
        fmt(1, 1, 44_100, 16),
        smpl(60, None),
        acid(4, f32::NAN),
        chunk(b"data", &data),
    ]);
    let meta = decode(&bytes).unwrap().meta;
    assert_eq!(
        (meta.unity_note, meta.loop_frames, meta.beats, meta.tempo),
        (Some(60), None, Some(4), None)
    );
    let mut lying = cue(&[7]);
    lying[8..12].copy_from_slice(&1000u32.to_le_bytes()); // 1000 points claimed, one present
    let bytes = riff(&[fmt(1, 1, 44_100, 16), lying, chunk(b"data", &data)]);
    assert_eq!(decode(&bytes).unwrap().meta.cues, [7]);
    assert_eq!(decode(FLAC_SINE).unwrap().meta, SampleMeta::default());
}

/// A `cue ` chunk is a u32 count, then the points. Points stored past the count are not markers.
#[test]
fn cue_points_past_the_count_are_ignored() {
    let mut short = cue(&[7, 9]);
    short[8..12].copy_from_slice(&1u32.to_le_bytes()); // count 1, two points stored
    let bytes = riff(&[fmt(1, 1, 44_100, 16), short, chunk(b"data", &[0; 40])]);
    assert_eq!(decode(&bytes).unwrap().meta.cues, [7]);
}

/// The `smpl` loop is read raw and fitted to the frame count: a start at or past it becomes 0, an
/// end past it becomes the frame count, and nothing is added to the end.
#[test]
fn smpl_loop_is_fitted_to_the_frames_without_a_plus_one() {
    let data = [0u8; 200];
    // 100 frames of mono 16-bit.
    let mono16 = |first_loop| {
        let chunks = [
            fmt(1, 1, 44_100, 16),
            smpl(60, first_loop),
            chunk(b"data", &data),
        ];
        decode(&riff(&chunks)).unwrap().meta
    };
    for (stored, want) in [
        ((10, 90), (10, 90)),
        ((0, 100), (0, 100)),
        ((99, 100), (99, 100)),
        ((0, 101), (0, 100)),
        ((100, 100), (0, 100)),
        ((100, 50), (0, 50)),
        ((5000, 70), (0, 70)),
        ((5000, 9000), (0, 100)),
        ((u32::MAX, u32::MAX), (0, 100)),
        ((50, 10), (50, 10)),
        ((0, 0), (0, 0)),
    ] {
        let meta = mono16(Some(stored));
        assert_eq!(meta.loop_frames, Some(want), "stored {stored:?}");
        assert_eq!(meta.unity_note, Some(60));
    }
    assert_eq!(mono16(None).loop_frames, None);
    // Frames, not samples or bytes: 200 bytes of stereo 16-bit are 50 frames, and of 8-bit mono 200.
    let loop_of = |fmt_chunk, first_loop| {
        let chunks = [fmt_chunk, smpl(60, first_loop), chunk(b"data", &data)];
        decode(&riff(&chunks)).unwrap().meta.loop_frames
    };
    assert_eq!(
        loop_of(fmt(1, 2, 44_100, 16), Some((10, 60))),
        Some((10, 50))
    );
    assert_eq!(
        loop_of(fmt(1, 1, 44_100, 8), Some((150, 300))),
        Some((150, 200))
    );
    // A data chunk cut short by the end of the file has the frames that are there.
    let chunks = [
        fmt(1, 1, 44_100, 16),
        smpl(60, Some((10, 90))),
        chunk(b"data", &data),
    ];
    let mut cut = riff(&chunks);
    cut.truncate(cut.len() - 100);
    assert_eq!(decode(&cut).unwrap().meta.loop_frames, Some((10, 50)));
}

#[test]
fn odd_chunks_are_padded_and_unknown_chunks_skipped() {
    let data: Vec<u8> = [8192i16, -8192]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    let bytes = riff(&[
        chunk(b"JUNK", b"abc"),
        fmt(1, 1, 44_100, 16),
        chunk(b"LIST", b"x"),
        chunk(b"data", &data),
    ]);
    assert_eq!(mono(&decode(&bytes).unwrap()), [0.25, -0.25]);
}

#[test]
fn a_truncated_data_chunk_keeps_the_whole_frames_present() {
    let data: Vec<u8> = [1i16, 2, 3, 4, 5]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    let mut bytes = wav(fmt(1, 2, 44_100, 16), &data);
    let data_size_at = bytes.len() - data.len() - 4;
    bytes[data_size_at..data_size_at + 4].copy_from_slice(&4000u32.to_le_bytes());
    let sample = decode(&bytes).unwrap();
    assert_eq!(
        sample.frames(),
        2,
        "five 16-bit samples are two stereo frames and half of one"
    );
}

#[test]
fn more_than_two_channels_keep_the_first_two() {
    let data: Vec<u8> = [1i16, 2, 3, 4, 5, 6, 7, 8]
        .iter()
        .flat_map(|s| (s * 2048).to_le_bytes())
        .collect();
    let sample = decode(&wav(fmt(1, 4, 44_100, 16), &data)).unwrap();
    let (left, right) = stereo(&sample);
    assert_eq!(left, [0.0625, 0.3125]);
    assert_eq!(right, [0.125, 0.375]);
}

#[test]
fn broken_or_unknown_input_is_an_error() {
    let cases: Vec<(&str, Vec<u8>, DecodeError)> = vec![
        ("empty", vec![], DecodeError::Unsupported),
        (
            "garbage",
            b"hello world, not audio".to_vec(),
            DecodeError::Unsupported,
        ),
        (
            "id3/mp3",
            b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec(),
            DecodeError::Unsupported,
        ),
        (
            "riff without wave",
            b"RIFF\x04\x00\x00\x00AVI ".to_vec(),
            DecodeError::Unsupported,
        ),
        (
            "adpcm",
            wav(fmt(2, 1, 44_100, 4), &[0; 8]),
            DecodeError::Unsupported,
        ),
        (
            "48-bit int",
            wav(fmt(1, 1, 44_100, 48), &[0; 12]),
            DecodeError::Unsupported,
        ),
        (
            "16-bit float",
            wav(fmt(3, 1, 44_100, 16), &[0; 4]),
            DecodeError::Unsupported,
        ),
        (
            "zero bits",
            wav(fmt(1, 1, 44_100, 0), &[0; 4]),
            DecodeError::Unsupported,
        ),
        (
            "20-bit",
            wav(fmt(1, 1, 44_100, 20), &[0; 6]),
            DecodeError::Unsupported,
        ),
        (
            "64-bit float",
            wav(fmt(3, 1, 44_100, 64), &[0; 16]),
            DecodeError::Unsupported,
        ),
        (
            "extensible adpcm",
            wav(fmt_extensible(2, 1, 44_100, 4), &[0; 8]),
            DecodeError::Unsupported,
        ),
        ("ogg magic only", b"OggS".to_vec(), DecodeError::Unsupported),
        (
            "ogg opus",
            ogg_page(2, 0, 0, &[b"OpusHead\x01\x02\x38\x01\x44\xAC\0\0\0\0\0"]),
            DecodeError::Unsupported,
        ),
        (
            "id3 then garbage",
            [&b"ID3\x04\0\0\0\0\0\x05"[..], b"12345", b"not audio"].concat(),
            DecodeError::Unsupported,
        ),
    ];
    for (name, bytes, expected) in cases {
        assert_eq!(decode(&bytes).unwrap_err(), expected, "{name}");
    }
    let malformed = [
        ("header only", riff(&[])),
        ("no fmt", riff(&[chunk(b"data", &[0; 4])])),
        ("no data", riff(&[fmt(1, 1, 44_100, 16)])),
        ("empty data", wav(fmt(1, 1, 44_100, 16), &[])),
        (
            "short fmt",
            riff(&[chunk(b"fmt ", &[1, 0, 1, 0]), chunk(b"data", &[0; 4])]),
        ),
        ("zero channels", wav(fmt(1, 0, 44_100, 16), &[0; 4])),
        ("zero rate", wav(fmt(1, 1, 0, 16), &[0; 4])),
        ("block align", {
            let mut bytes = wav(fmt(1, 2, 44_100, 16), &[0; 8]);
            bytes[32] = 2; // fmt body offset 12: 2 instead of 4
            bytes
        }),
        ("flac magic only", b"fLaC".to_vec()),
        ("flac cut in streaminfo", FLAC_SINE[..30].to_vec()),
        ("vorbis ident only", VORBIS_SINE[..58].to_vec()),
        ("ogg flac header only", OGG_FLAC_SINE[..151].to_vec()),
    ];
    for (name, bytes) in malformed {
        assert!(
            matches!(decode(&bytes), Err(DecodeError::Malformed(_))),
            "{name}: {:?}",
            decode(&bytes)
        );
    }
}

#[test]
fn the_frame_limit_is_enforced_for_wav_and_flac() {
    assert_eq!(MAX_FRAMES, 1 << 25);
    let bytes = wav(fmt(1, 1, 44_100, 16), &[0; 20]);
    assert_eq!(decode_with_limit(&bytes, 10).unwrap().frames(), 10);
    assert_eq!(
        decode_with_limit(&bytes, 9).unwrap_err(),
        DecodeError::TooLarge
    );
    assert_eq!(decode_with_limit(FLAC_SINE, 4410).unwrap().frames(), 4410);
    assert_eq!(
        decode_with_limit(FLAC_SINE, 4409).unwrap_err(),
        DecodeError::TooLarge
    );
    for bytes in [MP3_SINE, VORBIS_SINE, OGG_FLAC_SINE] {
        assert_eq!(decode_with_limit(bytes, 4410).unwrap().frames(), 4410);
        assert_eq!(
            decode_with_limit(bytes, 4000).unwrap_err(),
            DecodeError::TooLarge
        );
    }
}

#[test]
fn flac_stream_with_a_wav_name_decodes_bit_exact() {
    let sample = decode(FLAC_SINE).unwrap();
    assert_eq!(sample.sample_rate, 44_100);
    assert_eq!(sample.frames(), 4410);
    let (left, right) = stereo(&sample);
    for i in 0..4410 {
        let t = i as f64 / 44_100.0;
        let expect_l = (0.5 * (TAU * 441.0 * t).sin() * 32768.0).round() / 32768.0;
        let expect_r = (0.5 * (TAU * 882.0 * t).sin() * 32768.0).round() / 32768.0;
        assert!(
            (f64::from(left[i]) - expect_l).abs() <= 1.0 / 32768.0,
            "left {i}: {}",
            left[i]
        );
        assert!(
            (f64::from(right[i]) - expect_r).abs() <= 1.0 / 32768.0,
            "right {i}: {}",
            right[i]
        );
    }
    assert!((period(left, 80, 120) - 100.0).abs() < 0.05);
    assert!((period(right, 40, 60) - 50.0).abs() < 0.05);
}

#[test]
fn truncated_flac_keeps_what_decoded_or_errors_without_panicking() {
    for cut in [42, 100, FLAC_SINE.len() / 2, FLAC_SINE.len() - 1] {
        if let Ok(sample) = decode(&FLAC_SINE[..cut]) {
            assert!(sample.frames() < 4410, "cut at {cut}");
            assert_eq!(
                sample.right.as_ref().map(|r| r.len()),
                Some(sample.frames())
            );
        }
    }
}

fn xorshift(mut rng: u64) -> impl FnMut() -> u64 {
    move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    }
}

/// One to six random bit flips, byte sets, truncations, insertions or u32 overwrites at or after `keep`.
fn mutate(bytes: &mut Vec<u8>, keep: usize, next: &mut impl FnMut() -> u64) {
    for _ in 0..1 + next() % 6 {
        let at = keep + next() as usize % bytes.len().saturating_sub(keep).max(1);
        match next() % 5 {
            0 if at < bytes.len() => bytes[at] ^= 1 << (next() % 8),
            1 if at < bytes.len() => bytes[at] = [0, 0xFF, 0x80, next() as u8][next() as usize % 4],
            2 => bytes.truncate(at),
            3 if at <= bytes.len() => bytes.insert(at, next() as u8),
            4 if bytes.len() >= at + 4 => {
                bytes[at..at + 4].copy_from_slice(&(next() as u32).to_le_bytes())
            }
            _ => {}
        }
    }
}

/// No decoder panicked (caught or not), and a decoded sample is well-formed: a rate, 1..=`limit` frames, equal
/// channel lengths and finite values. Returns whether it decoded.
fn decoded_well_formed(
    result: Result<SampleData, DecodeError>,
    limit: usize,
    round: usize,
) -> bool {
    let sample = match result {
        Err(DecodeError::Malformed(message)) => {
            assert!(!message.ends_with("panicked"), "round {round}: {message}");
            return false;
        }
        Err(_) => return false,
        Ok(sample) => sample,
    };
    assert!(sample.sample_rate > 0, "round {round}");
    assert!((1..=limit).contains(&sample.frames()), "round {round}");
    if let Some(right) = &sample.right {
        assert_eq!(right.len(), sample.left.len(), "round {round}");
    }
    let all = sample
        .left
        .iter()
        .chain(sample.right.iter().flat_map(|r| r.iter()));
    assert!(all.into_iter().all(|s| s.is_finite()), "round {round}");
    true
}

/// Mutated and random inputs never panic, and whatever decodes is well-formed.
#[test]
fn fuzzed_input_never_panics() {
    let mut next = xorshift(0x9E37_79B9_7F4A_7C15);
    let pcm: Vec<u8> = (0..64u8).collect();
    let seeds = [
        FLAC_SINE.to_vec(),
        wav(fmt(1, 2, 44_100, 16), &pcm),
        wav(fmt(1, 3, 48_000, 24), &pcm),
        riff(&[
            fmt(3, 2, 44_100, 32),
            smpl(60, Some((2, 5))),
            acid(4, 120.0),
            cue(&[0, 3]),
            chunk(b"data", &pcm),
        ]),
        riff(&[
            chunk(b"JUNK", b"abc"),
            fmt(1, 1, 8_000, 8),
            chunk(b"data", &pcm),
        ]),
    ];
    const LIMIT: usize = 1 << 20;
    let rounds: usize = std::env::var("SAMPLE_FUZZ_ROUNDS").map_or(6000, |n| n.parse().unwrap());
    let (mut flac_ok, mut wav_ok, mut failed) = (0, 0, 0);
    for round in 0..rounds {
        let mut bytes = seeds[round % seeds.len()].clone();
        // Half the rounds keep the FLAC metadata (42 bytes) intact, so the frame decoder sees the damage.
        let keep = if round % 2 == 0 && bytes.starts_with(b"fLaC") {
            42
        } else {
            0
        };
        mutate(&mut bytes, keep, &mut next);
        if round % 11 == 0 {
            let magic = if round % 22 == 0 {
                &b"fLaC"[..]
            } else {
                &b"RIFF\0\0\0\0WAVE"[..]
            };
            bytes = magic
                .iter()
                .copied()
                .chain((0..next() % 600).map(|_| next() as u8))
                .collect();
        }
        if !decoded_well_formed(decode_with_limit(&bytes, LIMIT), LIMIT, round) {
            failed += 1;
            continue;
        }
        *if bytes.starts_with(b"fLaC") {
            &mut flac_ok
        } else {
            &mut wav_ok
        } += 1;
    }
    eprintln!("fuzz: {flac_ok} FLAC and {wav_ok} WAV decoded, {failed} rejected");
    assert!(
        flac_ok > 300 && wav_ok > 300 && failed > 300,
        "the mutations must exercise both paths"
    );
}

/// A PICTURE block whose MIME length, or a VORBIS_COMMENT whose one comment, claims about 4 GiB: symphonia would
/// `vec![0; n]` it before reading. Every metadata block but STREAMINFO is dropped unread.
#[test]
fn hostile_flac_metadata_lengths_allocate_nothing_large() {
    let picture = [
        &[0x06, 0, 0, 12][..],
        &3u32.to_be_bytes(),
        &0xFFFF_FFF0u32.to_be_bytes(),
        &[0; 4],
    ]
    .concat();
    let comment = [
        &[0x04, 0, 0, 12][..],
        &0u32.to_le_bytes(),
        &1u32.to_le_bytes(),
        &0xFFFF_FFF0u32.to_le_bytes(),
    ]
    .concat();
    for block in [picture, comment] {
        let mut last = block.clone();
        last[0] |= 0x80;
        let alone = [&b"fLaC"[..], &streaminfo(false, 16), &last].concat();
        assert_eq!(alone.len(), 58);
        assert!(decode(&alone).is_err());
        let spliced = [&FLAC_SINE[..42], &block, &FLAC_SINE[42..]].concat();
        assert_eq!(decode(&spliced).unwrap(), decode(FLAC_SINE).unwrap());
    }
    let largest = LARGEST_ALLOCATION.load(Ordering::Relaxed);
    assert!(largest < 64 << 20, "largest allocation {largest} bytes");
    let no_streaminfo = [&b"fLaC\x84\0\0\x08"[..], &[0; 8], &FLAC_SINE[86..]].concat();
    assert_eq!(
        decode(&no_streaminfo),
        Err(DecodeError::Malformed("bad FLAC stream"))
    );
}

/// symphonia slices `order` (up to 32) warm-up samples before it checks the block holds them. Blocks under 32
/// samples, and frames that fail to decode, become silence of their length, so later audio keeps its time.
#[test]
fn short_and_corrupt_flac_frames_become_silence_of_their_length() {
    let lpc_32_in_16 = [0x7E, 0, 0, 0, 0, 0, 0, 0];
    let fixed_4_in_2 = [0x18, 0, 0, 0, 0, 0, 0, 0];
    for (block, sub) in [(16, lpc_32_in_16), (2, fixed_4_in_2)] {
        let bytes = [
            &b"fLaC"[..],
            &streaminfo(true, 16),
            &flac_frame(0, block, &sub),
        ]
        .concat();
        assert_eq!(bytes.len(), 59);
        assert_eq!(
            decode(&bytes),
            Err(DecodeError::Malformed("no audio")),
            "nothing decoded"
        );
    }
    let constant = |value: i16| [&[0x00][..], &value.to_be_bytes()].concat();
    let reserved_type = [0x04, 0, 0];
    // 17 wasted bits in a 16-bit stream: symphonia subtracts unchecked (the dev profile override keeps it an error).
    let wasted_over_depth = [0x01, 0x00, 0x00, 0x80, 0, 0];
    let bytes = [
        &b"fLaC"[..],
        &streaminfo(true, 64),
        &flac_frame(0, 64, &constant(4096)),
        &flac_frame(1, 64, &reserved_type),
        &flac_frame(2, 64, &constant(-8192)),
        &flac_frame(3, 64, &wasted_over_depth),
        &flac_frame(4, 16, &lpc_32_in_16),
    ]
    .concat();
    let sample = decode(&bytes).unwrap();
    let expected: Vec<f32> = [(0.125, 64), (0.0, 64), (-0.25, 64), (0.0, 80)]
        .iter()
        .flat_map(|&(value, n)| std::iter::repeat_n(value, n))
        .collect();
    assert_eq!(mono(&sample), expected);
}

/// A frame damaged on disk fails its CRC and the parser drops it; padding to the next frame's position keeps the
/// later audio in time.
#[test]
fn a_frame_dropped_for_its_crc_keeps_later_audio_in_time() {
    let constant = |value: i16| [&[0x00][..], &value.to_be_bytes()].concat();
    let mut damaged = flac_frame(1, 64, &constant(-8192));
    damaged[8] ^= 0x40;
    let bytes = [
        &b"fLaC"[..],
        &streaminfo(true, 64),
        &flac_frame(0, 64, &constant(4096)),
        &damaged,
        &flac_frame(2, 64, &constant(8192)),
    ]
    .concat();
    let expected: Vec<f32> = [(0.125, 64), (0.0, 64), (0.25, 64)]
        .iter()
        .flat_map(|&(value, n)| std::iter::repeat_n(value, n))
        .collect();
    assert_eq!(mono(&decode(&bytes).unwrap()), expected);

    let clean = decode(FLAC_SINE).unwrap();
    let mut bytes = FLAC_SINE.to_vec();
    bytes[1000] ^= 0x10; // inside the first frame (4096 samples)
    let sample = decode(&bytes).unwrap();
    assert_eq!(sample.frames(), 4410);
    assert!(sample.left[..4096].iter().all(|&s| s == 0.0));
    assert_eq!(sample.left[4096..], clean.left[4096..]);
    assert_eq!(sample.right.unwrap()[4096..], clean.right.unwrap()[4096..]);
}

/// Damage inside a frame, with its CRCs re-signed, so it gets past the reader's checks to the decoder.
#[test]
fn crc_resigned_flac_fuzz_never_panics() {
    let mut rng = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let rounds: usize = std::env::var("SAMPLE_FUZZ_ROUNDS").map_or(3000, |n| n.parse().unwrap());
    let (mut ok, mut failed) = (0, 0);
    for round in 0..rounds {
        let (start, end, header) = FLAC_SINE_FRAMES[round % 2];
        let mut frame = FLAC_SINE[start..end - 2].to_vec();
        // One round in four also damages the header after the sync code and re-signs its CRC-8.
        let from = if round % 4 == 0 { 2 } else { header };
        for _ in 0..1 + next() % 4 {
            let at = from + next() as usize % (frame.len() - from).max(1);
            match next() % 5 {
                0 if at < frame.len() => frame[at] ^= 1 << (next() % 8),
                1 if at < frame.len() => frame[at] = next() as u8,
                2 => frame.truncate(at.max(header + 1)),
                3 => frame.insert(at.min(frame.len()), next() as u8),
                4 if at + 4 <= frame.len() => {
                    frame[at..at + 4].copy_from_slice(&(next() as u32).to_le_bytes())
                }
                _ => {}
            }
        }
        if from < header && frame.len() > header {
            frame[header - 1] = crc8(&frame[..header - 1]);
        }
        frame.extend(crc16(&frame).to_be_bytes());
        let bytes = [&FLAC_SINE[..start], &frame, &FLAC_SINE[end..]].concat();
        match decode(&bytes) {
            Ok(sample) => {
                ok += 1;
                assert_eq!(
                    sample.right.as_ref().map(|r| r.len()),
                    Some(sample.frames()),
                    "round {round}"
                );
                assert!(sample.left.iter().all(|s| s.is_finite()), "round {round}");
            }
            Err(error) => {
                failed += 1;
                assert_ne!(
                    error,
                    DecodeError::Malformed("FLAC decoder panicked"),
                    "round {round}"
                );
            }
        }
    }
    eprintln!("crc fuzz: {ok} decoded, {failed} rejected");
    assert!(ok > rounds / 2, "most damaged frames are padded, not fatal");
}

/// The reference tone at frame `i` (44.1 kHz): 0.5·sin(441 Hz) left, 0.5·sin(882 Hz) right.
fn tone(i: usize) -> (f64, f64) {
    let t = i as f64 / 44_100.0;
    (0.5 * (TAU * 441.0 * t).sin(), 0.5 * (TAU * 882.0 * t).sin())
}

/// RMS difference between a decoded stereo tone and the reference.
fn tone_error(sample: &SampleData) -> f64 {
    let (left, right) = stereo(sample);
    let sum: f64 = (0..sample.frames())
        .map(|i| {
            let (l, r) = tone(i);
            (f64::from(left[i]) - l).powi(2) + (f64::from(right[i]) - r).powi(2)
        })
        .sum();
    (sum / (2 * sample.frames()) as f64).sqrt()
}

/// Lossy decodes keep the tone's length, pitch and timing: the MP3 encoder delay and padding (LAME tag) and the Vorbis
/// end (last granule position) are trimmed. Measured RMS errors: MP3 0.018 and Vorbis 0.010 aligned, both 0.035 or
/// more one frame either way.
#[test]
fn mp3_and_ogg_vorbis_decode_in_time_and_in_tune() {
    for (name, bytes) in [("mp3", MP3_SINE), ("vorbis", VORBIS_SINE)] {
        let sample = decode(bytes).unwrap();
        assert_eq!(
            (sample.sample_rate, sample.frames()),
            (44_100, 4410),
            "{name}"
        );
        let (left, right) = stereo(&sample);
        assert!((period(left, 80, 120) - 100.0).abs() < 0.1, "{name}");
        assert!((period(right, 40, 60) - 50.0).abs() < 0.1, "{name}");
        let error = tone_error(&sample);
        assert!(error < 0.025, "{name}: RMS error {error}");
    }
    let mono_22k = decode(MP3_MONO_22K).unwrap();
    assert_eq!(mono_22k.sample_rate, 22_050);
    assert_eq!(
        mono(&mono_22k).len() % 576,
        0,
        "no LAME tag: whole MPEG-2 frames, untrimmed"
    );
    assert!((period(&mono_22k.left, 40, 60) - 50.0).abs() < 0.1);
}

#[test]
fn ogg_flac_decodes_as_the_native_stream_does() {
    assert_eq!(decode(OGG_FLAC_SINE).unwrap(), decode(FLAC_SINE).unwrap());
}

/// An ID3v2 tag: header, syncsafe size, `body`, and a footer when asked.
fn id3v2(body: &[u8], footer: bool) -> Vec<u8> {
    let n = body.len() as u32;
    let size = [n >> 21, n >> 14, n >> 7, n].map(|b| (b & 0x7F) as u8);
    let mut tag = [&b"ID3\x04\x00"[..], &[u8::from(footer) << 4], &size, body].concat();
    if footer {
        tag.extend([&b"3DI\x04\x00\x10"[..], &size].concat());
    }
    tag
}

/// The fallback order finds MP3 wherever it starts: after ID3v2 tags, after leading junk, and inside a RIFF the WAV
/// reader rejects (format tag 0x55). ID3v2 tags before FLAC and Ogg are skipped too.
#[test]
fn mp3_is_found_after_tags_junk_and_inside_riff() {
    let clean = decode(MP3_SINE).unwrap();
    // Two silent MPEG-1 frames inside the tag: read as audio, they would add 2304 frames and hide the LAME tag.
    let silent_frames = [&[0xFF, 0xFB, 0x90, 0x44][..], &[0; 413]]
        .concat()
        .repeat(2);
    let tagged = [
        id3v2(&silent_frames, false),
        id3v2(b"TIT2", true),
        MP3_SINE.to_vec(),
    ]
    .concat();
    assert_eq!(decode(&tagged).unwrap(), clean, "id3v2");
    let mut lying = id3v2(b"", false);
    lying[6..10].fill(0x7F); // claims 256 MiB: only its header is skipped
    assert_eq!(
        decode(&[&lying, MP3_SINE].concat()).unwrap(),
        clean,
        "lying"
    );
    let junk: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
    assert_eq!(decode(&[&junk, MP3_SINE].concat()).unwrap(), clean, "junk");
    let mp3_in_wav = riff(&[fmt(0x55, 2, 44_100, 0), chunk(b"data", MP3_SINE)]);
    assert_eq!(decode(&mp3_in_wav).unwrap(), clean, "riff");
    for bytes in [FLAC_SINE, VORBIS_SINE, OGG_FLAC_SINE] {
        let tagged = [id3v2(&silent_frames, true), bytes.to_vec()].concat();
        assert_eq!(decode(&tagged).unwrap(), decode(bytes).unwrap());
    }
}

/// An MP3 frame that fails to decode (side information with big_values over 288) becomes silence of its length.
#[test]
fn an_mp3_frame_that_fails_to_decode_becomes_silence_of_its_length() {
    let mut bytes = MP3_SINE.to_vec();
    bytes[MP3_SINE_FRAMES[2] + 8] = 0xFF;
    let sample = decode(&bytes).unwrap();
    assert_eq!(sample.frames(), 4410);
    // The second audio frame covers 1152..2304 before the 1105-frame delay is trimmed.
    let (left, right) = stereo(&sample);
    assert!(
        left[47..1199]
            .iter()
            .chain(&right[47..1199])
            .all(|&s| s == 0.0)
    );
    assert!(left[..47].iter().any(|&s| s != 0.0) && left[3500..].iter().any(|&s| s.abs() > 0.1));
}

/// `MP3_SINE` with its LAME tag's CRC re-signed after `edit` has changed the Info frame. The CRC-16 (polynomial 0x8005,
/// reflected) covers the frame's first 190 bytes.
fn relame(edit: impl FnOnce(&mut [u8])) -> Vec<u8> {
    let mut bytes = MP3_SINE.to_vec();
    edit(&mut bytes);
    let crc = bytes[..190].iter().fold(0u16, |crc, &b| {
        (0..8).fold(crc ^ u16::from(b), |c, _| {
            if c & 1 != 0 {
                (c >> 1) ^ 0xA001
            } else {
                c >> 1
            }
        })
    });
    bytes[190..192].copy_from_slice(&crc.to_be_bytes());
    bytes
}

/// A LAME tag (its CRC re-signed) whose delay and padding exceed the Xing frame count: symphonia subtracts them
/// unchecked when trimming gaplessly. Release builds wrap and skip the end trim; the dev profile does the same.
#[test]
fn a_lame_tag_longer_than_its_stream_trims_only_the_delay() {
    let bytes = relame(|b| b[0x2C..0x30].fill(0)); // Xing frame count
    assert_eq!(decode(&bytes).unwrap().frames(), 5 * 1152 - 1105);
}

/// Padding over a whole frame (the tag's 12-bit field holds up to 4095, 529 of it the decoder's delay) empties the
/// last packet, which still carries its stale timestamp: the decode ends at the tagged length, not padded out to it.
/// The fixture's tag has 1105 frames of delay and 5760 in all, so a raw padding of `p` tags `5184 - p` frames.
#[test]
fn mp3_padding_over_a_frame_ends_at_the_tagged_length() {
    let clean = decode(MP3_SINE).unwrap();
    // The last packet is empty with its timestamp 94, 14 and 1 frames past the tagged end for raw 1775, 1695 and 1682;
    // 1681 is a padding of exactly one frame (1681 - 529 = 1152), which empties it where the audio ends.
    for (raw, tagged) in [(1775u16, 3409), (1695, 3489), (1682, 3502), (1681, 3503)] {
        let bytes = relame(|b| {
            b[0xB2] = b[0xB2] & 0xF0 | (raw >> 8) as u8;
            b[0xB3] = raw as u8;
        });
        let sample = decode(&bytes).unwrap();
        assert_eq!(sample.frames(), tagged, "raw padding {raw}");
        assert_eq!(sample.left[..], clean.left[..tagged], "raw padding {raw}");
    }
}

/// A page that fails its CRC is dropped; padding to the next page's granule position keeps later audio in time.
#[test]
fn a_lost_ogg_page_keeps_later_audio_in_time() {
    let clean = decode(VORBIS_SINE).unwrap();
    let mut bytes = VORBIS_SINE.to_vec();
    bytes[VORBIS_SINE_PAGES[3] + 40] ^= 0x10; // the page ending at granule 2752
    let sample = decode(&bytes).unwrap();
    assert_eq!(sample.frames(), 4410);
    let (left, right) = stereo(&sample);
    let (clean_left, clean_right) = stereo(&clean);
    assert!(left[2752..3776].iter().all(|&s| s == 0.0));
    assert_eq!(
        (&left[3776..], &right[3776..]),
        (&clean_left[3776..], &clean_right[3776..])
    );
}

/// CRC-32 of an Ogg page (RFC 3533: polynomial 0x04C11DB7, not reflected, with the CRC field read as zero).
fn ogg_crc(page: &[u8]) -> u32 {
    page.iter().enumerate().fold(0, |crc, (i, &b)| {
        let b = if (22..26).contains(&i) { 0 } else { b };
        (0..8).fold(crc ^ (u32::from(b) << 24), |c, _| {
            if c & 0x8000_0000 != 0 {
                (c << 1) ^ 0x04C1_1DB7
            } else {
                c << 1
            }
        })
    })
}

fn resign(page: &mut [u8]) {
    let crc = ogg_crc(page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
}

/// An Ogg page of serial 0 holding whole packets.
fn ogg_page(flags: u8, granule: u64, sequence: u32, packets: &[&[u8]]) -> Vec<u8> {
    let lacing: Vec<u8> = packets
        .iter()
        .flat_map(|p| std::iter::repeat_n(255, p.len() / 255).chain([(p.len() % 255) as u8]))
        .collect();
    let mut page = [
        &b"OggS\0"[..],
        &[flags],
        &granule.to_le_bytes(),
        &[0; 4],
        &sequence.to_le_bytes(),
        &[0; 4],
        &[lacing.len() as u8],
        &lacing,
    ]
    .concat();
    page.extend(packets.concat());
    resign(&mut page);
    page
}

/// Packs `(value, width)` fields least significant bit first (Vorbis I §2.1.4).
fn pack_bits(fields: &[(u64, u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    let bits = fields.iter().flat_map(|&(value, width)| {
        (0..width).map(move |i| value.checked_shr(i).unwrap_or(0) as u8 & 1)
    });
    for (at, bit) in bits.enumerate() {
        if at % 8 == 0 {
            out.push(0);
        }
        *out.last_mut().unwrap() |= bit << (at % 8);
    }
    out
}

/// A setup header whose one codebook (Vorbis I §3.2.1) is length-ordered, `counts[k]` entries having length
/// `first + k`, with lookup type `lookup` and 1-bit multiplicands, then zero bits that are no valid rest of a setup.
fn vorbis_setup(dimensions: u64, entries: u64, first: u64, counts: &[u64], lookup: u64) -> Vec<u8> {
    let mut fields = vec![
        (0, 8),
        (0x56_4342, 24),
        (dimensions, 16),
        (entries, 24),
        (1, 1),
        (first - 1, 5),
    ];
    let mut entry = 0;
    for &count in counts {
        fields.push((count, 64 - (entries - entry).leading_zeros()));
        entry += count;
    }
    fields.extend([(lookup, 4), (0, 32), (0, 32), (0, 4), (0, 1)]);
    fields.extend([(0, 64); 8]);
    [&b"\x05vorbis"[..], &pack_bits(&fields)].concat()
}

/// The fixture's ident packet, a bare comment, `setup`, then the fixture's audio pages.
fn vorbis_with_setup(setup: &[u8]) -> Vec<u8> {
    let comment = b"\x03vorbis\0\0\0\0\0\0\0\0\x01";
    [
        ogg_page(2, 0, 0, &[&VORBIS_SINE[28..58]]),
        ogg_page(0, 0, 1, &[comment, setup]),
        VORBIS_SINE[VORBIS_SINE_PAGES[2]..].to_vec(),
    ]
    .concat()
}

/// Headers symphonia would trust: a Vorbis comment or an Ogg FLAC PICTURE claiming about 4 GiB (`vec![0; n]` before
/// reading), Vorbis codebooks that size a lookup table from unchecked counts or trip its asserts, and floors its renderer
/// indexes unchecked. Comments and pictures are never handed over; setup headers are checked first.
#[test]
fn hostile_ogg_headers_allocate_nothing_large_and_never_panic() {
    let mut vorbis = VORBIS_SINE.to_vec();
    vorbis[122..126].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes()); // the one comment's length
    resign(&mut vorbis[VORBIS_SINE_PAGES[1]..VORBIS_SINE_PAGES[2]]);
    assert_eq!(decode(&vorbis).unwrap(), decode(VORBIS_SINE).unwrap());
    let mut flac = OGG_FLAC_SINE.to_vec();
    flac[107] = 0x86; // the VORBIS_COMMENT block becomes a PICTURE...
    flac[111..115].copy_from_slice(&3u32.to_be_bytes());
    flac[115..119].copy_from_slice(&0xFFFF_FFF0u32.to_be_bytes()); // ...whose MIME type is about 4 GiB
    resign(&mut flac[79..151]);
    assert_eq!(decode(&flac).unwrap(), decode(OGG_FLAC_SINE).unwrap());

    let lengths_1_to_31_then_three_32s: Vec<u64> = [vec![1; 31], vec![3]].concat();
    let setups = [
        (
            "130 MB lookup table",
            vorbis_setup(31, 1 << 20, 20, &[1 << 20], 1),
        ),
        (
            "2^40 in lookup1_values",
            vorbis_setup(40, 1 << 20, 20, &[1 << 20], 1),
        ),
        (
            "2^32 type 2 values",
            vorbis_setup(1 << 15, 1 << 17, 17, &[1 << 17], 2),
        ),
        ("zero dimensions", vorbis_setup(0, 1, 1, &[1], 2)),
        ("length 33", vorbis_setup(1, 1, 32, &[0, 1], 0)),
        (
            "overfull at length 32",
            vorbis_setup(1, 34, 1, &lengths_1_to_31_then_three_32s, 0),
        ),
    ];
    for (name, setup) in setups {
        match decode(&vorbis_with_setup(&setup)) {
            Err(DecodeError::Malformed(message)) => {
                assert!(!message.ends_with("panicked"), "{name}: {message}")
            }
            other => panic!("{name}: {other:?}"),
        }
    }
    // Block sizes the setup's floors don't fit (256 and 2048 here, floors 128 and 1024 wide): symphonia's floor renderer
    // indexes past its buffer. 0x99 makes both blocks 512, 0xB6 makes the short one 64.
    for block_sizes in [0x99, 0xB6] {
        let mut bytes = VORBIS_SINE.to_vec();
        bytes[28 + 28] = block_sizes;
        resign(&mut bytes[..VORBIS_SINE_PAGES[1]]);
        assert_eq!(
            decode(&bytes),
            Err(DecodeError::Malformed("bad Ogg stream")),
            "{block_sizes:#x}"
        );
    }
    // One bit of the setup's floors (found by the CRC fuzz) lets floor values outgrow the dB table symphonia indexes.
    let mut bytes = VORBIS_SINE.to_vec();
    bytes[3785] ^= 0x80;
    resign(&mut bytes[VORBIS_SINE_PAGES[1]..VORBIS_SINE_PAGES[2]]);
    assert_eq!(
        decode(&bytes),
        Err(DecodeError::Malformed("bad Ogg stream"))
    );
    // Two setup bits (from the CRC fuzz) give a residue a class book past the last codebook, which symphonia indexes.
    let mut bytes = VORBIS_SINE.to_vec();
    bytes[3829] ^= 0x40;
    bytes[3869] ^= 0x80;
    resign(&mut bytes[VORBIS_SINE_PAGES[1]..VORBIS_SINE_PAGES[2]]);
    assert_eq!(
        decode(&bytes),
        Err(DecodeError::Malformed("bad Ogg stream"))
    );
    // One bit of a codebook (also from the CRC fuzz) decodes to infinities and NaNs: they become silence, as in a WAV.
    let mut bytes = VORBIS_SINE.to_vec();
    bytes[2957] ^= 0x40;
    resign(&mut bytes[VORBIS_SINE_PAGES[1]..VORBIS_SINE_PAGES[2]]);
    let sample = decode(&bytes).unwrap();
    let (left, right) = stereo(&sample);
    assert_eq!(sample.frames(), 4410);
    assert!(left.iter().chain(right).all(|s| s.is_finite()));
    // The most the budgets let through, behind a valid rest of the setup (floor 1 without partitions, so the big book
    // is no floor's): 2^18 one-dimensional entries of length 18. It decodes the fixture's packets as garbage.
    let book = [
        (0, 8),
        (0x56_4342, 24),
        (1, 16),
        (1 << 18, 24),
        (1, 1),
        (17, 5),
        (1 << 18, 19),
        (0, 4),
    ];
    let floor = [(1, 16), (0, 5), (1, 2), (7, 4)];
    let setup = setup_with(&book, &floor, 0, &ONE_SUBMAP);
    let (result, _, _, largest) = measure(|| decode(&vorbis_with_setup(&setup)));
    decoded_well_formed(result, MAX_FRAMES, 0); // no panic, and well-formed if anything decodes
    eprintln!("the most the budgets allow: largest allocation {largest} bytes");
    assert!(largest < 16 << 20, "largest allocation {largest} bytes");
    let largest = LARGEST_ALLOCATION.load(Ordering::Relaxed);
    eprintln!("largest allocation in this test binary so far: {largest} bytes");
    assert!(largest < 64 << 20, "largest allocation {largest} bytes");
}

/// Damage inside an Ogg page, with its CRC re-signed, so it reaches the demuxer and the decoders.
#[test]
fn crc_resigned_ogg_fuzz_never_panics() {
    const LIMIT: usize = 1 << 20;
    let mut next = xorshift(0xD1B5_4A32_D192_ED03);
    let rounds: usize = std::env::var("SAMPLE_FUZZ_ROUNDS").map_or(3000, |n| n.parse().unwrap());
    let ogg_flac_pages = [0, 79, 151, OGG_FLAC_SINE.len()];
    let (mut ok, mut failed) = (0, 0);
    for round in 0..rounds {
        let (seed, pages): (&[u8], &[usize]) = if round % 2 == 0 {
            (VORBIS_SINE, &VORBIS_SINE_PAGES)
        } else {
            (OGG_FLAC_SINE, &ogg_flac_pages)
        };
        let page = next() as usize % (pages.len() - 1);
        let (start, end) = (pages[page], pages[page + 1]);
        // One round in four also damages the header fields after the capture pattern.
        let from = start
            + if round % 4 == 0 {
                5
            } else {
                27 + usize::from(seed[start + 26])
            };
        let mut bytes = seed.to_vec();
        for _ in 0..1 + next() % 4 {
            let at = from + next() as usize % (end - from);
            match next() % 3 {
                0 => bytes[at] ^= 1 << (next() % 8),
                1 => bytes[at] = next() as u8,
                _ if at + 4 <= end => {
                    bytes[at..at + 4].copy_from_slice(&(next() as u32).to_le_bytes())
                }
                _ => {}
            }
        }
        resign(&mut bytes[start..end]);
        if decoded_well_formed(decode_with_limit(&bytes, LIMIT), LIMIT, round) {
            ok += 1;
        } else {
            failed += 1;
        }
    }
    eprintln!("ogg crc fuzz: {ok} decoded, {failed} rejected");
    assert!(ok > rounds / 3, "most damaged audio pages still decode");
}

/// Mutated and random MP3 and Ogg inputs never panic, and whatever decodes is well-formed.
#[test]
fn fuzzed_mp3_and_ogg_never_panic() {
    const LIMIT: usize = 1 << 20;
    let mut next = xorshift(0x5851_F42D_4C95_7F2D);
    let rounds: usize = std::env::var("SAMPLE_FUZZ_ROUNDS").map_or(3000, |n| n.parse().unwrap());
    let seeds = [
        MP3_SINE.to_vec(),
        MP3_MONO_22K.to_vec(),
        VORBIS_SINE.to_vec(),
        OGG_FLAC_SINE.to_vec(),
        [id3v2(b"TIT2", true), MP3_SINE.to_vec()].concat(),
        riff(&[fmt(0x55, 2, 44_100, 0), chunk(b"data", MP3_SINE)]),
    ];
    // Half the rounds damage only what follows the MP3 Info frame or the Ogg header pages.
    let audio_from = [417, 0, VORBIS_SINE_PAGES[2], 151, 417 + 24, 417 + 44];
    let (mut ok, mut failed) = ([0; 6], 0);
    for round in 0..rounds {
        let seed = round % seeds.len();
        let mut bytes = seeds[seed].clone();
        mutate(
            &mut bytes,
            if round % 2 == 0 { audio_from[seed] } else { 0 },
            &mut next,
        );
        if round % 13 == 0 {
            let magic = [&b"OggS"[..], b"ID3\x04\0\0\0\0\0\0", b"\xFF\xFB\x90\x44"][round % 3];
            bytes = magic
                .iter()
                .copied()
                .chain((0..next() % 2000).map(|_| next() as u8))
                .collect();
        }
        if decoded_well_formed(decode_with_limit(&bytes, LIMIT), LIMIT, round) {
            ok[seed] += 1;
        } else {
            failed += 1;
        }
    }
    eprintln!("mp3/ogg fuzz: {ok:?} decoded per seed, {failed} rejected");
    // Ogg FLAC keeps both frames on one page, whose CRC any damage fails: the CRC-resigned fuzz decodes it.
    assert!(
        ok.iter()
            .enumerate()
            .all(|(seed, &n)| seed == 3 || n > rounds / 20),
        "every other seed must still decode often"
    );
}

/// A Vorbis ident header: `channels` at 44.1 kHz, blocks of 256 and 2048.
fn vorbis_ident(channels: u8) -> Vec<u8> {
    [
        &b"\x01vorbis\0\0\0\0"[..],
        &[channels],
        &44_100u32.to_le_bytes(),
        &[0; 12],
        &[0xB8, 1],
    ]
    .concat()
}

/// A whole setup header (Vorbis I §4.2.4): `codebooks` (count field first), one time placeholder, `floor` (type and
/// fields), residue 0 of `residue_type` covering nothing, `mapping` (type and fields), and one short-block mode.
fn setup_with(
    codebooks: &[(u64, u32)],
    floor: &[(u64, u32)],
    residue_type: u64,
    mapping: &[(u64, u32)],
) -> Vec<u8> {
    let mut fields = codebooks.to_vec();
    fields.extend([(0, 6), (0, 16), (0, 6)]);
    fields.extend(floor);
    fields.extend([
        (0, 6),
        (residue_type, 16),
        (0, 72),
        (0, 6),
        (0, 8),
        (0, 4),
        (0, 6),
    ]);
    fields.extend(mapping);
    fields.extend([(0, 6), (0, 1), (0, 32), (0, 8), (1, 1)]);
    [&b"\x05vorbis"[..], &pack_bits(&fields)].concat()
}

/// `setup_with` codebook 0 alone: one dimension, two entries of length 1, scalar.
fn vorbis_setup_of(floor: &[(u64, u32)], residue_type: u64, mapping: &[(u64, u32)]) -> Vec<u8> {
    let two_entries = [
        (0, 8),
        (0x56_4342, 24),
        (1, 16),
        (2, 24),
        (0, 2),
        (0, 10),
        (0, 4),
    ];
    setup_with(&two_entries, floor, residue_type, mapping)
}

/// Floor 1: one partition of class 0 (one dimension, subclass book codebook 0), multiplier 2 (range 128), x range 2^7,
/// and the one x value `x`.
fn floor1_with_x(x: u64) -> Vec<(u64, u32)> {
    vec![
        (1, 16),
        (1, 5),
        (0, 4),
        (0, 3),
        (0, 2),
        (1, 8),
        (1, 2),
        (7, 4),
        (x, 7),
    ]
}

/// Mapping 0 with one submap: no coupling, floor 0, residue 0.
const ONE_SUBMAP: [(u64, u32); 7] = [(0, 16), (0, 1), (0, 1), (0, 2), (0, 8), (0, 8), (0, 8)];

/// An Ogg Vorbis stream: `ident`, a bare comment and `setup`, then `packets` on one page ending at granule 1024.
fn vorbis_stream(ident: &[u8], setup: &[u8], packets: &[&[u8]]) -> Vec<u8> {
    let comment = b"\x03vorbis\0\0\0\0\0\0\0\0\x01";
    [
        ogg_page(2, 0, 0, &[ident]),
        ogg_page(0, 0, 1, &[comment, setup]),
        ogg_page(4, 1024, 2, packets),
    ]
    .concat()
}

/// Setups symphonia accepts but decodes with a panic, each with a packet that reaches it. A floor 1 with x
/// 64 is the control that the builder makes streams that decode, and two submaps shaped like libvorbis's 5.1 mapping
/// (the last channel alone) still decode.
#[test]
fn vorbis_setups_whose_packets_panic_symphonia_are_refused() {
    // Floor 1 with one post set to 1 (codebook 0's second entry).
    let floor1_packet = pack_bits(&[(0, 1), (1, 1), (10, 7), (20, 7), (1, 1)]);
    let control = vorbis_setup_of(&floor1_with_x(64), 0, &ONE_SUBMAP);
    let control = vorbis_stream(
        &vorbis_ident(1),
        &control,
        &[&floor1_packet, &floor1_packet],
    );
    assert_eq!(decode(&control).unwrap().frames(), 128);

    // Floor 0 with 16 books reads a 5-bit book index into its 16-entry list.
    let mut floor0 = vec![(0, 16), (1, 8), (1, 16), (1, 16), (1, 6), (0, 8), (15, 4)];
    floor0.extend([(0, 8); 16]);
    let floor0_packet = pack_bits(&[(0, 1), (1, 1), (31, 5)]);
    // An x value equal to an implicit post (0 or 2^rangebits) makes a zero-width line, which it divides by.
    let duplicate_x = |x| vorbis_setup_of(&floor1_with_x(x), 0, &ONE_SUBMAP);
    // Two submaps over three channels with a type 2 residue not starting at channel 0: its deinterleave indexes past
    // the channels.
    let two_submaps = [
        (0, 16),
        (1, 1),
        (1, 4),
        (0, 1),
        (0, 2),
        (0, 4),
        (1, 4),
        (1, 4),
        (0, 48),
    ];
    let all_floors_unused = pack_bits(&[(0, 4)]);
    let mut like_5_1 = two_submaps;
    (like_5_1[6], like_5_1[7]) = ((0, 4), (1, 4)); // channels 0 and 1 in submap 0, channel 2 in submap 1
    let like_5_1 = vorbis_setup_of(&floor1_with_x(64), 2, &like_5_1);
    let like_5_1 = vorbis_stream(
        &vorbis_ident(3),
        &like_5_1,
        &[&all_floors_unused, &all_floors_unused],
    );
    assert_eq!(decode(&like_5_1).unwrap().frames(), 128);
    let cases = [
        (
            "floor 0",
            vorbis_stream(
                &vorbis_ident(1),
                &vorbis_setup_of(&floor0, 0, &ONE_SUBMAP),
                &[&floor0_packet],
            ),
        ),
        (
            "x at 0",
            vorbis_stream(&vorbis_ident(1), &duplicate_x(0), &[&floor1_packet]),
        ),
        (
            "x at 2^rangebits",
            vorbis_stream(&vorbis_ident(1), &duplicate_x(128), &[&floor1_packet]),
        ),
        (
            "two submaps",
            vorbis_stream(
                &vorbis_ident(3),
                &vorbis_setup_of(&floor1_with_x(64), 2, &two_submaps),
                &[&all_floors_unused],
            ),
        ),
    ];
    for (name, bytes) in cases {
        assert_eq!(
            decode(&bytes),
            Err(DecodeError::Malformed("bad Ogg stream")),
            "{name}"
        );
    }
}

/// One codebook with per-entry codeword lengths (`lengths`, packed densely) and no lookup, then zero bits.
fn dense_codebook_setup(lengths: &[u64]) -> Vec<u8> {
    let mut fields = vec![
        (0, 8),
        (0x56_4342, 24),
        (1, 16),
        (lengths.len() as u64, 24),
        (0, 2),
    ];
    fields.extend(lengths.iter().map(|&length| (length - 1, 5)));
    fields.extend([(0, 4)]);
    fields.extend([(0, 64); 8]);
    [&b"\x05vorbis"[..], &pack_bits(&fields)].concat()
}

/// symphonia's codebook lookup table grows by a block of up to 256 entries for each 8-bit prefix level a codeword
/// crosses, so its size depends on the code's shape, not just its entry count. Codes of lengths 1, 2, 4, 5
/// and 9 to 12 fill all but 10 000/65 536 of the tree, then 10 000 runs of 17, 18, …, 24, 24 fill one depth-16 subtree
/// each, every one a full 256-entry block: about 2.6 million table entries (20 MB) from 90 008 codewords.
#[test]
fn a_codebook_whose_lookup_table_would_be_huge_is_refused() {
    let run: Vec<u64> = (17..=24).chain([24]).collect();
    let lengths: Vec<u64> = [vec![1, 2, 4, 5, 9, 10, 11, 12], run.repeat(10_000)].concat();
    let bytes = vorbis_with_setup(&dense_codebook_setup(&lengths));
    let (result, _, _, largest) = measure(|| decode(&bytes));
    assert_eq!(result, Err(DecodeError::Malformed("bad Ogg stream")));
    assert!(largest < 16 << 20, "largest allocation {largest} bytes");
}

/// Zero-length and one-byte header-type packets carry nothing; they must not pile up in memory.
#[test]
fn tiny_ogg_packets_do_not_accumulate() {
    let one_byte = [1u8];
    let packets: Vec<&[u8]> = [&one_byte[..], &[]].repeat(127);
    let junk: Vec<u8> = (0..400)
        .flat_map(|k| ogg_page(0, 0, 100 + k, &packets))
        .collect();
    let split = VORBIS_SINE_PAGES[2];
    let bytes = [&VORBIS_SINE[..split], &junk, &VORBIS_SINE[split..]].concat();
    let (clean, _, clean_peak, _) = measure(|| decode(VORBIS_SINE).unwrap());
    let (padded, _, peak, _) = measure(|| decode(&bytes).unwrap());
    assert_eq!(padded, clean);
    assert!(
        peak < clean_peak + (256 << 10),
        "peak {peak} bytes, {clean_peak} without the {} junk bytes",
        junk.len()
    );
}

/// Once the output reaches the last granule position, later packets are not decoded: the work does not grow
/// with them.
#[test]
fn packets_past_the_stream_end_are_not_decoded() {
    let (start, end) = (VORBIS_SINE_PAGES[2], VORBIS_SINE_PAGES[3]);
    let stream = |pages: u32| {
        let mut bytes = VORBIS_SINE[..start].to_vec();
        for k in 0..pages {
            let mut page = VORBIS_SINE[start..end].to_vec();
            page[6..14].copy_from_slice(&100u64.to_le_bytes());
            page[18..22].copy_from_slice(&(2 + k).to_le_bytes());
            resign(&mut page);
            bytes.extend(page);
        }
        bytes
    };
    let (few, many) = (stream(4), stream(64));
    let (few, few_allocations, _, _) = measure(|| decode(&few).unwrap());
    let (many, many_allocations, _, _) = measure(|| decode(&many).unwrap());
    assert_eq!((few.frames(), many.frames()), (100, 100));
    assert!(
        many_allocations < few_allocations + 20,
        "{many_allocations} allocations with 60 more pages, {few_allocations} without"
    );
}

/// Each `OggS` that fails its CRC costs a CRC over the page it claims (up to 64 KiB). Scanning stops once four times
/// the input has been checked, so a stream behind a wall of such candidates is not found.
#[test]
fn ogg_scanning_stops_after_checking_four_times_the_input() {
    let mut candidate = [&b"OggS"[..], &[0; 22], &[255]].concat();
    candidate.extend([255; 255]); // claims 65 025 body bytes, and its CRC field (zero) is wrong
    let wall = candidate.repeat(300);
    let bytes = [&wall[..], VORBIS_SINE, &[0; 65_536]].concat();
    assert_eq!(decode(&bytes), Err(DecodeError::Unsupported));
}

/// `count` silent MPEG-1 layer III frames (128 kbps, 44.1 kHz, joint stereo).
fn silent_mp3_frames(count: usize) -> Vec<u8> {
    [&[0xFF, 0xFB, 0x90, 0x44][..], &[0; 413]]
        .concat()
        .repeat(count)
}

/// The MP3 catch-all needs four consecutive layer III frames, or a shorter run that ends the file, so a non-audio file
/// is refused rather than decoded as noise; and layer I/II sync words before real MP3 do not hide it.
#[test]
fn mp3_needs_a_run_of_layer_iii_frames() {
    let junk: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
    let two_frames_in_junk = [&junk[..], &silent_mp3_frames(2), &junk].concat();
    assert_eq!(decode(&two_frames_in_junk), Err(DecodeError::Unsupported));
    let short = &MP3_SINE[..MP3_SINE_FRAMES[3]];
    assert_eq!(decode(short).unwrap().frames(), 2 * 1152 - 1105); // an Info frame, then two
    // Two MPEG-1 layer II frames (160 kbps, 522 bytes) before the MP3.
    let layer_ii = [&[0xFF, 0xFD, 0x90, 0x44][..], &[0; 518]]
        .concat()
        .repeat(2);
    let behind_layer_ii = [&layer_ii[..], MP3_SINE].concat();
    assert_eq!(decode(&behind_layer_ii).unwrap(), decode(MP3_SINE).unwrap());
}

/// A FLAC frame at any position: fixed blocking, frame `number` (UTF-8 coded, under 2^16), mono, `block` samples.
fn flac_frame_numbered(number: u32, block: u8, sub: &[u8]) -> Vec<u8> {
    let coded = [
        0xE0 | (number >> 12) as u8,
        0x80 | (number >> 6 & 0x3F) as u8,
        0x80 | (number & 0x3F) as u8,
    ];
    let mut frame = [&[0xFF, 0xF8, 0x60, 0x00][..], &coded, &[block - 1]].concat();
    frame.push(crc8(&frame));
    frame.extend(sub);
    frame.extend(crc16(&frame).to_be_bytes());
    frame
}

/// Silence padded up to a far frame position is allocated once, at its size: no doubling, no second copy beyond the
/// shared one.
#[test]
fn padding_to_a_far_frame_allocates_its_size_once() {
    let constant = [0x00, 0x10, 0x00];
    let bytes = [
        &b"fLaC"[..],
        &streaminfo(true, 64),
        &flac_frame_numbered(0, 64, &constant),
        &flac_frame_numbered(50_000, 64, &constant),
    ]
    .concat();
    let (sample, _, _, largest) = measure(|| decode(&bytes).unwrap());
    assert_eq!(sample.frames(), 50_001 * 64);
    assert!(
        largest <= 4 * sample.frames() + 64,
        "largest allocation {largest} bytes for {} frames",
        sample.frames()
    );
}

#[test]
fn reader_hits_samples_exactly_and_interpolates_a_ramp() {
    let ramp = SampleData {
        sample_rate: 44_100,
        left: (0..8).map(|i| i as f32).collect(),
        right: None,
        ..Default::default()
    };
    for i in 0..8 {
        assert_eq!(ramp.read(i as f64), (i as f32, i as f32));
    }
    for pos in [1.25, 2.5, 5.75] {
        let (l, r) = ramp.read(pos);
        assert!((f64::from(l) - pos).abs() < 1e-6 && l == r, "{pos}: {l}");
    }
    let (edge, _) = ramp.read(6.5);
    assert!(
        edge > 6.0 && edge < 7.0,
        "the clamped right neighbour keeps it inside: {edge}"
    );
    assert_eq!(ramp.read(-0.01), (0.0, 0.0));
    assert_eq!(ramp.read(8.0), (0.0, 0.0));
    assert_eq!(ramp.read(f64::NAN), (0.0, 0.0));
}

#[test]
fn reader_returns_both_channels() {
    let sample = SampleData {
        sample_rate: 48_000,
        left: Arc::from([0.5f32, -0.5].as_slice()),
        right: Some(Arc::from([0.25f32, 1.0].as_slice())),
        ..Default::default()
    };
    assert_eq!(sample.read(0.0), (0.5, 0.25));
    assert_eq!(sample.read(1.0), (-0.5, 1.0));
    let empty = SampleData {
        sample_rate: 48_000,
        left: Arc::from([].as_slice()),
        right: None,
        ..Default::default()
    };
    assert_eq!(empty.read(0.0), (0.0, 0.0));
}

#[test]
fn reader_pitch_follows_rates_and_semitones() {
    let sine: Arc<[f32]> = (0..44_100)
        .map(|i| (TAU * 441.0 * i as f64 / 44_100.0).sin() as f32)
        .collect();
    let sample = SampleData {
        sample_rate: 44_100,
        left: sine,
        right: None,
        ..Default::default()
    };
    for (device_rate, semitones) in [
        (48_000.0f32, 12.0f32),
        (48_000.0, -12.0),
        (44_100.0, 7.0),
        (96_000.0, 0.0),
    ] {
        let step = sample.step(device_rate, semitones);
        let expected_hz = 441.0 * 2f64.powf(f64::from(semitones) / 12.0);
        assert!((step - 44_100.0 / f64::from(device_rate) * expected_hz / 441.0).abs() < 1e-9);
        let out: Vec<f32> = (0..8192).map(|n| sample.read(n as f64 * step).0).collect();
        let expected_period = f64::from(device_rate) / expected_hz;
        let lag = |f: f64| (expected_period * f) as usize;
        let measured = period(&out, lag(0.7), lag(1.3));
        assert!(
            (measured - expected_period).abs() < expected_period * 0.002,
            "{device_rate} Hz, {semitones} st: period {measured}, expected {expected_period}"
        );
    }
}

/// The lag in `min..=max` with the highest normalised autocorrelation, refined by a parabola through its
/// neighbours.
fn period(x: &[f32], min: usize, max: usize) -> f64 {
    let corr = |lag: usize| {
        let n = x.len() - lag;
        let dot: f64 = (0..n)
            .map(|i| f64::from(x[i]) * f64::from(x[i + lag]))
            .sum();
        dot / n as f64
    };
    let best = (min..=max)
        .max_by(|&a, &b| corr(a).total_cmp(&corr(b)))
        .unwrap();
    let (a, b, c) = (corr(best - 1), corr(best), corr(best + 1));
    best as f64 + 0.5 * (a - c) / (a - 2.0 * b + c)
}

// FLAC `FLSM` metadata: an APPLICATION block holding `fLmL` (tempo, beats, loop) and
// `fLsF` (embedded RIFF chunks, of which `cue ` is read). Every block below is built here.

/// A FLAC metadata block: type, last flag, u24 BE length, `data`.
fn flac_block(kind: u8, last: bool, data: &[u8]) -> Vec<u8> {
    let length = (data.len() as u32).to_be_bytes();
    [&[kind | u8::from(last) << 7][..], &length[1..], data].concat()
}

/// An APPLICATION block with ID `FLSM` holding `chunks`.
fn flsm_block(chunks: &[Vec<u8>]) -> Vec<u8> {
    flac_block(2, false, &[&b"FLSM"[..], &chunks.concat()].concat())
}

/// An FLSM chunk: id, u32 LE length, payload, no padding.
fn flsm_chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    [&id[..], &(payload.len() as u32).to_le_bytes(), payload].concat()
}

/// `fLmL`: f32 tempo, u16 beats, u32 loop start and end, u8 root note and a zero byte.
fn fl_ml(tempo: f32, beats: u16, start: u32, end: u32, root: u8) -> Vec<u8> {
    let payload = [
        &tempo.to_le_bytes()[..],
        &beats.to_le_bytes(),
        &start.to_le_bytes(),
        &end.to_le_bytes(),
        &[root, 0],
    ]
    .concat();
    flsm_chunk(b"fLmL", &payload)
}

/// `fLsF`: embedded RIFF chunks, padded to even as a WAV's are.
fn fl_sf(riff_chunks: &[Vec<u8>]) -> Vec<u8> {
    flsm_chunk(b"fLsF", &riff_chunks.concat())
}

/// `FLAC_SINE` with `blocks` between STREAMINFO and its VORBIS_COMMENT.
fn flac_with(blocks: &[Vec<u8>]) -> Vec<u8> {
    [&FLAC_SINE[..42], &blocks.concat(), &FLAC_SINE[42..]].concat()
}

/// The same tone as Ogg FLAC: the identification packet, then `blocks` and the comment as header packets of one page,
/// then the two frames.
fn ogg_flac_with(blocks: &[Vec<u8>]) -> Vec<u8> {
    let count = (blocks.len() as u16 + 1).to_be_bytes();
    let head = [&b"\x7FFLAC\x01\x00"[..], &count, &FLAC_SINE[..42]].concat();
    let mut headers: Vec<&[u8]> = blocks.iter().map(Vec::as_slice).collect();
    headers.push(&FLAC_SINE[42..86]);
    [
        ogg_page(2, 0, 0, &[&head]),
        ogg_page(0, 0, 1, &headers),
        ogg_page(0, 4410, 2, &[&FLAC_SINE[86..3135], &FLAC_SINE[3135..]]),
    ]
    .concat()
}

/// Every FLAC path with `blocks`: plain, behind an ID3v2 tag, Ogg FLAC, and Ogg FLAC behind a tag.
fn flac_paths(blocks: &[Vec<u8>]) -> [(&'static str, Vec<u8>); 4] {
    let tag = id3v2(b"TIT2", false);
    let (plain, ogg) = (flac_with(blocks), ogg_flac_with(blocks));
    [
        ("plain", plain.clone()),
        ("id3", [tag.clone(), plain].concat()),
        ("ogg", ogg.clone()),
        ("id3 ogg", [tag, ogg].concat()),
    ]
}

/// The tone's meta from every FLAC path, which must agree.
fn flsm_meta(blocks: &[Vec<u8>]) -> SampleMeta {
    let metas: Vec<_> = flac_paths(blocks)
        .map(|(name, bytes)| {
            (
                name,
                decode(&bytes)
                    .unwrap_or_else(|e| panic!("{name}: {e:?}"))
                    .meta,
            )
        })
        .into();
    assert!(
        metas.iter().all(|(_, meta)| *meta == metas[0].1),
        "{metas:?}"
    );
    metas[0].1.clone()
}

#[test]
fn flsm_tempo_beats_loop_and_cues_load_from_every_flac_path() {
    let block = flsm_block(&[
        fl_ml(128.5, 8, 100, 4000, 60),
        fl_sf(&[
            smpl(57, Some((1, 2))),
            acid(3, 90.0),
            chunk(b"LIST", b"odd"),
            cue(&[0, 2205, 6615]),
        ]),
    ]);
    let clean = decode(FLAC_SINE).unwrap();
    for (name, bytes) in flac_paths(&[block]) {
        let sample = decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        // The root note is ignored, and smpl/acid inside fLsF are not read.
        let want = SampleMeta {
            unity_note: None,
            loop_frames: Some((100, 4000)),
            beats: Some(8),
            tempo: Some(128.5),
            cues: vec![0, 2205, 6615],
        };
        assert_eq!(sample.meta, want, "{name}");
        let audio = SampleData {
            meta: SampleMeta::default(),
            ..sample
        };
        assert_eq!(audio, clean, "{name}: the audio is untouched");
    }
}

/// The loop is fitted to the decoded length in this order: start ≥ length → 0, end > length → length, and 0/0
/// is no loop. Nothing is added or taken off.
#[test]
fn flsm_loop_is_fitted_to_the_length_and_zero_zero_is_no_loop() {
    // FLAC_SINE is 4410 frames.
    for (stored, want) in [
        ((100, 4000), Some((100, 4000))),
        ((0, 4410), Some((0, 4410))),
        ((4409, 4410), Some((4409, 4410))),
        ((0, 4411), Some((0, 4410))),
        ((4410, 100), Some((0, 100))),
        ((4410, 99_999), Some((0, 4410))),
        ((u32::MAX, u32::MAX), Some((0, 4410))),
        ((7, 7), Some((7, 7))),
        ((50, 10), Some((50, 10))),
        ((0, 0), None),
        ((9999, 0), None),
    ] {
        let block = flsm_block(&[fl_ml(0.0, 0, stored.0, stored.1, 0xFF)]);
        let meta = flsm_meta(&[block]);
        assert_eq!(meta.loop_frames, want, "stored {stored:?}");
        // Tempo 0 is no tempo; beats 0 is stored; the root note 0xFF is not a unity note.
        assert_eq!(
            (meta.tempo, meta.beats, meta.unity_note),
            (None, Some(0), None)
        );
    }
}

#[test]
fn flsm_reads_only_its_own_blocks_and_chunks() {
    let ml = fl_ml(120.0, 4, 1, 2, 60);
    let ml_values = SampleMeta {
        loop_frames: Some((1, 2)),
        beats: Some(4),
        tempo: Some(120.0),
        ..Default::default()
    };
    let one = |chunks: &[Vec<u8>]| flsm_meta(&[flsm_block(chunks)]);
    assert_eq!(one(std::slice::from_ref(&ml)), ml_values);

    // Another application ID, other block types, data that does not start with the ID, or too short for it.
    let payload = [&b"FLSM"[..], &ml].concat();
    for (what, block) in [
        (
            "another ID",
            flac_block(2, false, &[&b"FLsM"[..], &ml].concat()),
        ),
        ("VORBIS_COMMENT", flac_block(4, false, &payload)),
        ("PADDING", flac_block(1, false, &payload)),
        ("CUESHEET", flac_block(5, false, &payload)),
        ("no ID", flac_block(2, false, &ml)),
        ("short ID", flac_block(2, false, b"FLS")),
        ("empty", flac_block(2, false, b"")),
    ] {
        assert_eq!(flsm_meta(&[block]), SampleMeta::default(), "{what}");
    }

    // fLmL must be 16 bytes: another length is skipped and parsing goes on.
    let sized = |length: usize| flsm_chunk(b"fLmL", &vec![0xAA; length]);
    assert_eq!(
        one(&[sized(15), sized(17), sized(100), ml.clone()]),
        ml_values
    );
    // A zero length stops parsing, so does a chunk that would overrun the block.
    assert_eq!(one(&[sized(0), ml.clone()]), SampleMeta::default());
    let mut cut = ml.clone();
    cut.pop();
    assert_eq!(one(&[cut.clone()]), SampleMeta::default());
    let cues = fl_sf(&[cue(&[7])]);
    let cue_only = SampleMeta {
        cues: vec![7],
        ..Default::default()
    };
    assert_eq!(one(&[cues.clone(), cut]), cue_only);
    // No padding between the chunks: an odd-length chunk is followed by the next one at once.
    assert_eq!(one(&[flsm_chunk(b"JUNK", b"abc"), ml.clone()]), ml_values);

    // The first of each kind wins, inside a block and across blocks, and fLmL and fLsF fill different fields.
    let other = fl_ml(90.0, 2, 5, 6, 1);
    assert_eq!(one(&[ml.clone(), other.clone()]), ml_values);
    let other_cues = fl_sf(&[cue(&[1, 2])]);
    let both = SampleMeta {
        cues: vec![7],
        ..ml_values.clone()
    };
    assert_eq!(one(&[cues.clone(), ml.clone(), other_cues.clone()]), both);
    assert_eq!(
        flsm_meta(&[
            flsm_block(std::slice::from_ref(&cues)),
            flsm_block(&[ml.clone(), other])
        ]),
        both
    );
    let others = [fl_sf(&[acid(1, 60.0)]), fl_sf(&[smpl(60, Some((1, 2)))])];
    assert_eq!(one(&[others[0].clone(), others[1].clone(), cues]), cue_only);

    // A tempo that is not finite and positive is none; the rest of fLmL stays.
    for tempo in [f32::NAN, f32::INFINITY, -120.0, 0.0] {
        let meta = one(&[fl_ml(tempo, 4, 1, 2, 60)]);
        assert_eq!(
            meta,
            SampleMeta {
                tempo: None,
                ..ml_values.clone()
            },
            "{tempo}"
        );
    }

    // The cue count is a claim: the points present are read. fLsF's own chunks follow the WAV rules (cut short keeps
    // what is there).
    let mut lying = cue(&[7]);
    lying[8..12].copy_from_slice(&1000u32.to_le_bytes());
    assert_eq!(one(&[fl_sf(&[lying.clone()])]), cue_only);
    let mut cut_cue = cue(&[7, 8]);
    cut_cue.truncate(cut_cue.len() - 14); // one whole point and part of another
    assert_eq!(one(&[fl_sf(&[cut_cue])]), cue_only);
    assert_eq!(one(&[fl_sf(&[chunk(b"LIST", b"abc"), lying])]), cue_only);
}

/// The block with the last-block flag can be the FLSM block.
#[test]
fn flsm_in_the_last_block_is_read() {
    let ml = fl_ml(120.0, 4, 1, 2, 60);
    let block = flac_block(2, true, &[&b"FLSM"[..], &ml].concat());
    // STREAMINFO, then only the FLSM block, then the frames (the tone's comment block is dropped).
    let bytes = [&FLAC_SINE[..42], &block, &FLAC_SINE[86..]].concat();
    let sample = decode(&bytes).unwrap();
    assert_eq!(sample.frames(), 4410);
    assert_eq!(sample.meta.loop_frames, Some((1, 2)));
    assert_eq!(sample.meta.tempo, Some(120.0));
}

/// Blocks and chunks that claim far more than they hold are read for what is there: nothing large is allocated, and
/// nothing panics.
#[test]
fn hostile_flsm_blocks_are_bounded() {
    let ml = fl_ml(120.0, 4, 1, 2, 60);
    // Chunk lengths up to u32::MAX, and one byte past what is there.
    for claim in [u32::MAX, u32::MAX - 7, 0x8000_0000, 1 << 24, 17] {
        let hostile = [&b"fLmL"[..], &claim.to_le_bytes(), &[0; 16]].concat();
        let (meta, _, _, largest) = measure(|| flsm_meta(&[flsm_block(&[hostile, ml.clone()])]));
        assert_eq!(meta, SampleMeta::default(), "claim {claim}");
        assert!(
            largest < 1 << 20,
            "claim {claim}: largest allocation {largest} bytes"
        );
    }
    // A cue count of u32::MAX over three points, and an fLsF chunk claiming u32::MAX.
    let mut many = cue(&[1, 2, 3]);
    many[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    let (meta, _, _, largest) = measure(|| flsm_meta(&[flsm_block(&[fl_sf(&[many])])]));
    assert_eq!(meta.cues, [1, 2, 3]);
    assert!(largest < 1 << 20, "{largest}");
    let claim = [&b"fLsF"[..], &u32::MAX.to_le_bytes(), &cue(&[1])].concat();
    assert_eq!(flsm_meta(&[flsm_block(&[claim])]), SampleMeta::default());
    let nested = fl_sf(&[[&b"cue "[..], &u32::MAX.to_le_bytes(), &[0; 4]].concat()]);
    assert_eq!(flsm_meta(&[flsm_block(&[nested])]).cues, Vec::<u32>::new());

    // A cue list is bounded by the bytes it comes with.
    let positions: Vec<u32> = (0..100_000).collect();
    let bytes = flac_with(&[flsm_block(&[fl_sf(&[cue(&positions)])])]);
    let (sample, _, _, largest) = measure(|| decode(&bytes).unwrap());
    assert_eq!(sample.meta.cues, positions);
    assert!(largest < 1 << 20, "{largest}");

    // A block header claiming 16 MiB swallows the rest of a short file; an Ogg packet shorter than its header or than
    // its claim is not read.
    let claiming = [
        &FLAC_SINE[..42],
        &[2, 0xFF, 0xFF, 0xFF],
        b"FLSM",
        &ml,
        &FLAC_SINE[42..],
    ]
    .concat();
    let (result, _, _, largest) = measure(|| decode(&claiming));
    assert!(matches!(result, Err(DecodeError::Malformed(m)) if !m.ends_with("panicked")));
    assert!(largest < 1 << 20, "{largest}");
    for packet in [
        vec![2],
        vec![2, 0, 0],
        vec![2, 0xFF, 0xFF, 0xFF],
        [&[2, 0xFF, 0xFF, 0xFF][..], b"FLSM", &ml].concat(),
        [&[2, 0, 0, 40][..], b"FLSM", &ml].concat(),
    ] {
        let sample = decode(&ogg_flac_with(std::slice::from_ref(&packet))).unwrap();
        assert_eq!(sample.meta, SampleMeta::default(), "{packet:?}");
        assert_eq!(sample.frames(), 4410);
    }
}

/// Damaged FLSM data, in a block whose header is rebuilt to fit it, reads the same on every FLAC path, never panics,
/// leaves the audio alone, and fits its loop to the length.
#[test]
fn flsm_fuzz_reads_alike_on_every_flac_path_and_never_panics() {
    let mut next = xorshift(0xF15F_15F1_5F15_F15F);
    let rounds: usize = std::env::var("SAMPLE_FUZZ_ROUNDS").map_or(3000, |n| n.parse().unwrap());
    let seed = [
        fl_ml(128.5, 8, 100, 4000, 60),
        fl_sf(&[
            smpl(57, Some((1, 2))),
            chunk(b"LIST", b"odd"),
            cue(&[0, 2205, 6615]),
        ]),
        flsm_chunk(b"JUNK", b"abc"),
        fl_ml(90.0, 2, 5, 6, 1),
    ]
    .concat();
    let clean = decode(FLAC_SINE).unwrap();
    let (mut with_loop, mut with_cues, mut with_tempo, mut none) = (0, 0, 0, 0);
    for round in 0..rounds {
        let mut data = [&b"FLSM"[..], &seed].concat();
        // One round in four may damage the ID too.
        mutate(&mut data, if round % 4 == 0 { 0 } else { 4 }, &mut next);
        let block = flac_block(2, false, &data);
        let mut first: Option<SampleData> = None;
        for (name, bytes) in flac_paths(&[block]) {
            let sample = decode(&bytes).unwrap_or_else(|e| panic!("round {round} {name}: {e:?}"));
            let audio = SampleData {
                meta: SampleMeta::default(),
                ..sample.clone()
            };
            assert_eq!(audio, clean, "round {round} {name}");
            assert_eq!(
                first.get_or_insert(sample.clone()),
                &sample,
                "round {round} {name}"
            );
        }
        let meta = first.unwrap().meta;
        assert_eq!(meta.unity_note, None, "round {round}");
        assert!(
            meta.tempo.is_none_or(|t| t.is_finite() && t > 0.0),
            "round {round}"
        );
        assert!(meta.cues.len() <= data.len() / 24, "round {round}");
        if let Some((start, end)) = meta.loop_frames {
            assert!(
                start < 4410 && end <= 4410 && (start, end) != (0, 0),
                "round {round}: {start} {end}"
            );
            with_loop += 1;
        }
        with_cues += usize::from(!meta.cues.is_empty());
        with_tempo += usize::from(meta.tempo.is_some());
        none += usize::from(meta == SampleMeta::default());
    }
    eprintln!(
        "flsm fuzz: loop {with_loop}, cues {with_cues}, tempo {with_tempo}, nothing {none} of {rounds}"
    );
    assert!(
        [with_loop, with_cues, with_tempo, none]
            .iter()
            .all(|&n| n > rounds / 20),
        "the mutations must reach each field and leave some blocks unread"
    );
}

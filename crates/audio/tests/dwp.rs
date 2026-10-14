//! The `.dwp` reader and the DW Sampler's `CSTM`. The input is hostile: nothing
//! panics and what is kept grows at most linearly with the input.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use honeydew_audio::dwp::{self, Cstm, Midi, Zone};
use honeydew_audio::sample::MAX_FRAMES;

/// The FLAC fixture: 4410 frames at 44.1 kHz, L = 0.5·sin(441 Hz), R = 0.5·sin(882 Hz).
const FLAC_SINE: &[u8] = include_bytes!("fixtures/sine_flac.wav");

struct Counting;

thread_local! {
    /// While counting, the bytes this thread has asked the allocator for (allocations and growth).
    static BYTES: Cell<Option<usize>> = const { Cell::new(None) };
}

fn count(bytes: usize) {
    let _ = BYTES.try_with(|n| n.set(n.get().map(|n| n + bytes)));
}

// SAFETY: every call forwards to `System` unchanged; counting touches only a const thread-local.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The bytes `f` asks the allocator for on this thread.
fn bytes_during<T>(f: impl FnOnce() -> T) -> (T, usize) {
    BYTES.with(|n| n.set(Some(0)));
    let value = f();
    (value, BYTES.with(|n| n.take()).unwrap())
}

fn record(tag: u32, data: &[u8]) -> Vec<u8> {
    [
        &tag.to_le_bytes()[..],
        &(data.len() as u64).to_le_bytes(),
        data,
    ]
    .concat()
}

fn program(records: &[Vec<u8>]) -> Vec<u8> {
    let top = [record(6, &[0; 16]), record(1, &records.concat())].concat();
    [&b"DwPr"[..], &38u32.to_le_bytes(), &top].concat()
}

/// 0x1F4: keys and velocities, gain, pan and the lead-in word P.
fn keys(root: u8, keys: (u8, u8), vels: (u8, u8), gain: f32, p: u32) -> Vec<u8> {
    let mut data = vec![root, keys.0, keys.1, vels.0, vels.1, 9, 9, 9, 9];
    data.extend(gain.to_le_bytes());
    data.extend(0.25f32.to_le_bytes());
    data.extend(77u32.to_le_bytes());
    data.extend(p.to_le_bytes());
    record(0x1F4, &data)
}

/// 0x1F7: frames, channels, format, rate, loop mode, start and end, then the ignored word.
fn format(frames: u64, channels: u32, rate: f32, looping: (i32, u32, u32)) -> Vec<u8> {
    let mut data = frames.to_le_bytes().to_vec();
    data.extend(channels.to_le_bytes());
    data.extend(16u32.to_le_bytes());
    data.extend(rate.to_le_bytes());
    data.extend(looping.0.to_le_bytes());
    data.extend(looping.1.to_le_bytes());
    data.extend(looping.2.to_le_bytes());
    data.extend([0; 8]);
    record(0x1F7, &data)
}

fn floats(values: [f32; 4]) -> Vec<u8> {
    record(0x1FD, &values.map(f32::to_le_bytes).concat())
}

fn zone(records: &[Vec<u8>]) -> Vec<u8> {
    record(3, &[records.concat(), record(4, &[])].concat())
}

fn pcm16(samples: &[i16]) -> Vec<u8> {
    record(
        0x205,
        &samples
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect::<Vec<_>>(),
    )
}

fn chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    [&id[..], &(payload.len() as u32).to_le_bytes(), payload].concat()
}

/// A `ZONE` payload (1065 bytes) for `path`.
fn cstm_zone(
    path: &[u8],
    root: u8,
    gain: f32,
    looping: (i32, u32, u32),
    floats: [f32; 4],
) -> Vec<u8> {
    let mut data = path.to_vec();
    data.resize(1024, 0);
    data.extend([root, 10, 90, 5, 120]);
    data.extend(gain.to_le_bytes());
    data.extend(0.5f32.to_le_bytes());
    data.extend(looping.0.to_le_bytes());
    data.extend(looping.1.to_le_bytes());
    data.extend(looping.2.to_le_bytes());
    data.extend(floats.map(f32::to_le_bytes).concat());
    chunk(b"ZONE", &data)
}

#[test]
fn zone_defaults_are_the_round_9_ones() {
    let zone = Zone::default();
    assert_eq!(
        (
            zone.root,
            zone.key_low,
            zone.key_high,
            zone.vel_low,
            zone.vel_high
        ),
        (48, 0, 128, 0, 127)
    );
    assert_eq!((zone.gain, zone.pan, zone.loop_mode), (1.0, 0.0, 0));
    assert_eq!(zone.floats, [0.0, 1.0, 1.0, 0.1]);
    // A user sample: one zone over all keys at root 48 that loops over the file's smpl loop (start = end).
    let user = Zone::user_sample();
    assert_eq!((user.root, user.key_low, user.key_high), (48, 0, 128));
    assert!(user.loop_mode > 1 && user.loop_start == user.loop_end);
    assert_eq!(Zone::default().attack_release(), (0.0, 0.1));
}

#[test]
fn zones_come_from_tag_3_up_to_tag_2() {
    let ignored = [
        record(0x64, &[1; 30]),
        record(0x66, b"Keys"),
        record(0x67, b"C:\\old\\Keys.dwp"),
        record(0x68, &[0; 8]),
        record(0x6E, &[2; 12]),
    ];
    let file = zone(&[
        keys(60, (55, 70), (10, 100), 0.5, 0),
        record(0x1F5, b"Piano C4"),
        record(0x1F6, b"%ILSharedData%\\Pianos\\Piano C4.wav\0junk"),
        format(1000, 2, 44_100.0, (2, 10, 900)),
        record(0x1F8, &[5; 8]),
        floats([0.3, 0.0, 1.0, 0.6]),
        record(0x204, &[1; 8]),
    ]);
    let embedded = zone(&[format(4, 1, 22_050.0, (0, 0, 0)), pcm16(&[1, 2, 3, 4])]);
    let after_end = zone(&[record(0x1F6, b"late.wav")]);
    let mut records = ignored.to_vec();
    records.extend([file, embedded, record(2, &[]), after_end]);
    let bytes = program(&records);
    let zones = dwp::read(&bytes).expect("a program");
    assert_eq!(zones.len(), 2, "tag 0x02 ends the list");

    let piano = &zones[0];
    assert_eq!(piano.file.as_deref(), Some("Piano C4.wav"));
    assert!(!piano.embedded());
    assert_eq!(piano.decode(), None);
    let z = piano.zone;
    assert_eq!(
        (z.root, z.key_low, z.key_high, z.vel_low, z.vel_high),
        (60, 55, 70, 10, 100)
    );
    assert_eq!((z.gain, z.pan), (0.5, 0.25));
    assert_eq!((z.loop_mode, z.loop_start, z.loop_end), (2, 10, 900));
    assert_eq!(z.floats, [0.3, 0.0, 1.0, 0.6]);
    assert_eq!(z.attack_release(), (0.3, 0.6));

    // No 0x1F4: the zone defaults.
    assert_eq!(zones[1].zone.root, 48);
    assert!(zones[1].embedded() && zones[1].file.is_none());
}

#[test]
fn not_a_program_reads_as_none() {
    assert!(dwp::read(b"").is_none());
    assert!(dwp::read(b"RIFF\0\0\0\0WAVE").is_none());
    assert_eq!(dwp::read(b"DwPr\x26\0\0\0").map(|z| z.len()), Some(0));
}

#[test]
fn embedded_pcm_is_16_bit_below_4_bytes_per_sample_else_float() {
    // 16-bit, mono, a lead-in of P/2 = 2 frames, then 0x1F7's 6 frames: the last 2 bytes' frame is cut.
    let samples: Vec<i16> = (0..10).map(|i| i * 1000).collect();
    let mono = zone(&[
        keys(48, (0, 127), (0, 127), 1.0, 4),
        format(6, 1, 32_000.0, (0, 0, 0)),
        pcm16(&samples),
    ]);
    // 16-bit stereo, interleaved.
    let stereo = zone(&[
        format(2, 2, 44_100.0, (0, 0, 0)),
        pcm16(&[100, -100, 200, -200]),
    ]);
    // Float stereo: 32 bytes for 4 frames · 2 channels is 4 bytes per sample.
    let data: Vec<u8> = [0.5f32, -0.5, 0.25, -0.25, f32::NAN, 1.0, 0.0, 0.0]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    let float = zone(&[format(4, 2, 48_000.0, (0, 0, 0)), record(0x205, &data)]);
    let bytes = program(&[mono, stereo, float]);
    let zones = dwp::read(&bytes).unwrap();

    let mono = zones[0].decode().unwrap();
    assert_eq!(mono.sample_rate, 32_000);
    assert_eq!(mono.right, None);
    let expected: Vec<f32> = (2..8).map(|i| (i * 1000) as f32 / 32768.0).collect();
    assert_eq!(&mono.left[..], &expected[..]);

    let stereo = zones[1].decode().unwrap();
    assert_eq!(&stereo.left[..], &[100.0 / 32768.0, 200.0 / 32768.0]);
    assert_eq!(
        stereo.right.as_deref(),
        Some(&[-100.0 / 32768.0, -200.0 / 32768.0][..])
    );

    let float = zones[2].decode().unwrap();
    assert_eq!(
        &float.left[..],
        &[0.5, 0.25, 0.0, 0.0],
        "NaN reads as silence"
    );
    assert_eq!(float.right.as_deref(), Some(&[-0.5, -0.25, 1.0, 0.0][..]));
}

#[test]
fn embedded_float_pcm_is_held_to_16() {
    let data: Vec<u8> = [1.6e38f32, -1e30, 0.5, f32::INFINITY]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    let records = [zone(&[
        format(4, 1, 48_000.0, (0, 0, 0)),
        record(0x205, &data),
    ])];
    let bytes = program(&records);
    let zones = dwp::read(&bytes).unwrap();
    assert_eq!(
        &zones[0].decode().unwrap().left[..],
        &[16.0, -16.0, 0.5, 0.0]
    );
}

#[test]
fn a_decode_limit_bounds_embedded_audio() {
    let samples: Vec<i16> = (0..10).collect();
    let records = [
        zone(&[format(10, 1, 48_000.0, (0, 0, 0)), pcm16(&samples)]),
        zone(&[
            format(4410, 2, 44_100.0, (0, 0, 0)),
            record(0x206, FLAC_SINE),
        ]),
    ];
    let bytes = program(&records);
    let zones = dwp::read(&bytes).unwrap();
    assert_eq!(zones[0].decode_with_limit(10).unwrap().frames(), 10);
    assert!(
        zones[0].decode_with_limit(9).is_none(),
        "PCM over its limit too"
    );
    assert_eq!(zones[1].decode_with_limit(4410).unwrap().frames(), 4410);
    assert!(
        zones[1].decode_with_limit(4000).is_none(),
        "FLAC over its limit is an error"
    );
}

#[test]
fn embedded_flac_decodes_with_or_without_its_length_word() {
    let bare = zone(&[
        format(4410, 2, 44_100.0, (0, 0, 0)),
        record(0x206, FLAC_SINE),
    ]);
    let prefixed = [&(FLAC_SINE.len() as u64).to_le_bytes()[..], FLAC_SINE].concat();
    let worded = zone(&[
        keys(48, (0, 128), (0, 127), 1.0, 20),
        record(0x206, &prefixed),
    ]);
    let bytes = program(&[bare, worded]);
    let zones = dwp::read(&bytes).unwrap();
    let whole = zones[0].decode().unwrap();
    assert_eq!((whole.sample_rate, whole.frames()), (44_100, 4410));
    assert!(whole.right.is_some());
    // L = 0.5·sin(2π·441·n/44100): a quarter period in (25 frames) it peaks.
    assert!((whole.left[25] - 0.5).abs() < 1e-3, "{}", whole.left[25]);
    // P = 20: ten lead-in frames skipped.
    let skipped = zones[1].decode().unwrap();
    assert_eq!(skipped.frames(), 4400);
    assert_eq!(skipped.left[15], whole.left[25]);
}

#[test]
fn a_bad_format_record_drops_the_zone() {
    let bad = [
        format(0, 1, 44_100.0, (0, 0, 0)),
        format(1 << 32, 1, 44_100.0, (0, 0, 0)),
        format(10, 0, 44_100.0, (0, 0, 0)),
        format(10, 3, 44_100.0, (0, 0, 0)),
        record(0x1F7, &[1, 0, 0]),
    ];
    for format in bad {
        let records = [zone(&[format, record(0x1F6, b"a.wav")])];
        assert_eq!(dwp::read(&program(&records)).unwrap().len(), 0);
    }
    let good = [zone(&[
        format((1 << 32) - 1, 2, 44_100.0, (0, 0, 0)),
        record(0x1F6, b"a.wav"),
    ])];
    assert_eq!(dwp::read(&program(&good)).unwrap().len(), 1);
    // A zone with no sample at all (no file name, no audio) cannot play.
    let silent = [
        zone(&[keys(60, (0, 127), (0, 127), 1.0, 0)]),
        zone(&[record(0x1F6, b"dir\\..")]),
    ];
    assert_eq!(dwp::read(&program(&silent)).unwrap().len(), 0);
}

#[test]
fn cstm_reads_the_program_zones_and_midi() {
    // No CSTM (or not a DirectWave one): a new module's PB range 2 and flag 0.
    let none = Cstm::read(&[]);
    assert_eq!(none.midi, Midi::default());
    assert_eq!((none.midi.pb_range, none.midi.attack_flag), (2, 0));
    assert!(none.program.is_none() && none.zones.is_empty());
    // Any CSTM first sets 4 and 1; MIDI then overrides; a 1-byte MIDI sets only the range.
    assert_eq!(
        Cstm::read(b"10WD").midi,
        Midi {
            pb_range: 4,
            attack_flag: 1
        }
    );
    let one = [&b"10WD"[..], &chunk(b"MIDI", &[7])].concat();
    assert_eq!(
        Cstm::read(&one).midi,
        Midi {
            pb_range: 7,
            attack_flag: 1
        }
    );
    let two = [&b"10WD"[..], &chunk(b"MIDI", &[12, 0])].concat();
    assert_eq!(
        Cstm::read(&two).midi,
        Midi {
            pb_range: 12,
            attack_flag: 0
        }
    );
    // A stored range outside 1…12 is held to it, as a live edit is.
    let low = [&b"10WD"[..], &chunk(b"MIDI", &[0])].concat();
    assert_eq!(Cstm::read(&low).midi.pb_range, 1);
    let high = [&b"10WD"[..], &chunk(b"MIDI", &[200])].concat();
    assert_eq!(Cstm::read(&high).midi.pb_range, 12);

    let path = b"@L@\0My Programs/Keys.dwp";
    let path_chunk = [
        &(path.len() as u32).to_le_bytes()[..],
        path,
        &0.25f32.to_le_bytes(),
        &0.5f32.to_le_bytes(),
    ]
    .concat();
    let blob = [
        &b"10WD"[..],
        &chunk(b"PATh", &path_chunk),
        &chunk(b"MIDI", &[3, 0]),
    ]
    .concat();
    let cstm = Cstm::read(&blob);
    let program = cstm.program.clone().unwrap();
    assert_eq!(program.path, path);
    assert_eq!(program.init, Some((0.25, 0.5)));
    assert_eq!(cstm.midi.pb_range, 3);
    let blob = [
        &b"10WD"[..],
        &cstm_zone(
            b"@L@\0My Samples/a.wav",
            60,
            0.75,
            (2, 5, 50),
            [0.1, 1.0, 1.0, 0.2],
        ),
        &chunk(b"ZONE", &[0; 1064]),
        &chunk(b"MIDI", &[3, 0]),
    ]
    .concat();
    let cstm = Cstm::read(&blob);
    assert_eq!(cstm.zones.len(), 1, "a short ZONE is skipped");
    let (zone_path, zone) = cstm.zones[0];
    assert_eq!(&zone_path[..20], b"@L@\0My Samples/a.wav");
    assert_eq!(zone_path.len(), 1024);
    assert_eq!(
        (
            zone.root,
            zone.key_low,
            zone.key_high,
            zone.vel_low,
            zone.vel_high
        ),
        (60, 10, 90, 5, 120)
    );
    assert_eq!(
        (
            zone.gain,
            zone.pan,
            zone.loop_mode,
            zone.loop_start,
            zone.loop_end
        ),
        (0.75, 0.5, 2, 5, 50)
    );
    assert_eq!(zone.floats, [0.1, 1.0, 1.0, 0.2]);
    assert_eq!(
        cstm.midi,
        Midi {
            pb_range: 3,
            attack_flag: 0
        }
    );

    // A PATh without its floats has no init values; one cut short in its path is ignored.
    let bare = [&b"10WD"[..], &chunk(b"PATh", &path_chunk[..path.len() + 4])].concat();
    assert_eq!(Cstm::read(&bare).program.unwrap().init, None);
    let cut = [&b"10WD"[..], &chunk(b"PATh", &path_chunk[..10])].concat();
    assert!(Cstm::read(&cut).program.is_none());
}

/// A `PATh` (or with `id` `PATH`) payload: the path as a `vstr32`, then `tail`.
fn path_chunk(id: &[u8; 4], path: &[u8], tail: &[u8]) -> Vec<u8> {
    chunk(
        id,
        &[&(path.len() as u32).to_le_bytes()[..], path, tail].concat(),
    )
}

/// Two `f32`s, as a `PATh` stores Attack and Release.
fn pair(a: f32, r: f32) -> Vec<u8> {
    [a.to_le_bytes(), r.to_le_bytes()].concat()
}

#[test]
fn a_loaded_path_or_zone_rules_out_the_chunks_after_it() {
    // A PATh/PATH is ignored once zones or a path are loaded; a ZONE is ignored after a path.
    let blob = |chunks: &[Vec<u8>]| [b"10WD".to_vec(), chunks.concat()].concat();
    let zone = cstm_zone(b"@L@\0z.wav", 60, 1.0, (0, 0, 0), [0.0, 1.0, 1.0, 0.1]);
    let a = path_chunk(b"PATh", b"@L@\0a.dwp", &pair(0.1, 0.2));
    let b = path_chunk(b"PATh", b"@L@\0b.dwp", &pair(0.3, 0.4));

    // The first path counts, not the last.
    let two = blob(&[a.clone(), b.clone()]);
    assert_eq!(Cstm::read(&two).program.unwrap().path, b"@L@\0a.dwp");
    // A ZONE after a path is ignored.
    let cstm_path_zone = blob(&[a.clone(), zone.clone()]);
    let read = Cstm::read(&cstm_path_zone);
    assert_eq!(read.program.unwrap().path, b"@L@\0a.dwp");
    assert!(read.zones.is_empty(), "the ZONE after the path");
    // A path after a ZONE is ignored; later ZONEs still load.
    let zone_path = blob(&[zone.clone(), a.clone(), zone.clone()]);
    let read = Cstm::read(&zone_path);
    assert!(read.program.is_none(), "the path after a ZONE");
    assert_eq!(read.zones.len(), 2);
    // MIDI applies wherever it is.
    let midi = blob(&[a.clone(), chunk(b"MIDI", &[9, 0]), b.clone()]);
    let read = Cstm::read(&midi);
    assert_eq!((read.midi.pb_range, read.midi.attack_flag), (9, 0));
    assert_eq!(read.program.unwrap().path, b"@L@\0a.dwp");

    // Legacy upper-case PATH loads the same way, without floats even when 8 bytes follow its path.
    let legacy = path_chunk(b"PATH", b"@L@\0old.dwp", &pair(0.5, 0.5));
    let legacy_first = blob(&[legacy.clone(), a.clone()]);
    let program = Cstm::read(&legacy_first).program.unwrap();
    assert_eq!((program.path, program.init), (&b"@L@\0old.dwp"[..], None));
    assert_eq!(
        Cstm::read(&blob(&[a.clone(), legacy.clone()]))
            .program
            .unwrap()
            .path,
        b"@L@\0a.dwp"
    );
    assert!(Cstm::read(&blob(&[zone.clone(), legacy])).program.is_none());

    // A path that is empty or cut short loads nothing, so the next one still counts.
    let empty = path_chunk(b"PATh", b"", &pair(0.9, 0.9));
    let cut = chunk(b"PATh", b"\x40\0\0\0@L@");
    for first in [empty, cut] {
        let then_b = blob(&[first.clone(), b.clone()]);
        assert_eq!(Cstm::read(&then_b).program.unwrap().path, b"@L@\0b.dwp");
        assert_eq!(Cstm::read(&blob(&[first, zone.clone()])).zones.len(), 1);
    }
}

#[test]
fn path_floats_count_only_as_a_pair() {
    // The floats apply only if both are present (≥ 8 bytes after the path).
    let init = |tail: &[u8]| {
        let blob = [&b"10WD"[..], &path_chunk(b"PATh", b"@L@\0p.dwp", tail)].concat();
        Cstm::read(&blob).program.unwrap().init
    };
    let both = pair(0.25, 0.75);
    for short in 0..8 {
        assert_eq!(init(&both[..short]), None, "{short} bytes");
    }
    assert_eq!(init(&both), Some((0.25, 0.75)));
    assert_eq!(init(&[&both[..], b"tail"].concat()), Some((0.25, 0.75)));
    // For hostile input, a pair with a float that is not finite counts as absent.
    assert_eq!(init(&pair(f32::NAN, 0.5)), None);
    assert_eq!(init(&pair(0.5, f32::INFINITY)), None);
    // A hostile length: past the chunk, or u32::MAX.
    for len in [64u32, u32::MAX] {
        let blob = [
            &b"10WD"[..],
            &chunk(b"PATh", &[&len.to_le_bytes()[..], b"@L@\0p"].concat()),
        ]
        .concat();
        assert!(Cstm::read(&blob).program.is_none());
    }
}

#[test]
fn the_program_names_its_chunk_for_the_writer() {
    let zone = cstm_zone(b"@L@\0z.wav", 60, 1.0, (0, 0, 0), [0.0, 1.0, 1.0, 0.1]);
    let a = path_chunk(b"PATh", b"@L@\0a.dwp", &pair(0.1, 0.2));
    let midi = chunk(b"MIDI", &[2, 1]);
    let blob = [&b"10WD"[..], &midi, &a, &a].concat();
    let at = 4 + midi.len();
    assert_eq!(Cstm::read(&blob).program.unwrap().chunk, at..at + a.len());
    assert!(
        Cstm::read(&[&b"10WD"[..], &zone, &a].concat())
            .program
            .is_none()
    );
}

#[test]
fn the_program_loader_takes_a_dwp_then_an_audio_file() {
    // The old instrument format (layout unknown: never taken), then a `.dwp`, whose last zone sets
    // Attack and Release from its floats #1 and #4, then a plain audio file, which sets nothing.
    let two = program(&[
        zone(&[record(0x1F6, b"a.wav"), floats([0.1, 1.0, 1.0, 0.2])]),
        zone(&[record(0x1F6, b"b.wav"), floats([0.3, 0.0, 1.0, 0.4])]),
        record(2, &[]),
    ]);
    assert_eq!(dwp::load_program(&two), Some(Some((0.3, 0.4))));
    // A zone without its 0x1FD has the defaults 0 and 0.1.
    let plain = program(&[zone(&[record(0x1F6, b"a.wav")])]);
    assert_eq!(dwp::load_program(&plain), Some(Some((0.0, 0.1))));
    // A `.dwp` without zones is taken but sets nothing.
    assert_eq!(dwp::load_program(&program(&[])), Some(None));
    assert_eq!(dwp::load_program(FLAC_SINE), Some(None), "an audio file");
    for rejected in [&b""[..], b"DwP", b"RIFF\0\0\0\0WAVE", b"garbage bytes"] {
        assert_eq!(dwp::load_program(rejected), None);
    }
}

#[test]
fn hostile_input_never_panics_and_allocation_stays_linear() {
    let mut rng = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let seeds = [
        program(&[
            record(0x66, b"Seed"),
            zone(&[
                keys(60, (0, 127), (0, 127), 1.0, 8),
                format(16, 2, 44_100.0, (2, 1, 9)),
                pcm16(&[7; 40]),
            ]),
            zone(&[record(0x1F6, b"x\\y.wav"), floats([0.1, 0.2, 0.3, 0.4])]),
            zone(&[
                format(4410, 2, 44_100.0, (0, 0, 0)),
                record(0x206, FLAC_SINE),
            ]),
            record(2, &[]),
        ]),
        [
            &b"10WD"[..],
            &chunk(b"PATh", b"\x09\0\0\0@L@\0a.dwp\0\0\x80\x3f\0\0\0\0"),
            &cstm_zone(b"@L@\0b.wav", 48, 1.0, (2, 0, 0), [0.0, 1.0, 1.0, 0.1]),
            &chunk(b"MIDI", &[2, 1]),
        ]
        .concat(),
    ];
    let rounds: usize = std::env::var("DWP_FUZZ_ROUNDS").map_or(3000, |n| n.parse().unwrap());
    let mut kept = 0;
    for round in 0..rounds {
        let mut bytes = seeds[round % seeds.len()].clone();
        for _ in 0..1 + next() % 8 {
            let at = (next() as usize) % bytes.len().max(1);
            match next() % 5 {
                0 if !bytes.is_empty() => bytes[at] ^= 1 << (next() % 8),
                1 if !bytes.is_empty() => bytes[at] = next() as u8,
                2 => bytes.truncate(at),
                3 => bytes.insert(at.min(bytes.len()), next() as u8),
                _ if bytes.len() >= at + 8 => {
                    bytes[at..at + 8].copy_from_slice(&next().to_le_bytes())
                }
                _ => {}
            }
        }
        let (zones, parse_bytes) = bytes_during(|| dwp::read(&bytes));
        let (cstm, cstm_bytes) = bytes_during(|| Cstm::read(&bytes).zones.len());
        assert!(
            parse_bytes + cstm_bytes <= 32 * bytes.len() + 4096,
            "round {round}: {parse_bytes} + {cstm_bytes} bytes for {} input bytes",
            bytes.len()
        );
        assert!(cstm <= bytes.len() / 1065);
        for zone in zones.iter().flatten() {
            if let Some(sample) = zone.decode() {
                kept += 1;
                assert!(sample.sample_rate > 0 && (1..=MAX_FRAMES).contains(&sample.frames()));
                assert!(
                    sample
                        .right
                        .as_ref()
                        .is_none_or(|r| r.len() == sample.frames())
                );
            }
        }
    }
    assert!(
        kept > 0,
        "no mutated zone decoded: the fuzz never got past the parser"
    );
}

#[test]
fn a_written_program_reads_back() {
    let piano = Zone {
        root: 60,
        key_low: 55,
        key_high: 64,
        vel_low: 1,
        vel_high: 80,
        gain: 0.5,
        pan: -0.25,
        loop_mode: 2,
        loop_start: 100,
        loop_end: 4_000,
        floats: [0.01, 0.0, 1.0, 0.35],
    };
    let high = Zone {
        root: 72,
        floats: [0.02, 1.0, 1.0, 0.6],
        ..Zone::default()
    };
    let bytes = dwp::write(&[
        (piano, "Piano C4 v1.flac", 5_000, true),
        (high, "Piano C5 v1.flac", 1, false),
    ]);
    let zones = dwp::read(&bytes).expect("a program");
    let got: Vec<_> = zones.iter().map(|z| (z.zone, z.file.as_deref())).collect();
    assert_eq!(
        got,
        [
            (piano, Some("Piano C4 v1.flac")),
            (high, Some("Piano C5 v1.flac"))
        ]
    );
    assert!(zones.iter().all(|z| !z.embedded()));
    assert_eq!(dwp::load_program(&bytes), Some(Some((0.02, 0.6))));
}

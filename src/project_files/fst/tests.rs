//! Synthetic `.fst` files built here from the format's event layout: never a real preset.

use std::path::Path;

use super::*;
use crate::song::ModuleInstance;

/// `.fst` of `events` (whole events, from the helpers below), format 0x20, the stated data length the events'.
pub(crate) fn fst(events: &[Vec<u8>]) -> Vec<u8> {
    with_format(0x20, events)
}

fn with_format(format: u16, events: &[Vec<u8>]) -> Vec<u8> {
    let body = events.concat();
    [
        &b"FLhd"[..],
        &6u32.to_le_bytes(),
        &format.to_le_bytes(),
        &[1, 0, 96, 0],
        b"FLdt",
        &(body.len() as u32).to_le_bytes(),
        &body,
    ]
    .concat()
}

/// A blob event (ids 192–255): the id, the length as a varint, the bytes.
pub(crate) fn blob(id: u8, data: &[u8]) -> Vec<u8> {
    let mut out = vec![id];
    let mut n = data.len();
    while n >= 0x80 {
        out.push(n as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
    out.extend(data);
    out
}

fn u8_event(id: u8, value: u8) -> Vec<u8> {
    vec![id, value]
}

fn u32_event(id: u8, value: u32) -> Vec<u8> {
    [&[id][..], &value.to_le_bytes()].concat()
}

pub(crate) fn ints(values: &[i32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// A NUL-terminated string.
fn text(s: &str) -> Vec<u8> {
    [s.as_bytes(), &[0]].concat()
}

/// The FL version event (199).
pub(crate) fn version(v: &str) -> Vec<u8> {
    blob(199, &text(v))
}

/// The custom waveform event (196).
pub(crate) fn shape(path: &str) -> Vec<u8> {
    blob(196, &text(path))
}

fn imported(events: &[Vec<u8>]) -> FstImport {
    import_fst(
        &fst(events),
        Path::new("Synth Presets2/3xOsc/Bass/Gen Bass.fst"),
    )
}

fn defaults() -> Vec<f32> {
    ModuleInstance::new(39, 0).params
}

/// The params `import` changed from their defaults, as (param, value).
fn changed(import: &FstImport) -> Vec<(usize, f32)> {
    let d = defaults();
    (import.params.iter().enumerate())
        .filter(|&(i, v)| *v != d[i])
        .map(|(i, v)| (i, *v))
        .collect()
}

/// The time law written plainly: (raw − 100)/65436.
fn time(raw: i32) -> f32 {
    (raw - 100) as f32 / 65436.0
}

/// `changed` equals `want`, each value within 1e-6.
#[track_caller]
fn same(changed: &[(usize, f32)], want: &[(usize, f32)]) {
    let ok = changed.len() == want.len()
        && (changed.iter().zip(want)).all(|(a, b)| a.0 == b.0 && (a.1 - b.1).abs() < 1e-6);
    assert!(ok, "changed {changed:?}\n   want {want:?}");
}

#[test]
fn the_header_must_be_a_channel_preset_and_events_run_to_the_end_of_the_file() {
    let empty = imported(&[]);
    assert!(empty.ok);
    assert_eq!(empty.params, defaults());
    assert_eq!(empty.params.len(), 149);
    assert!(import_fst(&with_format(0x30, &[]), Path::new("a.fst")).ok);
    // Another format or magic, or a cut header: a failure with every param at its default.
    let bad = |bytes: &[u8]| {
        let import = import_fst(bytes, Path::new("a.fst"));
        assert!(!import.ok);
        assert_eq!(import.params, defaults());
    };
    bad(&with_format(0x10, &[u8_event(16, 4)]));
    bad(&with_format(0, &[]));
    let mut magic = fst(&[]);
    magic[0] = b'X';
    bad(&magic);
    let mut data = fst(&[]);
    data[14] = b'X';
    bad(&data);
    bad(&fst(&[])[..21]);
    bad(b"");
    // The stated lengths are skipped: events past the data length still load.
    let mut long = fst(&[u8_event(16, 4)]);
    long[4..8].copy_from_slice(&0u32.to_le_bytes());
    long[18..22].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        changed(&import_fst(&long, Path::new("a.fst"))),
        [(138, 1.0)]
    );
}

#[test]
fn each_size_class_is_skipped_by_its_size_and_only_the_used_ids_count() {
    let unused = [
        u8_event(0, 1),
        u8_event(21, 2),
        vec![64, 0xFF, 0xFF],
        vec![127, 0xFF, 0xFF],
        u32_event(128, u32::MAX),
        u32_event(139, u32::MAX),
        u32_event(191, u32::MAX),
        // The channel name is ignored: the file names the preset.
        blob(192, &text("Channel name")),
        blob(201, &text("3x Osc")),
        blob(212, &[0xFF; 52]),
        // A two-byte varint length.
        blob(255, &[0xFF; 300]),
    ];
    let import = imported(&unused);
    assert!(import.ok);
    assert_eq!(changed(&import), []);
    assert_eq!(import.name, "Gen Bass");
    // u8 16: bit 1 Ping Pong, bit 2 Fat Mode, each set or cleared.
    let pong = [unused.to_vec(), vec![u8_event(16, 2 | 4)]].concat();
    assert_eq!(changed(&imported(&pong)), [(137, 1.0), (138, 1.0)]);
    assert_eq!(changed(&imported(&[u8_event(16, 4), u8_event(16, 1)])), []);
    // u32 138: Delay Cutoff from the signed high half, Delay Resonance from the low half.
    let delay = |high: i16, low: u16| {
        changed(&imported(&[u32_event(
            138,
            (high as u16 as u32) << 16 | low as u32,
        )]))
    };
    assert_eq!(delay(64, 192), [(135, 0.25), (136, 0.75)]);
    assert_eq!(delay(-64, 0xFFFF), [(135, 0.0), (136, 1.0)]);
}

#[test]
fn the_fl_version_is_three_dotted_numbers_and_fewer_parts_abort() {
    // 220 shows V: its Type is read from 2.5.1 on, and its Resonance converted before 3.2.2.
    let filter = blob(220, &ints(&[128, 80, 7]));
    let at =
        |events: &[Vec<u8>]| changed(&imported(&[events, std::slice::from_ref(&filter)].concat()));
    let converted = [(111, 0.5), (112, 1.0)];
    assert_eq!(at(&[]), converted, "no version yet: V = 0");
    assert_eq!(at(&[version("2.5.0")]), converted);
    assert_eq!(
        at(&[version("2.5.1")]),
        [(110, 1.0), (111, 0.5), (112, 1.0)]
    );
    assert_eq!(
        at(&[version("3.2.1")]),
        [(110, 1.0), (111, 0.5), (112, 1.0)]
    );
    let new = [(110, 1.0), (111, 0.5), (112, 0.3125)];
    assert_eq!(at(&[version("3.2.2")]), new);
    // A fourth part is ignored; the parts are decimal, b and c weigh 1 000 and 1.
    assert_eq!(at(&[version("20.8.4.2576")]), new);
    assert_eq!(at(&[version("3.2002.0")]), new);
    assert_eq!(at(&[version("3.0.2002")]), new);
    // Fewer than three parts abort: what was read stays, nothing after.
    let short = imported(&[u8_event(16, 4), version("3.1"), filter.clone()]);
    assert!(!short.ok);
    assert_eq!(changed(&short), [(138, 1.0)]);
    assert!(!imported(&[version("")]).ok);
}

#[test]
fn a_legacy_osc_blob_holds_shape_coarse_fine_and_the_next_oscs_mix() {
    // shape1, coarse1, fine1, mix2, shape2, coarse2, fine2, mix3, shape3, coarse3, fine3: no version field.
    let raws = [3, -12, 50, 32, 6, 24, -100, 96, 0, 0, 0];
    let import = imported(&[blob(213, &ints(&raws))]);
    assert!(import.ok);
    assert_eq!(
        changed(&import),
        [
            (1, 0.5),
            (2, 1.0),
            (13, 0.25),
            (14, 1.0),
            (15, 0.5),
            (16, 0.75),
            (17, 0.0),
            (22, 0.25),
            (23, 0.75)
        ]
    );
    // Any first value up to 9 is a legacy blob, a negative one too.
    let negative = [&[-6][..], &raws[1..]].concat();
    assert_eq!(imported(&[blob(213, &ints(&negative))]).params[1], 0.0);
    let nine = imported(&[blob(213, &ints(&[&[9][..], &raws[1..]].concat()))]);
    assert!(nine.ok);
    assert_eq!(nine.params[1], 1.0);
    // Shorter than 44 bytes: an abort, nothing written.
    for bytes in [&ints(&raws)[..43], &[3u8, 0, 0][..], &[][..]] {
        let short = imported(&[blob(213, bytes)]);
        assert!(!short.ok);
        assert_eq!(changed(&short), []);
    }
}

/// The 20 osc i32 of a v10+ blob: per osc pan, shape, coarse, fine, phase, detune, then osc 1's and 2's level.
const OSCS: [i32; 20] = [
    64, 2, 12, -50, -64, 25, 32, // osc 1, its level → Osc 2 Mix
    -32, 3, -24, 100, 32, -50, 128, // osc 2, its level → Osc 3 Mix
    32, 6, 0, 0, 0, 50, // osc 3
];

/// What [`OSCS`] sets.
fn oscs_set() -> Vec<(usize, f32)> {
    vec![
        (1, 2.0 / 6.0),
        (2, 0.5),
        (3, 1.0),
        (7, 0.0),
        (8, 0.75),
        (10, 0.75),
        (11, 0.0),
        (12, 1.0),
        (13, 0.75),
        (14, 0.0),
        (15, 0.5),
        (16, 0.25),
        (17, 1.0),
        (19, 1.0),
        (20, 0.25),
        (21, 0.75),
        (22, 0.25),
        (23, 1.0),
    ]
}

fn osc_blob(version: i32, tail: &[u8]) -> Vec<u8> {
    [&ints(&[version])[..], &ints(&OSCS), tail].concat()
}

/// `base` with `extra` merged in, by param.
fn with(mut base: Vec<(usize, f32)>, extra: &[(usize, f32)]) -> Vec<(usize, f32)> {
    for &(p, v) in extra {
        base.retain(|&(q, _)| q != p);
        base.push((p, v));
    }
    base.sort_by_key(|&(p, _)| p);
    base
}

#[test]
fn a_v10_or_v11_osc_blob_holds_20_i32_and_v11_the_invert_bytes() {
    // v10: the invert bytes are never read.
    let v10 = imported(&[blob(213, &osc_blob(10, &[1, 1, 1, 1]))]);
    assert!(v10.ok);
    same(&changed(&v10), &oscs_set());
    // v11: up to 3 bytes, each ≠ 0 turning its osc's Invert on; a 4th is not read.
    let v11 = imported(&[blob(213, &osc_blob(11, &[1, 0, 7, 1]))]);
    same(&changed(&v11), &with(oscs_set(), &[(4, 1.0), (6, 1.0)]));
    let one = imported(&[blob(213, &osc_blob(11, &[1]))]);
    same(&changed(&one), &with(oscs_set(), &[(4, 1.0)]));
    same(
        &changed(&imported(&[blob(213, &osc_blob(11, &[]))])),
        &oscs_set(),
    );
    // Shorter than 84 bytes: an abort, nothing written.
    let cut = &osc_blob(11, &[])[..83];
    let short = imported(&[blob(213, cut)]);
    assert!(!short.ok);
    assert_eq!(changed(&short), []);
}

#[test]
fn a_v12_osc_blob_adds_stereo_phase_randomness_and_four_flag_bytes_and_v14_the_blocks() {
    // #21 Stereo Phase Randomness = raw/64; Invert 1–3, Osc 3 = AM; an optional byte whose bit 0 is HQ.
    let flags = |tail: &[u8]| [&ints(&[48])[..], &[0, 1, 0, 2], tail].concat();
    let set = with(oscs_set(), &[(5, 1.0), (24, 1.0), (26, 0.75)]);
    same(
        &changed(&imported(&[blob(213, &osc_blob(12, &flags(&[])))])),
        &set,
    );
    same(
        &changed(&imported(&[blob(213, &osc_blob(13, &flags(&[3])))])),
        &with(set.clone(), &[(25, 1.0)]),
    );
    same(
        &changed(&imported(&[blob(213, &osc_blob(12, &flags(&[2])))])),
        &set,
    );
    // Shorter than 92 bytes: an abort.
    let cut = osc_blob(12, &flags(&[]));
    assert!(!imported(&[blob(213, &cut[..91])]).ok);
    // v14 without its extra part, or with only some of it: the load goes on without it.
    let partial = imported(&[
        blob(213, &osc_blob(14, &flags(&[1, 0, 0, 0, 9]))),
        u8_event(16, 4),
    ]);
    assert!(partial.ok);
    same(
        &changed(&partial),
        &with(set.clone(), &[(25, 1.0), (138, 1.0)]),
    );
    // v14: 3 bytes skipped, then the five blocks (as 218, blocks 0–4) and #87 cutoff, #88 resonance, #90 type.
    let mut extra: Vec<i32> = (0..5).flat_map(|_| BLOCK).collect();
    extra.extend([1, 2, 128, 64, 3, 7]);
    let v14 = imported(&[blob(
        213,
        &osc_blob(14, &[flags(&[1, 9, 9, 9]), ints(&extra)].concat()),
    )]);
    assert!(v14.ok);
    let p = &v14.params;
    assert_eq!((p[25], p[26]), (1.0, 0.75));
    assert_eq!((p[110], p[111], p[112]), (1.0, 0.5, 0.25));
    // Block 0 is Pan's LFO, block 4 Pitch's (its envelope too).
    assert_eq!((p[37], p[52]), (0.5, 1.0));
    assert_eq!(
        (p[41], p[56], p[65], p[109]),
        (0.5, 0.5 + 64.0 / 4800.0, 1.0, 0.75)
    );
}

/// A 17-i32 envelope/LFO block (218).
const BLOCK: [i32; 17] = [
    1 | 2 | 32 | 8, // flags: Env Tempo, LFO Tempo, LFO Global; bit 3 is not read with 17 i32
    1,              // Env Enable
    32818,          // predelay: T = 0.5
    16459,          // attack: 0.25
    49177,          // hold: 0.75
    65536,          // decay: 1
    64,             // sustain: 0.5
    100,            // release: 0
    64,             // Env Amount: 0.75
    16459,          // LFO Predelay: 0.25
    32818,          // LFO Attack: 0.5
    64,             // LFO Amount
    49177,          // LFO Speed: 0.75
    1,              // LFO Shape: 0.5
    -128,           // attack tension: 0
    64,             // decay tension: 0.75
    128,            // release tension: 1
];

fn blocks(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|_| blob(218, &ints(&BLOCK))).collect()
}

/// What [`BLOCK`] sets as block `k` (1–4: an envelope target), `amount` its LFO Amount.
fn block_set(k: usize, amount: f32) -> Vec<(usize, f32)> {
    let e = k - 1;
    let mut set = vec![
        (27 + k, 1.0),
        (32 + k, 1.0),
        (37 + k, 0.5),
        (42 + k, time(16459)),
        (47 + k, time(32818)),
        (52 + k, amount),
        (57 + k, time(49177)),
        (62 + e, 1.0),
        (66 + e, 1.0),
        (70 + e, time(32818)),
        (74 + e, time(16459)),
        (78 + e, 0.0),
        (82 + e, time(49177)),
        (86 + e, 1.0),
        (90 + e, 0.75),
        (94 + e, 0.5),
        (98 + e, time(100)),
        (102 + e, 1.0),
    ];
    if k >= 2 {
        set.push((106 + e, 0.75));
    }
    set.sort_by_key(|&(p, _)| p);
    set
}

#[test]
fn the_envelope_and_lfo_blocks_go_pan_volume_modx_mody_pitch_in_file_order() {
    // Block 0, Pan: its LFO only, no flags, no envelope; its Amount is 0.5 + raw/128.
    let pan = vec![
        (37, 0.5),
        (42, time(16459)),
        (47, time(32818)),
        (52, 1.0),
        (57, time(49177)),
    ];
    same(&changed(&imported(&blocks(1))), &pan);
    // Block 1, Volume: Amount 0.5 + raw/256; its Env Amount is never imported.
    let volume = with(pan.clone(), &block_set(1, 0.75));
    same(&changed(&imported(&blocks(2))), &volume);
    assert_eq!(imported(&blocks(2)).params[106], 1.0);
    // ModX, ModY: Env Amount too. Pitch: LFO Amount 0.5 + raw/4800.
    let modx = with(volume, &block_set(2, 0.75));
    let mody = with(modx, &block_set(3, 0.75));
    let pitch = with(mody, &block_set(4, 0.5 + 64.0 / 4800.0));
    same(&changed(&imported(&blocks(5))), &pitch);
    // A 6th block is not bounded: it writes shifted params (its LFO Shape onto Pan LFO Predelay, ...).
    let six = imported(&blocks(6));
    assert!(six.ok);
    assert_eq!(six.params[42], 0.5);
    assert!((six.params[47] - time(16459)).abs() < 1e-6);
    // Shifted far enough, a block writes nothing past the last param.
    assert!(imported(&blocks(120)).ok);
}

#[test]
fn the_time_law_is_a_fused_multiply_add_pinned_at_exact_raw_values() {
    let t = |raw: i32| {
        let mut block = BLOCK;
        block[7] = raw;
        imported(&[blob(218, &ints(&BLOCK)), blob(218, &ints(&block))]).params[98]
    };
    // Unfused, T(100) would be 0 and T(101) 1.5282189e-5.
    assert_eq!(t(100), 2.910383e-11);
    assert_eq!(t(101), 1.5282138e-5);
    assert_eq!(t(16459), 0.25);
    assert_eq!(t(49177), 0.75000006);
    assert_eq!(t(65536), 1.0);
    // Below 100 and far above: clamped.
    assert_eq!((t(0), t(i32::MIN), t(i32::MAX)), (0.0, 0.0, 1.0));
}

#[test]
fn a_short_block_takes_its_tensions_from_the_flags_and_a_shorter_one_aborts() {
    let tensions = |n: usize, flags: i32| {
        let mut block = BLOCK;
        block[0] = flags;
        let import = imported(&[blob(218, &ints(&BLOCK)), blob(218, &ints(&block[..n]))]);
        assert!(import.ok);
        (import.params[78], import.params[90], import.params[102])
    };
    // Bit 3 attack, bit 6 decay, bit 4 release: set → raw −101, clear → 0.
    let low = 0.5 - 101.0 / 256.0;
    assert_eq!(low, 0.10546875);
    for n in 14..=16 {
        assert_eq!(tensions(n, 8 | 16), (low, 0.5, low));
        assert_eq!(tensions(n, 64), (0.5, low, 0.5));
    }
    assert_eq!(tensions(17, 8 | 16 | 64), (0.0, 0.75, 1.0));
    // Fewer than 14 i32: an abort; the first block stays.
    let short = imported(&[blob(218, &ints(&BLOCK)), blob(218, &ints(&BLOCK[..13]))]);
    assert!(!short.ok);
    same(
        &changed(&short),
        &[
            (37, 0.5),
            (42, time(16459)),
            (47, time(32818)),
            (52, 1.0),
            (57, time(49177)),
        ],
    );
}

#[test]
fn the_filter_comes_from_219_or_220_whichever_is_later() {
    // 219: pan, volume and pitch are never imported; Cutoff, Resonance raw/256, Type raw/7; only from 24 bytes.
    let f219 = blob(219, &ints(&[999, 999, 999, 128, 64, 7]));
    assert_eq!(
        changed(&imported(std::slice::from_ref(&f219))),
        [(110, 1.0), (111, 0.5), (112, 0.25)]
    );
    let short = imported(&[blob(219, &ints(&[999, 999, 999, 128, 64]))]);
    assert!(short.ok);
    assert_eq!(changed(&short), []);
    // 220 after 219 wins, and the other way round.
    let f220 = blob(220, &ints(&[64, 128, 0]));
    let v = version("4.0.0");
    assert_eq!(
        changed(&imported(&[v.clone(), f219.clone(), f220.clone()])),
        [(111, 0.25), (112, 0.5)]
    );
    assert_eq!(
        changed(&imported(&[v.clone(), f220.clone(), f219.clone()])),
        [(110, 1.0), (111, 0.5), (112, 0.25)]
    );
    // Before 3.2.2 Resonance is trunc(256·raw/80) first.
    let old = |raw: i32| imported(&[version("3.1.1"), blob(220, &ints(&[0, raw, 0]))]).params[112];
    assert_eq!((old(40), old(1), old(-1)), (0.5, 3.0 / 256.0, 0.0));
    // 256·3/80 = 9.6: truncated, not rounded.
    assert_eq!(old(3), 9.0 / 256.0);
    // Shorter than 8 bytes, or than 12 from 2.5.1 on: an abort.
    assert!(!imported(&[blob(220, &ints(&[0]))]).ok);
    assert!(imported(&[blob(220, &ints(&[0, 0]))]).ok);
    assert!(!imported(&[v, blob(220, &ints(&[0, 0]))]).ok);
}

#[test]
fn the_voices_event_sets_polyphony_portamento_and_the_mono_and_porta_bits() {
    let v = version("3.5.2");
    assert_eq!(
        changed(&imported(&[
            v.clone(),
            blob(221, &[ints(&[99, 830]), vec![3]].concat())
        ])),
        [(117, 1.0), (118, 0.5), (119, 1.0), (120, 1.0)]
    );
    assert_eq!(
        changed(&imported(&[
            v.clone(),
            blob(221, &[ints(&[0, 830]), vec![1]].concat())
        ])),
        [(118, 0.5), (120, 1.0)]
    );
    // Each field only when the event reaches it.
    assert_eq!(
        changed(&imported(&[v, blob(221, &ints(&[33]))])),
        [(117, 1.0 / 3.0)]
    );
    // Before 3.5.2 the time is llround(ln(raw·13/24 + 1)·310.69498) first.
    let old = |raw: i32| imported(&[version("3.5.1"), blob(221, &ints(&[0, raw]))]).params[118];
    assert_eq!(old(384), 1.0);
    assert_eq!(old(100), 1246.0 / 1660.0);
    assert_eq!(old(0), 0.0);
}

#[test]
fn the_first_tracking_event_is_velocity_the_second_key_and_later_ones_are_ignored() {
    let tracking = |raws: [i32; 4]| blob(228, &ints(&raws));
    let import = imported(&[
        tracking([60, 128, -128, 256]),
        tracking([120, -256, 0, 64]),
        tracking([0, 0, 0, 0]),
    ]);
    assert_eq!(
        changed(&import),
        [
            (121, 1.0),
            (122, 0.0),
            (124, 0.625),
            (125, 0.5),
            (126, 0.75),
            (127, 0.25),
            (128, 1.0)
        ]
    );
}

#[test]
fn the_levels_event_scales_pan_and_volume_by_100_before_5_3_7() {
    let levels = blob(229, &ints(&[64, 64, 999, 128, -128]));
    assert_eq!(
        changed(&imported(&[version("5.3.6"), levels.clone()])),
        [(113, 1.0), (114, 0.25), (115, 0.75), (116, 0.25)]
    );
    let levels = blob(229, &ints(&[3200, 19200]));
    assert_eq!(
        changed(&imported(&[version("5.3.7"), levels])),
        [(113, 0.75), (114, 0.75)]
    );
}

#[test]
fn the_delay_event_converts_by_version() {
    let delay = |v: &str, raws: &[i32]| changed(&imported(&[version(v), blob(209, &ints(raws))]));
    assert_eq!(
        delay("5.3.7", &[12800, 12800, 1200, 10, 192]),
        [(130, 1.0), (131, 0.5), (132, 0.5), (133, 0.75), (134, 1.0)]
    );
    // Before 5.3.7 Feedback and Pan are ×100 first; before 3.3.1 Pitch is 10·raw − 1200 first.
    assert_eq!(
        delay("4.0.0", &[128, 128, -600]),
        [(131, 0.5), (133, 0.75), (134, 0.25)]
    );
    assert_eq!(delay("3.3.0", &[0, 64, 180]), [(134, 0.75)]);
    assert_eq!(delay("3.3.0", &[0, 64]), []);
    assert_eq!(delay("3.3.1", &[0, 64, 600]), [(134, 0.75)]);
}

#[test]
fn the_arp_event_reads_each_field_it_reaches() {
    let arp = |v: &str, fields: &[(usize, i32)], len: usize| {
        let mut bytes = vec![0u8; len];
        for (at, value) in [(10, 2 | 4), (60, 1)] {
            if let Some(byte) = bytes.get_mut(at) {
                *byte = value;
            }
        }
        for &(at, raw) in fields {
            bytes[at..at + 4].copy_from_slice(&raw.to_le_bytes());
        }
        changed(&imported(&[version(v), blob(215, &bytes)]))
    };
    let fields = [
        (40, 5),
        (44, 3),
        (48, 43),
        (52, 1447),
        (56, 24),
        (64, 450),
        (92, 6),
    ];
    assert_eq!(
        arp("3.5.2", &fields, 96),
        [
            (129, 0.0),
            (137, 1.0),
            (138, 1.0),
            (139, 1.0),
            (140, 1.0),
            (141, 0.5),
            (142, 0.5),
            (143, 1.0),
            (144, 1.0),
            (145, 0.5)
        ]
    );
    // Cut after byte 60: the gate and repeat are not read; under 11 bytes nothing is.
    assert_eq!(arp("3.5.2", &fields[..5], 61).len(), 8);
    assert_eq!(arp("3.5.2", &[], 10), []);
    // Before 3.5.2 Arp Time and Gate are clamp(llround(ln(raw·13/24 + 1)·310.69498), 450, 1447) first.
    let old = |raw: i32| {
        let set = arp("3.5.1", &[(52, raw), (64, raw)], 68);
        let at = |p: usize| set.iter().find(|s| s.0 == p).map_or(1.0, |s| s.1);
        assert_eq!(at(129), at(140));
        at(140)
    };
    assert_eq!(old(100), 796.0 / 997.0);
    assert_eq!(old(24), 370.0 / 997.0);
    assert_eq!((old(0), old(-2), old(1000)), (0.0, 0.0, 1.0));
}

#[test]
fn a_custom_waveform_resolves_only_under_instruments_shapes() {
    let path = |p: &str| imported(&[shape(p)]).custom_shape;
    let app = |rest: &str| {
        Some(ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: format!("3xOsc Shapes/{rest}"),
        })
    };
    assert_eq!(
        path(r"\Instruments\Shapes\HQ\Gen Shape.wav"),
        app("HQ/Gen Shape.wav")
    );
    // Both separators, runs collapsed, a trailing one dropped.
    assert_eq!(path(r"/Instruments//Shapes\\HQ/\x.wav\"), app("HQ/x.wav"));
    for other in [
        r"\Patches\Shapes\x.wav",
        r"\instruments\shapes\x.wav",
        r"Instruments\Shapes\x.wav",
        r"\Instruments\Shapes\",
        "",
    ] {
        assert_eq!(path(other), None, "{other}");
    }
    // One waveform: the last that resolves; it changes no param.
    let two = imported(&[
        shape(r"\Instruments\Shapes\a.wav"),
        shape(r"\Instruments\Shapes\b.wav"),
        shape(r"\Patches\c.wav"),
    ]);
    assert_eq!(two.custom_shape, app("b.wav"));
    assert_eq!(changed(&two), []);
}

/// A file with every event the bundled ones carry, `osc` its 213 blob: what is never written stays at its default.
fn full(osc: Vec<u8>) -> Vec<Vec<u8>> {
    let mut big = BLOCK.map(|r| r.saturating_mul(1000));
    big[0] = -1;
    let mut events = vec![
        version("3.1.1"),
        u8_event(0, 1),
        blob(201, &text("3x Osc")),
        blob(213, &osc),
        blob(192, &text("Name")),
        u32_event(128, 0x00FF_00FF),
        blob(209, &ints(&[99, 99, 99, 99, 99])),
        u32_event(138, 0x7FFF_7FFF),
        blob(215, &[0xFF; 100]),
    ];
    events.extend((0..5).map(|_| blob(218, &ints(&big))));
    events.extend([
        blob(219, &ints(&[99; 6])),
        blob(220, &ints(&[99; 3])),
        blob(221, &[ints(&[99, 99]), vec![3]].concat()),
        blob(228, &ints(&[99; 4])),
        blob(228, &ints(&[99; 4])),
        blob(229, &ints(&[99; 5])),
        shape(r"\Instruments\Shapes\HQ\Gen Shape.wav"),
    ]);
    events
}

#[test]
fn the_never_written_params_stay_at_their_defaults() {
    let d = defaults();
    let v11 = imported(&full(osc_blob(11, &[1, 1, 1])));
    assert!(v11.ok);
    assert!(changed(&v11).len() > 100, "{:?}", changed(&v11));
    // Pan LFO Global and Tempo, Volume Env Amount, Free Run; below v12 HQ, AM and Stereo Phase Randomness.
    for p in [27, 32, 106, 146, 147, 148, 25, 24, 26] {
        assert_eq!(v11.params[p], d[p], "param {p}");
    }
    // A v10 blob: the Inverts too.
    let v10 = imported(&full(osc_blob(10, &[1, 1, 1])));
    for p in [4, 5, 6] {
        assert_eq!(v10.params[p], d[p], "param {p}");
    }
    // A v12 blob without the 5th flag byte: HQ.
    let v12 = imported(&full(osc_blob(12, &[ints(&[64]), vec![1; 4]].concat())));
    assert!(v12.ok);
    assert_eq!(v12.params[25], d[25]);
    assert_eq!((v12.params[24], v12.params[26]), (1.0, 1.0));
}

#[test]
fn an_abort_keeps_what_was_written_and_the_name_and_category() {
    let import = imported(&[
        blob(229, &ints(&[0, 0, 0, 128])),
        blob(218, &ints(&[0; 13])),
        u8_event(16, 4),
    ]);
    assert!(!import.ok);
    assert_eq!(changed(&import), [(114, 0.0), (115, 0.75)]);
    assert_eq!((&*import.name, &*import.category), ("Gen Bass", "Bass"));
    let bad = import_fst(b"junk", Path::new("Lead/Saw Lead.v2.fst"));
    assert!(!bad.ok);
    assert_eq!((&*bad.name, &*bad.category), ("Saw Lead.v2", "Lead"));
}

#[test]
fn a_truncated_event_or_a_length_over_32_bits_aborts() {
    let whole = fst(&full(osc_blob(11, &[])));
    // Cut anywhere in the events: whatever was read stays, and it fails unless the cut falls between events.
    let mut ends = vec![22];
    let mut at = 22;
    for event in full(osc_blob(11, &[])) {
        at += event.len();
        ends.push(at);
    }
    for len in 0..whole.len() {
        let import = import_fst(&whole[..len], Path::new("a.fst"));
        assert_eq!(import.ok, ends.contains(&len), "cut at {len}");
    }
    // A varint length: 5 bytes up to 32 bits, no more.
    let varint = |length: &[u8], data: usize| {
        let file = [fst(&[]), vec![255], length.to_vec(), vec![0; data]].concat();
        import_fst(&file, Path::new("a.fst")).ok
    };
    assert!(varint(&[0x90, 0x80, 0x80, 0x80, 0x00], 16));
    assert!(
        !varint(&[0x90, 0x80, 0x80, 0x80, 0x00], 15),
        "cut in the data"
    );
    assert!(
        !varint(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F], 16),
        "4 GiB: cut short"
    );
    assert!(!varint(&[0x80, 0x80, 0x80, 0x80, 0x10], 16), "33 bits");
    assert!(
        !varint(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x00], 16),
        "6 bytes"
    );
    assert!(!varint(&[0x80], 0), "cut in the length");
}

#[test]
fn hostile_files_never_panic_and_every_param_stays_in_range() {
    let whole = fst(&full(osc_blob(14, &[vec![0; 12], ints(&[0; 91])].concat())));
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let check = |bytes: &[u8]| {
        let import = import_fst(bytes, Path::new("x/y.fst"));
        assert_eq!(import.params.len(), 149);
        assert!(import.params.iter().all(|v| (0.0..=1.0).contains(v)));
        // The fallback a failed import takes reads the same bytes.
        assert!(native_state(bytes).is_none_or(|m| m.params.len() == 149));
    };
    for _ in 0..3000 {
        // Flips in a whole file.
        let mut bytes = whole.clone();
        for _ in 0..1 + next() % 8 {
            let i = (next() as usize) % bytes.len();
            bytes[i] ^= 1 << (next() % 8);
        }
        check(&bytes);
        // Random events after a good header, random values in every used event.
        let n = (next() % 200) as usize;
        let tail: Vec<u8> = (0..n).map(|_| next() as u8).collect();
        check(&[fst(&[]), tail].concat());
        let id = [
            16u8, 138, 196, 199, 209, 213, 215, 218, 219, 220, 221, 228, 229,
        ][(next() % 13) as usize];
        let data: Vec<u8> = (0..next() % 600).map(|_| next() as u8).collect();
        check(&fst(&[version("1.2.3"), blob(id, &data), blob(id, &data)]));
    }
}

#[test]
fn a_failed_import_falls_back_to_the_files_native_chunks_with_those_tags_only() {
    use crate::flm::chunk::ChunkWriter;
    let chunk = |id: &[u8; 4], payload: &[u8]| {
        let mut writer = ChunkWriter::default();
        writer.chunk(*id, |w| w.raw(payload)).unwrap();
        writer.into_bytes()
    };
    let prms: Vec<u8> = [1.0f32, 0.5].iter().flat_map(|v| v.to_le_bytes()).collect();
    // A header that fails (format 0x10): FLhd and FLdt are skipped, PRMS is read, an unknown tag is not kept.
    let bytes = [
        with_format(0x10, &[]),
        chunk(b"JUNK", &[1, 2, 3]),
        chunk(b"PRMS", &prms),
        chunk(b"SMPR", &0.25f32.to_le_bytes()),
    ]
    .concat();
    assert!(!import_fst(&bytes, Path::new("a.fst")).ok);
    let native = native_state(&bytes).unwrap();
    assert_eq!(native.type_id, 39);
    assert_eq!((native.params[0], native.params[1]), (1.0, 0.5));
    assert_eq!(&native.params[2..], &defaults()[2..]);
    assert_eq!(native.common.0[0], 0.25);
    assert!(native.file.unknown_chunks.is_empty());
    // A whole native 3x Osc state reads too; another module's, junk, or no tagged chunk at all: none.
    let mut osc = ModuleInstance::new(39, 0);
    osc.params[3] = 0.5;
    let preset = crate::flm::write_preset(&osc).unwrap();
    assert_eq!(native_state(&preset).unwrap().params, osc.params);
    let reverb = crate::flm::write_preset(&ModuleInstance::new(32, 0)).unwrap();
    assert_eq!(native_state(&reverb), None);
    assert_eq!(native_state(b"junk"), None);
    assert_eq!(native_state(&fst(&[blob(218, &ints(&[0; 13]))])), None);
}

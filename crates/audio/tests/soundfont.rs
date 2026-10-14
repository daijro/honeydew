//! The SoundFont Player: the SoundFont 2 parser and the voices, tested
//! offline on SoundFonts built here.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

use honeydew_audio::ProcessContext;
use honeydew_audio::instrument::{Instrument, InstrumentModule};
use honeydew_audio::soundfont::{
    MAX_PATCHES, MAX_REGIONS, ParseError, SoundFont, SoundFontPlayer, VOICES, preset_headers,
};

// ---- the largest allocation, for the hostile-input tests ----

struct Largest;

thread_local! {
    /// While watching, the largest allocation this thread asked for.
    static LARGEST: Cell<Option<usize>> = const { Cell::new(None) };
}

fn watch(size: usize) {
    let _ = LARGEST.try_with(|l| l.set(l.get().map(|n| n.max(size))));
}

// SAFETY: every call forwards to `System` unchanged; watching touches only a const thread-local.
unsafe impl GlobalAlloc for Largest {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        watch(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        watch(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Largest = Largest;

/// The largest single allocation `f` makes on this thread.
fn largest_during(f: impl FnOnce()) -> usize {
    LARGEST.with(|l| l.set(Some(0)));
    f();
    LARGEST.with(|l| l.take()).unwrap()
}

// ---- building SoundFonts (the public SoundFont 2.01 layout) ----

/// A generator: operator and amount.
type Gen = (u16, [u8; 2]);

const PAN: u16 = 17;
const ATTACK: u16 = 34;
const DECAY: u16 = 36;
const SUSTAIN: u16 = 37;
const RELEASE: u16 = 38;
const INSTRUMENT: u16 = 41;
const KEYS: u16 = 43;
const VELS: u16 = 44;
const ATTENUATION: u16 = 48;
const COARSE: u16 = 51;
const FINE: u16 = 52;
const SAMPLE: u16 = 53;
const MODES: u16 = 54;
const SCALE: u16 = 56;
const EXCLUSIVE: u16 = 57;
const ROOT: u16 = 58;

fn amount(op: u16, value: i16) -> Gen {
    (op, value.to_le_bytes())
}

fn range(op: u16, lo: u8, hi: u8) -> Gen {
    (op, [lo, hi])
}

struct Sample {
    data: Vec<i16>,
    /// Loop start and end, from the sample's first frame.
    loop_points: (u32, u32),
    rate: u32,
    root: u8,
    correction: i8,
}

/// A RIFF chunk: id, size, payload and the pad byte an odd size takes.
fn chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = [&id[..], &(payload.len() as u32).to_le_bytes(), payload].concat();
    if payload.len() % 2 == 1 {
        out.push(0);
    }
    out
}

fn list(kind: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
    chunk(b"LIST", &[&kind[..], &chunks.concat()].concat())
}

fn name(text: &str) -> [u8; 20] {
    let mut out = [0; 20];
    out[..text.len()].copy_from_slice(text.as_bytes());
    out
}

/// Header records (`phdr` or `inst`) and their bags and generators, for zone lists `items`. `header(i, bag)` writes
/// one record; the terminal one is `header(None, bag)`.
fn tables(
    items: &[Vec<Vec<Gen>>],
    header: impl Fn(Option<usize>, u16) -> Vec<u8>,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (mut headers, mut bags, mut gens) = (Vec::new(), Vec::new(), Vec::new());
    let (mut bag, mut gen_count) = (0_u16, 0_u16);
    for (i, zones) in items.iter().enumerate() {
        headers.extend(header(Some(i), bag));
        for zone in zones {
            bags.extend([gen_count.to_le_bytes(), [0, 0]].concat());
            bag += 1;
            for (op, value) in zone {
                gens.extend([&op.to_le_bytes()[..], value].concat());
                gen_count += 1;
            }
        }
    }
    headers.extend(header(None, bag));
    bags.extend([gen_count.to_le_bytes(), [0, 0]].concat());
    gens.extend([0; 4]);
    (headers, bags, gens)
}

/// A SoundFont: presets and instruments as zone lists (each zone a generator list), and the samples.
fn soundfont(
    presets: &[Vec<Vec<Gen>>],
    instruments: &[Vec<Vec<Gen>>],
    samples: &[Sample],
) -> Vec<u8> {
    let (phdr, pbag, pgen) = tables(presets, |i, bag| {
        let text = i.map_or("EOP".to_owned(), |i| format!("Preset {i}"));
        let program = i.unwrap_or(255) as u16;
        [
            &name(&text)[..],
            &program.to_le_bytes(),
            &[0, 0],
            &bag.to_le_bytes(),
            &[0; 12],
        ]
        .concat()
    });
    let (inst, ibag, igen) = tables(instruments, |i, bag| {
        let text = i.map_or("EOI".to_owned(), |i| format!("Inst {i}"));
        [&name(&text)[..], &bag.to_le_bytes()].concat()
    });
    let (mut smpl, mut shdr) = (Vec::new(), Vec::new());
    for (i, sample) in samples.iter().enumerate() {
        let start = (smpl.len() / 2) as u32;
        smpl.extend(sample.data.iter().flat_map(|s| s.to_le_bytes()));
        smpl.extend([0; 2 * 46]);
        let end = start + sample.data.len() as u32;
        let (loop_start, loop_end) = sample.loop_points;
        shdr.extend(name(&format!("Sample {i}")));
        for value in [
            start,
            end,
            start + loop_start,
            start + loop_end,
            sample.rate,
        ] {
            shdr.extend(value.to_le_bytes());
        }
        shdr.extend([sample.root, sample.correction as u8, 0, 0, 1, 0]);
    }
    shdr.extend([&name("EOS")[..], &[0; 26]].concat());
    let riff = [
        &b"sfbk"[..],
        &list(b"INFO", &[chunk(b"ifil", &[2, 0, 1, 0])]),
        &list(b"sdta", &[chunk(b"smpl", &smpl)]),
        &list(
            b"pdta",
            &[
                chunk(b"phdr", &phdr),
                chunk(b"pbag", &pbag),
                chunk(b"pmod", &[0; 10]),
                chunk(b"pgen", &pgen),
                chunk(b"inst", &inst),
                chunk(b"ibag", &ibag),
                chunk(b"imod", &[0; 10]),
                chunk(b"igen", &igen),
                chunk(b"shdr", &shdr),
            ],
        ),
    ]
    .concat();
    chunk(b"RIFF", &riff)
}

/// `cycles` cycles of a sine with `period` frames per cycle, at amplitude 16000.
fn sine(period: usize, cycles: usize) -> Vec<i16> {
    (0..period * cycles)
        .map(|i| (16000.0 * (std::f64::consts::TAU * i as f64 / period as f64).sin()) as i16)
        .collect()
}

/// 400 frames of a 100-frame sine at 44 kHz: 440 Hz at its root key 69. Its loop is all four cycles.
fn sine_sample() -> Sample {
    Sample {
        data: sine(100, 4),
        loop_points: (0, 400),
        rate: 44_000,
        root: 69,
        correction: 0,
    }
}

/// 4800 frames of a constant level (half scale), looped whole: the output traces the envelope and the gains.
fn level_sample() -> Sample {
    Sample {
        data: vec![16384; 4800],
        loop_points: (0, 4800),
        rate: 48_000,
        root: 60,
        correction: 0,
    }
}

/// One preset holding one instrument (`preset_zone` gens, then the instrument), whose zones are `zones` (each
/// followed by its sample gen).
fn one_instrument(preset_zone: Vec<Gen>, zones: Vec<Vec<Gen>>, samples: &[Sample]) -> Vec<u8> {
    let preset = vec![[preset_zone, vec![amount(INSTRUMENT, 0)]].concat()];
    soundfont(&[preset], &[zones], samples)
}

/// The looped sine as preset 0.
fn sine_bank() -> Vec<u8> {
    one_instrument(
        vec![],
        vec![vec![amount(MODES, 1), amount(SAMPLE, 0)]],
        &[sine_sample()],
    )
}

fn font(bytes: Vec<u8>) -> Arc<SoundFont> {
    Arc::new(SoundFont::parse(bytes).expect("the SoundFont parses"))
}

const RATE: f32 = 48_000.0;

fn player(bytes: Vec<u8>, patch: i32) -> SoundFontPlayer {
    SoundFontPlayer::new(RATE, Some((font(bytes), patch)))
}

fn context(sample_rate: f32) -> ProcessContext {
    ProcessContext {
        sample_rate,
        tempo_bpm: 120.0,
        start_beat: 0.0,
        playing: true,
    }
}

/// `frames` frames from `player`, rendered in blocks of `block`.
fn render_blocks(
    player: &mut SoundFontPlayer,
    frames: usize,
    block: usize,
) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    for (l, r) in left.chunks_mut(block).zip(right.chunks_mut(block)) {
        player.process(l, r, context(RATE));
    }
    (left, right)
}

fn render(player: &mut SoundFontPlayer, frames: usize) -> (Vec<f32>, Vec<f32>) {
    render_blocks(player, frames, 256)
}

/// The frequency from rising zero crossings over `samples` at `RATE`.
fn pitch(samples: &[f32]) -> f32 {
    let rising: Vec<usize> = (1..samples.len())
        .filter(|&i| samples[i - 1] <= 0.0 && samples[i] > 0.0)
        .collect();
    let (first, last) = (rising[0], *rising.last().unwrap());
    (rising.len() - 1) as f32 * RATE / (last - first) as f32
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |m, s| m.max(s.abs()))
}

/// The pitch of `key` at velocity `vel` on patch 0 of `bytes`, after 0.1 s.
fn pitch_of(bytes: Vec<u8>, key: u8, vel: f32) -> f32 {
    let mut player = player(bytes, 0);
    player.note_on(key, vel, 0.5);
    let (left, _) = render(&mut player, 24_000);
    pitch(&left[4_800..])
}

fn close(got: f32, want: f32, tolerance: f32) -> bool {
    (got - want).abs() <= tolerance * want.abs().max(1e-6)
}

// ---- pitch ----

#[test]
fn pitch_follows_the_root_key_and_the_sample_rate() {
    for (key, want) in [(69, 440.0), (57, 220.0), (81, 880.0), (76, 659.26)] {
        let got = pitch_of(sine_bank(), key, 1.0);
        assert!(close(got, want, 0.002), "key {key}: {got} Hz, want {want}");
    }
    // overridingRootKey replaces the sample's own pitch.
    let bytes = one_instrument(
        vec![],
        vec![vec![amount(MODES, 1), amount(ROOT, 57), amount(SAMPLE, 0)]],
        &[sine_sample()],
    );
    assert!(close(pitch_of(bytes, 57, 1.0), 440.0, 0.002));
}

#[test]
fn tuning_generators_and_the_pitch_correction_add_up() {
    // Coarse +7 (preset) and +5 (instrument): an octave. Fine −50 (preset) + 30 (instrument) and the sample's
    // correction +20 cents: none.
    let mut sample = sine_sample();
    sample.correction = 20;
    let bytes = one_instrument(
        vec![amount(COARSE, 7), amount(FINE, -50)],
        vec![vec![
            amount(MODES, 1),
            amount(COARSE, 5),
            amount(FINE, 30),
            amount(SAMPLE, 0),
        ]],
        &[sample],
    );
    assert!(close(pitch_of(bytes, 69, 1.0), 880.0, 0.002));
    // scaleTuning 50: half a semitone per key, so 24 keys up is an octave.
    let bytes = one_instrument(
        vec![],
        vec![vec![amount(MODES, 1), amount(SCALE, 50), amount(SAMPLE, 0)]],
        &[sine_sample()],
    );
    assert!(close(pitch_of(bytes, 93, 1.0), 880.0, 0.002));
}

#[test]
fn pitch_bend_is_two_semitones_and_slides_add_to_sounding_voices_only() {
    let mut player = player(sine_bank(), 0);
    player.set_controller(4, 1.0);
    player.note_on(69, 1.0, 0.5);
    let (left, _) = render(&mut player, 24_000);
    let want = 440.0 * 2f32.powf(2.0 / 12.0);
    assert!(
        close(pitch(&left[4_800..]), want, 0.002),
        "{}",
        pitch(&left[4_800..])
    );
    player.set_controller(4, 0.5);

    // A slide offset moves the voice that sounds; a new voice starts without one.
    player.set_slide(12.0, 0.0, 0.0);
    let (left, _) = render(&mut player, 24_000);
    assert!(close(pitch(&left[4_800..]), 880.0, 0.002));
    player.reset();
    player.note_on(69, 1.0, 0.5);
    let (left, _) = render(&mut player, 24_000);
    assert!(close(pitch(&left[4_800..]), 440.0, 0.002));

    // Slide pan and velocity add to the note's: p = 0.5 + 0.25 (L 0.5, R 1) and velocity 1 − 0.5. A velocity past
    // full is clamped to it.
    let mut player = SoundFontPlayer::new(RATE, Some((font(level_bank(vec![], vec![])), 0)));
    player.note_on(60, 1.0, 0.5);
    render(&mut player, 480);
    player.set_slide(0.0, 0.25, -0.5);
    let (left, right) = render(&mut player, 480);
    assert!(close(left[479], 0.5 * 0.5 * 0.5, 1e-4), "{}", left[479]);
    assert!(close(right[479], 0.5 * 0.5, 1e-4), "{}", right[479]);
    player.set_slide(0.0, 0.0, 0.7);
    let (left, _) = render(&mut player, 480);
    assert!(close(left[479], 0.5, 1e-4), "{}", left[479]);
}

// ---- zones ----

#[test]
fn velocity_ranges_pick_the_zone_with_velocity_truncated_to_127_steps() {
    // 0–63 plays the sine at its root, 64–127 an octave up (root 57).
    let bytes = || {
        one_instrument(
            vec![],
            vec![
                vec![range(VELS, 0, 63), amount(MODES, 1), amount(SAMPLE, 0)],
                vec![
                    range(VELS, 64, 127),
                    amount(MODES, 1),
                    amount(ROOT, 57),
                    amount(SAMPLE, 0),
                ],
            ],
            &[sine_sample()],
        )
    };
    // trunc(127 · 0.5) = 63: the low zone, as is 0.3 (38). 0.504 → 64: the high one.
    for (vel, want) in [(0.3, 440.0), (0.5, 440.0), (0.504, 880.0), (1.0, 880.0)] {
        let got = pitch_of(bytes(), 69, vel);
        assert!(close(got, want, 0.002), "velocity {vel}: {got} Hz");
    }
}

#[test]
fn key_ranges_are_checked_at_preset_and_instrument_level() {
    // The preset zone takes keys 60–72, the instrument zone 48–64: keys 60–64 sound.
    let bytes = one_instrument(
        vec![range(KEYS, 60, 72)],
        vec![vec![
            range(KEYS, 48, 64),
            amount(MODES, 1),
            amount(SAMPLE, 0),
        ]],
        &[sine_sample()],
    );
    let mut player = player(bytes, 0);
    for key in [47, 59, 65, 72] {
        player.note_on(key, 1.0, 0.5);
    }
    assert_eq!(player.active_voices(), 0);
    for key in [60, 64] {
        player.note_on(key, 1.0, 0.5);
    }
    assert_eq!(player.active_voices(), 2);
}

#[test]
fn one_voice_per_matching_zone_and_exclusive_class_is_ignored() {
    let zone = |lo| {
        vec![
            range(KEYS, lo, 127),
            amount(EXCLUSIVE, 1),
            amount(MODES, 1),
            amount(SAMPLE, 0),
        ]
    };
    let bytes = one_instrument(vec![], vec![zone(0), zone(60), zone(70)], &[sine_sample()]);
    let mut player = player(bytes, 0);
    player.note_on(65, 1.0, 0.5);
    assert_eq!(player.active_voices(), 2, "zones 0 and 60 match key 65");
    player.note_on(66, 1.0, 0.5);
    render(&mut player, 4_800);
    assert_eq!(player.active_voices(), 4, "exclusive class cut nothing");
}

#[test]
fn global_zones_give_defaults_and_preset_values_add() {
    // The instrument's global zone: coarse +12. Zone 1 (keys 0–63) keeps it; zone 2 (64–127) sets its own 0.
    // The preset's global zone: coarse −12, which its zone keeps, or replaces with its own +12. Preset values add to
    // the instrument's.
    let instrument = vec![
        vec![amount(COARSE, 12)],
        vec![range(KEYS, 0, 63), amount(MODES, 1), amount(SAMPLE, 0)],
        vec![
            range(KEYS, 64, 127),
            amount(MODES, 1),
            amount(COARSE, 0),
            amount(SAMPLE, 0),
        ],
    ];
    let bytes = |zone: Vec<Gen>| {
        let preset = vec![
            vec![amount(COARSE, -12)],
            [zone, vec![amount(INSTRUMENT, 0)]].concat(),
        ];
        soundfont(
            &[preset],
            std::slice::from_ref(&instrument),
            &[sine_sample()],
        )
    };
    // −12 + 12 at key 57: the sample's 220 Hz. −12 + 0 at key 69: 220 Hz.
    assert!(close(pitch_of(bytes(vec![]), 57, 1.0), 220.0, 0.002));
    assert!(close(pitch_of(bytes(vec![]), 69, 1.0), 220.0, 0.002));
    // +12 + 12 at key 57: 880 Hz.
    assert!(close(
        pitch_of(bytes(vec![amount(COARSE, 12)]), 57, 1.0),
        880.0,
        0.002
    ));
    // A zone without a sample or instrument that is not first is ignored.
    let preset = vec![vec![amount(INSTRUMENT, 0)], vec![amount(COARSE, 5)]];
    let bytes = soundfont(
        &[preset],
        std::slice::from_ref(&instrument),
        &[sine_sample()],
    );
    assert_eq!(font(bytes).regions(0).len(), 2);
}

// ---- gain, pan and envelopes (on the constant-level sample) ----

/// The constant-level sample as patch 0, its one zone carrying `gens` and the preset zone `preset`.
fn level_bank(preset: Vec<Gen>, gens: Vec<Gen>) -> Vec<u8> {
    let zone = [gens, vec![amount(MODES, 1), amount(SAMPLE, 0)]].concat();
    one_instrument(preset, vec![zone], &[level_sample()])
}

#[test]
fn gain_is_linear_velocity_times_the_attenuation() {
    let level = |preset, gens, vel: f32| {
        let mut player = player(level_bank(preset, gens), 0);
        player.note_on(60, vel, 0.5);
        let (left, right) = render(&mut player, 4_800);
        assert_eq!(left[4_000], right[4_000]);
        left[4_000]
    };
    // Half scale, full velocity, centre pan: 0.5.
    let full = level(vec![], vec![], 1.0);
    assert!(close(full, 0.5, 1e-4), "{full}");
    assert!(close(level(vec![], vec![], 0.5), 0.25, 1e-4));
    // 100 cB (instrument) + 100 cB (preset) = 200 cB: 10^(−200/200) = 0.1.
    let attenuated = level(
        vec![amount(ATTENUATION, 100)],
        vec![amount(ATTENUATION, 100)],
        1.0,
    );
    assert!(close(attenuated, 0.05, 1e-4), "{attenuated}");
    // Attenuation and sustain stay in the standard's 0..1440 cB: no amplification, and 1440 at most.
    let quietest = 0.5 * 10f32.powf(-1440.0 / 200.0);
    let clamped = level(vec![], vec![amount(ATTENUATION, 2000)], 1.0);
    assert!(close(clamped, quietest, 1e-3), "{clamped}");
    assert!(close(
        level(vec![], vec![amount(ATTENUATION, -200)], 1.0),
        0.5,
        1e-4
    ));
    assert!(close(
        level(vec![], vec![amount(SUSTAIN, -200)], 1.0),
        0.5,
        1e-4
    ));
    let sustain = level(vec![], vec![amount(SUSTAIN, 2000)], 1.0);
    assert!(close(sustain, quietest, 1e-3), "{sustain}");
}

#[test]
fn pan_adds_the_generator_to_the_note_pan() {
    let sides = |preset, gens, pan: f32| {
        let mut player = player(level_bank(preset, gens), 0);
        player.note_on(60, 1.0, pan);
        let (left, right) = render(&mut player, 4_800);
        (left[4_000] / 0.5, right[4_000] / 0.5)
    };
    let near = |(l, r): (f32, f32), (wl, wr): (f32, f32)| close(l, wl, 1e-3) && close(r, wr, 1e-3);
    assert!(near(sides(vec![], vec![], 0.5), (1.0, 1.0)));
    assert!(near(sides(vec![], vec![], 0.0), (1.0, 0.0)));
    // Centre note pan (127/255) and +250 (25 %): p = 0.748, L = 0.504, R = 1.
    let got = sides(vec![], vec![amount(PAN, 250)], 127.0 / 255.0);
    assert!(
        near(got, (2.0 - 2.0 * (127.0 / 255.0 + 0.25), 1.0)),
        "{got:?}"
    );
    // 400 + 400 is clamped to 500: hard right from a left note.
    assert!(near(
        sides(vec![amount(PAN, 400)], vec![amount(PAN, 400)], 0.25),
        (0.5, 1.0)
    ));
    assert!(near(
        sides(vec![amount(PAN, 400)], vec![amount(PAN, 400)], 0.5),
        (0.0, 1.0)
    ));
}

/// The first frame at which `samples` reaches its maximum.
fn rise_time(samples: &[f32]) -> usize {
    let top = peak(samples);
    samples.iter().position(|&s| s >= top).unwrap()
}

#[test]
fn original_envelopes_follow_the_soundfont_timecents_with_the_preset_values_added() {
    // Instrument + preset: attack −2400 + 1200 and decay 0 − 1200 tc are 0.5 s each; sustain 30 + 30 = 60 cB
    // (10^(−0.3)); release −600 − 600 tc is 0.5 s. Every stage moves by c = 8.82 / (Fs · 0.5) a frame, so the attack
    // (towards 5) is full after ln 1.25 / −ln(1 − c) frames, about 2.5 % of 0.5 s.
    let preset = vec![
        amount(ATTACK, 1200),
        amount(DECAY, -1200),
        amount(SUSTAIN, 30),
        amount(RELEASE, -600),
    ];
    let gens = vec![
        amount(ATTACK, -2400),
        amount(DECAY, 0),
        amount(SUSTAIN, 30),
        amount(RELEASE, -600),
    ];
    let mut player = player(level_bank(preset, gens), 0);
    player.note_on(60, 1.0, 0.5);
    let (left, _) = render(&mut player, 96_000);
    let c = 8.82 / (f64::from(RATE) * 0.5);
    let want = (1.25_f64.ln() / -(1.0 - c).ln()).ceil() as usize - 1;
    let full = rise_time(&left);
    assert!(
        full.abs_diff(want) <= 1,
        "attack full at {full}, want {want}"
    );
    // 2400 frames into the decay: S + (1 − S)·(1 − c)^2400.
    let s = 10f64.powf(-0.3);
    let decaying = 0.5 * (s + (1.0 - s) * (1.0 - c).powi(2_400));
    assert!(
        close(left[full + 2_400], decaying as f32, 1e-3),
        "decay {} want {decaying}",
        left[full + 2_400]
    );
    assert!(
        close(left[95_999], 0.5 * s as f32, 1e-3),
        "sustain {}",
        left[95_999]
    );
    // Release from S ≈ 0.5 to 0.01: ln(S/0.01) / c frames.
    player.note_off(60);
    let (left, _) = render(&mut player, 48_000);
    let want = ((s / 0.01).ln() / c) as usize;
    let end = left.iter().rposition(|&x| x != 0.0).unwrap();
    assert!(
        end.abs_diff(want) <= want / 100,
        "release ended at {end}, want {want}"
    );
    assert_eq!(player.active_voices(), 0);
}

#[test]
fn default_envelopes_are_short_and_the_shortest_time_is_5_ms() {
    // −12000 tc is under a millisecond: 5 ms gives c = 8.82 / 240 at 48 kHz, so full in about 6 frames.
    let mut player = player(level_bank(vec![], vec![]), 0);
    player.note_on(60, 1.0, 0.5);
    let (left, _) = render(&mut player, 480);
    // (1 − c)^n ≤ 0.8 at n = 6 updates: frame 5.
    assert_eq!(rise_time(&left), 5);
    assert!(close(left[479], 0.5, 1e-4), "sustain 0 cB is full");
    // The default release is 5 ms too: below 0.01 after ln 100 / −ln(1 − c) = 123 updates, so frame 121 is the last.
    player.note_off(60);
    let (left, _) = render(&mut player, 1_000);
    let end = left.iter().rposition(|&x| x != 0.0).unwrap();
    assert!((118..=124).contains(&end), "release ended at {end}");
}

#[test]
fn the_envelope_toggle_switches_to_the_sliders() {
    // p5 Custom. Attack 0.5: t = 10 · (0.95 · 0.5 + 0.05)³ = 1.447 s, full at exactly t. Sustain 0.5: x = 64/127,
    // S = 10^(5 · x^0.2 − 5). The SoundFont's envelope (attack 1 s tc) is not used.
    let mut player = player(level_bank(vec![], vec![amount(ATTACK, 0)]), 0);
    for (index, value) in [(5, 1.0), (1, 0.5), (2, 0.3), (3, 0.5), (4, 0.0)] {
        player.set_parameter(index, value);
    }
    player.note_on(60, 1.0, 0.5);
    let (left, _) = render(&mut player, 192_000);
    let t = 10.0 * (0.95 * 0.5 + 0.05_f64).powi(3);
    let want = (t * f64::from(RATE)) as usize;
    let got = rise_time(&left);
    assert!(
        got.abs_diff(want) <= want / 500,
        "custom attack full at {got}, want {want}"
    );
    let s = 10f32.powf(5.0 * (64.0_f32 / 127.0).powf(0.2) - 5.0);
    // Decay 0.3: t = 10 · (0.95 · 0.3 + 0.05)³ s, and 1 % of the distance to S is left at t.
    let decay = 10.0 * (0.95 * 0.3 + 0.05_f64).powi(3);
    let at = got + (decay * f64::from(RATE)) as usize;
    let want = 0.5 * (s + 0.01 * (1.0 - s));
    assert!(
        close(left[at], want, 1e-3),
        "decay {} want {want}",
        left[at]
    );
    assert!(
        close(left[191_999], 0.5 * s, 1e-3),
        "sustain {} want {}",
        left[191_999],
        0.5 * s
    );
    // Release 0.5: 1 % of the distance is left at t.
    player.set_parameter(4, 0.5);
    player.note_off(60);
    let (left, _) = render(&mut player, 96_000);
    // Latched at the note's start: Release was 0 then, so the voice is gone at once.
    assert!(left.iter().position(|&x| x == 0.0).unwrap() < 480);

    // Release 0.5, latched by a new note, from full sustain: 1 % of the distance is left at t, so a tenth at t/2.
    player.set_parameter(3, 1.0);
    player.note_on(60, 1.0, 0.5);
    let (left, _) = render(&mut player, 96_000);
    assert!(close(left[95_999], 0.5, 1e-4), "sustain 1 is full");
    player.note_off(60);
    let (left, _) = render(&mut player, 96_000);
    let half = (t * f64::from(RATE) / 2.0) as usize;
    assert!(close(left[half], 0.05, 0.01), "{}", left[half]);

    // Back to Original at v = 0: the SoundFont's 1 s timecent attack (about 25 ms).
    player.reset();
    player.set_parameter(5, 0.0);
    player.note_on(60, 1.0, 0.5);
    let (left, _) = render(&mut player, 4_800);
    assert!(
        rise_time(&left).abs_diff(1_215) <= 2,
        "{}",
        rise_time(&left)
    );
}

#[test]
fn custom_sustain_zero_is_the_quietest_step_not_silence() {
    // x = clamp(round(127v), 1, 127) / 127: v = 0, and every v under half a step, play as x = 1/127, so
    // S = 10^(5 · (1/127)^0.2 − 5) ≈ 7.85e-4, not 10^−5 (x = 0).
    let held = |sustain: f32| {
        let mut player = player(level_bank(vec![], vec![]), 0);
        for (index, value) in [(5, 1.0), (1, 0.0), (2, 0.0), (3, sustain), (4, 0.0)] {
            player.set_parameter(index, value);
        }
        player.note_on(60, 1.0, 0.5);
        // 1 s is 200 decay times of 5 ms: settled.
        render(&mut player, 48_000).0[47_999]
    };
    let want = 0.5 * 10f32.powf(5.0 * (1.0_f32 / 127.0).powf(0.2) - 5.0);
    for sustain in [0.0, 0.003, 1.0 / 127.0] {
        let got = held(sustain);
        assert!(close(got, want, 1e-3), "v = {sustain}: {got} want {want}");
    }
}

// ---- loops and the sample's end ----

#[test]
fn loop_modes_1_and_3_loop_through_the_release_others_end() {
    for (mode, loops) in [(0, false), (1, true), (2, false), (3, true)] {
        let bytes = one_instrument(
            vec![],
            vec![vec![
                amount(RELEASE, 2400),
                amount(MODES, mode),
                amount(SAMPLE, 0),
            ]],
            &[sine_sample()],
        );
        let mut player = player(bytes, 0);
        player.note_on(69, 1.0, 0.5);
        let (left, _) = render(&mut player, 24_000);
        assert_eq!(
            peak(&left[12_000..]) > 0.4,
            loops,
            "mode {mode} still sounding"
        );
        player.note_off(69);
        let (left, _) = render(&mut player, 24_000);
        assert_eq!(
            peak(&left[..2_400]) > 0.1,
            loops,
            "mode {mode} in its release"
        );
    }
}

#[test]
fn a_sample_without_a_loop_fades_out_at_its_end() {
    let bytes = one_instrument(vec![], vec![vec![amount(SAMPLE, 0)]], &[sine_sample()]);
    let mut player = player(bytes, 0);
    player.note_on(69, 1.0, 0.5);
    // 400 frames at 44 kHz is 436 frames at 48 kHz; the fade (c = 0.01) takes about 460 more.
    let (left, _) = render(&mut player, 2_000);
    assert!(peak(&left[..400]) > 0.4);
    let last = left.iter().rposition(|&x| x != 0.0).unwrap();
    // The end is reached at frame 435; 0.99^k < 0.01 at k = 459, so frame 893 is the last.
    assert!((884..=904).contains(&last), "last sound at {last}");
    assert_eq!(player.active_voices(), 0);
    // An empty loop (end not after start) does not loop.
    let mut sample = sine_sample();
    sample.loop_points = (200, 200);
    let bytes = one_instrument(
        vec![],
        vec![vec![amount(MODES, 1), amount(SAMPLE, 0)]],
        &[sample],
    );
    let mut empty_loop = SoundFontPlayer::new(RATE, Some((font(bytes), 0)));
    empty_loop.note_on(69, 1.0, 0.5);
    render(&mut empty_loop, 2_000);
    assert_eq!(empty_loop.active_voices(), 0);
}

#[test]
fn loop_offsets_move_the_loop() {
    // 400 frames of the 440 Hz sine, then 400 of an 880 Hz one. The sample's loop is the first part; the offsets
    // (+400 frames each) move it onto the second.
    let mut sample = sine_sample();
    sample.data.extend(sine(50, 8));
    let zone = |offset| {
        vec![
            amount(MODES, 1),
            amount(2, offset),
            amount(3, offset),
            amount(SAMPLE, 0),
        ]
    };
    let samples = |s: &Sample| Sample {
        data: s.data.clone(),
        ..*s
    };
    let tail = |offset| {
        let bytes = one_instrument(vec![], vec![zone(offset)], &[samples(&sample)]);
        pitch_of(bytes, 69, 1.0)
    };
    assert!(close(tail(0), 440.0, 0.002));
    assert!(close(tail(400), 880.0, 0.002));
}

#[test]
fn coarse_loop_offsets_move_the_loop_by_32768_frames() {
    // 400 silent frames, 400 at a constant level, then silence to 32768 + 1000. The sample's loop is 32768 frames past
    // the level part; coarse offsets of −1 move it back onto it exactly (at 48 kHz on its root, one frame a frame).
    let mut data = vec![0; 400];
    data.extend([16384; 400]);
    data.resize(32_768 + 1_000, 0);
    let sample = Sample {
        data,
        loop_points: (32_768 + 400, 32_768 + 800),
        rate: 48_000,
        root: 60,
        correction: 0,
    };
    let zone = vec![
        amount(MODES, 1),
        amount(45, -1),
        amount(50, -1),
        amount(SAMPLE, 0),
    ];
    let mut player = player(one_instrument(vec![], vec![zone], &[sample]), 0);
    player.note_on(60, 1.0, 0.5);
    let (left, _) = render(&mut player, 9_600);
    let low = left[1_000..].iter().fold(1.0_f32, |m, &x| m.min(x));
    assert!(close(low, 0.5, 1e-4), "the loop left the level part: {low}");
}

#[test]
fn a_loops_last_frame_leads_into_its_first_not_into_the_frame_at_loop_end() {
    // 8 frames, looped 2..6 (frames 2–5; frame 6 is the first one past the loop). Key 48 on root 60 steps half a
    // frame: positions 0, 0.5, … 5.5, then 2.0 again. Position 5.5 is halfway from frame 5 to the loop start (frame
    // 2), not to frame 6.
    let data: Vec<i16> = vec![1000, 5000, 9000, 13000, 17000, 21000, 25000, 29000];
    let sample = Sample {
        data: data.clone(),
        loop_points: (2, 6),
        rate: 48_000,
        root: 60,
        correction: 0,
    };
    let bytes = one_instrument(
        vec![],
        vec![vec![amount(MODES, 1), amount(SAMPLE, 0)]],
        &[sample],
    );
    let mut player = player(bytes, 0);
    player.note_on(48, 1.0, 0.5);
    let (left, _) = render(&mut player, 24);
    let at = |i: usize| f32::from(data[i]) / 32_768.0;
    let mid = |a: usize, b: usize| (at(a) + at(b)) / 2.0;
    // Frame k reads position k/2 up to 5.5 (k = 11), then 2 + (k − 12)/2, a lap of 8 frames.
    for (frame, want) in [
        (9, mid(4, 5)),
        (10, at(5)),
        (11, mid(5, 2)),
        (12, at(2)),
        (13, mid(2, 3)),
        (18, at(5)),
        (19, mid(5, 2)),
        (20, at(2)),
    ] {
        assert!(
            close(left[frame], want, 1e-5),
            "frame {frame}: {} want {want}",
            left[frame]
        );
    }
}

#[test]
fn a_frame_between_two_samples_is_interpolated_linearly() {
    // Key 69 reads the 44 kHz sine at 48 kHz: frame 100 is at 100 · 44/48 = 91.67, between samples 91 and 92.
    let mut player = player(sine_bank(), 0);
    player.note_on(69, 1.0, 0.5);
    let (left, _) = render(&mut player, 101);
    let data = sine(100, 4);
    let position = 100.0 * 44_000.0 / 48_000.0_f64;
    let (i, frac) = (position as usize, position.fract() as f32);
    let (a, b) = (
        f32::from(data[i]) / 32_768.0,
        f32::from(data[i + 1]) / 32_768.0,
    );
    let want = a + (b - a) * frac;
    assert!(close(left[100], want, 1e-4), "{} want {want}", left[100]);
}

// ---- voices ----

#[test]
fn thirty_two_voices_and_the_oldest_started_is_stolen() {
    assert_eq!(VOICES, 32);
    let mut player = player(sine_bank(), 0);
    for key in 40..74 {
        player.note_on(key, 1.0, 0.5);
    }
    assert_eq!(player.active_voices(), 32);
    // Keys 40 and 41, the oldest, were stolen: their note-offs release nothing. Key 42's does.
    player.note_off(40);
    player.note_off(41);
    render(&mut player, 4_800);
    assert_eq!(player.active_voices(), 32);
    player.note_off(42);
    render(&mut player, 4_800);
    assert_eq!(player.active_voices(), 31);
    player.all_notes_off();
    render(&mut player, 4_800);
    assert_eq!(player.active_voices(), 0);
}

#[test]
fn a_note_matching_more_zones_than_voices_plays_the_last_32() {
    // 40 zones match: the first 8 are at 1440 cB, the last 32 at full. The voices sound the last 32.
    let zone = |cb| vec![amount(ATTENUATION, cb), amount(MODES, 1), amount(SAMPLE, 0)];
    let zones = [vec![zone(1440); 8], vec![zone(0); 32]].concat();
    let mut player = player(one_instrument(vec![], zones, &[level_sample()]), 0);
    player.note_on(60, 1.0, 0.5);
    assert_eq!(player.active_voices(), 32);
    let (left, _) = render(&mut player, 480);
    assert!(close(left[479], 32.0 * 0.5, 1e-4), "{}", left[479]);
}

#[test]
fn the_patch_is_the_preset_in_file_order_and_out_of_range_is_the_nearest_end() {
    // Preset 0 plays the sine at its root, preset 1 an octave up.
    let zone = |root| {
        vec![vec![
            amount(MODES, 1),
            amount(ROOT, root),
            amount(SAMPLE, 0),
        ]]
    };
    let presets = [
        vec![vec![amount(INSTRUMENT, 0)]],
        vec![vec![amount(INSTRUMENT, 1)]],
    ];
    let bytes = || soundfont(&presets, &[zone(69), zone(57)], &[sine_sample()]);
    for (patch, want) in [(-3, 440.0), (0, 440.0), (1, 880.0), (9, 880.0)] {
        let mut player = player(bytes(), patch);
        player.note_on(69, 1.0, 0.5);
        let (left, _) = render(&mut player, 24_000);
        assert!(close(pitch(&left[4_800..]), want, 0.002), "patch {patch}");
    }
    let headers = preset_headers(&bytes()).unwrap();
    assert_eq!(headers.len(), 2);
    assert_eq!(headers[1].name(), b"Preset 1");
    assert_eq!((headers[1].preset, headers[1].bank), (1, 0));
    assert_eq!(font(bytes()).presets(), &headers[..]);
    assert!(font(bytes()).regions(usize::MAX).is_empty());
}

#[test]
fn enable_off_freezes_the_voices_and_on_resumes_them() {
    // Enable switched off: the SoundFont Player freezes its voices (not rendered, not advanced) and
    // resumes them when it is enabled again.
    let module = || {
        let mut module = InstrumentModule::new(Box::new(player(sine_bank(), 0)), 256);
        module.instrument.note_on(69, 1.0, 0.5);
        module
    };
    let run = |module: &mut InstrumentModule, frames| {
        let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
        module.process(&mut left, &mut right, context(RATE));
        left
    };
    let want = run(&mut module(), 2_000);
    let mut paused = module();
    let mut got = run(&mut paused, 1_000);
    paused.set_parameter(0, 0.0);
    assert_eq!(
        peak(&run(&mut paused, 500)),
        0.0,
        "disabled, it renders nothing"
    );
    paused.set_parameter(0, 1.0);
    got.extend(run(&mut paused, 1_000));
    assert_eq!(got, want, "the voice resumes where it froze");
}

#[test]
fn without_a_soundfont_the_player_is_silent() {
    let mut player = SoundFontPlayer::new(RATE, None);
    player.note_on(60, 1.0, 0.5);
    let (left, right) = render(&mut player, 1_024);
    assert_eq!(peak(&left) + peak(&right), 0.0);
}

#[test]
fn one_large_block_and_many_odd_blocks_give_the_same_output() {
    let run = |block| {
        let mut player = player(sine_bank(), 0);
        player.note_on(69, 0.8, 0.3);
        player.note_on(76, 0.6, 0.7);
        let first = render_blocks(&mut player, 10_000, block);
        player.note_off(69);
        let second = render_blocks(&mut player, 10_000, block);
        (first, second)
    };
    let big = run(10_000);
    for block in [1, 7, 333, 1023] {
        assert_eq!(run(block), big, "block {block}");
    }
}

#[test]
fn it_plays_at_44_1_khz_too() {
    let mut player = SoundFontPlayer::new(44_100.0, Some((font(sine_bank()), 0)));
    player.note_on(69, 1.0, 0.5);
    let (mut left, mut right) = (vec![0.0; 22_050], vec![0.0; 22_050]);
    player.process(&mut left, &mut right, context(44_100.0));
    let rising = (1..left.len())
        .filter(|&i| left[i - 1] <= 0.0 && left[i] > 0.0)
        .count();
    assert!(rising.abs_diff(220) <= 1, "{rising} cycles in 0.5 s");
}

// ---- hostile files ----

#[test]
fn broken_files_are_errors() {
    let good = sine_bank();
    assert!(SoundFont::parse(good.clone()).is_ok());
    assert_eq!(
        SoundFont::parse(Vec::new()).err(),
        Some(ParseError::NotSoundFont)
    );
    assert_eq!(
        SoundFont::parse(b"RIFF\x04\0\0\0WAVE".to_vec()).err(),
        Some(ParseError::NotSoundFont)
    );
    for cut in [12, 40, good.len() / 2, good.len() - 20] {
        assert_eq!(
            SoundFont::parse(good[..cut].to_vec()).err(),
            Some(ParseError::Corrupted),
            "cut at {cut}"
        );
    }
    // A bag index past its table, and indices that go backwards.
    let at = |bytes: &[u8], id: &[u8; 4]| bytes.windows(4).position(|w| w == id).unwrap() + 8;
    let mut bad = good.clone();
    let pbag = at(&bad, b"pbag");
    bad[pbag..pbag + 2].copy_from_slice(&900_u16.to_le_bytes());
    assert_eq!(SoundFont::parse(bad).err(), Some(ParseError::Corrupted));
    let mut bad = good.clone();
    let phdr = at(&bad, b"phdr");
    bad[phdr + 38 + 24..phdr + 38 + 26].copy_from_slice(&0_u16.to_le_bytes());
    bad[phdr + 24..phdr + 26].copy_from_slice(&1_u16.to_le_bytes());
    assert_eq!(SoundFont::parse(bad).err(), Some(ParseError::Corrupted));
    // A huge chunk size.
    let mut bad = good.clone();
    let shdr = at(&bad, b"shdr") - 4;
    bad[shdr..shdr + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(SoundFont::parse(bad).err(), Some(ParseError::Corrupted));
}

#[test]
fn zones_that_name_nothing_usable_are_skipped() {
    // Sample 1 and instrument 3 do not exist; sample 0's rate is 0 in the second bank.
    let bytes = soundfont(
        &[vec![
            vec![amount(INSTRUMENT, 0)],
            vec![amount(INSTRUMENT, 3)],
        ]],
        &[vec![
            vec![amount(SAMPLE, 1)],
            vec![amount(MODES, 1), amount(SAMPLE, 0)],
        ]],
        &[sine_sample()],
    );
    assert_eq!(font(bytes).regions(0).len(), 1);
    let mut silent = sine_sample();
    silent.rate = 0;
    let bytes = one_instrument(vec![], vec![vec![amount(SAMPLE, 0)]], &[silent]);
    assert_eq!(font(bytes).regions(0).len(), 0);
    // Sample bounds past the data.
    let mut bytes = sine_bank();
    let shdr = bytes.windows(4).position(|w| w == b"shdr").unwrap() + 8;
    bytes[shdr + 24..shdr + 28].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(font(bytes).regions(0).len(), 0);
    // A ROM sample (sampleType bit 15): its data is not in the file.
    let mut bytes = sine_bank();
    bytes[shdr + 44..shdr + 46].copy_from_slice(&0x8001_u16.to_le_bytes());
    assert_eq!(font(bytes).regions(0).len(), 0);
}

#[test]
fn a_preset_that_multiplies_zones_stops_at_the_region_cap() {
    // 3000 preset zones on one instrument of 3000 zones would be 9 million regions.
    let preset = vec![vec![amount(INSTRUMENT, 0)]; 3_000];
    let zones = vec![vec![amount(MODES, 1), amount(SAMPLE, 0)]; 3_000];
    let bytes = soundfont(&[preset], &[zones], &[sine_sample()]);
    let font = font(bytes);
    let mut regions = 0;
    let largest = largest_during(|| regions = font.regions(0).len());
    assert_eq!(regions, MAX_REGIONS);
    assert!(largest < 4 << 20, "{largest} bytes");
}

#[test]
fn resolving_a_patch_stops_once_its_work_budget_is_spent() {
    // Instrument 0: 1000 zones naming a sample that does not exist, 2 units of work each. Instrument 1: one zone
    // that plays. `pairs` preset zones on instrument 0, then one on instrument 1: 2000 (4.0 M units) leave room for
    // its region; 2100 (4.2 M) spend the 2^22 budget first.
    let regions = |pairs| {
        let mut preset = vec![vec![amount(INSTRUMENT, 0)]; pairs];
        preset.push(vec![amount(INSTRUMENT, 1)]);
        let instruments = [
            vec![vec![amount(SAMPLE, 9)]; 1_000],
            vec![vec![amount(MODES, 1), amount(SAMPLE, 0)]],
        ];
        let bytes = soundfont(&[preset], &instruments, &[sine_sample()]);
        font(bytes).regions(0).len()
    };
    assert_eq!(regions(2_000), 1);
    assert_eq!(regions(2_100), 0);
}

#[test]
fn hostile_sizes_allocate_nothing_large() {
    let good = sine_bank();
    let mut huge = good.clone();
    huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut chunks = good.clone();
    for id in [
        b"smpl", b"phdr", b"pbag", b"pgen", b"inst", b"ibag", b"igen", b"shdr",
    ] {
        let at = chunks.windows(4).position(|w| w == id).unwrap();
        chunks[at + 4..at + 8].copy_from_slice(&0x7fff_fff0_u32.to_le_bytes());
    }
    for bytes in [huge, chunks] {
        let len = bytes.len();
        let largest = largest_during(|| drop(SoundFont::parse(bytes)));
        assert!(largest <= 2 * len, "{largest} bytes for a {len}-byte file");
    }
    let many = vec![vec![]; MAX_PATCHES + 2];
    let bytes = soundfont(&many, &[], &[]);
    assert_eq!(preset_headers(&bytes).unwrap().len(), MAX_PATCHES);
    assert_eq!(font(bytes).presets().len(), MAX_PATCHES);
}

/// A bank with global zones, ranges, loops and two presets, for the fuzzer to break.
fn rich_bank() -> Vec<u8> {
    let mut short = sine_sample();
    short.data.truncate(120);
    short.loop_points = (20, 100);
    soundfont(
        &[
            vec![
                vec![amount(COARSE, 2), amount(PAN, -200)],
                vec![range(KEYS, 0, 80), amount(INSTRUMENT, 0)],
                vec![
                    range(VELS, 30, 127),
                    amount(ATTENUATION, 60),
                    amount(INSTRUMENT, 1),
                ],
            ],
            vec![vec![amount(RELEASE, -1200), amount(INSTRUMENT, 1)]],
        ],
        &[
            vec![
                vec![amount(SCALE, 80), amount(ATTACK, -3000)],
                vec![range(KEYS, 40, 90), amount(MODES, 1), amount(SAMPLE, 0)],
                vec![
                    amount(MODES, 3),
                    amount(2, 4),
                    amount(45, 1),
                    amount(SAMPLE, 1),
                ],
            ],
            vec![vec![
                amount(MODES, 1),
                amount(ROOT, 50),
                amount(SUSTAIN, 300),
                amount(SAMPLE, 1),
            ]],
        ],
        &[sine_sample(), short],
    )
}

#[test]
fn mutated_soundfonts_never_panic_and_play_finite_audio() {
    let good = rich_bank();
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let (mut parsed, mut sounding) = (0, 0);
    for round in 0..4_000 {
        let mut bytes = good.clone();
        for _ in 0..1 + next() % 8 {
            let at = next() as usize % bytes.len();
            bytes[at] = match next() % 4 {
                0 => 0xFF,
                1 => 0,
                _ => next() as u8,
            };
        }
        if round % 5 == 0 {
            bytes.truncate(next() as usize % bytes.len());
        }
        // The patches are listed exactly when the SoundFont parses: a bank that lists its patches plays them.
        let listed = preset_headers(&bytes).is_ok();
        let parse = SoundFont::parse(bytes);
        assert_eq!(listed, parse.is_ok(), "round {round}");
        let Ok(font) = parse else {
            continue;
        };
        parsed += 1;
        let font = Arc::new(font);
        for patch in [0, 1] {
            let mut player = SoundFontPlayer::new(RATE, Some((font.clone(), patch)));
            for key in [0, 45, 69, 127] {
                player.note_on(key, (next() % 128) as f32 / 127.0, 0.5);
            }
            player.set_controller(4, 1.0);
            let (left, right) = render(&mut player, 512);
            player.all_notes_off();
            let (left2, right2) = render(&mut player, 512);
            let all = [left, right, left2, right2].concat();
            assert!(all.iter().all(|s| s.is_finite()), "round {round}");
            sounding += usize::from(peak(&all) > 0.0);
        }
    }
    assert!(
        parsed > 400 && sounding > 100,
        "{parsed} parsed, {sounding} sounding"
    );
}

/// The top-level chunks of `bytes` after the `RIFF` header: each one's bytes, a list's pad byte included.
fn top_chunks(bytes: &[u8]) -> Vec<&[u8]> {
    let (mut at, mut chunks) = (12, Vec::new());
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let end = (at + 8 + size + size % 2).min(bytes.len());
        chunks.push(&bytes[at..end]);
        at = end;
    }
    chunks
}

/// `bank` as a truncated download of a file that keeps `pdta` before `sdta`: every table is there, but the file ends
/// halfway through the sample data its `sdta` list announces.
fn cut_off_samples(bank: &[u8]) -> Vec<u8> {
    let chunks = top_chunks(bank);
    let kind = |c: &&[u8]| c[8..12].to_vec();
    let find = |wanted: &[u8; 4]| *chunks.iter().find(|c| kind(c) == wanted).unwrap();
    let (info, pdta, sdta) = (find(b"INFO"), find(b"pdta"), find(b"sdta"));
    let mut out = [&bank[..12], info, pdta].concat();
    out.extend(&sdta[..sdta.len() / 2]);
    out
}

#[test]
fn a_bank_whose_patch_headers_are_whole_but_whose_samples_are_cut_off_does_not_list_its_patches() {
    // The header-only reader once listed the patches of a bank the full parse rejected, so the
    // rack offered patches that played silence.
    let cut = cut_off_samples(&sine_bank());
    assert!(cut.len() < sine_bank().len());
    assert_eq!(
        SoundFont::parse(cut.clone()).err(),
        Some(ParseError::Corrupted)
    );
    assert_eq!(preset_headers(&cut), Err(ParseError::Corrupted));
    // The same bank whole lists its patch.
    assert_eq!(preset_headers(&sine_bank()).unwrap().len(), 1);
}

#[test]
fn a_bank_missing_a_table_or_with_bags_that_go_back_does_not_list_its_patches_either() {
    let good = sine_bank();
    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    // Each table other than `phdr` renamed away: missing.
    for id in [
        b"smpl", b"pbag", b"pgen", b"inst", b"ibag", b"igen", b"shdr",
    ] {
        let mut bytes = good.clone();
        let at = bytes.windows(4).position(|w| w == id).unwrap();
        bytes[at] = b'x';
        cases.push((String::from_utf8_lossy(id).into_owned(), bytes));
    }
    // A preset bag index that runs past the zones.
    let mut bytes = good.clone();
    let at = bytes.windows(4).position(|w| w == b"phdr").unwrap() + 8 + 24;
    bytes[at..at + 2].copy_from_slice(&200_u16.to_le_bytes());
    cases.push(("phdr bag index".into(), bytes));
    for (what, bytes) in cases {
        assert_eq!(
            SoundFont::parse(bytes.clone()).err(),
            Some(ParseError::Corrupted),
            "{what}"
        );
        assert_eq!(preset_headers(&bytes), Err(ParseError::Corrupted), "{what}");
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    for _ in 0..2_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let len = (state % 300) as usize;
        let mut bytes: Vec<u8> = (0..len).map(|i| (state >> (i % 56)) as u8).collect();
        if len >= 12 {
            bytes[..4].copy_from_slice(b"RIFF");
            bytes[8..12].copy_from_slice(b"sfbk");
        }
        let _ = preset_headers(&bytes);
        let _ = SoundFont::parse(bytes);
    }
}

/// The app's tests play `tests/fixtures/sine.sf2`: [`sine_bank`] as this builder writes it. `WRITE_SF2_FIXTURE=1`
/// writes it again.
#[test]
fn the_sine_fixture_is_the_builders_output() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sine.sf2");
    if std::env::var_os("WRITE_SF2_FIXTURE").is_some() {
        std::fs::write(path, sine_bank()).unwrap();
    }
    assert_eq!(std::fs::read(path).unwrap(), sine_bank());
}

#[test]
fn any_nonzero_envelope_value_plays_custom() {
    // Playback is Custom at v ≠ 0, though the text reads
    // `Original` up to 0.5. Custom Attack 0.2 against the SoundFont's own attack (1 s timecent).
    let out = |v: f32| {
        let mut player = player(level_bank(vec![], vec![amount(ATTACK, 0)]), 0);
        player.set_parameter(1, 0.2);
        player.set_parameter(5, v);
        player.note_on(60, 1.0, 0.5);
        render(&mut player, 9_600).0
    };
    let custom = out(1.0);
    assert!(out(0.0) != custom);
    for v in [0.5, 0.001] {
        assert!(out(v) == custom, "{v} plays Custom");
    }
}

#[test]
fn a_soundfont_reports_the_length_of_the_file_it_keeps() {
    // The app counts these bytes against its memory budget (`sample_cache`).
    let bytes = sine_bank();
    let len = bytes.len();
    assert_eq!(font(bytes).byte_len(), len);
}

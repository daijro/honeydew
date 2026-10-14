//! Real-time safety: nothing the audio thread calls allocates or frees. A counting global
//! allocator counts per thread, so the test threads don't see each other. Each render path adds its scenario here.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use honeydew_audio::analysis::analyzer::Setting;
use honeydew_audio::{AudioEffect, ProcessContext, effect_info};

struct Counting;

thread_local! {
    /// Allocations, reallocations and frees on this thread.
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    // A const thread-local without a destructor is always there, even while the thread is torn down.
    let _ = CALLS.try_with(|calls| calls.set(calls.get() + 1));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count();
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// How many allocator calls `f` makes on this thread.
fn allocator_calls(f: impl FnOnce()) -> usize {
    let before = CALLS.with(Cell::get);
    f();
    CALLS.with(Cell::get) - before
}

const SAMPLE_RATE: f32 = 48_000.0;
const COMMON: [u16; 3] = [0x7fc0, 0x7fc1, 0x7fc2];

/// Odd block sizes, so every block-rate path runs at several lengths.
const BLOCKS: [usize; 4] = [1, 37, 256, 1023];

/// An effect driven like the audio thread drives it: a tone gliding between 220 and 440 Hz every 2 s, in odd blocks
/// that cycle on across calls.
struct Rig {
    effect: AudioEffect,
    buffers: [Vec<f32>; 2],
    beat: f64,
    block: usize,
    frame: usize,
    phase: f32,
    /// Every output sample's bits, folded.
    checksum: u64,
}

impl Rig {
    fn new(id: u16) -> Self {
        Self {
            effect: AudioEffect::new(id, SAMPLE_RATE).unwrap(),
            buffers: [vec![0.0; 1024], vec![0.0; 1024]],
            beat: 0.0,
            block: 0,
            frame: 0,
            phase: 0.0,
            checksum: 0,
        }
    }

    /// Renders about `seconds` of audio at `tempo`.
    fn render(&mut self, tempo: f32, seconds: f32) {
        let end = self.frame + (seconds * SAMPLE_RATE) as usize;
        while self.frame < end {
            let frames = BLOCKS[self.block % BLOCKS.len()];
            self.block += 1;
            let [left, right] = &mut self.buffers;
            for (i, (l, r)) in left[..frames]
                .iter_mut()
                .zip(&mut right[..frames])
                .enumerate()
            {
                let t = (self.frame + i) as f32 / SAMPLE_RATE;
                let octave = 1.0 - (2.0 * (0.5 * t).fract() - 1.0).abs();
                self.phase = (self.phase + 220.0 * octave.exp2() / SAMPLE_RATE).fract();
                *l = 0.5 * (std::f32::consts::TAU * self.phase).sin();
                *r = -*l;
            }
            let context = ProcessContext {
                sample_rate: SAMPLE_RATE,
                tempo_bpm: tempo,
                start_beat: self.beat,
                playing: true,
            };
            self.effect
                .process(&mut left[..frames], &mut right[..frames], context)
                .unwrap();
            for sample in left[..frames].iter().chain(&right[..frames]) {
                self.checksum = self
                    .checksum
                    .wrapping_mul(31)
                    .wrapping_add(u64::from(sample.to_bits()));
            }
            self.frame += frames;
            self.beat += f64::from(tempo) / 60.0 * frames as f64 / f64::from(SAMPLE_RATE);
        }
    }
}

/// Audio after each setting (seconds).
const SETTLE: f32 = 0.5;

/// The allocator calls of `job` on a new `id` effect, after a first short render. Everything after construction runs
/// on the audio thread, the first block included.
fn counted(id: u16, job: impl FnOnce(&mut Rig)) -> usize {
    let mut rig = Rig::new(id);
    allocator_calls(|| {
        rig.render(120.0, 0.05);
        job(&mut rig);
    })
}

/// Param `index` at 0, ½ and 1, then back to its default (Pitcher 2 p25 goes Off → On → Off).
fn sweep_param(rig: &mut Rig, index: usize) {
    let default = rig.effect.parameter(index).unwrap();
    for value in [0.0, 0.5, 1.0, default] {
        rig.effect.set_parameter(index, value).unwrap();
        rig.render(120.0, SETTLE);
    }
}

/// The common params, the effect-specific inputs (each returns an error for the other effects), a tempo change, a
/// seek with a transport reset, and a block with no song beat.
fn sweep_the_rest(rig: &mut Rig) {
    for index in COMMON {
        for value in [0.0, 0.5, 1.0] {
            rig.effect.set_common_parameter(index, value).unwrap();
            rig.render(120.0, SETTLE);
        }
    }
    for note in [Some(60), None, Some(127)] {
        if rig.effect.set_comb_note(note).is_ok() {
            rig.render(120.0, SETTLE);
        }
    }
    for band in [Some(0), Some(2), None] {
        if rig.effect.set_multiband_solo_band(band).is_ok() {
            rig.render(120.0, SETTLE);
        }
    }
    // Pitcher note masks: D# minor, B alone, every note.
    let d_sharp_minor = std::array::from_fn(|n| [0, 2, 3, 5, 7, 8, 10].contains(&((n + 9) % 12)));
    for notes in [d_sharp_minor, std::array::from_fn(|n| n == 11), [true; 12]] {
        if rig.effect.set_pitcher_notes(notes).is_ok() {
            rig.render(120.0, SETTLE);
        }
    }
    // Analyzer settings: a source, the reactivities, the DC blocker on and off, a gain, a loudness mode (a reset).
    for setting in [
        Setting::Channel(0),
        Setting::Reactivity(0),
        Setting::DcBlock(true),
        Setting::InputGain(10.0),
        Setting::LoudnessMode(3),
        Setting::Reactivity(2),
        Setting::DcBlock(false),
        Setting::Channel(2),
    ] {
        if rig.effect.set_analyzer(setting).is_ok() {
            rig.render(120.0, SETTLE);
        }
    }
    rig.render(93.0, SETTLE);
    rig.effect.reset();
    rig.beat = 7.25;
    rig.render(120.0, SETTLE);
    rig.beat = -1.0;
    rig.render(120.0, 0.05);
    let _ = rig.effect.meter();
}

#[test]
fn effects_never_allocate_on_the_audio_thread() {
    // One thread per effect and param: the counts are per thread, and half a second per setting is slow in a debug
    // build (Pitcher runs at about half real time there).
    let failures: Vec<String> = std::thread::scope(|scope| {
        let mut jobs = Vec::new();
        for id in 0..=u16::MAX {
            let Some(info) = effect_info(id) else {
                continue;
            };
            for index in 0..info.parameter_count {
                let job = scope.spawn(move || counted(id, |rig| sweep_param(rig, index)));
                jobs.push((format!("{id} {} p{index}", info.name), job));
            }
            let job = scope.spawn(move || counted(id, sweep_the_rest));
            jobs.push((format!("{id} {} common and inputs", info.name), job));
        }
        jobs.into_iter()
            .filter_map(|(what, job)| {
                let calls = job.join().unwrap();
                (calls != 0).then(|| format!("{what}: {calls}"))
            })
            .collect()
    });
    assert!(
        failures.is_empty(),
        "allocator calls on the audio thread: {failures:?}"
    );
}

#[test]
fn re_applying_the_same_values_allocates_nothing_and_changes_nothing() {
    // The engine sets every param again at each loop wrap and seek, automated ones twice.
    for id in 0..=u16::MAX {
        let Some(info) = effect_info(id) else {
            continue;
        };
        let (mut plain, mut repeated) = (Rig::new(id), Rig::new(id));
        for rig in [&mut plain, &mut repeated] {
            let _ = rig.effect.set_pitcher_notes([true; 12]);
            // Pitcher: Stability (p8) holds the target, and with correction bypassed (p22) the gliding input's target
            // glides too, so a reset held target would sound.
            if id == 25 {
                rig.effect.set_parameter(8, 1.0).unwrap();
                rig.effect.set_parameter(22, 1.0).unwrap();
            }
        }
        let values: Vec<f32> = (0..info.parameter_count)
            .map(|i| plain.effect.parameter(i).unwrap())
            .collect();
        let mut same = true;
        let calls = allocator_calls(|| {
            for _ in 0..20 {
                plain.render(120.0, 0.05);
                for _ in 0..2 {
                    for (index, &value) in values.iter().enumerate() {
                        repeated.effect.set_parameter(index, value).unwrap();
                    }
                    let _ = repeated.effect.set_pitcher_notes([true; 12]);
                }
                repeated.render(120.0, 0.05);
                same &= plain.checksum == repeated.checksum;
            }
        });
        assert_eq!(calls, 0, "{id} {}", info.name);
        assert!(
            same,
            "{id} {}: re-applied values changed the output",
            info.name
        );
    }
}

#[test]
fn pitcher_2_switches_low_latency_without_allocating_after_a_sample_rate_change() {
    // The buffers are sized for the larger (normal) window at every rate; the rate changes off the audio thread.
    for (from, to) in [(44_100.0, 192_000.0), (96_000.0, 48_000.0)] {
        let mut effect = AudioEffect::new(45, from).unwrap();
        effect.set_parameter(25, 1.0).unwrap();
        effect.configure_sample_rate(to).unwrap();
        let (mut left, mut right) = (vec![0.1; 512], vec![0.1; 512]);
        let context = ProcessContext {
            sample_rate: to,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        };
        let calls = allocator_calls(|| {
            for low_latency in [0.0, 1.0, 0.0] {
                effect.set_parameter(25, low_latency).unwrap();
                effect.process(&mut left, &mut right, context).unwrap();
            }
        });
        assert_eq!(calls, 0, "{from} → {to} Hz");
    }
}

#[test]
fn tape_stop_follows_a_slower_tempo_without_allocating() {
    let mut rig = Rig::new(42);
    let calls = allocator_calls(|| rig.render(85.0, 0.05));
    assert_eq!(calls, 0);
}

#[test]
fn tape_stop_at_85_bpm_and_after_a_change_down_to_60_does_not_allocate() {
    // The host sizes the ring for the song tempo at build and for a slower tempo before it
    // plays, both off the audio thread (`configure_tempo`). A 16-beat synced Stop and Start at the slowest warp then
    // allocate nothing.
    let mut rig = Rig::new(42);
    for (index, value) in [(3, 1.0), (4, 1.0), (6, 0.0), (7, 1.0)] {
        rig.effect.set_parameter(index, value).unwrap();
    }
    for tempo in [85.0, 60.0] {
        rig.effect.configure_tempo(tempo).unwrap();
        let calls = allocator_calls(|| {
            for play in [0.0, 1.0] {
                rig.effect.set_parameter(5, play).unwrap();
                rig.render(tempo, 2.0);
            }
        });
        assert_eq!(calls, 0, "{tempo} BPM");
    }
}

#[test]
fn effect_tails_end_without_subnormals() {
    // Subnormal floats never reach 0 and cost the audio thread up to 100× per operation; every filter and feedback
    // state flushes them. 40 s of tail, at 8 kHz to keep the debug build quick (the decays are in seconds).
    const RATE: f32 = 8_000.0;
    let failures: Vec<String> = std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..=u16::MAX)
            .filter_map(|id| effect_info(id).map(|info| (id, info.name)))
            .map(|(id, name)| {
                let job = scope.spawn(move || {
                    let mut effect = AudioEffect::new(id, RATE).unwrap();
                    let frames = 40 * RATE as usize;
                    let mut left: Vec<f32> = (0..frames)
                        .map(|i| match i {
                            0..168 => 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / RATE).sin(),
                            _ => 0.0,
                        })
                        .collect();
                    let mut right = left.clone();
                    let context = ProcessContext {
                        sample_rate: RATE,
                        tempo_bpm: 120.0,
                        start_beat: 0.0,
                        playing: true,
                    };
                    for (l, r) in left.chunks_mut(1000).zip(right.chunks_mut(1000)) {
                        effect.process(l, r, context).unwrap();
                    }
                    let last = frames - RATE as usize..;
                    left[last.clone()]
                        .iter()
                        .chain(&right[last])
                        .filter(|v| v.is_subnormal())
                        .count()
                });
                (format!("{id} {name}"), job)
            })
            .collect();
        jobs.into_iter()
            .filter_map(|(what, job)| {
                let subnormal = job.join().unwrap();
                (subnormal != 0).then(|| format!("{what}: {subnormal}"))
            })
            .collect()
    });
    assert!(
        failures.is_empty(),
        "subnormal samples in the last second: {failures:?}"
    );
}

#[test]
fn every_setting_of_every_effect_ends_without_subnormals() {
    // Each param at 0, ½ and 1 and each common param likewise, the others at their defaults: 3 s of a square tone, then
    // 3 s of silence, at 8 kHz. A gain smoother heading to 0 shows in the tone's last second (the tone times a
    // subnormal), a filter or feedback tail in the silence's last second. Workers share the settings.
    const RATE: f32 = 8_000.0;
    let mut settings = Vec::new();
    for id in 0..=u16::MAX {
        let Some(info) = effect_info(id) else {
            continue;
        };
        for value in [0.0, 0.5, 1.0] {
            settings.extend((0..info.parameter_count).map(|index| (id, Some(index), 0, value)));
            settings.extend(COMMON.map(|common| (id, None, common, value)));
        }
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failures = std::sync::Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map_or(4, usize::from);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                while let Some(&(id, index, common, value)) =
                    settings.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                {
                    let mut effect = AudioEffect::new(id, RATE).unwrap();
                    match index {
                        Some(index) => effect.set_parameter(index, value).unwrap(),
                        None => effect.set_common_parameter(common, value).unwrap(),
                    }
                    let second = RATE as usize;
                    let mut left: Vec<f32> = (0..6 * second)
                        .map(|i| match i {
                            _ if i < 3 * second && (i / 9) % 2 == 0 => 0.5,
                            _ if i < 3 * second => -0.5,
                            _ => 0.0,
                        })
                        .collect();
                    let mut right: Vec<f32> = left.iter().map(|sample| -0.5 * sample).collect();
                    let context = ProcessContext {
                        sample_rate: RATE,
                        tempo_bpm: 120.0,
                        start_beat: 0.0,
                        playing: true,
                    };
                    for (l, r) in left.chunks_mut(1000).zip(right.chunks_mut(1000)) {
                        effect.process(l, r, context).unwrap();
                    }
                    let checked = [2 * second..3 * second, 5 * second..6 * second];
                    let subnormal = (checked.into_iter())
                        .flat_map(|range| left[range.clone()].iter().chain(&right[range]))
                        .filter(|sample| sample.is_subnormal())
                        .count();
                    if subnormal != 0 {
                        let name = effect_info(id).unwrap().name;
                        let setting =
                            index.map_or(format!("{common:#x}"), |index| format!("p{index}"));
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("{id} {name} {setting} = {value}: {subnormal}"));
                    }
                }
            });
        }
    });
    let mut failures = failures.into_inner().unwrap();
    failures.sort();
    assert!(failures.is_empty(), "subnormal samples: {failures:?}");
}

#[test]
fn gmsynth_never_allocates_on_the_audio_thread() {
    // Notes (steals, MONO legato and fallback), every param at 0, 0.5, 1 and 0.3, both controllers, the
    // `CSTM` settings on shapes the app thread built (and one past the menu), stop and play, all-notes-off and reset.
    use honeydew_audio::gmsynth::{
        GmSynth, MOD_WHEEL, MW_AMOUNT, MW_DEST, PARAMS, PB_RANGE, PITCH_BEND, WAVE_A, WAVE_B,
        prepare_setting,
    };
    use honeydew_audio::instrument::Instrument;
    let shapes = [3.0, 18.0, 26.0, 29.0];
    for slot in shapes {
        prepare_setting(WAVE_A, slot);
    }
    let mut synth = GmSynth::new(SAMPLE_RATE);
    let (mut left, mut right) = (vec![0.0; 1023], vec![0.0; 1023]);
    let mut energy = 0.0_f32;
    let mut beat = 0.0;
    let calls = allocator_calls(|| {
        for (n, &frames) in BLOCKS.iter().cycle().take(2000).enumerate() {
            let shape = shapes[n / 7 % shapes.len()];
            match n % 10 {
                0 => synth.note_on(40 + (n % 37) as u8, 0.8, 0.4),
                1 => synth.note_off(40 + ((n + 27) % 37) as u8),
                2 => synth.set_parameter(n / 10 % PARAMS, [0.0, 0.5, 1.0, 0.3][n / 750 % 4]),
                3 => synth.set_setting(WAVE_A, shape),
                4 => synth.set_setting(WAVE_B, shape),
                5 => synth.set_setting(PB_RANGE, (n % 4) as f32),
                6 => synth.set_setting(MW_DEST, (n % 3) as f32),
                7 => synth.set_setting(MW_AMOUNT, (n % 5) as f32 / 4.0),
                8 => synth.set_controller(PITCH_BEND, (n % 9) as f32 / 8.0),
                _ => synth.set_controller(MOD_WHEEL, (n % 6) as f32 / 5.0),
            }
            if n % 500 == 499 {
                synth.all_notes_off();
            }
            let context = ProcessContext {
                sample_rate: SAMPLE_RATE,
                tempo_bpm: 120.0,
                start_beat: beat,
                playing: n % 300 < 200,
            };
            synth.process(&mut left[..frames], &mut right[..frames], context);
            energy += left[..frames].iter().map(|s| s * s).sum::<f32>();
            beat += frames as f64 * 120.0 / (60.0 * f64::from(SAMPLE_RATE));
        }
        synth.reset();
    });
    assert_eq!(calls, 0, "allocator calls in GMSynth");
    assert!(energy > 0.0, "the counted render was silent");
}

#[test]
fn gmsynth_fx_slots_never_allocate_on_the_audio_thread() {
    // FX 1–3 all on, each stepping through all ten kinds (so every processor runs, winds down and clears),
    // with X, Y, Sync, LFO shape and amount moving, Enable toggling, the song stopping and playing, and a reset.
    use honeydew_audio::gmsynth::GmSynth;
    use honeydew_audio::instrument::Instrument;
    let mut synth = GmSynth::new(SAMPLE_RATE);
    for slot in 0..3 {
        synth.set_parameter(47 + 7 * slot, 1.0);
    }
    let (mut left, mut right) = (vec![0.0; 1023], vec![0.0; 1023]);
    let mut energy = 0.0_f32;
    let mut beat = 0.0;
    let calls = allocator_calls(|| {
        for (n, &frames) in BLOCKS.iter().cycle().take(1200).enumerate() {
            let slot = n / 12 % 3;
            let base = 46 + 7 * slot;
            match n % 12 {
                0 => synth.note_on(48 + (n % 25) as u8, 0.9, 0.5),
                1 => synth.note_off(48 + ((n + 13) % 25) as u8),
                2 => synth.set_parameter(base, ((n / 12 + slot * 3) % 10) as f32 / 9.0),
                3 => synth.set_parameter(base + 2, (n % 10) as f32 / 9.0),
                4 => synth.set_parameter(base + 3, (n % 3) as f32 / 2.0),
                5 => synth.set_parameter(base + 4, (n % 5) as f32 / 4.0),
                6 => synth.set_parameter(base + 5, (n % 7) as f32 / 6.0),
                7 => synth.set_parameter(base + 6, (n % 4) as f32 / 3.0),
                8 => synth.set_parameter(base + 1, f32::from(n % 72 != 8)),
                _ => {}
            }
            let context = ProcessContext {
                sample_rate: SAMPLE_RATE,
                tempo_bpm: 120.0,
                start_beat: beat,
                playing: n % 300 < 200,
            };
            synth.process(&mut left[..frames], &mut right[..frames], context);
            energy += left[..frames].iter().map(|s| s * s).sum::<f32>();
            beat += frames as f64 * 120.0 / (60.0 * f64::from(SAMPLE_RATE));
        }
        synth.reset();
    });
    assert_eq!(calls, 0, "allocator calls in the GMSynth FX slots");
    assert!(energy > 0.0, "the counted render was silent");
}

/// The DW Sampler: every param swept through its steps while notes start, end and slide, over a looping stereo
/// zone and a one-shot mono one, with transport, bend, reset and all-notes-off between blocks.
#[test]
fn dw_sampler_never_allocates_on_the_audio_thread() {
    use honeydew_audio::dw_sampler::{DwSampler, DwZone};
    use honeydew_audio::dwp::{Midi, Zone};
    use honeydew_audio::instrument::Instrument;
    use honeydew_audio::sample::SampleData;
    use std::sync::Arc;

    let wave: Arc<[f32]> = (0..20_000).map(|n| (n as f32 * 0.03).sin()).collect();
    let stereo = SampleData {
        sample_rate: 44_100,
        left: wave.clone(),
        right: Some(wave.clone()),
        meta: Default::default(),
    };
    let mono = SampleData {
        sample_rate: 22_050,
        left: wave,
        right: None,
        meta: Default::default(),
    };
    let looping = Zone {
        key_high: 70,
        loop_mode: 2,
        loop_start: 500,
        loop_end: 15_000,
        ..Zone::default()
    };
    let zones = vec![
        DwZone {
            zone: looping,
            sample: Arc::new(stereo),
        },
        DwZone {
            zone: Zone {
                key_low: 60,
                ..Zone::default()
            },
            sample: Arc::new(mono),
        },
    ];
    let mut dw = DwSampler::new(
        SAMPLE_RATE,
        zones,
        Midi {
            pb_range: 7,
            attack_flag: 1,
        },
    );
    let (mut left, mut right) = (vec![0.0; 1024], vec![0.0; 1024]);
    let mut beat = 0.0;
    let calls = allocator_calls(|| {
        for round in 0..34 * 6 {
            let (index, step) = (round % 34, round / 34);
            dw.set_parameter(index, step as f32 / 5.0);
            match round % 7 {
                0 => dw.note_on(40 + (round % 40) as u8, 0.9, 0.3),
                1 => dw.note_on(65 + (round % 9) as u8, 0.5, 0.8),
                2 => dw.note_off(40 + ((round + 33) % 40) as u8),
                3 => dw.set_controller(4, (round % 5) as f32 / 4.0),
                4 if round % 3 == 0 => dw.all_notes_off(),
                5 if round % 11 == 0 => dw.reset(),
                _ => {}
            }
            let frames = BLOCKS[round % BLOCKS.len()];
            let context = ProcessContext {
                sample_rate: SAMPLE_RATE,
                tempo_bpm: 128.0,
                start_beat: beat,
                playing: round % 4 != 0,
            };
            dw.process(&mut left[..frames], &mut right[..frames], context);
            beat += frames as f64 * 128.0 / 60.0 / f64::from(SAMPLE_RATE);
            assert!(
                left[..frames]
                    .iter()
                    .chain(&right[..frames])
                    .all(|s| s.is_finite())
            );
        }
    });
    assert_eq!(calls, 0, "allocator calls on the audio thread");
}

/// The 3x Osc scheduler's fixed pools drop what does not fit, without panicking or allocating: 60 keys × 10 echoes ask
/// for 600 pending echoes and get [`ECHO_SLOTS`]; 40 keys on a fixed chord ask for 40 arps and get [`MAX_ARPS`].
#[test]
fn three_osc_pools_drop_extras_without_allocating() {
    use honeydew_audio::three_osc::arp::{ArpEcho, MAX_ARPS};
    use honeydew_audio::three_osc::echo::{ECHO_SLOTS, ECHO_TAGS};
    use honeydew_audio::three_osc::{Params, Scheduler, Tick, VoiceEvent};

    let mut echo: Params = [0.5; 149];
    echo[120] = 0.0; // Mono off
    echo[130] = 1.0; // 10 echoes
    echo[137] = 0.0; // Ping Pong off
    echo[138] = 0.0; // Fat Mode off
    echo[139] = 0.0; // Arp off: keys go through Echo
    let mut arp = echo;
    arp[139] = 0.2; // Up
    arp[145] = 3.0 / 90.0; // Major: one arp per key
    let at = |count| Tick {
        bpm: 120.0,
        d: 4.0,
        count,
        beat: 0.0,
        playing: true,
    };
    let mut scheduler = ArpEcho::default();
    let (mut echoes, mut arp_keys) = (0, [false; 128]);
    let calls = allocator_calls(|| {
        for key in 0..60 {
            scheduler.key_on(key, 1.0, 0.5, &echo, &at(0), &mut |_| {});
        }
        for key in 0..60 {
            scheduler.key_off(key, &echo, &at(1), &mut |_| {});
        }
        for count in 0..2000 {
            scheduler.tick(&echo, &at(count), &mut |e| {
                if let VoiceEvent::On { tag, .. } = e
                    && tag >= ECHO_TAGS
                {
                    echoes += 1;
                }
            });
        }
        scheduler.reset();
        for key in 0..40 {
            scheduler.key_on(key, 1.0, 0.5, &arp, &at(0), &mut |_| {});
        }
        scheduler.tick(&arp, &at(0), &mut |e| {
            if let VoiceEvent::On { note, .. } = e {
                arp_keys[usize::from(note.key)] = true;
            }
        });
    });
    assert_eq!(calls, 0, "allocator calls on the audio thread");
    assert_eq!(echoes, ECHO_SLOTS);
    assert_eq!(arp_keys.iter().filter(|&&k| k).count(), MAX_ARPS);
}

#[test]
fn a_streaming_pad_plays_without_allocating_while_its_frames_arrive() {
    // A Stretch re-import's buffer streaming in: the voices read the frames written so
    // far and silence past them; the worker's writes in between are not the audio thread's.
    use std::sync::Arc;

    use honeydew_audio::drums::{Pad, PadSample, PadSampler, STREAM_FRAMES, Stream};

    let frames = 4 * STREAM_FRAMES;
    let stream = Arc::new(Stream::new(SAMPLE_RATE as u32, frames, true));
    let data = vec![0.5; frames];
    stream.push(&data[..STREAM_FRAMES], Some(&data[..STREAM_FRAMES]));
    let pad = Pad {
        sample: Some(PadSample::Streaming(stream.clone())),
        ..Pad::default()
    };
    let mut sampler = PadSampler::new(vec![pad], SAMPLE_RATE);
    let (mut left, mut right) = (vec![0.0; 1024], vec![0.0; 1024]);
    let (mut calls, mut heard) = (0, 0.0f32);
    for (block, &size) in BLOCKS.iter().cycle().take(120).enumerate() {
        if block % 16 == 0 {
            calls += allocator_calls(|| sampler.hit(0, 1.0, 0.5, 1.0));
        }
        if block % 30 == 29 {
            let at = stream.filled();
            let to = (at + STREAM_FRAMES).min(frames);
            stream.push(&data[at..to], Some(&data[at..to]));
        }
        let (l, r) = (&mut left[..size], &mut right[..size]);
        calls += allocator_calls(|| sampler.process(0, l, r));
        heard = heard.max(l.iter().fold(0.0, |m, s| m.max(s.abs())));
    }
    assert_eq!(calls, 0);
    assert!(heard > 0.1, "it played");
}

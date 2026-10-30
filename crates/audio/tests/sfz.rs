//! SFZ in the SoundFont Player: the parser, the mapping rules and the voices, offline on SFZs and samples built here.

use std::sync::Arc;

use honeydew_audio::ProcessContext;
use honeydew_audio::flac_writer::flac;
use honeydew_audio::instrument::Instrument;
use honeydew_audio::sample::{SampleData, decode};
use honeydew_audio::sfz::{LoopMode, SfzRegion, Trigger, parse};
use honeydew_audio::soundfont::{SoundFont, SoundFontPlayer};

fn no_includes(_: &str) -> Option<String> {
    None
}

// ---- the parser ----

#[test]
fn headers_inherit_from_global_master_and_group_and_a_new_level_resets_the_ones_below() {
    let text = "
        <control> default_path=Samples/Piano Hall/ note_offset=0
        <global> volume=-6 ampeg_release=0.5
        <master> pan=-20
        <group> lovel=1 hivel=64 tune=10
        <region> sample=C4 soft.wav key=60
        <region> sample=D4.wav lokey=61 hikey=63 pitch_keycenter=62 tune=-5
        <group> lovel=65 hivel=127
        <region> sample=C4 loud.wav key=c4
        <master>
        <region> sample=*sine
    ";
    let sfz = parse(text, &no_includes);
    let r = &sfz.regions;
    assert_eq!(r.len(), 4);
    // A path with spaces runs to the next opcode; `default_path` goes in front.
    assert_eq!(r[0].sample, "Samples/Piano Hall/C4 soft.wav");
    assert_eq!(
        (r[0].keys, r[0].pitch_keycenter, r[0].vels),
        ([60, 60], 60, [1, 64])
    );
    assert_eq!((r[0].volume, r[0].pan, r[0].tune), (-6.0, -20.0, 10.0));
    assert_eq!(r[0].ampeg[5], 0.5);
    // The region's own value wins over its group's.
    assert_eq!(
        (r[1].keys, r[1].pitch_keycenter, r[1].tune),
        ([61, 63], 62, -5.0)
    );
    // A new group drops the old group's opcodes, not the master's.
    assert_eq!(
        (r[2].vels, r[2].tune, r[2].pan, r[2].keys),
        ([65, 127], 0.0, -20.0, [60, 60])
    );
    // A new master drops the master's and the group's; the global stays. Built-in samples keep their name.
    assert_eq!(
        (r[3].sample.as_str(), r[3].pan, r[3].volume),
        ("*sine", 0.0, -6.0)
    );
    assert_eq!(r[3].vels, [1, 127]);
}

#[test]
fn defines_includes_comments_note_names_and_offsets() {
    let include = |name: &str| match name {
        "inc/layer.sfz" => Some(
            "#define $HI 72\n<region> sample=$NAME.wav lokey=$LO hikey=$HI // the top\n".into(),
        ),
        _ => None,
    };
    let text = "
        /* a block comment
           <region> sample=commented.wav */
        #define $NAME Grand C
        #define $LO c#4
        <control> octave_offset=1 note_offset=-2
        #include \"inc/layer.sfz\"
        #include \"missing.sfz\"
        <region> sample=x\\y.wav key=bb3 // a comment with sample=z.wav
    ";
    let sfz = parse(text, &include);
    let r = &sfz.regions;
    assert_eq!(r.len(), 2, "{r:?}");
    assert_eq!(r[0].sample, "Grand C.wav");
    // c#4 is 61 and 72 is 72, then the offsets add 12 − 2.
    assert_eq!(r[0].keys, [71, 82]);
    // Backslashes are separators. Bb3 is 58.
    assert_eq!(
        (r[1].sample.as_str(), r[1].keys, r[1].pitch_keycenter),
        ("x/y.wav", [68, 68], 68)
    );
}

#[test]
fn includes_stop_at_a_depth_and_a_file_that_includes_itself_ends() {
    let include = |_: &str| Some("<region> sample=a.wav\n#include \"self.sfz\"\n".into());
    let sfz = parse("#include \"self.sfz\"", &include);
    assert_eq!(sfz.regions.len(), honeydew_audio::sfz::MAX_INCLUDE_DEPTH);
}

#[test]
fn unknown_opcodes_and_headers_are_skipped_and_counted() {
    let text = "
        <curve> curve_index=7 v000=0
        <effect> type=reverb
        <group> cutoff=500 fil_type=lpf_2p lochan=2 hichan=3 loop_mode=loop_sustain
        <region> sample=a.wav cutoff=800 trigger=release off_by=3 group=2 seq_length=3 seq_position=2
        <region> sample=b.wav lorand=0.5 hirand=1 sw_last=c1 sw_lokey=24 sw_hikey=30 sw_default=25
        <region> sample=c.wav xfin_lovel=10 xfin_hivel=50 amp_velcurve_64=0.3 amp_veltrack=50 loopstart=5 loopend=90 offset=3 end=99
        <bogus> sample=ignored.wav
        <region> lokey=1 // no sample: no region
    ";
    let sfz = parse(text, &no_includes);
    assert_eq!(sfz.regions.len(), 3);
    // Counted per region that plays: the group's in three, and the first region's own.
    assert_eq!(sfz.skipped.get("cutoff"), Some(&4), "{:?}", sfz.skipped);
    assert_eq!(sfz.skipped.get("fil_type"), Some(&3));
    // Channels are not skipped but ignored: notes reach the player without one.
    assert!(!sfz.skipped.contains_key("lochan") && !sfz.skipped.contains_key("type"));
    let [a, b, c] = [&sfz.regions[0], &sfz.regions[1], &sfz.regions[2]];
    assert_eq!(
        (a.trigger, a.group, a.off_by, a.loop_mode),
        (Trigger::Release, 2, Some(3), Some(LoopMode::Sustain))
    );
    assert_eq!((a.seq_length, a.seq_position), (3, 2));
    assert_eq!(
        (b.rand, b.sw_last, b.sw_keys, b.sw_default),
        ([0.5, 1.0], Some(24), [24, 30], Some(25))
    );
    assert_eq!(
        (c.xfin, c.amp_velcurve.clone(), c.amp_veltrack),
        ([10, 50], vec![(64, 0.3)], 50.0)
    );
    assert_eq!(
        (c.loop_start, c.loop_end, c.offset, c.end),
        (Some(5), Some(90), 3, Some(99))
    );
    assert_eq!(SfzRegion::default().trigger, Trigger::Attack);
}

// ---- playing: the regions a note starts ----

const RATE: f32 = 48_000.0;

fn context() -> ProcessContext {
    ProcessContext {
        sample_rate: RATE,
        tempo_bpm: 120.0,
        start_beat: 0.0,
        playing: true,
    }
}

fn render(player: &mut SoundFontPlayer, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    for (l, r) in left.chunks_mut(256).zip(right.chunks_mut(256)) {
        player.process(l, r, context());
    }
    (left, right)
}

/// A looped constant level of `n`/256: which regions sound adds up to a bit mask.
fn level(n: u32) -> Arc<SampleData> {
    let mut sample = SampleData {
        sample_rate: 48_000,
        left: vec![n as f32 / 256.0; 48_000].into(),
        right: None,
        meta: Default::default(),
    };
    sample.meta.loop_frames = Some((0, 47_999));
    Arc::new(sample)
}

/// A player of the SFZ `text` whose regions each play the level their `sample=` names (`1`, `2`, `4`...), at full
/// level whatever the velocity.
fn levels(text: &str) -> SoundFontPlayer {
    let sfz = parse(text, &no_includes);
    let regions = (sfz.regions.into_iter())
        .map(|r| {
            let n = r.sample.parse().unwrap();
            let r = SfzRegion {
                amp_veltrack: 0.0,
                ..r
            };
            (r, level(n))
        })
        .collect();
    SoundFontPlayer::new(RATE, Some((Arc::new(SoundFont::sfz("Test", regions)), 0)))
}

/// The regions sounding after `frames`, as the sum of their sample names.
fn sounding(player: &mut SoundFontPlayer, frames: usize) -> u32 {
    let (left, _) = render(player, frames);
    (left[frames - 1] * 256.0).round() as u32
}

/// The regions `key` at MIDI velocity `velocity` starts, the others stopped first.
fn starts(player: &mut SoundFontPlayer, key: u8, velocity: u8) -> u32 {
    player.reset();
    player.note_on(key, f32::from(velocity) / 127.0, 0.5);
    sounding(player, 64)
}

#[test]
fn key_and_velocity_ranges_pick_the_regions() {
    let mut player = levels(
        "<region> sample=1 lokey=c4 hikey=e4 hivel=63
         <region> sample=2 lokey=60 hikey=64 lovel=64
         <region> sample=4 key=65
         <region> sample=8 lokey=-1 hikey=-1",
    );
    assert_eq!(starts(&mut player, 60, 40), 1);
    assert_eq!(starts(&mut player, 64, 64), 2);
    assert_eq!(starts(&mut player, 64, 63), 1);
    assert_eq!(starts(&mut player, 65, 100), 4);
    assert_eq!(starts(&mut player, 59, 100), 0);
}

#[test]
fn round_robin_takes_turns_on_each_note() {
    let mut player = levels(
        "<group> seq_length=3 key=60
         <region> sample=1 seq_position=1
         <region> sample=2 seq_position=2
         <region> sample=4 seq_position=3",
    );
    let turns: Vec<u32> = (0..5).map(|_| starts(&mut player, 60, 100)).collect();
    assert_eq!(turns, [1, 2, 4, 1, 2]);
}

#[test]
fn a_release_region_plays_on_note_off_at_the_note_on_velocity() {
    let mut player = levels(
        "<region> sample=1 key=60
         <region> sample=2 key=60 trigger=release lovel=100 loop_mode=one_shot",
    );
    assert_eq!(starts(&mut player, 60, 110), 1);
    player.note_off(60);
    // The attack voice releases (ampeg_release 1 ms), the release voice starts.
    assert_eq!(sounding(&mut player, 4800), 2);
    // A soft note's release is below the release region's velocities.
    assert_eq!(starts(&mut player, 60, 50), 1);
    player.note_off(60);
    assert_eq!(sounding(&mut player, 4800), 0);
}

#[test]
fn off_by_chokes_the_other_groups_voices() {
    let mut player = levels(
        "<region> sample=1 key=42 group=1 off_by=2
         <region> sample=2 key=46 group=2
         <region> sample=4 key=48 group=3",
    );
    player.note_on(42, 1.0, 0.5);
    player.note_on(48, 1.0, 0.5);
    assert_eq!(sounding(&mut player, 64), 5);
    // 46 (group 2) stops 42 (off_by 2), not 48.
    player.note_on(46, 1.0, 0.5);
    assert_eq!(sounding(&mut player, 4800), 6);
}

#[test]
fn a_keyswitch_picks_the_regions_until_the_next_one() {
    let mut player = levels(
        "<global> sw_lokey=24 sw_hikey=26 sw_default=24
         <region> sample=1 key=60 sw_last=24
         <region> sample=2 key=60 sw_last=25
         <region> sample=4 key=61",
    );
    assert_eq!(starts(&mut player, 60, 100), 1);
    // A keyswitch plays nothing here.
    assert_eq!(starts(&mut player, 25, 100), 0);
    assert_eq!(starts(&mut player, 60, 100), 2);
    // A region without `sw_last` plays whatever the switch.
    assert_eq!(starts(&mut player, 61, 100), 4);
    starts(&mut player, 24, 100);
    assert_eq!(starts(&mut player, 60, 100), 1);
}

// ---- playing: the samples ----

/// The frequency from rising zero crossings over `samples` at `RATE`.
fn pitch(samples: &[f32]) -> f32 {
    let rising: Vec<usize> = (1..samples.len())
        .filter(|&i| samples[i - 1] <= 0.0 && samples[i] > 0.0)
        .collect();
    let (first, last) = (rising[0], *rising.last().unwrap());
    (rising.len() - 1) as f32 * RATE / (last - first) as f32
}

fn sine(hz: f64, rate: u32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| (0.5 * (std::f64::consts::TAU * hz * i as f64 / f64::from(rate)).sin()) as f32)
        .collect()
}

fn player_of(text: &str, sample: SampleData) -> SoundFontPlayer {
    let sample = Arc::new(sample);
    let regions = (parse(text, &no_includes).regions.into_iter())
        .map(|r| (r, sample.clone()))
        .collect();
    SoundFontPlayer::new(RATE, Some((Arc::new(SoundFont::sfz("Test", regions)), 0)))
}

#[test]
fn a_stereo_flac_plays_both_channels_at_the_right_pitch() {
    // 440 Hz left and 660 Hz right at 44.1 kHz, its root key 69.
    let rate = 44_100;
    let bytes = flac(
        rate,
        16,
        &[sine(440.0, rate, 88_200), sine(660.0, rate, 88_200)],
    );
    let sample = decode(&bytes).unwrap();
    assert!(sample.right.is_some());
    let mut player = player_of("<region> sample=a.flac pitch_keycenter=69", sample);
    player.note_on(69, 1.0, 0.5);
    let (left, right) = render(&mut player, 24_000);
    let (l, r) = (pitch(&left[2_400..]), pitch(&right[2_400..]));
    assert!(
        (l - 440.0).abs() < 1.0 && (r - 660.0).abs() < 1.5,
        "{l} Hz, {r} Hz"
    );
    // An octave up: both channels.
    player.reset();
    player.note_on(81, 1.0, 0.5);
    let (left, right) = render(&mut player, 24_000);
    let (l, r) = (pitch(&left[2_400..]), pitch(&right[2_400..]));
    assert!(
        (l - 880.0).abs() < 2.0 && (r - 1320.0).abs() < 3.0,
        "{l} Hz, {r} Hz"
    );
}

#[test]
fn loop_continuous_loops_and_no_loop_ends() {
    // 0.1 s of 480 Hz at 48 kHz: 100 frames a cycle, 48 cycles.
    let sample = || SampleData {
        sample_rate: 48_000,
        left: sine(480.0, 48_000, 4_800).into(),
        right: None,
        meta: Default::default(),
    };
    let tail = |text: &str| {
        let mut player = player_of(text, sample());
        player.note_on(60, 1.0, 0.5);
        let (left, _) = render(&mut player, 48_000);
        left[24_000..].iter().fold(0.0_f32, |m, s| m.max(s.abs()))
    };
    let looped =
        tail("<region> sample=a.wav loop_mode=loop_continuous loop_start=1200 loop_end=2399");
    assert!(looped > 0.4, "the loop plays on: {looped}");
    assert_eq!(tail("<region> sample=a.wav"), 0.0);
}

//! A module's sample picker in the app: `Custom` in a 3x Osc Shape menu opens it, its pick sets the
//! module's `CSTM` `PATH` with no undo step (the rack is not covered by undo), the OSC tab names the file, and the
//! rebuilt engine plays the shape.

use std::f64::consts::TAU;
use std::path::Path;

use eframe::egui::Id;

use super::*;
use crate::audio_engine::SongAudioEngine;
use crate::audio_engine::samples::shape_path;
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::sample_cache::SampleKey;
use crate::sample_cache::tests::wav;
use crate::ui::picker_preview::LOADING;
use honeydew_audio::builtin_shapes::VOWEL_A_FORMANTS;
use honeydew_audio::device::AudioRenderer;

/// The 3x Osc's channel (after the rig's MiniSynth), and its instrument's instance id.
const CHANNEL: usize = 2;
const OSC: i32 = 2;
/// Osc 1's Shape spinner (`#2.1` on the OSC tab).
const SHAPE_1: u32 = 12100;

/// [`Rig::with_synth`] with our factory (the rig leaves it out), a 3x Osc channel (2), its rack open, with an empty log.
fn with_three_osc() -> Rig {
    let mut rig = Rig::with_synth();
    rig.app.files.factory = crate::content::factory();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::ThreeXOsc), 0);
    rig.open_rack(CHANNEL);
    rig.slide_rack();
    rig.take();
    rig
}

fn control(n: u32) -> Id {
    Id::new(("rack", CHANNEL))
        .with(("module panel", OSC))
        .with(n)
}

/// Waits out the picker's slide-in and clicks its row `i`; a pick decoding on the worker lands before it returns.
fn picker_row(rig: &mut Rig, i: usize) {
    super::rack_wiring::picker_row(rig, i);
}

/// Osc 1's Shape `v` chevron: the next shape down the menu.
fn next_shape(rig: &mut Rig) {
    let r = rig.ctx.read_response(control(SHAPE_1)).unwrap().rect;
    rig.click(pos2(r.center().x, r.bottom() - 0.1 * U));
}

fn osc(rig: &Rig) -> &crate::song::ModuleInstance {
    rig.app.song.find_module(CHANNEL, OSC).unwrap()
}

fn picker() -> Overlay {
    picker_on(CHANNEL)
}

fn picker_on(channel: usize) -> Overlay {
    Overlay::ModuleSamplePicker {
        channel,
        module: OSC,
    }
}

fn user_path(relative: &str) -> ContentPath {
    ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: relative.into(),
    }
}

/// The message overlay's lines, if one shows.
fn message(rig: &Rig) -> Option<Vec<String>> {
    rig.overlays().into_iter().find_map(|o| match o {
        Overlay::Message { lines, .. } => Some(lines),
        _ => None,
    })
}

fn app_path(relative: &str) -> ContentPath {
    ContentPath::Shorthand {
        root: ContentRoot::App,
        relative: relative.into(),
    }
}

/// The texts the last frame drew.
fn texts(rig: &Rig) -> Vec<String> {
    let texts = rig.out.shapes.iter().filter_map(|s| match &s.shape {
        egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
        _ => None,
    });
    texts.collect()
}

/// Steps osc 1's Shape from Sine to `Custom`, the last item: only that step opens the picker.
fn pick_custom(rig: &mut Rig) {
    for _ in 0..5 {
        next_shape(rig);
        assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL)]);
    }
    next_shape(rig);
    assert_eq!(osc(rig).params[1], 1.0, "Custom");
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), picker()]);
}

#[test]
fn custom_opens_the_picker_whose_pick_sets_the_path_with_no_undo_step() {
    let mut rig = with_three_osc();
    let steps = rig.app.history.steps();
    pick_custom(&mut rig);
    rig.take();
    // My Files, then our six shapes in index order (`builtin_shapes::SHAPES` is definition order).
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    let shown = texts(&rig);
    let rows = [
        "My Files",
        "Glass",
        "Grit",
        "Reed",
        "Thin Pulse",
        "Vowel A",
        "Vowel O",
    ];
    let at = |name: &str| shown.iter().position(|t| t == name);
    let order: Vec<_> = rows
        .iter()
        .map(|r| at(r).unwrap_or_else(|| panic!("{r}")))
        .collect();
    assert!(order.is_sorted(), "{shown:?}");
    picker_row(&mut rig, 5);
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL)]);
    let vowel = app_path("Built-in/Shapes/Vowel A.wav");
    assert_eq!(shape_path(osc(&rig)), Some(vowel.clone()));
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // The OSC tab names it.
    rig.frame(vec![]);
    assert!(texts(&rig).iter().any(|t| t == "Vowel A"));
    // The `PATH` saves and loads.
    let read = crate::flm::read(&crate::flm::write(&rig.app.song).unwrap()).unwrap();
    assert_eq!(
        shape_path(read.find_module(CHANNEL, OSC).unwrap()),
        Some(vowel)
    );
    // No undo step, for the pick or for Custom.
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn a_cancel_changes_nothing_and_a_user_file_takes_its_user_path() {
    let mut rig = with_three_osc();
    let name = "A custom waveform with a long file name";
    let file = format!("My Samples/{name}.wav");
    rig.app
        .files
        .keep_user_file(Path::new(&file), wav(&[0, 9000, 0, -9000]));
    pick_custom(&mut rig);
    rig.frame(vec![]);
    let song = rig.app.song.clone();
    rig.take();
    rig.frame(escape());
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL)]);
    assert_eq!((rig.app.song == song, rig.take()), (true, vec![]));
    // Custom again (away and back) opens it again: My Files, My Samples, the file.
    rig.app.song.channels[CHANNEL]
        .rack
        .get_mut(OSC)
        .unwrap()
        .params[1] = 5.0 / 6.0;
    next_shape(&mut rig);
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), picker()]);
    for row in [0, 1, 0] {
        picker_row(&mut rig, row);
    }
    assert_eq!(shape_path(osc(&rig)), Some(user_path(&file)));
    // Truncated, keeping 5 tail characters.
    rig.frame(vec![]);
    let shown = texts(&rig);
    let label = shown
        .iter()
        .find(|t| t.ends_with(" name") && t.contains("..."));
    let (head, _) = label
        .unwrap_or_else(|| panic!("{shown:?}"))
        .split_once("...")
        .unwrap();
    assert!(name.starts_with(head) && head.len() > 5, "{head}");
}

#[test]
fn the_picker_closes_when_its_module_goes_or_becomes_another_type() {
    let mut rig = with_three_osc();
    pick_custom(&mut rig);
    rig.app.song.channels[CHANNEL].rack.modules.clear();
    rig.frame(vec![]);
    assert!(!rig.overlays().contains(&picker()));
    // Another type under the same instance id (an undo under the picker): its pick would be no 3x Osc's.
    let mut rig = with_three_osc();
    pick_custom(&mut rig);
    rig.frame(vec![]);
    let module = rig.app.song.channels[CHANNEL].rack.get_mut(OSC).unwrap();
    *module = crate::song::ModuleInstance::new(0, OSC);
    rig.frame(vec![]);
    assert!(!rig.overlays().contains(&picker()));
}

#[test]
fn the_picker_follows_its_channel_when_an_earlier_one_comes_back() {
    // As the SoundFont picker's: 3x Osc A (2) and B (3). A goes, the picker opens on B at 2, and Ctrl+Z brings A back
    // there: the picker moves with B to 3, and its pick is B's.
    let mut rig = with_three_osc();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::ThreeXOsc), 0);
    rig.app.song.delete_channel(CHANNEL);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.open_rack(CHANNEL);
    rig.app.navigate(Nav::Open(picker()));
    rig.frame(vec![]);
    // Ctrl+Z waits under the modal picker, so the undo comes from the app, as a MIDI controller's would.
    let ctx = rig.ctx.clone();
    rig.app.step(&ctx, crate::history::Step::Undo, 0);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [Overlay::Rack(3), picker_on(3)]);
    picker_row(&mut rig, 5);
    let vowel = app_path("Built-in/Shapes/Vowel A.wav");
    let path = |rig: &Rig, c| shape_path(rig.app.song.find_module(c, OSC).unwrap());
    assert_eq!((path(&rig, 2), path(&rig, 3)), (None, Some(vowel)));
}

#[test]
fn a_file_that_does_not_decode_shows_the_error_and_changes_nothing() {
    let mut rig = with_three_osc();
    rig.app
        .files
        .keep_user_file(Path::new("My Samples/Broken.wav"), b"RIFF".to_vec());
    pick_custom(&mut rig);
    rig.frame(vec![]);
    let song = rig.app.song.clone();
    rig.take();
    for row in [0, 1, 0] {
        picker_row(&mut rig, row);
    }
    let lines = message(&rig).expect("a message");
    assert_eq!(lines[0], crate::project_files::LOAD_FAILED);
    assert!(!rig.overlays().contains(&picker()));
    assert_eq!((rig.app.song == song, rig.take()), (true, vec![]));
}

#[test]
fn a_pick_tries_a_missing_file_again() {
    // A song's `PATH` names a file that is not there; once it is, picking it decodes it (a new pick forgets what
    // was missing).
    let mut rig = with_three_osc();
    let file = "My Samples/Wave.wav";
    let module = rig.app.song.channels[CHANNEL].rack.get_mut(OSC).unwrap();
    crate::project_files::load_pick(module, Path::new(file), &[], Some(user_path(file))).unwrap();
    rig.app.song_edited(SongEdit::Final, 0);
    let key = SampleKey::File(user_path(file));
    assert!(rig.app.audio.samples.missing().any(|k| *k == key));
    rig.app
        .files
        .keep_user_file(Path::new(file), wav(&[0, 9000, 0, -9000]));
    pick_custom(&mut rig);
    for row in [0, 1, 0] {
        picker_row(&mut rig, row);
    }
    assert_eq!(shape_path(osc(&rig)), Some(user_path(file)));
    assert!(rig.app.audio.samples.shape(&key).is_some());
}

/// Holds the pickers' worker until the guard drops: a pick stays loading.
fn hold() -> std::sync::MutexGuard<'static, ()> {
    crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[test]
fn a_custom_shape_builds_its_table_on_the_worker_while_its_picker_hides_and_takes_every_click() {
    // The picker closes on the pick but waits for it, drawing nothing and taking every click, then goes once it
    // lands: the table built on the worker, with no undo step.
    let mut rig = with_three_osc();
    let file = Path::new("My Samples/Wave.wav");
    (rig.app.files).keep_user_file(file, wav(&[0, 9000, 0, -9000]));
    pick_custom(&mut rig);
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    let (steps, shape) = (rig.app.history.steps(), osc(&rig).params[1]);
    let held = hold();
    super::rack_wiring::click_row(&mut rig, 0);
    rig.frame(vec![]);
    assert!(rig.app.slots.sample_picker.loading());
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), picker()]);
    assert!(!rig.painted("Wave") && !rig.painted(LOADING), "hidden");
    assert_eq!(shape_path(osc(&rig)), None);
    // A click on the card under it reaches nothing.
    next_shape(&mut rig);
    assert_eq!(osc(&rig).params[1], shape);
    // Only the worker's decode, from the bytes read at the pick, can build the table now.
    (rig.app.files).keep_user_file(file, b"RIFF".to_vec());
    drop(held);
    super::rack_wiring::worker_done(&mut rig);
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL)]);
    let wave = user_path("My Samples/Wave.wav");
    assert_eq!(shape_path(osc(&rig)), Some(wave.clone()));
    assert!(
        rig.app
            .audio
            .samples
            .shape(&SampleKey::File(wave))
            .is_some()
    );
    assert_eq!(rig.app.history.steps(), steps);
    assert_eq!(
        crate::ui::views::sample_picker::decodes_here(),
        0,
        "decoded on the worker"
    );
}

/// The amplitude of the component at `f` Hz in `x` (48 kHz, Hann-windowed DFT).
fn amp(x: &[f32], f: f64) -> f64 {
    let n = x.len();
    let (mut re, mut im, mut sum) = (0.0, 0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (TAU * i as f64 / n as f64).cos();
        let a = TAU * f * i as f64 / 48_000.0;
        (re, im, sum) = (
            re + w * v as f64 * a.cos(),
            im + w * v as f64 * a.sin(),
            sum + w,
        );
    }
    2.0 * re.hypot(im) / sum
}

/// Acceptance: a 3x Osc on `Vowel A` plays the vowel: of its first 16 harmonics at A4 the loudest is within one of
/// The first formant the display draws, where a Saw's is the fundamental.
#[test]
fn a_three_osc_on_vowel_a_renders_its_formant() {
    let mut rig = with_three_osc();
    pick_custom(&mut rig);
    picker_row(&mut rig, 5);
    let harmonics = |song: &Song| {
        let samples = &rig.app.audio.samples;
        let mut engine = SongAudioEngine::with_samples(song, 48_000.0, samples).unwrap();
        engine.note_on(CHANNEL, 69, 1.0);
        let mut out = vec![0.0; 2 * 48_000];
        engine.render(&mut out, 2);
        // The second half, left channel.
        let left: Vec<f32> = out.iter().step_by(2).skip(24_000).copied().collect();
        (1..=16)
            .map(|k| amp(&left, 440.0 * k as f64))
            .collect::<Vec<_>>()
    };
    let loudest = |h: &[f64]| {
        (1..=16)
            .max_by(|&a, &b| h[a - 1].total_cmp(&h[b - 1]))
            .unwrap()
    };
    let vowel = harmonics(&rig.app.song);
    assert!(vowel[loudest(&vowel) - 1] > 0.01, "{vowel:?}");
    let peak = loudest(&vowel) as f64;
    assert!(
        (peak - VOWEL_A_FORMANTS[0]).abs() <= 1.0,
        "{peak}: {vowel:?}"
    );
    let mut saw = rig.app.song.clone();
    saw.channels[CHANNEL].rack.get_mut(OSC).unwrap().params[1] = 0.5;
    assert_eq!(loudest(&harmonics(&saw)), 1);
}

// The Slicer's `LOAD SAMPLE`: it opens the picker, whose pick sets the module's `CSTM` `PATH` with no
// undo step; the card then shows the loop's waveform, a tap on which opens the picker again, and the rebuilt engine
// plays the loop's slices.

/// The Slicer's instance id on its channel ([`CHANNEL`], after the rig's MiniSynth).
const SLICER: i32 = 2;
/// `LOAD SAMPLE`.
const LOAD: u32 = 1000;

/// [`Rig::with_synth`] with our factory (the rig leaves it out), a Slicer channel (2), its rack open, with an empty log.
fn with_slicer() -> Rig {
    let mut rig = Rig::with_synth();
    rig.app.files.factory = crate::content::factory();
    rig.app
        .add_bare_channel(ChannelKind::Synth(SynthKind::Slicer), 0);
    rig.open_rack(CHANNEL);
    rig.slide_rack();
    rig.take();
    rig
}

fn slicer(rig: &Rig) -> &crate::song::ModuleInstance {
    rig.app.song.find_module(CHANNEL, SLICER).unwrap()
}

fn loop_picker() -> Overlay {
    Overlay::ModuleSamplePicker {
        channel: CHANNEL,
        module: SLICER,
    }
}

/// The Slicer's loop: its `CSTM` `PATH`.
fn loop_of(rig: &Rig) -> Option<ContentPath> {
    crate::audio_engine::slicer::loop_path(slicer(rig))
}

/// The Slicer's `SENS` chunk.
fn sens_of(rig: &Rig) -> Option<Vec<u8>> {
    let blob = slicer(rig).file.custom_data.as_deref()?;
    let mut chunks = crate::flm::chunk::ChunkIter::new(blob, "CSTM").map_while(Result::ok);
    Some(chunks.find(|c| c.id == *b"SENS")?.payload.to_vec())
}

/// Gives the Slicer a `CSTM` of `SENS` `sens` alone, as a final edit.
fn set_sens(rig: &mut Rig, sens: &[u8]) {
    let mut w = crate::flm::chunk::ChunkWriter::default();
    w.chunk(*b"SENS", |w| w.raw(sens)).unwrap();
    let module = rig.app.song.channels[CHANNEL].rack.get_mut(SLICER).unwrap();
    module.file.custom_data = Some(w.into_bytes());
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.take();
}

/// The waveform's tap: where it is drawn.
fn wave() -> Id {
    Id::new(("rack", CHANNEL))
        .with(("rack card", SLICER))
        .with("loop")
}

/// `LOAD SAMPLE`, then the picker's row `row` (1–6: our loops).
fn load_loop(rig: &mut Rig, row: usize) {
    rig.frame(vec![]);
    rig.click_id(
        Id::new(("rack", CHANNEL))
            .with(("module panel", SLICER))
            .with(LOAD),
    );
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), loop_picker()]);
    picker_row(rig, row);
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL)]);
}

#[test]
fn load_sample_opens_the_picker_whose_loop_pick_sets_the_path_with_no_undo_step() {
    let mut rig = with_slicer();
    let steps = rig.app.history.steps();
    // A Sensitivity of its own and AUTO off: the pick keeps them.
    let sens = [&0.6f32.to_le_bytes()[..], &[0]].concat();
    set_sens(&mut rig, &sens);
    assert!(texts(&rig).iter().any(|t| t == "LOAD SAMPLE"));
    assert!(rig.ctx.read_response(wave()).is_none());
    rig.click_id(
        Id::new(("rack", CHANNEL))
            .with(("module panel", SLICER))
            .with(LOAD),
    );
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), loop_picker()]);
    // My Files, then our six loops in index order.
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    let shown = texts(&rig);
    let rows = [
        "My Files",
        "Dust Break 84",
        "Foundry Drive 128",
        "Foundry Rush 172",
        "Pocket Groove 92",
        "Relay Bounce 100",
        "Relay Roll 140",
    ];
    let at = |name: &str| shown.iter().position(|t| t == name);
    let order: Vec<_> = (rows.iter())
        .map(|r| at(r).unwrap_or_else(|| panic!("{r}")))
        .collect();
    assert!(order.is_sorted(), "{shown:?}");
    picker_row(&mut rig, 1);
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL)]);
    let dust = app_path("Built-in/Loops/Dust Break 84.wav");
    assert_eq!(loop_of(&rig), Some(dust.clone()));
    assert_eq!(sens_of(&rig), Some(sens.clone()));
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // The waveform takes the button's place: one column a device pixel.
    rig.frame(vec![]);
    assert!(!texts(&rig).iter().any(|t| t == "LOAD SAMPLE"));
    let area = rig.ctx.read_response(wave()).expect("the waveform").rect;
    let quads = crate::ui::audio_editor::mesh_quads(rig.out.shapes.iter().map(|s| &s.shape));
    let columns = (quads.iter())
        .filter(|(r, _)| area.contains_rect(*r) && r.width() < 2.0)
        .count();
    assert!(
        columns as f32 >= area.width() - 2.0,
        "{columns} in {area:?}"
    );
    // The `PATH` saves and loads.
    let read = crate::flm::read(&crate::flm::write(&rig.app.song).unwrap()).unwrap();
    let module = read.find_module(CHANNEL, SLICER).unwrap();
    assert_eq!(crate::audio_engine::slicer::loop_path(module), Some(dust));
    // No undo step (AUTO is off: no clip either).
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn a_tap_on_the_waveform_opens_the_picker_again_and_escape_changes_nothing() {
    let mut rig = with_slicer();
    load_loop(&mut rig, 1);
    rig.frame(vec![]);
    let song = rig.app.song.clone();
    rig.take();
    rig.click_id(wave());
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), loop_picker()]);
    rig.frame(escape());
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL)]);
    assert_eq!((rig.app.song == song, rig.take()), (true, vec![]));
    // Another loop replaces it.
    rig.click_id(wave());
    picker_row(&mut rig, 4);
    assert_eq!(
        loop_of(&rig),
        Some(app_path("Built-in/Loops/Pocket Groove 92.wav"))
    );
}

#[test]
fn a_user_loop_takes_its_user_path_and_the_channel_going_closes_the_picker() {
    let mut rig = with_slicer();
    let file = "My Samples/Mine.wav";
    rig.app
        .files
        .keep_user_file(Path::new(file), wav(&[0, 9000, 0, -9000]));
    rig.frame(vec![]);
    rig.click_id(
        Id::new(("rack", CHANNEL))
            .with(("module panel", SLICER))
            .with(LOAD),
    );
    for row in [0, 1, 0] {
        picker_row(&mut rig, row);
    }
    assert_eq!(loop_of(&rig), Some(user_path(file)));
    // A new Slicer gets a new module's Sensitivity with its loop: 0.25, AUTO on.
    assert_eq!(
        sens_of(&rig),
        Some([&0.25f32.to_le_bytes()[..], &[1]].concat())
    );
    rig.frame(vec![]);
    rig.click_id(wave());
    assert!(rig.overlays().contains(&loop_picker()));
    rig.app.song.delete_channel(CHANNEL);
    rig.frame(vec![]);
    assert!(!rig.overlays().contains(&loop_picker()));
}

/// Whether key `key` on the Slicer's channel sounds in the engine `song` builds from the rig's samples.
fn sounds(rig: &Rig, song: &Song, key: u8) -> bool {
    let samples = &rig.app.audio.samples;
    let mut engine = SongAudioEngine::with_samples(song, 48_000.0, samples).unwrap();
    engine.note_on(CHANNEL, key, 1.0);
    let mut out = vec![0.0; 2 * 12_000];
    engine.render(&mut out, 2);
    out.iter().any(|s| s.abs() > 1e-3)
}

/// The Slicer's slices as the engine sees them: one note each in AUTO's clip.
fn slices(rig: &Rig) -> usize {
    let samples = &rig.app.audio.samples;
    let clip = crate::audio_engine::slicer_auto_clip(&rig.app.song, CHANNEL, SLICER, samples);
    clip.unwrap().1.len()
}

#[test]
fn the_rebuilt_slicer_slices_the_loop_at_its_cues_at_sensitivity_0() {
    let mut rig = with_slicer();
    load_loop(&mut rig, 1);
    let key = SampleKey::of_path(&loop_of(&rig).unwrap());
    let sample = rig.app.audio.samples.get(&key).expect("rendered");
    // A new module's 0.25 (`SENS` from the pick): the detector's slices (the cues count only at 0).
    let detected = honeydew_audio::slicer::detect(&sample, 0.25).len();
    assert!(detected > 1);
    assert_eq!(slices(&rig), detected);
    // Sensitivity 0: one slice per cue, each a key from 48.
    let module = rig.app.song.channels[CHANNEL].rack.get_mut(SLICER).unwrap();
    let previous = module.file.custom_data.as_deref();
    module.file.custom_data =
        crate::panels::cstm::cstm_from_settings(33, &[(3100, 0.0)].into(), previous);
    rig.app.song_edited(SongEdit::Final, 0);
    assert_eq!(
        sens_of(&rig),
        Some([&0.0f32.to_le_bytes()[..], &[1]].concat())
    );
    let cues = sample.meta.cues.len();
    assert!(cues > 1);
    assert_eq!(slices(&rig), cues);
    let song = rig.app.song.clone();
    assert!(sounds(&rig, &song, 48 + cues as u8 - 1));
    assert!(!sounds(&rig, &song, 48 + cues as u8));
}

/// Acceptance: a new Slicer channel, `LOAD SAMPLE`, a factory loop: its first slice sounds on key 48 (the default
/// first note), and key 60 sounds slice 13.
#[test]
fn a_new_slicer_on_a_factory_loop_plays_its_slices() {
    let mut rig = with_slicer();
    load_loop(&mut rig, 4);
    assert!(slices(&rig) > 13, "{}", slices(&rig));
    let song = rig.app.song.clone();
    assert!(sounds(&rig, &song, 48) && sounds(&rig, &song, 60));
    assert!(!sounds(&rig, &song, 47));
}

#[test]
fn a_loop_decodes_on_the_worker_and_lands_with_autos_clip_in_one_undo_step() {
    // `LOAD SAMPLE`'s pick decodes on the worker; its picker closes on the pick, hidden and taking every click until
    // it lands: the loop, and AUTO's clip in one undo step.
    let mut rig = with_slicer();
    let file = Path::new("My Samples/Beat.wav");
    let beat = wav(&[0, 9000, 0, -9000, 0, 6000, 0, -6000]);
    (rig.app.files).keep_user_file(file, beat);
    let id = add_clip(&mut rig, 0.0);
    let before = clip(&rig, id).clone();
    rig.click_id(
        Id::new(("rack", CHANNEL))
            .with(("module panel", SLICER))
            .with(LOAD),
    );
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    let held = hold();
    super::rack_wiring::click_row(&mut rig, 0);
    rig.frame(vec![]);
    assert!(rig.app.slots.sample_picker.loading());
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), loop_picker()]);
    assert!(!rig.painted("Beat") && !rig.painted(LOADING), "hidden");
    assert_eq!((loop_of(&rig), clip(&rig, id)), (None, &before));
    // Only the worker's decode, from the bytes read at the pick, can slice the loop now.
    (rig.app.files).keep_user_file(file, b"RIFF".to_vec());
    drop(held);
    super::rack_wiring::worker_done(&mut rig);
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL)]);
    assert_eq!(loop_of(&rig), Some(user_path("My Samples/Beat.wav")));
    let (beats, notes) = auto_clip(&rig);
    assert!(!notes.is_empty());
    assert_eq!(
        (clip(&rig, id).events.clone(), clip(&rig, id).loop_len_beats),
        (notes, beats)
    );
    assert_eq!(
        crate::ui::views::sample_picker::decodes_here(),
        0,
        "decoded on the worker"
    );
    // One undo step takes the clip back; the loop stays (the rack is not covered by undo).
    let beat = loop_of(&rig);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!((loop_of(&rig), clip(&rig, id)), (beat, &before));
}

#[test]
fn a_loop_picks_slices_are_detected_on_the_worker() {
    // `LOAD SAMPLE`'s decode slices the loop at the Sensitivity the pick leaves (a new module's 0.25),
    // so AUTO's clip and the build that follow detect nothing on the UI thread.
    let mut rig = with_slicer();
    let file = Path::new("My Samples/Beat.wav");
    let beat = wav(&[0, 9000, 0, -9000, 0, 6000, 0, -6000]);
    (rig.app.files).keep_user_file(file, beat);
    let id = add_clip(&mut rig, 0.0);
    rig.click_id(
        Id::new(("rack", CHANNEL))
            .with(("module panel", SLICER))
            .with(LOAD),
    );
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    let detects = crate::audio_engine::slicer::detects_here();
    super::rack_wiring::click_row(&mut rig, 0);
    super::rack_wiring::worker_done(&mut rig);
    assert_eq!(loop_of(&rig), Some(user_path("My Samples/Beat.wav")));
    let (_, notes) = auto_clip(&rig);
    assert!(!notes.is_empty());
    assert_eq!(clip(&rig, id).events, notes);
    assert_eq!(
        crate::audio_engine::slicer::detects_here(),
        detects,
        "sliced on the worker"
    );
}

// ---- `Import` in the module sample picker (web), AUTO's clip ----

/// The web's module sample picker (the Slicer's `LOAD SAMPLE`, 3x Osc's `Custom`) offers `Import` as the drum sample
/// picker does: the file goes into `My Samples`, the open picker lists it at once, and a pick loads it. The desktop's
/// has none.
#[test]
fn import_in_the_module_sample_picker_keeps_an_audio_file_in_my_samples() {
    use crate::project_files::ProjectFiles;
    use crate::project_files::mem::MemFiles;
    for kind in [SynthKind::Slicer, SynthKind::ThreeXOsc] {
        let mut rig = Rig::with_synth();
        let bytes = wav(&[0, 9000, 0, -9000]);
        rig.app.files =
            ProjectFiles::with_provider(Box::new(MemFiles::with_file("Mine.wav", &bytes)));
        rig.app.add_channel(ChannelKind::Synth(kind), 0);
        let picker = Overlay::ModuleSamplePicker {
            channel: CHANNEL,
            module: SLICER,
        };
        rig.app.navigate(Nav::Open(picker.clone()));
        picker_row(&mut rig, 0);
        picker_row(&mut rig, 1);
        rig.frame(vec![]);
        assert!(rig.color_of("Mine").is_none(), "{kind:?}");
        rig.click_id(Id::new("preset_picker").with("import"));
        rig.frame(vec![]);
        let kept = rig
            .app
            .files
            .read_user_file(Path::new("My Samples/Mine.wav"));
        assert_eq!(kept.unwrap(), bytes, "{kind:?}");
        assert!(rig.overlays().contains(&picker), "{kind:?} stays open");
        rig.frame(vec![]);
        assert!(rig.color_of("Mine").is_some(), "{kind:?} lists it");
        picker_row(&mut rig, 0);
        assert!(!rig.overlays().contains(&picker), "{kind:?}");
        let module = slicer(&rig);
        let path = crate::audio_engine::slicer::loop_path(module).or_else(|| shape_path(module));
        assert_eq!(path, Some(user_path("My Samples/Mine.wav")), "{kind:?}");
    }
    // The desktop's has none.
    let mut rig = Rig::with_synth();
    rig.app.files = crate::project_files::ProjectFiles::on_disk(
        &std::env::temp_dir().join("honeydew-q6-no-such-folder"),
    );
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::Slicer), 0);
    rig.app.navigate(Nav::Open(loop_picker()));
    picker_row(&mut rig, 0);
    rig.frame(vec![]);
    assert!(rig.color_of("Search").is_some() && rig.color_of("Import").is_none());
}

/// Adds a clip of one note of its own to the Slicer's channel as the playlist's `+Clip` does (`clips::add_clip`, no
/// `DESc`@24), as a final edit; its id.
fn add_clip(rig: &mut Rig, tick: f64) -> u32 {
    let lane = &mut rig.app.song.channels[CHANNEL].lanes[0];
    let id = crate::ui::clips::add_clip(lane, tick, 128.0, 4);
    let clip = lane.clips.iter_mut().find(|c| c.id == id).unwrap();
    clip.events.push(crate::song::Event::Note {
        pos: 0,
        len_beats: 1.0,
        key: 30,
        velocity: 100,
        pan: crate::song::Event::DEFAULT_PAN,
        slide: false,
    });
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.take();
    id
}

/// The Slicer channel's clip `id`.
fn clip(rig: &Rig, id: u32) -> &crate::song::Clip {
    let lane = &rig.app.song.channels[CHANNEL].lanes[0];
    lane.clips.iter().find(|c| c.id == id).unwrap()
}

/// AUTO's clip for the Slicer as the engine sees the song now: its length in beats and its notes.
fn auto_clip(rig: &Rig) -> (f64, Vec<crate::song::Event>) {
    let samples = &rig.app.audio.samples;
    crate::audio_engine::slicer_auto_clip(&rig.app.song, CHANNEL, SLICER, samples).unwrap()
}

/// Drags the Sensitivity knob up by `px` and lets go.
fn turn_sensitivity(rig: &mut Rig, px: f32) {
    let knob = Id::new(("rack", CHANNEL))
        .with(("module panel", SLICER))
        .with(3100);
    let at = rig.ctx.read_response(knob).unwrap().rect.center();
    rig.press(at);
    rig.drag_to(at - egui::vec2(0.0, px / 2.0));
    rig.drag_to(at - egui::vec2(0.0, px));
    rig.release(at - egui::vec2(0.0, px));
    rig.frame(vec![]);
}

/// The Slicer card's momentary `AUTO`.
fn press_auto(rig: &mut Rig) {
    rig.click_id(
        Id::new(("rack", CHANNEL))
            .with(("module panel", SLICER))
            .with(3700),
    );
}

#[test]
fn auto_on_a_loop_pick_and_a_sensitivity_change_rewrite_the_current_clip_in_their_undo_step() {
    // AUTO = 1: a slice re-detect rewrites the current clip, one note per slice, velocity 0.7. An app-made
    // song: no piano roll open, no `DESc`@24, so the first notes lane's first clip.
    let mut rig = with_slicer();
    let id = add_clip(&mut rig, 0.0);
    let before = clip(&rig, id).clone();
    load_loop(&mut rig, 1);
    let (beats, notes) = auto_clip(&rig);
    assert_eq!(clip(&rig, id).events, notes);
    assert_eq!(
        (
            clip(&rig, id).loop_len_beats,
            clip(&rig, id).visible_len_beats
        ),
        (beats, beats)
    );
    let velocities = notes
        .iter()
        .all(|n| matches!(n, crate::song::Event::Note { velocity: 179, .. }));
    assert!(notes.len() > 1 && velocities);
    // The pick's own undo step: its clip; the loop stays (the rack is not covered by undo).
    let picked = loop_of(&rig);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!((clip(&rig, id), loop_of(&rig)), (&before, picked));
    rig.frame(z(egui::Modifiers::SHIFT));
    assert_eq!(clip(&rig, id).events, notes);
    // Another Sensitivity re-detects: the clip follows, once the knob is let go.
    let edited: Vec<_> = notes[..1].to_vec();
    let lane = &mut rig.app.song.channels[CHANNEL].lanes[0];
    lane.clips[0].events.clone_from(&edited);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    turn_sensitivity(&mut rig, 60.0);
    let sens = sens_of(&rig).unwrap();
    assert!(
        f32::from_le_bytes(sens[..4].try_into().unwrap()) > 0.25,
        "{sens:?}"
    );
    let (_, redetected) = auto_clip(&rig);
    assert_eq!(clip(&rig, id).events, redetected);
    // One undo takes the clip back; the Sensitivity stays.
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(clip(&rig, id).events, edited);
    assert_eq!(sens_of(&rig), Some(sens));
}

#[test]
fn auto_writes_the_clip_the_piano_roll_edits() {
    // Two clips; the piano roll opened on the second (a tap on it in the playlist): AUTO writes that one.
    let mut rig = with_slicer();
    let (first, second) = (add_clip(&mut rig, 0.0), add_clip(&mut rig, 1024.0));
    let untouched = clip(&rig, first).clone();
    rig.app.navigate(Nav::EditClip(crate::ui::preview::ClipKey {
        channel: CHANNEL,
        lane: 0,
        id: second,
    }));
    rig.frame(vec![]);
    rig.app.navigate(Nav::Open(loop_picker()));
    picker_row(&mut rig, 1);
    assert_eq!(
        loop_of(&rig),
        Some(app_path("Built-in/Loops/Dust Break 84.wav"))
    );
    assert_eq!(clip(&rig, second).events, auto_clip(&rig).1);
    assert_eq!(clip(&rig, first), &untouched);
}

#[test]
fn auto_off_leaves_the_clip_alone_and_the_auto_button_rewrites_it_once() {
    let mut rig = with_slicer();
    set_sens(&mut rig, &[&0.25f32.to_le_bytes()[..], &[0]].concat());
    load_loop(&mut rig, 1);
    // No clip: the button makes no edit.
    rig.frame(vec![]);
    let song = rig.app.song.clone();
    rig.take();
    press_auto(&mut rig);
    assert_eq!((rig.app.song == song, rig.take()), (true, vec![]));
    let id = add_clip(&mut rig, 0.0);
    let before = clip(&rig, id).clone();
    turn_sensitivity(&mut rig, 60.0);
    assert_eq!(clip(&rig, id), &before, "a Sensitivity change");
    // The AUTO button: a one-shot rewrite whatever the flag says, which it leaves off.
    rig.take();
    press_auto(&mut rig);
    let (beats, notes) = auto_clip(&rig);
    assert_eq!(clip(&rig, id).events, notes);
    assert_eq!(clip(&rig, id).loop_len_beats, beats);
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    assert_eq!(sens_of(&rig).unwrap()[4], 0);
    // Once: an edit after it stays through more frames.
    rig.app.song.channels[CHANNEL].lanes[0].clips[0]
        .events
        .truncate(1);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(clip(&rig, id).events.len(), 1);
}

#[test]
fn a_loop_from_the_slicers_preset_picker_writes_autos_clip_too() {
    // The card's preset picker lists the loops in the module's user folder (`My Presets/Slicer`): a loop and a module
    // preset both load on the worker (`finish`).
    use super::rack_wiring::worker_done;
    use crate::project_files::ProjectFiles;
    use crate::project_files::mem::MemFiles;
    use crate::ui::rack::PresetTarget;
    let mut rig = with_slicer();
    rig.app.files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
    let dir = rig.app.files.presets_dir(33).unwrap();
    rig.app
        .files
        .keep_user_file(&dir.join("Mine.wav"), wav(&[0, 9000, 0, -9000]));
    let id = add_clip(&mut rig, 0.0);
    let picker = Overlay::PresetPicker {
        channel: CHANNEL,
        target: PresetTarget::Module(SLICER),
    };
    rig.app.navigate(Nav::Open(picker.clone()));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    worker_done(&mut rig);
    let file = dir.join("Mine.wav");
    let shorthand = rig.app.files.shorthand(&file);
    assert_eq!(loop_of(&rig), shorthand);
    let (beats, notes) = auto_clip(&rig);
    assert!(!notes.is_empty());
    assert_eq!(
        (clip(&rig, id).events.clone(), clip(&rig, id).loop_len_beats),
        (notes.clone(), beats)
    );
    // A Slicer preset that names the loop: a re-detect too.
    let preset = crate::flm::write_preset(slicer(&rig)).unwrap();
    let preset_path = dir.join("Mine Preset.flmpst");
    rig.app.files.keep_user_file(&preset_path, preset);
    // The picker stays open after a pick: closed, it opens again at its root.
    super::rack_wiring::close_picker(&mut rig);
    rig.app.song.channels[CHANNEL].lanes[0].clips[0]
        .events
        .clear();
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.app.navigate(Nav::Open(picker));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    worker_done(&mut rig);
    assert_eq!(slicer(&rig).file.preset_name, "Mine Preset");
    assert_eq!(clip(&rig, id).events, notes);
}

/// With the piano roll open on the second of two clips, every AUTO write goes there and leaves the first alone: a
/// loop from the card's preset picker (`finish`, on the worker), a Sensitivity release (the rack's re-detect) and the
/// `AUTO` button. Each path hands the roll's clip down from the overlay dispatch.
#[test]
fn every_auto_write_takes_the_clip_the_piano_roll_edits() {
    use super::rack_wiring::worker_done;
    use crate::project_files::ProjectFiles;
    use crate::project_files::mem::MemFiles;
    use crate::ui::rack::PresetTarget;
    let mut rig = with_slicer();
    rig.app.files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
    let dir = rig.app.files.presets_dir(33).unwrap();
    let bytes = wav(&[0, 9000, 0, -9000, 0, 6000, 0, -6000]);
    rig.app.files.keep_user_file(&dir.join("Mine.wav"), bytes);
    let (first, second) = (add_clip(&mut rig, 0.0), add_clip(&mut rig, 1024.0));
    let untouched = clip(&rig, first).clone();
    rig.app.navigate(Nav::EditClip(crate::ui::preview::ClipKey {
        channel: CHANNEL,
        lane: 0,
        id: second,
    }));
    rig.frame(vec![]);
    rig.open_rack(CHANNEL);
    rig.slide_rack();
    // The second clip's notes back to one of its own, as a final edit.
    let reset = |rig: &mut Rig| {
        let lane = &mut rig.app.song.channels[CHANNEL].lanes[0];
        let clip = lane.clips.iter_mut().find(|c| c.id == second).unwrap();
        clip.events.truncate(1);
        clip.events[0] = untouched.events[0];
        rig.app.song_edited(SongEdit::Final, 0);
        rig.frame(vec![]);
    };
    let written = |rig: &Rig, what: &str| {
        assert_eq!(clip(rig, second).events, auto_clip(rig).1, "{what}");
        assert_eq!(clip(rig, first), &untouched, "{what}");
    };
    // (c) A loop from the card's preset picker.
    let picker = Overlay::PresetPicker {
        channel: CHANNEL,
        target: PresetTarget::Module(SLICER),
    };
    rig.app.navigate(Nav::Open(picker));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    worker_done(&mut rig);
    assert!(loop_of(&rig).is_some());
    written(&rig, "a preset picker loop");
    // The picker stays open after a pick; closed, the card under it takes the knob.
    super::rack_wiring::close_picker(&mut rig);
    // (a) A Sensitivity release.
    reset(&mut rig);
    assert_ne!(clip(&rig, second).events, auto_clip(&rig).1);
    turn_sensitivity(&mut rig, 60.0);
    written(&rig, "a Sensitivity release");
    // (b) The AUTO button.
    reset(&mut rig);
    press_auto(&mut rig);
    written(&rig, "the AUTO button");
}

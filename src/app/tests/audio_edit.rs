//! The audio clip editor's region ops on the sample and its saving, in the app (`app/audio_edit.rs`).

use std::path::Path;

use honeydew_audio::sample::decode;
use honeydew_audio::wav_writer::float32_wav;

use super::*;
use crate::app::audio_edit::{PROCESS_FAILED, PROCESSING, SAVE_QUESTION};
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::history::Step;
use crate::song::points::{self, Point, PointKey};
use crate::song::{SamplerSlot, SlotFileState};
use crate::ui::audio_editor::Action;
use crate::ui::automation::geom::Frame;
use crate::ui::main_screen::Regions;
use crate::ui::region::{RegionOp, TimeRange};
use crate::ui::save_dialog;

const VOL: PointKey = PointKey::Envelope(0);
/// 8000 frames a second at 120 BPM: 4000 a beat.
const RATE: u32 = 8000;
const SAMPLE: &str = "My Samples/quiet.wav";

fn user(relative: &str) -> ContentPath {
    ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: relative.into(),
    }
}

/// Four beats at 0.1.
fn quiet() -> Vec<u8> {
    float32_wav(RATE, &[vec![0.1_f32; 16_000]])
}

/// In-memory files holding `files`.
fn mem(files: &[(&str, Vec<u8>)]) -> ProjectFiles {
    let mut mem = ProjectFiles::with_provider(Box::new(MemFiles::default()));
    for (path, bytes) in files {
        mem.keep_user_file(Path::new(path), bytes.clone());
    }
    mem
}

/// With `files`, the Audio Clip channel (1) at 120 BPM, its clip 1 on `slot` with VOL points at beats 1.25 and 3,
/// shown in the audio editor (its sample loaded) with the audio running; nothing to undo and an empty log.
fn rig_on(files: ProjectFiles, slot: SamplerSlot) -> Rig {
    rig_on_audio(files, slot, true)
}

/// [`rig_on`], with an audio output or with none.
fn rig_on_audio(files: ProjectFiles, slot: SamplerSlot, output: bool) -> Rig {
    let mut rig = Rig::new();
    rig.no_device = !output;
    rig.app.files = files;
    rig.app.add_channel(ChannelKind::AudioClip, 0);
    rig.app.song.tempo_bpm = 120.0;
    let lane = &mut rig.app.song.channels[1].lanes[0];
    let clip = lane.add_clip(0, 4.0);
    clip.file.sample = Some(Box::new(slot));
    points::set_points(clip, VOL, vol_points());
    lane.file.current_clip_id = 1;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.edit(1);
    rig.click(OUTSIDE);
    rig.frame(vec![]);
    rig.frame(vec![]);
    // Fitted to its sample, as a loaded clip is, and played so: the next edit's engine takes over this song's.
    let clip = &mut rig.app.song.channels[1].lanes[0].clips[0];
    crate::audio_engine::fit_audio_clip(clip, &rig.app.audio.samples, 120.0, 4);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.history = History::new(&rig.app.song);
    rig.take();
    rig
}

/// The fixture's VOL points, at beats 1.25 and 3.
fn vol_points() -> Vec<Point> {
    vec![
        Point {
            tick: 160,
            value: 1000,
            ..Default::default()
        },
        Point {
            tick: 384,
            value: 2000,
            ..Default::default()
        },
    ]
}

/// Their ticks after `Clear` over beats 1 … 2 (the envelope's `Delete`).
fn vol_cleared() -> Vec<u32> {
    let mut pts = vol_points();
    points::delete(&mut pts, 128, 256);
    pts.iter().map(|p| p.tick).collect()
}

/// A user sample's slot.
fn user_slot(path: &str) -> SamplerSlot {
    SamplerSlot {
        name: "quiet".into(),
        folder: "My Samples".into(),
        file: SlotFileState {
            path: Some(user(path)),
            ..SlotFileState::default()
        },
        ..SamplerSlot::default()
    }
}

fn builtin_slot() -> SamplerSlot {
    SamplerSlot {
        name: "Synth Kick".into(),
        folder: "Built-in/Kick".into(),
        ..SamplerSlot::default()
    }
}

/// [`rig_on`] [`quiet`] at `My Samples/quiet.wav`.
fn quiet_rig() -> Rig {
    rig_on(mem(&[(SAMPLE, quiet())]), user_slot(SAMPLE))
}

fn slot(rig: &Rig) -> &SamplerSlot {
    let clip = &rig.app.song.channels[1].lanes[0].clips[0];
    clip.file.sample.as_deref().expect("a slot")
}

fn vol(rig: &Rig) -> Vec<u32> {
    let clip = &rig.app.song.channels[1].lanes[0].clips[0];
    points::points(clip, VOL).iter().map(|p| p.tick).collect()
}

/// The editor asks for `op` over beats 1 … 2 of the VOL lane, with the playhead at 0.
fn request(rig: &mut Rig, op: RegionOp) {
    request_over(rig, op, 1, 2);
}

/// [`request`] over beats `s` … `e`.
fn request_over(rig: &mut Rig, op: RegionOp, s: u32, e: u32) {
    let range = TimeRange {
        start: s * 128,
        end: e * 128,
    };
    let action = Action::Region {
        key: VOL,
        op,
        range,
        playhead: 0,
    };
    rig.app.slots.audio_editor.request = Some(((1, 0, 1), action));
    rig.frame(vec![]);
}

fn painted(rig: &Rig, text: &str) -> Option<egui::Rect> {
    rig.out.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()),
        _ => None,
    })
}

fn message(rig: &Rig) -> Option<Vec<String>> {
    rig.app.nav.overlays.iter().find_map(|o| match o {
        Overlay::Message { lines, .. } => Some(lines.clone()),
        _ => None,
    })
}

/// Picks row `row` of the open message (its lines come first).
fn pick(rig: &mut Rig, row: usize) {
    rig.frame(vec![]);
    rig.click_id(egui::Id::new("message").with((0_usize, row)));
}

/// The question's rows: `Save new`, `Replace`, `Cancel`.
const SAVE_NEW: usize = 1;
const REPLACE: usize = 2;
const CANCEL: usize = 3;

fn file(rig: &Rig, path: &str) -> Option<Vec<u8>> {
    rig.app.files.read_user_file(Path::new(path)).ok()
}

fn undo(rig: &mut Rig) {
    let ctx = rig.ctx.clone();
    rig.app.step(&ctx, Step::Undo, 0);
}

fn screen_metrics() -> (egui::Rect, Metrics) {
    let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
    (
        screen,
        Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE),
    )
}

#[test]
fn normalize_over_a_quiet_range_writes_a_float_wav_whose_peak_is_1() {
    let mut rig = quiet_rig();
    // Through the editor: a ruler range over beats 1 … 2, its `+`, then `Normalize`.
    let (screen, m) = screen_metrics();
    // Under the playlist (`ui::split`, settled at f = 0).
    let view = Regions::new(screen, &m).editor;
    let f = Frame::new(crate::ui::split::split_rects(view, 0.0, &m).1, &m);
    let y = f.time.center().y;
    let (a, b) = (
        pos2(f.grid.left() + 128.0, y),
        pos2(f.grid.left() + 256.0, y),
    );
    rig.press(a);
    for _ in 0..5 {
        rig.frame(vec![]);
    }
    rig.drag_to(b);
    rig.release(b);
    rig.click(pos2(b.x, f.time.bottom() + 20.0));
    rig.frame(vec![]);
    let at = painted(&rig, "Normalize")
        .expect("the region menu")
        .center();
    // The range is the play loop.
    assert_eq!(rig.take(), ["SetLoop 128 256"]);
    rig.click(at);
    // The sample's own file is the target: the question.
    assert_eq!(message(&rig), Some(vec![SAVE_QUESTION.to_owned()]));
    // A message's first frame only sizes it.
    rig.frame(vec![]);
    rig.frame(vec![]);
    for label in ["Save new", "Replace", "Cancel"] {
        assert!(painted(&rig, label).is_some(), "{label}");
    }
    pick(&mut rig, REPLACE);
    // `Processing...` shows, and the next frame writes.
    assert!(painted(&rig, PROCESSING).is_some());
    assert_eq!(file(&rig, SAMPLE), Some(quiet()));
    rig.frame(vec![]);
    let bytes = file(&rig, SAMPLE).unwrap();
    assert_eq!(&bytes[20..22], &3_u16.to_le_bytes(), "32-bit float");
    let written = decode(&bytes).unwrap();
    assert_eq!((written.sample_rate, written.right), (RATE, None));
    let peak = |r: std::ops::Range<usize>| written.left[r].iter().fold(0f32, |m, x| m.max(x.abs()));
    assert_eq!(peak(4000..8000), 1.0);
    assert_eq!(peak(0..4000), 0.1);
    assert_eq!(peak(8000..16_000), 0.1);
    assert_eq!(painted(&rig, PROCESSING), None);
    // The same file: the song is as it was (Normalize has no envelope part), the engine has the new sample and
    // re-syncs to play it.
    assert_eq!(slot(&rig).file.path, Some(user(SAMPLE)));
    assert!(!rig.app.history.can_undo());
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    assert_eq!(rig.app.song.play_loop, None, "Normalize clears the range");
    // The stale cache entry went: the rebuild decoded the new file.
    let cached = rig.app.audio.samples.slot(slot(&rig)).unwrap();
    assert_eq!(cached.left[5000], 1.0);
}

/// `Replace` that leaves the song as it was, while playing, is heard: its re-sync relaunches the playing clip with
/// The new sample data.
#[test]
fn a_replace_while_playing_relaunches_the_clip_with_the_new_sample() {
    use honeydew_audio::device::AudioRenderer;
    let mut rig = quiet_rig();
    rig.app.main_screen.playing = true;
    rig.app.audio.send(AudioCommand::Play);
    // The next `frames` frames' peak, at 48 kHz: 24 000 a beat.
    let peak = |rig: &Rig, frames: usize| {
        let mut out = vec![0.0; frames * 2];
        let mut rec = rig.rec.0.borrow_mut();
        rec.engine.as_mut().unwrap().render(&mut out, 2);
        out.iter().fold(0f32, |m, x| m.max(x.abs()))
    };
    // To beat 1.25, inside the range Normalize raises from 0.1 to 1.
    peak(&rig, 30_000);
    let before = peak(&rig, 2400);
    // Quiet: the VOL envelope is low there.
    assert!(before > 1e-5, "a silent clip");
    request(&mut rig, RegionOp::Normalize);
    rig.take();
    pick(&mut rig, REPLACE);
    rig.frame(vec![]);
    assert!(rig.take().iter().any(|c| c == "ReplaceSong adopting"));
    let ratio = peak(&rig, 2400) / before;
    assert!((ratio - 10.0).abs() < 0.1, "{ratio}");
}

#[test]
fn save_new_writes_the_named_file_repoints_the_slot_and_is_one_undo_step() {
    let mut rig = quiet_rig();
    request(&mut rig, RegionOp::Clear);
    // Nothing yet: the envelope part waits with the audio part.
    assert_eq!(vol(&rig), [160, 384]);
    pick(&mut rig, SAVE_NEW);
    // The name dialog, with the current name.
    assert_eq!(
        rig.app.nav.overlays.last(),
        Some(&Overlay::SampleName {
            name: "quiet".into()
        })
    );
    // The dialog's first frame only sizes it.
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(painted(&rig, "New sample name").is_some());
    assert!(painted(&rig, "Audio Wave (.wav)").is_some());
    // The current name is taken: asked again.
    let (screen, m) = screen_metrics();
    let save = save_dialog::layout(screen, &m).save.center();
    rig.click(save);
    assert_eq!(message(&rig), Some(vec![SAVE_QUESTION.to_owned()]));
    pick(&mut rig, SAVE_NEW);
    let Some(Overlay::SampleName { name }) = rig.app.nav.overlays.last_mut() else {
        panic!("no dialog");
    };
    *name = "quiet 2".into();
    rig.frame(vec![]);
    rig.click(save);
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.overlays, []);
    // The edited copy is written; the source file stays.
    assert_eq!(file(&rig, SAMPLE), Some(quiet()));
    let written = decode(&file(&rig, "My Samples/quiet 2.wav").unwrap()).unwrap();
    assert!(written.left[4000..8000].iter().all(|&x| x == 0.0));
    assert_eq!(written.left[3999], 0.1);
    // The slot names the new file; the envelope part is in.
    let s = slot(&rig);
    assert_eq!(s.file.path, Some(user("My Samples/quiet 2.wav")));
    assert_eq!(
        (s.name.as_str(), s.folder.as_str()),
        ("quiet 2", "My Samples")
    );
    assert_eq!(vol(&rig), vol_cleared());
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    // One undo step takes back both.
    undo(&mut rig);
    assert_eq!(slot(&rig).file.path, Some(user(SAMPLE)));
    assert_eq!(vol(&rig), [160, 384]);
    assert!(!rig.app.history.can_undo());
}

#[test]
fn cancel_or_closing_the_name_dialog_drops_the_whole_op() {
    let mut rig = quiet_rig();
    request(&mut rig, RegionOp::Clear);
    pick(&mut rig, CANCEL);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.app.nav.overlays, []);
    assert_eq!(vol(&rig), [160, 384]);
    assert_eq!(file(&rig, SAMPLE), Some(quiet()));
    assert!(!rig.app.history.can_undo());
    assert_eq!(rig.take(), Vec::<String>::new());
    // `Save new`, then the dialog's Escape.
    request(&mut rig, RegionOp::Clear);
    pick(&mut rig, SAVE_NEW);
    rig.frame(vec![]);
    let escape = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    rig.frame(vec![escape]);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.app.nav.overlays, []);
    assert_eq!(vol(&rig), [160, 384]);
    assert!(!rig.app.history.can_undo());
    // A new op asks again: nothing is left waiting.
    request(&mut rig, RegionOp::Clear);
    assert_eq!(message(&rig), Some(vec![SAVE_QUESTION.to_owned()]));
}

#[test]
fn a_built_in_sample_is_saved_to_my_samples_without_asking() {
    let mut rig = rig_on(mem(&[]), builtin_slot());
    request(&mut rig, RegionOp::InsertSilence);
    assert_eq!(message(&rig), None);
    assert!(painted(&rig, PROCESSING).is_some());
    rig.frame(vec![]);
    let path = "My Samples/Synth Kick.wav";
    assert!(file(&rig, path).is_some_and(|b| decode(&b).is_ok()));
    let s = slot(&rig);
    assert_eq!(s.file.path, Some(user(path)));
    assert_eq!(
        (s.name.as_str(), s.folder.as_str()),
        ("Synth Kick", "My Samples")
    );
}

#[test]
fn an_op_that_leaves_the_audio_as_it_was_is_its_envelope_part_at_once() {
    let silence = float32_wav(RATE, &[vec![0.0_f32; 16_000]]);
    let mut rig = rig_on(mem(&[(SAMPLE, silence)]), user_slot(SAMPLE));
    // Clear over silence: no question, the VOL points go as the envelope's `Delete` has them, one undo step.
    request(&mut rig, RegionOp::Clear);
    assert_eq!(message(&rig), None);
    assert_eq!(vol(&rig), vol_cleared());
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    undo(&mut rig);
    assert_eq!(vol(&rig), [160, 384]);
    // Normalize over silence: nothing at all.
    request(&mut rig, RegionOp::Normalize);
    assert_eq!(message(&rig), None);
    assert_eq!(rig.take(), ["ReplaceSong adopting"], "the undo's");
}

#[test]
fn an_op_that_leaves_no_frames_shows_the_error_and_changes_nothing() {
    let mut rig = quiet_rig();
    // Trim selection past the end of a 4-beat sample.
    request_over(&mut rig, RegionOp::Trim, 8, 9);
    assert_eq!(message(&rig), Some(vec![PROCESS_FAILED.to_owned()]));
    assert_eq!(vol(&rig), [160, 384]);
    assert!(!rig.app.history.can_undo());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_failed_write_shows_the_error_and_changes_nothing() {
    let root = std::env::temp_dir().join(format!("honeydew-h8-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    // A folder where the target file would go.
    std::fs::create_dir_all(root.join("My Samples/Synth Kick.wav")).unwrap();
    let mut rig = rig_on(ProjectFiles::on_disk(&root), builtin_slot());
    request(&mut rig, RegionOp::InsertSilence);
    pick(&mut rig, REPLACE);
    rig.frame(vec![]);
    assert_eq!(message(&rig), Some(vec![PROCESS_FAILED.to_owned()]));
    assert_eq!(slot(&rig).file.path, None);
    assert_eq!(vol(&rig), [160, 384]);
    assert!(!rig.app.history.can_undo());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_op_whose_clip_changed_sample_under_the_question_is_dropped() {
    let mut rig = quiet_rig();
    request(&mut rig, RegionOp::Clear);
    // As an undo under the question would: the clip plays another sample now.
    let clip = &mut rig.app.song.channels[1].lanes[0].clips[0];
    clip.file.sample.as_deref_mut().unwrap().file.path = Some(user("My Samples/other.wav"));
    pick(&mut rig, REPLACE);
    rig.frame(vec![]);
    assert_eq!(file(&rig, SAMPLE), Some(quiet()));
    assert_eq!(
        vol(&rig),
        vol_points().iter().map(|p| p.tick).collect::<Vec<_>>()
    );
    assert_eq!(slot(&rig).file.path, Some(user("My Samples/other.wav")));
}

#[test]
fn a_length_changing_op_refits_the_clip_without_an_output() {
    let mut rig = rig_on_audio(mem(&[(SAMPLE, quiet())]), user_slot(SAMPLE), false);
    assert!(!rig.app.audio.has_output());
    // No envelope points: only the sample's length moves the loop.
    let lane = &mut rig.app.song.channels[1].lanes[0];
    let clip = &mut lane.clips[0];
    points::set_points(clip, VOL, Vec::new());
    clip.fit_loop(4);
    // Another clip on the same file.
    let other = clip.clone();
    lane.add_clip(1024, 4.0).file = other.file;
    rig.app.history = History::new(&rig.app.song);
    let lengths = |rig: &Rig, k: usize| {
        let c = &rig.app.song.channels[1].lanes[0].clips[k];
        (c.file.source_len_beats, c.loop_len_beats)
    };
    let lengths = |rig: &Rig| lengths(rig, 0);
    let before = lengths(&rig);
    // A beat of silence at beat 1: five beats of sample, @28 = 5, and the loop grows to 4·2 = 8.
    request(&mut rig, RegionOp::InsertSilence);
    pick(&mut rig, REPLACE);
    rig.frame(vec![]);
    assert_eq!(lengths(&rig), (5.0, 8.0), "before: {before:?}");
    // The other clip plays the replaced file too: it is refitted with it.
    let c = &rig.app.song.channels[1].lanes[0].clips[1];
    assert_eq!((c.file.source_len_beats, c.loop_len_beats), (5.0, 8.0));
    // One undo step takes the lengths back with the rest.
    undo(&mut rig);
    assert_eq!(lengths(&rig), before);
    assert!(!rig.app.history.can_undo());
}

/// `quiet()`'s length with a sine at 0.1 instead: the stretch and pitch renders differ from it everywhere.
fn tone() -> Vec<f32> {
    (0..16_000).map(|i| (i as f32 * 0.05).sin() * 0.1).collect()
}

#[test]
fn a_sync_clip_edits_and_saves_its_source_and_the_engine_gets_the_new_render() {
    let mut s = user_slot(SAMPLE);
    (s.file.stretch_mode, s.file.stretch_ratio) = (1, 2.0);
    let mut rig = rig_on(mem(&[(SAMPLE, float32_wav(RATE, &[tone()]))]), s);
    // Drawn twice as long: beats 1 … 2 are source frames 2000 … 4000.
    request(&mut rig, RegionOp::InsertSilence);
    pick(&mut rig, REPLACE);
    rig.frame(vec![]);
    let written = decode(&file(&rig, SAMPLE).unwrap()).unwrap();
    let source = tone();
    assert_eq!(written.left.len(), 18_000, "the source, not the ×2 render");
    assert_eq!(written.left[..2000], source[..2000]);
    assert!(written.left[2000..4000].iter().all(|&x| x == 0.0));
    assert_eq!(written.left[4000..], source[2000..]);
    // The engine plays a render of the new file, streaming in, and the clip is fitted to it at once: 36000 frames, 9
    // beats at 120 BPM. No whole buffer renders here.
    let render = rig.app.audio.samples.pad_sample(slot(&rig)).unwrap();
    assert_eq!(render.frames(), 36_000);
    let c = &rig.app.song.channels[1].lanes[0].clips[0];
    assert_eq!((c.file.source_len_beats, c.loop_len_beats), (9.0, 16.0));
    assert_eq!(rig.app.audio.samples.renders(), 0);
    rig.landed(usize::MAX);
    let render = rig.app.audio.samples.slot(slot(&rig)).unwrap();
    assert_eq!(render.frames(), 36_000);
}

#[test]
fn a_pitched_clip_edits_and_saves_its_source() {
    let mut s = user_slot(SAMPLE);
    s.file.pitch_semitones = 12.0;
    let mut rig = rig_on(mem(&[(SAMPLE, float32_wav(RATE, &[tone()]))]), s);
    request(&mut rig, RegionOp::Normalize);
    pick(&mut rig, REPLACE);
    rig.frame(vec![]);
    let written = decode(&file(&rig, SAMPLE).unwrap()).unwrap();
    let source = tone();
    assert_eq!(written.left.len(), 16_000);
    // Only beats 1 … 2 (frames 4000 … 8000) change.
    assert_eq!(written.left[..4000], source[..4000]);
    assert_eq!(written.left[8000..], source[8000..]);
    let peak = written.left[4000..8000]
        .iter()
        .fold(0f32, |m, x| m.max(x.abs()));
    assert_eq!(peak, 1.0);
    // The engine's +12 render is of the new file, once it has streamed in.
    rig.landed(usize::MAX);
    let render = rig.app.audio.samples.slot(slot(&rig)).unwrap();
    let fresh = honeydew_audio::drums::stretch(&written, 1.0, 12.0);
    assert_eq!(render.left, fresh.left);
}

#[test]
fn an_op_follows_its_clip_when_an_undo_under_the_question_moves_it() {
    // A (1) and B (2) play the same file; A's VOL points differ from B's.
    let mut rig = quiet_rig();
    let a_points = vec![
        Point {
            tick: 200,
            value: 1000,
            ..Default::default()
        },
        Point {
            tick: 400,
            value: 2000,
            ..Default::default()
        },
    ];
    let a = &mut rig.app.song.channels[1].lanes[0].clips[0];
    points::set_points(a, VOL, a_points.clone());
    let b_clip = {
        let mut c = a.clone();
        points::set_points(&mut c, VOL, vol_points());
        c
    };
    rig.app.song.add_channel(ChannelKind::AudioClip);
    let lane = &mut rig.app.song.channels[2].lanes[0];
    let id = lane.add_clip(0, 4.0).id;
    lane.clips[0] = crate::song::Clip { id, ..b_clip };
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let ticks = |rig: &Rig, c: usize| {
        let clip = &rig.app.song.channels[c].lanes[0].clips[0];
        points::points(clip, VOL)
            .iter()
            .map(|p| p.tick)
            .collect::<Vec<_>>()
    };
    // A goes: B is channel 1 while its op asks.
    rig.app.delete_channel(1, 0);
    rig.frame(vec![]);
    assert_eq!(ticks(&rig, 1), [160, 384]);
    request(&mut rig, RegionOp::Clear);
    assert_eq!(message(&rig), Some(vec![SAVE_QUESTION.to_owned()]));
    // Undone under the question: A is back at 1, B at 2. The op stays B's.
    undo(&mut rig);
    assert_eq!(ticks(&rig, 2), [160, 384]);
    pick(&mut rig, REPLACE);
    rig.frame(vec![]);
    assert_eq!(ticks(&rig, 1), [200, 400], "A untouched");
    assert_eq!(ticks(&rig, 2), vol_cleared(), "B took the op");
    // The op's track is gone (A deleted again under the question): it is dropped, and B, now at 1, keeps its points.
    request(&mut rig, RegionOp::InsertSilence);
    rig.app.delete_channel(1, 0);
    rig.frame(vec![]);
    pick(&mut rig, REPLACE);
    rig.frame(vec![]);
    assert_eq!(ticks(&rig, 1), vol_cleared(), "B as it was");
}

#[test]
fn preview_on_and_off_decodes_no_clip_sample_again() {
    // Preview builds the engine from the previewed clip alone (`preview::solo`: every other clip cleared), but the
    // song still plays the others: their samples stay cached, or Preview off decodes them all again.
    let other = "My Samples/other.wav";
    let mut rig = rig_on(
        mem(&[(SAMPLE, quiet()), (other, quiet())]),
        user_slot(SAMPLE),
    );
    let lane = &mut rig.app.song.channels[1].lanes[0];
    lane.add_clip(1024, 4.0).file.sample = Some(Box::new(user_slot(other)));
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let key = crate::sample_cache::SampleKey::File(user(other));
    let before = rig
        .app
        .audio
        .samples
        .get(&key)
        .expect("the other clip's sample");
    rig.click_text_button("SOLO CLIP");
    assert!(rig.app.preview.is_some(), "Preview is on");
    let during = rig.app.audio.samples.get(&key);
    assert!(
        during.is_some_and(|d| Arc::ptr_eq(&before, &d)),
        "kept while previewing"
    );
    rig.click_text_button("SOLO CLIP");
    assert!(rig.app.preview.is_none(), "Preview is off");
    let after = rig.app.audio.samples.get(&key).expect("still cached");
    assert!(Arc::ptr_eq(&before, &after), "decoded again");
}

/// The audio editor keeps a view per clip (channel, lane, id), which must follow its channel when channels or tracks
/// move, as the automation editor's do, and go with its clip.
#[test]
fn the_audio_editors_saved_view_follows_its_channel_and_goes_with_its_clip() {
    let mut rig = Rig::new();
    rig.app.add_channel(ChannelKind::AudioClip, 0);
    rig.app.nav.overlays.clear();
    let lane = &mut rig.app.song.channels[1].lanes[0];
    lane.add_clip(0, 16.0);
    lane.add_clip(4096, 16.0);
    rig.app.song_edited(SongEdit::Final, 0);
    let key = |channel, id| ClipKey {
        channel,
        lane: 0,
        id,
    };
    rig.app.navigate(Nav::EditClip(key(1, 1)));
    rig.slide_split();
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::AudioEditor(1));
    // Panned two beats on (128 points a beat) on the ruler band.
    let grid = (rig.ctx)
        .read_response(egui::Id::new("audio_editor").with("grid"))
        .expect("the grid")
        .rect;
    let from = pos2(grid.left() + 600.0, grid.top() - 0.75 * U);
    rig.press(from);
    rig.drag_to(from - vec2(128.0, 0.0));
    rig.drag_to(from - vec2(256.0, 0.0));
    rig.release(from - vec2(256.0, 0.0));
    let left = |rig: &Rig, channel: usize| {
        let views = &rig.app.slots.audio_editor.editor.views;
        views.get(&(channel, 0, 1)).map(|v| v.left)
    };
    assert_eq!(left(&rig, 1), Some(2.0));
    // A channel inserted before it: the view goes with the clip to channel 2.
    rig.app.insert_channel(1, ChannelKind::Instrument, 0);
    rig.app.nav.overlays.clear();
    assert_eq!((left(&rig, 1), left(&rig, 2)), (None, Some(2.0)));
    // Shown there, it is that view.
    rig.app.navigate(Nav::EditClip(key(2, 1)));
    rig.slide_split();
    assert_eq!(rig.app.nav.view, View::AudioEditor(2));
    assert_eq!((left(&rig, 1), left(&rig, 2)), (None, Some(2.0)));
    // Its clip gone, the view goes once the editor shows another clip.
    rig.app.song.channels[2].lanes[0]
        .clips
        .retain(|c| c.id != 1);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.navigate(Nav::EditClip(key(2, 2)));
    rig.slide_split();
    assert_eq!(rig.app.nav.view, View::AudioEditor(2));
    let views = &rig.app.slots.audio_editor.editor.views;
    assert!(
        views.keys().all(|&(_, _, id)| id == 2),
        "{:?}",
        views.keys()
    );
}

//! The sample row in the app: it replaces the keyboard while the selected channel's main lane is audio and a
//! clip is current, and its edits are song edits.

use std::path::Path;

use super::*;
use crate::app::tests::rack_wiring::picker_row;
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::song::{LaneKind, SamplerSlot};
use crate::ui::preview::ClipKey;
use crate::ui::sample_row::{Ctl, id};
use crate::ui::track_menu::TrackOp;

/// The Audio Clip channel (1), selected, whose main lane's clip 1 is its current one, with an empty log.
fn audio_rig() -> Rig {
    let mut rig = Rig::new();
    rig.app.add_channel(ChannelKind::AudioClip, 0);
    let lane = &mut rig.app.song.channels[1].lanes[0];
    lane.add_clip(0, 4.0);
    lane.file.current_clip_id = 1;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.click(OUTSIDE);
    rig.frame(vec![]);
    rig.take();
    rig
}

const KEY: ClipKey = ClipKey {
    channel: 1,
    lane: 0,
    id: 1,
};

impl Rig {
    /// The current clip's slot.
    fn clip_slot(&self) -> Option<&SamplerSlot> {
        KEY.clip(&self.app.song)?.file.sample.as_deref()
    }

    fn row_drawn(&self, ctl: Ctl) -> bool {
        self.ctx.read_response(id(ctl)).is_some()
    }

    fn undo(&mut self) {
        self.frame(key_tap(egui::Key::Z, egui::Modifiers::COMMAND));
        self.frame(vec![]);
    }
}

#[test]
fn the_row_replaces_the_keyboard_while_an_audio_clip_is_current() {
    let mut rig = audio_rig();
    assert!(rig.row_drawn(Ctl::Pick) && rig.row_drawn(Ctl::Sync));
    // No keyboard under it: a press on C4's place, left of the row (which now covers E4's), plays nothing.
    rig.click(pos2(U, E4.y));
    assert_eq!(rig.take(), Vec::<String>::new());
    // No current clip: the keyboard is back.
    rig.app.song.channels[1].lanes[0].file.current_clip_id = 0;
    // (egui reads a widget's response from the pass before the last.)
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(!rig.row_drawn(Ctl::Level));
    rig.click(E4);
    assert_eq!(rig.take(), ["NoteOn 1 52 108", "NoteOff 1 52"]);
    // The audio editor's clip is current too, from its `Edit`.
    rig.edit(1);
    rig.slide_split();
    rig.frame(vec![]);
    assert!(rig.row_drawn(Ctl::Level));
}

#[test]
fn a_toggle_is_one_undo_step_and_one_rebuild() {
    let mut rig = audio_rig();
    assert_eq!(rig.clip_slot(), None);
    rig.click_id(id(Ctl::Rev));
    assert_eq!(rig.clip_slot().map(|s| s.file.reverse), Some(true));
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    rig.click_id(id(Ctl::Sync));
    assert_eq!(rig.clip_slot().map(|s| s.file.stretch_mode), Some(1));
    rig.undo();
    assert_eq!(rig.clip_slot().map(|s| s.file.stretch_mode), Some(0));
    rig.undo();
    assert_eq!(rig.clip_slot(), None);
}

#[test]
fn a_slider_drag_writes_as_it_goes_and_its_release_is_one_undo_step_and_one_rebuild() {
    let mut rig = audio_rig();
    // A quarter of a slider's range: the thumb follows the pointer along the track, 1 U shorter than the control.
    let quarter = |rig: &Rig, ctl| {
        let rect = rig.ctx.read_response(id(ctl)).unwrap().rect;
        (rect.center(), vec2((rect.width() - U) / 4.0, 0.0))
    };
    let (at, d) = quarter(&rig, Ctl::Pan);
    rig.press(at);
    rig.drag_to(at - d / 2.0);
    rig.drag_to(at - d);
    // Written live, no rebuild yet.
    assert_eq!(rig.clip_slot().map(|s| s.params[1]), Some(0.25));
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.release(at - d);
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    // Pitch: STR1 semitones, a quarter right is +12.
    let (at, d) = quarter(&rig, Ctl::Pitch);
    rig.press(at);
    rig.drag_to(at + d / 2.0);
    rig.drag_to(at + d);
    rig.release(at + d);
    rig.frame(vec![]);
    assert_eq!(rig.clip_slot().map(|s| s.file.pitch_semitones), Some(12.0));
    rig.undo();
    assert_eq!(rig.clip_slot().map(|s| s.file.pitch_semitones), Some(0.0));
    assert_eq!(rig.clip_slot().map(|s| s.params[1]), Some(0.25));
    rig.undo();
    assert_eq!(rig.clip_slot(), None);
}

/// A 16-bit mono WAV of four silent frames whose `acid` chunk holds `tempo` BPM.
fn wav_with_tempo(tempo: f32) -> Vec<u8> {
    let chunk = |id: &[u8], body: &[u8]| [id, &(body.len() as u32).to_le_bytes(), body].concat();
    let fmt = [
        &1u16.to_le_bytes()[..],
        &1u16.to_le_bytes(),
        &44100u32.to_le_bytes(),
        &88200u32.to_le_bytes(),
        &2u16.to_le_bytes(),
        &16u16.to_le_bytes(),
    ]
    .concat();
    let mut acid = vec![0; 24];
    acid[20..].copy_from_slice(&tempo.to_le_bytes());
    let body = [
        &b"WAVE"[..],
        &chunk(b"fmt ", &fmt),
        &chunk(b"acid", &acid),
        &chunk(b"data", &[0; 8]),
    ]
    .concat();
    chunk(b"RIFF", &body)
}

#[test]
fn the_icon_opens_the_picker_for_the_clips_slot_and_a_file_with_a_tempo_turns_sync_on() {
    let mut rig = audio_rig();
    let path = Path::new("My Samples/Loop 90.wav");
    (rig.app.files).keep_user_file(path, wav_with_tempo(90.0));
    rig.click_id(id(Ctl::Pick));
    assert_eq!(rig.overlays(), [Overlay::ClipSamplePicker(KEY)]);
    // `My Files`, `My Samples` (after `My Recordings`), its first file.
    let steps = rig.app.history.steps();
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    picker_row(&mut rig, 0);
    rig.frame(vec![]);
    // It stays open, the sample strip playing the pick.
    assert_eq!(rig.overlays(), [Overlay::ClipSamplePicker(KEY)]);
    assert_eq!(
        rig.take(),
        ["Preview 4 frames from 0", "ReplaceSong adopting"]
    );
    let slot = rig.clip_slot().expect("the clip's slot").clone();
    let user = ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: "My Samples/Loop 90.wav".into(),
    };
    assert_eq!(slot.file.path, Some(user));
    assert_eq!(
        (slot.name.as_str(), slot.folder.as_str()),
        ("Loop 90", "My Samples")
    );
    // fileBPM / songBPM (the new song's 120).
    assert_eq!(
        (slot.file.stretch_mode, slot.file.stretch_ratio),
        (1, 90.0 / 120.0)
    );
    // No undo step.
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn a_clip_sample_decodes_and_renders_its_sync_on_the_worker_and_the_last_pick_lands_with_its_tempo()
{
    // The pick decodes on the worker, which reads its tempo and renders the SYNC stretch the slot will play; the slot
    // takes the last pick once it lands, and the build that follows decodes and renders nothing.
    let mut rig = audio_rig();
    let files = [("Loop 90", 90.0), ("Loop 100", 100.0)];
    for (name, tempo) in files {
        let path = format!("My Samples/{name}.wav");
        (rig.app.files).keep_user_file(Path::new(&path), wav_with_tempo(tempo));
    }
    rig.click_id(id(Ctl::Pick));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    let (steps, renders) = (rig.app.history.steps(), rig.app.audio.samples.renders());
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // `Loop 90`, then `Loop 100` (A-Z, natural) while the first still decodes.
    crate::app::tests::rack_wiring::click_row(&mut rig, 0);
    rig.frame(vec![]);
    assert!(rig.app.slots.sample_picker.loading());
    assert!(rig.painted(crate::ui::picker_preview::LOADING));
    assert_eq!(rig.clip_slot(), None);
    assert_eq!(rig.take(), Vec::<String>::new());
    crate::app::tests::rack_wiring::click_row(&mut rig, 1);
    // Only the worker's decode, from the bytes read at each pick, can play them now.
    for (name, _) in files {
        let path = format!("My Samples/{name}.wav");
        (rig.app.files).keep_user_file(Path::new(&path), b"RIFF".to_vec());
    }
    drop(hold);
    crate::app::tests::rack_wiring::worker_done(&mut rig);
    let slot = rig.clip_slot().expect("the clip's slot").clone();
    assert_eq!(slot.name, "Loop 100");
    assert_eq!(
        (slot.file.stretch_mode, slot.file.stretch_ratio),
        (1, 100.0 / 120.0)
    );
    assert_eq!(
        rig.take(),
        ["Preview 4 frames from 0", "ReplaceSong adopting"]
    );
    assert_eq!(rig.overlays(), [Overlay::ClipSamplePicker(KEY)]);
    assert_eq!(rig.app.history.steps(), steps);
    // The slot plays the worker's render; the build rendered nothing.
    let samples = &rig.app.audio.samples;
    let key = crate::sample_cache::SampleKey::of_slot(&slot).unwrap();
    assert_eq!(samples.get(&key).map(|s| s.frames()), Some(4));
    assert!(samples.slot(&slot).is_some(), "the SYNC render");
    assert_eq!(samples.renders(), renders);
    assert_eq!(crate::ui::views::sample_picker::decodes_here(), 0);
}

#[test]
fn the_picker_follows_its_clips_channel_and_closes_when_the_clip_goes() {
    let mut rig = audio_rig();
    rig.click_id(id(Ctl::Pick));
    // A channel inserted above moves the clip to channel 2.
    rig.app
        .insert_bare_channel(1, ChannelKind::Synth(SynthKind::MiniSynth), 0);
    let moved = ClipKey { channel: 2, ..KEY };
    assert_eq!(rig.overlays(), [Overlay::ClipSamplePicker(moved)]);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [Overlay::ClipSamplePicker(moved)]);
    // The clip gone (an undo under the picker): it closes.
    rig.app.song.channels[2].lanes[0].clips.clear();
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
}

#[test]
fn the_picker_follows_its_clips_track_when_a_track_above_goes_and_comes_back() {
    // Audio Clip (1): an empty main track, and clip 1 on track 1, which the audio editor holds.
    let mut rig = Rig::new();
    rig.app.add_channel(ChannelKind::AudioClip, 0);
    let ch = &mut rig.app.song.channels[1];
    let lane = ch.add_lane(LaneKind::Audio);
    ch.lanes[lane].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    let on = |lane| ClipKey { lane, ..KEY };
    rig.app.navigate(Nav::EditClip(on(1)));
    rig.slide_split();
    rig.frame(vec![]);
    rig.click_id(id(Ctl::Pick));
    assert_eq!(rig.overlays(), [Overlay::ClipSamplePicker(on(1))]);
    // The empty track above goes: the clip moves up to track 0.
    rig.app.track_op(1, 0, TrackOp::DeleteTrack, 0);
    assert_eq!(rig.overlays(), [Overlay::ClipSamplePicker(on(0))]);
    // Its undo inserts the track above again: back to track 1, still open.
    let ctx = rig.ctx.clone();
    rig.app.step(&ctx, Step::Undo, 0);
    assert_eq!(rig.overlays(), [Overlay::ClipSamplePicker(on(1))]);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [Overlay::ClipSamplePicker(on(1))]);
}

#[test]
fn the_icons_sample_pick_is_a_change() {
    // For data safety, unlike the pad icon's, the clip's sample pick sets the song-changed flag.
    let mut rig = audio_rig();
    (rig.app.files).keep_user_file(Path::new("My Samples/Loop 90.wav"), wav_with_tempo(90.0));
    rig.app.files.saved();
    rig.click_id(id(Ctl::Pick));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    picker_row(&mut rig, 0);
    assert!(rig.clip_slot().is_some());
    assert!(rig.app.files.modified());
}

#[test]
fn a_slider_drag_whose_row_goes_is_a_change_and_ends_at_the_lift() {
    // A slider's ongoing moves set the song-changed flag; a drag whose row goes before its release
    // (another track current) ends at the pointer's lift as its release would: one undo step, which the engine gets.
    let mut rig = audio_rig();
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    let level = |rig: &Rig| rig.clip_slot().map(|s| s.params[0]);
    let before = level(&rig);
    let at = rig
        .ctx
        .read_response(id(Ctl::Level))
        .expect("drawn")
        .rect
        .center();
    rig.press(at);
    rig.drag_to(at - vec2(60.0, 0.0));
    rig.drag_to(at - vec2(120.0, 0.0));
    assert_ne!(level(&rig), before);
    assert!(rig.app.files.modified(), "the ongoing moves");
    // No current clip: the keyboard is back, under the finger.
    rig.app.song.channels[1].lanes[0].file.current_clip_id = 0;
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(!rig.row_drawn(Ctl::Level));
    rig.take();
    rig.release(at - vec2(120.0, 0.0));
    rig.frame(vec![]);
    let log = rig.take();
    assert!(log.iter().any(|c| c.starts_with("ReplaceSong")), "{log:?}");
    assert_eq!(rig.app.history.steps(), steps + 1);
}

//! The pickers' audition, close and undo rules in the app: what the audio hears, which pickers stay open, and how many
//! undo steps a browse makes.

use std::path::Path;

use eframe::egui::{Id, Rect};

use super::rack_wiring::{
    BROWSER, DW, PICKER, browser_row, click_row, close_picker, instance_id, layout, pick_keys,
    picker_row, preset_button, types, with_dw_files, with_effects, worker_done,
};
use super::*;
use crate::audio_engine::slicer::detects_here;
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::song::{LaneKind, ModuleInstance};
use crate::ui::picker_preview::LOADING;
use crate::ui::rack::PresetTarget;
use crate::ui::views::preset_picker::join_workers;
use crate::ui::views::sample_picker::decodes_here;

/// A 16-bit mono WAV at 44.1 kHz of `frames` silent frames.
fn wav(frames: usize) -> Vec<u8> {
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
    let body = [
        &b"WAVE"[..],
        &chunk(b"fmt ", &fmt),
        &chunk(b"data", &vec![0; frames * 2]),
    ]
    .concat();
    chunk(b"RIFF", &body)
}

/// Row `i` of the open picker, as laid out (after the frames its slide-in takes).
fn row(rig: &mut Rig, i: usize) -> Rect {
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    let id = Id::new(PICKER).with(("row", i));
    rig.ctx.read_response(id).expect("the row is drawn").rect
}

fn picker(channel: usize, target: PresetTarget) -> Overlay {
    Overlay::PresetPicker { channel, target }
}

/// [`Rig::with_synth`] with a MiniSynth preset `Bass` in the user's files, the MiniSynth's rack open.
fn with_bass() -> Rig {
    let mut rig = Rig::with_synth();
    let mut bass = ModuleInstance::new(0, 0);
    bass.params[1] = 0.25;
    let path = Path::new("My Presets/MiniSynth/Bass.flmpst");
    let bytes = crate::flm::write_preset(&bass).unwrap();
    rig.app.files.keep_user_file(path, bytes);
    rig.open_rack(1);
    rig.slide_rack();
    rig.take();
    rig
}

#[test]
fn an_instrument_pick_is_silent_then_a_touch_on_its_row_plays_the_x_note_on_its_channel() {
    let mut rig = with_bass();
    let id = instance_id(&rig, 1, 0);
    let steps = rig.app.history.steps();
    rig.click_id(preset_button(1, id));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    // The first tap only loads it.
    assert_eq!(rig.app.song.channels[1].rack.modules[0].params[1], 0.25);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    let open = picker(1, PresetTarget::Module(id));
    assert_eq!(rig.overlays(), [Overlay::Rack(1), open.clone()]);
    // The picked row now plays: left edge 24, right edge 84, velocity 1, held until the lift; no reload.
    let bass = row(&mut rig, 0);
    for (x, note) in [(bass.left(), 24), (bass.right(), 84)] {
        let at = pos2(x, bass.center().y);
        rig.press(at);
        rig.frame(vec![]);
        assert_eq!(rig.take(), [format!("NoteOn 1 {note} 127")]);
        rig.release(at);
        assert_eq!(rig.take(), [format!("NoteOff 1 {note}")]);
    }
    assert_eq!(rig.overlays(), [Overlay::Rack(1), open]);
    // No undo step.
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn effect_presets_never_audition_a_tap_on_the_picked_row_picks_it_again() {
    let mut rig = with_effects(&[6]);
    let id = instance_id(&rig, 2, 0);
    rig.click_id(preset_button(2, id));
    // `Default`, twice: each a load, no note.
    picker_row(&mut rig, 1);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    picker_row(&mut rig, 1);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
}

#[test]
fn a_tap_outside_closes_the_picker_and_reaches_nothing_under_it() {
    let mut rig = with_bass();
    let id = instance_id(&rig, 1, 0);
    rig.click_id(preset_button(1, id));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    rig.take();
    // The rack's `+`, under the backdrop: the picker closes at once, and no Modules browser opens.
    rig.click(layout(&rig, 1).add_module.center());
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    assert_eq!(rig.take(), Vec::<String>::new());
    // The last pick stays.
    assert_eq!(rig.app.song.channels[1].rack.modules[0].params[1], 0.25);
}

#[test]
fn closing_while_a_note_is_held_releases_it() {
    let mut rig = with_bass();
    let id = instance_id(&rig, 1, 0);
    rig.click_id(preset_button(1, id));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    let bass = row(&mut rig, 0);
    rig.take();
    rig.press(bass.center());
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOn 1 54 127"]);
    rig.frame(escape());
    assert_eq!(rig.take(), ["NoteOff 1 54"]);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
}

#[test]
fn a_pick_loading_on_a_worker_auditions_at_once_and_pulses_loading_over_its_row() {
    let mut rig = with_dw_files();
    let id = instance_id(&rig, DW, 0);
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    pick_keys(&mut rig);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.painted(LOADING), "the picked row pulses");
    // A touch during the load sends a note to the channel.
    let keys = row(&mut rig, 0);
    rig.take();
    rig.press(keys.center());
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOn 2 54 127"]);
    rig.release(keys.center());
    drop(hold);
    worker_done(&mut rig);
    rig.frame(vec![]);
    assert!(!rig.painted(LOADING));
    assert_eq!(rig.app.song.channels[DW].rack.modules[0].params[1], 0.25);
    assert_eq!(
        rig.overlays(),
        [Overlay::Rack(DW), picker(DW, PresetTarget::Module(id))]
    );
}

#[test]
fn the_modules_picker_stays_open_and_each_later_pick_replaces_the_first_in_its_slot() {
    let mut rig = with_effects(&[6, 11]);
    let steps = rig.app.history.steps();
    let distortion = instance_id(&rig, 2, 0);
    // `Insert below` the Distortion: the first pick goes there.
    let at = crate::ui::modules_browser::InsertAt::Below(distortion);
    rig.app
        .navigate(Nav::Open(Overlay::ModulesBrowser { channel: 2, at }));
    browser_row(&mut rig, 0);
    // Rows: Paste 0, Rack Preset 1, SYNTH 2 (dimmed on an effect track), …, ECHO, Reverb …
    rig.click_id(Id::new(BROWSER).with("search"));
    let browse = |rig: &mut Rig, name: &str| {
        rig.frame(vec![eframe::egui::Event::Text(name.into())]);
        rig.frame(vec![]);
        let hit = browser_row(rig, 1).expect("the search finds it");
        rig.click(hit.center());
        // Search again from an empty field: `Clear` keeps the bar open.
        rig.click_id(Id::new(BROWSER).with("query").with("clear"));
    };
    browse(&mut rig, "Tape D");
    assert_eq!(types(&rig, 2), [6, 18, 11]);
    browse(&mut rig, "Phaser");
    assert_eq!(types(&rig, 2), [6, 14, 11], "the same slot");
    browse(&mut rig, "Flanger");
    assert_eq!(types(&rig, 2), [6, 13, 11]);
    assert!(
        rig.overlays()
            .iter()
            .any(|o| matches!(o, Overlay::ModulesBrowser { .. })),
        "it stays open"
    );
    // The whole browse adds one module, with no undo step.
    assert_eq!(rig.app.history.steps(), steps);
    // A new session places its first pick again. With the search bar shown, the first Escape hides it.
    close_picker(&mut rig);
    close_picker(&mut rig);
    rig.app
        .navigate(Nav::Open(Overlay::ModulesBrowser { channel: 2, at }));
    browser_row(&mut rig, 0);
    rig.click_id(Id::new(BROWSER).with("search"));
    browse(&mut rig, "Tape D");
    assert_eq!(types(&rig, 2), [6, 18, 13, 11]);
}

#[test]
fn paste_in_the_modules_picker_closes_it() {
    let mut rig = with_effects(&[6]);
    rig.click(layout(&rig, 2).add_module.center());
    let paste = browser_row(&mut rig, 0).expect("Paste");
    rig.click(paste.center());
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
}

#[test]
fn the_add_channel_instrument_holds_a_supersaw_and_opens_the_instruments_picker_which_closing_keeps()
 {
    // With no `Close Grand` in the content, as here (`new_channel` has it loaded).
    let mut rig = Rig::with_synth();
    rig.app.insert_channel(2, ChannelKind::Instrument, 0);
    let ch = &rig.app.song.channels[2];
    assert_eq!(ch.name, "Instrument");
    let supersaw = SynthKind::SuperSaw.type_id();
    assert_eq!(types(&rig, 2), [supersaw]);
    // The Instruments picker, not the SuperSaw's own presets.
    assert_eq!(rig.overlays(), [picker(2, PresetTarget::Instruments)]);
    rig.frame(vec![]);
    rig.frame(escape());
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    assert_eq!(types(&rig, 2), [supersaw]);
}

/// A sample picker of `rig` over `My Files`, `My Samples`: `A.wav` (row 0) and `B.wav` (row 1).
fn samples(rig: &mut Rig) {
    for (name, frames) in [("A", 4410), ("B", 8820)] {
        let path = format!("My Samples/{name}.wav");
        rig.app.files.keep_user_file(Path::new(&path), wav(frames));
    }
}

#[test]
fn new_clip_on_an_empty_audio_track_cell_opens_a_picker_whose_browse_is_one_clip_and_one_undo_step()
{
    let mut rig = Rig::new();
    rig.app.add_channel(ChannelKind::AudioClip, 0);
    rig.click(OUTSIDE);
    samples(&mut rig);
    rig.frame(vec![]);
    rig.take();
    let steps = rig.app.history.steps();
    // A tap on the current audio track's empty space (track 1's row, past its header) shows the insert menu, whose
    // `New Clip` opens the picker.
    let at = pos2(600.0, header(1).y);
    rig.click(at);
    rig.frame(vec![]);
    let new_clip = rig.ctx.read_response(egui::Id::new("clip_insert").with(0));
    rig.click(new_clip.expect("the insert menu").rect.center());
    rig.frame(vec![]);
    let [Overlay::TrackSamplePicker { channel, lane, .. }] = rig.overlays()[..] else {
        panic!("{:?}", rig.overlays());
    };
    assert_eq!((channel, lane), (1, 0));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    // The first pick makes the clip: one undo step; the strip plays it.
    picker_row(&mut rig, 0);
    let clips = |rig: &Rig| rig.app.song.channels[1].lanes[0].clips.clone();
    let first = clips(&rig);
    assert_eq!(first.len(), 1);
    let name = |c: &crate::song::Clip| c.file.sample.as_ref().map(|s| s.name.clone());
    assert_eq!(name(&first[0]).as_deref(), Some("A"));
    assert!(first[0].file.source_len_beats > 0.0, "fitted to its sample");
    assert_eq!(rig.app.history.steps(), steps + 1);
    assert_eq!(
        rig.take(),
        ["Preview 4410 frames from 0", "ReplaceSong adopting"]
    );
    // A later pick reloads that clip, with no step; the new pick stops the preview playing.
    picker_row(&mut rig, 1);
    let second = clips(&rig);
    assert_eq!((second.len(), second[0].id), (1, first[0].id));
    assert_eq!(name(&second[0]).as_deref(), Some("B"));
    assert_eq!(rig.app.history.steps(), steps + 1);
    let sent = rig.take();
    assert!(
        sent.contains(&"Preview 8820 frames from 0".to_owned()),
        "{sent:?}"
    );
    // One undo takes the whole browse back.
    close_picker(&mut rig);
    rig.frame(z(egui::Modifiers::NONE));
    assert!(clips(&rig).is_empty());
}

#[test]
fn a_pad_sample_pick_stays_open_plays_on_the_preview_and_reloads_the_same_new_pad() {
    let mut rig = Rig::new();
    rig.app.add_default_drums(0);
    samples(&mut rig);
    rig.click(OUTSIDE);
    rig.take();
    let pads = rig.app.song.channels[1]
        .drum_kit
        .as_ref()
        .unwrap()
        .slots
        .len();
    let steps = rig.app.history.steps();
    rig.app.navigate(Nav::Open(Overlay::SamplePicker {
        channel: 1,
        pad: None,
    }));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    picker_row(&mut rig, 0);
    let open = Overlay::SamplePicker {
        channel: 1,
        pad: Some(pads as u8),
    };
    assert_eq!(rig.overlays(), [open]);
    assert_eq!(
        rig.take(),
        [
            "Preview 4410 frames from 0",
            "ReplaceSong adopting no-resync"
        ]
    );
    // The next pick reloads the pad the first one added.
    picker_row(&mut rig, 1);
    let kit = rig.app.song.channels[1].drum_kit.as_ref().unwrap();
    assert_eq!(kit.slots.len(), pads + 1);
    assert_eq!(kit.slots[pads].name, "B");
    assert_eq!(rig.app.history.steps(), steps);
    // Closing stops the preview.
    rig.take();
    close_picker(&mut rig);
    assert_eq!(rig.take(), ["Preview stop"]);
}

/// The Drums channel (1) of a fresh rig with `samples`, its sample picker open on pad 0 at `My Samples`, an empty log.
fn pad_picker() -> Rig {
    let mut rig = Rig::new();
    rig.app.add_default_drums(0);
    samples(&mut rig);
    rig.click(OUTSIDE);
    rig.app.navigate(Nav::Open(Overlay::SamplePicker {
        channel: 1,
        pad: Some(0),
    }));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    rig.take();
    rig
}

/// Pad 0's slot.
fn pad(rig: &Rig) -> crate::song::SamplerSlot {
    rig.app.song.channels[1].drum_kit.as_ref().unwrap().slots[0].clone()
}

#[test]
fn a_pad_sample_decodes_on_the_worker_pulsing_loading_and_the_last_pick_wins() {
    // The pick decodes on the worker. Meanwhile its row pulses, the strip names it with no waveform and no preview, and
    // The pad is as it was.
    let mut rig = pad_picker();
    rig.app.files.saved();
    let (before, steps) = (pad(&rig), rig.app.history.steps());
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    click_row(&mut rig, 0);
    rig.frame(vec![]);
    assert!(rig.app.slots.sample_picker.loading());
    assert!(rig.painted(LOADING), "the picked row pulses");
    assert_eq!(
        rig.app.slots.sample_picker.strip(),
        Some(("A", false)),
        "its name only"
    );
    assert_eq!(pad(&rig), before);
    assert_eq!(rig.take(), Vec::<String>::new());
    // `B` replaces `A`, still decoding: `A` never lands.
    click_row(&mut rig, 1);
    // Only the worker's decode, from the bytes read at the pick, can give the engine `B` now.
    rig.app
        .files
        .keep_user_file(Path::new("My Samples/B.wav"), b"RIFF".to_vec());
    drop(hold);
    worker_done(&mut rig);
    assert_eq!(pad(&rig).name, "B");
    // A pad sample pick sets no song-changed flag (app-set, like the other sample picks).
    assert!(!rig.app.files.modified());
    assert_eq!(
        rig.take(),
        [
            "Preview 8820 frames from 0",
            "ReplaceSong adopting no-resync"
        ]
    );
    assert_eq!(rig.app.slots.sample_picker.strip(), Some(("B", true)));
    let sample = rig.app.audio.samples.slot(&pad(&rig));
    assert_eq!(sample.map(|s| s.frames()), Some(8820));
    assert!(!rig.painted(LOADING));
    assert_eq!(rig.app.history.steps(), steps);
    assert_eq!(decodes_here(), 0, "decoded on the worker");
}

#[test]
fn a_pad_picks_str1_streams_its_first_8192_frames_with_the_pick_and_the_rest_on_the_worker() {
    // The pad's own STR1 (time-stretch ×2) renders off the UI thread, as a clip's SYNC does. The pad pick goes with
    // Stretch: the pick lands with the first 8192 frames rendered, which the pad plays at once; the rest streams in on
    // The worker, and the engine is rebuilt with the whole render once it lands.
    use honeydew_audio::drums::{PadSample, STREAM_FRAMES};
    let mut rig = Rig::new();
    rig.app.add_default_drums(0);
    samples(&mut rig);
    rig.click(OUTSIDE);
    let slot = &mut rig.app.song.channels[1].drum_kit.as_mut().unwrap().slots[0];
    (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 2.0);
    rig.app.navigate(Nav::Open(Overlay::SamplePicker {
        channel: 1,
        pad: Some(0),
    }));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    let renders = rig.app.audio.samples.renders();
    rig.take();
    click_row(&mut rig, 1);
    // The pick's decode lands; the rest of the render waits on the worker hold taken now.
    let rest = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    worker_done(&mut rig);
    assert_eq!(pad(&rig).name, "B");
    assert_eq!(pad(&rig).file.stretch_ratio, 2.0, "a pad's STR1 stays");
    let Some(PadSample::Streaming(stream)) = rig.app.audio.samples.pad_sample(&pad(&rig)) else {
        panic!("the pad plays the stream at once");
    };
    assert_eq!(
        (stream.filled(), stream.frames()),
        (STREAM_FRAMES, 2 * 8820)
    );
    assert_eq!(rig.app.slots.drum_editor.stretching(), 1);
    assert!(
        rig.take()
            .contains(&"ReplaceSong adopting no-resync".to_owned())
    );
    drop(rest);
    crate::ui::views::preset_picker::join_workers();
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    assert_eq!(stream.filled(), 2 * 8820);
    let sample = rig.app.audio.samples.slot(&pad(&rig));
    assert_eq!(sample.map(|s| s.frames()), Some(2 * 8820));
    assert_eq!(
        rig.app.audio.samples.renders(),
        renders,
        "rendered on the worker"
    );
    assert_eq!(decodes_here(), 0);
}

#[test]
fn a_pick_still_decoding_when_its_picker_goes_never_lands() {
    // A picker the app closes (its kit changed under it) or takes away drops its pick still decoding; one
    // opened in its place, even the very next frame, never sees it.
    for taken_away in [false, true] {
        let mut rig = pad_picker();
        let hold = crate::ui::views::preset_picker::WORKER_HOLD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        click_row(&mut rig, 0);
        rig.frame(vec![]);
        assert!(rig.app.slots.sample_picker.loading());
        let kit = rig.app.song.channels[1].drum_kit.as_mut().unwrap();
        kit.slots[1].params[0] = 0.9;
        match taken_away {
            // The app takes the overlay away; a frame passes without it.
            true => {
                rig.app.nav.overlays.clear();
                rig.frame(vec![]);
            }
            // Its kit changed: the picker closes itself.
            false => {
                rig.frame(vec![]);
                assert_eq!(rig.overlays(), []);
            }
        }
        let before = pad(&rig);
        rig.app.navigate(Nav::Open(Overlay::SamplePicker {
            channel: 1,
            pad: Some(0),
        }));
        drop(hold);
        // Once the dropped pick's worker has finished, a wrong pick would land in the next frames.
        join_workers();
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        assert!(!rig.app.slots.sample_picker.loading(), "{taken_away}");
        assert_eq!(pad(&rig), before, "{taken_away}: nothing lands");
    }
}

#[test]
fn on_the_desktop_the_worker_reads_the_picked_file() {
    // The desktop's pick is read by the worker from disk (the web's copies at the pick): a file rewritten
    // before the worker runs is the one it decodes.
    use crate::project_files::content::tests::{Temp, put};
    let tmp = Temp::new("pw2-desktop-pick");
    put(&tmp.0.join("My Samples/A.wav"), &wav(4410));
    let mut rig = Rig::new();
    rig.app.files = crate::project_files::ProjectFiles::on_disk(&tmp.0);
    rig.app.add_default_drums(0);
    rig.click(OUTSIDE);
    rig.app.navigate(Nav::Open(Overlay::SamplePicker {
        channel: 1,
        pad: Some(0),
    }));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    click_row(&mut rig, 0);
    put(&tmp.0.join("My Samples/A.wav"), &wav(8820));
    drop(hold);
    worker_done(&mut rig);
    assert_eq!(pad(&rig).name, "A");
    let sample = rig.app.audio.samples.slot(&pad(&rig));
    assert_eq!(sample.map(|s| s.frames()), Some(8820));
    assert_eq!(decodes_here(), 0);
}

#[test]
fn closing_a_sample_picker_while_its_pick_decodes_lands_the_pick_then_closes() {
    // Closing never reverts a pick: the X, Escape or a tap outside while it decodes waits for it.
    let mut rig = pad_picker();
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    click_row(&mut rig, 0);
    rig.frame(escape());
    rig.frame(vec![]);
    assert!(rig.app.slots.sample_picker.loading());
    assert!(!rig.painted("A"), "nothing drawn");
    drop(hold);
    worker_done(&mut rig);
    assert_eq!(rig.overlays(), []);
    assert_eq!(pad(&rig).name, "A");
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
}

#[test]
fn a_slicer_pick_of_an_audio_file_loads_names_it_and_shows_the_sample_strip_never_the_audition() {
    let mut rig = Rig::with_synth();
    rig.app
        .add_bare_channel(ChannelKind::Synth(SynthKind::Slicer), 0);
    let dir = rig.app.files.presets_dir(33).unwrap();
    rig.app
        .files
        .keep_user_file(&dir.join("Loop.wav"), wav(4410));
    rig.open_rack(2);
    rig.slide_rack();
    rig.take();
    let id = instance_id(&rig, 2, 0);
    rig.click_id(preset_button(2, id));
    picker_row(&mut rig, 0);
    // The loop decodes on the worker. Meanwhile its row pulses and the strip names it, with no waveform and
    // no preview; nothing is loaded yet.
    let before = rig.app.song.channels[2].rack.modules[0].clone();
    let detects = detects_here();
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    picker_row(&mut rig, 0);
    rig.frame(vec![]);
    assert!(rig.app.slots.preset_picker.loading());
    assert!(rig.painted(LOADING), "the picked row pulses");
    assert!(rig.app.slots.preset_picker.sample_shown());
    assert_eq!(rig.take(), Vec::<String>::new());
    assert_eq!(rig.app.song.channels[2].rack.modules[0], before);
    // Its bytes went to the worker at the pick: the file turning bad now changes nothing.
    rig.app
        .files
        .keep_user_file(&dir.join("Loop.wav"), b"RIFF".to_vec());
    drop(hold);
    worker_done(&mut rig);
    // Once it lands, the strip plays it.
    assert!(rig.app.slots.preset_picker.sample_shown());
    assert_eq!(
        rig.take(),
        [
            "Preview 4410 frames from 0",
            "ReplaceSong adopting no-resync"
        ]
    );
    assert_eq!(
        rig.overlays()
            .iter()
            .filter(|o| matches!(o, Overlay::Message { .. }))
            .count(),
        0
    );
    let module = &rig.app.song.channels[2].rack.modules[0];
    assert_eq!(module.file.preset_name, "Loop");
    let blob = module.file.custom_data.as_deref().expect("a CSTM");
    let named = b"Loop.wav";
    assert!(blob.windows(named.len()).any(|w| w == named));
    let key = crate::sample_cache::SampleKey::of_path(
        &rig.app.files.shorthand(&dir.join("Loop.wav")).unwrap(),
    );
    assert_eq!(
        rig.app.audio.samples.get(&key).map(|s| s.frames()),
        Some(4410)
    );
    assert_eq!(decodes_here(), 0, "decoded on the worker");
    // Sliced there too, so AUTO's clip and the build detect nothing here.
    assert_eq!(detects_here(), detects, "sliced on the worker");
    assert!(!rig.painted(LOADING), "loaded, and no audition strip");
    // A tap on the picked row picks it again: the strip plays it from the start, and no note sounds.
    let looped = row(&mut rig, 0);
    rig.app
        .files
        .keep_user_file(&dir.join("Loop.wav"), wav(4410));
    rig.click(looped.center());
    worker_done(&mut rig);
    let sent = rig.take();
    assert!(!sent.iter().any(|c| c.starts_with("NoteOn")), "{sent:?}");
    assert!(
        sent.contains(&"Preview 4410 frames from 0".to_owned()),
        "{sent:?}"
    );
    assert_eq!(rig.app.song.channels[2].lanes[0].kind, LaneKind::Notes);
}

/// 3x Osc's preset picker lists audio files (its `3xOsc Shapes`, the user's): a pick names the file in its `CSTM`, and
/// its Custom table is built on the worker while the audition strip, on at once, pulses `LOADING...`.
#[test]
fn a_three_osc_shape_from_its_preset_picker_builds_its_table_on_the_worker() {
    let mut rig = Rig::with_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::ThreeXOsc), 0);
    let dir = rig.app.files.presets_dir(39).unwrap();
    let file = dir.join("Wave.wav");
    let shape = crate::sample_cache::tests::wav(&[0, 9000, 0, -9000]);
    rig.app.files.keep_user_file(&file, shape);
    rig.open_rack(2);
    rig.slide_rack();
    rig.take();
    let id = instance_id(&rig, 2, 0);
    rig.click_id(preset_button(2, id));
    picker_row(&mut rig, 0);
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    picker_row(&mut rig, 0);
    rig.frame(vec![]);
    let shape_path = |rig: &Rig| {
        crate::audio_engine::samples::shape_path(&rig.app.song.channels[2].rack.modules[0])
    };
    assert!(rig.app.slots.preset_picker.loading());
    assert!(rig.painted(LOADING), "the picked row pulses");
    assert_eq!(shape_path(&rig), None);
    // Only the worker's decode, from the bytes read at the pick, can build the table now.
    rig.app.files.keep_user_file(&file, b"RIFF".to_vec());
    drop(hold);
    worker_done(&mut rig);
    let shorthand = rig.app.files.shorthand(&file);
    assert_eq!(shape_path(&rig), shorthand);
    let key = crate::sample_cache::SampleKey::of_path(&shorthand.unwrap());
    assert!(
        rig.app.audio.samples.shape(&key).is_some(),
        "the worker's table"
    );
    assert_eq!(decodes_here(), 0, "built on the worker");
    assert_eq!(
        rig.app.song.channels[2].rack.modules[0].file.preset_name,
        "Wave"
    );
    assert!(!rig.painted(LOADING));
    assert_eq!(
        rig.overlays(),
        [Overlay::Rack(2), picker(2, PresetTarget::Module(id))]
    );
}

#[test]
fn a_later_slicer_loop_replaces_one_still_loading() {
    // `A`, then `B` while `A` still decodes on the worker: only `B` lands, its sample and its preview.
    let mut rig = Rig::with_synth();
    rig.app
        .add_bare_channel(ChannelKind::Synth(SynthKind::Slicer), 0);
    let dir = rig.app.files.presets_dir(33).unwrap();
    for (name, frames) in [("A", 4410), ("B", 8820)] {
        let path = dir.join(format!("{name}.wav"));
        rig.app.files.keep_user_file(&path, wav(frames));
    }
    rig.open_rack(2);
    rig.slide_rack();
    let id = instance_id(&rig, 2, 0);
    rig.click_id(preset_button(2, id));
    picker_row(&mut rig, 0);
    rig.take();
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    // Only the worker's decode, from the bytes read at each pick, can play them now.
    for name in ["A", "B"] {
        let path = dir.join(format!("{name}.wav"));
        rig.app.files.keep_user_file(&path, b"RIFF".to_vec());
    }
    drop(hold);
    worker_done(&mut rig);
    let module = &rig.app.song.channels[2].rack.modules[0];
    let named = b"B.wav";
    let blob = module.file.custom_data.as_deref().expect("a CSTM");
    assert!(blob.windows(named.len()).any(|w| w == named));
    assert_eq!(module.file.preset_name, "B");
    let b = rig.app.files.shorthand(&dir.join("B.wav")).unwrap();
    let key = crate::sample_cache::SampleKey::of_path(&b);
    assert_eq!(
        rig.app.audio.samples.get(&key).map(|s| s.frames()),
        Some(8820)
    );
    assert_eq!(
        rig.take(),
        [
            "Preview 8820 frames from 0",
            "ReplaceSong adopting no-resync"
        ]
    );
    assert_eq!(decodes_here(), 0);
}

#[test]
fn a_later_pick_on_the_ui_thread_replaces_a_pick_still_loading() {
    // `Keys` (a DW program, on the worker), then `Default` (at once): `Keys` must not land over it.
    let mut rig = with_dw_files();
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    pick_keys(&mut rig);
    rig.frame(vec![]);
    assert!(rig.app.slots.preset_picker.loading());
    // Back to the root, then `Default`, which loads on the UI thread at once.
    rig.click_id(Id::new(PICKER).with("back"));
    picker_row(&mut rig, 1);
    let last = rig.app.song.channels[DW].rack.modules[0].clone();
    assert!(
        !rig.app.slots.preset_picker.loading(),
        "the load is dropped"
    );
    drop(hold);
    join_workers();
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    assert_eq!(
        rig.app.song.channels[DW].rack.modules[0], last,
        "the last pick stays"
    );
}

#[test]
fn a_slicer_preset_pick_shows_no_sample_strip() {
    // Only an audio file (a loop) shows the strip; a `.flmpst` preset has no sample to show.
    let mut rig = Rig::with_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::Slicer), 0);
    let dir = rig.app.files.presets_dir(33).unwrap();
    let preset = crate::flm::write_preset(&ModuleInstance::new(33, 0)).unwrap();
    rig.app
        .files
        .keep_user_file(&dir.join("Chop.flmpst"), preset);
    rig.open_rack(2);
    rig.slide_rack();
    rig.take();
    let id = instance_id(&rig, 2, 0);
    rig.click_id(preset_button(2, id));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    worker_done(&mut rig);
    rig.frame(vec![]);
    assert!(!rig.app.slots.preset_picker.sample_shown(), "no strip");
    assert!(rig.take().iter().all(|c| !c.starts_with("Preview")));
}

// ---- the pad sample picker's start folder (icon 21) ----

/// Opens the pad sample picker (icon 21, `Load Sample`) on pad 0 of a new Drums channel (1), whose slot plays `path`
/// (`None`: its built-in one-shot, no file), and lets it settle.
fn open_pad_picker(rig: &mut Rig, path: Option<ContentPath>) {
    rig.app.add_default_drums(0);
    rig.app.song.channels[1].drum_kit.as_mut().unwrap().slots[0]
        .file
        .path = path;
    rig.click(OUTSIDE);
    rig.app.navigate(Nav::Open(Overlay::SamplePicker {
        channel: 1,
        pad: Some(0),
    }));
    for _ in 0..3 {
        rig.frame(vec![]);
    }
}

fn shorthand(root: ContentRoot, relative: &str) -> Option<ContentPath> {
    Some(ContentPath::Shorthand {
        root,
        relative: relative.into(),
    })
}

/// The header's breadcrumb, as painted (a long one is truncated in its middle).
fn crumb(rig: &Rig) -> String {
    let texts = rig.out.shapes.iter().filter_map(|s| match &s.shape {
        egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
        _ => None,
    });
    let mut crumbs = texts.filter(|t| t.starts_with("     Samples | "));
    crumbs.next().expect("a breadcrumb")
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_pads_picker_opens_at_its_my_samples_folder_with_the_row_in_view() {
    // If the pad has a sample, the picker opens at that sample's folder with the row revealed. On the desktop: the
    // web's copies have no subfolders.
    use crate::project_files::content::tests::{Temp, put};
    let tmp = Temp::new("pk2-pad-my-samples");
    for i in 0..40 {
        put(
            &tmp.0.join(format!("My Samples/Kicks/Kick {i:02}.wav")),
            &wav(10),
        );
    }
    let mut rig = Rig::new();
    rig.app.files = crate::project_files::ProjectFiles::on_disk(&tmp.0);
    let path = shorthand(ContentRoot::User, "My Samples/Kicks/Kick 39.wav");
    open_pad_picker(&mut rig, path);
    assert!(crumb(&rig).ends_with("Samples | Kicks"), "{}", crumb(&rig));
    assert!(rig.painted("Kick 39"), "the row is revealed");
    assert!(!rig.painted("Kick 00"), "scrolled to it");
    // Back goes up the folders above it, as taps on them would have opened them.
    let back = |rig: &mut Rig| {
        rig.click_id(Id::new(PICKER).with("back"));
        rig.frame(vec![]);
    };
    back(&mut rig);
    assert!(crumb(&rig).ends_with(" | My Samples") && rig.painted("Kicks"));
    back(&mut rig);
    assert!(rig.painted("     Samples | My Files") && rig.painted("My Recordings"));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_pads_picker_opens_at_its_original_drum_samples_folder() {
    use crate::project_files::CONTENT_FOLDER;
    use crate::project_files::content::tests::{Temp, fake_tree};
    let tmp = Temp::new("pk2-pad-start-folder");
    fake_tree(&tmp.0.join(CONTENT_FOLDER));
    let mut rig = Rig::new();
    rig.app.files = crate::project_files::ProjectFiles::on_disk(&tmp.0);
    let path = shorthand(ContentRoot::App, "Drum Samples/Snare/Gen Snare.wav");
    open_pad_picker(&mut rig, path);
    // The root lists the factory folders (`Kick`, `Snare`) beside `My Files`.
    assert!(rig.painted("     Samples | Snare"));
    assert!(rig.painted("Gen Snare"));
    rig.click_id(Id::new(PICKER).with("back"));
    rig.frame(vec![]);
    assert!(rig.painted("Samples") && rig.painted("Kick"));
}

#[test]
fn an_empty_pads_picker_opens_at_the_root() {
    // The starting folder for an empty pad: the root, as before. A built-in one-shot has no file.
    let mut rig = Rig::new();
    samples(&mut rig);
    open_pad_picker(&mut rig, None);
    assert!(rig.painted("Samples") && rig.painted("My Files"));
    assert!(!rig.painted("A"));
}

#[test]
fn add_drums_new_pad_and_an_empty_cell_browses_later_pick_are_changes() {
    // For data safety only the pad icon's sample load is app-set; Add Drum's new pad, and each later pick of the
    // empty-cell browse (its first makes an undo step), set the song-changed flag.
    let mut rig = Rig::new();
    rig.app.add_default_drums(0);
    samples(&mut rig);
    rig.click(OUTSIDE);
    rig.app.files.saved();
    rig.app.navigate(Nav::Open(Overlay::SamplePicker {
        channel: 1,
        pad: None,
    }));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    picker_row(&mut rig, 0);
    assert!(rig.app.files.modified(), "Add Drum's pad");
    close_picker(&mut rig);

    let mut rig = Rig::new();
    rig.app.add_channel(ChannelKind::AudioClip, 0);
    rig.click(OUTSIDE);
    samples(&mut rig);
    rig.frame(vec![]);
    rig.click(pos2(600.0, header(1).y));
    rig.frame(vec![]);
    let new_clip = rig.ctx.read_response(egui::Id::new("clip_insert").with(0));
    rig.click(new_clip.expect("the insert menu").rect.center());
    rig.frame(vec![]);
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    picker_row(&mut rig, 0);
    assert_eq!(rig.app.song.channels[1].lanes[0].clips.len(), 1);
    rig.app.files.saved();
    picker_row(&mut rig, 1);
    assert!(rig.app.files.modified(), "the browse's later pick");
}

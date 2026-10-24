//! A new channel: the add-channel `Instrument` loads `Close Grand` where the content has it (else the SuperSaw);
//! `Drums` and `Instrument` prompt for their sound from the `+` and from `Insert Channel` alike; and a Drums,
//! Instrument or Synth channel starts with a clip at the song start, current, its editor open under the picker.

use eframe::egui::Id;

use super::instruments_picker::PLUCK;
use super::rack_wiring::{click_row, types, worker_done};
use super::*;
use crate::song::ModuleInstance;
use crate::ui::modules_browser::InsertAt;
use crate::ui::add_channel_menu::{AddChannelPick, add_channel_menu};
use crate::ui::rack::PresetTarget;

fn picker(channel: usize, target: PresetTarget) -> Overlay {
    Overlay::PresetPicker { channel, target }
}

fn center(rig: &Rig, id: Id) -> Pos2 {
    rig.ctx.read_response(id).unwrap().rect.center()
}

#[test]
fn a_new_drums_instrument_or_synth_channel_starts_with_a_bar_at_the_song_start_its_editor_open() {
    for (kind, view, prompt) in [
        (
            ChannelKind::Drums,
            View::DrumEditor(2),
            Some(PresetTarget::Drumset),
        ),
        (
            ChannelKind::Instrument,
            View::PianoRoll(2),
            Some(PresetTarget::Instruments),
        ),
        (
            ChannelKind::Synth(SynthKind::MiniSynth),
            View::PianoRoll(2),
            None,
        ),
    ] {
        let mut rig = Rig::with_synth();
        rig.app.files.saved();
        let steps = rig.app.history.steps();
        rig.app.add_channel(kind, 0);
        rig.frame(vec![]);
        // One bar (4/4) at tick 0, the tap's `Add clip`, current on the primary track.
        let lane = &rig.app.song.channels[2].lanes[0];
        let starts: Vec<_> = (lane.clips.iter())
            .map(|c| (c.start_tick, c.loop_len_beats))
            .collect();
        assert_eq!(starts, [(0, 4.0)], "{kind:?}");
        assert_eq!(lane.file.current_clip_id, lane.clips[0].id, "{kind:?}");
        assert_eq!(rig.app.nav.view, view, "{kind:?}");
        let prompted: Vec<_> = prompt.map(|t| picker(2, t)).into_iter().collect();
        assert_eq!(rig.overlays(), prompted, "{kind:?}");
        // The channel and its clip are one undo step, and the song is changed.
        assert_eq!(rig.app.history.steps(), steps + 1, "{kind:?}");
        assert!(rig.app.files.modified(), "{kind:?}");
        // No content: the Instrument keeps its SuperSaw.
        if kind == ChannelKind::Instrument {
            assert_eq!(types(&rig, 2), [23]);
        }
        // Closing the picker leaves the editor open.
        rig.frame(escape());
        rig.frame(vec![]);
        assert_eq!(rig.overlays(), [], "{kind:?}");
        assert_eq!(rig.app.nav.view, view, "{kind:?}");
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!(rig.app.song.channels.len(), 2, "{kind:?}: one undo");
    }
}

#[test]
fn an_audio_clip_audio_record_or_effect_channel_gets_no_clip_and_no_editor() {
    for kind in [
        ChannelKind::AudioClip,
        ChannelKind::AudioRecord,
        ChannelKind::EffectTrack,
    ] {
        let mut rig = Rig::with_synth();
        rig.app.add_channel(kind, 0);
        rig.frame(vec![]);
        let ch = &rig.app.song.channels[2];
        assert!(ch.lanes.iter().all(|l| l.clips.is_empty()), "{kind:?}");
        assert_eq!(rig.app.nav.view, View::Playlist, "{kind:?}");
    }
}

#[test]
fn drums_and_instrument_prompt_for_their_sound_from_the_plus_and_from_insert_channel() {
    for (row, target) in [(1, PresetTarget::Drumset), (2, PresetTarget::Instruments)] {
        // The `+`: at the end.
        let mut rig = Rig::with_synth();
        rig.click(center(&rig, Id::new("add_channel_button")));
        rig.frame(vec![]);
        rig.click(center(
            &rig,
            Id::new("add_channel_popup").with((0_usize, row)),
        ));
        assert_eq!(rig.overlays(), [picker(2, target)], "+ {target:?}");
        // `Insert Channel` on the synth's header: above it.
        let mut rig = Rig::with_synth();
        rig.track_menu(1, (0, 3));
        let item = Id::new("insert_channel_popup").with((0_usize, row));
        rig.click(center(&rig, item));
        assert_eq!(rig.overlays(), [picker(1, target)], "insert {target:?}");
    }
}

#[test]
fn a_plugin_pick_from_the_add_menu_creates_a_ready_instrument_track() {
    let uid = crate::plugins::builtin::LEMON_TRIO.uid;
    let item = add_channel_menu().into_iter()
        .find(|item| item.id == Some(AddChannelPick::Plugin(uid)))
        .expect("built-in instrument plugin is listed");
    for insert in [false, true] {
        let mut rig = Rig::with_synth();
        let steps = rig.app.history.steps();
        let at = if insert {
            rig.track_menu(1, (0, 3));
            1
        } else {
            rig.click(center(&rig, Id::new("add_channel_button")));
            rig.frame(vec![]);
            2
        };
        let popup = if insert { "insert_channel_popup" } else { "add_channel_popup" };
        rig.click(center(&rig, Id::new(popup).with((item.col, item.row))));
        let channel = &rig.app.song.channels[at];
        assert_eq!(channel.name, "Lemon Trio");
        assert_eq!(channel.lanes[0].clips.len(), 1);
        assert_eq!(channel.rack.modules.len(), 1);
        assert_eq!(channel.rack.modules[0].file.plugin.as_ref().unwrap().name, "Lemon Trio");
        assert_eq!(rig.app.selected_channel, at);
        assert_eq!(rig.app.history.steps(), steps + 1);
        assert!(!rig.overlays().iter().any(|o| matches!(o, Overlay::PresetPicker { .. })));
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!(rig.app.song.channels.len(), 2, "the plugin track is one undo step");
    }
}

/// Close Grand as the content keeps it: an Essential Pianos preset of the DW Sampler in the content
/// folder, whose zone names its sample `@L@Online Content/…`; the sample, when `zone`, in the content folder's
/// `DirectWave Samples EXP`.
#[cfg(not(target_arch = "wasm32"))]
fn close_grand_rig(zone: bool) -> (Rig, crate::project_files::content::tests::Temp) {
    use crate::project_files::CONTENT_FOLDER;
    use crate::project_files::content::tests::{Temp, put};
    // One folder per test thread: tests run at once, and a folder's drop deletes it.
    let thread = std::thread::current().id();
    let tmp = Temp::new(&format!("nc-close-grand-{zone}-{thread:?}"));
    let sample = "Essential Pianos EXP/Close Grand/Close Grand 1.wav";
    let mut path = format!("@L@\0Online Content/{sample}").into_bytes();
    path.resize(1024, 0);
    path.extend([60, 0, 127, 0, 127]);
    for word in [1.0f32.to_bits(), 0.5f32.to_bits(), 2, 100, 30_000] {
        path.extend(word.to_le_bytes());
    }
    for value in [0.0f32, 1.0, 1.0, 0.1] {
        path.extend(value.to_le_bytes());
    }
    let cstm = [&b"10WDZONE"[..], &(path.len() as u32).to_le_bytes(), &path].concat();
    let mut preset = ModuleInstance::new(1, 2);
    preset.file.custom_data = Some(cstm);
    let exp = tmp.0.join(CONTENT_FOLDER).join("DirectWave Samples EXP");
    let dw = crate::flm::write_preset(&preset).unwrap();
    put(&exp.join("Essential Pianos EXP/Close Grand.flmpst"), &dw);
    if zone {
        put(
            &exp.join(sample),
            &crate::sample_cache::tests::wav(&[0, 900, 0]),
        );
    }
    let mut rig = Rig::with_synth();
    rig.app.files = ProjectFiles::on_disk(&tmp.0);
    rig.app.files.saved();
    (rig, tmp)
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_new_instrument_loads_close_grand_from_the_content_and_a_later_pick_renames_it() {
    let (mut rig, _tmp) = close_grand_rig(true);
    let steps = rig.app.history.steps();
    rig.app.add_channel(ChannelKind::Instrument, 0);
    rig.frame(vec![]);
    // As a tap's pick: the audition strip is on.
    assert!(rig.app.slots.preset_picker.auditioning());
    worker_done(&mut rig);
    // Loaded as its pick loads: a DW Sampler in the SuperSaw's place, the channel named by the pick's rule.
    let ch = &rig.app.song.channels[2];
    assert_eq!(types(&rig, 2), [1]);
    assert_eq!(ch.rack.modules[0].file.preset_name, "Close Grand");
    assert_eq!(ch.name, "Close Grand");
    // Its zone decoded, from the content folder's `DirectWave Samples EXP`.
    assert_eq!(rig.app.audio.samples.missing().count(), 0);
    assert_eq!(rig.overlays(), [picker(2, PresetTarget::Instruments)]);
    assert_eq!(rig.app.nav.view, View::PianoRoll(2));
    assert_eq!(rig.app.history.steps(), steps + 1, "no step of its own");
    assert!(rig.app.files.modified());
    // Closing the picker keeps it.
    rig.frame(escape());
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    assert_eq!(types(&rig, 2), [1]);
    assert_eq!(rig.app.song.channels[2].name, "Close Grand");
    // A later pick renames it by the same rule: Pluck › `Ice Pluck`, ours.
    rig.app
        .navigate(Nav::Open(picker(2, PresetTarget::Instruments)));
    click_row(&mut rig, PLUCK);
    click_row(&mut rig, 0);
    rig.frame(vec![]);
    assert_eq!(types(&rig, 2), [23]);
    assert_eq!(rig.app.song.channels[2].name, "Ice Pluck");
}

/// A Close Grand none of whose zones decode would be a silent channel: the SuperSaw stays, and so does the name.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_close_grand_whose_zones_do_not_decode_leaves_the_supersaw_and_the_name() {
    let (mut rig, _tmp) = close_grand_rig(false);
    rig.app.add_channel(ChannelKind::Instrument, 0);
    rig.frame(vec![]);
    worker_done(&mut rig);
    assert_eq!(types(&rig, 2), [23]);
    assert_eq!(rig.app.song.channels[2].name, "Instrument");
    assert_eq!(rig.overlays(), [picker(2, PresetTarget::Instruments)]);
}

// ---- Every new DW Sampler loads Close Grand ----

/// The Modules browser's DW Sampler row ([`super::rack_wiring::browser_row`]: `Paste` 0, `Rack Preset` 1, `Instrument`
/// 2, SYNTH 3, 3x Osc 4, Lemon Trio 5, DW Sampler 6).
const DW_ROW: usize = 6;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_new_dw_sampler_channel_loads_close_grand_in_its_undo_step_named_by_the_channel_name_rule() {
    let (mut rig, _tmp) = close_grand_rig(true);
    let steps = rig.app.history.steps();
    // The add-channel popup's `Synth:` column, `DW Sampler`.
    rig.click(center(&rig, Id::new("add_channel_button")));
    rig.frame(vec![]);
    rig.click(center(
        &rig,
        Id::new("add_channel_popup").with((1_usize, 2_usize)),
    ));
    worker_done(&mut rig);
    let ch = &rig.app.song.channels[2];
    assert_eq!(types(&rig, 2), [1]);
    assert_eq!(ch.rack.modules[0].file.preset_name, "Close Grand");
    // Its name was no one's typing (`DW Sampler`): the rule gives it the preset's.
    assert_eq!(ch.name, "Close Grand");
    assert_eq!(rig.app.audio.samples.missing().count(), 0);
    // No picker; the channel's undo step holds it, and the song is changed.
    assert_eq!(rig.overlays(), []);
    assert_eq!(rig.app.history.steps(), steps + 1, "no step of its own");
    assert!(rig.app.files.modified());
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.app.song.channels.len(), 2, "one undo");
    rig.frame(z(egui::Modifiers::SHIFT));
    let module = &rig.app.song.channels[2].rack.modules[0];
    assert_eq!(module.file.preset_name, "Close Grand", "the redo keeps it");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_dw_sampler_picked_in_the_modules_browser_loads_close_grand_with_no_undo_step() {
    for (place, expected) in [
        ("+", vec![0, 1]),
        ("above", vec![1, 0]),
        ("replace", vec![1]),
    ] {
        let (mut rig, _tmp) = close_grand_rig(true);
        let synth = rig.app.song.channels[1].rack.modules[0].instance_id;
        let at = match place {
            "+" => InsertAt::End,
            "above" => InsertAt::Above(synth),
            _ => InsertAt::Replace(synth),
        };
        let steps = rig.app.history.steps();
        rig.open_rack(1);
        rig.slide_rack();
        let browser = Overlay::ModulesBrowser { channel: 1, at };
        rig.app.navigate(Nav::Open(browser.clone()));
        let row = super::rack_wiring::browser_row(&mut rig, DW_ROW).unwrap();
        rig.click(row.center());
        worker_done(&mut rig);
        assert_eq!(types(&rig, 1), expected, "{place}");
        let dw = expected.iter().position(|&t| t == 1).unwrap();
        let module = &rig.app.song.channels[1].rack.modules[dw];
        assert_eq!(module.file.preset_name, "Close Grand", "{place}");
        assert_eq!(rig.app.song.channels[1].name, "Close Grand", "{place}");
        // As the pick: no undo step, the song changed, the browser still open.
        assert_eq!(rig.app.history.steps(), steps, "{place}");
        assert!(rig.app.files.modified(), "{place}");
        assert_eq!(rig.overlays(), [Overlay::Rack(1), browser], "{place}");
    }
}

/// [`close_grand_rig`] with the MiniSynth's `Replace` by a DW Sampler held for `Delete automation?` (a lane targets the
/// MiniSynth), its Close Grand loaded meanwhile.
#[cfg(not(target_arch = "wasm32"))]
fn held_replace_rig() -> (Rig, crate::project_files::content::tests::Temp) {
    let (mut rig, tmp) = close_grand_rig(true);
    let synth = rig.app.song.channels[1].rack.modules[0].instance_id;
    let target = AutomationTarget {
        instance_id: synth,
        param: 1,
    };
    rig.app.song.channels[1].add_automation_lane(target);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.open_rack(1);
    rig.slide_rack();
    let at = InsertAt::Replace(synth);
    rig.app
        .navigate(Nav::Open(Overlay::ModulesBrowser { channel: 1, at }));
    let row = super::rack_wiring::browser_row(&mut rig, DW_ROW).unwrap();
    rig.click(row.center());
    // It lands while the question holds the module back.
    crate::ui::views::preset_picker::join_workers();
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    assert_eq!(types(&rig, 1), [0], "kept until the answer");
    (rig, tmp)
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_dw_sampler_replace_held_for_delete_automation_loads_close_grand_once_answered() {
    let (mut rig, _tmp) = held_replace_rig();
    assert!(rig.app.slots.preset_picker.loading(), "it waits");
    let at = rig.text_rect("Keep").center();
    rig.click(at);
    worker_done(&mut rig);
    assert_eq!(types(&rig, 1), [1]);
    let module = &rig.app.song.channels[1].rack.modules[0];
    assert_eq!(module.file.preset_name, "Close Grand");
}

/// A held `Replace` never made (its channel changed under the question) drops its Close Grand.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_held_replace_never_made_drops_its_close_grand() {
    let (mut rig, _tmp) = held_replace_rig();
    rig.app.song.channels[1].name = "Renamed".into();
    rig.app.song_edited(SongEdit::Final, 0);
    let at = rig.text_rect("Keep").center();
    rig.click(at);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(types(&rig, 1), [0]);
    assert!(!rig.app.slots.preset_picker.loading());
}

/// Nothing lands while a pointer is down, so a drag's edits stay one undo step.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn close_grand_lands_once_the_pointer_is_up() {
    let (mut rig, _tmp) = close_grand_rig(true);
    let hold = crate::ui::views::preset_picker::WORKER_HOLD.lock();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::DwSampler), 0);
    rig.press(OUTSIDE);
    drop(hold);
    crate::ui::views::preset_picker::join_workers();
    rig.frame(vec![]);
    rig.frame(vec![]);
    let preset = |rig: &Rig| {
        rig.app.song.channels[2].rack.modules[0]
            .file
            .preset_name
            .clone()
    };
    assert_eq!(preset(&rig), "Default");
    rig.release(OUTSIDE);
    worker_done(&mut rig);
    assert_eq!(preset(&rig), "Close Grand");
}

/// `Default` picked on the new card's picker while Close Grand loads stays.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_default_pick_on_the_new_card_while_close_grand_loads_stays() {
    let (mut rig, _tmp) = close_grand_rig(true);
    let hold = crate::ui::views::preset_picker::WORKER_HOLD.lock();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::DwSampler), 0);
    rig.frame(vec![]);
    let id = rig.app.song.channels[2].rack.modules[0].instance_id;
    rig.app
        .navigate(Nav::Open(picker(2, PresetTarget::Module(id))));
    // At the root: `My Files`, then `Default`.
    click_row(&mut rig, 1);
    drop(hold);
    worker_done(&mut rig);
    let ch = &rig.app.song.channels[2];
    assert_eq!(ch.rack.modules[0].file.preset_name, "Default");
    assert_eq!(ch.name, "DW Sampler");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_new_dw_sampler_keeps_default_without_close_grand_or_when_it_is_silent_or_already_changed() {
    let dw = ChannelKind::Synth(SynthKind::DwSampler);
    // No content.
    let mut rig = Rig::with_synth();
    rig.app.add_channel(dw, 0);
    rig.frame(vec![]);
    assert!(!rig.app.slots.preset_picker.loading());
    assert_eq!(
        rig.app.song.channels[2].rack.modules[0].file.preset_name,
        "Default"
    );
    // Close Grand with no zone that decodes.
    let (mut rig, _tmp) = close_grand_rig(false);
    rig.app.add_channel(dw, 0);
    worker_done(&mut rig);
    let ch = &rig.app.song.channels[2];
    assert_eq!(ch.rack.modules[0].file.preset_name, "Default");
    assert_eq!(ch.name, "DW Sampler");
    // A knob moved on the new card meanwhile stays.
    let (mut rig, _tmp) = close_grand_rig(true);
    let hold = crate::ui::views::preset_picker::WORKER_HOLD.lock();
    rig.app.add_channel(dw, 0);
    rig.frame(vec![]);
    rig.app.song.channels[2].rack.modules[0].params[0] = 0.9;
    drop(hold);
    worker_done(&mut rig);
    let module = &rig.app.song.channels[2].rack.modules[0];
    assert_eq!(module.params[0], 0.9);
    assert_eq!(module.file.preset_name, "Default");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_pasted_dw_sampler_keeps_its_copied_settings() {
    let (mut rig, _tmp) = close_grand_rig(true);
    rig.app.slots.clipboard = vec![ModuleInstance::new(1, 9)];
    rig.open_rack(1);
    rig.slide_rack();
    let at = InsertAt::End;
    rig.app
        .navigate(Nav::Open(Overlay::ModulesBrowser { channel: 1, at }));
    let paste = super::rack_wiring::browser_row(&mut rig, 0).unwrap();
    rig.click(paste.center());
    assert!(!rig.app.slots.preset_picker.loading());
    assert_eq!(types(&rig, 1), [0, 1]);
    let module = &rig.app.song.channels[1].rack.modules[1];
    assert_eq!(module.file.preset_name, "Default");
}

/// A new Drums channel added while the song position is past the first bar gets its clip at the song start; the
/// transport wraps at the song end, so a hit on its first step sounds within two bars. The song is two bars long (the
/// synth's clip), so the position is past bar 1 without a wrap first. No content: the built-in `Default` kit.
#[test]
fn a_new_drums_channel_added_past_the_first_bar_sounds_where_the_song_plays() {
    use honeydew_audio::device::AudioRenderer;
    let mut rig = Rig::with_synth();
    rig.app.song.channels[1].lanes[0].add_clip(0, 8.0);
    // The Recorder's engine (the device has one from its open), playing for three seconds: past bar 1 at 120 BPM.
    rig.app.rebuild_as(0, crate::audio_link::Handover::Edit);
    rig.click(PLAY);
    let render = |rig: &mut Rig, blocks: usize| {
        let mut peak = 0.0f32;
        for _ in 0..blocks {
            let mut out = vec![0.0; 512 * 2];
            let mut rec = rig.rec.0.borrow_mut();
            let engine = rec.engine.as_mut().unwrap();
            engine.render(&mut out, 2);
            rec.tick = engine.transport_tick();
            drop(rec);
            rig.frame(vec![]);
            peak = out.iter().fold(peak, |p, s| p.max(s.abs()));
        }
        peak
    };
    render(&mut rig, 3 * 48_000 / 512);
    let tick = rig.rec.0.borrow().tick;
    assert!(tick > 4 * u64::from(crate::song::PPQ), "past bar 1: {tick}");
    rig.app.add_channel(ChannelKind::Drums, tick);
    rig.frame(vec![]);
    super::rack_wiring::picker_row(&mut rig, 1);
    rig.slide_split();
    let at = rig.step(0, 0);
    rig.click(at);
    let peak = render(&mut rig, 2 * 2 * 48_000 / 512);
    assert!(peak > 0.01, "the hit plays within two bars: peak {peak}");
}

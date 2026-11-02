//! Live editing in a Playground session (`app/live.rs`): a remote change waits for a quiet frame and keeps this
//! person's unsent work and views; while connected, Undo and Redo go to the session.

use super::*;
use crate::app::live;
use crate::collaboration::{Conflict, checkpoint};
use crate::song::points::{self, PointKey};
use std::{cell::RefCell, rc::Rc};

/// Disconnects when the test ends: the connection is the thread's, and a test thread may run others after it.
struct Session;

impl Drop for Session {
    fn drop(&mut self) {
        live::disconnect();
    }
}

fn connected() -> (Rig, Vec<u8>, Session) {
    let mut rig = Rig::with_synth();
    live::connect();
    rig.frame(vec![]);
    let base = checkpoint::encode(&rig.app.song).unwrap();
    (rig, base, Session)
}

type Done = Rc<RefCell<Option<Result<Option<Vec<u8>>, Conflict>>>>;

fn queue(base: &[u8], edited: &Song) -> Done {
    let done = Done::default();
    let to = Rc::clone(&done);
    let edited = checkpoint::encode(edited).unwrap();
    live::queue(
        Some(base.to_vec()),
        edited,
        Box::new(move |result| *to.borrow_mut() = Some(result)),
    );
    done
}

#[test]
fn a_remote_change_keeps_local_work_and_views() {
    let (mut rig, base, _session) = connected();
    let mut remote = checkpoint::decode(&base).unwrap();
    remote.tempo_bpm = 99.0;
    rig.app.song.channels[1].name = "Mine".into();
    rig.app.main_screen.zoom = 0.7;
    let done = queue(&base, &remote);
    rig.frame(vec![]);
    assert!(matches!(*done.borrow(), Some(Ok(None))));
    assert_eq!(rig.app.song.tempo_bpm, 99.0);
    assert_eq!(rig.app.song.channels[1].name, "Mine");
    assert_eq!(rig.app.main_screen.zoom, 0.7);
    assert_eq!(rig.app.selected_channel, 1);
}

#[test]
fn a_remote_change_waits_for_the_gesture_to_end() {
    let (mut rig, base, _session) = connected();
    let mut remote = checkpoint::decode(&base).unwrap();
    remote.tempo_bpm = 99.0;
    rig.press(OUTSIDE);
    let done = queue(&base, &remote);
    rig.frame(vec![]);
    assert!(done.borrow().is_none());
    assert_ne!(rig.app.song.tempo_bpm, 99.0);
    let reported = live::changes();
    assert_eq!(
        rig.app.live_checkpoint(),
        Err(Conflict("busy".into())),
        "no half-done gesture is read"
    );
    rig.release(OUTSIDE);
    rig.frame(vec![]);
    assert!(matches!(*done.borrow(), Some(Ok(None))));
    assert_eq!(rig.app.song.tempo_bpm, 99.0);
    assert_eq!(
        live::changes(),
        reported + 1,
        "the quiet frame after a busy checkpoint reports again"
    );
}

#[test]
fn a_conflicting_remote_change_leaves_the_song_as_it_is() {
    let (mut rig, base, _session) = connected();
    let mut remote = checkpoint::decode(&base).unwrap();
    remote.tempo_bpm = 99.0;
    rig.app.song.tempo_bpm = 101.0;
    let before = rig.app.song.clone();
    let done = queue(&base, &remote);
    rig.frame(vec![]);
    assert!(matches!(*done.borrow(), Some(Err(_))));
    assert_eq!(rig.app.song, before);
}

#[test]
fn another_persons_edit_to_the_clip_keeps_the_note_selection() {
    let (mut rig, _, _session) = connected();
    let note = |pos, key| crate::song::Event::Note {
        pos,
        len_beats: 1.0,
        key,
        velocity: 204,
        pan: 127,
        slide: false,
    };
    let chord = vec![note(0, 60), note(0, 64), note(0, 67)];
    rig.app.song.channels[1].lanes[0].add_clip(0, 4.0).events = chord;
    let (channel, lane, id) = (1, 0, 1);
    rig.app
        .navigate(Nav::EditClip(ClipKey { channel, lane, id }));
    rig.slide_split();
    rig.frame(key_tap(egui::Key::A, egui::Modifiers::COMMAND));
    rig.frame(vec![]);
    let selected =
        |rig: &Rig| serde_json::to_value(rig.app.live_presence()).unwrap()["notes"].clone();
    assert_eq!(
        selected(&rig),
        serde_json::json!([[0, 60], [0, 64], [0, 67]])
    );
    // Alice adds a note to the clip: Bob's chord stays selected. She then moves its E: that note drops out.
    let mut base = checkpoint::encode(&rig.app.song).unwrap();
    for (change, left) in [
        (None, serde_json::json!([[0, 60], [0, 64], [0, 67]])),
        (Some(note(0, 65)), serde_json::json!([[0, 60], [0, 67]])),
    ] {
        let mut alice = checkpoint::decode(&base).unwrap();
        let events = &mut alice.channels[1].lanes[0].clips[0].events;
        match change {
            None => events.push(note(256, 72)),
            Some(moved) => events[1] = moved,
        }
        let edited = checkpoint::encode(&alice).unwrap();
        apply(&mut rig, &base, &edited);
        rig.frame(vec![]);
        assert_eq!(rig.app.song.channels[1].lanes[0].clips[0].events.len(), 4);
        assert_eq!(selected(&rig), left);
        base = edited;
    }
}

#[test]
fn only_shared_changes_are_reported_and_undo_goes_to_the_session() {
    let (mut rig, _, _session) = connected();
    let reported = live::changes();
    rig.app.main_screen.zoom = 0.3;
    rig.app.song.file.playhead_beats += 4.0;
    rig.frame(vec![]);
    assert_eq!(live::changes(), reported, "views are not shared");
    let tempo = rig.app.song.tempo_bpm;
    rig.app.song.tempo_bpm = tempo + 10.0;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    assert_eq!(live::changes(), reported + 1);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(
        rig.app.song.tempo_bpm,
        tempo + 10.0,
        "the session reverts only this person's operations"
    );
    assert_eq!(live::undos(), [false]);
}

#[test]
fn user_samples_are_listed_and_arrive_without_a_song_change() {
    use crate::flm::chunk::{ContentPath, ContentRoot};
    use crate::sample_cache::{SampleKey, tests::wav};
    let (mut rig, _, _session) = connected();
    rig.app.add_default_drums(0);
    let id = "My Samples/Kick.wav";
    let path = |root, relative: &str| {
        Some(ContentPath::Shorthand {
            root,
            relative: relative.into(),
        })
    };
    let slots = &mut rig.app.song.channels[2].drum_kit.as_mut().unwrap().slots;
    slots[0].file.path = path(ContentRoot::User, id);
    slots[1].file.path = path(ContentRoot::App, "Drum Samples/Snare.wav");
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let key = SampleKey::File(path(ContentRoot::User, id).unwrap());
    assert!(rig.app.audio.samples.missing().any(|k| *k == key));
    // Only the user's file: no built-in or content-folder sample.
    assert_eq!(
        rig.app.live_assets(),
        [(id.into(), "Kick.wav".into(), false)]
    );
    let shared = checkpoint::encode(&rig.app.song).unwrap();
    assert_eq!(live::assets_of(&shared).unwrap(), [id]);

    let (song, steps, reported) = (
        rig.app.song.clone(),
        rig.app.history.steps(),
        live::changes(),
    );
    rig.app.live_write_asset(id, wav(&[1, 2, 3]));
    rig.app.live_write_asset(id, wav(&[9]));
    rig.frame(vec![]);
    assert_eq!(
        rig.app.live_read_asset(id),
        Some(wav(&[1, 2, 3])),
        "never overwritten"
    );
    assert!(rig.app.live_assets()[0].2);
    assert!(
        rig.app.audio.samples.get(&key).is_some(),
        "decoded, no longer missing"
    );
    assert_eq!(rig.app.song, song);
    assert_eq!(
        (rig.app.history.steps(), live::changes()),
        (steps, reported)
    );
}

/// Settles a remote change on `rig` the way the session sends it: `edited` relative to `base`.
fn apply(rig: &mut Rig, base: &[u8], edited: &[u8]) {
    let done = Done::default();
    let to = Rc::clone(&done);
    let (base, edited) = (base.to_vec(), edited.to_vec());
    live::queue(
        Some(base),
        edited,
        Box::new(move |r| *to.borrow_mut() = Some(r)),
    );
    rig.frame(vec![]);
    assert!(
        matches!(*done.borrow(), Some(Ok(None))),
        "{:?}",
        done.borrow()
    );
}

#[test]
fn a_channel_added_on_each_side_keeps_the_shared_rack_order() {
    // Bob's new channel has no rack place yet in his live song (the format settles it); Alice's later one must not
    // take its place when Bob merges it, or his song differs from the session's and he proposes it again and again.
    let (mut alice, mut bob) = (Rig::new(), Rig::new());
    live::connect();
    let _session = Session;
    let start = checkpoint::encode(&bob.app.song).unwrap();
    bob.app
        .add_channel(ChannelKind::Synth(crate::song::SynthKind::MiniSynth), 0);
    bob.frame(vec![]);
    let bobs = checkpoint::encode(&bob.app.song).unwrap();
    apply(&mut alice, &start, &bobs);
    alice.app.add_channel(ChannelKind::AudioClip, 0);
    alice.frame(vec![]);
    let alices = checkpoint::encode(&alice.app.song).unwrap();
    apply(&mut bob, &bobs, &alices);
    assert!(checkpoint::encode(&bob.app.song).unwrap() == alices);
}

/// The last frame's marks in `color`: arrow tips (a filled path's first point), outlines (stroked rects) and ghosts
/// (rects filled translucent).
fn marks(rig: &Rig, color: egui::Color32) -> (Vec<Pos2>, Vec<egui::Rect>, Vec<egui::Rect>) {
    let (mut tips, mut outlines, mut ghosts) = (Vec::new(), Vec::new(), Vec::new());
    for s in &rig.out.shapes {
        match &s.shape {
            egui::Shape::Path(p) if p.fill == color => tips.push(p.points[0]),
            egui::Shape::Rect(r) if r.stroke.color == color => outlines.push(r.rect),
            // A ghost: the fill, and the edge that keeps a dark colour's readable.
            egui::Shape::Rect(r)
                if r.fill == color.gamma_multiply(live::GHOST)
                    && r.stroke.color == color.gamma_multiply(live::GHOST_EDGE)
                    && r.stroke.width >= 1.5 =>
            {
                ghosts.push(r.rect)
            }
            _ => {}
        }
    }
    (tips, outlines, ghosts)
}

/// The last frame's point marks in `color`: the centres of outlined dots and ghost dots, and the strokes' points.
fn dots(rig: &Rig, color: egui::Color32) -> (Vec<Pos2>, Vec<Pos2>, Vec<Vec<Pos2>>) {
    let stroke = egui::epaint::ColorMode::Solid(color.gamma_multiply(live::STROKE));
    let (mut outlines, mut ghosts, mut strokes) = (Vec::new(), Vec::new(), Vec::new());
    for s in &rig.out.shapes {
        match &s.shape {
            egui::Shape::Circle(c) if c.stroke.color == color => outlines.push(c.center),
            egui::Shape::Circle(c) if c.fill == color.gamma_multiply(live::GHOST) => {
                ghosts.push(c.center)
            }
            egui::Shape::Path(p) if !p.closed && p.stroke.color == stroke => {
                strokes.push(p.points.clone())
            }
            _ => {}
        }
    }
    (outlines, ghosts, strokes)
}

/// The shell's list of the others: Bob alone, sharing `state`.
fn only_bob(state: serde_json::Value) -> serde_json::Value {
    serde_json::json!([{ "id": "b", "name": "Bob", "color": "#e6194b", "state": state }])
}

#[test]
fn presence_is_in_song_terms_and_shows_the_others_through_this_view() {
    let (mut rig, _, _session) = connected();
    let id = rig.app.song.channels[1].id;
    rig.app.song.channels[1].lanes[0].add_clip(0, 16.0);
    let clip = crate::ui::clips::ClipRef {
        channel: 1,
        lane: 0,
        id: 1,
    };
    rig.app.main_screen.clips.selected = Some(clip);
    // Half a U below the row's middle: the pointer is shared as that far down the row, not as the row.
    let at = lane(1) + vec2(0.0, 0.5 * U);
    rig.frame(vec![egui::Event::PointerMoved(at)]);
    let mine = serde_json::to_value(rig.app.live_presence()).unwrap();
    assert_eq!(mine["view"], "playlist");
    assert_eq!(mine["pointer"]["track"], serde_json::json!([id, 0]));
    let y = mine["pointer"]["y"].as_f64().unwrap();
    assert!((y - 0.76).abs() < 0.02, "{y}");
    assert_eq!(mine["clips"], serde_json::json!([[id, 0, 1]]));

    // The others as the shell lists them. Bob shares this state: his arrow is where this person points, his outline
    // on the clip's row. A bad colour, a tick that is no integer, unknown ids: nothing, and no panic.
    let far =
        serde_json::json!({ "view": "playlist", "pointer": { "tick": 1e300, "track": [id, 0] } });
    let gone = serde_json::json!({
        "view": "playlist",
        "pointer": { "tick": 0, "track": [id + 99, 0] },
        "clips": [[id, 7, 1], [id, 0, 99]],
    });
    live::show_presence(&serde_json::json!([
        { "id": "b", "name": "Bob", "color": "#e6194b", "state": mine },
        { "id": "c", "name": "Cy", "color": "blue", "state": mine },
        { "id": "d", "name": "Dee", "color": "#000001", "state": far },
        { "id": "e", "name": "Eve", "color": "#000002", "state": gone },
    ]));
    rig.frame(vec![]);
    let bob = egui::Color32::from_rgb(0xe6, 0x19, 0x4b);
    let (tips, outlines, _) = marks(&rig, bob);
    assert!(tips.len() == 1 && (tips[0] - at).length() < 1.0, "{tips:?}");
    assert!(
        outlines.len() == 1 && (outlines[0].top() - RULER - 2.0 * U).abs() < 1.0,
        "{outlines:?}"
    );
    for other in [1, 2] {
        let (tips, outlines, _) = marks(&rig, egui::Color32::from_rgb(0, 0, other));
        assert!(tips.is_empty() && outlines.is_empty());
    }

    // Mid-drag (held past a tap, then 2 U on, not let go), the presence still names the clip where it was and has it
    // where the drag has it so far: two beats on (snapped; the clip is 16 beats long). Bob's ghost of it shows there.
    rig.press(at);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.drag_to(at + vec2(2.0 * U, 0.0));
    let mine = serde_json::to_value(rig.app.live_presence()).unwrap();
    assert_eq!(mine["clips"], serde_json::json!([[id, 0, 1]]));
    live::show_presence(
        &serde_json::json!([{ "id": "b", "name": "Bob", "color": "#e6194b", "state": mine }]),
    );
    rig.frame(vec![]);
    let (_, _, ghosts) = marks(&rig, bob);
    let to = outlines[0].translate(vec2(outlines[0].width() / 8.0, 0.0));
    assert!(
        ghosts.len() == 1 && (ghosts[0].min - to.min).length() < 1.0,
        "{ghosts:?} {to:?} {mine}"
    );

    // The drum editor's steps on a clip with hits on steps 0 and 4 of pad 0: Bob, selecting both and dragging the first
    // to step 2, has them outlined on their cells and its ghost on step 2's, halfway between.
    let mut rig = Rig::with_drums_and_synth();
    let step = crate::song::PPQ / 4;
    let hit = |pos| crate::song::Event::Drum {
        pos,
        len_beats: 0.25,
        pad: 0,
        velocity: 200,
        pan: 127,
        value: 0x7FFF,
        repeat: 0,
    };
    rig.app.song.channels[1].lanes[0].add_clip(0, 4.0).events = vec![hit(0), hit(4 * step)];
    rig.app.navigate(Nav::Edit(1));
    rig.slide_split();
    let id = rig.app.song.channels[1].id;
    live::show_presence(&only_bob(serde_json::json!({
        "view": "drums",
        "clip": [id, 0, 1],
        "notes": [[0, 0], [4 * step, 0]],
        "drag_notes": [[2 * step, 0, f64::from(step)]],
    })));
    rig.frame(vec![]);
    let (_, outlines, ghosts) = marks(&rig, bob);
    assert_eq!(
        (outlines.len(), ghosts.len()),
        (2, 1),
        "{outlines:?} {ghosts:?}"
    );
    let half = (outlines[0].center() + outlines[1].center().to_vec2()) / 2.0;
    assert!(
        (ghosts[0].center() - half).length() < 1.0,
        "{ghosts:?} {outlines:?}"
    );

    // The automation editor (the master's clip) and the audio editor's volume envelope (an Audio Clip's), each with
    // points at beats 0 and 2: Bob, dragging the selected first one to beat 1, has it outlined, its ghost at beat 1 and
    // the curve he redraws from there to the second point as a stroke.
    let mut rig = Rig::new();
    rig.app.add_channel(ChannelKind::AudioClip, 0);
    for (channel, view, key) in [
        (0, "automation", PointKey::Automation),
        (1, "audio", PointKey::Envelope(0)),
    ] {
        let clip = rig.app.song.channels[channel].lanes[0].add_clip(0, 4.0);
        let at = |tick| points::Point {
            tick,
            value: 20000,
            ..Default::default()
        };
        points::set_points(clip, key, vec![at(0), at(2 * crate::song::PPQ)]);
        let first = points::points(clip, key)[0];
        let (lane, id) = (0, 1);
        rig.app
            .navigate(Nav::EditClip(ClipKey { channel, lane, id }));
        rig.slide_split();
        live::show_presence(&only_bob(serde_json::json!({
            "view": view,
            "clip": [rig.app.song.channels[channel].id, 0, 1],
            "points": [[first.tick, first.value]],
            "drag_points": [[crate::song::PPQ, first.value]],
            "env": (view == "audio").then_some(0),
        })));
        rig.frame(vec![]);
        let (outlines, ghosts, strokes) = dots(&rig, bob);
        assert!(
            outlines.len() == 1 && ghosts.len() == 1 && strokes.len() == 1,
            "{view} {outlines:?} {ghosts:?} {strokes:?}"
        );
        let (ghost, line) = (ghosts[0], &strokes[0]);
        let half = (outlines[0] + line[1].to_vec2()) / 2.0;
        assert!(
            line.len() == 2 && (line[0] - ghost).length() < 1.0 && (ghost - half).length() < 1.0,
            "{view} {outlines:?} {ghosts:?} {strokes:?}"
        );
    }

    // The mixer: Bob, holding the Audio Clip's fader half way, has his pointer on its strip and the level he is setting
    // tagged under his name.
    rig.app.navigate(Nav::Show(View::Mixer));
    rig.slide_split();
    let id = rig.app.song.channels[1].id;
    live::show_presence(&only_bob(serde_json::json!({
        "view": "mixer",
        "pointer": { "tick": 0.5, "track": [id, 0], "y": 0.5 },
        "param": [id, 0, 0.5],
    })));
    rig.frame(vec![]);
    let text = |t: &str| {
        (rig.out.shapes.iter()).find_map(|s| match &s.shape {
            egui::Shape::Text(x) if x.galley.text() == t => Some(x.pos),
            _ => None,
        })
    };
    let level = crate::params::CHANNEL_PARAMS[0].display_text(0.5);
    let (name, value) = (text("Bob"), text(&level));
    assert!(
        name.zip(value)
            .is_some_and(|(n, v)| v.y > n.y && (v.x - n.x).abs() < 1.0),
        "{level} {name:?} {value:?}"
    );
}

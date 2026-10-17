//! Live editing in a Playground session (`app/live.rs`): a remote change waits for a quiet frame and keeps this
//! person's unsent work and views; while connected, Undo and Redo go to the session.

use super::*;
use crate::app::live;
use crate::collaboration::{Conflict, checkpoint};
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

/// The last frame's marks in `color`: arrow tips (a filled path's first point) and outlines (stroked rects).
fn marks(rig: &Rig, color: egui::Color32) -> (Vec<Pos2>, Vec<egui::Rect>) {
    let (mut tips, mut outlines) = (Vec::new(), Vec::new());
    for s in &rig.out.shapes {
        match &s.shape {
            egui::Shape::Path(p) if p.fill == color => tips.push(p.points[0]),
            egui::Shape::Rect(r) if r.stroke.color == color => outlines.push(r.rect),
            _ => {}
        }
    }
    (tips, outlines)
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
    let at = lane(1);
    rig.frame(vec![egui::Event::PointerMoved(at)]);
    let mine = serde_json::to_value(rig.app.live_presence()).unwrap();
    assert_eq!(mine["view"], "playlist");
    assert_eq!(mine["pointer"]["track"], serde_json::json!([id, 0]));
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
    let (tips, outlines) = marks(&rig, egui::Color32::from_rgb(0xe6, 0x19, 0x4b));
    assert!(tips.len() == 1 && (tips[0] - at).length() < 1.0, "{tips:?}");
    assert!(
        outlines.len() == 1 && (outlines[0].top() - 3.5 * U).abs() < 1.0,
        "{outlines:?}"
    );
    for other in [1, 2] {
        let (tips, outlines) = marks(&rig, egui::Color32::from_rgb(0, 0, other));
        assert!(tips.is_empty() && outlines.is_empty());
    }
}

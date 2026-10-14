//! A song load on the web waits for its bundled sounds (`app/bundle_wait.rs`).

use super::*;
use crate::app::bundle_wait::RETRYING;
use crate::content::Factory;
use crate::content::bundle::{Bundle, Kind};
use crate::content::web::tests::{advance, bundle, fake, fetched, pack};
use crate::content::web::{self, Outcome};
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::sample_cache::tests::{dw_cstm, dwp, record, wav};

/// A rig whose factory is the web's `bundle` alone, its audio started.
fn rig_with(bundle: Bundle) -> Rig {
    let mut rig = Rig::new();
    let factory = Factory::new(Vec::new()).with_bundle(bundle);
    rig.app.files.factory = Box::leak(Box::new(factory));
    rig.click(OUTSIDE);
    rig
}

/// Opens `song`, as a load does.
fn load(rig: &mut Rig, song: Song) {
    rig.app.song = song;
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.frame(vec![]);
}

fn busy(rig: &Rig) -> Option<String> {
    rig.overlays().into_iter().find_map(|o| match o {
        Overlay::Busy { title } => Some(title),
        _ => None,
    })
}

fn ours(relative: &str) -> ContentPath {
    ContentPath::Shorthand {
        root: ContentRoot::App,
        relative: relative.into(),
    }
}

#[test]
fn a_failing_part_is_tried_after_1_3_and_9_s_then_given_up_and_the_song_says_missing_once() {
    fake(u64::MAX, "");
    let (kicks, _) = pack(
        Kind::Samples,
        "Samples/Kick/",
        None,
        &[("K1.flac", 0, wav(&[0, 9000]))],
    );
    let mut rig = rig_with(bundle(vec![kicks]));
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let slot = &mut song.channels[1].drum_kit.as_mut().unwrap().slots[0];
    slot.file.path = Some(ours("Built-in/Samples/Kick/K1.flac"));
    load(&mut rig, song);
    assert_eq!(busy(&rig).as_deref(), Some("Downloading sounds (0/1)..."));
    for wait in [1000.0, 3000.0, 9000.0] {
        let log = fetched();
        let [(file, n, _)] = &log[..] else {
            panic!("one download: {log:?}")
        };
        web::landed(file, *n, Outcome::Failed);
        advance(0.8 * wait - 1.0);
        rig.frame(vec![]);
        assert_eq!(busy(&rig).as_deref(), Some(RETRYING));
        assert_eq!(fetched(), [], "not before {wait} ms less 20%");
        advance(0.4 * wait + 1.0);
        rig.frame(vec![]);
    }
    let log = fetched();
    let [(file, n, true)] = &log[..] else {
        panic!("the last try, from the network: {log:?}")
    };
    web::landed(file, *n, Outcome::Failed);
    rig.frame(vec![]);
    rig.frame(vec![]);
    let missing = Overlay::Message {
        tag: MISSING_TAG,
        lines: vec![MISSING_AUDIO.into()],
        confirm: None,
        extra: None,
    };
    assert_eq!(rig.overlays(), [missing]);
    rig.app.navigate(Nav::Close);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    assert_eq!(fetched(), []);
}

#[test]
fn a_song_plays_a_lite_program_once_part_0_is_in_then_upgrades_to_the_full_one() {
    fake(u64::MAX, "");
    let zone = |name: &[u8]| record(0x1F6, name);
    let tone = wav(&[0, 9000, -9000, 0]);
    let members = [
        ("Grand.dwp", 0, dwp(&[zone(b"C4.wav"), zone(b"C5.wav")])),
        ("Grand/lite.dwp", 0, dwp(&[zone(b"C4.wav")])),
        ("Grand/C4.wav", 0, tone.clone()),
        ("Grand/C5.wav", 1, tone),
    ];
    let path = "Presets/DW Sampler/Piano/Grand.dwp";
    let (grand, bodies) = pack(Kind::Program, path, Some("Grand/lite.dwp"), &members);
    let mut rig = rig_with(bundle(vec![grand]));
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::DwSampler));
    let cstm = dw_cstm(format!("@R@\0Built-in/{path}").as_bytes());
    song.channels[1].rack.modules[0].file.custom_data = Some(cstm);
    load(&mut rig, song);
    assert_eq!(busy(&rig).as_deref(), Some("Downloading sounds (0/1)..."));
    let log = fetched();
    let files: Vec<&str> = log.iter().map(|(file, ..)| file.as_str()).collect();
    assert_eq!(files, ["p/grand0.tar", "p/grand1.tar"], "part 0 first");
    let program = ours(&format!("Built-in/{path}"));
    let zones = |rig: &Rig| rig.app.audio.samples.program(&program).map(|p| p.len());
    web::landed(&log[0].0, log[0].1, Outcome::Body(bodies[0].clone()));
    rig.frame(vec![]);
    rig.frame(vec![]);
    // The lite program plays: the loading screen is gone, and nothing reads as missing.
    assert_eq!(rig.overlays(), []);
    assert_eq!(zones(&rig), Some(1));
    web::landed(&log[1].0, log[1].1, Outcome::Body(bodies[1].clone()));
    rig.frame(vec![]);
    assert_eq!(zones(&rig), Some(2));
    assert_eq!(rig.app.audio.samples.missing().count(), 0);
}

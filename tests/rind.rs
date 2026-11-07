//! Rind (`crates/rind`), Honeydew's own built-in synthesizer. Milestone 1 (editor spike): it is registered as a CLAP
//! instrument, and its embedded editor (`os_plugkit::embed`) paints, takes pointer input as the rack card drives it,
//! resets on a middle click and wakes from parking. No audio device.

use honeydew::plugins::{self, builtin};
use honeydew::song::plugin::PLUGIN_INSTRUMENT;
use honeydew::song::{ChannelKind, Song};
use os_plugkit::embed::{self, Button, Input};

const NONE: (bool, bool, bool) = (false, false, false);

/// A song with Rind on an instrument channel: its channel id, module id and the module's param index of each spike
/// param (knob, fader, choice).
fn rind_song() -> (Song, i32, i32, [i32; 3]) {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let uid = builtin::all()
        .iter()
        .find(|b| b.name == "Rind")
        .expect("Rind is built in")
        .uid;
    let ch = &mut song.channels[1];
    let id = ch.id;
    let module = plugins::add(&mut ch.rack, id, uid, 0).unwrap();
    let m = ch.rack.get(module).unwrap();
    assert_eq!(m.type_id, PLUGIN_INSTRUMENT);
    let params = &m.file.plugin.as_ref().unwrap().params;
    let at = |name: &str| 1 + params.iter().position(|p| p.name == name).unwrap() as i32;
    let ids = [at("Spike Knob"), at("Spike Fader"), at("Spike Choice")];
    plugins::sync(&song);
    (song, id, module, ids)
}

#[test]
fn rind_is_a_built_in_clap_instrument_with_its_permanent_id() {
    let rind = builtin::all().iter().find(|b| b.name == "Rind").unwrap();
    assert_eq!(rind.uid, "clap:org.honeydew.rind");
    assert!(rind.instrument());
    assert_eq!(rind.clap_id(), rind::CLAP_ID);
}

#[test]
fn the_editor_paints_a_first_frame_and_does_not_repaint_on_its_own() {
    let (_song, id, module, _) = rind_song();
    let editor = plugins::lemon_editor(id, module, 1.0).expect("the editor opens");
    embed::pump(editor);
    let mut size = (0, 0);
    let mut distinct = std::collections::HashSet::new();
    let version = embed::frame(editor, 0, |f| {
        size = (f.width, f.height);
        distinct.extend(f.pixels.iter().copied());
    })
    .expect("a first frame");
    assert_eq!(size, (rind::view::W, rind::view::H));
    assert!(
        distinct.len() > 3,
        "a picture, not a blank: {}",
        distinct.len()
    );
    // Nothing happened: no new frame.
    embed::pump(editor);
    assert_eq!(
        embed::frame(editor, version, |_| {}),
        None,
        "no spurious repaint"
    );
}

#[test]
fn the_editor_takes_drags_resets_and_clicks() {
    let (_song, id, module, [knob, fader, choice]) = rind_song();
    let editor = plugins::lemon_editor(id, module, 1.0).expect("the editor opens");
    let run = |e: &[Input]| {
        for i in e {
            embed::input(editor, i.clone(), NONE);
        }
        embed::pump(editor);
    };
    let moved = |param: i32| {
        let moves = plugins::idle();
        (moves.iter().rev())
            .find(|m| m.param == param && m.gesture == 1)
            .map(|m| m.value)
    };
    let (kx, ky) = rind::view::knob_center();
    let down = |x: f32, y: f32, btn| Input::Down {
        x,
        y,
        btn,
        dbl: false,
    };

    // A knob drag holds the cursor; up 30 px of 150 is 0.5 + 0.2.
    run(&[down(kx, ky, Button::Left)]);
    assert!(embed::locked(editor), "a knob drag holds the cursor");
    run(&[
        Input::Delta { dx: 0.0, dy: -30.0 },
        Input::Up { btn: Button::Left },
    ]);
    assert!(!embed::locked(editor));
    let dragged = moved(knob).expect("the drag moved the knob");
    assert!((dragged - 0.7).abs() < 1e-3, "{dragged}");

    // A middle click resets it to its default, 0.5.
    run(&[
        down(kx, ky, Button::Middle),
        Input::Up {
            btn: Button::Middle,
        },
    ]);
    let reset = moved(knob).expect("the reset moved the knob");
    assert!((reset - 0.5).abs() < 1e-3, "{reset}");

    // The fader's handle, dragged down, then reset to 1.0.
    let f = rind::view::fader_rect();
    let (fx, fy) = (f.x + f.w / 2.0, f.y + f.h / 2.0);
    run(&[down(fx, fy, Button::Left)]);
    run(&[
        Input::Delta { dx: 0.0, dy: 40.0 },
        Input::Up { btn: Button::Left },
    ]);
    let lowered = moved(fader).expect("the drag moved the fader");
    assert!(lowered < 1.0, "{lowered}");
    run(&[
        down(fx, fy, Button::Middle),
        Input::Up {
            btn: Button::Middle,
        },
    ]);
    let reset = moved(fader).expect("the reset moved the fader");
    assert!((reset - 1.0).abs() < 1e-3, "{reset}");

    // A click on the choice's third cell (value 2 of 0..=3).
    let c = rind::view::choice_rect(2);
    run(&[
        down(c.x + c.w / 2.0, c.y + c.h / 2.0, Button::Left),
        Input::Up { btn: Button::Left },
    ]);
    let picked = moved(choice).expect("the click picked a value");
    assert!((picked - 2.0 / 3.0).abs() < 1e-3, "{picked}");
}

#[test]
fn a_parked_editor_wakes_and_repaints_on_expose() {
    let (_song, id, module, _) = rind_song();
    let editor = plugins::lemon_editor(id, module, 1.0).expect("the editor opens");
    embed::pump(editor);
    let version = embed::frame(editor, 0, |_| {}).expect("a first frame");
    embed::input(editor, Input::Expose, NONE);
    embed::pump(editor);
    assert!(
        embed::frame(editor, version, |_| {}).is_some(),
        "an expose paints again"
    );
}

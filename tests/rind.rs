//! Rind (`crates/rind`), Honeydew's own built-in synthesizer: registered as a CLAP instrument with its permanent id, and
//! its embedded editor (`os_plugkit::embed`) paints, takes pointer input as the rack card drives it, switches pages,
//! resets on a middle click and wakes from parking. No audio device.

use honeydew::plugins::{self, builtin};
use honeydew::song::plugin::PLUGIN_INSTRUMENT;
use honeydew::song::{ChannelKind, Song};
use os_plugkit::embed::{self, Button, Input};
use rind::ids::{self, SPECS};
use rind::view::{self, Hit, Kind, Ui};

const NONE: (bool, bool, bool) = (false, false, false);

/// A song with Rind on an instrument channel: its channel id and module id.
fn rind_song() -> (Song, i32, i32) {
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
    plugins::sync(&song);
    (song, id, module)
}

/// The module param index of table parameter `p` (`params[0]` is the module's Enable).
fn module_param(p: usize) -> i32 {
    1 + p as i32
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
    let (_song, id, module) = rind_song();
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
fn the_editor_takes_drags_resets_taps_and_switches_pages() {
    let (_song, id, module) = rind_song();
    let editor = plugins::lemon_editor(id, module, 1.0).expect("the editor opens");
    let run = |e: &[Input]| {
        for i in e {
            embed::input(editor, i.clone(), NONE);
        }
        embed::pump(editor);
    };
    let moved = |param: usize| {
        let moves = plugins::idle();
        (moves.iter().rev())
            .find(|m| m.param == module_param(param) && m.gesture == 1)
            .map(|m| m.value)
    };
    let down = |h: &Hit, btn| Input::Down {
        x: h.r.cx(),
        y: h.r.cy(),
        btn,
        dbl: false,
    };
    let find = |ui: &Ui, p: usize| -> Hit {
        view::layout(ui)
            .into_iter()
            .find(|h| h.param == p && !matches!(h.kind, Kind::Tab(_) | Kind::Sub(_)))
            .expect("a control")
    };

    // Open the MAIN page by its tab.
    let tab = |i: usize| -> Hit {
        view::layout(&Ui::default())
            .into_iter()
            .find(|h| h.kind == Kind::Tab(i))
            .unwrap()
    };
    run(&[
        down(&tab(view::TAB_MAIN), Button::Left),
        Input::Up { btn: Button::Left },
    ]);
    let ui = Ui {
        tab: view::TAB_MAIN,
        ..Ui::default()
    };

    // A knob drag holds the cursor; up 30 px of 150 is the default 0.7 plus 0.2.
    let vol = find(&ui, ids::MASTER_VOL);
    run(&[down(&vol, Button::Left)]);
    assert!(embed::locked(editor), "a knob drag holds the cursor");
    run(&[
        Input::Delta { dx: 0.0, dy: -30.0 },
        Input::Up { btn: Button::Left },
    ]);
    assert!(!embed::locked(editor));
    let dragged = moved(ids::MASTER_VOL).expect("the drag moved the knob");
    assert!((dragged - 0.9).abs() < 1e-3, "{dragged}");

    // A middle click resets it to its default.
    run(&[
        down(&vol, Button::Middle),
        Input::Up {
            btn: Button::Middle,
        },
    ]);
    let reset = moved(ids::MASTER_VOL).expect("the reset moved the knob");
    assert!((reset - 0.7).abs() < 1e-3, "{reset}");

    // A tap on the arpeggiator's switch, on its own page.
    run(&[
        down(&tab(view::TAB_ARP), Button::Left),
        Input::Up { btn: Button::Left },
    ]);
    let ui = Ui {
        tab: view::TAB_ARP,
        ..Ui::default()
    };
    let on = find(&ui, ids::ARP_ON);
    run(&[down(&on, Button::Left), Input::Up { btn: Button::Left }]);
    assert_eq!(moved(ids::ARP_ON), Some(1.0));
    assert_eq!(SPECS[ids::ARP_ON].id, "arp_on");
}

#[test]
fn a_parked_editor_wakes_and_repaints_on_expose() {
    let (_song, id, module) = rind_song();
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

/// Rind's module in a song, mutable.
fn rind_module(song: &mut Song, module: i32) -> &mut honeydew::song::ModuleInstance {
    song.channels[1].rack.get_mut(module).expect("the module")
}

#[test]
fn a_sylenth1_program_loads_through_the_pick_and_a_native_preset_round_trips() {
    use honeydew::project_files::load_pick;
    use std::path::Path;
    let (mut song, _id, module) = rind_song();
    let before = rind_module(&mut song, module).clone();

    // A damaged file, a file of another type and a foreign module change nothing.
    for bytes in [&b"junk"[..], &[0u8; 8000][..], &[][..]] {
        let m = rind_module(&mut song, module);
        assert!(load_pick(m, Path::new("Bad.fxp"), bytes, None).is_err());
        assert_eq!(*m, before);
    }
    let fxp = rind::sylenth::build_fixture(&[("Test Pad", vec![])], false, false);
    let mut other = honeydew::song::ModuleInstance::new(6, 0);
    assert!(load_pick(&mut other, Path::new("A.fxp"), &fxp, None).is_err());

    // A program loads: its name, the category, and every parameter in range.
    let m = rind_module(&mut song, module);
    load_pick(m, Path::new("Test Pad.fxp"), &fxp, None).expect("the program loads");
    assert_eq!(m.file.preset_name, "Test Pad");
    assert_eq!(m.file.category, "Sylenth1");
    assert_eq!(m.params.len(), before.params.len());
    assert!(
        m.params.iter().all(|v| (0.0..=1.0).contains(v)),
        "{:?}",
        m.params
    );
    assert_ne!(m.params, before.params, "the values were applied");

    // A bank loads its first program.
    let bank = rind::sylenth::build_fixture(&[("First", vec![]), ("Second", vec![])], true, true);
    load_pick(m, Path::new("Bank.fxb"), &bank, None).expect("the bank loads");
    assert_eq!(m.file.preset_name, "First");

    // Saved natively and read back: the same values and still Rind's.
    let saved = honeydew::flm::write_preset(m).expect("saves");
    let mut again = before.clone();
    load_pick(&mut again, Path::new("First.flmpst"), &saved, None).expect("loads natively");
    assert_eq!(again.params, m.params);
    assert_eq!(again.file.preset_name, "First");
    assert_eq!(
        again.file.plugin.as_ref().map(|p| p.id.clone()),
        before.file.plugin.as_ref().map(|p| p.id.clone())
    );
}

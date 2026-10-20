//! Lemon Studio's own plugins built into the app (`plugins::builtin`): they play, they keep their settings in the song
//! file, and an installed copy of one is not listed. No audio device: songs render offline.

use honeydew::plugins::{self, builtin};
use honeydew::render::{Remainder, Render, RenderRange, RenderSpec};
use honeydew::sample_cache::SampleCache;
use honeydew::song::plugin::{PLUGIN_EFFECT, PLUGIN_INSTRUMENT};
use honeydew::song::{ChannelKind, Event, Song};

/// The uid of built-in plugin `name`.
fn uid(name: &str) -> String {
    builtin::ALL
        .iter()
        .find(|b| b.name == name)
        .unwrap()
        .uid
        .to_owned()
}

/// The energy of `song` rendered offline (the export's renderer), its tail cut.
fn rendered_energy(song: &Song) -> f32 {
    let spec = RenderSpec {
        range: RenderRange::Full,
        remainder: Remainder::Cut,
        normalize: false,
        high_quality: true,
        rate: 48_000,
        solo: None,
        master_effects: true,
    };
    let mut stream = Render::new(song, &SampleCache::default(), spec)
        .unwrap()
        .finish()
        .unwrap();
    let mut energy = 0.0f32;
    while !stream.done() {
        if let Some([l, r]) = stream.step() {
            energy += l.iter().chain(r).map(|s| s * s).sum::<f32>();
        }
    }
    energy
}

#[test]
fn a_built_in_lemon_trio_note_makes_sound() {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Instrument);
    let ch = &mut song.channels[1];
    let id = ch.id;
    plugins::add(&mut ch.rack, id, &uid("Lemon Trio"), 0).unwrap();
    assert_eq!(ch.rack.modules[0].type_id, PLUGIN_INSTRUMENT);
    let clip = ch.lanes[0].add_clip(0, 4.0);
    clip.events.push(Event::Note {
        pos: 0,
        len_beats: 2.0,
        key: 60,
        velocity: 204,
        pan: 127,
        slide: false,
    });
    plugins::sync(&song);
    let energy = rendered_energy(&song);
    assert!(energy > 1.0, "Lemon Trio plays: energy {energy}");
}

#[test]
fn the_built_in_plugins_keep_their_settings_in_the_song_file() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    song.add_channel(ChannelKind::Instrument);
    let (a, b) = (song.channels[1].id, song.channels[2].id);
    plugins::add(&mut song.channels[1].rack, a, &uid("Lemon Trio"), 0).unwrap();
    plugins::add(
        &mut song.channels[1].rack,
        a,
        &uid("Lemon Parametric EQ"),
        1,
    )
    .unwrap();
    plugins::add(&mut song.channels[2].rack, b, &uid("Lemon Sampler"), 0).unwrap();
    assert_eq!(song.channels[1].rack.modules[1].type_id, PLUGIN_EFFECT);
    // Each one's first param to the far end of its range (a stepped one too): a knob move in the song reaches the
    // plugin, and its state follows.
    let modules = [(1, 0), (1, 1), (2, 0)];
    let mut set = Vec::new();
    for &(c, m) in &modules {
        let p = &mut song.channels[c].rack.modules[m].params[1];
        *p = if *p > 0.5 { 0.0 } else { 1.0 };
        set.push(*p);
    }
    plugins::sync(&song);
    plugins::store_states(&mut song);
    let bytes = honeydew::flm::write(&song).unwrap();
    // Each is an `HDPm`, which other readers of the song format skip, none an `RMOd` of the rack.
    assert_eq!(bytes.windows(4).filter(|w| w == b"HDPm").count(), 3);
    let mut back = honeydew::flm::read(&bytes).unwrap();
    assert_eq!(back, song);
    // The app opened again: each plugin comes back from its saved state alone.
    plugins::song_replaced();
    for &(c, m) in &modules {
        let module = &mut back.channels[c].rack.modules[m];
        module.params.truncate(1);
        module.file.plugin.as_mut().unwrap().params.clear();
    }
    plugins::sync(&back);
    assert!(plugins::adopt_params(&mut back));
    for (&(c, m), want) in modules.iter().zip(set) {
        let module = &back.channels[c].rack.modules[m];
        let got = module.params[1];
        assert!(
            (got - want).abs() < 0.02,
            "{}: {got} against {want}",
            module.title()
        );
    }
}

/// Copies of the plugins installed on their own (fake scan results): none is listed, and a module naming the VST3
/// copy plays the built-in plugin.
#[cfg(plugin_host)]
#[test]
fn an_installed_copy_of_a_built_in_plugin_is_hidden() {
    let record = |id: &str, name: &str| serde_json::json!({ "id": id, "name": name, "vendor": "Lemon Studio", "kind": "instrument" });
    let file = |path: &str, format: &str, plugins: Vec<serde_json::Value>| serde_json::json!({ "path": path, "format": format, "status": "ok", "plugins": plugins });
    let trio_vst3 = "4F70656E53747564696F33784F736321";
    let cache = serde_json::json!({ "files": [
        file("/plugins/Lemon Trio.clap", "clap", vec![record("org.openstudio.open-3x-osc", "Lemon Trio")]),
        file("/plugins/Lemon Trio.vst3", "vst3", vec![record(trio_vst3, "Lemon Trio")]),
        file("/plugins/Other.clap", "clap", vec![record("com.example.other", "Other Synth")]),
    ]});
    plugins::load_cache(&cache.to_string());
    let names: Vec<String> = plugins::available().into_iter().map(|a| a.name).collect();
    assert_eq!(names, ["Other Synth"]);
    // A song made with the VST3 copy.
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let ch = &mut song.channels[1];
    let id = ch.id;
    let module = plugins::add(&mut ch.rack, id, &uid("Lemon Trio"), 0).unwrap();
    let plugin = ch
        .rack
        .get_mut(module)
        .unwrap()
        .file
        .plugin
        .as_mut()
        .unwrap();
    (plugin.format, plugin.id, plugin.state) =
        ("vst3".into(), trio_vst3.into(), Default::default());
    plugins::song_replaced();
    plugins::sync(&song);
    assert!(matches!(
        plugins::state(id, module),
        Some(plugins::State::Live(_))
    ));
}

/// The card's editor (`os_plugkit::embed`) as the card drives it: a knob drag (the editor holds the cursor and takes
/// movement), then its menu's `Type in value...` and a typed value, which come back to the song as the plugin's own
/// param moves.
#[test]
fn the_lemon_trio_editor_takes_a_drag_a_menu_and_a_typed_value() {
    use os_plugkit::embed::{self, Answer, Button, Input, Modal};
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let ch = &mut song.channels[1];
    let id = ch.id;
    let module = plugins::add(&mut ch.rack, id, &uid("Lemon Trio"), 0).unwrap();
    let names = &ch
        .rack
        .get(module)
        .unwrap()
        .file
        .plugin
        .as_ref()
        .unwrap()
        .params;
    let coarse = 1 + names
        .iter()
        .position(|p| p.name == "Osc 1 coarse pitch")
        .unwrap() as i32;
    plugins::sync(&song);
    let editor = plugins::lemon_editor(id, module, 1.0).expect("the editor opens");
    let moved = || {
        let moves = plugins::idle();
        (moves.iter().rev())
            .find(|m| m.param == coarse && m.gesture == 1)
            .map(|m| m.value)
    };
    // Oscillator 1's Coarse knob, at scale 1.
    let (x, y, m) = (322.0, 98.0, (false, false, false));
    let run = |e: &[Input]| {
        for i in e {
            embed::input(editor, i.clone(), m);
        }
        embed::pump(editor);
    };
    run(&[Input::Down {
        x,
        y,
        btn: Button::Left,
        dbl: false,
    }]);
    assert!(embed::locked(editor), "a knob drag holds the cursor");
    run(&[
        Input::Delta { dx: 0.0, dy: -50.0 },
        Input::Up { btn: Button::Left },
    ]);
    assert!(!embed::locked(editor));
    let dragged = moved().expect("the drag moved Coarse");
    assert!(dragged > 0.5, "up from 0 semitones: {dragged}");
    // Its menu: the press that opened it comes again with each answer.
    run(&[
        Input::Down {
            x,
            y,
            btn: Button::Right,
            dbl: false,
        },
        Input::Up { btn: Button::Right },
    ]);
    let Some(Modal::Menu { menu, .. }) = embed::modal(editor) else {
        panic!("a menu shows")
    };
    let item = |label: &str| {
        let is = |it: &os_ui::kit::menu::MenuItem| matches!(it, os_ui::kit::menu::MenuItem::Item { label: l, .. } if l == label);
        menu.items.iter().position(is).unwrap()
    };
    embed::answer(editor, Some(Answer::Pick(vec![item("Type in value...")])));
    embed::pump(editor);
    assert!(matches!(embed::modal(editor), Some(Modal::TypeIn { .. })));
    embed::answer(editor, Some(Answer::Text("-7".into())));
    embed::pump(editor);
    assert_eq!(embed::modal(editor), None);
    let typed = moved().expect("the typed value moved Coarse");
    // -7 of -24..24 semitones.
    assert!((typed - 17.0 / 48.0).abs() < 1e-3, "{typed}");
}

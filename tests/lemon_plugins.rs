//! Lemon Studio's own plugins built into the app (`plugins::builtin`): they play, they keep their settings in the song
//! file, and an installed copy of one is not listed. No audio device: songs render offline.

use honeydew::plugins::{self, builtin};
use honeydew::render::{Remainder, Render, RenderRange, RenderSpec};
use honeydew::sample_cache::SampleCache;
use honeydew::song::plugin::{PLUGIN_EFFECT, PLUGIN_INSTRUMENT};
use honeydew::song::{ChannelKind, Event, Song, SynthKind};

/// The uid of built-in plugin `name`.
fn uid(name: &str) -> String {
    builtin::all()
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

/// A note on each built-in synth sounds, with no NaN (the samplers play their samples in `tests/rt_alloc.rs`).
#[test]
fn a_built_in_lemon_trio_note_makes_sound() {
    for name in ["Lemon Trio", "Citrus"] {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Instrument);
        let ch = &mut song.channels[1];
        let id = ch.id;
        plugins::add(&mut ch.rack, id, &uid(name), 0).unwrap();
        assert_eq!(ch.rack.modules[0].type_id, PLUGIN_INSTRUMENT, "{name}");
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
        assert!(
            energy.is_finite() && energy > 1.0,
            "{name} plays: energy {energy}"
        );
    }
}

/// Lemon Vocoder's carrier is its sidechain input (0.4.9): keyed by a channel, it plays that channel's chord through its
/// own channel's note (the key is the source's rack output: its strip is silent); without the key, next to nothing.
#[test]
fn the_lemon_vocoder_plays_its_sidechain_through_the_note() {
    let song = |keyed: bool| {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let carrier = song.channels[2].id;
        song.channels[2].params.0[0] = 0.0;
        for (c, keys) in [(1, &[48][..]), (2, &[60, 64, 67][..])] {
            let clip = song.channels[c].lanes[0].add_clip(0, 4.0);
            for &key in keys {
                clip.events.push(Event::Note {
                    pos: 0,
                    len_beats: 2.0,
                    key,
                    velocity: 204,
                    pan: 127,
                    slide: false,
                });
            }
        }
        let ch = &mut song.channels[1];
        let id = ch.id;
        let vocoder = plugins::add(&mut ch.rack, id, &uid("Lemon Vocoder"), 1).unwrap();
        let module = ch.rack.get_mut(vocoder).unwrap();
        assert!(module.takes_key());
        // The modulator's own sound off: only the vocoded carrier is heard.
        let names = &module.file.plugin.as_ref().unwrap().params;
        let pass = names
            .iter()
            .position(|p| p.name == "Modulator pass-through level")
            .unwrap();
        module.params[1 + pass] = 0.0;
        module.sidechain = keyed.then_some(carrier);
        song
    };
    let (keyed, plain) = (song(true), song(false));
    plugins::sync(&keyed);
    let (heard, silent) = (rendered_energy(&keyed), rendered_energy(&plain));
    assert!(
        heard > 1.0 && silent < heard * 1e-3,
        "keyed {heard}, without {silent}"
    );
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
/// movement), then its menu's `Enter value...` and a typed value, which come back to the song as the plugin's own
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
    let editor = plugins::lemon_editor(id, module, 1.0).expect("the editor opens")
        as os_plugkit::embed::Parent;
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
    embed::answer(editor, Some(Answer::Pick(vec![item("Enter value...")])));
    embed::pump(editor);
    assert!(matches!(embed::modal(editor), Some(Modal::TypeIn { .. })));
    embed::answer(editor, Some(Answer::Text("-7".into())));
    embed::pump(editor);
    assert_eq!(embed::modal(editor), None);
    let typed = moved().expect("the typed value moved Coarse");
    // -7 of -24..24 semitones.
    assert!((typed - 17.0 / 48.0).abs() < 1e-3, "{typed}");
}

/// An editor slower to paint than its tick (Lemon Chorus's card at ppp 2 in a debug build, a slow phone) parks after each
/// tick, so opening or pumping it hands the app its frame instead of ticking on (`os_plugkit::embed::Window::next`).
#[test]
fn an_editor_slower_than_its_tick_parks_after_each_tick() {
    use os_plugkit::embed::{self, Input, Window};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Ed {
        win: Window,
        ticks: Arc<AtomicUsize>,
    }
    impl Ed {
        fn run(mut self) {
            while let Some(input) = self.win.next() {
                // A paint for every input, longer than a tick; without parking it would tick on (cut at 4).
                let n = self.ticks.load(Ordering::Relaxed);
                if matches!(input, Input::Tick) {
                    self.ticks.store(n + 1, Ordering::Relaxed);
                }
                if n < 4 {
                    std::thread::sleep(std::time::Duration::from_millis(40));
                }
            }
            os_plugkit::park!(self);
        }
    }
    let ticks = Arc::new(AtomicUsize::new(0));
    let t = ticks.clone();
    let id = 0xADDF_0049;
    let _editor = embed::spawn(id, (8, 8), &Default::default(), move |win| {
        Ed { win, ticks: t }.run()
    });
    assert_eq!(ticks.load(Ordering::Relaxed), 1, "one tick, then parked");
    std::thread::sleep(std::time::Duration::from_millis(40));
    embed::pump(id);
    assert_eq!(ticks.load(Ordering::Relaxed), 2);
}

/// Issue #72: an editor's frame names the pixels that changed since the app last took one, so that its card's texture
/// takes those only (a whole frame is megabytes a repaint on a phone).
#[test]
fn an_editor_frame_names_the_pixels_changed_since_the_app_took_the_last() {
    use os_plugkit::embed::{self, Input, Window};
    use std::sync::mpsc::{Receiver, channel};
    struct Ed {
        win: Window,
        frames: Receiver<Vec<u32>>,
    }
    impl Ed {
        fn run(mut self) {
            while self.win.next().is_some() {
                while let Ok(px) = self.frames.try_recv() {
                    self.win.blit(&px, 4, 3);
                }
            }
            os_plugkit::park!(self);
        }
    }
    let (paint, frames) = channel();
    let id = 0xADDF_0072;
    let _editor = embed::spawn(id, (4, 3), &Default::default(), move |win| {
        Ed { win, frames }.run()
    });
    let mut version = 0;
    // Paints `px`, each in turn, then takes the frame: its changed pixels (x0, y0, x1, y1).
    let mut show = |px: &[&[u32; 12]]| {
        px.iter().for_each(|p| paint.send(p.to_vec()).unwrap());
        embed::input(id, Input::Move { x: 0.0, y: 0.0 }, (false, false, false));
        embed::pump(id);
        let mut changed = None;
        version = embed::frame(id, version, |f| changed = Some(f.changed)).expect("a frame");
        changed.unwrap()
    };
    let a = [0; 12];
    let mut b = a;
    b[4 + 1] = 7;
    let mut c = b;
    c[2 * 4 + 3] = 9;
    assert_eq!(show(&[&a]), Some((0, 0, 4, 3)), "the first: all of it");
    assert_eq!(show(&[&b]), Some((1, 1, 2, 2)), "(1, 1)");
    assert_eq!(show(&[&b]), None, "painted the same");
    assert_eq!(
        show(&[&c, &a]),
        Some((1, 1, 4, 3)),
        "two paints: (3, 2), then both back"
    );
}

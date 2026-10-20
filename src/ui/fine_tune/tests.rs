use eframe::egui::{Color32, Event, Key, Modifiers, Pos2, Shape, epaint::ClippedShape};

use super::*;
use crate::metrics::DEFAULT_UI_SCALE;
use crate::params::module_spec;
use crate::song::{ChannelKind, SynthKind};
use crate::ui::modules_browser::{InsertAt, add_module};
use crate::ui::offscreen::{Offscreen, SCREEN};

fn target(instance_id: i32, param: i32) -> AutomationTarget {
    AutomationTarget { instance_id, param }
}

/// Param `i` of module type `type_id`.
fn mspec(type_id: u16, i: usize) -> &'static ParamSpec {
    &module_spec(type_id).unwrap().params[i]
}

const DISTORTION: u16 = 6;
const MINISYNTH_TRANSPOSE: usize = 2;
const RANDOMIZER: u16 = 49;

// ---- availability ----

#[test]
fn channel_level_pan_and_send_levels_are_supported_and_nothing_else_of_the_channel() {
    let on = |param| supported(target(0, param), None);
    assert!(on(0) && on(1), "Level and Pan");
    // Mute, Solo, Pitch Bend, Aftertouch, the unnamed 6 and the MIDI CC params 7…133.
    for param in [2, 3, 4, 5, 6, 7, 70, 133] {
        assert!(!on(param), "{param}");
    }
    // Send Level (0x2000 + the target's id), never Enable Send (0x1000 + it), a two-state param.
    assert!(on(0x2000) && on(0x2001) && on(0x2FFF));
    assert!(!on(0x1000) && !on(0x1001) && !on(0x3000));
}

#[test]
fn drum_pad_params_0_to_10_are_supported_but_mute_and_solo() {
    let on = |param| supported(target(-3, param), None);
    let all: Vec<_> = (-1..=12).filter(|&p| on(p)).collect();
    assert_eq!(all, [0, 1, 2, 5, 6, 7, 8, 9, 10]);
}

#[test]
fn only_continuous_module_params_without_steps_are_supported() {
    let on = |spec: &ParamSpec| supported(target(2, 1), Some(spec));
    // Distortion: Drive (continuous) yes; Enable (two-state) and Kind (a menu) no.
    assert!(on(mspec(DISTORTION, 1)));
    assert!(!on(mspec(DISTORTION, 0)));
    assert!(!on(mspec(DISTORTION, 5)));
    // A continuous param with a discrete step count: MiniSynth Transpose (25 semitone steps), Randomizer Pitch.
    let minisynth = SynthKind::MiniSynth.type_id();
    assert!(!on(mspec(minisynth, MINISYNTH_TRANSPOSE)));
    assert!(!on(mspec(RANDOMIZER, 1)));
    assert!(on(mspec(RANDOMIZER, 2)));
    // The common params are continuous.
    assert!(COMMON_PARAMS.iter().all(on));
    // A module param the registry lacks.
    assert!(!supported(target(2, 1), None));
}

// ---- what the dialog shows ----

/// Master (0), a MiniSynth (1), an effect track (2) with a Distortion and a Randomizer, Drums (3).
fn song() -> Song {
    songs().0
}

/// [`song`], and the instance ids of its Distortion and Randomizer.
fn songs() -> (Song, (i32, i32)) {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    let rack = &mut song.channels[2].rack;
    let ids = (
        add_module(rack, DISTORTION, InsertAt::End),
        add_module(rack, RANDOMIZER, InsertAt::End),
    );
    (song, ids)
}

#[test]
fn a_channel_params_scope_is_channel_and_its_reset_level_unity_and_pan_centre() {
    let song = song();
    let name = &song.channels[1].name;
    let level = param(&song, 1, target(0, 0)).unwrap();
    assert_eq!(level.title, format!("{name} | Channel | Level"));
    assert_eq!(level.reset, 0.85312);
    assert_eq!(
        level.spec.display_text(0.85312),
        CHANNEL_PARAMS[0].display_text(0.85312)
    );
    let pan = param(&song, 1, target(0, 1)).unwrap();
    assert_eq!(pan.title, format!("{name} | Channel | Pan"));
    assert_eq!(pan.reset, 0.5);
    // Its send to the master (channel id 0): named after it, read and reset as a level.
    let send = param(&song, 1, target(0, 0x2000)).unwrap();
    assert_eq!(
        send.title,
        format!("{name} | Channel | Send Level (to MASTER)")
    );
    assert_eq!(send.reset, 0.85312);
    assert_eq!(send.spec, CHANNEL_PARAMS[0]);
    // No send to channel 2; Mute is never fine-tuned.
    assert_eq!(param(&song, 1, target(0, 0x2002)), None);
    assert_eq!(param(&song, 1, target(0, 2)), None);
    assert!(!level.note_effect);
}

#[test]
fn a_drum_pad_params_scope_is_the_pad_and_its_resets_are_the_spec_list() {
    let mut song = song();
    let kit = song.channels[3].drum_kit.as_mut().unwrap();
    let slot = &mut kit.slots[1];
    slot.file.custom_pad_name = "Snare".into();
    let id = -(slot.file.id as i32);
    let resets: Vec<_> = (0..=10)
        .map(|p| param(&song, 3, target(id, p)).map(|p| p.reset))
        .collect();
    let want = [0.85312, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.5];
    for (p, (got, want)) in resets.iter().zip(want).enumerate() {
        match p {
            3 | 4 => assert_eq!(*got, None, "Mute and Solo"),
            _ => assert_eq!(*got, Some(want), "{p}"),
        }
    }
    let name = &song.channels[3].name;
    let start = param(&song, 3, target(id, 10)).unwrap();
    assert_eq!(start.title, format!("{name} | Snare | Sample Start"));
    assert_eq!(start.spec, DRUM_SLOT_PARAMS[10]);
    // A slot that is not there.
    assert_eq!(param(&song, 3, target(-9999, 0)), None);
}

#[test]
fn a_module_params_scope_is_the_module_and_its_reset_the_module_default() {
    let (song, (distortion, randomizer)) = songs();
    let name = &song.channels[2].name;
    let drive = param(&song, 2, target(distortion, 1)).unwrap();
    assert_eq!(drive.title, format!("{name} | Distortion | Drive"));
    assert_eq!(drive.reset, mspec(DISTORTION, 1).default);
    assert_eq!(drive.spec, *mspec(DISTORTION, 1));
    assert!(!drive.note_effect);
    // A common param: Mix (−2), default 1.
    let mix = param(&song, 2, target(distortion, AutomationTarget::MIX)).unwrap();
    assert_eq!(mix.title, format!("{name} | Distortion | Mix"));
    assert_eq!(mix.reset, 1.0);
    // A note effect's continuous param.
    let velocity = param(&song, 2, target(randomizer, 2)).unwrap();
    assert_eq!(velocity.title, format!("{name} | Randomizer | Velocity"));
    assert!(velocity.note_effect);
    // Stepped, two-state, gone.
    assert_eq!(param(&song, 2, target(randomizer, 1)), None);
    assert_eq!(param(&song, 2, target(distortion, 0)), None);
    assert_eq!(param(&song, 2, target(99, 1)), None);
    assert_eq!(param(&song, 7, target(0, 0)), None);
}

// ---- the box ----

#[test]
fn the_box_is_8_u_high_inset_1_u_centred_with_its_close_button_and_three_rows_at_ppp_1() {
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    assert_eq!(m.u(1.0), 32.0);
    let l = layout(SCREEN, &m);
    // 540 / 2 − 4 U = 142; 1 U in from the screen's sides.
    assert_eq!(
        l.dialog,
        Rect::from_min_max(pos2(32.0, 142.0), pos2(1138.0, 398.0))
    );
    assert_eq!(
        l.close,
        Rect::from_min_max(pos2(1074.0, 142.0), pos2(1138.0, 206.0))
    );
    // Inset 2 U left, right and top, 5 U high, thirds on whole pixels: 206, 259.33 → 259, 312.67 → 313, 366.
    let row = |top: f32, bottom: f32| Rect::from_min_max(pos2(96.0, top), pos2(1074.0, bottom));
    assert_eq!(
        l.rows,
        [row(206.0, 259.0), row(259.0, 313.0), row(313.0, 366.0)]
    );
}

#[test]
fn at_ppp_2_the_rows_fall_on_half_points() {
    let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE);
    assert_eq!((m.u(1.0), m.u_px()), (32.0, 64.0));
    let l = layout(SCREEN, &m);
    assert_eq!(
        l.dialog,
        Rect::from_min_max(pos2(32.0, 142.0), pos2(1138.0, 398.0))
    );
    assert_eq!(l.close.size(), vec2(64.0, 64.0));
    // 259.33 → 518.67 px → 519 px; 312.67 → 625.33 px → 625 px.
    let tops: Vec<_> = l.rows.iter().map(|r| r.top()).collect();
    assert_eq!(tops, [206.0, 259.5, 312.5]);
    assert_eq!(l.rows[2].bottom(), 366.0);
    // A screen whose centre is off the pixel grid: the box top is put back on it.
    let odd = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.25));
    let top = layout(odd, &m).dialog.top();
    assert_eq!((top * 2.0).fract(), 0.0, "{top}");
}

// ---- behaviour ----

const TITLE: &str = "Synth | Channel | Level";

struct Rig {
    screen: Offscreen,
    value: f32,
    shown: Shown,
    shapes: Vec<ClippedShape>,
}

impl Rig {
    fn new(value: f32) -> Self {
        let mut rig = Self {
            screen: Offscreen::new(),
            value,
            shown: Shown::default(),
            shapes: Vec::new(),
        };
        // The second frame shows the area's contents (egui lays it out unseen first).
        rig.frame(vec![]);
        rig.frame(vec![]);
        rig
    }

    fn run(&mut self, events: Option<Vec<Event>>, click: Option<Pos2>, drag: Option<(Pos2, Vec2)>) {
        let (value, shown) = (&mut self.value, &mut self.shown);
        *shown = Shown::default();
        let mut f = |ui: &mut eframe::egui::Ui, cx: &UiCtx| {
            let d = Dialog {
                title: TITLE,
                ink: Color32::RED,
                reset: 0.85312,
            };
            let s = show(ui.ctx(), cx, SCREEN, &d, value, |v| format!("v = {v:.3}"));
            shown.changed |= s.changed;
            shown.close |= s.close;
        };
        self.shapes = match (events, click, drag) {
            (Some(events), ..) => self.screen.frame(events, &mut f),
            (_, Some(at), _) => {
                self.screen.click(at, &mut f);
                Vec::new()
            }
            (.., Some((from, by))) => {
                self.screen.drag(from, &[by * 0.5, by], &mut f);
                Vec::new()
            }
            _ => unreachable!(),
        };
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.run(Some(events), None, None);
    }

    fn texts(&self) -> Vec<(String, Color32, f32)> {
        (self.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.galley.job.sections[0].format.color,
                    t.galley.job.sections[0].format.font_id.size,
                )),
                _ => None,
            })
            .collect()
    }

    fn rects(&self) -> Vec<(Rect, Color32)> {
        (self.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) => Some((r.rect, r.fill)),
                _ => None,
            })
            .collect()
    }
}

fn layout1() -> Layout {
    layout(SCREEN, &crate::ui::offscreen::metrics())
}

/// The slider's track at ppp 1: from 0.5 U into the row to 0.5 U before its end.
fn track_x(v: f32) -> f32 {
    let row = layout1().rows[1];
    row.left() + 16.0 + v * (row.width() - 32.0)
}

#[test]
fn it_shows_the_title_and_the_value_text_at_18_pt_and_no_other_text_on_the_dialog_panel() {
    let rig = Rig::new(0.25);
    let colors = &crate::theme::theme_by_id(0).colors;
    let text = colors.text.to_color32();
    let size = crate::ui::offscreen::metrics().font_id(FontSize::Pt18).size;
    assert_eq!(
        rig.texts(),
        [
            (TITLE.to_owned(), text, size),
            ("v = 0.250".to_owned(), text, size)
        ]
    );
    // The dialog base: the screen dimmed, then the box.
    crate::ui::dialog::assert_base(&rig.rects(), colors, SCREEN, layout1().dialog);
}

#[test]
fn a_drag_moves_the_value_along_the_track_and_the_text_follows() {
    let mut rig = Rig::new(0.25);
    let l = layout1();
    let from = pos2(track_x(0.25), l.rows[1].center().y);
    // A quarter of the track to the right.
    let len = l.rows[1].width() - 32.0;
    rig.run(None, None, Some((from, vec2(0.25 * len, 0.0))));
    assert!((rig.value - 0.5).abs() < 1e-4, "{}", rig.value);
    rig.frame(vec![]);
    assert!(rig.texts().iter().any(|(t, ..)| t == "v = 0.500"));
    assert!(!rig.shown.close);
}

#[test]
fn a_touch_does_not_jump_and_a_double_tap_resets() {
    let mut rig = Rig::new(0.25);
    let l = layout1();
    // A tap far from the thumb leaves the value.
    let at = pos2(track_x(0.9), l.rows[1].center().y);
    rig.run(None, Some(at), None);
    assert_eq!(rig.value, 0.25);
    // The second touch-down within 0.3 s sets the reset value.
    rig.frame(vec![Event::PointerButton {
        pos: at,
        button: eframe::egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    }]);
    assert_eq!(rig.value, 0.85312);
    assert!(rig.shown.changed);
}

#[test]
fn the_close_button_escape_or_a_tap_outside_closes_it_and_a_tap_on_the_box_does_not() {
    let l = layout1();
    for (what, at) in [
        ("close", Some(l.close.center())),
        ("outside", Some(pos2(500.0, 50.0))),
        ("box", Some(pos2(500.0, l.dialog.top() + 20.0))),
        ("escape", None),
    ] {
        let mut rig = Rig::new(0.25);
        let mut closed = false;
        match at {
            Some(at) => {
                // Each frame of the click: the press frame or the release frame closes.
                rig.frame(vec![Event::PointerMoved(at)]);
                for pressed in [true, false] {
                    rig.frame(vec![Event::PointerButton {
                        pos: at,
                        button: eframe::egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    }]);
                    closed |= rig.shown.close;
                }
            }
            None => {
                rig.frame(vec![Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }]);
                closed = rig.shown.close;
            }
        }
        assert_eq!(closed, what != "box", "{what}");
        assert_eq!(rig.value, 0.25, "{what}");
    }
}

#[test]
fn the_thumb_is_haloed_while_pressed() {
    let mut rig = Rig::new(0.5);
    let l = layout1();
    let at = pos2(track_x(0.5), l.rows[1].center().y);
    let halo = Color32::RED.gamma_multiply(0.4);
    let haloed = |rig: &Rig| {
        rig.rects()
            .iter()
            .any(|&(r, c)| c == halo && r.height() == 1.25 * 32.0 + 4.0)
    };
    rig.frame(vec![Event::PointerMoved(at)]);
    assert!(!haloed(&rig));
    rig.frame(vec![Event::PointerButton {
        pos: at,
        button: eframe::egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    }]);
    rig.frame(vec![]);
    assert!(haloed(&rig), "{:?}", rig.rects());
}

#[test]
fn the_slider_paints_its_parts_in_their_theme_roles_and_the_close_icon_at_0_75() {
    let rig = Rig::new(0.25);
    let colors = &crate::theme::theme_by_id(0).colors;
    // The row's centre line (y 286) and the value's place on the track (x 112…1058).
    let (cy, x) = (layout1().rows[1].center().y, track_x(0.25));
    let thumb = Rect::from_center_size(pos2(x, cy), vec2(20.0, 40.0));
    let want = [
        // The track in `controlFill`, 3 t thick; the fill from its left end in the channel colour.
        (
            Rect::from_min_max(pos2(112.0, cy - 3.0), pos2(1058.0, cy + 3.0)),
            colors.control_fill.to_color32(),
        ),
        (
            Rect::from_min_max(pos2(112.0, cy - 3.0), pos2(x, cy + 3.0)),
            Color32::RED,
        ),
        // The thumb in `moduleFill`, its 2 px channel inset and its 2 px `menuFill` centre line.
        (thumb, colors.module_fill.to_color32()),
        (thumb.shrink(2.0), Color32::RED),
        (
            Rect::from_center_size(pos2(x, cy), vec2(2.0, 32.0)),
            colors.menu_fill.to_color32(),
        ),
    ];
    let rects = rig.rects();
    for w in want {
        assert!(rects.contains(&w), "{w:?} in {rects:?}");
    }
    // The close icon at 0.75 scale: an edge of U · 0.75 · 4/3 = 1 U, centred in the 2 U button, in `text`.
    let m = crate::ui::offscreen::metrics();
    let close = layout1().close;
    let square = crate::icons::icon_square(close, 0.75, &m);
    assert_eq!(square.size(), vec2(32.0, 32.0));
    assert_eq!(square.center(), close.center());
    let icon = crate::icons::icon_shapes(
        crate::icons::IconId::Close,
        square,
        colors.text.to_color32(),
    );
    let shapes: Vec<_> = rig.shapes.iter().map(|s| s.shape.clone()).collect();
    assert!(!icon.is_empty() && icon.iter().all(|s| shapes.contains(s)));
}

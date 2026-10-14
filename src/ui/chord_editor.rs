//! The chord editor: four rows of buttons from the keys' top down, under the chord strips, editing the chord of the
//! strip last touched. It opens from the keyboard settings' `ChordSet` ≡ `Open Editor`, and only `Close Editor` and the
//! strip's `Reset` close it here (gamepad Menu is the third way; no gamepad input yet): not Escape, a kind change, a
//! hidden panel or a song load.
//!
//! - Labels are FontSize 2 (10 pt), centred on their buttons.
//! - A button press edits at once (a lit radio button edits nothing) and sounds the edited chord, inversion 0, while
//!   held; the app stores the edit, which marks the set edited. A finger holds its button as the keyboard's fingers
//!   hold keys: until it lifts, through a long touch, and never over a focus loss or a hidden web page.
//! - The root buttons use sharps (`C#`).
//! - Every button is a keyboard focus target, in reading order: Tab steps along a row and on to the next, the arrows
//!   move by place, and `open` (Enter) taps the focused button: it edits as a press does, and the chord sounds until
//!   Enter is let go or the focus moves off the button.

use eframe::egui::{
    self, Align2, Id, PointerButton, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::metrics::{FontSize, Metrics};
use crate::song::chords::Chord;
use crate::ui::widgets::pressed;
use crate::ui::{Finger, UiCtx, fingers_on, focus, focus_lost};

/// The strip row's height while the editor is open, at most, in U; the editor and strips' least height together.
const STRIP_ROW: f32 = 2.5;
const EDITOR_MIN: f32 = 8.5;
/// A button's least height and its inset, in U; the corner radii (the extensions' and the others').
const BUTTON_MIN_H: f32 = 1.5;
const INSET: f32 = 0.1;
const EXTENSION_R: f32 = 0.2;
const RADIO_R: f32 = 1.0;

/// The rows, top to bottom: their labels and values.
pub const EXTENSIONS: [(&str, i32); 7] = [
    ("6", 9),
    ("7", 10),
    ("Maj7", 11),
    ("b9", 13),
    ("9", 14),
    ("11", 17),
    ("#11", 18),
];
pub const QUALITIES: [&str; 6] = ["Maj", "Min", "Aug", "Dim", "Sus2", "Sus4"];
pub const ROOTS: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
/// The octave buttons: chord octave 2 … 6.
pub const OCTAVES: [&str; 5] = ["-2", "-1", "0", "+1", "+2"];

/// The editor's rect and the shrunk strip row in the chord keyboard's rect `keys` (its K tall): the strips are
/// s = min(2.5 U, K) at the bottom, the editor (x, keys top, w, max(K, 8.5 U) − s) from the keys' top down. When short
/// it runs on under the strips, which are drawn over it, and off the screen; it never reaches above the keys.
pub fn editor_rects(keys: Rect, m: &Metrics) -> (Rect, Rect) {
    let k = keys.height();
    let s = m.u(STRIP_ROW).min(k);
    let strips = keys.with_min_y(keys.bottom() - s);
    let h = k.max(m.u(EDITOR_MIN)) - s;
    (Rect::from_min_size(keys.min, vec2(keys.width(), h)), strips)
}

/// Button `i` of `n` in row `row` of the four in `editor`: equal widths, max(row h, 1.5 U) tall, inset 0.1 U.
pub fn button_rect(editor: Rect, row: usize, i: usize, n: usize, m: &Metrics) -> Rect {
    let row_h = editor.height() / 4.0;
    let w = editor.width() / n as f32;
    let top = editor.top() + row as f32 * row_h;
    let r = Rect::from_min_size(
        pos2(editor.left() + i as f32 * w, top),
        vec2(w, row_h.max(m.u(BUTTON_MIN_H))),
    );
    r.shrink(m.u(INSET))
}

/// What the editor did this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Edited {
    /// The chord after an edit this frame.
    pub chord: Option<Chord>,
    /// A button is held: the chord to sound (the edited one).
    pub held: Option<Chord>,
}

/// `chord` after pressing button `i` of `row`: an extension toggles; a radio sets its value (a lit one changes nothing).
pub fn press(chord: &Chord, row: usize, i: usize) -> Chord {
    let mut c = chord.clone();
    match row {
        0 => {
            let value = EXTENSIONS[i].1;
            if let Some(at) = c.extensions.iter().position(|&e| e == value) {
                c.extensions.remove(at);
            } else {
                c.extensions.push(value);
                c.extensions.sort_unstable();
            }
        }
        1 => c.quality = i as i32,
        2 => c.base_note = i as i32,
        _ => c.octave = 2 + i as i32,
    }
    c
}

/// Button `i` of `row` is lit for `chord`.
pub fn lit(chord: &Chord, row: usize, i: usize) -> bool {
    match row {
        0 => chord.extensions.contains(&EXTENSIONS[i].1),
        1 => chord.quality == i as i32,
        2 => chord.base_note == i as i32,
        _ => chord.octave == 2 + i as i32,
    }
}

/// Where the focus lands as the editor opens: its first target in reading order, `6`.
pub fn first_button() -> Id {
    Id::new("chord_editor").with((0_usize, 0_usize))
}

/// Every finger and key holding a button is let go, their chord silenced (Preview on or off): one still down holds
/// nothing until it presses again.
pub fn let_go(ctx: &egui::Context) {
    let id = Id::new("chord_editor");
    ctx.data_mut(|d| {
        d.remove::<(u64, Vec<(u64, (usize, usize))>)>(id.with("fingers"));
        d.remove::<(usize, usize)>(id.with("key"));
    });
}

/// Draws the editor over `rect` for `chord` (the target strip's) and reports its edit and held chord.
pub fn chord_editor(ui: &mut Ui, cx: &UiCtx, rect: Rect, chord: &Chord) -> Edited {
    let (m, c) = (cx.m, &cx.theme.colors);
    let (text, module) = (c.text.to_color32(), c.module_fill.to_color32());
    let rows: [Vec<&str>; 4] = [
        EXTENSIONS.iter().map(|e| e.0).collect(),
        QUALITIES.to_vec(),
        ROOTS.to_vec(),
        OCTAVES.to_vec(),
    ];
    let mut out = Edited::default();
    let mut shown = chord.clone();
    let id = Id::new("chord_editor");
    // The button an `open` press tapped, held while its key is down and it keeps the focus (a focus moved off it lets
    // go, as a finger lifted does).
    let key_id = id.with("key");
    let keys = crate::keymap::for_os(ui.ctx().os());
    let open_down = ui.input(|i| keys.down(i, "open"));
    let focused = focus::focused(ui.ctx());
    let mut key_held = (ui.data(|d| d.get_temp::<(usize, usize)>(key_id)))
        .filter(|&cell| open_down && focused == Some(id.with(cell)));
    // The fingers holding a button, as (finger, cell), tracked as the keyboard's are ([`fingers_on`]): egui's own press
    // ends at a long touch and outlives a focus loss. The last pass's, if it drew the editor; a focus loss (the web
    // page's hide too) lets go of them all, since the lift may never come.
    let (fingers_id, pass) = (id.with("fingers"), ui.ctx().cumulative_pass_nr());
    let mut fingers: Vec<(u64, (usize, usize))> = (ui.data(|d| d.get_temp(fingers_id)))
        .filter(|(drawn, _): &(u64, _)| drawn + 1 == pass && !ui.input(focus_lost))
        .map_or_else(Vec::new, |(_, held)| held);
    let mut edit = |cell: (usize, usize)| {
        let edited = press(&shown, cell.0, cell.1);
        if edited != shown {
            shown = edited.clone();
            out.chord = Some(edited);
        }
    };
    let mut cells = Vec::new();
    for (row, labels) in rows.iter().enumerate() {
        for i in 0..labels.len() {
            let r = button_rect(rect, row, i, labels.len(), m);
            let response = focus::interact(ui, r, id.with((row, i)), Sense::click());
            // `open` on the focused button: egui's keyboard click, by no pointer button.
            if response.clicked() && !response.clicked_by(PointerButton::Primary) {
                key_held = Some((row, i));
                edit((row, i));
            }
            cells.push(((row, i), r));
        }
    }
    for finger in fingers_on(ui, |w| cells.iter().any(|&(cell, _)| w == id.with(cell))) {
        match finger {
            // On the top button there: the last drawn, as egui's hit test picks it.
            Finger::Down(f, pos) => {
                if let Some(&(cell, _)) = cells.iter().rev().find(|(_, r)| r.contains(pos)) {
                    fingers.retain(|h| h.0 != f);
                    fingers.push((f, cell));
                    edit(cell);
                }
            }
            Finger::Up(f, _) => fingers.retain(|h| h.0 != f),
            Finger::Move(..) => {}
        }
    }
    if key_held.is_some() || !fingers.is_empty() {
        out.held = Some(shown.clone());
    }
    let held = |cell| key_held == Some(cell) || fingers.iter().any(|h| h.1 == cell);
    ui.data_mut(|d| match key_held {
        Some(cell) => d.insert_temp(key_id, cell),
        None => d.remove::<(usize, usize)>(key_id),
    });
    // Painted after the edit, as it now is.
    let painter = ui.painter();
    let px = 1.0 / m.ppp();
    for (row, labels) in rows.iter().enumerate() {
        let radius = m.u(if row == 0 { EXTENSION_R } else { RADIO_R });
        for (i, label) in labels.iter().enumerate() {
            let r = button_rect(rect, row, i, labels.len(), m);
            let on = lit(&shown, row, i);
            let (fill, ink) = if on { (text, module) } else { (module, text) };
            let fill = if held((row, i)) { pressed(fill) } else { fill };
            let radius = radius.min(r.height() / 2.0);
            painter.rect_filled(r, radius, fill);
            painter.rect_stroke(r, radius, Stroke::new(px, text), StrokeKind::Inside);
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                *label,
                m.font_id(FontSize::Pt10),
                ink,
            );
        }
    }
    ui.data_mut(|d| d.insert_temp(fingers_id, (pass, fingers)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE_INDEX;
    use crate::ui::offscreen::Offscreen;
    use eframe::egui::{
        Color32, Event, Key, Modifiers, PointerButton, Pos2, Shape, epaint::ClippedShape,
    };

    fn metrics(ppp: f32) -> Metrics {
        Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX)
    }

    fn c_major() -> Chord {
        Chord {
            base_note: 0,
            extensions: Vec::new(),
            octave: 3,
            quality: 0,
        }
    }

    #[test]
    fn the_editor_grows_down_from_the_keys_top_under_the_2_5_u_strips_at_k_0_25_5_and_10_u() {
        // The strips are s = min(2.5 U, K) at the bottom of the keys area; the editor is (x, keys top, w,
        // max(K, 8.5 U) − s), never above the keys' top, running under the strips and off the screen when short.
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            let u = m.u(1.0);
            for (k, strips_h, editor_h) in [(0.25, 0.25, 8.25), (5.0, 2.5, 6.0), (10.0, 2.5, 7.5)] {
                let keys = Rect::from_min_size(pos2(128.0, 540.0 - k * u), vec2(1042.0, k * u));
                let (editor, strips) = editor_rects(keys, &m);
                assert_eq!(
                    strips,
                    keys.with_min_y(keys.bottom() - strips_h * u),
                    "K {k}"
                );
                assert_eq!(editor.min, keys.min, "K {k}: from the keys' top");
                assert!(
                    (editor.height() - editor_h * u).abs() < 1e-3,
                    "K {k}: {editor:?}"
                );
                assert_eq!(editor.x_range(), keys.x_range());
            }
            // K ≥ 8.5 U: exactly the area above the strips.
            let keys = Rect::from_min_size(pos2(0.0, 156.0), vec2(1170.0, 12.0 * u));
            let (editor, strips) = editor_rects(keys, &m);
            assert_eq!((editor.top(), editor.bottom()), (keys.top(), strips.top()));
        }
    }

    #[test]
    fn the_rows_are_four_equal_bands_of_equal_buttons_at_least_1_5_u_tall_inset_0_1_u() {
        let m = metrics(1.0);
        let editor = Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 8.0 * 32.0));
        let r = button_rect(editor, 0, 0, 7, &m);
        assert_eq!(
            r,
            Rect::from_min_size(pos2(3.2, 3.2), vec2(200.0 - 6.4, 64.0 - 6.4))
        );
        let r = button_rect(editor, 3, 4, 5, &m);
        assert_eq!(r.left(), 4.0 * 280.0 + 3.2);
        assert_eq!(r.top(), 3.0 * 64.0 + 3.2);
        // Short rows: the buttons keep 1.5 U.
        let short = Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 4.0 * 32.0));
        assert!((button_rect(short, 1, 0, 6, &m).height() - (1.5 * 32.0 - 6.4)).abs() < 1e-4);
    }

    #[test]
    fn extensions_toggle_and_the_radios_set_quality_root_and_octave_2_to_6() {
        let c = c_major();
        let seventh = press(&c, 0, 1);
        assert_eq!(seventh.extensions, [10]);
        let both = press(&seventh, 0, 4);
        assert_eq!(both.extensions, [10, 14]);
        assert_eq!(press(&both, 0, 1).extensions, [14]);
        assert_eq!(press(&c, 1, 3).quality, 3);
        assert_eq!(press(&c, 2, 9).base_note, 9);
        assert_eq!(press(&c, 3, 0).octave, 2);
        assert_eq!(press(&c, 3, 4).octave, 6);
        // A lit radio button stays lit: the chord is unchanged.
        assert_eq!(press(&c, 1, 0), c);
        assert!(lit(&c, 1, 0) && lit(&c, 2, 0) && lit(&c, 3, 1) && !lit(&c, 0, 0));
        assert_eq!(QUALITIES.len(), 6, "no button for 6, power");
    }

    #[test]
    fn a_press_edits_and_holds_the_edited_chord_and_the_buttons_look_lit_or_not() {
        let mut rig = Offscreen::new();
        let rect = Rect::from_min_size(pos2(0.0, 200.0), vec2(1170.0, 8.0 * 32.0));
        let m = metrics(1.0);
        let min = button_rect(rect, 1, 1, 6, &m).center();
        let mut out = Edited::default();
        let mut shapes = Vec::new();
        let mut chord = c_major();
        for events in [
            vec![Event::PointerMoved(min)],
            vec![Event::PointerButton {
                pos: min,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        ] {
            shapes = rig.frame(events, &mut |ui, cx| {
                out = chord_editor(ui, cx, rect, &chord);
            });
            if let Some(c) = &out.chord {
                chord = c.clone();
            }
        }
        assert_eq!(chord.quality, 1);
        assert_eq!(out.held.as_ref().map(|c| c.quality), Some(1));
        let colors = crate::theme::theme_by_id(0).colors;
        let ink = |label: &str| {
            Offscreen::texts(&shapes)
                .into_iter()
                .find(|(t, _)| t == label)
                .map(|t| t.1)
        };
        // `Min` lit (a `text` button, `moduleFill` label); `Maj` not.
        assert_eq!(ink("Min"), Some(colors.module_fill.to_color32()));
        assert_eq!(ink("Maj"), Some(colors.text.to_color32()));
        let fill_at = |p: Pos2| {
            shapes.iter().rev().find_map(|s| match &s.shape {
                Shape::Rect(r) if r.rect.contains(p) && r.fill.a() > 0 => Some(r.fill),
                _ => None,
            })
        };
        assert_eq!(fill_at(min), Some(pressed(colors.text.to_color32())));
        let maj = button_rect(rect, 1, 0, 6, &m).center();
        assert_eq!(fill_at(maj), Some(colors.module_fill.to_color32()));
        // Let go: nothing held.
        rig.frame(
            vec![Event::PointerButton {
                pos: min,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
            &mut |ui, cx| {
                out = chord_editor(ui, cx, rect, &chord);
            },
        );
        assert_eq!(out, Edited::default());
    }

    /// Key `key` going down (`true`) or up, with no modifiers.
    fn key(key: Key, pressed: bool) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        }
    }

    /// The fill of the topmost filled rect at `p`.
    fn fill_at(shapes: &[ClippedShape], p: Pos2) -> Option<Color32> {
        shapes.iter().rev().find_map(|s| match &s.shape {
            Shape::Rect(r) if r.rect.contains(p) && r.fill.a() > 0 => Some(r.fill),
            _ => None,
        })
    }

    #[test]
    fn tab_and_the_arrows_reach_the_cells_and_enter_taps_the_focused_one() {
        use crate::ui::focus;
        use crate::ui::offscreen::key_tap;
        let mut rig = Offscreen::new();
        let rect = Rect::from_min_size(pos2(0.0, 200.0), vec2(1170.0, 8.0 * 32.0));
        let mut chord = c_major();
        let mut out = Edited::default();
        let mut frame = |rig: &mut Offscreen, events: Vec<Event>, chord: &mut Chord| {
            let shapes = rig.frame(events, &mut |ui, cx| {
                focus::begin_frame(ui.ctx());
                out = chord_editor(ui, cx, rect, chord);
                focus::end_frame(ui.ctx(), cx.theme);
            });
            if let Some(c) = &out.chord {
                *chord = c.clone();
            }
            (focus::focused(&rig.ctx), out.clone(), shapes)
        };
        let cell = |row: usize, i: usize| Some(Id::new("chord_editor").with((row, i)));
        frame(&mut rig, vec![], &mut chord);
        // Tab steps along a row, left to right; a row's last cell leads to the next row's first.
        let tap = |k| key_tap(k, Modifiers::NONE);
        assert_eq!(frame(&mut rig, tap(Key::Tab), &mut chord).0, cell(0, 0));
        assert_eq!(frame(&mut rig, tap(Key::Tab), &mut chord).0, cell(0, 1));
        for _ in 2..7 {
            frame(&mut rig, tap(Key::Tab), &mut chord);
        }
        assert_eq!(frame(&mut rig, tap(Key::Tab), &mut chord).0, cell(1, 0));
        // The arrows: down to the root row, right along it.
        let down = frame(&mut rig, tap(Key::ArrowDown), &mut chord).0;
        assert_eq!(down, cell(2, 0));
        let right = frame(&mut rig, tap(Key::ArrowRight), &mut chord).0;
        assert_eq!(right, cell(2, 1));
        // Enter taps `C#`: the root changes at once and the edited chord sounds while Enter is held, the button lit
        // (`text`) and pressed.
        let text = crate::theme::theme_by_id(0).colors.text.to_color32();
        let c_sharp = button_rect(rect, 2, 1, 12, &metrics(1.0)).center();
        let (_, edit, shapes) = frame(&mut rig, vec![key(Key::Enter, true)], &mut chord);
        assert_eq!(chord.base_note, 1);
        assert_eq!(edit.chord.as_ref().map(|c| c.base_note), Some(1));
        assert_eq!(edit.held.as_ref().map(|c| c.base_note), Some(1));
        assert_eq!(fill_at(&shapes, c_sharp), Some(pressed(text)));
        let (_, edit, shapes) = frame(&mut rig, vec![], &mut chord);
        assert_eq!(edit.chord, None);
        assert_eq!(edit.held.as_ref().map(|c| c.base_note), Some(1));
        assert_eq!(fill_at(&shapes, c_sharp), Some(pressed(text)));
        let (_, edit, shapes) = frame(&mut rig, vec![key(Key::Enter, false)], &mut chord);
        assert_eq!(edit, Edited::default());
        assert_eq!(fill_at(&shapes, c_sharp), Some(text));
    }

    #[test]
    fn a_focus_moved_off_the_button_enter_holds_lets_its_chord_go() {
        use crate::ui::focus;
        use crate::ui::offscreen::key_tap;
        let mut rig = Offscreen::new();
        let rect = Rect::from_min_size(pos2(0.0, 200.0), vec2(1170.0, 8.0 * 32.0));
        let chord = c_major();
        let mut out = Edited::default();
        let mut frame = |rig: &mut Offscreen, events: Vec<Event>| {
            let shapes = rig.frame(events, &mut |ui, cx| {
                focus::begin_frame(ui.ctx());
                out = chord_editor(ui, cx, rect, &chord);
                focus::end_frame(ui.ctx(), cx.theme);
            });
            (out.clone(), shapes)
        };
        frame(&mut rig, vec![]);
        // `6`, focused and held by Enter.
        frame(&mut rig, key_tap(Key::Tab, Modifiers::NONE));
        assert!(
            frame(&mut rig, vec![key(Key::Enter, true)])
                .0
                .held
                .is_some()
        );
        // Tab with Enter still down: the chord goes, and `6` is drawn unpressed; it stays let go when the focus comes
        // back.
        let six = button_rect(rect, 0, 0, 7, &metrics(1.0)).center();
        let module = crate::theme::theme_by_id(0).colors.module_fill.to_color32();
        let tab_back = key_tap(Key::Tab, Modifiers::SHIFT);
        for events in [key_tap(Key::Tab, Modifiers::NONE), vec![], tab_back] {
            let (edit, shapes) = frame(&mut rig, events);
            assert_eq!(edit.held, None);
            assert_eq!(fill_at(&shapes, six), Some(module));
        }
    }

    #[test]
    fn a_touch_held_past_egui_long_press_keeps_its_chord_until_it_lifts() {
        use eframe::egui::{TouchDeviceId, TouchId, TouchPhase};
        // egui turns a touch held 0.8 s into a long-touch click, which ends its pointer press; the finger is still
        // down, so the chord sounds on.
        let mut rig = Offscreen::new();
        let rect = Rect::from_min_size(pos2(0.0, 200.0), vec2(1170.0, 8.0 * 32.0));
        let at = button_rect(rect, 1, 1, 6, &metrics(1.0)).center();
        let mut chord = c_major();
        let frame = |rig: &mut Offscreen, events: Vec<Event>, chord: &mut Chord| {
            let mut out = Edited::default();
            rig.frame(events, &mut |ui, cx| {
                out = chord_editor(ui, cx, rect, chord);
            });
            if let Some(c) = &out.chord {
                *chord = c.clone();
            }
            out
        };
        let touch = |phase| Event::Touch {
            device_id: TouchDeviceId(1),
            id: TouchId(7),
            phase,
            pos: at,
            force: None,
        };
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&mut rig, vec![], &mut chord);
        // The integration sends the touch and the pointer press it stands for.
        let down = vec![
            touch(TouchPhase::Start),
            Event::PointerMoved(at),
            button(true),
        ];
        let out = frame(&mut rig, down, &mut chord);
        assert_eq!(chord.quality, 1, "`Min` edits at once");
        assert!(out.held.is_some());
        for _ in 0..3 {
            rig.time += 0.5;
            assert!(frame(&mut rig, vec![], &mut chord).held.is_some());
        }
        let up = vec![touch(TouchPhase::End), button(false), Event::PointerGone];
        assert_eq!(frame(&mut rig, up, &mut chord), Edited::default());
    }

    #[test]
    fn a_focus_loss_lets_a_held_button_go_and_the_press_holds_nothing_after_it() {
        // egui keeps the button down over a focus loss, and the release may never come.
        let mut rig = Offscreen::new();
        let rect = Rect::from_min_size(pos2(0.0, 200.0), vec2(1170.0, 8.0 * 32.0));
        let at = button_rect(rect, 0, 0, 7, &metrics(1.0)).center();
        let chord = c_major();
        let frame = |rig: &mut Offscreen, events: Vec<Event>| {
            let mut out = Edited::default();
            rig.frame(events, &mut |ui, cx| {
                out = chord_editor(ui, cx, rect, &chord);
            });
            out
        };
        frame(&mut rig, vec![Event::PointerMoved(at)]);
        let press = Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        };
        assert!(frame(&mut rig, vec![press]).held.is_some());
        assert_eq!(
            frame(&mut rig, vec![Event::WindowFocused(false)]).held,
            None
        );
        assert_eq!(frame(&mut rig, vec![]).held, None);
    }
}

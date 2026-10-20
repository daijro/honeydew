//! The panel API extensions the module panels need, one small test each.

use eframe::egui::epaint::TextShape;
use eframe::egui::{self, Event, PointerButton, Pos2, Rect, pos2, vec2};

use super::{Rig, U, check, metrics};
use crate::panels::build::*;
use crate::panels::{panel_for, place};
use crate::params::{ParamKind, menu_value, module_spec};
use crate::theme::{Rgba, theme_by_id};

/// A 14 × 5 U content rect at the origin, at U = 32.
pub(super) const RECT: Rect = Rect {
    min: pos2(0.0, 0.0),
    max: pos2(14.0 * U, 5.0 * U - 1.0),
};

impl Rig {
    pub(crate) fn press(&mut self, pos: Pos2, button: PointerButton, pressed: bool) {
        self.frame(vec![Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        }]);
    }

    pub(super) fn secondary_click(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.press(pos, PointerButton::Secondary, true);
        self.press(pos, PointerButton::Secondary, false);
        self.frame(vec![]);
    }

    /// The last text shape painted with `text`.
    pub(crate) fn text_shape(&self, text: &str) -> Option<TextShape> {
        self.shapes.iter().rev().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.clone()),
            _ => None,
        })
    }

    pub(crate) fn drawn(&self, text: &str) -> bool {
        self.text_shape(text).is_some()
    }

    /// Where `text` was painted.
    pub(crate) fn text_rect(&self, text: &str) -> Rect {
        let t = self
            .text_shape(text)
            .unwrap_or_else(|| panic!("{text} not drawn"));
        t.galley.rect.translate(t.pos.to_vec2())
    }

    /// The fills of the filled circles painted inside `rect`.
    pub(super) fn discs_in(&self, rect: Rect) -> Vec<egui::Color32> {
        self.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Circle(c) if rect.contains(c.center) => Some(c.fill),
                _ => None,
            })
            .collect()
    }

    /// The `^` and `v` taps of spinner `#id`: above and below its value text.
    pub(super) fn chevrons(&self, id: u32) -> (Pos2, Pos2) {
        let r = self.rect_of(id);
        let center = pos2(r.center().x, (r.top() + 0.75 * U + r.bottom()) / 2.0);
        (center - vec2(0.0, 0.85 * U), center + vec2(0.0, 0.85 * U))
    }
}

pub(super) fn no_problems(type_id: u16, panel: &Panel) {
    let problems = check(module_spec(type_id).unwrap(), panel);
    assert!(problems.is_empty(), "{problems:#?}");
}

// ---- settings-driven `Show`, items and defaults of controls without a param ----

/// The Analyzer's `Mode` swap (type 26): the spectrum or the spectrogram, chosen by a spinner that is a module
/// setting, not a param.
fn analyzer(mode: f32) -> Panel {
    Panel::new(hstack([
        text(1, "Spectrum").show(Show::Setting(3, 0)),
        text(2, "Spectrogram").show(Show::Setting(3, 1)),
        spinner(3, "Mode")
            .items(&["2D", "3D"])
            .default(mode)
            .w(Size::U(2.0)),
    ]))
}

#[test]
fn a_setting_shows_and_hides_parts() {
    let module = ModuleInstance::new(26, 2);
    let panel = analyzer(0.0);
    no_problems(26, &panel);
    let shown = |state: &PanelState| -> Vec<(u32, Rect)> {
        let got = place(&panel, &module, state, RECT, U);
        got.into_iter().map(|(l, r)| (l.id, r)).collect()
    };
    let mut state = PanelState::new(&panel);
    assert_eq!(state.settings.get(&3), Some(&0.0));
    let spectrum = shown(&state);
    state.settings.insert(3, 1.0);
    let spectrogram = shown(&state);
    assert_eq!(spectrum.iter().map(|s| s.0).collect::<Vec<_>>(), [1, 3]);
    assert_eq!(spectrogram.iter().map(|s| s.0).collect::<Vec<_>>(), [2, 3]);
    // The swapped part takes the same slot.
    assert_eq!(spectrum[0].1, spectrogram[0].1);
    // A default of item 1 starts on the spectrogram.
    let panel = analyzer(1.0);
    let ids = place(&panel, &module, &PanelState::new(&panel), RECT, U);
    assert_eq!(ids.iter().map(|(l, _)| l.id).collect::<Vec<_>>(), [2, 3]);
}

#[test]
fn an_unbound_spinner_shows_its_items_and_edits_its_setting() {
    // The Tuner's `Mode` and `Visual` (type 24): items without a param, `Visual` starting on `Strobe`.
    let panel = Panel::new(hstack([
        spinner(1, "Mode").items(&["Steady", "Reactive"]),
        spinner(2, "Visual")
            .items(&["Basic", "Strobe", "Orbit"])
            .default(1.0),
    ]));
    no_problems(24, &panel);
    let mut rig = Rig::new(24, panel, RECT);
    assert!(rig.drawn("Steady") && rig.drawn("Strobe"));
    let (_, down) = rig.chevrons(1);
    rig.click(down);
    assert_eq!(rig.state.settings[&1], 1.0);
    assert!(rig.drawn("Reactive") && !rig.drawn("Steady"));
    // A setting is not a param.
    assert!(rig.changed.is_empty());
    assert_eq!(rig.module.params, ModuleInstance::new(24, 2).params);
}

#[test]
fn a_setting_flip_relays_out_on_the_same_frame() {
    let mut rig = Rig::new(26, analyzer(0.0), RECT);
    assert!(rig.drawn("Spectrum") && !rig.drawn("Spectrogram"));
    let (_, down) = rig.chevrons(3);
    rig.click(down);
    assert!(rig.drawn("Spectrogram") && !rig.drawn("Spectrum"));
}

#[test]
fn controls_without_a_param_start_at_their_default_with_their_value_text() {
    // The Tuner's `Reference` slider (A4 = 400 + 80·v Hz, default 440 Hz) and its Input button (starts on).
    let panel = Panel::new(hstack([
        v_slider(5, "Reference")
            .value_text()
            .default(0.5)
            .value_fn(|v| format!("{:.0} Hz", 400.0 + 80.0 * v)),
        button(6, "", true).default(1.0),
    ]));
    no_problems(24, &panel);
    let mut rig = Rig::new(24, panel, RECT);
    assert_eq!(rig.state.settings[&5], 0.5);
    assert_eq!(rig.state.settings[&6], 1.0);
    assert!(rig.drawn("440 Hz"));
    // On: the default button's `text` disc.
    let text = theme_by_id(0).colors.text.to_color32();
    assert_eq!(rig.discs_in(rig.rect_of(6)), [text]);
    // A double click resets to the default, as for a param.
    rig.state.settings.insert(5, 1.0);
    let at = rig.rect_of(5).center();
    rig.click(at);
    rig.click(at);
    assert_eq!(rig.state.settings[&5], 0.5);
}

// ---- per-leaf text alignment, colour and font size ----

#[test]
fn texts_take_their_alignment_colour_and_font() {
    // The Spacer's `-` caption `+` row and the Comb Filter's `?` (FontSize 4, `text`).
    let panel = Panel::new(hstack([
        text(10, "-").align(Align2::LEFT_CENTER),
        text(11, "input phase").align(Align2::CENTER_CENTER),
        text(12, "+").align(Align2::RIGHT_CENTER),
        text(2, "?").font(FontSize::Pt18).ink(Ink::Text),
        knob(3, "Gain").ink(Ink::Text),
    ]));
    let rig = Rig::new(34, panel, RECT);
    let (minus, plus, caption) = (
        rig.text_rect("-"),
        rig.text_rect("+"),
        rig.text_rect("input phase"),
    );
    assert!(
        (minus.left() - rig.rect_of(10).left()).abs() < 1.0,
        "{minus:?}"
    );
    assert!(
        (plus.right() - rig.rect_of(12).right()).abs() < 1.0,
        "{plus:?}"
    );
    assert!((caption.center().x - rig.rect_of(11).center().x).abs() < 1.0);
    let colors = theme_by_id(0).colors;
    let question = rig.text_shape("?").unwrap();
    let format = &question.galley.job.sections[0].format;
    assert_eq!(format.font_id.size, metrics().font_id(FontSize::Pt18).size);
    assert_eq!(format.color, colors.text.to_color32());
    // The channel colour elsewhere: the `-`; and a knob's label in its ink.
    let minus = rig.text_shape("-").unwrap();
    assert_eq!(
        minus.galley.job.sections[0].format.color,
        egui::Color32::RED
    );
    let gain = rig.text_shape("Gain").unwrap();
    assert_eq!(
        gain.galley.job.sections[0].format.color,
        colors.text.to_color32()
    );
}

/// A text label without an alignment hangs left, vertically centred.
#[test]
fn a_text_without_an_alignment_hangs_left_and_vertically_centred() {
    let panel = Panel::new(hstack([text(1, "Rhythm"), knob(2, "Gain")]));
    let rig = Rig::new(34, panel, RECT);
    let (galley, rect) = (rig.text_rect("Rhythm"), rig.rect_of(1));
    assert!(
        (galley.left() - rect.left()).abs() < 1.0,
        "{galley:?} in {rect:?}"
    );
    assert!(
        (galley.center().y - rect.center().y).abs() < 1.0,
        "{galley:?} in {rect:?}"
    );
}

/// The text labels that are centred set it: the Tuner's `Input`, the Spacer's captions,
/// 3x Osc's `Levels` captions and SoundFont Player's `...`.
#[test]
fn the_texts_the_spec_centres_are_set_centred() {
    let centred: [(u16, &[&str]); 4] = [
        (24, &["Input"]),
        (34, &["input phase", "output phase"]),
        (39, &["Adjustment", "Vol Tracking", "Key Tracking"]),
        (52, &["..."]),
    ];
    for (type_id, labels) in centred {
        let leaves = crate::panels::panel_for(type_id).unwrap().root.leaves();
        for label in labels {
            let leaf = leaves.iter().find(|l| l.label == *label).unwrap();
            assert_eq!(leaf.align, Align2::CENTER_CENTER, "{type_id} {label}");
        }
    }
}

// ---- labels driven by params ----

/// The Tuned EQ's first band label: the selected key (p9) in octave 2, whole Hz.
///
/// A label function gets the module's params as they are, so it reads them with `.get()`: a module saved before a
/// param existed has fewer.
fn band_1(params: &[f32]) -> String {
    let key = crate::params::menu_index(params.get(9).copied().unwrap_or(0.0), 12) as f32;
    format!("{} Hz", (110.0 * 2f32.powf((key - 9.0) / 12.0)) as u32)
}

#[test]
fn a_label_can_follow_the_params() {
    let panel = Panel::new(hstack([
        v_slider(1, "65 Hz").p(1).label_fn(band_1),
        list(
            10,
            "C · C# · D · D# · E · F · F# · G · G# · A · A# · B",
            Bar::Bottom,
        )
        .p(9),
    ]));
    no_problems(30, &panel);
    let mut rig = Rig::new(30, panel, RECT);
    assert!(rig.drawn("65 Hz"));
    // Pick `A`, the tenth of twelve cells.
    let keys = rig.rect_of(10);
    rig.click(pos2(
        keys.left() + 9.5 * keys.width() / 12.0,
        keys.center().y,
    ));
    assert_eq!(rig.module.params[9], menu_value(9, 12));
    rig.frame(vec![]);
    assert!(rig.drawn("110 Hz") && !rig.drawn("65 Hz"));
    // A label function that does not give the reference label at the defaults is caught.
    let wrong = Panel::new(v_slider(1, "66 Hz").p(1).label_fn(band_1));
    assert_eq!(check(module_spec(30).unwrap(), &wrong).len(), 1);
}

// ---- leaf kinds of kits C and D ----

/// Every new leaf kind on Tape Stop's params (type 42): p1, p2, p6 continuous, p3, p4 menus, p5, p7 toggles.
fn kinds() -> Panel {
    Panel::new(hstack([
        arc(1, 135.0, 90.0, true, Ink::ModuleFill).p(2),
        arc(2, 315.0, 90.0, false, Ink::ModuleFill).p(3),
        tag(3, "Max", true).p(6),
        graph_knob(4, "A. Curve", false).p(1),
        toggle_right(5, "Sync").p(7),
        text_toggle(6, "Play").p(5),
        menu_button(7, "Chord").p(4),
        list(8, "DIST · DELAY", Bar::Boxed).default(1.0),
        spacer(9),
        separator(10),
        xy_pad(11, 1, 6),
    ]))
}

#[test]
fn every_new_leaf_kind_draws_in_its_rect_and_fits_its_param() {
    no_problems(42, &kinds());
    let rig = Rig::new(42, kinds(), RECT);
    // A right label starts 1.7 U into its rect and may run past it (`kits.rs` checks where).
    for (text, id) in [("Max", 3), ("A. Curve", 4), ("Play", 6), ("DELAY", 8)] {
        let at = rig.text_rect(text);
        assert!(rig.rect_of(id).contains(at.center()), "{text} at {at:?}");
    }
    // The separator is a `text` block at 10 %; the spacer draws nothing.
    let text = theme_by_id(0).colors.text.to_color32();
    let separator = rig.rect_of(10);
    assert!(rig.shapes.iter().any(|c| matches!(&c.shape,
        egui::Shape::Rect(r) if r.rect == separator && r.fill == text.gamma_multiply(0.1))));
    let spacer = rig.rect_of(9);
    assert!(
        !rig.shapes
            .iter()
            .any(|c| spacer.contains_rect(c.shape.visual_bounding_rect()))
    );
}

#[test]
fn the_checks_hold_the_new_kinds_to_their_params() {
    let bad = Panel::new(hstack([
        // A toggle kind on a continuous param, an XY pad with one param, a y param on a knob.
        toggle_right(1, "Sync").p(6),
        control_with_y(2),
        xy_pad(3, 1, 7),
        // An unbound spinner without items, a bound one with its own, a disabled item past the end.
        spinner(4, "Mode"),
        spinner(5, "Stop").p(3).items(&["a"]),
        spinner(6, "Start").p(4).disabled_items(&[10]),
        // A setting predicate on a control with a param, and one past its items.
        text(7, "x").show(Show::Setting(1, 0)),
        text(8, "y").show(Show::Setting(9, 2)),
        list(9, "DIST · DELAY", Bar::Boxed),
    ]));
    let problems = check(module_spec(42).unwrap(), &bad);
    for id in [
        "#1",
        "#2",
        "#3",
        "#4",
        "#5",
        "#6",
        "Setting(1, 0)",
        "Setting(9, 2)",
    ] {
        assert!(
            problems.iter().any(|p| p.contains(id)),
            "{id}: {problems:#?}"
        );
    }
    assert_eq!(problems.len(), 8, "{problems:#?}");
}

/// A knob with a stray y param.
fn control_with_y(id: u32) -> Part {
    let mut part = knob(id, "Gain").p(1);
    if let PartKind::Leaf(leaf) = &mut part.kind {
        leaf.y_param = Some(6);
    }
    part
}

#[test]
fn an_xy_pad_sets_both_params_and_reports_both() {
    // MultiFX (type 4): X = p2, Y = p3, Y up. A click sets both.
    let panel = Panel::new(hstack([xy_pad(4, 2, 3)]));
    no_problems(4, &panel);
    let mut rig = Rig::new(4, panel, RECT);
    let pad = rig.rect_of(4);
    rig.frame(vec![Event::PointerMoved(pad.center())]);
    let at = pos2(
        pad.left() + 0.25 * pad.width(),
        pad.bottom() - 0.75 * pad.height(),
    );
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.press(at, PointerButton::Primary, true);
    assert_eq!(rig.changed, [2, 3]);
    assert!(
        (rig.module.params[2] - 0.25).abs() < 0.01,
        "{}",
        rig.module.params[2]
    );
    assert!(
        (rig.module.params[3] - 0.75).abs() < 0.01,
        "{}",
        rig.module.params[3]
    );
    rig.press(at, PointerButton::Primary, false);
    // Secondary clicks move nothing.
    let before = rig.module.params.clone();
    rig.secondary_click(pad.center());
    assert_eq!(rig.module.params, before);
}

// ---- the post-show hook ----

/// The Multiband's master list (type 36): any band item solos (p4 = 1), `All` unsolos.
fn solo(module: &mut ModuleInstance, state: &mut PanelState, edited: &[u32]) -> Vec<usize> {
    let want = f32::from(u8::from(state.settings[&2] != 0.0));
    if edited.contains(&2) && module.params[4] != want {
        module.params[4] = want;
        return vec![4];
    }
    Vec::new()
}

#[test]
fn a_hook_couples_controls_after_each_pass() {
    let panel = Panel::new(hstack([list(2, "All · Low · Mid · High", Bar::Right)])).hook(solo);
    no_problems(36, &panel);
    let mut rig = Rig::new(36, panel, RECT);
    let list = rig.rect_of(2);
    let row = |k: f32| {
        pos2(
            list.center().x,
            list.top() + (k + 0.5) * list.height() / 4.0,
        )
    };
    rig.click(row(1.0));
    assert_eq!(rig.state.settings[&2], 1.0);
    assert_eq!((rig.module.params[4], rig.changed.clone()), (1.0, vec![4]));
    rig.frame(vec![]);
    assert!(rig.changed.is_empty());
    rig.click(row(0.0));
    assert_eq!((rig.module.params[4], rig.changed.clone()), (0.0, vec![4]));
}

// ---- spinner columns and disabled items ----

#[test]
fn a_spinner_leaf_passes_its_columns_and_disabled_items_on() {
    // The Scale's `Scale` (type 50): `Custom`, its 34th item, cannot be picked; from `Whole Tone` the `v`
    // chevron stays.
    let panel = Panel::new(hstack([spinner(14, "Scale").p(2).disabled_items(&[33])]));
    no_problems(50, &panel);
    let mut rig = Rig::new(50, panel, RECT);
    rig.module.params[2] = menu_value(32, 34);
    let (_, down) = rig.chevrons(14);
    rig.click(down);
    assert_eq!(rig.module.params[2], menu_value(32, 34));
    assert!(rig.changed.is_empty());
    // The Arpeggiator's `Arp` (type 51): 24 items in 3 columns of 8.
    let panel = Panel::new(hstack([spinner(14, "Arp").p(10).items_per_column(8)]));
    no_problems(51, &panel);
    let mut rig = Rig::new(51, panel, RECT);
    let at = rig.rect_of(14).center() + vec2(0.0, 0.2 * U);
    rig.click(at);
    rig.frame(vec![]);
    let (first, ninth) = (rig.text_rect("Chord"), rig.text_rect("Bass Up"));
    assert!((first.center().y - ninth.center().y).abs() < 1.0);
    assert!(ninth.center().x > first.center().x + U);
}

// ---- buttons: the parameter menu, module colours, momentary ----

#[test]
fn a_param_bound_button_has_the_parameter_menu() {
    let panel = Panel::new(hstack([
        button(3, "Snap", true).p(7),
        button(19, "Solo", true),
    ]));
    let mut rig = Rig::new(40, panel, RECT);
    rig.secondary_click(rig.rect_of(19).center());
    assert!(!rig.drawn("Link to MIDI CC"));
    rig.secondary_click(rig.rect_of(3).center());
    assert!(rig.drawn("Link to MIDI CC") && !rig.drawn("Reset"));
    assert!(rig.changed.is_empty());
}

#[test]
fn buttons_take_the_modules_colours() {
    // The Comb Filter's Snap: off = `background` fill, `text` label; on = the reverse. It starts on.
    let snap = ButtonColors {
        fill_on: Ink::Text,
        fill_off: Ink::Background,
        text_on: Ink::Background,
        text_off: Some(Ink::Text),
    };
    let panel = Panel::new(hstack([button(3, "Snap", true)
        .p(7)
        .colors(snap)
        .shape(0.7)]));
    let mut rig = Rig::new(40, panel, RECT);
    let colors = theme_by_id(0).colors;
    let (text, background) = (colors.text.to_color32(), colors.background.to_color32());
    let snap_rect = rig.rect_of(3);
    assert_eq!(rig.discs_in(snap_rect), [text]);
    let label = rig.text_shape("Snap").unwrap();
    assert_eq!(label.galley.job.sections[0].format.color, background);
    rig.click(snap_rect.center());
    assert_eq!((rig.module.params[7], rig.changed.clone()), (0.0, vec![7]));
    // The click frame drew the button before the click; the edit asks for the frame that shows it.
    assert!(rig.ctx.has_requested_repaint());
    // Off the button, which the hover would light.
    rig.frame(vec![Event::PointerMoved(pos2(790.0, 590.0))]);
    assert_eq!(rig.discs_in(snap_rect), [background]);
}

/// A two-state button draws on for any value but 0, and a tap writes 1 from exactly 0, else 0.
#[test]
fn a_button_is_on_for_any_value_but_0_and_a_tap_writes_1_only_from_0() {
    // The Comb Filter's Snap, drawn as in `buttons_take_the_modules_colours`: on = a `text` disc.
    let snap = ButtonColors {
        fill_on: Ink::Text,
        fill_off: Ink::Background,
        text_on: Ink::Background,
        text_off: Some(Ink::Text),
    };
    let panel = Panel::new(hstack([button(3, "Snap", true)
        .p(7)
        .colors(snap)
        .shape(0.7)]));
    let mut rig = Rig::new(40, panel, RECT);
    let colors = theme_by_id(0).colors;
    let (text, background) = (colors.text.to_color32(), colors.background.to_color32());
    let snap_rect = rig.rect_of(3);
    // NaN (a corrupt file) is off, as the toggles read it: their `clean` maps it to the default.
    for (v, on) in [
        (0.0, false),
        (0.001, true),
        (0.5, true),
        (1.0, true),
        (f32::NAN, false),
    ] {
        rig.module.params[7] = v;
        rig.frame(vec![]);
        let want = if on { text } else { background };
        assert_eq!(rig.discs_in(snap_rect), [want], "{v}");
    }
    for (from, to) in [(0.5, 0.0), (0.0, 1.0), (f32::NAN, 1.0)] {
        rig.module.params[7] = from;
        rig.click(snap_rect.center());
        assert_eq!(rig.module.params[7], to, "a tap from {from}");
    }
}

#[test]
fn a_momentary_button_is_on_only_while_pressed() {
    // Note Echo's `Stop` (type 47, p15).
    let panel = Panel::new(hstack([button(5, "Stop", false).p(15).momentary()]));
    no_problems(47, &panel);
    let mut rig = Rig::new(47, panel, RECT);
    let at = rig.rect_of(5).center();
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.press(at, PointerButton::Primary, true);
    assert_eq!(
        (rig.module.params[15], rig.changed.clone()),
        (1.0, vec![15])
    );
    rig.frame(vec![]);
    assert_eq!((rig.module.params[15], rig.changed.clone()), (1.0, vec![]));
    rig.press(at, PointerButton::Primary, false);
    assert_eq!(
        (rig.module.params[15], rig.changed.clone()),
        (0.0, vec![15])
    );
    rig.frame(vec![]);
    assert!(rig.changed.is_empty());
}

/// Note Echo's `Stop` (p15) and the SoundFont's `Prev` (unbound): a tap whose press and release land in one frame
/// (a quick touch in a slow frame, coalesced web events) reports 1 in that frame and 0 in the next.
#[test]
fn a_momentary_tap_within_one_frame_reports_the_press_and_the_release() {
    let panel = Panel::new(hstack([
        button(5, "Stop", false).p(15).momentary(),
        text_toggle(6, "Prev").momentary(),
    ]));
    let mut rig = Rig::new(47, panel, RECT);
    for (id, changed, settings) in [(5, vec![15], vec![]), (6, vec![], vec![6])] {
        let at = rig.rect_of(id).center();
        rig.frame(vec![Event::PointerMoved(at)]);
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        rig.frame(vec![button(true), button(false)]);
        let seen = |rig: &Rig| (rig.changed.clone(), rig.edited_settings.clone());
        assert_eq!(seen(&rig), (changed.clone(), settings.clone()), "#{id} tap");
        assert!(rig.ctx.has_requested_repaint(), "#{id}");
        rig.frame(vec![]);
        assert_eq!(seen(&rig), (changed, settings), "#{id} the frame after");
        assert_eq!(rig.module.params[15], 0.0);
        assert_eq!(rig.state.settings[&6], 0.0);
    }
}

/// egui keeps the button down when the window loses the focus, and the release may never come: a momentary button
/// held over a focus loss lets go then, and the next press holds it again (`ui::primary_down_live`).
#[test]
fn a_momentary_button_lets_go_on_a_focus_loss() {
    let panel = Panel::new(hstack([button(5, "Stop", false).p(15).momentary()]));
    let mut rig = Rig::new(47, panel, RECT);
    let at = rig.rect_of(5).center();
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.press(at, PointerButton::Primary, true);
    let seen = |rig: &Rig| (rig.module.params[15], rig.changed.clone());
    assert_eq!(seen(&rig), (1.0, vec![15]));
    rig.frame(vec![Event::WindowFocused(false)]);
    assert_eq!(seen(&rig), (0.0, vec![15]), "the focus loss");
    rig.frame(vec![Event::WindowFocused(true)]);
    rig.frame(vec![]);
    assert_eq!(seen(&rig), (0.0, vec![]), "still stale");
    rig.press(at, PointerButton::Primary, false);
    rig.press(at, PointerButton::Primary, true);
    assert_eq!(seen(&rig), (1.0, vec![15]), "a new press");
}

#[test]
fn a_button_off_its_enabled_predicate_is_dimmed_and_takes_no_input() {
    // Note Echo's `Stop`: enabled while echoes are pending (flag `pending`, which the card sets), else both
    // colours at 50 % alpha. Its label is the theme colour that stands out most against the channel colour.
    let stop = ButtonColors {
        fill_on: Ink::Channel,
        fill_off: Ink::Channel,
        text_on: Ink::Contrast,
        text_off: Some(Ink::Contrast),
    };
    let panel = Panel::new(hstack([button(5, "Stop", false)
        .p(15)
        .momentary()
        .colors(stop)
        .enabled(Show::Flag(flag::PENDING, 1))]));
    no_problems(47, &panel);
    let mut rig = Rig::new(47, panel, RECT);
    let colors = theme_by_id(0).colors;
    let contrast = colors.best_contrasting(Rgba::hex(0xFF0000), &colors.contrast_candidates());
    // Red on Classic Gray: `background`, not `moduleFill`.
    assert_eq!(contrast, colors.background);
    let (red, contrast) = (egui::Color32::RED, contrast.to_color32());
    let rect = rig.rect_of(5);
    let look = |rig: &Rig| {
        let fill = rig.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.rect == rect => Some(r.fill),
            _ => None,
        });
        let label = rig.text_shape("Stop").unwrap();
        (fill, label.galley.job.sections[0].format.color)
    };
    // No flag: nothing pending, the dim look, and a press does nothing.
    let half = |c: egui::Color32| c.gamma_multiply(0.5);
    assert_eq!(look(&rig), (Some(half(red)), half(contrast)));
    let at = rect.center();
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.press(at, PointerButton::Primary, true);
    assert_eq!((rig.module.params[15], rig.changed.clone()), (0.0, vec![]));
    rig.press(at, PointerButton::Primary, false);
    // Pending: full colours, and it works.
    rig.state.flags.insert(flag::PENDING, 1);
    rig.frame(vec![]);
    assert_eq!(look(&rig), (Some(red), contrast));
    rig.press(at, PointerButton::Primary, true);
    assert_eq!(
        (rig.module.params[15], rig.changed.clone()),
        (1.0, vec![15])
    );
}

impl Rig {
    /// Presses on control `#id`'s centre, drags `by`, and releases: the params the drag changed.
    fn drag(&mut self, id: u32, by: egui::Vec2) -> Vec<usize> {
        let at = self.rect_of(id).center();
        self.frame(vec![Event::PointerMoved(at)]);
        self.press(at, PointerButton::Primary, true);
        self.frame(vec![Event::PointerMoved(at + by)]);
        let changed = self.changed.clone();
        self.press(at + by, PointerButton::Primary, false);
        changed
    }

    /// The outer and inner thumb discs of slider `#id`.
    fn thumb(&self, id: u32) -> Vec<egui::Color32> {
        self.discs_in(self.rect_of(id))
    }

    /// The fills of the rects painted inside slider `#id`: its track, then its fill bar.
    fn bars(&self, id: u32) -> Vec<egui::Color32> {
        let rect = self.rect_of(id);
        self.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if rect.contains_rect(r.rect) => Some(r.fill),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn a_slider_off_its_lit_predicate_is_all_module_fill_and_still_takes_a_drag() {
    // The look of the Arpeggiator's `Rotation` with Rhythm Off: track, thumb rim, value bar and thumb centre all
    // `moduleFill`; it always takes touch.
    let pending = Show::Flag(flag::PENDING, 1);
    let panel = Panel::new(hstack([
        v_slider(1, "Bias").p(6).lit(pending),
        h_slider(2, "Drive").p(1).lit(pending),
    ]));
    no_problems(6, &panel);
    let mut rig = Rig::new(6, panel, RECT);
    let colors = theme_by_id(0).colors;
    let [control, module] = [colors.control_fill, colors.module_fill].map(|c| c.to_color32());
    for (flag, look) in [(0, [module, module]), (1, [control, egui::Color32::RED])] {
        rig.state.flags.insert(flag::PENDING, flag);
        // Off rest, so both fill bars have a length.
        (rig.module.params[6], rig.module.params[1]) = (0.75, 0.75);
        rig.frame(vec![]);
        for (id, p) in [(1, 6), (2, 1)] {
            assert_eq!(rig.bars(id), look, "#{id} flag {flag}");
            assert_eq!(rig.thumb(id), look, "#{id} flag {flag}");
            assert_eq!(rig.drag(id, vec2(U, -U)), [p], "#{id} flag {flag}");
        }
    }
}

#[test]
fn the_arpeggiator_rotation_is_flat_while_its_arp_has_no_rhythm_and_every_write_snaps() {
    // `Rotation` (#19 p15, #31 p26) always takes touch. While its own arp's `Rhythm` (p14, p25) is `Off`
    // all four parts are `moduleFill` and a write snaps to 0.5; with a rhythm of n steps it snaps to
    // v = (k + n − 1)/(2(n − 1)), k = round((2v − 1)(n − 1)).
    let mut rig = Rig::new(51, panel_for(51).unwrap().clone(), super::card(10.0).1);
    let colors = theme_by_id(0).colors;
    let [control, module] = [colors.control_fill, colors.module_fill].map(|c| c.to_color32());
    let [r1, r7] = [1, 7].map(|k| menu_value(k, 22));
    for (tab, id, p, rhythm, other) in [(1, 19, 15, 14, 25), (2, 31, 26, 25, 14)] {
        rig.state.tab = tab;
        (rig.module.params[rhythm], rig.module.params[other]) = (0.0, r1);
        rig.module.params[p] = 0.75;
        rig.frame(vec![]);
        assert_eq!(rig.bars(id), [module, module], "#{id} Off");
        assert_eq!(rig.thumb(id), [module, module], "#{id} Off");
        assert_eq!(rig.drag(id, vec2(U, 0.0)), [p], "#{id} Off");
        assert_eq!(rig.module.params[p], 0.5, "#{id} Off");
        // R7 has 6 steps: 0.5 + 0.9 U of the 4 U range is 0.725, k = round(2.25) = 2, v = 7/10.
        rig.module.params[rhythm] = r7;
        rig.frame(vec![]);
        assert_eq!(rig.thumb(id), [control, egui::Color32::RED], "#{id} R7");
        assert_eq!(rig.drag(id, vec2(0.9 * U, 0.0)), [p], "#{id} R7");
        assert_eq!(rig.module.params[p], 0.7, "#{id} R7");
    }
}

#[test]
fn a_page_predicate_needs_a_tab_strip_with_that_page() {
    let spec = module_spec(3).unwrap();
    let pages = |strip: Option<&'static str>, page: usize| {
        let strip = strip.map(|names| tabs(1, names));
        Panel::new(vstack(
            strip
                .into_iter()
                .chain([knob(2, "Gain").p(1).show(Show::Tab(page))]),
        ))
    };
    assert!(check(spec, &pages(Some("BAND 1 · BAND 2"), 1)).is_empty());
    assert_eq!(check(spec, &pages(Some("BAND 1 · BAND 2"), 2)).len(), 1);
    assert_eq!(check(spec, &pages(None, 0)).len(), 1);
}

// ---- bipolar: the registry, then the default ----

/// The leaf of a control part.
fn leaf_of(part: Part) -> Leaf {
    match part.kind {
        PartKind::Leaf(leaf) => leaf,
        _ => unreachable!("a control"),
    }
}

#[test]
fn bipolar_comes_from_the_registry_then_the_default() {
    use crate::panels::is_bipolar;
    // Distortion: `Drive` (p1, default 0.25) and `Stereo` (p4, default 0.5).
    let spec = module_spec(6).unwrap();
    let (drive, stereo) = (&spec.params[1], &spec.params[4]);
    assert!(stereo.bipolar() && !drive.bipolar());
    // 1. A bound control follows the registry, not its leaf default.
    // Instruments too, whose reset values differ from their defaults (MiniSynth `Modifier`).
    for module in crate::params::modules().chain(crate::params::instruments()) {
        for (p, param) in module.params.iter().enumerate() {
            let leaf = leaf_of(knob(1, "x").p(p).default(0.5));
            assert_eq!(
                is_bipolar(&leaf, Some(param)),
                param.bipolar(),
                "{} p{p}",
                module.display_name
            );
            let reset = crate::panels::reset_of(&leaf, Some(param), module.id);
            // The Arpeggiator's knobs pass 0.
            let want = if module.id == 51 { 0.0 } else { param.reset };
            assert_eq!(reset, want, "{} p{p}", module.display_name);
        }
    }
    assert!(!is_bipolar(
        &leaf_of(knob(3, "Drive").p(1).default(0.5)),
        Some(drive)
    ));
    // 2. A control without a param is bipolar when its default is exactly 0.5.
    assert!(is_bipolar(
        &leaf_of(knob(5, "Reference").default(0.5)),
        None
    ));
    assert!(!is_bipolar(
        &leaf_of(knob(5, "Reference").default(0.4)),
        None
    ));
}

#[test]
fn the_registrys_bipolar_reaches_the_widget() {
    // `Drive` and the bipolar `Stereo` both at 0.25: Stereo's ring is |0.25 − 0.5|·2 = 0.5 of the way to its full
    // stroke, Drive's 0.25, so their channel discs differ.
    let panel = Panel::new(hstack([knob(3, "Drive").p(1), knob(7, "Stereo").p(4)]));
    let mut rig = Rig::new(6, panel, RECT);
    rig.module.params[4] = 0.25;
    rig.frame(vec![]);
    let ring = |id: u32| {
        let rect = rig.rect_of(id);
        let radii = rig.shapes.iter().filter_map(|c| match &c.shape {
            egui::Shape::Circle(c) if c.fill == egui::Color32::RED && rect.contains(c.center) => {
                Some(c.radius)
            }
            _ => None,
        });
        radii.fold(0.0, f32::max)
    };
    assert!(ring(7) < ring(3) - 0.1, "{} vs {}", ring(7), ring(3));
}

// ---- module-state flags ----

#[test]
fn a_flag_set_by_the_card_shows_and_hides_parts() {
    // The Tuner's right column follows the channel kind, which the card knows: absent flags read 0.
    const AUDIO: Show = Show::Flag(flag::CHANNEL_KIND, 1);
    let panel = Panel::new(hstack([
        knob(4, "Reference").show(AUDIO),
        v_slider(5, "Reference").show(Show::Not(&AUDIO)),
    ]));
    no_problems(24, &panel);
    let module = ModuleInstance::new(24, 2);
    let mut state = PanelState::new(&panel);
    let ids = |state: &PanelState| -> Vec<u32> {
        place(&panel, &module, state, RECT, U)
            .iter()
            .map(|(l, _)| l.id)
            .collect()
    };
    assert_eq!(ids(&state), [5]);
    state.flags.insert(flag::CHANNEL_KIND, 1);
    assert_eq!(ids(&state), [4]);
}

// ---- halos ----

#[test]
fn a_halo_is_a_channel_disc_bigger_by_its_pad_unless_it_has_its_own_colour() {
    // The halo is the button's own halo colour, else the channel colour. The Tuner's Input button (4 px), a
    // Pitcher `BYP` button (t) and one with a `text` halo, shape 0.7.
    let panel = Panel::new(hstack([
        button(6, "", true).shape(0.7).halo(4.0),
        button(24, "BYP", true).p(10).shape(0.7).halo_t(1.0),
        button(25, "BYP", true)
            .p(11)
            .shape(0.7)
            .halo(2.0)
            .halo_color(Ink::Text),
    ]));
    let rig = Rig::new(25, panel, RECT);
    let text = theme_by_id(0).colors.text.to_color32();
    let m = metrics();
    for (id, pad, halo) in [
        (6, 4.0 / m.ppp(), egui::Color32::RED),
        (24, m.t(), egui::Color32::RED),
        (25, 2.0 / m.ppp(), text),
    ] {
        let rect = rig.rect_of(id);
        let radius = 0.7 * 0.5 * rect.width().min(rect.height());
        let discs: Vec<(egui::Color32, f32)> = rig
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Circle(c) if rect.contains(c.center) => Some((c.fill, c.radius)),
                _ => None,
            })
            .collect();
        assert_eq!(discs.len(), 2, "#{id}: {discs:?}");
        assert_eq!(discs[0], (halo, radius + pad), "#{id}");
        assert_eq!(discs[1].1, radius, "#{id}");
    }
}

#[test]
fn a_pad_in_t_is_t_device_pixels_at_any_density() {
    // ppp 2 at the default scale: U = 64 px, t = ceil(64 / 16) = 4 px.
    let m = crate::metrics::Metrics::from_pixels_per_point(2.0, crate::metrics::DEFAULT_UI_SCALE);
    assert_eq!(Pad::T(1.0).px(&m), 4.0);
    assert_eq!(Pad::T(0.5).px(&m), 2.0);
    assert_eq!(Pad::Px(4.0).px(&m), 4.0);
    assert_eq!(Pad::T(1.0).px(&metrics()), 2.0);
}

// ---- reset ----

#[test]
fn a_reset_comes_from_the_registry_then_the_default() {
    use crate::panels::reset_of;
    let spec = module_spec(6).unwrap();
    let drive = &spec.params[1];
    assert_eq!(
        reset_of(&leaf_of(knob(3, "Drive").p(1)), Some(drive), 6),
        drive.reset
    );
    assert_eq!(
        reset_of(&leaf_of(knob(9, "Ref").default(0.3)), None, 6),
        0.3
    );
    // The Arpeggiator's knobs reset to 0, bound or not; its other controls follow
    // the registry: the Rotation slider (p15) resets to 0.5.
    let arp = module_spec(51).unwrap();
    let gate = &arp.params[9];
    assert_ne!(gate.reset, 0.0);
    assert_eq!(
        reset_of(&leaf_of(knob(15, "Gate").p(9)), Some(gate), 51),
        0.0
    );
    assert_eq!(reset_of(&leaf_of(knob(9, "x").default(0.3)), None, 51), 0.0);
    let rotation = &arp.params[15];
    assert_eq!(
        reset_of(&leaf_of(h_slider(19, "").p(15)), Some(rotation), 51),
        rotation.reset
    );
    // A double tap resets to it, on the second touch-down.
    let panel = Panel::new(hstack([knob(3, "Drive").p(1)]));
    let mut rig = Rig::new(6, panel, RECT);
    rig.module.params[1] = 0.9;
    let at = rig.rect_of(3).center();
    rig.click(at);
    rig.frame(vec![Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    }]);
    assert_eq!(rig.module.params[1], drive.reset);
    assert_eq!(rig.changed, [1]);
}

#[test]
fn a_dw_samplers_program_is_what_a_double_tap_resets_its_attack_and_release_to() {
    // A program sets Attack and Release's value and default, and the reset value is the default.
    let spec = module_spec(1).unwrap();
    let mut rig = Rig::new(1, panel_for(1).unwrap().clone(), RECT);
    let double_tap = |rig: &mut Rig, id: u32| {
        let at = rig.rect_of(id).center();
        rig.click(at);
        rig.press(at, PointerButton::Primary, true);
        rig.press(at, PointerButton::Primary, false);
    };
    let (attack, decay, release) = (11300, 11400, 11600);
    let moved = |rig: &mut Rig| {
        (
            rig.module.params[1],
            rig.module.params[5],
            rig.module.params[2],
        ) = (0.9, 0.2, 0.8)
    };
    let read = |rig: &Rig| {
        (
            rig.module.params[1],
            rig.module.params[5],
            rig.module.params[2],
        )
    };
    // No program: the registry's.
    moved(&mut rig);
    for id in [attack, decay, release] {
        double_tap(&mut rig, id);
    }
    let registry = (
        spec.params[1].reset,
        spec.params[5].reset,
        spec.params[2].reset,
    );
    assert_eq!(read(&rig), registry);
    // A program (a pick, or a song opening) sets Attack and Release: the reset goes there, the other sliders keep the
    // registry's.
    rig.module.set_attack_release((0.25, 0.6));
    moved(&mut rig);
    for id in [attack, decay, release] {
        double_tap(&mut rig, id);
    }
    assert_eq!(read(&rig), (0.25, registry.1, 0.6));
    // Moving them does not move it.
    moved(&mut rig);
    rig.module.init_follows(1);
    double_tap(&mut rig, attack);
    assert_eq!(rig.module.params[1], 0.25);
}

// ---- parameter menus of controls without a param ----

#[test]
fn only_bound_controls_have_the_parameter_items() {
    let panel = Panel::new(hstack([
        knob(1, "Bound").p(1),
        knob(2, "Free"),
        toggle(3, "Free toggle"),
        toggle(4, "Bound toggle").p(4),
    ]));
    let menu = |id: u32| {
        let mut rig = Rig::new(42, panel.clone(), RECT);
        rig.secondary_click(rig.rect_of(id).center());
        ["Reset", "Link to MIDI CC"].map(|item| rig.drawn(item))
    };
    assert_eq!(menu(1), [true, true]);
    // An unbound knob or slider: `Reset` only. An unbound toggle: no menu.
    assert_eq!(menu(2), [true, false]);
    assert_eq!(menu(3), [false, false]);
    assert_eq!(menu(4), [false, true]);
}

// ---- edits of controls without a param ----

#[test]
fn show_reports_the_edited_settings() {
    // The SoundFont's `Prev` (an unbound momentary text button) and a bound toggle.
    let panel = Panel::new(hstack([
        text_toggle(3, "Prev").momentary(),
        toggle(4, "Sync").p(7),
    ]));
    let mut rig = Rig::new(42, panel, RECT);
    let prev = rig.rect_of(3).center();
    rig.frame(vec![Event::PointerMoved(prev)]);
    rig.press(prev, PointerButton::Primary, true);
    assert_eq!(
        (rig.edited_settings.clone(), rig.changed.clone()),
        (vec![3], vec![])
    );
    rig.press(prev, PointerButton::Primary, false);
    assert_eq!(rig.edited_settings, [3]);
    rig.click(rig.rect_of(4).center());
    assert_eq!(
        (rig.edited_settings.clone(), rig.changed.clone()),
        (vec![], vec![7])
    );
}

// ---- hook edge cases ----

/// Copies the list's item into setting #9 (a settings-only change) and returns a param past the module's params.
fn mirror(_: &mut ModuleInstance, state: &mut PanelState, _: &[u32]) -> Vec<usize> {
    let item = state.settings[&2];
    state.settings.insert(9, item);
    vec![999]
}

#[test]
fn a_hook_s_settings_ask_for_a_repaint_and_its_stray_params_are_dropped() {
    let panel = Panel::new(hstack([
        list(2, "All · Low", Bar::Right),
        text(8, "Low soloed").show(Show::Setting(9, 1)),
        toggle(9, "Mirror"),
    ]))
    .hook(mirror);
    let mut rig = Rig::new(36, panel, RECT);
    assert!(rig.changed.is_empty());
    rig.state.settings.insert(2, 1.0);
    rig.frame(vec![]);
    // Nothing was edited, but the hook changed a setting that shows a part: it is shown this frame.
    assert!(rig.ctx.has_requested_repaint());
    assert!(rig.drawn("Low soloed"));
    assert!(rig.changed.is_empty());
}

// ---- button shape, icon and font ----

#[test]
fn a_button_takes_its_shape_icon_and_font() {
    let panel = Panel::new(hstack([
        button(6, "Input", true)
            .shape(0.5)
            .icon(IconId::AudioRecord)
            .font(FontSize::Pt6),
        button(7, "Input", true),
    ]));
    let rig = Rig::new(24, panel, RECT);
    let disc = |id: u32| {
        let rect = rig.rect_of(id);
        let radii = rig.shapes.iter().filter_map(|c| match &c.shape {
            egui::Shape::Circle(c) if rect.contains(c.center) => Some(c.radius),
            _ => None,
        });
        (rect, radii.fold(0.0, f32::max))
    };
    let (rect, r) = disc(6);
    assert_eq!(r, 0.5 * 0.5 * rect.width().min(rect.height()));
    assert_eq!(disc(7).1, 0.5 * rect.width().min(rect.height()));
    // The icon: shapes in the label's colour (`menuFill` on an off button) other than its text, only on #6.
    let ink = theme_by_id(0).colors.text.to_color32();
    let icon = |id: u32| {
        let rect = rig.rect_of(id);
        rig.shapes.iter().any(|c| {
            !matches!(c.shape, egui::Shape::Text(_) | egui::Shape::Circle(_))
                && rect.contains_rect(c.shape.visual_bounding_rect())
                && format!("{:?}", c.shape).contains(&format!("{ink:?}"))
        })
    };
    assert!(icon(6) && !icon(7));
    let sizes: Vec<f32> = rig
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "Input" => {
                Some(t.galley.job.sections[0].format.font_id.size)
            }
            _ => None,
        })
        .collect();
    let pt = |f| metrics().font_id(f).size;
    assert_eq!(sizes, [pt(FontSize::Pt6), pt(FontSize::Pt10)]);
}

// ---- the 3x Osc's custom waveform name ----

#[test]
fn the_3x_osc_sample_name_hangs_from_the_module_area_while_a_shape_is_custom() {
    // A 17 U card's module area at U = 32, with the whole card (1.5 U header above, 0.5 U margin below) on screen.
    let area = Rect::from_min_size(pos2(10.0, 2.0 * U), vec2(16.96 * U, 8.0 * U - 1.0));
    let panel = crate::panels::panel_for(39).expect("3x Osc").clone();
    let mut rig = Rig::new(39, panel, area);
    let full = "A custom waveform with a long file name.wav";
    rig.state.sample_name = full.to_owned();
    // Params 1–3 are the oscillators' Shape menus; `Custom` is an item of each.
    let shapes = &module_spec(39).unwrap().params[1..=3];
    let custom = |p: usize| {
        let ParamKind::Menu { items } = shapes[p - 1].kind else {
            panic!("Shape {p} is a menu");
        };
        let i = items.iter().position(|&s| s == "Custom").expect("Custom");
        menu_value(i, items.len())
    };
    rig.frame(vec![]);
    let shown = |rig: &Rig| rig.texts().into_iter().find(|(t, _)| t.ends_with("e.wav"));
    assert_eq!(shown(&rig), None, "every Shape is Sine");
    // (m.x + 0.1 U, m.y + 0.4 U, 0.3·m.w, m.h), left and bottom aligned: the bottom 0.4 U below the area, in the card's
    // bottom margin and inside its clip. Truncated keeping 5 tail characters, 8 pt, in the channel colour.
    let placed = |rig: &Rig, text: &str| {
        let at = rig.text_rect(text);
        assert!((at.left() - (area.left() + 0.1 * U)).abs() < 1.0, "{at:?}");
        assert!(
            (at.bottom() - (area.bottom() + 0.4 * U)).abs() < 1.0,
            "{at:?}"
        );
        assert!(at.width() <= 0.3 * area.width(), "{at:?}");
        let t = rig.text_shape(text).unwrap();
        let format = &t.galley.job.sections[0].format;
        assert_eq!(format.font_id.size, metrics().font_id(FontSize::Pt8).size);
        assert_eq!(format.color, egui::Color32::RED);
        let clip = rig.shapes.iter().rev().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(c.clip_rect),
            _ => None,
        });
        let clip = clip.unwrap();
        assert_eq!(clip.bottom(), area.bottom() + 0.5 * U, "the card clip");
        assert!(clip.contains_rect(at), "{at:?} clipped by {clip:?}");
    };
    for p in 1..=3 {
        rig.module.params[p] = custom(p);
        rig.frame(vec![]);
        let (text, _) = shown(&rig).unwrap_or_else(|| panic!("Shape {p} Custom"));
        let (head, _) = text
            .split_once("...")
            .unwrap_or_else(|| panic!("{text}: no ..."));
        assert!(full.starts_with(head) && head.len() > 5, "{text}");
        placed(&rig, &text);
        rig.module.params[p] = 0.0;
    }
    rig.module.params[3] = custom(3);
    // While it loads, `loading...` instead.
    rig.state.flags.insert(flag::LOADING, 1);
    rig.frame(vec![]);
    placed(&rig, "loading...");
    assert_eq!(shown(&rig), None);
    // Only on the OSC tab.
    rig.state.tab = 1;
    rig.frame(vec![]);
    assert!(!rig.drawn("loading..."));
    rig.state.tab = 0;
    rig.state.flags.clear();
    rig.frame(vec![]);
    assert!(shown(&rig).is_some());
}

// ---- module-drawn overlays ----

#[test]
fn an_overlay_draws_after_the_controls_with_their_rects_and_the_module_area() {
    // The Tape Stop's readouts: drawn after the children, in each visible control's rect.
    fn mark(
        ui: &egui::Ui,
        cx: &crate::ui::UiCtx,
        m: &ModuleInstance,
        _: &PanelState,
        rects: &[(u32, Rect)],
        area: Rect,
        ink: egui::Color32,
    ) {
        let font = cx.m.font_id(FontSize::Pt8);
        ui.painter()
            .text(area.left_top(), Align2::LEFT_TOP, "area", font, ink);
        for (id, r) in rects {
            let text = format!("#{id} {}", m.params[1]);
            let font = cx.m.font_id(FontSize::Pt8);
            ui.painter()
                .text(r.center(), Align2::CENTER_CENTER, text, font, ink);
        }
    }
    let panel = Panel::new(hstack([knob(1, "Stop").p(1), knob(2, "Start").p(2)])).overlay(mark);
    no_problems(42, &panel);
    let rig = Rig::new(42, panel, RECT);
    for id in [1, 2] {
        let at = rig.text_rect(&format!("#{id} 0.2"));
        assert!(rig.rect_of(id).contains(at.center()), "#{id}");
    }
    // The module area is the content rect.
    assert_eq!(rig.text_rect("area").min, RECT.min);
    // After the controls: the last texts painted.
    let last = rig.texts().last().map(|(t, _)| t.clone());
    assert_eq!(last.as_deref(), Some("#2 0.2"));
}

// ---- module-drawn displays (`Leaf::draw`) and the underlay ----

/// Counts its passes in egui temp data under its id, paints its rect green and `pass n` at its centre.
fn counter(
    ui: &mut egui::Ui,
    cx: &crate::ui::UiCtx,
    id: egui::Id,
    rect: Rect,
    _: &mut ModuleInstance,
    _: &mut PanelState,
    _: egui::Color32,
) -> Vec<usize> {
    let n = ui.data_mut(|d| {
        let n = d.get_temp_mut_or_default::<u32>(id);
        *n += 1;
        *n
    });
    ui.painter().rect_filled(rect, 0.0, egui::Color32::GREEN);
    let font = cx.m.font_id(FontSize::Pt8);
    let ink = egui::Color32::GREEN;
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        format!("pass {n}"),
        font,
        ink,
    );
    Vec::new()
}

#[test]
fn a_display_with_a_draw_draws_itself_once_per_pass_instead_of_the_placeholder() {
    // The Arpeggiator's ring over its knob in a Layer, a display beside the controls, and one without a draw.
    let panel = Panel::new(hstack([
        layer([
            knob(1, "")
                .p(14)
                .w(Size::Percent(100.0))
                .h(Size::Percent(50.0)),
            display(2)
                .w(Size::Percent(100.0))
                .h(Size::Percent(100.0))
                .draw(counter),
        ]),
        display(3).draw(counter),
        display(4),
    ]));
    no_problems(51, &panel);
    let mut rig = Rig::new(51, panel, RECT);
    rig.frame(vec![]);
    // Two frames of one pass each: each display counted two under its own id, and drew in its rect.
    let passes: Vec<(String, Rect)> = rig
        .texts()
        .into_iter()
        .filter(|(t, _)| t.starts_with("pass"))
        .collect();
    assert_eq!(passes.len(), 2, "{passes:?}");
    for ((text, at), id) in passes.iter().zip([2, 3]) {
        assert_eq!(text, "pass 2", "#{id}");
        assert!(rig.rect_of(id).contains(at.center()), "#{id}");
    }
    // Its own rect, and no placeholder fill or outline there.
    let fills = |id: u32| -> Vec<egui::Color32> {
        let rect = rig.rect_of(id);
        let fills = rig.shapes.iter().filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.rect == rect => Some(r.fill),
            _ => None,
        });
        fills.collect()
    };
    assert_eq!(fills(2), [egui::Color32::GREEN]);
    assert_eq!(fills(3), [egui::Color32::GREEN]);
    // Without a draw, the placeholder.
    let colors = theme_by_id(0).colors;
    assert_eq!(fills(4), [colors.control_fill.to_color32()]);
    // Tree order: the ring after its knob.
    let at = |f: &dyn Fn(&egui::Shape) -> bool| rig.shapes.iter().rposition(|c| f(&c.shape));
    let knob = rig.rect_of(1);
    let last_knob = at(&|s| matches!(s, egui::Shape::Circle(c) if knob.contains(c.center)));
    let ring = at(&|s| matches!(s, egui::Shape::Rect(r) if r.rect == rig.rect_of(2)));
    assert!(last_knob.unwrap() < ring.unwrap());
}

/// A tap sets p3 to 1 and returns it, and a param the module lacks.
fn tap_sets_p3(
    ui: &mut egui::Ui,
    _: &crate::ui::UiCtx,
    id: egui::Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    _: egui::Color32,
) -> Vec<usize> {
    if ui.interact(rect, id, egui::Sense::click()).clicked() {
        module.params[3] = 1.0;
        return vec![3, 999];
    }
    Vec::new()
}

#[test]
fn a_display_s_params_are_edits_and_those_the_module_lacks_are_dropped() {
    // The Equalizer's handles move their band's params.
    let panel = Panel::new(hstack([knob(1, "Gain").p(1), display(2).draw(tap_sets_p3)]));
    let mut rig = Rig::new(3, panel, RECT);
    assert!(rig.changed.is_empty());
    rig.click(rig.rect_of(2).center());
    assert_eq!((rig.module.params[3], rig.changed.clone()), (1.0, vec![3]));
    // An edit of a control without a param: its `#n` too.
    assert_eq!(rig.edited_settings, [2]);
    rig.frame(vec![]);
    assert!(rig.changed.is_empty() && rig.edited_settings.is_empty());
}

/// A tap returns only a param the module lacks.
fn tap_returns_a_stray_param(
    ui: &mut egui::Ui,
    _: &crate::ui::UiCtx,
    id: egui::Id,
    rect: Rect,
    _: &mut ModuleInstance,
    _: &mut PanelState,
    _: egui::Color32,
) -> Vec<usize> {
    if ui.interact(rect, id, egui::Sense::click()).clicked() {
        return vec![999];
    }
    Vec::new()
}

#[test]
fn a_display_that_returns_only_params_the_module_lacks_edits_nothing() {
    let panel = Panel::new(hstack([display(2).draw(tap_returns_a_stray_param)]));
    let mut rig = Rig::new(3, panel, RECT);
    rig.click(rig.rect_of(2).center());
    assert!(rig.changed.is_empty() && rig.edited_settings.is_empty());
    assert_eq!(rig.module.params, ModuleInstance::new(3, 2).params);
}

/// Narrows the clip to its own rect and leaves it so.
fn narrows_the_clip(
    ui: &mut egui::Ui,
    _: &crate::ui::UiCtx,
    _: egui::Id,
    rect: Rect,
    _: &mut ModuleInstance,
    _: &mut PanelState,
    _: egui::Color32,
) -> Vec<usize> {
    ui.set_clip_rect(rect);
    Vec::new()
}

#[test]
fn a_draw_that_changes_the_clip_leaves_the_later_controls_the_card_clip() {
    let panel = Panel::new(hstack([
        knob(1, "Before").p(1),
        display(2).draw(narrows_the_clip),
        knob(3, "After").p(2),
    ]));
    let rig = Rig::new(3, panel, RECT);
    let clip = |text: &str| {
        let mut clips = rig.shapes.iter().filter_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(c.clip_rect),
            _ => None,
        });
        clips
            .next_back()
            .unwrap_or_else(|| panic!("{text} not drawn"))
    };
    assert_eq!(clip("After"), clip("Before"));
}

/// A tap selects page 2.
fn tap_picks_page_2(
    ui: &mut egui::Ui,
    _: &crate::ui::UiCtx,
    id: egui::Id,
    rect: Rect,
    _: &mut ModuleInstance,
    state: &mut PanelState,
    _: egui::Color32,
) -> Vec<usize> {
    if ui.interact(rect, id, egui::Sense::click()).clicked() {
        state.tab = 1;
    }
    Vec::new()
}

#[test]
fn a_page_picked_in_a_display_relays_out_on_the_same_frame() {
    // The Equalizer's handle tap selects its band's page.
    let page =
        |k: usize, gain: &'static str| knob(3 + k as u32, gain).p(1 + 4 * k).show(Show::Tab(k));
    let panel = Panel::new(vstack([
        tabs(1, "BAND 1 · BAND 2").h(Size::U(1.25)),
        display(2).draw(tap_picks_page_2),
        page(0, "Gain"),
        page(1, "Gain 2"),
    ]));
    no_problems(3, &panel);
    let mut rig = Rig::new(3, panel, RECT);
    assert!(rig.drawn("Gain") && !rig.drawn("Gain 2"));
    rig.click(rig.rect_of(2).center());
    assert_eq!(rig.state.tab, 1);
    assert!(rig.drawn("Gain 2") && !rig.drawn("Gain"));
    // Not a param; the display's `#n` is among the edited settings, so the card stores the page.
    assert!(rig.changed.is_empty());
    assert_eq!(rig.edited_settings, [2]);
}

/// Paints each visible control's rect green.
fn under(
    ui: &egui::Ui,
    _: &crate::ui::UiCtx,
    _: &ModuleInstance,
    _: &PanelState,
    rects: &[(u32, Rect)],
    _: Rect,
    _: egui::Color32,
) {
    for (_, r) in rects {
        ui.painter().rect_filled(*r, 0.0, egui::Color32::GREEN);
    }
}

#[test]
fn an_underlay_draws_after_the_layout_and_before_the_first_control() {
    // Pitcher 2's note bars, before the children, under its note buttons.
    let panel = Panel::new(hstack([
        button(2, "C", false).p(1),
        button(3, "C#", false).p(2),
    ]))
    .underlay(under);
    no_problems(45, &panel);
    let rig = Rig::new(45, panel, RECT);
    let green = |c: &egui::epaint::ClippedShape, rect: Rect| matches!(&c.shape, egui::Shape::Rect(r) if r.rect == rect && r.fill == egui::Color32::GREEN);
    let (c, c_sharp) = (rig.rect_of(2), rig.rect_of(3));
    let under: Vec<usize> = (rig.shapes.iter().enumerate())
        .filter(|(_, s)| green(s, c) || green(s, c_sharp))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(under.len(), 2, "one per control");
    let first_control = rig.shapes.iter().enumerate().position(|(i, s)| {
        !under.contains(&i)
            && c.union(c_sharp)
                .contains_rect(s.shape.visual_bounding_rect())
    });
    assert!(under[1] < first_control.expect("the buttons"));
}

/// Paints its rect in the colour it is given, and `p1 tab` from its right edge on, overhanging it.
fn marks(
    ui: &mut egui::Ui,
    cx: &crate::ui::UiCtx,
    _: egui::Id,
    rect: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    ink: egui::Color32,
) -> Vec<usize> {
    ui.painter().rect_filled(rect, 0.0, ink);
    let text = format!("{} {}", module.params[1], state.tab);
    let font = cx.m.font_id(FontSize::Pt8);
    ui.painter()
        .text(rect.right_center(), Align2::LEFT_CENTER, text, font, ink);
    Vec::new()
}

#[test]
fn display_shapes_gives_what_one_display_draws_where_show_lays_it_out() {
    use super::display_shapes_in;
    let panel = Panel::new(hstack([knob(1, "Gain").p(1), display(2).draw(marks)]));
    let mut params = ModuleInstance::new(3, 2).params;
    params[1] = 0.75;
    let state = PanelState {
        tab: 1,
        ..Default::default()
    };
    for ppp in [1.0, 2.0] {
        let shapes = display_shapes_in(&panel, 3, 2, &params, &state, RECT, ppp);
        // Only the display's shapes, the text past its rect too; the right half of the content rect at any density.
        let [egui::Shape::Rect(r), egui::Shape::Text(t)] = &shapes[..] else {
            panic!("ppp {ppp}: {shapes:?}");
        };
        let half = Rect::from_min_max(pos2(7.0 * U, 0.0), RECT.max);
        assert_eq!((r.rect, r.fill), (half, egui::Color32::RED), "ppp {ppp}");
        assert_eq!(t.galley.text(), "0.75 1", "ppp {ppp}");
        assert!(t.pos.x >= half.right(), "ppp {ppp}");
    }
    // No params: the registry defaults.
    let shapes = display_shapes_in(&panel, 3, 2, &[], &PanelState::default(), RECT, 1.0);
    let default = ModuleInstance::new(3, 2).params[1];
    assert!(
        shapes.iter().any(
            |s| matches!(s, egui::Shape::Text(t) if t.galley.text() == format!("{default} 0"))
        )
    );
}

#[test]
#[should_panic(expected = "#99 is not shown")]
fn display_shapes_names_a_display_it_cannot_find() {
    // Distortion has no `#99`.
    super::display_shapes(6, 99, &[], &PanelState::default(), RECT, 1.0);
}

#[test]
fn value_texts_read_the_tempo_the_sample_rate_and_the_modules_params() {
    let drawn = |rig: &Rig, text: &str| rig.texts().iter().any(|(t, _)| t == text);
    // Autoduck: Attack follows the song tempo, Hold reads Time (p4).
    let mut rig = Rig::new(29, panel_for(29).unwrap().clone(), super::card(6.5).1);
    rig.module.params[2] = 0.5;
    (rig.module.params[4], rig.module.params[6]) = (0.5, 0.5);
    rig.frame(vec![]);
    assert!(
        drawn(&rig, "inf ms") && drawn(&rig, "25 %"),
        "{:?}",
        rig.texts()
    );
    crate::panels::set_text_env(&rig.ctx, 120.0, 0.0);
    rig.frame(vec![]);
    assert!(drawn(&rig, "125 ms"), "{:?}", rig.texts());
    // Spacer: M/S Delay follows the sample rate.
    let mut rig = Rig::new(34, panel_for(34).unwrap().clone(), super::card(7.0).1);
    rig.module.params[5] = 0.4;
    rig.frame(vec![]);
    assert!(!drawn(&rig, "32.0 us"), "{:?}", rig.texts());
    crate::panels::set_text_env(&rig.ctx, 120.0, 44_100.0);
    rig.frame(vec![]);
    assert!(drawn(&rig, "32.0 us"), "{:?}", rig.texts());
}

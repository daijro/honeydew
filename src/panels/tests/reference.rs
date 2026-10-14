//! The data-driven loop over every module file (`fx/`, `inst/`). The panel batches add panels and reference rows,
//! not tests.

use std::collections::{HashMap, HashSet};

use super::{Rig, U, U_PX, check};
use crate::panels::build::*;
use crate::panels::{fx, inst, panel_for, place};
use crate::params::{ModuleSpec, module_spec, modules};
use eframe::egui::{Rect, pos2, vec2};

/// Every effect of the registry has a file, and the instrument files are the eight supported types.
#[test]
fn every_module_has_a_file() {
    let effects: Vec<u16> = modules().map(|m| m.id).collect();
    assert_eq!(fx::IDS, effects);
    let instruments: Vec<u16> = crate::params::instruments().map(|m| m.id).collect();
    assert_eq!(inst::IDS, [0, 1, 2, 19, 23, 33, 39, 52]);
    assert_eq!(inst::IDS, instruments);
}

/// Every registered panel fits its module ([`check`]) and its reference ([`compare`]). A file without a panel has no
/// reference data.
#[test]
fn every_panel_matches_its_reference() {
    let mut failures = Vec::new();
    for &id in fx::IDS.iter().chain(inst::IDS) {
        let reference = fx::reference(id).or_else(|| inst::reference(id)).unwrap();
        let Some(panel) = panel_for(id) else {
            let empty = reference.cases.is_empty() && reference.settings.is_empty();
            assert!(empty, "type {id}: reference data but no panel");
            continue;
        };
        let spec = module_spec(id)
            .unwrap_or_else(|| panic!("type {id}: a panel but no params in the registry"));
        failures.extend(check(spec, panel));
        failures.extend(compare(spec, panel, reference));
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The data a batch could get wrong without changing a rect at the reference width is caught too.
#[test]
fn the_loop_catches_data_that_keeps_the_rects() {
    let spec = module_spec(6).unwrap();
    let reference = fx::reference(6).unwrap();
    let distortion = |n: u32, f: fn(Part) -> Part| {
        let controls = [
            list(1, "Hard · Tube · Soft · Clip", Bar::Right).p(5),
            v_slider(2, "Bias").p(6),
            knob(3, "Drive").p(1),
            knob(4, "Stereo").p(4),
            v_slider(5, "BitCrush").p(2),
            knob(6, "Gain").p(3),
        ];
        let controls = controls.into_iter().enumerate();
        Panel::new(hstack(
            controls.map(|(i, c)| if i as u32 + 1 == n { f(c) } else { c }),
        ))
    };
    assert!(compare(spec, &distortion(0, |c| c), reference).is_empty());
    type Edit = fn(Part) -> Part;
    let mutations: [(&str, u32, Edit); 25] = [
        ("a knob drawn as a slider", 3, |_| v_slider(3, "Drive").p(1)),
        ("the bar side", 1, |_| {
            list(1, "Hard · Tube · Soft · Clip", Bar::Left).p(5)
        }),
        ("a value strip", 2, |c| c.value_text()),
        ("a 0.01 U margin", 4, |c| c.mt(0.01)),
        ("a percentage for the flex", 6, |c| {
            c.w(Size::Percent(100.0 / 6.0))
        }),
        ("a fixed size for the flex", 5, |c| c.h(Size::U(3.49))),
        ("another param of the same kind", 3, |_| {
            knob(3, "Drive").p(3)
        }),
        ("a label", 2, |_| v_slider(2, "Bias ").p(6)),
        ("a font", 2, |c| c.font(FontSize::Pt10)),
        ("momentary", 3, |c| set(c, |l| l.momentary = true)),
        ("a picker", 3, |c| set(c, |l| l.picker = true)),
        ("items per column", 1, |c| c.items_per_column(5)),
        ("own items", 3, |c| c.items(&["A", "B"])),
        ("disabled items", 1, |c| c.disabled_items(&[1])),
        ("a value text", 3, |c| c.value_fn(|v| format!("{v}"))),
        ("a halo in px", 3, |c| {
            set(c, |l| l.halo = Some(Pad::Px(2.0)))
        }),
        ("a halo in t", 3, |c| set(c, |l| l.halo = Some(Pad::T(1.0)))),
        ("a halo colour", 3, |c| {
            set(c, |l| l.halo_ink = Some(Ink::Text))
        }),
        ("a shape", 3, |c| set(c, |l| l.shape = 0.7)),
        ("an icon", 3, |c| set(c, |l| l.icon = Some(IconId::Play))),
        ("an ink", 3, |c| c.ink(Ink::Text)),
        ("button colours", 3, |c| {
            set(c, |l| {
                l.colors = Some(ButtonColors {
                    fill_on: Ink::Text,
                    fill_off: Ink::Background,
                    text_on: Ink::Background,
                    text_off: None,
                })
            })
        }),
        ("an alignment", 3, |c| c.align(Align2::CENTER_CENTER)),
        ("an enabled predicate", 3, |c| {
            set(c, |l| l.enabled = Show::Flag(flag::PENDING, 1))
        }),
        ("a lit predicate", 3, |c| {
            set(c, |l| l.lit = Show::Flag(flag::PENDING, 1))
        }),
    ];
    for (what, n, f) in mutations {
        assert!(
            !compare(spec, &distortion(n, f), reference).is_empty(),
            "{what}"
        );
    }
    // Edges are exact: `Bias` 1 px to the right is caught, unless its row allows the slack.
    macro_rules! shifted {
        ($slack:expr) => {
            Reference {
                size: [14.0, 3.49],
                settings: &[],
                cases: &[Case {
                    rows: &[
                        row(
                            1,
                            Widget::List(Bar::Right),
                            "Hard · Tube · Soft · Clip",
                            [0.0, 0.0, 2.33, 3.49],
                        )
                        .p(5),
                        row(2, Widget::VSlider, "Bias", [2.34, 0.0, 2.32, 3.49])
                            .p(6)
                            .slack($slack, "a test"),
                        row(3, Widget::Knob, "Drive", [4.66, 0.0, 2.34, 3.49]).p(1),
                        row(4, Widget::Knob, "Stereo", [7.0, 0.0, 2.33, 3.49]).p(4),
                        row(5, Widget::VSlider, "BitCrush", [9.33, 0.0, 2.33, 3.49]).p(2),
                        row(6, Widget::Knob, "Gain", [11.66, 0.0, 2.34, 3.49]).p(3),
                    ],
                    ..Case::DEFAULT
                }],
            }
        };
    }
    const EXACT: Reference = shifted!(0.0);
    const ONE_PX: Reference = shifted!(1.0);
    assert_eq!(compare(spec, &distortion(0, |c| c), &EXACT).len(), 1);
    assert!(compare(spec, &distortion(0, |c| c), &ONE_PX).is_empty());
    // A fresh card's settings must be the reference's.
    let settings = Reference {
        settings: &[(1, 0.0)],
        ..*reference
    };
    assert_eq!(compare(spec, &distortion(0, |c| c), &settings).len(), 1);
    // A draw with no input must change nothing, hooks included.
    let drift = |m: &mut ModuleInstance, _: &mut PanelState, _: &[u32]| {
        m.params[1] = (m.params[1] + 0.1).min(1.0);
        vec![1]
    };
    let drifting = distortion(0, |c| c).hook(drift);
    assert!(
        compare(spec, &drifting, reference)
            .iter()
            .any(|p| p.contains("a draw with no input"))
    );
    // A control that `Show::Never` hides needs no case; any other does.
    let with = |show: Show| {
        let mut panel = distortion(0, |c| c);
        if let PartKind::HStack(c) = &mut panel.root.kind {
            c.push(knob(7, "Hidden").p(1).show(show));
        }
        compare(spec, &panel, reference)
    };
    assert!(with(Show::Never).is_empty(), "{:?}", with(Show::Never));
    assert!(
        with(Show::Item(5, 3))
            .iter()
            .any(|p| p.contains("#7: in no reference case"))
    );
}

/// `part` with `f` applied to its leaf directly: the builders refuse button options on other widgets, and the loop
/// must catch them all the same.
fn set(mut part: Part, f: fn(&mut Leaf)) -> Part {
    if let PartKind::Leaf(leaf) = &mut part.kind {
        f(leaf);
    }
    part
}

/// `part`'s controls with their parts, and whether a `Show::Never` (on them or a container above) hides them.
fn controls(part: &Part, never: bool) -> Vec<(&Part, &Leaf, bool)> {
    let never = never || part.show == Show::Never;
    match &part.kind {
        PartKind::Leaf(leaf) => vec![(part, leaf, never)],
        PartKind::HStack(c) | PartKind::VStack(c) | PartKind::Layer(c) => {
            c.iter().flat_map(|c| controls(c, never)).collect()
        }
    }
}

/// What differs between `panel` and its `reference`, one line each:
/// - a fresh card's settings are the reference's;
/// - in every case, the visible controls are the rows, in order, each with the row's type, label as drawn, param,
///   value strip, font, size rule, items shown, value text and every other leaf option ([`Row`]), and its rect's
///   edges on the row's pixels (within its slack) at `U` = 100;
/// - one offscreen draw of every case (hooks, label functions, seeding) changes nothing;
/// - every control is in some case, except those `Show::Never` hides.
fn compare(spec: &ModuleSpec, panel: &Panel, reference: &Reference) -> Vec<String> {
    let name = spec.display_name;
    let mut out = Vec::new();
    let sorted = |m: &HashMap<u32, f32>| {
        let mut v: Vec<_> = m.iter().map(|(k, v)| (*k, *v)).collect();
        v.sort_by_key(|e| e.0);
        v
    };
    let fresh = PanelState::new(panel).settings;
    let settings: HashMap<u32, f32> = reference.settings.iter().copied().collect();
    if fresh != settings {
        let (fresh, settings) = (sorted(&fresh), sorted(&settings));
        out.push(format!(
            "{name}: fresh settings {fresh:?}, the reference {settings:?}"
        ));
    }
    if reference.cases.is_empty() {
        out.push(format!("{name}: no reference cases"));
        return out;
    }
    let controls = controls(&panel.root, false);
    let mut seen = HashSet::new();
    for case in reference.cases {
        seen.extend(case.rows.iter().map(|r| r.id));
        out.extend(compare_case(spec, panel, &controls, reference.size, case));
    }
    for (_, leaf, never) in &controls {
        if !never && !seen.contains(&leaf.id) {
            out.push(format!("{name} #{}: in no reference case", leaf.id));
        }
    }
    out
}

fn compare_case(
    spec: &ModuleSpec,
    panel: &Panel,
    controls: &[(&Part, &Leaf, bool)],
    [w, h]: [f32; 2],
    case: &Case,
) -> Vec<String> {
    let what = format!("{} [{}]", spec.display_name, case.name);
    let mut module = ModuleInstance::new(spec.id, 2);
    for &(p, v) in case.params {
        *module
            .params
            .get_mut(p)
            .unwrap_or_else(|| panic!("{what}: no p{p}")) = v;
    }
    let mut state = PanelState::new(panel);
    state.tab = case.tab;
    state.settings.extend(case.settings.iter().copied());
    state.flags.extend(case.flags.iter().copied());
    // Whole pixels, as the card gives them: 16.96 × 100 in f32 is 1695.9999, 1 px short.
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(w * U_PX, h * U_PX).round());
    let got = place(panel, &module, &state, rect, U_PX);

    let shown: Vec<u32> = got.iter().map(|(leaf, _)| leaf.id).collect();
    let table: Vec<u32> = case.rows.iter().map(|r| r.id).collect();
    if shown != table {
        return vec![format!("{what}: shows {shown:?}, the table {table:?}")];
    }
    let mut out = Vec::new();
    for ((leaf, r), row) in got.iter().zip(case.rows) {
        let at = format!("#{} of {what}", row.id);
        let label = leaf.label_for(&module.params);
        let record = controls
            .iter()
            .find(|c| c.1.id == leaf.id)
            .map(|c| c.0.record);
        // The items a spinner shows: its menu's when bound, else its own (a list's are its label).
        let spinner = matches!(leaf.widget, Widget::Spinner | Widget::MenuButton);
        let menu = match leaf
            .param
            .and_then(|p| spec.params.get(p))
            .map(|s| s.kind.items())
        {
            Some(items) if spinner && !items.is_empty() => Some(items.join(" · ")),
            _ => None,
        };
        let items = menu.clone().unwrap_or_else(|| leaf.items.join(" · "));
        let want_items = row.items.map(String::from).or(menu).unwrap_or_default();
        let value = leaf
            .param
            .and_then(|p| module.params.get(p))
            .or(state.settings.get(&leaf.id));
        let value = leaf.value_fn.zip(value).map(|(f, &v)| f(v));
        macro_rules! field {
            ($name:literal, $have:expr, $want:expr) => {
                (
                    $name,
                    $have != $want,
                    format!("{:?}", $have),
                    format!("{:?}", $want),
                )
            };
        }
        let fields = [
            field!("type", leaf.widget, row.widget),
            field!("label", label, row.label),
            field!("param", leaf.param, row.param),
            field!("y param", leaf.y_param, row.y_param),
            field!("value strip", leaf.value_text, row.value_text),
            field!("font", leaf.font, row.font),
            field!("size rule", record, Some(row.record)),
            field!("items", items, want_items),
            field!("value text", value.as_deref(), row.value),
            field!("items per column", leaf.column_len, row.column_len),
            field!("disabled items", leaf.disabled_items, row.disabled_items),
            field!("momentary", leaf.momentary, row.momentary),
            field!("picker", leaf.picker, row.picker),
            field!("halo", leaf.halo, row.halo),
            field!("halo colour", leaf.halo_ink, row.halo_ink),
            field!("enabled", leaf.enabled, row.enabled),
            field!("lit", leaf.lit, row.lit),
            field!("shape", leaf.shape, row.shape),
            field!("icon", leaf.icon, row.icon),
            field!("ink", leaf.ink, row.ink),
            field!("colours", leaf.colors, row.colors),
            field!("align", leaf.align, row.align),
        ];
        for (field, differs, have, want) in fields {
            if differs {
                out.push(format!("{at}: {field} {have}, the table {want}"));
            }
        }
        // Bound, a control draws its menu's items, never its own (`panels::show`): no row can pin them.
        if leaf.param.is_some() && !leaf.items.is_empty() {
            out.push(format!("{at}: own items {:?} on a bound leaf", leaf.items));
        }
        let have = [r.min.x, r.min.y, r.max.x, r.max.y];
        let [x, y, w, h] = row.rect.map(|v| (v * U_PX).round());
        let want = [x, y, x + w, y + h];
        let (slack, why) = row.slack;
        if have
            .iter()
            .zip(want)
            .any(|(a, b)| (a - b).abs() > slack + 1e-3)
        {
            out.push(format!(
                "{at}: edges {have:?} px, the table {want:?} (slack {slack} {why})"
            ));
        }
    }
    // One offscreen draw of the case, at U = 32: hooks, label functions and seeding run, and a fresh draw edits
    // nothing.
    let mut rig = Rig::new(
        spec.id,
        panel.clone(),
        Rect::from_min_size(pos2(0.0, 0.0), vec2(w * U, h * U)),
    );
    (rig.module, rig.state) = (module, state);
    rig.frame(vec![]);
    if !rig.changed.is_empty() || !rig.edited_settings.is_empty() {
        out.push(format!(
            "{what}: a draw with no input changed params {:?} and settings {:?}",
            rig.changed, rig.edited_settings
        ));
    }
    out
}

#[test]
fn an_xy_row_pins_both_params() {
    let spec = module_spec(4).unwrap();
    const XY: Reference = Reference {
        size: [14.0, 6.99],
        settings: &[],
        cases: &[Case {
            rows: &[row(4, Widget::XyPad, "", [0.0, 0.0, 14.0, 6.99]).xy(2, 3)],
            ..Case::DEFAULT
        }],
    };
    assert!(compare(spec, &Panel::new(hstack([xy_pad(4, 2, 3)])), &XY).is_empty());
    let swapped = compare(spec, &Panel::new(hstack([xy_pad(4, 2, 1)])), &XY);
    assert!(swapped.iter().any(|p| p.contains("y param")), "{swapped:?}");
}

/// A bound spinner draws its menu's items, never its own: own items on a bound leaf fail even when the row's items
/// are the menu's.
#[test]
fn a_bound_leaf_has_no_items_of_its_own() {
    let spec = module_spec(6).unwrap();
    const SPINNER: Reference = Reference {
        size: [14.0, 3.49],
        settings: &[],
        cases: &[Case {
            rows: &[row(1, Widget::Spinner, "Type", [0.0, 0.0, 14.0, 3.49]).p(5)],
            ..Case::DEFAULT
        }],
    };
    let panel = |f: fn(Part) -> Part| Panel::new(hstack([f(spinner(1, "Type").p(5))]));
    assert!(compare(spec, &panel(|c| c), &SPINNER).is_empty());
    let own = compare(spec, &panel(|c| c.items(&["A", "B"])), &SPINNER);
    assert!(own.iter().any(|p| p.contains("own items")), "{own:?}");
}

#[test]
#[should_panic(expected = "a reason")]
fn slack_needs_a_reason() {
    let _ = row(1, Widget::Knob, "x", [0.0; 4]).slack(1.0, "");
}

/// Options only a button draws (`momentary` also on a text toggle) fail a debug build on any other widget, which
/// would ignore them; `enabled` is a button's or a slider's (a slider also has `lit`).
#[test]
#[cfg(debug_assertions)]
fn a_button_option_on_another_widget_fails() {
    type Edit = fn(Part) -> Part;
    let options: [(&str, Edit); 7] = [
        ("halo", |c| c.halo(1.0)),
        ("halo_t", |c| c.halo_t(1.0)),
        ("halo_color", |c| c.halo_color(Ink::Text)),
        ("colors", |c| {
            c.colors(ButtonColors {
                fill_on: Ink::Text,
                fill_off: Ink::Text,
                text_on: Ink::Text,
                text_off: None,
            })
        }),
        ("icon", |c| c.icon(IconId::Play)),
        ("shape", |c| c.shape(0.7)),
        ("momentary", |c| c.momentary()),
    ];
    for (what, f) in options {
        let fails = |c: Part| std::panic::catch_unwind(|| f(c)).is_err();
        assert!(fails(knob(1, "x").p(1)), "{what} on a knob");
        assert!(fails(v_slider(1, "x").p(1)), "{what} on a slider");
        assert!(!fails(button(1, "x", true)), "{what} on a button");
    }
    let text_toggle = std::panic::catch_unwind(|| text_toggle(1, "x").momentary());
    assert!(text_toggle.is_ok(), "a momentary text toggle");
    // `enabled` is for buttons and sliders, `lit` for sliders.
    let enabled = |c: Part| std::panic::catch_unwind(|| c.enabled(Show::Always)).is_ok();
    assert!(!enabled(knob(1, "x").p(1)), "enabled on a knob");
    assert!(enabled(v_slider(1, "x").p(1)) && enabled(h_slider(1, "x").p(1)));
    assert!(enabled(button(1, "x", true)));
    let lit = |c: Part| std::panic::catch_unwind(|| c.lit(Show::Always)).is_ok();
    assert!(!lit(knob(1, "x").p(1)) && !lit(button(1, "x", true)));
    assert!(lit(v_slider(1, "x").p(1)) && lit(h_slider(1, "x").p(1)));
}

/// Each control bound to a param has the label the parameter tables quote for it in their "UI control" column:
/// A source independent of the panel tables the panels and their rows are written
/// from. Labels that follow the params (`label_fn`, checked at the defaults by [`check`]) and XY pads (which draw no
/// label; the tables quote the axis names) are skipped, and so are the exceptions below.
#[test]
fn bound_labels_are_the_parameter_tables_ui_controls() {
    /// `(type, #n, why)`.
    const EXCEPTIONS: &[(u16, u32, &str)] = &[
        (
            51,
            17,
            "the Arpeggiator's ARP 1 Rhythm knob: its label is hidden, the caption above names it",
        ),
        (
            51,
            29,
            "the Arpeggiator's ARP 2 Rhythm knob: its label is hidden, the caption above names it",
        ),
        (
            51,
            19,
            "the Arpeggiator's ARP 1 Rotation slider has no label strip",
        ),
        (
            51,
            31,
            "the Arpeggiator's ARP 2 Rotation slider has no label strip",
        ),
    ];
    let (mut out, mut used) = (Vec::new(), HashSet::new());
    for &(id, p, want) in super::spec_labels::LABELS {
        let Some(panel) = panel_for(id) else {
            continue;
        };
        for leaf in panel.root.leaves() {
            let skipped = leaf.label_fn.is_some() || leaf.widget == Widget::XyPad;
            if leaf.param != Some(p) || skipped || leaf.label == want {
                continue;
            }
            match EXCEPTIONS.iter().find(|e| (e.0, e.1) == (id, leaf.id)) {
                Some(e) => {
                    used.insert((e.0, e.1));
                }
                None => out.push(format!(
                    "type {id} #{} p{p}: {:?}, the table {want:?}",
                    leaf.id, leaf.label
                )),
            }
        }
    }
    for (id, n, _) in EXCEPTIONS {
        if !used.contains(&(*id, *n)) {
            out.push(format!("type {id} #{n}: a stale exception"));
        }
    }
    assert!(out.is_empty(), "\n{}", out.join("\n"));
}

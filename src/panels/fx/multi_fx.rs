//! MultiFX (ID 4), UIHeight 9 U.

use crate::panels::build::*;
#[cfg(test)]
use crate::params::menu_value;

const KINDS: &str = "Delay · Reverb · LP Filter · HP Filter · Flanger · Phaser · Panner · Vox";

/// The XY pad's axis names `[X, Y]` per FX Kind (p1), in the menu's order.
const AXES: [[&str; 2]; 8] = [
    ["Feedback", "Filter"],
    ["Decay", "Filter"],
    ["Frequency", "Resonance"],
    ["Frequency", "Resonance"],
    ["Frequency", "Feedback"],
    ["Frequency", "Feedback"],
    ["Panning", "Distance"],
    ["Vowel", "Size"],
];

/// The XY pad's label: its axis names for the current FX Kind, `"X · Y"`, which the pad draws beside its crosshair.
fn axes(params: &[f32]) -> String {
    let kind = params.get(1).map_or(0, |&v| {
        crate::params::module_spec(4).unwrap().params[1]
            .kind
            .item(v)
    });
    AXES[kind].join(" · ")
}

pub fn panel() -> Option<Panel> {
    let mut pad = xy_pad(4, 2, 3).w(Size::Flex(6.0)).mt(0.5).label_fn(axes);
    // `xy_pad` takes no label; the label is what `axes` gives at the defaults (Delay).
    if let PartKind::Leaf(leaf) = &mut pad.kind {
        leaf.label = "Feedback · Filter";
    }
    Some(Panel::new(vstack([
        list(1, KINDS, Bar::Bottom).p(1).h(Size::U(1.25)),
        hstack([
            vstack([spinner(2, "LFO Rate").p(4), spinner(3, "LFO Shape").p(6)]),
            pad,
            v_slider(5, "LFO Amnt").p(5),
        ]),
    ])))
}

/// The rows with the XY pad named `axes`: the layout is the same for every FX Kind.
#[cfg(test)]
#[rustfmt::skip]
const fn rows(axes: &'static str) -> [Row; 5] {
    [
        row(1, Widget::List(Bar::Bottom), KINDS, [0.0, 0.0, 14.0, 1.25]).p(1).h(Size::U(1.25)),
        row(2, Widget::Spinner, "LFO Rate", [0.0, 1.25, 1.75, 2.87]).p(4),
        row(3, Widget::Spinner, "LFO Shape", [0.0, 4.12, 1.75, 2.87]).p(6),
        row(4, Widget::XyPad, axes, [1.75, 1.75, 10.5, 5.24]).xy(2, 3).w(Size::Flex(6.0)).margins(0.5, 0.0, 0.0, 0.0),
        row(5, Widget::VSlider, "LFO Amnt", [12.25, 1.25, 1.75, 5.74]).p(5),
    ]
}

/// One case per FX Kind: the strip selects it, and the pad's axis names follow it.
#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 6.99],
    settings: &[],
    cases: &[
        Case { name: "Delay", rows: &rows("Feedback · Filter"), ..Case::DEFAULT },
        Case { name: "Reverb", params: &[(1, menu_value(1, 8))], rows: &rows("Decay · Filter"), ..Case::DEFAULT },
        Case { name: "LP Filter", params: &[(1, menu_value(2, 8))], rows: &rows("Frequency · Resonance"), ..Case::DEFAULT },
        Case { name: "HP Filter", params: &[(1, menu_value(3, 8))], rows: &rows("Frequency · Resonance"), ..Case::DEFAULT },
        Case { name: "Flanger", params: &[(1, menu_value(4, 8))], rows: &rows("Frequency · Feedback"), ..Case::DEFAULT },
        Case { name: "Phaser", params: &[(1, menu_value(5, 8))], rows: &rows("Frequency · Feedback"), ..Case::DEFAULT },
        Case { name: "Panner", params: &[(1, menu_value(6, 8))], rows: &rows("Panning · Distance"), ..Case::DEFAULT },
        Case { name: "Vox", params: &[(1, menu_value(7, 8))], rows: &rows("Vowel · Size"), ..Case::DEFAULT },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_axes_read_the_kind_by_its_param_law() {
        // FX Kind is trunc(7v): 0.5 is HP Filter, as the DSP picks, not the nearest item Flanger.
        assert_eq!(axes(&[1.0, 0.5]), "Frequency · Resonance");
    }
}

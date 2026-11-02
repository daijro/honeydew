//! Reverb (legacy) (ID 12), UIHeight 7 U.

use crate::panels::build::*;

pub fn panel() -> Option<Panel> {
    Some(Panel::new(vstack([
        tabs(1, "MAIN · TUNE · MIXER").h(Size::U(1.25)),
        hstack([
            knob(2, "Decay").p(1),
            knob(3, "High Damp").p(2),
            knob(4, "Mix").p(3),
        ])
        .show(Show::Tab(0)),
        hstack([
            v_slider(5, "Predelay").p(4),
            v_slider(6, "Room Size").p(5),
            v_slider(7, "Diffusion").p(6),
            v_slider(8, "Width").p(7),
            v_slider(9, "Lo-Cut").p(8),
            v_slider(10, "Hi-Cut").p(9),
            v_slider(11, "Mod Speed").p(19),
            v_slider(12, "Mod Amount").p(20),
        ])
        .show(Show::Tab(1)),
        hstack([
            knob(13, "Input").p(10),
            knob(14, "Early Refl.").p(11),
            knob(15, "Dry Level").p(12),
            v_slider(16, "Bass Mult.").p(17).w(Size::Flex(0.75)),
            v_slider(17, "CRSV").p(18).w(Size::Flex(0.75)),
        ])
        .show(Show::Tab(2)),
    ])))
}

#[cfg(test)]
const TABS: Row = row(
    1,
    Widget::Tabs,
    "MAIN · TUNE · MIXER",
    [0.0, 0.0, 14.0, 1.25],
)
.h(Size::U(1.25));

#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 4.99],
    settings: &[],
    cases: &[
        Case {
            name: "MAIN",
            rows: &[
                TABS,
                row(2, Widget::Knob, "Decay", [0.0, 1.25, 4.66, 3.74]).p(1),
                row(3, Widget::Knob, "High Damp", [4.66, 1.25, 4.67, 3.74]).p(2),
                row(4, Widget::Knob, "Mix", [9.33, 1.25, 4.67, 3.74]).p(3),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "TUNE",
            tab: 1,
            rows: &[
                TABS,
                row(5, Widget::VSlider, "Predelay", [0.0, 1.25, 1.75, 3.74]).p(4),
                row(6, Widget::VSlider, "Room Size", [1.75, 1.25, 1.75, 3.74]).p(5),
                row(7, Widget::VSlider, "Diffusion", [3.5, 1.25, 1.75, 3.74]).p(6),
                row(8, Widget::VSlider, "Width", [5.25, 1.25, 1.75, 3.74]).p(7),
                row(9, Widget::VSlider, "Lo-Cut", [7.0, 1.25, 1.75, 3.74]).p(8),
                row(10, Widget::VSlider, "Hi-Cut", [8.75, 1.25, 1.75, 3.74]).p(9),
                row(11, Widget::VSlider, "Mod Speed", [10.5, 1.25, 1.75, 3.74]).p(19),
                row(12, Widget::VSlider, "Mod Amount", [12.25, 1.25, 1.75, 3.74]).p(20),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "MIXER",
            tab: 2,
            rows: &[
                TABS,
                row(13, Widget::Knob, "Input", [0.0, 1.25, 3.11, 3.74]).p(10),
                row(14, Widget::Knob, "Early Refl.", [3.11, 1.25, 3.11, 3.74]).p(11),
                row(15, Widget::Knob, "Dry Level", [6.22, 1.25, 3.11, 3.74]).p(12),
                row(16, Widget::VSlider, "Bass Mult.", [9.33, 1.25, 2.33, 3.74]).p(17).w(Size::Flex(0.75)),
                row(17, Widget::VSlider, "CRSV", [11.66, 1.25, 2.34, 3.74]).p(18).w(Size::Flex(0.75)),
            ],
            ..Case::DEFAULT
        },
    ],
};

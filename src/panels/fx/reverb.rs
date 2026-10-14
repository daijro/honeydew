//! Reverb (ID 32), UIHeight 7 U. Pages `MAIN` (0), `TUNE` (1) and `MIXER` (2).

use crate::panels::build::*;

pub fn panel() -> Option<Panel> {
    Some(Panel::new(vstack([
        tabs(1, "MAIN · TUNE · MIXER").h(Size::U(1.25)),
        hstack([
            knob(2, "Decay").p(1),
            knob(3, "High Damp").p(2),
            knob(4, "Mix").p(3),
            knob(5, "Early Refl.").p(5),
            knob(6, "Room Size").p(7),
        ])
        .show(Show::Tab(0)),
        hstack([
            v_slider(7, "Predelay").p(6),
            v_slider(8, "Diffusion").p(8),
            v_slider(9, "Width").p(9),
            v_slider(10, "Lo-Cut").p(10),
            v_slider(11, "Hi-Cut").p(11),
            v_slider(12, "Mod Speed").p(12),
            v_slider(13, "Mod Amount").p(13),
        ])
        .show(Show::Tab(1)),
        hstack([
            knob(14, "Wet Level").p(14),
            knob(15, "Dry Level").p(15),
            knob(16, "Rev Pan").p(4),
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
pub const REFERENCE: Reference = Reference {
    size: [14.0, 4.99],
    settings: &[],
    cases: &[
        Case {
            name: "MAIN",
            rows: &[
                TABS,
                row(2, Widget::Knob, "Decay", [0.0, 1.25, 2.8, 3.74]).p(1),
                row(3, Widget::Knob, "High Damp", [2.8, 1.25, 2.8, 3.74]).p(2),
                row(4, Widget::Knob, "Mix", [5.6, 1.25, 2.8, 3.74]).p(3),
                row(5, Widget::Knob, "Early Refl.", [8.4, 1.25, 2.8, 3.74]).p(5),
                row(6, Widget::Knob, "Room Size", [11.2, 1.25, 2.8, 3.74]).p(7),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "TUNE",
            tab: 1,
            rows: &[
                TABS,
                row(7, Widget::VSlider, "Predelay", [0.0, 1.25, 2.0, 3.74]).p(6),
                row(8, Widget::VSlider, "Diffusion", [2.0, 1.25, 2.0, 3.74]).p(8),
                row(9, Widget::VSlider, "Width", [4.0, 1.25, 2.0, 3.74]).p(9),
                row(10, Widget::VSlider, "Lo-Cut", [6.0, 1.25, 2.0, 3.74]).p(10),
                row(11, Widget::VSlider, "Hi-Cut", [8.0, 1.25, 2.0, 3.74]).p(11),
                row(12, Widget::VSlider, "Mod Speed", [10.0, 1.25, 2.0, 3.74]).p(12),
                row(13, Widget::VSlider, "Mod Amount", [12.0, 1.25, 2.0, 3.74]).p(13),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "MIXER",
            tab: 2,
            rows: &[
                TABS,
                row(14, Widget::Knob, "Wet Level", [0.0, 1.25, 4.66, 3.74]).p(14),
                row(15, Widget::Knob, "Dry Level", [4.66, 1.25, 4.67, 3.74]).p(15),
                row(16, Widget::Knob, "Rev Pan", [9.33, 1.25, 4.67, 3.74]).p(4),
            ],
            ..Case::DEFAULT
        },
    ],
};

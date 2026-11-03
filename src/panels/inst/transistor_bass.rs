//! The Transistor Bass panel (type 19), card 7.0 U. Three tabs; DISTORTION starts with the
//! no-label pill toggle p7.

use crate::panels::build::*;

pub fn panel() -> Option<Panel> {
    Some(Panel::new(vstack([
        tabs(0, "MAIN · DISTORTION · PULSE WIDTH").h(Size::U(1.25)),
        hstack([
            knob(11000, "Cutoff").p(2),
            knob(12000, "Reso").p(3),
            knob(13000, "Env Mod").p(4),
            knob(14000, "Decay").p(5),
            knob(15000, "Accent").p(6),
            v_slider(16000, "Waveform").p(1),
        ])
        .show(Show::Tab(0)),
        hstack([
            toggle(21000, "").p(7),
            knob(22000, "Drive HP").p(8),
            knob(23000, "Drive").p(9),
            knob(24000, "Drive Vol.").p(10),
        ])
        .show(Show::Tab(1)),
        hstack([
            v_slider(31000, "PW").p(11),
            v_slider(32000, "LFO Speed").p(12),
            v_slider(33000, "LFO->PW").p(13),
        ])
        .show(Show::Tab(2)),
    ])))
}

#[cfg(test)]
const STRIP: Row = row(
    0,
    Widget::Tabs,
    "MAIN · DISTORTION · PULSE WIDTH",
    [0.0, 0.0, 16.96, 1.25],
)
.h(Size::U(1.25));

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [16.96, 4.99],
    settings: &[],
    cases: &[
        Case {
            name: "MAIN",
            tab: 0,
            rows: &[
                STRIP,
                row(11000, Widget::Knob, "Cutoff", [0.00, 1.25, 2.82, 3.74]).p(2),
                row(12000, Widget::Knob, "Reso", [2.82, 1.25, 2.83, 3.74]).p(3),
                row(13000, Widget::Knob, "Env Mod", [5.65, 1.25, 2.83, 3.74]).p(4),
                row(14000, Widget::Knob, "Decay", [8.48, 1.25, 2.82, 3.74]).p(5),
                row(15000, Widget::Knob, "Accent", [11.30, 1.25, 2.83, 3.74]).p(6),
                row(
                    16000,
                    Widget::VSlider,
                    "Waveform",
                    [14.13, 1.25, 2.82, 3.74],
                )
                .p(1),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "DISTORTION",
            tab: 1,
            rows: &[
                STRIP,
                row(21000, Widget::Toggle, "", [0.00, 1.25, 4.24, 3.74]).p(7),
                row(22000, Widget::Knob, "Drive HP", [4.24, 1.25, 4.24, 3.74]).p(8),
                row(23000, Widget::Knob, "Drive", [8.48, 1.25, 4.24, 3.74]).p(9),
                row(24000, Widget::Knob, "Drive Vol.", [12.72, 1.25, 4.24, 3.74]).p(10),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "PULSE WIDTH",
            tab: 2,
            rows: &[
                STRIP,
                row(31000, Widget::VSlider, "PW", [0.00, 1.25, 5.65, 3.74]).p(11),
                row(
                    32000,
                    Widget::VSlider,
                    "LFO Speed",
                    [5.65, 1.25, 5.65, 3.74],
                )
                .p(12),
                row(33000, Widget::VSlider, "LFO->PW", [11.30, 1.25, 5.66, 3.74]).p(13),
            ],
            ..Case::DEFAULT
        },
    ],
};

//! The Vocal Synth's clean-room formant and expression controls (type 53).

use crate::panels::build::*;

const TABS: &str = "VOICE · EXPRESSION";

pub fn panel() -> Option<Panel> {
    Some(Panel::new(vstack([
        tabs(0, TABS).h(Size::U(1.25)),
        hstack([
            knob(11000, "Gender / Formant").p(1),
            knob(12000, "Breathiness").p(2),
            knob(13000, "Brightness").p(3),
            spinner(14000, "Vowel").p(7),
        ])
        .show(Show::Tab(0)),
        hstack([
            knob(21000, "Portamento").p(4),
            knob(22000, "Vibrato Rate").p(5),
            knob(23000, "Vibrato Depth").p(6),
        ])
        .show(Show::Tab(1)),
    ])))
}

#[cfg(test)]
const STRIP: Row = row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25));

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [16.96, 6.49],
    settings: &[],
    cases: &[
        Case {
            name: "VOICE",
            tab: 0,
            rows: &[
                STRIP,
                row(11000, Widget::Knob, "Gender / Formant", [0.0, 1.25, 4.24, 5.24]).p(1),
                row(12000, Widget::Knob, "Breathiness", [4.24, 1.25, 4.24, 5.24]).p(2),
                row(13000, Widget::Knob, "Brightness", [8.48, 1.25, 4.24, 5.24]).p(3),
                row(14000, Widget::Spinner, "Vowel", [12.72, 1.25, 4.24, 5.24]).p(7),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "EXPRESSION",
            tab: 1,
            rows: &[
                STRIP,
                row(21000, Widget::Knob, "Portamento", [0.0, 1.25, 5.65, 5.24]).p(4),
                row(22000, Widget::Knob, "Vibrato Rate", [5.65, 1.25, 5.65, 5.24]).p(5),
                row(23000, Widget::Knob, "Vibrato Depth", [11.30, 1.25, 5.66, 5.24]).p(6),
            ],
            ..Case::DEFAULT
        },
    ],
};

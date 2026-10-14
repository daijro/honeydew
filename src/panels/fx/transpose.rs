//! The Transpose panel (ID 48), UIHeight 6 U.

use crate::panels::build::*;

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        knob(1, "Octave").p(1).h(Size::U(4.0)),
        knob(2, "Semitone").p(2).h(Size::U(4.0)),
    ])))
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 3.99],
    settings: &[],
    cases: &[Case {
        // 4 U knobs centred in the 3.99 U content: their y is -0.
        rows: &[
            row(1, Widget::Knob, "Octave", [0.0, 0.0, 7.0, 3.99])
                .p(1)
                .h(Size::U(4.0)),
            row(2, Widget::Knob, "Semitone", [7.0, 0.0, 7.0, 3.99])
                .p(2)
                .h(Size::U(4.0)),
        ],
        ..Case::DEFAULT
    }],
};

//! Randomizer (ID 49), UIHeight 6 U.

use crate::panels::build::*;

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        knob(1, "Pitch").p(1).h(Size::U(4.0)),
        knob(2, "Velocity").p(2).h(Size::U(4.0)),
        knob(3, "Pan").p(3).h(Size::U(4.0)),
        knob(4, "Gate").p(4).h(Size::U(4.0)),
    ])))
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 3.99],
    settings: &[],
    cases: &[Case {
        // 4 U knobs centred in the 3.99 U content: the table's y is -0.
        rows: &[
            row(1, Widget::Knob, "Pitch", [0.0, 0.0, 3.5, 3.99])
                .p(1)
                .h(Size::U(4.0)),
            row(2, Widget::Knob, "Velocity", [3.5, 0.0, 3.5, 3.99])
                .p(2)
                .h(Size::U(4.0)),
            row(3, Widget::Knob, "Pan", [7.0, 0.0, 3.5, 3.99])
                .p(3)
                .h(Size::U(4.0)),
            row(4, Widget::Knob, "Gate", [10.5, 0.0, 3.5, 3.99])
                .p(4)
                .h(Size::U(4.0)),
        ],
        ..Case::DEFAULT
    }],
};

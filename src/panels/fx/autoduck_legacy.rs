//! Autoduck (legacy) (ID 10), UIHeight 5.5 U.

use crate::panels::build::*;

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        knob(1, "Ducking").p(1),
        knob(2, "Attack").p(2),
        knob(3, "Curve").p(3),
    ])))
}

#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 3.49],
    settings: &[],
    cases: &[Case {
        rows: &[
            row(1, Widget::Knob, "Ducking", [0.0, 0.0, 4.66, 3.49]).p(1),
            row(2, Widget::Knob, "Attack", [4.66, 0.0, 4.67, 3.49]).p(2),
            row(3, Widget::Knob, "Curve", [9.33, 0.0, 4.67, 3.49]).p(3),
        ],
        ..Case::DEFAULT
    }],
};

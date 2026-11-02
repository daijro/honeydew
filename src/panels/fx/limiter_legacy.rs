//! Limiter (legacy) (ID 5), UIHeight 6 U.

use crate::panels::build::*;

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        knob(1, "Threshold").p(1),
        knob(2, "Release Time").p(2),
        knob(3, "Output Gain").p(3),
    ])))
}

#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 3.99],
    settings: &[],
    cases: &[Case {
        rows: &[
            row(1, Widget::Knob, "Threshold", [0.0, 0.0, 4.66, 3.99]).p(1),
            row(2, Widget::Knob, "Release Time", [4.66, 0.0, 4.67, 3.99]).p(2),
            row(3, Widget::Knob, "Output Gain", [9.33, 0.0, 4.67, 3.99]).p(3),
        ],
        ..Case::DEFAULT
    }],
};

//! Chorus (ID 11), UIHeight 5.5 U.

use crate::panels::build::*;

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        knob(1, "Rate").p(1),
        knob(2, "Depth").p(2),
        knob(3, "Stereo").p(3),
        knob(4, "Mix").p(4),
    ])))
}

#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 3.49],
    settings: &[],
    cases: &[Case {
        rows: &[
            row(1, Widget::Knob, "Rate", [0.0, 0.0, 3.5, 3.49]).p(1),
            row(2, Widget::Knob, "Depth", [3.5, 0.0, 3.5, 3.49]).p(2),
            row(3, Widget::Knob, "Stereo", [7.0, 0.0, 3.5, 3.49]).p(3),
            row(4, Widget::Knob, "Mix", [10.5, 0.0, 3.5, 3.49]).p(4),
        ],
        ..Case::DEFAULT
    }],
};

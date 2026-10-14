//! The Stereoizer panel (ID 15), UIHeight 5.5 U.

use crate::panels::build::*;

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        knob(1, "Strength").p(1),
        knob(2, "Timbre").p(2),
        knob(3, "Focus").p(3),
        knob(4, "L/R Delay").p(4),
        knob(5, "Pan").p(5),
    ])))
}

#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 3.49],
    settings: &[],
    cases: &[Case {
        rows: &[
            row(1, Widget::Knob, "Strength", [0.0, 0.0, 2.8, 3.49]).p(1),
            row(2, Widget::Knob, "Timbre", [2.8, 0.0, 2.8, 3.49]).p(2),
            row(3, Widget::Knob, "Focus", [5.6, 0.0, 2.8, 3.49]).p(3),
            row(4, Widget::Knob, "L/R Delay", [8.4, 0.0, 2.8, 3.49]).p(4),
            row(5, Widget::Knob, "Pan", [11.2, 0.0, 2.8, 3.49]).p(5),
        ],
        ..Case::DEFAULT
    }],
};

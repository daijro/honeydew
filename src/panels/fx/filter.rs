//! Filter (ID 9), UIHeight 6 U.

use crate::panels::build::*;

const KIND: &str = "Low Pass · Band Pass · High Pass";
const SLOPE: &str = "24 dB/Oct · 48 dB/Oct · 96 dB/Oct";

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        list(1, KIND, Bar::Right).p(3),
        knob(2, "Frequency").p(1),
        knob(3, "Resonance").p(2),
        list(4, SLOPE, Bar::Left).p(4),
    ])))
}

#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 3.99],
    settings: &[],
    cases: &[Case {
        rows: &[
            row(1, Widget::List(Bar::Right), KIND, [0.0, 0.0, 3.5, 3.99]).p(3),
            row(2, Widget::Knob, "Frequency", [3.5, 0.0, 3.5, 3.99]).p(1),
            row(3, Widget::Knob, "Resonance", [7.0, 0.0, 3.5, 3.99]).p(2),
            row(4, Widget::List(Bar::Left), SLOPE, [10.5, 0.0, 3.5, 3.99]).p(4),
        ],
        ..Case::DEFAULT
    }],
};

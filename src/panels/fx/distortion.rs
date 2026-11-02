//! Distortion (ID 6), UIHeight 5.5 U.

use crate::panels::build::*;

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        list(1, "Hard · Tube · Soft · Clip", Bar::Right).p(5),
        v_slider(2, "Bias").p(6),
        knob(3, "Drive").p(1),
        knob(4, "Stereo").p(4),
        v_slider(5, "BitCrush").p(2),
        knob(6, "Gain").p(3),
    ])))
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 3.49],
    settings: &[],
    cases: &[Case {
        rows: &[
            row(
                1,
                Widget::List(Bar::Right),
                "Hard · Tube · Soft · Clip",
                [0.0, 0.0, 2.33, 3.49],
            )
            .p(5),
            row(2, Widget::VSlider, "Bias", [2.33, 0.0, 2.33, 3.49]).p(6),
            row(3, Widget::Knob, "Drive", [4.66, 0.0, 2.34, 3.49]).p(1),
            row(4, Widget::Knob, "Stereo", [7.0, 0.0, 2.33, 3.49]).p(4),
            row(5, Widget::VSlider, "BitCrush", [9.33, 0.0, 2.33, 3.49]).p(2),
            row(6, Widget::Knob, "Gain", [11.66, 0.0, 2.34, 3.49]).p(3),
        ],
        ..Case::DEFAULT
    }],
};

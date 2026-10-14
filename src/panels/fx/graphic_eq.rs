//! Graphic EQ (ID 7), UIHeight 10 U.

use crate::panels::build::*;

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        v_slider(1, "80 Hz").p(1),
        v_slider(2, "160 Hz").p(2),
        v_slider(3, "375 Hz").p(3),
        v_slider(4, "750 Hz").p(4),
        v_slider(5, "1.5 kHz").p(5),
        v_slider(6, "3 kHz").p(6),
        v_slider(7, "6 kHz").p(7),
        v_slider(8, "12 kHz").p(8),
    ])))
}

#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 7.99],
    settings: &[],
    cases: &[Case {
        rows: &[
            row(1, Widget::VSlider, "80 Hz", [0.0, 0.0, 1.75, 7.99]).p(1),
            row(2, Widget::VSlider, "160 Hz", [1.75, 0.0, 1.75, 7.99]).p(2),
            row(3, Widget::VSlider, "375 Hz", [3.5, 0.0, 1.75, 7.99]).p(3),
            row(4, Widget::VSlider, "750 Hz", [5.25, 0.0, 1.75, 7.99]).p(4),
            row(5, Widget::VSlider, "1.5 kHz", [7.0, 0.0, 1.75, 7.99]).p(5),
            row(6, Widget::VSlider, "3 kHz", [8.75, 0.0, 1.75, 7.99]).p(6),
            row(7, Widget::VSlider, "6 kHz", [10.5, 0.0, 1.75, 7.99]).p(7),
            row(8, Widget::VSlider, "12 kHz", [12.25, 0.0, 1.75, 7.99]).p(8),
        ],
        ..Case::DEFAULT
    }],
};

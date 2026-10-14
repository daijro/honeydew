//! Flanger (ID 13), UIHeight 8 U.

use crate::panels::build::*;

/// `Sync` (p2) on, at any value but 0 as the engine syncs (as the toggle draws it): the `LFO Rate` spinner replaces
/// The `LFO Speed` knob.
const SYNC: Show = Show::Above(2, 0.0);

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        vstack([
            spinner(1, "LFO Rate").p(1).show(SYNC),
            knob(2, "LFO Speed").p(3).show(Show::Not(&SYNC)),
            spinner(3, "LFO Shape").p(4),
        ])
        .w(Size::Flex(1.5)),
        v_slider(4, "LFO Amount").p(5).value_text(),
        v_slider(5, "Feedback").p(7).value_text(),
        v_slider(6, "Delay").p(6).value_text(),
        v_slider(7, "Stereo").p(9).value_text(),
        v_slider(8, "Mix").p(10).value_text(),
        // Label-right toggles, not top-label ones.
        vstack([
            toggle_right(9, "Inv. Mix").p(11),
            toggle_right(10, "Inv. Fdbk").p(8),
            toggle_right(11, "Sync").p(2),
        ])
        .w(Size::Flex(2.0)),
    ])))
}

/// The columns right of the LFO slot, the same in both states.
#[cfg(test)]
#[rustfmt::skip]
const fn rows(lfo: Row) -> [Row; 10] {
    [
        lfo,
        row(3, Widget::Spinner, "LFO Shape", [0.0, 2.99, 2.47, 3.0]).p(4),
        row(4, Widget::VSlider, "LFO Amount", [2.47, 0.0, 1.64, 5.99]).p(5).value_text(),
        row(5, Widget::VSlider, "Feedback", [4.11, 0.0, 1.65, 5.99]).p(7).value_text(),
        row(6, Widget::VSlider, "Delay", [5.76, 0.0, 1.65, 5.99]).p(6).value_text(),
        row(7, Widget::VSlider, "Stereo", [7.41, 0.0, 1.64, 5.99]).p(9).value_text(),
        row(8, Widget::VSlider, "Mix", [9.05, 0.0, 1.65, 5.99]).p(10).value_text(),
        row(9, Widget::ToggleRight, "Inv. Mix", [10.7, 0.0, 3.3, 1.99]).p(11),
        row(10, Widget::ToggleRight, "Inv. Fdbk", [10.7, 1.99, 3.3, 2.0]).p(8),
        row(11, Widget::ToggleRight, "Sync", [10.7, 3.99, 3.3, 2.0]).p(2),
    ]
}

/// The table prints no rect for `LFO Rate` (hidden by default): with Sync on it takes the knob's slot.
#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 5.99],
    settings: &[],
    cases: &[
        Case {
            name: "Sync off",
            rows: &rows(row(2, Widget::Knob, "LFO Speed", [0.0, 0.0, 2.47, 2.99]).p(3)),
            ..Case::DEFAULT
        },
        Case {
            name: "Sync on",
            params: &[(2, 1.0)],
            rows: &rows(row(1, Widget::Spinner, "LFO Rate", [0.0, 0.0, 2.47, 2.99]).p(1)),
            ..Case::DEFAULT
        },
        // The engine syncs at any value but 0 (`v2 != 0`).
        Case {
            name: "Sync at 0.25",
            params: &[(2, 0.25)],
            rows: &rows(row(1, Widget::Spinner, "LFO Rate", [0.0, 0.0, 2.47, 2.99]).p(1)),
            ..Case::DEFAULT
        },
    ],
};

//! Phaser (ID 14), UIHeight 8 U.

use crate::panels::build::*;
use crate::params::module_spec;

/// `Sync` (p6, on by default), on at any value but 0 as the engine syncs (as the toggle draws it): the `LFO Rate`
/// spinner, off the `LFO Speed` knob.
const SYNC: Show = Show::Above(6, 0.0);

/// The `Sync` toggle's On/Off text, on where [`SYNC`] is: its automation lane's.
fn sync_text(v: f32) -> String {
    module_spec(14).map_or_else(String::new, |m| m.params[6].automation_text(v))
}

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        vstack([
            spinner(1, "LFO Rate").p(1).show(SYNC),
            knob(2, "LFO Speed").p(7).show(Show::Not(&SYNC)),
            spinner(3, "LFO Shape").p(8),
        ]),
        toggle(4, "Sync")
            .p(6)
            .value_text()
            .value_fn(sync_text)
            .h(Size::Percent(50.0)),
        v_slider(5, "LFO Amount").p(2).value_text(),
        v_slider(6, "Feedback").p(3).value_text(),
        v_slider(7, "Frequency").p(4).value_text(),
        v_slider(8, "Stereo").p(9).value_text(),
        v_slider(9, "Mix").p(5).value_text(),
    ])))
}

/// The columns right of the LFO slot, the same in both states.
#[cfg(test)]
#[rustfmt::skip]
const fn rows(lfo: Row, sync: &'static str) -> [Row; 8] {
    [
        lfo,
        row(3, Widget::Spinner, "LFO Shape", [0.0, 2.99, 2.0, 3.0]).p(8),
        row(4, Widget::Toggle, "Sync", [2.0, 1.49, 2.0, 3.0]).p(6).value_text().value(sync).h(Size::Percent(50.0)),
        row(5, Widget::VSlider, "LFO Amount", [4.0, 0.0, 2.0, 5.99]).p(2).value_text(),
        row(6, Widget::VSlider, "Feedback", [6.0, 0.0, 2.0, 5.99]).p(3).value_text(),
        row(7, Widget::VSlider, "Frequency", [8.0, 0.0, 2.0, 5.99]).p(4).value_text(),
        row(8, Widget::VSlider, "Stereo", [10.0, 0.0, 2.0, 5.99]).p(9).value_text(),
        row(9, Widget::VSlider, "Mix", [12.0, 0.0, 2.0, 5.99]).p(5).value_text(),
    ]
}

/// The table prints no rect for `LFO Speed` (hidden by default): with Sync off it takes the spinner's slot.
#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 5.99],
    settings: &[],
    cases: &[
        Case {
            name: "Sync on",
            rows: &rows(row(1, Widget::Spinner, "LFO Rate", [0.0, 0.0, 2.0, 2.99]).p(1), "On"),
            ..Case::DEFAULT
        },
        Case {
            name: "Sync off",
            params: &[(6, 0.0)],
            rows: &rows(row(2, Widget::Knob, "LFO Speed", [0.0, 0.0, 2.0, 2.99]).p(7), "Off"),
            ..Case::DEFAULT
        },
        // The engine syncs at any value but 0 (`v6 != 0`), and so do the swap and the text.
        Case {
            name: "Sync at 0.25",
            params: &[(6, 0.25)],
            rows: &rows(row(1, Widget::Spinner, "LFO Rate", [0.0, 0.0, 2.0, 2.99]).p(1), "On"),
            ..Case::DEFAULT
        },
    ],
};

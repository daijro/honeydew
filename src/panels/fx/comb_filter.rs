//! Comb Filter (ID 40), UIHeight 7 U.

use crate::panels::build::*;
#[cfg(test)]
use crate::params::menu_value;

/// `Tracking` (p1) on `Manual`: no tracking.
const MANUAL: Show = Show::Item(1, 0);
/// `Snap` (p7) on at any value but 0, as the engine snaps and the button draws it.
const SNAPS: Show = Show::Above(7, 0.0);
const SNAP_ON: Show = Show::All(&[MANUAL, SNAPS]);
const SNAP_OFF: Show = Show::All(&[MANUAL, Show::Not(&SNAPS)]);

/// `Snap`: off a `background` disc with a `text` label, on a `text` disc with a `background` label.
const SNAP_COLORS: ButtonColors = ButtonColors {
    fill_on: Ink::Text,
    fill_off: Ink::Background,
    text_on: Ink::Background,
    text_off: Some(Ink::Text),
};

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        spinner(1, "Tracking").p(1),
        // The tracked pitch replaces `?` once one is detected: module state, with the display trait.
        text(2, "?")
            .font(FontSize::Pt18)
            .ink(Ink::Text)
            .show(Show::Not(&MANUAL)),
        button(3, "Snap", true)
            .p(7)
            .colors(SNAP_COLORS)
            .font(FontSize::Pt8)
            .show(MANUAL),
        hstack([
            v_slider(4, "Octave")
                .p(5)
                .value_text()
                .show(Show::Not(&MANUAL)),
            v_slider(5, "Octave").p(10).value_text().show(SNAP_ON),
            v_slider(6, "Note").p(11).value_text().show(SNAP_ON),
            v_slider(7, "Freq").p(2).value_text().show(SNAP_OFF),
            spinner(8, "Polarity").p(8),
        ])
        .w(Size::Flex(2.5)),
        vstack([
            hstack([knob(9, "Asym").p(9), knob(10, "Feedback").p(3)]),
            hstack([knob(11, "Smoothing").p(6), knob(12, "Mix").p(4)]),
        ])
        .w(Size::Flex(2.0)),
    ])))
}

#[cfg(test)]
const TRACKING: Row = row(1, Widget::Spinner, "Tracking", [0.0, 0.0, 2.15, 4.99]).p(1);
#[cfg(test)]
const POLARITY: Row = row(8, Widget::Spinner, "Polarity", [7.89, 0.0, 1.8, 4.99]).p(8);
/// `Polarity` beside a single slider (half of the 5.38 U slider row).
#[cfg(test)]
const POLARITY_WIDE: Row = row(8, Widget::Spinner, "Polarity", [7.0, 0.0, 2.69, 4.99]).p(8);
#[cfg(test)]
const KNOBS: [Row; 4] = [
    row(9, Widget::Knob, "Asym", [9.69, 0.0, 2.15, 2.49]).p(9),
    row(10, Widget::Knob, "Feedback", [11.84, 0.0, 2.16, 2.49]).p(3),
    row(11, Widget::Knob, "Smoothing", [9.69, 2.49, 2.15, 2.5]).p(6),
    row(12, Widget::Knob, "Mix", [11.84, 2.49, 2.16, 2.5]).p(4),
];
/// Either tracking mode: the `?` text in Snap's slot, the `Octave` (p5) slider and `Polarity`.
#[cfg(test)]
const TRACKED: [Row; 8] = [
    TRACKING,
    row(2, Widget::Text, "?", [2.15, 0.0, 2.15, 4.99])
        .font(FontSize::Pt18)
        .ink(Ink::Text),
    row(4, Widget::VSlider, "Octave", [4.3, 0.0, 2.7, 4.99])
        .p(5)
        .value_text(),
    POLARITY_WIDE,
    KNOBS[0],
    KNOBS[1],
    KNOBS[2],
    KNOBS[3],
];
#[cfg(test)]
const SNAP: Row = row(
    3,
    Widget::Button { round: true },
    "Snap",
    [2.15, 0.0, 2.15, 4.99],
)
.p(7)
.font(FontSize::Pt8)
.colors(ButtonColors {
    fill_on: Ink::Text,
    fill_off: Ink::Background,
    text_on: Ink::Background,
    text_off: Some(Ink::Text),
});

/// Manual with Snap on: `Octave` (p10) and `Note` (p11).
#[cfg(test)]
const SNAPPED: [Row; 9] = [
    TRACKING,
    SNAP,
    row(5, Widget::VSlider, "Octave", [4.3, 0.0, 1.8, 4.99])
        .p(10)
        .value_text(),
    row(6, Widget::VSlider, "Note", [6.1, 0.0, 1.79, 4.99])
        .p(11)
        .value_text(),
    POLARITY,
    KNOBS[0],
    KNOBS[1],
    KNOBS[2],
    KNOBS[3],
];

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 4.99],
    settings: &[],
    cases: &[
        Case {
            name: "Manual, Snap on",
            rows: &SNAPPED,
            ..Case::DEFAULT
        },
        // The engine snaps at any value but 0 (`v7 != 0`).
        Case {
            name: "Manual, Snap at 0.25",
            params: &[(7, 0.25)],
            rows: &SNAPPED,
            ..Case::DEFAULT
        },
        Case {
            name: "Manual, Snap off",
            params: &[(7, 0.0)],
            rows: &[
                TRACKING,
                SNAP,
                // `Freq` takes the slots of `Octave` and `Note`.
                row(7, Widget::VSlider, "Freq", [4.3, 0.0, 2.7, 4.99])
                    .p(2)
                    .value_text(),
                POLARITY_WIDE,
                KNOBS[0],
                KNOBS[1],
                KNOBS[2],
                KNOBS[3],
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "MIDI Note",
            params: &[(1, menu_value(1, 3))],
            rows: &TRACKED,
            ..Case::DEFAULT
        },
        Case {
            name: "Audio Pitch",
            params: &[(1, menu_value(2, 3))],
            rows: &TRACKED,
            ..Case::DEFAULT
        },
    ],
};

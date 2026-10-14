//! The DW Sampler panel (type 1), card 7.5 U.
//! Ids: tab · 10000 + the control's `#n` number in four digits (SAMPLE `1.8` is 11800).

use crate::panels::build::*;
#[cfg(test)]
use crate::params::menu_value;

/// LFO mode (p29) on `Tmp Sync`: the `Rate` spinner (p30) takes the `Rate` knob's (p28) slot.
const TMP_SYNC: Show = Show::Item(29, 2);

/// The pitch-bend range `k` semitones, stored as k/12 (the `PB Range` knob has no param).
fn semitones(v: f32) -> String {
    ((v * 12.0).round() as i32).clamp(1, 12).to_string()
}

pub fn panel() -> Option<Panel> {
    Some(Panel::new(vstack([
        tabs(0, "SAMPLE · FILTER · FLT ENV · FRQ ENV · LFO").h(Size::U(1.25)),
        vstack([
            hstack([
                knob(11100, "Pitch").p(3),
                knob(11200, "Fine Tune").p(4),
                v_slider(11300, "Attack").p(1),
                v_slider(11400, "Decay").p(5),
                v_slider(11500, "Sustain").p(6),
                v_slider(11600, "Release").p(2),
                v_slider(11700, "Slide Time").p(7),
                knob(11800, "PB Range")
                    .default(2.0 / 12.0)
                    .value_fn(semitones),
            ]),
            hstack([
                text_toggle(12100, "Reverse").p(8),
                text_toggle(12200, "Loop").p(9),
                text_toggle(12300, "Legato").p(10),
                text_toggle(12400, "Mono").p(11),
            ])
            .h(Size::U(1.0))
            .mr(2.0)
            .ml(2.0),
        ])
        .show(Show::Tab(0)),
        hstack([
            knob(21000, "Cutoff").p(12),
            knob(22000, "Resonance").p(13),
            knob(23000, "Kbd Trk").p(14),
            knob(24000, "Vel Trk").p(15),
            spinner(25000, "Filter").p(16),
        ])
        .show(Show::Tab(1)),
        hstack([
            knob(31000, "Cutoff").p(12),
            v_slider(32000, "Attack").p(17),
            v_slider(33000, "Decay").p(18),
            v_slider(34000, "Sustain").p(19),
            v_slider(35000, "Release").p(20),
            v_slider(36000, "Amount").p(21),
        ])
        .show(Show::Tab(2)),
        hstack([
            v_slider(41000, "Attack").p(22),
            v_slider(42000, "Decay").p(23),
            v_slider(43000, "Sustain").p(24),
            v_slider(44000, "Release").p(25),
            v_slider(45000, "Amount").p(26),
        ])
        .show(Show::Tab(3)),
        hstack([
            list(51000, "Hertz · One Shot · Tmp Sync", Bar::Right).p(29),
            knob(52000, "Rate").p(28).show(Show::Not(&TMP_SYNC)),
            spinner(53000, "Rate")
                .p(30)
                .items_per_column(5)
                .show(TMP_SYNC),
            spinner(54000, "Shape").p(27),
            knob(55000, "Attack").p(33),
            knob(56000, "Amount").p(31),
            spinner(57000, "Destination").p(32),
        ])
        .show(Show::Tab(4)),
    ])))
}

#[cfg(test)]
const STRIP: Row = row(
    0,
    Widget::Tabs,
    "SAMPLE · FILTER · FLT ENV · FRQ ENV · LFO",
    [0.0, 0.0, 16.96, 1.25],
)
.h(Size::U(1.25));

#[cfg(test)]
const LFO: &str = "Hertz · One Shot · Tmp Sync";

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [16.96, 5.49],
    settings: &[(11800, 2.0 / 12.0)],
    cases: &[
        Case {
            name: "SAMPLE",
            tab: 0,
            rows: &[
                STRIP,
                row(11100, Widget::Knob, "Pitch", [0.00, 1.25, 2.12, 3.24]).p(3),
                row(11200, Widget::Knob, "Fine Tune", [2.12, 1.25, 2.12, 3.24]).p(4),
                row(11300, Widget::VSlider, "Attack", [4.24, 1.25, 2.12, 3.24]).p(1),
                row(11400, Widget::VSlider, "Decay", [6.36, 1.25, 2.12, 3.24]).p(5),
                row(11500, Widget::VSlider, "Sustain", [8.48, 1.25, 2.12, 3.24]).p(6),
                row(11600, Widget::VSlider, "Release", [10.60, 1.25, 2.12, 3.24]).p(2),
                row(
                    11700,
                    Widget::VSlider,
                    "Slide Time",
                    [12.72, 1.25, 2.12, 3.24],
                )
                .p(7),
                row(11800, Widget::Knob, "PB Range", [14.84, 1.25, 2.12, 3.24]).value("2"),
                row(
                    12100,
                    Widget::TextToggle,
                    "Reverse",
                    [2.00, 4.49, 3.24, 1.00],
                )
                .p(8),
                row(12200, Widget::TextToggle, "Loop", [5.24, 4.49, 3.24, 1.00]).p(9),
                row(
                    12300,
                    Widget::TextToggle,
                    "Legato",
                    [8.48, 4.49, 3.24, 1.00],
                )
                .p(10),
                row(12400, Widget::TextToggle, "Mono", [11.72, 4.49, 3.24, 1.00]).p(11),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FILTER",
            tab: 1,
            rows: &[
                STRIP,
                row(21000, Widget::Knob, "Cutoff", [0.00, 1.25, 3.39, 4.24]).p(12),
                row(22000, Widget::Knob, "Resonance", [3.39, 1.25, 3.39, 4.24]).p(13),
                row(23000, Widget::Knob, "Kbd Trk", [6.78, 1.25, 3.39, 4.24]).p(14),
                row(24000, Widget::Knob, "Vel Trk", [10.17, 1.25, 3.39, 4.24]).p(15),
                row(25000, Widget::Spinner, "Filter", [13.56, 1.25, 3.40, 4.24]).p(16),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FLT ENV",
            tab: 2,
            rows: &[
                STRIP,
                row(31000, Widget::Knob, "Cutoff", [0.00, 1.25, 2.82, 4.24]).p(12),
                row(32000, Widget::VSlider, "Attack", [2.82, 1.25, 2.83, 4.24]).p(17),
                row(33000, Widget::VSlider, "Decay", [5.65, 1.25, 2.83, 4.24]).p(18),
                row(34000, Widget::VSlider, "Sustain", [8.48, 1.25, 2.82, 4.24]).p(19),
                row(35000, Widget::VSlider, "Release", [11.30, 1.25, 2.83, 4.24]).p(20),
                row(36000, Widget::VSlider, "Amount", [14.13, 1.25, 2.82, 4.24]).p(21),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FRQ ENV",
            tab: 3,
            rows: &[
                STRIP,
                row(41000, Widget::VSlider, "Attack", [0.00, 1.25, 3.39, 4.24]).p(22),
                row(42000, Widget::VSlider, "Decay", [3.39, 1.25, 3.39, 4.24]).p(23),
                row(43000, Widget::VSlider, "Sustain", [6.78, 1.25, 3.39, 4.24]).p(24),
                row(44000, Widget::VSlider, "Release", [10.17, 1.25, 3.39, 4.24]).p(25),
                row(45000, Widget::VSlider, "Amount", [13.56, 1.25, 3.40, 4.24]).p(26),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "LFO Hertz",
            tab: 4,
            rows: &[
                STRIP,
                row(
                    51000,
                    Widget::List(Bar::Right),
                    LFO,
                    [0.00, 1.25, 2.82, 4.24],
                )
                .p(29),
                row(52000, Widget::Knob, "Rate", [2.82, 1.25, 2.83, 4.24]).p(28),
                row(54000, Widget::Spinner, "Shape", [5.65, 1.25, 2.83, 4.24]).p(27),
                row(55000, Widget::Knob, "Attack", [8.48, 1.25, 2.82, 4.24]).p(33),
                row(56000, Widget::Knob, "Amount", [11.30, 1.25, 2.83, 4.24]).p(31),
                row(
                    57000,
                    Widget::Spinner,
                    "Destination",
                    [14.13, 1.25, 2.82, 4.24],
                )
                .p(32),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "LFO One Shot",
            tab: 4,
            params: &[(29, menu_value(1, 3))],
            rows: &[
                STRIP,
                row(
                    51000,
                    Widget::List(Bar::Right),
                    LFO,
                    [0.00, 1.25, 2.82, 4.24],
                )
                .p(29),
                row(52000, Widget::Knob, "Rate", [2.82, 1.25, 2.83, 4.24]).p(28),
                row(54000, Widget::Spinner, "Shape", [5.65, 1.25, 2.83, 4.24]).p(27),
                row(55000, Widget::Knob, "Attack", [8.48, 1.25, 2.82, 4.24]).p(33),
                row(56000, Widget::Knob, "Amount", [11.30, 1.25, 2.83, 4.24]).p(31),
                row(
                    57000,
                    Widget::Spinner,
                    "Destination",
                    [14.13, 1.25, 2.82, 4.24],
                )
                .p(32),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "LFO Tmp Sync",
            tab: 4,
            params: &[(29, menu_value(2, 3))],
            rows: &[
                STRIP,
                row(
                    51000,
                    Widget::List(Bar::Right),
                    LFO,
                    [0.00, 1.25, 2.82, 4.24],
                )
                .p(29),
                row(53000, Widget::Spinner, "Rate", [2.82, 1.25, 2.83, 4.24])
                    .p(30)
                    .items_per_column(5),
                row(54000, Widget::Spinner, "Shape", [5.65, 1.25, 2.83, 4.24]).p(27),
                row(55000, Widget::Knob, "Attack", [8.48, 1.25, 2.82, 4.24]).p(33),
                row(56000, Widget::Knob, "Amount", [11.30, 1.25, 2.83, 4.24]).p(31),
                row(
                    57000,
                    Widget::Spinner,
                    "Destination",
                    [14.13, 1.25, 2.82, 4.24],
                )
                .p(32),
            ],
            ..Case::DEFAULT
        },
    ],
};

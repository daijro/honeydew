//! The SuperSaw panel (type 23), card 9.0 U.
//! Ids: tab · 10000 + the control's `#n` number in four digits (OSC `1.3` is 11300).

use crate::panels::build::*;
#[cfg(test)]
use crate::params::menu_value;

/// The card's construction state: the mode list has no selection and neither oscillator row is shown until the
/// first update. The card wiring never sets the flag.
const UPDATED: Show = Show::Flag(flag::CONSTRUCTING, 0);
/// The oscillator mode (p10).
const SUPERSAW: Show = Show::All(&[Show::Item(10, 0), UPDATED]);
const SUPERSYNC: Show = Show::All(&[Show::Item(10, 1), UPDATED]);
/// LFO mode (p41) on `Tmp Sync`: the `Rate` spinner (p42) takes the `Rate` knob's (p40) slot.
const TMP_SYNC: Show = Show::Item(41, 2);

pub fn panel() -> Option<Panel> {
    Some(Panel::new(vstack([
        tabs(0, "OSC · FILTER · LFO · LEVEL · MOD · VOICES · FX").h(Size::U(1.25)),
        vstack([
            hstack([
                toggle_right(11100, "Pulse Mode").p(2).show(SUPERSAW),
                spacer(11200).show(SUPERSYNC),
                list(11300, "SUPERSAW · SUPERSYNC", Bar::Bottom)
                    .p(10)
                    .w(Size::Percent(33.0))
                    .mr(2.0)
                    .ml(2.0),
                toggle_right(11400, "Retrig").p(8).show(SUPERSAW),
                toggle_right(11500, "Flip").p(18).show(SUPERSYNC),
            ])
            .h(Size::U(1.5)),
            hstack([
                v_slider(12100, "Pulse Width").p(1),
                v_slider(12200, "Sub Osc").p(3),
                spinner(12300, "Unison").p(4),
                v_slider(12400, "Detune").p(5),
                v_slider(12500, "Unison Pan").p(6),
                v_slider(12600, "Dtn. Spread").p(7),
                v_slider(12700, "Unison Phase").p(9),
            ])
            .show(SUPERSAW),
            hstack([
                v_slider(13100, "Sync").p(11),
                v_slider(13200, "Pulse Width").p(12),
                v_slider(13300, "Attack").p(13),
                v_slider(13400, "Decay").p(14),
                v_slider(13500, "Sustain").p(15),
                v_slider(13600, "Release").p(16),
                v_slider(13700, "Amount").p(17),
            ])
            .show(SUPERSYNC),
        ])
        .show(Show::Tab(0)),
        hstack([
            v_slider(21000, "Cutoff").p(19),
            v_slider(22000, "Resonance").p(20),
            v_slider(23000, "Kbd Trk").p(21),
            v_slider(24000, "Vel Trk").p(27),
            v_slider(25000, "Attack").p(22),
            v_slider(26000, "Decay").p(23),
            v_slider(27000, "Sustain").p(24),
            v_slider(28000, "Release").p(25),
            v_slider(29000, "Amount").p(26),
        ])
        .show(Show::Tab(1)),
        hstack([
            list(31000, "Hertz · One Shot · Tmp Sync", Bar::Right).p(41),
            knob(32000, "Rate").p(40).show(Show::Not(&TMP_SYNC)),
            spinner(33000, "Rate")
                .p(42)
                .items_per_column(5)
                .show(TMP_SYNC),
            spinner(34000, "Shape").p(39),
            knob(35000, "Attack").p(46),
            knob(36000, "Amount").p(44),
            spinner(37000, "Destination").p(45),
        ])
        .show(Show::Tab(2)),
        hstack([
            knob(41000, "Output Level").p(32).w(Size::Flex(1.25)),
            knob(42000, "Panning").p(33),
            v_slider(43000, "Attack").p(28).w(Size::Flex(0.5)),
            v_slider(44000, "Decay").p(29).w(Size::Flex(0.5)),
            v_slider(45000, "Sustain").p(30).w(Size::Flex(0.5)),
            v_slider(46000, "Release").p(31).w(Size::Flex(0.5)),
        ])
        .show(Show::Tab(3)),
        hstack([
            spinner(51000, "PW Range").p(37),
            knob(52000, "ModWheel Amount").p(58),
            spinner(53000, "ModWheel Dest").p(59),
        ])
        .show(Show::Tab(4)),
        hstack([
            spinner(61000, "Poly").p(51),
            knob(62000, "Transp").p(36),
            v_slider(63000, "Slide").p(35),
            vstack([
                toggle_right(64100, "Legato").p(34),
                toggle_right(64200, "Constant").p(38),
                toggle_right(64300, "Mono Mode").p(50),
            ])
            .w(Size::Flex(2.0)),
            spinner(65000, "Arp Rate").p(47),
            spinner(66000, "Arp Range").p(48),
            spinner(67000, "Arp Mode").p(49),
        ])
        .show(Show::Tab(5)),
        hstack([
            knob(71000, "Distortion").p(52),
            knob(72000, "Gain").p(53),
            spinner(73000, "Delay Time").p(54),
            v_slider(74000, "Delay Fdbk").p(55),
            v_slider(75000, "Delay Filter").p(57),
            knob(76000, "Delay Mix").p(56),
        ])
        .show(Show::Tab(6)),
    ])))
}

#[cfg(test)]
const STRIP: Row = row(
    0,
    Widget::Tabs,
    "OSC · FILTER · LFO · LEVEL · MOD · VOICES · FX",
    [0.0, 0.0, 16.96, 1.25],
)
.h(Size::U(1.25));

#[cfg(test)]
const MODE: &str = "SUPERSAW · SUPERSYNC";

#[cfg(test)]
const LFO: &str = "Hertz · One Shot · Tmp Sync";

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [16.96, 6.99],
    settings: &[],
    cases: &[
        Case {
            name: "OSC, no mode selected",
            tab: 0,
            flags: &[(flag::CONSTRUCTING, 1)],
            rows: &[
                STRIP,
                row(
                    11300,
                    Widget::List(Bar::Bottom),
                    MODE,
                    [2.00, 1.25, 5.59, 1.50],
                )
                .p(10)
                .w(Size::Percent(33.0))
                .margins(0.0, 2.0, 0.0, 2.0),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "OSC SUPERSAW",
            tab: 0,
            rows: &[
                STRIP,
                row(
                    11100,
                    Widget::ToggleRight,
                    "Pulse Mode",
                    [0.00, 1.25, 3.68, 1.50],
                )
                .p(2),
                row(
                    11300,
                    Widget::List(Bar::Bottom),
                    MODE,
                    [5.68, 1.25, 5.59, 1.50],
                )
                .p(10)
                .w(Size::Percent(33.0))
                .margins(0.0, 2.0, 0.0, 2.0),
                row(
                    11400,
                    Widget::ToggleRight,
                    "Retrig",
                    [13.27, 1.25, 3.69, 1.50],
                )
                .p(8),
                row(
                    12100,
                    Widget::VSlider,
                    "Pulse Width",
                    [0.00, 2.75, 2.42, 4.24],
                )
                .p(1),
                row(12200, Widget::VSlider, "Sub Osc", [2.42, 2.75, 2.42, 4.24]).p(3),
                row(12300, Widget::Spinner, "Unison", [4.84, 2.75, 2.42, 4.24]).p(4),
                row(12400, Widget::VSlider, "Detune", [7.26, 2.75, 2.43, 4.24]).p(5),
                row(
                    12500,
                    Widget::VSlider,
                    "Unison Pan",
                    [9.69, 2.75, 2.42, 4.24],
                )
                .p(6),
                row(
                    12600,
                    Widget::VSlider,
                    "Dtn. Spread",
                    [12.11, 2.75, 2.42, 4.24],
                )
                .p(7),
                row(
                    12700,
                    Widget::VSlider,
                    "Unison Phase",
                    [14.53, 2.75, 2.43, 4.24],
                )
                .p(9),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "OSC SUPERSYNC",
            tab: 0,
            params: &[(10, menu_value(1, 2))],
            rows: &[
                STRIP,
                row(11200, Widget::Spacer, "", [0.00, 1.25, 3.68, 1.50]),
                row(
                    11300,
                    Widget::List(Bar::Bottom),
                    MODE,
                    [5.68, 1.25, 5.59, 1.50],
                )
                .p(10)
                .w(Size::Percent(33.0))
                .margins(0.0, 2.0, 0.0, 2.0),
                row(
                    11500,
                    Widget::ToggleRight,
                    "Flip",
                    [13.27, 1.25, 3.69, 1.50],
                )
                .p(18),
                row(13100, Widget::VSlider, "Sync", [0.00, 2.75, 2.42, 4.24]).p(11),
                row(
                    13200,
                    Widget::VSlider,
                    "Pulse Width",
                    [2.42, 2.75, 2.42, 4.24],
                )
                .p(12),
                row(13300, Widget::VSlider, "Attack", [4.84, 2.75, 2.42, 4.24]).p(13),
                row(13400, Widget::VSlider, "Decay", [7.26, 2.75, 2.43, 4.24]).p(14),
                row(13500, Widget::VSlider, "Sustain", [9.69, 2.75, 2.42, 4.24]).p(15),
                row(13600, Widget::VSlider, "Release", [12.11, 2.75, 2.42, 4.24]).p(16),
                row(13700, Widget::VSlider, "Amount", [14.53, 2.75, 2.43, 4.24]).p(17),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FILTER",
            tab: 1,
            rows: &[
                STRIP,
                row(21000, Widget::VSlider, "Cutoff", [0.00, 1.25, 1.88, 5.74]).p(19),
                row(
                    22000,
                    Widget::VSlider,
                    "Resonance",
                    [1.88, 1.25, 1.88, 5.74],
                )
                .p(20),
                row(23000, Widget::VSlider, "Kbd Trk", [3.76, 1.25, 1.89, 5.74]).p(21),
                row(24000, Widget::VSlider, "Vel Trk", [5.65, 1.25, 1.88, 5.74]).p(27),
                row(25000, Widget::VSlider, "Attack", [7.53, 1.25, 1.89, 5.74]).p(22),
                row(26000, Widget::VSlider, "Decay", [9.42, 1.25, 1.88, 5.74]).p(23),
                row(27000, Widget::VSlider, "Sustain", [11.30, 1.25, 1.89, 5.74]).p(24),
                row(28000, Widget::VSlider, "Release", [13.19, 1.25, 1.88, 5.74]).p(25),
                row(29000, Widget::VSlider, "Amount", [15.07, 1.25, 1.89, 5.74]).p(26),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "LFO Hertz",
            tab: 2,
            rows: &[
                STRIP,
                row(
                    31000,
                    Widget::List(Bar::Right),
                    LFO,
                    [0.00, 1.25, 2.82, 5.74],
                )
                .p(41),
                row(32000, Widget::Knob, "Rate", [2.82, 1.25, 2.83, 5.74]).p(40),
                row(34000, Widget::Spinner, "Shape", [5.65, 1.25, 2.83, 5.74]).p(39),
                row(35000, Widget::Knob, "Attack", [8.48, 1.25, 2.82, 5.74]).p(46),
                row(36000, Widget::Knob, "Amount", [11.30, 1.25, 2.83, 5.74]).p(44),
                row(
                    37000,
                    Widget::Spinner,
                    "Destination",
                    [14.13, 1.25, 2.82, 5.74],
                )
                .p(45),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "LFO One Shot",
            tab: 2,
            params: &[(41, menu_value(1, 3))],
            rows: &[
                STRIP,
                row(
                    31000,
                    Widget::List(Bar::Right),
                    LFO,
                    [0.00, 1.25, 2.82, 5.74],
                )
                .p(41),
                row(32000, Widget::Knob, "Rate", [2.82, 1.25, 2.83, 5.74]).p(40),
                row(34000, Widget::Spinner, "Shape", [5.65, 1.25, 2.83, 5.74]).p(39),
                row(35000, Widget::Knob, "Attack", [8.48, 1.25, 2.82, 5.74]).p(46),
                row(36000, Widget::Knob, "Amount", [11.30, 1.25, 2.83, 5.74]).p(44),
                row(
                    37000,
                    Widget::Spinner,
                    "Destination",
                    [14.13, 1.25, 2.82, 5.74],
                )
                .p(45),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "LFO Tmp Sync",
            tab: 2,
            params: &[(41, menu_value(2, 3))],
            rows: &[
                STRIP,
                row(
                    31000,
                    Widget::List(Bar::Right),
                    LFO,
                    [0.00, 1.25, 2.82, 5.74],
                )
                .p(41),
                row(33000, Widget::Spinner, "Rate", [2.82, 1.25, 2.83, 5.74])
                    .p(42)
                    .items_per_column(5),
                row(34000, Widget::Spinner, "Shape", [5.65, 1.25, 2.83, 5.74]).p(39),
                row(35000, Widget::Knob, "Attack", [8.48, 1.25, 2.82, 5.74]).p(46),
                row(36000, Widget::Knob, "Amount", [11.30, 1.25, 2.83, 5.74]).p(44),
                row(
                    37000,
                    Widget::Spinner,
                    "Destination",
                    [14.13, 1.25, 2.82, 5.74],
                )
                .p(45),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "LEVEL",
            tab: 3,
            rows: &[
                STRIP,
                row(
                    41000,
                    Widget::Knob,
                    "Output Level",
                    [0.00, 1.25, 4.98, 5.74],
                )
                .p(32)
                .w(Size::Flex(1.25)),
                row(42000, Widget::Knob, "Panning", [4.98, 1.25, 3.99, 5.74]).p(33),
                row(43000, Widget::VSlider, "Attack", [8.97, 1.25, 2.00, 5.74])
                    .p(28)
                    .w(Size::Flex(0.5)),
                row(44000, Widget::VSlider, "Decay", [10.97, 1.25, 1.99, 5.74])
                    .p(29)
                    .w(Size::Flex(0.5)),
                row(45000, Widget::VSlider, "Sustain", [12.96, 1.25, 2.00, 5.74])
                    .p(30)
                    .w(Size::Flex(0.5)),
                row(46000, Widget::VSlider, "Release", [14.96, 1.25, 2.00, 5.74])
                    .p(31)
                    .w(Size::Flex(0.5)),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "MOD",
            tab: 4,
            rows: &[
                STRIP,
                row(51000, Widget::Spinner, "PW Range", [0.00, 1.25, 5.65, 5.74]).p(37),
                row(
                    52000,
                    Widget::Knob,
                    "ModWheel Amount",
                    [5.65, 1.25, 5.65, 5.74],
                )
                .p(58),
                row(
                    53000,
                    Widget::Spinner,
                    "ModWheel Dest",
                    [11.30, 1.25, 5.66, 5.74],
                )
                .p(59),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "VOICES",
            tab: 5,
            rows: &[
                STRIP,
                row(61000, Widget::Spinner, "Poly", [0.00, 1.25, 2.12, 5.74]).p(51),
                row(62000, Widget::Knob, "Transp", [2.12, 1.25, 2.12, 5.74]).p(36),
                row(63000, Widget::VSlider, "Slide", [4.24, 1.25, 2.12, 5.74]).p(35),
                row(
                    64100,
                    Widget::ToggleRight,
                    "Legato",
                    [6.36, 1.25, 4.24, 1.91],
                )
                .p(34),
                row(
                    64200,
                    Widget::ToggleRight,
                    "Constant",
                    [6.36, 3.16, 4.24, 1.91],
                )
                .p(38),
                row(
                    64300,
                    Widget::ToggleRight,
                    "Mono Mode",
                    [6.36, 5.07, 4.24, 1.91],
                )
                .p(50),
                row(
                    65000,
                    Widget::Spinner,
                    "Arp Rate",
                    [10.60, 1.25, 2.12, 5.74],
                )
                .p(47),
                row(
                    66000,
                    Widget::Spinner,
                    "Arp Range",
                    [12.72, 1.25, 2.12, 5.74],
                )
                .p(48),
                row(
                    67000,
                    Widget::Spinner,
                    "Arp Mode",
                    [14.84, 1.25, 2.12, 5.74],
                )
                .p(49),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FX",
            tab: 6,
            rows: &[
                STRIP,
                row(71000, Widget::Knob, "Distortion", [0.00, 1.25, 2.82, 5.74]).p(52),
                row(72000, Widget::Knob, "Gain", [2.82, 1.25, 2.83, 5.74]).p(53),
                row(
                    73000,
                    Widget::Spinner,
                    "Delay Time",
                    [5.65, 1.25, 2.83, 5.74],
                )
                .p(54),
                row(
                    74000,
                    Widget::VSlider,
                    "Delay Fdbk",
                    [8.48, 1.25, 2.82, 5.74],
                )
                .p(55),
                row(
                    75000,
                    Widget::VSlider,
                    "Delay Filter",
                    [11.30, 1.25, 2.83, 5.74],
                )
                .p(57),
                row(76000, Widget::Knob, "Delay Mix", [14.13, 1.25, 2.82, 5.74]).p(56),
            ],
            ..Case::DEFAULT
        },
    ],
};

//! The MiniSynth panel (type 0), card 8.5 U. Six tabs; the FX page swaps its knob row and its
//! toggle with the boxed sub-tab list `DIST` / `DELAY` / `CHORUS` / `PHASER` (#5.5, a setting).

use crate::panels::build::*;

/// The FX sub-tab list.
const FX: u32 = 65500;

pub fn panel() -> Option<Panel> {
    let fx = |k| Show::Setting(FX, k);
    Some(Panel::new(vstack([
        tabs(0, "OSC · FILTER · FLT EG · LFO · MASTER · FX").h(Size::U(1.25)),
        hstack([
            spinner(11000, "OSCILLATOR KIND").p(6),
            knob(12000, "Modifier").p(7),
            knob(13000, "Noise").p(8),
            knob(14000, "Transp").p(2),
        ])
        .show(Show::Tab(0)),
        hstack([
            knob(21000, "CUTOFF").p(9),
            v_slider(22000, "KBD TRK").p(11).w(Size::Flex(0.5)),
            v_slider(23000, "VEL TRK").p(12).w(Size::Flex(0.5)),
            knob(24000, "RES").p(10),
        ])
        .show(Show::Tab(1)),
        hstack([
            v_slider(31000, "ATK").p(13),
            v_slider(32000, "DEC").p(14),
            v_slider(33000, "SUS").p(15),
            v_slider(34000, "REL").p(16),
            knob(35000, "AMNT").p(17).w(Size::Flex(2.0)),
        ])
        .show(Show::Tab(2)),
        hstack([
            v_slider(41000, "LFO Amnt").p(21),
            knob(42000, "LFO Rate").p(20),
            spinner(43000, "LFO Shape").p(18),
            spinner(44000, "LFO Dest").p(19),
            v_slider(45000, "MW").p(5),
            spinner(46000, "MW Dest").p(4),
        ])
        .show(Show::Tab(3)),
        hstack([
            v_slider(51000, "ATK").p(22),
            v_slider(52000, "DEC").p(23),
            v_slider(53000, "SUS").p(24),
            v_slider(54000, "REL").p(25),
            spinner(55000, "VOICES MODE").p(1),
            v_slider(56000, "Slide").p(3),
        ])
        .show(Show::Tab(4)),
        vstack([
            hstack([knob(61100, "Drive").p(27), knob(61200, "Decim").p(28)]).show(fx(0)),
            hstack([
                knob(62100, "Fdbk").p(31),
                knob(62200, "Flt").p(32),
                knob(62300, "Mix").p(33),
                knob(62400, "Time").p(30),
            ])
            .show(fx(1)),
            hstack([
                knob(63100, "Rate").p(35),
                knob(63200, "Depth").p(36),
                knob(63300, "Mix").p(37),
            ])
            .show(fx(2)),
            hstack([
                knob(64100, "Speed").p(39),
                knob(64200, "Width").p(40),
                knob(64300, "Fdbk").p(41),
                knob(64400, "Freq").p(43),
                knob(64500, "Mix").p(42),
            ])
            .show(fx(3)),
            hstack([
                toggle(65100, "").p(26).ml(1.0).show(fx(0)),
                toggle(65200, "").p(29).ml(1.0).show(fx(1)),
                toggle(65300, "").p(34).ml(1.0).show(fx(2)),
                toggle(65400, "").p(38).ml(1.0).show(fx(3)),
                list(FX, "DIST · DELAY · CHORUS · PHASER", Bar::Boxed)
                    .w(Size::Percent(70.0))
                    .h(Size::U(1.25))
                    .margins(0.0, 1.0, 0.0, 1.0),
            ])
            .h(Size::U(2.0)),
        ])
        .show(Show::Tab(5)),
    ])))
}

#[cfg(test)]
const STRIP: Row = row(
    0,
    Widget::Tabs,
    "OSC · FILTER · FLT EG · LFO · MASTER · FX",
    [0.0, 0.0, 16.96, 1.25],
)
.h(Size::U(1.25));

#[cfg(test)]
const PILL: [f32; 4] = [1.00, 4.49, 2.08, 2.00];

#[cfg(test)]
const SUB_TABS: Row = row(
    65500,
    Widget::List(Bar::Boxed),
    "DIST · DELAY · CHORUS · PHASER",
    [4.08, 4.86, 11.88, 1.25],
)
.w(Size::Percent(70.0))
.h(Size::U(1.25))
.margins(0.0, 1.0, 0.0, 1.0);

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [16.96, 6.49],
    settings: &[(65500, 0.0)],
    cases: &[
        Case {
            name: "OSC",
            tab: 0,
            rows: &[
                STRIP,
                row(
                    11000,
                    Widget::Spinner,
                    "OSCILLATOR KIND",
                    [0.00, 1.25, 4.24, 5.24],
                )
                .p(6),
                row(12000, Widget::Knob, "Modifier", [4.24, 1.25, 4.24, 5.24]).p(7),
                row(13000, Widget::Knob, "Noise", [8.48, 1.25, 4.24, 5.24]).p(8),
                row(14000, Widget::Knob, "Transp", [12.72, 1.25, 4.24, 5.24]).p(2),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FILTER",
            tab: 1,
            rows: &[
                STRIP,
                row(21000, Widget::Knob, "CUTOFF", [0.00, 1.25, 5.65, 5.24]).p(9),
                row(22000, Widget::VSlider, "KBD TRK", [5.65, 1.25, 2.83, 5.24])
                    .p(11)
                    .w(Size::Flex(0.5)),
                row(23000, Widget::VSlider, "VEL TRK", [8.48, 1.25, 2.82, 5.24])
                    .p(12)
                    .w(Size::Flex(0.5)),
                row(24000, Widget::Knob, "RES", [11.30, 1.25, 5.66, 5.24]).p(10),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FLT EG",
            tab: 2,
            rows: &[
                STRIP,
                row(31000, Widget::VSlider, "ATK", [0.00, 1.25, 2.82, 5.24]).p(13),
                row(32000, Widget::VSlider, "DEC", [2.82, 1.25, 2.83, 5.24]).p(14),
                row(33000, Widget::VSlider, "SUS", [5.65, 1.25, 2.83, 5.24]).p(15),
                row(34000, Widget::VSlider, "REL", [8.48, 1.25, 2.82, 5.24]).p(16),
                row(35000, Widget::Knob, "AMNT", [11.30, 1.25, 5.66, 5.24])
                    .p(17)
                    .w(Size::Flex(2.0)),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "LFO",
            tab: 3,
            rows: &[
                STRIP,
                row(41000, Widget::VSlider, "LFO Amnt", [0.00, 1.25, 2.82, 5.24]).p(21),
                row(42000, Widget::Knob, "LFO Rate", [2.82, 1.25, 2.83, 5.24]).p(20),
                row(
                    43000,
                    Widget::Spinner,
                    "LFO Shape",
                    [5.65, 1.25, 2.83, 5.24],
                )
                .p(18),
                row(44000, Widget::Spinner, "LFO Dest", [8.48, 1.25, 2.82, 5.24]).p(19),
                row(45000, Widget::VSlider, "MW", [11.30, 1.25, 2.83, 5.24]).p(5),
                row(46000, Widget::Spinner, "MW Dest", [14.13, 1.25, 2.82, 5.24]).p(4),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "MASTER",
            tab: 4,
            rows: &[
                STRIP,
                row(51000, Widget::VSlider, "ATK", [0.00, 1.25, 2.82, 5.24]).p(22),
                row(52000, Widget::VSlider, "DEC", [2.82, 1.25, 2.83, 5.24]).p(23),
                row(53000, Widget::VSlider, "SUS", [5.65, 1.25, 2.83, 5.24]).p(24),
                row(54000, Widget::VSlider, "REL", [8.48, 1.25, 2.82, 5.24]).p(25),
                row(
                    55000,
                    Widget::Spinner,
                    "VOICES MODE",
                    [11.30, 1.25, 2.83, 5.24],
                )
                .p(1),
                row(56000, Widget::VSlider, "Slide", [14.13, 1.25, 2.82, 5.24]).p(3),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FX DIST",
            tab: 5,
            settings: &[(65500, 0.0)],
            rows: &[
                STRIP,
                row(61100, Widget::Knob, "Drive", [0.00, 1.25, 8.48, 3.24]).p(27),
                row(61200, Widget::Knob, "Decim", [8.48, 1.25, 8.48, 3.24]).p(28),
                row(65100, Widget::Toggle, "", PILL)
                    .p(26)
                    .margins(0.0, 0.0, 0.0, 1.0),
                SUB_TABS,
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FX DELAY",
            tab: 5,
            settings: &[(65500, 1.0)],
            rows: &[
                STRIP,
                row(62100, Widget::Knob, "Fdbk", [0.00, 1.25, 4.24, 3.24]).p(31),
                row(62200, Widget::Knob, "Flt", [4.24, 1.25, 4.24, 3.24]).p(32),
                row(62300, Widget::Knob, "Mix", [8.48, 1.25, 4.24, 3.24]).p(33),
                row(62400, Widget::Knob, "Time", [12.72, 1.25, 4.24, 3.24]).p(30),
                row(65200, Widget::Toggle, "", PILL)
                    .p(29)
                    .margins(0.0, 0.0, 0.0, 1.0),
                SUB_TABS,
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FX CHORUS",
            tab: 5,
            settings: &[(65500, 2.0)],
            rows: &[
                STRIP,
                row(63100, Widget::Knob, "Rate", [0.00, 1.25, 5.65, 3.24]).p(35),
                row(63200, Widget::Knob, "Depth", [5.65, 1.25, 5.65, 3.24]).p(36),
                row(63300, Widget::Knob, "Mix", [11.30, 1.25, 5.66, 3.24]).p(37),
                row(65300, Widget::Toggle, "", PILL)
                    .p(34)
                    .margins(0.0, 0.0, 0.0, 1.0),
                SUB_TABS,
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FX PHASER",
            tab: 5,
            settings: &[(65500, 3.0)],
            rows: &[
                STRIP,
                row(64100, Widget::Knob, "Speed", [0.00, 1.25, 3.39, 3.24]).p(39),
                row(64200, Widget::Knob, "Width", [3.39, 1.25, 3.39, 3.24]).p(40),
                row(64300, Widget::Knob, "Fdbk", [6.78, 1.25, 3.39, 3.24]).p(41),
                row(64400, Widget::Knob, "Freq", [10.17, 1.25, 3.39, 3.24]).p(43),
                row(64500, Widget::Knob, "Mix", [13.56, 1.25, 3.40, 3.24]).p(42),
                row(65400, Widget::Toggle, "", PILL)
                    .p(38)
                    .margins(0.0, 0.0, 0.0, 1.0),
                SUB_TABS,
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use crate::panels::build::*;
    use crate::panels::is_bipolar;
    use crate::params::module_spec;

    /// A knob draws bipolar exactly when its reset is 0.5: MiniSynth passes 0.5 for `Modifier`, `Noise` and
    /// `LFO Rate`, `Transp` and `AMNT` reset to their 0.5 default, and the FX knobs reset to 0.
    #[test]
    fn the_knobs_that_reset_to_one_half_are_bipolar() {
        let spec = module_spec(0).unwrap();
        let panel = super::panel().unwrap();
        let mut bipolar = Vec::new();
        for leaf in panel.root.leaves() {
            let pspec = &spec.params[leaf.param.unwrap_or(0)];
            if leaf.widget == Widget::Knob {
                assert_eq!(
                    is_bipolar(leaf, Some(pspec)),
                    pspec.reset == 0.5,
                    "{}",
                    leaf.label
                );
                if pspec.reset == 0.5 {
                    bipolar.push(leaf.label);
                }
            }
        }
        assert_eq!(bipolar, ["Modifier", "Noise", "Transp", "AMNT", "LFO Rate"]);
    }
}

//! The GMSynth panel (type 2), card 9.5 U: eight tabs, and on `FX` one page per effect slot.
//! Ids follow the instrument convention, tab·10000 plus the `#n` digits padded to four: FILTER #6.1 is 16100.

use crate::panels::build::*;
// The waveshapes of the OSC `A` and `B` spinners, kept with the preset, not as params.
use crate::params::GMSYNTH_WAVESHAPES as WAVESHAPES;

const PB_RANGE: &[&str] = &["1 smt", "2 smt", "7 smt", "Oct"];
const MW_DEST: &[&str] = &["Filter", "LFO", "Osc. Mod."];

const TABS: &str = "FILTER · OSC · MOD · LEVEL · EG · LFO · VOICES · FX";

pub fn panel() -> Option<Panel> {
    Some(Panel::new(vstack([
        tabs(0, TABS).h(Size::U(1.25)),
        hstack([
            v_slider(11000, "CUTOFF").p(21),
            v_slider(12000, "RES").p(22),
            v_slider(13000, "ATTACK").p(25),
            v_slider(14000, "DECAY").p(26),
            v_slider(15000, "AMOUNT").p(27),
            vstack([toggle(16100, "KBD").p(24), spinner(16200, "KIND").p(23)]),
        ])
        .show(Show::Tab(0)),
        hstack([
            vstack([
                spinner(21100, "A").items(WAVESHAPES),
                spinner(21200, "B").items(WAVESHAPES),
            ]),
            knob(22000, "PITCH B").p(11).w(Size::Flex(1.5)),
            knob(23000, "FINE B").p(12).ml(0.5),
            v_slider(24000, "PHASE B").p(14),
            v_slider(25000, "BALANCE").p(13),
            v_slider(26000, "NOISE").p(20),
        ])
        .show(Show::Tab(1)),
        hstack([
            v_slider(31000, "MOD. AMNT").p(15),
            spinner(32000, "KIND").p(16),
            v_slider(33000, "ATTACK").p(17),
            v_slider(34000, "DECAY").p(18),
            v_slider(35000, "EG AMOUNT").p(19),
            // A new GMSynth's `CSTM` (`cstm::GMSYNTH`, which owns the encoding): `2 smt`, `Osc. Mod.` and mid.
            vstack([
                spinner(36100, "PB RANGE").items(PB_RANGE).default(1.0),
                spinner(36200, "MW DEST").items(MW_DEST).default(2.0),
            ]),
            v_slider(37000, "MW AMOUNT").default(0.5),
        ])
        .show(Show::Tab(2)),
        hstack([
            knob(41000, "LEVEL").p(1).w(Size::Flex(1.5)),
            knob(42000, "PAN").p(2).ml(0.5),
            v_slider(43000, "ATK").p(3),
            v_slider(44000, "DEC").p(4),
            v_slider(45000, "SUS").p(5),
            v_slider(46000, "REL").p(6),
        ])
        .show(Show::Tab(3)),
        hstack([
            v_slider(51000, "AMOUNT").p(40),
            v_slider(52000, "ATTACK").p(36),
            v_slider(53000, "DECAY").p(37),
            v_slider(54000, "SUSTAIN").p(38),
            v_slider(55000, "RELEASE").p(39),
            vstack([toggle(56100, "VEL").p(74), spinner(56200, "DEST").p(41)]),
        ])
        .show(Show::Tab(4)),
        hstack([
            knob(61000, "AMOUNT").p(34),
            spinner(62000, "SHAPE").p(33),
            spinner(63000, "DEST").p(35),
            spinner(64000, "RATE").p(32).items_per_column(5),
        ])
        .show(Show::Tab(5)),
        hstack([
            knob(71000, "TRNSP").p(28),
            v_slider(72000, "SLIDE").p(29),
            v_slider(73000, "CHOIR DTN").p(44),
            v_slider(74000, "CHOIR STR").p(45),
            vstack([toggle(75100, "MONO").p(30), toggle(75200, "AUTO").p(31)]),
            vstack([
                spinner(76100, "VOICES").p(42),
                spinner(76200, "SCALE").p(43),
            ]),
        ])
        .show(Show::Tab(6)),
        fx().show(Show::Tab(7)),
    ])))
}

/// The `FX` page: the slot list over the selected slot's on/off pill, then the selected slot's controls. Slot k is
/// #(k + 2) with params 46 + 7k (kind) to 52 + 7k (Y).
fn fx() -> Part {
    let slot = |k: usize| Show::Setting(81100, k);
    let pill = |k: usize| {
        toggle(81200 + 100 * k as u32, "")
            .p(47 + 7 * k)
            .show(slot(k))
    };
    let page = |k: usize| {
        let (id, p) = (80000 + 1000 * (k as u32 + 2), 46 + 7 * k);
        hstack([
            vstack([
                hstack([
                    spinner(id + 111, "FX KIND").p(p).items_per_column(5),
                    knob(id + 112, "AMNT").p(p + 4),
                ]),
                hstack([
                    spinner(id + 121, "LFO RATE").p(p + 2).items_per_column(5),
                    spinner(id + 122, "LFO SHAPE").p(p + 3),
                ]),
            ])
            .w(Size::Flex(2.0)),
            v_slider(id + 200, "X").p(p + 5),
            v_slider(id + 300, "Y").p(p + 6),
        ])
        .w(Size::Flex(4.0))
        .show(slot(k))
    };
    hstack([
        vstack([
            list(81100, "1 · 2 · 3", Bar::Right).h(Size::Flex(3.0)),
            pill(0),
            pill(1),
            pill(2),
        ]),
        page(0),
        page(1),
        page(2),
    ])
}

#[cfg(test)]
const STRIP: Row = row(
    0,
    Widget::Tabs,
    "FILTER · OSC · MOD · LEVEL · EG · LFO · VOICES · FX",
    [0.0, 0.0, 16.96, 1.25],
)
.h(Size::U(1.25));
#[cfg(test)]
const SLOTS: Row = row(
    81100,
    Widget::List(Bar::Right),
    "1 · 2 · 3",
    [0.00, 1.25, 3.39, 4.68],
)
.h(Size::Flex(3.0));

/// OSC `A` and `B`: the 29 waveshapes.
#[cfg(test)]
const SHAPES: &str = "Beast · Crunch · Double Spike · Digital · Dirt · Electro · Filth · Fox · Funky · Grubby · Keys · \
    Liquid · Ogre · Rhodes · Saw Mean · Saw Retro · Saw · Silk · Sine · Smooth Sub · Square Retro · Square Smooth · \
    Square · Strat · String · Throat · Triangle · Troll · Vocal";

#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [16.96, 7.49],
    // A and B on Beast; PB RANGE, MW DEST and MW AMOUNT from the CSTM defaults (items 1 and 2, 0.5); FX slot 1.
    settings: &[(21100, 0.0), (21200, 0.0), (36100, 1.0), (36200, 2.0), (37000, 0.5), (81100, 0.0)],
    cases: &[
        Case {
            name: "FILTER",
            rows: &[
                STRIP,
                row(11000, Widget::VSlider, "CUTOFF", [0.00, 1.25, 2.82, 6.24]).p(21),
                row(12000, Widget::VSlider, "RES", [2.82, 1.25, 2.83, 6.24]).p(22),
                row(13000, Widget::VSlider, "ATTACK", [5.65, 1.25, 2.83, 6.24]).p(25),
                row(14000, Widget::VSlider, "DECAY", [8.48, 1.25, 2.82, 6.24]).p(26),
                row(15000, Widget::VSlider, "AMOUNT", [11.30, 1.25, 2.83, 6.24]).p(27),
                row(16100, Widget::Toggle, "KBD", [14.13, 1.25, 2.82, 3.12]).p(24),
                row(16200, Widget::Spinner, "KIND", [14.13, 4.37, 2.82, 3.12]).p(23),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "OSC",
            tab: 1,
            rows: &[
                STRIP,
                row(21100, Widget::Spinner, "A", [0.00, 1.25, 2.53, 3.12]).items(SHAPES),
                row(21200, Widget::Spinner, "B", [0.00, 4.37, 2.53, 3.12]).items(SHAPES),
                row(22000, Widget::Knob, "PITCH B", [2.53, 1.25, 3.80, 6.24]).p(11).w(Size::Flex(1.5)),
                row(23000, Widget::Knob, "FINE B", [6.83, 1.25, 2.53, 6.24]).p(12).margins(0.0, 0.0, 0.0, 0.5),
                row(24000, Widget::VSlider, "PHASE B", [9.36, 1.25, 2.53, 6.24]).p(14),
                row(25000, Widget::VSlider, "BALANCE", [11.89, 1.25, 2.53, 6.24]).p(13),
                row(26000, Widget::VSlider, "NOISE", [14.42, 1.25, 2.54, 6.24]).p(20),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "MOD",
            tab: 2,
            rows: &[
                STRIP,
                row(31000, Widget::VSlider, "MOD. AMNT", [0.00, 1.25, 2.42, 6.24]).p(15),
                row(32000, Widget::Spinner, "KIND", [2.42, 1.25, 2.42, 6.24]).p(16),
                row(33000, Widget::VSlider, "ATTACK", [4.84, 1.25, 2.42, 6.24]).p(17),
                row(34000, Widget::VSlider, "DECAY", [7.26, 1.25, 2.43, 6.24]).p(18),
                row(35000, Widget::VSlider, "EG AMOUNT", [9.69, 1.25, 2.42, 6.24]).p(19),
                row(36100, Widget::Spinner, "PB RANGE", [12.11, 1.25, 2.42, 3.12]).items("1 smt · 2 smt · 7 smt · Oct"),
                row(36200, Widget::Spinner, "MW DEST", [12.11, 4.37, 2.42, 3.12]).items("Filter · LFO · Osc. Mod."),
                row(37000, Widget::VSlider, "MW AMOUNT", [14.53, 1.25, 2.43, 6.24]),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "LEVEL",
            tab: 3,
            rows: &[
                STRIP,
                row(41000, Widget::Knob, "LEVEL", [0.00, 1.25, 3.79, 6.24]).p(1).w(Size::Flex(1.5)),
                row(42000, Widget::Knob, "PAN", [4.29, 1.25, 2.54, 6.24]).p(2).margins(0.0, 0.0, 0.0, 0.5),
                row(43000, Widget::VSlider, "ATK", [6.83, 1.25, 2.53, 6.24]).p(3),
                row(44000, Widget::VSlider, "DEC", [9.36, 1.25, 2.53, 6.24]).p(4),
                row(45000, Widget::VSlider, "SUS", [11.89, 1.25, 2.53, 6.24]).p(5),
                row(46000, Widget::VSlider, "REL", [14.42, 1.25, 2.54, 6.24]).p(6),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "EG",
            tab: 4,
            rows: &[
                STRIP,
                row(51000, Widget::VSlider, "AMOUNT", [0.00, 1.25, 2.82, 6.24]).p(40),
                row(52000, Widget::VSlider, "ATTACK", [2.82, 1.25, 2.83, 6.24]).p(36),
                row(53000, Widget::VSlider, "DECAY", [5.65, 1.25, 2.83, 6.24]).p(37),
                row(54000, Widget::VSlider, "SUSTAIN", [8.48, 1.25, 2.82, 6.24]).p(38),
                row(55000, Widget::VSlider, "RELEASE", [11.30, 1.25, 2.83, 6.24]).p(39),
                row(56100, Widget::Toggle, "VEL", [14.13, 1.25, 2.82, 3.12]).p(74),
                row(56200, Widget::Spinner, "DEST", [14.13, 4.37, 2.82, 3.12]).p(41),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "LFO",
            tab: 5,
            rows: &[
                STRIP,
                row(61000, Widget::Knob, "AMOUNT", [0.00, 1.25, 4.24, 6.24]).p(34),
                row(62000, Widget::Spinner, "SHAPE", [4.24, 1.25, 4.24, 6.24]).p(33),
                row(63000, Widget::Spinner, "DEST", [8.48, 1.25, 4.24, 6.24]).p(35),
                row(64000, Widget::Spinner, "RATE", [12.72, 1.25, 4.24, 6.24]).p(32).items_per_column(5),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "VOICES",
            tab: 6,
            rows: &[
                STRIP,
                row(71000, Widget::Knob, "TRNSP", [0.00, 1.25, 2.82, 6.24]).p(28),
                row(72000, Widget::VSlider, "SLIDE", [2.82, 1.25, 2.83, 6.24]).p(29),
                row(73000, Widget::VSlider, "CHOIR DTN", [5.65, 1.25, 2.83, 6.24]).p(44),
                row(74000, Widget::VSlider, "CHOIR STR", [8.48, 1.25, 2.82, 6.24]).p(45),
                row(75100, Widget::Toggle, "MONO", [11.30, 1.25, 2.83, 3.12]).p(30),
                row(75200, Widget::Toggle, "AUTO", [11.30, 4.37, 2.83, 3.12]).p(31),
                row(76100, Widget::Spinner, "VOICES", [14.13, 1.25, 2.82, 3.12]).p(42),
                row(76200, Widget::Spinner, "SCALE", [14.13, 4.37, 2.82, 3.12]).p(43),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FX slot 1",
            tab: 7,
            rows: &[
                STRIP,
                SLOTS,
                row(81200, Widget::Toggle, "", [0.00, 5.93, 3.39, 1.56]).p(47),
                row(82111, Widget::Spinner, "FX KIND", [3.39, 1.25, 3.39, 3.12]).p(46).items_per_column(5),
                row(82112, Widget::Knob, "AMNT", [6.78, 1.25, 3.39, 3.12]).p(50),
                row(82121, Widget::Spinner, "LFO RATE", [3.39, 4.37, 3.39, 3.12]).p(48).items_per_column(5),
                row(82122, Widget::Spinner, "LFO SHAPE", [6.78, 4.37, 3.39, 3.12]).p(49),
                row(82200, Widget::VSlider, "X", [10.17, 1.25, 3.39, 6.24]).p(51),
                row(82300, Widget::VSlider, "Y", [13.56, 1.25, 3.40, 6.24]).p(52),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FX slot 2",
            tab: 7,
            settings: &[(81100, 1.0)],
            rows: &[
                STRIP,
                SLOTS,
                row(81300, Widget::Toggle, "", [0.00, 5.93, 3.39, 1.56]).p(54),
                row(83111, Widget::Spinner, "FX KIND", [3.39, 1.25, 3.39, 3.12]).p(53).items_per_column(5),
                row(83112, Widget::Knob, "AMNT", [6.78, 1.25, 3.39, 3.12]).p(57),
                row(83121, Widget::Spinner, "LFO RATE", [3.39, 4.37, 3.39, 3.12]).p(55).items_per_column(5),
                row(83122, Widget::Spinner, "LFO SHAPE", [6.78, 4.37, 3.39, 3.12]).p(56),
                row(83200, Widget::VSlider, "X", [10.17, 1.25, 3.39, 6.24]).p(58),
                row(83300, Widget::VSlider, "Y", [13.56, 1.25, 3.40, 6.24]).p(59),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FX slot 3",
            tab: 7,
            settings: &[(81100, 2.0)],
            rows: &[
                STRIP,
                SLOTS,
                row(81400, Widget::Toggle, "", [0.00, 5.93, 3.39, 1.56]).p(61),
                row(84111, Widget::Spinner, "FX KIND", [3.39, 1.25, 3.39, 3.12]).p(60).items_per_column(5),
                row(84112, Widget::Knob, "AMNT", [6.78, 1.25, 3.39, 3.12]).p(64),
                row(84121, Widget::Spinner, "LFO RATE", [3.39, 4.37, 3.39, 3.12]).p(62).items_per_column(5),
                row(84122, Widget::Spinner, "LFO SHAPE", [6.78, 4.37, 3.39, 3.12]).p(63),
                row(84200, Widget::VSlider, "X", [10.17, 1.25, 3.39, 6.24]).p(65),
                row(84300, Widget::VSlider, "Y", [13.56, 1.25, 3.40, 6.24]).p(66),
            ],
            ..Case::DEFAULT
        },
    ],
};

//! Reverb 2 (ID 35), UIHeight 9 U. Pages `MAIN` (0) and `TUNE` (1).

use crate::panels::build::*;

pub fn panel() -> Option<Panel> {
    Some(Panel::new(vstack([
        tabs(1, "MAIN · TUNE").h(Size::U(1.25)),
        hstack([
            v_slider(2, "High Cut").p(1).value_text(),
            v_slider(3, "Low Cut").p(2).value_text(),
            vstack([
                hstack([knob(4, "Size").p(3), knob(5, "Decay").p(4)]).show(Show::Tab(0)),
                hstack([knob(6, "Diffusion").p(5), knob(7, "Damping").p(6)]).show(Show::Tab(0)),
            ])
            .w(Size::Flex(2.0))
            .show(Show::Tab(0)),
            v_slider(8, "Dry").p(7).value_text(),
            v_slider(9, "ER").p(8).value_text(),
            v_slider(10, "Wet").p(9).value_text(),
        ])
        .show(Show::Tab(0)),
        hstack([
            toggle(11, "Side Input").p(10).value_text(),
            v_slider(12, "Delay").p(11).value_text(),
            toggle(13, "Tempo").p(12).value_text(),
            vstack([
                hstack([
                    knob(14, "Mod").p(13),
                    knob(15, "Bass").p(14),
                    knob(16, "Separation").p(15),
                ])
                .show(Show::Tab(1)),
                hstack([
                    knob(17, "Speed").p(16),
                    knob(18, "Cross").p(17),
                    knob(19, "Pan").p(18),
                ])
                .show(Show::Tab(1)),
            ])
            .w(Size::Flex(4.0))
            .show(Show::Tab(1)),
        ])
        .show(Show::Tab(1)),
    ])))
}

#[cfg(test)]
const TABS: Row = row(1, Widget::Tabs, "MAIN · TUNE", [0.0, 0.0, 14.0, 1.25]).h(Size::U(1.25));

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 6.99],
    settings: &[],
    cases: &[
        Case {
            name: "MAIN",
            rows: &[
                TABS,
                row(2, Widget::VSlider, "High Cut", [0.0, 1.25, 2.0, 5.74])
                    .p(1)
                    .value_text(),
                row(3, Widget::VSlider, "Low Cut", [2.0, 1.25, 2.0, 5.74])
                    .p(2)
                    .value_text(),
                row(4, Widget::Knob, "Size", [4.0, 1.25, 2.0, 2.87]).p(3),
                row(5, Widget::Knob, "Decay", [6.0, 1.25, 2.0, 2.87]).p(4),
                row(6, Widget::Knob, "Diffusion", [4.0, 4.12, 2.0, 2.87]).p(5),
                row(7, Widget::Knob, "Damping", [6.0, 4.12, 2.0, 2.87]).p(6),
                row(8, Widget::VSlider, "Dry", [8.0, 1.25, 2.0, 5.74])
                    .p(7)
                    .value_text(),
                row(9, Widget::VSlider, "ER", [10.0, 1.25, 2.0, 5.74])
                    .p(8)
                    .value_text(),
                row(10, Widget::VSlider, "Wet", [12.0, 1.25, 2.0, 5.74])
                    .p(9)
                    .value_text(),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "TUNE",
            tab: 1,
            rows: &[
                TABS,
                row(11, Widget::Toggle, "Side Input", [0.0, 1.25, 2.0, 5.74])
                    .p(10)
                    .value_text(),
                row(12, Widget::VSlider, "Delay", [2.0, 1.25, 2.0, 5.74])
                    .p(11)
                    .value_text(),
                row(13, Widget::Toggle, "Tempo", [4.0, 1.25, 2.0, 5.74])
                    .p(12)
                    .value_text(),
                row(14, Widget::Knob, "Mod", [6.0, 1.25, 2.66, 2.87]).p(13),
                row(15, Widget::Knob, "Bass", [8.66, 1.25, 2.67, 2.87]).p(14),
                row(16, Widget::Knob, "Separation", [11.33, 1.25, 2.66, 2.87]).p(15),
                row(17, Widget::Knob, "Speed", [6.0, 4.12, 2.66, 2.87]).p(16),
                row(18, Widget::Knob, "Cross", [8.66, 4.12, 2.67, 2.87]).p(17),
                row(19, Widget::Knob, "Pan", [11.33, 4.12, 2.66, 2.87]).p(18),
            ],
            ..Case::DEFAULT
        },
    ],
};

//! The Trance Delay panel (ID 16), UIHeight 7 U.

use crate::panels::build::*;
#[cfg(test)]
use crate::params::menu_value;

const TIMES: &str = "1/4 · 3/8 · 1/2 · 3/4 · 1/1";
const MODES: &str = "Stereo · Ping-Pong · Mirror";

pub fn panel() -> Option<Panel> {
    Some(Panel::new(vstack([
        list(1, TIMES, Bar::Bottom).p(1).h(Size::U(1.25)),
        hstack([
            list(2, MODES, Bar::Right).p(5).w(Size::Percent(25.0)),
            // Only in `Ping-Pong` mode, where it takes a flex share.
            knob(3, "INPUT PAN").p(6).show(Show::Item(5, 1)),
            v_slider(4, "FEEDBACK").p(2),
            v_slider(5, "FILTER").p(4),
            v_slider(6, "MIX").p(3),
        ]),
    ])))
}

#[cfg(test)]
const STRIP: Row = row(1, Widget::List(Bar::Bottom), TIMES, [0.0, 0.0, 14.0, 1.25])
    .p(1)
    .h(Size::U(1.25));
#[cfg(test)]
const MODE: Row = row(2, Widget::List(Bar::Right), MODES, [0.0, 1.25, 3.5, 3.74])
    .p(5)
    .w(Size::Percent(25.0));
#[cfg(test)]
#[rustfmt::skip]
const SLIDERS: [Row; 3] = [
    row(4, Widget::VSlider, "FEEDBACK", [3.5, 1.25, 3.5, 3.74]).p(2),
    row(5, Widget::VSlider, "FILTER", [7.0, 1.25, 3.5, 3.74]).p(4),
    row(6, Widget::VSlider, "MIX", [10.5, 1.25, 3.5, 3.74]).p(3),
];

/// The rects in each mode. In Ping-Pong `INPUT PAN` takes the first of four flex shares of 10.5 U; those rects follow
/// from the stack layout rules.
#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 4.99],
    settings: &[],
    cases: &[
        Case {
            name: "Stereo",
            rows: &[STRIP, MODE, SLIDERS[0], SLIDERS[1], SLIDERS[2]],
            ..Case::DEFAULT
        },
        Case {
            name: "Ping-Pong",
            params: &[(5, menu_value(1, 3))],
            rows: &[
                STRIP,
                MODE,
                row(3, Widget::Knob, "INPUT PAN", [3.5, 1.25, 2.62, 3.74]).p(6),
                row(4, Widget::VSlider, "FEEDBACK", [6.12, 1.25, 2.63, 3.74]).p(2),
                row(5, Widget::VSlider, "FILTER", [8.75, 1.25, 2.62, 3.74]).p(4),
                row(6, Widget::VSlider, "MIX", [11.37, 1.25, 2.63, 3.74]).p(3),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "Mirror",
            params: &[(5, menu_value(2, 3))],
            rows: &[STRIP, MODE, SLIDERS[0], SLIDERS[1], SLIDERS[2]],
            ..Case::DEFAULT
        },
    ],
};

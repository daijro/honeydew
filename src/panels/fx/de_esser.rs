//! De-esser (`honeydew_audio::de_esser`), UIHeight 7.5 U: Honeydew's own panel (GitHub
//! #43). Its knobs, Listen and the Mode list, then the Compressor's GR / IN / OUT meter, fed the reduction.

use honeydew_audio::de_esser::{FREQUENCY, LISTEN, MODE, RANGE, THRESHOLD};

use super::compressor::level_meter;
use crate::panels::build::*;

pub(in crate::panels) fn panel() -> Panel {
    Panel::new(hstack([
        knob(1, "Frequency").p(FREQUENCY),
        knob(2, "Threshold").p(THRESHOLD),
        knob(3, "Range").p(RANGE),
        vstack([
            toggle_right(4, "Listen").p(LISTEN).h(Size::U(1.25)),
            list(5, "Split · Wide", Bar::Left).p(MODE),
        ])
        .w(Size::U(3.5)),
        display(6).w(Size::U(3.75)).mr(0.5).draw(level_meter),
    ]))
}

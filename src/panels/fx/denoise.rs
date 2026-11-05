//! Denoise (`honeydew_audio::denoise`), UIHeight 7.5 U: Honeydew's own panel (GitHub #43). Its knobs and Learn, then the Compressor's GR / IN / OUT meter, fed the reduction over all bins.

use honeydew_audio::denoise::{LEARN, REDUCTION, SENSITIVITY};

use super::compressor::level_meter;
use crate::panels::build::*;

pub(in crate::panels) fn panel() -> Panel {
    Panel::new(hstack([
        knob(1, "Reduction").p(REDUCTION).w(Size::U(3.0)),
        knob(2, "Sensitivity").p(SENSITIVITY).w(Size::U(3.0)),
        vstack([toggle_right(3, "Learn").p(LEARN).h(Size::U(1.25))]),
        display(4).w(Size::U(3.75)).mr(0.5).draw(level_meter),
    ]))
}

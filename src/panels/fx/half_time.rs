//! Half Time (`honeydew_audio::half_time`), UIHeight 6 U: Honeydew's own panel (GitHub #91). The Length and Speed
//! spinners and the Smooth and Mix knobs. Speed, Smooth and Length's 1/16 bar to 8 bars came with GitHub #122.

use honeydew_audio::half_time::{LENGTH, MIX, SMOOTH, SPEED};

use crate::panels::build::*;

pub(in crate::panels) fn panel() -> Panel {
    Panel::new(hstack([
        spinner(1, "Length").p(LENGTH),
        spinner(3, "Speed").p(SPEED),
        knob(4, "Smooth").p(SMOOTH),
        knob(2, "Mix").p(MIX),
    ]))
}

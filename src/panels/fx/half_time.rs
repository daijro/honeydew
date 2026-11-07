//! Half Time (`honeydew_audio::half_time`), UIHeight 6 U: Honeydew's own panel (GitHub #91).
//! The Length list and the Mix knob.

use honeydew_audio::half_time::{LENGTH, MIX};

use crate::panels::build::*;

pub(in crate::panels) fn panel() -> Panel {
    Panel::new(hstack([
        list(1, "1/4 bar · 1/2 bar · 1 bar · 2 bars", Bar::Left)
            .p(LENGTH)
            .w(Size::U(3.5)),
        knob(2, "Mix").p(MIX),
    ]))
}

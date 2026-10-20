//! Lemon Reverb and Lemon Limiter (`honeydew_audio::lemon`), UIHeight 7 U: Honeydew's own panels, a tab strip over a
//! page of knobs or sliders.

use honeydew_audio::lemon::{self, dynamics as d, reverb as r};

use crate::panels::build::*;

/// Pages `ROOM` (0), `TONE` (1) and `MIX` (2).
pub(in crate::panels) fn reverb() -> Panel {
    Panel::new(vstack([
        tabs(1, "ROOM · TONE · MIX").h(Size::U(1.25)),
        hstack([
            knob(2, "Predelay").p(1 + r::PREDELAY),
            knob(3, "Room Size").p(1 + r::ROOM),
            knob(4, "Diffusion").p(1 + r::DIFFUSION),
            knob(5, "Decay").p(1 + r::DECAY),
            vstack([toggle_right(6, "Tempo").p(lemon::REVERB_TEMPO)]).w(Size::Flex(1.5)),
        ])
        .show(Show::Tab(0)),
        hstack([
            v_slider(7, "Low Cut").p(1 + r::LOW_CUT).value_text(),
            v_slider(8, "High Cut").p(1 + r::HIGH_CUT).value_text(),
            v_slider(9, "Damping").p(1 + r::DAMPING).value_text(),
            v_slider(10, "Bass").p(1 + r::BASS).value_text(),
            v_slider(11, "Crossover").p(1 + r::CROSSOVER).value_text(),
            v_slider(12, "Mod Speed").p(1 + r::MOD_SPEED).value_text(),
            v_slider(13, "Mod Depth").p(1 + r::MOD_DEPTH).value_text(),
        ])
        .show(Show::Tab(1)),
        hstack([
            knob(14, "Dry").p(1 + r::DRY),
            knob(15, "Early Refl.").p(1 + r::EARLY),
            knob(16, "Wet").p(1 + r::WET),
            knob(17, "Separation").p(1 + r::SEPARATION),
            vstack([toggle_right(18, "Mid/Side").p(lemon::REVERB_MID_SIDE)]).w(Size::Flex(1.5)),
        ])
        .show(Show::Tab(2)),
    ]))
}

/// Pages `LIMIT` (0), `TIME` (1), `COMP` (2) and `GATE` (3).
pub(in crate::panels) fn limiter() -> Panel {
    Panel::new(vstack([
        tabs(1, "LIMIT · TIME · COMP · GATE").h(Size::U(1.25)),
        hstack([
            knob(2, "Gain").p(1 + d::GAIN),
            knob(3, "Ceiling").p(1 + d::CEILING),
            knob(4, "Saturation").p(1 + d::SAT),
            vstack([toggle_right(5, "Safety").p(lemon::LIMITER_SAFETY)]).w(Size::Flex(1.5)),
        ])
        .show(Show::Tab(0)),
        hstack([
            knob(6, "Attack").p(1 + d::LIM_ATT),
            knob(7, "Att. Curve").p(1 + d::LIM_ATT_CURVE),
            knob(8, "Release").p(1 + d::LIM_REL),
            knob(9, "Rel. Curve").p(1 + d::LIM_REL_CURVE),
            knob(10, "Peak Window").p(1 + d::PEAK_WINDOW),
        ])
        .show(Show::Tab(1)),
        hstack([
            v_slider(11, "Threshold").p(1 + d::COMP_THRES).value_text(),
            v_slider(12, "Ratio").p(1 + d::COMP_RATIO).value_text(),
            v_slider(13, "Knee").p(1 + d::COMP_KNEE).value_text(),
            v_slider(14, "Attack").p(1 + d::COMP_ATT).value_text(),
            v_slider(15, "Release").p(1 + d::COMP_REL).value_text(),
            v_slider(16, "Curve").p(1 + d::COMP_CURVE).value_text(),
            v_slider(17, "RMS").p(1 + d::RMS_WINDOW).value_text(),
        ])
        .show(Show::Tab(2)),
        hstack([
            knob(18, "Gain").p(1 + d::GATE_GAIN),
            knob(19, "Threshold").p(1 + d::GATE_THRES),
            knob(20, "Release").p(1 + d::GATE_REL),
        ])
        .show(Show::Tab(3)),
    ]))
}

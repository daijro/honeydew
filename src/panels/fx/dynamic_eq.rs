//! Dynamic EQ (`honeydew_audio::dynamic_eq`), UIHeight 11 U: Honeydew's own panel. A tab strip (a page per band, then
//! the envelope's times) over the response display #2 ([`curve`]) and the page's controls.
//!
//! The display is the Equalizer's ([`super::equalizer`]) with the Dynamic EQ's response: x is the Frequency param (a
//! log axis, 20 Hz to 20 kHz), 0 dB is mid-height and ±18 dB fills the height, which is also where a band's Gain puts
//! its handle. The static response (each band at its Gain) is the `menuFill` bars, the live one (with each band's gain
//! offset from its detector, as the engine reports it) a channel-coloured line over them. The handles, their drag
//! (frequency and gain) and their double tap are the Equalizer's.

use eframe::egui::{Color32, Id, Rect, Shape, Stroke, Ui, pos2};
use honeydew_audio::analysis;
use honeydew_audio::dynamic_eq::{
    ATTACK, BANDS, FREQUENCY, GAIN, ON, Q, RANGE, RELEASE, SHAPE, THRESHOLD, band, hz, response_db,
};

use super::equalizer::{self, Handles};
use super::live_analysis;
use crate::panels::build::*;
use crate::ui::UiCtx;

const fn gain_p(k: usize) -> usize {
    band(k, GAIN)
}
const fn freq_p(k: usize) -> usize {
    band(k, FREQUENCY)
}
const HANDLES: Handles = Handles {
    gain: gain_p,
    freq: freq_p,
};

/// The response drawn is the engine's at this rate.
const RATE: f32 = 48_000.0;
/// ±this many dB fill the display.
const SPAN_DB: f32 = 18.0;
/// The live line's width, U.
const LINE_U: f32 = 0.07;

/// The display: the Equalizer's fill and 0 dB line, the static response's bars, the live response's line, then the
/// handles.
fn curve(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let touched = equalizer::touch(ui, cx.m, id, rect, module, state, HANDLES);
    let (m, colors, params) = (cx.m, &cx.theme.colors, &module.params[..]);
    let painter = ui.painter();
    let px = 1.0 / m.ppp();
    painter.rect_filled(rect, 0.0, colors.control_fill.to_color32());
    let mid = m.floor_px(rect.center().y);
    let zero = Rect::from_x_y_ranges(rect.x_range(), mid..=mid + px);
    painter.rect_filled(zero, 0.0, colors.module_fill.to_color32());
    // The response at normalised x, as a fraction of the half height.
    let s =
        |x: f32, dynamic| (response_db(params, dynamic, hz(x), RATE) / SPAN_DB).clamp(-1.0, 1.0);
    for bar in equalizer::response_bars(rect, m, |x| s(x, [0.0; BANDS])) {
        painter.rect_filled(bar, 0.0, colors.menu_fill.to_color32());
    }
    // While it runs, the detectors' offsets from the engine; else none.
    let running = params.first().is_some_and(|&on| on >= 0.5);
    let dynamic = (live_analysis(ui, module.instance_id).and_then(|s| analysis::dynamic_eq(&s)))
        .filter(|d| running && d.iter().all(|v| v.is_finite()))
        .unwrap_or([0.0; BANDS]);
    // A point per device pixel column, at its centre.
    let columns = (rect.width() * m.ppp()).round().max(2.0) as usize;
    let line = (0..columns).map(|i| {
        let x = (i as f32 + 0.5) / columns as f32;
        let y = rect.center().y - s(x, dynamic) * rect.height() / 2.0;
        pos2(rect.left() + x * rect.width(), y)
    });
    painter.add(Shape::line(line.collect(), Stroke::new(m.u(LINE_U), ink)));
    equalizer::handles(ui, cx, rect, params, state.tab, &touched, ink, HANDLES);
    touched.set
}

/// Pages `BAND 1` … `BAND 4` (0–3) and `TIME` (4).
pub(in crate::panels) fn panel() -> Panel {
    let page = |k: usize, first: u32| {
        let p = |q: usize| band(k, q);
        hstack([
            vstack([
                toggle_right(first, "On").p(p(ON)).h(Size::U(1.25)),
                list(first + 1, "Bell · Low Shelf · High Shelf", Bar::Left).p(p(SHAPE)),
            ])
            .w(Size::U(2.8)),
            knob(first + 2, "Frequency").p(p(FREQUENCY)),
            knob(first + 3, "Q").p(p(Q)),
            knob(first + 4, "Gain").p(p(GAIN)),
            knob(first + 5, "Threshold").p(p(THRESHOLD)),
            knob(first + 6, "Range").p(p(RANGE)),
        ])
        .h(Size::U(4.0))
        .show(Show::Tab(k))
    };
    Panel::new(vstack([
        vstack([
            tabs(1, "BAND 1 · BAND 2 · BAND 3 · BAND 4 · TIME").h(Size::U(1.25)),
            display(2).draw(curve),
        ]),
        page(0, 3),
        page(1, 10),
        page(2, 17),
        page(3, 24),
        hstack([knob(31, "Attack").p(ATTACK), knob(32, "Release").p(RELEASE)])
            .h(Size::U(4.0))
            .show(Show::Tab(4)),
    ]))
}

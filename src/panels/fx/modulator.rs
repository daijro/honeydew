//! Modulator (`honeydew_audio::modulator`), UIHeight 8.5 U: Honeydew's own panel. The Source list beside a live meter
//! of the signal (#2, [`meter`]) over a row of the source's controls, after Amount. The card's ≡ menu picks the target
//! (`ui::rack`).

use eframe::egui::{Color32, Id, Rect, Ui};
use honeydew_audio::analysis;
use honeydew_audio::modulator::{
    AMOUNT, ATTACK, DIVISION, GAIN, RATE, RELEASE, SHAPE, SMOOTH, SOURCE, SYNC, Source,
};

use super::live_analysis;
use crate::panels::build::*;
use crate::ui::UiCtx;

const ENVELOPE: Show = Show::Item(SOURCE, 0);
const LFO: Show = Show::Item(SOURCE, 1);
const RANDOM: Show = Show::Item(SOURCE, 2);
/// A cycle's Rate: free (a knob, Hz) or with Sync on (a spinner of divisions).
const FREE: Show = Show::All(&[Show::Not(&ENVELOPE), Show::Item(SYNC, 0)]);
const SYNCED: Show = Show::All(&[Show::Not(&ENVELOPE), Show::Item(SYNC, 1)]);

/// The meter's bar, a fraction of the display's height, centred.
const BAR: f32 = 0.5;

/// The display: a `controlFill` well, its zero line in `moduleFill` (at the left, or the centre for an LFO's −1…1), and
/// a bar in the channel colour from it to the signal while the module runs. Nothing to edit.
fn meter(
    ui: &mut Ui,
    cx: &UiCtx,
    _: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, colors.control_fill.to_color32());
    let source = Source::from_param(module.params.get(SOURCE).copied().unwrap_or(0.0));
    let zero = match source.bipolar() {
        true => m.floor_px(rect.center().x),
        false => rect.left(),
    };
    let px = 1.0 / m.ppp();
    let line = Rect::from_x_y_ranges(zero..=zero + px, rect.y_range());
    painter.rect_filled(line, 0.0, colors.module_fill.to_color32());
    let running = module.params.first().is_some_and(|&on| on >= 0.5);
    let signal = (live_analysis(ui, module.instance_id).and_then(|s| analysis::modulator(&s)))
        .filter(|s| running && s.is_finite())
        .unwrap_or(0.0)
        .clamp(-1.0, 1.0);
    let end = match source.bipolar() {
        true => zero + signal * rect.width() / 2.0,
        false => zero + signal * rect.width(),
    };
    let h = rect.height() * BAR;
    let y = rect.center().y - h / 2.0..=rect.center().y + h / 2.0;
    let bar = Rect::from_x_y_ranges(zero.min(end)..=zero.max(end), y);
    ui.painter().rect_filled(bar, 0.0, ink);
    Vec::new()
}

pub(in crate::panels) fn panel() -> Panel {
    Panel::new(vstack([
        hstack([
            list(1, "Envelope · LFO · Random", Bar::Left)
                .p(SOURCE)
                .w(Size::U(3.5)),
            display(2).draw(meter).margins(0.5, 0.5, 0.5, 0.5),
        ])
        .h(Size::U(2.5)),
        hstack([
            knob(3, "Amount").p(AMOUNT),
            knob(4, "Gain").p(GAIN).show(ENVELOPE),
            knob(5, "Attack").p(ATTACK).show(ENVELOPE),
            knob(6, "Release").p(RELEASE).show(ENVELOPE),
            knob(7, "Rate").p(RATE).show(FREE),
            spinner(8, "Rate").p(DIVISION).show(SYNCED),
            vstack([toggle_right(9, "Sync").p(SYNC).h(Size::U(1.25))])
                .mt(1.375)
                .w(Size::U(2.5))
                .show(Show::Not(&ENVELOPE)),
            spinner(10, "Shape").p(SHAPE).show(LFO),
            knob(11, "Smooth").p(SMOOTH).show(RANDOM),
        ])
        .h(Size::U(4.0)),
    ]))
}

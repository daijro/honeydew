//! Compressor (ID 8), UIHeight 7.5 U.

use eframe::egui::{Color32, Id, Painter, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use honeydew_audio::MeterSnapshot;

use super::leveller::{animate, bounded, live_meter};
use crate::metrics::Metrics;
use crate::panels::build::*;
use crate::ui::UiCtx;

/// What the meter shows (meter step 6): the gain reduction as a linear gain (1 = none), and the input and output
/// peaks, linear (≤ 0 counts as −96 dB).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Levels {
    gain: f32,
    input: f32,
    output: f32,
}

/// No reduction and silence: the meter at rest, and without a meter feed (outside a rack).
const REST: Levels = Levels {
    gain: 1.0,
    input: 0.0,
    output: 0.0,
};

impl Levels {
    /// What the meter feed `m` asks the bars for (meter step 6): the gain of the reduction in dB, IN the module's
    /// input peak and OUT the louder output channel's, each [`bounded`]. Under −96 dB, where a bar is empty, a peak is
    /// silence: the engine's residue (its anti-denormal 1e-30) rests. IN is the stereo max(L, R) of what
    /// reaches the module, before its `Input gain`, which drives only the detector.
    // `gain_reduction_db` is the reduction, positive: the gain is 10^(−dB/20). `min` drops a NaN.
    fn target(m: MeterSnapshot) -> Levels {
        let audible = |peak: f32| {
            let peak = bounded(peak);
            if peak > 10f32.powf(-96.0 / 20.0) {
                peak
            } else {
                0.0
            }
        };
        Levels {
            gain: 10f32.powf(-m.gain_reduction_db / 20.0).min(1.0),
            input: audible(m.input_peak),
            output: audible(m.peak_left.max(m.peak_right)),
        }
    }

    /// After `dt` seconds, each meter value moves by (target − disp)·0.1 per 1/60 s frame.
    /// It snaps to its target within 1e-5 (under −96 dB, where a bar is empty).
    fn step(self, dt: f32, target: Levels) -> Levels {
        let k = 0.9f32.powf(dt * 60.0);
        let smooth = |disp: f32, target: f32| match target + (disp - target) * k {
            v if (v - target).abs() < 1e-5 => target,
            v => v,
        };
        Levels {
            gain: smooth(self.gain, target.gain),
            input: smooth(self.input, target.input),
            output: smooth(self.output, target.output),
        }
    }
}

/// The IN/OUT scale's labels, bottom to top, verbatim with their padding (meter step 3).
const IN_OUT: [&str; 9] = [
    " -inf ", " -48 ", " -24 ", " -12 ", " -6 ", " -3 ", " 0 ", " 3 ", " 6 ",
];

/// The GR, IN and OUT bars, in points: 0.2 U wide at c − 2.2 U, c − 1.0 U and c + 0.2 U, c = R − 1.0 U, from
/// top = y + 0.4 U to bottom = y + h − 0.8 U (so barH = h − 1.2 U).
fn bars(rect: Rect, m: &Metrics) -> [Rect; 3] {
    let c = rect.right() - m.u(1.0);
    let y = rect.top() + m.u(0.4)..=rect.bottom() - m.u(0.8);
    [-2.2, -1.0, 0.2].map(|x| Rect::from_x_y_ranges(c + m.u(x)..=c + m.u(x + 0.2), y.clone()))
}

/// The 9 ticks' y (meter step 3), bottom to top, from a bar: (bottom − 1 px) + i·((top + 2 px) − (bottom − 1 px))/8.
fn ticks(bar: Rect, ppp: f32) -> [f32; 9] {
    let (y0, y8) = (bar.bottom() - 1.0 / ppp, bar.top() + 2.0 / ppp);
    std::array::from_fn(|i| y0 + i as f32 * (y8 - y0) / 8.0)
}

/// The GR scale's label `i` (meter step 4): `" %.0f "` of 20·log10((i/8)^2.4), −60 when (i/8)^2.4 ≤ 0.001.
fn gr_label(i: usize) -> String {
    let g = (i as f32 / 8.0).powf(2.4);
    let db = if g <= 0.001 { -60.0 } else { 20.0 * g.log10() };
    format!(" {db:.0} ")
}

/// D(x) (meter step 6): x/3 for |x| < 3, else sign(x)·(log2(|x|/3) + 1).
fn d(x: f32) -> f32 {
    if x.abs() < 3.0 {
        x / 3.0
    } else {
        x.signum() * ((x.abs() / 3.0).log2() + 1.0)
    }
}

/// An IN/OUT bar's height as a fraction of barH (meter step 6): (clamp(D(dB), −6, 2) + 6)/8, a peak ≤ 0 being −96 dB.
fn level_fraction(peak: f32) -> f32 {
    let db = if peak > 0.0 {
        20.0 * peak.log10()
    } else {
        -96.0
    };
    (d(db).clamp(-6.0, 2.0) + 6.0) / 8.0
}

/// The GR bar's length as a fraction of barH (meter step 6): 1 − g^(1/2.3). The labels use 2.4; both are kept.
fn gr_fraction(gain: f32) -> f32 {
    1.0 - gain.clamp(0.0, 1.0).powf(1.0 / 2.3)
}

/// The meter (#9) from the module's meter feed, smoothed; it repaints while it moves or OUT is non-silent.
fn level_meter(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let target = live_meter(ui, module.instance_id).map_or(REST, Levels::target);
    let levels = animate(ui, id, REST, |levels, dt| levels.step(dt, target));
    meter(ui.painter(), cx, rect, ink, levels);
    // While easing, or while there is input or output: a GR held with silence (a Compressor switched off
    // mid-reduction) shows, but is not repainted for.
    if levels != target || target.output > 0.0 || target.input > 0.0 {
        ui.ctx().request_repaint();
    }
    Vec::new()
}

/// The meter (#9), steps 1–7, showing `levels` in the channel colour `ink`.
fn meter(painter: &Painter, cx: &UiCtx, rect: Rect, ink: Color32, levels: Levels) {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let (module_fill, text) = (colors.module_fill.to_color32(), colors.text.to_color32());
    let (px, small) = (1.0 / m.ppp(), m.font_id(FontSize::Pt6));
    let outline = Stroke::new(px, colors.background.to_color32());
    let tick = ink.gamma_multiply(0.75);
    let [gr, input, output] = bars(rect, m);
    // 1. The frame.
    painter.rect(rect, 0.0, module_fill, outline, StrokeKind::Inside);
    // 2. Captions, each centred in a 0.4 × 0.75 U box from its bar's x − 0.1 U, at y + h − 0.75 U.
    for (caption, bar) in [("OUT", output), ("IN", input), ("GR", gr)] {
        let at = pos2(bar.center().x, rect.bottom() - m.u(0.375));
        painter.text(
            at,
            Align2::CENTER_CENTER,
            caption,
            m.font_id(FontSize::Pt8),
            ink,
        );
    }
    let ys = ticks(gr, m.ppp());
    // 3. The IN/OUT scale: ticks across the 1 U gap, each label centred on a `moduleFill` box as wide as itself.
    let mid = (input.right() + output.left()) / 2.0;
    for (&y, label) in ys.iter().zip(IN_OUT) {
        let line = Rect::from_x_y_ranges(input.right()..=output.left(), y..=y + px);
        painter.rect_filled(line, 0.0, tick);
        let galley = painter.layout_no_wrap(label.to_owned(), small.clone(), text);
        let label = Rect::from_center_size(pos2(mid, y), vec2(galley.size().x, m.u(0.2)));
        painter.rect_filled(label, 0.0, module_fill);
        let at = Align2::CENTER_CENTER.anchor_size(label.center(), galley.size());
        painter.galley(at.min, galley, text);
    }
    // 4. The GR scale: 0.8 U ticks, their labels left-aligned on a 0.6 U box 0.2 U right of the bar.
    for (i, &y) in ys.iter().enumerate() {
        let line = Rect::from_min_size(pos2(gr.right(), y), vec2(m.u(0.8), px));
        painter.rect_filled(line, 0.0, tick);
        let label = Rect::from_min_size(
            pos2(gr.right() + m.u(0.2), y - m.u(0.1)),
            vec2(m.u(0.6), m.u(0.2)),
        );
        painter.rect_filled(label, 0.0, module_fill);
        painter.text(
            label.left_center(),
            Align2::LEFT_CENTER,
            gr_label(i),
            small.clone(),
            text,
        );
    }
    // 5. Troughs.
    for bar in [gr, input, output] {
        painter.rect_filled(bar, 0.0, colors.control_fill.to_color32());
    }
    // 6. Bars: GR down from the top, IN and OUT up from the bottom.
    let bar_h = gr.height();
    let fills = [
        gr.with_max_y(gr.top() + gr_fraction(levels.gain) * bar_h),
        input.with_min_y(input.bottom() - level_fraction(levels.input) * bar_h),
        output.with_min_y(output.bottom() - level_fraction(levels.output) * bar_h),
    ];
    for fill in fills.into_iter().filter(|f| f.height() > 0.0) {
        painter.rect_filled(fill, 0.0, ink);
    }
    // 7. Outlines.
    for bar in [gr, input, output] {
        painter.rect_stroke(bar, 0.0, outline, StrokeKind::Inside);
    }
}

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        hstack([
            v_slider(1, "Input gain").p(8).value_text(),
            v_slider(2, "Thresh").p(1).value_text(),
            v_slider(3, "Ratio").p(2).value_text(),
            v_slider(4, "Attack").p(3).value_text(),
            v_slider(5, "Hold").p(6).value_text(),
            v_slider(6, "Release").p(4).value_text(),
            v_slider(7, "Level").p(5).value_text(),
            toggle(8, "Auto gain").p(7).value_text(),
        ])
        .margins(0.0, 0.25, 0.0, 0.25),
        // The GR / IN / OUT meter.
        display(9).w(Size::U(3.75)).mr(0.5).draw(level_meter),
    ])))
}

#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 5.49],
    settings: &[],
    cases: &[Case {
        rows: &[
            row(1, Widget::VSlider, "Input gain", [0.25, 0.0, 1.15, 5.49]).p(8).value_text(),
            row(2, Widget::VSlider, "Thresh", [1.4, 0.0, 1.16, 5.49]).p(1).value_text(),
            row(3, Widget::VSlider, "Ratio", [2.56, 0.0, 1.15, 5.49]).p(2).value_text(),
            row(4, Widget::VSlider, "Attack", [3.71, 0.0, 1.16, 5.49]).p(3).value_text(),
            row(5, Widget::VSlider, "Hold", [4.87, 0.0, 1.16, 5.49]).p(6).value_text(),
            row(6, Widget::VSlider, "Release", [6.03, 0.0, 1.15, 5.49]).p(4).value_text(),
            row(7, Widget::VSlider, "Level", [7.18, 0.0, 1.16, 5.49]).p(5).value_text(),
            row(8, Widget::Toggle, "Auto gain", [8.34, 0.0, 1.16, 5.49]).p(7).value_text(),
            row(9, Widget::Display, "", [9.75, 0.0, 3.75, 5.49]).w(Size::U(3.75)).margins(0.0, 0.5, 0.0, 0.0),
        ],
        ..Case::DEFAULT
    }],
};

#[cfg(test)]
mod tests {
    use eframe::egui::Shape;

    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE_INDEX;
    use crate::panels::tests::{U, display_shapes};
    use crate::theme::theme_by_id;

    /// #9's rect at the reference: 9.75, 0, 3.75, 5.49 U.
    fn nine() -> Rect {
        Rect::from_min_size(pos2(9.75 * U, 0.0), vec2(3.75 * U, 5.49 * U))
    }

    fn metrics(ppp: f32) -> Metrics {
        Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn the_bars_sit_where_the_spec_puts_them() {
        // GR 10.3–10.5, IN 11.5–11.7, OUT 12.7–12.9 U; top 0.4, bottom 4.69 U.
        for ppp in [1.0, 2.0] {
            let bars = bars(nine(), &metrics(ppp));
            for (bar, (x0, x1)) in bars.iter().zip([(10.3, 10.5), (11.5, 11.7), (12.7, 12.9)]) {
                assert!(
                    close(bar.left(), x0 * U) && close(bar.right(), x1 * U),
                    "ppp {ppp}: {bar:?}"
                );
                assert!(
                    close(bar.top(), 0.4 * U) && close(bar.bottom(), 4.69 * U),
                    "ppp {ppp}: {bar:?}"
                );
            }
        }
    }

    #[test]
    fn the_nine_ticks_run_from_a_pixel_above_the_bottom_to_two_below_the_top() {
        // y_i = (bottom − 1 px) + i·((top + 2 px) − (bottom − 1 px))/8: at ppp 1 from 150.08 − 1 to 12.8 + 2 pt, at
        // ppp 2 from 150.08 − 0.5 to 12.8 + 1 pt.
        let at_1 = [
            149.08, 132.295, 115.51, 98.725, 81.94, 65.155, 48.37, 31.585, 14.8,
        ];
        let at_2 = [
            149.58, 132.6075, 115.635, 98.6625, 81.69, 64.7175, 47.745, 30.7725, 13.8,
        ];
        for (ppp, want) in [(1.0, at_1), (2.0, at_2)] {
            let [gr, ..] = bars(nine(), &metrics(ppp));
            let got = ticks(gr, ppp);
            assert!(
                got.iter().zip(want).all(|(&y, w)| close(y, w)),
                "ppp {ppp}: {got:?}"
            );
        }
    }

    #[test]
    fn both_scales_are_labelled_verbatim() {
        let in_out = [
            " -inf ", " -48 ", " -24 ", " -12 ", " -6 ", " -3 ", " 0 ", " 3 ", " 6 ",
        ];
        assert_eq!(IN_OUT, in_out);
        let gr: Vec<String> = (0..9).map(gr_label).collect();
        assert_eq!(
            gr,
            [
                " -60 ", " -43 ", " -29 ", " -20 ", " -14 ", " -10 ", " -6 ", " -3 ", " 0 "
            ]
        );
    }

    #[test]
    fn every_in_out_label_sits_on_an_equal_step_and_rest_draws_no_bar() {
        // D maps −96 … 6 dB to −6 … 2.
        let db = [-96.0, -48.0, -24.0, -12.0, -6.0, -3.0, 0.0, 3.0, 6.0];
        for (i, x) in db.into_iter().enumerate() {
            assert!(close(d(x), i as f32 - 6.0), "D({x}) = {}", d(x));
        }
        // At rest: silence (≤ 0 is −96 dB) fills nothing, nor does no reduction; full scale is 6/8.
        assert_eq!(
            (level_fraction(REST.input), level_fraction(REST.output)),
            (0.0, 0.0)
        );
        assert_eq!(gr_fraction(REST.gain), 0.0);
        assert!(close(level_fraction(1.0), 0.75) && close(level_fraction(1e9), 1.0));
        assert!(
            close(gr_fraction(0.0), 1.0) && close(gr_fraction(0.5), 1.0 - 0.5f32.powf(1.0 / 2.3))
        );
    }

    #[test]
    fn the_feed_sets_gr_from_its_db_and_out_from_the_louder_channel() {
        let m = MeterSnapshot {
            peak_left: 0.25,
            peak_right: 0.5,
            gain_reduction_db: 20.0 * 2f32.log10(),
            input_peak: 0.75,
        };
        let t = Levels::target(m);
        assert!(
            close(t.gain, 0.5) && t.output == 0.5 && t.input == 0.75,
            "{t:?}"
        );
        // A silent feed is rest, so the meter comes to rest exactly; a bad value cannot stick.
        assert_eq!(Levels::target(MeterSnapshot::default()), REST);
        let bad = MeterSnapshot {
            peak_left: f32::NAN,
            peak_right: f32::NAN,
            gain_reduction_db: f32::NAN,
            input_peak: f32::NAN,
        };
        assert_eq!(Levels::target(bad), REST);
        // The engine's residue (its anti-denormal 1e-30) is silence; an infinite peak is +12 dB, full reduction 0.
        let residue = MeterSnapshot {
            peak_left: 1e-30,
            peak_right: 1e-30,
            gain_reduction_db: 0.0,
            input_peak: 1e-30,
        };
        assert_eq!(Levels::target(residue), REST);
        let wild = MeterSnapshot {
            peak_left: f32::INFINITY,
            peak_right: 0.0,
            gain_reduction_db: f32::INFINITY,
            input_peak: f32::INFINITY,
        };
        let t = Levels::target(wild);
        assert!(t.output == 4.0 && t.input == 4.0 && t.gain == 0.0, "{t:?}");
        let t = REST.step(1.0 / 60.0, t).step(10.0, REST);
        assert_eq!(t, REST);
    }

    #[test]
    fn the_in_bar_rises_from_the_feeds_input_peak() {
        use crate::panels::fx::leveller::pass;
        // A 0 dB input peak with a silent output: a 60th of a second on, IN shows a tenth of it (−20 dB, D = −3.74), OUT
        // nothing.
        let (theme, m) = (theme_by_id(0), metrics(2.0));
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let ctx = eframe::egui::Context::default();
        let feed = MeterSnapshot {
            input_peak: 1.0,
            ..Default::default()
        };
        let draw = |ui: &mut Ui| {
            let mut module = ModuleInstance::new(8, 2);
            let mut state = PanelState::default();
            level_meter(
                ui,
                &cx,
                Id::new("meter"),
                nine(),
                &mut module,
                &mut state,
                Color32::RED,
            );
        };
        pass(&ctx, 0.0, 48_000.0, Some(feed), draw);
        let shapes = pass(&ctx, 1.0 / 60.0, 48_000.0, Some(feed), draw);
        let [_, input, output] = bars(nine(), &m);
        let fills: Vec<Rect> = (shapes.iter())
            .filter_map(|s| match s {
                Shape::Rect(r) if r.fill == Color32::RED => Some(r.rect),
                _ => None,
            })
            .collect();
        let want = level_fraction(0.1) * input.height();
        assert!(
            fills
                .iter()
                .any(|f| close(f.left(), input.left()) && close(f.height(), want)),
            "{fills:?}"
        );
        assert!(
            !fills.iter().any(|f| close(f.left(), output.left())),
            "{fills:?}"
        );
    }

    #[test]
    fn each_value_moves_a_tenth_of_the_way_per_60th_of_a_second() {
        let target = Levels {
            gain: 0.5,
            input: 0.0,
            output: 0.8,
        };
        let one = REST.step(1.0 / 60.0, target);
        assert!(
            close(one.gain, 0.95) && close(one.output, 0.08) && one.input == 0.0,
            "{one:?}"
        );
        // Stepped by time, not by calls: two half frames are one frame; no time changes nothing.
        let halves = REST.step(1.0 / 120.0, target).step(1.0 / 120.0, target);
        assert!(
            close(halves.gain, one.gain) && close(halves.output, one.output),
            "{halves:?}"
        );
        assert_eq!(one.step(0.0, target), one);
        // Within 1e-5 it lands: back at rest exactly.
        let mut back = target;
        for _ in 0..200 {
            back = back.step(1.0 / 60.0, REST);
        }
        assert_eq!(back, REST);
    }

    #[test]
    fn the_meter_at_rest_draws_steps_1_to_7_in_order() {
        let colors = theme_by_id(0).colors;
        let (module, control, text) = (
            colors.module_fill.to_color32(),
            colors.control_fill.to_color32(),
            colors.text.to_color32(),
        );
        let tick = Color32::RED.gamma_multiply(0.75);
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 5.49 * U));
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            let outline = Stroke::new(1.0 / ppp, colors.background.to_color32());
            let shapes = display_shapes(8, 9, &[], &PanelState::default(), content, ppp);
            let name = |s: &Shape| match s {
                Shape::Rect(r) if r.fill == module && r.stroke == outline => "frame".to_owned(),
                Shape::Rect(r) if r.fill == module => "box".to_owned(),
                Shape::Rect(r) if r.fill == control => "trough".to_owned(),
                Shape::Rect(r) if r.fill == tick => "tick".to_owned(),
                Shape::Rect(r) if r.fill == Color32::TRANSPARENT && r.stroke == outline => {
                    "outline".to_owned()
                }
                Shape::Text(t) => {
                    let format = &t.galley.job.sections[0].format;
                    let ink = match format.color {
                        c if c == text => "text",
                        Color32::RED => "channel",
                        _ => "?",
                    };
                    format!("{} {ink} {}", t.galley.text(), format.font_id.size)
                }
                other => format!("{other:?}"),
            };
            let (pt6, pt8) = (m.font_id(FontSize::Pt6).size, m.font_id(FontSize::Pt8).size);
            let mut want = vec!["frame".to_owned()];
            want.extend(["OUT", "IN", "GR"].map(|c| format!("{c} channel {pt8}")));
            let labels = IN_OUT
                .map(str::to_owned)
                .into_iter()
                .chain((0..9).map(gr_label));
            for label in labels {
                want.extend([
                    "tick".to_owned(),
                    "box".to_owned(),
                    format!("{label} text {pt6}"),
                ]);
            }
            want.extend(
                [
                    "trough", "trough", "trough", "outline", "outline", "outline",
                ]
                .map(str::to_owned),
            );
            assert_eq!(
                shapes.iter().map(name).collect::<Vec<_>>(),
                want,
                "ppp {ppp}"
            );
        }
    }

    #[test]
    fn captions_ticks_and_label_boxes_hang_off_the_bars() {
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 5.49 * U));
        let module = theme_by_id(0).colors.module_fill.to_color32();
        let tick = Color32::RED.gamma_multiply(0.75);
        for ppp in [1.0, 2.0] {
            let shapes = display_shapes(8, 9, &[], &PanelState::default(), content, ppp);
            let Shape::Rect(frame) = &shapes[0] else {
                panic!()
            };
            let [gr, input, output] = bars(frame.rect, &metrics(ppp));
            // Captions: centred on their bar, in #9's bottom 0.75 U.
            for (shape, bar) in shapes[1..4].iter().zip([output, input, gr]) {
                let Shape::Text(t) = shape else {
                    panic!("{shape:?}")
                };
                let at = t.galley.rect.translate(t.pos.to_vec2());
                assert!(
                    (at.center().x - bar.center().x).abs() < 0.5,
                    "ppp {ppp}: {at:?} on {bar:?}"
                );
                assert!(
                    (at.center().y - (frame.rect.bottom() - 0.375 * U)).abs() < 0.5,
                    "ppp {ppp}: {at:?}"
                );
            }
            let rects: Vec<Rect> = shapes
                .iter()
                .filter_map(|s| match s {
                    Shape::Rect(r)
                        if r.fill == tick || (r.fill == module && r.stroke.width == 0.0) =>
                    {
                        Some(r.rect)
                    }
                    _ => None,
                })
                .collect();
            let ys = ticks(gr, ppp);
            let px = 1.0 / ppp;
            for (i, &y) in ys.iter().enumerate() {
                // IN/OUT: the tick spans the gap between the bars; its box is centred in it, 0.2 U tall from y − 0.1 U.
                let (line, label) = (rects[2 * i], rects[2 * i + 1]);
                assert!(
                    close(line.left(), input.right()) && close(line.right(), output.left()),
                    "{line:?}"
                );
                assert!(close(line.top(), y) && close(line.height(), px), "{line:?}");
                assert!(
                    close(label.center().x, (input.right() + output.left()) / 2.0),
                    "{label:?}"
                );
                assert!(
                    close(label.top(), y - 0.1 * U) && close(label.height(), 0.2 * U),
                    "{label:?}"
                );
                // GR: the tick 0.8 U from the bar's right edge; its box 0.6 U wide, 0.2 U right of the bar.
                let (line, label) = (rects[18 + 2 * i], rects[18 + 2 * i + 1]);
                assert!(
                    close(line.left(), gr.right()) && close(line.width(), 0.8 * U),
                    "{line:?}"
                );
                assert!(close(line.top(), y) && close(line.height(), px), "{line:?}");
                assert!(
                    close(label.left(), gr.right() + 0.2 * U) && close(label.width(), 0.6 * U),
                    "{label:?}"
                );
                assert!(
                    close(label.top(), y - 0.1 * U) && close(label.height(), 0.2 * U),
                    "{label:?}"
                );
            }
        }
    }

    /// The rects `meter` fills in the channel colour (red) for `levels` in #9 at the reference.
    fn bars_drawn(levels: Levels, ppp: f32) -> Vec<Rect> {
        let (theme, m) = (theme_by_id(0), metrics(ppp));
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let mut raw = eframe::egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            ..Default::default()
        };
        raw.viewports
            .entry(raw.viewport_id)
            .or_default()
            .native_pixels_per_point = Some(ppp);
        let output = eframe::egui::Context::default().run(raw, |ctx| {
            let ui = Ui::new(ctx.clone(), Id::new("meter"), Default::default());
            meter(ui.painter(), &cx, nine(), Color32::RED, levels);
        });
        (output.shapes.into_iter())
            .filter_map(|c| match c.shape {
                Shape::Rect(r) if r.fill == Color32::RED => Some(r.rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn gr_hangs_from_the_top_and_in_and_out_stand_on_the_bottom() {
        // Gain 0.5; IN −12 dB (D = −3, so 3/8); OUT 0 dB (6/8).
        let levels = Levels {
            gain: 0.5,
            input: 10f32.powf(-12.0 / 20.0),
            output: 1.0,
        };
        for ppp in [1.0, 2.0] {
            let [gr, input, output] = bars(nine(), &metrics(ppp));
            let bar_h = gr.height();
            let drawn = bars_drawn(levels, ppp);
            let want = [
                gr.with_max_y(gr.top() + (1.0 - 0.5f32.powf(1.0 / 2.3)) * bar_h),
                input.with_min_y(input.bottom() - 3.0 / 8.0 * bar_h),
                output.with_min_y(output.bottom() - 6.0 / 8.0 * bar_h),
            ];
            assert_eq!(drawn.len(), 3, "ppp {ppp}: {drawn:?}");
            for (got, want) in drawn.iter().zip(want) {
                assert!(
                    got.min.distance(want.min) < 1e-3 && got.max.distance(want.max) < 1e-3,
                    "ppp {ppp}: {got:?} {want:?}"
                );
            }
            // Full reduction fills GR; silence draws no IN or OUT bar.
            let full = Levels { gain: 0.0, ..REST };
            let [all] = bars_drawn(full, ppp)[..] else {
                panic!("ppp {ppp}")
            };
            assert!(
                all.min.distance(gr.min) < 1e-3 && all.max.distance(gr.max) < 1e-3,
                "ppp {ppp}: {all:?}"
            );
        }
    }
}

//! Leveller (ID 20), UIHeight 5 U.

use eframe::egui::{Color32, Id, Painter, Rect, Ui, pos2};
use honeydew_audio::MeterSnapshot;

use crate::metrics::Metrics;
use crate::panels::build::*;
use crate::ui::UiCtx;

/// One channel's meter, linear amplitudes: the decaying peak and the peak hold, and how long the hold has stood, in
/// seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Level {
    peak: f32,
    hold: f32,
    held: f32,
}

/// The meter at rest, and without a meter feed (outside a rack).
const SILENCE: Level = Level {
    peak: 0.0,
    hold: 0.0,
    held: 0.0,
};

/// The audio buffer of the meter ballistics, in frames: the engine's block.
const FRAMES: f32 = 1024.0;

/// One audio buffer in seconds with no audio running: [`FRAMES`] at 44.1 kHz.
const BUFFER: f32 = FRAMES / 44_100.0;

/// One audio buffer in seconds at the audio's sample rate, which the rack lends the panels
/// ([`crate::panels::sample_rate`]); [`BUFFER`] without audio.
pub(super) fn buffer(ui: &Ui) -> f32 {
    let rate = crate::panels::sample_rate(ui);
    if rate.is_finite() && rate > 0.0 {
        FRAMES / rate
    } else {
        BUFFER
    }
}

/// A peak meter's peak after `dt` seconds: `peak` decays ×0.95 per audio `buffer` (s), then rises at once to the latest
/// block peak `target`. A decayed peak at or under 0.001 (−60 dB, the floor of the scales that use it) is silence, so a
/// meter comes to rest.
pub(super) fn decay(peak: f32, dt: f32, buffer: f32, target: f32) -> f32 {
    let decayed = peak * 0.95f32.powf(dt / buffer);
    let decayed = if decayed > 0.001 { decayed } else { 0.0 };
    decayed.max(target)
}

impl Level {
    /// The meter ballistics after `dt` seconds with the latest block peak `target`, per audio `buffer` (s): the peak as
    /// [`decay`]; the hold rises with the peak, stays put for 1 s, then falls by 0.005 (linear) per buffer, never
    /// under the peak.
    fn step(self, dt: f32, buffer: f32, target: f32) -> Level {
        let peak = decay(self.peak, dt, buffer, target);
        let held = self.held + dt;
        // Only the part of this step past the second falls.
        let falling = (held - self.held.max(1.0)).max(0.0);
        let hold = (self.hold - 0.005 * falling / buffer).max(0.0);
        if peak >= hold {
            Level {
                peak,
                hold: peak,
                held: 0.0,
            }
        } else {
            Level { peak, hold, held }
        }
    }
}

/// A feed peak as the meters take it: at most 4 (+12 dB, the top of every scale), and a NaN is silence, so a wild
/// value cannot stick in a meter's state.
pub(super) fn bounded(peak: f32) -> f32 {
    // Not `clamp`, which keeps a NaN.
    if peak > 0.0 { peak.min(4.0) } else { 0.0 }
}

/// The module's latest meter, which the rack posts in egui temp data before it draws the panel
/// ([`super::meter_id`]); `None` outside a rack or without a running stream. While there is one, the next
/// frame comes within 50 ms: audio can change with no input to repaint for (a held note while stopped), and a meter at
/// rest asks for no frame of its own.
pub(super) fn live_meter(ui: &Ui, instance_id: i32) -> Option<MeterSnapshot> {
    let at = super::meter_id(ui, instance_id);
    let meter = ui.data(|d| d.get_temp(at));
    if meter.is_some() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
    meter
}

/// Steps the meter state kept under `id` (`rest` the first time) by the input time since its last step, and keeps the
/// result. Input time, not draw calls: a frame's second pass steps by 0.
pub(super) fn animate<T: Clone + Send + Sync + 'static>(
    ui: &Ui,
    id: Id,
    rest: T,
    step: impl FnOnce(T, f32) -> T,
) -> T {
    let now = ui.input(|i| i.time);
    let (then, state) = ui.data(|d| d.get_temp(id)).unwrap_or((now, rest));
    let state = step(state, (now - then).max(0.0) as f32);
    ui.data_mut(|d| d.insert_temp(id, (now, state.clone())));
    state
}

/// A level's place on the meter as a fraction of its width: min(dB/72 + 5/6, 1), −60 dB when ≤ 0.001. So
/// −60 dB is at the left, 0 dB at 5/6 and +12 dB at the right.
fn fraction(level: f32) -> f32 {
    let db = if level <= 0.001 {
        -60.0
    } else {
        20.0 * level.log10()
    };
    (db / 72.0 + 5.0 / 6.0).min(1.0)
}

/// The peak bar: from the left edge over f·w, its edges on the whole device pixel at or below them, full
/// height.
fn bar(rect: Rect, f: f32, m: &Metrics) -> Rect {
    Rect::from_x_y_ranges(
        m.floor_px(rect.left())..=m.floor_px(rect.left() + f * rect.width()),
        rect.y_range(),
    )
}

/// The peak-hold tick: the device pixels from p to p + 2, p = x + f·w − 1 px, floored, so about 2 px centred
/// on x + f·w; only while the hold is > 0.001 and strictly between −60 and +12 dB.
fn hold_tick(rect: Rect, hold: f32, m: &Metrics) -> Option<Rect> {
    let shown = hold > 0.001 && 20.0 * hold.log10() < 12.0;
    let (x, px) = (rect.left() + fraction(hold) * rect.width(), 1.0 / m.ppp());
    shown.then(|| Rect::from_x_y_ranges(m.floor_px(x - px)..=m.floor_px(x + px), rect.y_range()))
}

/// #2's labels as (text, centre x): `-inf.` on the left edge, then each v at x + w·(v + 60)/72, so `12` is on the right
/// edge.
fn scale(rect: Rect) -> Vec<(String, f32)> {
    let at = |v: i32| rect.left() + rect.width() * (v + 60) as f32 / 72.0;
    let values = [-50, -40, -30, -20, -10, -5, 0, 3, 6, 9, 12].map(|v| (v.to_string(), at(v)));
    std::iter::once(("-inf.".to_owned(), rect.left()))
        .chain(values)
        .collect()
}

/// `controlFill` trough, then the peak bar and the hold tick in the channel colour `ink`.
fn meter(painter: &Painter, cx: &UiCtx, rect: Rect, ink: Color32, level: Level) {
    painter.rect_filled(rect, 0.0, cx.theme.colors.control_fill.to_color32());
    let peak = bar(rect, fraction(level.peak), cx.m);
    if peak.width() > 0.0 {
        painter.rect_filled(peak, 0.0, ink);
    }
    if let Some(tick) = hold_tick(rect, level.hold, cx.m) {
        painter.rect_filled(tick, 0.0, ink);
    }
}

/// A channel's meter from the module's meter feed (its `peak`), with the meter ballistics; it repaints until it rests.
fn live(
    ui: &Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    module: &ModuleInstance,
    ink: Color32,
    peak: fn(MeterSnapshot) -> f32,
) {
    let target = live_meter(ui, module.instance_id).map_or(SILENCE.peak, |m| bounded(peak(m)));
    // At or under −60 dB the meter draws nothing: the engine's residue (its anti-denormal 1e-30) rests as silence.
    let target = if target > 0.001 { target } else { 0.0 };
    let block = buffer(ui);
    let level = animate(ui, id, SILENCE, |level, dt| level.step(dt, block, target));
    meter(ui.painter(), cx, rect, ink, level);
    if level != SILENCE {
        ui.ctx().request_repaint();
    }
}

/// #1, the left channel's meter.
fn left(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    live(ui, cx, id, rect, module, ink, |m| m.peak_left);
    Vec::new()
}

/// #3, the right channel's meter.
fn right(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    live(ui, cx, id, rect, module, ink, |m| m.peak_right);
    Vec::new()
}

/// #2, the scale: each label centred in a 1 U box spanning #2's height, `text`, FontSize 1.
fn labels(
    ui: &mut Ui,
    cx: &UiCtx,
    _: Id,
    rect: Rect,
    _: &mut ModuleInstance,
    _: &mut PanelState,
    _: Color32,
) -> Vec<usize> {
    let (font, text) = (
        cx.m.font_id(FontSize::Pt8),
        cx.theme.colors.text.to_color32(),
    );
    for (label, x) in scale(rect) {
        let at = pos2(x, rect.center().y);
        ui.painter()
            .text(at, Align2::CENTER_CENTER, label, font.clone(), text);
    }
    Vec::new()
}

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        vstack([
            // The left meter, the scale and the right meter.
            vstack([
                display(1).h(Size::U(0.15)).draw(left),
                display(2).draw(labels),
                display(3).h(Size::U(0.15)).draw(right),
            ])
            .margins(0.0, 0.5, 0.35, 0.5),
            // The main line is the gain, FontSize 3; no value strip.
            h_slider(4, "0 dB")
                .p(1)
                .h(Size::U(1.5))
                .font(FontSize::Pt12)
                .label_fn(gain),
        ])
        .w(Size::Flex(4.0)),
        knob(5, "Pan").p(2),
    ])))
}

/// p1's text: `0 dB`, `+0.5 dB`, `-12.4 dB`.
fn gain(params: &[f32]) -> String {
    crate::params::Display::GainCurve.text(params.get(1).copied().unwrap_or(0.0))
}

/// One egui pass at input time `t` s on a bare `Ui`, for the tests of the meter draws: the rack lends `rate` Hz
/// ([`crate::panels::set_text_env`]) and module 2's meter is `feed` (`None`: no stream). Returns the shapes painted.
#[cfg(test)]
pub(super) fn pass(
    ctx: &eframe::egui::Context,
    t: f64,
    rate: f32,
    feed: Option<MeterSnapshot>,
    draw: impl FnOnce(&mut Ui),
) -> Vec<eframe::egui::Shape> {
    feed_pass(ctx, t, rate, feed.map(|m| (m, Vec::new())), draw)
}

/// [`pass`] with module 2's meter and analysis, posted as the rack posts them ([`super::post_feed`]).
#[cfg(test)]
pub(super) fn feed_pass(
    ctx: &eframe::egui::Context,
    t: f64,
    rate: f32,
    feed: Option<(MeterSnapshot, Vec<f32>)>,
    draw: impl FnOnce(&mut Ui),
) -> Vec<eframe::egui::Shape> {
    let raw = eframe::egui::RawInput {
        time: Some(t),
        ..Default::default()
    };
    let mut draw = Some(draw);
    let output = ctx.run(raw, |ctx| {
        crate::panels::set_text_env(ctx, 120.0, rate);
        let mut ui = Ui::new(ctx.clone(), Id::new("meter"), Default::default());
        let feed = feed.as_ref().map(|(m, slab)| (*m, &slab[..]));
        super::post_feed(&ui, 2, feed);
        (draw.take().expect("one pass"))(&mut ui);
    });
    output.shapes.into_iter().map(|c| c.shape).collect()
}

#[cfg(test)]
const LEVELLER: [Row; 4] = [
    row(1, Widget::Display, "", [0.5, 0.0, 10.2, 0.15]).h(Size::U(0.15)),
    row(2, Widget::Display, "", [0.5, 0.15, 10.2, 0.84]),
    row(3, Widget::Display, "", [0.5, 0.99, 10.2, 0.15]).h(Size::U(0.15)),
    row(5, Widget::Knob, "Pan", [11.2, 0.0, 2.8, 2.99]).p(2),
];

#[cfg(test)]
const fn gain_row(label: &'static str) -> Row {
    row(4, Widget::HSlider, label, [0.0, 1.49, 11.2, 1.5])
        .p(1)
        .font(FontSize::Pt12)
        .h(Size::U(1.5))
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 2.99],
    settings: &[],
    cases: &[
        Case {
            rows: &[
                LEVELLER[0],
                LEVELLER[1],
                LEVELLER[2],
                gain_row("0 dB"),
                LEVELLER[3],
            ],
            ..Case::DEFAULT
        },
        // The main line is the gain.
        Case {
            name: "+12 dB",
            params: &[(1, 1.0)],
            rows: &[
                LEVELLER[0],
                LEVELLER[1],
                LEVELLER[2],
                gain_row("+12.0 dB"),
                LEVELLER[3],
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "-60 dB",
            params: &[(1, 0.0)],
            rows: &[
                LEVELLER[0],
                LEVELLER[1],
                LEVELLER[2],
                gain_row("-60.0 dB"),
                LEVELLER[3],
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use eframe::egui::{Rangef, Shape, pos2, vec2};

    use super::*;
    use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
    use crate::panels::fx::limiter::tests::metrics;
    use crate::panels::tests::{U, display_shapes};
    use crate::theme::theme_by_id;

    fn content() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 2.99 * U))
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn the_scale_runs_from_minus_60_to_plus_12_db() {
        // #2 at the reference: 0.5, 0.15, 10.2, 0.84 U. `-inf.` on the left edge, v at x + w·(v + 60)/72.
        let two = Rect::from_min_size(pos2(0.5 * U, 0.15 * U), vec2(10.2 * U, 0.84 * U));
        let got = scale(two);
        let want = [
            ("-inf.", 0.5),
            ("-50", 0.5 + 10.2 * 10.0 / 72.0),
            ("-40", 0.5 + 10.2 * 20.0 / 72.0),
            ("-30", 0.5 + 10.2 * 30.0 / 72.0),
            ("-20", 0.5 + 10.2 * 40.0 / 72.0),
            ("-10", 0.5 + 10.2 * 50.0 / 72.0),
            ("-5", 0.5 + 10.2 * 55.0 / 72.0),
            ("0", 0.5 + 10.2 * 60.0 / 72.0),
            ("3", 0.5 + 10.2 * 63.0 / 72.0),
            ("6", 0.5 + 10.2 * 66.0 / 72.0),
            ("9", 0.5 + 10.2 * 69.0 / 72.0),
            ("12", 10.7),
        ];
        assert_eq!(got.len(), want.len());
        for ((text, x), (w_text, w_x)) in got.iter().zip(want) {
            assert!(
                text == w_text && close(*x, w_x * U),
                "{text} at {x}, want {w_text} at {}",
                w_x * U
            );
        }
    }

    #[test]
    fn the_scale_draws_in_text_at_fontsize_1_and_12_overhangs_the_right_edge() {
        let text = theme_by_id(0).colors.text.to_color32();
        for ppp in [1.0, 2.0] {
            let pt8 = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX)
                .font_id(FontSize::Pt8)
                .size;
            let shapes = display_shapes(20, 2, &[], &PanelState::default(), content(), ppp);
            let texts: Vec<_> = shapes
                .iter()
                .map(|s| match s {
                    Shape::Text(t) => t,
                    other => panic!("ppp {ppp}: {other:?}"),
                })
                .collect();
            let names: Vec<&str> = texts.iter().map(|t| t.galley.text()).collect();
            assert_eq!(
                names,
                [
                    "-inf.", "-50", "-40", "-30", "-20", "-10", "-5", "0", "3", "6", "9", "12"
                ]
            );
            for t in &texts {
                let format = &t.galley.job.sections[0].format;
                assert_eq!(
                    (format.color, format.font_id.size),
                    (text, pt8),
                    "ppp {ppp}: {}",
                    t.galley.text()
                );
            }
            // #2 spans 0.5 … 10.7 U: `-inf.` centred on its left edge, `12` on its right, past it.
            let at =
                |t: &&&eframe::egui::epaint::TextShape| t.galley.rect.translate(t.pos.to_vec2());
            let (inf, twelve) = (at(&texts.first().unwrap()), at(&texts.last().unwrap()));
            assert!((inf.center().x - 0.5 * U).abs() < 0.5, "ppp {ppp}: {inf:?}");
            assert!(
                (twelve.center().x - 10.7 * U).abs() < 0.5 && twelve.right() > 10.7 * U,
                "ppp {ppp}: {twelve:?}"
            );
            assert!(
                (twelve.center().y - (0.15 + 0.42) * U).abs() < 1.0,
                "ppp {ppp}: {twelve:?}"
            );
        }
    }

    #[test]
    fn at_rest_each_meter_is_its_trough_alone() {
        let control = theme_by_id(0).colors.control_fill.to_color32();
        for ppp in [1.0, 2.0] {
            for (n, y) in [(1, 0.0), (3, 0.99)] {
                let shapes = display_shapes(20, n, &[], &PanelState::default(), content(), ppp);
                let [Shape::Rect(r)] = &shapes[..] else {
                    panic!("ppp {ppp} #{n}: {shapes:?}")
                };
                let want = Rect::from_min_size(pos2(0.5 * U, y * U), vec2(10.2 * U, 0.15 * U));
                assert_eq!(r.fill, control);
                assert!(
                    r.rect.min.distance(want.min) < 1.0 && r.rect.max.distance(want.max) < 1.0,
                    "{r:?}"
                );
            }
        }
    }

    #[test]
    fn levels_map_minus_60_to_plus_12_db_across_the_meter() {
        // Silence and anything ≤ 0.001 is −60 dB, at the left; 0 dB at 5/6; +12 dB and over at the right.
        assert_eq!((fraction(0.0), fraction(0.001)), (0.0, 0.0));
        assert!(close(fraction(1.0), 5.0 / 6.0));
        assert!(close(fraction(10f32.powf(0.6)), 1.0) && fraction(100.0) == 1.0);
        // −20 dB: (−20 + 60)/72.
        assert!(close(fraction(0.1), 40.0 / 72.0));
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn the_peak_decays_by_0_95_per_buffer_and_rises_at_once() {
        assert!(near(decay(1.0, BUFFER, BUFFER, 0.0), 0.95));
        assert!(near(
            decay(1.0, 10.0 * BUFFER, BUFFER, 0.0),
            0.95f32.powi(10)
        ));
        // Stepped by time, not by calls: two half buffers are one buffer; no time changes nothing.
        assert!(near(
            decay(
                decay(0.5, BUFFER / 2.0, BUFFER, 0.0),
                BUFFER / 2.0,
                BUFFER,
                0.0
            ),
            0.5 * 0.95
        ));
        assert_eq!(decay(0.5, 0.0, BUFFER, 0.0), 0.5);
        // A louder block takes over at once; a quieter one leaves the decay alone.
        assert_eq!(decay(0.2, BUFFER, BUFFER, 0.5), 0.5);
        assert!(near(decay(0.5, BUFFER, BUFFER, 0.2), 0.475));
        // At or under −60 dB a decayed peak is silence; a block that quiet still shows.
        assert_eq!(decay(0.00105, BUFFER, BUFFER, 0.0), 0.0);
        assert_eq!(decay(0.00105, BUFFER, BUFFER, 0.0005), 0.0005);
        // About 3 s from full scale to rest.
        assert_eq!(decay(1.0, 3.2, BUFFER, 0.0), 0.0);
    }

    #[test]
    fn the_buffer_is_1024_frames_of_the_lent_sample_rate_and_of_44_1_khz_without_one() {
        let ctx = eframe::egui::Context::default();
        for (rate, want) in [
            (0.0, BUFFER),
            (-48_000.0, BUFFER),
            (f32::NAN, BUFFER),
            (f32::INFINITY, BUFFER),
            (44_100.0, BUFFER),
            (48_000.0, 1024.0 / 48_000.0),
        ] {
            let mut got = 0.0;
            pass(&ctx, 0.0, rate, None, |ui| got = buffer(ui));
            assert_eq!(got, want, "rate {rate}");
        }
        // The hold falls 0.005 per buffer of whatever length.
        let b = 1024.0 / 96_000.0;
        let held = SILENCE.step(0.0, b, 0.8).step(1.0, b, 0.0);
        assert_eq!(held.hold, 0.8);
        assert!(near(held.step(b, b, 0.0).hold, 0.795));
        assert!(near(held.step(3.0 * b, b, 0.0).hold, 0.785));
    }

    #[test]
    fn the_hold_stays_1_s_then_falls_0_005_per_buffer() {
        let hit = SILENCE.step(0.0, BUFFER, 0.8);
        assert_eq!(
            hit,
            Level {
                peak: 0.8,
                hold: 0.8,
                held: 0.0
            }
        );
        // Put for 1 s while the peak decays under it.
        let second = hit.step(0.5, BUFFER, 0.0).step(0.5, BUFFER, 0.0);
        assert_eq!(second.hold, 0.8);
        assert!(near(second.peak, 0.8 * 0.95f32.powf(1.0 / BUFFER)));
        // Then 0.005 per buffer, also for the part of a step past the second.
        assert!(near(second.step(BUFFER, BUFFER, 0.0).hold, 0.795));
        let late = hit
            .step(0.9, BUFFER, 0.0)
            .step(0.1 + 2.0 * BUFFER, BUFFER, 0.0);
        assert!(near(late.hold, 0.79), "{late:?}");
        // Down to nothing, not below.
        assert_eq!(second.step(200.0 * BUFFER, BUFFER, 0.0).hold, 0.0);
        // A peak at the hold or over it raises it and starts the second again.
        let again = late.step(BUFFER, BUFFER, 0.9);
        assert_eq!(
            again,
            Level {
                peak: 0.9,
                hold: 0.9,
                held: 0.0
            }
        );
        // Silence stays at rest.
        assert_eq!(SILENCE.step(1.0, BUFFER, 0.0), SILENCE);
        assert_eq!(second.step(10.0, BUFFER, 0.0), SILENCE);
    }

    /// The left meter's peak bar after one pass at `t` s: the feed's peak `peak`, the rack lending `rate` Hz.
    fn bar_right(ctx: &eframe::egui::Context, t: f64, rate: f32, peak: f32) -> Option<f32> {
        let (theme, m) = (theme_by_id(0), metrics(1.0));
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let feed = MeterSnapshot {
            peak_left: peak,
            ..Default::default()
        };
        let shapes = pass(ctx, t, rate, Some(feed), |ui| {
            let module = ModuleInstance::new(20, 2);
            live(
                ui,
                &cx,
                Id::new("left"),
                METER,
                &module,
                Color32::RED,
                |m| m.peak_left,
            );
        });
        shapes.iter().find_map(|s| match s {
            Shape::Rect(r) if r.fill == Color32::RED => Some(r.rect.right()),
            _ => None,
        })
    }

    /// A 300 pt meter at x = 16.
    const METER: Rect = Rect {
        min: pos2(16.0, 4.0),
        max: pos2(316.0, 9.0),
    };

    #[test]
    fn the_peak_falls_per_buffer_of_the_real_sample_rate_and_of_44_1_khz_without_audio() {
        // A 0 dB peak, then half a second of silence: 0.95 per 1024 frames, however many such buffers that is.
        for (rate, buffers) in [
            (0.0, 0.5 * 44_100.0 / 1024.0),
            (44_100.0, 0.5 * 44_100.0 / 1024.0),
            (48_000.0, 0.5 * 48_000.0 / 1024.0),
            (96_000.0, 0.5 * 96_000.0 / 1024.0),
        ] {
            let ctx = eframe::egui::Context::default();
            let full = bar_right(&ctx, 0.0, rate, 1.0).unwrap();
            assert!(close(full, 16.0 + 250.0), "rate {rate}: {full}");
            let fell = bar_right(&ctx, 0.5, rate, 0.0).unwrap();
            let want = 16.0 + 300.0 * fraction(0.95f32.powf(buffers));
            assert!(
                (fell - want).abs() <= 1.0,
                "rate {rate}: {fell}, want {want}"
            );
        }
    }

    #[test]
    fn the_bar_and_hold_tick_sit_on_whole_pixels() {
        // A meter at x = 16.5 pt, 300 pt wide: at 0 dB (5/6) the bar ends at trunc(16.5 + 250) = 266 pt at ppp 1,
        // and on 266.5 pt at ppp 2. The hold tick runs from trunc(p) to trunc(p + 2), p = x + f·w − 1 px.
        let rect = Rect::from_min_size(pos2(16.5, 4.0), vec2(300.0, 5.0));
        let one = bar(rect, 5.0 / 6.0, &metrics(1.0));
        assert_eq!(
            (one.left(), one.right(), one.y_range()),
            (16.0, 266.0, rect.y_range())
        );
        assert_eq!(
            bar(rect, 5.0 / 6.0, &metrics(2.0)).x_range(),
            Rangef::new(16.5, 266.5)
        );
        assert_eq!(bar(rect, 0.0, &metrics(1.0)).width(), 0.0);
        let tick = hold_tick(rect, 1.0, &metrics(1.0)).unwrap();
        assert_eq!(
            (tick.left(), tick.right(), tick.y_range()),
            (265.0, 267.0, rect.y_range())
        );
        assert_eq!(
            hold_tick(rect, 1.0, &metrics(2.0)).unwrap().x_range(),
            Rangef::new(266.0, 267.0)
        );
        // Only strictly inside −60 … +12 dB.
        assert_eq!(hold_tick(rect, 0.001, &metrics(1.0)), None);
        assert_eq!(hold_tick(rect, 0.0, &metrics(1.0)), None);
        assert_eq!(hold_tick(rect, 10f32.powf(0.6), &metrics(1.0)), None);
        assert!(hold_tick(rect, 0.002, &metrics(1.0)).is_some());
    }
}

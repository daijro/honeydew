//! Multiband Compressor (ID 36), UIHeight 9 U. Pages `Master` (0), `Low` (1), `Mid` (2) and `High` (3); the band pages
//! differ only in their params (base 6, 13, 20).

use eframe::egui::{Color32, Id, Rect, Ui, pos2, vec2};

use honeydew_audio::analysis;

use super::leveller::{animate, bounded, buffer, decay, live_meter};
use super::live_analysis;
use crate::metrics::Metrics;
use crate::panels::build::*;
use crate::panels::cstm::MULTIBAND_SOLO_BAND;
use crate::panels::setting_item;
use crate::ui::{UiCtx, widgets};

/// Band `Solo`: off = `controlFill` disc with a `text` label, on = `text` disc with a `controlFill` label. The `text`
/// ring is a `text` halo t wide (the halo is a disc behind the button, so it shows as a ring round the off
/// disc and merges with the on one; t is the Pitcher `BYP` halo's width).
const SOLO_COLORS: ButtonColors = ButtonColors {
    fill_on: Ink::Text,
    fill_off: Ink::ControlFill,
    text_on: Ink::ControlFill,
    text_off: Some(Ink::Text),
};

/// The output VU's levels at rest, and without a meter feed (outside a rack): silence, as the master output's linear
/// peaks L and R.
const REST_PEAKS: [f32; 2] = [0.0; 2];

/// The VU scale's `%d` levels after `-inf.` (−60 dB, the left edge).
const VU_SCALE: [i32; 9] = [-50, -40, -30, -20, -10, -5, 0, 3, 6];

/// The output VU's lines for linear `peaks` L and R, as (track, fill): for each channel with a level > 0, the
/// full-width track and the fill from the left over f·w, pixel-snapped, f = min(20·log10(v)/66 + 60/66, 1). Each is t
/// tall; the first at y + 0.37 U, the next t + 1 px lower. The step down follows a drawn line, so a silent left channel
/// leaves its slot to the right one.
fn vu_lines(r: Rect, m: &Metrics, peaks: [f32; 2]) -> Vec<(Rect, Rect)> {
    let mut y = r.top() + m.u(0.37);
    let mut lines = Vec::new();
    for v in peaks.into_iter().filter(|&v| v > 0.0) {
        let f = (20.0 * v.log10() / 66.0 + 60.0 / 66.0).min(1.0);
        let track = Rect::from_x_y_ranges(r.x_range(), y..=y + m.t());
        lines.push((
            track,
            track.with_max_x(r.left() + m.round_px(f * r.width()).max(0.0)),
        ));
        y += m.t() + 1.0 / m.ppp();
    }
    lines
}

/// The VU scale's labels and their boxes: `-inf.`, then [`VU_SCALE`], each centred at x + w·(v + 60)/66 in a 1 U wide
/// box from y + 0.5 U, the rect's height tall (so below the rect).
fn vu_labels(r: Rect, m: &Metrics) -> Vec<(String, Rect)> {
    let label = |text: String, db: i32| {
        let x = r.left() + r.width() * (db + 60) as f32 / 66.0;
        let at = Rect::from_min_size(
            pos2(x - m.u(0.5), r.top() + m.u(0.5)),
            vec2(m.u(1.0), r.height()),
        );
        (text, at)
    };
    let scale = VU_SCALE.map(|v| label(v.to_string(), v));
    std::iter::once(label("-inf.".to_owned(), -60))
        .chain(scale)
        .collect()
}

/// The output VU for linear `peaks` L and R: its lines, a `controlFill` track under a `text` fill each, then the
/// labels in `text`, FontSize 1, centred.
fn vu(ui: &Ui, cx: &UiCtx, r: Rect, peaks: [f32; 2]) {
    let colors = &cx.theme.colors;
    for (track, fill) in vu_lines(r, cx.m, peaks) {
        ui.painter()
            .rect_filled(track, 0.0, colors.control_fill.to_color32());
        ui.painter()
            .rect_filled(fill, 0.0, colors.text.to_color32());
    }
    let text = colors.text.to_color32();
    for (label, at) in vu_labels(r, cx.m) {
        widgets::text_label(
            ui,
            cx,
            at,
            &label,
            FontSize::Pt8,
            Align2::CENTER_CENTER,
            text,
        );
    }
}

/// #39, the output VU, from the module's meter feed (its output peaks L and R); it repaints while a line moves.
fn output_vu(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    r: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    _: Color32,
) -> Vec<usize> {
    let target = live_meter(ui, module.instance_id)
        .map_or(REST_PEAKS, |m| [m.peak_left, m.peak_right].map(bounded));
    // The VU takes the Leveller's peak decay.
    let block = buffer(ui);
    let peaks = animate(ui, id, REST_PEAKS, |p, dt| {
        [0, 1].map(|i| decay(p[i], dt, block, target[i]))
    });
    vu(ui, cx, r, peaks);
    // A level at or under −60 dB draws its track with an empty fill and needs no frames: the engine's residue (its
    // anti-denormal 1e-30) shows so, still.
    if peaks.iter().any(|&p| p > 0.001) {
        ui.ctx().request_repaint();
    }
    Vec::new()
}

/// The band graphs (#15, #24, #33), each bound to its band's `Thres` p(`THRES`): p7 Low, p14 Mid, p21 High. They are
/// The Limiter's history in its band variant, with T = p·48 − 48 dB. Bound, not looked up by `state.tab`: in the pass
/// where the tab strip switches pages, the outgoing graph still draws once with the new tab.
fn band_graph<const THRES: usize>(
    ui: &mut Ui,
    cx: &UiCtx,
    _: Id,
    r: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let threshold = module.params.get(THRES).map(|v| v * 48.0 - 48.0);
    // The band's own compressor's history: p7 is band 0, p14 band 1, p21 band 2.
    let band = (THRES - 7) / 7;
    let series = live_analysis(ui, module.instance_id).and_then(|s| analysis::multiband(&s, band));
    super::limiter::history(
        ui,
        cx,
        r,
        true,
        threshold,
        series.as_ref().map(|s| (s, ink)),
    );
    Vec::new()
}

/// The band graphs of band pages 0 (Low), 1 (Mid) and 2 (High).
const BAND_GRAPHS: [Draw; 3] = [band_graph::<7>, band_graph::<14>, band_graph::<21>];

pub fn panel() -> Option<Panel> {
    // Band page k (0 Low, 1 Mid, 2 High) is tab k + 1, its first control #(11 + 9k) and its first param 6 + 7k.
    let band = |k: usize| {
        let (page, n, p) = (Show::Tab(k + 1), 11 + 9 * k as u32, 6 + 7 * k);
        hstack([
            vstack([knob(n, "Input Gain").p(p), knob(n + 1, "Thres").p(p + 1)]).show(page),
            v_slider(n + 2, "Attack").p(p + 3).value_text(),
            v_slider(n + 3, "Release").p(p + 4).value_text(),
            // The band graph.
            display(n + 4)
                .w(Size::Flex(3.0))
                .mt(0.5)
                .draw(BAND_GRAPHS[k]),
            vstack([knob(n + 5, "Ratio").p(p + 2), knob(n + 6, "Gain").p(p + 5)]).show(page),
            vstack([
                list(n + 7, "On · Bps · Mute", Bar::Left)
                    .p(p + 6)
                    .h(Size::Flex(1.5)),
                button(n + 8, "Solo", true)
                    .shape(0.7)
                    .colors(SOLO_COLORS)
                    .halo_t(1.0)
                    .halo_color(Ink::Text),
            ])
            .show(page),
        ])
        .show(page)
    };
    Some(
        Panel::new(vstack([
            tabs(1, "Master · Low · Mid · High").h(Size::U(1.25)),
            hstack([
                list(2, "All · Low · Mid · High", Bar::Right),
                v_slider(3, "Low Freq").p(2).value_text(),
                v_slider(4, "High Freq").p(3).value_text(),
                vstack([
                    hstack([
                        v_slider(5, "Low Width").p(27),
                        v_slider(6, "Mid Width").p(28),
                        v_slider(7, "High Width").p(29),
                    ])
                    .show(Show::Tab(0)),
                    list(8, "Full · Safe", Bar::Bottom).p(30).h(Size::U(1.0)),
                ])
                .w(Size::Flex(3.0))
                .show(Show::Tab(0)),
                v_slider(9, "Dry Mix").p(5).value_text(),
                v_slider(10, "Gain").p(1).value_text(),
            ])
            .show(Show::Tab(0)),
            band(0),
            band(1),
            band(2),
            // It holds the solo state and is never shown.
            button(38, "Solo", false).p(4).show(Show::Never),
            // The output VU.
            display(39)
                .h(Size::U(0.75))
                .margins(0.0, 1.0, 0.0, 1.0)
                .draw(output_vu),
        ]))
        .hook(solo),
    )
}

/// The Solo logic: the master list (#2), the band `Solo` buttons (#19, #28, #37) and the hidden solo state p4.
/// It also keeps the band that solos at [`MULTIBAND_SOLO_BAND`], which the rack sends to the engine as it changes.
fn solo(module: &mut ModuleInstance, state: &mut PanelState, edited: &[u32]) -> Vec<usize> {
    const BUTTONS: [u32; 3] = [19, 28, 37];
    let Some(&before) = module.params.get(4) else {
        return Vec::new();
    };
    let mut p4 = before;
    // Any band item of the master list solos, `All` unsolos.
    if edited.contains(&2) {
        p4 = f32::from(u8::from(state.settings.get(&2).is_some_and(|&v| v != 0.0)));
    }
    // A band button: off sets the list to `All`, on to its band; p4 follows it.
    for (k, n) in BUTTONS.into_iter().enumerate() {
        if edited.contains(&n) {
            let on = state.settings.get(&n).is_some_and(|&v| v >= 0.5);
            state
                .settings
                .insert(2, if on { (k + 1) as f32 } else { 0.0 });
            p4 = f32::from(u8::from(on));
        }
    }
    // The refresh: on a band page with p4 on the list shows that band, and every band button shows p4.
    if state.tab != 0 && p4 != 0.0 {
        state.settings.insert(2, state.tab as f32);
    }
    for n in BUTTONS {
        state.settings.insert(n, f32::from(u8::from(p4 != 0.0)));
    }
    // The band this solos, for the engine: on a band page with p4 on, that page's; on Master, the
    // list's; otherwise none.
    let band = match state.tab {
        0 => state
            .settings
            .get(&2)
            .map_or(0, |&v| setting_item(v).min(3)),
        page if p4 != 0.0 => page,
        _ => 0,
    };
    state.settings.insert(MULTIBAND_SOLO_BAND, band as f32);
    module.params[4] = p4;
    if p4 != before { vec![4] } else { Vec::new() }
}

#[cfg(test)]
const TABS: Row = row(
    1,
    Widget::Tabs,
    "Master · Low · Mid · High",
    [0.0, 0.0, 14.0, 1.25],
)
.h(Size::U(1.25));

#[cfg(test)]
const VU: Row = row(39, Widget::Display, "", [1.0, 6.24, 12.0, 0.75])
    .h(Size::U(0.75))
    .margins(0.0, 1.0, 0.0, 1.0);

/// Band page `k` (0 Low, 1 Mid, 2 High): its controls are #(11 + 9k)… and its params 6 + 7k….
#[cfg(test)]
const fn band(k: usize) -> [Row; 11] {
    let (n, p) = (11 + 9 * k as u32, 6 + 7 * k);
    [
        TABS,
        row(n, Widget::Knob, "Input Gain", [0.0, 1.25, 1.75, 2.49]).p(p),
        row(n + 1, Widget::Knob, "Thres", [0.0, 3.74, 1.75, 2.5]).p(p + 1),
        row(n + 2, Widget::VSlider, "Attack", [1.75, 1.25, 1.75, 4.99])
            .p(p + 3)
            .value_text(),
        row(n + 3, Widget::VSlider, "Release", [3.5, 1.25, 1.75, 4.99])
            .p(p + 4)
            .value_text(),
        row(n + 4, Widget::Display, "", [5.25, 1.75, 5.25, 4.49])
            .w(Size::Flex(3.0))
            .margins(0.5, 0.0, 0.0, 0.0),
        row(n + 5, Widget::Knob, "Ratio", [10.5, 1.25, 1.75, 2.49]).p(p + 2),
        row(n + 6, Widget::Knob, "Gain", [10.5, 3.74, 1.75, 2.5]).p(p + 5),
        row(
            n + 7,
            Widget::List(Bar::Left),
            "On · Bps · Mute",
            [12.25, 1.25, 1.75, 2.99],
        )
        .p(p + 6)
        .h(Size::Flex(1.5)),
        // Shape 0.7; off = `controlFill` disc with `text` label, on = `text` disc with `controlFill` label, ring
        // `text`: a `text` halo t wide.
        row(
            n + 8,
            Widget::Button { round: true },
            "Solo",
            [12.25, 4.24, 1.75, 2.0],
        )
        .shape(0.7)
        .halo_t(1.0)
        .halo_color(Ink::Text)
        .colors(ButtonColors {
            fill_on: Ink::Text,
            fill_off: Ink::ControlFill,
            text_on: Ink::ControlFill,
            text_off: Some(Ink::Text),
        }),
        VU,
    ]
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 6.99],
    settings: &[(2, 0.0), (19, 0.0), (28, 0.0), (37, 0.0)],
    cases: &[
        Case {
            name: "Master",
            rows: &[
                TABS,
                row(
                    2,
                    Widget::List(Bar::Right),
                    "All · Low · Mid · High",
                    [0.0, 1.25, 1.75, 4.99],
                ),
                row(3, Widget::VSlider, "Low Freq", [1.75, 1.25, 1.75, 4.99])
                    .p(2)
                    .value_text(),
                row(4, Widget::VSlider, "High Freq", [3.5, 1.25, 1.75, 4.99])
                    .p(3)
                    .value_text(),
                row(5, Widget::VSlider, "Low Width", [5.25, 1.25, 1.75, 3.99]).p(27),
                row(6, Widget::VSlider, "Mid Width", [7.0, 1.25, 1.75, 3.99]).p(28),
                row(7, Widget::VSlider, "High Width", [8.75, 1.25, 1.75, 3.99]).p(29),
                row(
                    8,
                    Widget::List(Bar::Bottom),
                    "Full · Safe",
                    [5.25, 5.24, 5.25, 1.0],
                )
                .p(30)
                .h(Size::U(1.0)),
                row(9, Widget::VSlider, "Dry Mix", [10.5, 1.25, 1.75, 4.99])
                    .p(5)
                    .value_text(),
                row(10, Widget::VSlider, "Gain", [12.25, 1.25, 1.75, 4.99])
                    .p(1)
                    .value_text(),
                VU,
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "Low",
            tab: 1,
            rows: &band(0),
            ..Case::DEFAULT
        },
        Case {
            name: "Mid",
            tab: 2,
            rows: &band(1),
            ..Case::DEFAULT
        },
        Case {
            name: "High",
            tab: 3,
            rows: &band(2),
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use eframe::egui::{Shape, pos2};
    use honeydew_audio::MeterSnapshot;

    use super::*;
    use crate::panels::fx::limiter::tests::{metrics, near, rects, row_at, u};
    use crate::panels::place;
    use crate::panels::tests::{U, display_shapes, display_shapes_in};
    use crate::theme::theme_by_id;

    /// The reference content rect: 14 × 6.99 U at the origin.
    const CONTENT: Rect = Rect {
        min: pos2(0.0, 0.0),
        max: pos2(14.0 * U, 6.99 * U),
    };

    #[test]
    fn each_band_graph_is_the_limiter_s_with_a_fill_three_rows_and_its_band_s_threshold() {
        let colors = theme_by_id(0).colors;
        let text = colors.text.to_color32();
        for ppp in [1.0, 2.0] {
            // Tab k shows band k's graph; its `Thres` p(7k) sets T = p·48 − 48 dB: 0.5 → −24, 0 → −48, 1 → 0 dB.
            for (tab, thres, t) in [(1, 0.5, -24.0), (2, 0.0, -48.0), (3, 1.0, 0.0)] {
                let mut params = ModuleInstance::new(36, 2).params;
                params[7 * tab] = thres;
                let state = PanelState {
                    tab,
                    ..Default::default()
                };
                let got = rects(display_shapes(
                    36,
                    6 + 9 * tab as u32,
                    &params,
                    &state,
                    CONTENT,
                    ppp,
                ));
                assert_eq!(got.len(), 5, "ppp {ppp}, tab {tab}: {got:?}");
                // The fill is the display rect: (5.25, 1.75, 5.25, 4.49) U within a pixel.
                let (r, fill) = got[0];
                assert_eq!(fill, colors.control_fill.to_color32());
                let want = u(5.25, 1.75, 5.25, 4.49);
                assert!(
                    (r.min - want.min).length() <= 1.5 && (r.max - want.max).length() <= 1.5,
                    "{r:?}"
                );
                for (i, k) in [0.0, 10.0 / 22.0, 20.0 / 22.0].into_iter().enumerate() {
                    assert_eq!(got[1 + i].1, text.gamma_multiply(0.1), "ppp {ppp}, row {i}");
                    assert!(near(got[1 + i].0, row_at(r, ppp, k)), "ppp {ppp}, row {i}");
                }
                assert_eq!(got[4].1, text);
                let k = (60.0 + t) / 66.0;
                assert!(
                    near(got[4].0, row_at(r, ppp, k)),
                    "ppp {ppp}, tab {tab}: {:?}",
                    got[4].0
                );
            }
        }
    }

    #[test]
    fn each_band_graph_draws_its_own_bands_history() {
        // A slab where only band k's newest entry has an input: tab k + 1's graph draws one input bar; the others none.
        use honeydew_audio::MeterSnapshot;
        use honeydew_audio::analysis::HISTORY;
        let len = 1 + 3 * HISTORY;
        for k in 0..3 {
            let mut slab = vec![0.0; 3 * len];
            slab[k * len + 1] = 0.5;
            slab[k * len] = 1.0;
            for tab in 1..=3 {
                let state = PanelState {
                    tab,
                    ..Default::default()
                };
                let shapes = crate::panels::tests::display_shapes_fed(
                    36,
                    6 + 9 * tab as u32,
                    &ModuleInstance::new(36, 2).params,
                    &state,
                    CONTENT,
                    2.0,
                    (MeterSnapshot::default(), &slab),
                );
                // Fill, 3 grid rows, the threshold; one input bar on its own band's page.
                let want = if tab == k + 1 { 6 } else { 5 };
                assert_eq!(shapes.len(), want, "band {k}, tab {tab}");
            }
        }
    }

    #[test]
    fn a_band_graph_reads_its_own_band_s_thres_whatever_the_tab() {
        // In the pass where the tab strip switches pages, the outgoing graph still draws once with the new tab: it
        // must read its own band's `Thres` then too, never p0 (Master) nor the incoming band's.
        let text = theme_by_id(0).colors.text.to_color32();
        let mut module = ModuleInstance::new(36, 2);
        for (p, v) in [(7, 0.5), (14, 0.0), (21, 1.0)] {
            module.params[p] = v;
        }
        let panel = panel().expect("a panel");
        for (k, (leaf, t)) in [(15, -24.0), (24, -48.0), (33, 0.0)]
            .into_iter()
            .enumerate()
        {
            let shown = PanelState {
                tab: k + 1,
                ..Default::default()
            };
            let placed = place(&panel, &module, &shown, CONTENT, U);
            let draw = (placed.iter().find(|(l, _)| l.id == leaf))
                .and_then(|(l, _)| l.draw)
                .expect("a draw");
            let alone = Panel::new(display(leaf).draw(draw));
            for tab in 0..4 {
                let state = PanelState {
                    tab,
                    ..Default::default()
                };
                let got = rects(display_shapes_in(
                    &alone,
                    36,
                    leaf,
                    &module.params,
                    &state,
                    CONTENT,
                    1.0,
                ));
                let (r, threshold) = (got[0].0, got[4]);
                assert_eq!(threshold.1, text);
                let want = row_at(r, 1.0, (60.0 + t) / 66.0);
                assert!(
                    near(threshold.0, want),
                    "#{leaf} on tab {tab}: {:?}",
                    threshold.0
                );
            }
        }
    }

    /// The VU's reference rect (1, 6.24, 12, 0.75) U.
    fn vu_rect() -> Rect {
        u(1.0, 6.24, 12.0, 0.75)
    }

    #[test]
    fn the_vu_labels_are_centred_on_their_levels_below_the_lines() {
        // `-inf.` on the left edge, then −50 … 6 at x + w·(v + 60)/66, in 1 U boxes from y + 0.5 U, rect.h tall.
        let want = [
            ("-inf.", -60.0),
            ("-50", -50.0),
            ("-40", -40.0),
            ("-30", -30.0),
            ("-20", -20.0),
            ("-10", -10.0),
            ("-5", -5.0),
            ("0", 0.0),
            ("3", 3.0),
            ("6", 6.0),
        ];
        for ppp in [1.0, 2.0] {
            let got = vu_labels(vu_rect(), &metrics(ppp));
            assert_eq!(got.len(), want.len(), "{got:?}");
            for ((text, at), (want_text, db)) in got.iter().zip(want) {
                assert_eq!(text, want_text);
                let x = 1.0 + 12.0 * (db + 60.0) / 66.0;
                assert!(
                    near(*at, u(x - 0.5, 6.74, 1.0, 0.75)),
                    "ppp {ppp}, {text}: {at:?}"
                );
            }
        }
    }

    #[test]
    fn the_vu_draws_a_line_per_channel_with_a_level_stepping_down_after_each() {
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            let (t, px) = (m.t(), 1.0 / ppp);
            // t = ceil(U px / 16) device pixels: 2 points at either density.
            assert_eq!(t, 2.0);
            let y0 = (6.24 + 0.37) * U;
            let line = |y: f32, f: f32| {
                let track = Rect::from_x_y_ranges(U..=13.0 * U, y..=y + t);
                (track, track.with_max_x(U + m.round_px(f * 12.0 * U)))
            };
            let lines = |peaks| vu_lines(vu_rect(), &m, peaks);
            // At rest, nothing.
            assert!(lines(REST_PEAKS).is_empty());
            // 0 dB fills 60/66 of the width, pixel-snapped; +6 dB and more the whole of it.
            let both = lines([1.0, 2.5]);
            assert_eq!(both.len(), 2);
            let (a, b) = (line(y0, 60.0 / 66.0), line(y0 + t + px, 1.0));
            assert!(
                near(both[0].0, a.0) && near(both[0].1, a.1),
                "ppp {ppp}: {both:?}"
            );
            assert!(
                near(both[1].0, b.0) && near(both[1].1, b.1),
                "ppp {ppp}: {both:?}"
            );
            // A silent left channel leaves its slot to the right one.
            let right = lines([0.0, 0.1]);
            let c = line(y0, 40.0 / 66.0);
            assert_eq!(right.len(), 1);
            assert!(
                near(right[0].0, c.0) && near(right[0].1, c.1),
                "ppp {ppp}: {right:?}"
            );
            // Below −60 dB the track shows with an empty fill.
            let low = lines([0.0005, 0.0]);
            assert_eq!(low.len(), 1);
            assert_eq!(low[0].1.width(), 0.0);
        }
    }

    #[test]
    fn the_vu_falls_per_buffer_of_the_real_sample_rate_and_of_44_1_khz_without_audio() {
        use crate::panels::fx::leveller::pass;
        // A 0 dB peak, then half a second of silence: 0.95 per 1024 frames, however many such buffers that is.
        for (rate, buffers) in [
            (0.0, 0.5 * 44_100.0 / 1024.0),
            (48_000.0, 0.5 * 48_000.0 / 1024.0),
        ] {
            let (theme, m) = (theme_by_id(0), metrics(1.0));
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            let ctx = eframe::egui::Context::default();
            let level = |t: f64, peak: f32| {
                let feed = MeterSnapshot {
                    peak_left: peak,
                    ..Default::default()
                };
                let shapes = pass(&ctx, t, rate, Some(feed), |ui| {
                    let mut module = ModuleInstance::new(36, 2);
                    let mut state = PanelState::default();
                    let r = vu_rect();
                    output_vu(
                        ui,
                        &cx,
                        Id::new("vu"),
                        r,
                        &mut module,
                        &mut state,
                        Color32::RED,
                    );
                });
                // The first line's fill: the second `text` rect, after its track.
                let text = theme.colors.text.to_color32();
                shapes.iter().find_map(|s| match s {
                    Shape::Rect(r) if r.fill == text => Some(r.rect.width()),
                    _ => None,
                })
            };
            let full = level(0.0, 1.0).unwrap();
            let fell = level(0.5, 0.0).unwrap();
            let want = vu_lines(vu_rect(), &m, [0.95f32.powf(buffers), 0.0])[0]
                .1
                .width();
            assert!(full > fell, "rate {rate}");
            assert!(
                (fell - want).abs() <= 1.0,
                "rate {rate}: {fell}, want {want}"
            );
        }
    }

    #[test]
    fn the_vu_at_rest_draws_only_its_labels_on_every_page() {
        let text = theme_by_id(0).colors.text.to_color32();
        for ppp in [1.0, 2.0] {
            for tab in 0..4 {
                let state = PanelState {
                    tab,
                    ..Default::default()
                };
                let got: Vec<(String, Color32)> = display_shapes(36, 39, &[], &state, CONTENT, ppp)
                    .into_iter()
                    .map(|s| match s {
                        Shape::Text(t) => (
                            t.galley.text().to_owned(),
                            t.galley.job.sections[0].format.color,
                        ),
                        other => panic!("{other:?}"),
                    })
                    .collect();
                let names: Vec<&str> = got.iter().map(|(s, _)| s.as_str()).collect();
                assert_eq!(
                    names,
                    [
                        "-inf.", "-50", "-40", "-30", "-20", "-10", "-5", "0", "3", "6"
                    ]
                );
                assert!(got.iter().all(|(_, c)| *c == text), "ppp {ppp}, tab {tab}");
            }
        }
    }

    /// The hook the panel runs, which the tests go through: a panel without it fails them.
    fn solo(m: &mut ModuleInstance, s: &mut PanelState, edited: &[u32]) -> Vec<usize> {
        (panel().unwrap().hook.expect("a hook"))(m, s, edited)
    }

    /// p4, the list, and the three band buttons.
    fn solo_state(module: &ModuleInstance, state: &PanelState) -> (f32, f32, [f32; 3]) {
        let s = |n: u32| state.settings[&n];
        (module.params[4], s(2), [s(19), s(28), s(37)])
    }

    #[test]
    fn the_list_and_the_band_buttons_drive_the_solo_state() {
        let mut module = ModuleInstance::new(36, 2);
        let panel = panel().expect("a panel");
        let mut state = PanelState::new(&panel);
        // Nothing edited, nothing soloed: nothing changes.
        assert!(solo(&mut module, &mut state, &[]).is_empty());
        assert_eq!(solo_state(&module, &state), (0.0, 0.0, [0.0; 3]));
        // Master list `Mid`: p4 on, and every band button shows it.
        state.settings.insert(2, 2.0);
        assert_eq!(solo(&mut module, &mut state, &[2]), [4]);
        assert_eq!(solo_state(&module, &state), (1.0, 2.0, [1.0; 3]));
        // On the High page with p4 on, the list follows the page.
        state.tab = 3;
        assert!(solo(&mut module, &mut state, &[]).is_empty());
        assert_eq!(solo_state(&module, &state), (1.0, 3.0, [1.0; 3]));
        // Its Solo button pressed off: the list goes to `All` and p4 follows.
        state.settings.insert(37, 0.0);
        assert_eq!(solo(&mut module, &mut state, &[37]), [4]);
        assert_eq!(solo_state(&module, &state), (0.0, 0.0, [0.0; 3]));
        // On again, on the Low page: the list goes to `Low`.
        state.tab = 1;
        state.settings.insert(19, 1.0);
        assert_eq!(solo(&mut module, &mut state, &[19]), [4]);
        assert_eq!(solo_state(&module, &state), (1.0, 1.0, [1.0; 3]));
        // Back on Master, `All` unsolos.
        state.tab = 0;
        state.settings.insert(2, 0.0);
        assert_eq!(solo(&mut module, &mut state, &[2]), [4]);
        assert_eq!(solo_state(&module, &state), (0.0, 0.0, [0.0; 3]));
        // Automation of p4 shows on the buttons.
        module.params[4] = 1.0;
        assert!(solo(&mut module, &mut state, &[]).is_empty());
        assert_eq!(solo_state(&module, &state).2, [1.0; 3]);
    }

    #[test]
    fn the_hook_keeps_the_band_the_solo_logic_solos_for_the_engine() {
        let mut module = ModuleInstance::new(36, 2);
        let panel = panel().expect("a panel");
        let mut state = PanelState::new(&panel);
        let soloed = |module: &mut ModuleInstance, state: &mut PanelState, edited: &[u32]| {
            solo(module, state, edited);
            state.settings[&MULTIBAND_SOLO_BAND]
        };
        // None at first; then the master list's `Mid`.
        assert_eq!(soloed(&mut module, &mut state, &[]), 0.0);
        state.settings.insert(2, 2.0);
        assert_eq!(soloed(&mut module, &mut state, &[2]), 2.0);
        // On a band page with p4 on, that page's band, and still once back on Master (the list followed it).
        state.tab = 3;
        assert_eq!(soloed(&mut module, &mut state, &[]), 3.0);
        state.tab = 0;
        assert_eq!(soloed(&mut module, &mut state, &[]), 3.0);
        // The Low page's Solo pressed off: none, on any page.
        state.tab = 1;
        state.settings.insert(19, 0.0);
        assert_eq!(soloed(&mut module, &mut state, &[19]), 0.0);
        state.tab = 0;
        assert_eq!(soloed(&mut module, &mut state, &[]), 0.0);
        // p4 on (automation, a preset) with the list on `All`: Master still solos nothing, a band page its band.
        module.params[4] = 1.0;
        assert_eq!(soloed(&mut module, &mut state, &[]), 0.0);
        state.tab = 2;
        assert_eq!(soloed(&mut module, &mut state, &[]), 2.0);
        // p4 off with the list left on `Mid`: a band page solos nothing, Master follows the list.
        module.params[4] = 0.0;
        assert_eq!(soloed(&mut module, &mut state, &[]), 0.0);
        state.tab = 0;
        assert_eq!(soloed(&mut module, &mut state, &[]), 2.0);
    }
}

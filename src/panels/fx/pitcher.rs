//! Pitcher (ID 25), UIHeight 8 U. Pages `Basic` (0) and `Advanced` (1).

use eframe::egui::{Color32, Id, Rect, Ui, pos2, vec2};
use honeydew_audio::analysis;

use super::leveller::animate;
use super::live_analysis;
use crate::metrics::Metrics;
use crate::panels::build::*;
use crate::panels::setting_item;
use crate::ui::UiCtx;

const KEYS: &[&str] = &[
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// The semitones above the root of each `Scale` item: Minor, Major, Pentatonic (four notes, as coded), Chromatic,
/// Single.
const SCALES: [&[usize]; 5] = [
    &[0, 2, 3, 5, 7, 8, 10],
    &[0, 2, 4, 5, 7, 9, 11],
    &[0, 3, 5, 7],
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
    &[0],
];

/// Note `note` (0 = C) is in scale item `scale` on key `key`.
fn in_scale(scale: usize, key: usize, note: usize) -> bool {
    SCALES[scale].contains(&((note + 12 - key) % 12))
}

/// `Bypass snapping`: on = `moduleFill` fill with a `text` label, off the reverse.
const BYPASS_COLORS: ButtonColors = ButtonColors {
    fill_on: Ink::ModuleFill,
    fill_off: Ink::Text,
    text_on: Ink::Text,
    text_off: Some(Ink::ModuleFill),
};

pub fn panel() -> Option<Panel> {
    // Not params: they start on the Minor scale on C. Naturals have a 1 px halo, sharps none. Both rows keep the
    // button's own colours: on = `text` disc with a `menuFill` label, off the reverse.
    let notes = KEYS.iter().zip(0..).map(|(&key, i)| {
        let note = button(5 + i as u32, key, true)
            .shape(0.8)
            .default(f32::from(u8::from(in_scale(0, 0, i))));
        if key.len() == 1 { note.halo(1.0) } else { note }
    });
    let bypass = (0..12).map(|k| {
        button(24 + k as u32, "BYP", true)
            .p(10 + k)
            .font(FontSize::Pt6)
            .shape(0.7)
            .halo_t(1.0)
    });
    Some(
        Panel::new(vstack([
            hstack([
                spacer(1),
                tabs(2, "Basic · Advanced"),
                hstack([button(3, "Bypass snapping", false)
                    .p(22)
                    .margins(0.2, 1.0, 0.2, 1.0)
                    .colors(BYPASS_COLORS)]),
            ])
            .h(Size::U(1.25)),
            display(4).h(Size::U(0.5)).draw(strip),
            vstack([
                hstack(notes).h(Size::U(2.0)).show(Show::Tab(0)),
                hstack([
                    knob(17, "Speed").p(1),
                    knob(18, "Pitch").p(2),
                    knob(19, "Fine Tune").p(3),
                    spinner(20, "Scale").items(&[
                        "Minor",
                        "Major",
                        "Pentatonic",
                        "Chromatic",
                        "Single",
                    ]),
                    spinner(21, "Key").items(KEYS),
                    toggle_right(22, "F-Mode").p(4).w(Size::Flex(1.5)),
                    knob(23, "F-Shift").p(5),
                ])
                .show(Show::Tab(0)),
            ])
            .show(Show::Tab(0)),
            vstack([
                hstack(bypass).h(Size::U(2.0)).show(Show::Tab(1)),
                hstack([
                    spinner(36, "Pitch reference").p(6),
                    knob(37, "Base Freq").p(7),
                    knob(38, "Stability").p(8),
                    knob(39, "Mix").p(9),
                ])
                .show(Show::Tab(1)),
            ])
            .show(Show::Tab(1)),
        ]))
        .underlay(target)
        .hook(scale),
    )
}

/// Pitch strip (#4) step 2: the 12 dots, 2 × 2 px at the strip's vertical centre, x = strip.x + int(W/24 + i·W/12)
/// device px, the centre of each note column. Computed as int(W·(2i + 1)/24) so the exact centres stay whole in f32.
fn dots(strip: Rect, m: &Metrics) -> [Rect; 12] {
    let (w, px) = (strip.width() * m.ppp(), 1.0 / m.ppp());
    let top = m.floor_px(strip.center().y) - px;
    std::array::from_fn(|i| {
        let x = strip.left() + (w * (2 * i + 1) as f32 / 24.0).trunc() * px;
        Rect::from_min_size(pos2(x, top), vec2(2.0 * px, 2.0 * px))
    })
}

/// A detected frequency's pitch class: pc = (12·log2(f/base) + 9) mod 12, then pc ≥ 11.5 wrapped to pc − 12, so B's
/// upper half sits left of C. `None` without a pitch.
pub(super) fn pitch_class(hz: f32, base: f32) -> Option<f32> {
    let pc = (12.0 * (hz / base).log2() + 9.0).rem_euclid(12.0);
    (hz > 0.0 && pc.is_finite()).then_some(if pc >= 11.5 { pc - 12.0 } else { pc })
}

/// `Base Freq` (p7): A4 = 440·2^((2v − 1)/12) Hz.
fn base_freq(module: &ModuleInstance) -> f32 {
    let v = module.params.get(7).copied().unwrap_or(0.5);
    440.0 * 2f32.powf((2.0 * v - 1.0) / 12.0)
}

/// The Pitcher's feed: the detected pitch class on `module`'s `Base Freq`, and the target note's class, if any.
fn feed(ui: &Ui, module: &ModuleInstance) -> (Option<f32>, Option<usize>) {
    let feed = live_analysis(ui, module.instance_id).and_then(|s| analysis::pitcher(&s));
    let (hz, target) = feed.unwrap_or((0.0, None));
    (
        pitch_class(hz, base_freq(module)),
        target.filter(|&c| c < 12),
    )
}

/// A motion trail entry (pitch strip step 4): a marker position x (points), its step dx from the previous one (points),
/// and its amplitude a.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Entry {
    pub x: f32,
    pub dx: f32,
    pub a: f32,
}

/// Pitch strip step 4, shared with Pitcher 2 (its display step 5): up to 64 entries, one for each new marker position.
/// A starts at 1 and decays ×0.85 per frame (1/60 s, as the Compressor's), and an entry under 0.1 goes.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Trail {
    last: Option<f32>,
    pub entries: Vec<Entry>,
}

impl Trail {
    /// After `dt` s, with the marker at `x` (points; `None` without a pitch, which also forgets where it was).
    pub(super) fn step(mut self, dt: f32, x: Option<f32>) -> Trail {
        let k = 0.85f32.powf(dt * 60.0);
        self.entries.iter_mut().for_each(|e| e.a *= k);
        self.entries.retain(|e| e.a >= 0.1);
        if let Some(x) = x
            && self.last != Some(x)
        {
            let dx = self.last.map_or(0.0, |last| x - last);
            self.entries.push(Entry { x, dx, a: 1.0 });
            if self.entries.len() > 64 {
                self.entries.remove(0);
            }
        }
        self.last = x;
        self
    }

    /// Entry `e`'s quad's x range: from x − max(dx, 1 px) to x + max(−dx, 1 px), covering its step.
    pub(super) fn span(e: &Entry, px: f32) -> std::ops::RangeInclusive<f32> {
        e.x - e.dx.max(px)..=e.x + (-e.dx).max(px)
    }
}

/// The marker's left edge for pitch class `pc` (pitch strip step 3): strip.x + int(W/24 + pc·W/12) device px.
fn marker_x(strip: Rect, pc: f32, m: &Metrics) -> f32 {
    let w = strip.width() * m.ppp();
    strip.left() + (w / 24.0 + pc * w / 12.0).trunc() / m.ppp()
}

/// The pitch strip (#4), steps 1–4: a `controlFill` fill and the channel dots; with a detected pitch the
/// channel marker, 2t wide and full height; and the motion trail of its positions, each entry's quad a·h tall about
/// the centre line.
fn strip(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let (pc, _) = feed(ui, module);
    let x = pc.map(|pc| marker_x(rect, pc, cx.m));
    let trail = animate(ui, id, Trail::default(), |t, dt| t.step(dt, x));
    if !trail.entries.is_empty() {
        ui.ctx().request_repaint();
    }
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, cx.theme.colors.control_fill.to_color32());
    for dot in dots(rect, cx.m) {
        painter.rect_filled(dot, 0.0, ink);
    }
    if let Some(x) = x {
        let marker = Rect::from_x_y_ranges(x..=x + 2.0 * cx.m.t(), rect.y_range());
        painter.rect_filled(marker, 0.0, ink);
    }
    let (px, centre) = (1.0 / cx.m.ppp(), rect.center().y);
    for e in &trail.entries {
        let half = e.a * rect.height() / 2.0;
        let quad = Rect::from_x_y_ranges(Trail::span(e, px), centre - half..=centre + half);
        painter.rect_filled(quad, 0.0, ink);
    }
    Vec::new()
}

/// Pitch strip step 5, under the controls so the note buttons draw on top: on the Basic page, with a target note, a
/// filled channel disc on its note button (#5 + class), radius half the button's width.
fn target(
    ui: &Ui,
    _: &UiCtx,
    module: &ModuleInstance,
    state: &PanelState,
    rects: &[(u32, Rect)],
    _: Rect,
    ink: Color32,
) {
    if state.tab != 0 {
        return;
    }
    let (_, target) = feed(ui, module);
    let button = target.and_then(|class| rects.iter().find(|(n, _)| *n == 5 + class as u32));
    if let Some(&(_, r)) = button {
        ui.painter().circle_filled(r.center(), r.width() / 2.0, ink);
    }
}

/// `Scale` (#20) or `Key` (#21) change sets the note buttons (#5–#16) to the scale's notes rotated up by the key.
fn scale(_: &mut ModuleInstance, state: &mut PanelState, edited: &[u32]) -> Vec<usize> {
    if edited.contains(&20) || edited.contains(&21) {
        let item = |n: u32| state.settings.get(&n).map_or(0, |&v| setting_item(v));
        let (scale, key) = (item(20).min(SCALES.len() - 1), item(21) % 12);
        for note in 0..12 {
            let on = in_scale(scale, key, note);
            state
                .settings
                .insert(5 + note as u32, f32::from(u8::from(on)));
        }
    }
    Vec::new()
}

#[cfg(test)]
const HEADER: [Row; 4] = [
    row(1, Widget::Spacer, "", [0.0, 0.0, 4.66, 1.25]),
    row(2, Widget::Tabs, "Basic · Advanced", [4.66, 0.0, 4.67, 1.25]),
    row(
        3,
        Widget::Button { round: false },
        "Bypass snapping",
        [10.33, 0.2, 2.66, 0.85],
    )
    .p(22)
    .margins(0.2, 1.0, 0.2, 1.0)
    .colors(ButtonColors {
        fill_on: Ink::ModuleFill,
        fill_off: Ink::Text,
        text_on: Ink::Text,
        text_off: Some(Ink::ModuleFill),
    }),
    row(4, Widget::Display, "", [0.0, 1.25, 14.0, 0.5]).h(Size::U(0.5)),
];

/// A Basic note button: round, 2 U high, shape 0.8.
#[cfg(test)]
const fn note(id: u32, key: &'static str, x: f32, w: f32) -> Row {
    row(id, Widget::Button { round: true }, key, [x, 1.75, w, 2.0]).shape(0.8)
}

/// `BYP` button of the Advanced page: FontSize 0, shape 0.7, halo pad t.
#[cfg(test)]
const fn byp(id: u32, p: usize, x: f32, w: f32) -> Row {
    row(id, Widget::Button { round: true }, "BYP", [x, 1.75, w, 2.0])
        .p(p)
        .font(FontSize::Pt6)
        .shape(0.7)
        .halo_t(1.0)
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 5.99],
    // The note buttons show the default scale, Minor on C: 0 2 3 5 7 8 10.
    settings: &[
        (5, 1.0),
        (6, 0.0),
        (7, 1.0),
        (8, 1.0),
        (9, 0.0),
        (10, 1.0),
        (11, 0.0),
        (12, 1.0),
        (13, 1.0),
        (14, 0.0),
        (15, 1.0),
        (16, 0.0),
        (20, 0.0),
        (21, 0.0),
    ],
    cases: &[
        Case {
            name: "Basic",
            // Naturals have a 1 px halo pad, sharps none.
            rows: &[
                HEADER[0],
                HEADER[1],
                HEADER[2],
                HEADER[3],
                note(5, "C", 0.0, 1.16).halo(1.0),
                note(6, "C#", 1.16, 1.17),
                note(7, "D", 2.33, 1.17).halo(1.0),
                note(8, "D#", 3.5, 1.16),
                note(9, "E", 4.66, 1.17).halo(1.0),
                note(10, "F", 5.83, 1.17).halo(1.0),
                note(11, "F#", 7.0, 1.16),
                note(12, "G", 8.16, 1.17).halo(1.0),
                note(13, "G#", 9.33, 1.17),
                note(14, "A", 10.5, 1.16).halo(1.0),
                note(15, "A#", 11.66, 1.17),
                note(16, "B", 12.83, 1.16).halo(1.0),
                row(17, Widget::Knob, "Speed", [0.0, 3.75, 1.86, 2.24]).p(1),
                row(18, Widget::Knob, "Pitch", [1.86, 3.75, 1.87, 2.24]).p(2),
                row(19, Widget::Knob, "Fine Tune", [3.73, 3.75, 1.87, 2.24]).p(3),
                row(20, Widget::Spinner, "Scale", [5.6, 3.75, 1.86, 2.24])
                    .items("Minor · Major · Pentatonic · Chromatic · Single"),
                row(21, Widget::Spinner, "Key", [7.46, 3.75, 1.87, 2.24])
                    .items("C · C# · D · D# · E · F · F# · G · G# · A · A# · B"),
                row(22, Widget::ToggleRight, "F-Mode", [9.33, 3.75, 2.8, 2.24])
                    .p(4)
                    .w(Size::Flex(1.5)),
                row(23, Widget::Knob, "F-Shift", [12.13, 3.75, 1.87, 2.24]).p(5),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "Advanced",
            tab: 1,
            rows: &[
                HEADER[0],
                HEADER[1],
                HEADER[2],
                HEADER[3],
                byp(24, 10, 0.0, 1.16),
                byp(25, 11, 1.16, 1.17),
                byp(26, 12, 2.33, 1.17),
                byp(27, 13, 3.5, 1.16),
                byp(28, 14, 4.66, 1.17),
                byp(29, 15, 5.83, 1.17),
                byp(30, 16, 7.0, 1.16),
                byp(31, 17, 8.16, 1.17),
                byp(32, 18, 9.33, 1.17),
                byp(33, 19, 10.5, 1.16),
                byp(34, 20, 11.66, 1.17),
                byp(35, 21, 12.83, 1.16),
                row(
                    36,
                    Widget::Spinner,
                    "Pitch reference",
                    [0.0, 3.75, 3.5, 2.24],
                )
                .p(6),
                row(37, Widget::Knob, "Base Freq", [3.5, 3.75, 3.5, 2.24]).p(7),
                row(38, Widget::Knob, "Stability", [7.0, 3.75, 3.5, 2.24]).p(8),
                row(39, Widget::Knob, "Mix", [10.5, 3.75, 3.5, 2.24]).p(9),
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use eframe::egui::{Shape, pos2, vec2};

    use super::*;
    use crate::panels::fx::limiter::tests::metrics;
    use crate::panels::tests::{U, display_shapes};
    use crate::theme::theme_by_id;

    /// The reference strip #4, (0, 1.25, 14, 0.5) U.
    const STRIP: Rect = Rect {
        min: pos2(0.0, 1.25 * U),
        max: pos2(14.0 * U, 1.75 * U),
    };

    #[test]
    fn the_strip_dots_sit_on_the_note_column_centres() {
        // Step 2: x = strip.x + int(W/24 + i·W/12) device px, 2 × 2 px, at the strip's vertical centre. W is
        // 448 px at ppp 1 and 896 at ppp 2; the exact centres (i = 1, 4, 7, 10) stay whole.
        for (ppp, xs) in [
            (
                1.0,
                [18, 56, 93, 130, 168, 205, 242, 280, 317, 354, 392, 429],
            ),
            (
                2.0,
                [37, 112, 186, 261, 336, 410, 485, 560, 634, 709, 784, 858],
            ),
        ] {
            let dots = dots(STRIP, &metrics(ppp));
            assert_eq!(
                dots.map(|d| d.left() * ppp),
                xs.map(|x| x as f32),
                "ppp {ppp}"
            );
            for d in dots {
                assert_eq!(d.size() * ppp, vec2(2.0, 2.0), "ppp {ppp}");
                assert_eq!(d.center().y, STRIP.center().y, "ppp {ppp}");
            }
        }
    }

    #[test]
    fn a_frequency_becomes_its_pitch_class_on_the_base_freq() {
        let close = |a: Option<f32>, b: f32| a.is_some_and(|a| (a - b).abs() < 1e-3);
        assert!(close(pitch_class(220.0, 440.0), 9.0));
        assert!(close(pitch_class(261.6256, 440.0), 0.0));
        // A quarter tone above B wraps below C.
        assert!(close(
            pitch_class(493.8833 * 2f32.powf(0.6 / 12.0), 440.0),
            -0.4
        ));
        // On a 466.2 Hz A4 (Base Freq 1), 440 Hz is a semitone flat of A.
        assert!(close(pitch_class(440.0, 466.1638), 8.0));
        assert_eq!(pitch_class(0.0, 440.0), None);
    }

    #[test]
    fn the_trail_records_each_new_position_and_fades_by_0_85_a_frame() {
        let t = Trail::default().step(0.0, Some(100.0));
        assert_eq!(
            t.entries,
            [Entry {
                x: 100.0,
                dx: 0.0,
                a: 1.0
            }]
        );
        // The same position again adds nothing; a move adds its step.
        let t = t.step(0.0, Some(100.0)).step(1.0 / 60.0, Some(90.0));
        assert_eq!(t.entries.len(), 2);
        assert!((t.entries[0].a - 0.85).abs() < 1e-6 && t.entries[1].dx == -10.0);
        // The quad covers the step: from x − 1 px to x + 10 (moving left), and a still entry 1 px either side.
        assert_eq!(Trail::span(&t.entries[1], 1.0), 89.0..=100.0);
        assert_eq!(Trail::span(&t.entries[0], 0.5), 99.5..=100.5);
        // No pitch: nothing new, and the next marker starts afresh (dx 0). Under 0.1 an entry goes (0.85^15 < 0.1).
        let t = t.step(14.0 / 60.0, None).step(0.0, Some(50.0));
        assert_eq!(t.entries.last().map(|e| e.dx), Some(0.0));
        assert_eq!(t.entries.len(), 2, "{t:?}");
        let t = t.step(1.0 / 60.0, Some(50.0));
        assert_eq!(t.entries.len(), 1, "{t:?}");
        // At most 64.
        let t = (0..100).fold(Trail::default(), |t, i| t.step(0.0, Some(i as f32)));
        assert_eq!(t.entries.len(), 64);
        assert_eq!(t.entries[0].x, 36.0);
    }

    #[test]
    fn a_detected_pitch_draws_the_marker_and_its_trail_and_the_target_disc() {
        // 220 Hz is A (class 9): the marker at strip.x + int(W/24 + 9·W/12), 2t wide, full height; then its first trail
        // entry, a full-height quad 1 px either side of it. The target G# (8) gets the disc under its note button.
        use honeydew_audio::MeterSnapshot;
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 5.99 * U));
        let feed = [220.0, 8.0];
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            let shapes = crate::panels::tests::display_shapes_fed(
                25,
                4,
                &[],
                &PanelState::default(),
                content,
                ppp,
                (MeterSnapshot::default(), &feed),
            );
            assert_eq!(shapes.len(), 15, "ppp {ppp}");
            let Shape::Rect(fill) = &shapes[0] else {
                panic!()
            };
            let strip = fill.rect;
            let Shape::Rect(marker) = &shapes[13] else {
                panic!()
            };
            let x = marker_x(strip, 9.0, &m);
            assert_eq!(
                marker.rect,
                Rect::from_x_y_ranges(x..=x + 2.0 * m.t(), strip.y_range())
            );
            assert_eq!(marker.fill, Color32::RED);
            let Shape::Rect(quad) = &shapes[14] else {
                panic!()
            };
            assert_eq!(
                quad.rect,
                Rect::from_x_y_ranges(x - 1.0 / ppp..=x + 1.0 / ppp, strip.y_range())
            );
            // The disc, under the controls: on #13 (G#), radius half its width.
            let button = Rect::from_min_size(pos2(9.33 * U, 1.75 * U), vec2(1.17 * U, 2.0 * U));
            let (theme, cx_m) = (theme_by_id(0), metrics(ppp));
            let cx = UiCtx {
                theme: &theme,
                m: &cx_m,
            };
            let out = eframe::egui::Context::default().run(Default::default(), |ctx| {
                let ui = Ui::new(ctx.clone(), Id::new("panel"), Default::default());
                crate::panels::post_feed(&ui, 2, Some((MeterSnapshot::default(), &feed)));
                let module = ModuleInstance::new(25, 2);
                for tab in [0, 1] {
                    let state = PanelState {
                        tab,
                        ..Default::default()
                    };
                    target(
                        &ui,
                        &cx,
                        &module,
                        &state,
                        &[(13, button)],
                        content,
                        Color32::RED,
                    );
                }
            });
            let discs: Vec<Shape> = out.shapes.into_iter().map(|c| c.shape).collect();
            // On the Basic page only.
            let [Shape::Circle(disc)] = &discs[..] else {
                panic!("{discs:?}")
            };
            assert_eq!(
                (disc.center, disc.radius),
                (button.center(), button.width() / 2.0)
            );
        }
    }

    #[test]
    fn the_strip_at_rest_is_its_fill_and_the_dots_on_both_pages() {
        // Steps 1–2; no pitch is detected, so no marker, trail or target disc (steps 3–5).
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 5.99 * U));
        let fill = theme_by_id(0).colors.control_fill.to_color32();
        for (ppp, tab) in [(1.0, 0), (2.0, 0), (1.0, 1)] {
            let state = PanelState {
                tab,
                ..PanelState::default()
            };
            let shapes = display_shapes(25, 4, &[], &state, content, ppp);
            let rects: Vec<_> = shapes
                .iter()
                .map(|s| match s {
                    Shape::Rect(r) => (r.rect, r.fill),
                    _ => panic!("{s:?}"),
                })
                .collect();
            assert_eq!(rects[0], (STRIP, fill), "ppp {ppp}");
            let dots: Vec<_> = dots(STRIP, &metrics(ppp)).map(|d| (d, Color32::RED)).into();
            assert_eq!(rects[1..], dots[..], "ppp {ppp}, tab {tab}");
        }
    }

    /// The hook the panel runs, which the tests go through: a panel without it fails them.
    fn scale(m: &mut ModuleInstance, s: &mut PanelState, edited: &[u32]) -> Vec<usize> {
        (panel().unwrap().hook.expect("a hook"))(m, s, edited)
    }

    /// The note buttons that are on, as note names.
    fn on(state: &PanelState) -> Vec<&'static str> {
        let names = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        (0..12)
            .filter(|&i| state.settings[&(5 + i as u32)] == 1.0)
            .map(|i| names[i])
            .collect()
    }

    #[test]
    fn scale_and_key_set_the_note_buttons() {
        let mut module = ModuleInstance::new(25, 2);
        let panel = panel().expect("a panel");
        let mut state = PanelState::new(&panel);
        assert_eq!(on(&state), ["C", "D", "D#", "F", "G", "G#", "A#"]);
        // A pressed note stays pressed while Scale and Key are left alone.
        state.settings.insert(6, 1.0);
        assert!(scale(&mut module, &mut state, &[6]).is_empty());
        assert_eq!(on(&state)[..2], ["C", "C#"]);
        // Major on A; then Pentatonic, Chromatic and Single on D.
        state.settings.insert(20, 1.0);
        state.settings.insert(21, 9.0);
        scale(&mut module, &mut state, &[21]);
        assert_eq!(on(&state), ["C#", "D", "E", "F#", "G#", "A", "B"]);
        state.settings.insert(21, 2.0);
        for (item, notes) in [
            (2.0, vec!["D", "F", "G", "A"]),
            (
                3.0,
                vec![
                    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
                ],
            ),
            (4.0, vec!["D"]),
        ] {
            state.settings.insert(20, item);
            scale(&mut module, &mut state, &[20]);
            assert_eq!(on(&state), notes, "scale {item}");
        }
        assert_eq!(module.params, ModuleInstance::new(25, 2).params);
    }
}

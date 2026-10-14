//! A message box: header lines, then `OK`, or two choices, or three, in `text` on a full-width `background` box with
//! `menuFill` 2 px lines 1–3 px inside its top and bottom edges. The lines word-wrap to the screen less its margins
//! ([`super::wrap`]), so the box is as tall as the wrapped lines and choices, up to the screen height less the margins,
//! past which the choices scroll. It opens vertically from its centre, h = (1 − a)·H with a ← 0.4·a a drawn frame from
//! 1 at each new message, as the screen dims to α 0.6·(1 − a); the text and choices show once a = 0 and take input from
//! the start.

use eframe::egui::{self, Color32, Id, LayerId, Order, Rect, pos2, vec2};

use super::wrap::wrap;
use super::{Answer, AppCtx, Nav};
use crate::metrics::FontSize;
use crate::ui::anim;
use crate::ui::popup::{self, PAD_X, PopupItem};

/// The `menuFill` lines: from 1 to 3 device px inside the box's top and bottom edges.
const LINE_PX: [f32; 2] = [1.0, 3.0];
/// The gap in U between the text column and the screen edges. Text wraps to what is left.
const MARGIN: f32 = 1.0;

/// Shows `lines` centred on the screen, then `OK`, or the `extra` choice (if any) and the `(yes, no)` choices of
/// `confirm`, one per row. `OK` returns `Nav::Close`; a choice returns `Nav::Answer(tag, ..)` or `Nav::Extra(tag)`,
/// and Escape or a click outside answers no.
pub fn show(
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    tag: &'static str,
    lines: &[String],
    confirm: Option<&(String, String)>,
    extra: Option<&str>,
) -> Option<Nav> {
    debug_assert!(
        confirm.is_some() || extra.is_none(),
        "{tag}: an extra choice needs the yes and no ones"
    );
    let (ctx, id, m, screen) = (ui.ctx(), Id::new("message"), app.cx.m, app.regions.screen);
    let bounds = screen.shrink(m.u(MARGIN));
    // Headers are drawn in the 12 pt font (`popup.rs`).
    let font = m.font_id(FontSize::Pt12);
    let width = |text: &str| {
        let galley = ctx
            .fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER));
        galley.size().x
    };
    let max_w = m.floor_px(bounds.width() - 2.0 * m.u(PAD_X));
    let wrapped: Vec<String> = lines.iter().flat_map(|l| wrap(l, max_w, width)).collect();
    let mut items: Vec<PopupItem<Answer>> = wrapped
        .iter()
        .enumerate()
        .map(|(r, line)| PopupItem::header(0, r, line.clone()))
        .collect();
    let choices = match confirm {
        None => vec![(Answer::Yes, "OK".to_owned())],
        Some((yes, no)) => extra
            .map(|extra| (Answer::Extra, extra.to_owned()))
            .into_iter()
            .chain([(Answer::Yes, yes.clone()), (Answer::No, no.clone())])
            .collect(),
    };
    for (row, (answer, label)) in (wrapped.len()..).zip(choices) {
        items.push(PopupItem::new(answer, 0, row, label));
    }
    let colors = &app.cx.theme.colors;
    // A new message (another tag or other lines) opens from the start; a resize that wraps the lines again does not
    // replay it, so the unwrapped `lines` are hashed.
    let a = anim::message_open(anim::frame(ctx, id.with((tag, lines))));
    if a > 0.0 {
        ctx.request_repaint();
    }
    // As the items' scroll viewport: a text taller than `bounds` scrolls inside it.
    let full = popup::popup_rect(ctx, &items, screen.center(), bounds, m).intersect(bounds);
    let h = m.round_px((1.0 - a) * full.height());
    let boxed = Rect::from_center_size(
        pos2(screen.center().x, full.center().y),
        vec2(screen.width(), h),
    );
    // In the items' layer, before them: under them, over everything else.
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, id));
    let bg = colors.background.to_color32();
    painter.rect_filled(screen, 0.0, anim::dim(bg, anim::DIM_ALPHA * (1.0 - a)));
    painter.rect_filled(boxed, 0.0, bg);
    let px = 1.0 / m.ppp();
    let [from, to] = LINE_PX.map(|p| p * px);
    for y in [
        boxed.top() + from..=boxed.top() + to,
        boxed.bottom() - to..=boxed.bottom() - from,
    ] {
        let line = Rect::from_x_y_ranges(boxed.x_range(), y).intersect(boxed);
        if line.height() > 0.0 {
            painter.rect_filled(line, 0.0, colors.menu_fill.to_color32());
        }
    }
    let result = popup::show_on_box(ctx, id, screen.center(), bounds, &items, &app.cx, a == 0.0);
    result.close.then(|| match (confirm, result.selected) {
        (None, _) => Nav::Close,
        (Some(_), Some(Answer::Extra)) => Nav::Extra(tag),
        (Some(_), answer) => Nav::Answer(tag, answer == Some(Answer::Yes)),
    })
}

#[cfg(test)]
mod tests {
    use eframe::egui::{
        Color32, Context, Event, PointerButton, Pos2, RawInput, Shape, epaint::ClippedShape,
    };

    use super::*;
    use crate::audio_link::AudioLink;
    use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
    use crate::project_files::ProjectFiles;
    use crate::project_files::mem::MemFiles;
    use crate::settings::Settings;
    use crate::song::Song;
    use crate::theme::theme_by_id;
    use crate::ui::UiCtx;
    use crate::ui::main_screen::Regions;

    const SCREEN: Rect = Rect {
        min: pos2(0.0, 0.0),
        max: pos2(1170.0, 540.0),
    };

    /// One frame of `Delete?` (`Yes` / `No`) on a 1170 × 540 screen; returns what it asked for and painted.
    fn frame(ctx: &Context, time: f64, events: Vec<Event>) -> (Option<Nav>, Vec<ClippedShape>) {
        let raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(time),
            events,
            ..Default::default()
        };
        let (theme, m) = (
            theme_by_id(0),
            Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX),
        );
        let (mut song, mut audio, mut settings) =
            (Song::default(), AudioLink::default(), Settings::default());
        let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        let mut selected = 0;
        let mut nav = None;
        let out = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let cx = UiCtx {
                    theme: &theme,
                    m: &m,
                };
                let mut app = AppCtx {
                    cx,
                    regions: Regions::new(SCREEN, &m),
                    song: &mut song,
                    audio: &mut audio,
                    selected: &mut selected,
                    snapshot: None,
                    playing: false,
                    armed: false,
                    previewing: false,
                    middle_c_is_c4: false,
                    settings: &mut settings,
                    ghost_notes: true,
                    files: &mut files,
                    edit: None,
                    answer: None,
                    covered: false,
                    playhead: None,
                    seek: None,
                    midi_devices: &[],
                    midi_rescan: false,
                };
                let confirm = ("Yes".to_owned(), "No".to_owned());
                nav = show(
                    ui,
                    &mut app,
                    "ask",
                    &["Delete?".to_owned()],
                    Some(&confirm),
                    None,
                );
            });
        });
        (nav, out.shapes)
    }

    fn rects(shapes: &[ClippedShape], fill: Color32) -> Vec<Rect> {
        (shapes.iter())
            .filter_map(|c| match &c.shape {
                Shape::Rect(r) if r.fill == fill => Some(r.rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_box_opens_from_its_centre_over_11_frames_and_its_text_shows_at_the_end() {
        let ctx = Context::default();
        anim::enable(&ctx);
        let colors = theme_by_id(0).colors;
        let (bg, line) = (
            colors.background.to_color32(),
            colors.menu_fill.to_color32(),
        );
        // Three 1.25 U rows: H = 120, centred on the screen.
        let mut a = 1.0f32;
        for n in 1..=12 {
            let (_, shapes) = frame(&ctx, n as f64 / 60.0, vec![]);
            a *= 0.4;
            if a < 0.0001 {
                a = 0.0;
            }
            let h = ((1.0 - a) * 120.0).round();
            let dim = bg.gamma_multiply(0.6 * (1.0 - a));
            assert_eq!(
                rects(&shapes, dim).first(),
                Some(&SCREEN),
                "frame {n}: the dim"
            );
            let boxed = (rects(&shapes, bg).into_iter())
                .find(|r| r.width() == SCREEN.width() && r != &SCREEN);
            let want = Rect::from_center_size(pos2(585.0, 270.0), vec2(1170.0, h));
            assert_eq!(boxed, (h > 0.0).then_some(want), "frame {n}");
            if h >= 6.0 {
                let lines = rects(&shapes, line);
                assert!(lines.contains(&Rect::from_x_y_ranges(
                    0.0..=1170.0,
                    want.top() + 1.0..=want.top() + 3.0
                )));
                assert!(lines.contains(&Rect::from_x_y_ranges(
                    0.0..=1170.0,
                    want.bottom() - 3.0..=want.bottom() - 1.0
                )));
            }
            let texts = (shapes.iter())
                .filter(|c| matches!(&c.shape, Shape::Text(_)))
                .count();
            assert_eq!(texts > 0, n >= 11, "frame {n}: a = {a}");
        }
    }

    #[test]
    fn a_choice_takes_a_click_before_the_box_has_opened() {
        let ctx = Context::default();
        anim::enable(&ctx);
        frame(&ctx, 0.1, vec![]);
        frame(&ctx, 0.2, vec![]);
        // `Yes` is the second row: y = 270 − 60 + 1.5·40.
        let at = Pos2::new(585.0, 270.0);
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&ctx, 0.3, vec![Event::PointerMoved(at)]);
        frame(&ctx, 0.4, vec![button(true)]);
        let (nav, shapes) = frame(&ctx, 0.5, vec![button(false)]);
        assert_eq!(nav, Some(Nav::Answer("ask", true)));
        assert!(
            !shapes.iter().any(|c| matches!(&c.shape, Shape::Text(_))),
            "not drawn yet"
        );
    }
}

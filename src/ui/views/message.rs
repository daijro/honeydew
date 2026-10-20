//! A message box: a floating dialog panel ([`dialog::panel`]) min(20 U, W − 4 px) wide, centred on the screen. Its
//! lines are 12 pt `text`, centred and word-wrapped to 60 % of its width ([`super::wrap`]), one per U; under them,
//! 0.5 U down, a row of buttons, `OK` or two or three choices, each 4.5 × 1.25 U, 0.5 U apart and centred (12 pt
//! labels, 10 pt for one that does not fit). The panel is 1 U above the text and 1 U below the buttons, and at most
//! the screen less 1 U margins tall, past which the lines scroll. It grows from its centre, a ← 0.4·a each 1/60 s
//! from 1 at each new message, as the screen dims to α 0.6·(1 − a); the text and buttons show once a = 0 and take
//! input from the start.

use eframe::egui::{self, Area, Color32, Id, LayerId, Order, Rect, ScrollArea, Sense, pos2, vec2};

use super::wrap::wrap;
use super::{Answer, AppCtx, Nav};
use crate::metrics::FontSize;
use crate::ui::widgets::{Button, button};
use crate::ui::{anim, dialog};

/// The panel's room above the text and below the buttons, in U.
const PAD: f32 = 1.0;
/// A line's height, and the gap between the text and the buttons, in U.
const LINE: f32 = 1.0;
const GAP: f32 = 0.5;
/// A button's size and the gap between two, in U.
const BUTTON: [f32; 2] = [4.5, 1.25];
const BUTTON_GAP: f32 = 0.5;
/// The lines wrap to this share of the panel's width.
const TEXT_SHARE: f32 = 0.6;
/// The gap in U between the panel and the screen's top and bottom.
const MARGIN: f32 = 1.0;

/// Shows `lines` centred on the screen, then `OK`, or the `extra` choice (if any) and the `(yes, no)` choices of
/// `confirm`, side by side. `OK` returns `Nav::Close`; a choice returns `Nav::Answer(tag, ..)` or `Nav::Extra(tag)`,
/// and Escape or a click outside answers no. Choice `i`'s id is `message` with `(0, lines.len() + i)`.
pub fn show(
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    tag: &'static str,
    lines: &[String],
    confirm: Option<&(String, String)>,
    extra: Option<&str>,
) -> Option<Nav> {
    show_wrapped(ui, app, (tag, lines), confirm, extra, TEXT_SHARE)
}

/// [`show`] with the lines wrapped to `share` of the panel's width: the `Sound credits` document takes nearly all of
/// it, as its links are long.
pub fn show_wrapped(
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    (tag, lines): (&'static str, &[String]),
    confirm: Option<&(String, String)>,
    extra: Option<&str>,
    share: f32,
) -> Option<Nav> {
    debug_assert!(
        confirm.is_some() || extra.is_none(),
        "{tag}: an extra choice needs the yes and no ones"
    );
    let (ctx, id, cx, screen) = (
        ui.ctx().clone(),
        Id::new("message"),
        &app.cx,
        app.regions.screen,
    );
    let (m, c) = (cx.m, &cx.theme.colors);
    let x = dialog::floating_x(screen, m);
    let font = m.font_id(FontSize::Pt12);
    let width = |text: &str, font: &egui::FontId| {
        let galley = ctx
            .fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER));
        galley.size().x
    };
    let max_w = m.floor_px(share * x.span());
    let wrapped: Vec<String> = (lines.iter())
        .flat_map(|l| wrap(l, max_w, |t| width(t, &font)))
        .collect();
    let choices: Vec<(Answer, &str)> = match confirm {
        None => vec![(Answer::Yes, "OK")],
        Some((yes, no)) => (extra.map(|extra| (Answer::Extra, extra)).into_iter())
            .chain([(Answer::Yes, yes.as_str()), (Answer::No, no.as_str())])
            .collect(),
    };
    // The panel: the text, the gap under it (with text), the buttons, and the room above and below.
    let text_h = wrapped.len() as f32 * m.u(LINE);
    let gap = if wrapped.is_empty() { 0.0 } else { m.u(GAP) };
    let chrome = m.u(2.0 * PAD + BUTTON[1]) + gap;
    let h = m.round_px((chrome + text_h).min(screen.height() - 2.0 * m.u(MARGIN)));
    let top = m.round_px(screen.center().y - h / 2.0);
    let full = Rect::from_x_y_ranges(x, top..=top + h);
    // A new message (another tag or other lines) opens from the start; a resize that wraps the lines again does not
    // replay it, so the unwrapped `lines` are hashed.
    let a = anim::message_open(anim::elapsed(&ctx, id.with((tag, lines))));
    if a > 0.0 {
        ctx.request_repaint();
    }
    // In the panel's layer, before its area: under the text and buttons, over everything else.
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, id));
    let bg = c.background.to_color32();
    painter.rect_filled(screen, 0.0, anim::dim(bg, anim::DIM_ALPHA * (1.0 - a)));
    let grown = Rect::from_center_size(full.center(), full.size() * (1.0 - a));
    if grown.width() >= 1.0 / m.ppp() && grown.height() >= 1.0 / m.ppp() {
        dialog::panel(&painter, c, m, grown);
    }
    let backdrop = crate::ui::backdrop(&ctx, id);
    let outside = backdrop.clicked() || backdrop.secondary_clicked() || backdrop.middle_clicked();
    let mut answer = (crate::ui::close_pressed(&ctx) || outside).then_some(Answer::No);
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(full.min)
        .constrain(false)
        .fade_in(false)
        .show(&ctx, |ui| {
            // The whole panel, so it swallows the clicks that hit no button.
            ui.allocate_exact_size(full.size(), Sense::hover());
            if a > 0.0 {
                ui.set_opacity(0.0);
            }
            let text = c.text.to_color32();
            let buttons_top = full.bottom() - m.u(PAD + BUTTON[1]);
            let room =
                Rect::from_x_y_ranges(full.x_range(), full.top() + m.u(PAD)..=buttons_top - gap);
            let mut scrolled = ui.new_child(egui::UiBuilder::new().max_rect(room));
            ScrollArea::vertical()
                .id_salt(id.with("scroll"))
                .max_height(room.height())
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(&mut scrolled, |ui| {
                    let (column, _) =
                        ui.allocate_exact_size(vec2(room.width(), text_h), Sense::hover());
                    for (i, line) in wrapped.iter().enumerate() {
                        let y = column.top() + (i as f32 + 0.5) * m.u(LINE);
                        let at = pos2(column.center().x, y);
                        ui.painter().text(
                            at,
                            egui::Align2::CENTER_CENTER,
                            line,
                            font.clone(),
                            text,
                        );
                    }
                });
            let n = choices.len() as f32;
            let row_w = m.u(n * BUTTON[0] + (n - 1.0) * BUTTON_GAP);
            let left = m.round_px(full.center().x - row_w / 2.0);
            for (i, (choice, label)) in choices.iter().enumerate() {
                let x = left + i as f32 * m.u(BUTTON[0] + BUTTON_GAP);
                let rect =
                    Rect::from_min_size(pos2(x, buttons_top), vec2(m.u(BUTTON[0]), m.u(BUTTON[1])));
                let bid = id.with((0_usize, lines.len() + i));
                // 12 pt, or 10 pt when the label does not fit (with 0.25 U each side).
                let fits = width(label, &font) <= rect.width() - m.u(0.5);
                let font = if fits { FontSize::Pt12 } else { FontSize::Pt10 };
                let b = Button {
                    font,
                    ..dialog::button(bid, rect, label, c, m)
                };
                if button(ui, cx, &b, true).clicked() {
                    answer = Some(*choice);
                }
            }
        });
    answer.map(|answer| match (confirm, answer) {
        (None, _) => Nav::Close,
        (Some(_), Answer::Extra) => Nav::Extra(tag),
        (Some(_), answer) => Nav::Answer(tag, answer == Answer::Yes),
    })
}

#[cfg(test)]
mod tests {
    use eframe::egui::{
        Color32, Context, Event, PointerButton, Pos2, RawInput, Shape, epaint::ClippedShape,
    };

    use super::*;
    use crate::audio_link::AudioLink;
    use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
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
            Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE),
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

    /// `Delete?` with `Yes` / `No` at U = 32 on 1170 × 540: a 20 U panel, 1 + 1 + 0.5 + 1.25 + 1 U tall, centred.
    const PANEL: Rect = Rect {
        min: pos2(265.0, 194.0),
        max: pos2(905.0, 346.0),
    };

    #[test]
    fn the_panel_grows_from_its_centre_over_11_frames_and_its_text_shows_at_the_end() {
        let ctx = Context::default();
        anim::enable(&ctx);
        let colors = theme_by_id(0).colors;
        let (bg, panel) = (
            colors.background.to_color32(),
            colors.menu_fill.to_color32(),
        );
        for n in 1..=12 {
            let (_, shapes) = frame(&ctx, n as f64 / 60.0, vec![]);
            // Tests run at 60 fps: frame n is n/60 s in.
            let a = anim::message_open(n as f32 / 60.0);
            let dim = bg.gamma_multiply(0.6 * (1.0 - a));
            assert_eq!(
                rects(&shapes, dim).first(),
                Some(&SCREEN),
                "frame {n}: the dim"
            );
            let want = Rect::from_center_size(PANEL.center(), PANEL.size() * (1.0 - a));
            assert_eq!(rects(&shapes, panel), [want], "frame {n}");
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
        // `Yes` is the first of two 4.5 U buttons 0.5 U apart, centred, 1 U above the panel's bottom.
        let at = Pos2::new(585.0 - 2.5 * 32.0, PANEL.bottom() - 1.625 * 32.0);
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

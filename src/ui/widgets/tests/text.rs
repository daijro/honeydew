//! The text routines in a real context: the ASCII ellipsis, the kept tail, and shrink's 1 pt steps down to 6 pt before
//! it truncates.

use super::*;

/// Runs `draw` in one frame and returns its text boxes.
fn drawn(draw: impl Fn(&Ui, &UiCtx)) -> Vec<TextBox> {
    let (theme, m) = (theme_by_id(0), metrics());
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let raw = RawInput {
        screen_rect: Some(SCREEN),
        ..Default::default()
    };
    let output = Context::default().run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| draw(ui, &cx));
    });
    text_boxes(&output.shapes)
}

/// The width of `text` at `pt` points in a fresh context.
fn width(text: &str, pt: f32) -> f32 {
    let ctx = Context::default();
    let mut w = 0.0;
    let _ = ctx.run(RawInput::default(), |ctx| {
        w = ctx
            .fonts_mut(|f| {
                f.layout_no_wrap(text.to_owned(), metrics().font_id_pt(pt), Color32::WHITE)
            })
            .size()
            .x;
    });
    w
}

const RECT: Rect = Rect {
    min: pos2(100.0, 100.0),
    max: pos2(196.0, 132.0),
};

use eframe::egui::Ui;

#[test]
fn truncated_text_ends_in_three_ascii_periods_and_keeps_k_tail_characters() {
    // 140 points: narrower than `Reverb Hall Large` at 10 pt, wide enough to keep a head longer than the tail.
    const RECT: Rect = Rect {
        min: pos2(100.0, 100.0),
        max: pos2(240.0, 132.0),
    };
    let font = metrics().font_id(FontSize::Pt10);
    for (k, tail) in [(5, "Large"), (0, "")] {
        let boxes = drawn(|ui, _| {
            truncated(
                ui.painter(),
                RECT,
                Align2::LEFT_CENTER,
                "Reverb Hall Large",
                font.clone(),
                Color32::RED,
                k,
            )
        });
        assert_eq!(boxes.len(), 1);
        let (text, rect, ..) = &boxes[0];
        let (head, rest) = text.split_once("...").expect("an ASCII ellipsis");
        assert_eq!(rest, tail, "{text}");
        assert!(
            head.len() > 5 && "Reverb Hall Large".starts_with(head),
            "{text}"
        );
        assert!(rect.width() <= RECT.width(), "{text}: {rect:?}");
        // Left-aligned, vertically centred, and the galley itself is not elided by egui.
        assert!(
            (rect.left() - RECT.left()).abs() < 1e-3
                && (rect.center().y - RECT.center().y).abs() < 1e-3
        );
        assert!(!boxes[0].3);
    }
    // Text that fits is drawn whole.
    let boxes = drawn(|ui, _| {
        truncated(
            ui.painter(),
            RECT,
            Align2::LEFT_CENTER,
            "Hall",
            font.clone(),
            Color32::RED,
            5,
        )
    });
    assert_eq!(boxes[0].0, "Hall");
}

#[test]
fn shrink_steps_a_point_at_a_time_then_truncates_at_6_pt() {
    // A text that fits the 96-point rect at some size between 12 and 7 pt: drawn whole at the largest such size.
    let text = "Snare Tight";
    let want = (6u8..=12)
        .rev()
        .find(|&pt| width(text, f32::from(pt)) <= RECT.width())
        .expect("fits somewhere");
    assert!(want > 6 && want < 12, "{want}");
    let boxes = drawn(|ui, cx| {
        shrink(
            ui.painter(),
            cx.m,
            RECT,
            Align2::LEFT_CENTER,
            text,
            12,
            Color32::RED,
        )
    });
    assert_eq!(boxes[0].0, text);
    assert_eq!(boxes[0].2, metrics().font_id_pt(f32::from(want)).size);
    // Starting below that size, it keeps the starting size.
    let boxes = drawn(|ui, cx| {
        shrink(
            ui.painter(),
            cx.m,
            RECT,
            Align2::LEFT_CENTER,
            text,
            7,
            Color32::RED,
        )
    });
    assert_eq!(boxes[0].2, metrics().font_id_pt(7.0).size);
    // Too wide even at 6 pt: truncated at 6 pt, keeping 5 tail characters.
    let long = "A sample name far too long for the rect";
    let boxes = drawn(|ui, cx| {
        shrink(
            ui.painter(),
            cx.m,
            RECT,
            Align2::LEFT_CENTER,
            long,
            12,
            Color32::RED,
        )
    });
    assert_eq!(boxes[0].2, metrics().font_id_pt(6.0).size);
    assert!(boxes[0].0.ends_with("... rect"), "{}", boxes[0].0);
    assert!(boxes[0].1.width() <= RECT.width());
}

#[cfg(has_app_font)]
#[test]
fn truncation_with_the_app_font_keeps_every_character_that_fits() {
    // The app font is scaled by a `FontTweak`: the per-character widths of the fit must be too.
    let font = metrics().font_id(FontSize::Pt10);
    let ctx = Context::default();
    crate::fonts::install(&ctx);
    let mut boxes = Vec::new();
    for _ in 0..2 {
        let output = ctx.run(RawInput::default(), |ctx| {
            let w = |s: &str| {
                ctx.fonts_mut(|f| f.layout_no_wrap(s.into(), font.clone(), Color32::RED))
                    .size()
                    .x
            };
            // Room for seven of the twelve and the dots.
            let rect = Rect::from_min_size(RECT.min, vec2(w("mmmmmmm") + w("...") + 0.5, 32.0));
            egui::CentralPanel::default().show(ctx, |ui| {
                truncated(
                    ui.painter(),
                    rect,
                    Align2::LEFT_CENTER,
                    "mmmmmmmmmmmm",
                    font.clone(),
                    Color32::RED,
                    0,
                )
            });
        });
        boxes = text_boxes(&output.shapes);
    }
    assert_eq!(boxes[0].0, "mmmmmmm...");
}

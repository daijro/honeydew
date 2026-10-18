//! The app font (Open Sans SemiCondensed SemiBold). The file is committed and embedded at build time; if it is
//! removed, egui's built-in fonts are used. See `assets/fonts/README.md`.
//!
//! Line height H = ascent − descent at the pixel size, which equals the pixel size exactly (scale = px/(2189 + 600)),
//! and the baseline is T + 0.7849·px. epaint scales a font so that its em (2048 units) is the `FontId` size, which
//! made every text 2789/2048 ≈ 1.36× too big. `FontTweak` maps the em back so that a `FontId` of `px` has
//! ascent − descent = `px`; ascent, descent, row height, glyphs and advances all follow it.

use eframe::egui;

/// Makes the app font the first `Proportional` font, and draws every `Proportional` font at the app font's em.
#[cfg(has_app_font)]
pub fn install(ctx: &egui::Context) {
    use std::sync::Arc;

    const NAME: &str = "OpenSans_SemiCondensed-SemiBold";
    /// The em is 2048 units, ascent − descent (hhea 2189, −600) is 2789.
    const SCALE: f32 = 2048.0 / 2789.0;

    let mut fonts = egui::FontDefinitions::default();
    // egui's fallbacks (Ubuntu-Light for √ and arrows, the emoji fonts) at the same em, keeping their own tweaks.
    for name in &fonts.families[&egui::FontFamily::Proportional] {
        if let Some(data) = fonts.font_data.get_mut(name) {
            Arc::make_mut(data).tweak.scale *= SCALE;
        }
    }
    fonts.font_data.insert(
        NAME.to_owned(),
        Arc::new(
            egui::FontData::from_static(include_bytes!(
                "../assets/fonts/OpenSans_SemiCondensed-SemiBold.ttf"
            ))
            .tweak(egui::FontTweak {
                scale: SCALE,
                ..Default::default()
            }),
        ),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, NAME.to_owned());
    ctx.set_fonts(fonts);
}

/// No app font available: keep egui's defaults.
#[cfg(not(has_app_font))]
pub fn install(_ctx: &egui::Context) {}

#[cfg(all(test, has_app_font))]
mod tests {
    use eframe::egui::{
        self, Align2, Color32, Context, FontId, Rect, TextureId, epaint::Primitive, pos2, vec2,
    };

    /// A context with the app font at `ppp`; `f` runs inside a frame (the second: the first loads the fonts).
    fn with_fonts<R>(ppp: f32, mut f: impl FnMut(&Context) -> R) -> (Context, egui::FullOutput, R) {
        let ctx = Context::default();
        super::install(&ctx);
        let mut raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 100.0))),
            ..Default::default()
        };
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(ppp);
        let _ = ctx.run(raw.clone(), |_| {});
        let mut r = None;
        let out = ctx.run(raw, |ctx| r = Some(f(ctx)));
        (ctx, out, r.unwrap())
    }

    #[test]
    fn a_font_size_is_its_line_height() {
        // H = ascent − descent at the pixel size equals the pixel size.
        for ppp in [1.0f32, 2.0] {
            for px in [8.0f32, 13.0, 21.0, 50.0, 85.0] {
                let font = FontId::proportional(px / ppp);
                let (_, _, (h, row)) = with_fonts(ppp, |ctx| {
                    ctx.fonts_mut(|f| {
                        let g = f.layout_no_wrap("Hg".into(), font.clone(), Color32::WHITE);
                        (g.size().y, f.row_height(&font))
                    })
                });
                assert_eq!(h * ppp, px, "galley at {px} px, ppp {ppp}");
                assert!(
                    (row * ppp - px).abs() <= 1.0 / 16.0,
                    "row height {row} at {px} px"
                );
            }
        }
    }

    /// The width and first glyph's ink box size of `text` at `size` points, in a context with the app font or not.
    fn glyph_box(app_font: bool, text: &str, size: f32) -> (f32, egui::Vec2) {
        let ctx = Context::default();
        if app_font {
            super::install(&ctx);
        }
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        let mut out = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let g = ctx.fonts_mut(|f| {
                f.layout_no_wrap(text.into(), FontId::proportional(size), Color32::WHITE)
            });
            out = Some((g.size().x, g.rows[0].glyphs[0].uv_rect.size));
        });
        out.unwrap()
    }

    #[test]
    fn fallback_glyphs_are_drawn_at_the_app_fonts_em() {
        // `→` and `😀` are not in the app font: egui's fallbacks (Ubuntu-Light, NotoEmoji) draw them at the same
        // em as the app font, 2048/2789 of the size, with their own tweaks kept.
        let scale = 2048.0 / 2789.0;
        for c in ["→", "😀"] {
            assert_eq!(
                glyph_box(true, c, 50.0),
                glyph_box(false, c, 50.0 * scale),
                "{c}"
            );
        }
    }

    /// The device-pixel rows holding ink of `text` at `px` pixels, drawn centred in an `row_px` tall row (the
    /// glyph quads rasterised from the font atlas, nearest texel: glyphs sit on whole pixels).
    fn ink_rows(ppp: f32, text: &str, px: f32, row_px: f32) -> (usize, usize) {
        let row = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, row_px / ppp));
        let (ctx, out, ()) = with_fonts(ppp, |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            let font = FontId::proportional(px / ppp);
            painter.text(
                row.center(),
                Align2::CENTER_CENTER,
                text,
                font,
                Color32::WHITE,
            );
        });
        let atlas = ctx.fonts(|f| f.image());
        let mut rows = Vec::new();
        for prim in ctx.tessellate(out.shapes, ppp) {
            let Primitive::Mesh(mesh) = prim.primitive else {
                continue;
            };
            assert_eq!(mesh.texture_id, TextureId::default());
            for q in mesh.vertices.as_chunks::<4>().0 {
                let (a, b) = ((q[0].pos * ppp).round(), (q[3].pos * ppp).round());
                let (ua, ub) = (q[0].uv, q[3].uv);
                for y in a.y as usize..b.y as usize {
                    for x in a.x as usize..b.x as usize {
                        let fx = (x as f32 + 0.5 - a.x) / (b.x - a.x);
                        let fy = (y as f32 + 0.5 - a.y) / (b.y - a.y);
                        let tx = ((ua.x + fx * (ub.x - ua.x)) * atlas.size[0] as f32) as usize;
                        let ty = ((ua.y + fy * (ub.y - ua.y)) * atlas.size[1] as f32) as usize;
                        if atlas.pixels[ty * atlas.size[0] + tx].a() > 0 {
                            rows.push(y);
                        }
                    }
                }
            }
        }
        (*rows.iter().min().unwrap(), *rows.iter().max().unwrap())
    }

    #[test]
    fn the_worked_example_puts_h_on_rows_27_to_53() {
        // U = 80, 10 pt = 50 px centred in an 80 px row: T = 15, baseline 54.243,
        // "H" ink rows 27–53.
        for ppp in [1.0f32, 2.0] {
            assert_eq!(ink_rows(ppp, "H", 50.0, 80.0), (27, 53), "ppp {ppp}");
        }
    }
}

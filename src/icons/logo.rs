//! Our logo glyph (`assets/branding/honeydew.svg`), which [`IconId::AppLogo`](super::IconId::AppLogo) draws as a
//! coloured image. Each egui context loads a rasterisation once, the first time it draws the logo at that size.

use eframe::egui::{
    Color32, ColorImage, Context, Id, Rect, Shape, TextureHandle, TextureOptions, pos2,
};

/// The logo's rasterisations by edge in pixels, smallest first.
const PNGS: [(usize, &[u8]); 2] = [
    (
        32,
        include_bytes!("../../assets/branding/honeydew-32.png"),
    ),
    (
        256,
        include_bytes!("../../assets/branding/honeydew-256.png"),
    ),
];

/// The rasterisation for an edge of `px` device pixels: the smallest at least that large (only ever scaled down, by
/// less than 2), else the largest.
fn source_for(px: f32) -> (usize, &'static [u8]) {
    let last = PNGS[PNGS.len() - 1];
    PNGS.into_iter()
        .find(|&(n, _)| n as f32 >= px)
        .unwrap_or(last)
}

/// `png` as an image: `None` unless it decodes as an 8-bit RGBA PNG of `size` pixels.
fn decode_rgba(png: &[u8], size: [usize; 2]) -> Option<ColorImage> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    // At most the image's own pixels in memory, however the file says otherwise.
    decoder.set_limits(png::Limits {
        bytes: size[0] * size[1] * 4 * 2,
    });
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    let rgba8 = (info.color_type, info.bit_depth) == (png::ColorType::Rgba, png::BitDepth::Eight);
    if !rgba8 || (info.width, info.height) != (size[0] as u32, size[1] as u32) {
        return None;
    }
    let mut pixels = vec![0; reader.output_buffer_size()?];
    reader.next_frame(&mut pixels).ok()?;
    let mut image = ColorImage::from_rgba_unmultiplied(size, &pixels);
    recolor(&mut image);
    Some(image)
}

/// Recolors the original melon artwork to the requested orange-flesh/green-rind palette.
fn recolor(image: &mut ColorImage) {
    for pixel in &mut image.pixels {
        let [r, g, b, a] = pixel.to_srgba_unmultiplied();
        if a == 0 {
            continue;
        }

        let green_rind = g > r && g > b;
        let lightness = (u16::from(r) + u16::from(g) + u16::from(b)) as f32 / (3.0 * 255.0);
        let (base, shadow) = if green_rind {
            ([74.0, 145.0, 48.0], [20.0, 65.0, 35.0])
        } else {
            ([244.0, 105.0, 28.0], [168.0, 48.0, 12.0])
        };
        let amount = lightness.clamp(0.15, 0.95);
        let mix = (amount - 0.15) / 0.8;
        let [br, bg, bb] = base;
        let [sr, sg, sb] = shadow;
        *pixel = Color32::from_rgba_unmultiplied(
            (sr + (br - sr) * mix).round() as u8,
            (sg + (bg - sg) * mix).round() as u8,
            (sb + (bb - sb) * mix).round() as u8,
            a,
        );
    }
}

/// The logo on `square` in `ctx`.
pub(super) fn logo_shape(ctx: &Context, square: Rect, _color: Color32) -> Shape {
    let (n, png) = source_for(square.width() * ctx.pixels_per_point());
    let id = Id::new(("honeydew_logo", n));
    let texture = ctx
        .data(|d| d.get_temp::<TextureHandle>(id))
        .unwrap_or_else(|| {
            let image = decode_rgba(png, [n, n]).expect("our logo is an RGBA PNG");
            let handle = ctx.load_texture("honeydew_logo", image, TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
            handle
        });
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    Shape::image(texture.id(), square, uv, Color32::WHITE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::{IconId, painted_shapes};
    use eframe::egui::vec2;

    #[test]
    fn every_rasterisation_decodes_at_its_size() {
        for (n, png) in PNGS {
            let image = decode_rgba(png, [n, n]);
            assert!(image.is_some(), "honeydew-mono-{n}.png");
        }
    }

    #[test]
    fn every_rasterisation_has_orange_and_green_pixels() {
        for (n, png) in PNGS {
            let image = decode_rgba(png, [n, n]).unwrap();
            let px = &image.pixels;
            let orange = px.iter().any(|c| {
                let [r, g, b, a] = c.to_srgba_unmultiplied();
                a > 0 && r > g && g > b
            });
            let green = px.iter().any(|c| {
                let [r, g, b, a] = c.to_srgba_unmultiplied();
                a > 0 && g > r && g > b
            });
            assert!(orange && green, "honeydew-{n}.png");
            let opaque = px.iter().filter(|c| c.a() > 0).count();
            assert!(
                opaque > px.len() / 4,
                "honeydew-{n}.png: {opaque} opaque"
            );
            assert!(
                opaque < px.len(),
                "honeydew-{n}.png: {opaque} opaque"
            );
        }
    }

    #[test]
    fn the_logo_is_drawn_from_the_smallest_rasterisation_at_least_its_size() {
        // 4/3 U at U = 32: 42.7 px at ppp 1, 85.3 px at ppp 2.
        for (px, n) in [
            (10.0, 32),
            (31.9, 32),
            (32.0, 32),
            (48.0, 256),
            (128.0, 256),
            (256.0, 256),
            (900.0, 256),
        ] {
            assert_eq!(source_for(px).0, n, "{px} px");
        }
    }

    #[test]
    fn the_app_logo_paints_the_colored_artwork_once_loaded() {
        let ctx = Context::default();
        let square = Rect::from_min_size(pos2(10.0, 10.0), vec2(42.0, 42.0));
        let mesh = |color| match &painted_shapes(&ctx, IconId::AppLogo, square, color)[..] {
            [Shape::Mesh(mesh)] => mesh.clone(),
            other => panic!("{other:?}"),
        };
        let (a, b) = (mesh(Color32::WHITE), mesh(Color32::from_black_alpha(128)));
        assert_eq!(a.texture_id, b.texture_id, "loaded once");
        assert_eq!(a.calc_bounds(), square);
        // The full-color artwork stays unchanged by the requested tint.
        assert!(a.vertices.iter().all(|v| v.color == Color32::WHITE));
        assert!(
            b.vertices
                .iter()
                .all(|v| v.color == Color32::WHITE)
        );
    }
}

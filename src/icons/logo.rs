//! Our logo glyph (`assets/branding/honeydew-mono.svg`), which [`IconId::AppLogo`](super::IconId::AppLogo) draws as a
//! tinted image: the one-colour silhouette of the coloured app icon (`honeydew.svg`). Each egui context loads a
//! rasterisation once, the first time it draws the logo at that size.

use eframe::egui::{
    Color32, ColorImage, Context, Id, Rect, Shape, TextureHandle, TextureOptions, pos2,
};

/// The silhouette's rasterisations (white on transparency) by edge in pixels, smallest first.
const PNGS: [(usize, &[u8]); 4] = [
    (
        48,
        include_bytes!("../../assets/branding/honeydew-mono-48.png"),
    ),
    (
        64,
        include_bytes!("../../assets/branding/honeydew-mono-64.png"),
    ),
    (
        128,
        include_bytes!("../../assets/branding/honeydew-mono-128.png"),
    ),
    (
        256,
        include_bytes!("../../assets/branding/honeydew-mono-256.png"),
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
    Some(ColorImage::from_rgba_unmultiplied(size, &pixels))
}

/// The logo on `square` in `ctx`, tinted `color`.
pub(super) fn logo_shape(ctx: &Context, square: Rect, color: Color32) -> Shape {
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
    Shape::image(texture.id(), square, uv, color)
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
    fn every_rasterisation_is_a_white_mask_of_the_silhouette() {
        // The in-app logo is the one-colour silhouette (`honeydew-mono.svg`), white on transparency so that the draw's
        // tint is its colour; the coloured melon has no white pixel.
        for (n, png) in PNGS {
            let image = decode_rgba(png, [n, n]).unwrap();
            let px = &image.pixels;
            let white = px.iter().all(|c| {
                let [r, g, b, a] = c.to_srgba_unmultiplied();
                [r, g, b] == [255; 3] || a == 0
            });
            assert!(white, "honeydew-mono-{n}.png");
            let opaque = px.iter().filter(|c| c.a() == 255).count();
            assert!(
                opaque > px.len() / 4,
                "honeydew-mono-{n}.png: {opaque} opaque"
            );
            assert!(
                opaque < px.len() * 3 / 4,
                "honeydew-mono-{n}.png: {opaque} opaque"
            );
        }
    }

    #[test]
    fn the_logo_is_drawn_from_the_smallest_rasterisation_at_least_its_size() {
        // 4/3 U at U = 32: 42.7 px at ppp 1, 85.3 px at ppp 2.
        for (px, n) in [
            (10.0, 48),
            (42.7, 48),
            (48.0, 48),
            (85.3, 128),
            (128.0, 128),
            (170.7, 256),
            (900.0, 256),
        ] {
            assert_eq!(source_for(px).0, n, "{px} px");
        }
    }

    #[test]
    fn the_app_logo_paints_the_silhouette_tinted_once_loaded() {
        let ctx = Context::default();
        let square = Rect::from_min_size(pos2(10.0, 10.0), vec2(42.0, 42.0));
        let mesh = |color| match &painted_shapes(&ctx, IconId::AppLogo, square, color)[..] {
            [Shape::Mesh(mesh)] => mesh.clone(),
            other => panic!("{other:?}"),
        };
        let (a, b) = (mesh(Color32::RED), mesh(Color32::from_black_alpha(128)));
        assert_eq!(a.texture_id, b.texture_id, "loaded once");
        assert_eq!(a.calc_bounds(), square);
        // The tint is the glyph's colour, a translucent one fades it.
        assert!(a.vertices.iter().all(|v| v.color == Color32::RED));
        assert!(
            b.vertices
                .iter()
                .all(|v| v.color == Color32::from_black_alpha(128))
        );
    }
}

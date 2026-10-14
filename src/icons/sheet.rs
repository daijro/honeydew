//! The icon sheet of the user's imported content: `<content folder>/wrap.png`, a 512 × 1024
//! RGBA PNG, white shapes on transparency, 8 columns of 64 px cells (`IconID n` at column `n & 7`, row `n >> 3`).
//! Loaded, the app draws its icons from it, tinted as ours are, except the trademark IDs 6, 16, 17, 18 and 20, which
//! keep our neutral glyphs. A file that is missing or does not decode as such a sheet changes nothing: our drawn set
//! stays. Each egui context keeps its own texture ([`install_sheet`]).

use eframe::egui::{
    Color32, ColorImage, Context, Id, Rect, Shape, TextureHandle, TextureId, TextureOptions, pos2,
    vec2,
};

use super::IconId;

/// The sheet's file name, in the content folder.
pub const SHEET_FILE: &str = "wrap.png";
/// The sheet's size in pixels, and its cells'.
const SHEET_W: usize = 512;
const SHEET_H: usize = 1024;
const CELL: usize = 64;

/// The trademark IDs: never drawn from the imported sheet.
const TRADEMARKS: [IconId; 5] = [
    IconId::AppLogo,
    IconId::PlatformA,
    IconId::PlatformM,
    IconId::PlatformW,
    IconId::WirelessMidi,
];

/// Where a context keeps the sheet's texture handle and id.
fn handle_id() -> Id {
    Id::new("original_icon_sheet")
}

/// `png` as the sheet: `None` unless it decodes as a 512 × 1024, 8-bit RGBA PNG.
pub fn decode_sheet(png: &[u8]) -> Option<ColorImage> {
    decode_rgba(png, [SHEET_W, SHEET_H])
}

/// `png` as an image: `None` unless it decodes as an 8-bit RGBA PNG of `size` pixels.
pub(super) fn decode_rgba(png: &[u8], size: [usize; 2]) -> Option<ColorImage> {
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

/// Uploads the sheet `png` decodes to ([`decode_sheet`]) as `ctx`'s icon atlas. Returns whether it did; a bad sheet
/// leaves our drawn set.
pub fn install_sheet(ctx: &Context, png: &[u8]) -> bool {
    let Some(sheet) = decode_sheet(png) else {
        return false;
    };
    let handle = ctx.load_texture("original_icon_sheet", sheet, TextureOptions::LINEAR);
    let id = handle.id();
    // The handle keeps the texture; the id is what each draw reads.
    ctx.data_mut(|d| {
        d.insert_temp::<TextureHandle>(handle_id(), handle);
        d.insert_temp(handle_id(), id);
    });
    true
}

/// Icon `id` from `ctx`'s imported sheet, mapped onto `square` and tinted `color`; `None` without a sheet, and for a
/// trademark ID.
pub(super) fn sheet_shape(
    ctx: &Context,
    id: IconId,
    square: Rect,
    color: Color32,
) -> Option<Shape> {
    if TRADEMARKS.contains(&id) {
        return None;
    }
    let texture = ctx.data(|d| d.get_temp::<TextureId>(handle_id()))?;
    let n = id as usize;
    let (w, h) = (SHEET_W as f32, SHEET_H as f32);
    let corner = pos2(((n & 7) * CELL) as f32, ((n >> 3) * CELL) as f32);
    // Half a texel in, so that filtering never reads a neighbouring cell.
    let cell = Rect::from_min_size(corner, vec2(CELL as f32, CELL as f32)).shrink(0.5);
    let uv = Rect::from_min_max(
        pos2(cell.min.x / w, cell.min.y / h),
        pos2(cell.max.x / w, cell.max.y / h),
    );
    Some(Shape::image(texture, square, uv, color))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A PNG of `rgba` (`w` × `h`, 8-bit RGBA) from our own minimal encoder: one IDAT of stored deflate blocks.
    pub(crate) fn png(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
        fn crc(bytes: &[u8]) -> u32 {
            let mut crc = !0u32;
            for &b in bytes {
                crc ^= u32::from(b);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        let chunk = |out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]| {
            out.extend((data.len() as u32).to_be_bytes());
            let body = [&kind[..], data].concat();
            out.extend(&body);
            out.extend(crc(&body).to_be_bytes());
        };
        // Each row: filter type 0, then its pixels.
        let raw: Vec<u8> = rgba
            .chunks(w * 4)
            .flat_map(|row| [&[0][..], row].concat())
            .collect();
        let mut zlib = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
        for (i, block) in blocks.iter().enumerate() {
            zlib.push(u8::from(i + 1 == blocks.len()));
            let len = block.len() as u16;
            zlib.extend(len.to_le_bytes());
            zlib.extend((!len).to_le_bytes());
            zlib.extend(*block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in &raw {
            a = (a + u32::from(byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        zlib.extend(((b << 16) | a).to_be_bytes());
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = (w as u32).to_be_bytes().to_vec();
        ihdr.extend((h as u32).to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &zlib);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// A fake sheet: transparent, with a white square in every cell; the square's size names the cell (`2 + n % 60`
    /// px), so each icon's cell can be told apart.
    pub(crate) fn fake_sheet() -> Vec<u8> {
        let mut rgba = vec![0u8; SHEET_W * SHEET_H * 4];
        for n in 0..128 {
            let (cx, cy) = ((n & 7) * CELL, (n >> 3) * CELL);
            let side = 2 + n % 60;
            for y in cy + 2..cy + 2 + side {
                for x in cx + 2..cx + 2 + side {
                    rgba[(y * SHEET_W + x) * 4..][..4].copy_from_slice(&[255; 4]);
                }
            }
        }
        png(SHEET_W, SHEET_H, &rgba)
    }

    /// What `draw_icon` paints for `id` in `ctx`, tinted red.
    fn painted(ctx: &Context, id: IconId) -> Vec<Shape> {
        let square = Rect::from_min_size(pos2(10.0, 10.0), vec2(48.0, 48.0));
        crate::icons::painted_shapes(ctx, id, square, Color32::RED)
    }

    #[test]
    fn the_fake_sheet_decodes_cell_by_cell() {
        let sheet = decode_sheet(&fake_sheet()).expect("a 512 × 1024 RGBA sheet");
        assert_eq!(sheet.size, [SHEET_W, SHEET_H]);
        // Play (3): a 5 px square at its cell's corner, then transparency.
        let at = |x: usize, y: usize| sheet.pixels[y * SHEET_W + x];
        let (cx, cy) = (3 * CELL, 0);
        assert_eq!(at(cx + 2, cy + 2), Color32::WHITE);
        assert_eq!(at(cx + 6, cy + 6), Color32::WHITE);
        assert_eq!(at(cx + 7, cy + 7), Color32::TRANSPARENT);
    }

    #[test]
    fn a_bad_sheet_is_refused_and_our_set_stays() {
        let small = png(256, 512, &vec![255; 256 * 512 * 4]);
        for bad in [&b"not a png"[..], &[], &small, &fake_sheet()[..100]] {
            assert!(decode_sheet(bad).is_none());
            let ctx = Context::default();
            assert!(!install_sheet(&ctx, bad));
            assert!(
                painted(&ctx, IconId::Play)
                    .iter()
                    .all(|s| !matches!(s, Shape::Mesh(_)))
            );
        }
    }

    #[test]
    fn an_installed_sheet_draws_each_icon_from_its_cell_but_never_a_trademark() {
        let ctx = Context::default();
        assert!(install_sheet(&ctx, &fake_sheet()));
        let shapes = painted(&ctx, IconId::Play);
        let [Shape::Mesh(mesh)] = &shapes[..] else {
            panic!("{shapes:?}")
        };
        let texture = ctx.data(|d| d.get_temp::<TextureId>(handle_id()));
        assert_eq!(Some(mesh.texture_id), texture);
        // Cell 3: column 3, row 0, half a texel in on each side; tinted as ours are.
        let (u, v) = (
            mesh.vertices.iter().map(|v| v.uv.x),
            mesh.vertices.iter().map(|v| v.uv.y),
        );
        let (u0, u1) = (u.clone().fold(1.0, f32::min), u.fold(0.0, f32::max));
        let (v0, v1) = (v.clone().fold(1.0, f32::min), v.fold(0.0, f32::max));
        let half = (0.5 / SHEET_W as f32, 0.5 / SHEET_H as f32);
        assert!(
            (u0 - (3.0 / 8.0 + half.0)).abs() < 1e-6 && (u1 - (4.0 / 8.0 - half.0)).abs() < 1e-6
        );
        assert!((v0 - half.1).abs() < 1e-6 && (v1 - (1.0 / 16.0 - half.1)).abs() < 1e-6);
        assert!(mesh.vertices.iter().all(|v| v.color == Color32::RED));
        // Row 9, column 7: CloudSync (78).
        let [Shape::Mesh(cloud)] = &painted(&ctx, IconId::CloudSync)[..] else {
            panic!()
        };
        let v0 = cloud.vertices.iter().map(|v| v.uv.y).fold(1.0, f32::min);
        assert!((v0 - (9.0 / 16.0 + half.1)).abs() < 1e-6, "{v0}");
        // The trademark IDs keep our own glyphs (6, 16, 17, 18 and 20), and every other ID draws from the sheet.
        for id in IconId::ALL {
            let from_sheet = painted(&ctx, id)
                .iter()
                .any(|s| matches!(s, Shape::Mesh(m) if m.texture_id == mesh.texture_id));
            assert_eq!(
                from_sheet,
                ![6, 16, 17, 18, 20].contains(&(id as u8)),
                "{id:?}"
            );
        }
        for n in [6, 16, 17, 18, 20] {
            let ours = painted(&ctx, IconId::ALL[n]);
            assert!(
                !ours.is_empty()
                    && ours
                        .iter()
                        .all(|s| !matches!(s, Shape::Mesh(m) if m.texture_id == mesh.texture_id)),
                "{n}"
            );
        }
        // Another context has no sheet.
        assert!(
            painted(&Context::default(), IconId::Play)
                .iter()
                .all(|s| !matches!(s, Shape::Mesh(_)))
        );
    }
}

//! The UI unit `U`, the UI-scale setting and typography sizes.
//!
//! All layout is expressed in `U`. `U` is defined in device pixels; egui lays out in points, so [`Metrics`] converts
//! once per frame and hands out point values.

use eframe::egui;

/// Factor for each UI-scale index (the preference stores the index, not the factor).
pub const UI_SCALE_FACTORS: [f32; 9] = [0.75, 1.0, 1.25, 1.5, 0.9, 1.1, 1.75, 2.0, 0.6];

/// UI-scale index used when nothing has been chosen (100 %).
pub const DEFAULT_UI_SCALE_INDEX: usize = 1;

/// Desktop and web screens sit farther from the eye than a phone, so the reported DPI is boosted
/// to keep `U` comfortable. Tune this later.
pub const DESKTOP_DPI_BOOST: f32 = 2.0;

/// Lower bound of the DPI that feeds `U`, so `U >= 16`.
const DPI_MIN: f32 = 96.0;

/// Upper bound of the DPI as a fraction of the drawing surface width in device pixels, so `U <= width / 20`.
const DPI_MAX_PER_WIDTH: f32 = 0.3;

/// egui's logical DPI at `pixels_per_point == 1.0`.
const BASE_DPI: f32 = 96.0;

/// Indices of [`UI_SCALE_FACTORS`] sorted by ascending factor, for a 60 %..200 % settings list.
pub fn ui_scale_menu_order() -> [usize; 9] {
    let mut order: [usize; 9] = std::array::from_fn(|i| i);
    order.sort_by(|&a, &b| UI_SCALE_FACTORS[a].total_cmp(&UI_SCALE_FACTORS[b]));
    order
}

/// `U` in device pixels: `round(clamp(dpi * factor, 96, 0.3 * surface_w_px) / 6)`.
/// The minimum is applied after the maximum, so 96 wins on a surface narrower than 320 px. Halves round up. An
/// out-of-range index means a factor of 1.0.
pub fn ui_unit_px(dpi: f32, scale_index: usize, surface_w_px: f32) -> f32 {
    let factor = UI_SCALE_FACTORS.get(scale_index).copied().unwrap_or(1.0);
    // Not `f32::clamp`: it panics when min > max, and here max < min is the narrow-surface case.
    ((dpi * factor)
        .min(DPI_MAX_PER_WIDTH * surface_w_px)
        .max(DPI_MIN)
        / 6.0)
        .round()
}

/// Text sizes, named by their point size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontSize {
    Pt6,
    Pt8,
    Pt10,
    Pt12,
    Pt18,
    Pt64,
}

impl FontSize {
    pub fn points(self) -> f32 {
        match self {
            Self::Pt6 => 6.0,
            Self::Pt8 => 8.0,
            Self::Pt10 => 10.0,
            Self::Pt12 => 12.0,
            Self::Pt18 => 18.0,
            Self::Pt64 => 64.0,
        }
    }
}

/// The UI unit and pixel density for one frame. The fields are private so that `U` stays a whole number of
/// device pixels: read them through the accessors.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// `U` in points.
    u: f32,
    /// `U` in device pixels: a whole number.
    u_px: f32,
    /// egui's `pixels_per_point`.
    ppp: f32,
}

impl Metrics {
    /// Builds the metrics for this frame; call once per frame, so a resize or rotation recomputes `U` (the
    /// DPI clamp follows the drawing surface, which is the whole viewport).
    pub fn new(ctx: &egui::Context, scale_index: usize) -> Self {
        let ppp = ctx.pixels_per_point();
        Self::on_surface(ppp, scale_index, ctx.viewport_rect().width() * ppp)
    }

    /// For a surface of unknown width: only the DPI minimum applies. Tests that fix `U` use this.
    pub fn from_pixels_per_point(ppp: f32, scale_index: usize) -> Self {
        Self::on_surface(ppp, scale_index, f32::INFINITY)
    }

    /// `surface_w_px` is the drawing surface width in device pixels.
    fn on_surface(ppp: f32, scale_index: usize, surface_w_px: f32) -> Self {
        let dpi = BASE_DPI * ppp * DESKTOP_DPI_BOOST;
        let u_px = ui_unit_px(dpi, scale_index, surface_w_px);
        Self {
            u: u_px / ppp,
            u_px,
            ppp,
        }
    }

    /// `n` UI units, in points (`u(1.0)` is `U` itself).
    pub fn u(&self, n: f32) -> f32 {
        n * self.u
    }

    /// `U` in device pixels, always a whole number. This is the unit of the layout engine and of the fonts.
    pub fn u_px(&self) -> f32 {
        self.u_px
    }

    /// Device pixels per point.
    pub fn ppp(&self) -> f32 {
        self.ppp
    }

    /// The DPI the app is given, in device pixels per inch: the one `U` is made from.
    pub fn dpi(&self) -> f32 {
        BASE_DPI * self.ppp * DESKTOP_DPI_BOOST
    }

    /// `v` points on the nearest whole device pixel.
    pub fn round_px(&self, v: f32) -> f32 {
        (v * self.ppp).round() / self.ppp
    }

    /// `v` points on the whole device pixel at or below it.
    pub fn floor_px(&self, v: f32) -> f32 {
        (v * self.ppp).floor() / self.ppp
    }

    /// The hairline unit `t` of the widgets, `ceil(U / 16)` device pixels, in points.
    pub fn t(&self) -> f32 {
        (self.u_px / 16.0).ceil() / self.ppp
    }

    /// Proportional font of the given size, in points (the pixel size is floored first).
    pub fn font_id(&self, size: FontSize) -> egui::FontId {
        self.font_id_pt(size.points())
    }

    /// [`Self::font_id`] for any point size, such as the knob's value text fitted from 12 pt down to 6 pt.
    pub fn font_id_pt(&self, pts: f32) -> egui::FontId {
        egui::FontId::proportional((self.u_px * pts / 16.0).floor() / self.ppp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A surface wide enough that the maximum never binds.
    const WIDE: f32 = 100_000.0;

    #[test]
    fn ui_unit_at_each_scale() {
        assert_eq!(ui_unit_px(510.0, 1, WIDE), 85.0);
        assert_eq!(ui_unit_px(510.0, 8, WIDE), 51.0);
        // Rounded, not truncated: 100 / 6 = 16.67.
        assert_eq!(ui_unit_px(100.0, 1, WIDE), 17.0);
    }

    #[test]
    fn out_of_range_scale_index_is_100_percent() {
        assert_eq!(ui_unit_px(510.0, 99, WIDE), 85.0);
    }

    #[test]
    fn narrow_surface_gives_the_minimum_dpi() {
        // 0.3 * 300 = 90 < 96: the minimum is applied last, so 96 wins and U = 16 at every scale.
        for index in 0..UI_SCALE_FACTORS.len() {
            assert_eq!(ui_unit_px(510.0, index, 300.0), 16.0, "index {index}");
        }
        assert_eq!(ui_unit_px(510.0, 1, 0.0), 16.0);
        // The same wins below 96 even without the maximum: a dpi of 60 is raised to 96.
        assert_eq!(ui_unit_px(60.0, 1, WIDE), 16.0);
    }

    #[test]
    fn wide_high_dpi_is_clamped_by_a_third_of_the_width() {
        // 0.3 * 1000 = 300 < 510: U = 50, not 85.
        assert_eq!(ui_unit_px(510.0, 1, 1000.0), 50.0);
        // The clamp is on dpi * factor: 510 * 0.6 = 306 is just above 300, 510 * 0.9 and up are too.
        assert_eq!(ui_unit_px(510.0, 8, 1000.0), 50.0);
        assert_eq!(ui_unit_px(510.0, 7, 1000.0), 50.0);
        // Below the maximum nothing changes: 0.3 * 2000 = 600.
        assert_eq!(ui_unit_px(510.0, 1, 2000.0), 85.0);
        // The same dpi on 960 px: 288 -> U = 48 (U is at most width / 20).
        assert_eq!(ui_unit_px(510.0, 7, 960.0), 48.0);
    }

    #[test]
    fn halves_round_up() {
        // 99 / 6 = 16.5 and 111 / 6 = 18.5 exactly.
        assert_eq!(ui_unit_px(99.0, 1, WIDE), 17.0);
        assert_eq!(ui_unit_px(111.0, 1, WIDE), 19.0);
        // 0.75 * 148 = 111 through a scale factor.
        assert_eq!(ui_unit_px(148.0, 0, WIDE), 19.0);
    }

    /// `Metrics::new` for a surface of `size` points at `ppp`.
    fn metrics_on(size: egui::Vec2, ppp: f32, index: usize) -> Metrics {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        raw.viewports
            .entry(raw.viewport_id)
            .or_default()
            .native_pixels_per_point = Some(ppp);
        let mut out = None;
        let _ = ctx.run(raw, |ctx| out = Some(Metrics::new(ctx, index)));
        out.unwrap()
    }

    #[test]
    fn metrics_new_follows_the_surface_width_in_device_pixels() {
        let v = egui::vec2;
        // Wide enough at ppp 1: dpi 192 < 0.3 * 1000.
        assert_eq!(metrics_on(v(1000.0, 600.0), 1.0, 1).u_px(), 32.0);
        // Narrow: 300 px wide, 0.3 * 300 = 90 -> 96 -> 16 px.
        assert_eq!(metrics_on(v(300.0, 600.0), 1.0, 1).u_px(), 16.0);
        // 100 % at ppp 2 is dpi 384 on 800 px (400 pt): clamped to 240 -> U = 40 px = 20 pt.
        let m = metrics_on(v(400.0, 300.0), 2.0, 1);
        assert_eq!((m.u_px(), m.u(1.0)), (40.0, 20.0));
        // Resizing the window recomputes it: the same ppp on 1600 px is not clamped (0.3 * 1600 = 480).
        assert_eq!(metrics_on(v(800.0, 300.0), 2.0, 1).u_px(), 64.0);
    }

    #[test]
    fn font_pixel_sizes_are_floor_u_times_points_over_16() {
        let at = |u_px: f32, ppp: f32| Metrics {
            u: u_px / ppp,
            u_px,
            ppp,
        };
        assert_eq!(at(80.0, 1.0).font_id(FontSize::Pt10).size, 50.0);
        assert_eq!(at(85.0, 1.0).font_id(FontSize::Pt8).size, 42.0);
        // In points: 42 px at 2 pixels per point.
        assert_eq!(at(85.0, 2.0).font_id(FontSize::Pt8).size, 21.0);
    }

    #[test]
    fn menu_order_is_ascending_percentages() {
        let factors: Vec<f32> = ui_scale_menu_order()
            .iter()
            .map(|&i| UI_SCALE_FACTORS[i])
            .collect();
        assert_eq!(factors, [0.6, 0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0]);
    }

    #[test]
    fn metrics_convert_pixels_to_points() {
        // ppp 2.0 -> dpi 384 -> U = 64 px = 32 pt; 10 pt text = floor(64 * 10 / 16) = 40 px = 20 pt.
        let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE_INDEX);
        assert_eq!(m.u(1.0), 32.0);
        assert_eq!(m.u(2.5), 80.0);
        let id = m.font_id(FontSize::Pt10);
        assert_eq!(id.size, 20.0);
        assert_eq!(id.family, egui::FontFamily::Proportional);
    }

    #[test]
    fn hairline_is_a_sixteenth_of_u_rounded_up_in_pixels() {
        // U = 64 px at ppp 2: t = 4 px = 2 points.
        let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE_INDEX);
        assert_eq!(m.t(), 2.0);
        // U = 85 px (panels-instruments 4.1 check): t = ceil(5.3125) = 6 px.
        let m = Metrics::from_pixels_per_point(85.0 / 32.0, DEFAULT_UI_SCALE_INDEX);
        assert_eq!(m.u_px(), 85.0);
        assert!((m.t() * m.ppp() - 6.0).abs() < 1e-4, "{}", m.t() * m.ppp());
    }

    #[test]
    fn any_point_size_follows_the_font_rule() {
        // floor(64 * 9 / 16) = 36 px = 18 points; the named sizes agree.
        let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE_INDEX);
        assert_eq!(m.font_id_pt(9.0).size, 18.0);
        assert_eq!(m.font_id_pt(12.0), m.font_id(FontSize::Pt12));
    }

    #[test]
    fn metrics_font_size_is_exact_for_fractional_ppp() {
        // ppp 1.5 -> dpi 288 -> U = 48 px = 32 pt; 12 pt text = 36 px = 24 pt.
        let m = Metrics::from_pixels_per_point(1.5, DEFAULT_UI_SCALE_INDEX);
        assert_eq!(m.font_id(FontSize::Pt12).size, 24.0);
    }

    #[test]
    fn accessors_report_whole_pixels_at_every_scale_and_density() {
        for ppp in [1.0, 1.25, 1.5, 2.0, 2.625, 3.0] {
            for index in 0..UI_SCALE_FACTORS.len() {
                let m = Metrics::from_pixels_per_point(ppp, index);
                assert_eq!(m.ppp(), ppp);
                let px = m.u_px();
                assert!(px >= 1.0 && px == px.round(), "{px} px at ppp {ppp}");
                // Points and pixels describe the same unit.
                assert!((m.u(1.0) * m.ppp() - px).abs() < 1e-3, "ppp {ppp}");
                // The hairline and the font sizes are whole pixels too.
                for len in [m.t(), m.font_id(FontSize::Pt10).size] {
                    let len_px = len * m.ppp();
                    assert!(
                        (len_px - len_px.round()).abs() < 1e-3,
                        "{len_px} at ppp {ppp}"
                    );
                }
            }
        }
        let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE_INDEX);
        assert_eq!((m.u_px(), m.u(1.0), m.ppp()), (64.0, 32.0, 2.0));
    }
}

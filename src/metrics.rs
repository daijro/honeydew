//! The UI unit `U`, the UI-scale setting and typography sizes.
//!
//! All layout is expressed in `U`. `U` is defined in device pixels; egui lays out in points, so [`Metrics`] converts
//! once per frame and hands out point values.

use eframe::egui;

/// The legacy `uiscale` setting's factor for each index, read once to migrate it to a percent.
pub const UI_SCALE_FACTORS: [f32; 9] = [0.75, 1.0, 1.25, 1.5, 0.9, 1.1, 1.75, 2.0, 0.6];

/// `Scaling:` in percent when nothing has been chosen, and the range it is kept in.
pub const DEFAULT_UI_SCALE: u32 = 100;
pub const UI_SCALE_RANGE: std::ops::RangeInclusive<u32> = 60..=200;

/// Desktop and web screens sit farther from the eye than a phone, so the reported DPI is boosted
/// to keep `U` comfortable. Tune this later.
pub const DESKTOP_DPI_BOOST: f32 = 2.0;

/// A scale above 100 % grows `U` up to this fraction of the window width, no further.
const MAX_U_PER_WIDTH: f32 = 0.05;

/// egui's logical DPI at `pixels_per_point == 1.0`.
const BASE_DPI: f32 = 96.0;

/// Whether the window `screen` is portrait: taller than wide. Then the rack (in its full screen mode), the main menu
/// and the side pickers take the whole window width, even on a window 20 U or wider.
pub fn is_portrait(screen: egui::Rect) -> bool {
    screen.height() > screen.width()
}

/// `U` in device pixels at `percent` (kept in [`UI_SCALE_RANGE`]) on a window `surface_w_px` wide, and the percent
/// the scale is limited to when the width caps it. base = dpi / 6 and U = round(base × factor), capped at
/// max(floor(0.05 × width), round(base)): 100 % is never reduced by a narrow window, and a larger scale only grows
/// `U` to 5 % of the width. Halves round up.
pub fn ui_unit_px(dpi: f32, percent: u32, surface_w_px: f32) -> (f32, Option<u32>) {
    let factor = percent.clamp(*UI_SCALE_RANGE.start(), *UI_SCALE_RANGE.end()) as f32 / 100.0;
    let base = dpi / 6.0;
    let want = (dpi * factor / 6.0).round().max(1.0);
    // Whole pixels: `U` is one, and 100 % (round(base)) always fits.
    let cap = (MAX_U_PER_WIDTH * surface_w_px)
        .floor()
        .max(base.round())
        .max(1.0);
    if want <= cap {
        (want, None)
    } else {
        (cap, Some((100.0 * cap / base).round() as u32))
    }
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
    /// The density used for this frame's layout and physical knob travel.
    dpi: f32,
    /// The percent the UI scale is limited to by the window width, if it is.
    limited: Option<u32>,
}

impl Metrics {
    /// Builds the metrics for this frame at the UI scale `percent`; call once per frame, so a resize or rotation
    /// recomputes `U` (the width cap follows the drawing surface, which is the whole viewport).
    pub fn new(ctx: &egui::Context, percent: u32) -> Self {
        let ppp = ctx.pixels_per_point();
        Self::on_surface(ppp, percent, ctx.viewport_rect().width() * ppp)
    }

    /// For a surface of unknown width: no cap applies. Tests that fix `U` use this.
    pub fn from_pixels_per_point(ppp: f32, percent: u32) -> Self {
        Self::on_surface(ppp, percent, f32::INFINITY)
    }

    /// `surface_w_px` is the drawing surface width in device pixels.
    fn on_surface(ppp: f32, percent: u32, surface_w_px: f32) -> Self {
        #[cfg(target_os = "android")]
        let display_dpi = Some(crate::android::display_dpi());
        #[cfg(not(target_os = "android"))]
        let display_dpi = None;
        Self::from_display_dpi(ppp, percent, surface_w_px, display_dpi)
    }

    /// A frame's reported physical display density, if the platform supplies one.
    pub(crate) fn from_display_dpi(
        ppp: f32,
        percent: u32,
        surface_w_px: f32,
        display_dpi: Option<f32>,
    ) -> Self {
        // Android's logical density is not its physical xdpi; only the fallback uses the desktop estimate.
        let dpi = display_dpi
            .filter(|dpi| dpi.is_finite() && *dpi > 0.0)
            .unwrap_or(BASE_DPI * ppp * DESKTOP_DPI_BOOST);
        let (u_px, limited) = ui_unit_px(dpi, percent, surface_w_px);
        Self {
            u: u_px / ppp,
            u_px,
            ppp,
            dpi,
            limited,
        }
    }

    /// The percent the window width limits the UI scale to, while it does (SETTINGS shows it).
    pub fn scale_limited_to(&self) -> Option<u32> {
        self.limited
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
        self.dpi
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
    fn a_narrow_window_never_reduces_100_percent_and_caps_larger_scales_at_5_percent_of_its_width()
    {
        // U = 85 at 100 %. Landscape 3120 px: the cap is 156 px, so 200 % is limited to 183 %.
        assert_eq!(ui_unit_px(510.0, 100, 3120.0), (85.0, None));
        assert_eq!(ui_unit_px(510.0, 150, 3120.0), (128.0, None));
        assert_eq!(ui_unit_px(510.0, 200, 3120.0), (156.0, Some(184)));
        // Portrait 1440 px: the cap is U itself, so every scale above 100 % is limited to 100 %.
        assert_eq!(ui_unit_px(510.0, 100, 1440.0), (85.0, None));
        assert_eq!(ui_unit_px(510.0, 125, 1440.0), (85.0, Some(100)));
        // Below 100 % is never limited, and a percent outside 60–200 is kept in it.
        assert_eq!(ui_unit_px(510.0, 60, 300.0), (51.0, None));
        assert_eq!(ui_unit_px(510.0, 10, WIDE), (51.0, None));
        // Halves round up: 0.75 * 148 / 6 = 18.5.
        assert_eq!(ui_unit_px(148.0, 75, WIDE), (19.0, None));
    }

    /// `Metrics::new` for a surface of `size` points at `ppp`.
    fn metrics_on(size: egui::Vec2, ppp: f32, percent: u32) -> Metrics {
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
        let _ = ctx.run(raw, |ctx| out = Some(Metrics::new(ctx, percent)));
        out.unwrap()
    }

    #[test]
    fn metrics_new_follows_the_surface_width_in_device_pixels() {
        let v = egui::vec2;
        // dpi 192 at ppp 1: U = 32 px, on a wide or a portrait window.
        assert_eq!(metrics_on(v(1000.0, 600.0), 1.0, 100).u_px(), 32.0);
        assert_eq!(metrics_on(v(300.0, 600.0), 1.0, 100).u_px(), 32.0);
        // 200 % at ppp 2 (base 64 px) on 800 px: 5 % is 40 px, under 100 %, so U is 64 px and the scale 100 %.
        let m = metrics_on(v(400.0, 300.0), 2.0, 200);
        assert_eq!(
            (m.u_px(), m.u(1.0), m.scale_limited_to()),
            (64.0, 32.0, Some(100))
        );
        // Resizing the window recomputes it: on 3000 px the cap is 150 px.
        let m = metrics_on(v(1500.0, 300.0), 2.0, 200);
        assert_eq!((m.u_px(), m.scale_limited_to()), (128.0, None));
    }

    #[test]
    fn physical_dpi_keeps_manual_scale_and_rejects_invalid_reports() {
        let (ppp, dpi) = (2.625, 384.0);
        for percent in [60, 100, 145, 200] {
            let m = Metrics::from_display_dpi(ppp, percent, WIDE, Some(dpi));
            assert_eq!(m.u_px(), ui_unit_px(dpi, percent, WIDE).0);
            assert_eq!(m.dpi(), dpi);
            assert_eq!(m.ppp(), ppp);
        }
        for dpi in [
            None,
            Some(0.0),
            Some(-1.0),
            Some(f32::NAN),
            Some(f32::INFINITY),
        ] {
            let m = Metrics::from_display_dpi(ppp, 100, WIDE, dpi);
            assert_eq!(m.dpi(), 504.0);
            assert_eq!(m.u_px(), 84.0);
        }
    }

    #[test]
    fn font_pixel_sizes_are_floor_u_times_points_over_16() {
        let at = |u_px: f32, ppp: f32| Metrics {
            u: u_px / ppp,
            u_px,
            ppp,
            dpi: u_px * 6.0,
            limited: None,
        };
        assert_eq!(at(80.0, 1.0).font_id(FontSize::Pt10).size, 50.0);
        assert_eq!(at(85.0, 1.0).font_id(FontSize::Pt8).size, 42.0);
        // In points: 42 px at 2 pixels per point.
        assert_eq!(at(85.0, 2.0).font_id(FontSize::Pt8).size, 21.0);
    }

    #[test]
    fn metrics_convert_pixels_to_points() {
        // ppp 2.0 -> dpi 384 -> U = 64 px = 32 pt; 10 pt text = floor(64 * 10 / 16) = 40 px = 20 pt.
        let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE);
        assert_eq!(m.u(1.0), 32.0);
        assert_eq!(m.u(2.5), 80.0);
        let id = m.font_id(FontSize::Pt10);
        assert_eq!(id.size, 20.0);
        assert_eq!(id.family, egui::FontFamily::Proportional);
    }

    #[test]
    fn hairline_is_a_sixteenth_of_u_rounded_up_in_pixels() {
        // U = 64 px at ppp 2: t = 4 px = 2 points.
        let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE);
        assert_eq!(m.t(), 2.0);
        // U = 85 px (panels-instruments 4.1 check): t = ceil(5.3125) = 6 px.
        let m = Metrics::from_pixels_per_point(85.0 / 32.0, DEFAULT_UI_SCALE);
        assert_eq!(m.u_px(), 85.0);
        assert!((m.t() * m.ppp() - 6.0).abs() < 1e-4, "{}", m.t() * m.ppp());
    }

    #[test]
    fn any_point_size_follows_the_font_rule() {
        // floor(64 * 9 / 16) = 36 px = 18 points; the named sizes agree.
        let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE);
        assert_eq!(m.font_id_pt(9.0).size, 18.0);
        assert_eq!(m.font_id_pt(12.0), m.font_id(FontSize::Pt12));
    }

    #[test]
    fn metrics_font_size_is_exact_for_fractional_ppp() {
        // ppp 1.5 -> dpi 288 -> U = 48 px = 32 pt; 12 pt text = 36 px = 24 pt.
        let m = Metrics::from_pixels_per_point(1.5, DEFAULT_UI_SCALE);
        assert_eq!(m.font_id(FontSize::Pt12).size, 24.0);
    }

    #[test]
    fn accessors_report_whole_pixels_at_every_scale_and_density() {
        for ppp in [1.0, 1.25, 1.5, 2.0, 2.625, 3.0] {
            for percent in [60, 75, 90, 100, 110, 125, 150, 175, 200] {
                let m = Metrics::from_pixels_per_point(ppp, percent);
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
        let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE);
        assert_eq!((m.u_px(), m.u(1.0), m.ppp()), (64.0, 32.0, 2.0));
    }
}

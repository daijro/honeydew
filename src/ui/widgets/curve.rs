//! The curve plot and the envelope curves: the graph knob's contents, and the Tape Stop warp display's later on.

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

/// A(x) = 0.001^(1 − x): exponential, slow start.
fn exponential(x: f32) -> f32 {
    0.001f32.powf(1.0 - x)
}

/// B(x) = 1 + ln x / ln 1000: logarithmic, fast start. B(0) is taken as 0.
fn logarithmic(x: f32) -> f32 {
    if x > 0.0 {
        1.0 + x.ln() / 1000f32.ln()
    } else {
        0.0
    }
}

/// The envelope curve Env(x, c) for c in [−1, 1], through (0, 0) and (1, 1): a straight line at c = 0, A(x) at
/// c = +1 and B(x) at c = −1, and in between the straight line moved that far towards A or B.
///
/// shortcut: A and B use formulas instead of 1024-entry Catmull-Rom tables, use tables if subpixel accuracy is needed.
pub fn env(x: f32, c: f32) -> f32 {
    if c > 0.0 {
        x + c * (exponential(x) - x)
    } else {
        x - c * (logarithmic(x) - x)
    }
}

/// The curve plot: `f(t)` for t in [0, 1] as a polyline `lw` points wide across `rect`, `f` clamped to [0, 1] (NaN
/// as 0) with 1 at the top, the line kept inside the rect. One sample per device pixel of width: N = int(w − lw) in
/// pixels, so nothing is drawn on a rect less than two pixels wider than the line.
pub fn curve_plot(
    painter: &Painter,
    rect: Rect,
    lw: f32,
    ppp: f32,
    color: Color32,
    f: impl Fn(f32) -> f32,
) {
    let points = curve_points(rect, lw, ppp, f);
    if points.len() >= 2 {
        painter.add(Shape::line(points, Stroke::new(lw, color)));
    }
}

/// The samples of [`curve_plot`].
fn curve_points(rect: Rect, lw: f32, ppp: f32, f: impl Fn(f32) -> f32) -> Vec<Pos2> {
    // `as` truncates like int() and turns a negative or NaN width into 0.
    let n = ((rect.width() - lw) * ppp) as usize;
    if n < 2 {
        return Vec::new();
    }
    let (w, h) = (rect.width() - lw, rect.height() - lw);
    (0..n)
        .map(|i| {
            let t = i as f32 / (n - 1) as f32;
            let y = f(t);
            let y = if y.is_nan() { 0.0 } else { y.clamp(0.0, 1.0) };
            pos2(
                rect.left() + lw / 2.0 + t * w,
                rect.bottom() - lw / 2.0 - h * y,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn a_and_b_run_from_0_to_1() {
        assert!(near(exponential(0.0), 0.001) && near(exponential(1.0), 1.0));
        assert!(near(exponential(0.5), 0.001f32.sqrt()));
        assert_eq!(logarithmic(0.0), 0.0);
        assert!(near(logarithmic(1.0), 1.0));
        // ln(0.1) / ln(1000) = −1/3.
        assert!(near(logarithmic(0.1), 2.0 / 3.0));
    }

    #[test]
    fn env_blends_the_line_towards_a_or_b() {
        for x in [0.0, 0.1, 0.5, 0.9, 1.0] {
            assert!(near(env(x, 0.0), x), "{x}");
            assert!(near(env(x, 1.0), exponential(x)), "{x}");
            assert!(near(env(x, -1.0), logarithmic(x)), "{x}");
            assert!(near(env(x, 0.5), (x + exponential(x)) / 2.0), "{x}");
            assert!(near(env(x, -0.25), x + 0.25 * (logarithmic(x) - x)), "{x}");
        }
        // Every curve of the family ends at (1, 1); c ≤ 0 starts at (0, 0).
        for c in [-1.0, -0.3, 0.0, 0.4, 1.0] {
            assert!(near(env(1.0, c), 1.0), "{c}");
        }
        assert_eq!(env(0.0, -1.0), 0.0);
    }

    #[test]
    fn the_plot_has_one_sample_per_pixel_inside_the_line_width() {
        // 40 points wide at ppp 2 with a 2-point (4 px) line: int(80 − 4) = 76 samples.
        let rect = Rect::from_min_max(pos2(10.0, 20.0), pos2(50.0, 60.0));
        let pts = curve_points(rect, 2.0, 2.0, |t| t);
        assert_eq!(pts.len(), 76);
        // The first sample sits lw/2 in from the bottom left, the last lw/2 in from the top right.
        assert_eq!(pts[0], pos2(11.0, 59.0));
        assert!(
            near(pts[75].x, 49.0) && near(pts[75].y, 21.0),
            "{:?}",
            pts[75]
        );
        // F is clamped to [0, 1].
        let pts = curve_points(rect, 2.0, 2.0, |t| 3.0 * t - 1.0);
        assert!(pts.iter().all(|p| (21.0..=59.0).contains(&p.y)));
        assert_eq!(pts[0].y, 59.0);
        assert_eq!(pts[75].y, 21.0);
    }

    #[test]
    fn a_nan_sample_plots_as_0() {
        let rect = Rect::from_min_max(pos2(10.0, 20.0), pos2(50.0, 60.0));
        let pts = curve_points(rect, 2.0, 2.0, |_| f32::NAN);
        assert!(pts.iter().all(|p| p.y == 59.0), "{pts:?}");
    }

    #[test]
    fn a_plot_narrower_than_two_pixels_past_its_line_has_no_samples() {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(3.0, 3.0));
        assert!(curve_points(rect, 2.0, 1.0, |t| t).is_empty());
        assert!(curve_points(Rect::ZERO, 2.0, 1.0, |t| t).is_empty());
        assert!(curve_points(Rect::NOTHING, 2.0, 1.0, |t| t).is_empty());
    }
}

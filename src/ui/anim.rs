//! The opening animations of menus and panels. Every law steps once per drawn frame from the frame the menu or panel
//! is first shown; none blocks input, and every close is instant. A thing not drawn for a frame opens from the start
//! again.
//!
//! Tests elsewhere see everything fully open at once: [`frame`] reports a finished animation unless the context turned
//! them on ([`enable`]), so a test that reads a menu's items needs no 23 frames first (the laws' own tests turn them
//! on).

use eframe::egui::{Color32, Context, Id};

/// The screen behind a menu or panel: `background` at up to this alpha.
pub const DIM_ALPHA: f32 = 0.6;

/// A frame count past every animation's end.
const DONE: u32 = 1000;

/// Which drawn frame of `id`'s opening this is, from 1: the count restarts when `id` was not drawn the frame before,
/// and stays put for a second call in one frame. Past the end (or with the animations off) it is large.
pub fn frame(ctx: &Context, id: Id) -> u32 {
    if !enabled(ctx) {
        return DONE;
    }
    let now = ctx.cumulative_frame_nr();
    let key = id.with("opening");
    let n = match ctx.data(|d| d.get_temp::<(u32, u64)>(key)) {
        Some((n, last)) if last == now => n,
        Some((n, last)) if last + 1 == now => (n + 1).min(DONE),
        _ => 1,
    };
    ctx.data_mut(|d| d.insert_temp(key, (n, now)));
    n
}

/// Whether the animations run on `ctx`: always in the app, only once [`enable`]d in tests.
fn enabled(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(Id::new("animations")))
        .unwrap_or(!cfg!(test))
}

/// Turns the animations on for a test's context.
#[cfg(test)]
pub fn enable(ctx: &Context) {
    ctx.data_mut(|d| d.insert_temp(Id::new("animations"), true));
}

/// The dim over the screen, `background` at `alpha`.
pub fn dim(background: Color32, alpha: f32) -> Color32 {
    background.gamma_multiply(alpha)
}

/// The popup lists' sweep at drawn frame `n` (phase 1 the bar, phase 2 the fill): (phase, a), each phase from a = 1
/// with a ← 0.4·a after each drawn frame, ending once a < 0.0001 (11 frames); phase 3 is done (the items show, from
/// frame 23).
pub fn sweep(n: u32) -> (u8, f32) {
    let (mut phase, mut a) = (1, 1.0f32);
    for _ in 1..n.min(DONE) {
        a *= 0.4;
        if a < 0.0001 {
            (phase, a) = (phase + 1, 1.0);
        }
        if phase == 3 {
            break;
        }
    }
    (phase, a)
}

/// The main menu's slide at drawn frame `n`: u, from 1 down 0.1 a frame, snapping to 0 below 0.0001 (0 from frame
/// 10). The contents are shifted right by trunc(u⁴·W) and the dim is α 0.6·(1 − u⁴).
pub fn menu_slide(n: u32) -> f32 {
    let mut u = 1.0f32;
    for _ in 0..n.min(DONE) {
        u -= 0.1;
        if u < 0.0001 {
            return 0.0;
        }
    }
    u
}

/// The side browser's slide at drawn frame `n`: p, from 0 with p ← (p + 1)/2 a frame, set to 1 once the gap is below
/// 0.001 (1 from frame 10). The contents are drawn at x + (p − 1)·W and the dim is α 0.6·p.
pub fn browser_slide(n: u32) -> f32 {
    let mut p = 0.0f32;
    for _ in 0..n.min(DONE) {
        p = (p + 1.0) / 2.0;
        if 1.0 - p < 0.001 {
            return 1.0;
        }
    }
    p
}

/// A message box's opening at drawn frame `n`: a, from 1 with a ← 0.4·a at the start of each frame, 0 below 0.0001
/// (0 from frame 11). The box is (1 − a) of its height, the dim α 0.6·(1 − a); text and buttons show at a = 0.
pub fn message_open(n: u32) -> f32 {
    let mut a = 1.0f32;
    for _ in 0..n.min(DONE) {
        a *= 0.4;
        if a < 0.0001 {
            return 0.0;
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sweep_runs_11_frames_a_phase_and_the_items_show_on_frame_23() {
        assert_eq!(sweep(1), (1, 1.0));
        assert_eq!(sweep(2), (1, 0.4));
        assert!((sweep(11).1 - 0.4f32.powi(10)).abs() < 1e-9 && sweep(11).0 == 1);
        assert_eq!(sweep(12), (2, 1.0));
        assert_eq!(sweep(22).0, 2);
        assert_eq!(sweep(23).0, 3);
        assert_eq!(sweep(DONE).0, 3);
    }

    #[test]
    fn the_menu_and_browser_slides_and_the_message_take_10_10_and_11_frames() {
        let menu: Vec<f32> = (1..=11).map(menu_slide).collect();
        assert!(
            (menu[0] - 0.9).abs() < 1e-6 && (menu[8] - 0.1).abs() < 1e-6,
            "{menu:?}"
        );
        assert_eq!((menu[9], menu[10]), (0.0, 0.0));
        let browser: Vec<f32> = (1..=11).map(browser_slide).collect();
        assert_eq!(&browser[..3], [0.5, 0.75, 0.875]);
        assert!(browser[8] < 1.0);
        assert_eq!((browser[9], browser[10]), (1.0, 1.0));
        let message: Vec<f32> = (1..=12).map(message_open).collect();
        assert_eq!(message[0], 0.4);
        assert!(message[9] > 0.0);
        assert_eq!((message[10], message[11]), (0.0, 0.0));
    }

    #[test]
    fn the_count_restarts_after_a_frame_unshown_and_is_done_at_once_unless_enabled() {
        let ctx = Context::default();
        let id = Id::new("thing");
        let run = |show: bool| {
            let mut n = 0;
            let _ = ctx.run(Default::default(), |ctx| {
                if show {
                    n = frame(ctx, id);
                    assert_eq!(frame(ctx, id), n, "twice in a frame");
                }
            });
            n
        };
        assert_eq!(run(true), DONE);
        enable(&ctx);
        assert_eq!([run(true), run(true), run(true)], [1, 2, 3]);
        run(false);
        assert_eq!(run(true), 1);
    }
}

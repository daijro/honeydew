//! The UI's animations. Every law runs on frame time, exactly the old per-frame law at 60 fps: an ease is
//! x ← x + (target − x)·(1 − e^(−k·Δt)) ([`step`]), and an opening runs on the time it has been shown ([`elapsed`]).
//! Each rate k is the per-frame ratio r at 60 fps, k = −60·ln r (the panels' r = 0.5 is k ≈ 41.6).
//! None blocks input, and every close is instant. A thing not drawn for a frame opens from the start again.
//!
//! SETTINGS `Animation` ([`set_mode`]): `Reduced` makes every animation jump to its end, `Minimal` also stops scroll
//! inertia ([`inertia`]).
//!
//! Tests elsewhere see every opening finished at once: [`elapsed`] reports a finished animation unless the context
//! turned them on ([`enable`]), so a test that reads a menu's items needs no 21 frames first (the laws' own tests turn
//! them on).

use eframe::egui::{Color32, Context, Id};

/// The screen behind a menu or panel: `background` at up to this alpha.
pub const DIM_ALPHA: f32 = 0.6;

/// The panel eases (the rack's slide, the playlist / editor split, the keyboard panel's height) keep this much of the
/// way left each 1/60 s.
pub const PANEL_KEEP: f32 = 0.5;

/// The main menu's u falls this much a second (0.1 a frame at 60 fps).
const MENU_RATE: f32 = 6.0;

/// The message box's opening and the popup sweep's phases keep this much each 1/60 s.
const OPEN_KEEP: f32 = 0.4;

/// A time past every animation's end, in seconds.
const DONE: f32 = 1000.0;

/// Seconds `id`'s opening has run: this frame's time step on the frame it first shows, then each drawn frame's added.
/// It restarts when `id` was not drawn the frame before, and stays put for a second call in one frame. Past the end
/// (or with the animations off) it is large.
pub fn elapsed(ctx: &Context, id: Id) -> f32 {
    if !enabled(ctx) {
        return DONE;
    }
    let now = ctx.cumulative_frame_nr();
    let dt = frame_dt(ctx);
    let key = id.with("opening");
    let t = match ctx.data(|d| d.get_temp::<(f32, u64)>(key)) {
        Some((t, last)) if last == now => t,
        Some((t, last)) if last + 1 == now => (t + dt).min(DONE),
        _ => dt,
    };
    ctx.data_mut(|d| d.insert_temp(key, (t, now)));
    t
}

/// The share of the way an ease that keeps `keep` of it each 1/60 s moves this frame, 1 − keep^(60·Δt); all of it with
/// `Animation` reduced.
pub fn step(ctx: &Context, keep: f32) -> f32 {
    if reduced(ctx) {
        1.0
    } else {
        1.0 - keep.powf(60.0 * frame_dt(ctx))
    }
}

/// What is left at `t` seconds of a decay keeping `keep` each 1/60 s.
fn left(keep: f32, t: f32) -> f32 {
    keep.powf(60.0 * t)
}

/// This frame's time step in seconds, at most 0.1 s: a stalled frame is one short step. Tests run at 60 fps whatever
/// time their input reports (their rigs step the clock in tenths, for gestures).
fn frame_dt(ctx: &Context) -> f32 {
    if cfg!(test) {
        1.0 / 60.0
    } else {
        ctx.input(|i| i.stable_dt).clamp(0.0, 0.1)
    }
}

/// SETTINGS `Animation`, applied to `ctx`: `animations` off (`Reduced`) jumps every animation to its end, and
/// `inertia` off (`Minimal`) also stops scrolls at once when let go.
pub fn set_mode(ctx: &Context, animations: bool, inertia: bool) {
    ctx.data_mut(|d| {
        d.insert_temp(Id::new("animations_reduced"), !animations);
        d.insert_temp(Id::new("inertia_off"), !inertia);
    });
}

/// `Animation` is `Reduced` or `Minimal`.
fn reduced(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(Id::new("animations_reduced")))
        .unwrap_or(false)
}

/// Scrolls coast after a fling: `Animation` is not `Minimal`.
pub fn inertia(ctx: &Context) -> bool {
    !(ctx.data(|d| d.get_temp::<bool>(Id::new("inertia_off")))).unwrap_or(false)
}

/// Whether the openings run on `ctx`: in the app unless `Animation` is reduced, in tests only once [`enable`]d.
pub(crate) fn enabled(ctx: &Context) -> bool {
    !reduced(ctx)
        && ctx
            .data(|d| d.get_temp::<bool>(Id::new("animations")))
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

/// The popup lists' sweep `t` seconds in (phase 1 the bar, phase 2 the fill): (phase, a), each phase from a = 1 with
/// a ← 0.4·a each 1/60 s, ending once a < 0.0001 (10 frames at 60 fps); phase 3 is done (the items show).
pub fn sweep(t: f32) -> (u8, f32) {
    let phase_len = 1e4f32.ln() / (1.0 / OPEN_KEEP).ln() / 60.0;
    match (t / phase_len).floor() {
        n if n >= 2.0 => (3, 0.0),
        n => (n as u8 + 1, left(OPEN_KEEP, t - n * phase_len)),
    }
}

/// The main menu's slide `t` seconds in: u, from 1 down 6 a second, 0 below 0.0001 (from frame 10 at 60 fps). The
/// contents are shifted right by trunc(u⁴·W) and the dim is α 0.6·(1 − u⁴).
pub fn menu_slide(t: f32) -> f32 {
    let u = 1.0 - MENU_RATE * t;
    if u < 0.0001 { 0.0 } else { u }
}

/// The side browser's slide `t` seconds in: p, from 0 halving its gap to 1 each 1/60 s, 1 once the gap is below
/// 0.001 (from frame 10 at 60 fps). The contents are drawn at x + (p − 1)·W and the dim is α 0.6·p.
pub fn browser_slide(t: f32) -> f32 {
    let gap = left(PANEL_KEEP, t);
    if gap < 0.001 { 1.0 } else { 1.0 - gap }
}

/// A message box's opening `t` seconds in: a, from 1 with a ← 0.4·a each 1/60 s, 0 below 0.0001 (from frame 11 at
/// 60 fps). The box is (1 − a) of its height, the dim α 0.6·(1 − a); text and buttons show at a = 0.
pub fn message_open(t: f32) -> f32 {
    let a = left(OPEN_KEEP, t);
    if a < 0.0001 { 0.0 } else { a }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The time `n` frames in at 60 fps.
    fn at(n: u32) -> f32 {
        n as f32 / 60.0
    }

    #[test]
    fn at_60_fps_the_laws_keep_their_per_frame_feel() {
        // The sweep: 0.4 a frame, phase 2 from frame 11, the items from frame 21.
        assert_eq!(sweep(0.0), (1, 1.0));
        assert!((sweep(at(1)).1 - 0.4).abs() < 1e-5);
        assert_eq!((sweep(at(10)).0, sweep(at(11)).0), (1, 2));
        assert_eq!((sweep(at(20)).0, sweep(at(21)).0), (2, 3));
        // The menu: 0.1 a frame, 0 from frame 10; the browser: half the gap a frame, 1 from frame 10.
        assert!((menu_slide(at(1)) - 0.9).abs() < 1e-5);
        assert_eq!((menu_slide(at(9)) > 0.0, menu_slide(at(10))), (true, 0.0));
        assert!((browser_slide(at(1)) - 0.5).abs() < 1e-5);
        assert_eq!(
            (browser_slide(at(9)) < 1.0, browser_slide(at(10))),
            (true, 1.0)
        );
        // The message box: 0.4 a frame, 0 from frame 11.
        assert!((message_open(at(1)) - 0.4).abs() < 1e-5);
        assert_eq!(
            (message_open(at(10)) > 0.0, message_open(at(11))),
            (true, 0.0)
        );
    }

    #[test]
    fn the_time_restarts_after_a_frame_unshown_and_reduced_animations_are_done_at_once() {
        let ctx = Context::default();
        let id = Id::new("thing");
        let run = |show: bool| {
            let mut t = 0.0;
            let _ = ctx.run(Default::default(), |ctx| {
                if show {
                    t = elapsed(ctx, id);
                    assert_eq!(elapsed(ctx, id), t, "twice in a frame");
                }
            });
            (t * 60.0).round()
        };
        assert_eq!(run(true), DONE * 60.0);
        enable(&ctx);
        assert_eq!([run(true), run(true), run(true)], [1.0, 2.0, 3.0]);
        run(false);
        assert_eq!(run(true), 1.0);
        // `Reduced`: openings are done, and an ease goes all the way; `Minimal` stops inertia too.
        set_mode(&ctx, false, true);
        assert_eq!(run(true), DONE * 60.0);
        assert_eq!(step(&ctx, PANEL_KEEP), 1.0);
        assert!(inertia(&ctx));
        set_mode(&ctx, false, false);
        assert!(!inertia(&ctx));
        set_mode(&ctx, true, true);
        assert_eq!(step(&ctx, PANEL_KEEP), 0.5);
    }
}

//! The step grid's ruler beyond the editors' shared one ([`crate::ui::timeline::ruler`]): the playhead folded into the
//! loop. The step grid's ruler maps song ticks: content beat 0 sits at song beat s, so its bar numbers are song bars.

use crate::song::PPQ;

/// Ticks per beat as `f64`.
pub const BEAT: f64 = PPQ as f64;

/// The song tick `pos` folded into the clip whose content tick 0 plays at song tick `origin`, as a content tick in
/// [0, `loop_ticks`) (s + ((pos − s) mod L), negative values wrapped); 0 without a loop.
pub fn folded(pos: u64, origin: i64, loop_ticks: f64) -> f64 {
    if loop_ticks > 0.0 {
        (pos as f64 - origin as f64).rem_euclid(loop_ticks)
    } else {
        0.0
    }
}

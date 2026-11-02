//! The pickers' sample preview (the Sample strip): one sample played once from a frame, at gain 0.7, centred, at its
//! own pitch (rate 1), no loop. It goes out past the master and its meter, as the click does, so no channel, pad or
//! effect applies (master effects neither).

use std::sync::Arc;

use honeydew_audio::sample::SampleData;

/// The preview's gain.
pub const GAIN: f32 = 0.7;

pub(super) struct Preview {
    data: Arc<SampleData>,
    /// Where it is in the sample's frames, and how far it moves an output frame.
    pos: f64,
    step: f64,
}

impl Preview {
    /// `data` from frame `from`, at output rate `rate`.
    pub fn new(data: Arc<SampleData>, from: usize, rate: f32) -> Self {
        let step = f64::from(data.sample_rate) / f64::from(rate);
        Self {
            data,
            pos: from as f64,
            step,
        }
    }

    /// Adds its next frames into `left` and `right` (linear between frames); false once it is past the end.
    pub fn add(&mut self, left: &mut [f32], right: &mut [f32]) -> bool {
        let (l, r) = (&*self.data.left, self.data.right.as_deref());
        for (out_l, out_r) in left.iter_mut().zip(right.iter_mut()) {
            let i = self.pos as usize;
            if i >= l.len() || !self.step.is_finite() || self.step <= 0.0 {
                return false;
            }
            let t = (self.pos - i as f64) as f32;
            let at = |s: &[f32]| s[i] + (s.get(i + 1).copied().unwrap_or(0.0) - s[i]) * t;
            let a = at(l);
            let b = r.map_or(a, at);
            *out_l += GAIN * a;
            *out_r += GAIN * b;
            self.pos += self.step;
        }
        (self.pos as usize) < l.len()
    }
}

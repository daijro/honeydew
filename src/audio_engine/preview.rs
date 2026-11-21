//! The pickers' sample preview (the Sample strip): one sample played once from a frame, at gain 0.7, centred, at its
//! own pitch (rate 1), no loop. It goes out past the master and its meter, as the click does, so no channel, pad or
//! effect applies (master effects neither).
//!
//! A big file's preview starts on the part decoded so far and goes on in each longer part as it comes, from where it
//! is: a part is the whole's start, frame for frame, so the change is seamless. While more comes its end waits,
//! silent, for the next part rather than ending (issues #131, #153).

use std::sync::Arc;

use honeydew_audio::sample::SampleData;

/// The preview's gain.
pub const GAIN: f32 = 0.7;

pub(super) struct Preview {
    data: Arc<SampleData>,
    /// Where it is in the sample's frames, and how far it moves an output frame.
    pos: f64,
    step: f64,
    /// A longer part of the sample comes ([`Self::grow`]): the end waits for it.
    more: bool,
}

impl Preview {
    /// `data` from frame `from`, at output rate `rate`.
    pub fn new(data: Arc<SampleData>, from: usize, rate: f32) -> Self {
        let step = f64::from(data.sample_rate) / f64::from(rate);
        Self {
            data,
            pos: from as f64,
            step,
            more: false,
        }
    }

    /// It goes on in `data`, the same sample decoded further, from where it is; `more`: a longer part still comes.
    /// The Arc it replaces is never the last, as with [`AudioRenderer::preview`].
    pub fn grow(&mut self, data: Arc<SampleData>, more: bool) {
        (self.data, self.more) = (data, more);
    }

    /// Adds its next frames into `left` and `right` (linear between frames); false once it is past the end.
    pub fn add(&mut self, left: &mut [f32], right: &mut [f32]) -> bool {
        let (l, r) = (&*self.data.left, self.data.right.as_deref());
        for (out_l, out_r) in left.iter_mut().zip(right.iter_mut()) {
            let i = self.pos as usize;
            if !self.step.is_finite() || self.step <= 0.0 {
                return false;
            }
            if i >= l.len() {
                return self.more;
            }
            let t = (self.pos - i as f64) as f32;
            let at = |s: &[f32]| s[i] + (s.get(i + 1).copied().unwrap_or(0.0) - s[i]) * t;
            let a = at(l);
            let b = r.map_or(a, at);
            *out_l += GAIN * a;
            *out_r += GAIN * b;
            self.pos += self.step;
        }
        self.more || (self.pos as usize) < l.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issues #131, #153: a preview that goes on in a longer part keeps its place, so it sounds as the whole played from
    /// the start; while more comes, a part that runs out waits silent at its end, then goes on from there.
    #[test]
    fn a_preview_goes_on_in_a_longer_part_from_where_it_is() {
        let whole: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.05).sin()).collect();
        let part = |n: usize| {
            Arc::new(SampleData {
                sample_rate: 44_100,
                left: whole[..n].into(),
                right: None,
                meta: Default::default(),
            })
        };
        let render = |preview: &mut Preview, frames: usize| {
            let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
            let on = preview.add(&mut left, &mut right);
            (left, on)
        };
        // At 48 kHz, between frames.
        let (alone, _) = render(&mut Preview::new(part(1000), 0, 48_000.0), 900);
        let mut grown = Preview::new(part(400), 0, 48_000.0);
        grown.grow(part(400), true);
        let (start, on) = render(&mut grown, 300);
        assert!(on);
        grown.grow(part(1000), false);
        let (rest, _) = render(&mut grown, 600);
        assert_eq!([start, rest].concat(), alone, "seamless");
        let mut short = Preview::new(part(200), 0, 44_100.0);
        short.grow(part(200), true);
        let (waited, on) = render(&mut short, 300);
        assert!(on, "it waits");
        assert!(waited[200..].iter().all(|&s| s == 0.0), "silent");
        short.grow(part(1000), false);
        let (rest, _) = render(&mut short, 100);
        let expected: Vec<f32> = whole[200..300].iter().map(|s| GAIN * s).collect();
        assert_eq!(rest, expected, "on from its end");
    }
}

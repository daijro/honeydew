//! Display-side view of the same quantized envelope events the audio engine plays.
//! Cached independently of source peaks. Never reads/writes PCM or audio-thread state.
use super::*;
use banaforge_audio::automation::{AutomationClip, AutomationPoint};

pub(crate) struct VolumeEnvelope {
    base: f32,
    steps: Vec<(u32, f32)>,
}

impl VolumeEnvelope {
    pub(crate) fn gain_at(&self, tick: f64) -> f32 {
        let end = self.steps.partition_point(|&(t, _)| f64::from(t) <= tick);
        end.checked_sub(1).map_or(self.base, |i| self.steps[i].1)
    }

    /// Conservative gain for a simplified column. Retain short loud sections,
    /// rather than sampling every Nth point and losing transients at far zoom.
    pub(crate) fn column_gain(&self, first: f64, end: f64) -> f32 {
        let a = self.steps.partition_point(|&(t, _)| f64::from(t) <= first);
        let b = self.steps.partition_point(|&(t, _)| f64::from(t) < end);
        self.steps[a..b.max(a)].iter().fold(self.gain_at(first), |g, s| g.max(s.1))
    }
}

thread_local! {
    static CACHE: std::cell::RefCell<std::collections::VecDeque<(Vec<Point>, u32, Arc<VolumeEnvelope>)>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

pub(crate) fn envelope(pts: &[Point], base: f32) -> Arc<VolumeEnvelope> {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(i) = cache.iter().position(|(p, b, _)| p == pts && *b == base.to_bits()) {
            let hit = cache.remove(i).unwrap();
            let value = hit.2.clone();
            cache.push_back(hit);
            return value;
        }
        let controls = pts.iter().map(|p| AutomationPoint {
            tick: p.tick.into(), value: p.value, shape: p.shape,
        }).collect();
        let built = AutomationClip::new(0, 1, 1, 0, 1, controls).expect("valid synthetic envelope timing");
        let value = Arc::new(VolumeEnvelope {
            base,
            steps: built.events().iter().map(|e| (e.tick as u32,
                banaforge_audio::drums::level_gain(e.normalized_value()))).collect(),
        });
        let bytes = |p: &[Point], e: &VolumeEnvelope| p.len() * std::mem::size_of::<Point>()
            + e.steps.len() * std::mem::size_of::<(u32, f32)>();
        let size = bytes(pts, &value);
        if size <= 4 * 1024 * 1024 {
            while cache.len() >= 64 || cache.iter().map(|(p, _, e)| bytes(p, e)).sum::<usize>() + size > 4 * 1024 * 1024 {
                cache.pop_front();
            }
            cache.push_back((pts.to_vec(), base.to_bits(), value.clone()));
        }
        value
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn volume_envelope_matches_playback_events_and_invalidates_only_envelope_cache() {
        let pts = vec![Point { tick: 0, value: 0, ..Default::default() },
            Point { tick: 128, value: 55910, ..Default::default() }];
        let curve = envelope(&pts, 1.0);
        assert!(Arc::ptr_eq(&curve, &envelope(&pts, 1.0)));
        assert!(curve.gain_at(128.0) > curve.gain_at(64.0));
        assert!(curve.gain_at(64.0) > curve.gain_at(0.0));
        let built = AutomationClip::new(0, 1, 1, 0, 1, pts.iter().map(|p| AutomationPoint {
            tick: p.tick.into(), value: p.value, shape: p.shape }).collect()).unwrap();
        for event in built.events() {
            let last = built.events().iter().rfind(|e| e.tick == event.tick).unwrap();
            assert_eq!(curve.gain_at(event.tick as f64), banaforge_audio::drums::level_gain(last.normalized_value()));
        }
        let mut edited = pts.clone();
        edited[1].value = 20000;
        assert!(!Arc::ptr_eq(&curve, &envelope(&edited, 1.0)));
        assert_eq!(envelope(&[], 0.25).gain_at(100.0), 0.25);
        assert!(curve.column_gain(0.0, 129.0) >= curve.gain_at(128.0));
    }
}

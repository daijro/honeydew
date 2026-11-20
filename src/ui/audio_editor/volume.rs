//! Display-side view of the same quantized envelope events the audio engine plays.
//! Cached independently of source peaks. Never reads/writes PCM or audio-thread state.
use super::*;
use banaforge_audio::automation::{AutomationClip, AutomationPoint};

pub(crate) struct VolumeEnvelope {
    base: f32,
    steps: Vec<(u32, f32)>,
}

impl VolumeEnvelope {
    pub(crate) fn memory_bytes(&self) -> usize {
        self.steps.capacity() * std::mem::size_of::<(u32, f32)>()
    }
    pub(crate) fn gain_at(&self, tick: f64) -> f32 {
        let end = self.steps.partition_point(|&(t, _)| f64::from(t) <= tick);
        end.checked_sub(1).map_or(self.base, |i| self.steps[i].1)
    }

    /// Conservative gain for a simplified column. Retain short loud sections,
    /// rather than sampling every Nth point and losing transients at far zoom.
    pub(crate) fn column_gain(&self, first: f64, end: f64) -> f32 {
        let a = self.steps.partition_point(|&(t, _)| f64::from(t) <= first);
        let b = self.steps.partition_point(|&(t, _)| f64::from(t) < end);
        self.steps[a..b.max(a)]
            .iter()
            .fold(self.gain_at(first), |g, s| g.max(s.1))
    }
}

thread_local! {
    static CACHE: std::cell::RefCell<std::collections::VecDeque<(Vec<Point>, u32, Arc<VolumeEnvelope>)>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

pub(crate) fn envelope(pts: &[Point], base: f32) -> Arc<VolumeEnvelope> {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(i) = cache
            .iter()
            .position(|(p, b, _)| p == pts && *b == base.to_bits())
        {
            let hit = cache.remove(i).unwrap();
            let value = hit.2.clone();
            cache.push_back(hit);
            return value;
        }
        let controls = pts
            .iter()
            .map(|p| AutomationPoint {
                tick: p.tick.into(),
                value: p.value,
                shape: p.shape,
            })
            .collect();
        let built =
            AutomationClip::new(0, 1, 1, 0, 1, controls).expect("valid synthetic envelope timing");
        let value = Arc::new(VolumeEnvelope {
            base,
            steps: built
                .events()
                .iter()
                .map(|e| {
                    (
                        e.tick as u32,
                        banaforge_audio::drums::level_gain(e.normalized_value()),
                    )
                })
                .collect(),
        });
        let bytes = |p: &[Point], e: &VolumeEnvelope| {
            p.len() * std::mem::size_of::<Point>()
                + e.steps.len() * std::mem::size_of::<(u32, f32)>()
        };
        let size = bytes(pts, &value);
        if size <= 4 * 1024 * 1024 {
            while cache.len() >= 64
                || cache.iter().map(|(p, _, e)| bytes(p, e)).sum::<usize>() + size > 4 * 1024 * 1024
            {
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
        let pts = vec![
            Point {
                tick: 0,
                value: 0,
                ..Default::default()
            },
            Point {
                tick: 128,
                value: 55910,
                ..Default::default()
            },
        ];
        let curve = envelope(&pts, 1.0);
        assert!(Arc::ptr_eq(&curve, &envelope(&pts, 1.0)));
        assert!(curve.gain_at(128.0) > curve.gain_at(64.0));
        assert!(curve.gain_at(64.0) > curve.gain_at(0.0));
        let built = AutomationClip::new(
            0,
            1,
            1,
            0,
            1,
            pts.iter()
                .map(|p| AutomationPoint {
                    tick: p.tick.into(),
                    value: p.value,
                    shape: p.shape,
                })
                .collect(),
        )
        .unwrap();
        for event in built.events() {
            let last = built
                .events()
                .iter()
                .rfind(|e| e.tick == event.tick)
                .unwrap();
            assert_eq!(
                curve.gain_at(event.tick as f64),
                banaforge_audio::drums::level_gain(last.normalized_value())
            );
        }
        let mut edited = pts.clone();
        edited[1].value = 20000;
        assert!(!Arc::ptr_eq(&curve, &envelope(&edited, 1.0)));
        assert_eq!(envelope(&[], 0.25).gain_at(100.0), 0.25);
        assert!(curve.column_gain(0.0, 129.0) >= curve.gain_at(128.0));
    }
}

/// Stereo output-gain preview: sampler linear balance, not a stereo-position/fold-down effect.
/// Merge the actual quantized gain/balance changes; source PCM and source peak caches stay untouched.
pub(crate) fn stereo(volume: &Arc<VolumeEnvelope>, pan: &[Point], base: f32) -> [Arc<VolumeEnvelope>; 2] {
    type Entry = (Arc<VolumeEnvelope>, Vec<Point>, u32, [Arc<VolumeEnvelope>; 2]);
    thread_local! { static STEREO: std::cell::RefCell<std::collections::VecDeque<Entry>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) }; }
    STEREO.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(i) = cache.iter().position(|(v,p,b,_)| Arc::ptr_eq(v, volume) && p == pan && *b == base.to_bits()) {
            let hit = cache.remove(i).unwrap(); let out = hit.3.clone(); cache.push_back(hit); return out;
        }
        let built = AutomationClip::new(0,1,1,0,1,pan.iter().map(|p| AutomationPoint {tick:p.tick.into(), value:p.value, shape:p.shape}).collect()).unwrap();
        let steps: Vec<_> = built.events().iter().map(|e| (e.tick as u32,e.normalized_value())).collect();
        let mut ticks: Vec<_> = volume.steps.iter().map(|s| s.0).chain(steps.iter().map(|s| s.0)).collect();
        ticks.sort_unstable(); ticks.dedup();
        let balance = |value: f32, channel| { let p = 2.0 * value - 1.0; if channel == 0 { 1.0-p.max(0.0) } else { 1.0+p.min(0.0) } };
        let out = std::array::from_fn(|ch| Arc::new(VolumeEnvelope {
            base: volume.base * balance(base,ch),
            steps: ticks.iter().map(|&tick| {
                let n = steps.partition_point(|s| s.0 <= tick);
                let pan = n.checked_sub(1).map_or(base, |i| steps[i].1);
                (tick,volume.gain_at(f64::from(tick)) * balance(pan,ch))
            }).collect(),
        }));
        let size = |e: &Entry| e.0.memory_bytes()+ e.1.len()*std::mem::size_of::<Point>() + e.3.iter().map(|v| v.memory_bytes()).sum::<usize>();
        let entry = (volume.clone(),pan.to_vec(),base.to_bits(),out.clone());
        let bytes = size(&entry);
        if bytes <= 4*1024*1024 {
            while cache.len() >= 64 || cache.iter().map(size).sum::<usize>()+bytes > 4*1024*1024 { cache.pop_front(); }
            cache.push_back(entry);
        }
        out
    })
}

#[cfg(test)]
mod stereo_tests {
    use super::*;
    #[test]
    fn clip_envelope_stereo_preview_matches_balance_gain_and_cache_invalidates() {
        let volume = envelope(&[],1.0);
        let point = |value| vec![Point { tick:0,value,..Default::default() }];
        for (pan,want) in [(0,[1.0,0.0]),(32768,[1.0,1.0]),(65535,[0.0,1.0])] {
            let p=point(pan); let a=stereo(&volume,&p,0.5); let b=stereo(&volume,&p,0.5);
            for ch in 0..2 { assert!((a[ch].gain_at(0.0)-want[ch]).abs()<0.0001); assert!(Arc::ptr_eq(&a[ch],&b[ch])); }
        }
    }
}

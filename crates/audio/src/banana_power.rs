//! Original Banana rack processors. Functional references only; not proprietary emulations.
//! All history is allocated at construction, never in processing. Gross Beat uses causal
//! delay curves, a maximum eight-second history, and sixteen saved points per envelope.
use crate::ProcessContext;

pub const FRESH_AIR: u16 = 1009;
pub const SOUNDGOODIZER: u16 = 1010;
pub const BLOOD_OVERDRIVE: u16 = 1011;
pub const GROSS_BEAT: u16 = 1012;
pub const IDS: [u16; 4] = [FRESH_AIR, SOUNDGOODIZER, BLOOD_OVERDRIVE, GROSS_BEAT];
pub const TIME_POINTS: usize = 5;
pub const VOLUME_POINTS: usize = 21;
pub const POINTS: usize = 16;
pub const PARAMS: usize = 37;
pub const NAMES: [&str; 4] = [
    "Banana Fresh Air",
    "Banana Soundgoodizer",
    "Banana Blood Overdrive",
    "Banana Gross Beat",
];

pub fn defaults(id: u16) -> Vec<f32> {
    match id {
        FRESH_AIR => vec![1.0, 0.0, 0.0, 0.5],
        SOUNDGOODIZER => vec![1.0, 0.0, 0.0],
        BLOOD_OVERDRIVE => vec![1.0, 0.35, 0.5, 0.7, 0.5, 1.0],
        GROSS_BEAT => {
            let mut p = vec![0.0; PARAMS];
            p[0] = 1.0;
            p[3] = 0.5; // one bar
            p[4] = 1.0;
            p[VOLUME_POINTS..].fill(1.0);
            p
        }
        _ => Vec::new(),
    }
}

// Cache transcendental control conversions. Automation still uses the exact
// smoothed value every sample; steady controls do not repeat exp/powf/sqrt.
struct ScalarCache {
    key: f32,
    value: f32,
}
impl ScalarCache {
    fn new() -> Self {
        Self { key: f32::NAN, value: 0.0 }
    }
    fn get(&mut self, key: f32, convert: impl FnOnce(f32) -> f32) -> f32 {
        if self.key != key {
            self.key = key;
            self.value = convert(key);
        }
        self.value
    }
}

pub struct State {
    id: u16,
    rate: f32,
    target: [f32; PARAMS],
    p: [f32; PARAMS],
    count: usize,
    split: [f32; 2],
    trim_gain: ScalarCache,
    post_filter: ScalarCache,
    drive_gain: ScalarCache,
    low: [f32; 2],
    high: [f32; 2],
    dc: [f32; 2],
    envelope: [f32; 3],
    ring: Vec<(f32, f32)>,
    write: usize,
    valid: usize,
    beat: f64,
    step: f64,
    tempo: f32,
    last_wet: (f32, f32),
    correction: (f32, f32),
    last_delay: f32,
    level: f32,
}

impl State {
    pub fn new(id: u16, rate: f32, params: &[f32]) -> Self {
        let mut state = Self {
            id,
            rate,
            target: [0.0; PARAMS],
            p: [0.0; PARAMS],
            count: params.len().min(PARAMS),
            split: [0.0; 2],
            trim_gain: ScalarCache::new(),
            post_filter: ScalarCache::new(),
            drive_gain: ScalarCache::new(),
            low: [0.0; 2],
            high: [0.0; 2],
            dc: [0.0; 2],
            envelope: [0.0; 3],
            ring: if id == GROSS_BEAT {
                vec![(0.0, 0.0); (rate * 8.0).ceil() as usize + 2]
            } else {
                Vec::new()
            },
            write: 0,
            valid: 0,
            beat: 0.0,
            step: 0.0,
            tempo: 120.0,
            last_wet: (0.0, 0.0),
            correction: (0.0, 0.0),
            last_delay: 0.0,
            level: 1.0,
        };
        state.refresh(params, rate);
        state.p = state.target;
        state
    }

    pub fn refresh(&mut self, params: &[f32], rate: f32) {
        if self.rate != rate {
            self.post_filter.key = f32::NAN;
        }
        self.rate = rate;
        self.split = [250.0, 4500.0].map(|hz| 1.0 - (-std::f32::consts::TAU * hz / rate).exp());
        for (dst, &value) in self.target.iter_mut().zip(params) {
            *dst = if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
    }

    pub fn prepare_block(&mut self, context: ProcessContext) {
        self.tempo = if context.tempo_bpm.is_finite() {
            context.tempo_bpm.clamp(30.0, 300.0)
        } else {
            120.0
        };
        self.step = f64::from(self.tempo) / (60.0 * f64::from(self.rate));
        if context.playing && context.start_beat.is_finite() {
            if (context.start_beat - self.beat).abs() > self.step * 4.0 {
                // Invalidate, don't clear a large ring on the audio thread.
                self.valid = 0;
                self.last_wet = (0.0, 0.0);
            }
            self.beat = context.start_beat;
        }
    }

    pub fn reset(&mut self) {
        self.low = [0.0; 2];
        self.high = [0.0; 2];
        self.dc = [0.0; 2];
        self.envelope = [0.0; 3];
        self.valid = 0;
        self.write = 0;
        self.beat = 0.0;
        self.last_wet = (0.0, 0.0);
        self.correction = (0.0, 0.0);
        self.last_delay = 0.0;
        self.level = 1.0;
        self.p = self.target;
    }

    pub fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        for i in 1..self.count {
            self.p[i] += (self.target[i] - self.p[i]) * 0.002;
        }
        let input = [l, r].map(|v| {
            if v.is_finite() {
                v.clamp(-32.0, 32.0)
            } else {
                0.0
            }
        });
        if self.id == GROSS_BEAT {
            return self.warp(input[0], input[1]);
        }
        let [a, b] = self.split;
        let mut bands = [[0.0; 3]; 2];
        for c in 0..2 {
            self.low[c] += a * (input[c] - self.low[c]);
            self.high[c] += b * (input[c] - self.high[c]);
            bands[c] = [
                self.low[c],
                self.high[c] - self.low[c],
                input[c] - self.high[c],
            ];
        }
        let mut output = input;
        match self.id {
            FRESH_AIR => {
                let gain = self.trim_gain.get(self.p[3], trim);
                for c in 0..2 {
                    let mid = bands[c][1];
                    let high = bands[c][2];
                    let mid_extra = if self.p[1] == 0.0 {
                        0.0
                    } else {
                        self.p[1] * ((mid * 3.0).tanh() - mid) * 0.7
                    };
                    let high_extra = if self.p[2] == 0.0 {
                        0.0
                    } else {
                        self.p[2] * ((high * 4.0).tanh() - high) * 0.6
                    };
                    let extra = mid_extra + high_extra;
                    self.dc[c] += a * extra;
                    output[c] = (input[c] + extra - self.dc[c]) * gain;
                }
            }
            SOUNDGOODIZER => {
                let preset = (self.target[2] * 3.0).round() as usize;
                let (threshold, ratio, drive) = [
                    (0.18, 2.5, 1.3),
                    (0.12, 4.0, 1.6),
                    (0.25, 2.0, 2.2),
                    (0.08, 6.0, 2.8),
                ][preset.min(3)];
                let mut wet = [0.0; 2];
                for band in 0..3 {
                    let peak = bands[0][band].abs().max(bands[1][band].abs());
                    let k = if peak > self.envelope[band] {
                        0.015
                    } else {
                        0.0003
                    };
                    self.envelope[band] += k * (peak - self.envelope[band]);
                    let env = self.envelope[band].max(0.00001);
                    let gain = if env > threshold {
                        (threshold / env).powf(1.0 - 1.0 / ratio)
                    } else {
                        1.0
                    };
                    for c in 0..2 {
                        wet[c] += (bands[c][band] * gain * drive).tanh() / drive.tanh() * 1.25;
                    }
                }
                for c in 0..2 {
                    output[c] = input[c] + self.p[1] * (wet[c] - input[c]);
                }
            }
            BLOOD_OVERDRIVE => {
                let drive = 1.0 + 30.0 * self.p[1] * self.p[1];
                let rate = self.rate;
                let filter = self.post_filter.get(self.p[3], |v| {
                    1.0 - (-std::f32::consts::TAU * (700.0 + 15000.0 * v) / rate).exp()
                });
                let compensation = self.drive_gain.get(drive, f32::sqrt);
                let gain = self.trim_gain.get(self.p[4], trim);
                for c in 0..2 {
                    let tone = bands[c][0] + bands[c][1] + bands[c][2] * (0.2 + 1.6 * self.p[2]);
                    let shaped = (tone * drive).tanh() / compensation;
                    self.dc[c] += filter * (shaped - self.dc[c]);
                    let wet = self.dc[c] * gain;
                    output[c] = input[c] + self.p[5] * (wet - input[c]);
                }
            }
            _ => {}
        }
        (output[0], output[1])
    }

    fn warp(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.ring[self.write] = (l, r);
        self.valid = (self.valid + 1).min(self.ring.len());
        let length = [1.0, 2.0, 4.0, 8.0, 16.0][(self.target[3] * 4.0).round() as usize];
        let phase = (self.beat / length).rem_euclid(1.0) as f32;
        let frames = length as f32 * 60.0 * self.rate / self.tempo;
        let delay = match (self.target[1] * 4.0).round() as usize {
            1 => phase * 0.5,
            2 => 0.5 * phase * phase,
            3 => (phase * 8.0).floor() / 8.0,
            4 => curve(&self.p, TIME_POINTS, phase),
            _ => 0.0,
        };
        let delay = (delay * frames).min((self.valid - 1) as f32);
        let at = (self.write as f64 - f64::from(delay)).rem_euclid(self.ring.len() as f64);
        let i = at.floor() as usize;
        let f = at.fract() as f32;
        let x = self.ring[i];
        let y = self.ring[(i + 1) % self.ring.len()];
        let volume = match (self.target[2] * 3.0).round() as usize {
            1 => {
                if (phase * 8.0).fract() < 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            2 => phase,
            3 => curve(&self.p, VOLUME_POINTS, phase),
            _ => 1.0,
        };
        let wet = (x.0 + f * (y.0 - x.0), x.1 + f * (y.1 - x.1));
        // Smooth level and correct only read-head discontinuities; filtering the
        // entire signal would incorrectly remove treble even on a half-speed path.
        let smooth = (1.0 / (0.001 * self.rate)).min(1.0);
        self.level += smooth * (volume - self.level);
        let desired = (wet.0 * self.level, wet.1 * self.level);
        if (delay - self.last_delay).abs() > 2.0 {
            self.correction = (self.last_wet.0 - desired.0, self.last_wet.1 - desired.1);
        }
        self.last_delay = delay;
        self.last_wet = (desired.0 + self.correction.0, desired.1 + self.correction.1);
        self.correction.0 *= 1.0 - smooth;
        self.correction.1 *= 1.0 - smooth;
        self.write = (self.write + 1) % self.ring.len();
        self.beat += self.step;
        (
            l + self.p[4] * (self.last_wet.0 - l),
            r + self.p[4] * (self.last_wet.1 - r),
        )
    }
}
fn trim(v: f32) -> f32 {
    10.0_f32.powf((v - 0.5) * 24.0 / 20.0)
}
fn curve(p: &[f32; PARAMS], base: usize, phase: f32) -> f32 {
    let x = phase * (POINTS - 1) as f32;
    let i = (x as usize).min(POINTS - 2);
    p[base + i] + (x - i as f32) * (p[base + i + 1] - p[base + i])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_controls_convert_once_and_automation_is_not_quantized() {
        let mut cache = ScalarCache::new();
        let calls = std::cell::Cell::new(0);
        for v in [0.5, 0.5, 0.500001, 0.500001, 0.75, 0.5] {
            let actual = cache.get(v, |x| {
                calls.set(calls.get() + 1);
                trim(x)
            });
            assert_eq!(actual, trim(v));
        }
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn cached_controls_match_recalculation_across_automation_and_rate_changes() {
        for id in [FRESH_AIR, BLOOD_OVERDRIVE] {
            let p = defaults(id);
            let mut cached = State::new(id, 48000.0, &p);
            let mut reference = State::new(id, 48000.0, &p);
            for n in 0..6000 {
                if n == 2000 || n == 4000 {
                    let mut changed = p.clone();
                    changed[1..].fill(0.73);
                    let rate = if n == 4000 { 44100.0 } else { 48000.0 };
                    cached.refresh(&changed, rate);
                    reference.refresh(&changed, rate);
                }
                reference.trim_gain.key = f32::NAN;
                reference.post_filter.key = f32::NAN;
                reference.drive_gain.key = f32::NAN;
                let x = (n as f32 * 0.13).sin() * 0.4;
                assert_eq!(cached.process_frame(x, -x), reference.process_frame(x, -x));
            }
        }
    }

    #[test]
    fn banana_power_neutral_and_finite() {
        for rate in [44100.0, 48000.0, 96000.0] {
            for id in IDS {
                let params = defaults(id);
                let mut state = State::new(id, rate, &params);
                state.prepare_block(ProcessContext {
                    sample_rate: rate,
                    tempo_bpm: 120.0,
                    start_beat: 0.0,
                    playing: true,
                });
                for n in 0..4096 {
                    let x = (n as f32 * 0.13).sin() * 0.4;
                    let (l, r) = state.process_frame(x, x);
                    assert!(l.is_finite() && l.abs() < 10.0);
                    assert_eq!(l, r);
                    if id != BLOOD_OVERDRIVE {
                        assert!((l - x).abs() < 0.00001, "{id}");
                    }
                }
                let mut extreme = params.clone();
                extreme[1..].fill(1.0);
                state.refresh(&extreme, rate);
                for _ in 0..4096 {
                    let (l, r) = state.process_frame(0.3, -0.2);
                    assert!(l.is_finite() && r.is_finite());
                }
                state.reset();
            }
        }
    }
    #[test]
    fn banana_power_controls_change_audio() {
        for id in IDS {
            let mut p = defaults(id);
            p[1] = 1.0;
            if id == GROSS_BEAT {
                p[TIME_POINTS..VOLUME_POINTS].fill(0.1);
            }
            let mut state = State::new(id, 48000.0, &p);
            state.prepare_block(ProcessContext {
                sample_rate: 48000.0,
                tempo_bpm: 120.0,
                start_beat: 0.0,
                playing: true,
            });
            let mut difference = 0.0;
            for n in 0..8000 {
                let x = (n as f32 * 0.73).sin() * 0.4;
                difference += (state.process_frame(x, x).0 - x).abs();
            }
            assert!(difference > 0.1, "silent/no-op control for {id}");
        }
    }
}

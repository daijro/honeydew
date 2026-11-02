use crate::biquad::flush;

#[derive(Clone, Copy)]
struct Biquad {
    coeffs: [f32; 5],
    input: [[f32; 2]; 2],
    output: [[f32; 2]; 2],
}

impl Biquad {
    fn new(coeffs: [f32; 5]) -> Self {
        Self {
            coeffs,
            input: [[0.0; 2]; 2],
            output: [[0.0; 2]; 2],
        }
    }

    fn process(&mut self, channel: usize, sample: f32) -> f32 {
        let [b0, b1, b2, a1, a2] = self.coeffs;
        let result = flush(
            b0 * sample
                + b1 * self.input[channel][0]
                + b2 * self.input[channel][1]
                + a1 * self.output[channel][0]
                + a2 * self.output[channel][1],
        );
        self.input[channel][1] = self.input[channel][0];
        self.input[channel][0] = sample;
        self.output[channel][1] = self.output[channel][0];
        self.output[channel][0] = result;
        result
    }

    fn clear(&mut self) {
        self.input = [[0.0; 2]; 2];
        self.output = [[0.0; 2]; 2];
    }
}

struct Splitter {
    low: [Biquad; 2],
    high: [Biquad; 2],
}

impl Splitter {
    fn new(frequency: f32, sample_rate: f32) -> Self {
        let (low, high) = crossover_coefficients(frequency, sample_rate);
        Self {
            low: [Biquad::new(low); 2],
            high: [Biquad::new(high); 2],
        }
    }

    fn set_frequency(&mut self, frequency: f32, sample_rate: f32) {
        let (low, high) = crossover_coefficients(frequency, sample_rate);
        for filter in &mut self.low {
            filter.coeffs = low;
        }
        for filter in &mut self.high {
            filter.coeffs = high;
        }
    }

    fn process(&mut self, left: f32, right: f32) -> ((f32, f32), (f32, f32)) {
        let mut low = [left, right];
        let mut high = low;
        for stage in 0..2 {
            for channel in 0..2 {
                low[channel] = self.low[stage].process(channel, low[channel]);
                high[channel] = self.high[stage].process(channel, high[channel]);
            }
        }
        ((low[0], low[1]), (high[0], high[1]))
    }

    fn clear(&mut self) {
        for filter in self.low.iter_mut().chain(self.high.iter_mut()) {
            filter.clear();
        }
    }
}

fn crossover_coefficients(frequency: f32, sample_rate: f32) -> ([f32; 5], [f32; 5]) {
    let omega = 2.0 * std::f64::consts::PI * f64::from(frequency.min(sample_rate * 0.49))
        / f64::from(sample_rate);
    let (sine, cosine) = omega.sin_cos();
    let alpha = sine / (2.0 * 0.707000017);
    let denominator = 1.0 / (1.0 + alpha);
    let feedback = [
        (2.0 * cosine * denominator) as f32,
        ((alpha - 1.0) * denominator) as f32,
    ];
    let low_b0 = (0.5 * (1.0 - cosine) * denominator) as f32;
    let high_b0 = (0.5 * (1.0 + cosine) * denominator) as f32;
    (
        [
            low_b0,
            ((1.0 - cosine) * denominator) as f32,
            low_b0,
            feedback[0],
            feedback[1],
        ],
        [
            high_b0,
            (-(1.0 + cosine) * denominator) as f32,
            high_b0,
            feedback[0],
            feedback[1],
        ],
    )
}

struct Band {
    target_gain: f32,
    gain: f32,
    target_input: f32,
    input: f32,
    dry_mix: f32,
    dry: f32,
    target_width: f32,
    width: f32,
    threshold: f32,
    ratio_inverse: f32,
    attack: f32,
    release: f32,
    peak: f32,
    envelope: f32,
    bypass: bool,
    safe_width: bool,
    /// The last frame's input level (after Input Gain), output level and compressor gain, for the band graph.
    levels: [f32; 3],
}

impl Band {
    fn new() -> Self {
        Self {
            target_gain: 1.0,
            gain: -1.0,
            target_input: 1.0,
            input: -1.0,
            dry_mix: 0.0,
            dry: -1.0,
            target_width: 0.5,
            width: -1.0,
            threshold: 1.0,
            ratio_inverse: 1.0,
            attack: 1.0,
            release: 1.0,
            peak: -1.0,
            envelope: -1.0,
            bypass: false,
            safe_width: false,
            levels: [0.0, 0.0, 1.0],
        }
    }

    /// The band's frame `(left, right)`, its detector reading `(key_l, key_r)` (the frame itself, unkeyed).
    fn process(&mut self, (left, right): (f32, f32), (key_l, key_r): (f32, f32)) -> (f32, f32) {
        if self.bypass {
            let level = left.abs().max(right.abs());
            self.levels = [level, level, 1.0];
            return (left, right);
        }
        if self.gain < 0.0 {
            self.gain = self.target_gain;
        }
        if self.input < 0.0 {
            self.input = self.target_input;
        }
        self.gain += (self.target_gain - self.gain) * 0.01;
        self.input += (self.target_input - self.input) * 0.01;
        let input_left = left * self.input;
        let input_right = right * self.input;
        let level = (key_l * self.input).abs().max((key_r * self.input).abs()) * self.input;
        self.peak = if level > self.peak {
            level
        } else {
            flush(self.peak * (1.0 - self.release))
        };
        self.envelope = if self.envelope < 0.0 {
            self.peak
        } else {
            flush(self.envelope + (self.peak - self.envelope) * self.attack)
        };
        let compression = if self.envelope >= self.threshold && self.envelope > 0.0 {
            (self.threshold + self.ratio_inverse * (self.envelope - self.threshold)) / self.envelope
        } else {
            1.0
        };
        let amount = self.dry + compression;
        if self.dry < 0.0 {
            self.dry = self.dry_mix;
        }
        let mut left = input_left * self.gain * amount;
        let mut right = input_right * self.gain * amount;
        if self.safe_width {
            let width = self.target_width;
            if width <= 0.5 {
                let mid = (left + right) * 0.5;
                let side = (left - right) * width;
                left = mid + side;
                right = mid - side;
            } else {
                left -= (width - 0.5) * right;
                right -= (width - 0.5) * left;
            }
        } else {
            if self.width < 0.0 {
                self.width = self.target_width;
            } else if self.width != self.target_width {
                self.width += (self.target_width - self.width) * 0.01;
                if (self.width - self.target_width).abs() < 0.001 {
                    self.width = self.target_width;
                }
            }
            let mid = (left + right) * (2.0 - 2.0 * self.width).clamp(0.0, 1.0) * 0.5;
            let side = (left - right) * (2.0 * self.width).clamp(0.0, 1.0) * 0.5;
            left = mid + side;
            right = mid - side;
        }
        self.dry = flush(self.dry + (self.dry_mix - self.dry) * 0.01);
        let level_in = input_left.abs().max(input_right.abs());
        self.levels = [level_in, left.abs().max(right.abs()), compression];
        (left, right)
    }

    fn clear(&mut self) {
        self.gain = -1.0;
        self.input = -1.0;
        self.dry = -1.0;
        self.width = -1.0;
        self.peak = -1.0;
        self.envelope = -1.0;
        self.levels = [0.0, 0.0, 1.0];
    }
}

pub struct MultibandState {
    sample_rate: f32,
    params: [f32; 31],
    splitters: [Splitter; 3],
    /// The key's crossover, at the same frequencies ([`Self::keyed`]).
    key_splitters: [Splitter; 3],
    bands: [Band; 3],
    master_gain: f32,
    master_target: f32,
    solo_band: Option<usize>,
}

impl MultibandState {
    pub fn new(params: &[f32], sample_rate: f32) -> Self {
        assert!(params.len() >= 31);
        assert!(sample_rate.is_finite() && sample_rate > 0.0);
        let params: [f32; 31] = params[..31].try_into().unwrap();
        let low = crossover_frequency(params[2]);
        let high = crossover_frequency(params[3]);
        let mut state = Self {
            sample_rate,
            params,
            splitters: [
                Splitter::new(low, sample_rate),
                Splitter::new(high, sample_rate),
                Splitter::new(high, sample_rate),
            ],
            key_splitters: [
                Splitter::new(low, sample_rate),
                Splitter::new(high, sample_rate),
                Splitter::new(high, sample_rate),
            ],
            bands: std::array::from_fn(|_| Band::new()),
            master_gain: -1.0,
            master_target: 1.0,
            solo_band: None,
        };
        state.update_controls();
        state
    }

    /// Each band's peak detector, envelope and dry smoother (for the settle test).
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn band_states(&self) -> [[f32; 3]; 3] {
        self.bands
            .each_ref()
            .map(|band| [band.peak, band.envelope, band.dry])
    }

    /// Each band's last input level (after its Input Gain), output level and compressor gain (1 = none): what its
    /// band graph shows.
    pub(crate) fn band_levels(&self) -> [[f32; 3]; 3] {
        self.bands.each_ref().map(|band| band.levels)
    }

    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        assert!(params.len() >= 31);
        assert!(sample_rate.is_finite() && sample_rate > 0.0);
        if sample_rate != self.sample_rate {
            let solo = self.solo_band;
            *self = Self::new(params, sample_rate);
            self.solo_band = solo;
            self.update_controls();
            return;
        }
        for splitters in [&mut self.splitters, &mut self.key_splitters] {
            if params[2] != self.params[2] {
                splitters[0].set_frequency(crossover_frequency(params[2]), sample_rate);
            }
            if params[3] != self.params[3] {
                let high = crossover_frequency(params[3]);
                splitters[1].set_frequency(high, sample_rate);
                splitters[2].set_frequency(high, sample_rate);
            }
        }
        self.params.copy_from_slice(&params[..31]);
        self.update_controls();
    }

    pub fn set_solo_band(&mut self, band: Option<usize>) {
        assert!(band.is_none_or(|index| index < 3));
        self.solo_band = band;
        self.update_controls();
    }

    pub fn reset(&mut self) {
        for splitter in self.splitters.iter_mut().chain(&mut self.key_splitters) {
            splitter.clear();
        }
        for band in &mut self.bands {
            band.clear();
        }
        self.master_gain = -1.0;
    }

    pub fn process_frame(&mut self, left: f32, right: f32) -> (f32, f32) {
        self.keyed(left, right, None)
    }

    /// [`Self::process_frame`] with each band's detector reading that band of `key` (left, right), split by the same
    /// crossover; `None`: its own band.
    pub fn keyed(&mut self, left: f32, right: f32, key: Option<(f32, f32)>) -> (f32, f32) {
        if self.params[0] < 0.5 {
            return (left, right);
        }
        let bands = split(&mut self.splitters, (left, right));
        let keys = key.map_or(bands, |key| split(&mut self.key_splitters, key));
        let low = self.bands[0].process(bands[0], keys[0]);
        let mid = self.bands[1].process(bands[1], keys[1]);
        let high = self.bands[2].process(bands[2], keys[2]);
        if self.master_gain < 0.0 {
            self.master_gain = self.master_target;
        } else {
            self.master_gain += (self.master_target - self.master_gain) * 0.01;
        }
        (
            self.master_gain * (low.0 + mid.0 + high.0),
            self.master_gain * (low.1 + mid.1 + high.1),
        )
    }

    fn update_controls(&mut self) {
        let p = &self.params;
        self.master_target = decibels(48.0 * p[1] - 24.0);
        for band_index in 0..3 {
            let base = 6 + band_index * 7;
            let band = &mut self.bands[band_index];
            let mode = (2.0 * p[base + 6]) as u8;
            band.bypass = mode == 1;
            band.target_gain = if mode == 2 || self.solo_band.is_some_and(|s| s != band_index) {
                0.0
            } else {
                decibels(48.0 * p[base + 5] - 24.0)
            };
            band.target_input = decibels(48.0 * p[base] - 24.0);
            band.threshold = decibels(48.0 * p[base + 1] - 48.0);
            band.ratio_inverse = 1.0 / (1.0 + 30.0 * p[base + 2]);
            let attack_seconds = 0.05 * p[base + 3] * p[base + 3];
            band.attack = if attack_seconds < 0.000001 {
                1.0
            } else {
                1.0 - 0.1_f64.powf(1.0 / f64::from(44_100.0 * attack_seconds)) as f32
            };
            let release_seconds = 0.01 + 0.99 * p[base + 4];
            band.release = 1.0 - 0.1_f64.powf(1.0 / f64::from(44_100.0 * release_seconds)) as f32;
            band.dry_mix = p[5] * p[5];
            band.target_width = p[27 + band_index];
            band.safe_width = p[30] >= 0.5;
        }
    }
}

/// `(left, right)` in its low, mid and high bands, by the crossover `splitters`.
fn split(splitters: &mut [Splitter; 3], (left, right): (f32, f32)) -> [(f32, f32); 3] {
    let (low, upper) = splitters[0].process(left, right);
    let (mid, high) = splitters[1].process(upper.0, upper.1);
    let (low_a, low_b) = splitters[2].process(low.0, low.1);
    [(low_a.0 + low_b.0, low_a.1 + low_b.1), mid, high]
}

fn crossover_frequency(value: f32) -> f32 {
    20.0 * (9.0 * value).exp2()
}

fn decibels(value: f32) -> f32 {
    if value < -60.0 {
        0.0
    } else {
        10.0_f32.powf(value * 0.05)
    }
}

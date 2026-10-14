use crate::biquad::{Biquad, Response, flush};
use std::f32::consts::{FRAC_PI_2, TAU};

pub const DEFAULT_PARAMS: [f32; 21] = [
    1.0, 0.0, 0.0, 0.5, 0.5, 0.0, 0.5, 0.0, 1.0, 1.0, 0.0, 0.5, 0.5, 0.5, 0.5, 0.0, 0.0, 0.5, 0.0,
    0.0, 0.0,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaveshaperError {
    InvalidValue,
    InvalidSampleRate,
    InvalidTempo,
}

/// The shape index = round(p1·6), in the menu's order (Smooth, Analog,
/// Moshpit, Foldback, Chainsaw, Rectifuzz, Cyclone).
pub fn shape_index(p1: f32) -> usize {
    ((p1 * 6.0).round() as usize).min(6)
}

/// A point curve of `(x, y, curve)` points at `input`: the first y before the first point, the last y at or after the
/// last, and between P(i−1) and P(i), with c = P(i).curve and t = (in − x0)/(x1 − x0), y0 + s·(y1 − y0) where
/// s = t^(1−c) if c > 0, else 1 − (1 − t)^(1+c). A zero-width segment holds no input.
fn point_curve(points: &[(f32, f32, f32)], input: f32) -> f32 {
    let (x_first, y_first, _) = points[0];
    if input < x_first {
        return y_first;
    }
    match points.windows(2).find(|w| input < w[1].0) {
        Some(&[(x0, y0, _), (x1, y1, c)]) => {
            let t = (input - x0) / (x1 - x0);
            let s = if c > 0.0 {
                t.powf(1.0 - c)
            } else {
                1.0 - (1.0 - t).powf(1.0 + c)
            };
            y0 + s * (y1 - y0)
        }
        _ => points[points.len() - 1].1,
    }
}

/// The transfer function `index` ([`shape_index`]; 6 and above are Cyclone) at `input`,
/// given the effective X and Y in 0…1. The engine and the panel's curve both use it.
pub fn shape(index: usize, x: f32, y: f32, input: f32) -> f32 {
    match index {
        // Smooth (soft clip).
        0 => {
            let a = 2.0 * x - 1.0;
            let a = if a.abs() < 1e-4 { 1e-4 } else { a };
            let k = 1.0 + 9.0 * y;
            let g = |u: f32| u / ((u - a).powi(2) + 1.0).sqrt();
            g(k * input) / g(a.signum() * (1.0 + 1.0 / a.abs())).abs()
        }
        // Analog (tanh).
        1 => {
            let (a, k) = (2.0 * x - 1.0, 1.0 + 9.0 * y);
            (a.tanh() + (k * input - a).tanh()) / (1.0 + a.abs().tanh())
        }
        // Moshpit (crossover).
        2 => {
            let (d, e, m) = (
                x.min(1.0 - x),
                (2.0 * y - 1.0).abs(),
                (1.0 - 2.0 * y).max(0.0),
            );
            let (inner, outer) = (0.25 - 0.12 * x, 0.01 + 0.18 * d);
            point_curve(
                &[
                    (-1.0, -1.0 + 0.37 * m, 0.0),
                    (-0.75, -0.75 + 0.25 * m, 0.0),
                    (-inner, -0.1, 0.0),
                    (-outer, -(0.02 + 0.88 * e), 0.5),
                    (0.0, 0.0, -0.3),
                    (outer, 0.02 + 0.88 * e, 0.3),
                    (inner, 0.1, -0.3),
                    (0.75, 0.75 - 0.25 * m, 0.0),
                    (1.0, 1.0 - 0.37 * m, 0.16 * m),
                ],
                input,
            )
        }
        // Foldback (fold).
        3 => ((1.0 + 7.0 * y) * input + 2.0 - 4.0 * x).sin(),
        // Chainsaw (octaver).
        4 => {
            let (a, p) = (0.1 + 0.9 * x, 0.1 + 0.6 * y);
            let q = ((input + 1.0) / (2.0 * p)).rem_euclid(1.0);
            if q < a {
                2.0 * q / a - 1.0
            } else {
                (2.0 - 2.0 * q) / (1.0 - a) - 1.0
            }
        }
        // Rectifuzz (rectify).
        5 => {
            let yf = (4.0 * x - 1.0).clamp(-1.0, 1.0);
            let x0 = (-0.999 + 1.998 * (x - 0.5)).clamp(-0.999, 0.0);
            let c = 1.6 * (y - 0.5);
            let points = [
                (-1.0, yf, 0.0),
                (x0, yf, 0.0),
                (0.0, 0.0, c),
                (0.999, 1.0, -c),
                (1.0, 1.0, 0.0),
            ];
            point_curve(&points, input)
        }
        // Cyclone (cross-fold).
        _ => {
            let a = 0.6 + 0.4 * x;
            let k = if y > 0.5 {
                0.03 + 7.94 * (y - 0.5)
            } else {
                -0.03 - 7.94 * (0.5 - y)
            };
            let damp = (-(20f32.powf(1.0 - a)).exp() * input * input).exp();
            input + damp * (TAU * k * input).sin()
        }
    }
}

pub struct WaveshaperState {
    params: [f32; 21],
    sample_rate: f32,
    pre_low: [Biquad; 2],
    pre_high: [Biquad; 2],
    post_bass: [Biquad; 2],
    post_mid: [Biquad; 2],
    post_treble: [Biquad; 2],
    dc_input: [f32; 2],
    dc_output: [f32; 2],
    dry_power: f32,
    wet_power: f32,
    phase: f64,
    phase_step: f64,
    /// The effective X and Y, smoothed.
    x: f32,
    y: f32,
}

impl WaveshaperState {
    pub fn new(params: &[f32], sample_rate: f32) -> Result<Self, WaveshaperError> {
        let mut state = Self {
            params: DEFAULT_PARAMS,
            sample_rate,
            pre_low: [Biquad::default(); 2],
            pre_high: [Biquad::default(); 2],
            post_bass: [Biquad::default(); 2],
            post_mid: [Biquad::default(); 2],
            post_treble: [Biquad::default(); 2],
            dc_input: [0.0; 2],
            dc_output: [0.0; 2],
            dry_power: 0.0,
            wet_power: 0.0,
            phase: 0.0,
            phase_step: 0.0,
            x: 0.0,
            y: 0.0,
        };
        state.refresh(params, sample_rate)?;
        state.reset();
        Ok(state)
    }

    /// The effective X and Y the shaper uses: p3 / p4 plus the LFO part, clamped, smoothed.
    pub fn effective_xy(&self) -> (f32, f32) {
        (self.x, self.y)
    }

    pub fn parameters(&self) -> &[f32; 21] {
        &self.params
    }

    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) -> Result<(), WaveshaperError> {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return Err(WaveshaperError::InvalidSampleRate);
        }
        if params.len() != 21
            || params
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(WaveshaperError::InvalidValue);
        }
        self.params.copy_from_slice(params);
        self.sample_rate = sample_rate;
        let high_cut = (20.0 * 1000.0_f32.powf(params[9])).min(sample_rate * 0.45);
        let low_cut = (20.0 * 1000.0_f32.powf(params[10])).min(sample_rate * 0.45);
        let mid_frequency = (500.0 * 4.0_f32.powf(params[12])).min(sample_rate * 0.45);
        for channel in 0..2 {
            self.pre_low[channel].set_filter_response(
                Response::LowPass,
                high_cut,
                0.7,
                sample_rate,
            );
            self.pre_high[channel].set_filter_response(
                Response::HighPass,
                low_cut,
                0.7,
                sample_rate,
            );
            self.post_bass[channel].set_parametric_response(
                Response::LowShelf,
                200.0_f32.min(sample_rate * 0.45),
                1.0,
                36.0 * params[11] - 18.0,
                sample_rate,
            );
            self.post_mid[channel].set_parametric_response(
                Response::Peak,
                mid_frequency,
                1.0,
                36.0 * params[13] - 18.0,
                sample_rate,
            );
            self.post_treble[channel].set_parametric_response(
                Response::HighShelf,
                6000.0_f32.min(sample_rate * 0.45),
                1.0,
                36.0 * params[14] - 18.0,
                sample_rate,
            );
        }
        Ok(())
    }

    pub fn prepare_block(
        &mut self,
        tempo_bpm: f64,
        start_beat: Option<f64>,
    ) -> Result<(), WaveshaperError> {
        if !tempo_bpm.is_finite()
            || tempo_bpm <= 0.0
            || start_beat.is_some_and(|beat| !beat.is_finite())
        {
            return Err(WaveshaperError::InvalidTempo);
        }
        if self.params[16] >= 0.5 {
            let periods = [0.125, 0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0];
            let index = (self.params[17] * 9.0) as usize;
            let period = periods[index];
            if let Some(beat) = start_beat {
                self.phase = (beat / period).rem_euclid(1.0);
            }
            self.phase_step = tempo_bpm / (60.0 * period * f64::from(self.sample_rate));
        } else {
            let rate = 0.02 + 9.9 * self.params[18].powi(3);
            self.phase_step = f64::from(rate / self.sample_rate);
        }
        Ok(())
    }

    pub fn process_frame(&mut self, left: f32, right: f32) -> (f32, f32) {
        if !left.is_finite() || !right.is_finite() {
            return (left, right);
        }
        if self.params[0] < 0.5 {
            return (left, right);
        }
        let wave = match (self.params[15] * 2.0).round() as usize {
            1 => (2.0 * self.phase - 1.0) as f32,
            2 => {
                if self.phase < 0.5 {
                    -1.0
                } else {
                    1.0
                }
            }
            _ => (std::f64::consts::TAU * self.phase).sin() as f32,
        };
        self.phase = (self.phase + self.phase_step).rem_euclid(1.0);
        // p3 / p4 plus LFO × X Mod·0.5 (Y Mod·0.5), clamped, each smoothed per sample with coefficient
        // 1 − 44.1/sr.
        // The coefficient is the pole, x ← c·x + (1 − c)·target, a 1000-sample time constant at 44.1 kHz.
        let pole = (1.0 - 44.1 / self.sample_rate).max(0.0);
        let target_x = (self.params[3] + 0.5 * self.params[19] * wave).clamp(0.0, 1.0);
        let target_y = (self.params[4] + 0.5 * self.params[20] * wave).clamp(0.0, 1.0);
        self.x = flush(pole * self.x + (1.0 - pole) * target_x);
        self.y = flush(pole * self.y + (1.0 - pole) * target_y);
        let index = shape_index(self.params[1]);
        let drive = 10.0_f32.powf(2.0 * self.params[5]);
        let spread = self.params[7].powi(4);
        let post_gain = 10.0_f32.powf((36.0 * self.params[6] - 18.0) / 20.0);
        let mut wet = [0.0; 2];
        for (channel, input) in [left, right].into_iter().enumerate() {
            let filtered = self.pre_high[channel].process(self.pre_low[channel].process(input));
            let shifted = drive * filtered + if channel == 0 { -spread } else { spread };
            let shaped = shape(index, self.x, self.y, shifted);
            let dc_blocked =
                flush(shaped - self.dc_input[channel] + 0.995 * self.dc_output[channel]);
            self.dc_input[channel] = shaped;
            self.dc_output[channel] = dc_blocked;
            let bass = self.post_bass[channel].process(dc_blocked);
            let mid = self.post_mid[channel].process(bass);
            wet[channel] = self.post_treble[channel].process(mid);
        }
        let dry_mean = 0.5 * (left * left + right * right);
        let wet_mean = 0.5 * (wet[0] * wet[0] + wet[1] * wet[1]);
        self.dry_power = flush(self.dry_power + 0.01 * (dry_mean - self.dry_power));
        self.wet_power = flush(self.wet_power + 0.01 * (wet_mean - self.wet_power));
        let auto_gain = if self.params[2] >= 0.5 {
            (self.dry_power / self.wet_power.max(0.004))
                .sqrt()
                .min(16.0)
        } else {
            1.0
        };
        let mix = self.params[8];
        let dry_weight = (FRAC_PI_2 * mix).cos();
        let wet_weight = (FRAC_PI_2 * mix).sin() * post_gain * auto_gain;
        (
            left * dry_weight + wet[0] * wet_weight,
            right * dry_weight + wet[1] * wet_weight,
        )
    }

    pub fn reset(&mut self) {
        for filter in self
            .pre_low
            .iter_mut()
            .chain(&mut self.pre_high)
            .chain(&mut self.post_bass)
            .chain(&mut self.post_mid)
            .chain(&mut self.post_treble)
        {
            filter.reset();
        }
        self.dc_input = [0.0; 2];
        self.dc_output = [0.0; 2];
        self.dry_power = 0.0;
        self.wet_power = 0.0;
        self.phase = 0.0;
        self.phase_step = 0.0;
        // A transport reset starts the smoothed X/Y at p3/p4, as with no LFO output known.
        (self.x, self.y) = (self.params[3], self.params[4]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dc_blocker_and_power_meters_settle_to_zero() {
        // Subnormal states never reach 0 and cost the audio thread up to 100× per operation.
        let mut params = DEFAULT_PARAMS;
        params[2] = 1.0;
        let mut state = WaveshaperState::new(&params, 8_000.0).unwrap();
        state.prepare_block(120.0, None).unwrap();
        for i in 0..480_000 {
            let input = if i < 400 { 0.5 } else { 0.0 };
            if i == 400 {
                // X and Y (p3, p4) went up with the burst, and now fall to 0.
                state.params[3] = 0.0;
                state.params[4] = 0.0;
            }
            state.process_frame(input, input);
        }
        assert_eq!(state.dc_output, [0.0; 2]);
        assert_eq!((state.dry_power, state.wet_power), (0.0, 0.0));
        assert_eq!((state.x, state.y), (0.0, 0.0));
    }

    #[test]
    fn stepped_selectors_follow_round_15() {
        // p1 Shape round(f32(6v)), so f32(5/12), f32(7/12) and f32(11/12)
        // round up; p17 LFO Rate trunc(f32(9v)) over 0.125 … 16 beats; p15 LFO Shape the act-ID form with n = 3.
        use crate::tests::step_edges as edges;
        assert_eq!(
            edges(12, shape_index),
            [
                vec![0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 6, 6],
                vec![0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6],
                vec![0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6],
            ]
        );
        let with = |index: usize, v: f32| {
            let mut params = DEFAULT_PARAMS;
            (params[index], params[16], params[19]) = (v, 1.0, 1.0);
            WaveshaperState::new(&params, 48_000.0).unwrap()
        };
        let step = |v: f32| {
            let mut state = with(17, v);
            state.prepare_block(120.0, None).unwrap();
            state.phase_step
        };
        let beats = [0.125, 0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0];
        let at = |k: usize| 120.0 / (60.0 * beats[k] * 48_000.0);
        assert_eq!(
            edges(9, step),
            [
                (0..9).map(at).collect::<Vec<_>>(),
                (0..10).map(at).collect(),
                (0..9).map(at).collect()
            ]
        );
        // The LFO at a quarter cycle (Sine 1, Saw −0.5, Square −1) moves X by one smoothing step toward 0.5 + wave/2.
        let x = |v: f32| {
            let mut state = with(15, v);
            state.phase = 0.25;
            state.process_frame(0.0, 0.0);
            state.x
        };
        let shapes = [0.0, 0.5, 1.0].map(x);
        assert!(shapes[0] != shapes[1] && shapes[1] != shapes[2] && shapes[0] != shapes[2]);
        let as_shape = |ks: &[usize]| ks.iter().map(|&k| shapes[k]).collect::<Vec<_>>();
        assert_eq!(
            edges(4, x),
            [
                as_shape(&[0, 1, 1, 2]),
                as_shape(&[0, 1, 1, 2, 2]),
                as_shape(&[0, 1, 1, 2])
            ]
        );
    }
}

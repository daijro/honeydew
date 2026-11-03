use crate::biquad::flush;

pub struct SpreaderState {
    sample_rate: f32,
    params: [f32; 5],
    state: Box<[f32; 4160]>,
    cursor: u32,
    max_delay: i32,
    delay_a: i32,
    delay_b: i32,
}

impl SpreaderState {
    pub fn new(params: &[f32], sample_rate: f32) -> Self {
        assert!(params.len() >= 5);
        assert!(sample_rate.is_finite() && sample_rate > 0.0);
        let mut state = Self {
            sample_rate,
            params: params[..5].try_into().unwrap(),
            state: Box::new([0.0; 4160]),
            cursor: 0,
            max_delay: 0,
            delay_a: 0,
            delay_b: 0,
        };
        state.configure();
        state
    }

    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        assert!(params.len() >= 5);
        assert!(sample_rate.is_finite() && sample_rate > 0.0);
        if sample_rate != self.sample_rate {
            *self = Self::new(params, sample_rate);
        } else {
            self.params.copy_from_slice(&params[..5]);
            self.set_controls();
        }
    }

    /// As `new`, in place: the audio thread calls it (no allocation).
    pub fn reset(&mut self) {
        self.state.fill(0.0);
        (self.cursor, self.max_delay, self.delay_a, self.delay_b) = (0, 0, 0, 0);
        self.configure();
    }

    #[allow(clippy::approx_constant)]
    fn configure(&mut self) {
        let rate = self.sample_rate.clamp(1.0, 192_000.0);
        let f = &mut self.state;
        f[3] = (-500.0 / rate).exp();
        f[4] = 1.0 - f[3];
        // tan(πf/Fs) of 15 kHz and of the low bypass below are clamped short of π/2 (ω = 2πf/Fs at 3, as the
        // Equalizer's): past Nyquist (16 or 22.05 kHz device rates) the tangent turns negative and the filters
        // unstable. Neither reaches 1.5 at 44.1 kHz and above.
        let reciprocal_tan = 1.0 / (47124.0 / rate).min(1.5).tan();
        self.max_delay = (rate * 0.0063492) as i32;
        f[2085] = (rate * 0.0063492).trunc();
        f[15] = 3.1416 / rate;
        f[2097] = 1.0 - reciprocal_tan;
        f[11] = 1.0 / (reciprocal_tan + 1.0);
        f[2086] = 1.0 / (rate * 0.02322).trunc();
        f[4152] = 0.5 * reciprocal_tan;
        self.set_controls();
    }

    fn set_controls(&mut self) {
        let f = &mut self.state;
        // The DSP's own Enable, not param 0: forced to 1. Its current value (f[7])
        // restarts at 0 with the rest of the state at every reset, so the dry input fades out over the smoothing.
        f[5] = 1.0;
        f[8] = self.params[4];
        f[12] = 2.0 * self.params[1];
        f[16] = 10.0 * 1000.0_f32.powf(self.params[2]);
        f[4157] = self.params[3];
    }

    #[allow(clippy::approx_constant)]
    pub fn process_frame(&mut self, left: f32, right: f32) -> (f32, f32) {
        let f = &mut self.state;
        let smooth = f[3];
        let step = f[4];
        let enable = f[5] * step + f[7] * smooth;
        let separation = f[8] * step + f[10] * smooth;
        let width = f[12] * step + f[14] * smooth;
        let low_bypass = f[16] * step + f[18] * smooth;
        f[6] = enable;
        f[9] = separation;
        f[13] = width;
        f[17] = low_bypass;

        let t = (f[15] * low_bypass).min(1.5).tan();
        let denominator = 1.0 / ((t + 1.4142) * t + 1.0);
        let a = ((enable * left - (f[22] + f[20] * (t + 1.4142))) * t) * denominator;
        let a1 = a + f[20];
        f[19] = f[20] + 2.0 * a;
        f[21] = f[22] + 2.0 * (a1 * t);
        let b = ((a1 * t + f[22]) - (f[30] + f[28] * (t + 1.4142))) * t * denominator;
        let c = ((enable * right - (f[26] + f[24] * (t + 1.4142))) * t) * denominator;
        let c1 = (c + f[24]) * t;
        let d = ((c1 + f[26]) - (f[34] + f[32] * (t + 1.4142))) * t * denominator;
        let b1 = (b + f[28]) * t;
        let d1 = (d + f[32]) * t;
        let left_low = b1 + f[30];
        let right_low = d1 + f[34];
        f[23] = f[24] + 2.0 * c;
        f[25] = f[26] + 2.0 * c1;
        f[27] = f[28] + 2.0 * b;
        f[29] = f[30] + 2.0 * b1;
        f[31] = f[32] + 2.0 * d;
        f[33] = f[34] + 2.0 * d1;
        let left_high = enable * left - a1 * 2.8284;
        let right_high = enable * right - (c + f[24]) * 2.8284;
        let side_a = left_high + right_high - (left_low + right_low);
        let side_b = right_low + left_high;
        let side_c = left_low + right_high;

        let slot = (self.cursor as usize) & 2047;
        f[36 + slot] = side_a;
        f[2100 + slot] = side_b - side_c;

        let target_delay = (f[2085] * width) as i32;
        let mut ramp_step = f[2088];
        let ramp = f[2090];
        if ramp_step == 0.0 {
            if ramp != 0.0 || self.delay_a == target_delay {
                if ramp == 1.0 && self.delay_b != target_delay {
                    ramp_step = -f[2086];
                }
            } else {
                ramp_step = f[2086];
            }
        } else if ramp >= 1.0 || ramp <= 0.0 {
            ramp_step = 0.0;
        }
        let next_ramp = (ramp + ramp_step).clamp(0.0, 1.0);
        let next_delay_a = if self.delay_b == target_delay || ramp < 1.0 {
            self.delay_a
        } else {
            target_delay
        };
        let next_delay_b = if self.delay_a == target_delay || ramp > 0.0 {
            self.delay_b
        } else {
            target_delay
        };
        let delay_a = next_delay_a.max(0).min(self.max_delay) as u32;
        let delay_b = next_delay_b.max(0).min(self.max_delay) as u32;
        let read_a = (self.cursor.wrapping_sub(delay_a) as usize) & 2047;
        let read_b = (self.cursor.wrapping_sub(delay_b) as usize) & 2047;
        self.cursor = self.cursor.wrapping_add(1);
        self.delay_a = next_delay_a;
        self.delay_b = next_delay_b;
        f[2087] = ramp_step;
        f[2089] = next_ramp;

        let delayed_side_b =
            (f[2100 + read_a] + (f[2100 + read_b] - f[2100 + read_a]) * next_ramp) * width;
        let delayed_side_a = f[36 + read_a] + (f[36 + read_b] - f[36 + read_a]) * next_ramp;
        let allpass_pole = f[2097];
        let allpass_gain = f[11];
        let allpass_feed = f[4152];
        let previous_delayed_b = f[4149];
        let previous_delayed_a = f[2096];
        let next_delayed_a = delayed_side_a * width;
        let a2 = f[4154] * allpass_pole + allpass_feed * (previous_delayed_b - delayed_side_b);
        let b2 = f[4151] * allpass_pole - 0.5 * (delayed_side_b + previous_delayed_b);
        let c2 = f[4156] * allpass_pole + allpass_feed * (previous_delayed_a - next_delayed_a);
        let d2 = -(b2 * allpass_gain);
        let e2 = -(a2 * allpass_gain);
        let f2 = -(c2 * allpass_gain);
        let g2 =
            -(f[2099] * allpass_pole - 0.5 * (next_delayed_a + previous_delayed_a)) * allpass_gain;
        let wide_b = 2.0 * d2 + e2 * 0.79621;
        let wide_a = ((d2 + g2) + (f2 + e2) * 0.39811) - 0.5 * ((wide_b + side_c) - side_b);
        let bass_mono = f[4157] * step + f[4159] * smooth;
        let bass = (wide_b + left_high + right_high) - (left_low + right_low)
            + bass_mono * (left_low + right_low);
        let stereo = separation * (wide_a + wide_a);
        let output_left =
            stereo + (1.0 - enable) * left + (1.0 - bass_mono) * left_low + 0.5 * bass;
        let output_right =
            (1.0 - enable) * right + (1.0 - bass_mono) * right_low - stereo + 0.5 * bass;

        f[4148] = delayed_side_b;
        f[2095] = next_delayed_a;
        f[4150] = d2;
        f[4153] = e2;
        f[4155] = f2;
        f[2098] = g2;
        f[4158] = bass_mono;
        for (previous, current) in [
            (7, 6),
            (10, 9),
            (14, 13),
            (18, 17),
            (20, 19),
            (22, 21),
            (24, 23),
            (26, 25),
            (28, 27),
            (30, 29),
            (32, 31),
            (34, 33),
            (2088, 2087),
            (2090, 2089),
            (2096, 2095),
            (2099, 2098),
            (4149, 4148),
            (4151, 4150),
            (4154, 4153),
            (4156, 4155),
            (4159, 4158),
        ] {
            f[previous] = flush(f[current]);
        }
        (output_left, output_right)
    }
}

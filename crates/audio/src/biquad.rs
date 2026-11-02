use std::f32::consts::PI;

/// Below this a filter or feedback state is silence (−600 dB) and is stored as 0, so a tail ends instead of circling as
/// subnormal floats, which never reach 0 and cost the audio thread up to 100× per operation.
const SILENCE: f32 = 1e-30;

pub(crate) fn flush(x: f32) -> f32 {
    if x.abs() < SILENCE { 0.0 } else { x }
}

#[derive(Clone, Copy)]
pub enum Response {
    LowPass,
    BandPass,
    HighPass,
    LowShelf,
    Peak,
    HighShelf,
}

#[derive(Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }
}

impl Biquad {
    pub fn set_parametric_response(
        &mut self,
        response: Response,
        frequency: f32,
        bandwidth: f32,
        gain_db: f32,
        sample_rate: f32,
    ) {
        if gain_db == 0.0 {
            self.b0 = 1.0;
            self.b1 = 0.0;
            self.b2 = 0.0;
            self.a1 = 0.0;
            self.a2 = 0.0;
            return;
        }
        let a = 10.0_f32.powf(gain_db * 0.05);
        let omega = (frequency * std::f32::consts::TAU.next_down() / sample_rate).clamp(0.0, 3.0);
        let (sin, cos) = omega.sin_cos();
        let alpha = sin * (0.346_573_6 * bandwidth * omega / sin).sinh();
        let (b0, b1, b2, a0, a1, a2) = match response {
            Response::Peak => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            Response::LowShelf | Response::HighShelf => {
                let shelf = 2.0 * a.sqrt() * alpha;
                let a_plus = a + 1.0;
                let a_minus = a - 1.0;
                if matches!(response, Response::LowShelf) {
                    (
                        a * (a_plus - a_minus * cos + shelf),
                        2.0 * a * (a_minus - a_plus * cos),
                        a * (a_plus - a_minus * cos - shelf),
                        a_plus + a_minus * cos + shelf,
                        -2.0 * (a_minus + a_plus * cos),
                        a_plus + a_minus * cos - shelf,
                    )
                } else {
                    (
                        a * (a_plus + a_minus * cos + shelf),
                        -2.0 * a * (a_minus + a_plus * cos),
                        a * (a_plus + a_minus * cos - shelf),
                        a_plus - a_minus * cos + shelf,
                        2.0 * (a_minus - a_plus * cos),
                        a_plus - a_minus * cos - shelf,
                    )
                }
            }
            _ => return,
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    pub fn set_filter_response(
        &mut self,
        response: Response,
        frequency: f32,
        resonance: f32,
        sample_rate: f32,
    ) {
        // Clamped below π as the Equalizer's is: a cutoff past Nyquist (the Filter's 20 kHz top at a 16 kHz device
        // rate, which has no floor) turned the poles unstable. No cutoff at 44.1 kHz and above reaches 3.
        let omega = (frequency * (2.0 * PI) / sample_rate).min(3.0);
        let (sin, cos) = omega.sin_cos();
        let (b0, b1, b2, a1, a2) = match response {
            Response::LowPass | Response::HighPass => {
                let high = matches!(response, Response::HighPass);
                let b0 = (1.0 + if high { cos } else { -cos }) / (sin / resonance + 2.0);
                let b1 = if high { -2.0 * b0 } else { 2.0 * b0 };
                (
                    b0,
                    b1,
                    b0,
                    -2.0 * resonance * cos / (sin + resonance),
                    (resonance - sin) / (sin + resonance),
                )
            }
            Response::BandPass => {
                let alpha = sin / (2.0 * resonance);
                let denominator = 1.0 + alpha;
                let b0 = 0.5 * sin / denominator;
                (
                    b0,
                    0.0,
                    -b0,
                    -2.0 * cos / denominator,
                    (1.0 - alpha) / denominator,
                )
            }
            _ => return,
        };
        self.b0 = if b0.abs() < 1e-9 { 0.0 } else { b0 };
        self.b1 = if b1.abs() < 1e-9 { 0.0 } else { b1 };
        self.b2 = if b2.abs() < 1e-9 { 0.0 } else { b2 };
        self.a1 = if a1.abs() < 1e-9 { 0.0 } else { a1 };
        self.a2 = if a2.abs() < 1e-9 { 0.0 } else { a2 };
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    pub fn process(&mut self, input: f32) -> f32 {
        let output = flush(
            self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2
                - self.a1 * self.y1
                - self.a2 * self.y2,
        );
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

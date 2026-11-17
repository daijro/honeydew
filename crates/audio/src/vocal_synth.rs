//! A small, realtime-safe formant voice for the Vocal Synth instrument.
//!
//! This first playable layer is a procedural vowel voice: a voiced glottal source is shaped by three formants, with
//! breath noise, vibrato and glide. It is a foundation, not yet a UTAU voicebank renderer. The instrument deliberately
//! performs no file access, allocation or locking on the audio thread.

use crate::ProcessContext;
use crate::instrument::Instrument;

const VOICES: usize = 8;
const VOWELS: [[f32; 3]; 5] = [
    [800.0, 1_150.0, 2_900.0], // a
    [500.0, 1_900.0, 2_600.0], // e
    [300.0, 2_200.0, 3_000.0], // i
    [500.0, 1_000.0, 2_400.0], // o
    [350.0, 900.0, 2_200.0],   // u
];

/// Prepared per-note controls, independent of module knobs. Vowel 0..4 is a/e/i/o/u;
/// 255 leaves vowel selection to the rack. Dynamics 0.5 is unity.
#[derive(Clone, Copy, Debug)]
pub struct VocalExpression {
    pub cents: f32,
    pub dynamics: f32,
    pub breathiness: f32,
    pub formant: f32,
    pub vowel: u8,
}

impl Default for VocalExpression {
    fn default() -> Self {
        Self {
            cents: 0.0,
            dynamics: 0.5,
            breathiness: 0.0,
            formant: 0.5,
            vowel: 255,
        }
    }
}

#[derive(Clone, Copy)]
struct FormantCoefficients {
    feed: f32,
    feedback: f32,
    radius_squared: f32,
}

impl FormantCoefficients {
    fn new(frequency: f32, bandwidth: f32, sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(8_000.0);
        let radius = (-std::f32::consts::PI * bandwidth / sample_rate).exp();
        let angle = std::f32::consts::TAU * frequency.min(sample_rate * 0.44) / sample_rate;
        Self {
            feed: 1.0 - radius,
            feedback: 2.0 * radius * angle.cos(),
            radius_squared: radius * radius,
        }
    }
}

#[derive(Clone, Copy)]
struct Resonator {
    y1: f32,
    y2: f32,
}

impl Resonator {
    const fn new() -> Self {
        Self { y1: 0.0, y2: 0.0 }
    }

    fn tick(&mut self, input: f32, coefficients: FormantCoefficients) -> f32 {
        let output = coefficients.feed * input + coefficients.feedback * self.y1
            - coefficients.radius_squared * self.y2;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

#[derive(Clone, Copy)]
struct Voice {
    active: bool,
    releasing: bool,
    key: u8,
    velocity: f32,
    pan: [f32; 2],
    expression: VocalExpression,
    expression_pitch: f32,
    expression_gain: f32,
    target_pitch: f32,
    phase: f32,
    envelope: f32,
    age: u64,
    pitch_hz: f32,
    target_hz: f32,
    vibrato_phase: f32,
    noise: u32,
    formants: [Resonator; 3],
}

impl Voice {
    const fn new() -> Self {
        Self {
            active: false,
            releasing: false,
            key: 0,
            velocity: 0.0,
            pan: [1.0; 2],
            expression: VocalExpression {
                cents: 0.0,
                dynamics: 0.5,
                breathiness: 0.0,
                formant: 0.5,
                vowel: 255,
            },
            expression_pitch: 1.0,
            expression_gain: 1.0,
            target_pitch: 1.0,
            phase: 0.0,
            envelope: 0.0,
            age: 0,
            pitch_hz: 440.0,
            target_hz: 440.0,
            vibrato_phase: 0.0,
            noise: 0x6d2b_79f5,
            formants: [Resonator::new(); 3],
        }
    }

    fn noise_sample(&mut self) -> f32 {
        // Per-voice xorshift32; no shared PRNG or lock on the audio thread.
        let mut x = self.noise;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.noise = x.max(1);
        (self.noise as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Honeydew's procedural Vocal Synth. Normalized parameters: Enable, Gender/Formant, Breathiness, Brightness,
/// Glide, Vibrato Rate, Vibrato Depth, and Vowel (a/e/i/o/u).
pub struct VocalSynth {
    params: [f32; 10],
    pitch_bend_ratio: f32,
    voices: [Voice; VOICES],
    age: u64,
}

impl VocalSynth {
    pub fn new(_sample_rate: f32) -> Self {
        Self {
            params: [1.0, 0.5, 0.08, 0.5, 0.0, 0.35, 0.0, 0.0, 0.0, 0.0],
            pitch_bend_ratio: 1.0,
            voices: [Voice::new(); VOICES],
            age: 0,
        }
    }

    fn start(&mut self, key: u8, velocity: f32, pan: f32) {
        if !velocity.is_finite() || !pan.is_finite() || key > 127 {
            return;
        }
        let previous_hz = self
            .voices
            .iter()
            .filter(|v| v.active && !v.releasing)
            .max_by_key(|v| v.age)
            .map(|v| v.pitch_hz);
        self.age = self.age.wrapping_add(1);
        let slot = self
            .voices
            .iter()
            .position(|v| !v.active)
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, voice)| voice.age)
                    .map_or(0, |(index, _)| index)
            });
        let hz = 440.0 * 2.0f32.powf((f32::from(key) - 69.0) / 12.0);
        let voice = &mut self.voices[slot];
        voice.active = true;
        voice.releasing = false;
        voice.key = key;
        voice.velocity = velocity.clamp(0.0, 1.0);
        let pan = pan.clamp(0.0, 1.0);
        voice.pan = [(1.0 - pan) * 2.0, pan * 2.0].map(|gain| gain.min(1.0));
        voice.expression = VocalExpression::default();
        voice.expression_pitch = 1.0;
        voice.expression_gain = 1.0;
        voice.target_pitch = 1.0;
        voice.phase = 0.0;
        voice.envelope = 0.0;
        voice.age = self.age;
        voice.pitch_hz = if self.params[4] > 0.001 {
            previous_hz.unwrap_or(hz)
        } else {
            hz
        };
        voice.target_hz = hz;
        voice.vibrato_phase = 0.0;
        voice.noise = (u32::from(key) + 1).wrapping_mul(0x9e37_79b9) ^ self.age as u32;
        voice.formants = [Resonator::new(); 3];
    }

    fn voice_sample(
        &mut self,
        voice_index: usize,
        sample_rate: f32,
        playing: bool,
        coefficients: [FormantCoefficients; 3],
    ) -> f32 {
        let [breath, brightness, glide, vibrato_rate, vibrato_depth] = [
            self.params[2],
            self.params[3],
            self.params[4],
            self.params[5],
            self.params[6],
        ];
        let voice = &mut self.voices[voice_index];
        if !voice.active {
            return 0.0;
        }
        let attack = (1.0 / (sample_rate * (0.008 + self.params[8] * 0.5))).max(0.00001);
        let release = (1.0 / (sample_rate * (0.12 + self.params[9] * 2.0))).max(0.00001);
        if voice.releasing {
            voice.envelope = (voice.envelope - release).max(0.0);
            if voice.envelope == 0.0 {
                voice.active = false;
                return 0.0;
            }
        } else {
            voice.envelope = (voice.envelope + attack).min(1.0);
        }

        let glide_frames = glide * glide * 0.65 * sample_rate;
        if glide_frames > 1.0 {
            voice.pitch_hz += (voice.target_hz - voice.pitch_hz) / glide_frames.max(1.0);
        } else {
            voice.pitch_hz = voice.target_hz;
        }
        let vib = if playing {
            (voice.vibrato_phase).sin() * vibrato_depth * 0.08
        } else {
            0.0
        };
        let smoothing = (1.0 / (sample_rate * 0.003)).min(1.0);
        voice.expression_pitch += (voice.target_pitch - voice.expression_pitch) * smoothing;
        voice.expression_gain +=
            (voice.expression.dynamics * 2.0 - voice.expression_gain) * smoothing;
        let frequency =
            (voice.pitch_hz * voice.expression_pitch * self.pitch_bend_ratio * (1.0 + vib))
                .clamp(30.0, sample_rate * 0.4);
        voice.vibrato_phase = (voice.vibrato_phase
            + std::f32::consts::TAU * vibrato_rate * 10.0 / sample_rate)
            .rem_euclid(std::f32::consts::TAU);
        voice.phase = (voice.phase + frequency / sample_rate).fract();

        // A bright, gently softened glottal pulse. The formant resonators supply vowel identity.
        let phase = voice.phase;
        let saw = 2.0 * phase - 1.0;
        let source = 0.82 * saw + brightness * 0.18 * (std::f32::consts::TAU * phase * 2.0).sin();
        let breath = (breath + voice.expression.breathiness).clamp(0.0, 1.0);
        let noise = voice.noise_sample() * breath * 0.22;
        let input = source * (1.0 - breath * 0.55) + noise;
        let mut formants = 0.0;
        for (i, coefficient) in coefficients.iter().copied().enumerate() {
            formants += voice.formants[i].tick(input, coefficient) * [1.0, 0.68, 0.35][i];
        }
        formants * voice.envelope * voice.velocity * voice.expression_gain * 0.42
    }
}

impl Instrument for VocalSynth {
    fn set_parameter(&mut self, index: usize, value: f32) {
        if !value.is_finite() {
            return;
        }
        if let Some(param) = self.params.get_mut(index) {
            *param = value.clamp(0.0, 1.0);
        }
    }

    fn note_on(&mut self, key: u8, velocity: f32, pan: f32) {
        self.start(key, velocity, pan);
    }

    fn set_vocal(&mut self, key: u8, mut expression: VocalExpression) {
        if ![
            expression.cents,
            expression.dynamics,
            expression.breathiness,
            expression.formant,
        ]
        .iter()
        .all(|v| v.is_finite())
        {
            return;
        }
        expression.cents = expression.cents.clamp(-1200.0, 1200.0);
        expression.dynamics = expression.dynamics.clamp(0.0, 1.0);
        expression.breathiness = expression.breathiness.clamp(0.0, 1.0);
        expression.formant = expression.formant.clamp(0.0, 1.0);
        if let Some(voice) = self
            .voices
            .iter_mut()
            .filter(|v| v.active && !v.releasing && v.key == key)
            .max_by_key(|v| v.age)
        {
            voice.expression = expression;
            voice.target_pitch = 2.0f32.powf(expression.cents / 1200.0);
        }
    }

    fn set_controller(&mut self, index: usize, value: f32) {
        if index == 4 && value.is_finite() {
            let semitones = (value.clamp(0.0, 1.0) * 2.0 - 1.0) * 2.0;
            self.pitch_bend_ratio = 2.0f32.powf(semitones / 12.0);
        }
    }

    fn note_off(&mut self, key: u8) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .filter(|v| v.active && !v.releasing && v.key == key)
            .min_by_key(|v| v.age)
        {
            voice.releasing = true;
        }
    }

    fn all_notes_off(&mut self) {
        for voice in &mut self.voices {
            if voice.active {
                voice.releasing = true;
            }
        }
    }

    fn reset(&mut self) {
        self.voices = [Voice::new(); VOICES];
        self.pitch_bend_ratio = 1.0;
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], context: ProcessContext) {
        if self.params[0] < 0.5 || !self.voices.iter().any(|voice| voice.active) {
            return;
        }
        let sample_rate = context.sample_rate.max(8_000.0);
        // At most eight coefficient sets per segment, never transcendental work per sample.
        let coefficients: [[FormantCoefficients; 3]; VOICES] = std::array::from_fn(|voice| {
            let expression = self.voices[voice].expression;
            let vowel = if expression.vowel < 5 {
                usize::from(expression.vowel)
            } else {
                ((self.params[7] * 4.0).round() as usize).min(VOWELS.len() - 1)
            };
            let centers = VOWELS[vowel];
            let scale = 2.0f32.powf((self.params[1] + expression.formant - 1.0) * 0.9);
            std::array::from_fn(|i| {
                let center = centers[i] * scale;
                FormantCoefficients::new(
                    center,
                    center * if i == 0 { 0.18 } else { 0.12 },
                    sample_rate,
                )
            })
        });
        for (l, r) in left.iter_mut().zip(right) {
            for (voice, coefficients) in coefficients.iter().copied().enumerate() {
                if !self.voices[voice].active {
                    continue;
                }
                let sample = self.voice_sample(voice, sample_rate, context.playing, coefficients);
                *l += sample * self.voices[voice].pan[0];
                *r += sample * self.voices[voice].pan[1];
            }
        }
    }

    fn set_tempo_bpm(&mut self, _bpm: f32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(rate: f32) -> ProcessContext {
        ProcessContext {
            sample_rate: rate,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        }
    }

    #[test]
    fn vocal_envelope_controls_change_attack_and_release() {
        let mut fast = VocalSynth::new(8000.0);
        let mut slow = VocalSynth::new(8000.0);
        slow.set_parameter(8, 1.0);
        slow.set_parameter(9, 1.0);
        fast.note_on(60, 1.0, 0.5);
        slow.note_on(60, 1.0, 0.5);
        for synth in [&mut fast, &mut slow] {
            synth.process(&mut [0.0; 800], &mut [0.0; 800], context(8000.0));
        }
        assert!(fast.voices[0].envelope > slow.voices[0].envelope);
        fast.note_off(60);
        slow.note_off(60);
        let before_fast = fast.voices[0].envelope;
        let before_slow = slow.voices[0].envelope;
        for synth in [&mut fast, &mut slow] {
            synth.process(&mut [0.0; 80], &mut [0.0; 80], context(8000.0));
        }
        assert!(before_fast - fast.voices[0].envelope > before_slow - slow.voices[0].envelope);
    }

    #[test]
    fn per_note_expression_is_isolated_and_changes_the_audio() {
        let mut synth = VocalSynth::new(48_000.0);
        synth.note_on(60, 1.0, 0.5);
        synth.note_on(64, 1.0, 0.5);
        synth.set_vocal(
            60,
            VocalExpression {
                cents: 1200.0,
                dynamics: 0.0,
                breathiness: 0.8,
                formant: 0.2,
                vowel: 1,
            },
        );
        assert_eq!(synth.voices[0].target_pitch, 2.0);
        assert_eq!(synth.voices[1].target_pitch, 1.0);
        assert_eq!(synth.voices[1].expression.vowel, 255);
        let mut left = [0.0; 512];
        let mut right = [0.0; 512];
        synth.process(&mut left, &mut right, context(48_000.0));
        assert!(left.iter().all(|v| v.is_finite()));
        assert!(synth.voices[0].expression_gain < 0.1);
        assert_eq!(synth.voices[1].expression_gain, 1.0);
    }

    #[test]
    fn overlapping_same_key_notes_each_receive_their_own_release() {
        let mut synth = VocalSynth::new(48_000.0);
        synth.note_on(60, 1.0, 0.5);
        synth.note_on(60, 1.0, 0.5);
        synth.note_off(60);
        synth.note_off(60);
        assert!(
            synth
                .voices
                .iter()
                .filter(|v| v.active)
                .all(|v| v.releasing)
        );
    }

    #[test]
    fn respects_pan_and_ignores_nonfinite_controls() {
        let mut synth = VocalSynth::new(48_000.0);
        synth.set_parameter(7, f32::NAN);
        synth.set_controller(4, f32::NAN);
        synth.note_on(60, f32::NAN, 0.5);
        assert!(synth.voices.iter().all(|v| !v.active));
        synth.note_on(60, 1.0, 0.0);
        let mut left = [0.0; 512];
        let mut right = [0.0; 512];
        synth.process(&mut left, &mut right, context(48_000.0));
        assert!(left.iter().all(|v| v.is_finite()));
        assert!(left.iter().any(|v| *v != 0.0));
        assert!(right.iter().all(|v| *v == 0.0));
        synth.reset();
        assert_eq!(synth.pitch_bend_ratio, 1.0);
    }

    #[test]
    fn glide_starts_at_the_previous_sounding_pitch_not_a_reused_slot() {
        let mut synth = VocalSynth::new(48_000.0);
        synth.set_parameter(4, 0.5);
        synth.note_on(60, 1.0, 0.5);
        assert!((synth.voices[0].pitch_hz - synth.voices[0].target_hz).abs() < 1e-5);
        synth.note_on(72, 1.0, 0.5);
        assert_eq!(synth.voices[1].pitch_hz, synth.voices[0].pitch_hz);
    }

    #[test]
    fn renders_a_voiced_sample_and_releases_without_allocating_voice_state() {
        let mut synth = VocalSynth::new(48_000.0);
        synth.note_on(60, 0.8, 0.5);
        let mut left = [0.0; 512];
        let mut right = [0.0; 512];
        synth.process(&mut left, &mut right, context(48_000.0));
        assert!(left.iter().any(|sample| sample.abs() > 0.001));
        assert_eq!(left, right);
        synth.note_off(60);
        for _ in 0..20 {
            left.fill(0.0);
            right.fill(0.0);
            synth.process(&mut left, &mut right, context(48_000.0));
        }
        assert!(synth.voices.iter().all(|voice| !voice.active));
    }

    #[test]
    fn pitch_bend_controller_maps_the_normalized_range_to_two_semitones() {
        let mut synth = VocalSynth::new(48_000.0);
        synth.set_controller(4, 1.0);
        assert!((synth.pitch_bend_ratio - 2.0f32.powf(2.0 / 12.0)).abs() < 1e-6);
        synth.set_controller(4, 0.5);
        assert!((synth.pitch_bend_ratio - 1.0).abs() < 1e-6);
        synth.set_controller(4, 0.0);
        assert!((synth.pitch_bend_ratio - 2.0f32.powf(-2.0 / 12.0)).abs() < 1e-6);
    }

    #[test]
    fn controls_are_clamped_and_vowel_selection_changes_the_timbre() {
        let mut a = VocalSynth::new(44_100.0);
        let mut b = VocalSynth::new(44_100.0);
        a.set_parameter(7, 0.0);
        b.set_parameter(7, 1.0);
        a.note_on(60, 1.0, 0.5);
        b.note_on(60, 1.0, 0.5);
        let mut al = [0.0; 2048];
        let mut ar = [0.0; 2048];
        let mut bl = [0.0; 2048];
        let mut br = [0.0; 2048];
        a.process(&mut al, &mut ar, context(44_100.0));
        b.process(&mut bl, &mut br, context(44_100.0));
        assert_ne!(al, bl);
    }
}

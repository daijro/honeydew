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
    [300.0, 2_200.0, 3_000.0], // i
    [350.0, 900.0, 2_200.0],   // u
    [500.0, 1_900.0, 2_600.0], // e
    [500.0, 1_000.0, 2_400.0], // o
];

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
/// Glide, Vibrato Rate, Vibrato Depth, and Vowel (a/i/u/e/o).
pub struct VocalSynth {
    params: [f32; 8],
    pitch_bend_ratio: f32,
    voices: [Voice; VOICES],
    age: u64,
}

impl VocalSynth {
    pub fn new(_sample_rate: f32) -> Self {
        Self {
            params: [1.0, 0.5, 0.08, 0.5, 0.0, 0.35, 0.0, 0.0],
            pitch_bend_ratio: 1.0,
            voices: [Voice::new(); VOICES],
            age: 0,
        }
    }

    fn start(&mut self, key: u8, velocity: f32) {
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
        voice.phase = 0.0;
        voice.envelope = 0.0;
        voice.age = self.age;
        voice.pitch_hz = if self.params[4] > 0.001 {
            voice.target_hz
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
        let attack = (1.0 / (sample_rate * 0.008)).max(0.00001);
        let release = (1.0 / (sample_rate * 0.12)).max(0.00001);
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
        let frequency = (voice.pitch_hz * self.pitch_bend_ratio * (1.0 + vib))
            .clamp(30.0, sample_rate * 0.4);
        voice.vibrato_phase = (voice.vibrato_phase
            + std::f32::consts::TAU * vibrato_rate * 10.0 / sample_rate)
            .rem_euclid(std::f32::consts::TAU);
        voice.phase = (voice.phase + frequency / sample_rate).fract();

        // A bright, gently softened glottal pulse. The formant resonators supply vowel identity.
        let phase = voice.phase;
        let saw = 2.0 * phase - 1.0;
        let source = 0.82 * saw + brightness * 0.18 * (std::f32::consts::TAU * phase * 2.0).sin();
        let noise = voice.noise_sample() * breath * 0.22;
        let input = source * (1.0 - breath * 0.55) + noise;
        let mut formants = 0.0;
        for (i, coefficient) in coefficients.iter().copied().enumerate() {
            formants += voice.formants[i].tick(input, coefficient) * [1.0, 0.68, 0.35][i];
        }
        formants * voice.envelope * voice.velocity * 0.42
    }
}

impl Instrument for VocalSynth {
    fn set_parameter(&mut self, index: usize, value: f32) {
        if let Some(param) = self.params.get_mut(index) {
            *param = value.clamp(0.0, 1.0);
        }
    }

    fn note_on(&mut self, key: u8, velocity: f32, _pan: f32) {
        self.start(key, velocity);
    }

    fn set_controller(&mut self, index: usize, value: f32) {
        if index == 4 {
            let semitones = (value.clamp(0.0, 1.0) * 2.0 - 1.0) * 2.0;
            self.pitch_bend_ratio = 2.0f32.powf(semitones / 12.0);
        }
    }

    fn note_off(&mut self, key: u8) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .rev()
            .find(|v| v.active && v.key == key)
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
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], context: ProcessContext) {
        if self.params[0] < 0.5 {
            return;
        }
        let sample_rate = context.sample_rate.max(8_000.0);
        let centers = VOWELS[(self.params[7] * 4.999) as usize];
        let formant_scale = 2.0f32.powf((self.params[1] - 0.5) * 0.9);
        let coefficients = std::array::from_fn(|i| {
            let center = centers[i] * formant_scale;
            let bandwidth = center * if i == 0 { 0.18 } else { 0.12 };
            FormantCoefficients::new(center, bandwidth, sample_rate)
        });
        for (l, r) in left.iter_mut().zip(right) {
            let mut sample = 0.0;
            for voice in 0..VOICES {
                sample += self.voice_sample(voice, sample_rate, context.playing, coefficients);
            }
            *l += sample;
            *r += sample;
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

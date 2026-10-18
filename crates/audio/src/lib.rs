pub mod analysis;
pub mod automation;
mod biquad;
pub mod builtin_drums;
pub mod builtin_loops;
pub mod builtin_shapes;
mod comb;
pub mod device;
pub mod drums;
pub mod dw_sampler;
pub mod dwp;
pub mod flac_writer;
pub mod gm_waveshapes;
pub mod gmsynth;
pub mod input;
pub mod instrument;
pub mod metronome;
pub mod minisynth;
pub mod mixer;
pub mod mp3_writer;
mod multiband;
pub mod multifx;
mod multifx_delay;
mod multifx_reverb;
pub mod note_effects;
pub mod ogg_writer;
pub mod pitcher;
pub mod pitcher2;
pub mod playback;
mod reverbs;
pub mod sample;
pub mod sample_edit;
pub mod sequencer;
pub mod slicer;
pub mod soundfont;
mod spreader;
pub mod supersaw;
mod tape_stop;
pub mod three_osc;
pub mod transistor_bass;
pub mod voice;
pub mod wav_writer;
pub mod waveshaper;

use biquad::{Biquad, Response, flush};
use comb::CombState;
use multiband::MultibandState;
use multifx::MultiFxState;
use pitcher::PitcherState;
use pitcher2::Pitcher2State;
use reverbs::ReverbState;
use spreader::SpreaderState;
use tape_stop::TapeStopState;
use waveshaper::WaveshaperState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectInfo {
    pub id: u16,
    pub name: &'static str,
    pub parameter_count: usize,
}

pub fn effect_info(id: u16) -> Option<EffectInfo> {
    let (name, parameter_count) = match id {
        3 => ("Equalizer", 21),
        4 => ("MultiFX", 7),
        5 => ("Limiter (legacy)", 4),
        6 => ("Distortion", 7),
        7 => ("Graphic EQ", 9),
        8 => ("Compressor", 10),
        9 => ("Filter", 5),
        10 => ("Autoduck (legacy)", 4),
        11 => ("Chorus", 5),
        12 => ("Reverb (legacy)", 22),
        13 => ("Flanger", 12),
        14 => ("Phaser", 10),
        15 => ("Stereoizer", 7),
        16 => ("Trance Delay", 7),
        18 => ("Tape Delay", 5),
        20 => ("Leveller", 3),
        24 => ("Tuner", 1),
        25 => ("Pitcher", 23),
        26 => ("Analyzer", 1),
        27 => ("Gate", 9),
        28 => ("Limiter", 5),
        29 => ("Autoduck", 8),
        30 => ("Tuned EQ", 11),
        32 => ("Reverb", 16),
        34 => ("Spacer", 8),
        35 => ("Reverb 2", 19),
        36 => ("Multiband Compressor", 31),
        40 => ("Comb Filter", 12),
        41 => ("Waveshaper", 21),
        42 => ("Tape Stop", 8),
        43 => ("Wow & Flutter", 11),
        44 => ("Spreader", 5),
        45 => ("Pitcher 2", 27),
        46 => ("Tremolo", 12),
        _ => return None,
    };
    Some(EffectInfo {
        id,
        name,
        parameter_count,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcessContext {
    pub sample_rate: f32,
    pub tempo_bpm: f32,
    /// The song position of the block's first frame, in beats.
    pub start_beat: f64,
    /// The transport is running, so `start_beat` advances with the frames.
    pub playing: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterSnapshot {
    pub peak_left: f32,
    pub peak_right: f32,
    pub gain_reduction_db: f32,
    /// The peak max(|L|, |R|) of what reached the module, before it ran (the Compressor's IN bar).
    pub input_peak: f32,
}

impl MeterSnapshot {
    /// Folds `other` in: the larger of each value, so two stretches read as one.
    pub fn hold(&mut self, other: MeterSnapshot) {
        self.peak_left = self.peak_left.max(other.peak_left);
        self.peak_right = self.peak_right.max(other.peak_right);
        self.gain_reduction_db = self.gain_reduction_db.max(other.gain_reduction_db);
        self.input_peak = self.input_peak.max(other.input_peak);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectError {
    UnknownEffect(u16),
    UnsupportedEffect(u16),
    InvalidParameter(usize),
    InvalidCommonParameter(u16),
    InvalidValue,
    InvalidSampleRate,
    InvalidContext,
    MismatchedBuffers,
    UnsupportedNoteInput(u16),
    UnsupportedBandSelection(u16),
    UnsupportedPitcherNotes(u16),
    UnsupportedAnalyzerSetting(u16),
}

/// The rack wrapper around every module, effects and instruments alike:
/// Post Gain, Mix and Wet Pan, with their smoothing.
pub struct CommonModule {
    /// Post Gain's target amplitude, `10^((−20+40v)/20)`.
    target_gain: f32,
    /// Targets, normalized.
    mix: f32,
    wet_pan: f32,
    /// Smoothed current values.
    gain: f32,
    current_mix: f32,
    current_pan: f32,
    /// No block has run yet: a value set now (a saved one, applied at build before any render) is where
    /// the module starts, not a glide. The hosts clear it at the top of each block, whether or not a frame runs
    /// (a bypassed module, or an instrument's silent part); so does a frame.
    fresh: bool,
}

impl Default for CommonModule {
    fn default() -> Self {
        Self {
            target_gain: 1.0,
            mix: 1.0,
            wet_pan: 0.5,
            gain: 1.0,
            current_mix: 1.0,
            current_pan: 0.5,
            fresh: true,
        }
    }
}

impl CommonModule {
    /// Sets common param `index`: 0x7FC0 Post Gain, 0x7FC1 Mix, 0x7FC2 Wet Pan.
    pub fn set_parameter(&mut self, index: u16, value: f32) -> Result<(), EffectError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(EffectError::InvalidValue);
        }
        match index {
            0x7fc0 => self.target_gain = db_to_amplitude(-20.0 + 40.0 * value),
            0x7fc1 => self.mix = value,
            0x7fc2 => self.wet_pan = value,
            _ => return Err(EffectError::InvalidCommonParameter(index)),
        }
        if self.fresh {
            (self.gain, self.current_mix, self.current_pan) =
                (self.target_gain, self.mix, self.wet_pan);
        }
        Ok(())
    }

    /// Whether the module's processor runs: Enable on and Mix ≠ 0. Otherwise it is bypassed and dry passes unchanged.
    pub fn runs(&self, enabled: bool) -> bool {
        enabled && self.mix != 0.0
    }

    /// One frame, while [`Self::runs`]: `wet` × Post Gain, mixed into `dry` by Mix and Wet Pan.
    pub fn frame(&mut self, (dry_l, dry_r): (f32, f32), (wet_l, wet_r): (f32, f32)) -> (f32, f32) {
        self.fresh = false;
        if self.gain != self.target_gain {
            self.gain += (self.target_gain - self.gain) * 0.01;
            if (self.gain - self.target_gain).abs() < 0.001 {
                self.gain = self.target_gain;
            }
        }
        let (wet_l, wet_r) = (wet_l * self.gain, wet_r * self.gain);
        self.current_mix += (self.mix - self.current_mix) * 0.001;
        self.current_pan = flush(self.current_pan + (self.wet_pan - self.current_pan) * 0.001);
        if self.current_mix == 1.0 && self.current_pan == 0.5 {
            return (wet_l, wet_r);
        }
        let wet_left = (2.0 - 2.0 * self.current_pan).min(1.0);
        let wet_right = (2.0 * self.current_pan).min(1.0);
        (
            dry_l + (wet_l - dry_l) * self.current_mix * wet_left,
            dry_r + (wet_r - dry_r) * self.current_mix * wet_right,
        )
    }
}

/// An audio effect the host provides from outside the engine (a third-party plugin, desktop only). It processes
/// whole blocks in place; the module's Enable (param 0) and common params wrap it as they wrap the built-in effects
/// ([`AudioEffect::external`]). Its methods run on the audio thread: none may block or allocate.
pub trait ExternalEffect: Send {
    /// Param `index` ≥ 1 is now `value`, normalized (param 0, Enable, is the module's).
    fn set_parameter(&mut self, index: usize, value: f32);
    /// Processes one block in place.
    fn process(&mut self, left: &mut [f32], right: &mut [f32], context: ProcessContext);
    /// Clears its tails and delay lines (ResetSound).
    fn reset(&mut self);
}

/// An external effect and its dry copy, for the common params' mix.
struct External {
    effect: Box<dyn ExternalEffect>,
    dry_left: Vec<f32>,
    dry_right: Vec<f32>,
}

pub struct AudioEffect {
    id: u16,
    params: Vec<f32>,
    common: CommonModule,
    processor: Processor,
    sample_rate: f32,
    meter: MeterSnapshot,
    probe: analysis::Probe,
    external: Option<External>,
}

impl AudioEffect {
    pub fn new(id: u16, sample_rate: f32) -> Result<Self, EffectError> {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return Err(EffectError::InvalidSampleRate);
        }
        let info = effect_info(id).ok_or(EffectError::UnknownEffect(id))?;
        let (params, processor) = match id {
            3 => {
                let mut params = vec![0.5; info.parameter_count];
                params[0] = 1.0;
                for (i, frequency) in [0.2, 0.4, 0.6, 0.8].into_iter().enumerate() {
                    params[2 + 4 * i] = frequency;
                    params[4 + 4 * i] = 0.5;
                    params[17 + i] = 0.0;
                }
                (params, Processor::Eq(EqState::new(EqKind::Param)))
            }
            4 => {
                let params = [1.0, 0.0, 0.5, 0.5, 0.61, 0.0, 0.0];
                (
                    params.to_vec(),
                    Processor::MultiFx(Box::new(MultiFxState::new(&params, sample_rate))),
                )
            }
            6 => (
                vec![1.0, 0.25, 0.0, 1.0, 0.5, 0.0, 0.5],
                Processor::Distortion(DistortionState::default()),
            ),
            5 => (
                vec![1.0, 1.0, 0.7, 2.0 / 3.0],
                Processor::LegacyLimiter(LegacyLimiterState::default()),
            ),
            7 => {
                let mut params = vec![0.5; info.parameter_count];
                params[0] = 1.0;
                (params, Processor::Eq(EqState::new(EqKind::Graphic)))
            }
            8 => (
                vec![1.0, 0.499, 0.0, 0.6, 0.6, 0.5001, 0.0, 1.0, 0.5, 0.0],
                Processor::Compressor(CompressorState::default()),
            ),
            9 => (
                vec![1.0, 0.7, 0.0, 0.0, 0.5],
                Processor::Filter(FilterState::default()),
            ),
            10 => (
                vec![1.0, 0.501, 0.1, 0.501],
                Processor::AutoDuck(AutoDuckState::new(false)),
            ),
            11 => (
                vec![1.0, 0.4, 0.6, 1.0, 0.6],
                Processor::Chorus(ChorusState::default()),
            ),
            12 => {
                let params = vec![
                    1.0, 0.4, 0.8, 0.4, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.25, 0.0, 0.0, 0.0,
                    0.0, 0.0, 0.0, 0.0, 0.499, 0.0, 0.0,
                ];
                (
                    params.clone(),
                    Processor::Reverb(ReverbState::new(id, &params, sample_rate)),
                )
            }
            13 => (
                vec![
                    1.0, 0.4, 0.0, 0.55, 0.0, 1.0, 0.25, 0.75, 1.0, 0.0, 1.0, 1.0,
                ],
                Processor::Flanger(FlangerState::default()),
            ),
            14 => (
                vec![1.0, 0.1122, 0.25, 0.1, 0.25, 1.0, 1.0, 0.0, 0.0, 1.0],
                Processor::Phaser(PhaserState::default()),
            ),
            15 => (
                vec![1.0, 0.7, 0.52, 0.75, 0.5, 0.5, 0.0],
                Processor::Stereoizer(StereoizerState::default()),
            ),
            16 => (
                vec![1.0, 0.775, 0.4, 1.0, 0.5, 0.0, 0.5],
                Processor::TranceDelay(TranceDelayState::default()),
            ),
            18 => (
                vec![1.0, 0.6, 0.6, 0.5, 0.4],
                Processor::TapeDelay(TapeDelayState::default()),
            ),
            20 => (
                vec![1.0, 0.738, 0.5],
                Processor::Leveller(LevellerState::new(0.738)),
            ),
            24 | 26 => (vec![1.0], Processor::Monitor),
            25 => {
                let params = pitcher::DEFAULT_PARAMS.to_vec();
                let state = PitcherState::new(&params, sample_rate)
                    .map_err(|_| EffectError::InvalidSampleRate)?;
                (params, Processor::Pitcher(Box::new(state)))
            }
            27 => (
                vec![1.0, 0.0, 0.0, 0.01, 0.0, 0.25, 1.0, 1.0, 0.25],
                Processor::Gate(Box::default()),
            ),
            28 => (
                vec![1.0, 0.5, 0.5, 0.2, 1.0],
                Processor::LookaheadLimiter(LookaheadLimiterState::default()),
            ),
            29 => (
                vec![1.0, 0.501, 0.1, 0.501, 1.0, 0.5, 0.0, 0.0],
                Processor::AutoDuck(AutoDuckState::new(true)),
            ),
            30 => {
                let mut params = vec![0.5; info.parameter_count];
                params[0] = 1.0;
                params[9] = 0.0;
                params[10] = 0.6;
                (params, Processor::Eq(EqState::new(EqKind::Tuned)))
            }
            32 => {
                let params = vec![
                    1.0, 0.4, 0.8, 0.4, 0.5, 0.25, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.499, 0.0, 1.0,
                    1.0,
                ];
                (
                    params.clone(),
                    Processor::Reverb(ReverbState::new(id, &params, sample_rate)),
                )
            }
            34 => (
                vec![1.0, 0.5, 0.5, 0.5, 0.0, 0.5, 0.0, 0.0],
                Processor::Spacer(SpacerState::default()),
            ),
            35 => {
                let params = vec![
                    1.0, 0.16, 0.0187, 0.5, 0.068, 1.0, 0.16, 0.8, 0.8, 0.56, 0.0, 0.0, 0.0, 0.0,
                    0.28, 0.5, 0.33, 0.2405, 0.5,
                ];
                (
                    params.clone(),
                    Processor::Reverb(ReverbState::new(id, &params, sample_rate)),
                )
            }
            36 => {
                let params = vec![
                    1.0, 0.5, 0.3, 0.7, 0.0, 0.0, 0.5, 1.0, 0.0, 0.3, 0.3, 0.5, 0.0, 0.5, 1.0, 0.0,
                    0.3, 0.3, 0.5, 0.0, 0.5, 1.0, 0.0, 0.3, 0.3, 0.5, 0.0, 0.5, 0.5, 0.5, 0.0,
                ];
                (
                    params.clone(),
                    Processor::Multiband(Box::new(MultibandState::new(&params, sample_rate))),
                )
            }
            40 => (
                vec![
                    1.0, 0.0, 0.33, 0.8, 0.75, 0.5, 0.25, 1.0, 0.0, 0.5, 0.5, 0.4,
                ],
                Processor::Comb(Box::default()),
            ),
            41 => {
                let mut params = waveshaper::DEFAULT_PARAMS.to_vec();
                params[6] = 1.0 / 6.0;
                (
                    params.clone(),
                    Processor::Waveshaper(WaveshaperState::new(&params, sample_rate).unwrap()),
                )
            }
            42 => {
                let params = vec![1.0, 0.2, 0.2, 1.0 / 3.0, 1.0 / 3.0, 1.0, 0.5, 0.0];
                (
                    params.clone(),
                    Processor::TapeStop(TapeStopState::new(&params, sample_rate, 120.0)),
                )
            }
            43 => (
                vec![
                    1.0, 0.1846, 0.2, 0.25, 0.0668, 0.1, 0.25, 0.0, 0.0895, 0.8, 0.25,
                ],
                Processor::WowFlutter(WowFlutterState::default()),
            ),
            44 => {
                let params = vec![1.0, 0.0, 0.0, 0.0, 0.5];
                (
                    params.clone(),
                    Processor::Spreader(SpreaderState::new(&params, sample_rate)),
                )
            }
            45 => {
                let params = pitcher2::DEFAULT_PARAMS.to_vec();
                let state = Pitcher2State::new(&params, sample_rate)
                    .map_err(|_| EffectError::InvalidSampleRate)?;
                (params, Processor::Pitcher2(Box::new(state)))
            }
            46 => (
                vec![
                    1.0, 0.0455, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 0.5, 0.5,
                ],
                Processor::Tremolo(TremoloState::default()),
            ),
            _ => return Err(EffectError::UnsupportedEffect(id)),
        };
        let mut effect = Self {
            id,
            params,
            common: CommonModule::default(),
            processor,
            sample_rate,
            meter: MeterSnapshot::default(),
            probe: analysis::Probe::new(id, sample_rate),
            external: None,
        };
        effect.refresh_coefficients();
        effect.reset();
        Ok(effect)
    }

    /// Module type `id` processed by `effect` ([`ExternalEffect`]), with `params` params (Enable and the effect's),
    /// all at 0 but Enable until set, and room for blocks of `max_block` frames (longer ones run in parts). Nothing
    /// is reset: the effect may run on in the engine this one replaces.
    pub fn external(
        id: u16,
        sample_rate: f32,
        effect: Box<dyn ExternalEffect>,
        params: usize,
        max_block: usize,
    ) -> Result<Self, EffectError> {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return Err(EffectError::InvalidSampleRate);
        }
        let mut values = vec![0.0; params.max(1)];
        values[0] = 1.0;
        Ok(Self {
            id,
            params: values,
            common: CommonModule::default(),
            processor: Processor::Monitor,
            sample_rate,
            meter: MeterSnapshot::default(),
            probe: analysis::Probe::new(id, sample_rate),
            external: Some(External {
                effect,
                dry_left: vec![0.0; max_block.max(1)],
                dry_right: vec![0.0; max_block.max(1)],
            }),
        })
    }

    /// An [`ExternalEffect`]'s module ([`Self::external`]).
    pub fn is_external(&self) -> bool {
        self.external.is_some()
    }

    pub fn type_id(&self) -> u16 {
        self.id
    }

    /// No Spreader latency, no per-channel sum of enabled effects: nothing reads them.
    pub fn latency_samples(&self) -> usize {
        match &self.processor {
            Processor::LookaheadLimiter(state) => state.delay_left.len(),
            Processor::Gate(_) => 255,
            Processor::Pitcher(state) => state.latency_samples(),
            Processor::Pitcher2(state) => state.latency_samples(),
            _ => 0,
        }
    }

    pub fn configure_sample_rate(&mut self, sample_rate: f32) -> Result<(), EffectError> {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return Err(EffectError::InvalidSampleRate);
        }
        if matches!(self.id, 25 | 45) && sample_rate < 2000.0 {
            return Err(EffectError::InvalidSampleRate);
        }
        if sample_rate != self.sample_rate {
            self.sample_rate = sample_rate;
            // ponytail: the Analyzer's settings go back to a new module's here; no host changes the rate of a built
            // effect yet (the engine rebuilds and re-applies them), carry them over if one does.
            self.probe = analysis::Probe::new(self.id, sample_rate);
            self.refresh_coefficients();
            self.reset();
        }
        Ok(())
    }

    /// Sizes the tempo-dependent buffers (Tape Stop's ring) for `tempo_bpm`; a new effect
    /// assumes 120 BPM. Like [`Self::configure_sample_rate`] it may allocate, so the host calls it off the audio
    /// thread, at build with the song tempo. Nothing resizes on the audio thread: an edit's engine hands its buffers to
    /// the running effect it takes over ([`Self::take_buffers`]), and a Tape Stop ramp at a dragged tempo slower than
    /// its ring was sized for is cut to the ring (`TapeStopState::refresh`).
    pub fn configure_tempo(&mut self, tempo_bpm: f32) -> Result<(), EffectError> {
        if !tempo_bpm.is_finite() || tempo_bpm <= 0.0 {
            return Err(EffectError::InvalidContext);
        }
        if let Processor::TapeStop(state) = &mut self.processor {
            state.prepare(tempo_bpm);
        }
        Ok(())
    }

    /// Takes over what `built`, this effect built again for an edit's engine, sized off the audio thread for that
    /// engine's tempo: a Tape Stop's longer ring (`TapeStopState::take_ring`), its running ramp kept. Allocates nothing.
    pub fn take_buffers(&mut self, built: &mut AudioEffect) {
        if let (Processor::TapeStop(state), Processor::TapeStop(theirs)) =
            (&mut self.processor, &mut built.processor)
        {
            state.take_ring(theirs);
        }
    }

    pub fn parameter(&self, index: usize) -> Option<f32> {
        self.params.get(index).copied()
    }

    pub fn set_parameter(&mut self, index: usize, value: f32) -> Result<(), EffectError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(EffectError::InvalidValue);
        }
        let parameter = self
            .params
            .get_mut(index)
            .ok_or(EffectError::InvalidParameter(index))?;
        *parameter = value;
        if let Some(external) = &mut self.external {
            if index > 0 {
                external.effect.set_parameter(index, value);
            }
            return Ok(());
        }
        if self.id == 46 {
            if index == 3 && self.params[4] > value {
                self.params[4] = value;
            } else if index == 4 && self.params[3] < value {
                self.params[3] = value;
            }
        }
        self.refresh_coefficients();
        Ok(())
    }

    pub fn set_common_parameter(&mut self, index: u16, value: f32) -> Result<(), EffectError> {
        self.common.set_parameter(index, value)
    }

    pub fn set_comb_note(&mut self, note: Option<u8>) -> Result<(), EffectError> {
        if note.is_some_and(|pitch| pitch > 127) {
            return Err(EffectError::InvalidValue);
        }
        match &mut self.processor {
            Processor::Comb(state) => {
                state.set_note(note);
                Ok(())
            }
            _ => Err(EffectError::UnsupportedNoteInput(self.id)),
        }
    }

    pub fn set_multiband_solo_band(&mut self, band: Option<usize>) -> Result<(), EffectError> {
        if band.is_some_and(|index| index >= 3) {
            return Err(EffectError::InvalidValue);
        }
        match &mut self.processor {
            Processor::Multiband(state) => {
                state.set_solo_band(band);
                Ok(())
            }
            _ => Err(EffectError::UnsupportedBandSelection(self.id)),
        }
    }

    /// The Pitcher's note enables C…B, the pitch classes it snaps to (its `CSTM` notes).
    pub fn set_pitcher_notes(&mut self, notes: [bool; 12]) -> Result<(), EffectError> {
        match &mut self.processor {
            Processor::Pitcher(state) => {
                state.set_notes(notes);
                Ok(())
            }
            _ => Err(EffectError::UnsupportedPitcherNotes(self.id)),
        }
    }

    /// The Pitcher's note enables; `None` for another effect.
    pub fn pitcher_notes(&self) -> Option<[bool; 12]> {
        match &self.processor {
            Processor::Pitcher(state) => Some(state.notes()),
            _ => None,
        }
    }

    /// One of the Analyzer's panel settings; real-time safe.
    pub fn set_analyzer(
        &mut self,
        setting: analysis::analyzer::Setting,
    ) -> Result<(), EffectError> {
        if self.probe.set_analyzer(setting) {
            Ok(())
        } else {
            Err(EffectError::UnsupportedAnalyzerSetting(self.id))
        }
    }

    pub fn meter(&self) -> MeterSnapshot {
        self.meter
    }

    /// What the effect's panel draws from the audio besides the meter, in its type's layout ([`analysis`]); empty for
    /// an effect without one.
    pub fn analysis(&self) -> &[f32] {
        self.probe.slab()
    }

    pub fn reset(&mut self) {
        if let Some(external) = &mut self.external {
            external.effect.reset();
        }
        self.processor.reset(&self.params);
        self.meter = MeterSnapshot::default();
        self.probe.reset();
    }

    pub fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        context: ProcessContext,
    ) -> Result<(), EffectError> {
        if left.len() != right.len() {
            return Err(EffectError::MismatchedBuffers);
        }
        if !context.sample_rate.is_finite() || context.sample_rate <= 0.0 {
            return Err(EffectError::InvalidSampleRate);
        }
        if !context.tempo_bpm.is_finite()
            || context.tempo_bpm <= 0.0
            || !context.start_beat.is_finite()
        {
            return Err(EffectError::InvalidContext);
        }
        if context.sample_rate != self.sample_rate {
            return Err(EffectError::InvalidSampleRate);
        }
        // A block has run: from now on a change glides, bypassed or not.
        self.common.fresh = false;
        let runs = self.common.runs(self.params[0] >= 0.5);
        if let Some(external) = &mut self.external {
            self.meter = external.process(&mut self.common, runs, left, right, context);
            return Ok(());
        }
        // A bypassed processor stands still: MultiFX's free LFO, too, advanced by each prepared block. Tape Stop still
        // takes the tempo, which a synced stop pressed as it comes back times its ramp by.
        if runs {
            self.processor.prepare_block(context, left.len());
        } else if let Processor::TapeStop(state) = &mut self.processor {
            state.prepare_block(context.tempo_bpm);
        }
        let mut peak = MeterSnapshot::default();
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let input = (*l, *r);
            if runs {
                let wet = self.processor.process_frame(*l, *r);
                (*l, *r) = self.common.frame((*l, *r), wet);
            }
            peak.peak_left = peak.peak_left.max(l.abs());
            peak.peak_right = peak.peak_right.max(r.abs());
            peak.input_peak = peak.input_peak.max(input.0.abs()).max(input.1.abs());
            self.probe
                .frame(input, (*l, *r), &self.processor, context.playing);
        }
        self.probe.block(&self.processor);
        // A bypassed processor reduces nothing, whatever it last measured.
        if runs {
            peak.gain_reduction_db = self.processor.gain_reduction_db();
        }
        self.meter = peak;
        Ok(())
    }

    fn refresh_coefficients(&mut self) {
        self.processor
            .refresh_coefficients(&self.params, self.sample_rate);
    }
}

impl External {
    /// [`AudioEffect::process`] for an external effect: in parts of at most the dry copy's length, the effect in place,
    /// then the common params against the dry copy. Bypassed, the effect does not run.
    fn process(
        &mut self,
        common: &mut CommonModule,
        runs: bool,
        left: &mut [f32],
        right: &mut [f32],
        context: ProcessContext,
    ) -> MeterSnapshot {
        let mut peak = MeterSnapshot::default();
        let block = self.dry_left.len();
        let beats_per_frame =
            f64::from(context.tempo_bpm) / (60.0 * f64::from(context.sample_rate));
        for (part, (out_l, out_r)) in left
            .chunks_mut(block)
            .zip(right.chunks_mut(block))
            .enumerate()
        {
            let frames = out_l.len();
            for (l, r) in out_l.iter().zip(out_r.iter()) {
                peak.input_peak = peak.input_peak.max(l.abs()).max(r.abs());
            }
            if runs {
                let (dry_l, dry_r) = (&mut self.dry_left[..frames], &mut self.dry_right[..frames]);
                dry_l.copy_from_slice(out_l);
                dry_r.copy_from_slice(out_r);
                let offset = if context.playing { part * block } else { 0 };
                let context = ProcessContext {
                    start_beat: context.start_beat + offset as f64 * beats_per_frame,
                    ..context
                };
                self.effect.process(out_l, out_r, context);
                for (frame, (l, r)) in out_l.iter_mut().zip(out_r.iter_mut()).enumerate() {
                    (*l, *r) = common.frame((dry_l[frame], dry_r[frame]), (*l, *r));
                }
            }
            for (l, r) in out_l.iter().zip(out_r.iter()) {
                peak.peak_left = peak.peak_left.max(l.abs());
                peak.peak_right = peak.peak_right.max(r.abs());
            }
        }
        peak
    }
}

enum Processor {
    Eq(EqState),
    Filter(FilterState),
    Leveller(LevellerState),
    Compressor(CompressorState),
    Distortion(DistortionState),
    LegacyLimiter(LegacyLimiterState),
    LookaheadLimiter(LookaheadLimiterState),
    AutoDuck(AutoDuckState),
    Tremolo(TremoloState),
    Chorus(ChorusState),
    Phaser(PhaserState),
    Stereoizer(StereoizerState),
    Spacer(SpacerState),
    Gate(Box<GateState>),
    Flanger(FlangerState),
    TapeDelay(TapeDelayState),
    WowFlutter(WowFlutterState),
    TranceDelay(TranceDelayState),
    Reverb(ReverbState),
    MultiFx(Box<MultiFxState>),
    Multiband(Box<MultibandState>),
    Pitcher(Box<PitcherState>),
    Pitcher2(Box<Pitcher2State>),
    Comb(Box<CombState>),
    TapeStop(TapeStopState),
    Spreader(SpreaderState),
    Waveshaper(WaveshaperState),
    Monitor,
}

impl Processor {
    fn refresh_coefficients(&mut self, params: &[f32], sample_rate: f32) {
        match self {
            Self::Eq(state) => state.refresh(params, sample_rate),
            Self::Filter(state) => state.refresh(params, sample_rate),
            Self::Leveller(state) => state.refresh(params),
            Self::Compressor(state) => state.refresh(params, sample_rate),
            Self::Distortion(state) => state.refresh(params, sample_rate),
            Self::LegacyLimiter(state) => state.refresh(params, sample_rate),
            Self::LookaheadLimiter(state) => state.refresh(params, sample_rate),
            Self::AutoDuck(state) => state.refresh(params),
            Self::Tremolo(state) => state.refresh(params, sample_rate),
            Self::Chorus(state) => state.refresh(params, sample_rate),
            Self::Phaser(state) => state.refresh(params, sample_rate),
            Self::Stereoizer(state) => state.refresh(params, sample_rate),
            Self::Spacer(state) => state.refresh(params, sample_rate),
            Self::Gate(state) => state.refresh(params, sample_rate),
            Self::Flanger(state) => state.refresh(params, sample_rate),
            Self::TapeDelay(state) => state.refresh(params, sample_rate),
            Self::WowFlutter(state) => state.refresh(params, sample_rate),
            Self::TranceDelay(state) => state.refresh(params, sample_rate),
            Self::Reverb(state) => state.refresh(params, sample_rate),
            Self::MultiFx(state) => state.refresh(params.try_into().unwrap(), sample_rate),
            Self::Multiband(state) => state.refresh(params, sample_rate),
            Self::Pitcher(state) => {
                state.refresh(params, sample_rate).unwrap();
            }
            Self::Pitcher2(state) => {
                state.refresh(params, sample_rate).unwrap();
            }
            Self::Comb(state) => state.refresh(params, sample_rate),
            Self::TapeStop(state) => state.refresh(params, sample_rate),
            Self::Spreader(state) => state.refresh(params, sample_rate),
            Self::Waveshaper(state) => {
                state.refresh(params, sample_rate).unwrap();
            }
            Self::Monitor => {}
        }
    }

    fn prepare_block(&mut self, context: ProcessContext, frames: usize) {
        // A song-locked LFO follows the song beat only while the transport runs; stopped, the beat stands still and
        // the LFO free-runs from its stored phase (a negative beat means "no song beat" to every processor).
        let start_beat = if context.playing {
            context.start_beat
        } else {
            -1.0
        };
        let context = ProcessContext {
            start_beat,
            ..context
        };
        match self {
            Self::AutoDuck(state) => state.prepare_block(context),
            Self::Tremolo(state) => state.prepare_block(context),
            Self::Phaser(state) => state.prepare_block(context),
            Self::Flanger(state) => state.prepare_block(context),
            Self::WowFlutter(state) => state.prepare_block(context),
            Self::TranceDelay(state) => state.prepare_block(context),
            Self::Reverb(state) => state.prepare_block(context.tempo_bpm),
            Self::MultiFx(state) => {
                state.prepare_block(context.tempo_bpm, context.start_beat, frames)
            }
            Self::TapeStop(state) => state.prepare_block(context.tempo_bpm),
            Self::Waveshaper(state) => {
                state
                    .prepare_block(
                        f64::from(context.tempo_bpm),
                        (context.start_beat >= 0.0).then_some(context.start_beat),
                    )
                    .unwrap();
            }
            _ => {}
        }
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        match self {
            Self::Eq(state) => state.process_frame(l, r),
            Self::Filter(state) => state.process_frame(l, r),
            Self::Leveller(state) => state.process_frame(l, r),
            Self::Compressor(state) => state.process_frame(l, r),
            Self::Distortion(state) => state.process_frame(l, r),
            Self::LegacyLimiter(state) => state.process_frame(l, r),
            Self::LookaheadLimiter(state) => state.process_frame(l, r),
            Self::AutoDuck(state) => state.process_frame(l, r),
            Self::Tremolo(state) => state.process_frame(l, r),
            Self::Chorus(state) => state.process_frame(l, r),
            Self::Phaser(state) => state.process_frame(l, r),
            Self::Stereoizer(state) => state.process_frame(l, r),
            Self::Spacer(state) => state.process_frame(l, r),
            Self::Gate(state) => state.process_frame(l, r),
            Self::Flanger(state) => state.process_frame(l, r),
            Self::TapeDelay(state) => state.process_frame(l, r),
            Self::WowFlutter(state) => state.process_frame(l, r),
            Self::TranceDelay(state) => state.process_frame(l, r),
            Self::Reverb(state) => state.process_frame(l, r),
            Self::MultiFx(state) => state.process_frame(l, r).unwrap(),
            Self::Multiband(state) => state.process_frame(l, r),
            Self::Pitcher(state) => state.process_frame(l, r),
            Self::Pitcher2(state) => state.process_frame(l, r),
            Self::Comb(state) => state.process_frame(l, r),
            Self::TapeStop(state) => state.process_frame(l, r),
            Self::Spreader(state) => state.process_frame(l, r),
            Self::Waveshaper(state) => state.process_frame(l, r),
            Self::Monitor => (l, r),
        }
    }

    fn reset(&mut self, params: &[f32]) {
        match self {
            Self::Eq(state) => state.reset(),
            Self::Filter(state) => state.reset(),
            Self::Leveller(state) => {
                *state = LevellerState::new(params[1]);
                state.refresh(params);
            }
            Self::Compressor(state) => state.reset(),
            Self::Distortion(state) => state.reset(),
            Self::LegacyLimiter(state) => state.reset(),
            Self::LookaheadLimiter(state) => state.reset(),
            Self::AutoDuck(state) => state.reset(),
            Self::Tremolo(state) => state.reset(),
            Self::Chorus(state) => state.reset(),
            Self::Phaser(state) => state.reset(),
            Self::Stereoizer(state) => state.reset(),
            Self::Spacer(state) => state.reset(),
            Self::Gate(state) => state.reset(),
            Self::Flanger(state) => state.reset(),
            Self::TapeDelay(state) => state.reset(),
            Self::WowFlutter(state) => state.reset(),
            Self::TranceDelay(state) => state.reset(),
            Self::Reverb(state) => state.reset(),
            Self::MultiFx(state) => state.reset(),
            Self::Multiband(state) => state.reset(),
            Self::Pitcher(state) => state.reset(),
            Self::Pitcher2(state) => state.reset(),
            Self::Comb(state) => state.reset(),
            Self::TapeStop(state) => state.reset(),
            Self::Spreader(state) => state.reset(),
            Self::Waveshaper(state) => state.reset(),
            Self::Monitor => {}
        }
    }

    fn gain_reduction_db(&self) -> f32 {
        match self {
            Self::Compressor(state) => state.gain_reduction_db,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Copy)]
enum EqKind {
    Param,
    Graphic,
    Tuned,
}

struct EqState {
    kind: EqKind,
    bands: [[Biquad; 2]; 8],
    count: usize,
}

impl EqState {
    fn new(kind: EqKind) -> Self {
        Self {
            kind,
            bands: [[Biquad::default(); 2]; 8],
            count: if matches!(kind, EqKind::Param) { 4 } else { 8 },
        }
    }

    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        const GRAPHIC_HZ: [f32; 8] = [80.0, 160.0, 375.0, 750.0, 1500.0, 3000.0, 6000.0, 12000.0];
        for i in 0..self.count {
            let (hz, q, response, left_gain, right_gain) = match self.kind {
                EqKind::Param => {
                    let first = 1 + 4 * i;
                    let gain = -12.0 + 24.0 * params[first];
                    let hz = 2.0_f32.powf(5.0 + 9.25 * params[first + 1]);
                    let q = 0.25 + 7.0 * params[first + 2];
                    // Kind trunc(2v) in f64, Channel trunc(4v).
                    let response = match (f64::from(params[first + 3]) * 2.0).trunc() as u8 {
                        0 => Response::LowShelf,
                        2 => Response::HighShelf,
                        _ => Response::Peak,
                    };
                    let (left, right) = match (params[17 + i] * 4.0).trunc() as u8 {
                        1 => (gain, 0.0),
                        2 => (0.0, gain),
                        3 => (gain, -gain),
                        4 => (-gain, gain),
                        _ => (gain, gain),
                    };
                    (hz, q, response, left, right)
                }
                EqKind::Graphic => {
                    let gain = -12.0 + 24.0 * params[i + 1];
                    let response = if i == 0 {
                        Response::LowShelf
                    } else if i == 7 {
                        Response::HighShelf
                    } else {
                        Response::Peak
                    };
                    (GRAPHIC_HZ[i], 1.0, response, gain, gain)
                }
                EqKind::Tuned => {
                    let gain = -12.0 + 24.0 * params[i + 1];
                    // trunc(fma_f64(v, 11, f32(0.001))).
                    let key = f64::from(params[9])
                        .mul_add(11.0, f64::from(0.001_f32))
                        .trunc() as f32;
                    let hz = 110.0 * 2.0_f32.powf(key / 12.0 - 0.75 + i as f32);
                    let q = 0.1 + 10.0 * params[10].powi(3);
                    let response = if i == 0 {
                        Response::LowShelf
                    } else if i == 7 {
                        Response::HighShelf
                    } else {
                        Response::Peak
                    };
                    (hz, q, response, gain, gain)
                }
            };
            match self.kind {
                EqKind::Param => {
                    self.bands[i][0].set_parametric_response(
                        response,
                        hz,
                        q,
                        left_gain,
                        sample_rate * 2.0,
                    );
                    self.bands[i][1].set_parametric_response(
                        response,
                        hz,
                        q,
                        right_gain,
                        sample_rate * 2.0,
                    );
                }
                EqKind::Tuned => {
                    self.bands[i][0].set_parametric_response(
                        response,
                        hz,
                        q,
                        left_gain,
                        sample_rate,
                    );
                    self.bands[i][1].set_parametric_response(
                        response,
                        hz,
                        q,
                        right_gain,
                        sample_rate,
                    );
                }
                EqKind::Graphic => {
                    self.bands[i][0].set_parametric_response(
                        response,
                        hz,
                        q,
                        left_gain,
                        sample_rate,
                    );
                    self.bands[i][1].set_parametric_response(
                        response,
                        hz,
                        q,
                        right_gain,
                        sample_rate,
                    );
                }
            }
        }
    }

    fn process_frame(&mut self, mut l: f32, mut r: f32) -> (f32, f32) {
        for band in self.bands.iter_mut().take(self.count) {
            l = band[0].process(l);
            r = band[1].process(r);
        }
        (l, r)
    }

    fn reset(&mut self) {
        for band in &mut self.bands {
            for filter in band {
                filter.reset();
            }
        }
    }
}

#[derive(Default)]
struct FilterState {
    sections: [[Biquad; 2]; 8],
    stage_delay: [[f32; 2]; 4],
    active: usize,
    output_gain: f32,
}

impl FilterState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        let frequency = 2.0_f32.powf(4.0 + 10.3 * params[1]);
        let q = 0.8 + 2.0 * params[2].powi(2);
        let response = match (params[3] * 2.0).trunc() as u8 {
            0 => Response::LowPass,
            2 => Response::HighPass,
            _ => Response::BandPass,
        };
        let passes = match (params[4] * 2.0).trunc() as u8 {
            0 => 1,
            2 => 4,
            _ => 2,
        };
        self.active = passes * 2;
        self.output_gain = if matches!(response, Response::LowPass) {
            0.8_f32.powi(passes as i32)
        } else {
            1.0
        };
        for section in self.sections.iter_mut().take(self.active) {
            for filter in section {
                filter.set_filter_response(response, frequency, q, sample_rate);
            }
        }
    }

    fn process_frame(&mut self, mut l: f32, mut r: f32) -> (f32, f32) {
        for (pair, delay) in self.sections[..self.active]
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .zip(&mut self.stage_delay)
        {
            let next_l = pair[0][0].process(l);
            let next_r = pair[0][1].process(r);
            l = pair[1][0].process(delay[0]);
            r = pair[1][1].process(delay[1]);
            *delay = [next_l, next_r];
        }
        (l * self.output_gain, r * self.output_gain)
    }

    fn reset(&mut self) {
        self.stage_delay = [[0.0; 2]; 4];
        for section in &mut self.sections {
            for filter in section {
                filter.reset();
            }
        }
    }
}

struct LevellerState {
    gain: f32,
    balance_left: f32,
    balance_right: f32,
    target_gain: f32,
    target_left: f32,
    target_right: f32,
}

impl LevellerState {
    fn new(value: f32) -> Self {
        Self {
            gain: leveller_amplitude(value),
            balance_left: 1.0,
            balance_right: 1.0,
            target_gain: leveller_amplitude(value),
            target_left: 1.0,
            target_right: 1.0,
        }
    }

    fn refresh(&mut self, params: &[f32]) {
        self.target_gain = leveller_amplitude(params[1]);
        self.target_left = (2.0 * (1.0 - params[2])).min(1.0);
        self.target_right = (2.0 * params[2]).min(1.0);
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.gain = flush(self.gain + 0.01 * (self.target_gain - self.gain));
        self.balance_left =
            flush(self.balance_left + 0.01 * (self.target_left - self.balance_left));
        self.balance_right =
            flush(self.balance_right + 0.01 * (self.target_right - self.balance_right));
        (
            l * self.gain * self.balance_left,
            r * self.gain * self.balance_right,
        )
    }
}

fn leveller_amplitude(value: f32) -> f32 {
    let db = -60.0 + 72.0 * value.powf(0.6);
    10.0_f32.powf(db / 20.0)
}

#[derive(Default)]
struct CompressorState {
    held_peak: f32,
    envelope: f32,
    hold_remaining: usize,
    hold_frames: usize,
    attack_step: f32,
    release_multiplier: f32,
    threshold: f32,
    inverse_ratio: f32,
    detector_gain: f32,
    makeup_gain: f32,
    gain_reduction_db: f32,
}

impl CompressorState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        let attack_seconds = 0.05 * params[3].powi(2);
        self.attack_step = if attack_seconds < 1.0 / sample_rate {
            1.0
        } else {
            1.0 - 0.1_f32.powf(1.0 / (sample_rate * attack_seconds))
        };
        self.hold_frames = (sample_rate * 0.05 * params[6].powi(2)).trunc() as usize;
        let release_seconds = 0.01 + 0.49 * params[4].powi(2);
        self.release_multiplier = 0.1_f32.powf(1.0 / (sample_rate * release_seconds));
        self.threshold = db_to_amplitude(-40.0 + 40.0 * params[1]);
        let ratio = 1.0 + 19.0 * params[2].powi(2);
        self.inverse_ratio = if ratio >= 20.0 {
            0.0
        } else if self.threshold >= 1.0 {
            1.0 / ratio
        } else {
            let compressed_unity =
                db_to_amplitude((-40.0 + 40.0 * params[1]) * (1.0 - 1.0 / ratio));
            (compressed_unity - self.threshold) / (1.0 - self.threshold).max(f32::MIN_POSITIVE)
        };
        self.detector_gain = db_to_amplitude(-20.0 + 40.0 * params[8]);
        let auto_gain = if params[7] >= 0.5 {
            1.0 / (self.threshold + self.inverse_ratio * (1.0 - self.threshold))
        } else {
            1.0
        };
        let level_db = if params[7] >= 0.5 {
            -20.0 + 40.0 * params[5]
        } else {
            40.0 * params[5]
        };
        self.makeup_gain = db_to_amplitude(level_db) * auto_gain;
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let detector = l.abs().max(r.abs()) * self.detector_gain;
        if detector > self.held_peak {
            self.held_peak = detector;
            self.hold_remaining = self.hold_frames;
        } else if self.hold_remaining > 0 {
            self.hold_remaining -= 1;
        } else {
            self.held_peak = flush(self.held_peak * self.release_multiplier);
        }
        self.envelope = flush(self.envelope + self.attack_step * (self.held_peak - self.envelope));
        let reduction = if self.envelope < self.threshold {
            1.0
        } else {
            (self.threshold + self.inverse_ratio * (self.envelope - self.threshold))
                / self.envelope.max(f32::MIN_POSITIVE)
        };
        self.gain_reduction_db = -20.0 * reduction.max(f32::MIN_POSITIVE).log10();
        let gain = reduction * self.makeup_gain;
        (l * gain, r * gain)
    }

    fn reset(&mut self) {
        self.held_peak = 0.0;
        self.envelope = 0.0;
        self.hold_remaining = 0;
        self.gain_reduction_db = 0.0;
    }
}

fn db_to_amplitude(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

#[derive(Default)]
struct DistortionState {
    drive_left: f32,
    drive_right: f32,
    bias: f32,
    output_gain: f32,
    mode: u8,
    hold_interval: usize,
    hold_remaining: usize,
    held_left: f32,
    held_right: f32,
    bias_filter_left: f32,
    bias_filter_right: f32,
    bias_filter_step: f32,
}

impl DistortionState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        let left = (2.0 * params[4]).min(1.0);
        let right = (2.0 - 2.0 * params[4]).min(1.0);
        let drive = params[1];
        self.mode = (3.0 * params[5]).trunc() as u8;
        match self.mode {
            0 => {
                let left_scale = 10.0_f32.powf(5.0 * drive * left);
                let right_scale = 10.0_f32.powf(5.0 * drive * right);
                self.drive_left = left_scale - 1.0;
                self.drive_right = right_scale - 1.0;
            }
            1 => {
                self.drive_left = 10.0_f32.powf(4.0 * drive * left) - 0.99;
                self.drive_right = 10.0_f32.powf(4.0 * drive * right) - 0.99;
            }
            2 => {
                self.drive_left = 20.0 * drive * drive * left;
                self.drive_right = 20.0 * drive * drive * right;
            }
            _ => {
                self.drive_left = 10.0_f32.powf(3.0 * drive * drive * left);
                self.drive_right = 10.0_f32.powf(3.0 * drive * drive * right);
            }
        }
        self.bias = (2.0 * params[6] - 1.0).powi(5);
        self.output_gain = 10.0_f32.powf(1.2 * params[3] - 1.2);
        self.hold_interval = 1 + (256.0 * params[2].powi(4)).trunc() as usize;
        self.bias_filter_step = 220.5 / sample_rate;
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.hold_remaining == 0 {
            self.held_left = self.shape(l, self.drive_left);
            self.held_right = self.shape(r, self.drive_right);
            self.hold_remaining = self.hold_interval;
        }
        self.hold_remaining -= 1;
        let mut left = self.held_left * self.output_gain;
        let mut right = self.held_right * self.output_gain;
        if self.bias != 0.0 {
            // Flushed: a bias just off centre is a DC the filter all but follows, and the leftover sank to subnormals.
            self.bias_filter_left = flush(
                self.bias_filter_left + (left - self.bias_filter_left) * self.bias_filter_step,
            );
            self.bias_filter_right = flush(
                self.bias_filter_right + (right - self.bias_filter_right) * self.bias_filter_step,
            );
            left -= self.bias_filter_left;
            right -= self.bias_filter_right;
        }
        (left, right)
    }

    fn shape(&self, sample: f32, drive: f32) -> f32 {
        let signal = sample + self.bias;
        match self.mode {
            0 => (drive + 1.0) * signal / (1.0 + drive * signal.abs()),
            1 => (drive * signal).tanh() / drive.tanh(),
            2 => {
                if signal >= 0.0 {
                    1.0 + (signal - 1.0) * 10.0_f32.powf(-drive * signal)
                } else {
                    -1.0 + (signal + 1.0) * 10.0_f32.powf(drive * signal)
                }
            }
            _ => (drive * signal).clamp(-1.0, 1.0),
        }
    }

    fn reset(&mut self) {
        self.hold_remaining = 0;
        self.held_left = 0.0;
        self.held_right = 0.0;
        self.bias_filter_left = 0.0;
        self.bias_filter_right = 0.0;
    }
}

#[derive(Default)]
struct LegacyLimiterState {
    threshold: f32,
    output_gain: f32,
    release_step: f32,
    release_phase: f32,
    release_base: f32,
}

impl LegacyLimiterState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        self.threshold = 10.0_f32.powf(1.2 * params[1] - 1.2);
        self.output_gain = 10.0_f32.powf((36.0 * params[3] - 24.0) * 0.05);
        let release_frames = 1.0 + 100.0 * (0.2 * (1.0 - params[2])).powi(2);
        self.release_step = release_frames / sample_rate;
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let peak = l.abs().max(r.abs());
        let sine = (self.release_phase * f32::from_bits(0x3fc9_0ff9)).sin();
        let mut reduction = 1.0 + (sine * sine - 1.0) * (1.0 - self.release_base);
        if reduction * peak > self.threshold {
            reduction = self.threshold / peak;
            self.release_phase = 0.0;
            self.release_base = reduction;
        }
        self.release_phase = (self.release_phase + self.release_step).min(1.0);
        let gain = self.output_gain * reduction / self.threshold;
        (l * gain, r * gain)
    }

    fn reset(&mut self) {
        self.release_phase = 1.0;
        self.release_base = 1.0;
    }
}

#[derive(Default)]
struct LookaheadLimiterState {
    input_gain: f32,
    output_gain: f32,
    threshold: f32,
    release_seconds: f32,
    sample_rate: f32,
    delay_left: Vec<f32>,
    delay_right: Vec<f32>,
    write_index: usize,
    release_phase: f32,
    held_gain: f32,
    /// The gain the last frame applied, for the history display (`analysis`).
    gain: f32,
}

impl LookaheadLimiterState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        self.input_gain = 10.0_f32.powf((48.0 * params[1] - 24.0) * 0.05);
        self.output_gain = 10.0_f32.powf((48.0 * params[2] - 24.0) * 0.05);
        self.release_seconds = 0.1 + 1.9 * params[3].powi(4);
        self.threshold = 10.0_f32.powf((24.0 * params[4] - 24.0) * 0.05);
        if sample_rate != self.sample_rate {
            self.sample_rate = sample_rate;
            let size = (sample_rate * 0.002).trunc() as usize;
            self.delay_left.resize(size, 0.0);
            self.delay_right.resize(size, 0.0);
            self.reset();
        }
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let in_left = l * self.input_gain;
        let in_right = r * self.input_gain;
        let peak = in_left.abs().max(in_right.abs());
        let release_sine = if self.release_phase > 0.0 {
            let sine = (self.release_phase * std::f32::consts::FRAC_PI_2).sin();
            self.release_phase -= 1.0 / (self.sample_rate * self.release_seconds);
            if self.release_phase <= 0.0 {
                self.release_phase = 0.0;
                0.0
            } else {
                sine
            }
        } else {
            0.0
        };
        let gain = self.held_gain + (1.0 - self.held_gain) * (1.0 - release_sine);
        self.gain = gain;
        if gain * peak > self.threshold {
            self.release_phase = 1.0;
            self.held_gain = self.threshold / peak;
        }
        if self.delay_left.is_empty() {
            return (
                in_left * gain * self.output_gain,
                in_right * gain * self.output_gain,
            );
        }
        let delayed_left = self.delay_left[self.write_index];
        let delayed_right = self.delay_right[self.write_index];
        self.delay_left[self.write_index] = in_left;
        self.delay_right[self.write_index] = in_right;
        self.write_index += 1;
        if self.write_index == self.delay_left.len() {
            self.write_index = 0;
        }
        (
            delayed_left * gain * self.output_gain,
            delayed_right * gain * self.output_gain,
        )
    }

    fn reset(&mut self) {
        self.delay_left.fill(0.0);
        self.delay_right.fill(0.0);
        self.write_index = 0;
        self.release_phase = 1.0;
        self.held_gain = 1.0;
        self.gain = 1.0;
    }
}

struct AutoDuckState {
    modern: bool,
    params: [f32; 8],
    phase: f64,
    phase_step: f64,
    gain: f32,
}

impl AutoDuckState {
    fn new(modern: bool) -> Self {
        Self {
            modern,
            params: [0.0; 8],
            phase: 0.0,
            phase_step: 0.0,
            gain: -1.0,
        }
    }

    fn refresh(&mut self, params: &[f32]) {
        self.params[..params.len()].copy_from_slice(params);
    }

    fn prepare_block(&mut self, context: ProcessContext) {
        if context.start_beat >= 0.0 {
            self.phase = context.start_beat - context.start_beat.trunc();
        }
        self.phase_step = f64::from(context.tempo_bpm) / (f64::from(context.sample_rate) * 60.0);
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let target = if self.modern {
            1.0 - self.modern_curve() * self.params[1].sqrt()
        } else {
            1.0 - self.legacy_curve() * self.params[1].powi(2)
        };
        self.gain = if self.gain >= 0.0 {
            self.gain + (target - self.gain) * 0.01
        } else {
            target
        };
        self.phase += self.phase_step;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        (l * self.gain, r * self.gain)
    }

    fn legacy_curve(&self) -> f32 {
        let attack = self.params[2] * 0.5 + 0.001;
        let phase = self.phase as f32;
        let value = if phase < attack {
            phase / attack
        } else {
            1.0 + (attack - phase) / (1.0 - attack)
        };
        value.clamp(0.0, 1.0).powf(self.params[3] * 0.5 + 0.5)
    }

    fn modern_curve(&self) -> f32 {
        let phase = ((self.phase as f32) + 0.5 - self.params[5]).rem_euclid(1.0);
        let attack = self.params[2] * 0.5 + 0.001;
        let time = self.params[4].max(0.001);
        let hold_end = self.params[6] * time + attack;
        let value = if phase < attack {
            phase / attack
        } else if phase >= time {
            0.0
        } else if phase < hold_end {
            1.0
        } else {
            1.0 + (hold_end - phase) / (time - hold_end)
        }
        .clamp(0.0, 1.0);
        if value == 0.0 {
            return 0.0;
        }
        let shape = (self.params[7] * 5.0).trunc() as u8;
        let curve = self.params[3];
        if phase < attack || shape == 0 {
            return value.powf(curve * 0.5 + 0.5);
        }
        match shape {
            1 => value.powf(0.1 + 0.9 * curve),
            2 => value.powf(3.0 - 2.0 * curve),
            3 => value.powf(6.0 - 5.0 * curve),
            4 | 5 => {
                let strength = if shape == 4 { 2.0 } else { 5.0 };
                let exponent = (1.0 - curve) * strength + 1.0;
                let distance = value.min(1.0 - value);
                let half = 0.5 * 2.0_f32.powf(exponent) * distance.powf(exponent);
                if value < 0.5 { half } else { 1.0 - half }
            }
            _ => value,
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.gain = -1.0;
    }
}

#[derive(Default)]
struct TremoloState {
    phase: [f64; 2],
    increment: f64,
    oscillator_output: [f32; 2],
    smoothed_gain: [f32; 2],
    minimum: f32,
    maximum: f32,
    phase_offset: f32,
    stereo_offset: f32,
    shape: f64,
    smooth: f64,
    skew: f64,
    width: f64,
    synced: bool,
    beat_period: f64,
    frequency_target: f32,
    frequency_current: f32,
    frequency_smoothing: f32,
    gain_smoothing: f32,
}

impl TremoloState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        self.maximum = params[3];
        self.minimum = params[4];
        self.phase_offset = params[5];
        self.stereo_offset = params[6];
        self.synced = params[7] > 0.5;
        self.shape = f64::from(2.0 * params[8] - 1.0);
        self.smooth = f64::from(0.001 + 0.998 * params[9]);
        self.skew = f64::from(0.001 + 0.998 * params[10]);
        self.width = f64::from(0.999 - 1.998 * params[11]);
        self.beat_period = [0.125, 0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0]
            [((params[2] * 9.0).trunc() as usize).min(9)];
        let frequency = 0.1 + 19.9 * params[1];
        if frequency != self.frequency_target {
            self.frequency_target = frequency;
            self.frequency_current = frequency;
        }
        self.frequency_smoothing = 1.0 - 44.1 / sample_rate;
        self.gain_smoothing = (-2094.39508_f64 / f64::from(sample_rate)).exp() as f32;
    }

    fn prepare_block(&mut self, context: ProcessContext) {
        if self.synced {
            self.increment = f64::from(context.tempo_bpm)
                / (self.beat_period * 60.0 * f64::from(context.sample_rate));
            if context.start_beat >= 0.0 {
                let phase = (context.start_beat / self.beat_period).fract();
                self.phase = [phase, phase];
            }
        } else {
            self.increment = f64::from(self.frequency_current) / f64::from(context.sample_rate);
        }
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let span = (self.maximum - self.minimum) * 0.495;
        let target_left = (self.minimum + span * (self.oscillator_output[0] + 1.0))
            .clamp(self.minimum, self.maximum);
        let target_right = (self.minimum + span * (self.oscillator_output[1] + 1.0))
            .clamp(self.minimum, self.maximum);
        self.smoothed_gain[0] =
            flush(target_left + (self.smoothed_gain[0] - target_left) * self.gain_smoothing);
        self.smoothed_gain[1] =
            flush(target_right + (self.smoothed_gain[1] - target_right) * self.gain_smoothing);
        let output = (l * self.smoothed_gain[0], r * self.smoothed_gain[1]);
        self.oscillator_output[0] = self.wave(self.phase[0], self.phase_offset);
        self.oscillator_output[1] =
            self.wave(self.phase[1], self.phase_offset + self.stereo_offset);
        self.phase[0] = (self.phase[0] + self.increment).fract();
        self.phase[1] = (self.phase[1] + self.increment).fract();
        self.frequency_current = self.frequency_target
            + (self.frequency_current - self.frequency_target) * self.frequency_smoothing;
        output
    }

    fn wave(&self, phase: f64, offset: f32) -> f32 {
        let phase = (phase + f64::from(offset)).fract();
        let rising = phase / self.skew;
        let falling = (1.0 - phase) / (1.0 - self.skew);
        let triangle = rising.min(falling);
        let unshaped =
            ((2.0 * triangle - 1.0 - self.width) / (1.0 - self.width.abs())).clamp(-1.0, 1.0);
        let exponent = (self.shape * 4.0).exp2();
        let angle = unshaped.signum() * std::f64::consts::FRAC_PI_2 * unshaped.abs().powf(exponent);
        let linear = angle * std::f64::consts::FRAC_2_PI;
        (linear + (angle.sin() - linear) * self.smooth) as f32
    }

    fn reset(&mut self) {
        self.phase = [0.0; 2];
        self.increment = 0.0;
        self.oscillator_output = [0.0; 2];
        self.smoothed_gain = [0.0; 2];
        self.frequency_current = self.frequency_target;
    }
}

#[derive(Default)]
struct ChorusState {
    buffer: Vec<f32>,
    write_index: usize,
    phase: [f32; 2],
    phase_step: [f32; 2],
    depth: f32,
    stereo: f32,
    mix: f32,
}

impl ChorusState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        let length = (sample_rate * 0.01).trunc().max(1.0) as usize;
        if self.buffer.len() != length {
            self.buffer.resize(length, 0.0);
            self.reset();
        }
        let rate = 0.2 + 4.0 * params[1].powi(2);
        self.phase_step = [rate / sample_rate, (rate + 0.1) / sample_rate];
        self.depth = 0.04 + 0.36 * params[2];
        self.stereo = params[3];
        self.mix = params[4].powi(2);
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let length = self.buffer.len();
        self.buffer[self.write_index] = l + r;
        let phase_left = self.phase[0] * std::f32::consts::TAU;
        let phase_right = self.phase[1] * std::f32::consts::TAU;
        let motion_left = phase_left.sin() * self.depth + 0.5;
        let motion_right = phase_right.sin() * self.depth + 0.5;
        let first_offset = motion_left.powi(2) * self.depth * length as f32;
        let tap_left = self.read_forward(first_offset);
        let tap_right = self.read_forward(motion_right);
        self.phase[0] = (self.phase[0] + self.phase_step[0]).fract();
        self.phase[1] = (self.phase[1] + self.phase_step[1]).fract();
        self.write_index += 1;
        if self.write_index == length {
            self.write_index = 0;
        }
        let cross = 1.0 - self.stereo;
        (
            l + (tap_left + tap_right * cross) * self.mix,
            r + (tap_right + tap_left * cross) * self.mix,
        )
    }

    fn read_forward(&self, offset: f32) -> f32 {
        let whole = offset.trunc() as usize;
        let fraction = offset.fract();
        let first = (self.write_index + whole) % self.buffer.len();
        let second = (first + 1) % self.buffer.len();
        self.buffer[first] + fraction * (self.buffer[second] - self.buffer[first])
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_index = 0;
        self.phase = [0.62, 0.1];
    }
}

#[derive(Default)]
struct PhaserState {
    stages: [[f32; 6]; 2],
    feedback_output: [f32; 2],
    phase: f64,
    increment: f64,
    period_beats: f64,
    free_rate_hz: f32,
    synced: bool,
    shape: u8,
    stereo_offset: f64,
    low: f32,
    high: f32,
    center: f32,
    depth: f32,
    feedback: f32,
    mix: f32,
}

impl PhaserState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        const PERIODS: [f64; 13] = [
            0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0,
        ];
        self.period_beats = PERIODS[((params[1] * 12.0).trunc() as usize).min(12)];
        self.depth = 0.25 * params[2].powi(3);
        self.feedback = 0.95 * params[3];
        self.center = params[4].powi(2);
        self.mix = params[5].powi(2);
        self.synced = params[6] != 0.0;
        self.free_rate_hz = 0.1 + 9.9 * params[7].powi(3);
        self.shape = ((params[8] * 2.0).round() as u8).min(2);
        self.stereo_offset = f64::from(params[9]) * 0.5;
        self.low = 50.0 / sample_rate;
        self.high = (16_000.0 / sample_rate).min(0.499);
    }

    fn prepare_block(&mut self, context: ProcessContext) {
        self.increment = if self.synced {
            if context.start_beat >= 0.0 {
                self.phase = (context.start_beat / self.period_beats).fract();
            }
            f64::from(context.tempo_bpm)
                / (60.0 * self.period_beats * f64::from(context.sample_rate))
        } else {
            f64::from(self.free_rate_hz) / f64::from(context.sample_rate)
        };
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let left_coefficient = self.coefficient(self.phase);
        let right_coefficient = self.coefficient((self.phase + self.stereo_offset).fract());
        self.phase = (self.phase + self.increment).fract();
        (
            self.process_channel(0, l, left_coefficient),
            self.process_channel(1, r, right_coefficient),
        )
    }

    fn coefficient(&self, phase: f64) -> f32 {
        let oscillator = match self.shape {
            1 => (2.0 * phase - 1.0) as f32,
            2 => {
                if phase <= 0.5 {
                    -1.0
                } else {
                    1.0
                }
            }
            _ => (phase * std::f64::consts::TAU).sin() as f32,
        };
        let frequency = (self.low
            + (self.center + self.depth * oscillator) * (self.high - self.low))
            .clamp(self.low, self.high);
        (1.0 - frequency) / (1.0 + frequency)
    }

    fn process_channel(&mut self, channel: usize, input: f32, coefficient: f32) -> f32 {
        let mut sample = input + 1e-30 + self.feedback * self.feedback_output[channel];
        for history in &mut self.stages[channel] {
            let output = *history - coefficient * sample;
            *history = sample + coefficient * output;
            sample = output;
        }
        self.feedback_output[channel] = sample;
        input + self.mix * sample
    }

    fn reset(&mut self) {
        self.stages = [[0.0; 6]; 2];
        self.feedback_output = [0.0; 2];
        self.phase = 0.0;
        self.increment = 0.0;
    }
}

#[derive(Default)]
struct StereoizerState {
    shared: Vec<f32>,
    left_history: Vec<f32>,
    right_history: Vec<f32>,
    shared_write: usize,
    side_write: usize,
    shared_delay: f32,
    left_delay: f32,
    right_delay: f32,
    focus_state: f32,
    focus_coefficient: f32,
    focus_highpass: bool,
    strength: f32,
    pan_target: [f32; 2],
    pan_current: [f32; 2],
}

impl StereoizerState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        let shared_length = (sample_rate * 0.05).trunc().max(2.0) as usize;
        let side_length = (sample_rate * 0.025).trunc().max(2.0) as usize;
        if self.shared.len() != shared_length {
            self.shared = vec![0.0; shared_length];
            self.shared_write = 0;
        }
        if self.left_history.len() != side_length {
            self.left_history = vec![0.0; side_length];
            self.right_history = vec![0.0; side_length];
            self.side_write = 0;
        }
        self.strength = params[1].powi(2);
        let minimum = (sample_rate * 0.0005).trunc();
        self.shared_delay = minimum + params[2].powi(3) * (shared_length as f32 - minimum - 1.0);
        let focus = 1.0 - 2.0 * params[3];
        self.focus_highpass = focus < 0.0;
        let base = if self.focus_highpass { 0.99 } else { 0.8 };
        self.focus_coefficient = if self.focus_highpass {
            (-base * focus).powi(3)
        } else {
            (1.0 - base * focus).powi(3)
        };
        let side = (2.0 * params[4] - 1.0).powi(3) * (side_length - 1) as f32;
        self.left_delay = (-side).max(0.0);
        self.right_delay = side.max(0.0);
        self.pan_target = [
            (2.0 - 2.0 * params[5]).clamp(0.0, 1.0),
            (2.0 * params[5]).clamp(0.0, 1.0),
        ];
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mut wet_l = l;
        let mut wet_r = r;
        if self.strength > 0.001 {
            let tap = read_fractional(&self.shared, self.shared_write, self.shared_delay);
            self.focus_state =
                flush(self.focus_state + self.focus_coefficient * (tap - self.focus_state));
            let component = if self.focus_highpass {
                tap - self.focus_state
            } else {
                self.focus_state
            };
            wet_l += self.strength * component;
            wet_r -= self.strength * component;
        }
        self.shared[self.shared_write] = l + r;
        self.shared_write = (self.shared_write + 1) % self.shared.len();
        let output_l = if self.left_delay > 0.9 {
            read_fractional(&self.left_history, self.side_write, self.left_delay)
        } else {
            wet_l
        };
        let output_r = if self.right_delay > 0.9 {
            read_fractional(&self.right_history, self.side_write, self.right_delay)
        } else {
            wet_r
        };
        self.left_history[self.side_write] = wet_l;
        self.right_history[self.side_write] = wet_r;
        self.side_write = (self.side_write + 1) % self.left_history.len();
        for channel in 0..2 {
            self.pan_current[channel] +=
                0.01 * (self.pan_target[channel] - self.pan_current[channel]);
            if (self.pan_current[channel] - self.pan_target[channel]).abs() < 0.001 {
                self.pan_current[channel] = self.pan_target[channel];
            }
        }
        (
            output_l * self.pan_current[0],
            output_r * self.pan_current[1],
        )
    }

    fn reset(&mut self) {
        self.shared.fill(0.0);
        self.left_history.fill(0.0);
        self.right_history.fill(0.0);
        self.shared_write = 0;
        self.side_write = 0;
        self.focus_state = 0.0;
        self.pan_current = self.pan_target;
    }
}

fn read_fractional(buffer: &[f32], write_index: usize, delay: f32) -> f32 {
    let position = (write_index as f32 - delay).rem_euclid(buffer.len() as f32);
    // A tiny negative wraps to exactly the length in f32.
    let first = position.trunc() as usize % buffer.len();
    let second = (first + 1) % buffer.len();
    buffer[first] + (position.fract()) * (buffer[second] - buffer[first])
}

struct SpacerState {
    paths: [Vec<f32>; 4],
    positions: [usize; 2],
    maximum_delay: [f32; 2],
    controls: [f32; 4],
    targets: [f32; 4],
    inversion_lr: u8,
    inversion_ms: u8,
    mode: u8,
}

impl Default for SpacerState {
    fn default() -> Self {
        Self {
            paths: std::array::from_fn(|_| Vec::new()),
            positions: [0; 2],
            maximum_delay: [0.0; 2],
            controls: [0.5; 4],
            targets: [0.5; 4],
            inversion_lr: 0,
            inversion_ms: 0,
            mode: 0,
        }
    }
}

impl SpacerState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        self.targets = [params[1], params[2], params[3], params[5]];
        self.inversion_lr = (params[4] * 3.0).round() as u8;
        self.inversion_ms = (params[6] * 3.0).round() as u8;
        self.mode = (params[7] * 2.0).round() as u8;
        self.maximum_delay = [sample_rate * 0.3, sample_rate * 0.1];
        let sizes = [
            (self.maximum_delay[0].ceil() as usize).next_power_of_two(),
            (self.maximum_delay[1].ceil() as usize).next_power_of_two(),
        ];
        for (index, path) in self.paths.iter_mut().enumerate() {
            let size = sizes[index / 2].max(2);
            if path.len() != size {
                *path = vec![0.0; size];
                self.positions[index / 2] = 0;
            }
        }
    }

    fn process_frame(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
        for i in 0..4 {
            self.controls[i] += 0.001 * (self.targets[i] - self.controls[i]);
        }
        let lr_shift = (2.0 * self.controls[0] - 1.0).powi(3);
        let read_lr = self.positions[0];
        let mut left = write_and_read(
            &mut self.paths[0],
            read_lr,
            input_l,
            (self.maximum_delay[0] * lr_shift).clamp(0.0, self.maximum_delay[0]),
        );
        let mut right = write_and_read(
            &mut self.paths[1],
            read_lr,
            input_r,
            (-self.maximum_delay[0] * lr_shift).clamp(0.0, self.maximum_delay[0]),
        );
        self.positions[0] = (read_lr + 1) % self.paths[0].len();
        if self.inversion_lr & 1 != 0 {
            left = -left;
        }
        if self.inversion_lr & 2 != 0 {
            right = -right;
        }
        let separation = self.controls[2];
        let (mut left, mut right) = if separation < 0.5 {
            let half = 0.5 - separation;
            let adjusted_left = left - half * right;
            (adjusted_left, right - half * adjusted_left)
        } else {
            let mid = 0.5 * (left + right);
            let side = (left - right) * (1.0 - separation);
            (mid + side, mid - side)
        };
        let pan = self.controls[1];
        if pan != 0.5 {
            left *= (2.0 - 2.0 * pan).min(1.0);
            right *= (2.0 * pan).min(1.0);
        }
        if self.mode != 2 {
            let mid = 0.5 * (left + right);
            let side = 0.5 * (left - right);
            left = mid;
            right = side;
        }
        if self.inversion_ms & 1 != 0 {
            left = -left;
        }
        if self.inversion_ms & 2 != 0 {
            right = -right;
        }
        let ms_shift = (2.0 * self.controls[3] - 1.0).powi(5);
        let read_ms = self.positions[1];
        let mid = write_and_read(
            &mut self.paths[2],
            read_ms,
            left,
            (self.maximum_delay[1] * ms_shift).clamp(0.0, self.maximum_delay[1]),
        );
        let side = write_and_read(
            &mut self.paths[3],
            read_ms,
            right,
            (-self.maximum_delay[1] * ms_shift).clamp(0.0, self.maximum_delay[1]),
        );
        self.positions[1] = (read_ms + 1) % self.paths[2].len();
        if self.mode == 1 {
            (mid, side)
        } else {
            (mid + side, mid - side)
        }
    }

    fn reset(&mut self) {
        for path in &mut self.paths {
            path.fill(0.0);
        }
        self.positions = [0; 2];
    }
}

fn write_and_read(buffer: &mut [f32], write_index: usize, input: f32, delay: f32) -> f32 {
    buffer[write_index] = input;
    let whole = delay.trunc() as usize;
    let newer = (write_index + buffer.len() - whole % buffer.len()) % buffer.len();
    let older = (newer + buffer.len() - 1) % buffer.len();
    buffer[newer] + delay.fract() * (buffer[older] - buffer[newer])
}

struct GateState {
    history: [[f32; 256]; 2],
    write_index: usize,
    level_db: [f32; 2],
    phase: f32,
    gain: f32,
    open: bool,
    hold_remaining: usize,
    threshold_db: f32,
    asymmetry_db: f32,
    attack_step: f32,
    release_step: f32,
    attack_curve: f32,
    release_curve: f32,
    hold_frames: usize,
    floor: f32,
    inverted: bool,
}

impl Default for GateState {
    fn default() -> Self {
        Self {
            history: [[0.0; 256]; 2],
            write_index: 0,
            level_db: [-60.0; 2],
            phase: 0.0,
            gain: 0.0,
            open: false,
            hold_remaining: 0,
            threshold_db: -60.0,
            asymmetry_db: 0.0,
            attack_step: 1.0,
            release_step: 1.0,
            attack_curve: -1.0,
            release_curve: 1.0,
            hold_frames: 0,
            floor: 0.001,
            inverted: false,
        }
    }
}

impl GateState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        self.threshold_db = -60.0 + 66.0 * params[1];
        self.asymmetry_db = 66.0 * params[2].powi(2);
        self.attack_step = if params[3] == 0.0 {
            1.0
        } else {
            1.0 / (sample_rate * params[3])
        };
        self.attack_curve = 2.0 * params[4] - 1.0;
        self.release_step = if params[5] == 0.0 {
            1.0
        } else {
            1.0 / (sample_rate * params[5])
        };
        self.release_curve = 2.0 * params[6] - 1.0;
        let mix = 2.0 * params[7] - 1.0;
        self.floor = 10.0_f32.powf(-3.0 * mix.abs());
        self.inverted = mix < 0.0;
        self.hold_frames = (sample_rate * params[8]).trunc() as usize;
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.history[0][self.write_index] = l;
        self.history[1][self.write_index] = r;
        self.write_index = (self.write_index + 1) & 255;
        if self.write_index == 0 {
            for channel in 0..2 {
                let sum: f32 = self.history[channel]
                    .iter()
                    .map(|sample| sample * sample)
                    .sum();
                self.level_db[channel] = if sum <= 0.0 {
                    -60.0
                } else {
                    10.0 * (sum / 256.0).log10()
                };
            }
        }
        let level = self.level_db[0].max(self.level_db[1]);
        if level > self.threshold_db {
            self.open = true;
            self.hold_remaining = self.hold_frames;
        } else if level < self.threshold_db - self.asymmetry_db {
            if self.hold_remaining > 0 {
                self.hold_remaining -= 1;
            } else {
                self.open = false;
            }
        }
        if self.open {
            self.phase = (self.phase + self.attack_step).min(1.0);
        } else {
            self.phase = (self.phase - self.release_step).max(0.0);
        }
        let curve = if self.open {
            self.attack_curve
        } else {
            self.release_curve
        };
        let shaped = gate_curve(self.phase, curve);
        let envelope = if self.inverted { 1.0 - shaped } else { shaped };
        let target = self.floor + (1.0 - self.floor) * envelope;
        self.gain += 0.1 * (target - self.gain);
        (
            self.history[0][self.write_index] * self.gain,
            self.history[1][self.write_index] * self.gain,
        )
    }

    fn reset(&mut self) {
        self.history = [[0.0; 256]; 2];
        self.write_index = 0;
        self.level_db = [-60.0; 2];
        self.phase = 0.0;
        self.gain = 0.0;
        self.open = false;
        self.hold_remaining = 0;
    }
}

fn gate_curve(phase: f32, curvature: f32) -> f32 {
    if curvature == 0.0 || phase == 0.0 || phase == 1.0 {
        return phase;
    }
    let curved = if curvature > 0.0 {
        (phase * std::f32::consts::LN_10 * 3.0 - std::f32::consts::LN_10 * 3.0).exp()
    } else {
        (1.0 + phase.ln() / (std::f32::consts::LN_10 * 3.0)).max(0.0)
    };
    phase + curvature.abs() * (curved - phase)
}

#[derive(Default)]
struct VariableDelay {
    buffer: Vec<f32>,
    write_index: usize,
    current_samples: f32,
    target_samples: f32,
}

impl VariableDelay {
    fn resize(&mut self, maximum: usize) {
        let length = maximum.max(4) + 3;
        if self.buffer.len() != length {
            self.buffer = vec![0.0; length];
            self.write_index = 0;
        }
    }

    fn set_target(&mut self, samples: f32) {
        self.target_samples = samples.clamp(2.0, (self.buffer.len() - 2) as f32);
    }

    fn process(&mut self, input: f32, feedback: f32) -> f32 {
        self.current_samples += 0.001 * (self.target_samples - self.current_samples);
        let delayed = read_fractional(&self.buffer, self.write_index, self.current_samples);
        self.buffer[self.write_index] = flush(input + feedback * delayed);
        self.write_index = (self.write_index + 1) % self.buffer.len();
        delayed
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_index = 0;
        self.current_samples = self.target_samples;
    }
}

#[derive(Default)]
struct TapeDelayState {
    lines: [VariableDelay; 2],
    maximum: f32,
    normalized_delay: [f32; 2],
    feedback: f32,
    wet: f32,
}

impl TapeDelayState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        let capacity = sample_rate.trunc() as usize;
        self.maximum = capacity.max(4) as f32;
        for line in &mut self.lines {
            line.resize(capacity);
        }
        let base = params[1].powi(2);
        let async_value = params[3];
        self.normalized_delay = if async_value < 0.5 {
            [base * (async_value + 0.5), base]
        } else if async_value > 0.5 {
            [base, base * (1.5 - async_value)]
        } else {
            [base, base]
        };
        for channel in 0..2 {
            let samples = 2.0 + (self.maximum - 2.0) * self.normalized_delay[channel];
            self.lines[channel].set_target(samples);
        }
        self.feedback = params[2];
        self.wet = params[4] * params[2];
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let wet_l = self.lines[0].process(l, self.feedback);
        let wet_r = self.lines[1].process(r, self.feedback);
        (l + self.wet * wet_l, r + self.wet * wet_r)
    }

    fn reset(&mut self) {
        for line in &mut self.lines {
            line.reset();
        }
    }
}

#[derive(Default)]
struct FlangerState {
    lines: [VariableDelay; 2],
    phase: f64,
    increment: f64,
    beat_period: f64,
    free_rate_hz: f32,
    synced: bool,
    shape: u8,
    depth: f32,
    base: f32,
    stereo_offset: f64,
    feedback: f32,
    mix: f32,
    capacity: f32,
}

impl FlangerState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        const PERIODS: [f64; 13] = [
            0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0,
        ];
        self.beat_period = PERIODS[((params[1] * 12.0).trunc() as usize).min(12)];
        self.synced = params[2] != 0.0;
        self.free_rate_hz = 0.1 + 9.9 * params[3].powi(3);
        self.shape = ((params[4] * 2.0).round() as u8).min(2);
        self.base = 0.5 * params[6].powi(2);
        self.depth = 0.5 * self.base * params[5];
        self.feedback = params[7] * if params[8] != 0.0 { -1.0 } else { 1.0 };
        self.stereo_offset = 0.5 * f64::from(params[9]);
        self.mix = params[10].powi(2) * if params[11] != 0.0 { -1.0 } else { 1.0 };
        let capacity = (sample_rate * 0.005).trunc() as usize;
        self.capacity = capacity.max(4) as f32;
        for line in &mut self.lines {
            line.resize(capacity);
            line.set_target(2.0 + (self.capacity - 2.0) * self.base);
        }
    }

    fn prepare_block(&mut self, context: ProcessContext) {
        self.increment = if self.synced {
            if context.start_beat >= 0.0 {
                self.phase = (context.start_beat / self.beat_period).fract();
            }
            f64::from(context.tempo_bpm)
                / (60.0 * self.beat_period * f64::from(context.sample_rate))
        } else {
            f64::from(self.free_rate_hz) / f64::from(context.sample_rate)
        };
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let left_wave = simple_lfo(self.shape, self.phase);
        let right_wave = simple_lfo(self.shape, (self.phase + self.stereo_offset).fract());
        self.phase = (self.phase + self.increment).fract();
        for (channel, wave) in [left_wave, right_wave].into_iter().enumerate() {
            let normalized = (self.base + self.depth * wave).clamp(0.0, 1.0);
            self.lines[channel].set_target(2.0 + (self.capacity - 2.0) * normalized);
        }
        let wet_l = self.lines[0].process(l, self.feedback);
        let wet_r = self.lines[1].process(r, self.feedback);
        (l + self.mix * wet_l, r + self.mix * wet_r)
    }

    fn reset(&mut self) {
        for line in &mut self.lines {
            line.reset();
        }
        self.phase = 0.0;
        self.increment = 0.0;
    }
}

fn simple_lfo(shape: u8, phase: f64) -> f32 {
    match shape {
        1 => (2.0 * phase - 1.0) as f32,
        2 => {
            if phase <= 0.5 {
                -1.0
            } else {
                1.0
            }
        }
        _ => (phase * std::f64::consts::TAU).sin() as f32,
    }
}

struct WowFlutterState {
    lines: [Vec<f32>; 2],
    write_index: usize,
    sample_rate: f32,
    wow_max: f32,
    flutter_max: f32,
    wow_depth_target: f32,
    wow_depth: f32,
    flutter_depth_target: f32,
    flutter_depth: f32,
    base_wow_rate: f32,
    wow_rate_target: f32,
    wow_rate: f32,
    flutter_rate: f32,
    random_rate: f32,
    random_amount_target: f32,
    random_amount: f32,
    wow_period: f64,
    flutter_period: f64,
    synced: bool,
    phases: [f64; 2],
    increments: [f64; 2],
    outputs: [f32; 2],
    random_countdown: usize,
    random_seed: u64,
    /// The last frame's delay re-centred by half the wow and flutter depths, as a fraction of their maxima's sum: the
    /// trace's value, 0.5 at rest.
    trace: f32,
}

impl Default for WowFlutterState {
    fn default() -> Self {
        Self {
            lines: std::array::from_fn(|_| Vec::new()),
            write_index: 0,
            sample_rate: 0.0,
            wow_max: 0.0,
            flutter_max: 0.0,
            wow_depth_target: 0.0,
            wow_depth: 0.0,
            flutter_depth_target: 0.0,
            flutter_depth: 0.0,
            base_wow_rate: 1.0,
            wow_rate_target: 1.0,
            wow_rate: 1.0,
            flutter_rate: 6.0,
            random_rate: 1.0,
            random_amount_target: 0.0,
            random_amount: 0.0,
            wow_period: 0.25,
            flutter_period: 0.125,
            synced: false,
            phases: [0.0; 2],
            increments: [0.0; 2],
            outputs: [0.0; 2],
            random_countdown: 0,
            random_seed: 0x9e37_79b9_7f4a_7c15,
            trace: 0.5,
        }
    }
}

impl WowFlutterState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.wow_max = 0.142 * sample_rate;
        self.flutter_max = 0.003 * sample_rate;
        let length = ((self.wow_max + self.flutter_max).ceil() as usize)
            .max(2)
            .next_power_of_two();
        for line in &mut self.lines {
            if line.len() != length {
                *line = vec![0.0; length];
                self.write_index = 0;
            }
        }
        // Re-applying the same rate keeps the random wander's current target.
        let base_wow_rate = 0.1 + 4.9 * params[1];
        if base_wow_rate != self.base_wow_rate {
            self.base_wow_rate = base_wow_rate;
            self.wow_rate_target = base_wow_rate;
        }
        self.wow_period = wow_period(params[2]);
        self.wow_depth_target = (5.0 * params[3]).exp_m1() * 0.006_783_655;
        self.flutter_rate = 5.0 + 15.0 * params[4];
        self.flutter_period = wow_period(params[5]);
        self.flutter_depth_target = params[6];
        self.synced = params[7] > 0.5;
        self.random_rate = 0.1 + 9.9 * params[8];
        self.random_amount_target = params[10];
    }

    fn prepare_block(&mut self, context: ProcessContext) {
        if self.synced {
            self.increments = [
                f64::from(context.tempo_bpm)
                    / (60.0 * self.wow_period * f64::from(context.sample_rate)),
                f64::from(context.tempo_bpm)
                    / (60.0 * self.flutter_period * f64::from(context.sample_rate)),
            ];
            if context.start_beat >= 0.0 {
                self.phases[0] = (context.start_beat / self.wow_period).fract();
                self.phases[1] = (context.start_beat / self.flutter_period).fract();
            }
        } else {
            self.increments = [
                f64::from(self.wow_rate) / f64::from(context.sample_rate),
                f64::from(self.flutter_rate) / f64::from(context.sample_rate),
            ];
        }
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let smoothing = 44.1 / self.sample_rate;
        self.wow_depth += smoothing * (self.wow_depth_target - self.wow_depth);
        self.flutter_depth += smoothing * (self.flutter_depth_target - self.flutter_depth);
        self.random_amount += smoothing * (self.random_amount_target - self.random_amount);
        self.wow_rate += smoothing * (self.wow_rate_target - self.wow_rate);
        if self.random_countdown == 0 {
            self.random_seed ^= self.random_seed << 13;
            self.random_seed ^= self.random_seed >> 7;
            self.random_seed ^= self.random_seed << 17;
            let fraction = ((self.random_seed >> 40) as f32) / ((1_u64 << 24) as f32);
            let low = self.base_wow_rate + (0.1 - self.base_wow_rate) * self.random_amount;
            let high = self.base_wow_rate + (5.0 - self.base_wow_rate) * self.random_amount;
            self.wow_rate_target = low + fraction * (high - low);
            self.random_countdown = (self.sample_rate / self.random_rate).max(1.0) as usize;
        }
        self.random_countdown -= 1;
        let wow_delay = (0.5 * self.wow_max * (1.0 + self.outputs[0]) * self.wow_depth)
            .clamp(0.0, self.wow_max);
        let flutter_delay = (0.5 * self.flutter_max * (1.0 + self.outputs[1]) * self.flutter_depth)
            .clamp(0.0, self.flutter_max);
        let delay = wow_delay + flutter_delay;
        let span = self.wow_max + self.flutter_max;
        let centre = 0.5 * (self.wow_max * self.wow_depth + self.flutter_max * self.flutter_depth);
        self.trace = 0.5 + (delay - centre) / span.max(f32::MIN_POSITIVE);
        let output_l = write_and_read(&mut self.lines[0], self.write_index, l, delay);
        let output_r = write_and_read(&mut self.lines[1], self.write_index, r, delay);
        self.write_index = (self.write_index + 1) % self.lines[0].len();
        for channel in 0..2 {
            self.outputs[channel] = (std::f64::consts::TAU * self.phases[channel]).sin() as f32;
            self.phases[channel] = (self.phases[channel] + self.increments[channel]).fract();
        }
        if !self.synced {
            self.increments[0] = f64::from(self.wow_rate) / f64::from(self.sample_rate);
        }
        (output_l, output_r)
    }

    fn reset(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.write_index = 0;
        self.phases = [0.0; 2];
        self.outputs = [0.0; 2];
        self.random_countdown = 0;
        self.trace = 0.5;
    }
}

fn wow_period(value: f32) -> f64 {
    const PERIODS: [f64; 10] = [0.125, 0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0];
    PERIODS[((value * 9.0).trunc() as usize).min(9)]
}

#[derive(Default)]
struct TranceDelayState {
    left: Vec<f32>,
    right: Vec<f32>,
    aux: Vec<f32>,
    index_left: usize,
    index_right: usize,
    index_aux: usize,
    delay: usize,
    sample_rate: f32,
    beat_period: f64,
    mode: u8,
    feedback: f32,
    mix: f32,
    filter: f32,
    filter_state: [f32; 2],
    input_pan: f32,
    current_pan: f32,
}

impl TranceDelayState {
    fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
            let length = (sample_rate * 2.0).trunc().max(1.0) as usize;
            self.left.resize(length, 0.0);
            self.right.resize(length, 0.0);
            self.aux.resize(length, 0.0);
            self.reset();
        }
        // round(2v) = trunc(fma_f64(v, 2, 0.5)).
        let mode = f64::from(params[5]).mul_add(2.0, 0.5).trunc() as u8;
        if mode != self.mode {
            self.mode = mode;
            self.reset();
        }
        self.beat_period =
            [0.25, 0.375, 0.5, 0.75, 1.0][((params[1] * 4.0).trunc() as usize).min(4)];
        self.feedback = params[2] * 0.99;
        self.mix = params[3].powi(2);
        self.filter = params[4];
        self.input_pan = params[6];
    }

    fn prepare_block(&mut self, context: ProcessContext) {
        let frames = (f64::from(context.sample_rate) * 60.0 * self.beat_period
            / f64::from(context.tempo_bpm))
        .trunc() as usize;
        self.delay = frames.clamp(1, self.left.len());
    }

    fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.mode == 1 {
            return self.process_ping_pong(l, r);
        }
        let dry_left = if self.mode == 2 {
            let length = self.delay;
            if self.index_aux >= length {
                self.index_aux = 0;
            }
            self.aux[self.index_aux] = l;
            self.index_aux = (self.index_aux + 1) % length;
            self.aux[self.index_aux]
        } else {
            l
        };
        let length = if self.mode == 2 {
            self.delay.saturating_mul(2).min(self.left.len())
        } else {
            self.delay
        };
        if self.index_left >= length {
            self.index_left = 0;
        }
        if self.index_right >= length {
            self.index_right = 0;
        }
        let delayed_left = self.left[self.index_left] + 1e-30;
        let delayed_right = self.right[self.index_right] + 1e-30;
        let wet_left = self.filter_sample(0, delayed_left);
        let wet_right = self.filter_sample(1, delayed_right);
        self.left[self.index_left] = (dry_left + wet_left) * self.feedback;
        self.right[self.index_right] = (r + wet_right) * self.feedback;
        self.index_left = (self.index_left + 1) % length;
        self.index_right = (self.index_right + 1) % length;
        // The echo flushed as it is mixed: a low Mix scales a decaying tail into subnormals.
        (
            dry_left + flush(self.mix * wet_left),
            r + flush(self.mix * wet_right),
        )
    }

    fn process_ping_pong(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.index_left >= self.delay {
            self.index_left = 0;
        }
        if self.index_right >= self.delay {
            self.index_right = 0;
        }
        if self.current_pan < 0.0 {
            self.current_pan = self.input_pan;
        } else if self.current_pan != self.input_pan {
            self.current_pan += (self.input_pan - self.current_pan) * 0.01;
            if (self.input_pan - self.current_pan).abs() < 0.001 {
                self.current_pan = self.input_pan;
            }
        }
        let left_weight = (2.0 - 2.0 * self.current_pan).clamp(0.0, 1.0);
        let right_weight = (2.0 * self.current_pan).clamp(0.0, 1.0);
        let delayed_left = self.left[self.index_left] + 1e-30;
        let delayed_right = self.right[self.index_right] + 1e-30;
        let wet_left = self.filter_sample(0, delayed_left);
        let wet_right = self.filter_sample(1, delayed_right);
        self.left[self.index_left] = (r * right_weight + wet_right) * self.feedback;
        self.right[self.index_right] = (l * left_weight + wet_left) * self.feedback;
        self.index_left = (self.index_left + 1) % self.delay;
        self.index_right = (self.index_right + 1) % self.delay;
        (
            l + flush(self.mix * wet_left),
            r + flush(self.mix * wet_right),
        )
    }

    fn filter_sample(&mut self, channel: usize, input: f32) -> f32 {
        let control = self.filter * 2.0;
        let coefficient = if control <= 1.0 {
            control
        } else {
            (control - 1.0).powi(4)
        };
        // Flushed as the other filter states are: once the input is silent it runs on the line's 1e-30 floor.
        self.filter_state[channel] =
            flush(self.filter_state[channel] + (input - self.filter_state[channel]) * coefficient);
        if control <= 1.0 {
            self.filter_state[channel]
        } else {
            input - self.filter_state[channel]
        }
    }

    fn reset(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.aux.fill(0.0);
        self.index_left = 0;
        self.index_right = 0;
        self.index_aux = 0;
        self.filter_state = [0.0; 2];
        self.current_pan = -1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stepped selector's `law` at the edges of its m + 1 stepped values:
    /// one ulp below f32(k/m) for k = 1…m, at it for k = 0…m, and one ulp above it for k = 0…m − 1.
    pub(crate) fn step_edges<T>(m: u16, law: impl Fn(f32) -> T) -> [Vec<T>; 3] {
        let step = |k: u16| f32::from(k) / f32::from(m);
        [
            (1..=m).map(|k| law(step(k).next_down())).collect(),
            (0..=m).map(|k| law(step(k))).collect(),
            (0..m).map(|k| law(step(k).next_up())).collect(),
        ]
    }

    /// Effect `id` at 48 kHz with `params` set.
    fn effect_with(id: u16, params: &[(usize, f32)]) -> AudioEffect {
        let mut effect = AudioEffect::new(id, 48_000.0).unwrap();
        for &(index, value) in params {
            effect.set_parameter(index, value).unwrap();
        }
        effect
    }

    /// The stereo response of effect `id` with `params` to a unit impulse, left then right.
    fn impulse_response(id: u16, params: &[(usize, f32)]) -> Vec<f32> {
        let mut effect = effect_with(id, params);
        let (mut left, mut right) = (vec![0.0; 256], vec![0.0; 256]);
        (left[0], right[0]) = (1.0, 1.0);
        let context = ProcessContext {
            sample_rate: 48_000.0,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        };
        effect.process(&mut left, &mut right, context).unwrap();
        left.extend(right);
        left
    }

    /// The 13 synced LFO periods of Flanger and Phaser.
    const LFO_PERIODS: [f64; 13] = [
        0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0,
    ];

    #[test]
    fn flanger_and_phaser_selectors_follow_round_15() {
        // LFO Rate trunc(f32(12v)): the f32 product rounds f32(5/12) up to 5, and one ulp below f32(11/12) up to 11.
        let rate = [
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 11],
            (0..13).collect(),
            (0..12).collect(),
        ]
        .map(|ks: Vec<usize>| ks.into_iter().map(|k| LFO_PERIODS[k]).collect::<Vec<_>>());
        // LFO Shape, the act-ID form with n = 3: round-half-away(f32(2v)), so ¼ and ¾ round up.
        let shape = [vec![0, 1, 1, 2], vec![0, 1, 1, 2, 2], vec![0, 1, 1, 2]];
        let flanger = |index: usize, v: f32| {
            let Processor::Flanger(state) = effect_with(13, &[(index, v)]).processor else {
                unreachable!()
            };
            state
        };
        assert_eq!(step_edges(12, |v| flanger(1, v).beat_period), rate);
        assert_eq!(step_edges(4, |v| flanger(4, v).shape), shape);
        let phaser = |index: usize, v: f32| {
            let Processor::Phaser(state) = effect_with(14, &[(index, v)]).processor else {
                unreachable!()
            };
            state
        };
        assert_eq!(step_edges(12, |v| phaser(1, v).period_beats), rate);
        assert_eq!(step_edges(4, |v| phaser(8, v).shape), shape);
    }

    #[test]
    fn the_flanger_default_rate_runs_1_1() {
        // A default quirk: the stored 0.4 gives trunc(4.8000002) = 4, the `1/1` period, while
        // the text rounds to `2/1`. Picking `2/1` writes 5/12, which the DSP reads as 5.
        let Processor::Flanger(state) = AudioEffect::new(13, 48_000.0).unwrap().processor else {
            unreachable!()
        };
        assert_eq!(state.beat_period, 1.0);
        let Processor::Flanger(state) = effect_with(13, &[(1, 5.0 / 12.0)]).processor else {
            unreachable!()
        };
        assert_eq!(state.beat_period, 2.0);
    }

    #[test]
    fn spacer_selectors_round_half_away_in_f32() {
        // p4 and p6 round(f32(3v)), p7 round(f32(2v)): the f32 product rounds f32(5/6) up to 2.5, so 3.
        let spacer = |index: usize, v: f32| {
            let Processor::Spacer(state) = effect_with(34, &[(index, v)]).processor else {
                unreachable!()
            };
            state
        };
        let thirds = [
            vec![0, 1, 1, 2, 2, 3],
            vec![0, 1, 1, 2, 2, 3, 3],
            vec![0, 1, 1, 2, 2, 3],
        ];
        assert_eq!(step_edges(6, |v| spacer(4, v).inversion_lr), thirds);
        assert_eq!(step_edges(6, |v| spacer(6, v).inversion_ms), thirds);
        assert_eq!(
            step_edges(4, |v| spacer(7, v).mode),
            [vec![0, 1, 1, 2], vec![0, 1, 1, 2, 2], vec![0, 1, 1, 2]]
        );
    }

    #[test]
    fn trance_delay_mode_rounds_in_f64() {
        // p5 trunc(fma_f64(v, 2, 0.5)): one ulp below ¼ is still Stereo (an f32 sum would round up to 1).
        let mode = |v: f32| {
            let Processor::TranceDelay(state) = effect_with(16, &[(5, v)]).processor else {
                unreachable!()
            };
            state.mode
        };
        assert_eq!(
            step_edges(4, mode),
            [vec![0, 1, 1, 2], vec![0, 1, 1, 2, 2], vec![0, 1, 1, 2]]
        );
    }

    /// Which stepped value k/m param `index` of effect `id` (over `base`) plays `v` as, told apart by the impulse
    /// response.
    fn played_step(
        id: u16,
        base: &[(usize, f32)],
        index: usize,
        m: u16,
    ) -> impl Fn(f32) -> usize + '_ {
        let response = move |v: f32| impulse_response(id, &[base, &[(index, v)]].concat());
        let steps: Vec<_> = (0..=m)
            .map(|k| response(f32::from(k) / f32::from(m)))
            .collect();
        assert!(
            (1..steps.len()).all(|i| !steps[..i].contains(&steps[i])),
            "the stepped values sound alike"
        );
        move |v| {
            let played = response(v);
            (steps.iter().position(|step| *step == played)).expect("a stepped response")
        }
    }

    #[test]
    fn equalizer_kind_and_channel_truncate() {
        // Kind trunc(2v) in f64, Channel trunc(4v), heard on band 1 at +6 dB.
        let kind = played_step(3, &[(1, 0.75)], 4, 2);
        assert_eq!(step_edges(2, kind), [vec![0, 1], vec![0, 1, 2], vec![0, 1]]);
        let channel = played_step(3, &[(1, 0.75)], 17, 4);
        assert_eq!(
            step_edges(4, &channel),
            [vec![0, 1, 2, 3], vec![0, 1, 2, 3, 4], vec![0, 1, 2, 3]]
        );
        // Between the steps: ⅛ is still Both (round(4v) would give Left).
        assert_eq!(channel(0.125), 0);
    }

    #[test]
    fn tuned_eq_key_is_a_fused_f64_sum() {
        // Tuning p9 key = trunc(fma_f64(v, 11, f32(0.001))): the bias keeps one ulp below f32(k/11) on key k. Key k starts
        // at the first f32 at or above (k − 0.001000000047)/11; an f32 product and sum would already give k one ulp
        // below that for k = 1, 5, 9, 10, 11.
        let gains: Vec<(usize, f32)> = (1..=8).map(|band| (band, 0.75)).collect();
        let key = played_step(30, &gains, 9, 11);
        assert_eq!(
            step_edges(11, &key),
            [
                (1..=11).collect::<Vec<_>>(),
                (0..=11).collect(),
                (0..=10).collect()
            ]
        );
        let starts = [
            0x3db9_fee3,
            0x3e3a_16b7,
            0x3e8b_96ff,
            0x3eba_22a2,
            0x3ee8_ae45,
            0x3f0b_9cf4,
            0x3f22_e2c5,
            0x3f3a_2897,
            0x3f51_6e68,
            0x3f68_b43a,
            0x3f7f_fa0b,
        ]
        .map(f32::from_bits);
        assert_eq!(starts.map(&key), [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
        assert_eq!(
            starts.map(|start| key(start.next_down())),
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
        );
    }

    #[test]
    fn the_compressor_s_detector_settles_to_zero() {
        // Subnormal states never reach 0 and cost the audio thread up to 100× per operation.
        let mut effect = AudioEffect::new(8, 8_000.0).unwrap();
        let context = ProcessContext {
            sample_rate: 8_000.0,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        };
        let mut burst = [0.8; 400];
        effect
            .process(&mut burst.clone(), &mut burst, context)
            .unwrap();
        for _ in 0..1_200 {
            let (mut left, mut right) = ([0.0; 400], [0.0; 400]);
            effect.process(&mut left, &mut right, context).unwrap();
        }
        let Processor::Compressor(state) = &effect.processor else {
            unreachable!()
        };
        assert_eq!((state.held_peak, state.envelope), (0.0, 0.0));
    }

    #[test]
    fn the_wet_pan_smoother_settles_to_zero() {
        // Wet Pan fully left: the smoother heads to 0 and must not stop on a subnormal. (Mix 0 bypasses the module, so
        // its smoother never heads to 0.)
        let mut common = CommonModule::default();
        // A frame first: a value set before any is where the module starts, with nothing to glide.
        common.frame((0.5, 0.5), (0.25, 0.25));
        common.set_parameter(0x7fc2, 0.0).unwrap();
        for _ in 0..200_000 {
            common.frame((0.5, 0.5), (0.25, 0.25));
        }
        assert_eq!(common.current_pan, 0.0);
    }

    #[test]
    fn values_set_before_the_first_frame_hold_from_it_and_later_ones_glide() {
        // A saved Post Gain, Mix and Wet Pan are where the module starts, not where it glides to.
        let mut common = CommonModule::default();
        common.set_parameter(0x7fc0, 0.0).unwrap();
        common.set_parameter(0x7fc1, 0.5).unwrap();
        common.set_parameter(0x7fc2, 0.0).unwrap();
        let (l, r) = common.frame((0.0, 0.0), (1.0, 1.0));
        assert!((l - 0.05).abs() < 1e-6 && r == 0.0, "{l} {r}");
        // From then on a change glides: Post Gain back to 0 dB moves 1 % of the way per frame.
        common.set_parameter(0x7fc0, 0.5).unwrap();
        let (l, _) = common.frame((0.0, 0.0), (1.0, 1.0));
        assert!((l - 0.5 * (0.1 + 0.9 * 0.01)).abs() < 1e-6, "{l}");
    }

    #[test]
    fn a_module_bypassed_from_its_build_glides_into_its_first_mix() {
        // Mix 0 at build: its blocks run no frame, yet a later Mix 1 glides from 0 (0.001 a frame), not a snap.
        let mut effect = AudioEffect::new(8, 8_000.0).unwrap();
        effect.set_common_parameter(0x7fc1, 0.0).unwrap();
        let context = ProcessContext {
            sample_rate: 8_000.0,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        };
        let (mut left, mut right) = ([0.5; 400], [0.5; 400]);
        effect.process(&mut left, &mut right, context).unwrap();
        effect.set_common_parameter(0x7fc1, 1.0).unwrap();
        let (mut left, mut right) = ([0.5; 1], [0.5; 1]);
        effect.process(&mut left, &mut right, context).unwrap();
        assert!(
            (effect.common.current_mix - 0.001).abs() < 1e-6,
            "{}",
            effect.common.current_mix
        );
    }

    #[test]
    fn the_multiband_detectors_settle_to_zero() {
        let mut effect = AudioEffect::new(36, 8_000.0).unwrap();
        let context = ProcessContext {
            sample_rate: 8_000.0,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        };
        // Dry Mix (p5) up for the burst, then down: the dry smoother has a value to decay from.
        effect.set_parameter(5, 1.0).unwrap();
        let mut burst = [0.8; 400];
        effect
            .process(&mut burst.clone(), &mut burst, context)
            .unwrap();
        effect.set_parameter(5, 0.0).unwrap();
        for _ in 0..1_200 {
            let (mut left, mut right) = ([0.0; 400], [0.0; 400]);
            effect.process(&mut left, &mut right, context).unwrap();
        }
        let Processor::Multiband(state) = &effect.processor else {
            unreachable!()
        };
        for [peak, envelope, dry] in state.band_states() {
            assert_eq!((peak, envelope), (0.0, 0.0));
            assert_eq!(dry, 0.0);
        }
    }
}

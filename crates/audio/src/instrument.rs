//! Rack instruments (generators): the trait the engine drives, and the module that wraps
//! one with its common params.

use crate::minisynth::MiniSynth;
use crate::{CommonModule, MeterSnapshot, ProcessContext};

/// An instrument as the engine drives it. The engine builds it on the app thread; every method here runs on the
/// audio thread, so none may allocate, lock or do I/O.
pub trait Instrument: Send {
    /// Sets param `index` to `value`, normalized 0..=1. p0 is Enable: the
    /// [`InstrumentModule`] skips [`Self::process`] while it is off (or while Mix is 0), and on its On→Off edge resets
    /// the instrument unless [`Self::freezes_when_disabled`]. An unknown index is ignored.
    fn set_parameter(&mut self, index: usize, value: f32);
    /// Starts note `key` (0..=127). `velocity` is linear in (0, 1]: the note's raw EVN2 velocity byte a / 255,
    /// or the keyboard's; the gain law is the instrument's. `pan` is in 0..=1: a note's raw EVN2 b / 255, where
    /// b = 127 is centre, so centre arrives as 127/255; the keyboard sends 0.5. The
    /// instrument applies it per voice with its own pan law.
    fn note_on(&mut self, key: u8, velocity: f32, pan: f32);
    /// Releases a sounding `key`.
    fn note_off(&mut self, key: u8);
    /// Releases every voice; their releases still sound (Stop).
    fn all_notes_off(&mut self);
    /// Silences at once: voices, delay lines and modulation phases are cleared, params kept (Enable switched off).
    fn reset(&mut self);
    /// Renders one block and adds it into `left` and `right`, which have equal lengths. `context` gives the song
    /// position of the block's first frame (`start_beat`), the tempo and whether the transport runs (`playing`), for
    /// LFOs that lock to the song while it plays and run free otherwise.
    fn process(&mut self, left: &mut [f32], right: &mut [f32], context: ProcessContext);
    /// The song tempo, for tempo-synced params. The engine calls it once after construction.
    fn set_tempo_bpm(&mut self, _bpm: f32) {}
    /// Channel param `index` is now `value`, the param's normalized v in 0..=1: 4 Pitch Bend,
    /// 5 Aftertouch, 7 + n MIDI CC #n. The instrument applies the laws: Pitch Bend reaches modules as w = 4v − 2.
    /// The engine calls it for every channel param ≥ 4, at build, on seek and on each
    /// change, except the ones it handles itself: CC #7 (Level), CC #8 and #10 (Pan) and CC #64 (the sustain
    /// pedal holds note-offs), which only an instrument that [takes the pedal](Self::takes_pedal) gets.
    fn set_controller(&mut self, _index: usize, _value: f32) {}
    /// Issue #33: whether it takes the sustain pedal itself (a hosted plugin: its own pedal, resonance and release
    /// samples). It then gets CC #64 (param 71) through [`Self::set_controller`] as it moves, half-pedal values too, and
    /// its note-offs at once: the engine holds none for it.
    fn takes_pedal(&self) -> bool {
        false
    }
    /// The `CSTM` setting `key` is now `value`.
    fn set_setting(&mut self, _key: u32, _value: f32) {}
    /// A slide event: every voice's slide offsets are now `pitch` semitones, `pan`
    /// and `velocity`, added to its key, note pan and velocity, without smoothing; a newly launched voice starts at 0.
    /// The default ignores slides, as the Transistor Bass and Slicer do.
    fn set_slide(&mut self, _pitch: f32, _pan: f32, _velocity: f32) {}
    /// Whether the voices freeze while Enable is off (not rendered, not advanced) and resume when it is back on:
    /// Slicer, 3x Osc and SoundFont Player. The default, false, resets the instrument at once on the On→Off edge:
    /// MiniSynth, GMSynth, DW Sampler, Transistor Bass and SuperSaw.
    fn freezes_when_disabled(&self) -> bool {
        false
    }
}

/// The channel param MiniSynth reads as its mod wheel: MIDI CC #1 = 7 + 1.
const MINISYNTH_MOD_WHEEL: usize = 8;
/// Channel param 4, Pitch Bend: MiniSynth bends 4v − 2 semitones.
const MINISYNTH_PITCH_BEND: usize = 4;

impl Instrument for MiniSynth {
    fn set_parameter(&mut self, index: usize, value: f32) {
        let _ = MiniSynth::set_parameter(self, index, value);
    }

    fn note_on(&mut self, key: u8, velocity: f32, pan: f32) {
        self.note_on_with_pan(key, velocity, pan);
    }

    fn note_off(&mut self, key: u8) {
        MiniSynth::note_off(self, key);
    }

    fn all_notes_off(&mut self) {
        MiniSynth::all_notes_off(self);
    }

    fn reset(&mut self) {
        MiniSynth::reset(self);
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], context: ProcessContext) {
        if context.playing {
            self.set_song_beat(context.start_beat);
        }
        let _ = MiniSynth::set_tempo_bpm(self, context.tempo_bpm);
        let _ = MiniSynth::process(self, left, right);
    }

    fn set_tempo_bpm(&mut self, bpm: f32) {
        let _ = MiniSynth::set_tempo_bpm(self, bpm);
    }

    fn set_controller(&mut self, index: usize, value: f32) {
        match index {
            MINISYNTH_PITCH_BEND => self.set_pitch_bend(value),
            MINISYNTH_MOD_WHEEL => {
                let _ = self.set_mod_wheel(value);
            }
            _ => {}
        }
    }

    fn set_slide(&mut self, pitch: f32, pan: f32, velocity: f32) {
        MiniSynth::slide(self, pitch, pan, velocity);
    }
}

/// The engine-only setting key that turns a module's key-learn on (> 0.5) or off ([`InstrumentModule::set_setting`]).
/// Panel `#n` keys are small, so this one never meets a `CSTM` setting.
pub const KEY_LEARN: u32 = u32::MAX;

/// An instrument in a rack: its Enable (p0), its common params, its key range and a scratch block. It runs at its own
/// rack position, in place on the channel's running buffer: the
/// instrument renders into the scratch block, and the common params mix with dry = the running buffer (the audio from
/// the modules above) and wet = the running buffer + the instrument.
pub struct InstrumentModule {
    pub instrument: Box<dyn Instrument>,
    enabled: bool,
    common: CommonModule,
    left: Vec<f32>,
    right: Vec<f32>,
    meter: MeterSnapshot,
    /// The `ADD1` key range, both bounds inclusive. Full by default.
    pub keys: (u8, u8),
    /// Its key-learn is active (`Set Range` waits for a key): it hears no notes.
    learning: bool,
    /// How many of the channel's effects sit above it in the rack: the engine runs those first.
    pub effects_above: usize,
    /// What the host built it from besides its params (its type and the data it was built with), 0 by default: an
    /// engine built after an edit takes over a running module only when this matches.
    pub origin: u64,
}

impl InstrumentModule {
    /// Wraps `instrument`, with scratch room for `max_block` frames (larger blocks are rendered in parts).
    pub fn new(instrument: Box<dyn Instrument>, max_block: usize) -> Self {
        Self {
            instrument,
            enabled: true,
            common: CommonModule::default(),
            left: vec![0.0; max_block.max(1)],
            right: vec![0.0; max_block.max(1)],
            meter: MeterSnapshot::default(),
            keys: (0, 127),
            learning: false,
            effects_above: 0,
            origin: 0,
        }
    }

    /// Whether note `key`, on or off, reaches the instrument: inside its key range while it is not learning one.
    /// The per-module octave shift that would add 12·shift after this check is always
    /// 0, so keys pass unchanged.
    pub fn hears(&self, key: u8) -> bool {
        !self.learning && (self.keys.0..=self.keys.1).contains(&key)
    }

    /// The `CSTM` setting `key` for the instrument, or [`KEY_LEARN`] for the module.
    pub fn set_setting(&mut self, key: u32, value: f32) {
        match key {
            KEY_LEARN => self.learning = value > 0.5,
            _ => self.instrument.set_setting(key, value),
        }
    }

    /// Param `index` ≥ 0 of the instrument (p0 = Enable), or common param −1 Post Gain, −2 Mix, −3 Wet Pan.
    /// Other indices and values outside 0..=1 are ignored.
    pub fn set_parameter(&mut self, index: i32, value: f32) {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return;
        }
        match index {
            0.. => {
                if index == 0 {
                    let enabled = value >= 0.5;
                    if self.enabled && !enabled && !self.instrument.freezes_when_disabled() {
                        self.instrument.reset();
                    }
                    self.enabled = enabled;
                }
                self.instrument.set_parameter(index as usize, value);
            }
            -3..=-1 => {
                let _ = self
                    .common
                    .set_parameter((0x7fc0 - 1 - index) as u16, value);
            }
            _ => {}
        }
    }

    /// Renders one block at `context` into the running buffer `left` and `right` (equal lengths), in place: wet =
    /// running + instrument, then Post Gain, Mix and Wet Pan against dry = running, so at Mix 1 and Wet Pan centred
    /// the whole buffer is × Post Gain. Each part of a
    /// block longer than the scratch gets its own `start_beat`. While Enable is off or Mix is 0 the instrument is
    /// skipped and the buffer passes unchanged, as an effect's processor is bypassed (its voices were reset at the
    /// Enable edge unless it freezes). A part that is all silence, instrument and buffer, skips the common params, so
    /// their smoothing holds until signal is present (block by block, not a one-time latch). The meter holds the
    /// instrument's own output, before the common params.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], context: ProcessContext) {
        self.meter = MeterSnapshot::default();
        // A block has run: from now on a change glides, even through parts of silence that run no frame.
        self.common.fresh = false;
        if !self.common.runs(self.enabled) {
            return;
        }
        let block = self.left.len();
        let beats_per_frame =
            f64::from(context.tempo_bpm) / (60.0 * f64::from(context.sample_rate));
        let parts = left.chunks_mut(block).zip(right.chunks_mut(block));
        for (part, (out_l, out_r)) in parts.enumerate() {
            let frames = out_l.len();
            let offset = if context.playing { part * block } else { 0 };
            let context = ProcessContext {
                start_beat: context.start_beat + offset as f64 * beats_per_frame,
                ..context
            };
            let (wet_l, wet_r) = (&mut self.left[..frames], &mut self.right[..frames]);
            wet_l.fill(0.0);
            wet_r.fill(0.0);
            self.instrument.process(wet_l, wet_r, context);
            // Post Gain, Mix and Wet Pan apply only while some signal is present: a part where the
            // instrument and the buffer are both exact silence stays 0 and their smoothing holds.
            let signals = [&*wet_l, &*wet_r, &*out_l, &*out_r];
            if signals
                .iter()
                .all(|signal| signal.iter().all(|&s| s == 0.0))
            {
                continue;
            }
            for (frame, (l, r)) in out_l.iter_mut().zip(out_r.iter_mut()).enumerate() {
                let (own_l, own_r) = (wet_l[frame], wet_r[frame]);
                (*l, *r) = self.common.frame((*l, *r), (*l + own_l, *r + own_r));
                self.meter.peak_left = self.meter.peak_left.max(own_l.abs());
                self.meter.peak_right = self.meter.peak_right.max(own_r.abs());
            }
        }
    }

    /// The last block's peaks of the instrument's own output (no gain reduction).
    pub fn meter(&self) -> MeterSnapshot {
        self.meter
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const CONTEXT: ProcessContext = ProcessContext {
        sample_rate: 48_000.0,
        tempo_bpm: 120.0,
        start_beat: 1.0,
        playing: true,
    };

    /// Records the (start beat, playing) of every block it renders.
    struct Probe(Arc<Mutex<Vec<(f64, bool)>>>);

    impl Instrument for Probe {
        fn set_parameter(&mut self, _: usize, _: f32) {}
        fn note_on(&mut self, _: u8, _: f32, _: f32) {}
        fn note_off(&mut self, _: u8) {}
        fn all_notes_off(&mut self) {}
        fn reset(&mut self) {}
        fn process(&mut self, _: &mut [f32], _: &mut [f32], context: ProcessContext) {
            self.0
                .lock()
                .unwrap()
                .push((context.start_beat, context.playing));
        }
    }

    #[test]
    fn each_part_of_a_block_gets_its_own_song_position() {
        let blocks = Arc::new(Mutex::new(Vec::new()));
        let mut module = InstrumentModule::new(Box::new(Probe(blocks.clone())), 64);
        let (mut left, mut right) = (vec![0.0; 200], vec![0.0; 200]);
        module.process(&mut left, &mut right, CONTEXT);
        // 120 BPM at 48 kHz: a beat is 24 000 frames.
        let starts: Vec<f64> = blocks
            .lock()
            .unwrap()
            .iter()
            .map(|(beat, _)| *beat)
            .collect();
        assert_eq!(starts.len(), 4);
        for (start, frames) in starts.iter().zip([0.0, 64.0, 128.0, 192.0]) {
            assert!((start - (1.0 + frames / 24_000.0)).abs() < 1e-12, "{start}");
        }
        assert!(blocks.lock().unwrap().iter().all(|(_, playing)| *playing));

        blocks.lock().unwrap().clear();
        let stopped = ProcessContext {
            playing: false,
            ..CONTEXT
        };
        module.process(&mut left, &mut right, stopped);
        assert_eq!(*blocks.lock().unwrap(), [(1.0, false); 4]);
    }

    fn module(max_block: usize) -> InstrumentModule {
        let mut module =
            InstrumentModule::new(Box::new(MiniSynth::new(48_000.0).unwrap()), max_block);
        module.set_parameter(-1, 0.3);
        module.set_parameter(-3, 0.2);
        module.instrument.note_on(60, 1.0, 0.5);
        module
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let mut whole = (vec![0.0; 3000], vec![0.0; 3000]);
        module(1024).process(&mut whole.0, &mut whole.1, CONTEXT);
        let mut parts = (vec![0.0; 3000], vec![0.0; 3000]);
        let mut small = module(64);
        let mut start = 0;
        for size in [1, 7, 333, 1023, 1636] {
            let end = start + size;
            small.process(&mut parts.0[start..end], &mut parts.1[start..end], CONTEXT);
            start = end;
        }
        assert!(whole.0.iter().any(|sample| sample.abs() > 0.01));
        assert_eq!(whole, parts);
    }

    /// Counts its resets; freezes while disabled when `freezes`.
    struct Resets {
        count: Arc<Mutex<usize>>,
        freezes: bool,
    }

    impl Instrument for Resets {
        fn set_parameter(&mut self, _: usize, _: f32) {}
        fn note_on(&mut self, _: u8, _: f32, _: f32) {}
        fn note_off(&mut self, _: u8) {}
        fn all_notes_off(&mut self) {}
        fn reset(&mut self) {
            *self.count.lock().unwrap() += 1;
        }
        fn process(&mut self, _: &mut [f32], _: &mut [f32], _: ProcessContext) {}
        fn freezes_when_disabled(&self) -> bool {
            self.freezes
        }
    }

    #[test]
    fn enable_off_edge_resets_unless_the_instrument_freezes() {
        // Enable switched off.
        for (freezes, expected) in [(false, 1), (true, 0)] {
            let count = Arc::new(Mutex::new(0));
            let instrument = Resets {
                count: count.clone(),
                freezes,
            };
            let mut module = InstrumentModule::new(Box::new(instrument), 64);
            module.set_parameter(0, 1.0);
            module.set_parameter(0, 0.0);
            module.set_parameter(0, 0.2);
            module.set_parameter(0, 1.0);
            assert_eq!(*count.lock().unwrap(), expected, "freezes {freezes}");
        }
    }

    #[test]
    fn mix_zero_or_enable_off_passes_the_buffer_from_above_unchanged() {
        // Not rendered, nothing added.
        for (index, name) in [(-2, "Mix 0"), (0, "Enable off")] {
            let mut module = module(64);
            module.set_parameter(index, 0.0);
            let above: Vec<f32> = (0..512).map(|i| (i as f32 * 0.1).sin()).collect();
            let (mut left, mut right) = (above.clone(), above.clone());
            module.process(&mut left, &mut right, CONTEXT);
            assert_eq!((left, right), (above.clone(), above), "{name}");
        }
    }

    /// One frame with signal at the defaults: a value set from then on is a live change, which glides (a value set
    /// before the first frame is where the module starts).
    fn live(module: &mut InstrumentModule) {
        let (mut left, mut right) = (vec![0.5; 1], vec![0.5; 1]);
        module.process(&mut left, &mut right, CONTEXT);
    }

    #[test]
    fn post_gain_holds_its_smoothing_while_silent() {
        // Post Gain applies only while some signal is present, so a change during silence still ramps
        // at the next onset.
        let mut module = InstrumentModule::new(Box::new(Probe(Arc::default())), 64);
        live(&mut module);
        module.set_parameter(-1, 0.0);
        let (mut left, mut right) = (vec![0.0; 2000], vec![0.0; 2000]);
        module.process(&mut left, &mut right, CONTEXT);
        assert!(left.iter().chain(&right).all(|&s| s == 0.0));
        let (mut left, mut right) = (vec![0.5; 64], vec![0.5; 64]);
        module.process(&mut left, &mut right, CONTEXT);
        // One 0.01 step from 0 dB toward −20 dB: 0.5 × (1 − 0.009).
        assert!((left[0] - 0.4955).abs() < 1e-6, "{}", left[0]);
    }

    #[test]
    fn a_change_after_silent_blocks_glides_at_the_first_sound() {
        // Built, then only silence (no frame runs): Post Gain → −20 dB still glides from 0 dB at the onset.
        let mut module = InstrumentModule::new(Box::new(Probe(Arc::default())), 64);
        let (mut left, mut right) = (vec![0.0; 2000], vec![0.0; 2000]);
        module.process(&mut left, &mut right, CONTEXT);
        module.set_parameter(-1, 0.0);
        let (mut left, mut right) = (vec![0.5; 64], vec![0.5; 64]);
        module.process(&mut left, &mut right, CONTEXT);
        assert!((left[0] - 0.4955).abs() < 1e-6, "{}", left[0]);
    }

    #[test]
    fn post_gain_scales_the_running_buffer() {
        // Mix 1, Wet Pan centred: the whole running buffer × Post Gain, the audio from above included.
        let mut module = InstrumentModule::new(Box::new(Probe(Arc::default())), 64);
        live(&mut module);
        module.set_parameter(-1, 0.0);
        let (mut left, mut right) = (vec![0.5; 2000], vec![-0.5; 2000]);
        module.process(&mut left, &mut right, CONTEXT);
        // −20 dB, once the 0.01-per-sample smoothing has snapped.
        assert!((left[1999] - 0.05).abs() < 1e-6, "{}", left[1999]);
        assert!((right[1999] + 0.05).abs() < 1e-6, "{}", right[1999]);
        assert!(
            left[0] < 0.5 && left[0] > 0.49,
            "smoothed from 0 dB: {}",
            left[0]
        );
    }

    #[test]
    fn enable_is_on_from_one_half() {
        // 0.5 renders and is no off edge; 0.001 is off.
        let blocks = Arc::new(Mutex::new(Vec::new()));
        let mut module = InstrumentModule::new(Box::new(Probe(blocks.clone())), 64);
        let (mut left, mut right) = (vec![0.0; 64], vec![0.0; 64]);
        for (value, renders) in [(0.5, true), (0.001, false), (0.5, true)] {
            blocks.lock().unwrap().clear();
            module.set_parameter(0, value);
            module.process(&mut left, &mut right, CONTEXT);
            assert_eq!(
                blocks.lock().unwrap().is_empty(),
                !renders,
                "Enable {value}"
            );
        }
        let count = Arc::new(Mutex::new(0));
        let instrument = Resets {
            count: count.clone(),
            freezes: false,
        };
        let mut module = InstrumentModule::new(Box::new(instrument), 64);
        module.set_parameter(0, 0.5);
        assert_eq!(*count.lock().unwrap(), 0, "0.5 is on");
        module.set_parameter(0, 0.001);
        assert_eq!(*count.lock().unwrap(), 1, "0.001 is off");
    }
}

//! 3x Osc per-target LFOs and the per-voice modulation set the voice drives through [`VoiceMod`]
//! (the voice applies the depths). Tick-driven and allocation-free.

use std::f32::consts::TAU;

use super::env::{Env, EnvParams, time_ticks};
use super::{ModFrame, Params, Target, Tick, VoiceMod, toggle};

/// The LFO period is at least 3 ticks: phase += 1/max(3, N_S).
pub const MIN_PERIOD_TICKS: u32 = 3;

const TARGETS: [Target; 5] = [
    Target::Pan,
    Target::Vol,
    Target::Cut,
    Target::Res,
    Target::Pitch,
];
/// The enveloped targets, in [`Modulation`]'s order.
const ENV_TARGETS: [Target; 4] = [Target::Vol, Target::Cut, Target::Res, Target::Pitch];

/// One target's LFO params (Pan, Vol, ModX = Cut, ModY = Res, Pitch), normalized. Amount is the voice's depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LfoParams {
    pub global: bool,
    pub tempo: bool,
    /// 0 Sine, 1 Triangle, 2 Square.
    pub shape: u8,
    pub delay: f32,
    pub attack: f32,
    pub speed: f32,
}

impl LfoParams {
    /// `target`'s LFO from the param array (Global 27 + t, Tempo 32 + t, Shape 37 + t, Predelay 42 + t,
    /// Attack 47 + t, Speed 57 + t; t = `target as usize`).
    pub fn read(params: &Params, target: Target) -> Self {
        let t = target as usize;
        Self {
            global: toggle(params[27 + t]),
            tempo: toggle(params[32 + t]),
            shape: (params[37 + t] * 2.0).round() as u8,
            delay: params[42 + t],
            attack: params[47 + t],
            speed: params[57 + t],
        }
    }

    /// The period in ticks: max(3, N(Speed)). A higher Speed is slower.
    pub fn period(&self, tick: &Tick) -> u32 {
        time_ticks(self.speed, self.tempo, tick).max(MIN_PERIOD_TICKS)
    }
}

/// The LFO shapes at `phase` (cycles, 0..1): Sine sin 2πp; Triangle 0 → +1 at ¼ → 0 at ½ → −1 at ¾; Square +1 in
/// the first half, −1 in the second.
pub fn shape_value(shape: u8, phase: f32) -> f32 {
    match shape {
        0 => (TAU * phase).sin(),
        1 if phase < 0.25 => 4.0 * phase,
        1 if phase < 0.75 => 2.0 - 4.0 * phase,
        1 => 4.0 * phase - 4.0,
        _ if phase < 0.5 => 1.0,
        _ => -1.0,
    }
}

/// One voice's LFO: the phase restarts at 0 per note; Delay, then the fade-in g.
#[derive(Clone, Copy, Debug, Default)]
pub struct Lfo {
    age: u32,
    g: f32,
    phase: f32,
}

impl Lfo {
    pub fn trigger(&mut self) {
        *self = Self::default();
    }

    /// Advances one tick: 0 during the Delay (N_D ticks), then shape·g with g += 1/(N_A + 1) per tick from 0 and
    /// phase += 1/max(3, N_S). With Global on the voice reads `global_phase` instead of its own.
    pub fn tick(&mut self, p: &LfoParams, tick: &Tick, global_phase: f32) -> f32 {
        if self.age < time_ticks(p.delay, p.tempo, tick) {
            self.age += 1;
            return 0.0;
        }
        self.age = u32::MAX; // the Delay is over for this note
        let phase = if p.global { global_phase } else { self.phase };
        let value = shape_value(p.shape, phase) * self.g;
        self.g = (self.g + 1.0 / (time_ticks(p.attack, p.tempo, tick) as f32 + 1.0)).min(1.0);
        self.phase = (self.phase + 1.0 / p.period(tick) as f32).fract();
        value
    }
}

/// The shared phases of the five LFOs for Global mode: advanced once per tick, realigned to the song position
/// (beat · 384/D ticks into the period) when the transport starts.
#[derive(Clone, Copy, Debug, Default)]
pub struct GlobalLfo {
    /// This tick's phase per target.
    pub phase: [f32; 5],
    next: [f32; 5],
    playing: bool,
}

impl GlobalLfo {
    /// Once per tick, before the voices.
    pub fn tick(&mut self, params: &Params, tick: &Tick) {
        let start = tick.playing && !self.playing;
        self.playing = tick.playing;
        for (i, target) in TARGETS.into_iter().enumerate() {
            let period = LfoParams::read(params, target).period(tick);
            self.phase[i] = if start {
                (tick.beat * 384.0 / tick.d as f64 / period as f64).rem_euclid(1.0) as f32
            } else {
                self.next[i]
            };
            self.next[i] = (self.phase[i] + 1.0 / period as f32).fract();
        }
    }
}

/// One voice's modulation: the Vol, Cut, Res and Pitch envelopes and the five LFOs.
#[derive(Clone, Copy, Debug, Default)]
pub struct Modulation {
    /// Envelopes of Vol, Cut, Res, Pitch (`Target as usize − 1`).
    env: [Env; 4],
    lfo: [Lfo; 5],
}

impl VoiceMod for Modulation {
    type Global = GlobalLfo;

    fn tick_global(global: &mut GlobalLfo, params: &Params, tick: &Tick) {
        global.tick(params, tick);
    }

    fn start(&mut self, _params: &Params, _global: &GlobalLfo) {
        self.env.iter_mut().for_each(Env::trigger);
        self.lfo.iter_mut().for_each(Lfo::trigger);
    }

    fn release(&mut self, params: &Params) {
        for (env, target) in self.env.iter_mut().zip(ENV_TARGETS) {
            env.release(&EnvParams::read(params, target));
        }
    }

    fn cut(&mut self) {
        self.env.iter_mut().for_each(Env::cut);
    }

    fn tick(&mut self, params: &Params, global: &GlobalLfo, tick: &Tick) -> ModFrame {
        let mut frame = ModFrame::default();
        for (env, target) in self.env.iter_mut().zip(ENV_TARGETS) {
            frame.env[target as usize] = env.tick(&EnvParams::read(params, target), tick);
        }
        for (i, (lfo, target)) in self.lfo.iter_mut().zip(TARGETS).enumerate() {
            frame.lfo[i] = lfo.tick(&LfoParams::read(params, target), tick, global.phase[i]);
        }
        frame.done = self.env[0].is_done();
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::super::env::tests::T120;
    use super::*;

    fn lfo(shape: u8, delay: f32, attack: f32, speed: f32) -> LfoParams {
        LfoParams {
            global: false,
            tempo: false,
            shape,
            delay,
            attack,
            speed,
        }
    }

    fn run(l: &mut Lfo, p: &LfoParams, ticks: usize) -> Vec<f32> {
        (0..ticks).map(|_| l.tick(p, &T120, 0.0)).collect()
    }

    #[test]
    fn the_shapes() {
        for (p, sine, tri, square) in [
            (0.0, 0.0, 0.0, 1.0),
            (0.125, 0.70710677, 0.5, 1.0),
            (0.25, 1.0, 1.0, 1.0),
            (0.5, 0.0, 0.0, -1.0),
            (0.75, -1.0, -1.0, -1.0),
            (0.875, -0.70710677, -0.5, -1.0),
        ] {
            assert!((shape_value(0, p) - sine).abs() < 1e-6, "{p}");
            assert!((shape_value(1, p) - tri).abs() < 1e-6, "{p}");
            assert_eq!(shape_value(2, p), square, "{p}");
        }
    }

    #[test]
    fn delay_then_attack_ramp() {
        let p = lfo(2, 0.3, 0.2, 1.0);
        let nd = time_ticks(0.3, false, &T120) as usize;
        let na = time_ticks(0.2, false, &T120) as usize;
        assert!(nd > 5 && na > 5);
        let mut l = Lfo::default();
        l.trigger();
        let values = run(&mut l, &p, nd + na + 10);
        // 0 through the Delay, though the Square reads +1 at phase 0.
        assert!(values[..nd].iter().all(|v| *v == 0.0));
        // Then shape·g with g from 0 by 1/(N_A + 1) per tick (the Square is +1 for this long period).
        for j in 0..=na + 5 {
            let g = (j as f32 / (na as f32 + 1.0)).min(1.0);
            assert!(
                (values[nd + j] - g).abs() < 1e-5,
                "tick {j}: {} vs {g}",
                values[nd + j]
            );
        }
        // Attack 0: g is 0 on the first tick, then 1.
        let mut l = Lfo::default();
        l.trigger();
        assert_eq!(run(&mut l, &lfo(2, 0.0, 0.0, 1.0), 3), vec![0.0, 1.0, 1.0]);
    }

    #[test]
    fn the_phase_restarts_per_note_and_steps_by_the_period() {
        let p = lfo(0, 0.0, 0.0, 0.4);
        let period = p.period(&T120);
        assert_eq!(period, time_ticks(0.4, false, &T120));
        let mut l = Lfo::default();
        l.trigger();
        let values = run(&mut l, &p, 3 * period as usize);
        for (j, v) in values.iter().enumerate().skip(1) {
            let want = (std::f32::consts::TAU * (j as f32 / period as f32).fract()).sin();
            assert!((v - want).abs() < 2e-3, "tick {j}: {v} vs {want}");
        }
        l.trigger();
        assert_eq!(l.tick(&p, &T120, 0.0), 0.0);
        // Speed 0 still takes 3 ticks per cycle.
        let fast = lfo(2, 0.0, 0.0, 0.0);
        assert_eq!(fast.period(&T120), MIN_PERIOD_TICKS);
        let mut l = Lfo::default();
        l.trigger();
        assert_eq!(
            run(&mut l, &fast, 7),
            vec![0.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0]
        );
    }

    #[test]
    fn global_shares_one_phase_realigned_at_playback_start() {
        let mut params = [0.0; 149];
        params[28] = 1.0; // Volume LFO Global
        params[48] = 0.0; // Volume LFO Attack 0
        params[58] = 0.4; // Volume LFO Speed
        let p = LfoParams::read(&params, Target::Vol);
        assert!(p.global);
        let period = p.period(&T120) as f32;
        let mut global = GlobalLfo::default();
        let (mut a, mut b) = (Lfo::default(), Lfo::default());
        a.trigger();
        let mut tick = T120;
        for count in 0..40 {
            tick.count = count;
            global.tick(&params, &tick);
            if count == 25 {
                b.trigger();
            }
            let va = a.tick(&p, &tick, global.phase[1]);
            assert!((global.phase[1] - (count as f32 / period).fract()).abs() < 1e-4);
            if count > 26 {
                assert_eq!(va, b.tick(&p, &tick, global.phase[1]));
            } else if count >= 25 {
                b.tick(&p, &tick, global.phase[1]);
            }
        }
        // Transport start at beat 3.5: 3.5·96 ticks into the period.
        tick.playing = true;
        tick.beat = 3.5;
        global.tick(&params, &tick);
        assert!((global.phase[1] - (336.0 / period).fract()).abs() < 1e-4);
        let before = global.phase[1];
        global.tick(&params, &tick);
        assert!((global.phase[1] - (before + 1.0 / period).fract()).abs() < 1e-5);
    }

    #[test]
    fn params_follow_the_param_table() {
        let mut params = [0.0; 149];
        params[27] = 1.0; // Pan Global
        params[36] = 1.0; // Pitch Tempo
        params[39] = 1.0; // ModX Shape → Square
        params[40] = 0.5; // ModY Shape → Triangle
        params[45] = 0.25; // ModY Predelay
        params[51] = 0.7; // Pitch Attack
        params[57] = 0.9; // Pan Speed
        let pan = LfoParams::read(&params, Target::Pan);
        assert!(pan.global && !pan.tempo && pan.speed == 0.9);
        assert!(LfoParams::read(&params, Target::Pitch).tempo);
        assert_eq!(LfoParams::read(&params, Target::Pitch).attack, 0.7);
        assert_eq!(LfoParams::read(&params, Target::Cut).shape, 2);
        assert_eq!(LfoParams::read(&params, Target::Res).shape, 1);
        assert_eq!(LfoParams::read(&params, Target::Res).delay, 0.25);
    }

    #[test]
    fn the_voice_modulation_frame() {
        let mut params = [0.5; 149];
        params[62..=65].fill(0.0); // envelopes off
        params[63] = 1.0; // ModX envelope on
        for i in [70, 71, 72, 73, 82, 83, 84, 85] {
            params[i] = 0.0; // no Delay, no Hold
        }
        for i in [74, 75, 76, 77, 86, 87, 88, 89] {
            params[i] = 0.01; // A/D 1 %
        }
        params[99] = 0.3; // ModX Release
        let global = GlobalLfo::default();
        let mut m = Modulation::default();
        m.start(&params, &global);
        let frame = m.tick(&params, &global, &T120);
        assert_eq!(frame.env[Target::Pan as usize], 0.0);
        assert_eq!(frame.env[Target::Vol as usize], 1.0); // Enable off: the gate
        assert!(frame.env[Target::Cut as usize] > 0.0);
        assert_eq!(frame.env[Target::Res as usize], 0.0);
        assert!(!frame.done);
        let frames: Vec<ModFrame> = (0..20).map(|_| m.tick(&params, &global, &T120)).collect();
        assert!(frames.iter().all(|f| !f.done));
        m.release(&params);
        let frame = m.tick(&params, &global, &T120);
        assert_eq!(frame.env[Target::Vol as usize], 0.0);
        assert!(frame.done); // the Vol envelope (off) ends at release
        assert!(frame.env[Target::Cut as usize] > 0.0); // ModX still releasing
        m.start(&params, &global);
        m.cut();
        let frame = m.tick(&params, &global, &T120);
        assert!(frame.done && frame.env == [0.0; 5]);
    }

    #[test]
    fn shape_boundaries() {
        // Triangle: rising to ¼, falling through ½ to −1 at ¾, rising after.
        assert_eq!(shape_value(1, 0.25), 1.0);
        assert_eq!(shape_value(1, 0.75), -1.0);
        assert!(shape_value(1, 0.74) > -1.0 && shape_value(1, 0.74) < shape_value(1, 0.73));
        assert!(shape_value(1, 0.76) > -1.0 && shape_value(1, 0.76) < shape_value(1, 0.77));
        // Square: the second half starts at ½.
        assert_eq!(shape_value(2, 0.4999), 1.0);
        assert_eq!(shape_value(2, 0.5), -1.0);
        assert_eq!(shape_value(2, 0.9999), -1.0);
    }

    #[test]
    fn lfo_params_read_their_indices() {
        let params: Params = std::array::from_fn(|i| i as f32 / 149.0);
        for (t, target) in TARGETS.into_iter().enumerate() {
            let p = LfoParams::read(&params, target);
            let v = |base: usize| (base + t) as f32 / 149.0;
            assert_eq!(
                [p.delay, p.attack, p.speed],
                [v(42), v(47), v(57)],
                "{target:?}"
            );
            assert_eq!(p.shape, (v(37) * 2.0).round() as u8);
        }
    }

    #[test]
    fn start_restarts_the_lfos() {
        let mut params = [0.0; 149];
        params[37..=41].fill(1.0); // Square
        params[57..=61].fill(0.1); // a 12-tick period
        let global = GlobalLfo::default();
        let mut m = Modulation::default();
        m.start(&params, &global);
        let first: Vec<ModFrame> = (0..40).map(|_| m.tick(&params, &global, &T120)).collect();
        m.start(&params, &global);
        let again: Vec<ModFrame> = (0..40).map(|_| m.tick(&params, &global, &T120)).collect();
        assert_eq!(first[0].lfo, [0.0; 5]);
        assert!(first[1..].iter().any(|f| f.lfo[0] == -1.0));
        assert_eq!(
            first.iter().map(|f| f.lfo).collect::<Vec<_>>(),
            again.iter().map(|f| f.lfo).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_instrument_passes_the_song_beat_and_tempo() {
        // Pan LFO Global, Tempo off, Speed 0.4: the realign phase at transport start reads Tick.beat (with the
        // frames into the block) and Tick.bpm (the period at 140 BPM).
        use crate::ProcessContext;
        use crate::instrument::Instrument;
        let mut s = super::super::ThreeOsc::new(48_000.0, 4);
        s.set_parameter(27, 1.0);
        s.set_parameter(57, 0.4);
        let context = |start_beat, playing| ProcessContext {
            sample_rate: 48_000.0,
            tempo_bpm: 140.0,
            start_beat,
            playing,
        };
        // A tick is round(48000·15·4/(96·140)) = 214 frames: 100 stopped leave 114 to the next tick.
        let (mut l, mut r) = (vec![0.0; 300], vec![0.0; 300]);
        s.process(&mut l[..100], &mut r[..100], context(0.0, false));
        s.process(&mut l, &mut r, context(3.5, true));
        let beat = 3.5 + 114.0 * 140.0 / (60.0 * 48_000.0);
        let tick = Tick { bpm: 140.0, ..T120 };
        let period = time_ticks(0.4, false, &tick).max(MIN_PERIOD_TICKS) as f64;
        assert_ne!(period, time_ticks(0.4, false, &T120) as f64);
        let want = (beat * 96.0 / period).rem_euclid(1.0) as f32;
        assert!(
            (s.pool.global.phase[0] - want).abs() < 1e-6,
            "{} vs {want}",
            s.pool.global.phase[0]
        );
    }
}

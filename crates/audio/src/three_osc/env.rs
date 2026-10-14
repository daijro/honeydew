//! 3x Osc time law, tension curve and envelopes (the tension law is the one the panel's displays draw). Everything
//! advances once per control tick and is allocation-free.

use super::{Params, Target, Tick, toggle};

/// Time law K with Tempo on.
pub const K_TEMPO_ON: f32 = 49.152;
/// Time law K per BPM with Tempo off (K = 0.4096·BPM).
pub const K_PER_BPM: f32 = 0.4096;
/// Time law exponent: N(x) = round(K·(e^(6.9088x) − 1)/D).
pub const TIME_EXP: f32 = 6.9088;
/// Tension law exponent: k = sgn τ·(e^(8.7557|τ|) − 1)/10.
pub const TENSION_EXP: f32 = 8.7557;
/// A releasing envelope is done below 2^−24.
pub const DONE_LEVEL: f32 = 1.0 / 16_777_216.0;

/// The time law N(x) in ticks for a normalized time param `x`: K = 49.152 with `tempo` on, else 0.4096·BPM. It
/// times env Delay/Attack/Hold/Decay/Release and LFO Delay/Attack/Speed.
pub fn time_ticks(x: f32, tempo: bool, tick: &Tick) -> u32 {
    let k = if tempo {
        K_TEMPO_ON
    } else {
        K_PER_BPM * tick.bpm
    };
    (k * (TIME_EXP * x).exp_m1() / tick.d).round().max(0.0) as u32
}

/// The tension curve G of a `Tension` param (0..1, 0.5 = straight): τ = 2·Tension − 1,
/// k = sgn τ·(e^(8.7557|τ|) − 1)/10, L = ln(1 + |k|); G(u) = 1 + (1 − e^((1−u)L))/k for k > 0,
/// (e^(uL) − 1)/|k| for k < 0, and u for k = 0. G(0) = 0, G(1) = 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tension {
    k: f32,
    l: f32,
}

impl Tension {
    pub fn new(tension: f32) -> Self {
        let tau = 2.0 * tension - 1.0;
        let k = if tau == 0.0 {
            0.0
        } else {
            tau.signum() * (TENSION_EXP * tau.abs()).exp_m1() / 10.0
        };
        Self {
            k,
            l: k.abs().ln_1p(),
        }
    }

    /// G(u), u in 0..=1; exactly 0 and 1 at the ends.
    pub fn g(self, u: f32) -> f32 {
        if u <= 0.0 || u >= 1.0 {
            u.clamp(0.0, 1.0)
        } else if self.k > 0.0 {
            1.0 - ((1.0 - u) * self.l).exp_m1() / self.k
        } else if self.k < 0.0 {
            (u * self.l).exp_m1() / -self.k
        } else {
            u
        }
    }

    /// G⁻¹(y), y in 0..=1; exactly 0 and 1 at the ends.
    pub fn inverse(self, y: f32) -> f32 {
        if y <= 0.0 || y >= 1.0 {
            y.clamp(0.0, 1.0)
        } else if self.k > 0.0 {
            1.0 - (self.k * (1.0 - y)).ln_1p() / self.l
        } else if self.k < 0.0 {
            (-self.k * y).ln_1p() / self.l
        } else {
            y
        }
    }
}

/// One envelope's params (Vol, ModX = Cut, ModY = Res, Pitch), normalized. The Amount is the voice's (its depth).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvParams {
    pub enable: bool,
    pub tempo: bool,
    pub delay: f32,
    pub attack: f32,
    pub attack_tension: f32,
    pub hold: f32,
    pub decay: f32,
    pub decay_tension: f32,
    pub sustain: f32,
    pub release: f32,
    pub release_tension: f32,
    /// The Volume envelope: with Enable off it reads 1 while held and 0 from release; the others read 0.
    pub volume: bool,
}

impl EnvParams {
    /// `target`'s envelope from the param array (Enable 62 + i … Release Tension 102 + i; i = 0 Vol … 3 Pitch).
    /// Pan has none: it reads as a disabled modulation envelope.
    pub fn read(params: &Params, target: Target) -> Self {
        let Some(i) = (target as usize).checked_sub(1) else {
            return Self {
                enable: false,
                ..Self::read(params, Target::Cut)
            };
        };
        Self {
            enable: toggle(params[62 + i]),
            tempo: toggle(params[66 + i]),
            delay: params[70 + i],
            attack: params[74 + i],
            attack_tension: params[78 + i],
            hold: params[82 + i],
            decay: params[86 + i],
            decay_tension: params[90 + i],
            sustain: params[94 + i],
            release: params[98 + i],
            release_tension: params[102 + i],
            volume: target == Target::Vol,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Stage {
    #[default]
    Idle,
    Delay,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
}

/// One voice's envelope. [`Env::tick`] advances one control tick and returns that tick's value, 0..=1.
#[derive(Clone, Copy, Debug, Default)]
pub struct Env {
    stage: Stage,
    count: u32,
    u: f32,
    level: f32,
    /// Released before its first tick: the release starts after that tick (Attack 0 starts at 1, so a note shorter
    /// than a tick gap still plays its release).
    pending_release: bool,
}

impl Env {
    /// Starts the note: Delay, then Attack from u = 0.
    pub fn trigger(&mut self) {
        *self = Self {
            stage: Stage::Delay,
            ..Self::default()
        };
    }

    /// Release from the current level: u = G_R⁻¹(level). With Enable off it ends at once.
    pub fn release(&mut self, p: &EnvParams) {
        if !p.enable {
            self.cut();
        } else if self.stage == Stage::Delay && self.count == 0 {
            self.pending_release = true;
        } else if self.is_held() {
            self.stage = Stage::Release;
            self.u = Tension::new(p.release_tension).inverse(self.level);
        }
    }

    /// Zeroed at once (a stolen voice).
    pub fn cut(&mut self) {
        *self = Self::default();
    }

    /// Delay, Attack, Hold, Decay or Sustain: the key is still held.
    pub fn is_held(&self) -> bool {
        !matches!(self.stage, Stage::Idle | Stage::Release)
    }

    /// Finished, cut, or never started.
    pub fn is_done(&self) -> bool {
        self.stage == Stage::Idle
    }

    /// Advances one tick and returns the value.
    pub fn tick(&mut self, p: &EnvParams, tick: &Tick) -> f32 {
        if !p.enable {
            // Enable off: the Vol envelope is the gate; the others stay 0. Nothing advances, and a release ends.
            if self.stage == Stage::Release || self.pending_release {
                self.cut();
            }
            return if p.volume && self.is_held() { 1.0 } else { 0.0 };
        }
        let n = |x| time_ticks(x, p.tempo, tick);
        self.level = loop {
            match self.stage {
                Stage::Idle => break 0.0,
                Stage::Delay if self.count < n(p.delay) => {
                    self.count += 1;
                    break 0.0;
                }
                Stage::Delay => {
                    self.stage = Stage::Attack;
                    self.u = 0.0;
                }
                Stage::Attack => {
                    self.u += 1.0 / (n(p.attack) as f32 + 1.0);
                    if self.u < 1.0 {
                        break Tension::new(p.attack_tension).g(self.u);
                    }
                    self.stage = Stage::Hold;
                    self.count = 0;
                    break 1.0;
                }
                Stage::Hold if self.count < n(p.hold) => {
                    self.count += 1;
                    break 1.0;
                }
                Stage::Hold => {
                    self.stage = Stage::Decay;
                    self.u = 1.0;
                }
                Stage::Decay => {
                    self.u -= 1.0 / (n(p.decay) as f32 + 1.0);
                    let value = Tension::new(p.decay_tension).g(self.u);
                    if value > p.sustain {
                        break value;
                    }
                    self.stage = Stage::Sustain;
                }
                Stage::Sustain => break p.sustain,
                Stage::Release => {
                    self.u -= 1.0 / (n(p.release) as f32 + 1.0);
                    let value = Tension::new(p.release_tension).g(self.u);
                    if value >= DONE_LEVEL {
                        break value;
                    }
                    self.stage = Stage::Idle;
                }
            }
        };
        if std::mem::take(&mut self.pending_release) {
            self.stage = Stage::Release;
            self.u = Tension::new(p.release_tension).inverse(self.level);
        }
        self.level
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn tick_at(bpm: f32, d: f32) -> Tick {
        Tick {
            bpm,
            d,
            ..Tick::default()
        }
    }

    pub(crate) const T120: Tick = Tick {
        bpm: 120.0,
        d: 4.0,
        count: 0,
        beat: 0.0,
        playing: false,
    };

    /// ModX/ModY/Pitch defaults (A/D/R 1 %, S 100 %, tensions 0 %), enabled.
    const MOD: EnvParams = EnvParams {
        enable: true,
        tempo: false,
        delay: 0.0,
        attack: 0.01,
        attack_tension: 0.5,
        hold: 0.0,
        decay: 0.01,
        decay_tension: 0.5,
        sustain: 1.0,
        release: 0.01,
        release_tension: 0.5,
        volume: false,
    };

    fn tick_seconds(t: &Tick) -> f64 {
        t.d as f64 / 384.0 * 60.0 / t.bpm as f64
    }

    #[test]
    fn the_time_law_hits_the_spec_sample_points() {
        // Tempo off: 0.064·(e^(6.9088x) − 1) s; x = 0.01 → 4.6 ms, 0.305 → 0.46 s, 0.5 → 1.96 s, 1 → 64 s.
        for (x, seconds) in [(0.01, 0.0046), (0.305, 0.46), (0.5, 1.96), (1.0, 64.0)] {
            let law = 0.064 * ((TIME_EXP as f64 * x as f64).exp() - 1.0);
            assert!(
                (law - seconds).abs() / seconds < 0.03,
                "x {x}: {law} vs {seconds}"
            );
            for t in [T120, tick_at(97.0, 4.0), tick_at(140.0, 8.0)] {
                let got = time_ticks(x, false, &t) as f64 * tick_seconds(&t);
                assert!(
                    (got - law).abs() <= tick_seconds(&t) / 2.0 + 1e-6,
                    "x {x} {t:?}: {got} vs {law}"
                );
            }
        }
        // Tempo on: 0.128·(e^(6.9088x) − 1) beats at any BPM.
        for x in [0.01_f32, 0.305, 0.5, 1.0] {
            let law = 0.128 * ((TIME_EXP as f64 * x as f64).exp() - 1.0);
            for bpm in [60.0, 120.0, 174.0] {
                let t = tick_at(bpm, 4.0);
                let beats = time_ticks(x, true, &t) as f64 * t.d as f64 / 384.0;
                assert!(
                    (beats - law).abs() <= t.d as f64 / 768.0 + 1e-6,
                    "x {x}: {beats} vs {law}"
                );
            }
        }
        assert_eq!(time_ticks(0.0, false, &T120), 0);
        assert_eq!(time_ticks(1.0, false, &T120), 12289); // 64.0 s of 5.208 ms ticks
    }

    #[test]
    fn the_tension_curve_is_the_display_law() {
        for tension in [0.0, 0.105, 0.3, 0.5, 0.5001, 0.75, 1.0] {
            let g = Tension::new(tension);
            assert!(g.g(0.0).abs() < 1e-5, "{tension}");
            assert!((g.g(1.0) - 1.0).abs() < 1e-5, "{tension}");
            for u in [0.1, 0.25, 0.5, 0.9] {
                assert!((g.inverse(g.g(u)) - u).abs() < 1e-3, "{tension} {u}");
            }
        }
        assert_eq!(Tension::new(0.5).g(0.3), 0.3);
        // τ = ±1: k = ±(e^8.7557 − 1)/10, L = ln(1 + |k|).
        let k = (8.7557_f64.exp() - 1.0) / 10.0;
        let l = (1.0 + k).ln();
        let up = 1.0 + (1.0 - (0.5 * l).exp()) / k;
        let down = ((0.5 * l).exp() - 1.0) / k;
        assert!((Tension::new(1.0).g(0.5) as f64 - up).abs() < 1e-5);
        assert!((Tension::new(0.0).g(0.5) as f64 - down).abs() < 1e-5);
        // τ > 0 bows toward the top, τ < 0 toward the bottom.
        assert!(Tension::new(0.8).g(0.5) > 0.6);
        assert!(Tension::new(0.2).g(0.5) < 0.4);
    }

    fn run(env: &mut Env, p: &EnvParams, ticks: usize) -> Vec<f32> {
        (0..ticks).map(|_| env.tick(p, &T120)).collect()
    }

    #[test]
    fn envelope_segments_last_their_time_law_ticks() {
        let p = EnvParams {
            delay: 0.2,
            attack: 0.3,
            hold: 0.15,
            decay: 0.4,
            sustain: 0.5,
            decay_tension: 0.75,
            ..MOD
        };
        let n = |x| time_ticks(x, false, &T120) as usize;
        let (nd, na, nh, ndc) = (n(0.2), n(0.3), n(0.15), n(0.4));
        assert!(nd > 2 && na > 2 && nh > 2 && ndc > 2);
        let mut env = Env::default();
        env.trigger();
        let values = run(&mut env, &p, 2000);
        // Delay holds 0 for N_D ticks.
        assert!(values[..nd].iter().all(|v| *v == 0.0));
        // Attack: u += 1/(N_A + 1), value G_A(u); 1 on its (N_A + 1)-th tick.
        let attack = &values[nd..];
        assert!((attack[0] - 1.0 / (na as f32 + 1.0)).abs() < 1e-5);
        assert!(attack[na - 1] < 1.0);
        assert_eq!(attack[na], 1.0);
        // Hold N_H ticks at 1.
        let hold = &attack[na + 1..];
        assert!(hold[..nh].iter().all(|v| *v == 1.0));
        assert!(hold[nh] < 1.0);
        // Decay covers u = 1 → G_D⁻¹(S) only: (1 − G_D⁻¹(S))·(N_Dc + 1) ticks, ending on S.
        let decay = &hold[nh..];
        let span = (1.0 - Tension::new(0.75).inverse(0.5) as f64) * (ndc as f64 + 1.0);
        let reached = decay.iter().position(|v| *v == 0.5).unwrap() + 1;
        assert!(
            (reached as f64 - span).abs() <= 1.0,
            "decay {reached} vs {span}"
        );
        // A positive tension bows the curve up, so it takes longer than a straight line to reach S.
        assert!(span > (ndc as f64 + 1.0) / 2.0 + 10.0, "{span} of {ndc}");
        assert!(decay[..reached - 1].iter().all(|v| *v > 0.5));
        assert!(decay[reached..].iter().all(|v| *v == 0.5));
        assert!(env.is_held() && !env.is_done());
    }

    #[test]
    fn attack_zero_starts_at_one() {
        let p = EnvParams { attack: 0.0, ..MOD };
        let mut env = Env::default();
        env.trigger();
        assert_eq!(env.tick(&p, &T120), 1.0);
    }

    #[test]
    fn release_lasts_release_times_the_inverse_of_the_level() {
        for (tension, sustain) in [
            (0.5, 1.0),
            (0.5, 0.4),
            (0.105, 1.0),
            (0.105, 0.3),
            (0.9, 0.6),
        ] {
            let p = EnvParams {
                attack: 0.0,
                decay: 0.0,
                sustain,
                release: 0.45,
                release_tension: tension,
                volume: true,
                ..MOD
            };
            let nr = time_ticks(0.45, false, &T120) as f64;
            let mut env = Env::default();
            env.trigger();
            assert_eq!(*run(&mut env, &p, 10).last().unwrap(), sustain);
            env.release(&p);
            let values = run(&mut env, &p, 5000);
            let done = values.iter().position(|v| *v == 0.0).unwrap();
            let span = Tension::new(tension).inverse(sustain) as f64 * (nr + 1.0);
            assert!(
                (done as f64 - span).abs() <= 1.5,
                "{tension} {sustain}: {done} vs {span}"
            );
            assert!(values[..done].iter().all(|v| *v >= DONE_LEVEL));
            assert!(values[0] < sustain);
            assert!(env.is_done() && !env.is_held());
        }
    }

    #[test]
    fn release_starts_from_the_current_level() {
        let p = EnvParams {
            attack: 0.5,
            release: 0.2,
            ..MOD
        };
        let mut env = Env::default();
        env.trigger();
        let level = *run(&mut env, &p, 20).last().unwrap();
        assert!(level > 0.0 && level < 1.0);
        env.release(&p);
        let step = 1.0 / (time_ticks(0.2, false, &T120) as f32 + 1.0);
        assert!((env.tick(&p, &T120) - (level - step)).abs() < 1e-5);
        // Released in the Delay (level 0): done on the first release tick.
        let p = EnvParams { delay: 0.5, ..p };
        env.trigger();
        run(&mut env, &p, 3);
        env.release(&p);
        assert_eq!(env.tick(&p, &T120), 0.0);
        assert!(env.is_done());
    }

    #[test]
    fn enable_off_is_the_gate_for_volume_and_zero_for_the_rest() {
        let vol_p = EnvParams {
            enable: false,
            volume: true,
            ..MOD
        };
        let cut_p = EnvParams {
            enable: false,
            ..MOD
        };
        let (mut vol, mut cut) = (Env::default(), Env::default());
        vol.trigger();
        cut.trigger();
        assert_eq!(run(&mut vol, &vol_p, 50), vec![1.0; 50]);
        assert_eq!(run(&mut cut, &cut_p, 50), vec![0.0; 50]);
        vol.release(&vol_p);
        cut.release(&cut_p);
        assert_eq!(vol.tick(&vol_p, &T120), 0.0);
        assert_eq!(cut.tick(&cut_p, &T120), 0.0);
        assert!(vol.is_done());
    }

    #[test]
    fn cut_zeroes_at_once() {
        let mut env = Env::default();
        env.trigger();
        run(&mut env, &MOD, 5);
        env.cut();
        assert!(env.is_done());
        assert_eq!(env.tick(&MOD, &T120), 0.0);
    }

    #[test]
    fn params_follow_the_param_table() {
        let mut params = [0.0; 149];
        params[62] = 1.0; // Volume Enable
        params[67] = 1.0; // ModX Tempo
        params[77] = 0.3; // Pitch Attack
        params[96] = 0.2; // ModY Sustain
        params[102] = 0.105; // Volume Release Tension
        let vol = EnvParams::read(&params, Target::Vol);
        let cut = EnvParams::read(&params, Target::Cut);
        let res = EnvParams::read(&params, Target::Res);
        let pitch = EnvParams::read(&params, Target::Pitch);
        assert!(vol.enable && vol.volume && !vol.tempo && vol.release_tension == 0.105);
        assert!(cut.tempo && !cut.enable && !cut.volume);
        assert_eq!((pitch.attack, res.sustain), (0.3, 0.2));
        assert!(!EnvParams::read(&params, Target::Pan).enable);
    }

    #[test]
    fn every_param_index_reads_into_its_field() {
        let params: Params = std::array::from_fn(|i| i as f32 / 149.0);
        for (i, target) in [Target::Vol, Target::Cut, Target::Res, Target::Pitch]
            .into_iter()
            .enumerate()
        {
            let v = |base: usize| (base + i) as f32 / 149.0;
            let p = EnvParams::read(&params, target);
            assert_eq!(
                [
                    p.delay,
                    p.attack,
                    p.attack_tension,
                    p.hold,
                    p.decay,
                    p.decay_tension,
                    p.sustain,
                    p.release,
                    p.release_tension
                ],
                [
                    v(70),
                    v(74),
                    v(78),
                    v(82),
                    v(86),
                    v(90),
                    v(94),
                    v(98),
                    v(102)
                ],
                "{target:?}"
            );
            assert_eq!(p.volume, target == Target::Vol);
            // The toggles read their own index: only that one set to 1 turns it on.
            for (base, field) in [(62, 0), (66, 1)] {
                let mut one = params;
                one[base + i] = 1.0;
                let q = EnvParams::read(&one, target);
                assert!([q.enable, q.tempo][field], "{target:?} {base}");
                assert!(![q.enable, q.tempo][1 - field], "{target:?} {base}");
            }
        }
    }

    #[test]
    fn enable_off_during_release_ends_it() {
        let mut env = Env::default();
        env.trigger();
        run(&mut env, &MOD, 5);
        env.release(&MOD);
        env.tick(&MOD, &T120);
        assert!(!env.is_done());
        let off = EnvParams {
            enable: false,
            ..MOD
        };
        assert_eq!(env.tick(&off, &T120), 0.0);
        assert!(env.is_done());
    }

    #[test]
    fn a_release_before_the_first_tick_follows_it() {
        // Attack 0 starts at 1: a note released before its first tick plays its whole release from 1.
        let p = EnvParams {
            attack: 0.0,
            release: 0.3,
            ..MOD
        };
        let mut env = Env::default();
        env.trigger();
        env.release(&p);
        assert!(env.is_held());
        let values = run(&mut env, &p, 200);
        assert_eq!(values[0], 1.0);
        let nr = time_ticks(0.3, false, &T120) as usize;
        let done = values.iter().position(|v| *v == 0.0).unwrap();
        assert!(done.abs_diff(nr + 1) <= 1, "{done} vs {}", nr + 1);
        assert!(values[1..done].windows(2).all(|w| w[1] < w[0]));
        // With a Delay the note never left 0: done after the first tick.
        let p = EnvParams { delay: 0.3, ..p };
        env.trigger();
        env.release(&p);
        assert_eq!(run(&mut env, &p, 2), [0.0, 0.0]);
        assert!(env.is_done());
    }
}

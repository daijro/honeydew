//! 3x Osc Echo: the note-repeat scheduler and Fat Mode. Tick-driven and
//! allocation-free: pending echoes live in a fixed pool.

use super::{NoteSpec, Params, Tick, VoiceEvent, toggle};

/// Pending echoes and Fat Mode copies held at once.
// ponytail: fixed pool; a note whose echoes find it full loses the echoes that do not fit.
pub const ECHO_SLOTS: usize = 512;
/// Echo and Fat Mode copies take tags from here up; an original keeps tag = key (the pass-through rule).
pub const ECHO_TAGS: u32 = 128;

/// The Echo params (130 Echoes … 138 Fat Mode), normalized except the counts and toggles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EchoParams {
    /// 1..=10.
    pub echoes: u32,
    pub feedback: f32,
    pub time: f32,
    pub pan: f32,
    pub pitch: f32,
    pub cutoff: f32,
    pub resonance: f32,
    pub ping_pong: bool,
    pub fat: bool,
    /// Monophonic Mode (p120): Echo is off in Mono.
    pub mono: bool,
}

impl EchoParams {
    pub fn read(params: &Params) -> Self {
        Self {
            echoes: 1 + (params[130] * 9.0).round() as u32,
            feedback: params[131],
            time: params[132],
            pan: params[133],
            pitch: params[134],
            cutoff: params[135],
            resonance: params[136],
            ping_pong: toggle(params[137]),
            fat: toggle(params[138]),
            mono: toggle(params[120]),
        }
    }

    /// Echo runs: Feedback > 0 and not Mono.
    pub fn active(&self) -> bool {
        self.feedback > 0.0 && !self.mono
    }
}

/// One step of the echo filter offset law: x_k = 2c(x + 1) − 1 for c < 0.5, else 1 − (2 − 2c)(1 − x); x_0 = 0.
pub fn offset_step(c: f32, x: f32) -> f32 {
    if c < 0.5 {
        2.0 * c * (x + 1.0) - 1.0
    } else {
        1.0 - (2.0 - 2.0 * c) * (1.0 - x)
    }
}

/// Fat Mode gains for N = `echoes`: (the played note's velocity factor 1.5 − 4·Fb·N/(3N + 2), each of the 2N copies'
/// 2·Fb/(3N + 2)). Together they make 1.5.
pub fn fat_gains(feedback: f32, echoes: u32) -> (f32, f32) {
    let n = echoes as f32;
    let d = 3.0 * n + 2.0;
    (1.5 - 4.0 * feedback * n / d, 2.0 * feedback / d)
}

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    live: bool,
    sounding: bool,
    group: u32,
    tag: u32,
    on: u64,
    /// Ticks the played note was held; `u64::MAX` while it still is.
    hold: u64,
    note: NoteSpec,
}

#[derive(Clone, Copy, Debug, Default)]
struct Held {
    down: bool,
    group: u32,
    start: u64,
}

/// The echo scheduler: an original sounds at once (tag = key) and its echoes and copies are queued.
pub struct Echo {
    slots: [Slot; ECHO_SLOTS],
    held: [Held; 128],
    next_group: u32,
    next_tag: u32,
}

impl Default for Echo {
    fn default() -> Self {
        Self {
            slots: [Slot::default(); ECHO_SLOTS],
            held: [Held::default(); 128],
            next_group: 0,
            next_tag: ECHO_TAGS,
        }
    }
}

impl Echo {
    /// Key down at `tick`: emits the played note (Fat Mode: and its copies) and queues the echoes.
    pub fn key_on(
        &mut self,
        key: u8,
        velocity: f32,
        pan: f32,
        params: &Params,
        tick: &Tick,
        emit: &mut dyn FnMut(VoiceEvent),
    ) {
        self.finish(key, tick.count); // a re-press ends the old note's hold (its voice: the re-press rule)
        let group = self.next_group;
        self.next_group = self.next_group.wrapping_add(1);
        self.held[key as usize] = Held {
            down: true,
            group,
            start: tick.count,
        };
        let p = EchoParams::read(params);
        let original = NoteSpec {
            key,
            velocity,
            pan,
            ..NoteSpec::default()
        };
        let tag = key as u32;
        if !p.active() {
            emit(VoiceEvent::On {
                tag,
                note: original,
            });
            return;
        }
        let (pan_step, pitch_step) = (4.0 * p.pan - 2.0, 24.0 * p.pitch - 12.0);
        let (mut cut, mut res) = (0.0, 0.0);
        if p.fat {
            // Copy ±k takes ±k pan and pitch steps and ±x_k filter offsets, sounding with the played note.
            let (gain, copy) = fat_gains(p.feedback, p.echoes);
            let note = NoteSpec {
                velocity: velocity * gain,
                ..original
            };
            emit(VoiceEvent::On { tag, note });
            for k in 1..=p.echoes {
                cut = offset_step(p.cutoff, cut);
                res = offset_step(p.resonance, res);
                for sign in [1.0, -1.0] {
                    let k = sign * k as f32;
                    let note = NoteSpec {
                        velocity: velocity * copy,
                        pan: pan + k * pan_step,
                        pitch: k * pitch_step,
                        cut: sign * cut,
                        res: sign * res,
                        ..original
                    };
                    self.queue(group, tick.count, note);
                }
            }
            return;
        }
        emit(VoiceEvent::On {
            tag,
            note: original,
        });
        let (mut note, mut step) = (original, pan_step);
        for k in 1..=p.echoes {
            note.velocity *= 2.0 * p.feedback;
            note.pan += step;
            if p.ping_pong && (note.pan <= 0.0 || note.pan >= 1.0) {
                step = -step;
            }
            note.pitch = k as f32 * pitch_step;
            note.cut = offset_step(p.cutoff, note.cut);
            note.res = offset_step(p.resonance, note.res);
            let delay = (k as f64 * 2.0 * p.time as f64 * 384.0 / tick.d as f64).round() as u64;
            self.queue(group, tick.count + delay, note);
        }
    }

    /// Key up: the played note's Off; its echoes and copies now last as long as it was held.
    pub fn key_off(&mut self, key: u8, tick: &Tick, emit: &mut dyn FnMut(VoiceEvent)) {
        if self.held[key as usize].down {
            emit(VoiceEvent::Off { tag: key as u32 });
            self.finish(key, tick.count);
        }
    }

    /// Emits the echoes and copies due at `tick`.
    pub fn tick(&mut self, tick: &Tick, emit: &mut dyn FnMut(VoiceEvent)) {
        for slot in self.slots.iter_mut().filter(|s| s.live) {
            if !slot.sounding && slot.on <= tick.count {
                slot.sounding = true;
                emit(VoiceEvent::On {
                    tag: slot.tag,
                    note: slot.note,
                });
            }
            if slot.sounding && slot.hold != u64::MAX && slot.on + slot.hold <= tick.count {
                slot.live = false;
                emit(VoiceEvent::Off { tag: slot.tag });
            }
        }
    }

    fn queue(&mut self, group: u32, on: u64, note: NoteSpec) {
        let tag = self.next_tag;
        if let Some(slot) = self.slots.iter_mut().find(|s| !s.live) {
            self.next_tag = self.next_tag.wrapping_add(1);
            *slot = Slot {
                live: true,
                sounding: false,
                group,
                tag,
                on,
                hold: u64::MAX,
                note,
            };
        }
    }

    /// `key`'s note is up (or re-pressed) at `now`: its echoes last as long as it was held, at least a tick.
    fn finish(&mut self, key: u8, now: u64) {
        let held = &mut self.held[key as usize];
        if !held.down {
            return;
        }
        held.down = false;
        let (group, hold) = (held.group, now.saturating_sub(held.start).max(1));
        for slot in self.slots.iter_mut().filter(|s| s.live && s.group == group) {
            slot.hold = hold;
        }
    }

    /// Forgets everything, emitting nothing.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::super::env::tests::T120;
    use super::*;

    /// (tick, event) for `ticks` ticks: `key` 60 down at 0 (vel 1, pan 0.5), up after `hold` ticks.
    fn play(params: &Params, hold: u64, ticks: u64) -> Vec<(u64, VoiceEvent)> {
        let mut echo = Echo::default();
        let mut events = Vec::new();
        for count in 0..ticks {
            let tick = Tick { count, ..T120 };
            let mut emit = |e| events.push((count, e));
            if count == 0 {
                echo.key_on(60, 1.0, 0.5, params, &tick, &mut emit);
            }
            if count == hold {
                echo.key_off(60, &tick, &mut emit);
            }
            echo.tick(&tick, &mut emit);
        }
        events
    }

    fn base() -> Params {
        let mut p = [0.5; 149];
        p[120] = 0.0; // Mono off
        p[130] = 3.0 / 9.0; // 4 echoes
        p[131] = 0.4; // Feedback 80 %: velocity × 0.8^k
        p[132] = 0.375; // Time: 2·0.375 = 0.75 beat = 72 ticks
        p[137] = 0.0;
        p[138] = 0.0;
        p
    }

    fn ons(events: &[(u64, VoiceEvent)]) -> Vec<(u64, u32, NoteSpec)> {
        events
            .iter()
            .filter_map(|(t, e)| match e {
                VoiceEvent::On { tag, note } => Some((*t, *tag, *note)),
                _ => None,
            })
            .collect()
    }

    fn off_tick(events: &[(u64, VoiceEvent)], tag: u32) -> u64 {
        events
            .iter()
            .find(|(_, e)| *e == VoiceEvent::Off { tag })
            .unwrap()
            .0
    }

    #[test]
    fn echo_k_sounds_at_k_times_2_time_beats_with_velocity_times_2fb_to_the_k() {
        let events = play(&base(), 10, 400);
        let first = ons(&events);
        assert_eq!(first.len(), 5);
        assert_eq!(first[0].1, 60);
        assert_eq!(off_tick(&events, 60), 10);
        for (k, (t, tag, note)) in first.iter().enumerate() {
            assert_eq!(*t, 72 * k as u64, "echo {k}");
            assert!(
                (note.velocity - 0.8_f32.powi(k as i32)).abs() < 1e-6,
                "echo {k}"
            );
            assert_eq!(
                (note.key, note.pitch, note.cut, note.res),
                (60, 0.0, 0.0, 0.0)
            );
            assert_eq!(note.pan, 0.5);
            // Each echo lasts as long as the played note was held.
            assert_eq!(off_tick(&events, *tag), t + 10, "echo {k}");
        }
        // Held past the first echoes: they end when the played note is known to have been held 100 ticks.
        let events = play(&base(), 100, 600);
        for (t, tag, _) in ons(&events) {
            assert_eq!(off_tick(&events, tag), t + 100);
        }
    }

    #[test]
    fn echo_pan_pitch_and_filter_offsets_progress_per_echo() {
        let mut p = base();
        p[133] = 0.5625; // Pan step 4·Pan − 2 = +0.25
        p[134] = 0.75; // Pitch step 24·Pitch − 12 = +6 st
        p[135] = 0.25; // Cutoff: x → 2c(x + 1) − 1 = 0.5x − 0.5
        p[136] = 0.75; // Resonance: x → 1 − 0.5(1 − x)
        let ons = ons(&play(&p, 10, 400));
        let pans: Vec<f32> = ons.iter().map(|o| o.2.pan).collect();
        let pitches: Vec<f32> = ons.iter().map(|o| o.2.pitch).collect();
        let cuts: Vec<f32> = ons.iter().map(|o| o.2.cut).collect();
        let reses: Vec<f32> = ons.iter().map(|o| o.2.res).collect();
        assert_eq!(pans, [0.5, 0.75, 1.0, 1.25, 1.5]);
        assert_eq!(pitches, [0.0, 6.0, 12.0, 18.0, 24.0]);
        assert_eq!(cuts, [0.0, -0.5, -0.75, -0.875, -0.9375]);
        assert_eq!(reses, [0.0, 0.5, 0.75, 0.875, 0.9375]);
        // Ping Pong reverses the step at ≥ 1 (or ≤ 0).
        p[137] = 1.0;
        let pans: Vec<f32> = ons_of(&p).iter().map(|o| o.2.pan).collect();
        assert_eq!(pans, [0.5, 0.75, 1.0, 0.75, 0.5]);
        p[133] = 0.4375; // −0.25
        let pans: Vec<f32> = ons_of(&p).iter().map(|o| o.2.pan).collect();
        assert_eq!(pans, [0.5, 0.25, 0.0, 0.25, 0.5]);
    }

    fn ons_of(p: &Params) -> Vec<(u64, u32, NoteSpec)> {
        ons(&play(p, 10, 400))
    }

    #[test]
    fn echo_is_off_in_mono_and_at_zero_feedback() {
        let mut p = base();
        p[120] = 1.0;
        let events = play(&p, 10, 400);
        assert_eq!(
            events,
            [
                (
                    0,
                    VoiceEvent::On {
                        tag: 60,
                        note: NoteSpec {
                            key: 60,
                            velocity: 1.0,
                            pan: 0.5,
                            ..NoteSpec::default()
                        }
                    }
                ),
                (10, VoiceEvent::Off { tag: 60 })
            ]
        );
        let mut p = base();
        p[131] = 0.0;
        assert_eq!(play(&p, 10, 400), events);
    }

    #[test]
    fn fat_mode_gains_sum_to_one_and_a_half() {
        for n in 1..=10 {
            for fb in [0.05_f32, 0.4, 0.5, 1.0] {
                let (original, copy) = fat_gains(fb, n);
                assert!(
                    (original + 2.0 * n as f32 * copy - 1.5).abs() < 1e-6,
                    "{n} {fb}"
                );
                assert!(original > 0.0);
            }
        }
        let mut p = base();
        p[138] = 1.0;
        p[133] = 0.5625; // pan step +0.25
        p[134] = 0.75; // +6 st
        let events = play(&p, 10, 400);
        let ons = ons(&events);
        // The original and 2N = 8 copies at once; no delayed echoes.
        assert_eq!(ons.len(), 9);
        assert!(ons.iter().all(|o| o.0 == 0));
        let total: f32 = ons.iter().map(|o| o.2.velocity).sum();
        assert!((total - 1.5).abs() < 1e-5);
        let (original, copy) = fat_gains(0.4, 4);
        assert_eq!(ons[0].2.velocity, original);
        assert!(ons[1..].iter().all(|o| o.2.velocity == copy));
        let mut pitches: Vec<f32> = ons[1..].iter().map(|o| o.2.pitch).collect();
        pitches.sort_by(f32::total_cmp);
        assert_eq!(pitches, [-24.0, -18.0, -12.0, -6.0, 6.0, 12.0, 18.0, 24.0]);
        // All end with the played note.
        for (_, tag, _) in ons {
            assert_eq!(off_tick(&events, tag), 10);
        }
    }

    #[test]
    fn reset_forgets_pending_echoes() {
        let mut echo = Echo::default();
        let mut events = Vec::new();
        echo.key_on(60, 1.0, 0.5, &base(), &T120, &mut |e| events.push(e));
        echo.reset();
        for count in 0..400 {
            echo.tick(&Tick { count, ..T120 }, &mut |e| events.push(e));
        }
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn offset_step_law() {
        assert_eq!(offset_step(0.5, 0.3), 0.3);
        assert_eq!(offset_step(0.0, 0.3), -1.0);
        assert_eq!(offset_step(1.0, -0.3), 1.0);
    }

    #[test]
    fn fat_mode_explicit_gains_pans_and_offsets() {
        let (original, copy) = fat_gains(0.4, 4);
        assert!((original - (1.5 - 6.4 / 14.0)).abs() < 1e-6);
        assert!((copy - 0.8 / 14.0).abs() < 1e-7);
        let mut p = base();
        p[138] = 1.0;
        p[133] = 0.5625; // pan step +0.25
        p[134] = 0.75; // +6 st
        p[135] = 0.25; // x_k = −0.5, −0.75, −0.875, −0.9375
        let copies: Vec<(f32, f32, f32)> = ons(&play(&p, 10, 20))[1..]
            .iter()
            .map(|o| (o.2.pitch, o.2.pan, o.2.cut))
            .collect();
        for (pitch, pan, cut) in copies {
            let k = pitch / 6.0;
            assert_eq!(pan, 0.5 + 0.25 * k, "copy {k}");
            // +k copies take x_k (negative here), −k copies its mirror.
            let x = [-0.5, -0.75, -0.875, -0.9375][k.abs() as usize - 1];
            assert_eq!(cut, k.signum() * x, "copy {k}");
        }
    }

    #[test]
    fn a_re_press_ends_the_old_notes_hold_and_holds_last_at_least_a_tick() {
        let mut echo = Echo::default();
        let mut events = Vec::new();
        for count in 0..400 {
            let tick = Tick { count, ..T120 };
            let mut emit = |e| events.push((count, e));
            match count {
                0 | 5 => echo.key_on(60, 1.0, 0.5, &base(), &tick, &mut emit),
                20 => echo.key_off(60, &tick, &mut emit),
                _ => {}
            }
            echo.tick(&tick, &mut emit);
        }
        // The first note's echoes last 5 ticks (held until the re-press), the second's 15.
        for (t, tag, _) in ons(&events).into_iter().filter(|o| o.1 != 60) {
            let hold = if t % 72 == 0 { 5 } else { 15 };
            assert_eq!(off_tick(&events, tag), t + hold, "echo at {t}");
        }
        // Up on the tick it went down: the echoes last one tick.
        let events = play(&base(), 0, 400);
        for (t, tag, _) in ons(&events).into_iter().filter(|o| o.1 != 60) {
            assert_eq!(off_tick(&events, tag), t + 1);
        }
    }
}

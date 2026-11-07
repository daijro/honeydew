// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The arpeggiator: held notes in, timed note events out, with a sixteen-step pattern (velocity, transposition, tie).
//! Fixed-size storage: nothing allocates.

use crate::ids::*;
use crate::params::Snapshot;
use crate::util::Rng;

const MAX_HELD: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArpEv {
    On { note: u8, vel: f32, step_vel: f32 },
    Off { note: u8 },
}

#[derive(Clone, Copy, Debug, Default)]
struct Held {
    note: u8,
    vel: f32,
    released: bool,
}

pub struct Arp {
    held: [Held; MAX_HELD],
    n: usize,
    step: usize,
    /// Samples into the current step.
    clock: f32,
    started: bool,
    sounding: [u8; MAX_HELD],
    sounding_n: usize,
    off_at: f32,
    tie: bool,
    rng: Rng,
    pub step_vel: f32,
}

impl Default for Arp {
    fn default() -> Self {
        Arp {
            held: [Held::default(); MAX_HELD],
            n: 0,
            step: 0,
            clock: 0.0,
            started: false,
            sounding: [0; MAX_HELD],
            sounding_n: 0,
            off_at: 0.0,
            tie: false,
            rng: Rng::new(0x5eed),
            step_vel: 1.0,
        }
    }
}

impl Arp {
    pub fn clear(&mut self) {
        self.n = 0;
        self.step = 0;
        self.started = false;
        self.clock = 0.0;
    }

    pub fn note_on(&mut self, note: u8, vel: f32, hold: bool) {
        if hold && self.n > 0 && self.held[..self.n].iter().all(|h| h.released) {
            self.n = 0;
        }
        if let Some(h) = self.held[..self.n].iter_mut().find(|h| h.note == note) {
            h.vel = vel;
            h.released = false;
            return;
        }
        if self.n < MAX_HELD {
            self.held[self.n] = Held {
                note,
                vel,
                released: false,
            };
            self.n += 1;
        }
    }

    pub fn note_off(&mut self, note: u8, hold: bool) {
        if let Some(i) = self.held[..self.n].iter().position(|h| h.note == note) {
            if hold {
                self.held[i].released = true;
            } else {
                self.held.copy_within(i + 1..self.n, i);
                self.n -= 1;
            }
        }
    }

    /// The notes to release when the arpeggiator stops.
    pub fn stop(&mut self, out: &mut dyn FnMut(ArpEv)) {
        self.release_sounding(out);
        self.clear();
        self.n = 0;
    }

    fn release_sounding(&mut self, out: &mut dyn FnMut(ArpEv)) {
        for i in 0..self.sounding_n {
            out(ArpEv::Off {
                note: self.sounding[i],
            });
        }
        self.sounding_n = 0;
    }

    /// The `i`th note of the expanded sequence (`m` notes over `oct` octaves).
    fn expanded(&self, sorted: &[Held; MAX_HELD], m: usize, i: usize) -> (u8, f32) {
        let h = sorted[i % m];
        ((h.note as usize + 12 * (i / m)).min(127) as u8, h.vel)
    }

    /// Advances by `n` samples, emitting the events that fall in them.
    pub fn process(
        &mut self,
        n: usize,
        p: &Snapshot,
        tempo: f32,
        sr: f32,
        out: &mut dyn FnMut(ArpEv),
    ) {
        let hold = p[ARP_HOLD] >= 0.5;
        let beats = DIV_BEATS[(p[ARP_DIV] as usize).min(DIV_BEATS.len() - 1)];
        let len = (beats * 60.0 / tempo.max(20.0) * sr).max(8.0);
        if self.n == 0 {
            self.release_sounding(out);
            self.started = false;
            return;
        }
        if !self.started {
            self.started = true;
            self.clock = len;
            self.step = 0;
        }
        self.clock += n as f32;
        // The gate closes.
        if self.sounding_n > 0
            && !self.tie
            && self.clock - n as f32 <= self.off_at
            && self.clock >= self.off_at
        {
            self.release_sounding(out);
        }
        while self.clock >= len {
            self.clock -= len;
            self.release_sounding(out);
            self.fire(p, len, hold, out);
        }
    }

    fn fire(&mut self, p: &Snapshot, len: f32, _hold: bool, out: &mut dyn FnMut(ArpEv)) {
        let steps = (p[ARP_STEPS] as usize).clamp(1, 16);
        let si = self.step % steps;
        let step_vel = p[ARP_STEP_BASE + si];
        let transp = p[ARP_TR_BASE + si] as i32;
        self.tie = p[ARP_HOLD_BASE + si] >= 0.5;
        self.step = self.step.wrapping_add(1);
        let mode = p[ARP_MODE] as usize;
        let vmode = p[ARP_VELMODE] as usize;
        let oct = (p[ARP_OCT] as usize).clamp(1, 4);
        let m = self.n;
        // Ascending notes, unless the mode plays them as pressed.
        let mut sorted = self.held;
        if mode != 7 {
            sorted[..m].sort_by_key(|h| h.note);
        }
        let total = m * oct;
        let k = self.step - 1;
        let idx = match mode {
            0 => k % total,
            1 => total - 1 - k % total,
            2 | 3 => {
                let c = (2 * total).saturating_sub(2).max(1);
                let j = k % c;
                let up = if j < total { j } else { c - j };
                if mode == 2 { up } else { total - 1 - up }
            }
            4 | 5 => {
                let c = 2 * total;
                let j = k % c;
                let up = if j < total { j } else { c - 1 - j };
                if mode == 4 { up } else { total - 1 - up }
            }
            6 => (self.rng.next_u32() as usize) % total,
            7 => k % total,
            // Chord is handled below; the thumb alternates the lowest note with the rest climbing.
            _ => {
                if k.is_multiple_of(2) || total < 2 {
                    0
                } else {
                    1 + (k / 2) % (total - 1)
                }
            }
        };
        let mut emit = |this: &mut Arp, note: u8, vel: f32| {
            let note = (note as i32 + transp).clamp(0, 127) as u8;
            let sv = step_vel;
            let v = match vmode {
                0 | 1 => vel,
                2 | 4 => sv,
                _ => sv * vel,
            };
            if sv <= 0.0 {
                return;
            }
            this.step_vel = sv;
            if this.sounding_n < MAX_HELD {
                this.sounding[this.sounding_n] = note;
                this.sounding_n += 1;
            }
            out(ArpEv::On {
                note,
                vel: v.clamp(0.0, 1.0),
                step_vel: sv,
            });
        };
        if mode == 8 {
            for i in 0..m {
                let h = sorted[i];
                emit(self, h.note, h.vel);
            }
        } else {
            let (note, vel) = self.expanded(&sorted, m, idx);
            emit(self, note, vel);
        }
        self.off_at = len * p[ARP_GATE];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::RindParams;

    fn run(mode: f32, notes: &[u8], steps: usize) -> Vec<u8> {
        let mut p = RindParams::defaults();
        p[ARP_MODE] = mode;
        let mut a = Arp::default();
        for &n in notes {
            a.note_on(n, 1.0, false);
        }
        let mut ons = Vec::new();
        for _ in 0..steps * 13 {
            a.process(500, &p, 120.0, 48_000.0, &mut |e| {
                if let ArpEv::On { note, .. } = e {
                    ons.push(note);
                }
            });
        }
        ons.truncate(steps);
        ons
    }

    #[test]
    fn up_and_down_play_in_order() {
        assert_eq!(run(0.0, &[67, 60, 64], 6), [60, 64, 67, 60, 64, 67]);
        assert_eq!(run(1.0, &[60, 64, 67], 4), [67, 64, 60, 67]);
    }

    #[test]
    fn up_down_turns_around_without_repeating_ends() {
        assert_eq!(run(2.0, &[60, 64, 67], 6), [60, 64, 67, 64, 60, 64]);
        assert_eq!(run(4.0, &[60, 64, 67], 6), [60, 64, 67, 67, 64, 60]);
    }

    #[test]
    fn as_played_keeps_the_order() {
        assert_eq!(run(7.0, &[67, 60, 64], 3), [67, 60, 64]);
    }

    #[test]
    fn octaves_extend_the_sequence() {
        let mut p = RindParams::defaults();
        p[ARP_OCT] = 2.0;
        let mut a = Arp::default();
        a.note_on(60, 1.0, false);
        let mut ons = Vec::new();
        for _ in 0..70 {
            a.process(500, &p, 120.0, 48_000.0, &mut |e| {
                if let ArpEv::On { note, .. } = e {
                    ons.push(note);
                }
            });
        }
        assert_eq!(&ons[..4], &[60, 72, 60, 72]);
    }

    #[test]
    fn chord_plays_everything_at_once_and_rests_on_zero_velocity() {
        let mut p = RindParams::defaults();
        p[ARP_MODE] = 8.0;
        p[ARP_STEP_BASE + 1] = 0.0;
        let mut a = Arp::default();
        a.note_on(60, 1.0, false);
        a.note_on(64, 1.0, false);
        let mut per_step = Vec::new();
        for _ in 0..100 {
            let mut c = 0;
            a.process(500, &p, 120.0, 48_000.0, &mut |e| {
                if matches!(e, ArpEv::On { .. }) {
                    c += 1;
                }
            });
            if c > 0 {
                per_step.push(c);
            }
        }
        assert_eq!(&per_step[..3], &[2, 2, 2], "{per_step:?}");
        assert!(per_step.len() < 16, "a rest step plays nothing");
    }

    #[test]
    fn releasing_stops_the_notes() {
        let p = RindParams::defaults();
        let mut a = Arp::default();
        a.note_on(60, 1.0, false);
        let mut offs = 0;
        for _ in 0..30 {
            a.process(500, &p, 120.0, 48_000.0, &mut |e| {
                offs += matches!(e, ArpEv::Off { .. }) as i32
            });
        }
        a.note_off(60, false);
        a.process(500, &p, 120.0, 48_000.0, &mut |e| {
            offs += matches!(e, ArpEv::Off { .. }) as i32
        });
        assert!(offs > 0);
    }
}

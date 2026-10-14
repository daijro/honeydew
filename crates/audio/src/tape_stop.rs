use crate::biquad::flush;

pub struct TapeStopState {
    params: [f32; 8],
    sample_rate: f32,
    tempo_bpm: f32,
    left: Vec<f32>,
    right: Vec<f32>,
    write_head: u64,
    read_head: f64,
    phase: f32,
    phase_step: f32,
    remaining: usize,
    fade_in: f32,
    held: (f32, f32),
}

impl TapeStopState {
    /// The ring is sized for `tempo_bpm`, the song tempo.
    pub fn new(params: &[f32], sample_rate: f32, tempo_bpm: f32) -> Self {
        assert!(params.len() >= 8);
        assert!(sample_rate.is_finite() && sample_rate > 0.0);
        assert!(tempo_bpm.is_finite() && tempo_bpm > 0.0);
        let size = capacity(sample_rate, tempo_bpm);
        let mut state = Self {
            params: params[..8].try_into().unwrap(),
            sample_rate,
            tempo_bpm,
            left: vec![0.0; size],
            right: vec![0.0; size],
            write_head: 0,
            read_head: 0.0,
            phase: 1.0,
            phase_step: 0.0,
            remaining: 0,
            fade_in: 0.0,
            held: (0.0, 0.0),
        };
        state.reset();
        state
    }

    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        assert!(params.len() >= 8);
        assert!(sample_rate.is_finite() && sample_rate > 0.0);
        if sample_rate != self.sample_rate {
            *self = Self::new(params, sample_rate, self.tempo_bpm);
            return;
        }
        let was_playing = self.params[5] > 0.5;
        let now_playing = params[5] > 0.5;
        self.params.copy_from_slice(&params[..8]);
        if was_playing != now_playing {
            let control = if self.params[7] > 0.5 {
                if now_playing {
                    self.params[4]
                } else {
                    self.params[3]
                }
            } else if now_playing {
                self.params[2]
            } else {
                self.params[1]
            };
            let seconds = if self.params[7] > 0.5 {
                let beats = [0.0, 0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0];
                beats[(9.0 * control).round().clamp(0.0, 9.0) as usize] * 60.0 / self.tempo_bpm
            } else {
                5.0 * control
            };
            let target = (now_playing as u8) as f32;
            self.remaining =
                (seconds.max(0.005) * self.sample_rate * (target - self.phase).abs()) as usize;
            self.remaining = self.remaining.max(1);
            // At a tempo slower than the ring was sized for (a tempo drag; only the build
            // allocates) a ramp is cut to what the ring still holds past the read head's lag (it falls at most a
            // frame behind per ramp frame), so it ends early rather than read wrapped audio. At the sized tempo the
            // ramp keeps its length.
            if capacity(self.sample_rate, self.tempo_bpm) > self.left.len() {
                let lag = (self.write_head as f64 - self.read_head).max(0.0) as usize;
                let room = self.left.len().saturating_sub(lag + 1).max(1);
                self.remaining = self.remaining.min(room);
            }
            self.phase_step = (target - self.phase) / self.remaining as f32;
            self.fade_in = 0.0;
        }
    }

    /// Sizes the ring for `tempo_bpm`, keeping what a running ramp still reads. It allocates,
    /// so the host calls it off the audio thread: at build, with the song tempo.
    pub fn prepare(&mut self, tempo_bpm: f32) {
        assert!(tempo_bpm.is_finite() && tempo_bpm > 0.0);
        self.tempo_bpm = tempo_bpm;
        let needed = capacity(self.sample_rate, tempo_bpm);
        if needed > self.left.len() {
            self.regrow(&mut vec![0.0; needed], &mut vec![0.0; needed]);
        }
    }

    /// Takes the ring of `built`, this module built again for an edit's engine and sized off the audio thread for its
    /// tempo, when that ring is longer: a ramp running through the handover goes on, and the next one fits that tempo.
    /// `built` keeps the shorter one. Allocates nothing.
    pub fn take_ring(&mut self, built: &mut Self) {
        if built.left.len() > self.left.len() && built.sample_rate == self.sample_rate {
            self.regrow(&mut built.left, &mut built.right);
        }
    }

    /// Takes `left` and `right` (longer) as its ring, copying in the frames a running ramp still reads (only those: a
    /// ramp reads nothing written before it started), and leaves its old ring in them.
    fn regrow(&mut self, left: &mut Vec<f32>, right: &mut Vec<f32>) {
        std::mem::swap(&mut self.left, left);
        std::mem::swap(&mut self.right, right);
        let (old, new) = (left.len(), self.left.len());
        let first = match self.remaining {
            0 => self.write_head,
            _ => (self.read_head.floor().max(0.0) as u64)
                .max(self.write_head.saturating_sub(old as u64)),
        };
        // In runs that wrap neither ring.
        let mut frame = first;
        while frame < self.write_head {
            let (from, to) = (frame as usize % old, frame as usize % new);
            let n = ((self.write_head - frame) as usize)
                .min(old - from)
                .min(new - to);
            self.left[to..to + n].copy_from_slice(&left[from..from + n]);
            self.right[to..to + n].copy_from_slice(&right[from..from + n]);
            frame += n as u64;
        }
    }

    /// The block's tempo, for synced ramp times. It never resizes: a ramp that starts at a tempo slower than the ring
    /// was sized for is cut to the ring ([`Self::refresh`]).
    pub fn prepare_block(&mut self, tempo_bpm: f32) {
        assert!(tempo_bpm.is_finite() && tempo_bpm > 0.0);
        self.tempo_bpm = tempo_bpm;
    }

    pub fn process_frame(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
        let write = self.write_head as usize % self.left.len();
        self.left[write] = input_l;
        self.right[write] = input_r;
        self.write_head += 1;
        if self.remaining == 0 {
            self.read_head = self.write_head as f64;
            if self.params[5] > 0.5 {
                self.held = (input_l, input_r);
                return (input_l, input_r);
            }
            let smoothing = (1.0 - 44.1 / self.sample_rate).max(0.0);
            self.held.0 = flush(self.held.0 * smoothing);
            self.held.1 = flush(self.held.1 * smoothing);
            return self.held;
        }
        let mut output = (
            read(&self.left, self.read_head),
            read(&self.right, self.read_head),
        );
        let speed = self.playback_speed();
        self.read_head += speed as f64;
        self.phase = (self.phase + self.phase_step).clamp(0.0, 1.0);
        self.remaining -= 1;
        if self.params[5] > 0.5 {
            let crossfade_frames = (0.1 * self.sample_rate).max(1.0) as usize;
            if self.remaining < crossfade_frames {
                self.fade_in = (self.fade_in + 1.0 / crossfade_frames as f32).min(1.0);
                output.0 += (input_l - output.0) * self.fade_in;
                output.1 += (input_r - output.1) * self.fade_in;
            }
        }
        self.held = output;
        output
    }

    pub fn playback_speed(&self) -> f32 {
        let warp = 2.0 * self.params[6] - 1.0;
        let curve = if warp.abs() < 0.005 {
            0.005
        } else {
            1.0 - 0.005_f32.powf(-warp)
        };
        (((1.0 - curve).powf(-self.phase) - 1.0) * (1.0 / curve - 1.0)).clamp(0.0, 1.0)
    }

    pub fn reset(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.write_head = 0;
        self.read_head = 0.0;
        self.phase = if self.params[5] > 0.5 { 1.0 } else { 0.0 };
        self.phase_step = 0.0;
        self.remaining = 0;
        self.fade_in = 0.0;
        self.held = (0.0, 0.0);
    }
}

fn capacity(sample_rate: f32, tempo_bpm: f32) -> usize {
    ((sample_rate * (960.0 / tempo_bpm).max(5.0)).ceil() as usize)
        .max(2)
        .next_power_of_two()
}

fn read(buffer: &[f32], head: f64) -> f32 {
    let length = buffer.len();
    let frame = head.floor() as usize;
    let fraction = head.fract() as f32;
    let a = buffer[frame % length];
    a + (buffer[(frame + 1) % length] - a) * fraction
}

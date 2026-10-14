//! The drawn playhead (while following, the line sits at the song position, updated every drawn frame). The engine's
//! snapshot moves only once an audio block, so the app draws the playhead at the snapshot's tick extrapolated at the
//! tempo since that tick first showed, never more than about one snapshot interval ahead and never back unless the
//! engine went back (a seek, a loop or song-end wrap, Stop). For drawing only: edits, seeks and recording use the
//! snapshot's tick.

/// The playhead the editors draw, one step a frame ([`Self::advance`]).
///
/// Each frame it moves on by the frame's time at the tempo, kept between the snapshot's tick and that tick plus one
/// snapshot interval at the tempo (the longer of the last two times between snapshot changes, so that a block landing
/// on alternate frames counts whole), never past the tick the engine wraps at, and never back: past either bound it
/// waits for the engine. A snapshot behind the last one (a seek back, a loop or song-end wrap) is taken at once; one
/// ahead past the drawn tick (a seek) too. Until the snapshot first moves after Play it stays on it, so a transport held
/// by a count-in or a starting device is not run ahead of.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayheadClock {
    /// The tick drawn, fractional.
    drawn: f64,
    /// The last snapshot's tick and the egui time it first showed.
    tick: u64,
    since: f64,
    /// The last time between snapshot changes, and the longer of the last two (0 until one is seen), seconds.
    gap: f64,
    interval: f64,
    /// The last frame's time, while the transport runs.
    frame: Option<f64>,
}

impl PlayheadClock {
    /// One frame at egui time `now`, the snapshot at `tick`; `rate` is the transport's ticks a second while it runs,
    /// `None` while it is stopped (or counting in, or without a running output); `end` is the tick the engine wraps
    /// at (its loop's end, or the song length), infinite when it plays on. The tick to draw: `tick` or later.
    pub fn advance(&mut self, now: f64, tick: u64, rate: Option<f64>, end: f64) -> u64 {
        let (Some(rate), Some(last)) = (rate, self.frame) else {
            // Stopped: the snapshot. The first frame playing starts there.
            *self = Self {
                drawn: tick as f64,
                tick,
                since: now,
                frame: rate.map(|_| now),
                ..Self::default()
            };
            return tick;
        };
        let s = tick as f64;
        let mut step = rate * (now - last);
        if tick != self.tick {
            if tick < self.tick {
                (self.drawn, step) = (s, 0.0);
            }
            let gap = now - self.since;
            self.interval = gap.max(self.gap);
            (self.gap, self.tick, self.since) = (gap, tick, now);
        }
        let upper = (s + rate * self.interval).max(self.drawn);
        // Not past the wrap: the snapshot that shows it brings the line back.
        self.drawn = (self.drawn + step).min(upper).min(end).max(s);
        self.frame = Some(now);
        // ponytail: whole ticks (the editors take u64 ticks): about 4 a 60 Hz frame at 120 BPM. Hand the editors a
        // fractional tick if a very slow tempo at a deep zoom ever shows tick steps.
        self.drawn as u64
    }
}

#[cfg(test)]
mod tests {
    use super::PlayheadClock;

    /// 120 BPM at 128 ticks a beat.
    const RATE: f64 = 256.0;
    const FRAME: f64 = 1.0 / 60.0;

    /// The ticks drawn over `frames` frames at 60 Hz, the engine rendering blocks of `block` seconds: each frame's
    /// snapshot shows the blocks rendered by then.
    fn run(clock: &mut PlayheadClock, frames: usize, block: f64) -> Vec<u64> {
        (0..frames)
            .map(|i| {
                let now = i as f64 * FRAME;
                let tick = ((now / block).floor() * block * RATE) as u64;
                clock.advance(now, tick, Some(RATE), f64::INFINITY)
            })
            .collect()
    }

    #[test]
    fn between_snapshots_the_playhead_moves_on_every_frame_at_the_tempo() {
        // Blocks of 2048 frames at 48 kHz (43 ms) and of 512 (11 ms): the snapshot steps every two or three frames,
        // or by one or two blocks a frame.
        for block in [2048.0 / 48_000.0, 512.0 / 48_000.0] {
            let mut clock = PlayheadClock::default();
            let drawn = run(&mut clock, 120, block);
            // After the first two snapshots (the interval is known), every frame moves it on by about a frame's ticks.
            let per_frame = RATE * FRAME;
            for (i, w) in drawn.windows(2).enumerate().skip(8) {
                let step = (w[1] - w[0]) as f64;
                assert!(
                    step >= 0.5 * per_frame && step <= 2.0 * per_frame,
                    "block {block}: frame {i}: {} to {}",
                    w[0],
                    w[1]
                );
            }
            // Never more than about a snapshot interval ahead of the engine.
            let last = 119.0 * FRAME;
            let engine = ((last / block).floor() * block * RATE) as u64;
            assert!(drawn[119] >= engine);
            assert!((drawn[119] - engine) as f64 <= RATE * (block + FRAME) + 1.0);
        }
    }

    #[test]
    fn a_wrap_or_a_seek_in_a_snapshot_shows_at_once_and_stopped_it_is_the_snapshot() {
        let mut clock = PlayheadClock::default();
        let block = 2048.0 / 48_000.0;
        let drawn = run(&mut clock, 30, block);
        let now = 30.0 * FRAME;
        // A loop or song-end wrap: back to the loop start that very frame.
        assert_eq!(clock.advance(now, 64, Some(RATE), f64::INFINITY), 64);
        // And on from there.
        assert!(clock.advance(now + FRAME, 64, Some(RATE), f64::INFINITY) >= 64);
        // A seek ahead: there at once.
        assert_eq!(
            clock.advance(now + 2.0 * FRAME, 5000, Some(RATE), f64::INFINITY),
            5000
        );
        // A seek back while playing.
        assert_eq!(
            clock.advance(now + 3.0 * FRAME, 300, Some(RATE), f64::INFINITY),
            300
        );
        // Stopped: the snapshot's tick, even behind what was drawn.
        assert!(drawn[29] > 0);
        assert_eq!(
            clock.advance(now + 4.0 * FRAME, 290, None, f64::INFINITY),
            290
        );
        assert_eq!(
            clock.advance(now + 9.0 * FRAME, 290, None, f64::INFINITY),
            290
        );
    }

    #[test]
    fn an_engine_that_stalls_after_moving_is_run_ahead_of_by_one_interval_at_most() {
        // 43 ms blocks, then no block for half a second (an underrun, a song end that does not wrap).
        let block = 2048.0 / 48_000.0;
        let mut clock = PlayheadClock::default();
        let drawn = run(&mut clock, 30, block);
        let held = ((29.0 * FRAME / block).floor() * block * RATE) as u64;
        let mut last = drawn[29];
        for i in 30..60 {
            last = clock.advance(i as f64 * FRAME, held, Some(RATE), f64::INFINITY);
            // The interval is the longer of the last two gaps: 50 ms at most here.
            assert!(
                (last - held) as f64 <= RATE * (block + FRAME) + 1.0,
                "frame {i}: {last}"
            );
        }
        // It waits there.
        assert_eq!(
            clock.advance(60.0 * FRAME, held, Some(RATE), f64::INFINITY),
            last
        );
    }

    #[test]
    fn it_never_runs_past_the_tick_the_engine_wraps_at() {
        // A loop [0, 505) at 120 BPM in 43 ms blocks: the snapshot shows 502 for three frames before the block that
        // wraps, which runs a frame's 4 ticks ahead of it each.
        let (block, end) = (2048.0 / 48_000.0, 505.0);
        let mut clock = PlayheadClock::default();
        let mut wrapped = false;
        for i in 0..150 {
            let now = i as f64 * FRAME;
            let tick = ((now / block).floor() * block * RATE) as u64 % 505;
            let drawn = clock.advance(now, tick, Some(RATE), end);
            assert!(drawn <= 505, "frame {i}: {drawn}");
            wrapped |= tick < 64 && i > 30;
        }
        assert!(wrapped);
    }

    #[test]
    fn a_transport_that_does_not_move_is_not_run_ahead_of() {
        // Play pressed, the engine not started yet (or held by a count-in): no snapshot change, no extrapolation.
        let mut clock = PlayheadClock::default();
        for i in 0..20 {
            assert_eq!(
                clock.advance(i as f64 * FRAME, 384, Some(RATE), f64::INFINITY),
                384
            );
        }
    }
}

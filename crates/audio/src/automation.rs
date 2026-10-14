use std::collections::HashSet;

pub const TICKS_PER_BEAT: u64 = 128;
const LEVEL_WIDTH: f64 = 511.984375;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AutomationTarget {
    pub channel: usize,
    pub instance_id: i32,
    pub parameter_index: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomationPoint {
    pub tick: u64,
    pub value: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomationEvent {
    pub tick: u64,
    pub key: i16,
    pub value: u16,
}

impl AutomationEvent {
    pub fn normalized_value(self) -> f32 {
        f32::from(self.value) / 65535.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationError {
    InvalidKey,
    InvalidLoop,
    InvalidTiming,
}

#[derive(Clone, Debug)]
pub struct AutomationClip {
    pub start_tick: u64,
    pub length_ticks: u64,
    pub loop_ticks: u64,
    pub offset_ticks: u64,
    key: i16,
    points: Vec<AutomationPoint>,
    events: Vec<AutomationEvent>,
}

impl AutomationClip {
    pub fn new(
        start_tick: u64,
        length_ticks: u64,
        loop_ticks: u64,
        offset_ticks: u64,
        key: i16,
        points: Vec<AutomationPoint>,
    ) -> Result<Self, AutomationError> {
        if key <= 0 {
            return Err(AutomationError::InvalidKey);
        }
        if loop_ticks == 0 {
            return Err(AutomationError::InvalidLoop);
        }
        let mut clip = Self {
            start_tick,
            length_ticks,
            loop_ticks,
            offset_ticks,
            key,
            points,
            events: Vec::new(),
        };
        clip.rebuild_events();
        Ok(clip)
    }

    pub fn points(&self) -> &[AutomationPoint] {
        &self.points
    }

    pub fn events(&self) -> &[AutomationEvent] {
        &self.events
    }

    pub fn replace_points(&mut self, points: Vec<AutomationPoint>) {
        self.points = points;
        self.rebuild_events();
    }

    pub fn rebuild_events(&mut self) {
        self.points.sort_by_key(|point| point.tick);
        self.events.clear();
        let Some(&first) = self.points.first() else {
            return;
        };
        self.events.push(AutomationEvent {
            tick: first.tick,
            key: self.key,
            value: first.value,
        });
        for pair in self.points.windows(2) {
            let [a, b] = [pair[0], pair[1]];
            let level_a = (f64::from(a.value) / LEVEL_WIDTH).trunc() as i32;
            let level_b = (f64::from(b.value) / LEVEL_WIDTH).trunc() as i32;
            if b.tick > a.tick && (level_b - level_a).abs() >= 2 {
                let direction = (level_b - level_a).signum();
                for distance in 1..(level_b - level_a).abs() {
                    let level = level_a + distance * direction;
                    let fraction = f64::from(distance) / f64::from((level_b - level_a).abs());
                    self.events.push(AutomationEvent {
                        tick: (a.tick as f64 + fraction * (b.tick - a.tick) as f64).round() as u64,
                        key: -self.key,
                        value: (f64::from(level) * LEVEL_WIDTH).trunc().min(65534.0) as u16,
                    });
                }
            }
            self.events.push(AutomationEvent {
                tick: b.tick,
                key: self.key,
                value: b.value,
            });
        }
        self.events.sort_by_key(|event| event.tick);
    }

    fn visible_end(&self) -> u64 {
        self.start_tick.saturating_add(if self.length_ticks == 0 {
            self.loop_ticks
        } else {
            self.length_ticks
        })
    }

    fn first_occurrence(&self, event_tick: u64) -> u64 {
        let offset_phase = self.offset_ticks % self.loop_ticks;
        let event_phase = event_tick % self.loop_ticks;
        let phase = if event_phase >= offset_phase {
            event_phase - offset_phase
        } else {
            self.loop_ticks - (offset_phase - event_phase)
        };
        self.start_tick.saturating_add(phase)
    }
}

#[derive(Clone, Debug)]
pub struct AutomationLane {
    pub target: AutomationTarget,
    pub initial_value: f32,
    pub clips: Vec<AutomationClip>,
}

#[derive(Clone, Debug, Default)]
pub struct AutomationPlayback {
    pub lanes: Vec<AutomationLane>,
}

impl AutomationPlayback {
    pub fn value_at(&self, target: AutomationTarget, song_tick: u64) -> Option<f32> {
        let initial = self
            .lanes
            .iter()
            .find(|lane| lane.target == target)?
            .initial_value;
        self.automated_value_at(target, song_tick).or(Some(initial))
    }

    pub fn seek(&self, song_tick: u64, mut dispatch: impl FnMut(AutomationTarget, f32)) {
        let mut seen = HashSet::new();
        for lane in &self.lanes {
            if seen.insert(lane.target) {
                dispatch(lane.target, lane.initial_value);
            }
        }
        seen.clear();
        for lane in &self.lanes {
            if seen.insert(lane.target)
                && let Some(value) = self.automated_value_at(lane.target, song_tick)
            {
                dispatch(lane.target, value);
            }
        }
    }

    pub fn dispatch_block(
        &self,
        start_beat: f64,
        frame_count: usize,
        sample_rate: f64,
        tempo_bpm: f64,
        mut dispatch: impl FnMut(usize, AutomationTarget, f32),
    ) -> Result<(), AutomationError> {
        if !start_beat.is_finite()
            || start_beat < 0.0
            || !sample_rate.is_finite()
            || sample_rate <= 0.0
            || !tempo_bpm.is_finite()
            || tempo_bpm <= 0.0
        {
            return Err(AutomationError::InvalidTiming);
        }
        if frame_count == 0 {
            return Ok(());
        }
        let start_tick = start_beat * TICKS_PER_BEAT as f64;
        let ticks_per_frame = tempo_bpm * TICKS_PER_BEAT as f64 / (60.0 * sample_rate);
        let search_start_tick = (start_tick - ticks_per_frame).max(0.0);
        let end_tick = (start_beat + frame_count as f64 * tempo_bpm / (60.0 * sample_rate))
            * TICKS_PER_BEAT as f64;
        if !end_tick.is_finite() {
            return Err(AutomationError::InvalidTiming);
        }
        let mut scheduled = Vec::new();
        for lane in &self.lanes {
            for clip in &lane.clips {
                let visible_end = clip.visible_end();
                for event in clip.events() {
                    let first = clip.first_occurrence(event.tick);
                    if first >= visible_end {
                        continue;
                    }
                    let repetitions = ((search_start_tick - first as f64) / clip.loop_ticks as f64)
                        .ceil()
                        .max(0.0) as u64;
                    let Some(mut tick) = repetitions
                        .checked_mul(clip.loop_ticks)
                        .and_then(|delta| first.checked_add(delta))
                    else {
                        continue;
                    };
                    while tick < visible_end && (tick as f64) < end_tick {
                        let frame = ((tick as f64 / TICKS_PER_BEAT as f64 - start_beat)
                            * 60.0
                            * sample_rate
                            / tempo_bpm)
                            .ceil();
                        if frame >= 0.0 && frame < frame_count as f64 {
                            scheduled.push((
                                tick,
                                event.tick,
                                lane.target,
                                frame as usize,
                                event.normalized_value(),
                            ));
                        }
                        let Some(next_tick) = tick.checked_add(clip.loop_ticks) else {
                            break;
                        };
                        tick = next_tick;
                    }
                }
            }
        }
        scheduled.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
        for (_, _, target, frame, value) in scheduled {
            dispatch(frame, target, value);
        }
        Ok(())
    }

    fn automated_value_at(&self, target: AutomationTarget, song_tick: u64) -> Option<f32> {
        let mut latest = None;
        for lane in self.lanes.iter().filter(|lane| lane.target == target) {
            for clip in &lane.clips {
                if song_tick < clip.start_tick || song_tick >= clip.visible_end() {
                    continue;
                }
                for event in clip.events() {
                    let first = clip.first_occurrence(event.tick);
                    if first > song_tick {
                        continue;
                    }
                    let occurrence =
                        first + (song_tick - first) / clip.loop_ticks * clip.loop_ticks;
                    if latest.is_none_or(|(tick, source_tick, _)| {
                        occurrence > tick || (occurrence == tick && event.tick <= source_tick)
                    }) {
                        latest = Some((occurrence, event.tick, event.normalized_value()));
                    }
                }
            }
        }
        latest.map(|(_, _, value)| value)
    }
}

//! Sequencer lanes (`TRKH`), their clips (`CLIP`) and decoded events (`EVN2`).

use super::PPQ;
use super::file::{ClipFileState, LaneFileState};

/// The discriminant is the `DESc` lane kind byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneKind {
    Notes = 0,
    Automation = 1,
    Audio = 2,
    Drums = 3,
}

/// A drum lane's grid, `DESc` @1, which the drum editor follows as its mode. The discriminant is the file byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrumGrid {
    /// A step-sequencer lane: the step sequencer, which draws no hit lengths.
    Steps = 0,
    /// A notes lane: the free note-grid, whose hits keep their lengths (the same `Event::Drum` records).
    Notes = 1,
}

/// What an automation lane drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomationTarget {
    /// 0: the channel itself, `param` indexes [`super::ChannelParams`]. > 0: the module with this instance id in
    /// the same channel's rack. < 0: the drum-sampler slot with id −instance_id, `param` indexes its `PRMS`.
    pub instance_id: i32,
    /// Module param index (≥ 0) or common param ([`Self::POST_GAIN`], [`Self::MIX`], [`Self::WET_PAN`]).
    pub param: i32,
}

/// The common-param ids here follow the engine API (−1/−2/−3 = Post Gain/Mix/Wet Pan),
/// not the file. On a module (`instance_id > 0`) the file has −3 = Post Gain, −2 = Mix, −1 unused, and
/// the reserved aliases 0x7FC0–0x7FC2 = `SMPR[0..2]`. [`Self::model_param`] and [`Self::file_param`] map between
/// them: Wet Pan has only the 0x7FC2 form, and the unused negative file codes
/// −1, −4, −5 … are kept as −4, −7, −8 …
impl AutomationTarget {
    pub const POST_GAIN: i32 = -1;
    pub const MIX: i32 = -2;
    pub const WET_PAN: i32 = -3;

    /// The model param of a file (`DESc` @8) param.
    pub fn model_param(instance_id: i32, file: i32) -> i32 {
        match file {
            _ if instance_id <= 0 => file,
            -3 | 0x7FC0 => Self::POST_GAIN,
            -2 | 0x7FC1 => Self::MIX,
            0x7FC2 => Self::WET_PAN,
            f if f < 0 => f.saturating_sub(3),
            f => f,
        }
    }

    /// The file param of a model param: what the app writes (−3, −2, 0x7FC2).
    pub fn file_param(instance_id: i32, param: i32) -> i32 {
        match param {
            _ if instance_id <= 0 => param,
            Self::POST_GAIN => -3,
            Self::MIX => -2,
            Self::WET_PAN => 0x7FC2,
            p if p < 0 => p.saturating_add(3),
            p => p,
        }
    }

    /// The automation-lane order: `(inst & 0xFF) · 65536 + (file param & 0xFFFF)`.
    pub fn file_order(self) -> i32 {
        ((self.instance_id & 0xFF) << 16)
            + (Self::file_param(self.instance_id, self.param) & 0xFFFF)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub kind: LaneKind,
    /// `Some` for automation lanes.
    pub target: Option<AutomationTarget>,
    /// Automation: `"<Param> : <Module>"`, or the param name for a channel param. Audio: the sample name.
    pub label: String,
    pub clips: Vec<Clip>,
    /// The last allocated clip id (ids start at 1; 0 = none).
    pub last_clip_id: u32,
    /// `DESc` +12: the drum editor's last Swing amount in [0, 1] (0 = none; files hold 0.18…0.45), kept for the
    /// editor's swing control. Swing already moved the clip's hits when it was set, so
    /// playback must not apply it again. 0 on other lanes.
    pub swing: f64,
    /// A drum lane's grid (`DESc` @1). `Steps` on other lanes, whose @1 byte is `file.flag`.
    pub grid: DrumGrid,
    /// `Hide track` (Honeydew's own, an `HDHd` in the file): no playlist row, no clips, no ghost notes, while Settings
    /// `Show hidden tracks` is off. It plays all the same.
    pub hidden: bool,
    pub file: LaneFileState,
}

impl Lane {
    /// An empty lane without a target or label.
    pub fn new(kind: LaneKind) -> Self {
        Self {
            kind,
            target: None,
            label: String::new(),
            clips: Vec::new(),
            last_clip_id: 0,
            swing: 0.0,
            grid: DrumGrid::Steps,
            hidden: false,
            file: LaneFileState::default(),
        }
    }

    /// Equal to `other` but maybe for its current clip (DESc@24): view state, which the playlist's taps set. Clones
    /// `other` only when the two differ there.
    pub fn same_edit(&self, other: &Self) -> bool {
        if self.file.current_clip_id == other.file.current_clip_id {
            return self == other;
        }
        let mut other = other.clone();
        other.file.current_clip_id = self.file.current_clip_id;
        *self == other
    }

    /// Appends an empty clip with the next clip id, showing its whole loop once.
    pub fn add_clip(&mut self, start_tick: u32, loop_len_beats: f64) -> &mut Clip {
        self.last_clip_id += 1;
        let mut file = ClipFileState::default();
        if matches!(self.kind, LaneKind::Audio | LaneKind::Automation) {
            // `ZOOM`: the typical audio and automation view.
            file.zoom = [0.0, 127.0, 1.0 / 96.0, 2.0];
        }
        self.clips.push(Clip {
            id: self.last_clip_id,
            start_tick,
            visible_len_beats: 0.0,
            loop_len_beats,
            offset_beats: 0.0,
            events: Vec::new(),
            muted: false,
            file,
        });
        self.clips.last_mut().expect("just pushed")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    /// Unique within the lane.
    pub id: u32,
    /// Song position, in ticks.
    pub start_tick: u32,
    /// ≤ 0: the source length (`file.source_len_beats`) if set, else the loop length (`CLHd` @0).
    pub visible_len_beats: f64,
    /// The content repeats every `loop_len_beats` inside the visible length.
    pub loop_len_beats: f64,
    /// Where in the content the clip window starts.
    pub offset_beats: f64,
    /// Sorted by `pos`; positions lie in `[0, loop_len_beats × PPQ)`, an automation or envelope point's also on its end
    /// ([`Self::fit_loop`]).
    pub events: Vec<Event>,
    /// `CLHd` +36: the clip is muted (the playlist's `More...` Mute / Unmute). New clips play.
    pub muted: bool,
    pub file: ClipFileState,
}

/// The most plays [`Clip::plays_to_draw`] draws in a pixel column: 128 note rows' worth.
pub const MAX_PLAYS_A_PIXEL: f64 = 128.0;

impl Clip {
    /// The loop length in ticks. `None`: there is no loop, and nothing plays. A loop shorter than one tick is none
    /// (a file may hold any length, and one that short would repeat millions of times on a screen); the playlist and
    /// the audio bridge both go by this.
    pub fn loop_ticks(&self) -> Option<f64> {
        let lp = self.loop_len_beats * f64::from(PPQ);
        (lp >= 1.0).then_some(lp)
    }

    /// The visible length in ticks: when the visible length is ≤ 0, the source length if it is set, else the loop
    /// length (`CLHd` @0).
    pub fn visible_ticks(&self) -> f64 {
        let beats = if self.visible_len_beats > 0.0 {
            self.visible_len_beats
        } else if self.file.source_len_beats != 0.0 {
            self.file.source_len_beats
        } else {
            self.loop_len_beats
        };
        beats * f64::from(PPQ)
    }

    /// The generic clip-update rule: the loop becomes the smallest b·2ᵏ beats that holds the content end, at least b
    /// (b = beats per bar, ≥ 1); it shrinks as well as grows. The content end is the latest event end: an automation or
    /// envelope point ends where it is (CLHd@8: b·2ᵏ ≥ the last point, so a point on b·2ᵏ keeps it); a note or hit at
    /// least a tick after it starts (so a zero-length one stays inside the loop). An audio clip (a source
    /// length S, `file.source_len_beats` > 0) also holds the smallest 4·2ᵐ ≥ trunc(4·S)/4 (CLHd@8), so it is never
    /// below 4 either. The visible length is never written: a set one (> 0) stays fixed, even when it equals the loop,
    /// and the content loops inside it; a new clip's 0 shows the loop, so it grows and shrinks with it.
    pub fn fit_loop(&mut self, b: u32) {
        let end = (self.events.iter())
            .map(|e| {
                let len = match *e {
                    Event::Note { len_beats, .. } | Event::Drum { len_beats, .. } => {
                        (len_beats * f64::from(PPQ)).max(1.0)
                    }
                    Event::Automation { .. } | Event::Envelope { .. } => 0.0,
                };
                f64::from(e.pos()) + len
            })
            .fold(0.0, f64::max);
        // The smallest base·2ᵏ ≥ end, at least base.
        let doubled = |mut n: f64, end: f64| {
            while n < end {
                n *= 2.0;
            }
            n
        };
        let mut need = doubled(f64::from(b.max(1)), end / f64::from(PPQ));
        let source = self.file.source_len_beats;
        if source > 0.0 {
            need = need.max(doubled(4.0, (4.0 * source).trunc() / 4.0));
        }
        self.loop_len_beats = need;
    }

    /// Ticks from the clip start to the first play of content position `pos`: `(pos − offset) mod loop`.
    /// `None` without a loop length.
    fn rel_tick(&self, pos: u32) -> Option<f64> {
        let lp = self.loop_ticks()?;
        Some((f64::from(pos) - self.offset_beats * f64::from(PPQ)).rem_euclid(lp))
    }

    /// The song tick of the first play of content position `pos`, `None` when it falls outside the
    /// visible length. Later loop repetitions: [`Self::events_in_song_range`].
    pub fn song_tick(&self, pos: u32) -> Option<u64> {
        let rel = self
            .rel_tick(pos)
            .filter(|&rel| rel < self.visible_ticks())?;
        Some(u64::from(self.start_tick) + rel.round() as u64)
    }

    /// [`Self::events_in_song_range`] for a screen, `min_pass` ticks a pixel: none when the plays would pile up more
    /// than [`MAX_PLAYS_A_PIXEL`] to a pixel column, a loop pass shorter than `min_pass` × events / 128 (and than a
    /// pixel). A caller that walks only the ticks it shows then visits at most that many plays a pixel, whatever a
    /// file's loop and visible lengths and event count: every per-frame walk over loop passes goes through this.
    pub fn plays_to_draw(
        &self,
        start: u64,
        end: u64,
        min_pass: f64,
    ) -> impl Iterator<Item = (u64, &Event)> {
        let plays = (self.events.len() as f64 / MAX_PLAYS_A_PIXEL).max(1.0);
        let wide = self.loop_ticks().is_some_and(|lp| lp >= min_pass * plays);
        self.events_in_song_range(start, if wide { end } else { start })
    }

    /// Every play of every event at song ticks in `[start, end)`, over all loop repetitions, as
    /// `(song tick, event)`. In time order when `events` is sorted by `pos`.
    pub fn events_in_song_range(
        &self,
        start: u64,
        end: u64,
    ) -> impl Iterator<Item = (u64, &Event)> {
        let (lp, visible) = (self.loop_ticks().unwrap_or(0.0), self.visible_ticks());
        let clip_start = u64::from(self.start_tick);
        // The repetitions k whose span [k·loop, (k+1)·loop) meets the visible part of [start, end), and one more
        // on each side: a fractional offset can round a play onto a neighbouring repetition's tick. The per-play
        // filters below keep only the plays in range.
        let from = start.saturating_sub(clip_start) as f64;
        let to = (end.saturating_sub(clip_start) as f64).min(visible);
        let reps = if lp > 0.0 && end > start && end > clip_start {
            ((from / lp) as u64).saturating_sub(1)..(to / lp).ceil() as u64 + 1
        } else {
            0..0
        };
        // An offset of 0 counts as a whole loop, so a point on the loop end (which plays with content 0) comes first:
        // at that tie the lower position plays last and wins, as `honeydew_audio::automation` dispatches one.
        let off = match (self.offset_beats * f64::from(PPQ)).rem_euclid(lp) {
            0.0 => lp,
            off => off,
        };
        // The events are sorted by position, so a pass walks only those that can play in range, a tick wider each
        // side for the rounding: a heavy clip costs what it shows, not all its events. A position past the next pass
        // (a file's loop shorter than its content) plays wrapped, so such a clip is walked whole.
        let events = &self.events[..];
        let narrow = events.last().is_none_or(|e| f64::from(e.pos()) < off + lp);
        let at = move |pos: f64| events.partition_point(|e| f64::from(e.pos()) < pos);
        reps.flat_map(move |k| {
            // A position p at or after the offset plays at p − off in its pass, one before it at p − off + loop.
            let (lo, hi) = (from - k as f64 * lp - 1.0, to - k as f64 * lp + 1.0);
            let (late, early) = match narrow {
                true => {
                    let late = at(off.max(lo + off));
                    let early = at(lo + off - lp);
                    let (late_end, early_end) = (at(hi + off), at((hi + off - lp).min(off)));
                    (
                        &events[late..late_end.max(late)],
                        &events[early..early_end.max(early)],
                    )
                }
                false => (events, events),
            };
            // Content at or after the offset plays first in each repetition.
            let late = late.iter().filter(move |e| f64::from(e.pos()) >= off);
            let early = early.iter().filter(move |e| f64::from(e.pos()) < off);
            late.chain(early).filter_map(move |e| {
                let t = self.rel_tick(e.pos())? + k as f64 * lp;
                let tick = clip_start + t.round() as u64;
                (t < visible && (start..end).contains(&tick)).then_some((tick, e))
            })
        })
    }
}

/// An `EVN2` record, decoded per lane kind. `pos` is in ticks from the content start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Notes lane. `key` is the MIDI note; `velocity` and `pan` are /255. `slide` is the `EVN2` @19 slide flag (1 = a
    /// slide note), set by the piano roll's `Toggle Slide`; new notes don't slide, and playback ignores it for now.
    Note {
        pos: u32,
        len_beats: f64,
        key: u8,
        velocity: u8,
        pan: u8,
        slide: bool,
    },
    /// Drum-pattern lane. `pad` is the slot pad index; `value` is the per-hit Pitch, bipolar, 0x7FFF = centre, and
    /// `repeat` the per-hit Repeat count (`EVN2` @18), 0..=8, 0 = a single hit. The drum editor's graph lane
    /// edits both; playback ignores them for now.
    Drum {
        pos: u32,
        len_beats: f64,
        pad: u8,
        velocity: u8,
        pan: u8,
        value: u16,
        repeat: u8,
    },
    /// Automation lane: `value` in [0, 1]; `control` = user control point, else a generated step.
    Automation { pos: u32, value: f32, control: bool },
    /// Audio clip envelope point, with `env_param` identifying its destination.
    Envelope {
        pos: u32,
        env_param: i16,
        value: f32,
        control: bool,
    },
}

impl Event {
    /// 204/255 = 0.8.
    pub const DEFAULT_VELOCITY: u8 = 204;
    /// Centre.
    pub const DEFAULT_PAN: u8 = 127;
    /// A drum hit's per-hit Pitch at its centre (no change).
    pub const DEFAULT_PITCH: u16 = 0x7FFF;

    pub fn pos(&self) -> u32 {
        match *self {
            Self::Note { pos, .. }
            | Self::Drum { pos, .. }
            | Self::Automation { pos, .. }
            | Self::Envelope { pos, .. } => pos,
        }
    }
}

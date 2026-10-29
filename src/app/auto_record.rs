//! REC's live automation recording, the pure rules: which
//! clip a control's change goes to, and the point it adds there. The app arms REC, starts the transport and counts in;
//! while it rolls armed, each change of the recent control comes here, and so does each MIDI controller that reaches a
//! channel recording notes ([`HoneydewApp::record_cc`]).

use super::{HoneydewApp, tracks};
use crate::midi::route::Target;
use crate::midi::{MidiMsg, Out};
use crate::params::{self, ParamKind, ParamSpec, menu_index, menu_value};
use crate::song::points::{self, Point, PointKey};
use crate::song::{AutomationTarget, Channel, Clip, LaneKind, PPQ, Song};
use crate::ui::views::SongEdit;

/// The beats a recording clip starts on a multiple of: start = floor(beat/4)·4 (4, not the bar's b).
const START_BEATS: u32 = 4;
/// The count-in's beats.
pub(super) const COUNT_IN_BEATS: u32 = 4;

/// What REC records into, from its first change until the recording ends.
#[derive(Default)]
pub(super) struct AutoRecord {
    /// The recent control (channel id, target) and its value as the last frame left it: what a change starts from.
    seen: Option<(i32, AutomationTarget, f32)>,
    /// The clips being recorded: per channel id and target, the lane and the clip id.
    clips: Vec<(i32, AutomationTarget, usize, u32)>,
    /// The arm as the engine last heard it (`AudioCommand::Metronome`).
    pub(super) armed: bool,
    /// A recording that changed the song ended this frame: the app closes its undo step with the notes' take.
    pub(super) ended: bool,
    /// A change made a lane this frame and moved the clips after it already: the app's re-key of that change, which
    /// comes next, leaves them where they are ([`Self::rekey`]).
    shifted: bool,
}

/// Where a change at a song tick is recorded, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    /// Clip `.1` of lane `.0`: the clip already being recorded, or the current lane's empty clip at the start, which the
    /// recording replaces.
    Clip(usize, u32),
    /// A new clip at the start on lane `.0`: the first lane with the target that no clip covers there.
    Lane(usize),
    /// A new lane, with a new clip at the start.
    NewLane,
}

impl AutoRecord {
    /// The recent control `now` (channel id, target, value) after a frame: its value before, if the frame changed it.
    // ponytail: a control pressed and changed in one frame (a press and its release in the same frame) is seen only
    // from its next change on; a `SetParam` tap in `AudioLink` would see every change, of every control at once.
    pub(super) fn moved(&mut self, now: Option<(i32, AutomationTarget, f32)>) -> Option<f32> {
        let (id, target, old) = std::mem::replace(&mut self.seen, now)?;
        let (now_id, now_target, new) = now?;
        (id == now_id && target == now_target && old != new).then_some(old)
    }

    /// Records the change of `target` on channel `channel` from `old` to `new` at song tick `tick`: into the
    /// [`choose`]n clip, made first if needed, quantized, with a step point for a discrete param ([`record`]). `current`
    /// is the current lane, if it is this channel's. `None` when the song is unchanged, else the index of the lane it
    /// added, if any.
    pub(super) fn record(
        &mut self,
        song: &mut Song,
        channel: usize,
        target: AutomationTarget,
        (old, new): (f32, f32),
        tick: u64,
        current: Option<usize>,
    ) -> Option<Option<usize>> {
        // The re-key that follows the change that set it consumed it.
        debug_assert!(!self.shifted, "a lane added without the app's re-key");
        let bar = song.time_sig.bar_beats();
        let label = song.automation_label(channel, target).unwrap_or_default();
        let ch = song.channels.get_mut(channel)?;
        let key = |&(c, t, ..): &(i32, AutomationTarget, usize, u32)| (c, t) == (ch.id, target);
        let at = self.clips.iter().position(key);
        let recording = at.map(|i| (self.clips[i].2, self.clips[i].3));
        let start = u32::try_from(start(tick)).ok()?;
        let (lane, id, added) = match choose(ch, target, tick, recording, current) {
            Choice::Clip(lane, id) => (lane, id, None),
            Choice::Lane(lane) => (
                lane,
                ch.lanes[lane].add_clip(start, f64::from(bar)).id,
                None,
            ),
            Choice::NewLane => {
                let new = ch.add_automation_lane(target);
                new.label = label;
                let new: *const _ = new;
                let lane = (ch.lanes.iter())
                    .position(|l| std::ptr::eq(l, new))
                    .expect("added");
                // The lanes after it moved down one, the clips recorded there with them.
                for c in self
                    .clips
                    .iter_mut()
                    .filter(|c| c.0 == ch.id && c.2 >= lane)
                {
                    c.2 += 1;
                }
                self.shifted = true;
                (
                    lane,
                    ch.lanes[lane].add_clip(start, f64::from(bar)).id,
                    Some(lane),
                )
            }
        };
        match at {
            Some(i) => self.clips[i] = (ch.id, target, lane, id),
            None => self.clips.push((ch.id, target, lane, id)),
        }
        let spec = spec(ch, target);
        let (new, discrete) = spec.map_or((new, false), |s| quantized(s, new));
        let old = spec
            .map(|s| point_value(quantized(s, old).0))
            .filter(|_| discrete);
        let value = point_value(new);
        let clip = (ch.lanes[lane].clips.iter_mut()).find(|c| c.id == id)?;
        let pos = u32::try_from(tick.saturating_sub(u64::from(clip.start_tick))).ok()?;
        // A new clip is empty: its first point always goes in.
        record(clip, pos, value, old.filter(|&o| o != value), bar).then_some(added)
    }

    /// The recording ended: its clips are forgotten. Whether it changed the song.
    pub(super) fn stop(&mut self) -> bool {
        !std::mem::take(&mut self.clips).is_empty()
    }

    /// The tracks moved (the app's re-key; `ids`: the channel ids before): each clip being recorded goes with its track,
    /// or is forgotten with it, unless this recording made that change itself ([`Self::shifted`]).
    pub(super) fn rekey(
        &mut self,
        ids: &[i32],
        track: &dyn Fn(usize, usize) -> Option<(usize, usize)>,
    ) {
        if std::mem::take(&mut self.shifted) {
            return;
        }
        self.clips.retain_mut(|c| {
            let old = ids.iter().position(|&id| id == c.0);
            let moved = old.and_then(|old| track(old, c.2));
            moved.map(|(_, lane)| c.2 = lane).is_some()
        });
    }

    /// The clips being recorded, as (channel id, lane, clip id).
    pub(super) fn clips(&self) -> impl Iterator<Item = (i32, usize, u32)> + '_ {
        self.clips
            .iter()
            .map(|&(id, _, lane, clip)| (id, lane, clip))
    }

    /// Drops the last point but one of the clip recording `target` on `channel` when it lies on the straight line
    /// between its neighbours, within one MIDI value step (1/127 of the range): a controller's sweep keeps its turns.
    // ponytail: greedy, a point is checked once, against the neighbours it has then; a long gentle curve can drift a
    // few steps from the points it dropped. A pass over the whole take at its end if that shows.
    fn thin(&self, song: &mut Song, channel: usize, target: AutomationTarget) {
        let Some(ch) = song.channels.get_mut(channel) else {
            return;
        };
        let key = |&&(c, t, ..): &&(i32, AutomationTarget, usize, u32)| (c, t) == (ch.id, target);
        let Some(&(.., lane, id)) = self.clips.iter().find(key) else {
            return;
        };
        let clips = ch.lanes.get_mut(lane).map(|l| &mut l.clips);
        let Some(clip) = clips.and_then(|c| c.iter_mut().find(|c| c.id == id)) else {
            return;
        };
        let mut pts = points::points(clip, PointKey::Automation);
        let n = pts.len();
        let [.., a, b, c] = pts[..] else {
            return;
        };
        let (t, v) = (|p: Point| f64::from(p.tick), |p: Point| f64::from(p.value));
        let line = v(a) + (v(c) - v(a)) * (t(b) - t(a)) / (t(c) - t(a));
        let step = f64::from(PointKey::Automation.max()) / 127.0;
        if a.tick < b.tick && b.tick < c.tick && (v(b) - line).abs() <= step {
            pts.remove(n - 2);
            points::set_points(clip, PointKey::Automation, pts);
        }
    }
}

impl HoneydewApp {
    /// Records the change `values` (old, new) of `target` on channel `channel` at song tick `tick`
    /// ([`AutoRecord::record`]), with the current lane if it is this channel's; a lane it adds re-keys the tracks.
    /// Whether the song changed.
    pub(super) fn record_point(
        &mut self,
        channel: usize,
        target: AutomationTarget,
        values: (f32, f32),
        tick: u64,
    ) -> bool {
        let current = self.current_track().filter(|&(c, _)| c == channel);
        let current = current.map(|(_, lane)| lane);
        let song = &mut self.song;
        let Some(added) = (self.auto_record).record(song, channel, target, values, tick, current)
        else {
            return false;
        };
        if let Some(lane) = added {
            // ponytail: a song copy per lane added (`lane_shifts` compares two songs); one per take and target at most.
            let mut before = self.song.clone();
            before.channels[channel].lanes.remove(lane);
            self.rekey(&tracks::lane_shifts(&before, &self.song));
        }
        true
    }

    /// REC's MIDI controllers, at song tick `tick` while a take runs and no count-in does: a CC or pitch bend `msg`
    /// that reaches a channel recording notes (`targets` that [`Self::record_channels`] holds, as for its notes) is a
    /// point of that channel's CC param (7 + CC, pitch bend 4), each in a lane and clip of its own
    /// ([`AutoRecord::record`]), [`AutoRecord::thin`]ned. A CC that moved a link (`outs`) or was learned records nothing
    /// here: a link records through its control. An ongoing edit.
    ///
    /// A Honeydew addition: the spec records the recent control only, one target at a time; here every controller does.
    pub(super) fn record_cc(&mut self, msg: MidiMsg, targets: &[Target], outs: &[Out], tick: u64) {
        // As the router plays them; there is no CC #127 channel param.
        let (param, value) = match msg {
            MidiMsg::Cc { cc, value, .. } if cc < 127 => {
                (7 + i32::from(cc), f32::from(value) / 127.0)
            }
            MidiMsg::PitchBend { value, .. } => (4, f32::from(value) / 16383.0),
            _ => return,
        };
        let linked = (outs.iter()).any(|o| matches!(o, Out::Linked { .. } | Out::LearnedCc(_)));
        if linked || !self.recording() || self.audio.counting_in() {
            return;
        }
        let target = AutomationTarget {
            instance_id: 0,
            param,
        };
        let recording = self.record_channels();
        let mut changed = false;
        let channels = targets.iter().map(|t| t.0);
        for channel in channels.filter(|c| *c > 0 && recording.contains(c)) {
            if self.record_point(channel, target, (value, value), tick) {
                self.auto_record.thin(&mut self.song, channel, target);
                changed = true;
            }
        }
        if changed {
            self.song_edited(SongEdit::Ongoing, tick);
        }
    }
}

/// `v` on the steps of a discrete param `spec` (a toggle's 2, a menu's items, a stepped number's count), and whether it
/// is one; a continuous param's `v` as it is, unless its record snap n puts it on trunc(n·v)/n.
pub(super) fn quantized(spec: ParamSpec, v: f32) -> (f32, bool) {
    if let Some(n) = spec.record_snap.map(f32::from) {
        return ((n * params::clamp01(v)).trunc() / n, false);
    }
    let kind = spec.kind;
    match kind {
        ParamKind::Toggle => (if v >= 0.5 { 1.0 } else { 0.0 }, true),
        ParamKind::Menu { items } | ParamKind::MenuBy { items, .. } => {
            (menu_value(kind.item(v), items.len()), true)
        }
        ParamKind::Continuous { display } => match display.step_count() {
            Some(n) => (menu_value(menu_index(v, n), n), true),
            None => (v, false),
        },
    }
}

/// The clip start for a change at song tick `tick`: floor(beat/4)·4 beats, in ticks.
pub(super) fn start(tick: u64) -> u64 {
    let step = u64::from(START_BEATS * PPQ);
    tick / step * step
}

/// The registry's spec of `target`'s param on `ch`.
pub(crate) fn spec(ch: &Channel, target: AutomationTarget) -> Option<ParamSpec> {
    let index = usize::try_from(target.param);
    Some(match target.instance_id {
        0 => *params::CHANNEL_PARAMS.get(index.ok()?)?,
        id if id < 0 => *params::DRUM_SLOT_PARAMS.get(index.ok()?)?,
        id => match index {
            Ok(i) => ch.rack.get(id)?.param_spec(i)?,
            // −1, −2, −3: Post Gain, Mix, Wet Pan.
            Err(_) => *params::COMMON_PARAMS.get(usize::try_from(-1 - target.param).ok()?)?,
        },
    })
}

/// A recorded value as a stored point: as every new point, at most [`PointKey::max`].
pub(super) fn point_value(v: f32) -> u16 {
    points::raw(v).min(PointKey::Automation.max())
}

/// The clip a change of `target` at song tick `tick` goes to: the clip `recording` (lane, clip id) while the
/// playhead is not before its start; else, at start = floor(beat/4)·4, an empty clip there on the `current` lane if it
/// automates `target`; else the first `target` lane free at the start; else a new lane.
pub(super) fn choose(
    ch: &Channel,
    target: AutomationTarget,
    tick: u64,
    recording: Option<(usize, u32)>,
    current: Option<usize>,
) -> Choice {
    let automates = |lane: usize| {
        let l = ch.lanes.get(lane)?;
        (l.kind == LaneKind::Automation && l.target == Some(target)).then_some(l)
    };
    let recorded = recording.and_then(|(lane, id)| {
        let clip = automates(lane)?.clips.iter().find(|c| c.id == id)?;
        (u64::from(clip.start_tick) <= tick).then_some(Choice::Clip(lane, id))
    });
    let start = start(tick);
    let replaced = current.and_then(|lane| {
        let clips = &automates(lane)?.clips;
        let empty = clips
            .iter()
            .find(|c| u64::from(c.start_tick) == start && c.events.is_empty())?;
        Some(Choice::Clip(lane, empty.id))
    });
    let free = (0..ch.lanes.len()).find(|&lane| {
        automates(lane).is_some_and(|l| {
            let covers = |c: &Clip| {
                let from = f64::from(c.start_tick);
                (from..from + c.visible_ticks()).contains(&(start as f64))
            };
            !l.clips.iter().any(covers)
        })
    });
    recorded
        .or(replaced)
        .or(free.map(Choice::Lane))
        .unwrap_or(Choice::NewLane)
}

/// Adds the change to `value` at content tick `pos` to `clip`: nothing when the clip's last point holds it
/// already; for a discrete param a step point first, its `old` value at the same tick, unless that is the last point
/// already. A last point at `pos` takes the value instead: a step pair's second point, or a lone point (unless it holds
/// the old value, the step of a pair it then starts). The loop then fits the content, b = `bar` beats. Whether the clip
/// changed.
pub(super) fn record(clip: &mut Clip, pos: u32, value: u16, old: Option<u16>, bar: u32) -> bool {
    let mut pts = points::points(clip, PointKey::Automation);
    let n = pts.len();
    let pair = n >= 2 && pts[n - 2].tick == pos;
    match pts.last_mut() {
        Some(last) if last.value == value => return false,
        // A lone point at `pos` holding the old value is this change's step: the pair forms below.
        Some(last) if last.tick == pos && (pair || old != Some(last.value)) => last.value = value,
        _ => add(&mut pts, pos, value, old),
    }
    points::set_points(clip, PointKey::Automation, pts);
    clip.fit_loop(bar);
    true
}

/// [`record`]'s new point at `pos`, after the discrete step from `old` unless the last point is that already.
fn add(pts: &mut Vec<Point>, pos: u32, value: u16, old: Option<u16>) {
    if let Some(old) = old {
        let step = Point {
            tick: pos,
            value: old,
            ..Default::default()
        };
        if pts.last() != Some(&step) {
            points::insert(pts, step);
        }
    }
    points::insert(
        pts,
        Point {
            tick: pos,
            value,
            ..Default::default()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Display;
    use crate::song::{ChannelKind, Event, SynthKind};

    const BEAT: u64 = PPQ as u64;
    /// Channel 1's Level and Mute (instance 0).
    const LEVEL: AutomationTarget = AutomationTarget {
        instance_id: 0,
        param: 0,
    };
    const MUTE: AutomationTarget = AutomationTarget {
        instance_id: 0,
        param: 2,
    };
    /// The Equalizer's (instance 3) `Kind 1`, a three-item menu, and its `Gain 1`.
    const KIND_1: AutomationTarget = AutomationTarget {
        instance_id: 3,
        param: 4,
    };
    const GAIN_1: AutomationTarget = AutomationTarget {
        instance_id: 3,
        param: 1,
    };

    /// A MiniSynth channel (1; the synth is instance 2) with an Equalizer (instance 3) in its rack, and only its notes
    /// lane.
    fn song() -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let rack = &mut song.channels[1].rack;
        let eq = crate::ui::modules_browser::add_module(
            rack,
            3,
            crate::ui::modules_browser::InsertAt::End,
        );
        assert_eq!(eq, 3);
        song
    }

    fn pts(clip: &Clip) -> Vec<(u32, u16)> {
        let p = points::points(clip, PointKey::Automation);
        p.iter().map(|p| (p.tick, p.value)).collect()
    }

    /// The app's re-key after a change ([`HoneydewApp::auto_record`]): here the channels and tracks stay, so only what
    /// the change moved itself has moved.
    ///
    /// [`HoneydewApp::auto_record`]: crate::app::HoneydewApp
    fn rekeyed(rec: &mut AutoRecord, song: &Song) {
        let ids: Vec<_> = song.channels.iter().map(|c| c.id).collect();
        rec.rekey(&ids, &|c, l| Some((c, l)));
    }

    fn empty_clip() -> Clip {
        let mut lane = crate::song::Lane::new(LaneKind::Automation);
        lane.add_clip(0, 4.0).clone()
    }

    // ---- values ----

    #[test]
    fn discrete_params_snap_to_their_steps_and_continuous_ones_stay() {
        let menu = ParamSpec::menu("", 0.0, &["a", "b", "c"]);
        assert_eq!(quantized(menu, 0.3), (0.5, true));
        assert_eq!(quantized(menu, 0.2), (0.0, true));
        assert_eq!(quantized(menu, 0.8), (1.0, true));
        let toggle = ParamSpec::toggle("", 0.0);
        assert_eq!(quantized(toggle, 0.5), (1.0, true));
        assert_eq!(quantized(toggle, 0.49), (0.0, true));
        // A number in steps: −12 … +12 in 25 steps; also behind a special text at one end ("---" at 0).
        const STEPS: Display = Display::Steps {
            min: -12,
            count: 25,
            fmt: params::Fmt::new(0, ""),
        };
        let steps = ParamSpec::cont("", 0.0, STEPS);
        assert_eq!(quantized(steps, 0.51), (0.5, true));
        assert_eq!(quantized(steps, 13.0 / 24.0 - 0.01), (13.0 / 24.0, true));
        let special = Display::Special {
            inner: &STEPS,
            at: params::At::Min,
            text: "---",
        };
        assert_eq!(
            quantized(ParamSpec::cont("", 0.0, special), 0.51),
            (0.5, true)
        );
        let level = ParamSpec::plain("", 0.0);
        assert_eq!(quantized(level, 0.123), (0.123, false));
    }

    #[test]
    fn a_record_snap_truncates_to_its_steps_and_the_param_stays_continuous() {
        // The Gate's Asymmetry: recorded automation snaps to trunc(65v)/65, with no step point.
        let asymmetry = ParamSpec::plain("", 0.0).with_record_snap(65);
        assert_eq!(quantized(asymmetry, 0.5), (32.0 / 65.0, false));
        assert_eq!(quantized(asymmetry, 0.999), (64.0 / 65.0, false));
        assert_eq!(quantized(asymmetry, 1.0), (1.0, false));
        let mut song = song();
        let rack = &mut song.channels[1].rack;
        let gate = crate::ui::modules_browser::add_module(
            rack,
            27,
            crate::ui::modules_browser::InsertAt::End,
        );
        let target = AutomationTarget {
            instance_id: gate,
            param: 2,
        };
        assert_eq!(
            spec(&song.channels[1], target).map(|s| s.record_snap),
            Some(Some(65))
        );
        let mut rec = AutoRecord::default();
        rec.record(&mut song, 1, target, (0.0, 0.5), BEAT, None);
        let lane = song.channels[1]
            .lanes
            .iter()
            .find(|l| l.target == Some(target))
            .unwrap();
        assert_eq!(pts(&lane.clips[0]), [(PPQ, point_value(32.0 / 65.0))]);
    }

    #[test]
    fn the_registry_names_each_targets_kind() {
        let song = song();
        let ch = &song.channels[1];
        let kind = |ch: &Channel, target| spec(ch, target).map(|s| s.kind);
        assert_eq!(kind(ch, MUTE), Some(ParamKind::Toggle));
        assert!(matches!(
            kind(ch, LEVEL),
            Some(ParamKind::Continuous { .. })
        ));
        assert!(matches!(kind(ch, KIND_1), Some(ParamKind::Menu { items }) if items.len() == 3));
        assert!(matches!(
            kind(ch, GAIN_1),
            Some(ParamKind::Continuous { .. })
        ));
        // The common params (Mix is −2) and a drum slot's (Pan is param 0, a slot id 1 → instance −1).
        let mix = AutomationTarget {
            instance_id: 3,
            param: AutomationTarget::MIX,
        };
        assert!(matches!(kind(ch, mix), Some(ParamKind::Continuous { .. })));
        let mut drums = Channel::new(String::new(), ChannelKind::Drums, 0.0);
        drums.drum_kit = Some(crate::song::DrumKit::default_kit());
        let slot = drums.drum_kit.as_ref().unwrap().slots[0].file.id as i32;
        let pan = AutomationTarget {
            instance_id: -slot,
            param: 0,
        };
        assert!(kind(&drums, pan).is_some());
        // Nothing there.
        let gone = AutomationTarget {
            instance_id: 9,
            param: 1,
        };
        assert_eq!(kind(ch, gone), None);
    }

    #[test]
    fn a_recorded_value_tops_out_at_the_automation_maximum() {
        assert_eq!(point_value(0.0), 0);
        assert_eq!(point_value(0.5), 32768);
        assert_eq!(point_value(1.0), 65534);
    }

    // ---- points ----

    #[test]
    fn a_change_adds_a_point_only_when_it_differs_from_the_last_one() {
        let mut clip = empty_clip();
        assert!(record(&mut clip, 10, 100, None, 4));
        assert!(!record(&mut clip, 20, 100, None, 4), "the same value");
        assert!(record(&mut clip, 30, 200, None, 4));
        assert_eq!(pts(&clip), [(10, 100), (30, 200)]);
        // The generated steps are rebuilt from the points, never piled up.
        let generated = |c: &Clip| {
            c.events
                .iter()
                .filter(|e| matches!(e, Event::Automation { control: false, .. }))
                .count()
        };
        let before = generated(&clip);
        record(&mut clip, 40, 200, None, 4);
        assert_eq!(generated(&clip), before);
    }

    #[test]
    fn a_discrete_change_steps_from_its_old_value_at_the_same_tick() {
        let mut clip = empty_clip();
        assert!(record(&mut clip, 0, 0, Some(65534), 4));
        assert_eq!(pts(&clip), [(0, 65534), (0, 0)]);
        assert!(record(&mut clip, 256, 65534, Some(0), 4));
        assert_eq!(pts(&clip), [(0, 65534), (0, 0), (256, 0), (256, 65534)]);
        // The step is the last point already: it is not added twice.
        let mut held = empty_clip();
        record(&mut held, 64, 0, None, 4);
        record(&mut held, 64, 65534, Some(0), 4);
        assert_eq!(pts(&held), [(64, 0), (64, 65534)]);
    }

    #[test]
    fn a_change_at_the_last_points_tick_replaces_it_and_a_step_pair_keeps_its_step() {
        let mut clip = empty_clip();
        record(&mut clip, 10, 100, None, 4);
        assert!(record(&mut clip, 10, 200, None, 4));
        assert_eq!(pts(&clip), [(10, 200)]);
        let mut pair = empty_clip();
        record(&mut pair, 0, 0, Some(65534), 4);
        assert!(record(&mut pair, 0, 32768, Some(0), 4));
        assert_eq!(pts(&pair), [(0, 65534), (0, 32768)]);
    }

    #[test]
    fn a_discrete_change_within_one_step_adds_no_step_point() {
        let mut song = song();
        let mut rec = AutoRecord::default();
        rec.record(&mut song, 1, KIND_1, (0.0, 0.7), 0, None);
        rekeyed(&mut rec, &song);
        // The control moved within item 0 (0.1 → 0.2): its old item is the new one, so no step precedes it.
        rec.record(&mut song, 1, KIND_1, (0.1, 0.2), 2 * BEAT, None);
        let lane = song.channels[1]
            .lanes
            .iter()
            .find(|l| l.target == Some(KIND_1))
            .unwrap();
        assert_eq!(pts(&lane.clips[0]), [(0, 0), (0, 32768), (2 * PPQ, 0)]);
    }

    #[test]
    fn the_clip_starts_on_four_beats_whatever_the_bar() {
        // floor(beat/4)·4, not floor(beat/b)·b: at b = 3, beat 5 starts a clip at beat 4 (b would give 3).
        let mut song = song();
        song.time_sig.beats_per_bar = 3;
        let mut rec = AutoRecord::default();
        rec.record(&mut song, 1, LEVEL, (0.8, 0.5), 5 * BEAT, None);
        let lane = song.channels[1]
            .lanes
            .iter()
            .find(|l| l.target == Some(LEVEL))
            .unwrap();
        assert_eq!(lane.clips[0].start_tick, 4 * PPQ);
        assert_eq!(
            lane.clips[0].loop_len_beats, 3.0,
            "the loop is still in bars"
        );
        assert_eq!(start(5 * BEAT), 4 * BEAT);
        assert_eq!(start(3 * BEAT), 0);
    }

    #[test]
    fn the_loop_grows_to_hold_the_points_in_bars_doubled() {
        let mut clip = empty_clip();
        record(&mut clip, 3 * PPQ, 1, None, 3);
        assert_eq!(clip.loop_len_beats, 3.0, "3 holds a point on beat 3");
        record(&mut clip, 3 * PPQ + 1, 2, None, 3);
        assert_eq!(clip.loop_len_beats, 6.0, "3·2 holds a tick past it");
        record(&mut clip, 7 * PPQ, 3, None, 3);
        assert_eq!(clip.loop_len_beats, 12.0);
    }

    // ---- the clip choice ----

    #[test]
    fn the_clip_being_recorded_is_reused_while_the_playhead_is_not_before_it() {
        let mut song = song();
        let ch = &mut song.channels[1];
        let lane = ch.add_automation_lane(LEVEL);
        let id = lane.add_clip(8 * PPQ, 4.0).id;
        let lane = ch.lanes.len() - 1;
        let ch = &song.channels[1];
        assert_eq!(
            choose(ch, LEVEL, 9 * BEAT, Some((lane, id)), None),
            Choice::Clip(lane, id)
        );
        assert_eq!(
            choose(ch, LEVEL, 8 * BEAT, Some((lane, id)), None),
            Choice::Clip(lane, id)
        );
        // Before it (the song loop wrapped): chosen again. Its lane is free at beat 4.
        assert_eq!(
            choose(ch, LEVEL, 5 * BEAT, Some((lane, id)), None),
            Choice::Lane(lane)
        );
        // The clip being recorded comes first, even with the current lane's empty clip at the new start.
        let mut song = song.clone();
        let current = song.channels[1].add_automation_lane(LEVEL);
        current.add_clip(8 * PPQ, 4.0);
        let current = song.channels[1].lanes.len() - 1;
        let ch = &song.channels[1];
        assert_eq!(
            choose(ch, LEVEL, 9 * BEAT, None, Some(current)),
            Choice::Clip(current, 1)
        );
        let recording = choose(ch, LEVEL, 9 * BEAT, Some((lane, id)), Some(current));
        assert_eq!(recording, Choice::Clip(lane, id));
        // A reference gone stale (another target's lane, or no such clip) is chosen again too.
        assert_eq!(
            choose(ch, LEVEL, 9 * BEAT, Some((0, id)), None),
            Choice::NewLane
        );
        assert_eq!(
            choose(ch, LEVEL, 9 * BEAT, Some((lane, 7)), None),
            Choice::NewLane
        );
    }

    #[test]
    fn the_current_lanes_empty_clip_at_the_four_beat_start_is_replaced() {
        let mut song = song();
        let ch = &mut song.channels[1];
        let lane = ch.add_automation_lane(LEVEL);
        let id = lane.add_clip(4 * PPQ, 4.0).id;
        let lane = ch.lanes.len() - 1;
        let ch = &song.channels[1];
        // Beat 7½: start = 4.
        let tick = 7 * BEAT + 64;
        assert_eq!(
            choose(ch, LEVEL, tick, None, Some(lane)),
            Choice::Clip(lane, id)
        );
        // Not the current lane, another target, or not at the start: a clip there covers the start, so a new lane.
        assert_eq!(choose(ch, LEVEL, tick, None, None), Choice::NewLane);
        assert_eq!(choose(ch, GAIN_1, tick, None, Some(lane)), Choice::NewLane);
        assert_eq!(
            choose(ch, LEVEL, 9 * BEAT, None, Some(lane)),
            Choice::Lane(lane)
        );
        // A clip with points is no empty clip.
        let mut song = song.clone();
        let clip = &mut song.channels[1].lanes[lane].clips[0];
        record(clip, 0, 5, None, 4);
        assert_eq!(
            choose(&song.channels[1], LEVEL, tick, None, Some(lane)),
            Choice::NewLane
        );
    }

    #[test]
    fn the_first_lane_of_the_target_free_at_the_start_takes_a_new_clip() {
        let mut song = song();
        let ch = &mut song.channels[1];
        // Lane a: a clip over beats 0…8; lane b: free at 4; lane c: free too, but later.
        let a = ch.add_automation_lane(LEVEL);
        a.add_clip(0, 8.0);
        let b = ch.add_automation_lane(LEVEL);
        b.add_clip(8 * PPQ, 4.0);
        ch.add_automation_lane(LEVEL);
        let first = ch.lanes.len() - 3;
        let ch = &song.channels[1];
        assert_eq!(
            choose(ch, LEVEL, 5 * BEAT, None, None),
            Choice::Lane(first + 1)
        );
        assert_eq!(choose(ch, LEVEL, 9 * BEAT, None, None), Choice::Lane(first));
        // No lane of the target: a new one.
        assert_eq!(choose(ch, MUTE, 0, None, None), Choice::NewLane);
    }

    // ---- the recording ----

    #[test]
    fn a_recording_makes_its_lane_and_clip_once_and_records_into_them() {
        let mut song = song();
        let mut rec = AutoRecord::default();
        let lanes = song.channels[1].lanes.len();
        // Beat 5¼: a new lane, its clip at beat 4, the point a beat and a quarter in.
        let added = rec.record(&mut song, 1, LEVEL, (0.8, 0.5), 5 * BEAT + 32, None);
        rekeyed(&mut rec, &song);
        let lane = lanes;
        assert_eq!(added, Some(Some(lane)), "after the notes lane");
        let ch = &song.channels[1];
        assert_eq!(ch.lanes.len(), lanes + 1);
        assert_eq!(ch.lanes[lane].target, Some(LEVEL));
        assert_eq!(ch.lanes[lane].label, "Level");
        let [clip] = &ch.lanes[lane].clips[..] else {
            panic!()
        };
        assert_eq!(clip.start_tick, 4 * PPQ);
        assert_eq!(pts(clip), [(PPQ + 32, 32768)], "continuous: no step point");
        // Later changes go to the same clip; the same value adds nothing.
        assert_eq!(
            rec.record(&mut song, 1, LEVEL, (0.5, 0.25), 6 * BEAT, None),
            Some(None)
        );
        assert_eq!(
            rec.record(&mut song, 1, LEVEL, (0.25, 0.25), 7 * BEAT, None),
            None
        );
        let clip = &song.channels[1].lanes[lane].clips[0];
        assert_eq!(pts(clip), [(PPQ + 32, 32768), (2 * PPQ, 16384)]);
        assert!(rec.stop());
        assert!(!rec.stop(), "forgotten");
        // A new recording chooses again: the lane's clip covers beat 5, so another lane.
        rec.record(&mut song, 1, LEVEL, (0.25, 0.75), 5 * BEAT, None);
        let levels = song.channels[1]
            .lanes
            .iter()
            .filter(|l| l.target == Some(LEVEL));
        assert_eq!(levels.count(), 2);
    }

    #[test]
    fn a_discrete_recording_quantizes_and_steps_and_a_lane_added_before_another_keeps_its_clip() {
        let mut song = song();
        let mut rec = AutoRecord::default();
        // Kind 1 (file order after Level) first, then Level sorts before it: Kind 1's clip moves down a lane.
        rec.record(&mut song, 1, KIND_1, (0.0, 0.7), 0, None);
        rekeyed(&mut rec, &song);
        rec.record(&mut song, 1, LEVEL, (0.8, 0.1), BEAT, None);
        rekeyed(&mut rec, &song);
        rec.record(&mut song, 1, KIND_1, (0.7, 0.2), 2 * BEAT, None);
        let ch = &song.channels[1];
        let kinds: Vec<_> = ch
            .lanes
            .iter()
            .filter(|l| l.target == Some(KIND_1))
            .collect();
        let [kind_1] = &kinds[..] else {
            panic!("{kinds:?}")
        };
        let [clip] = &kind_1.clips[..] else { panic!() };
        // 0.7 → item 1 (0.5), 0.2 → item 0; each from the old item at the same tick.
        assert_eq!(
            pts(clip),
            [(0, 0), (0, 32768), (2 * PPQ, 32768), (2 * PPQ, 0)]
        );
    }

    #[test]
    fn moved_reports_the_old_value_of_the_same_control_only() {
        let mut rec = AutoRecord::default();
        assert_eq!(rec.moved(Some((1, LEVEL, 0.5))), None, "nothing seen yet");
        assert_eq!(rec.moved(Some((1, LEVEL, 0.5))), None, "unchanged");
        assert_eq!(rec.moved(Some((1, LEVEL, 0.25))), Some(0.5));
        assert_eq!(rec.moved(Some((1, MUTE, 1.0))), None, "another control");
        assert_eq!(rec.moved(Some((2, MUTE, 0.0))), None, "another channel");
        assert_eq!(rec.moved(None), None);
        assert_eq!(rec.moved(Some((2, MUTE, 1.0))), None, "seen again only now");
    }
}

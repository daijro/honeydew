//! Audio clip lanes: each clip's `CLSm` slot is one pad of its channel's own [`PadSampler`], so Mute and Solo stay
//! among that channel's clip slots. The clips' starts, loop repetitions, ends and envelope steps are prepared actions;
//! The seek rule relaunches the clips under the playhead.

use banaforge_audio::drums::{DEFAULTS, Pad, PadSampler, SPEED};
use banaforge_audio::mixer::OwnedStereoBuffer;
use banaforge_audio::sample_edit::speed_ratio;

use super::EngineError;
use super::prepare::{Action, ScheduledAction, automation_clip, beat_frame};
use crate::audio_bridge::playing_clips;
use crate::sample_cache::SampleCache;
use crate::song::{Channel, Clip, Event, LaneKind, PPQ, SamplerSlot, Song};

/// The envelopes a clip plays: `env_param` 0, 1, 2 (file keys ±1, ±2, ±3) set the slot's Level, Pan and Speed, which
/// are slot params 0, 1, 2. Other envelopes are ignored.
const ENVELOPES: [i16; 3] = [0, 1, 2];

/// A channel's audio clips as the engine plays them.
pub(super) struct Clips {
    pub sampler: PadSampler,
    /// Per pad, its clip's timing and envelopes, for the seek rule.
    spans: Vec<Span>,
    /// The channel's first lane is audio: the seek rule cuts its clip voices first.
    cut: bool,
}

struct Span {
    /// The clip's start and end (start + visible length), as frames.
    start: u64,
    end: u64,
    /// Its start, loop length and content offset (`CLHd`@16), in beats.
    start_beat: f64,
    loop_beats: f64,
    offset: f64,
    /// Per envelope, its steps (content tick, value) in tick order.
    envelopes: [Vec<(u32, f32)>; 3],
    /// Its slot's `STR1` mode is on (SYNC): the sample is stretched for the build's tempo.
    sync: bool,
}

impl Clips {
    /// Adds one block of every clip slot into the channel's buffer, before its rack runs. `speed`: the live tempo over
    /// the build's, which a SYNC clip's sample is stretched for. It resamples by it, so it stays on the beat with the
    /// transport, its pitch moving with the tempo like tape (BanaForge change, #126); SYNC off plays at its native speed.
    pub fn render(&mut self, buffer: &mut OwnedStereoBuffer, speed: f64) {
        for (pad, span) in self.spans.iter().enumerate() {
            let rate = if span.sync { speed } else { 1.0 };
            (self.sampler).process_at(pad, &mut buffer.left, &mut buffer.right, rate);
        }
    }

    /// Retain sample positions, attack/release and every sounding voice while adopting new prepared curves.
    /// Runs at handover on the audio thread: bounded scans and copies, no allocation or sample relaunch.
    pub fn adopt_envelopes(&mut self, old: &Clips, frame: u64, beat: f64) {
        self.sampler.adopt(&old.sampler, Some);
        for (pad, span) in self.spans.iter().enumerate() {
            if frame < span.start || frame >= span.end { continue; }
            let content = ((beat - span.start_beat).rem_euclid(span.loop_beats) + span.offset)
                .rem_euclid(span.loop_beats) * f64::from(PPQ);
            for (index, steps) in span.envelopes.iter().enumerate() {
                let n = steps.partition_point(|&(tick, _)| f64::from(tick) <= content);
                if let Some(&(_, value)) = n.checked_sub(1).and_then(|i| steps.get(i)) {
                    self.sampler.set_param(pad, index, value);
                }
            }
        }
    }

    /// The seek rule at `frame`, song beat `beat`, while playing: on a channel whose first lane is audio every clip
    /// voice is cut; then each clip with start < frame < end relaunches from content beat
    /// c = ((beat − start) mod L) + offset (less L once if ≥ L), source time c·60/BPM, after its envelopes take the
    /// value of their last step strictly before c (none: the slot keeps its value). A clip starting at `frame` starts
    /// through its own action.
    pub fn relaunch(&mut self, frame: u64, beat: f64, tempo_bpm: f32) {
        if self.cut {
            self.sampler.reset();
        }
        for (pad, span) in self.spans.iter().enumerate() {
            if !(span.start < frame && frame < span.end) {
                continue;
            }
            let mut c = (beat - span.start_beat).rem_euclid(span.loop_beats) + span.offset;
            if c >= span.loop_beats {
                c -= span.loop_beats;
            }
            for (index, steps) in span.envelopes.iter().enumerate() {
                let before = steps.partition_point(|&(pos, _)| f64::from(pos) < c * f64::from(PPQ));
                if let Some(&(_, value)) = before.checked_sub(1).and_then(|i| steps.get(i)) {
                    self.sampler.set_param(pad, index, value);
                }
            }
            self.sampler
                .hit_at(pad, 1.0, 0.5, 1.0, c * 60.0 / f64::from(tempo_bpm));
        }
    }

    /// Moves each clip's span to frame `at(frame)`: the frames of another tempo (an edit's engine taking these clips
    /// over at the song's new tempo).
    pub fn retime(&mut self, at: impl Fn(u64) -> u64) {
        for span in &mut self.spans {
            (span.start, span.end) = (at(span.start), at(span.end));
        }
    }

    /// These clips play the sample data `old` did, pad by pad, or none on both
    /// ([`banaforge_audio::drums::PadSample::same_data`]).
    pub fn same_samples(&self, old: &Clips) -> bool {
        let (a, b) = (&self.sampler, &old.sampler);
        a.pads() == b.pads()
            && (0..a.pads()).all(|pad| match (a.sample(pad), b.sample(pad)) {
                (Some(new), Some(old)) => new.same_data(old),
                (new, old) => new.is_none() && old.is_none(),
            })
    }

    /// Fast-fades every clip voice: Stop. Stop otherwise only releases notes, and a clip voice's release with
    /// The default EG R (τ 5.67 s) would play on for many seconds after Stop.
    pub fn fade_all(&mut self) {
        for pad in 0..self.sampler.pads() {
            self.sampler.fade(pad);
        }
    }
}

/// The clips of `channel`'s audio lanes that play a sample, in pad order: the unmuted ones with a loop
/// ([`playing_clips`]) and a `CLSm` slot.
fn clips(channel: &Channel) -> impl Iterator<Item = (&Clip, &SamplerSlot)> {
    let lanes = channel.lanes.iter().filter(|l| l.kind == LaneKind::Audio);
    lanes
        .flat_map(playing_clips)
        .filter_map(|clip| Some((clip, clip.file.sample.as_deref()?)))
}

/// Envelope `env_param` of `clip` as an automation clip of the same timing: its control points and the steps between
/// them.
fn envelope(clip: &Clip, env_param: i16) -> Result<Clip, EngineError> {
    let events = (clip.events.iter())
        .filter_map(|event| match *event {
            Event::Envelope {
                pos,
                env_param: k,
                value,
                control: true,
            } if k == env_param => Some(Event::Automation {
                pos,
                value,
                control: true,
            }),
            _ => None,
        })
        .collect();
    let mut converted = Clip { events, ..clip.clone() };
    // Audio clips store each curve independently; the automation converter reads HDSh.
    if env_param == 1 { converted.file.shapes = clip.file.pan_shapes.clone(); }
    else if env_param != 0 { converted.file.shapes.clear(); }
    let mut result = automation_clip(&converted, false)?;
    // An envelope beginning later must hold the base before its first point,
    // including later loop passes, rather than retaining the previous pass's end.
    if result.events.first().is_some_and(|e| e.pos() > 0) {
        if let Some(slot) = clip.file.sample.as_deref() {
            result.events.insert(0, Event::Automation { pos: 0,
                value: slot.params[env_param as usize], control: false });
        }
    }
    Ok(result)
}

/// The actions one loop pass of an audio clip adds, at most: its envelope steps, and a repetition's fade and start.
pub(super) fn plays_per_pass(clip: &Clip) -> Result<usize, EngineError> {
    let mut plays = 2;
    for env_param in ENVELOPES {
        let steps = envelope(clip, env_param)?;
        // Also account for the clip-start/offset initialization action.
        plays += steps.events.len() + usize::from(!steps.events.is_empty());
    }
    Ok(plays)
}

/// The clips of channel `channel`, `None` without any. A slot plays what `samples` holds for it, its `STR1` stretch
/// (the ratio while the mode, SYNC, is on) and pitch rendered once at load (`Refs::add_slot`, the pads' render). Its
/// params apply as on a pad, except the cut group and the beat offset; live notes never play it. The channel's Pitch
/// Bend bends it as it does the pads, 2^w (#59).
pub(super) fn build(
    song: &Song,
    channel: usize,
    sample_rate: f32,
    tempo_bpm: f32,
    samples: &SampleCache,
) -> Result<Option<Clips>, EngineError> {
    let channel = &song.channels[channel];
    let (mut pads, mut params, mut spans) = (Vec::new(), Vec::new(), Vec::new());
    for (clip, slot) in clips(channel) {
        let file = &slot.file;
        pads.push(Pad {
            sample: samples.pad_sample(slot),
            params: DEFAULTS,
            note: None,
            reverse: file.reverse,
            cut_group: 0,
            overlap: file.overlapping_voices,
        });
        params.push(slot.params);
        let start_beat = f64::from(clip.start_tick) / f64::from(PPQ);
        let end_beat = start_beat + clip.visible_ticks() / f64::from(PPQ);
        let mut envelopes: [Vec<(u32, f32)>; 3] = Default::default();
        for (steps, env_param) in envelopes.iter_mut().zip(ENVELOPES) {
            *steps = (envelope(clip, env_param)?.events.iter())
                .filter_map(|event| match *event {
                    Event::Automation { pos, value, .. } => Some((pos, value)),
                    _ => None,
                })
                .collect();
        }
        spans.push(Span {
            start: beat_frame(start_beat, sample_rate, tempo_bpm)?,
            end: beat_frame(end_beat, sample_rate, tempo_bpm)?,
            start_beat,
            loop_beats: clip.loop_len_beats,
            offset: clip.offset_beats,
            envelopes,
            sync: slot.file.stretch_mode != 0,
        });
    }
    if pads.is_empty() {
        return Ok(None);
    }
    // The slots' own params through the sampler's checked write, as the pads' (a hostile value never reaches a voice).
    let mut sampler = PadSampler::new(pads, sample_rate);
    for (pad, params) in params.into_iter().enumerate() {
        for (index, value) in params.into_iter().enumerate() {
            sampler.set_param(pad, index, value);
        }
    }
    Ok(Some(Clips {
        sampler,
        spans,
        cut: channel
            .lanes
            .first()
            .is_some_and(|l| l.kind == LaneKind::Audio),
    }))
}

/// `CLHd`@28, an audio clip's source length in beats: S = BPM × frames / (rate × 60 × the Speed ratio), with `frames`
/// and `rate` those of the sample after the load-time stretch and pitch, and the Speed ratio
/// 2^(trunc(48·Speed − 24)/12).
pub fn source_beats(frames: usize, rate: u32, speed: f32, bpm: f64) -> f64 {
    bpm * frames as f64 / (f64::from(rate) * 60.0 * speed_ratio(speed))
}

/// Recomputes `clip`'s `CLHd`@28 ([`source_beats`]) and @8 ([`Clip::fit_loop`] with `b` beats a bar) from its slot's
/// sample as the engine renders it. Call it when the clip updates (a new sample, Speed, `STR1` pitch or SYNC, a tempo
/// change), once `samples` has loaded the song; a stored value is not kept. True when either changed. Without a
/// `CLSm` slot, a loaded sample or a finite positive length, nothing changes.
pub fn fit_audio_clip(clip: &mut Clip, samples: &SampleCache, bpm: f64, b: u32) -> bool {
    let Some(slot) = clip.file.sample.as_deref() else {
        return false;
    };
    let Some(sample) = samples.pad_sample(slot) else {
        return false;
    };
    let beats = source_beats(
        sample.frames(),
        sample.sample_rate(),
        slot.params[SPEED],
        bpm,
    );
    if !(beats.is_finite() && beats > 0.0) {
        return false;
    }
    let before = (clip.file.source_len_beats, clip.loop_len_beats);
    clip.file.source_len_beats = beats;
    clip.fit_loop(b);
    before != (clip.file.source_len_beats, clip.loop_len_beats)
}

/// The actions of `channel`'s audio clips, pad by pad as [`build`] numbers them:
/// - at the start, a hit from source time offset·60/BPM (vel 1, centred; Speed does not scale it);
/// - at start − offset + n·L (n ≥ 1) inside [start, end), a fade then a hit from source time 0;
/// - at the end (start + visible length), a fade;
/// - each envelope step, repeated with the content loop inside the visible length.
pub(super) fn actions(
    song: &Song,
    channel: usize,
    sample_rate: f32,
    tempo_bpm: f32,
    actions: &mut Vec<ScheduledAction>,
) -> Result<(), EngineError> {
    let ppq = f64::from(PPQ);
    for (pad, (clip, _)) in clips(&song.channels[channel]).enumerate() {
        let mut push = |beat: f64, action| -> Result<(), EngineError> {
            let frame = beat_frame(beat, sample_rate, tempo_bpm)?;
            actions.push(ScheduledAction {
                frame,
                channel,
                action,
            });
            Ok(())
        };
        let (start, lp, offset) = (
            f64::from(clip.start_tick) / ppq,
            clip.loop_len_beats,
            clip.offset_beats,
        );
        let end = start + clip.visible_ticks() / ppq;
        push(
            start,
            Action::ClipHit {
                pad,
                offset: offset * 60.0 / f64::from(tempo_bpm),
            },
        )?;
        // At most one repetition a loop pass; `plays` bounded the passes.
        let (first, passes) = ((offset / lp).ceil().max(1.0), (end - start) / lp + 1.0);
        for n in 0..passes as u64 {
            let at = start - offset + (first + n as f64) * lp;
            if at >= end {
                break;
            }
            if at >= start {
                push(at, Action::ClipFade { pad })?;
                push(at, Action::ClipHit { pad, offset: 0.0 })?;
            }
        }
        push(end, Action::ClipFade { pad })?;
        for (index, env_param) in ENVELOPES.into_iter().enumerate() {
            let steps = envelope(clip, env_param)?;
            if let Some(Event::Automation { value, .. }) = steps.events.iter()
                .rfind(|e| f64::from(e.pos()) <= offset.rem_euclid(lp) * ppq) {
                push(start, Action::ClipParam { pad, index, value: *value })?;
            }
            for (tick, event) in steps.events_in_song_range(0, u64::MAX) {
                if let Event::Automation { value, .. } = *event {
                    let action = Action::ClipParam { pad, index, value };
                    push(tick as f64 / ppq, action)?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod volume_envelope_tests {
    use super::*;
    use crate::song::points::{self, Point, PointKey, Shape, ShapeKind};

    #[test]
    fn volume_envelope_playback_shapes_are_per_clip_not_shared_with_pan_or_overlaps() {
        let mut song = Song::default();
        song.add_channel(crate::song::ChannelKind::AudioClip);
        let clip = song.channels[1].lanes[0].add_clip(0, 4.0);
        let linear = vec![Point { tick: 0, value: 0, ..Default::default() },
            Point { tick: 128, value: 65535, ..Default::default() }];
        let mut smooth = linear.clone();
        smooth[0].shape = Shape { kind: ShapeKind::Smooth, tension: 80 };
        points::set_points(clip, PointKey::Envelope(0), smooth);
        points::set_points(clip, PointKey::Envelope(1), linear.clone());
        let a = envelope(clip, 0).unwrap().events;
        let pan = envelope(clip, 1).unwrap().events;
        let mut overlapping = clip.clone();
        points::set_points(&mut overlapping, PointKey::Envelope(0), linear);
        let b = envelope(&overlapping, 0).unwrap().events;
        assert_ne!(a, b);
        assert_eq!(pan, b, "VOL shapes must not change PAN");
        assert_eq!(envelope(clip, 0).unwrap().events, a, "editing another clip must not alter this clip");
    }
}

#[cfg(test)]
mod clip_envelope_audio_tests {
    use super::*;
    use crate::audio_engine::SongAudioEngine;
    use crate::song::points::{self, Point, PointKey, Shape, ShapeKind};
    use crate::flm::chunk::ContentPath;
    use banaforge_audio::device::AudioRenderer;

    fn fixture() -> (Song, SampleCache) {
        let mut song = Song { tempo_bpm:120.0, ..Default::default() };
        song.add_channel(crate::song::ChannelKind::AudioClip);
        let mut slot = SamplerSlot::default();
        slot.file.path = Some(ContentPath::Plain("clip-envelope-noise.wav".into()));
        let clip = song.channels[1].lanes[0].add_clip(0,4.0);
        clip.file.sample = Some(Box::new(slot));
        let mut x = 1u32;
        let data:Vec<i16> = (0..100000).map(|_| { x=x.wrapping_mul(1664525).wrapping_add(1013904223); ((x>>20) as i16)-2048 }).collect();
        let wav = crate::sample_cache::tests::wav(&data);
        let mut cache = SampleCache::default(); cache.load(&song,&|_| Some(wav.clone()));
        (song,cache)
    }
    fn curve(song:&mut Song, index:usize, points:Vec<Point>) {
        points::set_points(&mut song.channels[1].lanes[0].clips[0],PointKey::Envelope(index as i16),points);
    }
    fn constant(song:&mut Song,index:usize,value:u16) { curve(song,index,vec![Point { tick:0,value,..Default::default() }]); }
    fn engine(song:&Song, cache:&SampleCache) -> SongAudioEngine {
        let mut e=SongAudioEngine::with_samples(song,44100.0,cache).unwrap(); e.play();e
    }
    fn render(e:&mut SongAudioEngine, n:usize) -> Vec<f32> {
        let mut result=vec![0.0;n*2];
        for chunk in result.chunks_mut(254) { e.render(chunk,2); }
        result
    }
    fn energy(a:&[f32],channel:usize)->f32 { a.iter().skip(channel).step_by(2).map(|v|v*v).sum() }

    #[test]
    fn clip_envelope_pan_left_center_right_reaches_real_pcm() {
        let (song,cache)=fixture();
        for (pan,silent) in [(0,Some(1)),(32768,None),(65535,Some(0))] {
            let mut s=song.clone();constant(&mut s,1,pan);
            let audio=render(&mut engine(&s,&cache),8000);
            if let Some(c)=silent { assert!(energy(&audio,c)<1e-12);assert!(energy(&audio,1-c)>0.01); }
            else { assert!((energy(&audio,0)-energy(&audio,1)).abs()<0.001); }
        }
    }
    #[test]
    fn clip_envelope_gradual_and_instant_gain_pan_share_playback_events() {
        let (song,cache)=fixture();
        for kind in [ShapeKind::Linear,ShapeKind::Smooth,ShapeKind::Square,ShapeKind::Stairs] {
            let mut changed=song.clone();
            let pts=vec![Point { tick:0,value:0,shape:Shape { kind,tension:50 } },Point { tick:128,value:65535,..Default::default() }];
            curve(&mut changed,0,pts.clone());curve(&mut changed,1,pts);
            let mut heard=engine(&changed,&cache);
            let mut reference=engine(&song,&cache);
            // Compare 0.5 s of actual output with base PCM and the prepared slot's gain/balance at each block.
            for i in 0..100 {
                let frame=i*220;
                let a=render(&mut heard,220);let b=render(&mut reference,220);
                assert!(a.iter().all(|v|v.is_finite()));
                if kind==ShapeKind::Square && frame<10000 { assert!(energy(&a,0)+energy(&a,1)<1e-12); }
                if kind==ShapeKind::Linear && i>20 { assert!(energy(&a,0)+energy(&a,1)>0.0); }
                assert!(b.iter().any(|v|*v!=0.0));
            }
            render(&mut heard,50);
            let a=render(&mut heard,512);
            assert!(energy(&a,0)<1e-12, "full right at final point, {kind:?}");
            assert!(energy(&a,1)>0.0);
        }
    }
    #[test]
    fn clip_envelope_live_handover_keeps_sample_phase_and_applies_new_gain_pan_without_relaunch() {
        let (song,cache)=fixture();
        let mut old=engine(&song,&cache);let mut reference=engine(&song,&cache);
        render(&mut old,10000);render(&mut reference,10000);
        let mut changed=song.clone();constant(&mut changed,0,40000);constant(&mut changed,1,65535);
        let mut new=SongAudioEngine::with_samples(&changed,44100.0,&cache).unwrap().adopting_edit(false,Vec::new());
        new.take_over(&mut old);
        assert_eq!(new.clips[1].as_ref().unwrap().sampler.active_voices(0),1);
        let a=render(&mut new,2000);let b=render(&mut reference,2000);
        let gain=banaforge_audio::drums::level_gain(points::unit(40000))/banaforge_audio::drums::level_gain(DEFAULTS[0]);
        assert!(energy(&a,0)<1e-12);
        for (i,(a,b)) in a.iter().skip(1).step_by(2).zip(b.iter().skip(1).step_by(2)).enumerate() {
            assert!((*a-*b*gain).abs()<2e-5,"phase/gain changed at {i}: {a} vs {}",*b*gain);
        }
    }
    #[test]
    fn clip_envelope_overlaps_sum_independently_and_roundtrip_pan_shapes() {
        let (mut a,cache)=fixture();constant(&mut a,1,0);
        let mut b=a.clone();constant(&mut b,1,65535);constant(&mut b,0,40000);
        let mut together=a.clone();let mut second=b.channels[1].lanes[0].clips[0].clone();second.id+=1;
        together.channels[1].lanes[0].clips.push(second);
        let ab=render(&mut engine(&together,&cache),5000);let aa=render(&mut engine(&a,&cache),5000);let bb=render(&mut engine(&b,&cache),5000);
        for ((a,b),c) in aa.iter().zip(bb).zip(ab) { assert!((a+b-c).abs()<2e-5); }
        let points=vec![Point {tick:0,value:0,shape:Shape {kind:ShapeKind::Square,tension:20}},Point {tick:256,value:65535,..Default::default()}];
        curve(&mut together,1,points.clone());
        let saved=crate::flm::write(&together).unwrap();let loaded=crate::flm::read(&saved).unwrap();
        assert_eq!(points::points(&loaded.channels[1].lanes[0].clips[0],PointKey::Envelope(1)),points);
        let loaded_audio=render(&mut engine(&loaded,&cache),7000);let expected=render(&mut engine(&together,&cache),7000);
        for (a,b) in loaded_audio.iter().zip(expected) {assert!((a-b).abs()<2e-5);}
    }
}

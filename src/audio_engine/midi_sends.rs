//! MIDI sends (Honeydew's own, [`MidiSend`](crate::song::MidiSend)): every note a channel's instruments get, past all
//! its note effects, also goes to each target channel's note input, and on through the target's own note effects to its
//! instruments; the channel's MIDI CCs and Pitch Bend set the targets' too. Sequenced notes and automated CCs are
//! forwarded as the song is prepared ([`forwarded`], [`forward_params`]), live keys as the live chain sends them
//! ([`SongAudioEngine::forward_live`]) and live CCs as they are set ([`SongAudioEngine::forward_param`]). Every note
//! reaches its targets before any rack renders the segment, so the mixer order and the parallel stages need not know
//! of them.

use honeydew_audio::device::AudioRenderer;
use honeydew_audio::note_effects::LiveNoteEvent;
use honeydew_audio::sequencer::{InputNoteEvent, sort_note_events};

use super::prepare::{Action, ScheduledAction, note_effect, note_rack};
use super::routing::reaches;
use super::{EngineError, PITCH_BEND, SongAudioEngine, midi_cc, set_key};
use crate::audio_bridge::SongAudioBridge;
use crate::song::Song;

pub(super) struct MidiSends {
    /// Per channel, the channels its MIDI sends go to.
    pub targets: Vec<Vec<usize>>,
    /// Every channel, each before the targets of its MIDI sends: the order the live notes go out in.
    pub order: Vec<usize>,
}

/// `song`'s MIDI sends by channel index. A send is left out when it is off, names no channel, the master, the channel
/// itself or a target named already, or would close a cycle: the app never makes one, and a file's is broken at the
/// send that closes it, scanning the channels in order.
pub(super) fn build(song: &Song) -> MidiSends {
    let count = song.channels.len();
    let mut targets: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (index, channel) in song.channels.iter().enumerate().skip(1) {
        for send in channel.file.midi_sends.iter().filter(|send| send.enabled) {
            let target = (song.channels.iter()).position(|c| c.id == send.target_channel_id);
            if let Some(target) = target.filter(|&t| t != 0 && !targets[index].contains(&t))
                && !reaches(&targets, target, index)
            {
                targets[index].push(target);
            }
        }
    }
    // Kahn's order: sources before their targets.
    let mut sources = vec![0; count];
    targets.iter().flatten().for_each(|&t| sources[t] += 1);
    let mut order: Vec<usize> = (0..count).filter(|&c| sources[c] == 0).collect();
    let mut next = 0;
    while let Some(&channel) = order.get(next) {
        next += 1;
        for &target in &targets[channel] {
            sources[target] -= 1;
            if sources[target] == 0 {
                order.push(target);
            }
        }
    }
    MidiSends { targets, order }
}

impl MidiSends {
    /// The channels `channel`'s MIDI sends reach, through other channels' too.
    fn reach(&self, channel: usize) -> Vec<usize> {
        let mut reached = Vec::new();
        let mut stack = self.targets[channel].clone();
        while let Some(target) = stack.pop() {
            if !reached.contains(&target) {
                reached.push(target);
                stack.extend(&self.targets[target]);
            }
        }
        reached
    }
}

/// A channel param a MIDI send forwards: a MIDI CC (7–133) or Pitch Bend.
fn forwards(index: i32) -> bool {
    midi_cc(0, index) || index == PITCH_BEND as i32
}

/// What each channel's MIDI sources send it over the song (to `end_beat`): their input, their Notes lanes' notes and
/// what their own sources send, past all their note effects (as `Burn Notes to Track` takes them), sorted as playback
/// sorts them.
pub(super) fn forwarded(
    song: &Song,
    sends: &MidiSends,
    end_beat: f64,
    sample_rate: f32,
) -> Result<Vec<Vec<InputNoteEvent>>, EngineError> {
    let mut forwarded = vec![Vec::new(); song.channels.len()];
    for &channel in &sends.order {
        if sends.targets[channel].is_empty() {
            continue;
        }
        let input = input(song, channel, end_beat, &forwarded[channel])?;
        let modules = song.channels[channel].rack.modules.iter();
        let effects: Vec<_> = (modules.filter(|module| (47..=51).contains(&module.type_id)))
            .map(|module| note_effect(song, channel, module, end_beat))
            .collect::<Result<_, _>>()?;
        let tempo_bpm = f64::from(song.tempo_bpm as f32);
        let sent = (note_rack(song, effects, sample_rate)?)
            .process_events(0.0, end_beat, tempo_bpm, &input)
            .map_err(EngineError::Notes)?;
        for &target in &sends.targets[channel] {
            forwarded[target].extend_from_slice(&sent);
            sort_note_events(&mut forwarded[target]);
        }
    }
    Ok(forwarded)
}

/// `channel`'s notes at its input over the song (to `end_beat`): its Notes lanes', then the notes its MIDI sources
/// send it (`forwarded`), sorted as playback sorts them.
pub(super) fn input(
    song: &Song,
    channel: usize,
    end_beat: f64,
    forwarded: &[InputNoteEvent],
) -> Result<Vec<InputNoteEvent>, EngineError> {
    let mut input = SongAudioBridge::note_events_in_block(song, channel, 0.0, end_beat)
        .map_err(EngineError::Song)?;
    if !forwarded.is_empty() {
        input.extend_from_slice(forwarded);
        sort_note_events(&mut input);
    }
    Ok(input)
}

/// Each automated MIDI CC and Pitch Bend change sets the same param of every channel the channel's MIDI sends reach, at
/// the same frame.
pub(super) fn forward_params(actions: &mut Vec<ScheduledAction>, sends: &MidiSends) {
    let reach: Vec<_> = (0..sends.targets.len()).map(|c| sends.reach(c)).collect();
    let mut forwarded = Vec::new();
    for event in actions.iter() {
        if let Action::Param {
            instance_id: 0,
            index,
            ..
        } = event.action
            && forwards(index)
        {
            let copies = reach[event.channel]
                .iter()
                .map(|&channel| ScheduledAction { channel, ..*event });
            forwarded.extend(copies);
        }
    }
    actions.extend(forwarded);
}

impl SongAudioEngine {
    /// A MIDI CC or Pitch Bend set on `channel` live sets the targets of its MIDI sends too, as the song's value
    /// (`saved`, [`AudioRenderer::set_param`]) or as MIDI's own ([`AudioRenderer::set_live_param`]). Allocates nothing.
    pub(super) fn forward_param(&mut self, channel: usize, index: i32, value: f32, saved: bool) {
        if !forwards(index) {
            return;
        }
        let count = self.midi_sends.targets.get(channel).map_or(0, Vec::len);
        for k in 0..count {
            let target = self.midi_sends.targets[channel][k];
            match saved {
                true => self.set_param(target, 0, index, value),
                false => self.set_live_param(target, index, value),
            }
        }
    }

    /// A live note `channel` sent past all its note effects, at frame `now`, into the live keys of its MIDI sends'
    /// targets, which send it on through their own note effects in the same pass ([`MidiSends::order`]). A note-off
    /// ends a key the target holds; one it does not (a Stop forgot it) is dropped. Allocates nothing.
    pub(super) fn forward_live(&mut self, channel: usize, event: LiveNoteEvent, now: u64) {
        for k in 0..self.midi_sends.targets[channel].len() {
            let target = self.midi_sends.targets[channel][k];
            let Some(live) = &mut self.live[target] else {
                continue;
            };
            let bit = 1_u128.checked_shl(u32::from(event.key)).unwrap_or(0);
            match event.on {
                true => (live.chain).note_on(now, event.key, event.velocity, live.notes_lane),
                false if live.chain.keys_down() & bit != 0 => live.chain.note_off(now, event.key),
                false => continue,
            }
            set_key(&mut self.sounding[target], event.key, event.on);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{AutomationTarget, ChannelKind, Event, MidiSend, PPQ, SynthKind};

    /// One beat at 120 BPM and 48 kHz.
    const BEAT: usize = 24_000;

    /// At 120 BPM, MiniSynths A (channel 1), B (2) and C (3), each its synth instance 2; A sends MIDI to B, B to C. B
    /// hears key 72 alone, below a Transpose +12: only a note its own note effect moves there sounds on it. A plays 60
    /// from beat 0 for 2 beats, and its CC #1 (channel param 8) goes to 1 at beat 1.
    fn chain() -> Song {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        for _ in 0..3 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        for (from, to) in [(1, 2), (2, 3)] {
            let target_channel_id = song.channels[to].id;
            (song.channels[from].file.midi_sends).push(MidiSend {
                target_channel_id,
                enabled: true,
            });
        }
        let b = &mut song.channels[2].rack;
        let transpose = b.insert(0, 48);
        b.get_mut(transpose).unwrap().params[1] = 5.0 / 8.0;
        let synth = &mut b.get_mut(2).unwrap().file;
        (synth.key_low, synth.key_high) = (72, 72);
        let a = &mut song.channels[1];
        a.lanes[0].add_clip(0, 4.0).events.push(Event::Note {
            pos: 0,
            len_beats: 2.0,
            key: 60,
            velocity: 200,
            pan: 127,
            slide: false,
        });
        let cc = AutomationTarget {
            instance_id: 0,
            param: 8,
        };
        (a.add_automation_lane(cc).add_clip(PPQ, 1.0).events).push(Event::Automation {
            pos: 0,
            value: 1.0,
            control: true,
        });
        song
    }

    fn render(engine: &mut SongAudioEngine, frames: usize) {
        engine.render(&mut vec![0.0; frames * 2], 2);
    }

    /// The peak of channel `channel`'s synth (instance 2) over the last render.
    fn synth_peak(engine: &SongAudioEngine, channel: usize) -> f32 {
        let at = (0..engine.meter_count()).find(|&i| {
            let key = engine.meter_key(i);
            (key.channel, key.instance_id) == (channel, 2)
        });
        engine.meter(at.unwrap()).peak_left
    }

    /// A's sequenced note reaches B through B's Transpose (as 72, the one key B hears) and goes on to C as 72; A's CC
    /// automation moves B's and C's CC #1. Switched off, the send forwards nothing.
    #[test]
    fn a_sequenced_note_and_cc_go_through_the_targets_note_effects_down_the_chain() {
        let mut engine = SongAudioEngine::new(&chain(), 48_000.0).unwrap();
        engine.play();
        render(&mut engine, BEAT / 2);
        assert!(synth_peak(&engine, 2) > 0.01, "B sounds");
        let keys = [1, 2, 3].map(|c| engine.sounding(c));
        assert_eq!(keys, [1 << 60, 1 << 60, 1 << 72]);
        render(&mut engine, BEAT);
        let cc = |engine: &SongAudioEngine| [1, 2, 3].map(|c| engine.audio.channel_parameter(c, 8));
        assert_eq!(cc(&engine), [Some(1.0); 3]);

        let mut song = chain();
        song.channels[1].file.midi_sends[0].enabled = false;
        let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
        engine.play();
        render(&mut engine, BEAT * 3 / 2);
        assert_eq!(synth_peak(&engine, 2), 0.0);
        assert_eq!([2, 3].map(|c| engine.sounding(c)), [0, 0]);
        assert_eq!(cc(&engine)[1..], [Some(0.0); 2]);
    }

    /// A live key on A reaches B (and C) until it lifts; a live CC and MIDI's Pitch Bend on A set B's too.
    #[test]
    fn a_live_key_and_cc_on_the_source_reach_its_targets() {
        let mut engine = SongAudioEngine::new(&chain(), 48_000.0).unwrap();
        engine.note_on(1, 60, 1.0);
        render(&mut engine, 4096);
        assert!(synth_peak(&engine, 2) > 0.01, "B sounds");
        assert_eq!([2, 3].map(|c| engine.sounding(c)), [1 << 60, 1 << 72]);
        engine.note_off(1, 60);
        render(&mut engine, 64);
        assert_eq!([2, 3].map(|c| engine.sounding(c)), [0, 0]);
        render(&mut engine, 48_000);
        render(&mut engine, 1024);
        assert_eq!(synth_peak(&engine, 2), 0.0, "B released");
        engine.set_param(1, 0, 9, 0.7);
        engine.set_live_param(1, 4, 0.9);
        let param = |index| engine.audio.channel_parameter(3, index);
        assert_eq!([param(9), param(4)], [Some(0.7), Some(0.9)]);
    }

    /// A send that would close a cycle (C back to A) is dropped; the chain plays on.
    #[test]
    fn a_send_closing_a_cycle_is_dropped() {
        let mut song = chain();
        let target_channel_id = song.channels[1].id;
        (song.channels[3].file.midi_sends).push(MidiSend {
            target_channel_id,
            enabled: true,
        });
        let sends = build(&song);
        assert_eq!(sends.targets, [vec![], vec![2], vec![3], vec![]]);
        assert_eq!(sends.order, [0, 1, 2, 3]);
        let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
        engine.play();
        render(&mut engine, BEAT / 2);
        assert_eq!(engine.sounding(1), 1 << 60);
    }

    /// Stop in the middle of a forwarded note releases it: nothing held, nothing lit, B silent once its release ends.
    #[test]
    fn stop_mid_note_leaves_no_forwarded_note_stuck() {
        let mut engine = SongAudioEngine::new(&chain(), 48_000.0).unwrap();
        engine.play();
        render(&mut engine, BEAT / 2);
        assert!(synth_peak(&engine, 2) > 0.01);
        engine.stop();
        render(&mut engine, 48_000);
        render(&mut engine, 1024);
        assert_eq!(synth_peak(&engine, 2), 0.0, "B released");
        assert_eq!([2, 3].map(|c| engine.sounding(c)), [0, 0]);
        assert!(!engine.holds_any());
    }
}

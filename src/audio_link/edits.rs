//! Which re-sync a song edit runs. Every edit hands over an engine that takes over the playing one's sound, channel by
//! channel and module by module (`SongAudioEngine::adopting_edit`), so the sound goes on as if the engine were never
//! rebuilt, and the difference of the songs says whether it re-syncs and which channels get all notes off first.

use crate::song::{Channel, Clip, Event, Lane, LaneKind, Song};

/// How the engine for an edited song takes over the playing one.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Edit {
    /// A position re-sync follows.
    pub resync: bool,
    /// The channels (of the edited song) that first get all notes off: a deleted lane's.
    pub silence: Vec<usize>,
}

/// The edit from `last`, the song the playing engine was built from, to `new`; `tempo_moved`: a tempo drag moved the
/// engine's tempo live. Channels match by id.
///
/// Re-sync (derived from the songs): a channel deleted, any change of a channel's lanes but a change of the notes' and
/// hits' graph-lane values and an empty lane added, and a tempo change with a sampled audio clip (without one it
/// changes nothing). A deleted lane silences its channel. Nothing else re-syncs: a channel added, a module added,
/// removed, moved or loaded, a send, a drum slot's settings, sample, Shift or send target, a drum row added or removed
/// (its hits leave with it and the later rows move up, which is no lane change here). The songs cannot tell every
/// gesture apart, so the editors report the rest: the additions that run no re-sync (a tap-create, a tap on an empty
/// step, Clear and Fill) as [`super::Handover::Quiet`], and the edits that re-sync whatever they changed (Quantize,
/// Humanize, Snap to scale, a graph-lane touch that changed nothing) as [`super::Handover::Undo`]. A MIDI send added,
/// removed or switched (or gone with its channel) re-syncs too, and silences every channel a MIDI send played into, so
/// that no live note it forwarded is left without its note-off. `last` is scratch.
pub(super) fn edit(last: &mut Song, new: &Song, tempo_moved: bool) -> Edit {
    let tempo = tempo_moved || last.tempo_bpm != new.tempo_bpm;
    let deleted: Vec<i32> = (last.channels.iter().map(|c| c.id))
        .filter(|id| !new.channels.iter().any(|c| c.id == *id))
        .collect();
    let mut edit = Edit {
        // A tempo change re-renders the audio clips' samples and re-syncs; without any it does nothing.
        resync: (tempo && sampled(new)) || !deleted.is_empty(),
        silence: Vec::new(),
    };
    fn midi_sends(song: &Song, id: i32) -> &[crate::song::MidiSend] {
        let channel = song.channels.iter().find(|c| c.id == id);
        channel.map_or(&[], |c| &c.file.midi_sends)
    }
    if (last.channels.iter()).any(|a| midi_sends(new, a.id) != midi_sends(last, a.id)) {
        edit.resync = true;
        let index = |id| new.channels.iter().position(|c| c.id == id);
        let sends = last.channels.iter().flat_map(|a| &a.file.midi_sends);
        let targets = sends
            .filter(|s| s.enabled)
            .filter_map(|s| index(s.target_channel_id));
        for target in targets {
            if !edit.silence.contains(&target) {
                edit.silence.push(target);
            }
        }
    }
    for a in &mut last.channels {
        if let Some(b) = new.channels.iter().find(|b| b.id == a.id) {
            renumber(a, b, &deleted);
        }
    }
    last.keep_current_clips(new);
    for (index, b) in new.channels.iter().enumerate() {
        let Some(a) = last.channels.iter().find(|a| a.id == b.id) else {
            continue;
        };
        if a.lanes == b.lanes {
            continue;
        }
        match a.lanes.len().cmp(&b.lanes.len()) {
            // Only empty lanes added among the old ones: nothing to do. A lane with clips (a recorded take's)
            // re-syncs, which brings the new clips' audio slots in.
            std::cmp::Ordering::Less => edit.resync |= !only_empty_added(&a.lanes, &b.lanes),
            std::cmp::Ordering::Greater => {
                edit.resync = true;
                edit.silence.push(index);
            }
            std::cmp::Ordering::Equal => {
                let kept = |(a, b): (&Lane, &Lane)| a == b || retouched(a, b);
                edit.resync |= !a.lanes.iter().zip(&b.lanes).all(kept);
            }
        }
    }
    edit
}

/// `a`'s lanes as `b` (the same channel after the edit) holds them when nothing but its structure changed: without the
/// Enable Send and Send Level lanes for a deleted channel, which `Song::delete_channel` removes with it, and with each
/// drum hit on its slot's row in `b`'s kit (by slot id): a removed row's hits gone, the later rows moved.
fn renumber(a: &mut Channel, b: &Channel, deleted: &[i32]) {
    let sends = |param: i32| {
        deleted
            .iter()
            .any(|id| param == 0x1000 + id || param == 0x2000 + id)
    };
    a.lanes.retain(|lane| {
        lane.target
            .is_none_or(|t| t.instance_id != 0 || !sends(t.param))
    });
    let ids = |channel: &Channel| -> Vec<u32> {
        let slots = channel.drum_kit.iter().flat_map(|kit| &kit.slots);
        slots.map(|slot| slot.file.id).collect()
    };
    let (from, to) = (ids(a), ids(b));
    // A kit replaced whole (a drumset loaded: slot ids are never reused) keeps every hit on its row.
    let replaced = !to.is_empty() && !from.iter().any(|id| to.contains(id));
    if from == to || replaced {
        return;
    }
    let row = |pad: u8| {
        let id = from.get(usize::from(pad))?;
        u8::try_from(to.iter().position(|other| other == id)?).ok()
    };
    for clip in a.lanes.iter_mut().flat_map(|lane| &mut lane.clips) {
        clip.events.retain_mut(|event| match event {
            Event::Drum { pad, .. } => row(*pad).map(|row| *pad = row).is_some(),
            _ => true,
        });
    }
}

/// `new` is `old` with empty lanes inserted, the others unchanged and in order.
fn only_empty_added(old: &[Lane], new: &[Lane]) -> bool {
    let mut old = old.iter().peekable();
    new.iter().all(|lane| match old.peek() {
        Some(&kept) if kept == lane => old.next().is_some(),
        _ => lane.clips.is_empty(),
    }) && old.next().is_none()
}

/// Some audio lane's clip has a sample.
fn sampled(song: &Song) -> bool {
    let lanes = song.channels.iter().flat_map(|c| &c.lanes);
    let audio = lanes.filter(|lane| lane.kind == LaneKind::Audio);
    audio
        .flat_map(|lane| &lane.clips)
        .any(|clip| clip.file.sample.is_some())
}

/// `b` is notes or drum lane `a` with only the notes' and hits' graph-lane values (velocity, pan, pitch, repeat)
/// changed: its clips in place, each with `a`'s events but for those values. A clip's loop may differ: the clip-update
/// rule refits it to the same events (a drum row removed), which is no edit of them.
fn retouched(a: &Lane, b: &Lane) -> bool {
    let clip = |(a, b): (&Clip, &Clip)| {
        (a.id, a.start_tick, a.muted) == (b.id, b.start_tick, b.muted)
            && (a.visible_len_beats, a.offset_beats) == (b.visible_len_beats, b.offset_beats)
            && a.events.len() == b.events.len()
            && kept(&a.events, &b.events)
    };
    matches!(a.kind, LaneKind::Notes | LaneKind::Drums)
        && (a.kind, a.target, a.grid, a.swing) == (b.kind, b.target, b.grid, b.swing)
        && a.clips.len() == b.clips.len()
        && a.clips.iter().zip(&b.clips).all(clip)
}

/// Each event of `old` has its own in `new`, alike but maybe for its graph-lane values. Both are sorted by position.
fn kept(old: &[Event], new: &[Event]) -> bool {
    let bare = |event: &Event| match *event {
        Event::Note {
            pos,
            len_beats,
            key,
            slide,
            ..
        } => Event::Note {
            pos,
            len_beats,
            key,
            slide,
            velocity: 0,
            pan: 0,
        },
        Event::Drum {
            pos,
            len_beats,
            pad,
            ..
        } => Event::Drum {
            pos,
            len_beats,
            pad,
            velocity: 0,
            pan: 0,
            value: 0,
            repeat: 0,
        },
        other => other,
    };
    let mut used = vec![false; new.len()];
    let mut from = 0;
    old.iter().all(|event| {
        while new.get(from).is_some_and(|n| n.pos() < event.pos()) {
            from += 1;
        }
        let same = (from..new.len())
            .take_while(|&i| new[i].pos() == event.pos())
            .find(|&i| !used[i] && bare(&new[i]) == bare(event));
        same.map(|i| used[i] = true).is_some()
    })
}

#[cfg(test)]
mod tests {
    //! Song edits while playing through the link: the recorder's engine plays them as the device would, and each output
    //! is checked against an engine that never saw the edit.

    use honeydew_audio::device::{AudioCommand, AudioRenderer};

    use super::super::testing::Recorder;
    use super::super::{AudioLink, Handover};
    use crate::audio_engine::SongAudioEngine;
    use crate::flm::chunk::ContentPath;
    use crate::sample_cache::{SampleCache, tests::wav};
    use crate::song::{
        AutomationTarget, ChannelKind, Event, LaneKind, ModuleInstance, PPQ, SamplerSlot, Song,
        SynthKind,
    };

    /// One beat at 120 BPM and 48 kHz.
    const BEAT: f64 = 24_000.0;
    /// Tape Delay.
    const DELAY: u16 = 18;

    /// Every sample file: 0.5 for eight seconds.
    fn read(path: &ContentPath) -> Option<Vec<u8>> {
        matches!(path, ContentPath::Plain(_)).then(|| wav(&vec![16_384; 8 * 44_100]))
    }

    fn note(beat: f64, len_beats: f64, key: u8) -> Event {
        Event::Note {
            pos: (beat * f64::from(PPQ)) as u32,
            len_beats,
            key,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        }
    }

    fn hit(beat: f64) -> Event {
        Event::Drum {
            pos: (beat * f64::from(PPQ)) as u32,
            len_beats: 0.0,
            pad: 0,
            velocity: 255,
            pan: 127,
            value: Event::DEFAULT_PITCH,
            repeat: 0,
        }
    }

    /// 120 BPM, a MiniSynth on channel 1 playing `notes` from one 16-beat clip.
    fn synth(notes: &[Event]) -> Song {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let clip = song.channels[1].lanes[0].add_clip(0, 16.0);
        clip.events.extend_from_slice(notes);
        song
    }

    /// 120 BPM, Drums on channel 1 whose first pad plays the eight-second sample, and two step clips on its lane: 4
    /// beats from beat 0 with hits at `hits` (beats), and an empty one from beat 8.
    fn drums(hits: &[f64]) -> Song {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Drums);
        let kit = song.channels[1].drum_kit.as_mut().unwrap();
        kit.slots[0].file.path = Some(ContentPath::Plain("dc".into()));
        let lane = &mut song.channels[1].lanes[0];
        assert_eq!(lane.kind, LaneKind::Drums);
        lane.add_clip(0, 4.0)
            .events
            .extend(hits.iter().map(|&beat| hit(beat)));
        lane.add_clip(8 * PPQ, 4.0);
        song
    }

    /// The left side of `beats` beats of `engine`, in 777-frame calls.
    fn render(engine: &mut SongAudioEngine, beats: f64) -> Vec<f32> {
        let frames = (beats * BEAT) as usize;
        let mut left = Vec::with_capacity(frames);
        let mut output = vec![0.0; 777 * 2];
        while left.len() < frames {
            let n = 777.min(frames - left.len());
            engine.render(&mut output[..n * 2], 2);
            left.extend(output[..n * 2].iter().step_by(2));
        }
        left
    }

    /// The recorder's engine: what the device plays.
    fn rendered(rec: &Recorder, beats: f64) -> Vec<f32> {
        render(rec.0.borrow_mut().engine.as_mut().unwrap(), beats)
    }

    fn tick(rec: &Recorder) -> u64 {
        rec.0.borrow().engine.as_ref().unwrap().transport_tick()
    }

    fn sounding(rec: &Recorder, channel: usize) -> u128 {
        rec.0.borrow().engine.as_ref().unwrap().sounding(channel)
    }

    /// A link on `rec` playing `song` from beat 0, its log empty.
    fn playing(rec: &Recorder, song: &Song) -> AudioLink {
        let mut link = AudioLink::default();
        link.start(&Song::default(), false, &read, |_, _| rec.sink());
        assert!(link.replace_song(song, false, 0, &read));
        link.send(AudioCommand::Play);
        rec.take();
        link
    }

    /// `song` played by an engine that sees no edit: `beats` beats from beat `from`.
    fn reference(song: &Song, from: f64, beats: f64) -> Vec<f32> {
        let mut samples = SampleCache::default();
        samples.load(song, &read);
        let mut engine = SongAudioEngine::with_samples(song, 48_000.0, &samples).unwrap();
        engine.play();
        render(&mut engine, from);
        render(&mut engine, beats)
    }

    fn energy(samples: &[f32]) -> f32 {
        samples.iter().map(|sample| sample * sample).sum()
    }

    fn assert_close(actual: &[f32], expected: &[f32]) {
        assert!(energy(expected) > 0.01, "a silent reference");
        assert_eq!(actual.len(), expected.len());
        for (frame, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert!((a - e).abs() <= 1e-5, "frame {frame}: {a} vs {e}");
        }
    }

    /// A move re-syncs, but A3, which still covers the playhead, sounds on untouched, and the Reverb's tail of a note
    /// that ended rings on (effects are never reset).
    #[test]
    fn a_voice_and_a_reverb_tail_survive_a_note_move_while_playing() {
        let mut song = synth(&[note(0.0, 0.25, 60), note(0.0, 4.0, 57), note(8.0, 1.0, 64)]);
        song.channels[1].rack.add(32);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let mut moved = song.clone();
        moved.channels[1].lanes[0].clips[0].events[2] = note(10.0, 1.0, 64);
        assert!(link.replace_song(&moved, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        assert_eq!(sounding(&rec, 1), 1 << 57);
        assert_close(&rendered(&rec, 2.0), &reference(&song, 1.0, 2.0));
    }

    /// A sounding note deleted gets its note-off at once, a release as if it had ended there.
    #[test]
    fn a_sounding_note_deleted_while_playing_is_released_at_once() {
        let song = synth(&[note(0.0, 4.0, 57)]);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        assert!(link.replace_song(&synth(&[]), true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        assert_eq!(sounding(&rec, 1), 0);
        let ended = synth(&[note(0.0, 1.0, 57)]);
        assert_close(&rendered(&rec, 1.0), &reference(&ended, 1.0, 1.0));
    }

    /// A tap-created note does not re-sync (its editor reports [`Handover::Quiet`]); one under the playhead stays
    /// silent until the playhead next crosses its start (here after the loop wraps).
    #[test]
    fn a_note_created_under_the_playhead_stays_silent_until_its_next_pass() {
        let mut song = synth(&[note(0.0, 4.0, 57)]);
        song.play_loop = Some((0, 4 * PPQ));
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let mut added = song.clone();
        (added.channels[1].lanes[0].clips[0].events).push(note(0.5, 2.5, 64));
        let quiet = Handover::Quiet;
        assert!(link.replace_song_keeping(&added, &added, true, tick(&rec), &read, quiet));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
        assert_eq!(sounding(&rec, 1), 1 << 57);
        assert_close(&rendered(&rec, 2.0), &reference(&song, 1.0, 2.0));
        // Past the wrap at beat 4 and E4's start at 4.5.
        rendered(&rec, 1.75);
        assert_eq!(sounding(&rec, 1), 1 << 57 | 1 << 64);
    }

    /// A tap on an empty step does not re-sync ([`Handover::Quiet`]). Past its clip's end the pad of the last hit still
    /// rings, which a re-sync would release (no step covers it there): it rings on as if nothing happened.
    #[test]
    fn a_step_tapped_on_while_playing_does_not_re_sync() {
        let song = drums(&[3.75]);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 4.5);
        let mut tapped = song.clone();
        tapped.channels[1].lanes[0].clips[1].events.push(hit(1.0));
        let quiet = Handover::Quiet;
        assert!(link.replace_song_keeping(&tapped, &tapped, true, tick(&rec), &read, quiet));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
        assert_close(&rendered(&rec, 1.0), &reference(&song, 4.5, 1.0));
    }

    /// A Shift change: no re-sync, the sounding hit untouched; the shift moves later hits.
    #[test]
    fn a_shift_change_leaves_the_sounding_hits_untouched() {
        let song = drums(&[0.0, 2.0]);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let mut shifted = song.clone();
        let slot = &mut shifted.channels[1].drum_kit.as_mut().unwrap().slots[0];
        (slot.file.beat_offset, slot.file.beat_offset_control) = (0.25, 0.75);
        assert!(link.replace_song(&shifted, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
        assert_close(&rendered(&rec, 0.9), &reference(&song, 1.0, 0.9));
    }

    /// Deleting a lane gives its channel all notes off, then a re-sync: channel 1's covered A3 is released and not
    /// started again; channel 2's G4 sounds on.
    #[test]
    fn deleting_a_lane_gives_all_notes_off_on_its_channel_only() {
        let both = |a3_end: f64, lane: bool| {
            let mut song = synth(&[note(0.0, a3_end, 57)]);
            if lane {
                song.channels[1].add_lane(LaneKind::Notes);
            }
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
            let clip = song.channels[2].lanes[0].add_clip(0, 16.0);
            clip.events.push(note(0.0, 4.0, 67));
            song
        };
        let song = both(4.0, true);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        assert!(link.replace_song(&both(4.0, false), true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting silencing 1", "Play"]);
        assert_eq!((sounding(&rec, 1), sounding(&rec, 2)), (0, 1 << 67));
        assert_close(
            &rendered(&rec, 1.0),
            &reference(&both(1.0, false), 1.0, 1.0),
        );
    }

    /// Adding an effect hands over an engine that takes the playing one over without a re-sync; the delay tail of a
    /// note that ended rings on as if nothing happened.
    #[test]
    fn an_effect_added_takes_over_and_keeps_the_tails() {
        let mut song = synth(&[note(0.0, 0.25, 60)]);
        song.channels[1].rack.add(DELAY);
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let mut added = song.clone();
        added.channels[2].rack.add(32);
        assert!(link.replace_song(&added, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
        assert_close(&rendered(&rec, 1.0), &reference(&song, 1.0, 1.0));
    }

    /// Every structural edit takes over the playing engine; only deleting a channel or a MIDI send change re-syncs (the
    /// rest).
    #[test]
    fn structural_edits_take_over_and_only_a_deleted_channel_or_a_midi_send_re_syncs() {
        let mut song = drums(&[0.0]);
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        song.add_channel(ChannelKind::EffectTrack);
        fn kit(song: &mut Song) -> &mut crate::song::DrumKit {
            song.channels[1].drum_kit.as_mut().unwrap()
        }
        type Case = (&'static str, fn(&mut Song));
        let edits: [Case; 14] = [
            ("add a channel", |s| s.add_channel(ChannelKind::Drums)),
            ("delete a channel", |s| s.delete_channel(3)),
            ("add an effect", |s| _ = s.channels[2].rack.add(DELAY)),
            ("remove the instrument", |s| {
                _ = s.channels[2].rack.remove(2)
            }),
            ("switch its type", |s| {
                s.channels[2].rack.remove(2);
                s.channels[2].rack.add(23);
            }),
            ("load a preset", |s| {
                let mut preset = ModuleInstance::new(0, 9);
                preset.params[9] = 0.3;
                s.channels[2].rack.modules[0].load_preset(preset);
            }),
            ("change a send", |s| {
                s.channels[2].file.sends[0].target_channel_id = 3
            }),
            ("AUX", |s| kit(s).slots[0].file.send_target = 3),
            ("Stretch", |s| {
                (
                    kit(s).slots[0].file.stretch_ratio,
                    kit(s).slots[0].file.stretch_mode,
                ) = (2.0, 1)
            }),
            ("pick a pad sample", |s| {
                kit(s).slots[0].file.path = Some(ContentPath::Plain("other".into()))
            }),
            ("add a row", |s| {
                _ = kit(s).insert_slot(1, SamplerSlot::default())
            }),
            ("remove a row", |s| {
                kit(s).slots.remove(0);
                s.channels[1].lanes[0].clips[0].events.clear();
            }),
            ("reorder the channels", |s| {
                _ = s.set_playlist(&[0, 3, 1, 2])
            }),
            ("add a MIDI send", |s| {
                let target_channel_id = s.channels[1].id;
                (s.channels[2].file.midi_sends).push(crate::song::MidiSend {
                    target_channel_id,
                    enabled: true,
                })
            }),
        ];
        for (name, edit) in edits {
            let rec = Recorder::default();
            let mut link = playing(&rec, &song);
            rendered(&rec, 1.0);
            let mut edited = song.clone();
            edit(&mut edited);
            assert!(
                link.replace_song(&edited, true, tick(&rec), &read),
                "{name}"
            );
            let expected = match name {
                "delete a channel" | "add a MIDI send" => "ReplaceSong adopting",
                _ => "ReplaceSong adopting no-resync",
            };
            assert_eq!(rec.take(), [expected, "Play"], "{name}");
        }
    }

    /// Removing a row deletes its hits in every drum lane and moves the later rows up; that is no lane change, so
    /// nothing re-syncs, and the removed slot's voices stop at once while the others ring on.
    #[test]
    fn a_drum_row_removed_cuts_only_its_voices() {
        let hits = |pads: &[u8]| {
            let mut song = drums(&[]);
            let kit = song.channels[1].drum_kit.as_mut().unwrap();
            for slot in &mut kit.slots[..3] {
                slot.file.path = Some(ContentPath::Plain("dc".into()));
            }
            let events = &mut song.channels[1].lanes[0].clips[0].events;
            events.extend(pads.iter().map(|&pad| Event::Drum {
                pos: 0,
                len_beats: 0.0,
                pad,
                velocity: 255 - 60 * pad,
                pan: 127,
                value: Event::DEFAULT_PITCH,
                repeat: 0,
            }));
            song
        };
        let song = hits(&[0, 1, 2]);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let mut removed = song.clone();
        removed.channels[1]
            .drum_kit
            .as_mut()
            .unwrap()
            .slots
            .remove(1);
        let events = &mut removed.channels[1].lanes[0].clips[0].events;
        events.remove(1);
        if let Event::Drum { pad, .. } = &mut events[1] {
            *pad = 1;
        }
        assert!(link.replace_song(&removed, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
        assert_close(&rendered(&rec, 1.0), &reference(&hits(&[0, 2]), 1.0, 1.0));
    }

    /// A Stretch change: the pad's new render replaces its sample under the sounding voice, which reads it from the
    /// frame it is at; nothing re-syncs.
    #[test]
    fn a_stretch_swaps_the_pad_sample_under_its_sounding_voice() {
        let song = drums(&[0.0]);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let mut stretched = song.clone();
        let slot = &mut stretched.channels[1].drum_kit.as_mut().unwrap().slots[0];
        (slot.file.stretch_ratio, slot.file.stretch_mode) = (2.0, 1);
        assert!(link.replace_song(&stretched, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
        // The new render streams in (its first frames are the build's): its worker fills the very buffer the voice
        // plays, with no other engine.
        for job in link.samples.take_jobs() {
            job.render();
        }
        assert_close(&rendered(&rec, 1.0), &reference(&stretched, 1.0, 1.0));
    }

    /// Deleting a channel ends it at once; the others re-sync, their covered voices sounding on.
    #[test]
    fn deleting_a_channel_keeps_the_others_voices() {
        let mut song = synth(&[note(0.0, 8.0, 57)]);
        for key in [64, 67] {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
            let clip = song.channels.last_mut().unwrap().lanes[0].add_clip(0, 16.0);
            clip.events.push(note(0.0, 8.0, key));
        }
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let mut deleted = song.clone();
        deleted.delete_channel(2);
        assert!(link.replace_song(&deleted, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        assert_eq!((sounding(&rec, 1), sounding(&rec, 2)), (1 << 57, 1 << 67));
        let mut silent = song.clone();
        silent.channels[2].lanes[0].clips[0].events.clear();
        assert_close(&rendered(&rec, 1.0), &reference(&silent, 1.0, 1.0));
    }

    /// A knob (`SetParam`) and a drum strip REV (a live setting) leave the link's last song behind the app's; a note
    /// move after them still takes over the playing engine, which has both already.
    #[test]
    fn a_note_move_after_live_knob_and_rev_changes_still_adopts() {
        let mut song = synth(&[note(0.0, 4.0, 57), note(8.0, 1.0, 64)]);
        song.add_channel(ChannelKind::Drums);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        song.channels[1].params.0[0] = 0.3;
        link.send(AudioCommand::SetParam {
            channel: 1,
            instance_id: 0,
            index: 0,
            value: 0.3,
        });
        let slot = &mut song.channels[2].drum_kit.as_mut().unwrap().slots[0];
        slot.file.reverse = true;
        let id = -(slot.file.id as i32);
        link.send(AudioCommand::SetSetting {
            channel: 2,
            instance_id: id,
            key: crate::audio_engine::settings::SLOT_REVERSE,
            value: 1.0,
        });
        rec.take();
        song.channels[1].lanes[0].clips[0].events[1] = note(10.0, 1.0, 64);
        assert!(link.replace_song(&song, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
    }

    /// A param the engine never got (no live command since the last build), with a note move: the edit's engine takes
    /// over the playing one, and the song's new value is heard.
    #[test]
    fn a_param_changed_without_a_live_command_is_heard() {
        let mut song = synth(&[note(0.0, 4.0, 57), note(8.0, 1.0, 64)]);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        song.channels[1].params.0[0] = 0.3;
        song.channels[1].lanes[0].clips[0].events[1] = note(10.0, 1.0, 64);
        assert!(link.replace_song(&song, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        assert_close(&rendered(&rec, 1.0), &reference(&song, 1.0, 1.0));
    }

    /// A tempo drag sets the engine's tempo live, nothing rebuilt: the sounding note goes on as if nothing happened, at
    /// The same beat, and the transport runs at the new tempo. The drag's end, with no sampled audio clip, takes over
    /// without a re-sync; so does a step of the tempo.
    #[test]
    fn a_tempo_drag_keeps_the_voices() {
        let song = synth(&[note(0.0, 8.0, 57)]);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        link.set_tempo(240.0);
        link.set_tempo(240.0);
        assert_eq!(rec.take(), ["SetTempo 240.000"]);
        assert_eq!(link.engine_tempo(&song), 240.0);
        assert_eq!(tick(&rec), u64::from(PPQ));
        assert_close(&rendered(&rec, 0.5), &reference(&song, 1.0, 0.5));
        // Half a beat at 120 BPM is a whole one at 240.
        assert_eq!(tick(&rec), u64::from(2 * PPQ));
        let mut faster = song.clone();
        faster.set_tempo(240.0);
        assert!(link.replace_song(&faster, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
        assert_eq!(
            (tick(&rec), sounding(&rec, 1)),
            (u64::from(2 * PPQ), 1 << 57)
        );
        assert_eq!(link.engine_tempo(&song), 240.0);
        // A step to 120: the engine's position stays at its beat.
        assert!(link.replace_song(&song, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
        assert_eq!(
            (tick(&rec), sounding(&rec, 1)),
            (u64::from(2 * PPQ), 1 << 57)
        );
        assert!(energy(&rendered(&rec, 1.0)) > 0.01);
    }

    /// A tempo change re-renders the sampled audio clips and re-syncs; the synth's note sounds on.
    #[test]
    fn a_tempo_change_with_a_sampled_audio_clip_re_syncs() {
        let mut song = synth(&[note(0.0, 8.0, 57)]);
        song.add_channel(ChannelKind::AudioClip);
        let mut slot = SamplerSlot::default();
        slot.file.path = Some(ContentPath::Plain("dc".into()));
        song.channels[2].lanes[0].add_clip(0, 8.0).file.sample = Some(Box::new(slot));
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        link.set_tempo(130.0);
        rec.take();
        let mut faster = song.clone();
        faster.set_tempo(130.0);
        assert!(link.replace_song(&faster, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        assert_eq!(sounding(&rec, 1), 1 << 57);
        assert!(energy(&rendered(&rec, 0.5)) > 0.01);
    }

    /// A slot playing the eight-second sample.
    fn sampled() -> Box<SamplerSlot> {
        let mut slot = SamplerSlot::default();
        slot.file.path = Some(ContentPath::Plain("dc".into()));
        Box::new(slot)
    }

    /// 120 BPM, an Audio Clip channel (1) with `takes` lanes, each one sampled clip of 8 beats from beat 0.
    fn takes(takes: usize) -> Song {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::AudioClip);
        for take in 0..takes {
            let lane = match take {
                0 => 0,
                _ => song.channels[1].add_lane(LaneKind::Audio),
            };
            let mut slot = sampled();
            slot.file.path = Some(ContentPath::Plain(format!("take {take}")));
            song.channels[1].lanes[lane].add_clip(0, 8.0).file.sample = Some(slot);
        }
        song
    }

    /// A lane added with clips (a recorded take's, `recording/recorder.rs`) re-syncs, which brings its clips' audio
    /// slots in: after Stop and Play from the start, both takes sound.
    #[test]
    fn a_take_recorded_onto_a_new_lane_plays_after_stop_and_play() {
        let rec = Recorder::default();
        let mut link = playing(&rec, &takes(1));
        rendered(&rec, 1.0);
        assert!(link.replace_song(&takes(2), true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        for cmd in [
            AudioCommand::Stop,
            AudioCommand::Seek(0),
            AudioCommand::Play,
        ] {
            link.send(cmd);
        }
        assert_close(&rendered(&rec, 1.0), &reference(&takes(2), 0.0, 1.0));
    }

    /// An empty lane added does nothing: no re-sync.
    #[test]
    fn an_empty_lane_added_while_playing_does_not_re_sync() {
        let song = synth(&[note(0.0, 4.0, 57)]);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let mut added = song.clone();
        added.channels[1].add_lane(LaneKind::Notes);
        assert!(link.replace_song(&added, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
    }

    /// An unchanged automation lane on the channel leaves a graph-lane edit (a velocity) no re-sync.
    #[test]
    fn a_graph_lane_edit_on_a_channel_with_an_automation_lane_does_not_re_sync() {
        let mut song = synth(&[note(0.0, 4.0, 57)]);
        let level = AutomationTarget {
            instance_id: 0,
            param: 0,
        };
        let lane = song.channels[1].add_automation_lane(level);
        lane.add_clip(0, 4.0).events.push(Event::Automation {
            pos: 0,
            value: 0.5,
            control: true,
        });
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let mut added = song.clone();
        let notes = added.channels[1]
            .lanes
            .iter_mut()
            .find(|l| l.kind == LaneKind::Notes);
        if let Event::Note { velocity, .. } = &mut notes.unwrap().clips[0].events[0] {
            *velocity = 50;
        }
        assert!(link.replace_song(&added, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
    }

    /// An edit without a re-sync keeps the playing audio clips' slots: a step tapped on ([`Handover::Quiet`]) while an
    /// audio clip sounds leaves it sounding as if nothing happened.
    #[test]
    fn an_audio_clip_sounds_on_through_a_step_tap() {
        let mut song = takes(1);
        song.add_channel(ChannelKind::Drums);
        song.channels[2].lanes[0].add_clip(0, 4.0);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let mut tapped = song.clone();
        tapped.channels[2].lanes[0].clips[0].events.push(hit(3.0));
        let quiet = Handover::Quiet;
        assert!(link.replace_song_keeping(&tapped, &tapped, true, tick(&rec), &read, quiet));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
        assert_close(&rendered(&rec, 1.0), &reference(&song, 1.0, 1.0));
    }

    /// The engine taking over plays the new song's loop: an edit with the loop moved in to two beats wraps there.
    #[test]
    fn an_edit_takes_the_new_songs_loop() {
        let mut song = synth(&[note(0.0, 4.0, 57), note(8.0, 1.0, 64)]);
        song.play_loop = Some((0, 8 * PPQ));
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        song.play_loop = Some((0, 2 * PPQ));
        song.channels[1].lanes[0].clips[0].events[1] = note(10.0, 1.0, 64);
        assert!(link.replace_song(&song, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        rendered(&rec, 1.5);
        assert_eq!(tick(&rec), u64::from(PPQ / 2));
    }

    /// A knob still waiting in a full queue when an edit's engine follows goes out, before it: the engine taking over
    /// has it.
    #[test]
    fn a_knob_still_queued_goes_out_before_the_edits_engine() {
        let mut song = synth(&[note(0.0, 4.0, 57), note(8.0, 1.0, 64)]);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rec.0.borrow_mut().room = Some(0);
        song.channels[1].params.0[0] = 0.3;
        link.send(AudioCommand::SetParam {
            channel: 1,
            instance_id: 0,
            index: 0,
            value: 0.3,
        });
        song.channels[1].lanes[0].clips[0].events[1] = note(10.0, 1.0, 64);
        assert!(link.replace_song(&song, true, 0, &read));
        rec.0.borrow_mut().room = None;
        link.flush();
        let sent = ["SetParam 1 0 0 0.300", "ReplaceSong adopting", "Play"];
        assert_eq!(rec.take(), sent);
    }

    /// While the queue stays full, an edit's engine replaces the one still waiting, and takes on its re-sync and
    /// silenced channels.
    #[test]
    fn a_waiting_edit_engine_gives_way_to_the_next() {
        let lanes = |lanes: usize, e4: bool| {
            let mut song = synth(&[note(0.0, 4.0, 57)]);
            for _ in 1..lanes {
                song.channels[1].add_lane(LaneKind::Notes);
            }
            if e4 {
                song.channels[1].lanes[0].clips[0]
                    .events
                    .push(note(6.0, 1.0, 64));
            }
            song
        };
        let rec = Recorder::default();
        let mut link = playing(&rec, &lanes(2, false));
        rec.0.borrow_mut().room = Some(0);
        assert!(link.replace_song(&lanes(1, false), true, 0, &read));
        assert!(link.replace_song(&lanes(1, true), true, 0, &read));
        assert_eq!(link.pending(), 3);
        rec.0.borrow_mut().room = None;
        link.flush();
        let sent = ["ReplaceSong adopting silencing 1", "Play", "Play"];
        assert_eq!(rec.take(), sent);
        // The next edit moved the silenced channel (a channel deleted before it); it is silenced where it is.
        let mut song = lanes(2, false);
        song.insert_channel(1, ChannelKind::Synth(SynthKind::MiniSynth));
        let mut link = playing(&rec, &song);
        rec.0.borrow_mut().room = Some(0);
        let mut deleted = song.clone();
        deleted.channels[2].lanes.pop();
        assert!(link.replace_song(&deleted, true, 0, &read));
        deleted.delete_channel(1);
        assert!(link.replace_song(&deleted, true, 0, &read));
        rec.0.borrow_mut().room = None;
        link.flush();
        assert_eq!(
            rec.take(),
            ["ReplaceSong adopting silencing 1", "Play", "Play"]
        );
    }

    /// As the merge has it, a value sent live is the engine's only while the song holds it. A knob at 0.3, then an undo
    /// that puts it back: the song's value is heard. A reopened stream forgets a live tempo.
    #[test]
    fn a_value_sent_live_gives_way_to_the_songs() {
        let song = synth(&[note(0.0, 4.0, 57), note(8.0, 1.0, 64)]);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        link.send(AudioCommand::SetParam {
            channel: 1,
            instance_id: 0,
            index: 0,
            value: 0.3,
        });
        // The recorder logs params; the device's engine would have it.
        let mut recorded = rec.0.borrow_mut();
        recorded.engine.as_mut().unwrap().set_param(1, 0, 0, 0.3);
        drop(recorded);
        rec.take();
        let undone =
            link.replace_song_keeping(&song, &song, true, tick(&rec), &read, Handover::Undo);
        assert!(undone);
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        assert_close(&rendered(&rec, 1.0), &reference(&song, 1.0, 1.0));
        link.set_tempo(140.0);
        rec.0.borrow_mut().failed = true;
        link.poll();
        link.start(&song, true, &read, |_, _| panic!("reopened"));
        assert_eq!(link.engine_tempo(&song), 120.0);
    }

    /// A drag that ends back at the tempo it started from still re-renders the sampled audio clips and re-syncs;
    /// without one it changes nothing.
    #[test]
    fn a_drag_back_to_the_start_tempo_re_syncs_only_with_a_sampled_audio_clip() {
        for (song, handed) in [
            (takes(1), "ReplaceSong adopting"),
            (
                synth(&[note(0.0, 4.0, 57)]),
                "ReplaceSong adopting no-resync",
            ),
        ] {
            let rec = Recorder::default();
            let mut link = playing(&rec, &song);
            rendered(&rec, 1.0);
            link.set_tempo(130.0);
            link.set_tempo(120.0);
            rec.take();
            assert!(link.replace_song(&song, true, tick(&rec), &read));
            assert_eq!(rec.take(), [handed, "Play"]);
        }
    }

    /// A tempo step (no drag) hands over events built at the new tempo: the position keeps its beat, the sounding note
    /// goes on, and the transport runs at the new tempo.
    #[test]
    fn a_tempo_step_keeps_the_beat() {
        let song = synth(&[note(0.0, 8.0, 57)]);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let mut faster = song.clone();
        faster.set_tempo(240.0);
        assert!(link.replace_song(&faster, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
        assert_eq!((tick(&rec), sounding(&rec, 1)), (u64::from(PPQ), 1 << 57));
        // Half a beat at 120 BPM is a whole one at 240.
        rendered(&rec, 0.5);
        assert_eq!(tick(&rec), u64::from(2 * PPQ));
    }

    /// A MiniSynth channel holding A3 from beat 0 to 1.5, and a second channel.
    fn held_song() -> Song {
        let mut song = synth(&[note(0.0, 1.5, 57)]);
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        song
    }

    /// Re-sync step 1: the live controllers (a held MIDI sustain pedal, Pitch Bend, channel pressure, a CC) are in no
    /// song. An edit leaves them where they were. A re-sync (an undo's) keeps the MIDI CCs, so the pedal still holds
    /// The note-off it caught at beat 1.5, but puts Pitch Bend and channel pressure back to the song's, as a seek does;
    /// The next edit leaves them there.
    #[test]
    fn live_controllers_hold_through_an_edit_and_a_re_sync() {
        let song = held_song();
        let live = [(4, 0.75), (5, 0.3), (8, 0.6), (71, 1.0)];
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let mut held = SongAudioEngine::new(&song, 48_000.0).unwrap();
        held.play();
        render(&mut held, 1.0);
        // As the MIDI input sends them: Pitch Bend and channel pressure live, the CCs as params.
        for (index, value) in live {
            if matches!(index, 4 | 5) {
                link.send(AudioCommand::LiveParam {
                    channel: 1,
                    index,
                    value,
                });
                held.set_live_param(1, index, value);
            } else {
                link.send(AudioCommand::SetParam {
                    channel: 1,
                    instance_id: 0,
                    index,
                    value,
                });
                held.set_param(1, 0, index, value);
            }
        }
        let mut added = song.clone();
        added.channels[2].rack.add(32);
        rec.take();
        assert!(link.replace_song(&added, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
        assert_close(&rendered(&rec, 2.0), &render(&mut held, 2.0));
        let undone = Handover::Undo;
        assert!(link.replace_song_keeping(&song, &song, true, tick(&rec), &read, undone));
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        held.seek(tick(&rec));
        assert!(link.replace_song(&added, true, tick(&rec), &read));
        rec.take();
        assert_close(&rendered(&rec, 1.0), &render(&mut held, 1.0));
    }

    /// A held MIDI pedal sustains the same after a seek and after an undo (both re-sync).
    #[test]
    fn a_held_pedal_sustains_the_same_after_a_seek_and_after_an_undo() {
        let song = held_song();
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        let mut sought = SongAudioEngine::new(&song, 48_000.0).unwrap();
        sought.play();
        for engine in [rec.0.borrow_mut().engine.as_mut().unwrap(), &mut sought] {
            render(engine, 1.0);
        }
        link.send(AudioCommand::SetParam {
            channel: 1,
            instance_id: 0,
            index: 71,
            value: 1.0,
        });
        sought.set_param(1, 0, 71, 1.0);
        rendered(&rec, 1.0);
        render(&mut sought, 1.0);
        let at = tick(&rec);
        assert!(link.replace_song_keeping(&song, &song, true, at, &read, Handover::Undo));
        sought.seek(at);
        let after = rendered(&rec, 1.0);
        assert!(energy(&after) > 0.01, "the pedal let go");
        assert_close(&after, &render(&mut sought, 1.0));
    }

    /// Preview switched on gives every channel all notes off, then a re-sync; its engine takes over the playing one, so
    /// The delay tail rings on.
    #[test]
    fn preview_gives_every_channel_all_notes_off_and_keeps_the_tails() {
        let mut song = synth(&[note(0.0, 0.25, 60), note(0.0, 4.0, 57)]);
        song.channels[1].rack.add(DELAY);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let previewed = Handover::Preview { seek: None };
        let tick = tick(&rec);
        assert!(link.replace_song_keeping(&song, &song, true, tick, &read, previewed));
        assert_eq!(rec.take(), ["ReplaceSong adopting silencing 0,1", "Play"]);
        assert_eq!(sounding(&rec, 1), 0);
        let mut released = SongAudioEngine::new(&song, 48_000.0).unwrap();
        released.play();
        render(&mut released, 1.0);
        released.all_notes_off();
        assert_close(&rendered(&rec, 1.0), &render(&mut released, 1.0));
    }

    /// Deleting a channel removes the other channels' Enable Send and Send Level lanes for it (`Song::delete_channel`);
    /// that is no lane of theirs deleted, so none of them is silenced.
    #[test]
    fn a_deleted_channels_send_lanes_silence_no_other_channel() {
        let mut song = synth(&[note(0.0, 4.0, 57)]);
        song.add_channel(ChannelKind::EffectTrack);
        let id = song.channels[2].id;
        song.channels[1].add_automation_lane(AutomationTarget {
            instance_id: 0,
            param: 0x2000 + id,
        });
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        let mut deleted = song.clone();
        deleted.delete_channel(2);
        assert_eq!(deleted.channels[1].lanes.len(), 1);
        assert!(link.replace_song(&deleted, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        assert_eq!(sounding(&rec, 1), 1 << 57);
    }

    /// With the queue full, an undo, the pedal sent again, and an edit whose engine waits behind the pedal (it names
    /// The undo's channel); then the next edit with the queue free. None lifts the pedal: the note-off it caught at
    /// beat 1.5 sustains throughout.
    #[test]
    fn a_merged_re_sync_and_the_next_edit_keep_a_held_pedal() {
        let song = held_song();
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        let mut held = SongAudioEngine::new(&song, 48_000.0).unwrap();
        held.play();
        for engine in [rec.0.borrow_mut().engine.as_mut().unwrap(), &mut held] {
            render(engine, 1.0);
        }
        let pedal = || AudioCommand::SetParam {
            channel: 1,
            instance_id: 0,
            index: 71,
            value: 1.0,
        };
        link.send(pedal());
        held.set_param(1, 0, 71, 1.0);
        rendered(&rec, 1.0);
        render(&mut held, 1.0);
        rec.0.borrow_mut().room = Some(0);
        let undone = Handover::Undo;
        assert!(link.replace_song_keeping(&song, &song, true, tick(&rec), &read, undone));
        link.send(pedal());
        let mut added = song.clone();
        added.channels[2].rack.add(32);
        assert!(link.replace_song(&added, true, tick(&rec), &read));
        rec.0.borrow_mut().room = None;
        rec.take();
        link.flush();
        let sent = [
            "ReplaceSong adopting",
            "Play",
            "SetParam 1 0 71 1.000",
            "ReplaceSong adopting no-resync",
            "Play",
        ];
        assert_eq!(rec.take(), sent);
        assert_close(&rendered(&rec, 0.5), &render(&mut held, 0.5));
        added.channels[2].rack.add(18);
        assert!(link.replace_song(&added, true, tick(&rec), &read));
        assert_close(&rendered(&rec, 1.0), &render(&mut held, 1.0));
    }

    /// Paste and Copy at interval rebuild the clip, then re-sync. Notes added are no exemption; only a
    /// tap-create is, which its editor reports ([`Handover::Quiet`]).
    #[test]
    fn notes_pasted_or_copied_at_an_interval_re_sync() {
        let song = synth(&[note(0.0, 4.0, 57)]);
        let mut pasted = song.clone();
        let events = &mut pasted.channels[1].lanes[0].clips[0].events;
        events.extend([note(8.0, 1.0, 60), note(8.0, 1.0, 64)]);
        let mut copied = song.clone();
        (copied.channels[1].lanes[0].clips[0].events).push(note(4.0, 4.0, 57));
        for edited in [pasted, copied] {
            let rec = Recorder::default();
            let mut link = playing(&rec, &song);
            rendered(&rec, 1.0);
            assert!(link.replace_song(&edited, true, tick(&rec), &read));
            assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        }
    }

    /// A clip created or pasted in the playlist re-syncs.
    #[test]
    fn a_clip_created_or_pasted_in_the_playlist_re_syncs() {
        let song = synth(&[note(0.0, 4.0, 57)]);
        let mut created = song.clone();
        created.channels[1].lanes[0].add_clip(16 * PPQ, 4.0);
        let mut pasted = song.clone();
        let lane = &mut pasted.channels[1].lanes[0];
        lane.add_clip(16 * PPQ, 16.0).events = vec![note(0.0, 4.0, 57)];
        for edited in [created, pasted] {
            let rec = Recorder::default();
            let mut link = playing(&rec, &song);
            rendered(&rec, 1.0);
            assert!(link.replace_song(&edited, true, tick(&rec), &read));
            assert_eq!(rec.take(), ["ReplaceSong adopting", "Play"]);
        }
    }

    /// A drum row removed whose hit held its clip's loop at 8 beats; the clip-update rule shrinks it to 4. A refit of a
    /// loop whose events are kept is no lane change: nothing re-syncs.
    #[test]
    fn a_drum_row_removed_that_shrinks_a_loop_does_not_re_sync() {
        let mut song = drums(&[0.0]);
        let clip = &mut song.channels[1].lanes[0].clips[0];
        let mut late = hit(6.0);
        if let Event::Drum { pad, .. } = &mut late {
            *pad = 1;
        }
        clip.events.push(late);
        clip.fit_loop(4);
        assert_eq!(clip.loop_len_beats, 8.0);
        let mut removed = song.clone();
        removed.channels[1]
            .drum_kit
            .as_mut()
            .unwrap()
            .slots
            .remove(1);
        let clip = &mut removed.channels[1].lanes[0].clips[0];
        clip.events.pop();
        clip.fit_loop(4);
        assert_eq!(clip.loop_len_beats, 4.0);
        let rec = Recorder::default();
        let mut link = playing(&rec, &song);
        rendered(&rec, 1.0);
        assert!(link.replace_song(&removed, true, tick(&rec), &read));
        assert_eq!(rec.take(), ["ReplaceSong adopting no-resync", "Play"]);
    }

    /// A tap on an automation clip makes it its lane's current clip (DESc@24), view state the link's last song takes
    /// from the new one (`Song::keep_current_clips`). A graph-lane edit after it (velocity) still runs no re-sync: the
    /// automation lane differs only in its current clip.
    #[test]
    fn a_current_clip_tapped_on_another_lane_is_no_lane_change() {
        let mut song = synth(&[note(0.0, 4.0, 57)]);
        let level = AutomationTarget {
            instance_id: 0,
            param: 0,
        };
        let lane = song.channels[1].add_automation_lane(level);
        lane.add_clip(0, 4.0);
        let tapped = lane.add_clip(8 * PPQ, 4.0).id;
        let mut new = song.clone();
        let channel = &mut new.channels[1];
        let auto = (channel.lanes.iter_mut())
            .find(|l| l.kind == LaneKind::Automation)
            .unwrap();
        assert_ne!(auto.file.current_clip_id, tapped);
        auto.file.current_clip_id = tapped;
        let notes = (channel.lanes.iter_mut())
            .find(|l| l.kind == LaneKind::Notes)
            .unwrap();
        if let Event::Note { velocity, .. } = &mut notes.clips[0].events[0] {
            *velocity = 50;
        }
        assert!(!super::edit(&mut song, &new, false).resync);
    }

    /// A fresh engine and a reopened stream start from the song, so the live controllers are forgotten: the
    /// next edit leaves no stale pedal down, and A3's note-off at beat 2.5 releases it.
    #[test]
    fn a_fresh_engine_or_a_reopen_forgets_the_live_controllers() {
        let mut song = synth(&[note(2.0, 0.5, 57)]);
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let mut added = song.clone();
        added.channels[2].rack.add(32);
        for reopen in [false, true] {
            let rec = Recorder::default();
            let mut link = playing(&rec, &song);
            rendered(&rec, 0.5);
            link.send(AudioCommand::SetParam {
                channel: 1,
                instance_id: 0,
                index: 71,
                value: 1.0,
            });
            if reopen {
                rec.0.borrow_mut().failed = true;
                link.poll();
                link.start(&song, true, &read, |_, _| panic!("reopened"));
            } else {
                let at = tick(&rec);
                assert!(link.replace_song_keeping(&song, &song, true, at, &read, Handover::Fresh));
            }
            assert!(link.replace_song(&added, true, tick(&rec), &read));
            rendered(&rec, 3.0);
            assert_eq!(
                energy(&rendered(&rec, 0.5)),
                0.0,
                "a stale pedal, reopen {reopen}"
            );
        }
    }
}

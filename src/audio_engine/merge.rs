//! An edit's engine taking over the playing one: an edit cuts only what it destroys, as if the engine were never
//! rebuilt. The engine built for the edited song keeps its structure (routing, racks, kits, prepared events, params)
//! and takes the playing engine's sound into it, channel by channel (matched by channel id) and module by module: the
//! instruments of the same instance id and build, the effects of the same instance id and type, the pads' voices by
//! slot id, the audio clips' voices unless the edit re-syncs, the live keys, and what the sequencer and the sustain
//! pedals hold. What it does not take (a deleted channel, a removed effect or drum row, an instrument of another type
//! or build) stays in the old engine, so it stops at once, and is freed with it off the audio thread; what it built
//! afresh (an added module, row or channel) starts empty.

use honeydew_audio::device::AudioRenderer;
use honeydew_audio::instrument::InstrumentModule;

use super::{SongAudioEngine, midi_cc, position};

/// Channel params per channel: the strip's, Pitch Bend, channel pressure and 7 + each MIDI CC.
const CHANNEL_PARAMS: usize = 134;

impl SongAudioEngine {
    /// Takes over `old`'s sound (see the module docs), then, in this order: each channel of [`Self::silences`] gets all
    /// notes off; a rolling transport re-syncs to the playhead if [`Self::resyncs`]. Without a re-sync each param takes
    /// its value now (this song's initial value, plus its automation at the playhead while the transport rolls, as Stop
    /// and a re-sync leave them; a MIDI CC keeps `old`'s, [`Self::keep_ccs`]), set only where the module holding it has
    /// another: a param set again may act again (a Pitcher 2 Low Latency switch rebuilds). The params are this song's:
    /// A value sent live is the song's too when the song still holds it, and the song's wins when it does not (an
    /// undo). The transport goes on at this song's tempo (its live tempo is this build's) with its beat kept, its
    /// position counted at this build's tempo (`built_bpm`); so do the clicks, the count-in and the live clock.
    /// A re-syncing edit restarts every audio lane on every channel, since the new song's clips
    /// replace every clip slot (they are renumbered): each clip voice stops at once and the covering unmuted clips
    /// restart. An edit without a re-sync changes no audio clip, so `old`'s play on, unless a sample's data changed
    /// under them. Allocates nothing.
    pub(super) fn merge(&mut self, old: &mut Self) {
        // The position, at `old`'s build tempo, kept in beats at this one's (an exact frame stays exact).
        let built = f64::from(old.built_bpm) / f64::from(self.built_bpm);
        let at = (old.frame as f64 + old.phase) * built;
        let whole = if (at - at.round()).abs() < 1e-6 {
            at.round()
        } else {
            at.floor()
        };
        (self.frame, self.phase) = (whole as u64, (at - whole).max(0.0));
        (self.playing, self.rendered) = (old.playing, old.rendered);
        // The count-in counts output frames at the live tempo.
        self.count_in = old
            .count_in
            .map(|count_in| self.retime_count_in(count_in, old.tempo_bpm));
        self.count_in_beats = old.count_in_beats;
        std::mem::swap(&mut self.metronome, &mut old.metronome);
        (self.metronome_on, self.metronome_level) = (old.metronome_on, old.metronome_level);
        self.clicked_beat = old.clicked_beat;
        // Song length re-estimate: with no loop, a transport at or past a changed song end wraps modulo the length,
        // which re-syncs nothing itself ([`Self::wrap_range`]: not while recording).
        if let (None, Some((_, end))) = (self.loop_frames, self.wrap_range())
            && self.song_beats != old.song_beats
            && self.frame >= end
        {
            (self.frame, self.clicked_beat) = (self.frame % end, None);
        }
        (self.cover_replays, self.cover_events) = (old.cover_replays, old.cover_events);
        let retime = (old.built_bpm != self.built_bpm).then_some(built);
        for channel in 0..self.instruments.len() {
            if let Some(from) = old.routing.index(self.routing.ids[channel]) {
                self.adopt_channel(old, channel, from, retime);
            }
        }
        self.apply_settings();
        for position in 0..self.silence.len() {
            self.release_channel(self.silence[position]);
        }
        let frame = self.frame;
        self.action_index = self.actions.partition_point(|event| event.frame < frame);
        self.keep_ccs(old);
        if self.resync && self.rolling() {
            self.resync();
        } else {
            self.set_changed_params(old);
            self.keep_bend(old);
        }
        self.sync_controllers(old);
    }

    /// Without a re-sync, a live Pitch Bend or Aftertouch plays on as `old` had it: only a re-sync and Stop put it back
    /// to the song's value, and no song holds it. One the song changed (the pitch wheel writes it) is the song's.
    fn keep_bend(&mut self, old: &Self) {
        for channel in 0..self.instruments.len() {
            let Some(from) = old.routing.index(self.routing.ids[channel]) else {
                continue;
            };
            for index in [4, 5] {
                let saved = |engine: &Self, channel| {
                    position(&engine.initial, (channel, 0, index)).map(|at| engine.initial[at].3)
                };
                if saved(self, channel) == saved(old, from)
                    && let Some(value) = old.audio.channel_parameter(from, index as usize)
                {
                    self.apply_param(channel, 0, index, value);
                }
            }
        }
    }

    /// The MIDI CCs (7–133, the sustain pedal's too) of each channel taken over are `old`'s, whatever set them
    /// (automation, MIDI, the link): the song holds none, an edit leaves every live state as it was, and a re-sync
    /// keeps them, its note-offs meeting the pedal as it stands, but for those its automation at the playhead sets.
    /// An edit to a CC's automation is heard only through that re-sync; stopped, the CC keeps its value.
    fn keep_ccs(&mut self, old: &Self) {
        for channel in 0..self.instruments.len() {
            let Some(from) = old.routing.index(self.routing.ids[channel]) else {
                continue;
            };
            for index in 7..CHANNEL_PARAMS {
                if let Some(value) = old.audio.channel_parameter(from, index)
                    && self.audio.channel_parameter(channel, index) != Some(value)
                {
                    self.apply_param(channel, 0, index as i32, value);
                }
            }
        }
    }

    /// Each param in `initial` that the module holding it does not have yet takes its value now: its initial value,
    /// plus its automation at the playhead while the transport rolls. The CCs of a channel taken over keep `old`'s
    /// ([`Self::keep_ccs`]).
    fn set_changed_params(&mut self, old: &Self) {
        let (frame, rolling) = (self.frame, self.rolling());
        for at in 0..self.initial.len() {
            let (channel, instance_id, index, value) = self.initial[at];
            if midi_cc(instance_id, index) && old.routing.index(self.routing.ids[channel]).is_some()
            {
                continue;
            }
            let param = (channel, instance_id, index);
            let automated = rolling.then(|| self.automated(param, frame)).flatten();
            let target = automated.unwrap_or(value);
            if self.applied[at] != target {
                self.apply_param(channel, instance_id, index, target);
            }
        }
    }

    /// Pitch Bend and channel pressure, which the instruments and pads hear, where this engine's channel now holds
    /// another value than `old`'s did, reach them: the instruments and pad voices taken over hold `old`'s. A song's new
    /// value (a bend) gets to them; a live one comes through [`Self::keep_bend`] and so changes nothing here, and a
    /// re-sync puts them back to the song's. The MIDI CCs are `old`'s ([`Self::keep_ccs`]).
    fn sync_controllers(&mut self, old: &Self) {
        for channel in 0..self.instruments.len() {
            let Some(from) = old.routing.index(self.routing.ids[channel]) else {
                continue;
            };
            for index in 4..=5 {
                let value = self.audio.channel_parameter(channel, index);
                if let Some(value) =
                    value.filter(|&v| Some(v) != old.audio.channel_parameter(from, index))
                {
                    self.apply_param(channel, 0, index as i32, value);
                }
            }
        }
    }

    /// Channel `channel` takes over `old`'s channel `from` (the same channel id). `retime`: `old`'s build tempo over
    /// this one's, when they differ (the audio clips kept are timed at it).
    fn adopt_channel(&mut self, old: &mut Self, channel: usize, from: usize, retime: Option<f64>) {
        // An instrument of the same instance id and build plays on (a preset of its type applies to it: the new params
        // follow), with this song's rack place and key range; another type or build is a new one, which starts silent.
        for row in 0..self.instruments[channel].len() {
            let (id, origin) = (
                self.instruments[channel][row].0,
                self.instruments[channel][row].1.origin,
            );
            let same =
                |(other, module): &(i32, InstrumentModule)| *other == id && module.origin == origin;
            let Some(old_row) = old.instruments[from].iter().position(same) else {
                continue;
            };
            let (built, running) = (
                &mut self.instruments[channel][row].1,
                &mut old.instruments[from][old_row].1,
            );
            std::mem::swap(built, running);
            (built.keys, built.effects_above) = (running.keys, running.effects_above);
            take_applied(
                (&self.initial, &mut self.applied),
                (&old.initial, &old.applied),
                (channel, from, id),
            );
            if old.tempo_bpm != self.tempo_bpm {
                built.instrument.set_tempo_bpm(self.tempo_bpm);
            }
            self.held[channel][row] = old.held[from][old_row];
            self.sustained[channel][row] = old.sustained[from][old_row];
        }
        // The pads' notes, held after the instruments' rows.
        let (pads, old_pads) = (self.instruments[channel].len(), old.instruments[from].len());
        self.held[channel][pads] = old.held[from][old_pads];
        self.kit_sustained[channel] = old.kit_sustained[from];
        (self.held_keys[channel], self.sounding[channel]) =
            (old.held_keys[from], old.sounding[from]);
        if let (Some(kit), Some(running)) = (&mut self.kits[channel], &old.kits[from]) {
            kit.adopt(running, &mut self.held_pads[channel], &old.held_pads[from]);
        }
        // A sample whose data changed (rewritten under its path, or a missing one loaded) is not the old clips': the new
        // clips play it from their next action, or from the re-sync that relaunches them.
        let same = (self.clips[channel].as_ref().zip(old.clips[from].as_ref()))
            .is_some_and(|(new, running)| new.same_samples(running));
        if !self.resync && same {
            std::mem::swap(&mut self.clips[channel], &mut old.clips[from]);
            if let (Some(ratio), Some(clips)) = (retime, &mut self.clips[channel]) {
                clips.retime(|frame| (frame as f64 * ratio).round() as u64);
            }
        }
        let (initial, applied) = (&self.initial, &mut self.applied);
        let theirs = (&old.initial[..], &old.applied[..]);
        self.audio
            .adopt_effects(channel, &mut old.audio, from, |id| {
                take_applied((initial, applied), theirs, (channel, from, id));
            });
        if let (Some(live), Some(running)) = (&mut self.live[channel], &mut old.live[from]) {
            let rows = (&self.instruments[channel], &old.instruments[from]);
            live.adopt(running, rows.0, rows.1, self.rendered);
        }
    }

    /// Gives every module its saved settings ([`super::settings::saved`]): at build, and again to the modules an edit's
    /// engine takes over, so a setting the song changed applies to them.
    pub(super) fn apply_settings(&mut self) {
        for position in 0..self.settings.len() {
            let (channel, instance_id, key, value) = self.settings[position];
            self.set_setting(channel, instance_id, key, value);
        }
    }
}

/// A param's (channel, instance id, index) and its value before automation, as `SongAudioEngine::initial` holds it.
type Initial = (usize, i32, i32, f32);

/// Each param of module `id` on channel `channel` in `ours` (initial, applied) takes the value `theirs` applied to it on
/// channel `from` (the module was taken over from there): what it holds.
fn take_applied(
    ours: (&[Initial], &mut [f32]),
    theirs: (&[Initial], &[f32]),
    (channel, from, id): (usize, usize, i32),
) {
    let (initial, applied) = ours;
    let start = initial.partition_point(|&(c, i, _, _)| (c, i) < (channel, id));
    for at in start..initial.len() {
        let (c, i, index, _) = initial[at];
        if (c, i) != (channel, id) {
            break;
        }
        if let Some(before) = position(theirs.0, (from, id, index)) {
            applied[at] = theirs.1[before];
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use honeydew_audio::ProcessContext;
    use honeydew_audio::device::AudioRenderer;
    use honeydew_audio::instrument::Instrument;

    use super::*;
    use crate::song::{ChannelKind, Event, ModuleInstance, Song, SynthKind};

    /// Records every param it is set, as (index, value), and each tempo as (`TEMPO`, BPM).
    struct Probe(Arc<Mutex<Vec<(usize, f32)>>>);

    /// What [`Probe`] records a tempo as.
    const TEMPO: usize = usize::MAX;

    impl Instrument for Probe {
        fn set_parameter(&mut self, index: usize, value: f32) {
            self.0.lock().unwrap().push((index, value));
        }
        fn set_tempo_bpm(&mut self, bpm: f32) {
            self.0.lock().unwrap().push((TEMPO, bpm));
        }
        fn note_on(&mut self, _: u8, _: f32, _: f32) {}
        fn note_off(&mut self, _: u8) {}
        fn all_notes_off(&mut self) {}
        fn reset(&mut self) {}
        fn process(&mut self, _: &mut [f32], _: &mut [f32], _: ProcessContext) {}
    }

    /// A MiniSynth on channel 1 playing a note, and a second MiniSynth channel.
    fn song() -> Song {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        for _ in 0..2 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        song.channels[1].lanes[0]
            .add_clip(0, 16.0)
            .events
            .push(Event::Note {
                pos: 0,
                len_beats: 8.0,
                key: 57,
                velocity: 204,
                pan: 127,
                slide: false,
            });
        song
    }

    /// `song` playing (or stopped) with a probe for its channel 1 instrument, taken over by an engine for `edited`:
    /// the params the probe was set by the take-over.
    fn set_by_the_edit(edited: &Song, playing: bool, resync: bool) -> Vec<(usize, f32)> {
        let mut old = SongAudioEngine::new(&song(), 48_000.0).unwrap();
        let set = Arc::new(Mutex::new(Vec::new()));
        old.instruments[1][0].1.instrument = Box::new(Probe(set.clone()));
        if playing {
            old.play();
        }
        old.render(&mut [0.0; 2048], 2);
        set.lock().unwrap().clear();
        let mut new = SongAudioEngine::new(edited, 48_000.0)
            .unwrap()
            .adopting_edit(resync, Vec::new());
        new.take_over(&mut old);
        std::mem::take(&mut *set.lock().unwrap())
    }

    /// A take-over sets an adopted module only the params whose value it changes (a param set again may act again: a
    /// Pitcher 2 Low Latency switch rebuilds), playing or stopped; a re-sync sets every param.
    #[test]
    fn a_take_over_sets_only_the_params_it_changes_unless_it_re_syncs() {
        let mut added = song();
        added.channels[2].rack.add(32);
        let mut preset = song();
        let mut loaded = ModuleInstance::new(0, 9);
        loaded.params[9] = 0.3;
        preset.channels[1].rack.modules[0].load_preset(loaded);
        for playing in [true, false] {
            assert_eq!(
                set_by_the_edit(&added, playing, false),
                [],
                "playing {playing}"
            );
            let set = set_by_the_edit(&preset, playing, false);
            assert_eq!(set, [(9, 0.3)], "playing {playing}");
        }
        // A re-sync's note-offs meet the instruments' initial params first (`resync`).
        let all = set_by_the_edit(&added, true, true);
        let count = song().channels[1].rack.modules[0].params.len();
        assert!((0..count).all(|param| all.iter().any(|&(index, _)| index == param)));
    }

    /// An instrument taken over at another tempo takes the new one.
    #[test]
    fn an_instrument_taken_over_at_a_new_tempo_takes_it() {
        let mut faster = song();
        faster.tempo_bpm = 140.0;
        let set = set_by_the_edit(&faster, true, false);
        assert!(set.contains(&(TEMPO, 140.0)), "{set:?}");
    }

    /// A tempo drag reaches the instruments.
    #[test]
    fn a_live_tempo_reaches_the_instruments() {
        let mut engine = SongAudioEngine::new(&song(), 48_000.0).unwrap();
        let set = Arc::new(Mutex::new(Vec::new()));
        engine.instruments[1][0].1.instrument = Box::new(Probe(set.clone()));
        engine.set_tempo(140.0);
        assert_eq!(*set.lock().unwrap(), [(TEMPO, 140.0)]);
    }

    const BEAT: usize = 24_000;

    /// The left side of `beats` beats rendered from where the engine is, in odd blocks.
    fn render(engine: &mut SongAudioEngine, beats: f64) -> Vec<f32> {
        let frames = (beats * BEAT as f64) as usize;
        let mut left = Vec::with_capacity(frames);
        let mut output = vec![0.0; 777 * 2];
        while left.len() < frames {
            let n = 777.min(frames - left.len());
            engine.render(&mut output[..n * 2], 2);
            left.extend(output[..n * 2].iter().step_by(2));
        }
        left
    }

    fn energy(samples: &[f32]) -> f32 {
        samples.iter().map(|sample| sample * sample).sum()
    }

    fn assert_close(actual: &[f32], expected: &[f32]) {
        assert!(energy(expected) > 0.01, "a silent reference");
        for (frame, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert!((a - e).abs() <= 1e-5, "frame {frame}: {a} vs {e}");
        }
    }

    fn differ(a: &[f32], b: &[f32]) -> bool {
        a.iter().zip(b).any(|(a, b)| (a - b).abs() > 1e-3)
    }

    fn note(beat: f64, len_beats: f64, key: u8) -> Event {
        Event::Note {
            pos: (beat * 128.0) as u32,
            len_beats,
            key,
            velocity: 204,
            pan: 127,
            slide: false,
        }
    }

    /// 120 BPM, a `kind` channel 1 playing `key` from beat 0 for half a beat and from beat 4 for a beat, and a second
    /// MiniSynth channel.
    fn two_notes(kind: SynthKind, key: u8) -> Song {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(kind));
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let clip = song.channels[1].lanes[0].add_clip(0, 16.0);
        clip.events
            .extend([note(0.0, 0.5, key), note(4.0, 1.0, key)]);
        song
    }

    /// Every file a sine burst: 16-bit, the cache's test WAV, with weaker bursts between stronger ones (a Slicer's
    /// Sensitivity tells them apart).
    fn read(_: &crate::flm::chunk::ContentPath) -> Option<Vec<u8>> {
        let samples: Vec<i16> = (0..44_100)
            .map(|i| {
                let t = i % 4410;
                let level = if (i / 4410) % 2 == 0 {
                    20_000.0
                } else {
                    3_000.0
                };
                if t < 2000 {
                    ((t as f32 * 0.07).sin() * level) as i16
                } else {
                    0
                }
            })
            .collect();
        Some(crate::sample_cache::tests::wav(&samples))
    }

    /// What an edit changes is heard. `before` plays from beat 0; at beat 1 an engine for `after` takes it over without
    /// A re-sync; from beat 4 (a note there) it sounds as `after` played from the start does, and not as `before`.
    fn heard_from_beat_4(before: &Song, after: &Song) {
        let mut samples = crate::sample_cache::SampleCache::default();
        samples.load(before, &read);
        samples.load_keeping(after, before, &read);
        let engine = |song| SongAudioEngine::with_samples(song, 48_000.0, &samples).unwrap();
        let played = |song| {
            let mut engine = engine(song);
            engine.play();
            render(&mut engine, 4.0);
            render(&mut engine, 1.0)
        };
        let heard = played(after);
        assert!(
            differ(&played(before), &heard),
            "the edit changes nothing heard"
        );
        let mut old = engine(before);
        old.play();
        render(&mut old, 1.0);
        let mut new = engine(after).adopting_edit(false, Vec::new());
        new.take_over(&mut old);
        render(&mut new, 3.0);
        assert_close(&render(&mut new, 1.0), &heard);
    }

    /// The DW Sampler's `CSTM`: one zone over every key playing "My Samples/Loop.wav" from root key `root`.
    fn dw_zone(root: u8) -> Vec<u8> {
        let mut zone = b"@L@\0My Samples/Loop.wav".to_vec();
        zone.resize(1024, 0);
        zone.extend([root, 0, 127, 0, 127]);
        for word in [1.0f32.to_bits(), 0.5f32.to_bits(), 2, 100, 30_000] {
            zone.extend(word.to_le_bytes());
        }
        for value in [0.0f32, 1.0, 1.0, 0.1] {
            zone.extend(value.to_le_bytes());
        }
        let mut blob = b"10WDZONE".to_vec();
        blob.extend((zone.len() as u32).to_le_bytes());
        blob.extend(zone);
        blob
    }

    /// A DW Sampler zone edit with the same samples (its root key) is a new build, heard.
    #[test]
    fn a_dw_sampler_zone_edit_is_heard() {
        let zoned = |root| {
            let mut song = two_notes(SynthKind::DwSampler, 60);
            song.channels[1].rack.modules[0].file.custom_data = Some(dw_zone(root));
            song
        };
        heard_from_beat_4(&zoned(48), &zoned(60));
    }

    /// A Slicer's new Sensitivity slices its loop again, heard.
    #[test]
    fn a_slicer_sensitivity_change_is_heard() {
        let sliced = |sensitivity: f32| {
            let mut song = two_notes(SynthKind::Slicer, 49);
            let path = b"@L@\0loop.wav";
            let mut cstm = b"SENS\x05\0\0\0".to_vec();
            cstm.extend(sensitivity.to_le_bytes());
            cstm.push(1);
            cstm.extend(b"PATH");
            cstm.extend((4 + path.len() as u32).to_le_bytes());
            cstm.extend((path.len() as u32).to_le_bytes());
            cstm.extend(path);
            song.channels[1].rack.modules[0].file.custom_data = Some(cstm);
            song
        };
        heard_from_beat_4(&sliced(0.0), &sliced(1.0));
    }

    /// 3x Osc's Custom shape pick, heard.
    #[test]
    fn a_three_osc_custom_shape_pick_is_heard() {
        let shaped = |name: &str| {
            let mut song = two_notes(SynthKind::ThreeXOsc, 69);
            let module = &mut song.channels[1].rack.modules[0];
            let path = crate::flm::chunk::ContentPath::Shorthand {
                root: crate::flm::chunk::ContentRoot::App,
                relative: format!("Built-in/Shapes/{name}.wav"),
            };
            let picked = std::path::Path::new("shape.wav");
            crate::project_files::load_pick(module, picked, &[], Some(path)).unwrap();
            // Osc 1 alone, on Custom.
            for (index, value) in [(1, 1.0), (22, 0.0), (23, 0.0)] {
                module.params[index] = value;
            }
            song
        };
        heard_from_beat_4(&shaped("Vowel A"), &shaped("Glass"));
    }

    /// A SoundFont Player's patch change (its patch spinner writes the `SF2D`) is a new build, as a worker-loaded
    /// instrument's new data is: the note sounding at the edit stops, and the next is the new patch's. The
    /// fixture has one patch, which a higher one clamps to.
    #[test]
    fn a_soundfont_patch_change_builds_the_player_again() {
        let patched = |patch, notes: &[Event]| {
            let mut song = two_notes(SynthKind::SoundFontPlayer, 69);
            song.channels[1].lanes[0].clips[0].events = notes.to_vec();
            let sf2d = crate::flm::sf2d::Sf2d {
                path: crate::flm::chunk::ContentPath::Shorthand {
                    root: crate::flm::chunk::ContentRoot::User,
                    relative: "My SoundFonts/sine.sf2".into(),
                },
                patch,
            };
            song.channels[1].rack.modules[0].file.custom_data = Some(sf2d.write(None).unwrap());
            song
        };
        let mut samples = crate::sample_cache::SampleCache::default();
        let font = include_bytes!("../../crates/audio/tests/fixtures/sine.sf2");
        let held = [note(0.0, 8.0, 69)];
        samples.load(&patched(0, &held), &|_| Some(font.to_vec()));
        let engine = |song: &Song| -> SongAudioEngine {
            SongAudioEngine::with_samples(song, 48_000.0, &samples).unwrap()
        };
        let mut before = engine(&patched(0, &held));
        before.play();
        assert!(
            energy(&render(&mut before, 1.0)) > 0.01,
            "a silent SoundFont"
        );
        let mut new = engine(&patched(1, &held)).adopting_edit(false, Vec::new());
        new.take_over(&mut before);
        assert_eq!(
            energy(&render(&mut new, 0.5)),
            0.0,
            "the old patch plays on"
        );
        let notes = [note(0.0, 0.5, 69), note(4.0, 1.0, 69)];
        let mut before = engine(&patched(0, &notes));
        before.play();
        render(&mut before, 1.0);
        let mut new = engine(&patched(1, &notes)).adopting_edit(false, Vec::new());
        new.take_over(&mut before);
        render(&mut new, 3.0);
        let mut heard = engine(&patched(1, &notes));
        heard.play();
        render(&mut heard, 4.0);
        assert_close(&render(&mut new, 1.0), &render(&mut heard, 1.0));
    }

    /// A GMSynth preset of its type keeps the running synth and gives it the preset's settings (its waves).
    #[test]
    fn a_gmsynth_preset_with_other_waves_is_heard() {
        use honeydew_audio::gmsynth::WAVE_A;
        use std::collections::HashMap;
        let waved = |wave: f32| {
            let mut song = two_notes(SynthKind::GmSynth, 69);
            let settings = HashMap::from([(WAVE_A, wave)]);
            let blob = crate::panels::cstm::cstm_from_settings(2, &settings, None);
            song.channels[1].rack.modules[0].file.custom_data = blob;
            song
        };
        heard_from_beat_4(&waved(18.0), &waved(16.0));
    }

    /// A GMSynth whose blob goes away (its `Default` preset, an undo to a module without one) keeps the running synth,
    /// which takes a new module's settings: it sounds as a fresh build of the edited song.
    #[test]
    fn a_gmsynth_whose_blob_goes_away_is_heard_as_a_new_one() {
        use honeydew_audio::gmsynth::WAVE_A;
        use std::collections::HashMap;
        let mut waved = two_notes(SynthKind::GmSynth, 69);
        let blob =
            crate::panels::cstm::cstm_from_settings(2, &HashMap::from([(WAVE_A, 16.0)]), None);
        waved.channels[1].rack.modules[0].file.custom_data = blob;
        let mut default = waved.clone();
        default.channels[1].rack.modules[0].file.custom_data = None;
        heard_from_beat_4(&waved, &default);
        // A blob the `CSTM` reader rejects (under its 8 required bytes) is read as none, taken over or built afresh.
        let mut rejected = waved.clone();
        rejected.channels[1].rack.modules[0].file.custom_data = Some(vec![16, 0, 0, 0]);
        heard_from_beat_4(&waved, &rejected);
    }

    /// A channel param the song changes (Pitch Bend) reaches the instrument taken over.
    #[test]
    fn a_song_changed_bend_reaches_the_instrument_taken_over() {
        let before = two_notes(SynthKind::MiniSynth, 69);
        let mut after = before.clone();
        after.channels[1].params.0[4] = 0.75;
        heard_from_beat_4(&before, &after);
    }

    /// An edit without a re-sync while rolling leaves an automated param at its automation's value.
    #[test]
    fn an_automated_param_keeps_its_automation_value_through_an_edit() {
        use crate::song::AutomationTarget;
        let mut song = two_notes(SynthKind::MiniSynth, 69);
        song.channels[1].lanes[0].clips[0]
            .events
            .insert(1, note(0.5, 8.0, 57));
        let lane = song.channels[1].add_automation_lane(AutomationTarget {
            instance_id: 0,
            param: 0,
        });
        lane.add_clip(0, 16.0).events.push(Event::Automation {
            pos: 0,
            value: 0.2,
            control: true,
        });
        let mut after = song.clone();
        after.channels[2].rack.add(32);
        let mut old = SongAudioEngine::new(&song, 48_000.0).unwrap();
        old.play();
        render(&mut old, 1.0);
        let mut new = SongAudioEngine::new(&after, 48_000.0)
            .unwrap()
            .adopting_edit(false, Vec::new());
        new.take_over(&mut old);
        let mut heard = SongAudioEngine::new(&song, 48_000.0).unwrap();
        heard.play();
        render(&mut heard, 1.0);
        assert_close(&render(&mut new, 1.0), &render(&mut heard, 1.0));
    }

    /// two instruments of one type on a channel each keep their own voices and params.
    #[test]
    fn two_instruments_of_one_type_keep_their_own_voices() {
        let mut song = two_notes(SynthKind::MiniSynth, 69);
        song.channels[1].lanes[0].clips[0]
            .events
            .insert(1, note(0.5, 8.0, 57));
        let id = song.channels[1].rack.add(0);
        let second = song.channels[1].rack.get_mut(id).unwrap();
        second.params[2] = 0.75;
        second.params[9] = 0.3;
        let mut after = song.clone();
        after.channels[2].rack.add(32);
        let mut old = SongAudioEngine::new(&song, 48_000.0).unwrap();
        old.play();
        render(&mut old, 1.0);
        let mut new = SongAudioEngine::new(&after, 48_000.0)
            .unwrap()
            .adopting_edit(false, Vec::new());
        new.take_over(&mut old);
        let mut heard = SongAudioEngine::new(&song, 48_000.0).unwrap();
        heard.play();
        render(&mut heard, 1.0);
        assert_close(&render(&mut new, 1.0), &render(&mut heard, 1.0));
    }

    /// An effect of another type at the same instance id is a new one: the old one's tail ends.
    #[test]
    fn an_effect_of_another_type_at_its_id_ends_the_tail() {
        let mut song = two_notes(SynthKind::MiniSynth, 69);
        let id = song.channels[1].rack.add(18);
        let mut after = song.clone();
        *after.channels[1].rack.get_mut(id).unwrap() = ModuleInstance::new(32, id);
        let mut old = SongAudioEngine::new(&song, 48_000.0).unwrap();
        old.play();
        render(&mut old, 1.0);
        assert!(energy(&render(&mut old, 0.25)) > 0.0, "no tail");
        let mut new = SongAudioEngine::new(&after, 48_000.0)
            .unwrap()
            .adopting_edit(false, Vec::new());
        new.take_over(&mut old);
        assert_eq!(energy(&render(&mut new, 1.0)), 0.0);
    }

    /// another instrument type at the same instance id is a new instrument, silent.
    #[test]
    fn another_instrument_type_at_its_id_starts_silent() {
        let mut song = two_notes(SynthKind::MiniSynth, 69);
        song.channels[1].lanes[0].clips[0]
            .events
            .insert(1, note(0.5, 8.0, 57));
        let mut after = song.clone();
        after.channels[1].rack.modules[0] = ModuleInstance::new(23, 2);
        let mut old = SongAudioEngine::new(&song, 48_000.0).unwrap();
        old.play();
        render(&mut old, 1.0);
        let mut new = SongAudioEngine::new(&after, 48_000.0)
            .unwrap()
            .adopting_edit(false, Vec::new());
        new.take_over(&mut old);
        assert_eq!(energy(&render(&mut new, 1.0)), 0.0);
    }

    /// The note-offs a held sustain pedal (an automated CC #64) caught come over, and its lift releases them.
    #[test]
    fn the_note_offs_a_pedal_holds_are_released_when_it_lifts_after_an_edit() {
        use crate::song::AutomationTarget;
        let mut song = two_notes(SynthKind::MiniSynth, 69);
        let lane = song.channels[1].add_automation_lane(AutomationTarget {
            instance_id: 0,
            param: 71,
        });
        let points = [(0.0, 1.0), (2.9, 1.0), (3.0, 0.0)];
        lane.add_clip(0, 16.0)
            .events
            .extend(points.map(|(beat, value)| Event::Automation {
                pos: (beat * 128.0) as u32,
                value,
                control: true,
            }));
        let mut after = song.clone();
        after.channels[2].rack.add(32);
        let mut old = SongAudioEngine::new(&song, 48_000.0).unwrap();
        old.play();
        render(&mut old, 1.0);
        let mut new = SongAudioEngine::new(&after, 48_000.0)
            .unwrap()
            .adopting_edit(false, Vec::new());
        new.take_over(&mut old);
        let mut heard = SongAudioEngine::new(&song, 48_000.0).unwrap();
        heard.play();
        render(&mut heard, 1.0);
        assert_close(&render(&mut new, 3.5), &render(&mut heard, 3.5));
    }

    /// The instrument taken over plays from its place in the new rack (an effect moved above it no longer
    /// processes it) and hears only its new key range.
    #[test]
    fn an_instrument_taken_over_takes_its_rack_place_and_key_range() {
        let mut song = two_notes(SynthKind::MiniSynth, 69);
        let filter = song.channels[1].rack.add(9);
        song.channels[1].rack.get_mut(filter).unwrap().params[1] = 0.2;
        let mut moved = song.clone();
        moved.channels[1].rack.move_up(filter);
        heard_from_beat_4(&song, &moved);
        // E3 at beats 0 and 4, A4 at beat 4 too: a range up to C4 drops A4.
        let mut ranged = two_notes(SynthKind::MiniSynth, 52);
        (ranged.channels[1].lanes[0].clips[0].events).push(note(4.0, 1.0, 69));
        let mut narrow = ranged.clone();
        narrow.channels[1].rack.modules[0].file.key_high = 60;
        heard_from_beat_4(&ranged, &narrow);
    }

    /// An audio clip playing its `STR1` render while it streams in plays on when the whole render lands and the next
    /// engine plays it (the same data: the stream's length and rate), uncut.
    #[test]
    fn a_clip_playing_a_stream_plays_on_when_its_render_lands() {
        use crate::sample_cache::SampleCache;
        use crate::song::SamplerSlot;
        let mut slot = SamplerSlot {
            folder: "Built-in/Kick".into(),
            ..SamplerSlot::default()
        };
        (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 2.0);
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::AudioClip);
        song.channels[1].lanes[0].add_clip(0, 4.0).file.sample = Some(Box::new(slot));
        let mut samples = SampleCache::default();
        samples.load_live(&song, &song, &|_| None);
        let mut jobs = samples.take_jobs();
        assert_eq!(jobs.len(), 1, "a render streaming in");
        let engine = |samples: &SampleCache| {
            let mut engine = SongAudioEngine::with_samples(&song, 48_000.0, samples).unwrap();
            engine.play();
            render(&mut engine, 0.1);
            engine
        };
        let (mut old, mut heard) = (engine(&samples), engine(&samples));
        assert!(samples.put_stretch(jobs.pop().unwrap().render()));
        let new = SongAudioEngine::with_samples(&song, 48_000.0, &samples).unwrap();
        let mut new = new.adopting_edit(false, Vec::new());
        new.take_over(&mut old);
        assert_close(&render(&mut new, 0.1), &render(&mut heard, 0.1));
    }

    /// CC #1, which automation sets to 1.0 from beat 2 (the song holds no CC), keeps the playing engine's value through
    /// An edit's take-over, whatever set it, stopped or before the lane's point; only a re-sync's automation at the
    /// playhead sets it. The lane is on channel 2, which a deleted channel 1 moves to index 1; a channel the edit adds
    /// with such a lane takes its automation at the playhead.
    #[test]
    fn a_cc_keeps_its_live_value_through_an_edit() {
        use crate::song::{AutomationTarget, PPQ};
        let lane = |song: &mut Song, channel: usize| {
            let target = AutomationTarget {
                instance_id: 0,
                param: 8,
            };
            let clip = song.channels[channel]
                .add_automation_lane(target)
                .add_clip(2 * PPQ, 1.0);
            clip.events.push(Event::Automation {
                pos: 0,
                value: 1.0,
                control: true,
            });
        };
        let mut song = two_notes(SynthKind::MiniSynth, 69);
        lane(&mut song, 2);
        let mut edited = song.clone();
        edited.channels[1].rack.add(32);
        let mut deleted = song.clone();
        deleted.delete_channel(1);
        let mut added = song.clone();
        added.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        lane(&mut added, 3);
        // Played to beat 2.5 (with a live CC 0.2, which the link keeps, sent at beat `live`), then a seek to beat 1 if
        // `back`, Stop if `stop`, and the edit, re-syncing if `resync`: CC #1 of `channel`.
        let cases = [
            (&edited, 2, None, false, true, false, 1.0),
            (&edited, 2, None, false, true, true, 1.0),
            (&edited, 2, None, true, false, false, 1.0),
            (&edited, 2, None, true, false, true, 1.0),
            (&edited, 2, None, false, false, false, 1.0),
            (&edited, 2, Some(2.5), false, false, false, 0.2),
            (&edited, 2, Some(1.0), false, true, false, 1.0),
            (&edited, 2, Some(2.5), false, false, true, 1.0),
            (&deleted, 1, None, false, true, false, 1.0),
            (&added, 3, None, false, false, false, 1.0),
        ];
        let mut wrong = Vec::new();
        for (case, &(after, channel, live, back, stop, resync, expected)) in
            cases.iter().enumerate()
        {
            let mut old = SongAudioEngine::new(&song, 48_000.0).unwrap();
            old.play();
            let mut new = SongAudioEngine::new(after, 48_000.0).unwrap();
            let at = live.unwrap_or(2.5);
            render(&mut old, at);
            if live.is_some() {
                old.set_param(2, 0, 8, 0.2);
                new.set_param(channel, 0, 8, 0.2);
            }
            render(&mut old, 2.5 - at);
            if back {
                old.seek(u64::from(PPQ));
            }
            if stop {
                old.stop();
            }
            let mut new = new.adopting_edit(resync, Vec::new());
            new.take_over(&mut old);
            let heard = new.audio.channel_parameter(channel, 8);
            if heard != Some(expected) {
                wrong.push((case, heard));
            }
        }
        assert_eq!(wrong, []);
    }
}

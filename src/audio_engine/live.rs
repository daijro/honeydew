//! Live keys: keyboard, MIDI and pad notes enter the channel's note path. On a notes lane they are routed exactly like
//! sequenced notes: the pads get them as played, each instrument through the note effects above it. Off one (drums,
//! automation, audio) they go once, after every note effect, to the pads and every instrument (there is no
//! second, per-instrument copy). The note effects run in real time ([`LiveNoteChain`]).

use honeydew_audio::instrument::InstrumentModule;
use honeydew_audio::note_effects::{LiveNoteChain, LiveNoteEvent, PADS};

use super::prepare::{KIT, note_effect, song_end_beat};
use super::{EngineError, SongAudioEngine};
use crate::song::{LaneKind, Song};

/// Live notes in flight at once on a channel with note effects (an echo makes many of one key), and without.
const CAPACITY: usize = 4096;
const PLAIN_CAPACITY: usize = 256;

/// A channel's live keys: its note effects in real time, and per instrument the note effects above it.
pub(super) struct Live {
    chain: LiveNoteChain,
    stages: Vec<usize>,
    /// The channel's current lane is a notes lane; until the app says, its first lane.
    pub notes_lane: bool,
}

/// The live path of `channel`, whose `instruments` the engine plays; `None` when nothing on it plays a note.
pub(super) fn build(
    song: &Song,
    channel: usize,
    instruments: &[(i32, InstrumentModule)],
    sample_rate: f32,
    tempo_bpm: f32,
) -> Result<Option<Live>, EngineError> {
    let kit = song.channels[channel].drum_kit.as_ref();
    if instruments.is_empty() && kit.is_none_or(|kit| kit.slots.is_empty()) {
        return Ok(None);
    }
    let end_beat = song_end_beat(song)?;
    let mut effects = Vec::new();
    let mut stages = vec![0; instruments.len()];
    for module in &song.channels[channel].rack.modules {
        if (47..=51).contains(&module.type_id) {
            effects.push(note_effect(song, channel, module, end_beat)?);
        } else if let Some(at) = instruments
            .iter()
            .position(|(id, _)| *id == module.instance_id)
        {
            stages[at] = effects.len();
        }
    }
    let capacity = if effects.is_empty() {
        PLAIN_CAPACITY
    } else {
        CAPACITY
    };
    let chain = LiveNoteChain::new(
        effects,
        f64::from(song.time_sig.bar_beats()),
        f64::from(sample_rate),
        f64::from(tempo_bpm),
        capacity,
    )
    .map_err(EngineError::Notes)?;
    let lanes = &song.channels[channel].lanes;
    Ok(Some(Live {
        chain,
        stages,
        notes_lane: lanes
            .first()
            .is_some_and(|lane| lane.kind == LaneKind::Notes),
    }))
}

impl Live {
    /// Takes over `old`, the live path of the same channel in the engine this one replaces, at frame `now` (an edit
    /// while playing): its notes in flight, each note effect's state and its current lane go on here
    /// ([`LiveNoteChain::take_over`]), with this song's note effects, params, automation and tempo; `old` keeps the
    /// chain built here. Nothing is released: a note sent before the edit ends on the instrument rows its note-on
    /// reached (`old_rows`, by instance id among `rows`, this channel's), whatever note effect was added, removed or
    /// moved, or instrument moved; a row whose instrument is gone stays in `old`'s engine. Allocates nothing.
    pub(super) fn adopt(
        &mut self,
        old: &mut Live,
        rows: &[(i32, InstrumentModule)],
        old_rows: &[(i32, InstrumentModule)],
        now: u64,
    ) {
        let stages = &old.stages;
        self.chain
            .take_over(&mut old.chain, now, |route, level, notes_lane| {
                // Its rows in `old`'s engine: an earlier edit's route, else its level there, as `send_live` routes.
                let before = route.unwrap_or_else(|| {
                    let pads = u128::from(!notes_lane || level == 0) << PADS;
                    let reached = |&row: &usize| !notes_lane || stages[row] == level;
                    (0..stages.len().min(PADS))
                        .filter(reached)
                        .fold(pads, |rows, row| rows | 1 << row)
                });
                let here = |row: usize| {
                    let id = old_rows.get(row)?.0;
                    rows.iter()
                        .position(|(other, _)| *other == id)
                        .filter(|&at| at < PADS)
                };
                let (mut moved, mut left) = (before & 1 << PADS, before & !(1 << PADS));
                while left != 0 {
                    let row = left.trailing_zeros() as usize;
                    left &= left - 1;
                    moved |= here(row).map_or(0, |at| 1 << at);
                }
                moved
            });
        self.notes_lane = old.notes_lane;
    }

    /// Forgets its notes in flight: all notes off on its channel alone.
    pub(super) fn clear(&mut self) {
        self.chain.clear();
    }
}

impl SongAudioEngine {
    /// The live frame now: frames rendered, `cursor` into this render call.
    fn live_frame(&self, cursor: usize) -> u64 {
        self.rendered + cursor as u64
    }

    /// A live tempo (a drag, `AudioRenderer::set_tempo`) reaches the live keys' note effects from the next frame.
    pub(super) fn set_live_tempo(&mut self, bpm: f32) {
        let now = self.live_frame(0);
        for live in self.live.iter_mut().flatten() {
            live.chain.set_tempo(now, f64::from(bpm));
        }
    }

    /// Key `key` goes down on `channel`.
    pub(super) fn live_key_down(&mut self, channel: usize, key: u8, velocity: f32) {
        let now = self.live_frame(0);
        if let Some(Some(live)) = self.live.get_mut(channel) {
            live.chain.note_on(now, key, velocity, live.notes_lane);
            super::set_key(&mut self.sounding[channel], key, true);
        }
    }

    /// Key `key` lifts on `channel`. One that is not down (Play cut it) changes nothing: its key stays as the song
    /// lights it (a second note-off, with no effect).
    pub(super) fn live_key_up(&mut self, channel: usize, key: u8) {
        let now = self.live_frame(0);
        let bit = 1_u128.checked_shl(u32::from(key)).unwrap_or(0);
        if let Some(Some(live)) = self.live.get_mut(channel)
            && live.chain.keys_down() & bit != 0
        {
            live.chain.note_off(now, key);
            super::set_key(&mut self.sounding[channel], key, false);
        }
    }

    /// Sends every live note on and off due at `cursor` into the render call.
    pub(super) fn live_due(&mut self, cursor: usize) {
        let now = self.live_frame(cursor);
        let song_beat = self.beat();
        for channel in 0..self.live.len() {
            let Some(live) = &mut self.live[channel] else {
                continue;
            };
            self.live_out.clear();
            live.chain.due(now, song_beat, &mut self.live_out);
            for index in 0..self.live_out.len() {
                let event = self.live_out[index];
                self.send_live(channel, event);
            }
        }
    }

    /// Frames from `cursor` into the render call to the next live note due, if any.
    pub(super) fn live_next(&self, cursor: usize) -> Option<u64> {
        let now = self.live_frame(cursor);
        let next = self
            .live
            .iter()
            .flatten()
            .filter_map(|live| live.chain.next_due());
        next.min().map(|next| next.saturating_sub(now))
    }

    /// Every live note in flight is forgotten: all notes off (Stop, `AllNotesOff`). A position re-sync leaves live keys
    /// alone. `queued`: the keys pressed and not yet sent stay queued, and lit (Play start).
    pub(super) fn clear_live(&mut self, queued: bool) {
        for (live, lit) in self.live.iter_mut().zip(&mut self.sounding) {
            match live {
                Some(live) if queued => {
                    live.chain.clear_sent();
                    *lit |= live.chain.keys_down();
                }
                Some(live) => live.chain.clear(),
                None => {}
            }
        }
    }

    /// `event` to the pads and instruments of `channel` it is for.
    fn send_live(&mut self, channel: usize, event: LiveNoteEvent) {
        let LiveNoteEvent {
            level,
            notes_lane,
            on,
            key,
            velocity,
            pan,
            route,
        } = event;
        // A note-off of a note an edit took over: to the rows its note-on reached.
        if let Some(rows) = route {
            if rows >> PADS & 1 == 1 {
                self.key_up(channel, Some(KIT), key);
            }
            for row in (0..self.instruments[channel].len().min(PADS)).filter(|r| rows >> r & 1 == 1)
            {
                let id = self.instruments[channel][row].0;
                self.key_up(channel, Some(id), key);
            }
            return;
        }
        if !notes_lane || level == 0 {
            match on {
                true => {
                    if let Some(kit) = self.kit(channel) {
                        kit.note_on(key, velocity);
                    }
                }
                false => self.key_up(channel, Some(KIT), key),
            }
        }
        // An instrument hears the key only inside its key range and while it is not learning one; the note-offs check
        // The same in `key_up`.
        for position in 0..self.instruments[channel].len() {
            let stage = self.live[channel]
                .as_ref()
                .map(|live| live.stages[position]);
            if notes_lane && stage != Some(level) {
                continue;
            }
            let (id, module) = &mut self.instruments[channel][position];
            match on {
                true if module.hears(key) => module.instrument.note_on(key, velocity, pan),
                true => {}
                false => {
                    let id = *id;
                    self.key_up(channel, Some(id), key);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use honeydew_audio::device::AudioRenderer;

    use super::*;
    use crate::song::{ChannelKind, SynthKind};

    /// A dragged tempo reaches the live keys' note effects from the next block. A key down at the drag's 240 BPM
    /// through a Note Echo of one repeat a beat later (Sync on) has it due 12 000 frames on, not the build's 24 000.
    #[test]
    fn a_dragged_tempo_times_the_live_note_echo() {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let echo = song.channels[1].rack.insert(0, 47);
        let params = &mut song.channels[1].rack.get_mut(echo).unwrap().params;
        // One repeat; `Delay (sync)` at the item nearest b = 1 beat.
        (params[0], params[1], params[4]) = (1.0, 1.0 / 32.0, 1.0);
        params[3] = (0.96875_f32 / 7.96875).sqrt();
        let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
        let mut output = vec![0.0; 64 * 2];
        engine.render(&mut output, 2);
        engine.set_tempo(240.0);
        engine.note_on(1, 60, 0.8);
        engine.render(&mut output, 2);
        let live = engine.live[1].as_ref().unwrap();
        assert_eq!(live.chain.next_due(), Some(64 + 12_000));
    }
}

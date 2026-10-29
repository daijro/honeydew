//! A tempo change and the audio clips.

use super::{AutomationTarget, ChannelKind, LaneKind, Song};

/// The tempos the app sets, in BPM: TAP's and the MIDI import's clamp,
/// the widest. A read song's tempo is clamped to it too; the tempo dial's is narrower
/// ([`crate::ui::main_menu::TEMPO_RANGE`]).
pub const TEMPO_RANGE: (f64, f64) = (1.0, 999.0);

/// The master channel's param an automation lane moves the tempo with (Honeydew's own: FL Studio Mobile automates no
/// tempo, and channel param 6 is none of its). Its value v plays [`automated_tempo`]; files keep such a lane apart
/// (`flm::write`).
pub const TEMPO_PARAM: i32 = 6;
/// [`TEMPO_PARAM`] as a lane's target.
pub const TEMPO_TARGET: AutomationTarget = AutomationTarget {
    instance_id: 0,
    param: TEMPO_PARAM,
};
/// The tempos a tempo automation spans, the tempo dial's.
pub const AUTOMATION_TEMPO_RANGE: (f64, f64) = (10.0, 999.0);

/// The tempo automation value `v` plays: linear over [`AUTOMATION_TEMPO_RANGE`].
pub fn automated_tempo(v: f32) -> f64 {
    let (lo, hi) = AUTOMATION_TEMPO_RANGE;
    lo + f64::from(v.clamp(0.0, 1.0)) * (hi - lo)
}

/// The tempo automation value of `bpm`, [`automated_tempo`]'s inverse, clamped to its range.
pub fn tempo_value(bpm: f64) -> f32 {
    let (lo, hi) = AUTOMATION_TEMPO_RANGE;
    ((bpm - lo) / (hi - lo)).clamp(0.0, 1.0) as f32
}

impl Song {
    /// Whether channel `channel` with automation target `target` moves the tempo: the master's [`TEMPO_TARGET`].
    pub fn is_tempo(&self, channel: usize, target: AutomationTarget) -> bool {
        target == TEMPO_TARGET
            && self
                .channels
                .get(channel)
                .is_some_and(|c| c.kind == ChannelKind::Master)
    }

    /// Sets the tempo to `bpm`; every audio clip's stretch ratio (`STR1`, SYNC on or off) is multiplied by
    /// old / new, so the next render keeps a SYNC-on clip's length in beats and a SYNC-off clip plays at its native
    /// speed (its source length in beats, `CLHd` @28, scales with the tempo instead). Pads and other channels' samples
    /// keep theirs. `bpm` that is no tempo (not finite, or ≤ 0) is ignored; an old tempo that is none sets the
    /// tempo but scales nothing.
    ///
    /// Called on every frame of a tempo drag: the factors multiply out to old / new, and the error of 100 of them is
    /// ~1e-15, not worth a gesture-start snapshot (`tempo::tests`).
    pub fn set_tempo(&mut self, bpm: f64) {
        if !(bpm.is_finite() && bpm > 0.0) {
            return;
        }
        let factor = self.tempo_bpm / bpm;
        self.tempo_bpm = bpm;
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let audio = (self.channels.iter_mut())
            .flat_map(|c| &mut c.lanes)
            .filter(|lane| lane.kind == LaneKind::Audio);
        for (slot, source_len) in audio.flat_map(|lane| &mut lane.clips).filter_map(|clip| {
            Some((
                clip.file.sample.as_deref_mut()?,
                &mut clip.file.source_len_beats,
            ))
        }) {
            slot.file.stretch_ratio *= factor;
            // SYNC off plays at native speed: the source's length in beats (CLHd @28) scales with the tempo. SYNC
            // on keeps it. A zero stays zero. The loop length (@8) is refit on the clip's next update.
            if slot.file.stretch_mode == 0 {
                *source_len /= factor;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ChannelKind, SamplerSlot};
    use super::*;
    use crate::history::History;

    /// A slot with `STR1` mode `mode` and stretch ratio `ratio`.
    fn slot(mode: u8, ratio: f64) -> Box<SamplerSlot> {
        let mut slot = SamplerSlot::default();
        slot.file.stretch_mode = mode;
        slot.file.stretch_ratio = ratio;
        slot.file.pitch_semitones = 2.0;
        Box::new(slot)
    }

    /// 140 BPM; channel 1 an Audio Clip with two clips (SYNC on at 1.5, off at 0.8), channel 2 Drums whose first
    /// pad is stretched, channel 3 an Instrument whose clip holds a stray sample.
    fn song() -> Song {
        let mut song = Song {
            tempo_bpm: 140.0,
            ..Song::default()
        };
        for kind in [
            ChannelKind::AudioClip,
            ChannelKind::Drums,
            ChannelKind::Instrument,
        ] {
            song.add_channel(kind);
        }
        let lane = &mut song.channels[1].lanes[0];
        assert_eq!(lane.kind, LaneKind::Audio);
        lane.add_clip(0, 4.0).file.sample = Some(slot(1, 1.5));
        lane.add_clip(512, 4.0).file.sample = Some(slot(0, 0.8));
        let pad = &mut song.channels[2].drum_kit.as_mut().unwrap().slots[0];
        (pad.file.stretch_mode, pad.file.stretch_ratio) = (1, 2.0);
        song.channels[3].lanes[0].add_clip(0, 4.0).file.sample = Some(slot(1, 3.0));
        song
    }

    /// The (mode, ratio) of the two audio clips' samples.
    fn audio(song: &Song) -> Vec<(u8, f64)> {
        (song.channels[1].lanes[0].clips.iter())
            .map(|c| &c.file.sample.as_ref().unwrap().file)
            .map(|f| (f.stretch_mode, f.stretch_ratio))
            .collect()
    }

    #[test]
    fn a_tempo_change_scales_audio_clip_ratios_by_old_over_new_sync_on_or_off() {
        let mut song = song();
        song.set_tempo(120.0);
        assert_eq!(song.tempo_bpm, 120.0);
        let want = |r: f64| r * 140.0 / 120.0;
        let got = audio(&song);
        assert_eq!(got[0].0, 1);
        assert!((got[0].1 - want(1.5)).abs() < 1e-12, "{got:?}");
        assert_eq!(got[1].0, 0, "SYNC stays off");
        assert!((got[1].1 - want(0.8)).abs() < 1e-12, "{got:?}");
        // Pitch and the mode are theirs alone.
        let clip = &song.channels[1].lanes[0].clips[0];
        assert_eq!(clip.file.sample.as_ref().unwrap().file.pitch_semitones, 2.0);
        // Slower: ratios shrink... and back up.
        song.set_tempo(140.0);
        assert!((audio(&song)[0].1 - 1.5).abs() < 1e-12);
    }

    #[test]
    fn a_sync_off_clips_source_length_in_beats_follows_the_tempo_and_a_sync_on_clips_keeps_it() {
        let mut song = song();
        let lane = &mut song.channels[1].lanes[0];
        // Clip 1 is SYNC on, clip 2 SYNC off; a third SYNC-off clip has no source length.
        lane.clips[0].file.source_len_beats = 4.0;
        lane.clips[1].file.source_len_beats = 4.0;
        lane.add_clip(1024, 4.0).file.sample = Some(slot(0, 1.0));
        let lens = |song: &Song| {
            (song.channels[1].lanes[0].clips.iter())
                .map(|c| c.file.source_len_beats)
                .collect::<Vec<_>>()
        };
        song.set_tempo(70.0);
        assert_eq!(lens(&song), [4.0, 2.0, 0.0]);
        // A clip's loop length is refit on its next update, not here.
        assert_eq!(song.channels[1].lanes[0].clips[1].loop_len_beats, 4.0);
        song.set_tempo(140.0);
        assert_eq!(lens(&song), [4.0, 4.0, 0.0]);
    }

    #[test]
    fn pads_and_other_channels_keep_their_ratios() {
        let mut song = song();
        let before = song.clone();
        song.set_tempo(70.0);
        assert_eq!(song.channels[0], before.channels[0]);
        assert_eq!(song.channels[2], before.channels[2]);
        assert_eq!(song.channels[3], before.channels[3]);
        assert_ne!(song.channels[1], before.channels[1]);
    }

    #[test]
    fn a_tempo_that_is_no_tempo_changes_nothing_and_a_broken_old_one_scales_nothing() {
        let before = song();
        for bad in [0.0, -120.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut song = before.clone();
            song.set_tempo(bad);
            assert!(song == before, "{bad}");
        }
        for old in [0.0, f64::NAN, f64::INFINITY] {
            let mut song = before.clone();
            song.tempo_bpm = old;
            song.set_tempo(120.0);
            assert_eq!(song.tempo_bpm, 120.0);
            assert_eq!(audio(&song), audio(&before), "{old}");
        }
    }

    #[test]
    fn undo_restores_the_tempo_and_the_ratios_in_one_step() {
        let mut song = song();
        let before = song.clone();
        let mut history = History::new(&song);
        // A drag: ongoing frames, then the done one.
        for bpm in [139.0, 130.0, 118.0, 118.0] {
            song.set_tempo(bpm);
            history.edited(&song, bpm == 118.0);
        }
        assert_ne!(audio(&song), audio(&before));
        let after = song.clone();
        assert!(history.undo().as_ref() == Some(&before));
        assert!(history.redo().as_ref() == Some(&after));
        assert!(history.undo().as_ref() == Some(&before));
        assert!(history.undo().is_none());
    }

    #[test]
    fn a_hundred_drag_frames_keep_ratio_times_tempo_and_the_way_back_within_1e9() {
        let mut song = song();
        let start = audio(&song);
        let mut worst = 0.0_f64;
        for k in 1..=100 {
            let bpm = 140.0 + 60.0 * (f64::from(k) * 0.37).sin() * (f64::from(k) / 100.0);
            song.set_tempo(bpm);
            for (&(_, r0), (_, r)) in start.iter().zip(audio(&song)) {
                // ratio × tempo is what a tempo change keeps.
                worst = worst.max((r * bpm / (r0 * 140.0) - 1.0).abs());
            }
        }
        song.set_tempo(140.0);
        for (&(_, r0), (_, r)) in start.iter().zip(audio(&song)) {
            worst = worst.max((r / r0 - 1.0).abs());
        }
        assert!(worst < 1e-9, "{worst:e}");
    }
}

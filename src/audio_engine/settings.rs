//! Module settings in the engine: what a module's `CSTM` holds and what its card edits live, by the panel `#n` keys of
//! [`settings_from_cstm`]. A build reads every module's blob through it; a live edit arrives as `SetSetting` with the
//! same key. Instruments take theirs through `Instrument::set_setting`; of the effects, the Pitcher takes its note
//! enables, the Multiband its soloed band and the Analyzer the settings its analysis reads. The few a build derives
//! app-thread data from rebuild the engine instead ([`rebuilds`]).

use std::collections::HashMap;

use honeydew_audio::AudioEffect;
use honeydew_audio::analysis::analyzer::Setting;
use honeydew_audio::gmsynth;

use crate::panels::cstm::{
    DIRECTWAVE, Field, MULTIBAND_SOLO_BAND, PITCHER, SLICER, cstm_from_settings, settings_from_cstm,
};
use crate::song::Song;

/// A drum slot's `SetSetting` keys (instance −slot id), which the strip's `REV`, `Cut...` and `Poly` change live: its
/// `MAIN` reverse (@1036) and overlap (@1050) flags, on above 0, and its cut group (@1049).
pub const SLOT_REVERSE: u32 = 1036;
pub const SLOT_CUT_GROUP: u32 = 1049;
pub const SLOT_OVERLAP: u32 = 1050;

/// A setting the engine reads only at build to derive app-thread data: the DW Sampler's zones
/// from its `PATh`, `ZONE` and `MIDI` blob, and the Slicer's slices from Sensitivity. An edit rebuilds the
/// engine instead of sending `SetSetting`; every other setting is live, GMSynth's waveA and waveB included
/// ([`prepare`]).
pub fn rebuilds(type_id: u16, key: u32) -> bool {
    let fields: &[Field] = match type_id {
        1 => DIRECTWAVE,
        33 => SLICER,
        _ => &[],
    };
    fields.iter().any(|field| field.n == key)
}

/// App thread: builds what a live `SetSetting` of `key` = `value` on a `type_id` module needs, before it is sent. A
/// GMSynth's waveA and waveB are shape tables, built once per process (2 MB, a few ms) so the audio thread only takes
/// the built one (`gmsynth::prepare_setting`).
pub fn prepare(type_id: u16, key: u32, value: f32) {
    if type_id == 2 {
        gmsynth::prepare_setting(key, value);
    }
}

/// A setting that is on the screen and in the engine but not in the song (a Multiband's soloed band, a module's
/// key-learn: UI state): a new engine starts without it, so `AudioLink` sends it again.
pub fn screen_only(type_id: u16, key: u32) -> bool {
    (type_id == 36 && key == MULTIBAND_SOLO_BAND) || key == honeydew_audio::instrument::KEY_LEARN
}

/// Every module's saved settings as `(channel, instance_id, key, value)`, each module's in key order: what a build
/// applies. A module without a blob, or with one the `CSTM` reader rejects, has a new module's (its new blob's), so one
/// An edit's engine takes over from a module that had others (a `Default` preset, an undo) takes them, as a fresh build
/// has them.
pub(super) fn saved(song: &Song) -> Vec<(usize, i32, u32, f32)> {
    let mut out = Vec::new();
    for (channel, ch) in song.channels.iter().enumerate() {
        for module in &ch.rack.modules {
            let read = |blob: &[u8]| settings_from_cstm(module.type_id, blob);
            let new = || cstm_from_settings(module.type_id, &HashMap::new(), None);
            let blob = module.file.custom_data.as_deref();
            let Some(settings) = blob.and_then(read).or_else(|| read(&new()?)) else {
                continue;
            };
            let mut settings: Vec<(u32, f32)> = settings.into_iter().collect();
            settings.sort_by_key(|&(key, _)| key);
            out.extend(
                settings
                    .into_iter()
                    .map(|(key, value)| (channel, module.instance_id, key, value)),
            );
        }
    }
    out
}

/// Setting `key` = `value` on `effect`; a key the effect does not take is ignored. Real-time safe.
/// - Pitcher (25): a note button C…B (#5–#16, on from 0.5) sets that note enable. Scale (#20) and Key (#21) only set
///   The panel's spinners; the buttons they rewrite arrive as their own keys.
/// - Multiband (36): [`MULTIBAND_SOLO_BAND`] solos band k − 1 for k = 1…3, none for 0.
/// - Analyzer (26): `Channel` (#14), `Reactivity` (#15) and `DC Block` (#16, On is item 0) by item, `Input Gain` (#17)
///   as ×10^(2v − 1), LOUDNESS `Mode` (#8) by item, which resets the meter when it changes. Its other settings are the
///   panel's alone.
pub(super) fn apply(effect: &mut AudioEffect, key: u32, value: f32) {
    match effect.type_id() {
        25 => {
            let note = PITCHER[..12].iter().position(|field| field.n == key);
            if let (Some(note), Some(mut notes)) = (note, effect.pitcher_notes()) {
                notes[note] = value >= 0.5;
                let _ = effect.set_pitcher_notes(notes);
            }
        }
        26 => {
            // The item a spinner's setting holds.
            let item = value.max(0.0).round() as usize;
            let setting = match key {
                14 => Setting::Channel(item),
                15 => Setting::Reactivity(item),
                16 => Setting::DcBlock(item == 0),
                17 => Setting::InputGain(10f32.powf(2.0 * value.clamp(0.0, 1.0) - 1.0)),
                8 => Setting::LoudnessMode(item),
                _ => return,
            };
            let _ = effect.set_analyzer(setting);
        }
        36 if key == MULTIBAND_SOLO_BAND => {
            let band = (value.max(0.0).round() as usize).checked_sub(1);
            let _ = effect.set_multiband_solo_band(band.filter(|&band| band < 3));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::f32::consts::TAU;

    use honeydew_audio::device::AudioRenderer;
    use honeydew_audio::pitcher::DEFAULT_NOTES;
    use honeydew_audio::{AudioEffect, ProcessContext};

    use super::super::SongAudioEngine;
    use crate::panels::cstm::{MULTIBAND_SOLO_BAND, cstm_from_settings};
    use crate::song::{ChannelKind, Song, SynthKind};

    const RATE: f32 = 8_000.0;
    /// Channel 1's modules: its MiniSynth is 2, then the Pitcher and the Multiband.
    const PITCHER: i32 = 3;
    const MULTIBAND: i32 = 4;

    /// The note enables of `notes` (pitch classes, 0 = C).
    fn mask(notes: &[usize]) -> [bool; 12] {
        std::array::from_fn(|note| notes.contains(&note))
    }

    /// A Pitcher blob with the note buttons (#5–#16) of `notes`.
    fn pitcher_blob(notes: [bool; 12]) -> Option<Vec<u8>> {
        let settings: HashMap<u32, f32> = (0..12)
            .map(|note| (5 + note as u32, f32::from(u8::from(notes[note]))))
            .collect();
        cstm_from_settings(25, &settings, None)
    }

    /// Channel 1: a MiniSynth, a Pitcher holding `cstm`, then a Multiband.
    fn engine(cstm: Option<Vec<u8>>) -> SongAudioEngine {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let rack = &mut song.channels[1].rack;
        assert_eq!((rack.add(25), rack.add(36)), (PITCHER, MULTIBAND));
        rack.get_mut(PITCHER).unwrap().file.custom_data = cstm;
        SongAudioEngine::new(&song, RATE).unwrap()
    }

    fn notes(engine: &SongAudioEngine) -> [bool; 12] {
        let effect = engine.audio.effect(1, PITCHER).unwrap();
        effect.pitcher_notes().unwrap()
    }

    /// One second of a `hz` sine through `effect`; returns the left output.
    fn through(effect: &mut AudioEffect, hz: f32) -> Vec<f32> {
        let mut left: Vec<f32> = (0..RATE as usize)
            .map(|i| (TAU * hz * i as f32 / RATE).sin() * 0.4)
            .collect();
        let mut right = left.clone();
        let context = ProcessContext {
            sample_rate: RATE,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        };
        effect.process(&mut left, &mut right, context).unwrap();
        left
    }

    /// The pitch the engine's Pitcher makes of a 220 Hz A3 at its fastest Speed, from its upward zero crossings
    /// once the glide settles.
    fn heard(engine: &mut SongAudioEngine) -> f32 {
        let effect = engine.audio.effect_mut(1, PITCHER).unwrap();
        effect.set_parameter(1, 1.0).unwrap();
        let tail = through(effect, 220.0).split_off(5_000);
        let crossings = tail.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0);
        crossings.count() as f32 * RATE / tail.len() as f32
    }

    /// The Analyzer's instance in [`analyzer_engine`]'s song: channel 1's second module.
    const SLOT: i32 = 3;

    /// Channel 1: a MiniSynth, then an Analyzer ([`SLOT`]) holding `cstm`.
    fn analyzer_engine(cstm: Option<Vec<u8>>) -> SongAudioEngine {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let rack = &mut song.channels[1].rack;
        assert_eq!(rack.add(26), SLOT);
        rack.get_mut(SLOT).unwrap().file.custom_data = cstm;
        SongAudioEngine::new(&song, RATE).unwrap()
    }

    /// The engine's Analyzer view after `seconds` of `signal(i) -> (L, R)`.
    fn analysed(
        engine: &mut SongAudioEngine,
        seconds: usize,
        signal: impl Fn(usize) -> (f32, f32),
    ) -> honeydew_audio::analysis::AnalyzerView {
        let effect = engine.audio.effect_mut(1, SLOT).unwrap();
        let (mut left, mut right): (Vec<f32>, Vec<f32>) =
            (0..seconds * RATE as usize).map(signal).unzip();
        let context = ProcessContext {
            sample_rate: RATE,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        };
        effect.process(&mut left, &mut right, context).unwrap();
        honeydew_audio::analysis::analyzer(effect.analysis()).unwrap()
    }

    /// The spectrum's peak (dB) and the loudness Peak maximum after 4 s of a −6 dBFS 1 kHz tone on the left only (the
    /// rising smoothing settles).
    fn left_tone(engine: &mut SongAudioEngine) -> (f32, f32) {
        let view = analysed(engine, 4, |i| {
            ((TAU * 1000.0 * i as f32 / RATE).sin() * 0.5, 0.0)
        });
        let peak = view.spectrum.iter().copied().fold(f32::MIN, f32::max);
        (peak, view.maxima[0])
    }

    #[test]
    fn the_analyzer_settings_reach_its_analysis() {
        let near = |a: f32, b: f32| (a - b).abs() < 1.5;
        // A new Analyzer takes L + R ((L + R)/2): −12 dB.
        let (peak, _) = left_tone(&mut analyzer_engine(None));
        assert!(near(peak, -12.0), "{peak}");
        // A saved `Channel` Left (#14, item 0) reads −6 dB.
        let left = cstm_from_settings(26, &HashMap::from([(14, 0.0)]), None);
        let mut engine = analyzer_engine(left);
        let (peak, _) = left_tone(&mut engine);
        assert!(near(peak, -6.0), "{peak}");
        // Live: Right (item 1) hears nothing, so the spectrum falls away; L + R with `Input Gain` at its top (#17, ×10)
        // reads +8 dB.
        engine.set_setting(1, SLOT, 14, 1.0);
        let (peak, _) = left_tone(&mut engine);
        assert!(peak < -80.0, "{peak}");
        engine.set_setting(1, SLOT, 14, 2.0);
        engine.set_setting(1, SLOT, 17, 1.0);
        let (peak, loudest) = left_tone(&mut engine);
        assert!(near(peak, 8.0) && near(loudest, 8.0), "{peak} {loudest}");
        // LOUDNESS `Mode` (#8) Peak (item 4) resets the meter.
        engine.set_setting(1, SLOT, 8, 4.0);
        let effect = engine.audio.effect(1, SLOT).unwrap();
        let view = honeydew_audio::analysis::analyzer(effect.analysis()).unwrap();
        assert_eq!(view.maxima[0], honeydew_audio::analysis::analyzer::FLOOR);
        // `Reactivity` (#15) Fast (item 2): a second of silence takes the +8 dB peak far down (Medium holds about −30).
        engine.set_setting(1, SLOT, 15, 2.0);
        let view = analysed(&mut engine, 1, |_| (0.0, 0.0));
        let peak = view.spectrum.iter().copied().fold(f32::MIN, f32::max);
        assert!(peak < -80.0, "{peak}");
        // A DC offset (×10: 0.5) fills bin 0 until `DC Block` (#16) is On (item 0).
        let offset = |_| (0.05, 0.05);
        assert!(analysed(&mut engine, 2, offset).spectrum[0] > -10.0);
        engine.set_setting(1, SLOT, 16, 0.0);
        let bin0 = analysed(&mut engine, 2, offset).spectrum[0];
        assert!(bin0 < -60.0, "{bin0}");
    }

    #[test]
    fn the_settings_the_engine_derives_app_thread_data_from_rebuild_it() {
        use super::rebuilds;
        // The DW Sampler's (its zones come from its whole blob), the Slicer's Sensitivity (its slices).
        assert!(rebuilds(1, 11800) && rebuilds(33, 3100));
        // GMSynth's waveA and waveB (their shapes are built by `prepare` first) and PB RANGE, a Pitcher note, the
        // Multiband's band: live.
        assert!(!rebuilds(2, 21100) && !rebuilds(2, 21200) && !rebuilds(2, 36100));
        assert!(!rebuilds(25, 5) && !rebuilds(36, MULTIBAND_SOLO_BAND));
        assert!(!rebuilds(25, 3100) && !rebuilds(33, 21100));
    }

    #[test]
    fn a_pitcher_without_a_blob_snaps_to_c_minor() {
        assert_eq!(notes(&engine(None)), DEFAULT_NOTES);
    }

    #[test]
    fn a_loaded_pitcher_corrects_to_its_saved_mask() {
        // Only D and G: A3 goes to G3, 2 semitones down (D4 is 5 up).
        let mut engine = engine(pitcher_blob(mask(&[2, 7])));
        assert_eq!(notes(&engine), mask(&[2, 7]));
        let hz = heard(&mut engine);
        assert!((hz - 196.0).abs() < 10.0, "{hz}");
    }

    /// A Pitcher whose blob goes away (its `Default` preset, an undo) in an edit that re-syncs a rolling transport is
    /// taken over with a new module's notes, as a fresh build of the edited song has them.
    #[test]
    fn a_pitcher_whose_blob_goes_away_through_a_re_sync_takes_the_new_module_notes() {
        let mut old = engine(pitcher_blob(mask(&[2, 7])));
        old.play();
        old.render(&mut [0.0; 512], 2);
        let mut new = engine(None).adopting_edit(true, Vec::new());
        new.take_over(&mut old);
        assert_eq!(notes(&new), notes(&engine(None)));
    }

    #[test]
    fn a_live_note_edit_reaches_the_pitcher() {
        let mut engine = engine(pitcher_blob(mask(&[2, 7])));
        // The A button (#14) on, then D (#7) and G (#12) off: A3 stays A3.
        engine.set_setting(1, PITCHER, 14, 1.0);
        assert_eq!(notes(&engine), mask(&[2, 7, 9]));
        engine.set_setting(1, PITCHER, 7, 0.0);
        engine.set_setting(1, PITCHER, 12, 0.0);
        assert_eq!(notes(&engine), mask(&[9]));
        let hz = heard(&mut engine);
        assert!((hz - 220.0).abs() < 10.0, "{hz}");
        // Scale (#20) and Key (#21) only set the spinners; the buttons they rewrite come as their own keys. A NaN, and
        // a note key on another module, change nothing.
        engine.set_setting(1, PITCHER, 20, 1.0);
        engine.set_setting(1, PITCHER, 21, 3.0);
        engine.set_setting(1, PITCHER, 5, f32::NAN);
        engine.set_setting(1, MULTIBAND, 5, 1.0);
        assert_eq!(notes(&engine), mask(&[9]));
    }

    #[test]
    fn the_soloed_band_reaches_the_multiband() {
        // An 80 Hz tone is in the Low band: soloing High mutes it, soloing Low or none keeps it.
        let mut engine = engine(None);
        let mut energy = |band: f32| {
            engine.set_setting(1, MULTIBAND, MULTIBAND_SOLO_BAND, band);
            let effect = engine.audio.effect_mut(1, MULTIBAND).unwrap();
            effect.reset();
            through(effect, 80.0).iter().map(|s| s * s).sum::<f32>()
        };
        let (all, low, high, none) = (energy(0.0), energy(1.0), energy(3.0), energy(0.0));
        assert!(low > 10.0 * high, "Low {low}, High {high}");
        assert!(
            all > 10.0 * high && none > 10.0 * high,
            "{all} {none} {high}"
        );
    }
}

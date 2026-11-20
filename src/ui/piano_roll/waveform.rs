//! Bounded off-thread procedural voice previews, measured from PCM, not decorative wave drawings.
//! Dry/solo note at 8 kHz, without the channel FX, automation, glide from earlier notes or other voices.
use crate::song::{Clip, Event, VocalNoteState};
use banaforge_audio::ProcessContext;
use banaforge_audio::instrument::Instrument;
use banaforge_audio::vocal_synth::{VocalExpression, VocalSynth};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) type Peaks = HashMap<(u32, u8), [f32; 48]>;

#[derive(Default)]
pub(super) struct Waveforms {
    pub peaks: Peaks,
    signature: Option<u64>,
    quiet: PreviewQuietPeriod,
    cancel: Option<Arc<AtomicBool>>,
    worker: Option<Receiver<Peaks>>,
}

/// Coalesce a stream of pitch/knob edits before starting offline preview DSP.
/// Audio playback is never delayed: only the optional thumbnail waits.
#[derive(Default)]
struct PreviewQuietPeriod(Option<(u64, f64)>);
impl PreviewQuietPeriod {
    fn ready(&mut self, signature: u64, now: f64) -> bool {
        match self.0 {
            Some((seen, since)) if seen == signature => now - since >= 0.2,
            _ => {
                self.0 = Some((signature, now));
                false
            }
        }
    }
}

impl Drop for Waveforms {
    fn drop(&mut self) {
        self.suspend();
    }
}

impl Waveforms {
    /// Preview DSP must yield immediately to transport or a disabled visualization.
    pub fn suspend(&mut self) {
        self.quiet = PreviewQuietPeriod::default();
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        if self.worker.take().is_some() {
            self.signature = None;
        }
    }
    pub fn update(&mut self, ctx: &eframe::egui::Context, clip: &Clip, params: &[f32], bpm: f64) {
        let mut hash = DefaultHasher::new();
        bpm.to_bits().hash(&mut hash);
        for param in params {
            param.to_bits().hash(&mut hash);
        }
        for event in clip.events.iter().rev().take(64) {
            if let Event::Note {
                pos,
                key,
                len_beats,
                velocity,
                ..
            } = event
            {
                (pos, key, velocity, len_beats.to_bits()).hash(&mut hash);
                if let Some(data) = clip
                    .file
                    .vocal_notes
                    .iter()
                    .find(|n| n.pos == *pos && n.key == *key)
                {
                    for value in [data.dynamics, data.breathiness, data.formant] {
                        value.to_bits().hash(&mut hash);
                    }
                    for p in &data.pitch {
                        (p.offset.to_bits(), p.cents.to_bits()).hash(&mut hash);
                    }
                }
            }
        }
        let signature = hash.finish();
        if let Some(worker) = &self.worker {
            match worker.try_recv() {
                Ok(peaks) => {
                    if self.signature == Some(signature) {
                        self.peaks = peaks;
                    }
                    self.worker = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.worker = None;
                }
                Err(TryRecvError::Empty) => {
                    if self.signature != Some(signature) {
                        if let Some(cancel) = &self.cancel {
                            cancel.store(true, Ordering::Relaxed);
                        }
                        // Keep the receiver until the worker exits: never pile up
                        // competing preview jobs during a touch drag.
                        self.signature = None;
                    }
                    ctx.request_repaint_after_secs(0.1);
                    return;
                }
            }
        }
        if self.signature == Some(signature) {
            return;
        }
        let now = ctx.input(|input| input.time);
        if !self.quiet.ready(signature, now) {
            ctx.request_repaint_after_secs(0.2);
            return;
        }
        self.signature = Some(signature);
        // At most 64 notes and 512k decoded frames per job; at most one job per roll.
        let notes: Vec<_> = clip
            .events
            .iter()
            .rev()
            .take(64)
            .filter_map(|event| {
                let Event::Note {
                    pos,
                    key,
                    len_beats,
                    velocity,
                    slide: false,
                    ..
                } = event
                else {
                    return None;
                };
                Some((
                    *pos,
                    *key,
                    *len_beats,
                    *velocity,
                    clip.file
                        .vocal_notes
                        .iter()
                        .find(|n| n.pos == *pos && n.key == *key)
                        .cloned(),
                ))
            })
            .collect();
        let params = params.to_vec();
        let (sender, receiver) = sync_channel(1);
        self.worker = Some(receiver);
        let repaint = ctx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        let work = move || {
            let mut peaks = Peaks::new();
            let mut budget = 512_000;
            for (pos, key, beats, velocity, data) in notes {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let frames = (beats * 60.0 / bpm * 8000.0).round();
                // Leave unavailable/overbudget previews blank, never stretch fabricated data.
                if !frames.is_finite()
                    || !(1.0..=64_000.0).contains(&frames)
                    || frames > f64::from(budget)
                {
                    continue;
                }
                let frames = frames as usize;
                budget -= frames as i32;
                peaks.insert(
                    (pos, key),
                    render_cancellable(
                        key,
                        velocity,
                        frames,
                        &params,
                        data.as_ref(),
                        Some(&cancel),
                    ),
                );
            }
            let _ = sender.send(peaks);
            repaint.request_repaint();
        };
        #[cfg(not(target_arch = "wasm32"))]
        if std::thread::Builder::new()
            .name("vocal-waveform".into())
            .spawn(work)
            .is_err()
        {
            self.worker = None;
        }
        // Web has no threads: omit thumbnails rather than doing offline rendering in a UI frame.
        #[cfg(target_arch = "wasm32")]
        {
            drop(work);
            self.worker = None;
        }
        ctx.request_repaint_after_secs(0.1);
    }
}

#[cfg(test)]
fn render(
    key: u8,
    velocity: u8,
    frames: usize,
    params: &[f32],
    data: Option<&VocalNoteState>,
) -> [f32; 48] {
    render_cancellable(key, velocity, frames, params, data, None)
}

fn render_cancellable(
    key: u8,
    velocity: u8,
    frames: usize,
    params: &[f32],
    data: Option<&VocalNoteState>,
    cancel: Option<&AtomicBool>,
) -> [f32; 48] {
    let mut synth = VocalSynth::new(8000.0);
    for (i, value) in params.iter().take(10).enumerate() {
        synth.set_parameter(i, *value);
    }
    synth.note_on(key, f32::from(velocity) / 255.0, 0.5);
    let mut peaks = [0.0f32; 48];
    let mut left = [0.0; 80];
    let mut right = [0.0; 80];
    for start in (0..frames).step_by(80) {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return [0.0; 48];
        }
        let count = (frames - start).min(80);
        if let Some(data) = data {
            synth.set_vocal(
                key,
                VocalExpression {
                    cents: crate::audio_engine::vocal::pitch(
                        &data.pitch,
                        start as f32 / frames as f32,
                    ),
                    dynamics: data.dynamics,
                    breathiness: data.breathiness,
                    formant: data.formant,
                    vowel: crate::audio_engine::vocal::vowel(&data.lyric),
                },
            );
        }
        left.fill(0.0);
        right.fill(0.0);
        synth.process(
            &mut left[..count],
            &mut right[..count],
            ProcessContext {
                sample_rate: 8000.0,
                tempo_bpm: 120.0,
                start_beat: 0.0,
                playing: true,
            },
        );
        for (i, sample) in left[..count].iter().enumerate() {
            let bin = ((start + i) * 48 / frames).min(47);
            peaks[bin] = peaks[bin].max(sample.abs());
        }
    }
    peaks
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_waits_for_quiet_and_coalesces_drag_updates() {
        let mut quiet = PreviewQuietPeriod::default();
        assert!(!quiet.ready(1, 0.0));
        assert!(!quiet.ready(1, 0.1));
        assert!(!quiet.ready(2, 0.15));
        assert!(!quiet.ready(3, 0.25));
        assert!(!quiet.ready(3, 0.4));
        assert!(quiet.ready(3, 0.5));
        quiet = PreviewQuietPeriod::default();
        assert!(!quiet.ready(3, 0.6));
    }

    #[test]
    fn vocal_preview_cancellation_stops_dsp_and_keeps_cached_peaks() {
        let cancel = AtomicBool::new(true);
        assert_eq!(
            render_cancellable(60, 255, 64000, &[], None, Some(&cancel)),
            [0.0; 48]
        );
        let mut waveforms = Waveforms::default();
        waveforms.peaks.insert((0, 60), [0.5; 48]);
        let flag = Arc::new(AtomicBool::new(false));
        waveforms.cancel = Some(flag.clone());
        let (_, receiver) = sync_channel(1);
        waveforms.worker = Some(receiver);
        waveforms.signature = Some(1);
        waveforms.suspend();
        assert!(flag.load(Ordering::Relaxed));
        assert!(waveforms.worker.is_none());
        assert!(waveforms.signature.is_none());
        assert_eq!(waveforms.peaks[&(0, 60)], [0.5; 48]);
    }

    #[test]
    fn vocal_lyrics_do_not_change_the_default_voice_waveform() {
        let params = [1.0, 0.5, 0.08, 0.5, 0.0, 0.35, 0.0, 0.0];
        let mut data = VocalNoteState {
            pos: 0,
            key: 60,
            lyric: String::new(),
            dynamics: 0.5,
            breathiness: 0.0,
            formant: 0.5,
            pitch: Vec::new(),
        };
        let reference = render(60, 255, 800, &params, Some(&data));
        assert_eq!(render(60, 255, 800, &params, None), reference);
        for lyric in ["hello", "あ", "bonjour", "你好"] {
            data.lyric = lyric.into();
            assert_eq!(render(60, 255, 800, &params, Some(&data)), reference);
        }
    }

    #[test]
    fn vocal_waveform_is_measured_and_respects_silence() {
        let params = [1.0, 0.5, 0.08, 0.5, 0.0, 0.35, 0.0, 0.0];
        let peaks = render(60, 255, 8000, &params, None);
        assert!(peaks.iter().all(|v| v.is_finite()));
        assert!(peaks.iter().any(|v| *v > 0.001));
        assert_eq!(render(60, 0, 8000, &params, None), [0.0; 48]);
    }
}

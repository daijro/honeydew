//! The audio clip editor's region ops on the sample (its Region menu), and its saving.
//!
//! A region op from the editor (`views::audio_editor::State::request`) is one edit: its envelope part
//! ([`audio_editor::apply`]) with the operation on the decoded frames ([`edit_frames`]). An op that leaves
//! the frames as they were is its envelope part alone, made at once. Otherwise the edited sample is saved first, to
//! [`target`]:
//! - a target that does not exist yet is written at once;
//! - an existing target prompts to save a new copy or replace the source. `Save new` opens the `New sample name`
//!   dialog on the current name (a name whose file exists asks again), `Replace` writes over it, and `Cancel`, or closing the
//!   question or the dialog, drops the whole op;
//! - the write, a 32-bit float WAV at the sample's rate and channels through [`ProjectFiles`], happens on the frame
//!   after `Processing...` shows. An error shows `An error occurred while processing the audio sample.` and changes
//!   nothing;
//! - once written, the envelope part, the slot pointed at the file, the file decoded again into the cache and
//!   the clip fitted to its new length (`fit_audio_clip`: @28 and the loop) are one undo step.
//!
//! Details:
//! - The target folder is the sample's own in the user folder (an `@L@` path that stays inside it). Any other sample
//!   (factory `@R@`, built-in, temp `@T@`) goes to `My Samples`. The name is the file's without its extension, else
//!   the slot's.
//! - An op that would leave no frames (`Trim selection` or `Delete space` over the whole sample) is an error: an empty
//!   WAV does not load.
//! - An op is dropped if the clip's sample changed while it waited (an undo under the question).
//! - `Replace` over the sample's own file with no envelope part leaves the song as it was: no undo step, but the
//!   engine gets the new frames, and its re-sync relaunches the clips playing them.
//!
//! [`ProjectFiles`]: crate::project_files::ProjectFiles

use std::path::{Path, PathBuf};

use eframe::egui;
use honeydew_audio::sample::SampleData;
use honeydew_audio::sample_edit::{self as ops, frame_at, speed_ratio};
use honeydew_audio::wav_writer::float32_wav;

use super::HoneydewApp;
use crate::audio_engine::fit_audio_clip;
use crate::flm::chunk::ContentPath;
use crate::metrics::Metrics;
use crate::project_files::{CONTENT_FOLDER, SAMPLES_FOLDER, in_content_folder, in_user_folder};
use crate::sample_cache::{self, SampleKey};
use crate::song::{LaneKind, PPQ, SamplerSlot, Song};
use crate::ui::UiCtx;
use crate::ui::audio_editor::{self, Action, At, stretch_of};
use crate::ui::preview::ClipKey;
use crate::ui::region::{RegionOp, TimeRange};
use crate::ui::sample_row::clip_mut;
use crate::ui::views::sample_picker::give;
use crate::ui::views::{self, Answer, Nav, Overlay, SongEdit};

/// The save question: its tag, text and choices (`Save new` above `Replace` / `Cancel`).
const SAVE_TAG: &str = "audio edit save";
pub const SAVE_QUESTION: &str = "Save new sample or replace the original?";
const SAVE_NEW: &str = "Save new";
const REPLACE: &str = "Replace";
const CANCEL: &str = "Cancel";
/// The error message's tag and text.
const ERROR_TAG: &str = "audio edit error";
pub const PROCESS_FAILED: &str = "An error occurred while processing the audio sample.";
/// The loading screen's title.
pub const PROCESSING: &str = "Processing...";

/// A region op whose edited sample waits to be saved.
pub(super) struct Pending {
    at: At,
    /// The editor's `Action::Region`.
    action: Action,
    /// What the clip played when the op came.
    source: Option<SampleKey>,
    rate: u32,
    channels: Vec<Vec<f32>>,
    /// The file it is written to.
    target: PathBuf,
    stage: Stage,
}

impl Pending {
    /// The op with its clip's track where `track` (channel, lane → new place) moved it; `None` when the track went.
    /// Undo works under the question, and an op must not land on whatever clip its old place holds now.
    pub(super) fn moved(
        mut self,
        track: impl Fn(usize, usize) -> Option<(usize, usize)>,
    ) -> Option<Self> {
        (self.at.0, self.at.1) = track(self.at.0, self.at.1)?;
        Some(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// The save question is open.
    Asking,
    /// The `New sample name` dialog is open.
    Naming,
    /// `Processing...` shows; the next frame writes.
    Writing,
}

/// The audio part of `op` over the content ticks `range`, the playhead at `playhead`, on `sample` as `slot` plays it
/// at `tempo`: each channel's frames after the operation, or `None` when they are as they were (or `op` has none).
/// Beats map to frames as the waveform draws them: trunc(trunc(beat·60·rate/tempo)·speed) ([`frame_at`]) over the
/// SYNC stretch ratio ([`stretch_of`]). A reversed sample is reversed around the op, as it is drawn mirrored.
pub fn edit_frames(
    sample: &SampleData,
    slot: &SamplerSlot,
    tempo: f64,
    op: RegionOp,
    range: TimeRange,
    playhead: u32,
) -> Option<Vec<Vec<f32>>> {
    let (speed, stretch) = (speed_ratio(slot.params[2]), stretch_of(slot));
    let frame = |tick: u32| {
        let beat = f64::from(tick) / f64::from(PPQ);
        (frame_at(beat, sample.sample_rate, tempo, speed) as f64 / stretch) as usize
    };
    let (s, e, p) = (frame(range.start), frame(range.end), frame(playhead));
    let source: Vec<&[f32]> = (std::iter::once(&sample.left).chain(&sample.right))
        .map(|c| &c[..])
        .collect();
    let mut ch: Vec<Vec<f32>> = source.iter().map(|c| c.to_vec()).collect();
    let reverse = |ch: &mut Vec<Vec<f32>>| {
        if slot.file.reverse {
            ch.iter_mut().for_each(|c| c.reverse());
        }
    };
    reverse(&mut ch);
    match op {
        RegionOp::InsertSilence => ops::insert_silence(&mut ch, s, e - s),
        RegionOp::Clear => ops::clear(&mut ch, s..e),
        RegionOp::DeleteSpace => ops::delete_space(&mut ch, s..e),
        RegionOp::Trim => ops::trim(&mut ch, s..e),
        RegionOp::Duplicate => ops::duplicate(&mut ch, s..e),
        RegionOp::CopyToPlayhead => ops::copy_to(&mut ch, s..e, p),
        RegionOp::MoveToPlayhead => ops::move_to(&mut ch, s..e, p),
        RegionOp::Normalize => ops::normalize(&mut ch, s..e),
        RegionOp::FadeIn => ops::fade_in(&mut ch, s..e),
        RegionOp::FadeOut => ops::fade_out(&mut ch, s..e),
        _ => return None,
    }
    reverse(&mut ch);
    ch.iter().map(Vec::as_slice).ne(source).then_some(ch)
}

/// Where `slot`'s edited sample is saved, `user` being the user folder: `<its folder>/<name>.wav` for a sample in the
/// user folder (not in its original content), else `My Samples/<name>.wav`. The name is the file's without its extension, else the slot's.
pub fn target(slot: &SamplerSlot, user: &Path) -> PathBuf {
    let file = match &slot.file.path {
        Some(ContentPath::Shorthand { relative, .. } | ContentPath::Plain(relative)) => {
            relative.replace('\\', "/")
        }
        None => String::new(),
    };
    let name = Path::new(&file)
        .file_stem()
        .map_or_else(|| slot.name.clone(), |s| s.to_string_lossy().into_owned());
    let own = (slot.file.path.as_ref())
        .and_then(in_user_folder)
        .and_then(Path::parent)
        .map(|dir| user.join(dir))
        // The original content is read-only, however the path reaches it (a link, another case).
        .filter(|dir| !in_content_folder(&user.join(CONTENT_FOLDER), dir));
    own.unwrap_or_else(|| user.join(SAMPLES_FOLDER))
        .join(format!("{name}.wav"))
}

fn key((channel, lane, id): At) -> ClipKey {
    ClipKey { channel, lane, id }
}

fn slot_at(song: &Song, at: At) -> Option<&SamplerSlot> {
    key(at).clip(song)?.file.sample.as_deref()
}

impl HoneydewApp {
    /// The audio editor's region ops (see the module doc), after the frame's edit: the write `Processing...` showed
    /// for, the editor's new op, then the answer of the question or the dialog. `tick` is the transport position.
    pub(super) fn audio_edit(&mut self, ctx: &egui::Context, m: &Metrics, tick: u64) {
        if let Some(p) = self.audio_edit.take_if(|p| p.stage == Stage::Writing) {
            self.write_sample(p, tick);
        }
        // One op at a time: the question and the dialog are modal.
        if let Some((at, action)) = self.slots.audio_editor.request.take()
            && self.audio_edit.is_none()
        {
            self.audio_edit = self.audio_op(at, action, tick);
        }
        if let Some(p) = self.audio_edit.take() {
            self.audio_edit = self.save_answered(p);
        }
        if self
            .audio_edit
            .as_ref()
            .is_some_and(|p| p.stage == Stage::Writing)
        {
            self.processing(ctx, m);
            ctx.request_repaint();
        }
    }

    /// The editor's region `action` on clip `at`: its envelope part alone, at once, when the audio stays as it was;
    /// else the save it waits for, or the error when it cannot be saved.
    fn audio_op(&mut self, at: At, action: Action, tick: u64) -> Option<Pending> {
        let Action::Region {
            op,
            range,
            playhead,
            ..
        } = action
        else {
            return None;
        };
        let slot = slot_at(&self.song, at);
        // The source the editor draws, not the engine's stretch or pitch render: the file gets no `STR1` baked in.
        let sample = (slot.and_then(SampleKey::of_slot)).and_then(|k| self.audio.samples.get(&k));
        let tempo = self.song.tempo_bpm;
        let edited = (slot.zip(sample.as_deref()))
            .and_then(|(slot, s)| edit_frames(s, slot, tempo, op, range, playhead));
        let (Some(channels), Some(slot), Some(sample)) = (edited, slot, sample) else {
            if audio_editor::apply(&mut self.song, at, &action) {
                self.song_edited(SongEdit::Final, tick);
            }
            return None;
        };
        let target = self.files.user_dir().map(|user| target(slot, user));
        let Some(target) = target.filter(|_| channels.first().is_some_and(|c| !c.is_empty()))
        else {
            self.sample_failed();
            return None;
        };
        let pending = Pending {
            at,
            source: SampleKey::of_slot(slot),
            rate: sample.sample_rate,
            channels,
            target,
            action,
            stage: Stage::Writing,
        };
        Some(self.ask_or_write(pending))
    }

    /// `p` written on the next frame, unless its target exists: then asked.
    fn ask_or_write(&mut self, mut p: Pending) -> Pending {
        p.stage = if self.files.user_file_exists(&p.target) {
            self.navigate(Nav::Open(Overlay::Message {
                tag: SAVE_TAG,
                lines: vec![SAVE_QUESTION.into()],
                confirm: Some((REPLACE.into(), CANCEL.into())),
                extra: Some(SAVE_NEW.into()),
            }));
            Stage::Asking
        } else {
            Stage::Writing
        };
        p
    }

    /// `p` as the question or the dialog answers: `Save new` names it, `Replace` (or a free name) writes it, and
    /// `Cancel`, a question closed otherwise or the dialog closed drop it. `p` as it was while they wait.
    fn save_answered(&mut self, mut p: Pending) -> Option<Pending> {
        match p.stage {
            Stage::Asking => match self.nav.take_choice(SAVE_TAG) {
                None => Some(p),
                Some(Answer::Yes) => Some(Pending {
                    stage: Stage::Writing,
                    ..p
                }),
                Some(Answer::Extra) => {
                    let name = p.target.file_stem().unwrap_or_default();
                    let name = name.to_string_lossy().into_owned();
                    // A name typed into a dialog whose op was dropped must not answer this one.
                    self.slots.audio_editor.named = None;
                    self.navigate(Nav::Open(Overlay::SampleName { name }));
                    Some(Pending {
                        stage: Stage::Naming,
                        ..p
                    })
                }
                Some(Answer::No) => None,
            },
            Stage::Naming => {
                if let Some(name) = self.slots.audio_editor.named.take() {
                    p.target.set_file_name(format!("{name}.wav"));
                    return Some(self.ask_or_write(p));
                }
                let open =
                    (self.nav.overlays.iter()).any(|o| matches!(o, Overlay::SampleName { .. }));
                open.then_some(p)
            }
            Stage::Writing => Some(p),
        }
    }

    /// Writes `p`'s sample to its target, then makes the op: its envelope part, the slot on the file, the file's cache
    /// entry dropped, one undo step. Dropped when the clip's sample changed meanwhile; an error changes nothing.
    fn write_sample(&mut self, p: Pending, tick: u64) {
        if slot_at(&self.song, p.at).map(SampleKey::of_slot) != Some(p.source) {
            return;
        }
        let Some(file) = self.files.user_shorthand(&p.target) else {
            return self.sample_failed();
        };
        let bytes = float32_wav(p.rate, &p.channels);
        if self.files.write_user_file(&p.target, bytes).is_err() {
            return self.sample_failed();
        }
        audio_editor::apply(&mut self.song, p.at, &p.action);
        let (bpm, b) = (self.song.tempo_bpm, self.song.time_sig.bar_beats());
        let user = self.files.user_dir().unwrap_or(Path::new(""));
        if let Some(slot) =
            clip_mut(&mut self.song, key(p.at)).and_then(|c| c.file.sample.as_deref_mut())
            && slot.file.path.as_ref() != Some(&file)
        {
            give(slot, &p.target, user);
        }
        // The new file decoded here, and its stretch and pitch render streaming in (`load_live`: its first frames now,
        // the rest on the pickers' worker, by the pump `drum_editor::after_chrome` runs every frame whatever view
        // shows): the rebuild loads nothing without an output. Then every clip on it (the edited one, and others a
        // `Replace` changed under them) gets @28 and the loop of its new length, in this same undo step.
        let key = SampleKey::File(file);
        let samples = &mut self.audio.samples;
        samples.forget(&key);
        samples.load_live(&self.song, &self.song, &sample_cache::reader(&self.files));
        let clips = (self.song.channels.iter_mut())
            .flat_map(|c| &mut c.lanes)
            .filter(|l| l.kind == LaneKind::Audio)
            .flat_map(|l| &mut l.clips);
        for clip in clips {
            if clip.file.sample.as_deref().and_then(SampleKey::of_slot) == Some(key.clone()) {
                fit_audio_clip(clip, samples, bpm, b);
            }
        }
        // A re-sync relaunches the clips with the new data, though a `Replace` may leave the song as it was.
        self.song_edited(SongEdit::Resync, tick);
    }

    fn sample_failed(&mut self) {
        self.navigate(Nav::Open(Overlay::Message {
            tag: ERROR_TAG,
            lines: vec![PROCESS_FAILED.into()],
            confirm: None,
            extra: None,
        }));
    }

    /// The loading screen the write waits behind: `Processing...` ([`views::busy`], the export's too).
    fn processing(&self, ctx: &egui::Context, m: &Metrics) {
        views::busy(
            ctx,
            &UiCtx {
                theme: &self.theme,
                m,
            },
            PROCESSING,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use honeydew_audio::sample::SampleData;

    use super::*;
    use crate::flm::chunk::{ContentPath, ContentRoot};
    use crate::song::SamplerSlot;
    use crate::ui::region::{RegionOp, TimeRange};

    /// 8 frames a beat at 120 BPM.
    const RATE: u32 = 16;
    const TEMPO: f64 = 120.0;

    fn sample(left: Vec<f32>, right: Option<Vec<f32>>) -> SampleData {
        SampleData {
            sample_rate: RATE,
            left: left.into(),
            right: right.map(Arc::from),
            ..SampleData::default()
        }
    }

    fn ramp(n: usize) -> Vec<f32> {
        (1..=n).map(|i| i as f32 / 100.0).collect()
    }

    /// Beats `s` … `e` as content ticks.
    fn beats(s: u32, e: u32) -> TimeRange {
        TimeRange {
            start: s * 128,
            end: e * 128,
        }
    }

    fn edit(
        s: &SampleData,
        slot: &SamplerSlot,
        op: RegionOp,
        r: TimeRange,
    ) -> Option<Vec<Vec<f32>>> {
        edit_frames(s, slot, TEMPO, op, r, 0)
    }

    /// `ramp(n)` with frames `zeroed` cleared.
    fn cleared(n: usize, zeroed: std::ops::Range<usize>) -> Vec<f32> {
        let mut v = ramp(n);
        v[zeroed].fill(0.0);
        v
    }

    #[test]
    fn beats_map_to_frames_as_the_waveform_draws_them() {
        let s = sample(ramp(32), None);
        let mut slot = SamplerSlot::default();
        // Speed 1: beat 1 … 2 is frames 8 … 16.
        assert_eq!(
            edit(&s, &slot, RegionOp::Clear, beats(1, 2)),
            Some(vec![cleared(32, 8..16)])
        );
        // Speed +12 plays twice as fast: twice the frames a beat.
        slot.params[2] = 0.75;
        assert_eq!(
            edit(&s, &slot, RegionOp::Clear, beats(1, 2)),
            Some(vec![cleared(32, 16..32)])
        );
        // A SYNC slot stretched ×2 is drawn twice as long: the frame position is divided by the ratio.
        slot.params[2] = 0.5;
        (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 2.0);
        assert_eq!(
            edit(&s, &slot, RegionOp::Clear, beats(1, 2)),
            Some(vec![cleared(32, 4..8)])
        );
        // The ratio counts only while SYNC is on.
        slot.file.stretch_mode = 0;
        assert_eq!(
            edit(&s, &slot, RegionOp::Clear, beats(1, 2)),
            Some(vec![cleared(32, 8..16)])
        );
    }

    #[test]
    fn a_reversed_sample_is_edited_where_it_is_drawn() {
        let s = sample(ramp(16), Some(ramp(16)));
        let slot = SamplerSlot {
            file: crate::song::SlotFileState {
                reverse: true,
                ..Default::default()
            },
            ..SamplerSlot::default()
        };
        // Drawn mirrored: the first beat shows the last 8 frames.
        let want = cleared(16, 8..16);
        assert_eq!(
            edit(&s, &slot, RegionOp::Clear, beats(0, 1)),
            Some(vec![want.clone(), want])
        );
        // A fade in over it ramps up towards the sample's start, where the mirrored range ends.
        let faded = edit(&s, &slot, RegionOp::FadeIn, beats(0, 1)).unwrap();
        assert_eq!(faded[0][15], 0.0);
        assert!(faded[0][8] > 0.0 && faded[0][8] == ramp(16)[8] * 0.875);
    }

    #[test]
    fn each_audio_op_reaches_the_frames_and_nothing_changed_is_none() {
        let s = sample(ramp(16), None);
        let slot = SamplerSlot::default();
        let r = beats(0, 1);
        let len = |op| edit(&s, &slot, op, r).map(|ch| ch[0].len());
        assert_eq!(len(RegionOp::InsertSilence), Some(24));
        assert_eq!(len(RegionOp::DeleteSpace), Some(8));
        assert_eq!(len(RegionOp::Trim), Some(8));
        assert_eq!(len(RegionOp::Duplicate), Some(24));
        assert_eq!(len(RegionOp::FadeOut), Some(16));
        // Copy and move to the playhead at the range's end (beat 1, frame 8): a copy, and no change.
        let to = |op| edit_frames(&s, &slot, TEMPO, op, r, 128).map(|ch| ch[0].len());
        assert_eq!(to(RegionOp::CopyToPlayhead), Some(24));
        assert_eq!(to(RegionOp::MoveToPlayhead), None, "moved onto itself");
        // Normalize lifts the range's peak to 1.0.
        let n = edit(&s, &slot, RegionOp::Normalize, r).unwrap();
        assert_eq!(n[0][7], 1.0);
        assert_eq!(n[0][8], ramp(16)[8]);
        // Nothing changes: a range of silence cleared or normalized, an envelope-only row.
        let quiet = sample(vec![0.0; 16], None);
        assert_eq!(edit(&quiet, &slot, RegionOp::Clear, r), None);
        assert_eq!(edit(&quiet, &slot, RegionOp::Normalize, r), None);
        assert_eq!(edit(&s, &slot, RegionOp::Delete, r), None);
    }

    fn shorthand(root: ContentRoot, relative: &str) -> Option<ContentPath> {
        Some(ContentPath::Shorthand {
            root,
            relative: relative.into(),
        })
    }

    #[test]
    fn the_target_is_the_samples_own_folder_or_my_samples() {
        let user = Path::new("/u");
        let slot = |path: Option<ContentPath>, name: &str| SamplerSlot {
            name: name.into(),
            file: crate::song::SlotFileState {
                path,
                ..Default::default()
            },
            ..SamplerSlot::default()
        };
        // A user sample: its own folder, `.wav`, the file's name.
        let flac = slot(
            shorthand(ContentRoot::User, "My Samples/Kicks/Big.flac"),
            "x",
        );
        assert_eq!(target(&flac, user), user.join("My Samples/Kicks/Big.wav"));
        // Factory, temp and built-in samples, and a path that would leave the user folder: `My Samples`.
        let factory = slot(shorthand(ContentRoot::App, "Drum Samples\\Kick.wav"), "x");
        assert_eq!(target(&factory, user), user.join("My Samples/Kick.wav"));
        let temp = slot(shorthand(ContentRoot::Temp, "take.wav"), "x");
        assert_eq!(target(&temp, user), user.join("My Samples/take.wav"));
        let builtin = SamplerSlot {
            folder: "Built-in/Kick".into(),
            ..slot(None, "Synth Kick")
        };
        assert_eq!(
            target(&builtin, user),
            user.join("My Samples/Synth Kick.wav")
        );
        let escape = slot(shorthand(ContentRoot::User, "../out/Loop.wav"), "x");
        assert_eq!(target(&escape, user), user.join("My Samples/Loop.wav"));
        // A user-root path into the `Original Content` folder: `My Samples`, never there (it is read-only).
        let content = slot(
            shorthand(
                ContentRoot::User,
                "Original Content/Drum Samples/Kick/Gen.flac",
            ),
            "x",
        );
        assert_eq!(target(&content, user), user.join("My Samples/Gen.wav"));
    }

    #[cfg(unix)]
    #[test]
    fn a_sample_reached_through_a_link_into_the_original_content_is_saved_in_my_samples() {
        let user = std::env::temp_dir().join(format!("honeydew-byo-target-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user);
        let kicks = user
            .join(crate::project_files::CONTENT_FOLDER)
            .join("Drum Samples/Kick");
        std::fs::create_dir_all(&kicks).unwrap();
        std::fs::create_dir_all(user.join("My Samples")).unwrap();
        std::os::unix::fs::symlink(&kicks, user.join("My Samples/Linked")).unwrap();
        let slot = SamplerSlot {
            file: crate::song::SlotFileState {
                path: shorthand(ContentRoot::User, "My Samples/Linked/Gen.flac"),
                ..Default::default()
            },
            ..SamplerSlot::default()
        };
        let saved = target(&slot, &user);
        std::fs::remove_dir_all(&user).unwrap();
        assert_eq!(saved, user.join("My Samples/Gen.wav"));
    }
}

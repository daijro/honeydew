//! The tempo editor, opened by the transport's BPM button: [`crate::ui::tempo`]'s editor in a 3 U band across the
//! middle of the screen, as the Save dialog's band.

use eframe::egui::{self, Rect, vec2};

use super::{AppCtx, Nav};
use crate::metrics::Metrics;
use crate::ui::tempo::{TempoAction, TempoEditor};

/// The band's height, in U.
const BAND_H: f32 = 3.0;

/// The editor's band on `screen`.
pub fn band(screen: Rect, m: &Metrics) -> Rect {
    Rect::from_center_size(screen.center(), vec2(screen.width(), m.u(BAND_H)))
}

/// What the tempo editor keeps between frames.
#[derive(Default)]
pub struct State {
    editor: TempoEditor,
    /// The tempo a drag in progress last set. Any other tempo means the song changed under the drag (an undo,
    /// which also cancels the drag): there is no drag left to finish.
    dragging: Option<f64>,
}

/// Draws the editor. A new tempo goes into the song: mid-drag as an ongoing edit, at the end of the drag (or from a
/// step button or `TAP`) as a completed one. Closing it mid-drag (Escape) completes the drag.
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx) -> Option<Nav> {
    let rect = band(app.regions.screen, app.cx.m);
    let song = &mut *app.song;
    if state.dragging != Some(song.tempo_bpm) {
        state.dragging = None;
    }
    match state
        .editor
        .show(ui, &app.cx, rect, song.tempo_bpm, song.time_sig)?
    {
        TempoAction::Set { bpm, done } => {
            // Also scales the audio clips' stretch ratios, in the drag's one undo step (`Song::set_tempo`).
            song.set_tempo(bpm);
            state.dragging = (!done).then_some(bpm);
            app.song_edited(done);
            None
        }
        TempoAction::Close => {
            if state.dragging.take().is_some() {
                app.song_edited(true);
            }
            Some(Nav::Close)
        }
    }
}

/// The editor went another way than its own Close (the `showTempo` key's toggle): a dial drag under way at `tempo` ends
/// there, as Escape ends it. Whether that is a completed edit to make (`app::left_view`).
pub fn closed(state: &mut State, tempo: f64) -> bool {
    state.dragging.take() == Some(tempo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_link::AudioLink;
    use crate::project_files::{ProjectFiles, mem::MemFiles};
    use crate::settings::Settings;
    use crate::song::{ChannelKind, SamplerSlot, Song};
    use crate::ui::UiCtx;
    use crate::ui::main_screen::Regions;
    use crate::ui::offscreen::{Offscreen, SCREEN};
    use crate::ui::views::SongEdit;

    /// The tempo editor on a 140 BPM song with one SYNC-on audio clip (ratio 1.5), a frame at a time.
    struct Rig {
        screen: Offscreen,
        state: State,
        song: Song,
        files: ProjectFiles,
        /// The edit each frame reported.
        edits: Vec<Option<SongEdit>>,
    }

    impl Rig {
        /// A 140 BPM song with one SYNC-on audio clip (ratio 1.5).
        fn new() -> Self {
            let mut song = Song {
                tempo_bpm: 140.0,
                ..Song::default()
            };
            song.add_channel(ChannelKind::AudioClip);
            let mut sample = SamplerSlot::default();
            (sample.file.stretch_mode, sample.file.stretch_ratio) = (1, 1.5);
            song.channels[1].lanes[0].add_clip(0, 4.0).file.sample = Some(Box::new(sample));
            let files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
            let mut rig = Self {
                screen: Offscreen::new(),
                state: State::default(),
                song,
                files,
                edits: Vec::new(),
            };
            rig.run(|s, f| {
                s.frame(vec![], f);
            });
            rig
        }

        fn run(
            &mut self,
            input: impl FnOnce(&mut Offscreen, &mut dyn FnMut(&mut egui::Ui, &UiCtx)),
        ) {
            let (state, song, files, edits) = (
                &mut self.state,
                &mut self.song,
                &mut self.files,
                &mut self.edits,
            );
            input(&mut self.screen, &mut |ui, cx| {
                let mut app = AppCtx {
                    cx: *cx,
                    regions: Regions::new(SCREEN, cx.m),
                    song: &mut *song,
                    audio: &mut AudioLink::default(),
                    selected: &mut 1,
                    snapshot: None,
                    playing: false,
                    armed: false,
                    previewing: false,
                    middle_c_is_c4: false,
                    settings: &mut Settings::default(),
                    ghost_notes: true,
                    files: &mut *files,
                    edit: None,
                    answer: None,
                    covered: false,
                    playhead: None,
                    seek: None,
                    midi_devices: &[],
                    midi_rescan: false,
                };
                show(state, ui, &mut app);
                edits.push(app.edit);
            });
        }

        fn ratio(&self) -> f64 {
            let clip = &self.song.channels[1].lanes[0].clips[0];
            clip.file.sample.as_ref().unwrap().file.stretch_ratio
        }
    }

    #[test]
    fn a_step_button_rescales_the_audio_clips_in_its_final_edit() {
        let mut rig = Rig::new();
        let plus_10 = rig
            .screen
            .ctx
            .read_response(egui::Id::new("tempo_editor").with("+10"))
            .expect("the button was drawn")
            .rect
            .center();
        rig.run(|s, f| s.click(plus_10, f));
        assert_eq!(rig.song.tempo_bpm, 150.0);
        assert!(
            (rig.ratio() - 1.5 * 140.0 / 150.0).abs() < 1e-12,
            "{}",
            rig.ratio()
        );
        assert_eq!(rig.edits.last(), Some(&Some(SongEdit::Final)));
    }

    #[test]
    fn a_dial_turn_rescales_them_on_every_frame_and_ends_where_one_change_would() {
        let mut rig = Rig::new();
        let c = rig
            .screen
            .ctx
            .read_response(egui::Id::new("tempo_editor").with("dial"))
            .expect("the dial was drawn")
            .rect
            .center();
        // A full turn clockwise as seen, 32 points out (from 3 o'clock through 6, 9 and 12): +1 BPM.
        let r = 32.0;
        let offsets = [
            vec2(-r, r),
            vec2(-2.0 * r, 0.0),
            vec2(-r, -r),
            vec2(0.0, 0.0),
        ];
        rig.run(|s, f| s.drag(c + vec2(r, 0.0), &offsets, f));
        assert!((rig.song.tempo_bpm - 141.0).abs() < 1e-9);
        // Ongoing frames, then the release's final one.
        let ongoing = rig
            .edits
            .iter()
            .filter(|e| **e == Some(SongEdit::Ongoing))
            .count();
        assert_eq!(ongoing, 4, "{:?}", rig.edits);
        assert_eq!(rig.edits.last(), Some(&Some(SongEdit::Final)));
        assert!(
            (rig.ratio() - 1.5 * 140.0 / rig.song.tempo_bpm).abs() < 1e-12,
            "{}",
            rig.ratio()
        );
    }
}

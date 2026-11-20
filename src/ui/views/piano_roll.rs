//! The piano roll view: [`PianoRoll`] in the editor area, one per channel, kept across visits so each channel keeps its
//! snap, zoom and scroll.

use std::collections::HashMap;

use banaforge_audio::device::AudioCommand;
use eframe::egui;

use super::{AppCtx, Nav, View};
use crate::song::Song;
use crate::ui::clips::LiveTake;
use crate::ui::piano_roll::{PianoRoll, PianoRollAction, RollOptions};

mod desktop;

/// What the piano roll keeps between frames: each channel's roll, by channel index (the app re-keys them when
/// channels move, `app/tracks.rs`).
#[derive(Default)]
pub struct State {
    pub(crate) rolls: HashMap<usize, PianoRoll>,
    /// The keys sounding on the shown roll's channel outside it (the app sets them each frame,
    /// [`RollOptions::sounding`]).
    pub(crate) sounding: u128,
    /// The takes being recorded (the app sets them each frame): a roll on one's clip shows it live
    /// ([`RollOptions::recording`]).
    pub(crate) takes: Vec<LiveTake>,
    /// Metadata inspected in the header. Does not activate a nonexistent bank renderer.
    voicebank_labels: HashMap<usize, String>,
    workspaces: HashMap<usize, desktop::Workspace>,
}

impl State {
    /// Opens the `Generate` panel of `channel`'s roll from the transport bar's GENERATE button (PR #87). A channel
    /// with no roll yet gets one; a second tap writes another progression.
    pub(crate) fn open_generate(&mut self, ctx: &egui::Context, song: &Song, channel: usize) {
        let roll =
            (self.rolls.entry(channel)).or_insert_with(|| PianoRoll::for_channel(song, channel));
        roll.open_generate(ctx);
    }
}

/// Draws the piano roll of `channel`'s note lane in `app.regions.editor`; back to the playlist once the channel is gone
/// (the app collapses an editor whose clip goes).
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx, channel: usize) -> Option<Nav> {
    app.covered |= crate::ui::help::corner(
        ui,
        app.regions.editor,
        crate::ui::help::Topic::PianoRoll,
    );
    if channel >= app.song.channels.len() {
        return Some(Nav::Show(View::Playlist));
    }
    let is_vocal = crate::song::vocal_editor(&app.song.channels[channel]);
    let roll = state
        .rolls
        .entry(channel)
        .or_insert_with(|| PianoRoll::for_channel(app.song, channel));
    if !is_vocal {
        roll.pitch_draw_enabled = false;
    }
    let studio = is_vocal && app.settings.vocal_desktop_workspace;
    let editor = if studio {
        let workspace = state.workspaces.entry(channel).or_default();
        let selected = state.voicebank_labels.entry(channel).or_default();
        desktop::show(ui, app, channel, roll, workspace, selected)
    } else {
        Some(app.regions.editor)
    };
    roll.lyric_bar_enabled = app.settings.vocal_lyric_bar_enabled;
    roll.simple_mode = app.settings.vocal_simple_mode;
    roll.waveform_enabled = app.settings.vocal_waveform_enabled;
    roll.waveform_offset_y = app.settings.vocal_waveform_offset_y;
    let opts = RollOptions {
        ghost_notes: app.ghost_notes,
        middle_c_is_c4: app.middle_c_is_c4,
        key_names: app.settings.key_names,
        playhead: app.snapshot.map_or(0, |s| s.transport_tick),
        lead: app.audio.playhead_lead(),
        playing: app.playing,
        armed: app.armed,
        previewing: app.previewing,
        covered: app.covered,
        continuous: if studio {
            app.settings.vocal_follow_playhead
        } else {
            app.settings.continuous_scroll
        },
        sounding: state.sounding,
        recording: (state.takes.iter()).any(|t| {
            (t.channel, t.lane, t.clip) == (channel, roll.lane, roll.clip_id) && t.clip.is_some()
        }),
    };
    // The graph lane's value snap is an app setting: the snap menu's picks are saved with the settings.
    roll.value_snap = [app.settings.velocity_snap, app.settings.pan_snap];
    // So is the grid size (#53).
    roll.snap.size = app.settings.grid_size;
    // And the graph lane tab's place (PR #87).
    roll.graph_tab_shift = app.settings.graph_tab_shift;
    let local_theme = studio.then(|| desktop::theme(app.cx.theme));
    let cx = crate::ui::UiCtx {
        theme: local_theme.as_ref().unwrap_or(app.cx.theme),
        m: app.cx.m,
    };
    let actions = if let Some(editor) =
        editor.filter(|r| r.width() > 1.0 && r.height() > if studio { 96.0 } else { 1.0 })
    {
        roll.show(ui, editor, &cx, app.song, &opts)
    } else {
        // The compact inspector owns the editor area: release note gates and
        // finish pending edits rather than leaving a hidden roll sounding.
        roll.hide(app.song)
    };
    [app.settings.velocity_snap, app.settings.pan_snap] = roll.value_snap;
    app.settings.grid_size = roll.snap.size;
    app.settings.graph_tab_shift = roll.graph_tab_shift;
    for action in actions {
        match action {
            PianoRollAction::SongEdited { final_edit } => app.song_edited(final_edit),
            PianoRollAction::QuietEdit => app.quiet_edited(),
            PianoRollAction::LyricEdited => app.edit = app.edit.max(Some(super::SongEdit::Param)),
            PianoRollAction::LyricCommitted => {
                app.edit = app.edit.max(Some(super::SongEdit::Metadata))
            }
            PianoRollAction::Resync => app.resync_edited(),
            PianoRollAction::Audition => app.edit = app.edit.max(Some(super::SongEdit::Audition)),
            PianoRollAction::SetPlayhead(tick) => app.set_playhead(tick),
            PianoRollAction::Play => app.play(),
            PianoRollAction::Seek(tick) => app.seek = Some(tick),
            PianoRollAction::Note { key, velocity } => app.audio.send(match velocity {
                Some(velocity) => AudioCommand::NoteOn {
                    channel,
                    key,
                    velocity,
                },
                None => AudioCommand::NoteOff { channel, key },
            }),
        }
    }
    None
}

//! The piano roll view: [`PianoRoll`] in the editor area, one per channel, kept across visits so each channel keeps its
//! snap, zoom and scroll.

use std::collections::HashMap;

use banaforge_audio::device::AudioCommand;
use eframe::egui;

use super::{AppCtx, Nav, View};
use crate::song::Song;
use crate::ui::clips::LiveTake;
use crate::ui::piano_roll::{PianoRoll, PianoRollAction, RollOptions};

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

/// Compact circular actions above the native dark piano-roll tools.
fn vocal_round_button(
    ui: &mut egui::Ui,
    label: &str,
    hint: &str,
    selected: bool,
    enabled: bool,
) -> bool {
    let fill = if selected {
        egui::Color32::from_rgb(25, 89, 96)
    } else {
        egui::Color32::from_rgb(39, 45, 62)
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(label)
            .min_size(egui::vec2(30.0, 30.0))
            .corner_radius(egui::CornerRadius::same(255))
            .fill(fill)
            .stroke(egui::Stroke::new(
                1.0,
                egui::Color32::from_rgb(116, 255, 235),
            )),
    )
    .on_hover_text(hint)
    .clicked()
}

/// Draws the piano roll of `channel`'s note lane in `app.regions.editor`; back to the playlist once the channel is gone
/// (the app collapses an editor whose clip goes).
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx, channel: usize) -> Option<Nav> {
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
    let mut editor = app.regions.editor;
    if is_vocal {
        let height = app.cx.m.u(3.0).max(88.0);
        let toolbar = egui::Rect::from_min_max(
            editor.min,
            egui::pos2(editor.right(), (editor.top() + height).min(editor.bottom())),
        );
        let importing = app.files.importing_voicebank();
        let banks = app.files.voicebanks();
        let mut import = false;
        let mut toggle_pitch = false;
        let mut reset_pitch = false;
        let selected = state.voicebank_labels.entry(channel).or_insert_with(|| {
            app.song.channels[channel]
                .rack
                .modules
                .iter()
                .find(|m| m.type_id == 53)
                .map(|m| &m.file.preset_name)
                .filter(|name| banks.iter().any(|b| &b.name == *name))
                .cloned()
                .unwrap_or_default()
        });
        let ink = egui::Color32::from_rgb(116, 255, 235);
        ui.painter()
            .rect_filled(toolbar, 0.0, egui::Color32::from_rgb(27, 31, 46));
        ui.scope_builder(egui::UiBuilder::new().max_rect(toolbar.shrink(4.0)), |ui| {
            ui.visuals_mut().override_text_color = Some(egui::Color32::from_rgb(232, 236, 244));
            ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::from_rgb(39, 45, 62);
            ui.visuals_mut().widgets.hovered.bg_fill = egui::Color32::from_rgb(52, 62, 80);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    // Neutral profile, never a bundled singer's proprietary portrait.
                    let (avatar, _) =
                        ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        avatar.center(),
                        16.0,
                        egui::Color32::from_rgb(44, 55, 70),
                    );
                    ui.painter()
                        .circle_stroke(avatar.center(), 15.0, egui::Stroke::new(1.0, ink));
                    ui.painter().text(
                        avatar.center(),
                        egui::Align2::CENTER_CENTER,
                        "♪",
                        egui::FontId::proportional(22.0),
                        ink,
                    );
                    let title = if selected.is_empty() {
                        "Default voice"
                    } else {
                        selected.as_str()
                    };
                    egui::ComboBox::from_id_salt(("vocal-bank-header", channel))
                        .width((toolbar.width() - 160.0).clamp(90.0, 340.0))
                        .selected_text(title)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(selected, String::new(), "Default voice");
                            for bank in banks {
                                ui.selectable_value(selected, bank.name.clone(), &bank.name);
                            }
                            ui.separator();
                            ui.weak(
                                "Imported banks are indexed only; default audio remains active.",
                            );
                        });
                    ui.label(format!("{:.0} BPM", app.song.tempo_bpm));
                });
                ui.horizontal(|ui| {
                    toggle_pitch = vocal_round_button(
                        ui,
                        "⌁",
                        "Pitch Draw: edit cyan pitch nodes",
                        roll.pitch_draw_enabled,
                        true,
                    );
                    reset_pitch = vocal_round_button(
                        ui,
                        "—",
                        "Flatten selected pitch bends; keep lyrics",
                        false,
                        true,
                    );
                    import = vocal_round_button(ui, "+", "Import voicebank", false, !importing);
                    if importing {
                        ui.spinner();
                    }
                    ui.label(
                        egui::RichText::new("Default audio · lyrics are labels")
                            .small()
                            .color(ink),
                    );
                });
            });
        });
        if toggle_pitch {
            roll.pitch_draw_enabled = !roll.pitch_draw_enabled;
        }
        if reset_pitch && roll.reset_vocal_pitch(app.song) {
            app.song_edited(true);
        }
        if import {
            app.files.pick_voicebank();
        }
        editor.min.y = toolbar.bottom();
    }
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
        continuous: app.settings.continuous_scroll,
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
    let actions = roll.show(ui, editor, &app.cx, app.song, &opts);
    [app.settings.velocity_snap, app.settings.pan_snap] = roll.value_snap;
    app.settings.grid_size = roll.snap.size;
    app.settings.graph_tab_shift = roll.graph_tab_shift;
    for action in actions {
        match action {
            PianoRollAction::SongEdited { final_edit } => app.song_edited(final_edit),
            PianoRollAction::QuietEdit => app.quiet_edited(),
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

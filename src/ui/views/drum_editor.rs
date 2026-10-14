//! The drum editor view. The step sequencer fills the editor area. The pad editor strips are its drum mixer: a column
//! between the step view's row headers (which are the strips' header cells) and the steps, with the page tabs above it.
//!
//! The hexagon pads ([`Pads`]) are the bottom panel's, not the editor's: they take the keyboard's place in every
//! view while the current channel's first lane is a drum lane ([`pads_channel`]), and play that channel.
//!
//! The mixer folds: tapping the header of the current (selected) row toggles the drum mixer between collapsed and
//! expanded; the step view keeps that fold ([`StepSeqView::fold`]) and its header column of 3 U + c·12 U. The strips
//! fill the column's room past the header cells while it has any; the page tabs show only when fully expanded (c = 1).
//! The fold is in memory only: each app run starts collapsed, and this one step-sequencer editor keeps it across
//! closing, reopening, other channels and new songs (the app carries it over its reset of the views).
//!
//! The editor follows its current lane: kind 3 with flag 0 (`DESc`@1) → the step sequencer above; kind 3 with flag 1 →
//! the free note-grid. Neither editor has a mode toggle. [`State::mode`] is that lane's [`crate::song::DrumGrid`]: the
//! step view, or the free note-grid ([`DrumGridView`]), fills the editor area, and the drum mixer, its page tabs and
//! the graph lane's knob follow whichever shows. Both edit the current lane and clip ([`StepSeqView::lane`] /
//! [`StepSeqView::clip`]); each keeps its own fold, current row, graph lane and per-clip views, which the editor keeps
//! across channels and across closing and reopening. The pads and `Lock` are the same in both modes.
//!
//! The rows are the kit's pads, through [`DrumKit::slot_on_pad`]. Steps, swing and the row menu edit the song through
//! the funnel; the strips write their slot fields and send them live, or have the engine rebuilt, with no undo step
//! (pad params are rack state); the pads play their slots' effective notes ([`DrumKit::pad_note`]), a tap for at least
//! [`TAP_GATE`].

use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui::{self, Rect, UiBuilder, pos2, vec2};
use honeydew_audio::device::{AudioCommand, ModuleMeter};

use super::preset_picker::spawn;
use super::{AppCtx, Nav, Overlay, View};
use crate::app::TAP_GATE;
use crate::audio_engine::settings::{SLOT_CUT_GROUP, SLOT_OVERLAP, SLOT_REVERSE};
use crate::audio_link::AudioLink;
use crate::metrics::Metrics;
use crate::project_files::PresetSource;
use crate::sample_cache::{self, StretchJob};
use crate::song::{DrumGrid, DrumKit, LaneKind, SamplerSlot, Song};
use crate::theme::channel_color;
use crate::ui::drum_grid::DrumGridView;
use crate::ui::drums::{self, DrumsAction, DrumsState, PAD_COUNT, StripPage};
use crate::ui::focus_lost;
use crate::ui::graph_lane::{self, GraphLane};
use crate::ui::keyboard_settings::pads_rect;
use crate::ui::rack::PresetTarget;
use crate::ui::step_seq::{self, DrumPads, StepAction, StepSeqView};
use crate::ui::widgets::{Bar, ItemList, item_list_index};

/// The page tabs, in `StripPage::ALL` order.
const PAGES: [&str; 4] = ["Mixer", "Sample", "Envelope", "Seq"];
/// A pad's note-on velocity: always full, with no position velocity.
const PAD_VELOCITY: f32 = 1.0;
/// The `Lock` button's centre, in U from the transport bar's left edge: the slot after the mixer and keyboard settings
/// buttons, 1.75 U each.
const LOCK_X: f32 = 2.5 * 1.75;

impl DrumPads for DrumKit {
    /// Slot `i` is row (pad index) `i`.
    fn count(&self) -> usize {
        self.slots.len()
    }

    /// The row header's lines ([`drums::row_lines`]).
    fn names(&self, pad: usize) -> (&str, &str) {
        self.slot_on_pad(pad).map_or(("", ""), drums::row_lines)
    }

    /// The slot's stored note ([`SamplerSlot::midi_note`]).
    fn midi_note(&self, pad: usize) -> Option<u8> {
        self.slot_on_pad(pad).and_then(SamplerSlot::midi_note)
    }

    /// `MAIN`@1051, "" for none.
    fn custom_name(&self, pad: usize) -> &str {
        self.slot_on_pad(pad)
            .map_or("", |s| s.file.custom_pad_name.as_str())
    }
}

/// The kit side of a row action on slot (row) `pad`. Clone and Delete shift the later slots, and the `PADS` map with
/// them ([`pads_cloned`], [`pads_deleted`]); every slot keeps its id (its `SetParam` address, see
/// `send_slot_param`), and a copy gets its own.
fn edit_kit(kit: &mut DrumKit, action: &StepAction) {
    match action {
        // The copy has no custom name.
        StepAction::Clone { pad } => {
            if let Some(mut copy) = kit.slots.get(usize::from(*pad)).cloned() {
                copy.file.custom_pad_name.clear();
                kit.insert_slot(usize::from(*pad) + 1, copy);
                pads_cloned(&mut kit.file.pads, *pad);
            }
        }
        StepAction::Delete { pad } if usize::from(*pad) < kit.slots.len() => {
            kit.slots.remove(usize::from(*pad));
            pads_deleted(&mut kit.file.pads, *pad);
        }
        // `Set Name`: the custom pad name, MAIN@1051 ("" removes it); MAIN@12 stays.
        StepAction::Rename { pad, name } => {
            if let Some(slot) = kit.slots.get_mut(usize::from(*pad)) {
                slot.file.custom_pad_name.clone_from(name);
            }
        }
        // `LINk` @8: the stored note, or none (0xFFFF).
        StepAction::SetMidiNote { pad, note } => {
            if let Some(slot) = kit.slots.get_mut(usize::from(*pad)) {
                slot.file.midi_key = note.map_or(u16::MAX, u16::from);
            }
        }
        _ => {}
    }
}

/// The `PADS` map, read as the slots in pad order ([`drums::pad_slot`]), after slot `k` was deleted: its entry goes,
/// the later slots' entries go down one, and the freed last index goes last. An identity map stays the identity.
fn pads_deleted(pads: &mut [u8; PAD_COUNT], k: u8) {
    let order = (pads.iter())
        .filter(|&&s| s != k)
        .map(|&s| if s > k { s - 1 } else { s });
    *pads = pad_order(order.collect());
}

/// The `PADS` map after a copy of slot `k` went in at `k + 1`: the later slots' entries go up one, the copy's goes
/// right after `k`'s, and the index pushed past the last pad drops out. An identity map stays the identity.
fn pads_cloned(pads: &mut [u8; PAD_COUNT], k: u8) {
    let mut order: Vec<u8> = (pads.iter())
        .map(|&s| if s > k { s.saturating_add(1) } else { s })
        .collect();
    let at = order
        .iter()
        .position(|&s| s == k)
        .map_or(order.len(), |i| i + 1);
    order.insert(at, k.saturating_add(1));
    order.retain(|&s| usize::from(s) != PAD_COUNT);
    *pads = pad_order(order);
}

/// `order` as a `PADS` map: cut to the pad count, or filled up with the last index.
fn pad_order(mut order: Vec<u8>) -> [u8; PAD_COUNT] {
    order.resize(PAD_COUNT, PAD_COUNT as u8 - 1);
    std::array::from_fn(|i| order[i])
}

/// What the drum editor keeps between frames: the views of the channel it shows.
#[derive(Default)]
pub struct State {
    channel: Option<usize>,
    pub(crate) step: StepSeqView,
    /// The free note-grid, for a notes lane. It shares the step view's remembered values ("both drum modes").
    pub(crate) grid: DrumGridView,
    /// The strips' page, unbound knob values and touch.
    pub(crate) drums: DrumsState,
    /// The editor was drawn this frame: the transport shows its `Lock` (see [`after_chrome`]).
    drawn: bool,
    /// The bottom panel's pads, kept here across the editor's channel switches.
    pub(crate) pads: Pads,
    /// A strip control's touch, with the channel: the app takes it after the frame for its recent control.
    pub(crate) touch: Option<(usize, crate::ui::ctrl_menu::Touch)>,
    /// The audio cache's `STR1` renders streaming in on the pick worker (a `Stretch` re-import, a pad pick, a render a
    /// rebuild missed), kept across channel switches: the engine is rebuilt with their whole renders once the last
    /// lands ([`restretched`]).
    stretches: Vec<Receiver<StretchJob>>,
    /// A landed re-import went into the audio's cache, for the rebuild after the last.
    stretch_ready: bool,
    /// The strips changed rack state that the undo history has not been told of: it is, once no pointer is down
    /// ([`after_chrome`]), so a drag is one report.
    rack_pending: bool,
    /// A drum row was removed this frame: the app clears the undo history after the edit.
    pub(crate) row_removed: bool,
}

/// The hex pads in the keyboard slot: what they keep between frames, whatever view shows.
#[derive(Default)]
pub struct Pads {
    /// The channel they were last drawn for.
    channel: Option<usize>,
    /// The fingers on them ([`DrumsState::fingers`]).
    drums: DrumsState,
    /// The pads held and the key each played: a note-off uses it, whatever the kit has become since.
    held: Vec<(usize, u8)>,
    /// A tapped pad's note while its gate runs, as (channel, key, end of the gate in egui input time): see
    /// [`TAP_GATE`]. It outlives the pads, so its `NoteOff` never goes out early.
    tap: Option<(usize, u8, f64)>,
    /// The presses and releases this frame, as (channel, key, the press's velocity), for REC ([`take_hits`]).
    hits: Vec<(usize, u8, Option<f32>)>,
}

/// What the pads played since the last call, as live keys: (channel, key, the press's velocity, `None` for a release),
/// in order. REC records them as it does every live key; the app takes them every frame.
pub fn take_hits(state: &mut State) -> Vec<(usize, u8, Option<f32>)> {
    std::mem::take(&mut state.pads.hits)
}

impl State {
    /// Waits until the first `first` renders streaming in (in the order they started) have finished on the worker,
    /// without taking them: a test's wait for them to land, which a loaded machine only makes longer. Never for a render
    /// under this thread's held worker gate, which it would wait on forever.
    #[cfg(test)]
    pub(crate) fn wait_stretches(&mut self, first: usize) {
        for done in self.stretches.iter_mut().take(first) {
            if let Ok(job) = done.recv() {
                let (tx, rx) = std::sync::mpsc::channel();
                drop(tx.send(job));
                *done = rx;
            }
        }
    }

    /// How many `Stretch` re-imports are still rendering.
    #[cfg(test)]
    pub(crate) fn stretching(&self) -> usize {
        self.stretches.len()
    }

    /// The editor's mode: its current lane's grid ([`crate::song::Lane::grid`], `DESc` @1), `None` while that is no
    /// drum lane. The current lane is [`StepSeqView::lane`] in either mode.
    pub(crate) fn mode(&self, song: &Song) -> Option<DrumGrid> {
        let lane = song
            .channels
            .get(self.channel?)?
            .lanes
            .get(self.step.lane)?;
        (lane.kind == LaneKind::Drums).then_some(lane.grid)
    }

    /// Makes pad `pad` the current row of the editor showing ([`Self::mode`]).
    pub(crate) fn select_row(&mut self, song: &Song, pad: usize) {
        match self.mode(song) {
            Some(DrumGrid::Notes) => self.grid.current = pad,
            _ => self.step.selected = pad,
        }
    }
}

/// Draws `channel`'s drum editor in `app.regions.editor`; back to the playlist for a channel that is gone or has no
/// pads. The editor follows its current lane ([`State::mode`]): the step sequencer on a step lane, the free note-grid
/// on a notes lane.
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx, channel: usize) -> Option<Nav> {
    let Some(ch) = app.song.channels.get(channel) else {
        return Some(Nav::Show(View::Playlist));
    };
    let Some(kit) = ch.drum_kit.as_ref() else {
        return Some(Nav::Show(View::Playlist));
    };
    switch(state, channel);
    let steps = state.mode(app.song) != Some(DrumGrid::Notes);
    let (editor, m) = (app.regions.editor, app.cx.m);
    let color = channel_color(ch.hue, app.cx.theme).to_color32();

    // The step view paints the whole editor area; the mixer goes on top, in the room it leaves.
    // Its own ids per channel for the strips (and the pads, [`show_pads`]): a press held across a switch to another
    // drum channel does not carry onto them. The step view's ids are global (`step_seq`), so such a press does carry
    // onto its grid.
    let mut ui = ui.new_child(
        UiBuilder::new()
            .id(("drum_editor", channel))
            .max_rect(editor),
    );
    let playhead = app.snapshot.map_or(0, |s| s.transport_tick);
    let (grid, step_view) = (&mut state.grid, &mut state.step);
    (step_view.covered, grid.covered) = (app.covered, app.covered);
    (step_view.playhead, grid.playhead) = (playhead, playhead);
    let lead = app.audio.playhead_lead();
    (step_view.lead, grid.lead) = (lead, lead);
    (step_view.playing, grid.playing) = (app.playing, app.playing);
    (step_view.armed, grid.armed) = (app.armed, app.armed);
    (step_view.previewing, grid.previewing) = (app.previewing, app.previewing);
    let levels = slot_peaks(ui.ctx(), app, channel, kit);
    grid.levels.clone_from(&levels);
    step_view.levels = levels;
    step_view.follow = app.settings.continuous_scroll;
    let step = if steps {
        state.step.show(&mut ui, &app.cx, app.song, channel, kit)
    } else {
        let at = (state.step.lane, state.step.clip);
        let remembered = &mut state.step.remembered;
        state
            .grid
            .show(&mut ui, &app.cx, app.song, channel, at, kit, remembered)
    };
    // The mixer as the shown view laid it out this frame: its room, and the tabs once it is fully out.
    let (mixer_w, c, band, graph) = if steps {
        let s = &state.step;
        (s.mixer_w, s.fold.c, s.row_band(), s.graph)
    } else {
        let g = &state.grid;
        (g.mixer_w, g.fold.c, g.row_band(), g.graph)
    };
    if let Some(tabs) = tabs(editor, c, m) {
        let mut page = StripPage::ALL
            .iter()
            .position(|&p| p == state.drums.page)
            .unwrap_or(0);
        let list = ItemList::new(ui.id().with("pages"), tabs, &PAGES, Bar::Bottom, color);
        item_list_index(&mut ui, &app.cx, &list, &mut page);
        state.drums.page = StripPage::ALL[page];
    }
    state.drums.sends = drums::send_channels(app.song);
    let links = crate::midi::links::links(&app.song.channels[channel].file.cc_links);
    let kit = app.song.channels[channel]
        .drum_kit
        .as_mut()
        .expect("checked above");
    // The strips sit on the step rows and scroll with them, clipped to the rows' area.
    let mut actions = Vec::new();
    if let Some((top, pitch)) = band {
        let right = editor.left() + m.u(step_seq::HEADER_W) + mixer_w;
        let strips = Rect::from_min_max(
            pos2(editor.left(), top),
            pos2(right, top + pitch * kit.slots.len() as f32),
        );
        let body = rows(editor, &graph, m);
        // Collapsed, the column has no room for a page.
        if mixer_w > 0.0 {
            let outer = ui.clip_rect();
            ui.set_clip_rect(outer.intersect(body));
            let drums = &mut state.drums;
            actions = drums::show_strips(&mut ui, &app.cx, strips, kit, drums, color, &links);
            ui.set_clip_rect(outer);
            if let Some(touch) = state.drums.touch.take() {
                state.touch = Some((channel, touch));
            }
            // The graph lane's spacer, the mixer's room in the lane: the current row's sample.
            if let Some(lane) = step_seq::lane_split(editor, m, &graph).1 {
                let left = editor.left() + m.u(step_seq::HEADER_W);
                let rows = graph_lane::content(lane, m).y_range();
                let spacer = Rect::from_x_y_ranges(left..=left + mixer_w, rows);
                let row = if steps {
                    state.step.selected
                } else {
                    state.grid.current
                };
                let read = sample_cache::reader(app.files);
                if let Some(slot) = kit.slot_on_pad(row)
                    && let Some(wave) = app.audio.samples.waveform(slot, &read)
                {
                    let painter = ui.painter_at(spacer);
                    graph_lane::spacer_wave(&painter, spacer, &wave, slot.file.reverse, color, m);
                }
            }
        }
        // Over the strips.
        if steps {
            state.step.graph_knob(&mut ui, &app.cx, editor);
        } else {
            state.grid.graph_knob(&mut ui, &app.cx, editor);
        }
    }
    state.drawn = true;

    // No strip control makes an undo step: every one is rack state, which every undo step keeps, and what the audio
    // reads only at build (send targets, the beat offset, a stretch) gets a new engine. Only `Create FX Channel` is a
    // song edit: it adds a channel. The bound controls and a new `Stretch` ratio set the song-changed flag; `REV`,
    // `Poly`, `Cut...`, `Shift` and an `AUX` target do not.
    let (mut rack, mut rebuild, mut create, mut load) = (false, false, None, None);
    for action in actions {
        match action {
            // The kit has the new value already.
            DrumsAction::Param { slot, param: index } => {
                let slot = &kit.slots[slot];
                send_slot_param(app.audio, channel, slot.file.id, index, slot.params[index]);
                app.files.song_changed();
                rack = true;
            }
            DrumsAction::Reverse(slot) | DrumsAction::Poly(slot) | DrumsAction::CutGroup(slot) => {
                let (file, group) = (&kit.slots[slot].file, kit.slots[slot].cut_group);
                let (key, value) = match action {
                    DrumsAction::Reverse(_) => (SLOT_REVERSE, f32::from(u8::from(file.reverse))),
                    DrumsAction::Poly(_) => {
                        (SLOT_OVERLAP, f32::from(u8::from(file.overlapping_voices)))
                    }
                    _ => (SLOT_CUT_GROUP, f32::from(group)),
                };
                send_slot_setting(app.audio, channel, file.id, key, value);
                rack = true;
            }
            // A pad's Shift reaches its hits on every step and notes lane at once, at this rebuild.
            DrumsAction::SendTarget(_) | DrumsAction::Shift(_) => rebuild = true,
            // The new ratio's first frames render now from the sample the cache holds, and the engine is rebuilt at
            // once to play them; the rest streams in on the pick worker ([`restretched`]). With nothing to render (no
            // stretch, a render the cache has, no sample) only the rebuild.
            DrumsAction::Stretch(slot) => {
                app.files.song_changed();
                (rack, rebuild) = (true, true);
                app.audio.samples.stream_slot(&kit.slots[slot]);
            }
            DrumsAction::CreateFxChannel(slot) => create = Some(slot),
            DrumsAction::LoadSample(slot) => load = Some(slot),
            // The strips play no pads.
            DrumsAction::PadDown(_) | DrumsAction::PadUp(_) | DrumsAction::Swap(..) => {}
        }
    }
    state.rack_pending |= rack;
    if rebuild {
        app.rack_state_rebuilt();
    }
    // A new send channel goes last: no channel index moves.
    if let Some(slot) = create
        && drums::create_pad_send(app.song, channel, slot).is_some()
    {
        app.song_edited(true);
    }
    // Icon 21: its row becomes current, then the row menu's `Load Sample`.
    let load = load.and_then(|row| {
        state.select_row(app.song, row);
        let pad = Some(u8::try_from(row).ok()?);
        Some(Nav::Open(Overlay::SamplePicker { channel, pad }))
    });
    if let Some(action) = step {
        // The row menu's file items and the `+` open a picker or a dialog: no edit here.
        let ch = &app.song.channels[channel];
        let open = |overlay| Some(Nav::Open(overlay));
        match action {
            StepAction::LoadSample { pad } => {
                return open(Overlay::SamplePicker {
                    channel,
                    pad: Some(pad),
                });
            }
            StepAction::AddDrum => return open(Overlay::SamplePicker { channel, pad: None }),
            StepAction::LoadDrumset => {
                let target = PresetTarget::Drumset;
                return open(Overlay::PresetPicker { channel, target });
            }
            // The kit as it is now: not a song edit.
            StepAction::SaveDrumset => {
                let (name, kit) = (ch.name.clone(), ch.drum_kit.clone().unwrap_or_default());
                app.files
                    .ask_preset(PresetSource::Drumset { name, kit }, ch.id);
                return None;
            }
            _ => {}
        }
        // A step tap adds a hit on an empty step, else deletes the step's hits.
        let hits = |song: &Song| {
            let clips = song.channels[channel].lanes.iter().flat_map(|l| &l.clips);
            clips.map(|c| c.events.len()).sum::<usize>()
        };
        let before = hits(app.song);
        if steps {
            state.step.apply(app.song, channel, &action);
        } else {
            let at = (state.step.lane, state.step.clip);
            state.grid.apply(app.song, channel, at, &action);
        }
        let kit = app.song.channels[channel]
            .drum_kit
            .as_mut()
            .expect("checked above");
        edit_kit(kit, &action);
        match action {
            StepAction::Swing { final_edit, .. } => app.song_edited(final_edit),
            // The play loop is no edit; the playhead is the transport's.
            StepAction::PlayLoop(_) => {}
            StepAction::SetPlayhead(tick) => app.set_playhead(tick),
            StepAction::Play => app.play(),
            StepAction::Seek(tick) => app.seek = Some(tick),
            // A graph-lane drag: one undo step, closed by its end.
            StepAction::Graph { .. } => app.song_edited(false),
            // The free note-grid's gestures: a drag's step stays open until its release.
            StepAction::Hits { quiet: true, .. } => app.quiet_edited(),
            StepAction::Hits { final_edit, .. } => app.song_edited(final_edit),
            StepAction::GraphEnd => app.song_edited(true),
            // A graph-lane touch that changed nothing: its own undo re-syncs.
            StepAction::GraphCancel(_) => app.resync_edited(),
            StepAction::Delete { .. } => {
                state.row_removed = true;
                app.song_edited(true);
            }
            // A tap on an empty step (a free-grid tap's new hit above) adds a hit with no re-sync; Clear and Fill make
            // none either, though they may take hits away.
            StepAction::Toggle { .. } if hits(app.song) > before => app.quiet_edited(),
            StepAction::Fill { .. } | StepAction::Clear { .. } => app.quiet_edited(),
            _ => app.song_edited(true),
        }
    }
    load
}

/// Points the editor at clip `clip` (its id) of `channel`'s lane `lane`, switching channel as a visit does.
pub(crate) fn open(state: &mut State, channel: usize, lane: usize, clip: u32) {
    switch(state, channel);
    (state.step.lane, state.step.clip) = (lane, Some(clip));
}

/// Another channel's editor: a fresh view, keeping the transport's Lock, every clip's zoom and scroll, the mixer's
/// fold, and the pads.
fn switch(state: &mut State, channel: usize) {
    if state.channel == Some(channel) {
        return;
    }
    let (lock, views) = (state.step.lock, std::mem::take(&mut state.step.views));
    let fold = state.step.fold;
    let grid = std::mem::take(&mut state.grid).kept();
    *state = State {
        channel: Some(channel),
        grid,
        pads: std::mem::take(&mut state.pads),
        stretches: std::mem::take(&mut state.stretches),
        stretch_ready: state.stretch_ready,
        rack_pending: state.rack_pending,
        ..State::default()
    };
    state.step.lock = lock;
    state.step.views = views;
    state.step.fold = fold;
}

/// Follows the editor's channel to `channel(index)`, and its lane, every clip's view and the step selection to
/// `track(channel, lane)`, after channels or tracks moved; the pads' channel and tapped note follow too. A gone
/// channel's view starts afresh (keeping the transport's `Lock` and the mixer's fold); a gone channel's held and
/// tapped pads' notes stop; a gone lane's view goes back to the channel's lane 0.
pub(crate) fn rekey(
    state: &mut State,
    audio: &mut AudioLink,
    channel: &dyn Fn(usize) -> Option<usize>,
    track: &dyn Fn(usize, usize) -> Option<(usize, usize)>,
) {
    let pads = &mut state.pads;
    if let Some((c, key, off)) = pads.tap {
        match channel(c) {
            Some(c) => pads.tap = Some((c, key, off)),
            None => end_tap(pads, audio),
        }
    }
    match pads.channel.map(channel) {
        Some(Some(c)) => pads.channel = Some(c),
        Some(None) => {
            release(pads, audio);
            pads.channel = None;
        }
        None => {}
    }
    let key = |(c, l, id)| track(c, l).map(|(c, l)| (c, l, id));
    let views = std::mem::take(&mut state.step.views);
    let views = (views.into_iter()).filter_map(|(k, view)| key(k).map(|k| (k, view)));
    state.step.views = views.collect();
    state.step.selection_key = state.step.selection_key.and_then(key);
    state.grid.rekey(&key);
    let Some(old) = state.channel else {
        return;
    };
    match (channel(old), track(old, state.step.lane)) {
        (Some(_), Some((c, lane))) => (state.channel, state.step.lane) = (Some(c), lane),
        (Some(c), None) => {
            state.channel = Some(c);
            (state.step.lane, state.step.clip) = (0, None);
        }
        (None, _) => {
            let (lock, views) = (state.step.lock, std::mem::take(&mut state.step.views));
            let fold = state.step.fold;
            let grid = std::mem::take(&mut state.grid).kept();
            *state = State {
                grid,
                pads: std::mem::take(&mut state.pads),
                stretches: std::mem::take(&mut state.stretches),
                stretch_ready: state.stretch_ready,
                rack_pending: state.rack_pending,
                ..State::default()
            };
            (state.step.lock, state.step.views, state.step.fold) = (lock, views, fold);
        }
    }
}

/// After the chrome, every frame: the drum view's `Lock` in the transport bar while the drum editor is drawn, and
/// otherwise the close of its popups (the row menu and its prompts, the swing popup, the snap, `Cut...`, `AUX` and
/// region menus and the range's `+`), so that none comes back with the editor. Then the current channel's pads, if it has
/// any (else the release of a pad still held), and the end of a tap's gate: the frame that ends it is asked for, so
/// the note never hangs.
pub fn after_chrome(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx) {
    if std::mem::take(&mut state.drawn) {
        let transport = app.regions.transport;
        let at = pos2(transport.left() + app.cx.m.u(LOCK_X), transport.center().y);
        state.step.lock_button(ui, &app.cx, at);
    } else {
        // A drag the hidden editor ended had edits out: their undo step closes. Both views close.
        if state.step.close_popups() | state.grid.close_popups() {
            app.song_edited(true);
        }
        state.drums.menu = None;
    }
    // The strips' rack state, once no pointer is down (a drag's release, a tap's own frame, a key at once), or on a
    // focus loss, after which the release may never come.
    if state.rack_pending && ui.input(|i| !i.pointer.any_down() || focus_lost(i)) {
        state.rack_pending = false;
        app.rack_state_edited();
    }
    restretched(state, ui, app);
    let pads = &mut state.pads;
    // A keys area exactly 0 tall hides them.
    let shown = app.regions.keyboard.height() != 0.0;
    match pads_channel(app.song, *app.selected).filter(|_| shown) {
        Some(channel) => show_pads(pads, ui, app, channel),
        None => release(pads, app.audio),
    }
    if let Some((.., off)) = pads.tap {
        // A hidden web page runs no frame to end the gate: the focus-loss frame does.
        let (now, lost) = ui.input(|i| (i.time, focus_lost(i)));
        if now < off && !lost {
            ui.ctx().request_repaint_after_secs((off - now) as f32);
        } else {
            end_tap(pads, app.audio);
        }
    }
}

/// `Set MIDI Note`: while the step view or the free note-grid waits for a MIDI key, `key` becomes the waiting pad's
/// note in the kit, and the prompt closes. Whether it did, which the app makes one undo step.
pub fn learned_note(state: &mut State, song: &mut Song, key: u8) -> bool {
    let channel = state.channel.and_then(|c| song.channels.get_mut(c));
    let Some(kit) = channel.and_then(|c| c.drum_kit.as_mut()) else {
        return false;
    };
    let Some(action) = (state.step.midi_key(key)).or_else(|| state.grid.midi_key(key)) else {
        return false;
    };
    edit_kit(kit, &action);
    true
}

/// The channel whose pads replace the keyboard: the current channel, while its first lane (in lane order) is a drum
/// lane. Not the selected lane, nor the open editor's channel; an audio first lane has the sample row
/// ([`crate::ui::sample_row::current`]), anything else the keyboard.
pub fn pads_channel(song: &Song, current: usize) -> Option<usize> {
    let first = song.channels.get(current)?.lanes.first()?;
    (first.kind == LaneKind::Drums).then_some(current)
}

/// `channel`'s pads in the pads rect of `app.regions.keyboard` ([`pads_rect`]: 0.35 U under the keys' top, overflowing
/// below the panel when the grid is taller), playing its slots' notes: each finger's pad held until its lift (or
/// until the pads go), a tap for [`TAP_GATE`]. A pad dropped on another swaps their slots in the kit's `PADS` map
/// ([`drums::pad_slot`]), an undo step with nothing for the audio; unless the pads are locked (`lockpads`). A channel
/// without a kit has only unused pads. Switching channel lets go of the held pads first; the ids are per channel, so
/// a press carried over plays nothing.
fn show_pads(pads: &mut Pads, ui: &mut egui::Ui, app: &mut AppCtx, channel: usize) {
    if pads.channel != Some(channel) {
        release(pads, app.audio);
        pads.channel = Some(channel);
    }
    let ch = &app.song.channels[channel];
    let empty = DrumKit::default();
    let kit = ch.drum_kit.as_ref().unwrap_or(&empty);
    let color = channel_color(ch.hue, app.cx.theme).to_color32();
    let keyboard = pads_rect(app.regions.keyboard, app.cx.m);
    let mut ui = ui.new_child(
        UiBuilder::new()
            .id(("drum pads", channel))
            .max_rect(keyboard),
    );
    let peaks = slot_peaks(ui.ctx(), app, channel, kit).into_iter();
    pads.drums.peaks = peaks.map(|[l, r]| l.max(r)).collect();
    let locked = app.settings.lock_pads;
    let actions = drums::show_pads(
        &mut ui,
        &app.cx,
        keyboard,
        kit,
        &mut pads.drums,
        color,
        locked,
    );
    let (now, mut pressed, mut swap) = (ui.input(|i| i.time), Vec::new(), None);
    for action in actions {
        let command = match action {
            // A pad with no slot plays nothing.
            DrumsAction::PadDown(pad) => match kit.pad_note(drums::pad_slot(kit, pad)) {
                Some(key) => {
                    // A tapped pad still in its gate stops first.
                    end_tap(pads, app.audio);
                    pads.held.push((pad, key));
                    pressed.push(pad);
                    pads.hits.push((channel, key, Some(PAD_VELOCITY)));
                    AudioCommand::NoteOn {
                        channel,
                        key,
                        velocity: PAD_VELOCITY,
                    }
                }
                None => continue,
            },
            DrumsAction::PadUp(pad) => {
                let Some(i) = pads.held.iter().position(|h| h.0 == pad) else {
                    continue;
                };
                let (_, key) = pads.held.remove(i);
                pads.hits.push((channel, key, None));
                // Released in the frame of its press: a tap, which sounds until its gate ends (`after_chrome`).
                if pressed.contains(&pad) {
                    pads.tap = Some((channel, key, now + TAP_GATE));
                    continue;
                }
                AudioCommand::NoteOff { channel, key }
            }
            DrumsAction::Swap(a, b) => {
                swap = Some((a, b));
                continue;
            }
            _ => continue,
        };
        app.audio.send(command);
    }
    if let (Some((a, b)), Some(kit)) = (swap, app.song.channels[channel].drum_kit.as_mut()) {
        kit.file.pads.swap(a, b);
        app.param_edited();
    }
}

/// Each slot's output peaks (L, R) in `channel`'s `kit`, in slot order, from this frame's meters: the engine keys them
/// (channel, −slot id). None without a running stream; while there is one, the next frame comes within 50 ms, as for
/// the rack's meters: the audio moves with no input to repaint for.
fn slot_peaks(ctx: &egui::Context, app: &AppCtx, channel: usize, kit: &DrumKit) -> Vec<[f32; 2]> {
    let Some(snapshot) = app.snapshot.filter(|s| !s.stream_failed) else {
        return Vec::new();
    };
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
    let meters = &snapshot.module_meters;
    let peak = |id: u32| {
        // The engine keys no slot without an id: −0 would be the channel's own meter.
        let key = |m: &&ModuleMeter| {
            id > 0 && m.key.channel == channel && m.key.instance_id == -(id as i32)
        };
        meters
            .iter()
            .find(key)
            .map_or([0.0; 2], |m| [m.value.peak_left, m.value.peak_right])
    };
    kit.slots.iter().map(|slot| peak(slot.file.id)).collect()
}

/// Stops every note the pads sound: the held pad's and the tapped pad's still in its gate.
pub(crate) fn stop_notes(state: &mut State, audio: &mut AudioLink) {
    release(&mut state.pads, audio);
    end_tap(&mut state.pads, audio);
}

/// Preview's handover silenced every note (the app's `live_notes_silenced`): the pads let go of theirs as the keys do,
/// with their key-ups for REC and no note-off of their own, which would choke a pad's cut group at once rather than let
/// it ring out; a finger still on a pad holds nothing until it lifts, and a tapped pad's gate ends with no note-off
/// either. A picked-up pad's drag goes on to its drop.
pub(crate) fn silenced(state: &mut State) {
    let pads = &mut state.pads;
    pads.drums.fingers.retain(|f| f.lifted.is_some());
    pads.tap = None;
    if let Some(channel) = pads.channel {
        for (_, key) in pads.held.drain(..) {
            pads.hits.push((channel, key, None));
        }
    }
}

/// Stops the tapped pad's note still in its gate, if any.
fn end_tap(pads: &mut Pads, audio: &mut AudioLink) {
    if let Some((channel, key, _)) = pads.tap.take() {
        audio.send(AudioCommand::NoteOff { channel, key });
    }
}

/// Lets go of the pads held down, with a `NoteOff` for each key they played.
fn release(pads: &mut Pads, audio: &mut AudioLink) {
    pads.drums.fingers.clear();
    if let Some(channel) = pads.channel {
        for (_, key) in pads.held.drain(..) {
            pads.hits.push((channel, key, None));
            audio.send(AudioCommand::NoteOff { channel, key });
        }
    }
}

/// Sends `value` for param `param` of the drum slot with id `slot` (`LINk` id) in `channel`'s kit: instance `-slot`,
/// the negative-instance target. The id stays when `Clone` or `Delete` moves the slot. The one place that builds the
/// address.
fn send_slot_param(audio: &mut AudioLink, channel: usize, slot: u32, param: usize, value: f32) {
    audio.send(AudioCommand::SetParam {
        channel,
        // No kit gets near 2^31 slot ids (a file's are capped at `flm::MAX_ID`).
        instance_id: -(slot as i32),
        index: param as i32,
        value,
    });
}

/// Every frame: the audio cache's renders started streaming in since the last frame go to the pick worker for their
/// rest; each that landed goes into the cache in place of its stream, unless the cache has let that stream go (no slot
/// plays its ratio any more, or the file was read again), and once none is left rendering, the engine is rebuilt with
/// what went in, with no undo step.
fn restretched(state: &mut State, ui: &egui::Ui, app: &mut AppCtx) {
    let (ready, samples) = (&mut state.stretch_ready, &mut app.audio.samples);
    let started = samples.take_jobs().into_iter();
    (state.stretches).extend(started.map(|job| spawn(move || job.render())));
    state.stretches.retain(|done| match done.try_recv() {
        Err(TryRecvError::Empty) => true,
        Err(TryRecvError::Disconnected) => false,
        Ok(job) => {
            *ready |= samples.put_stretch(job);
            false
        }
    });
    if !state.stretches.is_empty() {
        ui.ctx().request_repaint();
    } else if std::mem::take(&mut state.stretch_ready) {
        app.rack_state_rebuilt();
    }
}

/// Sends `MAIN` setting `key` ([`SLOT_REVERSE`], [`SLOT_CUT_GROUP`], [`SLOT_OVERLAP`]) = `value` of the drum slot with
/// id `slot` in `channel`'s kit: a `SetSetting` on instance −slot, addressed as [`send_slot_param`] addresses a param.
fn send_slot_setting(audio: &mut AudioLink, channel: usize, slot: u32, key: u32, value: f32) {
    audio.send(AudioCommand::SetSetting {
        channel,
        instance_id: -(slot as i32),
        key,
        value,
    });
}

/// The page tabs for the mixer folded out to `c`, in `editor`'s ruler row: (x + 3 U, top, column width − 3 U,
/// 1.5 U − 1 px), only when it is fully out (c = 1).
fn tabs(editor: Rect, c: f32, m: &Metrics) -> Option<Rect> {
    let at = editor.min + vec2(m.u(step_seq::HEADER_W), 0.0);
    let size = vec2(m.u(step_seq::MIXER_W), m.u(step_seq::TOP_H) - 1.0 / m.ppp());
    (c == 1.0).then(|| Rect::from_min_size(at, size))
}

/// The rows' area in `editor`, between the top strip and the graph lane, if it shows.
fn rows(editor: Rect, graph: &GraphLane, m: &Metrics) -> Rect {
    let top = editor.top() + m.u(step_seq::TOP_H);
    step_seq::lane_split(editor, m, graph).0.with_min_y(top)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_link::testing::Recorder;
    use crate::ui::step_seq::RowView;

    /// Channel 3 removed: 1 and 2 stay, 4 becomes 3. Track 1 of channel 2 removed too.
    fn channel(c: usize) -> Option<usize> {
        match c {
            3 => None,
            4 => Some(3),
            c => Some(c),
        }
    }
    fn track(c: usize, l: usize) -> Option<(usize, usize)> {
        let c = channel(c)?;
        match (c, l) {
            (2, 1) => None,
            (2, l) if l > 1 => Some((2, l - 1)),
            _ => Some((c, l)),
        }
    }

    fn link() -> (AudioLink, Recorder) {
        let (mut audio, rec) = (AudioLink::default(), Recorder::default());
        audio.start(&Song::default(), false, &|_| None, |_, _| rec.sink());
        rec.take();
        (audio, rec)
    }

    fn view(pitch: f32) -> RowView {
        RowView { pitch, scroll: 0.0 }
    }

    #[test]
    fn kit_actions_keep_slot_ids_and_store_midi_notes() {
        let mut kit = DrumKit::default_kit();
        let ids = |kit: &DrumKit| kit.slots.iter().map(|s| s.file.id).collect::<Vec<_>>();
        edit_kit(&mut kit, &StepAction::Clone { pad: 1 });
        assert_eq!(ids(&kit), [1, 2, 9, 3, 4, 5, 6, 7, 8]);
        assert_eq!(kit.slots[2].name, kit.slots[1].name);
        edit_kit(&mut kit, &StepAction::Delete { pad: 0 });
        assert_eq!(ids(&kit), [2, 9, 3, 4, 5, 6, 7, 8]);
        let set = |note| StepAction::SetMidiNote { pad: 1, note };
        edit_kit(&mut kit, &set(Some(40)));
        assert_eq!(kit.pad_note(1), Some(40));
        edit_kit(&mut kit, &set(None));
        assert_eq!(kit.pad_note(1), Some(49));
    }

    #[test]
    fn delete_and_clone_keep_a_rearranged_pad_layout_in_pad_order() {
        let shown = |kit: &DrumKit| -> Vec<String> {
            (0..5)
                .map(|pad| kit.slot_on_pad(drums::pad_slot(kit, pad)))
                .map(|s| s.map_or_else(String::new, |s| s.name.replace("Synth ", "")))
                .collect()
        };
        // An identity map stays the identity.
        let identity: [u8; 22] = std::array::from_fn(|i| i as u8);
        let mut kit = DrumKit::default_kit();
        edit_kit(&mut kit, &StepAction::Delete { pad: 1 });
        edit_kit(&mut kit, &StepAction::Clone { pad: 2 });
        assert_eq!(kit.file.pads, identity);
        // The kick (pad 0) and the closed hat (pad 3) swapped.
        let mut kit = DrumKit::default_kit();
        kit.file.pads.swap(0, 3);
        assert_eq!(
            shown(&kit),
            ["Closed Hat", "Snare", "Clap", "Kick", "Open Hat"]
        );
        // Deleting the snare (slot 1) takes its pad out of the order: the later pads move up one, the freed index goes
        // last.
        edit_kit(&mut kit, &StepAction::Delete { pad: 1 });
        assert_eq!(kit.file.pads[..5], [2, 1, 0, 3, 4]);
        assert_eq!(kit.file.pads[21], 21);
        assert_eq!(
            shown(&kit),
            ["Closed Hat", "Clap", "Kick", "Open Hat", "Tom"]
        );
        // Cloning the hat (now slot 2) puts the copy on the pad after it.
        edit_kit(&mut kit, &StepAction::Clone { pad: 2 });
        assert_eq!(kit.file.pads[..5], [2, 3, 1, 0, 4]);
        assert_eq!(
            shown(&kit),
            ["Closed Hat", "Closed Hat", "Clap", "Kick", "Open Hat"]
        );
        let mut sorted = kit.file.pads;
        sorted.sort_unstable();
        assert_eq!(sorted, identity);
    }

    #[test]
    fn set_name_writes_only_the_custom_name_and_clone_does_not_copy_it() {
        // `Set Name` sets MAIN@1051 (empty text removes it) and never changes MAIN@12; `Clone` does not copy the custom
        // name.
        let mut kit = DrumKit::default_kit();
        let rename = |name: &str| StepAction::Rename {
            pad: 1,
            name: name.into(),
        };
        edit_kit(&mut kit, &rename("Mine"));
        assert_eq!(kit.slots[1].file.custom_pad_name, "Mine");
        assert_eq!(kit.slots[1].name, "Synth Snare");
        edit_kit(&mut kit, &StepAction::Clone { pad: 1 });
        assert_eq!(kit.slots[1].file.custom_pad_name, "Mine");
        let copy = &kit.slots[2];
        assert_eq!(
            (copy.name.as_str(), copy.file.custom_pad_name.as_str()),
            ("Synth Snare", "")
        );
        edit_kit(&mut kit, &rename(""));
        assert_eq!(kit.slots[1].file.custom_pad_name, "");
        assert_eq!(kit.slots[1].name, "Synth Snare");
    }

    #[test]
    fn the_views_and_a_tapped_note_follow_their_channel_and_lane() {
        let (mut audio, rec) = link();
        let mut state = State {
            channel: Some(4),
            pads: Pads {
                channel: Some(4),
                tap: Some((4, 50, 1.0)),
                ..Pads::default()
            },
            ..State::default()
        };
        state.step.lane = 2;
        // The step view's: the rows', and its time zoom and scroll.
        let view = |pitch| crate::ui::drum_grid::ClipView {
            rows: view(pitch),
            k: 0.2,
            x: f64::from(pitch),
        };
        state.step.views = [
            ((4, 2, Some(1)), view(2.0)),
            ((3, 0, Some(1)), view(3.0)),
            ((2, 1, None), view(4.0)),
            ((2, 2, Some(7)), view(5.0)),
        ]
        .into();
        rekey(&mut state, &mut audio, &channel, &track);
        assert_eq!((state.channel, state.step.lane), (Some(3), 2));
        assert_eq!(
            (state.pads.channel, state.pads.tap),
            (Some(3), Some((3, 50, 1.0)))
        );
        let mut views: Vec<_> = state.step.views.into_iter().collect();
        views.sort_by_key(|v| v.0);
        assert_eq!(
            views,
            [((2, 1, Some(7)), view(5.0)), ((3, 2, Some(1)), view(2.0))]
        );
        assert_eq!(rec.take(), Vec::<String>::new());
    }

    #[test]
    fn a_gone_channel_stops_its_pads_and_starts_a_fresh_view_keeping_the_lock() {
        let (mut audio, rec) = link();
        let mut state = State {
            channel: Some(3),
            pads: Pads {
                channel: Some(3),
                held: vec![(0, 48)],
                tap: Some((3, 50, 1.0)),
                ..Pads::default()
            },
            ..State::default()
        };
        state.step.lock = true;
        state.step.lane = 1;
        rekey(&mut state, &mut audio, &channel, &track);
        assert_eq!(rec.take(), ["NoteOff 3 50", "NoteOff 3 48"]);
        let pads = &state.pads;
        assert_eq!(
            (state.channel, pads.channel, pads.held.clone(), pads.tap),
            (None, None, vec![], None)
        );
        assert_eq!((state.step.lane, state.step.lock), (0, true));
    }

    #[test]
    fn the_pads_follow_the_current_channels_first_lane() {
        // Drums first → pads; audio, automation or notes first, no lanes or no such channel → none.
        let mut song = Song::default();
        song.add_channel(crate::song::ChannelKind::Drums);
        song.add_channel(crate::song::ChannelKind::Synth(
            crate::song::SynthKind::MiniSynth,
        ));
        assert_eq!(pads_channel(&song, 1), Some(1));
        assert_eq!(pads_channel(&song, 2), None);
        assert_eq!(pads_channel(&song, 0), None);
        assert_eq!(pads_channel(&song, 9), None);
        // Only the FIRST lane decides: [Drums, Notes] → pads, [Notes, Drums] → none.
        let mut extra = song.channels[1].lanes[0].clone();
        extra.kind = LaneKind::Notes;
        song.channels[1].lanes.push(extra);
        assert_eq!(pads_channel(&song, 1), Some(1));
        song.channels[1].lanes.reverse();
        assert_eq!(pads_channel(&song, 1), None);
        song.channels[1].lanes.reverse();
        song.channels[1].lanes.pop();
        for kind in [LaneKind::Notes, LaneKind::Audio, LaneKind::Automation] {
            song.channels[1].lanes[0].kind = kind;
            assert_eq!(pads_channel(&song, 1), None, "{kind:?}");
        }
        // The first lane in lane order, whatever the channel's kind.
        song.channels[2].lanes[0].kind = LaneKind::Drums;
        assert_eq!(pads_channel(&song, 2), Some(2));
        song.channels[2].lanes.clear();
        assert_eq!(pads_channel(&song, 2), None);
    }

    #[test]
    fn page_tabs_show_only_with_the_mixer_fully_out() {
        // (x + 3 U, top, column width − 3 U, 1.5 U − 1 px), at c = 1 only.
        for (ppp, px) in [(1.0, 1.0), (2.0, 0.5)] {
            let m = Metrics::from_pixels_per_point(ppp, crate::metrics::DEFAULT_UI_SCALE_INDEX);
            let editor = Rect::from_min_size(pos2(10.0, 20.0), egui::vec2(1170.0, 349.0));
            for c in [0.0, 0.4, 0.99] {
                assert_eq!(tabs(editor, c, &m), None, "c = {c}");
            }
            let want = Rect::from_min_size(pos2(10.0 + 96.0, 20.0), egui::vec2(384.0, 48.0 - px));
            assert_eq!(tabs(editor, 1.0, &m), Some(want));
        }
    }

    #[test]
    fn the_mixer_fold_is_kept_across_channels() {
        let (mut audio, _rec) = link();
        let mut state = State::default();
        // A new run starts collapsed.
        assert!(!state.step.fold.open && state.step.fold.c == 0.0);
        switch(&mut state, 1);
        state.step.fold.open = true;
        state.step.fold.c = 0.5;
        let fold = state.step.fold;
        // Another channel's editor keeps it (one step-sequencer editor), and so does a gone channel's fresh view.
        switch(&mut state, 2);
        assert_eq!(state.step.fold, fold);
        state.channel = Some(3);
        rekey(&mut state, &mut audio, &channel, &track);
        assert_eq!((state.channel, state.step.fold), (None, fold));
    }

    #[test]
    fn the_mode_is_the_current_lanes_grid() {
        // Kind 3 with flag 0 → the step sequencer; with flag 1 → the free note-grid.
        let mut song = Song::default();
        song.add_channel(crate::song::ChannelKind::Drums);
        let drums = &mut song.channels[1];
        let notes = drums.add_lane(LaneKind::Drums);
        drums.lanes[notes].grid = DrumGrid::Notes;
        let other = drums.add_lane(LaneKind::Notes);
        let mut state = State::default();
        assert_eq!(state.mode(&song), None);
        let mut mode = |lane| {
            open(&mut state, 1, lane, 0);
            state.mode(&song)
        };
        assert_eq!(mode(0), Some(DrumGrid::Steps));
        assert_eq!(mode(notes), Some(DrumGrid::Notes));
        // No drum lane there: no mode.
        assert_eq!(mode(other), None);
        assert_eq!(mode(9), None);
    }

    #[test]
    fn the_free_grid_keeps_its_own_fold_views_and_snap_across_channels_and_follows_moves() {
        // The step-sequencer and free-grid editors each keep their own state.
        let (mut audio, _rec) = link();
        let mut state = State::default();
        switch(&mut state, 4);
        state.grid.fold.open = true;
        state.grid.snap.on = false;
        let view = crate::ui::drum_grid::ClipView {
            rows: view(3.0),
            k: 0.5,
            x: 2.0,
        };
        state.grid.views.insert((4, 0, Some(1)), view);
        assert!(!state.step.fold.open, "the step view's fold is its own");
        // Another channel's editor keeps them.
        switch(&mut state, 2);
        switch(&mut state, 4);
        assert!(state.grid.fold.open && !state.grid.snap.on);
        // Channel 3 removed: the view follows channel 4 to 3.
        rekey(&mut state, &mut audio, &channel, &track);
        let views: Vec<_> = state.grid.views.into_iter().collect();
        assert_eq!(views, [((3, 0, Some(1)), view)]);
    }

    #[test]
    fn a_gone_lane_goes_back_to_lane_0() {
        let (mut audio, _rec) = link();
        let mut state = State {
            channel: Some(2),
            ..State::default()
        };
        (state.step.lane, state.step.clip) = (1, Some(3));
        rekey(&mut state, &mut audio, &channel, &track);
        assert_eq!(
            (state.channel, state.step.lane, state.step.clip),
            (Some(2), 0, None)
        );
    }
}

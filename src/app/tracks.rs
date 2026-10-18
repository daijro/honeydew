//! The playlist track header menu's actions, and the re-keying of what the app keeps by channel
//! index (and lane index) when channels or tracks move: a delete, an insert, a clone, an undo or redo of one.

use std::cmp::Ordering;
use std::collections::HashMap;

use honeydew_audio::device::AudioCommand;

use eframe::egui;

use super::HoneydewApp;
use crate::project_files::{MESSAGE_TAG, PresetSource};
use crate::smf::{self, import::ImportOpts};
use crate::song::{Channel, ChannelKind, DrumGrid, DrumKit, Lane, LaneKind, SamplerSlot, Song};
use crate::ui::clips::ClipRef;
use crate::ui::preview::ClipKey;
use crate::ui::rack::PresetTarget;
use crate::ui::track_menu::TrackOp;
use crate::ui::views::{self, Nav, Overlay, SongEdit, View};

const DELETE_CHANNEL_TAG: &str = "delete channel";
const DELETE_TRACK_TAG: &str = "delete empty tracks";

/// A question the track menu asked, until it is answered. The channel is named by its id, which no index shift
/// changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Asked {
    /// `Delete channel?` (or the FX channel's question): yes deletes it.
    DeleteChannel(i32),
    /// `Delete empty tracks?`, asked once the track's clips are gone: yes deletes the track too.
    DeleteTrack { channel: i32, lane: usize },
}

impl Asked {
    /// Its message's tag.
    fn tag(self) -> &'static str {
        match self {
            Self::DeleteChannel(_) => DELETE_CHANNEL_TAG,
            Self::DeleteTrack { .. } => DELETE_TRACK_TAG,
        }
    }
}

/// A track inserted or deleted at `lane` of channel `channel` (its index after the change).
#[derive(Clone, Copy, Debug)]
pub(super) struct LaneShift {
    channel: usize,
    lane: usize,
    inserted: bool,
}

impl LaneShift {
    pub(super) fn new(channel: usize, lane: usize, inserted: bool) -> Self {
        Self {
            channel,
            lane,
            inserted,
        }
    }

    /// Where track `lane` of `channel` is now, `None` for the deleted one.
    fn map(self, channel: usize, lane: usize) -> Option<usize> {
        if channel != self.channel {
            return Some(lane);
        }
        match (self.inserted, lane.cmp(&self.lane)) {
            (_, Ordering::Less) => Some(lane),
            (true, _) => Some(lane + 1),
            (false, Ordering::Equal) => None,
            (false, Ordering::Greater) => Some(lane - 1),
        }
    }
}

/// The tracks inserted or deleted in each channel between `old` and `new` (a step of the undo history, an import):
/// per channel matched by id whose track count changed, the tracks of the longer list that the shorter one, in order,
/// skips: matched as equal tracks, else (the step changed a kept track too) by kind, target and label. When neither
/// skips exactly as many as the count changed by, the channel gets no shift. Identical neighbours make the place
/// ambiguous, which does not matter: whatever was kept on one is as right on the other.
pub(super) fn lane_shifts(old: &Song, new: &Song) -> Vec<LaneShift> {
    let mut shifts = Vec::new();
    for (channel, ch) in new.channels.iter().enumerate() {
        let Some(before) = old.channels.iter().find(|c| c.id == ch.id) else {
            continue;
        };
        let (from, to) = (&before.lanes, &ch.lanes);
        let inserted = to.len() > from.len();
        let (long, short) = if inserted { (to, from) } else { (from, to) };
        // The tracks of `long` that `short`, in order, skips by `same`.
        let skipped = |same: &dyn Fn(&Lane, &Lane) -> bool| -> Vec<usize> {
            let mut short = short.iter().peekable();
            (long.iter().enumerate())
                .filter(|&(_, lane)| short.next_if(|s| same(s, lane)).is_none())
                .map(|(lane, _)| lane)
                .collect()
        };
        // Equal tracks first; a step that also changed a kept track (a recording take) matches it by what it is.
        let same_track =
            |a: &Lane, b: &Lane| (a.kind, a.target, &a.label) == (b.kind, b.target, &b.label);
        let n = from.len().abs_diff(to.len());
        let Some(mut skipped) = [skipped(&|a, b| a.same_edit(b)), skipped(&same_track)]
            .into_iter()
            .find(|s| s.len() == n)
        else {
            continue;
        };
        // In the order [`Rekey::track`] folds them: inserts up, deletes down.
        if !inserted {
            skipped.reverse();
        }
        shifts.extend(
            skipped
                .into_iter()
                .map(|lane| LaneShift::new(channel, lane, inserted)),
        );
    }
    shifts
}

/// Where [`Channel::add_lane`] puts a new track: [`Channel::new_lane_index`]. (`app.rs` calls it by this name.)
pub(super) fn new_track(channel: &Channel) -> usize {
    channel.new_lane_index()
}

/// The tracks a recording's stop added in `new` since `old`: per channel matched by id, the audio tracks
/// [`Channel::add_lane`] put at [`Channel::new_lane_index`] one after the other (`Recorder::stop`).
pub(super) fn tracks_added(old: &Song, new: &Song) -> Vec<LaneShift> {
    let mut shifts = Vec::new();
    for (channel, ch) in new.channels.iter().enumerate() {
        let Some(before) = old.channels.iter().find(|c| c.id == ch.id) else {
            continue;
        };
        let at = before.new_lane_index();
        let added = ch.lanes.len().saturating_sub(before.lanes.len());
        shifts.extend((at..at + added).map(|lane| LaneShift::new(channel, lane, true)));
    }
    shifts
}

/// Where each channel (by its index before a change) and track went.
struct Rekey<'a> {
    /// Old channel index to new, `None` when the channel is gone.
    channels: Vec<Option<usize>>,
    shifts: &'a [LaneShift],
}

impl<'a> Rekey<'a> {
    /// The channels matched by id between `old` and `new` (the ids before and after). Ids that repeat (a reader that
    /// left them 0) match nothing: what was kept for them goes, rather than move to a neighbour.
    fn new(old: &[i32], new: &[i32], shifts: &'a [LaneShift]) -> Self {
        let unique = |ids: &[i32]| {
            let mut sorted = ids.to_vec();
            sorted.sort_unstable();
            sorted.windows(2).all(|w| w[0] != w[1])
        };
        let channels = if unique(old) && unique(new) {
            (old.iter())
                .map(|id| new.iter().position(|n| n == id))
                .collect()
        } else {
            vec![None; old.len()]
        };
        Self { channels, shifts }
    }

    fn channel(&self, old: usize) -> Option<usize> {
        self.channels.get(old).copied().flatten()
    }

    /// Where track `lane` of channel `channel` went.
    fn track(&self, channel: usize, lane: usize) -> Option<(usize, usize)> {
        let channel = self.channel(channel)?;
        let lane = (self.shifts.iter()).try_fold(lane, |lane, s| s.map(channel, lane))?;
        Some((channel, lane))
    }
}

/// Each track's current clip (DESc@24, view state the playlist's taps set) from `old` onto `new`, a song an undo or redo
/// restored, wherever the step moved the track; a track the step brought back keeps its own.
// ponytail: a channel (or track) brought back by undoing its delete takes its current clip from the last snapshot that
// held it, not from a tap after that (taps make no snapshot); keep each deleted channel's ids beside the history if that
// matters.
pub(super) fn carry_current_clips(old: &Song, new: &mut Song, shifts: &[LaneShift]) {
    let rk = Rekey::new(&channel_ids(old), &channel_ids(new), shifts);
    for (c, ch) in old.channels.iter().enumerate() {
        for (l, lane) in ch.lanes.iter().enumerate() {
            let to = rk
                .track(c, l)
                .and_then(|(c, l)| new.channels.get_mut(c)?.lanes.get_mut(l));
            if let Some(to) = to {
                to.file.current_clip_id = lane.file.current_clip_id;
            }
        }
    }
}

/// The ids of `song`'s channels, in order.
pub(super) fn channel_ids(song: &Song) -> Vec<i32> {
    song.channels.iter().map(|c| c.id).collect()
}

/// The question `Delete Channel` asks for channel `index`: a send other channels output to
/// gets the FX channel's, by how many they are.
fn delete_question(song: &Song, index: usize) -> &'static str {
    let bus = song.channels[index].bus_id;
    let users = (song.channels.iter())
        .filter(|c| bus >= 1 && c.output_bus == bus)
        .count();
    match users {
        0 => "Delete channel? This action cannot be undone.",
        1 => "Another channel has this FX channel as the output destination. Delete FX channel?",
        _ => "Other channels have this FX channel as the output destination. Delete FX channel?",
    }
}

impl HoneydewApp {
    /// Carries out the track menu's `op` for track `lane` of channel `channel`. `tick` is the transport position
    /// before the edit. Each song change is one undo step; `Insert Channel` is [`Self::insert_channel`].
    pub(super) fn track_op(&mut self, channel: usize, lane: usize, op: TrackOp, tick: u64) {
        let Some(ch) = self.song.channels.get_mut(channel) else {
            return;
        };
        match op {
            TrackOp::ToggleZip => {
                ch.zipped = !ch.zipped;
                // A clip selected on a track that hides goes with it, as do such clips of the group.
                let clips = &mut self.main_screen.clips;
                let hidden = |r: &ClipRef| ch.zipped && r.channel == channel && r.lane > 0;
                if clips.selected.as_ref().is_some_and(hidden) {
                    *clips = Default::default();
                }
                clips.group.retain(|r| !hidden(r));
                // The audio never reads it: an undo step with no rebuild.
                self.song_edited(SongEdit::Param, tick);
            }
            TrackOp::ChannelSettings => {
                let name = ch.name.clone();
                self.navigate(Nav::Open(Overlay::ChannelSettings { channel, name }));
            }
            TrackOp::CloneChannel => {
                let copy = self.song.clone_channel(channel);
                // The master copies nothing; past the channel-id limit nothing is copied either, and
                // the message says why.
                if copy == channel {
                    if channel > 0 {
                        self.navigate(Nav::Open(views::preset_picker::channel_limit_message()));
                    }
                    return;
                }
                self.song_edited(SongEdit::Final, tick);
                // The copy's rack starts with its source's panel settings, which the song does not hold yet.
                self.slots.rack.copy_panels(channel, copy);
                self.selected_channel = copy;
            }
            TrackOp::DeleteChannel => {
                let asked = Asked::DeleteChannel(ch.id);
                let question = delete_question(&self.song, channel);
                self.ask(asked, DELETE_CHANNEL_TAG, question);
            }
            // An empty track goes at once; one with clips loses them, and then the question is whether the emptied
            // track goes too.
            TrackOp::DeleteTrack => {
                let asked = Asked::DeleteTrack {
                    channel: ch.id,
                    lane,
                };
                match ch.lanes.get_mut(lane).map(|l| std::mem::take(&mut l.clips)) {
                    Some(clips) if clips.is_empty() => self.delete_track(channel, lane, tick),
                    Some(_) => {
                        self.song_edited(SongEdit::Final, tick);
                        self.ask(asked, DELETE_TRACK_TAG, "Delete empty tracks?");
                    }
                    None => {}
                }
            }
            TrackOp::AddTrack => {
                let kind = ch.lanes.first().map_or(LaneKind::Notes, |l| l.kind);
                self.add_track(channel, kind, DrumGrid::Steps, tick);
            }
            // Kind-3 lanes of flag 0 and 1.
            TrackOp::AddStepSeqTrack => {
                self.add_track(channel, LaneKind::Drums, DrumGrid::Steps, tick)
            }
            TrackOp::AddNotesTrack => {
                self.add_track(channel, LaneKind::Drums, DrumGrid::Notes, tick)
            }
            // The drumset picker, whose pick replaces the kit (`views::preset_picker`).
            TrackOp::LoadDrumset => {
                let target = PresetTarget::Drumset;
                self.navigate(Nav::Open(Overlay::PresetPicker { channel, target }));
            }
            // The `Save Drumset` dialog, saving the kit as it is now: not a song edit.
            TrackOp::SaveDrumset => {
                let kit = ch.drum_kit.clone().unwrap_or_default();
                let name = ch.name.clone();
                self.files
                    .ask_preset(PresetSource::Drumset { name, kit }, ch.id);
            }
            // The screen shows the add-channel popup for it.
            TrackOp::InsertChannel => {}
            // The file goes to [`Self::midi_import`] once read.
            TrackOp::ImportMidi => self.files.pick_midi_tracks(ch.id),
            // The one-format Save dialog, proposing the channel's name; its `SAVE` writes the channel alone.
            TrackOp::SaveMidi => {
                let overlay = self.slots.export.save_as_midi(ch.id, &ch.name);
                self.navigate(Nav::Open(overlay));
            }
            // Disabled in the menu (see `TrackOp`).
            TrackOp::SyncAllSamples => {}
        }
    }

    /// After the files, each frame: `Import MIDI Tracks`. A file of exactly one MIDI channel goes into the channel
    /// whose menu it was at once (Split off, automation on, Only supported off); one of none does nothing; any other
    /// opens the MIDI import dialog, whose `Import` comes here too.
    pub(super) fn midi_import(&mut self, ctx: &egui::Context, tick: u64) {
        if let Some(picked) = self.files.take_midi_tracks() {
            ctx.request_repaint();
            match picked.scan.used.count_ones() {
                0 => {}
                1 => {
                    let opts = ImportOpts {
                        channels: picked.scan.used,
                        ..ImportOpts::ONE_CHANNEL
                    };
                    self.import_midi(picked.channel, &picked.bytes, &opts, tick);
                }
                _ => {
                    self.slots.midi_import.open(picked);
                    self.navigate(Nav::Open(Overlay::MidiImport));
                }
            }
        }
        if let Some((picked, opts)) = self.slots.midi_import.take_import() {
            self.import_midi(picked.channel, &picked.bytes, &opts, tick);
        }
    }

    /// Imports `bytes` with `opts` into the channel with id `channel` (not as a song: the tempo stays), one undo step.
    /// What the app keeps by track follows the tracks it inserts there, and that channel's tracks show. A channel gone
    /// meanwhile takes nothing; with Split on, which never imports into it, the new channels are made all the same.
    fn import_midi(&mut self, channel: i32, bytes: &[u8], opts: &ImportOpts, tick: u64) {
        let current = self.song.channels.iter().position(|c| c.id == channel);
        let Some(current) = current.or(opts.split.then_some(0)) else {
            return;
        };
        let before = self.song.clone();
        let factory = self.files.factory;
        let imported =
            match smf::import::import(&mut self.song, current, bytes, opts, false, factory) {
                Ok(imported) => imported,
                Err(detail) => {
                    let lines = vec![smf::import::LOAD_FAILED.into(), detail];
                    return self.navigate(Nav::Open(Overlay::Message {
                        tag: MESSAGE_TAG,
                        lines,
                        confirm: None,
                        extra: None,
                    }));
                }
            };
        // The new channels' DW programs load as a song's do when it opens.
        {
            let read = crate::sample_cache::reader(&self.files);
            for &i in &imported.channels {
                for module in &mut self.song.channels[i].rack.modules {
                    crate::panels::cstm::load_dw_program(module, &read);
                }
            }
        }
        if let Some(lines) = crate::project_files::stand_ins(imported.notices) {
            self.navigate(Nav::Open(Overlay::Message {
                tag: MESSAGE_TAG,
                lines,
                confirm: None,
                extra: None,
            }));
        }
        let shifts = lane_shifts(&before, &self.song);
        if !shifts.is_empty() {
            self.song.channels[current].zipped = false;
        }
        self.rekey(&shifts);
        self.song_edited(SongEdit::Final, tick);
    }

    /// `Insert Channel`'s pick, and the add-channel `+`'s at the end: a channel of `kind` at index `at`, selected. A
    /// `Rack` opens the rack preset picker instead, whose pick inserts the channel there. A Drums channel is `Drum`
    /// (numbered as a taken name is), with one empty pad, and opens the drumset picker; closing it loads no kit. An
    /// `Instrument` channel holds a SuperSaw and opens the Instruments picker; closing it keeps the SuperSaw. An
    /// `Audio Record` channel arms REC and opens the input.
    ///
    /// - The picker of a new `Instrument` first picks `Close Grand` where the content has it
    ///   ([`views::preset_picker::State::close_grand`]); the SuperSaw stays where it does not.
    /// - A Drums, Instrument or Synth channel starts with a one-bar clip at the song start on its primary track, as a
    ///   tap's `Add clip` makes it, in the channel's undo step; the clip is current and its editor opens, under the
    ///   picker.
    pub(super) fn insert_channel(&mut self, at: usize, kind: ChannelKind, tick: u64) {
        if kind == ChannelKind::Rack {
            return self.navigate(super::rack_channel_picker(Some(at)));
        }
        let Some(at) = self.song.insert_channel(at, kind) else {
            return self.navigate(Nav::Open(views::preset_picker::channel_limit_message()));
        };
        // Every new DW Sampler loads Close Grand.
        if let ChannelKind::Synth(synth) = kind {
            let ch = &self.song.channels[at];
            let (picker, id) = (
                &mut self.slots.preset_picker,
                ch.rack.modules[0].instance_id,
            );
            views::preset_picker::load_close_grand(picker, &self.files, ch.id, id, synth.type_id());
        }
        let drums = kind == ChannelKind::Drums;
        if drums {
            let name = self.song.free_name("Drum");
            let mut kit = DrumKit::default();
            kit.insert_slot(0, SamplerSlot::default());
            let ch = &mut self.song.channels[at];
            (ch.name, ch.drum_kit) = (name, Some(kit));
        }
        let instrument = (kind == ChannelKind::Instrument).then(|| {
            let rack = &mut self.song.channels[at].rack;
            rack.add(crate::song::SynthKind::SuperSaw.type_id())
        });
        // Drums, Instrument and Synth channels start with a clip at the song start.
        let takes_clip = drums || matches!(kind, ChannelKind::Instrument | ChannelKind::Synth(_));
        let clip = takes_clip.then(|| {
            let bar = self.song.time_sig.bar_beats();
            let id =
                crate::ui::clips::add_clip(&mut self.song.channels[at].lanes[0], 0.0, 1.0, bar);
            ClipKey {
                channel: at,
                lane: 0,
                id,
            }
        });
        self.song_edited(SongEdit::Final, tick);
        self.selected_channel = at;
        // `Audio Record` arms REC and asks for the input.
        if kind == ChannelKind::AudioRecord {
            (self.main_screen.armed, self.input_failed) = (true, false);
            self.open_input(tick);
        }
        // Its editor opens, the clip current; a picker opens over it.
        if let Some(key) = clip {
            self.navigate(Nav::EditClip(key));
        }
        let target = match instrument {
            Some(_) => {
                // `Close Grand` in place of the SuperSaw.
                self.slots.preset_picker.close_grand = true;
                Some(PresetTarget::Instruments)
            }
            None => drums.then_some(PresetTarget::Drumset),
        };
        if let Some(target) = target {
            self.navigate(Nav::Open(Overlay::PresetPicker {
                channel: at,
                target,
            }));
        }
    }

    /// Deletes channel `index` (not the master).
    pub(super) fn delete_channel(&mut self, index: usize, tick: u64) {
        if index == 0 || index >= self.song.channels.len() {
            return;
        }
        self.song.delete_channel(index);
        self.song_edited(SongEdit::Final, tick);
    }

    /// Adds a track of `kind` (a drum track of `grid`) to `channel` (`Add Track`, `Add StepSeq Track`, `Add Notes
    /// Track`), and shows the channel's tracks if they were hidden, so that the new one shows.
    fn add_track(&mut self, channel: usize, kind: LaneKind, grid: DrumGrid, tick: u64) {
        let ch = &mut self.song.channels[channel];
        let lane = ch.add_lane(kind);
        ch.lanes[lane].grid = grid;
        ch.zipped = false;
        self.rekey(&[LaneShift::new(channel, lane, true)]);
        self.song_edited(SongEdit::Final, tick);
    }

    /// Deletes track `lane` of `channel`; never a channel's last track.
    fn delete_track(&mut self, channel: usize, lane: usize, tick: u64) {
        let Some(ch) = self.song.channels.get_mut(channel) else {
            return;
        };
        if ch.lanes.len() < 2 || lane >= ch.lanes.len() {
            return;
        }
        ch.lanes.remove(lane);
        self.rekey(&[LaneShift::new(channel, lane, false)]);
        self.song_edited(SongEdit::Final, tick);
    }

    /// Puts the channels in playlist order `order` (a track header's drop, [`Song::set_playlist`]), one undo step: what
    /// the app keeps by channel index follows its channel ([`Self::rekey`], by id). Only the master's place moved: the
    /// audio has nothing to rebuild. Any other move is a completed edit whose engine takes over the playing one by
    /// channel id (`SongAudioEngine::adopting_edit`) without a re-sync, so every voice and tail sounds on.
    pub(super) fn set_playlist(&mut self, order: &[usize], tick: u64) {
        let ids = channel_ids(&self.song);
        if self.song.set_playlist(order) {
            let moved = channel_ids(&self.song) != ids;
            let edit = if moved {
                SongEdit::Final
            } else {
                SongEdit::Param
            };
            self.song_edited(edit, tick);
        }
    }

    /// Asks `question` (Yes / No) for `asked`.
    fn ask(&mut self, asked: Asked, tag: &'static str, question: &str) {
        self.asked = Some(asked);
        self.navigate(Nav::Open(Overlay::Message {
            tag,
            lines: vec![question.to_owned()],
            confirm: Some(("Yes".into(), "No".into())),
            extra: None,
        }));
    }

    /// Acts on the answer to the track menu's question, once it comes; anything but yes leaves the song alone.
    pub(super) fn answered(&mut self, tick: u64) {
        let Some(asked) = self.asked else {
            return;
        };
        let Some(yes) = self.nav.take_answer(asked.tag()) else {
            return;
        };
        self.asked = None;
        let index = |id: i32| self.song.channels.iter().position(|c| c.id == id);
        match asked {
            Asked::DeleteChannel(id) => {
                if let Some(channel) = index(id).filter(|_| yes) {
                    self.delete_channel(channel, tick);
                }
            }
            // Only while the track is still empty: an undo under the question may have brought its clips back.
            Asked::DeleteTrack { channel, lane } => {
                let empty = |c: usize| {
                    let lane = self.song.channels[c].lanes.get(lane);
                    lane.is_some_and(|l| l.clips.is_empty())
                };
                if let Some(channel) = index(channel).filter(|&c| yes && empty(c)) {
                    self.delete_track(channel, lane, tick);
                }
            }
        }
    }

    /// The current channel, by index and id: a change of either, over a frame, is another current channel. `Insert
    /// Channel` on the current track keeps the index (the new channel takes it); ids repeat in a song a reader left
    /// them 0 in.
    pub(super) fn current_channel(&self) -> (usize, Option<i32>) {
        let id = self.song.channels.get(self.selected_channel).map(|c| c.id);
        (self.selected_channel, id)
    }

    /// The open rack follows the current channel. The frame calls it once the current
    /// channel changed (a header or clip tap, `Select clip`, a clone, an undo, a delete): the rack shows it as opening
    /// it there would ([`crate::ui::rack::RackPanel`] keeps its states by channel and closes the old rack's menus),
    /// swapped in place, with no slide, edit or undo step. A rack whose channel goes follows in [`Self::rekey`].
    pub(super) fn rack_follows(&mut self, ctx: &egui::Context) {
        for overlay in &mut self.nav.overlays {
            if let Overlay::Rack(channel) = overlay {
                *channel = self.selected_channel;
                ctx.request_repaint();
            }
        }
    }

    /// Re-keys what the app keeps by channel and track index, after the song's channels changed (matched by id against
    /// the last re-key's) or `shifts` moved tracks. What belonged to a gone channel or track goes: its held notes stop
    /// (in the engine they sound in, before the rebuild that follows), its editor shows the playlist, its dialogs
    /// close, its rack shows the current channel, and Preview and the clip selection on it end. The track menu's open
    /// question goes too, with its message: an undo under it can bring back a channel whose id a newer one took (undo
    /// restores the id counter), so the channel or track it names may be another one now.
    pub(super) fn rekey(&mut self, shifts: &[LaneShift]) {
        let ids = channel_ids(&self.song);
        if ids == self.channel_ids && shifts.is_empty() {
            return;
        }
        let rk = Rekey::new(&self.channel_ids, &ids, shifts);
        // REC's clips and held keys go with their tracks and channels (`app/auto_record.rs`, `app/note_record.rs`).
        let track = |c: usize, l: usize| rk.track(c, l);
        self.auto_record.rekey(&self.channel_ids, &track);
        (self.note_record).rekey(&self.channel_ids, &|c| rk.channel(c), &track);
        self.channel_ids = ids;
        // A header drag's order names the channels as they were: it ends.
        self.main_screen.cancel_hold();
        if let Some(tag) = self.asked.take().map(Asked::tag) {
            (self.nav.overlays)
                .retain(|o| !matches!(o, Overlay::Message { tag: t, .. } if *t == tag));
        }

        for (channel, key) in std::mem::take(&mut self.notes) {
            match rk.channel(channel) {
                Some(channel) => self.notes.push((channel, key)),
                None => {
                    self.audio.send(AudioCommand::NoteOff { channel, key });
                    self.tap_off = None;
                }
            }
        }
        let channel = |c: usize| rk.channel(c);
        // So do the chord keyboard's, typed and MIDI notes: each ends on the channel it sounds on, which the engine
        // moves by id. A gone channel's stop now; its keys, still held, hold the master, which plays nothing.
        let mut gone = self.typed.rekey(&channel);
        let mut follow = |c: &mut usize, key: u8| match rk.channel(*c) {
            Some(to) => {
                *c = to;
                true
            }
            None => {
                gone.push((*c, key));
                false
            }
        };
        self.chord_on.retain_mut(|(c, key)| follow(c, *key));
        self.midi.snapped.retain_mut(|(c, _, key)| follow(c, *key));
        self.gated.retain_mut(|(_, c, key)| follow(c, *key));
        (self.fresh).retain_mut(|(c, _)| rk.channel(*c).map(|to| *c = to).is_some());
        // Play's cut keys stay dark on their channel, wherever it moved.
        (self.cut).retain_mut(|(c, _)| rk.channel(*c).map(|to| *c = to).is_some());
        self.midi.router.rekey(&channel);
        for (channel, key) in gone {
            self.audio.send(AudioCommand::NoteOff { channel, key });
        }
        let track = |c: usize, l: usize| rk.track(c, l);
        views::drum_editor::rekey(
            &mut self.slots.drum_editor,
            &mut self.audio,
            &channel,
            &track,
        );
        self.slots.rack.rekey(&channel);
        let rolls = std::mem::take(&mut self.slots.piano_roll.rolls);
        for mut roll in rolls.into_values() {
            if let Some((c, lane)) = rk.track(roll.channel, roll.lane) {
                (roll.channel, roll.lane) = (c, lane);
                self.slots.piano_roll.rolls.insert(c, roll);
            }
        }

        let last = self.song.channels.len().saturating_sub(1);
        let selected = rk.channel(self.selected_channel);
        // A deleted channel's keyboard goes to the channel now in its place.
        self.selected_channel = selected
            .unwrap_or(self.selected_channel)
            .clamp(last.min(1), last);
        self.preview = (self.preview).and_then(|k| {
            let (channel, lane) = rk.track(k.channel, k.lane)?;
            Some(ClipKey { channel, lane, ..k })
        });
        self.main_screen.clips.rekey(track);
        (self.main_screen.track_menu, self.main_screen.insert) = (None, None);

        // The automation and audio editors go with the track of the clip they hold for their channel.
        let follow = |held: &HashMap<usize, (usize, u32)>, c: usize| match held.get(&c) {
            Some(&(lane, _)) => rk.track(c, lane).map(|(c, _)| c),
            None => rk.channel(c),
        };
        let (automation, audio) = (
            &self.slots.automation_editor.clips,
            &self.slots.audio_editor.clips,
        );
        // The view last drawn moves with the view to draw (`left_view`).
        let moved = |view: View| match view {
            View::PianoRoll(c) => rk.channel(c).map_or(View::Playlist, View::PianoRoll),
            View::DrumEditor(c) => rk.channel(c).map_or(View::Playlist, View::DrumEditor),
            View::AutomationEditor(c) => {
                follow(automation, c).map_or(View::Playlist, View::AutomationEditor)
            }
            View::AudioEditor(c) => follow(audio, c).map_or(View::Playlist, View::AudioEditor),
            View::Playlist | View::Mixer => view,
        };
        self.shown = moved(self.shown);
        let (nav, selected) = (&mut self.nav, self.selected_channel);
        let stop_drag = &mut self.slots.rack.stop_drag;
        nav.view = moved(nav.view);
        let rekey = |channel: &mut usize| rk.channel(*channel).map(|c| *channel = c).is_some();
        nav.overlays.retain_mut(|o| match o {
            // The open rack follows the current channel, the one in a gone channel's place
            // too ([`HoneydewApp::rack_follows`]), and a drag of the gone one's control ends.
            Overlay::Rack(channel) => {
                if !rekey(channel) {
                    (*channel, *stop_drag) = (selected, true);
                }
                true
            }
            // A new channel's place: above the channel at `at`.
            Overlay::PresetPicker {
                target: PresetTarget::NewChannel(at),
                ..
            } => at.as_mut().is_none_or(rekey),
            Overlay::ModulesBrowser { channel, .. }
            | Overlay::ChannelSettings { channel, .. }
            | Overlay::PresetPicker { channel, .. }
            | Overlay::SamplePicker { channel, .. }
            | Overlay::SoundFontPicker { channel, .. }
            | Overlay::FineTune { channel, .. }
            | Overlay::ModuleSamplePicker { channel, .. } => rekey(channel),
            // The clip's track.
            Overlay::ClipSamplePicker(key) => rk
                .track(key.channel, key.lane)
                .map(|(channel, lane)| (key.channel, key.lane) = (channel, lane))
                .is_some(),
            Overlay::TrackSamplePicker { channel, lane, .. } => rk
                .track(*channel, *lane)
                .map(|(c, l)| (*channel, *lane) = (c, l))
                .is_some(),
            // Every overlay that keeps no channel index, named so that a new one must be placed.
            Overlay::MainMenu(_)
            | Overlay::SaveDialog { .. }
            | Overlay::KeyboardSettings
            | Overlay::CtrlMenu
            | Overlay::Message { .. }
            | Overlay::TempoEditor
            | Overlay::PresetDialog { .. }
            | Overlay::SampleName { .. }
            | Overlay::Busy { .. }
            | Overlay::LearnCc
            // Its channel is kept by id.
            | Overlay::MidiImport
            | Overlay::Credits => true,
        });
        // The audio editor's op waiting to be saved (`app/audio_edit.rs`) follows its clip, or goes with it.
        self.audio_edit = (self.audio_edit.take()).and_then(|p| p.moved(|c, l| rk.track(c, l)));
        for held in [
            &mut self.slots.automation_editor.clips,
            &mut self.slots.audio_editor.clips,
        ] {
            *held = (std::mem::take(held).into_iter())
                .filter_map(|(c, (lane, id))| rk.track(c, lane).map(|(c, lane)| (c, (lane, id))))
                .collect();
        }
        // The automation and audio editors' per-clip views and their selection's clip.
        self.slots.automation_editor.editor.rekey(&track);
        self.slots.audio_editor.editor.rekey(&track);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_match_by_id_and_repeated_ids_match_nothing() {
        let rk = Rekey::new(&[0, 1, 2, 3], &[0, 1, 3], &[]);
        assert_eq!(rk.channels, [Some(0), Some(1), None, Some(2)]);
        let rk = Rekey::new(&[0, 1, 3], &[0, 1, 2, 3], &[]);
        assert_eq!(rk.channels, [Some(0), Some(1), Some(3)]);
        // Ids that repeat (a reader that left them 0) match nothing: what was kept goes rather than moving to a
        // neighbour.
        let rk = Rekey::new(&[0, 0, 0], &[0, 0], &[]);
        assert_eq!(rk.channels, [None, None, None]);
        let rk = Rekey::new(&[0, 1, 2], &[0, 0, 2], &[]);
        assert_eq!(rk.channels, [None, None, None]);
    }

    #[test]
    fn an_undo_step_finds_the_track_it_inserts_or_deletes_by_channel_id() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        song.add_channel(ChannelKind::Instrument);
        let mut more = song.clone();
        let drums = &mut more.channels[1];
        drums.add_lane(LaneKind::Notes);
        drums.lanes[1].add_clip(0, 4.0);
        let drums = drums.id;
        let at = |shifts: Vec<LaneShift>| -> Vec<_> {
            (shifts.iter())
                .map(|s| (s.channel, s.lane, s.inserted))
                .collect()
        };
        assert_eq!(at(lane_shifts(&song, &more)), [(1, 1, true)]);
        assert_eq!(at(lane_shifts(&more, &song)), [(1, 1, false)]);
        // Matched by id, wherever the channel went.
        more.channels.swap(1, 2);
        assert_eq!(more.channels[2].id, drums);
        assert_eq!(at(lane_shifts(&song, &more)), [(2, 1, true)]);
        // Two tracks more are two shifts, upwards; two fewer two, downwards. The same count is none.
        more.channels[2].add_lane(LaneKind::Drums);
        assert_eq!(at(lane_shifts(&song, &more)), [(2, 1, true), (2, 2, true)]);
        assert_eq!(
            at(lane_shifts(&more, &song)),
            [(1, 2, false), (1, 1, false)]
        );
        assert_eq!(at(lane_shifts(&song, &song)), []);
    }

    #[test]
    fn a_step_that_changes_a_track_and_adds_two_after_it_shifts_only_the_added_ones() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Instrument);
        let t = |param| crate::song::AutomationTarget {
            instance_id: 0,
            param,
        };
        song.channels[1].add_automation_lane(t(0)).add_clip(0, 4.0);
        let mut take = song.clone();
        let ch = &mut take.channels[1];
        ch.lanes[1].add_clip(64 * crate::song::PPQ, 4.0);
        ch.add_automation_lane(t(1));
        ch.add_automation_lane(t(2));
        let at = |shifts: Vec<LaneShift>| -> Vec<_> {
            (shifts.iter())
                .map(|s| (s.channel, s.lane, s.inserted))
                .collect()
        };
        assert_eq!(at(lane_shifts(&song, &take)), [(1, 2, true), (1, 3, true)]);
        assert_eq!(
            at(lane_shifts(&take, &song)),
            [(1, 3, false), (1, 2, false)]
        );
        // Changed beyond telling which tracks went: no shift.
        let mut other = take.clone();
        other.channels[1].lanes[1].label = "Renamed".into();
        other.channels[1].lanes[1].clips.clear();
        assert_eq!(at(lane_shifts(&song, &other)), []);
    }

    #[test]
    fn a_track_shift_moves_only_its_channels_tracks_after_it() {
        let deleted = LaneShift::new(1, 1, false);
        let shifts = [deleted];
        let rk = Rekey::new(&[0, 1, 2], &[0, 1, 2], &shifts);
        let tracks: Vec<_> = (0..3).map(|l| rk.track(1, l)).collect();
        assert_eq!(tracks, [Some((1, 0)), None, Some((1, 1))]);
        assert_eq!(rk.track(2, 2), Some((2, 2)));
        let inserted = LaneShift::new(1, 1, true);
        let shifts = [inserted];
        let rk = Rekey::new(&[0, 1, 2], &[0, 1, 2], &shifts);
        let tracks: Vec<_> = (0..2).map(|l| rk.track(1, l)).collect();
        assert_eq!(tracks, [Some((1, 0)), Some((1, 2))]);
    }
}

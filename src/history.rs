//! App-level undo/redo: snapshots of the [`Song`], one per completed edit.

use std::collections::VecDeque;
use std::sync::Arc;

use eframe::egui::Context;

use crate::keymap;
use crate::song::{Channel, SamplerSlot, SlotFileState, Song};

/// An undo or redo request, from the transport's Undo control or [`shortcut`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Undo,
    Redo,
}

/// A [`Song`] whose unchanged channels are shared with the snapshot before it. Full copies cost too much
/// memory: a song of 32 channels × 50 clips × 500 events is 19 MB, 1.9 GB for 100 entries; shared, 100
/// one-channel edits of it take 61 MB.
// ponytail: an edit copies its whole channel (600 KB in that song), an edit of every channel the whole song;
// share per clip if that matters.
struct Snapshot {
    /// The song without its channels.
    head: Song,
    channels: Vec<Arc<Channel>>,
}

impl Snapshot {
    /// `song`, reusing every channel of `prev` it still has unchanged, wherever it moved to.
    fn new(song: &Song, prev: &[Arc<Channel>]) -> Self {
        // Exhaustive on purpose: a new song field must be added here.
        let Song {
            title,
            tempo_bpm,
            time_sig,
            channels,
            key_root,
            key_scale,
            play_loop,
            file,
            master_below,
        } = song;
        let head = Song {
            title: title.clone(),
            tempo_bpm: *tempo_bpm,
            time_sig: *time_sig,
            channels: Vec::new(),
            key_root: *key_root,
            key_scale: *key_scale,
            play_loop: *play_loop,
            file: file.clone(),
            master_below: *master_below,
        };
        // The same index first, so that identical channels keep their own.
        let channels = channels
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let same = prev.get(i).filter(|p| p.same_edit(c));
                let same = same.or_else(|| prev.iter().find(|p| p.same_edit(c)));
                same.map_or_else(|| Arc::new(c.clone()), Arc::clone)
            })
            .collect();
        Self { head, channels }
    }

    fn song(&self) -> Song {
        Song {
            channels: self.channels.iter().map(|c| Channel::clone(c)).collect(),
            ..self.head.clone()
        }
    }

    /// The same song as `other`, which it was built from. What [`Song::keep_unedited`] keeps is not an edit: the
    /// title and the save stamp (a save sets them), the play loop (the rulers set it) and the view state; the app
    /// keeps all of them across a step.
    fn same(&self, other: &Self) -> bool {
        let mut head = other.head.clone();
        head.keep_unedited(&self.head);
        self.head == head
            && self
                .channels
                .iter()
                .map(Arc::as_ptr)
                .eq(other.channels.iter().map(Arc::as_ptr))
    }
}

/// The song states to step back and forth through. The app reports each edit with [`Self::edited`], and on
/// [`Self::undo`] / [`Self::redo`] takes the returned song and sends it to the audio as `ReplaceSong`.
pub struct History {
    /// The song after the last completed edit, or the last restored one.
    current: Snapshot,
    /// Oldest first, at most [`Self::CAP`].
    undo: VecDeque<Snapshot>,
    /// The last undone on top.
    redo: Vec<Snapshot>,
    /// A gesture's edits are in, but not its final one.
    open: bool,
}

#[cfg(test)]
thread_local! {
    /// [`History::rack_state`]'s calls on this thread.
    static RACK_STATE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times [`History::rack_state`] ran on this thread.
#[cfg(test)]
pub(crate) fn rack_state_calls() -> usize {
    RACK_STATE.get()
}

/// Moves `from` into `to` when `from` changed from `was` and `to` does not have it already. Returns whether it did.
fn mv<T: PartialEq + Clone>(to: &mut T, from: &T, was: &T) -> bool {
    let moved = from != was && to != from;
    if moved {
        to.clone_from(from);
    }
    moved
}

/// Moves into `to` each setting the drum strips set (the params, the cut group, `REV`, `Poly`, `Shift`, `Stretch`,
/// `AUX`) that `from` changed from `was`, and nothing else; `Shift` and `Stretch` move their two fields together.
/// Returns whether `to` changed.
fn take_moved(to: &mut SamplerSlot, from: &SamplerSlot, was: &SamplerSlot) -> bool {
    let shift = |s: &SlotFileState| (s.beat_offset, s.beat_offset_control);
    let stretch = |s: &SlotFileState| (s.stretch_mode, s.stretch_ratio);
    let (f, w) = (&from.file, &was.file);
    let mut changed = false;
    for ((t, f), w) in to.params.iter_mut().zip(&from.params).zip(&was.params) {
        changed |= mv(t, f, w);
    }
    changed |= mv(&mut to.cut_group, &from.cut_group, &was.cut_group);
    let t = &mut to.file;
    changed |= mv(&mut t.reverse, &f.reverse, &w.reverse);
    changed |= mv(
        &mut t.overlapping_voices,
        &f.overlapping_voices,
        &w.overlapping_voices,
    );
    changed |= mv(&mut t.send_target, &f.send_target, &w.send_target);
    let (mut s, mut r) = (shift(t), stretch(t));
    changed |= mv(&mut s, &shift(f), &shift(w));
    changed |= mv(&mut r, &stretch(f), &stretch(w));
    (
        (t.beat_offset, t.beat_offset_control),
        (t.stretch_mode, t.stretch_ratio),
    ) = (s, r);
    changed
}

/// Gives `to` what of `from` no undo step covers, copying `to` only when some of it differs; whether it did. That is:
/// - its rack, whole: the modules, their params and card state, and the last instance id, so that no id comes back
///   (the rack and presets are not covered by undo);
/// - its own params and its sends' values, by target (a control bound to a channel param saves no undo step);
/// - its pads' params and samples, by slot id (pad params are rack state), and its kit's last slot id, so that no pad
///   id comes back either.
// ponytail: `to` is copied whole (its clips too) when it differs, once per sync or restore and channel; keep racks beside
// the channels in `Snapshot` if that shows.
fn keep_live(to: &mut Arc<Channel>, from: &Channel) -> bool {
    fn same_sample(a: &SamplerSlot, b: &SamplerSlot) -> bool {
        let (f, g) = (&a.file, &b.file);
        (a.name == b.name && a.folder == b.folder && f.path == g.path)
            && (f.preset_path == g.preset_path && f.provenance == g.provenance)
    }
    let send = |id| from.file.sends.iter().find(|s| s.target_channel_id == id);
    let pad = |id| from.drum_kit.as_ref()?.slot_by_id(id);
    let sends: Vec<_> = (to.file.sends.iter().enumerate())
        .filter_map(|(i, t)| {
            let f = send(t.target_channel_id)?;
            ((f.enabled, f.level) != (t.enabled, t.level)).then_some((i, f))
        })
        .collect();
    let pads: Vec<_> = (to.drum_kit.iter().flat_map(|k| k.slots.iter().enumerate()))
        .filter_map(|(i, t)| {
            let f = pad(t.file.id)?;
            (f.params != t.params || !same_sample(f, t)).then_some((i, f))
        })
        .collect();
    let last = |c: &Channel| c.drum_kit.as_ref().map(|k| k.file.last_slot_id);
    let last = (last(to).zip(last(from))).and_then(|(t, f)| (f > t).then_some(f));
    let same = to.rack == from.rack && to.params == from.params;
    if same && sends.is_empty() && pads.is_empty() && last.is_none() {
        return false;
    }
    let to = Arc::make_mut(to);
    if let (Some(kit), Some(last)) = (&mut to.drum_kit, last) {
        kit.file.last_slot_id = last;
    }
    to.rack.clone_from(&from.rack);
    to.params = from.params;
    for (i, f) in sends {
        let t = &mut to.file.sends[i];
        (t.enabled, t.level) = (f.enabled, f.level);
    }
    for (i, f) in pads {
        let t = &mut to.drum_kit.as_mut().expect("a kit").slots[i];
        t.params = f.params;
        (t.name, t.folder) = (f.name.clone(), f.folder.clone());
        let file = &mut t.file;
        file.path.clone_from(&f.file.path);
        file.preset_path.clone_from(&f.file.preset_path);
        file.provenance.clone_from(&f.file.provenance);
    }
    true
}

/// What a change with no undo step of its own moved from `was` to `from` that `to` takes, as a new channel, or `None`
/// when it takes nothing; [`keep_live`] covers the rest. That is the drum strips' rack state ([`take_moved`], by slot
/// id), and the channel's name and preset name, which an instrument preset's pick or save sets.
fn take_untracked(to: &Channel, from: &Channel, was: &Channel) -> Option<Channel> {
    let (mut name, mut preset) = (to.name.clone(), to.file.preset_name.clone());
    let mut changed = mv(&mut name, &from.name, &was.name);
    changed |= mv(&mut preset, &from.file.preset_name, &was.file.preset_name);
    // A kit is small next to its channel's clips.
    let mut kit = to.drum_kit.clone();
    if let (Some(t), Some(f), Some(w)) = (&mut kit, &from.drum_kit, &was.drum_kit) {
        for slot in &mut t.slots {
            let id = slot.file.id;
            if let (Some(f), Some(w)) = (f.slot_by_id(id), w.slot_by_id(id)) {
                changed |= take_moved(slot, f, w);
            }
        }
    }
    changed.then(|| {
        let mut new = Channel {
            name,
            drum_kit: kit,
            ..to.clone()
        };
        new.file.preset_name = preset;
        new
    })
}

impl History {
    /// Undo entries kept.
    pub const CAP: usize = 100;

    /// A history starting at `song`, with nothing to undo (a new or loaded song).
    pub fn new(song: &Song) -> Self {
        Self {
            current: Snapshot::new(song, &[]),
            undo: VecDeque::new(),
            redo: Vec::new(),
            open: false,
        }
    }

    /// The app changed its song to `song`. Edits before a `final_edit` (a drag in progress) and that final edit
    /// make one entry. A completed edit clears the redo steps, unless it left the song as it was. What no undo step
    /// covers ([`keep_live`]) goes into the step before it first ([`Self::sync`]), so that it is no change of the step,
    /// even one in the same frame. Returns whether it saved an undo step.
    pub fn edited(&mut self, song: &Song, final_edit: bool) -> bool {
        self.open = !final_edit;
        if !final_edit {
            return false;
        }
        self.sync(song);
        let next = Snapshot::new(song, &self.current.channels);
        if next.same(&self.current) {
            return false;
        }
        self.undo
            .push_back(std::mem::replace(&mut self.current, next));
        if self.undo.len() > Self::CAP {
            self.undo.pop_front();
        }
        self.redo.clear();
        true
    }

    /// The app changed its song from `before` to `song` with no undo step (an audio recording's clips):
    /// the steps there are lead back from `song`. The redo steps go, as they would drop the change. A gesture still
    /// open is closed first, at `before`, as the step it is.
    // ponytail: snapshots, so an undo past the change drops it too (its file stays) and a redo brings it back; a
    // per-step diff would keep it.
    pub fn amend(&mut self, before: &Song, song: &Song) {
        if self.open {
            self.edited(before, true);
        }
        self.current = Snapshot::new(song, &self.current.channels);
        self.redo.clear();
    }

    /// Gives the current step what of `song` no undo step covers ([`keep_live`], by channel id): what an undo or redo
    /// keeps. Every other step lacks it, and needs none: a step an undo or redo restores takes it from the current one
    /// ([`Self::restore`]). An edit calls it first; the app calls it before an undo or redo and once a control's
    /// gesture ends, before the channel may go. A gesture still open stays open. Whether anything changed.
    pub fn sync(&mut self, song: &Song) -> bool {
        let mut changed = false;
        for ch in &mut self.current.channels {
            if let Some(from) = song.channels.iter().find(|c| c.id == ch.id) {
                changed |= keep_live(ch, from);
            }
        }
        changed
    }

    /// The app changed rack state in `song` with no undo step: the drum strips' controls, or a pick (an instrument,
    /// module or rack preset, a pad or clip sample). Each step, undo or redo, that held a channel as the change found
    /// it takes it as the change left it; one before an earlier edit of that channel takes what [`take_untracked`]
    /// moves, by channel id; so undoing or redoing another edit keeps the change ([`keep_live`] keeps the rest). A
    /// gesture still open becomes a step first, and then every step takes only what [`take_untracked`] moves; returns
    /// whether it did (the song-changed flag).
    // ponytail: a pick's lanes (the Slicer's AUTO clip) and a new pad's slot reach only the steps that held the channel
    // as the pick found it; carry them by clip and slot id if that shows.
    pub fn rack_state(&mut self, song: &Song) -> bool {
        #[cfg(test)]
        RACK_STATE.set(RACK_STATE.get() + 1);
        let was = self.current.channels.clone();
        let stepped = self.open && self.edited(song, true);
        let next = Snapshot::new(song, &self.current.channels);
        self.carry(song, &was, (!stepped).then_some(&next));
        self.current = next;
        stepped
    }

    /// Moves into each undo, redo and current step's channel what [`take_untracked`] moves from `song`'s same-id
    /// channel, given `was`, the history's channels before the change; nothing else changes. Steps that shared a
    /// channel share its new copy. With `next`, a step that held a channel of `was` takes `next`'s same-id one whole:
    /// it was the song the change found.
    // ponytail: each version of a moved channel the steps hold (up to 2 × `CAP` + 1) has its kit copied, and the whole
    // channel, clips too, when it takes something, per strip release or pick; keep kits beside the channels in
    // `Snapshot` if that shows.
    fn carry(&mut self, song: &Song, was: &[Arc<Channel>], next: Option<&Snapshot>) {
        let old = |id: i32| was.iter().find(|w| w.id == id);
        // The channels it moved: (the song's, the history's).
        let moved: Vec<_> = (song.channels.iter())
            .filter_map(|c| Some((c, old(c.id).filter(|w| !w.same_edit(c))?)))
            .collect();
        // (old, new).
        let mut done: Vec<(Arc<Channel>, Arc<Channel>)> = (next.iter())
            .flat_map(|next| &next.channels)
            .filter_map(|n| Some((Arc::clone(old(n.id)?), Arc::clone(n))))
            .collect();
        let steps = (self.undo.iter_mut()).chain(self.redo.iter_mut());
        let steps = steps.chain(std::iter::once(&mut self.current));
        for ch in steps.flat_map(|s| s.channels.iter_mut()) {
            if let Some((_, new)) = done.iter().find(|(old, _)| Arc::ptr_eq(old, ch)) {
                *ch = Arc::clone(new);
                continue;
            }
            let Some(&(from, was)) = moved.iter().find(|m| m.0.id == ch.id) else {
                continue;
            };
            let old = Arc::clone(ch);
            if let Some(new) = take_untracked(ch, from, was) {
                *ch = Arc::new(new);
            }
            done.push((old, Arc::clone(ch)));
        }
    }

    /// The current step's song, which an undo or redo restores, keeping what no undo step covers from `live`, the
    /// channels before it ([`keep_live`], by id): a channel only the step has (a delete undone) keeps its own. A pad's AUX
    /// target that names no channel of it (carried from a later step by [`take_moved`]) goes back to the channel output,
    /// as when its target is deleted (`Song::delete_channel`): else a new channel that takes the id back gets the pad.
    fn restore(&mut self, live: &[Arc<Channel>]) -> Song {
        // (old, new): the undo and redo steps that held a channel this gives the live state share its new copy, as
        // `carry`'s do; that is the live channel itself when it differed in nothing else.
        let mut done: Vec<(Arc<Channel>, Arc<Channel>)> = Vec::new();
        for ch in &mut self.current.channels {
            let Some(from) = live.iter().find(|l| l.id == ch.id) else {
                continue;
            };
            let old = Arc::clone(ch);
            if keep_live(ch, from) {
                if **ch == **from {
                    *ch = Arc::clone(from);
                }
                done.push((old, Arc::clone(ch)));
            }
        }
        let steps = (self.undo.iter_mut()).chain(self.redo.iter_mut());
        for ch in steps.flat_map(|s| s.channels.iter_mut()) {
            if let Some((_, new)) = done.iter().find(|(old, _)| Arc::ptr_eq(old, ch)) {
                *ch = Arc::clone(new);
            }
        }
        // Only this step: another one that held the channel may have the target.
        let ids: Vec<i32> = self.current.channels.iter().map(|c| c.id).collect();
        let dangling =
            |s: &SamplerSlot| s.file.send_target >= 0 && !ids.contains(&s.file.send_target);
        for ch in &mut self.current.channels {
            if ch.drum_kit.iter().flat_map(|k| &k.slots).any(dangling) {
                let slots = (Arc::make_mut(ch).drum_kit.iter_mut()).flat_map(|k| &mut k.slots);
                for slot in slots.filter(|s| dangling(s)) {
                    slot.file.send_target = -1;
                }
            }
        }
        self.current.song()
    }

    /// The song to restore, `None` when there is nothing to undo. During a gesture it only reverts the gesture;
    /// the app must also cancel the drag, or its next frame applies the edit again.
    pub fn undo(&mut self) -> Option<Song> {
        let live = self.current.channels.clone();
        if !std::mem::take(&mut self.open) {
            let prev = self.undo.pop_back()?;
            self.redo.push(std::mem::replace(&mut self.current, prev));
        }
        Some(self.restore(&live))
    }

    /// The song to restore, `None` when there is nothing to redo. It drops a gesture in progress.
    pub fn redo(&mut self) -> Option<Song> {
        let next = self.redo.pop()?;
        self.open = false;
        let live = self.current.channels.clone();
        self.undo
            .push_back(std::mem::replace(&mut self.current, next));
        Some(self.restore(&live))
    }

    /// The undo steps kept.
    #[cfg(test)]
    pub(crate) fn steps(&self) -> usize {
        self.undo.len()
    }

    pub fn can_undo(&self) -> bool {
        self.open || !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

/// The keymap's `undo` (Ctrl+Z) and `redo` (Ctrl+Shift+Z, and off macOS Ctrl+Alt+Z and Ctrl+Y); Cmd replaces Ctrl on
/// macOS (`assets/content/keymaps/`). Consumes the key. `None` while a text field has the keyboard: it keeps its own undo.
/// `None` too while a modal message, dialog or popup is open: each lays a backdrop that takes every click outside it,
/// and marks the frame ([`crate::ui::backdrop`]); so do the main menu, whose outside area closes it, and the tempo
/// dialog, whose backdrop is its own. Call it after the frame's screens, so this frame's ones count. Panels that block
/// nothing (the rack, keyboard settings) leave it live.
pub fn shortcut(ctx: &Context) -> Option<Step> {
    if ctx.wants_keyboard_input() || crate::ui::modal_open(ctx) {
        return None;
    }
    let keys = keymap::for_os(ctx.os());
    // Redo first: a binding ignores extra Shift and Alt.
    ctx.input_mut(|i| {
        if keys.consume_press(i, "redo") {
            Some(Step::Redo)
        } else {
            keys.consume_press(i, "undo").then_some(Step::Undo)
        }
    })
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Event, Key, Modifiers, RawInput};

    use super::*;
    use crate::song::{ChannelKind, Event as NoteEvent, ModuleInstance};

    fn song_with(channels: usize) -> Song {
        let mut song = Song::default();
        for _ in 0..channels {
            song.add_channel(ChannelKind::Instrument);
        }
        song
    }

    fn with_tempo(song: &Song, bpm: f64) -> Song {
        Song {
            tempo_bpm: bpm,
            ..song.clone()
        }
    }

    #[test]
    fn an_untracked_pick_stays_through_an_undo_of_another_channels_edit() {
        // The rack and presets are not covered by undo.
        let start = song_with(2);
        let n = start.channels.len();
        let mut history = History::new(&start);
        let mut edited = start.clone();
        edited.channels[n - 2].name = "Edited".into();
        history.edited(&edited, true);
        let mut picked = edited.clone();
        picked.channels[n - 1].name = "Picked".into();
        history.rack_state(&picked);
        let undone = history.undo().unwrap();
        assert_eq!(undone.channels[n - 2].name, start.channels[n - 2].name);
        assert_eq!(undone.channels[n - 1].name, "Picked");
        // Redo brings the edit back with the pick.
        let redone = history.redo().unwrap();
        assert_eq!(redone.channels[n - 2].name, "Edited");
        assert_eq!(redone.channels[n - 1].name, "Picked");
    }

    /// A song with one Drums channel (the last) on the default kit, and that channel's index.
    fn drums_song() -> (Song, usize) {
        let mut song = song_with(1);
        song.add_channel(ChannelKind::Drums);
        let n = song.channels.len() - 1;
        (song, n)
    }

    fn kit(song: &mut Song, channel: usize) -> &mut crate::song::DrumKit {
        song.channels[channel].drum_kit.as_mut().unwrap()
    }

    #[test]
    fn rack_state_reaches_the_steps_before_an_edit_of_its_own_channel() {
        // Regression: tap a step, drag the strip's Level to 1.0, undo the tap: the Level came back as before the drag,
        // as the step before the tap held the channel as it was (pad params are rack state, with no undo).
        let (start, n) = drums_song();
        let mut history = History::new(&start);
        let mut tapped = start.clone();
        tapped.channels[n].name = "Tapped".into();
        history.edited(&tapped, true);
        let mut dragged = tapped.clone();
        let slot = &mut kit(&mut dragged, n).slots[1];
        (slot.params[0], slot.cut_group, slot.file.reverse) = (1.0, 2, true);
        (slot.file.beat_offset, slot.file.beat_offset_control) = (0.25, 1.0);
        // An AUX target that every step has (one that a step lacks goes back to the channel output on restore).
        (slot.file.send_target, slot.file.stretch_ratio) = (1, 2.0);
        (slot.file.overlapping_voices, slot.file.stretch_mode) = (false, 1);
        history.rack_state(&dragged);
        let mut undone = history.undo().unwrap();
        assert_eq!(undone.channels[n].name, start.channels[n].name);
        assert_eq!(kit(&mut undone, n).slots[1], kit(&mut dragged, n).slots[1]);
        assert_eq!(
            kit(&mut undone, n).slots[0],
            kit(&mut start.clone(), n).slots[0]
        );
        let redone = history.redo().unwrap();
        assert_eq!(redone, dragged);
        // The redo steps too, and a slot matched by its id wherever it is in the kit.
        history.undo();
        let mut moved = history.current.song();
        kit(&mut moved, n).slots.swap(0, 1);
        kit(&mut moved, n).slots[0].params[0] = 0.5;
        history.rack_state(&moved);
        let mut redone = history.redo().unwrap();
        assert_eq!(kit(&mut redone, n).slots[1].params[0], 0.5);
        assert_eq!(redone.channels[n].name, "Tapped");
    }

    #[test]
    fn rack_state_moves_only_what_it_changed_so_an_undo_of_a_send_edit_keeps_its_target() {
        // Regression: the strip's release wrote every setting into every step. A: `Create FX Channel` (an undo
        // step that targets the new send) then any strip change: its undo kept the pad on the deleted channel's id,
        // which the next new channel reuses. B: a send deleted (its pads back to −1) then a strip change: its undo
        // left the pad at −1.
        let (start, n) = drums_song();
        let mut history = History::new(&start);
        let mut created = start.clone();
        created.add_channel(ChannelKind::EffectTrack);
        let send = created.channels.last().unwrap().id;
        kit(&mut created, n).slots[0].file.send_target = send;
        history.edited(&created, true);
        let mut level = created.clone();
        kit(&mut level, n).slots[0].params[0] = 1.0;
        history.rack_state(&level);
        let mut undone = history.undo().unwrap();
        let slot = &kit(&mut undone, n).slots[0];
        assert_eq!((slot.file.send_target, slot.params[0]), (-1, 1.0), "A");
        // B, from the song with the send.
        let mut history = History::new(&level);
        let mut deleted = level.clone();
        deleted.delete_channel(deleted.channels.len() - 1);
        assert_eq!(kit(&mut deleted, n).slots[0].file.send_target, -1);
        history.edited(&deleted, true);
        let mut level = deleted.clone();
        kit(&mut level, n).slots[0].params[0] = 0.5;
        history.rack_state(&level);
        let mut undone = history.undo().unwrap();
        let slot = &kit(&mut undone, n).slots[0];
        assert_eq!((slot.file.send_target, slot.params[0]), (send, 0.5), "B");
    }

    #[test]
    fn a_rack_state_call_that_changed_nothing_writes_nothing() {
        // The restretch landing (and the pad picks' landings): no setting moved, so no step changes, not even one an
        // undoable edit of the kit left different.
        let (start, n) = drums_song();
        let mut history = History::new(&start);
        let mut cut = start.clone();
        kit(&mut cut, n).slots[0].cut_group = 3;
        history.edited(&cut, true);
        let before: Vec<_> = history.undo[0].channels.iter().map(Arc::as_ptr).collect();
        history.rack_state(&cut);
        let after: Vec<_> = history.undo[0].channels.iter().map(Arc::as_ptr).collect();
        assert_eq!(before, after);
        assert_eq!(
            history
                .undo()
                .map(|mut s| kit(&mut s, n).slots[0].cut_group),
            Some(0)
        );
    }

    #[test]
    fn rack_state_says_whether_it_closed_an_open_gesture_into_a_step() {
        // For the song-changed flag.
        let (start, n) = drums_song();
        let mut history = History::new(&start);
        let mut changed = start.clone();
        kit(&mut changed, n).slots[0].params[0] = 1.0;
        assert!(!history.rack_state(&changed));
        history.edited(&with_tempo(&changed, 100.0), false);
        assert!(history.rack_state(&with_tempo(&changed, 100.0)));
    }

    #[test]
    fn rack_state_keeps_a_channel_shared_across_the_steps_that_shared_it() {
        // Memory: steps that held one copy of the channel still hold one after the change (`Snapshot`).
        let (start, n) = drums_song();
        let mut history = History::new(&start);
        for bpm in [100.0, 110.0] {
            history.edited(&with_tempo(&start, bpm), true);
        }
        let mut changed = with_tempo(&start, 110.0);
        kit(&mut changed, n).slots[0].params[0] = 1.0;
        history.rack_state(&changed);
        let [a, b] = [0, 1].map(|i| &history.undo[i].channels[n]);
        assert!(Arc::ptr_eq(a, b));
        assert_eq!(a.drum_kit.as_ref().unwrap().slots[0].params[0], 1.0);
    }

    #[test]
    fn a_gesture_open_at_a_rack_state_change_becomes_a_step_that_keeps_it() {
        let (start, n) = drums_song();
        let mut history = History::new(&start);
        history.edited(&with_tempo(&start, 100.0), false);
        let mut changed = with_tempo(&start, 100.0);
        kit(&mut changed, n).slots[0].params[0] = 1.0;
        history.rack_state(&changed);
        assert!(!history.open);
        let mut undone = history.undo().unwrap();
        assert_eq!(undone.tempo_bpm, start.tempo_bpm);
        assert_eq!(kit(&mut undone, n).slots[0].params[0], 1.0);
    }

    #[test]
    fn a_pick_keeps_the_redo_steps_and_a_redo_keeps_the_pick() {
        // Presets are not covered by undo, so a pick drops no redo step either.
        let start = song_with(2);
        let n = start.channels.len();
        let mut history = History::new(&start);
        let mut edited = start.clone();
        edited.channels[n - 2].name = "Edited".into();
        history.edited(&edited, true);
        history.undo();
        let mut picked = start.clone();
        picked.channels[n - 1].name = "Picked".into();
        history.rack_state(&picked);
        assert!(history.can_redo());
        let redone = history.redo().unwrap();
        assert_eq!(redone.channels[n - 2].name, "Edited");
        assert_eq!(redone.channels[n - 1].name, "Picked");
        // And back.
        let undone = history.undo().unwrap();
        assert_eq!(undone.channels[n - 2].name, start.channels[n - 2].name);
        assert_eq!(undone.channels[n - 1].name, "Picked");
    }

    #[test]
    fn an_untracked_change_makes_no_step_and_the_steps_before_lead_back_from_it() {
        // A preset pick: no undo step of its own.
        let start = song_with(1);
        let mut history = History::new(&start);
        let edited = with_tempo(&start, 100.0);
        history.edited(&edited, true);
        let picked = with_tempo(&edited, 110.0);
        history.rack_state(&picked);
        assert_eq!(history.steps(), 1);
        assert!(!history.can_redo());
        // An edit after it steps back to the pick, not past it.
        let after = with_tempo(&picked, 120.0);
        history.edited(&after, true);
        assert_eq!(history.undo(), Some(picked));
        assert_eq!(history.undo(), Some(start));
    }

    #[test]
    fn a_drag_is_one_entry() {
        let start = Song::default();
        let mut history = History::new(&start);
        assert!(!history.can_undo());
        for bpm in [141.0, 145.0, 150.0] {
            history.edited(&with_tempo(&start, bpm), false);
        }
        let end = with_tempo(&start, 152.0);
        history.edited(&end, true);
        assert_eq!(history.undo(), Some(start));
        assert!(!history.can_undo());
        assert_eq!(history.undo(), None);
        assert_eq!(history.redo(), Some(end));
        assert_eq!(history.redo(), None);
    }

    #[test]
    fn undo_mid_drag_reverts_only_the_drag() {
        let start = Song::default();
        let mut history = History::new(&start);
        let first = with_tempo(&start, 100.0);
        history.edited(&first, true);
        history.edited(&with_tempo(&start, 120.0), false);
        assert!(history.can_undo());
        assert_eq!(history.undo(), Some(first.clone()));
        assert!(!history.can_redo());
        assert_eq!(history.undo(), Some(start));
        assert_eq!(history.redo(), Some(first));
    }

    #[test]
    fn a_final_edit_without_change_records_nothing() {
        let start = song_with(2);
        let mut history = History::new(&start);
        let first = with_tempo(&start, 90.0);
        history.edited(&first, true);
        history.undo();
        history.edited(&start, false);
        history.edited(&start, true);
        assert_eq!(history.redo(), Some(first));
    }

    #[test]
    fn a_play_loop_or_a_save_stamp_alone_is_no_edit() {
        let start = song_with(1);
        let mut history = History::new(&start);
        let mut looped = start.clone();
        looped.play_loop = Some((0, 512));
        history.edited(&looped, true);
        looped.stamp_save("2026-10-11 09:08:07");
        history.edited(&looped, true);
        assert!(!history.can_undo());
    }

    #[test]
    fn undo_and_redo_restore_the_file_state() {
        let start = Song::default();
        let mut history = History::new(&start);
        let mut edited = start.clone();
        edited.file.meta.author = "Alex".into();
        edited.file.last_send_bus = 4;
        history.edited(&edited, true);
        assert_eq!(history.undo().unwrap().file, start.file);
        assert_eq!(history.redo().unwrap().file, edited.file);
    }

    #[test]
    fn a_new_title_alone_is_no_edit() {
        // A save names the song after its file, outside the edits.
        let start = Song::default();
        let mut history = History::new(&start);
        history.edited(&with_tempo(&start, 100.0), true);
        history.undo();
        let renamed = Song {
            title: "Beat".into(),
            ..start
        };
        history.edited(&renamed, true);
        assert!(!history.can_undo());
        assert_eq!(history.redo().map(|s| s.tempo_bpm), Some(100.0));
    }

    #[test]
    fn identical_channels_are_no_edit() {
        let mut song = song_with(1);
        song.channels.push(song.channels[1].clone());
        let mut history = History::new(&song);
        history.edited(&song, true);
        assert!(!history.can_undo());
    }

    #[test]
    fn an_open_drag_can_be_undone() {
        let start = Song::default();
        let mut history = History::new(&start);
        history.edited(&with_tempo(&start, 120.0), false);
        assert!(history.can_undo());
        assert_eq!(history.undo(), Some(start));
        assert!(!history.can_undo());
    }

    #[test]
    fn redo_mid_drag_drops_the_drag() {
        let start = Song::default();
        let mut history = History::new(&start);
        let first = with_tempo(&start, 100.0);
        history.edited(&first, true);
        history.undo();
        history.edited(&with_tempo(&start, 120.0), false);
        assert_eq!(history.redo(), Some(first));
        // The drag is gone: undo steps back over the redone entry, not the drag.
        assert_eq!(history.undo(), Some(start));
    }

    #[test]
    fn history_is_capped() {
        let start = Song::default();
        let mut history = History::new(&start);
        for i in 1..=History::CAP + 50 {
            history.edited(&with_tempo(&start, i as f64), true);
        }
        let mut last = None;
        for _ in 0..History::CAP {
            last = history.undo();
            assert!(last.is_some());
        }
        assert_eq!(last.map(|s| s.tempo_bpm), Some(50.0));
        assert_eq!(history.undo(), None);
    }

    #[test]
    fn a_new_edit_clears_redo() {
        let start = Song::default();
        let mut history = History::new(&start);
        history.edited(&with_tempo(&start, 1.0), true);
        history.edited(&with_tempo(&start, 2.0), true);
        assert_eq!(history.undo().map(|s| s.tempo_bpm), Some(1.0));
        assert!(history.can_redo());
        history.edited(&with_tempo(&start, 3.0), true);
        assert!(!history.can_redo());
        assert_eq!(history.redo(), None);
        assert_eq!(history.undo().map(|s| s.tempo_bpm), Some(1.0));
    }

    #[test]
    fn unchanged_channels_are_shared() {
        let mut song = song_with(3);
        let mut history = History::new(&song);
        let clip = song.channels[2].lanes[0].add_clip(0, 4.0);
        clip.events.push(NoteEvent::Note {
            pos: 0,
            len_beats: 1.0,
            key: 60,
            velocity: NoteEvent::DEFAULT_VELOCITY,
            pan: NoteEvent::DEFAULT_PAN,
            slide: false,
        });
        history.edited(&song, true);
        let (before, after) = (&history.undo[0].channels, &history.current.channels);
        let shared: Vec<bool> = before
            .iter()
            .zip(after)
            .map(|(a, b)| Arc::ptr_eq(a, b))
            .collect();
        assert_eq!(shared, [true, true, false, true]);
        // A moved channel is still shared.
        let unmoved = song.clone();
        song.channels.swap(1, 3);
        history.edited(&song, true);
        let (before, after) = (&history.undo[1].channels, &history.current.channels);
        assert!(
            after
                .iter()
                .all(|a| before.iter().any(|b| Arc::ptr_eq(a, b)))
        );
        assert_eq!(history.undo(), Some(unmoved));
    }

    #[test]
    fn an_amend_is_no_step_and_undo_and_redo_lead_back_to_it() {
        // An audio recording's clips: the song changes, with no undo step of its own.
        let start = Song::default();
        let mut history = History::new(&start);
        let edited = with_tempo(&start, 100.0);
        history.edited(&edited, true);
        let recorded = song_with(1);
        history.amend(&edited, &recorded);
        assert_eq!(history.undo(), Some(start));
        assert_eq!(history.undo(), None);
        assert_eq!(history.redo(), Some(recorded.clone()));
        // A final edit that leaves the amended song as it is records nothing.
        history.edited(&recorded, true);
        assert_eq!(history.redo(), None);
        assert!(history.undo().is_some());
        assert_eq!(history.undo(), None);
    }

    #[test]
    fn an_amend_clears_redo_and_closes_an_open_gesture_first_as_its_own_step() {
        let start = Song::default();
        let mut history = History::new(&start);
        history.edited(&with_tempo(&start, 100.0), true);
        history.undo();
        // The step undone would drop the recording: it goes.
        history.amend(&start, &song_with(1));
        assert!(!history.can_redo());
        // A drag still open when the recording ends is a step of its own, from before the recording's change.
        let start = song_with(1);
        let mut history = History::new(&start);
        let dragged = with_tempo(&start, 130.0);
        history.edited(&dragged, false);
        let mut recorded = dragged.clone();
        recorded.add_channel(ChannelKind::Instrument);
        history.amend(&dragged, &recorded);
        history.edited(&recorded, true);
        assert_eq!(history.undo(), Some(start));
        assert_eq!(history.undo(), None);
        assert_eq!(history.redo(), Some(recorded));
        assert_eq!(history.redo(), None);
    }

    /// A song with one Instrument channel (the last) holding a DW Sampler (instance 2) and a send to the master, and
    /// that channel's index.
    fn rack_song() -> (Song, usize) {
        let mut song = song_with(1);
        let n = song.channels.len() - 1;
        let ch = &mut song.channels[n];
        ch.rack.modules.push(ModuleInstance::new(1, 2));
        ch.rack.last_instance_id = 2;
        ch.file.sends.push(crate::song::SendState::default());
        (song, n)
    }

    /// The channel's Level, its send's level, and its module's param 1 and Mix.
    fn knobs(song: &Song, n: usize) -> (f32, f32, f32, f32) {
        let ch = &song.channels[n];
        let module = ch.rack.get(2).unwrap();
        let send = ch.file.sends[0].level;
        (ch.params.0[0], send, module.params[1], module.common.0[1])
    }

    #[test]
    fn channel_params_make_no_step_and_no_undo_or_redo_puts_an_old_value_back() {
        // A control bound to a channel param saves no undo step.
        let (start, n) = rack_song();
        let mut history = History::new(&start);
        // An edit of the channel's own tracks, then its knobs.
        let mut edited = start.clone();
        edited.channels[n].add_lane(crate::song::LaneKind::Notes);
        history.edited(&edited, true);
        let mut knobbed = edited.clone();
        let ch = &mut knobbed.channels[n];
        (ch.params.0[0], ch.file.sends[0].level) = (0.3, 0.4);
        let module = ch.rack.get_mut(2).unwrap();
        (module.params[1], module.common.0[1]) = (0.7, 0.2);
        history.sync(&knobbed);
        assert_eq!(history.steps(), 1);
        let undone = history.undo().unwrap();
        let lanes = |s: &Song| s.channels[n].lanes.len();
        assert_eq!(lanes(&undone), lanes(&start), "the track edit is undone");
        assert_eq!(knobs(&undone, n), knobs(&knobbed, n));
        // The redo step takes a later move too.
        let mut moved = undone.clone();
        moved.channels[n].params.0[0] = 0.9;
        history.sync(&moved);
        let redone = history.redo().unwrap();
        assert_eq!(lanes(&redone), lanes(&edited));
        assert_eq!(knobs(&redone, n), (0.9, 0.4, 0.7, 0.2));
    }

    #[test]
    fn channel_params_leave_an_open_gesture_open_and_out_of_its_step() {
        let (start, n) = rack_song();
        let mut history = History::new(&start);
        let dragged = with_tempo(&start, 100.0);
        history.edited(&dragged, false);
        let mut knobbed = dragged.clone();
        knobbed.channels[n].params.0[1] = 0.1;
        history.sync(&knobbed);
        assert!(history.open);
        history.edited(&knobbed, true);
        let undone = history.undo().unwrap();
        assert_eq!(undone.tempo_bpm, start.tempo_bpm);
        assert_eq!(undone.channels[n].params.0[1], 0.1);
    }

    #[test]
    fn a_pick_stays_through_an_undo_of_an_earlier_edit_of_its_own_channel() {
        // The rack and presets are not covered by undo.
        let (start, n) = rack_song();
        let mut history = History::new(&start);
        let mut edited = start.clone();
        edited.channels[n].add_lane(crate::song::LaneKind::Notes);
        history.edited(&edited, true);
        // A module preset: the module's params and file state.
        let mut picked = edited.clone();
        let module = picked.channels[n].rack.get_mut(2).unwrap();
        (module.params[1], module.file.preset_name) = (0.6, "Picked".into());
        history.rack_state(&picked);
        let lanes = |s: &Song| s.channels[n].lanes.len();
        let undone = history.undo().unwrap();
        assert_eq!(lanes(&undone), lanes(&start));
        assert_eq!(undone.channels[n].rack, picked.channels[n].rack);
        assert_eq!(history.redo(), Some(picked));
        // An instrument pick in place of the module, which names the channel: the whole rack, and the name.
        history.undo();
        let mut swapped = history.current.song();
        let ch = &mut swapped.channels[n];
        ch.rack.modules[0] = crate::song::ModuleInstance::new(39, 3);
        (ch.rack.last_instance_id, ch.name) = (3, "3x Osc".into());
        history.rack_state(&swapped);
        let redone = history.redo().unwrap();
        assert_eq!(lanes(&redone), lanes(&edited));
        assert_eq!(redone.channels[n].rack, swapped.channels[n].rack);
        assert_eq!(redone.channels[n].name, "3x Osc");
    }

    #[test]
    fn an_undo_that_takes_away_a_pads_aux_target_sends_the_pad_to_its_channel() {
        // Regression: the target moved into the step from before its channel, whose id a new channel takes back after
        // The undo.
        let (start, n) = drums_song();
        let mut history = History::new(&start);
        let mut added = start.clone();
        added.add_channel(ChannelKind::EffectTrack);
        let fx = added.channels.last().unwrap().id;
        history.edited(&added, true);
        let mut aux = added.clone();
        kit(&mut aux, n).slots[0].file.send_target = fx;
        history.rack_state(&aux);
        let mut undone = history.undo().unwrap();
        assert_eq!(kit(&mut undone, n).slots[0].file.send_target, -1);
        undone.add_channel(ChannelKind::Instrument);
        assert_eq!(undone.channels.last().unwrap().id, fx, "the id comes back");
        // The redo brings the target back with its channel.
        let mut redone = history.redo().unwrap();
        assert_eq!(kit(&mut redone, n).slots[0].file.send_target, fx);
    }

    #[test]
    fn a_final_edit_keeps_the_param_values_of_its_frame_out_of_its_step() {
        // Regression: a channel param moved in the frame of a stronger edit joined its undo step.
        let (start, n) = rack_song();
        let mut history = History::new(&start);
        let mut both = with_tempo(&start, 100.0);
        both.channels[n].rack.get_mut(2).unwrap().common.0[1] = 0.2;
        assert!(history.edited(&both, true));
        let undone = history.undo().unwrap();
        assert_eq!(undone.tempo_bpm, start.tempo_bpm);
        assert_eq!(knobs(&undone, n).3, 0.2);
        // A final edit that changed only those makes no step.
        let mut only = undone.clone();
        only.channels[n].params.0[0] = 0.1;
        assert!(!history.edited(&only, true));
        assert!(history.can_redo());
    }

    #[test]
    fn rack_edits_make_no_step_and_no_undo_or_redo_changes_a_rack() {
        // The rack and presets are not covered by undo: delete a module, pick one, undo the delete: the rack stays.
        let (start, n) = rack_song();
        let mut history = History::new(&start);
        let mut edited = start.clone();
        edited.channels[n].name = "Edited".into();
        history.edited(&edited, true);
        let mut deleted = edited.clone();
        deleted.channels[n].rack.modules.clear();
        assert!(!history.edited(&deleted, true), "a delete is no step");
        let mut picked = deleted.clone();
        let rack = &mut picked.channels[n].rack;
        (rack.modules, rack.last_instance_id) = (vec![ModuleInstance::new(6, 3)], 3);
        history.rack_state(&picked);
        assert_eq!(history.steps(), 1);
        let undone = history.undo().unwrap();
        assert_eq!(undone.channels[n].name, start.channels[n].name);
        assert_eq!(undone.channels[n].rack, picked.channels[n].rack);
        // A channel an undo brings back keeps the rack it went with.
        history.redo();
        let mut gone = history.current.song();
        gone.delete_channel(n);
        assert!(history.edited(&gone, true));
        let back = history.undo().unwrap();
        assert_eq!(back.channels[n].rack, picked.channels[n].rack);
    }

    #[test]
    fn module_ids_never_come_back_across_undo_branches() {
        // Regression: delete, pick a Distortion (id 3), undo, pick a Tremolo, move its p1, redo: the Distortion is
        // untouched.
        let (start, n) = rack_song();
        let mut history = History::new(&start);
        let mut edited = start.clone();
        edited.channels[n].name = "Edited".into();
        history.edited(&edited, true);
        let mut picked = edited.clone();
        let rack = &mut picked.channels[n].rack;
        (rack.modules, rack.last_instance_id) = (vec![ModuleInstance::new(6, 3)], 3);
        history.edited(&picked, true);
        let mut tremolo = history.undo().unwrap();
        let rack = &mut tremolo.channels[n].rack;
        assert_eq!(rack.last_instance_id, 3);
        let id = rack.last_instance_id + 1;
        rack.modules.push(ModuleInstance::new(46, id));
        (rack.last_instance_id, rack.modules[1].params[1]) = (id, 0.9);
        history.rack_state(&tremolo);
        let redone = history.redo().unwrap();
        let rack = &redone.channels[n].rack;
        assert_eq!(rack.get(3), Some(&ModuleInstance::new(6, 3)));
        assert_eq!(rack.get(4).map(|m| m.params[1]), Some(0.9));
    }

    #[test]
    fn init_values_pad_params_and_pad_samples_stay_through_an_undo() {
        // A DW Sampler's kept init values follow its Attack (`init_follows`); a pad param moved by a fine tune or a MIDI
        // link; a pad's sample pick (rack state).
        let (mut start, d) = drums_song();
        let i = d - 1;
        start.channels[i]
            .rack
            .modules
            .push(ModuleInstance::new(1, 2));
        let mut history = History::new(&start);
        let mut edited = start.clone();
        for c in [i, d] {
            edited.channels[c].name = "Edited".into();
        }
        history.edited(&edited, true);
        let mut moved = edited.clone();
        let dw = moved.channels[i].rack.get_mut(2).unwrap();
        (dw.params[1], dw.file.program_init) = (0.3, Some([0.3, 0.7]));
        let pad = &mut kit(&mut moved, d).slots[0];
        (pad.params[3], pad.name, pad.folder) = (0.9, "Picked".into(), "Drums/Picked".into());
        history.sync(&moved);
        let mut undone = history.undo().unwrap();
        assert_eq!(undone.channels[d].name, start.channels[d].name);
        let init = undone.channels[i].rack.get(2).unwrap().file.program_init;
        assert_eq!(init, Some([0.3, 0.7]));
        let pad = &kit(&mut undone, d).slots[0];
        assert_eq!((pad.params[3], pad.name.as_str()), (0.9, "Picked"));
    }

    #[test]
    fn param_values_the_history_was_not_given_stay_through_an_amend_and_its_undo() {
        // An audio take that ends while a knob's values wait for its gesture's end (`HoneydewApp::audio_record`).
        let (start, n) = rack_song();
        let mut history = History::new(&start);
        history.edited(&with_tempo(&start, 100.0), true);
        let mut knobbed = with_tempo(&start, 100.0);
        knobbed.channels[n].params.0[0] = 0.3;
        let mut recorded = knobbed.clone();
        recorded.add_channel(ChannelKind::Instrument);
        history.amend(&knobbed, &recorded);
        let undone = history.undo().unwrap();
        assert_eq!(undone.tempo_bpm, start.tempo_bpm);
        assert_eq!(undone.channels[n].params.0[0], 0.3);
    }

    #[test]
    fn an_undo_shares_the_channel_it_gives_the_live_state_with_every_step_that_held_it() {
        // Regression: one knob move, then undos, left a copy of the channel (clips too) per step.
        let (start, n) = rack_song();
        let mut history = History::new(&start);
        for bpm in [100.0, 110.0, 120.0] {
            history.edited(&with_tempo(&start, bpm), true);
        }
        let mut knobbed = with_tempo(&start, 120.0);
        knobbed.channels[n].params.0[0] = 0.3;
        history.sync(&knobbed);
        for _ in 0..3 {
            assert_eq!(history.undo().unwrap().channels[n].params.0[0], 0.3);
        }
        let steps = (history.undo.iter())
            .chain(&history.redo)
            .chain([&history.current]);
        let copies: Vec<_> = steps.map(|s| Arc::as_ptr(&s.channels[n])).collect();
        assert!(copies.iter().all(|&c| c == copies[0]), "{copies:?}");
    }

    #[test]
    fn knob_moves_between_undos_leave_one_shared_copy_of_the_channel() {
        // Regression: a knob move before each undo left one more copy of the channel in the redo steps.
        let (start, n) = rack_song();
        let mut history = History::new(&start);
        for i in 1..=50 {
            history.edited(&with_tempo(&start, 100.0 + f64::from(i)), true);
        }
        let mut live = history.current.song();
        for i in 0..50u16 {
            let level = f32::from(i) / 100.0;
            live.channels[n].params.0[0] = level;
            history.sync(&live);
            live = history.undo().unwrap();
            assert_eq!(live.channels[n].params.0[0], level);
        }
        let steps = (history.undo.iter())
            .chain(&history.redo)
            .chain([&history.current]);
        let copies: Vec<_> = steps.map(|s| Arc::as_ptr(&s.channels[n])).collect();
        assert!(copies.iter().all(|&c| c == copies[0]), "{copies:?}");
    }

    #[test]
    fn pad_ids_never_come_back_across_undo_branches() {
        // Regression: a pad duplicated (X), undone, a sample picked onto a new pad (Y), redone: X took Y's id,
        // sample and params.
        let (start, d) = drums_song();
        let mut history = History::new(&start);
        let mut duplicated = start.clone();
        let x = kit(&mut duplicated, d).slots[0].clone();
        let at = kit(&mut duplicated, d).slots.len();
        let x_id = kit(&mut duplicated, d).insert_slot(at, x.clone());
        history.edited(&duplicated, true);
        let mut picked = history.undo().unwrap();
        let y = SamplerSlot {
            name: "Y".into(),
            params: [0.1; 11],
            ..SamplerSlot::default()
        };
        let y_id = kit(&mut picked, d).insert_slot(at, y);
        assert_ne!(y_id, x_id);
        history.rack_state(&picked);
        let mut redone = history.redo().unwrap();
        let slot = kit(&mut redone, d).slot_by_id(x_id).unwrap();
        assert_eq!((&slot.name, slot.params), (&x.name, x.params));
    }

    /// What [`shortcut`] returns for one Z press with `modifiers`, with a text field focused if `focused`.
    fn press_z(modifiers: Modifiers, focused: bool) -> Option<Step> {
        press(Key::Z, modifiers, focused)
    }

    /// [`press_z`] for `key`.
    fn press(key: Key, modifiers: Modifiers, focused: bool) -> Option<Step> {
        let ctx = Context::default();
        if focused {
            ctx.memory_mut(|m| m.request_focus(eframe::egui::Id::new("field")));
        }
        let input = RawInput {
            events: vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        };
        let mut step = None;
        let _ = ctx.run(input, |ctx| step = shortcut(ctx));
        step
    }

    #[test]
    fn shortcuts() {
        let ctrl = Modifiers::CTRL | Modifiers::COMMAND;
        let cmd = Modifiers::MAC_CMD | Modifiers::COMMAND;
        assert_eq!(press_z(ctrl, false), Some(Step::Undo));
        assert_eq!(press_z(cmd, false), Some(Step::Undo));
        assert_eq!(press_z(ctrl | Modifiers::SHIFT, false), Some(Step::Redo));
        assert_eq!(press_z(cmd | Modifiers::SHIFT, false), Some(Step::Redo));
        // Windows' redos: Ctrl+Alt+Z, and Ctrl+Y.
        assert_eq!(press_z(ctrl | Modifiers::ALT, false), Some(Step::Redo));
        assert_eq!(press(Key::Y, ctrl, false), Some(Step::Redo));
        assert_eq!(press_z(Modifiers::NONE, false), None);
        assert_eq!(press_z(Modifiers::SHIFT, false), None);
        // Ctrl alone on macOS is not Cmd.
        assert_eq!(press_z(Modifiers::CTRL, false), None);
        // A focused text field keeps its own undo.
        assert_eq!(press_z(ctrl, true), None);
    }
}

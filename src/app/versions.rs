//! A project version restored (#69, [`crate::ui::views::versions`]): the song the versions window left becomes the song.

use super::{Handover, HoneydewApp};

impl HoneydewApp {
    /// The song the versions window restored becomes the song as one undo step, which undo and redo restore whole
    /// ([`crate::history::History::replaced`]); the plugins start anew from its states, as for a song opened.
    pub(super) fn restore_version(&mut self, tick: u64) {
        let Some(song) = self.slots.versions.restored.take() else {
            return;
        };
        // The step before is the song as it is now, the plugins' states too.
        self.params_moved = false;
        crate::plugins::store_states(&mut self.song);
        self.history.edited(&self.song, true);
        self.song = song;
        self.history.replaced(&self.song);
        self.files.song_changed();
        crate::plugins::song_replaced();
        self.rekey(&[]);
        self.selected_channel = self.selected_channel.min(self.song.channels.len() - 1);
        self.rebuild_as(tick, Handover::Undo);
    }
}

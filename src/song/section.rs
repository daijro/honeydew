//! Song sections (Honeydew's own, #80): named, coloured time ranges of the playlist, which tint its track rows and
//! name themselves on its ruler. A file keeps each in an `HDSe` (`flm::write`), which FL Studio Mobile skips. The audio
//! never reads them.

use super::{HUE_START, HUE_STEP, Song};

/// A section of the song.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// Song ticks, `start` < `end`.
    pub start: u32,
    pub end: u32,
    pub name: String,
    /// Its colour, as a channel's hue ([`crate::theme::channel_color`]).
    pub hue: f32,
}

impl Song {
    /// The section over song ticks `start..end`: the one there already, else a new one, `Section N` (N its number) in
    /// the next hue as channels take them. Returns its index in [`Song::sections`].
    pub fn section_at(&mut self, start: u32, end: u32) -> usize {
        if let Some(i) = (self.sections.iter()).position(|s| (s.start, s.end) == (start, end)) {
            return i;
        }
        let n = self.sections.len() + 1;
        self.sections.push(Section {
            start,
            end,
            name: format!("Section {n}"),
            hue: (HUE_START + n as f64 * HUE_STEP).fract() as f32,
        });
        n - 1
    }
}

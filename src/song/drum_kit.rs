//! The Drums channel's pad sampler (`RSMP`) and its sampler slots, with their slot params. The fields the drums UI
//! uses are on the structs; the rest of the file's (the pad map, `LINk`, `STRC`, content path …) is in their `file`.

use super::file::{KitFileState, SlotFileState};
use crate::params::DRUM_SLOT_PARAMS;

/// The pads of a Drums channel (`RSMP`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrumKit {
    /// Slot `i` has pad index `i` (`MAIN`@4, the key drum events use; pad indexes run 0, 1, 2 … in every file).
    /// Which hex pad shows it is the `PADS` map's (`file.pads`, pad → slot), which rearranging the pads permutes;
    /// a pad whose entry is past the last slot is unused.
    pub slots: Vec<SamplerSlot>,
    /// `RSMP` `lastSlotId`, `PADS` and unknown chunks.
    pub file: KitFileState,
}

/// The default kit's pads, in pad order; hats share cut group 1.
const DEFAULT_PADS: [(&str, u8); 8] = [
    ("Kick", 0),
    ("Snare", 0),
    ("Clap", 0),
    ("Closed Hat", 1),
    ("Open Hat", 1),
    ("Tom", 0),
    ("Perc", 0),
    ("Crash", 0),
];

impl DrumKit {
    /// The kit [`super::Song::insert_channel`] gives a Drums channel, and the drumset picker's `Default` (the app's new
    /// Drums channel has one empty pad instead): [`DEFAULT_PADS`] on built-in one-shots. A slot plays built-in
    /// one-shot `<Pad>` when its folder is `Built-in/<Pad>`; it has no sample file and no content path (`PTH1`),
    /// so the engine synthesizes the sound.
    /// The slots get ids 1, 2, 3 … (`lastSlotId` is the slot count in a fresh kit).
    pub fn default_kit() -> Self {
        let mut kit = Self::default();
        for (pad, cut_group) in DEFAULT_PADS {
            let slot = SamplerSlot {
                name: format!("Synth {pad}"),
                folder: format!("Built-in/{pad}"),
                cut_group,
                ..SamplerSlot::default()
            };
            kit.insert_slot(kit.slots.len(), slot);
        }
        kit
    }

    /// Inserts `slot` at pad `index` (clamped to the end), with a new id (`lastSlotId + 1`), and returns the
    /// id. The later slots move up a pad. Ids are never reused, so a copy of a slot gets its own.
    pub fn insert_slot(&mut self, index: usize, mut slot: SamplerSlot) -> u32 {
        self.file.last_slot_id += 1;
        slot.file.id = self.file.last_slot_id;
        self.slots.insert(index.min(self.slots.len()), slot);
        self.file.last_slot_id
    }

    /// The slot with `LINk` id `id` (what a negative automation instance names).
    pub fn slot_by_id(&self, id: u32) -> Option<&SamplerSlot> {
        self.slots.iter().find(|s| s.file.id == id)
    }

    /// The slot with pad index `pad` (`MAIN`@4, its place in the kit), `None` past the last slot. Drum events and the
    /// step rows use it as it is; a hex pad on screen reaches its slot through the `PADS` map first
    /// (`ui::drums::pad_slot`).
    pub fn slot_on_pad(&self, pad: usize) -> Option<&SamplerSlot> {
        self.slots.get(pad)
    }

    /// The key the slot with pad index `pad` plays: its effective note, the
    /// slot's stored MIDI note or else 48 + its pad index (`MAIN`@4, here `pad`). `None` past the last slot, or for a
    /// default past MIDI key 127. A hex pad plays the note of its `PADS` entry's slot (`ui::drums::pad_slot`).
    pub fn pad_note(&self, pad: usize) -> Option<u8> {
        let slot = self.slot_on_pad(pad)?;
        slot.midi_note()
            .or_else(|| u8::try_from(48 + pad).ok().filter(|&n| n < 128))
    }
}

/// One pad's sample and settings (`SMPl` / `SMPL`).
#[derive(Clone, Debug, PartialEq)]
pub struct SamplerSlot {
    /// `MAIN` display name: the sample's name without its extension, the small line under the pad name
    /// (e.g. `Kick 01`).
    pub name: String,
    /// `MAIN` content folder, e.g. `Drum Samples/Kick`.
    pub folder: String,
    /// `MAIN` cut (choke) group: 0 none, 1–3 the `Cut...` menu's groups A–C.
    pub cut_group: u8,
    /// `PRMS`, normalised, indexed like [`DRUM_SLOT_PARAMS`].
    pub params: [f32; 11],
    /// Its id, `LINk`, stretch, path and the rest of `MAIN`/`PRST`.
    pub file: SlotFileState,
}

impl Default for SamplerSlot {
    fn default() -> Self {
        Self {
            name: String::new(),
            folder: String::new(),
            cut_group: 0,
            params: std::array::from_fn(|i| DRUM_SLOT_PARAMS[i].default),
            file: SlotFileState::default(),
        }
    }
}

impl SamplerSlot {
    /// The stored MIDI note (`LINk`@8), `None` for none (0xFFFF, read as −1) or a value
    /// that is no MIDI key.
    pub fn midi_note(&self) -> Option<u8> {
        u8::try_from(self.file.midi_key).ok().filter(|&n| n < 128)
    }

    /// The sample file's parent folder (`Kick` for `Drum Samples/Kick`): the last part of the content folder. The
    /// pad name drawn large unless a custom name replaces it ([`Self::display_name`]).
    pub fn pad_name(&self) -> &str {
        self.folder.rsplit(['\\', '/']).next().unwrap_or_default()
    }

    /// The pad's name: the custom pad name (`MAIN`@1051, "" = none), else [`Self::pad_name`]. A row header's first
    /// line, and what its automation lanes name.
    pub fn display_name(&self) -> &str {
        match self.file.custom_pad_name.as_str() {
            "" => self.pad_name(),
            name => name,
        }
    }
}

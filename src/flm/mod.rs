pub mod chunk;
mod legacy;
mod read;
mod write;

pub use chunk::FlmError;
pub use legacy::{read_flgsynth, read_flms};
pub use read::{
    BUDGET_BASE, MAX_ID, preset_fits, preset_type, read, read_drumset, read_preset, read_rack,
    read_with_notices,
};
pub use write::{write, write_drumset, write_preset, write_rack};
pub mod sf2d;

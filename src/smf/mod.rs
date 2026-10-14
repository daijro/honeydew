//! Standard MIDI files: the byte codec ([`codec`]) and the song export ([`export`]). Import builds on the same codec.

pub mod codec;
pub mod export;
pub mod import;

pub use codec::{Smf, SmfEvent};
pub use export::{NoData, smf_of};

#[cfg(test)]
mod tests;

//! MIDI input, pure part: messages, CC links and the input router.

pub mod device;
pub mod links;
pub mod message;
pub mod route;

pub use links::CcLink;
pub use message::MidiMsg;
pub use route::{Learn, Out, Router};

#[cfg(test)]
mod tests;

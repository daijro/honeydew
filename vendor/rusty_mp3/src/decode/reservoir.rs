//! The bit reservoir.
//!
//! MP3 main data is not aligned to frame boundaries: a frame's `main_data_begin`
//! points *backwards* by up to 511 bytes into bytes carried over from previous
//! frames. The reservoir holds that tail so each frame's main data can be
//! reassembled into one contiguous bitstream for the Huffman/scalefactor stages.

// Parses untrusted bytes: narrowing casts are lint-enforced here (see the crate
// lint policy in Cargo.toml) -- every one is masked, typed, or states its bound.
#![warn(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

/// Rolling buffer of recent main-data bytes.
#[derive(Default)]
pub struct Reservoir {
    /// The previous frame's ASSEMBLED main data, in full. Its last 512 bytes
    /// (or all of it, if shorter) are the reservoir the next frame may reach
    /// back into; the buffer doubles as that frame's assembly space.
    buf: Vec<u8>,
}

/// How far back a frame may reach (`main_data_begin` is at most 511).
const RESERVOIR_BYTES: usize = 512;

impl Reservoir {
    /// Reassemble this frame's main data: take `main_data_begin` bytes from the
    /// reservoir tail, append the current frame's main data, and keep the result
    /// as the reservoir for the next frame.
    ///
    /// Assembled IN PLACE: slide the bytes this frame borrows (at most 511) to
    /// the front of the one buffer, append, and lend it out. It used to build a
    /// fresh `Vec` per frame and then a second one for the carried-over tail
    /// (`rev().take(512).rev().collect()`) -- two allocations and two copies a
    /// frame. The bytes are the same: the reservoir is still "the last 512 of
    /// the previous assembly", and a frame still gets `min(begin, available)` of
    /// them.
    pub fn assemble(&mut self, main_data_begin: u16, frame_main_data: &[u8]) -> &[u8] {
        let available = self.buf.len().min(RESERVOIR_BYTES);
        let take = available.min(main_data_begin as usize);
        self.buf.drain(..self.buf.len() - take);
        self.buf.extend_from_slice(frame_main_data);
        &self.buf
    }

    /// Drop carried-over state (seek / discontinuity).
    pub fn reset(&mut self) {
        self.buf.clear();
    }
}

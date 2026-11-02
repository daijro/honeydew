//! Ogg Vorbis writing: `OggEncoder` takes the render in
//! slices, as `FlacEncoder` does, and makes a stereo VBR Vorbis stream in Ogg pages, quality 1.0 (libvorbis q10) with
//! `High quality` on and 0.4 (q4) off, through zvidlib-vorbis-encoder (a port of libvorbis 1.3.7 whose packets are
//! libvorbis's own; MIT, with Xiph's BSD-3 notice) and the `ogg` crate's page writer (BSD-3). Mono input is duplicated.
//!
//! - **Pages.** The identification, comment and setup headers each fill a page of their own, BOS on the first and
//!   granule position 0; audio packets follow with the granule positions the encoder gives them, and the last one ends
//!   the stream (EOS). Its granule position is the stream's length (the input's, or its averaged length above
//!   [`MAX_RATE`]), so a decoder trims the stream to it exactly.
//! - **Deterministic.** The serial number is the constant [`SERIAL`], not random, and the encoder has no randomness:
//!   the same input pushed in the same slices makes the same bytes. (libvorbis extrapolates the stream's start from
//!   the samples its first writes hold, so the first slices' sizes are part of "the same input".)
//! - **Rate.** The engine's rate, not resampled, up to [`MAX_RATE`]. libvorbis has no setup above it, so there whole
//!   groups of frames are averaged into one: 384 kHz goes out at 192 kHz.
//!
//! This file builds on its own, with no `crate::` paths: `tools/wasm-smoke` includes it by `#[path]`.

use ogg::{PacketWriteEndInfo, PacketWriter};
use zvidlib_vorbis_encoder::vorbis_encoder::VorbisEncoder;

/// The comment header's vendor string: the one zvidlib's own Vorbis encoder writes, naming the encoder.
pub const VENDOR: &str = "zvidlib 0.5.0 (port of Xiph.Org libVorbis I 20200704)";
/// The logical stream's serial number (any constant; the file holds one stream).
pub const SERIAL: u32 = 1;
/// The highest rate libvorbis has a setup for.
pub const MAX_RATE: u32 = 200_000;

/// A streaming stereo Ogg Vorbis encoder.
pub struct OggEncoder {
    vorbis: VorbisEncoder,
    pages: PacketWriter<Vec<u8>>,
    /// The last audio packet and its granule position, held back until the next one shows it is not the last.
    held: Option<(Vec<u8>, u64)>,
    /// Input frames averaged into one stream frame: 1 up to [`MAX_RATE`].
    ///
    /// ponytail: a box average, no real low-pass. What lies above the new Nyquist folds down barely damped (a box of 2
    /// still passes half the amplitude at 2/3 of the new rate). Only rates above 200 kHz come here; `mp3_writer`'s
    /// windowed-sinc halving if one matters.
    group: usize,
    /// The sums and count of the group under way.
    partial: ([f32; 2], usize),
}

impl OggEncoder {
    /// An encoder of audio at `rate` Hz: VBR quality 1.0 with `high_quality`, else 0.4.
    ///
    /// # Panics
    /// On a zero `rate`.
    pub fn new(rate: u32, high_quality: bool) -> Self {
        assert!(rate > 0, "Vorbis needs a sample rate");
        let group = rate.div_ceil(MAX_RATE);
        let quality = if high_quality { 1.0 } else { 0.4 };
        let mut encoder = Self {
            vorbis: VorbisEncoder::new(rate / group, 2, quality).expect("libvorbis has a setup"),
            pages: PacketWriter::new(Vec::new()),
            held: None,
            group: group as usize,
            partial: ([0.0; 2], 0),
        };
        for header in encoder.vorbis.headers(VENDOR) {
            encoder.write(header, 0, PacketWriteEndInfo::EndPage);
        }
        encoder
    }

    /// Appends one slice per channel: one (mono, duplicated) or two (left, right). The slices may be any length, but
    /// of equal length: if they differ, the shortest counts and the rest of the longer one is dropped.
    ///
    /// # Panics
    /// If there are not one or two slices.
    pub fn push<C: AsRef<[f32]>>(&mut self, channels: &[C]) {
        assert!(
            matches!(channels.len(), 1 | 2),
            "Vorbis takes 1 or 2 channels"
        );
        let [left, right] = [0, 1].map(|c| channels[c.min(channels.len() - 1)].as_ref());
        let mut frames = Vec::with_capacity(2 * left.len() / self.group + 2);
        for (&l, &r) in left.iter().zip(right) {
            let (sums, count) = &mut self.partial;
            (sums[0], sums[1], *count) = (sums[0] + l, sums[1] + r, *count + 1);
            if *count == self.group {
                frames.extend(self.take_group());
            }
        }
        self.encode(&frames);
    }

    /// The finished file: the group under way, the encoder's last packets, the last of them ending the stream.
    pub fn finish(mut self) -> Vec<u8> {
        if self.partial.1 > 0 {
            let frame = self.take_group();
            self.encode(&frame);
        }
        let last = self.vorbis.finish().expect("finished once");
        let mut packets = self.held.take().into_iter().chain(last).peekable();
        while let Some((packet, granule)) = packets.next() {
            let end = match packets.peek() {
                Some(_) => PacketWriteEndInfo::NormalPacket,
                None => PacketWriteEndInfo::EndStream,
            };
            self.write(packet, granule, end);
        }
        self.pages.into_inner()
    }

    /// The average of the group under way, which starts a new one.
    fn take_group(&mut self) -> [f32; 2] {
        let ([l, r], count) = std::mem::take(&mut self.partial);
        let count = count as f32;
        [l / count, r / count]
    }

    /// Encodes interleaved stereo `frames`, writing every packet but the newest.
    fn encode(&mut self, frames: &[f32]) {
        let packets = self.vorbis.encode(frames).expect("whole stereo frames");
        for packet in packets {
            if let Some((packet, granule)) = self.held.replace(packet) {
                self.write(packet, granule, PacketWriteEndInfo::NormalPacket);
            }
        }
    }

    fn write(&mut self, packet: Vec<u8>, granule: u64, end: PacketWriteEndInfo) {
        (self.pages.write_packet(packet.into(), SERIAL, end, granule))
            .expect("a Vec takes every write");
    }
}

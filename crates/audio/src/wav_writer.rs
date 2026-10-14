//! WAV writing. `WavWriter` streams the 16-bit PCM files recordings make; `float32_wav` builds the 32-bit float file
//! the audio clip editor saves. Both use the 46-byte header: RIFF, WAVE, an 18-byte `fmt ` chunk with cbSize 0, then
//! `data`.

use std::io::{self, Seek, SeekFrom, Write};

pub const HEADER_LEN: usize = 46;

/// The header for `data_len` data bytes. Sizes past what a `u32` can hold saturate, so an unknown size
/// (`u64::MAX`) reads as "to the end of the file".
fn header(format: u16, channels: u16, rate: u32, bits: u16, data_len: u64) -> Vec<u8> {
    let data = u32::try_from(data_len)
        .unwrap_or(u32::MAX)
        .min(u32::MAX - 38);
    let align = channels * (bits / 8);
    let mut h = Vec::with_capacity(HEADER_LEN);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(38 + data).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&18u32.to_le_bytes());
    h.extend_from_slice(&format.to_le_bytes());
    h.extend_from_slice(&channels.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&rate.saturating_mul(u32::from(align)).to_le_bytes());
    h.extend_from_slice(&align.to_le_bytes());
    h.extend_from_slice(&bits.to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes()); // cbSize
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    h
}

/// A streaming 16-bit PCM WAV writer. Samples are stored as trunc(x·32767), clamped to ±32767 (NaN → 0).
pub struct WavWriter<W: Write + Seek> {
    out: W,
    start: u64,
    channels: u16,
    rate: u32,
    data_len: u64,
    buf: Vec<u8>,
}

impl<W: Write + Seek> WavWriter<W> {
    /// Writes the header at the output's current position. Until `finish` its sizes say "to the end of the file", so
    /// a recording cut short by a crash still decodes.
    pub fn new(mut out: W, rate: u32, channels: u16) -> io::Result<Self> {
        let start = out.stream_position()?;
        out.write_all(&header(1, channels, rate, 16, u64::MAX))?;
        Ok(Self {
            out,
            start,
            channels,
            rate,
            data_len: 0,
            buf: Vec::new(),
        })
    }

    /// Appends interleaved samples, in whole frames. A partial frame (a length that is not a multiple of the channel
    /// count) is an `InvalidInput` error and writes nothing, so the file stays frame-aligned.
    pub fn write(&mut self, interleaved: &[f32]) -> io::Result<()> {
        if !interleaved
            .len()
            .is_multiple_of(usize::from(self.channels.max(1)))
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "partial frame"));
        }
        self.buf.clear();
        // f64: an f32 product can round up onto the next integer before the truncation.
        let pcm = |x: f32| (f64::from(x) * 32767.0).clamp(-32767.0, 32767.0) as i16;
        self.buf
            .extend(interleaved.iter().flat_map(|&x| pcm(x).to_le_bytes()));
        self.out.write_all(&self.buf)?;
        self.data_len += self.buf.len() as u64;
        Ok(())
    }

    /// Patches the sizes, flushes and returns the output, positioned after the data.
    pub fn finish(mut self) -> io::Result<W> {
        self.out.seek(SeekFrom::Start(self.start))?;
        self.out
            .write_all(&header(1, self.channels, self.rate, 16, self.data_len))?;
        self.out.seek(SeekFrom::Start(
            self.start + HEADER_LEN as u64 + self.data_len,
        ))?;
        self.out.flush()?;
        Ok(self.out)
    }
}

/// A 32-bit float WAV (format 3) of `channels`, which hold equal-length frames; extra frames in a longer channel are
/// dropped. No `fact` chunk.
pub fn float32_wav<C: AsRef<[f32]>>(rate: u32, channels: &[C]) -> Vec<u8> {
    let frames = channels.iter().map(|c| c.as_ref().len()).min().unwrap_or(0);
    let data_len = frames * channels.len() * 4;
    let mut out = header(3, channels.len() as u16, rate, 32, data_len as u64);
    out.reserve(data_len);
    for i in 0..frames {
        for c in channels {
            out.extend_from_slice(&c.as_ref()[i].to_le_bytes());
        }
    }
    out
}

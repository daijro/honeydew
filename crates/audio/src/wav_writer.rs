//! WAV writing. `WavWriter` streams the 16-bit PCM files recordings make; `float32_wav` builds the 32-bit float file
//! the audio clip editor saves. Both use the 46-byte header: RIFF, WAVE, an 18-byte `fmt ` chunk with cbSize 0, then
//! `data`. A tagged file ([`WavWriter::with_tags`]) has a `LIST` `INFO` chunk ([`info_chunk`]) before `data`.

use std::io::{self, Seek, SeekFrom, Write};

pub const HEADER_LEN: usize = 46;

/// The header for `data_len` data bytes, `info` (a whole chunk, or nothing) before `data`. Sizes past what a `u32` can
/// hold saturate, so an unknown size (`u64::MAX`) reads as "to the end of the file".
pub fn header(
    format: u16,
    channels: u16,
    rate: u32,
    bits: u16,
    data_len: u64,
    info: &[u8],
) -> Vec<u8> {
    let fixed = 38 + info.len() as u32;
    let data = u32::try_from(data_len)
        .unwrap_or(u32::MAX)
        .min(u32::MAX - fixed);
    let align = channels * (bits / 8);
    let mut h = Vec::with_capacity(HEADER_LEN + info.len());
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(fixed + data).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&18u32.to_le_bytes());
    h.extend_from_slice(&format.to_le_bytes());
    h.extend_from_slice(&channels.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&rate.saturating_mul(u32::from(align)).to_le_bytes());
    h.extend_from_slice(&align.to_le_bytes());
    h.extend_from_slice(&bits.to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes()); // cbSize
    h.extend_from_slice(info);
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
    /// The `LIST` `INFO` chunk, empty for none.
    info: Vec<u8>,
}

impl<W: Write + Seek> WavWriter<W> {
    /// Writes the header at the output's current position. Until `finish` its sizes say "to the end of the file", so
    /// a recording cut short by a crash still decodes.
    pub fn new(out: W, rate: u32, channels: u16) -> io::Result<Self> {
        Self::with_tags(out, rate, channels, &[])
    }

    /// [`Self::new`], with a `LIST` `INFO` chunk of `tags` before the data ([`info_chunk`]).
    pub fn with_tags(
        mut out: W,
        rate: u32,
        channels: u16,
        tags: &[(&str, &str)],
    ) -> io::Result<Self> {
        let start = out.stream_position()?;
        let info = info_chunk(tags);
        out.write_all(&header(1, channels, rate, 16, u64::MAX, &info))?;
        Ok(Self {
            out,
            start,
            channels,
            rate,
            data_len: 0,
            buf: Vec::new(),
            info,
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
        let header = header(1, self.channels, self.rate, 16, self.data_len, &self.info);
        self.out.write_all(&header)?;
        (self.out).seek(SeekFrom::Start(
            self.start + header.len() as u64 + self.data_len,
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
    let mut out = header(3, channels.len() as u16, rate, 32, data_len as u64, &[]);
    out.reserve(data_len);
    for i in 0..frames {
        for c in channels {
            out.extend_from_slice(&c.as_ref()[i].to_le_bytes());
        }
    }
    out
}

/// A `LIST` `INFO` chunk of `tags`, Vorbis comment fields as INFO ids: `TITLE` INAM, `ARTIST` IART, `ALBUM` IPRD,
/// `GENRE` IGNR, `DATE` ICRD, `COMMENT` ICMT, `ENCODER` ISFT; each text in UTF-8, zero-terminated, its chunk padded to
/// an even size. Other fields are left out; with none there is no chunk.
pub fn info_chunk(tags: &[(&str, &str)]) -> Vec<u8> {
    let mut info = b"INFO".to_vec();
    for &(field, value) in tags {
        let id: &[u8; 4] = match field {
            "TITLE" => b"INAM",
            "ARTIST" => b"IART",
            "ALBUM" => b"IPRD",
            "GENRE" => b"IGNR",
            "DATE" => b"ICRD",
            "COMMENT" => b"ICMT",
            "ENCODER" => b"ISFT",
            _ => continue,
        };
        let size = value.len() as u32 + 1;
        info.extend([&id[..], &size.to_le_bytes(), value.as_bytes(), &[0]].concat());
        if size % 2 == 1 {
            info.push(0);
        }
    }
    if info.len() == 4 {
        return Vec::new();
    }
    [&b"LIST"[..], &(info.len() as u32).to_le_bytes(), &info].concat()
}

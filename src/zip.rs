//! The ZIP container of a project archive (standard ZIP; the PKWARE APPNOTE layout): local headers, central
//! directory, end record, methods 0 (stored) and 8 (deflate), no zip64, no encryption. Deflate and the CRC-32 are
//! `miniz_oxide` and `crc32fast`.
//!
//! [`read`] takes hostile bytes (archives come from users) and never panics or allocates beyond its budget. The
//! central directory is the truth; the local headers only say where the data starts. What it refuses (all as
//! [`ZipError::Corrupt`], "Failed to decompress project archive"):
//! - names that leave the archive's folder: absolute, `..`, drive letters, empty, NUL, over 4096 bytes (also on folder
//!   records), and symlinks or other special files (the Unix mode in the external attributes);
//! - bombs: the declared sizes are checked before anything is inflated, against the caller's `max_total` and against
//!   [`RATIO_ALLOWANCE`] + [`MAX_RATIO`] × the archive's size, and an entry inflates into exactly its declared size;
//! - encryption, zip64 (its 0xFFFF / 0xFFFFFFFF markers fail the caps and bounds), split archives, other methods, a
//!   CRC mismatch, entries that overlap each other or the directory (the overlapping-file bomb), more than
//!   [`MAX_ENTRIES`] records, and an end record that counts fewer entries than the directory holds.
//!
//! `read` does not look at what the extracted paths are on disk: a file and a folder of the same name are the
//! extractor's error.
//!
//! [`open_entry`] streams one file of an archive too large to hold in memory (the OBB in an APK).

use std::collections::HashSet;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};

use miniz_oxide::deflate::compress_to_vec;
use miniz_oxide::inflate::decompress_to_vec_with_limit;
use miniz_oxide::inflate::stream::{InflateState, inflate};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

/// More central directory records than this (folders included) are refused.
pub const MAX_ENTRIES: usize = 10_000;
/// The zip-bomb ratio cap: all entries together may inflate to at most [`RATIO_ALLOWANCE`] + this × the archive's size.
/// Deflate itself tops out near 1032:1, so 250 refuses only data that is almost all one repeated byte (a minute of
/// digital silence is ~5 MiB and fits the allowance).
pub const MAX_RATIO: u64 = 250;
/// Output allowed whatever the ratio, so a small archive of silence still opens.
pub const RATIO_ALLOWANCE: u64 = 16 << 20;
/// Longer entry names are refused (no file system takes one).
const MAX_NAME: usize = 4096;

const SIG_LOCAL: [u8; 4] = *b"PK\x03\x04";
const SIG_DIR: [u8; 4] = *b"PK\x01\x02";
const SIG_END: [u8; 4] = *b"PK\x05\x06";
const LOCAL_LEN: usize = 30;
const DIR_LEN: usize = 46;
const END_LEN: usize = 22;
/// General purpose flag bits: 0 encrypted, 6 strong encryption, 11 UTF-8 names, 13 masked directory.
const ENCRYPTED: u16 = 1 | 1 << 6 | 1 << 13;
const UTF8: u16 = 1 << 11;
/// Unix mode file types (high half of the external attributes when the archive was made on Unix).
const MODE_TYPE: u32 = 0o170000;
const MODE_REGULAR: u32 = 0o100000;
const MODE_FOLDER: u32 = 0o040000;
/// Info-ZIP records a file it read from a pipe (`cat f | zip - -`) as a FIFO.
const MODE_FIFO: u32 = 0o010000;
/// What the writer records: made on Unix (host 3, so Info-ZIP `unzip` keeps UTF-8 names instead of decoding them as
/// CP437 as it does for DOS) as spec 2.0, and a regular file with mode 644.
const MADE_BY_UNIX: [u8; 2] = [20, 3];
const FILE_ATTR: u32 = (MODE_REGULAR | 0o644) << 16;

/// One file of an archive: a relative path with `/` separators, and its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub data: Vec<u8>,
}

/// Why [`read`] failed: the import shows "Failed to open project archive" for `NotZip` and "Failed to decompress
/// project archive" for `Corrupt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZipError {
    /// No end-of-central-directory record in the last 64 KiB: not a ZIP file (or cut short).
    NotZip,
    /// A ZIP file that is damaged, hostile, over a cap or uses what this codec does not read.
    Corrupt,
}

/// Why [`write`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// Over 4 GiB or 65 534 entries: both need zip64.
    TooLarge,
    /// A name [`read`] would refuse or rewrite: not a normalised relative path with `/` separators, or a folder.
    BadName,
}

impl fmt::Display for ZipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotZip => "not a ZIP archive",
            Self::Corrupt => "the ZIP archive is damaged or unsupported",
        })
    }
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "the archive would be too large for a ZIP without zip64",
            Self::BadName => "an entry name is not a safe relative path",
        })
    }
}

impl std::error::Error for ZipError {}
impl std::error::Error for WriteError {}

fn u16_at(b: &[u8], at: usize) -> Result<u16, ZipError> {
    let field = b.get(at..).and_then(|s| s.first_chunk());
    field
        .map(|&f| u16::from_le_bytes(f))
        .ok_or(ZipError::Corrupt)
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, ZipError> {
    let field = b.get(at..).and_then(|s| s.first_chunk());
    field
        .map(|&f| u32::from_le_bytes(f))
        .ok_or(ZipError::Corrupt)
}

/// The entry's path, or `None` for a folder record (a trailing `/`). `\` reads as `/`; empty and `.` components drop.
/// Components that are only dots and spaces (`..`, `...`, `. .`: Windows trims them to `..`) and drive letters
/// (`C:`, at any depth) are refused, so no result can leave the folder it is extracted into.
pub(crate) fn clean(name: &str) -> Result<Option<String>, ZipError> {
    let name = name.replace('\\', "/");
    if name.is_empty() || name.len() > MAX_NAME || name.starts_with('/') || name.contains('\0') {
        return Err(ZipError::Corrupt);
    }
    let mut parts = Vec::new();
    for part in name.split('/') {
        let bytes = part.as_bytes();
        let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
        match part {
            "" | "." => {}
            _ if drive || part.trim_matches(['.', ' ']).is_empty() => {
                return Err(ZipError::Corrupt);
            }
            _ => parts.push(part),
        }
    }
    match (name.ends_with('/'), parts.is_empty()) {
        (true, _) => Ok(None),
        (false, true) => Err(ZipError::Corrupt),
        (false, false) => Ok(Some(parts.join("/"))),
    }
}

/// The end record: the last signature in the last `22 + 65 535` bytes whose comment fits in the file.
fn find_end(b: &[u8]) -> Option<usize> {
    let last = b.len().checked_sub(END_LEN)?;
    (last.saturating_sub(0xFFFF)..=last).rev().find(|&at| {
        let comment = u16_at(b, at + 20).map(usize::from);
        b[at..].starts_with(&SIG_END) && comment.is_ok_and(|n| at + END_LEN + n <= b.len())
    })
}

/// A central directory record's fields, as read (both readers check them).
struct Header<'a> {
    made_by: u16,
    flags: u16,
    method: u16,
    crc: u32,
    packed_len: u32,
    size: u32,
    disk: u16,
    attr: u32,
    /// The local header's offset.
    at: u32,
    name: &'a [u8],
}

impl Header<'_> {
    /// A symlink, device node or socket (the Unix mode, when the archive was made on Unix): extraction never makes
    /// one, but a reader that did would write through a link. A Unix archive's other entries are files and folders.
    fn special(&self) -> bool {
        let mode = (self.attr >> 16) & MODE_TYPE;
        self.made_by >> 8 == 3 && ![0, MODE_REGULAR, MODE_FOLDER, MODE_FIFO].contains(&mode)
    }
}

/// The central directory record at the start of `rest`, which moves past it.
fn header<'a>(rest: &mut &'a [u8]) -> Result<Header<'a>, ZipError> {
    let h: &'a [u8] = rest;
    if !h.starts_with(&SIG_DIR) {
        return Err(ZipError::Corrupt);
    }
    let (name_len, extra_len, note_len) = (u16_at(h, 28)?, u16_at(h, 30)?, u16_at(h, 32)?);
    let var = usize::from(name_len) + usize::from(extra_len) + usize::from(note_len);
    let name = h
        .get(DIR_LEN..DIR_LEN + usize::from(name_len))
        .ok_or(ZipError::Corrupt)?;
    *rest = h.get(DIR_LEN + var..).ok_or(ZipError::Corrupt)?;
    Ok(Header {
        made_by: u16_at(h, 4)?,
        flags: u16_at(h, 8)?,
        method: u16_at(h, 10)?,
        crc: u32_at(h, 16)?,
        packed_len: u32_at(h, 20)?,
        size: u32_at(h, 24)?,
        disk: u16_at(h, 34)?,
        attr: u32_at(h, 38)?,
        at: u32_at(h, 42)?,
        name,
    })
}

/// A central directory record, checked, with its data located.
struct Record<'a> {
    /// `None`: a folder or the repeat of an earlier name, nothing to extract.
    path: Option<String>,
    method: u16,
    crc: u32,
    size: u32,
    packed: &'a [u8],
    /// The local header's first byte and the data's end, for the overlap check.
    span: (usize, usize),
}

/// The files of the archive `bytes`, in directory order. `max_total` caps the bytes of all files together (the
/// caller's file-size limit); the ratio cap may lower it. A repeated name keeps the first file, folders are left out.
pub fn read(bytes: &[u8], max_total: u64) -> Result<Vec<Entry>, ZipError> {
    let end = find_end(bytes).ok_or(ZipError::NotZip)?;
    let (disk, dir_disk) = (u16_at(bytes, end + 4)?, u16_at(bytes, end + 6)?);
    let (here, total) = (u16_at(bytes, end + 8)?, u16_at(bytes, end + 10)?);
    let (dir_len, dir_at) = (u32_at(bytes, end + 12)?, u32_at(bytes, end + 16)?);
    // Split archives are refused. zip64's markers (0xFFFF entries, 0xFFFFFFFF sizes and offsets, here and in the
    // records) need no check of their own: they fail the entry cap and the bounds checks, since no file is 4 GiB.
    if disk != 0 || dir_disk != 0 || here != total || usize::from(total) > MAX_ENTRIES {
        return Err(ZipError::Corrupt);
    }
    let (dir_at, dir_len) = (dir_at as usize, dir_len as usize);
    let dir = dir_at
        .checked_add(dir_len)
        .filter(|&e| e <= end)
        .and_then(|e| bytes.get(dir_at..e));
    let dir = dir.ok_or(ZipError::Corrupt)?;

    let mut records = Vec::with_capacity(usize::from(total));
    let mut names = HashSet::new();
    let mut rest = dir;
    for _ in 0..total {
        let h = header(&mut rest)?;
        let special = h.special();
        let Header {
            flags,
            method,
            crc,
            packed_len,
            size,
            disk,
            at,
            name,
            ..
        } = h;
        if flags & ENCRYPTED != 0 || disk != 0 || ![0, 8].contains(&method) || special {
            return Err(ZipError::Corrupt);
        }
        let name = match std::str::from_utf8(name) {
            Ok(s) => s.to_owned(),
            Err(_) if flags & UTF8 != 0 => return Err(ZipError::Corrupt),
            // ponytail: Latin-1 for names written without the UTF-8 flag (CP437 or a locale code page in the
            // wild); ASCII is exact and nothing fails. A CP437 table if such archives show up with accents.
            Err(_) => name.iter().map(|&b| char::from(b)).collect(),
        };
        let path = clean(&name)?.filter(|p| names.insert(p.clone()));

        // The local header only locates the data (its own sizes may be zero behind a data descriptor).
        let at = at as usize;
        let local = bytes
            .get(at..)
            .filter(|l| l.starts_with(&SIG_LOCAL) && l.len() >= LOCAL_LEN);
        let local = local.ok_or(ZipError::Corrupt)?;
        let start =
            at + LOCAL_LEN + usize::from(u16_at(local, 26)?) + usize::from(u16_at(local, 28)?);
        let stop = start
            .checked_add(packed_len as usize)
            .filter(|&e| e <= dir_at);
        let stop = stop.ok_or(ZipError::Corrupt)?;
        let packed = &bytes[start..stop];
        records.push(Record {
            path,
            method,
            crc,
            size,
            packed,
            span: (at, stop),
        });
    }

    // An end record that counts fewer entries than the directory holds would silently drop the rest.
    if rest.starts_with(&SIG_DIR) {
        return Err(ZipError::Corrupt);
    }

    // Entries side by side: a record pointing into another's bytes (or the same ones twice) is how a few KiB become
    // gigabytes without a nested archive.
    let mut spans: Vec<_> = records.iter().map(|r| r.span).collect();
    spans.sort_unstable();
    if spans.windows(2).any(|w| w[0].1 > w[1].0) {
        return Err(ZipError::Corrupt);
    }

    let ratio_cap = RATIO_ALLOWANCE.saturating_add(MAX_RATIO.saturating_mul(bytes.len() as u64));
    let mut left = max_total.min(ratio_cap);
    let mut files = Vec::with_capacity(records.len());
    for r in records {
        let Some(path) = r.path else { continue };
        if u64::from(r.size) > left {
            return Err(ZipError::Corrupt);
        }
        // Inflated into exactly the declared size: one byte more or fewer is an error, and the allocation never
        // exceeds what was just checked against the budget.
        let mut data = match r.method {
            0 if r.packed.len() == r.size as usize => r.packed.to_vec(),
            0 => return Err(ZipError::Corrupt),
            _ => decompress_to_vec_with_limit(r.packed, r.size as usize)
                .map_err(|_| ZipError::Corrupt)?,
        };
        if data.len() != r.size as usize || crc32fast::hash(&data) != r.crc {
            return Err(ZipError::Corrupt);
        }
        // Inflating grows the buffer by doubling; without this the kept entries can hold up to twice `max_total`.
        data.shrink_to_fit();
        left -= u64::from(r.size);
        files.push(Entry { path, data });
    }
    Ok(files)
}

/// The central directory may be at most this long for [`open_entry`] (an APK's is well under 1 MiB).
const MAX_STREAMED_DIR: u32 = 16 << 20;

/// `len` bytes of `file` from `at`.
fn read_at(file: &mut (impl Read + Seek), at: u64, len: usize) -> Result<Vec<u8>, ZipError> {
    let mut out = vec![0; len];
    file.seek(SeekFrom::Start(at))
        .and_then(|_| file.read_exact(&mut out))
        .map_err(|_| ZipError::Corrupt)?;
    Ok(out)
}

/// A file of an archive as it streams ([`open_entry`]): a reader of its bytes, and its size.
pub type Streamed<'a> = (Box<dyn Read + 'a>, u64);

/// The file `name` (a path as [`read`] returns it) of the archive `file`, streamed: its bytes and its size, `None`
/// without such a file. [`read`] holds the archive and its files in memory, too much for an APK's
/// OBB of hundreds of MB, so this reads only the end record, the central directory (up to [`MAX_STREAMED_DIR`]) and
/// the file's local header whole. Its data copies (method 0) or inflates (8) as it is read, and a read fails with
/// `InvalidData` or `UnexpectedEof` when the stream is damaged, ends before or goes on past the declared size, or its
/// CRC does not match once read to the end (an empty file's too). The refusals of [`read`] for what it looks at:
/// split archives, encryption, other methods, a symlink or other special file, a stored file whose sizes differ,
/// data that runs into the directory, and a size over the ratio cap. Other records are only compared by name, never
/// refused, and [`MAX_ENTRIES`] does not apply (an APK may hold more files; the directory's size cap bounds the
/// work).
pub fn open_entry<'a>(
    mut file: impl Read + Seek + 'a,
    name: &str,
) -> Result<Option<Streamed<'a>>, ZipError> {
    let (dir, total, dir_at, len) = directory(&mut file)?;
    let mut rest = &dir[..];
    for _ in 0..total {
        let h = header(&mut rest)?;
        let path = std::str::from_utf8(h.name).ok().map(clean);
        if path != Some(Ok(Some(name.to_owned()))) {
            continue;
        }
        let ratio_cap = RATIO_ALLOWANCE.saturating_add(MAX_RATIO.saturating_mul(len));
        if h.flags & ENCRYPTED != 0
            || h.disk != 0
            || h.special()
            || ![0, 8].contains(&h.method)
            || (h.method == 0 && h.packed_len != h.size)
            || u64::from(h.size) > ratio_cap
        {
            return Err(ZipError::Corrupt);
        }
        let local = read_at(&mut file, h.at.into(), LOCAL_LEN)?;
        if !local.starts_with(&SIG_LOCAL) {
            return Err(ZipError::Corrupt);
        }
        let skip =
            LOCAL_LEN as u64 + u64::from(u16_at(&local, 26)?) + u64::from(u16_at(&local, 28)?);
        let start = u64::from(h.at) + skip;
        if start + u64::from(h.packed_len) > dir_at.into() {
            return Err(ZipError::Corrupt);
        }
        file.seek(SeekFrom::Start(start))
            .map_err(|_| ZipError::Corrupt)?;
        let packed = file.take(h.packed_len.into());
        let data: Box<dyn Read + 'a> = match h.method {
            0 => Box::new(packed),
            _ => Box::new(Inflate {
                packed,
                state: InflateState::new_boxed(DataFormat::Raw),
                buf: vec![0; 1 << 16],
                at: 0,
                len: 0,
                done: false,
            }),
        };
        let checked = Checked {
            data,
            left: h.size.into(),
            ended: false,
            crc: h.crc,
            hasher: crc32fast::Hasher::new(),
        };
        return Ok(Some((Box::new(checked), h.size.into())));
    }
    Ok(None)
}

/// The central directory of `file` as [`open_entry`] reads it: its bytes, its record count, where it starts, and the
/// archive's length.
fn directory(file: &mut (impl Read + Seek)) -> Result<(Vec<u8>, u16, u32, u64), ZipError> {
    let len = file.seek(SeekFrom::End(0)).map_err(|_| ZipError::NotZip)?;
    let tail_at = len.saturating_sub((END_LEN + 0xFFFF) as u64);
    let tail = read_at(file, tail_at, (len - tail_at) as usize)?;
    let end = find_end(&tail).ok_or(ZipError::NotZip)?;
    let (disk, dir_disk) = (u16_at(&tail, end + 4)?, u16_at(&tail, end + 6)?);
    let (here, total) = (u16_at(&tail, end + 8)?, u16_at(&tail, end + 10)?);
    let (dir_len, dir_at) = (u32_at(&tail, end + 12)?, u32_at(&tail, end + 16)?);
    let dir_end = u64::from(dir_at) + u64::from(dir_len);
    if disk != 0
        || dir_disk != 0
        || here != total
        || dir_len > MAX_STREAMED_DIR
        || dir_end > tail_at + end as u64
    {
        return Err(ZipError::Corrupt);
    }
    Ok((
        read_at(file, dir_at.into(), dir_len as usize)?,
        total,
        dir_at,
        len,
    ))
}

/// The files of the archive `file`, named as [`open_entry`] takes them, each with its size and CRC-32: the central
/// directory's word, which [`open_entry`] checks as the file streams. Folders and names [`read`] refuses are left out.
pub fn list(mut file: impl Read + Seek) -> Result<Vec<(String, u64, u32)>, ZipError> {
    let (dir, total, ..) = directory(&mut file)?;
    let mut rest = &dir[..];
    let mut files = Vec::new();
    for _ in 0..total {
        let h = header(&mut rest)?;
        if let Some(Ok(Some(path))) = std::str::from_utf8(h.name).ok().map(clean) {
            files.push((path, h.size.into(), h.crc));
        }
    }
    Ok(files)
}

/// A raw deflate stream inflated as it is read (`miniz_oxide`'s streaming inflater).
struct Inflate<R> {
    packed: R,
    state: Box<InflateState>,
    /// Packed bytes read and not yet inflated: `buf[at..len]`.
    buf: Vec<u8>,
    at: usize,
    len: usize,
    done: bool,
}

impl<R: Read> Read for Inflate<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while !self.done && !out.is_empty() {
            if self.at == self.len {
                (self.at, self.len) = (0, self.packed.read(&mut self.buf)?);
            }
            let input = &self.buf[self.at..self.len];
            let step = inflate(&mut self.state, input, out, MZFlush::None);
            self.at += step.bytes_consumed;
            match step.status {
                Ok(MZStatus::StreamEnd) => self.done = true,
                Ok(_) if step.bytes_consumed + step.bytes_written > 0 => {}
                // No input left (`Buf` on an empty input) or no progress: the stream is cut short or damaged.
                Err(MZError::Buf) if input.is_empty() => {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                _ => return Err(io::ErrorKind::InvalidData.into()),
            }
            if step.bytes_written > 0 {
                return Ok(step.bytes_written);
            }
        }
        Ok(0)
    }
}

/// The file's bytes, exactly its declared size: they must all come and then end (a deflate stream at its end), and
/// their CRC is checked once the last one is read.
struct Checked<R> {
    data: R,
    left: u64,
    /// The end was checked.
    ended: bool,
    crc: u32,
    hasher: crc32fast::Hasher,
}

impl<R: Read> Read for Checked<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let want = out
            .len()
            .min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let n = match want {
            0 => 0,
            _ => self.data.read(&mut out[..want])?,
        };
        if n == 0 && want > 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.hasher.update(&out[..n]);
        self.left -= n as u64;
        if self.left == 0 && !std::mem::replace(&mut self.ended, true) {
            let more = self.data.read(&mut [0])?;
            if more > 0 || self.hasher.clone().finalize() != self.crc {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    ZipError::Corrupt,
                ));
            }
        }
        Ok(n)
    }
}

fn put(out: &mut Vec<u8>, parts: &[&[u8]]) {
    for part in parts {
        out.extend_from_slice(part);
    }
}

/// An archive of `entries` (a path with `/` separators and its bytes, see [`WriteError::BadName`]): each file deflated
/// at level 6 unless storing it is smaller, UTF-8 names, made on Unix with mode 644, the fixed time 1980-01-01 00:00 so
/// equal input gives equal bytes, no extra fields or comments. No zip64: more than 65 534 entries or over 4 GiB is
/// [`WriteError::TooLarge`].
pub fn write(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, WriteError> {
    write_limited(entries, u32::MAX.into())
}

/// [`write`] with the archive size limit as a parameter, so the 4 GiB check can be tested without gigabytes.
/// `max_len` must not exceed `u32::MAX`: the offsets in the headers are cast to `u32` after the length checks.
fn write_limited(entries: &[(String, Vec<u8>)], max_len: u64) -> Result<Vec<u8>, WriteError> {
    let input: u64 = entries.iter().map(|(_, data)| data.len() as u64).sum();
    if entries.len() >= usize::from(u16::MAX) || input > max_len {
        return Err(WriteError::TooLarge);
    }
    if entries
        .iter()
        .any(|(name, _)| clean(name).ok().flatten().as_deref() != Some(name))
    {
        return Err(WriteError::BadName);
    }
    let (mut out, mut dir) = (Vec::new(), Vec::new());
    for (name, data) in entries {
        let deflated = compress_to_vec(data, 6);
        let (method, packed): (u16, &[u8]) = if deflated.len() < data.len() {
            (8, &deflated)
        } else {
            (0, data)
        };
        let at = (out.len() as u32).to_le_bytes();
        let (crc, len, size) = (
            crc32fast::hash(data),
            packed.len() as u32,
            data.len() as u32,
        );
        let version = (if method == 8 { 20u16 } else { 10 }).to_le_bytes();
        let (flags, method) = (UTF8.to_le_bytes(), method.to_le_bytes());
        // 1980-01-01 00:00:00 in DOS time and date.
        let stamp = [0, 0, 0x21, 0];
        let (crc, len, size) = (crc.to_le_bytes(), len.to_le_bytes(), size.to_le_bytes());
        let name_len = (name.len() as u16).to_le_bytes();
        put(
            &mut out,
            &[
                &SIG_LOCAL,
                &version,
                &flags,
                &method,
                &stamp,
                &crc,
                &len,
                &size,
                &name_len,
                &[0, 0],
            ],
        );
        put(&mut out, &[name.as_bytes(), packed]);
        put(
            &mut dir,
            &[
                &SIG_DIR,
                &MADE_BY_UNIX,
                &version,
                &flags,
                &method,
                &stamp,
                &crc,
                &len,
                &size,
                &name_len,
            ],
        );
        // Extra and comment lengths, disk number and internal attributes (8 zero bytes), then the external attributes.
        put(
            &mut dir,
            &[&[0; 8], &FILE_ATTR.to_le_bytes(), &at, name.as_bytes()],
        );
        if out.len() as u64 > max_len {
            return Err(WriteError::TooLarge);
        }
    }
    if (out.len() + dir.len() + END_LEN) as u64 > max_len {
        return Err(WriteError::TooLarge);
    }
    let (count, dir_len, dir_at) = (
        (entries.len() as u16).to_le_bytes(),
        dir.len() as u32,
        out.len() as u32,
    );
    out.extend_from_slice(&dir);
    put(
        &mut out,
        &[
            &SIG_END,
            &[0; 4],
            &count,
            &count,
            &dir_len.to_le_bytes(),
            &dir_at.to_le_bytes(),
            &[0, 0],
        ],
    );
    Ok(out)
}

#[cfg(test)]
mod tests;

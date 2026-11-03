use std::collections::BTreeMap;
use std::fmt;

pub const MAX_FILE_SIZE: usize = 512 * 1024 * 1024;
pub const MAX_CHUNK_SIZE: usize = 256 * 1024 * 1024;

pub const MAGIC_FL01: [u8; 4] = *b"10LF";
pub const MAGIC_RK01: [u8; 4] = *b"10KR";
pub const MAGIC_CH02: [u8; 4] = *b"20HC";
pub const MAGIC_CH01: [u8; 4] = *b"10HC";
pub const MAGIC_CL02: [u8; 4] = *b"20LC";
pub const MAGIC_CL01: [u8; 4] = *b"10LC";
pub const MAGIC_SL02: [u8; 4] = *b"20LS";
pub const MAGIC_SL01: [u8; 4] = *b"10LS";
pub const MAGIC_SM01: [u8; 4] = *b"10MS";
pub const MAGIC_DW01: [u8; 4] = *b"10WD";
/// A drum kit template, `.tmpl`: TMPL.
pub const MAGIC_TMPL: [u8; 4] = *b"LPMT";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlmErrorKind {
    UnexpectedEof,
    TooLarge,
    InvalidLength,
    InvalidChunkId,
    InvalidMagic,
    InvalidContentPath,
    TooShort,
    MissingChunk,
    BadValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlmError {
    pub path: String,
    pub kind: FlmErrorKind,
}

impl FlmError {
    pub fn new(path: impl Into<String>, kind: FlmErrorKind) -> Self {
        Self {
            path: path.into(),
            kind,
        }
    }
}

impl fmt::Display for FlmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {:?}", self.path, self.kind)
    }
}

impl std::error::Error for FlmError {}

#[derive(Clone, Debug)]
pub struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
    path: String,
}

impl<'a> Cursor<'a> {
    pub fn new(bytes: &'a [u8], path: impl Into<String>) -> Self {
        Self {
            bytes,
            position: 0,
            path: path.into(),
        }
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    pub fn take(&mut self, length: usize) -> Result<&'a [u8], FlmError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| self.error(FlmErrorKind::TooLarge))?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| self.error(FlmErrorKind::UnexpectedEof))?;
        self.position = end;
        Ok(bytes)
    }

    pub fn skip(&mut self, length: usize) -> Result<(), FlmError> {
        self.take(length).map(|_| ())
    }

    pub fn rest(&mut self) -> &'a [u8] {
        let bytes = &self.bytes[self.position..];
        self.position = self.bytes.len();
        bytes
    }

    pub fn u8(&mut self) -> Result<u8, FlmError> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, FlmError> {
        let mut bytes = [0; 2];
        bytes.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(bytes))
    }

    pub fn u32(&mut self) -> Result<u32, FlmError> {
        let mut bytes = [0; 4];
        bytes.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn i16(&mut self) -> Result<i16, FlmError> {
        let mut bytes = [0; 2];
        bytes.copy_from_slice(self.take(2)?);
        Ok(i16::from_le_bytes(bytes))
    }

    pub fn i32(&mut self) -> Result<i32, FlmError> {
        let mut bytes = [0; 4];
        bytes.copy_from_slice(self.take(4)?);
        Ok(i32::from_le_bytes(bytes))
    }

    pub fn u64(&mut self) -> Result<u64, FlmError> {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(bytes))
    }

    /// A value that is not finite reads as 0.0: a NaN in a song would never equal itself, so an unedited song would
    /// look edited, and an infinite length, tempo or position has no meaning anywhere it goes.
    pub fn f32(&mut self) -> Result<f32, FlmError> {
        let value = f32::from_bits(self.u32()?);
        Ok(if value.is_finite() { value } else { 0.0 })
    }

    /// A value that is not finite reads as 0.0, as for [`Self::f32`].
    pub fn f64(&mut self) -> Result<f64, FlmError> {
        let value = f64::from_bits(self.u64()?);
        Ok(if value.is_finite() { value } else { 0.0 })
    }

    fn optional<T>(
        &mut self,
        size: usize,
        default: T,
        read: fn(&mut Self) -> Result<T, FlmError>,
    ) -> T {
        if self.remaining() < size {
            self.position = self.bytes.len();
            default
        } else {
            read(self).unwrap_or(default)
        }
    }

    pub fn u8_or(&mut self, default: u8) -> u8 {
        self.optional(1, default, Self::u8)
    }

    pub fn u16_or(&mut self, default: u16) -> u16 {
        self.optional(2, default, Self::u16)
    }

    pub fn u32_or(&mut self, default: u32) -> u32 {
        self.optional(4, default, Self::u32)
    }

    pub fn i16_or(&mut self, default: i16) -> i16 {
        self.optional(2, default, Self::i16)
    }

    pub fn i32_or(&mut self, default: i32) -> i32 {
        self.optional(4, default, Self::i32)
    }

    pub fn u64_or(&mut self, default: u64) -> u64 {
        self.optional(8, default, Self::u64)
    }

    pub fn f32_or(&mut self, default: f32) -> f32 {
        self.optional(4, default, Self::f32)
    }

    pub fn f64_or(&mut self, default: f64) -> f64 {
        self.optional(8, default, Self::f64)
    }

    /// Reads a fixed field; invalid UTF-8 is replaced ([`text`]) and cannot round-trip byte for byte.
    pub fn char<const N: usize>(&mut self) -> Result<String, FlmError> {
        let bytes = self.take(N)?;
        let end = bytes.iter().position(|&byte| byte == 0).unwrap_or(N);
        Ok(text(&bytes[..end]))
    }

    pub fn vstr16(&mut self) -> Result<&'a [u8], FlmError> {
        let length = usize::from(self.u16()?);
        self.take(length)
    }

    pub fn vstr32(&mut self) -> Result<&'a [u8], FlmError> {
        let length =
            usize::try_from(self.u32()?).map_err(|_| self.error(FlmErrorKind::TooLarge))?;
        if length > MAX_CHUNK_SIZE {
            return Err(self.error(FlmErrorKind::TooLarge));
        }
        self.take(length)
    }

    pub fn content_path<const N: usize>(&mut self) -> Result<ContentPath, FlmError> {
        Ok(ContentPath::parse(self.take(N)?))
    }

    pub fn content_path16(&mut self) -> Result<ContentPath, FlmError> {
        Ok(ContentPath::parse(self.vstr16()?))
    }

    pub fn content_path32(&mut self) -> Result<ContentPath, FlmError> {
        Ok(ContentPath::parse(self.vstr32()?))
    }

    pub fn expect_magic(&mut self, magic: [u8; 4]) -> Result<(), FlmError> {
        if self.take(4)? == magic {
            Ok(())
        } else {
            Err(self.error(FlmErrorKind::InvalidMagic))
        }
    }

    fn error(&self, kind: FlmErrorKind) -> FlmError {
        FlmError::new(self.path.clone(), kind)
    }
}

/// `bytes` as text, each invalid UTF-8 sequence read as `?`: never longer than `bytes`, so a string read from a field
/// always fits back into it.
pub fn text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push('?');
        }
    }
    text
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContentRoot {
    App,
    User,
    Temp,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ContentPath {
    Shorthand { root: ContentRoot, relative: String },
    Plain(String),
}

impl ContentPath {
    fn parse(bytes: &[u8]) -> Self {
        let shorthand = match bytes.get(..4) {
            Some(b"@R@\0") => Some(ContentRoot::App),
            Some(b"@L@\0") => Some(ContentRoot::User),
            Some(b"@T@\0") => Some(ContentRoot::Temp),
            _ => None,
        };
        let path = if shorthand.is_some() {
            &bytes[4..]
        } else {
            bytes
        };
        let end = path
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(path.len());
        let path = text(&path[..end]).replace('\\', "/");
        match shorthand {
            Some(root) => Self::Shorthand {
                root,
                relative: path,
            },
            None => Self::Plain(path),
        }
    }

    fn encoded(&self, limit: usize) -> Result<Vec<u8>, FlmErrorKind> {
        let (prefix, path) = match self {
            Self::Shorthand {
                root: ContentRoot::App,
                relative,
            } => (&b"@R@\0"[..], relative),
            Self::Shorthand {
                root: ContentRoot::User,
                relative,
            } => (&b"@L@\0"[..], relative),
            Self::Shorthand {
                root: ContentRoot::Temp,
                ..
            } => {
                return Err(FlmErrorKind::InvalidContentPath);
            }
            Self::Plain(path) => (&b""[..], path),
        };
        if path.as_bytes().contains(&0) {
            return Err(FlmErrorKind::InvalidContentPath);
        }
        let length = prefix
            .len()
            .checked_add(path.len())
            .ok_or(FlmErrorKind::TooLarge)?;
        if length > MAX_CHUNK_SIZE {
            return Err(FlmErrorKind::TooLarge);
        }
        if length > limit {
            return Err(FlmErrorKind::InvalidLength);
        }
        let path = path.replace('\\', "/");
        let mut bytes = Vec::with_capacity(length);
        bytes.extend_from_slice(prefix);
        bytes.extend_from_slice(path.as_bytes());
        Ok(bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefixKind {
    Magic,
    U32Magic,
    MagicU32,
    TwoU32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prefix {
    Magic([u8; 4]),
    U32Magic(u32, [u8; 4]),
    MagicU32([u8; 4], u32),
    TwoU32(u32, u32),
}

#[derive(Clone, Debug)]
pub struct Chunk<'a> {
    pub id: [u8; 4],
    pub payload: &'a [u8],
    pub path: String,
}

impl<'a> Chunk<'a> {
    pub fn reader(&self) -> Cursor<'a> {
        Cursor::new(self.payload, self.path.clone())
    }

    pub fn fixed<const N: usize>(&self) -> ([u8; N], &'a [u8]) {
        self.fixed_or([0; N])
    }

    pub fn fixed_or<const N: usize>(&self, mut fixed: [u8; N]) -> ([u8; N], &'a [u8]) {
        let length = self.payload.len().min(N);
        fixed[..length].copy_from_slice(&self.payload[..length]);
        (fixed, self.payload.get(N..).unwrap_or_default())
    }

    pub fn children(&self) -> ChunkIter<'a> {
        ChunkIter::new(self.payload, self.path.clone())
    }

    pub fn container(&self, kind: PrefixKind) -> Result<(Prefix, ChunkIter<'a>), FlmError> {
        let mut reader = self.reader();
        let mut magic = [0; 4];
        let prefix = match kind {
            PrefixKind::Magic => {
                magic.copy_from_slice(reader.take(4)?);
                Prefix::Magic(magic)
            }
            PrefixKind::U32Magic => {
                let value = reader.u32()?;
                magic.copy_from_slice(reader.take(4)?);
                Prefix::U32Magic(value, magic)
            }
            PrefixKind::MagicU32 => {
                magic.copy_from_slice(reader.take(4)?);
                Prefix::MagicU32(magic, reader.u32()?)
            }
            PrefixKind::TwoU32 => Prefix::TwoU32(reader.u32()?, reader.u32()?),
        };
        Ok((
            prefix,
            ChunkIter::new(
                &self.payload[self.payload.len() - reader.remaining()..],
                self.path.clone(),
            ),
        ))
    }

    pub fn container_expect(
        &self,
        kind: PrefixKind,
        magics: &[[u8; 4]],
    ) -> Result<(Prefix, ChunkIter<'a>), FlmError> {
        let (prefix, chunks) = self.container(kind)?;
        let magic = match prefix {
            Prefix::Magic(magic) | Prefix::U32Magic(_, magic) | Prefix::MagicU32(magic, _) => magic,
            Prefix::TwoU32(_, _) => return Err(FlmError::new(&self.path, FlmErrorKind::BadValue)),
        };
        if magics.contains(&magic) {
            Ok((prefix, chunks))
        } else {
            Err(FlmError::new(&self.path, FlmErrorKind::InvalidMagic))
        }
    }
}

#[derive(Debug)]
pub struct ChunkIter<'a> {
    reader: Cursor<'a>,
    occurrences: BTreeMap<[u8; 4], usize>,
    finished: bool,
}

impl<'a> ChunkIter<'a> {
    /// Chunk indexes in paths (`CLIP[3]`) are counted for this many distinct ids per list; later new ids show
    /// `[?]`, so a hostile list of distinct ids cannot grow the count map.
    const COUNTED_IDS: usize = 64;

    pub fn new(bytes: &'a [u8], path: impl Into<String>) -> Self {
        Self {
            reader: Cursor::new(bytes, path),
            occurrences: BTreeMap::new(),
            finished: false,
        }
    }

    /// The bytes not read as chunks (after the iteration: the tail too short for a chunk header, or what follows
    /// an error).
    pub fn remainder(&self) -> &'a [u8] {
        &self.reader.bytes[self.reader.position..]
    }
}

impl<'a> Iterator for ChunkIter<'a> {
    type Item = Result<Chunk<'a>, FlmError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished || self.reader.remaining() < 8 {
            return None;
        }
        let result = (|| {
            let mut id = [0; 4];
            id.copy_from_slice(self.reader.take(4)?);
            let counted =
                self.occurrences.len() < Self::COUNTED_IDS || self.occurrences.contains_key(&id);
            let count = counted.then(|| self.occurrences.entry(id).or_default());
            let label = if id.iter().all(|byte| (0x20..=0x7e).contains(byte)) {
                String::from_utf8_lossy(&id).into_owned()
            } else {
                format!("0x{:08x}", u32::from_be_bytes(id))
            };
            let index = match count {
                Some(count) => {
                    *count += 1;
                    (*count - 1).to_string()
                }
                None => "?".to_owned(),
            };
            let path = if self.reader.path.is_empty() {
                format!("{label}[{index}]")
            } else {
                format!("{}/{label}[{index}]", self.reader.path)
            };
            let declared_size = self
                .reader
                .u32()
                .map_err(|error| FlmError::new(path.clone(), error.kind))?
                as usize;
            let size = declared_size.min(self.reader.remaining());
            if size > MAX_CHUNK_SIZE {
                return Err(FlmError::new(path, FlmErrorKind::TooLarge));
            }
            let payload = self
                .reader
                .take(size)
                .map_err(|error| FlmError::new(path.clone(), error.kind))?;
            Ok(Chunk { id, payload, path })
        })();
        if result.is_err() {
            self.finished = true;
        }
        Some(result)
    }
}

pub fn file_chunks(bytes: &[u8]) -> Result<ChunkIter<'_>, FlmError> {
    if bytes.len() > MAX_FILE_SIZE {
        return Err(FlmError::new("FLM", FlmErrorKind::TooLarge));
    }
    let mut reader = Cursor::new(bytes, "FLM");
    reader.expect_magic(MAGIC_FL01)?;
    Ok(ChunkIter::new(&bytes[4..], ""))
}

#[derive(Debug, Default)]
pub struct ChunkWriter {
    bytes: Vec<u8>,
    open_chunks: Vec<usize>,
    path: Vec<String>,
    occurrences: Vec<BTreeMap<[u8; 4], usize>>,
}

impl ChunkWriter {
    pub fn new_file() -> Self {
        Self {
            bytes: MAGIC_FL01.to_vec(),
            ..Self::default()
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn raw(&mut self, bytes: &[u8]) -> Result<(), FlmError> {
        self.check_additional(bytes.len())?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    pub fn u8(&mut self, value: u8) -> Result<(), FlmError> {
        self.raw(&[value])
    }

    pub fn u16(&mut self, value: u16) -> Result<(), FlmError> {
        self.raw(&value.to_le_bytes())
    }

    pub fn u32(&mut self, value: u32) -> Result<(), FlmError> {
        self.raw(&value.to_le_bytes())
    }

    pub fn i16(&mut self, value: i16) -> Result<(), FlmError> {
        self.raw(&value.to_le_bytes())
    }

    pub fn i32(&mut self, value: i32) -> Result<(), FlmError> {
        self.raw(&value.to_le_bytes())
    }

    pub fn u64(&mut self, value: u64) -> Result<(), FlmError> {
        self.raw(&value.to_le_bytes())
    }

    pub fn f32(&mut self, value: f32) -> Result<(), FlmError> {
        self.u32(value.to_bits())
    }

    pub fn f64(&mut self, value: f64) -> Result<(), FlmError> {
        self.u64(value.to_bits())
    }

    pub fn char<const N: usize>(&mut self, value: &str) -> Result<(), FlmError> {
        let mut length = value.len().min(N);
        while !value.is_char_boundary(length) {
            length -= 1;
        }
        let mut field = [0; N];
        field[..length].copy_from_slice(&value.as_bytes()[..length]);
        self.raw(&field)
    }

    pub fn vstr16(&mut self, value: &[u8]) -> Result<(), FlmError> {
        let length =
            u16::try_from(value.len()).map_err(|_| self.error(FlmErrorKind::InvalidLength))?;
        self.check_additional(2 + value.len())?;
        self.bytes.extend_from_slice(&length.to_le_bytes());
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    pub fn vstr32(&mut self, value: &[u8]) -> Result<(), FlmError> {
        let length =
            u32::try_from(value.len()).map_err(|_| self.error(FlmErrorKind::InvalidLength))?;
        let additional = value
            .len()
            .checked_add(4)
            .ok_or_else(|| self.error(FlmErrorKind::TooLarge))?;
        self.check_additional(additional)?;
        self.bytes.extend_from_slice(&length.to_le_bytes());
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    pub fn content_path<const N: usize>(&mut self, path: &ContentPath) -> Result<(), FlmError> {
        let encoded = path.encoded(N).map_err(|kind| self.error(kind))?;
        let mut field = [0; N];
        field[..encoded.len()].copy_from_slice(&encoded);
        self.raw(&field)
    }

    pub fn content_path16(&mut self, path: &ContentPath) -> Result<(), FlmError> {
        let encoded = path
            .encoded(u16::MAX as usize)
            .map_err(|kind| self.error(kind))?;
        self.vstr16(&encoded)
    }

    pub fn content_path32(&mut self, path: &ContentPath) -> Result<(), FlmError> {
        let encoded = path
            .encoded(MAX_CHUNK_SIZE)
            .map_err(|kind| self.error(kind))?;
        self.vstr32(&encoded)
    }

    pub fn chunk(
        &mut self,
        id: [u8; 4],
        write_payload: impl FnOnce(&mut Self) -> Result<(), FlmError>,
    ) -> Result<(), FlmError> {
        if !id.iter().all(|byte| (0x20..=0x7e).contains(byte)) {
            return Err(self.error(FlmErrorKind::InvalidChunkId));
        }
        self.check_additional(8)?;
        let header = self.bytes.len();
        self.bytes.extend_from_slice(&id);
        self.bytes.extend_from_slice(&[0; 4]);
        let payload_start = self.bytes.len();
        if self.occurrences.is_empty() {
            self.occurrences.push(BTreeMap::new());
        }
        let count = self.occurrences.last_mut().unwrap().entry(id).or_default();
        let index = *count;
        *count += 1;
        self.open_chunks.push(payload_start);
        self.path
            .push(format!("{}[{index}]", String::from_utf8_lossy(&id)));
        self.occurrences.push(BTreeMap::new());
        let result = write_payload(self);
        self.occurrences.pop();
        self.path.pop();
        self.open_chunks.pop();
        if let Err(error) = result {
            self.bytes.truncate(header);
            *self.occurrences.last_mut().unwrap().get_mut(&id).unwrap() -= 1;
            return Err(error);
        }
        let size = self.bytes.len() - payload_start;
        self.bytes[header + 4..header + 8].copy_from_slice(&(size as u32).to_le_bytes());
        Ok(())
    }

    fn check_additional(&self, additional: usize) -> Result<(), FlmError> {
        let end = self
            .bytes
            .len()
            .checked_add(additional)
            .ok_or_else(|| self.error(FlmErrorKind::TooLarge))?;
        if end > MAX_FILE_SIZE
            || self
                .open_chunks
                .iter()
                .any(|start| end - start > MAX_CHUNK_SIZE)
        {
            return Err(self.error(FlmErrorKind::TooLarge));
        }
        Ok(())
    }

    fn error(&self, kind: FlmErrorKind) -> FlmError {
        FlmError::new(
            if self.path.is_empty() {
                "FLM".into()
            } else {
                self.path.join("/")
            },
            kind,
        )
    }
}

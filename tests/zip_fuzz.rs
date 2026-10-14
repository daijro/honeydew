//! The ZIP reader on hostile input: structure-aware mutations of real archives never panic, every accepted
//! archive is safe to extract and writes back to the same files, and memory stays within the reader's budget.
//! `ZIP_FUZZ_ITERS` sets the number of mutated archives (default 60 000), `ZIP_FUZZ_SEED` changes the mutations.
//!
//! The mutations know the format: they overwrite single header fields with the values that trip parsers (0, 1, the
//! field's maximum, the file length and its neighbours, zip64 markers), swap methods, forge Unix modes, plant `..`,
//! `\`, drive letters and NULs into names, duplicate and delete directory records, move the directory, add archive
//! comments with fake end records, and cut, flip, insert and delete raw bytes.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};

use honeydew::zip::{self, Entry, MAX_ENTRIES, MAX_RATIO, RATIO_ALLOWANCE, ZipError};
use miniz_oxide::deflate::compress_to_vec;

/// Counts the bytes allocated now and the peak since [`read_peak`] reset it.
struct Counting;

static NOW: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

fn grow(by: isize) {
    let now = NOW.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grow(layout.size() as isize);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        grow(layout.size() as isize);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        grow(-(layout.size() as isize));
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        grow(new_size as isize - layout.size() as isize);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The tests measure a global counter, so they run one at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn exclusive() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner())
}

/// `read(bytes, max)` and the most it allocated on top of what was allocated before it.
fn read_peak(bytes: &[u8], max: u64) -> (Result<Vec<Entry>, ZipError>, usize) {
    let before = NOW.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let result = zip::read(bytes, max);
    (
        result,
        (PEAK.load(Ordering::Relaxed) - before).max(0) as usize,
    )
}

/// What a read of `len` bytes with the budget `max` may allocate. The output is at most the budget (the ratio cap
/// included) and, whatever the headers claim, at most what deflate can expand `len` bytes to (about 1032:1). It is
/// held once as a kept file and once more while inflating (the buffer doubles), plus records, names and sets.
fn memory_bound(len: usize, max: u64) -> usize {
    let cap = max.min(RATIO_ALLOWANCE + MAX_RATIO * len as u64);
    let reachable = 1100 * len as u64 + 4096;
    3 * cap.min(reachable) as usize + 40 * len + (1 << 20)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn seed() -> u64 {
    let seed = std::env::var("ZIP_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok());
    seed.unwrap_or(0x9e37_79b9_7f4a_7c15)
}

fn le16(b: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes([b[at], b[at + 1]]))
}

fn le32(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
}

/// Where the parts of an archive written by [`zip::write`] (no comment) are.
struct Shape {
    end: usize,
    /// The first byte of each central directory record, and of the local header it points at.
    dir: Vec<usize>,
    local: Vec<usize>,
}

fn shape(b: &[u8]) -> Shape {
    let end = b.len() - 22;
    let (n, mut at) = (le16(b, end + 10), le32(b, end + 16));
    let (mut dir, mut local) = (Vec::new(), Vec::new());
    for _ in 0..n {
        dir.push(at);
        local.push(le32(b, at + 42));
        at += 46 + le16(b, at + 28);
    }
    Shape { end, dir, local }
}

/// (offset, width) of the fields of a local header, a directory record and the end record.
const LOCAL_FIELDS: [(usize, usize); 11] = [
    (0, 4),
    (4, 2),
    (6, 2),
    (8, 2),
    (10, 2),
    (12, 2),
    (14, 4),
    (18, 4),
    (22, 4),
    (26, 2),
    (28, 2),
];
const DIR_FIELDS: [(usize, usize); 17] = [
    (0, 4),
    (4, 2),
    (6, 2),
    (8, 2),
    (10, 2),
    (12, 2),
    (14, 2),
    (16, 4),
    (20, 4),
    (24, 4),
    (28, 2),
    (30, 2),
    (32, 2),
    (34, 2),
    (36, 2),
    (38, 4),
    (42, 4),
];
const END_FIELDS: [(usize, usize); 8] = [
    (0, 4),
    (4, 2),
    (6, 2),
    (8, 2),
    (10, 2),
    (12, 4),
    (16, 4),
    (20, 2),
];

const FRAGMENTS: [&[u8]; 12] = [
    b"../", b"..\\", b"/", b"\\", b"C:", b"\0", b"./", b"a/../..", b"//", b"...", b" ", b"/..",
];

fn set(b: &mut [u8], at: usize, width: usize, value: u64) {
    if let Some(field) = b.get_mut(at..at + width) {
        field.copy_from_slice(&value.to_le_bytes()[..width]);
    }
}

/// A value that trips a parser, for a field of `width` bytes currently holding `old` in a file of `len` bytes.
fn nasty(rng: &mut Rng, width: usize, old: u64, len: usize) -> u64 {
    let max = if width == 2 { 0xFFFF } else { 0xFFFF_FFFF };
    let len = len as u64;
    let v = match rng.below(16) {
        0 => 0,
        1 => 1,
        2 => max,
        3 => max - 1,
        4 => old.wrapping_add(1),
        5 => old.wrapping_sub(1),
        6 => len,
        7 => len + 1,
        8 => len.wrapping_sub(1),
        9 => max / 2,
        10 => max / 2 + 1,
        11 => 0x0800,
        12 => 1 << rng.below(width * 8),
        13 => old ^ (1 << rng.below(width * 8)),
        14 => old.wrapping_mul(2),
        _ => rng.next(),
    };
    v & max
}

/// A random edit that keeps the archive's shape (the header fields and names keep their positions).
fn mutate_fields(rng: &mut Rng, b: &mut Vec<u8>, s: &Shape) {
    match rng.below(10) {
        0..=3 => {
            let (base, fields): (usize, &[(usize, usize)]) = match rng.below(5) {
                0 => (s.local[rng.below(s.local.len())], &LOCAL_FIELDS),
                1 => (s.end, &END_FIELDS),
                _ => (s.dir[rng.below(s.dir.len())], &DIR_FIELDS),
            };
            let (at, width) = *rng.pick(fields);
            let old = if base + at + width <= b.len() {
                u64::from_le_bytes({
                    let mut v = [0u8; 8];
                    v[..width].copy_from_slice(&b[base + at..base + at + width]);
                    v
                })
            } else {
                0
            };
            let value = nasty(rng, width, old, b.len());
            set(b, base + at, width, value);
        }
        // A hostile fragment in a name, in the directory and often in the local header too.
        4 | 5 => {
            let i = rng.below(s.dir.len());
            let fragment = *rng.pick(&FRAGMENTS);
            for base in [s.dir[i] + 46, s.local[i] + 30] {
                if base == s.local[i] + 30 && rng.below(3) == 0 {
                    continue;
                }
                let name_len = le16(b, s.dir[i] + 28);
                let room = name_len.saturating_sub(fragment.len());
                let at = base + rng.below(room + 1);
                if let Some(field) = b.get_mut(at..at + fragment.len().min(name_len)) {
                    let n = field.len();
                    field.copy_from_slice(&fragment[..n]);
                }
            }
        }
        // The method of an entry, in both headers.
        6 => {
            let i = rng.below(s.dir.len());
            let method = *rng.pick(&[0u64, 8, 8, 1, 9, 12, 14, 93, 99]);
            set(b, s.dir[i] + 10, 2, method);
            set(b, s.local[i] + 8, 2, method);
        }
        // Made on Unix with a forged mode (symlinks and device nodes among them) or on DOS.
        7 => {
            let i = rng.below(s.dir.len());
            let mode = *rng.pick(&[
                0o120777u64,
                0o100644,
                0o040755,
                0o010644,
                0o140755,
                0o060660,
                0o020660,
                0,
                0o177777,
            ]);
            let host = *rng.pick(&[0x031eu64, 0x0314, 0x0014, 0x0a14, 0x0300]);
            set(b, s.dir[i] + 4, 2, host);
            set(
                b,
                s.dir[i] + 38,
                4,
                (mode << 16) | ((rng.next() & 0xFFFF) * (rng.below(2) as u64)),
            );
        }
        // Point one entry at another's bytes (overlap), or the directory at the wrong place.
        8 => {
            let (i, j) = (rng.below(s.dir.len()), rng.below(s.dir.len()));
            set(b, s.dir[i] + 42, 4, s.local[j] as u64);
        }
        // Archive comment: bytes after the end record, with or without a fake end record in them.
        _ => {
            let n = rng.below(80);
            let mut comment: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
            if rng.below(2) == 0 {
                let at = rng.below(n + 1);
                comment.splice(at..at, *b"PK\x05\x06");
                let fake: Vec<u8> = (0..18).map(|_| rng.next() as u8).collect();
                comment.splice(at + 4..at + 4, fake);
            }
            if rng.below(3) != 0 {
                set(b, s.end + 20, 2, comment.len() as u64);
            }
            b.extend_from_slice(&comment);
        }
    }
}

/// A random edit that changes the archive's size or shape.
fn mutate_bytes(rng: &mut Rng, b: &mut Vec<u8>, s: &Shape) {
    let len = b.len().max(1);
    match rng.below(9) {
        0 => {
            let at = rng.below(len);
            if let Some(x) = b.get_mut(at) {
                *x ^= 1 << rng.below(8);
            }
        }
        1 => {
            let at = rng.below(len);
            if let Some(x) = b.get_mut(at) {
                *x = rng.next() as u8;
            }
        }
        2 => b.truncate(rng.below(len)),
        3 => {
            let at = rng.below(len);
            let end = (at + 1 + rng.below(40)).min(b.len());
            b.drain(at.min(end)..end);
        }
        4 => {
            let at = rng.below(len).min(b.len());
            let insert: Vec<u8> = (0..1 + rng.below(12)).map(|_| rng.next() as u8).collect();
            b.splice(at..at, insert);
        }
        // Repeat a directory record (a duplicate name, a second pointer at the same bytes).
        5 | 6 => {
            let i = rng.below(s.dir.len());
            let end = s.dir.get(i + 1).copied().unwrap_or(s.end);
            let record = b[s.dir[i]..end].to_vec();
            b.splice(end..end, record.iter().copied());
            let eocd = s.end + record.len();
            if rng.below(4) != 0 {
                let count = (le16(b, eocd + 10) + 1) as u64;
                set(b, eocd + 8, 2, count);
                set(b, eocd + 10, 2, count);
                let size = (le32(b, eocd + 12) + record.len()) as u64;
                set(b, eocd + 12, 4, size);
            }
        }
        // Drop a directory record, keeping the counts right: the entry is simply not listed.
        7 => {
            let i = rng.below(s.dir.len());
            let end = s.dir.get(i + 1).copied().unwrap_or(s.end);
            let removed = end - s.dir[i];
            b.drain(s.dir[i]..end);
            let eocd = s.end - removed;
            let count = le16(b, eocd + 10).saturating_sub(1) as u64;
            set(b, eocd + 8, 2, count);
            set(b, eocd + 10, 2, count);
            let size = le32(b, eocd + 12).saturating_sub(removed) as u64;
            set(b, eocd + 12, 4, size);
        }
        // Prefix junk, which shifts every offset.
        _ => {
            let junk: Vec<u8> = (0..1 + rng.below(8)).map(|_| rng.next() as u8).collect();
            b.splice(0..0, junk);
        }
    }
}

/// A name that is safe to extract under a folder (the invariant of every accepted path).
fn is_safe(path: &str) -> bool {
    let drive =
        |c: &str| c.len() >= 2 && c.as_bytes()[0].is_ascii_alphabetic() && c.as_bytes()[1] == b':';
    !path.is_empty()
        && path.len() <= 4096
        && !path.starts_with('/')
        && !path.contains(['\\', '\0'])
        && path.split('/').all(|c| {
            !c.is_empty() && c != "." && !c.trim_matches(['.', ' ']).is_empty() && !drive(c)
        })
}

const NAMES: [&str; 10] = [
    "song.flm",
    "Samples/kick.wav",
    "My Songs/x y.flm",
    "d/e/f/g.bin",
    "ü/ñ ✓.wav",
    "readme",
    "a.b.c",
    "Unzipped/Drums/snare 2.wav",
    "x",
    "deep/er/and/deeper/still/file.dat",
];

fn data(rng: &mut Rng) -> Vec<u8> {
    match rng.below(6) {
        0 => Vec::new(),
        1 => vec![rng.next() as u8],
        2 => b"the quick brown fox ".repeat(1 + rng.below(100)),
        3 => vec![0; rng.below(1 << 16)],
        4 => (0..1 + rng.below(2000)).map(|_| rng.next() as u8).collect(),
        _ => (0..1 + rng.below(3000))
            .map(|i| (i % 7) as u8 + b'a')
            .collect(),
    }
}

/// `count` archives of 1…6 files each, written by the codec.
fn bases(rng: &mut Rng, count: usize) -> Vec<Vec<u8>> {
    (0..count)
        .map(|_| {
            let mut names: Vec<&str> = NAMES.to_vec();
            let files: Vec<(String, Vec<u8>)> = (0..1 + rng.below(6))
                .map(|_| {
                    (
                        names.swap_remove(rng.below(names.len())).to_owned(),
                        data(rng),
                    )
                })
                .collect();
            zip::write(&files).unwrap()
        })
        .collect()
}

#[derive(Debug, Default)]
struct Seen {
    rounds: usize,
    accepted: usize,
    not_zip: usize,
    corrupt: usize,
    peak: usize,
}

#[test]
fn mutated_archives_never_panic_stay_in_budget_and_what_reads_writes_back() {
    let _one = exclusive();
    let rounds: usize = std::env::var("ZIP_FUZZ_ITERS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(60_000);
    let mut rng = Rng(seed());
    let bases = bases(&mut rng, 32);
    let budgets = [0u64, 1000, 64 << 10, 1 << 20, 1 << 30, u64::MAX];
    let mut seen = Seen::default();
    for round in 0..rounds {
        let base = rng.pick(&bases);
        let s = shape(base);
        let mut bytes = base.clone();
        for _ in 0..rng.below(4) {
            mutate_fields(&mut rng, &mut bytes, &s);
        }
        // Byte edits after the field edits: they shift positions, `s` describes the unedited base.
        if rng.below(3) == 0 {
            mutate_bytes(&mut rng, &mut bytes, &s);
        }
        let max = *rng.pick(&budgets);
        let (result, peak) = read_peak(&bytes, max);
        seen.rounds += 1;
        seen.peak = seen.peak.max(peak);
        assert!(
            peak <= memory_bound(bytes.len(), max),
            "round {round}: {peak} B for {} B, budget {max}",
            bytes.len()
        );
        match result {
            Err(ZipError::NotZip) => seen.not_zip += 1,
            Err(ZipError::Corrupt) => seen.corrupt += 1,
            Ok(files) => {
                seen.accepted += 1;
                let total: u64 = files.iter().map(|f| f.data.len() as u64).sum();
                assert!(
                    total <= max,
                    "round {round}: {total} B past the budget {max}"
                );
                assert!(files.len() <= MAX_ENTRIES);
                let mut paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
                assert!(
                    paths.iter().all(|p| is_safe(p)),
                    "round {round}: unsafe path in {paths:?}"
                );
                paths.sort_unstable();
                paths.dedup();
                assert_eq!(paths.len(), files.len(), "round {round}: duplicate paths");
                // What was read can be written (names are canonical) and reads back the same.
                if total <= 256 << 10 {
                    let again: Vec<(String, Vec<u8>)> = files
                        .iter()
                        .map(|f| (f.path.clone(), f.data.clone()))
                        .collect();
                    let written = zip::write(&again)
                        .unwrap_or_else(|e| panic!("round {round}: read but not written: {e}"));
                    let back = zip::read(&written, u64::MAX)
                        .unwrap_or_else(|e| panic!("round {round}: written, not read: {e}"));
                    assert_eq!(back, files, "round {round}: read(write(files)) != files");
                }
            }
        }
    }
    println!("{seen:?}");
    // The edits must reach the extraction code, not only the front door.
    assert!(seen.accepted * 50 > seen.rounds, "{seen:?}");
    assert!(seen.corrupt * 5 > seen.rounds, "{seen:?}");
    assert!(seen.not_zip > 0, "{seen:?}");
}

#[test]
fn random_valid_archives_round_trip() {
    let _one = exclusive();
    let mut rng = Rng(seed() ^ 0x5555);
    for round in 0..4000 {
        let files: Vec<(String, Vec<u8>)> = (0..rng.below(9))
            .map(|i| {
                (
                    format!("{}{i}", NAMES[rng.below(NAMES.len())]),
                    data(&mut rng),
                )
            })
            .collect();
        let bytes = zip::write(&files).unwrap();
        let want: Vec<Entry> = files
            .iter()
            .map(|(p, d)| Entry {
                path: p.clone(),
                data: d.clone(),
            })
            .collect();
        assert_eq!(zip::read(&bytes, u64::MAX).unwrap(), want, "round {round}");
        assert_eq!(
            bytes,
            zip::write(&files).unwrap(),
            "round {round}: not deterministic"
        );
    }
}

/// A local entry of a hand-built hostile archive.
#[derive(Clone)]
struct Part {
    method: u16,
    crc: u32,
    packed: Vec<u8>,
}

/// A directory record pointing at `parts[part]` and claiming `size` bytes.
struct Record {
    name: String,
    part: usize,
    size: u32,
}

fn archive(parts: &[Part], records: &[Record]) -> Vec<u8> {
    let (mut out, mut at) = (Vec::new(), Vec::new());
    for p in parts {
        at.push(out.len() as u32);
        out.extend_from_slice(b"PK\x03\x04\x14\0\0\x08");
        out.extend_from_slice(&p.method.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0x21, 0]);
        out.extend_from_slice(&p.crc.to_le_bytes());
        out.extend_from_slice(&(p.packed.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&[1, 0, 0, 0]);
        out.push(b'p');
        out.extend_from_slice(&p.packed);
    }
    let dir_at = out.len();
    for r in records {
        let p = &parts[r.part];
        out.extend_from_slice(b"PK\x01\x02\x14\0\x14\0\0\x08");
        out.extend_from_slice(&p.method.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0x21, 0]);
        out.extend_from_slice(&p.crc.to_le_bytes());
        out.extend_from_slice(&(p.packed.len() as u32).to_le_bytes());
        out.extend_from_slice(&r.size.to_le_bytes());
        out.extend_from_slice(&(r.name.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(&at[r.part].to_le_bytes());
        out.extend_from_slice(r.name.as_bytes());
    }
    let (n, dir_len) = (records.len() as u16, (out.len() - dir_at) as u32);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&[n.to_le_bytes(), n.to_le_bytes()].concat());
    out.extend_from_slice(&dir_len.to_le_bytes());
    out.extend_from_slice(&(dir_at as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

fn zeros_part(len: usize) -> Part {
    let zeros = vec![0u8; len];
    Part {
        method: 8,
        crc: crc32fast::hash(&zeros),
        packed: compress_to_vec(&zeros, 6),
    }
}

/// Hostile shapes with the memory they may take: refused, and without the allocation they ask for.
#[test]
fn bombs_are_refused_without_their_allocation() {
    let _one = exclusive();
    let mib = |n: u32| n << 20;
    let bomb = zeros_part(40 << 20);
    let one_mib = zeros_part(1 << 20);

    let record = |name: &str, part: usize, size: u32| Record {
        name: name.into(),
        part,
        size,
    };
    // Each case with the most it may allocate: nothing like what its headers ask for.
    let cases: Vec<(&str, Vec<u8>, usize)> = vec![
        // 40 MiB from ~40 KB: past the ratio cap even with an unlimited budget.
        (
            "ratio",
            archive(std::slice::from_ref(&bomb), &[record("b", 0, mib(40))]),
            1 << 16,
        ),
        // The headers claim 100 bytes, the stream holds 40 MiB: inflating stops at the claim.
        (
            "lying small",
            archive(std::slice::from_ref(&bomb), &[record("b", 0, 100)]),
            1 << 16,
        ),
        // The headers claim 4 GiB - 2 for a 1 KB stream.
        (
            "lying large",
            archive(
                std::slice::from_ref(&one_mib),
                &[record("b", 0, u32::MAX - 1)],
            ),
            1 << 16,
        ),
        // A stored entry claiming 4 bytes over 8 MB of data is refused before its bytes are copied.
        (
            "lying stored",
            archive(
                &[Part {
                    method: 0,
                    crc: 0,
                    packed: vec![7; 8 << 20],
                }],
                &[record("b", 0, 4)],
            ),
            1 << 16,
        ),
        // 10 000 records over one entry: the overlapping-file bomb (~10 GiB claimed, 141 MiB past the ratio cap).
        (
            "overlap",
            archive(
                std::slice::from_ref(&one_mib),
                &(0..MAX_ENTRIES)
                    .map(|i| record(&format!("n{i}"), 0, mib(1)))
                    .collect::<Vec<_>>(),
            ),
            4 << 20,
        ),
    ];
    for (name, bytes, allowed) in cases {
        let (result, peak) = read_peak(&bytes, u64::MAX);
        println!("{name}: {} B in, {peak} B peak, {result:?}", bytes.len());
        assert_eq!(result.map(|_| ()), Err(ZipError::Corrupt), "{name}");
        assert!(peak <= allowed, "{name}: {peak} B");
    }

    // Distinct entries, each fine on its own: the caller's budget stops them at the 64th MiB.
    let parts: Vec<Part> = (0..2000)
        .map(|_| Part {
            method: 8,
            crc: one_mib.crc,
            packed: one_mib.packed.clone(),
        })
        .collect();
    let records: Vec<Record> = (0..2000)
        .map(|i| Record {
            name: format!("f{i}"),
            part: i,
            size: mib(1),
        })
        .collect();
    let bytes = archive(&parts, &records);
    let (result, peak) = read_peak(&bytes, 64 << 20);
    println!(
        "budget: {} B in, {peak} B peak, {:?}",
        bytes.len(),
        result.as_ref().map(|_| ())
    );
    assert_eq!(result.map(|_| ()), Err(ZipError::Corrupt));
    assert!(peak <= memory_bound(bytes.len(), 64 << 20), "{peak}");
    // And the same archive within a budget that fits reads, every file exactly 1 MiB.
    let few = archive(&parts[..8], &records[..8]);
    let (result, peak) = read_peak(&few, 8 << 20);
    assert_eq!(
        result
            .unwrap()
            .iter()
            .map(|f| f.data.len())
            .collect::<Vec<_>>(),
        [1 << 20; 8]
    );
    assert!(peak <= memory_bound(few.len(), 8 << 20), "{peak}");
}

#[test]
fn long_names_cost_linear_time_and_memory() {
    let _one = exclusive();
    // Records with 4000-byte names, 8 MB at 2000. One local entry for all of them is an overlap, but every name is read
    // before the archive is refused: the time and memory are linear in the input, the debug build is slow per byte.
    let long = "a/".repeat(2000);
    let parts = [Part {
        method: 0,
        crc: 0,
        packed: Vec::new(),
    }];
    let build = |n: usize| {
        let records: Vec<Record> = (0..n)
            .map(|i| Record {
                name: format!("{long}{i}"),
                part: 0,
                size: 0,
            })
            .collect();
        archive(&parts, &records)
    };
    // One refused read: the time it took, with the memory bound (linear in the input) checked on the way.
    let refused = |bytes: &[u8]| {
        let t = std::time::Instant::now();
        let (result, peak) = read_peak(bytes, u64::MAX);
        let took = t.elapsed();
        assert_eq!(result.map(|_| ()), Err(ZipError::Corrupt));
        assert!(
            peak <= 4 * bytes.len() + (1 << 20),
            "{peak} B for {} B",
            bytes.len()
        );
        took
    };
    // The time is judged by how it grows, not by the clock, which a loaded machine stretches without limit (it once
    // took 17 s against a 10 s limit): 4 times the records should take about 4 times as long (a quadratic reader, 16).
    // A busy machine inflates single reads, so each size is read again, keeping its fastest, until the growth is fine.
    // The bar sits between linear (4) and quadratic (16) with room for a loaded machine's noise (it once read 8.x).
    let (small, big) = (build(500), build(2000));
    let (mut a, mut b) = (std::time::Duration::MAX, std::time::Duration::MAX);
    for _ in 0..10 {
        a = a.min(refused(&small));
        b = b.min(refused(&big));
        if b < 12 * a {
            return;
        }
    }
    panic!(
        "4x the records took {:.1}x as long (fastest of 10 reads each): {a:?}, {b:?}",
        b.as_secs_f64() / a.as_secs_f64()
    );
}

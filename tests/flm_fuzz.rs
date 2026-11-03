//! The reader on hostile input: mutated real and synthetic files never
//! panic, every song read can be written and reads back equal, and memory stays within the reader's budget.
//! `FLM_FUZZ_ITERS` sets the number of mutated buffers (default 6000); with it set, a local
//! `~/Downloads/sample.flm` joins the corpus when present (never copied into the repo). `FLM_FUZZ_SEED` changes the
//! mutations.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use honeydew::flm::chunk::ChunkWriter;
use honeydew::flm::{read, write};
use honeydew::song::{ChannelKind, Song, SynthKind};

mod common;

/// Counts the bytes allocated now and the peak since [`reset_peak`].
struct Counting;

static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now = NOW.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(now, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        NOW.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() {
            let grow = new_size - layout.size();
            let now = NOW.fetch_add(grow, Ordering::Relaxed) + grow;
            PEAK.fetch_max(now, Ordering::Relaxed);
        } else {
            NOW.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The most `read(bytes)` allocates on top of what was allocated before it.
fn read_peak(bytes: &[u8]) -> (Result<Song, honeydew::flm::FlmError>, usize) {
    let before = NOW.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let song = read(bytes);
    (song, PEAK.load(Ordering::Relaxed) - before)
}

/// The reader's budget (64 MiB + 2 × the file) plus the strings and payload copies it does not count (at most the
/// file again, twice for a lossy copy and its source) and the transient decode buffers.
fn memory_bound(len: usize) -> usize {
    (80 << 20) + 6 * len
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
}

/// The offsets of the size fields of every chunk header in `bytes` (four printable bytes then a size).
fn size_fields(bytes: &[u8]) -> Vec<usize> {
    (4..bytes.len().saturating_sub(8))
        .filter(|&at| bytes[at..at + 4].iter().all(|b| b.is_ascii_alphanumeric()))
        .map(|at| at + 4)
        .collect()
}

fn mutate(rng: &mut Rng, base: &[u8], sizes: &[usize]) -> Vec<u8> {
    let mut bytes = base.to_vec();
    for _ in 0..1 + rng.below(3) {
        let len = bytes.len().max(1);
        match rng.below(8) {
            0 => {
                let at = rng.below(len);
                if let Some(b) = bytes.get_mut(at) {
                    *b ^= 1 << rng.below(8);
                }
            }
            1 => bytes.truncate(rng.below(len)),
            2 if sizes.iter().any(|&at| at + 4 <= bytes.len()) => {
                let at = sizes[rng.below(sizes.len())];
                if at + 4 > bytes.len() {
                    continue;
                }
                let size = [0, 1, 7, 8, 0x0fff_ffff, 0x1000_0001, u32::MAX][rng.below(7)];
                let size = if rng.below(2) == 0 {
                    size
                } else {
                    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
                        .wrapping_add(rng.below(3) as u32)
                        .wrapping_sub(1)
                };
                bytes[at..at + 4].copy_from_slice(&size.to_le_bytes());
            }
            3 => {
                let at = rng.below(len);
                let insert: Vec<u8> = (0..1 + rng.below(16)).map(|_| rng.next() as u8).collect();
                bytes.splice(at.min(bytes.len())..at.min(bytes.len()), insert);
            }
            4 => {
                let at = rng.below(len);
                let end = (at + 1 + rng.below(64)).min(bytes.len());
                bytes.drain(at.min(end)..end);
            }
            5 => {
                let at = rng.below(len);
                let end = (at + 1 + rng.below(256)).min(bytes.len());
                let block = bytes[at.min(end)..end].to_vec();
                bytes.splice(end..end, block);
            }
            6 => {
                let at = rng.below(len.saturating_sub(8).max(1));
                let patterns: [[u8; 8]; 5] = [
                    f64::NAN.to_le_bytes(),
                    f64::INFINITY.to_le_bytes(),
                    f64::MAX.to_le_bytes(),
                    [0xff; 8],
                    (f32::NAN.to_bits() as u64 | (u64::from(f32::NAN.to_bits()) << 32))
                        .to_le_bytes(),
                ];
                let pattern = patterns[rng.below(5)];
                let end = (at + 8).min(bytes.len());
                if at < end {
                    bytes[at..end].copy_from_slice(&pattern[..end - at]);
                }
            }
            _ => {
                let at = rng.below(len);
                if let Some(b) = bytes.get_mut(at) {
                    *b = rng.next() as u8;
                }
            }
        }
    }
    bytes
}

/// What `fuzz` saw.
#[derive(Default, Debug)]
struct Seen {
    buffers: usize,
    accepted: usize,
    peak: usize,
}

/// `iterations` mutations of `base`: no panic, every accepted song saves and reads back equal and saves again to the
/// same bytes, and no read takes more than [`memory_bound`].
fn fuzz(name: &str, base: &[u8], iterations: usize, seen: &mut Seen) {
    let sizes = size_fields(base);
    let mut rng = Rng(seed() ^ base.len() as u64);
    for i in 0..iterations {
        let bytes = mutate(&mut rng, base, &sizes);
        let (song, peak) = read_peak(&bytes);
        seen.buffers += 1;
        seen.peak = seen.peak.max(peak);
        assert!(
            peak <= memory_bound(bytes.len()),
            "{name} #{i}: {peak} B for {} B",
            bytes.len()
        );
        let Ok(song) = song else {
            continue;
        };
        seen.accepted += 1;
        let written =
            write(&song).unwrap_or_else(|e| panic!("{name} #{i}: read but not written: {e}"));
        let again =
            read(&written).unwrap_or_else(|e| panic!("{name} #{i}: written but not read: {e}"));
        assert!(again == song, "{name} #{i}: read(write(song)) != song");
        assert!(
            write(&again).unwrap() == written,
            "{name} #{i}: the writer is not idempotent"
        );
    }
}

/// `FLM_FUZZ_SEED`, or a fixed seed.
fn seed() -> u64 {
    let seed = std::env::var("FLM_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok());
    seed.unwrap_or(0x9e37_79b9_7f4a_7c15)
}

fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut corpus = vec![
        (
            "owner-fresh-song.flm".to_owned(),
            include_bytes!("fixtures/owner-fresh-song.flm").to_vec(),
        ),
        (
            "every feature".to_owned(),
            write(&common::every_feature_song()).unwrap(),
        ),
    ];
    let mut small = Song::default();
    small.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    corpus.push(("small".to_owned(), write(&small).unwrap()));
    // The local file, when present and asked for (it is slow in a debug build).
    if std::env::var_os("FLM_FUZZ_ITERS").is_some()
        && let Some(home) = std::env::var_os("HOME")
        && let Ok(bytes) = std::fs::read(std::path::Path::new(&home).join("Downloads/sample.flm"))
    {
        corpus.push(("sample.flm".to_owned(), bytes));
    }
    corpus
}

#[test]
fn mutated_files_never_panic_and_what_reads_writes_back() {
    let iterations: usize = std::env::var("FLM_FUZZ_ITERS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(6_000);
    let corpus = corpus();
    let mut seen = Seen::default();
    for (name, bytes) in &corpus {
        fuzz(name, bytes, iterations / corpus.len(), &mut seen);
    }
    println!("{seen:?}");
    assert!(seen.accepted > seen.buffers / 10, "{seen:?}");

    // Hostile shapes that cost the old reader 35–70× their size, each far past the 64 MiB base.
    let rmod_stubs = |type_id: u32, count: usize| {
        let mut writer = ChunkWriter::new_file();
        writer
            .chunk(*b"RACK", |w| {
                w.u32(0)?;
                w.raw(b"10KR")?;
                for id in 0..count as u32 {
                    w.chunk(*b"RMOd", |w| {
                        w.u32(type_id)?;
                        w.u32(id + 2)
                    })?;
                }
                Ok(())
            })
            .unwrap();
        writer.into_bytes()
    };
    let master = |writer: &mut ChunkWriter| {
        let rack = |w: &mut ChunkWriter| {
            w.u32(0)?;
            w.raw(b"10KR")
        };
        writer.chunk(*b"RACK", rack).unwrap();
    };
    let empty_clips = |count: usize| {
        let mut writer = ChunkWriter::new_file();
        master(&mut writer);
        writer
            .chunk(*b"CHNL", |w| {
                w.u32(0)?;
                w.raw(b"20HC")?;
                w.chunk(*b"TRKH", |w| {
                    w.chunk(*b"DESc", |w| w.raw(&[0; 1052]))?;
                    for _ in 0..count {
                        w.chunk(*b"CLIP", |w| {
                            w.u32(0)?;
                            w.raw(b"20LC")
                        })?;
                    }
                    Ok(())
                })
            })
            .unwrap();
        writer.into_bytes()
    };
    let small_records = |size: u16, count: usize| {
        let mut writer = ChunkWriter::new_file();
        master(&mut writer);
        writer
            .chunk(*b"CHNL", |w| {
                w.u32(0)?;
                w.raw(b"20HC")?;
                w.chunk(*b"TRKH", |w| {
                    w.chunk(*b"DESc", |w| w.raw(&[0; 1052]))?;
                    w.chunk(*b"CLIP", |w| {
                        w.u32(0)?;
                        w.raw(b"20LC")?;
                        w.chunk(*b"EVN2", |w| {
                            w.u16(size)?;
                            // Positions run backwards where a record holds one, so the reader must sort.
                            let record = |n: usize| (u32::MAX - n as u32).to_le_bytes();
                            let bytes: Vec<u8> = (0..count)
                                .flat_map(|n| record(n).into_iter().cycle().take(usize::from(size)))
                                .collect();
                            w.raw(&bytes)
                        })
                    })
                })
            })
            .unwrap();
        writer.into_bytes()
    };
    let hostile = [
        ("RMOd stubs, 3x Osc", rmod_stubs(39, 1 << 20)),
        ("RMOd stubs, MiniSynth", rmod_stubs(0, 1 << 20)),
        ("empty CLIPs", empty_clips(1 << 20)),
        // 2.7 MB of 1-byte records peaked at 151 MB (the re-encode was not charged).
        (
            "EVN2 with 1-byte records, accepted",
            small_records(1, 2_700_000),
        ),
        (
            "EVN2 with 2-byte records, accepted",
            small_records(2, 2_500_000),
        ),
        (
            "EVN2 with 3-byte records, accepted",
            small_records(3, 2_300_000),
        ),
        (
            "EVN2 with 4-byte records out of order",
            small_records(4, 2_000_000),
        ),
        (
            "EVN2 with 1-byte records, refused",
            small_records(1, 24 << 20),
        ),
    ];
    for (name, bytes) in hostile {
        let (song, peak) = read_peak(&bytes);
        println!(
            "{name}: {} B in, {peak} B peak, {:?}",
            bytes.len(),
            song.as_ref().map(|_| ()).map_err(|e| e.kind)
        );
        assert!(
            peak <= memory_bound(bytes.len()),
            "{name}: {peak} B for {} B",
            bytes.len()
        );
    }
}

/// Replays mutation `FLM_FUZZ_REPLAY` of the first corpus file and prints where `read(write(song))` first differs.
#[test]
#[ignore = "a debugging aid: FLM_FUZZ_REPLAY=n cargo test --test flm_fuzz replay -- --ignored --nocapture"]
fn replay_one_mutation() {
    let (name, base) = corpus().into_iter().next().unwrap();
    let target: usize = std::env::var("FLM_FUZZ_REPLAY").unwrap().parse().unwrap();
    let sizes = size_fields(&base);
    let mut rng = Rng(seed() ^ base.len() as u64);
    for i in 0..=target {
        let bytes = mutate(&mut rng, &base, &sizes);
        if i == target {
            let Ok(song) = read(&bytes) else {
                return println!("{name} #{i}: not read");
            };
            let again = read(&write(&song).unwrap()).unwrap();
            let (a, b) = (format!("{song:#?}"), format!("{again:#?}"));
            for (n, (x, y)) in a.lines().zip(b.lines()).enumerate() {
                if x != y {
                    let lines: Vec<&str> = a.lines().collect();
                    println!(
                        "{name} line {n}: {x} -> {y}\n{}",
                        lines[n.saturating_sub(30)..n].join("\n")
                    );
                    break;
                }
            }
        }
    }
}

//! The input ring: one producer, readers with their own positions, overruns, the 25 ms peak
//! history, and no allocation on the producer's or a reader's path. No device is opened: the tests push frames.
//!
//! The `rt_alloc` scenario lives here, with its own per-thread counting allocator (as `rt_alloc.rs` counts), until
//! `crates/audio/tests/rt_alloc.rs` exists to take it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

use honeydew_audio::input::{InputReader, InputRing, PEAK_SLOTS};

struct Counting;

thread_local! {
    /// Allocations, reallocations and frees on this thread.
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    // A const thread-local without a destructor is always there, even while the thread is torn down.
    let _ = CALLS.try_with(|calls| calls.set(calls.get() + 1));
}

// SAFETY: every call forwards to `System` unchanged; counting touches only a const thread-local.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count();
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// How many allocator calls `f` makes on this thread.
fn allocator_calls(f: impl FnOnce()) -> usize {
    let before = CALLS.with(Cell::get);
    f();
    CALLS.with(Cell::get) - before
}

fn ring(channels: usize, rate: u32, capacity: usize) -> Arc<InputRing> {
    Arc::new(InputRing::new(channels, rate, capacity))
}

/// Pushes mono frames `range` as their own values.
fn push_range(ring: &InputRing, range: std::ops::Range<u32>) {
    ring.push(range.map(|i| i as f32));
}

/// Everything `reader` has, as mono frames of port 0.
fn drain(reader: &mut InputReader) -> Vec<f32> {
    let mut out = vec![0.0; 64];
    let n = reader.read(&[0], &mut out);
    out.truncate(n);
    out
}

fn frames(range: std::ops::Range<u32>) -> Vec<f32> {
    range.map(|i| i as f32).collect()
}

#[test]
fn a_reader_follows_the_ring_across_its_wrap() {
    let ring = ring(1, 48_000, 8);
    let mut reader = InputReader::new(ring.clone());
    push_range(&ring, 0..5);
    assert_eq!(drain(&mut reader), frames(0..5));
    // Frames 5..11 wrap past slot 7 back to slot 0.
    push_range(&ring, 5..11);
    assert_eq!(drain(&mut reader), frames(5..11));
    assert_eq!(drain(&mut reader), Vec::<f32>::new());
    assert_eq!(reader.overruns(), 0);
    assert_eq!(ring.written(), 11);
}

#[test]
fn a_short_buffer_takes_what_fits_and_the_rest_waits() {
    let ring = ring(1, 48_000, 16);
    let mut reader = InputReader::new(ring.clone());
    push_range(&ring, 0..10);
    let mut out = [0.0; 4];
    assert_eq!(reader.read(&[0], &mut out), 4);
    assert_eq!(out, [0.0, 1.0, 2.0, 3.0]);
    assert_eq!(drain(&mut reader), frames(4..10));
}

#[test]
fn readers_keep_their_own_positions_and_start_at_the_newest_frame() {
    let ring = ring(1, 48_000, 8);
    let mut early = InputReader::new(ring.clone());
    push_range(&ring, 0..4);
    let mut late = InputReader::new(ring.clone());
    push_range(&ring, 4..6);
    assert_eq!(drain(&mut early), frames(0..6));
    assert_eq!(drain(&mut late), frames(4..6));
    push_range(&ring, 6..9);
    assert_eq!(drain(&mut late), frames(6..9));
    assert_eq!(drain(&mut early), frames(6..9));
}

#[test]
fn a_reader_picks_its_ports_from_the_interleaved_frames() {
    let ring = ring(2, 48_000, 8);
    let mut left = InputReader::new(ring.clone());
    let mut swapped = InputReader::new(ring.clone());
    let mut beyond = InputReader::new(ring.clone());
    // Two whole stereo frames; the trailing half frame is dropped.
    ring.push([1.0, -1.0, 2.0, -2.0, 3.0].into_iter());
    assert_eq!(ring.written(), 2);
    let mut out = [0.0; 8];
    assert_eq!(left.read(&[0], &mut out), 2);
    assert_eq!(out[..2], [1.0, 2.0]);
    assert_eq!(swapped.read(&[1, 0], &mut out), 2);
    assert_eq!(out[..4], [-1.0, 1.0, -2.0, 2.0]);
    // A port the device does not have reads silence.
    assert_eq!(beyond.read(&[0, 5], &mut out), 2);
    assert_eq!(out[..4], [1.0, 0.0, 2.0, 0.0]);
}

#[test]
fn an_overrun_skips_to_the_oldest_frame_still_held_and_is_counted() {
    let ring = ring(1, 48_000, 8);
    let mut reader = InputReader::new(ring.clone());
    push_range(&ring, 0..20);
    assert_eq!(drain(&mut reader), frames(12..20));
    assert_eq!(reader.overruns(), 1);
    push_range(&ring, 20..23);
    assert_eq!(drain(&mut reader), frames(20..23));
    assert_eq!(reader.overruns(), 1);
    // One push larger than the ring keeps its last frames.
    push_range(&ring, 23..40);
    assert_eq!(drain(&mut reader), frames(32..40));
    assert_eq!(reader.overruns(), 2);
}

#[test]
fn the_peak_history_closes_a_slot_every_25_ms_of_frames() {
    // 48 kHz: 1200 frames a slot.
    let ring = ring(2, 48_000, 4096);
    assert_eq!(ring.window_frames(), 1200);
    let mut block = vec![0.0_f32; 2 * 1199];
    block[20] = 0.5;
    block[41] = -0.75;
    ring.push(block.iter().copied());
    assert_eq!(ring.peaks_done(), 0);
    assert_eq!(ring.peak(0, 0), 0.0);
    ring.push([0.0, 0.0].into_iter());
    assert_eq!(ring.peaks_done(), 1);
    assert_eq!((ring.peak(0, 0), ring.peak(0, 1)), (0.5, 0.75));
    // The next slot, pushed in odd blocks: its peak is its own.
    ring.push(std::iter::repeat_n(0.25, 2 * 7));
    ring.push(std::iter::repeat_n(-0.125, 2 * 1192));
    assert_eq!(ring.peaks_done(), 1);
    ring.push([0.0, 0.0].into_iter());
    assert_eq!(ring.peaks_done(), 2);
    assert_eq!((ring.peak(0, 0), ring.peak(1, 0)), (0.25, 0.5));
    // Older than the history, or a port the device lacks: silence.
    assert_eq!(ring.peak(2, 0), 0.0);
    assert_eq!(ring.peak(0, 2), 0.0);

    // 44.1 kHz: 1102.5 frames round to 1103.
    assert_eq!(InputRing::new(1, 44_100, 16).window_frames(), 1103);
}

#[test]
fn the_peak_history_keeps_the_newest_slots() {
    let ring = ring(1, 48_000, 64);
    // The meter's 48, without the slot the producer writes next.
    const { assert!(PEAK_SLOTS > 48) };
    let slots = PEAK_SLOTS as u32 + 10;
    for slot in 0..slots {
        ring.push(std::iter::repeat_n(slot as f32 / 1000.0, 1200));
    }
    assert_eq!(ring.peaks_done(), u64::from(slots));
    for age in 0..PEAK_SLOTS - 1 {
        assert_eq!(ring.peak(age, 0), (slots - 1 - age as u32) as f32 / 1000.0);
    }
    assert_eq!(ring.peak(PEAK_SLOTS - 1, 0), 0.0);
    assert_eq!(ring.peak(PEAK_SLOTS, 0), 0.0);
}

#[test]
fn available_is_the_backlog_a_read_would_take() {
    let ring = ring(1, 48_000, 8);
    let mut reader = InputReader::new(ring.clone());
    assert_eq!(reader.available(), 0);
    push_range(&ring, 0..5);
    assert_eq!(reader.available(), 5);
    let mut out = [0.0; 3];
    reader.read(&[0], &mut out);
    assert_eq!(reader.available(), 2);
    // A whole ring behind: what is still held.
    push_range(&ring, 5..30);
    assert_eq!(reader.available(), 8);
}

#[test]
fn a_failed_stream_is_flagged_on_its_ring() {
    let ring = ring(1, 48_000, 8);
    assert!(!ring.failed());
    ring.set_failed();
    assert!(ring.failed());
}

/// Frame `n` holds `n + port/4` on each port: exact in f32 for these counts.
fn stamp(n: u64, port: usize) -> f32 {
    n as f32 + port as f32 * 0.25
}

#[test]
fn readers_racing_the_producer_get_only_whole_consecutive_frames() {
    // A tiny ring the producer laps constantly, so readers copy slots while they are overwritten.
    const CHANNELS: usize = 3;
    const FRAMES: u64 = 200_000;
    let ring = ring(CHANNELS, 48_000, 8);
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let readers: Vec<_> = [vec![0, 1, 2], vec![2, 0], vec![1], vec![0, 2, 1]]
        .into_iter()
        .enumerate()
        .map(|(r, ports)| {
            let mut reader = InputReader::new(ring.clone());
            let done = done.clone();
            std::thread::spawn(move || {
                let width = ports.len();
                let mut out = vec![0.0_f32; width * (1 + r * 3)];
                let (mut next, mut overruns, mut frames) = (None, 0, 0);
                // One last read after the producer is done, so every reader reads and overruns at least once.
                loop {
                    let finished = done.load(std::sync::atomic::Ordering::Acquire);
                    let n = reader.read(&ports, &mut out);
                    if n == 0 {
                        if finished {
                            break;
                        }
                        continue;
                    }
                    let first = (out[0] - ports[0] as f32 * 0.25) as u64;
                    for (i, frame) in out[..n * width].chunks_exact(width).enumerate() {
                        for (&got, &port) in frame.iter().zip(&ports) {
                            assert_eq!(
                                got,
                                stamp(first + i as u64, port),
                                "reader {r}: torn frame"
                            );
                        }
                    }
                    if let Some(expected) = next {
                        // A gap only ever skips ahead, and only with an overrun counted.
                        assert!(first >= expected, "reader {r}: went back");
                        assert!(
                            first == expected || reader.overruns() > overruns,
                            "reader {r}: gap {expected} → {first} without an overrun"
                        );
                    }
                    next = Some(first + n as u64);
                    overruns = reader.overruns();
                    frames += n;
                    if finished {
                        break;
                    }
                }
                (frames, overruns)
            })
        })
        .collect();
    let mut block = Vec::with_capacity(CHANNELS * 16);
    let (mut n, mut seed) = (0_u64, 12_345_u32);
    while n < FRAMES {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let frames = 1 + u64::from(seed >> 28); // 1…16: some blocks lap the whole ring
        block.clear();
        block.extend((n..n + frames).flat_map(|f| (0..CHANNELS).map(move |p| stamp(f, p))));
        ring.push(block.iter().copied());
        n += frames;
    }
    done.store(true, std::sync::atomic::Ordering::Release);
    for reader in readers {
        let (frames, overruns) = reader.join().expect("a reader failed");
        assert!(frames > 0 && overruns > 0);
    }
}

#[test]
fn rt_alloc_pushing_and_reading_never_allocate() {
    let ring = ring(2, 48_000, 4096);
    let mut monitor = InputReader::new(ring.clone());
    let mut recorder = InputReader::new(ring.clone());
    let mut out = vec![0.0_f32; 2 * 1024];
    let device: Vec<i16> = (0..2 * 5000)
        .map(|i| (i * 37 % 65_536 - 32_768) as i16)
        .collect();
    let mut peaks = 0.0;
    let calls = allocator_calls(|| {
        // Odd block sizes, one larger than the ring, converted on the way in as a device callback does.
        for &frames in [1, 37, 256, 1023, 5000].iter().cycle().take(40) {
            ring.push(
                device[..2 * frames]
                    .iter()
                    .map(|&s| f32::from(s) / 32_768.0),
            );
            monitor.read(&[0, 1], &mut out);
            recorder.read(&[1], &mut out[..512]);
            peaks += ring.peak(0, 0) + ring.peak(47, 1);
        }
    });
    assert_eq!(calls, 0);
    assert!(peaks > 0.0);
    assert!(monitor.overruns() > 0 && recorder.overruns() > 0);
}

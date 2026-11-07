//! The sound never allocates: with every effect, the arpeggiator, full unison and the matrix on, rendering and playing
//! notes allocate nothing once the engine is built (the audio thread must never wait on the allocator).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use rind::engine::Engine;
use rind::ids;
use rind::params::RindParams;

struct Counting;

thread_local! {
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(ptr, layout, new) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocs() -> usize {
    ALLOCS.with(Cell::get)
}

#[test]
fn rendering_allocates_nothing() {
    let mut p = RindParams::defaults();
    for on in [
        ids::ARP_ON,
        ids::DIST_ON,
        ids::PHS_ON,
        ids::CHO_ON,
        ids::EQ_ON,
        ids::DLY_ON,
        ids::REV_ON,
        ids::CMP_ON,
    ] {
        p[on] = 1.0;
    }
    for i in 0..4 {
        p[ids::OSC_BASE + i * ids::OSC_STRIDE + ids::osc::VOICES] = 8.0;
    }
    // A matrix slot from LFO 1 to the filter's cutoff.
    p[ids::MAT_BASE + ids::mat::SRC] = 1.0;
    p[ids::MAT_BASE + ids::mat::DST] = 1.0;
    p[ids::MAT_BASE + ids::mat::AMT] = 0.5;

    let mut engine = Engine::new(48_000.0);
    let (mut l, mut r) = (vec![0.0f32; 480], vec![0.0f32; 480]);
    // Warm up: any lazy setup happens here.
    engine.note_on(60, 0.8);
    engine.render(&p, 120.0, &mut l, &mut r);

    let before = allocs();
    for block in 0..400 {
        match block % 40 {
            0 => {
                for k in [48, 55, 60, 64, 67] {
                    engine.note_on(k, 0.9);
                }
            }
            20 => {
                for k in [48, 55, 60, 64, 67] {
                    engine.note_off(k);
                }
            }
            _ => {}
        }
        if block % 50 == 7 {
            engine.pitch_bend(0.5);
            engine.cc(1, 0.7);
            engine.pressure(0.4);
        }
        engine.render(&p, 120.0, &mut l, &mut r);
    }
    engine.all_notes_off(true);
    engine.render(&p, 120.0, &mut l, &mut r);
    assert_eq!(allocs() - before, 0, "the render path allocated");
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
}

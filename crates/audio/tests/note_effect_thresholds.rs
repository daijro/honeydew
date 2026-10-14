//! Two-state thresholds of the note effects.
//!
//! Each param below is on at v > 0.5, so exactly 0.5 is off; the Arpeggiator's Swing rate is the exception (a two-item
//! menu, `round(v)`, so 0.5 picks the second item). Every test renders the effect at 0 and at 1, checks that the two
//! differ, and then checks that each probe value renders exactly like one of them.

use std::fmt::Debug;

use honeydew_audio::note_effects::{NoteEffectModule, NoteEffectRack};
use honeydew_audio::sequencer::{InputNoteEvent, NoteKind};

/// 0.001, exactly 0.5, the next f32 above it, and the ends.
const PROBES: [f32; 6] = [
    0.0,
    0.001,
    0.5,
    f32::from_bits(0.5f32.to_bits() + 1),
    0.75,
    1.0,
];

/// `render` at the probes against its renders at 0 and 1: on above 0.5, and at 0.5 itself only if `half_is_on`.
fn two_state<T: PartialEq + Debug>(site: &str, half_is_on: bool, render: impl Fn(f32) -> T) {
    let (off, on) = (render(0.0), render(1.0));
    assert_ne!(off, on, "{site}: on and off must differ");
    for v in PROBES {
        let is_on = v > 0.5 || (half_is_on && v == 0.5);
        assert_eq!(
            render(v),
            *if is_on { &on } else { &off },
            "{site} at {v:?} must be {}",
            if is_on { "on" } else { "off" }
        );
    }
}

fn module(type_id: u16, params: Vec<f32>) -> NoteEffectModule {
    NoteEffectModule {
        type_id,
        instance_id: 2,
        params,
        changes: Vec::new(),
    }
}

/// `module` on `input` over beats 0 … `end`, at 120 BPM.
fn run(module: NoteEffectModule, end: f64, input: &[InputNoteEvent]) -> Vec<InputNoteEvent> {
    NoteEffectRack::new(vec![module])
        .unwrap()
        .process_events(0.0, end, 120.0, input)
        .unwrap()
}

/// A note of `pitch` from beat 0 to `off`.
fn held(pitch: i16, off: f64) -> [InputNoteEvent; 2] {
    [
        InputNoteEvent::on(0.0, pitch, 100),
        InputNoteEvent::off(off, pitch),
    ]
}

/// The note-ons of `events` as (beat, pitch, pan).
fn ons(events: &[InputNoteEvent]) -> Vec<(f64, i16, f32)> {
    (events.iter())
        .filter(|event| event.kind == NoteKind::On)
        .map(|event| (event.beat, event.pitch, event.pan))
        .collect()
}

// Note Echo (47)

/// Note Echo with 1 repetition, Sync on (a delay of 1/2 beat in p3; p2 reads 2.02 beats while Sync is off) and the
/// echo an octave up, so that the held played note does not suppress it, and `set` on top.
fn echo(set: &[(usize, f32)]) -> NoteEffectModule {
    let mut params = vec![0.5; 16];
    params[0] = 1.0;
    params[1] = 1.0 / 32.0;
    params[3] = ((0.5 - 1.0 / 32.0) / (8.0 - 1.0 / 32.0_f64)).sqrt() as f32;
    params[4] = 1.0;
    params[5] = 0.75;
    params[11] = 0.0;
    params[14] = 0.0;
    for &(index, value) in set {
        params[index] = value;
    }
    module(47, params)
}

/// Sync (p4): the delay is p3's item while it is on, p2's value while off.
#[test]
fn note_echo_sync_is_on_above_half() {
    two_state("Note Echo p4 Sync", false, |v| {
        ons(&run(echo(&[(2, 0.5), (4, v)]), 400.0, &held(60, 1.0)))
    });
}

/// Ping-Pong (p14): negates the emitted pan of repeat 2.
#[test]
fn note_echo_ping_pong_is_on_above_half() {
    two_state("Note Echo p14 Ping-Pong", false, |v| {
        // Two repeats at pan +0.5 (Pan Offset 0.75, no drift): repeat 2 is -0.5 with Ping-Pong.
        let module = echo(&[(1, 2.0 / 32.0), (12, 0.75), (13, 0.5), (14, v)]);
        ons(&run(module, 400.0, &held(60, 0.25)))
    });
}

// Scale (50)

/// Scale with Snap `snap` and the note toggles p4–p15 all off but `set` (pitch class, value).
fn scale(snap: f32, set: &[(usize, f32)]) -> NoteEffectModule {
    let mut params = vec![0.0; 16];
    params[0] = 1.0;
    params[3] = snap;
    for &(class, value) in set {
        params[4 + class] = value;
    }
    module(50, params)
}

/// Snap (p3): a note outside the scale snaps to the nearest note in it while on, and is dropped while off.
#[test]
fn scale_snap_is_on_above_half() {
    two_state("Scale p3 Snap", false, |v| {
        ons(&run(scale(v, &[(0, 1.0)]), 2.0, &held(61, 1.0)))
    });
}

/// Each note toggle (p4–p15): its pitch class is in the scale (so the note passes) above 0.5. Snap is off, so a note
/// outside the scale is dropped.
#[test]
fn scale_note_toggles_are_on_above_half() {
    for class in 0..12 {
        two_state(&format!("Scale p{} Note {class}", 4 + class), false, |v| {
            let pitch = 60 + class as i16;
            ons(&run(scale(0.0, &[(class, v)]), 2.0, &held(pitch, 1.0)))
        });
    }
}

// Arpeggiator (51)

/// Arp 1 on over a note-on at 1/8 (p1 = 2/7, half a beat), Rate (ms) 50 (p2 = 0, a tenth of a beat at 120 BPM), and
/// every other param at its default; Arp 2 off.
fn arp(set: &[(usize, f32)]) -> NoteEffectModule {
    let mut params = vec![0.0; 33];
    params[0] = 1.0;
    params[1] = 2.0 / 7.0;
    params[2] = 0.0;
    params[3] = 1.0;
    params[7] = 0.5;
    params[8] = 0.5;
    params[9] = 0.4444;
    params[12] = 0.5;
    params[13] = 0.5;
    params[15] = 0.5;
    params[19] = 1.0;
    params[20] = 0.4444;
    params[23] = 0.5;
    params[24] = 0.5;
    params[26] = 0.5;
    params[31] = 6.0 / 14.0;
    params[32] = 6.0 / 14.0;
    for &(index, value) in set {
        params[index] = value;
    }
    module(51, params)
}

/// Sync (p3): the step is Rate's (p1) while on, Rate (ms)'s (p2) while off.
#[test]
fn arpeggiator_sync_is_on_above_half() {
    two_state("Arpeggiator p3 Sync", false, |v| {
        ons(&run(arp(&[(3, v)]), 2.0, &held(60, 1.0)))
    });
}

/// Latch (p6): the arpeggio goes on after the note is released.
#[test]
fn arpeggiator_latch_is_on_above_half() {
    two_state("Arpeggiator p6 Latch", false, |v| {
        ons(&run(arp(&[(6, v)]), 2.0, &held(60, 1.0)))
    });
}

/// Swing rate (p5) is a two-item menu, `round(v)`, not a toggle: 0.5 picks
/// `1/16`. A full Swing delays the odd steps by 1/8 beat at 1/16 and by 1/4 beat at 1/8.
#[test]
fn arpeggiator_swing_rate_is_a_menu_that_rounds_half_up() {
    two_state("Arpeggiator p5 Swing rate", true, |v| {
        ons(&run(arp(&[(4, 1.0), (5, v)]), 2.0, &held(60, 1.0)))
    });
}

/// Enable (Arp 1) (p19) and (Arp 2) (p30) are the GLOBAL toggles `Arp 1` and `Arp 2`, not param 0.
#[test]
fn arpeggiator_engine_enables_are_on_above_half() {
    two_state("Arpeggiator p19 Arp 1", false, |v| {
        ons(&run(arp(&[(19, v), (30, 0.0)]), 2.0, &held(60, 1.0)))
    });
    two_state("Arpeggiator p30 Arp 2", false, |v| {
        ons(&run(arp(&[(19, 0.0), (30, v)]), 2.0, &held(60, 1.0)))
    });
}

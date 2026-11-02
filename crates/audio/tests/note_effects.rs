use honeydew_audio::note_effects::{
    LiveNoteChain, NoteEffectError, NoteEffectModule, NoteEffectRack,
};
use honeydew_audio::sequencer::{InputNoteEvent, NoteKind};

fn module(type_id: u16, instance_id: i32, params: Vec<f32>) -> NoteEffectModule {
    NoteEffectModule {
        type_id,
        instance_id,
        params,
        changes: Vec::new(),
    }
}

/// Scale (50) with Snap on and the notes p4–p15 on for the pitch classes in `notes` (C = 0).
fn scale_params(notes: &[usize]) -> Vec<f32> {
    let mut params = vec![0.0; 16];
    params[0] = 1.0;
    params[3] = 1.0;
    for &note in notes {
        params[4 + note] = 1.0;
    }
    params
}

const C_MAJOR: [usize; 7] = [0, 2, 4, 5, 7, 9, 11];

/// The note-on pitches of `events`.
fn pitches(events: &[InputNoteEvent]) -> Vec<i16> {
    events
        .iter()
        .filter(|event| event.kind == NoteKind::On)
        .map(|event| event.pitch)
        .collect()
}

fn run(modules: Vec<NoteEffectModule>, events: &[InputNoteEvent]) -> Vec<InputNoteEvent> {
    NoteEffectRack::new(modules)
        .unwrap()
        .process_events(0.0, 2.0, 120.0, events)
        .unwrap()
}

fn note() -> [InputNoteEvent; 2] {
    [
        InputNoteEvent::on(0.0, 60, 100),
        InputNoteEvent::off(1.0, 60),
    ]
}

fn arp_params() -> Vec<f32> {
    let mut params = vec![0.0; 33];
    params[0] = 1.0;
    params[1] = 2.0 / 7.0;
    params[2] = 0.2105;
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
    params
}

#[test]
fn rack_order_transforms_both_ends_of_a_note() {
    let transpose = module(48, 2, vec![1.0, 0.5, 13.0 / 24.0]);
    let scale = module(50, 3, scale_params(&C_MAJOR));
    let input = [
        InputNoteEvent::on(0.0, 61, 100),
        InputNoteEvent::off(1.0, 61),
    ];
    let events = run(vec![transpose.clone(), scale.clone()], &input);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].pitch, 62);
    assert_eq!(events[1].pitch, 62);
    assert_eq!(events[1].kind, NoteKind::Off);
    assert_eq!(run(vec![scale, transpose], &input)[0].pitch, 61);
}

#[test]
fn scale_snaps_or_filters_with_custom_pitch_classes() {
    let mut params = vec![0.0; 16];
    params[0] = 1.0;
    params[2] = 1.0;
    params[3] = 1.0;
    params[4] = 1.0;
    let input = [
        InputNoteEvent::on(0.0, 61, 100),
        InputNoteEvent::off(1.0, 61),
    ];
    assert_eq!(
        run(vec![module(50, 2, params.clone())], &input)[0].pitch,
        60
    );
    params[3] = 0.0;
    assert!(run(vec![module(50, 2, params)], &input).is_empty());
}

/// Key and Scale only write the note toggles (the engine couples them, one implementation with the
/// panel); the toggles are the scale the notes snap to, whatever Key and Scale read.
#[test]
fn the_scale_is_its_note_toggles() {
    let mut input = Vec::new();
    for pitch in 60..72 {
        input.push(InputNoteEvent::on(0.0, pitch, 100));
    }
    for pitch in 60..72 {
        input.push(InputNoteEvent::off(1.0, pitch));
    }
    // D major's notes, while Key reads F# and Scale reads Blues.
    let mut params = scale_params(&[1, 2, 4, 6, 7, 9, 11]);
    params[1] = 6.0 / 11.0;
    params[2] = 22.0 / 33.0;
    params[3] = 0.0;
    let played = pitches(&run(vec![module(50, 2, params)], &input));
    assert_eq!(played, [61, 62, 64, 66, 67, 69, 71]);
}

#[test]
fn note_echo_emits_complete_tail_pairs() {
    let mut params = vec![0.5; 16];
    params[0] = 1.0;
    params[1] = 2.0 / 32.0;
    params[3] = 4.0 / 9.0;
    params[4] = 1.0;
    params[11] = 0.0;
    // Pitch Drift +12: p_k = k·12.
    params[8] = 1.0;
    let output = run(vec![module(47, 2, params)], &note());
    let on: Vec<_> = output.iter().filter(|e| e.kind == NoteKind::On).collect();
    let off: Vec<_> = output.iter().filter(|e| e.kind == NoteKind::Off).collect();
    assert_eq!(on.len(), 3);
    assert_eq!(off.len(), 3);
    assert_eq!(on.iter().map(|e| e.pitch).collect::<Vec<_>>(), [60, 72, 84]);
    assert!(off.last().unwrap().beat > 2.0);
}

#[test]
fn echo_chain_rejects_unbounded_expansion() {
    let mut params = vec![0.5; 16];
    params[0] = 1.0;
    params[1] = 1.0;
    let rack =
        NoteEffectRack::new((2..=4).map(|id| module(47, id, params.clone())).collect()).unwrap();
    let mut input = Vec::new();
    for pitch in 0..100 {
        input.push(InputNoteEvent::on(0.0, pitch, 100));
    }
    for pitch in 0..100 {
        input.push(InputNoteEvent::off(1.0, pitch));
    }
    assert_eq!(
        rack.process_events(0.0, 2.0, 120.0, &input),
        Err(NoteEffectError::TooManyEvents)
    );
}

#[test]
fn a_channel_without_note_effects_passes_any_number_of_notes() {
    // A Black MIDI channel: more notes than note effects may make, and no note effect.
    let rack = NoteEffectRack::new(Vec::new()).unwrap();
    let n = 150_000;
    let mut input = Vec::with_capacity(2 * n);
    for i in 0..n {
        let beat = i as f64 / 1000.0;
        input.push(InputNoteEvent::on(beat, 60, 100));
        input.push(InputNoteEvent::off(beat + 0.0005, 60));
    }
    let output = rack.process_events(0.0, 200.0, 120.0, &input).unwrap();
    assert_eq!(output.len(), 2 * n);
}

#[test]
fn note_echo_default_sync_delay_is_half_a_beat() {
    let mut params = vec![0.5; 16];
    params[0] = 1.0;
    params[1] = 1.0 / 32.0;
    // The default 0.242536 is the `1/2` item.
    params[3] = 0.242536;
    params[4] = 1.0;
    // Pitch Offset +12: the played note (held until beat 1) does not swallow its echo.
    params[5] = 0.75;
    params[11] = 0.0;
    let output = run(vec![module(47, 2, params)], &note());
    let on: Vec<_> = output.iter().filter(|e| e.kind == NoteKind::On).collect();
    assert_eq!(on.len(), 2);
    // Half a beat at 120 BPM and the rack's 48 kHz.
    assert_eq!(frame(on[1].beat), 12_000);
}

/// The delay is b = 1/32 + (8 − 1/32)·v² beats, D = trunc(b·60·fs/BPM) frames. Sync On takes the sync
/// item nearest p3's b; Sync Off takes p2's b, fma_f32(7.96875, v², 0.03125).
#[test]
fn the_echo_delay_is_the_spec_law_in_both_modes() {
    let echo_at = |sync: f32, p2: f32, p3: f32| {
        let mut params = vec![0.5; 16];
        params[0] = 1.0;
        params[1] = 1.0 / 32.0;
        params[2] = p2;
        params[3] = p3;
        params[4] = sync;
        // Pitch Offset +12: echoes before beat 1 are not swallowed by the held played note.
        params[5] = 0.75;
        params[11] = 0.0;
        params[14] = 0.0;
        // From frame 3000 (block 2 ends at 3072), so no repeat lands in the note's own block.
        let input = [
            InputNoteEvent::on(0.125, 60, 100),
            InputNoteEvent::off(1.5, 60),
        ];
        let output = run(vec![module(47, 2, params)], &input);
        let on = output.iter().filter(|e| e.kind == NoteKind::On).nth(1);
        frame(on.unwrap().beat) - 3_000
    };
    let items = [0.125, 0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 3.0, 4.0, 8.0];
    for b in items {
        let v = ((b - 1.0 / 32.0) / (8.0 - 1.0 / 32.0_f64)).sqrt() as f32;
        assert_eq!(echo_at(1.0, 0.0, v), (b * 24_000.0) as u64, "item {b}");
    }
    // An unsnapped p3 takes its nearest item: 0.12 is b = 0.146, nearest 1/8.
    assert_eq!(echo_at(1.0, 0.0, 0.12), 3_000);
    // Sync Off reads p2: b = 2.0234375 at v = 0.5 (48562.5 frames, truncated), 8 at 1.
    assert_eq!(echo_at(0.0, 0.5, 1.0), 48_562);
    assert_eq!(echo_at(0.0, 1.0, 0.0), 192_000);
}

/// The engine's frame for `beat` at 120 BPM and 48 kHz, the rack's defaults (`ceil`, as the engine places notes).
fn frame(beat: f64) -> u64 {
    (beat * 60.0 * 48_000.0 / 120.0).ceil() as u64
}

/// The beat the engine places on frame `frame` (half a frame early, so [`frame`] lands on it exactly).
fn at(frame: u64) -> f64 {
    (frame as f64 - 0.5) / 24_000.0
}

/// Note Echo's repeat 1 lands D frames after the note-on, unless that falls in the note's own processing block
/// (1024 frames from frame 0): then it lands D after the next block start, a deliberate quirk.
#[test]
fn repeat_1_after_the_note_s_own_block_is_late_by_the_rest_of_the_block() {
    // Sync Off, p2 = 0: b = 1/32, D = 750 frames. Pitch Offset +12 keeps clear of the held key.
    let mut params = echo_params(1.0, false);
    params[4] = 0.0;
    params[2] = 0.0;
    params[5] = 0.75;
    // From frame 100 it is late by the rest of the block (924), not a whole one (which would give 1874).
    for (start, repeat) in [(0.0, 1_774), (at(100), 1_774), (1.0 / 32.0, 1_500)] {
        let input = [
            InputNoteEvent::on(start, 60, 100),
            InputNoteEvent::off(1.0, 60),
        ];
        let ons = echo_ons(&params, &input);
        assert_eq!(ons[1], (repeat, 72), "from {start}");
    }
}

/// Automation: a note reads the params in force where it starts, and its note-off follows its note-on.
#[test]
fn an_automated_transpose_acts_on_the_notes_that_start_after_it() {
    let mut transpose = module(48, 2, vec![1.0, 0.5, 0.5]);
    transpose.changes = vec![(1.0, 2, 13.0 / 24.0), (3.0, 0, 0.0)];
    let input = [
        InputNoteEvent::on(0.0, 60, 100),
        InputNoteEvent::on(0.5, 64, 100),
        InputNoteEvent::off(1.5, 60),
        InputNoteEvent::on(2.0, 60, 100),
        InputNoteEvent::off(2.5, 64),
        InputNoteEvent::off(2.5, 60),
        // Enable is off from beat 3: the note passes as it is.
        InputNoteEvent::on(3.0, 60, 100),
        InputNoteEvent::off(3.5, 60),
    ];
    let output = NoteEffectRack::new(vec![transpose])
        .unwrap()
        .process_events(0.0, 4.0, 120.0, &input)
        .unwrap();
    assert_eq!(pitches(&output), [60, 64, 61, 60]);
    let offs: Vec<_> = output
        .iter()
        .filter(|event| event.kind == NoteKind::Off)
        .map(|event| (event.beat, event.pitch))
        .collect();
    assert_eq!(offs, [(1.5, 60), (2.5, 64), (2.5, 61), (3.5, 60)]);
}

/// The Arpeggiator switches its settings at the change: Rate 1/8 (half a beat) then 1/16 from beat 1.
#[test]
fn an_automated_arpeggiator_switches_rate_at_the_change() {
    let mut arp = module(51, 2, arp_params());
    arp.changes = vec![(1.0, 1, 0.0)];
    let input = [
        InputNoteEvent::on(0.0, 60, 100),
        InputNoteEvent::off(2.0, 60),
    ];
    let output = NoteEffectRack::new(vec![arp])
        .unwrap()
        .process_events(0.0, 4.0, 120.0, &input)
        .unwrap();
    let beats: Vec<_> = output
        .iter()
        .filter(|event| event.kind == NoteKind::On)
        .map(|event| event.beat)
        .collect();
    assert_eq!(beats, [0.0, 0.5, 1.0, 1.25, 1.5, 1.75]);
    assert_eq!(
        output.iter().filter(|e| e.kind == NoteKind::Off).count(),
        beats.len()
    );

    // Slower from beat 1 (1/16 to 1/8): the steps at 1.0 and 1.5 still play.
    let mut params = arp_params();
    params[1] = 0.0;
    let mut arp = module(51, 2, params);
    arp.changes = vec![(1.0, 1, 2.0 / 7.0)];
    let output = NoteEffectRack::new(vec![arp])
        .unwrap()
        .process_events(0.0, 4.0, 120.0, &input)
        .unwrap();
    let beats: Vec<_> = output
        .iter()
        .filter(|event| event.kind == NoteKind::On)
        .map(|event| event.beat)
        .collect();
    assert_eq!(beats, [0.0, 0.25, 0.5, 0.75, 1.0, 1.5]);
}

/// Latch automated Off releases, at the change, the latched notes no longer held. Arpeggiator 1 Up
/// at 1/8 over C (held to beat 0.5, then latched) and E (held to beat 3).
#[test]
fn latch_automated_off_releases_the_notes_no_longer_held() {
    let mut params = arp_params();
    params[6] = 1.0;
    params[10] = 2.0 / 23.0;
    let mut arp = module(51, 2, params);
    arp.changes = vec![(1.0, 6, 0.0)];
    let input = [
        InputNoteEvent::on(0.0, 60, 100),
        InputNoteEvent::on(0.0, 64, 100),
        InputNoteEvent::off(0.5, 60),
        InputNoteEvent::off(3.0, 64),
    ];
    let output = NoteEffectRack::new(vec![arp])
        .unwrap()
        .process_events(0.0, 4.0, 120.0, &input)
        .unwrap();
    let ons: Vec<_> = output
        .iter()
        .filter(|event| event.kind == NoteKind::On)
        .map(|event| (event.beat, event.pitch))
        .collect();
    assert_eq!(
        ons,
        [
            (0.0, 60),
            (0.5, 64),
            (1.0, 64),
            (1.5, 64),
            (2.0, 64),
            (2.5, 64)
        ]
    );
}

/// Note Echo's output, the played note and then its repeats, as (start, pitch, velocity byte, pan, length) for one note
/// of pitch 60, velocity `velocity` and pan `pan`, from beat 0 for 1 beat, with a 1-beat synced delay, no Delay Rand.
/// and the params `set` (index, value) on top of the defaults.
fn echoes(velocity: u8, pan: f32, set: &[(usize, f32)]) -> Vec<(f64, i16, u8, f32, f64)> {
    echoes_of(60, velocity, pan, set)
}

/// [`echoes`] of a note of pitch `pitch`.
fn echoes_of(
    pitch: i16,
    velocity: u8,
    pan: f32,
    set: &[(usize, f32)],
) -> Vec<(f64, i16, u8, f32, f64)> {
    let mut params = vec![0.5; 16];
    params[0] = 1.0;
    params[3] = ((1.0 - 1.0 / 32.0) / (8.0 - 1.0 / 32.0_f64)).sqrt() as f32;
    params[4] = 1.0;
    params[11] = 0.0;
    params[14] = 0.0;
    for &(index, value) in set {
        params[index] = value;
    }
    let input = [
        InputNoteEvent::on(0.0, pitch, velocity).with_pan(pan),
        InputNoteEvent::off(1.0, pitch),
    ];
    let output = run(vec![module(47, 2, params)], &input);
    let mut notes = Vec::new();
    for on in output.iter().filter(|event| event.kind == NoteKind::On) {
        let off = output
            .iter()
            .find(|e| e.kind == NoteKind::Off && e.beat > on.beat && e.pitch == on.pitch);
        let length = off.unwrap().beat - on.beat;
        notes.push((on.beat, on.pitch, on.velocity, on.pan, length));
    }
    notes
}

/// The played note is never altered: no offset touches it, and it is never dropped for
/// low velocity. With Repetitions 0 it passes alone.
#[test]
fn the_played_note_passes_unchanged() {
    // Pitch Offset +24, Vel. Offset −90 %, Pan Offset −1, and byte 4 (vel₁ = 0: no repeat sounds).
    let set = [(1, 2.0 / 32.0), (5, 1.0), (6, 0.0), (12, 0.0)];
    assert_eq!(echoes(4, 0.7, &set), [(0.0, 60, 4, 0.7, 1.0)]);
    let set = [(1, 0.0), (5, 1.0), (6, 1.0), (12, 1.0)];
    assert_eq!(echoes(200, 0.7, &set), [(0.0, 60, 200, 0.7, 1.0)]);
}

/// key₁ = clamp(clamp(key + PO, 0, 127) + PD, 0, 127), keyₖ = clamp(keyₖ₋₁ + PD, 0, 127); PO = llroundf(48·v5 − 24),
/// PD = llroundf(24·v8 − 12).
#[test]
fn the_echo_keys_are_offset_then_drift_clamped_at_each_step() {
    // Offset +12 (p5 = 0.75), Drift +1 (p8 = 0.5 + 1/24), two repetitions.
    let set = [(1, 2.0 / 32.0), (5, 0.75), (8, 0.5 + 1.0 / 24.0)];
    let keys: Vec<_> = echoes(200, 0.5, &set).iter().map(|n| n.1).collect();
    assert_eq!(keys, [60, 73, 74]);
    // From 120, Offset +24 clamps to 127 before Drift −12 (p8 = 0) applies: 115, then 103.
    let set = [(1, 2.0 / 32.0), (5, 1.0), (8, 0.0)];
    let keys: Vec<_> = echoes_of(120, 200, 0.5, &set).iter().map(|n| n.1).collect();
    assert_eq!(keys, [120, 115, 103]);
    // From 10, Offset −24 clamps to 0, Drift +12 then adds; Drift −12 from 0 stays at 0.
    let set = [(1, 2.0 / 32.0), (5, 0.0), (8, 1.0)];
    let keys: Vec<_> = echoes_of(10, 200, 0.5, &set).iter().map(|n| n.1).collect();
    assert_eq!(keys, [10, 12, 24]);
    let set = [(1, 2.0 / 32.0), (5, 0.0), (8, 0.0)];
    let keys: Vec<_> = echoes_of(10, 200, 0.5, &set).iter().map(|n| n.1).collect();
    assert_eq!(keys, [10, 0, 0]);
}

/// vel₁ = clamp(clamp(vel + VO, 0, 1)·VD, 0, 1), velₖ = clamp(velₖ₋₁·VD, 0, 1); VO is −0.9 … 0 … +1, VD = 1 + the same
/// map, 0.1 … 2.
#[test]
fn the_echo_velocity_adds_the_offset_and_multiplies_by_the_drift() {
    // VO −0.45 (p6 = 0.25), VD 1.5 (p7 = 0.75): 1 − 0.45 = 0.55, ×1.5 = 0.825 (byte 210), then 1.24 clamps to 1.
    let set = [(1, 2.0 / 32.0), (6, 0.25), (7, 0.75)];
    let velocities: Vec<_> = echoes(255, 0.5, &set).iter().map(|n| n.2).collect();
    assert_eq!(velocities, [255, 210, 255]);
    // VO +1 (p6 = 1) saturates the repeats; the played note keeps its 100.
    let set = [(1, 2.0 / 32.0), (6, 1.0)];
    let velocities: Vec<_> = echoes(100, 0.5, &set).iter().map(|n| n.2).collect();
    assert_eq!(velocities, [100, 255, 255]);
}

/// The chain stops after a repeat whose velocity is ≤ 1/127; that repeat itself is still emitted.
#[test]
fn the_echo_chain_stops_after_a_repeat_at_or_below_1_127() {
    // VD 0.1 (p7 = 0): 200/255 → 0.078 (byte 20) → 0.0078 (≤ 1/127, byte 2, the last); the third is never scheduled.
    let set = [(1, 3.0 / 32.0), (7, 0.0)];
    let velocities: Vec<_> = echoes(200, 0.5, &set).iter().map(|n| n.2).collect();
    assert_eq!(velocities, [200, 20, 2]);
}

/// Gaps: D to repeat 1, then gₖ = f32(TD^(k+1)·D) from repeat k to k + 1, TD = 0.7 + 0.6·v9: D, D·TD², D·TD³, … (TD¹
/// is never used). Repeat k + 1 is at tₖ + floor(gₖ).
#[test]
fn the_echo_gaps_are_d_then_d_times_td_squared_then_cubed() {
    for v9 in [1.0, 0.0, 0.3] {
        let notes = echoes(200, 0.5, &[(1, 3.0 / 32.0), (9, v9)]);
        let starts: Vec<_> = notes.iter().map(|n| frame(n.0)).collect();
        // TD² and TD³ by repeated squaring in double, as the law has it.
        let td = f64::from((0.6 * v9 + 0.7_f32).clamp(0.7, 1.3));
        let gap = |power: f64| (power * 24_000.0) as f32 as f64;
        let second = 24_000 + gap(td * td).floor() as u64;
        let third = second + gap(td * (td * td)).floor() as u64;
        assert_eq!(starts, [0, 24_000, second, third], "{v9}");
    }
}

/// Delay Rand. (v11): gₖ = max(f32((1 + v11·u)·gₖ), 0), u uniform in [−1, 1) per gap; repeat 1 is never randomized.
#[test]
fn delay_rand_spreads_each_later_gap_within_its_bounds() {
    // 40 notes 8 beats apart (each its own random draws), a 1-beat delay and Delay Rand. 50 %: repeat 1 one beat after
    // its note, every later gap within 24000·(1 ± 0.5) frames.
    let mut params = echo_params(3.0, false);
    params[5] = 0.75;
    params[11] = 0.5;
    let mut input = Vec::new();
    for note in 0..40 {
        input.push(InputNoteEvent::on(8.0 * f64::from(note), 60, 100));
        input.push(InputNoteEvent::off(8.0 * f64::from(note) + 0.25, 60));
    }
    let repeats: Vec<_> = (echo_ons(&params, &input).into_iter())
        .filter(|on| on.1 == 72)
        .map(|on| on.0)
        .collect();
    let mut seconds = Vec::new();
    for (note, chain) in repeats.chunks(3).enumerate() {
        assert_eq!(chain[0], 192_000 * note as u64 + 24_000, "note {note}");
        for pair in chain.windows(2) {
            assert!(
                (12_000..=36_000).contains(&(pair[1] - pair[0])),
                "note {note}: {chain:?}"
            );
        }
        seconds.push(chain[1]);
    }
    assert_eq!(seconds.len(), 40);
    // Repeat 2 is 24000 + 12000 … 36000 frames after its note: the draws reach both ends of that range.
    let spread: Vec<_> = seconds
        .iter()
        .enumerate()
        .map(|(i, s)| s - 192_000 * i as u64)
        .collect();
    assert!(
        spread.iter().any(|&at| at < 40_000) && spread.iter().any(|&at| at > 56_000),
        "{spread:?}"
    );
}

/// Length Lₖ in frames: prev = the played note's held length (the gap before the repeat while it is still held), then
/// Lₖ₋₁; L = trunc(fma_f32(LD, prev, 0.5)), LD 0.5 … 2, at most trunc(the gap before it), and at most
/// max(trunc(gₖ), 1) − 1 when another repeat follows.
#[test]
fn the_echo_length_follows_the_played_note_and_is_clamped_by_the_gaps() {
    let lengths = |held: f64, v10: f32, repetitions: f32| -> Vec<u64> {
        let mut params = echo_params(repetitions, false);
        params[5] = 0.75;
        params[10] = v10;
        let input = [
            InputNoteEvent::on(0.0, 60, 100),
            InputNoteEvent::off(held, 60),
        ];
        let output = echo_run(&params, &input);
        let mut lengths = Vec::new();
        for on in output
            .iter()
            .filter(|e| e.kind == NoteKind::On && e.pitch != 60)
        {
            let off = output
                .iter()
                .find(|e| e.kind == NoteKind::Off && e.pitch == on.pitch && e.beat > on.beat);
            lengths.push(frame(off.unwrap().beat) - frame(on.beat));
        }
        lengths
    };
    // Released at 6000 frames (¼ beat), LD 0.5 (v10 = 0): 3000, then 1500.
    assert_eq!(lengths(0.25, 0.0, 2.0), [3_000, 1_500]);
    // Released at 375 frames (1/64 beat): 187.5 rounds half up to 188.
    assert_eq!(lengths(1.0 / 64.0, 0.0, 1.0), [188]);
    // LD 2 (v10 = 1) from ½ beat: 24000, the gap's whole length when no repeat follows, one frame less when one does.
    assert_eq!(lengths(0.5, 1.0, 1.0), [24_000]);
    assert_eq!(lengths(0.5, 1.0, 2.0), [23_999, 24_000]);
    // Still held at repeat 1 (3 beats), LD 0.75: prev is the gap D (18000), then L₁ (13500).
    assert_eq!(lengths(3.0, 0.25, 2.0), [18_000, 13_500]);
}

/// No further repeat is scheduled once Lₖ is below one tick, trunc(fs·0.46875/BPM) = 187 frames here.
#[test]
fn the_echo_chain_stops_after_a_repeat_shorter_than_a_tick() {
    // Released at 375 frames, LD 0.5: 188 (still a tick), then 94, the last.
    let mut params = echo_params(5.0, false);
    params[5] = 0.75;
    params[10] = 0.0;
    let input = [
        InputNoteEvent::on(0.0, 60, 100),
        InputNoteEvent::off(1.0 / 64.0, 60),
    ];
    let ons = echo_ons(&params, &input);
    assert_eq!(ons, [(0, 60), (24_000, 72), (48_000, 72)]);
}

/// Pan is bipolar: pan₁ = clamp(pan + PnO, ±1), panₖ = clamp(panₖ₋₁ + PnD, ±1), PnO = 2·v12 − 1, PnD = v13 − 0.5, so the
/// drift first shows on repeat 2. Ping-Pong (v14 > 0.5) negates the emitted pan of repeats 2, 4, …; repeat 1 keeps its
/// sign.
#[test]
fn the_echo_pan_is_bipolar_and_ping_pong_flips_the_even_repeats() {
    // From 0.9 (+0.8): Offset +1 clamps to +1 (1.0), then Drift −0.5 a step: +0.5 (0.75), 0 (0.5).
    let set = [(1, 3.0 / 32.0), (12, 1.0), (13, 0.0)];
    let pans: Vec<_> = echoes(200, 0.9, &set).iter().map(|n| n.3).collect();
    assert_eq!(pans, [0.9, 1.0, 0.75, 0.5]);
    let ping_pong = [set[0], set[1], set[2], (14, 1.0)];
    let pans: Vec<_> = echoes(200, 0.9, &ping_pong).iter().map(|n| n.3).collect();
    assert_eq!(pans, [0.9, 1.0, 0.25, 0.5]);
    // Ping-Pong is on above 0.5 only.
    let half = [set[0], set[1], set[2], (14, 0.5)];
    let pans: Vec<_> = echoes(200, 0.9, &half).iter().map(|n| n.3).collect();
    assert_eq!(pans, [0.9, 1.0, 0.75, 0.5]);
}

/// A repeat whose gap ends inside the block the previous repeat was processed in is processed in the next
/// block before its start, and its note-on is dropped. Its note-off and the later repeats are still scheduled, so a
/// note-off with no note-on follows; a later note effect passes it on.
#[test]
fn a_repeat_due_before_its_block_loses_its_note_on_but_keeps_its_note_off() {
    // Sync Off, p2 = 0: D = 750 frames; Time Drift 0 %: every gap 750. Pitch Offset +12. The note is held 375 frames.
    // Repeat 1 at 1774 (the block quirk), 2524 and 3274 sound; 4024 falls in 3274's block (3072 … 4096), so its note-on
    // is dropped, and so is 4774's (in 4096 … 5120, where 4024 was processed). Each ends 375 frames on.
    let mut params = echo_params(5.0, false);
    params[2] = 0.0;
    params[4] = 0.0;
    params[5] = 0.75;
    let input = [
        InputNoteEvent::on(0.0, 60, 100),
        InputNoteEvent::off(1.0 / 64.0, 60),
    ];
    let transpose = module(48, 3, vec![1.0, 0.5, 13.0 / 24.0]);
    for modules in [vec![], vec![transpose]] {
        let mut rack = vec![module(47, 2, params.clone())];
        rack.extend(modules.iter().cloned());
        let shift = modules.len() as i16;
        let output = NoteEffectRack::new(rack)
            .unwrap()
            .process_events(0.0, 2.0, 120.0, &input)
            .unwrap();
        let events = |kind: NoteKind| -> Vec<u64> {
            (output.iter())
                .filter(|e| e.kind == kind && e.pitch == 72 + shift)
                .map(|e| frame(e.beat))
                .collect()
        };
        assert_eq!(events(NoteKind::On), [1_774, 2_524, 3_274], "{shift}");
        assert_eq!(
            events(NoteKind::Off),
            [2_149, 2_899, 3_649, 4_399, 5_149],
            "{shift}"
        );
    }
}

/// An echo whose key the input holds is not emitted, its note-off included. A repeat whose note-on is dropped
/// still emits no lone note-off while the key is held, so it never cuts the played note.
#[test]
fn a_dropped_repeat_of_a_held_key_emits_no_note_off() {
    // PO = PD = 0, Sync Off with p2 = 0 (D = 750), Time Drift 0 %, 8 repeats; key 60 held for 1 beat. Every repeat
    // lands while it is held, and from 4024 on they are dropped repeats too.
    let mut params = echo_params(8.0, false);
    params[2] = 0.0;
    params[4] = 0.0;
    let events: Vec<_> = (echo_run(&params, &note()).iter())
        .filter(|e| e.pitch == 60)
        .map(|e| (e.kind, frame(e.beat)))
        .collect();
    assert_eq!(events, [(NoteKind::On, 0), (NoteKind::Off, 24_000)]);
}

/// R, PO and PD are llroundf, half away from zero, off the f32(k/n) grid too: 32·v1 = 2.5 gives 3 repeats, 48·v5 − 24 =
/// 0.5 gives PO +1, and v8 = 0.5 − 1/48 (24·v8 − 12 = −0.5) gives PD −1. Truncation would give 2, 0 and 0.
#[test]
fn echo_steps_round_half_away_from_zero_off_the_grid() {
    let set = [(1, 2.5 / 32.0), (5, 24.5 / 48.0), (8, 0.5 - 1.0 / 48.0)];
    let keys: Vec<_> = echoes(200, 0.5, &set).iter().map(|n| n.1).collect();
    assert_eq!(keys, [60, 60, 59, 58]);
}

/// vel₁ = clamp(clamp(vel + VO, 0, 1)·VD, 0, 1): 1 + 0.5 (p6 = 0.75) clamps to 1 before × 0.55 (p7 = 0.25), giving
/// 0.55 (byte 140) and 0.3025 (77). Without the inner clamp: 210 and 116.
#[test]
fn the_echo_velocity_clamps_vel_plus_offset_before_the_drift() {
    let set = [(1, 2.0 / 32.0), (6, 0.75), (7, 0.25)];
    let velocities: Vec<_> = echoes(255, 0.5, &set).iter().map(|n| n.2).collect();
    assert_eq!(velocities, [255, 140, 77]);
}

/// PnO = 2·v12 − 1 and PnD = v13 − 0.5 at full size, unsaturated: from pan 0.25 (−0.5), PnO +0.25 (v12 = 0.625) gives
/// −0.25 (0.375), then PnD +0.125 (v13 = 0.625) gives −0.125 (0.4375).
#[test]
fn the_echo_pan_offset_and_drift_apply_at_full_size() {
    let set = [(1, 2.0 / 32.0), (12, 0.625), (13, 0.625)];
    let pans: Vec<_> = echoes(200, 0.25, &set).iter().map(|n| n.3).collect();
    assert_eq!(pans, [0.25, 0.375, 0.4375]);
}

/// The velocity stop is ≤ 1/127: byte 3 with Vel. Drift v7 = 0.31627294 makes vel₁ exactly the f32 1/127, so repeat
/// 1 is the last; with the next f32 of v7, vel₁ is just above and repeat 2 (below) is the last.
#[test]
fn the_velocity_stop_includes_exactly_1_127() {
    let count = |v7: f32| echoes(3, 0.5, &[(1, 4.0 / 32.0), (7, v7)]).len();
    assert_eq!(count(0.316_272_94), 2);
    assert_eq!(count(0.316_272_97), 3);
}

/// The tick stop: no repeat after one shorter than trunc(fs·0.46875/BPM) = trunc(187.5) = 187 frames. A note held 187
/// frames (LD 1, so L = 187) keeps the chain going; 186 ends it after repeat 1. An untruncated tick (187.5) or a ≤
/// would end both.
#[test]
fn the_tick_stop_is_below_the_truncated_tick() {
    let repeats = |held: u64| {
        let input = [
            InputNoteEvent::on(0.0, 60, 100),
            InputNoteEvent::off(at(held), 60),
        ];
        echo_ons(&echo_params(3.0, false), &input).len() - 1
    };
    assert_eq!(repeats(187), 3);
    assert_eq!(repeats(186), 1);
}

/// Repeat k + 1 sounds when tₖ + gₖ is at or after the start of the block after repeat k's block. D = 750 (Sync Off,
/// p2 = 0): from frame 548, repeat 1 is at 1298 (block 1024 … 2048), so repeat 2 is due at exactly 2048 and sounds;
/// from 547 it is due at 2047 and loses its note-on.
#[test]
fn a_repeat_due_exactly_at_the_next_block_start_sounds() {
    let mut params = echo_params(2.0, false);
    params[2] = 0.0;
    params[4] = 0.0;
    params[5] = 0.75;
    let run = |start: u64| {
        let input = [
            InputNoteEvent::on(at(start), 60, 100),
            InputNoteEvent::off(at(start + 300), 60),
        ];
        echo_ons(&params, &input)
    };
    assert_eq!(run(548), [(548, 60), (1_298, 72), (2_048, 72)]);
    assert_eq!(run(547), [(547, 60), (1_297, 72)]);
}

/// A dropped repeat sits at tₖ + ceil(gₖ), a sounding one at tₖ + floor(gₖ); its note-off is Lₖ later. D = 750, Time
/// Drift −15 % (v9 = 0.25, TD = 0.85), a note held 300 frames: repeat 1 at 1774; g₁ = 541.875, so repeat 2 sounds at
/// 2315; g₂ = 460.59…, so repeat 3 is due at 2775.6, inside 2315's block (2048 … 3072): it is dropped to 2776 and its
/// note-off is at 3076.
#[test]
fn a_dropped_repeat_sits_at_the_ceiling_of_a_fractional_gap() {
    let mut params = echo_params(3.0, false);
    params[2] = 0.0;
    params[4] = 0.0;
    params[5] = 0.75;
    params[9] = 0.25;
    let input = [
        InputNoteEvent::on(0.0, 60, 100),
        InputNoteEvent::off(at(300), 60),
    ];
    let output = echo_run(&params, &input);
    let echo = |kind: NoteKind| -> Vec<u64> {
        (output.iter())
            .filter(|e| e.kind == kind && e.pitch == 72)
            .map(|e| frame(e.beat))
            .collect()
    };
    assert_eq!(echo(NoteKind::On), [1_774, 2_315]);
    assert_eq!(echo(NoteKind::Off), [2_074, 2_615, 3_076]);
}

/// A zero-length repeat (Lₖ = 0) ends on its own frame, its note-off after its note-on. A note held 0 frames (on and
/// off both on frame 10) makes L₁ = 0, and the tick rule then ends the chain.
#[test]
fn a_zero_length_repeat_ends_on_its_own_frame_after_its_note_on() {
    let mut params = echo_params(2.0, false);
    params[5] = 0.75;
    let input = [
        InputNoteEvent::on(at(10), 60, 100),
        InputNoteEvent::off(9.6 / 24_000.0, 60),
    ];
    let echo: Vec<_> = (echo_run(&params, &input).iter())
        .filter(|e| e.pitch == 72)
        .map(|e| (e.kind, frame(e.beat)))
        .collect();
    assert_eq!(echo, [(NoteKind::On, 24_010), (NoteKind::Off, 24_010)]);
}

/// Note Echo with `params` (16) on `input` (before beat 400), at 120 BPM.
fn echo_run(params: &[f32], input: &[InputNoteEvent]) -> Vec<InputNoteEvent> {
    NoteEffectRack::new(vec![module(47, 2, params.to_vec())])
        .unwrap()
        .process_events(0.0, 400.0, 120.0, input)
        .unwrap()
}

/// [`echo_run`]'s note-ons as (frame, pitch).
fn echo_ons(params: &[f32], input: &[InputNoteEvent]) -> Vec<(u64, i16)> {
    echo_run(params, input)
        .iter()
        .filter(|event| event.kind == NoteKind::On)
        .map(|event| (frame(event.beat), event.pitch))
        .collect()
}

/// Note Echo at its defaults with `repetitions` and a 1-beat synced delay (or `1/2` beat with `half`).
fn echo_params(repetitions: f32, half: bool) -> Vec<f32> {
    let mut params = vec![0.5; 16];
    params[0] = 1.0;
    params[1] = repetitions / 32.0;
    let beats = if half { 0.5 } else { 1.0 };
    params[3] = ((beats - 1.0 / 32.0) / (8.0 - 1.0 / 32.0_f64)).sqrt() as f32;
    params[4] = 1.0;
    params[11] = 0.0;
    params
}

/// An echo whose key is held by the input is not emitted; the chain goes on.
#[test]
fn an_echo_of_a_held_key_is_not_emitted() {
    // Every half beat: 0.5 falls while the played note (until beat 1) is held; 1.0 and 1.5 sound.
    let ons = echo_ons(&echo_params(3.0, true), &note());
    assert_eq!(ons, [(0, 60), (24_000, 60), (36_000, 60)]);
    // Another key is not held.
    let mut params = echo_params(3.0, true);
    params[5] = 0.75;
    let ons = echo_ons(&params, &note());
    assert_eq!(ons, [(0, 60), (12_000, 72), (24_000, 72), (36_000, 72)]);
}

/// An echo within ±1 sample frame of an input note-on of the same key is dropped; input notes are never removed.
#[test]
fn an_echo_on_an_input_note_on_of_its_key_is_dropped() {
    // 120 BPM at the rack's 48 kHz: a frame is 1/24000 beat.
    let frame = 1.0 / 24_000.0;
    let short = |second: f64| {
        [
            InputNoteEvent::on(0.0, 60, 100),
            InputNoteEvent::off(0.25, 60),
            InputNoteEvent::on(second, 60, 100),
            InputNoteEvent::off(second + 0.25, 60),
        ]
    };
    let params = echo_params(1.0, false);
    // The echo is at frame 24000: a second note-on at 24000 or 24001 drops it, one at 24002 does not.
    for (second, at) in [(1.0, 24_000), (1.0 + 0.9 * frame, 24_001)] {
        let ons = echo_ons(&params, &short(second));
        assert_eq!(ons, [(0, 60), (at, 60), (at + 24_000, 60)], "{second}");
    }
    let ons = echo_ons(&params, &short(1.0 + 1.9 * frame));
    assert_eq!(ons, [(0, 60), (24_000, 60), (24_002, 60), (48_002, 60)]);
}

/// A note-on at velocity 0 is a note-off.
#[test]
fn a_note_on_at_velocity_zero_is_a_note_off() {
    let input = [
        InputNoteEvent::on(0.0, 60, 100),
        InputNoteEvent::on(0.5, 60, 0),
    ];
    let output = run(vec![module(47, 2, echo_params(0.0, false))], &input);
    let events: Vec<_> = output
        .iter()
        .map(|e| (e.kind, e.beat, e.velocity))
        .collect();
    assert_eq!(events, [(NoteKind::On, 0.0, 100), (NoteKind::Off, 0.5, 0)]);
}

/// Sync is on at v > 0.5 and Enable at v ≥ 0.5.
#[test]
fn echo_sync_is_on_above_half_and_enable_from_half() {
    // Sync 0.5 is Off: the delay is p2's 1/32 + (8 − 1/32)/4 beats, not p3's half beat.
    let mut params = echo_params(1.0, true);
    params[0] = 0.5;
    params[2] = 0.5;
    params[4] = 0.5;
    params[5] = 0.75;
    let ons = echo_ons(&params, &note());
    assert_eq!(ons, [(0, 60), (48_562, 72)]);
}

/// Enable
/// switched Off drops the repeats still to come.
#[test]
fn echo_enable_off_drops_the_repeats_still_to_come() {
    let mut params = vec![0.5; 16];
    params[0] = 1.0;
    params[1] = 3.0 / 32.0;
    params[3] = ((1.0 - 1.0 / 32.0) / (8.0 - 1.0 / 32.0_f64)).sqrt() as f32;
    params[4] = 1.0;
    params[11] = 0.0;
    params[14] = 0.0;
    let mut echo = module(47, 2, params);
    echo.changes = vec![(1.5, 0, 0.0)];
    let output = NoteEffectRack::new(vec![echo])
        .unwrap()
        .process_events(0.0, 2.0, 120.0, &note())
        .unwrap();
    let ons: Vec<_> = output
        .iter()
        .filter(|event| event.kind == NoteKind::On)
        .map(|event| event.beat)
        .collect();
    assert_eq!(ons.len(), 2, "{ons:?}");
    assert!(ons[1] < 1.5);
    // The sounding repeat's note-off (due at 2) is flushed where Enable goes Off.
    let offs: Vec<_> = (output.iter())
        .filter(|e| e.kind == NoteKind::Off)
        .map(|e| e.beat)
        .collect();
    assert_eq!(offs, [1.0, 1.5]);
}

#[test]
fn randomizer_is_deterministic_and_keeps_note_off_pitch() {
    let params = vec![1.0, 0.5, 0.5, 0.0, 1.0];
    let effects = vec![module(49, 42, params)];
    let a = run(effects.clone(), &note());
    let b = run(effects, &note());
    assert_eq!(a, b);
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].pitch, a[1].pitch);
    assert_eq!(a[1].beat, 1.0);
    assert!((0..=127).contains(&a[0].pitch));
}

/// Gate is the chance a note plays (#31): of 200 notes about half
/// play at 50 %, the same ones each time and at their own lengths; all play at 100 %, none at 0 %.
#[test]
fn the_randomizer_gate_is_the_chance_a_note_plays() {
    let input: Vec<_> = (0..200)
        .flat_map(|k| {
            let beat = f64::from(k) / 2.0;
            [
                InputNoteEvent::on(beat, 60, 100),
                InputNoteEvent::off(beat + 0.25, 60),
            ]
        })
        .collect();
    let play = |gate| {
        run_until(
            vec![module(49, 7, vec![1.0, 0.0, 0.0, 0.0, gate])],
            &input,
            101.0,
        )
    };
    let half = play(0.5);
    assert_eq!(half, play(0.5));
    let ons = pitches(&half).len();
    assert!((80..=120).contains(&ons), "{ons} of 200 played");
    assert_eq!(half.len(), 2 * ons);
    assert!(
        half.chunks(2).all(|n| n[1].beat - n[0].beat == 0.25),
        "{half:?}"
    );
    assert_eq!(pitches(&play(1.0)).len(), 200);
    assert_eq!(play(0.0), []);
}

#[test]
fn pan_survives_ordered_effects_and_echo_ping_pongs() {
    let mut echo = vec![0.5; 16];
    echo[0] = 1.0;
    echo[1] = 2.0 / 32.0;
    echo[3] = 0.2425;
    echo[4] = 1.0;
    echo[11] = 0.0;
    echo[14] = 1.0;
    let input = [
        InputNoteEvent::on(0.0, 60, 100).with_pan(0.2),
        // Shorter than the half-beat delay, so the held note does not swallow the first repeat.
        InputNoteEvent::off(0.25, 60),
    ];
    let output = run(
        vec![module(48, 2, vec![1.0, 0.5, 0.5]), module(47, 3, echo)],
        &input,
    );
    let on: Vec<_> = output
        .iter()
        .filter(|event| event.kind == NoteKind::On)
        .collect();
    // Repeat 1 keeps the pan, Ping-Pong flips repeat 2.
    let pans: Vec<_> = on.iter().map(|event| event.pan).collect();
    for (pan, expected) in pans.iter().zip([0.2, 0.2, 0.8]) {
        assert!((pan - expected).abs() < 1e-6, "{pans:?}");
    }
    assert_eq!(pans.len(), 3);
    assert_eq!(
        output
            .iter()
            .filter(|event| event.kind == NoteKind::Off)
            .count(),
        3
    );
}

#[test]
fn randomizer_changes_pan_deterministically() {
    let input = [
        InputNoteEvent::on(0.0, 60, 100).with_pan(0.5),
        InputNoteEvent::off(1.0, 60),
    ];
    let effect = module(49, 9, vec![1.0, 0.0, 0.0, 1.0, 1.0]);
    let a = run(vec![effect.clone()], &input);
    let b = run(vec![effect], &input);
    assert_eq!(a, b);
    assert_ne!(a[0].pan, 0.5);
    assert_eq!(a[0].pan, a[1].pan);
    assert!((0.0..=1.0).contains(&a[0].pan));
}

#[test]
fn arpeggiator_uses_sequencer_and_releases_last_note() {
    let output = run(vec![module(51, 2, arp_params())], &note());
    let on = output.iter().filter(|e| e.kind == NoteKind::On).count();
    let off = output.iter().filter(|e| e.kind == NoteKind::Off).count();
    assert_eq!(on, 2);
    assert_eq!(off, on);
}

#[test]
fn full_arp_params_set_each_engine_rate() {
    let mut params = arp_params();
    params[30] = 1.0;
    params[31] = 8.0 / 14.0;
    let input = [
        InputNoteEvent::on(0.0, 60, 100),
        InputNoteEvent::off(1.5, 60),
    ];
    let beats: Vec<_> = run(vec![module(51, 2, params)], &input)
        .iter()
        .filter(|event| event.kind == NoteKind::On)
        .map(|event| event.beat)
        .collect();
    assert_eq!(beats, [0.0, 0.0, 0.5, 1.0, 1.0]);
}

#[test]
fn latched_arp_stops_at_requested_end_and_releases_notes() {
    let mut params = arp_params();
    params[6] = 1.0;
    let output = run(vec![module(51, 2, params)], &note());
    let on = output
        .iter()
        .filter(|event| event.kind == NoteKind::On)
        .count();
    let off = output
        .iter()
        .filter(|event| event.kind == NoteKind::Off)
        .count();
    assert_eq!(on, 4);
    assert_eq!(off, on);
    assert!(
        output
            .iter()
            .filter(|event| event.kind == NoteKind::On)
            .all(|event| event.beat < 2.0)
    );
}

#[test]
fn echo_tail_feeds_later_arpeggiator() {
    let mut echo = vec![0.5; 16];
    echo[0] = 1.0;
    echo[1] = 1.0 / 32.0;
    echo[3] = 1.0;
    echo[4] = 1.0;
    echo[8] = 1.0;
    echo[11] = 0.0;
    let output = run(
        vec![module(47, 2, echo), module(51, 3, arp_params())],
        &note(),
    );
    assert!(
        output
            .iter()
            .any(|event| event.kind == NoteKind::On && event.pitch == 72 && event.beat >= 8.0)
    );
    assert_eq!(
        output
            .iter()
            .filter(|event| event.kind == NoteKind::On)
            .count(),
        output
            .iter()
            .filter(|event| event.kind == NoteKind::Off)
            .count()
    );
}

/// Arpeggiator 1 stepping Up once a beat (Rate 1) over C and E, its sequence restarted every bar (Retrig `1 bar`).
fn bar_retrigger_arp() -> NoteEffectModule {
    let mut params = arp_params();
    params[1] = 4.0 / 7.0;
    params[10] = 2.0 / 23.0;
    params[18] = 7.0 / 11.0;
    module(51, 2, params)
}

/// The bar of the Arpeggiator's bar retrigger is the song's beats per bar.
#[test]
fn the_arpeggiator_bar_retrigger_follows_the_beats_per_bar() {
    let input = [
        InputNoteEvent::on(0.0, 60, 100),
        InputNoteEvent::on(0.0, 64, 100),
        InputNoteEvent::off(6.0, 60),
        InputNoteEvent::off(6.0, 64),
    ];
    let played = |bar_beats: f64| {
        let mut rack = NoteEffectRack::new(vec![bar_retrigger_arp()]).unwrap();
        rack.bar_beats = bar_beats;
        pitches(&rack.process_events(0.0, 8.0, 120.0, &input).unwrap())
    };
    assert_eq!(played(4.0), [60, 64, 60, 64, 60, 64]);
    // The sequence starts over at beat 3.
    assert_eq!(played(3.0), [60, 64, 60, 60, 64, 60]);
}

#[test]
fn malformed_input_and_module_values_fail_without_partial_output() {
    assert_eq!(
        NoteEffectRack::new(vec![module(48, 2, vec![1.0, f32::NAN, 0.5])]),
        Err(NoteEffectError::InvalidParameter)
    );
    for change in [(0.0, 3, 0.5), (0.0, 1, f32::NAN), (f64::NAN, 1, 0.5)] {
        let mut transpose = module(48, 2, vec![1.0, 0.5, 0.5]);
        transpose.changes = vec![change];
        assert_eq!(
            NoteEffectRack::new(vec![transpose]),
            Err(NoteEffectError::InvalidParameter),
            "{change:?}"
        );
    }
    let mut transpose = module(48, 2, vec![1.0, 0.5, 0.5]);
    transpose.changes = vec![(2.0, 1, 0.5), (1.0, 1, 0.5)];
    assert_eq!(
        NoteEffectRack::new(vec![transpose]),
        Err(NoteEffectError::InvalidParameter),
        "out of beat order"
    );
    let rack = NoteEffectRack::new(vec![]).unwrap();
    assert_eq!(
        rack.process_events(0.0, 2.0, 120.0, &[InputNoteEvent::off(0.0, 60)]),
        Err(NoteEffectError::UnpairedNote(60))
    );
}

/// Live keys: `input` played key by key through a [`LiveNoteChain`] of `modules` at 120
/// BPM and 48 kHz, each note-off as its key lifts, as (frame, on, key, velocity byte) of what leaves the last note
/// effect, sorted, with its pan in thousandths.
fn live(modules: Vec<NoteEffectModule>, input: &[InputNoteEvent]) -> Vec<(u64, bool, u8, u8, i32)> {
    const FRAMES_PER_BEAT: f64 = 24_000.0;
    let last = modules.len();
    let mut chain = LiveNoteChain::new(modules, 4.0, 48_000.0, 120.0, 4096).unwrap();
    let mut out = Vec::new();
    let mut played = Vec::new();
    let mut keys = input.iter().peekable();
    let mut now = 0;
    loop {
        while let Some(key) = keys.next_if(|key| (key.beat * FRAMES_PER_BEAT) as u64 <= now) {
            let pitch = key.pitch as u8;
            match key.kind {
                NoteKind::On => chain.note_on(now, pitch, f32::from(key.velocity) / 255.0, true),
                NoteKind::Off => chain.note_off(now, pitch),
            }
        }
        out.clear();
        chain.due(now, 0.0, &mut out);
        for event in out.iter().filter(|event| event.level == last) {
            let velocity = (event.velocity * 255.0).round() as u8;
            let velocity = if event.on { velocity } else { 0 };
            let pan = (event.pan * 1000.0).round() as i32;
            played.push((now, event.on, event.key, velocity, pan));
        }
        let key = keys.peek().map(|key| (key.beat * FRAMES_PER_BEAT) as u64);
        match chain.next_due().into_iter().chain(key).min() {
            Some(next) => {
                assert!(next > now, "the chain is due again at {next}, now {now}");
                now = next;
            }
            None => break,
        }
    }
    played.sort_by_key(|&(frame, on, key, ..)| (frame, on, key));
    played
}

/// What the prepared chain plays for `input`, in the form [`live`] gives.
fn prepared(
    modules: Vec<NoteEffectModule>,
    input: &[InputNoteEvent],
) -> Vec<(u64, bool, u8, u8, i32)> {
    let events = run_until(modules, input, 16.0);
    let mut played: Vec<_> = (events.iter())
        .map(|e| {
            let on = e.kind == NoteKind::On;
            let velocity = if on { e.velocity } else { 0 };
            let pan = (e.pan * 1000.0).round() as i32;
            (
                (e.beat * 24_000.0).ceil() as u64,
                on,
                e.pitch as u8,
                velocity,
                pan,
            )
        })
        .collect();
    played.sort_by_key(|&(frame, on, key, ..)| (frame, on, key));
    played
}

fn run_until(
    modules: Vec<NoteEffectModule>,
    events: &[InputNoteEvent],
    end: f64,
) -> Vec<InputNoteEvent> {
    NoteEffectRack::new(modules)
        .unwrap()
        .process_events(0.0, end, 120.0, events)
        .unwrap()
}

/// Live keys are routed exactly like sequenced notes: through a Transpose, a Note
/// Echo, the Arpeggiator (Up over two octaves), a Randomizer and a Transpose, a live chain plays what the prepared
/// chain plays for the same notes.
#[test]
fn the_live_chain_plays_what_the_prepared_chain_plays() {
    let mut echo = vec![0.5; 16];
    // Two repeats a quarter beat apart (Sync on), no Delay Rand., no Ping-Pong.
    (echo[0], echo[1], echo[4], echo[11], echo[14]) = (1.0, 2.0 / 32.0, 1.0, 0.0, 0.0);
    echo[3] = ((0.25 - 1.0 / 32.0) / (8.0 - 1.0 / 32.0_f64)).sqrt() as f32;
    let mut arp = arp_params();
    (arp[10], arp[11]) = (2.0 / 23.0, 1.0 / 3.0);
    let modules = vec![
        module(48, 2, vec![1.0, 5.0 / 8.0, 0.5]),
        module(47, 3, echo),
        module(51, 4, arp),
        // ±2 semitones, velocity ±30 %, pan ±0.2, full gate.
        module(49, 5, vec![1.0, 2.0 / 24.0, 0.3, 0.2, 1.0]),
        module(48, 6, vec![1.0, 0.5, 13.0 / 24.0]),
    ];
    let input = [
        InputNoteEvent::on(0.0, 48, 100),
        InputNoteEvent::on(0.5, 52, 90),
        InputNoteEvent::off(1.0, 48),
        InputNoteEvent::off(1.75, 52),
    ];
    let expected = prepared(modules.clone(), &input);
    assert!(expected.len() >= 8, "{expected:?}");
    assert_eq!(live(modules, &input), expected);
}

/// The live Note Echo follows the same law as the prepared one does: a key lifted before its first repeat and
/// one held through it (the lengths differ), repeats a semitone apart, a gap shorter than the 1024-frame block (a
/// repeat processed before its start loses its note-on, a note-off alone, which the Transpose below moves).
#[test]
fn the_live_note_echo_plays_the_prepared_series() {
    for (delay, time_drift) in [(0.0, 0.5), (0.2, 0.9), (0.1, 0.0)] {
        let mut echo = vec![0.5; 16];
        // Six repeats, Sync off at `delay`, Pitch Drift +1, no Delay Rand.
        (echo[0], echo[1], echo[2], echo[4]) = (1.0, 6.0 / 32.0, delay, 0.0);
        (echo[8], echo[9], echo[11]) = (13.0 / 24.0, time_drift, 0.0);
        let modules = vec![
            module(47, 3, echo),
            module(48, 4, vec![1.0, 0.5, 14.0 / 24.0]),
        ];
        let input = [
            InputNoteEvent::on(0.0, 60, 100),
            InputNoteEvent::off(0.01, 60),
            InputNoteEvent::on(1.0, 72, 120),
            InputNoteEvent::on(1.3, 50, 90),
            InputNoteEvent::off(1.4, 50),
            InputNoteEvent::off(3.0, 72),
        ];
        let expected = prepared(modules.clone(), &input);
        assert!(expected.len() >= 16, "{expected:?}");
        assert_eq!(
            live(modules, &input),
            expected,
            "delay {delay}, drift {time_drift}"
        );
    }
}

/// The traces of `modules` over beats 0–2 for `input`.
fn traced(
    modules: Vec<NoteEffectModule>,
    input: &[InputNoteEvent],
) -> (
    Vec<InputNoteEvent>,
    Vec<honeydew_audio::note_effects::NoteEffectTrace>,
) {
    let mut traces = Vec::new();
    let output = NoteEffectRack::new(modules)
        .unwrap()
        .process_events_traced(0.0, 2.0, 120.0, input, &mut traces)
        .unwrap();
    (output, traces)
}

#[test]
fn an_arpeggiators_trace_puts_its_playhead_on_the_steps_it_plays() {
    // A note held from beat 0.25 to 1.25 (Arp 1 at half-beat steps, Arp 2 off).
    use honeydew_audio::analysis::{arpeggiator, note_effect_state};
    let input = [
        InputNoteEvent::on(0.25, 60, 100),
        InputNoteEvent::off(1.25, 60),
    ];
    let (output, traces) = traced(vec![module(51, 2, arp_params())], &input);
    let [trace] = &traces[..] else {
        panic!("{traces:?}")
    };
    let at = |beat: f64, rolls: bool| {
        let mut slab = [0.0; 4];
        note_effect_state(trace, beat, rolls, &mut slab);
        arpeggiator(&slab).unwrap()
    };
    // Each note the arp plays starts on a whole step.
    for on in output.iter().filter(|e| e.kind == NoteKind::On) {
        let step = at(on.beat, true)[0].unwrap();
        assert!((step - step.round()).abs() < 1e-4, "{} at {step}", on.beat);
    }
    let step = at(1.0, true)[0].unwrap();
    assert!((step - 1.5).abs() < 1e-5, "{step}");
    assert_eq!(at(1.0, true)[1], None, "Arp 2 is off");
    assert_eq!(at(1.0, false)[0], None, "stopped");
    assert_eq!(at(0.1, true)[0], None, "nothing held yet");
    assert_eq!(at(1.5, true)[0], None, "released");
}

#[test]
fn a_note_echos_trace_is_pending_from_a_played_note_until_its_last_echo_ends() {
    use honeydew_audio::analysis::{note_echo, note_effect_state};
    use honeydew_audio::note_effects::NoteEffectTrace;
    let mut params = vec![0.5; 16];
    params[0] = 1.0;
    params[1] = 2.0 / 32.0;
    params[3] = 4.0 / 9.0;
    params[4] = 1.0;
    params[11] = 0.0;
    let (output, traces) = traced(vec![module(47, 2, params)], &note());
    let last = output.iter().map(|e| e.beat).fold(0.0, f64::max);
    let [NoteEffectTrace::Echo { pending, .. }] = &traces[..] else {
        panic!("{traces:?}")
    };
    assert_eq!(pending[..], [(0.0, last)]);
    let at = |beat: f64, rolls: bool| {
        let mut slab = [0.0];
        note_effect_state(&traces[0], beat, rolls, &mut slab);
        note_echo(&slab).unwrap()
    };
    assert!(at(0.0, true) && at(last - 0.01, true));
    assert!(!at(last, true) && !at(last + 1.0, true));
    assert!(!at(0.5, false), "stopped");
    // Without repeats nothing is pending.
    let mut none = vec![0.5; 16];
    (none[0], none[1]) = (1.0, 0.0);
    let (_, traces) = traced(vec![module(47, 2, none)], &note());
    assert!(matches!(&traces[..], [NoteEffectTrace::Echo { pending, .. }] if pending.is_empty()));
}

/// The note effects' steps are f32, like their texts. Transpose is the
/// clamped range map round(f32(f32(n·v) + lo)): at v = 1/16, 8v − 4 = −3.5 plays −4 octaves and 24v − 12 = −10.5 plays
/// −11 semitones (an f64 round-then-subtract played −3 and −10).
#[test]
fn transpose_rounds_its_f32_range_map_halves_away_from_zero() {
    let play = |octave: f32, semitone: f32| {
        pitches(&run(
            vec![module(48, 2, vec![1.0, octave, semitone])],
            &note(),
        ))
    };
    assert_eq!(play(0.0625, 0.5), [12]);
    assert_eq!(play(0.5, 0.0625), [49]);
}

/// The Randomizer's range is round(f32(24v)): at v = f32(2.5/24) the f32 product is exactly 2.5, a range of 3 (the f64
/// product, 2.4999999, rounded to 2).
#[test]
fn the_randomizer_range_rounds_the_f32_product() {
    let v = 5.0f32 / 48.0;
    let widest = (0..200)
        .flat_map(|id| {
            pitches(&run(
                vec![module(49, id, vec![1.0, v, 0.0, 0.0, 1.0])],
                &note(),
            ))
        })
        .map(|p| (p - 60).abs())
        .max();
    assert_eq!(widest, Some(3));
}

/// The Arpeggiator's menus read round(f32(n − 1)·v): Rate at v = f32(4.5/7) is item 5 (1/3, steps of 4/3 beat); the f64
/// product rounded to item 4 (1/4, one beat).
#[test]
fn the_arpeggiator_menus_round_the_f32_product() {
    let mut params = arp_params();
    params[1] = 9.0f32 / 14.0;
    let input = [
        InputNoteEvent::on(0.0, 60, 100),
        InputNoteEvent::off(3.5, 60),
    ];
    let output = NoteEffectRack::new(vec![module(51, 2, params)])
        .unwrap()
        .process_events(0.0, 4.0, 120.0, &input)
        .unwrap();
    let beats: Vec<_> = output
        .iter()
        .filter(|e| e.kind == NoteKind::On)
        .map(|e| e.beat)
        .collect();
    assert!((beats[1] - 4.0 / 3.0).abs() < 1e-9, "{beats:?}");
}

/// The live chain steps in f32 like the texts too: Transpose at v = 1/16 plays −4 octaves and −11 semitones, and the
/// Arpeggiator's Rate at v = f32(4.5/7) is item 5 (1/3, a step every 4/3 beat = 32000 frames).
#[test]
fn the_live_chain_steps_in_f32_like_the_texts() {
    let ons = |modules, input: &[InputNoteEvent]| -> Vec<(u64, u8)> {
        live(modules, input)
            .iter()
            .filter(|e| e.1)
            .map(|e| (e.0, e.2))
            .collect()
    };
    assert_eq!(
        ons(vec![module(48, 2, vec![1.0, 0.0625, 0.0625])], &note()),
        [(0, 1)]
    );
    let mut params = arp_params();
    params[1] = 9.0f32 / 14.0;
    let input = [
        InputNoteEvent::on(0.0, 60, 100),
        InputNoteEvent::off(3.5, 60),
    ];
    let frames: Vec<_> = ons(vec![module(51, 2, params)], &input)
        .iter()
        .map(|e| e.0)
        .collect();
    assert_eq!(frames[..2], [0, 32_000]);
}

/// Notes that start on one beat leave in their note-ons' order (a clip's order), not in the order they end nor by
/// pitch.
#[test]
fn notes_on_one_beat_keep_their_note_on_order() {
    let chord = |keys: [i16; 2]| {
        [
            InputNoteEvent::on(0.0, keys[0], 100),
            InputNoteEvent::on(0.0, keys[1], 100),
            InputNoteEvent::off(1.0, keys[1]),
            InputNoteEvent::off(1.5, keys[0]),
        ]
    };
    assert_eq!(pitches(&run(Vec::new(), &chord([69, 57]))), [69, 57]);
    assert_eq!(pitches(&run(Vec::new(), &chord([57, 69]))), [57, 69]);
}

/// Runs `chain` from frame `from` to `to` (not included): each event due, as (frame, level, on, key).
fn drive(chain: &mut LiveNoteChain, from: u64, to: u64, out: &mut Vec<(u64, usize, bool, u8)>) {
    let (mut now, mut events) = (from, Vec::new());
    while now < to {
        events.clear();
        chain.due(now, 0.0, &mut events);
        out.extend(events.iter().map(|e| (now, e.level, e.on, e.key)));
        now = chain
            .next_due()
            .filter(|&next| next > now)
            .map_or(to, |next| next.min(to));
    }
}

/// A live key the Randomizer drops (Gate 0 %) sounds nothing below it, and its lift leaves nothing held or due.
#[test]
fn a_live_key_the_randomizer_drops_sounds_nothing_and_sticks_nothing() {
    let mut chain = chain_of(vec![module(49, 7, vec![1.0, 0.0, 0.0, 0.0, 0.0])]);
    let mut out = Vec::new();
    chain.note_on(0, 60, 0.8, true);
    drive(&mut chain, 0, 10_000, &mut out);
    chain.note_off(10_000, 60);
    drive(&mut chain, 10_000, 20_000, &mut out);
    assert_eq!(out, [(0, 0, true, 60), (10_000, 0, false, 60)]);
    assert_eq!((chain.keys_down(), chain.next_due()), (0, None));
}

/// A live tempo reaches the live chain from the frame it is set. A synced Arpeggiator's steps follow the beats; an
/// unsynced one keeps its milliseconds, its beat interval recomputed; neither restarts its pattern.
#[test]
fn a_live_tempo_retimes_the_live_arpeggiator_from_then_on() {
    // Half a beat (Sync on), or 250 ms (Sync off): 12 000 frames at 120 BPM either way. From 50 000 frames (beat 2 1/12)
    // on, at 240 BPM, the steps keep the grid of the Arpeggiator's origin: beat 2.5, or (a beat a step now) beat 3.
    for (sync, first, after) in [(1.0, 55_000, 6_000), (0.0, 61_000, 12_000)] {
        let mut params = arp_params();
        (params[2], params[3]) = (200.0 / 950.0, sync);
        let mut chain =
            LiveNoteChain::new(vec![module(51, 2, params)], 4.0, 48_000.0, 120.0, 4096).unwrap();
        chain.note_on(0, 60, 0.8, true);
        let mut out = Vec::new();
        drive(&mut chain, 0, 50_000, &mut out);
        chain.set_tempo(50_000, 240.0);
        drive(&mut chain, 50_000, 150_000, &mut out);
        let ons: Vec<u64> = (out.iter())
            .filter(|e| e.1 == 1 && e.2)
            .map(|e| e.0)
            .collect();
        let near = |a: u64, b: u64| a.abs_diff(b) <= 1;
        assert!(
            (0..5).all(|k| near(ons[k], 12_000 * k as u64)),
            "sync {sync}: {ons:?}"
        );
        let later: Vec<u64> = ons.iter().copied().filter(|&on| on > 50_000).collect();
        assert!(
            later.len() >= 4 && near(later[0], first),
            "sync {sync}: {ons:?}"
        );
        assert!(
            later.windows(2).all(|w| near(w[1] - w[0], after)),
            "sync {sync}: {ons:?}"
        );
    }
}

/// Every note-on in `out` has its note-off, at the level it went out at.
fn assert_balanced(out: &[(u64, usize, bool, u8)]) {
    for level in 0..=3 {
        for key in 0..128 {
            let count = |on| {
                (out.iter())
                    .filter(|e| (e.1, e.2, e.3) == (level, on, key))
                    .count()
            };
            assert_eq!(
                count(true),
                count(false),
                "level {level}, key {key}: {out:?}"
            );
        }
    }
}

/// The note-ons in `out` at `level` from frame `from` on, as (frame, key).
fn ons_at(out: &[(u64, usize, bool, u8)], level: usize, from: u64) -> Vec<(u64, u8)> {
    (out.iter())
        .filter(|e| e.0 >= from && e.1 == level && e.2)
        .map(|e| (e.0, e.3))
        .collect()
}

/// A chain of `modules` at 120 BPM and 48 kHz.
fn chain_of(modules: Vec<NoteEffectModule>) -> LiveNoteChain {
    LiveNoteChain::new(modules, 4.0, 48_000.0, 120.0, 4096).unwrap()
}

/// Note Echo: `repeats` a beat apart (Sync on), no Delay Rand., no Ping-Pong.
fn echo_module(id: i32, repeats: f32) -> NoteEffectModule {
    let mut params = vec![0.5; 16];
    (params[0], params[1], params[4], params[11], params[14]) =
        (1.0, repeats / 32.0, 1.0, 0.0, 0.0);
    params[3] = (0.96875_f32 / 7.96875).sqrt();
    module(47, id, params)
}

/// The Arpeggiator, Up over two octaves, half a beat a step.
fn arp_module(id: i32) -> NoteEffectModule {
    let mut params = arp_params();
    (params[10], params[11]) = (2.0 / 23.0, 1.0 / 3.0);
    module(51, id, params)
}

/// Transpose an octave up.
fn octave_up(id: i32) -> NoteEffectModule {
    module(48, id, vec![1.0, 5.0 / 8.0, 0.5])
}

/// Moved modules keep their state: the chain built for a rack whose note effects moved takes over the running one.
/// The held key and what was made from it go on, the moved Arpeggiator (Up over two octaves) goes on with its pattern
/// where it stood, now below the Transpose, and once the key lifts every note-on has had its note-off at the level it
/// left.
#[test]
fn a_chain_whose_note_effects_moved_takes_over_the_notes_in_flight() {
    let (mut running, mut unmoved) = (
        chain_of(vec![arp_module(2), octave_up(3)]),
        chain_of(vec![arp_module(2), octave_up(3)]),
    );
    let (mut out, mut reference) = (Vec::new(), Vec::new());
    for (chain, out) in [(&mut running, &mut out), (&mut unmoved, &mut reference)] {
        chain.note_on(0, 60, 0.8, true);
        drive(chain, 0, 30_000, out);
    }
    drive(&mut unmoved, 30_000, 60_000, &mut reference);
    let mut moved = chain_of(vec![octave_up(3), arp_module(2)]);
    moved.take_over(&mut running, 30_000, |_, _, _| 1);
    drive(&mut moved, 30_000, 60_000, &mut out);
    // The Arpeggiator's steps at 36 000 and 48 000 frames, as it would have played them unmoved, now leave it last.
    assert_eq!(ons_at(&out, 2, 30_000), ons_at(&reference, 1, 30_000));
    assert_eq!(ons_at(&out, 2, 30_000).len(), 2, "{out:?}");
    moved.note_off(60_000, 60);
    drive(&mut moved, 60_000, 200_000, &mut out);
    assert_balanced(&out);
    assert!(moved.next_due().is_none(), "notes left in flight");
}

/// Note Echo moved below the Transpose with its repeats still to come keeps them: they leave it at its
/// new place, on the frames they had, no longer transposed.
#[test]
fn a_moved_note_echo_plays_its_pending_repeats_from_its_new_place() {
    let (mut running, mut unmoved) = (
        chain_of(vec![echo_module(2, 2.0), octave_up(3)]),
        chain_of(vec![echo_module(2, 2.0), octave_up(3)]),
    );
    let (mut out, mut reference) = (Vec::new(), Vec::new());
    for (chain, out) in [(&mut running, &mut out), (&mut unmoved, &mut reference)] {
        chain.note_on(0, 60, 0.8, true);
        drive(chain, 0, 1_000, out);
        chain.note_off(1_000, 60);
        drive(chain, 1_000, 5_000, out);
    }
    drive(&mut unmoved, 5_000, 100_000, &mut reference);
    let mut moved = chain_of(vec![octave_up(3), echo_module(2, 2.0)]);
    moved.take_over(&mut running, 5_000, |_, _, _| 1);
    drive(&mut moved, 5_000, 100_000, &mut out);
    let repeats = ons_at(&reference, 2, 5_000);
    assert_eq!(repeats.len(), 2, "{reference:?}");
    let untransposed: Vec<_> = repeats
        .iter()
        .map(|&(frame, key)| (frame, key - 12))
        .collect();
    assert_eq!(ons_at(&out, 2, 5_000), untransposed);
    assert_balanced(&out);
}

/// A note effect added under a held key releases nothing. The key and
/// what was made from it sound on, untouched by the new Arpeggiator, and their note-offs go where their note-ons went
/// (each carries its route); a new key goes through the new chain.
#[test]
fn a_note_effect_added_under_a_held_key_releases_nothing() {
    let mut running = chain_of(vec![octave_up(3)]);
    let mut out = Vec::new();
    running.note_on(0, 60, 0.8, true);
    drive(&mut running, 0, 10_000, &mut out);
    let mut added = chain_of(vec![octave_up(3), arp_module(2)]);
    let mut routes = Vec::new();
    added.take_over(&mut running, 10_000, |route, level, _| {
        routes.push((route, level));
        1 << level
    });
    assert_eq!(routes, [(None, 0), (None, 1)]);
    assert_eq!(added.keys_down(), 1 << 60);
    let mut events = Vec::new();
    added.due(20_000, 0.0, &mut events);
    assert!(events.is_empty(), "{events:?}");
    added.note_off(20_000, 60);
    added.due(20_000, 0.0, &mut events);
    let offs: Vec<_> = events.iter().map(|e| (e.on, e.key, e.route)).collect();
    assert_eq!(offs, [(false, 60, Some(1)), (false, 72, Some(2))]);
    added.note_on(30_000, 64, 0.8, true);
    drive(&mut added, 30_000, 31_000, &mut out);
    assert_eq!(ons_at(&out, 2, 30_000), [(30_000, 76)]);
}

/// A shorter chain takes over its surviving note effects. The removed
/// Arpeggiator is destroyed: its sounding step ends now (its pending note-off) and no step follows; the held key
/// sounds on untouched until it lifts.
#[test]
fn a_shorter_chain_takes_over_the_note_effects_left() {
    let mut running = chain_of(vec![arp_module(2), octave_up(3)]);
    let mut out = Vec::new();
    running.note_on(0, 60, 0.8, true);
    drive(&mut running, 0, 30_000, &mut out);
    let mut removed = chain_of(vec![octave_up(3)]);
    removed.take_over(&mut running, 30_000, |_, level, _| 1 << level);
    assert_eq!(removed.keys_down(), 1 << 60);
    drive(&mut removed, 30_000, 100_000, &mut out);
    let after: Vec<_> = (out.iter()).filter(|e| e.0 >= 30_000).copied().collect();
    // The step sounding at the edit (72 out of the Transpose, from the Arpeggiator's 60) ends at once; nothing else.
    assert_eq!(after, [(30_000, 1, false, 60), (30_000, 2, false, 72)]);
    removed.note_off(100_000, 60);
    drive(&mut removed, 100_000, 200_000, &mut out);
    assert_balanced(&out);
    assert!(removed.next_due().is_none(), "notes left in flight");
}

/// A tempo step or a drag's end: the chain built at the new tempo takes over at it, with no
/// `set_tempo`: the held Arpeggiator's next steps come at 240 BPM's half beat.
#[test]
fn a_chain_built_at_another_tempo_takes_over_at_it() {
    let mut running = chain_of(vec![arp_module(2)]);
    let mut out = Vec::new();
    running.note_on(0, 60, 0.8, true);
    drive(&mut running, 0, 30_000, &mut out);
    let mut faster = LiveNoteChain::new(vec![arp_module(2)], 4.0, 48_000.0, 240.0, 4096).unwrap();
    faster.take_over(&mut running, 30_000, |_, _, _| 1);
    drive(&mut faster, 30_000, 60_000, &mut out);
    let frames: Vec<u64> = ons_at(&out, 1, 30_000).iter().map(|e| e.0).collect();
    // Beat 1.25 at 30 000 frames: steps on beats 1.5, 2, 2.5 … at 6 000 frames a half beat from there.
    assert_eq!(frames[..3], [33_000, 39_000, 45_000], "{out:?}");
}

/// A Note Echo series still to be made when the tempo changes falls due at the new tempo, so its first
/// repeat (a beat after the held key) comes at 240 BPM's beat, not late at 120's.
#[test]
fn a_pending_note_echo_series_falls_due_at_a_new_tempo() {
    // A semitone up (Pitch Drift): a repeat of the held key itself is not emitted.
    let mut echo = echo_module(2, 1.0);
    echo.params[8] = 13.0 / 24.0;
    let mut chain = chain_of(vec![echo]);
    let mut out = Vec::new();
    chain.note_on(0, 60, 0.8, true);
    drive(&mut chain, 0, 100, &mut out);
    chain.set_tempo(100, 240.0);
    drive(&mut chain, 100, 30_000, &mut out);
    assert_eq!(ons_at(&out, 1, 100), [(12_000, 61)], "{out:?}");
}

/// A chain that takes over a plain one (256 notes in flight, no note effect) keeps the room built for
/// its own Note Echo: sixteen keys through 32 repeats each play every repeat, as a chain built so from the start does.
#[test]
fn a_chain_taking_over_a_plain_one_keeps_its_own_room() {
    // Each key held 250 frames (an echo shorter than a tick, 187 frames, would end its series).
    let play = |chain: &mut LiveNoteChain, out: &mut Vec<(u64, usize, bool, u8)>| {
        let mut at = 900;
        for k in 0..16 {
            let (on, key) = (1_000 + 300 * k, 40 + 2 * k as u8);
            drive(chain, at, on, out);
            chain.note_on(on, key, 0.8, true);
            drive(chain, on, on + 250, out);
            chain.note_off(on + 250, key);
            at = on + 250;
        }
        drive(chain, at, 2_000_000, out);
        out.iter().filter(|e| e.1 == 1 && e.2).count()
    };
    let mut plain = LiveNoteChain::new(Vec::new(), 4.0, 48_000.0, 120.0, 256).unwrap();
    drive(&mut plain, 0, 900, &mut Vec::new());
    let mut echoing = chain_of(vec![echo_module(2, 32.0)]);
    echoing.take_over(&mut plain, 900, |_, _, _| 1);
    let mut fresh = chain_of(vec![echo_module(2, 32.0)]);
    drive(&mut fresh, 0, 900, &mut Vec::new());
    let (mut out, mut reference) = (Vec::new(), Vec::new());
    let expected = play(&mut fresh, &mut reference);
    assert!(expected > 256, "{expected}");
    assert_eq!(play(&mut echoing, &mut out), expected);
}

/// A removed Note Echo makes no more repeats. Under a held key its series is still to be made (it
/// would be at the first repeat, and must not reach the gone stage); after a lifted key it is made, its repeats still to
/// start, and they are dropped. The notes sounding end as their keys do.
#[test]
fn a_removed_note_echo_makes_no_more_repeats() {
    let mut running = chain_of(vec![echo_module(2, 2.0), octave_up(3)]);
    let mut out = Vec::new();
    running.note_on(0, 60, 0.8, true);
    running.note_on(0, 64, 0.8, true);
    drive(&mut running, 0, 1_000, &mut out);
    running.note_off(1_000, 64);
    drive(&mut running, 1_000, 5_000, &mut out);
    let mut removed = chain_of(vec![octave_up(3)]);
    removed.take_over(&mut running, 5_000, |_, level, _| 1 << level);
    drive(&mut removed, 5_000, 100_000, &mut out);
    let later = |e: &&(u64, usize, bool, u8)| e.0 >= 5_000 && e.2;
    assert_eq!(out.iter().filter(later).count(), 0, "{out:?}");
    removed.note_off(100_000, 60);
    drive(&mut removed, 100_000, 200_000, &mut out);
    assert_balanced(&out);
    assert!(removed.next_due().is_none(), "notes left in flight");
}

/// The edit's params reach a kept Arpeggiator under a held key at once: its rate from half a beat to
/// a quarter (12 000 to 6 000 frames a step at 120 BPM), its pattern going on.
#[test]
fn a_kept_arpeggiator_takes_the_edits_params() {
    let mut running = chain_of(vec![arp_module(2)]);
    let mut out = Vec::new();
    running.note_on(0, 60, 0.8, true);
    drive(&mut running, 0, 30_000, &mut out);
    let mut faster = arp_module(2);
    faster.params[1] = 0.0;
    let mut edited = chain_of(vec![faster]);
    edited.take_over(&mut running, 30_000, |_, _, _| 1);
    drive(&mut edited, 30_000, 60_000, &mut out);
    let frames: Vec<u64> = ons_at(&out, 1, 30_000).iter().map(|e| e.0).collect();
    assert_eq!(frames[..3], [30_000, 36_000, 42_000], "{out:?}");
}

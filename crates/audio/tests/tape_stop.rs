// The module uses `crate::biquad::flush`.
#[allow(dead_code)]
#[path = "../src/biquad.rs"]
mod biquad;
#[path = "../src/tape_stop.rs"]
mod tape_stop;

use tape_stop::TapeStopState;

fn params() -> [f32; 8] {
    [1.0, 0.2, 0.2, 1.0 / 3.0, 1.0 / 3.0, 1.0, 0.5, 0.0]
}

#[test]
fn playing_passes_stereo_input_without_latency() {
    let mut state = TapeStopState::new(&params(), 1_000.0, 120.0);
    for i in 0..100 {
        let left = i as f32 / 100.0;
        let right = -left;
        assert_eq!(state.process_frame(left, right), (left, right));
    }
}

#[test]
fn stop_slows_read_head_and_eventually_fades() {
    let mut state = TapeStopState::new(&params(), 1_000.0, 120.0);
    for i in 0..100 {
        state.process_frame(i as f32 / 100.0, -(i as f32) / 100.0);
    }
    let mut stopped = params();
    stopped[5] = 0.0;
    state.refresh(&stopped, 1_000.0);
    let mut pitched = false;
    for i in 0..1_200 {
        let input = (i + 100) as f32 / 100.0;
        let (left, right) = state.process_frame(input, -input);
        if i > 100 && i < 900 && (left - input).abs() > 0.05 {
            pitched = true;
        }
        assert!((left + right).abs() < 1e-5);
    }
    assert!(pitched);
    assert!(state.playback_speed() < 0.001);
    let (left, right) = state.process_frame(100.0, -100.0);
    assert!(left.abs() < 0.01 && right.abs() < 0.01);
}

#[test]
fn start_resumes_live_audio_after_ramp() {
    let mut state = TapeStopState::new(&params(), 1_000.0, 120.0);
    for _ in 0..100 {
        state.process_frame(0.5, -0.5);
    }
    let mut stopped = params();
    stopped[5] = 0.0;
    state.refresh(&stopped, 1_000.0);
    for _ in 0..1_100 {
        state.process_frame(0.0, 0.0);
    }
    state.refresh(&params(), 1_000.0);
    for _ in 0..1_100 {
        state.process_frame(0.0, 0.0);
    }
    assert!((state.playback_speed() - 1.0).abs() < 0.001);
    assert_eq!(state.process_frame(0.75, -0.25), (0.75, -0.25));
}

#[test]
fn synced_duration_tracks_tempo() {
    let mut p = params();
    p[7] = 1.0;
    p[3] = 5.0 / 9.0;
    let mut fast = TapeStopState::new(&p, 1_000.0, 120.0);
    let mut slow = TapeStopState::new(&p, 1_000.0, 120.0);
    fast.prepare_block(120.0);
    slow.prepare_block(60.0);
    p[5] = 0.0;
    fast.refresh(&p, 1_000.0);
    slow.refresh(&p, 1_000.0);
    for _ in 0..600 {
        fast.process_frame(0.0, 0.0);
        slow.process_frame(0.0, 0.0);
    }
    assert!(fast.playback_speed() < 0.001);
    assert!(slow.playback_speed() > 0.1);
}

#[test]
fn reversing_mid_ramp_uses_only_the_remaining_travel_time() {
    let mut state = TapeStopState::new(&params(), 1_000.0, 120.0);
    let mut stopped = params();
    stopped[5] = 0.0;
    state.refresh(&stopped, 1_000.0);
    for _ in 0..500 {
        state.process_frame(0.0, 0.0);
    }
    state.refresh(&params(), 1_000.0);
    for _ in 0..550 {
        state.process_frame(0.0, 0.0);
    }
    assert!((state.playback_speed() - 1.0).abs() < 0.001);
}

#[test]
fn reset_discards_buffer_and_restores_control_state() {
    let mut state = TapeStopState::new(&params(), 1_000.0, 120.0);
    for i in 0..2_000 {
        state.process_frame(if i == 0 { 1.0 } else { 0.0 }, 0.0);
    }
    state.reset();
    for _ in 0..2_000 {
        assert_eq!(state.process_frame(0.0, 0.0), (0.0, 0.0));
    }
    assert_eq!(state.playback_speed(), 1.0);
}

/// Synced, 16-beat Stop, the slowest warp (the read head lags furthest: about 0.82 of the ramp): `history` frames of
/// noise played through at `tempo`, then Stop for 20 s. The output from the Stop on, at 8 kHz.
fn sixteen_beat_stop(state: &mut TapeStopState, tempo: f32, history: usize) -> Vec<(f32, f32)> {
    const RATE: f32 = 8_000.0;
    let mut p = params();
    p[3] = 1.0;
    p[6] = 0.0;
    p[7] = 1.0;
    state.refresh(&p, RATE);
    state.prepare_block(tempo);
    let mut seed = 1_u32;
    let mut noise = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1 << 23) as f32 - 1.0
    };
    for _ in 0..history {
        let input = noise();
        state.process_frame(input, -input);
    }
    p[5] = 0.0;
    state.refresh(&p, RATE);
    (0..20 * RATE as usize)
        .map(|_| {
            let input = noise();
            state.process_frame(input, -input)
        })
        .collect()
}

/// The ring holds the next power of two above Fs·max(5, 960/BPM) frames, sized for the song
/// tempo at build. A ring sized for 10 BPM is the reference that holds everything.
#[test]
fn a_ring_sized_for_the_song_tempo_holds_a_whole_sixteen_beat_stop() {
    const RATE: f32 = 8_000.0;
    for tempo in [85.0, 60.0] {
        let mut sized = TapeStopState::new(&params(), RATE, tempo);
        let mut roomy = TapeStopState::new(&params(), RATE, 10.0);
        let mut small = TapeStopState::new(&params(), RATE, 120.0);
        let reference = sixteen_beat_stop(&mut roomy, tempo, 200_000);
        assert!(
            sixteen_beat_stop(&mut sized, tempo, 200_000) == reference,
            "{tempo} BPM"
        );
        // The scenario needs the room: a ring sized for 120 BPM has lost the oldest audio the stop reads.
        assert!(sixteen_beat_stop(&mut small, tempo, 200_000) != reference);
    }
}

/// A tempo change down to 60 BPM is prepared off the audio thread (`prepare`, which keeps the audio held so far);
/// `prepare_block` on the audio thread never resizes. A stop right after the change reads back into the audio played
/// at 120 BPM, further than a 120 BPM ring reaches.
#[test]
fn a_tempo_change_down_to_60_is_prepared_before_it_plays() {
    const RATE: f32 = 8_000.0;
    let mut changed = TapeStopState::new(&params(), RATE, 120.0);
    let mut roomy = TapeStopState::new(&params(), RATE, 10.0);
    for state in [&mut changed, &mut roomy] {
        state.prepare_block(120.0);
        for i in 0..60_000 {
            let input = (i as f32 * 0.01).sin();
            state.process_frame(input, input);
        }
        state.prepare(60.0);
    }
    assert!(sixteen_beat_stop(&mut changed, 60.0, 0) == sixteen_beat_stop(&mut roomy, 60.0, 0));
}

/// A synced stop that starts at a tempo slower than the ring was sized for (a tempo drag; only
/// the build allocates) is cut to what the ring holds rather than reading wrapped audio. On a rising ramp, a read head
/// that never outruns the audio reads values that never rise faster than the input; a wrapped one jumps by the ring.
#[test]
fn a_stop_longer_than_the_ring_is_cut_to_it_and_never_reads_wrapped_audio() {
    const RATE: f32 = 8_000.0;
    // 16 beats at 30 BPM are 32 s; the ring, sized for 120 BPM, holds 65 536 frames (8.2 s).
    let mut p = params();
    (p[3], p[6], p[7]) = (1.0, 0.0, 1.0);
    let mut state = TapeStopState::new(&p, RATE, 120.0);
    state.prepare_block(30.0);
    let step = 1e-5;
    let mut frame = 0;
    let mut input = || {
        frame += 1;
        frame as f32 * step
    };
    for _ in 0..1_000 {
        let x = input();
        state.process_frame(x, x);
    }
    p[5] = 0.0;
    state.refresh(&p, RATE);
    let mut last = state.process_frame(input(), 0.0).0;
    for at in 0..(40 * RATE as usize) {
        let (left, _) = state.process_frame(input(), 0.0);
        assert!(left - last <= 1.1 * step, "frame {at}: {last} → {left}");
        last = left;
    }
    assert!(state.playback_speed() < 0.001, "still stopping");
}

/// An edit's engine takes over a running Tape Stop and hands it the ring it built for the new, slower
/// tempo (`take_ring`, allocating nothing): the stop running through the handover goes on as in a ring that was large
/// from the start, and the next one runs whole at that tempo.
#[test]
fn a_handover_moves_a_running_stop_into_the_ring_built_for_the_slower_tempo() {
    const RATE: f32 = 8_000.0;
    let mut p = params();
    (p[3], p[6], p[7]) = (1.0, 0.0, 1.0);
    let mut running = TapeStopState::new(&p, RATE, 120.0);
    let mut roomy = TapeStopState::new(&p, RATE, 10.0);
    running.prepare_block(120.0);
    roomy.prepare_block(120.0);
    let mut seed = 7_u32;
    let mut noise = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1 << 23) as f32 - 1.0
    };
    let mut both = |frames: usize, running: &mut TapeStopState, roomy: &mut TapeStopState| {
        for _ in 0..frames {
            let x = noise();
            assert_eq!(running.process_frame(x, -x), roomy.process_frame(x, -x));
        }
    };
    both(70_000, &mut running, &mut roomy);
    // A 16-beat stop at 120 BPM: 64 000 frames, the read head falling ever further behind.
    p[5] = 0.0;
    running.refresh(&p, RATE);
    roomy.refresh(&p, RATE);
    both(50_000, &mut running, &mut roomy);
    let mut built = TapeStopState::new(&p, RATE, 60.0);
    running.take_ring(&mut built);
    both(20_000, &mut running, &mut roomy);
    // The next stop, a whole 16 beats at 60 BPM (128 000 frames), plays as in the roomy ring.
    p[5] = 1.0;
    running.refresh(&p, RATE);
    roomy.refresh(&p, RATE);
    both(70_000, &mut running, &mut roomy);
    running.prepare_block(60.0);
    roomy.prepare_block(60.0);
    p[5] = 0.0;
    running.refresh(&p, RATE);
    roomy.refresh(&p, RATE);
    both(140_000, &mut running, &mut roomy);
}

/// A reversal takes only the remaining travel time: at the tempo the ring was sized for, a
/// synced 16-beat Stop reversed at 90 % starts again over 90 % of 16 beats, uncut, though the read head lags far.
#[test]
fn a_stop_reversed_at_the_song_tempo_keeps_its_whole_start_ramp() {
    const RATE: f32 = 8_000.0;
    let mut p = params();
    (p[3], p[4], p[6], p[7]) = (1.0, 1.0, 0.0, 1.0);
    let mut state = TapeStopState::new(&p, RATE, 120.0);
    state.prepare_block(120.0);
    let mut seed = 3_u32;
    let mut noise = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1 << 23) as f32 - 1.0
    };
    for _ in 0..1_000 {
        let x = noise();
        state.process_frame(x, x);
    }
    // 16 beats at 120 BPM: 64 000 frames; reversed after 57 600.
    p[5] = 0.0;
    state.refresh(&p, RATE);
    for _ in 0..57_600 {
        let x = noise();
        state.process_frame(x, x);
    }
    p[5] = 1.0;
    state.refresh(&p, RATE);
    let ramp = (1..200_000_u32)
        .find(|_| {
            let x = noise();
            state.process_frame(x, -x) == (x, -x)
        })
        .unwrap();
    // 57 600 frames, give or take the f32 phase steps.
    assert!(ramp.abs_diff(57_600) <= 100, "{ramp} frames");
}

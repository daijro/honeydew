use honeydew_audio::sequencer::*;
use std::time::Duration;

fn beats(events: &[ArpNoteEvent], engine: usize, kind: NoteKind) -> Vec<f64> {
    events
        .iter()
        .filter(|e| e.engine == engine && e.kind == kind)
        .map(|e| e.beat)
        .collect()
}

#[test]
fn arp_preserves_input_pan_and_constructors_center_it() {
    assert_eq!(InputNoteEvent::on(0.0, 60, 100).pan, 0.5);
    assert_eq!(InputNoteEvent::off(0.5, 60).pan, 0.5);
    let mut arp = Arpeggiator::new(ArpSettings::default());
    let events = arp
        .process_block(
            0.0,
            0.6,
            120.0,
            &[InputNoteEvent::on(0.0, 60, 100).with_pan(0.2)],
        )
        .unwrap();
    assert!(events.iter().all(|event| event.pan == 0.2));
}

#[test]
fn expanded_arp_notes_keep_their_source_pan() {
    let mut settings = ArpSettings::default();
    settings.engines[0].range_octaves = 2;
    let mut arp = Arpeggiator::new(settings);
    let output = arp
        .process_block(
            0.0,
            1.1,
            120.0,
            &[
                InputNoteEvent::on(0.0, 60, 100).with_pan(0.2),
                InputNoteEvent::on(0.0, 72, 100).with_pan(0.8),
            ],
        )
        .unwrap();
    let on: Vec<_> = output
        .iter()
        .filter(|event| event.kind == NoteKind::On)
        .collect();
    assert_eq!(
        on.iter().map(|event| event.pitch).collect::<Vec<_>>(),
        [60, 72, 84]
    );
    assert_eq!(
        on.iter().map(|event| event.pan).collect::<Vec<_>>(),
        [0.2, 0.2, 0.8]
    );
}

#[test]
fn drum_swing_edits_ticks_once_using_pre_edit_positions() {
    let mut ticks = [0, 32, 64, 96];
    apply_drum_swing(&mut ticks, 4, 1.0, false).unwrap();
    assert_eq!(ticks, [0, 48, 64, 112]);
    let mut half = [0, 32, 64, 96];
    apply_drum_swing(&mut half, 4, 0.5, false).unwrap();
    assert_eq!(half, [0, 40, 64, 104]);

    let mut alternate = [32];
    apply_drum_swing(&mut alternate, 4, 0.5, true).unwrap();
    apply_drum_swing(&mut alternate, 4, 0.5, true).unwrap();
    assert_eq!(alternate, [48]);
}

#[test]
fn every_rhythm_mask_repeats_and_rotation_moves_hits() {
    let masks = [
        "10", "101", "1001", "1011", "10101", "100101", "100110", "101011", "1001010", "1001011",
        "1001101", "1010101", "1011011", "10001011", "10001101", "10010010", "10010110",
        "10010111", "10011010", "10101011", "10101101",
    ];
    for (index, mask) in masks.iter().enumerate() {
        let length = mask.len();
        for step in 0..length * 2 {
            assert_eq!(
                rhythm_hit((index + 1) as u8, 0.5, step as u64).unwrap(),
                mask.as_bytes()[step % length] == b'1'
            );
        }
    }
    assert!(!rhythm_hit(1, 1.0, 0).unwrap());
    assert!(rhythm_hit(1, 1.0, 1).unwrap());
}

#[test]
fn arp_engines_merge_without_restarting_phase_and_flush_note_offs() {
    let mut settings = ArpSettings::default();
    settings.engines[0].rhythm = 1;
    settings.engines[1].enabled = true;
    settings.engines[1].rate_multiplier = 8;
    settings.engines[1].gate = 0.5;
    let mut arp = Arpeggiator::new(settings);
    let first = arp
        .process_block(0.0, 0.2, 120.0, &[InputNoteEvent::on(0.0, 60, 100)])
        .unwrap();
    assert_eq!(beats(&first, 0, NoteKind::On), [0.0]);
    assert_eq!(beats(&first, 1, NoteKind::On), [0.0]);
    let second = arp.process_block(0.2, 1.1, 120.0, &[]).unwrap();
    assert_eq!(beats(&second, 0, NoteKind::On), [1.0]);
    assert_eq!(beats(&second, 1, NoteKind::On), [1.0]);
    assert!(
        second
            .iter()
            .any(|e| e.engine == 0 && e.kind == NoteKind::Off && e.beat == 0.25)
    );
    assert!(
        second
            .iter()
            .any(|e| e.engine == 1 && e.kind == NoteKind::Off && e.beat == 0.5)
    );
}

#[test]
fn rests_preserve_pitch_order_and_two_engines_hold_independently() {
    let mut settings = ArpSettings::default();
    settings.engines[0].mode = ArpMode::Up;
    settings.engines[0].rhythm = 1;
    settings.engines[1].enabled = true;
    settings.engines[1].mode = ArpMode::Down;
    let mut arp = Arpeggiator::new(settings);
    let events = arp
        .process_block(
            0.0,
            1.6,
            120.0,
            &[
                InputNoteEvent::on(0.0, 60, 100),
                InputNoteEvent::on(0.0, 64, 80),
                InputNoteEvent::on(0.0, 67, 90),
            ],
        )
        .unwrap();
    let first: Vec<_> = events
        .iter()
        .filter(|e| e.kind == NoteKind::On && e.engine == 0)
        .map(|e| (e.beat, e.pitch))
        .collect();
    let second: Vec<_> = events
        .iter()
        .filter(|e| e.kind == NoteKind::On && e.engine == 1)
        .map(|e| (e.beat, e.pitch))
        .collect();
    assert_eq!(first, [(0.0, 60), (1.0, 64)]);
    assert_eq!(second, [(0.0, 67), (0.5, 64), (1.0, 60), (1.5, 67)]);
}

#[test]
fn swing_delays_emission_not_step_phase_or_pitch_phase() {
    let settings = ArpSettings {
        swing: 1.0,
        ..ArpSettings::default()
    };
    let mut arp = Arpeggiator::new(settings);
    let events = arp
        .process_block(0.0, 1.6, 120.0, &[InputNoteEvent::on(0.0, 60, 100)])
        .unwrap();
    assert_eq!(beats(&events, 0, NoteKind::On), [0.0, 0.75, 1.0]);
    assert_eq!(arp_swing_delay(0.75, 0.5, 1.0, SwingRate::Eighth), 0.125);
    assert_eq!(
        arp_swing_delay(0.25, 0.25, 1.0, SwingRate::Sixteenth),
        0.125
    );
}

#[test]
fn free_rate_uses_tempo_and_supersaw_handles_one_note() {
    let mut settings = ArpSettings {
        sync: false,
        ..ArpSettings::default()
    };
    assert_eq!(settings.step_interval(0, 120.0).unwrap(), 0.5);
    assert_eq!(settings.step_interval(0, 60.0).unwrap(), 0.25);
    settings.engines[0].mode = ArpMode::SuperSaw;
    let mut arp = Arpeggiator::new(settings);
    assert_eq!(
        beats(
            &arp.process_block(0.0, 1.0, 120.0, &[InputNoteEvent::on(0.0, 60, 100)])
                .unwrap(),
            0,
            NoteKind::On,
        ),
        [0.0, 0.5]
    );
}

#[test]
fn disabled_engine_keeps_held_input_and_flushes_existing_offs() {
    let mut arp = Arpeggiator::new(ArpSettings::default());
    assert!(
        arp.process_block(0.0, 0.1, 120.0, &[InputNoteEvent::on(0.0, 60, 100)])
            .unwrap()
            .iter()
            .all(|e| e.engine == 0)
    );
    arp.settings.engines[1].enabled = true;
    let enabled = arp.process_block(0.1, 0.6, 120.0, &[]).unwrap();
    assert_eq!(beats(&enabled, 1, NoteKind::On), [0.5]);
    arp.settings.engines[1].enabled = false;
    let disabled = arp.process_block(0.6, 0.8, 120.0, &[]).unwrap();
    assert!(
        disabled
            .iter()
            .any(|e| e.engine == 1 && e.kind == NoteKind::Off && e.beat == 0.75)
    );
}

#[test]
fn free_rate_tempo_change_keeps_origin_without_retroactive_events() {
    let settings = ArpSettings {
        sync: false,
        ..ArpSettings::default()
    };
    let mut arp = Arpeggiator::new(settings);
    arp.process_block(0.0, 0.6, 120.0, &[InputNoteEvent::on(0.0, 60, 100)])
        .unwrap();
    let after = arp.process_block(0.6, 1.1, 60.0, &[]).unwrap();
    assert!(after.iter().all(|event| event.beat >= 0.6));
    assert_eq!(beats(&after, 0, NoteKind::On), [0.75, 1.0]);
}

#[test]
fn released_source_stops_new_steps_but_finishes_pending_gate() {
    let mut arp = Arpeggiator::new(ArpSettings::default());
    arp.process_block(0.0, 0.1, 120.0, &[InputNoteEvent::on(0.0, 60, 100)])
        .unwrap();
    let after = arp
        .process_block(0.1, 0.8, 120.0, &[InputNoteEvent::off(0.2, 60)])
        .unwrap();
    assert!(beats(&after, 0, NoteKind::On).is_empty());
    assert_eq!(beats(&after, 0, NoteKind::Off), [0.25]);
}

#[test]
fn rhythm_rests_do_not_consume_limited_pitch_cycles() {
    let mut settings = ArpSettings::default();
    settings.engines[0].rhythm = 1;
    settings.engines[0].repeats = Some(1);
    let mut arp = Arpeggiator::new(settings);
    let events = arp
        .process_block(
            0.0,
            4.1,
            120.0,
            &[
                InputNoteEvent::on(0.0, 60, 100),
                InputNoteEvent::on(0.0, 64, 100),
                InputNoteEvent::on(0.0, 67, 100),
            ],
        )
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == NoteKind::On)
            .map(|e| (e.beat, e.pitch))
            .collect::<Vec<_>>(),
        [(0.0, 60), (1.0, 64), (2.0, 67)]
    );
}

#[test]
fn uncommon_order_sequences_keep_their_distinct_pitch_paths() {
    let notes = [60, 62, 64, 65, 67];
    assert_eq!(
        arp_pitch_order(ArpMode::Outward, &notes, 1, 0).unwrap(),
        [64, 62, 65, 60, 67]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::Inward, &notes, 1, 0).unwrap(),
        [60, 67, 62, 65, 64]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::BassUp, &notes, 1, 0).unwrap(),
        [60, 62, 60, 64, 60, 65, 60, 67]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::TrebleDown, &notes, 1, 0).unwrap(),
        [67, 65, 67, 64, 67, 62, 67, 60]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::JumpUp, &notes, 1, 0).unwrap(),
        [60, 64, 67, 62, 65]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::JumpDown, &notes, 1, 0).unwrap(),
        [67, 64, 60, 65, 62]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::CascadeUp, &notes, 1, 0).unwrap(),
        [60, 62, 64, 62, 64, 65, 64, 65, 67, 65, 67]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::CascadeDown, &notes, 1, 0).unwrap(),
        [67, 65, 64, 65, 64, 62, 64, 62, 60, 62, 60]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::TrebleDownUp, &notes, 1, 0).unwrap(),
        [67, 65, 67, 64, 67, 62, 67, 60, 67, 62, 67, 64]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::CascadeUpDown, &[60, 62], 1, 0).unwrap(),
        [60, 62, 60]
    );
}

#[test]
fn pitch_and_retrigger_menu_indices_match_the_declared_orders() {
    assert_eq!(ArpMode::from_menu_index(6).unwrap(), ArpMode::Outward);
    assert_eq!(
        ArpMode::from_menu_index(19).unwrap(),
        ArpMode::CascadeUpDown
    );
    assert_eq!(ArpMode::from_menu_index(23).unwrap(), ArpMode::RandomWalk);
    assert_eq!(
        ArpMode::from_menu_index(24),
        Err(SequencerError::InvalidValue)
    );

    let mut engine = EngineSettings::default();
    engine.set_retrigger_menu_index(1).unwrap();
    assert!(engine.retrigger_on_notes);
    assert_eq!(engine.retrigger_interval, None);
    engine.set_retrigger_menu_index(4).unwrap();
    assert!(!engine.retrigger_on_notes);
    assert_eq!(engine.retrigger_interval.unwrap().beats(), 1.5);
    engine.set_retrigger_menu_index(11).unwrap();
    assert_eq!(engine.retrigger_interval.unwrap().beats(), 32.0);
    engine.set_retrigger_menu_index(0).unwrap();
    assert!(!engine.retrigger_on_notes);
    assert_eq!(engine.retrigger_interval, None);
    assert_eq!(
        engine.set_retrigger_menu_index(12),
        Err(SequencerError::InvalidValue)
    );
}

#[test]
fn range_expands_octaves_before_sorting_and_anchor_uses_start_note() {
    assert_eq!(
        arp_pitch_order(ArpMode::Up, &[60, 64, 67], 2, 0).unwrap(),
        [60, 64, 67, 72, 76, 79]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::Anchor, &[60, 64, 67], 2, 1).unwrap(),
        [64, 76]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::Up, &[120, 124], 2, 0).unwrap(),
        [120, 124]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::Up, &[60], 5, 0),
        Err(SequencerError::InvalidValue)
    );
}

#[test]
fn jump_up_down_and_bass_up_down_include_their_cycle_turns() {
    assert_eq!(
        arp_pitch_order(ArpMode::JumpUpDown, &[60, 62, 64, 65], 1, 0).unwrap(),
        [60, 64, 62, 65, 65, 62, 64, 60]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::BassUpDown, &[60, 62, 64, 65], 1, 0).unwrap(),
        [60, 62, 60, 64, 60, 65, 60, 64]
    );
}

#[test]
fn stateless_pitch_order_rejects_stateful_random_modes() {
    assert_eq!(
        arp_pitch_order(ArpMode::RandomWalk, &[60, 64, 67], 1, 0),
        Err(SequencerError::UnsupportedMode(ArpMode::RandomWalk))
    );
}

#[test]
fn supersaw_uses_grouped_source_indices_and_skips_high_transpositions() {
    assert_eq!(
        arp_pitch_order(ArpMode::SuperSaw, &[60, 64, 67], 1, 0).unwrap(),
        [60, 64, 64, 67]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::SuperSaw, &[120, 124], 2, 0).unwrap(),
        [120, 124, 120]
    );
    assert_eq!(
        arp_pitch_order(ArpMode::SuperSaw, &[60], 1, 0).unwrap(),
        [60]
    );
}

fn random_ons(mode: ArpMode, seed: u64, notes: &[i16], end: f64) -> Vec<i16> {
    let mut settings = ArpSettings::default();
    settings.engines[0].mode = mode;
    let mut arp = Arpeggiator::with_random_seed(settings, seed);
    let input = notes
        .iter()
        .map(|pitch| InputNoteEvent::on(0.0, *pitch, 100))
        .collect::<Vec<_>>();
    arp.process_block(0.0, end, 120.0, &input)
        .unwrap()
        .into_iter()
        .filter(|event| event.engine == 0 && event.kind == NoteKind::On)
        .map(|event| event.pitch)
        .collect()
}

#[test]
fn random_modes_repeat_for_a_seed_and_select_only_held_pitches() {
    for mode in [
        ArpMode::Random,
        ArpMode::RandomNoRepeat,
        ArpMode::RandomOnce,
        ArpMode::RandomWalk,
    ] {
        let first = random_ons(mode, 32, &[60, 64, 67], 12.1);
        assert_eq!(first, random_ons(mode, 32, &[60, 64, 67], 12.1));
        assert!(first.iter().all(|pitch| [60, 64, 67].contains(pitch)));
        assert_eq!(first.len(), 25);
    }
}

#[test]
fn random_no_repeat_consumes_bags_without_cross_cycle_repeat() {
    let pitches = random_ons(ArpMode::RandomNoRepeat, 15, &[60, 64, 67, 72], 8.1);
    for bag in pitches.as_chunks::<4>().0 {
        let mut sorted = bag.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, [60, 64, 67, 72]);
    }
    for boundary in (4..pitches.len()).step_by(4) {
        assert_ne!(pitches[boundary - 1], pitches[boundary]);
    }
}

#[test]
fn random_once_cycles_one_shuffle_and_retrigger_preserves_it() {
    let pitches = random_ons(ArpMode::RandomOnce, 77, &[60, 64, 67, 72], 8.1);
    for cycle in pitches.as_chunks::<4>().0 {
        assert_eq!(cycle, &pitches[..4]);
    }
    let mut settings = ArpSettings::default();
    settings.engines[0].mode = ArpMode::RandomOnce;
    settings.engines[0].retrigger_interval = Some(RetriggerInterval::Quarter);
    let mut arp = Arpeggiator::with_random_seed(settings, 77);
    let input = [60, 64, 67, 72].map(|pitch| InputNoteEvent::on(0.0, pitch, 100));
    let retriggered = arp
        .process_block(0.0, 2.1, 120.0, &input)
        .unwrap()
        .into_iter()
        .filter(|event| event.engine == 0 && event.kind == NoteKind::On)
        .map(|event| event.pitch)
        .collect::<Vec<_>>();
    assert_eq!(
        retriggered,
        [pitches[0], pitches[1], pitches[0], pitches[1], pitches[0]]
    );
}

#[test]
fn random_walk_reverses_at_edges_and_stays_near_previous_note() {
    let pitches = random_ons(ArpMode::RandomWalk, 140, &[60, 62, 64, 65, 67], 30.1);
    assert!(pitches.windows(2).all(|pair| {
        let left = [60, 62, 64, 65, 67]
            .iter()
            .position(|p| *p == pair[0])
            .unwrap();
        let right = [60, 62, 64, 65, 67]
            .iter()
            .position(|p| *p == pair[1])
            .unwrap();
        left.abs_diff(right) <= 2 && left != right
    }));
}

#[test]
fn pitch_set_change_preserves_last_random_pitch_when_still_held() {
    for mode in [ArpMode::RandomNoRepeat, ArpMode::RandomWalk] {
        let mut settings = ArpSettings::default();
        settings.engines[0].mode = mode;
        let mut arp = Arpeggiator::with_random_seed(settings, 24);
        let input = [
            InputNoteEvent::on(0.0, 60, 100),
            InputNoteEvent::on(0.0, 64, 100),
            InputNoteEvent::on(0.0, 67, 100),
            InputNoteEvent::on(0.0, 72, 100),
            InputNoteEvent::on(1.25, 74, 100),
        ];
        let ons = arp
            .process_block(0.0, 1.6, 120.0, &input)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == NoteKind::On)
            .collect::<Vec<_>>();
        let before = ons.iter().find(|event| event.beat == 1.0).unwrap().pitch;
        let after = ons.iter().find(|event| event.beat == 1.5).unwrap().pitch;
        if mode == ArpMode::RandomNoRepeat {
            assert_ne!(before, after);
        } else {
            let pitches = [60, 64, 67, 72, 74];
            let left = pitches.iter().position(|pitch| *pitch == before).unwrap();
            let right = pitches.iter().position(|pitch| *pitch == after).unwrap();
            assert!(left.abs_diff(right) <= 2 && left != right);
        }
    }
}

#[test]
fn transport_retrigger_resets_pitch_order_on_absolute_beat_grid() {
    let mut settings = ArpSettings::default();
    settings.engines[0].mode = ArpMode::Up;
    settings.engines[0].retrigger_interval = Some(RetriggerInterval::Quarter);
    let input = [
        InputNoteEvent::on(0.25, 60, 100),
        InputNoteEvent::on(0.25, 64, 100),
        InputNoteEvent::on(0.25, 67, 100),
    ];
    let mut whole = Arpeggiator::new(settings);
    let one = whole.process_block(0.0, 2.1, 120.0, &input).unwrap();
    let expected = [(0.25, 60), (0.75, 64), (1.0, 60), (1.5, 64), (2.0, 60)];
    assert_eq!(
        one.iter()
            .filter(|e| e.kind == NoteKind::On)
            .map(|e| (e.beat, e.pitch))
            .collect::<Vec<_>>(),
        expected
    );
    let mut split = Arpeggiator::new(settings);
    let mut many = split.process_block(0.0, 0.6, 120.0, &input).unwrap();
    many.extend(split.process_block(0.6, 1.0, 120.0, &[]).unwrap());
    many.extend(split.process_block(1.0, 1.4, 120.0, &[]).unwrap());
    many.extend(split.process_block(1.4, 2.1, 120.0, &[]).unwrap());
    assert_eq!(one, many);
}

#[test]
fn retrigger_period_never_runs_faster_than_the_step_interval() {
    let mut settings = ArpSettings::default();
    settings.engines[0].mode = ArpMode::Up;
    settings.engines[0].rate_multiplier = 8;
    settings.engines[0].retrigger_interval = Some(RetriggerInterval::Eighth);
    let mut arp = Arpeggiator::new(settings);
    let input = [60, 64, 67].map(|pitch| InputNoteEvent::on(0.25, pitch, 100));
    let events = arp.process_block(0.0, 2.2, 120.0, &input).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == NoteKind::On)
            .map(|event| (event.beat, event.pitch))
            .collect::<Vec<_>>(),
        [(0.25, 60), (1.0, 60), (2.0, 60)]
    );
}

#[test]
fn bar_retrigger_uses_supplied_measure_length() {
    let mut settings = ArpSettings {
        bar_length_beats: 3.0,
        ..ArpSettings::default()
    };
    settings.engines[0].mode = ArpMode::Up;
    settings.engines[0].retrigger_interval = Some(RetriggerInterval::Bar);
    assert_eq!(RetriggerInterval::TwoBars.beats_for_bar(3.0), 6.0);
    let mut arp = Arpeggiator::new(settings);
    let input = [60, 64, 67].map(|pitch| InputNoteEvent::on(0.25, pitch, 100));
    let events = arp.process_block(0.0, 3.1, 120.0, &input).unwrap();
    assert!(
        events
            .iter()
            .any(|event| { event.kind == NoteKind::On && event.beat == 3.0 && event.pitch == 60 })
    );
}

#[test]
fn enabling_periodic_retrigger_mid_playback_never_emits_past_events() {
    let mut settings = ArpSettings::default();
    settings.engines[0].mode = ArpMode::Up;
    let mut arp = Arpeggiator::new(settings);
    let input = [60, 64, 67].map(|pitch| InputNoteEvent::on(0.25, pitch, 100));
    arp.process_block(0.0, 2.0, 120.0, &input).unwrap();
    arp.settings.engines[0].set_retrigger_menu_index(3).unwrap();
    let after = arp.process_block(2.0, 3.1, 120.0, &[]).unwrap();
    assert!(after.iter().all(|event| event.beat >= 2.0));
    assert_eq!(
        after
            .iter()
            .filter(|event| event.kind == NoteKind::On)
            .map(|event| (event.beat, event.pitch))
            .collect::<Vec<_>>(),
        [(2.0, 60), (2.5, 64), (3.0, 60)]
    );
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn chord_timing_orders_pitches_and_release_cancels_future_ons() {
    let mut keyboard = ChordKeyboard::default();
    assert_eq!(
        keyboard
            .touch(1, &[60, 64, 67, 72], 1.0, 0, 100, ms(10))
            .unwrap(),
        [TimedNoteEvent::on(ms(10), 60, 100)]
    );
    assert_eq!(
        keyboard.advance(ms(55)),
        [TimedNoteEvent::on(ms(50), 64, 100)]
    );
    assert_eq!(
        keyboard.release(1, ms(60)),
        [
            TimedNoteEvent::off(ms(60), 60),
            TimedNoteEvent::off(ms(60), 64)
        ]
    );
    assert!(keyboard.advance(ms(200)).is_empty());
    assert_eq!(
        chord_onsets(&[60, 64, 67, 72], 0.0, 0, 100, ms(0)).unwrap(),
        [
            TimedNoteEvent::on(ms(0), 72, 100),
            TimedNoteEvent::on(ms(40), 67, 100),
            TimedNoteEvent::on(ms(80), 64, 100),
            TimedNoteEvent::on(ms(120), 60, 100)
        ]
    );
}

#[test]
fn chord_release_or_slide_at_a_queued_onset_cancels_that_note() {
    let mut keyboard = ChordKeyboard::default();
    keyboard.touch(1, &[60, 64], 1.0, 0, 100, ms(0)).unwrap();
    assert_eq!(
        keyboard.release(1, ms(40)),
        [TimedNoteEvent::off(ms(40), 60)]
    );
    assert!(keyboard.advance(ms(100)).is_empty());

    let mut keyboard = ChordKeyboard::default();
    keyboard.touch(1, &[60, 64], 1.0, 0, 100, ms(0)).unwrap();
    assert_eq!(
        keyboard.release(1, ms(60)),
        [
            TimedNoteEvent::on(ms(40), 64, 100),
            TimedNoteEvent::off(ms(60), 60),
            TimedNoteEvent::off(ms(60), 64)
        ]
    );

    let mut keyboard = ChordKeyboard::default();
    keyboard.touch(1, &[60, 64], 1.0, 0, 100, ms(0)).unwrap();
    assert!(
        keyboard
            .slide(1, &[60, 67], 1.0, 0, 100, ms(40))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        keyboard.advance(ms(80)),
        [TimedNoteEvent::on(ms(80), 67, 100)]
    );
}

#[test]
fn chord_slide_sustains_shared_pitch_and_retrigger_orders_off_first() {
    let mut keyboard = ChordKeyboard::default();
    keyboard.touch(1, &[60, 64], 0.5, 0, 100, ms(0)).unwrap();
    assert_eq!(
        keyboard.touch(2, &[64], 0.5, 0, 90, ms(1)).unwrap(),
        [
            TimedNoteEvent::off(ms(1), 64),
            TimedNoteEvent::on(ms(1), 64, 90)
        ]
    );
    assert_eq!(
        keyboard.slide(1, &[64, 67], 0.5, 0, 100, ms(2)).unwrap(),
        [
            TimedNoteEvent::off(ms(2), 60),
            TimedNoteEvent::on(ms(2), 67, 100)
        ]
    );
    assert!(keyboard.release(2, ms(3)).is_empty());
    assert_eq!(
        keyboard.release(1, ms(4)),
        [
            TimedNoteEvent::off(ms(4), 64),
            TimedNoteEvent::off(ms(4), 67)
        ]
    );
}

#[test]
fn chord_hold_defers_release_until_disabled() {
    let mut keyboard = ChordKeyboard::default();
    keyboard.touch(4, &[60], 0.5, 0, 100, ms(0)).unwrap();
    keyboard.set_hold(true, ms(1));
    assert!(keyboard.release(4, ms(2)).is_empty());
    assert_eq!(
        keyboard.set_hold(false, ms(20)),
        [TimedNoteEvent::off(ms(20), 60)]
    );
}

#[test]
fn strum_uses_crossing_times_without_automatic_spacing() {
    let mut keyboard = StrumKeyboard::default();
    assert_eq!(
        keyboard.enter_cell(1, 0, 60, 0, 100, ms(10)),
        [TimedNoteEvent::on(ms(10), 60, 100)]
    );
    assert_eq!(
        keyboard.enter_cell(1, 1, 64, 0, 100, ms(13)),
        [
            TimedNoteEvent::off(ms(13), 60),
            TimedNoteEvent::on(ms(13), 64, 100)
        ]
    );
    assert_eq!(
        keyboard.enter_cell(1, 2, 67, 0, 100, ms(100)),
        [
            TimedNoteEvent::off(ms(100), 64),
            TimedNoteEvent::on(ms(100), 67, 100)
        ]
    );
    assert_eq!(
        keyboard.release(1, ms(101)),
        [TimedNoteEvent::off(ms(101), 67)]
    );
}

#[test]
fn note_events_sort_as_the_stable_sort_by_beat_with_offs_first() {
    // Interleaved clips' notes with many equal beats and pitches: the order is the stable sort's, ties included.
    let mut state = 7_u64;
    let mut next = |n: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % n
    };
    let mut events = Vec::new();
    for clip in 0..4 {
        for i in 0..500 {
            let on = f64::from(clip) + (i / 3) as f64 / 8.0;
            let pitch = next(4) as i16;
            events.push(InputNoteEvent::on(on, pitch, (i % 200) as u8 + 1));
            events.push(InputNoteEvent::off(on + next(5) as f64 / 8.0, pitch));
        }
    }
    let mut want = events.clone();
    want.sort_by(|a, b| {
        a.beat
            .total_cmp(&b.beat)
            .then_with(|| (a.kind != NoteKind::Off).cmp(&(b.kind != NoteKind::Off)))
    });
    sort_note_events(&mut events);
    assert_eq!(events, want);
}

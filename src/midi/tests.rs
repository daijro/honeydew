use super::links::{self, CcLink};
use super::message::{MidiMsg, data_len};
use super::route::{Learn, Out, Router};
use crate::song::{AutomationTarget, Song};

#[test]
fn parse_table() {
    use MidiMsg::*;
    let cases: &[(&[u8], Option<MidiMsg>)] = &[
        (
            &[0x90, 60, 100],
            Some(NoteOn {
                ch: 0,
                key: 60,
                vel: 100,
            }),
        ),
        (
            &[0x9F, 127, 127],
            Some(NoteOn {
                ch: 15,
                key: 127,
                vel: 127,
            }),
        ),
        // Note-on with velocity 0 is a note-off.
        (
            &[0x93, 60, 0],
            Some(NoteOff {
                ch: 3,
                key: 60,
                vel: 0,
            }),
        ),
        (
            &[0x81, 60, 64],
            Some(NoteOff {
                ch: 1,
                key: 60,
                vel: 64,
            }),
        ),
        (
            &[0xB0, 7, 127],
            Some(Cc {
                ch: 0,
                cc: 7,
                value: 127,
            }),
        ),
        (
            &[0xBF, 127, 0],
            Some(Cc {
                ch: 15,
                cc: 127,
                value: 0,
            }),
        ),
        (&[0xE0, 0, 0x40], Some(PitchBend { ch: 0, value: 8192 })),
        (
            &[0xE2, 0x7F, 0x7F],
            Some(PitchBend {
                ch: 2,
                value: 16383,
            }),
        ),
        (&[0xE0, 0, 0], Some(PitchBend { ch: 0, value: 0 })),
        (&[0xD5, 90], Some(ChannelPressure { ch: 5, value: 90 })),
        (&[0xC9, 12], Some(Program { ch: 9, program: 12 })),
        // Poly pressure, system messages.
        (&[0xA0, 60, 10], None),
        (&[0xF0, 0x7E, 0xF7], None),
        (&[0xF8], None),
        (&[0xFE], None),
        (&[0xFF], None),
        // No running status: a message must start with a status byte.
        (&[60, 100], None),
        (&[0x00, 0x00, 0x00], None),
        // Wrong lengths.
        (&[], None),
        (&[0x90], None),
        (&[0x90, 60], None),
        (&[0x90, 60, 100, 0], None),
        (&[0xC0], None),
        (&[0xC0, 1, 2], None),
        // A status byte where a data byte belongs.
        (&[0x90, 0x80, 100], None),
        (&[0xB0, 7, 0x90], None),
    ];
    for (bytes, want) in cases {
        assert_eq!(MidiMsg::parse(bytes), *want, "{bytes:02X?}");
    }
}

#[test]
fn data_lengths() {
    assert_eq!(data_len(0x80), Some(2));
    assert_eq!(data_len(0x9F), Some(2));
    assert_eq!(data_len(0xA0), Some(2));
    assert_eq!(data_len(0xB0), Some(2));
    assert_eq!(data_len(0xC0), Some(1));
    assert_eq!(data_len(0xDF), Some(1));
    assert_eq!(data_len(0xE0), Some(2));
    assert_eq!(data_len(0xF0), None);
    assert_eq!(data_len(0x7F), None);
}

fn on(key: u8, vel: u8) -> MidiMsg {
    MidiMsg::NoteOn { ch: 0, key, vel }
}

fn off(key: u8) -> MidiMsg {
    MidiMsg::NoteOff { ch: 0, key, vel: 0 }
}

fn cc(cc: u8, value: u8) -> MidiMsg {
    MidiMsg::Cc { ch: 0, cc, value }
}

fn note(channel: usize, key: u8, on: bool, velocity: f32) -> Out {
    Out::Note {
        channel,
        key,
        on,
        velocity,
        sound: true,
    }
}

/// Routes `msg` from the device `Keys` to `channel` alone.
fn route(r: &mut Router, msg: MidiMsg, channel: usize, links: &[(usize, CcLink)]) -> Vec<Out> {
    r.route("Keys", msg, &[(channel, true)], links)
}

#[test]
fn notes_carry_velocity_from_any_midi_channel() {
    // v·0.007874: 127 is just under 1, so it records as 254.
    let mut r = Router::default();
    assert_eq!(
        route(&mut r, on(60, 127), 2, &[]),
        [note(2, 60, true, 127.0 * 0.007874)]
    );
    let omni = MidiMsg::NoteOn {
        ch: 9,
        key: 64,
        vel: 0x40,
    };
    assert_eq!(
        route(&mut r, omni, 2, &[]),
        [note(2, 64, true, 64.0 * 0.007874)]
    );
    let release = MidiMsg::NoteOff {
        ch: 9,
        key: 64,
        vel: 127,
    };
    assert_eq!(route(&mut r, release, 2, &[]), [note(2, 64, false, 1.0)]);
}

#[test]
fn held_note_ends_on_its_channel_after_selection_change() {
    let mut r = Router::default();
    route(&mut r, on(60, 100), 1, &[]);
    route(&mut r, on(62, 100), 1, &[]);
    // The selection moves to channel 3: new notes go there, held ones end where they started.
    assert_eq!(
        route(&mut r, on(64, 100), 3, &[]),
        [note(3, 64, true, 100.0 * 0.007874)]
    );
    assert_eq!(route(&mut r, off(60), 3, &[]), [note(1, 60, false, 0.0)]);
    // A note-off nobody holds goes to the selected channel.
    assert_eq!(route(&mut r, off(60), 3, &[]), [note(3, 60, false, 0.0)]);
    assert_eq!(
        r.release_all(),
        [note(1, 62, false, 0.0), note(3, 64, false, 0.0)]
    );
    assert!(r.release_all().is_empty());
}

#[test]
fn retrigger_on_another_channel_releases_the_first() {
    let mut r = Router::default();
    route(&mut r, on(60, 100), 1, &[]);
    assert_eq!(
        route(&mut r, on(60, 100), 2, &[]),
        [note(1, 60, false, 0.0), note(2, 60, true, 100.0 * 0.007874)]
    );
    assert_eq!(r.release_all(), [note(2, 60, false, 0.0)]);
}

#[test]
fn retrigger_on_the_same_channel_sends_no_note_off() {
    let mut r = Router::default();
    route(&mut r, on(60, 100), 1, &[]);
    assert_eq!(
        route(&mut r, on(60, 90), 1, &[]),
        [note(1, 60, true, 90.0 * 0.007874)]
    );
}

#[test]
fn each_device_holds_its_own_keys_and_a_record_only_target_gets_notes_but_no_params() {
    // Two keyboards on two channels press the same key: each note-off ends its own.
    let mut r = Router::default();
    let v = 100.0 * 0.007874;
    r.route("Keys", on(60, 100), &[(1, true)], &[]);
    r.route("Pads", on(60, 100), &[(2, true)], &[]);
    assert_eq!(
        r.route("Keys", off(60), &[(1, true)], &[]),
        [note(1, 60, false, 0.0)]
    );
    // Channel 3 only records: its notes say so, and a CC or pitch bend moves only the channels that sound.
    let both = [(1, true), (3, false)];
    let silent = |on, velocity| Out::Note {
        channel: 3,
        key: 64,
        on,
        velocity,
        sound: false,
    };
    assert_eq!(
        r.route("Keys", on(64, 100), &both, &[]),
        [note(1, 64, true, v), silent(true, v)]
    );
    assert_eq!(r.route("Keys", cc(7, 127), &both, &[]), [param(1, 14, 1.0)]);
    assert_eq!(
        r.route("Keys", off(64), &[], &[]),
        [note(1, 64, false, 0.0), silent(false, 0.0)]
    );
}

fn param(channel: usize, index: i32, value: f32) -> Out {
    Out::ChannelParam {
        channel,
        index,
        value,
    }
}

#[test]
fn ccs_become_channel_params() {
    let mut r = Router::default();
    // #7 = param 14 (the engine aliases it to Level), #64 = param 71 (sustain), #1 = param 8 (mod wheel).
    assert_eq!(route(&mut r, cc(7, 127), 1, &[]), [param(1, 14, 1.0)]);
    assert_eq!(route(&mut r, cc(64, 127), 1, &[]), [param(1, 71, 1.0)]);
    assert_eq!(route(&mut r, cc(1, 0), 1, &[]), [param(1, 8, 0.0)]);
    assert_eq!(route(&mut r, cc(126, 127), 1, &[]), [param(1, 133, 1.0)]);
    // There is no CC #127 channel param.
    assert!(route(&mut r, cc(127, 127), 1, &[]).is_empty());
}

#[test]
fn pitch_bend_pressure_and_program() {
    let mut r = Router::default();
    let bend = |value| MidiMsg::PitchBend { ch: 0, value };
    let [
        Out::ChannelParam {
            channel: 1,
            index: 4,
            value,
        },
    ] = route(&mut r, bend(8192), 1, &[])[..]
    else {
        panic!("pitch bend");
    };
    assert!((value - 0.50003).abs() < 1e-5, "{value}");
    assert_eq!(route(&mut r, bend(16383), 1, &[]), [param(1, 4, 1.0)]);
    assert_eq!(route(&mut r, bend(0), 1, &[]), [param(1, 4, 0.0)]);
    let pressure = MidiMsg::ChannelPressure { ch: 0, value: 127 };
    assert_eq!(route(&mut r, pressure, 1, &[]), [param(1, 5, 1.0)]);
    let program = MidiMsg::Program { ch: 0, program: 5 };
    assert!(route(&mut r, program, 1, &[]).is_empty());
}

fn link(cc: u8, instance_id: i32, param: i32) -> CcLink {
    CcLink {
        cc,
        instance_id,
        param,
    }
}

fn linked(channel: usize, instance_id: i32, param: i32, value: f32) -> Out {
    Out::Linked {
        channel,
        instance_id,
        param,
        value,
    }
}

#[test]
fn a_cc_moves_its_links_on_every_channel() {
    let mut r = Router::default();
    let links = [
        (4, link(20, 2, 6)),
        (2, link(20, -3, 0)),
        (4, link(21, 2, 5)),
    ];
    // A link fired: the selected channel's own CC param is left alone.
    assert_eq!(
        route(&mut r, cc(20, 127), 4, &links),
        [linked(4, 2, 6, 1.0), linked(2, -3, 0, 1.0)]
    );
    // Links fire on their own channel, whichever is selected.
    assert_eq!(route(&mut r, cc(21, 0), 1, &links), [linked(4, 2, 5, 0.0)]);
    // No link on the CC: the selected channel's CC param.
    assert_eq!(route(&mut r, cc(22, 0), 1, &links), [param(1, 29, 0.0)]);
    // A link on CC #127 still moves its param.
    assert_eq!(
        route(&mut r, cc(127, 0), 4, &[(3, link(127, 2, 1))]),
        [linked(3, 2, 1, 0.0)]
    );
}

#[test]
fn the_mod_wheel_reaches_the_channel_even_when_linked() {
    let mut r = Router::default();
    assert_eq!(
        route(&mut r, cc(1, 127), 4, &[(2, link(1, 2, 5))]),
        [linked(2, 2, 5, 1.0), param(4, 8, 1.0)]
    );
}

#[test]
fn learn_note_consumes_one_note_on_and_it_still_plays() {
    let mut r = Router::default();
    r.learn = Learn::Note;
    // CCs and note-offs don't learn.
    assert_eq!(route(&mut r, cc(1, 3), 1, &[]), [param(1, 8, 3.0 / 127.0)]);
    assert_eq!(route(&mut r, off(50), 1, &[]), [note(1, 50, false, 0.0)]);
    assert_eq!(
        route(&mut r, on(40, 127), 1, &[]),
        [note(1, 40, true, 127.0 * 0.007874), Out::LearnedNote(40)]
    );
    assert_eq!(r.learn, Learn::Off);
    assert_eq!(
        route(&mut r, on(41, 127), 1, &[]),
        [note(1, 41, true, 127.0 * 0.007874)]
    );
}

#[test]
fn learn_cc_consumes_the_next_cc() {
    let mut r = Router::default();
    r.learn = Learn::Cc;
    // Notes pass through and keep the learn waiting.
    assert_eq!(
        route(&mut r, on(40, 127), 1, &[]),
        [note(1, 40, true, 127.0 * 0.007874)]
    );
    assert_eq!(r.learn, Learn::Cc);
    // Any CC learns, channel-mode numbers included; it is not forwarded, not even to its existing links.
    assert_eq!(
        route(&mut r, cc(123, 64), 1, &[(1, link(123, 2, 0))]),
        [Out::LearnedCc(123)]
    );
    assert_eq!(r.learn, Learn::Off);
    assert_eq!(route(&mut r, cc(123, 127), 1, &[]), [param(1, 130, 1.0)]);
}

#[test]
fn song_links_list_every_channel() {
    let mut song = Song::default();
    song.channels.push(song.channels[0].clone());
    song.channels[0].file.cc_links = raw(&[[1, 0, 14]]);
    song.channels[1].file.cc_links = raw(&[[7, 2, 5], [-1, 0, 0]]);
    assert_eq!(
        links::song_links(&song),
        [(0, link(1, 0, 14)), (1, link(7, 2, 5))]
    );
}

fn raw(records: &[[i32; 3]]) -> Vec<u8> {
    records
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect()
}

#[test]
fn links_follow_the_cclk_rule() {
    let bytes = raw(&[
        [1, 2, 5],
        [-1, 2, 6],  // a < 0
        [7, 2, -1],  // c < 0
        [1, 2, 5],   // duplicate
        [128, 2, 7], // no such CC
        [64, 0, 14],
        [10, -3, 2], // a drum slot
        [5, 2, 0],   // param 0 is valid
        [127, 0, 14],
    ]);
    let mut with_tail = bytes.clone();
    with_tail.extend([0xAA; 5]); // a partial record is ignored
    let want = [
        link(1, 2, 5),
        link(64, 0, 14),
        link(10, -3, 2),
        link(5, 2, 0),
        link(127, 0, 14),
    ];
    assert_eq!(links::links(&bytes), want);
    assert_eq!(links::links(&with_tail), want);
    assert!(links::links(&[]).is_empty());
}

#[test]
fn common_params_link_through_their_positive_ids() {
    // Post Gain, Mix, Wet Pan of a module are stored as 0x7FC0…0x7FC2 (c < 0 would be dropped).
    let mut bytes = Vec::new();
    let common = [
        AutomationTarget::POST_GAIN,
        AutomationTarget::MIX,
        AutomationTarget::WET_PAN,
    ];
    let wanted: Vec<_> = common.iter().map(|&p| link(1, 2, p)).collect();
    links::set_links(&mut bytes, &wanted);
    assert_eq!(
        bytes,
        raw(&[[1, 2, 0x7FC0], [1, 2, 0x7FC1], [1, 2, 0x7FC2]])
    );
    assert_eq!(links::links(&bytes), wanted);
}

#[test]
fn link_appends_a_new_link_and_unlink_removes_all_of_the_param() {
    // A param may have several CCs; only an exact duplicate is not stored.
    let mut bytes = raw(&[[1, 2, 5], [3, 2, 5], [9, 0, 14]]);
    assert!(links::linked(&bytes, 2, 5));
    assert!(!links::linked(&bytes, 2, 6));
    links::link(&mut bytes, 20, 2, 5);
    assert_eq!(bytes, raw(&[[1, 2, 5], [3, 2, 5], [9, 0, 14], [20, 2, 5]]));
    links::link(&mut bytes, 1, 2, 6);
    assert_eq!(
        bytes,
        raw(&[[1, 2, 5], [3, 2, 5], [9, 0, 14], [20, 2, 5], [1, 2, 6]])
    );

    let mut two = raw(&[[1, 2, 5], [3, 2, 5], [9, 0, 14]]);
    links::unlink(&mut two, 2, 5);
    assert_eq!(two, raw(&[[9, 0, 14]]));
    assert!(!links::linked(&two, 2, 5));
    links::unlink(&mut two, 0, 14);
    assert!(two.is_empty());
}

#[test]
fn no_op_link_and_unlink_keep_the_raw_bytes() {
    // The invalid record survives as long as no link really changes.
    let before = raw(&[[1, 2, 5], [-1, 2, 6]]);
    let mut bytes = before.clone();
    links::link(&mut bytes, 1, 2, 5);
    assert_eq!(bytes, before);
    links::unlink(&mut bytes, 2, 9);
    assert_eq!(bytes, before);
    // A real change rewrites the valid links: the invalid record goes.
    links::link(&mut bytes, 2, 2, 5);
    assert_eq!(bytes, raw(&[[1, 2, 5], [2, 2, 5]]));
}

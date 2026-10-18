use honeydew::flm::{
    chunk::{FlmErrorKind, PrefixKind, file_chunks},
    read, write,
};
use honeydew::song::{
    AutomationTarget, ChannelKind, Event, Raw, SamplerSlot, SlotFileState, Song, SynthKind,
};

mod common;
use common::{every_feature_song, grid};

#[test]
fn new_song_writes_a_stable_readable_project() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let clip = song.channels[1].lanes[0].add_clip(128, 4.0);
    clip.events.push(Event::Note {
        pos: 0,
        len_beats: 1.0,
        key: 60,
        velocity: 204,
        pan: 127,
        slide: false,
    });
    let expected_events = clip.events.clone();
    let bytes = write(&song).unwrap();
    assert_eq!(write(&song).unwrap(), bytes);
    let loaded = read(&bytes).unwrap();
    assert_eq!(loaded.channels.len(), 2);
    assert_eq!(loaded.channels[1].lanes[0].clips[0].events, expected_events);
    assert_eq!(loaded.channels[1].rack.modules[0].type_id, 0);
    assert_eq!(
        loaded.channels[1].kind,
        ChannelKind::Synth(SynthKind::MiniSynth)
    );
    assert_eq!(read(&write(&loaded).unwrap()).unwrap(), loaded);
}

#[test]
fn drum_slots_and_common_param_automation_survive_round_trip() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let drums = &mut song.channels[1];
    let slot = SamplerSlot {
        name: "Kick".into(),
        folder: "Drum Samples/Kick".into(),
        file: honeydew::song::SlotFileState {
            id: 1,
            path: Some(honeydew::flm::chunk::ContentPath::Shorthand {
                root: honeydew::flm::chunk::ContentRoot::App,
                relative: "Drum Samples/Kick.wav".into(),
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    drums.drum_kit.as_mut().unwrap().slots = vec![slot];
    let module_id = drums.rack.add(9);
    let lane = drums.add_automation_lane(AutomationTarget {
        instance_id: module_id,
        param: -1,
    });
    lane.add_clip(0, 4.0).events.push(Event::Automation {
        pos: 0,
        value: 0.5,
        control: true,
    });
    let loaded = read(&write(&song).unwrap()).unwrap();
    assert_eq!(loaded.channels[1].kind, ChannelKind::Drums);
    assert_eq!(
        loaded.channels[1].drum_kit.as_ref().unwrap().slots[0].name,
        "Kick"
    );
    assert_eq!(loaded.channels[1].lanes[1].target.unwrap().param, -1);
    assert_eq!(read(&write(&loaded).unwrap()).unwrap(), loaded);
}

#[test]
fn all_registered_module_types_save_and_load() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let types = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 19, 20, 23, 24, 25, 26, 27,
        28, 29, 30, 32, 33, 34, 35, 36, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52,
    ];
    for type_id in types {
        song.channels[1].rack.add(type_id);
    }
    let loaded = read(&write(&song).unwrap()).unwrap();
    assert_eq!(
        loaded.channels[1]
            .rack
            .modules
            .iter()
            .map(|module| module.type_id)
            .collect::<Vec<_>>(),
        types
    );
    assert_eq!(read(&write(&loaded).unwrap()).unwrap(), loaded);
}

#[test]
fn channel_names_survive_save_after_an_edit_or_duplicate() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::Drums);
    song.channels[1].name = "Custom Kit".into();
    let loaded = read(&write(&song).unwrap()).unwrap();
    assert_eq!(loaded.channels[1].name, "Custom Kit");
    assert_eq!(loaded.channels[2].name, "Drums 2");
}

#[test]
fn common_parameter_indices_round_trip_through_disk_aliases() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let id = song.channels[1].rack.modules[0].instance_id;
    for param in [-1, -2, -3, -4] {
        let lane = song.channels[1].add_automation_lane(AutomationTarget {
            instance_id: id,
            param,
        });
        if param != -4 {
            lane.add_clip(0, 4.0).events.push(Event::Automation {
                pos: 0,
                value: 0.5,
                control: true,
            });
        }
    }
    let bytes = write(&song).unwrap();
    let loaded = read(&bytes).unwrap();
    let mut params: Vec<_> = loaded.channels[1]
        .lanes
        .iter()
        .filter_map(|lane| lane.target.map(|target| target.param))
        .collect();
    params.sort();
    assert_eq!(params, [-4, -3, -2, -1]);
    // The app's codes: Post Gain −3 and Mix −2, Wet Pan 0x7FC2; a control point's key is +(param + 1).
    let lanes = file_lanes(&bytes);
    assert!(lanes.contains(&(-3, vec![-2])), "{lanes:?}");
    assert!(lanes.contains(&(-2, vec![-1])), "{lanes:?}");
    assert!(lanes.contains(&(0x7fc2, vec![0x7fc3])), "{lanes:?}");
    assert!(lanes.contains(&(-1, vec![])), "{lanes:?}");
    // And in the app's lane order: by the file code, so Wet Pan, Post Gain, Mix.
    let codes: Vec<i32> = lanes.iter().map(|l| l.0).filter(|&p| p != 0).collect();
    // (The MiniSynth's notes lane comes first, with −1.)
    assert_eq!(codes, [-1, 0x7fc2, -3, -2, -1]);
    // A retargeted lane gets new keys.
    let mut retargeted = loaded;
    let lane = (retargeted.channels[1].lanes.iter_mut())
        .find(|l| {
            l.target
                .is_some_and(|t| t.param == AutomationTarget::POST_GAIN)
        })
        .unwrap();
    lane.target.as_mut().unwrap().param = AutomationTarget::MIX;
    let lanes = file_lanes(&write(&retargeted).unwrap());
    assert_eq!(lanes.iter().filter(|l| l.0 == -2 && l.1 == [-1]).count(), 2);
}

/// Every TRKH of every CHNL as (DESc param, the EVN2 keys).
fn file_lanes(bytes: &[u8]) -> Vec<(i32, Vec<i16>)> {
    let mut lanes = Vec::new();
    for chunk in file_chunks(bytes).unwrap().map(Result::unwrap) {
        if chunk.id != *b"CHNL" {
            continue;
        }
        let (_, children) = chunk.container(PrefixKind::U32Magic).unwrap();
        for lane in children.map(Result::unwrap).filter(|c| c.id == *b"TRKH") {
            let (mut param, mut keys) = (0, Vec::new());
            for child in lane.children().map(Result::unwrap) {
                if child.id == *b"DESc" {
                    param = i32::from_le_bytes(child.payload[8..12].try_into().unwrap());
                }
                if child.id == *b"CLIP" {
                    let (_, clip) = child.container(PrefixKind::U32Magic).unwrap();
                    for events in clip.map(Result::unwrap).filter(|e| e.id == *b"EVN2") {
                        let records = events.payload[2..].as_chunks::<20>().0.iter();
                        keys.extend(records.map(|r| i16::from_le_bytes([r[12], r[13]])));
                    }
                }
            }
            lanes.push((param, keys));
        }
    }
    lanes
}

#[test]
fn generated_steps_on_a_common_param_lane_take_the_other_sign() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 2,
        param: AutomationTarget::MIX,
    });
    let clip = lane.add_clip(0, 4.0);
    for (pos, control) in [(0, true), (8, false), (16, true)] {
        clip.events.push(Event::Automation {
            pos,
            value: grid(32768),
            control,
        });
    }
    let bytes = write(&song).unwrap();
    assert!(file_lanes(&bytes).contains(&(-2, vec![-1, 1, -1])));
    assert_eq!(read(&bytes).unwrap(), song);
}

#[test]
fn a_song_with_every_feature_reads_back_equal() {
    let song = every_feature_song();
    let bytes = write(&song).unwrap();
    assert_eq!(read(&bytes).unwrap(), song);
    assert_eq!(write(&read(&bytes).unwrap()).unwrap(), bytes);
    let mut new = Song::default();
    new.add_channel(ChannelKind::Drums);
    assert_eq!(read(&write(&new).unwrap()).unwrap(), new);
}

#[test]
fn the_metadata_lock_is_meta_byte_776_and_reads_back() {
    // META@776: u8, 1 = locked; a new empty song = 0.
    let meta_776 = |song: &Song| {
        let bytes = write(song).unwrap();
        let meta = file_chunks(&bytes)
            .unwrap()
            .map(Result::unwrap)
            .find(|c| &c.id == b"META")
            .unwrap();
        assert_eq!(meta.payload.len(), 3081);
        (meta.payload[776], read(&bytes).unwrap().file.meta.locked)
    };
    let mut song = Song::default();
    assert_eq!(meta_776(&song), (0, false));
    song.file.meta.locked = true;
    assert_eq!(meta_776(&song), (1, true));
}

#[test]
fn a_new_drums_channel_saves_playable_pads() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    song.channels[1].drum_kit.as_mut().unwrap().slots[3]
        .file
        .midi_key = 60;
    let read_back = read(&write(&song).unwrap()).unwrap();
    let (kit, saved) = (
        read_back.channels[1].drum_kit.as_ref().unwrap(),
        song.channels[1].drum_kit.as_ref().unwrap(),
    );
    assert_eq!(kit, saved);
    let ids: Vec<u32> = kit.slots.iter().map(|s| s.file.id).collect();
    assert_eq!(ids, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(kit.file.last_slot_id, 8);
    assert_eq!(kit.slots[0].folder, "Built-in/Kick");
    assert_eq!(kit.slots[3].cut_group, 1);
    assert_eq!(
        (0..8).map(|p| kit.pad_note(p)).collect::<Vec<_>>(),
        [48, 49, 50, 60, 52, 53, 54, 55].map(Some)
    );
    // MAIN @4 holds the pad index 0, 1, 2 …
    let bytes = write(&song).unwrap();
    let pads: Vec<f64> = (bytes.windows(4).enumerate())
        .filter(|(_, w)| *w == b"MAIN")
        .map(|(at, _)| f64::from_le_bytes(bytes[at + 12..at + 20].try_into().unwrap()))
        .collect();
    assert_eq!(pads, [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
}

#[test]
fn a_cstm_is_written_only_where_the_spec_allows_one() {
    let mut song = Song::default();
    let eq = song.channels[0].rack.add(3);
    song.channels[0].rack.get_mut(eq).unwrap().file.custom_data = Some(vec![1, 2, 3]);
    assert_eq!(write(&song).unwrap_err().kind, FlmErrorKind::BadValue);
    // Trance Delay may carry its 1-byte flag, and an empty blob; a short Tuner blob may not.
    let mut song = Song::default();
    let delay = song.channels[0].rack.add(16);
    song.channels[0]
        .rack
        .get_mut(delay)
        .unwrap()
        .file
        .custom_data = Some(vec![]);
    assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
    let tuner = song.channels[0].rack.add(24);
    song.channels[0]
        .rack
        .get_mut(tuner)
        .unwrap()
        .file
        .custom_data = Some(vec![0; 12]);
    assert_eq!(write(&song).unwrap_err().kind, FlmErrorKind::TooShort);
}

#[test]
fn channel_names_are_custom_only_when_they_are_not_the_auto_name() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::Instrument);
    song.channels[2].name = "Drums 2".into();
    song.channels[3].file.preset_name = "Piano".into();
    song.channels[3].name = "Piano".into();
    let bytes = write(&song).unwrap();
    // (base, preset, custom) per CHHD: the base is "<Kind> <id>"; our names that differ are custom.
    let names = chhd_names(&bytes);
    assert_eq!(
        names,
        [
            ["MASTER", "", ""],
            ["Drums 1", "", "Drums"],
            ["Drums 2", "", ""],
            ["Instrument 3", "Piano", ""],
        ]
    );
    assert_eq!(read(&bytes).unwrap(), song);
}

#[test]
fn a_new_send_channel_is_named_effect_and_its_id_and_saves_that_base_name() {
    // A new send channel is named `Effect <channel ID>`.
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::EffectTrack);
    assert_eq!(song.channels[2].name, "Effect 2");
    let bytes = write(&song).unwrap();
    // The file keeps the kinds in order: master, sends, the rest.
    assert_eq!(chhd_names(&bytes)[1], ["Effect 2", "", ""]);
    assert_eq!(read(&bytes).unwrap(), song);
}

fn chhd_names(bytes: &[u8]) -> Vec<[String; 3]> {
    let mut all = Vec::new();
    for at in (0..bytes.len() - 4).filter(|&at| &bytes[at..at + 4] == b"CHHD") {
        let mut cursor = honeydew::flm::chunk::Cursor::new(&bytes[at + 8 + 1084..], "");
        let mut name = || String::from_utf8(cursor.vstr32().unwrap().to_vec()).unwrap();
        all.push([name(), name(), name()]);
    }
    all
}

#[test]
fn the_masters_level_lane_is_saved_with_its_auto_name() {
    let bytes = write(&Song::default()).unwrap();
    let at = bytes.windows(4).position(|w| w == b"DESc").unwrap() + 8;
    assert_eq!(&bytes[at + 28..at + 34], b"Level\0");
}

#[test]
fn the_header_holds_the_counters_and_a_new_song_the_default_chord_set() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    song.channels[1].bus_id = 6;
    song.file.next_channel_id = 4095;
    let bytes = write(&song).unwrap();
    let head = bytes.windows(4).position(|w| w == b"HEAD").unwrap() + 8;
    let i32_at =
        |at: usize| i32::from_le_bytes(bytes[head + at..head + at + 4].try_into().unwrap());
    assert_eq!(i32_at(4), 6, "HEAD@4 ≥ the highest bus");
    assert_eq!(i32_at(354), 4095, "HEAD@354 below 4096");
    let chords = honeydew::song::DEFAULT_CHORD_JSON.as_bytes();
    assert!(bytes.windows(chords.len()).any(|w| w == chords));
    // RHED[8]: the send's bus, 0 on every other channel.
    let buses: Vec<i32> = (bytes.windows(4).enumerate())
        .filter(|(_, w)| *w == b"RHED")
        .map(|(at, _)| i32::from_le_bytes(bytes[at + 16..at + 20].try_into().unwrap()))
        .collect();
    assert_eq!(buses, [0, 6, 0]);
}

#[test]
fn ids_up_to_4095_save_and_the_rest_are_refused() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    song.channels[1].id = 4095;
    assert!(write(&song).is_ok());
    song.channels[1].id = 4096;
    assert_eq!(write(&song).unwrap_err().kind, FlmErrorKind::BadValue);
    song.channels[1].id = 0;
    assert_eq!(write(&song).unwrap_err().kind, FlmErrorKind::BadValue);
    // One master, first, with id 0.
    let mut moved = Song::default();
    moved.channels[0].id = 5;
    assert_eq!(write(&moved).unwrap_err().kind, FlmErrorKind::BadValue);
    let mut two = Song::default();
    two.add_channel(ChannelKind::Drums);
    two.channels[1].kind = ChannelKind::Master;
    assert_eq!(write(&two).unwrap_err().kind, FlmErrorKind::BadValue);
    // A send needs a bus of its own, ≥ 1.
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::EffectTrack);
    song.channels[2].bus_id = 1;
    assert_eq!(write(&song).unwrap_err().kind, FlmErrorKind::BadValue);
    song.channels[2].bus_id = 0;
    assert_eq!(write(&song).unwrap_err().kind, FlmErrorKind::BadValue);
}

#[test]
fn folders_categories_and_sample_paths_are_written_as_the_app_writes_them() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let drums = &mut song.channels[1];
    let kit = drums.drum_kit.as_mut().unwrap();
    kit.slots[0].folder = "Drum Samples\\Kick".into();
    kit.slots[1].file.path = Some(honeydew::flm::chunk::ContentPath::Plain(
        "/tmp/x.wav".into(),
    ));
    kit.slots[2].file.path = Some(honeydew::flm::chunk::ContentPath::Shorthand {
        root: honeydew::flm::chunk::ContentRoot::Temp,
        relative: "x.wav".into(),
    });
    let fx = drums.rack.add(9);
    drums.rack.get_mut(fx).unwrap().file.category = "Effects\\Filters".into();
    let bytes = write(&song).unwrap();
    assert!(bytes.windows(17).any(|w| w == b"Drum Samples/Kick"));
    assert!(bytes.windows(15).any(|w| w == b"Effects/Filters"));
    // Only a shorthand into the content or user root is a sample path: the others write no PTH1.
    assert!(!bytes.windows(4).any(|w| w == b"PTH1"));
}

#[test]
fn kept_event_bytes_go_back_until_the_events_change() {
    // An automation clip whose junk bytes (a, b) are not the ones we write: read and written back unchanged.
    let mut song = Song::default();
    song.channels[0].lanes[0]
        .add_clip(0, 4.0)
        .events
        .push(Event::Automation {
            pos: 0,
            value: 0.5,
            control: true,
        });
    let mut bytes = write(&song).unwrap();
    let evn2 = bytes.windows(4).position(|w| w == b"EVN2").unwrap() + 8;
    bytes[evn2 + 2 + 14] = 0x11;
    let loaded = read(&bytes).unwrap();
    assert_eq!(write(&loaded).unwrap(), bytes);
    // After an edit the clip is encoded afresh.
    let mut edited = loaded;
    let Event::Automation { value, .. } = &mut edited.channels[0].lanes[0].clips[0].events[0]
    else {
        panic!()
    };
    *value = grid(16384);
    let again = write(&edited).unwrap();
    assert_eq!(again[evn2 + 2 + 14], 0xb2);
    assert_eq!(read(&again).unwrap(), edited);
}
#[test]
fn complete_reader_never_panics_on_corruptions() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.channels[1].lanes[0]
        .add_clip(0, 4.0)
        .events
        .push(Event::Note {
            pos: 0,
            len_beats: 1.0,
            key: 60,
            velocity: 204,
            pan: 127,
            slide: false,
        });
    let bytes = write(&song).unwrap();
    let mut seed = 0x1234_5678_u64;
    for _ in 0..10_000 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let mut damaged = bytes.clone();
        let index = seed as usize % damaged.len();
        damaged[index] ^= (seed >> 24) as u8 | 1;
        let length = if seed & 1 == 0 {
            damaged.len()
        } else {
            seed as usize % damaged.len()
        };
        assert!(std::panic::catch_unwind(|| read(&damaged[..length])).is_ok());
    }
}

#[test]
fn optional_owner_projects_round_trip() {
    let Ok(folder) = std::env::var("STUDIO_FLM_FIXTURES") else {
        return;
    };
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if !matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("flm" | "songtmpl")
        ) {
            continue;
        }
        let original = read(&std::fs::read(&path).unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let loaded = read(&write(&original).unwrap()).unwrap();
        assert_eq!(loaded, original, "{}", path.display());
    }
}

#[test]
fn every_tail_and_extra_reads_back() {
    let mut song = every_feature_song();
    let tail = |n: u8| vec![n, n + 1, n + 2];
    song.file.head_extra = tail(1);
    song.file.time_sig_extra = tail(2);
    song.file.meta.extra = tail(3);
    song.file.keyboard.extra = tail(4);
    let drums = &mut song.channels[1];
    drums.file.rack_header_extra = tail(5);
    drums.file.params_extra = tail(6);
    drums.file.header_extra = tail(7);
    drums.file.cc_links = vec![1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0];
    drums.file.sends[0].extra = tail(8);
    drums.lanes[0].file.desc_extra = tail(9);
    drums.lanes[0].clips[0].file.header_extra = tail(10);
    let module = &mut drums.rack.modules[0].file;
    (module.head_extra, module.desc_extra, module.add1_extra) = (tail(11), tail(12), tail(13));
    let slots = &mut drums.drum_kit.as_mut().unwrap().slots;
    (slots[0].file.main_extra, slots[0].file.link_extra) = (tail(14), tail(15));
    // A pad that was not renamed but has bytes after MAIN @1051: the empty pad name goes first, or the bytes would
    // read as the name.
    slots[1].file.main_extra = vec![2, 0, b'h', b'i', 7];
    assert_eq!(slots[1].file.custom_pad_name, "");
    let bytes = write(&song).unwrap();
    assert_eq!(read(&bytes).unwrap(), song);
    // RHED, RPRM, CCLK, RSMP: the CCLK keeps its place, right after the 3-byte-tailed RPRM.
    let cclk = bytes.windows(4).position(|w| w == b"CCLK").unwrap();
    assert_eq!(&bytes[cclk - 8 - 27..cclk - 27], b"RPRM\x1b\0\0\0");
    assert_eq!(&bytes[cclk + 20..cclk + 24], b"RSMP");
}

/// The first EVN2 record after `from` in `bytes`.
fn record_after(bytes: &[u8], from: usize) -> [u8; 20] {
    let at = from + bytes[from..].windows(4).position(|w| w == b"EVN2").unwrap() + 8 + 2;
    bytes[at..at + 20].try_into().unwrap()
}

#[test]
fn automation_and_envelope_records_are_the_apps_bytes() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::AudioClip);
    song.channels[0].lanes[0]
        .add_clip(0, 4.0)
        .events
        .push(Event::Automation {
            pos: 3,
            value: grid(0x1234),
            control: false,
        });
    song.channels[1].lanes[0]
        .add_clip(0, 4.0)
        .events
        .push(Event::Envelope {
            pos: 5,
            env_param: 2,
            value: 1.0,
            control: true,
        });
    let bytes = write(&song).unwrap();
    // pos, length 0.0, key −(0 + 1), 0xB2, 0x7F, value, 0, 0.
    let mut automation = [0; 20];
    automation[0] = 3;
    automation[12..14].copy_from_slice(&(-1_i16).to_le_bytes());
    automation[14..18].copy_from_slice(&[0xb2, 0x7f, 0x34, 0x12]);
    assert_eq!(record_after(&bytes, 0), automation);
    let audio = bytes.windows(4).rposition(|w| w == b"CLIP").unwrap();
    let mut envelope = [0; 20];
    envelope[0] = 5;
    envelope[12..14].copy_from_slice(&3_i16.to_le_bytes());
    envelope[14..18].copy_from_slice(&[0xb2, 0x7f, 0xff, 0xff]);
    assert_eq!(record_after(&bytes, audio), envelope);
}

#[test]
fn an_event_chunk_without_a_record_size_is_written_back_as_it_was() {
    let mut song = Song::default();
    song.channels[0].lanes[0].add_clip(0, 4.0);
    let mut bytes = write(&song).unwrap();
    // The EVN2 payload [20, 0] becomes [0, 0]: record size 0, so no events (each field is read only if R reaches
    // it); not what the writer would write, so the bytes are kept.
    let at = bytes.windows(4).position(|w| w == b"EVN2").unwrap();
    bytes[at + 8] = 0;
    let loaded = read(&bytes).unwrap();
    assert!(loaded.channels[0].lanes[0].clips[0].events.is_empty());
    assert_eq!(write(&loaded).unwrap(), bytes);
}

#[test]
fn the_rack_header_and_kit_counters_cover_what_the_song_holds() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    let send = song.channels[1].id;
    let drums = &mut song.channels[2];
    drums.file.sends[0].target_channel_id = send;
    drums.drum_kit.as_mut().unwrap().file.last_slot_id = 0;
    drums.lanes[0].add_clip(0, 4.0).id = 3;
    drums.lanes[0].last_clip_id = 0;
    let bytes = write(&song).unwrap();
    // DESc@20 covers clip id 3.
    let desc = bytes.windows(4).rposition(|w| w == b"DESc").unwrap() + 8;
    assert_eq!(
        u32::from_le_bytes(bytes[desc + 20..desc + 24].try_into().unwrap()),
        3
    );
    let rheds: Vec<[i32; 4]> = (bytes.windows(4).enumerate())
        .filter(|(_, w)| *w == b"RHED")
        .map(|(at, _)| {
            std::array::from_fn(|i| {
                i32::from_le_bytes(bytes[at + 8 + 4 * i..at + 12 + 4 * i].try_into().unwrap())
            })
        })
        .collect();
    // The drums output to the send's bus 1 (RHED[12], from the SEND); every module counter at least 1.
    assert_eq!(rheds, [[-1, 1, 0, -1], [-1, 1, 1, -1], [-1, 1, 0, 1]]);
    // lastSlotId covers slot ids 1..8.
    let rsmp = bytes.windows(4).rposition(|w| w == b"RSMP").unwrap() + 12;
    assert_eq!(
        u32::from_le_bytes(bytes[rsmp..rsmp + 4].try_into().unwrap()),
        8
    );
}

/// CHHD @1028 of each CHNL, in file order.
fn rows(bytes: &[u8]) -> Vec<f64> {
    (bytes.windows(4).enumerate())
        .filter(|(_, w)| *w == b"CHHD")
        .map(|(at, _)| f64::from_le_bytes(bytes[at + 8 + 1028..at + 8 + 1036].try_into().unwrap()))
        .collect()
}

#[test]
fn a_playlist_row_counts_tracks_and_a_zipped_channel_as_one() {
    // @1028: master (1 lane), a 2-lane channel, then one more.
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    song.add_channel(ChannelKind::Instrument);
    song.channels[1].add_lane(honeydew::song::LaneKind::Notes);
    assert_eq!(rows(&write(&song).unwrap()), [0.0, 1.0, 3.0]);
    song.channels[1].zipped = true;
    let bytes = write(&song).unwrap();
    assert_eq!(rows(&bytes), [0.0, 1.0, 2.0]);
    assert_eq!(read(&bytes).unwrap(), song);
}

#[test]
fn kept_automation_bytes_are_not_written_for_a_retargeted_lane() {
    // A Level clip whose junk byte keeps its bytes; the lane then targets Pan (param 1): the keys must become ±2.
    let mut song = Song::default();
    song.channels[0].lanes[0]
        .add_clip(0, 4.0)
        .events
        .push(Event::Automation {
            pos: 0,
            value: 0.5,
            control: true,
        });
    let mut bytes = write(&song).unwrap();
    let evn2 = bytes.windows(4).position(|w| w == b"EVN2").unwrap() + 8;
    bytes[evn2 + 2 + 14] = 0x11;
    let mut loaded = read(&bytes).unwrap();
    loaded.channels[0].lanes[0].target.as_mut().unwrap().param = 1;
    let written = write(&loaded).unwrap();
    assert_eq!(record_after(&written, 0)[12..14], 2_i16.to_le_bytes());
}

// ---- `.flmpst` and `.flmrack` ----

/// A module with every chunk set: params, common params, `CSTM`, `DESC` / `PRST` and the `ADD1` flags.
fn preset_module(type_id: u16, instance_id: i32) -> honeydew::song::ModuleInstance {
    let mut module = honeydew::song::ModuleInstance::new(type_id, instance_id);
    let n = module.params.len();
    for (i, p) in module.params.iter_mut().enumerate() {
        *p = i as f32 / n as f32;
    }
    module.common.0 = [0.4, 0.6, 0.3];
    module.minimized = true;
    // Only these modules may carry a CSTM.
    module.file.custom_data = matches!(type_id, 2 | 16).then(|| vec![1, 2, 3, 4, 5, 6, 7, 8]);
    module.file.category = "Echoes".into();
    module.file.preset_name = "Long".into();
    module.file.preset_path = "My Presets/Echoes/Long.flmpst".into();
    module.file.source_path = "My Presets/Echoes/Long.flmpst".into();
    module.file.provenance = "Echoes/Long".into();
    module
}

#[test]
fn a_module_preset_is_an_rmod_body_and_reads_back_equal_but_for_the_id() {
    use honeydew::flm::{read_preset, write_preset};
    // Trance Delay (16) and GMSynth (2) may carry a CSTM.
    for type_id in [16, 2] {
        let module = preset_module(type_id, 7);
        let bytes = write_preset(&module).unwrap();
        // A bare chunk list, no magic and no 8-byte prefix: HEAD names the module type.
        assert_eq!(&bytes[..4], b"HEAD");
        assert_eq!(bytes[8..12], u32::from(type_id).to_le_bytes());
        let ids: Vec<[u8; 4]> = honeydew::flm::chunk::ChunkIter::new(&bytes, "")
            .map(|c| c.unwrap().id)
            .collect();
        assert_eq!(
            ids,
            [
                *b"HEAD", *b"DESC", *b"PRST", *b"ADD1", *b"PRMS", *b"SMPR", *b"CSTM"
            ]
        );
        let back = read_preset(&bytes, None).unwrap();
        assert_eq!(back.params, module.params);
        assert_eq!(back.file.custom_data, module.file.custom_data);
        // The instance id is the rack's business: a read preset has none.
        assert_eq!(back.instance_id, 0);
        let module = honeydew::song::ModuleInstance {
            instance_id: 0,
            ..module
        };
        assert_eq!(back, module);
    }
    // No HEAD, or a module type nobody knows: no preset.
    assert!(read_preset(b"", None).is_err());
    let mut unknown = write_preset(&preset_module(16, 2)).unwrap();
    unknown[8..12].copy_from_slice(&17_u32.to_le_bytes());
    assert!(read_preset(&unknown, None).is_err());
}

#[test]
fn a_rack_preset_keeps_the_chain_and_takes_fresh_ids_on_read() {
    use honeydew::flm::{read_rack, write_rack};
    let modules = [
        preset_module(51, 9),
        preset_module(2, 4),
        preset_module(16, 12),
    ];
    let bytes = write_rack(&modules).unwrap();
    // RMOd chunks with the full prefix, the saving rack's type and id, in chain order.
    let prefixes: Vec<([u8; 4], u32, u32)> = honeydew::flm::chunk::ChunkIter::new(&bytes, "")
        .map(|c| {
            let c = c.unwrap();
            let word = |at: usize| u32::from_le_bytes(c.payload[at..at + 4].try_into().unwrap());
            (c.id, word(0), word(4))
        })
        .collect();
    assert_eq!(
        prefixes,
        [(*b"RMOd", 51, 9), (*b"RMOd", 2, 4), (*b"RMOd", 16, 12)]
    );
    // Read into a rack whose counter is at 40: the modules take 41, 42, 43, and the counter follows.
    let mut last = 40;
    let back = read_rack(&bytes, &mut last).unwrap();
    assert_eq!(last, 43);
    let ids: Vec<i32> = back.iter().map(|m| m.instance_id).collect();
    assert_eq!(ids, [41, 42, 43]);
    for (back, module) in back.iter().zip(&modules) {
        let module = honeydew::song::ModuleInstance {
            instance_id: back.instance_id,
            ..module.clone()
        };
        assert_eq!(*back, module);
    }
    // A broken file reads as nothing and leaves the counter where it was.
    let mut broken = bytes.clone();
    broken[8..12].copy_from_slice(&17_u32.to_le_bytes());
    assert!(read_rack(&broken, &mut last).is_err());
    assert_eq!(last, 43);
    // An empty rack is an empty file.
    assert_eq!(write_rack(&[]).unwrap(), Vec::<u8>::new());
    assert_eq!(read_rack(&[], &mut last).unwrap(), []);
}

/// Every `DESc`'s lane-type bytes @0 and @1, in file order.
fn lane_types(bytes: &[u8]) -> Vec<[u8; 2]> {
    let descs = bytes.windows(4).enumerate().filter(|(_, w)| *w == b"DESc");
    descs
        .map(|(at, _)| [bytes[at + 8], bytes[at + 9]])
        .collect()
}

#[test]
fn a_drum_lanes_grid_is_desc_byte_1_and_other_lanes_keep_their_byte() {
    use honeydew::song::{DrumGrid, LaneKind};
    // DESc@1: 0 = a step-sequencer lane, 1 = a notes lane (free note-grid), on drum lanes. A notes lane's hits
    // keep their lengths.
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::AudioClip);
    let drums = &mut song.channels[1];
    let notes = drums.add_lane(LaneKind::Drums);
    drums.lanes[notes].grid = DrumGrid::Notes;
    drums.lanes[notes]
        .add_clip(0, 4.0)
        .events
        .push(Event::Drum {
            pos: 16,
            len_beats: 0.75,
            pad: 2,
            velocity: 255,
            pan: 127,
            value: Event::DEFAULT_PITCH,
            repeat: 0,
        });
    // The audio template's 1 is kept as it was read.
    song.channels[2].lanes[0].file.flag = 1;
    let bytes = write(&song).unwrap();
    // Master Level, drums step and notes, audio.
    assert_eq!(lane_types(&bytes), [[1, 0], [3, 0], [3, 1], [2, 1]]);
    let back = read(&bytes).unwrap();
    assert_eq!(back, song);
    let grids: Vec<_> = back.channels[1].lanes.iter().map(|l| l.grid).collect();
    assert_eq!(grids, [DrumGrid::Steps, DrumGrid::Notes]);
    assert_eq!(back.channels[1].lanes[1].file.flag, 0);
    // Any other non-zero byte on a drum lane reads as a notes lane (and saves as 1).
    let mut odd = bytes.clone();
    let at = (odd.windows(4).enumerate())
        .filter(|(_, w)| *w == b"DESc")
        .nth(2)
        .unwrap()
        .0;
    odd[at + 9] = 7;
    let odd = read(&odd).unwrap();
    assert_eq!(odd.channels[1].lanes[1].grid, DrumGrid::Notes);
    assert_eq!(lane_types(&write(&odd).unwrap())[2], [3, 1]);
}

// ---- Drum kit templates, `.tmpl` ----

#[test]
fn a_drumset_is_lpmt_pads_and_slots_and_reads_back_equal() {
    use honeydew::flm::chunk::{ChunkIter, ContentPath, ContentRoot};
    use honeydew::flm::{read_drumset, write_drumset};
    use honeydew::song::DrumKit;
    let mut kit = DrumKit::default_kit();
    // A slot with a sample in the user folder, renamed, on a MIDI note, in a cut group, with a param moved.
    let slot = &mut kit.slots[1];
    slot.name = "Snare 02".into();
    slot.folder = "My Samples/Snares".into();
    slot.cut_group = 2;
    slot.params[0] = 0.25;
    slot.file.custom_pad_name = "Snap".into();
    slot.file.midi_key = 40;
    slot.file.path = Some(ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: "My Samples/Snares/Snare 02.wav".into(),
    });
    // A clone: a higher id on an earlier pad, which comes back on its pad.
    let copy = kit.slots[0].clone();
    kit.insert_slot(1, copy);
    let bytes = write_drumset(&kit).unwrap();
    // "LPMT" [PADS(22)] slot*, the slots `SMPl` (`20LS`) as in an RSMP.
    assert_eq!(&bytes[..4], b"LPMT");
    let chunks: Vec<_> = ChunkIter::new(&bytes[4..], "")
        .map(|c| c.unwrap())
        .collect();
    let ids: Vec<[u8; 4]> = chunks.iter().map(|c| c.id).collect();
    assert_eq!(ids, [vec![*b"PADS"], vec![*b"SMPl"; 9]].concat());
    assert_eq!(chunks[0].payload.len(), 22);
    assert!(chunks[1..].iter().all(|c| c.payload.starts_with(b"20LS")));
    let back = read_drumset(&bytes).unwrap();
    assert_eq!(back, kit);
    assert_eq!(back.slots[1].file.id, 9);
    // Not a drumset: another magic, or nothing.
    for bad in [&b""[..], b"LPM", &write(&Song::default()).unwrap()] {
        assert!(read_drumset(bad).is_err());
    }
}

#[test]
fn a_drumset_reads_legacy_smpl_slots_beside_smpl_ones() {
    use honeydew::flm::chunk::{ChunkWriter, ContentPath, ContentRoot};
    use honeydew::flm::{read_drumset, write_drumset};
    use honeydew::song::DrumKit;
    // One `SMPl` (`20LS`) kit of three pads, then two legacy `SMPL` (`10LS`) slots as 12 of the factory kits have them:
    // MAIN of 1037 and 1036 bytes, a LINk of 8 or none, PATH char[2048] and 10 PRMS. Their pads (MAIN@4) are
    // 4 and 3, in that file order. The kit's PADS map is not the identity.
    let mut pads: [u8; 22] = std::array::from_fn(|i| i as u8);
    pads.swap(0, 1);
    let mut first = DrumKit {
        slots: DrumKit::default_kit().slots[..3].to_vec(),
        ..DrumKit::default()
    };
    first.file.pads = pads;
    let mut bytes = write_drumset(&first).unwrap();
    let mut w = ChunkWriter::default();
    for (pad, name, link) in [(4.0, "Snare", true), (3.0, "Kick", false)] {
        w.chunk(*b"SMPL", |w| {
            w.raw(b"10LS")?;
            w.chunk(*b"MAIN", |w| {
                w.i32(-1)?;
                w.f64(pad)?;
                w.char::<512>(name)?;
                w.char::<512>("Drum Samples\\808")?;
                if link {
                    w.u8(0)?;
                }
                Ok(())
            })?;
            if link {
                w.chunk(*b"LINk", |w| {
                    w.u32(5)?;
                    w.i32(-1)
                })?;
            }
            w.chunk(*b"PATH", |w| {
                w.content_path::<2048>(&ContentPath::Shorthand {
                    root: ContentRoot::App,
                    relative: format!("Drum Samples/808/{name}.wav"),
                })
            })?;
            w.chunk(*b"PRMS", |w| {
                (0..10).try_for_each(|i| w.f32(i as f32 / 10.0))
            })
        })
        .unwrap();
    }
    let legacy = w.into_bytes();
    bytes.extend(&legacy);
    let kit = read_drumset(&bytes).unwrap();
    assert_eq!(kit.file.pads, pads);
    let names: Vec<_> = kit.slots.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["Synth Kick", "Synth Snare", "Synth Clap", "Kick", "Snare"]
    );
    let kick = &kit.slots[3];
    assert_eq!(kick.folder, "Drum Samples/808");
    assert_eq!(
        kick.file.path,
        Some(ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "Drum Samples/808/Kick.wav".into(),
        })
    );
    assert_eq!(
        kick.params[..10],
        std::array::from_fn::<f32, 10, _>(|i| i as f32 / 10.0)
    );
    assert_eq!(kick.params[10], SamplerSlot::default().params[10]);
    // The ids: the file's where it has one, else a fresh one past them all.
    let ids: Vec<u32> = kit.slots.iter().map(|s| s.file.id).collect();
    assert_eq!(ids, [1, 2, 3, 6, 5]);
    assert_eq!(kit.file.last_slot_id, 6);
    // The legacy layout without PADS (`[PADS(22)]`): the identity map (default bytes 0..21).
    let bare = read_drumset(&[&b"LPMT"[..], &legacy].concat()).unwrap();
    let names: Vec<_> = bare.slots.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Kick", "Snare"]);
    assert_eq!(bare.file.pads, std::array::from_fn(|i| i as u8));
}

#[test]
fn tdiv_byte_0_is_the_subdivision_and_byte_1_the_beats_per_bar() {
    use honeydew::song::TimeSig;
    // Byte 0 = steps per beat, byte 1 = beats per bar; written and read back byte-exact.
    let song = Song {
        time_sig: TimeSig {
            subdivision: 3,
            beats_per_bar: 7,
        },
        ..Song::default()
    };
    let bytes = write(&song).unwrap();
    let at = bytes.windows(4).position(|w| w == b"TDIV").unwrap();
    assert_eq!(bytes[at + 4..at + 10], [2, 0, 0, 0, 3, 7]);
    let back = read(&bytes).unwrap();
    assert_eq!(back.time_sig, song.time_sig);
    assert_eq!(write(&back).unwrap(), bytes);
    // A new song writes `04 04`.
    let bytes = write(&Song::default()).unwrap();
    let at = bytes.windows(4).position(|w| w == b"TDIV").unwrap();
    assert_eq!(bytes[at + 8..at + 10], [4, 4]);
}

// ---- DirectWave `PATh` init values ----

fn cstm_chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    [&id[..], &(payload.len() as u32).to_le_bytes(), payload].concat()
}

/// A DW `PATh` (or `PATH`) chunk naming `path`, then `tail`.
fn dw_path_chunk(id: &[u8; 4], path: &[u8], tail: &[u8]) -> Vec<u8> {
    cstm_chunk(
        id,
        &[&(path.len() as u32).to_le_bytes()[..], path, tail].concat(),
    )
}

fn floats(a: f32, r: f32) -> Vec<u8> {
    [a.to_le_bytes(), r.to_le_bytes()].concat()
}

/// The `CSTM` a written DW Sampler preset holds.
fn written_cstm(module: &honeydew::song::ModuleInstance) -> Vec<u8> {
    let bytes = honeydew::flm::write_preset(module).unwrap();
    let cstm = honeydew::flm::chunk::ChunkIter::new(&bytes, "")
        .map(|c| c.unwrap())
        .find(|c| c.id == *b"CSTM")
        .unwrap();
    cstm.payload.to_vec()
}

#[test]
fn a_dw_program_saves_its_init_values_as_its_path_floats_not_its_current_ones() {
    use honeydew::flm::read_preset;
    let midi = cstm_chunk(b"MIDI", &[5, 0]);
    let path = b"@L@\0My Presets/DW Sampler/Keys.dwp";
    let mut module = honeydew::song::ModuleInstance::new(1, 2);
    module.params[1] = 0.3;
    module.params[2] = 0.4;
    let blob = |program: Vec<u8>| [&b"10WD"[..], &midi, &program].concat();
    let saved = |program: Vec<u8>| blob(dw_path_chunk(b"PATh", path, &program));

    // The init values are the params unless the module keeps others: the floats as stored are not saved.
    module.file.custom_data = Some(saved(floats(0.9, 0.9)));
    assert_eq!(written_cstm(&module), saved(floats(0.3, 0.4)));
    module.file.program_init = Some([0.2, 0.6]);
    let written = written_cstm(&module);
    assert_eq!(written, saved(floats(0.2, 0.6)), "init, not current");
    // PRMS keeps the current values, and the floats the module's init values on the way back.
    let bytes = honeydew::flm::write_preset(&module).unwrap();
    let back = read_preset(&bytes, None).unwrap();
    assert_eq!((back.params[1], back.params[2]), (0.3, 0.4));
    assert_eq!(back.file.program_init, Some([0.2, 0.6]));

    // Floats that are the params read back as no init values of their own.
    module.file.program_init = None;
    let back = read_preset(&honeydew::flm::write_preset(&module).unwrap(), None).unwrap();
    assert_eq!(back.file.program_init, None);
    let again = read_preset(&honeydew::flm::write_preset(&back).unwrap(), None).unwrap();
    assert_eq!(again, back, "a second save changes nothing");

    // Bytes after the floats stay. A PATh without both floats, or a legacy PATH, is saved as it is: until its
    // program loads, its init values are the program's, unknown here.
    let tail = [floats(0.1, 0.1), b"xy".to_vec()].concat();
    module.file.custom_data = Some(saved(tail));
    let tail = [floats(0.3, 0.4), b"xy".to_vec()].concat();
    assert_eq!(written_cstm(&module), saved(tail));
    for unknown in [
        saved(Vec::new()),
        saved(0.5f32.to_le_bytes().to_vec()),
        saved([f32::NAN.to_le_bytes(), 0.5f32.to_le_bytes()].concat()),
        blob(dw_path_chunk(b"PATH", path, &floats(0.1, 0.1))),
    ] {
        module.file.custom_data = Some(unknown.clone());
        assert_eq!(written_cstm(&module), unknown);
    }

    // The chunk that counts is the one rewritten; the rest stays as it was. ZONEs alone are saved as they are.
    let other = dw_path_chunk(b"PATh", b"@L@\0Other.dwp", &floats(0.7, 0.7));
    let two = [saved(floats(0.9, 0.9)), other.clone()].concat();
    module.file.custom_data = Some(two);
    assert_eq!(
        written_cstm(&module),
        [saved(floats(0.3, 0.4)), other].concat()
    );
    let mut zone = b"@L@\0z.wav".to_vec();
    zone.resize(1065, 0);
    let zones = blob(cstm_chunk(b"ZONE", &zone));
    module.file.custom_data = Some(zones.clone());
    assert_eq!(written_cstm(&module), zones);
}

/// A slot with the given stretch state, nothing else set.
fn stretched(id: u32, mode: u8, ratio: f64) -> SamplerSlot {
    SamplerSlot {
        file: SlotFileState {
            id,
            stretch_mode: mode,
            stretch_ratio: ratio,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The mode byte of every `STRC` and `STR1` chunk in `bytes`, in file order.
fn stretch_modes(bytes: &[u8]) -> Vec<(String, u8)> {
    let mut found = Vec::new();
    for (at, id) in bytes.windows(4).enumerate() {
        if id == b"STRC" || id == b"STR1" {
            found.push((String::from_utf8_lossy(id).into_owned(), bytes[at + 8]));
        }
    }
    found
}

#[test]
fn a_pads_stretch_mode_is_1_exactly_when_its_ratio_is_not_1() {
    // Pads write the mode as 1 exactly when the ratio is not 1, whatever
    // mode the slot holds; an audio clip keeps its SYNC button's.
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let slots = [(1, 1, 1.0), (2, 0, 1.5), (3, 0, 1.0), (4, 1, 0.5)];
    song.channels[1].drum_kit.as_mut().unwrap().slots = slots
        .map(|(id, mode, ratio)| stretched(id, mode, ratio))
        .into();
    let bytes = write(&song).unwrap();
    for (chunk, pair) in stretch_modes(&bytes).chunks(2).zip([0, 1, 0, 1]) {
        assert_eq!(chunk, [("STRC".into(), pair), ("STR1".into(), pair)]);
    }
    let loaded = read(&bytes).unwrap();
    let state = |song: &Song| -> Vec<(u8, f64)> {
        (song.channels[1].drum_kit.as_ref().unwrap().slots.iter())
            .map(|s| (s.file.stretch_mode, s.file.stretch_ratio))
            .collect()
    };
    assert_eq!(state(&loaded), [(0, 1.0), (1, 1.5), (0, 1.0), (1, 0.5)]);
    assert_eq!(read(&write(&loaded).unwrap()).unwrap(), loaded);

    // An audio clip is not a pad: SYNC on with a ratio of 1 stays on.
    let mut song = Song::default();
    song.add_channel(ChannelKind::AudioClip);
    song.channels[1].lanes[0].add_clip(0, 4.0).file.sample = Some(Box::new(stretched(1, 1, 1.0)));
    let loaded = read(&write(&song).unwrap()).unwrap();
    let slot = loaded.channels[1].lanes[0].clips[0].file.sample.as_deref();
    assert_eq!(slot.map(|s| s.file.stretch_mode), Some(1));
}

#[test]
fn a_dw_program_fits_only_the_dw_sampler() {
    // The DW Sampler's program picker lists `.dwp` files, which its own loader takes.
    let dwp = [&b"DwPr"[..], &38u32.to_le_bytes()].concat();
    assert!(honeydew::flm::preset_fits(&dwp, 1));
    assert!(!honeydew::flm::preset_fits(&dwp, 0));
    assert!(!honeydew::flm::preset_fits(b"junk", 1));
}

#[test]
fn a_pads_kept_stretch_bytes_are_judged_by_the_derived_mode_not_the_stored_one() {
    // The writer saves a pad's mode as `ratio != 1`, so the file's STRC / STR1 bytes go back only
    // while (that mode, ratio, semitones) is what the reader recorded. The slot's own `stretch_mode` plays no part.
    const STRC: [u8; 30] = [0xC1; 30];
    const STR1: [u8; 18] = [0xC2; 18];
    let written = |stored_mode: u8, ratio: f64, recorded_mode: u8| {
        let mut slot = stretched(1, stored_mode, ratio);
        slot.file.raw_strc = Raw(Some(STRC.to_vec()));
        slot.file.raw_str1 = Raw(Some(STR1.to_vec()));
        slot.file.raw_stretch_state = Raw(Some((recorded_mode, ratio, 0.0)));
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        song.channels[1].drum_kit.as_mut().unwrap().slots = vec![slot];
        let bytes = write(&song).unwrap();
        // The payload of the STRC and the STR1 chunk (an id, then a u32 length).
        ["STRC", "STR1"].map(|id| {
            let at = bytes.windows(4).position(|w| w == id.as_bytes()).unwrap();
            let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            bytes[at + 8..at + 8 + len].to_vec()
        })
    };
    // Ratio 1 derives mode 0, as recorded: kept, though the slot says 1.
    assert_eq!(written(1, 1.0, 0), [STRC.to_vec(), STR1.to_vec()]);
    // Ratio 2 derives mode 1, as recorded: kept.
    assert_eq!(written(1, 2.0, 1), [STRC.to_vec(), STR1.to_vec()]);
    // Ratio 2 derives mode 1 but the file held 0: rewritten with 1, though the slot says 0 as the file did.
    let [strc, str1] = written(0, 2.0, 0);
    assert_eq!((strc.len(), strc[0]), (25, 1));
    assert_eq!((str1.len(), str1[0]), (17, 1));
}

/// A slot's `MAIN` fields past its names at non-default values are written at their offsets (reverse @1036, beat
/// offset @1037 f64, its control @1045 f32, cut group @1049, overlap @1050) and read back.
#[test]
fn a_slots_main_flags_and_beat_offset_round_trip() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let slot = &mut song.channels[1].drum_kit.as_mut().unwrap().slots[0];
    slot.file.reverse = true;
    slot.file.beat_offset = 0.375;
    slot.file.beat_offset_control = 0.7;
    slot.cut_group = 3;
    slot.file.overlapping_voices = false;
    let bytes = write(&song).unwrap();
    let smpl = bytes.windows(4).position(|w| w == b"SMPl").unwrap();
    let main = smpl + bytes[smpl..].windows(4).position(|w| w == b"MAIN").unwrap() + 8;
    let at = |offset: usize, len: usize| &bytes[main + offset..main + offset + len];
    assert_eq!(at(1036, 1), [1]);
    assert_eq!(at(1037, 8), 0.375f64.to_le_bytes());
    assert_eq!(at(1045, 4), 0.7f32.to_le_bytes());
    assert_eq!(at(1049, 2), [3, 0]);
    let loaded = read(&bytes).unwrap();
    let read_back = &loaded.channels[1].drum_kit.as_ref().unwrap().slots[0];
    assert_eq!(
        (
            read_back.file.reverse,
            read_back.file.beat_offset,
            read_back.file.beat_offset_control,
            read_back.cut_group,
            read_back.file.overlapping_voices,
        ),
        (true, 0.375, 0.7, 3, false)
    );
}

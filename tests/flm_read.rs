use honeydew::flm::{
    chunk::{
        ChunkWriter, FlmErrorKind, MAGIC_CH02, MAGIC_CL02, MAGIC_RK01, MAGIC_SL02, MAGIC_SM01,
    },
    read, write,
};
use honeydew::song::{ChannelKind, Event, LaneKind, TimeSig};

fn minimal_file() -> Vec<u8> {
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"RACK", |writer| {
            writer.u32(0)?;
            writer.raw(&MAGIC_RK01)
        })
        .unwrap();
    writer
        .chunk(*b"CHNL", |writer| {
            writer.u32(0)?;
            writer.raw(&MAGIC_CH02)?;
            writer.chunk(*b"TRKH", |writer| {
                writer.chunk(*b"DESc", |writer| writer.raw(&[0; 1052]))
            })
        })
        .unwrap();
    writer.into_bytes()
}

#[test]
fn minimal_file_uses_loader_defaults() {
    let song = read(&minimal_file()).unwrap();
    assert_eq!(song.tempo_bpm, 120.0);
    assert_eq!(
        song.time_sig,
        TimeSig {
            subdivision: 4,
            beats_per_bar: 4
        }
    );
    assert_eq!(song.channels.len(), 1);
    assert_eq!(song.channels[0].kind, ChannelKind::Master);
    assert_eq!(song.channels[0].lanes[0].kind, LaneKind::Notes);
}

/// HEAD@264 is any f64. A tempo outside the app's range, [1, 999] BPM (tap tempo, MIDI), is clamped
/// into it, one that is no number (NaN, an infinity) too, as the 0 every such f64 reads as. None is a load error.
#[test]
fn a_hostile_tempo_is_clamped_or_the_default() {
    let tempo = |bpm: f64| {
        let mut writer = ChunkWriter::new_file();
        writer
            .chunk(*b"HEAD", |writer| {
                writer.raw(&[0; 264])?;
                writer.f64(bpm)?;
                writer.raw(&[0; 56])
            })
            .unwrap();
        let mut bytes = writer.into_bytes();
        bytes.extend_from_slice(&minimal_file()[4..]);
        read(&bytes).unwrap().tempo_bpm
    };
    for (file, loaded) in [
        (95.5, 95.5),
        (1.0, 1.0),
        (999.0, 999.0),
        (0.5, 1.0),
        (0.0, 1.0),
        (-0.0, 1.0),
        (-140.0, 1.0),
        (f64::MIN_POSITIVE, 1.0),
        (999.5, 999.0),
        (1e300, 999.0),
        (f64::NAN, 1.0),
        (f64::INFINITY, 1.0),
        (f64::NEG_INFINITY, 1.0),
    ] {
        assert_eq!(tempo(file), loaded, "{file}");
    }
}

#[test]
fn missing_channel_and_short_head_are_errors() {
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"HEAD", |writer| writer.raw(&[0; 327]))
        .unwrap();
    assert_eq!(
        read(&writer.into_bytes()).unwrap_err().kind,
        FlmErrorKind::TooShort
    );
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"RACK", |writer| {
            writer.u32(0)?;
            writer.raw(&MAGIC_RK01)
        })
        .unwrap();
    assert_eq!(
        read(&writer.into_bytes()).unwrap_err().kind,
        FlmErrorKind::MissingChunk
    );
}

#[test]
fn header_rack_channel_and_lane_fields_load() {
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"HEAD", |writer| {
            writer.i32(1)?;
            writer.i32(3)?;
            writer.char::<256>("Song")?;
            writer.f64(95.5)?;
            for value in [2.0, 8.0, 3.0, 1.0, 2.0, 0.5, 0.25] {
                writer.f64(value)?;
            }
            writer.raw(&[7; 16])?;
            writer.i32(5)?;
            writer.i32(2)?;
            writer.u8(0)?;
            writer.u8(1)?;
            writer.i32(4)?;
            writer.vstr32(b"4.10.19")
        })
        .unwrap();
    writer
        .chunk(*b"TDIV", |writer| writer.raw(&[3, 8]))
        .unwrap();
    writer
        .chunk(*b"RACK", |writer| {
            writer.u32(0)?;
            writer.raw(&MAGIC_RK01)?;
            writer.chunk(*b"RHED", |writer| {
                for value in [-1, 1, 0, -1] {
                    writer.i32(value)?;
                }
                Ok(())
            })
        })
        .unwrap();
    writer
        .chunk(*b"RACK", |writer| {
            writer.u32(0x100)?;
            writer.raw(&MAGIC_RK01)?;
            writer.chunk(*b"RHED", |writer| {
                for value in [-1, 7, 0, -1] {
                    writer.i32(value)?;
                }
                Ok(())
            })?;
            writer.chunk(*b"RPRM", |writer| {
                for value in [0.7, 0.4, 0.0, 0.0, 0.5, 0.0] {
                    writer.f32(value)?;
                }
                Ok(())
            })
        })
        .unwrap();
    for kind in [0, 0x100] {
        writer
            .chunk(*b"CHNL", |writer| {
                writer.u32(kind)?;
                writer.raw(&MAGIC_CH02)?;
                writer.chunk(*b"CHHD", |writer| {
                    writer.char::<1024>("Track")?;
                    writer.u32(1)?;
                    writer.f64(4.0)?;
                    writer.f32(0.8)?;
                    writer.f64(0.0)?;
                    writer.f64(0.0)?;
                    writer.u32(0)?;
                    writer.u32(1)?;
                    writer.raw(&[8; 16])?;
                    writer.i32(if kind == 0 { 0 } else { 3 })?;
                    writer.vstr32(b"Instrument 3")?;
                    writer.vstr32(b"Preset")?;
                    writer.vstr32(b"Custom")
                })?;
                writer.chunk(*b"TRKH", |writer| {
                    writer.chunk(*b"DESc", |writer| {
                        writer.u8(1)?;
                        writer.u8(1)?;
                        writer.u16(0)?;
                        writer.i32(2)?;
                        writer.i32(-3)?;
                        writer.f64(0.0)?;
                        writer.u32(0)?;
                        writer.u32(0)?;
                        writer.char::<1024>("Gain")?;
                        writer.vstr32(b"Custom gain")
                    })
                })
            })
            .unwrap();
    }
    let song = read(&writer.into_bytes()).unwrap();
    assert_eq!(song.title, "Song");
    assert_eq!(song.tempo_bpm, 95.5);
    // TDIV `03 08`: byte 0 the subdivision, byte 1 the beats per bar.
    assert_eq!(
        song.time_sig,
        TimeSig {
            subdivision: 3,
            beats_per_bar: 8
        }
    );
    assert_eq!(song.file.selected_channel, 1);
    assert_eq!(song.play_loop, Some((256, 1024)));
    assert_eq!(song.key_root, 5);
    assert_eq!(song.channels[1].rack.last_instance_id, 7);
    assert_eq!(song.channels[1].params.level(), 0.7);
    assert_eq!(song.channels[1].id, 3);
    assert_eq!(song.channels[1].name, "Custom");
    assert_eq!(song.channels[1].lanes[0].target.unwrap().param, -1);
    assert_eq!(song.channels[1].lanes[0].file.custom_label, "Custom gain");
}

fn event_file(kind: u8, key: i16, value: u16) -> Vec<u8> {
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"RACK", |writer| {
            writer.u32(0)?;
            writer.raw(&MAGIC_RK01)
        })
        .unwrap();
    writer
        .chunk(*b"CHNL", |writer| {
            writer.u32(0)?;
            writer.raw(&MAGIC_CH02)?;
            writer.chunk(*b"TRKH", |writer| {
                let mut desc = [0; 1052];
                desc[0] = kind;
                writer.chunk(*b"DESc", |writer| writer.raw(&desc))?;
                writer.chunk(*b"CLIP", |writer| {
                    writer.u32(128)?;
                    writer.raw(&MAGIC_CL02)?;
                    writer.chunk(*b"CLHd", |writer| {
                        writer.f64(4.0)?;
                        writer.f64(4.0)?;
                        writer.f64(0.0)?;
                        writer.u32(1)?;
                        writer.f64(0.0)?;
                        writer.u8(0)
                    })?;
                    writer.chunk(*b"EVN2", |writer| {
                        writer.u16(20)?;
                        writer.u32(64)?;
                        writer.f64(0.5)?;
                        writer.i16(key)?;
                        writer.u8(204)?;
                        writer.u8(127)?;
                        writer.u16(value)?;
                        writer.u8(0)?;
                        writer.u8(0)
                    })
                })
            })
        })
        .unwrap();
    writer.into_bytes()
}

#[test]
fn clips_decode_note_automation_and_envelope_records() {
    let note = read(&event_file(0, 60, 0x7fff)).unwrap();
    let clip = &note.channels[0].lanes[0].clips[0];
    assert_eq!(clip.start_tick, 128);
    assert_eq!(
        clip.events,
        [Event::Note {
            pos: 64,
            len_beats: 0.5,
            key: 60,
            velocity: 204,
            pan: 127,
            slide: false,
        }]
    );

    let automation = read(&event_file(1, 1, 32768)).unwrap();
    assert_eq!(
        automation.channels[0].lanes[0].clips[0].events,
        [Event::Automation {
            pos: 64,
            value: 32768.0 / 65535.0,
            control: true
        }]
    );

    let envelope = read(&event_file(2, -3, 65535)).unwrap();
    assert_eq!(
        envelope.channels[0].lanes[0].clips[0].events,
        [Event::Envelope {
            pos: 64,
            env_param: 2,
            value: 1.0,
            control: false
        }]
    );
}

#[test]
fn metadata_keyboard_and_module_fields_load() {
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"META", |writer| {
            writer.char::<256>("2026-10-11 09-00-00")?;
            writer.f64(123.5)?;
            writer.char::<256>("Alex")?;
            writer.char::<256>("Displayed")?;
            writer.u8(1)?;
            writer.char::<2048>("https://example.test")?;
            writer.char::<256>("2026-10-11 09:10:00")
        })
        .unwrap();
    writer
        .chunk(*b"KEYB", |writer| {
            writer.vstr32(b"{\"chords\":[]}")?;
            writer.u32(1)?;
            writer.u32(7)?;
            writer.vstr32(b"Minor")?;
            writer.i32(-1)?;
            writer.f32(0.75)?;
            writer.f32(-0.1)?;
            writer.u8(1)
        })
        .unwrap();
    writer
        .chunk(*b"RACK", |writer| {
            writer.u32(0)?;
            writer.raw(&MAGIC_RK01)?;
            writer.chunk(*b"RMOd", |writer| {
                writer.u32(9)?;
                writer.u32(2)?;
                writer.chunk(*b"HEAD", |writer| {
                    writer.u32(9)?;
                    writer.u32(0)?;
                    writer.u8(1)
                })?;
                writer.chunk(*b"DESC", |writer| {
                    writer.char::<256>("Effects")?;
                    writer.char::<256>("Warm")?;
                    writer.u32(0)
                })?;
                writer.chunk(*b"PRST", |writer| {
                    writer.vstr32(b"preset.flmpst")?;
                    writer.vstr32(b"Effects/Warm")
                })?;
                writer.chunk(*b"ADD1", |writer| writer.raw(&[1, 12, 100]))?;
                writer.chunk(*b"PRMS", |writer| {
                    for value in [1.0, 0.2, 0.3, 0.4, 0.5] {
                        writer.f32(value)?;
                    }
                    Ok(())
                })?;
                writer.chunk(*b"SMPR", |writer| {
                    for value in [0.6, 0.7, 0.8] {
                        writer.f32(value)?;
                    }
                    Ok(())
                })
            })
        })
        .unwrap();
    writer
        .chunk(*b"CHNL", |writer| {
            writer.u32(0)?;
            writer.raw(&MAGIC_CH02)?;
            writer.chunk(*b"TRKH", |writer| {
                writer.chunk(*b"DESc", |writer| writer.raw(&[0; 1052]))
            })
        })
        .unwrap();
    let song = read(&writer.into_bytes()).unwrap();
    assert_eq!(song.file.meta.author, "Alex");
    assert_eq!(song.file.meta.editing_seconds, 123.5);
    assert_eq!(song.file.keyboard.chord_set_name, "Minor");
    assert_eq!(song.file.keyboard.octave_shift, -1);
    let module = &song.channels[0].rack.modules[0];
    assert_eq!((module.type_id, module.instance_id), (9, 2));
    assert_eq!(module.params[1], 0.2);
    assert_eq!(module.common.0, [0.6, 0.7, 0.8]);
    assert!(module.minimized);
    assert_eq!(module.file.category, "Effects");
    assert_eq!(module.file.preset_name, "Warm");
    assert_eq!(module.file.key_low, 12);
}

#[test]
fn drum_sampler_slot_loads_its_path_and_settings() {
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"RACK", |writer| {
            writer.u32(0)?;
            writer.raw(&MAGIC_RK01)
        })
        .unwrap();
    writer
        .chunk(*b"RACK", |writer| {
            writer.u32(0x100)?;
            writer.raw(&MAGIC_RK01)?;
            writer.chunk(*b"RSMP", |writer| {
                writer.raw(&MAGIC_SM01)?;
                writer.u32(1)?;
                writer.chunk(*b"PADS", |writer| {
                    writer.raw(&(0u8..22).collect::<Vec<_>>())
                })?;
                writer.chunk(*b"SMPl", |writer| {
                    writer.raw(&MAGIC_SL02)?;
                    writer.chunk(*b"MAIN", |writer| {
                        writer.i32(-1)?;
                        writer.f64(0.0)?;
                        writer.char::<512>("Kick")?;
                        writer.char::<512>("Drum Samples/Kick")?;
                        writer.u8(1)?;
                        writer.f64(0.125)?;
                        writer.f32(0.6)?;
                        writer.u8(2)?;
                        writer.u8(0)?;
                        writer.vstr16(b"My Kick")
                    })?;
                    writer.chunk(*b"LINk", |writer| {
                        writer.u32(1)?;
                        writer.i32(-1)?;
                        writer.u16(36)?;
                        writer.i32(0)
                    })?;
                    writer.chunk(*b"STRC", |writer| writer.raw(&[0; 25]))?;
                    writer.chunk(*b"STR1", |writer| {
                        writer.u8(1)?;
                        writer.f64(1.25)?;
                        writer.f64(3.0)
                    })?;
                    writer.chunk(*b"PTH1", |writer| {
                        writer.vstr16(b"@R@\0Drum Samples/Kick.wav")
                    })?;
                    writer.chunk(*b"PRST", |writer| {
                        writer.vstr32(b"source")?;
                        writer.vstr32(b"Drums/Kick")
                    })?;
                    writer.chunk(*b"PRMS", |writer| {
                        for _ in 0..11 {
                            writer.f32(0.5)?;
                        }
                        Ok(())
                    })
                })
            })
        })
        .unwrap();
    for kind in [0, 0x100] {
        writer
            .chunk(*b"CHNL", |writer| {
                writer.u32(kind)?;
                writer.raw(&MAGIC_CH02)?;
                writer.chunk(*b"TRKH", |writer| {
                    let mut desc = [0; 1052];
                    desc[0] = if kind == 0 { 1 } else { 3 };
                    writer.chunk(*b"DESc", |writer| writer.raw(&desc))
                })
            })
            .unwrap();
    }
    let song = read(&writer.into_bytes()).unwrap();
    assert_eq!(song.channels[1].kind, ChannelKind::Drums);
    let kit = song.channels[1].drum_kit.as_ref().unwrap();
    assert_eq!(kit.file.last_slot_id, 1);
    let slot = &kit.slots[0];
    assert_eq!(slot.name, "Kick");
    assert_eq!(slot.cut_group, 2);
    assert_eq!(slot.file.id, 1);
    assert_eq!(slot.file.custom_pad_name, "My Kick");
    assert_eq!(slot.file.stretch_ratio, 1.25);
    assert_eq!(
        slot.file.path.as_ref().unwrap(),
        &honeydew::flm::chunk::ContentPath::Shorthand {
            root: honeydew::flm::chunk::ContentRoot::App,
            relative: "Drum Samples/Kick.wav".into(),
        }
    );
    let saved = read(&write(&song).unwrap()).unwrap();
    let saved_slot = &saved.channels[1].drum_kit.as_ref().unwrap().slots[0];
    assert_eq!(saved_slot.file.raw_strc, slot.file.raw_strc);
    assert_eq!(saved_slot.file.raw_str1, slot.file.raw_str1);
}

#[test]
fn audio_clip_sample_gets_a_new_slot_id() {
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"RACK", |writer| {
            writer.u32(0)?;
            writer.raw(&MAGIC_RK01)
        })
        .unwrap();
    writer
        .chunk(*b"RACK", |writer| {
            writer.u32(0x100)?;
            writer.raw(&MAGIC_RK01)
        })
        .unwrap();
    for kind in [0, 0x100] {
        writer
            .chunk(*b"CHNL", |writer| {
                writer.u32(kind)?;
                writer.raw(&MAGIC_CH02)?;
                writer.chunk(*b"TRKH", |writer| {
                    let mut desc = [0; 1052];
                    desc[0] = if kind == 0 { 1 } else { 2 };
                    writer.chunk(*b"DESc", |writer| writer.raw(&desc))?;
                    if kind != 0 {
                        writer.chunk(*b"CLIP", |writer| {
                            writer.u32(0)?;
                            writer.raw(&MAGIC_CL02)?;
                            writer.chunk(*b"CLSm", |writer| {
                                writer.raw(&MAGIC_SL02)?;
                                writer.chunk(*b"MAIN", |writer| {
                                    writer.i32(-1)?;
                                    writer.f64(-1.0)?;
                                    writer.char::<512>("Voice")?;
                                    writer.char::<512>("Recordings")
                                })?;
                                writer.chunk(*b"LINk", |writer| writer.u32(99))
                            })
                        })?;
                    }
                    Ok(())
                })
            })
            .unwrap();
    }
    let song = read(&writer.into_bytes()).unwrap();
    assert_eq!(song.channels[1].kind, ChannelKind::AudioClip);
    assert_eq!(
        song.channels[1].lanes[0].clips[0]
            .file
            .sample
            .as_ref()
            .unwrap()
            .file
            .id,
        1
    );
}

// --- Reader and writer fixes ---

use honeydew::flm::{chunk::ChunkWriter as W, read_with_notices};
use honeydew::song::{AutomationTarget, DrumKit, Song, SynthKind};

type Build<'a> = &'a dyn Fn(&mut W) -> Result<(), honeydew::flm::FlmError>;

fn chunk(w: &mut W, id: &[u8; 4], body: Build) {
    w.chunk(*id, body).unwrap();
}

/// A file of `body`'s top-level chunks.
fn file(body: Build) -> Vec<u8> {
    let mut w = W::new_file();
    body(&mut w).unwrap();
    w.into_bytes()
}

fn head(w: &mut W, next_channel_id: Option<u32>, last_bus: i32, f: [f64; 3]) {
    chunk(w, b"HEAD", &|w| {
        w.i32(0)?;
        w.i32(last_bus)?;
        w.char::<256>("Song")?;
        w.f64(f[0])?;
        w.f64(f[1])?;
        w.f64(f[2])?;
        for value in [0.0, 0.0, 0.0, 0.01, 0.02] {
            w.f64(value)?;
        }
        w.raw(&[0; 16])?;
        w.i32(0)?;
        w.i32(0)?;
        w.u8(1)?;
        w.u8(1)?;
        if let Some(next) = next_channel_id {
            w.u32(next)?;
            w.vstr32(b"4.10.19")?;
        }
        Ok(())
    });
}

/// A RACK of `kind` with RHED [−1, 1, bus, output] and `body`.
fn rack(w: &mut W, kind: u32, bus: i32, output: i32, body: Build) {
    chunk(w, b"RACK", &|w| {
        w.u32(kind)?;
        w.raw(&MAGIC_RK01)?;
        w.chunk(*b"RHED", |w| {
            for value in [-1, 1, bus, output] {
                w.i32(value)?;
            }
            Ok(())
        })?;
        w.chunk(*b"RPRM", |w| {
            for value in [0.8531, 0.5, 0.0, 0.0, 0.5, 0.0] {
                w.f32(value)?;
            }
            Ok(())
        })?;
        body(w)
    });
}

/// A CHHD (v2 + extension).
fn chhd(w: &mut W, row: f64, id: i32, names: [&str; 3]) -> Result<(), honeydew::flm::FlmError> {
    w.chunk(*b"CHHD", |w| {
        w.char::<1024>(names.iter().find(|n| !n.is_empty()).unwrap_or(&""))?;
        w.u32(0)?;
        w.f64(row)?;
        w.f32(0.5)?;
        w.f64(0.0)?;
        w.f64(0.0)?;
        w.u32(0)?;
        w.u32(0)?;
        w.raw(&[0; 16])?;
        w.i32(id)?;
        for name in names {
            w.vstr32(name.as_bytes())?;
        }
        Ok(())
    })
}

fn send(w: &mut W, target: i32) -> Result<(), honeydew::flm::FlmError> {
    w.chunk(*b"SEND", |w| {
        w.i32(target)?;
        for value in [1.0, 1.0, 1.0, 0.8531, 0.8531] {
            w.f32(value)?;
        }
        Ok(())
    })
}

/// A TRKH whose DESc has `kind`, `inst`, `param`, then `body` (clips).
fn lane(
    w: &mut W,
    kind: u8,
    inst: i32,
    param: i32,
    body: Build,
) -> Result<(), honeydew::flm::FlmError> {
    w.chunk(*b"TRKH", |w| {
        w.chunk(*b"DESc", |w| {
            w.u8(kind)?;
            w.u8(0)?;
            w.u16(0)?;
            w.i32(inst)?;
            w.i32(param)?;
            w.f64(0.0)?;
            w.u32(0)?;
            w.u32(0)?;
            w.char::<1024>("")?;
            w.vstr32(b"")
        })?;
        body(w)
    })
}

/// A CHNL with a CHHD (`row`, `id`, base name), sends to `sends` and one lane of `lane_kind`.
fn chnl(w: &mut W, row: f64, id: i32, base: &str, sends: &[i32], lane_kind: u8) {
    chunk(w, b"CHNL", &|w| {
        w.u32(0)?;
        w.raw(&MAGIC_CH02)?;
        chhd(w, row, id, [base, "", ""])?;
        for &target in sends {
            send(w, target)?;
        }
        let (inst, param) = if lane_kind == 1 { (0, 0) } else { (-1, -1) };
        lane(w, lane_kind, inst, param, &|_| Ok(()))
    });
}

/// A CLIP at tick 0 with a 4-beat loop, visible `visible` beats, and EVN2 `records` (pos, key, value, @18, @19).
fn clip(
    w: &mut W,
    visible: f64,
    records: &[(u32, i16, u16, u8, u8)],
) -> Result<(), honeydew::flm::FlmError> {
    w.chunk(*b"CLIP", |w| {
        w.u32(0)?;
        w.raw(&MAGIC_CL02)?;
        w.chunk(*b"CLHd", |w| {
            w.f64(visible)?;
            w.f64(4.0)?;
            w.f64(0.0)?;
            w.u32(1)?;
            w.f64(0.0)?;
            w.u8(0)
        })?;
        w.chunk(*b"EVN2", |w| {
            w.u16(20)?;
            for &(pos, key, value, repeat, slide) in records {
                w.u32(pos)?;
                w.f64(0.25)?;
                w.i16(key)?;
                w.u8(0xb2)?;
                w.u8(0x7f)?;
                w.u16(value)?;
                w.u8(repeat)?;
                w.u8(slide)?;
            }
            Ok(())
        })
    })
}

/// Master, one instrument (id 1) with a module (instance 2) and `lanes` after its notes lane.
fn instrument_file(lanes: Build) -> Vec<u8> {
    file(&|w| {
        head(w, Some(2), 0, [120.0, 0.0, 0.0]);
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, &|w| {
            w.chunk(*b"RMOd", |w| {
                w.u32(9)?;
                w.u32(2)?;
                w.chunk(*b"HEAD", |w| {
                    w.u32(9)?;
                    w.u32(0)?;
                    w.u8(1)
                })?;
                w.chunk(*b"DESC", |w| w.raw(&[0; 516]))
            })
        });
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chunk(w, b"CHNL", &|w| {
            w.u32(0x100)?;
            w.raw(&MAGIC_CH02)?;
            chhd(w, 1.0, 1, ["Instrument 1", "", ""])?;
            send(w, 0)?;
            lane(w, 0, -1, -1, &|_| Ok(()))?;
            lanes(w)
        });
        Ok(())
    })
}

fn control_flags(song: &Song, lane: usize) -> Vec<bool> {
    let events = &song.channels[1].lanes[lane].clips[0].events;
    events
        .iter()
        .map(|e| match *e {
            Event::Automation { control, .. } => control,
            _ => panic!("{e:?}"),
        })
        .collect()
}

#[test]
fn common_param_lanes_read_the_apps_negative_control_keys() {
    // Mix (file −2): param + 1 = −1, so −1 is a control point and +1 a generated step (sample.flm's pattern).
    let song = read(&instrument_file(&|w| {
        lane(w, 1, 2, -2, &|w| {
            clip(
                w,
                4.0,
                &[(0, -1, 0, 0, 0), (8, 1, 9, 0, 0), (16, -1, 99, 0, 0)],
            )
        })
    }))
    .unwrap();
    assert_eq!(
        song.channels[1].lanes[1].target.unwrap().param,
        AutomationTarget::MIX
    );
    assert_eq!(control_flags(&song, 1), [true, false, true]);
    // The 0x7FC0 alias is positive: +0x7FC1 is a control point.
    let song = read(&instrument_file(&|w| {
        lane(w, 1, 2, 0x7fc0, &|w| {
            clip(w, 4.0, &[(0, 0x7fc1, 0, 0, 0), (8, -0x7fc1, 9, 0, 0)])
        })
    }))
    .unwrap();
    assert_eq!(
        song.channels[1].lanes[1].target.unwrap().param,
        AutomationTarget::POST_GAIN
    );
    assert_eq!(control_flags(&song, 1), [true, false]);
}

#[test]
fn a_clip_keeps_its_visible_length_of_zero() {
    let song = read(&instrument_file(&|w| {
        lane(w, 0, -1, -1, &|w| clip(w, 0.0, &[]))
    }))
    .unwrap();
    let clip = &song.channels[1].lanes[1].clips[0];
    assert_eq!(clip.visible_len_beats, 0.0);
    assert_eq!(clip.visible_ticks(), 512.0);
}

#[test]
fn notes_and_hits_keep_slide_repeat_and_pitch() {
    let song = read(&instrument_file(&|w| {
        lane(w, 0, -1, -1, &|w| clip(w, 4.0, &[(0, 60, 0x7fff, 0, 1)]))?;
        lane(w, 3, -1, -1, &|w| clip(w, 4.0, &[(0, 2, 0x1234, 8, 0)]))
    }))
    .unwrap();
    let lanes = &song.channels[1].lanes;
    assert!(matches!(
        lanes[1].clips[0].events[0],
        Event::Note { slide: true, .. }
    ));
    assert!(matches!(
        lanes[2].clips[0].events[0],
        Event::Drum {
            value: 0x1234,
            repeat: 8,
            ..
        }
    ));
}

/// Master; a send (bus `bus`, id 1); an instrument (id 2) with `sends` and RHED[12] `output`.
fn routed_file(bus: i32, sends: &[i32], output: i32, legacy: bool) -> Vec<u8> {
    file(&|w| {
        head(w, (!legacy).then_some(3), 0, [120.0, 0.0, 0.0]);
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x80, bus, -1, &|_| Ok(()));
        rack(w, 0x100, if legacy { -1 } else { 0 }, output, &|_| Ok(()));
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chnl(w, 1.0, 1, "Send 1", &[0], 1);
        chnl(w, 2.0, 2, "Instrument 2", sends, 0);
        Ok(())
    })
}

#[test]
fn the_output_bus_comes_from_the_first_send_to_a_send_channel() {
    // RHED[12] says master; the SEND says the send channel: the SEND wins.
    let song = read(&routed_file(1, &[0, 1], -1, false)).unwrap();
    assert_eq!(song.channels[2].output_bus, 1);
    assert_eq!(song.channels[1].output_bus, -1);
    // The writer derives RHED[12] from the sends too.
    let mut song = song;
    song.channels[2].output_bus = -1;
    let again = read(&write(&song).unwrap()).unwrap();
    assert_eq!(again.channels[2].output_bus, 1);
}

#[test]
fn legacy_routing_sends_to_the_send_channel_with_that_bus_or_the_master() {
    let song = read(&routed_file(1, &[], 1, true)).unwrap();
    assert_eq!(song.channels[2].file.sends.len(), 1);
    assert_eq!(song.channels[2].file.sends[0].target_channel_id, 1);
    assert_eq!(song.channels[2].output_bus, 1);
    // An output to the master (−1) never matches a channel whose bus is −1.
    let song = read(&routed_file(1, &[], -1, true)).unwrap();
    assert_eq!(song.channels[2].file.sends[0].target_channel_id, 0);
    assert_eq!(song.file.next_channel_id, 3);
}

#[test]
fn counters_are_raised_above_what_the_file_holds() {
    // HEAD@4 is 0 but a send owns bus 3; @354 is absent (legacy).
    let song = read(&routed_file(3, &[], -1, true)).unwrap();
    assert_eq!(song.file.last_send_bus, 3);
    assert_eq!(song.file.next_channel_id, 3);
    let mut song = song;
    song.add_channel(ChannelKind::EffectTrack);
    assert_eq!(song.channels[3].bus_id, 4);
    assert_eq!(song.channels[3].id, 3);
    assert!(write(&song).is_ok());
}

#[test]
fn a_channel_without_a_send_gets_one_to_the_master() {
    let song = read(&routed_file(1, &[], -1, false)).unwrap();
    assert_eq!(song.channels[2].file.sends.len(), 1);
    assert_eq!(song.channels[2].file.sends[0].target_channel_id, 0);
    assert!(song.master().file.sends.is_empty());
}

#[test]
fn a_channel_without_a_rack_is_skipped_with_a_notice() {
    let bytes = file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chnl(w, 1.0, 1, "Instrument 1", &[0], 0);
        Ok(())
    });
    let (song, notices) = read_with_notices(&bytes).unwrap();
    assert_eq!(song.channels.len(), 1);
    assert_eq!(notices, ["Missing rack for Channel #2 skipped"]);
}

#[test]
fn channel_ids_are_repaired_and_the_master_is_0() {
    // Ids 0, 7, 7, 5000 and −1: the duplicate, the out-of-range and the missing id get fresh ones.
    let bytes = file(&|w| {
        head(w, Some(8), 0, [120.0, 0.0, 0.0]);
        for _ in 0..5 {
            rack(w, 0x100, 0, -1, &|_| Ok(()));
        }
        rack(w, 0, 0, -1, &|_| Ok(()));
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        for (row, id) in [7, 7, 4095, 5000, -1].into_iter().enumerate() {
            chnl(w, row as f64 + 1.0, id, "Instrument", &[0], 0);
        }
        Ok(())
    });
    let song = read(&bytes).unwrap();
    let ids: Vec<i32> = song.channels.iter().map(|c| c.id).collect();
    assert_eq!(ids, [0, 7, 8, 4095, 9, 10]);
    assert_eq!(song.file.next_channel_id, 4095);
    let again = read(&write(&song).unwrap()).unwrap();
    assert_eq!(again, song);
}

#[test]
fn the_first_rack_is_the_master_and_extra_masters_become_instruments() {
    let bytes = file(&|w| {
        rack(w, 0x100, 0, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, &|_| Ok(()));
        chnl(w, 0.0, 0, "", &[], 1);
        chnl(w, 1.0, 1, "", &[0], 0);
        Ok(())
    });
    let song = read(&bytes).unwrap();
    assert_eq!(song.master().kind, ChannelKind::Master);
    assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
    let bytes = file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x80, 1, -1, &|_| Ok(()));
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chnl(w, 1.0, 1, "", &[0], 0);
        chnl(w, 2.0, 2, "", &[0], 1);
        Ok(())
    });
    let song = read(&bytes).unwrap();
    let kinds: Vec<_> = song.channels.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            ChannelKind::Master,
            ChannelKind::Instrument,
            ChannelKind::EffectTrack
        ]
    );
    assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
}

#[test]
fn channels_take_their_playlist_order_after_renumbering_duplicate_rows() {
    // Rack order: master, send, A, B. Rows: 0, 3, 1, 1: A's row is a duplicate, so every other row ≥ 1 moves up
    // one (send 4, B 2), giving master, A, B, send: the playlist order, not the rack order.
    let bytes = file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x80, 1, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, &|_| Ok(()));
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chnl(w, 3.0, 1, "S", &[0], 1);
        chnl(w, 1.0, 2, "A", &[0], 0);
        chnl(w, 1.0, 3, "B", &[0], 0);
        Ok(())
    });
    let song = read(&bytes).unwrap();
    let names: Vec<&str> = song.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["MASTER", "A", "B", "S"]);
    assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
}

#[test]
fn drum_slots_sit_on_their_pad_and_get_unique_ids() {
    let slot = |w: &mut W, pad: f64, id: u32| {
        w.chunk(*b"SMPl", |w| {
            w.raw(&MAGIC_SL02)?;
            w.chunk(*b"MAIN", |w| {
                w.i32(-1)?;
                w.f64(pad)?;
                w.char::<512>(&format!("Pad {pad}"))?;
                w.char::<512>("Drum Samples\\Kick")
            })?;
            w.chunk(*b"LINk", |w| w.u32(id))
        })
    };
    let bytes = file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, &|w| {
            w.chunk(*b"RSMP", |w| {
                w.raw(&MAGIC_SM01)?;
                w.u32(2)?;
                w.chunk(*b"PADS", |w| w.raw(&[0; 22]))?;
                slot(w, 2.0, 4)?;
                slot(w, 0.0, 4)?;
                slot(w, 1.0, 0)
            })
        });
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chnl(w, 1.0, 1, "Drums 1", &[0], 3);
        Ok(())
    });
    let song = read(&bytes).unwrap();
    let kit = song.channels[1].drum_kit.as_ref().unwrap();
    let names: Vec<&str> = kit.slots.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Pad 0", "Pad 1", "Pad 2"]);
    let ids: Vec<u32> = kit.slots.iter().map(|s| s.file.id).collect();
    assert_eq!(ids, [5, 6, 4]);
    assert_eq!(kit.file.last_slot_id, 6);
    // The folder reads with `/`.
    assert_eq!(kit.slots[0].folder, "Drum Samples/Kick");
    assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
}

#[test]
fn a_drums_channel_without_rsmp_gets_an_empty_kit() {
    let bytes = file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, &|_| Ok(()));
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chnl(w, 1.0, 1, "Drums 1", &[0], 3);
        Ok(())
    });
    let song = read(&bytes).unwrap();
    assert_eq!(song.channels[1].kind, ChannelKind::Drums);
    assert_eq!(song.channels[1].drum_kit, Some(DrumKit::default()));
    assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
}

#[test]
fn the_play_loop_reads_in_ticks_with_a_reversed_pair_swapped() {
    let song = |start: f64, end: f64| {
        read(&file(&|w| {
            head(w, Some(1), 0, [120.0, start, end]);
            rack(w, 0, 0, -1, &|_| Ok(()));
            chnl(w, 0.0, 0, "MASTER", &[], 1);
            Ok(())
        }))
        .unwrap()
    };
    assert_eq!(song(2.0, 8.0).play_loop, Some((256, 1024)));
    assert_eq!(song(8.0, 2.0).play_loop, Some((256, 1024)));
    assert_eq!(song(4.0, 4.0).play_loop, None);
    assert_eq!(song(-3.0, 0.0).play_loop, None);
}

#[test]
fn nan_reads_as_zero_so_a_read_song_equals_itself() {
    let song = read(&file(&|w| {
        head(w, Some(1), 0, [f64::NAN, 0.0, 0.0]);
        rack(w, 0, 0, -1, &|_| Ok(()));
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        Ok(())
    }))
    .unwrap();
    // Read as 0, which the tempo's range then clamps.
    assert_eq!(song.tempo_bpm, 1.0);
    assert_eq!(song, song.clone());
    assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
}

#[test]
fn unknown_chunks_with_unwritable_ids_are_dropped() {
    let song = read(&file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        Ok(())
    }))
    .unwrap();
    let mut bytes = file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        w.chunk(*b"ZZZZ", |w| w.u8(1))?;
        w.chunk(*b"YYYY", |w| w.u8(2))
    });
    let at = bytes.windows(4).position(|x| x == b"ZZZZ").unwrap();
    bytes[at..at + 4].copy_from_slice(&[0, 1, 2, 3]);
    let read_back = read(&bytes).unwrap();
    assert_eq!(read_back.file.unknown_chunks.len(), 1);
    assert_eq!(&read_back.file.unknown_chunks[0].id, b"YYYY");
    assert_eq!(read_back.channels, song.channels);
    assert!(write(&read_back).is_ok());
}

#[test]
fn module_ids_below_2_or_repeated_get_fresh_ones() {
    let module = |w: &mut W, id: u32| {
        w.chunk(*b"RMOd", |w| {
            w.u32(9)?;
            w.u32(id)?;
            w.chunk(*b"HEAD", |w| {
                w.u32(9)?;
                w.u32(0)?;
                w.u8(1)
            })?;
            w.chunk(*b"DESC", |w| w.raw(&[0; 516]))?;
            w.chunk(*b"ADD1", |w| w.raw(&[0, 100, 20]))
        })
    };
    let song = read(&file(&|w| {
        rack(w, 0, 0, -1, &|w| {
            module(w, 1)?;
            module(w, 3)?;
            module(w, 3)?;
            module(w, 0x8000_0000)?;
            module(w, 7)
        });
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        Ok(())
    }))
    .unwrap();
    let rack = &song.master().rack;
    let ids: Vec<i32> = rack.modules.iter().map(|m| m.instance_id).collect();
    assert_eq!(ids, [8, 3, 9, 10, 7]);
    assert_eq!(rack.last_instance_id, 10);
    // ADD1: an inverted key range is swapped.
    assert_eq!(
        (rack.modules[0].file.key_low, rack.modules[0].file.key_high),
        (20, 100)
    );
    assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
}

#[test]
fn audio_record_and_rack_channels_keep_their_kind() {
    let mut song = Song::default();
    for kind in [
        ChannelKind::AudioRecord,
        ChannelKind::Rack,
        ChannelKind::AudioClip,
        ChannelKind::Instrument,
    ] {
        song.add_channel(kind);
    }
    song.channels[4].rack.add(0);
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let read_back = read(&write(&song).unwrap()).unwrap();
    let kinds: Vec<_> = read_back.channels.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        song.channels.iter().map(|c| c.kind).collect::<Vec<_>>()
    );
}

#[test]
fn a_key_range_is_swapped_clamped_and_then_stable() {
    let song_with = |add1: [u8; 3]| {
        read(&file(&|w| {
            rack(w, 0, 0, -1, &|w| {
                w.chunk(*b"RMOd", |w| {
                    w.u32(9)?;
                    w.u32(2)?;
                    w.chunk(*b"HEAD", |w| {
                        w.u32(9)?;
                        w.u32(0)?;
                        w.u8(1)
                    })?;
                    w.chunk(*b"DESC", |w| w.raw(&[0; 516]))?;
                    w.chunk(*b"ADD1", |w| w.raw(&add1))
                })
            });
            chnl(w, 0.0, 0, "MASTER", &[], 1);
            Ok(())
        }))
        .unwrap()
    };
    for (add1, range) in [
        ([0, 100, 20], (20, 100)),
        ([0, 0xfb, 30], (0, 30)),
        ([0, 0xfb, 0xff], (0, 0)),
    ] {
        let song = song_with(add1);
        let file = &song.master().rack.modules[0].file;
        assert_eq!((file.key_low, file.key_high), range, "{add1:?}");
        assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
    }
}

#[test]
fn a_zero_key_is_a_generated_point_where_param_plus_one_is_positive() {
    let song = read(&instrument_file(&|w| {
        lane(w, 1, 2, 5, &|w| {
            clip(w, 4.0, &[(0, 0, 0, 0, 0), (8, 6, 0, 0, 0)])
        })?;
        lane(w, 2, -1, 0, &|w| {
            clip(w, 4.0, &[(0, 0, 0, 0, 0), (8, 1, 0, 0, 0)])
        })
    }))
    .unwrap();
    let lanes = &song.channels[1].lanes;
    // Lanes: notes, audio (main lanes first), then the automation lane.
    assert_eq!(control_flags(&song, 2), [false, true]);
    let envelope: Vec<bool> = (lanes[1].clips[0].events.iter())
        .map(|e| matches!(e, Event::Envelope { control: true, .. }))
        .collect();
    assert_eq!(envelope, [false, true]);
}

#[test]
fn counters_cover_the_clips_and_slots_read() {
    let song = read(&instrument_file(&|w| {
        lane(w, 0, -1, -1, &|w| clip(w, 4.0, &[]))
    }))
    .unwrap();
    // DESc@20 says 0, the clip has id 1.
    assert_eq!(song.channels[1].lanes[1].last_clip_id, 1);
}

#[test]
fn more_than_4096_racks_is_too_large() {
    let racks = |count: usize| {
        file(&|w| {
            for _ in 0..count {
                rack(w, 0x100, 0, -1, &|_| Ok(()));
            }
            Ok(())
        })
    };
    assert_eq!(read(&racks(4097)).unwrap_err().kind, FlmErrorKind::TooLarge);
    // 4096 racks are read (and fail later: they have no CHNL, so no lanes).
    assert_eq!(
        read(&racks(4096)).unwrap_err().kind,
        FlmErrorKind::MissingChunk
    );
}

#[test]
fn a_short_channel_header_and_link_take_their_defaults() {
    let bytes = file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, &|w| {
            w.chunk(*b"RSMP", |w| {
                w.raw(&MAGIC_SM01)?;
                w.u32(0)?;
                w.chunk(*b"PADS", |w| w.raw(&[0; 22]))?;
                w.chunk(*b"SMPl", |w| {
                    w.raw(&MAGIC_SL02)?;
                    w.chunk(*b"MAIN", |w| w.raw(&[0; 1036]))?;
                    w.chunk(*b"LINk", |w| w.u32(3))
                })
            })
        });
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chunk(w, b"CHNL", &|w| {
            w.u32(0x100)?;
            w.raw(&MAGIC_CH02)?;
            w.chunk(*b"CHHD", |w| {
                w.char::<1024>("Old")?;
                w.u32(1)?;
                w.f64(1.0)?;
                w.f32(0.25)
            })?;
            lane(w, 3, -1, -1, &|_| Ok(()))
        });
        Ok(())
    });
    let song = read(&bytes).unwrap();
    let old = &song.channels[1];
    assert_eq!(
        (old.name.as_str(), old.zipped, old.hue),
        ("Old", true, 0.25)
    );
    assert_eq!(old.file.view, [0.0, 0.0]);
    assert!(!old.file.input_monitoring && !old.file.record_armed);
    let slot = &old.drum_kit.as_ref().unwrap().slots[0].file;
    assert_eq!(
        (slot.id, slot.output_bus, slot.midi_key, slot.send_target),
        (3, -1, 0xffff, -1)
    );
    assert!(slot.overlapping_voices);
    assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
}

#[test]
fn a_module_keeps_its_preset_path_and_sends_keep_their_own_buses() {
    let song = read(&file(&|w| {
        rack(w, 0, 0, -1, &|w| {
            w.chunk(*b"RMOd", |w| {
                w.u32(9)?;
                w.u32(2)?;
                w.chunk(*b"HEAD", |w| {
                    w.u32(9)?;
                    w.u32(0)?;
                    w.u8(1)
                })?;
                w.chunk(*b"DESC", |w| {
                    w.char::<256>("Filters")?;
                    w.char::<256>("Warm")?;
                    w.vstr16(b"My Presets/Warm.flmpst")
                })
            })
        });
        rack(w, 0x80, 2, -1, &|_| Ok(()));
        rack(w, 0x80, 2, -1, &|_| Ok(()));
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chnl(w, 1.0, 1, "A", &[0], 1);
        chnl(w, 2.0, 2, "B", &[0], 1);
        Ok(())
    }))
    .unwrap();
    assert_eq!(
        song.master().rack.modules[0].file.preset_path,
        "My Presets/Warm.flmpst"
    );
    let buses: Vec<i32> = song.channels.iter().map(|c| c.bus_id).collect();
    assert_eq!(buses, [-1, 2, 3]);
    assert_eq!(song.file.last_send_bus, 3);
    assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
}

#[test]
fn the_shortest_heads_are_read() {
    // 328 bytes: the oldest layout (legacy routing). 358: @354 without a version string.
    let head_of = |len: usize, next: u32| {
        file(&|w| {
            w.chunk(*b"HEAD", |w| {
                let mut head = vec![0; len];
                head[264..272].copy_from_slice(&100.0_f64.to_le_bytes());
                if len >= 358 {
                    head[354..358].copy_from_slice(&next.to_le_bytes());
                }
                w.raw(&head)
            })?;
            rack(w, 0, 0, -1, &|_| Ok(()));
            rack(w, 0x100, 0, -1, &|_| Ok(()));
            chnl(w, 0.0, 0, "MASTER", &[], 1);
            chnl(w, 1.0, 9, "Drums", &[0], 3);
            Ok(())
        })
    };
    let oldest = read(&head_of(328, 0)).unwrap();
    assert_eq!(oldest.tempo_bpm, 100.0);
    assert_eq!(oldest.channels[1].id, 1, "legacy: ids 0..N−1");
    let newer = read(&head_of(358, 12)).unwrap();
    assert_eq!(newer.channels[1].id, 9);
    assert_eq!(newer.file.next_channel_id, 12);
}

#[test]
fn a_channel_without_a_header_sorts_at_row_0() {
    let bytes = file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, &|_| Ok(()));
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chnl(w, 1.0, 1, "A", &[0], 0);
        chunk(w, b"CHNL", &|w| {
            w.u32(0x100)?;
            w.raw(&MAGIC_CH02)?;
            lane(w, 0, -1, -1, &|_| Ok(()))
        });
        Ok(())
    });
    let song = read(&bytes).unwrap();
    let names: Vec<&str> = song.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["MASTER", "Instrument", "A"]);
}

fn module_file(
    chunk_id: &[u8; 4],
    type_id: u32,
    head_type: u32,
    prms: &[f32],
    tail: &[u8],
) -> Vec<u8> {
    file(&|w| {
        rack(w, 0, 0, -1, &|w| {
            w.chunk(*chunk_id, |w| {
                w.u32(type_id)?;
                w.u32(2)?;
                w.chunk(*b"HEAD", |w| {
                    w.u32(head_type)?;
                    w.u32(0)?;
                    w.u8(1)
                })?;
                w.chunk(*b"DESC", |w| w.raw(&[0; 512]))?;
                w.chunk(*b"PRMS", |w| prms.iter().try_for_each(|&v| w.f32(v)))?;
                w.raw(tail)
            })
        });
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        Ok(())
    })
}

#[test]
fn a_legacy_rmod_keeps_its_trailing_minimized_and_key_range() {
    let song = read(&module_file(b"RMOD", 9, 9, &[1.0, 0.5], &[1, 10, 90])).unwrap();
    let module = &song.master().rack.modules[0];
    assert!(module.minimized);
    assert_eq!((module.file.key_low, module.file.key_high), (10, 90));
    assert_eq!(module.params[1], 0.5);
}

#[test]
fn a_reverb_v2_with_a_legacy_reverb_head_takes_its_params_by_name() {
    // Legacy Reverb (12): Enable, DECAY 0.11, HIGH DAMP 0.22, MIX 0.33, PREDELAY 0.44 …
    let legacy = [1.0, 0.11, 0.22, 0.33, 0.44];
    let song = read(&module_file(b"RMOd", 32, 12, &legacy, &[])).unwrap();
    let params = &song.master().rack.modules[0].params;
    // Reverb v2 (32): Enable, DECAY, HIGH DAMP, MIX, PAN (default 0.5), EARLY REFL. (default), PREDELAY …
    assert_eq!(&params[..7], [1.0, 0.11, 0.22, 0.33, 0.5, 0.25, 0.44]);
}

#[test]
fn names_read_are_written_back() {
    let bytes = file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, &|_| Ok(()));
        chunk(w, b"CHNL", &|w| {
            w.u32(0)?;
            w.raw(&MAGIC_CH02)?;
            chhd(w, 0.0, 0, ["MASTER", "", "MASTER"])?;
            lane(w, 1, 0, 0, &|_| Ok(()))
        });
        chunk(w, b"CHNL", &|w| {
            w.u32(0x100)?;
            w.raw(&MAGIC_CH02)?;
            chhd(w, 1.0, 1, ["CHANNEL", "Close Grand", "piano"])?;
            lane(w, 0, -1, -1, &|_| Ok(()))
        });
        Ok(())
    });
    let song = read(&bytes).unwrap();
    assert_eq!(song.channels[1].name, "piano");
    let written = write(&song).unwrap();
    for name in [&b"CHANNEL"[..], b"piano", b"Close Grand"] {
        assert!(written.windows(name.len()).any(|w| w == name));
    }
    // The master's custom name, the same as its base name, stays too.
    let master = written.windows(4).position(|w| w == b"CHHD").unwrap() + 8 + 1084;
    assert_eq!(
        &written[master..master + 4 + 6 + 4 + 4 + 6],
        b"\x06\0\0\0MASTER\0\0\0\0\x06\0\0\0MASTER"
    );
}

/// Master + one instrument whose rack and channel bodies are `rack_body` and `lane_body`.
fn one_channel(rack_body: Build, chnl_body: Build) -> Vec<u8> {
    file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, rack_body);
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chunk(w, b"CHNL", &|w| {
            w.u32(0x100)?;
            w.raw(&MAGIC_CH02)?;
            chhd(w, 1.0, 1, ["Instrument 1", "", ""])?;
            chnl_body(w)
        });
        Ok(())
    })
}

/// A v2 slot with a MAIN of `main` bytes, a LINk unless `link` is false, and an STRC of `strc` bytes.
fn slot_rack(
    main: usize,
    link: bool,
    strc: usize,
) -> impl Fn(&mut W) -> Result<(), honeydew::flm::FlmError> {
    move |w| {
        w.chunk(*b"RSMP", |w| {
            w.raw(&MAGIC_SM01)?;
            w.u32(1)?;
            w.chunk(*b"PADS", |w| w.raw(&[0; 22]))?;
            w.chunk(*b"SMPl", |w| {
                w.raw(&MAGIC_SL02)?;
                if main > 0 {
                    w.chunk(*b"MAIN", |w| w.raw(&vec![0; main]))?;
                }
                if link {
                    w.chunk(*b"LINk", |w| w.u32(1))?;
                }
                w.chunk(*b"STRC", |w| w.raw(&vec![0; strc]))
            })
        })
    }
}

/// A module of `type_id` (instance 2) with `children`.
fn module_rack(
    type_id: u32,
    children: Build<'_>,
) -> impl Fn(&mut W) -> Result<(), honeydew::flm::FlmError> + '_ {
    move |w| {
        w.chunk(*b"RMOd", |w| {
            w.u32(type_id)?;
            w.u32(2)?;
            children(w)
        })
    }
}

fn notes_lane(w: &mut W) -> Result<(), honeydew::flm::FlmError> {
    lane(w, 0, -1, -1, &|_| Ok(()))
}

#[test]
fn every_fatal_load_error_fails_the_song() {
    let head9 = |w: &mut W, t: u32| {
        w.chunk(*b"HEAD", |w| {
            w.u32(t)?;
            w.u32(0)?;
            w.u8(1)
        })
    };
    let desc = |w: &mut W| w.chunk(*b"DESC", |w| w.raw(&[0; 516]));
    let fine = |w: &mut W| -> Result<(), honeydew::flm::FlmError> { notes_lane(w) };
    let cases: Vec<(&str, Vec<u8>, FlmErrorKind)> = vec![
        (
            "TDIV < 2",
            file(&|w| w.chunk(*b"TDIV", |w| w.u8(4))),
            FlmErrorKind::TooShort,
        ),
        (
            "RHED < 16",
            one_channel(&|w| w.chunk(*b"RHED", |w| w.raw(&[0; 15])), &fine),
            FlmErrorKind::TooShort,
        ),
        (
            "RPRM < 24",
            one_channel(&|w| w.chunk(*b"RPRM", |w| w.raw(&[0; 23])), &fine),
            FlmErrorKind::TooShort,
        ),
        (
            "RSMP magic",
            one_channel(&|w| w.chunk(*b"RSMP", |w| w.raw(b"XXXX\0\0\0\0")), &fine),
            FlmErrorKind::InvalidMagic,
        ),
        (
            "PADS < 22",
            one_channel(
                &|w| {
                    w.chunk(*b"RSMP", |w| {
                        w.raw(&MAGIC_SM01)?;
                        w.u32(0)?;
                        w.chunk(*b"PADS", |w| w.raw(&[0; 21]))
                    })
                },
                &fine,
            ),
            FlmErrorKind::TooShort,
        ),
        (
            "MAIN < 1036",
            one_channel(&slot_rack(1035, true, 17), &fine),
            FlmErrorKind::TooShort,
        ),
        (
            "no MAIN",
            one_channel(&slot_rack(0, true, 17), &fine),
            FlmErrorKind::MissingChunk,
        ),
        (
            "no LINk",
            one_channel(&slot_rack(1036, false, 17), &fine),
            FlmErrorKind::MissingChunk,
        ),
        (
            "STRC < 17",
            one_channel(&slot_rack(1036, true, 16), &fine),
            FlmErrorKind::TooShort,
        ),
        (
            "unknown module type",
            one_channel(&module_rack(17, &|_| Ok(())), &fine),
            FlmErrorKind::BadValue,
        ),
        (
            "module HEAD < 9",
            one_channel(
                &module_rack(9, &|w| {
                    w.chunk(*b"HEAD", |w| w.raw(&[9, 0, 0, 0, 0, 0, 0, 0]))
                }),
                &fine,
            ),
            FlmErrorKind::TooShort,
        ),
        (
            "module HEAD type",
            one_channel(&module_rack(9, &|w| head9(w, 8)), &fine),
            FlmErrorKind::BadValue,
        ),
        (
            "ADD1 < 3",
            one_channel(
                &module_rack(9, &|w| w.chunk(*b"ADD1", |w| w.raw(&[0, 0]))),
                &fine,
            ),
            FlmErrorKind::TooShort,
        ),
        (
            "DESC < 512",
            one_channel(
                &module_rack(9, &|w| w.chunk(*b"DESC", |w| w.raw(&[0; 511]))),
                &fine,
            ),
            FlmErrorKind::TooShort,
        ),
        (
            "PRST unreadable",
            one_channel(
                &module_rack(9, &|w| w.chunk(*b"PRST", |w| w.u32(99))),
                &fine,
            ),
            FlmErrorKind::UnexpectedEof,
        ),
        (
            "CSTM not allowed",
            one_channel(
                &module_rack(9, &|w| {
                    head9(w, 9)?;
                    desc(w)?;
                    w.chunk(*b"CSTM", |w| w.u8(0))
                }),
                &fine,
            ),
            FlmErrorKind::BadValue,
        ),
        (
            "CSTM short",
            one_channel(
                &module_rack(24, &|w| w.chunk(*b"CSTM", |w| w.raw(&[0; 15]))),
                &fine,
            ),
            FlmErrorKind::TooShort,
        ),
        (
            "CHNL magic",
            file(&|w| {
                rack(w, 0, 0, -1, &|_| Ok(()));
                w.chunk(*b"CHNL", |w| {
                    w.u32(0)?;
                    w.raw(b"30HC")
                })
            }),
            FlmErrorKind::InvalidMagic,
        ),
        (
            "CHHD < 1040",
            one_channel(&|_| Ok(()), &|w| w.chunk(*b"CHHD", |w| w.raw(&[0; 1039]))),
            FlmErrorKind::TooShort,
        ),
        (
            "DESc < 1052",
            one_channel(&|_| Ok(()), &|w| {
                w.chunk(*b"TRKH", |w| w.chunk(*b"DESc", |w| w.raw(&[0; 1051])))
            }),
            FlmErrorKind::TooShort,
        ),
        (
            "CLIP < 4",
            one_channel(&|_| Ok(()), &|w| {
                lane(w, 0, -1, -1, &|w| w.chunk(*b"CLIP", |w| w.raw(&[0; 3])))
            }),
            FlmErrorKind::UnexpectedEof,
        ),
        (
            "CLIP magic",
            one_channel(&|_| Ok(()), &|w| {
                lane(w, 0, -1, -1, &|w| {
                    w.chunk(*b"CLIP", |w| {
                        w.u32(0)?;
                        w.raw(b"30LC")
                    })
                })
            }),
            FlmErrorKind::InvalidMagic,
        ),
        (
            "no channels",
            file(&|w| w.chunk(*b"META", |w| w.u8(0))),
            FlmErrorKind::MissingChunk,
        ),
        (
            "a channel without lanes",
            one_channel(&|_| Ok(()), &|_| Ok(())),
            FlmErrorKind::MissingChunk,
        ),
    ];
    for (name, bytes, kind) in cases {
        assert_eq!(
            read(&bytes).map(|_| ()).map_err(|e| e.kind),
            Err(kind),
            "{name}"
        );
    }
    // The shortest accepted forms of the same chunks.
    let ok = one_channel(&slot_rack(1036, true, 17), &|w| {
        w.chunk(*b"CHHD", |w| w.raw(&[0; 1040]))?;
        lane(w, 0, -1, -1, &|w| {
            w.chunk(*b"CLIP", |w| {
                w.u32(0)?;
                w.raw(&MAGIC_CL02)
            })
        })
    });
    assert!(read(&ok).is_ok());
}

#[test]
fn short_clip_and_meta_fields_take_their_defaults() {
    let song = read(&file(&|w| {
        w.chunk(*b"META", |w| w.raw(&[b'x'; 100]))?;
        rack(w, 0, 0, -1, &|_| Ok(()));
        chunk(w, b"CHNL", &|w| {
            w.u32(0)?;
            w.raw(&MAGIC_CH02)?;
            // Two DESc: the second is an unknown chunk kept with the lane. Two clips without CLHd.
            w.chunk(*b"TRKH", |w| {
                w.chunk(*b"DESc", |w| w.raw(&[0; 1052]))?;
                w.chunk(*b"DESc", |w| w.raw(&[7; 1052]))?;
                for start in [5, 0] {
                    w.chunk(*b"CLIP", |w| {
                        w.u32(start)?;
                        w.raw(&MAGIC_CL02)?;
                        w.chunk(*b"CLHd", |w| w.raw(&[0; 24]).and_then(|_| w.u32(start + 1)))
                    })?;
                }
                Ok(())
            })
        });
        Ok(())
    }))
    .unwrap();
    assert_eq!(song.file.meta.created, "x".repeat(100));
    assert_eq!(
        (
            song.file.meta.editing_seconds,
            song.file.meta.author.as_str()
        ),
        (0.0, "")
    );
    let lane = &song.master().lanes[0];
    assert_eq!(lane.file.unknown_chunks.len(), 1);
    assert_eq!(
        lane.last_clip_id, 6,
        "the counter covers the highest clip id"
    );
    let clip = &lane.clips[0];
    assert_eq!(
        (
            clip.visible_len_beats,
            clip.offset_beats,
            clip.file.source_len_beats
        ),
        (0.0, 0.0, 0.0)
    );
}

#[test]
fn only_a_send_rack_makes_a_send_channel() {
    // A 0x100 channel named like our sends is not one.
    let song = read(&file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, &|_| Ok(()));
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chnl(w, 1.0, 1, "Effect Track (AUX) 1", &[0], 1);
        Ok(())
    }))
    .unwrap();
    assert_eq!(song.channels[1].kind, ChannelKind::Instrument);
    // Legacy: a send without a bus is no target; an output of −1 goes to the master.
    let song = read(&routed_file(-1, &[], -1, true)).unwrap();
    assert_eq!(song.channels[2].file.sends[0].target_channel_id, 0);
}

#[test]
fn a_stretch_record_with_a_tail_is_written_back_as_it_was() {
    let bytes = one_channel(
        &|w| {
            w.chunk(*b"RSMP", |w| {
                w.raw(&MAGIC_SM01)?;
                w.u32(1)?;
                w.chunk(*b"PADS", |w| w.raw(&[0; 22]))?;
                w.chunk(*b"SMPl", |w| {
                    w.raw(&MAGIC_SL02)?;
                    w.chunk(*b"MAIN", |w| w.raw(&[0; 1051]))?;
                    w.chunk(*b"LINk", |w| w.u32(1))?;
                    w.chunk(*b"STR1", |w| {
                        w.u8(1)?;
                        w.f64(1.5)?;
                        w.f64(2.0)?;
                        w.u8(9)
                    })
                })
            })
        },
        &|w| lane(w, 3, -1, -1, &|_| Ok(())),
    );
    let song = read(&bytes).unwrap();
    let slot = &song.channels[1].drum_kit.as_ref().unwrap().slots[0].file;
    assert_eq!(
        (slot.stretch_mode, slot.stretch_ratio, slot.pitch_semitones),
        (1, 1.5, 2.0)
    );
    let written = write(&song).unwrap();
    let str1 = written.windows(4).position(|w| w == b"STR1").unwrap();
    assert_eq!(written[str1 + 4], 18, "the 18-byte STR1 stays");
}

#[test]
fn a_pads_stretch_mode_reads_and_saves_as_on_exactly_when_the_ratio_is_not_1() {
    // A file with the mode on and a ratio of 1 gives mode 0, and saves it as 0; a foreign file with
    // the mode off and a ratio of 1.5 gives mode 1, which plays stretched (`stretch_mode` is
    // what the sample cache renders by).
    for (mode, ratio, read_mode) in [(1, 1.0, 0), (0, 1.5, 1)] {
        let bytes = one_channel(
            &|w| {
                w.chunk(*b"RSMP", |w| {
                    w.raw(&MAGIC_SM01)?;
                    w.u32(1)?;
                    w.chunk(*b"PADS", |w| w.raw(&[0; 22]))?;
                    w.chunk(*b"SMPl", |w| {
                        w.raw(&MAGIC_SL02)?;
                        w.chunk(*b"MAIN", |w| w.raw(&[0; 1051]))?;
                        w.chunk(*b"LINk", |w| w.u32(1))?;
                        w.chunk(*b"STR1", |w| {
                            w.u8(mode)?;
                            w.f64(ratio)?;
                            w.f64(2.0)
                        })
                    })
                })
            },
            &|w| lane(w, 3, -1, -1, &|_| Ok(())),
        );
        let song = read(&bytes).unwrap();
        let slot = &song.channels[1].drum_kit.as_ref().unwrap().slots[0].file;
        assert_eq!(
            (slot.stretch_mode, slot.stretch_ratio, slot.pitch_semitones),
            (read_mode, ratio, 2.0)
        );
        let written = write(&song).unwrap();
        let str1 = written.windows(4).position(|w| w == b"STR1").unwrap();
        assert_eq!(written[str1 + 8], read_mode, "the STR1 mode byte");
        assert_eq!(read(&written).unwrap(), song);
    }
}

#[test]
fn legacy_drum_slots_send_to_the_send_channel_with_their_bus() {
    let slot = |w: &mut W, id: u32, bus: i32| {
        w.chunk(*b"SMPl", |w| {
            w.raw(&MAGIC_SL02)?;
            w.chunk(*b"MAIN", |w| w.raw(&[0; 1036]))?;
            w.chunk(*b"LINk", |w| {
                w.u32(id)?;
                w.i32(bus)
            })
        })
    };
    let bytes = file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x80, 3, -1, &|_| Ok(()));
        rack(w, 0x100, -1, -1, &|w| {
            w.chunk(*b"RSMP", |w| {
                w.raw(&MAGIC_SM01)?;
                w.u32(2)?;
                w.chunk(*b"PADS", |w| w.raw(&[0; 22]))?;
                slot(w, 1, 3)?;
                slot(w, 2, -1)
            })
        });
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chnl(w, 1.0, 1, "Send", &[], 1);
        chnl(w, 2.0, 2, "Drums", &[], 3);
        Ok(())
    });
    let song = read(&bytes).unwrap();
    let slots = &song.channels[2].drum_kit.as_ref().unwrap().slots;
    assert_eq!(
        (slots[0].file.send_target, slots[1].file.send_target),
        (1, -1)
    );
}

// --- More reader fixes ---

#[test]
fn a_lane_on_file_param_minus_1_reads_every_point_as_a_control_point() {
    // Param −1 (an unused common code, model −4): param + 1 = 0, so the keys carry no sign. The fuzz found the
    // regression: keys ±1 read half the points as generated, the writer saved them with key 0, and those read back as
    // control points.
    let bytes = instrument_file(&|w| {
        lane(w, 1, 2, -1, &|w| {
            clip(
                w,
                4.0,
                &[(0, 1, 0, 0, 0), (8, -1, 9, 0, 0), (16, 0, 99, 0, 0)],
            )
        })
    });
    let song = read(&bytes).unwrap();
    assert_eq!(song.channels[1].lanes[1].target.unwrap().param, -4);
    assert_eq!(control_flags(&song, 1), [true, true, true]);
    assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
}

#[test]
fn a_send_on_the_master_is_dropped() {
    let bytes = file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x80, 1, -1, &|_| Ok(()));
        chnl(w, 0.0, 0, "MASTER", &[1], 1);
        chnl(w, 1.0, 1, "Send", &[0], 1);
        Ok(())
    });
    let song = read(&bytes).unwrap();
    assert!(song.master().file.sends.is_empty());
    assert_eq!(song.master().output_bus, -1);
    assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
}

#[test]
fn drum_hits_follow_their_slot_when_the_pads_are_not_0_to_n() {
    // Slots on pads 2, 0, 5 (file order) sit at 1, 0, 2, and the hits keyed by pad follow
    // them. A hit on a pad without a slot (1 here, played by nothing) keeps playing nothing: below the slots it
    // goes to 255, past them it stays.
    let slot = |w: &mut W, pad: f64, id: u32| {
        w.chunk(*b"SMPl", |w| {
            w.raw(&MAGIC_SL02)?;
            w.chunk(*b"MAIN", |w| {
                w.i32(-1)?;
                w.f64(pad)?;
                w.char::<512>(&format!("Pad {pad}"))?;
                w.char::<512>("Drum Samples/Kick")
            })?;
            w.chunk(*b"LINk", |w| w.u32(id))
        })
    };
    let bytes = file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, &|w| {
            w.chunk(*b"RSMP", |w| {
                w.raw(&MAGIC_SM01)?;
                w.u32(3)?;
                w.chunk(*b"PADS", |w| w.raw(&[0; 22]))?;
                slot(w, 2.0, 1)?;
                slot(w, 0.0, 2)?;
                slot(w, 5.0, 3)
            })
        });
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chunk(w, b"CHNL", &|w| {
            w.u32(0x100)?;
            w.raw(&MAGIC_CH02)?;
            chhd(w, 1.0, 1, ["Drums 1", "", ""])?;
            let hits =
                [(0, 0), (1, 2), (2, 5), (3, 1), (4, 7)].map(|(pos, pad)| (pos, pad, 0x7fff, 0, 0));
            lane(w, 3, -1, -1, &|w| clip(w, 4.0, &hits))
        });
        Ok(())
    });
    let song = read(&bytes).unwrap();
    let kit = song.channels[1].drum_kit.as_ref().unwrap();
    let names: Vec<&str> = kit.slots.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Pad 0", "Pad 2", "Pad 5"]);
    let pads: Vec<u8> = (song.channels[1].lanes[0].clips[0].events.iter())
        .map(|e| match *e {
            Event::Drum { pad, .. } => pad,
            _ => panic!("{e:?}"),
        })
        .collect();
    assert_eq!(pads, [0, 1, 2, 255, 7]);
    assert_eq!(read(&write(&song).unwrap()).unwrap(), song);
}

#[test]
fn a_hit_on_a_pad_two_slots_share_lands_on_the_first() {
    // Slots on pads 0, 1, 1 (file order): the hit on pad 1 goes to the first slot there, at position 1.
    let slot = |w: &mut W, pad: f64, name: &str| {
        w.chunk(*b"SMPl", |w| {
            w.raw(&MAGIC_SL02)?;
            w.chunk(*b"MAIN", |w| {
                w.i32(-1)?;
                w.f64(pad)?;
                w.char::<512>(name)?;
                w.char::<512>("Drum Samples/Kick")
            })?;
            w.chunk(*b"LINk", |w| w.u32(0))
        })
    };
    let bytes = file(&|w| {
        rack(w, 0, 0, -1, &|_| Ok(()));
        rack(w, 0x100, 0, -1, &|w| {
            w.chunk(*b"RSMP", |w| {
                w.raw(&MAGIC_SM01)?;
                w.u32(0)?;
                w.chunk(*b"PADS", |w| w.raw(&[0; 22]))?;
                slot(w, 0.0, "A")?;
                slot(w, 1.0, "B")?;
                slot(w, 1.0, "C")
            })
        });
        chnl(w, 0.0, 0, "MASTER", &[], 1);
        chunk(w, b"CHNL", &|w| {
            w.u32(0x100)?;
            w.raw(&MAGIC_CH02)?;
            chhd(w, 1.0, 1, ["Drums 1", "", ""])?;
            lane(w, 3, -1, -1, &|w| clip(w, 4.0, &[(0, 1, 0x7fff, 0, 0)]))
        });
        Ok(())
    });
    let song = read(&bytes).unwrap();
    let kit = song.channels[1].drum_kit.as_ref().unwrap();
    let names: Vec<&str> = kit.slots.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["A", "B", "C"]);
    assert!(matches!(
        song.channels[1].lanes[0].clips[0].events[0],
        Event::Drum { pad: 1, .. }
    ));
}

#[test]
fn sorting_events_counts_against_the_memory_budget() {
    // 3M records of 4 bytes (a position only) in a 12 MB file: the budget is 64 MiB + 2 × 12 MB = 91 MB. The
    // events take 24 B each, 72 MB: they fit in order. Out of order the stable sort's scratch (half the events,
    // 36 MB more) does not, so that file is refused before the sort allocates it.
    let events = |count: u32, reversed: bool| {
        file(&|w| {
            rack(w, 0, 0, -1, &|_| Ok(()));
            chunk(w, b"CHNL", &|w| {
                w.u32(0)?;
                w.raw(&MAGIC_CH02)?;
                lane(w, 0, -1, -1, &|w| {
                    w.chunk(*b"CLIP", |w| {
                        w.u32(0)?;
                        w.raw(&MAGIC_CL02)?;
                        w.chunk(*b"EVN2", |w| {
                            w.u16(4)?;
                            let pos = |n: u32| if reversed { count - n } else { n };
                            let bytes: Vec<u8> =
                                (0..count).flat_map(|n| pos(n).to_le_bytes()).collect();
                            w.raw(&bytes)
                        })
                    })
                })
            });
            Ok(())
        })
    };
    assert!(read(&events(3_000_000, false)).is_ok());
    assert_eq!(
        read(&events(3_000_000, true)).unwrap_err().kind,
        FlmErrorKind::TooLarge
    );
}

// --- Module presets ---

use honeydew::flm::{preset_fits, read_preset};
use honeydew::song::ModuleInstance;

/// A module preset of the old effect form: HEAD of `type_id`, a 512-byte DESC (Rooms / Hall), PRMS `params`, and
/// no ADD1 or SMPR.
fn legacy_preset(type_id: u32, params: &[f32]) -> Vec<u8> {
    let mut w = W::default();
    chunk(&mut w, b"HEAD", &|w| {
        w.u32(type_id)?;
        w.u32(0)?;
        w.u8(1)
    });
    chunk(&mut w, b"DESC", &|w| {
        w.char::<256>("Rooms")?;
        w.char::<256>("Hall")
    });
    chunk(&mut w, b"PRMS", &|w| {
        params.iter().try_for_each(|&v| w.f32(v))
    });
    w.into_bytes()
}

#[test]
fn a_legacy_preset_takes_the_common_defaults_and_a_short_prms_the_module_defaults() {
    // Distortion (6).
    let defaults = ModuleInstance::new(6, 0);
    let preset = read_preset(&legacy_preset(6, &[1.0, 0.75]), Some(6)).unwrap();
    assert_eq!(preset.params[..2], [1.0, 0.75]);
    assert_eq!(preset.params[2..], defaults.params[2..]);
    assert_eq!(preset.common, defaults.common);
    assert_eq!(preset.file.custom_data, None);
    assert_eq!(
        (
            preset.file.category.as_str(),
            preset.file.preset_name.as_str()
        ),
        ("Rooms", "Hall")
    );
    // Extra values are ignored.
    let long = vec![0.5; defaults.params.len() + 3];
    let preset = read_preset(&legacy_preset(6, &long), Some(6)).unwrap();
    assert_eq!(preset.params.len(), defaults.params.len());
}

#[test]
fn a_preset_loads_into_its_own_type_only_and_a_legacy_reverbs_into_reverb_v2() {
    let distortion = legacy_preset(6, &[1.0]);
    assert!(preset_fits(&distortion, 6));
    assert!(!preset_fits(&distortion, 11));
    assert!(read_preset(&distortion, Some(11)).is_err());
    assert!(!preset_fits(b"", 6));
    // Legacy Reverb (12) into Reverb v2 (32), by param name, as in a song.
    let legacy = legacy_preset(12, &[1.0, 0.11, 0.22, 0.33, 0.44]);
    assert!(preset_fits(&legacy, 32) && preset_fits(&legacy, 12));
    let reverb = read_preset(&legacy, Some(32)).unwrap();
    assert_eq!(reverb.type_id, 32);
    assert_eq!(
        &reverb.params[..7],
        [1.0, 0.11, 0.22, 0.33, 0.5, 0.25, 0.44]
    );
    // Not the other way, and without a target the HEAD's own type.
    assert!(!preset_fits(&legacy_preset(32, &[1.0]), 12));
    assert_eq!(read_preset(&legacy, None).unwrap().type_id, 12);
}

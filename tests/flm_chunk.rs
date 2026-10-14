use honeydew::flm::chunk::{
    ChunkIter, ChunkWriter, ContentPath, ContentRoot, Cursor, FlmError, FlmErrorKind, MAGIC_CL01,
    MAGIC_CL02, MAGIC_FL01, MAGIC_RK01, MAX_CHUNK_SIZE, MAX_FILE_SIZE, Prefix, PrefixKind,
    file_chunks,
};

#[test]
fn primitives_strings_and_content_paths_round_trip() {
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"DATA", |writer| {
            writer.u8(7)?;
            writer.u16(0x1234)?;
            writer.u32(0x1234_5678)?;
            writer.i16(-123)?;
            writer.i32(-123_456)?;
            writer.u64(0x1234_5678_90ab_cdef)?;
            writer.f32(1.25)?;
            writer.f64(-2.5)?;
            writer.char::<8>("hé")?;
            writer.vstr16(b"one\0two")?;
            writer.vstr32(b"longer")?;
            writer.content_path::<48>(&ContentPath::Shorthand {
                root: ContentRoot::App,
                relative: "Drum Samples/SFX/Name.wav".into(),
            })?;
            Ok(())
        })
        .unwrap();
    let bytes = writer.into_bytes();
    assert_eq!(&bytes[..4], &MAGIC_FL01);
    let mut chunks = file_chunks(&bytes).unwrap();
    let chunk = chunks.next().unwrap().unwrap();
    assert_eq!(chunk.id, *b"DATA");
    assert!(chunks.next().is_none());
    let mut reader = chunk.reader();
    assert_eq!(reader.u8().unwrap(), 7);
    assert_eq!(reader.u16().unwrap(), 0x1234);
    assert_eq!(reader.u32().unwrap(), 0x1234_5678);
    assert_eq!(reader.i16().unwrap(), -123);
    assert_eq!(reader.i32().unwrap(), -123_456);
    assert_eq!(reader.u64().unwrap(), 0x1234_5678_90ab_cdef);
    assert_eq!(reader.f32().unwrap(), 1.25);
    assert_eq!(reader.f64().unwrap(), -2.5);
    assert_eq!(reader.char::<8>().unwrap(), "hé");
    assert_eq!(reader.vstr16().unwrap(), b"one\0two");
    assert_eq!(reader.vstr32().unwrap(), b"longer");
    let path = reader.content_path::<48>().unwrap();
    assert_eq!(
        path,
        ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "Drum Samples/SFX/Name.wav".into(),
        }
    );
    assert_eq!(reader.remaining(), 0);
}

#[test]
fn nested_container_prefixes_and_fixed_struct_tails() {
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"RACK", |writer| {
            writer.u32(0x100)?;
            writer.raw(&MAGIC_RK01)?;
            writer.chunk(*b"RHED", |writer| writer.raw(&[1, 2, 3]))?;
            writer.chunk(*b"CLIP", |writer| {
                writer.u32(128)?;
                writer.raw(&MAGIC_CL02)?;
                writer.chunk(*b"CLHd", |writer| writer.raw(&[9; 10]))
            })
        })
        .unwrap();
    let bytes = writer.into_bytes();
    let rack = file_chunks(&bytes).unwrap().next().unwrap().unwrap();
    let (prefix, mut children) = rack.container(PrefixKind::U32Magic).unwrap();
    assert_eq!(prefix, Prefix::U32Magic(0x100, MAGIC_RK01));
    let header = children.next().unwrap().unwrap();
    assert_eq!(header.path, "RACK[0]/RHED[0]");
    let (fixed, extra) = header.fixed::<8>();
    assert_eq!(fixed, [1, 2, 3, 0, 0, 0, 0, 0]);
    assert!(extra.is_empty());
    let clip = children.next().unwrap().unwrap();
    let (prefix, mut grandkids) = clip.container(PrefixKind::U32Magic).unwrap();
    assert_eq!(prefix, Prefix::U32Magic(128, MAGIC_CL02));
    let clip_header = grandkids.next().unwrap().unwrap();
    let (fixed, extra) = clip_header.fixed::<4>();
    assert_eq!(fixed, [9; 4]);
    assert_eq!(extra, &[9; 6]);
    assert!(grandkids.next().is_none());
    assert!(children.next().is_none());
}

#[test]
fn every_container_prefix_form_reads_its_children() {
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"CLSm", |writer| {
            writer.raw(b"20LS")?;
            writer.chunk(*b"MAIN", |writer| writer.u8(1))
        })
        .unwrap();
    writer
        .chunk(*b"RSMP", |writer| {
            writer.raw(b"10MS")?;
            writer.u32(7)?;
            writer.chunk(*b"PADS", |writer| writer.u8(2))
        })
        .unwrap();
    writer
        .chunk(*b"RMOd", |writer| {
            writer.u32(45)?;
            writer.u32(2)?;
            writer.chunk(*b"HEAD", |writer| writer.u8(3))
        })
        .unwrap();
    let bytes = writer.into_bytes();
    let mut chunks = file_chunks(&bytes).unwrap();
    for (kind, expected) in [
        (PrefixKind::Magic, Prefix::Magic(*b"20LS")),
        (PrefixKind::MagicU32, Prefix::MagicU32(*b"10MS", 7)),
        (PrefixKind::TwoU32, Prefix::TwoU32(45, 2)),
    ] {
        let chunk = chunks.next().unwrap().unwrap();
        let (prefix, mut children) = chunk.container(kind).unwrap();
        assert_eq!(prefix, expected);
        assert!(children.next().unwrap().is_ok());
        assert!(children.next().is_none());
    }
    assert!(chunks.next().is_none());
}

#[test]
fn chunk_lists_clamp_payloads_and_ignore_short_tails() {
    let truncated = b"10LFHEAD\x08\x00\x00\x00abc";
    let mut chunks = file_chunks(truncated).unwrap();
    assert_eq!(chunks.next().unwrap().unwrap().payload, b"abc");
    assert!(chunks.next().is_none());

    let short_header = b"10LFHEAD\x08\x00";
    assert!(file_chunks(short_header).unwrap().next().is_none());

    let trailing = b"10LFHEAD\x00\x00\x00\x00x";
    let mut chunks = file_chunks(trailing).unwrap();
    assert!(chunks.next().unwrap().is_ok());
    assert!(chunks.next().is_none());

    let oversized = b"10LFHEAD\x01\x00\x00\x10";
    assert_eq!(
        file_chunks(oversized)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .payload,
        b""
    );

    let garbage = b"10LF\xff\xff\xff\xff\x00\x00\x00\x00";
    let odd = file_chunks(garbage).unwrap().next().unwrap().unwrap();
    assert_eq!(odd.id, [0xff; 4]);
    assert!(odd.path.contains("ff"));
    assert_eq!(
        file_chunks(b"junk").unwrap_err().kind,
        FlmErrorKind::InvalidMagic
    );

    let mut reader = Cursor::new(b"x\0junk!!", "NAME");
    assert_eq!(reader.char::<8>().unwrap(), "x");
    let mut reader = Cursor::new(b"\xff\0", "NAME");
    assert_eq!(reader.char::<2>().unwrap(), "?");
    let mut reader = Cursor::new(b"@R@\0name.wav\0old", "PATH");
    assert_eq!(
        reader.content_path::<16>().unwrap(),
        ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "name.wav".into(),
        }
    );
    let mut reader = Cursor::new(b"xxxy", "PATH");
    assert_eq!(
        reader.content_path::<4>().unwrap(),
        ContentPath::Plain("xxxy".into())
    );
    let mut reader = Cursor::new(b"\x05\x00ab", "NAME");
    assert_eq!(reader.vstr16().unwrap_err().path, "NAME");
    let mut reader = Cursor::new(b"nope", "MAGIC");
    assert_eq!(
        reader.expect_magic(MAGIC_RK01).unwrap_err().kind,
        FlmErrorKind::InvalidMagic
    );

    let mut writer = ChunkWriter::new_file();
    assert_eq!(
        writer.vstr16(&vec![0; 65_536]).unwrap_err().kind,
        FlmErrorKind::InvalidLength
    );
    assert!(
        writer
            .chunk(*b"FAIL", |writer| writer.vstr16(&vec![0; 65_536]))
            .is_err()
    );
    assert_eq!(writer.into_bytes(), MAGIC_FL01);

    let mut writer = ChunkWriter::default();
    writer.char::<8>("x").unwrap();
    assert_eq!(writer.into_bytes(), b"x\0\0\0\0\0\0\0");
}

#[test]
fn fixed_strings_truncate_on_utf8_boundaries_and_accept_full_fields() {
    let mut writer = ChunkWriter::default();
    writer.char::<8>("abcdefgh").unwrap();
    writer.char::<4>("ééé").unwrap();
    writer.char::<3>("éé").unwrap();
    writer.char::<4>("a\0b").unwrap();
    let bytes = writer.into_bytes();
    assert_eq!(&bytes[..8], b"abcdefgh");
    assert_eq!(&bytes[8..12], "éé".as_bytes());
    assert_eq!(&bytes[12..15], &[0xc3, 0xa9, 0]);
    assert_eq!(&bytes[15..19], b"a\0b\0");
    let mut reader = Cursor::new(&bytes, "STR");
    assert_eq!(reader.char::<8>().unwrap(), "abcdefgh");
    assert_eq!(reader.char::<4>().unwrap(), "éé");
    assert_eq!(reader.char::<3>().unwrap(), "é");
    assert_eq!(reader.char::<4>().unwrap(), "a");
}

#[test]
fn content_paths_preserve_root_and_plain_form_in_each_encoding() {
    let user = ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: "My Samples/kick.wav".into(),
    };
    let plain = ContentPath::Plain("@R@/outside.wav".into());
    let mut writer = ChunkWriter::default();
    writer.content_path::<32>(&user).unwrap();
    writer.content_path16(&plain).unwrap();
    writer.content_path32(&user).unwrap();
    let bytes = writer.into_bytes();
    assert_eq!(&bytes[..4], b"@L@\0");
    let mut reader = Cursor::new(&bytes, "PATH");
    assert_eq!(reader.content_path::<32>().unwrap(), user);
    assert_eq!(reader.content_path16().unwrap(), plain);
    assert_eq!(reader.content_path32().unwrap(), user);

    let mut reader = Cursor::new(b"@T@\0folder\\item\0", "PATH");
    assert_eq!(
        reader.content_path::<16>().unwrap(),
        ContentPath::Shorthand {
            root: ContentRoot::Temp,
            relative: "folder/item".into(),
        }
    );
    let mut writer = ChunkWriter::default();
    assert_eq!(
        writer
            .content_path::<16>(&ContentPath::Shorthand {
                root: ContentRoot::Temp,
                relative: "item".into(),
            })
            .unwrap_err()
            .kind,
        FlmErrorKind::InvalidContentPath
    );
}

#[test]
fn chunk_paths_and_writer_errors_are_specific() {
    let mut writer = ChunkWriter::new_file();
    writer.chunk(*b"CLIP", |_| Ok(())).unwrap();
    writer.chunk(*b"CLIP", |_| Ok(())).unwrap();
    let bytes = writer.into_bytes();
    let mut chunks = file_chunks(&bytes).unwrap();
    assert_eq!(chunks.next().unwrap().unwrap().path, "CLIP[0]");
    assert_eq!(chunks.next().unwrap().unwrap().path, "CLIP[1]");

    let mut writer = ChunkWriter::default();
    writer
        .content_path::<4>(&ContentPath::Plain("long".into()))
        .unwrap();
    assert_eq!(writer.into_bytes(), b"long");
    let mut writer = ChunkWriter::default();
    assert_eq!(
        writer
            .content_path::<4>(&ContentPath::Plain("longer".into()))
            .unwrap_err()
            .kind,
        FlmErrorKind::InvalidLength
    );
    assert_eq!(
        writer
            .content_path::<8>(&ContentPath::Plain("a\0b".into()))
            .unwrap_err()
            .kind,
        FlmErrorKind::InvalidContentPath
    );
    assert_eq!(
        writer.chunk([0, 0, 0, 0], |_| Ok(())).unwrap_err().kind,
        FlmErrorKind::InvalidChunkId
    );
    assert_eq!(
        writer.chunk([0, 0, 0, 0], |_| Ok(())).unwrap_err().path,
        "FLM"
    );
    assert_eq!(MAGIC_CL01, *b"10LC");

    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"RACK", |writer| {
            let error = writer
                .chunk(*b"CLIP", |writer| {
                    writer.content_path::<4>(&ContentPath::Plain("too long".into()))
                })
                .unwrap_err();
            assert_eq!(error.path, "RACK[0]/CLIP[0]");
            writer.chunk(*b"CLIP", |writer| writer.u8(1))
        })
        .unwrap();
    let bytes = writer.into_bytes();
    let rack = file_chunks(&bytes).unwrap().next().unwrap().unwrap();
    assert_eq!(
        rack.children().next().unwrap().unwrap().path,
        "RACK[0]/CLIP[0]"
    );

    let mut writer = ChunkWriter::default();
    writer
        .content_path::<8>(&ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "1234".into(),
        })
        .unwrap();
    assert_eq!(writer.into_bytes(), b"@R@\x001234");
    let mut writer = ChunkWriter::default();
    assert_eq!(
        writer
            .content_path::<8>(&ContentPath::Shorthand {
                root: ContentRoot::App,
                relative: "12345".into(),
            })
            .unwrap_err()
            .kind,
        FlmErrorKind::InvalidLength
    );
}

#[test]
fn variable_strings_reject_invalid_lengths_with_paths() {
    let mut reader = Cursor::new(&[5, 0, 0, 0], "RACK[0]/PATH");
    assert_eq!(reader.vstr32().unwrap_err().path, "RACK[0]/PATH");
    let mut reader = Cursor::new(&[5, 0, 0, 0], "RACK[0]/PATH");
    assert_eq!(
        reader.vstr32().unwrap_err().kind,
        FlmErrorKind::UnexpectedEof
    );
    let length = 0x1000_0001_u32.to_le_bytes();
    let mut reader = Cursor::new(&length, "RACK[0]/PATH");
    assert_eq!(reader.vstr32().unwrap_err().kind, FlmErrorKind::TooLarge);
}

#[test]
fn fixed_defaults_cursor_tail_and_expected_container_magic() {
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"CLIP", |writer| {
            writer.u32(7)?;
            writer.raw(&MAGIC_CL01)?;
            writer.chunk(*b"CLHd", |writer| writer.raw(&[1, 2, 3]))
        })
        .unwrap();
    let bytes = writer.into_bytes();
    let clip = file_chunks(&bytes).unwrap().next().unwrap().unwrap();
    let (_, mut children) = clip
        .container_expect(PrefixKind::U32Magic, &[MAGIC_CL02, MAGIC_CL01])
        .unwrap();
    let header = children.next().unwrap().unwrap();
    let (field, extra) = header.fixed_or::<5>([9; 5]);
    assert_eq!(field, [1, 2, 3, 9, 9]);
    assert!(extra.is_empty());
    let mut cursor = Cursor::new(header.payload, &header.path);
    cursor.skip(2).unwrap();
    assert_eq!(cursor.rest(), &[3]);
    assert_eq!(cursor.remaining(), 0);
    let mut cursor = Cursor::new(&[7, 0, 9], "HEAD[0]");
    assert_eq!(cursor.u16_or(4), 7);
    assert_eq!(cursor.u32_or(120), 120);
    assert_eq!(cursor.u8_or(5), 5);
    assert_eq!(cursor.f32_or(0.5), 0.5);
    assert_eq!(
        clip.container_expect(PrefixKind::U32Magic, &[MAGIC_RK01])
            .unwrap_err()
            .kind,
        FlmErrorKind::InvalidMagic
    );
    assert_eq!(
        clip.container_expect(PrefixKind::U32Magic, &[MAGIC_RK01])
            .unwrap_err()
            .path,
        "CLIP[0]"
    );
}

#[test]
fn every_header_length_boundary_is_bounded() {
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"RACK", |writer| {
            writer.u32(1)?;
            writer.raw(&MAGIC_RK01)?;
            writer.chunk(*b"RHED", |writer| writer.raw(&[1; 9]))?;
            writer.chunk(*b"CLIP", |writer| {
                writer.u32(1)?;
                writer.raw(&MAGIC_CL02)?;
                writer.chunk(*b"CLHd", |writer| writer.raw(&[2; 13]))
            })
        })
        .unwrap();
    let original = writer.into_bytes();
    for id in [b"RACK", b"RHED", b"CLIP", b"CLHd"] {
        let offset = original.windows(4).position(|bytes| bytes == id).unwrap();
        let remaining = original.len() - offset - 8;
        for length in [
            0,
            1,
            remaining.saturating_sub(1),
            remaining,
            remaining + 1,
            0x0fff_ffff,
            0x1000_0001,
            u32::MAX as usize,
        ] {
            let mut bytes = original.clone();
            bytes[offset + 4..offset + 8].copy_from_slice(&(length as u32).to_le_bytes());
            let result =
                std::panic::catch_unwind(|| inspect(&bytes)).expect("mutated input panicked");
            if let Err(error) = result {
                assert!(
                    !error.path.is_empty(),
                    "missing path for {id:?}, size {length}"
                );
            }
        }
    }
}

#[test]
fn mutated_text_fields_never_panic_and_errors_keep_paths() {
    let original = b"@R@\0folder\\sound.wav\0";
    for index in 0..original.len() {
        for replacement in [0, 0x80, 0xff] {
            let mut bytes = original.to_vec();
            bytes[index] = replacement;
            let result = std::panic::catch_unwind(|| {
                let mut reader = Cursor::new(&bytes, "RACK[0]/PATH");
                let _ = reader.char::<8>();
                let _ = reader.content_path::<8>();
                let _ = reader.vstr16();
                reader.vstr32()
            })
            .expect("mutated text field panicked");
            if let Err(error) = result {
                assert_eq!(error.path, "RACK[0]/PATH");
            }
        }
    }
}

#[test]
fn file_and_chunk_size_limits() {
    {
        let large_file = vec![0; MAX_FILE_SIZE + 1];
        assert_eq!(
            file_chunks(&large_file).unwrap_err().kind,
            FlmErrorKind::TooLarge
        );
        let mut writer = ChunkWriter::new_file();
        assert_eq!(
            writer.raw(&large_file).unwrap_err().kind,
            FlmErrorKind::TooLarge
        );
    }
    let mut large_chunk = vec![0; MAX_CHUNK_SIZE + 13];
    large_chunk[..4].copy_from_slice(&MAGIC_FL01);
    large_chunk[4..8].copy_from_slice(b"DATA");
    large_chunk[8..12].copy_from_slice(&((MAX_CHUNK_SIZE + 1) as u32).to_le_bytes());
    let mut chunks = file_chunks(&large_chunk).unwrap();
    assert_eq!(
        chunks.next().unwrap().unwrap_err().kind,
        FlmErrorKind::TooLarge
    );
    assert!(chunks.next().is_none());
    let mut writer = ChunkWriter::new_file();
    assert_eq!(
        writer
            .chunk(*b"DATA", |writer| writer
                .raw(&large_chunk[..MAX_CHUNK_SIZE + 1]))
            .unwrap_err()
            .kind,
        FlmErrorKind::TooLarge
    );
}

fn inspect(bytes: &[u8]) -> Result<(), FlmError> {
    for chunk in file_chunks(bytes)? {
        let chunk = chunk?;
        if chunk.id == *b"RACK" {
            let (_, children) = chunk.container(PrefixKind::U32Magic)?;
            for child in children {
                let child = child?;
                if child.id == *b"CLIP" {
                    let (_, grandchildren) = child.container(PrefixKind::U32Magic)?;
                    for grandchild in grandchildren {
                        grandchild?;
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn ten_thousand_deterministic_corruptions_never_panic() {
    let mut writer = ChunkWriter::new_file();
    writer
        .chunk(*b"RACK", |writer| {
            writer.u32(0x100)?;
            writer.raw(&MAGIC_RK01)?;
            writer.chunk(*b"RHED", |writer| writer.raw(&[0; 16]))?;
            writer.chunk(*b"CLIP", |writer| {
                writer.u32(0)?;
                writer.raw(&MAGIC_CL02)?;
                writer.chunk(*b"CLHd", |writer| writer.raw(&[0; 37]))
            })
        })
        .unwrap();
    let original = writer.into_bytes();
    assert!(inspect(&original).is_ok());
    let mut seed = 0x1234_5678_u64;
    for iteration in 0..10_000 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let mut bytes = original.clone();
        let offset = seed as usize % bytes.len();
        match iteration % 3 {
            0 => bytes[offset] ^= (seed >> 32) as u8,
            1 => bytes.truncate(offset),
            _ => {
                for byte in bytes.iter_mut().skip(offset).take(4) {
                    *byte = (seed >> 24) as u8;
                }
            }
        }
        let result = std::panic::catch_unwind(|| inspect(&bytes)).expect("mutation panicked");
        if let Err(error) = result {
            assert!(!error.path.is_empty());
        }
    }
}

#[test]
fn floats_that_are_not_finite_read_as_zero() {
    let mut writer = ChunkWriter::default();
    writer.f32(f32::NAN).unwrap();
    writer.f64(-f64::NAN).unwrap();
    writer.f32(f32::INFINITY).unwrap();
    writer.f64(f64::NEG_INFINITY).unwrap();
    writer.f64(f64::MAX).unwrap();
    let bytes = writer.into_bytes();
    let mut reader = Cursor::new(&bytes, "X");
    assert_eq!(reader.f32().unwrap().to_bits(), 0);
    assert_eq!(reader.f64_or(1.0).to_bits(), 0);
    assert_eq!(reader.f32().unwrap().to_bits(), 0);
    assert_eq!(reader.f64().unwrap().to_bits(), 0);
    assert_eq!(reader.f64().unwrap(), f64::MAX);
}

#[test]
fn lossy_text_never_takes_more_bytes_than_it_read() {
    let mut reader = Cursor::new(&[0xff, 0xfe, b'a', 0xff], "X");
    let text = reader.char::<4>().unwrap();
    assert!(text.len() <= 4, "{text:?}");
    assert_eq!(honeydew::flm::chunk::text(&[0xff; 7]), "???????");
    assert_eq!(honeydew::flm::chunk::text(b"ok"), "ok");
}

#[test]
fn chunk_indexes_are_counted_for_the_first_64_ids_only() {
    let mut writer = ChunkWriter::default();
    for n in 0..70_u32 {
        let id = format!("{n:04}");
        writer
            .chunk(id.as_bytes().try_into().unwrap(), |_| Ok(()))
            .unwrap();
        writer.chunk(*b"0000", |_| Ok(())).unwrap();
    }
    let bytes = writer.into_bytes();
    let paths: Vec<String> = ChunkIter::new(&bytes, "")
        .map(|c| c.unwrap().path)
        .collect();
    assert_eq!(paths[0], "0000[0]");
    assert_eq!(paths[3], "0000[2]");
    assert_eq!(paths[126], "0063[0]");
    assert_eq!(paths[128], "0064[?]");
    assert_eq!(paths[139], "0000[70]");
}

#[test]
fn a_chunk_list_hands_over_the_bytes_after_its_last_chunk() {
    let mut writer = ChunkWriter::default();
    writer.chunk(*b"HEAD", |w| w.u8(1)).unwrap();
    // 7 bytes: one short of a chunk header, so no chunk and no error.
    writer.raw(&[1, 2, 3, 4, 5, 6, 7]).unwrap();
    let bytes = writer.into_bytes();
    let mut chunks = ChunkIter::new(&bytes, "");
    assert_eq!(chunks.next().unwrap().unwrap().id, *b"HEAD");
    assert!(chunks.next().is_none());
    assert_eq!(chunks.remainder(), [1, 2, 3, 4, 5, 6, 7]);
}

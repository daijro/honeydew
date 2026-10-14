//! The ZIP codec's tests: archives are built field by field from the PKWARE APPNOTE layout (so the reader is not
//! checked against the writer alone), two real Info-ZIP archives are read, and hostile shapes are refused.

use miniz_oxide::deflate::compress_to_vec;

use super::*;

const BIG: u64 = u64::MAX;

fn put(v: &mut Vec<u8>, parts: &[&[u8]]) {
    for part in parts {
        v.extend_from_slice(part);
    }
}

/// One entry of a hand-built archive with every header field explicit (APPNOTE 4.3.7 and 4.3.12).
#[derive(Clone)]
struct Raw {
    name: Vec<u8>,
    flags: u16,
    method: u16,
    made_by: u16,
    attr: u32,
    crc: u32,
    size: u32,
    packed: Vec<u8>,
    /// Bytes after the local name (the local header's extra field).
    extra: Vec<u8>,
    /// Bit 3's layout: the local header's crc and sizes are zero and a descriptor follows the data.
    descriptor: bool,
}

fn stored(name: &str, data: &[u8]) -> Raw {
    Raw {
        name: name.as_bytes().to_vec(),
        flags: 0x0800,
        method: 0,
        made_by: 20,
        attr: 0,
        crc: crc32fast::hash(data),
        size: data.len() as u32,
        packed: data.to_vec(),
        extra: Vec::new(),
        descriptor: false,
    }
}

fn deflated(name: &str, data: &[u8]) -> Raw {
    Raw {
        method: 8,
        packed: compress_to_vec(data, 6),
        ..stored(name, data)
    }
}

/// Local headers, central directory and end record, in the APPNOTE order.
fn build(raws: &[Raw], comment: &[u8]) -> Vec<u8> {
    let (mut out, mut cd) = (Vec::new(), Vec::new());
    for r in raws {
        let at = (out.len() as u32).to_le_bytes();
        let (nlen, csize) = (
            (r.name.len() as u16).to_le_bytes(),
            (r.packed.len() as u32).to_le_bytes(),
        );
        let (flags, method, crc, size) = (
            r.flags.to_le_bytes(),
            r.method.to_le_bytes(),
            r.crc.to_le_bytes(),
            r.size.to_le_bytes(),
        );
        let (time, date, zero) = ([0, 0], [0x21, 0], [0; 4]);
        let (l_crc, l_csize, l_size) = if r.descriptor {
            (zero, zero, zero)
        } else {
            (crc, csize, size)
        };
        let elen = (r.extra.len() as u16).to_le_bytes();
        put(
            &mut out,
            &[
                b"PK\x03\x04",
                &[20, 0],
                &flags,
                &method,
                &time,
                &date,
                &l_crc,
                &l_csize,
                &l_size,
            ],
        );
        put(&mut out, &[&nlen, &elen, &r.name, &r.extra, &r.packed]);
        if r.descriptor {
            put(&mut out, &[b"PK\x07\x08", &crc, &csize, &size]);
        }
        put(
            &mut cd,
            &[
                b"PK\x01\x02",
                &r.made_by.to_le_bytes(),
                &[20, 0],
                &flags,
                &method,
                &time,
                &date,
            ],
        );
        put(
            &mut cd,
            &[
                &crc,
                &csize,
                &size,
                &nlen,
                &[0; 8],
                &r.attr.to_le_bytes(),
                &at,
                &r.name,
            ],
        );
    }
    let (cd_at, n) = (out.len() as u32, (raws.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd);
    put(
        &mut out,
        &[
            b"PK\x05\x06",
            &[0; 4],
            &n,
            &n,
            &(cd.len() as u32).to_le_bytes(),
            &cd_at.to_le_bytes(),
        ],
    );
    put(&mut out, &[&(comment.len() as u16).to_le_bytes(), comment]);
    out
}

fn find(bytes: &[u8], sig: &[u8; 4]) -> usize {
    bytes.windows(4).position(|w| w == sig).unwrap()
}

/// The first central directory record, the end record.
fn cd_at(bytes: &[u8]) -> usize {
    find(bytes, b"PK\x01\x02")
}

fn eocd_at(bytes: &[u8]) -> usize {
    bytes.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap()
}

fn patch(mut bytes: Vec<u8>, at: usize, with: &[u8]) -> Vec<u8> {
    bytes[at..at + with.len()].copy_from_slice(with);
    bytes
}

fn e(name: &str, data: &[u8]) -> (String, Vec<u8>) {
    (name.to_owned(), data.to_vec())
}

fn entry(path: &str, data: &[u8]) -> Entry {
    Entry {
        path: path.to_owned(),
        data: data.to_vec(),
    }
}

/// Deterministic bytes that deflate cannot shrink.
fn noise(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 11) as u8
        })
        .collect()
}

#[test]
fn round_trip_stored_and_deflated() {
    let text = b"the quick brown fox ".repeat(200);
    let random = noise(3000, 7);
    let entries = vec![
        e("song.flm", &text),
        e("Samples/kick.wav", &random),
        e("empty.txt", b""),
        e("one", b"x"),
    ];
    let zip = write(&entries).unwrap();
    let got = read(&zip, BIG).unwrap();
    let want: Vec<Entry> = entries.iter().map(|(p, d)| entry(p, d)).collect();
    assert_eq!(got, want);
    // Deflated only when that is smaller: text yes, noise, the empty file and one byte no.
    let methods: Vec<u16> = zip
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"PK\x01\x02")
        .map(|(at, _)| u16::from_le_bytes([zip[at + 10], zip[at + 11]]))
        .collect();
    assert_eq!(methods, [8, 0, 0, 0]);
    assert!(zip.len() < text.len() / 4 + random.len() + 600);
}

#[test]
fn output_is_deterministic_utf8_and_dos_1980() {
    let entries = vec![e("sämple ✓.wav", b"abc"), e("b/c", &b"zz".repeat(50))];
    let zip = write(&entries).unwrap();
    assert_eq!(zip, write(&entries).unwrap());
    assert_eq!(&zip[..4], b"PK\x03\x04");
    // Local header: flags 0x0800 (UTF-8), time 00:00:00, date 1980-01-01.
    assert_eq!(&zip[6..8], &[0x00, 0x08]);
    assert_eq!(&zip[10..14], &[0, 0, 0x21, 0]);
    let cd = cd_at(&zip);
    assert_eq!(&zip[cd + 8..cd + 10], &[0x00, 0x08]);
    assert_eq!(&zip[cd + 12..cd + 16], &[0, 0, 0x21, 0]);
    assert_eq!(read(&zip, BIG).unwrap()[0].path, "sämple ✓.wav");
}

#[test]
fn the_writer_says_unix_with_a_regular_file_mode() {
    // Info-ZIP `unzip` decodes a name without the "made on Unix" host as CP437 when the host is DOS and turns
    // `ünï` into `├╝n├п`; with host 3 it keeps UTF-8. Version made by = 20 (2.0), host 3; mode 100644 in the
    // high half of the external attributes.
    let zip = write(&[e("ünï.wav", b"abc"), e("b", &b"zz".repeat(50))]).unwrap();
    let dirs: Vec<usize> = zip
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"PK\x01\x02")
        .map(|(at, _)| at)
        .collect();
    assert_eq!(dirs.len(), 2);
    for cd in dirs {
        assert_eq!(&zip[cd + 4..cd + 6], &[20, 3]);
        assert_eq!(&zip[cd + 38..cd + 42], &(0o100644u32 << 16).to_le_bytes());
    }
    assert_eq!(read(&zip, BIG).unwrap()[0].path, "ünï.wav");
}

#[test]
fn an_end_record_that_claims_fewer_entries_is_corrupt() {
    // The directory holds three records; an end record that counts fewer must not silently drop the rest.
    let zip = build(
        &[stored("a", b"1"), stored("b", b"2"), stored("c", b"3")],
        b"",
    );
    assert_eq!(read(&zip, BIG).unwrap().len(), 3);
    let eocd = eocd_at(&zip);
    for claimed in [0u8, 1, 2] {
        let short = patch(zip.clone(), eocd + 8, &[claimed, 0, claimed, 0]);
        assert_eq!(read(&short, BIG), Err(ZipError::Corrupt), "{claimed}");
    }
}

#[test]
fn entries_are_returned_without_spare_capacity() {
    // 100 000 equal bytes deflate to ~100 B, so the output buffer grows by doubling past the data's size.
    let zip = write(&[
        e("deflated", &vec![7u8; 100_000]),
        e("stored", &noise(3000, 9)),
        e("empty", b""),
    ])
    .unwrap();
    for file in read(&zip, BIG).unwrap() {
        assert_eq!(file.data.capacity(), file.data.len(), "{}", file.path);
    }
}

#[test]
fn empty_archive() {
    let zip = write(&[]).unwrap();
    assert_eq!(zip.len(), 22);
    assert_eq!(read(&zip, BIG).unwrap(), []);
    assert_eq!(read(&zip, 0).unwrap(), []);
}

#[test]
fn reads_an_info_zip_archive() {
    // `zip -X -r` of a folder: a stored text, a directory record, a stored binary and a deflated text; Unix modes
    // in the external attributes.
    let got = read(&INFO_ZIP, BIG).unwrap();
    let rep = "abc".repeat(40) + "\n";
    assert_eq!(
        got,
        [
            entry("hello.txt", b"hello world\n"),
            entry(
                "sub/rand.bin",
                &(0..40u32).map(|i| (i * 167 + 13) as u8).collect::<Vec<_>>()
            ),
            entry("sub/rep.txt", rep.as_bytes()),
        ]
    );
}

#[test]
fn reads_a_streamed_info_zip_archive() {
    // `cat f | zip -X - -`: the local header holds 0xFFFFFFFF sizes and a zip64 extra field, and the archive ends in a
    // zip64 end record and locator, but the central directory and the classic end record carry the real values; the
    // entry's Unix mode is a FIFO (it came from a pipe).
    assert_eq!(
        read(&INFO_ZIP_STREAM, BIG).unwrap(),
        [entry("-", b"hello world\n")]
    );
}

#[test]
fn reads_data_descriptors_extra_fields_and_an_archive_comment() {
    // Bit 3: the local header's crc and sizes are zero and a descriptor follows the data; the central directory is the
    // truth. An extra field in the local header moves the data. The comment holds a fake end record signature.
    let first = Raw {
        flags: 0x0808,
        descriptor: true,
        extra: vec![0xAA, 0xBB, 0, 0],
        ..deflated("a.txt", b"abc")
    };
    assert_eq!(first.packed, [0x4b, 0x4c, 0x4a, 0x06, 0x00]);
    let comment = [b"made by hand, PK\x05\x06".as_slice(), &[b'x'; 40]].concat();
    let zip = build(&[first, stored("b.txt", b"bee")], &comment);
    assert_eq!(
        read(&zip, BIG).unwrap(),
        [entry("a.txt", b"abc"), entry("b.txt", b"bee")]
    );
}

#[test]
fn names_are_normalised() {
    for (raw, want) in [
        ("a\\b\\c.wav", "a/b/c.wav"),
        ("./a.wav", "a.wav"),
        ("a//b.wav", "a/b.wav"),
        ("a/./b.wav", "a/b.wav"),
        ("trail. ", "trail. "),
    ] {
        let zip = build(&[stored(raw, b"x")], b"");
        assert_eq!(read(&zip, BIG).unwrap(), [entry(want, b"x")], "{raw}");
    }
}

#[test]
fn zip_slip_names_are_rejected() {
    for name in [
        "../evil",
        "a/../../evil",
        "a/..",
        "..",
        "/etc/passwd",
        "//server/share/x",
        "\\abs",
        "\\\\server\\share",
        "..\\evil",
        "a\\..\\..\\evil",
        "C:\\evil",
        "c:evil",
        "a/D:/evil",
        "a/b\0c",
        "",
        "...",
        "a/. ./b",
        "a/ /b",
        "..../x",
    ] {
        let zip = build(&[stored("ok.txt", b"1"), stored(name, b"x")], b"");
        assert_eq!(read(&zip, BIG), Err(ZipError::Corrupt), "{name:?}");
    }
    // A name past 4096 bytes is refused too.
    let zip = build(&[stored(&"a/".repeat(2049), b"x")], b"");
    assert_eq!(read(&zip, BIG), Err(ZipError::Corrupt));
}

#[test]
fn directories_are_skipped_but_their_names_are_checked() {
    let dir = |name: &str| Raw {
        flags: 0x0800,
        ..stored(name, b"")
    };
    let zip = build(
        &[dir("a/"), stored("a/x", b"1"), dir("./"), dir("b\\")],
        b"",
    );
    assert_eq!(read(&zip, BIG).unwrap(), [entry("a/x", b"1")]);
    for bad in ["../x/", "/abs/", "a/../../x/"] {
        assert_eq!(
            read(&build(&[dir(bad)], b""), BIG),
            Err(ZipError::Corrupt),
            "{bad}"
        );
    }
}

#[test]
fn symlinks_and_special_files_are_rejected() {
    let unix = |mode: u32, name: &str| Raw {
        made_by: 0x031e,
        attr: mode << 16,
        ..stored(name, b"target")
    };
    for mode in [0o120777, 0o120000, 0o020644, 0o060644, 0o140644] {
        let zip = build(&[stored("ok", b"1"), unix(mode, "link")], b"");
        assert_eq!(read(&zip, BIG), Err(ZipError::Corrupt), "{mode:o}");
    }
    // A FIFO is what Info-ZIP records for a file it read from a pipe (the streamed fixture below).
    for mode in [0o100644, 0o100755, 0o040755, 0o010600, 0] {
        let zip = build(&[unix(mode, "file")], b"");
        assert!(read(&zip, BIG).is_ok(), "{mode:o}");
    }
    // The mode only means something when the archive was made on Unix (host 3).
    let dos = Raw {
        attr: 0o120777 << 16,
        ..stored("odd", b"x")
    };
    assert!(read(&build(&[dos], b""), BIG).is_ok());
}

/// 40 MiB of zeros packed into about 40 KiB.
fn bomb() -> Raw {
    static BOMB: std::sync::OnceLock<Raw> = std::sync::OnceLock::new();
    BOMB.get_or_init(|| {
        let zeros = vec![0u8; 40 << 20];
        let raw = Raw {
            method: 8,
            packed: compress_to_vec(&zeros, 6),
            ..stored("bomb", b"")
        };
        assert!(raw.packed.len() < 50_000);
        Raw {
            crc: crc32fast::hash(&zeros),
            size: zeros.len() as u32,
            ..raw
        }
    })
    .clone()
}

#[test]
fn an_inflate_bomb_hits_the_budget() {
    let zip = build(&[bomb()], b"");
    for max in [0, 1 << 20, (40 << 20) - 1] {
        assert_eq!(read(&zip, max), Err(ZipError::Corrupt), "{max}");
    }
}

#[test]
fn the_ratio_cap_stops_a_bomb_the_budget_would_allow() {
    // 16 MiB + 250 x ~40 KB is less than 40 MiB.
    let zip = build(&[bomb()], b"");
    assert!(40 << 20 > RATIO_ALLOWANCE + MAX_RATIO * zip.len() as u64);
    assert_eq!(read(&zip, BIG), Err(ZipError::Corrupt));
    // The same ratio inside the allowance is fine: 8 MiB of silence.
    let silence = deflated("silence.wav", &vec![0u8; 8 << 20]);
    let small = build(&[silence], b"");
    assert_eq!(read(&small, BIG).unwrap()[0].data.len(), 8 << 20);
}

#[test]
fn a_wrong_declared_size_is_refused() {
    let data = vec![7u8; 100_000];
    for declared in [0u32, 1, 99_999, 100_001, 1 << 30, u32::MAX - 1] {
        let raw = Raw {
            size: declared,
            ..deflated("x", &data)
        };
        assert_eq!(
            read(&build(&[raw], b""), BIG),
            Err(ZipError::Corrupt),
            "{declared}"
        );
    }
    let raw = Raw {
        size: 99,
        ..stored("s", &data)
    };
    assert_eq!(read(&build(&[raw], b""), BIG), Err(ZipError::Corrupt));
}

#[test]
fn the_budget_counts_every_entry_and_is_inclusive() {
    let zip = build(
        &[
            stored("a", &noise(512 << 10, 1)),
            stored("b", &noise(512 << 10, 2)),
        ],
        b"",
    );
    assert!(read(&zip, 1 << 20).is_ok());
    assert_eq!(read(&zip, (1 << 20) - 1), Err(ZipError::Corrupt));
    assert_eq!(read(&zip, 512 << 10), Err(ZipError::Corrupt));
}

#[test]
fn the_first_of_duplicate_names_wins_and_a_later_one_is_never_read() {
    let mut bad = deflated("dup", b"third");
    bad.packed = vec![0xFF; 3];
    bad.crc ^= 1;
    let zip = build(
        &[
            stored("dup", b"first"),
            stored("dup", b"second"),
            bad,
            stored("other", b"o"),
        ],
        b"",
    );
    assert_eq!(
        read(&zip, BIG).unwrap(),
        [entry("dup", b"first"), entry("other", b"o")]
    );
    // Duplicates spelled differently (`\`, `./`) are duplicates too, and they spend no budget.
    let zip = build(
        &[
            stored("a/b", b"1"),
            stored("a\\b", &noise(4096, 3)),
            stored("./a/b", b"3"),
        ],
        b"",
    );
    assert_eq!(read(&zip, 1).unwrap(), [entry("a/b", b"1")]);
}

#[test]
fn a_crc_mismatch_is_corrupt() {
    for raw in [stored("a", b"data"), deflated("a", &b"data".repeat(30))] {
        let zip = build(
            &[Raw {
                crc: raw.crc ^ 0x8000_0000,
                ..raw
            }],
            b"",
        );
        assert_eq!(read(&zip, BIG), Err(ZipError::Corrupt));
    }
    // And a flipped payload byte of a stored entry.
    let zip = build(&[stored("a", b"data")], b"");
    assert_eq!(read(&patch(zip, 30 + 1, b"X"), BIG), Err(ZipError::Corrupt));
}

#[test]
fn encrypted_zip64_and_other_methods_are_refused() {
    let base = stored("a", b"data");
    let with = |f: &dyn Fn(Raw) -> Raw| build(&[f(base.clone())], b"");
    for flags in [0x0801, 0x0840, 0x2800, 0x0841] {
        assert_eq!(
            read(&with(&|r| Raw { flags, ..r }), BIG),
            Err(ZipError::Corrupt),
            "{flags:x}"
        );
    }
    // Shrink, Deflate64, bzip2, LZMA, AES-encrypted (99): never read as deflate, even when the bytes are a deflate
    // stream of the right data.
    for method in [1, 6, 9, 12, 14, 93, 98, 99] {
        let zip = build(
            &[Raw {
                method,
                ..deflated("a", &b"data".repeat(20))
            }],
            b"",
        );
        assert_eq!(read(&zip, BIG), Err(ZipError::Corrupt), "{method}");
        assert_eq!(
            read(&with(&|r| Raw { method, ..r }), BIG),
            Err(ZipError::Corrupt),
            "{method}"
        );
    }
    let zip = with(&|r| r);
    let (cd, eocd) = (cd_at(&zip), eocd_at(&zip));
    for (at, len) in [
        (cd + 20, 4),
        (cd + 24, 4),
        (cd + 42, 4),
        (eocd + 8, 2),
        (eocd + 10, 2),
        (eocd + 12, 4),
        (eocd + 16, 4),
    ] {
        let zip64 = patch(zip.clone(), at, &[0xFF; 4][..len]);
        assert_eq!(read(&zip64, BIG), Err(ZipError::Corrupt), "{at}");
    }
}

#[test]
fn archives_over_the_entry_cap_are_refused() {
    let files = |n: usize| (0..n).map(|i| e(&format!("f{i}"), b"")).collect::<Vec<_>>();
    assert_eq!(
        read(&write(&files(MAX_ENTRIES)).unwrap(), BIG)
            .unwrap()
            .len(),
        MAX_ENTRIES
    );
    assert_eq!(
        read(&write(&files(MAX_ENTRIES + 1)).unwrap(), BIG),
        Err(ZipError::Corrupt)
    );
}

#[test]
fn entries_may_not_overlap_each_other_or_the_directory() {
    // Two central records for one local entry, the shape of an overlapping-file bomb.
    let zip = build(&[stored("a", b"data")], b"");
    let (cd, eocd) = (cd_at(&zip), eocd_at(&zip));
    let mut twin = zip[..eocd].to_vec();
    twin.extend_from_slice(&zip[cd..eocd]);
    twin.extend_from_slice(&zip[eocd..]);
    let twin_eocd = eocd_at(&twin);
    let twin = patch(twin, twin_eocd + 8, &[2, 0, 2, 0]);
    let size = ((eocd - cd) * 2) as u32;
    let twin = patch(twin, twin_eocd + 12, &size.to_le_bytes());
    // The second record is a duplicate name, so it would be skipped; rename it and the overlap is what refuses it.
    let renamed = patch(twin.clone(), cd + (eocd - cd) + 46, b"b");
    assert_eq!(read(&renamed, BIG), Err(ZipError::Corrupt));
    assert_eq!(read(&twin, BIG), Err(ZipError::Corrupt));

    // An entry whose data runs into the central directory, with the CRC of what it then covers.
    let blob = &zip[31..31 + 14];
    let long = patch(zip.clone(), cd + 16, &crc32fast::hash(blob).to_le_bytes());
    let long = patch(
        patch(long, cd + 20, &14u32.to_le_bytes()),
        cd + 24,
        &14u32.to_le_bytes(),
    );
    assert_eq!(read(&long, BIG), Err(ZipError::Corrupt));
    // A local offset past the end, or into the middle of nowhere.
    for off in [zip.len() as u32, u32::MAX - 1, 5, 1] {
        assert_eq!(
            read(&patch(zip.clone(), cd + 42, &off.to_le_bytes()), BIG),
            Err(ZipError::Corrupt),
            "{off}"
        );
    }
}

#[test]
fn what_is_not_a_zip_is_not_a_zip() {
    for bytes in [
        &b""[..],
        b"PK",
        b"PK\x05\x06",
        &[0u8; 21],
        &[0u8; 4096],
        b"10LF\x00\x00\x00\x00 an FLM-looking file, not a zip at all, long enough to hold an end record",
        b"MThd\x00\x00\x00\x06\x00\x00\x00\x01\x00\x80",
    ] {
        assert_eq!(read(bytes, BIG), Err(ZipError::NotZip), "{bytes:?}");
    }
    // An end record whose comment would run past the end of the file is not an end record.
    let zip = build(&[stored("a", b"1")], b"");
    let eocd = eocd_at(&zip);
    assert_eq!(
        read(&patch(zip.clone(), eocd + 20, &[1, 0]), BIG),
        Err(ZipError::NotZip)
    );
    // The end record is looked for in the last 64 KiB + 22 bytes only.
    let mut far = zip.clone();
    far.extend(std::iter::repeat_n(b'x', 70_000));
    assert_eq!(read(&far, BIG), Err(ZipError::NotZip));
    let mut near = build(&[stored("a", b"1")], &vec![b'c'; 65_535]);
    assert!(read(&near, BIG).is_ok());
    near.push(0);
    assert_eq!(read(&near, BIG), Err(ZipError::NotZip));
}

#[test]
fn a_damaged_directory_is_corrupt() {
    let zip = build(&[stored("a", b"1"), stored("b", b"2")], b"");
    let (cd, eocd) = (cd_at(&zip), eocd_at(&zip));
    let patched = |at: usize, with: &[u8]| patch(zip.clone(), at, with);
    for bad in [
        patched(eocd + 4, &[1, 0]),                 // this disk is not disk 0
        patched(eocd + 6, &[1, 0]),                 // the directory starts on another disk
        patched(eocd + 8, &[1, 0]),                 // entries on this disk != entries
        patched(eocd + 8, &[3, 0, 3, 0]),           // more records than the directory holds
        patched(eocd + 12, &1000u32.to_le_bytes()), // directory size past the file
        patched(eocd + 16, &(zip.len() as u32).to_le_bytes()), // directory offset past the file
        patched(cd, b"PK\x03\x04"),                 // not a central record
        patched(cd + 28, &[200, 0]),                // a name longer than the directory
        patched(cd + 34, &[1, 0]),                  // the entry starts on another disk
        patched(cd + 32, &[0xFF, 0xFF]),            // a comment longer than the directory
        patched(cd + 47, b"PK\x03\x04"),            // the second record is not one
    ] {
        assert_eq!(read(&bad, BIG), Err(ZipError::Corrupt));
    }
    // A local header that is not one.
    assert_eq!(
        read(&patched(0, b"PK\x01\x02"), BIG),
        Err(ZipError::Corrupt)
    );
}

#[test]
fn names_without_the_utf8_flag_are_read_as_utf8_or_latin_1() {
    let plain = |name: &[u8]| Raw {
        flags: 0,
        name: name.to_vec(),
        ..stored("x", b"1")
    };
    let got = read(&build(&[plain("é.wav".as_bytes())], b""), BIG).unwrap();
    assert_eq!(got[0].path, "é.wav");
    let got = read(&build(&[plain(&[b'a', 0x82, b'.', b'w'])], b""), BIG).unwrap();
    assert_eq!(got[0].path, "a\u{82}.w");
    // With the flag, the name must be UTF-8.
    let flagged = Raw {
        name: vec![b'a', 0x82],
        ..stored("x", b"1")
    };
    assert_eq!(read(&build(&[flagged], b""), BIG), Err(ZipError::Corrupt));
}

#[test]
fn the_writer_refuses_names_the_reader_would() {
    for name in [
        "",
        "../x",
        "/x",
        "a\\b",
        "C:x",
        "a//b",
        "./a",
        "dir/",
        "a/\0",
        &"n".repeat(5000),
    ] {
        assert_eq!(
            write(&[e(name, b"x")]),
            Err(WriteError::BadName),
            "{name:?}"
        );
    }
    assert!(write(&[e("a b/ü.wav", b"x")]).is_ok());
}

#[test]
fn the_writer_refuses_what_zip64_would_hold() {
    let files = (0..0xFFFF).map(|_| e("a", b"")).collect::<Vec<_>>();
    assert_eq!(write(&files), Err(WriteError::TooLarge));
    assert!(write(&files[..0xFFFE]).is_ok());
}

#[test]
fn the_writer_refuses_what_does_not_fit_four_gibibytes() {
    // `write` limits the archive to u32::MAX bytes; the same check with a small limit, so no test holds gigabytes.
    let files = [e("a", &noise(100, 1)), e("b", &noise(100, 2))];
    let size = write(&files).unwrap().len() as u64;
    assert_eq!(write_limited(&files, size).unwrap().len() as u64, size);
    assert_eq!(write_limited(&files, size - 1), Err(WriteError::TooLarge));
    assert_eq!(write_limited(&files[..1], 120), Err(WriteError::TooLarge));
    assert_eq!(write_limited(&[], 21), Err(WriteError::TooLarge));
    assert!(write_limited(&[], 22).is_ok());
}

/// Archives that exercise every branch: the writer's, and the hand-built and real ones.
fn corpus() -> Vec<Vec<u8>> {
    vec![
        write(&[
            e("a.txt", b"hello"),
            e("d/b.bin", &noise(300, 5)),
            e("c", &b"ab".repeat(100)),
        ])
        .unwrap(),
        INFO_ZIP.to_vec(),
        INFO_ZIP_STREAM.to_vec(),
        build(
            &[
                deflated("x", b"abcabcabc"),
                stored("y\\z", b"1"),
                stored("d/", b""),
            ],
            b"comment",
        ),
    ]
}

#[test]
fn truncation_at_every_byte_never_panics() {
    for zip in corpus() {
        assert!(read(&zip, BIG).is_ok());
        for len in 0..zip.len() {
            assert!(read(&zip[..len], BIG).is_err(), "{len} of {}", zip.len());
        }
        // And with the front cut off.
        for from in 1..zip.len() {
            let _ = read(&zip[from..], BIG);
        }
    }
}

#[test]
fn flipping_any_bit_never_panics() {
    for zip in corpus() {
        for at in 0..zip.len() {
            for bit in 0..8 {
                let mut bad = zip.clone();
                bad[at] ^= 1 << bit;
                if let Ok(entries) = read(&bad, 1 << 20) {
                    assert!(entries.iter().all(|e| e.path.split('/').all(|c| c != "..")));
                }
            }
        }
    }
}

#[test]
fn errors_have_messages() {
    assert_eq!(ZipError::NotZip.to_string(), "not a ZIP archive");
    assert_eq!(
        ZipError::Corrupt.to_string(),
        "the ZIP archive is damaged or unsupported"
    );
    assert_eq!(
        WriteError::TooLarge.to_string(),
        "the archive would be too large for a ZIP without zip64"
    );
    assert_eq!(
        WriteError::BadName.to_string(),
        "an entry name is not a safe relative path"
    );
}

/// `name` of `zip` through [`open_entry`], `chunk` bytes per read; `Ok(None)` when there is no such file.
fn streamed(zip: &[u8], name: &str, chunk: usize) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read;
    let opened = open_entry(std::io::Cursor::new(zip.to_vec()), name);
    let Some((mut data, size)) = opened.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let (mut out, mut buf) = (Vec::new(), vec![0; chunk]);
    loop {
        match data.read(&mut buf).map_err(|e| e.to_string())? {
            0 => break,
            n => out.extend_from_slice(&buf[..n]),
        }
    }
    assert_eq!(
        out.len() as u64,
        size,
        "a stream that ends ends at its size"
    );
    Ok(Some(out))
}

#[test]
fn one_entry_streams_as_read_reads_it() {
    for zip in corpus() {
        for e in read(&zip, BIG).unwrap() {
            for chunk in [1, 7, 1 << 16] {
                let got = streamed(&zip, &e.path, chunk);
                assert_eq!(got, Ok(Some(e.data.clone())), "{}", e.path);
            }
        }
        assert_eq!(streamed(&zip, "missing", 64), Ok(None));
    }
    // An APK's OBB: one large deflated entry among others, inflated a buffer at a time.
    let obb = b"the quick brown fox ".repeat(50_000);
    let apk = write(&[
        e("classes.dex", &noise(500, 3)),
        e("assets/obb/assets.obb", &obb),
    ])
    .unwrap();
    assert!(apk.len() < obb.len() / 10, "deflated");
    assert_eq!(streamed(&apk, "assets/obb/assets.obb", 4096), Ok(Some(obb)));
}

#[test]
fn a_damaged_streamed_entry_fails_and_never_panics() {
    let text = b"hello hello hello hello ".repeat(40);
    let zip = write(&[e("a", &text), e("b", &noise(200, 9))]).unwrap();
    // The CRC is checked once the entry is read to its end.
    let bad_crc = patch(zip.clone(), cd_at(&zip) + 16, &[0; 4]);
    assert!(streamed(&bad_crc, "a", 64).is_err());
    // A declared size off by one either way.
    let size = u32::try_from(text.len()).unwrap();
    for wrong in [size - 1, size + 1] {
        let bad = patch(zip.clone(), cd_at(&zip) + 24, &wrong.to_le_bytes());
        assert!(streamed(&bad, "a", 64).is_err(), "{wrong}");
    }
    // A stored entry whose sizes disagree, and other methods.
    let b_at = find(&zip[cd_at(&zip) + 4..], b"PK\x01\x02") + cd_at(&zip) + 4;
    assert!(streamed(&patch(zip.clone(), b_at + 24, &[1, 0, 0, 0]), "b", 64).is_err());
    assert!(streamed(&patch(zip.clone(), b_at + 10, &[12, 0]), "b", 64).is_err());
    for len in 0..zip.len() {
        assert!(streamed(&zip[..len], "a", 64).is_err(), "{len}");
    }
    for at in 0..zip.len() {
        for bit in 0..8 {
            let mut bad = zip.clone();
            bad[at] ^= 1 << bit;
            let _ = streamed(&bad, "a", 64);
            let _ = streamed(&bad, "b", 64);
        }
    }
}

/// Whether [`open_entry`] finds `name` in `zip`, or why it refuses at once.
fn opens(zip: &[u8], name: &str) -> Result<bool, ZipError> {
    open_entry(std::io::Cursor::new(zip.to_vec()), name).map(|o| o.is_some())
}

#[test]
fn a_streamed_entry_is_refused_at_open_as_read_refuses_it() {
    let base = stored("a", b"data");
    let with = |f: &dyn Fn(Raw) -> Raw| build(&[stored("b", b"x"), f(base.clone())], b"");
    assert_eq!(opens(&with(&|r| r), "a"), Ok(true));
    for flags in [0x0801, 0x0840, 0x2800] {
        assert_eq!(
            opens(&with(&|r| Raw { flags, ..r }), "a"),
            Err(ZipError::Corrupt),
            "{flags:x}"
        );
    }
    let link = |r| Raw {
        made_by: 0x031e,
        attr: 0o120777 << 16,
        ..r
    };
    assert_eq!(opens(&with(&link), "a"), Err(ZipError::Corrupt), "symlink");
    let zip = with(&|r| r);
    let a_cd = cd_at(&zip) + find(&zip[cd_at(&zip) + 4..], b"PK\x01\x02") + 4;
    let a_local = find(&zip[4..], b"PK\x03\x04") + 4;
    assert_eq!(
        opens(&patch(zip.clone(), a_cd + 34, &[1, 0]), "a"),
        Err(ZipError::Corrupt),
        "disk"
    );
    assert_eq!(
        opens(&patch(zip.clone(), a_local, b"PK\x03\x05"), "a"),
        Err(ZipError::Corrupt),
        "local"
    );
    // Data that runs into the directory (sizes of 5, 4 fit), a directory that runs into the end record, and a stored
    // size that differs from the packed one.
    let (four, five) = (4u32.to_le_bytes(), 5u32.to_le_bytes());
    let end = eocd_at(&zip);
    let dir_len = u32::from_le_bytes(zip[end + 12..end + 16].try_into().unwrap());
    for (at, with, what) in [
        (a_cd + 20, &[five, five].concat()[..], "into the directory"),
        (end + 12, &(dir_len + 1).to_le_bytes()[..], "into the end"),
    ] {
        assert_eq!(
            opens(&patch(zip.clone(), at, with), "a"),
            Err(ZipError::Corrupt),
            "{what}"
        );
    }
    assert_eq!(
        opens(&patch(zip.clone(), a_cd + 20, &[four, four].concat()), "a"),
        Ok(true)
    );
    let b_packed = patch(zip.clone(), cd_at(&zip) + 20, &2u32.to_le_bytes());
    assert_eq!(
        opens(&b_packed, "b"),
        Err(ZipError::Corrupt),
        "stored sizes"
    );
    // The ratio cap: 40 MiB declared in about 40 KiB.
    assert_eq!(
        opens(&build(&[bomb()], b""), "bomb"),
        Err(ZipError::Corrupt),
        "ratio"
    );
    // A directory over its cap, of records with 64 KiB comments before the one looked for; under it, it is read.
    let zip = build(&[base], b"");
    let (cd, end) = (cd_at(&zip), eocd_at(&zip));
    let mut filler = patch(zip[cd..end].to_vec(), 32, &[0xFF, 0xFF]);
    filler[DIR_LEN] = b'f';
    filler.extend(vec![0; 0xFFFF]);
    for (fillers, ok) in [(250, true), (270, false)] {
        let dir = [filler.repeat(fillers), zip[cd..end].to_vec()].concat();
        let n = u16::try_from(fillers + 1).unwrap().to_le_bytes();
        let mut big = [&zip[..cd], &dir[..]].concat();
        put(&mut big, &[b"PK\x05\x06", &[0; 4], &n, &n]);
        put(
            &mut big,
            &[
                &(dir.len() as u32).to_le_bytes(),
                &(cd as u32).to_le_bytes(),
                &[0, 0],
            ],
        );
        assert_eq!(dir.len() <= MAX_STREAMED_DIR as usize, ok);
        let want = if ok { Ok(true) } else { Err(ZipError::Corrupt) };
        assert_eq!(opens(&big, "a"), want, "{fillers}");
    }
}

#[test]
fn a_streamed_entry_ends_at_its_size_and_an_empty_one_has_its_crc_checked() {
    let data = b"hello hello hello hello ".repeat(40);
    // A deflate stream with more than the declared size, and a CRC that matches what is declared.
    let short = Raw {
        size: 100,
        crc: crc32fast::hash(&data[..100]),
        ..deflated("a", &data)
    };
    assert!(streamed(&build(&[short], b""), "a", 64).is_err());
    // A deflate stream that never ends: one stored block, not marked final; marked final, it reads.
    let block = |last: u8| {
        let len = (data.len() as u16).to_le_bytes();
        [
            &[last],
            &len[..],
            &(!u16::from_le_bytes(len)).to_le_bytes(),
            &data,
        ]
        .concat()
    };
    for (last, ok) in [(0, false), (1, true)] {
        let zip = build(
            &[Raw {
                method: 8,
                packed: block(last),
                ..stored("a", &data)
            }],
            b"",
        );
        assert_eq!(streamed(&zip, "a", 64).is_ok(), ok, "{last}");
    }
    let empty = |crc| {
        build(
            &[Raw {
                crc,
                ..stored("a", b"")
            }],
            b"",
        )
    };
    assert_eq!(streamed(&empty(0), "a", 64), Ok(Some(Vec::new())));
    assert!(streamed(&empty(1), "a", 64).is_err());
}

const INFO_ZIP: [u8; 459] = [
    0x50, 0x4b, 0x03, 0x04, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x73, 0x24, 0x4c, 0x5d, 0x2d, 0x3b,
    0x08, 0xaf, 0x0c, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x68, 0x65,
    0x6c, 0x6c, 0x6f, 0x2e, 0x74, 0x78, 0x74, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x20, 0x77, 0x6f, 0x72,
    0x6c, 0x64, 0x0a, 0x50, 0x4b, 0x03, 0x04, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x73, 0x24, 0x4c,
    0x5d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00,
    0x00, 0x73, 0x75, 0x62, 0x2f, 0x50, 0x4b, 0x03, 0x04, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x77,
    0x24, 0x4c, 0x5d, 0x3f, 0xe5, 0xcf, 0x8e, 0x28, 0x00, 0x00, 0x00, 0x28, 0x00, 0x00, 0x00, 0x0c,
    0x00, 0x00, 0x00, 0x73, 0x75, 0x62, 0x2f, 0x72, 0x61, 0x6e, 0x64, 0x2e, 0x62, 0x69, 0x6e, 0x0d,
    0xb4, 0x5b, 0x02, 0xa9, 0x50, 0xf7, 0x9e, 0x45, 0xec, 0x93, 0x3a, 0xe1, 0x88, 0x2f, 0xd6, 0x7d,
    0x24, 0xcb, 0x72, 0x19, 0xc0, 0x67, 0x0e, 0xb5, 0x5c, 0x03, 0xaa, 0x51, 0xf8, 0x9f, 0x46, 0xed,
    0x94, 0x3b, 0xe2, 0x89, 0x30, 0xd7, 0x7e, 0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x00, 0x00, 0x08,
    0x00, 0x73, 0x24, 0x4c, 0x5d, 0x4f, 0xc9, 0x45, 0xdc, 0x09, 0x00, 0x00, 0x00, 0x79, 0x00, 0x00,
    0x00, 0x0b, 0x00, 0x00, 0x00, 0x73, 0x75, 0x62, 0x2f, 0x72, 0x65, 0x70, 0x2e, 0x74, 0x78, 0x74,
    0x4b, 0x4c, 0x4a, 0x4e, 0x1c, 0x08, 0xc4, 0x05, 0x00, 0x50, 0x4b, 0x01, 0x02, 0x1e, 0x03, 0x0a,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x73, 0x24, 0x4c, 0x5d, 0x2d, 0x3b, 0x08, 0xaf, 0x0c, 0x00, 0x00,
    0x00, 0x0c, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
    0x00, 0xa4, 0x81, 0x00, 0x00, 0x00, 0x00, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x2e, 0x74, 0x78, 0x74,
    0x50, 0x4b, 0x01, 0x02, 0x1e, 0x03, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x73, 0x24, 0x4c, 0x5d,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0xed, 0x41, 0x33, 0x00, 0x00, 0x00, 0x73, 0x75,
    0x62, 0x2f, 0x50, 0x4b, 0x01, 0x02, 0x1e, 0x03, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x77, 0x24,
    0x4c, 0x5d, 0x3f, 0xe5, 0xcf, 0x8e, 0x28, 0x00, 0x00, 0x00, 0x28, 0x00, 0x00, 0x00, 0x0c, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa4, 0x81, 0x55, 0x00, 0x00, 0x00,
    0x73, 0x75, 0x62, 0x2f, 0x72, 0x61, 0x6e, 0x64, 0x2e, 0x62, 0x69, 0x6e, 0x50, 0x4b, 0x01, 0x02,
    0x1e, 0x03, 0x14, 0x00, 0x00, 0x00, 0x08, 0x00, 0x73, 0x24, 0x4c, 0x5d, 0x4f, 0xc9, 0x45, 0xdc,
    0x09, 0x00, 0x00, 0x00, 0x79, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0xa4, 0x81, 0xa7, 0x00, 0x00, 0x00, 0x73, 0x75, 0x62, 0x2f, 0x72, 0x65,
    0x70, 0x2e, 0x74, 0x78, 0x74, 0x50, 0x4b, 0x05, 0x06, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x04,
    0x00, 0xdc, 0x00, 0x00, 0x00, 0xd9, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const INFO_ZIP_STREAM: [u8; 208] = [
    0x50, 0x4b, 0x03, 0x04, 0x2d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x77, 0x24, 0x4c, 0x5d, 0x2d, 0x3b,
    0x08, 0xaf, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01, 0x00, 0x14, 0x00, 0x2d, 0x01,
    0x00, 0x10, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x20, 0x77, 0x6f, 0x72, 0x6c, 0x64, 0x0a, 0x50,
    0x4b, 0x01, 0x02, 0x1e, 0x03, 0x2d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x77, 0x24, 0x4c, 0x5d, 0x2d,
    0x3b, 0x08, 0xaf, 0x0c, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x80, 0x11, 0x00, 0x00, 0x00, 0x00, 0x2d, 0x50, 0x4b,
    0x06, 0x06, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1e, 0x03, 0x2d, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3f, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x50, 0x4b, 0x06, 0x07, 0x00, 0x00, 0x00, 0x00, 0x6e, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x50, 0x4b, 0x05, 0x06, 0x00, 0x00,
    0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x2f, 0x00, 0x00, 0x00, 0x3f, 0x00, 0x00, 0x00, 0x00, 0x00,
];

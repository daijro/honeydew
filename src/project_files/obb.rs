//! The imported content container, `assets.obb`,
//! which the importer ([`super::import`]) unpacks from the user's own copy; no such file ever enters the repo or
//! its tests. All big-endian: a 28-byte header (u32 version 3, a 16-byte hash, a u32 nobody reads, the entry count),
//! then per entry a u32 name length (its NUL included), the NUL-padded `/`-separated relative name, a u32 absolute
//! offset, a u32 size and u16 flags (0: stored raw, the only kind seen), then the data.
//!
//! The file is hostile until read. [`read`] streams it front to back, never whole in memory (a full one is 358 MB):
//! the directory first, checked whole before any data is handed out, then each entry's data in offset order. It
//! refuses, as an error and never a panic: a header that cannot be (over [`MAX_ENTRIES`], or more entries than the
//! file can hold), a directory over [`MAX_DIR`] bytes, a name that is empty, over [`MAX_NAME`] bytes, not UTF-8 or
//! able to leave the folder (the project archives' policy, [`crate::zip::clean`]: `\` reads as `/`; no root, drive
//! or `..`), data outside the file or inside the directory, and entries that overlap (the overlapping-file bomb:
//! every byte is handed out at most once, so what is unpacked is never larger than the file). A version other than 3
//! is only logged; an entry with nonzero flags (a packing no known copy uses) and a folder entry (a name ending in
//! `/`, which unpacks to nothing) are logged and skipped.

use std::io::{self, Read};

/// More entries are refused (a full container has 5571).
pub const MAX_ENTRIES: u32 = 1 << 16;
/// Longer names are refused (no file system takes one).
pub const MAX_NAME: u32 = 4096;
/// A longer directory is refused (the zip reader's cap for an APK's directory, too).
pub const MAX_DIR: u64 = 16 << 20;
const HEADER_LEN: u64 = 28;
/// An entry's fixed fields: name length, offset, size and flags.
const FIELDS_LEN: u64 = 14;

/// A refusal, with what [`super::import`] shows.
fn bad(why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("the content archive is damaged: {why}"),
    )
}

/// A stream that ends early is a damaged container.
fn early(e: io::Error) -> io::Error {
    match e.kind() {
        io::ErrorKind::UnexpectedEof => bad("it ends early"),
        _ => e,
    }
}

fn take<const N: usize>(obb: &mut impl Read) -> io::Result<[u8; N]> {
    let mut out = [0; N];
    obb.read_exact(&mut out).map_err(early)?;
    Ok(out)
}

/// An entry to hand out.
struct Entry {
    name: String,
    offset: u64,
    size: u64,
}

/// Reads the container `obb`, `len` bytes from its first, and hands `each` every entry to unpack, in offset order
/// (empty ones first): its relative name and a reader of its bytes, which `each` reads as far as it likes. A reader
/// that still has a limit once read to its end means the stream ended early: `each` must not keep that file (the read
/// then fails). `each`'s error stops the read and is returned.
pub(crate) fn read<R: Read>(
    mut obb: R,
    len: u64,
    mut each: impl FnMut(&str, &mut io::Take<&mut R>) -> io::Result<()>,
) -> io::Result<()> {
    let head: [u8; HEADER_LEN as usize] = take(&mut obb)?;
    let be32 = |at: usize| u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
    let (version, count) = (be32(0), be32(24));
    // The smallest entry is its fields and a one-byte name.
    let room = len.saturating_sub(HEADER_LEN) / (FIELDS_LEN + 1);
    if count > MAX_ENTRIES {
        return Err(bad("too many entries"));
    }
    if u64::from(count) > room {
        return Err(bad("more entries than the file can hold"));
    }
    if version != 3 {
        eprintln!("assets.obb: version {version}, read as version 3");
    }

    let mut pos = HEADER_LEN;
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let name_len = u32::from_be_bytes(take(&mut obb)?);
        if name_len == 0 || name_len > MAX_NAME {
            return Err(bad("a name is empty or too long"));
        }
        let mut name = vec![0; name_len as usize];
        obb.read_exact(&mut name).map_err(early)?;
        let [o0, o1, o2, o3, s0, s1, s2, s3, f0, f1] = take(&mut obb)?;
        pos += FIELDS_LEN + u64::from(name_len);
        if pos - HEADER_LEN > MAX_DIR {
            return Err(bad("the directory is too long"));
        }
        let name = name.split(|&b| b == 0).next().unwrap_or_default();
        let name = std::str::from_utf8(name).map_err(|_| bad("a name is not UTF-8"))?;
        let Some(name) = crate::zip::clean(name).map_err(|_| bad("a name leaves the folder"))?
        else {
            eprintln!("assets.obb: {name}: a folder, skipped");
            continue;
        };
        let flags = u16::from_be_bytes([f0, f1]);
        if flags != 0 {
            eprintln!("assets.obb: {name}: flags {flags:#x}, skipped");
            continue;
        }
        entries.push(Entry {
            name,
            offset: u32::from_be_bytes([o0, o1, o2, o3]).into(),
            size: u32::from_be_bytes([s0, s1, s2, s3]).into(),
        });
    }

    // The data after the directory, inside the file, side by side. An empty entry's offset says nothing.
    entries.sort_unstable_by_key(|e| (e.size > 0, e.offset, e.size));
    let mut end = pos;
    for e in entries.iter().filter(|e| e.size > 0) {
        if e.offset < end || e.offset + e.size > len {
            return Err(bad(
                "an entry's data overlaps another's or lies outside the file",
            ));
        }
        end = e.offset + e.size;
    }

    for e in &entries {
        if e.size > 0 {
            io::copy(&mut (&mut obb).take(e.offset - pos), &mut io::sink())?;
            pos = e.offset + e.size;
        }
        let mut data = (&mut obb).take(e.size);
        each(&e.name, &mut data).map_err(early)?;
        io::copy(&mut data, &mut io::sink())?;
        if data.limit() > 0 {
            return Err(bad("it ends early"));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::project_files::ProjectFiles;
    use std::path::Path;

    /// A container with every field explicit: per entry the name as stored (its NULs included), the offset, the size
    /// and the flags; then `data`. Every byte is made here.
    fn raw(entries: &[(&[u8], u32, u32, u16)], data: &[u8]) -> Vec<u8> {
        let mut out = [&3u32.to_be_bytes()[..], &[0xA5; 16], &[0; 4]].concat();
        out.extend((entries.len() as u32).to_be_bytes());
        for (name, offset, size, flags) in entries {
            out.extend((name.len() as u32).to_be_bytes());
            out.extend(*name);
            out.extend(offset.to_be_bytes());
            out.extend(size.to_be_bytes());
            out.extend(flags.to_be_bytes());
        }
        out.extend(data);
        out
    }

    /// A container of `entries` as the format lays it out: the directory, then the data in
    /// order.
    pub(crate) fn container(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let names: Vec<Vec<u8>> = entries
            .iter()
            .map(|(n, _)| [n.as_bytes(), &[0]].concat())
            .collect();
        let mut at = 28 + names.iter().map(|n| 14 + n.len()).sum::<usize>();
        let fields: Vec<_> = (names.iter().zip(entries))
            .map(|(name, (_, data))| {
                at += data.len();
                (&name[..], (at - data.len()) as u32, data.len() as u32, 0)
            })
            .collect();
        raw(
            &fields,
            &entries
                .iter()
                .flat_map(|(_, d)| *d)
                .copied()
                .collect::<Vec<_>>(),
        )
    }

    /// What [`read`] hands out of `obb` (its length as `len`), each entry read whole.
    fn unpacked(obb: &[u8], len: u64) -> io::Result<Vec<(String, Vec<u8>)>> {
        let mut got = Vec::new();
        read(obb, len, |name, data| {
            let mut bytes = Vec::new();
            data.read_to_end(&mut bytes)?;
            got.push((name.to_owned(), bytes));
            Ok(())
        })?;
        Ok(got)
    }

    fn whole(obb: &[u8]) -> io::Result<Vec<(String, Vec<u8>)>> {
        unpacked(obb, obb.len() as u64)
    }

    fn owned(entries: &[(&str, &[u8])]) -> Vec<(String, Vec<u8>)> {
        entries
            .iter()
            .map(|(n, d)| ((*n).to_owned(), d.to_vec()))
            .collect()
    }

    const SMALL: [(&str, &[u8]); 4] = [
        ("Drum Samples/Kick/Gen Kick.wav", b"fLaC gen kick"),
        ("empty.txt", b""),
        ("Chord Sets/Gen Minor.chords", b"{}"),
        ("Gen Art.png", b"\x89PNG gen"),
    ];

    /// Where entry 0's name length, offset, size and flags are in a [`container`] whose first name is `name`.
    fn fields0(name: &str) -> (usize, usize, usize, usize) {
        let at = 28 + 4 + name.len() + 1;
        (28, at, at + 4, at + 8)
    }

    fn patch(mut obb: Vec<u8>, at: usize, with: &[u8]) -> Vec<u8> {
        obb[at..at + with.len()].copy_from_slice(with);
        obb
    }

    #[test]
    fn a_container_hands_out_every_entry_at_its_path() {
        let got = whole(&container(&SMALL)).unwrap();
        // Empty entries first, then in offset order.
        let want = owned(&[SMALL[1], SMALL[0], SMALL[2], SMALL[3]]);
        assert_eq!(got, want);
        assert_eq!(whole(&container(&[])).unwrap(), []);
        // `\` reads as `/`, `.` and empty folders drop, and a name may be padded with more NULs.
        let padded = raw(&[(&b"./Gen\\/a.wav\0\0\0"[..], 28 + 14 + 15, 1, 0)], b"x");
        assert_eq!(whole(&padded).unwrap(), owned(&[("Gen/a.wav", b"x")]));
        // As many entries as allowed.
        let most = container(&vec![("a", &b""[..]); MAX_ENTRIES as usize]);
        assert_eq!(whole(&most).unwrap().len(), MAX_ENTRIES as usize);
    }

    #[test]
    fn a_container_cut_short_anywhere_is_refused() {
        let obb = container(&SMALL);
        for len in 0..obb.len() {
            // As the file says it is, and as a stream that ends before the length it was given.
            assert!(whole(&obb[..len]).is_err(), "{len}");
            assert!(
                unpacked(&obb[..len], obb.len() as u64).is_err(),
                "{len} of a longer one"
            );
        }
    }

    #[test]
    fn impossible_headers_and_entries_are_refused_before_anything_is_handed_out() {
        let name = SMALL[0].0;
        let obb = container(&SMALL);
        let (name_len, offset, size, _) = fields0(name);
        let count = |n: u32| patch(obb.clone(), 24, &n.to_be_bytes());
        let (many, room, long, outside) = (
            "too many entries",
            "more entries than the file can hold",
            "a name is empty or too long",
            "an entry's data overlaps another's or lies outside the file",
        );
        // Over the cap in a file with room for them (16 bytes each), and a directory over its cap.
        let over = container(&vec![("a", &b""[..]); MAX_ENTRIES as usize + 1]);
        assert!(over.len() >= 1 << 20);
        let name = "a".repeat(MAX_NAME as usize - 1);
        let entry = [name.as_bytes(), b"\0"].concat();
        let entries = (MAX_DIR / (FIELDS_LEN + u64::from(MAX_NAME)) + 1) as usize;
        let long_dir = raw(&vec![(&entry[..], 0, 0, 0); entries], b"");
        let mut bad = vec![
            (count(u32::MAX), many),
            (over, many),
            (count(1000), room),
            (long_dir, "the directory is too long"),
            (patch(obb.clone(), name_len, &u32::MAX.to_be_bytes()), long),
            (
                patch(obb.clone(), name_len, &(MAX_NAME + 1).to_be_bytes()),
                long,
            ),
            (patch(obb.clone(), name_len, &0u32.to_be_bytes()), long),
            (
                patch(obb.clone(), offset, &(obb.len() as u32).to_be_bytes()),
                outside,
            ),
            (
                patch(obb.clone(), size, &(obb.len() as u32).to_be_bytes()),
                outside,
            ),
            (patch(obb.clone(), offset, &[0xFF; 8]), outside),
            // Inside the header or the directory.
            (patch(obb.clone(), offset, &0u32.to_be_bytes()), outside),
            (patch(obb.clone(), offset, &40u32.to_be_bytes()), outside),
            // Overlapping the next entry's data.
            (
                patch(
                    obb.clone(),
                    size,
                    &(SMALL[0].1.len() as u32 + 1).to_be_bytes(),
                ),
                outside,
            ),
        ];
        for hostile in [
            "../Gen Kick.wav",
            "a/../../Gen.wav",
            "/etc/gen",
            "..\\..\\Gen.wav",
            "C:/Gen.wav",
            "a/. ./Gen.wav",
            "",
        ] {
            let obb = container(&[(hostile, b"x"), ("ok.wav", b"y")]);
            bad.push((obb, "a name leaves the folder"));
        }
        let latin1 = raw(&[(&b"\xE9.wav\0"[..], 28 + 14 + 6, 1, 0)], b"x");
        bad.push((latin1, "a name is not UTF-8"));
        for (i, (obb, why)) in bad.iter().enumerate() {
            let mut called = false;
            let got = read(&obb[..], obb.len() as u64, |_, _| {
                called = true;
                Ok(())
            });
            let got = got.map_err(|e| e.to_string());
            assert_eq!(
                got,
                Err(format!("the content archive is damaged: {why}")),
                "case {i}"
            );
            assert!(!called, "case {i}");
        }
    }

    #[test]
    fn folder_entries_are_skipped() {
        let obb = container(&[("Gen/", b"x"), ("./", b""), SMALL[0], ("Gen\\Sub\\", b"")]);
        assert_eq!(whole(&obb).unwrap(), owned(&[SMALL[0]]));
    }

    #[test]
    fn flagged_entries_are_skipped_and_another_version_still_reads() {
        let flags = fields0(SMALL[0].0).3;
        let flagged = patch(container(&SMALL), flags, &[0, 1]);
        let want = owned(&[SMALL[1], SMALL[2], SMALL[3]]);
        assert_eq!(whole(&flagged).unwrap(), want);
        // A flagged entry's bounds are never looked at.
        let off = fields0(SMALL[0].0).1;
        let wild = patch(flagged, off, &[0xFF; 8]);
        assert_eq!(whole(&wild).unwrap(), want);
        let v4 = patch(container(&SMALL), 0, &4u32.to_be_bytes());
        assert_eq!(whole(&v4).unwrap().len(), SMALL.len());
    }

    #[test]
    fn what_each_leaves_unread_is_skipped_and_its_error_stops_the_read() {
        let obb = container(&SMALL);
        let mut names = Vec::new();
        read(&obb[..], obb.len() as u64, |name, data| {
            let mut one = [0; 1];
            let _ = data.read(&mut one)?;
            names.push(name.to_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!(names.len(), SMALL.len());
        let mut calls = 0;
        let err = read(&obb[..], obb.len() as u64, |_, _| {
            calls += 1;
            Err(io::Error::other("disk full"))
        });
        assert_eq!(err.unwrap_err().to_string(), "disk full");
        assert_eq!(calls, 1);
    }

    #[test]
    fn flipping_any_bit_never_panics_or_leaves_the_folder() {
        let obb = container(&SMALL[..3]);
        for at in 0..obb.len() {
            for bit in 0..8 {
                let mut bad = obb.clone();
                bad[at] ^= 1 << bit;
                if let Ok(got) = whole(&bad) {
                    let total: usize = got.iter().map(|(_, d)| d.len()).sum();
                    assert!(total <= bad.len());
                    assert!(
                        got.iter()
                            .all(|(n, _)| ProjectFiles::stays_inside(Path::new(n)))
                    );
                }
            }
        }
    }
}

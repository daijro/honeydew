//! Real files: the committed `owner-fresh-song.flm` (a 4.10.19 save, approved for the repo) and, when present, a
//! local `~/Downloads/sample.flm` (never copied into the repo). `cargo test --test flm_fixtures --
//! --nocapture` prints the byte-diff report: every chunk of `write(read(f))` that differs from `f`.

use honeydew::flm::chunk::{Chunk, ChunkIter, PrefixKind, file_chunks};
use honeydew::flm::{read, write};
use honeydew::song::{Raw, Song};

const OWNER: &[u8] = include_bytes!("fixtures/owner-fresh-song.flm");

/// The prefix length and children of a container chunk, by its id; `None` for a leaf.
fn children<'a>(chunk: &Chunk<'a>) -> Option<(usize, ChunkIter<'a>)> {
    let (kind, prefix) = match &chunk.id {
        b"RACK" | b"CHNL" | b"CLIP" => (PrefixKind::U32Magic, 8),
        b"RSMP" => (PrefixKind::MagicU32, 8),
        b"SMPl" | b"SMPL" | b"CLSm" => (PrefixKind::Magic, 4),
        b"RMOd" | b"RMOD" => (PrefixKind::TwoU32, 8),
        b"TRKH" => return Some((0, chunk.children())),
        _ => return None,
    };
    chunk
        .container(kind)
        .ok()
        .map(|(_, children)| (prefix, children))
}

/// The leaf chunks of `a` and `b` that differ, as "path: the differing byte ranges".
fn diff(a: ChunkIter, b: ChunkIter, out: &mut Vec<String>) {
    let (a, b): (Vec<_>, Vec<_>) = (
        a.map(Result::unwrap).collect(),
        b.map(Result::unwrap).collect(),
    );
    if a.iter().map(|c| c.id).ne(b.iter().map(|c| c.id)) {
        let ids = |list: &[Chunk]| {
            list.iter()
                .map(|c| String::from_utf8_lossy(&c.id).into_owned())
                .collect::<Vec<_>>()
        };
        out.push(format!(
            "{}: children {:?} -> {:?}",
            a.first().map_or("", |c| &c.path),
            ids(&a),
            ids(&b)
        ));
        return;
    }
    for (a, b) in a.iter().zip(&b) {
        match (children(a), children(b)) {
            (Some((prefix, x)), Some((_, y))) => {
                if a.payload[..prefix] != b.payload[..prefix] {
                    out.push(format!(
                        "{} prefix: {:?} -> {:?}",
                        a.path,
                        &a.payload[..prefix],
                        &b.payload[..prefix]
                    ));
                }
                diff(x, y, out);
            }
            _ if a.payload != b.payload => {
                let len = a.payload.len().max(b.payload.len());
                let mut ranges = Vec::new();
                let mut at = 0;
                while at < len {
                    if a.payload.get(at) != b.payload.get(at) {
                        let start = at;
                        while at < len && a.payload.get(at) != b.payload.get(at) {
                            at += 1;
                        }
                        ranges.push(format!("@{start}..{at}"));
                    }
                    at += 1;
                }
                out.push(format!(
                    "{} ({} -> {} B): {}",
                    a.path,
                    a.payload.len(),
                    b.payload.len(),
                    ranges.join(" ")
                ));
            }
            _ => {}
        }
    }
}

/// `song` without the bytes the reader kept, as after an edit of everything they belong to.
fn without_kept_bytes(mut song: Song) -> Song {
    for channel in &mut song.channels {
        for lane in &mut channel.lanes {
            for clip in &mut lane.clips {
                clip.file.raw_events = Raw(None);
                if let Some(slot) = clip.file.sample.as_deref_mut() {
                    (slot.file.raw_strc, slot.file.raw_str1) = (Raw(None), Raw(None));
                }
            }
        }
        for slot in channel.drum_kit.iter_mut().flat_map(|kit| &mut kit.slots) {
            (slot.file.raw_strc, slot.file.raw_str1) = (Raw(None), Raw(None));
        }
    }
    song
}

/// Checks `read(write(read(f))) == read(f)` and that writing is idempotent; returns the byte-diff report, then the
/// report for the song written without the bytes the reader kept.
fn round_trip(name: &str, bytes: &[u8]) -> (Vec<String>, Vec<String>) {
    let song = read(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut fresh = Vec::new();
    let reencoded = write(&without_kept_bytes(song.clone())).unwrap();
    diff(
        file_chunks(bytes).unwrap(),
        file_chunks(&reencoded).unwrap(),
        &mut fresh,
    );
    let written = write(&song).unwrap();
    let again = read(&written).unwrap();
    assert!(again == song, "{name}: read(write(read(f))) != read(f)");
    assert_eq!(
        write(&again).unwrap(),
        written,
        "{name}: the writer is not idempotent"
    );
    let mut report = Vec::new();
    diff(
        file_chunks(bytes).unwrap(),
        file_chunks(&written).unwrap(),
        &mut report,
    );
    println!(
        "{name}: {} B -> {} B, {} chunks differ",
        bytes.len(),
        written.len(),
        report.len()
    );
    for line in &report {
        println!("  {line}");
    }
    println!("{name} without kept bytes: {} chunks differ", fresh.len());
    for line in &fresh {
        println!("  {line}");
    }
    (report, fresh)
}

#[test]
fn the_owners_fresh_song_round_trips() {
    let (report, fresh) = round_trip("owner-fresh-song.flm", OWNER);
    assert!(report.is_empty(), "{report:#?}");
    assert!(fresh.is_empty(), "{fresh:#?}");
}

#[test]
fn the_owners_local_sample_round_trips_when_present() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let Ok(bytes) = std::fs::read(std::path::Path::new(&home).join("Downloads/sample.flm")) else {
        return;
    };
    round_trip("sample.flm", &bytes);
}

/// The view zooms the zoom-units flag covers: the playlist's, then per channel its `CHHD`@1040 and its clips' `ZOOM`
/// zooms.
fn view_zooms(song: &Song) -> Vec<f64> {
    let mut zooms = song.file.playlist_view[2..].to_vec();
    for c in &song.channels {
        zooms.push(c.file.view[0]);
        let clips = c.lanes.iter().flat_map(|l| &l.clips);
        zooms.extend(clips.flat_map(|clip| clip.file.zoom[2..].to_vec()));
    }
    zooms
}

/// Both layouts open, save and open again as the app does: a file without the zoom-units flag (4.10.19) has its view
/// zooms per device pixel and takes them per U at the U it opens with; a 5.0.1 file (flag 1) has them per U already
/// and writes back byte for byte. Either saves as 5.0.1 (HEAD 368 bytes: the version, then the flag) and opens again
/// with the same zooms. Real 5.0.1 saves come from `STUDIO_FLM_FIXTURES` when it is set (never copied into the repo).
#[test]
fn both_layouts_open_save_and_open_again_with_their_view_zooms() {
    // The app's U in device pixels as it opens them.
    const U: f64 = 85.0;
    let check = |name: &str, bytes: &[u8]| -> Vec<u8> {
        let file = read(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let scale = if file.file.zooms_per_u() { 1.0 } else { U };
        let want: Vec<f64> = view_zooms(&file).iter().map(|z| z * scale).collect();
        let mut song = file.clone();
        song.zooms_to_units(U);
        assert_eq!(view_zooms(&song), want, "{name}: opened");
        song.stamp_save("2026-10-19 12:00:00");
        let saved = write(&song).unwrap();
        assert_eq!(saved[8..12], 368_u32.to_le_bytes(), "{name}");
        assert_eq!(
            &saved[12 + 358..12 + 368],
            b"\x05\0\0\x005.0.1\x01",
            "{name}"
        );
        let mut again = read(&saved).unwrap();
        assert_eq!(view_zooms(&again), want, "{name}: opened again");
        let opened = again.clone();
        again.zooms_to_units(U);
        assert!(again == opened, "{name}: converted again");
        if file.file.zooms_per_u() {
            let (report, _) = round_trip(name, bytes);
            assert!(report.is_empty(), "{name}: {report:#?}");
        }
        saved
    };
    let saved = check("owner-fresh-song.flm", OWNER);
    check("owner-fresh-song.flm saved", &saved);
    let Ok(folder) = std::env::var("STUDIO_FLM_FIXTURES") else {
        return;
    };
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "flm") {
            check(&path.display().to_string(), &std::fs::read(&path).unwrap());
        }
    }
}

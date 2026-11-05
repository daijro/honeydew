//! Project versions (#69): named snapshots of the song in branches, which the song file keeps.
//!
//! A version is the song as it was at a `Save version` (or the Save dialog's `Save project version`): a name, a note, the
//! time, and the song itself as a deflated `.flm` without its versions. A branch is a line of versions, oldest first.
//! `New branch` starts one from a version, its first version a copy of that one, so no branch needs another: deleting or
//! trimming one never touches the rest.
//!
//! The file keeps each branch in an `HDVb` chunk at its end, which FL Studio Mobile skips as it skips every unknown chunk,
//! and which an older version of the app keeps as it is. A song without versions writes none.
//!
//! Size: a save of a version trims the branch it goes into, oldest first, to the `Project version size limit` (SETTINGS,
//! the undo history's limit too, [`crate::history`]) and to what [`FILE_ROOM`] leaves it after the other branches; it
//! never trims another branch, and always keeps the version it saved.

use std::sync::Arc;

use crate::flm::FlmError;
use crate::flm::chunk::{ChunkWriter, Cursor, MAX_FILE_SIZE, text};
use crate::song::Song;

/// A branch's chunk.
pub const CHUNK: [u8; 4] = *b"HDVb";
/// What the versions of all branches may take together: a quarter of the largest file the app opens, which leaves the
/// rest to the song, and keeps each branch's chunk under the chunk limit.
pub const FILE_ROOM: usize = (crate::project_files::MAX_FILE_BYTES / 4) as usize;
/// The first branch's name.
const MAIN: &str = "main";

/// The song's versions ([`crate::song::SongFileState::versions`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Versions {
    /// In the order they were made.
    pub branches: Vec<Branch>,
    /// The branch a save goes into when none is picked: the last one saved into, restored from or made.
    pub current: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    /// Oldest first.
    pub versions: Vec<Version>,
    /// The number in the next version's name, `Version N`.
    pub next: u32,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Version {
    pub name: String,
    pub note: String,
    /// When it was saved: seconds since 1970, as `META`'s dates count them (local time on the web, UTC elsewhere).
    pub time: i64,
    /// The song: a deflated `.flm` without versions.
    pub data: Arc<[u8]>,
}

impl std::fmt::Debug for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Version")
            .field("name", &self.name)
            .field("note", &self.note)
            .field("time", &self.time)
            .field("bytes", &self.data.len())
            .finish()
    }
}

impl Version {
    /// The song it keeps.
    pub fn song(&self) -> Result<Song, String> {
        let flm = miniz_oxide::inflate::decompress_to_vec_with_limit(&self.data, MAX_FILE_SIZE)
            .map_err(|_| "The version's data is damaged".to_owned())?;
        crate::flm::read(&flm).map_err(|e| e.to_string())
    }
}

impl Branch {
    /// What its versions take in the file.
    pub fn bytes(&self) -> usize {
        self.versions.iter().map(|v| v.data.len()).sum()
    }
}

impl Versions {
    pub fn bytes(&self) -> usize {
        self.branches.iter().map(Branch::bytes).sum()
    }

    /// The version at (branch, index).
    pub fn get(&self, (b, v): (usize, usize)) -> Option<&Version> {
        self.branches.get(b)?.versions.get(v)
    }

    /// Starts a branch, `Branch N`, from the version at `at`: its first version a copy of that one. It becomes the
    /// current branch; returns its index.
    pub fn branch_from(&mut self, at: (usize, usize)) -> Option<usize> {
        let version = self.get(at)?.clone();
        let next = self.branches[at.0].next;
        let name = (2..)
            .map(|n| format!("Branch {n}"))
            .find(|n| !self.branches.iter().any(|b| &b.name == n))
            .expect("an unbounded range has a free name");
        self.branches.push(Branch {
            name,
            versions: vec![version],
            next,
        });
        self.current = self.branches.len() - 1;
        Some(self.current)
    }

    /// Deletes version `v` of branch `b`, or with `None` the whole branch. An emptied branch stays.
    pub fn delete(&mut self, b: usize, v: Option<usize>) {
        match v {
            Some(v) if v < self.branches.get(b).map_or(0, |b| b.versions.len()) => {
                self.branches[b].versions.remove(v);
            }
            None if b < self.branches.len() => {
                self.branches.remove(b);
                if self.current > b || self.current >= self.branches.len() {
                    self.current = self.current.saturating_sub(1);
                }
            }
            _ => {}
        }
    }
}

/// `song` as a version keeps it: its plugins' states as they are now, without versions, deflated.
fn encode(song: &Song) -> Result<Arc<[u8]>, FlmError> {
    let mut song = song.clone();
    song.file.versions = Arc::default();
    crate::plugins::store_states(&mut song);
    let flm = crate::flm::write(&song)?;
    Ok(miniz_oxide::deflate::compress_to_vec(&flm, 6).into())
}

/// Saves `song` as a new version, `Version N`, at the end of branch `branch` (else the current one; a new `main` when
/// there is none), which becomes the current branch, and trims that branch to `limit` bytes and to what [`FILE_ROOM`]
/// leaves it. Returns where the version is.
pub fn commit(
    song: &mut Song,
    branch: Option<usize>,
    time: i64,
    limit: usize,
) -> Result<(usize, usize), FlmError> {
    let data = encode(song)?;
    let versions = Arc::make_mut(&mut song.file.versions);
    if versions.branches.is_empty() {
        versions.branches.push(Branch {
            name: MAIN.into(),
            versions: Vec::new(),
            next: 1,
        });
    }
    let b = branch
        .unwrap_or(versions.current)
        .min(versions.branches.len() - 1);
    let room = FILE_ROOM.saturating_sub(versions.bytes() - versions.branches[b].bytes());
    let branch = &mut versions.branches[b];
    branch.versions.push(Version {
        name: format!("Version {}", branch.next),
        note: String::new(),
        time,
        data,
    });
    branch.next += 1;
    // ponytail: recounts per dropped version, O(n²) in the versions dropped at once; fine for one save's few.
    while branch.versions.len() > 1 && branch.bytes() > limit.min(room) {
        branch.versions.remove(0);
    }
    versions.current = b;
    Ok((b, branch.versions.len() - 1))
}

/// The song the version at `at` keeps, to open as `song`: what is no edit ([`Song::keep_unedited`]: the title, the save
/// stamp, the view state, the versions) stays `song`'s, and its next channel id never goes back, so that no channel id
/// comes back. Its branch becomes the current one.
pub fn restored(song: &Song, at: (usize, usize)) -> Result<Song, String> {
    let version = (song.file.versions.get(at)).ok_or("There is no such version")?;
    let mut restored = version.song()?;
    restored.keep_unedited(song);
    let next = &mut restored.file.next_channel_id;
    *next = (*next).max(song.file.next_channel_id);
    Arc::make_mut(&mut restored.file.versions).current = at.0;
    Ok(restored)
}

/// Writes each branch as an `HDVb`: a `u8` (1: the current branch), its name, the next version number (`u32`), the number
/// of versions (`u32`), then each version's name, note, time (`i64`) and data; strings and data are `u32`-length-prefixed.
pub fn write(writer: &mut ChunkWriter, versions: &Versions) -> Result<(), FlmError> {
    for (i, branch) in versions.branches.iter().enumerate() {
        writer.chunk(CHUNK, |w| {
            w.u8(u8::from(i == versions.current))?;
            w.vstr32(branch.name.as_bytes())?;
            w.u32(branch.next)?;
            w.u32(branch.versions.len() as u32)?;
            for v in &branch.versions {
                w.vstr32(v.name.as_bytes())?;
                w.vstr32(v.note.as_bytes())?;
                w.u64(v.time as u64)?;
                w.vstr32(&v.data)?;
            }
            Ok(())
        })?;
    }
    Ok(())
}

/// Reads an `HDVb`'s `payload` as the next branch of `versions`. Fails on a short one, which the reader then keeps as an
/// unknown chunk.
pub fn read(payload: &[u8], path: &str, versions: &mut Versions) -> Result<(), FlmError> {
    let mut r = Cursor::new(payload, path);
    let current = r.u8()? & 1 == 1;
    let name = text(r.vstr32()?);
    let next = r.u32()?;
    let count = r.u32()?;
    let mut list = Vec::new();
    for _ in 0..count {
        list.push(Version {
            name: text(r.vstr32()?),
            note: text(r.vstr32()?),
            time: r.u64()? as i64,
            data: r.vstr32()?.into(),
        });
    }
    if current {
        versions.current = versions.branches.len();
    }
    versions.branches.push(Branch {
        name,
        versions: list,
        next,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flm::chunk::file_chunks;
    use crate::song::ChannelKind;

    const LIMIT: usize = 256 << 20;

    fn song() -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Instrument);
        song.tempo_bpm = 100.0;
        song
    }

    #[test]
    fn versions_round_trip_through_the_file_whose_reading_without_their_chunks_is_the_same_song() {
        let mut song = song();
        assert!(
            !crate::flm::write(&song)
                .unwrap()
                .windows(4)
                .any(|w| w == CHUNK)
        );
        commit(&mut song, None, 1_000, LIMIT).unwrap();
        song.tempo_bpm = 140.0;
        commit(&mut song, None, 2_000, LIMIT).unwrap();
        let versions = Arc::make_mut(&mut song.file.versions);
        versions.branch_from((0, 0)).unwrap();
        versions.branches[1].versions[0].note = "Slower, ünïcode".into();
        versions.current = 0;
        let bytes = crate::flm::write(&song).unwrap();

        let mut read = crate::flm::read(&bytes).unwrap();
        assert_eq!(read.file.versions, song.file.versions);
        let old = read.file.versions.get((1, 0)).unwrap().song().unwrap();
        assert_eq!((old.tempo_bpm, old.file.versions.bytes()), (100.0, 0));

        // FL Studio Mobile skips every chunk it does not know: without the `HDVb`s, the file is the song alone.
        let mut kept = bytes[..4].to_vec();
        for chunk in file_chunks(&bytes).unwrap().map(Result::unwrap) {
            if chunk.id != CHUNK {
                kept.extend(chunk.id);
                kept.extend((chunk.payload.len() as u32).to_le_bytes());
                kept.extend(chunk.payload);
            }
        }
        let alone = Song {
            file: crate::song::SongFileState {
                versions: Arc::default(),
                ..song.file.clone()
            },
            ..song.clone()
        };
        assert_eq!(kept, crate::flm::write(&alone).unwrap());
        read.file.versions = Arc::default();
        assert_eq!(crate::flm::read(&kept).unwrap(), read);
    }

    #[test]
    fn a_save_trims_only_its_own_branchs_oldest_versions_and_keeps_its_new_one() {
        let mut song = song();
        for t in 0..3 {
            commit(&mut song, None, t, LIMIT).unwrap();
        }
        Arc::make_mut(&mut song.file.versions).branch_from((0, 1));
        // The same song every time: each version takes the same bytes.
        let one = song.file.versions.get((0, 0)).unwrap().data.len();
        for t in 3..6 {
            commit(&mut song, None, t, 2 * one).unwrap();
        }
        let names = |song: &Song, b: usize| -> Vec<String> {
            let versions = &song.file.versions.branches[b].versions;
            versions.iter().map(|v| v.name.clone()).collect()
        };
        assert_eq!(names(&song, 0), ["Version 1", "Version 2", "Version 3"]);
        assert_eq!(names(&song, 1), ["Version 5", "Version 6"]);
        // Below one version, the new one still stays.
        commit(&mut song, Some(0), 6, 1).unwrap();
        assert_eq!(names(&song, 0), ["Version 4"]);
        assert_eq!(names(&song, 1).len(), 2);
    }
}

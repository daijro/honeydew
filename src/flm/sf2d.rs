//! The SoundFont Player's (52) `SF2D`: the chunk in its `CSTM` blob that names the SoundFont, a `vstr32` shorthand
//! path, and the patch, an `i32` index. A module carries it only once a SoundFont is loaded. The rest of the blob is
//! kept as it is.

use super::chunk::{ChunkIter, ChunkWriter, ContentPath, FlmError};

const ID: [u8; 4] = *b"SF2D";

/// What a SoundFont Player plays: the SoundFont at `path` and its patch `patch` (an index into its patches in file
/// order, [`crate::sf2::presets`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sf2d {
    pub path: ContentPath,
    pub patch: i32,
}

impl Sf2d {
    /// The `SF2D` in the `CSTM` blob `cstm`. `None` without one, or with one whose path is cut short or empty. A
    /// missing patch index is 0.
    pub fn read(cstm: &[u8]) -> Option<Self> {
        let chunk = ChunkIter::new(cstm, "CSTM")
            .map_while(Result::ok)
            .find(|c| c.id == ID)?;
        let mut reader = chunk.reader();
        let path = reader.content_path32().ok()?;
        let empty = matches!(&path, ContentPath::Shorthand { relative: p, .. } | ContentPath::Plain(p) if p.is_empty());
        (!empty).then(|| Self {
            path,
            patch: reader.i32_or(0),
        })
    }

    /// `cstm` (the module's blob, if it has one) with its `SF2D` set to this one, in its place or else last; the other
    /// chunks are kept.
    pub fn write(&self, cstm: Option<&[u8]>) -> Result<Vec<u8>, FlmError> {
        let mut writer = ChunkWriter::default();
        let sf2d = |w: &mut ChunkWriter| {
            w.chunk(ID, |w| {
                w.content_path32(&self.path)?;
                w.i32(self.patch)
            })
        };
        let mut written = false;
        for chunk in ChunkIter::new(cstm.unwrap_or_default(), "CSTM").map_while(Result::ok) {
            if chunk.id == ID {
                // In place of the first; any later one goes, as reading takes the first.
                if !std::mem::replace(&mut written, true) {
                    sf2d(&mut writer)?;
                }
            } else {
                writer.chunk(chunk.id, |w| w.raw(chunk.payload))?;
            }
        }
        if !written {
            sf2d(&mut writer)?;
        }
        Ok(writer.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flm::chunk::ContentRoot;

    fn piano(patch: i32) -> Sf2d {
        Sf2d {
            path: ContentPath::Shorthand {
                root: ContentRoot::User,
                relative: "My SoundFonts/Keys/Piano.sf2".into(),
            },
            patch,
        }
    }

    #[test]
    fn sf2d_is_a_shorthand_path_and_the_patch_index() {
        let blob = piano(7).write(None).unwrap();
        let path = b"@L@\0My SoundFonts/Keys/Piano.sf2";
        let payload = [
            &(path.len() as u32).to_le_bytes()[..],
            path,
            &7_i32.to_le_bytes(),
        ]
        .concat();
        let want = [
            &b"SF2D"[..],
            &(payload.len() as u32).to_le_bytes(),
            &payload,
        ]
        .concat();
        assert_eq!(blob, want);
        assert_eq!(Sf2d::read(&blob), Some(piano(7)));
    }

    #[test]
    fn a_new_sf2d_replaces_the_old_and_keeps_the_other_chunks() {
        let other = b"XTRA\x02\0\0\0ab";
        let blob = [&other[..], &piano(1).write(None).unwrap(), other].concat();
        let next = piano(2).write(Some(&blob)).unwrap();
        let want = [&other[..], &piano(2).write(None).unwrap(), other].concat();
        assert_eq!(next, want);
        // Without one, it goes last.
        let added = piano(3).write(Some(other)).unwrap();
        assert_eq!(added, [&other[..], &piano(3).write(None).unwrap()].concat());
    }

    #[test]
    fn short_or_missing_sf2ds_read_as_none_or_patch_0() {
        assert_eq!(Sf2d::read(&[]), None);
        assert_eq!(Sf2d::read(b"XTRA\0\0\0\0"), None);
        // An empty path names no SoundFont.
        assert_eq!(Sf2d::read(b"SF2D\x08\0\0\0\0\0\0\0\x01\0\0\0"), None);
        // A path cut short.
        assert_eq!(Sf2d::read(b"SF2D\x06\0\0\0\x09\0\0\0@L"), None);
        // No patch index: patch 0.
        let blob = piano(5).write(None).unwrap();
        let short = [
            &b"SF2D"[..],
            &((blob.len() - 12) as u32).to_le_bytes(),
            &blob[8..blob.len() - 4],
        ]
        .concat();
        assert_eq!(Sf2d::read(&short), Some(piano(0)));
    }

    #[test]
    fn a_songs_soundfont_player_keeps_its_sf2d_through_a_save() {
        use crate::song::{ChannelKind, Song, SynthKind};
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::SoundFontPlayer));
        let module = &mut song.channels[1].rack.modules[0];
        assert_eq!(module.type_id, 52);
        module.file.custom_data = Some(piano(4).write(None).unwrap());
        let back = crate::flm::read(&crate::flm::write(&song).unwrap()).unwrap();
        let blob = back.channels[1].rack.modules[0].file.custom_data.as_deref();
        assert_eq!(blob.and_then(Sf2d::read), Some(piano(4)));
    }
}

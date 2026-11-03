//! The existing FLM format as a canonical, bounded peer checkpoint.

use super::{Conflict, shared, validate};
use crate::{flm, song::Song};

/// Same limit as a file imported by the web app; samples travel separately.
pub const MAX_BYTES: usize = 128 * 1024 * 1024;

fn error(e: impl std::fmt::Display) -> Conflict {
    Conflict(format!("Invalid shared checkpoint: {e}"))
}

fn read(bytes: &[u8]) -> Result<Song, Conflict> {
    if bytes.len() > MAX_BYTES {
        return Err(error("project exceeds the web file limit"));
    }
    let (song, notices) = flm::read_with_notices(bytes).map_err(error)?;
    if !notices.is_empty() {
        return Err(error("project requires repairs"));
    }
    let song = shared(&song);
    validate(&song)?;
    for lane in song.channels.iter().flat_map(|c| &c.lanes) {
        for raw in lane
            .clips
            .iter()
            .filter_map(|c| c.file.raw_events.0.as_deref())
        {
            // Raw records preserve unknown tails, but also bypass the writer's ordinary numeric checks.
            let size = raw
                .get(..2)
                .map(|s| u16::from_le_bytes([s[0], s[1]]) as usize)
                .ok_or_else(|| error("short event header"))?;
            if size == 0 && raw.len() == 2 {
                continue;
            }
            if size < 14 || (raw.len() - 2) % size != 0 {
                return Err(error("incomplete event records"));
            }
            for record in raw[2..].chunks_exact(size) {
                let length = f64::from_le_bytes(record[4..12].try_into().unwrap());
                let key = i16::from_le_bytes([record[12], record[13]]);
                let key_valid = match lane.kind {
                    crate::song::LaneKind::Notes => (0..=127).contains(&key),
                    crate::song::LaneKind::Drums => (0..=255).contains(&key),
                    _ => true,
                };
                if !length.is_finite() || !key_valid {
                    return Err(error("event requires repairs"));
                }
            }
        }
    }
    Ok(song)
}

/// Settle the format's derived fields once when entering a session.
pub fn encode(song: &Song) -> Result<Vec<u8>, Conflict> {
    let song = shared(song);
    fixed_text(&song)?;
    validate(&song)?;
    let bytes = flm::write(&song).map_err(error)?;
    let bytes = flm::write(&read(&bytes)?).map_err(error)?;
    decode(&bytes)?;
    Ok(bytes)
}

fn fixed_text(song: &Song) -> Result<(), Conflict> {
    let text = |value: &str, limit: usize| {
        if value.len() > limit || value.contains('\0') {
            Err(error("text would be truncated by the project format"))
        } else {
            Ok(())
        }
    };
    text(&song.title, 256)?;
    text(&song.file.meta.author, 256)?;
    text(&song.file.meta.display_title, 256)?;
    text(&song.file.meta.info_url, 2048)?;
    for ch in &song.channels {
        text(&ch.name, 1024)?;
        for lane in &ch.lanes {
            text(&lane.label, 1024)?;
        }
        for module in &ch.rack.modules {
            text(&module.file.category, 256)?;
            text(&module.file.preset_name, 256)?;
        }
        let slots = ch.drum_kit.iter().flat_map(|k| &k.slots).chain(
            ch.lanes
                .iter()
                .flat_map(|l| &l.clips)
                .filter_map(|c| c.file.sample.as_deref()),
        );
        for slot in slots {
            text(&slot.name, 512)?;
            text(&slot.folder, 512)?;
        }
    }
    Ok(())
}

/// A peer may not rely on the file reader silently repairing or dropping its changes.
pub fn decode(bytes: &[u8]) -> Result<Song, Conflict> {
    let song = read(bytes)?;
    if flm::write(&song).map_err(error)? != bytes {
        return Err(error("noncanonical project"));
    }
    Ok(song)
}

pub fn merge_bytes(base: &[u8], current: &[u8], edited: &[u8]) -> Result<Vec<u8>, Conflict> {
    encode(&super::merge(
        &decode(base)?,
        &decode(current)?,
        &decode(edited)?,
    )?)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = playgroundHoneydewMerge)]
pub fn merge_web(
    base: &[u8],
    current: &[u8],
    edited: &[u8],
) -> Result<Vec<u8>, wasm_bindgen::JsValue> {
    merge_bytes(base, current, edited).map_err(|e| wasm_bindgen::JsValue::from_str(&e.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{collaboration, flm, song::*};

    fn song() -> Song {
        let mut s = Song::default();
        s.add_channel(ChannelKind::Synth(SynthKind::DwSampler));
        s.channels[1].lanes[0]
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
        s.file.unknown_chunks.push(RawChunk {
            id: *b"TEST",
            payload: vec![7, 9],
        });
        s
    }

    #[test]
    fn checkpoints_are_stable_and_keep_musical_and_unknown_data() {
        let s = song();
        let bytes = encode(&s).unwrap();
        let loaded = decode(&bytes).unwrap();
        assert_eq!(encode(&loaded).unwrap(), bytes);
        assert_eq!(
            loaded.channels[1].lanes[0].clips[0].events,
            s.channels[1].lanes[0].clips[0].events
        );
        assert_eq!(loaded.file.unknown_chunks, s.file.unknown_chunks);
        assert_eq!(
            loaded.channels[1].rack.modules[0].type_id,
            s.channels[1].rack.modules[0].type_id
        );
        assert_eq!(loaded.channels[1].file.guid, s.channels[1].file.guid);
    }

    #[test]
    fn view_changes_do_not_change_checkpoint_bytes() {
        let s = song();
        let mut viewed = s.clone();
        viewed.channels[1].file.view = [7.0; 2];
        viewed.channels[1].lanes[0].clips[0].file.zoom = [9.0; 4];
        viewed.file.selected_channel = 1;
        viewed.file.playhead_beats = 50.0;
        assert_eq!(encode(&s).unwrap(), encode(&viewed).unwrap());
    }

    #[test]
    fn repaired_or_noncanonical_peer_input_is_rejected() {
        assert!(decode(b"bad").is_err());
        let mut viewed = song();
        viewed.file.playhead_beats = 12.0;
        assert!(
            decode(&flm::write(&viewed).unwrap()).is_err(),
            "personal views cannot arrive as document edits"
        );
        let mut corrupt = encode(&song()).unwrap();
        corrupt.truncate(corrupt.len() - 1);
        assert!(decode(&corrupt).is_err());
        let mut invalid = song();
        invalid.channels[1].file.sends[0].target_channel_id = 999;
        assert!(encode(&invalid).is_err());
    }

    #[test]
    fn oversized_checkpoints_fail_before_decoding() {
        assert!(
            decode(&vec![0; MAX_BYTES + 1])
                .unwrap_err()
                .0
                .contains("web file limit")
        );
    }

    #[test]
    fn decoded_edits_still_merge_by_musical_object() {
        let mut base = song();
        base.channels[1].lanes[0].add_clip(512, 4.0);
        let base = encode(&base).unwrap();
        let mut a = decode(&base).unwrap();
        let mut b = a.clone();
        a.channels[1].lanes[0].clips[0].start_tick = 128;
        b.channels[1].lanes[0].clips[1].start_tick = 768;
        let (a, b) = (encode(&a).unwrap(), encode(&b).unwrap());
        let both = merge_bytes(&base, &b, &a).unwrap();
        let combined = decode(&both).unwrap();
        assert_eq!(combined.channels[1].lanes[0].clips[0].start_tick, 128);
        assert_eq!(combined.channels[1].lanes[0].clips[1].start_tick, 768);
        assert_eq!(merge_bytes(&a, &both, &base).unwrap(), b);
        let mut conflicting = decode(&base).unwrap();
        conflicting.channels[1].lanes[0].clips[0].start_tick = 64;
        assert!(merge_bytes(&base, &a, &encode(&conflicting).unwrap()).is_err());
        assert_eq!(collaboration::shared(&combined), combined);
    }

    #[test]
    fn independent_moves_can_cross_other_clips_without_an_order_conflict() {
        let mut s = song();
        s.channels[1].lanes[0].add_clip(512, 4.0);
        s.channels[1].lanes[0].add_clip(1024, 4.0);
        let base = encode(&s).unwrap();
        let mut a = decode(&base).unwrap();
        let mut b = a.clone();
        a.channels[1].lanes[0].clips[0].start_tick = 2048;
        b.channels[1].lanes[0].clips[2].start_tick = 256;
        let (a, b) = (encode(&a).unwrap(), encode(&b).unwrap());
        let out = decode(&merge_bytes(&base, &b, &a).unwrap()).unwrap();
        let clips = &out.channels[1].lanes[0].clips;
        assert_eq!(
            clips
                .iter()
                .map(|c| (c.id, c.start_tick))
                .collect::<Vec<_>>(),
            [(3, 256), (2, 512), (1, 2048)]
        );
    }

    #[test]
    fn invalid_clip_ids_cannot_enter_the_shared_document() {
        let mut s = song();
        s.channels[1].lanes[0].add_clip(512, 4.0).id = 1;
        assert!(encode(&s).is_err());
        assert!(decode(&flm::write(&s).unwrap()).is_err());
        s.channels[1].lanes[0].clips[1].id = flm::MAX_ID + 1;
        assert!(encode(&s).is_err());
        assert!(decode(&flm::write(&s).unwrap()).is_err());
    }

    #[test]
    fn module_and_pad_id_repairs_are_also_rejected() {
        let mut s = song();
        s.channels[1].rack.modules[0].instance_id = flm::MAX_ID as i32 + 1;
        assert!(encode(&s).is_err());
        let mut s = Song::default();
        s.add_channel(ChannelKind::Drums);
        for id in [0, 1, flm::MAX_ID + 1] {
            s.channels[1].drum_kit.as_mut().unwrap().slots[1].file.id = id;
            assert!(encode(&s).is_err());
        }
    }

    #[test]
    fn preserved_event_bytes_cannot_hide_repairs() {
        let bytes = encode(&song()).unwrap();
        let payload = bytes.windows(4).position(|v| v == b"EVN2").unwrap() + 8;
        let mut bad_key = bytes.clone();
        bad_key[payload + 14..payload + 16].copy_from_slice(&200i16.to_le_bytes());
        assert!(decode(&bad_key).is_err());
        let mut bad_length = bytes.clone();
        bad_length[payload + 6..payload + 14].copy_from_slice(&f64::NAN.to_le_bytes());
        assert!(decode(&bad_length).is_err());
        let mut trailing = decode(&bytes).unwrap();
        let mut raw = bytes[payload..payload + 22].to_vec();
        raw.push(42);
        trailing.channels[1].lanes[0].clips[0].file.raw_events = Raw(Some(raw));
        assert!(encode(&trailing).is_err());
    }

    #[test]
    fn extended_event_records_keep_their_unknown_bytes() {
        let mut s = song();
        let bytes = encode(&s).unwrap();
        let payload = bytes.windows(4).position(|v| v == b"EVN2").unwrap() + 8;
        let mut raw = bytes[payload..payload + 22].to_vec();
        raw[..2].copy_from_slice(&22u16.to_le_bytes());
        raw.extend([77, 88]);
        s.channels[1].lanes[0].clips[0].file.raw_events = Raw(Some(raw.clone()));
        let out = decode(&encode(&s).unwrap()).unwrap();
        assert_eq!(
            out.channels[1].lanes[0].clips[0].file.raw_events.0,
            Some(raw)
        );
    }

    #[test]
    fn shared_fixed_width_text_must_not_be_silently_truncated() {
        for field in 0..7 {
            let mut s = song();
            match field {
                0 => s.title = "a".repeat(257),
                1 => s.file.meta.author = "é".repeat(129),
                2 => s.file.meta.display_title = "a\0hidden".into(),
                3 => s.file.meta.info_url = "a".repeat(2049),
                4 => s.channels[1].name = "a".repeat(1025),
                5 => s.channels[1].lanes[0].label = "a".repeat(1025),
                _ => s.channels[1].rack.modules[0].file.preset_name = "a".repeat(257),
            }
            assert!(encode(&s).is_err(), "field {field}");
        }
    }
}

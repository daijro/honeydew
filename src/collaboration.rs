//! Musical document changes for the optional browser collaboration adapter.

use crate::song::*;

pub mod checkpoint;

// Exhaustive patterns make a new upstream field require a shared/local decision here.
macro_rules! local_fields {
    ($ty:ident, $to:ident, $from:ident; $($local:ident),*; $($shared:ident),* $(,)?) => {
        let $ty { $($local: _,)* $($shared: _,)* } = &*$to;
        $($to.$local.clone_from(&$from.$local);)*
    };
}

/// A deterministic musical projection. It includes unknown format data but no personal view state.
pub fn shared(song: &Song) -> Song {
    let mut song = song.clone();
    views(&mut song, None);
    song
}

/// Restore the recipient's views after applying a shared change.
pub fn keep_local(song: &mut Song, local: &Song) {
    views(song, Some(local));
}

fn lane_key(lane: &Lane) -> (LaneKind, Option<AutomationTarget>, &str) {
    (lane.kind, lane.target, &lane.label)
}

fn lane_views(lane: &mut Lane, from: &Lane) {
    // Hidden, as a zipped channel: each person's own view.
    lane.hidden = from.hidden;
    let to = &mut lane.file;
    let old = &from.file;
    local_fields!(LaneFileState, to, old; current_clip_id; flag, custom_label, unknown_chunks, desc_extra);
    let default = ClipFileState::default();
    for clip in &mut lane.clips {
        let old = from
            .clips
            .iter()
            .find(|c| c.id == clip.id)
            .map_or(&default, |c| &c.file);
        let to = &mut clip.file;
        local_fields!(ClipFileState, to, old; zoom;
            source_len_beats, sample, unknown_chunks, header_extra, raw_events, shapes);
    }
}

fn lane_matches(old: &[Lane], new: &[Lane]) -> Vec<Option<usize>> {
    let document = |l: &Lane| {
        let mut l = l.clone();
        lane_views(&mut l, &Lane::new(LaneKind::Notes));
        l.last_clip_id = 0;
        l
    };
    let (old, new) = (
        old.iter().map(document).collect::<Vec<_>>(),
        new.iter().map(document).collect::<Vec<_>>(),
    );
    new.iter()
        .map(|lane| {
            let unique = |same: &dyn Fn(&Lane) -> bool| {
                let mut matches = old.iter().enumerate().filter(|(_, l)| same(l));
                let first = matches.next();
                (matches.next().is_none() && new.iter().filter(|l| same(l)).count() == 1)
                    .then(|| first.map(|(i, _)| i))
                    .flatten()
            };
            unique(&|l| l == lane).or_else(|| unique(&|l| lane_key(l) == lane_key(lane)))
        })
        .collect()
}

fn views(song: &mut Song, local: Option<&Song>) {
    let default = Song::default();
    let from = local.unwrap_or(&default);
    local_fields!(Song, song, from; play_loop;
        title, tempo_bpm, time_sig, channels, key_root, key_scale, file, master_below);
    let to = &mut song.file;
    let from = &from.file;
    local_fields!(SongFileState, to, from;
        version, selected_channel, playhead_beats, playlist_view, show_sharps;
        last_send_bus, guid, play_truncated_notes, next_channel_id, head_extra,
        time_sig_extra, meta, keyboard, unknown_chunks);
    let to = &mut song.file.meta;
    let from = &from.meta;
    local_fields!(MetaState, to, from; created, modified, editing_seconds;
        author, display_title, locked, info_url, extra);
    let to = &mut song.file.keyboard;
    let from = &local.unwrap_or(&default).file.keyboard;
    local_fields!(KeyboardState, to, from; vertical_mode, octave_shift, velocity, timing, hold;
        chord_json, set_state, chord_set_name, extra);
    if let Some(local) = local {
        song.file.selected_channel = usize::try_from(local.file.selected_channel)
            .ok()
            .and_then(|i| local.channels.get(i))
            .and_then(|old| {
                song.channels
                    .iter()
                    .position(|c| c.id == old.id && c.file.guid == old.file.guid)
            })
            .unwrap_or(0) as i32;
    }

    let empty_channel = Channel::master();
    let empty_lane = Lane::new(LaneKind::Notes);
    for channel in &mut song.channels {
        let original = local.and_then(|s| {
            s.channels
                .iter()
                .find(|c| c.id == channel.id && c.file.guid == channel.file.guid)
        });
        let from = original.unwrap_or(&empty_channel);
        local_fields!(Channel, channel, from; zipped;
            name, kind, id, hue, params, bus_id, output_bus, rack, lanes, drum_kit, file);
        let to = &mut channel.file;
        let from = &from.file;
        local_fields!(ChannelFileState, to, from; view, input_monitoring, record_armed, midi_input;
            guid, preset_name, sends, names, rack_order, cc_links, rack_header_extra,
            params_extra, rack_unknown_chunks, channel_unknown_chunks, header_extra, midi_sends,
            modulations);
        for module in &mut channel.rack.modules {
            let ModuleInstance {
                type_id,
                instance_id,
                minimized,
                params: _,
                common: _,
                file: _,
                sidechain: _,
            } = module;
            *minimized = original
                .and_then(|c| {
                    c.rack
                        .modules
                        .iter()
                        .find(|m| m.instance_id == *instance_id && m.type_id == *type_id)
                })
                .is_some_and(|m| m.minimized);
        }
        let matches = original.map(|c| lane_matches(&c.lanes, &channel.lanes));
        for (index, lane) in channel.lanes.iter_mut().enumerate() {
            let from = original
                .and_then(|c| {
                    matches
                        .as_ref()
                        .and_then(|m| m[index])
                        .and_then(|i| c.lanes.get(i))
                })
                .unwrap_or(&empty_lane);
            lane_views(lane, from);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict(pub String);

fn choose<T: Clone + PartialEq>(
    base: &T,
    current: &T,
    edited: &T,
    path: &str,
) -> Result<T, Conflict> {
    if edited == base {
        Ok(current.clone())
    } else if current == base {
        Ok(edited.clone())
    } else {
        Err(Conflict(path.into()))
    }
}

macro_rules! fields {
    ($ty:ident, $b:ident, $c:ident, $e:ident, $out:ident, $path:expr; $($field:ident),*; $($separate:ident),* $(,)?) => {
        let $ty { $($field: _,)* $($separate: _,)* } = $b;
        $($out.$field = choose(&$b.$field, &$c.$field, &$e.$field, &format!("{}.{}", $path, stringify!($field)))?;)*
    };
}

/// Apply only the proposed musical changes, atomically. Invert an acknowledged edit by swapping its
/// base and edited arguments. The ordered browser protocol supplies authors, revision fencing and IDs.
pub fn merge(base: &Song, current: &Song, edited: &Song) -> Result<Song, Conflict> {
    let (b, c, e) = (shared(base), shared(current), shared(edited));
    let mut out = c.clone();
    fields!(Song, b, c, e, out, "song";
        title, tempo_bpm, time_sig, key_root, key_scale, play_loop, master_below; channels, file);
    out.file = merge_file(&b.file, &c.file, &e.file)?;
    let bar = out.time_sig.bar_beats();
    out.channels = keyed(
        &b.channels,
        &c.channels,
        &e.channels,
        |ch| ch.id,
        |b, c, e, path| merge_channel(b, c, e, path, bar),
        "channels",
        true,
    )?;
    validate(&out)?;
    keep_local(&mut out, current);
    Ok(out)
}

fn keyed<T: Clone + PartialEq, K: Ord + Copy + std::fmt::Debug>(
    b: &[T],
    c: &[T],
    e: &[T],
    key: impl Fn(&T) -> K,
    merge: impl Fn(&T, &T, &T, &str) -> Result<T, Conflict>,
    path: &str,
    ordered: bool,
) -> Result<Vec<T>, Conflict> {
    use std::collections::BTreeMap;
    fn index<'a, T, K: Ord>(v: &'a [T], key: &impl Fn(&T) -> K) -> BTreeMap<K, &'a T> {
        v.iter().map(|x| (key(x), x)).collect()
    }
    let (bm, cm, em) = (index(b, &key), index(c, &key), index(e, &key));
    if bm.len() != b.len() || cm.len() != c.len() || em.len() != e.len() {
        return Err(Conflict(format!("{path}: duplicate ID")));
    }
    let order = |v: &[T]| {
        let mut ids = v.iter().map(&key).collect::<Vec<_>>();
        if !ordered {
            ids.sort_unstable();
        }
        ids
    };
    let order = choose(&order(b), &order(c), &order(e), &format!("{path}.order"))?;
    let mut merged = BTreeMap::new();
    for id in bm.keys().chain(cm.keys()).chain(em.keys()) {
        if merged.contains_key(id) {
            continue;
        }
        let at = format!("{path}[{id:?}]");
        let value = match (bm.get(id), cm.get(id), em.get(id)) {
            (Some(b), Some(c), Some(e)) => Some(merge(b, c, e, &at)?),
            (None, Some(_), Some(_)) => {
                return Err(Conflict(format!("{at}: concurrent ID allocation")));
            }
            (b, c, e) => choose(
                &b.copied().cloned(),
                &c.copied().cloned(),
                &e.copied().cloned(),
                &at,
            )?,
        };
        merged.insert(*id, value);
    }
    order
        .into_iter()
        .map(|id| {
            merged
                .remove(&id)
                .flatten()
                .ok_or_else(|| Conflict(format!("{path}: missing ID")))
        })
        .collect()
}

fn merge_file(
    b: &SongFileState,
    c: &SongFileState,
    e: &SongFileState,
) -> Result<SongFileState, Conflict> {
    let mut out = c.clone();
    fields!(SongFileState, b, c, e, out, "song.file";
        version, selected_channel, playhead_beats, playlist_view, guid, show_sharps,
        play_truncated_notes, head_extra, time_sig_extra, keyboard, unknown_chunks;
        last_send_bus, next_channel_id, meta);
    out.last_send_bus = b.last_send_bus.max(c.last_send_bus).max(e.last_send_bus);
    out.next_channel_id = b
        .next_channel_id
        .max(c.next_channel_id)
        .max(e.next_channel_id);
    let (b, c, e) = (&b.meta, &c.meta, &e.meta);
    let mut meta = c.clone();
    fields!(MetaState, b, c, e, meta, "metadata";
        created, editing_seconds, author, display_title, locked, info_url, modified, extra;);
    out.meta = meta;
    Ok(out)
}

fn merge_channel(
    b: &Channel,
    c: &Channel,
    e: &Channel,
    path: &str,
    bar: u32,
) -> Result<Channel, Conflict> {
    if b.file.guid != c.file.guid || b.file.guid != e.file.guid {
        return choose(b, c, e, path);
    }
    let pads = |ch: &Channel| {
        ch.drum_kit
            .iter()
            .flat_map(|k| k.slots.iter().map(|s| s.file.id))
            .collect::<Vec<_>>()
    };
    if c != e
        && ((pads(b) != pads(c) && b.lanes != e.lanes)
            || (pads(b) != pads(e) && b.lanes != c.lanes))
    {
        return Err(Conflict(format!("{path}: drum pad order changed")));
    }
    let mut out = c.clone();
    fields!(Channel, b, c, e, out, path;
        name, kind, id, zipped, hue, params, bus_id, output_bus, file; rack, lanes, drum_kit);
    // Allocators stay raised after an Undo: compare without them, then keep the highest below.
    let rack = |ch: &Channel| Rack {
        last_instance_id: 0,
        ..ch.rack.clone()
    };
    out.rack = choose(&rack(b), &rack(c), &rack(e), &format!("{path}.rack"))?;
    let kit = |ch: &Channel| {
        ch.drum_kit.clone().map(|mut kit| {
            kit.file.last_slot_id = 0;
            kit
        })
    };
    out.drum_kit = choose(&kit(b), &kit(c), &kit(e), &format!("{path}.drum_kit"))?;
    out.rack.last_instance_id = b
        .rack
        .last_instance_id
        .max(c.rack.last_instance_id)
        .max(e.rack.last_instance_id);
    if let Some(kit) = &mut out.drum_kit {
        for old in [&b.drum_kit, &c.drum_kit, &e.drum_kit]
            .into_iter()
            .flatten()
        {
            kit.file.last_slot_id = kit.file.last_slot_id.max(old.file.last_slot_id);
        }
    }
    let same_shape = |a: &Channel| {
        b.lanes.len() == a.lanes.len()
            && b.lanes
                .iter()
                .zip(&a.lanes)
                .all(|(x, y)| lane_key(x) == lane_key(y))
    };
    // shortcut: same-header lanes have no identity; merge them atomically until upstream supplies IDs.
    let ambiguous = b.lanes.iter().enumerate().any(|(i, l)| {
        b.lanes[..i]
            .iter()
            .any(|other| lane_key(l) == lane_key(other))
    });
    if !same_shape(c) || !same_shape(e) || ambiguous {
        let lanes = |ch: &Channel| {
            let mut lanes = ch.lanes.clone();
            lanes.iter_mut().for_each(|lane| lane.last_clip_id = 0);
            lanes
        };
        out.lanes = choose(&lanes(b), &lanes(c), &lanes(e), &format!("{path}.tracks"))?;
        // Lanes have no stable ID, so preserve the highest observed counter across a structure change.
        let last = [&b.lanes, &c.lanes, &e.lanes]
            .into_iter()
            .flatten()
            .map(|l| l.last_clip_id)
            .max()
            .unwrap_or(0);
        for lane in &mut out.lanes {
            lane.last_clip_id = lane.last_clip_id.max(last);
        }
    } else {
        out.lanes = b
            .lanes
            .iter()
            .zip(&c.lanes)
            .zip(&e.lanes)
            .enumerate()
            .map(|(i, ((b, c), e))| merge_lane(b, c, e, &format!("{path}.track[{i}]"), bar))
            .collect::<Result<_, _>>()?;
    }
    Ok(out)
}

fn merge_lane(b: &Lane, c: &Lane, e: &Lane, path: &str, bar: u32) -> Result<Lane, Conflict> {
    let mut out = c.clone();
    fields!(Lane, b, c, e, out, path; kind, target, label, swing, grid, hidden, file; clips, last_clip_id);
    out.last_clip_id = b.last_clip_id.max(c.last_clip_id).max(e.last_clip_id);
    out.clips = keyed(
        &b.clips,
        &c.clips,
        &e.clips,
        |clip| clip.id,
        |b, c, e, path| merge_clip(b, c, e, path, bar),
        &format!("{path}.clips"),
        false,
    )?;
    // Clip order follows positions in FLM; moving separate clips must not become competing reorders.
    out.clips.sort_by_key(|clip| clip.start_tick);
    Ok(out)
}

/// A clip field by field, and its events item by item when both sides changed them ([`merge_events`]); the loop then
/// fits the merged content (`bar` beats a bar), as every editor's edit fits it.
fn merge_clip(b: &Clip, c: &Clip, e: &Clip, path: &str, bar: u32) -> Result<Clip, Conflict> {
    let mut out = c.clone();
    fields!(Clip, b, c, e, out, path;
        id, start_tick, visible_len_beats, offset_beats, muted, file; events, loop_len_beats);
    if b.events == c.events || b.events == e.events {
        out.events = choose(&b.events, &c.events, &e.events, path)?;
        out.loop_len_beats = choose(
            &b.loop_len_beats,
            &c.loop_len_beats,
            &e.loop_len_beats,
            path,
        )?;
    } else {
        out.events = merge_events(b, c, e, &format!("{path}.events"))?;
        out.fit_loop(bar);
    }
    Ok(out)
}

/// Events have no identity, so an item is a value: the n-th copy of an exact event. A moved or changed note is its
/// old value removed and a new one added. Each side's additions and removals apply; a value both sides add or remove
/// (one note changed two ways, or deleted by both) is a conflict, as [`choose`] has it. The steps between automation
/// and envelope points are derived ([`points::set_points`]): a curve whose points the merge changed gets them rebuilt.
fn merge_events(b: &Clip, c: &Clip, e: &Clip, path: &str) -> Result<Vec<Event>, Conflict> {
    use points::PointKey;
    use std::collections::BTreeMap;
    // An automation or envelope point's curve, and whether the event is a step between points.
    let curve = |x: &Event| match *x {
        Event::Automation { control, .. } => Some((PointKey::Automation, !control)),
        Event::Envelope {
            env_param, control, ..
        } => Some((PointKey::Envelope(env_param), !control)),
        _ => None,
    };
    let step = |x: &&Event| curve(x).is_some_and(|(_, step)| step);
    // Exact values: Debug prints each float so that it reads back to the same bits.
    let value = |x: &Event| format!("{x:?}");
    let mut counts = BTreeMap::<String, [usize; 3]>::new();
    for (side, clip) in [b, c, e].into_iter().enumerate() {
        for x in clip.events.iter().filter(|x| !step(x)) {
            counts.entry(value(x)).or_default()[side] += 1;
        }
    }
    let mut left = BTreeMap::new();
    for (v, [nb, nc, ne]) in counts {
        left.insert(v, choose(&nb, &nc, &ne, path)?);
    }
    // Current's copies of each value in order, then edited's new ones; and current's steps, for now.
    let mut out = c.clone();
    out.events = (c.events.iter().chain(&e.events))
        .filter(|x| !step(x))
        .filter(|x| {
            let n = left.get_mut(&value(x)).expect("counted");
            let keep = *n > 0;
            *n -= usize::from(keep);
            keep
        })
        .chain(c.events.iter().filter(step))
        .copied()
        .collect();
    out.events.sort_by_key(Event::pos);
    let (all, mut curves) = ([b, c, e].map(|clip| &clip.events), Vec::new());
    for (k, _) in all.into_iter().flatten().filter_map(curve) {
        if !curves.contains(&k) {
            curves.push(k);
        }
    }
    for k in curves {
        let pts = points::points(&out, k);
        if pts != points::points(c, k) {
            points::set_points(&mut out, k, pts);
        }
    }
    Ok(out.events)
}

fn validate(song: &Song) -> Result<(), Conflict> {
    crate::flm::write(song).map_err(|e| Conflict(format!("Invalid merged project: {e}")))?;
    let index = |id| song.channels.iter().position(|ch| ch.id == id);
    let mut edges = vec![Vec::new(); song.channels.len()];
    let mut indegree = vec![0; song.channels.len()];
    for (i, ch) in song.channels.iter().enumerate() {
        if ch.rack.last_instance_id > crate::flm::MAX_ID as i32
            || ch
                .rack
                .modules
                .iter()
                .any(|m| m.instance_id > crate::flm::MAX_ID as i32)
        {
            return Err(Conflict(format!(
                "Channel {}: module ID exceeds the file limit",
                ch.id
            )));
        }
        if let Some(kit) = &ch.drum_kit {
            let mut ids = std::collections::HashSet::new();
            if kit.file.last_slot_id > crate::flm::MAX_ID
                || kit.slots.iter().any(|s| {
                    s.file.id == 0 || s.file.id > crate::flm::MAX_ID || !ids.insert(s.file.id)
                })
            {
                return Err(Conflict(format!("Channel {}: invalid drum slot ID", ch.id)));
            }
        }
        for send in &ch.file.sends {
            let j = index(send.target_channel_id)
                .ok_or_else(|| Conflict(format!("Channel {}: send target removed", ch.id)))?;
            if i == 0 || i == j || edges[i].contains(&j) {
                return Err(Conflict(format!("Channel {}: invalid send", ch.id)));
            }
            edges[i].push(j);
            indegree[j] += 1;
        }
        for slot in ch.drum_kit.iter().flat_map(|k| &k.slots) {
            if slot.file.send_target >= 0 && index(slot.file.send_target).is_none() {
                return Err(Conflict(format!(
                    "Channel {}: pad send target removed",
                    ch.id
                )));
            }
        }
        for lane in &ch.lanes {
            let mut ids = std::collections::HashSet::new();
            if lane.last_clip_id > crate::flm::MAX_ID
                || lane
                    .clips
                    .iter()
                    .any(|c| c.id > crate::flm::MAX_ID || !ids.insert(c.id))
            {
                return Err(Conflict(format!("Channel {}: invalid clip ID", ch.id)));
            }
            if let Some(target) = lane.target {
                let valid = match target.instance_id {
                    0 => honeydew_audio::mixer::send_param(target.param).is_none_or(|(id, _)| {
                        index(id).is_some()
                            && ch.file.sends.iter().any(|s| s.target_channel_id == id)
                    }),
                    id if id > 0 => ch.rack.get(id).is_some(),
                    id => ch
                        .drum_kit
                        .as_ref()
                        .is_some_and(|k| k.slot_by_id(id.unsigned_abs()).is_some()),
                };
                if !valid {
                    return Err(Conflict(format!(
                        "Channel {}: automation target removed",
                        ch.id
                    )));
                }
            }
        }
    }
    // Pad sends feed pre-mix buffers, so only channel sends participate in the mixing-order graph.
    let mut ready: Vec<_> = indegree
        .iter()
        .enumerate()
        .filter_map(|(i, &n)| (n == 0).then_some(i))
        .collect();
    let mut visited = 0;
    while let Some(i) = ready.pop() {
        visited += 1;
        for &j in &edges[i] {
            indegree[j] -= 1;
            if indegree[j] == 0 {
                ready.push(j);
            }
        }
    }
    if visited != edges.len() {
        return Err(Conflict("Routing cycle".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Song {
        let mut s = Song::default();
        s.add_channel(ChannelKind::Synth(SynthKind::DwSampler));
        for start in [0, 512] {
            s.channels[1].lanes[0]
                .add_clip(start, 4.0)
                .events
                .push(Event::Note {
                    pos: 0,
                    len_beats: 1.0,
                    key: 60,
                    velocity: 204,
                    pan: 127,
                    slide: false,
                });
        }
        s
    }

    #[test]
    fn views_and_performance_controls_are_not_document_edits() {
        let a = song();
        let mut b = a.clone();
        b.play_loop = Some((0, 1024));
        b.file.selected_channel = 1;
        b.file.playhead_beats = 7.0;
        b.file.playlist_view = [3.0; 4];
        b.file.keyboard.octave_shift = 2;
        b.file.keyboard.hold = true;
        b.file.keyboard.velocity = 0.1;
        b.file.keyboard.timing = 0.7;
        b.file.keyboard.vertical_mode = 1;
        b.file.meta.editing_seconds = 900.0;
        b.file.meta.modified = "tomorrow".into();
        b.channels[1].zipped = true;
        b.channels[1].file.view = [4.0; 2];
        b.channels[1].file.record_armed = true;
        b.channels[1].file.input_monitoring = true;
        b.channels[1].rack.modules[0].minimized = true;
        b.channels[1].lanes[0].file.current_clip_id = 2;
        b.channels[1].lanes[0].clips[0].file.zoom = [9.0; 4];
        assert_eq!(shared(&a), shared(&b));
        let mut restored = shared(&b);
        keep_local(&mut restored, &b);
        assert_eq!(restored, b);
    }

    #[test]
    fn projection_retains_metadata_sound_and_unknown_file_data() {
        let a = song();
        let mut b = a.clone();
        b.file.meta.author = "a collaborator".into();
        b.file.unknown_chunks.push(RawChunk {
            id: *b"TEST",
            payload: vec![7, 9],
        });
        b.channels[1].rack.modules[0].params[0] = 0.2;
        b.channels[1].lanes[0].clips[0].start_tick = 128;
        b.file.keyboard.chord_set_name = "My chords".into();
        assert_eq!(shared(&b), b);
        assert_ne!(shared(&a), shared(&b));
    }

    #[test]
    fn local_views_follow_ids_when_channels_and_clips_move() {
        let mut local = song();
        local.add_channel(ChannelKind::AudioClip);
        local.file.selected_channel = 1;
        local.channels[1].file.view = [7.0; 2];
        local.channels[1].lanes[0].clips[0].file.zoom = [3.0; 4];
        let mut changed = shared(&local);
        changed.channels.swap(1, 2);
        changed.channels[2].lanes[0].clips.swap(0, 1);
        keep_local(&mut changed, &local);
        assert_eq!(changed.channels[2].file.view, [7.0; 2]);
        assert_eq!(changed.channels[2].lanes[0].clips[1].file.zoom, [3.0; 4]);
        assert_eq!(changed.channels[1].file.view, [0.0; 2]);
        assert_eq!(changed.file.selected_channel, 2);
    }

    #[test]
    fn independent_clip_edits_merge_and_personal_undo_keeps_the_other_edit() {
        let base = song();
        let mut alice = base.clone();
        alice.channels[1].lanes[0].clips[0].start_tick = 128;
        let mut bob = base.clone();
        bob.channels[1].lanes[0].clips[1].start_tick = 768;
        bob.file.playlist_view = [7.0; 4];
        let both = merge(&base, &bob, &alice).unwrap();
        assert_eq!(both.channels[1].lanes[0].clips[0].start_tick, 128);
        assert_eq!(both.channels[1].lanes[0].clips[1].start_tick, 768);
        assert_eq!(both.file.playlist_view, bob.file.playlist_view);
        let undone = merge(&alice, &both, &base).unwrap();
        assert_eq!(undone, bob);
        assert!(
            merge(&base, &both, &alice).is_err(),
            "equal values do not prove authorship; only the journal can deduplicate an operation"
        );
    }

    #[test]
    fn edits_inside_one_clip_merge_note_by_note_and_one_note_changed_two_ways_conflicts() {
        let note = |pos, key, velocity| Event::Note {
            pos,
            len_beats: 1.0,
            key,
            velocity,
            pan: 127,
            slide: false,
        };
        let mut base = song();
        base.channels[1].lanes[0].clips[0].events = [60, 64, 67].map(|k| note(0, k, 204)).into();
        let edit = |change: &dyn Fn(&mut Vec<Event>)| {
            let mut s = base.clone();
            let events = &mut s.channels[1].lanes[0].clips[0].events;
            change(events);
            events.sort_by_key(Event::pos);
            s
        };
        let notes = |s: &Song| {
            let mut n = s.channels[1].lanes[0].clips[0].events.clone();
            n.sort_by_key(|e| format!("{e:?}"));
            n
        };
        let tap_a = |ev: &mut Vec<Event>| ev.push(note(128, 72, 204));
        let tap_b = |ev: &mut Vec<Event>| ev.push(note(256, 74, 204));
        let move_e = |ev: &mut Vec<Event>| ev[1] = note(0, 65, 204);
        let drag_chord = |ev: &mut Vec<Event>| {
            for (i, k) in [62, 66, 69].into_iter().enumerate() {
                ev[i] = note(384, k, 204);
            }
        };
        // Two taps at once; a tap made while Alice's edit to the chord waited; a tap during Bob's chord drag.
        type Change<'a> = &'a dyn Fn(&mut Vec<Event>);
        let probes: [(Change, Change); 3] =
            [(&tap_a, &tap_b), (&move_e, &tap_b), (&drag_chord, &tap_a)];
        for (i, (a, b)) in probes.into_iter().enumerate() {
            let want = notes(&edit(&|ev| {
                a(ev);
                b(ev);
            }));
            let (a, b) = (edit(a), edit(b));
            assert_eq!(notes(&merge(&base, &a, &b).unwrap()), want, "probe {i}");
            assert_eq!(
                notes(&merge(&base, &b, &a).unwrap()),
                want,
                "probe {i} reversed"
            );
        }
        let softer = edit(&|ev| ev[1] = note(0, 64, 100));
        let moved = edit(&move_e);
        assert!(merge(&base, &softer, &moved).is_err());
        assert!(merge(&base, &moved, &softer).is_err());
    }

    #[test]
    fn conflicting_events_and_undo_fail_atomically_without_mutating_inputs() {
        let base = song();
        let mut alice = base.clone();
        alice.tempo_bpm = 140.0;
        alice.channels[1].lanes[0].clips[0].events.clear();
        let mut bob = base.clone();
        bob.channels[1].lanes[0].clips[0].events[0] = Event::Note {
            pos: 64,
            len_beats: 2.0,
            key: 64,
            velocity: 190,
            pan: 127,
            slide: false,
        };
        let before = bob.clone();
        assert!(merge(&base, &bob, &alice).is_err());
        assert_eq!(bob, before);
        assert_eq!(bob.tempo_bpm, 120.0);
        assert!(
            merge(&alice, &bob, &base).is_err(),
            "undo must not erase a later person's note"
        );
    }

    #[test]
    fn channel_reordering_keeps_independent_edits_and_views_on_their_ids() {
        let mut base = song();
        base.add_channel(ChannelKind::AudioClip);
        let mut alice = base.clone();
        alice.channels.swap(1, 2);
        let mut bob = base.clone();
        bob.channels[1].lanes[0].clips[0].muted = true;
        bob.channels[1].file.view = [4.0; 2];
        let both = merge(&base, &bob, &alice).unwrap();
        assert_eq!(both.channels[2].id, base.channels[1].id);
        assert!(both.channels[2].lanes[0].clips[0].muted);
        assert_eq!(both.channels[2].file.view, [4.0; 2]);
    }

    #[test]
    fn track_structure_changes_conflict_with_edits_to_the_old_tracks() {
        let mut base = song();
        base.channels[1].add_lane(LaneKind::Notes);
        base.channels[1].lanes[1].add_clip(2048, 4.0);
        let mut editing = base.clone();
        editing.channels[1].lanes[0].clips[0].start_tick = 64;
        for change in 0..3 {
            let mut structural = base.clone();
            match change {
                0 => structural.channels[1]
                    .lanes
                    .insert(0, Lane::new(LaneKind::Notes)),
                1 => {
                    structural.channels[1].lanes.remove(0);
                }
                _ => structural.channels[1].lanes.swap(0, 1),
            }
            assert!(
                merge(&base, &editing, &structural).is_err(),
                "structure {change}"
            );
            assert!(
                merge(&base, &structural, &editing).is_err(),
                "reverse {change}"
            );
        }
    }

    #[test]
    fn deletion_and_colliding_new_ids_do_not_discard_another_edit() {
        let base = song();
        let mut deleted = base.clone();
        deleted.delete_channel(1);
        let mut edited = base.clone();
        edited.channels[1].name = "new name".into();
        assert!(merge(&base, &edited, &deleted).is_err());
        let mut a = base.clone();
        let mut b = base.clone();
        a.add_channel(ChannelKind::AudioClip);
        b.add_channel(ChannelKind::Drums);
        assert_eq!(a.channels[2].id, b.channels[2].id);
        assert!(merge(&base, &a, &b).is_err());
    }

    #[test]
    fn undo_does_not_rewind_id_allocators_or_another_rack_change() {
        let base = song();
        let mut alice = base.clone();
        alice.channels[1].lanes[0].add_clip(1024, 4.0);
        let allocated = alice.channels[1].lanes[0].last_clip_id;
        let mut bob = alice.clone();
        bob.channels[1].rack.modules[0].params[0] = 0.25;
        let mut undone = merge(&alice, &bob, &base).unwrap();
        assert_eq!(undone.channels[1].rack.modules[0].params[0], 0.25);
        assert!(undone.channels[1].lanes[0].add_clip(1536, 4.0).id > allocated);
    }

    #[test]
    fn a_combined_edit_cannot_leave_automation_pointing_at_a_removed_module() {
        let base = song();
        let module = base.channels[1].rack.modules[0].instance_id;
        let mut removed = base.clone();
        removed.channels[1].rack.remove(module);
        let mut automation = base.clone();
        automation.channels[1].add_automation_lane(AutomationTarget {
            instance_id: module,
            param: 0,
        });
        assert!(merge(&base, &removed, &automation).is_err());
    }

    #[test]
    fn identical_new_clips_with_the_same_allocated_id_are_still_a_collision() {
        let base = song();
        let mut a = base.clone();
        let mut b = base.clone();
        a.channels[1].lanes[0].add_clip(1024, 4.0);
        b.channels[1].lanes[0].add_clip(1024, 4.0);
        assert!(
            merge(&base, &a, &b).is_err(),
            "two creations must not silently become one"
        );
    }

    #[test]
    fn undo_of_a_track_change_keeps_clip_ids_monotonic() {
        let base = song();
        let mut changed = base.clone();
        let old_id = changed.channels[1].lanes[0].add_clip(1024, 4.0).id;
        changed.channels[1].add_lane(LaneKind::Audio);
        let mut undone = merge(&changed, &changed, &base).unwrap();
        assert!(undone.channels[1].lanes[0].add_clip(2048, 4.0).id > old_id);
    }

    fn similar_tracks() -> Song {
        let mut s = song();
        let mut second = s.channels[1].lanes[0].clone();
        second.clips[1].start_tick = 2048;
        s.channels[1].lanes.push(second);
        s
    }

    #[test]
    fn reorder_and_rename_cannot_redirect_an_older_clip_edit() {
        let base = similar_tracks();
        let mut reordered = base.clone();
        reordered.channels[1].lanes.swap(0, 1);
        reordered.channels[1].lanes[0].file.custom_label = "B".into();
        reordered.channels[1].lanes[1].file.custom_label = "A".into();
        let mut edited = base.clone();
        edited.channels[1].lanes[0].clips[0].start_tick = 128;
        assert!(merge(&base, &reordered, &edited).is_err());
        assert!(merge(&base, &edited, &reordered).is_err());
    }

    #[test]
    fn similar_tracks_restore_views_only_with_one_to_one_matches() {
        let mut base = similar_tracks();
        base.channels[1].lanes[0].clips[0].file.zoom = [3.0; 4];
        base.channels[1].lanes[1].clips[0].file.zoom = [7.0; 4];
        let mut reordered = shared(&base);
        reordered.channels[1].lanes.swap(0, 1);
        let out = merge(&base, &base, &reordered).unwrap();
        assert_eq!(out.channels[1].lanes[0].clips[0].file.zoom, [7.0; 4]);
        assert_eq!(out.channels[1].lanes[1].clips[0].file.zoom, [3.0; 4]);

        base.channels[1].lanes.pop();
        let mut added = shared(&base);
        added.channels[1].add_lane(LaneKind::Notes);
        added.channels[1].lanes[1].add_clip(2048, 4.0);
        let out = merge(&base, &base, &added).unwrap();
        assert_eq!(out.channels[1].lanes[0].clips[0].file.zoom, [3.0; 4]);
        assert_eq!(
            out.channels[1].lanes[1].clips[0].file.zoom,
            ClipFileState::default().zoom
        );
        added.channels[1].lanes[1] = added.channels[1].lanes[0].clone();
        let out = merge(&base, &base, &added).unwrap();
        assert!(
            out.channels[1]
                .lanes
                .iter()
                .all(|l| l.clips[0].file.zoom == ClipFileState::default().zoom)
        );
    }

    #[test]
    fn undone_additions_do_not_block_older_undo() {
        let mut s0 = song();
        s0.add_channel(ChannelKind::Drums);
        let mut s1 = s0.clone();
        s1.channels[1].rack.modules[0].params[0] = 0.2;
        s1.channels[2].drum_kit.as_mut().unwrap().slots[0].name = "Kick 2".into();
        let mut s2 = s1.clone();
        s2.channels[1].rack.add(32);
        s2.channels[1].lanes.push(Lane::new(LaneKind::Notes));
        let kit = s2.channels[2].drum_kit.as_mut().unwrap();
        kit.insert_slot(0, SamplerSlot::default());
        let undone = merge(&s2, &s2, &s1).unwrap();
        let older = merge(&s1, &undone, &s0).expect("the undone additions cancel out");
        assert_eq!(older.channels[1].rack.modules, s0.channels[1].rack.modules);
        assert_eq!(
            older.channels[2].drum_kit.as_ref().unwrap().slots,
            s0.channels[2].drum_kit.as_ref().unwrap().slots
        );
        assert_eq!(
            older.channels[1].rack.last_instance_id,
            s2.channels[1].rack.last_instance_id
        );
    }

    #[test]
    fn pad_structure_changes_conflict_with_concurrent_hits_and_their_undo() {
        let mut base = Song::default();
        base.add_channel(ChannelKind::Drums);
        base.channels[1].lanes[0].add_clip(0, 4.0);
        let mut hit = base.clone();
        hit.channels[1].lanes[0].clips[0].events.push(Event::Drum {
            pos: 0,
            len_beats: 1.0,
            pad: 1,
            velocity: 204,
            pan: 127,
            value: 0x7fff,
            repeat: 0,
        });
        for change in 0..3 {
            let mut pads = base.clone();
            let kit = pads.channels[1].drum_kit.as_mut().unwrap();
            match change {
                0 => {
                    kit.slots.remove(0);
                }
                1 => {
                    kit.insert_slot(0, SamplerSlot::default());
                }
                _ => kit.slots.swap(0, 1),
            }
            assert!(merge(&base, &pads, &hit).is_err());
            assert!(merge(&base, &hit, &pads).is_err());
            let mut later_hit = pads.clone();
            later_hit.channels[1].lanes[0] = hit.channels[1].lanes[0].clone();
            assert!(
                merge(&pads, &later_hit, &base).is_err(),
                "pad Undo cannot redirect a later hit"
            );
        }
    }

    #[test]
    fn concurrent_routes_to_a_removed_channel_are_rejected() {
        let mut base = Song::default();
        base.add_channel(ChannelKind::Drums);
        base.add_channel(ChannelKind::EffectTrack);
        let target = base.channels[2].id;
        let mut deleted = base.clone();
        deleted.delete_channel(2);
        for route in 0..3 {
            let mut edited = base.clone();
            match route {
                0 => edited.channels[1].file.sends.push(SendState {
                    target_channel_id: target,
                    ..SendState::default()
                }),
                1 => {
                    edited.channels[1].drum_kit.as_mut().unwrap().slots[0]
                        .file
                        .send_target = target
                }
                _ => {
                    edited.channels[1].add_automation_lane(AutomationTarget {
                        instance_id: 0,
                        param: 0x2000 + target,
                    });
                }
            }
            assert!(merge(&base, &deleted, &edited).is_err(), "route {route}");
            assert!(
                merge(&base, &edited, &deleted).is_err(),
                "reverse route {route}"
            );
        }
    }

    #[test]
    fn individually_valid_sends_cannot_merge_into_a_cycle() {
        let mut base = Song::default();
        base.add_channel(ChannelKind::EffectTrack);
        base.add_channel(ChannelKind::EffectTrack);
        let mut a = base.clone();
        let mut b = base.clone();
        a.channels[1].file.sends.push(SendState {
            target_channel_id: base.channels[2].id,
            ..SendState::default()
        });
        b.channels[2].file.sends.push(SendState {
            target_channel_id: base.channels[1].id,
            ..SendState::default()
        });
        assert!(merge(&base, &base, &a).is_ok());
        assert!(merge(&base, &base, &b).is_ok());
        assert!(merge(&base, &a, &b).is_err());
        assert!(merge(&base, &b, &a).is_err());
    }
}

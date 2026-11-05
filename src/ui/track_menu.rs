//! The playlist track header menu, opened by a long press (or secondary click) on a track header: column 0 the
//! channel and track items, column 1 the items of the channel's primary track kind. The app carries the items out
//! (`app/tracks.rs`).

use crate::song::{ChannelKind, LaneKind, Song};
use crate::ui::popup::PopupItem;

/// The menu's items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackOp {
    /// `Show Tracks` / `Hide tracks` (zipping the sub-tracks inside the channel).
    ToggleZip,
    /// `Hide track` / `Show track`: hides the whole track (the channel's row goes, with Settings `Show hidden tracks`
    /// it shows dimmed).
    ToggleHide,
    ChannelSettings,
    CloneChannel,
    InsertChannel,
    DeleteTrack,
    DeleteChannel,
    /// `Import MIDI Tracks`: a MIDI file into the channel, or through the MIDI import dialog.
    ImportMidi,
    /// `Save as MIDI`: the channel alone as a MIDI file.
    SaveMidi,
    /// `Burn Notes to Track`: the channel's notes as its note effects play them, on a new notes track.
    BurnNotes,
    /// A track of the primary track's kind.
    AddTrack,
    /// The `Drumsets` picker over `My Drumsets`, whose pick replaces the channel's kit.
    LoadDrumset,
    /// The `Save Drumset` dialog, writing the channel's kit to `My Drumsets`.
    SaveDrumset,
    AddStepSeqTrack,
    AddNotesTrack,
    /// `Add MIDI CC track…`: the MIDI CC picker, whose pick is an automation lane of that CC on the channel.
    AddCcTrack,
    /// `Add Pitch Bend track`: an automation lane of the channel's Pitch Bend (#59).
    AddPitchTrack,
    /// Disabled: resyncing the audio clips is not implemented.
    SyncAllSamples,
}

/// The menu for track `lane` of channel `channel`. The primary track is the channel's lane 0.
/// - Column 0: `Show Tracks` while the sub-tracks are hidden, else `Hide tracks`, enabled with more than one track
///   (hides the tracks inside the channel); `Hide track` while the track shows, else `Show track` (hides the whole
///   channel's row); `Channel Settings`; `Clone Channel` and `Insert Channel` (not on the master); `Delete Track`,
///   enabled when the channel has more than one track of the primary's kind or this track's kind differs from it;
///   then `Delete Channel` (not on the master), last (#68). The master's menu is the zip and hide items (disabled on
///   one track), `Channel Settings` and `Delete Track`.
/// - Column 1, generator channels only (not the master, not a send), by the primary track's kind: notes
///   `Import MIDI Tracks`, `Save as MIDI`, `Burn Notes to Track`, `Add Track`; audio `Add Track`, `Sync All Samples`; drums `Load Drumset`,
///   `Save Drumset`, `Add StepSeq Track`, `Add Notes Track`, `Save as MIDI`. `Sync All Samples` is disabled (see
///   [`TrackOp`]). Notes and drums then have `Add MIDI CC track…` (issue #19); all three end with
///   `Add Pitch Bend track` (issue #59).
///
/// Empty when there is no such track.
pub fn track_menu(song: &Song, channel: usize, lane: usize) -> Vec<PopupItem<TrackOp>> {
    use TrackOp::*;
    let Some(ch) = song.channels.get(channel) else {
        return Vec::new();
    };
    let (Some(primary), Some(this)) = (ch.lanes.first(), ch.lanes.get(lane)) else {
        return Vec::new();
    };
    let primary = primary.kind;
    let master = ch.kind == ChannelKind::Master;
    let of_primary = ch.lanes.iter().filter(|l| l.kind == primary).count();
    let zip = if ch.zipped {
        "Show Tracks"
    } else {
        "Hide tracks"
    };
    let hide = if this.hidden { "Show track" } else { "Hide track" };
    let channel_items = [
        Some((ToggleZip, zip, ch.lanes.len() > 1)),
        Some((ToggleHide, hide, true)),
        Some((ChannelSettings, "Channel Settings", true)),
        (!master).then_some((CloneChannel, "Clone Channel", true)),
        (!master).then_some((InsertChannel, "Insert Channel", true)),
        Some((
            DeleteTrack,
            "Delete Track",
            of_primary > 1 || this.kind != primary,
        )),
        (!master).then_some((DeleteChannel, "Delete Channel", true)),
    ];
    let generator = !matches!(ch.kind, ChannelKind::Master | ChannelKind::EffectTrack);
    let kind_items: &[(TrackOp, &'static str, bool)] = match primary {
        _ if !generator => &[],
        LaneKind::Notes => &[
            (ImportMidi, "Import MIDI Tracks", true),
            (SaveMidi, "Save as MIDI", true),
            (BurnNotes, "Burn Notes to Track", true),
            (AddTrack, "Add Track", true),
            (AddCcTrack, "Add MIDI CC track…", true),
            (AddPitchTrack, "Add Pitch Bend track", true),
        ],
        LaneKind::Audio => &[
            (AddTrack, "Add Track", true),
            (SyncAllSamples, "Sync All Samples", false),
            (AddPitchTrack, "Add Pitch Bend track", true),
        ],
        LaneKind::Drums => &[
            (LoadDrumset, "Load Drumset", true),
            (SaveDrumset, "Save Drumset", true),
            (AddStepSeqTrack, "Add StepSeq Track", true),
            (AddNotesTrack, "Add Notes Track", true),
            (SaveMidi, "Save as MIDI", true),
            (AddCcTrack, "Add MIDI CC track…", true),
            (AddPitchTrack, "Add Pitch Bend track", true),
        ],
        LaneKind::Automation => &[],
    };
    let column = |col: usize, items: Vec<(TrackOp, &'static str, bool)>| {
        items
            .into_iter()
            .enumerate()
            .map(move |(row, (op, label, enabled))| PopupItem {
                disabled: !enabled,
                ..PopupItem::new(op, col, row, label)
            })
    };
    column(0, channel_items.into_iter().flatten().collect())
        .chain(column(1, kind_items.to_vec()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{AutomationTarget, SynthKind};

    use TrackOp::*;

    /// The items as (column, label, enabled), column by column.
    fn menu(song: &Song, channel: usize, lane: usize) -> Vec<(usize, &'static str, bool)> {
        let items = track_menu(song, channel, lane);
        // Rows count up within each column.
        for col in 0..2 {
            let rows: Vec<usize> = (items.iter().filter(|i| i.col == col))
                .map(|i| i.row)
                .collect();
            assert_eq!(rows, (0..rows.len()).collect::<Vec<_>>());
        }
        let label = |i: &PopupItem<TrackOp>| match i.label {
            std::borrow::Cow::Borrowed(s) => s,
            _ => panic!("owned label"),
        };
        (items.iter())
            .map(|i| (i.col, label(i), !i.disabled))
            .collect()
    }

    /// Master (0), MiniSynth (1), Drums (2), Audio Clip (3), Effect Track (4), Instrument (5), Audio Record (6),
    /// Rack (7).
    fn song() -> Song {
        let mut song = Song::default();
        for kind in [
            ChannelKind::Synth(SynthKind::MiniSynth),
            ChannelKind::Drums,
            ChannelKind::AudioClip,
            ChannelKind::EffectTrack,
            ChannelKind::Instrument,
            ChannelKind::AudioRecord,
            ChannelKind::Rack,
        ] {
            song.add_channel(kind);
        }
        song
    }

    const CHANNEL: [(usize, &str, bool); 7] = [
        (0, "Hide tracks", false),
        (0, "Hide track", true),
        (0, "Channel Settings", true),
        (0, "Clone Channel", true),
        (0, "Insert Channel", true),
        (0, "Delete Track", false),
        (0, "Delete Channel", true),
    ];

    #[test]
    fn an_instrument_channel_adds_the_midi_items_burn_notes_and_add_track() {
        let song = song();
        for channel in [1, 5, 7] {
            let want: Vec<_> = (CHANNEL.iter().copied())
                .chain([
                    (1, "Import MIDI Tracks", true),
                    (1, "Save as MIDI", true),
                    (1, "Burn Notes to Track", true),
                    (1, "Add Track", true),
                    (1, "Add MIDI CC track…", true),
                    (1, "Add Pitch Bend track", true),
                ])
                .collect();
            assert_eq!(menu(&song, channel, 0), want, "{channel}");
        }
    }

    #[test]
    fn a_drums_channel_adds_the_drumset_items_and_two_track_kinds() {
        let want: Vec<_> = (CHANNEL.iter().copied())
            .chain([
                (1, "Load Drumset", true),
                (1, "Save Drumset", true),
                (1, "Add StepSeq Track", true),
                (1, "Add Notes Track", true),
                (1, "Save as MIDI", true),
                (1, "Add MIDI CC track…", true),
                (1, "Add Pitch Bend track", true),
            ])
            .collect();
        assert_eq!(menu(&song(), 2, 0), want);
    }

    #[test]
    fn an_audio_channel_adds_add_track_and_a_disabled_sync_all_samples() {
        let song = song();
        for channel in [3, 6] {
            let want: Vec<_> = (CHANNEL.iter().copied())
                .chain([
                    (1, "Add Track", true),
                    (1, "Sync All Samples", false),
                    (1, "Add Pitch Bend track", true),
                ])
                .collect();
            assert_eq!(menu(&song, channel, 0), want, "{channel}");
        }
    }

    #[test]
    fn the_master_has_the_zip_and_hide_items_channel_settings_and_delete_track() {
        let song = song();
        assert_eq!(
            menu(&song, 0, 0),
            [CHANNEL[0], CHANNEL[1], CHANNEL[2], CHANNEL[5]],
            "the master"
        );
        assert_eq!(menu(&song, 4, 0), CHANNEL, "the send");
    }

    #[test]
    fn hide_tracks_needs_two_tracks_and_hide_track_reads_show_track_while_hidden() {
        let mut song = song();
        let first = |song: &Song| menu(song, 1, 0)[0];
        let second = |song: &Song| menu(song, 1, 0)[1];
        song.channels[1].add_lane(LaneKind::Notes);
        assert_eq!(first(&song), (0, "Hide tracks", true));
        song.channels[1].zipped = true;
        assert_eq!(first(&song), (0, "Show Tracks", true));
        // A single track: nothing to hide or show.
        song.channels[2].zipped = true;
        assert_eq!(menu(&song, 2, 0)[0], (0, "Show Tracks", false));
        // `Hide track` hides the whole track; while hidden it reads `Show track`.
        assert_eq!(second(&song), (0, "Hide track", true));
        song.channels[1].lanes[0].hidden = true;
        assert_eq!(second(&song), (0, "Show track", true));
    }

    #[test]
    fn delete_track_needs_another_track_of_the_primary_kind_or_another_kind() {
        let mut song = song();
        let delete = |song: &Song, channel, lane| {
            let items = menu(song, channel, lane);
            items
                .iter()
                .find(|i| i.1 == "Delete Track")
                .copied()
                .unwrap()
                .2
        };
        let drums = &mut song.channels[2];
        let notes = drums.add_lane(LaneKind::Notes);
        // A notes track on a drums channel: another kind than the primary's, deletable; the primary is not (the
        // only drums track).
        assert!(delete(&song, 2, notes));
        assert!(!delete(&song, 2, 0));
        let drums = &mut song.channels[2];
        let second = drums.add_lane(LaneKind::Drums);
        assert!(delete(&song, 2, 0));
        assert!(delete(&song, 2, second));
        // An automation track of the master: the same kind as its primary (the Level track), the only other.
        song.channels[0].add_automation_lane(AutomationTarget {
            instance_id: 0,
            param: 1,
        });
        assert!(delete(&song, 0, 0));
        assert!(delete(&song, 0, 1));
        // An automation track of an instrument.
        let synth = &mut song.channels[1];
        synth.add_automation_lane(AutomationTarget {
            instance_id: 0,
            param: 1,
        });
        assert!(delete(&song, 1, 1));
        assert!(!delete(&song, 1, 0));
    }

    #[test]
    fn a_missing_channel_or_track_has_no_menu() {
        let song = song();
        assert!(track_menu(&song, 9, 0).is_empty());
        assert!(track_menu(&song, 1, 1).is_empty());
    }

    #[test]
    fn the_items_return_their_ops() {
        let ops: Vec<TrackOp> = (track_menu(&song(), 2, 0).iter())
            .map(|i| i.id.unwrap())
            .collect();
        assert_eq!(
            ops,
            [
                ToggleZip,
                ToggleHide,
                ChannelSettings,
                CloneChannel,
                InsertChannel,
                DeleteTrack,
                DeleteChannel,
                LoadDrumset,
                SaveDrumset,
                AddStepSeqTrack,
                AddNotesTrack,
                SaveMidi,
                AddCcTrack,
                AddPitchTrack
            ]
        );
    }
}

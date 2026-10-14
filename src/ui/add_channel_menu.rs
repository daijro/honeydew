//! The add-channel popup's contents, opened from the playlist `+`.

use crate::song::{ChannelKind, SynthKind};
use crate::ui::popup::PopupItem;

/// Two columns: `Channel:` with the six channel kinds and `Synth:` with the eight instrument modules.
/// No `dot` flags: content tracking is out of scope.
pub fn add_channel_menu() -> Vec<PopupItem<ChannelKind>> {
    let column = |col: usize, header: &'static str, kinds: &[ChannelKind]| {
        let items = kinds
            .iter()
            .enumerate()
            .map(|(i, &kind)| PopupItem::new(kind, col, i + 1, kind.label()));
        std::iter::once(PopupItem::header(col, 0, header))
            .chain(items)
            .collect::<Vec<_>>()
    };
    let synths = SynthKind::ALL.map(ChannelKind::Synth);
    [
        column(0, "Channel:", &ChannelKind::ADDABLE),
        column(1, "Synth:", &synths),
    ]
    .concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_is_the_spec_table() {
        let menu = add_channel_menu();
        let cells = |headers: bool| -> Vec<(usize, usize, &str)> {
            let mut v: Vec<_> = menu
                .iter()
                .filter(|i| i.is_header() == headers)
                .map(|i| (i.col, i.row, &*i.label))
                .collect();
            v.sort();
            v
        };
        assert_eq!(cells(true), [(0, 0, "Channel:"), (1, 0, "Synth:")]);
        assert_eq!(
            cells(false),
            [
                (0, 1, "Drums"),
                (0, 2, "Instrument"),
                (0, 3, "Audio Clip"),
                (0, 4, "Effect Track (AUX)"),
                (0, 5, "Audio Record"),
                (0, 6, "Rack"),
                (1, 1, "MiniSynth"),
                (1, 2, "DW Sampler"),
                (1, 3, "GMSynth"),
                (1, 4, "Transistor Bass"),
                (1, 5, "SuperSaw"),
                (1, 6, "Slicer"),
                (1, 7, "3x Osc"),
                (1, 8, "SoundFont Player"),
            ]
        );
    }

    #[test]
    fn items_return_their_kind_and_set_no_flags() {
        for item in add_channel_menu() {
            assert!(
                !item.disabled && !item.dot && !item.keep_open,
                "{}",
                item.label
            );
            if let Some(kind) = item.id {
                assert_eq!(kind.label(), item.label);
            }
        }
        let synth = |label| {
            add_channel_menu()
                .into_iter()
                .find(|i| i.label == label)
                .and_then(|i| i.id)
        };
        assert_eq!(
            synth("3x Osc"),
            Some(ChannelKind::Synth(SynthKind::ThreeXOsc))
        );
        assert_eq!(synth("Rack"), Some(ChannelKind::Rack));
    }
}

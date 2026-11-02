//! Each Audio Record channel's input: the `INPUT` menu's pick, kept per song and channel. A map in the app storage
//! holds them, from the song's GUID and the channel's GUID (CHHD@1064) to the device and its port or port
//! pair.
//!
//! One device is open at a time: the input channel's ([`input_channel`]). A channel without a stored input takes the
//! default device's `Input 1`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::input::{InputChoice, InputDevice};
use crate::song::{Channel, ChannelKind, Song};

/// The [`eframe::Storage`] key the inputs are stored under, as one JSON object.
pub const STORAGE_KEY: &str = "honeydew.routingPrefs";

/// An input picked in a channel's `INPUT` menu: the device by name (its place in the list is no lasting id), and its
/// port or stereo pair, 0-based.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub device: String,
    pub first: usize,
    pub second: Option<usize>,
}

impl Route {
    /// The pick `choice` of the menu built from `devices`.
    pub fn picked(choice: InputChoice, devices: &[InputDevice]) -> Option<Self> {
        Some(Self {
            device: devices.get(choice.device)?.name.clone(),
            first: choice.first,
            second: choice.second,
        })
    }

    /// This input as an item of the menu built from `devices`: `None` when its device is not listed.
    pub fn choice(&self, devices: &[InputDevice]) -> Option<InputChoice> {
        Some(InputChoice {
            device: devices.iter().position(|d| d.name == self.device)?,
            first: self.first,
            second: self.second,
        })
    }
}

/// Every channel's stored input, by song and channel GUID.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Routing(BTreeMap<String, Route>);

/// The map key of a channel of a song: both GUIDs in hex.
fn key(song: [u8; 16], channel: [u8; 16]) -> String {
    let hex = |guid: [u8; 16]| guid.map(|b| format!("{b:02x}")).concat();
    format!("{}/{}", hex(song), hex(channel))
}

impl Routing {
    /// The inputs stored in `storage`; none when nothing or garbage is stored.
    pub fn load(storage: &dyn eframe::Storage) -> Self {
        let json = storage.get_string(STORAGE_KEY);
        Self(
            json.and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default(),
        )
    }

    /// Writes the inputs to `storage`. The caller flushes it.
    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        let json = serde_json::to_string(&self.0).expect("plain data always serialises");
        storage.set_string(STORAGE_KEY, json);
    }

    /// The input stored for `channel` of `song`. One picked before the song's first save, while its GUID was still
    /// zero (a save gives it one), counts too.
    pub fn get(&self, song: &Song, channel: &Channel) -> Option<&Route> {
        let mut keys = [song.file.guid, [0; 16]].map(|guid| key(guid, channel.file.guid));
        keys.iter_mut().find_map(|key| self.0.get(key))
    }

    /// Stores `route` as the input of `channel` of `song`.
    pub fn set(&mut self, song: &Song, channel: &Channel, route: Route) {
        self.0.insert(key(song.file.guid, channel.file.guid), route);
    }
}

/// The Audio Record channel whose input picks the device to open (one device at a time): the
/// open rack's channel, else the selected one, else the first. `None` without an Audio Record channel.
pub fn input_channel(song: &Song, rack: Option<usize>, selected: usize) -> Option<usize> {
    let records =
        |&i: &usize| (song.channels.get(i)).is_some_and(|c| c.kind == ChannelKind::AudioRecord);
    (rack
        .into_iter()
        .chain([selected])
        .chain(0..song.channels.len()))
    .find(records)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct Mem(HashMap<String, String>);

    impl eframe::Storage for Mem {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }

        fn flush(&mut self) {}
    }

    fn device(name: &str, channels: u16) -> InputDevice {
        InputDevice {
            name: name.to_owned(),
            channels,
        }
    }

    fn route(device: &str, first: usize, second: Option<usize>) -> Route {
        Route {
            device: device.to_owned(),
            first,
            second,
        }
    }

    /// A saved song with two Audio Record channels (1, 2) and a MiniSynth (3).
    fn song() -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::AudioRecord);
        song.add_channel(ChannelKind::AudioRecord);
        song.add_channel(ChannelKind::Synth(crate::song::SynthKind::MiniSynth));
        song.stamp_save("2026-10-12 10:00:00");
        song
    }

    #[test]
    fn a_pick_is_its_device_by_name_and_finds_its_item_again_in_another_list() {
        let devices = [device("Interface", 4), device("Mic", 1)];
        let pair = InputChoice {
            device: 0,
            first: 2,
            second: Some(3),
        };
        let r = Route::picked(pair, &devices).unwrap();
        assert_eq!(r, route("Interface", 2, Some(3)));
        assert_eq!(r.choice(&devices), Some(pair));
        // The list changed: the device is found by name; gone, the input is no item.
        let moved = [device("USB", 2), device("Interface", 4)];
        assert_eq!(r.choice(&moved), Some(InputChoice { device: 1, ..pair }));
        assert_eq!(r.choice(&[device("Mic", 1)]), None);
        let beyond = InputChoice {
            device: 2,
            first: 0,
            second: None,
        };
        assert_eq!(Route::picked(beyond, &devices), None);
    }

    #[test]
    fn inputs_survive_a_save_and_load_per_song_and_channel() {
        let song = song();
        let (a, b) = (&song.channels[1], &song.channels[2]);
        let mut routing = Routing::default();
        routing.set(&song, a, route("Interface", 0, Some(1)));
        routing.set(&song, b, route("Mic", 0, None));
        let mut mem = Mem::default();
        routing.save(&mut mem);
        let loaded = Routing::load(&mem);
        assert_eq!(loaded, routing);
        assert_eq!(loaded.get(&song, a), Some(&route("Interface", 0, Some(1))));
        assert_eq!(loaded.get(&song, b), Some(&route("Mic", 0, None)));
        // Another song's channel with the same GUID (a copy saved as another song) has its own.
        let mut other = song.clone();
        other.file.guid = [9; 16];
        assert_eq!(loaded.get(&other, &other.channels[1]), None);
        // A new pick replaces the old.
        routing.set(&song, a, route("Mic", 0, None));
        assert_eq!(routing.get(&song, a), Some(&route("Mic", 0, None)));
        // Nothing or garbage stored: none.
        assert_eq!(Routing::load(&Mem::default()), Routing::default());
        let mut garbage = Mem::default();
        eframe::Storage::set_string(&mut garbage, STORAGE_KEY, "[1, 2".into());
        assert_eq!(Routing::load(&garbage), Routing::default());
    }

    #[test]
    fn a_pick_before_the_songs_first_save_holds_after_it() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::AudioRecord);
        assert_eq!(song.file.guid, [0; 16]);
        let mut routing = Routing::default();
        routing.set(&song, &song.channels[1], route("Mic", 0, None));
        song.stamp_save("2026-10-12 10:00:00");
        assert_ne!(song.file.guid, [0; 16]);
        assert_eq!(
            routing.get(&song, &song.channels[1]),
            Some(&route("Mic", 0, None))
        );
        // A pick after the save is the one that counts.
        routing.set(&song, &song.channels[1], route("USB", 1, None));
        assert_eq!(
            routing.get(&song, &song.channels[1]),
            Some(&route("USB", 1, None))
        );
    }

    #[test]
    fn the_input_channel_is_the_racks_else_the_selected_else_the_first_audio_record_one() {
        let song = song();
        assert_eq!(input_channel(&song, Some(2), 1), Some(2));
        // The rack shows another kind of channel: the selected one.
        assert_eq!(input_channel(&song, Some(3), 2), Some(2));
        assert_eq!(input_channel(&song, None, 1), Some(1));
        // Neither is an Audio Record channel: the first.
        assert_eq!(input_channel(&song, Some(3), 3), Some(1));
        assert_eq!(input_channel(&song, None, 9), Some(1));
        assert_eq!(input_channel(&Song::default(), None, 0), None);
    }
}

//! Lemon Studio's reverb and limiter as rack modules: in the song file, a rack preset and an FL Studio project. The
//! Dynamic EQ, Honeydew's own effect, is saved as they are.

use honeydew::content::Factory;
use honeydew::sample_cache::SampleCache;
use honeydew::song::{ChannelKind, ModuleInstance, Song};
use honeydew_audio::dynamic_eq::{self, band};
use honeydew_audio::lemon::{self, dynamics, norm, reverb};

/// A song with a Lemon Reverb between two of the song format's own effects and a Lemon Limiter on the master, off
/// their defaults.
fn song() -> Song {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let rack = &mut song.channels[1].rack;
    rack.add(9);
    let id = rack.add(lemon::REVERB);
    rack.add(11);
    let p = &mut rack.get_mut(id).unwrap().params;
    for (k, x) in [
        (reverb::DECAY, 45),
        (reverb::LOW_CUT, 300),
        (reverb::SEPARATION, -32),
    ] {
        p[1 + k] = norm(reverb::RANGES[k], x);
    }
    p[lemon::REVERB_TEMPO] = 1.0;
    let rack = &mut song.channels[0].rack;
    let id = rack.add(lemon::LIMITER);
    let p = &mut rack.get_mut(id).unwrap().params;
    for (k, x) in [
        (dynamics::GAIN, 1500),
        (dynamics::COMP_THRES, 600),
        (dynamics::COMP_RATIO, -300),
    ] {
        p[1 + k] = norm(dynamics::RANGES[k], x);
    }
    p[lemon::LIMITER_SAFETY] = 0.0;
    song
}

fn lemon_modules<'a>(
    modules: impl IntoIterator<Item = &'a ModuleInstance>,
) -> Vec<(u16, Vec<f32>)> {
    (modules.into_iter())
        .filter(|m| matches!(m.type_id, lemon::REVERB | lemon::LIMITER))
        .map(|m| (m.type_id, m.params.clone()))
        .collect()
}

#[test]
fn a_saved_song_and_a_rack_preset_keep_them_where_fl_studio_mobile_skips_them() {
    let mut song = song();
    // And a Dynamic EQ after them, off its defaults.
    let rack = &mut song.channels[1].rack;
    let id = rack.add(dynamic_eq::DYNAMIC_EQ);
    let p = &mut rack.get_mut(id).unwrap().params;
    (p[band(1, dynamic_eq::GAIN)], p[band(2, dynamic_eq::RANGE)]) = (0.8, 0.1);
    (p[band(3, dynamic_eq::ON)], p[dynamic_eq::RELEASE]) = (0.0, 0.3);
    let bytes = honeydew::flm::write(&song).unwrap();
    // Each one's `RMOd` is inside an `HDPm`, which a reader that does not know its type skips, never the rack's own.
    let rmods: Vec<usize> = (bytes.windows(4).enumerate())
        .filter(|&(i, w)| {
            w == b"RMOd" && u32::from_le_bytes(bytes[i + 8..i + 12].try_into().unwrap()) >= 1000
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(rmods.len(), 3);
    assert!(rmods.iter().all(|&i| &bytes[i - 16..i - 12] == b"HDPm"));
    // Read back, and written again byte for byte (as a live session's checkpoints require).
    let back = honeydew::flm::read(&bytes).unwrap();
    assert_eq!(back, song);
    assert_eq!(honeydew::flm::write(&back).unwrap(), bytes);
    // A rack preset: the modules in their order, with their params.
    let modules = &song.channels[1].rack.modules;
    let preset = honeydew::flm::write_rack(modules).unwrap();
    let back = honeydew::flm::read_rack(&preset, &mut 1).unwrap();
    let chain = |m: &[ModuleInstance]| {
        m.iter()
            .map(|m| (m.type_id, m.params.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(chain(&back), chain(modules));
}

#[test]
fn an_flp_carries_them_as_fruity_reeverb_2_and_fruity_limiter_with_their_settings() {
    // A fresh instance's saved states are Lemon Studio's (Lemon Limiter's 169-byte layout, version 7).
    let fresh = lemon::limiter_fl_state(&lemon::defaults(lemon::LIMITER));
    assert_eq!((fresh.len(), &fresh[..4]), (169, &7i32.to_le_bytes()[..]));
    let field = |k: usize| i32::from_le_bytes(fresh[4 * k..4 * k + 4].try_into().unwrap());
    assert_eq!(
        (1..=dynamics::PARAMS).map(field).collect::<Vec<_>>(),
        dynamics::DEFAULTS
    );
    assert_eq!(
        lemon::reverb_fl_state(&lemon::defaults(lemon::REVERB)),
        reverb::Settings::default().to_state()
    );
    let song = song();
    let exported = honeydew::flp::export::export(&song, &SampleCache::default(), &[]);
    assert!(
        exported.notices.iter().all(|n| !n.contains("Lemon")),
        "{:?}",
        exported.notices
    );
    let imported = honeydew::flp::import::import(&exported.flp, &Factory::new(Vec::new())).unwrap();
    let sent = lemon_modules(song.channels.iter().flat_map(|c| &c.rack.modules));
    let came = lemon_modules(imported.song.channels.iter().flat_map(|c| &c.rack.modules));
    assert_eq!(came, sent);
    assert!(
        imported.notices.iter().all(|n| !n.contains("Lemon")),
        "{:?}",
        imported.notices
    );
}

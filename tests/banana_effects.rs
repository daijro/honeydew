//! Banana Studio's reverb and limiter as rack modules: in the song file, a rack preset and an FL Studio project. The
//! Dynamic EQ, De-esser, Denoise and Half Time, BanaForge's own effects, are saved as they are.

use banaforge::content::Factory;
use banaforge::sample_cache::SampleCache;
use banaforge::song::{ChannelKind, ModuleInstance, Song};
use banaforge_audio::banana::{self, dynamics, norm, reverb};
use banaforge_audio::dynamic_eq::{self, band};
use banaforge_audio::{de_esser, denoise, half_time};

/// A song with a Banana Reverb between two of the song format's own effects and a Banana Limiter on the master, off
/// their defaults.
fn song() -> Song {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let rack = &mut song.channels[1].rack;
    rack.add(9);
    let id = rack.add(banana::REVERB);
    rack.add(11);
    let p = &mut rack.get_mut(id).unwrap().params;
    for (k, x) in [
        (reverb::DECAY, 45),
        (reverb::LOW_CUT, 300),
        (reverb::SEPARATION, -32),
    ] {
        p[1 + k] = norm(reverb::RANGES[k], x);
    }
    p[banana::REVERB_TEMPO] = 1.0;
    let rack = &mut song.channels[0].rack;
    let id = rack.add(banana::LIMITER);
    let p = &mut rack.get_mut(id).unwrap().params;
    for (k, x) in [
        (dynamics::GAIN, 1500),
        (dynamics::COMP_THRES, 600),
        (dynamics::COMP_RATIO, -300),
    ] {
        p[1 + k] = norm(dynamics::RANGES[k], x);
    }
    p[banana::LIMITER_SAFETY] = 0.0;
    song
}

fn banana_modules<'a>(
    modules: impl IntoIterator<Item = &'a ModuleInstance>,
) -> Vec<(u16, Vec<f32>)> {
    (modules.into_iter())
        .filter(|m| matches!(m.type_id, banana::REVERB | banana::LIMITER))
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
    // And a De-esser and Denoise.
    let id = rack.add(de_esser::DE_ESSER);
    rack.get_mut(id).unwrap().params[de_esser::MODE] = 1.0;
    let id = rack.add(denoise::DENOISE);
    rack.get_mut(id).unwrap().params[denoise::REDUCTION] = 0.6;
    // And a Half Time at 1/16 bar.
    let id = rack.add(half_time::HALF_TIME);
    rack.get_mut(id).unwrap().params[half_time::LENGTH] = 0.0;
    let bytes = banaforge::flm::write(&song).unwrap();
    // Each one's `RMOd` is inside an `HDPm`, which a reader that does not know its type skips, never the rack's own.
    let rmods: Vec<usize> = (bytes.windows(4).enumerate())
        .filter(|&(i, w)| {
            w == b"RMOd" && u32::from_le_bytes(bytes[i + 8..i + 12].try_into().unwrap()) >= 1000
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(rmods.len(), 6);
    assert!(rmods.iter().all(|&i| &bytes[i - 16..i - 12] == b"HDPm"));
    // Read back, and written again byte for byte (as a live session's checkpoints require).
    let back = banaforge::flm::read(&bytes).unwrap();
    assert_eq!(back, song);
    assert_eq!(banaforge::flm::write(&back).unwrap(), bytes);
    // A rack preset: the modules in their order, with their params.
    let modules = &song.channels[1].rack.modules;
    let preset = banaforge::flm::write_rack(modules).unwrap();
    let back = banaforge::flm::read_rack(&preset, &mut 1).unwrap();
    let chain = |m: &[ModuleInstance]| {
        m.iter()
            .map(|m| (m.type_id, m.params.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(chain(&back), chain(modules));
}

/// A Half Time saved before 0.5.4 holds three params, its Length in param 1: a song and a rack preset load it at that
/// Length, at Speed 1/2 and Smooth 10 ms, as it played (#122).
#[test]
fn an_older_half_time_loads_as_it_played() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let rack = &mut song.channels[1].rack;
    let id = rack.add(half_time::HALF_TIME);
    // 1/2 bar and Mix 70 %, as 0.5.3 saved them.
    rack.get_mut(id).unwrap().params = vec![1.0, 1.0 / 3.0, 0.7];
    // 1/2 bar is item 3 of 1/16 bar … 8 bars.
    let mut want = half_time::DEFAULTS;
    want[..3].copy_from_slice(&[1.0, 1.0 / 3.0, 0.7]);
    want[half_time::LENGTH] = 3.0 / 7.0;
    let back = banaforge::flm::read(&banaforge::flm::write(&song).unwrap()).unwrap();
    assert_eq!(back.channels[1].rack.modules[0].params, want);
    let preset = banaforge::flm::write_rack(&song.channels[1].rack.modules).unwrap();
    let back = banaforge::flm::read_rack(&preset, &mut 1).unwrap();
    assert_eq!(back[0].params, want);
}

#[test]
fn an_flp_carries_them_as_fruity_reeverb_2_and_fruity_limiter_with_their_settings() {
    // A fresh instance's saved states are Banana Studio's (Banana Limiter's 169-byte layout, version 7).
    let fresh = banana::limiter_fl_state(&banana::defaults(banana::LIMITER));
    assert_eq!((fresh.len(), &fresh[..4]), (169, &7i32.to_le_bytes()[..]));
    let field = |k: usize| i32::from_le_bytes(fresh[4 * k..4 * k + 4].try_into().unwrap());
    assert_eq!(
        (1..=dynamics::PARAMS).map(field).collect::<Vec<_>>(),
        dynamics::DEFAULTS
    );
    assert_eq!(
        banana::reverb_fl_state(&banana::defaults(banana::REVERB)),
        reverb::Settings::default().to_state()
    );
    let song = song();
    let exported = banaforge::flp::export::export(&song, &SampleCache::default(), &[]);
    assert!(
        exported.notices.iter().all(|n| !n.contains("Banana")),
        "{:?}",
        exported.notices
    );
    let imported =
        banaforge::flp::import::import(&exported.flp, &Factory::new(Vec::new())).unwrap();
    let sent = banana_modules(song.channels.iter().flat_map(|c| &c.rack.modules));
    let came = banana_modules(imported.song.channels.iter().flat_map(|c| &c.rack.modules));
    assert_eq!(came, sent);
    assert!(
        imported.notices.iter().all(|n| !n.contains("Banana")),
        "{:?}",
        imported.notices
    );
}

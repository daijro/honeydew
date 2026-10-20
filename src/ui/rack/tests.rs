use eframe::egui::{
    self, Context, Event, Key, Modifiers, MouseWheelUnit, PointerButton, RawInput, pos2, vec2,
};

use super::*;
use crate::metrics::DEFAULT_UI_SCALE;
use crate::params::module_spec;
use crate::song::{AutomationTarget, ChannelKind, Raw, SendState, SynthKind};
use crate::theme::theme_by_id;
use crate::ui::ctrl_menu::{CtrlAction, Touch};
use crate::ui::offscreen::key_tap;
use honeydew_audio::instrument::KEY_LEARN;

// ---- the height table ----

/// One heading per module: its name, type id and card height (UIHeight).
const HEADINGS: [&str; 39] = [
    r#"### 2.1 Equalizer (ID 3, effect) — UIHeight 10 U"#,
    r#"### 2.2 MultiFX (ID 4, effect) — UIHeight 9 U"#,
    r#"### 2.3 Limiter (legacy) (ID 5, effect) — UIHeight 6 U"#,
    r#"### 2.4 Distortion (ID 6, effect) — UIHeight 5.5 U"#,
    r#"### 2.5 Graphic EQ (ID 7, effect) — UIHeight 10 U"#,
    r#"### 2.6 Compressor (ID 8, effect) — UIHeight 7.5 U"#,
    r#"### 2.7 Filter (ID 9, effect) — UIHeight 6 U"#,
    r#"### 2.8 Autoduck (legacy) (ID 10, effect) — UIHeight 5.5 U"#,
    r#"### 2.9 Chorus (ID 11, effect) — UIHeight 5.5 U"#,
    r#"### 2.10 Reverb (legacy) (ID 12, effect) — UIHeight 7 U"#,
    r#"### 2.11 Flanger (ID 13, effect) — UIHeight 8 U"#,
    r#"### 2.12 Phaser (ID 14, effect) — UIHeight 8 U"#,
    r#"### 2.13 Stereoizer (ID 15, effect) — UIHeight 5.5 U"#,
    r#"### 2.14 Trance Delay (ID 16, effect) — UIHeight 7 U"#,
    r#"### 2.15 Tape Delay (ID 18, effect) — UIHeight 5.5 U"#,
    r#"### 2.16 Leveller (ID 20, effect) — UIHeight 5 U"#,
    r#"### 2.17 Tuner (ID 24, effect) — UIHeight 7 U"#,
    r#"### 2.18 Pitcher (ID 25, effect) — UIHeight 8 U"#,
    r#"### 2.19 Analyzer (ID 26, effect) — UIHeight 10 U"#,
    r#"### 2.20 Gate (ID 27, effect) — UIHeight 8 U"#,
    r#"### 2.21 Limiter (ID 28, effect) — UIHeight 6 U"#,
    r#"### 2.22 Autoduck (ID 29, effect) — UIHeight 6.5 U"#,
    r#"### 2.23 Tuned EQ (ID 30, effect) — UIHeight 10 U"#,
    r#"### 2.24 Reverb (ID 32, effect) — UIHeight 7 U"#,
    r#"### 2.25 Spacer (ID 34, effect) — UIHeight 7 U"#,
    r#"### 2.26 Reverb 2 (ID 35, effect) — UIHeight 9 U"#,
    r#"### 2.27 Multiband Compressor (ID 36, effect) — UIHeight 9 U"#,
    r#"### 2.28 Comb Filter (ID 40, effect) — UIHeight 7 U"#,
    r#"### 2.29 Waveshaper (ID 41, effect) — UIHeight 11 U"#,
    r#"### 2.30 Tape Stop (ID 42, effect) — UIHeight 8.4 U"#,
    r#"### 2.31 Wow & Flutter (ID 43, effect) — UIHeight 9 U"#,
    r#"### 2.32 Spreader (ID 44, effect) — UIHeight 8 U"#,
    r#"### 2.33 Pitcher 2 (ID 45, effect) — UIHeight 8 U"#,
    r#"### 2.34 Tremolo (ID 46, effect) — UIHeight 9 U"#,
    r#"### 2.35 Note Echo (ID 47, note effect) — UIHeight 11 U"#,
    r#"### 2.36 Transpose (ID 48, note effect) — UIHeight 6 U"#,
    r#"### 2.37 Randomizer (ID 49, note effect) — UIHeight 6 U"#,
    r#"### 2.38 Scale (ID 50, note effect) — UIHeight 6.5 U"#,
    r#"### 2.39 Arpeggiator (ID 51, note effect) — UIHeight 10 U"#,
];

/// The text between `from` and the next `to`.
fn between<'a>(s: &'a str, from: &str, to: &str) -> &'a str {
    let rest = &s[s.find(from).unwrap() + from.len()..];
    &rest[..rest.find(to).unwrap()]
}

#[test]
fn ui_height_follows_the_section_headings() {
    for heading in HEADINGS {
        let id: u16 = between(heading, "(ID ", ",").parse().unwrap();
        let height: f32 = between(heading, "UIHeight ", " U").parse().unwrap();
        assert_eq!(ui_height(id), Some(height), "{heading}");
        // The heading names the module the registry has under that id.
        // `### 2.4 Distortion (ID 6, …`: the words between the section number and `(ID`.
        let name = between(heading, "### ", " (ID").split_once(' ').unwrap().1;
        let name = name.trim_end_matches(" (legacy)");
        assert_eq!(module_spec(id).unwrap().display_name, name, "{heading}");
    }
    // Every registered module has a heading.
    let ids: Vec<u16> = crate::params::modules().map(|m| m.id).collect();
    assert_eq!(ids.len(), HEADINGS.len());
    assert!(ids.iter().all(|&id| ui_height(id).is_some()));
}

#[test]
fn instruments_take_the_card_heights_of_their_table() {
    // By type id.
    let table = [
        (0, "MiniSynth", 8.5),
        (1, "DW Sampler", 7.5),
        (2, "GMSynth", 9.5),
        (19, "Transistor Bass", 7.0),
        (23, "SuperSaw", 9.0),
        (33, "Slicer", 8.5),
        (39, "3x Osc", 10.0),
        (52, "SoundFont Player", 8.5),
    ];
    for (id, name, height) in table {
        assert_eq!(ui_height(id), Some(height), "{name}");
        assert_eq!(module_name(id), name);
    }
    assert_eq!(SynthKind::ALL.map(|s| s.label()), table.map(|(_, n, _)| n));
    assert_eq!(ui_height(17), None);
    assert_eq!(module_name(6), "Distortion");
}

// ---- geometry ----

const U: f32 = 32.0;

#[test]
fn cards_stack_a_card_gap_apart_from_half_a_gap_under_the_header() {
    let rack = Rect::from_min_size(pos2(100.0, 50.0), vec2(640.0, 900.0));
    let f = flow(rack, 0.0, U, false, &[5.5, 1.5, 7.0], true);
    let sends = |f: &Flow| {
        let s = f.sends.clone()?;
        Some((s.header, s.rows, s.add, s.disconnected))
    };
    // x = rack x + 1.5 U, width = rack width − 3 U.
    let col = |y0: f32, y1: f32| Rect::from_min_max(pos2(148.0, y0), pos2(692.0, y1));
    // The header ends at floor(50 + 1.3 U) = 91; the gap is trunc(0.35 U) = 11 px, the first top 91 + 5 = 96; then
    // + height + 11 px each. Edges floor to whole pixels.
    assert_eq!(
        f.cards,
        [col(96.0, 272.0), col(283.0, 331.0), col(342.0, 566.0)]
    );
    // Add Module (1.75 U) follows the last card like another card; SENDS 0.75 U lower, then the send row, 1 px,
    // and Add Send.
    assert_eq!(f.add_module, col(577.0, 633.0));
    assert_eq!(
        sends(&f),
        Some((
            col(657.0, 713.0),
            vec![col(713.0, 769.0)],
            col(770.0, 826.0),
            None
        ))
    );
    assert_eq!(f.bottom, 826.0);

    // Scrolling moves everything up by the offset.
    let scrolled = flow(rack, 30.0, U, false, &[5.5, 1.5, 7.0], true);
    assert_eq!(scrolled.cards[0], col(66.0, 242.0));
    // No cards: Add Module drops 1 U. No sends: the flow ends with it.
    let empty = flow(rack, 0.0, U, false, &[], false);
    assert_eq!(empty.add_module, col(128.0, 184.0));
    assert!(empty.sends.is_none());
    assert_eq!(empty.bottom, 184.0);
}

#[test]
fn a_minimized_card_is_1_5_u_and_others_their_ui_height() {
    let mut module = ModuleInstance::new(6, 2);
    assert_eq!(card_height(&module), 5.5);
    module.minimized = true;
    assert_eq!(card_height(&module), 1.5);
    // A type without a known height shows its header only.
    assert_eq!(card_height(&ModuleInstance::new(17, 3)), 1.5);
}

#[test]
fn the_content_rect_is_uiheight_minus_2_u_minus_1_px_unrounded() {
    let card = Rect::from_min_size(pos2(148.0, 77.0), vec2(544.0, 176.0));
    assert_eq!(
        content_rect(card, 5.5, U),
        Rect::from_min_size(pos2(150.0, 126.0), vec2(540.0, 111.0))
    );
    // U = 33: 3.5 × 33 − 1 = 114.5, not rounded.
    let r = content_rect(card, 5.5, 33.0);
    assert_eq!((r.top(), r.height()), (127.5, 114.5));
}

// ---- the card menu ----

/// Ids 2, 3, 4: Distortion, Chorus, Equalizer.
fn rack() -> Rack {
    let mut rack = Rack::default();
    for t in [6, 11, 3] {
        rack.add(t);
    }
    rack
}

fn order(rack: &Rack) -> Vec<i32> {
    rack.modules.iter().map(|m| m.instance_id).collect()
}

#[test]
fn move_and_delete_change_the_chain() {
    let mut r = rack();
    assert_eq!(apply(&mut r, 3, CardAction::MoveUp), Outcome::ChainChanged);
    assert_eq!(order(&r), [3, 2, 4]);
    assert_eq!(apply(&mut r, 3, CardAction::MoveUp), Outcome::Nothing);
    assert_eq!(
        apply(&mut r, 3, CardAction::MoveDown),
        Outcome::ChainChanged
    );
    assert_eq!(order(&r), [2, 3, 4]);
    assert_eq!(apply(&mut r, 4, CardAction::MoveDown), Outcome::Nothing);
    assert_eq!(apply(&mut r, 3, CardAction::Delete), Outcome::ChainChanged);
    assert_eq!(order(&r), [2, 4]);
    assert_eq!(apply(&mut r, 3, CardAction::Delete), Outcome::Nothing);
}

#[test]
fn minimize_toggles_the_card_only() {
    let mut r = rack();
    assert_eq!(apply(&mut r, 3, CardAction::Minimize), Outcome::Minimized);
    assert!(r.modules[1].minimized);
    assert_eq!(order(&r), [2, 3, 4]);
    apply(&mut r, 3, CardAction::Minimize);
    assert!(!r.modules[1].minimized);
}

#[test]
fn inserts_open_the_browser_at_the_card_or_after_it() {
    let mut r = rack();
    assert_eq!(
        apply(&mut r, 3, CardAction::InsertAbove),
        Outcome::Browse(InsertAt::Above(3))
    );
    assert_eq!(
        apply(&mut r, 3, CardAction::InsertBelow),
        Outcome::Browse(InsertAt::Below(3))
    );
    assert_eq!(apply(&mut r, 9, CardAction::InsertBelow), Outcome::Nothing);
    assert_eq!(r, rack());
}

#[test]
fn save_preset_asks_for_the_preset_dialog_and_leaves_the_rack() {
    let mut r = rack();
    assert_eq!(
        apply(&mut r, 3, CardAction::SavePreset),
        Outcome::SavePreset
    );
    assert_eq!(apply(&mut r, 9, CardAction::SavePreset), Outcome::Nothing);
    assert_eq!(r, rack());
}

#[test]
fn copy_leaves_the_rack_and_replace_opens_the_browser_on_the_card() {
    let mut r = rack();
    assert_eq!(apply(&mut r, 3, CardAction::Copy), Outcome::Copied);
    assert_eq!(
        apply(&mut r, 3, CardAction::Replace),
        Outcome::Browse(InsertAt::Replace(3))
    );
    assert_eq!(r, rack());
    for action in [CardAction::Copy, CardAction::Replace] {
        assert_eq!(apply(&mut r, 9, action), Outcome::Nothing, "{action:?}");
    }
}

#[test]
fn clear_rack_keeps_only_the_instrument() {
    // A synth channel: note effects, effects and a module of an unregistered type (17) go, the instrument stays
    // with its id.
    assert!(
        module_spec(17).is_none() && ui_height(17).is_none(),
        "17 is unregistered"
    );
    let mut r = Rack::default();
    for t in [51, 0, 6, 17, 11] {
        r.add(t);
    }
    assert_eq!(clear(&mut r), Outcome::ChainChanged);
    assert_eq!(order(&r), [3]);
    assert_eq!(r.modules[0].type_id, 0);
    assert_eq!(r.last_instance_id, 6, "ids are never reused");
    assert_eq!(clear(&mut r), Outcome::Nothing);
    // An effects-only channel ends empty.
    let mut r = rack();
    assert_eq!(clear(&mut r), Outcome::ChainChanged);
    assert!(r.modules.is_empty());
    assert_eq!(clear(&mut r), Outcome::Nothing);
}

/// A rack preset's modules of `types`, with the saving rack's ids.
fn preset(types: &[u16]) -> Vec<ModuleInstance> {
    (types.iter().zip(20..))
        .map(|(&t, id)| ModuleInstance::new(t, id))
        .collect()
}

fn types(rack: &Rack) -> Vec<u16> {
    rack.modules.iter().map(|m| m.type_id).collect()
}

/// A channel of `kind` holding `types`.
fn channel(kind: ChannelKind, types: &[u16]) -> Channel {
    let mut channel = Channel::new("Lead".into(), kind, 0.3);
    for &t in types {
        channel.rack.add(t);
    }
    channel
}

#[test]
fn load_rack_preset_replaces_the_modules_and_the_instrument_if_the_preset_has_one() {
    // MiniSynth (2), Arpeggiator (3), Distortion (4).
    let mut ch = channel(ChannelKind::Synth(SynthKind::MiniSynth), &[0, 51, 6]);
    // No instrument in the preset (a "Rack FX"): the channel's stays, the note effect above it, in preset order.
    let loaded = load_rack(&mut ch, &preset(&[47, 11, 3]), None);
    assert_eq!(loaded, (Outcome::ChainChanged, 0));
    assert_eq!(types(&ch.rack), [47, 0, 11, 3]);
    assert_eq!(order(&ch.rack), [5, 2, 6, 7], "fresh ids in preset order");
    // The preset's instrument takes the channel's place.
    let loaded = load_rack(&mut ch, &preset(&[2, 6]), None);
    assert_eq!(loaded, (Outcome::ChainChanged, 0));
    assert_eq!(types(&ch.rack), [2, 6]);
    assert_eq!(order(&ch.rack), [8, 9]);
    assert_eq!(ch.rack.modules[0].params, ModuleInstance::new(2, 0).params);
}

#[test]
fn an_instrument_preset_names_the_channel_only_while_its_name_is_automatic() {
    let synth = ChannelKind::Synth(SynthKind::MiniSynth);
    let named = |name: &str, preset: &str| {
        let mut ch = channel(synth, &[0]);
        ch.id = 7;
        ch.name = name.into();
        ch.file.preset_name = preset.into();
        ch
    };
    let after = |mut ch: Channel, preset: &str| {
        name_channel(&mut ch, preset.into(), &|_| false);
        (ch.name, ch.file.preset_name)
    };
    let pair = |name: &str, preset: &str| (name.to_owned(), preset.to_owned());
    // A new channel's name, numbered or not, and a preset's, follow the preset.
    assert_eq!(after(named("MiniSynth", ""), "Bass"), pair("Bass", "Bass"));
    assert_eq!(
        after(named("MiniSynth 2", ""), "Bass"),
        pair("Bass", "Bass")
    );
    assert_eq!(after(named("Bass", "Bass"), "Lead"), pair("Lead", "Lead"));
    // `Default` gives back the writer's base name only to a name that came from the preset (a clone's numbered).
    assert_eq!(after(named("Bass", "Bass"), ""), pair("MiniSynth 7", ""));
    assert_eq!(after(named("Bass 2", "Bass"), ""), pair("MiniSynth 7", ""));
    assert_eq!(after(named("MiniSynth 2", ""), ""), pair("MiniSynth 2", ""));
    assert_eq!(after(named("Piano 2", "Bass"), ""), pair("Piano 2", ""));
    // Not when another channel has the base name (new channels are numbered `<label> N`, not by id): the name stays,
    // and a save writes it as the custom name.
    let mut ch = named("Bass", "Bass");
    name_channel(&mut ch, String::new(), &|name| name == "MiniSynth 7");
    assert_eq!((ch.name, ch.file.preset_name), pair("Bass", ""));
    // A preset's name may be another channel's too: both are named after the preset.
    let mut ch = named("MiniSynth", "");
    name_channel(&mut ch, "Bass".into(), &|name| name == "Bass");
    assert_eq!((ch.name, ch.file.preset_name), pair("Bass", "Bass"));
    // The base name read is the one given back.
    let mut read = named("Bass", "Bass");
    read.file.names = Raw(Some(("Instrument 3".into(), String::new())));
    assert_eq!(after(read, ""), pair("Instrument 3", ""));
    // A name the user typed stays: one of no rule's, or one the file says was typed (its custom name).
    assert_eq!(after(named("Keys", "Bass"), "Lead"), pair("Keys", "Lead"));
    let mut typed = named("Bass", "Bass");
    typed.file.names = Raw(Some(("Instrument 3".into(), "Bass".into())));
    assert_eq!(after(typed, "Lead"), pair("Bass", "Lead"));
}

/// A song holding `ch` (1) beside a MiniSynth channel named `other` (2).
fn song_with(ch: Channel, other: &str) -> crate::song::Song {
    let mut song = crate::song::Song::default();
    let mut neighbour = channel(ChannelKind::Synth(SynthKind::MiniSynth), &[0]);
    neighbour.name = other.into();
    song.channels.extend([ch, neighbour]);
    song
}

#[test]
fn a_rack_presets_instrument_names_the_channel() {
    let mut bass = ModuleInstance::new(2, 20);
    bass.file.preset_name = "Bass".into();
    let (arp, reverb) = (ModuleInstance::new(51, 21), ModuleInstance::new(32, 22));
    let names = |song: &crate::song::Song| {
        let ch = &song.channels[1];
        (ch.name.clone(), ch.file.preset_name.clone())
    };
    let pair = |name: &str, preset: &str| (name.to_owned(), preset.to_owned());
    // Replacing: the new instrument's preset is the channel's, and so is its automatic name.
    let mut ch = channel(ChannelKind::Synth(SynthKind::MiniSynth), &[0, 6]);
    ch.name = "MiniSynth".into();
    let mut song = song_with(ch, "Keys");
    let modules = [arp.clone(), bass.clone(), reverb.clone()];
    load_rack_into(&mut song, 1, &modules, None);
    assert_eq!(names(&song), pair("Bass", "Bass"));
    // A preset with the instrument at its defaults (`Default`) names none, and gives back the base name…
    song.channels[1].id = 4;
    load_rack_into(&mut song, 1, &preset(&[2]), None);
    assert_eq!(names(&song), pair("MiniSynth 4", ""));
    // … unless another channel has it.
    let mut ch = channel(ChannelKind::Synth(SynthKind::MiniSynth), &[0]);
    (ch.id, ch.name, ch.file.preset_name) = (2, "Bass".into(), "Bass".into());
    let mut song = song_with(ch, "MiniSynth 2");
    load_rack_into(&mut song, 1, &preset(&[2]), None);
    assert_eq!(names(&song), pair("Bass", ""));
    // A preset without one keeps the channel's name and preset.
    let mut ch = channel(ChannelKind::Synth(SynthKind::MiniSynth), &[0]);
    (ch.name, ch.file.preset_name) = ("Keys".into(), "Keys".into());
    let mut song = song_with(ch, "Lead");
    load_rack_into(&mut song, 1, std::slice::from_ref(&reverb), None);
    assert_eq!(names(&song), pair("Keys", "Keys"));
    // Appended from `+` into a Rack channel without one: at the bottom, as `Paste` puts it.
    let mut ch = channel(ChannelKind::Rack, &[51, 6]);
    ch.name = "Rack".into();
    let mut song = song_with(ch, "Lead");
    let end = Some(InsertAt::End);
    let loaded = load_rack_into(&mut song, 1, &[bass, reverb], end);
    assert_eq!(loaded.0, Outcome::ChainChanged);
    assert_eq!(types(&song.channels[1].rack), [51, 6, 2, 32]);
    assert_eq!(song.channels[1].name, "Bass");
}

#[test]
fn an_instrument_the_channel_cannot_take_is_skipped_and_counted() {
    let mut ch = channel(ChannelKind::EffectTrack, &[6]);
    let loaded = load_rack(&mut ch, &preset(&[0, 11]), None);
    assert_eq!(loaded, (Outcome::ChainChanged, 1));
    assert_eq!(types(&ch.rack), [11]);
    // Everything skipped, or nothing to load: no edit.
    let kept = ch.clone();
    assert_eq!(
        load_rack(&mut ch, &preset(&[0, 2]), None),
        (Outcome::Nothing, 2)
    );
    assert_eq!(load_rack(&mut ch, &[], None), (Outcome::Nothing, 0));
    let end = Some(InsertAt::End);
    assert_eq!(load_rack(&mut ch, &[], end), (Outcome::Nothing, 0));
    assert_eq!(ch, kept);
    // A mixed preset: both instruments are skipped and counted, the effect loads.
    assert_eq!(
        load_rack(&mut ch, &preset(&[0, 2, 11]), None),
        (Outcome::ChainChanged, 2)
    );
    assert_eq!(types(&ch.rack), [11]);
}

#[test]
fn a_rack_preset_skips_module_ids_above_52_like_paste_and_counts_them() {
    // Module ids above 52 are skipped, by the same attach routine as `Paste`.
    let end = Some(InsertAt::End);
    let cases: [(_, &[u16]); 3] = [
        (None, &[11]),
        (end, &[6, 11]),
        (Some(InsertAt::Above(2)), &[11, 6]),
    ];
    for (at, expected) in cases {
        let mut ch = channel(ChannelKind::Rack, &[6]);
        assert_eq!(
            load_rack(&mut ch, &preset(&[53, 11, 99]), at),
            (Outcome::ChainChanged, 2),
            "{at:?}"
        );
        assert_eq!(types(&ch.rack), expected, "{at:?}");
    }
    // On a channel that takes no instrument they add up: an instrument, ids 53 and 54, and the effect that loads.
    let mut ch = channel(ChannelKind::EffectTrack, &[6]);
    assert_eq!(
        load_rack(&mut ch, &preset(&[0, 53, 54, 11]), None),
        (Outcome::ChainChanged, 3)
    );
    assert_eq!(types(&ch.rack), [11]);
    // Nothing but ids above 52: no edit, the rack's modules and instrument stay.
    let mut ch = channel(ChannelKind::Synth(SynthKind::MiniSynth), &[0, 6]);
    let kept = ch.clone();
    assert_eq!(
        load_rack(&mut ch, &preset(&[53, 60]), None),
        (Outcome::Nothing, 2)
    );
    assert_eq!(ch, kept);
    // The last id is 52 (SoundFont Player): it loads.
    let mut ch = channel(ChannelKind::Rack, &[]);
    assert_eq!(
        load_rack(&mut ch, &preset(&[52, 53]), end),
        (Outcome::ChainChanged, 1)
    );
    assert_eq!(types(&ch.rack), [52]);
}

#[test]
fn only_a_channel_whose_primary_lane_is_a_notes_lane_takes_instruments() {
    let kinds = [
        (ChannelKind::Instrument, true),
        (ChannelKind::Rack, true),
        (ChannelKind::Synth(SynthKind::MiniSynth), true),
        (ChannelKind::Drums, false),
        (ChannelKind::AudioClip, false),
        (ChannelKind::AudioRecord, false),
        (ChannelKind::EffectTrack, false),
        (ChannelKind::Master, false),
    ];
    let mut cases: Vec<_> = kinds
        .into_iter()
        .map(|(kind, takes)| (channel(kind, &[]), takes))
        .collect();
    // The lane decides, not the kind: a Rack channel without lanes or whose first lane is no notes lane takes none.
    let mut bare = channel(ChannelKind::Rack, &[]);
    bare.lanes.clear();
    let mut automation = channel(ChannelKind::Rack, &[]);
    automation.lanes[0].kind = LaneKind::Automation;
    cases.extend([(bare, false), (automation, false)]);
    let end = Some(InsertAt::End);
    let changes = |ch: &Channel, at| {
        let (outcome, _) = load_rack(&mut ch.clone(), &preset(&[0]), at);
        outcome == Outcome::ChainChanged
    };
    for (ch, takes) in cases {
        let kind = ch.kind;
        assert_eq!(takes_instruments(&ch), takes, "{kind:?}");
        // The three paths agree: the Modules browser's `Paste` and its rack preset, and `Load Rack preset`.
        let pasted = paste(&mut ch.clone(), &preset(&[0]), InsertAt::End) == Outcome::ChainChanged;
        assert_eq!(pasted, takes, "{kind:?}: paste");
        assert_eq!(changes(&ch, end), takes, "{kind:?}: appended preset");
        assert_eq!(changes(&ch, None), takes, "{kind:?}: Load Rack preset");
    }
}

#[test]
fn a_rack_that_takes_instruments_loads_every_one_the_preset_has_beside_its_own() {
    // Appended to a rack that has its instrument: both are kept (no one-instrument rule), the new one under the first.
    let end = Some(InsertAt::End);
    let mut ch = channel(ChannelKind::Synth(SynthKind::MiniSynth), &[0]);
    assert_eq!(
        load_rack(&mut ch, &preset(&[2, 11]), end),
        (Outcome::ChainChanged, 0)
    );
    assert_eq!(types(&ch.rack), [0, 2, 11]);
    // A preset with several: all of them, in preset order.
    let mut ch = channel(ChannelKind::Rack, &[]);
    assert_eq!(
        load_rack(&mut ch, &preset(&[2, 0, 11]), end),
        (Outcome::ChainChanged, 0)
    );
    assert_eq!(types(&ch.rack), [2, 0, 11]);
    assert_eq!(order(&ch.rack), [2, 3, 4]);
    // From `+`, at the bottom as `Paste` puts them, below the effects too.
    let mut ch = channel(ChannelKind::Rack, &[51, 0, 6]);
    load_rack(&mut ch, &preset(&[2, 0]), end);
    assert_eq!(types(&ch.rack), [51, 0, 6, 2, 0]);
    assert_eq!(order(&ch.rack), [2, 3, 4, 5, 6]);
    // `Load Rack preset` puts each under the note effects and the instruments before it, above the effects.
    let mut ch = channel(ChannelKind::Rack, &[51, 6]);
    load_rack(&mut ch, &preset(&[11, 2, 0]), None);
    assert_eq!(types(&ch.rack), [2, 0, 11]);
    // At an insert point, in order from there.
    let mut ch = channel(ChannelKind::Rack, &[0, 6]);
    load_rack(&mut ch, &preset(&[2, 1]), Some(InsertAt::Below(2)));
    assert_eq!(types(&ch.rack), [0, 2, 1, 6]);
    // `Load Rack preset` replaces the rack's modules and its instruments with the preset's, every one.
    let mut ch = channel(ChannelKind::Synth(SynthKind::MiniSynth), &[0, 51, 6]);
    assert_eq!(
        load_rack(&mut ch, &preset(&[1, 11, 2]), None),
        (Outcome::ChainChanged, 0)
    );
    assert_eq!(types(&ch.rack), [1, 2, 11]);
}

#[test]
fn appending_a_rack_preset_keeps_its_order_at_the_insert_point() {
    // Distortion (2), Chorus (3), Equalizer (4).
    let load = |at| {
        let mut ch = channel(ChannelKind::Rack, &[6, 11, 3]);
        assert_eq!(
            load_rack(&mut ch, &preset(&[51, 13, 14]), Some(at)).0,
            Outcome::ChainChanged
        );
        (types(&ch.rack), order(&ch.rack))
    };
    // From `+`, appended at the bottom as `Paste` puts them: the note effect too.
    assert_eq!(
        load(InsertAt::End),
        (vec![6, 11, 3, 51, 13, 14], vec![2, 3, 4, 5, 6, 7])
    );
    assert_eq!(load(InsertAt::Above(3)).0, [6, 51, 13, 14, 11, 3]);
    assert_eq!(load(InsertAt::Below(3)).0, [6, 11, 51, 13, 14, 3]);
    assert_eq!(load(InsertAt::Replace(3)).0, [6, 51, 13, 14, 3]);
}

#[test]
fn the_browsers_rack_preset_goes_where_paste_puts_the_same_modules() {
    // One `+`, one placement. A [MiniSynth, Reverb] preset into [Arpeggiator (2), Distortion (3)] from `+` is appended
    // at the bottom in file order (loading appends).
    let rack = || channel(ChannelKind::Rack, &[51, 6]);
    let modules = preset(&[0, 53, 32]);
    let ats = [
        InsertAt::End,
        InsertAt::Above(3),
        InsertAt::Below(2),
        InsertAt::Replace(3),
    ];
    for at in ats {
        let (mut loaded, mut pasted) = (rack(), rack());
        let outcome = paste(&mut pasted, &modules, at);
        assert_eq!(load_rack(&mut loaded, &modules, Some(at)), (outcome, 1));
        assert_eq!(loaded, pasted, "{at:?}");
    }
    let mut ch = rack();
    load_rack(&mut ch, &modules, Some(InsertAt::End));
    assert_eq!(types(&ch.rack), [51, 6, 0, 32]);
}

/// What `Copy Rack` puts on the clipboard from a synth channel, top to bottom with the ids it had there: Arpeggiator,
/// MiniSynth, Distortion with its Drive at 0.9.
fn copied_rack() -> Vec<ModuleInstance> {
    let mut modules = preset(&[51, 0, 6]);
    modules[2].params[1] = 0.9;
    modules
}

#[test]
fn paste_inserts_every_compatible_module_in_clipboard_order_with_fresh_ids_and_settings() {
    // From `+`, into another notes channel (an instrument, Chorus): appended at the bottom, the note effect too.
    let mut ch = channel(ChannelKind::Rack, &[2, 11]);
    assert_eq!(
        paste(&mut ch, &copied_rack(), InsertAt::End),
        Outcome::ChainChanged
    );
    assert_eq!(types(&ch.rack), [2, 11, 51, 0, 6]);
    assert_eq!(order(&ch.rack), [2, 3, 4, 5, 6]);
    let copy = |i: usize, instance_id| ModuleInstance {
        instance_id,
        ..copied_rack()[i].clone()
    };
    assert_eq!(ch.rack.modules[2..], [copy(0, 4), copy(1, 5), copy(2, 6)]);
    // Into an empty rack, the same order.
    let mut ch = channel(ChannelKind::Rack, &[]);
    paste(&mut ch, &copied_rack(), InsertAt::End);
    assert_eq!(types(&ch.rack), [51, 0, 6]);
    // `Insert above` / `Insert below` (Distortion 2, Chorus 3, Equalizer 4): from there, each below the one before.
    let at = |at| {
        let mut ch = channel(ChannelKind::Rack, &[6, 11, 3]);
        assert_eq!(paste(&mut ch, &copied_rack(), at), Outcome::ChainChanged);
        types(&ch.rack)
    };
    assert_eq!(at(InsertAt::Above(3)), [6, 51, 0, 6, 11, 3]);
    assert_eq!(at(InsertAt::Below(3)), [6, 11, 51, 0, 6, 3]);
    // `Replace`: in that module's place, which goes.
    assert_eq!(at(InsertAt::Replace(3)), [6, 51, 0, 6, 3]);
}

#[test]
fn paste_skips_instruments_off_a_notes_lane_and_ids_above_52() {
    // A drums, audio, effect-track or master channel's primary lane is no notes lane: the MiniSynth is skipped.
    let mut channels = [
        ChannelKind::Drums,
        ChannelKind::AudioClip,
        ChannelKind::AudioRecord,
        ChannelKind::EffectTrack,
    ]
    .map(|kind| channel(kind, &[]))
    .to_vec();
    channels.push(Channel::master());
    for mut ch in channels {
        assert_eq!(
            paste(&mut ch, &copied_rack(), InsertAt::End),
            Outcome::ChainChanged
        );
        assert_eq!(types(&ch.rack), [51, 6], "{:?}", ch.kind);
    }
    // Nothing compatible (an instrument, and 53, past the last module id): nothing changes, and a `Replace` keeps
    // its module.
    let clipboard = preset(&[0, 53]);
    let mut ch = channel(ChannelKind::Drums, &[6]);
    let kept = ch.clone();
    assert_eq!(
        paste(&mut ch, &clipboard, InsertAt::Replace(2)),
        Outcome::Nothing
    );
    assert_eq!(paste(&mut ch, &[], InsertAt::End), Outcome::Nothing);
    assert_eq!(ch, kept);
    // A notes lane takes the instrument, beside its own; 53 never.
    let mut ch = channel(ChannelKind::Instrument, &[2]);
    assert_eq!(
        paste(&mut ch, &clipboard, InsertAt::End),
        Outcome::ChainChanged
    );
    assert_eq!(types(&ch.rack), [2, 0]);
}

#[test]
fn the_rack_menu_enables_load_drumset_load_and_save_rack_preset_clear_rack_and_the_minimize_items()
{
    let flags = |drums| -> Vec<(String, bool)> {
        rack_menu(drums, false)
            .into_iter()
            .map(|i| (i.label.into_owned(), i.disabled))
            .collect()
    };
    let item = |label: &str, disabled| (label.to_owned(), disabled);
    assert_eq!(
        flags(false),
        [
            item("Load Rack preset", false),
            item("Save Rack preset", false),
            item("Copy Rack", false),
            item("Clear Rack", false),
            item("Minimize all", false),
            item("Maximize all", false),
        ]
    );
    assert_eq!(flags(true)[0], item("Load Drumset", false));
    assert_eq!(flags(true)[4], item("Clear Rack", false));
    // An empty rack has no preset to save; it copies (nothing, which `Paste` then says).
    let empty = rack_menu(false, true);
    let disabled = |label: &str| empty.iter().find(|i| i.label == label).unwrap().disabled;
    assert!(disabled("Save Rack preset"));
    assert!(!disabled("Copy Rack"));
}

#[test]
fn the_card_menu_has_two_columns_with_every_item_enabled() {
    let cells = |minimized| -> Vec<(usize, usize, String, bool)> {
        card_menu(minimized, 6, true)
            .into_iter()
            .map(|i| (i.col, i.row, i.label.into_owned(), i.disabled))
            .collect()
    };
    let cell = |col, row, label: &str, disabled| (col, row, label.to_owned(), disabled);
    assert_eq!(
        cells(false),
        [
            cell(0, 0, "Minimize", false),
            cell(0, 1, "Save preset", false),
            cell(0, 2, "Move up", false),
            cell(0, 3, "Move down", false),
            cell(1, 0, "Copy", false),
            cell(1, 1, "Insert above", false),
            cell(1, 2, "Insert below", false),
            cell(1, 3, "Replace", false),
            cell(1, 4, "Delete", false),
        ]
    );
    assert_eq!(cells(true)[0], cell(0, 0, "Maximize", false));
}

// ---- offscreen ----

const SCREEN: Rect = Rect {
    min: pos2(0.0, 0.0),
    max: pos2(1280.0, 800.0),
};

/// The channel's index in the song.
const CHANNEL: usize = 3;

struct Rig {
    ctx: Context,
    /// The input devices the host lists when the `INPUT` menu asks, and the channel's input among them.
    devices: Vec<InputDevice>,
    current: Option<InputChoice>,
    /// The channel's index in the song: [`CHANNEL`] unless a test shows another rack.
    index: usize,
    rack: RackPanel,
    channel: Channel,
    time: f64,
    /// What the last frame returned, every pass merged.
    actions: RackActions,
    shapes: Vec<egui::epaint::ClippedShape>,
    /// When the last frame asked for the next one: `ZERO` at once, `MAX` not until input.
    repaint: std::time::Duration,
    /// Points to device pixels: 1 unless a test sets it.
    ppp: f32,
}

impl Rig {
    fn new(types: &[u16]) -> Self {
        let mut channel = Channel::new("Lead".into(), ChannelKind::Instrument, 0.3);
        // A new channel's one send, to the master (`Song::insert_channel`).
        channel.file.sends.push(SendState::default());
        for &t in types {
            channel.rack.add(t);
        }
        Self::with(channel)
    }

    fn with(channel: Channel) -> Self {
        let mut rig = Self {
            ctx: Context::default(),
            devices: Vec::new(),
            current: None,
            index: CHANNEL,
            rack: RackPanel::default(),
            channel,
            time: 0.0,
            actions: RackActions::default(),
            shapes: Vec::new(),
            repaint: std::time::Duration::MAX,
            ppp: 1.0,
        };
        rig.frame(vec![]);
        rig
    }

    fn frame(&mut self, events: Vec<Event>) {
        // As the host does after the frame that asked.
        if self.actions.list_inputs {
            self.rack.inputs = Some((self.devices.clone(), self.current));
        }
        self.time += 1.0 / 60.0;
        let mut raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let viewport = raw.viewports.entry(raw.viewport_id).or_default();
        viewport.native_pixels_per_point = Some(self.ppp);
        let theme = theme_by_id(0);
        // ppp 1 unless a test sets it, so points are device pixels: the tests' rects are floored pixel edges (`flow`),
        // which move by up to a pixel at a fractional ppp.
        let m = Metrics::from_pixels_per_point(self.ppp, DEFAULT_UI_SCALE);
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let (rack, channel, index) = (&mut self.rack, &mut self.channel, self.index);
        let mut all = RackActions::default();
        let output = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let rect = super::rack_rect(SCREEN, 1.0, false, &m);
                    let a = rack.show(ui, &cx, rect, index, channel);
                    all.params.extend(a.params);
                    all.replace_song |= a.replace_song;
                    all.minimized |= a.minimized;
                    all.list_inputs |= a.list_inputs;
                    all.input = all.input.take().or(a.input);
                    all.settings |= a.settings;
                    all.setting_changes.extend(a.setting_changes);
                    all.open_browser = all.open_browser.or(a.open_browser);
                    all.copy = all.copy.take().or(a.copy);
                    all.preset = all.preset.take().or(a.preset);
                    all.picker = all.picker.or(a.picker);
                    all.soundfont = all.soundfont.or(a.soundfont);
                    all.close |= a.close;
                    all.touch = all.touch.or(a.touch);
                    all.ask_delete = all.ask_delete.or(a.ask_delete);
                    all.create_send = all.create_send.or(a.create_send);
                });
        });
        self.actions = all;
        self.shapes = output.shapes;
        self.repaint = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    }

    fn button(&mut self, pos: Pos2, pressed: bool) {
        self.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }]);
    }

    fn click(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.button(pos, true);
        self.button(pos, false);
    }

    /// Clicks what opens a popup or dialog, and runs one more frame: egui's first frame of a new area only sizes it.
    fn open(&mut self, pos: Pos2) {
        self.click(pos);
        self.frame(vec![]);
    }

    /// Presses at `from`, moves by `by` in four steps and returns the actions of each step; does not release.
    fn drag(&mut self, from: Pos2, by: Vec2) -> Vec<RackActions> {
        self.frame(vec![Event::PointerMoved(from)]);
        self.button(from, true);
        (1..=4)
            .map(|i| {
                self.frame(vec![Event::PointerMoved(from + by * i as f32 / 4.0)]);
                std::mem::take(&mut self.actions)
            })
            .collect()
    }

    /// Where `text` was painted in the last frame.
    fn text(&self, text: &str) -> Rect {
        self.shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(t.galley.rect.translate(t.pos.to_vec2()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{text:?} not drawn"))
    }

    fn drawn(&self, text: &str) -> bool {
        self.shapes
            .iter()
            .any(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text() == text))
    }

    /// The cards (points = px at ppp 1) as the rack lays them out unscrolled.
    fn cards(&self) -> Vec<Rect> {
        let heights: Vec<f32> = self.channel.rack.modules.iter().map(card_height).collect();
        let input = self.channel.kind == ChannelKind::AudioRecord;
        flow(rack_rect(SCREEN), 0.0, U, input, &heights, true).cards
    }

    /// Centre of header item `x` U from the card's left (negative: from its right), mid-row.
    fn header(&self, card: usize, x: f32) -> Pos2 {
        let c = self.cards()[card];
        let x = if x < 0.0 {
            c.right() + x * U
        } else {
            c.left() + x * U
        };
        pos2(x, c.top() + 0.75 * U)
    }

    fn module(&self, id: i32) -> &ModuleInstance {
        self.channel.rack.get(id).unwrap()
    }

    /// One frame `dt` seconds after the last.
    fn after(&mut self, dt: f64, events: Vec<Event>) {
        self.time += dt - 1.0 / 60.0;
        self.frame(events);
    }

    /// The cards as the rack lays them out, scrolled: all minimized while one is held. Without swap slides.
    fn shown(&self) -> Vec<Rect> {
        let modules = self.channel.rack.modules.iter();
        let held = self.rack.drag.is_some();
        let heights: Vec<f32> = modules
            .map(|m| if held { 1.5 } else { card_height(m) })
            .collect();
        let scroll = self.rack.scroll.round();
        flow(rack_rect(SCREEN), scroll, U, false, &heights, true).cards
    }

    fn minimize_all(&mut self) {
        for module in &mut self.channel.rack.modules {
            module.minimized = true;
        }
        self.frame(vec![]);
    }
}

#[test]
fn a_knob_drag_in_a_card_is_written_and_sent_on_every_frame() {
    let mut rig = Rig::new(&[6]);
    let card = rig.cards()[0];
    let content = content_rect(card, 5.5, U);
    let module = rig.module(2).clone();
    let drive = panels::place(
        panels::panel_for(6).unwrap(),
        &module,
        &PanelState::default(),
        content,
        U,
    )
    .into_iter()
    .find(|(l, _)| l.id == 3)
    .unwrap()
    .1;
    let steps = rig.drag(drive.center(), vec2(0.0, -200.0));
    // Drive went from 0.25 up a quarter of the range (200 points), and each step sent where it was.
    assert!((rig.module(2).params[1] - 0.5).abs() < 1e-5);
    for a in &steps {
        let [change] = a.params[..] else {
            panic!("{a:?}")
        };
        assert_eq!(
            (change.channel, change.instance_id, change.index),
            (CHANNEL, 2, 1)
        );
        assert!(!a.replace_song);
    }
    assert_eq!(steps[3].params[0].value, rig.module(2).params[1]);
}

#[test]
fn the_enable_toggle_sends_param_0() {
    let mut rig = Rig::new(&[6]);
    let at = rig.header(0, 0.75);
    rig.click(at);
    assert_eq!(rig.module(2).params[0], 0.0);
    assert_eq!(
        rig.actions.params,
        [ParamChange {
            channel: CHANNEL,
            instance_id: 2,
            index: 0,
            value: 0.0
        }]
    );
    assert!(!rig.actions.replace_song);
}

#[test]
fn the_icon_opens_the_common_dialog_whose_sliders_send_negative_indices() {
    let mut rig = Rig::new(&[6]);
    assert!(!rig.drawn("Wet Pan"));
    let at = rig.header(0, 2.25);
    rig.open(at);
    for label in ["Distortion", "Wet Pan", "Mix", "Post Gain"] {
        assert!(rig.drawn(label), "{label}");
    }
    // Rows top to bottom: title, Wet Pan, Mix, Post Gain.
    assert!(rig.text("Wet Pan").top() < rig.text("Mix").top());
    assert!(rig.text("Mix").top() < rig.text("Post Gain").top());
    // Their value texts.
    for value in ["Centered", "100 %", "0 dB"] {
        assert!(rig.drawn(value), "{value}");
    }
    drag_the_common_params(&mut rig, 2);
    rig.frame(vec![]);
    for value in ["50 % Right", "75 %", "-10 dB"] {
        assert!(rig.drawn(value), "{value}");
    }
    // A click outside closes it.
    rig.click(pos2(20.0, 20.0));
    rig.frame(vec![]);
    assert!(!rig.drawn("Wet Pan"));
}

#[test]
fn a_note_effect_edit_asks_for_a_new_song_when_the_drag_ends() {
    // Transpose (48), a note effect.
    let mut rig = Rig::new(&[48]);
    let at = rig.header(0, 2.25);
    rig.open(at);
    let mix = rig.text("Mix").center();
    let steps = rig.drag(mix, vec2(-U, 0.0));
    assert!(
        steps
            .iter()
            .all(|a| !a.params.is_empty() && !a.replace_song)
    );
    rig.button(mix + vec2(-U, 0.0), false);
    assert!(rig.actions.replace_song);
    rig.frame(vec![]);
    assert!(!rig.actions.replace_song, "only once");
    // A click edit asks on its release.
    let toggle = rig.header(0, 0.75);
    rig.click(pos2(20.0, 20.0));
    rig.click(toggle);
    assert_eq!(rig.actions.params[0].index, 0);
    assert!(rig.actions.replace_song);
}

#[test]
fn the_card_menu_moves_deletes_minimizes_and_inserts() {
    let mut rig = Rig::new(&[6, 11, 3]);
    let pick = |rig: &mut Rig, card: usize, item: &str| {
        let menu = rig.header(card, -0.75);
        rig.open(menu);
        let at = rig.text(item).center();
        rig.click(at);
        rig.frame(vec![]);
        assert!(!rig.drawn("Insert above"), "the menu closes");
    };
    pick(&mut rig, 0, "Move down");
    assert_eq!(order(&rig.channel.rack), [3, 2, 4]);
    // The swap slide comes to rest before the next menu.
    rig.settle();
    pick(&mut rig, 1, "Delete");
    assert_eq!(order(&rig.channel.rack), [3, 4]);
    pick(&mut rig, 0, "Minimize");
    assert!(rig.module(3).minimized);
    assert_eq!(rig.cards()[0].height(), 1.5 * U);
    pick(&mut rig, 1, "Insert below");
    assert_eq!(order(&rig.channel.rack), [3, 4]);
}

// ---- the recent control and `Delete automation?` ----

impl Rig {
    /// Presses `pos` (a move, then the press) and returns that frame's touch; then releases.
    fn press_touch(&mut self, pos: Pos2) -> Option<Touch> {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.button(pos, true);
        let touch = self.actions.touch;
        self.button(pos, false);
        touch
    }

    /// The Distortion card's Drive knob (#3, param 1), at the top of the rack.
    fn drive(&self) -> Pos2 {
        let content = content_rect(self.cards()[0], 5.5, U);
        let panel = panels::panel_for(6).unwrap();
        let placed = panels::place(panel, self.module(2), &PanelState::default(), content, U);
        placed
            .into_iter()
            .find(|(l, _)| l.id == 3)
            .unwrap()
            .1
            .center()
    }
}

fn target(instance_id: i32, param: i32) -> AutomationTarget {
    AutomationTarget { instance_id, param }
}

#[test]
fn presses_on_a_cards_bound_controls_report_their_targets() {
    let mut rig = Rig::new(&[6]);
    let pressed = |target| Some(Touch { target, pick: None });
    let drive = rig.drive();
    assert_eq!(rig.press_touch(drive), pressed(target(2, 1)));
    assert_eq!(rig.actions.touch, None, "the press frame only");
    // The enable toggle is param 0.
    let toggle = rig.header(0, 0.75);
    assert_eq!(rig.press_touch(toggle), pressed(target(2, 0)));
    // The common dialog's sliders: Mix is −2. The icon and the ≡ are no controls.
    let icon = rig.header(0, 2.25);
    assert_eq!(rig.press_touch(icon), None);
    rig.frame(vec![]);
    let mix = rig.text("Mix").center();
    assert_eq!(
        rig.press_touch(mix),
        pressed(target(2, AutomationTarget::MIX))
    );
}

#[test]
fn add_automation_track_from_a_cards_knob_reports_its_target() {
    let mut rig = Rig::new(&[6]);
    let drive = rig.drive();
    rig.secondary_click(drive);
    rig.frame(vec![]);
    let item = rig.text("Add automation track").center();
    rig.click(item);
    let add = Touch {
        target: target(2, 1),
        pick: Some(CtrlAction::AddAutomationTrack),
    };
    assert_eq!(rig.actions.touch, Some(add));
    assert_eq!(rig.module(2).params[1], 0.25, "the value stays");
}

#[test]
fn the_channels_cc_links_enable_unlink_on_a_cards_controls_the_header_toggle_and_the_common_dialog()
{
    let mut channel = Channel::new("Lead".into(), ChannelKind::Instrument, 0.3);
    channel.rack.add(6);
    // The channel links CC 20 to the Distortion's Drive (param 1), its enable (param 0) and its Mix.
    for param in [1, 0, AutomationTarget::MIX] {
        crate::midi::links::link(&mut channel.file.cc_links, 20, 2, param);
    }
    let mut rig = Rig::with(channel);
    let drive = rig.drive();
    assert_eq!(unlink(&mut rig, drive), unlinked(target(2, 1)));
    let toggle = rig.header(0, 0.75);
    assert_eq!(unlink(&mut rig, toggle), unlinked(target(2, 0)));
    // Gain (#6, param 3) has no link: dimmed, nothing picked.
    let gain = rig.text("Gain").center();
    assert_eq!(unlink(&mut rig, gain), None);
    // The common dialog's Mix is linked, its Post Gain is not.
    let icon = rig.header(0, 2.25);
    rig.click(icon);
    rig.frame(vec![]);
    let post_gain = rig.text("Post Gain").center();
    assert_eq!(unlink(&mut rig, post_gain), None);
    // The Escape that closed the dimmed menu closed the dialog too.
    if !rig.drawn("Mix") {
        rig.click(icon);
        rig.frame(vec![]);
    }
    let mix = rig.text("Mix").center();
    assert_eq!(
        unlink(&mut rig, mix),
        unlinked(target(2, AutomationTarget::MIX))
    );
}

/// Opens the parameter menu of the control at `at` and picks `Unlink from MIDI CC`, then Escape for a menu left open
/// (the item dimmed): the touch the pick reported.
fn unlink(rig: &mut Rig, at: Pos2) -> Option<Touch> {
    rig.secondary_click(at);
    rig.frame(vec![]);
    let item = rig.text("Unlink from MIDI CC").center();
    rig.frame(vec![Event::PointerMoved(item)]);
    rig.button(item, true);
    rig.button(item, false);
    let touch = rig.actions.touch;
    rig.frame(key_tap(egui::Key::Escape, Default::default()));
    touch
}

/// The touch of `Unlink from MIDI CC` on `target`.
fn unlinked(target: AutomationTarget) -> Option<Touch> {
    Some(Touch {
        target,
        pick: Some(CtrlAction::UnlinkMidiCc),
    })
}

#[test]
fn delete_asks_first_when_automation_lanes_target_the_module() {
    let mut channel = Channel::new("Lead".into(), ChannelKind::Instrument, 0.3);
    channel.rack.add(6);
    channel.rack.add(11);
    channel.add_automation_lane(target(3, 1));
    let mut rig = Rig::with(channel);
    let delete = |rig: &mut Rig, card: usize| {
        let menu = rig.header(card, -0.75);
        rig.open(menu);
        let at = rig.text("Delete").center();
        rig.click(at);
    };
    // The Chorus (3) has a lane: it stays, and the app asks.
    delete(&mut rig, 1);
    assert_eq!(rig.actions.ask_delete, Some(3));
    assert!(!rig.actions.replace_song);
    assert_eq!(order(&rig.channel.rack), [2, 3]);
    // The Distortion (2) has none: it goes at once.
    rig.frame(vec![]);
    delete(&mut rig, 0);
    assert_eq!(rig.actions.ask_delete, None);
    assert!(rig.actions.replace_song);
    assert_eq!(order(&rig.channel.rack), [3]);
}

#[test]
fn menu_picks_report_a_new_song_or_the_browser() {
    let mut rig = Rig::new(&[6, 11]);
    let menu = rig.header(1, -0.75);
    rig.open(menu);
    let at = rig.text("Move up").center();
    rig.click(at);
    assert!(rig.actions.replace_song);
    // The swap slide comes to rest before the next menu.
    rig.settle();
    let menu = rig.header(1, -0.75);
    rig.open(menu);
    let at = rig.text("Insert above").center();
    rig.click(at);
    assert_eq!(rig.actions.open_browser, Some(InsertAt::Above(2)));
    assert!(!rig.actions.replace_song);
}

#[test]
fn copy_hands_out_the_module_and_replace_opens_the_browser_on_its_card() {
    let mut rig = Rig::new(&[6, 11]);
    rig.channel.rack.modules[1].params[1] = 0.9;
    let menu = rig.header(1, -0.75);
    rig.open(menu);
    let at = rig.text("Copy").center();
    rig.click(at);
    assert_eq!(rig.actions.copy, Some(vec![rig.module(3).clone()]));
    assert!(!rig.actions.replace_song);
    assert_eq!(order(&rig.channel.rack), [2, 3]);
    let menu = rig.header(0, -0.75);
    rig.open(menu);
    let at = rig.text("Replace").center();
    rig.click(at);
    assert_eq!(rig.actions.open_browser, Some(InsertAt::Replace(2)));
    assert_eq!(rig.actions.copy, None);
    assert_eq!(order(&rig.channel.rack), [2, 3]);
}

#[test]
fn save_preset_and_save_rack_preset_hand_out_what_they_save_and_change_nothing() {
    let mut rig = Rig::new(&[6, 11]);
    rig.channel.rack.modules[1].params[1] = 0.9;
    let before = rig.channel.clone();
    let menu = rig.header(1, -0.75);
    rig.open(menu);
    let at = rig.text("Save preset").center();
    rig.click(at);
    let module = rig.module(3).clone();
    assert_eq!(rig.actions.preset, Some(PresetSource::Module(module)));
    rig.open(rack_menu_at());
    let at = rig.text("Save Rack preset").center();
    rig.click(at);
    let rack = PresetSource::Rack {
        name: "Lead".into(),
        modules: before.rack.modules.clone(),
    };
    assert_eq!(rig.actions.preset, Some(rack));
    assert!(!rig.actions.replace_song && !rig.actions.minimized);
    assert_eq!(rig.channel, before);
}

#[test]
fn copy_rack_hands_out_every_module_top_to_bottom_the_instrument_included_and_changes_nothing() {
    let mut rig = Rig::new(&[51, 0, 6]);
    rig.channel.rack.modules[2].params[1] = 0.9;
    let before = rig.channel.clone();
    rig.open(rack_menu_at());
    let at = rig.text("Copy Rack").center();
    rig.click(at);
    assert_eq!(rig.actions.copy, Some(before.rack.modules.clone()));
    assert!(!rig.actions.replace_song && !rig.actions.minimized);
    assert_eq!(rig.channel, before);
    // An instrument card's `Copy` copies it.
    let menu = rig.header(1, -0.75);
    rig.open(menu);
    let at = rig.text("Copy").center();
    rig.click(at);
    assert_eq!(rig.actions.copy, Some(vec![rig.module(3).clone()]));
}

#[test]
fn clear_rack_empties_an_effects_rack_and_asks_for_a_new_song() {
    let mut rig = Rig::new(&[6, 11]);
    rig.open(rack_menu_at());
    let at = rig.text("Clear Rack").center();
    rig.click(at);
    assert!(rig.channel.rack.modules.is_empty());
    assert!(rig.actions.replace_song);
}

#[test]
fn add_module_opens_the_browser_at_the_end_and_the_notch_closes() {
    let mut rig = Rig::new(&[6, 11]);
    let at = rig.text("Add Module").center();
    rig.click(at);
    assert_eq!(rig.actions.open_browser, Some(InsertAt::End));
    assert!(!rig.actions.close);
    // The notch: 3 px right of the rack's left edge, centred 0.5 U above the middle.
    let rack = rack_rect(SCREEN);
    rig.click(pos2(rack.left() + 3.0 + 0.2 * U, rack.center().y - 0.5 * U));
    assert!(rig.actions.close);
}

#[test]
fn the_sends_section_shows_the_send_to_the_master() {
    let rig = Rig::new(&[6]);
    assert!(rig.drawn("SENDS"));
    assert!(rig.drawn("MASTER"));
    assert!(rig.drawn("Add Send    "));
    // The rack header carries the channel name.
    assert!(rig.drawn("Lead"));
}

#[test]
fn the_rack_scrolls_when_the_cards_overflow_and_stops_at_the_end() {
    let mut rig = Rig::new(&[3, 3, 3]);
    let top = rig.text("Add Module").top();
    let over = rack_rect(SCREEN).center();
    let wheel = |dy| Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, dy),
        modifiers: Default::default(),
    };
    rig.frame(vec![Event::PointerMoved(over)]);
    rig.frame(vec![wheel(-5000.0)]);
    for _ in 0..60 {
        rig.frame(vec![]);
    }
    let end = rig.text("Add Send    ").bottom();
    assert!(rig.text("Add Module").top() < top);
    // The end of the flow sits at the rack's bottom edge (the text is inside its 1.75 U row).
    assert!(
        end <= SCREEN.bottom() && end > SCREEN.bottom() - 1.75 * U,
        "{end}"
    );
}

// ---- instrument cards ----

#[test]
fn the_header_row_follows_section_3_3() {
    // U = 100 px, a 17 U card at (150, 80), a 2 U preset button.
    let card = Rect::from_min_size(pos2(150.0, 80.0), vec2(1700.0, 850.0));
    let r = |x0, y0, x1, y1| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
    // Toggle, icon, preset and ≡: 8 px top and bottom; the name 4 px and 0.25 U left; the name takes what is left.
    assert_eq!(
        header_row(card, 100.0, 2.0, false)[..5],
        [
            r(150.0, 88.0, 300.0, 222.0),
            r(300.0, 88.0, 450.0, 222.0),
            r(475.0, 84.0, 1500.0, 226.0),
            r(1500.0, 88.0, 1700.0, 222.0),
            r(1700.0, 88.0, 1850.0, 222.0),
        ]
    );
    // The preset button is the name's 10 pt width + 1 U, at most 5 U.
    assert_eq!(preset_width(150.0, 100.0), 2.5);
    assert_eq!(preset_width(400.0, 100.0), 5.0);
    assert_eq!(preset_width(900.0, 100.0), 5.0);
    // ▶: (R − 0.35 U, cy ∓ 0.175 U), (R − 0.175 U, cy).
    let preset = r(1500.0, 88.0, 1700.0, 222.0);
    assert_eq!(
        play_triangle(preset, 100.0),
        [
            pos2(1665.0, 137.5),
            pos2(1665.0, 172.5),
            pos2(1682.5, 155.0)
        ]
    );
}

#[test]
fn the_module_area_of_an_instrument_card_is_16_96_u_by_h_minus_2_01_u() {
    // At U = 100 a 17 U card's module area is 16.96 U × (H − 2.01) U, 2 px in and 1 px under the header.
    for (id, h) in [
        (0, 8.5),
        (1, 7.5),
        (2, 9.5),
        (19, 7.0),
        (23, 9.0),
        (33, 8.5),
        (39, 10.0),
        (52, 8.5),
    ] {
        let module = ModuleInstance::new(id, 2);
        assert_eq!(card_height(&module), h);
        let card = Rect::from_min_size(pos2(0.0, 0.0), vec2(1700.0, h * 100.0));
        assert_eq!(
            content_rect(card, h, 100.0),
            Rect::from_min_size(pos2(2.0, 151.0), vec2(1696.0, (h - 2.01) * 100.0)),
            "{id}"
        );
    }
}

#[test]
fn the_level_popup_hangs_from_the_icon_centre_and_moves_up_to_stay_on_screen() {
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let popup = |x, y| Rect::from_min_size(pos2(x, y), vec2(10.0 * U, 8.0 * U));
    assert_eq!(
        level_popup_rect(pos2(760.0, 51.0), SCREEN, &m),
        popup(760.0, 51.0)
    );
    // Its bottom would leave the screen: it moves up onto the bottom edge, x unchanged.
    assert_eq!(
        level_popup_rect(pos2(760.0, 701.0), SCREEN, &m),
        popup(760.0, 544.0)
    );
    // Snapped to whole pixels.
    assert_eq!(
        level_popup_rect(pos2(760.4, 51.6), SCREEN, &m),
        popup(760.0, 52.0)
    );
}

#[test]
fn instrument_card_menus_add_set_range_or_set_first_note() {
    let row = |type_id, full, row| {
        card_menu(false, type_id, full)
            .into_iter()
            .find(|i| (i.col, i.row) == (0, row))
            .map(|i| (i.id, i.label.into_owned(), i.disabled))
    };
    let item = |action, label: &str| Some((Some(action), label.to_owned(), false));
    for id in [0, 1, 2, 19, 23, 39, 52] {
        assert_eq!(
            row(id, true, 4),
            item(CardAction::SetRange, "Set Range"),
            "{id}"
        );
        // Row 5 only while the key range is not the full 0–127.
        assert_eq!(row(id, true, 5), None, "{id}");
        assert_eq!(
            row(id, false, 5),
            item(CardAction::SetFullRange, "Set Full Range"),
            "{id}"
        );
    }
    assert_eq!(
        row(33, true, 4),
        item(CardAction::SetRange, "Set First Note")
    );
    assert_eq!(row(33, false, 5), None, "never on a Slicer");
    // Effects keep the nine items.
    assert_eq!(row(6, true, 4), None);
    assert_eq!(card_menu(false, 6, false).len(), 9);
    assert_eq!(card_menu(false, 0, true).len(), 10);
    assert_eq!(card_menu(false, 0, false).len(), 11);
    // `Set Range` starts a key-learn, panel state: the rack is unchanged. `Set Full Range` resets the range.
    let mut r = rack();
    assert_eq!(apply(&mut r, 2, CardAction::SetRange), Outcome::Nothing);
    assert_eq!(r, rack());
    let module = r.get_mut(2).unwrap();
    (module.file.key_low, module.file.key_high) = (10, 90);
    assert_eq!(
        apply(&mut r, 2, CardAction::SetFullRange),
        Outcome::ChainChanged
    );
    assert_eq!(r, rack());
    // All items are always enabled.
    let disabled = |type_id, action| {
        let items = card_menu(false, type_id, true);
        items
            .into_iter()
            .find(|i| i.id == Some(action))
            .unwrap()
            .disabled
    };
    for action in [CardAction::Delete, CardAction::Copy, CardAction::Replace] {
        assert!(!disabled(0, action) && !disabled(33, action), "{action:?}");
        assert!(!disabled(6, action), "{action:?}");
    }
    // An instrument's preset saves like an effect's.
    assert!(!disabled(0, CardAction::SavePreset) && !disabled(33, CardAction::SavePreset));
}

impl Rig {
    /// A rig on the song's new `synth` channel.
    fn synth(synth: SynthKind) -> Self {
        let mut song = crate::song::Song::default();
        song.add_channel(ChannelKind::Synth(synth));
        Self::with(song.channels.pop().unwrap())
    }

    /// Whether a filled rect exactly `rect` was painted in `color`.
    fn rect_drawn(&self, rect: Rect, color: Color32) -> bool {
        self.shapes
            .iter()
            .any(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.rect == rect && r.fill == color))
    }

    /// The point size `text` was painted at.
    fn font_size(&self, text: &str) -> f32 {
        self.shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(t.galley.job.sections[0].format.font_id.size)
                }
                _ => None,
            })
            .unwrap()
    }

    /// Whether a filled triangle with these corners (within 0.01) was painted.
    fn triangle_drawn(&self, corners: [Pos2; 3]) -> bool {
        self.shapes.iter().any(|c| match &c.shape {
            egui::Shape::Path(p) => {
                p.points.len() == 3
                    && p.points
                        .iter()
                        .zip(corners)
                        .all(|(a, b)| (*a - b).length() < 0.01)
            }
            _ => false,
        })
    }

    /// Presses `pos` with a finger and holds it still for a second.
    fn long_touch(&mut self, pos: Pos2) {
        use egui::{TouchDeviceId, TouchId, TouchPhase};
        let touch = |phase| Event::Touch {
            device_id: TouchDeviceId(0),
            id: TouchId(0),
            phase,
            pos,
            force: None,
        };
        let press = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(vec![
            touch(TouchPhase::Start),
            Event::PointerMoved(pos),
            press(true),
        ]);
        for _ in 0..60 {
            self.frame(vec![]);
        }
        self.frame(vec![touch(TouchPhase::End), press(false)]);
        self.frame(vec![Event::PointerGone]);
    }
}

#[test]
fn a_new_synth_channel_shows_its_instrument_card() {
    let rig = Rig::synth(SynthKind::DwSampler);
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let card = rig.cards()[0];
    assert_eq!(card.height(), (7.5 * U).floor());
    // The name: 12 pt, 0.25 U past the toggle and the icon.
    let name = rig.text("DW Sampler");
    assert_eq!(rig.font_size("DW Sampler"), m.font_id(FontSize::Pt12).size);
    assert!(
        (name.left() - (card.left() + 3.25 * U)).abs() <= 1.0,
        "{name:?}"
    );
    // The preset button left of the ≡: `Default` 10 pt, 0.2 U in, then the ▶ at its right edge.
    let preset = rig.text("Default");
    assert_eq!(rig.font_size("Default"), m.font_id(FontSize::Pt10).size);
    let right = card.right() - 1.5 * U;
    let left = right - (preset.width() + U);
    assert!(
        (preset.left() - (left + 0.2 * U)).abs() <= 1.0,
        "{preset:?}"
    );
    let cy = card.top() + 0.75 * U;
    assert!(rig.triangle_drawn([
        pos2(right - 0.35 * U, cy - 0.175 * U),
        pos2(right - 0.35 * U, cy + 0.175 * U),
        pos2(right - 0.175 * U, cy),
    ]));
    // The body is the DW Sampler panel in the module area, its own tab strip on top; the card draws no strip and no
    // placeholder.
    let area = content_rect(card, 7.5, U);
    let tab = rig.text("SAMPLE");
    assert!(area.contains(tab.center()), "{tab:?} in {area:?}");
    let fill = theme_by_id(0).colors.control_fill.to_color32();
    assert!(!rig.rect_drawn(area, fill));
}

#[test]
fn a_long_touch_on_the_instrument_icon_opens_the_level_popup_bound_to_the_common_params() {
    let mut rig = Rig::synth(SynthKind::MiniSynth);
    let icon = rig.header(0, 2.25);
    rig.long_touch(icon);
    for label in ["Wet Pan", "Mix", "Post Gain"] {
        assert!(rig.drawn(label), "{label}");
    }
    drag_the_common_params(&mut rig, 2);
}

/// Drags the open dialog's sliders of module `id`: Wet Pan (0.5) right, Mix (1.0) left and Post Gain (0.5) left, a
/// quarter of the range each. Every step must send that param, −3, −2 and −1, at its new value.
fn drag_the_common_params(rig: &mut Rig, id: i32) {
    // The sliders span the dialog's 9.5 U inside its inset; the thumb follows the pointer along their 8.5 U track.
    let q = 8.5 * U / 4.0;
    for (label, by, index) in [("Wet Pan", q, -3), ("Mix", -q, -2), ("Post Gain", -q, -1)] {
        let at = rig.text(label).center();
        let steps = rig.drag(at, vec2(by, 0.0));
        rig.button(at + vec2(by, 0.0), false);
        let sent: Vec<(usize, i32, i32)> = steps
            .iter()
            .flat_map(|a| &a.params)
            .map(|c| (c.channel, c.instance_id, c.index))
            .collect();
        assert_eq!(sent, [(CHANNEL, id, index); 4], "{label}");
        assert_eq!(
            steps[3].params[0].value,
            rig.module(id).param(index).unwrap()
        );
    }
    // [Post Gain, Mix, Wet Pan].
    assert_eq!(rig.module(id).common.0, [0.25, 0.75, 0.75]);
}

#[test]
fn the_level_popup_of_a_low_card_moves_up_onto_the_screen_bottom() {
    // Two 10 U Equalizers push the MiniSynth card down: its icon centre is at y = 708 + 0.75 U = 732.
    let mut rig = Rig::new(&[3, 3, 0]);
    let icon = rig.header(2, 2.25);
    assert_eq!(icon, pos2(760.0, 732.0));
    rig.secondary_click(icon);
    rig.frame(vec![]);
    let menu_fill = theme_by_id(0).colors.menu_fill.to_color32();
    let popup = Rect::from_min_size(pos2(760.0, 800.0 - 8.0 * U), vec2(10.0 * U, 8.0 * U));
    assert!(rig.rect_drawn(popup, menu_fill));
    assert!(rig.text("Post Gain").bottom() < SCREEN.bottom());
}

impl Rig {
    fn secondary_click(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
    }
}

// ---- edge cases ----

#[test]
fn a_click_opens_an_effect_dialog_but_an_instrument_needs_a_long_press() {
    // An effect: a plain click (the common-dialog test above). An instrument: a click does nothing ...
    let mut rig = Rig::synth(SynthKind::MiniSynth);
    let icon = rig.header(0, 2.25);
    rig.open(icon);
    assert!(!rig.drawn("Wet Pan"));
    // ... a secondary click (the long press of a mouse) opens it.
    rig.secondary_click(icon);
    rig.frame(vec![]);
    assert!(rig.drawn("Wet Pan"));
}

#[test]
fn inserting_a_picked_module_always_changes_the_chain_and_its_edits_are_sent() {
    let mut rig = Rig::new(&[6]);
    // Add Module asks for the browser at the end; the pick goes in through `insert`.
    let add = rig.text("Add Module").center();
    rig.click(add);
    let at = rig.actions.open_browser.unwrap();
    assert_eq!(insert(&mut rig.channel.rack, 11, at), Outcome::ChainChanged);
    // Insert above the Distortion, from its menu.
    let menu = rig.header(0, -0.75);
    rig.open(menu);
    let above = rig.text("Insert above").center();
    rig.click(above);
    let at = rig.actions.open_browser.unwrap();
    assert_eq!(insert(&mut rig.channel.rack, 3, at), Outcome::ChainChanged);
    assert_eq!(order(&rig.channel.rack), [4, 2, 3]);
    let types: Vec<u16> = rig.channel.rack.modules.iter().map(|m| m.type_id).collect();
    assert_eq!(types, [3, 6, 11]);
    // The new cards are live: the Chorus's enable toggle sends its own instance id.
    rig.frame(vec![]);
    assert!(rig.drawn("Chorus") && rig.drawn("Equalizer"));
    let toggle = rig.header(2, 0.75);
    rig.click(toggle);
    assert_eq!(
        rig.actions.params,
        [ParamChange {
            channel: CHANNEL,
            instance_id: 3,
            index: 0,
            value: 0.0
        }]
    );
}

#[test]
fn the_collapse_tab_and_the_left_margin_close_the_rack_and_the_header_does_not() {
    let mut rig = Rig::new(&[6]);
    let c = Chrome::new(rack_rect(SCREEN), U);
    rig.click(c.collapse.unwrap().center());
    assert!(rig.actions.close);
    // Anywhere in the 1.5 U margin under the header.
    rig.click(pos2(c.bar.left() - 0.1 * U, c.header_bottom + 2.0 * U));
    assert!(rig.actions.close);
    // The header's name does nothing; a side rack has no close X.
    rig.click(c.name.center());
    assert!(!rig.actions.close);
}

#[test]
fn a_fractional_card_height_stacks_on_floored_edges() {
    // Tape Stop (42) is 8.4 U: 268.8 px at U = 32.
    assert_eq!(card_height(&ModuleInstance::new(42, 2)), 8.4);
    let rack = Rect::from_min_size(pos2(100.0, 50.0), vec2(640.0, 900.0));
    let f = flow(rack, 0.0, U, false, &[8.4, 5.5], false);
    let col = |y0: f32, y1: f32| Rect::from_min_max(pos2(148.0, y0), pos2(692.0, y1));
    // 96 → 364.8, + 11 px → 375.8 → 551.8, + 11 px → Add Module 562.8 → 618.8; each edge floored on its own.
    assert_eq!(f.cards, [col(96.0, 364.0), col(375.0, 551.0)]);
    assert_eq!(f.add_module, col(562.0, 618.0));
    assert!((f.bottom - 618.8).abs() < 1e-3, "{}", f.bottom);
    // The module area stays unrounded: 6.4 U − 1 px.
    assert!((content_rect(f.cards[0], 8.4, U).height() - 203.8).abs() < 1e-3);
}

#[test]
fn an_audio_record_rack_has_a_4_u_input_row_above_the_first_card() {
    let rack = Rect::from_min_size(pos2(100.0, 50.0), vec2(640.0, 900.0));
    let col = |y0: f32, y1: f32| Rect::from_min_max(pos2(148.0, y0), pos2(692.0, y1));
    // 96 → 224; the first card follows directly, Add Module a card gap after it.
    let f = flow(rack, 0.0, U, true, &[5.5], false);
    assert_eq!(f.input, Some(col(96.0, 224.0)));
    assert_eq!(f.cards, [col(224.0, 400.0)]);
    assert_eq!(f.add_module, col(411.0, 467.0));
    // No cards: Add Module directly under the row, not 1 U lower.
    let empty = flow(rack, 0.0, U, true, &[], false);
    assert_eq!(empty.add_module, col(224.0, 280.0));
    assert_eq!(flow(rack, 0.0, U, false, &[5.5], false).input, None);

    // Drawn for an audio-record channel only: the mic button and `INPUT`, whose menu has no devices.
    let mut song = crate::song::Song::default();
    song.add_channel(ChannelKind::AudioRecord);
    let mut rig = Rig::with(song.channels.pop().unwrap());
    let row = col(46.0, 174.0).translate(vec2(540.0, 0.0));
    assert_eq!(rig.cards(), []);
    assert_eq!(
        rig.text("Add Module").center().y.floor(),
        (row.bottom() + 0.875 * U).floor()
    );
    let input = rig.text("INPUT");
    assert!(row.contains_rect(input), "{input:?}");
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    assert_eq!(rig.font_size("INPUT"), m.font_id(FontSize::Pt12).size);
    rig.open(input.center());
    // The host lists the devices after the frame that opened the menu: none, so `<no input>`.
    rig.frame(vec![]);
    assert!(rig.drawn("<no input>"));
    assert!(!Rig::new(&[6]).drawn("INPUT"));
}

// ---- the input row of an Audio Record channel ----

/// A rack of an Audio Record channel with no modules, its input row faded in.
fn input_rig() -> Rig {
    let mut song = crate::song::Song::default();
    song.add_channel(ChannelKind::AudioRecord);
    let mut rig = Rig::with(song.channels.pop().unwrap());
    for _ in 0..INPUT_APPEAR_FRAMES {
        rig.frame(vec![]);
    }
    rig
}

/// The input row, and its mic button's centre and disc radius.
fn input_row_at() -> (Rect, Pos2, f32) {
    let row = flow(rack_rect(SCREEN), 0.0, U, true, &[], true)
        .input
        .unwrap();
    (
        row,
        row.split_top_bottom_at_fraction(0.5).0.center(),
        0.7 * U,
    )
}

/// The filled circles of the last frame centred on `at`, as (radius, fill, clip rect), outermost first.
fn circles(rig: &Rig, at: Pos2) -> Vec<(f32, Color32, Rect)> {
    let circles = rig.shapes.iter().filter_map(|c| match &c.shape {
        egui::Shape::Circle(s) if s.center == at => Some((s.radius, s.fill, c.clip_rect)),
        _ => None,
    });
    circles.collect()
}

#[test]
fn the_meter_rings_follow_the_level_law_at_hand_peaks() {
    // f = (20·log10(peak) + 60)/66, alpha = f^(5/3)·(1 − k/48)².
    let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
    assert!(near(meter_alpha(1.0, 0), 0.853_124), "0 dB: f = 60/66");
    assert!(near(meter_alpha(0.1, 24), 0.108_509), "−20 dB half way out");
    assert!(near(meter_alpha(0.5, 12), 0.402_343));
    assert!(near(meter_alpha(1.0, 47), 0.000_370_28), "the oldest ring");
    // −60 dB and below, silence and no peak draw nothing; +6 dB and above is whole.
    for quiet in [0.001, 0.0001, 0.0, f32::NAN] {
        assert_eq!(meter_alpha(quiet, 0), 0.0, "{quiet}");
    }
    assert_eq!(meter_alpha(2.0, 0), 1.0);
    assert_eq!(meter_alpha(4.0, 24), 0.25);
    // r + (k/48)·0.4·rack width.
    assert_eq!(meter_radius(22.4, 0, 640.0), 22.4);
    assert_eq!(meter_radius(22.4, 24, 640.0), 150.4);
    assert!((meter_radius(10.0, 47, 480.0) - 198.0).abs() < 1e-4);
}

#[test]
fn the_meter_draws_a_ring_per_peak_around_the_mic_button_in_the_channel_colour() {
    let mut rig = input_rig();
    let (row, mic, r) = input_row_at();
    // No open input: no meter, only the button's disc.
    assert_eq!(circles(&rig, mic).len(), 1);
    // The newest windows loud, older ones quieter, the oldest silent.
    let peaks: [f32; METER_RINGS] =
        std::array::from_fn(|k| if k < 40 { 1.0 / (k + 1) as f32 } else { 0.0 });
    rig.rack.input_peaks = Some(peaks);
    rig.frame(vec![]);
    let ink = channel_color(rig.channel.hue, &theme_by_id(0)).to_color32();
    let check = |rig: &Rig, base: f32| {
        let drawn = circles(rig, mic);
        // Rings 39 to 0, outermost first, then the button's disc.
        assert_eq!(drawn.len(), 41);
        for (&(radius, fill, clip), k) in drawn.iter().zip((0..40).rev()) {
            assert_eq!(radius, meter_radius(r, k, 640.0), "ring {k}");
            let alpha = base * meter_alpha(peaks[k], k);
            let want = ink.gamma_multiply(alpha);
            let close = fill
                .to_array()
                .iter()
                .zip(want.to_array())
                .all(|(a, b)| a.abs_diff(b) <= 1);
            assert!(close, "ring {k}: {fill:?}, want {want:?}");
            assert_eq!(clip, row, "clipped to the row");
        }
    };
    // Not monitoring: faint.
    check(&rig, 0.15);
    assert!(
        rig.repaint <= std::time::Duration::from_millis(25),
        "the meter keeps itself live"
    );
    rig.channel.file.input_monitoring = true;
    rig.frame(vec![]);
    check(&rig, 0.5);
}

#[test]
fn the_input_row_fades_in_over_12_frames_each_time_the_rack_shows() {
    assert_eq!(
        [1, 6, 11, 12, 13, 400].map(input_appear),
        [1.0 / 12.0, 0.5, 11.0 / 12.0, 1.0, 1.0, 1.0]
    );
    let mut song = crate::song::Song::default();
    song.add_channel(ChannelKind::AudioRecord);
    let mut rig = Rig::with(song.channels.pop().unwrap());
    let (_, mic, r) = input_row_at();
    let text = theme_by_id(0).colors.text.to_color32();
    let disc = |rig: &Rig| {
        let drawn = circles(rig, mic);
        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].0, r);
        drawn[0].1
    };
    let opacity = |rig: &Rig| {
        rig.shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == "INPUT" => Some(t.fallback_color),
                _ => None,
            })
            .unwrap()
    };
    // The first frame: a twelfth, and the next frame asked for.
    assert_eq!(disc(&rig), text.gamma_multiply(1.0 / 12.0));
    assert_eq!(opacity(&rig), text.gamma_multiply(1.0 / 12.0));
    assert_eq!(rig.repaint, std::time::Duration::ZERO);
    for n in 2..=12 {
        rig.frame(vec![]);
        assert_eq!(
            opacity(&rig),
            text.gamma_multiply(n as f32 / 12.0),
            "frame {n}"
        );
    }
    assert_eq!(disc(&rig), text);
    // A rack open and still asks for no frames; the cards never moved.
    rig.frame(vec![]);
    assert_eq!(rig.repaint, std::time::Duration::MAX);
    // Closed for a frame (not shown) and opened again: it fades in again.
    let _ = rig.ctx.run(RawInput::default(), |_| {});
    rig.frame(vec![]);
    assert_eq!(opacity(&rig), text.gamma_multiply(1.0 / 12.0));
}

#[test]
fn another_audio_record_racks_input_row_fades_in_again() {
    let mut rig = input_rig();
    let text = theme_by_id(0).colors.text.to_color32();
    let ink = |rig: &Rig| {
        let input = rig.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "INPUT" => Some(t.fallback_color),
            _ => None,
        });
        input.unwrap()
    };
    assert_eq!(ink(&rig), text);
    // Another Audio Record channel's rack in its place, with no frame between: a twelfth again.
    let mut song = crate::song::Song::default();
    song.add_channel(ChannelKind::AudioRecord);
    song.add_channel(ChannelKind::AudioRecord);
    (rig.channel, rig.index) = (song.channels.pop().unwrap(), CHANNEL + 1);
    rig.frame(vec![]);
    assert_eq!(ink(&rig), text.gamma_multiply(1.0 / 12.0));
    for _ in 1..INPUT_APPEAR_FRAMES {
        rig.frame(vec![]);
    }
    assert_eq!(ink(&rig), text);
}

#[test]
fn the_mic_button_toggles_monitoring_and_with_multitrack_the_arm_button_the_record_arm() {
    let mut rig = input_rig();
    let (row, mic, r) = input_row_at();
    let colors = theme_by_id(0).colors;
    let ink = channel_color(rig.channel.hue, &theme_by_id(0)).to_color32();
    assert!(!rig.channel.file.input_monitoring);
    rig.click(mic);
    assert!(rig.channel.file.input_monitoring);
    assert!(
        rig.actions.replace_song,
        "a final edit: an undo step and a new engine"
    );
    // Monitoring: a channel-colour disc (the pointer moved off: a hovered fill is lit).
    let away = Event::PointerMoved(pos2(5.0, 5.0));
    rig.frame(vec![away.clone()]);
    let disc = circles(&rig, mic);
    assert_eq!((disc.len(), disc[0].0, disc[0].1), (1, r, ink));
    rig.click(mic);
    assert!(!rig.channel.file.input_monitoring && rig.actions.replace_song);
    rig.frame(vec![away.clone()]);
    assert_eq!(circles(&rig, mic)[0].1, colors.text.to_color32());

    // The arm button: at the upper half's right end, with multitrack only.
    let arm = pos2(row.right() - U, row.top() + U);
    rig.click(arm);
    assert!(!rig.channel.file.record_armed && !rig.actions.replace_song);
    assert!(circles(&rig, arm).is_empty());
    rig.rack.multitrack = true;
    rig.frame(vec![]);
    assert_eq!(circles(&rig, arm).len(), 1, "a 2 U icon button");
    rig.click(arm);
    assert!(rig.channel.file.record_armed && rig.actions.replace_song);
    assert!(!rig.channel.file.input_monitoring, "the arm only");
    rig.frame(vec![away]);
    assert_eq!(circles(&rig, arm)[0].1, ink);
    rig.click(arm);
    assert!(!rig.channel.file.record_armed);
}

#[test]
fn the_input_menu_marks_the_channels_input() {
    let devices = [
        crate::recording::input::InputDevice {
            name: "Interface".into(),
            channels: 2,
        },
        crate::recording::input::InputDevice {
            name: "Mic".into(),
            channels: 1,
        },
    ];
    let pair = InputChoice {
        device: 0,
        first: 0,
        second: Some(1),
    };
    let marked = |current| -> Vec<String> {
        let items = input_items(&devices, current);
        let items = items.into_iter().filter(|i| i.current);
        items.map(|i| i.label.into_owned()).collect()
    };
    assert_eq!(marked(Some(pair)), ["Inputs 1 + 2"]);
    let mic = InputChoice {
        device: 1,
        first: 0,
        second: None,
    };
    assert_eq!(marked(Some(mic)), ["Input 1"]);
    assert_eq!(marked(None), Vec::<String>::new());
    // Otherwise the input menu.
    let labels: Vec<_> = (input_items(&devices, None).into_iter())
        .map(|i| (i.label.into_owned(), i.disabled))
        .collect();
    let want = [
        ("Interface", true),
        ("Input 1", false),
        ("Input 2", false),
        ("Inputs 1 + 2", false),
        ("Mic", true),
        ("Input 1", false),
    ];
    assert_eq!(labels, want.map(|(l, d)| (l.to_owned(), d)));
}

#[test]
fn the_input_menu_lists_the_hosts_devices_and_a_pick_is_the_channels_input() {
    let mut rig = input_rig();
    rig.devices = vec![
        crate::recording::input::InputDevice {
            name: "Interface".into(),
            channels: 2,
        },
        crate::recording::input::InputDevice {
            name: "Mic".into(),
            channels: 1,
        },
    ];
    let input = rig.text("INPUT");
    rig.click(input.center());
    assert!(rig.actions.list_inputs, "the host lists the devices");
    assert!(!rig.drawn("Interface"), "not before they are listed");
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.drawn("Interface") && rig.drawn("Inputs 1 + 2") && rig.drawn("Mic"));
    // A device's own row picks nothing.
    rig.click(rig.text("Interface").center());
    assert_eq!(rig.actions.input, None);
    rig.frame(vec![]);
    assert!(rig.drawn("Inputs 1 + 2"), "still open");
    rig.click(rig.text("Inputs 1 + 2").center());
    assert_eq!(
        rig.actions.input,
        Some(Route {
            device: "Interface".into(),
            first: 0,
            second: Some(1),
        })
    );
    assert!(!rig.actions.replace_song, "no song edit: the app keeps it");
    rig.frame(vec![]);
    assert!(!rig.drawn("Interface"), "closed");
    assert_eq!(rig.rack.inputs, None, "listed again on the next open");
    // The next open lists again; the host now marks the pick.
    rig.current = Some(InputChoice {
        device: 1,
        first: 0,
        second: None,
    });
    rig.click(input.center());
    assert!(rig.actions.list_inputs);
    rig.frame(vec![]);
    rig.frame(vec![]);
    // The Mic's `Input 1`, the last one.
    let mic_input = (rig.shapes.iter())
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "Input 1" => {
                Some(t.galley.rect.translate(t.pos.to_vec2()))
            }
            _ => None,
        })
        .max_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap();
    rig.click(mic_input.center());
    assert_eq!(
        rig.actions.input,
        Some(Route {
            device: "Mic".into(),
            first: 0,
            second: None,
        })
    );
}

#[test]
fn panel_states_of_removed_modules_are_dropped() {
    let mut rig = Rig::new(&[6, 11]);
    let key = |id: i32| {
        let ty = rig.channel.rack.get(id).unwrap().type_id;
        (CHANNEL, id, ty)
    };
    let (distortion, chorus) = (key(2), key(3));
    assert!(rig.rack.panels.contains_key(&distortion));
    rig.channel.rack.remove(2);
    rig.frame(vec![]);
    assert!(!rig.rack.panels.contains_key(&distortion));
    // An id reused by a module of another type starts afresh.
    rig.channel.rack.get_mut(3).unwrap().type_id = 6;
    rig.frame(vec![]);
    assert!(!rig.rack.panels.contains_key(&chorus));
    assert!(rig.rack.panels.contains_key(&(CHANNEL, 3, 6)));
}

// Truncated texts: the card name (K 5), the preset name (K 0), the dialog title (K 5).

/// The text shapes of one frame of `draw`: text, the rect its galley covers, font size.
fn one_frame(draw: impl FnMut(&mut Ui, &UiCtx)) -> Vec<(String, Rect, f32)> {
    let mut draw = draw;
    let theme = theme_by_id(0);
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let raw = RawInput {
        screen_rect: Some(SCREEN),
        ..Default::default()
    };
    let output = Context::default().run(raw, |ctx| {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| draw(ui, &cx));
    });
    output
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((
                t.galley.text().to_owned(),
                Rect::from_min_size(t.pos, t.galley.size()),
                t.galley.job.sections[0].format.font_id.size,
            )),
            _ => None,
        })
        .collect()
}

/// The head and tail around `shown`'s `...`, checked against `full`.
fn ellipsis<'a>(shown: &'a str, full: &str) -> (&'a str, &'a str) {
    let (head, tail) = shown
        .split_once("...")
        .unwrap_or_else(|| panic!("{shown}: no ..."));
    assert!(full.starts_with(head) && full.ends_with(tail), "{shown}");
    (head, tail)
}

#[test]
fn a_card_name_wider_than_its_label_is_truncated_keeping_5_tail_characters() {
    // A 14 U (full-screen) rack: 0.5 U margins and 13 U cards, whose name label (3.25 U in) is narrower than
    // `SoundFont Player` at 12 pt.
    let mut song = crate::song::Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::SoundFontPlayer));
    let mut channel = song.channels.pop().unwrap();
    let mut rack = RackPanel::default();
    let view = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 800.0));
    let texts = one_frame(|ui, cx| {
        rack.show(ui, cx, view, CHANNEL, &mut channel);
    });
    let full = "SoundFont Player";
    let (shown, rect, size) = texts
        .iter()
        .find(|t| t.0.starts_with("Sound"))
        .expect("the name")
        .clone();
    let (head, tail) = ellipsis(&shown, full);
    assert_eq!(tail, "layer", "{shown}");
    assert!(head.len() > 5, "{shown}");
    assert_eq!(size, 24.0);
    let card = flow(
        view,
        0.0,
        U,
        false,
        &[card_height(&channel.rack.modules[0])],
        true,
    )
    .cards[0];
    assert!(
        (rect.left() - (card.left() + 3.25 * U)).abs() < 1e-3,
        "{rect:?} {card:?}"
    );
}

#[test]
fn the_preset_name_is_truncated_with_an_end_ellipsis() {
    let full = "A preset name far too long";
    let button = Rect::from_min_size(pos2(100.0, 100.0), vec2(4.0 * U, 1.5 * U));
    let texts = one_frame(|ui, cx| preset_button(ui, cx, button, full, Color32::RED));
    let (shown, rect, size) = texts[0].clone();
    let (head, tail) = ellipsis(&shown, full);
    assert!(head.len() > 3 && tail.is_empty(), "{shown}");
    assert_eq!(size, 20.0);
    // Left-aligned in the button inset 0.2 U, and clear of the ▶ (its left edge 0.35 U in).
    assert!(
        (rect.left() - (button.left() + 0.2 * U)).abs() < 1e-3,
        "{rect:?}"
    );
    assert!(rect.right() <= button.right() - 0.5 * U, "{rect:?}");
    assert!(rect.right() > button.right() - 1.5 * U, "{rect:?}");
}

#[test]
fn the_dialog_title_is_truncated_keeping_5_tail_characters_on_its_rows_bottom() {
    let full = "A dialog title far too long";
    let row = Rect::from_min_size(pos2(100.0, 100.0), vec2(5.0 * U, 0.5 * U));
    let texts = one_frame(|ui, cx| dialog_title(ui.painter(), cx, row, full, Color32::RED));
    let (shown, rect, size) = texts[0].clone();
    let (head, tail) = ellipsis(&shown, full);
    assert!(head.len() > 5 && tail == " long", "{shown}");
    assert_eq!(size, 24.0);
    // Centred, its bottom on the row's bottom.
    assert!((rect.center().x - row.center().x).abs() < 1e-3, "{rect:?}");
    assert!((rect.bottom() - row.bottom()).abs() < 1e-3, "{rect:?}");
    assert!(rect.width() <= row.width(), "{rect:?}");
}

// ---- more edge cases ----

#[test]
fn the_input_rows_mic_button_is_a_light_disc_holding_its_icon() {
    let rig = input_rig();
    // The 2 U button centred in the upper half of the 4 U row; its disc is 0.7 × half of 2 U.
    let row = flow(rack_rect(SCREEN), 0.0, U, true, &[], true)
        .input
        .unwrap();
    let button = Rect::from_center_size(
        row.split_top_bottom_at_fraction(0.5).0.center(),
        vec2(2.0 * U, 2.0 * U),
    );
    let (center, radius) = (button.center(), 0.7 * U);
    let colors = theme_by_id(0).colors;
    let disc = |c: &egui::epaint::ClippedShape| {
        matches!(&c.shape, egui::Shape::Circle(c)
            if c.center == center && c.radius == radius && c.fill == colors.text.to_color32())
    };
    assert!(rig.shapes.iter().any(disc), "no light disc at {center:?}");
    // Icon 22 in `moduleFill`: every corner of the box around its shapes on or inside the disc.
    let ink = format!("{:?}", colors.module_fill.to_color32());
    let icon = rig
        .shapes
        .iter()
        .filter(|c| !matches!(c.shape, egui::Shape::Text(_) | egui::Shape::Circle(_)))
        .filter(|c| format!("{:?}", c.shape).contains(&ink))
        .map(|c| c.shape.visual_bounding_rect())
        .filter(|r| button.contains(r.center()))
        .reduce(Rect::union)
        .expect("an icon in the button");
    for corner in [
        icon.left_top(),
        icon.right_top(),
        icon.left_bottom(),
        icon.right_bottom(),
    ] {
        let d = (corner - center).length();
        assert!(
            d <= radius,
            "{corner:?} is {d} from the centre, radius {radius}"
        );
    }
}

#[test]
fn an_instrument_cards_power_toggle_sends_param_0() {
    // Item 1 is the power toggle, param 0 Enable, which every instrument's registry entry has (on).
    for synth in SynthKind::ALL {
        let mut rig = Rig::synth(synth);
        assert_eq!(rig.module(2).params[0], 1.0, "{synth:?}");
        let at = rig.header(0, 0.75);
        rig.click(at);
        assert_eq!(rig.module(2).params[0], 0.0, "{synth:?}");
        assert_eq!(
            rig.actions.params,
            [ParamChange {
                channel: CHANNEL,
                instance_id: 2,
                index: 0,
                value: 0.0
            }],
            "{synth:?}"
        );
    }
}

#[test]
fn escape_closes_the_dialog_drawing_nothing_so_the_next_press_reaches_the_rack() {
    let mut rig = Rig::new(&[6]);
    let icon = rig.header(0, 2.25);
    rig.open(icon);
    assert!(rig.drawn("Wet Pan"));
    rig.frame(vec![Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert!(!rig.drawn("Wet Pan"));
    // A press on the notch, outside where the dialog was, in the very next frame: no backdrop is left to take it.
    let rack = rack_rect(SCREEN);
    let notch = pos2(rack.left() + 1.4 * U, rack.center().y - 0.5 * U);
    rig.frame(vec![
        Event::PointerMoved(notch),
        Event::PointerButton {
            pos: notch,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
    ]);
    rig.button(notch, false);
    assert!(rig.actions.close);
}

#[test]
fn a_tuner_card_shows_the_input_column_on_an_audio_input_channel_only() {
    let mut channel = Channel::new("Lead".into(), ChannelKind::Synth(SynthKind::SuperSaw), 0.3);
    channel.rack.add(24);
    let mut rig = Rig::with(channel);
    let kind = |rig: &Rig| rig.rack.panels.values().next().unwrap().state.flags["channel_kind"];
    // Off an audio input, the slider takes the column; the flag follows the channel's kind every frame.
    assert_eq!((kind(&rig), rig.drawn("Input")), (2, false));
    rig.channel.kind = ChannelKind::AudioRecord;
    rig.frame(vec![]);
    assert_eq!((kind(&rig), rig.drawn("Input")), (1, true));
    rig.channel.kind = ChannelKind::Instrument;
    rig.frame(vec![]);
    assert_eq!((kind(&rig), rig.drawn("Input")), (2, false));
}

// ---- Panel settings ↔ the module's `CSTM` ----

use crate::panels::cstm::{cstm_from_settings, settings_from_cstm};

/// A rig with one module of `type_id`, holding `blob`.
fn with_blob(type_id: u16, blob: Option<Vec<u8>>) -> Rig {
    let mut channel = Channel::new("Lead".into(), ChannelKind::Instrument, 0.3);
    channel.rack.add(type_id);
    channel.rack.get_mut(2).unwrap().file.custom_data = blob;
    Rig::with(channel)
}

/// The panel state of the rig's module 2.
fn state(rig: &Rig) -> &PanelState {
    let type_id = rig.module(2).type_id;
    &rig.rack.panels[&(CHANNEL, 2, type_id)].state
}

/// The centre of control `n` of the rig's top card, of `ui_height` U, as a card on this channel lays it out.
fn control(rig: &Rig, ui_height: f32, n: u32) -> Rect {
    let content = content_rect(rig.cards()[0], ui_height, U);
    let module = rig.module(2);
    let panel = panels::panel_for(module.type_id).unwrap();
    (panels::place(panel, module, state(rig), content, U).into_iter())
        .find(|(l, _)| l.id == n)
        .unwrap()
        .1
}

#[test]
fn a_card_shows_its_modules_cstm_and_a_setting_edit_writes_it_back() {
    // Tuner: Visual (#2) Orbit, Reference (#5) 0.25.
    let blob = cstm_from_settings(24, &HashMap::from([(2, 2.0), (5, 0.25)]), None);
    let mut rig = with_blob(24, blob);
    assert_eq!(
        (state(&rig).settings[&2], state(&rig).settings[&5]),
        (2.0, 0.25)
    );
    assert!(rig.drawn("Orbit"));
    let steps = rig.drag(control(&rig, 7.0, 5).center(), vec2(0.0, -U));
    assert!(
        steps.iter().all(|a| a.params.is_empty()),
        "settings are no params"
    );
    assert!(steps.iter().all(|a| a.settings), "{steps:?}");
    let value = state(&rig).settings[&5];
    assert!(value > 0.25, "{value}");
    let stored = settings_from_cstm(24, rig.module(2).file.custom_data.as_deref().unwrap());
    assert_eq!(stored.map(|s| (s[&5], s[&2])), Some((value, 2.0)));
}

#[test]
fn a_cstm_changed_under_a_card_seeds_it_again() {
    // Undo, redo and an opened song change a module's blob: its card follows.
    let mut rig = with_blob(24, None);
    assert_eq!(state(&rig).settings[&2], 0.0);
    let blob = cstm_from_settings(24, &HashMap::from([(2, 1.0)]), None);
    rig.channel.rack.get_mut(2).unwrap().file.custom_data = blob.clone();
    rig.frame(vec![]);
    assert_eq!(state(&rig).settings[&2], 1.0);
    // The hook turns Input (#6) off in the card on this channel, but that is no edit of the blob.
    assert_eq!(state(&rig).settings[&6], 0.0);
    assert!(!rig.actions.settings, "no edit");
    assert_eq!(rig.module(2).file.custom_data, blob);
    rig.channel.rack.get_mut(2).unwrap().file.custom_data = None;
    rig.frame(vec![]);
    assert_eq!(state(&rig).settings[&2], 0.0);
    // A setting no blob keeps (a Slicer without its sample) stays as edited.
    let mut rig = with_blob(33, None);
    let key = (CHANNEL, 2, 33);
    rig.rack
        .panels
        .get_mut(&key)
        .unwrap()
        .state
        .settings
        .insert(3100, 0.9);
    rig.frame(vec![]);
    assert_eq!(state(&rig).settings[&3100], 0.9);
    assert_eq!(rig.module(2).file.custom_data, None);
}

#[test]
fn the_analyzers_page_is_kept_in_its_cstm() {
    let blob = cstm_from_settings(26, &HashMap::from([(19, 2.0)]), None);
    let mut rig = with_blob(26, blob);
    assert_eq!(state(&rig).tab, 2);
    // The fifth of six pages.
    let tabs = control(&rig, 10.0, 19);
    rig.click(pos2(
        tabs.left() + tabs.width() * 4.5 / 6.0,
        tabs.center().y,
    ));
    assert_eq!(state(&rig).tab, 4);
    let stored = settings_from_cstm(26, rig.module(2).file.custom_data.as_deref().unwrap());
    assert_eq!(stored.map(|s| s[&19]), Some(4.0));
}

#[test]
fn a_page_change_gives_a_module_at_its_new_settings_no_blob() {
    // DW Sampler: FILTER, then back. A module with no blob loads as a new one's.
    let mut rig = with_blob(1, None);
    let tabs = control(&rig, 7.5, 0);
    for page in [1, 0] {
        let x = tabs.left() + tabs.width() * (page as f32 + 0.5) / 5.0;
        rig.click(pos2(x, tabs.center().y));
        assert_eq!(state(&rig).tab, page);
    }
    assert_eq!(rig.module(2).file.custom_data, None);
}

#[test]
fn the_preset_button_shows_the_modules_preset_name_and_is_as_wide_as_it() {
    let mut channel = channel(ChannelKind::Instrument, &[6, 11]);
    channel.rack.modules[0].file.preset_name = "Crunch".into();
    let long = "A preset name far too long for the button";
    channel.rack.modules[1].file.preset_name = long.into();
    let rig = Rig::with(channel);
    let cards = rig.cards();
    // The name and its 1 U of room, left of the ≡; the name starts 0.2 U in. The layout snaps to pixels.
    let crunch = rig.text("Crunch");
    let width = preset_width(crunch.width(), U);
    let left = cards[0].right() - (1.5 + width) * U + 0.2 * U;
    assert!((crunch.left() - left).abs() < 1.0, "{crunch:?} {left}");
    // At most 5 U, the name cut short.
    let (shown, rect) = (rig.shapes.iter())
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text().starts_with("A preset") => Some((
                t.galley.text().to_owned(),
                t.galley.rect.translate(t.pos.to_vec2()),
            )),
            _ => None,
        })
        .expect("drawn");
    ellipsis(&shown, long);
    let left = cards[1].right() - (1.5 + 5.0) * U + 0.2 * U;
    assert!((rect.left() - left).abs() < 1.0, "{rect:?} {left}");
}

#[test]
fn the_preset_button_and_load_rack_preset_ask_for_the_preset_picker() {
    let mut rig = Rig::new(&[6, 11]);
    rig.click(rig.header(1, -2.0));
    assert_eq!(rig.actions.picker, Some(PresetTarget::Module(3)));
    let before = rig.channel.clone();
    rig.open(rack_menu_at());
    let at = rig.text("Load Rack preset").center();
    rig.click(at);
    assert_eq!(rig.actions.picker, Some(PresetTarget::RackReplace));
    assert_eq!(rig.channel, before, "the picker loads, not the menu");
}

#[test]
fn load_drumset_asks_for_the_drumset_picker() {
    let mut rig = Rig::with(channel(ChannelKind::Drums, &[6]));
    let before = rig.channel.clone();
    rig.open(rack_menu_at());
    let at = rig.text("Load Drumset").center();
    rig.click(at);
    assert_eq!(rig.actions.picker, Some(PresetTarget::Drumset));
    assert_eq!(rig.channel, before, "the picker loads, not the menu");
}

// ---- Live meters: each card posts its module's meter from the output snapshot for the panel's draws ----

use honeydew_audio::MeterSnapshot;

/// The rects filled in `fill` inside control `n` of the rig's card of `ui_height` U, in drawing order.
fn fills(rig: &Rig, ui_height: f32, n: u32, fill: Color32) -> Vec<Rect> {
    let at = control(rig, ui_height, n).expand(0.5);
    (rig.shapes.iter())
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == fill && at.contains_rect(r.rect) => Some(r.rect),
            _ => None,
        })
        .collect()
}

/// Feeds `value` as module 2's meter, beside a loud one for module 2 of another channel, which the card never shows.
fn feed(rig: &mut Rig, value: MeterSnapshot) {
    let loud = MeterSnapshot {
        peak_left: 1.0,
        peak_right: 1.0,
        gain_reduction_db: 12.0,
        ..Default::default()
    };
    let meter = |channel, value| ModuleMeter {
        key: ModuleKey {
            channel,
            instance_id: 2,
        },
        value,
        analysis: Vec::new(),
    };
    rig.rack.meters = Some(vec![meter(CHANNEL + 1, loud), meter(CHANNEL, value)]);
}

/// Runs frames until none asks for the next one at once (a slow poll may remain), at most `limit`; returns how many
/// it ran.
fn until_rest(rig: &mut Rig, limit: usize) -> usize {
    (1..=limit)
        .find(|_| {
            rig.frame(vec![]);
            rig.repaint > std::time::Duration::ZERO
        })
        .unwrap_or_else(|| panic!("still moving after {limit} frames"))
}

fn ink(rig: &Rig) -> Color32 {
    channel_color(rig.channel.hue, &theme_by_id(0)).to_color32()
}

#[test]
fn the_leveller_bars_follow_the_feed_hold_a_second_and_come_to_rest() {
    let mut rig = with_blob(20, None);
    let ink = ink(&rig);
    feed(&mut rig, MeterSnapshot::default());
    until_rest(&mut rig, 5);
    assert!(fills(&rig, 5.0, 1, ink).is_empty() && fills(&rig, 5.0, 3, ink).is_empty());
    // 0 dB left, −20 dB right: bars to 5/6 and 40/72 of the width at once, each with its hold tick at its end.
    feed(
        &mut rig,
        MeterSnapshot {
            peak_left: 1.0,
            peak_right: 0.1,
            gain_reduction_db: 0.0,
            ..Default::default()
        },
    );
    rig.frame(vec![]);
    assert!(rig.ctx.has_requested_repaint());
    let mut ticks = Vec::new();
    for (n, f) in [(1, 5.0 / 6.0), (3, 40.0 / 72.0)] {
        let r = control(&rig, 5.0, n);
        let [bar, tick] = fills(&rig, 5.0, n, ink)[..] else {
            panic!("#{n}")
        };
        let end = r.left() + f * r.width();
        assert!(
            (bar.right() - end).abs() <= 1.0 && (tick.center().x - end).abs() <= 1.0,
            "#{n}: {bar:?} {tick:?}"
        );
        ticks.push((bar, tick));
    }
    // Silence: half a second on (21.5 buffers, −9.6 dB), the bars have fallen and the ticks stand.
    feed(&mut rig, MeterSnapshot::default());
    for _ in 0..30 {
        rig.frame(vec![]);
    }
    let [bar, tick] = fills(&rig, 5.0, 1, ink)[..] else {
        panic!()
    };
    let fell = ticks[0].0.right() - bar.right();
    assert!(
        (fell - 9.6 / 72.0 * control(&rig, 5.0, 1).width()).abs() <= 1.0,
        "{bar:?}"
    );
    assert_eq!(tick, ticks[0].1);
    // Past the second the ticks fall too, and then all rests: troughs only, no more repaints.
    for _ in 0..40 {
        rig.frame(vec![]);
    }
    let left = fills(&rig, 5.0, 1, ink);
    assert!(
        left.last().unwrap().right() < ticks[0].1.right(),
        "{left:?}"
    );
    until_rest(&mut rig, 600);
    assert!(fills(&rig, 5.0, 1, ink).is_empty() && fills(&rig, 5.0, 3, ink).is_empty());
}

#[test]
fn the_compressor_gr_and_out_bars_ease_to_the_feed_and_in_stays_at_rest() {
    let mut rig = with_blob(8, None);
    let ink = ink(&rig);
    let meter = control(&rig, 7.5, 9);
    // The bars: GR, IN, OUT 0.2 U wide at R − 3.2, − 2.0 and − 0.8 U; barH = h − 1.2 U from y + 0.4 U.
    let (top, bar_h) = (meter.top() + 0.4 * U, meter.height() - 1.2 * U);
    let at = |x: f32| meter.right() - x * U;
    // 6 dB of reduction (gain 0.5) and a −6 dB peak on the louder channel.
    feed(
        &mut rig,
        MeterSnapshot {
            peak_left: 0.25,
            peak_right: 0.5,
            gain_reduction_db: 20.0 * 2f32.log10(),
            ..Default::default()
        },
    );
    rig.frame(vec![]);
    let first = fills(&rig, 7.5, 9, ink);
    for _ in 0..120 {
        rig.frame(vec![]);
    }
    let [gr, out] = fills(&rig, 7.5, 9, ink)[..] else {
        panic!("{:?}", fills(&rig, 7.5, 9, ink))
    };
    // A tenth of the way after a frame; there after two seconds. GR hangs from the top, OUT stands on the bottom.
    assert!(
        (first[0].height() - 0.1 * gr.height()).abs() <= 1.0,
        "{first:?} {gr:?}"
    );
    assert!(
        (gr.left() - at(3.2)).abs() <= 0.5 && (gr.top() - top).abs() <= 0.5,
        "{gr:?}"
    );
    assert!(
        (gr.height() - (1.0 - 0.5f32.powf(1.0 / 2.3)) * bar_h).abs() <= 1.0,
        "{gr:?}"
    );
    assert!(
        (out.left() - at(0.8)).abs() <= 0.5 && (out.bottom() - (top + bar_h)).abs() <= 0.5,
        "{out:?}"
    );
    assert!((out.height() - 0.5 * bar_h).abs() <= 1.0, "{out:?}");
    // Silence: both ease back down, then the meter rests and stops asking for frames.
    feed(&mut rig, MeterSnapshot::default());
    rig.frame(vec![]);
    let [gr2, out2] = fills(&rig, 7.5, 9, ink)[..] else {
        panic!()
    };
    assert!(gr2.height() < gr.height() && out2.height() < out.height());
    until_rest(&mut rig, 600);
    assert!(fills(&rig, 7.5, 9, ink).is_empty());
}

#[test]
fn the_multiband_vu_lines_follow_the_feed_and_come_to_rest() {
    let mut rig = with_blob(36, None);
    let text = theme_by_id(0).colors.text.to_color32();
    let vu = control(&rig, 9.0, 39);
    // 0 dB left at 60/66 of the width, −20 dB right at 40/66.
    feed(
        &mut rig,
        MeterSnapshot {
            peak_left: 1.0,
            peak_right: 0.1,
            gain_reduction_db: 0.0,
            ..Default::default()
        },
    );
    rig.frame(vec![]);
    let [left, right] = fills(&rig, 9.0, 39, text)[..] else {
        panic!("{:?}", fills(&rig, 9.0, 39, text))
    };
    assert!(
        (left.right() - (vu.left() + vu.width() * 60.0 / 66.0)).abs() <= 1.0,
        "{left:?}"
    );
    assert!(
        (right.right() - (vu.left() + vu.width() * 40.0 / 66.0)).abs() <= 1.0,
        "{right:?}"
    );
    // Silence: half a second on (21.5 buffers, −9.6 dB) the lines have fallen, then they go.
    feed(&mut rig, MeterSnapshot::default());
    for _ in 0..30 {
        rig.frame(vec![]);
    }
    let [left2, _] = fills(&rig, 9.0, 39, text)[..] else {
        panic!()
    };
    assert!(
        (left.right() - left2.right() - 9.6 / 66.0 * vu.width()).abs() <= 1.0,
        "{left2:?}"
    );
    until_rest(&mut rig, 600);
    assert!(fills(&rig, 9.0, 39, text).is_empty());
}

/// The three live meters: (type, UIHeight, display, the fill its bars draw in).
fn live_meters(rig: &Rig) -> [(u16, f32, u32, Color32); 3] {
    let (ink, text) = (ink(rig), theme_by_id(0).colors.text.to_color32());
    [(20, 5.0, 1, ink), (8, 7.5, 9, ink), (36, 9.0, 39, text)]
}

/// Whether display `n` draws any bar wider and taller than nothing (the VU's empty fill is a zero-width rect).
fn shows_bars(rig: &Rig, ui_height: f32, n: u32, fill: Color32) -> bool {
    (fills(rig, ui_height, n, fill).iter()).any(|r| r.width() > 0.0 && r.height() > 0.0)
}

#[test]
fn residue_and_wild_values_in_the_feed_let_every_meter_rest() {
    for (ty, h, n, fill) in live_meters(&with_blob(20, None)) {
        let mut rig = with_blob(ty, None);
        // An infinite or NaN peak shows full scale, then falls like any other.
        let wild = MeterSnapshot {
            peak_left: f32::INFINITY,
            peak_right: f32::NAN,
            gain_reduction_db: f32::NAN,
            ..Default::default()
        };
        feed(&mut rig, wild);
        rig.frame(vec![]);
        assert!(shows_bars(&rig, h, n, fill), "type {ty}");
        // The engine's anti-denormal residue (1e-30, forever) is silence: the meter rests on it and stops repainting.
        let residue = MeterSnapshot {
            peak_left: 1e-30,
            peak_right: 1e-30,
            gain_reduction_db: 0.0,
            ..Default::default()
        };
        feed(&mut rig, residue);
        // A +12 dB hold falls 0.005 a buffer: about 19 s.
        until_rest(&mut rig, 1300);
        assert!(!shows_bars(&rig, h, n, fill), "type {ty}");
    }
}

#[test]
fn a_meter_whose_module_leaves_the_feed_falls_to_rest() {
    for (ty, h, n, fill) in live_meters(&with_blob(20, None)) {
        let mut rig = with_blob(ty, None);
        let loud = MeterSnapshot {
            peak_left: 0.5,
            peak_right: 0.5,
            gain_reduction_db: 6.0,
            ..Default::default()
        };
        feed(&mut rig, loud);
        rig.frame(vec![]);
        assert!(shows_bars(&rig, h, n, fill), "type {ty}");
        // Gone from the snapshot (an engine without it, a failed stream): the card posts silence, not the last value.
        (rig.rack.meters.as_mut().unwrap()).retain(|m| m.key.channel != CHANNEL);
        until_rest(&mut rig, 600);
        assert!(!shows_bars(&rig, h, n, fill), "type {ty}");
        // The stream stops: no feed, no frames.
        rig.rack.meters = None;
        rig.frame(vec![]);
        assert!(!rig.ctx.has_requested_repaint(), "type {ty}");
    }
}

#[test]
fn a_steady_gr_with_no_output_stops_repainting_once_eased() {
    // A Compressor switched off mid-reduction reports its last GR with a silent output: shown, but not repainted for.
    let mut rig = with_blob(8, None);
    let held = MeterSnapshot {
        peak_left: 0.0,
        peak_right: 0.0,
        gain_reduction_db: 6.0,
        ..Default::default()
    };
    feed(&mut rig, held);
    until_rest(&mut rig, 600);
    let ink = ink(&rig);
    assert_eq!(fills(&rig, 7.5, 9, ink).len(), 1);
}

#[test]
fn a_meter_follows_audio_that_rises_while_a_key_is_held_still() {
    // Transport stopped, a pad held on the keyboard: no input events, so egui runs a frame only if one asked for it.
    let mut rig = with_blob(20, None);
    let ink = ink(&rig);
    feed(&mut rig, MeterSnapshot::default());
    let key = rig.cards()[0].center();
    rig.frame(vec![Event::PointerMoved(key)]);
    rig.button(key, true);
    until_rest(&mut rig, 10);
    // The pad's slow attack reaches the meter feed after the last frame.
    let rising = MeterSnapshot {
        peak_left: 0.5,
        peak_right: 0.5,
        gain_reduction_db: 0.0,
        ..Default::default()
    };
    feed(&mut rig, rising);
    assert!(
        rig.repaint <= std::time::Duration::from_millis(50),
        "no frame comes: {:?}",
        rig.repaint
    );
    rig.frame(vec![]);
    assert!(!fills(&rig, 5.0, 1, ink).is_empty());
}

#[test]
fn a_live_meter_polls_every_50_ms_until_the_stream_stops_the_rack_closes_or_its_card_folds() {
    let poll = std::time::Duration::from_millis(50);
    for (ty, ..) in live_meters(&with_blob(20, None)) {
        let mut rig = with_blob(ty, None);
        // No stream: nothing to poll.
        until_rest(&mut rig, 10);
        assert_eq!(rig.repaint, std::time::Duration::MAX, "type {ty}");
        // A running stream, silent: a slow poll, never a frame at once.
        feed(&mut rig, MeterSnapshot::default());
        until_rest(&mut rig, 10);
        for _ in 0..3 {
            rig.frame(vec![]);
            assert!(
                rig.repaint > std::time::Duration::ZERO && rig.repaint <= poll,
                "type {ty}: {:?}",
                rig.repaint
            );
        }
        // The rack closes: a frame without it asks for nothing.
        rig.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(rig.time),
            ..Default::default()
        };
        let closed = rig.ctx.run(raw, |_| {});
        assert_eq!(
            closed.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            std::time::Duration::MAX
        );
        // Its card folds: the meter is off screen.
        rig.channel.rack.get_mut(2).unwrap().minimized = true;
        rig.frame(vec![]);
        assert_eq!(rig.repaint, std::time::Duration::MAX, "type {ty}");
        // The stream stops.
        rig.channel.rack.get_mut(2).unwrap().minimized = false;
        rig.frame(vec![]);
        assert!(rig.repaint <= poll, "type {ty}");
        rig.rack.meters = None;
        rig.frame(vec![]);
        assert_eq!(rig.repaint, std::time::Duration::MAX, "type {ty}");
    }
}

/// `ADD1` byte 0 is the card minimized: the card menu's `Minimize` / `Maximize` and the rack menu's `Minimize all` /
/// `Maximize all` write it, and a load restores it.
#[test]
fn minimize_and_maximize_write_add1_byte_0_and_a_load_restores_it() {
    // A MiniSynth channel of a song, with a Distortion and a Chorus after the instrument.
    let mut song = crate::song::Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let mut channel = song.channels[1].clone();
    channel.rack.add(6);
    channel.rack.add(11);
    let mut rig = Rig::with(channel);
    // Each module's ADD1 byte 0, in rack order (a chunk is its id, a u32 size, then the payload), and the cards as
    // the saved song loads them.
    let saved = |rig: &Rig| -> (Vec<u8>, Vec<bool>) {
        let mut song = song.clone();
        song.channels[1] = rig.channel.clone();
        let bytes = crate::flm::write(&song).unwrap();
        let add1 = (bytes.windows(4).enumerate())
            .filter(|(_, w)| *w == b"ADD1")
            .map(|(i, _)| bytes[i + 8])
            .collect();
        let loaded = crate::flm::read(&bytes).unwrap();
        let rack = &loaded.channels[1].rack;
        (add1, rack.modules.iter().map(|m| m.minimized).collect())
    };
    let card_item = |rig: &mut Rig, card: usize, item: &str| {
        let menu = rig.header(card, -0.75);
        rig.open(menu);
        let at = rig.text(item).center();
        rig.click(at);
        rig.frame(vec![]);
    };
    let rack_item = |rig: &mut Rig, item: &str| {
        rig.open(rack_menu_at());
        let at = rig.text(item).center();
        rig.click(at);
        rig.frame(vec![]);
    };
    assert_eq!(saved(&rig), (vec![0, 0, 0], vec![false, false, false]));
    card_item(&mut rig, 2, "Minimize");
    assert_eq!(saved(&rig), (vec![0, 0, 1], vec![false, false, true]));
    card_item(&mut rig, 2, "Maximize");
    assert_eq!(saved(&rig), (vec![0, 0, 0], vec![false, false, false]));
    rack_item(&mut rig, "Minimize all");
    assert_eq!(saved(&rig), (vec![1, 1, 1], vec![true, true, true]));
    rack_item(&mut rig, "Maximize all");
    assert_eq!(saved(&rig), (vec![0, 0, 0], vec![false, false, false]));
}

// ---- Settings reach the engine ----

/// The settings the last frame sent, as (key, value), for the rig's module 2.
fn sent(rig: &Rig) -> Vec<(u32, f32)> {
    let changes = rig.actions.setting_changes.iter();
    changes
        .inspect(|c| assert_eq!((c.channel, c.instance_id), (CHANNEL, 2)))
        .map(|c| (c.key, c.value))
        .collect()
}

#[test]
fn a_pitcher_note_press_sends_its_setting_and_nothing_else_does() {
    let mut rig = with_blob(25, None);
    assert_eq!(sent(&rig), [], "a new card sends nothing");
    // E (#9) is off in C minor: on.
    rig.click(control(&rig, 8.0, 9).center());
    assert_eq!(sent(&rig), [(9, 1.0)]);
    rig.frame(vec![]);
    assert_eq!(sent(&rig), [], "a frame without input sends nothing");
    let change = SettingChange {
        channel: 1,
        instance_id: 2,
        key: 9,
        value: 1.0,
    };
    assert!(matches!(
        change.command::<()>(),
        AudioCommand::SetSetting {
            channel: 1,
            instance_id: 2,
            key: 9,
            value: 1.0,
        }
    ));
}

#[test]
fn a_scale_change_sends_the_spinner_and_every_note_it_rewrote() {
    let mut rig = with_blob(25, None);
    // The `Scale` spinner's `v` chevron: Minor to Major, on C.
    let r = control(&rig, 8.0, 20);
    rig.click(pos2(r.center().x, r.bottom() - 0.1 * U));
    // D#, G# and A# go off, E, A and B on.
    let want = [
        (8, 0.0),
        (9, 1.0),
        (13, 0.0),
        (14, 1.0),
        (15, 0.0),
        (16, 1.0),
        (20, 1.0),
    ];
    assert_eq!(sent(&rig), want);
}

#[test]
fn the_multiband_sends_the_band_its_solo_logic_solos() {
    use crate::panels::cstm::MULTIBAND_SOLO_BAND;
    let mut rig = with_blob(36, None);
    let solo = |rig: &Rig| {
        let sent = sent(rig).into_iter();
        sent.filter(|&(key, _)| key == MULTIBAND_SOLO_BAND)
            .map(|(_, v)| v)
            .collect::<Vec<_>>()
    };
    // The master list's `Mid`, the third of its four rows.
    let list = control(&rig, 9.0, 2);
    rig.click(pos2(
        list.center().x,
        list.top() + list.height() * 2.5 / 4.0,
    ));
    assert_eq!(solo(&rig), [2.0]);
    // The `High` page solos High: a page change alone sends it.
    let tabs = control(&rig, 9.0, 1);
    rig.click(pos2(
        tabs.left() + tabs.width() * 3.5 / 4.0,
        tabs.center().y,
    ));
    assert_eq!(state(&rig).tab, 3);
    assert_eq!(solo(&rig), [3.0]);
    // Its `Solo` button off: none.
    rig.click(control(&rig, 9.0, 37).center());
    assert_eq!(solo(&rig), [0.0]);
    rig.frame(vec![]);
    assert_eq!(sent(&rig), []);
}

#[test]
fn a_clones_multiband_sends_its_soloed_band_on_its_first_pass() {
    // The clone's card sends the band its copied panel state holds, so its engine solos what the card shows.
    use crate::panels::cstm::MULTIBAND_SOLO_BAND;
    let mut rig = with_blob(36, None);
    let list = control(&rig, 9.0, 2);
    rig.click(pos2(
        list.center().x,
        list.top() + list.height() * 2.5 / 4.0,
    ));
    let solo = |rig: &Rig| {
        let sent = sent(rig).into_iter();
        let band = sent.filter(|&(key, _)| key == MULTIBAND_SOLO_BAND);
        band.map(|(_, v)| v).collect::<Vec<_>>()
    };
    assert_eq!(solo(&rig), [2.0]);
    rig.rack.copy_panels(CHANNEL, CHANNEL + 1);
    let clone = &rig.rack.panels[&(CHANNEL + 1, 2, 36)].state;
    assert_eq!(clone.settings.get(&MULTIBAND_SOLO_BAND), None);
    assert_eq!(clone.settings.get(&2), Some(&2.0), "the list stays");
    // Show the clone's rack (its states swapped onto the rig's channel): its first pass sends the band.
    let swap = |c: usize| {
        Some(if c == CHANNEL {
            CHANNEL + 1
        } else if c == CHANNEL + 1 {
            CHANNEL
        } else {
            c
        })
    };
    rig.rack.rekey(&swap);
    rig.frame(vec![]);
    assert_eq!(solo(&rig), [2.0]);
    rig.frame(vec![]);
    assert_eq!(sent(&rig), []);
}

#[test]
fn a_note_echo_card_shows_whether_echoes_are_pending() {
    // The card sets the `pending` flag from the Note Echo's feed; no stream is none pending.
    let mut rig = with_blob(47, None);
    let pending = |rig: &mut Rig, slab: Option<f32>| {
        rig.rack.meters = slab.map(|v| {
            vec![ModuleMeter {
                key: ModuleKey {
                    channel: CHANNEL,
                    instance_id: 2,
                },
                value: MeterSnapshot::default(),
                analysis: vec![v],
            }]
        });
        rig.frame(vec![]);
        state(rig).flags.get(crate::panels::flag::PENDING).copied()
    };
    assert_eq!(pending(&mut rig, Some(1.0)), Some(1));
    assert_eq!(pending(&mut rig, Some(0.0)), Some(0));
    assert_eq!(pending(&mut rig, Some(1.0)), Some(1));
    assert_eq!(pending(&mut rig, None), Some(0));
}

#[test]
fn a_slicer_sensitivity_edit_rebuilds_the_engine_once_the_pointer_is_up() {
    // The Slicer slices on the app thread at build, from its `SENS`: the edit is a new song, not a `SetSetting`.
    let sens = [
        b"SENS".as_slice(),
        &5_u32.to_le_bytes(),
        &0.25_f32.to_le_bytes(),
        &[1],
    ]
    .concat();
    let mut rig = with_blob(33, Some(sens));
    let knob = control(&rig, 8.5, 3100).center();
    let steps = rig.drag(knob, vec2(0.0, -U));
    assert!(steps.iter().all(|a| a.settings), "stored: {steps:?}");
    assert!(
        steps
            .iter()
            .all(|a| a.setting_changes.is_empty() && !a.replace_song),
        "{steps:?}"
    );
    rig.button(knob - vec2(0.0, U), false);
    assert!(rig.actions.replace_song);
    assert_eq!(sent(&rig), []);
}

// ---- key ranges and sends ----

/// The setting changes of the last frame that turn a module's key-learn on (1) or off (0).
fn learns(rig: &Rig) -> Vec<(usize, i32, f32)> {
    (rig.actions.setting_changes.iter())
        .filter(|c| c.key == KEY_LEARN)
        .map(|c| (c.channel, c.instance_id, c.value))
        .collect()
}

impl Rig {
    /// Opens card `card`'s ≡ menu and picks `item`; the actions are the pick's frame's.
    fn card_item(&mut self, card: usize, item: &str) {
        let menu = self.header(card, -0.75);
        self.open(menu);
        let at = self.text(item).center();
        self.click(at);
    }

    /// The keyboard plays `key` on the channel; one frame passes.
    fn play(&mut self, key: u8) {
        self.rack.key_played(CHANNEL, key);
        self.frame(vec![]);
    }
}

#[test]
fn set_range_learns_the_next_two_keys_as_the_bounds() {
    let mut rig = Rig::synth(SynthKind::MiniSynth);
    assert_eq!(rig.rack.learn_prompt(), None);
    rig.rack.key_played(CHANNEL, 30);
    rig.card_item(0, "Set Range");
    // The module stops hearing notes while it learns; a key played before is not a bound.
    assert_eq!(learns(&rig), [(CHANNEL, 2, 1.0)]);
    assert_eq!(
        rig.rack.learn_prompt(),
        Some("Play a note to set the lower range bound...")
    );
    rig.play(72);
    assert_eq!(
        rig.rack.learn_prompt(),
        Some("Play a note to set the upper range bound...")
    );
    assert_eq!(
        (rig.module(2).file.key_low, rig.module(2).file.key_high),
        (0, 127)
    );
    // A lower second key: the range is kept in order, as the loader keeps it.
    rig.play(48);
    assert_eq!(
        (rig.module(2).file.key_low, rig.module(2).file.key_high),
        (48, 72)
    );
    assert_eq!(learns(&rig), [(CHANNEL, 2, 0.0)]);
    assert!(
        rig.actions.replace_song,
        "the engine reads the range at build"
    );
    assert_eq!(rig.rack.learn_prompt(), None);
    rig.play(60);
    assert_eq!(
        (rig.module(2).file.key_low, rig.module(2).file.key_high),
        (48, 72)
    );
    // Now the menu offers `Set Full Range`, which puts it back.
    rig.card_item(0, "Set Full Range");
    assert_eq!(
        (rig.module(2).file.key_low, rig.module(2).file.key_high),
        (0, 127)
    );
    assert!(rig.actions.replace_song);
}

#[test]
fn set_first_note_learns_one_key_as_the_slicers_low_key() {
    let mut rig = Rig::synth(SynthKind::Slicer);
    rig.card_item(0, "Set First Note");
    assert_eq!(learns(&rig), [(CHANNEL, 2, 1.0)]);
    rig.play(36);
    assert_eq!(
        (rig.module(2).file.key_low, rig.module(2).file.key_high),
        (36, 127)
    );
    assert_eq!(learns(&rig), [(CHANNEL, 2, 0.0)]);
    assert!(rig.actions.replace_song);
}

#[test]
fn another_channel_or_another_card_ends_a_key_learn() {
    let mut song = crate::song::Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let mut channel = song.channels[1].clone();
    let second = channel.rack.add(0);
    let mut rig = Rig::with(channel.clone());
    rig.card_item(0, "Set Range");
    rig.card_item(1, "Set Range");
    assert_eq!(learns(&rig), [(CHANNEL, 2, 0.0), (CHANNEL, second, 1.0)]);
    // The rack shows another channel: the learn ends on the one it was on.
    channel.name = "Other".into();
    let (rack, ch) = (&mut rig.rack, &mut channel);
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let theme = theme_by_id(0);
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let mut ended = Vec::new();
    let _ = rig.ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ended = rack
                .show(ui, &cx, rack_rect(SCREEN), CHANNEL + 1, ch)
                .setting_changes;
        });
    });
    let ended: Vec<_> = ended
        .iter()
        .map(|c| (c.channel, c.instance_id, c.key, c.value))
        .collect();
    assert_eq!(ended, [(CHANNEL, second, KEY_LEARN, 0.0)]);
    assert_eq!(rig.rack.learn_prompt(), None);
}

/// A rig on a MiniSynth channel (id 7) sending to the master and to send channel 5 (`Effect 5`), with the targets the
/// host gives: the master, send channels 5 and 6 (`Bus`), and channel 8 (a synth: never offered).
fn send_rig() -> Rig {
    let mut channel = Channel::new("Lead".into(), ChannelKind::Synth(SynthKind::MiniSynth), 0.3);
    channel.id = 7;
    channel.file.sends = vec![
        SendState::default(),
        SendState {
            target_channel_id: 5,
            level: 0.5,
            ..SendState::default()
        },
    ];
    let mut rig = Rig::with(channel);
    let target = |id, name: &str, offered, bus| SendTarget {
        id,
        name: name.into(),
        offered,
        bus,
        to_master: true,
    };
    rig.rack.targets = vec![
        target(0, "MASTER", true, 0),
        target(5, "Effect 5", true, 1),
        target(6, "Bus", true, 2),
        target(8, "Synth", false, 0),
    ];
    rig.frame(vec![]);
    rig
}

/// A SoundFont Player (module 2) naming `My SoundFonts/Piano.sf2` at patch `patch`, which the rack read as `count`
/// patches named `Voice 00`, `Voice 01` and so on (none: the read is left out).
fn soundfont_rig(patch: i32, count: Option<usize>) -> Rig {
    use crate::flm::chunk::{ContentPath, ContentRoot};
    use crate::flm::sf2d::Sf2d;
    let path = ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: "My SoundFonts/Piano.sf2".into(),
    };
    let blob = Sf2d {
        path: path.clone(),
        patch,
    }
    .write(None);
    let mut rig = with_blob(52, blob.ok());
    if let Some(count) = count {
        let voice = |i| crate::sf2::Preset {
            name: format!("Voice {i:02}"),
            preset: i as u16,
            bank: 0,
        };
        let patches = (0..count).map(voice).collect();
        rig.rack.soundfonts.insert(path, Ok(patches));
    }
    rig.frame(vec![]);
    rig
}

impl Rig {
    /// The SENDS section as the rack lays it out unscrolled for the channel's sends.
    fn sends_flow(&self) -> SendsFlow {
        let heights: Vec<f32> = self.channel.rack.modules.iter().map(card_height).collect();
        let rows = SendRows {
            rows: self.channel.file.sends.len(),
            disconnected: disconnected(&self.channel, &self.rack.targets),
        };
        flow_sends(rack_rect(SCREEN), 0.0, U, false, &heights, Some(rows))
            .sends
            .unwrap()
    }

    /// Centre of send row `row`'s item `x` U from its left (negative: from its right).
    fn send_item(&self, row: usize, x: f32) -> Pos2 {
        let r = self.sends_flow().rows[row];
        let x = if x < 0.0 {
            r.right() + x * U
        } else {
            r.left() + x * U
        };
        pos2(x, r.center().y)
    }
}

#[test]
fn the_sends_section_has_one_row_per_send() {
    let flow = |rows, disconnected| {
        let rows = Some(SendRows { rows, disconnected });
        flow_sends(rack_rect(SCREEN), 0.0, U, false, &[], rows)
            .sends
            .unwrap()
    };
    let two = flow(2, false);
    // Header, the rows back to back, 1 px, `Add Send`; then the label row when disconnected.
    assert_eq!(two.rows.len(), 2);
    assert_eq!(two.rows[0].top(), two.header.bottom());
    assert_eq!(two.rows[1].top(), two.rows[0].bottom());
    assert_eq!(two.add.top(), two.rows[1].bottom() + 1.0);
    assert_eq!(two.disconnected, None);
    let none = flow(0, true);
    assert_eq!(none.add.top(), none.header.bottom() + 1.0);
    let label = none.disconnected.unwrap();
    assert_eq!((label.top(), label.height()), (none.add.bottom(), 56.0));

    let rig = send_rig();
    for name in ["SENDS", "MASTER", "Effect 5", "Add Send    "] {
        assert!(rig.drawn(name), "{name}");
    }
    assert!(!rig.drawn("Disconnected from MASTER"));
}

#[test]
fn a_send_rows_toggle_and_slider_are_its_enable_send_and_send_level() {
    let mut rig = send_rig();
    // The pill toggle, the first 1.5 U of the row.
    let toggle = rig.send_item(1, 0.75);
    rig.click(toggle);
    assert_eq!(rig.channel.file.sends[1].enabled, 0.0);
    let param = |index, value| ParamChange {
        channel: CHANNEL,
        instance_id: 0,
        index,
        value,
    };
    assert_eq!(rig.actions.params, [param(0x1000 + 5, 0.0)]);
    assert!(!rig.actions.replace_song);
    // The level slider: the right half of the flexible room, before the 2 U remove button.
    let r = rig.sends_flow().rows[1];
    let slider = pos2(
        r.right() - 2.0 * U - 0.25 * (r.width() - 5.5 * U),
        r.center().y,
    );
    let steps = rig.drag(slider, vec2(-2.0 * U, 0.0));
    let sent: Vec<_> = steps.iter().flat_map(|a| a.params.clone()).collect();
    let level = rig.channel.file.sends[1].level;
    assert!(level < 0.5, "{level}");
    assert_eq!(sent.last(), Some(&param(0x2000 + 5, level)));
    assert!(steps.iter().all(|a| !a.replace_song));
    // The slider's press is the recent control, for `Add automation track`.
    rig.button(slider, false);
    rig.frame(vec![Event::PointerMoved(slider)]);
    rig.button(slider, true);
    let touch = rig.actions.touch.unwrap();
    assert_eq!(
        touch.target,
        AutomationTarget {
            instance_id: 0,
            param: 0x2000 + 5
        }
    );
}

#[test]
fn a_send_rows_controls_unlink_only_a_send_param_the_channel_links() {
    let mut rig = send_rig();
    // CC 20 drives the Enable Send to channel 5 (channel param 0x1000 + 5).
    crate::midi::links::link(&mut rig.channel.file.cc_links, 20, 0, 0x1005);
    rig.frame(vec![]);
    let toggle = rig.send_item(1, 0.75);
    assert_eq!(unlink(&mut rig, toggle), unlinked(target(0, 0x1000 + 5)));
    // The master send's toggle (0x1000) is not linked: dimmed, nothing picked.
    let master = rig.send_item(0, 0.75);
    assert_eq!(unlink(&mut rig, master), None);
}

#[test]
fn the_remove_button_drops_the_send() {
    let mut rig = send_rig();
    let remove = rig.send_item(0, -1.0);
    rig.click(remove);
    let targets: Vec<i32> = rig
        .channel
        .file
        .sends
        .iter()
        .map(|s| s.target_channel_id)
        .collect();
    assert_eq!(targets, [5]);
    assert!(rig.actions.replace_song);
    // Its output bus follows: send channel 5's bus.
    assert_eq!(rig.channel.output_bus, 1);
    rig.frame(vec![]);
    let remove = rig.send_item(0, -1.0);
    rig.click(remove);
    rig.frame(vec![]);
    assert!(rig.channel.file.sends.is_empty());
    assert!(rig.drawn("Disconnected from MASTER"));
}

#[test]
fn add_send_offers_the_targets_not_sent_to_yet_then_create_fx_channel() {
    let mut rig = send_rig();
    let add = rig.sends_flow().add.center();
    rig.open(add);
    // `Bus` only: MASTER and Effect 5 have sends already, Synth is no send channel.
    assert!(rig.drawn("Bus") && rig.drawn("Create FX Channel"));
    let bus = rig.text("Bus");
    assert!(bus.top() < rig.text("Create FX Channel").top());
    rig.click(bus.center());
    let last = rig.channel.file.sends.last().unwrap();
    assert_eq!(
        *last,
        SendState {
            target_channel_id: 6,
            ..SendState::default()
        }
    );
    assert!(rig.actions.replace_song);
    // `Create FX Channel` asks the host for a new send channel, its send appended.
    rig.frame(vec![]);
    let add = rig.sends_flow().add.center();
    rig.open(add);
    let create = rig.text("Create FX Channel").center();
    rig.click(create);
    assert_eq!(rig.actions.create_send, Some(3));
    assert_eq!(rig.channel.file.sends.len(), 3, "the host adds it");
}

#[test]
fn the_swap_button_retargets_its_send() {
    let mut rig = send_rig();
    // The swap icon: 2 U after the 1.5 U toggle.
    let swap = rig.send_item(1, 2.5);
    rig.open(swap);
    assert!(rig.drawn("Bus") && rig.drawn("Create FX Channel"));
    let bus = rig.text("Bus").center();
    rig.click(bus);
    let send = &rig.channel.file.sends[1];
    assert_eq!(
        (send.target_channel_id, send.level),
        (6, 0.5),
        "level and enable kept"
    );
    assert!(rig.actions.replace_song);
    assert_eq!(rig.channel.output_bus, 2);
}

#[test]
fn send_targets_name_every_other_channel_and_offer_the_master_and_acyclic_sends() {
    let mut song = crate::song::Song::default();
    song.add_channel(ChannelKind::EffectTrack); // 1: id 1
    song.add_channel(ChannelKind::EffectTrack); // 2: id 2
    song.add_channel(ChannelKind::Drums); // 3: id 3
    // Effect 2 sends to Effect 1, which sends nowhere.
    song.channels[2].file.sends[0].target_channel_id = 1;
    song.channels[1].file.sends.clear();
    let of = |channel| {
        send_targets(&song, channel)
            .into_iter()
            .map(|t| (t.id, t.name, t.offered, t.to_master))
            .collect::<Vec<_>>()
    };
    let t = |id, name: &str, offered, to_master| (id, name.to_owned(), offered, to_master);
    assert_eq!(
        of(3),
        [
            t(0, "MASTER", true, true),
            t(1, "Effect 1", true, false),
            t(2, "Effect 2", true, false),
        ]
    );
    // Effect 1 is fed by Effect 2: offering Effect 2 would close a cycle.
    assert_eq!(
        of(1),
        [
            t(0, "MASTER", true, true),
            t(2, "Effect 2", false, false),
            t(3, "Drums", false, true),
        ]
    );
    assert_eq!(send_targets(&song, 2)[1].bus, song.channels[1].bus_id);
}

#[test]
fn create_send_adds_an_effect_channel_and_sends_to_it() {
    let mut song = crate::song::Song::default();
    song.add_channel(ChannelKind::Drums);
    assert_eq!(create_send(&mut song, 1, 1), Some(2));
    let new = &song.channels[2];
    assert_eq!(
        (new.kind, new.name.as_str()),
        (ChannelKind::EffectTrack, "Effect 2")
    );
    let targets: Vec<i32> = song.channels[1]
        .file
        .sends
        .iter()
        .map(|s| s.target_channel_id)
        .collect();
    assert_eq!(targets, [0, 2]);
    // At a send's index it retargets that send.
    assert_eq!(create_send(&mut song, 1, 0), Some(3));
    let targets: Vec<i32> = song.channels[1]
        .file
        .sends
        .iter()
        .map(|s| s.target_channel_id)
        .collect();
    assert_eq!(targets, [3, 2]);
    assert_eq!(song.channels[1].output_bus, song.channels[3].bus_id);
}

fn sf2d_patch(rig: &Rig) -> Option<i32> {
    let blob = rig.module(2).file.custom_data.as_deref();
    blob.and_then(crate::flm::sf2d::Sf2d::read).map(|s| s.patch)
}

#[test]
fn the_patch_spinner_opens_the_picker_with_or_without_patches_to_list() {
    // Over a SoundFont's patches (the picker starts in them), and with nothing loaded (the files). A SoundFont the
    // rack could not read shows its error instead, whose tap opens the picker: `soundfont_player::error_tap`.
    let mut rig = soundfont_rig(14, Some(30));
    rig.click(control(&rig, 8.5, 1200).center());
    assert_eq!(rig.actions.soundfont, Some(2));
    assert_eq!(sf2d_patch(&rig), Some(14));
    let mut rig = soundfont_rig(0, None);
    rig.channel.rack.get_mut(2).unwrap().file.custom_data = None;
    rig.frame(vec![]);
    rig.click(control(&rig, 8.5, 1200).center());
    assert_eq!(rig.actions.soundfont, Some(2));
}

#[test]
fn the_preset_button_of_a_soundfont_player_opens_the_preset_picker_like_every_module() {
    // One preset picker for every module; a `.sf2` it lists goes to the module's own loader. The SoundFont picker is
    // the `Patch` spinner's, when there are no patches to list.
    for read in [Some(5), None] {
        let mut rig = soundfont_rig(1, read);
        rig.click(rig.header(0, -2.0));
        assert_eq!(rig.actions.picker, Some(PresetTarget::Module(2)));
        assert_eq!(rig.actions.soundfont, None);
    }
}

// ---- panel geometry ----

/// The open rack over `area` at the tests' U (32 px, ppp 1). It shadows the glob import, so the panel tests above keep
/// their one-argument calls.
fn rack_rect(area: Rect) -> Rect {
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    super::rack_rect(area, 1.0, false, &m)
}

/// The header's ≡ of the rig's rack.
fn rack_menu_at() -> Pos2 {
    Chrome::new(rack_rect(SCREEN), U).menu.center()
}

#[test]
fn the_rack_is_20_u_wide_capped_at_the_area_width_right_aligned_and_full_height() {
    // U = 32 px at ppp 1 and 64 px at ppp 2, so 20 U = 640 px and 1280 px. `w` is the window width in px.
    for (ppp, w, rack_px) in [
        (1.0, 1000.0, 640.0),
        (1.0, 2000.0, 640.0),
        (1.0, 3000.0, 640.0),
        (1.0, 500.0, 500.0),
        (2.0, 1000.0, 1000.0),
        (2.0, 2000.0, 1280.0),
        (2.0, 3000.0, 1280.0),
    ] {
        let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE);
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(w / ppp, 300.0));
        let rack = super::rack_rect(area, 1.0, false, &m);
        assert_eq!(rack.width() * ppp, rack_px, "ppp {ppp}, W {w}");
        assert_eq!(rack.right(), area.right(), "ppp {ppp}, W {w}");
        assert_eq!(rack.y_range(), area.y_range(), "ppp {ppp}, W {w}");
    }
    // Right-aligned in an area that does not start at x 0; closed, it lies just past the area's right edge.
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let area = Rect::from_min_max(pos2(100.0, 20.0), pos2(1100.0, 400.0));
    let at = |r: f32| super::rack_rect(area, r, false, &m);
    assert_eq!(
        at(1.0),
        Rect::from_min_max(pos2(460.0, 20.0), pos2(1100.0, 400.0))
    );
    assert_eq!(at(0.5).left(), 780.0);
    assert_eq!(at(0.0).left(), 1100.0);
    assert_eq!(at(0.0).width(), 640.0);
}

#[test]
fn a_portrait_window_over_20_u_wide_gets_the_full_screen_rack_and_full_width_menu_and_pickers() {
    // 900 × 1400 px at U = 32 is 28 U wide, but taller than wide.
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(900.0, 1400.0));
    let rack = crate::ui::main_screen::Regions::new(screen, &m)
        .with_rack(1.0, &m)
        .rack;
    assert_eq!(rack.x_range(), screen.x_range());
    let c = Chrome::new(rack, m.u_px());
    assert!(c.full && c.close.is_some() && c.collapse.is_none());
    assert_eq!(crate::ui::main_menu::shell_layout(screen, &m).panel, screen);
    let picker = crate::ui::modules_browser::layout(screen, 1.0, &m).panel;
    assert_eq!(picker.x_range(), screen.x_range());
    // The same width in landscape keeps the 20 U side rack.
    let wide = Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0));
    let side = crate::ui::main_screen::Regions::new(wide, &m)
        .with_rack(1.0, &m)
        .rack;
    assert_eq!(side.width(), 640.0);
    assert!(!Chrome::new(side, m.u_px()).full);
}

#[test]
fn the_rack_slides_halfway_each_frame_and_snaps_within_a_pixel() {
    assert_eq!(slide(0.0, 1.0, 1000.0, 0.5), 0.5);
    assert_eq!(slide(0.5, 1.0, 1000.0, 0.5), 0.75);
    let frames = |mut r: f32, target: f32, width: f32| {
        let mut n = 0;
        while r != target {
            r = slide(r, target, width, 0.5);
            n += 1;
            assert!(n < 100, "never snaps");
        }
        n
    };
    // 2⁻¹⁰ < 1/1000: the tenth frame snaps, opening and closing alike; 2⁻¹² < 1/3000 < 2⁻¹¹.
    assert_eq!(frames(0.0, 1.0, 1000.0), 10);
    assert_eq!(frames(1.0, 0.0, 1000.0), 10);
    assert_eq!(frames(0.0, 1.0, 3000.0), 12);
    // At rest it stays put.
    assert_eq!(slide(1.0, 1.0, 1000.0, 0.5), 1.0);
    assert_eq!(slide(0.0, 0.0, 1000.0, 0.5), 0.0);
}

#[test]
fn the_chrome_is_full_screen_below_20_u_with_margins_from_1_5_down_to_0_5_u() {
    let u = 32.0;
    // A 20 U side rack: 1.5 U margins, a 1.3 U header (its bar 2 px less), the collapse tab and no X.
    let side = Chrome::new(Rect::from_min_size(pos2(530.0, 0.0), vec2(640.0, 540.0)), u);
    assert!(!side.full && side.close.is_none());
    assert_eq!((side.margin, side.header_bottom), (48.0, 41.0));
    assert_eq!(side.bar, Rect::from_x_y_ranges(578.0..=1122.0, 0.0..=39.0));
    let tab = side.collapse.unwrap();
    assert_eq!((tab.left(), tab.width()), (538.0, 32.0));
    assert!((tab.height() - 1.6 * u).abs() < 1e-3);
    // 19 U: full screen already (narrower than 20 U), with 1 U margins.
    let narrower = Chrome::new(Rect::from_min_size(Pos2::ZERO, vec2(608.0, 540.0)), u);
    assert_eq!((narrower.full, narrower.margin), (true, 32.0));
    // Full screen on a 16.9 U phone: 0.5 U margins, a 1.75 U header with the X at its right end, no collapse tab.
    let full = Chrome::new(Rect::from_min_size(Pos2::ZERO, vec2(540.0, 900.0)), u);
    assert!(full.full && full.collapse.is_none());
    assert_eq!((full.margin, full.header_bottom), (16.0, 56.0));
    assert_eq!(
        full.close,
        Some(Rect::from_min_size(pos2(470.0, 0.0), vec2(54.0, 54.0)))
    );
    assert_eq!(full.name, Rect::from_x_y_ranges(70.0..=470.0, 0.0..=54.0));
}

#[test]
fn the_view_ends_at_the_racks_left_edge_but_keeps_1_u() {
    use crate::ui::main_screen::Regions;
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let regions = |w: f32, r: f32| {
        Regions::new(Rect::from_min_size(Pos2::ZERO, vec2(w, 540.0)), &m).with_rack(r, &m)
    };
    // Closed: the view is the whole area, and the rack lies past its right edge.
    let closed = regions(1170.0, 0.0);
    assert_eq!(closed.editor, closed.area);
    assert_eq!(closed.rack.left(), 1170.0);
    // Open: 20 U at the right, the view up to its left edge, both the area's height.
    let open = regions(1170.0, 1.0);
    assert_eq!(
        open.rack,
        Rect::from_min_max(pos2(530.0, 0.0), open.area.max)
    );
    assert_eq!(open.editor, open.area.with_max_x(530.0));
    // Halfway in, the view is already narrower.
    assert_eq!(regions(1170.0, 0.5).editor.right(), 850.0);
    // Narrower than 20 U the rack is the full width, and over the view's last 1 U.
    let narrow = regions(600.0, 1.0);
    assert_eq!(narrow.rack.x_range(), narrow.area.x_range());
    assert_eq!(narrow.editor.width(), 32.0);
    // Without the keyboard panel the area, the view and the rack reach the screen's bottom.
    let low = Regions::new(Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0)), &m)
        .without_panel()
        .with_rack(1.0, &m);
    assert_eq!(low.rack.bottom(), 540.0);
    assert_eq!(low.editor.bottom(), 540.0);
}

// ---- a long press with the mouse ----

impl Rig {
    /// Presses `pos` with the mouse and holds it still, a frame every 1/60 s, until the frame `secs` after the press,
    /// which releases when `release`.
    fn hold(&mut self, pos: Pos2, secs: f64, release: bool) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.button(pos, true);
        let start = self.time;
        while self.time + 1.0 / 60.0 < start + secs - 1e-9 {
            self.frame(vec![]);
        }
        self.time = start + secs - 1.0 / 60.0;
        if release {
            self.button(pos, false);
        } else {
            self.frame(vec![]);
        }
    }
}

#[test]
fn a_mouse_held_still_300_ms_on_the_instrument_icon_opens_the_level_popup_but_250_ms_does_not() {
    let mut rig = Rig::synth(SynthKind::MiniSynth);
    let icon = rig.header(0, 2.25);
    rig.hold(icon, 0.25, true);
    rig.frame(vec![]);
    assert!(!rig.drawn("Wet Pan"));
    rig.hold(icon, 0.3, false);
    rig.frame(vec![]);
    assert!(rig.drawn("Wet Pan"));
    rig.button(icon, false);
    rig.frame(vec![]);
    assert!(rig.drawn("Wet Pan"), "the release keeps it open");
}

// ---- module card gestures ----

impl Rig {
    /// The middle of card `i`'s module-name label, unscrolled: where its hold and double tap start.
    fn name(&self, i: usize) -> Pos2 {
        self.header(i, 5.0)
    }

    /// Holds card `i`'s name label still for 300 ms: the hold is confirmed on the last frame.
    fn grab(&mut self, i: usize) -> Pos2 {
        let at = self.name(i);
        self.hold(at, 0.3, false);
        at
    }

    /// A finger move to `to`, `dt` after the last frame.
    fn to(&mut self, dt: f64, to: Pos2) {
        self.after(dt, vec![Event::PointerMoved(to)]);
    }

    /// Frames until every swap slide is at rest.
    fn settle(&mut self) {
        for _ in 0..20 {
            self.frame(vec![]);
        }
    }

    /// The instance ids top to bottom.
    fn ids(&self) -> Vec<i32> {
        order(&self.channel.rack)
    }
}

#[test]
fn two_header_touch_downs_under_0_3_s_apart_toggle_the_card_and_the_rack_lays_out_again() {
    for (gap, toggled) in [(0.29, true), (0.31, false)] {
        let mut rig = Rig::new(&[6, 11]);
        // In the name, between the icon and the preset button.
        let name = rig.header(0, 5.0);
        let chorus = rig.text("Chorus").top();
        // Touch-downs `gap` apart, the first one released a frame after it.
        rig.click(name);
        let press = Event::PointerButton {
            pos: name,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        };
        rig.after(gap - 1.0 / 60.0, vec![press]);
        let minimized = rig.channel.rack.modules[0].minimized;
        assert_eq!(
            (minimized, rig.actions.minimized),
            (toggled, toggled),
            "{gap}"
        );
        assert!(!rig.actions.replace_song);
        rig.button(name, false);
        rig.frame(vec![]);
        // Minimized, the Distortion card is its 1.5 U header row: the Chorus card comes up by the rest of its 5.5 U.
        let up = if toggled { 4.0 * U } else { 0.0 };
        assert_eq!(rig.text("Chorus").top(), chorus - up, "{gap}");
    }
}

#[test]
fn taps_and_holds_outside_the_name_label_neither_minimize_nor_hold_the_card() {
    let mut rig = Rig::new(&[6, 11]);
    let card = rig.cards()[0];
    let params = rig.module(2).params.clone();
    // The header margin between the icon and the name, and the blank strip under the module panel.
    let margin = pos2(card.left() + 3.1 * U, card.top() + 0.75 * U);
    let blank = pos2(card.center().x, card.bottom() - 0.25 * U);
    for at in [margin, blank] {
        rig.click(at);
        rig.click(at);
        assert!(!rig.module(2).minimized, "{at:?}");
        assert!(!rig.actions.minimized, "{at:?}");
        rig.hold(at, 0.4, false);
        assert!(rig.rack.drag.is_none(), "{at:?}");
        rig.button(at, false);
    }
    assert_eq!(rig.module(2).params, params);
    // The enable toggle takes both taps (off, then on again); so does the module panel under the header.
    let toggle = rig.header(0, 0.75);
    let panel = content_rect(card, 5.5, U).center();
    for at in [toggle, panel] {
        rig.click(at);
        rig.click(at);
        assert!(!rig.module(2).minimized);
        assert!(!rig.actions.minimized);
    }
    assert_eq!(rig.module(2).params[0], 1.0);
}

#[test]
fn moving_over_0_5_u_vertically_before_300_ms_cancels_the_hold_but_sideways_does_not() {
    for (by, held) in [
        (vec2(3.0 * U, 0.45 * U), true),
        (vec2(0.0, 0.55 * U), false),
        (vec2(0.0, -0.55 * U), false),
    ] {
        let mut rig = Rig::new(&[6, 11, 15]);
        let at = rig.name(1);
        rig.frame(vec![Event::PointerMoved(at)]);
        rig.button(at, true);
        rig.after(0.1, vec![Event::PointerMoved(at + by)]);
        rig.after(0.2, vec![]);
        assert_eq!(rig.rack.drag.is_some(), held, "{by:?}");
    }
}

#[test]
fn a_confirmed_hold_shows_every_card_minimized_keeps_the_held_one_in_place_and_release_restores_them()
 {
    let mut rig = Rig::new(&[6, 11, 15, 18]);
    let before = rig.cards();
    let (name, next) = (rig.text("Stereoizer").top(), rig.text("Tape Delay").top());
    let at = rig.grab(2);
    rig.frame(vec![]);
    // Shown minimized, all of them: the song is untouched (no minimized flag, no edit to undo).
    assert!(rig.channel.rack.modules.iter().all(|m| !m.minimized));
    assert!(!rig.actions.minimized && !rig.actions.replace_song);
    assert!(!rig.drawn("Drive"), "no module panel shows");
    // The held card keeps its place: the offset moved by its top's shift (164 − 420) and is not clamped.
    assert_eq!(rig.rack.scroll, -256.0);
    assert_eq!(rig.shown()[2].top(), before[2].top());
    assert_eq!(rig.shown()[2].height(), 1.5 * U);
    // As drawn: its name where it was, the next card a minimized slot (1.5 U and an 11 px gap) under it.
    assert_eq!(rig.text("Stereoizer").top(), name);
    assert_eq!(rig.text("Tape Delay").top(), name + 59.0);
    assert!(next > name + 59.0);
    // Released without a drag: full cards, its top far down the viewport is past p + 0.9 H, so p takes the shift
    // back, and the clamp is on again.
    rig.button(at, false);
    rig.frame(vec![]);
    assert!(rig.drawn("Drive"));
    assert_eq!(rig.rack.scroll, 0.0);
    assert_eq!(rig.shown(), before);
}

#[test]
fn a_release_moves_the_offset_back_by_the_held_cards_shift_when_it_lands_out_of_range() {
    // Four 10 U cards scrolled to 400: card 2 shows at 708 − 400 = 308. Held, p = 400 + (164 − 708) = −144.
    let mut rig = Rig::new(&[3, 7, 26, 30]);
    rig.rack.scroll = 400.0;
    rig.frame(vec![]);
    let top = rig.shown()[2].top();
    assert_eq!(top, 308.0);
    let at = pos2(rig.name(2).x, top + 0.75 * U);
    rig.hold(at, 0.3, false);
    assert_eq!(rig.rack.scroll, -144.0);
    // Let go: full again its top is 852, 811 down the viewport, past p + 0.9 H = 539.1: p += 852 − 308.
    rig.button(at, false);
    assert_eq!(rig.rack.scroll, 400.0);
    rig.frame(vec![]);
    assert_eq!(rig.shown()[2].top(), 308.0);
}

#[test]
fn the_held_card_has_a_card_gap_wide_channel_coloured_outline_outside_it_until_release() {
    let mut rig = Rig::new(&[6, 11, 15]);
    let ink = channel_color(rig.channel.hue, &theme_by_id(0)).to_color32();
    let outlines = |rig: &Rig| -> Vec<Rect> {
        (rig.shapes.iter())
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r)
                    if r.stroke == Stroke::new(11.0_f32, ink)
                        && r.stroke_kind == StrokeKind::Outside =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect()
    };
    assert_eq!(outlines(&rig), []);
    let at = rig.grab(1);
    rig.frame(vec![]);
    assert_eq!(outlines(&rig), [rig.shown()[1]]);
    rig.button(at, false);
    rig.frame(vec![]);
    assert_eq!(outlines(&rig), []);
}

#[test]
fn the_drag_starts_at_a_0_5_u_vertical_move_and_a_still_finger_steps_nothing() {
    let mut rig = Rig::new(&[6, 11, 15, 18]);
    // Held by the top of its name label, so that 0.4 U up is already out of its band (5 px above its top).
    let top = rig.cards()[1].top();
    let at = pos2(rig.name(1).x, top + 5.0);
    rig.hold(at, 0.3, false);
    assert_eq!(rig.shown()[1].top(), top);
    for _ in 0..10 {
        rig.after(0.2, vec![]);
    }
    // Under 0.5 U up, or sideways anywhere: the drag has not started.
    for to in [
        at - vec2(0.0, 0.4 * U),
        at + vec2(-6.0 * U, -0.45 * U),
        at + vec2(8.0 * U, 0.3 * U),
    ] {
        rig.to(0.5, to);
        assert_eq!(rig.ids(), [2, 3, 4, 5], "{to:?}");
    }
    // Past 0.5 U it starts, and the finger out of the band above steps the card up at once.
    rig.to(0.5, at - vec2(0.0, 0.55 * U));
    assert_eq!(rig.ids(), [3, 2, 4, 5]);
    assert!(rig.actions.replace_song);
    rig.button(at - vec2(0.0, 0.55 * U), false);
    rig.frame(vec![]);

    // Far above, then still for 2 s: one step only, no timer repeats.
    rig.settle();
    rig.grab(3);
    rig.to(0.1, pos2(at.x, 1.0));
    assert_eq!(rig.ids(), [3, 2, 5, 4]);
    for _ in 0..10 {
        rig.after(0.2, vec![]);
    }
    assert_eq!(rig.ids(), [3, 2, 5, 4]);
}

#[test]
fn next_to_the_held_card_in_the_card_gaps_or_beside_it_nothing_steps() {
    let mut rig = Rig::new(&[6, 11, 15]);
    rig.grab(1);
    let c = rig.shown()[1];
    // The gaps are 11 px.
    let near = [
        pos2(c.center().x, c.top() - 11.0),
        pos2(c.center().x, c.top() + 1.5 * U + 11.0),
        pos2(c.left() - 1.0 * U, c.top() + 0.75 * U),
        pos2(c.right() + 1.0 * U, c.top() - 4.0),
        pos2(c.right() + 1.0 * U, c.top() + 1.5 * U + 4.0),
    ];
    for _ in 0..3 {
        for to in near {
            rig.to(0.2, to);
            assert_eq!(rig.ids(), [2, 3, 4], "{to:?}");
        }
    }
    // A pixel further: up above, down below.
    rig.to(0.4, pos2(c.center().x, c.top() - 12.0));
    assert_eq!(rig.ids(), [3, 2, 4]);
    rig.settle();
    let c = rig.shown()[0];
    rig.to(0.4, pos2(c.center().x, c.top() + 1.5 * U + 12.0));
    assert_eq!(rig.ids(), [2, 3, 4]);
}

#[test]
fn the_step_interval_is_345_ms_on_the_new_top_and_105_ms_1_5_u_away_or_more() {
    let d = |y: f32| step_interval(100.0, y, U);
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    // 0.3·(1.15 − 0.8·min(1, |fy − y′| / 1.5 U)).
    assert!(near(d(100.0), 0.345), "{}", d(100.0));
    assert!(near(d(124.0), 0.225), "{}", d(124.0));
    assert!(near(d(76.0), 0.225), "{}", d(76.0));
    for y in [148.0, 52.0, 1000.0, -1000.0] {
        assert!(near(d(y), 0.105), "{y}: {}", d(y));
    }
}

#[test]
fn moves_within_d_of_a_step_are_dropped_not_queued() {
    // Minimized cards: 59 px apart (tops 46, 105, 164, 223, ...), and the hold moves no offset.
    let mut rig = Rig::new(&[6, 11, 15, 18, 10, 12]);
    rig.minimize_all();
    let x = rig.name(3).x;
    rig.grab(3);
    // Far above: up into slot 2, and its new top is over 1.5 U from the finger: d = 105 ms.
    rig.to(0.1, pos2(x, 20.0));
    assert_eq!(rig.ids(), [2, 3, 5, 4, 6, 7]);
    // The swap slides: the moved card from its old place by its neighbour's 1.5 U, the neighbour the other way.
    assert_eq!(rig.rack.slides.get(&5), Some(&1.5));
    assert_eq!(rig.rack.slides.get(&4), Some(&-1.5));
    rig.to(0.05, pos2(x, 21.0));
    rig.to(0.05, pos2(x, 20.0));
    assert_eq!(rig.ids(), [2, 3, 5, 4, 6, 7]);
    // Not queued: a still finger after them steps nothing; the next move does.
    for _ in 0..5 {
        rig.after(0.1, vec![]);
    }
    assert_eq!(rig.ids(), [2, 3, 5, 4, 6, 7]);
    rig.to(0.1, pos2(x, 21.0));
    assert_eq!(rig.ids(), [2, 5, 3, 4, 6, 7]);

    // Close: onto the slot above, so the new top is the finger's height: d = 345 ms.
    rig.settle();
    rig.to(0.1, pos2(x, 46.0));
    assert_eq!(rig.ids(), [5, 2, 3, 4, 6, 7]);
    rig.to(0.3, pos2(x, 200.0));
    assert_eq!(rig.ids(), [5, 2, 3, 4, 6, 7]);
    rig.to(0.05, pos2(x, 201.0));
    assert_eq!(rig.ids(), [2, 5, 3, 4, 6, 7]);
}

#[test]
fn past_90_percent_of_the_viewport_a_step_moves_the_offset_by_the_cards_shift() {
    // Four 10 U cards; held, they show minimized and p = 105 − 377 = −272: [p, p + 0.9 H] = [−272, 411.1] (H = 759).
    let mut rig = Rig::new(&[3, 7, 26, 30]);
    rig.grab(1);
    assert_eq!(rig.rack.scroll, -272.0);
    let x = rig.name(1).x;
    // Down into slot 2: its new top is 436 − 41 = 395 down the viewport, in range.
    rig.to(0.1, pos2(x, 700.0));
    assert_eq!(rig.ids(), [2, 4, 3, 5]);
    assert_eq!(rig.rack.scroll, -272.0);
    rig.settle();
    // Into slot 3: 454 is past 411.1, so p moves by y′ − y = 59 and the card keeps its place on screen.
    rig.to(0.1, pos2(x, 701.0));
    assert_eq!(rig.ids(), [2, 4, 5, 3]);
    assert_eq!(rig.rack.scroll, -213.0);
    rig.frame(vec![]);
    assert_eq!(rig.shown()[3].top(), 436.0);
}

#[test]
fn above_the_offset_a_step_moves_it_by_the_cards_shift_too() {
    // 15 minimized cards, scrolled to 150: card 3 shows at 73, 32 px down the viewport, which is less than p.
    let mut rig = Rig::new(&[6; 15]);
    rig.minimize_all();
    rig.rack.scroll = 150.0;
    rig.frame(vec![]);
    let top = rig.shown()[3].top();
    assert_eq!(top, 73.0);
    let at = pos2(rig.name(3).x, top + 0.75 * U);
    rig.hold(at, 0.3, false);
    assert_eq!(rig.rack.scroll, 150.0);
    rig.to(0.1, pos2(at.x, 1.0));
    assert_eq!(rig.ids()[2], 5);
    assert_eq!(rig.rack.scroll, 91.0);
    rig.frame(vec![]);
    assert_eq!(rig.shown()[2].top(), 73.0);
}

#[test]
fn neither_the_finger_nor_the_wheel_scrolls_a_held_rack() {
    let mut rig = Rig::new(&[3, 7, 26, 30]);
    let at = rig.grab(1);
    let p = rig.rack.scroll;
    let wheel = Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, -400.0),
        modifiers: Default::default(),
    };
    rig.after(
        0.1,
        vec![Event::PointerMoved(at + vec2(2.0 * U, 0.3 * U)), wheel],
    );
    for _ in 0..30 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.rack.scroll, p);
}

#[test]
fn a_focus_loss_cancels_the_hold_and_restores_the_cards() {
    let mut rig = Rig::new(&[6, 11, 15]);
    let before = rig.cards();
    rig.grab(1);
    assert_eq!(rig.rack.scroll, -128.0);
    rig.frame(vec![Event::WindowFocused(false)]);
    assert!(rig.rack.drag.is_none());
    // The offset the hold left springs back into range by the scroll law, a frame on: 10 % of the overshoot a frame.
    rig.frame(vec![]);
    assert!(rig.drawn("Drive"));
    assert!(
        (rig.rack.scroll + 128.0 * 0.9).abs() < 1e-3,
        "{}",
        rig.rack.scroll
    );
    for _ in 0..150 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.rack.scroll, 0.0);
    assert_eq!(rig.shown(), before);
}

#[test]
fn a_menu_move_slides_both_cards_from_their_old_places_decaying_0_7_a_frame() {
    let mut rig = Rig::new(&[6, 11]);
    let top = rig.text("Distortion").top();
    rig.open(rig.header(0, -0.75));
    let at = rig.text("Move down").center();
    rig.click(at);
    assert_eq!(rig.ids(), [3, 2]);
    // The moved card (5.5 U tall Chorus above it now) starts 5.5 U up, toward its old place; the Chorus 5.5 U down.
    assert_eq!(rig.rack.slides.get(&2), Some(&-5.5));
    assert_eq!(rig.rack.slides.get(&3), Some(&5.5));
    rig.frame(vec![]);
    // Drawn at its new top − 5.5 U: a card gap (11 px) under where it was.
    assert_eq!(rig.text("Distortion").top(), top + 11.0);
    let left = |rig: &Rig| rig.rack.slides.get(&2).copied();
    assert!((left(&rig).unwrap() + 5.5 * 0.7).abs() < 1e-5);
    // 176 px · 0.7^14 = 1.2 px still slides; · 0.7 = 0.83 px snaps to 0.
    for _ in 0..13 {
        rig.frame(vec![]);
        assert_eq!(rig.repaint, std::time::Duration::ZERO);
    }
    assert!(left(&rig).is_some());
    rig.frame(vec![]);
    assert_eq!((left(&rig), rig.rack.slides.get(&3)), (None, None));
}

#[test]
fn a_header_press_that_leaves_its_calm_zone_drags_the_rack_like_a_press_anywhere_else() {
    let mut rig = Rig::new(&[3, 7, 26, 30]);
    let at = rig.header(1, 5.0);
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.button(at, true);
    for i in 1..=4 {
        rig.after(
            0.05,
            vec![Event::PointerMoved(at - vec2(0.0, i as f32 * 0.5 * U))],
        );
    }
    // Up 2 U before its 300 ms: no hold, and the content follows the finger once it is out of the 0.5 U calm zone, from
    // the move that left it.
    assert_eq!(rig.rack.scroll, 1.5 * U);
    rig.after(0.5, vec![Event::PointerMoved(at - vec2(0.0, 6.0 * U))]);
    assert_eq!(types(&rig.channel.rack), [3, 7, 26, 30]);
    assert_eq!(rig.rack.scroll, 5.5 * U);
}

// ---- `Fine tune control` ----

/// Opens the parameter menu of the control at `at` and picks `Fine tune control` when it offers it (the touch the pick
/// reported), else closes the menu with Escape (`None`).
fn fine_tune(rig: &mut Rig, at: Pos2) -> Option<Touch> {
    rig.secondary_click(at);
    rig.frame(vec![]);
    assert!(rig.drawn("Add automation track"), "the menu is open");
    if !rig.drawn("Fine tune control") {
        rig.frame(vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        return None;
    }
    let item = rig.text("Fine tune control").center();
    rig.frame(vec![Event::PointerMoved(item)]);
    rig.button(item, true);
    rig.button(item, false);
    rig.actions.touch
}

/// The touch of `Fine tune control` on `target`.
fn tuned(target: AutomationTarget) -> Option<Touch> {
    Some(Touch {
        target,
        pick: Some(CtrlAction::FineTune),
    })
}

#[test]
fn fine_tune_control_is_offered_on_a_cards_knob_and_the_common_dialogs_slider_which_hands_over() {
    let mut rig = Rig::new(&[6]);
    let drive = rig.drive();
    assert_eq!(fine_tune(&mut rig, drive), tuned(target(2, 1)));
    // The enable toggle is two-state: no item.
    let toggle = rig.header(0, 0.75);
    assert_eq!(fine_tune(&mut rig, toggle), None);
    // The common dialog's Mix (−2) offers it; its pick closes the dialog, which would stay live above the fine tune
    // dialog's dim.
    let icon = rig.header(0, 2.25);
    rig.click(icon);
    rig.frame(vec![]);
    let mix = rig.text("Mix").center();
    assert_eq!(
        fine_tune(&mut rig, mix),
        tuned(target(2, AutomationTarget::MIX))
    );
    rig.frame(vec![]);
    assert!(!rig.drawn("Wet Pan"), "the level dialog closed");
}

#[test]
fn a_send_rows_level_slider_offers_fine_tune_control_and_its_toggle_does_not() {
    let mut rig = send_rig();
    let r = rig.sends_flow().rows[1];
    let level = pos2(
        r.right() - 2.0 * U - 0.25 * (r.width() - 5.5 * U),
        r.center().y,
    );
    assert_eq!(fine_tune(&mut rig, level), tuned(target(0, 0x2000 + 5)));
    let toggle = rig.send_item(1, 0.75);
    assert_eq!(fine_tune(&mut rig, toggle), None);
}

// ---- module card gestures: thresholds and edges (RK minors) ----

#[test]
fn a_second_finger_cancels_the_hold_and_restores_the_cards() {
    use egui::{TouchDeviceId, TouchId, TouchPhase};
    let touch = |id, phase, pos| Event::Touch {
        device_id: TouchDeviceId(1),
        id: TouchId(id),
        phase,
        pos,
        force: None,
    };
    let mut rig = Rig::new(&[6, 11, 15]);
    let before = rig.cards();
    let at = rig.name(1);
    // The first finger: its touch with the pointer events an integration sends, held 300 ms.
    rig.frame(vec![Event::PointerMoved(at)]);
    let press = Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    };
    rig.frame(vec![touch(1, TouchPhase::Start, at), press]);
    rig.after(0.3, vec![]);
    assert!(rig.rack.drag.is_some());
    // A second finger: its touch event only.
    rig.frame(vec![touch(2, TouchPhase::Start, at + vec2(3.0 * U, 0.0))]);
    assert!(rig.rack.drag.is_none());
    rig.frame(vec![]);
    assert!(rig.drawn("Drive"));
    // The scroller has let go of the touch with the hold: the offset springs back while the fingers stay down.
    assert!(
        (rig.rack.scroll + 128.0 * 0.9).abs() < 1e-3,
        "{}",
        rig.rack.scroll
    );
    for _ in 0..150 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.rack.scroll, 0.0);
    assert_eq!(rig.shown(), before);
}

#[test]
fn the_0_9_h_rule_measures_down_the_viewport_below_the_header_bar() {
    // 15 minimized cards scrolled to 100: card 4 (top 182) steps up into slot 3, whose top 123 is 82 down the
    // viewport, above p = 100, so p moves by 123 − 182. (From the rack's own top, 123, it would be in range.)
    let mut rig = Rig::new(&[6; 15]);
    rig.minimize_all();
    rig.rack.scroll = 100.0;
    rig.frame(vec![]);
    let top = rig.shown()[4].top();
    assert_eq!(top, 182.0);
    let at = pos2(rig.name(4).x, top + 0.75 * U);
    rig.hold(at, 0.3, false);
    rig.to(0.1, pos2(at.x, 1.0));
    assert_eq!(rig.ids()[3], 6);
    assert_eq!(rig.rack.scroll, 41.0);

    // Scrolled to 20: card 12 (top 734) steps down into slot 13, whose top 793 is 752 down the viewport, past
    // p + 0.9 H = 703.1 with H the 759 px under the header.
    let mut rig = Rig::new(&[6; 15]);
    rig.minimize_all();
    rig.rack.scroll = 20.0;
    rig.frame(vec![]);
    let top = rig.shown()[12].top();
    assert_eq!(top, 734.0);
    let at = pos2(rig.name(12).x, top + 0.75 * U);
    rig.hold(at, 0.3, false);
    rig.to(0.1, pos2(at.x, 798.0));
    assert_eq!(rig.ids()[13], 14);
    assert_eq!(rig.rack.scroll, 79.0);
}

#[test]
fn a_release_moves_the_offset_only_past_its_thresholds() {
    // (cards, how many from the second on are minimized, the held card, p before, p as the release leaves it)
    let cases: [(&[u16], usize, usize, f32, f32); 4] = [
        // Four 10 U cards, card 1 held at p = 98: p = −174, and let go its full top is 510 down the viewport, past
        // p + 0.9 H = 509.1: p += 272. From 99 it lands at 509, short of 510.1: p stays.
        (&[3, 7, 26, 30], 0, 1, 98.0, 98.0),
        (&[3, 7, 26, 30], 0, 1, 99.0, -173.0),
        // A 10 U card over 20 minimized ones, card 6 held at p = 588: p = 316, and its full top lands 315 down the
        // viewport, above p: p += 272. From 587 it lands at 316, not above 315: p stays.
        (&[3; 21], 20, 6, 588.0, 588.0),
        (&[3; 21], 20, 6, 587.0, 315.0),
    ];
    for (types, minimized, held, p, after) in cases {
        let mut rig = Rig::new(types);
        for module in &mut rig.channel.rack.modules[1..=minimized] {
            module.minimized = true;
        }
        rig.rack.scroll = p;
        rig.frame(vec![]);
        assert_eq!(rig.rack.scroll, p);
        let at = pos2(rig.name(0).x, rig.shown()[held].top() + 0.75 * U);
        rig.hold(at, 0.3, false);
        assert!(rig.rack.drag.is_some(), "{p}");
        rig.button(at, false);
        assert_eq!(rig.rack.scroll, after, "{p}");
    }
}

#[test]
fn a_menu_move_between_cards_of_different_heights_slides_each_by_the_others_height() {
    // Distortion (5.5 U) over an Equalizer (10 U).
    let mut rig = Rig::new(&[6, 3]);
    let (distortion, equalizer) = (rig.text("Distortion").top(), rig.text("Equalizer").top());
    rig.open(rig.header(0, -0.75));
    let at = rig.text("Move down").center();
    rig.click(at);
    assert_eq!(rig.ids(), [3, 2]);
    // The moved card by the Equalizer's 10 U toward its old place, the Equalizer by the Distortion's 5.5 U.
    assert_eq!(rig.rack.slides.get(&2), Some(&-10.0));
    assert_eq!(rig.rack.slides.get(&3), Some(&5.5));
    rig.frame(vec![]);
    // Each a card gap past its old place: the slide is a height, not the 11 px longer slot.
    assert_eq!(rig.text("Distortion").top(), distortion + 11.0);
    assert_eq!(rig.text("Equalizer").top(), equalizer - 11.0);
}

#[test]
fn the_held_card_draws_over_the_others() {
    let mut rig = Rig::new(&[6, 11, 15]);
    let at = rig.grab(0);
    rig.frame(vec![]);
    let index = |rig: &Rig, text: &str| {
        (rig.shapes.iter())
            .position(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text() == text))
            .unwrap()
    };
    assert!(index(&rig, "Distortion") > index(&rig, "Stereoizer"));
    rig.button(at, false);
    rig.frame(vec![]);
    assert!(index(&rig, "Distortion") < index(&rig, "Chorus"));
}

// ---- the scroll law (through `ui::scroller`) ----

/// A wheel of `dy` points.
fn wheel(dy: f32) -> Event {
    Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, dy),
        modifiers: Default::default(),
    }
}

impl Rig {
    /// A spot in the rack's left gutter, under the notch: only the rack takes a touch there.
    fn gutter(&self) -> Pos2 {
        let r = rack_rect(SCREEN);
        pos2(r.left() + 0.75 * U, r.bottom() - 2.0 * U)
    }

    /// Wheels the rack to its end and returns the offset there.
    fn wheel_to_end(&mut self) -> f32 {
        self.frame(vec![Event::PointerMoved(self.gutter()), wheel(-5000.0)]);
        self.rack.scroll
    }
}

#[test]
fn past_either_end_the_content_follows_the_finger_a_fifth_of_its_way_then_springs_back() {
    let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
    let mut rig = Rig::new(&[3, 7, 26, 30]);
    let at = rig.gutter();
    // At the top, a pull down 4 U in 1 U moves: the first leaves the 0.5 U calm zone, and each moves the content 0.2 U.
    rig.drag(at, vec2(0.0, 4.0 * U));
    assert!(close(rig.rack.scroll, -0.8 * U), "{}", rig.rack.scroll);
    // Let go: the overshoot shrinks 10 % a frame, the lift's frame included, until it is back on the end.
    rig.button(at + vec2(0.0, 4.0 * U), false);
    assert!(
        close(rig.rack.scroll, -0.8 * U * 0.9),
        "{}",
        rig.rack.scroll
    );
    rig.frame(vec![]);
    assert!(
        close(rig.rack.scroll, -0.8 * U * 0.81),
        "{}",
        rig.rack.scroll
    );
    assert_eq!(rig.repaint, std::time::Duration::ZERO);
    for _ in 0..150 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.rack.scroll, 0.0);
    // At the end, a push up 4 U: 0.8 U past it, then back.
    let end = rig.wheel_to_end();
    let heights: Vec<f32> = rig.channel.rack.modules.iter().map(card_height).collect();
    let bottom = flow(rack_rect(SCREEN), 0.0, U, false, &heights, true).bottom;
    assert_eq!(end, bottom - rack_rect(SCREEN).bottom());
    rig.drag(at, vec2(0.0, -4.0 * U));
    assert!(close(rig.rack.scroll, end + 0.8 * U), "{}", rig.rack.scroll);
    rig.button(at - vec2(0.0, 4.0 * U), false);
    assert!(
        close(rig.rack.scroll, end + 0.8 * U * 0.9),
        "{}",
        rig.rack.scroll
    );
    for _ in 0..150 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.rack.scroll, end);
}

#[test]
fn a_fast_lift_flings_the_rack_on_and_the_fling_decays_0_9_a_frame() {
    let mut rig = Rig::new(&[3, 7, 26, 30]);
    let at = rig.gutter();
    // Up 4 U in 1 U moves: the content follows 1:1 once out of the calm zone, the move that left it included.
    rig.drag(at, vec2(0.0, -4.0 * U));
    assert_eq!(rig.rack.scroll, 4.0 * U);
    // Let go a frame after the last 1 U move: v = 1 U a frame, × 0.9 before each step, the lift's frame included.
    rig.button(at - vec2(0.0, 4.0 * U), false);
    let mut p = 4.0 * U;
    for v in [0.9 * U, 0.81 * U, 0.729 * U] {
        assert!(
            (rig.rack.scroll - (p + v)).abs() < 1e-3,
            "{}",
            rig.rack.scroll
        );
        assert_eq!(rig.repaint, std::time::Duration::ZERO);
        p = rig.rack.scroll;
        rig.frame(vec![]);
    }
    for _ in 0..200 {
        rig.frame(vec![]);
    }
    // It comes to rest 9 U on (v·0.9/(1 − 0.9)), short of the end.
    assert!(
        (rig.rack.scroll - 13.0 * U).abs() < 0.05,
        "{}",
        rig.rack.scroll
    );
    let rest = rig.rack.scroll;
    rig.frame(vec![]);
    assert_eq!(rig.rack.scroll, rest);
}

#[test]
fn a_touch_inside_the_calm_zone_scrolls_nothing_and_a_tap_still_reaches_the_card() {
    let mut rig = Rig::new(&[3, 7, 26, 30]);
    // 0.4 U up and let go: still calm.
    let at = rig.gutter();
    rig.drag(at, vec2(0.0, -0.4 * U));
    rig.button(at - vec2(0.0, 0.4 * U), false);
    rig.frame(vec![]);
    assert_eq!(rig.rack.scroll, 0.0);
    // A tap that wobbles 3 px is the enable toggle's.
    let toggle = rig.header(0, 0.75);
    rig.frame(vec![Event::PointerMoved(toggle)]);
    rig.button(toggle, true);
    rig.frame(vec![Event::PointerMoved(toggle - vec2(0.0, 3.0))]);
    rig.button(toggle - vec2(0.0, 3.0), false);
    assert_eq!(rig.module(2).params[0], 0.0);
    rig.frame(vec![]);
    assert_eq!(rig.rack.scroll, 0.0);
}

#[test]
fn a_held_cards_drag_neither_scrolls_nor_flings_the_rack() {
    let mut rig = Rig::new(&[6, 11, 15]);
    let at = rig.grab(0);
    // Down 20 px inside the held card's band (no step), and let go at once.
    let to = at + vec2(0.0, 20.0);
    rig.to(0.02, to);
    assert!(rig.rack.drag.is_some_and(|d| d.started));
    rig.button(to, false);
    for _ in 0..3 {
        rig.frame(vec![]);
        assert_eq!(rig.rack.scroll, 0.0);
    }
}

#[test]
fn the_wheel_scrolls_by_its_delta_at_once_clamped_without_a_rubber_band_and_stops_a_fling() {
    let mut rig = Rig::new(&[3, 7, 26, 30]);
    let at = rig.gutter();
    rig.frame(vec![Event::PointerMoved(at), wheel(-100.0)]);
    assert_eq!(rig.rack.scroll, 100.0);
    rig.frame(vec![wheel(300.0)]);
    assert_eq!(rig.rack.scroll, 0.0);
    // Flinging: a wheel event stops it where it puts the content.
    rig.drag(at, vec2(0.0, -4.0 * U));
    rig.button(at - vec2(0.0, 4.0 * U), false);
    let p = rig.rack.scroll;
    rig.frame(vec![wheel(-10.0)]);
    assert_eq!(rig.rack.scroll, p + 10.0);
    rig.frame(vec![]);
    assert_eq!(rig.rack.scroll, p + 10.0);
}

#[test]
fn the_cards_get_the_app_octave_offset_of_set_middle_c_to_c4() {
    let mut rig = Rig::new(&[24]);
    let offset = |rig: &Rig| {
        let card = &rig.rack.panels[&(CHANNEL, 2, 24)];
        card.state.flags.get(panels::flag::OCTAVE_OFFSET).copied()
    };
    assert_eq!(offset(&rig), Some(0));
    rig.rack.middle_c_is_c4 = true;
    rig.frame(vec![]);
    assert_eq!(offset(&rig), Some(1));
}

#[test]
fn another_channels_rack_keeps_what_the_host_set_for_the_frame() {
    let mut rig = Rig::new(&[24]);
    let peaks = Some([0.5; METER_RINGS]);
    (rig.rack.middle_c_is_c4, rig.rack.covered) = (true, true);
    (rig.rack.multitrack, rig.rack.input_peaks) = (true, peaks);
    rig.index = CHANNEL + 1;
    rig.frame(vec![]);
    let card = &rig.rack.panels[&(CHANNEL + 1, 2, 24)];
    assert_eq!(card.state.flags.get(panels::flag::OCTAVE_OFFSET), Some(&1));
    assert!(rig.rack.covered && rig.rack.multitrack);
    assert_eq!(rig.rack.input_peaks, peaks);
}

#[test]
fn the_wheel_scrolls_no_rack_under_its_open_menu_or_an_overlay() {
    let mut rig = Rig::new(&[3, 7, 26, 30]);
    let at = rig.gutter();
    rig.open(rig.header(0, -0.75));
    assert!(rig.rack.card_menu.is_some());
    rig.frame(vec![Event::PointerMoved(at), wheel(-100.0)]);
    assert_eq!(rig.rack.scroll, 0.0);
    rig.frame(vec![Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    rig.frame(vec![]);
    assert!(rig.rack.card_menu.is_none());
    // The host says an overlay is under the pointer.
    rig.rack.covered = true;
    rig.frame(vec![wheel(-100.0)]);
    assert_eq!(rig.rack.scroll, 0.0);
    rig.rack.covered = false;
    rig.frame(vec![wheel(-100.0)]);
    assert_eq!(rig.rack.scroll, 100.0);
}

#[test]
fn the_dialog_a_long_press_opened_keeps_the_fingers_drag_from_the_rack() {
    let mut rig = Rig::synth(SynthKind::MiniSynth);
    for t in [3, 7, 26] {
        rig.channel.rack.add(t);
    }
    let icon = rig.header(0, 2.25);
    rig.hold(icon, 0.3, false);
    rig.frame(vec![]);
    assert!(rig.drawn("Wet Pan"));
    for i in 1..=4 {
        rig.to(0.05, icon - vec2(0.0, i as f32 * 0.5 * U));
    }
    rig.button(icon - vec2(0.0, 2.0 * U), false);
    rig.frame(vec![]);
    assert_eq!(rig.rack.scroll, 0.0);
}

#[test]
fn at_ppp_2_the_law_runs_in_points_and_the_end_is_the_last_device_pixel_row() {
    let mut rig = Rig::new(&[3, 7, 26, 30]);
    rig.ppp = 2.0;
    rig.frame(vec![]);
    // The calm zone is 0.5 U = 16 points: up 2 U in 0.5 U moves, the second leaves it.
    let at = rig.gutter();
    for i in 0..=4 {
        let to = at - vec2(0.0, i as f32 * 0.5 * U);
        rig.frame(vec![Event::PointerMoved(to)]);
        if i == 0 {
            rig.button(at, true);
        }
    }
    assert_eq!(rig.rack.scroll, 1.5 * U);
    rig.button(at - vec2(0.0, 2.0 * U), false);
    // The end: the flow's bottom (device pixels, U = 64 px) on the rack's, in points.
    let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE);
    let px = super::rack_rect(SCREEN, 1.0, false, &m) * 2.0;
    let heights: Vec<f32> = rig.channel.rack.modules.iter().map(card_height).collect();
    let bottom = flow(px, 0.0, m.u_px(), false, &heights, true).bottom;
    assert_eq!(rig.wheel_to_end(), (bottom - px.bottom()) / 2.0);
}

#[test]
fn the_instruments_picker_names_the_channel_by_the_preset_or_by_instrument_and_id_unless_the_user_renamed_it()
 {
    let mut song = Song::default();
    let at = song.insert_channel(1, ChannelKind::Instrument).unwrap();
    let id = song.channels[at].id;
    let name = |song: &Song| {
        (
            song.channels[at].name.clone(),
            song.channels[at].file.preset_name.clone(),
        )
    };
    // The first pick renames `Instrument` (read as no user rename).
    assert_eq!(song.channels[at].name, "Instrument");
    name_instrument_channel(&mut song, at, "Ice Pluck", "SuperSaw");
    assert_eq!(name(&song), ("Ice Pluck".into(), "Ice Pluck".into()));
    name_instrument_channel(&mut song, at, "Frost Chime", "GMSynth");
    assert_eq!(name(&song), ("Frost Chime".into(), "Frost Chime".into()));
    // An empty or `Default` preset name: `<instrument name> <channel id>`, and the next pick renames that too.
    name_instrument_channel(&mut song, at, "Default", "GMSynth");
    assert_eq!(name(&song), (format!("GMSynth {id}"), String::new()));
    name_instrument_channel(&mut song, at, "", "MiniSynth");
    assert_eq!(name(&song).0, format!("MiniSynth {id}"));
    name_instrument_channel(&mut song, at, "Felt Keys", "MiniSynth");
    assert_eq!(name(&song).0, "Felt Keys");
    // A name the user gave stays; the preset is still the channel's.
    song.channels[at].name = "My Lead".into();
    name_instrument_channel(&mut song, at, "Wood Mallet", "SuperSaw");
    assert_eq!(name(&song), ("My Lead".into(), "Wood Mallet".into()));
    name_instrument_channel(&mut song, at, "Default", "SuperSaw");
    assert_eq!(name(&song).0, "My Lead");
    // A second `Instrument` channel, numbered, is automatic too.
    let mut song = Song::default();
    song.insert_channel(1, ChannelKind::Instrument).unwrap();
    let second = song.insert_channel(2, ChannelKind::Instrument).unwrap();
    assert_eq!(song.channels[second].name, "Instrument 2");
    name_instrument_channel(&mut song, second, "Low Hum", "Transistor Bass");
    assert_eq!(song.channels[second].name, "Low Hum");
}

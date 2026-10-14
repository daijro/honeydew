//! Our song templates, `Templates/<Name>.songtmpl`: songs of ours built with the `Song` API by a small [`Builder`] and
//! written with [`flm::write`], so SONGS `New` opens them through the `.songtmpl` load as any template. Our kits
//! ([`super::kits`]) and presets ([`super::presets::find`]) play them; [`more`] holds the rest, chained into
//! [`templates`].
//!
//! Details:
//! - The names are the menu's nine: generic genre words that pin its byte order.
//! - A genre template has one drum clip and one clip per synth, with patterns of our own. Each clip starts at tick 0,
//!   loops its pattern of 1 or 2 bars and shows 2 bars, and the song's play loop is those 2 bars, so Play loops the
//!   groove. Audio Recording has no clip.
//! - Steps are sixteenths (`TDIV` 4/4). A swung template moves its odd sixteenths as the drum editor's Swing does
//!   ([`apply_drum_swing`], the drum lane's `swing` set to match), the synths' notes with them.
//! - Levels: each channel's Level is set in dB so that a 2-bar render peaks in [−30, −1] dBFS: a [−30, 0) target
//!   with 1 dB of margin (`tests/content_templates.rs`).
//! - The GUIDs are zero, so a template is the same bytes on every read; a song made from one gets its own
//!   (`Song::new_guids` in the `.songtmpl` load).

use honeydew_audio::sequencer::apply_drum_swing;

mod more;

use super::kits::{self, level};
use super::presets::find;
use super::{Entry, TEMPLATES};
use crate::flm;
use crate::params::module_spec;
use crate::song::{ChannelKind, Clip, Event, PPQ, SendState, Song, SynthKind, output_bus};
use crate::ui::rack::name_song_channel;

/// Ticks a sixteenth.
const STEP: u32 = PPQ / 4;
/// Every clip shows two bars.
const BARS: u32 = 2;
/// Reverb 2.
const REVERB: u16 = 35;
/// Chorus.
const CHORUS: u16 = 11;

/// A song being built: each part is a new channel. `.effect`, `.level` and `.reverb_send` apply to the last channel
/// added, which after a `.reverb_send` that made the `Reverb` send channel is that channel: give a part its level and
/// effects before its send, as the templates do.
pub(super) struct Builder {
    song: Song,
    /// The swing every part's odd sixteenths take, 0..=1.
    swing: f64,
}

impl Builder {
    pub fn new(bpm: f64) -> Self {
        let mut song = Song {
            tempo_bpm: bpm,
            ..Song::default()
        };
        song.play_loop = Some((0, BARS * 4 * PPQ));
        Self { song, swing: 0.0 }
    }

    /// Swing for the parts added after this, as the drum editor's `SWING` (0..=1).
    pub fn swing(mut self, amount: f64) -> Self {
        self.swing = amount;
        self
    }

    /// The tick of sixteenth `step`, swung.
    fn tick(&self, step: u32) -> u32 {
        let mut tick = [i64::from(step * STEP)];
        apply_drum_swing(&mut tick, 4, self.swing, false).expect("a swing in 0..=1");
        tick[0] as u32
    }

    fn last(&mut self) -> &mut crate::song::Channel {
        self.song.channels.last_mut().expect("the master at least")
    }

    /// Adds a channel of `kind`, with a clip of `len` sixteenths at tick 0 showing [`BARS`] bars, holding `events`.
    fn part(&mut self, kind: ChannelKind, len: usize, events: Vec<Event>) {
        self.song.add_channel(kind);
        let clip = self.last().lanes[0].add_clip(0, len as f64 / 4.0);
        clip.visible_len_beats = f64::from(BARS * 4);
        clip.events = events;
        clip.events.sort_by_key(Event::pos);
    }

    /// A Drums channel on our kit `kit_key` (`Built-in/<Key>/<Pad>`), playing a row per pad: `x` a hit, `X` an accent,
    /// anything else a rest; 16 or 32 sixteenths, all rows alike.
    pub fn drums(mut self, kit_key: &str, rows: &[(&str, &str)]) -> Self {
        let kit = kits::kit(kit_key).expect("a kit of ours");
        let len = rows.first().map_or(16, |(_, row)| row.len());
        assert!(rows.iter().all(|(_, row)| row.len() == len), "rows alike");
        let mut hits = Vec::new();
        for (pad, row) in rows {
            let index = (kit.slots.iter())
                .position(|s| s.pad_name() == *pad)
                .expect("a pad of the kit");
            for (step, c) in row.bytes().enumerate() {
                let velocity = match c {
                    b'X' => 255,
                    b'x' => 170,
                    _ => continue,
                };
                hits.push(Event::Drum {
                    pos: self.tick(step as u32),
                    len_beats: 0.0,
                    pad: index as u8,
                    velocity,
                    pan: Event::DEFAULT_PAN,
                    value: Event::DEFAULT_PITCH,
                    repeat: 0,
                });
            }
        }
        self.part(ChannelKind::Drums, len, hits);
        let swing = self.swing;
        let channel = self.last();
        channel.lanes[0].swing = swing;
        channel.drum_kit = Some(kit);
        self
    }

    /// `kind` channel on our preset `preset`, loaded and naming the channel as a pick does, playing `notes` of
    /// (sixteenth, key, length in sixteenths, velocity /255) in a clip of `len` sixteenths.
    pub fn synth(
        mut self,
        kind: SynthKind,
        preset: &str,
        len: usize,
        notes: &[(u32, u8, u32, u8)],
    ) -> Self {
        let events = (notes.iter())
            .map(|&(step, key, steps, velocity)| Event::Note {
                pos: self.tick(step),
                len_beats: f64::from(self.tick(step + steps) - self.tick(step)) / f64::from(PPQ),
                key,
                velocity,
                pan: Event::DEFAULT_PAN,
                slide: false,
            })
            .collect();
        self.part(ChannelKind::Synth(kind), len, events);
        let module = find(kind.type_id(), preset).expect("a preset of ours");
        let name = module.file.preset_name.clone();
        self.last().rack.modules[0].load_preset(module);
        let index = self.song.channels.len() - 1;
        name_song_channel(&mut self.song, index, name);
        self
    }

    /// Adds effect `type_id` to the last channel's rack, its params set by name (the registry's).
    pub fn effect(mut self, type_id: u16, params: &[(&str, f32)]) -> Self {
        let rack = &mut self.last().rack;
        let id = rack.add(type_id);
        let spec = module_spec(type_id).expect("a module type");
        let module = rack.get_mut(id).expect("just added");
        for &(name, value) in params {
            let index = spec.params.iter().position(|p| p.name == name);
            module.params[index.expect("a param of the module")] = value;
        }
        self
    }

    /// The last part's clip at each of `at`, (first bar, bars shown), in place of its one at tick 0: an arrangement
    /// (the demo song).
    pub fn at(mut self, at: &[(u32, u32)]) -> Self {
        let lane = &mut self.last().lanes[0];
        let clip = lane.clips.pop().expect("the part's clip");
        lane.clips = (at.iter().zip(1..))
            .map(|(&(bar, bars), id)| Clip {
                id,
                start_tick: bar * 4 * PPQ,
                visible_len_beats: f64::from(bars * 4),
                ..clip.clone()
            })
            .collect();
        lane.last_clip_id = at.len() as u32;
        self
    }

    /// The last channel's Level, in dB.
    pub fn level(mut self, db: f32) -> Self {
        self.last().params.0[0] = level(db);
        self
    }

    /// The last channel also sends to the song's `Reverb` send channel at `db` dB: an Effect Track holding Reverb 2,
    /// wet only, made the first time.
    pub fn reverb_send(mut self, db: f32) -> Self {
        let from = self.song.channels.len() - 1;
        let reverb = match self.song.channels.iter().position(|c| c.name == "Reverb") {
            Some(at) => at,
            None => {
                self.song.add_channel(ChannelKind::EffectTrack);
                self.last().name = "Reverb".into();
                self = self.effect(
                    REVERB,
                    &[("SIZE", 0.6), ("DECAY", 0.1), ("DRY", 0.0), ("WET", 0.6)],
                );
                self.song.channels.len() - 1
            }
        };
        let id = self.song.channels[reverb].id;
        let buses = self.song.send_buses();
        let channel = &mut self.song.channels[from];
        channel.file.sends.push(SendState {
            target_channel_id: id,
            level: level(db),
            ..SendState::default()
        });
        channel.output_bus = output_bus(&channel.file.sends, &buses);
        self
    }

    /// The song, with zero GUIDs (a template is the same bytes every time).
    pub fn build(mut self) -> Song {
        self.song.file.guid = [0; 16];
        self.song
            .channels
            .iter_mut()
            .for_each(|c| c.file.guid = [0; 16]);
        self.song
    }
}

/// A template: its name and what makes the song.
type Template = (&'static str, fn() -> Song);

/// The factory templates, including the additional set in [`more`].
fn templates() -> Vec<Template> {
    let mut all: Vec<Template> = vec![
        ("Audio Recording", audio_recording),
        ("Boom Bap", boom_bap),
        ("D&B", drum_and_bass),
        ("Lo-Fi", lo_fi),
        ("Techno", techno),
    ];
    all.extend(more::templates());
    all
}

pub(super) fn entries() -> Vec<Entry> {
    let one = |(name, song): Template| {
        Entry::new(format!("{TEMPLATES}/{name}.songtmpl"), move || {
            flm::write(&song()).expect("our template writes")
        })
    };
    templates().into_iter().map(one).collect()
}

/// The master and one Audio Record channel, nothing to play: an empty audio track setup.
fn audio_recording() -> Song {
    let mut b = Builder::new(120.0);
    b.song.play_loop = None;
    b.song.add_channel(ChannelKind::AudioRecord);
    b.build()
}

/// A dusty, swung 90 BPM groove in A minor: Am9 to Dm9 on felt keys over a round sub.
fn boom_bap() -> Song {
    Builder::new(90.0)
        .swing(0.4)
        .drums(
            "Dust",
            &[
                ("Kick", "X......x..x.....X......x..x...x."),
                ("Snare", "....X.......X.......X.......X..."),
                ("Rim", "...............x.......x........"),
                ("Closed Hat", "X.x.X.x.X.x.X.x.X.x.X.x.X.x.X.x."),
                ("Open Hat", "..............................x."),
                ("Crackle", "X...............X..............."),
            ],
        )
        .level(-1.0)
        .synth(
            SynthKind::MiniSynth,
            "Velvet Sub",
            32,
            &[
                (0, 33, 6, 220),
                (7, 33, 2, 170),
                (10, 40, 4, 200),
                (16, 38, 6, 220),
                (23, 38, 2, 170),
                (26, 45, 2, 200),
                (30, 36, 2, 170),
            ],
        )
        .level(-4.0)
        .synth(
            SynthKind::MiniSynth,
            "Felt Keys",
            32,
            &chords(&[
                (0, 7, &[55, 59, 60, 64]),
                (10, 4, &[55, 59, 60, 64]),
                (16, 7, &[53, 57, 60, 64]),
                (26, 4, &[53, 57, 60, 64]),
            ]),
        )
        .level(-13.0)
        .reverb_send(-8.0)
        .build()
}

/// A 174 BPM two-step in F minor: a squelchy line under a wide Fm7 to Dbmaj7 pad.
fn drum_and_bass() -> Song {
    Builder::new(174.0)
        .drums(
            "Foundry",
            &[
                ("Kick", "X.........X.....X.x.......X....."),
                ("Snare", "....X.......X.......X.......X..."),
                ("Rim", ".......x..x............x..x...x."),
                ("Closed Hat", "x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x."),
                ("Shaker", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"),
                ("Crash", "X..............................."),
            ],
        )
        .level(-2.0)
        .synth(
            SynthKind::TransistorBass,
            "Rubber Square",
            32,
            &[
                (0, 41, 6, 220),
                (10, 41, 2, 170),
                (12, 44, 4, 200),
                (16, 37, 6, 220),
                (26, 37, 2, 170),
                (28, 39, 4, 200),
            ],
        )
        .level(1.0)
        .synth(
            SynthKind::SuperSaw,
            "Warm Blanket",
            32,
            &chords(&[(0, 16, &[60, 63, 65, 68]), (16, 16, &[60, 61, 65, 68])]),
        )
        .level(-14.0)
        .reverb_send(-6.0)
        .build()
}

/// A slow, lazy 80 BPM loop: a soft sub under Fmaj9, Em7 and Am9 on chorused keys, crackle under it all.
fn lo_fi() -> Song {
    Builder::new(80.0)
        .swing(0.5)
        .drums(
            "Dust",
            &[
                ("Kick", "X.......x.x.....X.......x.....x."),
                ("Snare", "....X.......X.......X.......X..."),
                ("Closed Hat", "x.x.x.xxx.x.x.xxx.x.x.xxx.x.x.x."),
                ("Shaker", "......x.......x.......x.......x."),
                ("Crackle", "X...............X..............."),
            ],
        )
        .level(-1.0)
        .synth(
            SynthKind::MiniSynth,
            "Velvet Sub",
            32,
            &[
                (0, 41, 7, 220),
                (10, 36, 4, 180),
                (16, 40, 7, 220),
                (24, 33, 6, 200),
            ],
        )
        .level(-5.0)
        .synth(
            SynthKind::MiniSynth,
            "Felt Keys",
            32,
            &chords(&[
                (0, 14, &[57, 60, 64, 67]),
                (16, 7, &[55, 59, 62, 64]),
                (24, 7, &[55, 59, 60, 64]),
            ]),
        )
        .effect(CHORUS, &[("Rate", 0.3), ("Depth", 0.5), ("Mix", 0.5)])
        .level(-11.0)
        .reverb_send(-6.0)
        .build()
}

/// A 128 BPM four-to-the-floor in A minor: an acid line and offbeat chord stabs.
fn techno() -> Song {
    Builder::new(128.0)
        .drums(
            "Foundry",
            &[
                ("Kick", "X...X...X...X...X...X...X...X..."),
                ("Clap", "....x.......x.......x.......x..."),
                ("Open Hat", "..x...x...x...x...x...x...x...x."),
                ("Shaker", "x.xxx.xxx.xxx.xxx.xxx.xxx.xxx.xx"),
                ("Metal", "..............................x."),
            ],
        )
        .level(-2.0)
        .synth(
            SynthKind::TransistorBass,
            "Acid Drip",
            16,
            &[
                (0, 33, 1, 230),
                (2, 45, 1, 170),
                (3, 33, 1, 170),
                (5, 43, 1, 230),
                (6, 33, 1, 170),
                (8, 45, 1, 170),
                (10, 33, 1, 230),
                (11, 36, 1, 170),
                (12, 33, 1, 170),
                (14, 45, 1, 230),
                (15, 40, 1, 170),
            ],
        )
        .level(2.0)
        .synth(
            SynthKind::SuperSaw,
            "Warehouse Stab",
            32,
            &chords(&[
                (3, 2, &[57, 60, 64, 67]),
                (6, 2, &[57, 60, 64, 67]),
                (11, 2, &[57, 60, 64, 67]),
                (19, 2, &[57, 60, 64, 67]),
                (22, 2, &[57, 60, 64, 67]),
                (27, 2, &[55, 60, 62, 67]),
                (30, 2, &[55, 60, 62, 67]),
            ]),
        )
        .level(-10.0)
        .build()
}

/// The notes of `chords`, each (sixteenth, length in sixteenths, keys), at velocity 190.
pub(super) fn chords(chords: &[(u32, u32, &[u8])]) -> Vec<(u32, u8, u32, u8)> {
    let notes = chords
        .iter()
        .flat_map(|&(step, len, keys)| keys.iter().map(move |&k| (step, k, len, 190)));
    notes.collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::content::{ROOT, factory};
    use crate::project_files::mem::MemFiles;
    use crate::project_files::{ProjectFiles, Replace};
    use crate::ui::views::Overlay;

    fn at(name: &str) -> PathBuf {
        Path::new(ROOT)
            .join(TEMPLATES)
            .join(format!("{name}.songtmpl"))
    }

    /// SONGS `New`'s route (`Replace::Template`) on our real factory, each of ours through the `.songtmpl` load:
    /// an untitled `New Song`, never saved, with the template's channels and tempo and GUIDs of its own.
    #[test]
    fn each_template_opens_through_replace_template_as_an_untitled_new_song() {
        for (name, make) in templates() {
            let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
            files.factory = factory();
            let song = files.replace(Replace::Template(at(name)));
            let song = song.unwrap_or_else(|| panic!("{name}: {:?}", files.take_overlay()));
            let built = make();
            assert_eq!(song.title, "New Song", "{name}");
            assert!(!files.modified(), "{name}: nothing to save yet");
            // Untitled: the first save asks, with that name.
            files.save(&song);
            let asked = files.take_overlay();
            assert!(
                matches!(&asked, Some(Overlay::SaveDialog { name }) if name == "New Song"),
                "{name}: {asked:?}"
            );
            assert_eq!(song.tempo_bpm, built.tempo_bpm, "{name}");
            let names = |s: &Song| {
                s.channels
                    .iter()
                    .map(|c| (c.kind, c.name.clone()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(names(&song), names(&built), "{name}");
            // GUIDs of its own, and new ones on every open.
            let guids = |s: &Song| {
                let channels = s.channels.iter().map(|c| c.file.guid);
                std::iter::once(s.file.guid)
                    .chain(channels)
                    .collect::<Vec<_>>()
            };
            assert!(guids(&song).iter().all(|g| *g != [0; 16]), "{name}");
            let again = files.replace(Replace::Template(at(name))).unwrap();
            let (first, second) = (guids(&song), guids(&again));
            assert_eq!(first.len(), second.len(), "{name}");
            let differ = first.iter().zip(&second).all(|(a, b)| a != b);
            assert!(differ, "{name}: {first:?} {second:?}");
        }
    }
}

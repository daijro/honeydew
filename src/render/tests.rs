use super::*;
use crate::song::{ChannelKind, Event, PPQ, SynthKind};

/// A low rate keeps the 30 s tails cheap.
const RATE: u32 = 8_000;

fn spec(range: RenderRange, remainder: Remainder) -> RenderSpec {
    RenderSpec {
        range,
        remainder,
        normalize: false,
        high_quality: true,
        rate: RATE,
        solo: None,
        master_effects: true,
    }
}

/// 120 BPM, a MiniSynth with one clip of `clip_beats` holding a one-beat note at `pos` ticks.
fn note_song(pos: u32, clip_beats: f64) -> Song {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.channels[1].lanes[0]
        .add_clip(0, clip_beats)
        .events
        .push(Event::Note {
            pos,
            len_beats: 1.0,
            key: 69,
            velocity: 204,
            pan: 127,
            slide: false,
        });
    song
}

/// `note_song` with a Tape Delay at full feedback and mix on the synth channel.
fn delay_song() -> Song {
    let mut song = note_song(0, 2.0);
    let id = song.channels[1].rack.add(18);
    let delay = song.channels[1].rack.get_mut(id).unwrap();
    delay.params[2] = 0.9;
    delay.params[4] = 1.0;
    song
}

/// Frames of `beats` at 120 BPM.
fn frames(beats: f64) -> usize {
    (beats * 60.0 * f64::from(RATE) / 120.0).ceil() as usize
}

/// The stream's frames, all of them, as an encoder takes them.
fn collect(mut stream: Stream) -> [Vec<f32>; 2] {
    let mut audio = [Vec::new(), Vec::new()];
    while !stream.done() {
        if let Some(blocks) = stream.step() {
            assert!(blocks[0].len() <= BLOCK && blocks[0].len() == blocks[1].len());
            audio.iter_mut().zip(blocks).for_each(|(a, b)| a.extend(b));
        }
    }
    assert_eq!(audio[0].len(), stream.frames());
    audio
}

/// The render, both passes.
fn render_with(
    song: &Song,
    samples: &SampleCache,
    spec: RenderSpec,
) -> Result<[Vec<f32>; 2], RenderError> {
    Ok(collect(Render::new(song, samples, spec)?.finish()?))
}

fn render(song: &Song, spec: RenderSpec) -> Result<[Vec<f32>; 2], RenderError> {
    render_with(song, &SampleCache::default(), spec)
}

fn energy(audio: &[Vec<f32>; 2]) -> f32 {
    audio.iter().flatten().map(|x| x * x).sum()
}

fn peak(audio: &[Vec<f32>; 2]) -> f32 {
    audio.iter().flatten().fold(0.0, |m, x| m.max(x.abs()))
}

#[test]
fn a_one_note_song_renders_sound() {
    let audio = render(&note_song(0, 2.0), spec(RenderRange::Full, Remainder::Cut)).unwrap();
    assert!(energy(&audio) > 0.01);
    assert_eq!(audio[0].len(), audio[1].len());
}

#[test]
fn cut_is_exactly_the_range() {
    let audio = render(&note_song(0, 2.0), spec(RenderRange::Full, Remainder::Cut)).unwrap();
    // A 2-beat clip makes a one-bar song.
    assert_eq!(audio[0].len(), frames(4.0));
}

#[test]
fn a_delay_tail_makes_leave_longer_and_wrap_adds_it_onto_the_start() {
    let song = delay_song();
    // The 2-beat clip makes a one-bar song.
    let range = frames(4.0);
    let cut = render(&song, spec(RenderRange::Full, Remainder::Cut)).unwrap();
    let leave = render(&song, spec(RenderRange::Full, Remainder::Leave)).unwrap();
    let wrap = render(&song, spec(RenderRange::Full, Remainder::Wrap)).unwrap();
    assert!(leave[0].len() > range, "{}", leave[0].len());
    assert!(leave[0].len() <= range + 30 * RATE as usize);
    // Leave ends on its last audible frame.
    let last = leave[0].len() - 1;
    assert!(leave[0][last].abs() > AUDIBLE || leave[1][last].abs() > AUDIBLE);
    assert_eq!(wrap[0].len(), range);
    // Wrap = Cut + the faded tail, sample by sample.
    let overlap = range.min(leave[0].len() - range);
    for ch in 0..2 {
        for i in 0..overlap {
            assert_eq!(
                wrap[ch][i],
                cut[ch][i] + leave[ch][range + i],
                "ch {ch} frame {i}"
            );
        }
    }
    let head = |a: &[Vec<f32>; 2]| a[0][..overlap].iter().map(|x| x * x).sum::<f32>();
    assert!(head(&wrap) != head(&cut));
}

#[test]
fn selection_renders_the_play_loop() {
    // The note starts with the loop: a seek does not chase notes already sounding.
    let mut song = note_song(64, 4.0);
    assert_eq!(range_ticks(&song, RenderRange::Selection), None);
    song.play_loop = Some((64, 192));
    assert_eq!(
        range_ticks(&song, RenderRange::Selection),
        Some((64.0, 192.0))
    );
    let audio = render(&song, spec(RenderRange::Selection, Remainder::Cut)).unwrap();
    assert_eq!(audio[0].len(), frames(1.5) - frames(0.5));
}

#[test]
fn the_song_play_loop_does_not_loop_a_full_render() {
    let mut song = note_song(2 * PPQ, 4.0);
    song.play_loop = Some((0, PPQ));
    let audio = render(&song, spec(RenderRange::Full, Remainder::Cut)).unwrap();
    assert_eq!(audio[0].len(), frames(4.0));
    assert!(energy(&audio) > 0.01);
}

#[test]
fn a_selection_with_leave_does_not_replay_its_loop_in_the_tail() {
    let mut song = note_song(0, 4.0);
    song.play_loop = Some((0, PPQ));
    let audio = render(&song, spec(RenderRange::Selection, Remainder::Leave)).unwrap();
    // The note releases at the range end and rings out within a second.
    assert!(
        audio[0].len() < frames(1.0) + RATE as usize,
        "{}",
        audio[0].len()
    );
}

#[test]
fn the_tail_plays_no_song_events_after_the_range_end() {
    let mut song = note_song(0, 8.0);
    song.channels[1].lanes[0].clips[0].events.push(Event::Note {
        pos: 4 * PPQ,
        len_beats: 1.0,
        key: 69,
        velocity: 204,
        pan: 127,
        slide: false,
    });
    song.play_loop = Some((0, PPQ));
    let audio = render(&song, spec(RenderRange::Selection, Remainder::Leave)).unwrap();
    assert!(audio[0].len() < frames(4.0), "{}", audio[0].len());
}

#[test]
fn a_selection_without_a_loop_renders_as_the_full_song() {
    // The note ends long before the clip: Leave may end before the range end, as with Full song.
    let song = note_song(0, 4.0);
    let full = render(&song, spec(RenderRange::Full, Remainder::Leave)).unwrap();
    let selection = render(&song, spec(RenderRange::Selection, Remainder::Leave)).unwrap();
    assert!(full[0].len() < frames(4.0), "{}", full[0].len());
    assert_eq!(selection, full);
}

#[test]
fn trim_initial_silence_drops_the_leading_zero_frames() {
    let song = note_song(PPQ, 2.0);
    let full = render(&song, spec(RenderRange::Full, Remainder::Leave)).unwrap();
    let trim = render(&song, spec(RenderRange::TrimSilence, Remainder::Leave)).unwrap();
    let zeros = full[0]
        .iter()
        .zip(&full[1])
        .take_while(|(l, r)| **l == 0.0 && **r == 0.0)
        .count();
    assert!(zeros >= frames(1.0), "{zeros}");
    assert_eq!(trim[0], full[0][zeros..]);
    assert_eq!(trim[1], full[1][zeros..]);
}

#[test]
fn normalize_brings_the_peak_to_one() {
    let mut normalized = spec(RenderRange::Full, Remainder::Cut);
    normalized.normalize = true;
    let audio = render(&note_song(0, 2.0), normalized).unwrap();
    assert_eq!(peak(&audio), 1.0);
}

#[test]
fn an_empty_song_has_no_sound() {
    for remainder in [Remainder::Leave, Remainder::Cut, Remainder::Wrap] {
        let result = render(&Song::default(), spec(RenderRange::Full, remainder));
        assert!(matches!(result, Err(RenderError::Silent)), "{remainder:?}");
    }
}

#[test]
fn each_pass_steps_one_block_at_a_time() {
    let song = delay_song();
    let leave = spec(RenderRange::Full, Remainder::Leave);
    let whole = render(&song, leave).unwrap();
    let mut render = Render::new(&song, &SampleCache::default(), leave).unwrap();
    let mut steps = 1;
    while !render.step(1) {
        steps += 1;
    }
    // The block at the range end is split there.
    let blocks = frames(4.0).div_ceil(BLOCK) + (30 * RATE as usize).div_ceil(BLOCK);
    assert_eq!(steps, blocks);
    // Pass 2 renders a block a step, up to the last audible frame, and hands out what it has in full blocks.
    let mut stream = render.finish().unwrap();
    let (mut steps, mut audio) = (0, [Vec::<f32>::new(), Vec::new()]);
    while !stream.done() {
        let handed = stream.step().map(|blocks| {
            audio.iter_mut().zip(blocks).for_each(|(a, b)| a.extend(b));
            blocks[0].len()
        });
        assert!(handed.is_none_or(|n| n == BLOCK || stream.done()));
        steps += 1;
    }
    assert_eq!(audio, whole);
    // A block a step, and one more step for what is left once the pass ends.
    let blocks = frames(2.0).div_ceil(BLOCK) + (whole[0].len() - frames(2.0)).div_ceil(BLOCK);
    assert!(
        steps == blocks || steps == blocks + 1,
        "{steps} for {blocks}"
    );
}

#[test]
fn solo_renders_that_channel_alone() {
    let mut song = note_song(0, 2.0);
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    // A Solo in the song does not survive a stem render of another channel.
    song.channels[1].params.0[3] = 1.0;
    let mut stem = spec(RenderRange::Full, Remainder::Cut);
    stem.solo = Some(2);
    assert!(matches!(render(&song, stem), Err(RenderError::Silent)));
    stem.solo = Some(1);
    assert!(energy(&render(&song, stem).unwrap()) > 0.01);
}

#[test]
fn the_estimated_length_is_the_range_at_the_song_tempo() {
    let mut song = note_song(0, 4.0);
    assert_eq!(estimated_secs(&song, RenderRange::Full), 2.0);
    assert_eq!(estimated_secs(&song, RenderRange::TrimSilence), 2.0);
    // No play loop: Selection falls back to the full song.
    assert_eq!(estimated_secs(&song, RenderRange::Selection), 2.0);
    song.play_loop = Some((0, PPQ));
    assert_eq!(estimated_secs(&song, RenderRange::Selection), 0.5);
}

#[test]
fn a_range_over_thirty_minutes_is_refused() {
    // 30 min at 120 BPM = 3600 beats; the clip ends one beat later.
    let song = note_song(3600 * PPQ, 3601.0);
    let result = Render::new(
        &song,
        &SampleCache::default(),
        spec(RenderRange::Full, Remainder::Cut),
    );
    assert!(matches!(result, Err(RenderError::TooLong)));
}

// ---- The processing pass ----

/// The oracle's processing pass, checked against the two passes' ([`streamed`]) on the same audio.
fn process(
    audio: [Vec<f32>; 2],
    range: usize,
    spec: &RenderSpec,
) -> Result<[Vec<f32>; 2], RenderError> {
    let want = super::process(audio.clone(), range, spec);
    let got = streamed(audio, range, spec);
    assert_eq!(format!("{got:?}"), format!("{want:?}"), "{spec:?}");
    want
}

fn buf(l: &[f32], r: &[f32]) -> [Vec<f32>; 2] {
    [l.to_vec(), r.to_vec()]
}

#[test]
fn the_tail_fades_out_linearly_over_its_length() {
    // Range 2, tail 4: gains 1, 3/4, 1/2, 1/4.
    let out = process(
        buf(&[0.5; 6], &[0.5; 6]),
        2,
        &spec(RenderRange::Full, Remainder::Leave),
    )
    .unwrap();
    assert_eq!(
        out,
        buf(
            &[0.5, 0.5, 0.5, 0.375, 0.25, 0.125],
            &[0.5, 0.5, 0.5, 0.375, 0.25, 0.125]
        )
    );
}

#[test]
fn leave_ends_after_the_last_frame_above_minus_60_dbfs() {
    let leave = spec(RenderRange::Full, Remainder::Leave);
    // Range 4, no tail: the frame at 0.001 is not audible, the one above on the right channel is.
    let out = process(
        buf(&[0.5, 0.0, 0.0, 0.0], &[0.0, 0.002, 0.001, 0.0]),
        4,
        &leave,
    )
    .unwrap();
    assert_eq!(out, buf(&[0.5, 0.0], &[0.0, 0.002]));
}

#[test]
fn a_selection_never_ends_before_its_range_end() {
    let audio = buf(&[0.5, 0.0, 0.0, 0.0, 0.0, 0.0], &[0.0; 6]);
    let out = process(
        audio.clone(),
        3,
        &spec(RenderRange::Selection, Remainder::Leave),
    )
    .unwrap();
    assert_eq!(out[0], [0.5, 0.0, 0.0]);
    let out = process(audio, 3, &spec(RenderRange::Full, Remainder::Leave)).unwrap();
    assert_eq!(out[0], [0.5]);
}

#[test]
fn wrap_adds_the_tail_onto_the_start_and_loses_what_is_past_the_range_length() {
    // Range 2, tail 4 faded to 1, 3/4, 1/2, 1/4: only the first two tail frames land.
    let out = process(
        buf(&[0.25, 0.25, 0.5, 0.5, 0.5, 0.5], &[0.0; 6]),
        2,
        &spec(RenderRange::Full, Remainder::Wrap),
    )
    .unwrap();
    assert_eq!(out, buf(&[0.75, 0.625], &[0.0, 0.0]));
}

#[test]
fn trim_drops_exact_zero_frames_and_only_with_leave() {
    let audio = buf(&[0.0, 0.0, 1e-9, 0.5], &[0.0, 0.0, 0.0, 0.5]);
    let trim = spec(RenderRange::TrimSilence, Remainder::Leave);
    let out = process(audio.clone(), 4, &trim).unwrap();
    assert_eq!(out, buf(&[1e-9, 0.5], &[0.0, 0.5]));
    // The first sample ≠ 0 rings in the tail (range 2 of 3 frames, the tail's first at gain 1).
    let tail = process(buf(&[0.0, 0.0, 0.5], &[0.0; 3]), 2, &trim).unwrap();
    assert_eq!(tail, buf(&[0.5], &[0.0]));
    // Through the 16-bit intermediate the tiny frame is 0 too.
    let low = RenderSpec {
        high_quality: false,
        ..trim
    };
    assert_eq!(process(audio.clone(), 4, &low).unwrap()[0].len(), 1);
    for remainder in [Remainder::Cut, Remainder::Wrap] {
        let out = process(audio.clone(), 4, &RenderSpec { remainder, ..trim }).unwrap();
        assert_eq!(out[0].len(), 4, "{remainder:?}");
    }
}

#[test]
fn normalize_divides_by_the_peak_of_the_render_including_the_tail() {
    let normalized = RenderSpec {
        normalize: true,
        ..spec(RenderRange::Full, Remainder::Wrap)
    };
    // The peak is the tail's 0.5, taken before the wrap adds it onto 0.25.
    let out = process(buf(&[0.25, 0.25, 0.5, 0.0], &[0.0; 4]), 2, &normalized).unwrap();
    assert_eq!(out, buf(&[1.5, 0.5], &[0.0, 0.0]));
    // A tail frame the wrap loses still counts: range 1, tail 0.25 (kept) then 0.8 · 1/2 (lost).
    let out = process(buf(&[0.1, 0.25, 0.8], &[0.0; 3]), 1, &normalized).unwrap();
    assert_eq!(out, buf(&[(0.1 + 0.25) / 0.4], &[0.0]));
}

#[test]
fn high_quality_off_quantizes_before_normalize() {
    let normalized = RenderSpec {
        normalize: true,
        high_quality: false,
        ..spec(RenderRange::Full, Remainder::Cut)
    };
    let out = process(buf(&[0.5, 0.3], &[0.0; 2]), 2, &normalized).unwrap();
    assert_eq!(out[0], [1.0, pcm16(0.3) / pcm16(0.5)]);
    assert_ne!(out[0][1], pcm16(0.3 / 0.5));
    assert_ne!(out[0][1], 0.3 / 0.5);
}

#[test]
fn silence_below_minus_60_dbfs_has_no_sound() {
    for remainder in [Remainder::Leave, Remainder::Cut, Remainder::Wrap] {
        let result = process(
            buf(&[0.001, -0.001], &[0.0, 0.0005]),
            2,
            &spec(RenderRange::Full, remainder),
        );
        assert!(matches!(result, Err(RenderError::Silent)), "{remainder:?}");
    }
}

#[test]
fn the_16_bit_intermediate_is_what_the_wav_writer_writes_back() {
    let wav = |x: f32| (f64::from(x) * 32767.0).clamp(-32767.0, 32767.0) as i16;
    for q in -32767..=32767_i16 {
        // An input that is surely q: half a step past it.
        let x = ((f64::from(q) + 0.5 * f64::from(q.signum())) / 32767.0) as f32;
        assert_eq!(wav(x), q);
        assert_eq!(wav(pcm16(x)), q, "{q}");
        assert_eq!(pcm16(pcm16(x)), pcm16(x));
    }
    assert_eq!(pcm16(2.0), pcm16(1.0));
    assert_eq!(pcm16(f32::NAN), 0.0);
}

#[test]
fn the_tail_keeps_the_params_where_the_range_left_them() {
    // The export's stop at the range end is no transport Stop, which puts the params back before their automation. A
    // channel automated to Level 0 throughout sends its delay's echoes nowhere in the tail either.
    let mut song = delay_song();
    let lane = song.channels[1].add_automation_lane(crate::song::AutomationTarget {
        instance_id: 0,
        param: 0,
    });
    lane.add_clip(0, 2.0).events.push(Event::Automation {
        pos: 0,
        value: 0.0,
        control: true,
    });
    let result = render(&song, spec(RenderRange::Full, Remainder::Leave));
    assert!(
        matches!(result, Err(RenderError::Silent)),
        "{:?}",
        result.map(|a| a[0].len())
    );
}

#[test]
fn a_master_gain_changes_a_stem_only_with_master_effects_on() {
    // A Leveller on the master at a low Gain: the stem of the synth channel goes through it only when on.
    let plain = note_song(0, 2.0);
    let mut song = plain.clone();
    let id = song.channels[0].rack.add(20);
    song.channels[0].rack.get_mut(id).unwrap().params[1] = 0.3;
    let mut stem = spec(RenderRange::Full, Remainder::Cut);
    stem.solo = Some(1);
    let through = render(&song, stem).unwrap();
    let bare = render(&plain, stem).unwrap();
    assert!(
        peak(&through) < 0.9 * peak(&bare),
        "{} vs {}",
        peak(&through),
        peak(&bare)
    );
    stem.master_effects = false;
    assert_eq!(render(&song, stem).unwrap(), bare);
}

// ---- Determinism: the two passes render the same audio ----

/// Every sample's bits, so NaN and −0 compare too.
fn bits(audio: &[Vec<f32>; 2]) -> Vec<u32> {
    audio.iter().flatten().map(|x| x.to_bits()).collect()
}

/// The engine's stereo output for `song` (the range and its tail, faded), as a pass renders it.
fn raw(song: &Song, samples: &SampleCache, spec: RenderSpec) -> [Vec<f32>; 2] {
    let mut pass = Render::new(song, samples, spec).unwrap().pass;
    let mut audio = [Vec::new(), Vec::new()];
    while let Some((_, block)) = pass.next() {
        audio[0].extend(block.iter().step_by(2));
        audio[1].extend(block.iter().skip(1).step_by(2));
    }
    audio
}

/// The factory templates, read as the app reads a `.songtmpl`, each with the samples it plays (ours, made in code).
fn templates() -> Vec<(String, Song, SampleCache)> {
    use crate::ui::main_menu::SongLibrary;
    let factory = crate::content::factory();
    let folder = std::path::Path::new(crate::content::ROOT).join(crate::content::TEMPLATES);
    let entries = factory.list(Some(&folder)).unwrap();
    assert!(entries.len() >= 5, "{}", entries.len());
    (entries.into_iter())
        .map(|entry| {
            let song = crate::flm::read(&factory.read(&entry.path).unwrap()).unwrap();
            let mut samples = SampleCache::default();
            samples.load(&song, &|_| None);
            (entry.name, song, samples)
        })
        .collect()
}

#[test]
fn the_factory_templates_render_twice_to_the_same_bits() {
    for (name, song, samples) in templates() {
        let leave = spec(RenderRange::Full, Remainder::Leave);
        let first = raw(&song, &samples, leave);
        assert_eq!(bits(&first), bits(&raw(&song, &samples, leave)), "{name}");
        if name == "Audio Recording.songtmpl" {
            continue;
        }
        assert!(energy(&first) > 0.01, "{name}");
        // Both passes and the single pass agree.
        let cut = RenderSpec {
            normalize: true,
            ..spec(RenderRange::Full, Remainder::Cut)
        };
        let oracle = single_pass(&song, &samples, cut).unwrap();
        assert_eq!(
            bits(&render_with(&song, &samples, cut).unwrap()),
            bits(&oracle),
            "{name}"
        );
    }
}

/// A user file the every-module song plays, by its path below `@L@`.
const LOOP: &str = "My Samples/Loop.wav";
const SF2: &str = "My SoundFonts/sine.sf2";

/// Every instrument on a channel of its own (the DW Sampler, Slicer and SoundFont Player with files to play), each
/// playing a run of notes under every note effect; every effect spread over the racks, a send to a Reverb channel; a
/// drum kit's built-in one-shots. With the samples it plays.
fn every_module_song() -> (Song, SampleCache) {
    use crate::flm::chunk::{ContentPath, ContentRoot};
    use crate::params::{Role, modules};
    let user = |relative: &str| ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: relative.into(),
    };
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    for kind in SynthKind::ALL {
        song.add_channel(ChannelKind::Synth(kind));
        let channel = song.channels.len() - 1;
        let module = &mut song.channels[channel].rack.modules[0];
        module.file.custom_data = match kind {
            SynthKind::DwSampler => {
                // One `ZONE` looping the user sample from key 48.
                let mut zone = b"@L@\0".to_vec();
                zone.extend(LOOP.as_bytes());
                zone.resize(1024, 0);
                zone.extend([48, 0, 127, 0, 127]);
                for word in [1.0f32.to_bits(), 0.5f32.to_bits(), 2, 100, 30_000] {
                    zone.extend(word.to_le_bytes());
                }
                for value in [0.0f32, 1.0, 1.0, 0.1] {
                    zone.extend(value.to_le_bytes());
                }
                let mut blob = b"10WDZONE".to_vec();
                blob.extend((zone.len() as u32).to_le_bytes());
                blob.extend(zone);
                Some(blob)
            }
            SynthKind::Slicer => {
                let path = [&b"@L@\0"[..], LOOP.as_bytes()].concat();
                let mut blob = b"PATH".to_vec();
                blob.extend((4 + path.len() as u32).to_le_bytes());
                blob.extend((path.len() as u32).to_le_bytes());
                blob.extend(path);
                Some(blob)
            }
            SynthKind::SoundFontPlayer => {
                let sf2d = crate::flm::sf2d::Sf2d {
                    path: user(SF2),
                    patch: 0,
                };
                Some(sf2d.write(None).unwrap())
            }
            _ => None,
        };
        // Note effects sit above the instrument they act on.
        for spec in modules().filter(|m| m.role == Role::NoteEffect) {
            song.channels[channel].rack.insert(0, spec.id);
        }
        let clip = song.channels[channel].lanes[0].add_clip(0, 4.0);
        for (i, key) in [48, 52, 55, 60, 64, 49, 50].into_iter().enumerate() {
            clip.events.push(Event::Note {
                pos: i as u32 * PPQ / 2,
                len_beats: 0.75,
                key,
                velocity: 204,
                pan: 127,
                slide: false,
            });
        }
    }
    let synths = SynthKind::ALL.len();
    let effects = modules().filter(|m| m.role == Role::Effect);
    for (i, spec) in effects.enumerate() {
        song.channels[1 + i % synths].rack.add(spec.id);
    }
    song.add_channel(ChannelKind::Drums);
    let drums = song.channels.len() - 1;
    let clip = song.channels[drums].lanes[0].add_clip(0, 4.0);
    for pad in 0..8 {
        clip.events.push(Event::Drum {
            pos: u32::from(pad) * PPQ / 2,
            len_beats: 0.0,
            pad,
            velocity: 255,
            pan: 127,
            value: Event::DEFAULT_PITCH,
            repeat: 0,
        });
    }
    // A one-second loop with a decaying burst on each quarter: slices for the Slicer, a zone for the DW Sampler.
    let burst = (0..44_100).map(|i| {
        let t = (i % 11_025) as f32 / 44_100.0;
        0.8 * (std::f32::consts::TAU * 440.0 * t).sin() * (-t / 0.02).exp()
    });
    let wav = honeydew_audio::wav_writer::float32_wav(44_100, &[burst.collect::<Vec<_>>()]);
    let sf2 = include_bytes!("../../crates/audio/tests/fixtures/sine.sf2");
    let mut samples = SampleCache::default();
    samples.load(&song, &|path| match path {
        ContentPath::Shorthand { relative, .. } if relative == LOOP => Some(wav.clone()),
        ContentPath::Shorthand { relative, .. } if relative == SF2 => Some(sf2.to_vec()),
        _ => None,
    });
    assert_eq!(samples.missing().count(), 0);
    (song, samples)
}

#[test]
fn every_instrument_effect_and_note_effect_renders_twice_to_the_same_bits() {
    let (song, samples) = every_module_song();
    // The notes ring out within the range; the templates' renders cover the tail.
    let cut = spec(RenderRange::Full, Remainder::Cut);
    let first = raw(&song, &samples, cut);
    assert_eq!(bits(&first), bits(&raw(&song, &samples, cut)));
    // Each instrument sounds, so none is left out of the comparison.
    for channel in 1..=SynthKind::ALL.len() {
        let stem = RenderSpec {
            solo: Some(channel),
            ..cut
        };
        let audio = raw(&song, &samples, stem);
        assert!(energy(&audio) > 1e-6, "{:?}", song.channels[channel].kind);
    }
}

// ---- The two passes against today's single pass ----

/// Every range, remainder, Normalize and High quality: the stream is the single pass, bit for bit, or fails as it does
/// (`Silent` exactly when the song has no `sound`).
fn matches_the_single_pass(song: &Song, rate: u32, sound: bool) {
    let ranges = [
        RenderRange::Full,
        RenderRange::TrimSilence,
        RenderRange::Selection,
    ];
    for range in ranges {
        for remainder in [Remainder::Leave, Remainder::Cut, Remainder::Wrap] {
            for (normalize, high_quality) in
                [(false, false), (false, true), (true, false), (true, true)]
            {
                let spec = RenderSpec {
                    normalize,
                    high_quality,
                    rate,
                    ..spec(range, remainder)
                };
                let samples = SampleCache::default();
                let want = single_pass(song, &samples, spec).map(|a| bits(&a));
                let got = render_with(song, &samples, spec).map(|a| bits(&a));
                assert_eq!(format!("{got:?}"), format!("{want:?}"), "{spec:?}");
                assert_eq!(want.is_ok(), sound, "{spec:?}");
            }
        }
    }
}

/// At this rate the 30 s tails are cheap; the notes are below its Nyquist.
const LOW_RATE: u32 = 2_000;

#[test]
fn the_stream_is_the_single_pass_for_a_late_note_with_echoes() {
    // A beat of silence first (trim), echoes ringing into the tail (leave, wrap) and a loop over the note.
    let mut song = delay_song();
    song.channels[1].lanes[0].clips[0].events[0] = Event::Note {
        pos: PPQ,
        len_beats: 0.5,
        key: 57,
        velocity: 204,
        pan: 60,
        slide: false,
    };
    song.play_loop = Some((PPQ / 2, 3 * PPQ / 2));
    matches_the_single_pass(&song, LOW_RATE, true);
}

#[test]
fn the_stream_is_the_single_pass_for_a_short_note_in_a_long_clip_and_for_silence() {
    // The note rings out long before the range end: leave ends early, a selection at its range end.
    let mut song = note_song(PPQ / 2, 4.0);
    song.play_loop = Some((0, 3 * PPQ));
    matches_the_single_pass(&song, LOW_RATE, true);
    matches_the_single_pass(&Song::default(), LOW_RATE, false);
}

#[test]
fn the_stream_is_the_single_pass_when_the_range_outlasts_the_tail() {
    // 32 s with echoes from its start: a wrap adds the 30 s tail onto part of the range only.
    let mut song = note_song(0, 64.0);
    let id = song.channels[1].rack.add(18);
    let delay = song.channels[1].rack.get_mut(id).unwrap();
    delay.params[2] = 0.9;
    delay.params[4] = 1.0;
    for remainder in [Remainder::Wrap, Remainder::Leave] {
        for (normalize, high_quality) in [(false, true), (true, false)] {
            let spec = RenderSpec {
                normalize,
                high_quality,
                rate: LOW_RATE,
                ..spec(RenderRange::Full, remainder)
            };
            let samples = SampleCache::default();
            let want = bits(&single_pass(&song, &samples, spec).unwrap());
            assert_eq!(
                bits(&render_with(&song, &samples, spec).unwrap()),
                want,
                "{spec:?}"
            );
        }
    }
}

/// Render range `Full song` is the song length estimate: the latest clip end rounded up to whole bars, at least one
/// bar.
#[test]
fn full_song_is_the_last_clip_end_rounded_up_to_whole_bars() {
    let bars = |song: &Song| range_ticks(song, RenderRange::Full).unwrap().1 / f64::from(PPQ);
    // A clip ending at 4.5 beats in 4/4: two bars.
    assert_eq!(bars(&note_song(0, 4.5)), 8.0);
    // Exactly one bar stays one bar; an empty song is one bar.
    assert_eq!(bars(&note_song(0, 4.0)), 4.0);
    assert_eq!(bars(&Song::default()), 4.0);
    // In 3/4 a bar is three beats.
    let mut song = note_song(0, 4.0);
    song.time_sig.beats_per_bar = 3;
    assert_eq!(bars(&song), 6.0);
    assert_eq!(
        range_ticks(&song, RenderRange::TrimSilence),
        Some((0.0, 6.0 * f64::from(PPQ)))
    );
}

/// Playback wraps at the song end without a loop; an export's engine plays on past it into the tail.
#[test]
fn an_export_does_not_wrap_at_the_song_end() {
    let mut engine = engine(&note_song(0, 4.0), &SampleCache::default(), RATE, 0.0).unwrap();
    engine.render(&mut vec![0.0; frames(6.0) * 2], 2);
    assert_eq!(engine.transport_tick(), 6 * u64::from(PPQ));
}

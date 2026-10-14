//! Sliced loops: one-shot renders sequenced on a 16th-note grid, with the Slicer's tempo and a cue on every step that
//! has a hit. The rules follow the code-made loops (`honeydew_audio::builtin_loops`): one hit at a time per voice, a
//! hit stopped by its voice's next one or by the loop's end fades over its last 5 ms, and no tail wraps round.
//!
//! A variant has `bpm`, `bars` and `voice` rows: `{ render, steps, level, pan, <model parameters> }`, where `steps`
//! has one character per step (spaces ignored): `.` rests, `x` hits and `X` accents.

use std::collections::BTreeSet;

use honeydew_audio::sample::SampleMeta;

use super::{RATE, normalize, one_shot, seed};

/// A plain hit (`x`) as a fraction of an accent (`X`).
const PLAIN: f32 = 0.6;
/// A stopped hit's fade.
const FADE: usize = RATE as usize / 200;

pub fn render(key: &str, v: &toml::Table) -> Result<(Vec<Vec<f32>>, SampleMeta), String> {
    let int = |k: &str| {
        v.get(k)
            .and_then(toml::Value::as_integer)
            .filter(|&n| n > 0)
            .map(|n| n as usize)
            .ok_or_else(|| format!("`{k}` must be a positive integer"))
    };
    let (bpm, bars) = (int("bpm")?, int("bars")?);
    // Exactly `bars · 4 · 60 / bpm` seconds, so the Slicer's tempo spans the loop to the frame.
    let total = bars * 240 * RATE as usize;
    if !total.is_multiple_of(bpm) {
        return Err(format!(
            "{bars} bars at {bpm} BPM is not a whole number of frames"
        ));
    }
    let (frames, steps) = (total / bpm, 16 * bars);
    let step = |s: usize| (s * frames + steps / 2) / steps;
    let voices = (v.get("voice").and_then(toml::Value::as_array)).ok_or("no `voice` rows")?;
    let mut out = vec![vec![0.0_f32; frames]; 2];
    let mut cues = BTreeSet::new();
    for (i, voice) in voices.iter().enumerate() {
        let mut params = voice.as_table().ok_or("a voice is not a table")?.clone();
        let mut take = |k: &str| params.remove(k);
        let model = take("render")
            .and_then(|m| m.as_str().map(str::to_owned))
            .ok_or("a voice has no `render`")?;
        let cells: Vec<char> = (take("steps").and_then(|s| s.as_str().map(str::to_owned)))
            .ok_or("a voice has no `steps`")?
            .chars()
            .filter(|c| *c != ' ')
            .collect();
        let num = |x: Option<toml::Value>, default: f32| {
            x.and_then(|x| x.as_float().or(x.as_integer().map(|i| i as f64)))
                .map_or(default, |x| x as f32)
        };
        let (level, pan) = (num(take("level"), 1.0), num(take("pan"), 0.0));
        if cells.len() != steps {
            return Err(format!("{model}: {} steps, not {steps}", cells.len()));
        }
        let one = one_shot(&model, &params, seed(&format!("{key}/{i}")))?;
        // A stereo one-shot plays as its mid.
        let one: Vec<f32> = (0..one[0].len())
            .map(|j| one.iter().map(|c| c[j]).sum::<f32>() / one.len() as f32)
            .collect();
        let mut hits = Vec::new();
        for (s, c) in cells.into_iter().enumerate() {
            match c {
                '.' => {}
                'x' => hits.push((step(s), PLAIN)),
                'X' => hits.push((step(s), 1.0)),
                _ => return Err(format!("{model}: step `{c}` is not `.`, `x` or `X`")),
            }
        }
        let gains = [(1.0 - pan).min(1.0) * level, (1.0 + pan).min(1.0) * level];
        for (k, &(start, accent)) in hits.iter().enumerate() {
            let ring = start + one.len();
            let end = hits
                .get(k + 1)
                .map_or(frames, |h| h.0)
                .min(frames)
                .min(ring);
            for (j, x) in one[..end - start].iter().enumerate() {
                let fade = if end < ring {
                    ((end - 1 - start - j) as f32 / (FADE - 1) as f32).min(1.0)
                } else {
                    1.0
                };
                for (o, g) in out.iter_mut().zip(gains) {
                    o[start + j] += x * accent * g * fade;
                }
            }
            cues.insert(start as u32);
        }
    }
    normalize(&mut out)?;
    let meta = SampleMeta {
        tempo: Some(bpm as f32),
        cues: cues.into_iter().collect(),
        ..SampleMeta::default()
    };
    Ok((out, meta))
}

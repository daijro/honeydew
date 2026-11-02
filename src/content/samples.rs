//! Our Slicer loops, `Loops/<Name>.wav`, and 3x Osc shapes, `Shapes/<Name>.wav`: each read is a 32-bit
//! float WAV of the crate's render (`builtin_loops`, `builtin_shapes`), made then.
//!
//! The app never plays these bytes. A pick stores `@R@Built-in/<rel>` ([`super::content_path`]), which the
//! sample cache renders from code ([`crate::sample_cache::SampleKey::of_path`]) with the loop's tempo and cues, which
//! the WAV does not carry.

use honeydew_audio::sample::SampleData;
use honeydew_audio::wav_writer::float32_wav;
use honeydew_audio::{builtin_loops, builtin_shapes};

use super::{Entry, LOOPS, SHAPES};

pub(super) fn entries() -> Vec<Entry> {
    type Render = fn(&str) -> Option<SampleData>;
    let loops = (builtin_loops::LOOPS.iter())
        .map(|l| (LOOPS, l.name, builtin_loops::loop_sample as Render));
    let shapes = (builtin_shapes::SHAPES.iter())
        .map(|&name| (SHAPES, name, builtin_shapes::shape as Render));
    (loops.chain(shapes))
        .map(|(dir, name, render)| {
            Entry::new(format!("{dir}/{name}.wav"), move || wav(render(name)))
        })
        .collect()
}

/// `sample` as a float WAV; nothing for none.
fn wav(sample: Option<SampleData>) -> Vec<u8> {
    sample.map_or_else(Vec::new, |s| {
        let channels: Vec<&[f32]> = std::iter::once(&s.left)
            .chain(&s.right)
            .map(|c| &c[..])
            .collect();
        float32_wav(s.sample_rate, &channels)
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use honeydew_audio::builtin_loops::loop_sample;
    use honeydew_audio::builtin_shapes::shape;
    use honeydew_audio::sample::decode;

    use crate::content::{LOOPS, ROOT, SHAPES, factory};
    use crate::ui::main_menu::SongLibrary;

    /// The names `folder` lists, in index order.
    fn names(folder: &str) -> Vec<String> {
        let listed = factory().list(Some(&Path::new(ROOT).join(folder)));
        listed.unwrap().into_iter().map(|e| e.name).collect()
    }

    #[test]
    fn the_loops_and_shapes_list_in_index_order_and_read_as_wavs_of_their_renders() {
        assert_eq!(
            names(LOOPS),
            [
                "Dust Break 84.wav",
                "Foundry Drive 128.wav",
                "Foundry Rush 172.wav",
                "Pocket Groove 92.wav",
                "Relay Bounce 100.wav",
                "Relay Roll 140.wav",
            ]
        );
        assert_eq!(
            names(SHAPES),
            [
                "Glass.wav",
                "Grit.wav",
                "Reed.wav",
                "Thin Pulse.wav",
                "Vowel A.wav",
                "Vowel O.wav",
            ]
        );
        let read =
            |rel: &str| decode(&factory().read(&Path::new(ROOT).join(rel)).unwrap()).unwrap();
        let (wav, render) = (
            read("Loops/Dust Break 84.wav"),
            loop_sample("Dust Break 84").unwrap(),
        );
        assert_eq!(
            (wav.sample_rate, wav.right.is_none()),
            (render.sample_rate, true)
        );
        assert_eq!(wav.left, render.left);
        let (wav, render) = (read("Shapes/Vowel A.wav"), shape("Vowel A").unwrap());
        assert_eq!(
            (wav.sample_rate, wav.left),
            (render.sample_rate, render.left)
        );
    }
}

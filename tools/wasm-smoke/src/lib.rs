//! Exports for `run.mjs`. The encoders come in by `#[path]`, not by linking `honeydew-audio`, so the module has no
//! wasm-bindgen imports and instantiates with none.

#[path = "../../../crates/audio/src/mp3_writer.rs"]
mod mp3_writer;
#[path = "../../../crates/audio/src/ogg_writer.rs"]
mod ogg_writer;

/// 1 s of stereo noise at 44.1 kHz in -0.5..0.5 from an LCG: integer arithmetic and exact conversions only, so every
/// target makes the same samples (`sin` need not: wasm has no system libm).
fn noise() -> [Vec<f32>; 2] {
    let mut x = 1u32;
    let mut next = move || {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (x >> 8) as f32 / (1 << 24) as f32 - 0.5
    };
    [(); 2].map(|()| (0..44_100).map(|_| next()).collect())
}

/// A file's length in the high 32 bits and its FNV-1a hash in the low 32.
fn summary(file: &[u8]) -> u64 {
    let fnv = (file.iter()).fold(0x811c_9dc5_u32, |h, &b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    (file.len() as u64) << 32 | u64::from(fnv)
}

/// [`noise`] encoded as `Save → MP3` does with `High quality` off: [`summary`] of the file.
#[unsafe(no_mangle)]
pub extern "C" fn smoke_mp3() -> u64 {
    let mut encoder = mp3_writer::Mp3Encoder::new(44_100, false);
    encoder.push(&noise());
    summary(&encoder.finish())
}

/// [`noise`] encoded as `Save → OGG` does with `High quality` off: [`summary`] of the file.
#[unsafe(no_mangle)]
pub extern "C" fn smoke_ogg() -> u64 {
    let mut encoder = ogg_writer::OggEncoder::new(44_100, false);
    encoder.push(&noise());
    summary(&encoder.finish())
}

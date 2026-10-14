# rusty_mp3

[![crates.io](https://img.shields.io/crates/v/rusty_mp3.svg)](https://crates.io/crates/rusty_mp3)
[![docs.rs](https://img.shields.io/docsrs/rusty_mp3)](https://docs.rs/rusty_mp3)
[![Hardening](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/actions/workflows/mp3-hardening.yml/badge.svg)](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/actions/workflows/mp3-hardening.yml)
[![MSRV 1.85](https://img.shields.io/badge/MSRV-1.85-informational)](#compatibility)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue)](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/LICENSE)

A complete MP3 (MPEG-1, MPEG-2 and MPEG-2.5 Audio Layer III) **decoder and
encoder** in pure Rust. No C, no FFI, and no runtime dependencies.

- **Conformant.** The decoder passes all 16 ISO 11172-4 / 13818-4 Layer III
  conformance vectors and matches FFmpeg bit for bit on a 678-stream corpus.
- **Complete.** Encoding covers CBR (8–320 kbps) and VBR, mono, stereo and
  joint stereo, a psychoacoustic model, transient block switching, per-band
  noise shaping, the bit reservoir, and Xing/Info headers.
- **Fast.** On one x86-64 core, decoding runs at about 800× real time and
  encoding at about 120× real time. The decoder is faster than FFmpeg's.
- **Safe.** The only `unsafe` code is explicit SIMD, each kernel paired with a
  safe scalar reference that produces identical output. The crate is fuzzed,
  and its tests pass under Miri and the Address, Thread and Memory sanitizers.
- **Portable.** It builds for every Rust target. It uses AVX on x86-64 and NEON
  on AArch64, and WebAssembly builds produce bit-identical output to native.

MP3's patents expired in 2017, so the format is royalty-free worldwide.

## Installation

```toml
[dependencies]
rusty_mp3 = "1"
```

## Decoding

```rust
use rusty_mp3::{Error, Mp3Decoder};

fn main() -> Result<(), Error> {
    let bytes = std::fs::read("input.mp3").expect("read input");

    let mut decoder = Mp3Decoder::new();
    decoder.push(&bytes); // any chunking: the decoder finds frame boundaries itself
    decoder.flush();      // end of input

    let mut pcm = Vec::new(); // interleaved f32 samples in [-1, 1]
    loop {
        match decoder.next_frame() {
            Ok(frame) => pcm.extend_from_slice(&frame.samples),
            Err(Error::Again | Error::Eof) => break, // need more input / fully drained
            Err(e) => return Err(e),
        }
    }
    println!("decoded {} samples", pcm.len());
    Ok(())
}
```

`Mp3Decoder` is a streaming decoder. Push bytes as they arrive and pull frames
as they become available. Each frame reports its sample rate and channel count.
For a complete in-memory file, `decode_pipelined` runs the entropy and transform
stages on two threads. Its output is identical to the streaming decoder's.

## Encoding

```rust
use rusty_mp3::{Error, Mp3Encoder, Mp3EncoderConfig};

fn main() -> Result<(), Error> {
    let rate = 44_100;
    let pcm: Vec<f32> = (0..2 * rate)
        .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin())
        .collect();

    let mut encoder = Mp3Encoder::new(Mp3EncoderConfig {
        bitrate_kbps: 192, // CBR; snapped to the nearest legal Layer III bitrate
        vbr_quality: None, // or Some(rusty_mp3::vbr_quality_index(2.0)) for VBR
    });
    encoder.push_pcm_f32(&pcm, 1, rate)?; // mono, 44.1 kHz
    encoder.finish();                      // flush the tail and write the Info header

    let mut mp3 = Vec::new();
    while let Ok(packet) = encoder.next_packet() {
        mp3.extend_from_slice(&packet);
    }
    std::fs::write("out.mp3", mp3).expect("write output");
    Ok(())
}
```

PCM can be pushed as `f32` or `i16` slices, or directly as little-endian bytes
with `push_pcm_f32le` / `push_pcm_s16le`, the layout of a WAV `data` chunk.
The encoder converts its input in one pass and buffers at most one frame,
however much audio each call delivers.

## API overview

| Type / function | Purpose |
|---|---|
| `Mp3Decoder` | Streaming decoder: `push`, `next_frame`, `flush` |
| `decode_pipelined` | Two-thread decode of a complete byte slice |
| `Mp3Encoder`, `Mp3EncoderConfig` | Streaming encoder: `push_pcm_*`, `next_packet`, `finish` |
| `vbr_quality_index` | Maps an FFmpeg/LAME-style `-q:a` value (0 = best, 9 = smallest) to a VBR target |
| `Error` | Typed errors; `Again` and `Eof` follow FFmpeg's drain protocol |
| `header`, `decode`, `encode` | Frame-level building blocks: header parsing, per-frame decode and encode |

## Conformance and quality

**Decoder.** All 16 Layer III vectors from ISO/IEC 11172-4 and 13818-4 pass,
including mixed blocks and both forms of intensity stereo. On a 678-stream
corpus (streams from three encoders across nine sample rates, mono and stereo,
CBR and VBR) the output matches FFmpeg bit for bit. There is one deliberate
divergence: for MPEG-2 intensity stereo at positions FFmpeg does not
implement, rusty_mp3 follows the standard and the ISO reference output.

**Encoder.** Perceptual quality, measured with PEAQ against LAME at the same
bitrate (ODG difference; positive means LAME scores higher). The clips are three
24-second CC0 music recordings, 44.1 kHz mono:

| Clip | 96 kbps | 128 kbps | 160 kbps | 192 kbps |
|---|--:|--:|--:|--:|
| Guitar | +0.054 | +0.020 | −0.034 | +0.030 |
| Piano | +0.564 | +0.322 | +0.134 | +0.096 |
| Vocal | +0.295 | +0.334 | +0.174 | +0.125 |
| **Mean** | **+0.304** | **+0.225** | **+0.091** | **+0.084** |

At 160 kbps and above, rusty_mp3 is within about 0.1 ODG of LAME, and it is
ahead on the guitar clip at 160 kbps. The regeneration command and the full
methodology are in the
[development history](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/docs/history.md).

## Performance

Measured on one desktop x86-64 core with AVX, using a 412.9-second 44.1 kHz
stereo track:

| Operation | Time | Speed |
|---|--:|--:|
| Decode (192 kbps CBR) | 0.50 s | ~830× real time |
| Encode (192 kbps CBR) | 3.4 s | ~120× real time |

Against FFmpeg's native decoder (a 27.5-minute stream, both sides discarding
their output, pinned to the same cores, 15 paired runs):

| Cores | rusty_mp3 | FFmpeg | Result |
|---|--:|--:|---|
| 1 | 1,952 ms | 2,431 ms | rusty_mp3 1.24× faster |
| 2 (`decode_pipelined`) | 1,017 ms | 1,455 ms | rusty_mp3 1.39× faster |

Those figures predate later decoder optimizations, so current releases are at
least as fast.

## Platform support

| Target | Kernels | Status |
|---|---|---|
| x86-64 | AVX (runtime-detected), portable fallback | Primary; all measurements above |
| AArch64 | NEON | Bit-identical to the portable path (verified under emulation) |
| WebAssembly (`wasm32-unknown-unknown`, `wasm32-wasip1`) | Portable | Bit-identical to native, with no host imports |
| Any other Rust target | Portable | Supported |

Setting `MP3_ISA=scalar` in the environment forces the portable kernels, for
A/B testing.

## Security

The decoder treats every input byte as hostile, and a panic on malformed input
is treated as a bug.

- [Threat model](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/docs/threat-model.md):
  assets, adversaries, entry points, and a STRIDE analysis.
- [`UNSAFE.md`](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/UNSAFE.md):
  every `unsafe` block and the bounds argument for each.
- [Hardening audit](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/docs/plans/use-protection-please.md):
  gate-by-gate status, summarized at the end of this page.
- Continuous fuzzing of the decoder, the frame-level decoder and the encoder.
  The supply chain is checked with `cargo audit`, `cargo deny` and `cargo vet`;
  the crate has no runtime dependencies.
- Report vulnerabilities privately; see
  [`SECURITY.md`](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/SECURITY.md).

## Compatibility

- **Minimum supported Rust version: 1.85** for the library. The test suite and
  examples need Rust 1.88 or later, because of a development dependency.
- **Semantic versioning.** The 1.x public API is stable. Decoder output is
  bit-exact and stable across releases. Encoder output may change in minor
  releases when encoding quality improves; such changes are noted in the
  [changelog](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/CHANGELOG.md).

**Not supported:** free-format bitrate streams (the decoder rejects them) and
MPEG Layers I and II. The encoder does not produce intensity stereo or mixed
blocks. It uses the bit reservoir for MPEG-1 CBR up to 256 kbps, and a fixed
per-frame budget elsewhere.

## Part of Remade With Rust

This crate is the MP3 engine of
**[remade_ffmpeg_rs](https://github.com/Remade-With-Rust/remade_ffmpeg_rs)**, a
ground-up, permissively licensed Rust rebuild of FFmpeg: a drop-in
`ffmpeg`/`ffprobe` CLI on pure-Rust codecs, with no copyleft. Related crates
include [`rusty_h264`](https://crates.io/crates/rusty_h264),
[`rusty_h265`](https://crates.io/crates/rusty_h265),
[`rusty_vp9`](https://crates.io/crates/rusty_vp9),
[`rusty_aac`](https://crates.io/crates/rusty_aac),
[`rusty_flac`](https://crates.io/crates/rusty_flac) and
[`rusty-opus`](https://crates.io/crates/rusty-opus). See also
**[FFAI](https://github.com/Remade-With-Rust/FFAI)**, media for an AI-first world.

## About Mata Network

<!-- ORG BOILERPLATE — keep identical across repos -->

[Mata Network](https://www.mata.network) builds sovereign, self-hostable
infrastructure. **Remade With Rust** is our open-source home for the
permissively-licensed building blocks that work depends on.

<!-- /ORG BOILERPLATE -->

## License

Apache-2.0. See the workspace
[LICENSE](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/LICENSE).

---

<!-- HARDENING-TABLE:BEGIN generated by use-protection-please — edit docs/plans/use-protection-please.md, not this block -->
## Hardening status

**Tier** critical-path · **Audited** 2026-10-04 (deep) · **v1.0.0 gates** 13/15 · [Full checklist](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/docs/plans/use-protection-please.md)

`█████████████████░░░` **88%** &nbsp;·&nbsp; 30 Completed · 0 Scheduled · 4 Incomplete · 21 N/A

| Phase | ✅ Completed | 🗓 Scheduled | ⬜ Incomplete | · N/A |
|---|--:|--:|--:|--:|
| 0 — Threat modeling | 2 | 0 | 0 | 0 |
| 1 — Toolchain | 3 | 0 | 1 | 0 |
| 2 — Supply chain | 8 | 0 | 0 | 0 |
| 3 — Code level | 6 | 0 | 0 | 1 |
| 4 — Static analysis | 1 | 0 | 0 | 0 |
| 5 — Dynamic analysis | 3 | 0 | 0 | 0 |
| 6 — Fuzzing and properties | 3 | 0 | 1 | 0 |
| 7 — Formal verification | 0 | 0 | 1 | 0 |
| 8 — Build and binary | 0 | 0 | 0 | 2 |
| 9 — Runtime privilege | 0 | 0 | 0 | 1 |
| 10 — Cryptography | 0 | 0 | 0 | 3 |
| 11 — CI/CD, release, and operations | 4 | 0 | 1 | 0 |
| 12 — Compliance controls | 0 | 0 | 0 | 14 |
| **Total** | **30** | **0** | **4** | **21** |

**Architect** — Tim Almond
<!-- HARDENING-TABLE:END -->

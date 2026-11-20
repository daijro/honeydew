# Measured waveform workload — 0.5.21

Validated implementation: `a5bee71` on `arena/1c4cf8e7-honeydew`.

[Successful controls + release benchmark run](https://github.com/jellyscoll76-Real/honeydew/actions/runs/44748130014) · [Raw results, host information and process resources](https://github.com/jellyscoll76-Real/honeydew/actions/runs/44748130014/artifacts/14634556945).

## Method

Optimized Rust 1.91 Linux test harness, one 32-second 48 kHz stereo sample and eight visible clip meshes. Each cell is the median of three repeats. Decode/summary construction and compilation are outside timings. Cold build: 64 batches of eight meshes, clearing the geometry cache before each batch. Warm: 256 batches of eight cached Arc lookups. Tessellation: 128 CPU egui tessellations of the eight cached meshes. No GPU submission/execution timing is included.

Close: 1024 device pixels, 1 frame/pixel, density 1. Medium: 1024 pixels, 64 frames/pixel, density 1. Overview: 1024 pixels, 1024 frames/pixel, density 1. HiDPI: 2048 pixels, 16 frames/pixel, density 2. Existing automatic zoom LOD remains active in all modes.

## Results

Times below are **microseconds per batch of eight stereo clips**, not per frame of the complete application. Geometry bytes are raw vertex/index payload (20-byte vertices, six 32-bit indices per quad), not total heap/GPU memory. Counts exclude source PCM and summaries, which all modes share unchanged.

| View | Quality | Cold build µs | Warm lookup µs | CPU tessellation µs | Vertices | Indices | Geometry bytes |
|---|---|---:|---:|---:|---:|---:|---:|
| close | High | 388.19 | 0.45 | 254.58 | 65,536 | 98,304 | 1,703,936 |
| close | Balanced | 134.92 | 0.45 | 70.71 | 21,888 | 32,832 | 569,088 |
| close | Performance | 67.58 | 0.48 | 25.22 | 8,192 | 12,288 | 212,992 |
| medium | High | 2531.55 | 0.45 | 255.78 | 65,536 | 98,304 | 1,703,936 |
| medium | Balanced | 2421.16 | 0.45 | 71.06 | 21,888 | 32,832 | 569,088 |
| medium | Performance | 625.27 | 0.47 | 25.41 | 8,192 | 12,288 | 212,992 |
| overview | High | 236.85 | 0.45 | 121.36 | 32,768 | 49,152 | 851,968 |
| overview | Balanced | 100.67 | 0.45 | 34.70 | 10,944 | 16,416 | 284,544 |
| overview | Performance | 37.27 | 0.45 | 13.31 | 4,096 | 6,144 | 106,496 |
| hidpi | High | 1441.19 | 0.45 | 561.70 | 131,072 | 196,608 | 3,407,872 |
| hidpi | Balanced | 1291.13 | 0.45 | 180.28 | 43,712 | 65,568 | 1,136,512 |
| hidpi | Performance | 1223.60 | 0.45 | 50.60 | 16,384 | 24,576 | 425,984 |

## Interpretation and limits

- Balanced reduced actual vertices/indices/payload by about **66.6%**; Performance reduced them by **87.5%** in all four tested views. Short final groups account for rounding.
- CPU tessellation fell about **68–72%** for Balanced and **89–91%** for Performance in this controlled run.
- Cold-build savings are workload dependent: Balanced's medium view changed only 2531.55 → 2421.16 µs (~4.4%), while its close view changed 388.19 → 134.92 µs (~65.2%). Summary/PCM query cost can dominate generation even after vertex counts fall. Performance's HiDPI cold build changed 1441.19 → 1223.60 µs (~15.1%), not an 87.5% CPU improvement.
- Warm cache hits were effectively unchanged at ~0.45–0.48 µs/eight meshes. No cache-hit speedup is claimed.
- These are synthetic shared-renderer CPU timings, not complete-app CPU utilization, Android frame times, battery consumption, GPU time or audible-sync measurements. Editor/Slicer/vocal previews are functionally wired to the same policy, but these timings do not separately benchmark their full screens.
- Regression groups passed: quality/persistence/context tests (6, plus the separately run ignored benchmark), Audio Editor (44), waveform mesh cache (5), settings UI (49), follow handle (3), main-screen interactions (126), picker (9), Slicer (4), offline render (33), audio device settings (19). Groups overlap; these are not a unique-test total. All had zero failures on the validated commit.
- No source audio, song data or transport code is altered by the quality policy. Physical Android touchscreen, sustained playback responsiveness and frame tracing remain device acceptance work; see [testing instructions](WAVEFORM_QUALITY_TESTING.md).

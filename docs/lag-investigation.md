# Low-end phone lag: first pass

Priority: playtester issue #1. Reports describe both persistent lag and a startup
slowdown that settles. No device traces/videos have been supplied yet; the changes
below remove identified redundant work, not establish the report's root cause.

## Changes

- Fresh Air: skip the four stereo nonlinear evaluations when both Air amounts are
  exactly zero. Continue updating crossover/DC state so tails and later edits work.
- Fresh Air/Blood Overdrive: cache trim gain, and Blood's filter coefficient and
  drive compensation. Recompute for each changed **smoothed** scalar, including
  tiny automation changes; do not quantize controls. Invalidate rate-dependent
  filter cache when sample rate changes. No allocations or locks in this path.
- Vocal Studio: wait for 200 ms of unchanged preview inputs before starting an
  optional offline waveform job. Cancel stale work when edited inputs arrive and
  retain its receiver until exit rather than starting overlapping replacements.
  Existing thumbnails remain visible. Playback still cancels preview work; the
  quiet period delays only thumbnails, never notes or live audio.

## Validation

Added cache conversion-count, cached-vs-recalculated stereo/automation/rate-change,
and preview debounce tests. Existing finite/neutral DSP and real-time allocation
tests remain relevant. CI results must be checked before claiming these pass.
There is no local Rust toolchain in this workspace.

## Phone verification needed

Record phone model/RAM, Android version, app version and audio-buffer setting.
Use the same project for cold launch and a second playback, then compare:

1. Audio crackles/dropouts vs UI-only frame drops, and CPU meter before/after.
2. Empty song vs the affected project; number of synths/effects and visible screen.
3. Vocal Studio waveform display on/off, stopped pitch drags vs playback.
4. Fresh Air neutral/active and Blood Overdrive in the same rack.

Do not raise the default audio buffer blindly: that can hide underruns by adding
playing latency. Startup sample decoding, font/glyph warm-up, content scanning,
engine rebuilds and GPU fill remain candidates needing traces. No measured phone
speedup, startup root-cause fix or crash-free claim yet. Feedback #2–#7 is not
silently marked fixed by this performance pass.


## Arrangement waveform LOD / mobile sound-mode revision

- Waveform meshes now merge 2 columns at >=1/16 beat/device-pixel and 4 at
  >=1/8 beat/device-pixel, with a 2048-column maximum per mesh/channel. Close zoom
  keeps one-device-pixel detail. Each column queries the whole interval's min/max;
  it does not skip samples. The final partial column is queried separately,
  including reversed audio. Vertical amplitude resolution remains one pixel.
- Existing weak-source mesh caching remains memory-bounded. This reduces geometry
  generation/copy/upload; it does not yet introduce a multilevel sample-summary
  pyramid or establish a measured phone FPS improvement.
- Mobile Sound Mode restores stable IDs: NATIVE=3, FAST=1, SAFE=0, ULTRA SAFE=2.
  The raw slider is desktop-only; its persisted value cannot override mobile modes.
  Android requests device-selected/default, 256, 1024 and 2048 callback frames.
  iOS retains its session-managed buffer. Desktop controls remain unchanged.
- CPAL 0.18.2 Android backend (`realtime` feature already enabled) requests AAudio
  LowLatency and uses burst-based underrun tuning for the default/native stream.
  Fixed profiles request callback frames, not a guaranteed total output latency.
  CPAL's fixed-buffer capacity request is twice the callback size. No promise of
  96–192-frame bursts, near-zero latency, per-profile thread priority or special
  aggressive underrun algorithm is made. System/driver scheduling remains backend
  managed; denied real-time/low-latency requests are possible.
- Added LOD geometry/transient/reversal/tail tests and stable preset-policy tests;
  expanded playback regression workflow to exercise settings and mapping tests.
  Android smoke checks and on-phone FPS/underrun/latency comparisons still needed.

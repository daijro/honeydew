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

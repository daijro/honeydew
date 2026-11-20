# 0.5.24 — PAN playback, stereo waveform preview, contextual help

## Audio trace and scope

Playlist point gestures call `audio_editor::apply` / `song::points::set_points` with `Envelope(1)`. The Audio Editor uses the same clip events and independent `pan_shapes`. Mid-gesture `SongEdit::Audition` rebuilds through `AudioLink::replace_song_keeping`, preserving sounding voices with `Clips::adopt_envelopes`. `audio_clips::envelope` converts PAN controls/shapes to prepared events. Scheduled `ClipParam { index: 1 }` reaches `PadSampler::set_param`. The sampler reads PAN during each process block: left gain = `1-max(2*pan-1,0)`, right gain = `1+min(2*pan-1,0)`. Oboe output requests Stereo and copies separate interleaved output channels.

The pre-existing direct mono-fixture PAN PCM tests passed. This does not reproduce the reported phone symptom, and no physical phone/route claim is made. This update hardens the runtime path rather than pretending the line alone was the fix: compiled clip-local envelope state is reasserted at every active render boundary, with base parameters restored when no prior step applies, including seek/relaunch. Scheduled events still split blocks at automation boundaries; voices/phase are not restarted by this restoration. Processing uses bounded binary searches, no allocation, locks, or waveform work on the audio thread. Existing live handover tests continue to check sample phase.

PAN is **stereo balance**, not a mono fold-down/stereo-position effect: hard left silences the source right channel, not mixes its content into the left. Center retains both source channels. A mono speaker/OS accessibility mono-audio setting cannot reproduce stereo position. Channel/master effects and routing can change the final stereo result; test with a clean rack and stereo headphones as well as the user's normal project.

## Waveform semantics and cost

When PAN controls exist, Playlist and Audio Editor show two waveform rows: predicted left above predicted right, even for a mono source. Cached source peak summaries are reused. Display envelopes merge actual quantized VOL/PAN steps, applying their product to each channel's peak range. Broad overview columns conservatively retain the highest gain within the column, so a short abrupt transition can be visually wider at overview zoom. Zoom in for exact timing. This is pre-channel-effects output prediction, not a meter or audio analysis. Hiding curve overlays still does not change audio or erase automation.

Stereo gain caches are bounded to 64 entries/4 MiB; waveform geometry keys include both gain-envelope identities, so PAN edits invalidate visible output geometry without decoding source PCM. Colors/help do not affect gain caches. Mono duplication is a second read of cached peaks, not another decoded sample.

## Contextual help

A reusable small circular i icon with a 30-point touch target opens compact scrollable help. Workspace icons are beside the upper-right controls in Playlist, Piano Roll, Mixer and Audio Editor; relevant Settings rows have adjacent icons (Waveform Quality, Android Sound mode, envelope visibility/colors, continuous scrolling, key names). Tap outside, Escape, or × dismisses. Workspace navigation dismisses the previous workspace's help. Help content is static UI data, never sent to the audio engine. Existing controls/parameter values are not changed by help taps.

`src/ui/help.rs` is the content catalog. Feature changes should update the matching topic in the same patch; add a new Topic and call the reusable button for future unfamiliar controls. Content describes real behavior and important limitations, not generic labels. This is initial contextual coverage of the requested workspaces and advanced preferences, not a claim that every plugin parameter already has individual documentation.

## Automated and device acceptance

- PCM: left/center/right balance, true stereo source channel independence, live envelope handover with phase continuity, seek/base restoration, stale-slot restoration at a render boundary, VOL/PAN overlap and curve shapes, save/load round trip, visualization-hidden PCM equality.
- UI: quantized stereo preview gain/cache, help tap-to-open and Escape close, existing Playlist/Audio Editor gestures/sync/undo tests, project serialization, output-device regressions.
- Build: Android ARM64 release APK with independent package metadata/content verification.
- Physical acceptance still required: stereo headphones, phone speaker and Bluetooth routes; OS mono-audio OFF/ON; clean and effect-heavy racks; live point/shape drags during playback; short/landscape screen help positioning, outside dismissal and scroll; no accidental underlying edits; responsiveness on a large project. Synthetic tests cannot certify phone latency, FPS or absence of glitches.

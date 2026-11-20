# BanaForge 0.5.19 control-fix validation

## Changes and scope

- Show Key Names now gates **all** keyboard and Piano Roll key-column and note-block text, including root/octave names. The colored root markers remain (they are not text). The existing `showKeyNames` preference is retained, defaults off, and is saved using the existing settings storage.
- Android output now uses Oboe 0.6.1. Native requests low-latency shared output via Oboe's recommended backend (AAudio where supported/recommended; OpenSL ES fallback). It requests two device bursts. Fast explicitly uses OpenSL ES with LowLatency and a 256-frame callback; Safe uses OpenSL ES with performance None and 1024 frames; Ultra Safe uses OpenSL ES with performance None and 2048 frames. Compatibility modes request twice the callback size as capacity and one callback as active buffer. These are requests, not guarantees; devices may clamp them.
- Existing project/device rate negotiation is retained. An unexpected Oboe rate mismatch fails safely instead of running DSP at the wrong rate. Android's default route supports compatible external devices; no new manual route picker is claimed.
- A mode change closes/reopens output on the app thread, restoring transport, pending seek, loop, metronome, multicore setting, screen-only settings, live tempo and saved MIDI CC controllers. It can cause a brief gap and clear active voices/effect tails; it is not a seamless crossfade. Song/project data is not rewritten. On stream failure the existing recovery path retries; diagnostics distinguish requested policy from accepted configuration.
- Sound-mode preferences remain the original stable IDs: Safe=0, Fast=1, Ultra Safe=2, Native=3. Desktop and iOS output policies are unchanged. Offline export does not open this Android output stream or consume its buffer policy; its DSP/quality settings are unchanged.

## Diagnostics

On a connected Android phone:

```sh
adb logcat -c
adb logcat -s BanaForgeAudio:I '*:S'
```

Each stream open reports selected mode, actual Oboe backend, performance mode, sample rate, callback request, requested/accepted active buffer and capacity. Approximately every five seconds while the UI polls it reports actual callback frames, active buffer frames, callback CPU load and hardware xrun query result. OpenSL ES may not expose hardware xruns; in that case the in-app underrun counter uses callback deadline misses, not measured hardware underruns. Callback CPU load is render time divided by callback duration, **not whole-process CPU utilization**. Use Android Studio CPU Profiler or Perfetto for process CPU.

Buffer duration (`frames / sample_rate`) and queue-depth visualization are not measured touch-to-sound latency. Bluetooth and routing add latency. Do not infer measured latency or glitch-free playback from a requested buffer size.

## Automated checks

The Keyboard and sound mode validation workflow runs keyboard/roll rendering regressions, all-label hiding after a toggle/reopen, settings save/load for both label values and each platform-supported mode ID (four on Android, two on desktop), mode mapping, audio-device regressions, and offline-render regressions. The Android APK workflow compiles the actual Android path, verifies its signature, install metadata and ZIP CRCs. See workflow outcomes for the exact SHA; this document is not itself a passing test result.

## Required physical-device matrix — not yet performed

Use the same phone, route, sample rate and project for every run. Test built-in speaker, wired/USB output where supported, then Bluetooth separately. Run both a light synth project and the same heavy project for at least ten minutes per mode; repeat three times, keeping thermal/battery conditions comparable.

For Native / Fast / Safe / Ultra Safe, record:

- Backend, performance mode, callback size, active queue size, capacity and sample rate from diagnostics.
- Touch-to-sound median/p95 using a synchronized external audio/video or electrical-loopback measurement. Existing calibration measures round-trip latency, not touch-to-sound latency.
- Process CPU average/p95 via Perfetto/Android Studio, separately from callback CPU load.
- Hardware xrun delta if supported, otherwise explicitly mark unavailable and report deadline misses separately.
- Audible cracks/pops/stutters and any lost transport sync.
- Playback state and loop before/after changing modes, stopped and playing; change while monitoring/recording and verify recording integrity.
- Unplug/reconnect USB/headsets and verify recovery without a project edit or loss.
- Force-stop/relaunch and verify selected mode and key-label setting are restored.
- Export identical WAV settings in each mode and compare decoded PCM; no quality difference is expected because output profiles do not enter offline rendering.

Do not mark 0.5.18/0.5.19 fully Android-validated until these device measurements are completed. No phone or physical loopback equipment is attached to this coding environment.

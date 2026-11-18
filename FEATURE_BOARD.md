# Banana Studio — rack effects feature board

## Scope and status

**Status: implemented for 0.5.10; targeted automated validation passed, replacement APK build in progress.** All four now have registered module IDs, original DSP, parameter specs, browser entries and native cards. This board retains design goals below; those goals are not all release guarantees. Device listening, spectral/aliasing evaluation and mobile CPU measurements are still pending.

**Correction:** Stheven/Emmy's “air freshener” means a high-end audio exciter. The requested product is **Banana Fresh Air**, not a pine-tree animation. The mistaken UPDATES easter egg is removed. Fresh Air is separate from Gross Beat, Soundgoodizer, and Blood Overdrive; it is not being described as an FL Studio plugin.

Reference products establish functional goals only. Implement original Rust DSP, original presets and artwork, and native BanaForge controls. Do not copy proprietary code, assets, preset data, branding, or desktop screenshots. Do not promise identical algorithms or sound. Existing plugin compatibility IDs and third-party notices remain unchanged.

## Feature board

| Effect | Audio goal | Native horizontal rack card | Status |
|---|---|---|---|
| **Banana Fresh Air** | Mid/high harmonic excitement and top-end clarity | Two large knobs: **Mid Air**, **High Air**; smaller **Trim** knob | Implemented; targeted checks passed |
| **Banana Soundgoodizer** | One-knob multiband compression and saturation | One large central glowing **Amount** knob; large **A / B / C / D** buttons below | Implemented; targeted checks passed |
| **Banana Blood Overdrive** | Gritty, bass-friendly nonlinear drive for Phonk and Brazilian Funk | Rotary drive/tone/output controls within the standard card | Implemented; targeted checks passed |
| **Banana Gross Beat** | Custom time warps, tape-stop curves, stutters and volume envelopes | Time/volume envelope grid inside the card; thumb-friendly nodes, snapping and side preset slots | Implemented with fixed-grid causal delay curves; targeted checks passed |

This ordering delivers the smaller processors before the more complex real-time time-warp engine. It is not a release-date commitment.

## Shared UI / UX contract

- **Desktop processing capability, mobile-native presentation.** Use the existing right-side horizontal rack module layout, sizing rules, enable/collapse controls and parameter widgets. No separate desktop-style plugin window or copied desktop skin.
- Dark-slate card surfaces, clean borders, readable parameter values and high-contrast selection states. Respect the app's theme and accessibility scaling rather than hard-coding an entire screen.
- Large touch-friendly rotary controls with at least 44 logical-pixel hit targets where space permits. Reflow or use the rack's existing pages on narrow screens; never shrink a dense desktop panel to phone size.
- Double-click / double-tap resets to the documented default. Automation, undo/redo, MIDI control and project save/load work through the existing parameter system.
- Use subtle banana-yellow (`#F2C94C`) or coral-red (`#F07868`) as **proposed** default accents for Banana Studio cards without an explicit channel-color override. These are accents, not bright card backgrounds.
- Do not expose a browser entry until its DSP, serialization and rack panel are actually wired. No silent placeholder plugins.

### Dynamic channel-to-module color sync

**Required for effects and synths, not just the channel strip.**

1. An explicit channel RGB color is authoritative. Apply it to module headers/selection, knob arcs, active buttons, envelope curves and snap-node highlights throughout that channel's rack.
2. Resolve color from the channel every frame or invalidate cached visuals on a color edit. Do not snapshot a module's color when it is inserted.
3. If no explicit RGB override exists, use the Banana Studio yellow/coral accent for these cards; leave unrelated modules' existing color behavior unchanged.
4. Maintain contrast for labels, borders, inactive controls and envelope grids. A custom color must not make controls unreadable.
5. Color changes affect UI metadata only: no DSP reconstruction, sound changes or audio-thread work. Undo/redo, cloning, channel switching and project reload must restore the expected tint.

**Existing foundation:** the rack already resolves `channel.color_token()` through `theme::channel_color` and passes the resulting ink into native panels and plugin cards. Exact RGB is stored in the optional `BFCr` project extension while preserving legacy hue. New panels must consume this live ink, not hard-code their active colors. The four new cards now use yellow/coral fallback accents when no explicit RGB override exists. Physical-device contrast verification remains pending.

## Processor specifications

### 1. Banana Fresh Air

- **Mid Air / High Air:** separate, smoothly varying harmonic-excitation amounts. Zero on both must add no excitation; turning a knob should not simply act as a high-shelf EQ or an unfiltered full-band distortion.
- Start with complementary mid/high analysis bands and band-limited nonlinear enhancement. Determine crossover frequencies and harmonic curves through listening and measurement; do not claim the reference's proprietary topology.
- Protect the low end; include DC rejection and tame out-of-band products. Choose oversampling / antialiasing after CPU and spectral tests.
- **Trim:** output level, default 0 dB. Define its range before freezing parameter indices.
- Default excitation amounts: zero for safe insertion. Preserve stereo balance and handle silence without hiss, denormals or oscillation.
- Card: Mid Air and High Air dominate visually; Trim remains clearly accessible. Keep the rack's normal enable and common mix/output behavior consistent.

### 2. Banana Soundgoodizer

- **Amount:** smoothly blends / scales a coordinated multiband dynamics and saturation treatment; dry or neutral at zero.
- **A–D:** four original processing profiles with documented tonal/dynamic intent, not extracted proprietary preset curves. Select one profile at a time and visibly indicate it.
- Use complementary band splitting and stable detector smoothing. Avoid unexplained gain jumps when switching profiles; smooth or crossfade state changes.
- Keep the central knob's glow subtle, bounded, and tint-aware. Do not require continuous animation while idle.
- Evaluate loudness-matched examples and bass transients, not merely a louder output.

### 3. Banana Blood Overdrive

- Original nonlinear drive with pre-emphasis/filtering and post-shaping, tuned to preserve useful bass weight while allowing aggressive harmonics.
- Proposed exposed controls: **Drive**, **Color**, **Pre Filter**, **Post Filter**, **Trim**. Confirm the smallest useful touch layout before freezing the registry. Use the standard rack mix if no extra Mix control is needed.
- Test low-frequency/DC behavior, bass-note pitch stability, stereo tracking and aliasing at high drive. Output compensation must not unexpectedly mute the effect.
- No proprietary circuit-model or bit-identical-emulation claim.

### 4. Banana Gross Beat

- Separate editable **Time** and **Volume** envelope pages in the same card, with clear active-page indication. Native grid, large drag targets, endpoint handling, snap toggles and side preset slots.
- Define a deterministic piecewise curve format and bounded point count. Store curves in versioned custom module state; expose automation only where it has a stable, meaningful scalar representation.
- Dragging a node keeps it within the valid envelope domain. Nodes must remain selectable at phone scale; crossing / duplicate times need a documented policy. A gesture creates one undo step.
- Original preset examples: straight-through, half-rate, tape stop, repeated stutter and rhythmic volume gate. No reference preset-bank imports bundled with the app.
- Tempo-synchronized, bounded preallocated history buffer. Time curves can only read audio available in history; **do not promise arbitrary future-audio playback without explicit latency**.
- Decide and document the beat/bar convention, supported tempo range, maximum lookback and any introduced latency before implementation. Existing Half Time and Tape Stop are useful infrastructure to audit, not proof that Gross Beat is already implemented.
- Interpolated reads and short crossfades around jumps / preset switches / seeks. Smooth volume envelopes to avoid unintended clicks while preserving deliberately sharp stutters.
- Stop, seek, loop wrap and tempo changes must not play stale history or cause unbounded allocation.

## Engineering and verification gates

Before an effect is marked delivered:

- Audit registry / serialization IDs and allocate new stable IDs without reusing an existing one. Append parameters compatibly; old projects and unknown chunks must keep loading.
- Wire the audio processor, parameter registry, browser category, native panel, automation, project round trip, rendering/export and enable/bypass paths.
- Preallocate buffers off the audio thread. No file I/O, heap allocation or blocking UI locks in processing. Publish bounded control/curve updates safely and smooth audible scalar changes.
- Test silence, impulses, sine sweeps, stereo inputs, extreme controls, finite outputs, bypass/neutral behavior, multiple sample rates and block sizes. Add real-time allocation tests.
- Verify time-effect startup, transport stop/seek/loop, tempo changes and dry/wet latency alignment. Do not regress the existing audible-playhead work.
- UI tests: touch bounds and rack fit in portrait/landscape; reset gestures; A–D selection; envelope node gestures; explicit RGB tinting on all four cards; live recolor, undo/redo and reload.
- Measure several simultaneous instances on representative Android hardware. Reduce visual mesh/glow work at overview scale without silently changing the audio algorithm as the user zooms.
- Publish categorized release notes distinguishing implemented features, measured improvements, unsuccessful fixes and known limitations. Attach a tested APK only after the relevant checks complete.

## Existing unresolved work (not displaced by this board)

- Language switching / localization.
- Custom editor background images and opacity.
- Broader overview rendering performance.
- Reported physical-device playback delay and the specific wrong-sounding DWP file; these still need reproductions. New effects are not fixes for either issue.


## 0.5.10 implementation boundaries

- IDs 1009–1012; all enabled by the normal rack system. Fresh Air Trim spans −12 to +12 dB; both excitement amounts default to zero. DSP uses complementary 250/4500 Hz analysis bands and original nonlinear enhancement with DC rejection. No oversampling claim.
- Soundgoodizer uses linked multiband detectors, saturation and an Amount blend. A: mild compression; B: stronger compression; C: stronger saturation with gentler compression; D: strongest compression/saturation. Profiles are original, not proprietary preset data.
- Blood Overdrive exposes Drive, Color, Post Filter, Trim and Mix. A separate Pre Filter control is not in this initial layout.
- Gross Beat offers 16 fixed-time nodes per curve, 1/16 value snapping and preset selectors alongside the grid. Time nodes specify nonnegative delay as a fraction of the loop; they cannot read future audio. History is preallocated for eight seconds; delay requests clamp to available history. Curves and presets save as ordinary module parameters. Transport-stopped audition free-runs.
- Existing native controls provide tint-aware active states; no extra animated glow is added. Narrow-phone usability and physical-device audio remain unverified.
- Tests added for finite/stereo/neutral audio, audible parameter changes, specs/panels/default consistency, project round-trip, browser visibility, panel binding and allocation-free engine rendering. Consult CI for actual pass/fail, not this checklist.


Targeted CI passed at `20202ac`: https://github.com/jellyscoll76-Real/honeydew/actions/runs/44288442119 . This runs DSP tests, the registration/save-load integration test, all module-browser tests, panel contract tests and the real-time allocation integration suite. Vocal validation also passed for the equivalent runtime sources at `810a10c`: https://github.com/jellyscoll76-Real/honeydew/actions/runs/44287776405 . These results are not physical-device listening or full-suite certification.

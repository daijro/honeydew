# Android ARM compatibility

BanaForge supports Android 8.0/API 26 or newer, GLES 3.0, and either:

| Android userspace | APK native directory | Rust compilation target |
|---|---|---|
| 32-bit ARM | `armeabi-v7a` | `armv7-linux-androideabi` |
| 64-bit ARM | `arm64-v8a` | `aarch64-linux-android` |

A CPU advertised as 64-bit does not imply a 64-bit Android userspace. For the tester's SM-T510, use the ARMv7 or universal APK. The reported userspace has not been independently inspected on that physical tablet.

## Building / downloading

The Android APK workflow defaults to `abi=all`, producing a universal APK with both actual native engines and both matching `libc++_shared.so` runtimes. Its `abi` selector can instead produce `armeabi-v7a` or `arm64-v8a` APKs. Previous playtest links explicitly selected ARM64 to reduce download size; those artifacts cannot install on a 32-bit userspace.

`tools/android-dist.sh --release --base --abi all` runs cargo-ndk for both targets. This compiles the same Rust audio engine, Oboe/native audio backend and C/C++ dependencies (including time stretching) for each ABI. Android's NativeActivity/System.loadLibrary selects its process ABI from the APK; there is no Java-to-Rust raw pointer serialized as a 32-bit integer. Built-in Banana modules are compiled into the engine; this does not add support for desktop VST/CLAP binaries on Android.

All packaged `.so` files must pass actual ELF class/machine validation, same-ABI non-system DT_NEEDED closure checks, and Activity/JNI export checks. Existing signature, ZIP CRC, uncompressed-native-library and 16 KB alignment checks remain enabled. These are static packaging checks, not proof of successful runtime initialization.

## Physical acceptance (not performed in CI)

Use both a 32-bit Android device (including the reported SM-T510) and an ARM64 device:

1. Check `adb shell getprop ro.product.cpu.abilist` and Android version; install the universal APK or matching per-ABI APK. Record any installer error verbatim. A signing-key mismatch with an older sideload requires the matching signing key or uninstall/reinstall (back up projects first).
2. Cold launch; confirm no `UnsatisfiedLinkError`, `dlopen failed`, missing JNI callbacks or native crash in logcat. Record selected ABI and audio diagnostics.
3. Open the same small MIDI project; initialize Fast and Native/Oboe audio, play Piano Roll notes, edit/move/resize them, exercise all note styles and built-in instrument/effect modules, then stop/restart playback.
4. Import an audio clip, play/time-stretch it, change effects, grant microphone permission, record and replay. Check monitored and unmonitored recording and interruptions/headphones.
5. Repeat on ARM64; compare transport/project behavior, not assumed bit-identical floating-point output. Stress memory gradually: 32-bit address space is smaller, and full ARM64 project capacity is not promised.

No physical-device installation, microphone, latency, stability or performance results are claimed by the APK build checks.

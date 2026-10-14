<p align="center"><img src="assets/branding/honeydew.svg" width="160" alt="Honeydew logo"></p>
<h1 align="center">Honeydew</h1>
<p align="center">An open-source, clean-room reimplementation of FL Studio Mobile, rebuilt in pure Rust.</p>
<p align="center">It runs natively on the desktop and in the browser (wasm).</p>

<p align="center"><img src="assets/screenshots/01-playlist-sampler.webp" width="100%" alt="The playlist and a sampler's rack, playing the demo song Edwan - Permich - Tropical"></p>
<p align="center"><img src="assets/screenshots/02-playlist-supersaw.webp" width="100%" alt="The playlist and a synth's rack, playing the demo song Parrik - Initiated"></p>

## Screens

FL Studio Mobile demo projects, opened as `.flm` files.

<table>
  <tr>
    <td width="50%"><img src="assets/screenshots/03-piano-roll.webp" alt="Piano roll"><br><b>Piano roll</b>: Parrik - Initiated</td>
    <td width="50%"><img src="assets/screenshots/04-drum-editor.webp" alt="Drum editor"><br><b>Drums</b>: Nucleon - Shogun</td>
  </tr>
  <tr>
    <td><img src="assets/screenshots/05-mixer.webp" alt="Mixer"><br><b>Mixer</b>: Parrik - Initiated</td>
    <td><img src="assets/screenshots/06-effects-rack.webp" alt="Effects rack"><br><b>Effects rack</b>: Nucleon - Shogun</td>
  </tr>
  <tr>
    <td><img src="assets/screenshots/07-instrument-picker.webp" alt="Instrument picker"><br><b>Instrument picker</b>: Edwan - Permich - Tropical</td>
    <td><img src="assets/screenshots/08-songs-menu.webp" alt="Songs menu"><br><b>Songs menu</b>: the demo songs, over Parrik - Initiated</td>
  </tr>
  <tr>
    <td><img src="assets/screenshots/09-automation-editor.webp" alt="Automation editor"><br><b>Automation</b>: Parrik - Initiated</td>
    <td><img src="assets/screenshots/10-chord-editor.webp" alt="Chord editor"><br><b>Chord editor</b>: Edwan - Permich - Tropical</td>
  </tr>
  <tr>
    <td colspan="2" align="center"><img src="assets/screenshots/11-audio-editor.webp" width="50%" alt="Audio editor"><br><b>Audio editor</b>: Goldhands - Elisa (Featuring Tevlo)</td>
  </tr>
</table>

## How it was made

Honeydew is a clean-room reimplementation. The original app's behaviour, file formats and look were written down as a
specification, and the code was written only from that specification. Honeydew contains no code, samples, presets or
artwork from FL Studio Mobile.

Honeydew is an independent project, not affiliated with or endorsed by Image-Line. FL Studio is a trademark of
Image-Line.

## Bundled sounds

Honeydew bundles instruments, drum kits, one-shots, effects, loops and waveforms from these sources.
The app's SETTINGS → Sound credits screen includes the full notices.

For desktop, download the app for your operating system and `honeydew-content-0.1.0.zip` from
[Releases](https://github.com/daijro/honeydew/releases/latest). Extract the app, then extract the content zip into the
same folder: `content/` must sit beside `honeydew-native` or `honeydew-native.exe`. Restart Honeydew and the sounds
appear in the pickers automatically. No import step is needed.

The web release includes the sound library and loads sounds as you use them.

| Source | Author | Licence |
|---|---|---|
| [Adventure Kid Waveforms (AKWF-FREE)](https://github.com/KristofferKarlAxelEkstrand/AKWF-FREE) | Kristoffer Ekstrand (Adventure Kid) | CC0 |
| [FreePats Church Organ Emulation](https://freepats.zenvoid.org/Organ/pipe-organ.html) | Roberto (FreePats) | CC0 |
| [FreePats Concert Harp](https://freepats.zenvoid.org/OrchestralStrings/harp.html) | Versilian Studios samples, assembled by Roberto (FreePats) | CC0 |
| [FreePats Electric Organ Emulations](https://freepats.zenvoid.org/Organ/electric-organ.html) | Roberto (FreePats); Strix SoundFont Team (percussive, rock) | CC0 |
| [FreePats FM Pianos](https://freepats.zenvoid.org/ElectricPiano/synthesized-piano.html) | FreePats project and contributors | CC0 |
| [FreePats FSBS Clean Guitars](https://freepats.zenvoid.org/ElectricGuitar/clean-electric-guitar.html) | FreePats project and contributors | CC0 |
| [FreePats Old Piano FB](https://freepats.zenvoid.org/Piano/honky-tonk-piano.html) | FreePats project and contributors | CC0 |
| [FreePats Spanish Classical Guitar](https://freepats.zenvoid.org/Guitar/acoustic-guitar.html) | Roberto (FreePats) | CC0 |
| [FreePats Synth Bass](https://freepats.zenvoid.org/Synthesizer/synth-bass.html) | FreePats project and contributors | CC0 |
| [FreePats Synth Lead](https://freepats.zenvoid.org/Synthesizer/synth-lead.html) | FreePats project and contributors | CC0 |
| [FreePats Synth Pad](https://freepats.zenvoid.org/Synthesizer/synth-pad.html) | FreePats project and contributors | CC0 |
| [FreePats Upright Piano KW](https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html) | Gonzalo and Roberto (FreePats) | CC0 |
| [Big Rusty Drums](https://github.com/sfzinstruments/karoryfer.big-rusty-drums) | Karoryfer Samples | CC0 |
| [Bigcat Cello](https://github.com/sfzinstruments/karoryfer-bigcat.cello) | Karoryfer Lecolds x bigcat instruments (cellist Kamila Borowiak) | CC0 |
| [Body Percussion](https://github.com/sfzinstruments/body_percussion) | Karoryfer Samples | CC0 |
| [Caveman Cosmonaut](https://github.com/sfzinstruments/karoryfer.caveman-cosmonaut) | Karoryfer Samples | CC0 |
| [Frankensnare](https://github.com/sfzinstruments/karoryfer.frankensnare) | Karoryfer Samples | CC0 |
| [Growlybass](https://github.com/sfzinstruments/karoryfer.growlybass) | Karoryfer Samples | CC0 |
| [The Hat With The Phat](https://github.com/sfzinstruments/Karoryfer.TheHatWithThePhat) | Karoryfer Samples | CC0 |
| [Meatbass](https://github.com/sfzinstruments/karoryfer.meatbass) | Karoryfer Samples | CC0 |
| [Swirly Drums](https://github.com/sfzinstruments/karoryfer.swirly-drums) | Karoryfer Samples | CC0 |
| [Unruly Drums](https://github.com/sfzinstruments/karoryfer.unruly-drums) | Karoryfer Samples | CC0 |
| [War Tuba](https://github.com/sfzinstruments/karoryfer.war-tuba) | Karoryfer Samples | CC0 |
| [Digital Audio](https://kenney.nl/assets/digital-audio) | Kenney (kenney.nl) | CC0 |
| [Impact Sounds](https://kenney.nl/assets/impact-sounds) | Kenney (kenney.nl) | CC0 |
| [Interface Sounds](https://kenney.nl/assets/interface-sounds) | Kenney (kenney.nl) | CC0 |
| [RPG Audio](https://kenney.nl/assets/rpg-audio) | Kenney (kenney.nl) | CC0 |
| [Sci-fi Sounds](https://kenney.nl/assets/sci-fi-sounds) | Kenney (kenney.nl) | CC0 |
| [MuseScore_General](https://ftp.osuosl.org/pub/musescore/soundfont/MuseScore_General/) | S. Christian Collins; FluidR3 by Frank Wen; FluidR3Mono by Michael Cowgill; MuseScore | MIT |
| [512 Sound Effects (8-bit style)](https://opengameart.org/content/512-sound-effects-8-bit-style) | SubspaceAudio (Juhani Junkala) | CC0 |
| [Salamander Grand Piano V3](https://archive.org/details/SalamanderGrandPianoV3) | Alexander Holm | Public domain declaration |
| [Splendid Grand Piano](https://github.com/sfzinstruments/SplendidGrandPiano) | AKAI Professional (samples); SFZ by sfzinstruments | Public domain declaration |
| [Versilian Community Sample Library](https://github.com/sgossner/VCSL) | Versilian Studios LLC (Sam Gossner) | CC0 |
| [Virtuosity Drums](https://github.com/sfzinstruments/virtuosity_drums) | Versilian Studios x Karoryfer Samples (drummer Austin McMahon) | CC0 |
| [VSCO 2 Community Edition](https://github.com/sgossner/VSCO-2-CE) | Versilian Studios LLC (Sam Gossner) | CC0 |
| Honeydew procedural sounds | The Honeydew contributors | CC0 |

## Build

Desktop:

```sh
cargo run --release
```

Web (development):

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
trunk serve
```

In the browser, the app's shortcuts take priority over the browser's, except Ctrl+C, V and X. Chrome keeps
Ctrl+Shift+T for itself.

## Web release

```sh
tools/web-dist.sh          # build the site into dist/
tools/web-dist.sh --check  # check an existing build's files and sizes
tools/web-dist.sh --zip    # also write target/honeydew-web-<version>.zip
```

It needs Trunk and binaryen's `wasm-opt` (or set `WASM_OPT`).

- Release builds use `opt-level = 3`, fat LTO, SIMD and `wasm-opt -O3`.
- Supported browsers: Chrome 96, Firefox 113, Safari 16.4 or later. Older browsers see a notice.
- Everything is gzipped ahead of time. If the host doesn't send `Content-Encoding: gzip` (Cloudflare, for example),
  the browser unpacks the files itself. No file is over 25 MiB.

To host it, serve `dist/`. `deploy/` has example configs for nginx, Caddy and Cloudflare (`_headers`).

To test the web build with the bundled sounds locally:

```sh
python3 -m http.server 8081 --bind 127.0.0.1 --directory content-out &
trunk serve
```

## Desktop icon (Linux)

Wayland only shows an app's icon if a desktop entry is installed. To install it:

```sh
cargo build --release
tools/install-desktop-entry.sh              # add --uninstall to remove it
```

The entry runs `target/release/honeydew-native`. Run the script again if you move the repository.

## Using your own FL Studio Mobile sounds

Honeydew does not include any FL Studio Mobile content. If you own the app, you can import its sounds and presets
from your own copy. They stay in your Honeydew user folder; nothing is downloaded or uploaded, and exported project
ZIPs leave them out. Your FL Studio Mobile licence still applies to them, so don't share them.

1. Copy your app's `.apk` or `.obb` into the **Original Content** folder (SETTINGS has an `Open folder` button):

   | OS | Folder |
   |---|---|
   | Linux | `~/.local/share/honeydew/Original Content` |
   | macOS | `~/Library/Application Support/Honeydew/Original Content` |
   | Windows | `%APPDATA%\Honeydew\data\Original Content` |

2. Start Honeydew (or open SETTINGS) and choose `Import`.

Or from the command line:

```sh
cargo run --release --bin honeydew-import -- path/to/your.apk
```

Imported sounds and presets show up in the pickers, and demo songs that use them play with the right sounds. Each
file imports once; to import it again, delete the hidden `.imported-…` file next to it. Existing files are never
overwritten. The web version can't import.

### Getting the files from your phone

Use only a copy you own, from your own device. With USB debugging on and Android's platform tools installed:

```sh
adb shell pm list packages | grep -i studio   # find the package name
adb shell pm path <package>                    # find the APK (pull base.apk first if there are several)
adb pull <path>
adb pull /sdcard/Android/obb/<package>/        # the OBB, if the app has one
```

## License

Code: [GPL-3.0-or-later](LICENSE). Copyright (c) 2026 The Honeydew contributors.

Font: Open Sans SemiCondensed SemiBold, SIL Open Font License 1.1 ([`assets/fonts/OFL.txt`](assets/fonts/OFL.txt)).

The icon is a derivative of "Melon" by afitrose. It is licensed as Free for commercial use WITH ATTRIBUTION.

Bundled sounds: each source and its licence is listed in the app under SETTINGS → Sound credits.

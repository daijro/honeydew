# Content recipes

One TOML file per bundled entry (an instrument, a kit or a sample folder). `tools/content-build` turns the recipes plus
the raw source folders into `content-out/`: pack parts, `index.json` and `CREDITS.txt`, each with a `.gz` beside it.
Nothing it writes goes into git.

```sh
cd tools/content-build
cargo run --release -- check  --content ../../content            # gates only; needs no sources
cargo run --release -- build  --content ../../content --sources <content-sources> --out ../../content-out [--only <id>]... [--update-lock] [--release <ver>]
cargo run --release -- verify --out ../../content-out [--previous <old index.json>]
```

## Files in `content/`

- `sources.json`: the raw source folders, copied from content-sources' `MANIFEST.json` (its folder-mapping column dropped,
  `PD-declared` written `LicenseRef-PD-declared`), plus a `title` for the credits screen, an optional `credit` line and,
  for MIT, the `notice` file whose text `CREDITS.txt` carries.
- `inputs.lock`: `<sha256>  <folder>/<path>` for every source file a build opened. A changed or new file fails the build
  until `--update-lock` records it. The upstream archives were deleted after unpacking, so these hashes pin the tree.
- `recipes/**/*.toml`: the recipes.

## Recipe fields

| field | meaning |
|---|---|
| `id` | Stable unique id, `[a-z0-9-]`. |
| `kind` | `program`, `kit` or `samples`. |
| `path` | Place below `Built-in/`: `Presets/DW Sampler/<Category>/<Name>.dwp`, `Drumsets/<Name>.tmpl`, or a folder `Samples/<Category>/`, `Loops/` or `Shapes/`. |
| `source` | The `sources.json` folder files are read from. Paths are relative to it. |
| `[[sample]]` | `name`, `file`, optional `source`: one `<name>.flac` in a `samples` folder. |
| `[[pad]]` | `name`, `level_db`, `pan`, optional `source`, and `mix = [{ file, gain_db, pan }]` mic takes summed into one sample, or `render = "<model>"` with `params = { … }` for a synthesized pad. |
| `render`, `[[variant]]` | A synthesized `samples` folder (`procedural/`, source `honeydew`): each variant is `name` plus the model's parameters, and may name its own `render`. Models and their parameters are in `tools/content-build/src/procedural/`; `loop` sequences them (`procedural/loops.toml`). |
| `sfz`, `cc`, `keys`, `velocity_layers`, `round_robin`, `channels`, `tail`, `release`, `lite`, `gain`, `loop` | Program options for the SFZ and DSP stages. |
| `files`, `layers`, `transpose` | A program from a sample tree without an SFZ, in place of `sfz`: `files = "<folder>/<file name pattern>"`. The file name may hold `{note}` (a name such as `C#4`, or a MIDI number), `{layer}` and `{rr}` (digits); the folder holds none. `layers` lists the `{layer}` values kept, softest first (default every one: numbers in order, then text). `transpose` adds semitones to each note: VSCO 2 and VCSL name middle C `C3`, so they take `12` (VCSL's TX81Z Clavisynth `24`). On an `sfz` it moves every region's keys and root (a source mapped an octave off); `[[zone]]`s are never moved. |
| `[[zone]]` | A program's zones written out, in place of `sfz` or `files`, for trees a pattern can't describe: `file`, `root` (a MIDI note), optional `lokey`/`hikey` (default the root), `lovel`/`hivel` (default 0 and 127) and `tune` (cents the file is raised by, for a recording between semitones). Each zone plays exactly there: no gap is filled. One zone per root and `lovel`. |

Unknown fields are errors.

## Gates (`check`, and the start of every `build`)

- Licences: `CC0-1.0`, `LicenseRef-PD-declared`, `MIT` with a `notice`, `CC-BY-3.0`/`CC-BY-4.0` with a `credit`. One
  licence per entry. Nothing from a banned host (sample stores and Image-Line's own hosts).
- Inputs: only `.sfz`, `.wav`, `.flac`, `.ogg` and `.sf2`, inside their source folder. Kontakt files are never read.
- Names: the path, pad and sample names contain no FL, Image-Line, Rhodes, Wurlitzer, Steinway, Yamaha, Roland,
  TR-808/909, TB-303, Kontakt, Fender, Hammond or Leslie. Sources may name these in their credits.
- Ids and paths are unique, and each `path` has its kind's shape.

## Packs

Members go into uncompressed ustar parts of at most 24,000,000 bytes, both raw and gzipped. A member never spans parts,
parts never mix entries, and part 0 closes after an entry's lite set. Each part is named `p/<first 16 hex of its
sha256>.tar`. `verify` checks the parts against the cap and their hashes, checks the index budget, makes sure no bundled
stem shadows a code-made Built-in sound, and with `--previous` makes sure every path a song may store is kept or
aliased.

## Stages

- **`samples`**: FLAC inputs pass through as they are.
- **`program`** (`src/stages.rs`): the SFZ's attack regions of round robin `round_robin` (default 1), with `cc` set.
  A sample point is a `pitch_keycenter`; `keys` keeps the first in `from..=to` and each next one at least `every`
  semitones above the last kept, and the kept points split the source's key span at their midpoints. At each point
  the regions by `lovel` are layers 1, 2 …; `velocity_layers` keeps those listed (all when empty), and they split
  velocities 0–127 evenly. Each sample is resampled to 44.1 kHz and folded by `channels` (`auto`, `mono`,
  `stereo`); then `loop = { find = true, … }` searches and bakes a loop, an SFZ `loop_continuous` keeps the source's
  own loop (its end the one of `loop_end` and `loop_end + 1` that continues the audio into `loop_start` best), and
  anything else is trimmed by `tail`. `gain = "normalize"` puts the loudest sample at −6 dBFS, for chord headroom.
  `release` is the DW Release float. `lite = { every, layer }` adds `<Name>/lite.dwp`: that source layer only, every
  `every` semitones, over the same files.

  A `files` pattern's matches go through the same steps as SFZ regions, each file a point at its `{note}` (plus
  `transpose`) that spans 0–127 until neighbours split it, with its `{layer}` as the `lovel` order. Of one note and
  layer's files, the `{rr}` equal to `round_robin` (default 1) is kept, else the lowest. Names that don't match, and
  layers not in `layers`, are skipped; two files of one note, layer and round robin are an error. For example:

  ```toml
  source = "vcsl"
  files = "unpacked/VCSL-master/Idiophones/Struck Idiophones/Marimba/Marimba_hit_Outrigger_{note}_{layer}_{rr}.wav"
  layers = ["soft", "med", "loud"]
  transpose = 12
  ```

  ```toml
  source = "vsco2-ce"
  files = "unpacked/VSCO-2-CE-master/Strings/Violin Section/Pizz/VlnEns_Pizz_{note}_v{layer}_rr{rr}.wav"
  transpose = 12
  ```

  `[[zone]]`s are read as written. Neither has a loop of its own: `loop = { find = true, … }` finds one, and without
  it each sample is trimmed as a one-shot (mallets, plucks).
- **`kit`**: each `[[pad]]` sums its `mix` takes, folds by `channels`, is trimmed by `tail` and peaks at −1 dBFS. The
  `.tmpl` sets the pad's Level (`level_db`) and Pan (−1…1), puts pads named `…Hat` in cut group 1 and points `PTH1` at
  `<Name>/<Pad>.flac`.

Members are 16-bit 44.1 kHz FLAC with seeded TPDF dither. A build also writes `sizes.txt`: each entry's parts raw and
gzipped and its decoded f32 bytes, per category and in all.

# BanaForge branding

`banana-source.webp` is the image supplied by the app owner for this rebrand.
It has native transparency; generated assets preserve its proportions and alpha.
The original author and redistribution license have not been established. The
app owner's upload is not a license grant for third parties: verify permission
before publishing or redistributing these assets. The former melon logo is no
longer used, and its attribution does not apply to this replacement image.

Run `python3 tools/branding/generate.py` with Pillow installed to regenerate:

- `banaforge.svg` and desktop/web PNG icons;
- white-alpha banana masks for the existing tintable in-app logo renderer;
- Linux hicolor icons and the opaque iOS icon;
- Android app, splash, adaptive foreground and monochrome layers.

The adaptive foreground uses the central 60% of its canvas, inside Android's
safe region. The original full-color image is retained in the launcher and
splash; in-app and Android themed icons use its silhouette.

## Compatibility and retained names

The app/package/crate name is `BanaForge` / `banaforge`; Android now uses
`dev.daijro.banaforge`, and the native library is `libbanaforge.so`.
Plugin display names, Rust classes and crate paths use `Banana` / `banana`.

Intentional old-name occurrences are limited to compatibility/provenance:

- CLAP IDs (`org.openstudio.lemon-*`) and 16-byte VST3 class IDs are stable
  serialized identities, not titles. Renaming them would strand saved projects.
- `Honeydew` folder names and settings keys are read only for migration.
  Desktop migration does not merge two populated libraries or delete old data.
- Original copyrights, license documents, historical vendor patches, real
  upstream URLs and original upstream sync paths are not new-brand claims.
- The existing multi-gigabyte sound archive parts retain their tracked filenames;
  release outputs are named `banaforge-*`. No sound data was duplicated.
- The GitHub repository remains `jellyscoll76-Real/honeydew`; repository and branch
  renaming are separate operations and have not been performed.

**Android:** because its application ID changed, BanaForge installs alongside
Honeydew and cannot directly read Honeydew's private sandbox. Export songs/banks
from the old app and import them into the new app; do not uninstall before export.

Run `python3 -m unittest discover -s tests/branding -v` for the static package,
JNI, resource, crate-path and plugin-identity contract tests.

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

# App font

The app expects this file here:

    assets/fonts/OpenSans_SemiCondensed-SemiBold.ttf

It is Open Sans, SemiCondensed width, SemiBold weight. The font is licensed under the SIL Open Font License (OFL).

Get it from Google Fonts: family "Open Sans", style SemiCondensed SemiBold
(<https://fonts.google.com/specimen/Open+Sans>). Download the family and take the static
`OpenSans_SemiCondensed-SemiBold.ttf` from the archive.

The font is committed here together with its license (`OFL.txt`, from the official Open Sans repository).
Source: the static file from the Google Fonts "Open Sans" download bundle (version 196805,
sha256 `d4baec1280b3c9be62f48abfd8acfd09a04958d663d316d16c4e567a32440828`).

`build.rs` detects the file: with the file present it is embedded and becomes the first
`Proportional` font; without it the app builds and runs with egui's built-in fonts. After adding or removing the
file, `cargo build` picks the change up automatically.

## Japanese fallback (voicebank metadata and lyrics)

`MPLUS1p-Regular.ttf` is **M PLUS 1p Regular**, from
https://github.com/google/fonts/tree/main/ofl/mplus1p (font blob
`e16cd93613c26bb9d66d2060d6fd491c7222689f`), SIL Open Font License 1.1.
See `MPLUS1p-OFL.txt`. It is embedded only as a fallback after the app fonts,
so Japanese filenames, bank names and kana lyrics render without replacement boxes.
This freely licensed font is not part of any user voicebank.

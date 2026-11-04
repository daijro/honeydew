# Branding

Logo supplied by the owner (2026-10-14) as two source files: `honeydew (1).svg` (the coloured melon) and
`honeydew-mono.svg` (its one-colour silhouette, one `currentColor` path). Both derive from "Melon" by afitrose,
licensed as Free for commercial use WITH ATTRIBUTION, credited in the README and in the app.

- `honeydew.svg`: the coloured app icon (a copy of `honeydew (1).svg`).
- `honeydew-{16,32,256}.png`: its rasterisations. The web page's icons are the 16 and 32 (`index.html`),
  the window icon the 256 (`src/main.rs`).
- `../linux/icons/hicolor/`: the Linux desktop icons (`<n>x<n>/apps/honeydew.png` for n = 16, 32, 48, 64, 128, 256 and
  512, and the cropped `scalable/apps/honeydew.svg`), which `tools/install-desktop-entry.sh` installs.
- `honeydew-ios-512.png`: the iOS app icon, the melon at about 72 % on an opaque honeydew `#F0FFF0` square (iOS draws an
  icon's transparency black), which `tools/ios-dist.sh` scales to each icon size.
- `honeydew-mono.svg`: the silhouette (a copy of the owner's file).
- `honeydew-mono-{48,64,128,256}.png`: its rasterisations, white on transparency: the in-app logo (`AppLogo`, tinted by
  the draw: `menuFill` on the main screen's logo button, `text` in the main menu header; `src/icons/logo.rs`).

Both sources have a 1200 × 1000 viewBox with the melon in x 185–1008, y 105–873. The coloured rasterisations crop it
to the square `176.5 69.25 840 840` around the coloured melon. The silhouette (x 184.75–977, y 105.25–873.25) is
cropped to the same-sized square `160.875 69.25 840 840` centred on its own bounds, so the in-app logo sits centred
wherever it is drawn, and is drawn white:

```sh
BOX='s/viewBox="0 0 1200 1000"/viewBox="176.5 69.25 840 840"/'
for n in 16 32 256; do
  sed "$BOX" honeydew.svg | rsvg-convert -w $n -h $n -o honeydew-$n.png
done
H=../linux/icons/hicolor
for n in 16 32 48 64 128 256 512; do
  mkdir -p $H/${n}x$n/apps
  sed "$BOX" honeydew.svg | rsvg-convert -w $n -h $n -o $H/${n}x$n/apps/honeydew.png
done
mkdir -p $H/scalable/apps && sed "$BOX" honeydew.svg > $H/scalable/apps/honeydew.svg
sed 's/viewBox="0 0 1200 1000"/viewBox="13 -94.25 1167 1167"/' honeydew.svg |
  rsvg-convert -w 512 -h 512 --background-color '#F0FFF0' | magick - -alpha off honeydew-ios-512.png
MONO='s/viewBox="0 0 1200 1000"/viewBox="160.875 69.25 840 840"/'
for n in 48 64 128 256; do
  sed -e "$MONO" -e 's/color="#1C1B1F"/color="#FFFFFF"/' honeydew-mono.svg | rsvg-convert -w $n -h $n -o honeydew-mono-$n.png
done
```

# Branding

Logo supplied by the owner (2026-10-14) as two source files: `honeydew (1).svg` (the coloured melon) and
`honeydew-mono.svg` (its one-colour silhouette, one `currentColor` path); check its license and attribution requirement
before release.

- `honeydew.svg`: the coloured app icon (a copy of `honeydew (1).svg`).
- `honeydew-{16,32,256}.png`: its rasterisations. The web page's icons are the 16 and 32 (`index.html`),
  the window icon the 256 (`src/main.rs`).
- `../linux/icons/hicolor/`: the Linux desktop icons (`<n>x<n>/apps/honeydew.png` for n = 16, 32, 48, 64, 128, 256 and
  512, and the cropped `scalable/apps/honeydew.svg`), which `tools/install-desktop-entry.sh` installs.
- `honeydew-mono.svg`: the silhouette (a copy of the owner's file).
- `honeydew-mono-{48,64,128,256}.png`: its rasterisations, white on transparency: the in-app logo (`AppLogo`, tinted by
  the draw: `menuFill` on the main screen's logo button, `text` in the main menu header; `src/icons/logo.rs`).

Both sources have a 1200 × 1000 viewBox with the melon in x 185–1008, y 105–873. The rasterisations crop it to the
square `176.5 69.25 840 840` around the coloured melon (the same box for both, so the silhouette lies where the colours
do), and the silhouette is drawn white:

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
for n in 48 64 128 256; do
  sed -e "$BOX" -e 's/color="#1C1B1F"/color="#FFFFFF"/' honeydew-mono.svg | rsvg-convert -w $n -h $n -o honeydew-mono-$n.png
done
```

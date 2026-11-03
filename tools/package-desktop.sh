#!/usr/bin/env bash
# A desktop release build's two archives, from the repository root: <name>-no-sounds, the app alone, which uses the
# sound library installed in the app data folder (the release's own `honeydew-<tag>-sounds.zip`), and <name>, with the
# sound library in `content/`. .zip on Windows, .tar.gz elsewhere. The Linux, Alpine and FreeBSD ones (built on Linux
# runners) carry the desktop entry and its installer.
#
#   tools/package-desktop.sh NAME BIN_DIR   (RUNNER_OS: Linux, Windows or macOS, as in GitHub Actions)
set -euo pipefail
dir=$1
bin=$2
mkdir "$dir"
ext=; [ "$RUNNER_OS" = Windows ] && ext=.exe
cp "$bin/honeydew-native$ext" "$bin/honeydew-import$ext" README.md LICENSE "$dir/"
if [ "$RUNNER_OS" = Linux ]; then
  cp -R assets/linux tools/install-desktop-entry.sh "$dir/"
fi
# The archive of the folder as it is, named $1 and .zip on Windows, .tar.gz elsewhere.
pack() {
  if [ "$RUNNER_OS" = Windows ]; then
    7z a "$1.zip" "$dir" >/dev/null
  else
    tar czf "$1.tar.gz" "$dir"
  fi
}
pack "$dir-no-sounds"
# Without the web's gzipped copies (`*.gz`, ~700 MB): only the web reads them.
cat sounds/honeydew-sounds.zip.[0-9][0-9][0-9] > target/honeydew-sounds.zip
if [ "$RUNNER_OS" = Windows ]; then
  7z x target/honeydew-sounds.zip "-o$dir" '-xr!*.gz' >/dev/null
else
  unzip -q target/honeydew-sounds.zip -d "$dir" -x '*.gz'
fi
pack "$dir"

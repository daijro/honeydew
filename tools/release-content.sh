#!/usr/bin/env bash
# Builds target/honeydew-sounds.zip, a `content/` folder that desktop users unzip next to honeydew-native, and
# splits it into tracked honeydew-sounds.zip.* parts for release CI. With --web, also rebuilds the web zip with
# the sounds inside (needs trunk, wasm-opt and unzip).
#
#   tools/release-content.sh <version> [--web]
#
# The raw sources are read from $HONEYDEW_SOURCES, default ../content-sources beside the repository.
set -euo pipefail
v=${1:?usage: tools/release-content.sh <version> [--web]}
root=$(git rev-parse --show-toplevel)
sources=${HONEYDEW_SOURCES:-$root/../content-sources}
[ -d "$sources" ] || { echo "no sources at $sources (set HONEYDEW_SOURCES)" >&2; exit 1; }
cd "$root/tools/content-build"
cargo run --release -q -- build --content ../../content --sources "$sources" --out ../../content-out --release "$v"
cargo run --release -q -- verify --out ../../content-out
cd "$root"
pkg=target/release-content
rm -rf "$pkg" && mkdir -p "$pkg" && cp -R content-out "$pkg/content"
rm -f target/honeydew-sounds.zip
(cd "$pkg" && zip -qr0 ../honeydew-sounds.zip content)
split -b 90000000 -d -a 3 target/honeydew-sounds.zip "$pkg/honeydew-sounds.zip."
rm -f honeydew-sounds.zip.[0-9][0-9][0-9]
mv "$pkg"/honeydew-sounds.zip.[0-9][0-9][0-9] .
rm -rf "$pkg"
files=(target/honeydew-sounds.zip)
if [ "${2:-}" = --web ]; then
    tools/web-dist.sh --zip
    files+=("${CARGO_TARGET_DIR:-target}/honeydew-web-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1).zip")
fi
ls -lh "${files[@]}"
echo "Commit honeydew-sounds.zip.* with the content changes; tagged releases upload the sounds automatically."

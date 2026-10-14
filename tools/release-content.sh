#!/usr/bin/env bash
# Builds the bundled sounds and packs them for a GitHub release: target/honeydew-content-<version>.zip, a `content/`
# folder that desktop users unzip next to honeydew-native. With --web, also rebuilds the web zip with the sounds
# inside (needs trunk and wasm-opt).
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
rm -f "target/honeydew-content-$v.zip"
(cd "$pkg" && zip -qr0 "../honeydew-content-$v.zip" content)
rm -rf "$pkg"
files=("target/honeydew-content-$v.zip")
if [ "${2:-}" = --web ]; then
    tools/web-dist.sh --zip
    files+=("${CARGO_TARGET_DIR:-target}/honeydew-web-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1).zip")
fi
ls -lh "${files[@]}"
echo "upload: gh release upload v$v ${files[*]} --clobber"

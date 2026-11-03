#!/bin/sh
# The Alpine (musl) release build, run as root in an Alpine container with the repository as the working directory:
# Rust from rustup, and the binaries linked to musl dynamically, as a static binary cannot load the X11, Wayland and
# GL libraries at run time. libclang is bindgen's (signalsmith-stretch, Lemon Sampler's time stretching).
# Usage: tools/alpine-build.sh. It leaves honeydew-native and honeydew-import in target/release (or
# $CARGO_TARGET_DIR/release), owned by the repository's owner.
set -eu
apk add --no-cache build-base pkgconf linux-headers alsa-lib-dev clang-dev libxkbcommon-dev wayland-dev rustup
rustup-init -y -q --profile minimal --default-toolchain 1.99.1
. "$HOME/.cargo/env"
RUSTFLAGS='-C target-feature=-crt-static' cargo +1.99.1 build --release --locked --bins
chown -R "$(stat -c %u:%g .)" "${CARGO_TARGET_DIR:-target}"

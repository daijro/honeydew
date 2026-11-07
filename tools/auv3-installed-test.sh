#!/bin/sh
# Test one installed macOS AUv3 without an audio device or a Honeydew project.
# Usage: tools/auv3-installed-test.sh 'Super J8' [--editor]
set -eu
cd "$(dirname "$0")/.."
out=${CARGO_TARGET_DIR:-target}/auv3-installed-test
mkdir -p "$out"
xcrun clang -fobjc-arc -fblocks -Wall -Wextra -Werror -Wno-unused-parameter \
    src/plugins/auv3/bridge.m tools/auv3-installed-test.m \
    -framework Foundation -framework AppKit -framework CoreAudioKit \
    -framework AVFoundation -framework AudioToolbox -framework CoreFoundation \
    -o "$out/auv3-installed-test"
"$out/auv3-installed-test" "$@"

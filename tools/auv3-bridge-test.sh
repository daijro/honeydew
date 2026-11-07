#!/bin/sh
# macOS smoke test using registered test Audio Units; no third-party plugins required.
set -eu
cd "$(dirname "$0")/.."
out=${1:-target/auv3-bridge-test}
mkdir -p "$out"
xcrun clang -fobjc-arc -fblocks -Wall -Wextra -Werror -Wno-unused-parameter \
    src/plugins/auv3/bridge.m tools/auv3-bridge-test.m \
    -framework Foundation -framework AVFoundation -framework AudioToolbox -framework CoreFoundation -framework AppKit -framework CoreAudioKit \
    -o "$out/auv3-bridge-test"
"$out/auv3-bridge-test"

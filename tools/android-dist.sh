#!/bin/sh
# Build an arm64 APK with the bundled sounds. Release signing uses environment variables, never checked-in keys.
set -eu
cd "$(dirname "$0")/.."

variant=debug
task=assembleDebug
case ${1:-} in
    '') ;;
    --release) variant=release; task=assembleRelease ;;
    *) echo "usage: tools/android-dist.sh [--release]" >&2; exit 2 ;;
esac
if [ "$variant" = release ]; then
    : "${ANDROID_KEYSTORE_FILE:?set the path to your release keystore}"
    : "${ANDROID_KEYSTORE_PASSWORD:?set your keystore password}"
    : "${ANDROID_KEY_ALIAS:?set your signing key alias}"
    : "${ANDROID_KEY_PASSWORD:?set your signing key password}"
    [ -f "$ANDROID_KEYSTORE_FILE" ] || { echo 'release keystore does not exist' >&2; exit 1; }
    ANDROID_KEYSTORE_FILE=$(cd "$(dirname "$ANDROID_KEYSTORE_FILE")" && pwd)/$(basename "$ANDROID_KEYSTORE_FILE")
    export ANDROID_KEYSTORE_FILE
fi
: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK directory}"
export ANDROID_SDK_ROOT=$ANDROID_HOME
export ANDROID_NDK_HOME=${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/28.0.13004108}
sdk_tools=$ANDROID_HOME/build-tools/36.0.0
for tool in cargo python3 unzip java keytool; do
    command -v "$tool" >/dev/null || { echo "needs $tool on PATH" >&2; exit 1; }
done

mkdir -p "${CARGO_TARGET_DIR:-target}"
export CARGO_TARGET_DIR=$(cd "${CARGO_TARGET_DIR:-target}" && pwd)
export HONEYDEW_ANDROID_BUILD_DIR=$CARGO_TARGET_DIR/android
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
mkdir -p "$HONEYDEW_ANDROID_BUILD_DIR/assets" "$HONEYDEW_ANDROID_BUILD_DIR/jniLibs"
cat honeydew-sounds.zip.[0-9][0-9][0-9] > "$HONEYDEW_ANDROID_BUILD_DIR/sounds.zip"
# The .gz siblings are for web servers; Android reads the raw packs directly from its APK.
rm -rf "$HONEYDEW_ANDROID_BUILD_DIR/assets/content"
unzip -q "$HONEYDEW_ANDROID_BUILD_DIR/sounds.zip" 'content/*' -x '*.gz' -d "$HONEYDEW_ANDROID_BUILD_DIR/assets"
rm "$HONEYDEW_ANDROID_BUILD_DIR/sounds.zip"
cargo ndk -t arm64-v8a --platform 26 -o "$HONEYDEW_ANDROID_BUILD_DIR/jniLibs" rustc --release --locked --lib -- -C link-arg=-Wl,-z,max-page-size=16384
debug_keystore=$PWD/android/debug.keystore
if [ "$variant" = debug ] && [ ! -f "$debug_keystore" ]; then
    # Keep the existing signing identity when moving it out of disposable build output.
    if [ -f "$HONEYDEW_ANDROID_BUILD_DIR/debug.keystore" ]; then
        cp -p "$HONEYDEW_ANDROID_BUILD_DIR/debug.keystore" "$debug_keystore"
    else
        keytool -genkeypair -noprompt -keystore "$debug_keystore" \
            -storepass android -keypass android -alias androiddebugkey -keyalg RSA -keysize 2048 \
            -validity 10000 -dname 'CN=Android Debug,O=Android,C=US' >/dev/null 2>&1
    fi
fi
android/gradlew -p android --no-daemon "$task"

apk=$HONEYDEW_ANDROID_BUILD_DIR/outputs/apk/$variant/app-$variant.apk
"$sdk_tools/apksigner" verify "$apk"
"$sdk_tools/zipalign" -c -P 16 4 "$apk"
python3 - "$apk" <<'PY'
import base64
import hashlib
import json
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1]) as apk:
    assert apk.getinfo('lib/arm64-v8a/libhoneydew.so').compress_type == zipfile.ZIP_STORED
    assert apk.read('assets/content/CREDITS.txt'), 'missing sound credits'
    index = json.loads(apk.read('assets/content/index.json'))
    parts = {part['file']: part for entry in index['entries'] for part in entry['parts']}
    for part in parts.values():
        name = 'assets/content/' + part['file']
        assert apk.getinfo(name).compress_type == zipfile.ZIP_STORED, name
        data = apk.read(name)
        assert len(data) == part['bytes'], part['file']
        assert base64.b64encode(hashlib.sha256(data).digest()).decode() == part['sha256'], part['file']
    assert not any(name.endswith('.gz') for name in apk.namelist()), 'redundant web assets'
    print(f"Verified APK signature, alignment and {len(parts)} bundled sound packs")
PY
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
suffix=
[ "$variant" = release ] || suffix=-debug
out=$CARGO_TARGET_DIR/honeydew-android-$version-arm64$suffix.apk
cp "$apk" "$out"
echo "wrote $out"

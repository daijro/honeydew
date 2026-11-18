#!/bin/sh
# Build an Android APK for 64-bit and/or 32-bit ARM with optional bundled sounds.
# Release signing uses environment variables, never checked-in keys.
# --base leaves the sounds out (-base.apk): the app downloads them from the sound library's release (#66).
# --abi arm64-v8a|armeabi-v7a|all narrows native targets (arm64-v8a cuts native .so weight ~45% and halves compile time).
set -eu
cd "$(dirname "$0")/.."

variant=debug
task=assembleDebug
base=
abi_mode=all
profile=android-release
while [ "$#" -gt 0 ]; do
    case "$1" in
        --release) variant=release; task=assembleRelease ;;
        --base) base=-base ;;
        --abi)
            shift
            [ "$#" -gt 0 ] || { echo 'missing value for --abi (arm64-v8a|armeabi-v7a|all)' >&2; exit 2; }
            abi_mode=$1
            ;;
        --abi=*) abi_mode=${1#--abi=} ;;
        --profile=*) profile=${1#--profile=} ;;
        *)
            echo "usage: tools/android-dist.sh [--release] [--base] [--abi arm64-v8a|armeabi-v7a|all]" >&2
            exit 2
            ;;
    esac
    shift
done

case "$abi_mode" in
    all)
        abis_csv=arm64-v8a,armeabi-v7a
        abi_suffix=arm
        ;;
    arm64-v8a)
        abis_csv=arm64-v8a
        abi_suffix=arm64-v8a
        ;;
    armeabi-v7a)
        abis_csv=armeabi-v7a
        abi_suffix=armeabi-v7a
        ;;
    *)
        echo "unsupported --abi '$abi_mode' (expected arm64-v8a, armeabi-v7a, or all)" >&2
        exit 2
        ;;
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
export BANAFORGE_ANDROID_BUILD_DIR=$CARGO_TARGET_DIR/android
export BANAFORGE_ANDROID_ABIS=$abis_csv
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
mkdir -p "$BANAFORGE_ANDROID_BUILD_DIR/assets" "$BANAFORGE_ANDROID_BUILD_DIR/jniLibs"
rm -rf "$BANAFORGE_ANDROID_BUILD_DIR/assets/content" "$BANAFORGE_ANDROID_BUILD_DIR/jniLibs"
mkdir -p "$BANAFORGE_ANDROID_BUILD_DIR/jniLibs"
export BANAFORGE_ANDROID_BASE=$base
if [ -z "$base" ]; then
    cat sounds/honeydew-sounds.zip.[0-9][0-9][0-9] > "$BANAFORGE_ANDROID_BUILD_DIR/sounds.zip"
    # The .gz siblings are for web servers; Android reads the raw packs directly from its APK.
    unzip -q "$BANAFORGE_ANDROID_BUILD_DIR/sounds.zip" 'content/*' -x '*.gz' -d "$BANAFORGE_ANDROID_BUILD_DIR/assets"
    rm "$BANAFORGE_ANDROID_BUILD_DIR/sounds.zip"
fi

# signalsmith-stretch's C++ links libc++_shared.so, which the APK has to ship beside libbanaforge.so.
# Uses [profile.android-release] (ThinLTO + codegen-units=1 + strip="symbols") and 16 KB ELF page alignment.
case "$abi_mode" in
    all)
        cargo ndk -t arm64-v8a -t armeabi-v7a --platform 26 --link-libcxx-shared \
            -o "$BANAFORGE_ANDROID_BUILD_DIR/jniLibs" \
            rustc --profile "$profile" --locked --lib -- -C link-arg=-Wl,-z,max-page-size=16384
        ;;
    arm64-v8a|armeabi-v7a)
        cargo ndk -t "$abi_mode" --platform 26 --link-libcxx-shared \
            -o "$BANAFORGE_ANDROID_BUILD_DIR/jniLibs" \
            rustc --profile "$profile" --locked --lib -- -C link-arg=-Wl,-z,max-page-size=16384
        ;;
esac

debug_keystore=$PWD/android/debug.keystore
if [ "$variant" = debug ] && [ ! -f "$debug_keystore" ]; then
    # Keep the existing signing identity when moving it out of disposable build output.
    if [ -f "$BANAFORGE_ANDROID_BUILD_DIR/debug.keystore" ]; then
        cp -p "$BANAFORGE_ANDROID_BUILD_DIR/debug.keystore" "$debug_keystore"
    else
        keytool -genkeypair -noprompt -keystore "$debug_keystore" \
            -storepass android -keypass android -alias androiddebugkey -keyalg RSA -keysize 2048 \
            -validity 10000 -dname 'CN=Android Debug,O=Android,C=US' >/dev/null 2>&1
    fi
fi

# Packaged afresh: Gradle's incremental packaging would keep the space of the sounds a --base build drops.
rm -rf "$BANAFORGE_ANDROID_BUILD_DIR/outputs/apk/$variant"
android/gradlew -p android --no-daemon "$task"

apk=$BANAFORGE_ANDROID_BUILD_DIR/outputs/apk/$variant/app-$variant.apk
"$sdk_tools/apksigner" verify "$apk"
"$sdk_tools/zipalign" -c -P 16 4 "$apk"
python3 - "$apk" "$base" "$abis_csv" <<'PY'
import base64
import hashlib
import json
import sys
import zipfile

abis = [a.strip() for a in sys.argv[3].split(',') if a.strip()]
with zipfile.ZipFile(sys.argv[1]) as apk:
    for abi in abis:
        for lib in ('libbanaforge.so', 'libc++_shared.so'):
            assert apk.getinfo(f'lib/{abi}/{lib}').compress_type == zipfile.ZIP_STORED, abi + '/' + lib
    if sys.argv[2]:
        assert not any(name.startswith('assets/content/') for name in apk.namelist()), 'sounds in the base APK'
        print(f"Verified APK signature and 16 KB alignment ({', '.join(abis)}, base)")
        sys.exit()
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
    print(f"Verified APK signature, 16 KB alignment ({', '.join(abis)}) and {len(parts)} bundled sound packs")
PY
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
suffix=
[ "$variant" = release ] || suffix=-debug
out=$CARGO_TARGET_DIR/banaforge-android-$version-$abi_suffix$base$suffix.apk
cp "$apk" "$out"
echo "wrote $out"

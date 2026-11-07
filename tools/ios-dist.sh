#!/bin/sh
# The iOS app, built on macOS with Xcode's command line tools. By default two IPAs in the target folder:
# honeydew-ios-<version>.ipa with the bundled sounds (`content/` in the app, the packs Android's APK holds) and
# honeydew-ios-<version>-no-sounds.ipa without. Both are ad-hoc signed only, with the `Payload/Honeydew.app` layout
# that AltStore, SideStore and Sideloadly install, signing it with the user's own Apple ID.
#
#   tools/ios-dist.sh [NAME]   NAME: the IPAs' base name, honeydew-ios-<version> by default
#   tools/ios-dist.sh --sim    the Simulator's app, with sounds: <target folder>/ios-sim/Honeydew.app
set -eu
cd "$(dirname "$0")/.."

target=aarch64-apple-ios
platform=iPhoneOS
if [ "${1:-}" = --sim ]; then
    target=aarch64-apple-ios-sim
    platform=iPhoneSimulator
    # bindgen (signalsmith-stretch) hands clang Rust's triple, whose `sim` clang does not take.
    export BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_ios_sim=--target=arm64-apple-ios-simulator
fi
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
name=${1:-honeydew-ios-$version}
mkdir -p "${CARGO_TARGET_DIR:-target}"
out=$(cd "${CARGO_TARGET_DIR:-target}" && pwd)
export IPHONEOS_DEPLOYMENT_TARGET=14.0

# winit runs UIKit's application from the native binary's `main`, so the binary is the app's executable.
cargo build --release --locked --target "$target" --bin honeydew-native

work=$out/ios
rm -rf "$work"
app=$work/Payload/Honeydew.app
mkdir -p "$app"
cp "$out/$target/release/honeydew-native" "$app/Honeydew"
# The icon on its opaque background (iOS draws transparency black), at each size the plist names.
for icon in AppIcon60x60@2x:120 AppIcon60x60@3x:180 AppIcon76x76@2x~ipad:152 AppIcon83.5x83.5@2x~ipad:167; do
    sips -z "${icon#*:}" "${icon#*:}" assets/branding/honeydew-ios-512.png --out "$app/${icon%:*}.png" >/dev/null
done
cat > "$app/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleExecutable</key><string>Honeydew</string>
    <key>CFBundleIdentifier</key><string>dev.daijro.honeydew</string>
    <key>CFBundleName</key><string>Honeydew</string>
    <key>CFBundleDisplayName</key><string>Honeydew</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>CFBundleSupportedPlatforms</key><array><string>$platform</string></array>
    <key>CFBundleIcons</key><dict><key>CFBundlePrimaryIcon</key><dict>
        <key>CFBundleIconFiles</key><array><string>AppIcon60x60</string></array>
    </dict></dict>
    <key>CFBundleIcons~ipad</key><dict><key>CFBundlePrimaryIcon</key><dict>
        <key>CFBundleIconFiles</key><array><string>AppIcon60x60</string><string>AppIcon76x76</string><string>AppIcon83.5x83.5</string></array>
    </dict></dict>
    <key>MinimumOSVersion</key><string>$IPHONEOS_DEPLOYMENT_TARGET</string>
    <key>LSRequiresIPhoneOS</key><true/>
    <key>UIDeviceFamily</key><array><integer>1</integer><integer>2</integer></array>
    <key>UIRequiredDeviceCapabilities</key><array><string>arm64</string><string>metal</string></array>
    <key>UILaunchScreen</key><dict/>
    <key>UIApplicationSceneManifest</key><dict>
        <key>UIApplicationSupportsMultipleScenes</key><false/>
        <key>UISceneConfigurations</key><dict><key>UIWindowSceneSessionRoleApplication</key><array><dict>
            <key>UISceneConfigurationName</key><string>Honeydew</string>
            <key>UISceneDelegateClassName</key><string>WinitSceneDelegate</string>
        </dict></array></dict>
    </dict>
    <key>UIStatusBarHidden</key><true/>
    <key>UIViewControllerBasedStatusBarAppearance</key><false/>
    <key>UISupportedInterfaceOrientations</key><array>
        <string>UIInterfaceOrientationPortrait</string>
        <string>UIInterfaceOrientationLandscapeLeft</string>
        <string>UIInterfaceOrientationLandscapeRight</string>
    </array>
    <key>UISupportedInterfaceOrientations~ipad</key><array>
        <string>UIInterfaceOrientationPortrait</string>
        <string>UIInterfaceOrientationPortraitUpsideDown</string>
        <string>UIInterfaceOrientationLandscapeLeft</string>
        <string>UIInterfaceOrientationLandscapeRight</string>
    </array>
    <key>UIFileSharingEnabled</key><true/>
    <key>LSSupportsOpeningDocumentsInPlace</key><true/>
    <key>NSMicrophoneUsageDescription</key><string>Honeydew uses the microphone to record audio into your songs.</string>
</dict>
</plist>
PLIST
plutil -lint "$app/Info.plist" >/dev/null
printf 'APPL????' > "$app/PkgInfo"

# The sounds as Android bundles them: the packs, without the web's gzipped copies.
add_sounds() {
    cat sounds/honeydew-sounds.zip.[0-9][0-9][0-9] > "$work/sounds.zip"
    unzip -q "$work/sounds.zip" 'content/*' -x '*.gz' -d "$app"
    rm "$work/sounds.zip"
    test -s "$app/content/index.json"
}
sign() {
    if [ "$platform" = iPhoneSimulator ]; then
        codesign --force --sign - --timestamp=none "$app"
    else
        # The final device signer must preserve this entitlement and use a profile
        # whose App ID enables Inter-App Audio; AUv3 discovery needs it as well.
        codesign --force --sign - --timestamp=none --entitlements tools/ios.entitlements "$app"
    fi
}
if [ "$platform" = iPhoneSimulator ]; then
    add_sounds
    sign
    rm -rf "$out/ios-sim"
    mv "$work/Payload" "$out/ios-sim"
    echo "wrote $out/ios-sim/Honeydew.app"
    exit 0
fi
ipa() {
    sign
    rm -f "$out/$1.ipa"
    (cd "$work" && zip -qry "$out/$1.ipa" Payload)
    echo "wrote $out/$1.ipa"
}
ipa "$name-no-sounds"
add_sounds
ipa "$name"

#!/bin/sh
# iPad Simulator: verify the actual AUv3 wrapper's toolbar, layout and dismissal.
set -eu
cd "$(dirname "$0")/.."
out=${CARGO_TARGET_DIR:-target}/ios-auv3-editor-test
app=$out/AUv3EditorTest.app
mkdir -p "$app"
sdk=$(xcrun --sdk iphonesimulator --show-sdk-path)
xcrun --sdk iphonesimulator clang -target arm64-apple-ios14.0-simulator -isysroot "$sdk" \
    -fobjc-arc -fblocks -Wall -Wextra -Werror -Wno-unused-parameter tools/ios-auv3-editor-test.m \
    -framework UIKit -framework CoreGraphics -framework Foundation -framework CoreAudioKit -framework AVFoundation -framework AudioToolbox \
    -o "$app/AUv3EditorTest"
python3 - "$app/Info.plist" <<'PY'
import plistlib, sys
with open(sys.argv[1], 'wb') as f:
    plistlib.dump(dict(CFBundleIdentifier='dev.honeydew.auv3-editor-test', CFBundleExecutable='AUv3EditorTest',
        CFBundleName='AUv3EditorTest', CFBundlePackageType='APPL', MinimumOSVersion='14.0',
        UIDeviceFamily=[1,2], UILaunchScreen={},
        UIApplicationSceneManifest=dict(UIApplicationSupportsMultipleScenes=False, UISceneConfigurations={
            'UIWindowSceneSessionRoleApplication':[dict(UISceneConfigurationName='Default', UISceneDelegateClassName='HDEditorTestSceneDelegate')]}),
        UISupportedInterfaceOrientations=['UIInterfaceOrientationPortrait','UIInterfaceOrientationLandscapeLeft','UIInterfaceOrientationLandscapeRight']), f)
PY
codesign --force --sign - "$app" >/dev/null 2>&1
device=${HONEYDEW_IOS_TEST_DEVICE:-$(xcrun simctl list devices available -j | python3 -c '
import json,sys
print(next(d["udid"] for runtime,ds in json.load(sys.stdin)["devices"].items() if "iOS" in runtime for d in ds if d["name"].startswith("iPad")))')}
xcrun simctl boot "$device" 2>/dev/null || true
xcrun simctl bootstatus "$device" -b >/dev/null
xcrun simctl install "$device" "$app"
trap 'xcrun simctl uninstall "$device" dev.honeydew.auv3-editor-test >/dev/null 2>&1 || true' EXIT
xcrun simctl launch --terminate-running-process --console "$device" dev.honeydew.auv3-editor-test > "$out/test.log" 2>&1
cat "$out/test.log"
grep -q 'AUv3 iPad return toolbar/layout test passed' "$out/test.log"

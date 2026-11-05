#!/bin/sh
# The app's iOS self-test (`src/app/selftest.rs`) in the Simulator, on macOS with Xcode: boots an iPhone, installs APP,
# runs `--selftest` with the microphone allowed, then `--selftest-mic-denied` with it denied. Each step the app prints
# gets a screenshot, and the logs, in OUT. Fails on `SELFTEST FAIL`, or when a run gives no verdict in time.
#
#   tools/ios-selftest.sh APP OUT
set -eu
app=$1
mkdir -p "$2"
out=$(cd "$2" && pwd)
bundle=dev.daijro.honeydew

# The newest iPhone the installed runtimes offer.
device=$(xcrun simctl list devices available -j | python3 -c '
import json, sys
devices = [d for runtime, ds in json.load(sys.stdin)["devices"].items() if "iOS" in runtime for d in ds]
print([d["udid"] for d in devices if d["name"].startswith("iPhone")][-1])')
xcrun simctl boot "$device" 2>/dev/null || true
xcrun simctl bootstatus "$device" -b >/dev/null
xcrun simctl install "$device" "$app"

# In the app's container: its log of its steps (the Simulator hands the app's own output on late), and how many of them
# have their screenshot, which the app waits for before it goes on.
caches=$(xcrun simctl get_app_container "$device" "$bundle" data)/Library/Caches
steps=$caches/selftest.log
shot=$caches/selftest.shot

# One run of the app with `--$1`, a screenshot named after each step it logs.
run() {
    rm -f "$steps" "$shot"
    xcrun simctl launch --terminate-running-process --stdout="$out/$1.out" --stderr="$out/$1.err" \
        "$device" "$bundle" "--$1"
    start=$(date +%s)
    shots=0
    while :; do
        log=$(cat "$steps" 2>/dev/null || true)
        count=$(printf '%s\n' "$log" | grep -c '^SELFTEST STEP' || true)
        while [ "$shots" -lt "$count" ]; do
            shots=$((shots + 1))
            step=$(printf '%s\n' "$log" | grep '^SELFTEST STEP' | sed -n "${shots}p" | cut -d' ' -f3)
            # The step drawn.
            sleep 1
            xcrun simctl io "$device" screenshot "$out/$1-$shots-$step.png" >/dev/null 2>&1
            echo "$shots" > "$shot"
        done
        verdict=$(printf '%s\n' "$log" | grep -E '^SELFTEST (PASS|FAIL)' || true)
        if [ -n "$verdict" ]; then
            case $verdict in *FAIL*) xcrun simctl io "$device" screenshot "$out/$1-fail.png" >/dev/null 2>&1 ;; esac
            printf '%s\n' "$log" > "$out/$1.log"
            printf '%s\n' "$log"
            case $verdict in *PASS*) return 0 ;; esac
            sleep 3
            tail -n 40 "$out/$1.err"
            return 1
        fi
        if [ $(($(date +%s) - start)) -gt 900 ]; then
            echo "SELFTEST FAIL: $1 gave no verdict within 900 s"
            xcrun simctl io "$device" screenshot "$out/$1-timeout.png" >/dev/null 2>&1 || true
            printf '%s\n' "$log" > "$out/$1.log"
            tail -n 40 "$out/$1.err"
            return 1
        fi
        sleep 0.5
    done
}

status=0
xcrun simctl privacy "$device" grant microphone "$bundle"
run selftest || status=1
xcrun simctl privacy "$device" revoke microphone "$bundle"
run selftest-mic-denied || status=1
exit $status

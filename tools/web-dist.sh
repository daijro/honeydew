#!/bin/sh
# The web release (README "Web"): the whole site in dist/.
#   tools/web-dist.sh          trunk build --release, wasm-opt -O3, a gzip copy (`<file>.gz`) beside the app's wasm, JS
#                              and HTML, then honeydew-sounds.zip.* unpacked into content/, then --check
#   tools/web-dist.sh --zip    the same, then the release zip: target/honeydew-web-<version>.zip (or under
#                              $CARGO_TARGET_DIR), dist/ in one honeydew-web-<version>/ folder
#   tools/web-dist.sh --check  no build: prints the size table, and fails when a budget is over, an app file's .gz is
#                              missing or stale, a pack part is over 24,000,000 bytes or any file over 25 MiB
# Needs trunk, wasm-opt (binaryen; or set $WASM_OPT) and unzip. The tracked sound archive parts contain the packs,
# their .gz files and the index: this script neither builds nor recompresses the sounds.
set -eu
cd "$(dirname "$0")/.."

# Budgets in bytes: provisional, until measured releases ratchet them.
WASM_RAW=16000000 WASM_GZ=4500000 JS_GZ=60000 INDEX_RAW=256000 INDEX_GZ=48000 FIRST_LOAD=5000000
PART_MAX=24000000 # one pack part, raw and gzipped
FILE_MAX=26214400 # 25 MiB, Cloudflare's limit per file
# The binaryen release this was measured with; another one works, with a note.
BINARYEN=version_133
# Rust's default wasm32 features, plus SIMD. wasm-opt reads them from the module's target_features section, which
# --strip-debug and some builds drop; without them it assumes the 2017 baseline and rejects e.g. memory.copy.
FEATURES="--enable-bulk-memory --enable-bulk-memory-opt --enable-call-indirect-overlong --enable-multivalue
  --enable-mutable-globals --enable-nontrapping-float-to-int --enable-reference-types --enable-sign-ext --enable-simd"

# The app's own files that go out compressed. Trunk's names have no spaces.
app_files() {
    find dist -path dist/content -prune -o -type f \( -name '*.wasm' -o -name '*.js' -o -name '*.html' \) -print
}
size() { wc -c <"$1" | tr -d ' '; }

build() {
    wasm_opt=${WASM_OPT:-wasm-opt}
    if ! command -v "$wasm_opt" >/dev/null; then
        echo "needs wasm-opt (binaryen $BINARYEN) on PATH, or WASM_OPT" >&2
        exit 1
    fi
    "$wasm_opt" --version | grep -q "($BINARYEN)" || echo "note: $("$wasm_opt" --version), measured with $BINARYEN" >&2
    trunk build --release
    for f in dist/*.wasm; do
        # shellcheck disable=SC2086 # one flag per word
        "$wasm_opt" -O3 --strip-debug $FEATURES "$f" -o "$f"
    done
    # -n: no name or time inside, so the same file always gives the same bytes; -k keeps the file for clients and
    # hosts without gzip.
    app_files | xargs -r gzip -9 -k -n -f
    cp deploy/_headers dist/
    mkdir -p target
    cat honeydew-sounds.zip.[0-9][0-9][0-9] > target/honeydew-sounds.zip
    unzip -q target/honeydew-sounds.zip -d dist
}

fail=0
over() { # what, bytes, budget
    if [ "$2" -gt "$3" ]; then
        echo "OVER: $1 is $2 bytes, budget $3"
        fail=1
    fi
}

check() {
    set -- dist/*_bg.wasm
    if [ $# -ne 1 ] || [ ! -f "$1" ]; then
        echo "dist/ has no single *_bg.wasm: run tools/web-dist.sh first"
        exit 1
    fi
    wasm=$1 js=${1%_bg.wasm}.js
    printf '%-48s %11s %11s\n' file raw gzip
    for f in $(app_files | sort); do
        if [ ! -f "$f.gz" ] || [ "$f" -nt "$f.gz" ]; then
            echo "MISSING or stale: $f.gz"
            fail=1
        else
            printf '%-48s %11s %11s\n' "$f" "$(size "$f")" "$(size "$f.gz")"
        fi
    done
    [ $fail = 0 ] || return 0

    over "$wasm" "$(size "$wasm")" $WASM_RAW
    over "$wasm.gz" "$(size "$wasm.gz")" $WASM_GZ
    over "$js.gz" "$(size "$js.gz")" $JS_GZ
    first=$(($(size "$wasm.gz") + $(size "$js.gz") + $(size dist/index.html.gz)))
    index=dist/content/index.json
    if [ -f $index ]; then
        # Hosts send it compressed; this is about what they send.
        index_gz=$(gzip -9 -n -c $index | wc -c | tr -d ' ')
        printf '%-48s %11s %11s\n' $index "$(size $index)" "$index_gz"
        over $index "$(size $index)" $INDEX_RAW
        over "$index, gzipped" "$index_gz" $INDEX_GZ
        first=$((first + index_gz))
    fi
    printf '%-48s %23s\n' "first load (wasm, JS, HTML, index; gzipped)" $first
    over "the first load" $first $FIRST_LOAD
    packs=0 packs_bytes=0
    for f in $(find dist -type f); do
        case $f in
            dist/content/p/*)
                packs=$((packs + 1)) packs_bytes=$((packs_bytes + $(size "$f")))
                over "$f" "$(size "$f")" $PART_MAX
                ;;
            *) over "$f" "$(size "$f")" $FILE_MAX ;;
        esac
    done
    if [ $packs -gt 0 ]; then printf '%-48s %23s\n' "dist/content/p/ ($packs files)" $packs_bytes; fi
}

release_zip() {
    v=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
    out=${CARGO_TARGET_DIR:-target}/honeydew-web-$v
    rm -rf "$out" "$out.zip"
    cp -R dist "$out"
    (cd "$out/.." && zip -qr9 "honeydew-web-$v.zip" "honeydew-web-$v")
    rm -rf "$out"
    echo "wrote $out.zip"
}

case ${1:-} in
    "")
        build
        check
        ;;
    --zip)
        build
        check
        if [ $fail = 0 ]; then release_zip; fi
        ;;
    --check) check ;;
    *)
        sed -n '2,10p' "$0"
        exit 2
        ;;
esac
exit $fail

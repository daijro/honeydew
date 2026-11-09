#!/usr/bin/env bash
# Updates Lemon Studio's plugins vendored in vendor/lemon-studio (those vendor/lemon-studio/plugins.txt lists, the
# plugins and crates they use and the parts of os-engine they need; vendor/lemon-studio/README.md) to Lemon Studio at
# `commit` (default: upstream's main): replaces them with upstream's files, embeds each plugin's editor (embed_editor),
# applies Honeydew's changes (vendor/lemon-studio/honeydew.patch), prints the diff and pins the commit in vendor/lemon-studio/README.md. The
# manifests are Honeydew's own: the script shows upstream's changes to them (a new one's whole manifest) for you to
# carry over. Commit the result yourself after building and testing (see that README).
#
#   tools/sync-lemon-plugins.sh [commit]
#   tools/sync-lemon-plugins.sh --save-patch      writes honeydew.patch: the vendored files against the pinned commit
#   tools/sync-lemon-plugins.sh --paths [commit]  prints the upstream paths they come from (the lemon-effects workflow)
#
# LEMON_REPO: a local clone of Lemon Studio to read from instead of fetching (it must hold the commit).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
url=https://github.com/ItsAlphaNeon/Lemon-Studio
vendor=vendor/lemon-studio
readme=$vendor/README.md
patch=$vendor/honeydew.patch

pinned=$(sed -nE 's/^- Plugins commit: `([0-9a-f]{40})`.*/\1/p' "$readme")
mode=sync
case ${1:-} in
--save-patch) mode=save && set -- "$pinned" ;;
--paths) mode=paths && shift ;;
esac
want=${1:-main}

if [ -n "${LEMON_REPO:-}" ]; then
    repo=$LEMON_REPO
    if git -C "$repo" fetch -q "$url" "$want" 2>/dev/null; then
        commit=$(git -C "$repo" rev-parse FETCH_HEAD)
    else
        commit=$(git -C "$repo" rev-parse --verify "$want^{commit}")
    fi
    tmp=$(mktemp -d)
else
    tmp=$(mktemp -d)
    repo=$tmp/lemon
    git clone -q --filter=blob:none --no-checkout "$url" "$repo"
    git -C "$repo" fetch -q origin "$want"
    commit=$(git -C "$repo" rev-parse FETCH_HEAD)
fi
trap 'rm -rf "$tmp"' EXIT

# The listed plugins and the os-engine files they use.
plugins=() engine=()
while read -r line; do
    case $line in
    '' | '#'*) ;;
    os-engine/src/*) engine+=("${line#os-engine/src/}") ;;
    *) plugins+=("plugins/$line") ;;
    esac
done <"$vendor/plugins.txt"
for d in "${plugins[@]}"; do
    git -C "$repo" cat-file -e "$commit:$d/Cargo.toml" || { echo "no $d at $commit ($vendor/plugins.txt)" >&2 && exit 1; }
done

# Every upstream plugin and crate vendored for them: the listed plugins and the plugins and crates their manifests'
# dependencies name at `commit`, transitively. os-engine (Honeydew's subset, its files listed) and the hosts (synced by
# hand, README.md) are not followed.
sources=() todo=("${plugins[@]}")
while [ ${#todo[@]} -gt 0 ]; do
    d=${todo[0]} && todo=("${todo[@]:1}")
    case " ${sources[*]} " in *" $d "*) continue ;; esac
    sources+=("$d")
    for name in $(git -C "$repo" show "$commit:$d/Cargo.toml" |
        awk '/^\[/ { on = /^\[(target\..+\.)?dependencies\]$/ } on && /^[A-Za-z0-9_-]+ *[.=]/ { sub(/[ .=].*/, ""); print }'); do
        case $name in os-engine | os-host | os-clap | os-vst3 | os-vst2) continue ;; esac
        for p in "plugins/$name" "crates/$name"; do
            if git -C "$repo" cat-file -e "$commit:$p/Cargo.toml" 2>/dev/null; then todo+=("$p"); fi
        done
    done
done
if [ $mode = paths ]; then
    printf '%s\n' "${sources[@]}" "${engine[@]/#/crates/os-engine/src/}"
    exit 0
fi
# upstream path -> vendored path, each copied whole: every folder of those plugins and crates but their examples, tests
# and benches (their manifests are Honeydew's).
dirs=()
for d in "${sources[@]}"; do
    for f in $(git -C "$repo" ls-tree -d --name-only "$commit" "$d/"); do
        case ${f##*/} in examples | tests | benches) ;; *) dirs+=("$f:${f#*/}") ;; esac
    done
done

# Honeydew's one change to a plugin's editor ($1), made here rather than in honeydew.patch so that a plugin needs no
# hunk of its own (README.md, "Adding a plugin"). Upstream's editors share one template; built in (`embed`), its Linux
# and macOS editor runs on every platform in Honeydew's window (`os_plugkit::embed`):
# - every `#[cfg(any(target_os = "linux", target_os = "macos"))]` gains `feature = "embed"` (`mod gui`, its waker);
# - the X11 window's arm takes Honeydew's window everywhere but macOS (nih-plug gives it as an X11 window id);
# - after the input loop the editor parks (`os_plugkit::park!`), having no thread of its own;
# - its clock is `os_plugkit::time` (a browser build's `std::time` has none);
# - a knob's "Create automation clip" is offered without the editor joining the host's strip (#113): Honeydew answers it
#   (`os_plugkit::embed::take_automate`).
# An editor off the template gets what matched and a warning: make the rest by hand, then save the patch.
embed_editor() {
    if ! awk '
    held != "" {
        if ($0 ~ /ParentWindowHandle::X11Window/) {
            sub(/target_os = "linux"/, "any(target_os = \"linux\", all(feature = \"embed\", not(target_os = \"macos\")))", held)
            x11++
        }
        print held
        held = ""
    }
    /^ *#\[cfg\(target_os = "linux"\)\]$/ { held = $0; next }
    /^ *#\[cfg\(any\(target_os = "linux", target_os = "macos"\)\)\]$/ { sub(/"macos"\)\)/, "\"macos\", feature = \"embed\"))"); gui++ }
    /^use std::time::/ { sub(/^use std::time::/, "use os_plugkit::time::") }
    /MenuItem::item\("Create automation clip"\)\.enabled\(self\.hosted\)/ { sub(/\.enabled\(self\.hosted\)/, ".enabled(self.hosted || cfg!(feature = \"embed\"))") }
    /^ *while let Some\(input\) = self\.win\.next\(\) \{$/ { match($0, /^ */); indent = substr($0, 1, RLENGTH); end = indent "}" }
    { print }
    end != "" && $0 == end { print indent "os_plugkit::park!(self);"; end = ""; park++ }
    END { if (held != "") print held; exit !(gui && x11 == 1 && park == 1) }
    ' "$1" >"$1.embed"; then
        echo "warning: ${1#*/$vendor/} is off the editor template Honeydew embeds (vendor/lemon-studio/README.md):" >&2
        echo "  make the rest of its change by hand, then run tools/sync-lemon-plugins.sh --save-patch" >&2
    fi
    mv "$1.embed" "$1"
}

# Upstream's files at `commit` laid out as the vendored tree, under $1, with each plugin's editor embedded.
upstream() {
    local out=$1/$vendor pair d
    rm -rf "$tmp/x"
    mkdir -p "$tmp/x"
    for pair in "${dirs[@]}"; do
        mkdir -p "$out/${pair#*:}"
        git -C "$repo" archive "$commit" "${pair%%:*}" | tar -x -C "$tmp/x"
        cp -r "$tmp/x/${pair%%:*}/." "$out/${pair#*:}/"
    done
    mkdir -p "$out/os-engine/src"
    for f in "${engine[@]}"; do
        git -C "$repo" show "$commit:crates/os-engine/src/$f" >"$out/os-engine/src/$f"
    done
    for d in "${sources[@]}"; do
        case $d in plugins/*) if [ -f "$out/${d#*/}/src/editor.rs" ]; then embed_editor "$out/${d#*/}/src/editor.rs"; fi ;; esac
    done
}

# The vendored tree's upstream parts, under $1.
vendored() {
    local out=$1/$vendor pair
    for pair in "${dirs[@]}"; do
        mkdir -p "$out/${pair#*:}"
        cp -r "$vendor/${pair#*:}/." "$out/${pair#*:}/"
    done
    mkdir -p "$out/os-engine/src"
    for f in "${engine[@]}"; do
        cp "$vendor/os-engine/src/$f" "$out/os-engine/src/"
    done
}

if [ $mode = save ]; then
    upstream "$tmp/a"
    vendored "$tmp/b"
    # Without diff's file times, so that a patch saved again changes only where the files did.
    (cd "$tmp" && diff -ruN a b || true) | sed -E 's/^((---|\+\+\+) [ab]\/[^\t]*)\t[0-9]{4}-[0-9]{2}-[0-9]{2} .*$/\1/' >"$patch"
    echo "wrote $patch: the vendored files against Lemon Studio $commit"
    exit 0
fi

echo "Lemon Studio plugins: ${pinned:-none} -> $commit"
for pair in "${dirs[@]}"; do
    rm -rf "${vendor:?}/${pair#*:}"
done
upstream "$tmp/u"
cp -r "$tmp/u/$vendor/." "$vendor/"
if ! git apply --whitespace=nowarn "$patch"; then
    echo "honeydew.patch does not apply to $commit: port Honeydew's changes by hand (vendor/lemon-studio/README.md)," >&2
    echo "then run tools/sync-lemon-plugins.sh --save-patch" >&2
    exit 1
fi
sed -i -E "s/^(- Plugins commit: \`)[0-9a-f]{40}(\`.*)$/\1$commit\2/" "$readme"

git add -N "$vendor"
git --no-pager diff --stat -- "$vendor"
if [ -n "$pinned" ] && git -C "$repo" cat-file -e "$pinned^{commit}" 2>/dev/null; then
    echo "Upstream's manifest changes since $pinned (Honeydew's manifests are its own; carry over what applies):"
    git -C "$repo" --no-pager diff "$pinned" "$commit" -- "${sources[@]/%//Cargo.toml}" crates/os-engine/Cargo.toml
    git -C "$repo" --no-pager diff "$pinned" "$commit" -- Cargo.lock | grep -E '^[-+]source = .*nih-plug' | sort -u |
        sed 's/^/nih-plug (vendor\/nih-plug): /' || true
fi
for d in "${sources[@]}"; do
    if [ ! -f "$vendor/${d#*/}/Cargo.toml" ]; then
        echo "New: $vendor/${d#*/} needs Honeydew's manifest (and a path in the root Cargo.toml); upstream's $d/Cargo.toml:"
        git -C "$repo" --no-pager show "$commit:$d/Cargo.toml"
    fi
done

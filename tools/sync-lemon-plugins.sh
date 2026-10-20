#!/usr/bin/env bash
# Updates Lemon Studio's plugins vendored in vendor/lemon-studio (Lemon Trio, Lemon Sampler, Lemon Parametric EQ, the
# crates they use and the parts of os-engine they need; vendor/lemon-studio/README.md) to Lemon Studio at `commit`
# (default: upstream's main): replaces them with upstream's files, applies Honeydew's changes
# (vendor/lemon-studio/honeydew.patch), prints the diff and pins the commit in vendor/lemon-studio/README.md. The
# manifests are Honeydew's own: the script shows upstream's changes to them for you to carry over. Commit the result
# yourself after building and testing (see that README).
#
#   tools/sync-lemon-plugins.sh [commit]
#   tools/sync-lemon-plugins.sh --save-patch    writes honeydew.patch: the vendored files against the pinned commit
#
# LEMON_REPO: a local clone of Lemon Studio to read from instead of fetching (it must hold the commit).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
url=https://github.com/ItsAlphaNeon/Lemon-Studio
vendor=vendor/lemon-studio
readme=$vendor/README.md
patch=$vendor/honeydew.patch
# upstream path -> vendored path, each copied whole.
dirs=(
    plugins/lemon-3x-osc/src:lemon-3x-osc/src
    plugins/lemon-sampler/src:lemon-sampler/src
    plugins/lemon-parametric-eq-2/src:lemon-parametric-eq-2/src
    crates/os-plugkit/src:os-plugkit/src
    crates/os-ui/src:os-ui/src
    crates/os-ui/fonts:os-ui/fonts
    crates/os-ui/icons:os-ui/icons
    crates/os-theme/src:os-theme/src
    crates/os-theme/themes:os-theme/themes
    crates/os-hostlink/src:os-hostlink/src
    crates/os-model/src:os-model/src
    crates/os-dsp/src:os-dsp/src
    crates/os-paths/src:os-paths/src
)
engine=(clock envelope laws sample)
manifests=(plugins/lemon-3x-osc plugins/lemon-sampler plugins/lemon-parametric-eq-2 crates/os-plugkit crates/os-ui
    crates/os-theme crates/os-hostlink crates/os-model crates/os-dsp crates/os-paths crates/os-engine)

pinned=$(sed -nE 's/^- Plugins commit: `([0-9a-f]{40})`.*/\1/p' "$readme")
save=false
want=${1:-main}
if [ "$want" = --save-patch ]; then
    save=true
    want=$pinned
fi

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

# Upstream's files at `commit` laid out as the vendored tree, under $1.
upstream() {
    local out=$1/$vendor pair
    rm -rf "$tmp/x"
    mkdir -p "$tmp/x"
    for pair in "${dirs[@]}"; do
        mkdir -p "$out/${pair#*:}"
        git -C "$repo" archive "$commit" "${pair%%:*}" | tar -x -C "$tmp/x"
        cp -r "$tmp/x/${pair%%:*}/." "$out/${pair#*:}/"
    done
    mkdir -p "$out/os-engine/src"
    for f in "${engine[@]}"; do
        git -C "$repo" show "$commit:crates/os-engine/src/$f.rs" >"$out/os-engine/src/$f.rs"
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
        cp "$vendor/os-engine/src/$f.rs" "$out/os-engine/src/"
    done
}

if $save; then
    upstream "$tmp/a"
    vendored "$tmp/b"
    (cd "$tmp" && diff -ruN a b || true) >"$patch"
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
    git -C "$repo" --no-pager diff "$pinned" "$commit" -- "${manifests[@]/%//Cargo.toml}" Cargo.lock | grep -E '^[-+]|^diff' |
        grep -vE '^(\+\+\+|---) ' | grep -E 'Cargo.toml|nih|^diff' || true
fi

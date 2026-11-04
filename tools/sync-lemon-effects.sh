#!/usr/bin/env bash
# Updates the vendored Lemon Studio effects (vendor/lemon-studio/os-engine/src/effects, upstream's files unmodified) to
# Lemon Studio at `commit` (default: upstream's main): copies every file of upstream's crates/os-engine/src/effects
# (subfolders too), removes those upstream removed, copies the engine files they use (`uses`), prints the diff and pins
# the commit in vendor/lemon-studio/README.md. Commit the result yourself after building and testing (see that README).
#
#   tools/sync-lemon-effects.sh [commit]
#
# LEMON_REPO: a local clone of Lemon Studio to read from instead of fetching (it must hold the commit).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
url=https://github.com/ItsAlphaNeon/Lemon-Studio
src=crates/os-engine/src/effects
dst=vendor/lemon-studio/os-engine/src/effects
# The files of crates/os-engine/src the effects use, each upstream's too (Lemon Formula Controller's formula language).
uses=(formula.rs)
readme=vendor/lemon-studio/README.md
want=${1:-main}

if [ -n "${LEMON_REPO:-}" ]; then
    repo=$LEMON_REPO
    # Upstream's `want` when it can be fetched, else the clone's own (an abbreviated commit, no network).
    if git -C "$repo" fetch -q "$url" "$want" 2>/dev/null; then
        commit=$(git -C "$repo" rev-parse FETCH_HEAD)
    else
        commit=$(git -C "$repo" rev-parse --verify "$want^{commit}")
    fi
else
    repo=$(mktemp -d)
    trap 'rm -rf "$repo"' EXIT
    # Commits and trees only; `git show` fetches the few blobs read.
    git clone -q --filter=blob:none --no-checkout "$url" "$repo"
    git -C "$repo" fetch -q origin "$want"
    commit=$(git -C "$repo" rev-parse FETCH_HEAD)
fi
pinned=$(sed -nE 's/^- Effects commit: `([0-9a-f]{40})`.*/\1/p' "$readme")
echo "Lemon Studio effects: ${pinned:-none} -> $commit"

files=$(git -C "$repo" ls-tree -r --name-only "$commit" "$src/")
[ -n "$files" ] || { echo "no $src at $commit" >&2; exit 1; }
# Every file is upstream's, subfolders included: the folder is replaced whole.
rm -rf "$dst"
for f in $files; do
    out=$dst/${f#"$src"/}
    mkdir -p "$(dirname "$out")"
    git -C "$repo" show "$commit:$f" >"$out"
done
for f in "${uses[@]}"; do
    git -C "$repo" show "$commit:crates/os-engine/src/$f" >"vendor/lemon-studio/os-engine/src/$f"
done
used=("${uses[@]/#/vendor/lemon-studio/os-engine/src/}")
sed -i -E "s/^(- Effects commit: \`)[0-9a-f]{40}(\`.*)$/\1$commit\2/" "$readme"

git add -N "$dst" "${used[@]}"
git --no-pager diff --stat -- "$dst" "${used[@]}" "$readme"
git --no-pager diff -- "$dst" "${used[@]}"

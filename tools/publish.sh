#!/usr/bin/env bash
# Builds the public release branch from HEAD without touching the working tree: HEAD's tree minus docs/ (gitignored
# there), as one new commit. The first run makes `public` an orphan with no history; later runs add one commit on top.
# Push it yourself: git push <remote> public:main
#
#   tools/publish.sh [message]
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
branch=${PUBLIC_BRANCH:-public}
msg=${1:-"Honeydew $(git describe --tags --always HEAD)"}
[ -z "$(git status --porcelain)" ] || echo "note: uncommitted changes are not published (HEAD only)" >&2

index=$(mktemp)
trap 'rm -f "$index"' EXIT
export GIT_INDEX_FILE=$index
git read-tree HEAD
git rm -r -q --cached --ignore-unmatch docs
ignore=$( (git show HEAD:.gitignore 2>/dev/null; echo "/docs/") | git hash-object -w --stdin)
git update-index --add --cacheinfo "100644,$ignore,.gitignore"
tree=$(git write-tree)
unset GIT_INDEX_FILE

if git rev-parse -q --verify refs/heads/$branch >/dev/null; then
    [ "$(git rev-parse "$branch^{tree}")" != "$tree" ] || { echo "$branch is already up to date"; exit 0; }
    commit=$(git commit-tree "$tree" -p "$branch" -m "$msg")
else
    commit=$(git commit-tree "$tree" -m "$msg")
fi
git update-ref refs/heads/$branch "$commit"
echo "$branch -> $(git log --oneline -1 "$branch")"
echo "files: $(git ls-tree -r --name-only "$branch" | wc -l); docs/: $(git ls-tree -r --name-only "$branch" | grep -c '^docs/' || true)"

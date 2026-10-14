#!/usr/bin/env bash
# Installs Honeydew's desktop entry and icons for the current user, into ${XDG_DATA_HOME:-~/.local/share}
# (applications/ and icons/hicolor/), so the taskbar shows its logo: on Wayland the compositor ignores the window icon
# and takes the Icon= of the entry that matches the window's app_id, `honeydew`.
#
#   tools/install-desktop-entry.sh [BINARY]   Exec= runs BINARY (default: <repo>/target/release/honeydew-native)
#   tools/install-desktop-entry.sh --uninstall
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="$repo/assets/linux"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
apps="$data/applications"
icons="$data/icons/hicolor"

refresh() {
    if command -v update-desktop-database >/dev/null && [ -d "$apps" ]; then
        update-desktop-database "$apps"
    fi
    if command -v gtk-update-icon-cache >/dev/null && [ -d "$icons" ]; then
        gtk-update-icon-cache -f -t "$icons"
    fi
}

# The icons' paths under hicolor/, e.g. `48x48/apps/honeydew.png`.
icon_files=("$src"/icons/hicolor/*/apps/honeydew.*)
icon_files=("${icon_files[@]#"$src/icons/hicolor/"}")

case "${1:-}" in
--uninstall)
    rm -f "$apps/honeydew.desktop"
    for f in "${icon_files[@]}"; do rm -f "$icons/$f"; done
    refresh
    echo "Uninstalled the Honeydew desktop entry and icons from $data"
    exit 0
    ;;
-*)
    echo "usage: $0 [BINARY] | --uninstall" >&2
    exit 2
    ;;
esac

bin="${1:-$repo/target/release/honeydew-native}"
if [ ! -f "$bin" ] || [ ! -x "$bin" ]; then
    echo "error: no executable at $bin. Build it first (cargo build --release) or pass its path." >&2
    exit 1
fi
bin="$(realpath -s -- "$bin")"
# These would need escaping in Exec= (desktop entry spec); a build path has none of them.
case "$bin" in
*[\"\`\$\\%]* | *$'\n'*)
    echo "error: the path $bin has a character Exec= cannot take (\" \` \$ \\ % or a newline)." >&2
    exit 1
    ;;
esac

mkdir -p "$apps"
while IFS= read -r line; do
    case "$line" in
    Exec=*) printf 'Exec="%s" %%F\n' "$bin" ;;
    *) printf '%s\n' "$line" ;;
    esac
done <"$src/honeydew.desktop" >"$apps/honeydew.desktop"
for f in "${icon_files[@]}"; do install -Dm644 "$src/icons/hicolor/$f" "$icons/$f"; done
refresh
echo "Installed the Honeydew desktop entry ($apps/honeydew.desktop, Exec=$bin) and icons into $icons"

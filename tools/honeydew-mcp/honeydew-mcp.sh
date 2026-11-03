#!/usr/bin/env bash
set -euo pipefail

# One-time/local launcher for Honeydew's authenticated control bridge.
#
# Start the desktop app with:
#   tools/honeydew-mcp/honeydew-mcp.sh app
#
# Configure the MCP client to run this same script with the `adapter` argument.
# The adapter reads the same token file and starts the stdio MCP server.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(cd -- "$script_dir/../.." && pwd)
config_dir="${XDG_CONFIG_HOME:-${HOME}/.config}/honeydew"
token_file="${HONEYDEW_TOKEN_FILE:-${config_dir}/control-token}"
port="${HONEYDEW_CONTROL_PORT:-57831}"

usage() {
    cat >&2 <<EOF
Usage:
  $0 app [honeydew arguments...]   Start Honeydew with the MCP bridge enabled
  $0 adapter                       Run the stdio MCP adapter
  $0 token                         Print the token-file path

Configure the MCP client to run:
  $0 adapter
EOF
}

ensure_token() {
    if [[ ! -s "$token_file" ]]; then
        mkdir -p -- "$(dirname -- "$token_file")"
        umask 077
        python3 -c 'import secrets; print(secrets.token_urlsafe(32))' >"$token_file"
    fi

    HONEYDEW_CONTROL_TOKEN=$(<"$token_file")
    if (( ${#HONEYDEW_CONTROL_TOKEN} < 24 )); then
        echo "Honeydew MCP token must contain at least 24 characters: $token_file" >&2
        exit 1
    fi
}

run_app() {
    ensure_token
    export HONEYDEW_CONTROL_TOKEN
    export HONEYDEW_CONTROL_PORT="$port"
    cd -- "$repo_dir"
    exec cargo run --release --bin honeydew-native "$@"
}

run_adapter() {
    ensure_token
    export HONEYDEW_CONTROL_TOKEN
    export HONEYDEW_CONTROL_PORT="$port"
    exec python3 "$script_dir/server.py"
}

case "${1:-app}" in
    app|start)
        shift || true
        run_app "$@"
        ;;
    adapter)
        shift || true
        [[ $# -eq 0 ]] || { usage; exit 2; }
        run_adapter
        ;;
    token)
        [[ $# -eq 1 ]] || { usage; exit 2; }
        ensure_token
        printf '%s\n' "$token_file"
        ;;
    -h|--help|help)
        usage
        ;;
    *)
        usage
        exit 2
        ;;
esac

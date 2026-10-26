#!/usr/bin/env python3
"""Dependency-free local MCP adapter for Honeydew's opt-in control socket.

The adapter reads the desktop token file automatically. HONEYDEW_CONTROL_TOKEN
remains supported as an override for compatibility. Only JSON-RPC MCP messages
go to stdout; diagnostic errors go to stderr.
"""

import json
import os
import socket
import sys


def token_path():
    override = os.environ.get("HONEYDEW_TOKEN_FILE")
    if override:
        return override
    if sys.platform == "win32":
        base = os.environ.get("APPDATA")
        return os.path.join(base, "Honeydew", "control-token") if base else None
    if sys.platform == "darwin":
        home = os.path.expanduser("~")
        return os.path.join(home, "Library", "Application Support", "Honeydew", "control-token")
    base = os.environ.get("XDG_CONFIG_HOME", os.path.join(os.path.expanduser("~"), ".config"))
    return os.path.join(base, "Honeydew", "control-token")


def load_token():
    token = os.environ.get("HONEYDEW_CONTROL_TOKEN")
    if token:
        return token
    path = token_path()
    if not path:
        return ""
    try:
        with open(path, encoding="utf-8") as token_file:
            return token_file.read().strip()
    except OSError:
        return ""


TOOLS = [
    {
        "name": "get_song",
        "description": "Inspect Honeydew's currently open song, channel IDs, configured instruments/VSTs, drum pad indices, and clip IDs. Read this before composing or replacing a clip.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
    },
    {
        "name": "list_instruments",
        "description": "List Honeydew's built-in synths and installed instrument plugins. Existing configured plugin tracks should be reused by channel ID; creating new external plugin instances is not supported yet.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
    },
    {
        "name": "get_instrument_params",
        "description": "Read built-in synth modules and their parameter IDs, names, normalized values (0-1), defaults and display text on an existing channel. Call get_song first. External plugin state is not exposed.",
        "inputSchema": {"type": "object", "properties": {
            "channel_id": {"type": "integer"}
        }, "required": ["channel_id"], "additionalProperties": False},
    },
    {
        "name": "set_instrument_params",
        "description": "Adjust 1-32 normalized parameters of one existing built-in synth module. Call get_instrument_params first for its module_id and indexes. Values must be 0-1. All changes validate before applying; they update live audio and song state. Honeydew's existing synth knob edits are not individually undoable. Does not edit external VST/CLAP presets.",
        "inputSchema": {"type": "object", "properties": {
            "channel_id": {"type": "integer"}, "module_id": {"type": "integer"},
            "changes": {"type": "array", "minItems": 1, "maxItems": 32,
                        "items": {"type": "object", "properties": {
                            "index": {"type": "integer", "minimum": 0},
                            "value": {"type": "number", "minimum": 0, "maximum": 1}
                        }, "required": ["index", "value"], "additionalProperties": False}}
        }, "required": ["channel_id", "module_id", "changes"], "additionalProperties": False},
    },
    {
        "name": "apply_arrangement",
        "description": "Add or replace 1-16 note/drum clips and optionally set tempo in Honeydew's open song as one undoable edit. Use an existing channel ID to preserve its configured VST sound; use a new built-in synth or Drums for a new channel. Call get_song first. Bars are zero-based; note beats are quarter-note positions relative to each clip. For drums, key is the pad index returned by get_song. To revise a clip, pass replace_clip_id and the existing start_bar/bars. Does not change plugin presets or render audio.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tempo_bpm": {"type": "number", "minimum": 1, "maximum": 999},
                "tracks": {
                    "type": "array", "minItems": 1, "maxItems": 16,
                    "items": {
                        "type": "object",
                        "properties": {
                            "target": {
                                "oneOf": [
                                    {"type": "object", "properties": {
                                        "kind": {"const": "existing"}, "channel_id": {"type": "integer"},
                                        "replace_clip_id": {"type": "integer", "minimum": 1}
                                    }, "required": ["kind", "channel_id"], "additionalProperties": False},
                                    {"type": "object", "properties": {
                                        "kind": {"const": "new"}, "instrument": {"type": "string"},
                                        "name": {"type": "string"}
                                    }, "required": ["kind", "instrument"], "additionalProperties": False},
                                ]
                            },
                            "start_bar": {"type": "integer", "minimum": 0, "maximum": 4095},
                            "bars": {"type": "integer", "minimum": 1, "maximum": 16},
                            "notes": {
                                "type": "array", "maxItems": 512,
                                "items": {"type": "object", "properties": {
                                    "beat": {"type": "number", "minimum": 0},
                                    "length_beats": {"type": "number", "exclusiveMinimum": 0},
                                    "key": {"type": "integer", "minimum": 0, "maximum": 127},
                                    "velocity": {"type": "integer", "minimum": 1, "maximum": 255}
                                }, "required": ["beat", "length_beats", "key", "velocity"],
                                    "additionalProperties": False}
                            }
                        }, "required": ["target", "start_bar", "bars", "notes"],
                        "additionalProperties": False
                    }
                }
            }, "required": ["tracks"], "additionalProperties": False
        },
    },
]


def honeydew(action, payload=None, channel_id=None):
    token = load_token()
    if len(token) < 24:
        raise RuntimeError("Honeydew MCP token file is missing or invalid; generate a token in Honeydew Settings")
    port = int(os.environ.get("HONEYDEW_CONTROL_PORT", "57831"))
    request = {"token": token, "action": action}
    if payload is not None:
        request["arrangement" if action == "arrange" else "edit"] = payload
    if channel_id is not None:
        request["channel_id"] = channel_id
    with socket.create_connection(("127.0.0.1", port), timeout=4) as conn:
        conn.settimeout(4)
        conn.sendall(json.dumps(request, separators=(",", ":")).encode())
        conn.shutdown(socket.SHUT_WR)
        chunks = []
        while True:
            chunk = conn.recv(65536)
            if not chunk:
                break
            chunks.append(chunk)
            if sum(map(len, chunks)) > 1_048_576:
                raise RuntimeError("Honeydew response too large")
    return json.loads(b"".join(chunks))


def handle(message):
    method = message.get("method")
    if method == "initialize":
        return {"protocolVersion": message.get("params", {}).get("protocolVersion", "2025-03-26"),
                "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": {"name": "honeydew", "version": "0.1.0"},
                "instructions": "Use get_song before edits. Reuse existing configured VST channels by ID. apply_arrangement changes the open song and is one undo step. For built-in synth sound design, use get_instrument_params before set_instrument_params; these knob changes are not individually undoable. Do not claim to have auditioned the result; ask the user to listen and give feedback."}
    if method == "ping":
        return {}
    if method == "tools/list":
        return {"tools": TOOLS}
    if method == "tools/call":
        params = message.get("params", {})
        name = params.get("name")
        args = params.get("arguments", {})
        action = {"get_song": "song", "list_instruments": "instruments",
                  "get_instrument_params": "instrument_params",
                  "set_instrument_params": "set_instrument_params",
                  "apply_arrangement": "arrange"}.get(name)
        if action is None:
            return {"content": [{"type": "text", "text": f"Unknown tool: {name}"}], "isError": True}
        try:
            response = honeydew(action, args if action in ("arrange", "set_instrument_params") else None,
                                channel_id=args.get("channel_id") if action == "instrument_params" else None)
            error = response.get("error")
            return {"content": [{"type": "text", "text": json.dumps(response.get("result") if not error else {"error": error})}],
                    "isError": bool(error)}
        except (OSError, ValueError, RuntimeError) as exc:
            return {"content": [{"type": "text", "text": str(exc)}], "isError": True}
    return None


def main():
    for line in sys.stdin:
        try:
            message = json.loads(line)
            if "id" not in message:
                continue
            result = handle(message)
            response = {"jsonrpc": "2.0", "id": message["id"]}
            if result is None:
                response["error"] = {"code": -32601, "message": "Method not found"}
            else:
                response["result"] = result
        except (ValueError, TypeError, KeyError) as exc:
            response = {"jsonrpc": "2.0", "id": None,
                        "error": {"code": -32700, "message": str(exc)}}
        sys.stdout.write(json.dumps(response, separators=(",", ":")) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()

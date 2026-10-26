# Honeydew live MCP (local desktop)

This opt-in adapter lets a local MCP client edit the **open** Honeydew song. It
can inspect channels, identify configured VST/CLAP instruments, list installed
instruments, create note/drum clips, and adjust built-in synth parameters. An
arrangement is one undoable edit. Honeydew's existing synth knob changes are
not individually undoable, including those made through this adapter.
External plugin presets are kept as-is; adding a new external plugin or choosing
its preset is not supported yet. The adapter does not listen on the network:
it talks only to Honeydew's authenticated `127.0.0.1` bridge.

## Start

Enable MCP in Honeydew Settings and use `Generate` under Authentication token.
The Python adapter reads the same protected token file automatically. The port
defaults to 57831; set `HONEYDEW_CONTROL_PORT` only when using a custom port.
`HONEYDEW_CONTROL_TOKEN` remains available as a compatibility override.

From this repository:

```sh
cargo run --release --bin honeydew-native
```

For a one-command token setup, use the launcher instead:

```sh
tools/honeydew-mcp/honeydew-mcp.sh app
```

It stores a private token at `~/.config/honeydew/control-token`. Configure the
MCP client to launch `tools/honeydew-mcp/honeydew-mcp.sh adapter`; that mode
reads the same token and starts the stdio adapter automatically.

The MCP client starts the adapter as a **stdio** server; normally you do not
run it by hand. Configure its command as `python3` and its argument as the
absolute path to `tools/honeydew-mcp/server.py`. In Codex, for example, add
the stdio server in Settings → MCP servers, then use its tools in a new chat.
ChatGPT web requires a separate remote/tunneled MCP connection;
this local stdio configuration does not automatically make it available there.

## Example request

Ask the client: “Use the Honeydew tools to inspect my open song. Make an
original uplifting trance groove in eight bars using my existing Vital bass
track if there is one; put drums on a new Drums track. Show me the track IDs
and ask me to listen before revising.”

The `get_song` tool returns the channel IDs, their instrument names, clip IDs,
and drum pad indices. `apply_arrangement` accepts 1–16 tracks and optional
`tempo_bpm`. Each track uses
an existing channel ID or creates a new *built-in* synth/Drums track. Clip
positions are zero-based bars; notes' `beat` and `length_beats` are quarter-note
beats **relative to the clip**. A note's `key` is a MIDI pitch (0–127) or a
drum pad index, and velocity is 1–255. To revise a clip, use `replace_clip_id`
with its existing `start_bar` and `bars`. Invalid input changes nothing.

To shape a built-in synth, first call `get_song` for its `channel_id`, then
`get_instrument_params` for its `module_id`, parameter indexes, names, current
values, and defaults. `set_instrument_params` accepts 1–32 changes on that
module as normalized values from 0 to 1. All changes are validated together
and sent to the live audio engine; a bad index or value changes nothing. For
Transistor Bass, for example, indexes 2 and 9 are Cutoff and Drive. The tool
does not load named presets or alter a VST/CLAP plugin's private state.

Only enable the bridge for trusted local clients. The token is a bearer secret:
any local process that obtains it can edit the song. The bridge binds only to
loopback and validates/bounds each request, but it does not implement per-tool
permissions or expose a public endpoint.

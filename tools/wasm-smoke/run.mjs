// Runs the smoke exports of the wasm module that `cargo build --release --target wasm32-unknown-unknown` made here
// (or under $CARGO_TARGET_DIR). Fails on a missing import (instantiated with none), a missing export, a trap, or a file
// whose length or FNV-1a hash is not the pinned one.
import { readFileSync } from "node:fs";
import { join } from "node:path";

// Each export's file: [length, FNV-1a]. A native x86_64 (AVX2) build of `src/lib.rs` makes the same files, so the
// scalar wasm paths match the SIMD ones bit for bit. An encoder upgrade re-pins these.
const pinned = {
  smoke_mp3: [25666, 0xf6df1a09],
  smoke_ogg: [19795, 0xe12b681d],
};

const target = process.env.CARGO_TARGET_DIR ?? join(import.meta.dirname, "target");
const wasm = join(target, "wasm32-unknown-unknown", "release", "wasm_smoke.wasm");
const { instance } = await WebAssembly.instantiate(readFileSync(wasm), {});
const hex = (n) => n.toString(16).padStart(8, "0");
let failed = false;
for (const [name, [bytes, fnv]] of Object.entries(pinned)) {
  const summary = instance.exports[name]();
  const [gotBytes, gotFnv] = [Number(summary >> 32n), Number(summary & 0xffffffffn)];
  const line = `${name}: ${gotBytes} bytes, FNV-1a ${hex(gotFnv)}`;
  if (gotBytes === bytes && gotFnv === fnv) {
    console.log(line);
  } else {
    console.error(`${line}; pinned ${bytes} bytes, FNV-1a ${hex(fnv)}`);
    failed = true;
  }
}
process.exit(failed ? 1 : 0);

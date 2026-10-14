# `unsafe` in rusty_mp3

**Last audited**: 2026-10-04 (use-protection-please, H-16). **Policy**: `unsafe` exists
only for explicit SIMD intrinsics. No FFI, no raw allocation, no `transmute`, no
`static mut`, no manual `Send`/`Sync`. Every `unsafe` block carries a `// SAFETY:`
comment naming its bounds argument; every `unsafe fn` carries a `# Safety` section.

## The one class: SIMD kernels

Each kernel is a twin of a safe scalar function that stays in the tree as its
**oracle** (`*_simd_matches_scalar` tests, `assert_eq!` — the twins are
bit-identical by construction: output index = lane, original accumulation order,
separate mul + add, never FMA). The scalar twin is also the fallback on every CPU
without the ISA, and is forced for A/B with `MP3_ISA=scalar`.

The two obligations, and how each is discharged:

1. **The ISA is present.** `isa::use_simd()` / `simd_available()` decide once
   (AVX by runtime detection on x86_64; NEON is baseline on AArch64; scalar
   elsewhere). Every call into an AVX function is behind that flag, and the
   shipping paths enter one `#[target_feature(enable = "avx")]` function per
   granule (`hybrid_avx`, `polyphase_avx`) so the kernels inline inside it.
2. **Every access is in bounds.** All operands are **fixed-size arrays** whose
   lengths are part of the type, and every pointer offset is a function of loop
   counters and constant table dimensions — never of stream data. The two kernels
   that take a ring-buffer position (`window_*`, `fold_*`) mask it themselves
   (`head & !63`, `head & !31`), so their bounds hold for any argument; the
   quantizer kernels' loads are capped by `i + width <= hi <= 576` whatever the
   band table holds.

## Inventory

| File | Kernel(s) | Operands (types fix the bounds) | Oracle test |
|---|---|---|---|
| `src/decode/imdct.rs` | `imdct36_avx`, `imdct36_neon`, `imdct36_simd`, `hybrid_avx` | `cos36_t: [[f32; 24]; 18]`, `lines` via `first_chunk::<18>`, `out: [f32; 24]` | `imdct_simd_matches_scalar` |
| `src/decode/synthesis.rs` | `matrixing_avx/_neon/_simd`, `window_avx/_neon/_simd`, `polyphase_avx` | `half_dct_t: [[f32; 32]; 16]`, `s: [f32; 32]`, `fifo: [f32; 1024]`, `d: [f32; 512]`, `out: [f32; 32]` | `matrixing_simd_matches_scalar`, `window_simd_matches_scalar` (every legal `head`) |
| `src/encode/filterbank.rs` | `fold_avx/_neon/_simd`, `matrix_avx/_neon/_simd` | `c, fifo: [f32; 512]`, `matrix_t: [[f32; 32]; 64]`, `y: [f32; 64]` | `fold_simd_matches_scalar` (every legal `head`), `matrix_simd_matches_scalar` |
| `src/encode/quantize.rs` | `quantize_lines_avx/_neon/_simd` | `freq: [f32; 576]`, `xrp: [f64; 576]`, `out: [i32; 576]`, `off: [u16; 23]`, `steps: [f64; 22]` | `quantize_lines_simd_matches_scalar` (rounding seams, saturation, `-0.0`) |

`src/lab/` (the opt-in `lab` feature) mentions `unsafe` only in documentation.

Outside the library: `examples/allocaudit.rs` (a dev instrument, never shipped)
implements `GlobalAlloc` as a counting shim that forwards every call unchanged to
the project allocator; it carries its own SAFETY comments.

`cargo geiger --all-features` baseline (2026-10-04): unsafe functions 20/20,
expressions 572/609 (each intrinsic call counts), impls 0/1 (the example's
shim, not in the library build), zero dependencies. The count must not grow
without a row above (H-11).

## Verification

- Oracle tests run on x86_64 natively and on AArch64 under qemu
  (`tools/bench/arm_qemu_test.sh`); poisoning a NEON twin with a fused
  `vfmaq` fails all three decode oracles.
- Miri: `MIRIFLAGS=-Zmiri-deterministic-floats cargo +nightly miri test --lib`
  -> 86 passed, 0 failed, 21 ignored (18 end-to-end tests too slow to interpret,
  3 manual profiling runs), 2026-10-04. Miri's ISA probe reports no SIMD on its
  target (verified: `simd_available() == false` under Miri, `true` natively), so
  it exercises the scalar twins and all safe code; the intrinsic kernels are
  covered by the oracle tests and the bounds argument above.
- Sanitizers on the lib suite: ASan + LeakSanitizer, TSan, MSan -- all clean.
- Byte identity of the whole pipeline across arms: 720/720 decode hashes,
  `MP3_ISA=scalar` vs default (docs: `docs/plans/mp3-kernel-ledger.md`).

## Changing this file

Adding an `unsafe` block requires: a scalar oracle and a `*_matches_scalar` test, a
`// SAFETY:` comment with the bounds argument, a row above, and a bump of the
audit date. The crate's unsafe count must not grow without that (H-11).

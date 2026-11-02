// Modified from rusty_mp3 1.0.0 (see Cargo.toml).
//! MP3 decode pipeline.
//!
//! One frame flows through these bricks in order:
//!
//! ```text
//!  header ─▶ side info ─▶ bit reservoir ─▶ Huffman ─▶ scalefactors ─▶
//!  requantize ─▶ stereo (MS/intensity) ─▶ alias reduction ─▶
//!  hybrid IMDCT (+overlap) ─▶ polyphase synthesis ─▶ PCM
//! ```
//!
//! State that persists across frames lives on [`Mp3Decode`]: the bit reservoir,
//! the per-channel IMDCT overlap, and the synthesis filterbank FIFO.

use crate::{Error, Result};

use crate::frame::{GranuleSideInfo, GranuleSpectrum, SideInfo, GRANULE_LINES};
use crate::header::FrameHeader;

pub mod antialias;
pub mod codebooks;
pub mod huffman;
pub mod imdct;
pub mod requantize;
pub mod reservoir;
pub mod scalefactors;
pub mod sideinfo;
pub mod stereo;
pub mod synth_window;
pub mod synthesis;

/// Lightweight decode-stage profiler (near-zero cost; read via [`prof::dump`]).
/// Same role as the encoder's: find the real decode hotspots before optimizing.
pub mod prof {
    use std::sync::atomic::{AtomicU64, Ordering};
    // No clock on the browser target -- see `time` below.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    use std::time::Instant;

    pub static HUFFMAN: AtomicU64 = AtomicU64::new(0);
    pub static SCALEFAC: AtomicU64 = AtomicU64::new(0);
    pub static REQUANT: AtomicU64 = AtomicU64::new(0);
    pub static STEREO: AtomicU64 = AtomicU64::new(0);
    pub static ANTIALIAS: AtomicU64 = AtomicU64::new(0);
    pub static IMDCT: AtomicU64 = AtomicU64::new(0);
    pub static SYNTH: AtomicU64 = AtomicU64::new(0);

    /// Deterministic block-type census, counted per granule x channel. A stage
    /// share tells you which path is hot; this tells you what POPULATION that
    /// path serves — the two IMDCT paths (36-point long vs 3x12-point short)
    /// have different kernels, so "the IMDCT got 2x cheaper" is only a claim
    /// about the population these counters actually saw.
    pub static N_LONG: AtomicU64 = AtomicU64::new(0);
    pub static N_SHORT: AtomicU64 = AtomicU64::new(0);
    /// Mixed blocks: short overall, but the lowest two subbands stay long, so
    /// they exercise BOTH IMDCT paths within one granule.
    pub static N_MIXED: AtomicU64 = AtomicU64::new(0);

    /// KERNEL REACH census, tallied once per granule (never per element, so the
    /// tap cannot become the thing measured). Units: long subbands through the
    /// 36-point IMDCT, and synthesis passes (18 per granule per channel, each one
    /// matrixing + windowing call). Every arm counts, so "the fast arm ran" comes
    /// with "the slow arm did not" (codec-vectorize-kernel REACHABILITY.md).
    pub static K_IMDCT_SIMD: AtomicU64 = AtomicU64::new(0);
    pub static K_IMDCT_SCALAR: AtomicU64 = AtomicU64::new(0);
    pub static K_SYNTH_SIMD: AtomicU64 = AtomicU64::new(0);
    pub static K_SYNTH_SCALAR: AtomicU64 = AtomicU64::new(0);

    /// Add `n` units to a kernel-arm counter.
    #[inline]
    pub fn kernel_tally(c: &AtomicU64, n: u64) {
        c.fetch_add(n, Ordering::Relaxed);
    }

    /// One line: how much of the decode reached each kernel arm.
    pub fn kernel_census() -> String {
        let pct = |a: u64, b: u64| 100.0 * a as f64 / (a + b).max(1) as f64;
        let (ia, is) = (
            K_IMDCT_SIMD.load(Ordering::Relaxed),
            K_IMDCT_SCALAR.load(Ordering::Relaxed),
        );
        let (sa, ss) = (
            K_SYNTH_SIMD.load(Ordering::Relaxed),
            K_SYNTH_SCALAR.load(Ordering::Relaxed),
        );
        format!(
            "kernel census: imdct36 simd {ia} scalar {is} ({:.2}% simd) | synthesis simd {sa} scalar {ss} ({:.2}% simd)",
            pct(ia, is),
            pct(sa, ss)
        )
    }

    /// Print the block-type census and reset it.
    pub fn census() {
        let (l, s, m) = (
            N_LONG.swap(0, Ordering::Relaxed),
            N_SHORT.swap(0, Ordering::Relaxed),
            N_MIXED.swap(0, Ordering::Relaxed),
        );
        let tot = (l + s + m).max(1);
        eprintln!(
            "  block census: long {l} ({:.1}%), short {s} ({:.1}%), mixed {m} ({:.1}%)  \
             [granule x channel]",
            100.0 * l as f64 / tot as f64,
            100.0 * s as f64 / tot as f64,
            100.0 * m as f64 / tot as f64,
        );
    }

    /// Time `f` into `bucket` (nanoseconds, summed across calls).
    ///
    /// **There is no clock on `wasm32-unknown-unknown`.** `Instant::now()` traps
    /// there, and this wrapper sits on EVERY stage, so the codec aborted on its
    /// first frame in a browser while compiling cleanly and passing every test on
    /// the host. WASI has a clock and keeps its timing; only the bare-wasm target
    /// compiles it out and runs `f` unwrapped.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    #[inline]
    pub fn time<T>(bucket: &AtomicU64, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let r = f();
        bucket.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
        r
    }

    /// Clock-free twin for the browser target: run the stage, record nothing.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[inline]
    pub fn time<T>(_bucket: &AtomicU64, f: impl FnOnce() -> T) -> T {
        f()
    }

    pub fn dump() {
        let stages = [
            ("huffman", &HUFFMAN),
            ("scalefactors", &SCALEFAC),
            ("requantize", &REQUANT),
            ("stereo", &STEREO),
            ("antialias", &ANTIALIAS),
            ("imdct+overlap", &IMDCT),
            ("synthesis", &SYNTH),
        ];
        let total: u64 = stages.iter().map(|(_, b)| b.load(Ordering::Relaxed)).sum();
        eprintln!(
            "--- decode stage profile (total {:.1} ms) ---",
            total as f64 / 1e6
        );
        for (name, b) in stages {
            let ns = b.swap(0, Ordering::Relaxed);
            eprintln!(
                "  {name:<14} {:>8.1} ms  {:>5.1}%",
                ns as f64 / 1e6,
                100.0 * ns as f64 / total.max(1) as f64
            );
        }
    }
}

/// Which instruction set the decode kernels may use.
///
/// `MP3_ISA=scalar` forces every kernel onto its scalar twin, read ONCE per
/// process. It exists for two measurements no build flag can give cleanly: the
/// ISA-rung A/B (the value of every vector kernel at once, same binary both
/// arms) and the scalar-arm census (proof that the slow arm is not also what
/// production runs). Both arms are bit-identical by construction, so flipping it
/// changes speed only.
pub mod isa {
    /// A SIMD twin exists AND can run on this CPU (raw, ignores the override):
    /// AVX by runtime detection on x86_64; NEON on aarch64, where it is baseline
    /// (every AArch64 core has it, so there is nothing to detect); none elsewhere.
    #[inline]
    #[must_use]
    pub fn simd_available() -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            std::arch::is_x86_feature_detected!("avx")
        }
        #[cfg(target_arch = "aarch64")]
        {
            true
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            false
        }
    }

    /// Whether the shipping path takes the SIMD kernels: available AND not forced off.
    ///
    /// Vendored (studio): the `MP3_ISA` override is removed; the default.
    #[inline]
    pub fn use_simd() -> bool {
        simd_available()
    }
}

/// The transform half of the decoder's persistent state.
///
/// Split out from [`Mp3Decode`] because the decoder's state divides cleanly in
/// two: the bit reservoir belongs to the ENTROPY stage (frame-sync, Huffman,
/// scalefactors, requantize), while the IMDCT overlap and synthesis FIFO belong
/// to the TRANSFORM stage. Nothing is shared, which is what lets the two stages
/// run as a pipeline on separate threads — see [`crate::Mp3Decoder::decode_pipelined`].
pub struct TransformState {
    /// Previous granule's IMDCT tail for overlap-add, `[channel][line]`.
    imdct_overlap: [[f32; GRANULE_LINES]; 2],
    /// Synthesis filterbank FIFO `V[]`, `[channel][1024]`.
    synth_fifo: [[f32; 1024]; 2],
}

impl Default for TransformState {
    fn default() -> Self {
        Self {
            imdct_overlap: [[0.0; GRANULE_LINES]; 2],
            synth_fifo: [[0.0; 1024]; 2],
        }
    }
}

impl TransformState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage 2 for one granule: alias reduction → hybrid IMDCT → polyphase
    /// synthesis, appending interleaved PCM. Sequential per channel (the overlap
    /// and FIFO carry across granules), which is why the pipeline parallelises
    /// ACROSS stages rather than across granules.
    pub fn granule_to_pcm(
        &mut self,
        gi: &[GranuleSideInfo; 2],
        spectrum: &mut GranuleSpectrum,
        channels: usize,
        pcm: &mut Vec<f32>,
    ) {
        let mut chan_pcm = [[0f32; GRANULE_LINES]; 2];
        for ch in 0..channels {
            prof::time(&prof::ANTIALIAS, || {
                antialias::reduce(&gi[ch], &mut spectrum.lines[ch]);
            });
            let time = prof::time(&prof::IMDCT, || {
                imdct::hybrid(&gi[ch], &spectrum.lines[ch], &mut self.imdct_overlap[ch])
            });
            chan_pcm[ch] = prof::time(&prof::SYNTH, || {
                synthesis::polyphase(&time, &mut self.synth_fifo[ch])
            });
        }
        for s in 0..GRANULE_LINES {
            for cp in chan_pcm.iter().take(channels) {
                pcm.push(cp[s]);
            }
        }
    }
}

/// One granule handed from the entropy stage to the transform stage.
pub struct GranuleWork {
    pub side: [GranuleSideInfo; 2],
    pub spectrum: GranuleSpectrum,
}

/// Persistent decoder state across frames.
#[derive(Default)]
pub struct Mp3Decode {
    /// Carries leftover main-data bytes between frames (`main_data_begin`).
    reservoir: reservoir::Reservoir,
    /// The transform half (IMDCT overlap + synthesis FIFO).
    transform: TransformState,
}

impl Mp3Decode {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode one frame's side-info + main-data into interleaved PCM samples.
    ///
    /// The orchestration below is the wiring diagram; each `*::*` call is a brick
    /// still to be laid (`todo!()`). The public [`crate::Mp3Decoder`] returns
    /// `Unimplemented` until they're built, so this is never reached at runtime.
    /// Decode one frame's side-info + main-data into interleaved PCM samples.
    ///
    /// Thin wrapper over the two stages, so the serial path and the pipelined
    /// path run exactly the same code.
    ///
    /// # Errors
    /// Whatever [`decode_frame_entropy`](Self::decode_frame_entropy) returns:
    /// a malformed side-information block.
    pub fn decode_frame(
        &mut self,
        header: &FrameHeader,
        side_info_bytes: &[u8],
        frame_main_data: &[u8],
    ) -> Result<Vec<f32>> {
        let channels = header.channel_mode.channels();
        let mut pcm = Vec::with_capacity(header.version.granules() * GRANULE_LINES * channels);
        // Each granule goes from the entropy stage straight into the transform
        // stage, by reference. The serial path used to collect the frame's
        // granules into a `Vec<GranuleWork>` first -- an allocation per frame,
        // and every granule's spectrum (4.6 KB) copied in and copied out again.
        // The two stages own disjoint state, so running them per granule rather
        // than per frame changes nothing they compute.
        let Self {
            reservoir,
            transform,
        } = self;
        entropy(
            reservoir,
            header,
            side_info_bytes,
            frame_main_data,
            |side, spectrum| transform.granule_to_pcm(side, spectrum, channels, &mut pcm),
        )?;
        Ok(pcm)
    }

    /// **Stage 1 (entropy).** Side info → reservoir → Huffman → scalefactors →
    /// requantize → joint stereo, producing one [`GranuleWork`] per granule.
    ///
    /// Touches only the bit reservoir, never the IMDCT overlap or synthesis FIFO,
    /// which is what makes it safe to run on its own thread ahead of stage 2.
    ///
    /// # Errors
    /// [`Error::InvalidData`](crate::Error::InvalidData) when the side information
    /// is malformed (see [`sideinfo::parse`]).
    pub fn decode_frame_entropy(
        &mut self,
        header: &FrameHeader,
        side_info_bytes: &[u8],
        frame_main_data: &[u8],
    ) -> Result<Vec<GranuleWork>> {
        // The pipelined decoder hands granules to another thread, so it needs
        // them OWNED; this is the one place they are collected.
        let mut out = Vec::with_capacity(header.version.granules());
        entropy(
            &mut self.reservoir,
            header,
            side_info_bytes,
            frame_main_data,
            |side, spectrum| {
                out.push(GranuleWork {
                    side: side.clone(),
                    spectrum: spectrum.clone(),
                });
            },
        )?;
        Ok(out)
    }
}

/// The entropy stage over one frame, handing each finished granule to
/// `granule` by reference (its side info and its spectrum, ready for the
/// transform stage). A free function over the reservoir alone, so a caller can
/// lend it the reservoir while holding the transform state for `granule`.
fn entropy(
    reservoir: &mut reservoir::Reservoir,
    header: &FrameHeader,
    side_info_bytes: &[u8],
    frame_main_data: &[u8],
    mut granule: impl FnMut(&[GranuleSideInfo; 2], &mut GranuleSpectrum),
) -> Result<()> {
    let channels = header.channel_mode.channels();
    let granules = header.version.granules();

    // 1. Side information → the per-granule decode recipe.
    let si: SideInfo = sideinfo::parse(header, side_info_bytes)?;

    // 2. Reassemble main data across the reservoir boundary.
    let main = reservoir.assemble(si.main_data_begin, frame_main_data);

    let mut bit_pos = 0usize;
    // Granule 0's scalefactors are retained per channel for granule 1 `scfsi`
    // reuse.
    let mut scalefac: [[scalefactors::ScaleFactors; 2]; 2] = Default::default();
    for gr in 0..granules {
        let mut spectrum = GranuleSpectrum::default();
        for ch in 0..channels {
            let gi = &si.granules[gr][ch];
            // Block-type census: which IMDCT path this granule/channel takes.
            {
                use std::sync::atomic::Ordering::Relaxed;
                let short = gi.window_switching && gi.block_type == crate::frame::BlockType::Short;
                match (short, gi.mixed_block) {
                    (true, true) => prof::N_MIXED.fetch_add(1, Relaxed),
                    (true, false) => prof::N_SHORT.fetch_add(1, Relaxed),
                    _ => prof::N_LONG.fetch_add(1, Relaxed),
                };
            }
            // part2 (scalefactors) + part3 (Huffman) share one bit budget.
            let part2_3_start = bit_pos;
            let prev = (gr == 1).then(|| &scalefac[0][ch]);
            let sf = prof::time(&prof::SCALEFAC, || {
                scalefactors::decode(main, &mut bit_pos, header, &si, gr, ch, prev)
            });
            let part2_3_end = part2_3_start + gi.part2_3_length as usize;
            let (coeffs, nz) = prof::time(&prof::HUFFMAN, || {
                huffman::decode(main, &mut bit_pos, part2_3_end, header, gi)
            });
            prof::time(&prof::REQUANT, || {
                requantize::apply(header, gi, &sf, &coeffs, nz, &mut spectrum.lines[ch]);
            });
            spectrum.nonzero[ch] = nz;
            scalefac[gr][ch] = sf;
        }

        // 7. Joint-stereo (MS / intensity) across the two channels.
        prof::time(&prof::STEREO, || {
            stereo::process(header, &si.granules[gr], &scalefac[gr][1], &mut spectrum);
        });

        granule(&si.granules[gr], &mut spectrum);
    }
    Ok(())
}

/// Entry used by the public decoder once the bricks are in place.
///
/// # Errors
/// Always [`Error::Unimplemented`](crate::Error::Unimplemented).
pub fn decode_frame_stub() -> Result<()> {
    Err(Error::Unimplemented("mp3 decode: pipeline not yet built"))
}

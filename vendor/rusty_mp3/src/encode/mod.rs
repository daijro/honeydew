// Modified from rusty_mp3 1.0.0 (see Cargo.toml).
//! MP3 encode pipeline (the inverse of decode, plus a psychoacoustic model).
//!
//! ```text
//!  PCM ─▶ analysis filterbank ─▶ MDCT ─▶ psychoacoustic model ─▶
//!  quantize (inner rate loop + outer distortion loop) ─▶ Huffman ─▶
//!  bitstream (side info + main data + reservoir) ─▶ frame
//! ```
//!
//! The psychoacoustic model and the two-loop quantizer are the quality-defining
//! bricks — they decide block type and how to shape quantization noise under the
//! masking threshold, the way LAME does. Everything else is mechanical.

use crate::Result;

use crate::bitio::BitWriter;
use crate::decode::scalefactors::ScaleFactors;
use crate::frame::{BlockType, GranuleSideInfo, SideInfo, GRANULE_LINES};
use crate::header::FrameHeader;

pub mod antialias;
pub mod bitstream;
pub mod fft;
pub mod filterbank;
pub mod huffman;
pub mod mdct;
pub mod psychoacoustic;
pub mod quantize;
pub mod shortblock;
pub mod stereo;

/// Lightweight stage profiler (near-zero cost; read via [`prof::dump`]). Used to
/// find the real encode hotspots instead of guessing — the basis of the
/// optimization plan's measurements.
pub mod prof {
    use std::sync::atomic::{AtomicU64, Ordering};
    // No clock on the browser target -- see `time` below.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    use std::time::Instant;

    pub static FILTERBANK: AtomicU64 = AtomicU64::new(0);
    pub static PSY: AtomicU64 = AtomicU64::new(0);
    pub static MDCT: AtomicU64 = AtomicU64::new(0);
    pub static QUANT: AtomicU64 = AtomicU64::new(0);
    pub static HUFF: AtomicU64 = AtomicU64::new(0);
    // Block-type distribution (the split decides which quantizer path runs).
    pub static N_LONG: AtomicU64 = AtomicU64::new(0);
    pub static N_SHORT: AtomicU64 = AtomicU64::new(0);
    // Distortion-loop behaviour: how often the kept result is iteration 0 (the loop
    // never improved past the rate-loop quantization) vs total long granules.
    pub static OUTER_KEPT0: AtomicU64 = AtomicU64::new(0);
    pub static OUTER_TOTAL: AtomicU64 = AtomicU64::new(0);
    /// Outer-loop iterations actually executed. Each one runs a full inner rate
    /// loop (a binary search of quantize + Huffman table selection), so when
    /// `OUTER_KEPT0 == OUTER_TOTAL` this counter is the work the encoder did and
    /// then discarded.
    pub static OUTER_ITERS: AtomicU64 = AtomicU64::new(0);
    /// Histogram of per-band noise-to-mask ratio at the rate loop's chosen gain,
    /// bucketed by `log10(noise/threshold)`: index 0 is `< -6` decades (noise a
    /// million times under the mask), index 7 is `>= 0` (noise AT or ABOVE the
    /// mask, i.e. audible). The distortion loop only ever fires on bucket 7, so
    /// this says whether it is silent because the coding is good or because the
    /// thresholds are unreachable.
    pub static NMR_HIST: [AtomicU64; 8] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];

    /// Times the pre-fix `MAX_SF` guard would have amplified a band the
    /// bitstream cannot represent (bands 11..20 above 7), which the serializer
    /// silently truncated. Non-zero here means the truncation bug was reachable
    /// on this content, not merely latent.
    pub static SF_OVERFLOW: AtomicU64 = AtomicU64::new(0);

    /// Scalefactor amplification steps accepted by the slack refinement -- the
    /// per-band shaping the encoder actually bought with its leftover bits.
    pub static REFINE_STEPS: AtomicU64 = AtomicU64::new(0);

    /// Refinement steps where the band the ranking wanted most was already at its
    /// scalefactor ceiling (`max_sf`), split low (0..=10, cap 15) / high (11..=20,
    /// cap 7). The demand `scalefac_scale` (doubles every band's range) and
    /// `preflag` (adds the pretab to the high bands) could serve.
    pub static CAP_BLOCK_LO: AtomicU64 = AtomicU64::new(0);
    pub static CAP_BLOCK_HI: AtomicU64 = AtomicU64::new(0);
    /// Shaped long granules whose high bands ALL sit at or above the pretab after
    /// refinement -- i.e. `preflag` could be set losslessly, freeing scalefactor bits.
    pub static PREFLAG_OK: AtomicU64 = AtomicU64::new(0);
    /// Scalefactor steps accepted by short-block shaping (`quantize_short_slack`).
    pub static SHORT_REFINE_STEPS: AtomicU64 = AtomicU64::new(0);
    /// Analysis-filterbank passes by kernel arm, `[scalar, simd]` -- the encoder's
    /// kernel-reach census (18 per granule per channel), tallied once per granule.
    pub static FB_PASSES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
    /// `quantize_into` calls by kernel arm, `[scalar, simd]` (~8 per long granule).
    pub static QUANT_CALLS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

    /// Which term decided each band's masking threshold: the energy cap, the
    /// absolute threshold of hearing, or the spread masker itself. Only the last
    /// carries any masking information.
    pub static THR_CAPPED: AtomicU64 = AtomicU64::new(0);
    pub static THR_ATH: AtomicU64 = AtomicU64::new(0);
    pub static THR_MASK: AtomicU64 = AtomicU64::new(0);

    /// Sum of `1000 * log10(fft_energy / mdct_energy)` over granules, and the
    /// granule count. The psymodel's thresholds are derived from an unnormalized
    /// 1024-point FFT; the distortion loop's noise is squared error on MDCT
    /// coefficients. If those two domains do not share a scale, every comparison
    /// between them is off by this ratio -- so measure it instead of deriving it.
    pub static DOMAIN_LOG_SUM: AtomicU64 = AtomicU64::new(0);
    pub static DOMAIN_N: AtomicU64 = AtomicU64::new(0);

    /// Fold one granule's FFT-vs-MDCT energy ratio in.
    #[inline]
    pub fn note_domain(fft_energy: f32, mdct_energy: f32) {
        if fft_energy > 0.0 && mdct_energy > 0.0 {
            let l = (fft_energy / mdct_energy).log10() * 1000.0;
            DOMAIN_LOG_SUM.fetch_add(l.max(0.0) as u64, Ordering::Relaxed);
            DOMAIN_N.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Fold one band's noise-to-mask ratio into [`NMR_HIST`].
    ///
    /// Bucketed by comparison rather than `log10`: this runs once per band per
    /// granule on the shipping encode path, and a transcendental there would be
    /// the instrument charging the thing it measures (~39k `log10` calls on a
    /// 24 s clip). Seven predictable compares cost nothing.
    #[inline]
    pub fn note_nmr(nmr: f32) {
        const EDGES: [f32; 7] = [1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 1e-1, 1.0];
        let mut d = 0;
        while d < 7 && nmr >= EDGES[d] {
            d += 1;
        }
        NMR_HIST[d].fetch_add(1, Ordering::Relaxed);
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

    /// Print the per-stage breakdown and reset the counters.
    pub fn dump() {
        let stages = [
            ("filterbank", &FILTERBANK),
            ("psychoacoustic", &PSY),
            ("mdct+alias", &MDCT),
            ("quantize", &QUANT),
            ("huffman-emit", &HUFF),
        ];
        let total: u64 = stages.iter().map(|(_, b)| b.load(Ordering::Relaxed)).sum();
        eprintln!(
            "--- encode stage profile (total {:.1} ms) ---",
            total as f64 / 1e6
        );
        for (name, b) in stages {
            let ns = b.swap(0, Ordering::Relaxed);
            eprintln!(
                "  {name:<16} {:>8.1} ms  {:>5.1}%",
                ns as f64 / 1e6,
                100.0 * ns as f64 / total.max(1) as f64
            );
        }
        let (nl, ns) = (
            N_LONG.swap(0, Ordering::Relaxed),
            N_SHORT.swap(0, Ordering::Relaxed),
        );
        eprintln!("  granules: {nl} long, {ns} short");
    }
}

/// Persistent encoder state across frames.
pub struct Mp3Encode {
    /// Analysis filterbank FIFO, `[channel][512]`.
    analysis_fifo: [[f32; 512]; 2],
    /// MDCT overlap tail, `[channel][line]`.
    mdct_overlap: [[f32; GRANULE_LINES]; 2],
    /// Encoder-side bit reservoir (spare bits donated to future frames).
    reservoir: bitstream::EncReservoir,
    /// Last granule's window type — the block-switch FSM carries it across frames.
    prev_block_type: crate::frame::BlockType,
    /// **3R1 reservoir-RD** buffered path: raw frames banked for B8 assembly at flush,
    /// the running spare-bit bank (bits, ≤ `RESV_MAX_BANK`), and the smoothed frame
    /// perceptual entropy the causal budget is measured against.
    resv_frames: Vec<(FrameHeader, SideInfo, Vec<u8>)>,
    resv_bank: usize,
    resv_pe_avg: f32,
    /// Reusable mid/side PCM scratch, `[mid, side]`. Persisting it across frames
    /// keeps the joint-stereo path allocation-free in steady state — it used to
    /// build two fresh zero-filled `Vec`s (plus an outer one) every frame.
    ms_scratch: [Vec<f32>; 2],
}

impl Default for Mp3Encode {
    fn default() -> Self {
        Self {
            analysis_fifo: [[0.0; 512]; 2],
            mdct_overlap: [[0.0; GRANULE_LINES]; 2],
            reservoir: bitstream::EncReservoir::default(),
            prev_block_type: crate::frame::BlockType::Long,
            resv_frames: Vec::new(),
            resv_bank: 0,
            resv_pe_avg: 0.0,
            ms_scratch: [Vec::new(), Vec::new()],
        }
    }
}

/// B8 back-reference cap (`main_data_begin` is 9 bits): the most a frame can borrow.
const RESV_MAX_BANK: usize = 511 * 8;

/// `MP3_STEREO` -- the joint-stereo decision override (see `decide_stereo`).
#[derive(Clone, Copy)]
enum StereoOverride {
    Lr,
    Ms,
    Pe,
}

/// Read once, like every other knob. It was read from the environment on every
/// frame -- an allocation per frame (the lookup's key and value buffers), the
/// last unexplained one in the encoder's steady state.
///
/// Vendored (studio): the `MP3_STEREO` override is removed; the default.
fn stereo_override() -> Option<StereoOverride> {
    None
}

/// **3R1** causal per-frame main-data budget: a frame `pe/pe_avg` above the running
/// average draws extra bits from the bank (capped by what's physically available),
/// an easier one banks — sum-preserving around the CBR `base`, so the average bitrate
/// holds and the bank never goes negative (⇒ B8's cumulative constraint is satisfied).
fn reservoir_budget(pe: f32, pe_avg: f32, base: usize, bank: usize, gain: f32) -> usize {
    if pe_avg <= 0.0 || gain <= 0.0 {
        return base;
    }
    let demand = f64::from(pe / pe_avg - 1.0); // >0 harder than average
    let extra = (demand * f64::from(gain) * base as f64) as i64;
    // draw at most the available bank; give back at most 30% of base (don't starve).
    let extra = extra.clamp(-(base as i64) * 3 / 10, bank.min(RESV_MAX_BANK) as i64);
    ((base as i64) + extra).max(base as i64 / 2) as usize
}

impl Mp3Encode {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Encode one frame from per-channel PCM (`channels[ch]` holds this frame's
    /// `granules × 576` samples): per granule, per channel, analysis filterbank →
    /// forward MDCT → rate-loop quantize → Huffman, then assemble (reservoir-free).
    ///
    /// Mono, independent stereo, or **mid/side joint stereo** (chosen per frame
    /// when the channels are correlated). Main data is laid out granule-major then
    /// channel-major, the order the decoder reads it.
    ///
    /// `quality` selects the rate mode: `None` is CBR (the header's bitrate);
    /// `Some(target_nmr)` is **VBR** — each granule quantizes to that quality and
    /// the frame's bitrate is picked to fit the result.
    ///
    /// # Errors
    /// None today -- the frame always encodes; the `Result` keeps the signature
    /// stable for callers.
    pub fn encode_frame<C: AsRef<[f32]>>(
        &mut self,
        header: &FrameHeader,
        channels: &[C],
        quality: Option<f32>,
    ) -> Result<Vec<u8>> {
        // B7 (streaming, reservoir-free): the frame's whole region is its own budget.
        let frame_budget = bitstream::region_capacity(header) * 8;
        let (fheader, side, main_data, _pe) =
            self.encode_frame_raw(header, channels, quality, |_pe| frame_budget);
        Ok(bitstream::format(
            &fheader,
            &side,
            &main_data,
            &mut self.reservoir,
        ))
    }

    /// **3R1** — encode a CBR frame into the reservoir buffer: the causal PE-weighted
    /// budget lets a demanding frame borrow banked bits, an easy one bank them. The raw
    /// frame is stored for B8 assembly at [`finish_reservoir`](Self::finish_reservoir). `gain` is the knob.
    pub fn encode_frame_reservoir<C: AsRef<[f32]>>(
        &mut self,
        header: &FrameHeader,
        channels: &[C],
        gain: f32,
    ) {
        let base = bitstream::region_capacity(header) * 8;
        let (bank, pe_avg) = (self.resv_bank, self.resv_pe_avg);
        let (fheader, side, main_data, pe) = self.encode_frame_raw(header, channels, None, |pe| {
            reservoir_budget(pe, pe_avg, base, bank, gain)
        });
        // Bank what this frame left unspent (bounded by the 511-byte back-reference).
        let used = main_data.len() * 8;
        self.resv_bank = (bank + base).saturating_sub(used).min(RESV_MAX_BANK);
        self.resv_pe_avg = if self.resv_frames.is_empty() {
            pe
        } else {
            0.9 * pe_avg + 0.1 * pe
        };
        self.resv_frames.push((fheader, side, main_data));
    }

    /// **3R1** — assemble all banked frames into one reservoir-borrowed CBR stream (B8),
    /// clearing the buffer. Returns the frames in order, each padded to its fixed size.
    pub fn finish_reservoir(&mut self) -> Vec<u8> {
        let frames = std::mem::take(&mut self.resv_frames);
        self.resv_bank = 0;
        self.resv_pe_avg = 0.0;
        bitstream::assemble_stream(&frames)
    }

    /// [`finish_reservoir`](Self::finish_reservoir), one packet per frame: the
    /// stream-level encoder emits frames as packets, and assembling them
    /// directly saves cutting one contiguous stream back up with a copy apiece.
    pub(crate) fn finish_reservoir_frames(&mut self) -> Vec<Vec<u8>> {
        let frames = std::mem::take(&mut self.resv_frames);
        self.resv_bank = 0;
        self.resv_pe_avg = 0.0;
        bitstream::assemble_frames(&frames)
    }

    /// Analyse every granule×channel of a frame (advancing the filterbank/MDCT state)
    /// and its perceptual entropy, then quantise all of them to a frame main-data
    /// budget chosen by `budget_fn(frame_pe)` — the seam that lets B7 pass a flat
    /// per-region budget and 3R1 pass a reservoir-weighted one. Returns the (possibly
    /// M/S) header, side info, raw main data, and the frame's total perceptual entropy.
    fn encode_frame_raw<C: AsRef<[f32]>>(
        &mut self,
        header: &FrameHeader,
        channels: &[C],
        quality: Option<f32>,
        budget_fn: impl FnOnce(f32) -> usize,
    ) -> (FrameHeader, SideInfo, Vec<u8>, f32) {
        let fa = self.analyze_frame(header, channels);
        let budget = budget_fn(fa.frame_pe);
        let (fheader, side, main_data) = Self::quantize_frame(&fa, budget, quality);
        (fheader, side, main_data, fa.frame_pe)
    }

    /// Per-frame joint-stereo (M/S vs independent L/R) decision.
    ///
    /// Default is the raw-energy heuristic ([`stereo::prefer_mid_side`]), whose choice
    /// changes SLOWLY (energy is smooth frame-to-frame → mode switches in blocks). That
    /// matters: per-frame M/S switching has a residual corruption bug — a mode that
    /// flips nearly every frame destroys the reconstruction (corr → 0.27, confirmed by
    /// BOTH ffmpeg and our own decoder), while blocky switching reconstructs cleanly
    /// (corr 1.0). The frequency-domain M/S in `analyze_frame` fixed the common *blocky*
    /// case (piano@128 −3.87 → −0.71); the every-frame case is still broken.
    ///
    /// `MP3_STEREO=pe` selects a cost-based decision — pick the lower summed perceptual
    /// entropy (bit demand), `PE(M)+PE(S)` vs `PE(L)+PE(R)`; M/S is lossless so fewer
    /// bits ⇒ finer quantiser ⇒ better CBR quality. It out-scores the energy test *in
    /// principle* (force-M/S beat it by up to +0.07 ODG) but oscillates the mode every
    /// frame → triggers the switching bug. OPT-IN until that bug is root-caused; a robust
    /// version needs hysteresis to keep switches blocky. `lr`/`ms` force a fixed mode.
    fn decide_stereo<C: AsRef<[f32]>>(channels: &[C], granules: usize, sample_rate: u32) -> bool {
        let (l, r) = (channels[0].as_ref(), channels[1].as_ref());
        match stereo_override() {
            Some(StereoOverride::Lr) => return false,
            Some(StereoOverride::Ms) => return true,
            Some(StereoOverride::Pe) => {
                let ms = stereo::mid_side(l, r);
                let (mut pe_lr, mut pe_ms) = (0f32, 0f32);
                let pe = |pcm: &[f32]| psychoacoustic::analyze(pcm, sample_rate).perceptual_entropy;
                for gr in 0..granules {
                    let s = gr * GRANULE_LINES;
                    pe_lr += pe(&l[s..]) + pe(&r[s..]);
                    pe_ms += pe(&ms[0][s..]) + pe(&ms[1][s..]);
                }
                return pe_ms < pe_lr;
            }
            None => {}
        }
        stereo::prefer_mid_side(l, r)
    }

    /// Analyse one frame (M/S decision, block-switch FSM, filterbank → MDCT → psymodel
    /// per granule×channel) WITHOUT quantising — advances the persistent state exactly
    /// once. Splitting this from [`quantize_frame`] lets the 3R1 lookahead path analyse
    /// every frame first (to know the global perceptual-entropy distribution) and only
    /// then choose per-frame budgets.
    fn analyze_frame<C: AsRef<[f32]>>(
        &mut self,
        header: &FrameHeader,
        channels: &[C],
    ) -> FrameAnalysis {
        let nch = header.channel_mode.channels();
        let granules = header.version.granules();

        let use_ms = nch == 2 && Self::decide_stereo(channels, granules, header.sample_rate);
        // The psymodel below is the only consumer of the CODED domain. For L/R that
        // IS `channels`, so borrow it; for M/S, fill scratch that persists across
        // frames. Neither branch allocates in steady state — the L/R branch used to
        // deep-copy both channels via `to_vec()` purely to match the M/S branch's
        // `Vec<Vec<f32>>` type.
        let mut ms = std::mem::take(&mut self.ms_scratch);
        if use_ms {
            let [m, s] = &mut ms;
            stereo::mid_side_into(channels[0].as_ref(), channels[1].as_ref(), m, s);
        }
        // Bind the RAW channels into a fixed-size array too, exactly as `coded`
        // below. `channels` is a slice, so every `channels[ch]` is a bounds
        // check against a length the compiler cannot know, and the granule slice
        // that follows is a second one -- paid per granule, per channel, in the
        // attack scan and again in the filterbank loop.
        let raw: [&[f32]; 2] = if nch == 2 {
            [channels[0].as_ref(), channels[1].as_ref()]
        } else {
            [channels[0].as_ref(), channels[0].as_ref()]
        };
        let coded: [&[f32]; 2] = if use_ms {
            [ms[0].as_slice(), ms[1].as_slice()]
        } else {
            raw
        };
        let mut fheader = header.clone();
        if use_ms {
            fheader.channel_mode = crate::frame::ChannelMode::JointStereo {
                ms_stereo: true,
                intensity_stereo: false,
            };
        }

        // Detect attacks on the RAW L/R channels (what the lapped MDCT actually
        // transforms), NOT on `coded`: if attacks tracked the M/S choice, a per-frame
        // M/S flip would make the block-type window oscillate too, and rapid block-type
        // churn breaks MDCT time-domain aliasing cancellation across frames.
        // At most two granules a frame: fixed arrays, not two `Vec`s a frame.
        let mut attacks = [false; 2];
        for (gr, attack) in attacks.iter_mut().enumerate().take(granules) {
            *attack = (0..nch).any(|ch| {
                let g = &raw[ch][gr * GRANULE_LINES..(gr + 1) * GRANULE_LINES];
                psychoacoustic::detect_attack(g)
            });
        }
        let (block_types, new_prev) =
            shortblock::decide_block_types_pair(self.prev_block_type, &attacks[..granules]);
        self.prev_block_type = new_prev;

        let n_units = granules * nch;
        let mut analyzed = Vec::with_capacity(n_units);
        let mut frame_pe = 0f32;
        for gr in 0..granules {
            let bt = block_types[gr];
            let block = GranuleSideInfo {
                window_switching: bt != BlockType::Long,
                block_type: bt,
                ..Default::default()
            };
            // The lapped filterbank/MDCT carry PERSISTENT per-channel overlap
            // (`analysis_fifo`/`mdct_overlap`), so they must run on a domain that is
            // CONTINUOUS across frames. Always transform the raw L/R channels; M/S is
            // then a per-line SPECTRAL rotation (below). That rotation commutes with
            // MDCT+antialias (identical coefficients in steady state) and matches the
            // decoder — but, unlike rotating in the PCM domain before the lapped
            // transform, it does NOT corrupt the overlap when the mode switches
            // L/R<->M/S between adjacent frames (which mangled every switch boundary).
            //
            // Each channel's spectrum goes STRAIGHT into its `analyzed` slot, and
            // the M/S rotation happens there. It used to be collected into a
            // `freqs` Vec first and then copied into `analyzed` -- an allocation
            // per granule and every spectrum copied twice. The psymodel reads
            // only the PCM, never the spectrum, so running it first changes
            // nothing it computes; `frame_pe` still accumulates in channel order.
            let first = analyzed.len();
            for ch in 0..nch {
                // Psymodel on the CODED domain (M/S or L/R): stateless and frame-local,
                // so it follows the mode without the lapped-overlap constraint.
                let mut psy = prof::time(&prof::PSY, || {
                    psychoacoustic::analyze(&coded[ch][gr * GRANULE_LINES..], fheader.sample_rate)
                });
                if !matches!(fheader.version, crate::header::MpegVersion::V1) {
                    psy.thresholds = [f32::MAX; crate::frame::SFB_LONG];
                }
                frame_pe += psy.perceptual_entropy;
                let gpcm = &raw[ch][gr * GRANULE_LINES..];
                let sub = prof::time(&prof::FILTERBANK, || {
                    filterbank::analyze(gpcm, &mut self.analysis_fifo[ch])
                });
                let freq = prof::time(&prof::MDCT, || {
                    let mut freq = mdct::forward(&sub, bt, &mut self.mdct_overlap[ch]);
                    antialias::expand(&block, &mut freq);
                    freq
                });
                analyzed.push((freq, psy, bt));
            }
            // Spectral M/S: M=(L+R)/√2, S=(L−R)/√2 — the inverse of the decoder's
            // (M,S)→(L,R) rotation, applied to the full granule spectrum.
            if use_ms && nch == 2 {
                let inv_sqrt2 = std::f32::consts::FRAC_1_SQRT_2;
                // Bind the two spectra ONCE, so the 576-line loop indexes two
                // plain arrays rather than re-indexing the `Vec` per access.
                let (l, r) = analyzed[first..].split_at_mut(1);
                for (lv, rv) in l[0].0.iter_mut().zip(r[0].0.iter_mut()) {
                    let (a, b) = (*lv, *rv);
                    *lv = (a + b) * inv_sqrt2;
                    *rv = (a - b) * inv_sqrt2;
                }
            }
        }
        // `coded`'s last use is inside the loop, so the borrow of `ms` has ended
        // and the scratch can go back for the next frame to reuse.
        self.ms_scratch = ms;
        FrameAnalysis {
            fheader,
            analyzed,
            frame_pe,
        }
    }

    /// Quantise a pre-analysed frame to `frame_budget` main-data bits and emit the raw
    /// bitstream — the stateless (given the analysis) half of the encode. `quality`
    /// `Some` selects VBR (targets a quality, ignores the budget).
    fn quantize_frame(
        fa: &FrameAnalysis,
        frame_budget: usize,
        quality: Option<f32>,
    ) -> (FrameHeader, SideInfo, Vec<u8>) {
        use std::sync::atomic::Ordering::Relaxed;
        let mut fheader = fa.fheader.clone();
        let nch = fheader.channel_mode.channels();
        let n_units = fa.analyzed.len().max(1);
        // VBR: the quality target IS a target bitrate, so it becomes a per-granule
        // BIT BUDGET and then runs the same two-loop quantizer CBR uses. The old
        // path was a separate NMR gain search that PEAQ measured 3.5 ODG behind
        // LAME -- worse at 268 kbps than CBR managed at 192 -- because its
        // criterion was unanchored. Rate-driving it inherits CBR's quality, which
        // PEAQ puts within ~0.3 ODG of LAME.
        let per_gran = match quality {
            Some(target_kbps) => {
                let mut h = fheader.clone();
                h.bitrate_kbps = crate::snap_bitrate(h.version, target_kbps.round() as u32);
                bitstream::region_capacity(&h) * 8 / n_units
            }
            None => frame_budget / n_units,
        };

        let mut side = SideInfo::default();
        // The main-data region is capacity-bounded by the header, so the writer
        // never needs to grow.
        let mut main = BitWriter::with_capacity(bitstream::region_capacity(&fheader));
        for (idx, (freq, psy, bt)) in fa.analyzed.iter().enumerate() {
            let (gr, ch) = (idx / nch, idx % nch);
            let bt = *bt;
            let quant = prof::time(&prof::QUANT, || {
                if bt == BlockType::Short {
                    prof::N_SHORT.fetch_add(1, Relaxed);
                    let fbs = shortblock::reorder_subband_to_bitstream(fheader.sample_rate, freq);
                    // `per_gran` already encodes the mode: CBR's frame budget or
                    // VBR's target-rate budget. Short blocks need no special case.
                    shortblock::quantize_short_slack(
                        &fheader,
                        &fbs,
                        per_gran,
                        &psy.thresholds,
                        psy.signal_energy,
                    )
                } else {
                    prof::N_LONG.fetch_add(1, Relaxed);
                    quantize::loops(&fheader, freq, psy, per_gran, bt)
                }
            });

            side.granules[gr][ch] = quant.side.clone();
            let mut sfac = ScaleFactors::default();
            if bt == BlockType::Short {
                // Short layout: scalefactors[3·band + window].
                for b in 0..12 {
                    for w in 0..3 {
                        sfac.short[w][b] = quant.scalefactors[3 * b + w];
                    }
                }
            } else {
                sfac.long.copy_from_slice(&quant.scalefactors[..22]);
            }
            let part2_3_start = main.bit_len();
            prof::time(&prof::HUFF, || {
                bitstream::serialize_scalefactors(&mut main, &fheader, &side, gr, ch, &sfac);
                huffman::encode(&quant, &fheader, &mut main);
            });
            side.granules[gr][ch].part2_3_length = (main.bit_len() - part2_3_start) as u16;
        }
        let main_data = main.finish();
        if quality.is_some() {
            fheader.bitrate_kbps = bitstream::smallest_bitrate_for(&fheader, main_data.len());
            // VBR RATE CEILING. `smallest_bitrate_for` saturates at 320 kbps, so
            // a quality target that demands more bits than the largest legal
            // frame can hold silently produced main data that does not fit —
            // which corrupts `main_data_begin` and makes the stream
            // non-conformant (FFmpeg: "invalid new backstep -1"). A quality
            // target is a request, not a licence to emit an illegal frame: when
            // it cannot be met, fall back to the proven CBR rate loop at the
            // largest budget that fits, and let the frame be as good as 320 kbps
            // allows.
            let mut capped = fheader.clone();
            capped.bitrate_kbps = 320;
            let ceiling = bitstream::region_capacity(&capped);
            if main_data.len() > ceiling {
                let (mut h, s, d) = Self::quantize_frame(fa, ceiling * 8, None);
                h.bitrate_kbps = bitstream::smallest_bitrate_for(&h, d.len());
                return (h, s, d);
            }
        }
        (fheader, side, main_data)
    }

    /// **3R1 LOOKAHEAD** — encode a whole CBR stream with two-pass reservoir allocation:
    /// analyse every frame first (so the perceptual-entropy demand is measured against
    /// the GLOBAL mean, not a lagging causal average), then quantise each to a budget
    /// that lets demanding frames borrow and easy ones bank. Because easy frames are
    /// classified correctly from frame 0, the bank is pre-filled ahead of a transient
    /// instead of only reacting after it. `frames` is `(header, channels-PCM)` per frame.
    pub fn encode_reservoir_lookahead(
        &mut self,
        frames: &[(FrameHeader, Vec<Vec<f32>>)],
        gain: f32,
    ) -> Vec<u8> {
        // Pass 1: analyse every frame (advances state once), collect the PE distribution.
        let analyses: Vec<FrameAnalysis> = frames
            .iter()
            .map(|(h, ch)| self.analyze_frame(h, ch))
            .collect();
        let pe_mean = if analyses.is_empty() {
            1.0
        } else {
            analyses.iter().map(|a| a.frame_pe).sum::<f32>() / analyses.len() as f32
        };

        // Pass 2: quantise in order; demand vs the GLOBAL mean, bank clamps feasibility.
        let mut out = Vec::with_capacity(frames.len());
        let mut bank = 0usize;
        for fa in &analyses {
            let base = bitstream::region_capacity(&fa.fheader) * 8;
            let budget = reservoir_budget(fa.frame_pe, pe_mean, base, bank, gain);
            let (fheader, side, main_data) = Self::quantize_frame(fa, budget, None);
            let used = main_data.len() * 8;
            bank = (bank + base).saturating_sub(used).min(RESV_MAX_BANK);
            out.push((fheader, side, main_data));
        }
        bitstream::assemble_stream(&out)
    }
}

/// One frame's completed analysis (post-M/S, post-block-switch), ready to quantise.
struct FrameAnalysis {
    fheader: FrameHeader,
    analyzed: Vec<([f32; GRANULE_LINES], psychoacoustic::PsyResult, BlockType)>,
    frame_pe: f32,
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    use crate::frame::ChannelMode;
    use crate::header::{FrameHeader, MpegVersion};
    use std::sync::atomic::Ordering::Relaxed;

    fn cbr_header() -> FrameHeader {
        FrameHeader {
            version: MpegVersion::V1,
            crc_protected: false,
            bitrate_kbps: 128,
            sample_rate: 44100,
            padding: false,
            channel_mode: ChannelMode::Mono,
            copyright: false,
            original: true,
            emphasis: 0,
        }
    }

    /// **3R1** — the reservoir path must emit a frame-aligned, sync-valid CBR stream
    /// AND actually borrow (some frame's `main_data_begin > 0`): with energy varying
    /// frame-to-frame, easy frames bank slack that demanding ones reach back into.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn reservoir_stream_is_frame_aligned_and_borrows() {
        let header = cbr_header();
        let fsize = header.frame_size();
        let spf = 2 * GRANULE_LINES;
        let n = 24;
        let mut enc = Mp3Encode::new();
        let mut s = 0x9E37_79B9u32;
        for f in 0..n {
            let loud = f % 3 == 0; // dense every 3rd frame → perceptual-entropy spikes
            let pcm: Vec<f32> = (0..spf)
                .map(|_| {
                    s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let r = (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                    r * if loud { 0.8 } else { 0.02 }
                })
                .collect();
            enc.encode_frame_reservoir(&header, &[pcm], 0.5);
        }
        let stream = enc.finish_reservoir();
        assert_eq!(
            stream.len(),
            n * fsize,
            "reservoir stream must be frame-aligned"
        );
        let mut borrowed = false;
        for f in 0..n {
            let h = &stream[f * fsize..];
            assert_eq!(h[0], 0xFF, "frame {f} lost sync (byte 0)");
            assert_eq!(h[1], 0xFB, "frame {f} bad version/layer (byte 1)");
            // main_data_begin = first 9 bits of side info (bytes 4..6, no CRC).
            let mdb = (u16::from(h[4]) << 1) | u16::from(h[5] >> 7);
            borrowed |= mdb > 0;
        }
        assert!(
            borrowed,
            "reservoir never borrowed — main_data_begin was 0 everywhere"
        );
    }

    /// **3R1 lookahead** — the two-pass path must emit a frame-aligned, sync-valid stream,
    /// AND at gain=0 be byte-identical to the causal path (both flat: budget = base).
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn lookahead_stream_valid_and_flat_matches_causal() {
        let header = cbr_header();
        let fsize = header.frame_size();
        let spf = 2 * GRANULE_LINES;
        let n = 24;
        let mkframes = || {
            let mut s = 0x9E37_79B9u32;
            (0..n)
                .map(|f| {
                    let loud = f % 3 == 0;
                    let pcm: Vec<f32> = (0..spf)
                        .map(|_| {
                            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            let r = (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                            r * if loud { 0.8 } else { 0.02 }
                        })
                        .collect();
                    (header.clone(), vec![pcm])
                })
                .collect::<Vec<_>>()
        };

        let stream = Mp3Encode::new().encode_reservoir_lookahead(&mkframes(), 0.5);
        assert_eq!(
            stream.len(),
            n * fsize,
            "lookahead stream must be frame-aligned"
        );
        for f in 0..n {
            assert_eq!(stream[f * fsize], 0xFF, "frame {f} lost sync");
            assert_eq!(stream[f * fsize + 1], 0xFB, "frame {f} bad version/layer");
        }

        // gain=0 ⇒ every frame gets exactly `base` bits ⇒ identical to the causal path.
        let la0 = Mp3Encode::new().encode_reservoir_lookahead(&mkframes(), 0.0);
        let mut causal = Mp3Encode::new();
        for (h, ch) in &mkframes() {
            causal.encode_frame_reservoir(h, ch, 0.0);
        }
        assert_eq!(
            la0,
            causal.finish_reservoir(),
            "gain=0 lookahead must equal causal"
        );
    }

    /// Profiling driver (run explicitly): encodes ~10 s of *dense* broadband audio
    /// — the case that actually stresses the two-loop quantizer — and prints the
    /// per-stage breakdown. `cargo test -p rff-codec-mp3 profile_encode_dense --
    /// --ignored --nocapture`.
    /// Diagnostic: how many granules of a REAL clip go short vs long? If short
    /// dominates, the psymodel (long-block only) is bypassed — the lever is the
    /// attack detector / short-block shaping, not Floor-1 psymodel tuning.
    /// `cargo test -p rff-codec-mp3 block_mix_real_clip -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs MP3_BLOCK_CLIP pointing at a real WAV"]
    fn block_mix_real_clip() {
        let Ok(path) = std::env::var("MP3_BLOCK_CLIP") else {
            eprintln!("set MP3_BLOCK_CLIP=<f32le mono wav> to run");
            return;
        };
        let d = std::fs::read(&path).expect("MP3_BLOCK_CLIP wav");
        let i = d.windows(4).position(|w| w == b"data").unwrap() + 8;
        let pcm: Vec<f32> = d[i..]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let header = FrameHeader {
            version: MpegVersion::V1,
            crc_protected: false,
            bitrate_kbps: 128,
            sample_rate: 44100,
            padding: false,
            channel_mode: ChannelMode::Mono,
            copyright: false,
            original: true,
            emphasis: 0,
        };
        prof::N_LONG.store(0, std::sync::atomic::Ordering::Relaxed);
        prof::N_SHORT.store(0, std::sync::atomic::Ordering::Relaxed);
        let mut enc = Mp3Encode::new();
        let per = 2 * GRANULE_LINES;
        for ch in pcm.chunks(per) {
            if ch.len() == per {
                enc.encode_frame(&header, &[ch.to_vec()], None).unwrap();
            }
        }
        let (nl, ns) = (prof::N_LONG.load(Relaxed), prof::N_SHORT.load(Relaxed));
        let (k0, ot) = (
            prof::OUTER_KEPT0.load(Relaxed),
            prof::OUTER_TOTAL.load(Relaxed),
        );
        eprintln!(
            "[block-mix] long={nl} short={ns} ({:.0}% short); distortion loop kept iter-0 in {k0}/{ot} long granules ({:.0}%)",
            100.0 * ns as f64 / (nl + ns).max(1) as f64,
            100.0 * k0 as f64 / ot.max(1) as f64
        );
    }

    #[test]
    #[ignore = "manual profiling run (prints the encode stage profile)"]
    fn profile_encode_dense() {
        let header = FrameHeader {
            version: MpegVersion::V1,
            crc_protected: false,
            bitrate_kbps: 128,
            sample_rate: 44100,
            padding: false,
            channel_mode: ChannelMode::Stereo,
            copyright: false,
            original: true,
            emphasis: 0,
        };
        let frames = 380; // ~10 s at 1152 samples/frame
        let per_ch = 2 * GRANULE_LINES; // V1: 2 granules/frame
        let mut enc = Mp3Encode::new();
        let mut s = 0x2545_F491u32;
        let mut rng = || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        // Harmonically dense but *smooth* (no transients) → long blocks, the path
        // real music spends most of its time in and the two-loop quantizer's home.
        let mut n = 0usize;
        for _ in 0..frames {
            let mut l = vec![0f32; per_ch];
            let mut r = vec![0f32; per_ch];
            for i in 0..per_ch {
                let t = n as f32 / 44100.0;
                let mut v = 0f32;
                for k in 1..=28 {
                    v += (1.0 / k as f32)
                        * (2.0 * std::f32::consts::PI * 110.0 * k as f32 * t).sin();
                }
                l[i] = (0.25 * v + 0.02 * rng()).clamp(-1.0, 1.0);
                r[i] = (0.25 * v + 0.02 * rng()).clamp(-1.0, 1.0); // slight decorrelation
                n += 1;
            }
            enc.encode_frame(&header, &[l, r], None).unwrap();
        }
        eprintln!("[profile] {frames} frames (~10 s, harmonic-dense stereo, 128k CBR)");
        prof::dump();
    }
}

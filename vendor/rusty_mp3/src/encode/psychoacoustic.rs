// Modified from rusty_mp3 1.0.0 (see Cargo.toml).
//! Psychoacoustic model — the quality brain (bricks **Q1–Q4**).
//!
//! Estimates, per scalefactor band, how much quantization noise the ear will not
//! hear (the masking threshold). The quantizer's distortion loop (Q6) then shapes
//! noise to sit under it. All constants are computed from the **published**
//! psychoacoustics formulas — Terhardt's absolute threshold of hearing, Zwicker's
//! Bark scale, Schroeder's spreading function — not copied from any encoder.
//!
//! This is a first, untuned model: long blocks only (Q5 block-switching is
//! deferred), a fixed signal-to-mask offset, and the FFT power spectrum mapped to
//! the MDCT scalefactor bands by the `1024/1152` frequency-grid ratio. It captures
//! the dominant effect — noise tolerance proportional to per-band masking, spread
//! across neighbouring bands — which is what the distortion loop needs.

use std::sync::OnceLock;

use crate::frame::{BlockType, SFB_LONG};
use crate::tables;

use super::fft;

/// Psymodel FFT size (Q2). 1024-point, like LAME's long-block analysis.
const N_FFT: usize = 1024;
/// Signal-to-mask offset (dB): the masking energy is lowered by this to get the
/// just-masked noise threshold. A fixed value — tonality-dependent variation (Floor
/// 1A) was MEASURED to give zero corpus-ODG change at CBR (the rate loop dominates
/// the allocation, not the threshold shape), so it was reverted. The lever for these
/// clips was the block-type decision (attack detector), not the threshold values.
const SMR_OFFSET_DB: f32 = 3.0;

/// Multiplier on the band-energy cap. 1.0 is the shipped behaviour; larger
/// loosens the cap so the spread masker decides more bands; a very large value
/// removes it entirely. Read once, like the other calibration knobs.
///
/// Vendored (studio): the `MP3_THR_CAP` override is removed; the shipped value.
fn thr_cap_scale() -> f32 {
    1.0
}

/// The signal-to-mask offset as a linear power ratio.
///
/// Both `10f32.powf` and [`smr_offset_db`] are constant for the whole encode, so
/// computing this per granule was one `powf` per granule for a value that never
/// changes (~1,790 of them on a 24 s clip). `powf` has no SIMD instruction and is
/// among the most expensive scalar ops available; a constant deserves to be
/// computed once.
fn smr_linear() -> f32 {
    static V: OnceLock<f32> = OnceLock::new();
    *V.get_or_init(|| 10f32.powf(-smr_offset_db() / 10.0))
}

/// Calibration override for [`SMR_OFFSET_DB`], via `MP3_SMR_DB`.
///
/// Read ONCE and cached: an env lookup inside the per-granule path would be
/// overhead added in order to take a measurement. Unset (the shipping default)
/// returns the constant, so the production build is unaffected.
///
/// Vendored (studio): the `MP3_SMR_DB` override is removed; the constant.
fn smr_offset_db() -> f32 {
    SMR_OFFSET_DB
}

/// Per-granule perceptual analysis result.
#[derive(Debug, Clone, Default)]
pub struct PsyResult {
    /// Chosen window sequence for this granule.
    pub block_type: BlockType,
    /// Masking threshold (allowed noise energy) per long-block scalefactor band.
    pub thresholds: [f32; SFB_LONG],
    /// Perceptual entropy — the rough bit demand, used by reservoir budgeting.
    pub perceptual_entropy: f32,
    /// Total band energy the thresholds were derived from, in the FFT power
    /// domain they live in.
    ///
    /// The thresholds are NOT directly comparable to MDCT-domain noise: the FFT
    /// is a windowed 1024-point power spectrum, the MDCT is 576 lines with a
    /// different normalisation, and on real content the two scales sit ~4 orders
    /// of magnitude apart. Any ABSOLUTE noise-vs-threshold test has to divide
    /// that out first. (CBR never noticed because it only ranks bands against
    /// each other, where a global scale factor cancels.)
    pub signal_energy: f32,
}

/// Hann analysis window, `0.5·(1 − cos(2πn/N))`.
fn hann() -> &'static [f32; N_FFT] {
    static W: OnceLock<[f32; N_FFT]> = OnceLock::new();
    W.get_or_init(|| {
        let mut w = [0f32; N_FFT];
        for (n, wn) in w.iter_mut().enumerate() {
            *wn = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / N_FFT as f32).cos());
        }
        w
    })
}

/// Zwicker's critical-band rate (Bark) for a frequency in Hz.
fn bark(f: f32) -> f32 {
    13.0 * (0.000_76 * f).atan() + 3.5 * (f / 7500.0).powi(2).atan()
}

/// Schroeder's spreading function (dB) at a Bark distance `dz = bark_maskee −
/// bark_masker`. Peak ≈ 0 dB at the masker; asymmetric (spreads further upward).
fn spreading_db(dz: f32) -> f32 {
    let x = dz + 0.474;
    15.81 + 7.5 * x - 17.5 * (1.0 + x * x).sqrt()
}

/// Terhardt's absolute threshold of hearing (dB SPL) for a frequency in Hz.
fn ath_db(f: f32) -> f32 {
    let k = (f / 1000.0).max(0.01);
    3.64 * k.powf(-0.8) - 6.5 * (-0.6 * (k - 3.3).powi(2)).exp() + 1e-3 * k.powi(4)
}

/// **Prometheus telemetry hooks** (feature-gated, off by default — the whole
/// module compiles out, so the production build is byte-identical). Exposes the
/// psychoacoustic model's *signal-independent* curves so the private Prometheus
/// refinery can harvest them and discover compact closed-form replacements. This
/// is generic instrumentation: it only surfaces functions this file already
/// computes, and reveals nothing about Prometheus itself. See
/// `Prometheus/docs/telemetry-hooks.md`.
#[cfg(feature = "prometheus-telemetry")]
pub mod prometheus {
    /// Schema version of these telemetry records; bump on any signature change.
    pub const TELEMETRY_SCHEMA: u32 = 1;

    /// Terhardt absolute threshold of hearing (dB SPL) at each frequency (Hz).
    /// The canonical C1 discovery target: a 1-variable perceptual curve.
    #[must_use]
    pub fn sample_ath(freqs_hz: &[f32]) -> Vec<f32> {
        freqs_hz.iter().map(|&f| super::ath_db(f)).collect()
    }

    /// Schroeder spreading function (dB) at each Bark distance `dz`.
    #[must_use]
    pub fn sample_spreading(dz: &[f32]) -> Vec<f32> {
        dz.iter().map(|&d| super::spreading_db(d)).collect()
    }

    /// Bark-scale value at each frequency (Hz).
    #[must_use]
    pub fn sample_bark(freqs_hz: &[f32]) -> Vec<f32> {
        freqs_hz.iter().map(|&f| super::bark(f)).collect()
    }
}

/// **SIMD-house masking brick:** the signal-INDEPENDENT band geometry, computed
/// once per sample rate and cached. The Q3 masking loop used to recompute all of
/// this — ~22×22 `spreading_db` (`sqrt`) + `powf`, plus per-band `bark`/`ath`
/// transcendentals — for *every granule*. None of it depends on the signal.
struct BandModel {
    /// FFT-bin range `[lo, hi)` summed for each band's energy.
    bin_lo: [usize; SFB_LONG],
    bin_hi: [usize; SFB_LONG],
    /// Spreading matrix `10^(spreading_db(barkᵢ−barkⱼ)/10)` — the per-granule
    /// masking sum is now `Σⱼ energy[j]·spread[i][j]` (no transcendentals; a dot
    /// product that auto-vectorizes).
    spread: [[f32; SFB_LONG]; SFB_LONG],
    /// `10^(ath_db/10)` per band (scaled by the per-granule `ath_scale` at runtime).
    ath_base: [f32; SFB_LONG],
}

impl BandModel {
    /// Build the band model for `sample_rate` — each value computed by the exact
    /// same f32 formula the Q3 loop used inline, so the result is bit-identical.
    fn new(sample_rate: u32) -> Self {
        let sfb = tables::sfb_long_offsets(sample_rate);
        let bin_per_line = N_FFT as f32 / 1152.0;
        let mut bin_lo = [0usize; SFB_LONG];
        let mut bin_hi = [0usize; SFB_LONG];
        let mut center_bark = [0f32; SFB_LONG];
        let mut ath_base = [0f32; SFB_LONG];
        for b in 0..SFB_LONG {
            bin_lo[b] = (f32::from(sfb[b]) * bin_per_line).round() as usize;
            bin_hi[b] =
                ((f32::from(sfb[b + 1]) * bin_per_line).round() as usize).min(N_FFT / 2 + 1);
            let center_line = (f32::from(sfb[b]) + f32::from(sfb[b + 1])) * 0.5;
            center_bark[b] = bark(center_line * sample_rate as f32 / 1152.0);
            ath_base[b] = 10f32.powf(ath_db(center_line * sample_rate as f32 / 1152.0) / 10.0);
        }
        let mut spread = [[0f32; SFB_LONG]; SFB_LONG];
        for i in 0..SFB_LONG {
            for j in 0..SFB_LONG {
                spread[i][j] = 10f32.powf(spreading_db(center_bark[i] - center_bark[j]) / 10.0);
            }
        }
        Self {
            bin_lo,
            bin_hi,
            spread,
            ath_base,
        }
    }
}

/// The nine MPEG Layer III sample rates -- every rate the encoder accepts.
const MPEG_RATES: [u32; 9] = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000];

/// The band model for one of the nine MPEG rates, built once per process and
/// shared by every thread; `None` for any other rate.
///
/// It used to be a one-slot thread-local holding a `Box::leak`ed model, which
/// leaked a fresh ~2.4 KB model on EVERY change of rate, on every thread (its doc
/// claimed "once per rate"): a host encoding streams of alternating rates grew
/// without bound. Found by the `encode` fuzz target under LeakSanitizer. A static
/// table of nine cells cannot grow and leaks nothing.
fn cached_band_model(sample_rate: u32) -> Option<&'static BandModel> {
    static MODELS: [OnceLock<BandModel>; 9] = [const { OnceLock::new() }; 9];
    let i = MPEG_RATES.iter().position(|&r| r == sample_rate)?;
    Some(MODELS[i].get_or_init(|| BandModel::new(sample_rate)))
}

/// Detect a transient/attack in a granule's PCM: a sub-block whose energy jumps
/// well above the recent running average. Such granules want short blocks so the
/// pre-echo a long window would smear before the attack is confined to one short
/// window. (Brick **Q5** — the block-type trigger; the FSM that turns it into a
/// valid Long/Start/Short/Stop sequence lives in `shortblock`.)
#[must_use]
pub fn detect_attack(pcm: &[f32]) -> bool {
    const BLOCKS: usize = 8;
    // RATIO=10 is corpus-tuned (PEAQ, 2026-07-08): lowering it adds short blocks but
    // does NOT help guitar transients (our short-block path uses flat scalefactors, so
    // more short blocks trade pre-echo for worse steady-state noise) and REGRESSES
    // piano (piano@320k −0.62→−0.87 at RATIO=4). Fix the short-block quantizer before
    // touching this. See [[mp3-encoder-progress]].
    const RATIO: f32 = 10.0;
    let n = pcm.len().min(crate::frame::GRANULE_LINES);
    let bs = n / BLOCKS;
    if bs == 0 {
        return false;
    }
    // `chunks_exact` carries its own length, so the sub-block energies need no
    // per-call range check on `pcm[b * bs..(b + 1) * bs]`.
    let mut blocks = pcm[..n]
        .chunks_exact(bs)
        .map(|c| c.iter().map(|x| x * x).sum::<f32>() / bs as f32)
        .take(BLOCKS);
    // **Quality fix:** seed the running baseline from the FIRST sub-block, not a
    // ~zero constant. The old `running = 1e-6` meant block 0's energy (any audible
    // signal) instantly exceeded `running·RATIO`, so a transient was "detected" in
    // essentially every non-silent granule — 95% short blocks on a ringtone, which
    // then quantize with flat scalefactors and bypass the psymodel. Now an attack is
    // a sub-block that genuinely SPIKES ≥RATIO× above the recent energy.
    let Some(first) = blocks.next() else {
        return false;
    };
    let mut running = first.max(1e-9);
    for e in blocks {
        if e > running * RATIO {
            return true;
        }
        running = (running * 2.0 + e) / 3.0; // smoothed recent energy
    }
    false
}

/// Run the psychoacoustic model over one granule of PCM at `sample_rate`.
pub fn analyze(pcm: &[f32], sample_rate: u32) -> PsyResult {
    let sfb = tables::sfb_long_offsets(sample_rate);
    let win = hann();
    // Signal-independent geometry: cached for the MPEG rates, built on the spot
    // (never leaked) for any other rate a direct caller passes.
    let uncached;
    let model: &BandModel = if let Some(m) = cached_band_model(sample_rate) {
        m
    } else {
        uncached = BandModel::new(sample_rate);
        &uncached
    };

    // Q2 — windowed FFT power spectrum.
    let mut re = [0f32; N_FFT];
    let mut im = [0f32; N_FFT];
    // Zip rather than index: the iterators stop at the shortest of the three,
    // which IS `min(pcm.len(), N_FFT)`, so the bound is carried rather than
    // re-checked per sample.
    for ((r, &p), &w) in re.iter_mut().zip(pcm.iter()).zip(win.iter()) {
        *r = p * w;
    }
    let mut power = [0f32; N_FFT / 2 + 1];
    fft::power_spectrum(&mut re, &mut im, &mut power);

    // Per-band signal energy, over the cached bin ranges.
    let mut energy = [0f32; SFB_LONG];
    for b in 0..SFB_LONG {
        let mut e = 1e-12f32;
        // A slice, not `take().skip()`: one range check instead of a per-element
        // counter pair. The accumulation ORDER is unchanged -- `e` still starts at
        // the floor and takes the bins in the same sequence -- which is what float
        // addition cares about.
        for &p in &power[model.bin_lo[b]..model.bin_hi[b]] {
            e += p;
        }
        energy[b] = e;
    }

    // Q3 — spread energy across Bark (cached matrix → a dot product, no powf),
    // lower by the SMR offset, floor at the ATH.
    let smr = smr_linear();
    let total_energy: f32 = energy.iter().sum();
    let ath_scale = (total_energy / N_FFT as f32).max(1e-9) * 1e-3;
    let mut thresholds = [0f32; SFB_LONG];
    for i in 0..SFB_LONG {
        let row = &model.spread[i];
        let mut masker = 0f32;
        for j in 0..SFB_LONG {
            masker += energy[j] * row[j];
        }
        let ath = model.ath_base[i] * ath_scale;
        // Census: which of the three terms actually decides this band's threshold.
        // A masking model whose output is pinned to the band's own energy for most
        // bands has no discriminating power left -- it is saying "you may put as
        // much noise here as there is signal", i.e. every band may be dropped.
        {
            use std::sync::atomic::Ordering::Relaxed;
            let m = masker * smr;
            if m.max(ath) >= energy[i] {
                crate::encode::prof::THR_CAPPED.fetch_add(1, Relaxed);
            } else if ath > m {
                crate::encode::prof::THR_ATH.fetch_add(1, Relaxed);
            } else {
                crate::encode::prof::THR_MASK.fetch_add(1, Relaxed);
            }
        }
        // Cap at the band's own energy: a band quantized to zero already produces
        // noise = its energy, so a higher threshold means the same thing (drop it)
        // while bounding the wild ATH values in the inaudible top bands.
        // The energy cap ("a band quantized to zero already makes noise equal to
        // its own energy, so a higher threshold means the same thing") is sound as
        // a CAP, but it binds on 37-48% of bands on real music -- and where it
        // binds it sets threshold = energy, so the band's noise-to-mask ratio
        // becomes 1/SNR, a signal-independent quantity. That flattens the masking
        // SHAPE on nearly half the spectrum, and shape is the half of the model
        // that demonstrably matters: ranking by noise-to-mask beats ranking by raw
        // noise by +0.030 ODG, while a global level offset is inert because it
        // cancels out of a ranking. `MP3_THR_CAP` scales the cap so its cost can
        // be measured.
        let cap = energy[i] * thr_cap_scale();
        thresholds[i] = (masker * smr).max(ath).min(cap);
    }

    // Q4 — perceptual entropy: rough bit demand from the signal/threshold ratio.
    let mut pe = 0f32;
    for b in 0..SFB_LONG {
        let lines = f32::from(sfb[b + 1] - sfb[b]);
        pe += lines * (1.0 + energy[b] / thresholds[b]).log2().max(0.0);
    }

    PsyResult {
        block_type: BlockType::Long, // Q5 (block switching) deferred
        thresholds,
        perceptual_entropy: pe,
        signal_energy: total_energy,
    }
}

#[cfg(test)]
mod tests {

    /// The band-model cache must not grow with rate changes: one model per MPEG
    /// rate, the SAME allocation however often the rate alternates and from any
    /// thread (the old one-slot thread-local leaked a new model per change).
    #[test]
    fn band_model_cache_is_one_shared_model_per_rate() {
        let first = std::ptr::from_ref::<BandModel>(cached_band_model(44_100).unwrap());
        for _ in 0..100 {
            let _ = cached_band_model(48_000).unwrap();
            assert!(std::ptr::eq(first, cached_band_model(44_100).unwrap()));
        }
        let from_thread = std::thread::spawn(|| {
            std::ptr::from_ref::<BandModel>(cached_band_model(44_100).unwrap()) as usize
        })
        .join()
        .unwrap();
        assert_eq!(from_thread, first as usize);
        assert!(cached_band_model(7_999).is_none());
        // An uncached rate still analyses (built on the spot, then dropped).
        let psy = analyze(&[0.25f32; 1152], 7_999);
        assert!(psy.perceptual_entropy.is_finite());
    }

    use super::*;

    #[test]
    fn bark_is_monotonic_and_spans_critical_bands() {
        assert!(bark(100.0) < bark(1000.0) && bark(1000.0) < bark(10000.0));
        assert!(
            (bark(1000.0) - 8.5).abs() < 1.5,
            "1 kHz ≈ 8.5 Bark, got {}",
            bark(1000.0)
        );
    }

    #[test]
    fn ath_has_a_minimum_in_the_ear_canal_band() {
        // Hearing is most sensitive ~3–4 kHz (ATH near its minimum, ~ -5 dB).
        assert!(ath_db(3500.0) < ath_db(200.0));
        assert!(ath_db(3500.0) < ath_db(15000.0));
    }

    #[test]
    fn spreading_peaks_at_zero_and_decays() {
        assert!(spreading_db(0.0).abs() < 0.5);
        assert!(spreading_db(2.0) < -3.0);
        assert!(spreading_db(-2.0) < -3.0);
        // Asymmetric: a masker spreads further toward higher frequencies (dz>0).
        assert!(spreading_db(1.5) > spreading_db(-1.5));
    }

    /// **Masking-brick gate.** The cached band model must hold the EXACT values the
    /// Q3 loop used to compute inline (same f32 formulas) — so precomputing them
    /// leaves the thresholds, and thus the output, bit-identical.
    #[test]
    fn band_model_matches_inline() {
        for &sr in &[44100u32, 48000, 32000, 24000] {
            let sfb = tables::sfb_long_offsets(sr);
            let m = BandModel::new(sr);
            let bin_per_line = N_FFT as f32 / 1152.0;
            let mut cb = [0f32; SFB_LONG];
            for b in 0..SFB_LONG {
                let lo = (f32::from(sfb[b]) * bin_per_line).round() as usize;
                let hi =
                    ((f32::from(sfb[b + 1]) * bin_per_line).round() as usize).min(N_FFT / 2 + 1);
                assert_eq!(m.bin_lo[b], lo, "bin_lo b{b} sr{sr}");
                assert_eq!(m.bin_hi[b], hi, "bin_hi b{b} sr{sr}");
                let center_line = (f32::from(sfb[b]) + f32::from(sfb[b + 1])) * 0.5;
                cb[b] = bark(center_line * sr as f32 / 1152.0);
                let ath = 10f32.powf(ath_db(center_line * sr as f32 / 1152.0) / 10.0);
                assert_eq!(m.ath_base[b], ath, "ath_base b{b} sr{sr}");
            }
            for i in 0..SFB_LONG {
                for j in 0..SFB_LONG {
                    let s = 10f32.powf(spreading_db(cb[i] - cb[j]) / 10.0);
                    assert_eq!(m.spread[i][j], s, "spread[{i}][{j}] sr{sr}");
                }
            }
        }
    }

    #[test]
    fn tone_thresholds_are_bounded_by_signal_energy() {
        // A 1 kHz tone: every band's threshold sits below the loudest band's
        // energy (masking can't exceed the masker) and stays finite/positive.
        let sr = 44100;
        let pcm: Vec<f32> = (0..N_FFT)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sr as f32).sin())
            .collect();
        let psy = analyze(&pcm, sr);
        let peak_energy = {
            // recompute band energies the same way for the bound
            let sfb = tables::sfb_long_offsets(sr);
            let win = hann();
            let mut re = [0f32; N_FFT];
            let mut im = [0f32; N_FFT];
            for i in 0..N_FFT {
                re[i] = pcm[i] * win[i];
            }
            let mut p = [0f32; N_FFT / 2 + 1];
            fft::power_spectrum(&mut re, &mut im, &mut p);
            let mut m = 0f32;
            for b in 0..SFB_LONG {
                let lo = (f32::from(sfb[b]) * N_FFT as f32 / 1152.0).round() as usize;
                let hi = ((f32::from(sfb[b + 1]) * N_FFT as f32 / 1152.0).round() as usize)
                    .min(N_FFT / 2 + 1);
                let e: f32 = p[lo..hi].iter().sum();
                m = m.max(e);
            }
            m
        };
        for (b, &t) in psy.thresholds.iter().enumerate() {
            assert!(t.is_finite() && t > 0.0, "band {b} threshold {t}");
            assert!(
                t <= peak_energy * 1.01,
                "band {b} threshold {t} > peak {peak_energy}"
            );
        }
        assert!(psy.perceptual_entropy > 0.0);
    }
}

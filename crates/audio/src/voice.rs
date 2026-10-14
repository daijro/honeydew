//! The LFO laws and tempo tables shared by the instruments. MiniSynth keeps its own in `minisynth.rs`.

/// The DW Sampler and SuperSaw free (Hertz) LFO rate: 1/(30·(1 − v)³ + 0.033333) Hz. It reproduces the DW Sampler's
/// rate curve at v = 0, .05 … 1 to 2 decimals.
pub fn lfo_rate_hz(v: f32) -> f32 {
    1.0 / (30.0 * (1.0 - v.clamp(0.0, 1.0)).powi(3) + 0.033333)
}

/// The DW Sampler LFO Attack (p33): the fade-in rises by this much per sample, so it lasts
/// `1/(c·Fs)` seconds. v < 0.001 → 1 (instant), else 0.0002 / (500·(0.95v + 0.05)⁴).
pub fn lfo_attack_increment(v: f32) -> f32 {
    if v < 0.001 {
        1.0
    } else {
        0.0002 / (500.0 * 0.95f32.mul_add(v, 0.05).powi(4))
    }
}

// Tempo-step menus as periods in beats, in menu order: the DW Sampler (Tmp Sync
// periods 1/8 … 32/1 = 0.125 … 32 beats), GMSynth (LFO beats {0.25 … 32} = int(9·v32); FX Sync R, the
// same list) and SuperSaw (LFO {1/8, 1/4, 3/4, 1/2, 1 … 32}[int(9·v42)], Delay {1/4 … 1}[int(4·v54)], Arp
// {1/8 … 1}[int(5·v47)]). MiniSynth keeps its own list (`minisynth.rs`).

/// DW Sampler LFO Sync Rate (p30): 1/8, 1/4, 1/2, 3/4, 1/1, 2/1, 4/1, 8/1, 16/1, 32/1.
pub const LFO_SYNC_BEATS: [f32; 10] = [0.125, 0.25, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
/// SuperSaw LFO Beat (p42), captured with 3/4 before 1/2.
pub const SUPERSAW_LFO_BEATS: [f32; 10] = [0.125, 0.25, 0.75, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
/// GMSynth LFO Rate and FX 1–3 Sync (p32, p48, p55, p62): 1/4, 3/8, 1/2, 3/4, Beat, 2 Beats, Bar … 8 Bars.
pub const GMSYNTH_RATE_BEATS: [f32; 10] = [0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
/// SuperSaw ARP Rate (p47): 1/8, 1/4, 3/8, 1/2, 3/4, 1/1.
pub const ARP_RATE_BEATS: [f32; 6] = [0.125, 0.25, 0.375, 0.5, 0.75, 1.0];
/// SuperSaw Delay Time (p54): 1/4, 3/8, 1/2, 3/4, 1/1.
pub const SUPERSAW_DELAY_BEATS: [f32; 5] = [0.25, 0.375, 0.5, 0.75, 1.0];

/// The step a menu value picks in the DSP: `list[int(v·(n − 1))]`, truncated (the display rounds).
pub fn tempo_step(list: &[f32], v: f32) -> f32 {
    list[(v.clamp(0.0, 1.0) * (list.len() - 1) as f32) as usize]
}

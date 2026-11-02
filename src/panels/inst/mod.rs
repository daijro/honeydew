//! Instrument panels, one file per instrument type, laid out like the effect files
//! (`fx/`).

mod dw_sampler;
mod gm_synth;
mod mini_synth;
pub(crate) mod slicer;
pub(crate) mod soundfont_player;
mod super_saw;
pub(crate) mod three_osc;
mod transistor_bass;

modules! {
    0 => mini_synth,
    1 => dw_sampler,
    2 => gm_synth,
    19 => transistor_bass,
    23 => super_saw,
    33 => slicer,
    39 => three_osc,
    52 => soundfont_player,
}

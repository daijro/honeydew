//! Effect and note-effect panels, one file per module. Each file has `panel()` and
//! `REFERENCE`, the section table's rects for the data-driven test.

pub(super) use pitcher_2::derive_scale;
pub(crate) use scale::couple_scale;
pub(crate) use tremolo::couple_param;

use std::sync::Arc;

use eframe::egui::{Id, Ui};
use honeydew_audio::MeterSnapshot;

mod analyzer;
mod arpeggiator;
mod autoduck;
mod autoduck_legacy;
mod chorus;
mod comb_filter;
mod compressor;
mod distortion;
mod equalizer;
mod filter;
mod flanger;
mod gate;
mod graphic_eq;
mod leveller;
mod limiter;
mod limiter_legacy;
mod multi_fx;
mod multiband;
mod note_echo;
mod phaser;
mod pitcher;
mod pitcher_2;
mod randomizer;
mod reverb;
mod reverb_2;
mod reverb_legacy;
mod scale;
mod spacer;
mod spreader;
mod stereoizer;
mod tape_delay;
mod tape_stop;
mod trance_delay;
mod transpose;
mod tremolo;
mod tuned_eq;
mod tuner;
mod waveshaper;
mod wow_flutter;

modules! {
    3 => equalizer,
    4 => multi_fx,
    5 => limiter_legacy,
    6 => distortion,
    7 => graphic_eq,
    8 => compressor,
    9 => filter,
    10 => autoduck_legacy,
    11 => chorus,
    12 => reverb_legacy,
    13 => flanger,
    14 => phaser,
    15 => stereoizer,
    16 => trance_delay,
    18 => tape_delay,
    20 => leveller,
    24 => tuner,
    25 => pitcher,
    26 => analyzer,
    27 => gate,
    28 => limiter,
    29 => autoduck,
    30 => tuned_eq,
    32 => reverb,
    34 => spacer,
    35 => reverb_2,
    36 => multiband,
    40 => comb_filter,
    41 => waveshaper,
    42 => tape_stop,
    43 => wow_flutter,
    44 => spreader,
    45 => pitcher_2,
    46 => tremolo,
    47 => note_echo,
    48 => transpose,
    49 => randomizer,
    50 => scale,
    51 => arpeggiator,
}

/// Where the rack posts a module's latest meter (a [`MeterSnapshot`]) for its panel's draws, in egui temp data: under
/// the id of the `ui` the panel draws in. It lives with the panels, below the rack, so they need not reach up into it.
fn meter_id(ui: &Ui, instance_id: i32) -> Id {
    ui.id().with(("module meter", instance_id))
}

/// Where the rack posts the module's analysis (`honeydew_audio::analysis`), beside its meter.
fn analysis_id(ui: &Ui, instance_id: i32) -> Id {
    meter_id(ui, instance_id).with("analysis")
}

/// The rack's hand-off to the panel draws: module `instance_id`'s meter and analysis this frame, or `None` without a
/// running stream, so the draws rest and stop polling.
pub fn post_feed(ui: &Ui, instance_id: i32, feed: Option<(MeterSnapshot, &[f32])>) {
    let (meter, analysis) = (meter_id(ui, instance_id), analysis_id(ui, instance_id));
    ui.data_mut(|d| match feed {
        Some((value, slab)) => {
            d.insert_temp(meter, value);
            d.insert_temp(analysis, Arc::<[f32]>::from(slab));
        }
        None => {
            d.remove_temp::<MeterSnapshot>(meter);
            d.remove_temp::<Arc<[f32]>>(analysis);
        }
    });
}

/// The module's analysis as the rack posted it ([`post_feed`]); `None` outside a rack or without a running stream.
/// While there is one, the next frame comes within 50 ms, as for [`leveller::live_meter`]: the audio moves with no
/// input to repaint for.
pub(super) fn live_analysis(ui: &Ui, instance_id: i32) -> Option<Arc<[f32]>> {
    let slab = ui.data(|d| d.get_temp(analysis_id(ui, instance_id)));
    if slab.is_some() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
    slab
}

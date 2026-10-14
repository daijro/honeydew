//! The Slicer panel (type 33), card 8.5 U, no tab strip.
//! Ids: the control's `#n` number in four digits (`3.6` is 3600).
//!
//! - The card wiring ([`wire`]) sets `sample_loaded` while the module's `CSTM` `PATH` names a loop:
//!   The waveform then takes the `LOAD SAMPLE` button's slot.
//! - `LOAD SAMPLE`, or a tap on the waveform (the button is hidden once a loop is loaded), asks for the
//!   module's sample picker ([`asks_for_loop`]). `AUTO` asks for AUTO's clip once ([`auto_pressed`]).
//! - The waveform ([`waveform`]), a graph display: the controlFill rect, the loop on top in the channel colour.
//!   The rack hands it the loop the sample cache holds, with its min/max summary, made once per loop ([`post_loop`]).
//!
//! Details:
//! - The loop is drawn whole across the display by the audio clip editor's waveform law: one column a device pixel
//!   from min/max peaks, mono centred, stereo in halves, a column under a pixel a pixel line. No slice markers.
//! - `sample_loaded` follows `PATH`, so a loop that is missing shows an empty waveform, which a tap replaces.

use std::sync::{Arc, Weak};

use eframe::egui::{Color32, Context, Id, Mesh, Rect, Sense, Ui};
use honeydew_audio::sample::SampleData;
use honeydew_audio::sample_edit::peaks;

use crate::audio_engine::slicer::loop_path;
use crate::flm::chunk::ContentPath;
use crate::panels::build::*;
use crate::params::Display;
use crate::sample_cache::{SampleCache, SampleKey, Waveform};
use crate::ui::UiCtx;
use crate::ui::audio_editor::{add_column, pixel_columns, wave_rows};

/// The Slicer's module type.
pub const TYPE_ID: u16 = 33;
/// `LOAD SAMPLE`.
const LOAD: u32 = 1000;
/// The momentary `AUTO`.
const AUTO: u32 = 3700;
/// The waveform.
const WAVE: u32 = 2000;

/// The card wiring: `sample_loaded` while `module`'s `CSTM` `PATH` names a loop.
pub fn wire(module: &ModuleInstance, state: &mut PanelState) {
    let loaded = loop_path(module).is_some();
    state.flags.insert(flag::SAMPLE_LOADED, usize::from(loaded));
}

/// Whether the user asked for a loop this pass: `LOAD SAMPLE` pressed (`edited`, the controls without a param edited,
/// [`crate::panels::Edits::settings`]), or a tap on the waveform in the card's `content` rect (`u` per U, in its units;
/// `id` is the tap's).
pub fn asks_for_loop(
    ui: &Ui,
    id: Id,
    module: &ModuleInstance,
    state: &PanelState,
    content: Rect,
    u: f32,
    edited: &[u32],
) -> bool {
    if pressed(state, edited, LOAD) {
        return true;
    }
    let Some(panel) = crate::panels::panel_for(TYPE_ID) else {
        return false;
    };
    let placed = crate::panels::place(panel, module, state, content, u);
    let wave = placed.into_iter().find(|(l, _)| l.id == WAVE);
    wave.is_some_and(|(_, r)| ui.interact(r, id, Sense::click()).clicked())
}

/// Whether `AUTO` went down this pass (`edited` as [`asks_for_loop`]'s): the wiring rewrites the current clip with
/// The slices once, whatever the `SENS` AUTO flag says .
pub fn auto_pressed(state: &PanelState, edited: &[u32]) -> bool {
    pressed(state, edited, AUTO)
}

/// Whether the momentary button `n` went down this pass.
fn pressed(state: &PanelState, edited: &[u32], n: u32) -> bool {
    edited.contains(&n) && state.settings.get(&n).is_some_and(|&v| v >= 0.5)
}

/// Where the rack posts a loop for the waveforms that draw it.
fn loop_id(path: &ContentPath) -> Id {
    Id::new(("slicer loop", path))
}

/// A posted loop: its frames, kept weakly so a loop the cache drops is freed, and its summary ([`Waveform`]).
#[derive(Clone)]
struct Posted {
    data: Weak<SampleData>,
    summary: Arc<[Vec<(f32, f32)>]>,
}

/// Hands the waveform of the Slicer `module` (another module is skipped) the loop its `PATH` names, as `samples` holds
/// it: the rack calls it each frame before drawing the card. The summary is made when the loop is new.
pub fn post_loop(ctx: &Context, module: &ModuleInstance, samples: &SampleCache) {
    let Some(path) = loop_path(module) else {
        return;
    };
    let sample = samples.get(&SampleKey::of_path(&path));
    let id = loop_id(&path);
    let posted = ctx.data(|d| d.get_temp::<Posted>(id));
    if sample.is_none() && posted.is_some() {
        return;
    }
    let ptr = sample.as_ref().map_or(std::ptr::null(), Arc::as_ptr);
    if posted.is_some_and(|p| p.data.as_ptr() == ptr) {
        return;
    }
    let posted = match sample {
        Some(sample) => {
            let w = Waveform::new(sample);
            Posted {
                data: Arc::downgrade(&w.data),
                summary: w.summary,
            }
        }
        None => Posted {
            data: Weak::new(),
            summary: Arc::new([]),
        },
    };
    ctx.data_mut(|d| d.insert_temp(id, posted));
}

/// The waveform display (see the module docs). Nothing to edit.
fn waveform(
    ui: &mut Ui,
    cx: &UiCtx,
    _: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, cx.theme.colors.control_fill.to_color32());
    let posted = loop_path(module).and_then(|p| ui.data(|d| d.get_temp::<Posted>(loop_id(&p))));
    let Some((sample, posted)) = posted.and_then(|p| Some((p.data.upgrade()?, p))) else {
        return Vec::new();
    };
    let (x0, width) = pixel_columns(rect, cx.m);
    let px = 1.0 / cx.m.ppp();
    let per_column = sample.frames() as f64 / width.max(1) as f64;
    let channels = (std::iter::once(&sample.left).chain(&sample.right)).zip(posted.summary.iter());
    let mut mesh = Mesh::default();
    for ((samples, summary), row) in channels.zip(wave_rows(rect, sample.right.is_some())) {
        for (i, peak) in peaks(samples, summary, per_column, 0.0, width)
            .into_iter()
            .enumerate()
        {
            add_column(&mut mesh, x0 + i as f32 * px, peak, row, px, ink);
        }
    }
    painter.add(mesh);
    Vec::new()
}

/// The `LOAD SAMPLE` button until a loop is loaded, then the waveform in its slot.
const LOADED: Show = Show::Flag(flag::SAMPLE_LOADED, 1);

/// The `Sensitivity` knob has no param: the generic `v × 100`, truncated; 0 shows `M`, the file's markers.
fn sensitivity_text(v: f32) -> String {
    if v <= 0.0 {
        "M".to_owned()
    } else {
        Display::Generic.text(v)
    }
}

pub fn panel() -> Option<Panel> {
    Some(Panel::new(vstack([
        text_toggle(LOAD, "LOAD SAMPLE")
            .momentary()
            .mr(0.5)
            .ml(0.5)
            .show(Show::Not(&LOADED)),
        display(WAVE).mr(0.5).ml(0.5).show(LOADED).draw(waveform),
        hstack([
            // The slice detector's sensitivity: CSTM `SENS`, 0.25 for a new module.
            knob(3100, "Sensitivity")
                .default(0.25)
                .value_fn(sensitivity_text),
            knob(3200, "Pitch").p(1),
            knob(3300, "Attack").p(3),
            knob(3400, "Decay").p(4),
            spinner(3500, "Sustain").p(5),
            text_toggle(3600, "MONO").p(6).mt(0.5),
            text_toggle(AUTO, "AUTO").momentary().mt(0.5),
        ])
        .h(Size::U(2.75)),
    ])))
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [16.96, 6.49],
    settings: &[(1000, 0.0), (3100, 0.25), (3700, 0.0)],
    cases: &[
        Case {
            name: "no sample",
            rows: &[
                row(
                    1000,
                    Widget::TextToggle,
                    "LOAD SAMPLE",
                    [0.50, 0.00, 15.96, 3.74],
                )
                .margins(0.0, 0.5, 0.0, 0.5)
                .momentary(),
                row(3100, Widget::Knob, "Sensitivity", [0.00, 3.74, 2.42, 2.75]).value("25"),
                row(3200, Widget::Knob, "Pitch", [2.42, 3.74, 2.42, 2.75]).p(1),
                row(3300, Widget::Knob, "Attack", [4.84, 3.74, 2.42, 2.75]).p(3),
                row(3400, Widget::Knob, "Decay", [7.26, 3.74, 2.43, 2.75]).p(4),
                row(3500, Widget::Spinner, "Sustain", [9.69, 3.74, 2.42, 2.75]).p(5),
                row(3600, Widget::TextToggle, "MONO", [12.11, 4.24, 2.42, 2.25])
                    .p(6)
                    .margins(0.5, 0.0, 0.0, 0.0),
                row(3700, Widget::TextToggle, "AUTO", [14.53, 4.24, 2.43, 2.25])
                    .margins(0.5, 0.0, 0.0, 0.0)
                    .momentary(),
            ],
            ..Case::DEFAULT
        },
        // The waveform takes the button's slot.
        Case {
            name: "sample loaded",
            flags: &[(flag::SAMPLE_LOADED, 1)],
            rows: &[
                row(2000, Widget::Display, "", [0.50, 0.00, 15.96, 3.74])
                    .margins(0.0, 0.5, 0.0, 0.5),
                row(3100, Widget::Knob, "Sensitivity", [0.00, 3.74, 2.42, 2.75]).value("25"),
                row(3200, Widget::Knob, "Pitch", [2.42, 3.74, 2.42, 2.75]).p(1),
                row(3300, Widget::Knob, "Attack", [4.84, 3.74, 2.42, 2.75]).p(3),
                row(3400, Widget::Knob, "Decay", [7.26, 3.74, 2.43, 2.75]).p(4),
                row(3500, Widget::Spinner, "Sustain", [9.69, 3.74, 2.42, 2.75]).p(5),
                row(3600, Widget::TextToggle, "MONO", [12.11, 4.24, 2.42, 2.25])
                    .p(6)
                    .margins(0.5, 0.0, 0.0, 0.0),
                row(3700, Widget::TextToggle, "AUTO", [14.53, 4.24, 2.43, 2.25])
                    .margins(0.5, 0.0, 0.0, 0.0)
                    .momentary(),
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::flm::chunk::{ChunkWriter, ContentRoot};

    /// Sensitivity 0 shows `M`, the file's markers.
    #[test]
    fn sensitivity_0_shows_m() {
        let panel = panel().unwrap();
        let leaves = panel.root.leaves();
        let knob = leaves.iter().find(|leaf| leaf.id == 3100).unwrap();
        let text = knob.value_fn.unwrap();
        assert_eq!([text(0.0), text(0.25), text(1.0)], ["M", "25", "100"]);
    }

    #[test]
    fn the_waveform_shows_while_the_cstm_names_a_loop() {
        let mut state = PanelState::new(&panel().unwrap());
        let mut module = ModuleInstance::new(TYPE_ID, 2);
        let loaded = |module: &ModuleInstance, state: &mut PanelState| {
            wire(module, state);
            state.flags.get(flag::SAMPLE_LOADED).copied()
        };
        assert_eq!(loaded(&module, &mut state), Some(0));
        // `SENS` alone names no loop.
        let mut sens = ChunkWriter::default();
        sens.chunk(*b"SENS", |w| w.raw(&[0, 0, 0, 0, 1])).unwrap();
        module.file.custom_data = Some(sens.into_bytes());
        assert_eq!(loaded(&module, &mut state), Some(0));
        let path = ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "Built-in/Loops/Dust Break 84.wav".into(),
        };
        crate::project_files::load_pick(&mut module, Path::new("a.wav"), &[], Some(path)).unwrap();
        assert_eq!(loaded(&module, &mut state), Some(1));
    }

    #[test]
    fn a_posted_loop_is_summarized_once_and_held_weakly() {
        let mut song = crate::song::Song::default();
        song.add_channel(crate::song::ChannelKind::Synth(
            crate::song::SynthKind::Slicer,
        ));
        let module = &mut song.channels.last_mut().unwrap().rack.modules[0];
        let path = ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "Built-in/Loops/Relay Roll 140.wav".into(),
        };
        crate::project_files::load_pick(module, Path::new("a.wav"), &[], Some(path.clone()))
            .unwrap();
        let module = module.clone();
        let mut samples = SampleCache::default();
        samples.load(&song, &|_| None);
        let ctx = Context::default();
        let posted_in = |ctx: &Context| ctx.data(|d| d.get_temp::<Posted>(loop_id(&path))).unwrap();
        let posted = || posted_in(&ctx);
        post_loop(&ctx, &module, &samples);
        let first = posted();
        assert_eq!(first.summary.len(), 1, "mono");
        post_loop(&ctx, &module, &samples);
        assert!(Arc::ptr_eq(&first.summary, &posted().summary));
        // The cache drops it (a rewritten file, a new song's cache): the waveform lets it go.
        samples.forget(&SampleKey::of_path(&path));
        assert!(first.data.upgrade().is_none());
        // A missing loop posts nothing new: the old entry draws nothing.
        post_loop(&ctx, &module, &samples);
        assert!(Arc::ptr_eq(&first.summary, &posted().summary));
        // Nor does a loop that was never there.
        let ctx = Context::default();
        post_loop(&ctx, &module, &samples);
        assert!(posted_in(&ctx).summary.is_empty());
        post_loop(&ctx, &module, &samples);
        assert!(posted_in(&ctx).data.upgrade().is_none());
    }
}

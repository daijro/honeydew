//! Lemon Studio's own plugins built into the app, on every platform (Lemon Trio and Lemon
//! Sampler, instruments, and Lemon Parametric EQ, an effect, from `vendor/lemon-studio`). As in Lemon Studio, each is
//! compiled in (`linked`: no exported entry points) and hosted through Lemon Studio's CLAP host from its CLAP entry
//! table, so a module of one is a plugin module like any other (types 1000 and 1001, saved in `HDPm`), only always
//! installed. Their editors draw into the rack cards (`embed`, `crate::ui::rack::lemon_card`).
//!
//! Adding one is its crate in [`lemon_plugins!`]'s list and its dependency line in Cargo.toml
//! (features `linked` and `embed`). Everything else is read from the plugin, once: at the first use its name, CLAP id
//! and kind from its CLAP descriptor (no instance), which the picker lists in its LEMON STUDIO section
//! (`crate::ui::modules_browser`); its editor size from CLAP's `gui.get_size` only when a card of it first shows
//! ([`Builtin::size`]). Only its VST3 copy's class id is not there ([`VST3_CLASSES`]).
//!
//! A plugin installed on its own as well (the same CLAP id, or its VST3 copy's class id) is not listed, and a module
//! naming that copy plays the built-in one.

use std::ffi::c_void;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use os_host::{Descriptor, Library};

use crate::song::plugin::PluginRef;

/// The built-in plugins' CLAP entry tables, in listing order: one Lemon Studio plugin crate each, its
/// `LINKED_CLAP_ENTRY` (`os_plugkit::linked_clap_entry!`).
macro_rules! lemon_plugins {
    ($($krate:ident),+ $(,)?) => {
        [$(&$krate::LINKED_CLAP_ENTRY as *const _ as *const c_void),+]
    };
}

const ENTRIES: &[*const c_void] = &lemon_plugins![
    lemon_3x_osc,
    lemon_sampler,
    lemon_parametric_eq_2,
    lemon_formula_controller,
    lemon_peak_controller,
    tesla,
    citrus,
    lemon_drum_sampler,
    lemon_slicer,
    lemon_soft_clipper,
    lemon_fast_distortion,
    lemon_balance,
    lemon_stereo_enhancer,
    lemon_transient_processor,
    lemon_chorus,
    lemon_stereo_shaper,
    lemon_filter,
    lemon_flanger,
    lemon_phaser,
    lemon_delay,
    lemon_vocoder,
    press,
    pulp_filter,
];

/// Each one's VST3 copy's class id, by CLAP id: upstream's `Vst3Plugin::VST3_CLASS_ID` (`plugins/*/src/lib.rs`), which
/// the `linked` build leaves out. A plugin not here has no VST3 copy to know.
const VST3_CLASSES: &[(&str, &[u8; 16])] = &[
    ("org.openstudio.open-3x-osc", b"OpenStudio3xOsc!"),
    ("org.openstudio.open-sampler", b"OpenStudioSamplr"),
    ("org.openstudio.open-parametric-eq-2", b"OpenStudioParEQ2"),
    ("org.openstudio.lemon-soft-clipper", b"LemonStudioClipr"),
    ("org.openstudio.lemon-fast-distortion", b"LemonStudioFDist"),
    ("org.openstudio.lemon-balance", b"LemonStudioBalnc"),
    ("org.openstudio.lemon-stereo-enhancer", b"LemonStudioStEnh"),
    (
        "org.openstudio.lemon-transient-processor",
        b"LemonStudioTrPro",
    ),
    ("org.openstudio.lemon-chorus", b"LemonStudioChrus"),
    ("org.openstudio.lemon-stereo-shaper", b"LemonStudioStShp"),
    ("org.openstudio.lemon-filter", b"LemonStudioFiltr"),
    ("org.openstudio.lemon-flanger", b"LemonStudioFlngr"),
    ("org.openstudio.lemon-phaser", b"LemonStudioPhasr"),
    ("org.openstudio.lemon-delay", b"LemonStudioDelay"),
    ("org.openstudio.lemon-vocoder", b"LemonStudioVocdr"),
    ("org.openstudio.press", b"LemonStudioPress"),
    ("org.openstudio.pulp-filter", b"LemonStudioPulpF"),
    ("org.openstudio.citrus", b"LemonStudioCitrs"),
    ("org.openstudio.lemon-drum-sampler", b"LemonStudioDrums"),
    ("org.openstudio.lemon-slicer", b"LemonStudioSlicr"),
];

/// The editor size of a plugin that reports none.
const FALLBACK_SIZE: (u32, u32) = (640, 480);

/// One built-in plugin.
pub struct Builtin {
    pub name: &'static str,
    /// `clap:` and its CLAP plugin id, which its modules save: its identity in plugin lists.
    pub uid: &'static str,
    /// Its VST3 copy's class id, by which an installed copy is known.
    vst3_class: Option<&'static [u8; 16]>,
    /// [`Builtin::size`], once read.
    size: OnceLock<(u32, u32)>,
    /// [`Builtin::sidechain`], once read.
    keyed: OnceLock<bool>,
    /// The editor id [`Builtin::size`] opens its editor at: never a card's, which count up from 1
    /// ([`super::lemon_editor`]).
    probe: u32,
    /// Its CLAP descriptor: kind, features.
    pub desc: Descriptor,
    pub(super) lib: Arc<dyn Library>,
}

impl Builtin {
    /// Its CLAP plugin id.
    pub fn clap_id(&self) -> &'static str {
        &self.uid[CLAP.len()..]
    }

    pub fn instrument(&self) -> bool {
        self.desc.is_instrument()
    }

    /// Its editor's size in its own pixels: a card shows it at the card's width, keeping this aspect. Read at the first
    /// call (a card of it shown; the UI thread's), from an instance that opens its editor once, unseen.
    pub fn size(&self) -> (u32, u32) {
        *(self.size).get_or_init(|| {
            editor_size(&self.lib, &self.desc.id, self.probe).unwrap_or(FALLBACK_SIZE)
        })
    }

    /// Whether it has a sidechain input (Lemon Vocoder's carrier), which a sidechain feeds
    /// ([`crate::song::ModuleInstance::takes_key`]). Read at the first call, from an instance made to ask
    /// (`os_host::Plugin::sidechain`).
    pub fn sidechain(&self) -> bool {
        *(self.keyed).get_or_init(|| {
            let plugin = self.lib.clone().create(&self.desc.id).ok();
            plugin.is_some_and(|p| p.lock().is_ok_and(|p| p.sidechain()))
        })
    }

    /// Whether a plugin of `format` with `id` is this one (its CLAP id, or its VST3 copy's class id).
    fn is(&self, format: &str, id: &str) -> bool {
        match (format, self.vst3_class) {
            ("clap", _) => id.eq_ignore_ascii_case(self.clap_id()),
            ("vst3", Some(class)) => id.eq_ignore_ascii_case(
                &class.iter().map(|b| format!("{b:02X}")).collect::<String>(),
            ),
            _ => false,
        }
    }
}

/// A uid's format part.
const CLAP: &str = "clap:";

/// The built-in plugins, read from their CLAP entries' descriptors at the first call (the UI thread's, never the audio
/// thread's).
pub fn all() -> &'static [Builtin] {
    static ALL: OnceLock<Vec<Builtin>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut all = Vec::new();
        for &entry in ENTRIES {
            // SAFETY: the entry is a linked plugin's static CLAP entry table.
            let lib: Arc<dyn Library> =
                match unsafe { os_clap::Library::from_entry(entry, Path::new("")) } {
                    Ok(lib) => lib,
                    Err(e) => {
                        eprintln!("plugins: a built-in plugin: {e}");
                        continue;
                    }
                };
            for desc in lib.plugins() {
                all.push(Builtin {
                    name: desc.name.clone().leak(),
                    uid: desc.uid().leak(),
                    vst3_class: (VST3_CLASSES.iter())
                        .find(|(id, _)| *id == desc.id)
                        .map(|(_, class)| *class),
                    size: OnceLock::new(),
                    keyed: OnceLock::new(),
                    probe: u32::MAX - all.len() as u32,
                    desc: desc.clone(),
                    lib: lib.clone(),
                });
            }
        }
        all
    })
}

/// Plugin `id`'s editor size at scale 1 (CLAP's `gui.get_size`): an instance opens its editor once, unseen, as editor
/// `probe` ([`os_plugkit::embed::Parent`]).
fn editor_size(lib: &Arc<dyn Library>, id: &str, probe: u32) -> Option<(u32, u32)> {
    let plugin = lib.clone().create(id).ok()?;
    let mut plugin = plugin.lock().ok()?;
    let size = plugin.open_editor(probe as usize as *mut c_void, 1.0).ok();
    plugin.close_editor();
    size.filter(|&(w, h)| w > 0 && h > 0)
}

/// The built-in plugin a plugin of `format` (lower case) with `id` is, if any.
pub fn find(format: &str, id: &str) -> Option<&'static Builtin> {
    all().iter().find(|b| b.is(format, id))
}

/// The built-in plugin a module's `plugin` is, if any.
pub fn of(plugin: &PluginRef) -> Option<&'static Builtin> {
    find(&plugin.format, &plugin.id)
}

/// The controllers (0.4.9), by CLAP id: they put out a value (MIDI, `os_clap`'s `control_fx`) that their
/// link sets a param of the song to (`crate::audio_engine`'s modulation), as a Modulator's target is.
const CONTROLLERS: [&str; 2] = [
    "org.openstudio.lemon-formula-controller",
    "org.openstudio.lemon-peak-controller",
];

/// Whether a module's `plugin` is a built-in controller ([`CONTROLLERS`]).
pub fn controller(plugin: &PluginRef) -> bool {
    of(plugin).is_some_and(|b| CONTROLLERS.contains(&b.clap_id()))
}

/// The built-in plugin named `name`.
#[cfg(test)]
pub(crate) fn named(name: &str) -> &'static Builtin {
    all().iter().find(|b| b.name == name).unwrap()
}

#[cfg(test)]
mod tests {
    /// Songs know the built-in plugins by their uids (an installed copy by its VST3 class id) and their cards by their
    /// editors' aspect: what the plugins report is what was written out by hand before.
    #[test]
    fn the_plugins_report_what_songs_know_them_by() {
        let got: Vec<_> = (super::all().iter())
            .map(|b| (b.name, b.uid, b.vst3_class.copied(), b.size()))
            .collect();
        assert_eq!(
            got[..9],
            [
                (
                    "Lemon Trio",
                    "clap:org.openstudio.open-3x-osc",
                    Some(*b"OpenStudio3xOsc!"),
                    (lemon_3x_osc::view::EDITOR_W, lemon_3x_osc::view::EDITOR_H)
                ),
                (
                    "Lemon Sampler",
                    "clap:org.openstudio.open-sampler",
                    Some(*b"OpenStudioSamplr"),
                    (lemon_sampler::view::EDITOR_W, lemon_sampler::view::EDITOR_H)
                ),
                (
                    "Lemon Parametric EQ",
                    "clap:org.openstudio.open-parametric-eq-2",
                    Some(*b"OpenStudioParEQ2"),
                    (
                        lemon_parametric_eq_2::view::W,
                        lemon_parametric_eq_2::view::H
                    )
                ),
                (
                    "Lemon Formula Controller",
                    "clap:org.openstudio.lemon-formula-controller",
                    None,
                    (486, 159)
                ),
                (
                    "Lemon Peak Controller",
                    "clap:org.openstudio.lemon-peak-controller",
                    None,
                    (731, 159)
                ),
                (
                    "Tesla",
                    "clap:org.openstudio.tesla",
                    None,
                    (tesla::view::W, tesla::view::H)
                ),
                (
                    "Citrus",
                    "clap:org.openstudio.citrus",
                    Some(*b"LemonStudioCitrs"),
                    (citrus::view::W, citrus::view::H)
                ),
                (
                    "Lemon Drum Sampler",
                    "clap:org.openstudio.lemon-drum-sampler",
                    Some(*b"LemonStudioDrums"),
                    (lemon_drum_sampler::view::W, lemon_drum_sampler::view::H)
                ),
                (
                    "Lemon Slicer",
                    "clap:org.openstudio.lemon-slicer",
                    Some(*b"LemonStudioSlicr"),
                    (lemon_slicer::view::W, lemon_slicer::view::H)
                ),
            ]
        );
        // 0.4.9's effects: Lemon Small FX's ten, one plugin each, Lemon Delay, Lemon Vocoder, Press and Pulp Filter, each
        // with its VST3 copy's class id and its editor's size; only Lemon Vocoder has a sidechain input.
        let added: Vec<_> = (got[9..].iter())
            .map(|&(name, uid, class, _)| {
                (
                    name,
                    uid,
                    class.map(|c| String::from_utf8_lossy(&c).into_owned()),
                )
            })
            .collect();
        let row = |name, id: &str, class: &str| {
            (
                name,
                format!("clap:org.openstudio.{id}"),
                Some(class.to_owned()),
            )
        };
        let want = [
            row(
                "Lemon Soft Clipper",
                "lemon-soft-clipper",
                "LemonStudioClipr",
            ),
            row(
                "Lemon Fast Distortion",
                "lemon-fast-distortion",
                "LemonStudioFDist",
            ),
            row("Lemon Balance", "lemon-balance", "LemonStudioBalnc"),
            row(
                "Lemon Stereo Enhancer",
                "lemon-stereo-enhancer",
                "LemonStudioStEnh",
            ),
            row(
                "Lemon Transient Processor",
                "lemon-transient-processor",
                "LemonStudioTrPro",
            ),
            row("Lemon Chorus", "lemon-chorus", "LemonStudioChrus"),
            row(
                "Lemon Stereo Shaper",
                "lemon-stereo-shaper",
                "LemonStudioStShp",
            ),
            row("Lemon Filter", "lemon-filter", "LemonStudioFiltr"),
            row("Lemon Flanger", "lemon-flanger", "LemonStudioFlngr"),
            row("Lemon Phaser", "lemon-phaser", "LemonStudioPhasr"),
            row("Lemon Delay", "lemon-delay", "LemonStudioDelay"),
            row("Lemon Vocoder", "lemon-vocoder", "LemonStudioVocdr"),
            row("Press", "press", "LemonStudioPress"),
            row("Pulp Filter", "pulp-filter", "LemonStudioPulpF"),
        ];
        let want: Vec<_> = (want.iter())
            .map(|(n, u, c)| (*n, u.as_str(), c.clone()))
            .collect();
        assert_eq!(added, want);
        for &(name, _, _, size) in &got[9..] {
            assert!(
                size != super::FALLBACK_SIZE && size.0 > 0 && size.1 > 0,
                "{name}: {size:?}"
            );
        }
        let keyed: Vec<_> = (super::all().iter())
            .filter(|b| b.sidechain())
            .map(|b| b.name)
            .collect();
        assert_eq!(keyed, ["Lemon Vocoder"]);
        // Each one starts and plays a block (an instrument a note, an effect a sine), every sample finite.
        for b in super::all() {
            let plugin = b.lib.clone().create(&b.desc.id).unwrap();
            let mut p = plugin.lock().unwrap();
            assert!(p.activate(48_000), "{}", b.name);
            let mut l: Vec<f32> = (0..512).map(|i| (i as f32 * 0.05).sin()).collect();
            let mut r = l.clone();
            if b.instrument() {
                p.note_on(0, 60, 1.0);
                p.process(48_000, &mut l, &mut r);
            } else {
                p.process_fx(48_000, &mut l, &mut r);
            }
            assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{}", b.name);
        }
    }
}

//! Lemon Studio's own plugins built into the app, on every platform: Lemon Trio and Lemon Sampler (instruments) and
//! Lemon Parametric EQ (an effect), from `vendor/lemon-studio`. As in Lemon Studio, each is compiled in (`linked`: no
//! exported entry points) and hosted through Lemon Studio's CLAP host from its CLAP entry table, so a module of one is
//! a plugin module like any other (types 1000 and 1001, saved in `HDPm`), only always installed. Their editors draw
//! into the rack cards (`embed`, `crate::ui::rack::lemon_card`).
//!
//! A plugin installed on its own as well (the same CLAP id, or its VST3 copy's class id) is not listed, and a module
//! naming that copy plays the built-in one.

use std::ffi::c_void;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use os_host::{Descriptor, Library};

use crate::song::plugin::PluginRef;

/// One built-in plugin.
pub struct Builtin {
    pub name: &'static str,
    /// `clap:` and its CLAP plugin id, which its modules save: its identity in plugin lists.
    pub uid: &'static str,
    /// Its VST3 copy's class id, by which an installed copy is known.
    vst3_class: &'static [u8; 16],
    /// Its editor's size in its own pixels: a card shows it at the card's width, keeping this aspect.
    pub size: (u32, u32),
    entry: fn() -> *const c_void,
}

impl Builtin {
    /// Its CLAP plugin id.
    pub fn clap_id(&self) -> &'static str {
        &self.uid[CLAP.len()..]
    }

    /// Whether a plugin of `format` with `id` is this one (its CLAP id, or its VST3 copy's class id).
    fn is(&self, format: &str, id: &str) -> bool {
        match format {
            "clap" => id.eq_ignore_ascii_case(self.clap_id()),
            "vst3" => id.eq_ignore_ascii_case(
                &self
                    .vst3_class
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<String>(),
            ),
            _ => false,
        }
    }
}

/// A uid's format part.
const CLAP: &str = "clap:";

/// The built-in plugins. Their ids and class ids are upstream's (`plugins/*/src/lib.rs`).
pub const ALL: [Builtin; 3] = [LEMON_TRIO, LEMON_SAMPLER, LEMON_EQ];

pub const LEMON_TRIO: Builtin = Builtin {
    name: "Lemon Trio",
    uid: "clap:org.openstudio.open-3x-osc",
    vst3_class: b"OpenStudio3xOsc!",
    size: (lemon_3x_osc::view::EDITOR_W, lemon_3x_osc::view::EDITOR_H),
    entry: || &lemon_3x_osc::LINKED_CLAP_ENTRY as *const _ as *const c_void,
};

pub const LEMON_SAMPLER: Builtin = Builtin {
    name: "Lemon Sampler",
    uid: "clap:org.openstudio.open-sampler",
    vst3_class: b"OpenStudioSamplr",
    size: (lemon_sampler::view::EDITOR_W, lemon_sampler::view::EDITOR_H),
    entry: || &lemon_sampler::LINKED_CLAP_ENTRY as *const _ as *const c_void,
};

pub const LEMON_EQ: Builtin = Builtin {
    name: "Lemon Parametric EQ",
    uid: "clap:org.openstudio.open-parametric-eq-2",
    vst3_class: b"OpenStudioParEQ2",
    size: (
        lemon_parametric_eq_2::view::W,
        lemon_parametric_eq_2::view::H,
    ),
    entry: || &lemon_parametric_eq_2::LINKED_CLAP_ENTRY as *const _ as *const c_void,
};

/// The built-in plugin a plugin of `format` (lower case) with `id` is, if any.
pub fn find(format: &str, id: &str) -> Option<&'static Builtin> {
    static LIST: [Builtin; 3] = ALL;
    LIST.iter().find(|b| b.is(format, id))
}

/// The built-in plugin a module's `plugin` is, if any.
pub fn of(plugin: &PluginRef) -> Option<&'static Builtin> {
    find(&plugin.format, &plugin.id)
}

/// The built-in plugins' libraries, opened once: each one's descriptor and library.
pub(super) fn libraries() -> &'static [(Descriptor, Arc<dyn Library>)] {
    static LIBS: OnceLock<Vec<(Descriptor, Arc<dyn Library>)>> = OnceLock::new();
    LIBS.get_or_init(|| {
        let mut libs = Vec::new();
        for b in ALL {
            // SAFETY: the entry is a linked plugin's static CLAP entry table.
            match unsafe { os_clap::Library::from_entry((b.entry)(), Path::new("")) } {
                Ok(lib) => {
                    let lib: Arc<dyn Library> = lib;
                    libs.extend(lib.plugins().iter().map(|d| (d.clone(), lib.clone())));
                }
                Err(e) => eprintln!("plugins: built-in {}: {e}", b.name),
            }
        }
        libs
    })
}

//! A channel's rack (`RACK`/`RMOd`): its modules in signal-chain order.

use std::path::Path;

use super::file::ModuleFileState;
use crate::params;

/// A module's common params, the `SMPR` array `[Post Gain, Mix, Wet Pan]`. Automation targets them with the
/// negative param indices of [`super::AutomationTarget`]: `-1 - param` indexes this array.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommonParams(pub [f32; 3]);

impl Default for CommonParams {
    /// The `params::COMMON_PARAMS` defaults (whose names automation labels use).
    fn default() -> Self {
        Self(params::COMMON_PARAMS.map(|p| p.default))
    }
}

impl CommonParams {
    pub fn post_gain(&self) -> f32 {
        self.0[0]
    }

    pub fn mix(&self) -> f32 {
        self.0[1]
    }

    pub fn wet_pan(&self) -> f32 {
        self.0[2]
    }
}

/// The array index of a negative (common) param index, `None` for `param >= 0`.
pub(super) fn common_index(param: i32) -> Option<usize> {
    usize::try_from(-1 - param).ok()
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModuleInstance {
    /// Module type id ([`crate::params::module_spec`]).
    pub type_id: u16,
    /// Unique within the rack, never reused; what automation lanes target.
    pub instance_id: i32,
    /// `PRMS`, normalised, indexed like the registry's params (`params[0]` is `Enable`).
    pub params: Vec<f32>,
    pub common: CommonParams,
    /// The module card is collapsed in the rack.
    pub minimized: bool,
    pub file: ModuleFileState,
    /// Honeydew's own: the channel id (`CHHD`@1080) whose output keys this module's detector, a sidechain; `None`: its
    /// own input. Only a module that [takes a key](honeydew_audio::takes_key) uses it ([`Self::key_source`]). The `.flm`
    /// keeps it in an `HDSc` chunk FL Studio Mobile skips (`flm::write`).
    pub sidechain: Option<i32>,
}

impl ModuleInstance {
    /// The name its card and its automation labels show: a plugin module's plugin name, else the registry's display
    /// name ("" for an unknown type).
    pub fn title(&self) -> &str {
        match &self.file.plugin {
            Some(plugin) if super::plugin::is_plugin(self.type_id) => &plugin.name,
            _ => params::module_spec(self.type_id).map_or("", |spec| spec.display_name),
        }
    }

    /// Param `index`'s spec: the registry's, or on a plugin module Enable and then the plugin's params, by their names
    /// (a plain continuous law: the plugin itself has the value texts).
    pub fn param_spec(&self, index: usize) -> Option<params::ParamSpec> {
        let spec = params::module_spec(self.type_id)?;
        if let Some(param) = spec.params.get(index) {
            return Some(*param);
        }
        let plugin =
            (self.file.plugin.as_ref()).filter(|_| super::plugin::is_plugin(self.type_id))?;
        let param = plugin.params.get(index.checked_sub(1)?)?;
        Some(params::ParamSpec::plain(
            super::plugin::intern(&param.name),
            param.default,
        ))
    }

    /// A module with every param at its registry default.
    pub fn new(type_id: u16, instance_id: i32) -> Self {
        let params = params::module_spec(type_id)
            .map(|spec| spec.params.iter().map(|p| p.default).collect())
            .unwrap_or_default();
        Self {
            type_id,
            instance_id,
            params,
            common: CommonParams::default(),
            minimized: false,
            file: ModuleFileState::default(),
            sidechain: None,
        }
    }

    /// Loads `preset`, a module of this type read from a preset file ([`crate::flm::read_preset`]): its params, common
    /// params, `CSTM` and the rest of its file chunks. The module keeps its instance id and its place in the rack
    /// (`ADD1`: minimized, the key range), and its sidechain. [`Self::name_preset`] then names the file.
    pub fn load_preset(&mut self, preset: ModuleInstance) {
        let file = ModuleFileState {
            key_low: self.file.key_low,
            key_high: self.file.key_high,
            add1_extra: std::mem::take(&mut self.file.add1_extra),
            ..preset.file
        };
        *self = Self {
            instance_id: self.instance_id,
            minimized: self.minimized,
            file,
            sidechain: self.sidechain,
            ..preset
        };
    }

    /// Loads `Default`, the module's init preset: its registry defaults, no `CSTM`, named Default.
    pub fn load_default(&mut self) {
        self.load_preset(Self::new(self.type_id, 0));
    }

    /// Names the preset file at `path` in `DESC` and `PRST`: the category is its folder and the preset path the file,
    /// both from the user folder `user` (`My Presets/<module folder>`); the name is its stem; `PRST` holds the path and
    /// `Category/Name`.
    pub fn name_preset(&mut self, path: &Path, user: &Path) {
        let text = |path: &Path| path.to_string_lossy().replace('\\', "/");
        let relative = path.strip_prefix(user).unwrap_or(path);
        let file = &mut self.file;
        file.category = text(relative.parent().unwrap_or(Path::new("")));
        file.preset_name = text(Path::new(path.file_stem().unwrap_or_default()));
        file.preset_path = text(relative);
        file.source_path = text(path);
        file.provenance = format!("{}/{}", file.category, file.preset_name);
    }

    /// The DW Sampler's Attack (p1) and Release (p2) init values, which a save writes as its `PATh` floats:
    /// [`ModuleFileState::program_init`], else the params.
    pub fn program_init(&self) -> [f32; 2] {
        (self.file.program_init).unwrap_or_else(|| self.attack_release())
    }

    /// Keeps `init`, the `PATh` floats a program did not apply, as the Attack and Release init values; none of their
    /// own when they are the params (or there are none).
    pub fn keep_program_init(&mut self, init: Option<(f32, f32)>) {
        let params = self.attack_release().map(f32::to_bits);
        self.file.program_init = init
            .map(|(a, r)| [a, r])
            .filter(|init| init.map(f32::to_bits) != params);
    }

    /// A control moved param `index` (when not recording, that sets its init value too). A DW
    /// Sampler's Attack (1) or Release (2) init value kept in `program_init` becomes the param's; the other stays.
    pub fn init_follows(&mut self, index: i32) {
        let Some(mut init) = self.file.program_init.filter(|_| self.type_id == 1) else {
            return;
        };
        let (Ok(i @ 1..=2), Some(value)) = (usize::try_from(index), self.param(index)) else {
            return;
        };
        init[i - 1] = value;
        self.keep_program_init(Some((init[0], init[1])));
    }

    /// Params 1 and 2 (a DW Sampler's Attack and Release), 0 where there are none.
    fn attack_release(&self) -> [f32; 2] {
        [1, 2].map(|i| self.params.get(i).copied().unwrap_or_default())
    }

    /// A program load or pick: Attack (p1) and Release (p2) become `attack` and `release`, held to 0…1, as
    /// both their init and current values, and as the default they reset to.
    pub fn set_attack_release(&mut self, (attack, release): (f32, f32)) {
        for (i, v) in [(1, attack), (2, release)] {
            if let Some(param) = self.params.get_mut(i) {
                *param = v.clamp(0.0, 1.0);
            }
        }
        self.file.program_init = None;
        self.file.program_reset = Some(self.attack_release());
    }

    /// What a double tap and `Reset` set param `index` to where a program set it ([`ModuleFileState::program_reset`]):
    /// a DW Sampler's Attack (1) and Release (2). Not [`Self::program_init`], which follows every move (`init_follows`),
    /// so a reset there would set nothing.
    pub fn program_reset(&self, index: usize) -> Option<f32> {
        let reset = self.file.program_reset.filter(|_| self.type_id == 1)?;
        reset.get(index.checked_sub(1)?).copied()
    }

    /// Param `param` (≥ 0: `params` index; −1, −2, −3: common param), `None` when out of range.
    pub fn param(&self, param: i32) -> Option<f32> {
        match usize::try_from(param) {
            Ok(i) => self.params.get(i).copied(),
            Err(_) => self.common.0.get(common_index(param)?).copied(),
        }
    }

    /// The channel id of the sidechain source this module's detector reads: [`Self::sidechain`] on a module that takes a
    /// key.
    pub fn key_source(&self) -> Option<i32> {
        self.sidechain
            .filter(|_| honeydew_audio::takes_key(self.type_id))
    }

    pub fn param_mut(&mut self, param: i32) -> Option<&mut f32> {
        match usize::try_from(param) {
            Ok(i) => self.params.get_mut(i),
            Err(_) => self.common.0.get_mut(common_index(param)?),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rack {
    /// Signal-chain order: generator first, then effects.
    pub modules: Vec<ModuleInstance>,
    /// `RHED[4]`: the last allocated instance id; a new module gets this + 1. Never decreases.
    pub last_instance_id: i32,
}

impl Default for Rack {
    /// A fresh rack starts at 1, so the first module is id 2 (ids are ≥ 2, 0 is the channel).
    fn default() -> Self {
        Self {
            modules: Vec::new(),
            last_instance_id: 1,
        }
    }
}

impl Rack {
    /// Appends a new module at its defaults; returns its instance id.
    pub fn add(&mut self, type_id: u16) -> i32 {
        self.insert(self.modules.len(), type_id)
    }

    /// Inserts a new module at chain position `index` (clamped to the end); returns its instance id.
    pub fn insert(&mut self, index: usize, type_id: u16) -> i32 {
        self.last_instance_id += 1;
        let id = self.last_instance_id;
        let index = index.min(self.modules.len());
        self.modules.insert(index, ModuleInstance::new(type_id, id));
        id
    }

    pub fn remove(&mut self, instance_id: i32) -> Option<ModuleInstance> {
        let index = self.index_of(instance_id)?;
        Some(self.modules.remove(index))
    }

    /// Swaps the module with its predecessor; false when it is first or absent.
    pub fn move_up(&mut self, instance_id: i32) -> bool {
        match self.index_of(instance_id) {
            Some(i) if i > 0 => {
                self.modules.swap(i - 1, i);
                true
            }
            _ => false,
        }
    }

    /// Swaps the module with its successor; false when it is last or absent.
    pub fn move_down(&mut self, instance_id: i32) -> bool {
        match self.index_of(instance_id) {
            Some(i) if i + 1 < self.modules.len() => {
                self.modules.swap(i, i + 1);
                true
            }
            _ => false,
        }
    }

    /// The chain position of module `instance_id`.
    pub fn index_of(&self, instance_id: i32) -> Option<usize> {
        self.modules
            .iter()
            .position(|m| m.instance_id == instance_id)
    }

    pub fn get(&self, instance_id: i32) -> Option<&ModuleInstance> {
        self.modules.iter().find(|m| m.instance_id == instance_id)
    }

    pub fn get_mut(&mut self, instance_id: i32) -> Option<&mut ModuleInstance> {
        self.modules
            .iter_mut()
            .find(|m| m.instance_id == instance_id)
    }
}

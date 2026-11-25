//! The SoundFont Player panel (type 52), card 8.5 U. No tab strip: a column of two rows
//! (1 : 1.5), the patch picker and the envelope override. While the SoundFont loads (flag `loading` = 1) both rows
//! hide and `...` fills the area, centred in 18 pt `text`; when it failed to load (2), the error message instead.
//!
//! The card wiring (`ui::rack`) drives the patch picker from the module's `SF2D` ([`crate::flm::sf2d`]) and the
//! SoundFont's patches ([`wire`], [`press`], [`error_tap`]): the spinner shows `Load Patch` until a patch is chosen,
//! then the patch's name; `Prev` and `Next` step through the patches. In the error state, where the spinner is hidden,
//! a tap on the content opens the SoundFont picker.
//!
//! A tap on the spinner opens the SoundFont picker (`ui::soundfont_picker`): the side panel of the SoundFont's
//! patches, or of the SoundFont files while there are none to list (no SoundFont chosen yet, one still loading, or
//! one that failed). Another SoundFont is also the card's preset button's: the module has one preset picker, whose
//! `.sf2` files its own loader takes (`ui::views::preset_picker`, `My SoundFonts` beside `My Files`).
//!
//! Issue #51: an SFZ that names controls (`label_ccN`) gets a `PATCH · CONTROLS` tab strip; `CONTROLS` shows the first
//! [`SLOTS`] as knobs under their names, each at its `set_ccN` value when the SFZ is first shown, its moves sent to the
//! instrument as MIDI CC N ([`wire`], [`cc_moves`]). The song keeps none of them, as it keeps no CC.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use eframe::egui::{
    Color32, Id, Rect, ScrollArea, Sense, Ui, UiBuilder, pos2,
    scroll_area::ScrollBarVisibility, vec2,
};
use crate::ui::UiCtx;

use crate::flm::chunk::ContentPath;
use crate::flm::sf2d::{Sf2d, Sf2m, Sf2z};
use crate::panels::build::*;
use crate::sf2::{Control, Preset, Sf2Error};
use honeydew_audio::soundfont::{
    MOD_CLEAR_KEY, MOD_CTL_BASE, MOD_SELECT_KEY, PATCH_MODS, PatchMod, ZONE_CTL_BASE, ZONE_FIELDS,
    ZONE_RESET_KEY, ZONE_SELECT_KEY, ZoneInfo, ZoneOverride,
};

/// The SoundFont Player's module type.
pub const TYPE_ID: u16 = 52;

/// The `#n` the wiring drives: `Prev`, the `Patch` spinner, `Next` and the `...` / error text.
pub const PREV: u32 = 1100;
pub const PATCH: u32 = 1200;
pub const NEXT: u32 = 1300;
pub const MESSAGE: u32 = 3000;
/// The `PATCH · CONTROLS` tab strip, and the first named control's knob (the next ones follow by 100).
pub const TABS: u32 = 4000;
pub const KNOBS: u32 = 4100;
/// The named controls the card shows.
pub const SLOTS: usize = 8;

pub const ZONE_SELECT: u32 = ZONE_SELECT_KEY; // 7000
const ZONE_PREV: u32 = 7001;
const ZONE_LABEL: u32 = 7002;
const ZONE_NEXT: u32 = 7003;
const ZONE_PAGE: u32 = 7004;
const ZONE_RESET: u32 = ZONE_RESET_KEY; // 7005
const HAS_ZONES: &str = "sf2_zones";
const ZONE_SEEDED: &str = "sf2_zone_seeded";
const ZONE_PAGE_FLAG: &str = "sf2_zone_page";

const ZONE_PIANO: u32 = 7007;
const ZONE_KEY_LO: &str = "sf2_key_lo";
const ZONE_KEY_HI: &str = "sf2_key_hi";
const ZONE_VEL_LO: &str = "sf2_vel_lo";
const ZONE_VEL_HI: &str = "sf2_vel_hi";

const MOD_SELECT: u32 = MOD_SELECT_KEY; // 7200
const MOD_PREV: u32 = 7201;
const MOD_LABEL: u32 = 7202;
const MOD_NEXT: u32 = 7203;
const MOD_CLEAR: u32 = MOD_CLEAR_KEY; // 7205
const MOD_GRID: u32 = 7207;
const MOD_SEEDED: &str = "sf2_mod_seeded";
const MOD_SYNC: &str = "sf2_mod_sync";

/// Honeydew change (GitHub #135): the card wiring's flag for a portrait phone-size window (`ui::rack`, whose
/// card is taller there): the ten controls under the patch row take two rows of five, so their labels and values fit.
pub const TWO_ROWS: &str = "two_rows";
/// The SFZ names controls; and the knobs that show one, each 1.
const HAS_CONTROLS: &str = "sfz_controls";
const SHOWN: [&str; SLOTS] = [
    "sfz_cc_0", "sfz_cc_1", "sfz_cc_2", "sfz_cc_3", "sfz_cc_4", "sfz_cc_5", "sfz_cc_6", "sfz_cc_7",
];
/// What the knobs were last set to their `set_ccN` values for: the controls' hash.
const SEEDED: &str = "sfz_controls_seeded";

/// The patches of each SoundFont the rack's SoundFont Players name, by their `SF2D` path, as read
/// ([`crate::project_files::ProjectFiles::soundfont`]): the host reads each one once.
pub type SoundFonts = HashMap<ContentPath, Result<Vec<Preset>, Sf2Error>>;

#[derive(Clone, Debug, Default)]
pub struct ZoneOutcome {
    pub sent: Vec<(u32, f32)>,
    pub stored: bool,
    pub zone_edit: bool,
    pub commit_now: bool,
}

pub fn edit_zones(
    module: &mut ModuleInstance,
    state: &mut PanelState,
    edited: &[u32],
    fonts: &SoundFonts,
) -> ZoneOutcome {
    let mut out = ZoneOutcome::default();
    let pressed = |id| edited.contains(&id) && state.settings.get(&id).is_some_and(|&v| v >= 0.5);
    let mut msel = state
        .settings
        .get(&MOD_SELECT)
        .copied()
        .unwrap_or(0.0)
        .round()
        .clamp(0.0, (PATCH_MODS - 1) as f32) as usize;
    if let Some(by) = match (pressed(MOD_PREV), pressed(MOD_NEXT)) {
        (true, false) => Some(-1_i64),
        (false, true) => Some(1_i64),
        _ => None,
    } {
        msel = (msel as i64 + by).rem_euclid(PATCH_MODS as i64) as usize;
        state.settings.insert(MOD_SELECT, msel as f32);
        state.flags.remove(MOD_SEEDED);
        state.flags.insert(MOD_SYNC, msel);
        out.sent.push((MOD_SELECT, msel as f32));
        return out;
    }
    if state.flags.get(MOD_SYNC).copied() != Some(msel) {
        state.flags.insert(MOD_SYNC, msel);
        state.flags.remove(MOD_SEEDED);
        out.sent.push((MOD_SELECT, msel as f32));
    }
    let cstm = module.file.custom_data.as_deref();
    if pressed(MOD_CLEAR) {
        let mut sf2m = cstm.and_then(Sf2m::read).unwrap_or_default();
        sf2m.mods[msel] = PatchMod::default();
        if let Ok(blob) = sf2m.write(cstm) {
            module.file.custom_data = Some(blob);
            state.flags.remove(MOD_SEEDED);
            state.settings.insert(MOD_CTL_BASE, 0.0);
            state.settings.insert(MOD_CTL_BASE + 1, 0.0);
            state.settings.insert(MOD_CTL_BASE + 2, 0.0);
            state.settings.insert(MOD_CTL_BASE + 3, 0.5);
            out.sent.push((MOD_SELECT, msel as f32));
            out.sent.push((MOD_CLEAR, 1.0));
            out.stored = true;
            out.zone_edit = true;
            out.commit_now = true;
        }
        return out;
    }
    let mut mod_touched = false;
    let mut sf2m = cstm.and_then(Sf2m::read).unwrap_or_default();
    for &id in edited {
        if let Some(f) = id.checked_sub(MOD_CTL_BASE)
            && f < 4
            && let Some(&v) = state.settings.get(&id)
        {
            if !mod_touched {
                out.sent.push((MOD_SELECT, msel as f32));
                mod_touched = true;
            }
            out.sent.push((id, v));
            let pm = &mut sf2m.mods[msel];
            match f {
                0 => pm.src = v.round().clamp(0.0, 14.0) as u8,
                1 => pm.curve = v.round().clamp(0.0, 6.0) as u8,
                2 => pm.dest = v.round().clamp(0.0, 16.0) as u8,
                3 => pm.amount = v.clamp(0.0, 1.0),
                _ => {}
            }
        }
    }
    if mod_touched && let Ok(blob) = sf2m.write(cstm) {
        module.file.custom_data = Some(blob);
        out.stored = true;
        out.zone_edit = true;
        return out;
    }

    let Some((patch, zones)) = current_zones(module, fonts) else {
        return out;
    };
    if zones.is_empty() {
        return out;
    }
    let sel = state.settings.get(&ZONE_SELECT).copied().unwrap_or(0.0).round().clamp(0.0, (zones.len() - 1) as f32) as usize;
    if let Some(by) = match (pressed(ZONE_PREV), pressed(ZONE_NEXT)) {
        (true, false) => Some(-1_i64),
        (false, true) => Some(1_i64),
        _ => None,
    } {
        let next = (sel as i64 + by).rem_euclid(zones.len() as i64) as usize;
        state.settings.insert(ZONE_SELECT, next as f32);
        state.flags.remove(ZONE_SEEDED);
        out.sent.push((ZONE_SELECT, next as f32));
        return out;
    }
    let cstm = module.file.custom_data.as_deref();
    let mut sf2z = cstm.and_then(Sf2z::read).filter(|z| z.patch == patch).unwrap_or(Sf2z {
        patch,
        zones: Vec::new(),
    });
    if pressed(ZONE_RESET) {
        sf2z.zones.retain(|o| usize::from(o.zone) != sel);
        if let Ok(blob) = sf2z.write(cstm) {
            module.file.custom_data = Some(blob);
            state.flags.remove(ZONE_SEEDED);
            for (f, v) in zones[sel].defaults.into_iter().enumerate() {
                state.settings.insert(ZONE_CTL_BASE + f as u32, v);
            }
            out.sent.push((ZONE_SELECT, sel as f32));
            out.sent.push((ZONE_RESET, 1.0));
            out.stored = true;
            out.commit_now = true;
        }
        return out;
    }
    let mut touched = false;
    for &id in edited {
        if let Some(f) = id.checked_sub(ZONE_CTL_BASE)
            && (f as usize) < ZONE_FIELDS
            && let Some(&v) = state.settings.get(&id)
        {
            if !touched {
                out.sent.push((ZONE_SELECT, sel as f32));
                touched = true;
            }
            out.sent.push((id, v));
            if let Some(ov) = sf2z.zones.iter_mut().find(|o| usize::from(o.zone) == sel) {
                ov.mask |= 1u64 << f;
                ov.values[f as usize] = v;
            } else {
                let mut ov = ZoneOverride {
                    zone: sel as u16,
                    mask: 1u64 << f,
                    values: zones[sel].defaults,
                };
                ov.values[f as usize] = v;
                sf2z.zones.push(ov);
            }
        }
    }
    if touched && let Ok(blob) = sf2z.write(cstm) {
        module.file.custom_data = Some(blob);
        out.stored = true;
        out.zone_edit = true;
    }
    out
}

/// Sets a card's `state` for `module`'s `SF2D` before the panel draws:
/// - no SoundFont: the spinner's `Load Patch`;
/// - a SoundFont `fonts` holds the patches of: the patch's name on the spinner (an index past either end reads as
///   the patch at that end), and the patch's named controls on the knobs;
/// - one it could not read: the error state (flag `loading` 2), its message in place of `...`;
/// - one `fonts` lacks: loading (flag `loading` 1).
///
/// Controls other than the last ones shown set their knobs to their `set_ccN` values: the CC moves (channel param,
/// value) that sends to the instrument too, so it plays as they read.
pub fn wire(
    module: &ModuleInstance,
    state: &mut PanelState,
    fonts: &SoundFonts,
) -> Vec<(i32, f32)> {
    state.texts.clear();
    let sf2d = module.file.custom_data.as_deref().and_then(Sf2d::read);
    let loading = match sf2d.map(|s| (fonts.get(&s.path), s.patch)) {
        None => 0,
        Some((None, _)) => 1,
        Some((Some(Err(err)), _)) => {
            state.texts.insert(MESSAGE, err.to_string());
            2
        }
        Some((Some(Ok(patches)), patch)) => {
            if let Some(p) = patches.get(clamp(patch, patches.len())) {
                state.texts.insert(PATCH, p.name.clone());
            }
            0
        }
    };
    state.flags.insert(flag::LOADING, loading);
    let controls = controls(module, fonts);
    state
        .flags
        .insert(HAS_CONTROLS, usize::from(!controls.is_empty()));
        
    if controls.is_empty() && state.tab >= 5 {
        state.tab = 0;
    } else if !controls.is_empty() && state.tab >= 4 {
        state.tab = 0;
    }

    let zones = current_zones(module, fonts).map(|(_, z)| z).unwrap_or(&[]);
    state.flags.insert(HAS_ZONES, usize::from(!zones.is_empty()));
    if !zones.is_empty() {
        state.flags.entry(ZONE_PAGE_FLAG).or_insert(0);
        let sel = state.settings.get(&ZONE_SELECT).copied().unwrap_or(0.0).round().clamp(0.0, (zones.len() - 1) as f32) as usize;
        state.settings.insert(ZONE_SELECT, sel as f32);
        let z = &zones[sel];
        let oct_off = state.flags.get(flag::OCTAVE_OFFSET).copied().unwrap_or(0) as i32;
        let range = if z.keys[0] == z.keys[1] {
            note_name(z.keys[0], oct_off)
        } else {
            format!("{}-{}", note_name(z.keys[0], oct_off), note_name(z.keys[1], oct_off))
        };
        let vel = if z.vels == [0, 127] {
            String::new()
        } else {
            format!(" v{}-{}", z.vels[0], z.vels[1])
        };
        let label = if z.name.is_empty() {
            format!("{}/{} ({range}{vel})", sel + 1, zones.len())
        } else {
            format!("{}/{}: {} ({range}{vel})", sel + 1, zones.len(), z.name)
        };
        state.texts.insert(ZONE_LABEL, label);
        state.flags.insert(ZONE_KEY_LO, usize::from(z.keys[0]));
        state.flags.insert(ZONE_KEY_HI, usize::from(z.keys[1]));
        state.flags.insert(ZONE_VEL_LO, usize::from(z.vels[0]));
        state.flags.insert(ZONE_VEL_HI, usize::from(z.vels[1]));
        let mut zh = std::collections::hash_map::DefaultHasher::new();
        (sel, module.file.custom_data.as_deref()).hash(&mut zh);
        let zkey = zh.finish() as usize;
        if state.flags.insert(ZONE_SEEDED, zkey) != Some(zkey) {
            let mut vals = z.defaults;
            if let (Some(sf2d), Some(sf2z)) = (
                module.file.custom_data.as_deref().and_then(Sf2d::read),
                module.file.custom_data.as_deref().and_then(Sf2z::read),
            ) && sf2z.patch == sf2d.patch
                && let Some(ov) = sf2z.zones.iter().find(|o| usize::from(o.zone) == sel)
            {
                for f in 0..ZONE_FIELDS {
                    if ov.mask & (1u64 << f) != 0 {
                        vals[f] = ov.values[f];
                    }
                }
            }
            for (f, v) in vals.into_iter().enumerate() {
                state.settings.insert(ZONE_CTL_BASE + f as u32, v);
            }
        }
        let f_norm = state.settings.get(&(ZONE_CTL_BASE + 13)).copied().unwrap_or(0.5);
        let m_norm = state.settings.get(&(ZONE_CTL_BASE + 38)).copied().unwrap_or(0.0);
        let zmult = (23.0 * m_norm).round() + 1.0;
        if zmult > 1.0 {
            let zcents = ((200.0 * f_norm - 100.0) * zmult).round() as i32;
            let label = if zcents == 0 {
                "Fine 0c".into()
            } else {
                format!("Fine {zcents:+}c")
            };
            state.texts.insert(ZONE_CTL_BASE + 13, label);
        }
    }
    
    let msel = state
        .settings
        .get(&MOD_SELECT)
        .copied()
        .unwrap_or(0.0)
        .round()
        .clamp(0.0, (PATCH_MODS - 1) as f32) as usize;
    state.settings.insert(MOD_SELECT, msel as f32);
    state.texts.insert(MOD_LABEL, format!("Mod {}/{PATCH_MODS}", msel + 1));
    let mut mh = std::collections::hash_map::DefaultHasher::new();
    (msel, module.file.custom_data.as_deref()).hash(&mut mh);
    let mkey = mh.finish() as usize;
    if state.flags.insert(MOD_SEEDED, mkey) != Some(mkey) {
        let pm = module
            .file
            .custom_data
            .as_deref()
            .and_then(Sf2m::read)
            .unwrap_or_default()
            .mods[msel];
        state.settings.insert(MOD_CTL_BASE, f32::from(pm.src));
        state.settings.insert(MOD_CTL_BASE + 1, f32::from(pm.curve));
        state.settings.insert(MOD_CTL_BASE + 2, f32::from(pm.dest));
        state.settings.insert(MOD_CTL_BASE + 3, pm.amount);
    }

    for (slot, key) in SHOWN.iter().enumerate() {
        state.flags.insert(key, usize::from(slot < controls.len()));
    }
    for (slot, control) in controls.iter().enumerate() {
        state.texts.insert(knob_id(slot), control.label.clone());
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for control in controls {
        (control.cc, &control.label, control.value.to_bits()).hash(&mut hasher);
    }
    let seeded = hasher.finish() as usize;
    if state.flags.insert(SEEDED, seeded) == Some(seeded) {
        return Vec::new();
    }
    let seed = |(slot, control): (usize, &Control)| {
        state.settings.insert(knob_id(slot), control.value);
        (cc_param(control.cc), control.value)
    };
    controls.iter().enumerate().map(seed).collect()
}

/// The knob of control slot `slot`.
fn knob_id(slot: usize) -> u32 {
    KNOBS + 100 * slot as u32
}

/// The channel param of MIDI CC `cc`.
fn cc_param(cc: u8) -> i32 {
    7 + i32::from(cc)
}

/// The named controls of the patch `module` plays, as `fonts` read it, the first [`SLOTS`].
fn controls<'a>(module: &ModuleInstance, fonts: &'a SoundFonts) -> &'a [Control] {
    let controls = patches(module, fonts).map_or(&[][..], |(p, i)| &p[i].controls);
    &controls[..controls.len().min(SLOTS)]
}

/// The CC moves (channel param, value) of the control knobs this pass edited (`edited`).
pub fn cc_moves(
    module: &ModuleInstance,
    state: &PanelState,
    edited: &[u32],
    fonts: &SoundFonts,
) -> Vec<(i32, f32)> {
    let controls = controls(module, fonts).iter().enumerate();
    let moved = controls.filter(|(slot, _)| edited.contains(&knob_id(*slot)));
    let value = |slot| state.settings.get(&knob_id(slot)).copied().unwrap_or(0.0);
    moved
        .map(|(slot, c)| (cc_param(c.cc), value(slot)))
        .collect()
}

/// Patch `patch` of `len`, an index past either end read as the patch at that end.
fn clamp(patch: i32, len: usize) -> usize {
    usize::try_from(patch)
        .unwrap_or(0)
        .min(len.saturating_sub(1))
}

/// What a pass's edited controls without a param (`edited`, [`crate::panels::Edits::settings`]) asked of the wiring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// The `Patch` spinner, or a tap on the error: open the SoundFont picker.
    Picker,
    /// `Prev` or `Next` set `module`'s `SF2D` to another patch.
    Stepped,
}

/// The patches of the SoundFont `module`'s `SF2D` names as `fonts` read them, and the current one (an index past
/// either end reads as the patch at that end). `None` without a SoundFont, for one `fonts` lacks or could not read, and
/// for one without patches.
fn patches<'a>(module: &ModuleInstance, fonts: &'a SoundFonts) -> Option<(&'a [Preset], usize)> {
    let sf2d = module.file.custom_data.as_deref().and_then(Sf2d::read)?;
    let patches = fonts.get(&sf2d.path)?.as_deref().ok()?;
    (!patches.is_empty()).then(|| (patches, clamp(sf2d.patch, patches.len())))
}

/// Sets `module`'s `SF2D` patch, keeping the rest of its blob. Whether it had an `SF2D` to set.
fn set_patch(module: &mut ModuleInstance, patch: usize) -> bool {
    let blob = module.file.custom_data.as_deref();
    let Some(mut sf2d) = blob.and_then(Sf2d::read) else {
        return false;
    };
    sf2d.patch = patch as i32;
    let Ok(blob) = sf2d.write(blob).and_then(|b| Sf2z::clear(Some(&b))) else {
        return false;
    };
    module.file.custom_data = Some(blob);
    true
}

/// Acts on `Prev` / `Next` pressed (not released) and the `Patch` spinner tapped this pass. `Prev` and `Next` step the
/// patch through the SoundFont's patches in file order and wrap at either end; without a SoundFont that `fonts` read
/// they do nothing. The spinner opens the picker.
pub fn press(
    module: &mut ModuleInstance,
    state: &PanelState,
    edited: &[u32],
    fonts: &SoundFonts,
) -> Option<Press> {
    if edited.contains(&PATCH) {
        return Some(Press::Picker);
    }
    let pressed = |id| edited.contains(&id) && state.settings.get(&id).is_some_and(|&v| v >= 0.5);
    let by = match (pressed(PREV), pressed(NEXT)) {
        (true, false) => -1,
        (false, true) => 1,
        _ => return None,
    };
    let (patches, current) = patches(module, fonts)?;
    // Both ends wrap.
    let to = (current as i64 + by).rem_euclid(patches.len() as i64);
    set_patch(module, to as usize).then_some(Press::Stepped)
}

/// In the error state (`loading` 2) the controls hide, the spinner with them, so a tap anywhere on the content
/// (`rect`, points) asks for the picker: the way to another SoundFont. `id` is the tap's.
pub fn error_tap(ui: &Ui, id: Id, rect: Rect, state: &PanelState) -> Option<Press> {
    let failed = state.flags.get(flag::LOADING) == Some(&2);
    (failed && ui.interact(rect, id, Sense::click()).clicked()).then_some(Press::Picker)
}

const TABS_SFZ: u32 = 4050;

/// Loaded, or nothing to load: the controls show.
const IDLE: Show = Show::Flag(flag::LOADING, 0);
const NO_CONTROLS: Show = Show::All(&[IDLE, Show::Flag(HAS_CONTROLS, 0)]);
const WITH_CONTROLS: Show = Show::All(&[IDLE, Show::Flag(HAS_CONTROLS, 1)]);
const PATCH_SHOWN: Show = Show::All(&[IDLE, Show::Tab(0)]);
const VIB_SHOWN: Show = Show::All(&[IDLE, Show::Tab(1)]);
const FILTER_SHOWN: Show = Show::All(&[IDLE, Show::Tab(2)]);
const MODS_SHOWN: Show = Show::All(&[IDLE, Show::Flag(HAS_CONTROLS, 0), Show::Tab(3)]);
const CONTROLS_SHOWN: Show = Show::All(&[IDLE, Show::Flag(HAS_CONTROLS, 1), Show::Tab(3)]);
const ZONES_SHOWN: Show = Show::All(&[IDLE, Show::Flag(HAS_CONTROLS, 0), Show::Tab(4)]);
const ZONE_LIST: Show = Show::All(&[ZONES_SHOWN, Show::Flag(HAS_ZONES, 1)]);
const NO_ZONES: Show = Show::All(&[ZONES_SHOWN, Show::Flag(HAS_ZONES, 0)]);

/// `Envelope` is `Custom` (above 0.5): the sliders' envelope, which only then takes their input.
const VIB_CUSTOM: Show = Show::Above(15, 0.5);
const FILTER_CUSTOM: Show = Show::Above(12, 0.5);

/// The `Original` / `Custom` toggle's state text.
fn envelope_text(v: f32) -> String {
    if v > 0.5 { "Global" } else { "Instrument" }.to_owned()
}

/// The `Quality` toggle's state text: `Linear` interpolation, or `High` (cubic) above 0.5.
fn quality_text(v: f32) -> String {
    if v > 0.5 { "High" } else { "Linear" }.to_owned()
}

/// A control knob's value text: the CC value, 0…127.
fn cc_text(v: f32) -> String {
    format!("{}", (127.0 * v).round())
}

fn note_name(key: u8, oct_offset: i32) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let oct = i32::from(key / 12) - oct_offset;
    format!("{}{oct}", NAMES[usize::from(key % 12)])
}

fn zone_time_text(v: f32) -> String {
    let t = (10.0 * 0.95f32.mul_add(v, 0.05).powi(3)).max(0.005);
    if t >= 1.0 { format!("{t:.2} s") } else { format!("{} ms", (1000.0 * t).round()) }
}
fn zone_pct_text(v: f32) -> String { format!("{}%", (100.0 * v).round()) }
fn zone_rate_text(v: f32) -> String { format!("{:.2} Hz", 20.0 * v) }
fn zone_cents_text(v: f32) -> String { format!("{:+} c", (200.0 * v - 100.0).round() as i32) }
fn zone_delay_text(v: f32) -> String { format!("{:.2} s", 5.0 * v) }
fn zone_cutoff_text(v: f32) -> String {
    let hz = 8.176 * 2f32.powf((1500.0 + 12000.0 * v) / 1200.0);
    if hz >= 1000.0 { format!("{:.2} kHz", hz / 1000.0) } else { format!("{} Hz", hz.round()) }
}
fn zone_res_text(v: f32) -> String { format!("{:.1} dB", 40.0 * v) }
fn zone_tune_text(v: f32) -> String { format!("{:+} st", (48.0 * v).round() as i32 - 24) }
fn zone_pan_text(v: f32) -> String {
    let p = ((v - 0.5) * 200.0).round() as i32;
    if p == 0 { "Center".into() } else if p < 0 { format!("{}% L", -p) } else { format!("{p}% R") }
}
fn zone_db_text(v: f32) -> String { format!("{:+.1} dB", 60.0 * v - 48.0) }

fn current_zones<'a>(module: &ModuleInstance, fonts: &'a SoundFonts) -> Option<(i32, &'a [ZoneInfo])> {
    let sf2d = module.file.custom_data.as_deref().and_then(Sf2d::read)?;
    let (presets, idx) = patches(module, fonts)?;
    Some((sf2d.patch, &presets[idx].zones))
}

fn is_black_key(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

fn mod_time_text(v: f32) -> String {
    if v <= 0.001 {
        "0 ms".into()
    } else {
        let t = (20.0 * v.powi(3)).max(0.001);
        if t >= 1.0 { format!("{t:.2} s") } else { format!("{} ms", (1000.0 * t).round()) }
    }
}
fn mod_lfo_pitch_text(v: f32) -> String { format!("{:+} c", (2400.0 * v - 1200.0).round() as i32) }
fn mod_env_pitch_text(v: f32) -> String { format!("{:+} c", (4800.0 * v - 2400.0).round() as i32) }
fn mod_filter_text(v: f32) -> String { format!("{:+} c", (24000.0 * v - 12000.0).round() as i32) }
fn mod_vol_text(v: f32) -> String { format!("{:+.1} dB", 48.0 * v - 24.0) }
fn mod_key_text(v: f32) -> String { format!("{:+} c", (2400.0 * v - 1200.0).round() as i32) }

fn fine_tune_label(params: &[f32]) -> String {
    let mult = (23.0 * params.get(16).copied().unwrap_or(0.0)).round() + 1.0;
    if mult <= 1.0 {
        "Fine Tune".into()
    } else {
        let cents = ((200.0 * params.get(6).copied().unwrap_or(0.5) - 100.0) * mult).round() as i32;
        if cents == 0 {
            "Fine 0c".into()
        } else {
            format!("Fine {cents:+}c")
        }
    }
}

fn zone_mult_text(v: f32) -> String {
    format!("{}x", (23.0 * v.clamp(0.0, 1.0)).round() as i32 + 1)
}

const ZONE_TABS: [&str; 7] = [
    "Vol Env",
    "Mod Env",
    "Env Map",
    "Pitch LFO",
    "Mod LFO",
    "Tone",
    "Sample",
];

const EXCL_ITEMS: [&str; 33] = [
    "Off", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10",
    "11", "12", "13", "14", "15", "16", "17", "18", "19", "20",
    "21", "22", "23", "24", "25", "26", "27", "28", "29", "30",
    "31", "32",
];

const LOOP_ITEMS: [&str; 3] = ["Off", "Loop", "Hold"];

fn zone_pct1_text(v: f32) -> String {
    format!("{:.1}%", 100.0 * v.clamp(0.0, 1.0))
}
fn zone_fixed_key_text(v: f32) -> String {
    let idx = (128.0 * v.clamp(0.0, 1.0)).round() as i32;
    if idx <= 0 {
        "Off".into()
    } else {
        note_name((idx - 1).min(127) as u8, 0)
    }
}
fn zone_fixed_vel_text(v: f32) -> String {
    let idx = (128.0 * v.clamp(0.0, 1.0)).round() as i32;
    if idx <= 0 {
        "Off".into()
    } else {
        format!("{}", (idx - 1).min(127))
    }
}

fn zone_tabs(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    _: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    if !state.flags.contains_key(ZONE_PAGE_FLAG) {
        state.flags.insert(ZONE_PAGE_FLAG, 0);
        ui.ctx().request_repaint();
    }
    let selected = state
        .flags
        .get(ZONE_PAGE_FLAG)
        .copied()
        .unwrap_or(0)
        .min(ZONE_TABS.len() - 1);
    
    if !state.settings.contains_key(&ZONE_PAGE) {
        state.settings.insert(ZONE_PAGE, 0.0);
        ui.ctx().request_repaint();
    }

    let t = cx.m.t();
    let text_col = cx.theme.colors.text.to_color32();
    let menu_fill = cx.theme.colors.menu_fill.to_color32();
    let font = cx.m.font_id(FontSize::Pt10);

    let mut scrolled = ui.new_child(UiBuilder::new().max_rect(rect));
    ScrollArea::horizontal()
        .id_salt(id.with("scroll"))
        .max_width(rect.width())
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        .show(&mut scrolled, |ui| {
            let tab_w = rect.width() / 3.8;
            let row_h = rect.height();
            let total_w = tab_w * ZONE_TABS.len() as f32;
            let (strip, _) = ui.allocate_exact_size(vec2(total_w, row_h), Sense::hover());
            let painter = ui.painter();
            for (i, label) in ZONE_TABS.iter().enumerate() {
                let cell = Rect::from_min_size(
                    pos2(strip.left() + i as f32 * tab_w, strip.top()),
                    vec2(tab_w, row_h),
                );
                let resp = ui.interact(cell, id.with(("tab", i)), Sense::click());
                if resp.clicked() && i != selected {
                    state.flags.insert(ZONE_PAGE_FLAG, i);
                    ui.ctx().request_repaint();
                }
                let active = i == selected;
                let bar = cell.with_min_y(cell.bottom() - 2.0 * t);
                let text_rect = cell.with_max_y(cell.bottom() - 3.0 * t);
                if resp.is_pointer_button_down_on() {
                    painter.rect_filled(cell, 0.0, text_col.gamma_multiply(0.1));
                }
                painter.rect_filled(bar, 0.0, if active { text_col } else { menu_fill });
                painter.text(
                    text_rect.center(),
                    Align2::CENTER_CENTER,
                    label,
                    font.clone(),
                    if active { text_col } else { ink },
                );
            }
        });
    Vec::new()
}

const MOD_SRC_ITEMS: [&str; 15] = [
    "Vel", "Key", "ModWhl", "AfterT", "PitchWhl",
    "CC 2", "CC 11", "CC 71", "CC 72", "CC 73",
    "CC 74", "CC 75", "CC 76", "CC 77", "Const",
];

const MOD_CURVE_ITEMS: [&str; 7] = [
    "Lin", "Inv Lin", "Concave", "Inv Conc", "Convex", "Bipolar", "Switch",
];

const MOD_DEST_ITEMS: [&str; 17] = [
    "Off", "Pitch", "Cutoff", "Resonance", "Atten", "Pan",
    "Vib Depth", "Vib Rate", "Mod Pitch", "Mod Filter", "Mod Vol", "Mod Rate",
    "Vol Atk", "Vol Dec", "Vol Rel", "Mod Atk", "Mod Dec",
];

fn mod_amt_text(v: f32) -> String {
    let pct = (200.0 * v.clamp(0.0, 1.0) - 100.0).round() as i32;
    if pct == 0 { "0%".into() } else { format!("{pct:+}%") }
}

fn mod_grid(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let px = 1.0 / cx.m.ppp();
    let u = rect.height() / 2.0;
    let pad_x = (0.35 * u).min(rect.width() * 0.05);
    let pad_y = (0.14 * u).min(rect.height() * 0.12);
    let area = Rect::from_min_max(
        pos2(rect.left() + pad_x, rect.top() + pad_y),
        pos2(rect.right() - pad_x, rect.bottom() - pad_y),
    );
    if area.width() <= 8.0 * px || area.height() <= 8.0 * px {
        return Vec::new();
    }

    let sf2m = module
        .file
        .custom_data
        .as_deref()
        .and_then(Sf2m::read)
        .unwrap_or_default();
    let sel = state
        .settings
        .get(&MOD_SELECT)
        .copied()
        .unwrap_or(0.0)
        .round()
        .clamp(0.0, (PATCH_MODS - 1) as f32) as usize;

    let painter = ui.painter();
    let bg = cx.theme.colors.menu_fill.to_color32();
    let text_col = cx.theme.colors.text.to_color32();
    let active_bg = Color32::from_rgb(
        ((u16::from(bg.r()) * 3 + u16::from(ink.r())) / 4) as u8,
        ((u16::from(bg.g()) * 3 + u16::from(ink.g())) / 4) as u8,
        ((u16::from(bg.b()) * 3 + u16::from(ink.b())) / 4) as u8,
    );
    let font = cx.m.font_id(FontSize::Pt8);
    let gap = (3.0 * px).max(0.08 * u);
    let cw = (area.width() - 3.0 * gap) / 4.0;
    let ch = (area.height() - gap) / 2.0;
    let rad = (3.5 * px).min(ch * 0.25);

    for (i, pm) in sf2m.mods.iter().enumerate() {
        let col = (i % 4) as f32;
        let row = (i / 4) as f32;
        let cell = Rect::from_min_size(
            pos2(area.left() + col * (cw + gap), area.top() + row * (ch + gap)),
            vec2(cw, ch),
        );
        let resp = ui.interact(cell, id.with(("slot", i)), Sense::click());
        if resp.clicked() && i != sel {
            state.settings.insert(MOD_SELECT, i as f32);
            state.flags.remove(MOD_SEEDED);
            ui.ctx().request_repaint();
        }
        let active = pm.dest != 0;
        let selected = i == sel;
        if selected {
            painter.rect_filled(cell, rad, if active { ink } else { text_col });
            painter.rect_filled(
                cell.shrink(1.5 * px),
                (rad - 1.5 * px).max(0.0),
                if active { active_bg } else { bg },
            );
        } else {
            painter.rect_filled(cell, rad, if active { active_bg } else { bg });
        }
        let src = MOD_SRC_ITEMS.get(usize::from(pm.src)).copied().unwrap_or("?");
        let dest = MOD_DEST_ITEMS.get(usize::from(pm.dest)).copied().unwrap_or("Off");
        let label = if active {
            format!("{}: {src} > {dest}", i + 1)
        } else {
            format!("{}: Off", i + 1)
        };
        painter.text(
            cell.center(),
            Align2::CENTER_CENTER,
            label,
            font.clone(),
            if active || selected { text_col } else { ink },
        );
    }
    Vec::new()
}

fn zone_piano(
    ui: &mut Ui,
    cx: &UiCtx,
    _: Id,
    rect: Rect,
    _: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let k_lo = state.flags.get(ZONE_KEY_LO).copied().unwrap_or(0) as u8;
    let k_hi = state.flags.get(ZONE_KEY_HI).copied().unwrap_or(127) as u8;
    let v_lo = state.flags.get(ZONE_VEL_LO).copied().unwrap_or(0).min(127) as f32 / 127.0;
    let v_hi = state.flags.get(ZONE_VEL_HI).copied().unwrap_or(127).min(127) as f32 / 127.0;

    let px = 1.0 / cx.m.ppp();
    let u = rect.height() / 1.45;
    let pad_x = (0.35 * u).min(rect.width() * 0.05);
    let pad_top = (0.14 * u).min(rect.height() * 0.12);
    let pad_bot = (0.18 * u).min(rect.height() * 0.16);
    let outer = Rect::from_min_max(
        pos2(rect.left() + pad_x, rect.top() + pad_top),
        pos2(rect.right() - pad_x, rect.bottom() - pad_bot),
    );
    if outer.width() <= 4.0 * px || outer.height() <= 4.0 * px {
        return Vec::new();
    }

    let painter = ui.painter();
    let bg = cx.theme.colors.menu_fill.to_color32();
    let white_bg = cx.theme.colors.white_key_disabled.to_color32();
    let black_bg = cx.theme.colors.black_key.to_color32();
    let middle_c = cx.theme.colors.text.to_color32();
    let ghost = Color32::from_rgba_unmultiplied(ink.r(), ink.g(), ink.b(), 65);
    let black_ghost = Color32::from_rgba_unmultiplied(ink.r(), ink.g(), ink.b(), 40);
    let mix = |x: u8, y: u8, t: f32| {
        (f32::from(x) + (f32::from(y) - f32::from(x)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    let black_ink = Color32::from_rgb(
        mix(black_bg.r(), ink.r(), 0.58),
        mix(black_bg.g(), ink.g(), 0.58),
        mix(black_bg.b(), ink.b(), 0.58),
    );

    let tray_r = (0.20 * u).min(outer.height() * 0.3);
    let cheek = (tray_r * 0.55).max(2.0 * px);
    let rail_top = (2.0 * px).max(0.05 * u);
    let rail_bot = (2.0 * px).max(0.05 * u);
    let inner = Rect::from_min_max(
        pos2(outer.left() + cheek, outer.top() + rail_top),
        pos2(outer.right() - cheek, outer.bottom() - rail_bot),
    );
    let kw = inner.width() / 75.0; // 75 white keys across MIDI 0..=127
    let key_r = (kw * 0.24).clamp(px, 2.5 * px);

    let round_bottom = |r: Rect, rad: f32, color: Color32| {
        let rad = rad.min(r.width() * 0.49).min(r.height() * 0.49);
        painter.rect_filled(r, rad, color);
        if rad > 0.0 && r.height() > rad {
            let top = Rect::from_min_max(r.min, pos2(r.right(), r.top() + rad));
            painter.rect_filled(top, 0.0, color);
        }
    };
    let paint_band = |kr: Rect, rad: f32, g: Color32, fill: Color32| {
        round_bottom(kr, rad, g);
        let top = kr.top() + kr.height() * v_lo;
        let bot = (kr.top() + kr.height() * v_hi).max(top + px);
        let band = Rect::from_min_max(pos2(kr.left(), top), pos2(kr.right(), bot));
        if v_hi >= 0.98 {
            round_bottom(band, rad, fill);
        } else {
            painter.rect_filled(band, 0.0, fill);
        }
    };

    painter.rect_filled(outer, tray_r, bg);

    // Pass 1: white keys.
    let mut w = 0usize;
    for note in 0..=127u8 {
        if is_black_key(note) {
            continue;
        }
        let x0 = inner.left() + w as f32 * kw;
        let x1 = (inner.left() + (w + 1) as f32 * kw - px).max(x0 + px);
        let kr = Rect::from_min_max(pos2(x0, inner.top()), pos2(x1, inner.bottom()));
        round_bottom(kr, key_r, white_bg);
        if note >= k_lo && note <= k_hi {
            paint_band(kr, key_r, ghost, ink);
        }
        if note == 60 {
            let dot = if note >= k_lo && note <= k_hi && v_hi > 0.85 { bg } else { middle_c };
            painter.circle_filled(pos2(kr.center().x, kr.bottom() - 2.5 * px), 1.4 * px, dot);
        }
        w += 1;
    }

    // Pass 2: black keys over the boundaries.
    let mut w = 0usize;
    let bw = (kw * 0.65).max(2.0 * px);
    let bh = inner.height() * 0.58;
    for note in 0..=127u8 {
        if !is_black_key(note) {
            w += 1;
            continue;
        }
        let cx_pos = inner.left() + w as f32 * kw;
        let kr = Rect::from_min_max(
            pos2(cx_pos - bw * 0.5, inner.top()),
            pos2(cx_pos + bw * 0.5, inner.top() + bh),
        );
        round_bottom(kr.expand(px * 0.5), key_r + 0.5 * px, bg);
        round_bottom(kr, key_r, black_bg);
        if note >= k_lo && note <= k_hi {
            paint_band(kr, key_r, black_ghost, black_ink);
        }
    }

    Vec::new()
}

pub fn panel() -> Option<Panel> {
    let knobs = (0..SLOTS).map(|slot| {
        knob(knob_id(slot), "")
            .value_fn(cc_text)
            .show(Show::Flag(SHOWN[slot], 1))
    });
    Some(Panel::new(vstack([
        tabs(TABS, "PATCH · VIBRATO · FILTER · MODS · ZONES")
            .h(Size::U(1.25))
            .show(NO_CONTROLS),
        tabs(TABS_SFZ, "PATCH · VIBRATO · FILTER · CONTROLS")
            .h(Size::U(1.25))
            .show(WITH_CONTROLS),
        vstack([
            hstack([
                text_toggle(1100, "Prev")
                    .momentary()
                    .margins(0.75, 0.75, 0.75, 0.75),
                spinner(1200, "Patch").items(&["Load Patch"]).picker(),
                text_toggle(1300, "Next")
                    .momentary()
                    .margins(0.75, 0.75, 0.75, 0.75),
                knob(2800, "Fine Range").p(16).w(Size::U(2.5)),
            ])
            .h(Size::U(2.596)),
            hstack([
                knob(2600, "Transpose").p(11),
                knob(2700, "Fine Tune").p(6).label_fn(fine_tune_label),
                v_slider(2200, "Attack").p(1).value_text(),
                v_slider(2300, "Decay").p(2).value_text(),
                v_slider(2400, "Sustain").p(3).value_text(),
                v_slider(2500, "Release").p(4).value_text(),
            ]),
        ])
        .show(PATCH_SHOWN),
        hstack([
            toggle(3100, "Vibrato")
                .p(15)
                .value_text()
                .value_fn(envelope_text),
            v_slider(3200, "Vib Rate")
                .p(7)
                .value_text()
                .enabled(VIB_CUSTOM),
            v_slider(3300, "Vib Depth")
                .p(8)
                .value_text()
                .enabled(VIB_CUSTOM),
            v_slider(3400, "Vib Delay")
                .p(9)
                .value_text()
                .enabled(VIB_CUSTOM),
        ])
        .show(VIB_SHOWN),
        hstack([
            toggle(3500, "Filter")
                .p(12)
                .value_text()
                .value_fn(envelope_text),
            v_slider(3600, "Cutoff")
                .p(13)
                .value_text()
                .enabled(FILTER_CUSTOM),
            v_slider(3700, "Resonance")
                .p(14)
                .value_text()
                .enabled(FILTER_CUSTOM),
            toggle(3800, "Quality")
                .p(10)
                .value_text()
                .value_fn(quality_text),
        ])
        .show(FILTER_SHOWN),
        vstack([
            hstack([
                text_toggle(MOD_PREV, "<")
                    .momentary()
                    .w(Size::U(0.9))
                    .margins(0.06, 0.05, 0.06, 0.05),
                text(MOD_LABEL, "Mod 1/8")
                    .align(Align2::CENTER_CENTER)
                    .ink(Ink::Text)
                    .w(Size::U(4.5)),
                text_toggle(MOD_NEXT, ">")
                    .momentary()
                    .w(Size::U(0.9))
                    .margins(0.06, 0.05, 0.06, 0.05),
                spacer(7208).w(Size::Flex(1.0)),
                text_toggle(MOD_CLEAR, "Clear")
                    .momentary()
                    .w(Size::U(2.2))
                    .margins(0.06, 0.1, 0.06, 0.1),
            ])
            .h(Size::U(1.1))
            .margins(0.12, 0.35, 0.08, 0.35),
            hstack([
                spinner(7210, "Source")
                    .items(&MOD_SRC_ITEMS)
                    .items_per_column(5),
                spinner(7211, "Curve").items(&MOD_CURVE_ITEMS),
                spinner(7212, "Dest")
                    .items(&MOD_DEST_ITEMS)
                    .items_per_column(6),
                knob(7213, "Amount").default(0.5).value_fn(mod_amt_text),
            ])
            .ml(0.3)
            .mr(0.3),
            display(MOD_GRID)
                .h(Size::U(2.0))
                .margins(0.0, 0.0, 0.0, 0.0)
                .draw(mod_grid),
        ])
        .show(MODS_SHOWN),
        vstack([
            hstack([
                text_toggle(ZONE_PREV, "<")
                    .momentary()
                    .w(Size::U(0.9))
                    .margins(0.06, 0.05, 0.06, 0.05),
                text(ZONE_LABEL, "")
                    .align(Align2::CENTER_CENTER)
                    .ink(Ink::Text)
                    .w(Size::U(5.8)),
                text_toggle(ZONE_NEXT, ">")
                    .momentary()
                    .w(Size::U(0.9))
                    .margins(0.06, 0.05, 0.06, 0.05),
                spacer(7008).w(Size::Flex(1.0)),
                text_toggle(ZONE_RESET, "Reset")
                    .momentary()
                    .w(Size::U(2.0))
                    .margins(0.06, 0.1, 0.06, 0.1),
            ])
            .h(Size::U(1.05))
            .margins(0.08, 0.35, 0.02, 0.35),
            display(ZONE_PAGE)
                .h(Size::U(0.95))
                .margins(0.0, 0.35, 0.08, 0.35)
                .draw(zone_tabs),
            hstack([
                knob(7128, "Delay").value_fn(mod_time_text),
                v_slider(7100, "Attack").value_text().value_fn(zone_time_text),
                knob(7129, "Hold").value_fn(mod_time_text),
                v_slider(7101, "Decay").value_text().value_fn(zone_time_text),
                v_slider(7102, "Sustain").default(1.0).value_text().value_fn(zone_pct_text),
                v_slider(7103, "Release").value_text().value_fn(zone_time_text),
            ])
            .ml(0.2)
            .mr(0.2)
            .show(Show::Flag(ZONE_PAGE_FLAG, 0)),
            hstack([
                knob(7118, "Delay").value_fn(mod_time_text),
                v_slider(7119, "Attack").value_text().value_fn(mod_time_text),
                knob(7120, "Hold").value_fn(mod_time_text),
                v_slider(7121, "Decay").value_text().value_fn(mod_time_text),
                v_slider(7122, "Sustain").default(1.0).value_text().value_fn(zone_pct_text),
                v_slider(7123, "Release").value_text().value_fn(mod_time_text),
            ])
            .ml(0.2)
            .mr(0.2)
            .show(Show::Flag(ZONE_PAGE_FLAG, 1)),
            hstack([
                knob(7124, "Mod->Pit").default(0.5).value_fn(mod_env_pitch_text),
                knob(7125, "Mod->Fil").default(0.5).value_fn(mod_filter_text),
                knob(7130, "Vol KeyH").default(0.5).value_fn(mod_key_text),
                knob(7131, "Vol KeyD").default(0.5).value_fn(mod_key_text),
                knob(7126, "Mod KeyH").default(0.5).value_fn(mod_key_text),
                knob(7127, "Mod KeyD").default(0.5).value_fn(mod_key_text),
            ])
            .ml(0.2)
            .mr(0.2)
            .show(Show::Flag(ZONE_PAGE_FLAG, 2)),
            hstack([
                knob(7104, "Rate").default(8.176 / 20.0).value_fn(zone_rate_text),
                v_slider(7105, "Depth").default(0.5).value_text().value_fn(zone_cents_text),
                v_slider(7106, "Delay").value_text().value_fn(zone_delay_text),
            ])
            .ml(0.2)
            .mr(0.2)
            .show(Show::Flag(ZONE_PAGE_FLAG, 3)),
            hstack([
                knob(7107, "Rate").default(8.176 / 20.0).value_fn(zone_rate_text),
                v_slider(7109, "Delay").value_text().value_fn(zone_delay_text),
                knob(7108, "-> Pitch").default(0.5).value_fn(mod_lfo_pitch_text),
                knob(7116, "-> Filter").default(0.5).value_fn(mod_filter_text),
                knob(7117, "-> Vol").default(0.5).value_fn(mod_vol_text),
            ])
            .ml(0.2)
            .mr(0.2)
            .show(Show::Flag(ZONE_PAGE_FLAG, 4)),
            hstack([
                knob(7110, "Cutoff").default(1.0).value_fn(zone_cutoff_text),
                knob(7111, "Resonance").value_fn(zone_res_text),
                knob(7112, "Tune").default(0.5).value_fn(zone_tune_text),
                knob(7113, "Fine").default(0.5).value_fn(zone_cents_text),
                knob(7138, "Fine x").value_fn(zone_mult_text),
                knob(7114, "Pan").default(0.5).value_fn(zone_pan_text),
                knob(7115, "Level").default(0.8).value_fn(zone_db_text),
            ])
            .ml(0.2)
            .mr(0.2)
            .show(Show::Flag(ZONE_PAGE_FLAG, 5)),
            hstack([
                knob(7134, "Start").value_fn(zone_pct1_text),
                knob(7135, "End").default(1.0).value_fn(zone_pct1_text),
                spinner(7132, "Loop").items(&LOOP_ITEMS),
                spinner(7133, "Excl")
                    .items(&EXCL_ITEMS)
                    .items_per_column(11),
                knob(7136, "Fix Key").value_fn(zone_fixed_key_text),
                knob(7137, "Fix Vel").value_fn(zone_fixed_vel_text),
            ])
            .ml(0.2)
            .mr(0.2)
            .show(Show::Flag(ZONE_PAGE_FLAG, 6)),
            display(ZONE_PIANO)
                .h(Size::U(1.45))
                .margins(0.0, 0.0, 0.0, 0.0)
                .draw(zone_piano),
        ])
        .show(ZONE_LIST),
        text(7006, "No zones")
            .font(FontSize::Pt18)
            .align(Align2::CENTER_CENTER)
            .ink(Ink::Text)
            .show(NO_ZONES),
        hstack(knobs).show(CONTROLS_SHOWN),
        text(3000, "...")
            .font(FontSize::Pt18)
            .align(Align2::CENTER_CENTER)
            .ink(Ink::Text)
            .show(Show::Not(&IDLE)),
    ])))
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [16.96, 6.49],
    settings: &[
        (1100, 0.0),
        (1200, 0.0),
        (1300, 0.0),
        (4100, 0.0),
        (4200, 0.0),
        (4300, 0.0),
        (4400, 0.0),
        (4500, 0.0),
        (4600, 0.0),
        (4700, 0.0),
        (4800, 0.0),
    ],
    cases: &[
        // An SFZ that names eight controls, on its `CONTROLS` page.
        Case {
            name: "controls",
            tab: 1,
            flags: &[
                (HAS_CONTROLS, 1),
                (SHOWN[0], 1),
                (SHOWN[1], 1),
                (SHOWN[2], 1),
                (SHOWN[3], 1),
                (SHOWN[4], 1),
                (SHOWN[5], 1),
                (SHOWN[6], 1),
                (SHOWN[7], 1),
            ],
            rows: &[
                row(
                    TABS,
                    Widget::Tabs,
                    "PATCH · CONTROLS",
                    [0.0, 0.0, 16.96, 1.25],
                )
                .h(Size::U(1.25)),
                row(4100, Widget::Knob, "", [0.00, 1.25, 2.12, 5.24]).value("0"),
                row(4200, Widget::Knob, "", [2.12, 1.25, 2.12, 5.24]).value("0"),
                row(4300, Widget::Knob, "", [4.24, 1.25, 2.12, 5.24]).value("0"),
                row(4400, Widget::Knob, "", [6.36, 1.25, 2.12, 5.24]).value("0"),
                row(4500, Widget::Knob, "", [8.48, 1.25, 2.12, 5.24]).value("0"),
                row(4600, Widget::Knob, "", [10.60, 1.25, 2.12, 5.24]).value("0"),
                row(4700, Widget::Knob, "", [12.72, 1.25, 2.12, 5.24]).value("0"),
                row(4800, Widget::Knob, "", [14.84, 1.25, 2.12, 5.24]).value("0"),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "loaded",
            rows: &[
                // Transpose is a Honeydew addition (#102) beside Prev, Patch and Next.
                row(1100, Widget::TextToggle, "Prev", [0.75, 0.75, 3.82, 1.09])
                    .margins(0.75, 0.75, 0.75, 0.75)
                    .momentary()
                    .slack(1.0, "spec f32 trace"),
                row(1200, Widget::Spinner, "Patch", [5.32, 0.0, 3.82, 2.59])
                    .items("Load Patch")
                    .picker()
                    .slack(1.0, "spec f32 trace"),
                row(1300, Widget::TextToggle, "Next", [9.89, 0.75, 3.82, 1.09])
                    .margins(0.75, 0.75, 0.75, 0.75)
                    .momentary()
                    .slack(1.0, "spec f32 trace"),
                row(3200, Widget::Knob, "Transpose", [14.46, 0.0, 2.50, 2.59])
                    .p(11)
                    .w(Size::U(2.5)),
                // Ten columns: Fine Tune (#90), the vibrato and Quality (#102) are Honeydew additions.
                row(2100, Widget::Toggle, "Envelope", [0.00, 2.59, 1.69, 3.90])
                    .p(5)
                    .value_text()
                    .value("Original"),
                row(2200, Widget::VSlider, "Attack", [1.69, 2.59, 1.70, 3.90])
                    .p(1)
                    .value_text()
                    .enabled(CUSTOM),
                row(2300, Widget::VSlider, "Decay", [3.39, 2.59, 1.69, 3.90])
                    .p(2)
                    .value_text()
                    .enabled(CUSTOM),
                row(2400, Widget::VSlider, "Sustain", [5.08, 2.59, 1.70, 3.90])
                    .p(3)
                    .value_text()
                    .enabled(CUSTOM),
                row(2500, Widget::VSlider, "Release", [6.78, 2.59, 1.70, 3.90])
                    .p(4)
                    .value_text()
                    .enabled(CUSTOM),
                row(2600, Widget::VSlider, "Fine Tune", [8.48, 2.59, 1.69, 3.90])
                    .p(6)
                    .value_text(),
                row(2700, Widget::VSlider, "Vib Rate", [10.17, 2.59, 1.70, 3.90])
                    .p(7)
                    .value_text(),
                row(
                    2800,
                    Widget::VSlider,
                    "Vib Depth",
                    [11.87, 2.59, 1.69, 3.90],
                )
                .p(8)
                .value_text(),
                row(
                    2900,
                    Widget::VSlider,
                    "Vib Delay",
                    [13.56, 2.59, 1.70, 3.90],
                )
                .p(9)
                .value_text(),
                row(3100, Widget::Toggle, "Quality", [15.26, 2.59, 1.69, 3.90])
                    .p(10)
                    .value_text()
                    .value("Linear"),
            ],
            ..Case::DEFAULT
        },
        // Honeydew change (#135): a portrait phone's two rows of five (the card wiring's flag).
        Case {
            name: "two rows",
            flags: &[(TWO_ROWS, 1)],
            rows: &[
                row(1100, Widget::TextToggle, "Prev", [0.75, 0.75, 3.82, 1.09])
                    .margins(0.75, 0.75, 0.75, 0.75)
                    .momentary()
                    .slack(1.0, "spec f32 trace"),
                row(1200, Widget::Spinner, "Patch", [5.32, 0.0, 3.82, 2.59])
                    .items("Load Patch")
                    .picker()
                    .slack(1.0, "spec f32 trace"),
                row(1300, Widget::TextToggle, "Next", [9.89, 0.75, 3.82, 1.09])
                    .margins(0.75, 0.75, 0.75, 0.75)
                    .momentary()
                    .slack(1.0, "spec f32 trace"),
                row(3200, Widget::Knob, "Transpose", [14.46, 0.0, 2.50, 2.59])
                    .p(11)
                    .w(Size::U(2.5)),
                row(5100, Widget::Toggle, "Envelope", [0.00, 2.59, 3.39, 1.95])
                    .p(5)
                    .value_text()
                    .value("Original"),
                row(5200, Widget::VSlider, "Attack", [3.39, 2.59, 3.39, 1.95])
                    .p(1)
                    .value_text()
                    .enabled(CUSTOM),
                row(5300, Widget::VSlider, "Decay", [6.78, 2.59, 3.39, 1.95])
                    .p(2)
                    .value_text()
                    .enabled(CUSTOM),
                row(5400, Widget::VSlider, "Sustain", [10.17, 2.59, 3.39, 1.95])
                    .p(3)
                    .value_text()
                    .enabled(CUSTOM),
                row(5500, Widget::VSlider, "Release", [13.56, 2.59, 3.40, 1.95])
                    .p(4)
                    .value_text()
                    .enabled(CUSTOM),
                row(5600, Widget::VSlider, "Fine Tune", [0.00, 4.54, 3.39, 1.95])
                    .p(6)
                    .value_text(),
                row(5700, Widget::VSlider, "Vib Rate", [3.39, 4.54, 3.39, 1.95])
                    .p(7)
                    .value_text(),
                row(5800, Widget::VSlider, "Vib Depth", [6.78, 4.54, 3.39, 1.95])
                    .p(8)
                    .value_text(),
                row(
                    5900,
                    Widget::VSlider,
                    "Vib Delay",
                    [10.17, 4.54, 3.39, 1.95],
                )
                .p(9)
                .value_text(),
                row(6100, Widget::Toggle, "Quality", [13.56, 4.54, 3.40, 1.95])
                    .p(10)
                    .value_text()
                    .value("Linear"),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "loading",
            flags: &[(flag::LOADING, 1)],
            rows: &[MESSAGE_ROW],
            ..Case::DEFAULT
        },
        // The wiring puts the error in place of `...` (`texts`).
        Case {
            name: "failed",
            flags: &[(flag::LOADING, 2)],
            rows: &[MESSAGE_ROW],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
const MESSAGE_ROW: Row = row(3000, Widget::Text, "...", [0.0, 0.0, 16.96, 6.49])
    .font(FontSize::Pt18)
    .align(Align2::CENTER_CENTER)
    .ink(Ink::Text);

#[cfg(test)]
mod tests {
    use eframe::egui::{Rect, pos2};

    use super::*;
    use crate::flm::chunk::ContentRoot;
    use crate::panels::tests::{Rig, U};

    #[test]
    fn the_envelope_toggle_reads_original_off_and_custom_on() {
        assert_eq!(super::envelope_text(0.0), "Original");
        // Original while v ≤ 0.5.
        assert_eq!(super::envelope_text(0.5), "Original");
        assert_eq!(super::envelope_text(1.0), "Custom");
    }

    #[test]
    fn the_envelope_sliders_are_enabled_only_while_it_is_custom() {
        use eframe::egui::{Color32, Event, PointerButton, Shape, vec2};
        // `Original` while v ≤ 0.5, else `Custom`; the four sliders are enabled only when Custom. A disabled
        // slider's fill and thumb centre are menuFill, not the channel's.
        let menu = crate::theme::theme_by_id(0).colors.menu_fill.to_color32();
        for (envelope, enabled) in [(0.0, false), (0.5, false), (0.51, true), (1.0, true)] {
            let mut rig = rig();
            rig.module.params[5] = envelope;
            for p in 1..=4 {
                rig.module.params[p] = 0.5;
            }
            rig.frame(vec![]);
            for (id, p) in [(2200, 1), (2300, 2), (2400, 3), (2500, 4)] {
                let what = format!("#{id} at Envelope {envelope}");
                let r = rig.rect_of(id);
                let inks: Vec<Color32> = (rig.shapes.iter())
                    .filter_map(|c| match &c.shape {
                        Shape::Circle(c) if r.contains(c.center) => Some(c.fill),
                        _ => None,
                    })
                    .collect();
                let ink = if enabled { Color32::RED } else { menu };
                assert_eq!(inks.last(), Some(&ink), "{what}");
                // A tap changes nothing either way. Each gesture starts a second after the last: no double tap.
                let at = r.center();
                rig.time += 1.0;
                rig.click(at);
                assert_eq!(
                    (&rig.changed, rig.module.params[p]),
                    (&vec![], 0.5),
                    "{what}"
                );
                // A drag all the way up (the thumb follows the pointer), then a double tap (Reset, to the registry's
                // default).
                let up = at - vec2(0.0, 10.0 * U);
                rig.time += 1.0;
                rig.press(at, PointerButton::Primary, true);
                rig.frame(vec![Event::PointerMoved(up)]);
                let dragged = (rig.changed.clone(), rig.module.params[p]);
                rig.press(up, PointerButton::Primary, false);
                rig.time += 1.0;
                rig.click(at);
                rig.click(at);
                let reset = rig.module.params[p];
                let default = ModuleInstance::new(52, 2).params[p];
                if enabled {
                    assert_eq!((dragged, reset), ((vec![p], 1.0), default), "{what}");
                } else {
                    assert_eq!((dragged, reset), ((vec![], 0.5), 0.5), "{what}");
                }
            }
        }
        assert_eq!(super::envelope_text(0.5), "Original");
        assert_eq!(super::envelope_text(0.51), "Custom");
    }

    /// A double tap resets each envelope slider to its default (0, 0, 1, 0).
    #[test]
    fn a_double_tap_resets_the_envelope_sliders_to_the_spec_defaults() {
        let mut rig = rig();
        rig.module.params[5] = 1.0;
        for (id, p, default) in [
            (2200, 1, 0.0),
            (2300, 2, 0.0),
            (2400, 3, 1.0),
            (2500, 4, 0.0),
        ] {
            rig.module.params[p] = 0.5;
            rig.frame(vec![]);
            let at = rig.rect_of(id).center();
            rig.time += 1.0;
            rig.click(at);
            rig.click(at);
            assert_eq!(rig.module.params[p], default, "#{id}");
        }
    }

    /// The content rect of the 8.5 U card: 16.96 × 6.49 U.
    const RECT: Rect = Rect {
        min: pos2(10.0, 20.0),
        max: pos2(10.0 + 16.96 * U, 20.0 + 6.49 * U),
    };

    fn rig() -> Rig {
        Rig::new(52, panel().unwrap(), RECT)
    }

    /// Where the last frame drew `text`.
    fn drawn(rig: &Rig, text: &str) -> Vec<Rect> {
        let texts = rig.texts().into_iter();
        texts.filter(|(t, _)| t == text).map(|(_, r)| r).collect()
    }

    #[test]
    fn the_patch_spinner_reads_load_patch_until_the_wiring_names_a_patch() {
        let mut rig = rig();
        let spinner = rig.rect_of(PATCH);
        let at = drawn(&rig, "Load Patch");
        assert!(at.len() == 1 && spinner.contains(at[0].center()), "{at:?}");
        rig.state.texts.insert(PATCH, "Grand Piano".into());
        rig.frame(vec![]);
        let at = drawn(&rig, "Grand Piano");
        assert!(at.len() == 1 && spinner.contains(at[0].center()), "{at:?}");
        assert_eq!(drawn(&rig, "Load Patch"), []);
    }

    #[test]
    fn a_tap_anywhere_on_the_patch_spinner_is_reported_and_opens_no_popup() {
        let mut rig = rig();
        let r = rig.rect_of(PATCH);
        // The value text, and the `^` chevron's zone (more than 0.5 U above the body's centre, under the caption).
        let body = (r.top() + 0.75 * U + r.bottom()) / 2.0;
        for at in [pos2(r.center().x, body), pos2(r.center().x, body - 0.7 * U)] {
            rig.click(at);
            assert_eq!(rig.edited_settings, [PATCH], "{at:?}");
            rig.frame(vec![]);
            assert_eq!(drawn(&rig, "Load Patch").len(), 1, "a popup opened");
            assert_eq!(rig.state.settings[&PATCH], 0.0);
        }
    }

    #[test]
    fn a_failed_load_shows_its_message_in_place_of_the_controls() {
        let mut rig = rig();
        rig.state.flags.insert(flag::LOADING, 2);
        rig.state
            .texts
            .insert(MESSAGE, "Corrupted SoundFont file".into());
        rig.frame(vec![]);
        let placed = crate::panels::place(&rig.panel, &rig.module, &rig.state, RECT, U);
        let ids: Vec<u32> = placed.iter().map(|(l, _)| l.id).collect();
        assert_eq!(ids, [MESSAGE]);
        let at = drawn(&rig, "Corrupted SoundFont file");
        assert!(
            at.len() == 1 && (at[0].center() - RECT.center()).length() < 1.0,
            "{at:?}"
        );
        assert_eq!(drawn(&rig, "..."), []);
        assert_eq!(drawn(&rig, "Prev"), []);
    }

    fn piano() -> ContentPath {
        ContentPath::Shorthand {
            root: ContentRoot::User,
            relative: "My SoundFonts/Piano.sf2".into(),
        }
    }

    /// A SoundFont Player playing patch `patch` of `My SoundFonts/Piano.sf2`.
    fn player(patch: i32) -> ModuleInstance {
        let mut module = ModuleInstance::new(52, 2);
        let sf2d = Sf2d {
            path: piano(),
            patch,
        };
        module.file.custom_data = Some(sf2d.write(None).unwrap());
        module
    }

    fn patch(module: &ModuleInstance) -> i32 {
        Sf2d::read(module.file.custom_data.as_deref().unwrap())
            .unwrap()
            .patch
    }

    /// `Piano.sf2` read as `read`, its patches named `names`.
    fn fonts(read: Result<&[&str], Sf2Error>) -> SoundFonts {
        let patch = |(i, name): (usize, &&str)| Preset {
            name: (*name).into(),
            preset: i as u16,
            ..Preset::default()
        };
        let read = read.map(|names| names.iter().enumerate().map(patch).collect());
        SoundFonts::from([(piano(), read)])
    }

    #[test]
    fn wire_names_the_patch_or_sets_the_load_state() {
        let abc = fonts(Ok(&["A", "B", "C"]));
        let wired = |module: &ModuleInstance, fonts: &SoundFonts| {
            // A stale state from before: the wiring replaces it.
            let mut state = PanelState::default();
            state.texts.insert(PATCH, "Old".into());
            state.texts.insert(MESSAGE, "Old".into());
            state.flags.insert(flag::LOADING, 2);
            wire(module, &mut state, fonts);
            let loading = state.flags.get(flag::LOADING).copied().unwrap_or(0);
            let text = |n| state.texts.get(&n).cloned();
            (loading, text(PATCH), text(MESSAGE))
        };
        let some = |t: &str| Some(t.to_owned());
        assert_eq!(wired(&ModuleInstance::new(52, 2), &abc), (0, None, None));
        assert_eq!(wired(&player(1), &abc), (0, some("B"), None));
        assert_eq!(wired(&player(9), &abc), (0, some("C"), None));
        assert_eq!(wired(&player(-1), &abc), (0, some("A"), None));
        let bad = fonts(Err(Sf2Error::Corrupted));
        assert_eq!(
            wired(&player(0), &bad),
            (2, None, some("Corrupted SoundFont file"))
        );
        assert_eq!(wired(&player(0), &SoundFonts::new()), (1, None, None));
    }

    #[test]
    fn an_sfzs_named_controls_are_knobs_set_once_to_their_set_cc_that_send_their_ccs() {
        // The patch an SFZ lists: its `label_ccN`s, at their `set_ccN`.
        let text = b"<control> label_cc1=Vibrato set_cc74=127 label_cc74=Bright Ness\n<region> sample=*sine";
        let path = std::path::Path::new("My SoundFonts/Lead.sfz");
        let preset = crate::sf2::patches(path, text).unwrap().remove(0);
        let control = |cc, label: &str, value| Control {
            cc,
            label: label.into(),
            value,
        };
        let want = [control(1, "Vibrato", 0.0), control(74, "Bright Ness", 1.0)];
        assert_eq!(
            (preset.name.as_str(), &preset.controls[..]),
            ("Lead", &want[..])
        );
        let fonts = SoundFonts::from([(piano(), Ok(vec![preset]))]);
        let module = player(0);
        let mut state = PanelState::default();
        // The first wiring names the knobs, sets them and sends their CCs (channel params 7 + n); later ones keep them.
        assert_eq!(wire(&module, &mut state, &fonts), [(8, 0.0), (81, 1.0)]);
        let knob = |n: u32| KNOBS + 100 * n;
        assert_eq!(
            [&state.texts[&knob(0)], &state.texts[&knob(1)]],
            ["Vibrato", "Bright Ness"]
        );
        assert_eq!(state.settings[&knob(1)], 1.0);
        state.settings.insert(knob(1), 0.5);
        assert_eq!(wire(&module, &mut state, &fonts), []);
        assert_eq!(state.settings[&knob(1)], 0.5);
        assert_eq!(cc_moves(&module, &state, &[knob(1)], &fonts), [(81, 0.5)]);
        // `CONTROLS` shows the two knobs under their names, in place of the patch's controls.
        let mut rig = rig();
        (rig.module, rig.state) = (module, state);
        rig.state.tab = 1;
        rig.frame(vec![]);
        assert_eq!(drawn(&rig, "Bright Ness").len(), 1);
        assert_eq!(drawn(&rig, "Prev"), []);
        let placed = crate::panels::place(&rig.panel, &rig.module, &rig.state, RECT, U);
        let ids: Vec<u32> = placed.iter().map(|(l, _)| l.id).collect();
        assert_eq!(ids, [TABS, knob(0), knob(1)]);
    }

    #[test]
    fn prev_and_next_step_through_the_patches_on_the_press_and_wrap() {
        let abc = fonts(Ok(&["A", "B", "C"]));
        let pressed = |id: u32, on: f32| {
            let mut state = PanelState::default();
            state.settings.insert(id, on);
            state
        };
        let mut module = player(1);
        for (id, want) in [(NEXT, 2), (NEXT, 0), (PREV, 2), (PREV, 1)] {
            let got = press(&mut module, &pressed(id, 1.0), &[id], &abc);
            assert_eq!((got, patch(&module)), (Some(Press::Stepped), want), "#{id}");
        }
        // The release, another control, no SoundFont read, no SoundFont: nothing.
        assert_eq!(press(&mut module, &pressed(NEXT, 0.0), &[NEXT], &abc), None);
        assert_eq!(press(&mut module, &pressed(NEXT, 1.0), &[2100], &abc), None);
        let bad = fonts(Err(Sf2Error::NotFound));
        assert_eq!(press(&mut module, &pressed(NEXT, 1.0), &[NEXT], &bad), None);
        assert_eq!(patch(&module), 1);
        let mut empty = ModuleInstance::new(52, 2);
        assert_eq!(press(&mut empty, &pressed(NEXT, 1.0), &[NEXT], &abc), None);
        assert_eq!(empty.file.custom_data, None);
    }
}

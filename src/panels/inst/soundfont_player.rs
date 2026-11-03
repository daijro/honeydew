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

use std::collections::HashMap;

use eframe::egui::{Id, Rect, Sense, Ui};

use crate::flm::chunk::ContentPath;
use crate::flm::sf2d::Sf2d;
use crate::panels::build::*;
use crate::sf2::{Preset, Sf2Error};

/// The SoundFont Player's module type.
pub const TYPE_ID: u16 = 52;

/// The `#n` the wiring drives: `Prev`, the `Patch` spinner, `Next` and the `...` / error text.
pub const PREV: u32 = 1100;
pub const PATCH: u32 = 1200;
pub const NEXT: u32 = 1300;
pub const MESSAGE: u32 = 3000;

/// The patches of each SoundFont the rack's SoundFont Players name, by their `SF2D` path, as read
/// ([`crate::project_files::ProjectFiles::soundfont`]): the host reads each one once.
pub type SoundFonts = HashMap<ContentPath, Result<Vec<Preset>, Sf2Error>>;

/// Sets a card's `state` for `module`'s `SF2D` before the panel draws:
/// - no SoundFont: the spinner's `Load Patch`;
/// - a SoundFont `fonts` holds the patches of: the patch's name on the spinner (an index past either end reads as
///   the patch at that end);
/// - one it could not read: the error state (flag `loading` 2), its message in place of `...`;
/// - one `fonts` lacks: loading (flag `loading` 1).
pub fn wire(module: &ModuleInstance, state: &mut PanelState, fonts: &SoundFonts) {
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
    let Ok(blob) = sf2d.write(blob) else {
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

/// Loaded, or nothing to load: the controls show.
const IDLE: Show = Show::Flag(flag::LOADING, 0);

/// `Envelope` is `Custom` (above 0.5): the sliders' envelope, which only then takes their input.
const CUSTOM: Show = Show::Above(5, 0.5);

/// The `Envelope` toggle's state text: `Original` keeps the SoundFont's own envelopes, `Custom` uses the sliders'.
fn envelope_text(v: f32) -> String {
    if v > 0.5 { "Custom" } else { "Original" }.to_owned()
}

pub fn panel() -> Option<Panel> {
    Some(Panel::new(vstack([
        hstack([
            text_toggle(1100, "Prev")
                .momentary()
                .margins(0.75, 0.75, 0.75, 0.75),
            // The wiring's text (`wire`) once a patch is chosen.
            spinner(1200, "Patch").items(&["Load Patch"]).picker(),
            text_toggle(1300, "Next")
                .momentary()
                .margins(0.75, 0.75, 0.75, 0.75),
        ])
        .show(IDLE),
        hstack([
            toggle(2100, "Envelope")
                .p(5)
                .value_text()
                .value_fn(envelope_text),
            v_slider(2200, "Attack").p(1).value_text().enabled(CUSTOM),
            v_slider(2300, "Decay").p(2).value_text().enabled(CUSTOM),
            v_slider(2400, "Sustain").p(3).value_text().enabled(CUSTOM),
            v_slider(2500, "Release").p(4).value_text().enabled(CUSTOM),
        ])
        .h(Size::Flex(1.5))
        .show(IDLE),
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
    settings: &[(1100, 0.0), (1200, 0.0), (1300, 0.0)],
    cases: &[
        Case {
            name: "loaded",
            rows: &[
                row(1100, Widget::TextToggle, "Prev", [0.75, 0.74, 4.65, 1.10])
                    .margins(0.75, 0.75, 0.75, 0.75)
                    .momentary()
                    .slack(1.0, "spec f32 trace"),
                row(1200, Widget::Spinner, "Patch", [6.15, -0.01, 4.65, 2.60])
                    .items("Load Patch")
                    .picker()
                    .slack(1.0, "spec f32 trace"),
                row(1300, Widget::TextToggle, "Next", [11.55, 0.74, 4.66, 1.10])
                    .margins(0.75, 0.75, 0.75, 0.75)
                    .momentary()
                    .slack(1.0, "spec f32 trace"),
                row(2100, Widget::Toggle, "Envelope", [0.00, 2.59, 3.39, 3.90])
                    .p(5)
                    .value_text()
                    .value("Original"),
                row(2200, Widget::VSlider, "Attack", [3.39, 2.59, 3.39, 3.90])
                    .p(1)
                    .value_text()
                    .enabled(CUSTOM),
                row(2300, Widget::VSlider, "Decay", [6.78, 2.59, 3.39, 3.90])
                    .p(2)
                    .value_text()
                    .enabled(CUSTOM),
                row(2400, Widget::VSlider, "Sustain", [10.17, 2.59, 3.39, 3.90])
                    .p(3)
                    .value_text()
                    .enabled(CUSTOM),
                row(2500, Widget::VSlider, "Release", [13.56, 2.59, 3.40, 3.90])
                    .p(4)
                    .value_text()
                    .enabled(CUSTOM),
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
            bank: 0,
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

//! The keymap's editing and zoom actions in the editor grid that holds the keyboard: `copy`, `cut`, `delete`, `paste`,
//! `selectAll`, `close`, `open` and `zoom{In,Out}{Horizontal,Vertical}`. The focused control is asked before the app
//! root, so the editors take them in their own frame, before the global actions (`app/keys.rs`).
//!
//! - The grid holds the keyboard while the app says so ([`set_focus`]), no text field has the keyboard and no popup
//!   shows. The app says so for the focus container holding the focus (`ui/focus.rs`: the playlist's or the bottom
//!   editor's) while no overlay is over the view. The editors ask at the end of their frame, after their own popups,
//!   which take Escape first. An editor drawn without the app (a test rig) holds it.
//!   While nothing has the focus, the open bottom editor's grid holds the editing keys, else the playlist's, so copy
//!   and paste work before any tap.
//! - One editing action a frame, the first pressed in [`ACTIONS`] order. An auto-repeat does nothing.
//! - Zoom: z (content units a point) ← z·(1 ∓ 0.1) a press. The editors' [`Scroller`] applies it about the view's
//!   centre, clamped to the axis's zoom range ([`zoom`]).
//!
//! [`Scroller`]: crate::ui::scroller::Scroller

use eframe::egui::{Context, Id, InputState};

use crate::keymap::{self, Keymap};

/// An editing action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKey {
    Copy,
    Cut,
    Delete,
    Paste,
    SelectAll,
    /// `Deselect` (`Cancel selection`).
    Close,
    /// `Add note` / `Add point` / `Add clip`.
    Open,
}

const ACTIONS: [(&str, EditKey); 7] = [
    ("cut", EditKey::Cut),
    ("copy", EditKey::Copy),
    ("paste", EditKey::Paste),
    ("delete", EditKey::Delete),
    ("selectAll", EditKey::SelectAll),
    ("close", EditKey::Close),
    ("open", EditKey::Open),
];

/// The zoom actions: the axis (`true`: vertical) and the step of z. The Shift (vertical) ones first: a binding
/// ignores an extra Shift.
const ZOOMS: [(&str, bool, f64); 4] = [
    ("zoomInVertical", true, -0.1),
    ("zoomOutVertical", true, 0.1),
    ("zoomInHorizontal", false, -0.1),
    ("zoomOutHorizontal", false, 0.1),
];

/// The first editing action of `keys` pressed in `input`, its key consumed.
pub fn take_from(keys: &Keymap, input: &mut InputState) -> Option<EditKey> {
    (ACTIONS.iter())
        .find(|(action, _)| keys.consume_press(input, action))
        .map(|&(_, k)| k)
}

/// The temp-data key of [`set_focus`].
const FOCUS: &str = "edit keys grid focus";

/// Says whether the grid drawn next (the playlist's, then the bottom editor's) holds the keyboard this pass; the app
/// calls it before each, from the focus containers.
pub fn set_focus(ctx: &Context, on: bool) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(Id::new(FOCUS), (pass, on)));
}

/// The grid holds the keyboard now: the app says so this pass (or nothing says otherwise), and no text field or
/// popup has it.
fn holds(ctx: &Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    let set = ctx.data(|d| d.get_temp::<(u64, bool)>(Id::new(FOCUS)));
    let focused = set.is_none_or(|(at, on)| at != pass || on);
    focused && !ctx.wants_keyboard_input() && !crate::ui::modal_open(ctx)
}

/// This frame's editing action for a grid that holds the keyboard ([`holds`]).
pub fn take(ctx: &Context) -> Option<EditKey> {
    let keys = keymap::for_os(ctx.os());
    holds(ctx).then(|| ctx.input_mut(|i| take_from(keys, i)))?
}

/// This frame's `focusUp` (1) or `focusDown` (−1) for a grid that holds the keyboard ([`holds`]) and the arrows
/// ([`crate::ui::focus::hold_vertical`]), and whether Shift is held; taken.
pub fn vertical(ctx: &Context) -> Option<(i32, bool)> {
    let keys = keymap::for_os(ctx.os());
    let take = |i: &mut InputState| {
        let dir = match () {
            _ if keys.consume_press(i, "focusUp") => 1,
            _ if keys.consume_press(i, "focusDown") => -1,
            _ => return None,
        };
        Some((dir, i.modifiers.shift))
    };
    holds(ctx).then(|| ctx.input_mut(take))?
}

/// The zoom actions of `keys` pressed in `input`, their keys consumed: the factor each axis's z (content units a
/// point; x, y) is multiplied by, 1 for none.
pub fn zoom_from(keys: &Keymap, input: &mut InputState) -> [f64; 2] {
    let mut z = [1.0; 2];
    for (action, vertical, step) in ZOOMS {
        if keys.consume_press(input, action) {
            z[usize::from(vertical)] *= 1.0 + step;
        }
    }
    z
}

/// This frame's zoom for a grid that holds the keyboard ([`holds`]), as [`zoom_from`] gives it.
pub fn zoom(ctx: &Context) -> [f64; 2] {
    let keys = keymap::for_os(ctx.os());
    match holds(ctx) {
        true => ctx.input_mut(|i| zoom_from(keys, i)),
        false => [1.0; 2],
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use eframe::egui::{Event, Key, Modifiers, os::OperatingSystem};

    use super::*;

    /// A press of `key` and its release.
    pub(crate) fn tap(key: Key, modifiers: Modifiers) -> [Event; 2] {
        let event = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        [event(true), event(false)]
    }

    fn input(events: &[Event]) -> InputState {
        let mut input = InputState::default();
        input.events = events.to_vec();
        input
    }

    #[test]
    fn the_default_keys_give_each_editing_action() {
        let keys = keymap::for_os(OperatingSystem::Windows);
        let cmd = Modifiers::COMMAND;
        for (key, mods, want) in [
            (Key::C, cmd, EditKey::Copy),
            (Key::X, cmd, EditKey::Cut),
            (Key::V, cmd, EditKey::Paste),
            (Key::Delete, Modifiers::NONE, EditKey::Delete),
            (Key::A, cmd, EditKey::SelectAll),
            (Key::Escape, Modifiers::NONE, EditKey::Close),
            (Key::Enter, Modifiers::NONE, EditKey::Open),
        ] {
            let mut input = input(&tap(key, mods));
            assert_eq!(take_from(keys, &mut input), Some(want), "{key:?}");
            assert!(
                !input
                    .events
                    .iter()
                    .any(|e| matches!(e, Event::Key { pressed: true, .. }))
            );
        }
        assert_eq!(
            take_from(keys, &mut input(&tap(Key::C, Modifiers::NONE))),
            None
        );
    }

    #[test]
    fn a_zoom_press_scales_z_by_one_tenth_and_shift_picks_the_vertical_axis() {
        let keys = keymap::for_os(OperatingSystem::Windows);
        let (cmd, shift) = (Modifiers::COMMAND, Modifiers::COMMAND | Modifiers::SHIFT);
        let z = |key, mods| zoom_from(keys, &mut input(&tap(key, mods)));
        // z (units a point) ← z·(1 ∓ 0.1).
        assert_eq!(z(Key::Equals, cmd), [0.9, 1.0]);
        assert_eq!(z(Key::Minus, cmd), [1.1, 1.0]);
        assert_eq!(z(Key::Equals, shift), [1.0, 0.9]);
        assert_eq!(z(Key::Minus, shift), [1.0, 1.1]);
        assert_eq!(z(Key::Equals, Modifiers::NONE), [1.0; 2]);
    }

    #[test]
    fn a_user_keymap_moves_an_editing_action() {
        let user = r#"{"modifiers":["ctrl"],"actions":[
            {"key":"ctrl+c","action":""},{"key":"ctrl+2","action":"copy"},{"key":"]","action":"zoomInHorizontal"}]}"#;
        let keys = Keymap::parse(user).unwrap();
        let cmd = Modifiers::COMMAND;
        assert_eq!(
            take_from(&keys, &mut input(&tap(Key::Num2, cmd))),
            Some(EditKey::Copy)
        );
        assert_eq!(take_from(&keys, &mut input(&tap(Key::C, cmd))), None);
        let z = zoom_from(&keys, &mut input(&tap(Key::CloseBracket, Modifiers::NONE)));
        assert_eq!(z, [0.9, 1.0]);
    }
}

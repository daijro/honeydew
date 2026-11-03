//! The main menu's SETTINGS tab: the main menu draws [`SettingsTab`] over the app's settings, and the app applies and
//! keeps what changed after the frame.

use crate::ui::settings_tab::SettingsTab;

/// What the settings tab keeps between frames: its open popup, and the MIDI ports it lists for `Input device:`.
/// The main menu view lends the ports from [`AppCtx::midi_devices`](super::AppCtx::midi_devices) each frame and hands
/// the tab's rescan request on to [`AppCtx::midi_rescan`](super::AppCtx::midi_rescan).
pub type State = SettingsTab;

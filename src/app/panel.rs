//! The keyboard panel's layout each frame: the transport bar's rows by the window's width, and the panel's height in U
//! by its rules ([`panel_resize`]):
//! - a change of rows keeps a collapsed panel collapsed and an open one at least the new bar + 0.75 U;
//! - the keyboard settings strip shows a hidden panel as it opens, and opens a collapsed one, or one with no keys area
//!   under it, to min(M, max(recent, c + strip + 6.5 U)); it closes itself once the panel is hidden or collapsed.

use eframe::egui;

use crate::metrics::Metrics;
use crate::settings::Settings;
use crate::ui::keyboard_settings::{panel_target, strip_height};
use crate::ui::main_screen::{MainScreen, Regions, bar_rows};
use crate::ui::panel_resize::{self, Sizes, collapsed};
use crate::ui::views::Overlay;

/// The regions of `screen` for the keyboard panel (before the rack and the strip go in), and the open strip's height
/// in U (0 while closed). `grabbed`: a hand resize let go this frame, which the panel aims at at once. `strip_n`: the
/// strip's controls for the current channel ([`crate::ui::keyboard_settings::strip_controls`]). The app's settings, its
/// keyboard panel and its overlays are the ones it lays out by.
#[allow(clippy::too_many_arguments)]
pub(super) fn panel_layout(
    ctx: &egui::Context,
    screen: egui::Rect,
    grabbed: Option<Sizes>,
    strip_n: usize,
    s: &mut Settings,
    ms: &mut MainScreen,
    overlays: &mut Vec<Overlay>,
    m: &Metrics,
) -> (Regions, f32) {
    let (u, screen_h) = (m.u(1.0), screen.height());
    // The rows by the panel's last height: the right group moves in under a high panel top.
    let top = screen_h - ms.panel.unwrap_or(0.0);
    let rows = bar_rows(screen.width(), top, s.hardware_meters, m);
    let c = collapsed(rows, m);
    if ms.rows != 0 && ms.rows != rows {
        let old = collapsed(ms.rows, m);
        s.preferred_kbd_size = panel_resize::rows_changed(s.preferred_kbd_size * u, old, c, m) / u;
    }
    ms.rows = rows;
    let is_strip = |o: &Overlay| matches!(o, Overlay::KeyboardSettings);
    let mut strip = overlays.iter().any(is_strip);
    let strip_u = strip_height(strip_n, screen.width(), m);
    if strip && !ms.strip_open {
        let (preferred, recent) = (s.preferred_kbd_size * u, s.recent_kbd_size * u);
        let hidden = std::mem::take(&mut ms.panel_hidden);
        let opened =
            panel_resize::strip_opened(preferred, recent, c, m.u(strip_u), screen_h, hidden, m);
        if let Some(h) = opened {
            s.preferred_kbd_size = h / u;
        }
    }
    // A resize let go this frame aims at its settled height at once.
    let preferred = grabbed.map_or(s.preferred_kbd_size, |g| g.preferred / u);
    if strip && (ms.panel_hidden || (preferred * u - c) * m.ppp() <= 0.5) {
        overlays.retain(|o| !is_strip(o));
        strip = false;
    }
    ms.strip_open = strip;
    let target = panel_target(preferred, screen_h, c, m);
    let panel_h = ms.panel_h(ctx, target, m);
    let mut regions = Regions::sized(screen, panel_h, rows, m);
    if ms.panel_hidden {
        regions = regions.without_panel();
    }
    (regions, if strip { strip_u } else { 0.0 })
}

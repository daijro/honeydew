//! A plugin module's card content (`crate::plugins`): a status line (the format and vendor, the plugin's latency,
//! or why it is not running), the `Editor` button (filled in the channel colour) that opens the plugin's own window,
//! and its params as sliders, a page at a time. The sliders are the generic editor where the plugin has no window of
//! its own (or it failed), and each is a bound control: the recent control for REC and the CTRL menu, its moves
//! `SetParam`s.

use eframe::egui::{Align2, Color32, Rect, Ui, pos2, vec2};

use crate::metrics::FontSize;
use crate::plugins::{self, State};
use crate::song::ModuleInstance;
use crate::ui::UiCtx;
use crate::ui::widgets::{self, Button, Control};

/// The status row and the page buttons, in U.
const ROW: f32 = 1.2;
/// One param slider, in U.
const SLIDER: f32 = 1.5;
/// The page buttons' width, in U.
const PAGE_W: f32 = 1.5;
/// The `Editor` button's width, in U.
const EDITOR_W: f32 = 2.6;

/// A plugin card's own state: its page of params.
#[derive(Debug, Default)]
pub(super) struct PluginCard {
    page: usize,
}

/// Draws module `module`'s plugin card in `content` (the module is on the channel with id `channel_id`); `ink` is the
/// channel colour. Returns the params the user moved (module indices, ≥ 1) and the last one pressed.
pub(super) fn show(
    ui: &mut Ui,
    cx: &UiCtx,
    content: Rect,
    channel_id: i32,
    module: &mut ModuleInstance,
    card: &mut PluginCard,
    ink: Color32,
) -> (Vec<i32>, Option<i32>) {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let u = m.u(1.0);
    let text = colors.text.to_color32();
    let id = ui
        .id()
        .with(("plugin card", channel_id, module.instance_id));
    let row = Rect::from_min_size(content.min, vec2(content.width(), ROW * u));
    let state = plugins::state(channel_id, module.instance_id);
    let live = match &state {
        Some(State::Live(info)) => Some(info.clone()),
        _ => None,
    };
    // The status line, left; the editor button and the page buttons, right.
    let mut right = row.right();
    if let Some(info) = live
        .as_ref()
        .filter(|i| i.has_editor && i.stopped.is_none() && i.editor_failed.is_none())
    {
        let rect = Rect::from_min_max(
            row.right_top() - vec2(EDITOR_W * u, 0.0),
            row.right_bottom(),
        );
        right = rect.left();
        let label = if info.editor_open { "Close" } else { "Editor" };
        let b = Button {
            fill_on: ink,
            ..Button::new(
                id.with("editor"),
                rect.shrink(0.1 * u),
                label,
                false,
                colors,
            )
        };
        if widgets::button(ui, cx, &b, true).clicked() {
            // A failure is kept by the host (`Info::editor_failed`) and shown below.
            let _ = plugins::show_editor(channel_id, module.instance_id, !info.editor_open);
        }
    }
    let count = module.params.len().saturating_sub(1);
    let per_page = (((content.height() - ROW * u) / (SLIDER * u)).floor() as usize).max(1);
    let pages = count.div_ceil(per_page).max(1);
    card.page = card.page.min(pages - 1);
    if pages > 1 {
        for (k, (label, step)) in [("<", -1isize), (">", 1)].into_iter().enumerate() {
            let left = right - (2 - k) as f32 * PAGE_W * u;
            let rect = Rect::from_min_size(pos2(left, row.top()), vec2(PAGE_W * u, ROW * u));
            let c = Control {
                label_font: FontSize::Pt10,
                ..Control::new(id.with(("page", k)), rect, label, ink, 0.0)
            };
            if widgets::text_button(ui, cx, &c).clicked() {
                card.page = (card.page as isize + step).rem_euclid(pages as isize) as usize;
            }
        }
        right -= 2.0 * PAGE_W * u;
    }
    let plugin = module.file.plugin.as_deref();
    let status = match (&state, plugin) {
        (_, None) => "No plugin".to_owned(),
        (Some(State::Missing(why)), _) => why.clone(),
        (_, Some(p)) => match &live {
            Some(info) if info.stopped.is_some() => info.stopped.clone().unwrap_or_default(),
            Some(info) if info.editor_failed.is_some() => {
                info.editor_failed.clone().unwrap_or_default()
            }
            Some(info) => {
                let mut s = p.format_label().to_owned();
                if !p.vendor.is_empty() {
                    s = format!("{s} · {}", p.vendor);
                }
                if info.latency > 0 {
                    s = format!("{s} · latency {} samples", info.latency);
                }
                if pages > 1 {
                    s = format!("{s} · {}/{pages}", card.page + 1);
                }
                s
            }
            None => format!("{} · not running", p.format_label()),
        },
    };
    let status_rect = row.with_max_x(right).shrink2(vec2(0.2 * u, 0.0));
    let font = m.font_id(FontSize::Pt10);
    widgets::truncated(
        ui.painter(),
        status_rect,
        Align2::LEFT_CENTER,
        &status,
        font,
        text,
        5,
    );

    // The params of the page.
    let (mut changed, mut pressed) = (Vec::new(), None);
    let names: Vec<(String, f32)> = (plugin.iter())
        .flat_map(|p| p.params.iter().map(|p| (p.name.clone(), p.default)))
        .collect();
    let first = card.page * per_page;
    for (k, index) in (first..count.min(first + per_page)).enumerate() {
        let param = index + 1;
        let top = row.bottom() + k as f32 * SLIDER * u;
        let rect =
            Rect::from_min_size(pos2(content.left(), top), vec2(content.width(), SLIDER * u))
                .shrink2(vec2(0.3 * u, 0.0));
        let (name, default) = names.get(index).cloned().unwrap_or_default();
        let c = Control {
            value_text: true,
            disabled: live.is_none(),
            ..Control::new(id.with(("param", param)), rect, &name, ink, default)
        };
        let (channel, instance) = (channel_id, module.instance_id);
        let display = |v: f32| {
            plugins::param_text(channel, instance, param, v)
                .unwrap_or_else(|| format!("{:.0}%", v * 100.0))
        };
        let Some(value) = module.params.get_mut(param) else {
            continue;
        };
        let response = widgets::h_slider(ui, cx, &c, value, display);
        if response.changed() {
            changed.push(param as i32);
        }
        if response.is_pointer_button_down_on() || response.changed() {
            pressed = Some(param as i32);
        }
    }
    (changed, pressed)
}

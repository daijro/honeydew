//! Vocal-only workstation chrome. Reuses the native roll/audio commands; never renders audio or decodes bank samples.
use super::super::AppCtx;
use crate::theme::{Rgba, Theme};
use crate::ui::piano_roll::{GridSize, PianoRoll};
use banaforge_audio::device::AudioCommand;
use eframe::egui::{self, Color32, Rect, RichText, Sense, Ui, UiBuilder, vec2};

const BG: Color32 = Color32::from_rgb(17, 20, 24);
const PANEL: Color32 = Color32::from_rgb(30, 34, 40);
const ACCENT: Color32 = Color32::from_rgb(77, 196, 217);

#[derive(Default)]
pub(super) struct Workspace {
    drawer: bool,
    tab: usize,
    inspector_width: f32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Layout {
    pub canvas: Rect,
    header: Rect,
    tools: Rect,
    footer: Rect,
    inspector: Option<Rect>,
    browser: Option<Rect>,
    drawer: bool,
}

/// All bounds are logical points. Sidebars disappear before the editable grid gets squeezed.
pub(super) fn layout(
    area: Rect,
    inspector: bool,
    browser: bool,
    drawer: bool,
    width: f32,
) -> Layout {
    let header_h = 34.0_f32.min(area.height() * 0.17);
    let tools_h = 46.0_f32.min(area.height() * 0.22);
    let footer_h = if area.height() > 240.0 { 24.0 } else { 0.0 };
    let header = area.with_max_y(area.top() + header_h);
    let tools = area
        .with_min_y(header.bottom())
        .with_max_y(header.bottom() + tools_h);
    let footer = area.with_min_y(area.bottom() - footer_h);
    let body = area.with_min_y(tools.bottom()).with_max_y(footer.top());
    let wide = area.width() >= 760.0;
    let left =
        (wide && inspector).then(|| body.with_max_x(body.left() + width.clamp(180.0, 280.0)));
    let right = (area.width() >= 1180.0 && browser).then(|| body.with_min_x(body.right() - 200.0));
    let canvas = body
        .with_min_x(left.map_or(body.left(), |r| r.right() + 4.0))
        .with_max_x(right.map_or(body.right(), |r| r.left() - 4.0));
    Layout {
        canvas,
        header,
        tools,
        footer,
        inspector: left,
        browser: right,
        drawer: !wide && drawer,
    }
}

pub(super) fn theme(base: &Theme) -> Theme {
    let mut theme = base.clone();
    theme.colors.background = Rgba::hex(0x111418);
    theme.colors.control_fill = Rgba::hex(0x292E36);
    theme.colors.module_fill = Rgba::hex(0x20252B);
    theme.colors.menu_fill = Rgba::hex(0x303741);
    theme.colors.text = Rgba::hex(0xDEE5EC);
    theme.colors.accent = Rgba::hex(0x4DC4D9);
    theme.colors.text_on_accent = Rgba::hex(0x111418);
    theme.colors.white_key = Rgba::hex(0xC3CBD0);
    theme.colors.black_key = Rgba::hex(0x161A20);
    theme
}

fn style(ui: &mut Ui, large: bool) {
    let visuals = ui.visuals_mut();
    *visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(Color32::from_rgb(216, 225, 233));
    visuals.selection.bg_fill = Color32::from_rgb(31, 89, 106);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(43, 49, 58);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(57, 72, 85);
    visuals.widgets.active.bg_fill = Color32::from_rgb(30, 105, 124);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.corner_radius = egui::CornerRadius::same(2);
    }
    ui.spacing_mut().interact_size.y = if large { 40.0 } else { 30.0 };
    ui.spacing_mut().slider_width = 105.0;
    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(9.0);
    ui.label(RichText::new(title).size(10.0).color(ACCENT).strong());
    ui.separator();
}

/// Returns None while a compact inspector replaces the canvas, rather than covering touch targets.
pub(super) fn show(
    ui: &mut Ui,
    app: &mut AppCtx,
    channel: usize,
    roll: &mut PianoRoll,
    workspace: &mut Workspace,
    selected: &mut String,
) -> Option<Rect> {
    let area = app.regions.editor;
    let width = if workspace.inspector_width == 0.0 {
        210.0
    } else {
        workspace.inspector_width
    };
    let mut bounds = layout(
        area,
        app.settings.vocal_inspector_visible,
        app.settings.vocal_browser_visible,
        workspace.drawer,
        width,
    );
    ui.painter().rect_filled(area, 0.0, BG);
    ui.painter()
        .rect_filled(bounds.header, 0.0, Color32::from_rgb(36, 41, 48));
    ui.painter().rect_filled(bounds.tools, 0.0, PANEL);
    ui.scope_builder(
        UiBuilder::new().max_rect(bounds.header.shrink2(vec2(8.0, 0.0))),
        |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(bounds.header));
            style(ui, false);
            ui.horizontal_centered(|ui| {
                ui.label(RichText::new("B").color(ACCENT).strong());
                ui.label(
                    RichText::new("BANAFORGE / VOCAL STUDIO")
                        .size(12.0)
                        .strong(),
                );
                if area.width() > 560.0 {
                    ui.separator();
                    ui.label(RichText::new(&app.song.channels[channel].name).size(11.0));
                    ui.label(
                        RichText::new(format!("{:.0} BPM", app.song.tempo_bpm))
                            .size(11.0)
                            .color(ACCENT),
                    );
                    ui.label(RichText::new("• PROCEDURAL VOICE").color(ACCENT).size(10.0));
                }
            });
        },
    );
    ui.scope_builder(
        UiBuilder::new().max_rect(bounds.tools.shrink2(vec2(6.0, 2.0))),
        |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(bounds.tools));
            style(ui, app.settings.vocal_large_controls);
            egui::ScrollArea::horizontal()
                .id_salt(("vocal-tools", channel))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(
                                bounds.inspector.is_some() || workspace.drawer,
                                "Inspector",
                            )
                            .clicked()
                        {
                            if area.width() >= 760.0 {
                                app.settings.vocal_inspector_visible =
                                    !app.settings.vocal_inspector_visible;
                            } else {
                                workspace.drawer = !workspace.drawer;
                            }
                        }
                        ui.separator();
                        if ui
                            .button(if app.playing { "Ⅱ Stop" } else { "▶ Play" })
                            .clicked()
                        {
                            app.playing = !app.playing;
                        }
                        if !app.settings.vocal_simple_mode {
                            ui.selectable_value(&mut roll.pitch_draw_enabled, false, "Notes");
                            ui.selectable_value(&mut roll.pitch_draw_enabled, true, "Pitch");
                        }
                        ui.separator();
                        ui.toggle_value(&mut roll.snap.on, "Snap");
                        egui::ComboBox::from_id_salt(("studio-grid", channel))
                            .width(70.0)
                            .selected_text(app.settings.grid_size.label())
                            .show_ui(ui, |ui| {
                                for size in GridSize::ALL {
                                    ui.selectable_value(
                                        &mut app.settings.grid_size,
                                        size,
                                        size.label(),
                                    );
                                }
                            });
                        ui.toggle_value(&mut app.settings.vocal_lyric_bar_enabled, "Lyrics");
                        ui.toggle_value(&mut app.settings.vocal_waveform_enabled, "Wave");
                        if ui
                            .selectable_label(app.settings.vocal_browser_visible, "Banks")
                            .clicked()
                        {
                            app.settings.vocal_browser_visible =
                                !app.settings.vocal_browser_visible;
                            if area.width() < 1180.0 {
                                workspace.drawer = true;
                                workspace.tab = 1;
                            }
                        }
                        if ui.button("View / Settings").clicked() {
                            workspace.drawer = true;
                            workspace.tab = 2;
                        }
                    });
                });
        },
    );
    // Update after toolbar toggles, in the same frame.
    bounds = layout(
        area,
        app.settings.vocal_inspector_visible,
        app.settings.vocal_browser_visible,
        workspace.drawer,
        width,
    );
    if let Some(left) = bounds.inspector {
        let divider = left.with_min_x(left.right()).with_max_x(left.right() + 4.0);
        let response = ui.interact(divider, ui.id().with("vocal-divider"), Sense::drag());
        ui.painter().rect_filled(
            divider,
            0.0,
            if response.hovered() {
                ACCENT
            } else {
                Color32::from_gray(56)
            },
        );
        if response.dragged() {
            workspace.inspector_width = (width + response.drag_delta().x).clamp(180.0, 280.0);
        }
        panel(ui, left, app, channel, roll, workspace, selected);
    }
    if let Some(right) = bounds.browser {
        ui.painter().rect_filled(right, 0.0, PANEL);
        ui.scope_builder(UiBuilder::new().max_rect(right.shrink(10.0)), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(right));
            style(ui, app.settings.vocal_large_controls);
            banks(ui, app, selected);
        });
    }
    // On tablets Settings uses the left inspector; on phones it replaces only
    // the canvas, never the outer app's transport/navigation.
    if workspace.drawer && (bounds.drawer || bounds.inspector.is_none()) {
        panel(ui, bounds.canvas, app, channel, roll, workspace, selected);
        return None;
    }
    if workspace.drawer && bounds.inspector.is_some() {
        workspace.drawer = false;
    }
    if bounds.footer.height() > 0.0 {
        ui.painter().rect_filled(bounds.footer, 0.0, PANEL);
        ui.painter_at(bounds.footer).text(
            bounds.footer.left_center() + vec2(10.0, 0.0),
            egui::Align2::LEFT_CENTER,
            if roll.pitch_draw_enabled {
                "PITCH DRAW   •   Drag a cyan node / draw a curve   |   Lyrics in the top lane"
            } else {
                "NOTE EDIT   •   Tap to add / edit lyrics   ·   Drag to move   ·   Edge to resize"
            },
            egui::FontId::proportional(10.0),
            Color32::from_gray(165),
        );
    }
    Some(bounds.canvas)
}

fn panel(
    ui: &mut Ui,
    rect: Rect,
    app: &mut AppCtx,
    channel: usize,
    roll: &mut PianoRoll,
    workspace: &mut Workspace,
    selected: &mut String,
) {
    ui.painter().rect_filled(rect, 0.0, PANEL);
    ui.scope_builder(UiBuilder::new().max_rect(rect.shrink(10.0)), |ui| {
        style(ui, app.settings.vocal_large_controls);
        ui.set_clip_rect(ui.clip_rect().intersect(rect));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut workspace.tab, 0, "Track");
            ui.selectable_value(&mut workspace.tab, 1, "Voice");
            ui.selectable_value(&mut workspace.tab, 2, "View");
            if workspace.drawer && ui.small_button("× Close").clicked() { workspace.drawer = false; }
        });
        egui::ScrollArea::vertical().id_salt(("studio-inspector", channel)).show(ui, |ui| {
            match workspace.tab {
                1 => banks(ui, app, selected),
                2 => {
                    section(ui, "WORKSPACE");
                    ui.checkbox(&mut app.settings.vocal_simple_mode, "Simple editing");
                    ui.checkbox(&mut app.settings.vocal_large_controls, "Large touch controls");
                    ui.checkbox(&mut app.settings.vocal_follow_playhead, "Follow playhead");
                    ui.checkbox(&mut app.settings.vocal_lyric_bar_enabled, "Top lyric lane");
                    ui.checkbox(&mut app.settings.vocal_waveform_enabled, "Waveform below notes");
                    let response = ui.add(egui::Slider::new(&mut app.settings.vocal_waveform_offset_y, 0..=40).text("Gap (rows)"));
                    if response.double_clicked() {
                        app.settings.vocal_waveform_offset_y = 5;
                    }
                    section(ui, "WORKFLOW");
                    ui.label("Pinch to zoom. Two-finger drag to scroll. Long-press notes for Copy / Delete.");
                    ui.label("Waveform previews yield to playback. No bank audio is decoded by this panel.");
                }
                _ => {
                    section(ui, "TRACK / OUTPUT");
                    ui.label(RichText::new(&app.song.channels[channel].name).strong());
                    for (index, label) in [(0, "Level"), (1, "Pan")] { parameter(ui, app, channel, 0, index, label); }
                    section(ui, "VOICE / PROCEDURAL");
                    ui.label("Original formant voice");
                    ui.label(RichText::new("Bank samples are not active").size(11.0).color(Color32::from_rgb(226, 180, 106)));
                    if let Some(id) = app.song.channels[channel].rack.modules.iter().find(|m| m.type_id == 53).map(|m| m.instance_id) {
                        if !app.settings.vocal_simple_mode { vowel(ui, app, channel, id); }
                        for (index, label) in [(1,"Formant"), (2,"Breath"), (3,"Brightness"), (4,"Portamento"), (5,"Vibrato rate"), (6,"Vibrato depth"), (8,"Attack"), (9,"Release")] {
                            if !app.settings.vocal_simple_mode || matches!(index, 3 | 8 | 9) { parameter(ui, app, channel, id, index, label); }
                        }
                    }
                    section(ui, "NOTE EXPRESSION");
                    if !app.settings.vocal_simple_mode {
                        ui.checkbox(&mut roll.graph.open, "Show expression lane");
                        if ui.button("Flatten selected pitch").clicked() && roll.reset_vocal_pitch(app.song) { app.song_edited(true); }
                    } else { ui.weak("Advanced controls hidden in Simple mode."); }
                    section(ui, "ENGINE STATUS");
                    ui.label("Realtime procedural synthesis");
                    ui.add_enabled(false, egui::Button::new("Bank renderer · not available"));
                }
            }
        });
    });
}

fn banks(ui: &mut Ui, app: &mut AppCtx, selected: &mut String) {
    section(ui, "VOICEBANK LIBRARY");
    ui.label("Indexed metadata only");
    ui.label(
        RichText::new("Choosing a bank here does not activate sample singing.")
            .size(11.0)
            .color(Color32::from_rgb(226, 180, 106)),
    );
    let importing = app.files.importing_voicebank();
    if ui
        .add_enabled(
            !importing,
            egui::Button::new(if importing {
                "Importing…"
            } else {
                "+ Import voicebank"
            }),
        )
        .clicked()
    {
        app.files.pick_voicebank();
    }
    if importing {
        ui.spinner();
    }
    ui.separator();
    if ui
        .selectable_label(selected.is_empty(), "Original procedural voice")
        .clicked()
    {
        selected.clear();
    }
    egui::ScrollArea::vertical()
        .id_salt("studio-banks")
        .max_height(500.0)
        .show(ui, |ui| {
            let banks = app.files.voicebanks();
            if banks.is_empty() {
                ui.weak("No imported banks. The original voice works offline.");
            }
            for bank in banks {
                ui.selectable_value(selected, bank.name.clone(), &bank.name);
            }
        });
}

fn vowel(ui: &mut Ui, app: &mut AppCtx, channel: usize, instance_id: i32) {
    let target = crate::song::AutomationTarget {
        instance_id,
        param: 7,
    };
    let Some(value) = app.song.channels[channel].param_mut(target) else {
        return;
    };
    let before = *value;
    let mut index = (before * 4.0).round().clamp(0.0, 4.0) as usize;
    let mut changed = false;
    egui::ComboBox::from_id_salt(("studio-vowel", channel))
        .selected_text(["A", "E", "I", "O", "U"][index])
        .show_ui(ui, |ui| {
            for (i, label) in ["A", "E", "I", "O", "U"].iter().enumerate() {
                changed |= ui.selectable_value(&mut index, i, *label).changed();
            }
        });
    let next = index as f32 / 4.0;
    if changed && (next - before).abs() > 0.001 {
        *value = next;
        app.audio.send(AudioCommand::SetParam {
            channel,
            instance_id,
            index: 7,
            value: next,
        });
        app.channel_param_edited();
    }
}

fn parameter(
    ui: &mut Ui,
    app: &mut AppCtx,
    channel: usize,
    instance_id: i32,
    index: i32,
    label: &str,
) {
    let target = crate::song::AutomationTarget {
        instance_id,
        param: index,
    };
    let reset = if instance_id == 0 {
        crate::params::CHANNEL_PARAMS.get(index as usize).map(|p| p.reset)
    } else {
        app.song.channels[channel].rack.get(instance_id)
            .and_then(|module| module.param_spec(index as usize)).map(|p| p.reset)
    };
    let Some(value) = app.song.channels[channel].param_mut(target) else {
        return;
    };
    let response = ui.add(
        egui::Slider::new(value, 0.0..=1.0)
            .text(label)
            .show_value(false),
    );
    let double_reset = response.double_clicked() && reset.is_some();
    if double_reset {
        *value = reset.unwrap_or(*value);
    }
    let changed = response.changed() || double_reset;
    let value = *value;
    if changed {
        app.audio.send(AudioCommand::SetParam {
            channel,
            instance_id,
            index,
            value,
        });
        app.channel_param_edited();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;
    #[test]
    fn vocal_workspace_responsive_layout_keeps_a_usable_grid() {
        for (w, h) in [
            (320.0, 240.0),
            (390.0, 600.0),
            (740.0, 320.0),
            (900.0, 500.0),
            (1440.0, 900.0),
        ] {
            let area = Rect::from_min_size(pos2(12.0, 24.0), vec2(w, h));
            let l = layout(area, true, true, false, 210.0);
            assert!(area.contains_rect(l.canvas));
            assert!(l.canvas.width() >= 300.0);
            assert!(l.canvas.height() >= h * 0.5);
            assert!(!l.inspector.is_some_and(|r| r.intersects(l.canvas)));
            assert!(!l.browser.is_some_and(|r| r.intersects(l.canvas)));
            assert!(!l.drawer);
        }
        assert!(
            layout(
                Rect::from_min_size(pos2(0.0, 0.0), vec2(390.0, 600.0)),
                true,
                true,
                true,
                210.0
            )
            .drawer
        );
    }
    #[test]
    fn vocal_workspace_theme_is_local_and_dark() {
        let base = crate::theme::theme_by_id(0);
        let saved = base.clone();
        let studio = theme(&base);
        assert_eq!(base, saved);
        assert_eq!(studio.colors.background, Rgba::hex(0x111418));
        assert!(studio.colors.is_dark());
    }
}

//! Native horizontal touch cards for the original Banana processors.
use crate::panels::build::*;
use crate::ui::UiCtx;
use banaforge_audio::banana_power as b;
use eframe::egui::{Color32, Id, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};

pub(in crate::panels) fn panel(id: u16) -> Panel {
    match id {
        b::FRESH_AIR => Panel::new(hstack([
            knob(1, "Mid Air").p(1).value_text(),
            knob(2, "High Air").p(2).value_text(),
            knob(3, "Trim").p(3).w(Size::Flex(0.7)).value_text(),
        ])),
        b::SOUNDGOODIZER => Panel::new(vstack([
            knob(1, "Amount").p(1).value_text(),
            list(2, "A · B · C · D", Bar::Bottom).p(2).h(Size::U(1.25)),
        ])),
        b::BLOOD_OVERDRIVE => Panel::new(hstack([
            knob(1, "Drive").p(1),
            knob(2, "Color").p(2),
            knob(3, "Post Filter").p(3),
            knob(4, "Trim").p(4),
            knob(5, "Mix").p(5),
        ])),
        _ => Panel::new(vstack([
            tabs(1, "TIME · VOLUME").h(Size::U(1.25)),
            hstack([
                display(2).draw(envelope).w(Size::Flex(4.0)),
                vstack([
                    spinner(3, "Time mode").p(1).h(Size::U(1.5)).show(Show::Tab(0)),
                    spinner(4, "Volume mode").p(2).h(Size::U(1.5)).show(Show::Tab(1)),
                    spinner(5, "Length").p(3).h(Size::U(1.5)).mt(0.2),
                    knob(6, "Mix").p(4).h(Size::U(2.0)).mt(0.2),
                ])
                .w(Size::Flex(1.7)),
            ]),
        ])),
    }
}

// The fixed x lattice avoids crossed/duplicate times. Drag a column to set its
// y value, snapped to sixteenths. Whole-grid touch sensing gives large hit areas
// even when individual nodes are closer than a thumb's width.
fn envelope(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let plot = rect.shrink(8.0 / cx.m.ppp());
    let painter = ui.painter_at(rect);
    painter.rect_filled(plot, 2.0, cx.theme.colors.background.to_color32());
    let line = cx.theme.colors.text.to_color32().gamma_multiply(0.16);
    for i in 0..=8 {
        let t = i as f32 / 8.0;
        painter.vline(
            plot.left() + t * plot.width(),
            plot.y_range(),
            Stroke::new(1.0 / cx.m.ppp(), line),
        );
        painter.hline(
            plot.x_range(),
            plot.top() + t * plot.height(),
            Stroke::new(1.0 / cx.m.ppp(), line),
        );
    }
    let volume = state.tab == 1;
    let base = if volume {
        b::VOLUME_POINTS
    } else {
        b::TIME_POINTS
    };
    let preset = if volume { 2 } else { 1 };
    let custom = 1.0;
    let response = ui.interact(plot, id.with("curve"), Sense::click_and_drag())
        .on_hover_text("Drag a column to edit a saved custom envelope. Values snap to 1/16. Time is causal delay, volume is level.");
    let mut edits = Vec::new();
    if (response.clicked() || response.dragged())
        && let Some(p) = response.interact_pointer_pos()
    {
        let column = (((p.x - plot.left()) / plot.width()).clamp(0.0, 1.0) * 15.0).round() as usize;
        let value = (((plot.bottom() - p.y) / plot.height()).clamp(0.0, 1.0) * 16.0).round() / 16.0;
        if module.params[base + column] != value {
            module.params[base + column] = value;
            module.params[preset] = custom;
            edits.extend([base + column, preset]);
        }
    }
    // Display the active preset, not a misleading custom curve while a preset plays.
    let mode = if volume {
        (module.params[2] * 3.0).round() as usize
    } else {
        (module.params[1] * 4.0).round() as usize
    };
    let points: Vec<_> = (0..b::POINTS)
        .map(|i| {
            let x = i as f32 / 15.0;
            let y = if volume {
                match mode {
                    0 => 1.0,
                    1 => {
                        if (x * 8.0).fract() < 0.5 {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    2 => x,
                    _ => module.params[base + i],
                }
            } else {
                match mode {
                    0 => 0.0,
                    1 => x * 0.5,
                    2 => 0.5 * x * x,
                    3 => (x * 8.0).floor() / 8.0,
                    _ => module.params[base + i],
                }
            };
            pos2(
                plot.left() + x * plot.width(),
                plot.bottom() - y * plot.height(),
            )
        })
        .collect();
    painter.add(Shape::line(
        points.clone(),
        Stroke::new(2.0 / cx.m.ppp(), ink),
    ));
    for point in points {
        painter.circle_filled(point, 4.0 / cx.m.ppp(), ink);
    }
    painter.text(
        plot.left_top() + vec2(5.0, 4.0),
        Align2::LEFT_TOP,
        if volume {
            "VOLUME · 1/16 SNAP"
        } else {
            "TIME DELAY · 1/16 SNAP"
        },
        cx.m.font_id(FontSize::Pt8),
        cx.theme.colors.text.to_color32(),
    );
    edits
}

#[cfg(test)]
mod layout_regressions {
    use super::*;
    #[test]
    fn layout_regression_gross_beat_modes_are_single_selectors_in_disjoint_rows() {
        let module = ModuleInstance::new(b::GROSS_BEAT, 1);
        for tab in [0, 1] {
            for width in [12.0, 20.0] {
                let bounds = Rect::from_min_size(pos2(0.0,0.0),vec2(width*32.0,6.5*32.0));
                let state = PanelState { tab, ..Default::default() };
                let panel = panel(b::GROSS_BEAT);
                let leaves = crate::panels::place(&panel,&module,&state,bounds,32.0);
                assert!(leaves.iter().any(|(l,_)| l.id == if tab == 0 { 3 } else { 4 }));
                assert!(!leaves.iter().any(|(l,_)| l.id == if tab == 0 { 4 } else { 3 }));
                for (i,(_,a)) in leaves.iter().enumerate() {
                    assert!(bounds.contains_rect(*a));
                    for (_,b) in &leaves[i+1..] { assert!(!a.intersect(*b).is_positive()); }
                }
            }
        }
    }
}

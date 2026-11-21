//! MIDI-note decoration only. No song, hit-testing, transport or audio dependencies.
use eframe::egui::{self, Color32, Painter, Rect, Stroke, StrokeKind, pos2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteStyle {
    #[default]
    Classic,
    Smooth,
    Outline,
    Gradient,
    Accent,
}

impl NoteStyle {
    pub const ALL: [Self; 5] = [
        Self::Classic,
        Self::Smooth,
        Self::Outline,
        Self::Gradient,
        Self::Accent,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Smooth => "Smooth",
            Self::Outline => "Outline",
            Self::Gradient => "Gradient",
            Self::Accent => "Accent",
        }
    }
    /// Constant bounded geometry per visible note; no textures, animation or per-pixel gradients.
    pub(super) fn paint(self, painter: &Painter, rect: Rect, color: Color32, px: f32) {
        if !rect.is_positive() {
            return;
        }
        let edge = px.min(rect.width() * 0.5).min(rect.height() * 0.5);
        match self {
            Self::Classic => {
                painter.rect_filled(rect, 0.0, color);
            }
            Self::Smooth => {
                painter.rect_filled(rect, (rect.height() * 0.25).min(rect.width() * 0.5), color);
            }
            Self::Outline => {
                painter.rect_stroke(rect, 0.0, Stroke::new(edge, color), StrokeKind::Inside);
            }
            Self::Gradient => {
                let mut mesh = egui::Mesh::default();
                mesh.colored_vertex(rect.left_top(), color);
                mesh.colored_vertex(rect.right_top(), color);
                mesh.colored_vertex(rect.left_bottom(), color.gamma_multiply(0.72));
                mesh.colored_vertex(rect.right_bottom(), color.gamma_multiply(0.72));
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(2, 1, 3);
                painter.add(egui::Shape::mesh(mesh));
            }
            Self::Accent => {
                painter.rect_filled(rect, 0.0, color);
                let luminance = u32::from(color.r()) + u32::from(color.g()) + u32::from(color.b());
                let accent = if luminance > 420 {
                    Color32::BLACK
                } else {
                    Color32::WHITE
                };
                painter.rect_filled(
                    Rect::from_min_max(rect.min, pos2(rect.left() + edge, rect.bottom())),
                    0.0,
                    accent.gamma_multiply(0.65 * f32::from(color.a()) / 255.0),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn note_appearance_all_styles_are_bounded_and_outline_is_unfilled() {
        let ctx = egui::Context::default();
        for style in NoteStyle::ALL {
            for size in [
                egui::vec2(1.0, 1.0),
                egui::vec2(4.0, 8.0),
                egui::vec2(160.0, 24.0),
            ] {
                let rect = Rect::from_min_size(pos2(20.0, 20.0), size);
                let output = ctx.run(Default::default(), |ctx| {
                    let p = ctx.layer_painter(egui::LayerId::background());
                    style.paint(&p, rect, Color32::RED, 1.0);
                });
                assert!(output.shapes.len() <= 2);
                for s in output.shapes {
                    match s.shape {
                        egui::Shape::Rect(r) => {
                            assert!(rect.contains_rect(r.rect));
                            if style == NoteStyle::Outline {
                                assert_eq!(r.fill, Color32::TRANSPARENT);
                            }
                        }
                        egui::Shape::Mesh(mesh) => {
                            assert_eq!(mesh.vertices.len(), 4);
                            assert_eq!(mesh.indices.len(), 6);
                            assert!(mesh.vertices.iter().all(|v| rect.contains(v.pos)));
                        }
                        _ => panic!("unexpected decoration"),
                    }
                }
            }
        }
    }
}

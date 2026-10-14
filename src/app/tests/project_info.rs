//! The PROJECT tab's `Samplerate:` row: the output rate with a thousands separator (`48,000 Hz`).

use eframe::egui::Shape;

use super::*;

impl Rig {
    /// The text the last frame drew on `label`'s line, the nearest to its right.
    fn value_of(&self, label: &str) -> Option<String> {
        let texts: Vec<_> = (self.out.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Text(t) => Some((t.galley.text().to_owned(), t.visual_bounding_rect())),
                _ => None,
            })
            .collect();
        let at = texts.iter().find(|(t, _)| t == label)?.1;
        (texts.into_iter())
            .filter(|(_, r)| (r.center().y - at.center().y).abs() < 1.0 && r.left() >= at.right())
            .min_by(|a, b| a.1.left().total_cmp(&b.1.left()))
            .map(|(t, _)| t)
    }
}

#[test]
fn samplerate_is_the_running_outputs_and_a_dash_without_audio() {
    // No output yet: `-`.
    let mut rig = Rig::new();
    rig.open_menu(MenuTab::Project);
    assert_eq!(rig.value_of("Samplerate:").as_deref(), Some("-"));
    // The recorder's output runs at 48 kHz.
    let mut rig = Rig::with_synth();
    rig.open_menu(MenuTab::Project);
    assert_eq!(rig.value_of("Samplerate:").as_deref(), Some("48,000 Hz"));
}

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

/// PR #87: an output that runs at another rate than the project's (the recorder stays at 48 kHz) says so once for
/// each pair of rates, and again only after they matched in between; a song that follows the output (0) never does.
#[test]
fn an_output_that_falls_back_from_the_project_rate_says_so_once() {
    let fallback = |rig: &Rig| {
        (rig.app.nav.overlays.iter()).any(|o| {
            matches!(
                o,
                Overlay::Message {
                    tag: "sample_rate_fallback",
                    ..
                }
            )
        })
    };
    let mut rig = Rig::with_synth();
    rig.frame(vec![]);
    assert!(!fallback(&rig), "the output's own rate");
    rig.app.song.sample_rate = 96_000;
    rig.frame(vec![]);
    assert!(fallback(&rig));
    rig.app.nav.overlays.clear();
    rig.frame(vec![]);
    assert!(!fallback(&rig), "once");
    rig.rec.0.borrow_mut().sample_rate = Some(96_000);
    rig.frame(vec![]);
    rig.rec.0.borrow_mut().sample_rate = None;
    rig.frame(vec![]);
    assert!(fallback(&rig), "again after the rates matched");
}

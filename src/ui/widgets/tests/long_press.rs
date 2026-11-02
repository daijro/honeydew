use super::*;

// The parameter menu opens on a long press of the mouse too (300 ms within 0.5 U).

impl Rig {
    /// Presses the control's centre with the mouse and holds it still, a frame every 1/60 s, until the frame `secs`
    /// after the press, which releases when `release`.
    fn hold(&mut self, secs: f64, release: bool) {
        let at = RECT.center();
        self.frame(vec![Event::PointerMoved(at)]);
        self.button(at, PointerButton::Primary, true);
        let start = self.time;
        while self.time + 1.0 / 60.0 < start + secs - 1e-9 {
            self.frame(vec![]);
        }
        self.time = start + secs - 1.0 / 60.0;
        if release {
            self.button(at, PointerButton::Primary, false);
        } else {
            self.frame(vec![]);
        }
    }
}

/// Knobs, sliders and toggles (bound: the `ParamMenu::Bound` items, after `Reset` on knobs and sliders), and a bound
/// button.
fn rigs() -> Vec<Rig> {
    let mut rigs: Vec<Rig> = [Kind::Knob, Kind::VSlider, Kind::HSlider, Kind::Toggle]
        .map(|kind| Rig::new(kind, 0.25))
        .into();
    let mut button = Rig::new(Kind::Button, 0.0);
    button.style = |b| Button {
        menu: ParamMenu::Bound,
        ..b
    };
    rigs.push(button);
    rigs
}

#[test]
fn a_mouse_held_still_300_ms_opens_the_parameter_menu_and_its_release_neither_clicks_nor_edits() {
    for mut rig in rigs() {
        let kind = rig.kind;
        rig.hold(0.3, false);
        assert!(rig.popup_is_open(), "{kind:?}: open at 300 ms");
        // Held on past egui's 0.8 s and moved: the press drags nothing under the menu.
        for _ in 0..40 {
            rig.frame(vec![]);
        }
        rig.frame(vec![Event::PointerMoved(RECT.center() + vec2(40.0, -40.0))]);
        rig.button(RECT.center(), PointerButton::Primary, false);
        assert!(!rig.clicked, "{kind:?}");
        assert_eq!(
            rig.value,
            if matches!(kind, Kind::Button) {
                0.0
            } else {
                0.25
            },
            "{kind:?}"
        );
        rig.frame(vec![]);
        assert!(rig.menu_rect().is_some(), "{kind:?}: still open");
        let texts: Vec<String> = texts(&rig.shapes).into_iter().map(|t| t.0).collect();
        assert!(
            texts.iter().any(|t| t == "Add automation track"),
            "{kind:?}"
        );
    }
}

#[test]
fn a_mouse_released_at_250_ms_is_a_click_and_opens_no_menu() {
    for mut rig in rigs() {
        let kind = rig.kind;
        rig.hold(0.25, true);
        rig.frame(vec![]);
        assert!(!rig.popup_is_open(), "{kind:?}");
        match kind {
            // A click flips a toggle (0.25 is on: off) and is reported by a button.
            Kind::Toggle => assert_eq!(rig.value, 0.0),
            Kind::Button => {
                rig.hold(0.25, true);
                assert!(rig.clicked);
            }
            _ => assert_eq!(rig.value, 0.25, "{kind:?}"),
        }
    }
}

#[test]
fn a_long_press_on_a_toggle_does_not_flip_it() {
    let mut rig = Rig::new(Kind::Toggle, 0.0);
    rig.hold(0.4, true);
    assert_eq!(rig.value, 0.0);
    assert!(rig.popup_is_open());
}

//! The performance monitor (PROJECT's `Performance` block, and the transport bar's CPU/RAM tap): the data the app
//! feeds it, and the readout's tap.

use eframe::egui::Shape;

use super::*;
use crate::ui::performance::{IDLE_TEXT, footer_line, status_line};

impl Rig {
    /// The painted text `text`: its position among the shapes and its visual rect.
    fn text_shape(&self, text: &str) -> Option<(usize, egui::Rect)> {
        (self.out.shapes.iter().enumerate()).find_map(|(i, s)| match &s.shape {
            Shape::Text(t) if t.galley.text() == text => Some((i, t.visual_bounding_rect())),
            _ => None,
        })
    }

    /// Frames for the keyboard panel to slide to its place, where the transport bar's readout is; the centre of the
    /// CPU/RAM slot there (one b = 1.75 U slot left of ↘'s, at the bar's right edge).
    fn settle(&mut self) -> Pos2 {
        for _ in 0..40 {
            self.frame(vec![]);
        }
        let id = egui::Id::new(("transport", "meters"));
        let at = self
            .ctx
            .read_response(id)
            .expect("the readout")
            .rect
            .center();
        assert!((at.x - (1170.0 - 1.5 * 1.75 * U)).abs() < 0.01, "{at:?}");
        at
    }

    /// The footer the block shows: the app's memory from the `RAM` readout, as the app puts it.
    fn footer(&self) -> String {
        footer_line(&self.app.slots.performance.app_memory)
    }

    /// The status line with no load and no underruns.
    fn status(&self) -> String {
        status_line(0.0, 0)
    }

    /// The PROJECT tab scrolled to its end, where the Performance block is.
    fn scroll_project_down(&mut self) {
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -5000.0),
            modifiers: Default::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(content().center()), wheel]);
        for _ in 0..60 {
            self.frame(vec![]);
        }
    }

    /// PROJECT, scrolled to its Performance block.
    fn performance_page(&mut self) {
        self.open_menu(MenuTab::Project);
        self.scroll_project_down();
    }
}

/// The main menu's content area at U = 32.
fn content() -> egui::Rect {
    crate::ui::main_menu::shell_layout(
        egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0)),
        &Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE),
    )
    .content
}

#[test]
fn a_tap_on_cpu_or_ram_opens_project_with_the_performance_block_in_view() {
    // A release inside the CPU/RAM slot opens the main menu on PROJECT, then scrolls: the block is below the fold at
    // first, so it is scrolled to the end, where the clamp leaves it fully in view.
    {
        let mut rig = Rig::with_synth();
        let label = rig.settle();
        rig.click(label);
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        assert_eq!(
            rig.overlays(),
            [Overlay::MainMenu(MenuTab::Project)],
            "{label:?}"
        );
        // In view up and down: the plain footer, with the machine's memory, may run past the right edge.
        let (_, footer) = rig.text_shape(&rig.footer()).expect("the block's footer");
        assert!(
            content().y_range().contains(footer.top())
                && content().y_range().contains(footer.bottom()),
            "{footer:?} in {:?}",
            content()
        );
        let status = rig.text_shape(&rig.status()).expect("status line").1;
        assert!(content().contains_rect(status));
    }
}

#[test]
fn the_app_feeds_the_data_every_frame_and_the_idle_text_leaves_at_the_first_playback_for_good() {
    let mut rig = Rig::with_synth();
    // A clip 8 beats long makes the song's graph 8 beats; the transport at beat 2 is a quarter of the way.
    rig.app.song.channels[1].lanes[0].add_clip(0, 8.0);
    rig.click(OUTSIDE);
    rig.performance_page();
    assert!(rig.painted(IDLE_TEXT), "idle before any playback");
    assert!(!rig.app.slots.performance.played);
    rig.rec.0.borrow_mut().tick = 2 * 128;
    rig.app.main_screen.playing = true;
    rig.frame(vec![]);
    let p = &rig.app.slots.performance;
    assert!(p.playing && p.played, "the playing flag is fed");
    assert!(
        (p.position - 0.25).abs() < 1e-6,
        "the tick and the song's length are fed: {}",
        p.position
    );
    assert!(!rig.painted(IDLE_TEXT), "the idle text is gone");
    // Stopped, and a song loaded: it never comes back.
    rig.app.main_screen.playing = false;
    rig.frame(vec![]);
    assert!(!rig.painted(IDLE_TEXT));
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.app.slots.performance.played);
    assert!(
        !rig.painted(IDLE_TEXT),
        "a song load does not bring it back"
    );
}

#[test]
fn the_data_waits_while_a_loading_screen_is_up() {
    // Not while the loading screen is up: no update, so no first playback is seen.
    let mut rig = Rig::with_synth();
    rig.app.navigate(Nav::Open(Overlay::Busy {
        title: "Rendering".to_owned(),
    }));
    rig.app.main_screen.playing = true;
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(
        !rig.app.slots.performance.played,
        "the loading screen holds the data"
    );
    rig.app.navigate(Nav::Close);
    rig.frame(vec![]);
    assert!(rig.app.slots.performance.played, "and it moves on after it");
}

#[test]
fn the_footers_memory_is_the_ram_readouts_reads() {
    let mut rig = Rig::with_synth();
    rig.settle();
    rig.performance_page();
    rig.frame(vec![]);
    let readout = rig.app.main_screen.ram_text().to_owned();
    assert!(!readout.is_empty());
    let shown = if readout.starts_with("--") {
        "-"
    } else {
        readout.as_str()
    };
    assert_eq!(rig.app.slots.performance.app_memory, shown);
    assert!(rig.painted(&footer_line(shown)), "{shown}");
}

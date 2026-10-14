//! The performance monitor (PROJECT's `Performance` block, the floating monitor, and the transport bar's CPU/RAM taps
//! and tooltips): `Detach` / `Attach`, the floating view's place and layering, and the `CPU` / `RAM` labels' presses.

use eframe::egui::{Color32, Shape};

use super::*;
use crate::ui::performance::{IDLE_TEXT, footer_line, monitor_rect, status_line};

/// Somewhere over neither label, nor the monitor.
const ELSEWHERE: Pos2 = pos2(300.0, 200.0);

impl Rig {
    /// The painted text `text`: its position among the shapes and its visual rect.
    fn text_shape(&self, text: &str) -> Option<(usize, egui::Rect)> {
        (self.out.shapes.iter().enumerate()).find_map(|(i, s)| match &s.shape {
            Shape::Text(t) if t.galley.text() == text => Some((i, t.visual_bounding_rect())),
            _ => None,
        })
    }

    fn text_centre(&self, text: &str) -> Option<Pos2> {
        self.text_shape(text).map(|(_, r)| r.center())
    }

    /// Frames for the keyboard panel to slide to its place, where the transport bar's labels are; the centres of the
    /// `CPU` and `RAM` labels' slots there (b = 1.75 U slots right to left from the close X's, RAM then CPU).
    fn settle(&mut self) -> (Pos2, Pos2) {
        for _ in 0..40 {
            self.frame(vec![]);
        }
        let centre = |which: u8| {
            let id = egui::Id::new(("transport", "meter label", which));
            self.ctx.read_response(id).expect("the label").rect.center()
        };
        let (cpu, ram) = (centre(0), centre(1));
        let b = 1.75 * U;
        assert!(
            (cpu.x - (1170.0 - 2.5 * b)).abs() < 0.01 && (ram.x - (1170.0 - 1.5 * b)).abs() < 0.01,
            "{cpu:?} {ram:?}"
        );
        (cpu, ram)
    }

    /// The centre of the transport bar's close X.
    fn close_x(&self) -> Pos2 {
        let id = egui::Id::new(("transport", "close"));
        self.ctx
            .read_response(id)
            .expect("the close X")
            .rect
            .center()
    }

    /// The footer the monitor shows: the app's memory from the `RAM` readout, as the app puts it.
    fn footer(&self) -> String {
        let p = &self.app.slots.performance;
        footer_line(&p.app_memory, p.system_memory)
    }

    /// The status line the song's counts give, with no load.
    fn status(&self) -> String {
        let song = &self.app.song;
        let modules = song.channels.iter().map(|c| c.rack.modules.len()).sum();
        status_line(song.channels.len() - 1, modules, 0.0)
    }

    /// The PROJECT tab scrolled to its end, where `Detach` is.
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

    /// Detaches the monitor from PROJECT's `Detach`.
    fn detach(&mut self) {
        self.open_menu(MenuTab::Project);
        self.scroll_project_down();
        self.click_id(project_id("detach"));
        self.frame(vec![]);
    }

    /// A press at `at`, held for `frames` frames (0.1 s each), and released.
    fn hold(&mut self, at: Pos2, frames: usize) {
        self.press(at);
        for _ in 0..frames {
            self.frame(vec![]);
        }
        self.release(at);
    }

    /// The filled rects of the last frame with their fill.
    fn fills(&self) -> Vec<(egui::Rect, Color32)> {
        (self.out.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) => Some((r.rect, r.fill)),
                _ => None,
            })
            .collect()
    }
}

/// The main menu's content area at U = 32.
fn content() -> egui::Rect {
    crate::ui::main_menu::shell_layout(
        egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0)),
        &Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX),
    )
    .content
}

#[test]
fn detach_shows_the_floating_monitor_and_closes_the_menu_and_attach_hides_it_and_leaves_the_menu() {
    let mut rig = Rig::with_synth();
    assert!(
        !rig.app.slots.performance.detached,
        "every launch starts attached"
    );
    rig.open_menu(MenuTab::Project);
    rig.scroll_project_down();
    rig.click_id(project_id("detach"));
    rig.frame(vec![]);
    // Detach: the monitor is up and the menu is gone.
    assert!(rig.app.slots.performance.detached);
    assert_eq!(rig.overlays(), []);
    assert!(rig.painted(IDLE_TEXT), "no monitor");
    assert!(rig.painted(&rig.status()));
    // The page now offers `Attach`, and its block stays: Attach hides the monitor and leaves the menu open.
    rig.open_menu(MenuTab::Project);
    rig.scroll_project_down();
    assert!(rig.painted("Attach") && !rig.painted("Detach"));
    rig.click_id(project_id("detach"));
    rig.frame(vec![]);
    assert!(!rig.app.slots.performance.detached);
    assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Project)]);
    assert!(rig.painted(IDLE_TEXT), "the in-page block stays");
    assert!(rig.painted("Detach"));
    // With the menu closed, no monitor is left.
    rig.app.navigate(Nav::Close);
    rig.frame(vec![]);
    assert!(!rig.painted(IDLE_TEXT));
}

#[test]
fn the_monitor_is_not_saved_and_survives_a_song_load() {
    let mut rig = Rig::with_synth();
    let settings = rig.app.settings.clone();
    rig.detach();
    assert_eq!(rig.app.settings, settings, "no setting is read or written");
    // Loading a song resets the main-menu slot, the graph and the song, not the monitor.
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.frame(vec![]);
    assert!(rig.app.slots.performance.detached);
    assert!(rig.painted(IDLE_TEXT));
}

#[test]
fn the_floating_monitor_is_16_by_9_u_against_the_right_edge_and_centred() {
    let mut rig = Rig::with_synth();
    rig.detach();
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
    let want = monitor_rect(
        egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0)),
        &m,
    );
    assert_eq!(
        want,
        egui::Rect::from_min_size(
            pos2(1170.0 - 16.0 * U, (540.0 - 9.0 * U) / 2.0),
            vec2(16.0 * U, 9.0 * U)
        )
    );
    // The status line at its top left, the footer at its bottom.
    let (_, status) = rig.text_shape(&rig.status()).expect("status");
    let (_, footer) = rig.text_shape(&rig.footer()).expect("footer");
    assert!(
        (status.left() - want.left()).abs() < 2.0 && (status.top() - want.top()).abs() < 4.0,
        "{status:?} {want:?}"
    );
    assert!(
        (footer.left() - want.left()).abs() < 2.0 && (footer.bottom() - want.bottom()).abs() < 4.0,
        "{footer:?} {want:?}"
    );
}

#[test]
fn the_monitor_has_no_touch_handling_of_its_own() {
    // The close X of the transport bar lies under the monitor (x 658 to 1170, y 126 to 414): its tap still hides the
    // panel, and the monitor stays: touches pass through it.
    let mut rig = Rig::with_synth();
    rig.settle();
    rig.detach();
    let on_it = rig.text_centre(IDLE_TEXT).expect("painted");
    rig.click(on_it);
    rig.frame(vec![]);
    assert!(rig.painted(IDLE_TEXT), "a tap on it does nothing to it");
    assert_eq!(rig.overlays(), []);
    rig.click(rig.close_x());
    assert!(
        rig.app.main_screen.panel_hidden,
        "the tap reached the close X under the monitor"
    );
    assert!(rig.app.slots.performance.detached);
}

#[test]
fn the_monitor_is_above_the_keyboard_panel_and_below_the_main_menu() {
    let mut rig = Rig::with_synth();
    rig.detach();
    let monitor = rig.text_shape(IDLE_TEXT).expect("monitor").0;
    // Over the transport bar and the keys, and over the playlist.
    for under in ["BPM", "C4"] {
        let at = rig
            .text_shape(under)
            .unwrap_or_else(|| panic!("no {under}"))
            .0;
        assert!(monitor > at, "the monitor is painted after `{under}`");
    }
    // Under the main menu, which opens over it.
    rig.open_menu(MenuTab::Songs);
    let (monitor, menu) = (rig.text_shape(IDLE_TEXT), rig.text_shape("SONGS"));
    let (monitor, menu) = (monitor.expect("still floating").0, menu.expect("menu").0);
    assert!(monitor < menu, "the menu is painted after the monitor");
}

#[test]
fn the_monitors_status_follows_the_song_while_it_floats() {
    // The shared data is refreshed every frame, not only while the menu draws it.
    let mut rig = Rig::with_synth();
    rig.detach();
    assert!(rig.painted(&rig.status()));
    rig.app.add_default_drums(0);
    rig.frame(vec![]);
    assert_eq!(rig.app.song.channels.len() - 1, 2);
    assert!(rig.painted(&rig.status()), "{:?}", rig.status());
}

#[test]
fn a_tap_on_cpu_or_ram_opens_project_with_the_performance_block_in_view() {
    // Releasing inside the CPU or RAM label rect <= 0.3 s after the press opens the main menu on PROJECT, then scrolls:
    // the block is below the fold at first, so it is scrolled to the end, where the clamp leaves it fully in view.
    for which in 0..2 {
        let mut rig = Rig::with_synth();
        let (cpu, ram) = rig.settle();
        let label = [cpu, ram][which];
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
fn a_press_that_leaves_the_label_or_runs_past_0_3_s_does_not_open_the_menu() {
    let mut rig = Rig::with_synth();
    let (cpu, _) = rig.settle();
    // Released outside the label.
    rig.press(cpu);
    rig.frame(vec![egui::Event::PointerMoved(ELSEWHERE)]);
    rig.release(ELSEWHERE);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    // Held past 0.3 s: a tooltip's press, not a tap.
    rig.hold(cpu, 5);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
}

#[test]
fn a_long_press_pins_a_tooltip_and_pinning_one_unpins_the_other_and_hover_shows_it() {
    let menu_fill = rig_menu_fill();
    let mut rig = Rig::with_synth();
    let (cpu, ram) = rig.settle();
    assert!(!rig.painted("Utilization:"));
    // Pinned by a press > 0.3 s: still there with the pointer gone.
    rig.hold(cpu, 5);
    rig.frame(vec![egui::Event::PointerMoved(ELSEWHERE)]);
    rig.frame(vec![]);
    for text in ["Utilization:", "Peak:", "Underruns:"] {
        assert!(rig.painted(text), "{text}");
    }
    assert!(!rig.painted("Physical memory:"));
    // 5 U wide, a `menuFill` box.
    let width = |rig: &Rig, w: f32| {
        rig.fills()
            .iter()
            .any(|(r, c)| *c == menu_fill && (r.width() - w).abs() < 0.01)
    };
    assert!(width(&rig, 5.0 * U), "the CPU tooltip is 5 U wide");
    assert_eq!(rig.overlays(), [], "a long press is no tap");
    // Pinning RAM's unpins CPU's: RAM's is 7 U wide with its three rows.
    rig.hold(ram, 5);
    rig.frame(vec![egui::Event::PointerMoved(ELSEWHERE)]);
    rig.frame(vec![]);
    for text in ["Physical memory:", "Memory used:", "App memory:"] {
        assert!(rig.painted(text), "{text}");
    }
    assert!(!rig.painted("Utilization:"));
    assert!(width(&rig, 7.0 * U), "the RAM tooltip is 7 U wide");
    // A second long press on it unpins it.
    rig.hold(ram, 5);
    rig.frame(vec![egui::Event::PointerMoved(ELSEWHERE)]);
    rig.frame(vec![]);
    assert!(!rig.painted("Physical memory:"));
    // A hovering mouse shows it without a press.
    rig.frame(vec![egui::Event::PointerMoved(cpu)]);
    rig.frame(vec![]);
    assert!(rig.painted("Utilization:") && rig.painted("Underruns:"));
    rig.frame(vec![egui::Event::PointerMoved(ELSEWHERE)]);
    rig.frame(vec![]);
    assert!(!rig.painted("Utilization:"));
}

fn rig_menu_fill() -> Color32 {
    crate::theme::theme_by_id(0).colors.menu_fill.to_color32()
}

#[test]
fn the_monitor_floats_over_the_rack_and_the_corner_buttons_and_under_the_menu_and_dialogs() {
    // Above the rack and the corner buttons (the `↖`, the rack tab and the logo), below the main menu, the dialogs and
    // The popups.
    let mut rig = Rig::with_synth();
    rig.settle();
    rig.detach();
    rig.app.navigate(Nav::Open(Overlay::Rack(1)));
    rig.slide_rack();
    rig.frame(vec![]);
    let monitor = rig.text_shape(&rig.status()).expect("the monitor").0;
    for under in ["Add Module", "SENDS"] {
        let at = rig
            .text_shape(under)
            .unwrap_or_else(|| panic!("no rack text {under}"))
            .0;
        assert!(
            monitor > at,
            "the monitor is painted after the rack's `{under}`"
        );
    }
    for (id, what) in [("app_logo", "logo"), ("edge_tab", "rack tab")] {
        let centre = rig
            .ctx
            .read_response(egui::Id::new(id))
            .expect("the button")
            .rect
            .center();
        let disc = (rig.out.shapes.iter())
            .position(|s| matches!(&s.shape, Shape::Circle(c) if c.center == centre && c.fill != Color32::TRANSPARENT))
            .unwrap_or_else(|| panic!("no {what} disc"));
        assert!(monitor > disc, "the monitor is painted after the {what}");
    }
    // A dialog opened over it is painted after it.
    rig.app.navigate(Nav::Open(Overlay::Message {
        tag: "perf_layering",
        lines: vec!["A question".to_owned()],
        confirm: Some(("Yes".to_owned(), "No".to_owned())),
        extra: None,
    }));
    rig.frame(vec![]);
    rig.frame(vec![]);
    let (monitor, dialog) = (rig.text_shape(&rig.status()), rig.text_shape("A question"));
    assert!(monitor.expect("still floating").0 < dialog.expect("the dialog").0);
}

#[test]
fn the_app_feeds_the_data_every_frame_and_the_idle_text_leaves_at_the_first_playback_for_good() {
    let mut rig = Rig::with_synth();
    // A clip 8 beats long makes the song's graph 8 beats; the transport at beat 2 is a quarter of the way.
    rig.app.song.channels[1].lanes[0].add_clip(0, 8.0);
    rig.click(OUTSIDE);
    rig.detach();
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
    assert!(rig.app.slots.performance.played && rig.app.slots.performance.detached);
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
    rig.detach();
    rig.frame(vec![]);
    let readout = rig.app.main_screen.ram_text().to_owned();
    assert!(!readout.is_empty());
    let shown = if readout.starts_with("--") {
        "-"
    } else {
        readout.as_str()
    };
    let system = rig.app.main_screen.system_memory();
    let p = &rig.app.slots.performance;
    assert_eq!((p.app_memory.as_str(), p.system_memory), (shown, system));
    assert!(
        rig.painted(&footer_line(shown, system)),
        "{shown} {system:?}"
    );
    // Linux reports the machine's memory in `/proc/meminfo`.
    if cfg!(target_os = "linux") {
        assert!(system.is_some_and(|m| m.used <= m.total));
    }
}

#[test]
fn the_cpu_meters_background_blinks_with_the_smoothed_load_under_the_load_bar() {
    // CPU background blink: the meter rect is filled from the ramp at u, the clock deciding the blink.
    let mut rig = Rig::with_synth();
    rig.settle();
    let background = crate::theme::theme_by_id(0).colors.background.to_color32();
    let meter = |rig: &Rig| {
        // 0.6 U x 0.12 U, the green bar's track.
        (rig.fills().into_iter())
            .filter(|(r, _)| {
                (r.width() - 0.6 * U).abs() < 0.01 && (r.height() - 0.12 * U).abs() < 0.01
            })
            .collect::<Vec<_>>()
    };
    // At rest: the track is the background.
    assert_eq!(meter(&rig).first().map(|f| f.1), Some(background));
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..14 {
        // Held at the top: the next update eases it to 0.98.
        rig.app.slots.performance.smoothed = 1.0;
        rig.frame(vec![]);
        let (u, t) = (rig.app.slots.performance.smoothed, rig.time);
        assert!((u - 0.98).abs() < 1e-6);
        let want = crate::ui::transport_extras::cpu_blink(u, t, background);
        let found = meter(&rig);
        assert_eq!(found.first().map(|f| f.1), Some(want), "t {t}");
        seen.insert([want.r(), want.g(), want.b()]);
    }
    assert_eq!(seen.len(), 2, "tinted and plain both: {seen:?}");
    // The load bar is drawn on top of it: the green fill is painted after the meter's rect (here it is empty).
    let after = |rig: &Rig| {
        let at = rig.out.shapes.iter().position(|s| matches!(&s.shape, Shape::Rect(r) if (r.rect.width() - 0.6 * U).abs() < 0.01 && (r.rect.height() - 0.12 * U).abs() < 0.01 && r.fill != Color32::TRANSPARENT));
        at.is_some()
    };
    assert!(after(&rig));
}

#[test]
fn the_cpu_tooltip_shows_the_smoothed_load_not_the_raw_one() {
    // The CPU/RAM tooltips: `Utilization:` is the smoothed load and `Peak:` the peak, both as unpadded `%.0f %%`.
    let mut rig = Rig::with_synth();
    let (cpu, _) = rig.settle();
    rig.hold(cpu, 5);
    rig.app.slots.performance.smoothed = 0.5;
    rig.frame(vec![egui::Event::PointerMoved(ELSEWHERE)]);
    // This frame's update eased it 2 % of the way toward the raw load, 0.
    assert!(rig.painted("Utilization:"));
    assert!(rig.painted("49 %"));
}

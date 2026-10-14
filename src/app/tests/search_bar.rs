//! The browsers' search bar in the app: `startSearch` (Ctrl+F) acts only in a browser that has the keyboard focus,
//! Escape hides the bar before it closes the picker, and each opening starts with the search reset.

use eframe::egui::{Event, Id, Key, Modifiers};

use super::rack_wiring::BROWSER;
use super::*;
use crate::ui::modules_browser::InsertAt;

/// The SONGS page's search field.
fn songs_field() -> Id {
    Id::new("main_menu_songs").with("search")
}

/// The Modules browser's search field.
fn browser_field() -> Id {
    Id::new(BROWSER).with("query")
}

impl Rig {
    /// A key's press and release, then a frame.
    fn search_key(&mut self, key: Key, modifiers: Modifiers) {
        self.frame(key_tap(key, modifiers));
        self.frame(vec![]);
    }

    fn ctrl_f(&mut self) {
        self.search_key(Key::F, Modifiers::COMMAND);
    }

    fn has_focus(&self, id: Id) -> bool {
        self.ctx.memory(|mem| mem.has_focus(id))
    }

    /// [`Rig::with_synth`] with its rack and the Modules browser open over it, past their slide-ins.
    fn with_browser() -> Self {
        let mut rig = Rig::with_synth();
        rig.open_rack(1);
        let at = InsertAt::End;
        rig.app
            .navigate(Nav::Open(Overlay::ModulesBrowser { channel: 1, at }));
        rig.slide_rack();
        rig
    }
}

#[test]
fn ctrl_f_with_no_browser_focused_does_nothing_and_opens_nothing() {
    let mut rig = Rig::with_synth();
    rig.ctrl_f();
    assert_eq!(rig.overlays(), []);
    assert!(!rig.ctx.wants_keyboard_input());
    assert_eq!(rig.take(), Vec::<String>::new(), "no note either");
}

#[test]
fn a_picker_moves_the_focus_into_its_browser_where_ctrl_f_shows_the_bar_and_focuses_its_field() {
    let mut rig = Rig::with_browser();
    rig.ctrl_f();
    assert!(rig.painted("Clear") && rig.has_focus(browser_field()));
}

#[test]
fn ctrl_f_in_a_picker_needs_the_focus_inside_its_browser() {
    let mut rig = Rig::with_browser();
    // A tap on the header's empty part: the focus goes to the root, out of the browser.
    rig.click(pos2(200.0, 30.0));
    rig.ctrl_f();
    assert!(!rig.painted("Clear") && !rig.has_focus(browser_field()));
    // A tap on `Sort` puts it back inside; Escape then closes its menu only.
    rig.click_id(Id::new(BROWSER).with("sort"));
    rig.frame(escape());
    rig.frame(vec![]);
    rig.ctrl_f();
    assert!(rig.painted("Clear") && rig.has_focus(browser_field()));
}

#[test]
fn escape_hides_the_pickers_bar_and_a_second_escape_closes_it() {
    let mut rig = Rig::with_browser();
    let browser = rig.overlays();
    rig.ctrl_f();
    rig.search_key(Key::Escape, Modifiers::NONE);
    assert_eq!(rig.overlays(), browser, "the first Escape hides the bar");
    assert!(!rig.painted("Clear"));
    rig.search_key(Key::Escape, Modifiers::NONE);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
}

#[test]
fn reopening_a_picker_resets_its_search() {
    let mut rig = Rig::with_browser();
    rig.ctrl_f();
    rig.frame(vec![Event::Text("tape".into())]);
    rig.frame(vec![]);
    assert!(!rig.painted("Paste"));
    for _ in 0..2 {
        rig.search_key(Key::Escape, Modifiers::NONE);
    }
    let at = InsertAt::End;
    rig.app
        .navigate(Nav::Open(Overlay::ModulesBrowser { channel: 1, at }));
    rig.slide_rack();
    assert!(!rig.painted("Clear") && rig.painted("Paste"));
    rig.ctrl_f();
    assert!(rig.painted("Clear") && !rig.painted("tape") && rig.painted("Paste"));
}

#[test]
fn f8_opens_songs_with_the_focus_in_its_list_so_ctrl_f_searches_it_and_a_reopening_resets_it() {
    let mut rig = Rig::new();
    rig.ctrl_f();
    assert_eq!(rig.overlays(), []);
    rig.search_key(Key::F8, Modifiers::NONE);
    assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Songs)]);
    rig.ctrl_f();
    assert!(rig.painted("Clear") && rig.has_focus(songs_field()));
    rig.frame(vec![Event::Text("zz".into())]);
    rig.frame(vec![]);
    // Escape hides the bar, then closes the menu.
    rig.search_key(Key::Escape, Modifiers::NONE);
    assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Songs)]);
    rig.search_key(Key::Escape, Modifiers::NONE);
    assert_eq!(rig.overlays(), []);
    // Opened again: no bar, and Ctrl+F shows an empty field.
    rig.search_key(Key::F8, Modifiers::NONE);
    assert!(!rig.painted("Clear"));
    rig.ctrl_f();
    assert!(rig.painted("Clear") && !rig.painted("zz"));
}

#[test]
fn f8_on_the_open_menu_moves_the_focus_back_into_the_songs_list() {
    let mut rig = Rig::new();
    rig.search_key(Key::F8, Modifiers::NONE);
    // A tap on the menu's header: the focus leaves the list.
    let header = crate::ui::main_menu::shell_layout(
        egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0)),
        &Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX),
    )
    .header;
    rig.click(header.center());
    rig.ctrl_f();
    assert!(!rig.painted("Clear"));
    rig.search_key(Key::F8, Modifiers::NONE);
    rig.ctrl_f();
    assert!(rig.painted("Clear") && rig.has_focus(songs_field()));
}

#[test]
fn closing_the_modules_browser_gives_the_focus_back_to_the_rack() {
    // F6 shows the rack and focuses it; the browser opened over it takes the focus into its list; Escape closes it.
    let mut rig = Rig::with_synth();
    rig.search_key(Key::F6, Modifiers::NONE);
    rig.slide_rack();
    let rack = Id::new(crate::ui::focus::RACK);
    assert!(crate::ui::focus::within(&rig.ctx, rack));
    let at = InsertAt::End;
    rig.app
        .navigate(Nav::Open(Overlay::ModulesBrowser { channel: 1, at }));
    rig.slide_rack();
    assert!(!crate::ui::focus::within(&rig.ctx, rack));
    rig.search_key(Key::Escape, Modifiers::NONE);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    rig.frame(vec![]);
    assert!(crate::ui::focus::within(&rig.ctx, rack), "back in the rack");
}

#[test]
fn closing_the_main_menu_f8_opened_gives_the_focus_back() {
    let mut rig = Rig::with_synth();
    rig.search_key(Key::F6, Modifiers::NONE);
    rig.slide_rack();
    let rack = Id::new(crate::ui::focus::RACK);
    assert!(crate::ui::focus::within(&rig.ctx, rack));
    rig.search_key(Key::F8, Modifiers::NONE);
    assert!(!crate::ui::focus::within(&rig.ctx, rack));
    rig.search_key(Key::Escape, Modifiers::NONE);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    rig.frame(vec![]);
    assert!(crate::ui::focus::within(&rig.ctx, rack), "back in the rack");
}

#[test]
fn f8_twice_then_escape_gives_the_focus_back_and_keeps_no_way_back() {
    let mut rig = Rig::with_synth();
    rig.search_key(Key::F6, Modifiers::NONE);
    rig.slide_rack();
    let rack = Id::new(crate::ui::focus::RACK);
    let opener = crate::ui::focus::focused(&rig.ctx);
    assert!(crate::ui::focus::within(&rig.ctx, rack));
    for _ in 0..2 {
        rig.search_key(Key::F8, Modifiers::NONE);
        rig.frame(vec![]);
    }
    rig.search_key(Key::Escape, Modifiers::NONE);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    rig.frame(vec![]);
    assert_eq!(
        crate::ui::focus::focused(&rig.ctx),
        opener,
        "back on the opener"
    );
    assert_eq!(crate::ui::focus::ways_back(&rig.ctx), 0);
}

#[test]
fn songs_opened_from_the_logo_gives_the_focus_back_to_it_on_close() {
    let mut rig = Rig::new();
    rig.click(LOGO);
    assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Songs)]);
    rig.frame(vec![]);
    assert!(crate::ui::focus::within(
        &rig.ctx,
        Id::new(crate::ui::focus::SONGS)
    ));
    rig.search_key(Key::Escape, Modifiers::NONE);
    assert_eq!(rig.overlays(), []);
    rig.frame(vec![]);
    assert_eq!(
        crate::ui::focus::focused(&rig.ctx),
        Some(Id::new("app_logo"))
    );
}

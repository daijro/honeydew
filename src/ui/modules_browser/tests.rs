use super::*;

/// The Modules browser's list: `Paste`, `Rack Preset`, `Instrument`, then the sections. The NOTE EFFECT section's five entries are
/// all the note effects of the registry. Lemon Reverb and Lemon Limiter are Honeydew's own (`honeydew_audio::lemon`), as is
/// the Dynamic EQ, and Lemon Trio, Lemon Sampler and Lemon Parametric EQ its built-in plugins (`crate::plugins::builtin`).
const SPEC: &[(&str, &[&str])] = &[
    ("", &["Paste", "Rack Preset", "Instrument"]),
    (
        "SYNTH",
        &[
            "3x Osc",
            "Lemon Trio",
            "DW Sampler",
            "Lemon Sampler",
            "GMSynth",
            "MiniSynth",
            "Slicer",
            "SoundFont Player",
            "SuperSaw",
            "Transistor Bass",
        ],
    ),
    (
        "ECHO",
        &[
            "Reverb",
            "Reverb 2",
            "Lemon Reverb",
            "Tape Delay",
            "Trance Delay",
        ],
    ),
    (
        "LEVEL",
        &[
            "Autoduck",
            "Compressor",
            "Distortion",
            "Gate",
            "Leveller",
            "Limiter",
            "Lemon Limiter",
            "Multiband Compressor",
            "Waveshaper",
        ],
    ),
    (
        "FILTER",
        &[
            "Comb Filter",
            "Dynamic EQ",
            "Equalizer",
            "Lemon Parametric EQ",
            "Filter",
            "Graphic EQ",
            "Spacer",
            "Spreader",
            "Stereoizer",
            "Tuned EQ",
        ],
    ),
    (
        "MISC",
        &[
            "Analyzer",
            "Chorus",
            "Flanger",
            "MultiFX",
            "Phaser",
            "Pitcher",
            "Pitcher 2",
            "Tape Stop",
            "Tremolo",
            "Tuner",
            "Wow & Flutter",
        ],
    ),
    (
        "NOTE EFFECT",
        &[
            "Arpeggiator",
            "Note Echo",
            "Randomizer",
            "Scale",
            "Transpose",
        ],
    ),
];

/// Rows as text: a section is its label, an entry its name.
fn names(rows: &[Row]) -> Vec<&'static str> {
    rows.iter().map(|r| r.name()).collect()
}

/// The list in the Modules picker's own ordering, `Category`, for a channel that takes instruments.
fn rows(query: &str) -> Vec<Row> {
    super::rows(query, ModuleOrdering::Category, true)
}

#[test]
fn the_list_is_the_spec_table() {
    let spec: Vec<&str> = SPEC
        .iter()
        .flat_map(|(section, items)| {
            (!section.is_empty())
                .then_some(*section)
                .into_iter()
                .chain(items.iter().copied())
        })
        .collect();
    assert_eq!(names(&rows("")), spec);
    // Headers are exactly the section labels.
    let headers: Vec<_> = rows("")
        .into_iter()
        .filter_map(|r| match r {
            Row::Section(s) => Some(s),
            Row::Item(_) => None,
        })
        .collect();
    assert_eq!(
        headers,
        ["SYNTH", "ECHO", "LEVEL", "FILTER", "MISC", "NOTE EFFECT"]
    );
}

#[test]
fn every_current_effect_is_listed_once_and_legacy_ids_never() {
    let listed: Vec<u16> = rows("")
        .into_iter()
        .filter_map(|r| match r {
            Row::Item(Entry::Module(id)) => Some(id),
            _ => None,
        })
        .collect();
    for legacy in [5, 10, 12] {
        assert!(!listed.contains(&legacy), "legacy id {legacy} is listed");
    }
    let mut listed_sorted = listed.clone();
    listed_sorted.sort();
    let current: Vec<u16> = (params::modules().chain(&params::LEMON_MODULES))
        .chain([&params::DYNAMIC_EQ])
        .filter(|m| !m.legacy)
        .map(|m| m.id)
        .collect();
    assert_eq!(listed_sorted, current);
    // The duplicate display names resolve to the current versions.
    let id_of = |name| {
        rows("")
            .into_iter()
            .find_map(|r| match r {
                Row::Item(e @ Entry::Module(id)) if e.name() == name => Some(id),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(
        (id_of("Reverb"), id_of("Limiter"), id_of("Autoduck")),
        (32, 28, 29)
    );
}

/// The SYNTH rows' module ids by display name.
const SYNTH_IDS: [(&str, u16); 8] = [
    ("3x Osc", 39),
    ("DW Sampler", 1),
    ("GMSynth", 2),
    ("MiniSynth", 0),
    ("Slicer", 33),
    ("SoundFont Player", 52),
    ("SuperSaw", 23),
    ("Transistor Bass", 19),
];

#[test]
fn where_no_instrument_goes_the_list_leaves_out_instrument_the_synths_and_the_note_effects() {
    // Each row picks its module; the actions are their own.
    for (name, id) in SYNTH_IDS {
        let entry = rows(name).into_iter().find_map(|r| match r {
            Row::Item(e @ Entry::Synth(_)) => Some(e),
            _ => None,
        });
        let entry = entry.unwrap_or_else(|| panic!("{name} not listed"));
        assert_eq!(entry.action(), BrowserAction::Pick(id), "{name}");
        // The id is the registry's instrument of that name.
        assert_eq!(params::module_spec(id).map(|s| s.display_name), Some(name));
    }
    assert_eq!(Entry::Instrument.action(), BrowserAction::Instrument);
    // Drums, audio, an effect track: the same list without `Instrument`, SYNTH and NOTE EFFECT, in both orderings.
    for sort in [ModuleOrdering::Category, ModuleOrdering::AZ] {
        let all = names(&super::rows("", sort, true));
        let some = names(&super::rows("", sort, false));
        let synths = SYNTH_IDS.map(|(name, _)| name);
        let lemon = ["Lemon Trio", "Lemon Sampler"];
        let notes = [
            "NOTE EFFECT",
            "Arpeggiator",
            "Note Echo",
            "Randomizer",
            "Scale",
            "Transpose",
        ];
        let gone = |n: &&str| {
            *n == "Instrument"
                || *n == "SYNTH"
                || synths.contains(n)
                || lemon.contains(n)
                || notes.contains(n)
        };
        let kept: Vec<_> = all.iter().filter(|n| !gone(n)).copied().collect();
        assert_eq!(some, kept, "{sort:?}");
        assert!(all.contains(&"Instrument") && some.contains(&"Paste") && some.contains(&"Reverb"));
    }
    let gone = names(&super::rows("", ModuleOrdering::Category, false));
    assert_eq!(&gone[..3], ["Paste", "Rack Preset", "ECHO"]);
}

#[test]
fn search_keeps_matching_entries_and_hides_empty_sections() {
    assert_eq!(
        names(&rows("delay")),
        ["ECHO", "Tape Delay", "Trance Delay"]
    );
    // Case is ignored, on both sides.
    assert_eq!(names(&rows("DeLaY")), names(&rows("delay")));
    assert_eq!(
        names(&rows("echo")),
        ["NOTE EFFECT", "Note Echo"],
        "section labels are not searched"
    );
    assert_eq!(
        names(&rows("pa")),
        ["Paste", "FILTER", "Lemon Parametric EQ", "Spacer"],
        "the actions are entries too"
    );
    assert_eq!(
        names(&rows("eq")),
        [
            "FILTER",
            "Dynamic EQ",
            "Equalizer",
            "Lemon Parametric EQ",
            "Graphic EQ",
            "Tuned EQ"
        ]
    );
    assert_eq!(
        names(&rows("tr")),
        [
            "Instrument",
            "SYNTH",
            "Lemon Trio",
            "Transistor Bass",
            "ECHO",
            "Trance Delay",
            "FILTER",
            "Lemon Parametric EQ",
            "MISC",
            "Tremolo",
            "NOTE EFFECT",
            "Transpose",
        ]
    );
    assert!(rows("zzz").is_empty());
}

// ---- applying a pick ----

/// A rack with Equalizer (id 2), Arpeggiator (id 3), Reverb (id 4): the counter is at 4.
fn rack() -> Rack {
    let mut rack = Rack::default();
    for id in [3, 51, 32] {
        rack.add(id);
    }
    rack
}

fn chain(rack: &Rack) -> Vec<(u16, i32)> {
    rack.modules
        .iter()
        .map(|m| (m.type_id, m.instance_id))
        .collect()
}

#[test]
fn add_module_appends_an_effect_with_a_fresh_id_and_defaults() {
    let mut r = rack();
    assert_eq!(add_module(&mut r, 6, InsertAt::End), 5);
    assert_eq!(chain(&r), [(3, 2), (51, 3), (32, 4), (6, 5)]);
    let distortion = params::module_spec(6).unwrap();
    let defaults: Vec<f32> = distortion.params.iter().map(|p| p.default).collect();
    assert_eq!(r.modules[3].params, defaults);
    // Ids are never reused, even after a removal.
    r.remove(5);
    assert_eq!(add_module(&mut r, 6, InsertAt::End), 6);
}

#[test]
fn insert_above_and_below_a_module() {
    let mut r = rack();
    assert_eq!(add_module(&mut r, 8, InsertAt::Above(3)), 5);
    assert_eq!(chain(&r), [(3, 2), (8, 5), (51, 3), (32, 4)]);
    assert_eq!(add_module(&mut r, 9, InsertAt::Below(4)), 6);
    assert_eq!(chain(&r), [(3, 2), (8, 5), (51, 3), (32, 4), (9, 6)]);
    assert_eq!(add_module(&mut r, 11, InsertAt::Above(2)), 7);
    assert_eq!(r.modules[0].instance_id, 7);
    assert_eq!(add_module(&mut r, 13, InsertAt::Below(2)), 8);
    assert_eq!(chain(&r)[..3], [(11, 7), (3, 2), (13, 8)]);
    // An explicit position holds for note effects too.
    assert_eq!(add_module(&mut r, 48, InsertAt::Below(4)), 9);
    assert_eq!(r.index_of(9), Some(r.index_of(4).unwrap() + 1));
}

#[test]
fn insert_next_to_a_module_that_is_gone_appends() {
    let mut r = rack();
    assert_eq!(add_module(&mut r, 6, InsertAt::Above(99)), 5);
    assert_eq!(add_module(&mut r, 7, InsertAt::Below(99)), 6);
    assert_eq!(chain(&r)[3..], [(6, 5), (7, 6)]);
}

#[test]
fn a_note_effect_added_at_the_end_joins_the_note_effects_at_the_top() {
    let mut r = Rack::default();
    // Empty rack: the first place.
    assert_eq!(add_module(&mut r, 51, InsertAt::End), 2);
    r.add(3);
    // After the leading note effect, before the Equalizer.
    assert_eq!(add_module(&mut r, 47, InsertAt::End), 4);
    assert_eq!(chain(&r), [(51, 2), (47, 4), (3, 3)]);
    // A note effect further down does not count: only the run at the top.
    r.add(49);
    assert_eq!(add_module(&mut r, 50, InsertAt::End), 6);
    assert_eq!(chain(&r), [(51, 2), (47, 4), (50, 6), (3, 3), (49, 5)]);
}

#[test]
fn replace_puts_the_pick_in_the_modules_place_with_a_fresh_id() {
    let mut r = rack();
    assert_eq!(add_module(&mut r, 6, InsertAt::Replace(3)), 5);
    assert_eq!(chain(&r), [(3, 2), (6, 5), (32, 4)]);
    assert_eq!(r.modules[1], ModuleInstance::new(6, 5));
    // The place holds for a note effect too; a module that is gone appends.
    assert_eq!(add_module(&mut r, 48, InsertAt::Replace(4)), 6);
    assert_eq!(chain(&r), [(3, 2), (6, 5), (48, 6)]);
    assert_eq!(add_module(&mut r, 7, InsertAt::Replace(99)), 7);
    assert_eq!(chain(&r)[3], (7, 7));
}

/// A Chorus copied off another rack: changed params and common params, a minimized card, a `CSTM` blob.
fn clipboard() -> ModuleInstance {
    let mut m = ModuleInstance::new(11, 9);
    m.params[1] = 0.9;
    m.common.0[1] = 0.4;
    m.minimized = true;
    m.file.custom_data = Some(vec![1, 2, 3]);
    m
}

#[test]
fn paste_inserts_a_copy_with_a_fresh_id_where_a_pick_would_go() {
    let copy = |id| ModuleInstance {
        instance_id: id,
        ..clipboard()
    };
    let mut r = rack();
    assert_eq!(paste_module(&mut r, &clipboard(), InsertAt::Above(3)), 5);
    assert_eq!(r.modules[1], copy(5));
    assert_eq!(paste_module(&mut r, &clipboard(), InsertAt::End), 6);
    assert_eq!(r.modules[4], copy(6));
    assert_eq!(paste_module(&mut r, &clipboard(), InsertAt::Replace(2)), 7);
    assert_eq!(chain(&r), [(11, 7), (11, 5), (51, 3), (32, 4), (11, 6)]);
    assert_eq!(r.modules[0], copy(7));
    // A copied note effect at the end joins the run of note effects at the top (none here: the first place).
    let mut r = rack();
    let arp = ModuleInstance::new(51, 9);
    assert_eq!(paste_module(&mut r, &arp, InsertAt::End), 5);
    assert_eq!(chain(&r), [(51, 5), (3, 2), (51, 3), (32, 4)]);
}

// ---- the panel, offscreen ----

use eframe::egui::epaint::ClippedShape;
use eframe::egui::{self, Event, Key, Modifiers, PointerButton, RawInput, Shape, pos2, vec2};

use crate::metrics::DEFAULT_UI_SCALE;
use crate::theme::theme_by_id;

const SCREEN: Rect = Rect {
    min: pos2(0.0, 0.0),
    max: pos2(1000.0, 800.0),
};

/// `U` = 32 points at ppp 1.
fn metrics() -> Metrics {
    Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE)
}

struct Rig {
    ctx: Context,
    browser: ModulesBrowser,
    time: f64,
    /// Whether the channel takes instruments: the SYNTH rows are enabled.
    instruments: bool,
    /// What the last frame returned and painted.
    action: Option<BrowserAction>,
    shapes: Vec<ClippedShape>,
    /// The keyboard focus runs as in the app: its frame start and end around the browser.
    focus: bool,
}

impl Rig {
    /// The browser just opened over a channel that takes instruments, run until the slide-in is over.
    fn new() -> Self {
        Self::over(true)
    }

    /// The browser just opened over a channel that takes instruments or not.
    fn over(instruments: bool) -> Self {
        let mut rig = Self {
            ctx: Context::default(),
            browser: ModulesBrowser::default(),
            time: 0.0,
            instruments,
            action: None,
            shapes: Vec::new(),
            focus: false,
        };
        for _ in 0..30 {
            rig.frame(vec![]);
        }
        rig
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (theme, m) = (theme_by_id(0), metrics());
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let browser = &mut self.browser;
        let mut action = None;
        let (instruments, focus) = (self.instruments, self.focus);
        let output = self.ctx.run(raw, |ctx| {
            if focus {
                crate::ui::focus::begin_frame(ctx);
            }
            action = browser.show(ctx, &cx, instruments);
            if focus {
                crate::ui::focus::end_frame(ctx, &theme);
            }
        });
        self.action = action;
        self.shapes = output.shapes;
    }

    fn button(&mut self, pos: Pos2, button: PointerButton, pressed: bool) {
        self.frame(vec![Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }]);
    }

    /// Returns what the release frame returned.
    fn click_with(&mut self, pos: Pos2, button: PointerButton) -> Option<BrowserAction> {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.button(pos, button, true);
        self.button(pos, button, false);
        self.action
    }

    fn click(&mut self, pos: Pos2) -> Option<BrowserAction> {
        self.click_with(pos, PointerButton::Primary)
    }

    fn press_escape(&mut self) {
        self.frame(vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
    }

    /// A mouse wheel turn of `dy` points over `pos` (negative scrolls the list down), then the frames the
    /// smoothing takes to deliver it.
    fn wheel(&mut self, pos: Pos2, dy: f32) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, dy),
            modifiers: Modifiers::NONE,
        }]);
        for _ in 0..30 {
            self.frame(vec![]);
        }
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.frame(vec![Event::PointerMoved(from)]);
        self.button(from, PointerButton::Primary, true);
        for i in 1..=10 {
            self.frame(vec![Event::PointerMoved(
                from + (to - from) * (i as f32 / 10.0),
            )]);
        }
        self.button(to, PointerButton::Primary, false);
    }

    /// Texts painted in the last frame, with where they were drawn.
    fn texts(&self) -> Vec<(String, Rect)> {
        self.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.galley.rect.translate(t.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect()
    }

    fn shows(&self, text: &str) -> bool {
        self.texts().iter().any(|(t, _)| t == text)
    }

    fn text_at(&self, text: &str) -> Pos2 {
        let texts = self.texts();
        let found = texts.iter().find(|(t, _)| t == text);
        found
            .unwrap_or_else(|| panic!("{text} not drawn: {texts:?}"))
            .1
            .center()
    }

    fn layout(&self) -> Layout {
        layout(SCREEN, 1.0, &metrics())
    }
}

#[test]
fn the_panel_is_18_u_wide_on_the_left() {
    // 18 U (U = 32), at most the screen's width.
    let m = metrics();
    let at = layout(SCREEN, 1.0, &m);
    assert_eq!(
        at.panel,
        Rect::from_min_size(pos2(0.0, 0.0), vec2(576.0, 800.0))
    );
    assert_eq!(
        at.header,
        Rect::from_min_size(pos2(0.0, 0.0), vec2(576.0, 64.0))
    );
    assert_eq!(
        at.list,
        Rect::from_min_max(pos2(64.0, 64.0), pos2(576.0, 800.0))
    );
    // The buttons: Search and Sort down the left edge, 0.25 U apart, the close X top-right.
    assert_eq!(at.button(0), pos2(32.0, 64.0 + 16.0 + 24.0));
    assert_eq!(at.button(1), pos2(32.0, 104.0 + 48.0 + 8.0));
    assert_eq!(at.close, pos2(576.0 - 32.0, 32.0));
    let narrow = Rect::from_min_size(Pos2::ZERO, vec2(500.0, 800.0));
    assert_eq!(layout(narrow, 1.0, &m).panel.width(), 500.0);
}

#[test]
fn the_panel_slides_in_from_the_left_with_p_halving_its_gap_a_frame_as_the_screen_dims() {
    // p ← (p + 1)/2 a drawn frame from 0, 1 once the gap is under 0.001 (frame 10); drawn at x + (p − 1)·W, the
    // screen dimmed to α 0.6·p.
    let mut rig = Rig {
        ctx: Context::default(),
        browser: ModulesBrowser::default(),
        time: 0.0,
        instruments: true,
        action: None,
        shapes: Vec::new(),
        focus: false,
    };
    crate::ui::anim::enable(&rig.ctx);
    let colors = theme_by_id(0).colors;
    let (fill, bg) = (
        colors.menu_fill.to_color32(),
        colors.background.to_color32(),
    );
    let mut p = 0.0f32;
    for n in 1..=11 {
        rig.frame(vec![]);
        p = if n >= 10 { 1.0 } else { (p + 1.0) / 2.0 };
        let panel = (rig.shapes.iter()).find_map(|c| match &c.shape {
            Shape::Rect(r) if r.fill == fill && r.rect.height() == 800.0 => Some(r.rect),
            _ => None,
        });
        let dim = (rig.shapes.iter()).find_map(|c| match &c.shape {
            Shape::Rect(r) if r.rect == SCREEN => Some(r.fill),
            _ => None,
        });
        assert_eq!(dim, Some(bg.gamma_multiply(0.6 * p)), "frame {n}");
        // egui lays a new area out unseen the first time it ever shows it.
        if n > 1 {
            let panel = panel.expect("the panel");
            assert_eq!(panel.left(), ((p - 1.0) * 576.0).round(), "frame {n}");
        }
    }
    assert_eq!(p, 1.0);
}

#[test]
fn the_rows_are_drawn_in_order_down_the_list() {
    let rig = Rig::new();
    assert!(rig.shows("Modules") && rig.shows("Search") && rig.shows("Sort"));
    let list = rig.layout().list;
    let ys: Vec<f32> = ["Paste", "Rack Preset", "SYNTH", "3x Osc", "ECHO", "Reverb"]
        .iter()
        .map(|t| rig.text_at(t).y)
        .collect();
    assert!(ys.windows(2).all(|w| w[1] > w[0]), "{ys:?}");
    // One row per 1.5 U, the first at the top of the list.
    assert_eq!(ys[0], list.top() + 24.0);
    assert_eq!(ys[1] - ys[0], 48.0);
    // Rows below the list are not drawn.
    assert!(!rig.shows("Transpose"));
}

#[test]
fn picking_an_effect_returns_its_id() {
    // ECHO in view: the list scrolled 200 points.
    let scrolled = || {
        let mut rig = Rig::new();
        rig.drag(pos2(200.0, 480.0), pos2(200.0, 280.0));
        rig
    };
    let mut rig = scrolled();
    let at = rig.text_at("Reverb 2");
    assert_eq!(rig.click(at), Some(BrowserAction::Pick(35)));
    let mut rig = scrolled();
    let at = rig.text_at("Reverb");
    assert_eq!(rig.click(at), Some(BrowserAction::Pick(32)));
}

#[test]
fn section_headers_pick_nothing_and_an_instrument_row_picks_its_module_id() {
    let mut rig = Rig::new();
    let at = rig.text_at("ECHO");
    assert_eq!(rig.click(at), None);
    for (name, id) in SYNTH_IDS {
        let mut rig = Rig::new();
        let at = rig.text_at(name);
        assert_eq!(rig.click(at), Some(BrowserAction::Pick(id)), "{name}");
    }
}

impl Rig {
    /// The colour `text` was painted in.
    fn color(&self, text: &str) -> egui::Color32 {
        let found = self.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == text => {
                Some(t.galley.job.sections[0].format.color)
            }
            _ => None,
        });
        found.unwrap_or_else(|| panic!("{text} not drawn"))
    }
}

#[test]
fn paste_is_always_enabled() {
    // Even with nothing on the clipboard: the caller then flashes `Cannot paste module`.
    let text = theme_by_id(0).colors.text.to_color32();
    let mut rig = Rig::new();
    assert_eq!(rig.color("Paste"), text);
    let at = rig.text_at("Paste");
    assert_eq!(rig.click(at), Some(BrowserAction::Paste));
}

#[test]
fn rack_preset_asks_for_the_rack_preset_picker() {
    let text = theme_by_id(0).colors.text.to_color32();
    let mut rig = Rig::new();
    assert_eq!(rig.color("Rack Preset"), text);
    let at = rig.text_at("Rack Preset");
    assert_eq!(rig.click(at), Some(BrowserAction::RackPreset));
}

#[test]
fn the_close_x_an_outside_click_and_escape_close() {
    let mut rig = Rig::new();
    let close = rig.layout().close;
    assert_eq!(rig.click(close), Some(BrowserAction::Close));

    let mut rig = Rig::new();
    assert_eq!(rig.click(pos2(800.0, 250.0)), Some(BrowserAction::Close));

    let mut rig = Rig::new();
    rig.press_escape();
    assert_eq!(rig.action, Some(BrowserAction::Close));

    // A click on the panel outside any control does not close it.
    let mut rig = Rig::new();
    assert_eq!(rig.click(pos2(200.0, 30.0)), None);
}

#[test]
fn a_secondary_or_middle_click_outside_closes_like_the_popup() {
    for button in [PointerButton::Secondary, PointerButton::Middle] {
        let mut rig = Rig::new();
        assert_eq!(
            rig.click_with(pos2(800.0, 250.0), button),
            Some(BrowserAction::Close),
            "{button:?}"
        );
        // On the panel they close nothing, as the primary click does not.
        let mut rig = Rig::new();
        assert_eq!(
            rig.click_with(pos2(200.0, 30.0), button),
            None,
            "{button:?}"
        );
    }
}

#[test]
fn escape_hides_the_search_bar_first_then_closes() {
    // Escape order: bar shown → hide it; else close.
    let mut rig = Rig::new();
    let search = rig.layout().button(0);
    assert_eq!(rig.click(search), None);
    rig.frame(vec![Event::Text("de".into())]);
    // Typing reached the field, so it has focus.
    assert!(rig.typing());
    assert!(rig.shows("de"));
    rig.escape();
    assert_eq!(rig.action, None, "the first Escape only hides the bar");
    rig.frame(vec![]);
    assert!(
        !rig.shows("Clear") && rig.shows("Paste"),
        "hidden, the query cleared"
    );
    let text = theme_by_id(0).colors.text.to_color32();
    assert_eq!(rig.color("Search"), text, "Search unlit");
    rig.escape();
    assert_eq!(rig.action, Some(BrowserAction::Close));
}

#[test]
fn search_filters_as_you_type_and_hides_again_keeping_its_text() {
    let mut rig = Rig::new();
    let search = rig.layout().button(0);
    assert_eq!(rig.click(search), None);
    rig.frame(vec![Event::Text("de".into())]);
    rig.frame(vec![Event::Text("lay".into())]);
    rig.frame(vec![]);
    assert!(rig.shows("ECHO") && rig.shows("Tape Delay") && rig.shows("Trance Delay"));
    for gone in ["Paste", "SYNTH", "Reverb", "LEVEL"] {
        assert!(!rig.shows(gone), "{gone}");
    }
    // The query is centred in the bar (y + 0.25 U, 1.5 U tall); the list starts 1.75 U lower.
    let list = rig.layout().list;
    let query = rig.text_at("delay").y - list.top();
    assert!((query - 32.0).abs() <= 1.0, "{query}");
    assert_eq!(rig.text_at("ECHO").y, list.top() + 56.0 + 24.0);
    let at = rig.text_at("Trance Delay");
    assert_eq!(rig.click(at), Some(BrowserAction::Pick(16)));

    // Search again hides the bar and clears the filter; the field keeps its text, so showing it again filters again.
    let mut rig = Rig::new();
    rig.click(search);
    rig.frame(vec![Event::Text("zzz".into())]);
    rig.frame(vec![]);
    assert!(!rig.shows("Paste"));
    rig.click(search);
    rig.frame(vec![]);
    assert!(rig.shows("Paste") && rig.shows("Reverb") && !rig.shows("zzz"));
    rig.click(search);
    rig.frame(vec![]);
    assert!(rig.shows("zzz") && !rig.shows("Paste"));
}

// ---- the orderings and the `Sort` menu ----

#[test]
fn the_modules_picker_orders_by_category_and_stores_each_ordering_by_its_id() {
    // The ordering ids are A-Z 1, Top Level 2, Category 3.
    use ModuleOrdering::*;
    assert_eq!(ModuleOrdering::default(), Category);
    for (id, ordering) in [(1, AZ), (2, TopLevel), (3, Category)] {
        assert_eq!(ordering.id(), id);
        assert_eq!(ModuleOrdering::from_id(id), Some(ordering));
    }
    for id in [0, 4] {
        assert_eq!(ModuleOrdering::from_id(id), None);
    }
    assert_eq!(
        [AZ, TopLevel, Category].map(ModuleOrdering::label),
        ["A-Z", "Top Level", "Category"]
    );
}

#[test]
fn top_level_lists_the_actions_then_the_instruments_then_the_effects_each_by_type_id() {
    // By item type, then a per-type index, else alphanumeric. The types are the actions, the instruments and the
    // effects (note effects included); the index is the module type id; no section rows.
    let top = super::rows("", ModuleOrdering::TopLevel, true);
    assert_eq!(
        names(&top),
        [
            // The actions, which have no index: by name.
            "Instrument",
            "Paste",
            "Rack Preset",
            // Instruments: 0, 1, 2, 19, 23, 33, 39, 52.
            "MiniSynth",
            "DW Sampler",
            "GMSynth",
            "Transistor Bass",
            "SuperSaw",
            "Slicer",
            "3x Osc",
            "SoundFont Player",
            // The built-in plugins, which have no type id of their own: by name.
            "Lemon Sampler",
            "Lemon Trio",
            // Effects: 3 … 46, then the note effects 47 … 51.
            "Equalizer",
            "MultiFX",
            "Distortion",
            "Graphic EQ",
            "Compressor",
            "Filter",
            "Chorus",
            "Flanger",
            "Phaser",
            "Stereoizer",
            "Trance Delay",
            "Tape Delay",
            "Leveller",
            "Tuner",
            "Pitcher",
            "Analyzer",
            "Gate",
            "Limiter",
            "Autoduck",
            "Tuned EQ",
            "Reverb",
            "Spacer",
            "Reverb 2",
            "Multiband Compressor",
            "Comb Filter",
            "Waveshaper",
            "Tape Stop",
            "Wow & Flutter",
            "Spreader",
            "Pitcher 2",
            "Tremolo",
            "Note Echo",
            "Transpose",
            "Randomizer",
            "Scale",
            "Arpeggiator",
            // Honeydew's own: 1002, 1003, 1004, then the built-in plugin.
            "Lemon Reverb",
            "Lemon Limiter",
            "Dynamic EQ",
            "Lemon Parametric EQ",
        ]
    );
    // The search filters it, in the same order.
    assert_eq!(
        names(&super::rows("DELAY", ModuleOrdering::TopLevel, true)),
        ["Trance Delay", "Tape Delay"]
    );
}

#[test]
fn a_z_lists_every_entry_by_its_name_naturally_with_no_sections() {
    // A-Z: natural alphanumeric compare of the display name, with no grouping before it, so the actions sort in with
    // the modules.
    let az = names(&super::rows("", ModuleOrdering::AZ, true));
    let entries = rows("").into_iter().filter(|r| matches!(r, Row::Item(_)));
    assert_eq!(az.len(), entries.count());
    assert_eq!(&az[..4], ["3x Osc", "Analyzer", "Arpeggiator", "Autoduck"]);
    let at = |name| az.iter().position(|n| *n == name).unwrap();
    assert_eq!(
        (at("Pitcher") + 1, at("Reverb") + 1),
        (at("Pitcher 2"), at("Reverb 2"))
    );
    assert!(at("Note Echo") < at("Paste") && at("Paste") < at("Phaser"));
    assert!(at("Graphic EQ") < at("Instrument") && at("Instrument") < at("Lemon Limiter"));
    assert!(at("Pitcher 2") < at("Rack Preset") && at("Rack Preset") < at("Randomizer"));
    // Without case: a byte compare would put `GMSynth` first.
    assert!(at("Gate") < at("GMSynth"));
    assert_eq!(az.last(), Some(&"Wow & Flutter"));
}

impl Rig {
    /// Taps `Sort`, then lets its menu settle: egui lays a new area out unseen the first time it shows it.
    fn open_sort(&mut self) {
        let sort = self.layout().button(1);
        assert_eq!(self.click(sort), None);
        self.frame(vec![]);
        self.frame(vec![]);
    }
}

#[test]
fn sort_offers_a_z_and_category_whose_pick_reorders_the_list() {
    // `Sort`: the Modules picker offers A-Z and Category in creation order, never Top Level; Category is active on
    // opening, and the active one is the highlighted row.
    let mut rig = Rig::new();
    assert!(rig.shows("SYNTH"));
    rig.open_sort();
    assert!(rig.shows("A-Z") && rig.shows("Category"));
    assert!(!rig.shows("Top Level"));
    assert!(rig.text_at("A-Z").y < rig.text_at("Category").y);
    let (plain, current) = (rig.color("A-Z"), rig.color("Category"));
    assert_ne!(plain, current, "Category highlighted");
    let item = rig.text_at("A-Z");
    assert_eq!(rig.click(item), None);
    rig.frame(vec![]);
    assert_eq!(rig.browser.sort, ModuleOrdering::AZ);
    assert!(!rig.shows("Category"), "the menu closed");
    assert!(!rig.shows("SYNTH"), "no section rows");
    assert!(rig.text_at("3x Osc").y < rig.text_at("Analyzer").y);
    let osc = rig.text_at("3x Osc");
    assert_eq!(rig.click(osc), Some(BrowserAction::Pick(39)));
    // Open again: A-Z is the current one.
    rig.open_sort();
    assert_eq!(
        (rig.color("A-Z"), rig.color("Category")),
        (current, plain),
        "A-Z highlighted"
    );
    // Escape closes the menu only.
    rig.press_escape();
    rig.frame(vec![]);
    assert!(!rig.shows("Category") && rig.shows("Modules"));
    assert_eq!(rig.action, None);
}

#[test]
fn dragging_scrolls_to_the_end_of_the_list_and_no_further() {
    let mut rig = Rig::new();
    let list = rig.layout().list;
    for _ in 0..6 {
        rig.drag(pos2(200.0, 480.0), pos2(200.0, 80.0));
    }
    // The drags pick nothing, and the last row now sits at the bottom of the list.
    assert_eq!(rig.action, None);
    assert!(!rig.shows("Paste"));
    assert_eq!(rig.text_at("Transpose").y, list.bottom() - 24.0);
    let at = rig.text_at("Transpose");
    assert_eq!(rig.click(at), Some(BrowserAction::Pick(48)));
}

#[test]
fn the_wheel_scrolls_the_list_only_over_it() {
    let mut rig = Rig::new();
    let list = rig.layout().list;
    let first = rig.text_at("3x Osc").y;
    assert_eq!(first, list.top() + 4.0 * 48.0 + 24.0);
    // Over the panel's button column: nothing moves.
    rig.wheel(pos2(30.0, 400.0), -80.0);
    assert_eq!(rig.text_at("3x Osc").y, first);
    // Over the list, a turn of 80 points moves the rows up by that much, and back.
    rig.wheel(pos2(200.0, 400.0), -80.0);
    assert_eq!(rig.text_at("3x Osc").y, first - 80.0);
    rig.wheel(pos2(200.0, 400.0), 40.0);
    assert_eq!(rig.text_at("3x Osc").y, first - 40.0);
    // It stops at the top.
    rig.wheel(pos2(200.0, 400.0), 1000.0);
    assert_eq!(rig.text_at("3x Osc").y, first);
    // And at the end.
    rig.wheel(pos2(200.0, 400.0), -5000.0);
    assert_eq!(rig.text_at("Transpose").y, list.bottom() - 24.0);
}

#[test]
fn a_narrower_query_clamps_the_scroll_to_the_shorter_list() {
    // Scrolled to the end of the full list, with the search field open.
    let scrolled = || {
        let mut rig = Rig::new();
        let search = rig.layout().button(0);
        rig.click(search);
        for _ in 0..7 {
            rig.drag(pos2(200.0, 480.0), pos2(200.0, 120.0));
        }
        assert!(rig.shows("Transpose"));
        rig
    };

    // Still longer than the list: the end of the new list sits at the bottom, not past it.
    let mut rig = scrolled();
    let list = rig.layout().list;
    // The search bar moves the list down 1.75 U; the rows are 48 points high.
    let shown = rows("e");
    assert!(
        shown.len() as f32 * 48.0 > list.height() - 56.0,
        "pick a longer query"
    );
    rig.frame(vec![Event::Text("e".into())]);
    rig.frame(vec![]);
    let last = shown.last().unwrap().name();
    assert_eq!(rig.text_at(last).y, list.bottom() - 24.0);

    // Short enough to fit: back at the top, under the search bar, not scrolled out of sight.
    let mut rig = scrolled();
    let list = rig.layout().list;
    rig.frame(vec![Event::Text("delay".into())]);
    rig.frame(vec![]);
    assert_eq!(rig.text_at("ECHO").y, list.top() + 56.0 + 24.0);
    assert!(rig.shows("Tape Delay") && rig.shows("Trance Delay"));
}

#[test]
fn the_rows_take_touches_at_rest_while_the_panel_slides_in() {
    // No animation blocks input, and touch handlers never check the animation state. On its 3rd frame the panel is
    // drawn 72 points short of its rest (p = 0.875), but a row takes a touch where it will rest.
    let paste = Rig::new().text_at("Paste");
    let at = pos2(560.0, paste.y);
    let mut rig = Rig {
        ctx: Context::default(),
        browser: ModulesBrowser::default(),
        time: 0.0,
        instruments: true,
        action: None,
        shapes: Vec::new(),
        focus: false,
    };
    crate::ui::anim::enable(&rig.ctx);
    rig.frame(vec![]);
    assert_eq!(rig.click(at), Some(BrowserAction::Paste));
    // Drawn where it is in the slide meanwhile: the row text is still left of its rest.
    let mut rig = Rig {
        ctx: Context::default(),
        browser: ModulesBrowser::default(),
        time: 0.0,
        instruments: true,
        action: None,
        shapes: Vec::new(),
        focus: false,
    };
    crate::ui::anim::enable(&rig.ctx);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(
        (rig.text_at("Paste").x - (paste.x - 144.0)).abs() < 0.5,
        "frame 2: p = 0.75"
    );
}

impl Rig {
    /// The size of the font `text` was drawn in.
    fn font_size(&self, text: &str) -> f32 {
        let size = self.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == text => {
                Some(t.galley.job.sections[0].format.font_id.size)
            }
            _ => None,
        });
        size.unwrap_or_else(|| panic!("{text} not drawn"))
    }
}

/// Every item row in the folder rows' FontSize 4, a section's label in FontSize 3; the left buttons' labels are 8 pt,
/// shrunk to fit their button.
#[test]
fn every_row_is_in_fontsize_4_and_the_button_labels_in_8_pt() {
    let rig = Rig::new();
    let m = metrics();
    let pt = |s: FontSize| m.font_id(s).size;
    assert_eq!(rig.font_size("Reverb"), pt(FontSize::Pt18));
    assert_eq!(rig.font_size("Paste"), pt(FontSize::Pt18));
    assert_eq!(rig.font_size("ECHO"), pt(FontSize::Pt12));
    let d = m.u(BUTTON_D);
    for label in ["Search", "Sort"] {
        let size = rig.font_size(label);
        assert!(
            size <= pt(FontSize::Pt8) && size > pt(FontSize::Pt6),
            "{label}: {size}"
        );
        let (_, rect) = (rig.texts().into_iter()).find(|(t, _)| t == label).unwrap();
        assert!(
            rect.width() <= d,
            "{label} fits its button: {}",
            rect.width()
        );
    }
}

#[test]
fn a_sort_pick_lists_from_the_top_and_refreshes_the_list() {
    // A sort is a list refresh (`SidePanel::refreshed`): the audition strip turns off; the reordered list starts at
    // its top.
    let mut rig = Rig::new();
    rig.drag(pos2(200.0, 480.0), pos2(200.0, 80.0));
    assert!(!rig.shows("Paste") && rig.browser.panel.scroll_y > 0.0);
    rig.browser.panel.audition.on = true;
    rig.open_sort();
    let item = rig.text_at("A-Z");
    rig.click(item);
    rig.frame(vec![]);
    assert_eq!(rig.browser.panel.scroll_y, 0.0);
    assert_eq!(rig.text_at("3x Osc").y, rig.layout().list.top() + 24.0);
    assert!(!rig.browser.panel.audition.on, "refreshed");
}

// ---- the search bar ----

impl Rig {
    fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.frame(crate::ui::offscreen::key_tap(key, modifiers));
    }

    /// `startSearch`: Ctrl+F (Cmd+F on a Mac).
    fn ctrl_f(&mut self) {
        self.key(Key::F, Modifiers::COMMAND);
    }

    /// `close`: Escape, pressed and released.
    fn escape(&mut self) {
        self.key(Key::Escape, Modifiers::NONE);
    }

    /// The search field has the keyboard.
    fn typing(&self) -> bool {
        let field = Id::new("modules_browser").with("query");
        self.ctx.memory(|mem| mem.has_focus(field))
    }

    /// Where widget `id` was laid out.
    fn rect_of(&self, id: Id) -> Rect {
        self.ctx.read_response(id).expect("drawn").rect
    }

    /// The texts `text` was painted as, with their colours, in paint order.
    fn painted_as(&self, text: &str) -> Vec<(Rect, egui::Color32, f32)> {
        (self.shapes.iter())
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    let format = &t.galley.job.sections[0].format;
                    let rect = t.galley.rect.translate(t.pos.to_vec2());
                    Some((rect, format.color, format.font_id.size))
                }
                _ => None,
            })
            .collect()
    }
}

#[test]
fn ctrl_f_shows_the_bar_lit_moves_the_list_down_and_gives_the_field_the_keyboard() {
    // The picker moved the focus into its browser as it opened, so Ctrl+F is the browser's.
    let mut rig = Rig::new();
    assert!(!rig.shows("Clear") && !rig.typing());
    let paste = rig.text_at("Paste").y;
    rig.ctrl_f();
    rig.frame(vec![]);
    let id = Id::new("modules_browser");
    // (x + 1.75 U, y + 0.25 U, w − 2 U, 1.5 U) under the 2 U header (U = 32, the panel 576 wide); `Clear` 2 U wide at
    // its right end, the field the rest.
    let bar = Rect::from_min_size(pos2(56.0, 72.0), vec2(512.0, 48.0));
    assert_eq!(
        rig.rect_of(id.with("query").with("clear")),
        bar.with_min_x(bar.right() - 64.0)
    );
    assert_eq!(
        rig.rect_of(id.with("query")),
        bar.with_max_x(bar.right() - 64.0)
    );
    assert!(rig.typing());
    // `Search` is lit: its label in the ink of a filled circle.
    let c = theme_by_id(0).colors;
    let (text, ink) = (c.text.to_color32(), c.background.to_color32());
    assert_eq!(rig.color("Search"), ink);
    // The empty field shows `Search` at 25 % alpha, in 12 pt; `Clear` is a `menuFill` label on its `text` fill.
    let m = metrics();
    let hint = rig.painted_as("Search")[1];
    assert!(bar.contains(hint.0.center()));
    assert_eq!(hint.1, text.gamma_multiply(0.25));
    assert_eq!(hint.2, m.font_id(FontSize::Pt12).size);
    assert_eq!(rig.color("Clear"), c.menu_fill.to_color32());
    // The list moves down 1.75 U.
    assert_eq!(rig.text_at("Paste").y, paste + 56.0);
    // Typing goes to the field.
    rig.frame(vec![Event::Text("tape".into())]);
    rig.frame(vec![]);
    assert!(rig.shows("tape") && rig.shows("Tape Delay") && !rig.shows("Paste"));
}

#[test]
fn a_second_ctrl_f_keeps_the_bar_its_text_and_its_filter() {
    let mut rig = Rig::new();
    rig.ctrl_f();
    rig.frame(vec![Event::Text("tape".into())]);
    rig.frame(vec![]);
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(rig.shows("Clear") && rig.shows("tape") && rig.typing());
    assert!(rig.shows("Tape Delay") && !rig.shows("Paste"));
}

#[test]
fn ctrl_f_brings_back_the_hidden_fields_text_as_the_query() {
    let mut rig = Rig::new();
    rig.ctrl_f();
    rig.frame(vec![Event::Text("tape".into())]);
    rig.escape();
    rig.frame(vec![]);
    assert!(rig.shows("Paste") && !rig.shows("tape"));
    rig.ctrl_f();
    assert!(
        rig.shows("Tape Delay") && !rig.shows("Paste"),
        "filtered at once"
    );
    assert!(rig.shows("tape") && rig.typing());
}

#[test]
fn clear_or_deleting_every_character_empties_the_query_and_keeps_the_bar_open_and_focused() {
    let mut rig = Rig::new();
    rig.ctrl_f();
    rig.frame(vec![Event::Text("tape".into())]);
    rig.frame(vec![]);
    assert!(!rig.shows("Paste"));
    let clear = rig
        .rect_of(Id::new("modules_browser").with("query").with("clear"))
        .center();
    assert_eq!(rig.click(clear), None);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.shows("Paste") && rig.shows("Clear") && !rig.shows("tape"));
    assert!(rig.typing());

    let mut rig = Rig::new();
    rig.ctrl_f();
    rig.frame(vec![Event::Text("ta".into())]);
    for _ in 0..2 {
        rig.key(Key::Backspace, Modifiers::NONE);
    }
    rig.frame(vec![]);
    assert!(rig.shows("Paste") && rig.shows("Clear") && rig.typing());
}

#[test]
fn each_edit_refilters_on_the_next_frame_with_the_query_trimmed() {
    let mut rig = Rig::new();
    rig.ctrl_f();
    rig.frame(vec![Event::Text("  tape ".into())]);
    assert!(
        rig.shows("Paste"),
        "the edit's own frame still shows the old rows"
    );
    rig.frame(vec![]);
    assert!(rig.shows("Tape Delay") && !rig.shows("Paste"), "trimmed");
}

#[test]
fn enter_in_the_field_does_nothing() {
    let mut rig = Rig::new();
    rig.ctrl_f();
    rig.frame(vec![Event::Text("tape".into())]);
    rig.frame(vec![]);
    rig.key(Key::Enter, Modifiers::NONE);
    assert_eq!(rig.action, None);
    rig.frame(vec![]);
    assert!(rig.typing() && rig.shows("tape") && rig.shows("Tape Delay"));
    assert!(!rig.shows("Paste"));
}

#[test]
fn the_caret_is_1_px_wide_and_shows_for_half_of_a_60_frame_cycle() {
    let mut rig = Rig::new();
    rig.ctrl_f();
    let text = theme_by_id(0).colors.text.to_color32();
    let field = Rect::from_min_size(pos2(56.0, 72.0), vec2(448.0, 48.0));
    let caret = |rig: &Rig| {
        rig.shapes.iter().any(|c| match &c.shape {
            egui::Shape::LineSegment { points, stroke } => {
                stroke.width == 1.0 && stroke.color == text && field.contains(points[0])
            }
            _ => false,
        })
    };
    let mut shown = Vec::new();
    for _ in 0..120 {
        rig.frame(vec![]);
        shown.push(caret(&rig));
    }
    let on = shown.iter().filter(|&&s| s).count();
    assert!((58..=62).contains(&on), "{on} of 120 frames");
    assert!(shown[..25].iter().all(|&s| s) && shown[35..55].iter().all(|&s| !s));
}

#[test]
fn ctrl_f_restores_the_hidden_text_trimmed() {
    let mut rig = Rig::new();
    rig.ctrl_f();
    rig.frame(vec![Event::Text(" tape ".into())]);
    rig.escape();
    rig.frame(vec![]);
    rig.ctrl_f();
    assert!(
        rig.shows("Tape Delay") && !rig.shows("Paste"),
        "filtered at once, trimmed"
    );
}

#[test]
fn up_or_down_in_the_field_leaves_it_for_the_list() {
    // The field's keys are Left/Right, Backspace, Delete, paste and the printable ones.
    let mut rig = Rig::new();
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(rig.typing());
    rig.key(Key::ArrowDown, Modifiers::NONE);
    rig.frame(vec![]);
    assert!(!rig.typing());
    let row = Id::new("modules_browser").with(("row", 0_usize));
    assert_eq!(crate::ui::focus::focused(&rig.ctx), Some(row));
    // The bar stays, with what was typed.
    assert!(rig.shows("Clear"));
}

#[test]
fn a_search_tap_then_down_moves_from_the_field_into_the_list() {
    // The tap puts the keyboard focus on `Search`; the field taking the keyboard moves it onto the field.
    let mut rig = Rig::new();
    rig.focus = true;
    let search = rig.layout().button(0);
    rig.click(search);
    rig.frame(vec![]);
    assert!(rig.typing());
    rig.key(Key::ArrowDown, Modifiers::NONE);
    rig.frame(vec![]);
    let row = Id::new("modules_browser").with(("row", 0_usize));
    assert_eq!(crate::ui::focus::focused(&rig.ctx), Some(row));
}

/// One frame at `t` of a bare side panel listing `rows` (name, folder), all enabled; its painted shapes.
fn bare_frame(
    ctx: &Context,
    panel: &mut SidePanel,
    t: f64,
    rows: &[(&str, bool)],
    events: Vec<Event>,
) -> Vec<ClippedShape> {
    let raw = RawInput {
        screen_rect: Some(SCREEN),
        time: Some(t),
        events,
        ..Default::default()
    };
    let (theme, m) = (theme_by_id(0), metrics());
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let chrome = Chrome {
        id: Id::new("bare"),
        title: "Bare",
        back: false,
        buttons: &[SideButton::Search],
        popup: false,
    };
    let output = ctx.run(raw, |ctx| {
        panel.show(ctx, &cx, &chrome, |_| {
            (rows.iter())
                .map(|&(name, folder)| PanelRow {
                    name,
                    folder,
                    sub: None,
                    kind: RowKind::Enabled(()),
                    picked: false,
                })
                .collect()
        });
    });
    output.shapes
}

/// Whether a flash (a translucent fill no taller than a row, brighter than the hover and press bands) is painted over
/// row `i` of the bare panel.
fn bare_flash(ctx: &Context, shapes: &[ClippedShape], i: usize) -> bool {
    let row = ctx
        .read_response(Id::new("bare").with(("row", i)))
        .unwrap()
        .rect;
    shapes.iter().any(|c| match &c.shape {
        Shape::Rect(r) => r.rect == row && (64..255).contains(&r.fill.a()),
        _ => false,
    })
}

#[test]
fn the_flash_skips_folders_starts_the_frame_after_the_tap_and_stays_on_its_rows_name() {
    // Row flash on pick.
    let (ctx, mut panel) = (Context::default(), SidePanel::in_place());
    let mut t = 0.0;
    let mut frame = |panel: &mut SidePanel, rows: &[(&str, bool)], events| {
        t += 1.0 / 60.0;
        bare_frame(&ctx, panel, t, rows, events)
    };
    let rows = [("Folder", true), ("Item", false), ("Other", false)];
    for _ in 0..3 {
        frame(&mut panel, &rows, vec![]);
    }
    let mut tap = |panel: &mut SidePanel, i: usize| {
        let at = (ctx.read_response(Id::new("bare").with(("row", i))))
            .unwrap()
            .rect
            .center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(panel, &rows, vec![Event::PointerMoved(at)]);
        frame(panel, &rows, vec![button(true)]);
        frame(panel, &rows, vec![button(false)])
    };
    // A folder's opening is its feedback.
    tap(&mut panel, 0);
    assert!(panel.flash.is_none());
    // An item: nothing on the tap's own frame (a picker that closes on it shows no flash), then the flash.
    let shapes = tap(&mut panel, 1);
    assert!(panel.flash.is_some() && !bare_flash(&ctx, &shapes, 1));
    let shapes = frame(&mut panel, &rows, vec![]);
    assert!(bare_flash(&ctx, &shapes, 1));
    // The list read again with a row above it: the row now at its index is another, and nothing flashes.
    let moved = [("Folder", true), ("New", false), ("Item", false)];
    let shapes = frame(&mut panel, &moved, vec![]);
    assert!(!bare_flash(&ctx, &shapes, 1) && !bare_flash(&ctx, &shapes, 2));
}

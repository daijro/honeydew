use eframe::egui::{
    CentralPanel, Color32, Frame, FullOutput, Key, Modifiers, PointerButton, RawInput, Shape, Vec2,
    pos2, vec2,
};

use super::*;
use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
use crate::theme::theme_by_id;
use crate::ui::UiCtx;
use crate::ui::widgets::{Control, knob};

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(pos2(x, y), vec2(w, h))
}

fn leaf(name: &str, rect: Rect) -> Node {
    Node {
        id: Id::new(name),
        rect,
        clip: Rect::EVERYTHING,
        kind: Kind::Button,
        order: Order::Background,
        parent: None,
        filled: false,
    }
}

/// Builds a tree the way the widgets do: `open` / `close` bracket a container's children.
#[derive(Default)]
struct Build(State);

impl Build {
    fn leaf(mut self, name: &str, rect: Rect) -> Self {
        self.0.add(leaf(name, rect));
        self
    }

    fn open(mut self, name: &str, rect: Rect) -> Self {
        let i = self.0.add(Node {
            kind: Kind::Container,
            ..leaf(name, rect)
        });
        self.0.open.push(i);
        self
    }

    fn close(mut self) -> Self {
        self.0.open.pop();
        self
    }

    fn modal(mut self, lowest: Order) -> Self {
        self.0.tree.modal = Some((self.0.tree.nodes.len(), lowest));
        self
    }

    fn layer(mut self, name: &str, rect: Rect, order: Order) -> Self {
        self.0.add(Node {
            order,
            ..leaf(name, rect)
        });
        self
    }

    fn tree(self) -> Tree {
        self.0.tree
    }
}

/// The names `dir` visits from `from`, `n` steps.
fn walk(tree: &Tree, from: Option<&str>, dir: Dir, n: usize, names: &[&str]) -> Vec<String> {
    let name = |id: Id| {
        names
            .iter()
            .find(|n| Id::new(**n) == id)
            .unwrap()
            .to_string()
    };
    let mut at = from.map(Id::new);
    (0..n)
        .map(|_| {
            at = tree.step(at, dir);
            at.map_or("-".into(), name)
        })
        .collect()
}

const NAMES: [&str; 8] = ["a", "rack", "b1", "b2", "hidden", "grid", "d", "x"];

/// a, the rack {b1, b2, a zero-size control}, an empty grid, d.
fn sample() -> Tree {
    Build::default()
        .leaf("a", r(0.0, 0.0, 10.0, 10.0))
        .open("rack", r(0.0, 20.0, 100.0, 50.0))
        .leaf("b1", r(0.0, 20.0, 10.0, 10.0))
        .leaf("b2", r(20.0, 20.0, 10.0, 10.0))
        .leaf("hidden", r(40.0, 20.0, 0.0, 10.0))
        .close()
        .open("grid", r(0.0, 80.0, 100.0, 50.0))
        .close()
        .leaf("d", r(0.0, 140.0, 10.0, 10.0))
        .tree()
}

#[test]
fn tab_steps_through_the_targets_in_tree_order_entering_containers_and_wrapping_at_the_root() {
    let tree = sample();
    assert_eq!(
        walk(&tree, None, Dir::Next, 6, &NAMES),
        ["a", "b1", "b2", "grid", "d", "a"]
    );
    // Moving back enters a container at its last control; Shift+Tab from nothing goes to the last target.
    assert_eq!(
        walk(&tree, None, Dir::Prev, 6, &NAMES),
        ["d", "grid", "b2", "b1", "a", "d"]
    );
    // A container focused as a whole passes Tab to its first control; one not drawn starts from the top.
    assert_eq!(walk(&tree, Some("rack"), Dir::Next, 1, &NAMES), ["b1"]);
    assert_eq!(walk(&tree, Some("x"), Dir::Next, 1, &NAMES), ["a"]);
    assert_eq!(walk(&tree, Some("x"), Dir::Prev, 1, &NAMES), ["d"]);
    assert_eq!(walk(&Tree::default(), None, Dir::Next, 1, &NAMES), ["-"]);
}

#[test]
fn a_modal_keeps_the_focus_among_what_it_draws_above_its_backdrop() {
    // A popup opened by b1 while the rack draws: b2 comes after it, but under its backdrop.
    let tree = Build::default()
        .leaf("a", r(0.0, 0.0, 10.0, 10.0))
        .leaf("b1", r(0.0, 20.0, 10.0, 10.0))
        .modal(Order::Foreground)
        .layer("x", r(50.0, 50.0, 10.0, 10.0), Order::Foreground)
        .layer("d", r(50.0, 70.0, 10.0, 10.0), Order::Foreground)
        .leaf("b2", r(20.0, 20.0, 10.0, 10.0))
        .tree();
    assert_eq!(
        walk(&tree, Some("b1"), Dir::Next, 3, &NAMES),
        ["x", "d", "x"]
    );
    assert_eq!(walk(&tree, Some("b1"), Dir::Down, 1, &NAMES), ["x"]);
    // A modal in the screens' own layer (the main menu): what it draws after its mark, whatever the layer.
    let tree = Build::default()
        .leaf("a", r(0.0, 0.0, 10.0, 10.0))
        .modal(Order::Background)
        .leaf("b1", r(0.0, 20.0, 10.0, 10.0))
        .leaf("b2", r(20.0, 20.0, 10.0, 10.0))
        .tree();
    assert_eq!(
        walk(&tree, Some("a"), Dir::Next, 3, &NAMES),
        ["b1", "b2", "b1"]
    );
}

#[test]
fn the_directional_score_is_the_gap_plus_the_misalignment_of_a_candidate_wholly_to_one_side() {
    let from = r(0.0, 0.0, 10.0, 10.0);
    // Level with it: the gap alone; overlapping across, however little: no misalignment.
    assert_eq!(score(from, r(15.0, 0.0, 10.0, 10.0), Dir::Right), Some(5.0));
    assert_eq!(score(from, r(15.0, 9.0, 10.0, 10.0), Dir::Right), Some(5.0));
    // Wholly below: the smaller of the start and end offsets (12 and 30).
    assert_eq!(
        score(from, r(15.0, 12.0, 10.0, 28.0), Dir::Right),
        Some(17.0)
    );
    // Wholly above: the start offset is 40, the end offset 20.
    assert_eq!(
        score(from, r(15.0, -40.0, 10.0, 30.0), Dir::Right),
        Some(25.0)
    );
    // Touching the edge is a gap of 0; reaching back past it is no candidate.
    assert_eq!(score(from, r(10.0, 0.0, 10.0, 10.0), Dir::Right), Some(0.0));
    assert_eq!(score(from, r(9.0, 0.0, 10.0, 10.0), Dir::Right), None);
    assert_eq!(
        score(from, r(-20.0, 0.0, 10.0, 10.0), Dir::Left),
        Some(10.0)
    );
    assert_eq!(
        score(from, r(20.0, -30.0, 10.0, 10.0), Dir::Up),
        Some(20.0 + 20.0)
    );
    assert_eq!(score(from, r(0.0, 25.0, 10.0, 10.0), Dir::Down), Some(15.0));
    assert_eq!(score(from, r(0.0, 25.0, 10.0, 10.0), Dir::Up), None);
    assert_eq!(score(from, r(0.0, 25.0, 10.0, 10.0), Dir::Next), None);
}

#[test]
fn the_arrows_take_the_least_score_keep_the_earlier_on_a_tie_and_stay_without_a_candidate() {
    // a  b       (b is far but level; c is near but low)
    //  c
    let tree = Build::default()
        .leaf("a", r(0.0, 0.0, 10.0, 10.0))
        .leaf("b1", r(40.0, 0.0, 10.0, 10.0))
        .leaf("b2", r(15.0, 30.0, 10.0, 10.0))
        .leaf("d", r(15.0, -30.0, 10.0, 10.0))
        .tree();
    // b1: 30 + 0; b2: 5 + 30; d: 5 + 30.
    assert_eq!(walk(&tree, Some("a"), Dir::Right, 1, &NAMES), ["b1"]);
    assert_eq!(walk(&tree, Some("b1"), Dir::Right, 1, &NAMES), ["b1"]);
    assert_eq!(walk(&tree, Some("b1"), Dir::Left, 1, &NAMES), ["a"]);
    assert_eq!(walk(&tree, Some("a"), Dir::Down, 1, &NAMES), ["b2"]);
    // b2 and d tie from between them (both 20 + 0 away vertically from a rect level with neither): the earlier.
    let tree = Build::default()
        .leaf("a", r(0.0, 0.0, 10.0, 10.0))
        .leaf("b2", r(20.0, 30.0, 10.0, 10.0))
        .leaf("d", r(20.0, -30.0, 10.0, 10.0))
        .tree();
    assert_eq!(walk(&tree, Some("a"), Dir::Right, 1, &NAMES), ["b2"]);
    // Nothing focused: the first target.
    assert_eq!(walk(&tree, None, Dir::Up, 1, &NAMES), ["a"]);
}

#[test]
fn the_cursor_frames_grow_3p_px_and_the_inner_one_sits_3_px_in_at_any_density() {
    let rect = r(100.0, 100.0, 40.0, 20.0);
    for ppp in [1.0, 2.0] {
        let px = 1.0 / ppp;
        let (outer, inner) = frames(rect, 1.0, ppp);
        assert_eq!(outer, rect.expand(3.0 * px), "{ppp}");
        assert_eq!(inner, rect, "{ppp}");
        let (outer, inner) = frames(rect, 0.0, ppp);
        assert_eq!((outer, inner), (rect, rect.shrink(3.0 * px)), "{ppp}");
        let (outer, _) = frames(rect, 0.5, ppp);
        assert_eq!(outer, rect.expand(1.5 * px), "{ppp}");
    }
}

#[test]
fn the_cursor_eases_a_third_of_the_way_and_the_pulse_runs_over_2_5_s() {
    let (from, to) = (r(0.0, 0.0, 10.0, 10.0), r(100.0, 50.0, 20.0, 10.0));
    let once = ease(from, to);
    assert_eq!(once.min, pos2(33.0, 16.5));
    assert_eq!(once.max, pos2(10.0 + 110.0 * 0.33, 10.0 + 50.0 * 0.33));
    let near = |a: f32, b: f32| (a - b).abs() < 1e-6;
    assert!(near(pulse(0.0), 0.5));
    assert!(near(pulse(0.625), 1.0));
    assert!(near(pulse(1.875), 0.0));
    assert!(near(pulse(2.5), 0.5));
}

#[test]
fn the_blink_grows_from_nothing_as_it_fades() {
    let rect = r(0.0, 0.0, 40.0, 20.0);
    let (fill, alpha) = flash(rect, 1.0);
    assert_eq!(
        (fill.size(), fill.center(), alpha),
        (Vec2::ZERO, rect.center(), 1.0)
    );
    let (fill, alpha) = flash(rect, 0.64);
    assert_eq!((fill.center(), alpha), (rect.center(), 0.8));
    assert!((fill.size() - vec2(14.4, 7.2)).length() < 1e-4);
}

/// One frame of an offscreen context at `ppp`, with the app's focus calls around `draw`.
struct Rig {
    ctx: Context,
    time: f64,
    ppp: f32,
    out: FullOutput,
}

impl Rig {
    fn new(ppp: f32) -> Self {
        let ctx = Context::default();
        // As the app sets it up: Ctrl+= is no egui zoom.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        Self {
            ctx,
            time: 0.0,
            ppp,
            out: FullOutput::default(),
        }
    }

    fn frame(&mut self, events: Vec<Event>, draw: &mut dyn FnMut(&mut Ui, &UiCtx)) {
        let mut raw = RawInput {
            screen_rect: Some(r(0.0, 0.0, 800.0, 600.0)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let viewport = raw.viewports.entry(raw.viewport_id).or_default();
        viewport.native_pixels_per_point = Some(self.ppp);
        self.time += 1.0 / 60.0;
        let (theme, m) = (
            theme_by_id(0),
            Metrics::from_pixels_per_point(self.ppp, DEFAULT_UI_SCALE_INDEX),
        );
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        self.out = self.ctx.run(raw, |ctx| {
            begin_frame(ctx);
            CentralPanel::default()
                .frame(Frame::NONE)
                .show(ctx, |ui| draw(ui, &cx));
            end_frame(ctx, &theme);
        });
    }

    /// The outer cursor frame painted last frame: the rect stroked 9 px wide.
    fn cursor(&self) -> Option<Rect> {
        let px = 9.0 / self.ppp;
        self.out.shapes.iter().find_map(|s| match &s.shape {
            Shape::Rect(rect) if rect.stroke.width == px => Some(rect.rect),
            _ => None,
        })
    }
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn up(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers,
    }
}

/// A key pressed and released in one frame. Two presses with no release between are a held key: egui marks the
/// second a repeat.
fn tap(key: Key, modifiers: Modifiers) -> Vec<Event> {
    vec![self::key(key, modifiers), up(key, modifiers)]
}

fn press(pos: Pos2) -> Vec<Event> {
    let button = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    vec![Event::PointerMoved(pos), button(true), button(false)]
}

const BUTTONS: [Rect; 3] = [
    Rect::from_min_max(pos2(10.0, 10.0), pos2(50.0, 30.0)),
    Rect::from_min_max(pos2(60.0, 10.0), pos2(100.0, 30.0)),
    Rect::from_min_max(pos2(10.0, 40.0), pos2(50.0, 60.0)),
];

/// Three buttons through [`interact`]; `clicks` counts each one's clicks.
fn buttons(clicks: &mut [usize; 3]) -> impl FnMut(&mut Ui, &UiCtx) + '_ {
    move |ui, _| {
        for (i, rect) in BUTTONS.into_iter().enumerate() {
            if interact(ui, rect, Id::new(("button", i)), Sense::click()).clicked() {
                clicks[i] += 1;
            }
        }
    }
}

#[test]
fn tab_turns_the_keyboard_look_on_and_a_touch_turns_it_off_and_focuses_what_it_touched() {
    let mut rig = Rig::new(1.0);
    let mut clicks = [0; 3];
    let mut draw = buttons(&mut clicks);
    rig.frame(vec![], &mut draw);
    assert_eq!(
        (focused(&rig.ctx), is_keyboard_mode(&rig.ctx)),
        (None, false)
    );
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
    assert_eq!(focused(&rig.ctx), Some(Id::new(("button", 0))));
    assert!(is_keyboard_mode(&rig.ctx));
    assert!(rig.cursor().is_some());
    rig.frame(tap(Key::ArrowRight, Modifiers::NONE), &mut draw);
    assert_eq!(focused(&rig.ctx), Some(Id::new(("button", 1))));
    rig.frame(tap(Key::Tab, Modifiers::SHIFT), &mut draw);
    assert_eq!(focused(&rig.ctx), Some(Id::new(("button", 0))));
    rig.frame(tap(Key::ArrowDown, Modifiers::NONE), &mut draw);
    assert_eq!(focused(&rig.ctx), Some(Id::new(("button", 2))));
    // A touch: the look goes, and the focus is what it touched.
    let events = press(BUTTONS[1].center());
    rig.frame(events, &mut draw);
    assert_eq!(focused(&rig.ctx), Some(Id::new(("button", 1))));
    assert!(!is_keyboard_mode(&rig.ctx));
    assert_eq!(rig.cursor(), None);
    // Ctrl+Left is `rewind`, no focus move.
    rig.frame(tap(Key::ArrowLeft, Modifiers::COMMAND), &mut draw);
    assert!(!is_keyboard_mode(&rig.ctx));
    drop(draw);
    assert_eq!(clicks, [0, 1, 0]);
}

#[test]
fn enter_presses_the_focused_button_and_space_does_not() {
    let mut rig = Rig::new(1.0);
    let mut clicks = [0; 3];
    let mut draw = buttons(&mut clicks);
    rig.frame(vec![], &mut draw);
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
    rig.frame(tap(Key::Enter, Modifiers::NONE), &mut draw);
    rig.frame(tap(Key::Space, Modifiers::NONE), &mut draw);
    // egui's own focus (which Tab would give the first button) is let go: Space clicks nothing.
    assert_eq!(rig.ctx.memory(|m| m.focused()), None);
    drop(draw);
    assert_eq!(clicks, [0, 1, 0]);
}

#[test]
fn the_cursor_snaps_on_first_show_then_eases_and_is_drawn_at_ppp_1_and_2() {
    for ppp in [1.0, 2.0] {
        let mut rig = Rig::new(ppp);
        let mut clicks = [0; 3];
        let mut draw = buttons(&mut clicks);
        rig.frame(vec![], &mut draw);
        // The pulse's peak at t = 0.625 s.
        rig.time = 0.625;
        rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
        assert_eq!(rig.cursor(), Some(BUTTONS[0].expand(3.0 / ppp)), "{ppp}");
        rig.time = 0.625;
        rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
        let eased = ease(BUTTONS[0], BUTTONS[1]);
        assert_eq!(rig.cursor(), Some(eased.expand(3.0 / ppp)), "{ppp}");
        // The inner frame: 3 px inside, 3 px wide, in `text`; the outer in the colour that contrasts best with it.
        let text = theme_by_id(0).colors.text.to_color32();
        let inner = rig.out.shapes.iter().find_map(|s| match &s.shape {
            Shape::Rect(rect) if rect.stroke.width == 3.0 / ppp => Some(rect.clone()),
            _ => None,
        });
        let inner = inner.expect("inner frame");
        let off = (inner.rect.min - eased.min).length() + (inner.rect.max - eased.max).length();
        assert!(off < 1e-4, "{ppp}: {:?} vs {eased:?}", inner.rect);
        assert_eq!(inner.stroke.color, text);
        let colors = theme_by_id(0).colors;
        let outer = colors.best_contrasting(colors.text, &colors.contrast_candidates());
        let outer_color = rig.out.shapes.iter().find_map(|s| match &s.shape {
            Shape::Rect(rect) if rect.stroke.width == 9.0 / ppp => Some(rect.stroke.color),
            _ => None,
        });
        assert_eq!(outer_color, Some(outer.to_color32()));
        assert_ne!(outer.to_color32(), text);
    }
}

#[test]
fn blink_cursor_flashes_a_text_fill_that_grows_and_fades_by_0_8_a_frame() {
    let mut rig = Rig::new(1.0);
    let mut clicks = [0; 3];
    let mut draw = buttons(&mut clicks);
    rig.frame(vec![], &mut draw);
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
    blink(&rig.ctx);
    let text = theme_by_id(0).colors.text.to_color32();
    let fill = |rig: &Rig| {
        rig.out.shapes.iter().find_map(|s| match &s.shape {
            Shape::Rect(rect) if rect.fill != Color32::TRANSPARENT && rect.stroke.width == 0.0 => {
                Some((rect.rect, rect.fill))
            }
            _ => None,
        })
    };
    let mut v = 1.0_f32;
    for _ in 0..3 {
        rig.frame(vec![], &mut draw);
        let (rect, color) = fill(&rig).expect("flash");
        assert!((rect.size() - BUTTONS[0].size() * (1.0 - v)).length() < 1e-3);
        assert_eq!(color, text.gamma_multiply(v.sqrt()));
        v *= 0.8;
    }
    // It stops once v is below 0.001: 0.8³¹ < 0.001 ≤ 0.8³⁰.
    for _ in 3..31 {
        rig.frame(vec![], &mut draw);
    }
    assert!(fill(&rig).is_some());
    rig.frame(vec![], &mut draw);
    assert_eq!(fill(&rig), None);
}

/// An opener button, and while `popup` a modal with one item.
fn opener(popup: bool) -> impl FnMut(&mut Ui, &UiCtx) {
    move |ui, _| {
        interact(ui, BUTTONS[0], Id::new("opener"), Sense::click());
        if popup {
            modal(ui.ctx(), Order::Background);
            interact(ui, BUTTONS[1], Id::new("item"), Sense::click());
        }
    }
}

#[test]
fn a_modal_takes_the_focus_and_gives_it_back_when_it_closes() {
    let mut rig = Rig::new(1.0);
    rig.frame(vec![], &mut opener(false));
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut opener(false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("opener")));
    rig.frame(vec![], &mut opener(true));
    assert_eq!(focused(&rig.ctx), Some(Id::new("item")));
    rig.frame(vec![], &mut opener(false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("opener")));
    // A focused control that stops drawing loses the focus to the root.
    rig.frame(vec![], &mut |_, _| {});
    assert_eq!(focused(&rig.ctx), None);
}

#[test]
fn a_container_focused_as_a_whole_passes_the_focus_to_its_first_control_and_holds_it() {
    let mut rig = Rig::new(1.0);
    let mut draw = |ui: &mut Ui, _: &UiCtx| {
        interact(ui, BUTTONS[0], Id::new("outside"), Sense::click());
        container(ui, Id::new(RACK), r(0.0, 35.0, 200.0, 100.0), |ui| {
            interact(ui, BUTTONS[2], Id::new("inside"), Sense::click());
        });
    };
    rig.frame(vec![], &mut draw);
    focus(&rig.ctx, Id::new(RACK));
    assert!(within(&rig.ctx, Id::new(RACK)));
    rig.frame(vec![], &mut draw);
    assert_eq!(focused(&rig.ctx), Some(Id::new("inside")));
    assert!(within(&rig.ctx, Id::new(RACK)));
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
    assert_eq!(focused(&rig.ctx), Some(Id::new("outside")));
    assert!(!within(&rig.ctx, Id::new(RACK)));
    clear(&rig.ctx);
    assert_eq!(focused(&rig.ctx), None);
}

/// Two knobs: a continuous one and a 5-step one.
const KNOBS: [Rect; 2] = [
    Rect::from_min_max(pos2(10.0, 10.0), pos2(106.0, 138.0)),
    Rect::from_min_max(pos2(120.0, 10.0), pos2(216.0, 138.0)),
];

#[test]
fn increment_and_decrement_step_the_focused_knob_0_05_or_0_0125_fine_and_an_n_step_one_a_step() {
    let mut rig = Rig::new(1.0);
    let mut values = [0.5_f32, 0.5];
    let mut changed = [0; 2];
    let disabled = std::cell::Cell::new(false);
    {
        let mut draw = |ui: &mut Ui, cx: &UiCtx| {
            for i in 0..2 {
                let c = Control {
                    steps: (i == 1).then_some(5),
                    disabled: i == 1 && disabled.get(),
                    ..Control::new(Id::new(("knob", i)), KNOBS[i], "Knob", Color32::RED, 0.5)
                };
                if knob(ui, cx, &c, &mut values[i], |v| format!("{v}")).changed() {
                    changed[i] += 1;
                }
            }
        };
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        rig.frame(vec![], &mut draw);
        // Nothing focused: Ctrl+= is not the knobs'.
        rig.frame(tap(Key::Equals, cmd), &mut draw);
        rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
        rig.frame(tap(Key::Equals, cmd), &mut draw);
        rig.frame(tap(Key::Equals, cmd_shift), &mut draw);
        rig.frame(tap(Key::Minus, cmd), &mut draw);
        rig.frame(tap(Key::Minus, cmd), &mut draw);
        rig.frame(tap(Key::Minus, cmd_shift), &mut draw);
        rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
        // The 5-step knob: a step of 0.25 either way, fine or not, held at the ends.
        rig.frame(tap(Key::Equals, cmd_shift), &mut draw);
        rig.frame(tap(Key::Equals, cmd), &mut draw);
        rig.frame(tap(Key::Equals, cmd), &mut draw);
        // Disabled while focused: the step asked for at the frame's start is not its.
        disabled.set(true);
        rig.frame(tap(Key::Minus, cmd), &mut draw);
    }
    let near = |a: f32, b: f32| (a - b).abs() < 1e-6;
    assert!(
        near(values[0], 0.5 + 0.05 + 0.0125 - 0.1 - 0.0125),
        "{values:?}"
    );
    assert_eq!(values[1], 1.0);
    assert_eq!(changed, [5, 2]);
}

#[test]
fn a_container_focused_before_it_first_draws_keeps_the_focus_until_it_does() {
    // KA's hidden → show-and-focus: the action runs inside the frame, before the panel draws in the next one.
    let mut rig = Rig::new(1.0);
    let draw = |ui: &mut Ui, shown: bool| {
        interact(ui, BUTTONS[0], Id::new("outside"), Sense::click());
        if shown {
            container(ui, Id::new(RACK), r(0.0, 35.0, 200.0, 100.0), |ui| {
                interact(ui, BUTTONS[2], Id::new("inside"), Sense::click());
            });
        }
    };
    rig.frame(vec![], &mut |ui, _| draw(ui, false));
    rig.frame(vec![], &mut |ui, _| {
        draw(ui, false);
        focus(ui.ctx(), Id::new(RACK));
    });
    assert_eq!(focused(&rig.ctx), Some(Id::new(RACK)));
    rig.frame(vec![], &mut |ui, _| draw(ui, true));
    assert_eq!(focused(&rig.ctx), Some(Id::new("inside")));
    // Still not drawn a pass later: the root.
    rig.frame(vec![], &mut |ui, _| {
        draw(ui, false);
        focus(ui.ctx(), Id::new(RACK));
    });
    rig.frame(vec![], &mut |ui, _| draw(ui, false));
    assert_eq!(focused(&rig.ctx), None);
}

#[test]
fn an_auto_repeat_moves_nothing_steps_nothing_and_presses_nothing() {
    // A press of a key already held (egui marks it a repeat) is ignored, and spent.
    let mut rig = Rig::new(1.0);
    let mut clicks = [0; 3];
    let mut draw = buttons(&mut clicks);
    rig.frame(vec![], &mut draw);
    rig.frame(vec![key(Key::Tab, Modifiers::NONE)], &mut draw);
    rig.frame(vec![key(Key::Tab, Modifiers::NONE)], &mut draw);
    assert_eq!(focused(&rig.ctx), Some(Id::new(("button", 0))));
    // Let go, a press moves again.
    rig.frame(vec![up(Key::Tab, Modifiers::NONE)], &mut draw);
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
    assert_eq!(focused(&rig.ctx), Some(Id::new(("button", 1))));
    rig.frame(vec![key(Key::Enter, Modifiers::NONE)], &mut draw);
    rig.frame(vec![key(Key::Enter, Modifiers::NONE)], &mut draw);
    drop(draw);
    assert_eq!(clicks, [0, 1, 0]);
    let mut value = 0.5_f32;
    let mut draw = |ui: &mut Ui, cx: &UiCtx| {
        let c = Control::new(Id::new("k"), KNOBS[0], "Knob", Color32::RED, 0.5);
        knob(ui, cx, &c, &mut value, |v| format!("{v}"));
    };
    rig.frame(vec![], &mut draw);
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
    rig.frame(vec![key(Key::Equals, Modifiers::COMMAND)], &mut draw);
    // Held: spent, so nothing behind the focused control sees it either.
    let mut seen = false;
    rig.frame(vec![key(Key::Equals, Modifiers::COMMAND)], &mut |ui, cx| {
        draw(ui, cx);
        seen = ui.input(|i| !i.events.is_empty());
    });
    assert!(!seen);
    assert!((value - 0.55).abs() < 1e-6, "{value}");
}

#[test]
fn a_text_field_with_the_keyboard_keeps_the_arrows_and_enter() {
    let mut rig = Rig::new(1.0);
    let mut text = String::from("abc");
    let field = Id::new("field");
    let mut clicks = [0; 3];
    {
        let mut draw = |ui: &mut Ui, cx: &UiCtx| {
            buttons(&mut clicks)(ui, cx);
            let edit = eframe::egui::TextEdit::singleline(&mut text).id(field);
            ui.put(r(300.0, 10.0, 100.0, 20.0), edit);
        };
        rig.frame(vec![], &mut draw);
        rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
        assert_eq!(focused(&rig.ctx), Some(Id::new(("button", 0))));
        rig.ctx.memory_mut(|m| m.request_focus(field));
        rig.frame(vec![], &mut draw);
        rig.frame(tap(Key::ArrowRight, Modifiers::NONE), &mut draw);
        assert_eq!(focused(&rig.ctx), Some(Id::new(("button", 0))));
        assert_eq!(rig.ctx.memory(|m| m.focused()), Some(field));
        rig.frame(tap(Key::Enter, Modifiers::NONE), &mut draw);
    }
    assert_eq!(clicks, [0, 0, 0]);
}

#[test]
fn a_modal_without_targets_keeps_the_keys_from_the_control_under_it() {
    let mut rig = Rig::new(1.0);
    let mut value = 0.5_f32;
    let modal_on = std::cell::Cell::new(false);
    {
        let mut draw = |ui: &mut Ui, cx: &UiCtx| {
            let c = Control::new(Id::new("k"), KNOBS[0], "Knob", Color32::RED, 0.5);
            knob(ui, cx, &c, &mut value, |v| format!("{v}"));
            if modal_on.get() {
                modal(ui.ctx(), Order::Background);
            }
        };
        rig.frame(vec![], &mut draw);
        rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
        modal_on.set(true);
        rig.frame(vec![], &mut draw);
        rig.frame(tap(Key::Equals, Modifiers::COMMAND), &mut draw);
    }
    assert_eq!(value, 0.5);
}

#[test]
fn a_focused_control_scrolled_out_of_its_clip_shows_no_cursor() {
    let mut rig = Rig::new(1.0);
    let mut draw = |ui: &mut Ui, _: &UiCtx| {
        ui.set_clip_rect(r(0.0, 200.0, 800.0, 400.0));
        interact(ui, BUTTONS[0], Id::new("scrolled"), Sense::click());
    };
    rig.frame(vec![], &mut draw);
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
    assert_eq!(focused(&rig.ctx), Some(Id::new("scrolled")));
    assert!(is_keyboard_mode(&rig.ctx));
    assert_eq!(rig.cursor(), None);
}

#[test]
fn the_cursor_is_painted_after_everything_else() {
    let mut rig = Rig::new(1.0);
    let mut clicks = [0; 3];
    let mut draw = |ui: &mut Ui, cx: &UiCtx| {
        buttons(&mut clicks)(ui, cx);
        // Painted after the buttons, in a layer above the screens.
        let layer = LayerId::new(Order::Foreground, Id::new("popup"));
        ui.ctx()
            .layer_painter(layer)
            .rect_filled(r(0.0, 0.0, 200.0, 200.0), 0.0, Color32::RED);
    };
    rig.frame(vec![], &mut draw);
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
    let widths: Vec<f32> = (rig.out.shapes.iter().rev().take(2))
        .map(|s| match &s.shape {
            Shape::Rect(rect) => rect.stroke.width,
            _ => 0.0,
        })
        .collect();
    assert_eq!(widths, [3.0, 9.0]);
}

#[test]
fn enter_on_a_focused_knob_stays_in_the_input_for_whatever_offers_open() {
    let mut rig = Rig::new(1.0);
    let mut value = 0.5_f32;
    let mut enter = false;
    {
        let mut draw = |ui: &mut Ui, cx: &UiCtx| {
            let c = Control::new(Id::new("k"), KNOBS[0], "Knob", Color32::RED, 0.5);
            knob(ui, cx, &c, &mut value, |v| format!("{v}"));
            enter = ui.input(|i| i.key_pressed(Key::Enter));
        };
        rig.frame(vec![], &mut draw);
        rig.frame(tap(Key::Tab, Modifiers::NONE), &mut draw);
        rig.frame(tap(Key::Enter, Modifiers::NONE), &mut draw);
    }
    assert!(enter);
}

/// `opener`, under an outer modal with two items, under an inner modal with one.
fn nested(outer: bool, inner: bool) -> impl FnMut(&mut Ui, &UiCtx) {
    move |ui, _| {
        interact(ui, BUTTONS[0], Id::new("opener"), Sense::click());
        if outer {
            modal(ui.ctx(), Order::Background);
            interact(ui, BUTTONS[1], Id::new("outer 1"), Sense::click());
            interact(ui, BUTTONS[2], Id::new("outer 2"), Sense::click());
        }
        if inner {
            modal(ui.ctx(), Order::Background);
            interact(
                ui,
                r(60.0, 40.0, 40.0, 20.0),
                Id::new("inner"),
                Sense::click(),
            );
        }
    }
}

#[test]
fn an_inner_modal_closing_leaves_the_focus_in_the_outer_one() {
    // The inner modal entered over the outer one gives the focus back to its opener in the outer one.
    let mut rig = Rig::new(1.0);
    rig.frame(vec![], &mut nested(false, false));
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut nested(false, false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("opener")));
    rig.frame(vec![], &mut nested(true, false));
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut nested(true, false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("outer 2")));
    enter(&rig.ctx, Id::new("inner"));
    rig.frame(vec![], &mut nested(true, true));
    assert_eq!(focused(&rig.ctx), Some(Id::new("inner")));
    rig.frame(vec![], &mut nested(true, false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("outer 2")));
    // With no opener of its own (nothing was focused), the way back is the outer one's (the opener, under it): its
    // closing falls back to the outer modal's first target, not to the root.
    clear(&rig.ctx);
    enter(&rig.ctx, Id::new("inner"));
    rig.frame(vec![], &mut nested(true, true));
    assert_eq!(focused(&rig.ctx), Some(Id::new("inner")));
    rig.frame(vec![], &mut nested(true, false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("outer 1")));
}

#[test]
fn the_focus_goes_back_to_the_opener_after_a_popup_opened_and_closed_in_a_modal() {
    // F8 from a focused playlist control, Sort opened and closed in SONGS, then Escape: the focus is back on the
    // control, not at the root. Each way back is kept: a modal entered or taking the focus pushes one, each closing
    // pops one.
    let mut rig = Rig::new(1.0);
    rig.frame(vec![], &mut nested(false, false));
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut nested(false, false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("opener")));
    enter(&rig.ctx, Id::new("outer 2"));
    rig.frame(vec![], &mut nested(true, false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("outer 2")));
    // The popup takes the focus as it opens and gives it back as it closes.
    rig.frame(vec![], &mut nested(true, true));
    assert_eq!(focused(&rig.ctx), Some(Id::new("inner")));
    rig.frame(vec![], &mut nested(true, false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("outer 2")));
    rig.frame(vec![], &mut nested(false, false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("opener")));
}

/// `opener`, under a modal with `outer 1` and, while `second`, `outer 2`.
fn outer(second: bool) -> impl FnMut(&mut Ui, &UiCtx) {
    move |ui, _| {
        interact(ui, BUTTONS[0], Id::new("opener"), Sense::click());
        modal(ui.ctx(), Order::Background);
        interact(ui, BUTTONS[1], Id::new("outer 1"), Sense::click());
        if second {
            interact(ui, BUTTONS[2], Id::new("outer 2"), Sense::click());
        }
    }
}

#[test]
fn a_focused_row_vanishing_inside_a_modal_keeps_the_way_back_to_its_opener() {
    // F8 from control P, the focused SONGS row disappears, then the menu closes: the focus is back on P. Meanwhile it
    // is on the modal's first target.
    let mut rig = Rig::new(1.0);
    rig.frame(vec![], &mut nested(false, false));
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut nested(false, false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("opener")));
    enter(&rig.ctx, Id::new("outer 2"));
    rig.frame(vec![], &mut outer(true));
    assert_eq!(focused(&rig.ctx), Some(Id::new("outer 2")));
    rig.frame(vec![], &mut outer(false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("outer 1")));
    assert_eq!(ways_back(&rig.ctx), 1);
    rig.frame(vec![], &mut nested(false, false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("opener")));
    assert_eq!(ways_back(&rig.ctx), 0);
}

#[test]
fn an_inner_and_an_outer_modal_closing_at_once_give_the_focus_back_to_the_opener() {
    let mut rig = Rig::new(1.0);
    rig.frame(vec![], &mut nested(false, false));
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut nested(false, false));
    enter(&rig.ctx, Id::new("outer 2"));
    rig.frame(vec![], &mut nested(true, false));
    rig.frame(vec![], &mut nested(true, true));
    assert_eq!(focused(&rig.ctx), Some(Id::new("inner")));
    assert_eq!(ways_back(&rig.ctx), 2);
    rig.frame(vec![], &mut nested(false, false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("opener")));
    assert_eq!(ways_back(&rig.ctx), 0);
}

#[test]
fn a_touch_forgets_every_way_back() {
    let mut rig = Rig::new(1.0);
    rig.frame(vec![], &mut nested(false, false));
    rig.frame(tap(Key::Tab, Modifiers::NONE), &mut nested(false, false));
    enter(&rig.ctx, Id::new("outer 2"));
    rig.frame(vec![], &mut nested(true, false));
    assert_eq!(ways_back(&rig.ctx), 1);
    rig.frame(press(BUTTONS[1].center()), &mut nested(true, false));
    assert_eq!(focused(&rig.ctx), Some(Id::new("outer 1")));
    assert_eq!(ways_back(&rig.ctx), 0);
    // The modal closing then leaves the focus at the root.
    rig.frame(vec![], &mut nested(false, false));
    assert_eq!(focused(&rig.ctx), None);
}

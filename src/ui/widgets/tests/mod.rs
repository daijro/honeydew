use eframe::egui::{self, Context, Event, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

use super::spinner::popup_id;
use super::*;
use crate::icons::IconId;
use crate::metrics::DEFAULT_UI_SCALE;
use crate::theme::theme_by_id;

mod button;
mod kit_a;
mod list;
mod spinner;

/// `U` = 32 points at ppp 1.
fn metrics() -> Metrics {
    Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

// Offscreen egui: the widgets in a real context, driven by synthetic pointer events.

#[derive(Clone, Copy, Debug)]
enum Kind {
    Knob,
    VSlider,
    HSlider,
    Toggle,
    Spinner,
    SpinnerIndex,
    List(Bar),
    ListIndex(Bar),
    Button,
}

const ITEMS: [&str; 12] = [
    "Sine", "Saw", "Square", "Triangle", "Noise", "Ramp", "Pulse", "Random", "Step", "Slew",
    "Hold", "Walk",
];

const SCREEN: Rect = Rect {
    min: pos2(0.0, 0.0),
    max: pos2(800.0, 600.0),
};
/// The control: 3 × 4 U at U = 32.
const RECT: Rect = Rect {
    min: pos2(100.0, 100.0),
    max: pos2(196.0, 228.0),
};
const U: f32 = 32.0;

struct Rig {
    ctx: Context,
    kind: Kind,
    value: f32,
    /// The control's default; 0.5 (bipolar) unless a test sets another.
    default: f32,
    steps: Option<usize>,
    disabled: bool,
    /// Kit A: the label and the value strip.
    label: &'static str,
    value_text: bool,
    modifiers: Modifiers,
    time: f64,
    changed: bool,
    clicked: bool,
    /// The control's [`Signal`] of the last frame.
    signal: Signal,
    /// The control is bound to no parameter ([`ParamMenu::Unbound`]).
    unbound: bool,
    /// Bound, its parameter has a MIDI CC link ([`ParamMenu::Linked`]).
    linked: bool,
    /// The fine tune dialog supports its parameter ([`Control::fine_tune`]).
    fine_tune: bool,
    /// Spinners and lists: the items, the header indices, and the index of the unbound forms.
    items: &'static [&'static str],
    headers: &'static [usize],
    index: usize,
    /// Spinners: items per popup column, and the items that cannot be picked.
    column_len: Option<usize>,
    disabled_items: &'static [usize],
    /// Buttons: the state, and how the test restyles the default button.
    on: bool,
    style: for<'a> fn(Button<'a>) -> Button<'a>,
    /// The control is not drawn (an undo hid it).
    hidden: bool,
    /// What the last frame painted.
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Rig {
    fn new(kind: Kind, value: f32) -> Self {
        let mut rig = Self {
            ctx: Context::default(),
            kind,
            value,
            default: 0.5,
            steps: None,
            disabled: false,
            label: "Label",
            value_text: false,
            modifiers: Modifiers::NONE,
            time: 0.0,
            changed: false,
            clicked: false,
            signal: Signal::default(),
            unbound: false,
            linked: false,
            fine_tune: false,
            items: &ITEMS[..5],
            headers: &[],
            index: 0,
            column_len: None,
            disabled_items: &[],
            on: false,
            style: |b| b,
            hidden: false,
            shapes: Vec::new(),
        };
        rig.frame(vec![]);
        rig
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(self.time),
            modifiers: self.modifiers,
            events,
            ..Default::default()
        };
        let (theme, m) = (theme_by_id(0), metrics());
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let c = Control {
            steps: self.steps,
            disabled: self.disabled,
            value_text: self.value_text,
            column_len: self.column_len,
            disabled_items: self.disabled_items,
            menu: if self.unbound {
                ParamMenu::Unbound
            } else {
                ParamMenu::bound(self.linked)
            },
            fine_tune: self.fine_tune,
            ..Control::new(Id::new("w"), RECT, self.label, Color32::RED, self.default)
        };
        let (kind, value, index) = (self.kind, &mut self.value, &mut self.index);
        let (items, headers, on, style) = (self.items, self.headers, self.on, self.style);
        let hidden = self.hidden;
        let (mut changed, mut clicked, mut sig) = (false, false, Signal::default());
        let output = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if hidden {
                    return;
                }
                let display = |v: f32| format!("{v:.2}");
                let list = |bar| ItemList {
                    headers,
                    disabled: c.disabled,
                    ..ItemList::new(c.id, c.rect, items, bar, c.channel)
                };
                let response = match kind {
                    Kind::Knob => knob(ui, &cx, &c, value, display),
                    Kind::VSlider => v_slider(ui, &cx, &c, value, display),
                    Kind::HSlider => h_slider(ui, &cx, &c, value, display),
                    Kind::Toggle => toggle(ui, &cx, &c, value, display),
                    Kind::Spinner => spinner(ui, &cx, &c, items, value),
                    Kind::SpinnerIndex => spinner_index(ui, &cx, &c, items, index),
                    Kind::List(bar) => item_list(ui, &cx, &list(bar), value),
                    Kind::ListIndex(bar) => item_list_index(ui, &cx, &list(bar), index),
                    Kind::Button => {
                        let b = Button {
                            disabled: c.disabled,
                            ..Button::new(c.id, c.rect, "Mono", true, &theme.colors)
                        };
                        button(ui, &cx, &style(b), on)
                    }
                };
                (changed, clicked) = (response.changed(), response.clicked());
                sig = signal(ui, c.id);
            });
        });
        (self.changed, self.clicked, self.signal) = (changed, clicked, sig);
        self.shapes = output.shapes;
    }

    fn button(&mut self, pos: Pos2, button: PointerButton, pressed: bool) {
        self.frame(vec![Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: self.modifiers,
        }]);
    }

    fn click_with(&mut self, pos: Pos2, button: PointerButton) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.button(pos, button, true);
        self.button(pos, button, false);
    }

    fn click(&mut self, pos: Pos2) {
        self.click_with(pos, PointerButton::Primary);
    }

    /// Presses at the control's centre, moves through `offsets` (from the press point), releases.
    fn drag(&mut self, offsets: &[Vec2]) {
        self.drag_with(PointerButton::Primary, offsets);
    }

    /// [`Rig::drag`] with any button. A separate gesture: it starts a double-tap window after the last.
    fn drag_with(&mut self, button: PointerButton, offsets: &[Vec2]) {
        self.time += DOUBLE_TAP_S;
        let start = RECT.center();
        self.frame(vec![Event::PointerMoved(start)]);
        self.button(start, button, true);
        for &d in offsets {
            self.frame(vec![Event::PointerMoved(start + d)]);
        }
        let end = start + offsets.last().copied().unwrap_or_default();
        self.button(end, button, false);
    }

    /// The id of the control's popup: the spinner's item popup, or the parameter menu of the other widgets.
    fn menu_id(&self) -> Id {
        let id = Id::new("w");
        match self.kind {
            Kind::Spinner | Kind::SpinnerIndex => popup_id(id),
            _ => menu_id(id),
        }
    }

    fn menu_rect(&self) -> Option<Rect> {
        self.ctx.memory(|m| m.area_rect(self.menu_id()))
    }
}

// Widget kit B: spinners, item lists, buttons and the static widgets.

/// The text shapes of a frame: text, colour and the clip rect it was painted under.
fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Color32, Rect)> {
    shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => {
                Some((t.galley.text().to_owned(), t.fallback_color, c.clip_rect))
            }
            _ => None,
        })
        .collect()
}

/// A text shape as drawn: text, the rect its galley covers, font size, whether it was elided, and its clip rect.
type TextBox = (String, Rect, f32, bool, Rect);

fn text_boxes(shapes: &[egui::epaint::ClippedShape]) -> Vec<TextBox> {
    shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((
                t.galley.text().to_owned(),
                Rect::from_min_size(t.pos, t.galley.size()),
                t.galley.job.sections[0].format.font_id.size,
                t.galley.elided,
                c.clip_rect,
            )),
            _ => None,
        })
        .collect()
}

/// Plain text drawing: `b` is drawn in full at `size`, centred on `at`, and not clipped.
fn assert_plain(b: &TextBox, size: f32, at: Pos2) {
    let (text, rect, font, elided, clip) = b;
    assert!(!elided, "{text}: elided");
    assert_eq!(*font, size, "{text}: shrunk");
    assert!(
        clip.contains_rect(*rect),
        "{text}: {rect:?} clipped to {clip:?}"
    );
    assert!(
        (rect.center() - at).length() < 1e-3,
        "{text}: {rect:?} not centred on {at:?}"
    );
}

/// Filled rects inside the control (so not the panel's background): fill and rect.
fn rects(shapes: &[egui::epaint::ClippedShape]) -> Vec<(Color32, Rect)> {
    shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) if RECT.expand(4.0).contains_rect(r.rect) => {
                Some((r.fill, r.rect))
            }
            _ => None,
        })
        .collect()
}

/// Filled circles: fill, centre and radius.
fn circles(shapes: &[egui::epaint::ClippedShape]) -> Vec<(Color32, Pos2, f32)> {
    shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Circle(c) => Some((c.fill, c.center, c.radius)),
            _ => None,
        })
        .collect()
}

/// The bounding rects of the non-text shapes that fill or stroke with `ink` (the icon, when no other shape uses
/// that colour).
fn painted_in(shapes: &[egui::epaint::ClippedShape], ink: Color32) -> Vec<Rect> {
    shapes
        .iter()
        .filter(|c| inks(&c.shape).contains(&ink))
        .map(|c| c.shape.visual_bounding_rect())
        .collect()
}

/// The fill and stroke colours of a shape (none for text, which [`texts`] reads).
fn inks(shape: &egui::Shape) -> Vec<Color32> {
    use egui::epaint::ColorMode;
    let solid = |mode: &ColorMode| match mode {
        ColorMode::Solid(c) => *c,
        ColorMode::UV(_) => Color32::PLACEHOLDER,
    };
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter().flat_map(inks).collect(),
        egui::Shape::Rect(r) => vec![r.fill, r.stroke.color],
        egui::Shape::Circle(c) => vec![c.fill, c.stroke.color],
        egui::Shape::Ellipse(e) => vec![e.fill, e.stroke.color],
        egui::Shape::Path(p) => vec![p.fill, solid(&p.stroke.color)],
        egui::Shape::LineSegment { stroke, .. } => vec![stroke.color],
        egui::Shape::Mesh(m) => m.vertices.iter().map(|v| v.color).collect(),
        _ => Vec::new(),
    }
}

/// Where the spinner's `^` and `v` are tapped, and its value text: the control is 96 × 128 at (100, 100); the
/// caption takes the top 24, so the value centre is (148, 176) and the chevron apexes 27.2 above and below.
const UP: Pos2 = pos2(148.0, 150.0);
const DOWN: Pos2 = pos2(148.0, 202.0);
const VALUE: Pos2 = pos2(148.0, 176.0);
/// A popup row is 1.25 U = 40 points high.
const ROW: f32 = 40.0;

impl Rig {
    /// Opens the popup by clicking `at` and returns its rect, once it is on screen.
    fn open_popup(&mut self, at: Pos2) -> Rect {
        self.click(at);
        self.frame(vec![]);
        self.menu_rect().expect("popup open")
    }

    /// Clicks popup row `row`.
    fn pick(&mut self, menu: Rect, row: usize) {
        self.click(pos2(
            menu.left() + 10.0,
            menu.top() + (row as f32 + 0.5) * ROW,
        ));
    }

    fn popup_is_open(&self) -> bool {
        self.ctx
            .data(|d| d.get_temp::<Anchor>(self.menu_id()))
            .is_some()
    }
}

mod input;
mod kit_c;
mod kit_d;
mod text;

#[test]
fn every_kit_widget_tells_accessibility_tools_its_role_and_name() {
    use egui::accesskit::{NodeId, Role, Toggled};
    let ctx = Context::default();
    ctx.enable_accesskit();
    let (theme, m) = (theme_by_id(0), metrics());
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let id = |i: usize| Id::new(("access", i));
    let rect = |i: usize| {
        let at = pos2((i % 6) as f32 * 120.0, (i / 6) as f32 * 140.0);
        Rect::from_min_size(at, vec2(96.0, 128.0))
    };
    let c = |i, label| Control::new(id(i), rect(i), label, Color32::RED, 0.0);
    let raw = RawInput {
        screen_rect: Some(SCREEN),
        ..Default::default()
    };
    let output = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let (mut v, mut y, mut index) = (0.25, 0.5, 1);
            let show = |v: f32| format!("{v:.2}");
            // The knob's parameter menu is open, so its items are there too.
            open_popup(ui, menu_id(id(0)), pos2(400.0, 300.0));
            knob(ui, &cx, &c(0, "Knob"), &mut v, show);
            v_slider(ui, &cx, &c(1, "VSlider"), &mut v, show);
            h_slider(ui, &cx, &c(2, "HSlider"), &mut v, show);
            fader(ui, &cx, &c(3, "Fader"), &mut v);
            graph_knob(ui, &cx, &c(4, "Graph"), &mut v, false);
            let arc = ArcSlider {
                start: 135.0,
                span: 90.0,
                mirrored: true,
                track: Color32::BLUE,
            };
            arc_slider(ui, &cx, &c(5, "Arc"), &arc, &mut v, show);
            tag_slider(ui, &cx, &c(6, "Tag"), &mut v, true, show);
            toggle(ui, &cx, &c(7, "Toggle"), &mut v, show);
            text_toggle(ui, &cx, &c(8, "TextToggle"), &mut v);
            text_button(ui, &cx, &c(9, "TextButton"));
            let b = Button::new(id(10), rect(10), "Button", true, &theme.colors);
            button(ui, &cx, &b, false);
            spinner_index(ui, &cx, &c(11, "Spinner"), &ITEMS[..3], &mut index);
            menu_button(ui, &cx, &c(12, "Menu"), &ITEMS[..3], &mut v);
            let list = ItemList::new(id(13), rect(13), &ITEMS[..3], Bar::Bottom, Color32::RED);
            item_list_index(ui, &cx, &list, &mut index);
            xy_pad(ui, &cx, &c(14, "X · Y"), &mut v, &mut y);
        });
    });
    let update = output
        .platform_output
        .accesskit_update
        .expect("accesskit on");
    let node = |id: Id| {
        let found = update.nodes.iter().find(|(n, _)| *n == NodeId(id.value()));
        found.map(|(_, node)| node).expect("a node")
    };
    let expected = [
        (Role::Slider, "Knob"),
        (Role::Slider, "VSlider"),
        (Role::Slider, "HSlider"),
        (Role::Slider, "Fader"),
        (Role::Slider, "Graph"),
        (Role::Slider, "Arc"),
        (Role::Slider, "Tag"),
        (Role::CheckBox, "Toggle"),
        (Role::CheckBox, "TextToggle"),
        (Role::Button, "TextButton"),
        (Role::Button, "Button"),
        (Role::ComboBox, "Spinner"),
        (Role::ComboBox, "Menu"),
        // A list has no caption: it is named by its selected item.
        (Role::RadioGroup, "Saw"),
        (Role::Slider, "X · Y"),
    ];
    for (i, (role, label)) in expected.into_iter().enumerate() {
        let n = node(id(i));
        assert_eq!((n.role(), n.label()), (role, Some(label)), "{i}");
    }
    // Values: the slider's number, the toggle's state (0.25 is on: any value but 0), the spinner's item.
    assert_eq!(node(id(0)).numeric_value(), Some(0.25));
    assert_eq!(node(id(7)).toggled(), Some(Toggled::True));
    assert_eq!(node(id(11)).value(), Some("Saw"));
    // The open menu's items are buttons; the disabled ones take no input and are not listed.
    let reset = update
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Reset"))
        .expect("Reset");
    assert_eq!(reset.1.role(), Role::Button);
}
// The long press with a mouse.
mod long_press;

use eframe::egui::{self, Context, Event, PointerButton, Pos2, RawInput, pos2, vec2};

use super::build::*;
use super::*;
use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
use crate::midi::CcLink;
use crate::params::{ModuleSpec, menu_value, module_spec};
use crate::theme::theme_by_id;
use crate::ui::ctrl_menu::CtrlAction;

mod api;
mod kits;
mod reference;
mod spec_labels;

/// The reference scale: `U = 100` px.
const U_PX: f32 = 100.0;

/// Places `panel` for a fresh `type_id` module (or `module`) in a `w × h` U content rect at the reference `U`.
fn placed(
    panel: &Panel,
    module: &ModuleInstance,
    tab: usize,
    w: f32,
    h: f32,
) -> Vec<(u32, [f32; 4])> {
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(w * U_PX, h * U_PX));
    place(panel, module, &on_tab(tab), rect, U_PX)
        .into_iter()
        .map(|(leaf, r)| (leaf.id, [r.min.x, r.min.y, r.width(), r.height()]))
        .collect()
}

/// A panel state with page `tab` selected.
fn on_tab(tab: usize) -> PanelState {
    PanelState {
        tab,
        ..Default::default()
    }
}

/// The registered Distortion panel.
fn distortion() -> Panel {
    panel_for(6).expect("Distortion is registered").clone()
}

/// The visible `#n` in order.
fn ids(got: &[(u32, [f32; 4])]) -> Vec<u32> {
    got.iter().map(|(id, _)| *id).collect()
}

/// `[x, y, w, h]` in U as the reference rects give them, to pixels at `U = 100`, edge by edge.
fn px(r: [f32; 4]) -> [f32; 4] {
    let [x, y, w, h] = r.map(|v| (v * U_PX).round());
    [x, y, (x + w) - x, (y + h) - y]
}

// ---- Distortion ----

#[test]
fn distortion_matches_the_reference() {
    let got = placed(&distortion(), &ModuleInstance::new(6, 2), 0, 14.0, 3.49);
    let want: [(u32, [f32; 4]); 6] = [
        (1, [0.0, 0.0, 2.33, 3.49]),
        (2, [2.33, 0.0, 2.33, 3.49]),
        (3, [4.66, 0.0, 2.34, 3.49]),
        (4, [7.0, 0.0, 2.33, 3.49]),
        (5, [9.33, 0.0, 2.33, 3.49]),
        (6, [11.66, 0.0, 2.34, 3.49]),
    ];
    assert_eq!(ids(&got), [1, 2, 3, 4, 5, 6]);
    for ((id, have), (_, r)) in got.iter().zip(want) {
        assert_eq!(*have, px(r), "#{id}");
    }
}

#[test]
fn distortion_has_the_tables_controls() {
    let panel = distortion();
    let got: Vec<_> = panel
        .root
        .leaves()
        .into_iter()
        .map(|l| (l.id, l.widget, l.label, l.param))
        .collect();
    assert_eq!(
        got,
        [
            (
                1,
                Widget::List(Bar::Right),
                "Hard · Tube · Soft · Clip",
                Some(5)
            ),
            (2, Widget::VSlider, "Bias", Some(6)),
            (3, Widget::Knob, "Drive", Some(1)),
            (4, Widget::Knob, "Stereo", Some(4)),
            (5, Widget::VSlider, "BitCrush", Some(2)),
            (6, Widget::Knob, "Gain", Some(3)),
        ]
    );
}

#[test]
fn only_registered_panels_are_found() {
    assert!(panel_for(6).is_some());
    // Type 17 is a gap of the id range, and no module has type 9999.
    assert!(panel_for(17).is_none());
    assert!(panel_for(9999).is_none());
}

/// Every predicate of `part`'s subtree (`enabled` and `lit` ones too), through `Not` and `All`.
fn predicates(part: &Part) -> Vec<Show> {
    fn of(show: &Show) -> Vec<Show> {
        match *show {
            Show::Not(s) => of(s),
            Show::All(all) => all.iter().flat_map(of).collect(),
            other => vec![other],
        }
    }
    let mut all = of(&part.show);
    match &part.kind {
        PartKind::HStack(c) | PartKind::VStack(c) | PartKind::Layer(c) => {
            all.extend(c.iter().flat_map(predicates));
        }
        PartKind::Leaf(leaf) => all.extend([&leaf.enabled, &leaf.lit].into_iter().flat_map(of)),
    }
    all
}

/// How many items a control has, as [`Show::Item`] and [`Show::Setting`] count them: a menu's, a list's or a
/// spinner's, and 2 (off and on) for anything else.
fn item_count(leaf: &Leaf, kind: Option<ParamKind>) -> usize {
    match (leaf.widget, kind) {
        (_, Some(ParamKind::Menu { items } | ParamKind::MenuBy { items, .. })) => items.len(),
        (Widget::List(_), None) => item_names(leaf.label).len(),
        (Widget::Spinner | Widget::MenuButton, None) => leaf.items.len(),
        _ => 2,
    }
}

/// Latching buttons on a continuous param, as (module, `#n`): a button reads its param as on from 0.5 and writes 0
/// or 1, which only these need. Spreader's Mono reaches p3 Mono Bass, a generic param.
const LATCHING_ON_CONTINUOUS: [(u16, u32); 1] = [(44, 5)];

/// What keeps `panel` from fitting module `spec`, one line each:
/// - the `#n` are unique;
/// - every `Tab(k)` names a page of the panel's tab strip; every `Item(p, k)` reads a param the module has and an
///   item it has, and every `Above(p, x)` a param it has; every `Setting(n, k)` reads a control without a param that
///   has a value, and an item it has;
/// - every bound param exists with a kind the widget can edit (lists show the menu's items), XY pads have both, and
///   spinners have items from their menu or their own, not both;
/// - a label that follows the params reads the control's own label at the registry defaults.
fn check(spec: &ModuleSpec, panel: &Panel) -> Vec<String> {
    use ParamKind::{Continuous, Menu, MenuBy, Toggle};
    let name = spec.display_name;
    let mut out = Vec::new();
    let leaves = panel.root.leaves();
    let mut ids: Vec<u32> = leaves.iter().map(|l| l.id).collect();
    ids.sort_unstable();
    if ids.windows(2).any(|w| w[0] == w[1]) {
        out.push(format!("{name}: #n {ids:?}"));
    }
    let kind = |p: usize| spec.params.get(p).map(|s| s.kind);
    for show in predicates(&panel.root) {
        match show {
            Show::Item(p, k) => match kind(p) {
                Some(Menu { items } | MenuBy { items, .. }) if k >= items.len() => {
                    out.push(format!("{name}: {show:?} of {} items", items.len()))
                }
                Some(Menu { .. } | MenuBy { .. }) => {}
                Some(_) if k >= 2 => out.push(format!("{name}: {show:?} of 2 items")),
                Some(_) => {}
                None => out.push(format!("{name}: {show:?}: no p{p}")),
            },
            Show::Above(p, _) if kind(p).is_none() => {
                out.push(format!("{name}: {show:?}: no p{p}"));
            }
            Show::Tab(k) => {
                let strip = leaves.iter().find(|l| l.widget == Widget::Tabs);
                let pages = strip.map_or(0, |l| item_names(l.label).len());
                if k >= pages {
                    out.push(format!("{name}: {show:?} of {pages} pages"));
                }
            }
            Show::Setting(n, k) => match leaves.iter().find(|l| l.id == n) {
                Some(l) if l.param.is_none() && l.widget.has_value() => {
                    let count = item_count(l, None);
                    if k >= count {
                        out.push(format!("{name}: {show:?} of {count} items"));
                    }
                }
                _ => out.push(format!("{name}: {show:?}: no #{n} with a setting")),
            },
            _ => {}
        }
    }
    let defaults = ModuleInstance::new(spec.id, 2).params;
    for leaf in leaves {
        let what = format!("{name} #{}", leaf.id);
        let label = leaf.label_for(&defaults);
        if label != leaf.label {
            out.push(format!(
                "{what}: label {label:?} at the defaults, not {:?}",
                leaf.label
            ));
        }
        let menu_ok = |items: &[&str]| leaf.disabled_items.iter().all(|&i| i < items.len());
        let params = [leaf.param, leaf.y_param].map(|p| p.map(|p| (p, kind(p))));
        match (leaf.widget, params) {
            (_, [Some((p, None)), _] | [_, Some((p, None))]) => {
                out.push(format!("{what}: no p{p}"))
            }
            (Widget::XyPad, [Some((_, Some(x))), Some((_, Some(y)))])
                if [x, y]
                    .iter()
                    .all(|k| matches!(k, Continuous { .. } | Menu { .. })) => {}
            (_, [_, Some(_)]) | (Widget::XyPad, _) => {
                out.push(format!("{what}: {:?} with {params:?}", leaf.widget))
            }
            (Widget::List(_), [Some((_, Some(Menu { items } | MenuBy { items, .. }))), _]) => {
                if item_names(leaf.label) != items {
                    out.push(format!(
                        "{what}: items {:?}, the menu {items:?}",
                        leaf.label
                    ));
                }
            }
            (Widget::Spinner, [Some((_, Some(Menu { items } | MenuBy { items, .. }))), _])
                if leaf.items.is_empty() && menu_ok(items) => {}
            (Widget::MenuButton, [Some((_, Some(Menu { items } | MenuBy { items, .. }))), _])
                if leaf.items.is_empty() && menu_ok(items) => {}
            (Widget::Spinner | Widget::MenuButton, [None, _])
                if !leaf.items.is_empty() && menu_ok(leaf.items) => {}
            (
                Widget::Toggle | Widget::ToggleRight | Widget::TextToggle | Widget::Button { .. },
                [Some((_, Some(Toggle))), _],
            ) => {}
            // A momentary button (Note Echo's Stop), or a latching one that opts in.
            (Widget::Button { .. }, [Some((_, Some(Continuous { .. }))), _])
                if leaf.momentary || LATCHING_ON_CONTINUOUS.contains(&(spec.id, leaf.id)) => {}
            (
                Widget::Knob
                | Widget::VSlider
                | Widget::HSlider
                | Widget::Arc { .. }
                | Widget::Tag { .. }
                | Widget::GraphKnob { .. },
                [
                    Some((_, Some(Continuous { .. } | Menu { .. } | MenuBy { .. }))),
                    _,
                ],
            ) => {}
            (
                Widget::Tabs | Widget::Text | Widget::Display | Widget::Spacer | Widget::Separator,
                [None, _],
            ) => {}
            (Widget::Spinner | Widget::MenuButton, _) => out.push(format!(
                "{what}: spinner items {:?} with {params:?}",
                leaf.items
            )),
            (widget, [None, _]) if widget.has_value() => {}
            (widget, _) => out.push(format!("{what}: {widget:?} with {params:?}")),
        }
    }
    out
}

#[test]
fn only_an_opted_in_latching_button_may_press_a_continuous_param() {
    // Comb Filter p2 (Freq) is continuous.
    let spec = module_spec(40).unwrap();
    let on_freq = |b: Part| check(spec, &Panel::new(hstack([b.p(2)])));
    assert!(on_freq(button(1, "Freq", false).momentary()).is_empty());
    assert_eq!(on_freq(button(1, "Freq", false)).len(), 1);
}

/// Every registered panel fits its module, and so do the test panels built on real modules.
#[test]
fn every_panel_fits_its_module() {
    let mut problems = Vec::new();
    for spec in crate::params::modules() {
        if let Some(panel) = panel_for(spec.id) {
            problems.extend(check(spec, panel));
        }
    }
    problems.extend(check(module_spec(3).unwrap(), &bands()));
    problems.extend(check(module_spec(40).unwrap(), &comb()));
    assert!(problems.is_empty(), "{problems:#?}");
}

// ---- visibility ----

/// An Equalizer-like panel (type 3): a tab strip, a display, and one row per band page. The second page's labels
/// differ so a render can tell the pages apart.
fn bands() -> Panel {
    let band = |k: usize| {
        let first = 3 + 2 * k as u32;
        let [gain, frequency] = [["Gain", "Frequency"], ["Gain 2", "Frequency 2"]][k];
        hstack([
            knob(first, gain).p(1 + 4 * k),
            knob(first + 1, frequency).p(2 + 4 * k),
        ])
        .show(Show::Tab(k))
    };
    Panel::new(vstack([
        vstack([tabs(1, "BAND 1 · BAND 2").h(Size::U(1.25)), display(2)]),
        band(0),
        band(1),
    ]))
}

#[test]
fn a_tab_predicate_hides_the_other_pages() {
    let eq = ModuleInstance::new(3, 2);
    let page = |tab| placed(&bands(), &eq, tab, 14.0, 7.99);
    assert_eq!(ids(&page(0)), [1, 2, 3, 4]);
    assert_eq!(ids(&page(1)), [1, 2, 5, 6]);
    // The hidden page gives its space back: both pages get the same rects (Equalizer reference).
    let rects = |got: Vec<(u32, [f32; 4])>| got.into_iter().map(|(_, r)| r).collect::<Vec<_>>();
    assert_eq!(rects(page(0)), rects(page(1)));
    assert_eq!(page(1)[2].1, px([0.0, 3.99, 7.0, 4.0]));
    // A stale tab past the last page shows no page.
    assert_eq!(ids(&page(5)), [1, 2]);
}

/// The Comb Filter's swaps (type 40): Mode p1 (`Manual` = no tracking) and Snap p7.
fn comb() -> Panel {
    const MANUAL: Show = Show::Item(1, 0);
    Panel::new(hstack([
        spinner(1, "Tracking").p(1),
        text(2, "?").show(Show::Not(&MANUAL)),
        button(3, "Snap", true).p(7).show(MANUAL),
        hstack([
            v_slider(4, "Octave").p(5).show(Show::Not(&MANUAL)),
            v_slider(5, "Octave")
                .p(10)
                .show(Show::All(&[MANUAL, Show::Item(7, 1)])),
            v_slider(6, "Note")
                .p(11)
                .show(Show::All(&[MANUAL, Show::Item(7, 1)])),
            v_slider(7, "Freq")
                .p(2)
                .show(Show::All(&[MANUAL, Show::Item(7, 0)])),
            spinner(8, "Polarity").p(8),
        ])
        .w(Size::Flex(2.5)),
    ]))
}

#[test]
fn param_predicates_follow_the_params() {
    let mut comb_filter = ModuleInstance::new(40, 2);
    let shown = |m: &ModuleInstance| ids(&placed(&comb(), m, 0, 14.0, 4.99));
    // Defaults: Manual, Snap on.
    assert_eq!(shown(&comb_filter), [1, 3, 5, 6, 8]);
    comb_filter.params[7] = 0.0;
    assert_eq!(shown(&comb_filter), [1, 3, 7, 8]);
    // Either tracking mode: the pitch text and the other Octave slider, whatever Snap says.
    for mode in 1..3 {
        comb_filter.params[1] = menu_value(mode, 3);
        assert_eq!(shown(&comb_filter), [1, 2, 4, 8], "mode {mode}");
    }
    // Menu items are read like the widgets read them: 0.2 of three items is item 0.
    comb_filter.params[1] = 0.2;
    assert_eq!(shown(&comb_filter), [1, 3, 7, 8]);
}

#[test]
fn a_toggle_is_item_1_when_on() {
    let mut comb_filter = ModuleInstance::new(40, 2);
    let snap_on = |m: &ModuleInstance| Show::Item(7, 1).holds(m, &on_tab(0));
    assert!(snap_on(&comb_filter));
    for (v, on) in [(0.0, false), (0.49, false), (0.5, true), (1.0, true)] {
        comb_filter.params[7] = v;
        assert_eq!(snap_on(&comb_filter), on, "{v}");
    }
    // A param the module does not have holds nothing.
    assert!(!Show::Item(99, 0).holds(&comb_filter, &on_tab(0)));
}

// ---- rendering, offscreen ----

/// `U` = 32 points at ppp 1.
fn metrics() -> Metrics {
    Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX)
}

pub(crate) const U: f32 = 32.0;
/// The content rect of a 14 U card at UIHeight 5.5: 14 U × ((5.5 − 2) U − 1 px), at (20, 30).
const DISTORTION: Rect = Rect {
    min: pos2(20.0, 30.0),
    max: pos2(20.0 + 14.0 * U, 30.0 + 3.5 * U - 1.0),
};

/// A panel shown in an offscreen egui context at U = 32 points (ppp 1, or the [`Rig::with_ppp`] one), one frame per
/// call, for synthetic-input tests. The module files' tests use it too.
pub(crate) struct Rig {
    pub(crate) ctx: Context,
    /// Device pixels per point.
    pub(crate) ppp: f32,
    pub(crate) panel: Panel,
    pub(crate) module: ModuleInstance,
    pub(crate) state: PanelState,
    /// The content rect (points).
    pub(crate) rect: Rect,
    /// The input time (s): each frame adds 1/60.
    pub(crate) time: f64,
    /// What the last frame returned and painted.
    pub(crate) changed: Vec<usize>,
    pub(crate) edited_settings: Vec<u32>,
    /// The last frame's [`Edits::pressed`] and [`Edits::pick`], of any pass.
    pub(crate) pressed: Option<usize>,
    pub(crate) pick: Option<(usize, CtrlAction)>,
    pub(crate) shapes: Vec<egui::epaint::ClippedShape>,
}

impl Rig {
    /// A fresh `type_id` module's `panel` in the content rect `rect`, after one frame.
    pub(crate) fn new(type_id: u16, panel: Panel, rect: Rect) -> Self {
        Self::with_ppp(type_id, panel, rect, 1.0)
    }

    /// [`Self::new`] on a screen of `ppp` device pixels per point. `rect` and all input stay in points.
    pub(crate) fn with_ppp(type_id: u16, panel: Panel, rect: Rect, ppp: f32) -> Self {
        let mut rig = Self {
            ctx: Context::default(),
            ppp,
            panel,
            module: ModuleInstance::new(type_id, 2),
            state: PanelState::default(),
            rect,
            time: 0.0,
            changed: Vec::new(),
            edited_settings: Vec::new(),
            pressed: None,
            pick: None,
            shapes: Vec::new(),
        };
        rig.frame(vec![]);
        rig
    }

    pub(crate) fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let mut raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        raw.viewports
            .entry(raw.viewport_id)
            .or_default()
            .native_pixels_per_point = Some(self.ppp);
        let m = Metrics::from_pixels_per_point(self.ppp, DEFAULT_UI_SCALE_INDEX);
        let theme = theme_by_id(0);
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let (panel, module, state, rect) =
            (&self.panel, &mut self.module, &mut self.state, self.rect);
        let mut changed = Vec::new();
        let output = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let edits = show(ui, &cx, panel, module, state, rect, Color32::RED);
                changed.push(edits);
            });
        });
        // A pass that is discarded and run again reports in both.
        self.changed = changed.iter().flat_map(|e| e.params.clone()).collect();
        self.edited_settings = changed.iter().flat_map(|e| e.settings.clone()).collect();
        self.pressed = changed.iter().find_map(|e| e.pressed);
        self.pick = changed.iter().find_map(|e| e.pick);
        self.shapes = output.shapes;
    }

    /// A primary click at `pos` over three frames: move, press, release.
    pub(crate) fn click(&mut self, pos: Pos2) {
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
    }

    /// Texts painted in the last frame, with where they were drawn.
    pub(crate) fn texts(&self) -> Vec<(String, Rect)> {
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

    /// The rect (points) of control `#id` as the renderer lays it out now.
    pub(crate) fn rect_of(&self, id: u32) -> Rect {
        let u_px = Metrics::from_pixels_per_point(self.ppp, DEFAULT_UI_SCALE_INDEX).u_px();
        let rect = content_px(self.rect, self.ppp).expect("a finite content rect");
        place(&self.panel, &self.module, &self.state, rect, u_px)
            .into_iter()
            .find(|(l, _)| l.id == id)
            .map(|(_, r)| r / self.ppp)
            .expect("placed")
    }
}

#[test]
fn the_distortion_panel_draws_each_control_in_its_rect() {
    let rig = Rig::new(6, distortion(), DISTORTION);
    let texts = rig.texts();
    let inside = |text: &str, id: u32| {
        let rect = rig.rect_of(id);
        let found = texts.iter().find(|(t, _)| t == text);
        let (_, at) = found.unwrap_or_else(|| panic!("{text} not drawn: {texts:?}"));
        assert!(
            rect.contains(at.center()),
            "{text} at {at:?}, #{id} is {rect:?}"
        );
    };
    for item in ["Hard", "Tube", "Soft", "Clip"] {
        inside(item, 1);
    }
    inside("Bias", 2);
    inside("Drive", 3);
    inside("Stereo", 4);
    inside("BitCrush", 5);
    inside("Gain", 6);
    // Knob value texts come from the registry: Drive 0.25 → "25", Stereo 0.5 → "0", Gain 1.0 → "0.0 dB".
    inside("25", 3);
    inside("0", 4);
    inside("0.0 dB", 6);
    // At U = 32 the six columns are 74.67 points wide: edges 20, 94, 169, 244, 318, 393, 468. The last one may
    // be 1 px short: 448 / 6 summed six times is 447.99997 in f32, which truncates to 467 (not a reference value).
    let xs: Vec<_> = (1..=6).map(|id| rig.rect_of(id).x_range()).collect();
    let want = [20.0, 94.0, 169.0, 244.0, 318.0, 393.0, 468.0];
    for (i, x) in xs.iter().enumerate() {
        let slack = if i == 5 { 1.0 } else { 0.0 };
        assert_eq!(x.min, want[i], "#{}", i + 1);
        assert!((x.max - want[i + 1]).abs() <= slack, "#{}: {x:?}", i + 1);
    }
    assert!(rig.changed.is_empty());
}

#[test]
fn dragging_a_knob_writes_its_param_and_reports_it() {
    let mut rig = Rig::new(6, distortion(), DISTORTION);
    let at = rig.rect_of(3).center();
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.frame(vec![Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    }]);
    rig.frame(vec![Event::PointerMoved(at - vec2(0.0, 10.0))]);
    assert_eq!(rig.changed, [1]);
    rig.frame(vec![Event::PointerMoved(at - vec2(0.0, 200.0))]);
    // Drive went from 0.25 up 200 points = a quarter of the range (0.24 per 192-point inch).
    assert!(
        (rig.module.params[1] - 0.5).abs() < 1e-5,
        "{}",
        rig.module.params[1]
    );
    assert_eq!(rig.changed, [1]);
    // Nothing else moved.
    let defaults = ModuleInstance::new(6, 2).params;
    for (p, (v, d)) in rig.module.params.iter().zip(&defaults).enumerate() {
        assert!(p == 1 || v == d, "p{p}");
    }
}

#[test]
fn the_rig_lays_out_in_points_at_ppp_2_as_at_ppp_1() {
    // U is 32 points at any ppp, so the controls sit where they do at ppp 1, within a device pixel of ppp 2.
    let (one, two) = (
        Rig::new(6, distortion(), DISTORTION),
        Rig::with_ppp(6, distortion(), DISTORTION, 2.0),
    );
    let pixel = 1.0 / two.ppp;
    for id in 1..=6 {
        let (a, b) = (one.rect_of(id), two.rect_of(id));
        assert!(
            (a.min - b.min).abs().max_elem() <= pixel,
            "#{id}: {a:?} {b:?}"
        );
        assert!(
            (a.max - b.max).abs().max_elem() <= pixel,
            "#{id}: {a:?} {b:?}"
        );
    }
    assert_eq!(two.ctx.pixels_per_point(), 2.0);
    // The labels are drawn in the rects the rig reports.
    for (text, id) in [("Bias", 2), ("Drive", 3), ("Gain", 6)] {
        let at = two.text_rect(text).center();
        assert!(two.rect_of(id).contains(at), "{text} at {at:?}");
    }
}

#[test]
fn dragging_a_knob_at_ppp_2_travels_the_same_inches_as_at_ppp_1() {
    let mut rig = Rig::with_ppp(6, distortion(), DISTORTION, 2.0);
    let at = rig.rect_of(3).center();
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.frame(vec![button(true)]);
    rig.frame(vec![Event::PointerMoved(at - vec2(0.0, 10.0))]);
    assert_eq!(rig.changed, [1]);
    rig.frame(vec![Event::PointerMoved(at - vec2(0.0, 200.0))]);
    // As at ppp 1: 200 points is a quarter of the range (0.24 per inch), whatever the pixel density.
    assert!(
        (rig.module.params[1] - 0.5).abs() < 1e-5,
        "{}",
        rig.module.params[1]
    );
    rig.frame(vec![button(false)]);
}

#[test]
fn tapping_a_button_at_ppp_2_toggles_its_param() {
    let mut rig = Rig::with_ppp(44, fx::panel(44).unwrap(), card(8.0).1, 2.0);
    let mono = rig.rect_of(5).center();
    rig.click(mono);
    assert_eq!((rig.module.params[3], &rig.changed[..]), (1.0, &[3][..]));
    rig.click(mono);
    assert_eq!(rig.module.params[3], 0.0);
}

#[test]
fn a_bound_controls_press_and_its_add_automation_pick_report_its_param() {
    let mut rig = Rig::new(6, distortion(), DISTORTION);
    let button = |pos, button, pressed| Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: Default::default(),
    };
    // Drive (#3) is param 1: its press, in that frame only.
    let at = rig.rect_of(3).center();
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.frame(vec![button(at, PointerButton::Primary, true)]);
    assert_eq!((rig.pressed, rig.pick), (Some(1), None));
    rig.frame(vec![button(at, PointerButton::Primary, false)]);
    assert_eq!(rig.pressed, None);
    // Its parameter menu's `Add automation track`.
    rig.frame(vec![button(at, PointerButton::Secondary, true)]);
    rig.frame(vec![button(at, PointerButton::Secondary, false)]);
    rig.frame(vec![]);
    let item = rig
        .texts()
        .into_iter()
        .find(|(t, _)| t == "Add automation track");
    let item = item.expect("menu open").1.center();
    rig.click(item);
    assert_eq!(
        (rig.pressed, rig.pick),
        (None, Some((1, CtrlAction::AddAutomationTrack)))
    );
    assert!(rig.changed.is_empty(), "no value changed");
    // An item list has no parameter menu and reports no press.
    let list = rig.rect_of(1).center();
    rig.frame(vec![Event::PointerMoved(list)]);
    rig.frame(vec![button(list, PointerButton::Primary, true)]);
    assert_eq!(rig.pressed, None);
}

#[test]
fn clicking_a_list_item_sets_the_menu_param() {
    let mut rig = Rig::new(6, distortion(), DISTORTION);
    let list = rig.rect_of(1);
    // Third of four rows: Soft.
    rig.click(pos2(
        list.center().x,
        list.top() + 2.5 * list.height() / 4.0,
    ));
    assert_eq!(rig.module.params[5], menu_value(2, 4));
    assert_eq!(rig.changed, [5]);
    assert_eq!(
        module_spec(6).unwrap().params[5].display_text(rig.module.params[5]),
        "Soft"
    );
}

#[test]
fn the_spreader_readouts_follow_the_knobs() {
    let mut rig = Rig::new(44, fx::panel(44).unwrap(), card(8.0).1);
    let drawn = |rig: &Rig, text: &str| rig.texts().iter().any(|(t, _)| t == text);
    for text in ["0.0", "10 Hz", "Original"] {
        assert!(drawn(&rig, text), "{text} at the defaults");
    }
    rig.module.params[1] = 0.5;
    rig.module.params[2] = 0.5;
    rig.module.params[4] = 0.25;
    rig.frame(vec![]);
    for text in ["1.0", "316 Hz", "-50 %"] {
        assert!(drawn(&rig, text), "{text}");
    }
}

#[test]
fn the_spreader_mono_button_switches_mono_bass() {
    let mut rig = Rig::new(44, fx::panel(44).unwrap(), card(8.0).1);
    let mono = rig.rect_of(5).center();
    // p3 is a generic param: the button reads it as on from 0.5 and writes 0 or 1.
    assert_eq!(module_spec(44).unwrap().params[3].display_text(0.0), "0");
    rig.click(mono);
    assert_eq!((rig.module.params[3], &rig.changed[..]), (1.0, &[3][..]));
    rig.click(mono);
    assert_eq!(rig.module.params[3], 0.0);
    rig.module.params[3] = 0.7;
    rig.click(mono);
    assert_eq!(rig.module.params[3], 0.0);
}

/// Presses on control `#id`'s centre, moves 10 points up and back, and releases: a drag with no net travel.
fn drag_there_and_back(rig: &mut Rig, id: u32) {
    let at = rig.rect_of(id).center();
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.frame(vec![button(true)]);
    rig.frame(vec![Event::PointerMoved(at - vec2(0.0, 10.0))]);
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.frame(vec![button(false)]);
}

#[test]
fn comb_filter_sliders_snap_ties_as_their_texts_read() {
    let mut rig = Rig::new(40, fx::panel(40).unwrap(), card(7.0).1);
    let spec = module_spec(40).unwrap();
    // Offset (#4, shown off Manual): round(4v − 2), halves away from zero, so 0.375 reads -1 Oct (ties to even: 0).
    rig.module.params[1] = 0.5;
    rig.module.params[5] = 0.375;
    rig.frame(vec![]);
    assert_eq!(spec.params[5].display_text(0.375), "-1 Oct");
    drag_there_and_back(&mut rig, 4);
    assert_eq!(rig.module.params[5], 0.25);
    // Pitch Note (#6, Manual with Snap on): round(11v), so 0.5/11 reads C# (ties to even: C).
    let tie = 0.5 / 11.0;
    rig.module.params[1] = 0.0;
    rig.module.params[11] = tie;
    rig.frame(vec![]);
    assert_eq!(spec.params[11].display_text(tie), "C#");
    drag_there_and_back(&mut rig, 6);
    assert_eq!(rig.module.params[11], menu_value(1, 12));
}

#[test]
fn the_wow_and_flutter_spinners_show_item_trunc_9v() {
    let mut rig = Rig::new(43, fx::panel(43).unwrap(), card(9.0).1);
    rig.module.params[7] = 1.0;
    rig.frame(vec![]);
    // Drawn and left alone, a spinner keeps its stored value: writing its item back would turn 0.2 into 1/9.
    let p = &rig.module.params;
    assert_eq!([p[2], p[5], p[9]], [0.2, 0.1, 0.8]);
    let texts: Vec<String> = rig.texts().into_iter().map(|(t, _)| t).collect();
    // The defaults 0.2, 0.1 and 0.8: round(9v) would read 3/8, 1/4 and 4 beats.
    for text in ["1/4", "1/8", "4 beats"] {
        assert!(texts.iter().any(|t| t == text), "{text} in {texts:?}");
    }
    assert!(!texts.iter().any(|t| t == "3/8"), "{texts:?}");
}

#[test]
fn a_tab_click_relays_out_on_the_same_frame() {
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 8.0 * U - 1.0));
    let mut rig = Rig::new(3, bands(), rect);
    let drawn = |rig: &Rig, label: &str| {
        rig.texts()
            .iter()
            .find(|(t, _)| t == label)
            .map(|(_, r)| *r)
    };
    let gain = drawn(&rig, "Gain").expect("page 1");
    assert!(drawn(&rig, "Gain 2").is_none());
    // The second tab's cell.
    let strip = rig.rect_of(1);
    rig.click(pos2(strip.left() + 0.75 * strip.width(), strip.center().y));
    assert_eq!(rig.state.tab, 1);
    // The release frame already shows page 2 in page 1's place, and only it.
    assert!(drawn(&rig, "Gain").is_none() && drawn(&rig, "Frequency").is_none());
    let gain_2 = drawn(&rig, "Gain 2").expect("page 2");
    assert_eq!(gain_2.center().y, gain.center().y);
    assert!(drawn(&rig, "Frequency 2").is_some());
    assert!(rig.changed.is_empty(), "a tab is not a parameter");
    // The strip's `#n` is among the pass's edited settings.
    assert_eq!(rig.edited_settings, [1]);
}

#[test]
fn a_visibility_param_relays_out_on_the_same_frame() {
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 5.0 * U - 1.0));
    let mut rig = Rig::new(40, comb(), rect);
    let has = |rig: &Rig, s: &str| rig.texts().iter().any(|(t, _)| t == s);
    assert!(has(&rig, "Note") && !has(&rig, "Freq"));
    // Snap off: Freq replaces Octave and Note in the frame that handled the click.
    let snap = rig.rect_of(3).center();
    rig.click(snap);
    assert_eq!(rig.module.params[7], 0.0);
    assert_eq!(rig.changed, [7]);
    assert!(has(&rig, "Freq") && !has(&rig, "Note"));
}

#[test]
fn a_non_finite_rect_draws_nothing_and_changes_nothing() {
    let inf = f32::INFINITY;
    for rect in [
        Rect::NAN,
        Rect::EVERYTHING,
        Rect::from_min_size(pos2(inf, 0.0), vec2(14.0 * U, 3.5 * U)),
    ] {
        let mut rig = Rig::new(6, distortion(), rect);
        for at in [pos2(50.0, 50.0), pos2(0.0, 0.0)] {
            rig.click(at);
            assert!(rig.changed.is_empty(), "{rect:?}");
        }
        assert!(rig.texts().is_empty(), "{rect:?}");
        assert_eq!(rig.module.params, ModuleInstance::new(6, 2).params);
    }
}

#[test]
fn the_content_rect_is_laid_out_from_a_whole_device_pixel() {
    let rect = Rect::from_min_size(pos2(20.4, 30.6), vec2(448.0, 111.0));
    for (ppp, origin) in [(1.0, pos2(20.0, 31.0)), (2.0, pos2(41.0, 61.0))] {
        let px = content_px(rect, ppp).unwrap();
        assert_eq!(px.min, origin);
        assert!((px.size() - rect.size() * ppp).length() < 1e-3, "{px:?}");
    }
    assert_eq!(content_px(Rect::NAN, 1.0), None);
    assert_eq!(content_px(Rect::EVERYTHING, 1.0), None);
    // Finite in points, overflowing in pixels.
    assert_eq!(
        content_px(
            Rect::from_min_size(pos2(f32::MAX, 0.0), vec2(1.0, 1.0)),
            2.0
        ),
        None
    );
}

#[test]
fn show_draws_a_fractional_rect_from_a_whole_device_pixel_at_a_fractional_ppp() {
    // A display fills its leaf, which starts at the content origin: at ppp 1.5 the rect (20.4, 30.6) is
    // (30.6, 45.9) px, drawn from (31, 46) px.
    let ppp = 1.5;
    let panel = Panel::new(hstack([display(1)]));
    let rect = Rect::from_min_size(pos2(20.4, 30.6), vec2(448.0, 111.0));
    let mut raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
        ..Default::default()
    };
    raw.viewports
        .entry(raw.viewport_id)
        .or_default()
        .native_pixels_per_point = Some(ppp);
    let theme = theme_by_id(0);
    let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let (mut module, mut state) = (ModuleInstance::new(6, 2), PanelState::default());
    let output = Context::default().run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            show(ui, &cx, &panel, &mut module, &mut state, rect, Color32::RED);
        });
    });
    let fill = theme.colors.control_fill.to_color32();
    let drawn = output
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == fill => Some(r.rect),
            _ => None,
        })
        .expect("the display is drawn");
    assert!(
        (drawn.min * ppp - pos2(31.0, 46.0)).length() < 1e-3,
        "{drawn:?}"
    );
}

// ---- plain text in the panels ----

/// The Compressor panel (ID 8).
fn compressor() -> Panel {
    Panel::new(hstack([
        hstack([
            v_slider(1, "Input gain").p(8).value_text(),
            v_slider(2, "Thresh").p(1).value_text(),
            v_slider(3, "Ratio").p(2).value_text(),
            v_slider(4, "Attack").p(3).value_text(),
            v_slider(5, "Hold").p(6).value_text(),
            v_slider(6, "Release").p(4).value_text(),
            v_slider(7, "Level").p(5).value_text(),
            toggle(8, "Auto gain").p(7).value_text(),
        ])
        .margins(0.0, 0.25, 0.0, 0.25),
        display(9).w(Size::U(3.75)).mr(0.5),
    ]))
}

/// The MultiFX panel (ID 4), the XY pad as a display.
fn multi_fx() -> Panel {
    Panel::new(vstack([
        list(
            1,
            "Delay · Reverb · LP Filter · HP Filter · Flanger · Phaser · Panner · Vox",
            Bar::Bottom,
        )
        .p(1)
        .h(Size::U(1.25)),
        hstack([
            vstack([spinner(2, "LFO Rate").p(4), spinner(3, "LFO Shape").p(6)]),
            display(4).w(Size::Flex(6.0)).mt(0.5),
            v_slider(5, "LFO Amnt").p(5),
        ]),
    ]))
}

/// The module card around a `ui_height` U panel at (20, 30), 14 U + 4 px wide, and its content rect.
pub(crate) fn card(ui_height: f32) -> (Rect, Rect) {
    let card = Rect::from_min_size(pos2(20.0, 30.0), vec2(14.0 * U + 4.0, ui_height * U));
    let content = Rect::from_min_max(
        pos2(card.left() + 2.0, card.top() + 1.5 * U + 1.0),
        pos2(card.right() - 2.0, card.bottom() - 0.5 * U),
    );
    (card, content)
}

#[test]
fn panel_labels_are_drawn_in_full_and_only_clipped_to_the_card() {
    // At 14 U the edge columns' labels are wider than the card leaves them: those overhang it and only the card
    // clips them. The others must lie inside it whole.
    let pt8 = metrics().font_id(FontSize::Pt8).size;
    for (type_id, panel, ui_height, inside, edge) in [
        (
            8,
            compressor(),
            7.5,
            &[
                "Thresh",
                "Ratio",
                "Attack",
                "Hold",
                "Release",
                "Level",
                "Auto gain",
            ][..],
            &["Input gain"][..],
        ),
        (
            4,
            multi_fx(),
            9.0,
            &[][..],
            &["LFO Rate", "LFO Shape", "LFO Amnt"][..],
        ),
    ] {
        let (card, content) = card(ui_height);
        let rig = Rig::new(type_id, panel, content);
        let shapes: Vec<_> = rig
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some((t.clone(), c.clip_rect)),
                _ => None,
            })
            .collect();
        for label in inside.iter().chain(edge) {
            let (t, _) = shapes
                .iter()
                .find(|(t, _)| t.galley.text() == *label)
                .unwrap_or_else(|| panic!("{label} not drawn"));
            let rect = Rect::from_min_size(t.pos, t.galley.size());
            assert!(!t.galley.elided, "{label}");
            assert_eq!(t.galley.job.sections[0].format.font_id.size, pt8, "{label}");
            assert_eq!(
                card.contains_rect(rect),
                inside.contains(label),
                "{label}: {rect:?} in {card:?}"
            );
        }
        // Every text is clipped to the card and nothing smaller.
        for (t, clip) in &shapes {
            assert_eq!(*clip, card, "{}", t.galley.text());
        }
    }
}

// ---- module-drawn displays ----

/// The shapes display `#leaf_id` of `type_id`'s registered panel draws ([`Leaf::draw`]) in one offscreen frame, in
/// drawing order: with `params` (empty: the registry defaults) and panel state `state`, in the rect [`show`] gives it
/// for the content rect `content` (points) at `ppp` and the default UI scale (U = 32 points), in the channel colour
/// `Color32::RED`. Only that display's shapes, overhangs included. Panics when the leaf is hidden or has no draw.
pub(crate) fn display_shapes(
    type_id: u16,
    leaf_id: u32,
    params: &[f32],
    state: &PanelState,
    content: Rect,
    ppp: f32,
) -> Vec<egui::Shape> {
    let panel = panel_for(type_id).expect("a registered panel");
    display_shapes_in(panel, type_id, leaf_id, params, state, content, ppp)
}

/// [`display_shapes`] for any panel, shown for a `type_id` module.
pub(crate) fn display_shapes_in(
    panel: &Panel,
    type_id: u16,
    leaf_id: u32,
    params: &[f32],
    state: &PanelState,
    content: Rect,
    ppp: f32,
) -> Vec<egui::Shape> {
    display_shapes_with(panel, type_id, leaf_id, params, state, content, ppp, None)
}

/// [`display_shapes`] with the module's meter and analysis posted as the rack posts them.
pub(crate) fn display_shapes_fed(
    type_id: u16,
    leaf_id: u32,
    params: &[f32],
    state: &PanelState,
    content: Rect,
    ppp: f32,
    feed: (honeydew_audio::MeterSnapshot, &[f32]),
) -> Vec<egui::Shape> {
    let panel = panel_for(type_id).expect("a registered panel");
    display_shapes_with(
        panel,
        type_id,
        leaf_id,
        params,
        state,
        content,
        ppp,
        Some(feed),
    )
}

#[allow(clippy::too_many_arguments)]
fn display_shapes_with(
    panel: &Panel,
    type_id: u16,
    leaf_id: u32,
    params: &[f32],
    state: &PanelState,
    content: Rect,
    ppp: f32,
    feed: Option<(honeydew_audio::MeterSnapshot, &[f32])>,
) -> Vec<egui::Shape> {
    let mut module = ModuleInstance::new(type_id, 2);
    if !params.is_empty() {
        module.params = params.to_vec();
    }
    let mut state = state.clone();
    state.seed(panel);
    let (theme, m) = (
        theme_by_id(0),
        Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX),
    );
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let px = content_px(content, ppp).expect("a finite content rect");
    let placed = place(panel, &module, &state, px, m.u_px());
    let (leaf, rect) = (placed.into_iter().find(|(l, _)| l.id == leaf_id))
        .unwrap_or_else(|| panic!("#{leaf_id} is not shown"));
    let draw = leaf
        .draw
        .unwrap_or_else(|| panic!("#{leaf_id} has no draw"));
    let mut raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
        ..Default::default()
    };
    raw.viewports
        .entry(raw.viewport_id)
        .or_default()
        .native_pixels_per_point = Some(ppp);
    // A bare `Ui`: a panel would paint its frame too.
    let output = Context::default().run(raw, |ctx| {
        let mut ui = egui::Ui::new(ctx.clone(), egui::Id::new("display"), Default::default());
        if feed.is_some() {
            crate::panels::post_feed(&ui, module.instance_id, feed);
        }
        let id = ui
            .id()
            .with(("module panel", module.instance_id))
            .with(leaf_id);
        draw(
            &mut ui,
            &cx,
            id,
            rect / ppp,
            &mut module,
            &mut state,
            Color32::RED,
        );
    });
    output.shapes.into_iter().map(|c| c.shape).collect()
}

#[test]
fn a_param_with_a_lent_cc_link_can_be_unlinked_and_the_others_cannot() {
    let mut rig = Rig::new(6, distortion(), DISTORTION);
    // The rack lends its channel's links: CC 20 drives this Distortion's (instance 2) Drive, param 1.
    let drive = CcLink {
        cc: 20,
        instance_id: 2,
        param: 1,
    };
    // Another module's Gain (param 3) does not count for this one's.
    let other = CcLink {
        instance_id: 3,
        param: 3,
        ..drive
    };
    set_cc_links(&rig.ctx, vec![drive, other]);
    let unlink = |rig: &mut Rig, id: u32| {
        let at = rig.rect_of(id).center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        };
        rig.frame(vec![Event::PointerMoved(at)]);
        rig.frame(vec![button(true)]);
        rig.frame(vec![button(false)]);
        rig.frame(vec![]);
        let item = rig
            .texts()
            .into_iter()
            .find(|(t, _)| t == "Unlink from MIDI CC");
        rig.click(item.expect("menu open").1.center());
        let pick = rig.pick;
        rig.frame(vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        pick
    };
    // Drive (#3, param 1) is linked: its `Unlink from MIDI CC` is picked.
    assert_eq!(unlink(&mut rig, 3), Some((1, CtrlAction::UnlinkMidiCc)));
    // Gain (#6, param 3) is not: the item is dimmed and does nothing.
    assert_eq!(unlink(&mut rig, 6), None);
}

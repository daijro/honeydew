//! The builders of panel data, one call per line of a panel's tree, and the names a module file needs. Module files
//! start with `use crate::panels::build::*;`, which brings in no generic names from [`crate::panels`] (its `show`).
//!
//! ```
//! # use honeydew::panels::build::*;
//! let distortion = Panel::new(hstack([
//!     list(1, "Hard · Tube · Soft · Clip", Bar::Right).p(5),
//!     v_slider(2, "Bias").p(6),
//!     knob(3, "Drive").p(1),
//! ]));
//! let page = hstack([knob(3, "Gain").p(1), knob(4, "Frequency").p(2)]).show(Show::Tab(0));
//! let rate = knob(2, "LFO Speed").p(3).h(Size::Flex(2.0)).mt(0.5).show(Show::Item(2, 0));
//! ```

pub use super::{
    ButtonColors, Draw, Hook, Ink, Leaf, Overlay, Pad, Panel, PanelState, Part, PartKind, Show,
    Widget, flag,
};
pub use crate::icons::IconId;
pub use crate::metrics::FontSize;
pub use crate::song::ModuleInstance;
pub use crate::ui::layout::Size;
pub use crate::ui::widgets::Bar;
pub use eframe::egui::Align2;

use crate::ui::layout::{LayoutRecord, Margins};

fn part(kind: PartKind) -> Part {
    Part {
        kind,
        record: LayoutRecord::default(),
        show: Show::Always,
    }
}

fn control(id: u32, widget: Widget, label: &'static str) -> Part {
    part(PartKind::Leaf(Leaf {
        id,
        widget,
        label,
        param: None,
        y_param: None,
        value_text: false,
        font: None,
        align: Align2::LEFT_CENTER,
        ink: Ink::Channel,
        label_fn: None,
        value_fn: None,
        draw: None,
        items: &[],
        column_len: None,
        disabled_items: &[],
        default: 0.0,
        colors: None,
        shape: 1.0,
        icon: None,
        momentary: false,
        picker: false,
        halo: None,
        halo_ink: None,
        enabled: Show::Always,
        lit: Show::Always,
    }))
}

pub fn hstack(children: impl IntoIterator<Item = Part>) -> Part {
    part(PartKind::HStack(children.into_iter().collect()))
}

pub fn vstack(children: impl IntoIterator<Item = Part>) -> Part {
    part(PartKind::VStack(children.into_iter().collect()))
}

pub fn layer(children: impl IntoIterator<Item = Part>) -> Part {
    part(PartKind::Layer(children.into_iter().collect()))
}

pub fn knob(id: u32, label: &'static str) -> Part {
    control(id, Widget::Knob, label)
}

pub fn v_slider(id: u32, label: &'static str) -> Part {
    control(id, Widget::VSlider, label)
}

pub fn h_slider(id: u32, label: &'static str) -> Part {
    control(id, Widget::HSlider, label)
}

pub fn toggle(id: u32, label: &'static str) -> Part {
    control(id, Widget::Toggle, label)
}

pub fn spinner(id: u32, label: &'static str) -> Part {
    control(id, Widget::Spinner, label)
}

/// `items` separated by ` · `: `"Hard · Tube · Soft · Clip"`. `Bar::Boxed` is the boxed style.
pub fn list(id: u32, items: &'static str, bar: Bar) -> Part {
    control(id, Widget::List(bar), items)
}

/// `tabs` separated by ` · `: `"BAND 1 · BAND 2 · BAND 3 · BAND 4"`.
pub fn tabs(id: u32, tabs: &'static str) -> Part {
    control(id, Widget::Tabs, tabs)
}

pub fn button(id: u32, label: &'static str, round: bool) -> Part {
    control(id, Widget::Button { round }, label)
}

pub fn text(id: u32, label: &'static str) -> Part {
    control(id, Widget::Text, label)
}

pub fn display(id: u32) -> Part {
    control(id, Widget::Display, "")
}

/// Pill toggle with the label on the right.
pub fn toggle_right(id: u32, label: &'static str) -> Part {
    control(id, Widget::ToggleRight, label)
}

/// Text toggle; `.momentary()` makes it a text button.
pub fn text_toggle(id: u32, label: &'static str) -> Part {
    control(id, Widget::TextToggle, label)
}

pub fn menu_button(id: u32, label: &'static str) -> Part {
    control(id, Widget::MenuButton, label)
}

pub fn spacer(id: u32) -> Part {
    control(id, Widget::Spacer, "")
}

pub fn separator(id: u32) -> Part {
    control(id, Widget::Separator, "")
}

/// Arc slider: `start` and `span` in degrees (counter-clockwise from +x), the `track` colour (usually `Ink::Clear`,
/// Tape Stop's `Ink::ModuleFill`). Bound to a menu param it is segmented.
pub fn arc(id: u32, start: f32, span: f32, mirrored: bool, track: Ink) -> Part {
    let arc = Widget::Arc {
        start,
        span,
        mirrored,
        track,
    };
    control(id, arc, "")
}

/// Tag slider pointing right (to a display on its right) when `faces_right`.
pub fn tag(id: u32, label: &'static str, faces_right: bool) -> Part {
    control(id, Widget::Tag { faces_right }, label)
}

/// Graph knob; `falling` plots the curve backwards (`R. Curve`).
pub fn graph_knob(id: u32, label: &'static str, falling: bool) -> Part {
    control(id, Widget::GraphKnob { falling }, label)
}

/// XY pad bound to params `x` and `y`.
pub fn xy_pad(id: u32, x: usize, y: usize) -> Part {
    control(id, Widget::XyPad, "")
        .p(x)
        .with_leaf(|l| l.y_param = Some(y))
}

impl Part {
    pub fn w(mut self, w: Size) -> Self {
        self.record.w = w;
        self
    }

    pub fn h(mut self, h: Size) -> Self {
        self.record.h = h;
        self
    }

    /// Margins in `U`: top, right, bottom, left.
    pub fn margins(mut self, t: f32, r: f32, b: f32, l: f32) -> Self {
        self.record.margins = Margins::new(t, r, b, l);
        self
    }

    pub fn mt(mut self, v: f32) -> Self {
        self.record.margins.t = v;
        self
    }

    pub fn mr(mut self, v: f32) -> Self {
        self.record.margins.r = v;
        self
    }

    pub fn mb(mut self, v: f32) -> Self {
        self.record.margins.b = v;
        self
    }

    pub fn ml(mut self, v: f32) -> Self {
        self.record.margins.l = v;
        self
    }

    pub fn show(mut self, show: Show) -> Self {
        self.show = show;
        self
    }

    /// Binds the control to param `p`.
    pub fn p(self, p: usize) -> Self {
        self.with_leaf(|l| l.param = Some(p))
    }

    /// Turns on the value strip (sliders, "value text") or the `On` / `Off` strip (toggles).
    pub fn value_text(self) -> Self {
        self.with_leaf(|l| l.value_text = true)
    }

    pub fn font(self, font: FontSize) -> Self {
        self.with_leaf(|l| l.font = Some(font))
    }

    /// Text controls: where the text hangs in the rect; left-centre unless set.
    pub fn align(self, align: Align2) -> Self {
        self.with_leaf(|l| l.align = align)
    }

    /// Draws the control in `ink` instead of the channel colour.
    pub fn ink(self, ink: Ink) -> Self {
        self.with_leaf(|l| l.ink = ink)
    }

    /// The label follows the params: `label` must be what `f` gives at the registry defaults.
    pub fn label_fn(self, f: fn(&[f32]) -> String) -> Self {
        self.with_leaf(|l| l.label_fn = Some(f))
    }

    /// The value text, when the registry has none for the control.
    pub fn value_fn(self, f: fn(f32) -> String) -> Self {
        self.with_leaf(|l| l.value_fn = Some(f))
    }

    /// Displays: the module draws `f` in it instead of the placeholder ([`Draw`]).
    pub fn draw(self, f: Draw) -> Self {
        self.with_leaf(|l| {
            debug_assert!(
                l.widget == Widget::Display,
                "draw on #{} {:?}",
                l.id,
                l.widget
            );
            l.draw = Some(f);
        })
    }

    /// Spinners: a tap is the card wiring's and opens no popup ([`Leaf::picker`]).
    pub fn picker(self) -> Self {
        self.with_leaf(|l| {
            debug_assert!(
                l.widget == Widget::Spinner,
                "picker on #{} {:?}",
                l.id,
                l.widget
            );
            l.picker = true;
        })
    }

    /// The items of a spinner or menu button without a param.
    pub fn items(self, items: &'static [&'static str]) -> Self {
        self.with_leaf(|l| l.items = items)
    }

    /// Spinners and menu buttons: `len` items per popup column, filled top to bottom (the Arpeggiator's `Arp`: 8).
    pub fn items_per_column(self, len: usize) -> Self {
        self.with_leaf(|l| l.column_len = Some(len))
    }

    /// Spinners and menu buttons: items that cannot be picked.
    pub fn disabled_items(self, items: &'static [usize]) -> Self {
        self.with_leaf(|l| l.disabled_items = items)
    }

    /// The start value of a control without a param (see [`Leaf::default`]).
    pub fn default(self, v: f32) -> Self {
        self.with_leaf(|l| l.default = v)
    }

    pub fn colors(self, colors: ButtonColors) -> Self {
        self.with_button(|l| l.colors = Some(colors))
    }

    /// Round buttons: the disc's share of the short side.
    pub fn shape(self, shape: f32) -> Self {
        self.with_button(|l| l.shape = shape)
    }

    pub fn icon(self, icon: IconId) -> Self {
        self.with_button(|l| l.icon = Some(icon))
    }

    /// Buttons and text toggles: on only while pressed.
    pub fn momentary(self) -> Self {
        self.with_leaf(|l| {
            let ok = matches!(l.widget, Widget::Button { .. } | Widget::TextToggle);
            debug_assert!(
                ok,
                "momentary on #{} {:?}, which ignores it",
                l.id, l.widget
            );
            l.momentary = true;
        })
    }

    /// Buttons: a halo `px` device pixels wider than the button (the Tuner's Input: 4).
    pub fn halo(self, px: f32) -> Self {
        self.with_button(|l| l.halo = Some(Pad::Px(px)))
    }

    /// Buttons: a halo `n` t wider than the button (the Pitcher's `BYP`: 1).
    pub fn halo_t(self, n: f32) -> Self {
        self.with_button(|l| l.halo = Some(Pad::T(n)))
    }

    /// Buttons: the halo's colour; without it the channel colour. The Multiband's band `Solo` buttons are
    /// `text`.
    pub fn halo_color(self, ink: Ink) -> Self {
        self.with_button(|l| l.halo_ink = Some(ink))
    }

    /// Buttons and sliders: enabled only while `show` holds, else inert, a button at half alpha and a slider in
    /// `menuFill` ([`Leaf::enabled`]).
    pub fn enabled(self, show: Show) -> Self {
        self.with_leaf(|l| {
            let ok = matches!(
                l.widget,
                Widget::Button { .. } | Widget::HSlider | Widget::VSlider
            );
            debug_assert!(ok, "enabled on #{} {:?}, which ignores it", l.id, l.widget);
            l.enabled = show;
        })
    }

    /// Sliders: in their colours only while `show` holds, else all `moduleFill` but still live ([`Leaf::lit`]).
    pub fn lit(self, show: Show) -> Self {
        self.with_leaf(|l| {
            let ok = matches!(l.widget, Widget::HSlider | Widget::VSlider);
            debug_assert!(ok, "lit on #{} {:?}, which ignores it", l.id, l.widget);
            l.lit = show;
        })
    }

    /// [`Self::with_leaf`] for an option only a button draws: a debug build panics on any other widget, which would
    /// ignore it.
    fn with_button(self, f: impl FnOnce(&mut Leaf)) -> Self {
        self.with_leaf(|l| {
            let ok = matches!(l.widget, Widget::Button { .. });
            debug_assert!(
                ok,
                "a button option on #{} {:?}, which ignores it",
                l.id, l.widget
            );
            f(l);
        })
    }

    /// Panics on a container: these only describe controls, and the definitions are fixed data under test.
    fn with_leaf(mut self, f: impl FnOnce(&mut Leaf)) -> Self {
        match &mut self.kind {
            PartKind::Leaf(leaf) => f(leaf),
            _ => panic!("control options on a container"),
        }
        self
    }
}

/// A module's reference values, for the data-driven test: the content size, the settings of a fresh card, and one
/// case per page and dynamic state.
#[cfg(test)]
pub struct Reference {
    /// The content rect in `U`: 14 × its content height for an effect, 16.96 × (card height −
    /// 2.01) for an instrument.
    pub size: [f32; 2],
    /// The settings a fresh card starts with, `(#n, value)`: every control without a param that has a value, at its
    /// default (an item index for lists and spinners).
    pub settings: &'static [(u32, f32)],
    pub cases: &'static [Case],
}

#[cfg(test)]
impl Reference {
    /// No cases yet: the module's batch has not transcribed it.
    pub const fn new(size: [f32; 2]) -> Self {
        Self {
            size,
            settings: &[],
            cases: &[],
        }
    }
}

/// One state of a module and what the table shows in it. Start from [`Case::DEFAULT`] with struct update syntax.
#[cfg(test)]
pub struct Case {
    /// Names the state in failure messages: `"LEVEL"`, `"Sync on"`.
    pub name: &'static str,
    /// The selected page of the tab strip.
    pub tab: usize,
    /// Params set before the layout, `(index, normalised value)`; the others are at their registry defaults.
    pub params: &'static [(usize, f32)],
    /// Settings of controls without a param, `(#n, value)`; the others are at their defaults.
    pub settings: &'static [(u32, f32)],
    /// Module-state flags the card sets, `(key, item)` ([`Show::Flag`]); the others are 0.
    pub flags: &'static [(&'static str, usize)],
    /// The visible controls in tree order, one [`row`] each.
    pub rows: &'static [Row],
}

#[cfg(test)]
impl Case {
    pub const DEFAULT: Self = Self {
        name: "default",
        tab: 0,
        params: &[],
        settings: &[],
        flags: &[],
        rows: &[],
    };
}

/// A visible control as the table gives it: `#n`, type, label as drawn, param, value strip, font, size rule and rect,
/// and every other option of its [`Leaf`]. Make one with [`row`] and the chainable setters, named as [`Part`]'s. A
/// column the row leaves alone says the leaf has the default, so an option the table does not give fails the test.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub id: u32,
    pub widget: Widget,
    pub label: &'static str,
    pub param: Option<usize>,
    /// XY pads: the Y param.
    pub y_param: Option<usize>,
    pub value_text: bool,
    /// The font the table gives (FontSize 4 for the Comb Filter's `?`); `None`: the widget's own.
    pub font: Option<FontSize>,
    /// Spinners and menu buttons: the items they show, joined by ` · `: the menu's when bound, else the leaf's own.
    /// `None`: the leaf has no items of its own (a bound one shows its menu's).
    pub items: Option<&'static str>,
    /// The value text a [`Leaf::value_fn`] gives at the case's value; `None`: the leaf has no `value_fn`.
    pub value: Option<&'static str>,
    pub column_len: Option<usize>,
    pub disabled_items: &'static [usize],
    pub momentary: bool,
    pub picker: bool,
    pub halo: Option<Pad>,
    pub halo_ink: Option<Ink>,
    pub enabled: Show,
    pub lit: Show,
    pub shape: f32,
    pub icon: Option<IconId>,
    pub ink: Ink,
    pub colors: Option<ButtonColors>,
    pub align: Align2,
    /// The size rule: flex 1 by flex 1, no margins, unless set.
    pub record: LayoutRecord,
    /// `[x, y, w, h]` in `U`, as printed.
    pub rect: [f32; 4],
    /// How many device pixels each edge may be off at `U` = 100, and why. Exact by default: each printed number
    /// rounded to a pixel, the far edges `x + w` and `y + h` of those.
    pub slack: (f32, &'static str),
}

/// A row with no param, no value strip, the widget's font, the default size rule, no slack, and every other option
/// at the [`Leaf`] default.
#[cfg(test)]
pub const fn row(id: u32, widget: Widget, label: &'static str, rect: [f32; 4]) -> Row {
    Row {
        id,
        widget,
        label,
        param: None,
        y_param: None,
        value_text: false,
        font: None,
        items: None,
        value: None,
        column_len: None,
        disabled_items: &[],
        momentary: false,
        picker: false,
        halo: None,
        halo_ink: None,
        enabled: Show::Always,
        lit: Show::Always,
        shape: 1.0,
        icon: None,
        ink: Ink::Channel,
        colors: None,
        align: Align2::LEFT_CENTER,
        record: LayoutRecord {
            w: Size::FILL,
            h: Size::FILL,
            margins: Margins::ZERO,
        },
        rect,
        slack: (0.0, ""),
    }
}

#[cfg(test)]
impl Row {
    pub const fn p(mut self, p: usize) -> Self {
        self.param = Some(p);
        self
    }

    /// XY pads: the X and Y params.
    pub const fn xy(mut self, x: usize, y: usize) -> Self {
        (self.param, self.y_param) = (Some(x), Some(y));
        self
    }

    pub const fn value_text(mut self) -> Self {
        self.value_text = true;
        self
    }

    pub const fn font(mut self, font: FontSize) -> Self {
        self.font = Some(font);
        self
    }

    /// The items shown, joined by ` · `: `"Log · Lin"`.
    pub const fn items(mut self, items: &'static str) -> Self {
        self.items = Some(items);
        self
    }

    /// The value text at the case's value: `"440 Hz"`.
    pub const fn value(mut self, text: &'static str) -> Self {
        self.value = Some(text);
        self
    }

    pub const fn items_per_column(mut self, len: usize) -> Self {
        self.column_len = Some(len);
        self
    }

    pub const fn disabled_items(mut self, items: &'static [usize]) -> Self {
        self.disabled_items = items;
        self
    }

    pub const fn momentary(mut self) -> Self {
        self.momentary = true;
        self
    }

    pub const fn picker(mut self) -> Self {
        self.picker = true;
        self
    }

    pub const fn halo(mut self, px: f32) -> Self {
        self.halo = Some(Pad::Px(px));
        self
    }

    pub const fn halo_t(mut self, n: f32) -> Self {
        self.halo = Some(Pad::T(n));
        self
    }

    pub const fn halo_color(mut self, ink: Ink) -> Self {
        self.halo_ink = Some(ink);
        self
    }

    pub const fn enabled(mut self, show: Show) -> Self {
        self.enabled = show;
        self
    }

    pub const fn lit(mut self, show: Show) -> Self {
        self.lit = show;
        self
    }

    pub const fn shape(mut self, shape: f32) -> Self {
        self.shape = shape;
        self
    }

    pub const fn icon(mut self, icon: IconId) -> Self {
        self.icon = Some(icon);
        self
    }

    pub const fn ink(mut self, ink: Ink) -> Self {
        self.ink = ink;
        self
    }

    pub const fn colors(mut self, colors: ButtonColors) -> Self {
        self.colors = Some(colors);
        self
    }

    pub const fn align(mut self, align: Align2) -> Self {
        self.align = align;
        self
    }

    pub const fn w(mut self, w: Size) -> Self {
        self.record.w = w;
        self
    }

    pub const fn h(mut self, h: Size) -> Self {
        self.record.h = h;
        self
    }

    /// Margins in `U`: top, right, bottom, left.
    pub const fn margins(mut self, t: f32, r: f32, b: f32, l: f32) -> Self {
        self.record.margins = Margins::new(t, r, b, l);
        self
    }

    /// Lets each edge be `px` device pixels off at `U` = 100, for the reason given (f32 noise in the expected
    /// numbers, say).
    pub const fn slack(mut self, px: f32, why: &'static str) -> Self {
        assert!(!why.is_empty(), "slack needs a reason");
        self.slack = (px, why);
        self
    }
}

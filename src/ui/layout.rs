//! The panel layout engine: trees of `HStack`, `VStack` and `Layer` containers with leaf controls, solved to one pixel
//! rect per leaf.
//!
//! It is pure: no painting, no egui `Context`. A panel definition is a [`Node`] tree built as data, one builder call
//! per node. A tab or a parameter that decides what is shown is resolved by the caller while building the tree, with
//! [`Node::visible`]; build the tree each frame, or keep it and flip the flag. [`layout`] then returns `(leaf id,
//! rect)` for every visible leaf in tree order, which is the creation and drawing order (`#n`). Hidden nodes take no
//! space and none of their subtree is placed.
//!
//! Everything is in device pixels: the caller passes the content rect and `U` in pixels
//! ([`Metrics::u_px`](crate::metrics::Metrics::u_px)) and converts the result to points if it needs to.
//!
//! # The rules
//!
//! Along the main axis of a stack fixed children (`U`, `%`) and all main-axis margins are summed first, the remainder
//! `free = max(0, length - fixed)` is shared among the `Flex` children by weight (`free * weight / sum of weights`),
//! and the children are laid end to end from the left or top edge. Space left over stays empty at the far end.
//!
//! On the cross axis, and on both axes of a [`Layer`](Kind::Layer) child, the size is `U` times `u`,
//! `%` of the parent's length, or for `Flex` the *fill*: the parent's length minus the leading and trailing
//! margin, times the weight. The box is then centred in the parent and shifted by half the difference of the
//! margins: `offset = (parent - size + leading - trailing) / 2`. For `Flex(1)` that is exactly the parent
//! minus its margins; with equal margins it is a plain centring, whatever the size. The MultiFX XY pad (5.24 U
//! high in a 5.74 U row with only a 0.5 U top margin) and the Analyzer's side column (6.14 U in 6.74 U with only
//! a 0.6 U bottom margin) pin the leading-plus-trailing reading.
//!
//! # Whole pixels
//!
//! Rects are solved in floating point without rounding, from the root down, and snapped once per leaf; containers
//! are never snapped (the Comb Filter reference shows it). Each *edge* is truncated on its own: `x0 = trunc(x)`,
//! `x1 = trunc(x + w)`, and the width is `x1 - x0` (likewise for y). This is what the reference tables show.
//! Six equal children of 14 U at `U = 100` are 233.33 px each. They start at 0, 233, 466, 700, 933, 1166 and
//! end at 233, 466, 700, 933, 1166, 1400, so the widths are 2.33, 2.33, 2.34, 2.33, 2.33, 2.34 U with no gap
//! and no overlap. Truncating the width itself would give 2.33 six times and leave 2 px unused. Likewise a row
//! that runs from y = 399.5 to 799 is 400 px high, not 399. Truncation is toward zero and relative to the root's
//! top-left corner, so the result does not depend on where the root sits; pass a whole-pixel origin to get
//! pixel-aligned rects.
//!
//! The arithmetic is plain `f32` in a fixed order, which the reference tables carry the traces of:
//! Spreader's `60 %` column is 840.00006 px, so its neighbours end at 279.99997 px, and the reference reads
//! 2.29 and 2.79 where exact arithmetic would give 2.30 and 2.80. The tests hit every edge exactly.

use eframe::egui::{Rect, pos2};

/// A leaf control's id: its `#n`, or whatever the panel definition wants to look the rect up by.
pub type NodeId = u32;

/// One side length of a control.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    /// A fixed length in `U`.
    U(f32),
    /// A percentage of the parent's length on that axis: `Percent(60.0)` is 60 %.
    Percent(f32),
    /// A weight. On the main axis the share of the free space, on the cross axis the fill factor.
    Flex(f32),
}

impl Size {
    /// The default: flex 1.
    pub const FILL: Self = Self::Flex(1.0);
}

/// Margins in `U`, top, right, bottom, left (`t r b l`). May be negative.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Margins {
    pub t: f32,
    pub r: f32,
    pub b: f32,
    pub l: f32,
}

impl Margins {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    pub const fn new(t: f32, r: f32, b: f32, l: f32) -> Self {
        Self { t, r, b, l }
    }
}

/// A control's layout record: width, height and margins. The default is flex 1 by flex 1 with no margins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutRecord {
    pub w: Size,
    pub h: Size,
    pub margins: Margins,
}

impl Default for LayoutRecord {
    fn default() -> Self {
        Self {
            w: Size::FILL,
            h: Size::FILL,
            margins: Margins::ZERO,
        }
    }
}

impl LayoutRecord {
    /// Size and (leading, trailing) margins in `U` along one axis.
    fn along(&self, horizontal: bool) -> (Size, f32, f32) {
        let m = self.margins;
        if horizontal {
            (self.w, m.l, m.r)
        } else {
            (self.h, m.t, m.b)
        }
    }
}

/// What a node is.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// Children left to right.
    HStack(Vec<Node>),
    /// Children top to bottom.
    VStack(Vec<Node>),
    /// Every child over the full parent rect, each sized by its own record and centred.
    Layer(Vec<Node>),
    /// A control. The layout reports its rect under this id.
    Leaf(NodeId),
}

/// A node of a layout tree: its kind, its layout record and whether it takes part in the layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: Kind,
    pub record: LayoutRecord,
    pub visible: bool,
}

impl Node {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            record: LayoutRecord::default(),
            visible: true,
        }
    }

    pub fn leaf(id: NodeId) -> Self {
        Self::new(Kind::Leaf(id))
    }

    pub fn hstack(children: impl IntoIterator<Item = Node>) -> Self {
        Self::new(Kind::HStack(children.into_iter().collect()))
    }

    pub fn vstack(children: impl IntoIterator<Item = Node>) -> Self {
        Self::new(Kind::VStack(children.into_iter().collect()))
    }

    pub fn layer(children: impl IntoIterator<Item = Node>) -> Self {
        Self::new(Kind::Layer(children.into_iter().collect()))
    }

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

    /// Top margin in `U`; the other sides stay.
    pub fn mt(mut self, v: f32) -> Self {
        self.record.margins.t = v;
        self
    }

    /// Right margin in `U`; the other sides stay.
    pub fn mr(mut self, v: f32) -> Self {
        self.record.margins.r = v;
        self
    }

    /// Bottom margin in `U`; the other sides stay.
    pub fn mb(mut self, v: f32) -> Self {
        self.record.margins.b = v;
        self
    }

    /// Left margin in `U`; the other sides stay.
    pub fn ml(mut self, v: f32) -> Self {
        self.record.margins.l = v;
        self
    }

    /// Hidden nodes (and their subtrees) take no space and produce no rects.
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }
}

/// An unsnapped rect in pixels, relative to the root's top-left corner.
#[derive(Clone, Copy)]
struct Frame {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Frame {
    /// Start and length along one axis.
    fn span(self, horizontal: bool) -> (f32, f32) {
        if horizontal {
            (self.x, self.w)
        } else {
            (self.y, self.h)
        }
    }

    /// A frame from a main-axis and a cross-axis (start, length).
    fn from_spans(horizontal: bool, main: (f32, f32), cross: (f32, f32)) -> Self {
        let ((x, w), (y, h)) = if horizontal {
            (main, cross)
        } else {
            (cross, main)
        };
        Self { x, y, w, h }
    }

    /// Each edge truncated on its own (see the module docs).
    fn snap(self) -> Rect {
        let (x0, y0) = (self.x.trunc(), self.y.trunc());
        let (x1, y1) = ((self.x + self.w).trunc(), (self.y + self.h).trunc());
        Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
    }
}

/// Lays `root` out in `rect_px` (device pixels) at `u_px` pixels per `U`, and returns the rect of every
/// visible leaf in tree order. The root fills the rect whatever its own record says; a hidden root gives
/// nothing.
pub fn layout(root: &Node, rect_px: Rect, u_px: f32) -> Vec<(NodeId, Rect)> {
    let mut out = Vec::new();
    if root.visible {
        let frame = Frame {
            x: 0.0,
            y: 0.0,
            w: rect_px.width(),
            h: rect_px.height(),
        };
        place(root, frame, u_px, &mut out);
    }
    let origin = rect_px.min.to_vec2();
    for (_, rect) in &mut out {
        *rect = rect.translate(origin);
    }
    out
}

/// The rect of leaf `id` in a [`layout`] result; `None` when it was not placed (hidden, or not in the tree).
pub fn rect_of(rects: &[(NodeId, Rect)], id: NodeId) -> Option<Rect> {
    rects.iter().find(|(i, _)| *i == id).map(|(_, r)| *r)
}

fn place(node: &Node, frame: Frame, u: f32, out: &mut Vec<(NodeId, Rect)>) {
    match &node.kind {
        Kind::Leaf(id) => out.push((*id, frame.snap())),
        Kind::HStack(children) => stack(children, frame, true, u, out),
        Kind::VStack(children) => stack(children, frame, false, u, out),
        Kind::Layer(children) => {
            for child in children.iter().filter(|c| c.visible) {
                let (x, w) = cross_axis(child.record.along(true), frame.w, u);
                let (y, h) = cross_axis(child.record.along(false), frame.h, u);
                let frame = Frame {
                    x: frame.x + x,
                    y: frame.y + y,
                    w,
                    h,
                };
                place(child, frame, u, out);
            }
        }
    }
}

/// Lays out a stack: the main-axis rules along the stack's axis, the cross-axis rules across it.
fn stack(children: &[Node], frame: Frame, horizontal: bool, u: f32, out: &mut Vec<(NodeId, Rect)>) {
    let (main_start, main_len) = frame.span(horizontal);
    let (cross_start, cross_len) = frame.span(!horizontal);
    let visible = || children.iter().filter(|c| c.visible);
    let fixed_len = |size: Size| match size {
        Size::U(v) => v * u,
        Size::Percent(p) => p / 100.0 * main_len,
        Size::Flex(_) => 0.0,
    };

    let (mut fixed, mut weights) = (0.0, 0.0);
    for child in visible() {
        let (size, lead, trail) = child.record.along(horizontal);
        fixed += fixed_len(size) + (lead + trail) * u;
        if let Size::Flex(weight) = size {
            weights += weight;
        }
    }
    let free = (main_len - fixed).max(0.0);

    let mut cursor = main_start;
    for child in visible() {
        let (size, lead, trail) = child.record.along(horizontal);
        let main = match size {
            Size::Flex(weight) if weights > 0.0 => free * weight / weights,
            _ => fixed_len(size),
        };
        let start = cursor + lead * u;
        cursor += main + (lead + trail) * u;
        let (offset, len) = cross_axis(child.record.along(!horizontal), cross_len, u);
        let child_frame = Frame::from_spans(horizontal, (start, main), (cross_start + offset, len));
        place(child, child_frame, u, out);
    }
}

/// The cross-axis rule: a child's (offset, length) on an axis of `parent` pixels, from its size and margins along it.
fn cross_axis((size, lead, trail): (Size, f32, f32), parent: f32, u: f32) -> (f32, f32) {
    let (lead, trail) = (lead * u, trail * u);
    let len = match size {
        Size::U(v) => v * u,
        Size::Percent(p) => p / 100.0 * parent,
        Size::Flex(weight) => ((parent - lead - trail) * weight).max(0.0),
    };
    ((parent - len + lead - trail) / 2.0, len)
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Rect, pos2, vec2};

    use super::Size::{Flex, Percent, U};
    use super::*;

    /// The reference layouts' scale: `U = 100` px, 14 U wide content.
    const U_PX: f32 = 100.0;

    fn content(w_px: f32, h_px: f32) -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(w_px, h_px))
    }

    fn run(root: &Node, w_px: f32, h_px: f32) -> Vec<(NodeId, Rect)> {
        layout(root, content(w_px, h_px), U_PX)
    }

    /// Asserts the leaves come back in this order and that each rect `[x, y, w, h]` (U, as the tables print
    /// them) is hit exactly, edge by edge. The engine reproduces every table to the pixel, so no tolerance.
    fn assert_reference(got: &[(NodeId, Rect)], expected: &[(NodeId, [f32; 4])]) {
        let ids: Vec<NodeId> = got.iter().map(|(id, _)| *id).collect();
        let want_ids: Vec<NodeId> = expected.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, want_ids, "leaves placed");
        for ((id, rect), (_, r)) in got.iter().zip(expected) {
            let [x, y, w, h] = r.map(|v| (v * U_PX).round());
            let want = [x, y, x + w, y + h];
            let have = [rect.min.x, rect.min.y, rect.max.x, rect.max.y];
            assert_eq!(have, want, "#{id} (x0 y0 x1 y1 in px)");
        }
    }

    // ---- the reference tables ----

    #[test]
    fn distortion_matches_the_reference() {
        let root = Node::hstack((1..=6).map(Node::leaf));
        assert_reference(
            &run(&root, 1400.0, 349.0),
            &[
                (1, [0.0, 0.0, 2.33, 3.49]),
                (2, [2.33, 0.0, 2.33, 3.49]),
                (3, [4.66, 0.0, 2.34, 3.49]),
                (4, [7.0, 0.0, 2.33, 3.49]),
                (5, [9.33, 0.0, 2.33, 3.49]),
                (6, [11.66, 0.0, 2.34, 3.49]),
            ],
        );
    }

    #[test]
    fn multifx_matches_the_reference() {
        let root = Node::vstack([
            Node::leaf(1).h(U(1.25)),
            Node::hstack([
                Node::vstack([Node::leaf(2), Node::leaf(3)]),
                Node::leaf(4).w(Flex(6.0)).margins(0.5, 0.0, 0.0, 0.0),
                Node::leaf(5),
            ]),
        ]);
        assert_reference(
            &run(&root, 1400.0, 699.0),
            &[
                (1, [0.0, 0.0, 14.0, 1.25]),
                (2, [0.0, 1.25, 1.75, 2.87]),
                (3, [0.0, 4.12, 1.75, 2.87]),
                (4, [1.75, 1.75, 10.5, 5.24]),
                (5, [12.25, 1.25, 1.75, 5.74]),
            ],
        );
    }

    /// The Equalizer's four band rows; `tab` is the selected band.
    fn equalizer(tab: usize) -> Node {
        let band = |k: usize| {
            let first = 3 + 5 * k as NodeId;
            Node::hstack((first..first + 5).map(Node::leaf)).visible(tab == k)
        };
        Node::vstack([
            Node::vstack([Node::leaf(1).h(U(1.25)), Node::leaf(2)]),
            band(0),
            band(1),
            band(2),
            band(3),
        ])
    }

    #[test]
    fn equalizer_band_1_matches_the_reference() {
        assert_reference(
            &run(&equalizer(0), 1400.0, 799.0),
            &[
                (1, [0.0, 0.0, 14.0, 1.25]),
                (2, [0.0, 1.25, 14.0, 2.74]),
                (3, [0.0, 3.99, 2.8, 4.0]),
                (4, [2.8, 3.99, 2.8, 4.0]),
                (5, [5.6, 3.99, 2.8, 4.0]),
                (6, [8.4, 3.99, 2.8, 4.0]),
                (7, [11.2, 3.99, 2.8, 4.0]),
            ],
        );
    }

    #[test]
    fn every_equalizer_band_page_gets_the_same_rects() {
        // Hidden bands give their space back: whichever band is selected, the row is the same.
        let rects = |tab| -> Vec<Rect> {
            run(&equalizer(tab), 1400.0, 799.0)
                .iter()
                .skip(2)
                .map(|(_, r)| *r)
                .collect()
        };
        for tab in 1..4 {
            assert_eq!(rects(tab), rects(0), "band {}", tab + 1);
        }
        assert_eq!(run(&equalizer(2), 1400.0, 799.0)[2].0, 13);
    }

    // ---- more references, for the rules the three tables above do not reach ----

    /// Wow & Flutter: hidden spinners, main-axis margins, flex 1.5 and `fill x 0.5` on the cross axis.
    #[test]
    fn wow_and_flutter_matches_the_reference() {
        let column = |a: NodeId| {
            Node::vstack([
                Node::leaf(a),
                Node::leaf(a + 1).visible(false),
                Node::leaf(a + 2).margins(0.5, 0.0, 0.0, 0.0),
            ])
        };
        let root = Node::hstack([
            column(1),
            column(4),
            Node::vstack([
                Node::leaf(7).w(Flex(0.5)).margins(0.0, 0.0, 0.5, 0.0),
                Node::leaf(8).h(Flex(1.5)),
            ])
            .w(Flex(2.0)),
            column(9),
        ]);
        assert_reference(
            &run(&root, 1400.0, 699.0),
            &[
                (1, [0.0, 0.0, 2.8, 3.24]),
                (3, [0.0, 3.74, 2.8, 3.25]),
                (4, [2.8, 0.0, 2.8, 3.24]),
                (6, [2.8, 3.74, 2.8, 3.25]),
                (7, [7.0, 0.0, 2.8, 2.59]),
                (8, [5.6, 3.09, 5.6, 3.9]),
                (9, [11.2, 0.0, 2.8, 3.24]),
                (11, [11.2, 3.74, 2.8, 3.25]),
            ],
        );
    }

    /// Spreader: `%` sizes on both axes, a negative bottom margin and `fill x 0.9`. The reference shows the
    /// f32 error of `60 %` (2.29 and 2.79 rather than 2.30 and 2.80).
    #[test]
    fn spreader_matches_the_reference() {
        let root = Node::hstack([
            Node::vstack([Node::leaf(1), Node::leaf(2)]).margins(0.0, 0.0, 0.0, 0.5),
            Node::leaf(3).w(Percent(60.0)).h(Percent(85.0)),
            Node::vstack([
                Node::leaf(4),
                Node::leaf(5).w(Flex(0.9)).margins(0.4, 0.0, -0.4, 0.0),
            ])
            .margins(0.0, 0.5, 0.0, 0.0),
        ]);
        assert_reference(
            &run(&root, 1400.0, 599.0),
            &[
                (1, [0.5, 0.0, 2.29, 2.99]),
                (2, [0.5, 2.99, 2.29, 3.0]),
                (3, [2.79, 0.44, 8.41, 5.1]),
                (4, [11.2, 0.0, 2.3, 2.99]),
                (5, [11.31, 3.39, 2.07, 3.0]),
            ],
        );
    }

    /// Tremolo: unequal main-axis margins, a weight below 1 and `fill x 0.88` centred on the cross axis.
    #[test]
    fn tremolo_matches_the_reference() {
        let root = Node::hstack([
            Node::vstack([
                Node::leaf(1),
                Node::leaf(2).margins(0.5, 0.0, 0.0, 0.0),
                Node::leaf(3).margins(0.5, 0.0, 0.0, 0.0).visible(false),
            ]),
            Node::vstack([
                Node::hstack([
                    Node::leaf(4),
                    Node::leaf(5).w(Flex(6.5)).h(Flex(0.88)),
                    Node::leaf(6),
                ])
                .margins(0.2, 0.0, 0.4, 0.0),
                Node::hstack([
                    Node::leaf(7).w(Flex(0.5)).margins(0.0, 0.0, 0.5, 0.0),
                    Node::leaf(8),
                    Node::leaf(9),
                    Node::leaf(10),
                ])
                .h(Flex(0.5)),
            ])
            .w(Flex(3.0)),
            Node::vstack([Node::leaf(11), Node::leaf(12).margins(0.5, 0.0, 0.0, 0.0)]),
        ]);
        assert_reference(
            &run(&root, 1400.0, 699.0),
            &[
                (1, [0.0, 0.0, 2.8, 3.24]),
                (2, [0.0, 3.74, 2.8, 3.25]),
                (4, [2.8, 0.2, 0.98, 4.26]),
                (5, [3.78, 0.45, 6.43, 3.75]),
                (6, [10.21, 0.2, 0.99, 4.26]),
                (7, [2.8, 4.86, 1.2, 1.63]),
                (8, [4.0, 4.86, 2.4, 2.13]),
                (9, [6.4, 4.86, 2.4, 2.13]),
                (10, [8.8, 4.86, 2.4, 2.13]),
                (11, [11.2, 0.0, 2.8, 3.24]),
                (12, [11.2, 3.74, 2.8, 3.25]),
            ],
        );
    }

    /// Analyzer: a fixed-width column whose margins are unequal on the cross axis (bottom 0.6 U only) is
    /// 0.6 U shorter than its row and stays at the top; the row's other children are inset by equal margins.
    fn analyzer(page: usize) -> Node {
        let graph = |id: NodeId| Node::leaf(id).margins(0.3, 0.0, 0.3, 0.7);
        let side = |ids: [NodeId; 2]| {
            Node::vstack(ids.map(Node::leaf))
                .w(U(2.0))
                .margins(0.0, 0.0, 0.6, 0.0)
        };
        Node::vstack([
            Node::hstack([graph(1), side([3, 4])]).visible(page == 0),
            Node::hstack([graph(10).margins(0.3, 0.7, 0.3, 0.7)]).visible(page == 3),
            Node::vstack([
                Node::leaf(13),
                Node::hstack((14..=17).map(Node::leaf)).h(U(3.5)),
                Node::leaf(18),
            ])
            .visible(page == 5),
            Node::leaf(19).h(U(1.25)),
        ])
    }

    #[test]
    fn analyzer_pages_match_the_reference() {
        let tabs = (19, [0.0, 6.74, 14.0, 1.25]);
        assert_reference(
            &run(&analyzer(0), 1400.0, 799.0),
            &[
                (1, [0.7, 0.3, 11.3, 6.14]),
                (3, [12.0, 0.0, 2.0, 3.07]),
                (4, [12.0, 3.07, 2.0, 3.07]),
                tabs,
            ],
        );
        assert_reference(
            &run(&analyzer(3), 1400.0, 799.0),
            &[(10, [0.7, 0.3, 12.6, 6.14]), tabs],
        );
        assert_reference(
            &run(&analyzer(5), 1400.0, 799.0),
            &[
                (13, [0.0, 0.0, 14.0, 1.62]),
                (14, [0.0, 1.62, 3.5, 3.5]),
                (15, [3.5, 1.62, 3.5, 3.5]),
                (16, [7.0, 1.62, 3.5, 3.5]),
                (17, [10.5, 1.62, 3.5, 3.5]),
                (18, [0.0, 5.12, 14.0, 1.62]),
                tabs,
            ],
        );
    }

    /// Arpeggiator: `%` widths and heights, margins on `100 %` containers (they only shift, never shrink), and
    /// a Layer holding a half-height knob over a full-size display.
    fn arpeggiator(page: usize) -> Node {
        let pct = |id: NodeId, h: f32| Node::leaf(id).w(Percent(100.0)).h(Percent(h));
        let flex = |id: NodeId| Node::leaf(id).w(Percent(100.0));
        let inset = |n: Node| n.h(Percent(100.0)).margins(0.25, 0.0, 0.25, 0.0);
        let page_row =
            |children: Vec<Node>| Node::hstack(children).h(U(6.0)).margins(0.5, 0.0, 0.0, 0.0);
        let global = page_row(vec![
            inset(Node::vstack([pct(2, 50.0), pct(3, 50.0)]).w(Percent(10.0))),
            inset(Node::vstack([pct(4, 50.0), pct(5, 50.0)]).w(Percent(27.5))),
            inset(
                Node::vstack([pct(6, 60.0), pct(7, 60.0).visible(false), pct(8, 40.0)])
                    .w(Percent(25.0)),
            ),
            inset(Node::vstack([pct(9, 100.0)]).w(Percent(18.75))),
            Node::vstack([flex(10), flex(11)])
                .w(Percent(18.75))
                .h(Percent(100.0)),
        ])
        .visible(page == 0);
        let arp1 = page_row(vec![
            inset(Node::hstack([
                inset(Node::vstack([flex(12), flex(13)])),
                inset(Node::vstack([flex(14), flex(15)])),
            ])),
            inset(Node::vstack([
                Node::leaf(16).h(U(0.7)),
                Node::layer([pct(17, 50.0), pct(18, 100.0)]).w(Percent(100.0)),
                Node::leaf(19).w(Percent(100.0)).h(U(0.4)),
            ])),
            inset(Node::hstack([
                inset(Node::vstack([flex(20), flex(21)])),
                inset(Node::vstack([flex(22), flex(23)])),
            ])),
        ])
        .visible(page == 1);
        Node::vstack([Node::leaf(1).h(U(1.25)), global, arp1])
    }

    #[test]
    fn arpeggiator_pages_match_the_reference() {
        let tabs = (1, [0.0, 0.0, 14.0, 1.25]);
        assert_reference(
            &run(&arpeggiator(0), 1400.0, 799.0),
            &[
                tabs,
                (2, [0.0, 1.75, 1.4, 3.0]),
                (3, [0.0, 4.75, 1.4, 3.0]),
                (4, [1.4, 1.75, 3.85, 3.0]),
                (5, [1.4, 4.75, 3.85, 3.0]),
                (6, [5.25, 1.75, 3.5, 3.6]),
                (8, [5.25, 5.35, 3.5, 2.4]),
                (9, [8.75, 1.75, 2.62, 6.0]),
                (10, [11.37, 1.75, 2.63, 3.0]),
                (11, [11.37, 4.75, 2.63, 3.0]),
            ],
        );
        assert_reference(
            &run(&arpeggiator(1), 1400.0, 799.0),
            &[
                tabs,
                (12, [0.0, 1.75, 2.33, 3.0]),
                (13, [0.0, 4.75, 2.33, 3.0]),
                (14, [2.33, 1.75, 2.33, 3.0]),
                (15, [2.33, 4.75, 2.33, 3.0]),
                (16, [4.66, 1.75, 4.67, 0.7]),
                (17, [4.66, 3.67, 4.67, 2.45]),
                (18, [4.66, 2.45, 4.67, 4.9]),
                (19, [4.66, 7.35, 4.67, 0.4]),
                (20, [9.33, 1.75, 2.33, 3.0]),
                (21, [9.33, 4.75, 2.33, 3.0]),
                (22, [11.66, 1.75, 2.34, 3.0]),
                (23, [11.66, 4.75, 2.34, 3.0]),
            ],
        );
    }

    /// Comb Filter (Tracking off, Snap on): pins leaf-only snapping. The nested HStack starts at 4.3077 U
    /// and its three children are 1.7949 U each; snapping the container first (to 4.30) would put #6 at 6.09.
    #[test]
    fn comb_filter_matches_the_reference() {
        let root = Node::hstack([
            Node::leaf(1),
            Node::leaf(2).visible(false),
            Node::leaf(3),
            Node::hstack([
                Node::leaf(4).visible(false),
                Node::leaf(5),
                Node::leaf(6),
                Node::leaf(7).visible(false),
                Node::leaf(8),
            ])
            .w(Flex(2.5)),
            Node::vstack([
                Node::hstack([Node::leaf(9), Node::leaf(10)]),
                Node::hstack([Node::leaf(11), Node::leaf(12)]),
            ])
            .w(Flex(2.0)),
        ]);
        assert_reference(
            &run(&root, 1400.0, 499.0),
            &[
                (1, [0.0, 0.0, 2.15, 4.99]),
                (3, [2.15, 0.0, 2.15, 4.99]),
                (5, [4.3, 0.0, 1.8, 4.99]),
                (6, [6.1, 0.0, 1.79, 4.99]),
                (8, [7.89, 0.0, 1.8, 4.99]),
                (9, [9.69, 0.0, 2.15, 2.49]),
                (10, [11.84, 0.0, 2.16, 2.49]),
                (11, [9.69, 2.49, 2.15, 2.5]),
                (12, [11.84, 2.49, 2.16, 2.5]),
            ],
        );
    }

    // ---- the rules, on their own ----

    #[test]
    fn one_side_margins_set_only_their_side() {
        let n = Node::leaf(1).margins(1.0, 2.0, 3.0, 4.0);
        assert_eq!(
            n.clone().mt(9.0).record.margins,
            Margins::new(9.0, 2.0, 3.0, 4.0)
        );
        assert_eq!(
            n.clone().mr(9.0).record.margins,
            Margins::new(1.0, 9.0, 3.0, 4.0)
        );
        assert_eq!(
            n.clone().mb(9.0).record.margins,
            Margins::new(1.0, 2.0, 9.0, 4.0)
        );
        assert_eq!(n.ml(9.0).record.margins, Margins::new(1.0, 2.0, 3.0, 9.0));
    }

    #[test]
    fn rect_of_finds_a_placed_leaf_and_nothing_for_a_hidden_one() {
        let root = Node::hstack([Node::leaf(7), Node::leaf(8).visible(false), Node::leaf(9)]);
        let got = run(&root, 1000.0, 100.0);
        assert_eq!(rect_of(&got, 9), Some(got[1].1));
        assert_eq!(rect_of(&got, 7), Some(got[0].1));
        assert_eq!(rect_of(&got, 8), None);
        assert_eq!(rect_of(&got, 1), None);
    }

    fn rects(root: &Node, w_px: f32, h_px: f32) -> Vec<(NodeId, [f32; 4])> {
        run(root, w_px, h_px)
            .into_iter()
            .map(|(id, r)| (id, [r.min.x, r.min.y, r.width(), r.height()]))
            .collect()
    }

    #[test]
    fn hidden_nodes_take_no_space_and_hide_their_subtree() {
        let root = Node::hstack([
            Node::leaf(1),
            Node::hstack([Node::leaf(2), Node::leaf(3)]).visible(false),
            Node::leaf(4).visible(false),
            Node::leaf(5),
        ]);
        assert_eq!(
            rects(&root, 1000.0, 100.0),
            [
                (1, [0.0, 0.0, 500.0, 100.0]),
                (5, [500.0, 0.0, 500.0, 100.0])
            ]
        );
        assert!(run(&root.visible(false), 1000.0, 100.0).is_empty());
    }

    #[test]
    fn leftover_space_is_not_centred_along_the_main_axis() {
        let root = Node::hstack([Node::leaf(1).w(U(1.0)), Node::leaf(2).w(U(2.0))]);
        assert_eq!(
            rects(&root, 1000.0, 100.0),
            [
                (1, [0.0, 0.0, 100.0, 100.0]),
                (2, [100.0, 0.0, 200.0, 100.0])
            ]
        );
    }

    #[test]
    fn flex_takes_the_space_the_fixed_children_and_margins_leave() {
        let root = Node::hstack([
            Node::leaf(1).w(U(2.0)).margins(0.0, 0.5, 0.0, 0.25),
            Node::leaf(2).w(Flex(1.0)),
            Node::leaf(3).w(Percent(10.0)),
            Node::leaf(4).w(Flex(3.0)),
        ]);
        // 1000 - 200 - 75 - 100 = 625 free, shared 1 : 3.
        let got = rects(&root, 1000.0, 100.0);
        assert_eq!(got[0].1, [25.0, 0.0, 200.0, 100.0]);
        assert_eq!(got[1].1, [275.0, 0.0, 156.0, 100.0]);
        assert_eq!(got[2].1, [431.0, 0.0, 100.0, 100.0]);
        assert_eq!(got[3].1, [531.0, 0.0, 469.0, 100.0]);
    }

    #[test]
    fn free_space_never_goes_negative() {
        let root = Node::hstack([
            Node::leaf(1).w(U(8.0)),
            Node::leaf(2),
            Node::leaf(3).w(U(4.0)),
        ]);
        assert_eq!(
            rects(&root, 1000.0, 100.0),
            [
                (1, [0.0, 0.0, 800.0, 100.0]),
                (2, [800.0, 0.0, 0.0, 100.0]),
                (3, [800.0, 0.0, 400.0, 100.0])
            ]
        );
    }

    #[test]
    fn flex_children_whose_weights_sum_to_zero_get_no_length_not_nan() {
        let root = Node::hstack([
            Node::leaf(1).w(Flex(0.0)),
            Node::leaf(2).w(U(2.0)),
            Node::leaf(3).w(Flex(0.0)),
        ]);
        assert_eq!(
            rects(&root, 1000.0, 100.0),
            [
                (1, [0.0, 0.0, 0.0, 100.0]),
                (2, [0.0, 0.0, 200.0, 100.0]),
                (3, [200.0, 0.0, 0.0, 100.0])
            ]
        );
    }

    #[test]
    fn cross_axis_is_centred_and_unequal_margins_shift_by_half_the_difference() {
        let child = || Node::leaf(1).h(U(2.0));
        let y = |n: Node| rects(&Node::hstack([n]), 500.0, 600.0)[0].1;
        assert_eq!(y(child()), [0.0, 200.0, 500.0, 200.0]);
        // top margin 0.5 U: the box moves down by 25 px; bottom margin 0.5 U moves it up.
        assert_eq!(y(child().margins(0.5, 0.0, 0.0, 0.0))[1], 225.0);
        assert_eq!(y(child().margins(0.0, 0.0, 0.5, 0.0))[1], 175.0);
        assert_eq!(y(child().margins(0.5, 0.0, 0.5, 0.0))[1], 200.0);
        // fill: parent minus both margins, whatever the split.
        assert_eq!(
            y(Node::leaf(1).margins(1.0, 0.0, 0.5, 0.0)),
            [0.0, 100.0, 500.0, 450.0]
        );
    }

    #[test]
    fn a_layer_centres_every_child_over_the_full_rect() {
        let root = Node::layer([
            Node::leaf(1),
            Node::leaf(2).w(Percent(50.0)).h(U(1.0)),
            Node::leaf(3).w(U(2.0)).margins(0.0, 0.0, 0.0, 1.0),
            Node::leaf(4).visible(false),
        ]);
        assert_eq!(
            rects(&root, 400.0, 300.0),
            [
                (1, [0.0, 0.0, 400.0, 300.0]),
                (2, [100.0, 100.0, 200.0, 100.0]),
                (3, [150.0, 0.0, 200.0, 300.0]),
            ]
        );
    }

    #[test]
    fn edges_truncate_on_their_own() {
        // 3 equal children of 100 px: 33.33 each. Edges 0, 33, 66, 100.
        let root = Node::hstack((1..=3).map(Node::leaf));
        let xs: Vec<(f32, f32)> = run(&root, 100.0, 10.0)
            .iter()
            .map(|(_, r)| (r.min.x, r.max.x))
            .collect();
        assert_eq!(xs, [(0.0, 33.0), (33.0, 66.0), (66.0, 100.0)]);
    }

    #[test]
    fn the_rect_may_sit_anywhere_and_results_follow_it() {
        let root = Node::hstack([Node::leaf(1), Node::leaf(2)]);
        let at = Rect::from_min_size(pos2(40.0, 25.0), vec2(101.0, 50.0));
        let got = layout(&root, at, U_PX);
        assert_eq!(
            got[0].1,
            Rect::from_min_max(pos2(40.0, 25.0), pos2(90.0, 75.0))
        );
        assert_eq!(
            got[1].1,
            Rect::from_min_max(pos2(90.0, 25.0), pos2(141.0, 75.0))
        );
    }
}

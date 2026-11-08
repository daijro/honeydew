// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Rind's picture and its hit regions, in logical pixels: one screen with everything in view, in the usual arrangement of
//! a hardware-style synth. The oscillators sit left and right, a display between them (matrix, effects, arpeggiator),
//! then the filters and LFOs, and the envelopes, master and filter response along the bottom. It is dark with warm orange
//! accents, at 16:9, and a selector with many names opens a drop-down list instead of stepping.
//!
//! Everything is driven by the parameter table ([`crate::ids::SPECS`]): a control's kind follows from its spec (a named
//! range is a selector, a two-step Off/On range a switch, anything else a knob), so a page is a list of parameter
//! indices.

use os_ui::{Canvas, Color, Font, Rect};

use crate::ids::{self, Fmt, Kind as Range, SPECS};

/// The editor's size at scale 1.
pub const W: u32 = 640;
pub const H: u32 = 360;

/// Drag length in px for the whole range of a knob.
pub const KNOB_PX: f32 = 150.0;

const BG: Color = Color::rgb(0x14110f);
const PANEL: Color = Color::rgb(0x1f1a17);
const EDGE: Color = Color::rgb(0x3a2f27);
const TRACK: Color = Color::rgb(0x3b322c);
const CAP: Color = Color::rgb(0x2a231f);
const ORANGE: Color = Color::rgb(0xff8a2a);
const AMBER: Color = Color::rgb(0xffc36b);
const TEXT: Color = Color::rgb(0xece1d6);
const MUTED: Color = Color::rgb(0x9d8c7c);
const DARK_TEXT: Color = Color::rgb(0x1a1410);
const SHADE: Color = Color::rgb(0x000000).with_alpha(0x99);

const LABEL: Font = Font::new(8);
const VALUE: Font = Font::new(9).bold();
const TITLE: Font = Font::new(9).bold();

/// The display's tabs.
pub const TABS: [&str; 3] = ["MATRIX", "EFFECTS", "ARP"];
pub const TAB_MATRIX: usize = 0;
pub const TAB_FX: usize = 1;
pub const TAB_ARP: usize = 2;

const MATRIX_SUBS: [&str; 4] = ["1-4", "5-8", "9-12", "13-16"];
const FX_SUBS: [&str; 7] = ["DIST", "PHASE", "CHORUS", "EQ", "DELAY", "REVERB", "COMP"];
const ARP_SUBS: [&str; 2] = ["STEPS 1-8", "STEPS 9-16"];
/// Each effect's on switch, in [`FX_SUBS`] order.
const FX_ON: [usize; 7] = [
    ids::DIST_ON,
    ids::PHS_ON,
    ids::CHO_ON,
    ids::EQ_ON,
    ids::DLY_ON,
    ids::REV_ON,
    ids::CMP_ON,
];

/// What the display is showing, and the drop-down list open, if any (the parameter it picks).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ui {
    pub tab: usize,
    pub subs: [usize; 3],
    pub popup: Option<usize>,
}

impl Ui {
    pub fn sub(&self) -> usize {
        self.subs[self.tab.min(2)]
    }
}

fn subs_of(tab: usize) -> usize {
    match tab {
        TAB_MATRIX => MATRIX_SUBS.len(),
        TAB_FX => FX_SUBS.len(),
        _ => ARP_SUBS.len(),
    }
}

/// What a hit region is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Drag vertically (the cursor held).
    Knob,
    /// A named range: a tap steps to the next name, or opens the list when there are many.
    Cycle,
    Toggle,
    /// A horizontal slider: a press sets, a drag moves.
    Slider,
    /// A vertical bar: a press sets, a drag moves.
    Bar,
    /// A display tab.
    Tab(usize),
    /// A display sub tab.
    Sub(usize),
    /// Name number `i` in the open drop-down list.
    Item(usize),
    /// Anywhere else while a list is open: it closes.
    Dismiss,
}

/// A control's parameter index (into the table), kind and rect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub param: usize,
    pub kind: Kind,
    pub r: Rect,
}

impl Hit {
    /// True for a control of a parameter (not chrome or a list).
    pub fn is_param(&self) -> bool {
        !matches!(
            self.kind,
            Kind::Tab(_) | Kind::Sub(_) | Kind::Item(_) | Kind::Dismiss
        )
    }
}

/// The values the editor shows.
pub trait Values {
    /// Normalised.
    fn norm(&self, p: usize) -> f32;
    fn plain(&self, p: usize) -> f32;
    fn text(&self, p: usize) -> String;
}

/// The kind of control a parameter gets.
pub fn widget(p: usize) -> Kind {
    match SPECS[p].fmt {
        Fmt::Names(n) if n.len() == 2 && n[0] == "Off" => Kind::Toggle,
        Fmt::Names(_) => Kind::Cycle,
        _ => Kind::Knob,
    }
}

/// True for a parameter that steps in whole units.
pub fn is_int(p: usize) -> bool {
    matches!(SPECS[p].kind, Range::Int(..))
}

/// True for a parameter that runs through zero (drawn from its middle).
pub fn bipolar(p: usize) -> bool {
    match SPECS[p].kind {
        Range::Lin(lo, hi) | Range::Skew(lo, hi, _) => lo < 0.0 && hi > 0.0,
        Range::Int(lo, hi) => lo < 0 && hi > 0,
    }
}

/// The names of a selector, and the value the first stands for.
pub fn choices(p: usize) -> (&'static [&'static str], i32) {
    match (SPECS[p].fmt, SPECS[p].kind) {
        (Fmt::Names(n), Range::Int(lo, _)) => (n, lo),
        (Fmt::Names(n), _) => (n, 0),
        _ => (&[], 0),
    }
}

/// True for a selector that opens a list (one with four names or more); a shorter one steps on a tap.
pub fn dropdown(p: usize) -> bool {
    widget(p) == Kind::Cycle && choices(p).0.len() >= 4
}

/// The normalised value a press at `x` sets on a slider in `r`.
pub fn slider_norm(r: Rect, x: f32) -> f32 {
    ((x - r.x - 6.0) / (r.w - 12.0)).clamp(0.0, 1.0)
}

/// The normalised value a press at `y` sets on a bar in `r`.
pub fn bar_norm(r: Rect, y: f32) -> f32 {
    (1.0 - (y - r.y - 2.0) / (r.h - 4.0)).clamp(0.0, 1.0)
}

// ---- labels ----------------------------------------------------------------------------------------------------

/// The prefixes the table's names share within a group, stripped from a label.
const PREFIXES: [&str; 20] = [
    "Osc A1 ",
    "Osc A2 ",
    "Osc B1 ",
    "Osc B2 ",
    "Part A ",
    "Part B ",
    "Filter 1 ",
    "Filter 2 ",
    "Env 3 ",
    "Env 4 ",
    "LFO 1 ",
    "LFO 2 ",
    "Distortion ",
    "Phaser ",
    "Chorus ",
    "EQ ",
    "Delay ",
    "Reverb ",
    "Compressor ",
    "Arp ",
];

/// What a name becomes where a label is a few letters wide.
const SHORT: [(&str, &str); 38] = [
    ("Master Volume", "Master"),
    ("Master Tune", "Tune"),
    ("Polyphony", "Poly"),
    ("Voice Mode", "Mode"),
    ("Portamento", "Porta"),
    ("Portamento Mode", "Glide"),
    ("Bend Range", "Bend"),
    ("Velocity Sensitivity", "Vel"),
    ("Attack", "Att"),
    ("Decay", "Dec"),
    ("Sustain", "Sus"),
    ("Release", "Rel"),
    ("Octave", "Oct"),
    ("Note", "Semi"),
    ("Volume", "Vol"),
    ("Phase Random", "Rand"),
    ("Invert", "Inv"),
    ("Resonance", "Reso"),
    ("Key Track", "Key"),
    ("Input A", "In A"),
    ("Input B", "In B"),
    ("Division", "Div"),
    ("Offset", "Offs"),
    ("Retrigger", "Trig"),
    ("Velocity Source", "Vel"),
    ("Frequency", "Freq"),
    ("Feedback", "Fdbk"),
    ("Bass Frequency", "Bass F"),
    ("Bass Gain", "Bass G"),
    ("Treble Frequency", "Treb F"),
    ("Treble Gain", "Treb G"),
    ("Division L", "Div L"),
    ("Division R", "Div R"),
    ("Ping-Pong", "Ping"),
    ("Low-pass", "LP"),
    ("High-pass", "HP"),
    ("Pre-delay", "Pre"),
    ("Threshold", "Thresh"),
];

/// Parameter `p`'s label over its control.
pub fn short(p: usize) -> &'static str {
    let name = SPECS[p].name;
    let name = PREFIXES
        .iter()
        .find_map(|pre| name.strip_prefix(pre))
        .unwrap_or(name);
    SHORT
        .iter()
        .find(|(long, _)| *long == name)
        .map_or(name, |(_, s)| *s)
}

// ---- layout ----------------------------------------------------------------------------------------------------

const X0: f32 = 3.0;
const TOP_BAR: f32 = 24.0;
const BAR_H: f32 = 20.0;
const HEAD: f32 = 12.0;
const CELL_H: f32 = 34.0;
/// The widths of a cell in the top block (22 columns) and in the two below (20).
const CW1: f32 = 28.8;
const CW2: f32 = 31.8;
const Y1: f32 = 26.0;
const Y2: f32 = 210.0;
const Y3: f32 = 292.0;
const BLOCK1_H: f32 = HEAD + 5.0 * CELL_H;
const BLOCK2_H: f32 = HEAD + 2.0 * CELL_H;
const BLOCK3_H: f32 = HEAD + CELL_H;

/// The display's rect: between the oscillators.
fn display() -> Rect {
    Rect::new(X0 + 7.0 * CW1, Y1, 8.0 * CW1, BLOCK1_H)
}

/// Where the display's tab body starts, and its height.
fn body() -> Rect {
    let d = display();
    Rect::new(d.x + 1.0, d.y + 50.0, d.w - 2.0, d.h - 52.0)
}

fn tab_rect(i: usize) -> Rect {
    let d = display();
    let w = (d.w - 8.0) / 3.0;
    Rect::new(d.x + 4.0 + i as f32 * w, d.y + 3.0, w - 2.0, 21.0)
}

fn sub_rect(tab: usize, i: usize) -> Rect {
    let d = display();
    let n = subs_of(tab) as f32;
    let w = (d.w - 8.0) / n;
    Rect::new(d.x + 4.0 + i as f32 * w, d.y + 26.0, w - 2.0, 21.0)
}

struct PanelDef {
    title: &'static str,
    r: Rect,
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    /// A label over a control (knob, selector, switch).
    Cell,
    /// A selector button or slider alone in its rect, no label.
    Bare,
}

struct Item {
    param: usize,
    kind: Kind,
    r: Rect,
    look: Look,
}

#[derive(Default)]
struct Page {
    panels: Vec<PanelDef>,
    items: Vec<Item>,
}

impl Page {
    fn frame(&mut self, title: &'static str, r: Rect) {
        self.panels.push(PanelDef { title, r });
    }

    /// `params` in cells `cw` wide and one cell high, `cols` to a row from (`x`, `y`); a wave selector takes two cells.
    fn cells(&mut self, x: f32, y: f32, cw: f32, cols: usize, params: &[usize]) {
        let (mut col, mut row) = (0, 0);
        for &p in params {
            let span = if is_wave(p) { 2 } else { 1 };
            if col + span > cols {
                col = 0;
                row += 1;
            }
            self.items.push(Item {
                param: p,
                kind: widget(p),
                r: Rect::new(
                    x + col as f32 * cw,
                    y + row as f32 * CELL_H,
                    span as f32 * cw,
                    CELL_H,
                ),
                look: Look::Cell,
            });
            col += span;
        }
    }

    fn bare(&mut self, param: usize, kind: Kind, r: Rect) {
        self.items.push(Item {
            param,
            kind,
            r,
            look: Look::Bare,
        });
    }
}

fn osc_params(n: usize) -> [usize; 12] {
    use ids::osc::*;
    let b = ids::OSC_BASE + n * ids::OSC_STRIDE;
    [
        b + WAVE,
        b + VOICES,
        b + DETUNE,
        b + STEREO,
        b + VOL,
        b + PAN,
        b + OCT,
        b + SEMI,
        b + FINE,
        b + PHASE,
        b + RAND,
        b + INV,
    ]
}

fn part_params(n: usize) -> [usize; 7] {
    use ids::part::*;
    let b = ids::PART_BASE + n * ids::PART_STRIDE;
    [
        b + ON,
        b + LEVEL,
        b + PAN,
        b + ATT,
        b + DEC,
        b + SUS,
        b + REL,
    ]
}

fn filter_params(n: usize) -> [usize; 11] {
    use ids::filt::*;
    let b = ids::FILT_BASE + n * ids::FILT_STRIDE;
    [
        b + ON,
        b + TYPE,
        b + SLOPE,
        b + CUTOFF,
        b + RES,
        b + DRIVE,
        b + KEY,
        b + IN_A,
        b + IN_B,
        b + VOL,
        b + PAN,
    ]
}

fn env_params(n: usize) -> [usize; 4] {
    let b = ids::ENV_BASE + n * ids::ENV_STRIDE;
    [b, b + 1, b + 2, b + 3]
}

fn lfo_params(n: usize) -> [usize; 8] {
    let b = ids::LFO_BASE + n * ids::LFO_STRIDE;
    [b, b + 1, b + 2, b + 3, b + 4, b + 5, b + 6, b + 7]
}

/// The fixed panels: oscillators, filters, LFOs, envelopes, master.
fn front(g: &mut Page) {
    // The oscillators, two to a part, and the part's amplitude envelope.
    for (part, (title, col)) in [("OSCILLATOR A", 0.0), ("OSCILLATOR B", 15.0)]
        .into_iter()
        .enumerate()
    {
        let x = X0 + col * CW1;
        g.frame(title, Rect::new(x, Y1, 7.0 * CW1, BLOCK1_H));
        for n in 0..2 {
            let y = Y1 + HEAD + n as f32 * 2.0 * CELL_H;
            g.cells(x, y, CW1, 7, &osc_params(part * 2 + n));
        }
        g.cells(x, Y1 + HEAD + 4.0 * CELL_H, CW1, 7, &part_params(part));
    }
    for (n, (title, col)) in [("FILTER A", 0.0), ("FILTER B", 6.0)]
        .into_iter()
        .enumerate()
    {
        let x = X0 + col * CW2;
        g.frame(title, Rect::new(x, Y2, 6.0 * CW2, BLOCK2_H));
        g.cells(x, Y2 + HEAD, CW2, 6, &filter_params(n));
    }
    for (n, (title, col)) in [("LFO 1", 12.0), ("LFO 2", 16.0)].into_iter().enumerate() {
        let x = X0 + col * CW2;
        g.frame(title, Rect::new(x, Y2, 4.0 * CW2, BLOCK2_H));
        g.cells(x, Y2 + HEAD, CW2, 4, &lfo_params(n));
    }
    for (n, (title, col)) in [("ENVELOPE 3", 0.0), ("ENVELOPE 4", 4.0)]
        .into_iter()
        .enumerate()
    {
        let x = X0 + col * CW2;
        g.frame(title, Rect::new(x, Y3, 4.0 * CW2, BLOCK3_H));
        g.cells(x, Y3 + HEAD, CW2, 4, &env_params(n));
    }
    let x = X0 + 8.0 * CW2;
    g.frame("MASTER", Rect::new(x, Y3, 8.0 * CW2, BLOCK3_H));
    g.cells(
        x,
        Y3 + HEAD,
        CW2,
        8,
        &[
            ids::MASTER_VOL,
            ids::MASTER_TUNE,
            ids::POLY,
            ids::MONO,
            ids::PORTA,
            ids::PORTA_MODE,
            ids::BEND,
            ids::VEL_AMP,
        ],
    );
}

/// The display's body for `ui`.
fn display_body(g: &mut Page, ui: &Ui) {
    use ids::*;
    let b = body();
    match ui.tab {
        TAB_MATRIX => {
            for row in 0..4 {
                let slot = ui.sub().min(3) * 4 + row;
                let base = MAT_BASE + slot * MAT_STRIDE;
                let y = b.y + row as f32 * 33.0;
                g.bare(
                    base + mat::SRC,
                    self::Kind::Cycle,
                    Rect::new(b.x, y, 72.0, 31.0),
                );
                g.bare(
                    base + mat::DST,
                    self::Kind::Cycle,
                    Rect::new(b.x + 75.0, y, 72.0, 31.0),
                );
                g.bare(
                    base + mat::AMT,
                    self::Kind::Slider,
                    Rect::new(b.x + 150.0, y, 70.0, 31.0),
                );
            }
        }
        TAB_FX => {
            let sub = ui.sub().min(6);
            let params: Vec<usize> = match sub {
                0 => vec![DIST_ON, DIST_TYPE, DIST_DRIVE, DIST_TONE, DIST_MIX],
                1 => vec![
                    PHS_ON, PHS_RATE, PHS_DEPTH, PHS_FREQ, PHS_FB, PHS_SPREAD, PHS_MIX,
                ],
                2 => vec![CHO_ON, CHO_RATE, CHO_DEPTH, CHO_DELAY, CHO_FB, CHO_MIX],
                3 => vec![EQ_ON, EQ_BASS_F, EQ_BASS_G, EQ_TREB_F, EQ_TREB_G],
                4 => vec![
                    DLY_ON, DLY_SYNC, DLY_L, DLY_R, DLY_DIV_L, DLY_DIV_R, DLY_FB, DLY_PP, DLY_LP,
                    DLY_HP, DLY_SMEAR, DLY_MIX,
                ],
                5 => vec![REV_ON, REV_PRE, REV_DAMP, REV_SIZE, REV_WIDTH, REV_MIX],
                _ => vec![CMP_ON, CMP_THR, CMP_RATIO, CMP_ATT, CMP_REL, CMP_GAIN],
            };
            debug_assert_eq!(params[0], FX_ON[sub]);
            g.cells(b.x, b.y + 4.0, b.w / 8.0, 8, &params);
        }
        _ => {
            g.cells(
                b.x,
                b.y,
                b.w / 8.0,
                8,
                &[
                    ARP_ON,
                    ARP_MODE,
                    ARP_DIV,
                    ARP_OCT,
                    ARP_GATE,
                    ARP_STEPS,
                    ARP_HOLD,
                    ARP_VELMODE,
                ],
            );
            let pitch = b.w / 8.0;
            for i in 0..8 {
                let step = ui.sub().min(1) * 8 + i;
                let x = b.x + i as f32 * pitch;
                g.bare(
                    ARP_STEP_BASE + step,
                    self::Kind::Bar,
                    Rect::new(x, b.y + 37.0, pitch - 2.0, 42.0),
                );
                g.bare(
                    ARP_TR_BASE + step,
                    self::Kind::Bar,
                    Rect::new(x, b.y + 81.0, pitch - 2.0, 28.0),
                );
                g.bare(
                    ARP_HOLD_BASE + step,
                    self::Kind::Toggle,
                    Rect::new(x, b.y + 111.0, pitch - 2.0, 20.0),
                );
            }
        }
    }
}

fn page(ui: &Ui) -> Page {
    let mut g = Page::default();
    front(&mut g);
    g.frame("", display());
    display_body(&mut g, ui);
    g
}

// ---- the drop-down list ----------------------------------------------------------------------------------------

const ITEM_W: f32 = 88.0;

/// The rect of a list of `n` names opened from `anchor`, and of each name: in columns, below the anchor or above it,
/// inside the window.
pub fn popup_rects(anchor: Rect, n: usize) -> (Rect, Vec<Rect>) {
    let ih = if n <= 10 { 26.0 } else { 22.0 };
    let room = H as f32 - BAR_H - 6.0;
    let per_col = ((room / ih).floor() as usize).max(1);
    let cols = n.div_ceil(per_col).max(1);
    let rows = n.div_ceil(cols).max(1);
    let (w, h) = (cols as f32 * ITEM_W, rows as f32 * ih);
    let x = anchor.x.min(W as f32 - w - 3.0).max(3.0);
    let below = anchor.bottom() + 2.0;
    let y = if below + h <= H as f32 - BAR_H - 2.0 {
        below
    } else {
        (anchor.y - h - 2.0).max(3.0)
    };
    let items = (0..n)
        .map(|i| {
            Rect::new(
                x + (i / rows) as f32 * ITEM_W,
                y + (i % rows) as f32 * ih,
                ITEM_W,
                ih,
            )
        })
        .collect();
    (Rect::new(x, y, w, h), items)
}

/// Every hit region of `ui`'s screen, the last on top (no painting). With a list open, only the list and the rest of
/// the window that closes it.
pub fn layout(ui: &Ui) -> Vec<Hit> {
    let pg = page(ui);
    if let Some(p) = ui.popup {
        let anchor = pg.items.iter().find(|i| i.param == p).map(|i| i.r);
        let mut hits = vec![Hit {
            param: 0,
            kind: Kind::Dismiss,
            r: Rect::new(0.0, 0.0, W as f32, H as f32),
        }];
        if let Some(anchor) = anchor {
            let (names, _) = choices(p);
            let (_, rects) = popup_rects(anchor, names.len());
            hits.extend(rects.into_iter().enumerate().map(|(i, r)| Hit {
                param: p,
                kind: Kind::Item(i),
                r,
            }));
        }
        return hits;
    }
    let mut hits = Vec::new();
    for i in 0..TABS.len() {
        hits.push(Hit {
            param: 0,
            kind: Kind::Tab(i),
            r: tab_rect(i),
        });
    }
    for i in 0..subs_of(ui.tab) {
        hits.push(Hit {
            param: 0,
            kind: Kind::Sub(i),
            r: sub_rect(ui.tab, i),
        });
    }
    hits.extend(pg.items.iter().map(|it| Hit {
        param: it.param,
        kind: it.kind,
        r: it.r,
    }));
    hits
}

// ---- painting --------------------------------------------------------------------------------------------------

/// Paints the editor for `ui` with every control at its value in `v`, `pressed` the parameter being dragged and
/// `last` the one touched last (named in the bottom bar). Returns the hit regions, the last on top.
pub fn paint(
    c: &mut Canvas,
    v: &dyn Values,
    ui: &Ui,
    pressed: Option<usize>,
    last: Option<usize>,
) -> Vec<Hit> {
    c.begin_frame(None);
    c.clear(BG);
    c.fill_rect(Rect::new(0.0, 0.0, W as f32, TOP_BAR), PANEL);
    c.fill_rect(Rect::new(0.0, TOP_BAR - 1.0, W as f32, 1.0), EDGE);
    c.text(Font::new(16).bold(), 10.0, 3.0, ORANGE, "RIND");
    c.text(
        LABEL,
        64.0,
        9.0,
        MUTED,
        "subtractive synthesizer for Honeydew",
    );
    let pg = page(ui);
    for p in &pg.panels {
        c.fill_rrect(p.r, 6.0, PANEL);
        c.stroke_rrect(p.r, 6.0, 1.0, EDGE);
        if !p.title.is_empty() {
            c.text(TITLE, p.r.x + 8.0, p.r.y + 2.0, AMBER, p.title);
        }
    }
    paint_display_chrome(c, v, ui);
    for it in &pg.items {
        paint_item(c, v, it, pressed);
    }
    paint_extras(c, v, ui);
    paint_status(c, v, last);
    if let Some(p) = ui.popup
        && let Some(anchor) = pg.items.iter().find(|i| i.param == p).map(|i| i.r)
    {
        paint_popup(c, v, p, anchor);
    }
    layout(ui)
}

fn paint_display_chrome(c: &mut Canvas, v: &dyn Values, ui: &Ui) {
    for (i, name) in TABS.iter().enumerate() {
        let r = tab_rect(i);
        let on = i == ui.tab;
        c.fill_rrect(r, 5.0, if on { ORANGE } else { CAP });
        if !on {
            c.stroke_rrect(r, 5.0, 1.0, EDGE);
        }
        c.text_center(VALUE, r, if on { DARK_TEXT } else { TEXT }, name);
    }
    for i in 0..subs_of(ui.tab) {
        let r = sub_rect(ui.tab, i);
        let on = i == ui.sub();
        c.fill_rrect(r, 5.0, if on { AMBER } else { CAP });
        if !on {
            c.stroke_rrect(r, 5.0, 1.0, EDGE);
        }
        let name = match ui.tab {
            TAB_MATRIX => MATRIX_SUBS[i],
            TAB_FX => FX_SUBS[i],
            _ => ARP_SUBS[i],
        };
        let f = c.fit_font(LABEL, name, r.w - 2.0);
        c.text_center(f, r, if on { DARK_TEXT } else { TEXT }, name);
        if ui.tab == TAB_FX && v.plain(FX_ON[i]) > 0.5 {
            c.fill_circle(
                r.x + 5.0,
                r.y + 5.0,
                2.0,
                if on { DARK_TEXT } else { ORANGE },
            );
        }
    }
}

fn paint_item(c: &mut Canvas, v: &dyn Values, it: &Item, pressed: Option<usize>) {
    let held = pressed == Some(it.param);
    match (it.kind, it.look) {
        (Kind::Knob, _) => paint_knob(c, v, it, held),
        (Kind::Cycle, Look::Cell) => {
            label(c, it);
            let r = Rect::new(it.r.x + 2.0, it.r.y + 12.0, it.r.w - 4.0, 20.0);
            paint_cycle(c, v, it.param, r, held, is_wave(it.param));
        }
        (Kind::Cycle, _) => paint_cycle(c, v, it.param, it.r, held, false),
        (Kind::Toggle, Look::Cell) => {
            label(c, it);
            let r = Rect::new(it.r.x + 3.0, it.r.y + 13.0, it.r.w - 6.0, 18.0);
            paint_switch(c, v, it.param, r);
        }
        (Kind::Toggle, _) => paint_tie(c, v, it),
        (Kind::Slider, _) => paint_slider(c, v, it, held),
        (Kind::Bar, _) => paint_bar(c, v, it, held),
        _ => {}
    }
}

fn label(c: &mut Canvas, it: &Item) {
    let text = short(it.param);
    let f = c.fit_font(LABEL, text, it.r.w - 2.0);
    let w = c.text_width(f, text);
    c.text(f, (it.r.cx() - w / 2.0).round(), it.r.y + 1.0, MUTED, text);
}

fn paint_knob(c: &mut Canvas, v: &dyn Values, it: &Item, held: bool) {
    label(c, it);
    let n = v.norm(it.param).clamp(0.0, 1.0);
    let (cx, cy) = (it.r.cx(), it.r.y + 22.0);
    let (start, sweep) = (0.75 * std::f32::consts::PI, 1.5 * std::f32::consts::PI);
    c.arc(cx, cy, 9.5, start, sweep, 2.5, TRACK);
    if bipolar(it.param) {
        let mid = start + sweep * 0.5;
        let a = start + sweep * n;
        if (a - mid).abs() > 0.01 {
            c.arc(cx, cy, 9.5, mid.min(a), (a - mid).abs(), 2.5, ORANGE);
        }
    } else if n > 0.002 {
        c.arc(cx, cy, 9.5, start, sweep * n, 2.5, ORANGE);
    }
    c.fill_circle(cx, cy, 6.5, if held { EDGE } else { CAP });
    let a = start + sweep * n;
    c.line(
        cx + 2.5 * a.cos(),
        cy + 2.5 * a.sin(),
        cx + 6.5 * a.cos(),
        cy + 6.5 * a.sin(),
        1.6,
        AMBER,
    );
}

fn is_wave(p: usize) -> bool {
    let end = ids::OSC_BASE + 4 * ids::OSC_STRIDE;
    p >= ids::OSC_BASE && p < end && (p - ids::OSC_BASE) % ids::OSC_STRIDE == ids::osc::WAVE
}

/// A small downward chevron at (`x`, `y`).
fn chevron(c: &mut Canvas, x: f32, y: f32) {
    c.polyline(
        &[(x - 3.0, y - 1.5), (x, y + 1.5), (x + 3.0, y - 1.5)],
        1.2,
        MUTED,
    );
}

fn paint_cycle(c: &mut Canvas, v: &dyn Values, p: usize, r: Rect, held: bool, icon: bool) {
    c.fill_rrect(r, 5.0, if held { EDGE } else { CAP });
    c.stroke_rrect(r, 5.0, 1.0, EDGE);
    let text = v.text(p);
    let mut text_r = Rect::new(r.x, r.y, r.w - 8.0, r.h);
    if icon {
        wave_icon(
            c,
            v.plain(p).round() as i32,
            Rect::new(r.x + 4.0, r.y + 4.0, 18.0, r.h - 8.0),
        );
        text_r = Rect::new(r.x + 24.0, r.y, r.w - 32.0, r.h);
    }
    let f = c.fit_font(VALUE, &text, text_r.w - 2.0);
    c.text_center(f, text_r, TEXT, &text);
    if dropdown(p) {
        chevron(c, r.right() - 6.0, r.cy());
    }
}

fn paint_switch(c: &mut Canvas, v: &dyn Values, p: usize, r: Rect) {
    let on = v.plain(p) > 0.5;
    c.fill_rrect(r, 5.0, if on { ORANGE } else { CAP });
    c.stroke_rrect(r, 5.0, 1.0, if on { ORANGE } else { EDGE });
    c.text_center(
        VALUE,
        r,
        if on { DARK_TEXT } else { MUTED },
        if on { "ON" } else { "OFF" },
    );
}

fn paint_tie(c: &mut Canvas, v: &dyn Values, it: &Item) {
    let on = v.plain(it.param) > 0.5;
    c.fill_rrect(it.r, 4.0, if on { AMBER } else { CAP });
    c.stroke_rrect(it.r, 4.0, 1.0, if on { AMBER } else { EDGE });
}

fn paint_slider(c: &mut Canvas, v: &dyn Values, it: &Item, held: bool) {
    let n = v.norm(it.param).clamp(0.0, 1.0);
    let r = it.r;
    let track = Rect::new(r.x + 6.0, r.cy() - 2.5, r.w - 12.0, 5.0);
    c.fill_rrect(track, 2.5, TRACK);
    let x = track.x + n * track.w;
    if bipolar(it.param) {
        let mid = track.x + track.w * 0.5;
        let (a, b) = if x < mid { (x, mid) } else { (mid, x) };
        c.fill_rrect(
            Rect::new(a, track.y, (b - a).max(1.0), track.h),
            2.5,
            ORANGE,
        );
        c.fill_rect(
            Rect::new(mid - 0.5, track.y - 3.0, 1.0, track.h + 6.0),
            MUTED,
        );
    } else {
        c.fill_rrect(
            Rect::new(track.x, track.y, (x - track.x).max(1.0), track.h),
            2.5,
            ORANGE,
        );
    }
    let knob = Rect::new(x - 5.0, r.cy() - 9.0, 10.0, 18.0);
    c.fill_rrect(knob, 4.0, if held { ORANGE } else { CAP });
    c.stroke_rrect(knob, 4.0, 1.5, ORANGE);
}

fn paint_bar(c: &mut Canvas, v: &dyn Values, it: &Item, held: bool) {
    let n = v.norm(it.param).clamp(0.0, 1.0);
    let r = it.r;
    c.fill_rrect(r, 3.0, TRACK);
    let live = step_of(it.param).is_none_or(|s| (s as f32) < v.plain(ids::ARP_STEPS));
    let col = if live { ORANGE } else { MUTED };
    let y = r.y + 2.0 + (1.0 - n) * (r.h - 4.0);
    if bipolar(it.param) {
        let mid = r.y + r.h * 0.5;
        let (a, b) = if y < mid { (y, mid) } else { (mid, y) };
        c.fill_rect(Rect::new(r.x + 2.0, a, r.w - 4.0, (b - a).max(1.0)), col);
        c.fill_rect(Rect::new(r.x, mid - 0.5, r.w, 1.0), MUTED);
    } else {
        c.fill_rect(
            Rect::new(r.x + 2.0, y, r.w - 4.0, r.bottom() - 2.0 - y),
            col,
        );
    }
    if held {
        c.stroke_rrect(r, 3.0, 1.5, AMBER);
    }
}

/// The step a per-step parameter belongs to (0-based).
fn step_of(p: usize) -> Option<usize> {
    [ids::ARP_STEP_BASE, ids::ARP_TR_BASE, ids::ARP_HOLD_BASE]
        .iter()
        .find(|&&b| p >= b && p < b + 16)
        .map(|&b| p - b)
}

fn paint_popup(c: &mut Canvas, v: &dyn Values, p: usize, anchor: Rect) {
    c.fill_rect(Rect::new(0.0, 0.0, W as f32, H as f32), SHADE);
    let (names, lo) = choices(p);
    let (all, rects) = popup_rects(anchor, names.len());
    c.fill_rrect(all.inset(-2.0), 6.0, EDGE);
    c.fill_rrect(all, 5.0, PANEL);
    let now = v.plain(p).round() as i32 - lo;
    for (i, (r, name)) in rects.iter().zip(names).enumerate() {
        let on = i as i32 == now;
        if on {
            c.fill_rrect(r.inset(1.0), 4.0, ORANGE);
        }
        let f = c.fit_font(VALUE, name, r.w - 8.0);
        c.text_center(f, *r, if on { DARK_TEXT } else { TEXT }, name);
    }
}

fn wave_icon(c: &mut Canvas, wave: i32, r: Rect) {
    let pts: Vec<(f32, f32)> = (0..=18)
        .map(|i| {
            let t = i as f32 / 18.0;
            (r.x + t * r.w, r.cy() - wave_shape(wave, t) * r.h * 0.45)
        })
        .collect();
    c.polyline(&pts, 1.3, AMBER);
}

/// One cycle of oscillator wave `wave` (see [`ids::WAVE_NAMES`]) at phase `t` in `[0, 1]`, for the icons.
fn wave_shape(wave: i32, t: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let tri = 1.0 - 4.0 * ((t + 0.25).rem_euclid(1.0) - 0.5).abs();
    let pulse = |width: f32| if t < width { 1.0 } else { -1.0 };
    match wave {
        0 => (t * tau).sin(),
        1 => 1.0 - 2.0 * t,
        2 => tri,
        3 => pulse(0.5),
        4 => pulse(0.25),
        5 => pulse(0.125),
        6 => 0.5 * (1.0 - 2.0 * t) + 0.5 * tri,
        _ => (((t * 7919.0).sin() * 43758.5).fract() * 2.0 - 1.0).clamp(-1.0, 1.0),
    }
}

/// The magnitude response (dB) of Rind's state-variable filter of `mode` (0 low, 1 band, 2 high) at `hz`.
fn filter_db(mode: i32, slope24: bool, cutoff: f32, res: f32, hz: f32) -> f32 {
    let k = crate::filter::damping(res);
    let r = hz / cutoff.max(1.0);
    let den = ((1.0 - r * r).powi(2) + (k * r).powi(2)).sqrt().max(1e-6);
    let mag = match mode {
        0 => 1.0 / den,
        1 => k * r / den,
        _ => r * r / den,
    };
    let db = 20.0 * mag.max(1e-6).log10();
    if slope24 { db * 2.0 } else { db }
}

/// The filter response panel, at the bottom right.
fn response_rect() -> Rect {
    Rect::new(X0 + 16.0 * CW2, Y3, 4.0 * CW2, BLOCK3_H)
}

fn paint_extras(c: &mut Canvas, v: &dyn Values, ui: &Ui) {
    let r = response_rect();
    c.fill_rrect(r, 6.0, PANEL);
    c.stroke_rrect(r, 6.0, 1.0, EDGE);
    c.text(TITLE, r.x + 8.0, r.y + 2.0, AMBER, "RESPONSE");
    let g = Rect::new(r.x + 6.0, r.y + HEAD, r.w - 12.0, r.h - HEAD - 4.0);
    for (n, col) in [(0usize, ORANGE), (1, AMBER)] {
        let b = ids::FILT_BASE + n * ids::FILT_STRIDE;
        if v.plain(b + ids::filt::ON) < 0.5 {
            continue;
        }
        let (mode, slope) = (
            v.plain(b + ids::filt::TYPE).round() as i32,
            v.plain(b + ids::filt::SLOPE) > 0.5,
        );
        let (cut, res) = (v.plain(b + ids::filt::CUTOFF), v.plain(b + ids::filt::RES));
        let pts: Vec<(f32, f32)> = (0..=64)
            .map(|i| {
                let t = i as f32 / 64.0;
                let hz = 20.0 * 1000.0f32.powf(t);
                let db = filter_db(mode, slope, cut, res, hz).clamp(-40.0, 24.0);
                (g.x + t * g.w, g.bottom() - (db + 40.0) / 64.0 * g.h)
            })
            .collect();
        c.polyline(&pts, 1.5, col);
    }
    if ui.tab == TAB_ARP {
        let b = body();
        let pitch = b.w / 8.0;
        for i in 0..8 {
            let n = format!("{}", ui.sub().min(1) * 8 + i + 1);
            c.text_center(
                LABEL,
                Rect::new(b.x + i as f32 * pitch, b.y + 132.0, pitch - 2.0, 8.0),
                MUTED,
                &n,
            );
        }
    }
}

fn paint_status(c: &mut Canvas, v: &dyn Values, last: Option<usize>) {
    let r = Rect::new(0.0, H as f32 - BAR_H, W as f32, BAR_H);
    c.fill_rect(r, PANEL);
    c.fill_rect(Rect::new(0.0, r.y, W as f32, 1.0), EDGE);
    if let Some(p) = last {
        c.text(LABEL, 10.0, r.y + 6.0, MUTED, SPECS[p].name);
        c.text_right(VALUE, W as f32 - 10.0, r.y + 5.0, TEXT, &v.text(p));
    } else {
        c.text(
            LABEL,
            10.0,
            r.y + 6.0,
            MUTED,
            "Touch a control to see its value",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every screen: each tab and sub tab.
    fn screens() -> Vec<Ui> {
        let mut out = Vec::new();
        for tab in 0..TABS.len() {
            for sub in 0..subs_of(tab) {
                let mut ui = Ui {
                    tab,
                    ..Ui::default()
                };
                ui.subs[tab] = sub;
                out.push(ui);
            }
        }
        out
    }

    fn window() -> Rect {
        Rect::new(0.0, 0.0, W as f32, H as f32 - BAR_H)
    }

    fn inside(win: Rect, r: Rect) -> bool {
        let i = win.intersect(&r);
        (i.w - r.w).abs() < 0.01 && (i.h - r.h).abs() < 0.01
    }

    #[test]
    fn every_screen_lays_out_inside_the_window_without_overlap() {
        for ui in screens() {
            let hits = layout(&ui);
            for (i, a) in hits.iter().enumerate() {
                assert!(inside(window(), a.r), "{ui:?}: {a:?} leaves the window");
                for b in &hits[i + 1..] {
                    let i = a.r.intersect(&b.r);
                    assert!(i.w < 0.01 || i.h < 0.01, "{ui:?}: {a:?} overlaps {b:?}");
                }
            }
        }
    }

    #[test]
    fn controls_are_finger_sized() {
        for ui in screens() {
            for h in layout(&ui) {
                let (w, hh) = match h.kind {
                    Kind::Bar | Kind::Toggle if step_of(h.param).is_some() => (24.0, 18.0),
                    Kind::Slider => (60.0, 28.0),
                    Kind::Tab(_) | Kind::Sub(_) => (28.0, 20.0),
                    _ => (28.0, 30.0),
                };
                assert!(h.r.w >= w && h.r.h >= hh, "{ui:?}: {h:?}");
            }
        }
    }

    #[test]
    fn every_parameter_has_a_control_on_some_screen() {
        let mut seen = vec![false; ids::COUNT];
        for ui in screens() {
            for h in layout(&ui) {
                if h.is_param() {
                    seen[h.param] = true;
                }
            }
        }
        let missing: Vec<_> = (0..ids::COUNT)
            .filter(|&i| !seen[i])
            .map(|i| SPECS[i].id)
            .collect();
        assert!(missing.is_empty(), "no control for {missing:?}");
    }

    #[test]
    fn no_parameter_has_two_controls_on_one_screen() {
        for ui in screens() {
            let mut seen = std::collections::HashSet::new();
            for h in layout(&ui).iter().filter(|h| h.is_param()) {
                assert!(
                    seen.insert(h.param),
                    "{ui:?}: {} is twice",
                    SPECS[h.param].id
                );
            }
        }
    }

    #[test]
    fn every_drop_down_list_fits_the_window_and_holds_every_name() {
        for ui in screens() {
            for h in layout(&ui)
                .iter()
                .filter(|h| h.is_param() && dropdown(h.param))
            {
                let open = Ui {
                    popup: Some(h.param),
                    ..ui
                };
                let hits = layout(&open);
                let items: Vec<_> = hits
                    .iter()
                    .filter(|i| matches!(i.kind, Kind::Item(_)))
                    .collect();
                let (names, _) = choices(h.param);
                assert_eq!(items.len(), names.len(), "{}", SPECS[h.param].id);
                for (k, a) in items.iter().enumerate() {
                    assert!(inside(window(), a.r), "{}: {a:?}", SPECS[h.param].id);
                    assert!(a.r.w >= 80.0 && a.r.h >= 22.0, "{a:?}");
                    for b in &items[k + 1..] {
                        let i = a.r.intersect(&b.r);
                        assert!(i.w < 0.01 || i.h < 0.01, "{a:?} overlaps {b:?}");
                    }
                }
                // Anywhere else closes it.
                assert_eq!(hits[0].kind, Kind::Dismiss);
            }
        }
    }

    #[test]
    fn selectors_with_many_names_are_drop_downs_and_short_ones_step() {
        assert!(dropdown(ids::MAT_BASE + ids::mat::SRC));
        assert!(dropdown(ids::MAT_BASE + ids::mat::DST));
        assert!(dropdown(ids::OSC_BASE + ids::osc::WAVE));
        assert!(dropdown(ids::ARP_MODE));
        assert!(!dropdown(ids::MONO));
        assert!(!dropdown(ids::FILT_BASE + ids::filt::SLOPE));
        assert!(!dropdown(ids::MASTER_VOL));
        assert!(!dropdown(ids::ARP_ON));
    }

    #[test]
    fn widgets_follow_the_spec() {
        assert_eq!(widget(ids::ARP_ON), Kind::Toggle);
        assert_eq!(widget(ids::ARP_MODE), Kind::Cycle);
        assert_eq!(widget(ids::MASTER_VOL), Kind::Knob);
        assert!(is_int(ids::POLY));
        assert!(bipolar(ids::MASTER_TUNE));
        assert!(!bipolar(ids::MASTER_VOL));
    }

    #[test]
    fn labels_are_short() {
        for p in (0..ids::COUNT)
            .filter(|&p| step_of(p).is_none() && !(ids::MAT_BASE..ids::ARP_ON).contains(&p))
        {
            if widget(p) == Kind::Knob || widget(p) == Kind::Toggle {
                assert!(
                    short(p).chars().count() <= 9,
                    "{}: {}",
                    SPECS[p].id,
                    short(p)
                );
            }
        }
        assert_eq!(short(ids::MASTER_VOL), "Master");
        assert_eq!(short(ids::OSC_BASE + ids::osc::DETUNE), "Detune");
        assert_eq!(short(ids::DLY_PP), "Ping");
    }

    #[test]
    fn slider_and_bar_positions_map_to_the_range() {
        let r = Rect::new(100.0, 10.0, 200.0, 30.0);
        assert_eq!(slider_norm(r, 0.0), 0.0);
        assert_eq!(slider_norm(r, 1000.0), 1.0);
        assert!((slider_norm(r, 200.0) - 0.5).abs() < 1e-6);
        let b = Rect::new(0.0, 100.0, 30.0, 80.0);
        assert_eq!(bar_norm(b, 100.0), 1.0);
        assert_eq!(bar_norm(b, 180.0), 0.0);
    }

    #[test]
    fn the_filter_curve_peaks_with_resonance_and_falls_off() {
        let flat = filter_db(0, true, 1000.0, 0.0, 100.0);
        assert!(flat.abs() < 2.0, "{flat}");
        assert!(filter_db(0, true, 1000.0, 0.0, 8000.0) < -30.0);
        assert!(filter_db(0, false, 1000.0, 0.9, 1000.0) > 6.0);
        assert!(filter_db(2, false, 1000.0, 0.0, 50.0) < -20.0);
    }

    /// The defaults, as the editor would show them.
    struct Defaults;

    impl Values for Defaults {
        fn norm(&self, p: usize) -> f32 {
            crate::ranges::to_norm(p, SPECS[p].default)
        }
        fn plain(&self, p: usize) -> f32 {
            SPECS[p].default
        }
        fn text(&self, p: usize) -> String {
            format!("{}", SPECS[p].default)
        }
    }

    #[test]
    fn every_screen_paints_and_reports_the_regions_it_lays_out() {
        let mut c = Canvas::new(W, H, 1.0).unwrap();
        for ui in screens() {
            assert_eq!(paint(&mut c, &Defaults, &ui, Some(0), Some(1)), layout(&ui));
            let open = Ui {
                popup: Some(ids::MAT_BASE + ids::mat::DST),
                ..ui
            };
            if ui.tab == TAB_MATRIX {
                assert_eq!(paint(&mut c, &Defaults, &open, None, None), layout(&open));
            }
        }
    }
}

// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Rind's picture and its hit regions, in logical pixels: an original dark panel with warm orange accents, laid out in
//! tabs (oscillators, filters, modulation, matrix, effects, arpeggiator, main) at 16:9 and sized for fingers.
//!
//! Everything is driven by the parameter table ([`crate::ids::SPECS`]): a control's kind follows from its spec
//! (a named range is a selector, a two-step Off/On range a switch, anything else a knob) and its text comes from the
//! parameter itself, so a page is just a list of parameter indices.

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

const LABEL: Font = Font::new(10);
const VALUE: Font = Font::new(10).bold();
const SMALL: Font = Font::new(9);
const TITLE: Font = Font::new(11).bold();

pub const TABS: [&str; 8] = [
    "OSC A", "OSC B", "FILTER", "MOD", "MATRIX", "FX", "ARP", "MAIN",
];
pub const TAB_OSC_A: usize = 0;
pub const TAB_OSC_B: usize = 1;
pub const TAB_FILTER: usize = 2;
pub const TAB_MOD: usize = 3;
pub const TAB_MATRIX: usize = 4;
pub const TAB_FX: usize = 5;
pub const TAB_ARP: usize = 6;
pub const TAB_MAIN: usize = 7;

const MATRIX_SUBS: [&str; 2] = ["SLOTS 1-8", "SLOTS 9-16"];
const FX_SUBS: [&str; 7] = [
    "DISTORTION",
    "PHASER",
    "CHORUS",
    "EQ",
    "DELAY",
    "REVERB",
    "COMPRESSOR",
];
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

/// The pages the editor is showing: the tab and, for the pages that have them, the sub tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ui {
    pub tab: usize,
    pub subs: [usize; 8],
}

impl Ui {
    pub fn sub(&self) -> usize {
        self.subs[self.tab.min(7)]
    }
}

/// What a hit region is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Drag vertically (the cursor held).
    Knob,
    /// A named range: a tap steps to the next name.
    Cycle,
    Toggle,
    /// A horizontal slider: a press sets, a drag moves.
    Slider,
    /// A vertical bar: a press sets, a drag moves.
    Bar,
    Tab(usize),
    Sub(usize),
}

/// A control's parameter index (into the table), kind and rect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub param: usize,
    pub kind: Kind,
    pub r: Rect,
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

/// The normalised value a press at `x` sets on a slider in `r`.
pub fn slider_norm(r: Rect, x: f32) -> f32 {
    ((x - r.x - 6.0) / (r.w - 12.0)).clamp(0.0, 1.0)
}

/// The normalised value a press at `y` sets on a bar in `r`.
pub fn bar_norm(r: Rect, y: f32) -> f32 {
    (1.0 - (y - r.y - 2.0) / (r.h - 4.0)).clamp(0.0, 1.0)
}

// ---- layout ----------------------------------------------------------------------------------------------------

const CELL_W: f32 = 62.0;
const CELL_H: f32 = 68.0;
const X0: f32 = 10.0;
const HEAD: f32 = 22.0;
const TOP: f32 = 44.0;
const SUB_Y: f32 = 40.0;
const TOP_SUB: f32 = 70.0;
const BAR_H: f32 = 22.0;

/// A panel: its title, rect and the switch in its header.
struct PanelDef {
    title: String,
    r: Rect,
    toggle: Option<usize>,
}

/// How a cell looks.
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
    label: String,
}

#[derive(Default)]
struct Page {
    panels: Vec<PanelDef>,
    items: Vec<Item>,
}

impl Page {
    /// A panel of `cols` x `rows` cells at column `col` and pixel row `y`, holding `params` row by row. `prefix` is
    /// stripped from each name for its label.
    fn panel(
        &mut self,
        title: &str,
        col: f32,
        y: f32,
        cols: usize,
        rows: usize,
        toggle: Option<usize>,
        prefix: &str,
        params: &[usize],
    ) {
        let x = X0 + col * CELL_W;
        let r = Rect::new(
            x + 2.0,
            y,
            cols as f32 * CELL_W - 4.0,
            HEAD + rows as f32 * CELL_H + 4.0,
        );
        self.panels.push(PanelDef {
            title: title.to_owned(),
            r,
            toggle,
        });
        for (i, &p) in params.iter().enumerate() {
            let (cx, cy) = ((i % cols) as f32, (i / cols) as f32);
            let cell = Rect::new(x + cx * CELL_W, y + HEAD + cy * CELL_H, CELL_W, CELL_H);
            let name = SPECS[p].name;
            let label = name.strip_prefix(prefix).unwrap_or(name).to_owned();
            self.items.push(Item {
                param: p,
                kind: widget(p),
                r: cell,
                look: Look::Cell,
                label,
            });
        }
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
        b + OCT,
        b + SEMI,
        b + FINE,
        b + PAN,
        b + VOL,
        b + PHASE,
        b + RAND,
        b + INV,
    ]
}

fn part_params(n: usize) -> [usize; 6] {
    use ids::part::*;
    let b = ids::PART_BASE + n * ids::PART_STRIDE;
    [b + LEVEL, b + PAN, b + ATT, b + DEC, b + SUS, b + REL]
}

fn filter_params(n: usize) -> [usize; 10] {
    use ids::filt::*;
    let b = ids::FILT_BASE + n * ids::FILT_STRIDE;
    [
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

fn osc_page(part: usize) -> Page {
    let mut g = Page::default();
    let (a, b) = (part * 2, part * 2 + 1);
    let letter = if part == 0 { "A" } else { "B" };
    let prefix = |n: usize| format!("Osc {letter}{} ", n - part * 2 + 1);
    g.panel(
        &format!("OSCILLATOR {letter}1"),
        0.0,
        TOP,
        4,
        3,
        None,
        &prefix(a),
        &osc_params(a),
    );
    g.panel(
        &format!("OSCILLATOR {letter}2"),
        4.0,
        TOP,
        4,
        3,
        None,
        &prefix(b),
        &osc_params(b),
    );
    let on = ids::PART_BASE + part * ids::PART_STRIDE + ids::part::ON;
    g.panel(
        &format!("PART {letter}"),
        8.0,
        TOP,
        2,
        3,
        Some(on),
        &format!("Part {letter} "),
        &part_params(part),
    );
    g
}

fn filter_page() -> Page {
    let mut g = Page::default();
    for n in 0..2 {
        let on = ids::FILT_BASE + n * ids::FILT_STRIDE + ids::filt::ON;
        g.panel(
            &format!("FILTER {}", n + 1),
            n as f32 * 5.0,
            TOP,
            5,
            2,
            Some(on),
            &format!("Filter {} ", n + 1),
            &filter_params(n),
        );
    }
    g
}

fn mod_page() -> Page {
    let mut g = Page::default();
    for n in 0..2 {
        g.panel(
            &format!("ENVELOPE {}", n + 3),
            n as f32 * 4.0,
            TOP,
            4,
            1,
            None,
            &format!("Env {} ", n + 3),
            &env_params(n),
        );
        g.panel(
            &format!("LFO {}", n + 1),
            n as f32 * 4.0,
            TOP + HEAD + CELL_H + 8.0,
            4,
            2,
            None,
            &format!("LFO {} ", n + 1),
            &lfo_params(n),
        );
    }
    g
}

fn main_page() -> Page {
    let mut g = Page::default();
    g.panel(
        "VOICE AND OUTPUT",
        0.0,
        TOP,
        8,
        1,
        None,
        "",
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
    g
}

fn fx_page(sub: usize) -> Page {
    use ids::*;
    let mut g = Page::default();
    let (prefix, params): (&str, Vec<usize>) = match sub {
        0 => (
            "Distortion ",
            vec![DIST_TYPE, DIST_DRIVE, DIST_TONE, DIST_MIX],
        ),
        1 => (
            "Phaser ",
            vec![PHS_RATE, PHS_DEPTH, PHS_FREQ, PHS_FB, PHS_SPREAD, PHS_MIX],
        ),
        2 => (
            "Chorus ",
            vec![CHO_RATE, CHO_DEPTH, CHO_DELAY, CHO_FB, CHO_MIX],
        ),
        3 => ("EQ ", vec![EQ_BASS_F, EQ_BASS_G, EQ_TREB_F, EQ_TREB_G]),
        4 => (
            "Delay ",
            vec![
                DLY_SYNC, DLY_L, DLY_R, DLY_DIV_L, DLY_DIV_R, DLY_FB, DLY_PP, DLY_LP, DLY_HP,
                DLY_SMEAR, DLY_MIX,
            ],
        ),
        5 => (
            "Reverb ",
            vec![REV_PRE, REV_DAMP, REV_SIZE, REV_WIDTH, REV_MIX],
        ),
        _ => (
            "Compressor ",
            vec![CMP_THR, CMP_RATIO, CMP_ATT, CMP_REL, CMP_GAIN],
        ),
    };
    let rows = params.len().div_ceil(10).max(1);
    g.panel(
        FX_SUBS[sub],
        0.0,
        TOP_SUB,
        10,
        rows,
        Some(FX_ON[sub]),
        prefix,
        &params,
    );
    g
}

fn matrix_page(sub: usize) -> Page {
    let mut g = Page::default();
    for row in 0..8 {
        let slot = sub * 8 + row;
        let b = ids::MAT_BASE + slot * ids::MAT_STRIDE;
        let y = TOP_SUB + row as f32 * 31.0;
        let mut item = |param, kind, x: f32, w: f32| {
            g.items.push(Item {
                param,
                kind,
                r: Rect::new(x, y, w, 29.0),
                look: Look::Bare,
                label: String::new(),
            });
        };
        item(b + ids::mat::SRC, Kind::Cycle, 40.0, 160.0);
        item(b + ids::mat::DST, Kind::Cycle, 206.0, 160.0);
        item(b + ids::mat::AMT, Kind::Slider, 372.0, 258.0);
    }
    g
}

// ---- the arpeggiator page's own geometry -------------------------------------------------------------------------

const STEP_X: f32 = 62.0;
const STEP_PITCH: f32 = 35.0;
const ARP_PANEL_Y: f32 = TOP;
const STEPS_Y: f32 = TOP + HEAD + CELL_H + 8.0;
const VEL_Y: f32 = STEPS_Y + HEAD;
const VEL_H: f32 = 76.0;
const TR_Y: f32 = VEL_Y + VEL_H + 4.0;
const TR_H: f32 = 40.0;
const TIE_Y: f32 = TR_Y + TR_H + 4.0;
const TIE_H: f32 = 26.0;

fn arp_page() -> Page {
    use ids::*;
    let mut g = Page::default();
    g.panel(
        "ARPEGGIATOR",
        0.0,
        ARP_PANEL_Y,
        10,
        1,
        Some(ARP_ON),
        "Arp ",
        &[
            ARP_MODE,
            ARP_DIV,
            ARP_OCT,
            ARP_GATE,
            ARP_STEPS,
            ARP_HOLD,
            ARP_VELMODE,
        ],
    );
    g.panels.push(PanelDef {
        title: "STEP PATTERN".to_owned(),
        r: Rect::new(
            X0 + 2.0,
            STEPS_Y,
            10.0 * CELL_W - 4.0,
            HEAD + VEL_H + TR_H + TIE_H + 28.0,
        ),
        toggle: None,
    });
    for i in 0..16 {
        let x = STEP_X + i as f32 * STEP_PITCH;
        let mut item = |param, y, h| {
            g.items.push(Item {
                param,
                kind: if widget(param) == self::Kind::Toggle {
                    self::Kind::Toggle
                } else {
                    self::Kind::Bar
                },
                r: Rect::new(x, y, STEP_PITCH - 3.0, h),
                look: Look::Bare,
                label: String::new(),
            });
        };
        item(ARP_STEP_BASE + i, VEL_Y, VEL_H);
        item(ARP_TR_BASE + i, TR_Y, TR_H);
        item(ARP_HOLD_BASE + i, TIE_Y, TIE_H);
    }
    g
}

fn page(ui: &Ui) -> Page {
    match ui.tab {
        TAB_OSC_A => osc_page(0),
        TAB_OSC_B => osc_page(1),
        TAB_FILTER => filter_page(),
        TAB_MOD => mod_page(),
        TAB_MATRIX => matrix_page(ui.sub().min(1)),
        TAB_FX => fx_page(ui.sub().min(6)),
        TAB_ARP => arp_page(),
        _ => main_page(),
    }
}

/// Every hit region of `ui`'s page, the last one on top (no painting).
pub fn layout(ui: &Ui) -> Vec<Hit> {
    let mut hits = chrome_hits(ui);
    let pg = page(ui);
    for p in &pg.panels {
        if let Some(t) = p.toggle {
            hits.push(Hit {
                param: t,
                kind: Kind::Toggle,
                r: toggle_rect(p.r),
            });
        }
    }
    for it in &pg.items {
        hits.push(Hit {
            param: it.param,
            kind: it.kind,
            r: it.r,
        });
    }
    hits
}

fn tab_rect(i: usize) -> Rect {
    Rect::new(96.0 + i as f32 * 67.0, 5.0, 64.0, 30.0)
}

fn sub_rect(ui_tab: usize, i: usize) -> Rect {
    let w = if ui_tab == TAB_FX { 86.0 } else { 100.0 };
    Rect::new(X0 + i as f32 * (w + 4.0), SUB_Y, w, 26.0)
}

fn subs_of(tab: usize) -> usize {
    match tab {
        TAB_MATRIX => 2,
        TAB_FX => 7,
        _ => 0,
    }
}

fn chrome_hits(ui: &Ui) -> Vec<Hit> {
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
    hits
}

/// A panel's header switch.
fn toggle_rect(panel: Rect) -> Rect {
    Rect::new(panel.right() - 56.0, panel.y, 56.0, HEAD)
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
    paint_chrome(c, v, ui);
    let pg = page(ui);
    for p in &pg.panels {
        paint_panel(c, v, p, pressed);
    }
    for it in &pg.items {
        paint_item(c, v, it, pressed);
    }
    paint_extras(c, v, ui);
    paint_status(c, v, last);
    layout(ui)
}

fn paint_chrome(c: &mut Canvas, v: &dyn Values, ui: &Ui) {
    c.fill_rect(Rect::new(0.0, 0.0, W as f32, 40.0), PANEL);
    c.fill_rect(Rect::new(0.0, 39.0, W as f32, 1.0), EDGE);
    c.text(Font::new(20).bold(), 12.0, 8.0, ORANGE, "RIND");
    for (i, name) in TABS.iter().enumerate() {
        let r = tab_rect(i);
        let on = i == ui.tab;
        c.fill_rrect(r, 7.0, if on { ORANGE } else { CAP });
        if !on {
            c.stroke_rrect(r, 7.0, 1.0, EDGE);
        }
        c.text_center(VALUE, r, if on { DARK_TEXT } else { TEXT }, name);
    }
    for i in 0..subs_of(ui.tab) {
        let r = sub_rect(ui.tab, i);
        let on = i == ui.sub();
        c.fill_rrect(r, 6.0, if on { AMBER } else { CAP });
        if !on {
            c.stroke_rrect(r, 6.0, 1.0, EDGE);
        }
        let name = if ui.tab == TAB_FX {
            FX_SUBS[i]
        } else {
            MATRIX_SUBS[i]
        };
        c.text_center(SMALL, r, if on { DARK_TEXT } else { TEXT }, name);
        if ui.tab == TAB_FX && v.plain(FX_ON[i]) > 0.5 {
            c.fill_circle(
                r.x + 8.0,
                r.y + 6.0,
                2.5,
                if on { DARK_TEXT } else { ORANGE },
            );
        }
    }
}

fn paint_panel(c: &mut Canvas, v: &dyn Values, p: &PanelDef, pressed: Option<usize>) {
    c.fill_rrect(p.r, 8.0, PANEL);
    c.stroke_rrect(p.r, 8.0, 1.0, EDGE);
    c.text(TITLE, p.r.x + 10.0, p.r.y + 5.0, AMBER, &p.title);
    if let Some(t) = p.toggle {
        let on = v.plain(t) > 0.5;
        let r = toggle_rect(p.r).inset(3.0);
        c.fill_rrect(r, 6.0, if on { ORANGE } else { CAP });
        c.stroke_rrect(r, 6.0, 1.0, if on { ORANGE } else { EDGE });
        c.text_center(
            SMALL,
            r,
            if on { DARK_TEXT } else { MUTED },
            if on { "ON" } else { "OFF" },
        );
        let _ = pressed;
    }
}

fn paint_item(c: &mut Canvas, v: &dyn Values, it: &Item, pressed: Option<usize>) {
    let held = pressed == Some(it.param);
    match (it.kind, it.look) {
        (Kind::Knob, _) => paint_knob(c, v, it, held),
        (Kind::Cycle, Look::Cell) => {
            label(c, it);
            let r = Rect::new(it.r.x + 3.0, it.r.y + 15.0, it.r.w - 6.0, 48.0);
            paint_cycle(c, v, it.param, r, held, true);
        }
        (Kind::Cycle, _) => paint_cycle(c, v, it.param, it.r, held, false),
        (Kind::Toggle, Look::Cell) => {
            label(c, it);
            let r = Rect::new(it.r.x + 6.0, it.r.y + 22.0, it.r.w - 12.0, 26.0);
            paint_switch(c, v, it.param, r);
        }
        (Kind::Toggle, _) => paint_tie(c, v, it),
        (Kind::Slider, _) => paint_slider(c, v, it, held),
        (Kind::Bar, _) => paint_bar(c, v, it, held),
        (Kind::Tab(_) | Kind::Sub(_), _) => {}
    }
}

fn label(c: &mut Canvas, it: &Item) {
    let f = c.fit_font(LABEL, &it.label, it.r.w - 4.0);
    let w = c.text_width(f, &it.label);
    c.text(
        f,
        (it.r.cx() - w / 2.0).round(),
        it.r.y + 2.0,
        MUTED,
        &it.label,
    );
}

fn paint_knob(c: &mut Canvas, v: &dyn Values, it: &Item, held: bool) {
    label(c, it);
    let n = v.norm(it.param).clamp(0.0, 1.0);
    let (cx, cy) = (it.r.cx(), it.r.y + 33.0);
    let (start, sweep) = (0.75 * std::f32::consts::PI, 1.5 * std::f32::consts::PI);
    c.arc(cx, cy, 18.0, start, sweep, 4.0, TRACK);
    if bipolar(it.param) {
        let mid = start + sweep * 0.5;
        let a = start + sweep * n;
        if (a - mid).abs() > 0.01 {
            c.arc(cx, cy, 18.0, mid.min(a), (a - mid).abs(), 4.0, ORANGE);
        }
    } else if n > 0.002 {
        c.arc(cx, cy, 18.0, start, sweep * n, 4.0, ORANGE);
    }
    c.fill_circle(cx, cy, 12.5, if held { EDGE } else { CAP });
    let a = start + sweep * n;
    c.line(
        cx + 5.0 * a.cos(),
        cy + 5.0 * a.sin(),
        cx + 12.0 * a.cos(),
        cy + 12.0 * a.sin(),
        2.5,
        AMBER,
    );
    let text = v.text(it.param);
    let f = c.fit_font(VALUE, &text, it.r.w - 2.0);
    let w = c.text_width(f, &text);
    c.text(f, (cx - w / 2.0).round(), it.r.y + 54.0, TEXT, &text);
}

fn is_wave(p: usize) -> bool {
    matches!(SPECS[p].fmt, Fmt::Names(n) if std::ptr::eq(n, ids::WAVE_NAMES))
}

fn paint_cycle(c: &mut Canvas, v: &dyn Values, p: usize, r: Rect, held: bool, tall: bool) {
    c.fill_rrect(r, 7.0, if held { EDGE } else { CAP });
    c.stroke_rrect(r, 7.0, 1.0, EDGE);
    let text = v.text(p);
    let mut text_r = r;
    if tall && is_wave(p) {
        let icon = Rect::new(r.x + 8.0, r.y + 5.0, r.w - 16.0, 22.0);
        wave_icon(c, v.plain(p).round() as i32, icon);
        text_r = Rect::new(r.x, r.y + 28.0, r.w, r.h - 28.0);
    }
    let f = c.fit_font(VALUE, &text, text_r.w - 8.0);
    c.text_center(f, text_r, TEXT, &text);
    // A hint that the button steps.
    if !tall {
        c.text(
            SMALL,
            r.right() - 11.0,
            r.y + r.h / 2.0 - 5.0,
            MUTED,
            "\u{203a}",
        );
    }
}

fn paint_switch(c: &mut Canvas, v: &dyn Values, p: usize, r: Rect) {
    let on = v.plain(p) > 0.5;
    c.fill_rrect(r, 7.0, if on { ORANGE } else { CAP });
    c.stroke_rrect(r, 7.0, 1.0, if on { ORANGE } else { EDGE });
    c.text_center(
        VALUE,
        r,
        if on { DARK_TEXT } else { MUTED },
        if on { "ON" } else { "OFF" },
    );
}

fn paint_tie(c: &mut Canvas, v: &dyn Values, it: &Item) {
    let on = v.plain(it.param) > 0.5;
    let r = it.r;
    c.fill_rrect(r, 5.0, if on { AMBER } else { CAP });
    c.stroke_rrect(r, 5.0, 1.0, if on { AMBER } else { EDGE });
}

fn paint_slider(c: &mut Canvas, v: &dyn Values, it: &Item, held: bool) {
    let n = v.norm(it.param).clamp(0.0, 1.0);
    let r = it.r;
    let track = Rect::new(r.x + 6.0, r.cy() - 3.0, r.w - 70.0, 6.0);
    c.fill_rrect(track, 3.0, TRACK);
    let x = track.x + n * track.w;
    if bipolar(it.param) {
        let mid = track.x + track.w * 0.5;
        let (a, b) = if x < mid { (x, mid) } else { (mid, x) };
        c.fill_rrect(
            Rect::new(a, track.y, (b - a).max(1.0), track.h),
            3.0,
            ORANGE,
        );
        c.fill_rect(
            Rect::new(mid - 0.5, track.y - 3.0, 1.0, track.h + 6.0),
            MUTED,
        );
    } else {
        c.fill_rrect(
            Rect::new(track.x, track.y, (x - track.x).max(1.0), track.h),
            3.0,
            ORANGE,
        );
    }
    let knob = Rect::new(x - 7.0, r.cy() - 10.0, 14.0, 20.0);
    c.fill_rrect(knob, 5.0, if held { ORANGE } else { CAP });
    c.stroke_rrect(knob, 5.0, 2.0, ORANGE);
    let text = format!("{:+.0}%", v.plain(it.param) * 100.0);
    c.text_right(VALUE, r.right() - 2.0, r.cy() - 6.0, TEXT, &text);
}

fn paint_bar(c: &mut Canvas, v: &dyn Values, it: &Item, held: bool) {
    let n = v.norm(it.param).clamp(0.0, 1.0);
    let r = it.r;
    c.fill_rrect(r, 4.0, TRACK);
    let step = step_of(it.param);
    let live = step.is_none_or(|s| (s as f32) < v.plain(ids::ARP_STEPS));
    let col = if live { ORANGE } else { MUTED };
    if bipolar(it.param) {
        let mid = r.y + r.h * 0.5;
        let y = r.y + 2.0 + (1.0 - n) * (r.h - 4.0);
        let (a, b) = if y < mid { (y, mid) } else { (mid, y) };
        c.fill_rect(Rect::new(r.x + 2.0, a, r.w - 4.0, (b - a).max(1.0)), col);
        c.fill_rect(Rect::new(r.x, mid - 0.5, r.w, 1.0), MUTED);
        let t = v.text(it.param);
        let t = t.trim_end_matches(" st");
        c.text_center(SMALL, Rect::new(r.x, r.y, r.w, 12.0), TEXT, t);
    } else {
        let y = r.y + 2.0 + (1.0 - n) * (r.h - 4.0);
        c.fill_rect(
            Rect::new(r.x + 2.0, y, r.w - 4.0, r.bottom() - 2.0 - y),
            col,
        );
    }
    if held {
        c.stroke_rrect(r, 4.0, 1.5, AMBER);
    }
}

/// The step a per-step parameter belongs to (0-based).
fn step_of(p: usize) -> Option<usize> {
    [ids::ARP_STEP_BASE, ids::ARP_TR_BASE, ids::ARP_HOLD_BASE]
        .iter()
        .find(|&&b| p >= b && p < b + 16)
        .map(|&b| p - b)
}

fn wave_icon(c: &mut Canvas, wave: i32, r: Rect) {
    let pts: Vec<(f32, f32)> = (0..=24)
        .map(|i| {
            let t = i as f32 / 24.0;
            (r.x + t * r.w, r.cy() - wave_shape(wave, t) * r.h * 0.45)
        })
        .collect();
    c.polyline(&pts, 1.6, AMBER);
}

/// One cycle of oscillator wave `wave` (see [`ids::WAVE_NAMES`]) at phase `t` in `[0, 1]`, for the icons.
fn wave_shape(wave: i32, t: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    match wave {
        0 => (t * tau).sin(),
        1 => 1.0 - 2.0 * t,
        2 => 1.0 - 4.0 * ((t + 0.25).rem_euclid(1.0) - 0.5).abs(),
        3 => {
            if t < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        4 => {
            if t < 0.25 {
                1.0
            } else {
                -1.0
            }
        }
        5 => {
            if t < 0.125 {
                1.0
            } else {
                -1.0
            }
        }
        6 => 0.5 * (1.0 - 2.0 * t) + 0.5 * (1.0 - 4.0 * ((t + 0.25).rem_euclid(1.0) - 0.5).abs()),
        _ => (((t * 7919.0).sin() * 43758.5).fract() * 2.0 - 1.0).clamp(-1.0, 1.0),
    }
}

/// An LFO wave (see [`ids::LFO_WAVE_NAMES`]) at phase `t`, for its preview.
fn lfo_shape(wave: i32, t: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    match wave {
        0 => (t * tau).sin(),
        1 => 1.0 - 4.0 * ((t + 0.25).rem_euclid(1.0) - 0.5).abs(),
        2 => 2.0 * t - 1.0,
        3 => 1.0 - 2.0 * t,
        4 => {
            if t < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        5 => {
            if t < 0.25 {
                1.0
            } else {
                -1.0
            }
        }
        6 => {
            if t < 0.125 {
                1.0
            } else {
                -1.0
            }
        }
        7 => [0.6, -0.4, 0.9, -0.8][((t * 4.0) as usize).min(3)],
        8 => {
            let (a, b) = ([0.6, -0.5, 0.8, -0.7], (t * 4.0) as usize);
            let (i, f) = (b.min(3), (t * 4.0).fract());
            a[i] + (a[(i + 1) % 4] - a[i]) * f * f * (3.0 - 2.0 * f)
        }
        9 => ((t * 4.0).floor() / 3.0).clamp(0.0, 1.0) * 2.0 - 1.0,
        _ => (t * tau).sin().abs() * 2.0 - 1.0,
    }
}

/// The ADSR outline of `(att, dec, sus, rel)` in `r`.
fn env_points(r: Rect, att: f32, dec: f32, sus: f32, rel: f32) -> Vec<(f32, f32)> {
    let seg = |t: f32| t.max(0.0).sqrt();
    let (wa, wd, wr) = (seg(att), seg(dec), seg(rel));
    let hold = (0.35 * (wa + wd + wr)).max(0.15);
    let total = wa + wd + hold + wr;
    let x = |u: f32| r.x + r.w * u / total;
    let y = |level: f32| r.bottom() - 2.0 - level * (r.h - 4.0);
    let mut pts = vec![(x(0.0), y(0.0)), (x(wa), y(1.0))];
    for i in 1..=8 {
        let f = i as f32 / 8.0;
        pts.push((x(wa + wd * f), y(sus + (1.0 - sus) * (1.0 - f).powi(2))));
    }
    pts.push((x(wa + wd + hold), y(sus)));
    for i in 1..=8 {
        let f = i as f32 / 8.0;
        pts.push((x(wa + wd + hold + wr * f), y(sus * (1.0 - f).powi(2))));
    }
    pts
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

fn paint_extras(c: &mut Canvas, v: &dyn Values, ui: &Ui) {
    match ui.tab {
        TAB_OSC_A | TAB_OSC_B => {
            let part = ui.tab - TAB_OSC_A;
            let b = ids::PART_BASE + part * ids::PART_STRIDE;
            let r = Rect::new(
                X0 + 2.0,
                TOP + HEAD + 3.0 * CELL_H + 10.0,
                10.0 * CELL_W - 4.0,
                52.0,
            );
            c.fill_rrect(r, 8.0, PANEL);
            c.stroke_rrect(r, 8.0, 1.0, EDGE);
            c.text(SMALL, r.x + 10.0, r.y + 5.0, MUTED, "AMP ENVELOPE");
            let g = Rect::new(r.x + 100.0, r.y + 4.0, r.w - 112.0, r.h - 8.0);
            let pts = env_points(
                g,
                v.plain(b + ids::part::ATT),
                v.plain(b + ids::part::DEC),
                v.plain(b + ids::part::SUS),
                v.plain(b + ids::part::REL),
            );
            c.polyline(&pts, 2.0, ORANGE);
        }
        TAB_FILTER => {
            let y = TOP + HEAD + 2.0 * CELL_H + 10.0;
            let r = Rect::new(X0 + 2.0, y, 10.0 * CELL_W - 4.0, 330.0 - y);
            c.fill_rrect(r, 8.0, PANEL);
            c.stroke_rrect(r, 8.0, 1.0, EDGE);
            c.text(SMALL, r.x + 10.0, r.y + 5.0, MUTED, "RESPONSE");
            let g = Rect::new(r.x + 70.0, r.y + 6.0, r.w - 84.0, r.h - 12.0);
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
                let pts: Vec<(f32, f32)> = (0..=96)
                    .map(|i| {
                        let t = i as f32 / 96.0;
                        let hz = 20.0 * 1000.0f32.powf(t);
                        let db = filter_db(mode, slope, cut, res, hz).clamp(-40.0, 24.0);
                        (g.x + t * g.w, g.bottom() - (db + 40.0) / 64.0 * g.h)
                    })
                    .collect();
                c.polyline(&pts, 2.0, col);
            }
        }
        TAB_MOD => {
            let x = X0 + 8.0 * CELL_W + 2.0;
            let w = 2.0 * CELL_W - 4.0;
            let r = Rect::new(x, TOP, w, HEAD + CELL_H + 4.0);
            c.fill_rrect(r, 8.0, PANEL);
            c.stroke_rrect(r, 8.0, 1.0, EDGE);
            c.text(TITLE, r.x + 10.0, r.y + 5.0, AMBER, "SHAPES");
            let g = Rect::new(r.x + 8.0, r.y + HEAD, r.w - 16.0, r.h - HEAD - 6.0);
            for (n, col) in [(0usize, ORANGE), (1, AMBER)] {
                let b = ids::ENV_BASE + n * ids::ENV_STRIDE;
                let pts = env_points(
                    g,
                    v.plain(b),
                    v.plain(b + 1),
                    v.plain(b + 2),
                    v.plain(b + 3),
                );
                c.polyline(&pts, 1.8, col);
            }
            let y = TOP + HEAD + CELL_H + 8.0;
            let r = Rect::new(x, y, w, HEAD + 2.0 * CELL_H + 4.0);
            c.fill_rrect(r, 8.0, PANEL);
            c.stroke_rrect(r, 8.0, 1.0, EDGE);
            c.text(TITLE, r.x + 10.0, r.y + 5.0, AMBER, "LFO SHAPES");
            for (n, col) in [(0usize, ORANGE), (1, AMBER)] {
                let b = ids::LFO_BASE + n * ids::LFO_STRIDE;
                let wave = v.plain(b + ids::lfo::WAVE).round() as i32;
                let gain = v.plain(b + ids::lfo::GAIN);
                let off = v.plain(b + ids::lfo::OFFSET);
                let g = Rect::new(
                    r.x + 8.0,
                    r.y + HEAD + n as f32 * (CELL_H + 4.0),
                    r.w - 16.0,
                    CELL_H - 8.0,
                );
                let pts: Vec<(f32, f32)> = (0..=48)
                    .map(|i| {
                        let t = i as f32 / 48.0;
                        let y = (lfo_shape(wave, t.min(0.9999)) * gain + off).clamp(-1.0, 1.0);
                        (g.x + t * g.w, g.cy() - y * g.h * 0.45)
                    })
                    .collect();
                c.polyline(&pts, 1.8, col);
            }
        }
        TAB_MATRIX => {
            let sub = ui.sub().min(1);
            for row in 0..8 {
                let y = TOP_SUB + row as f32 * 31.0;
                let n = format!("{}", sub * 8 + row + 1);
                c.text_right(VALUE, 34.0, y + 8.0, MUTED, &n);
            }
        }
        TAB_ARP => {
            for (text, y, h) in [
                ("VEL", VEL_Y, VEL_H),
                ("TRANS", TR_Y, TR_H),
                ("TIE", TIE_Y, TIE_H),
            ] {
                c.text(SMALL, X0 + 8.0, y + h / 2.0 - 5.0, MUTED, text);
            }
            for i in 0..16 {
                let x = STEP_X + i as f32 * STEP_PITCH;
                let n = format!("{}", i + 1);
                let live = (i as f32) < v.plain(ids::ARP_STEPS);
                c.text_center(
                    SMALL,
                    Rect::new(x, TIE_Y + TIE_H + 1.0, STEP_PITCH - 3.0, 12.0),
                    if live { TEXT } else { MUTED },
                    &n,
                );
            }
        }
        TAB_MAIN => {
            let y = TOP + HEAD + CELL_H + 14.0;
            c.text(
                Font::new(14).bold(),
                X0 + 6.0,
                y,
                TEXT,
                "Rind \u{2014} Honeydew's built-in synthesizer",
            );
            let lines = [
                "4 oscillators with 8-voice unison, 2 filters, 4 envelopes, 2 LFOs and a 16-slot matrix.",
                "Open the preset browser to load Rind presets, or import Sylenth1 .fxp / .fxb files.",
                "Tap a selector to step it. Drag a knob up or down. Double tap resets to default.",
            ];
            for (i, l) in lines.iter().enumerate() {
                c.text(LABEL, X0 + 6.0, y + 28.0 + i as f32 * 16.0, MUTED, l);
            }
        }
        _ => {}
    }
}

fn paint_status(c: &mut Canvas, v: &dyn Values, last: Option<usize>) {
    let r = Rect::new(0.0, H as f32 - BAR_H, W as f32, BAR_H);
    c.fill_rect(r, PANEL);
    c.fill_rect(Rect::new(0.0, r.y, W as f32, 1.0), EDGE);
    if let Some(p) = last {
        c.text(LABEL, 12.0, r.y + 5.0, MUTED, SPECS[p].name);
        c.text_right(VALUE, W as f32 - 12.0, r.y + 5.0, TEXT, &v.text(p));
    } else {
        c.text(LABEL, 12.0, r.y + 5.0, MUTED, "Rind");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_lays_out_inside_the_window_without_overlap() {
        for tab in 0..TABS.len() {
            for sub in 0..subs_of(tab).max(1) {
                let mut ui = Ui {
                    tab,
                    ..Ui::default()
                };
                ui.subs[tab] = sub;
                let hits = layout(&ui);
                let win = Rect::new(0.0, 0.0, W as f32, H as f32 - BAR_H);
                for (i, a) in hits.iter().enumerate() {
                    assert!(
                        win.intersect(&a.r).w >= a.r.w - 0.01
                            && win.intersect(&a.r).h >= a.r.h - 0.01,
                        "tab {tab}/{sub}: {a:?} leaves the window"
                    );
                    for b in &hits[i + 1..] {
                        assert!(
                            a.r.intersect(&b.r).is_empty(),
                            "tab {tab}/{sub}: {a:?} overlaps {b:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn controls_are_finger_sized() {
        for tab in 0..TABS.len() {
            for sub in 0..subs_of(tab).max(1) {
                let mut ui = Ui {
                    tab,
                    ..Ui::default()
                };
                ui.subs[tab] = sub;
                for h in layout(&ui) {
                    if step_of(h.param).is_some() && !matches!(h.kind, Kind::Tab(_) | Kind::Sub(_))
                    {
                        assert!(h.r.w >= 30.0 && h.r.h >= 24.0, "{h:?}");
                    } else {
                        assert!(h.r.w >= 40.0 && h.r.h >= 22.0, "{h:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn every_parameter_has_a_control_on_some_page() {
        let mut seen = vec![false; ids::COUNT];
        for tab in 0..TABS.len() {
            for sub in 0..subs_of(tab).max(1) {
                let mut ui = Ui {
                    tab,
                    ..Ui::default()
                };
                ui.subs[tab] = sub;
                for h in layout(&ui) {
                    if !matches!(h.kind, Kind::Tab(_) | Kind::Sub(_)) {
                        seen[h.param] = true;
                    }
                }
            }
        }
        let missing: Vec<_> = seen
            .iter()
            .enumerate()
            .filter(|(_, s)| !**s)
            .map(|(i, _)| SPECS[i].id)
            .collect();
        assert!(missing.is_empty(), "no control for {missing:?}");
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

    #[test]
    fn the_envelope_outline_rises_then_returns_to_zero() {
        let r = Rect::new(0.0, 0.0, 100.0, 40.0);
        let pts = env_points(r, 0.01, 0.2, 0.5, 0.3);
        assert!(pts[0].1 > 36.0);
        let top = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min);
        assert!(top < 4.0);
        assert!(pts.last().unwrap().1 > 36.0);
        assert!(pts.windows(2).all(|w| w[1].0 >= w[0].0));
    }

    /// The defaults, as the editor would show them.
    struct Defaults;

    impl Values for Defaults {
        fn norm(&self, p: usize) -> f32 {
            crate::presets::to_norm(p, SPECS[p].default)
        }
        fn plain(&self, p: usize) -> f32 {
            SPECS[p].default
        }
        fn text(&self, p: usize) -> String {
            format!("{}", SPECS[p].default)
        }
    }

    #[test]
    fn every_page_paints_and_reports_the_regions_it_lays_out() {
        let mut c = Canvas::new(W, H, 1.0).unwrap();
        for tab in 0..TABS.len() {
            for sub in 0..subs_of(tab).max(1) {
                let mut ui = Ui {
                    tab,
                    ..Ui::default()
                };
                ui.subs[tab] = sub;
                let hits = paint(&mut c, &Defaults, &ui, Some(0), Some(1));
                assert_eq!(hits, layout(&ui), "tab {tab}/{sub}");
            }
        }
    }
}

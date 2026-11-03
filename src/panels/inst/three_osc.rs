//! The 3x Osc panel (type 39), card 10.0 U.
//! Every tab is a row: a left column (a list and toggles) and a right part. On OSC, VOL … PITCH and MISC a list
//! without a param picks the sub-page (UI state, `Show::Setting`).
//!
//! The LFO displays ([`lfo`]) and the envelope displays ([`envelopes`]) are module-drawn graph displays: a
//! controlFillColor rect, the curve on top in the channel colour with line width t = ceil(U/16) px, no grid lines.
//! Envelope displays draw the three inactive envelopes first at 20 % alpha, then the current one; breakpoint discs
//! (current envelope only) are channel discs of radius 2 t, with a controlFillColor centre of radius t only while
//! that envelope's `Enable` is off. With the display rect (x, y, w, h) and H = h − t, a value v (0…1) is drawn at
//! y + t/2 + H·(1 − v). Every stroke is the channel colour at width t. All params below are normalized (0…1). Each
//! function documents the rest.

use std::f32::consts::{FRAC_PI_8, TAU};
use std::path::Path;

use eframe::egui::{Color32, Id, Pos2, Rect, Shape, Stroke, Ui, pos2, vec2};

use crate::flm::chunk::ContentPath;
use crate::panels::build::*;
use crate::params::{clamp01, menu_index, module_spec};
use crate::ui::{UiCtx, widgets};

/// Param `p` in [0, 1]; a missing or NaN one reads 0. The FLM loader copies PRMS values as they are, and a wild
/// Speed or Decay would otherwise push billions of points or overflow a piece count.
fn param(params: &[f32], p: usize) -> f32 {
    clamp01(params.get(p).copied().unwrap_or(0.0))
}

/// The y of value `v` in the display `rect` with hairline `t`, y + t/2 + H·(1 − v).
fn value_y(rect: Rect, t: f32, v: f32) -> f32 {
    rect.top() + t / 2.0 + (rect.height() - t) * (1.0 - v)
}

/// The LFO display: the polyline of LFO `i` (Pan, Volume, ModX, ModY, Pitch; Shape p37 + i, Delay p42 + i,
/// Attack p47 + i, Amount p52 + i, Speed p57 + i) in `rect` with hairline `t`. Samples at f = 0, s, 2s, … while
/// f < 1, drawn at x + f·w, with s = 1/(32·(16 − 14·Speed)). The value stays 0.5 while f < 0.5·Delay, then
/// sample n reads the shape at phase p = n/32 cycles; value = clamp(0.5 + 0.5·g·σ·|2·Amount − 1|·wave, 0, 1);
/// g = 1 when Attack = 0, otherwise g = 0 at the first post-delay sample, and after each sample
/// g ← min(1, g + s·cot(Attack·π/8)). Params read through [`param`].
///
/// The polyline starts at the left edge on the centre line, (x, centre), then runs through the
/// samples. Sample 0 is that point unless a Square starts at full depth (g = 1 at phase 0), which then rises or falls
/// from it at the left edge.
fn lfo_points(params: &[f32], i: usize, rect: Rect, t: f32) -> Vec<Pos2> {
    let v = |p: usize| param(params, p + i);
    let (shape, delay, attack, amount) = (menu_index(v(37), 3), v(42), v(47), v(52));
    // 1/s: f = k·s < 1 is k < 1/s, which a sum of s's would miss by a rounding.
    let samples = 32.0 * (16.0 - 14.0 * v(57));
    let depth = if amount >= 0.5 { 1.0 } else { -1.0 } * (2.0 * amount - 1.0).abs();
    // s·cot(Attack·π/8); infinite at Attack 0, where g starts at 1 anyway.
    let step = 1.0 / (samples * (attack * FRAC_PI_8).tan());
    let mut g = if attack == 0.0 { 1.0 } else { 0.0 };
    let mut n = 0;
    let start = pos2(rect.left(), value_y(rect, t, 0.5));
    let samples = (0..)
        .map(|k| k as f32)
        .take_while(|&k| k < samples)
        .map(|k| {
            let f = k / samples;
            let mut value = 0.5;
            if f >= 0.5 * delay {
                let p = (n as f32 / 32.0).fract();
                let wave = match shape {
                    0 => (TAU * p).sin(),
                    // 0 → +1 at ¼ → 0 at ½ → −1 at ¾.
                    1 if p < 0.25 => 4.0 * p,
                    1 if p < 0.75 => 2.0 - 4.0 * p,
                    1 => 4.0 * p - 4.0,
                    _ if p < 0.5 => 1.0,
                    _ => -1.0,
                };
                value = (0.5 + 0.5 * g * depth * wave).clamp(0.0, 1.0);
                n += 1;
                g = (g + step).min(1.0);
            }
            pos2(rect.left() + f * rect.width(), value_y(rect, t, value))
        });
    let mut points: Vec<Pos2> = std::iter::once(start).chain(samples).collect();
    // x rises from sample to sample, so only sample 0 can repeat the start.
    points.dedup();
    points
}

/// The envelope display: envelope `e` (Volume, ModX, ModY, Pitch; Delay p70 + e, Attack p74 + e, Hold p82 + e,
/// Decay p86 + e, Sustain p94 + e, Release p98 + e, Amount p106 + e; the Attack, Decay and Release Tensions p78,
/// p90, p102 + e) in `rect` with hairline `t` at `ppp`: the polyline from P0 and the breakpoints P1…P5. With base =
/// y + h − t/2, seg = 0.2·w, A = |2·Amount − 1|: P1 = P0 + (Delay·seg, 0), straight; P2 = (P1.x + Attack·seg,
/// base − A·H) with the Attack tension; P3 = P2 + (Hold·seg, 0), straight; P4 = (P3.x + Decay·seg·(1 − Sustain),
/// base − A·H·Sustain) with the Decay tension; P5 = (P4.x + Release·seg, base) with the Release tension. Params read
/// through [`param`].
fn envelope(params: &[f32], e: usize, rect: Rect, t: f32, ppp: f32) -> (Vec<Pos2>, [Pos2; 5]) {
    let v = |p: usize| param(params, p + e);
    let (base, seg) = (rect.bottom() - t / 2.0, 0.2 * rect.width());
    let top = (2.0 * v(106) - 1.0).abs() * (rect.height() - t);
    let p0 = pos2(rect.left(), base);
    let p1 = p0 + vec2(v(70) * seg, 0.0);
    let p2 = pos2(p1.x + v(74) * seg, base - top);
    let p3 = p2 + vec2(v(82) * seg, 0.0);
    let p4 = pos2(p3.x + v(86) * seg * (1.0 - v(94)), base - top * v(94));
    let p5 = pos2(p4.x + v(98) * seg, base);
    let mut line = vec![p0];
    for (end, tension) in [(p1, 0.5), (p2, v(78)), (p3, 0.5), (p4, v(90)), (p5, v(102))] {
        segment(&mut line, end, 2.0 * tension - 1.0, ppp);
    }
    (line, [p1, p2, p3, p4, p5])
}

/// The tension law: extends `line` from its last point to `end` with τ = `tau`. A segment is a straight line
/// when τ = 0, when it is flat, or when it is under 1 px wide. Otherwise it is drawn as N = round(dx) pieces;
/// piece i (0…N−1) ends at x0 + (i + 1)·dx/N with u = i/(N − 1). Rising segments (upward on screen):
/// y = y0 + G(u)·(y1 − y0); falling: y = y1 − G(1 − u)·(y1 − y0). dx is in device px.
///
/// N = 1 (dx from 1 to 1.5 px) has no u = i/(N − 1); it is straight too. So N < 2 is the whole width
/// rule, under 1 px included.
fn segment(line: &mut Vec<Pos2>, end: Pos2, tau: f32, ppp: f32) {
    let start = *line.last().expect("the line starts at P0");
    let (dx, dy) = (end.x - start.x, end.y - start.y);
    let pieces = (dx * ppp).round() as usize;
    if tau == 0.0 || dy == 0.0 || pieces < 2 {
        if end != start {
            line.push(end);
        }
        return;
    }
    let last = (pieces - 1) as f32;
    line.extend((0..pieces).map(|i| {
        let u = i as f32 / last;
        let y = if dy < 0.0 {
            start.y + tension(u, tau) * dy
        } else {
            end.y - tension(1.0 - u, tau) * dy
        };
        pos2(start.x + (i + 1) as f32 * dx / pieces as f32, y)
    }));
}

/// The tension law's G(u) for τ: k = sgn τ·(e^(8.7557·|τ|) − 1)/10 and L = ln(1 + |k|); G(u) = 1 +
/// (1 − e^((1−u)·L))/k for k > 0, G(u) = (e^(u·L) − 1)/|k| for k < 0; G(u) = u at τ = 0. τ > 0 rises fast first,
/// so the curve bows toward the top.
fn tension(u: f32, tau: f32) -> f32 {
    if tau == 0.0 {
        return u;
    }
    let k = tau.signum() * (8.7557 * tau.abs()).exp_m1() / 10.0;
    let l = k.abs().ln_1p();
    if k > 0.0 {
        1.0 - ((1.0 - u) * l).exp_m1() / k
    } else {
        (u * l).exp_m1() / -k
    }
}

/// The graph display of the tab's LFO, one polyline with no grid and no dots: PAN (tab 1) is LFO 0, VOL … PITCH
/// (tabs 2–5) LFOs 1–4. Nothing to edit.
fn lfo(
    ui: &mut Ui,
    cx: &UiCtx,
    _: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let t = cx.m.t();
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, cx.theme.colors.control_fill.to_color32());
    let points = lfo_points(&module.params, state.tab.saturating_sub(1), rect, t);
    painter.add(Shape::line(points, Stroke::new(t, ink)));
    Vec::new()
}

/// The graph display of the tab's envelope (VOL … PITCH, tabs 2–5, are envelopes 0–3). The other three envelopes
/// are drawn first into the same rect at alpha 0.2, without dots; each is skipped while its Amount = 0.5. The current
/// envelope is drawn at the channel colour's own alpha. Dots on the current envelope only, at P1…P5 (not P0): channel
/// discs of radius 2t. While that envelope's `Enable` is off, a controlFillColor disc of radius t is added on each
/// (rings). Nothing to edit.
///
/// all discs first, then all centres. Either order keeps the ring on coincident
/// breakpoints (Hold 0); they differ only where two dots partly overlap, and there this one keeps both centres.
fn envelopes(
    ui: &mut Ui,
    cx: &UiCtx,
    _: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let (t, ppp, fill) = (
        cx.m.t(),
        cx.m.ppp(),
        cx.theme.colors.control_fill.to_color32(),
    );
    let (p, current) = (&module.params, state.tab.saturating_sub(2));
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, fill);
    let dim = Stroke::new(t, ink.gamma_multiply(0.2));
    for e in (0..4).filter(|&e| e != current && param(p, 106 + e) != 0.5) {
        painter.add(Shape::line(envelope(p, e, rect, t, ppp).0, dim));
    }
    let (line, dots) = envelope(p, current, rect, t, ppp);
    painter.add(Shape::line(line, Stroke::new(t, ink)));
    for d in dots {
        painter.circle_filled(d, 2.0 * t, ink);
    }
    // Rings while the engine has the envelope off: on above 0.5 only.
    if !honeydew_audio::three_osc::toggle(param(p, 62 + current)) {
        for d in dots {
            painter.circle_filled(d, t, fill);
        }
    }
    Vec::new()
}

/// The module type id.
pub const TYPE_ID: u16 = 39;

const TABS: &str = "OSC · PAN · VOL · CUT · RES · PITCH · FILT · MISC";
const OSCS: &str = "1 · 2 · 3";
const ENV_LFO: &str = "Envelope · LFO";
const MISC: &str = "Poly · Levels · Arp · Echo";
/// The `Custom` item of the oscillators' Shape menus (params 1–3).
const CUSTOM: usize = 6;

/// The id of `#n` on tab `tab` (from 1): `tab·10000` plus `#n`'s components as four digits, left-aligned, so tab 1
/// `#2.5.1.1` is 12511 and tab 7 `#3` is 73000. The tab strip is 0.
const fn id(tab: u32, n: u32) -> u32 {
    tab * 10_000 + n
}

/// Where each tab's sub-page list is: `#1.1`, or `#1` on MISC.
const OSC_LIST: u32 = id(1, 1100);
const MISC_LIST: u32 = id(8, 1000);

pub fn panel() -> Option<Panel> {
    Some(
        Panel::new(vstack([
            tabs(0, TABS).h(Size::U(1.25)),
            osc().show(Show::Tab(0)),
            hstack([
                vstack([
                    spacer(id(2, 1100)),
                    list(id(2, 1200), "LFO", Bar::Right),
                    toggle_right(id(2, 1300), "Global").p(27),
                    toggle_right(id(2, 1400), "Tempo").p(32),
                ]),
                lfo_page(2, 0),
            ])
            .show(Show::Tab(1)),
            modulation(3),
            modulation(4),
            modulation(5),
            modulation(6),
            filt().show(Show::Tab(6)),
            misc().show(Show::Tab(7)),
        ]))
        .overlay(sample_name),
    )
}

/// The card wiring: the custom waveform's file name is its `CSTM` `PATH`'s file stem,
/// empty without one. `loading` stays 0, since the sample cache decodes the file on the app thread before the
/// engine is built, so there is no loading time to show.
pub fn wire(module: &ModuleInstance, state: &mut PanelState) {
    let name = crate::audio_engine::samples::shape_path(module).and_then(|path| {
        let (ContentPath::Shorthand { relative: text, .. } | ContentPath::Plain(text)) = path;
        Some(Path::new(&text).file_stem()?.to_string_lossy().into_owned())
    });
    state.sample_name = name.unwrap_or_default();
}

/// Whether the user's edits `changed` (param indices) picked `Custom` in an oscillator's Shape menu (params 1–3): the
/// wiring then opens the sample picker.
pub fn custom_picked(module: &ModuleInstance, changed: &[i32]) -> bool {
    module.type_id == TYPE_ID
        && (changed.iter())
            .any(|&p| (1..=3).contains(&p) && shape(module, p as usize) == Some(CUSTOM))
}

/// The item oscillator Shape menu `p` (params 1–3) shows, by its param's own index law.
fn shape(module: &ModuleInstance, p: usize) -> Option<usize> {
    let kind = module_spec(module.type_id)?.params.get(p)?.kind;
    Some(kind.item(*module.params.get(p)?))
}

/// The custom waveform's file name, or `loading...` while the `loading` flag is set, on
/// the `OSC` tab while an oscillator's Shape is `Custom`: truncated keeping 5 tail characters, 8 pt, in the channel
/// colour, left and bottom aligned in (m.x + 0.1 U, m.y + 0.4 U, 0.3·m.w, m.h), where m is the module area `area`
/// (below the card header, tab strip included). Its bottom lands 0.4 U below the area, inside the card's 0.5 U bottom
/// margin and so inside the card clip.
fn sample_name(
    ui: &Ui,
    cx: &UiCtx,
    module: &ModuleInstance,
    state: &PanelState,
    _: &[(u32, Rect)],
    area: Rect,
    channel: Color32,
) {
    let loading = state.flags.get(flag::LOADING).is_some_and(|&v| v != 0);
    let text = if loading {
        "loading..."
    } else {
        &state.sample_name
    };
    // Params 1–3 are the oscillators' Shape menus.
    let custom = (1..=3).any(|p| shape(module, p) == Some(CUSTOM));
    if state.tab != 0 || !custom || text.is_empty() {
        return;
    }
    let m = cx.m;
    let min = area.min + vec2(m.u(0.1), m.u(0.4));
    let rect = Rect::from_min_size(min, vec2(0.3 * area.width(), area.height()));
    let font = m.font_id(FontSize::Pt8);
    widgets::truncated(
        ui.painter(),
        rect,
        Align2::LEFT_BOTTOM,
        text,
        font,
        channel,
        5,
    );
}

/// Tab 1 `OSC`: the oscillator list over `HQ`, then one page per oscillator. Osc 2 and 3 `Coarse` and osc 3 `Mix`
/// reset to 0.5 in the registry, so their rings are bipolar.
fn osc() -> Part {
    let page = |o: usize| {
        // Oscillator `o` (from 0) is `#2` + o; its params step by one per oscillator.
        let n = |n: u32| id(1, n + 1000 * o as u32);
        let mut switches = vec![
            toggle(n(2210), "Invert").p(4 + o),
            toggle(n(2220), "Free run").p(146 + o),
        ];
        if o == 2 {
            switches.push(toggle(n(2230), "AM").p(24));
        }
        let (last, p) = if o == 0 {
            ("Phase Rand", 26)
        } else {
            ("Mix", 21 + o)
        };
        hstack([
            spinner(n(2100), "Shape").p(1 + o),
            vstack(switches),
            v_slider(n(2300), "Phase").p(7 + o),
            v_slider(n(2400), "Detune").p(10 + o),
            vstack([
                hstack([
                    knob(n(2511), "Coarse").p(13 + o),
                    knob(n(2512), "Fine").p(16 + o),
                ]),
                hstack([knob(n(2521), "Pan").p(19 + o), knob(n(2522), last).p(p)]),
            ])
            .w(Size::Flex(3.0)),
        ])
        .w(Size::Flex(6.0))
        .show(Show::Setting(OSC_LIST, o))
    };
    hstack([
        vstack([
            list(OSC_LIST, OSCS, Bar::Right).h(Size::Flex(3.0)),
            toggle_right(id(1, 1200), "HQ").p(25),
        ]),
        page(0),
        page(1),
        page(2),
    ])
}

/// The LFO page `#2` of tab `tab` for LFO target `t` (Pan, Volume, ModX, ModY, Pitch; params step by one).
fn lfo_page(tab: u32, t: usize) -> Part {
    let n = |n: u32| id(tab, n);
    hstack([
        spinner(n(2100), "Shape").p(37 + t),
        vstack([
            knob(n(2210), "Delay").p(42 + t),
            knob(n(2220), "Attack").p(47 + t),
        ]),
        vstack([
            knob(n(2310), "Amount").p(52 + t),
            knob(n(2320), "Speed").p(57 + t),
        ]),
        display(n(2400))
            .w(Size::Flex(2.0))
            .margins(0.5, 0.5, 0.5, 0.5)
            .draw(lfo),
    ])
    .w(Size::Flex(4.0))
}

/// Tabs 3–6 `VOL` `CUT` `RES` `PITCH`: the `Envelope`/`LFO` list over the shown one's toggles, then the LFO page
/// `#2` or the envelope page `#3`.
fn modulation(tab: u32) -> Part {
    // Envelope targets are Volume, ModX, ModY, Pitch; the LFO targets have Pan first.
    let e = tab as usize - 3;
    let n = |n: u32| id(tab, n);
    let list_id = n(1100);
    let (env, lfo) = (Show::Setting(list_id, 0), Show::Setting(list_id, 1));
    let column = |[a, b, c]: [(u32, &'static str, usize); 3]| {
        vstack([
            knob(n(a.0), a.1).p(a.2 + e),
            knob(n(b.0), b.1).p(b.2 + e),
            h_slider(n(c.0), c.1).p(c.2 + e).h(Size::Flex(0.6)),
        ])
    };
    let mut graph = vec![
        display(n(3410))
            .h(Size::Percent(50.0))
            .margins(0.5, 0.5, 0.5, 0.5)
            .draw(envelopes),
    ];
    // Hidden on VOL, so the Volume envelope's Amount (p106) has no control.
    if e > 0 {
        graph.push(h_slider(n(3420), "Amount").p(106 + e).w(Size::Flex(0.9)));
    }
    hstack([
        vstack([
            list(list_id, ENV_LFO, Bar::Right).h(Size::Flex(2.0)),
            toggle_right(n(1200), "Global").p(28 + e).show(lfo),
            toggle_right(n(1300), "Tempo").p(33 + e).show(lfo),
            toggle_right(n(1400), "Enable").p(62 + e).show(env),
            toggle_right(n(1500), "Tempo").p(66 + e).show(env),
        ]),
        lfo_page(tab, e + 1).show(lfo),
        hstack([
            column([
                (3110, "Delay", 70),
                (3120, "Attack", 74),
                (3130, "Tension", 78),
            ]),
            column([
                (3210, "Hold", 82),
                (3220, "Decay", 86),
                (3230, "Tension", 90),
            ]),
            column([
                (3310, "Sustain", 94),
                (3320, "Release", 98),
                (3330, "Tension", 102),
            ]),
            vstack(graph).w(Size::Flex(2.0)),
        ])
        .w(Size::Flex(4.0))
        .show(env),
    ])
    .show(Show::Tab(tab as usize - 1))
}

/// Tab 7 `FILT`: `Type`, `Cutoff` and `Resonance` between flex-0.25 spacers.
fn filt() -> Part {
    let gap = |n: u32| spacer(id(7, n)).w(Size::Flex(0.25));
    hstack([
        gap(1000),
        spinner(id(7, 2000), "Type").p(110),
        gap(3000),
        knob(id(7, 4000), "Cutoff").p(111),
        gap(5000),
        knob(id(7, 6000), "Resonance").p(112),
        gap(7000),
    ])
}

/// Tab 8 `MISC`: the `Poly`/`Levels`/`Arp`/`Echo` list (a fifth of the width) and its four pages.
fn misc() -> Part {
    let n = |n: u32| id(8, n);
    let pair = |a: (u32, &'static str, usize), b: (u32, &'static str, usize)| {
        hstack([knob(n(a.0), a.1).p(a.2), knob(n(b.0), b.1).p(b.2)])
    };
    // A captioned 2×2 knob group of `Levels`: the caption is 10 pt, centred, 1 U tall.
    let group = |c: u32, caption: &'static str, p: [usize; 4], labels: [&'static str; 4]| {
        vstack([
            text(n(c), caption)
                .font(FontSize::Pt10)
                .align(Align2::CENTER_CENTER)
                .h(Size::U(1.0)),
            pair((c + 11, labels[0], p[0]), (c + 12, labels[1], p[1])),
            pair((c + 21, labels[2], p[2]), (c + 22, labels[3], p[3])),
        ])
    };
    let tracking = ["Middle", "To Pan", "To Cutoff", "To Resonance"];
    hstack([
        list(MISC_LIST, MISC, Bar::Right).w(Size::Percent(20.0)),
        hstack([
            knob(n(2100), "Voices").p(117),
            knob(n(2200), "Slide").p(118),
            vstack([
                toggle(n(2310), "Porta").p(119),
                toggle(n(2320), "Mono").p(120),
            ]),
            knob(n(2400), "Gate").p(129),
        ])
        .show(Show::Setting(MISC_LIST, 0)),
        hstack([
            group(
                3110,
                "Adjustment",
                [113, 114, 115, 116],
                ["Pan", "Volume", "Cutoff", "Resonance"],
            ),
            separator(n(3200)).w(Size::U(0.1)),
            group(3310, "Vol Tracking", [125, 126, 127, 128], tracking),
            separator(n(3400)).w(Size::U(0.1)),
            group(3510, "Key Tracking", [121, 122, 123, 124], tracking),
        ])
        .show(Show::Setting(MISC_LIST, 1)),
        vstack([
            hstack([
                spinner(n(4110), "Direction").p(139),
                knob(n(4120), "Gate").p(141),
                knob(n(4130), "Time").p(140),
                knob(n(4140), "Range").p(142),
                knob(n(4150), "Repeat").p(143),
            ]),
            hstack([
                toggle(n(4210), "Slide").p(144),
                // The popup's two columns break before `9b5` (item 46).
                menu_button(n(4220), "Chord")
                    .p(145)
                    .items_per_column(46)
                    .w(Size::Flex(4.0))
                    .h(Size::U(1.5))
                    .margins(0.0, 0.5, 0.0, 0.5),
            ]),
        ])
        .show(Show::Setting(MISC_LIST, 2)),
        hstack([
            v_slider(n(5100), "Echoes").p(130),
            vstack([
                knob(n(5210), "Time").p(132),
                knob(n(5220), "Feedback").p(131),
            ]),
            vstack([knob(n(5310), "Pan").p(133), knob(n(5320), "Cutoff").p(135)]),
            vstack([
                knob(n(5410), "Pitch").p(134),
                knob(n(5420), "Resonance").p(136),
            ]),
            vstack([
                toggle(n(5510), "Ping Pong").p(137),
                toggle(n(5520), "Fat Mode").p(138),
            ]),
        ])
        .show(Show::Setting(MISC_LIST, 3)),
    ])
}

#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [16.96, 7.99],
    // The sub-page lists start on their first item.
    settings: &[(11100, 0.0), (21200, 0.0), (31100, 0.0), (41100, 0.0), (51100, 0.0), (61100, 0.0), (81000, 0.0)],
    cases: &[
        Case {
            name: "OSC 1",
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(11100, Widget::List(Bar::Right), OSCS, [0.00, 1.25, 2.42, 5.05]).h(Size::Flex(3.0)),
                row(11200, Widget::ToggleRight, "HQ", [0.00, 6.30, 2.42, 1.69]).p(25),
                row(12100, Widget::Spinner, "Shape", [2.42, 1.25, 2.07, 6.74]).p(1),
                row(12210, Widget::Toggle, "Invert", [4.49, 1.25, 2.08, 3.37]).p(4),
                row(12220, Widget::Toggle, "Free run", [4.49, 4.62, 2.08, 3.37]).p(146),
                row(12300, Widget::VSlider, "Phase", [6.57, 1.25, 2.08, 6.74]).p(7),
                row(12400, Widget::VSlider, "Detune", [8.65, 1.25, 2.07, 6.74]).p(10),
                row(12511, Widget::Knob, "Coarse", [10.72, 1.25, 3.12, 3.37]).p(13),
                row(12512, Widget::Knob, "Fine", [13.84, 1.25, 3.12, 3.37]).p(16),
                row(12521, Widget::Knob, "Pan", [10.72, 4.62, 3.12, 3.37]).p(19),
                row(12522, Widget::Knob, "Phase Rand", [13.84, 4.62, 3.12, 3.37]).p(26),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "OSC 2",
            settings: &[(11100, 1.0)],
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(11100, Widget::List(Bar::Right), OSCS, [0.00, 1.25, 2.42, 5.05]).h(Size::Flex(3.0)),
                row(11200, Widget::ToggleRight, "HQ", [0.00, 6.30, 2.42, 1.69]).p(25),
                row(13100, Widget::Spinner, "Shape", [2.42, 1.25, 2.07, 6.74]).p(2),
                row(13210, Widget::Toggle, "Invert", [4.49, 1.25, 2.08, 3.37]).p(5),
                row(13220, Widget::Toggle, "Free run", [4.49, 4.62, 2.08, 3.37]).p(147),
                row(13300, Widget::VSlider, "Phase", [6.57, 1.25, 2.08, 6.74]).p(8),
                row(13400, Widget::VSlider, "Detune", [8.65, 1.25, 2.07, 6.74]).p(11),
                row(13511, Widget::Knob, "Coarse", [10.72, 1.25, 3.12, 3.37]).p(14),
                row(13512, Widget::Knob, "Fine", [13.84, 1.25, 3.12, 3.37]).p(17),
                row(13521, Widget::Knob, "Pan", [10.72, 4.62, 3.12, 3.37]).p(20),
                row(13522, Widget::Knob, "Mix", [13.84, 4.62, 3.12, 3.37]).p(22),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "OSC 3",
            settings: &[(11100, 2.0)],
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(11100, Widget::List(Bar::Right), OSCS, [0.00, 1.25, 2.42, 5.05]).h(Size::Flex(3.0)),
                row(11200, Widget::ToggleRight, "HQ", [0.00, 6.30, 2.42, 1.69]).p(25),
                row(14100, Widget::Spinner, "Shape", [2.42, 1.25, 2.07, 6.74]).p(3),
                row(14210, Widget::Toggle, "Invert", [4.49, 1.25, 2.08, 2.24]).p(6),
                row(14220, Widget::Toggle, "Free run", [4.49, 3.49, 2.08, 2.25]).p(148),
                row(14230, Widget::Toggle, "AM", [4.49, 5.74, 2.08, 2.25]).p(24),
                row(14300, Widget::VSlider, "Phase", [6.57, 1.25, 2.08, 6.74]).p(9),
                row(14400, Widget::VSlider, "Detune", [8.65, 1.25, 2.07, 6.74]).p(12),
                row(14511, Widget::Knob, "Coarse", [10.72, 1.25, 3.12, 3.37]).p(15),
                row(14512, Widget::Knob, "Fine", [13.84, 1.25, 3.12, 3.37]).p(18),
                row(14521, Widget::Knob, "Pan", [10.72, 4.62, 3.12, 3.37]).p(21),
                row(14522, Widget::Knob, "Mix", [13.84, 4.62, 3.12, 3.37]).p(23),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "PAN",
            tab: 1,
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(21100, Widget::Spacer, "", [0.00, 1.25, 3.39, 1.68]),
                row(21200, Widget::List(Bar::Right), "LFO", [0.00, 2.93, 3.39, 1.69]),
                row(21300, Widget::ToggleRight, "Global", [0.00, 4.62, 3.39, 1.68]).p(27),
                row(21400, Widget::ToggleRight, "Tempo", [0.00, 6.30, 3.39, 1.69]).p(32),
                row(22100, Widget::Spinner, "Shape", [3.39, 1.25, 2.51, 6.74]).p(37),
                row(22210, Widget::Knob, "Delay", [5.90, 1.25, 2.51, 3.37]).p(42),
                row(22220, Widget::Knob, "Attack", [5.90, 4.62, 2.51, 3.37]).p(47),
                row(22310, Widget::Knob, "Amount", [8.41, 1.25, 2.52, 3.37]).p(52),
                row(22320, Widget::Knob, "Speed", [8.41, 4.62, 2.52, 3.37]).p(57),
                row(22400, Widget::Display, "", [11.43, 1.75, 5.03, 5.74]).w(Size::Flex(2.0)).margins(0.5, 0.5, 0.5, 0.5),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "VOL Envelope",
            tab: 2,
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(31100, Widget::List(Bar::Right), ENV_LFO, [0.00, 1.25, 3.39, 3.37]).h(Size::Flex(2.0)),
                row(31400, Widget::ToggleRight, "Enable", [0.00, 4.62, 3.39, 1.68]).p(62),
                row(31500, Widget::ToggleRight, "Tempo", [0.00, 6.30, 3.39, 1.69]).p(66),
                row(33110, Widget::Knob, "Delay", [3.39, 1.25, 2.71, 2.59]).p(70),
                row(33120, Widget::Knob, "Attack", [3.39, 3.84, 2.71, 2.59]).p(74),
                row(33130, Widget::HSlider, "Tension", [3.39, 6.43, 2.71, 1.56]).p(78).h(Size::Flex(0.6)),
                row(33210, Widget::Knob, "Hold", [6.10, 1.25, 2.71, 2.59]).p(82),
                row(33220, Widget::Knob, "Decay", [6.10, 3.84, 2.71, 2.59]).p(86),
                row(33230, Widget::HSlider, "Tension", [6.10, 6.43, 2.71, 1.56]).p(90).h(Size::Flex(0.6)),
                row(33310, Widget::Knob, "Sustain", [8.81, 1.25, 2.72, 2.59]).p(94),
                row(33320, Widget::Knob, "Release", [8.81, 3.84, 2.72, 2.59]).p(98),
                row(33330, Widget::HSlider, "Tension", [8.81, 6.43, 2.72, 1.56]).p(102).h(Size::Flex(0.6)),
                row(33410, Widget::Display, "", [12.03, 1.75, 4.43, 3.37]).h(Size::Percent(50.0)).margins(0.5, 0.5, 0.5, 0.5),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "VOL LFO",
            tab: 2,
            settings: &[(31100, 1.0)],
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(31100, Widget::List(Bar::Right), ENV_LFO, [0.00, 1.25, 3.39, 3.37]).h(Size::Flex(2.0)),
                row(31200, Widget::ToggleRight, "Global", [0.00, 4.62, 3.39, 1.68]).p(28),
                row(31300, Widget::ToggleRight, "Tempo", [0.00, 6.30, 3.39, 1.69]).p(33),
                row(32100, Widget::Spinner, "Shape", [3.39, 1.25, 2.51, 6.74]).p(38),
                row(32210, Widget::Knob, "Delay", [5.90, 1.25, 2.51, 3.37]).p(43),
                row(32220, Widget::Knob, "Attack", [5.90, 4.62, 2.51, 3.37]).p(48),
                row(32310, Widget::Knob, "Amount", [8.41, 1.25, 2.52, 3.37]).p(53),
                row(32320, Widget::Knob, "Speed", [8.41, 4.62, 2.52, 3.37]).p(58),
                row(32400, Widget::Display, "", [11.43, 1.75, 5.03, 5.74]).w(Size::Flex(2.0)).margins(0.5, 0.5, 0.5, 0.5),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "CUT Envelope",
            tab: 3,
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(41100, Widget::List(Bar::Right), ENV_LFO, [0.00, 1.25, 3.39, 3.37]).h(Size::Flex(2.0)),
                row(41400, Widget::ToggleRight, "Enable", [0.00, 4.62, 3.39, 1.68]).p(63),
                row(41500, Widget::ToggleRight, "Tempo", [0.00, 6.30, 3.39, 1.69]).p(67),
                row(43110, Widget::Knob, "Delay", [3.39, 1.25, 2.71, 2.59]).p(71),
                row(43120, Widget::Knob, "Attack", [3.39, 3.84, 2.71, 2.59]).p(75),
                row(43130, Widget::HSlider, "Tension", [3.39, 6.43, 2.71, 1.56]).p(79).h(Size::Flex(0.6)),
                row(43210, Widget::Knob, "Hold", [6.10, 1.25, 2.71, 2.59]).p(83),
                row(43220, Widget::Knob, "Decay", [6.10, 3.84, 2.71, 2.59]).p(87),
                row(43230, Widget::HSlider, "Tension", [6.10, 6.43, 2.71, 1.56]).p(91).h(Size::Flex(0.6)),
                row(43310, Widget::Knob, "Sustain", [8.81, 1.25, 2.72, 2.59]).p(95),
                row(43320, Widget::Knob, "Release", [8.81, 3.84, 2.72, 2.59]).p(99),
                row(43330, Widget::HSlider, "Tension", [8.81, 6.43, 2.72, 1.56]).p(103).h(Size::Flex(0.6)),
                row(43410, Widget::Display, "", [12.03, 1.75, 4.43, 3.37]).h(Size::Percent(50.0)).margins(0.5, 0.5, 0.5, 0.5),
                row(43420, Widget::HSlider, "Amount", [11.80, 5.62, 4.88, 2.37]).p(107).w(Size::Flex(0.9)),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "CUT LFO",
            tab: 3,
            settings: &[(41100, 1.0)],
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(41100, Widget::List(Bar::Right), ENV_LFO, [0.00, 1.25, 3.39, 3.37]).h(Size::Flex(2.0)),
                row(41200, Widget::ToggleRight, "Global", [0.00, 4.62, 3.39, 1.68]).p(29),
                row(41300, Widget::ToggleRight, "Tempo", [0.00, 6.30, 3.39, 1.69]).p(34),
                row(42100, Widget::Spinner, "Shape", [3.39, 1.25, 2.51, 6.74]).p(39),
                row(42210, Widget::Knob, "Delay", [5.90, 1.25, 2.51, 3.37]).p(44),
                row(42220, Widget::Knob, "Attack", [5.90, 4.62, 2.51, 3.37]).p(49),
                row(42310, Widget::Knob, "Amount", [8.41, 1.25, 2.52, 3.37]).p(54),
                row(42320, Widget::Knob, "Speed", [8.41, 4.62, 2.52, 3.37]).p(59),
                row(42400, Widget::Display, "", [11.43, 1.75, 5.03, 5.74]).w(Size::Flex(2.0)).margins(0.5, 0.5, 0.5, 0.5),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "RES Envelope",
            tab: 4,
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(51100, Widget::List(Bar::Right), ENV_LFO, [0.00, 1.25, 3.39, 3.37]).h(Size::Flex(2.0)),
                row(51400, Widget::ToggleRight, "Enable", [0.00, 4.62, 3.39, 1.68]).p(64),
                row(51500, Widget::ToggleRight, "Tempo", [0.00, 6.30, 3.39, 1.69]).p(68),
                row(53110, Widget::Knob, "Delay", [3.39, 1.25, 2.71, 2.59]).p(72),
                row(53120, Widget::Knob, "Attack", [3.39, 3.84, 2.71, 2.59]).p(76),
                row(53130, Widget::HSlider, "Tension", [3.39, 6.43, 2.71, 1.56]).p(80).h(Size::Flex(0.6)),
                row(53210, Widget::Knob, "Hold", [6.10, 1.25, 2.71, 2.59]).p(84),
                row(53220, Widget::Knob, "Decay", [6.10, 3.84, 2.71, 2.59]).p(88),
                row(53230, Widget::HSlider, "Tension", [6.10, 6.43, 2.71, 1.56]).p(92).h(Size::Flex(0.6)),
                row(53310, Widget::Knob, "Sustain", [8.81, 1.25, 2.72, 2.59]).p(96),
                row(53320, Widget::Knob, "Release", [8.81, 3.84, 2.72, 2.59]).p(100),
                row(53330, Widget::HSlider, "Tension", [8.81, 6.43, 2.72, 1.56]).p(104).h(Size::Flex(0.6)),
                row(53410, Widget::Display, "", [12.03, 1.75, 4.43, 3.37]).h(Size::Percent(50.0)).margins(0.5, 0.5, 0.5, 0.5),
                row(53420, Widget::HSlider, "Amount", [11.80, 5.62, 4.88, 2.37]).p(108).w(Size::Flex(0.9)),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "RES LFO",
            tab: 4,
            settings: &[(51100, 1.0)],
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(51100, Widget::List(Bar::Right), ENV_LFO, [0.00, 1.25, 3.39, 3.37]).h(Size::Flex(2.0)),
                row(51200, Widget::ToggleRight, "Global", [0.00, 4.62, 3.39, 1.68]).p(30),
                row(51300, Widget::ToggleRight, "Tempo", [0.00, 6.30, 3.39, 1.69]).p(35),
                row(52100, Widget::Spinner, "Shape", [3.39, 1.25, 2.51, 6.74]).p(40),
                row(52210, Widget::Knob, "Delay", [5.90, 1.25, 2.51, 3.37]).p(45),
                row(52220, Widget::Knob, "Attack", [5.90, 4.62, 2.51, 3.37]).p(50),
                row(52310, Widget::Knob, "Amount", [8.41, 1.25, 2.52, 3.37]).p(55),
                row(52320, Widget::Knob, "Speed", [8.41, 4.62, 2.52, 3.37]).p(60),
                row(52400, Widget::Display, "", [11.43, 1.75, 5.03, 5.74]).w(Size::Flex(2.0)).margins(0.5, 0.5, 0.5, 0.5),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "PITCH Envelope",
            tab: 5,
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(61100, Widget::List(Bar::Right), ENV_LFO, [0.00, 1.25, 3.39, 3.37]).h(Size::Flex(2.0)),
                row(61400, Widget::ToggleRight, "Enable", [0.00, 4.62, 3.39, 1.68]).p(65),
                row(61500, Widget::ToggleRight, "Tempo", [0.00, 6.30, 3.39, 1.69]).p(69),
                row(63110, Widget::Knob, "Delay", [3.39, 1.25, 2.71, 2.59]).p(73),
                row(63120, Widget::Knob, "Attack", [3.39, 3.84, 2.71, 2.59]).p(77),
                row(63130, Widget::HSlider, "Tension", [3.39, 6.43, 2.71, 1.56]).p(81).h(Size::Flex(0.6)),
                row(63210, Widget::Knob, "Hold", [6.10, 1.25, 2.71, 2.59]).p(85),
                row(63220, Widget::Knob, "Decay", [6.10, 3.84, 2.71, 2.59]).p(89),
                row(63230, Widget::HSlider, "Tension", [6.10, 6.43, 2.71, 1.56]).p(93).h(Size::Flex(0.6)),
                row(63310, Widget::Knob, "Sustain", [8.81, 1.25, 2.72, 2.59]).p(97),
                row(63320, Widget::Knob, "Release", [8.81, 3.84, 2.72, 2.59]).p(101),
                row(63330, Widget::HSlider, "Tension", [8.81, 6.43, 2.72, 1.56]).p(105).h(Size::Flex(0.6)),
                row(63410, Widget::Display, "", [12.03, 1.75, 4.43, 3.37]).h(Size::Percent(50.0)).margins(0.5, 0.5, 0.5, 0.5),
                row(63420, Widget::HSlider, "Amount", [11.80, 5.62, 4.88, 2.37]).p(109).w(Size::Flex(0.9)),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "PITCH LFO",
            tab: 5,
            settings: &[(61100, 1.0)],
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(61100, Widget::List(Bar::Right), ENV_LFO, [0.00, 1.25, 3.39, 3.37]).h(Size::Flex(2.0)),
                row(61200, Widget::ToggleRight, "Global", [0.00, 4.62, 3.39, 1.68]).p(31),
                row(61300, Widget::ToggleRight, "Tempo", [0.00, 6.30, 3.39, 1.69]).p(36),
                row(62100, Widget::Spinner, "Shape", [3.39, 1.25, 2.51, 6.74]).p(41),
                row(62210, Widget::Knob, "Delay", [5.90, 1.25, 2.51, 3.37]).p(46),
                row(62220, Widget::Knob, "Attack", [5.90, 4.62, 2.51, 3.37]).p(51),
                row(62310, Widget::Knob, "Amount", [8.41, 1.25, 2.52, 3.37]).p(56),
                row(62320, Widget::Knob, "Speed", [8.41, 4.62, 2.52, 3.37]).p(61),
                row(62400, Widget::Display, "", [11.43, 1.75, 5.03, 5.74]).w(Size::Flex(2.0)).margins(0.5, 0.5, 0.5, 0.5),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "FILT",
            tab: 6,
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(71000, Widget::Spacer, "", [0.00, 1.25, 1.06, 6.74]).w(Size::Flex(0.25)),
                row(72000, Widget::Spinner, "Type", [1.06, 1.25, 4.24, 6.74]).p(110),
                row(73000, Widget::Spacer, "", [5.30, 1.25, 1.06, 6.74]).w(Size::Flex(0.25)),
                row(74000, Widget::Knob, "Cutoff", [6.36, 1.25, 4.24, 6.74]).p(111),
                row(75000, Widget::Spacer, "", [10.60, 1.25, 1.06, 6.74]).w(Size::Flex(0.25)),
                row(76000, Widget::Knob, "Resonance", [11.66, 1.25, 4.24, 6.74]).p(112),
                row(77000, Widget::Spacer, "", [15.90, 1.25, 1.06, 6.74]).w(Size::Flex(0.25)),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "MISC Poly",
            tab: 7,
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(81000, Widget::List(Bar::Right), MISC, [0.00, 1.25, 3.39, 6.74]).w(Size::Percent(20.0)),
                row(82100, Widget::Knob, "Voices", [3.39, 1.25, 3.39, 6.74]).p(117),
                row(82200, Widget::Knob, "Slide", [6.78, 1.25, 3.39, 6.74]).p(118),
                row(82310, Widget::Toggle, "Porta", [10.17, 1.25, 3.39, 3.37]).p(119),
                row(82320, Widget::Toggle, "Mono", [10.17, 4.62, 3.39, 3.37]).p(120),
                row(82400, Widget::Knob, "Gate", [13.56, 1.25, 3.40, 6.74]).p(129),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "MISC Levels",
            tab: 7,
            settings: &[(81000, 1.0)],
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(81000, Widget::List(Bar::Right), MISC, [0.00, 1.25, 3.39, 6.74]).w(Size::Percent(20.0)),
                row(83110, Widget::Text, "Adjustment", [3.39, 1.25, 4.45, 1.00]).font(FontSize::Pt10).h(Size::U(1.0))
                    .align(Align2::CENTER_CENTER),
                row(83121, Widget::Knob, "Pan", [3.39, 2.25, 2.23, 2.87]).p(113),
                row(83122, Widget::Knob, "Volume", [5.62, 2.25, 2.22, 2.87]).p(114),
                row(83131, Widget::Knob, "Cutoff", [3.39, 5.12, 2.23, 2.87]).p(115),
                row(83132, Widget::Knob, "Resonance", [5.62, 5.12, 2.22, 2.87]).p(116),
                row(83200, Widget::Separator, "", [7.84, 1.25, 0.10, 6.74]).w(Size::U(0.1)),
                row(83310, Widget::Text, "Vol Tracking", [7.94, 1.25, 4.46, 1.00]).font(FontSize::Pt10).h(Size::U(1.0))
                    .align(Align2::CENTER_CENTER),
                row(83321, Widget::Knob, "Middle", [7.94, 2.25, 2.23, 2.87]).p(125),
                row(83322, Widget::Knob, "To Pan", [10.17, 2.25, 2.23, 2.87]).p(126),
                row(83331, Widget::Knob, "To Cutoff", [7.94, 5.12, 2.23, 2.87]).p(127),
                row(83332, Widget::Knob, "To Resonance", [10.17, 5.12, 2.23, 2.87]).p(128),
                row(83400, Widget::Separator, "", [12.40, 1.25, 0.10, 6.74]).w(Size::U(0.1)),
                row(83510, Widget::Text, "Key Tracking", [12.50, 1.25, 4.46, 1.00]).font(FontSize::Pt10).h(Size::U(1.0))
                    .align(Align2::CENTER_CENTER),
                row(83521, Widget::Knob, "Middle", [12.50, 2.25, 2.23, 2.87]).p(121),
                row(83522, Widget::Knob, "To Pan", [14.73, 2.25, 2.23, 2.87]).p(122),
                row(83531, Widget::Knob, "To Cutoff", [12.50, 5.12, 2.23, 2.87]).p(123),
                row(83532, Widget::Knob, "To Resonance", [14.73, 5.12, 2.23, 2.87]).p(124),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "MISC Arp",
            tab: 7,
            settings: &[(81000, 2.0)],
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(81000, Widget::List(Bar::Right), MISC, [0.00, 1.25, 3.39, 6.74]).w(Size::Percent(20.0)),
                row(84110, Widget::Spinner, "Direction", [3.39, 1.25, 2.71, 3.37]).p(139),
                row(84120, Widget::Knob, "Gate", [6.10, 1.25, 2.71, 3.37]).p(141),
                row(84130, Widget::Knob, "Time", [8.81, 1.25, 2.72, 3.37]).p(140),
                row(84140, Widget::Knob, "Range", [11.53, 1.25, 2.71, 3.37]).p(142),
                row(84150, Widget::Knob, "Repeat", [14.24, 1.25, 2.72, 3.37]).p(143),
                row(84210, Widget::Toggle, "Slide", [3.39, 4.62, 2.51, 3.37]).p(144),
                // The box sits right of the caption `Chord: ` (8.57, w 7.89); the control is the whole cell.
                row(84220, Widget::MenuButton, "Chord", [6.40, 5.55, 10.06, 1.50]).p(145).w(Size::Flex(4.0)).h(Size::U(1.5)).margins(0.0, 0.5, 0.0, 0.5)
                    .items_per_column(46),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "MISC Echo",
            tab: 7,
            settings: &[(81000, 3.0)],
            rows: &[
                row(0, Widget::Tabs, TABS, [0.0, 0.0, 16.96, 1.25]).h(Size::U(1.25)),
                row(81000, Widget::List(Bar::Right), MISC, [0.00, 1.25, 3.39, 6.74]).w(Size::Percent(20.0)),
                row(85100, Widget::VSlider, "Echoes", [3.39, 1.25, 2.71, 6.74]).p(130),
                row(85210, Widget::Knob, "Time", [6.10, 1.25, 2.71, 3.37]).p(132),
                row(85220, Widget::Knob, "Feedback", [6.10, 4.62, 2.71, 3.37]).p(131),
                row(85310, Widget::Knob, "Pan", [8.81, 1.25, 2.72, 3.37]).p(133),
                row(85320, Widget::Knob, "Cutoff", [8.81, 4.62, 2.72, 3.37]).p(135),
                row(85410, Widget::Knob, "Pitch", [11.53, 1.25, 2.71, 3.37]).p(134),
                row(85420, Widget::Knob, "Resonance", [11.53, 4.62, 2.71, 3.37]).p(136),
                row(85510, Widget::Toggle, "Ping Pong", [14.24, 1.25, 2.72, 3.37]).p(137),
                row(85520, Widget::Toggle, "Fat Mode", [14.24, 4.62, 2.72, 3.37]).p(138),
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use eframe::epaint::ColorMode;

    use super::*;
    use crate::panels::place;
    use crate::panels::tests::{U, display_shapes};
    use crate::theme::theme_by_id;

    /// The hairline of the pure-geometry tests; their rect is [`rect`].
    const T: f32 = 2.0;
    const SINE: f32 = 0.0;
    const TRIANGLE: f32 = 0.5;
    const SQUARE: f32 = 1.0;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    /// x 10, y 20, 160 × 100: with t = 2, value v sits at y = 21 + 98·(1 − v) and seg = 32.
    fn rect() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), vec2(160.0, 100.0))
    }

    /// The value (0…1) a point of [`rect`] stands for.
    fn value(p: Pos2) -> f32 {
        1.0 - (p.y - 21.0) / 98.0
    }

    /// The registry defaults with params `(p, v)` set.
    fn with(set: &[(usize, f32)]) -> Vec<f32> {
        let mut p = ModuleInstance::new(TYPE_ID, 2).params;
        for &(i, v) in set {
            p[i] = v;
        }
        p
    }

    /// The defaults with LFO `i`'s Shape, Delay, Attack, Amount and Speed set.
    fn lfo_with(i: usize, [shape, delay, attack, amount, speed]: [f32; 5]) -> Vec<f32> {
        with(&[
            (37 + i, shape),
            (42 + i, delay),
            (47 + i, attack),
            (52 + i, amount),
            (57 + i, speed),
        ])
    }

    /// The values of LFO 0's points in [`rect`].
    fn lfo_values(lfo: [f32; 5]) -> Vec<f32> {
        lfo_points(&lfo_with(0, lfo), 0, rect(), T)
            .into_iter()
            .map(value)
            .collect()
    }

    #[test]
    fn a_value_is_drawn_at_y_plus_half_t_plus_h_minus_t_times_one_minus_v() {
        assert_eq!(value_y(rect(), T, 0.0), 119.0);
        assert_eq!(value_y(rect(), T, 0.5), 70.0);
        assert_eq!(value_y(rect(), T, 1.0), 21.0);
    }

    #[test]
    fn the_lfo_shows_16_minus_14_speed_cycles_at_32_samples_each() {
        for (speed, count) in [(0.0, 512), (1.0, 64), (0.5, 288), (0.25, 400), (0.3, 378)] {
            let points = lfo_points(&lfo_with(0, [SINE, 0.0, 0.0, 1.0, speed]), 0, rect(), T);
            assert_eq!(points.len(), count, "Speed {speed}");
            let s = 1.0 / (32.0 * (16.0 - 14.0 * speed));
            for (k, p) in points.iter().enumerate() {
                assert!(
                    near(p.x, 10.0 + k as f32 * s * 160.0),
                    "Speed {speed} sample {k}: {p:?}"
                );
            }
            // f < 1: the last sample is short of the right edge.
            assert!(points[count - 1].x < 170.0);
            // It starts at the left edge on the centre line, which is sample 0 of a Sine.
            assert_eq!(points[0], pos2(10.0, 70.0));
            // A Square at full depth from phase 0 starts there too, and rises to sample 0 at the left edge.
            let square = lfo_points(&lfo_with(0, [SQUARE, 0.0, 0.0, 1.0, speed]), 0, rect(), T);
            assert_eq!(square.len(), count + 1, "Speed {speed}");
            assert_eq!(square[..2], [pos2(10.0, 70.0), pos2(10.0, 21.0)]);
            assert!(
                square[1..].iter().zip(&points).all(|(a, b)| a.x == b.x),
                "Speed {speed}"
            );
        }
    }

    #[test]
    fn the_lfo_shapes_read_phase_n_over_32() {
        // Speed 1: 64 samples, two cycles; Amount 1, no Delay, no Attack.
        let sine = lfo_values([SINE, 0.0, 0.0, 1.0, 1.0]);
        for (n, v) in [
            (0, 0.5),
            (4, 0.853_553),
            (8, 1.0),
            (16, 0.5),
            (24, 0.0),
            (32, 0.5),
            (40, 1.0),
        ] {
            assert!(near(sine[n], v), "sine {n}: {}", sine[n]);
        }
        let triangle = lfo_values([TRIANGLE, 0.0, 0.0, 1.0, 1.0]);
        for (n, v) in [
            (0, 0.5),
            (4, 0.75),
            (8, 1.0),
            (12, 0.75),
            (16, 0.5),
            (24, 0.0),
            (28, 0.25),
            (40, 1.0),
        ] {
            assert!(near(triangle[n], v), "triangle {n}: {}", triangle[n]);
        }
        // The centre start point, then the 64 samples.
        let square = lfo_values([SQUARE, 0.0, 0.0, 1.0, 1.0]);
        assert_eq!((square.len(), square[0]), (1 + 64, 0.5));
        for (n, &v) in square[1..].iter().enumerate() {
            assert!(
                near(v, if n % 32 < 16 { 1.0 } else { 0.0 }),
                "square {n}: {v}"
            );
        }
    }

    #[test]
    fn the_lfo_holds_the_centre_while_f_is_under_half_the_delay() {
        // Delay 0.5 at Speed 1: f < 0.25 for samples 0…15; sample 16 is the first post-delay one, phase 0.
        let v = lfo_values([SQUARE, 0.5, 0.0, 1.0, 1.0]);
        assert!(v[..16].iter().all(|&v| v == 0.5), "{v:?}");
        assert!(near(v[16], 1.0) && near(v[31], 1.0) && near(v[32], 0.0) && near(v[48], 1.0));
        // Delay 1: the first half of the width.
        let v = lfo_values([SINE, 1.0, 0.0, 1.0, 1.0]);
        assert!(v[..32].iter().all(|&v| v == 0.5));
        assert!(near(v[40], 1.0));
    }

    /// g of a Square at Amount 1: the distance from the centre, doubled.
    fn depth(values: Vec<f32>) -> Vec<f32> {
        values.iter().map(|v| (2.0 * v - 1.0).abs()).collect()
    }

    #[test]
    fn the_attack_ramp_reaches_full_depth_after_tan_attack_pi_over_8_of_the_width() {
        let s = 1.0 / 64.0;
        for attack in [0.25, 0.5, 1.0] {
            let reach = (attack * FRAC_PI_8).tan();
            let g = depth(lfo_values([SQUARE, 0.0, attack, 1.0, 1.0]));
            assert_eq!(g[0], 0.0, "Attack {attack}: g starts at 0");
            for (k, &g) in g.iter().enumerate() {
                assert!(
                    near(g, (k as f32 * s / reach).min(1.0)),
                    "Attack {attack} sample {k}: {g}"
                );
            }
            let full = g.iter().position(|&g| g == 1.0).unwrap();
            assert!(
                full as f32 * s >= reach && (full - 1) as f32 * s < reach,
                "Attack {attack}: full at {full}"
            );
        }
        // After a delay the ramp starts at the first post-delay sample.
        let g = depth(lfo_values([SQUARE, 0.5, 1.0, 1.0, 1.0]));
        assert_eq!(g[16], 0.0);
        assert!(near(g[17], s / FRAC_PI_8.tan()));
        // Attack 0: full depth at once, so the line rises (Amount ≥ 0.5) or falls from the centre at the left edge.
        let start =
            |amount: f32| lfo_points(&lfo_with(0, [SQUARE, 0.0, 0.0, amount, 1.0]), 0, rect(), T);
        assert_eq!(
            start(1.0)[..3],
            [pos2(10.0, 70.0), pos2(10.0, 21.0), pos2(12.5, 21.0)]
        );
        assert_eq!(
            start(0.0)[..3],
            [pos2(10.0, 70.0), pos2(10.0, 119.0), pos2(12.5, 119.0)]
        );
    }

    #[test]
    fn the_amount_below_half_flips_the_wave_and_half_is_flat() {
        for amount in [1.0, 0.75, 0.6] {
            let up = lfo_values([SINE, 0.0, 0.0, amount, 1.0]);
            let down = lfo_values([SINE, 0.0, 0.0, 1.0 - amount, 1.0]);
            for (a, b) in up.iter().zip(&down) {
                assert!(near(a + b, 1.0), "Amount {amount}: {a} {b}");
            }
            // The peak is at 0.5 + 0.5·|2·Amount − 1|, upward for Amount ≥ 0.5.
            assert!(near(up[8], amount), "Amount {amount}: {}", up[8]);
        }
        assert!(
            lfo_values([SINE, 0.0, 0.0, 0.5, 1.0])
                .iter()
                .all(|&v| v == 0.5)
        );
    }

    #[test]
    fn the_tension_law_is_straight_at_0_and_bows_up_or_down() {
        for u in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert_eq!(tension(u, 0.0), u);
        }
        let table = [
            (1.0, [0.802_099, 0.961_848, 0.993_663]),
            (0.5, [0.473_944, 0.748_598, 0.907_763]),
            (-0.5, [0.092_237, 0.251_402, 0.526_056]),
            (-1.0, [0.006_337, 0.038_152, 0.197_901]),
        ];
        for (tau, quarters) in table {
            assert!(
                near(tension(0.0, tau), 0.0) && near(tension(1.0, tau), 1.0),
                "τ {tau}"
            );
            for (u, g) in [0.25, 0.5, 0.75].into_iter().zip(quarters) {
                assert!(
                    near(tension(u, tau), g),
                    "τ {tau} u {u}: {}",
                    tension(u, tau)
                );
                // −τ mirrors τ.
                assert!(near(tension(u, -tau), 1.0 - tension(1.0 - u, tau)));
            }
        }
        // A tension a hair off 0.5 is (almost) straight, not noise.
        assert!(near(tension(0.5, 1e-6), 0.5));
    }

    /// Envelope `e`'s Delay, Attack, Hold, Decay, Sustain, Release, Amount and the three tensions set.
    fn envelope_with(
        e: usize,
        [d, a, h, dc, s, r, amount]: [f32; 7],
        [ta, td, tr]: [f32; 3],
    ) -> Vec<f32> {
        let set = [70, 74, 82, 86, 94, 98, 106, 78, 90, 102].map(|p| p + e);
        let values = [d, a, h, dc, s, r, amount, ta, td, tr];
        with(&set.into_iter().zip(values).collect::<Vec<_>>())
    }

    const STRAIGHT: [f32; 3] = [0.5, 0.5, 0.5];
    const HAND: [f32; 7] = [0.5, 0.25, 0.5, 0.5, 0.5, 1.0, 0.0];

    #[test]
    fn the_envelope_breakpoints_follow_the_params() {
        // base 119, seg 32, H 98; Amount 0 is A = 1.
        let points = [
            pos2(26.0, 119.0),
            pos2(34.0, 21.0),
            pos2(50.0, 21.0),
            pos2(58.0, 70.0),
            pos2(90.0, 119.0),
        ];
        for e in 0..4 {
            let (line, dots) = envelope(&envelope_with(e, HAND, STRAIGHT), e, rect(), T, 1.0);
            assert_eq!(dots, points, "envelope {e}");
            assert_eq!(line, [&[pos2(10.0, 119.0)], &points[..]].concat());
        }
        // Amount 1 is the same; Amount 0.75 is A = 0.5. The curve is never inverted.
        let amount = |a: f32| {
            let mut v = HAND;
            v[6] = a;
            envelope(&envelope_with(1, v, STRAIGHT), 1, rect(), T, 1.0).1
        };
        assert_eq!(amount(1.0), points);
        assert_eq!(amount(0.75)[1], pos2(34.0, 70.0));
        assert_eq!(amount(0.25)[3], pos2(58.0, 94.5));
        // Sustain 1: no decay width; Release 0: P5 under P4.
        let (_, dots) = envelope(
            &envelope_with(0, [0.0, 0.5, 0.0, 1.0, 1.0, 0.0, 1.0], STRAIGHT),
            0,
            rect(),
            T,
            1.0,
        );
        assert_eq!(
            dots,
            [
                pos2(10.0, 119.0),
                pos2(26.0, 21.0),
                pos2(26.0, 21.0),
                pos2(26.0, 21.0),
                pos2(26.0, 119.0)
            ]
        );
    }

    #[test]
    fn a_tensioned_segment_has_round_dx_pieces_lagging_u_by_one() {
        // Attack 0.25: 8 points wide from (10, 119) up to (18, 21) at τ = 1.
        for ppp in [1.0, 2.0] {
            let p = envelope_with(0, [0.0, 0.25, 0.0, 0.0, 1.0, 0.0, 1.0], [1.0, 0.5, 0.5]);
            let (line, _) = envelope(&p, 0, rect(), T, ppp);
            let n = 8 * ppp as usize;
            assert_eq!(line.len(), 1 + n + 1, "ppp {ppp}: {line:?}");
            for (i, q) in line[1..=n].iter().enumerate() {
                let u = i as f32 / (n - 1) as f32;
                assert!(
                    near(q.x, 10.0 + (i + 1) as f32 * 8.0 / n as f32),
                    "ppp {ppp} piece {i}: {q:?}"
                );
                assert!(
                    near(q.y, 119.0 + tension(u, 1.0) * (21.0 - 119.0)),
                    "ppp {ppp} piece {i}: {q:?}"
                );
            }
            // The first piece is flat (u = 0) and the last ends at P2; τ > 0 bows toward the top.
            assert!(near(line[1].y, 119.0));
            assert!(near(line[n].x, 18.0) && near(line[n].y, 21.0));
            assert!(line[n / 2].y < 70.0);
        }
        // Falling at τ = −1 (Release Tension 0): y = y1 − G(1 − u)·(y1 − y0), bowed toward the bottom.
        let p = envelope_with(0, [0.0, 0.0, 0.0, 0.0, 1.0, 0.25, 1.0], [0.5, 0.5, 0.0]);
        let (line, _) = envelope(&p, 0, rect(), T, 1.0);
        assert_eq!(line.len(), 2 + 8, "{line:?}");
        for (i, q) in line[2..].iter().enumerate() {
            let u = i as f32 / 7.0;
            assert!(
                near(q.y, 119.0 - tension(1.0 - u, -1.0) * (119.0 - 21.0)),
                "piece {i}: {q:?}"
            );
        }
        assert!(line[6].y > 70.0);
    }

    #[test]
    fn a_segment_under_1_px_or_flat_is_straight() {
        // Attack 0.9 points wide: 0.9 px at ppp 1 (straight), 1.8 px at ppp 2 (two pieces). Release 0 drops
        // straight back to the base: P0, the attack, P5.
        let p = envelope_with(
            0,
            [0.0, 0.9 / 32.0, 0.0, 0.0, 1.0, 0.0, 1.0],
            [1.0, 0.5, 0.5],
        );
        assert_eq!(envelope(&p, 0, rect(), T, 1.0).0.len(), 1 + 1 + 1);
        assert_eq!(envelope(&p, 0, rect(), T, 2.0).0.len(), 1 + 2 + 1);
        // 1.2 px rounds to N = 1, which has no u = i/(N − 1): straight.
        let p = envelope_with(
            0,
            [0.0, 1.2 / 32.0, 0.0, 0.0, 1.0, 0.0, 1.0],
            [1.0, 0.5, 0.5],
        );
        assert_eq!(envelope(&p, 0, rect(), T, 1.0).0.len(), 1 + 1 + 1);
        // Amount 0.5: every segment is flat on the base line.
        let p = envelope_with(0, [0.0, 0.5, 0.0, 0.5, 0.5, 0.5, 0.5], [1.0, 1.0, 1.0]);
        let (line, _) = envelope(&p, 0, rect(), T, 1.0);
        assert!(
            line.iter().all(|q| q.y == 119.0) && line.len() <= 6,
            "{line:?}"
        );
    }

    /// The content rect at the reference (U = 32 points at any `ppp`) and display `#n`'s rect in it.
    fn reference(n: u32, state: &PanelState, ppp: f32) -> (Rect, Rect) {
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(16.96 * U, 7.99 * U));
        let (panel, module) = (panel().unwrap(), ModuleInstance::new(TYPE_ID, 2));
        let placed = place(&panel, &module, state, content * ppp, U * ppp);
        (
            content,
            placed.iter().find(|(l, _)| l.id == n).unwrap().1 / ppp,
        )
    }

    /// Tab `tab` with the sub-page list `list` (if any) on `item`.
    fn on(tab: usize, list: Option<(u32, f32)>) -> PanelState {
        let mut state = PanelState {
            tab,
            ..Default::default()
        };
        state.settings.extend(list);
        state
    }

    #[test]
    fn each_lfo_display_draws_its_own_lfo_over_a_control_fill() {
        let fill = theme_by_id(0).colors.control_fill.to_color32();
        // Five different LFOs.
        let mut p = ModuleInstance::new(TYPE_ID, 2).params;
        for i in 0..5 {
            let lfo = [SINE, TRIANGLE, SQUARE, SINE, TRIANGLE][i];
            let k = i as f32;
            for (base, v) in [
                (37, lfo),
                (42, 0.1 * k),
                (47, 0.2),
                (52, 0.9 - 0.1 * k),
                (57, 0.25 * k),
            ] {
                p[base + i] = v;
            }
        }
        for ppp in [1.0, 2.0] {
            for (i, (n, state)) in [
                (22400, on(1, None)),
                (32400, on(2, Some((31100, 1.0)))),
                (42400, on(3, Some((41100, 1.0)))),
                (52400, on(4, Some((51100, 1.0)))),
                (62400, on(5, Some((61100, 1.0)))),
            ]
            .into_iter()
            .enumerate()
            {
                let (content, rect) = reference(n, &state, ppp);
                let shapes = display_shapes(TYPE_ID, n, &p, &state, content, ppp);
                assert_eq!(shapes.len(), 2, "#{n} ppp {ppp}");
                let Shape::Rect(back) = &shapes[0] else {
                    panic!("{:?}", shapes[0])
                };
                assert_eq!((back.rect, back.fill), (rect, fill));
                let Shape::Path(line) = &shapes[1] else {
                    panic!("{:?}", shapes[1])
                };
                // t = ceil(U/16) px is 2 points at ppp 1 and 2.
                assert_eq!(line.points, lfo_points(&p, i, rect, 2.0), "#{n} ppp {ppp}");
                assert_ne!(line.points, lfo_points(&p, (i + 1) % 5, rect, 2.0));
                assert_eq!(
                    (line.stroke.width, &line.stroke.color),
                    (2.0, &ColorMode::Solid(Color32::RED))
                );
            }
        }
    }

    #[test]
    fn each_envelope_display_draws_the_others_dim_then_its_own_and_the_dots() {
        let fill = theme_by_id(0).colors.control_fill.to_color32();
        // Four different envelopes; RES's Amount is 0.5, so it is skipped unless it is the current one.
        let mut p = ModuleInstance::new(TYPE_ID, 2).params;
        for (e, amount) in [0.8, 0.9, 0.5, 0.2].into_iter().enumerate() {
            let k = e as f32;
            for (base, v) in [
                (70, 0.1 * k),
                (74, 0.25),
                (82, 0.2),
                (86, 0.5),
                (94, 0.3 + 0.1 * k),
                (98, 0.6),
                (106, amount),
                (78, 0.9 - 0.2 * k),
                (90, 0.2),
                (102, 0.7),
            ] {
                p[base + e] = v;
            }
        }
        let t = 2.0;
        let (dim, full) = (Color32::RED.gamma_multiply(0.2), Color32::RED);
        for ppp in [1.0, 2.0] {
            // VOL … PITCH (tabs 2–5) are envelopes 0–3, each on its list's `Envelope` item.
            for tab in 2..=5 {
                let (e, n) = (tab - 2, (tab as u32 + 1) * 10_000);
                let state = on(tab, Some((n + 1100, 0.0)));
                let (content, rect) = reference(n + 3410, &state, ppp);
                let others: Vec<usize> = (0..4).filter(|&o| o != e && o != 2).collect();
                // Enable is on above 0.5, as the engine applies it (`three_osc::toggle`).
                for enable in [0.0, 0.5, 1.0] {
                    p[62 + e] = enable;
                    let shapes = display_shapes(TYPE_ID, n + 3410, &p, &state, content, ppp);
                    let rings = if enable <= 0.5 { 5 } else { 0 };
                    assert_eq!(
                        shapes.len(),
                        1 + others.len() + 1 + 5 + rings,
                        "tab {tab} ppp {ppp} Enable {enable}"
                    );
                    let Shape::Rect(back) = &shapes[0] else {
                        panic!("{:?}", shapes[0])
                    };
                    assert_eq!((back.rect, back.fill), (rect, fill));
                    let lines = others.iter().map(|&o| (o, dim)).chain([(e, full)]);
                    for (shape, (o, color)) in shapes[1..].iter().zip(lines) {
                        let Shape::Path(line) = shape else {
                            panic!("{shape:?}")
                        };
                        assert_eq!(
                            line.points,
                            envelope(&p, o, rect, t, ppp).0,
                            "tab {tab} envelope {o} ppp {ppp}"
                        );
                        assert_eq!(
                            (line.stroke.width, &line.stroke.color),
                            (t, &ColorMode::Solid(color))
                        );
                    }
                    let dots = envelope(&p, e, rect, t, ppp).1;
                    for (k, shape) in shapes[2 + others.len()..].iter().enumerate() {
                        let Shape::Circle(c) = shape else {
                            panic!("{shape:?}")
                        };
                        // Discs of radius 2t first, then (Enable off) control-fill centres of radius t: rings.
                        let (radius, color) = if k < 5 {
                            (2.0 * t, Color32::RED)
                        } else {
                            (t, fill)
                        };
                        assert_eq!(
                            (c.center, c.radius, c.fill),
                            (dots[k % 5], radius, color),
                            "tab {tab} dot {k}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn wild_param_values_draw_bounded_without_panicking() {
        // The FLM loader copies PRMS values as they are. Out-of-range ones first: unclamped, they fail fast where the
        // huge ones would hang (Speed −1e30: about 2³¹ samples) or panic (Decay 1e30: usize::MAX pieces).
        let wild = [
            -1.0,
            2.0,
            f32::NAN,
            -1e30,
            f32::NEG_INFINITY,
            1e30,
            f32::INFINITY,
        ];
        let inside = |q: &Pos2| q.is_finite() && rect().contains(*q);
        for v in wild {
            let points = lfo_points(&lfo_with(0, [v; 5]), 0, rect(), T);
            assert!(
                (64..=1 + 512).contains(&points.len()),
                "LFO at {v}: {} points",
                points.len()
            );
            assert!(points.iter().all(inside), "LFO at {v}");
            // Every envelope param at v, and Decay at v under a tension off the centre.
            let all = envelope_with(0, [v; 7], [v; 3]);
            let decay = envelope_with(0, [0.0, 0.5, 0.0, v, 0.0, 0.5, 1.0], [0.5, 0.9, 0.5]);
            for (p, ppp) in [(&all, 1.0), (&all, 2.0), (&decay, 1.0), (&decay, 2.0)] {
                let (line, dots) = envelope(p, 0, rect(), T, ppp);
                // At most P0, round(160 px) pieces per segment and a riser each.
                assert!(
                    line.len() <= 1 + 5 * (160 * ppp as usize + 1),
                    "envelope at {v}: {} points",
                    line.len()
                );
                assert!(line.iter().chain(&dots).all(inside), "envelope at {v}");
            }
        }
        // Through the draw fns: every LFO and envelope param of the module wild.
        for v in wild {
            let mut p = ModuleInstance::new(TYPE_ID, 2).params;
            p[37..110].fill(v);
            let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(16.96 * U, 7.99 * U));
            for (n, state) in [(22400, on(1, None)), (33410, on(2, Some((31100, 0.0))))] {
                let shapes = display_shapes(TYPE_ID, n, &p, &state, content, 1.0);
                assert!(shapes.len() >= 2, "#{n} at {v}");
            }
        }
    }

    #[test]
    fn the_wiring_names_the_custom_waveform_by_its_path_stem() {
        use crate::flm::chunk::ContentRoot;
        let named = |path: Option<(ContentRoot, &str)>| {
            let mut module = ModuleInstance::new(TYPE_ID, 2);
            if let Some((root, relative)) = path {
                let path = ContentPath::Shorthand {
                    root,
                    relative: relative.into(),
                };
                let file = std::path::Path::new("x.wav");
                crate::project_files::load_pick(&mut module, file, &[], Some(path)).unwrap();
            }
            let mut state = PanelState {
                sample_name: "stale".into(),
                ..PanelState::default()
            };
            wire(&module, &mut state);
            state.sample_name
        };
        assert_eq!(named(None), "");
        let built_in = (ContentRoot::App, "Built-in/Shapes/Vowel A.wav");
        assert_eq!(named(Some(built_in)), "Vowel A");
        let mine = (ContentRoot::User, "My Samples/Waves\\Bright.Saw 2.wav");
        assert_eq!(named(Some(mine)), "Bright.Saw 2");
    }

    #[test]
    fn custom_is_picked_when_an_oscillator_shape_lands_on_it() {
        let mut module = ModuleInstance::new(TYPE_ID, 2);
        module.params[2] = 1.0;
        module.params[3] = 0.5;
        assert!(custom_picked(&module, &[0, 2]));
        // Saw, another param, nothing changed.
        assert!(!custom_picked(&module, &[3]));
        module.params[3] = 1.0;
        assert!(custom_picked(&module, &[3]));
        module.params[3] = 0.5;
        module.params[24] = 1.0;
        assert!(!custom_picked(&module, &[24]));
        assert!(!custom_picked(&module, &[]));
        // The same values on another module type.
        let mut other = ModuleInstance::new(0, 2);
        other.params[2] = 1.0;
        assert!(!custom_picked(&other, &[2]));
    }
}

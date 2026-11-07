// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The spike editor's picture and its hit regions, in logical pixels. Shapes only (no text, no polish: the dedicated
//! synthesizer interface comes later): a dark panel with warm orange accents, a large knob, a fader and a four-way
//! choice, laid out to be touch-sized.

use os_ui::{Canvas, Color, Rect};

use crate::params::{CHOICE, CHOICE_MAX, FADER, KNOB};

/// The editor's size at scale 1.
pub const W: u32 = 360;
pub const H: u32 = 200;

/// Drag lengths in px for the whole range of a knob and of the fader.
pub const KNOB_PX: f32 = 150.0;
pub const FADER_PX: f32 = 120.0;

const BG: Color = Color::rgb(0x14110f);
const PANEL: Color = Color::rgb(0x1f1a17);
const EDGE: Color = Color::rgb(0x3a2f27);
const TRACK: Color = Color::rgb(0x3b322c);
const CAP: Color = Color::rgb(0x2a231f);
const ORANGE: Color = Color::rgb(0xff8a2a);

/// What a hit region is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Knob,
    Fader,
    /// The choice's value this cell picks.
    Choice(i32),
}

/// A control's id, kind and rect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub id: &'static str,
    pub kind: Kind,
    pub r: Rect,
}

const KNOB_C: (f32, f32) = (90.0, 100.0);
const KNOB_R: f32 = 46.0;
const FADER_X: f32 = 190.0;
const FADER_TOP: f32 = 40.0;
const FADER_LEN: f32 = 120.0;

/// The knob's centre.
pub fn knob_center() -> (f32, f32) {
    KNOB_C
}

/// The knob's hit region: its disc, a margin around it.
fn knob_rect() -> Rect {
    let r = KNOB_R + 6.0;
    Rect::new(KNOB_C.0 - r, KNOB_C.1 - r, 2.0 * r, 2.0 * r)
}

/// The fader's hit region.
pub fn fader_rect() -> Rect {
    Rect::new(FADER_X - 22.0, FADER_TOP - 6.0, 44.0, FADER_LEN + 12.0)
}

/// Cell `i` of the choice (`0..=CHOICE_MAX`).
pub fn choice_rect(i: i32) -> Rect {
    Rect::new(250.0, 36.0 + 38.0 * i as f32, 90.0, 32.0)
}

/// Paints the editor with every control at its normalised value `norm(id)`, `pressed` the one held; returns the hit
/// regions, the last one on top.
pub fn paint(c: &mut Canvas, norm: &dyn Fn(&str) -> f32, pressed: Option<&str>) -> Vec<Hit> {
    let mut hits = Vec::new();
    c.begin_frame(None);
    c.clear(BG);
    let panel = Rect::new(8.0, 8.0, W as f32 - 16.0, H as f32 - 16.0);
    c.fill_rrect(panel, 10.0, PANEL);
    c.stroke_rrect(panel, 10.0, 1.0, EDGE);

    // The knob: a 270° track from the lower left round to the lower right, filled up to the value.
    let (cx, cy) = KNOB_C;
    let v = norm(KNOB).clamp(0.0, 1.0);
    let (start, sweep) = (0.75 * std::f32::consts::PI, 1.5 * std::f32::consts::PI);
    c.arc(cx, cy, KNOB_R - 4.0, start, sweep, 7.0, TRACK);
    if v > 0.001 {
        c.arc(cx, cy, KNOB_R - 4.0, start, sweep * v, 7.0, ORANGE);
    }
    c.fill_circle(
        cx,
        cy,
        KNOB_R - 16.0,
        if pressed == Some(KNOB) { EDGE } else { CAP },
    );
    let a = start + sweep * v;
    c.line(
        cx + (KNOB_R - 36.0) * a.cos(),
        cy + (KNOB_R - 36.0) * a.sin(),
        cx + (KNOB_R - 20.0) * a.cos(),
        cy + (KNOB_R - 20.0) * a.sin(),
        3.0,
        ORANGE,
    );
    hits.push(Hit {
        id: KNOB,
        kind: Kind::Knob,
        r: knob_rect(),
    });

    // The fader: a vertical track filled from the bottom, with a handle.
    let v = norm(FADER).clamp(0.0, 1.0);
    let bottom = FADER_TOP + FADER_LEN;
    let handle_y = bottom - v * FADER_LEN;
    c.fill_rrect(
        Rect::new(FADER_X - 3.0, FADER_TOP, 6.0, FADER_LEN),
        3.0,
        TRACK,
    );
    c.fill_rrect(
        Rect::new(FADER_X - 3.0, handle_y, 6.0, bottom - handle_y),
        3.0,
        ORANGE,
    );
    let handle = Rect::new(FADER_X - 15.0, handle_y - 10.0, 30.0, 20.0);
    c.fill_rrect(
        handle,
        6.0,
        if pressed == Some(FADER) { ORANGE } else { CAP },
    );
    c.stroke_rrect(handle, 6.0, 2.0, ORANGE);
    hits.push(Hit {
        id: FADER,
        kind: Kind::Fader,
        r: fader_rect(),
    });

    // The choice: four cells, the picked one lit.
    let picked = (norm(CHOICE).clamp(0.0, 1.0) * CHOICE_MAX as f32).round() as i32;
    for i in 0..=CHOICE_MAX {
        let r = choice_rect(i);
        c.fill_rrect(r, 8.0, if i == picked { ORANGE } else { TRACK });
        hits.push(Hit {
            id: CHOICE,
            kind: Kind::Choice(i),
            r,
        });
    }
    hits
}

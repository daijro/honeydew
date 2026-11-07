// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Rind's parameters: one flat list built from the permanent table ([`crate::ids::SPECS`]). Continuous parameters are
//! `FloatParam`s and discrete ones `IntParam`s, all exposed normalised to the host as nih-plug does.

use std::sync::Arc;

use nih_plug::prelude::*;
use os_plugkit::live::LiveEdits;

use crate::ids::{COUNT, Fmt, Kind, SPECS, Spec};

/// A parameter's plain value for the sound, as an `f32` whatever its kind.
pub type Snapshot = [f32; COUNT];

enum Slot {
    F(FloatParam),
    I(IntParam),
}

impl Slot {
    fn ptr(&self) -> ParamPtr {
        match self {
            Slot::F(p) => p.as_ptr(),
            Slot::I(p) => p.as_ptr(),
        }
    }
}

pub struct RindParams {
    slots: Vec<Slot>,
    /// The editor's edits the host has not handed back yet. The sound reads every parameter through it
    /// ([`LiveEdits::value`]), so an edit sounds at once.
    pub live: Arc<LiveEdits>,
}

/// `v` as text, by `fmt`.
pub fn format(fmt: Fmt, v: f32) -> String {
    match fmt {
        Fmt::Plain => format!("{v:.2}"),
        Fmt::Int => format!("{}", v.round() as i32),
        Fmt::Pct => format!("{:.0}%", v * 100.0),
        Fmt::Hz => {
            if v >= 1000.0 {
                format!("{:.2} kHz", v / 1000.0)
            } else if v >= 100.0 {
                format!("{v:.0} Hz")
            } else {
                format!("{v:.1} Hz")
            }
        }
        Fmt::Sec => {
            if v < 0.01 {
                format!("{:.1} ms", v * 1000.0)
            } else if v < 1.0 {
                format!("{:.0} ms", v * 1000.0)
            } else {
                format!("{v:.2} s")
            }
        }
        Fmt::Ms => format!("{v:.1} ms"),
        Fmt::Db => format!("{v:+.1} dB"),
        Fmt::Semi => format!("{:+} st", v.round() as i32),
        Fmt::Cents => format!("{v:+.0} ct"),
        Fmt::Pan => {
            let p = (v * 100.0).round() as i32;
            match p {
                0 => "C".to_owned(),
                p if p < 0 => format!("L{}", -p),
                p => format!("R{p}"),
            }
        }
        Fmt::Oct => format!("{:+} oct", v.round() as i32),
        Fmt::Ratio => format!("{v:.1}:1"),
        Fmt::Names(names) => {
            let i = (v.round().max(0.0) as usize).min(names.len().saturating_sub(1));
            names.get(i).copied().unwrap_or("").to_owned()
        }
    }
}

fn float(s: &Spec, lo: f32, hi: f32, skew: Option<f32>) -> FloatParam {
    let range = match skew {
        None => FloatRange::Linear { min: lo, max: hi },
        Some(k) => FloatRange::Skewed {
            min: lo,
            max: hi,
            factor: FloatRange::skew_factor(k),
        },
    };
    let fmt = s.fmt;
    FloatParam::new(s.name, s.default, range)
        .with_value_to_string(Arc::new(move |v| format(fmt, v)))
}

fn slot(s: &Spec) -> Slot {
    match s.kind {
        Kind::Lin(lo, hi) => Slot::F(float(s, lo, hi, None)),
        Kind::Skew(lo, hi, k) => Slot::F(float(s, lo, hi, Some(k))),
        Kind::Int(lo, hi) => {
            let fmt = s.fmt;
            Slot::I(
                IntParam::new(
                    s.name,
                    s.default as i32,
                    IntRange::Linear { min: lo, max: hi },
                )
                .with_value_to_string(Arc::new(move |v| format(fmt, v as f32))),
            )
        }
    }
}

impl Default for RindParams {
    fn default() -> Self {
        RindParams {
            slots: SPECS.iter().map(slot).collect(),
            live: Arc::new(LiveEdits::default()),
        }
    }
}

impl RindParams {
    /// Every parameter's plain value for the sound: the editor's pending edit, else the host's.
    pub fn snapshot(&self, out: &mut Snapshot) {
        for (o, s) in out.iter_mut().zip(&self.slots) {
            *o = match s {
                Slot::F(p) => self.live.value(p),
                Slot::I(p) => self.live.value(p) as f32,
            };
        }
    }

    /// The default snapshot.
    pub fn defaults() -> Snapshot {
        let mut out = [0.0; COUNT];
        for (o, s) in out.iter_mut().zip(SPECS.iter()) {
            *o = s.default;
        }
        out
    }
}

/// The index of parameter `id` in the table.
pub fn index(id: &str) -> Option<usize> {
    SPECS.iter().position(|s| s.id == id)
}

// SAFETY: the pointers are to fields of `self`, which the plugin keeps in an `Arc` the wrapper shares.
unsafe impl Params for RindParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        SPECS
            .iter()
            .zip(&self.slots)
            .map(|(spec, s)| (spec.id.to_owned(), s.ptr(), String::new()))
            .collect()
    }
}

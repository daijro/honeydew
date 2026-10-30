//! The automation editor's values in the target's units, a Honeydew addition (GitHub issue #21): the value text of
//! the lane's target ([`Target`]), typed values ([`Target::parse`]), and the value grid with its snap ([`ValueSnap`],
//! [`Land`]) and its snap-menu column ([`snap_items`]), which the piano roll's graph lane shares (#32).
//!
//! Values stay the stored u16 (0…65534 for new points); a point plays (and reads) value/65535. The value grid's lines
//! sit at k·g of the range, g = its size (a percent, or a discrete target's step 1/(n − 1)).

use serde::{Deserialize, Serialize};

use crate::params::{CHANNEL_PARAMS, ParamKind, ParamSpec, TextCx, menu_value};
use crate::song::points::{self, Point, unit};
use crate::song::{AutomationTarget, Song, automated_tempo};
use crate::ui::piano_roll::SnapMode;
use crate::ui::popup::PopupItem;
use crate::ui::tempo::transport_bpm_text;

/// The value grid's size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueGrid {
    /// The editor's rows, 1/126 of the range: whole-row drags, as before the grid.
    #[default]
    Rows,
    /// This percent of the range.
    Percent(u8),
    /// A discrete target's own steps (`Rows` on a continuous one).
    Steps,
}

impl ValueGrid {
    /// The sizes in the snap menu's order.
    pub const ALL: [Self; 6] = [
        Self::Rows,
        Self::Percent(1),
        Self::Percent(5),
        Self::Percent(10),
        Self::Percent(25),
        Self::Steps,
    ];

    pub fn label(self) -> String {
        match self {
            Self::Rows => "Rows".into(),
            Self::Percent(p) => format!("{p} %"),
            Self::Steps => "Steps".into(),
        }
    }
}

/// The value snap, an app setting: on, and its grid. On with `Rows` by default, the editor's law before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueSnap {
    pub on: bool,
    pub grid: ValueGrid,
}

impl Default for ValueSnap {
    fn default() -> Self {
        Self {
            on: true,
            grid: ValueGrid::Rows,
        }
    }
}

impl ValueSnap {
    /// Off, the grid at `Rows`: the piano roll's default (its lane has no rows, so it lists no `Rows`).
    pub const OFF: Self = Self {
        on: false,
        grid: ValueGrid::Rows,
    };

    /// How vertical moves land on a target of `steps` discrete steps (`None`: continuous), values up to `max`.
    pub fn land(self, steps: Option<usize>, max: u16) -> Land {
        let frac = match self.grid {
            ValueGrid::Percent(p) if p > 0 => Some(f64::from(p.min(100)) / 100.0),
            ValueGrid::Steps => steps.filter(|&n| n > 1).map(|n| 1.0 / (n - 1) as f64),
            _ => None,
        };
        Land {
            on: self.on,
            step: frac.map(|f| f * f64::from(max)),
        }
    }

    /// The value snap after a pick in the snap menu's value column: on at `grid`, or off (`None`) keeping the grid.
    pub fn picked(self, grid: Option<ValueGrid>) -> Self {
        grid.map_or(Self { on: false, ..self }, |grid| Self { on: true, grid })
    }
}

/// A pick in a snap menu with a value column: a time mode, or a value grid (`None`: value snap off).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    Time(SnapMode),
    Value(Option<ValueGrid>),
}

/// A snap menu's items: the time modes under `Time`, `mode` highlighted; beside them the value snap `vs` under `head`:
/// `Off`, then `grids` with their labels, the current one highlighted.
pub fn snap_items(
    mode: SnapMode,
    (vs, head): (ValueSnap, &'static str),
    grids: impl IntoIterator<Item = (ValueGrid, String)>,
) -> Vec<PopupItem<Pick>> {
    let time = (SnapMode::ALL.iter().enumerate()).map(|(row, &m)| PopupItem {
        current: m == mode,
        ..PopupItem::new(Pick::Time(m), 0, row + 1, m.label())
    });
    let off = [(None, "Off".to_owned())];
    let values = off
        .into_iter()
        .chain(grids.into_iter().map(|(g, l)| (Some(g), l)));
    let values = values.enumerate().map(|(row, (g, label))| PopupItem {
        current: g == vs.on.then_some(vs.grid),
        ..PopupItem::new(Pick::Value(g), 1, row + 1, label)
    });
    let heads = [
        PopupItem::header(0, 0, "Time"),
        PopupItem::header(1, 0, head),
    ];
    heads.into_iter().chain(time).chain(values).collect()
}

/// How the selected points' values move this frame: snap on or off, and the grid step in stored units (`None`: rows).
///
/// - Rows, snap on: whole rows ([`points::moved`]); created and pasted values as they are.
/// - Snap off: continuous, to the stored unit.
/// - A grid, snap on: the selection's top value lands on the nearest line and the others keep their offsets, as the
///   box's left beat snaps in time; created and pasted points land the same way.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Land {
    pub on: bool,
    pub step: Option<f64>,
}

impl Land {
    /// The grid step while a grid's snap is on.
    pub fn grid(self) -> Option<f64> {
        self.step.filter(|_| self.on)
    }

    /// `v` (stored units) on the nearest line while a grid's snap is on, else as it is.
    pub fn snap(self, v: f64) -> f64 {
        self.grid().map_or(v, |g| (v / g).round() * g)
    }

    /// The stored value of each line from 0 up to `max` while a grid's snap is on; none else.
    pub fn lines(self, max: u16) -> Vec<u16> {
        let Some(g) = self.grid() else {
            return Vec::new();
        };
        let n = (f64::from(max) / g + 1e-9).floor() as u32;
        (0..=n).map(|k| (f64::from(k) * g).round() as u16).collect()
    }
}

/// The selected points' highest and lowest values.
fn bounds(pts: &[Point], sel: &[bool]) -> Option<(f64, f64)> {
    let mut chosen = (pts.iter().zip(sel)).filter(|(_, s)| **s);
    let first = f64::from(chosen.next()?.0.value);
    Some(chosen.fold((first, first), |(hi, lo), (p, _)| {
        let v = f64::from(p.value);
        (hi.max(v), lo.min(v))
    }))
}

/// Moves the selected points' values together so that their top goes to `to` (rounded), the selection kept within
/// 0…`max` (an imported 65535 above it may only move down).
fn shift_to(pts: &mut [Point], sel: &[bool], to: f64, max: u16) {
    let Some((top, bottom)) = bounds(pts, sel) else {
        return;
    };
    let shift = (to.round() - top).clamp(-bottom, (f64::from(max) - top).max(0.0));
    for (p, _) in pts.iter_mut().zip(sel).filter(|(_, s)| **s) {
        p.value = (f64::from(p.value) + shift) as u16;
    }
}

/// The selected points' values moved by `dv` (stored units, up) as `land` says ([`Land`]): the rows' law is the
/// caller's ([`points::moved`]), so here a grid's snap or the continuous move. `dv` = 0 lands created and pasted points.
pub fn landed(pts: &mut [Point], sel: &[bool], dv: f64, land: Land, max: u16) {
    let Some((top, _)) = bounds(pts, sel) else {
        return;
    };
    shift_to(pts, sel, land.snap(top + dv), max);
}

/// Up (`lines` > 0) or Down: the selection moved by `lines` grid steps, as one edit. Rows move whole rows; a grid with
/// snap on takes the selection's top to the `lines`-th line past it, off by `lines` steps. `None` when nothing moved.
pub fn nudge(
    pts: &[Point],
    sel: &[bool],
    lines: i32,
    land: Land,
    max: u16,
) -> Option<(Vec<Point>, Vec<bool>)> {
    let Some(g) = land.step else {
        return points::moved(pts, sel, 0.0, -f64::from(lines), None, max);
    };
    let (top, _) = bounds(pts, sel)?;
    // Within half a unit of a line (a step's stored value rounds) counts as on it.
    let to = match (land.on, lines > 0) {
        (true, true) => (((top + 0.5) / g).floor() + f64::from(lines)) * g,
        (true, false) => (((top - 0.5) / g).ceil() + f64::from(lines)) * g,
        (false, _) => top + f64::from(lines) * g,
    };
    let mut new = pts.to_vec();
    shift_to(&mut new, sel, to, max);
    (new != pts).then(|| (new, sel.to_vec()))
}

/// What the editor knows of its lane's target: the registry's spec and what its text reads (as the panels pass it).
#[derive(Clone, Debug, Default)]
pub struct Target {
    spec: Option<ParamSpec>,
    /// The master's tempo lane.
    tempo: bool,
    params: Vec<f32>,
    tempo_bpm: f64,
    sample_rate: f32,
}

impl Target {
    /// Lane `target` of channel `channel`: the registry's spec ([`crate::app::auto_record::spec`]; a send's Level is
    /// the channel Level, its Enable a switch), or the tempo; `sample_rate` is the audio's (0 without).
    pub fn of(
        song: &Song,
        channel: usize,
        target: Option<AutomationTarget>,
        sample_rate: f32,
    ) -> Self {
        let ch = song.channels.get(channel);
        let spec = (ch.zip(target)).and_then(|(ch, t)| match t.param {
            0x1000..=0x1FFF if t.instance_id == 0 => Some(ParamSpec::toggle("", 1.0)),
            0x2000..=0x2FFF if t.instance_id == 0 => Some(CHANNEL_PARAMS[0]),
            _ => crate::app::auto_record::spec(ch, t),
        });
        let module = (ch.zip(target)).and_then(|(ch, t)| ch.rack.get(t.instance_id));
        Self {
            spec,
            tempo: target.is_some_and(|t| song.is_tempo(channel, t)),
            params: module.map(|m| m.params.clone()).unwrap_or_default(),
            tempo_bpm: song.tempo_bpm,
            sample_rate,
        }
    }

    /// The text of normalised `v`: the lane's value text ([`ParamSpec::automation_text_in`]), `120 BPM` on the tempo
    /// lane, else percent of the range without a unit (the axis's labels before units).
    pub fn text(&self, v: f32) -> String {
        if self.tempo {
            return format!("{} BPM", transport_bpm_text(automated_tempo(v)));
        }
        let cx = TextCx {
            params: &self.params,
            tempo_bpm: self.tempo_bpm,
            sample_rate: self.sample_rate,
        };
        match self.spec {
            Some(spec) => spec.automation_text_in(v, cx),
            None => format!("{}", (v * 100.0).round() as i32),
        }
    }

    /// [`Self::text`] as the value axis labels it: a continuous pan's short form (`100R`, `C`, `50L` for
    /// `100% right`, `center`, `50 % Left`), which fits the axis.
    pub fn axis_text(&self, v: f32) -> String {
        let text = self.text(v);
        let kind = self.spec.map(|s| s.automation.unwrap_or(s.kind));
        if !matches!(kind, Some(ParamKind::Continuous { .. })) {
            return text;
        }
        let lower = text.to_lowercase();
        if lower.starts_with("center") {
            return "C".into();
        }
        let side = match () {
            _ if lower.ends_with("left") => 'L',
            _ if lower.ends_with("right") => 'R',
            _ => return text,
        };
        let digits: String = (text.chars())
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        match digits.is_empty() {
            true => text,
            false => format!("{digits}{side}"),
        }
    }

    /// The text of stored value `value`, which plays value/65535.
    pub fn value_text(&self, value: u16) -> String {
        self.text(unit(value))
    }

    /// A discrete target's step count: a switch's 2, a menu's items, a stepped number's count.
    pub fn steps(&self) -> Option<usize> {
        if self.tempo {
            return None;
        }
        match self.spec?.kind {
            ParamKind::Toggle => Some(2),
            ParamKind::Menu { items } | ParamKind::MenuBy { items, .. } => Some(items.len()),
            ParamKind::Continuous { display } => display.step_count(),
        }
    }

    /// The stored value (≤ `max`) typed `typed` names, `None` when it names none. The candidates are a discrete
    /// target's steps, else every stored value. A candidate whose text is the typed one (any case) wins, else the one
    /// whose text's number is nearest the typed number ([`number`]), so out-of-range input clamps; of a run of equal
    /// candidates, the middle one (or its end at an end of the range).
    pub fn parse(&self, typed: &str, max: u16) -> Option<u16> {
        let typed = typed.trim();
        let candidates: Vec<u16> = match self.steps() {
            Some(n) => (0..n)
                .map(|k| points::raw(menu_value(k, n)).min(max))
                .collect(),
            None => (0..=max).collect(),
        };
        let want = number(typed);
        let distance = |v: u16| {
            let text = self.value_text(v);
            if text.eq_ignore_ascii_case(typed) {
                return 0.0;
            }
            match (want, number(&text)) {
                (Some(a), Some(b)) if a == b => 0.0,
                (Some(a), Some(b)) => (a - b).abs(),
                _ => f64::INFINITY,
            }
        };
        let d: Vec<f64> = candidates.iter().map(|&v| distance(v)).collect();
        let best = d.iter().copied().fold(f64::INFINITY, f64::min);
        if best.is_nan() || best == f64::INFINITY {
            return None;
        }
        let first = d.iter().position(|&x| x == best)?;
        let last = first + d[first..].iter().take_while(|&&x| x == best).count() - 1;
        let i = match (first, last) {
            (0, _) => 0,
            (_, l) if l == d.len() - 1 => l,
            (f, l) => (f + l) / 2,
        };
        Some(candidates[i])
    }
}

/// The number a value text reads, in base units: the first number (or `inf`) with its sign, ×1000 before a `k`
/// (`1.2 kHz`), ×0.001 before `ms`, negative on the left side (`50% left`, `50 L`). `None` without one.
fn number(text: &str) -> Option<f64> {
    let s = text.trim().to_lowercase();
    let digits = s.find(|c: char| c.is_ascii_digit() || c == '.');
    let inf = s.find("inf");
    let start = digits.into_iter().chain(inf).min()?;
    let end = if Some(start) == inf {
        start + 3
    } else {
        let len = s[start..].find(|c: char| !(c.is_ascii_digit() || c == '.'));
        start + len.unwrap_or(s.len() - start)
    };
    let mut x: f64 = s[start..end].parse().ok()?;
    let rest = s[end..].trim_start();
    if rest.starts_with("ms") {
        x *= 1e-3;
    } else if rest.starts_with('k') {
        x *= 1e3;
    }
    let minus = s[..start].trim_end().ends_with('-');
    let left =
        (rest.split(|c: char| c.is_whitespace() || c == '%')).any(|w| w == "l" || w == "left");
    Some(if minus != left { -x } else { x })
}

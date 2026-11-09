//! The side browser's previews: the audition strip over the picked row of an instrument's presets, and the sample strip
//! at the bottom of a sample picker. [`crate::ui::modules_browser::SidePanel`] draws them; the caller turns them on,
//! and sends what they ask for to the audio.
//!
//! **Audition strip.** Invisible, over the picked row's full rect, catching a touch before the list does: a finger down
//! plays note trunc(24 + 60u) (u: where across the row, clamped to [0, 1]) at velocity 1 until it lifts, one note at a
//! time; while one finger holds, another touch falls through and re-picks the row. Enter (action `open`, `Preview`)
//! holds note 60 while the strip has the keyboard focus (it takes it after each pick); the focus leaving it lets the
//! note go. While the module loads it draws the `LOADING...` pulse over the row. Over a drum kit's pick it plays the
//! kit's pads instead, across the row in pad order (#88).
//!
//! **Keys** (#98, a Honeydew addition). The app's keyboard ([`keyboard::keyboard_at`]),
//! [`KEYS_H`] tall across the bottom of an instrument's picker, its 2 U white keys round middle C, playing the picker's
//! channel as the audition strip does: velocity 1, every key a finger holds held, a tap for the keyboard's tap gate.
//!
//! **Sample strip.** 1.75 U at the bottom of the panel, `menuFill`: the play button (icon 3, 1.2 U, centred in a 2 U
//! column), the waveform (from x + 2 U, max(W − 2.4 U, 0) wide, 1 U tall, `text` at 0.85), and under it the sample's
//! name and its progress in a 5 : 1 width. It plays the pick once from the start; the play button (or the play key)
//! stops or plays from the start, a touch on the waveform stops and its release plays from there. The waveform sits
//! 0.125 U below the strip's top, the labels fill the rest; the progress is the position as m:ss (0:00 while stopped).
//! The waveform is one smooth band of the file's peaks, scaled to its loudest ([`envelope`], #108):
//! the audio editor's min/max columns showed a low note's single cycles, hairy and thin.

use eframe::egui::{Align2, Color32, Id, Mesh, Painter, Rect, Sense, Ui, pos2, vec2};

use crate::icons::{IconId, draw_icon};
use crate::metrics::FontSize;
use crate::sample_cache::Waveform;
use crate::settings::Settings;
use crate::ui::audio_editor::{add_column, pixel_columns, wave_rows};
use crate::ui::keyboard::{self, KeyInput, KeyView, Piano};
use crate::ui::widgets::truncated;
use crate::ui::{Finger, UiCtx, fingers};
use honeydew_audio::sample_edit::peaks;

/// The lowest and highest notes the strip plays, across the row (24 to 84).
const LOW: f32 = 24.0;
const SPAN: f32 = 60.0;
/// Enter's note (`open`, `Preview`).
pub const KEY_NOTE: u8 = 60;
/// The touch id Enter holds its note with: no finger's.
const KEY: u64 = u64::MAX - 1;
/// The loading fill's inset, in U, and its peak `text` alpha.
const LOADING_INSET: f32 = 0.25;
const LOADING_ALPHA: f32 = 0.5;
pub const LOADING: &str = "LOADING...";

/// The note a touch at `x` plays over `row`: trunc(24 + 60u) ([`across`]).
pub fn note_at(x: f32, row: Rect) -> u8 {
    (LOW + SPAN * across(x, row)) as u8
}

/// Where `x` is across `row`: u = clamp((x − left)/width, 0, 1).
fn across(x: f32, row: Rect) -> f32 {
    let u = ((x - row.left()) / row.width()).clamp(0.0, 1.0);
    if u.is_nan() { 0.0 } else { u }
}

/// The loading fill's `text` alpha at `t` seconds: 0.5·|sin(2π·frac(t))|, two pulses a second.
pub fn pulse(t: f64) -> f32 {
    LOADING_ALPHA * (std::f64::consts::TAU * t.fract()).sin().abs() as f32
}

/// A note the audition strip turns on or off, on the picked module's channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    /// Velocity 1, pan centre.
    On(u8),
    Off(u8),
}

/// The audition strip's state.
#[derive(Default)]
pub(crate) struct Audition {
    /// It catches touches on the picked row.
    pub on: bool,
    /// The picked module is loading: the pulse shows.
    pub loading: bool,
    /// The touch holding the note (a finger's id, or [`KEY`]), and the note.
    held: Option<(u64, u8)>,
    /// `LOADING...`'s alpha, a ← 0.95a + 0.05 a frame while loading, else 0.
    text_alpha: f32,
    sounds: Vec<Sound>,
    /// A drum kit's pad notes, in pad order (#88): a touch plays pad trunc(n·u) of the n, not [`note_at`]; Enter the
    /// first. Empty for an instrument.
    pub pads: Vec<u8>,
}

impl Audition {
    /// Turns it off (a list refresh); a held note is released.
    pub fn off(&mut self) {
        self.release();
        self.on = false;
    }

    /// Releases the held note, if any.
    pub fn release(&mut self) {
        if let Some((_, key)) = self.held.take() {
            self.sounds.push(Sound::Off(key));
        }
    }

    /// The notes to send, in order, since the last call.
    pub fn take_sounds(&mut self) -> Vec<Sound> {
        std::mem::take(&mut self.sounds)
    }

    /// A new trigger: the note held before is released first.
    fn trigger(&mut self, by: u64, key: u8) {
        self.release();
        self.held = Some((by, key));
        self.sounds.push(Sound::On(key));
    }

    /// This frame's finger steps over the strip on the picked row `row` (`None`: no picked row shows, so nothing goes
    /// down). Returns whether a second finger went down while one held a note: it re-picks the row.
    pub fn touch(&mut self, steps: &[Finger], row: Option<Rect>) -> bool {
        let mut repick = false;
        for &step in steps {
            match step {
                Finger::Down(..) if self.held.is_some_and(|(by, _)| by != KEY) => repick = true,
                Finger::Down(id, pos) => {
                    if let Some(row) = row {
                        let key = match self.pads.len() {
                            0 => note_at(pos.x, row),
                            n => self.pads[((n as f32 * across(pos.x, row)) as usize).min(n - 1)],
                        };
                        self.trigger(id, key);
                    }
                }
                Finger::Up(id, _) if self.held.is_some_and(|(by, _)| by == id) => self.release(),
                _ => {}
            }
        }
        repick
    }

    /// Enter (`open`): `pressed` this frame (a repeat is not), and still `down`.
    pub fn key(&mut self, pressed: bool, down: bool) {
        let holding = self.held.is_some_and(|(by, _)| by == KEY);
        if pressed && !holding {
            self.trigger(KEY, self.pads.first().copied().unwrap_or(KEY_NOTE));
        } else if holding && !down {
            self.release();
        }
    }

    /// Draws the loading pulse over `row` (its full rect, clipped by the painter) while loading, at `t` seconds; steps
    /// the text's fade-in a frame.
    pub fn paint(&mut self, painter: &Painter, cx: &UiCtx, row: Rect, t: f64) {
        if !self.loading {
            self.text_alpha = 0.0;
            return;
        }
        self.text_alpha = 0.95 * self.text_alpha + 0.05;
        let (c, m) = (&cx.theme.colors, cx.m);
        let fill = row.shrink(m.u(LOADING_INSET));
        painter.rect_filled(fill, 0.0, c.text.to_color32().gamma_multiply(pulse(t)));
        let ink = c.menu_fill.to_color32().gamma_multiply(self.text_alpha);
        let font = m.font_id(FontSize::Pt10);
        painter.text(fill.center(), Align2::CENTER_CENTER, LOADING, font, ink);
        painter.ctx().request_repaint();
    }
}

/// The keys' height, in U: under a phone's landscape header the list keeps most of its rows.
pub const KEYS_H: f32 = 3.0;

/// The keys at the bottom of an instrument's picker (see the module docs).
pub struct KeyStrip {
    /// The channel's colour, which a pressed key and a root's marker take.
    pub ink: Color32,
    /// What the keys' labels follow (`Show key names`, `Set middle C to C4`).
    pub settings: Settings,
    /// The keys sounding, in key order.
    on: Vec<u8>,
    /// The keys last tapped, sounding until the gate ends (egui seconds).
    taps: (Vec<u8>, f64),
    sounds: Vec<Sound>,
}

impl KeyStrip {
    pub fn new(ink: Color32, settings: Settings) -> Self {
        Self {
            ink,
            settings,
            on: Vec::new(),
            taps: (Vec::new(), 0.0),
            sounds: Vec::new(),
        }
    }

    /// Every key sounding goes off (take the notes after).
    pub fn release(&mut self) {
        self.sounds.extend(self.on.drain(..).map(Sound::Off));
        self.taps.0.clear();
    }

    /// The notes to send, in order, since the last call.
    pub fn take_sounds(&mut self) -> Vec<Sound> {
        std::mem::take(&mut self.sounds)
    }

    /// Draws the keys at the bottom of `panel` and plays them. `id`: the panel's.
    pub fn show(&mut self, ui: &mut Ui, cx: &UiCtx, panel: Rect, id: Id) {
        let m = cx.m;
        let rect = panel.with_min_y(panel.bottom() - m.u(KEYS_H));
        // Middle C's white key in the middle, as the keyboard panel starts.
        let white_w = KeyView::default().white_w;
        let left = keyboard::slots(0..=59, None) + 0.5 - rect.width() / (2.0 * m.u(white_w));
        let piano = Piano {
            rect,
            two_rows: false,
            view: KeyView { left, white_w },
        };
        let input = KeyInput {
            enabled: true,
            ..KeyInput::default()
        };
        let (ink, settings) = (self.ink, &self.settings);
        let played = keyboard::keyboard_at(ui, id.with("keys"), piano, cx, ink, settings, input);
        let now = ui.input(|i| i.time);
        if !played.tapped.is_empty() {
            let gate = now + crate::app::TAP_GATE;
            self.taps = (played.tapped.iter().map(|t| t.0).collect(), gate);
        }
        if now >= self.taps.1 {
            self.taps.0.clear();
        }
        let mut keys: Vec<u8> = (played.held.iter().map(|h| h.0))
            .chain(self.taps.0.iter().copied())
            .collect();
        keys.sort_unstable();
        keys.dedup();
        let off = self
            .on
            .iter()
            .filter(|k| !keys.contains(k))
            .map(|&k| Sound::Off(k));
        let on = keys
            .iter()
            .filter(|k| !self.on.contains(k))
            .map(|&k| Sound::On(k));
        self.sounds.extend(off.chain(on));
        self.on = keys;
        if !self.taps.0.is_empty() {
            ui.ctx()
                .request_repaint_after_secs((self.taps.1 - now) as f32);
        }
    }
}

// Sample strip sizes, in U.
pub const STRIP_H: f32 = 1.75;
const COLUMN_W: f32 = 2.0;
const PLAY: f32 = 1.2;
const WAVE_X: f32 = 2.0;
const WAVE_MARGIN: f32 = 2.4;
const WAVE_H: f32 = 1.0;
const WAVE_ALPHA: f32 = 0.85;
/// How long the waveform holds a peak, in seconds: the cycles of a low note fill in ([`envelope`]).
const HOLD_S: f64 = 0.02;
/// The waveform's top below the strip's, and the name's share of the label row (5 : 1).
const WAVE_TOP: f32 = 0.125;
const NAME_SHARE: f32 = 5.0 / 6.0;

/// What the sample strip asks of the preview player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Play {
    /// Play the sample once from this frame.
    From(usize),
    Stop,
}

/// The sample strip of a sample picker: the last pick, played on the preview player.
pub struct SampleStrip {
    pub name: String,
    /// The decoded sample; `None` while it loads or when it does not decode: no play button, waveform or progress.
    pub wave: Option<Waveform>,
    /// While it plays: when it started (egui seconds; `None` until its first frame) and from which frame.
    playing: Option<(Option<f64>, usize)>,
    /// A touch on the waveform: stopped until its release.
    touching: Option<u64>,
    plays: Vec<Play>,
}

impl SampleStrip {
    /// A pick's strip, which plays it from the start once it is decoded.
    pub fn new(name: String, wave: Option<Waveform>) -> Self {
        let mut strip = Self {
            name,
            wave: None,
            playing: None,
            touching: None,
            plays: Vec::new(),
        };
        if wave.is_some() {
            strip.wave = wave;
            strip.play(0);
        }
        strip
    }

    fn play(&mut self, from: usize) {
        self.playing = Some((None, from));
        self.plays.push(Play::From(from));
    }

    /// Stops it, if it plays (a refresh, a new pick, the picker closing).
    pub fn stop(&mut self) {
        if self.playing.take().is_some() {
            self.plays.push(Play::Stop);
        }
    }

    pub fn playing(&self) -> bool {
        self.playing.is_some()
    }

    /// What to send to the preview player, in order, since the last call.
    pub fn take_plays(&mut self) -> Vec<Play> {
        std::mem::take(&mut self.plays)
    }

    /// The frame it is at, at `now` seconds; `None` while stopped.
    fn position(&self, now: f64) -> Option<f64> {
        let (since, from) = self.playing?;
        let rate = self.wave.as_ref()?.data.sample_rate;
        Some(from as f64 + (now - since.unwrap_or(now)).max(0.0) * f64::from(rate))
    }

    /// Where the strip and its parts go on `panel` (a panel `w` wide): the strip, the play button, the waveform, the
    /// name and the progress.
    pub fn layout(panel: Rect, cx: &UiCtx) -> [Rect; 5] {
        let m = cx.m;
        let strip = panel.with_min_y(panel.bottom() - m.u(STRIP_H));
        let play = Rect::from_center_size(
            pos2(strip.left() + m.u(COLUMN_W / 2.0), strip.center().y),
            vec2(m.u(PLAY), m.u(PLAY)),
        );
        let wave_w = (strip.width() - m.u(WAVE_MARGIN)).max(0.0);
        let wave = Rect::from_min_size(
            pos2(strip.left() + m.u(WAVE_X), strip.top() + m.u(WAVE_TOP)),
            vec2(wave_w, m.u(WAVE_H)),
        );
        let labels = Rect::from_x_y_ranges(wave.x_range(), wave.bottom()..=strip.bottom());
        let split = labels.left() + labels.width() * NAME_SHARE;
        let name = labels.with_max_x(split);
        let progress = labels.with_min_x(split);
        [strip, play, wave, name, progress]
    }

    /// Draws the strip at the bottom of `panel` and takes its touches. `id`: the panel's.
    pub fn show(&mut self, ui: &mut Ui, cx: &UiCtx, panel: Rect, id: Id) {
        let (c, m) = (&cx.theme.colors, cx.m);
        let now = ui.input(|i| i.time);
        let [strip, play, wave_rect, name, progress] = Self::layout(panel, cx);
        let text = c.text.to_color32();
        let painter = ui.painter().clone();
        // The whole strip, so it swallows the touches that hit no control.
        ui.interact(strip, id.with("sample strip"), Sense::click_and_drag());
        painter.rect_filled(strip, 0.0, c.menu_fill.to_color32());
        let font = m.font_id(FontSize::Pt8);
        truncated(
            &painter,
            name,
            Align2::LEFT_CENTER,
            &self.name,
            font.clone(),
            text,
            5,
        );
        let Some(wave) = self.wave.clone() else {
            return;
        };
        if let Some((since @ None, _)) = &mut self.playing {
            *since = Some(now);
        }
        let frames = wave.data.frames();
        if self.position(now).is_some_and(|at| at >= frames as f64) {
            // It ended: the player stopped by itself.
            self.playing = None;
        }
        if self.playing.is_some() {
            ui.ctx().request_repaint();
        }

        let button = ui.interact(play, id.with("sample play"), Sense::click());
        draw_icon(&painter, play, IconId::Play, text, 1.0, m);
        // The play key plays or stops it too (`Play preview` / `Pause preview`), unless a text field has the keys.
        let key = !ui.ctx().wants_keyboard_input() && crate::keymap::pressed(ui.ctx(), "play");
        if button.clicked() || key {
            match self.playing.is_some() {
                true => self.stop(),
                false => self.play(0),
            }
        }

        let wave_id = id.with("sample wave");
        ui.interact(wave_rect, wave_id, Sense::click_and_drag());
        for step in fingers(ui, wave_id) {
            match step {
                Finger::Down(finger, _) if self.touching.is_none() => {
                    self.touching = Some(finger);
                    self.stop();
                }
                Finger::Up(finger, pos) if self.touching == Some(finger) => {
                    self.touching = None;
                    let u = ((pos.x - wave_rect.left()) / wave_rect.width()).clamp(0.0, 1.0);
                    let at = ((u * frames as f32) as usize).min(frames.saturating_sub(1));
                    self.play(at);
                }
                _ => {}
            }
        }

        let ink = text.gamma_multiply(WAVE_ALPHA);
        let px = 1.0 / m.ppp();
        let (x0, width) = pixel_columns(wave_rect, m);
        let (centre, scale) = wave_rows(wave_rect, false)[0];
        let mut mesh = Mesh::default();
        for (i, h) in envelope(&wave, width).into_iter().enumerate() {
            add_band_column(&mut mesh, x0 + i as f32 * px, h * scale, centre, px, ink);
        }
        painter.add(mesh);

        let seconds = self.position(now).unwrap_or(0.0) / f64::from(wave.data.sample_rate);
        let label = clock(seconds);
        truncated(
            &painter,
            progress,
            Align2::RIGHT_CENTER,
            &label,
            font,
            text,
            5,
        );
    }
}

/// The sample strip's waveform over `width` columns (#108): each column's peak |x| over both channels ([`peaks`]), held
/// [`HOLD_S`] after it, then averaged over as many columns ahead, as a share of the loudest; the strip draws ± that about
/// its middle, one band with smoothed edges ([`add_band_column`]; a column under 1 px a 1 px line).
pub fn envelope(wave: &Waveform, width: usize) -> Vec<f32> {
    let fpp = wave.data.frames() as f64 / width.max(1) as f64;
    let mut peak = vec![0f32; width];
    for (samples, summary) in wave.channels() {
        for (p, (lo, hi)) in peak
            .iter_mut()
            .zip(peaks(samples, summary, fpp, 0.0, width))
        {
            *p = p.max(lo.abs()).max(hi.abs());
        }
    }
    // ponytail: O(width · hold), the hold at most width/50 columns; a running max if strips grow much wider.
    let hold = (HOLD_S * f64::from(wave.data.sample_rate) / fpp).ceil() as usize;
    let hold = hold.min(width / 50);
    let held: Vec<f32> = (0..width)
        .map(|i| {
            peak[i.saturating_sub(hold)..=i]
                .iter()
                .fold(0f32, |a, &b| a.max(b))
        })
        .collect();
    // The held peaks averaged over the next `hold` columns (into `peak`): the hold's flat steps become ramps from one
    // crest to the next, and the two windows together look as far back as ahead.
    let mut sum = 0f32;
    for i in (0..width).rev() {
        sum += held[i];
        if let Some(gone) = held.get(i + hold + 1) {
            sum -= gone;
        }
        peak[i] = (sum / (width - i).min(hold + 1) as f32).max(0.0);
    }
    let top = peak.iter().fold(0f32, |a, &b| a.max(b));
    for p in &mut peak {
        *p = if top > 0.0 { *p / top } else { 0.0 };
    }
    peak
}

/// Adds the strip's band column at `x` to `mesh`: `px` wide, ± `half` about `centre`. The rows its edges cut through
/// get `ink` at the share they cover, so a gentle slope reads smooth, not as steps of whole device pixels; a band under
/// a pixel tall is the audio editor's 1 px line ([`add_column`]).
fn add_band_column(mesh: &mut Mesh, x: f32, half: f32, centre: f32, px: f32, ink: Color32) {
    let (top, bottom) = ((centre - half) / px, (centre + half) / px);
    // A row covered but for 2 % is whole: solid, not a cap the colour of `ink`.
    let (inner_top, inner_bottom) = ((top - 0.02).ceil(), (bottom + 0.02).floor());
    if inner_bottom <= inner_top {
        add_column(mesh, x, (-half, half), (centre, 1.0), px, ink);
        return;
    }
    let mut rows = |from: f32, to: f32, ink: Color32| {
        let r = Rect::from_x_y_ranges(x..=x + px, from * px..=to * px);
        mesh.add_colored_rect(r, ink);
    };
    rows(inner_top, inner_bottom, ink);
    // The cut rows; a nearly bare one stays bare.
    for (row, cover) in [
        (inner_top - 1.0, inner_top - top),
        (inner_bottom, bottom - inner_bottom),
    ] {
        if (0.02..0.98).contains(&cover) {
            rows(row, row + 1.0, ink.gamma_multiply(cover));
        }
    }
}

/// `seconds` as m:ss.
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests;

//! The side browser's previews: the audition strip over the picked row of an instrument's presets, and the sample strip
//! at the bottom of a sample picker. [`crate::ui::modules_browser::SidePanel`] draws them; the caller turns them on,
//! and sends what they ask for to the audio.
//!
//! **Audition strip.** Invisible, over the picked row's full rect, catching a touch before the list does: a finger down
//! plays note trunc(24 + 60u) (u: where across the row, clamped to [0, 1]) at velocity 1 until it lifts, one note at a
//! time; while one finger holds, another touch falls through and re-picks the row. Enter (action `open`, `Preview`)
//! holds note 60 while the strip has the keyboard focus (it takes it after each pick); the focus leaving it lets the
//! note go. While the module loads it draws the `LOADING...` pulse over the row.
//!
//! **Sample strip.** 1.75 U at the bottom of the panel, `menuFill`: the play button (icon 3, 1.2 U, centred in a 2 U
//! column), the waveform (from x + 2 U, max(W − 2.4 U, 0) wide, 1 U tall, `text` at 0.85), and under it the sample's
//! name and its progress in a 5 : 1 width. It plays the pick once from the start; the play button (or the play key)
//! stops or plays from the start, a touch on the waveform stops and its release plays from there. The waveform sits
//! 0.125 U below the strip's top, the labels fill the rest; the progress is the position as m:ss (0:00 while stopped).

use eframe::egui::{Align2, Id, Mesh, Painter, Rect, Sense, Ui, pos2, vec2};

use crate::icons::{IconId, draw_icon};
use crate::metrics::FontSize;
use crate::sample_cache::Waveform;
use crate::ui::audio_editor::{add_column, pixel_columns, wave_rows};
use crate::ui::widgets::truncated;
use crate::ui::{Finger, UiCtx, fingers};

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

/// The note a touch at `x` plays over `row`: trunc(24 + 60u), u = clamp((x − left)/width, 0, 1).
pub fn note_at(x: f32, row: Rect) -> u8 {
    let u = ((x - row.left()) / row.width()).clamp(0.0, 1.0);
    let u = if u.is_nan() { 0.0 } else { u };
    (LOW + SPAN * u) as u8
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
                        self.trigger(id, note_at(pos.x, row));
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
            self.trigger(KEY, KEY_NOTE);
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

// Sample strip sizes, in U.
pub const STRIP_H: f32 = 1.75;
const COLUMN_W: f32 = 2.0;
const PLAY: f32 = 1.2;
const WAVE_X: f32 = 2.0;
const WAVE_MARGIN: f32 = 2.4;
const WAVE_H: f32 = 1.0;
const WAVE_ALPHA: f32 = 0.85;
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
        let fpp = frames as f64 / width.max(1) as f64;
        let rows = wave_rows(wave_rect, wave.summary.len() > 1);
        let mut mesh = Mesh::default();
        for ((samples, summary), &row) in wave.channels().zip(&rows) {
            let peaks = honeydew_audio::sample_edit::peaks(samples, summary, fpp, 0.0, width);
            for (i, peak) in peaks.into_iter().enumerate() {
                add_column(&mut mesh, x0 + i as f32 * px, peak, row, px, ink);
            }
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

/// `seconds` as m:ss.
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests;

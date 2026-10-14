//! The MIDI import dialog, which the track menu's `Import MIDI Tracks` opens for a file of more than one MIDI channel.
//! A dialog box (no animation, the screen dimmed to α 0.6 under a `moduleFill` box from the first frame), centred in
//! the content area. Top to bottom:
//! - the title label `MIDI Import:`;
//! - the toggles `Split MIDI channels`, `Import automation` and `Only supported`, all on at first; the last two only
//!   when the file has CC or pitch-wheel data;
//! - 16 buttons `1`…`16`, one per MIDI channel: accent-coloured and enabled only for the channels the file uses, which
//!   start selected (filled); a tap toggles one;
//! - one row per used channel, `Channel N: ` (1-based) and its program names joined by `, `, in one column, or two once
//!   there are more than eight;
//! - `Import` and `Cancel`.
//!
//! The sizes below are in U. A selected channel is filled `accent` with a `background` label, an unselected one
//! `controlFill` with an `accent` label, an unused one `controlFill` and `text` at the disabled alpha; the toggles are
//! the Save dialog's render page toggles; `Import` and `Cancel` are its light `SAVE` buttons. Escape and a click
//! outside the box cancel, as the other dialog boxes close.

use eframe::egui::{Align2, Area, Context, Id, Order, Rect, Sense, pos2, vec2};

use crate::metrics::{FontSize, Metrics};
use crate::smf::import::{ImportOpts, Scan};
use crate::theme::DISABLED_ALPHA;
use crate::ui::UiCtx;
use crate::ui::dialog;
use crate::ui::save_dialog::toggle;
use crate::ui::widgets::{Button, button, truncated};

/// The box's width (or the content area's, when narrower) and its inner inset, in U.
const WIDTH: f32 = 24.0;
const PAD: f32 = 0.5;
/// Rows from the box's top: (top, height), in U.
const TITLE: (f32, f32) = (0.25, 1.0);
const TOGGLES: (f32, f32) = (1.5, 1.0);
const CHANNELS: (f32, f32) = (2.75, 1.0);
/// The channel rows start here, each this high; the buttons follow after a gap, and the box ends a gap below them.
const ROWS_TOP: f32 = 4.0;
const ROW_H: f32 = 0.625;
const GAP: f32 = 0.375;
const BUTTON: [f32; 2] = [3.0, 1.0];
const BUTTON_GAP: f32 = 0.5;
/// Between the channel buttons.
const CHANNEL_GAP: f32 = 0.25;
/// More rows than this go into two columns.
const ONE_COLUMN: usize = 8;

const TOGGLE_LABELS: [&str; 3] = ["Split MIDI channels", "Import automation", "Only supported"];
const CHANNEL_LABELS: [&str; 16] = [
    "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
];

/// The dialog's state while it is open: what the scan found and the choices so far.
#[derive(Clone, Debug, PartialEq)]
pub struct MidiImport {
    pub scan: Scan,
    /// `split`, `automation` and `only_supported` are the toggles; `channels` the selected MIDI channels.
    pub opts: ImportOpts,
}

/// What the user did this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiImportAction {
    /// `Import`, with the choices.
    Import(ImportOpts),
    /// `Cancel`, Escape or a click outside the box.
    Cancel,
}

/// Where everything goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub rect: Rect,
    pub title: Rect,
    /// `Split MIDI channels`, `Import automation`, `Only supported`.
    pub toggles: [Rect; 3],
    /// MIDI channels 1…16.
    pub channels: [Rect; 16],
    /// One per used channel, in order.
    pub rows: Vec<Rect>,
    pub import: Rect,
    pub cancel: Rect,
}

impl MidiImport {
    /// The dialog for a file that `scan` found: every toggle on, the used channels selected.
    pub fn new(scan: Scan) -> Self {
        let opts = ImportOpts {
            split: true,
            automation: true,
            only_supported: true,
            channels: scan.used,
        };
        Self { scan, opts }
    }

    /// The rows' texts: each used channel's `Channel N: ` (1-based) and its program names.
    pub fn rows(&self) -> Vec<String> {
        (0..16)
            .filter(|&c| self.scan.used >> c & 1 == 1)
            .map(|c| format!("Channel {}: {}", c + 1, self.scan.programs[c].join(", ")))
            .collect()
    }

    /// Shows the dialog over everything, its box centred in `area`; call every frame while it is open.
    pub fn show(&mut self, ctx: &Context, cx: &UiCtx, area: Rect) -> Option<MidiImportAction> {
        let (c, m) = (&cx.theme.colors, cx.m);
        let (text, bg, accent, control) = (
            c.text.to_color32(),
            c.background.to_color32(),
            c.accent.to_color32(),
            c.control_fill.to_color32(),
        );
        let rows = self.rows();
        let l = layout(area, m, rows.len());
        let screen = ctx.content_rect();
        let id = Id::new("midi_import");
        // A dialog box ([`dialog::dialog_box`]): shown from the first frame, over a modal backdrop that takes every touch
        // outside it and holds Ctrl+Z, as the other dialogs on the base do.
        let backdrop = dialog::dialog_box(ctx, id, c, screen, l.rect);
        let escape = crate::ui::close_pressed(ctx);
        let mut action = (backdrop.clicked() || backdrop.secondary_clicked() || escape)
            .then_some(MidiImportAction::Cancel);
        Area::new(id)
            .order(Order::Foreground)
            .fixed_pos(l.rect.min)
            .constrain(false)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.allocate_exact_size(l.rect.size(), Sense::hover());
                let painter = ui.painter().clone();
                let font = m.font_id(FontSize::Pt12);
                painter.text(
                    l.title.left_center(),
                    Align2::LEFT_CENTER,
                    "MIDI Import:",
                    font,
                    text,
                );

                let o = &mut self.opts;
                let shown = if self.scan.automation { 3 } else { 1 };
                let options = [&mut o.split, &mut o.automation, &mut o.only_supported];
                for ((rect, label), on) in l
                    .toggles
                    .into_iter()
                    .zip(TOGGLE_LABELS)
                    .zip(options)
                    .take(shown)
                {
                    toggle(ui, cx, id, rect, label, on);
                }

                for (ch, (rect, label)) in l.channels.into_iter().zip(CHANNEL_LABELS).enumerate() {
                    let used = self.scan.used >> ch & 1 == 1;
                    let b = if used {
                        Button {
                            fill_on: accent,
                            fill_off: control,
                            text_on: bg,
                            text_off: Some(accent),
                            ..Button::new(id.with("channel").with(ch), rect, label, false, c)
                        }
                    } else {
                        Button {
                            fill_off: control.gamma_multiply(DISABLED_ALPHA),
                            text_off: Some(text.gamma_multiply(DISABLED_ALPHA)),
                            disabled: true,
                            ..Button::new(id.with("channel").with(ch), rect, label, false, c)
                        }
                    };
                    let b = Button {
                        font: FontSize::Pt8,
                        ..b
                    };
                    let on = used && self.opts.channels >> ch & 1 == 1;
                    if button(ui, cx, &b, on).clicked() {
                        self.opts.channels ^= 1 << ch;
                    }
                }

                let font = m.font_id(FontSize::Pt8);
                for (rect, row) in l.rows.iter().zip(&rows) {
                    truncated(
                        &painter,
                        *rect,
                        Align2::LEFT_CENTER,
                        row,
                        font.clone(),
                        text,
                        0,
                    );
                }

                let import = Button::new(id.with("import"), l.import, "Import", false, c);
                if button(ui, cx, &import, true).clicked() {
                    action = Some(MidiImportAction::Import(self.opts));
                }
                let cancel = Button::new(id.with("cancel"), l.cancel, "Cancel", false, c);
                if button(ui, cx, &cancel, true).clicked() {
                    action = Some(MidiImportAction::Cancel);
                }
            });
        action
    }
}

/// Lays the dialog out in `area` for `rows` channel rows, in whole device pixels.
pub fn layout(area: Rect, m: &Metrics, rows: usize) -> Layout {
    let px = |u: f32| m.round_px(m.u(u));
    let per_column = if rows > ONE_COLUMN {
        rows.div_ceil(2)
    } else {
        rows
    };
    let buttons_top = ROWS_TOP + per_column as f32 * ROW_H + GAP;
    let size = vec2(
        px(WIDTH).min(area.width()),
        px(buttons_top + BUTTON[1] + GAP),
    );
    let min = pos2(
        m.round_px(area.center().x - size.x / 2.0),
        m.round_px(area.center().y - size.y / 2.0),
    );
    let rect = Rect::from_min_size(min, size);
    let (left, right) = (rect.left() + px(PAD), rect.right() - px(PAD));
    let inner = right - left;
    let line = |(top, h): (f32, f32)| {
        Rect::from_x_y_ranges(
            left..=right,
            rect.top() + px(top)..=rect.top() + px(top + h),
        )
    };
    // Equal parts of the inner width, `gap` apart, on `row`.
    let parts = |row: Rect, n: usize, gap: f32| -> Vec<Rect> {
        let w = (inner - (n - 1) as f32 * gap) / n as f32;
        (0..n)
            .map(|i| {
                let x = m.round_px(left + i as f32 * (w + gap));
                let x_end = m.round_px(left + i as f32 * (w + gap) + w);
                Rect::from_x_y_ranges(x..=x_end, row.y_range())
            })
            .collect()
    };
    let toggles = parts(line(TOGGLES), 3, 0.0);
    let channels = parts(line(CHANNELS), 16, px(CHANNEL_GAP));
    let rows = (0..rows)
        .map(|i| {
            let (column, row) = (i / per_column.max(1), i % per_column.max(1));
            let x = if column == 0 { left } else { rect.center().x };
            let top = rect.top() + px(ROWS_TOP + row as f32 * ROW_H);
            Rect::from_min_size(pos2(x, top), vec2(rect.center().x - left, px(ROW_H)))
        })
        .collect();
    let (w, gap) = (px(BUTTON[0]), px(BUTTON_GAP));
    let top = rect.top() + px(buttons_top);
    let import = Rect::from_min_size(
        pos2(m.round_px(rect.center().x - gap / 2.0 - w), top),
        vec2(w, px(BUTTON[1])),
    );
    Layout {
        rect,
        title: line(TITLE),
        toggles: [toggles[0], toggles[1], toggles[2]],
        channels: std::array::from_fn(|i| channels[i]),
        rows,
        import,
        cancel: import.translate(vec2(w + gap, 0.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::offscreen::{Offscreen, SCREEN, metrics};
    use eframe::egui::{Event, Ui};
    use eframe::egui::{Key, Modifiers};

    /// Channels 1 and 10 (index 0 and 9) and 3 (index 2): programs, drums, and the default program.
    fn scan(automation: bool) -> Scan {
        let mut scan = Scan {
            used: 1 | 1 << 2 | 1 << 9,
            automation,
            ..Scan::default()
        };
        scan.programs[0] = vec!["Close Grand".into(), "Program 5".into()];
        scan.programs[2] = vec!["Close Grand".into()];
        scan.programs[9] = vec!["Drums".into()];
        // An unused channel's program change lists a name, but makes no row.
        scan.programs[4] = vec!["Program 7".into()];
        scan
    }

    struct Rig {
        off: Offscreen,
        dialog: MidiImport,
        action: Option<MidiImportAction>,
    }

    impl Rig {
        fn new(automation: bool) -> Self {
            let mut rig = Self {
                off: Offscreen::new(),
                dialog: MidiImport::new(scan(automation)),
                action: None,
            };
            rig.frame(vec![]);
            rig
        }

        fn frame(&mut self, events: Vec<Event>) -> Vec<(String, eframe::egui::Color32)> {
            let (dialog, action) = (&mut self.dialog, &mut self.action);
            let shapes = self.off.frame(events, &mut |ui: &mut Ui, cx| {
                *action = dialog.show(ui.ctx(), cx, SCREEN).or(*action);
            });
            Offscreen::texts(&shapes)
        }

        fn click(&mut self, at: eframe::egui::Pos2) -> Option<MidiImportAction> {
            self.action = None;
            let (dialog, action) = (&mut self.dialog, &mut self.action);
            self.off.click(at, &mut |ui: &mut Ui, cx| {
                *action = dialog.show(ui.ctx(), cx, SCREEN).or(*action);
            });
            self.action
        }

        fn layout(&self) -> Layout {
            layout(SCREEN, &metrics(), self.dialog.rows().len())
        }
    }

    /// The dialog base: screen dim α 0.6, then a `moduleFill` box, from the first frame.
    #[test]
    fn the_dialog_base_dims_the_screen_then_fills_the_box_with_module_fill() {
        let (mut off, mut dialog) = (Offscreen::new(), MidiImport::new(scan(true)));
        let shapes = off.frame(vec![], &mut |ui: &mut Ui, cx| {
            dialog.show(ui.ctx(), cx, SCREEN);
        });
        let l = layout(SCREEN, &metrics(), dialog.rows().len());
        let painted = crate::ui::dialog::painted_rects(shapes.iter().map(|s| &s.shape));
        let colors = crate::theme::theme_by_id(0).colors;
        crate::ui::dialog::assert_base(&painted, &colors, SCREEN, l.rect);
        // Modal, like the others on the base: its frames are marked, which holds Ctrl+Z.
        assert!(crate::ui::modal_was_open(&off.ctx));
    }

    #[test]
    fn every_option_starts_on_and_the_used_channels_selected() {
        let dialog = MidiImport::new(scan(true));
        let want = ImportOpts {
            split: true,
            automation: true,
            only_supported: true,
            channels: 0b10_0000_0101,
        };
        assert_eq!(dialog.opts, want);
    }

    #[test]
    fn a_row_per_used_channel_names_it_1_based_with_its_programs() {
        assert_eq!(
            MidiImport::new(scan(true)).rows(),
            [
                "Channel 1: Close Grand, Program 5",
                "Channel 3: Close Grand",
                "Channel 10: Drums"
            ]
        );
    }

    #[test]
    fn the_automation_toggles_are_laid_out_only_with_cc_or_pitch_wheel_data() {
        let drawn = |automation| {
            let mut rig = Rig::new(automation);
            let texts: Vec<String> = rig.frame(vec![]).into_iter().map(|t| t.0).collect();
            TOGGLE_LABELS.map(|l| texts.iter().any(|t| t == l))
        };
        assert_eq!(drawn(true), [true; 3]);
        assert_eq!(drawn(false), [true, false, false]);
        // Without them, their places take no tap.
        let mut rig = Rig::new(false);
        let l = rig.layout();
        rig.click(l.toggles[1].center());
        rig.click(l.toggles[2].center());
        assert!(rig.dialog.opts.automation && rig.dialog.opts.only_supported);
    }

    #[test]
    fn it_draws_its_title_rows_channel_buttons_and_buttons() {
        let mut rig = Rig::new(true);
        let texts: Vec<String> = rig.frame(vec![]).into_iter().map(|t| t.0).collect();
        let mut want = vec!["MIDI Import:".to_owned()];
        want.extend(TOGGLE_LABELS.map(String::from));
        want.extend(CHANNEL_LABELS.map(String::from));
        want.extend(rig.dialog.rows());
        want.extend(["Import".to_owned(), "Cancel".to_owned()]);
        for t in &want {
            assert!(texts.contains(t), "{t} not drawn in {texts:?}");
        }
    }

    #[test]
    fn the_box_fits_its_rows_and_more_than_eight_take_two_columns() {
        let m = metrics();
        let u = m.u(1.0);
        let l = layout(SCREEN, &m, 3);
        assert_eq!(l.rect.width(), WIDTH * u);
        assert_eq!(l.rect.center(), SCREEN.center());
        let x: Vec<f32> = l.rows.iter().map(|r| r.left()).collect();
        assert_eq!(x, [l.rect.left() + PAD * u; 3]);
        let y: Vec<f32> = l.rows.iter().map(|r| r.top() - l.rect.top()).collect();
        assert_eq!(
            y,
            [
                ROWS_TOP * u,
                (ROWS_TOP + ROW_H) * u,
                (ROWS_TOP + 2.0 * ROW_H) * u
            ]
        );
        assert_eq!(
            l.import.top() - l.rect.top(),
            (ROWS_TOP + 3.0 * ROW_H + GAP) * u
        );
        assert_eq!(l.rect.bottom(), l.import.bottom() + GAP * u);
        // 9 rows: 5 in the left column, 4 in the right.
        let l = layout(SCREEN, &m, 9);
        let left = l
            .rows
            .iter()
            .filter(|r| r.left() == l.rect.left() + PAD * u);
        assert_eq!(left.count(), 5);
        assert_eq!(l.rows[5].top(), l.rows[0].top());
        assert_eq!(l.rows[5].left(), l.rect.center().x);
        assert_eq!(
            l.import.top() - l.rect.top(),
            (ROWS_TOP + 5.0 * ROW_H + GAP) * u
        );
        // The channel buttons span the inner width, 1 to 16 left to right.
        assert_eq!(l.channels[0].left(), l.rect.left() + PAD * u);
        assert_eq!(l.channels[15].right(), l.rect.right() - PAD * u);
        assert!(l.channels.windows(2).all(|w| w[0].right() < w[1].left()));
        // A narrow area: the box takes its width.
        let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(20.0 * u, 540.0));
        assert_eq!(layout(narrow, &m, 3).rect.width(), 20.0 * u);
    }

    #[test]
    fn a_tap_toggles_a_used_channel_and_an_unused_one_takes_none() {
        let mut rig = Rig::new(true);
        let l = rig.layout();
        rig.click(l.channels[2].center());
        assert_eq!(rig.dialog.opts.channels, 1 | 1 << 9);
        rig.click(l.channels[1].center());
        assert_eq!(rig.dialog.opts.channels, 1 | 1 << 9);
        rig.click(l.channels[2].center());
        assert_eq!(rig.dialog.opts.channels, 0b10_0000_0101);
        // Selected: accent-filled, its label the background's; unselected: the accent label; unused: dimmed.
        rig.click(l.channels[0].center());
        let texts = rig.frame(vec![Event::PointerGone]);
        let colors = crate::theme::theme_by_id(0).colors;
        let color = |label: &str| texts.iter().find(|t| t.0 == label).unwrap().1;
        assert_eq!(color("10"), colors.background.to_color32());
        assert_eq!(color("1"), colors.accent.to_color32());
        assert_eq!(
            color("2"),
            colors.text.to_color32().gamma_multiply(DISABLED_ALPHA)
        );
    }

    #[test]
    fn the_toggles_flip_their_options_and_import_returns_them() {
        let mut rig = Rig::new(true);
        let l = rig.layout();
        for t in l.toggles {
            assert_eq!(rig.click(t.center()), None);
        }
        let want = ImportOpts {
            split: false,
            automation: false,
            only_supported: false,
            channels: 0b10_0000_0101,
        };
        assert_eq!(rig.dialog.opts, want);
        assert_eq!(
            rig.click(l.import.center()),
            Some(MidiImportAction::Import(want))
        );
    }

    #[test]
    fn import_automation_and_only_supported_each_flip_their_own_option() {
        let mut rig = Rig::new(true);
        let l = rig.layout();
        rig.click(l.toggles[1].center());
        let o = rig.dialog.opts;
        assert_eq!(
            (o.split, o.automation, o.only_supported),
            (true, false, true)
        );
        rig.click(l.toggles[1].center());
        rig.click(l.toggles[2].center());
        let o = rig.dialog.opts;
        assert_eq!(
            (o.split, o.automation, o.only_supported),
            (true, true, false)
        );
    }

    #[test]
    fn cancel_escape_and_a_click_outside_cancel() {
        let mut rig = Rig::new(true);
        let l = rig.layout();
        assert_eq!(rig.click(l.cancel.center()), Some(MidiImportAction::Cancel));
        assert_eq!(
            rig.click(pos2(5.0, 5.0)),
            Some(MidiImportAction::Cancel),
            "outside"
        );
        rig.action = None;
        let escape = Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        rig.frame(vec![escape]);
        assert_eq!(rig.action, Some(MidiImportAction::Cancel));
        // A click inside the box, on nothing, does nothing.
        assert_eq!(rig.click(l.title.right_center() + vec2(-2.0, 0.0)), None);
    }
}

//! The Song options dialog (`New Song`).
//!
//! On the dialog base: the screen dimmed at α 0.6 and a `moduleFill` band across its middle, holding top to bottom: the
//! title `New Song`, the tempo menu (`80 BPM` … `200 BPM`), the mode list `Time signature:` / `Time division:`, the
//! first menu (`1` … `16`) `/` the second menu, and `Cancel` / `OK`. The menus are light buttons showing their value; a
//! tap opens the value's popup with the current item highlighted. In `Time signature:` the second menu is `1` … `16`
//! too; in `Time division:` it is `2`, `4`, `8`, `16`. The band is modal: the screen around it swallows every click,
//! and only `Cancel`, Escape and `OK` end it.
//!
//! What `OK` makes ([`SongOptions::apply`]): `Time signature:` writes the TDIV pair (first menu, second menu) as
//! (beats per bar, subdivision), the tempo as picked. `Time division:` writes (first menu, 4) and the tempo times
//! division / 4.

use eframe::egui::{Align2, Id, Pos2, Rect, Sense, Ui, pos2, vec2};

use crate::metrics::{FontSize, Metrics};
use crate::song::{Song, TimeSig};
use crate::ui::UiCtx;
use crate::ui::dialog;
use crate::ui::popup::{self, PopupItem};
use crate::ui::widgets::{Bar, Button, ItemList, button, item_list_index, text_label};

/// The dialog's title.
pub const TITLE: &str = "New Song";
/// The mode list's items, in [`Mode`] order.
pub const MODES: [&str; 2] = ["Time signature:", "Time division:"];
/// The division menu's items.
pub const DIVISIONS: [u16; 4] = [2, 4, 8, 16];
/// The widgets' ids derive from this.
pub const ID: &str = "song_options";

/// The tempo menu's items: 80 … 200 BPM in steps of 10.
pub fn tempos() -> impl Iterator<Item = u16> {
    (0..13).map(|i| 80 + 10 * i)
}

/// Which pair the menus write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// `Time signature:`: (first menu, second menu), the tempo as picked.
    #[default]
    Signature,
    /// `Time division:`: (first menu, 4), the tempo times division / 4.
    Division,
}

/// The dialog's choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SongOptions {
    /// The tempo menu, BPM.
    pub tempo: u16,
    pub mode: Mode,
    /// The first menu, 1 … 16: the beats per bar.
    pub beats: u8,
    /// The second menu in `Time signature:`, 1 … 16: the subdivision.
    pub steps: u8,
    /// The second menu in `Time division:`: one of [`DIVISIONS`].
    pub division: u16,
}

/// `140 BPM`, `4` / `4`; division 4 (×1).
impl Default for SongOptions {
    fn default() -> Self {
        Self {
            tempo: 140,
            mode: Mode::Signature,
            beats: 4,
            steps: 4,
            division: 4,
        }
    }
}

impl SongOptions {
    /// The TDIV pair the choices write.
    pub fn time_sig(self) -> TimeSig {
        TimeSig {
            subdivision: match self.mode {
                Mode::Signature => self.steps,
                Mode::Division => 4,
            },
            beats_per_bar: self.beats,
        }
    }

    /// The song tempo the choices give, BPM.
    pub fn tempo_bpm(self) -> f64 {
        let tempo = f64::from(self.tempo);
        match self.mode {
            Mode::Signature => tempo,
            Mode::Division => tempo * f64::from(self.division) / 4.0,
        }
    }

    /// Writes the choices into `song`: TDIV and the tempo.
    pub fn apply(self, song: &mut Song) {
        song.time_sig = self.time_sig();
        song.tempo_bpm = self.tempo_bpm();
    }
}

/// The dialog's menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Menu {
    Tempo,
    /// The first menu.
    Beats,
    /// The second menu: the subdivision or the division, by the mode.
    Second,
}

/// `menu`'s popup items over `o`, each holding its value.
fn items(menu: Menu, o: SongOptions) -> Vec<PopupItem<u16>> {
    // (value, column, row, label).
    let cells: Vec<(u16, usize, usize, String)> = match (menu, o.mode) {
        (Menu::Tempo, _) => (tempos().enumerate())
            .map(|(row, bpm)| (bpm, 0, row, format!("{bpm} BPM")))
            .collect(),
        // 1–8 in the left column, 9–16 in the right.
        (Menu::Beats, _) | (Menu::Second, Mode::Signature) => (1..=16)
            .map(|n: u16| {
                (
                    n,
                    usize::from(n - 1) / 8,
                    usize::from(n - 1) % 8,
                    n.to_string(),
                )
            })
            .collect(),
        (Menu::Second, Mode::Division) => (DIVISIONS.into_iter().enumerate())
            .map(|(row, d)| (d, 0, row, d.to_string()))
            .collect(),
    };
    let current = value(menu, o);
    (cells.into_iter())
        .map(|(v, col, row, label)| PopupItem {
            current: v == current,
            ..PopupItem::new(v, col, row, label)
        })
        .collect()
}

/// `menu`'s value in `o`.
fn value(menu: Menu, o: SongOptions) -> u16 {
    match (menu, o.mode) {
        (Menu::Tempo, _) => o.tempo,
        (Menu::Beats, _) => o.beats.into(),
        (Menu::Second, Mode::Signature) => o.steps.into(),
        (Menu::Second, Mode::Division) => o.division,
    }
}

// Sizes in U. The band is the fine-tune dialog box's 8 U.
const BAND_H: f32 = 8.0;
/// Rows from the band's top: (top, height).
const TITLE_ROW: (f32, f32) = (0.5, 1.0);
const TEMPO_ROW: (f32, f32) = (1.75, 1.25);
const MODE_ROW: (f32, f32) = (3.25, 1.0);
const MENU_ROW: (f32, f32) = (4.5, 1.25);
const BUTTON_ROW: (f32, f32) = (6.25, 1.25);
const TEMPO_W: f32 = 4.0;
/// Each mode cell holds `Time signature:` (about 4.4 U at 10 pt).
const MODE_W: f32 = 11.0;
const MENU_W: f32 = 2.0;
const SLASH_W: f32 = 1.0;
const BUTTON_W: f32 = 3.0;
const BUTTON_GAP: f32 = 0.5;
/// The mode list's other item: `text` dimmed, not the disabled look (as in the Save dialog). The accent read as the
/// selected one.
const DIM_ALPHA: f32 = 0.5;

/// Where the dialog's parts go, in whole device pixels.
struct Layout {
    band: Rect,
    title: Rect,
    tempo: Rect,
    modes: Rect,
    beats: Rect,
    slash: Rect,
    second: Rect,
    cancel: Rect,
    ok: Rect,
}

/// The band is the screen's width, centred on it.
fn layout(screen: Rect, m: &Metrics) -> Layout {
    let band_h = m.round_px(m.u(BAND_H));
    let top = m.round_px(screen.center().y - band_h / 2.0);
    let band = Rect::from_x_y_ranges(screen.x_range(), top..=top + band_h);
    // Rects `widths` wide side by side, centred, at `row`.
    let row = |(t, h): (f32, f32), widths: &[f32]| -> Vec<Rect> {
        let w: Vec<f32> = widths.iter().map(|&w| m.round_px(m.u(w))).collect();
        let mut x = m.round_px(screen.center().x - w.iter().sum::<f32>() / 2.0);
        let y = top + m.round_px(m.u(t));
        (w.into_iter())
            .map(|w| {
                x += w;
                Rect::from_min_size(pos2(x - w, y), vec2(w, m.round_px(m.u(h))))
            })
            .collect()
    };
    let menus = row(MENU_ROW, &[MENU_W, SLASH_W, MENU_W]);
    let buttons = row(BUTTON_ROW, &[BUTTON_W, BUTTON_GAP, BUTTON_W]);
    Layout {
        band,
        title: row(TITLE_ROW, &[MODE_W])[0],
        tempo: row(TEMPO_ROW, &[TEMPO_W])[0],
        modes: row(MODE_ROW, &[MODE_W])[0],
        beats: menus[0],
        slash: menus[1],
        second: menus[2],
        cancel: buttons[0],
        ok: buttons[2],
    }
}

/// What the user did this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SongOptionsAction {
    /// `OK`: make the new song with these.
    Ok(SongOptions),
    /// `Cancel` or Escape: no song.
    Cancel,
}

/// The dialog's state while it is open; it opens on the defaults.
#[derive(Debug, Default)]
pub struct SongOptionsDialog {
    pub options: SongOptions,
    /// The open menu and the point its popup is centred on.
    menu: Option<(Menu, Pos2)>,
}

impl SongOptionsDialog {
    /// Paints the dialog over `screen` (which it makes modal) and handles its input; call it last in the frame's
    /// `ui`, so that it is on top. Returns what ends it.
    pub fn show(&mut self, ui: &mut Ui, cx: &UiCtx, screen: Rect) -> Option<SongOptionsAction> {
        let (c, m) = (&cx.theme.colors, cx.m);
        let text = c.text.to_color32();
        let l = layout(screen, m);
        let id = Id::new(ID);
        // Modal: drawn last, the screen takes every click the band's widgets do not. The dialog base: the screen dimmed
        // at α 0.6, then a `moduleFill` box, from the first frame.
        ui.interact(screen, id.with("backdrop"), Sense::click_and_drag());
        dialog::paint_base(ui.painter(), ui.painter(), c, screen, l.band);
        text_label(
            ui,
            cx,
            l.title,
            TITLE,
            FontSize::Pt12,
            Align2::CENTER_CENTER,
            text,
        );
        let o = self.options;
        let mut menu = |ui: &mut Ui, menu: Menu, part: &str, rect: Rect, label: &str| {
            let b = Button::new(id.with(part), rect, label, false, c);
            if button(ui, cx, &b, true).clicked() {
                self.menu = Some((menu, rect.center()));
            }
        };
        menu(
            ui,
            Menu::Tempo,
            "tempo",
            l.tempo,
            &format!("{} BPM", o.tempo),
        );
        let mut mode = o.mode as usize;
        let list = ItemList::new(
            id.with("mode"),
            l.modes,
            &MODES,
            Bar::Bottom,
            text.gamma_multiply(DIM_ALPHA),
        );
        item_list_index(ui, cx, &list, &mut mode);
        menu(ui, Menu::Beats, "beats", l.beats, &o.beats.to_string());
        text_label(
            ui,
            cx,
            l.slash,
            "/",
            FontSize::Pt10,
            Align2::CENTER_CENTER,
            text,
        );
        let second = value(Menu::Second, o).to_string();
        menu(ui, Menu::Second, "second", l.second, &second);
        self.options.mode = [Mode::Signature, Mode::Division][mode];

        let mut action = None;
        let ends = [
            ("cancel", l.cancel, "Cancel", SongOptionsAction::Cancel),
            ("ok", l.ok, "OK", SongOptionsAction::Ok(self.options)),
        ];
        for (part, rect, label, end) in ends {
            let b = Button::new(id.with(part), rect, label, false, c);
            if button(ui, cx, &b, true).clicked() {
                action = Some(end);
            }
        }
        // The open menu, over the band; it takes Escape first.
        if let Some((menu, at)) = self.menu {
            let items = items(menu, self.options);
            let result = popup::show(ui.ctx(), id.with("menu"), at, screen, &items, cx);
            if let Some(v) = result.selected {
                let o = &mut self.options;
                match (menu, o.mode) {
                    (Menu::Tempo, _) => o.tempo = v,
                    (Menu::Beats, _) => o.beats = v as u8,
                    (Menu::Second, Mode::Signature) => o.steps = v as u8,
                    (Menu::Second, Mode::Division) => o.division = v,
                }
            }
            if result.close {
                self.menu = None;
            }
        }
        if crate::ui::close_pressed(ui.ctx()) {
            action = Some(SongOptionsAction::Cancel);
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Color32, Event, Shape};

    use super::*;
    use crate::ui::offscreen::{Offscreen, SCREEN};
    use eframe::egui::{Key, Modifiers};

    fn sig(subdivision: u8, beats_per_bar: u8) -> TimeSig {
        TimeSig {
            subdivision,
            beats_per_bar,
        }
    }

    /// The TDIV chunk of `song` as written.
    fn tdiv(song: &Song) -> Vec<u8> {
        let bytes = crate::flm::write(song).unwrap();
        let chunks = crate::flm::chunk::file_chunks(&bytes).unwrap();
        (chunks.map(Result::unwrap))
            .find(|c| &c.id == b"TDIV")
            .unwrap()
            .payload
            .to_vec()
    }

    #[test]
    fn the_dialog_opens_on_140_bpm_and_4_4() {
        let o = SongOptionsDialog::default().options;
        assert_eq!(
            o,
            SongOptions {
                tempo: 140,
                mode: Mode::Signature,
                beats: 4,
                steps: 4,
                division: 4,
            }
        );
        let mut song = Song {
            tempo_bpm: 99.0,
            time_sig: sig(3, 7),
            ..Song::default()
        };
        o.apply(&mut song);
        assert_eq!((tdiv(&song), song.tempo_bpm), (vec![4, 4], 140.0));
        // The other mode at the defaults writes the same.
        let division = SongOptions {
            mode: Mode::Division,
            ..o
        };
        assert_eq!(
            (division.time_sig(), division.tempo_bpm()),
            (sig(4, 4), 140.0)
        );
    }

    /// The pair is (beats per bar, subdivision); beats per bar is TDIV byte 1, the subdivision byte 0.
    #[test]
    fn time_signature_writes_the_first_menu_to_byte_1_and_the_second_to_byte_0_at_the_picked_tempo()
    {
        for beats in 1..=16 {
            for steps in 1..=16 {
                for tempo in tempos() {
                    let o = SongOptions {
                        tempo,
                        beats,
                        steps,
                        // Not this mode's: ignored.
                        division: 16,
                        ..SongOptions::default()
                    };
                    assert_eq!(o.time_sig(), sig(steps, beats), "{o:?}");
                    assert_eq!(o.tempo_bpm(), f64::from(tempo), "{o:?}");
                }
                let mut song = Song::default();
                SongOptions {
                    beats,
                    steps,
                    ..SongOptions::default()
                }
                .apply(&mut song);
                assert_eq!(tdiv(&song), [steps, beats]);
            }
        }
    }

    #[test]
    fn time_division_writes_the_first_menu_and_4_and_scales_the_tempo_by_division_over_4() {
        for (division, factor) in DIVISIONS.into_iter().zip([0.5, 1.0, 2.0, 4.0]) {
            for beats in 1..=16 {
                for tempo in tempos() {
                    let o = SongOptions {
                        tempo,
                        mode: Mode::Division,
                        beats,
                        // Not this mode's: ignored.
                        steps: 13,
                        division,
                    };
                    assert_eq!(o.time_sig(), sig(4, beats), "{o:?}");
                    assert_eq!(o.tempo_bpm(), f64::from(tempo) * factor, "{o:?}");
                    let mut song = Song::default();
                    o.apply(&mut song);
                    assert_eq!(
                        (tdiv(&song), song.tempo_bpm),
                        (vec![4, beats], o.tempo_bpm())
                    );
                }
            }
        }
    }

    #[test]
    fn the_menus_list_the_spec_items_with_the_current_one_highlighted() {
        let o = SongOptions::default();
        let cells = |items: Vec<PopupItem<u16>>| -> Vec<(String, usize, usize, bool)> {
            (items.into_iter())
                .map(|i| (i.label.into_owned(), i.col, i.row, i.current))
                .collect()
        };
        let tempo = cells(items(Menu::Tempo, o));
        assert_eq!(tempo.len(), 13);
        assert_eq!(tempo[0], ("80 BPM".into(), 0, 0, false));
        assert_eq!(tempo[6], ("140 BPM".into(), 0, 6, true));
        assert_eq!(tempo[12], ("200 BPM".into(), 0, 12, false));
        // 1–8 in the left column, 9–16 in the right.
        let beats = cells(items(Menu::Beats, SongOptions { beats: 9, ..o }));
        assert_eq!(beats.len(), 16);
        assert_eq!(beats[0], ("1".into(), 0, 0, false));
        assert_eq!(beats[7], ("8".into(), 0, 7, false));
        assert_eq!(beats[8], ("9".into(), 1, 0, true));
        assert_eq!(beats[15], ("16".into(), 1, 7, false));
        let steps = cells(items(Menu::Second, SongOptions { steps: 16, ..o }));
        assert_eq!(steps.len(), 16);
        assert_eq!(steps[15], ("16".into(), 1, 7, true));
        let division = SongOptions {
            mode: Mode::Division,
            division: 8,
            ..o
        };
        assert_eq!(
            cells(items(Menu::Second, division)),
            [
                ("2".into(), 0, 0, false),
                ("4".into(), 0, 1, false),
                ("8".into(), 0, 2, true),
                ("16".into(), 0, 3, false),
            ]
        );
    }

    /// The dialog driven offscreen at U = 32.
    struct Rig {
        screen: Offscreen,
        dialog: SongOptionsDialog,
        action: Option<SongOptionsAction>,
        texts: Vec<(String, Rect)>,
        /// The filled rects of the last frame, in paint order.
        rects: Vec<(Rect, Color32)>,
    }

    impl Rig {
        fn new() -> Self {
            let mut rig = Self {
                screen: Offscreen::new(),
                dialog: SongOptionsDialog::default(),
                action: None,
                texts: Vec::new(),
                rects: Vec::new(),
            };
            rig.frame(vec![]);
            rig
        }

        fn draw(
            &mut self,
            events: Option<Vec<Event>>,
            click: Option<Pos2>,
        ) -> Option<SongOptionsAction> {
            let (dialog, mut action) = (&mut self.dialog, None);
            let mut f = |ui: &mut Ui, cx: &UiCtx| {
                action = dialog.show(ui, cx, SCREEN).or(action);
            };
            let shapes = match click {
                Some(pos) => {
                    self.screen.click(pos, &mut f);
                    self.screen.frame(vec![], &mut f)
                }
                None => self.screen.frame(events.unwrap_or_default(), &mut f),
            };
            self.texts = (shapes.iter())
                .filter_map(|c| match &c.shape {
                    Shape::Text(t) => Some((
                        t.galley.text().to_owned(),
                        t.galley.rect.translate(t.pos.to_vec2()),
                    )),
                    _ => None,
                })
                .collect();
            self.rects = (shapes.iter())
                .filter_map(|c| match &c.shape {
                    Shape::Rect(r) => Some((r.rect, r.fill)),
                    _ => None,
                })
                .collect();
            self.action = action;
            action
        }

        fn frame(&mut self, events: Vec<Event>) -> Option<SongOptionsAction> {
            self.draw(Some(events), None)
        }

        fn click(&mut self, pos: Pos2) -> Option<SongOptionsAction> {
            self.draw(None, Some(pos))
        }

        /// Where `text` was drawn last (a popup item over the button it covers).
        fn find(&self, text: &str) -> Rect {
            let found = self.texts.iter().rev().find(|(t, _)| t == text);
            found
                .unwrap_or_else(|| panic!("{text:?} not drawn: {:?}", self.texts))
                .1
        }

        fn click_text(&mut self, text: &str) -> Option<SongOptionsAction> {
            self.click(self.find(text).center())
        }

        fn click_id(&mut self, part: &str) -> Option<SongOptionsAction> {
            let id = Id::new(ID).with(part);
            let rect = self.screen.ctx.read_response(id).expect("drawn").rect;
            self.click(rect.center())
        }

        fn texts(&self) -> Vec<&str> {
            self.texts.iter().map(|(t, _)| t.as_str()).collect()
        }
    }

    #[test]
    fn the_band_shows_the_title_the_menus_and_the_buttons_top_to_bottom() {
        let rig = Rig::new();
        assert_eq!(
            rig.texts(),
            [
                TITLE,
                "140 BPM",
                "Time signature:",
                "Time division:",
                "4",
                "/",
                "4",
                "Cancel",
                "OK"
            ]
        );
        let top = |t: &str| rig.find(t).center().y;
        assert!(top(TITLE) < top("140 BPM"));
        assert!(top("140 BPM") < top("Time signature:"));
        assert!(top("Time signature:") < top("/"));
        assert!(top("/") < top("OK"));
        assert_eq!(top("Cancel"), top("OK"));
        assert!(rig.find("Cancel").center().x < rig.find("OK").center().x);
        // Everything inside the band, which is 8 U across the middle of the screen.
        let band = layout(SCREEN, &crate::ui::offscreen::metrics()).band;
        assert_eq!(band, Rect::from_x_y_ranges(0.0..=1170.0, 142.0..=398.0));
        assert!(rig.texts.iter().all(|(_, r)| band.contains_rect(*r)));
    }

    /// The dialog base: screen dim α 0.6, then a `moduleFill` box, from the first frame.
    #[test]
    fn the_dialog_base_dims_the_screen_then_fills_the_band_with_module_fill() {
        let rig = Rig::new();
        let c = crate::theme::theme_by_id(0).colors;
        let band = layout(SCREEN, &crate::ui::offscreen::metrics()).band;
        crate::ui::dialog::assert_base(&rig.rects, &c, SCREEN, band);
    }

    #[test]
    fn ok_returns_the_choices_and_cancel_and_escape_return_cancel() {
        let mut rig = Rig::new();
        assert_eq!(
            rig.click_text("OK"),
            Some(SongOptionsAction::Ok(SongOptions::default()))
        );
        let mut rig = Rig::new();
        assert_eq!(rig.click_text("Cancel"), Some(SongOptionsAction::Cancel));
        let mut rig = Rig::new();
        let escape = Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(rig.frame(vec![escape]), Some(SongOptionsAction::Cancel));
        // A click outside the band does nothing.
        let mut rig = Rig::new();
        assert_eq!(rig.click(pos2(20.0, 20.0)), None);
        assert!(rig.texts().contains(&TITLE));
    }

    #[test]
    fn the_menus_pick_their_values_and_the_mode_swaps_the_second_menu() {
        let mut rig = Rig::new();
        assert_eq!(rig.click_text("140 BPM"), None);
        assert!(rig.texts().contains(&"200 BPM"), "the tempo menu is open");
        assert_eq!(rig.click_text("90 BPM"), None);
        assert!(!rig.texts().contains(&"200 BPM"), "and closed");
        rig.click_id("beats");
        rig.click_text("7");
        rig.click_id("second");
        rig.click_text("12");
        assert_eq!(
            rig.dialog.options,
            SongOptions {
                tempo: 90,
                beats: 7,
                steps: 12,
                ..SongOptions::default()
            }
        );
        assert_eq!(
            rig.texts()[1..],
            ["90 BPM", MODES[0], MODES[1], "7", "/", "12", "Cancel", "OK"]
        );
        // `Time division:`: the second menu is the division, 4 at first.
        rig.click_text("Time division:");
        assert_eq!(rig.dialog.options.mode, Mode::Division);
        assert_eq!(rig.texts()[4..7], ["7", "/", "4"]);
        rig.click_id("second");
        assert!(
            !rig.texts().contains(&"12"),
            "the division menu: {:?}",
            rig.texts()
        );
        rig.click_text("16");
        let picked = SongOptions {
            tempo: 90,
            mode: Mode::Division,
            beats: 7,
            steps: 12,
            division: 16,
        };
        assert_eq!(rig.click_text("OK"), Some(SongOptionsAction::Ok(picked)));
        assert_eq!((picked.time_sig(), picked.tempo_bpm()), (sig(4, 7), 360.0));
        // Back to `Time signature:`, the subdivision kept.
        rig.click_text("Time signature:");
        assert_eq!(rig.texts()[4..7], ["7", "/", "12"]);
    }

    #[test]
    fn escape_closes_an_open_menu_first() {
        let mut rig = Rig::new();
        rig.click_text("140 BPM");
        let escape = Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(rig.frame(vec![escape]), None);
        rig.frame(vec![]);
        assert!(!rig.texts().contains(&"200 BPM"));
        assert!(rig.texts().contains(&TITLE));
    }
}

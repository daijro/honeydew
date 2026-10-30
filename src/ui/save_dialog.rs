//! The Save dialog and its render page: a full-width band across the middle of the screen with a close X at the
//! right. Centred in it, top to bottom: a small `File Name` label over a dark text field, the selected format's
//! description over the format tabs `FLM FLP ZIP MIDI WAV FLAC OGG MP3`, a divider, and the light rectangular buttons
//! `Save` and `Send`. It is on the dialog base ([`crate::ui::dialog`]): the screen dimmed at α 0.6 and the band in
//! `moduleFill`, from the first frame.
//!
//! The format tabs are a row of equal cells, labels FontSize 2 in `text`; the selected one has a `text` underline 2t
//! thick along its cell's bottom, and the mouse lights a cell with `text` at α 0.05. The divider (1 px of `text` at
//! α 0.18, 0.25 U in from the band's sides) runs along the tabs' bottom. `Send` (no share target) is dimmed and takes no
//! input. With FLP selected, an `Include rendered audio` toggle (off whenever a dialog opens) sits left of `Save`: on,
//! the project also holds each channel's render as an audio clip. With MIDI selected (and in `Save as MIDI`), an `Apply
//! note effects` toggle (on by default, kept in the settings) sits there: on, the notes are written as the channels'
//! note effects play them.
//! `Save` with FLM, FLP (an FL Studio project and its samples, as a `.zip`), ZIP or MIDI saves; with WAV, FLAC, OGG or
//! MP3 it turns the band into
//! the render page: the title `<name>.<ext>` (with Split on, the stems' folder `<name> - Multitrack <ext>`) over a dim
//! `Estimated length:`, the `Normalize level` and `High quality` toggles on the left, `Split mixer tracks` (and, only
//! while it is on, `Enable master FX`) on the right, the range and remainder spinners side by side in the centre under
//! their captions `Range` and `Tail`, `Render` under them, and `Back` (the arrow icon) left of the close X. The band is
//! modal: a backdrop under it takes every click elsewhere, and only the close X, Escape, `Save` (or Enter in the field)
//! and `Render` end it.
//!
//! The render options ([`RenderPage`]) all start off. The caller keeps them while the app runs and opens each dialog
//! with [`RenderPage::reopened`], which turns `Split mixer tracks` off.
//!
//! A preset's dialog (`Save preset`, `Save Rack preset`, `Save Drumset`) and a one-format dialog (`Save as MIDI`) have
//! no format row: the description names the type, and the title sits at the band's left, level with the close X.

use eframe::egui::{
    self, Align, Align2, Area, Context, Id, Key, Order, Rect, Sense, TextEdit, Ui, pos2, vec2,
};

use crate::icons::{IconId, draw_icon};
use crate::metrics::{FontSize, Metrics};
use crate::project_files::PresetKind;
use crate::render::{Remainder, RenderRange};
use crate::theme::DISABLED_ALPHA;
use crate::ui::UiCtx;
use crate::ui::buttons::close_x;
use crate::ui::main_menu::{SONGS_FOLDER, duration};
use crate::ui::widgets::{
    Button, Control, ParamMenu, Pill, STRIP_H, button, pill_toggle, spinner_index, truncated,
};

/// The format tabs, in order.
pub const FORMATS: [&str; 8] = ["FLM", "FLP", "ZIP", "MIDI", "WAV", "FLAC", "OGG", "MP3"];
/// The description over the format tabs for FLM.
const FLM_DESCRIPTION: &str = "Project (.flm)";
/// The range spinner's and the remainder spinner's items, in order.
const RANGES: [&str; 3] = ["Full song", "Trim initial silence", "Selection"];
const REMAINDERS: [&str; 3] = ["Leave remainder", "Cut remainder", "Wrap remainder"];
/// The `Estimated length:` line's alpha: dim, but not the disabled look.
const DIM_ALPHA: f32 = 0.5;
/// A format tab's light under the mouse, and the divider: `text` at these alphas.
const HOVER_ALPHA: f32 = 0.05;
const DIVIDER_ALPHA: f32 = 0.18;

// Sizes in U.
const BAND_H: f32 = 7.0;
/// Rows from the band's top: (top, height).
const NAME_LABEL_ROW: (f32, f32) = (0.5, 0.5);
const FIELD_ROW: (f32, f32) = (1.0, 1.25);
const DESCRIPTION_ROW: (f32, f32) = (2.75, 0.5);
const FORMAT_ROW: (f32, f32) = (3.5, 1.25);
const BUTTON_ROW: (f32, f32) = (5.5, 1.0);
const FIELD_W: f32 = 10.0;
/// A format tab's cell: the row is 16 U, which the narrowest screen (20 U) holds.
const FORMAT_W: f32 = 2.0;
const BUTTON_W: f32 = 3.0;
const BUTTON_GAP: f32 = 0.5;
/// The close X: the transport's round button size, inset from the band's top-right corner.
const CLOSE_D: f32 = 1.25;
const CLOSE_INSET: f32 = 0.5;
/// The render page: its title and estimate lines (at most this wide, and clear of `Back`), the toggles' two rows (the
/// spinners centred on the line between them), the spinners (their captions in a strip above) and the gap from them to
/// each toggle column. `Render` takes `Save`'s row and `Back` sits `BACK_GAP` left of the close X. `TOGGLE_W` holds the
/// longest label. The spinners take what the toggle columns leave, up to `SPINNER_W`: 3.375 U on the narrowest
/// screen, 20 U (U ≤ width / 20), which still holds `Trim initial silence` (3.5 U) apart from its neighbour's text.
const HEADING_ROW: (f32, f32) = (0.5, 0.75);
const ESTIMATE_ROW: (f32, f32) = (1.25, 0.5);
const HEADING_W: f32 = 16.0;
const TOGGLE_ROWS: [(f32, f32); 2] = [(2.0, 1.5), (3.5, 1.5)];
const TOGGLE_W: f32 = 6.0;
const SPINNER_ROW: (f32, f32) = (2.5, 2.0);
const SPINNER_W: f32 = 4.5;
const SPINNER_GAP: f32 = 0.25;
const COLUMN_GAP: f32 = 0.25;
const EDGE_INSET: f32 = 0.25;
const BACK_GAP: f32 = 0.25;

/// The formats of the format row, in its order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExportFormat {
    #[default]
    Flm,
    Flp,
    Zip,
    Midi,
    Wav,
    Flac,
    Ogg,
    Mp3,
}

impl ExportFormat {
    pub const ALL: [Self; 8] = [
        Self::Flm,
        Self::Flp,
        Self::Zip,
        Self::Midi,
        Self::Wav,
        Self::Flac,
        Self::Ogg,
        Self::Mp3,
    ];

    /// The stored format id.
    pub fn id(self) -> u8 {
        [0, 8, 6, 1, 2, 5, 7, 3][self as usize]
    }

    /// The format stored as `id`.
    pub fn from_id(id: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == id)
    }

    /// Its tab's text.
    pub fn button(self) -> &'static str {
        FORMATS[self as usize]
    }

    /// The line over the format tabs.
    pub fn description(self) -> &'static str {
        use ExportFormat::*;
        match self {
            Flm => FLM_DESCRIPTION,
            Flp => "Project (.flp)",
            Zip => "Project Archive",
            Midi => "MIDI Sequence",
            Wav => "Audio Wave",
            Flac => "Lossless FLAC Audio",
            Ogg | Mp3 => "Compressed Audio",
        }
    }

    /// The file extension, without the dot. FLP projects are saved as `.zip`.
    pub fn ext(self) -> &'static str {
        use ExportFormat::*;
        match self {
            Flm => "flm",
            Flp | Zip => "zip",
            Midi => "mid",
            Wav => "wav",
            Flac => "flac",
            Ogg => "ogg",
            Mp3 => "mp3",
        }
    }

    /// The folder under the user folder its files go to.
    pub fn folder(self) -> &'static str {
        use ExportFormat::*;
        match self {
            Flm | Flp | Zip => SONGS_FOLDER,
            Midi => crate::project_files::MIDI_FOLDER,
            Wav | Flac | Ogg | Mp3 => crate::project_files::TRACKS_FOLDER,
        }
    }

    /// Rendered audio: `Save` opens the render page.
    pub fn is_audio(self) -> bool {
        use ExportFormat::*;
        matches!(self, Wav | Flac | Ogg | Mp3)
    }

    /// Selectable: every format (FLP is the FL Studio project, `crate::flp::export`).
    pub fn available(self) -> bool {
        true
    }
}

/// The render page's options. The default is the state at start: `Full song`,
/// `Leave remainder`, every toggle off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderPage {
    pub range: RenderRange,
    pub remainder: Remainder,
    pub normalize: bool,
    pub high_quality: bool,
    /// `Split mixer tracks`.
    pub split: bool,
    /// `Enable master FX`: shown only while `split` is on, and kept while hidden.
    pub master_effects: bool,
}

impl RenderPage {
    /// The options a newly opened dialog starts with, from the last dialog's: all kept while the app runs, except
    /// `Split mixer tracks`, which resets to off.
    pub fn reopened(self) -> Self {
        Self {
            split: false,
            ..self
        }
    }
}

/// Which page the band shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    /// The name field, the format tabs, `Save` and `Send`.
    #[default]
    Formats,
    /// The render page of an audio format.
    Render,
}

/// What the user did in the dialog this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogAction {
    /// `Save`, or Enter in the name field, with a format that is not audio (a preset dialog's is FLM).
    Save(ExportFormat),
    /// `Render` on the render page, with its options (`master_effects` only with `split`). The format is
    /// [`SaveDialog::format`], the name [`SaveDialog::name`].
    Render(RenderPage),
    /// The close X, or Escape.
    Close,
}

/// The dialog's state while it is open. The caller keeps it from frame to frame.
pub struct SaveDialog {
    /// The `File Name` field.
    pub name: String,
    /// A preset's dialog; `None`: the song's.
    pub preset: Option<PresetKind>,
    /// A one-format dialog with this title and no format tabs ([`SaveDialog::save_as_midi`]).
    pub fixed: Option<&'static str>,
    /// The selected format.
    pub format: ExportFormat,
    pub page: Page,
    pub render: RenderPage,
    /// FLP's `Include rendered audio`.
    pub flp_audio: bool,
    /// MIDI's `Apply note effects` (#31): the notes as the channels' note effects play them. The caller keeps it.
    pub midi_effects: bool,
    /// The caller's estimated lengths in seconds, for `Estimated length:`: the song's (`Full song`, `Trim initial
    /// silence`) and the selection's. `None`: there is no selection, and the range spinner skips `Selection`.
    pub song_secs: f32,
    pub selection_secs: Option<f32>,
}

impl SaveDialog {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            preset: None,
            fixed: None,
            format: ExportFormat::Flm,
            page: Page::Formats,
            render: RenderPage::default(),
            flp_audio: false,
            midi_effects: true,
            song_secs: 0.0,
            selection_secs: None,
        }
    }

    /// The track menu's `Save as MIDI` dialog: MIDI only, titled `Save as MIDI`.
    pub fn save_as_midi(name: impl Into<String>) -> Self {
        Self {
            fixed: Some("Save as MIDI"),
            format: ExportFormat::Midi,
            ..Self::new(name)
        }
    }

    /// Shows the dialog over everything; call every frame while it is open. On the format page the name field keeps
    /// the keyboard focus.
    pub fn show(&mut self, ctx: &Context, cx: &UiCtx) -> Option<DialogAction> {
        let (c, m) = (&cx.theme.colors, cx.m);
        let screen = ctx.content_rect();
        let l = layout(screen, m);
        let id = Id::new("save_dialog");
        // The dialog base: the screen dimmed at α 0.6, then a `moduleFill` band, from the first frame; below the band
        // (`Foreground`), above the screens, the backdrop.
        crate::ui::dialog::dialog_box(ctx, id, (c, m), screen, l.band);
        // A kept `Selection` with no selection now reads as the first item.
        if self.selection_secs.is_none() && self.render.range == RenderRange::Selection {
            self.render.range = RenderRange::Full;
        }

        let mut action = None;
        Area::new(id)
            .order(Order::Foreground)
            .fixed_pos(l.band.min)
            .constrain(false)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.allocate_exact_size(l.band.size(), Sense::hover());
                action = match self.page {
                    Page::Formats => self.formats_page(ui, cx, id, &l),
                    Page::Render => self.render_page(ui, cx, id, &l),
                };
                let surface = (cx.theme.colors.menu_fill.to_color32(), 0.375);
                if close_x(ui, cx, id.with("close"), l.close, surface, 0.7).clicked() {
                    action = Some(DialogAction::Close);
                }
            });
        // After the band, so that an open spinner popup takes Escape first.
        if crate::ui::close_pressed(ctx) {
            action = Some(DialogAction::Close);
        }
        action
    }

    /// `Save`: saves a project format, opens the render page for an audio one.
    fn save(&mut self) -> Option<DialogAction> {
        if self.format.is_audio() {
            self.page = Page::Render;
            return None;
        }
        Some(DialogAction::Save(self.format))
    }

    fn formats_page(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        id: Id,
        l: &DialogLayout,
    ) -> Option<DialogAction> {
        let (c, m) = (&cx.theme.colors, cx.m);
        let (text, bg, fill) = (
            c.text.to_color32(),
            c.background.to_color32(),
            c.menu_fill.to_color32(),
        );
        let mut action = None;
        let painter = ui.painter().clone();
        let small = m.font_id(FontSize::Pt8);
        painter.text(
            l.name_label.center(),
            Align2::CENTER_CENTER,
            "File Name",
            small.clone(),
            text,
        );
        painter.text(
            l.description.center(),
            Align2::CENTER_CENTER,
            self.preset
                .map_or(self.format.description(), PresetKind::format),
            small.clone(),
            text,
        );
        let title = self.preset.map(PresetKind::title).or(self.fixed);
        if let Some(title) = title {
            // 12 pt, shrunk to fit the room left of the field on the 20 U panel.
            let pt = FontSize::Pt12.points() as u8;
            crate::ui::widgets::shrink(&painter, m, l.title, Align2::LEFT_CENTER, title, pt, text);
        }

        painter.rect_filled(l.field, 0.0, bg);
        let visuals = ui.visuals_mut();
        visuals.text_cursor.stroke.color = text;
        visuals.selection.bg_fill = c.accent.to_color32().gamma_multiply(0.5);
        visuals.selection.stroke.color = text;
        let field = TextEdit::singleline(&mut self.name)
            .id(id.with("name"))
            .frame(false)
            .margin(egui::Margin::ZERO)
            .font(m.font_id(FontSize::Pt10))
            .text_color(text)
            .desired_width(f32::INFINITY)
            .min_size(l.field.size())
            .horizontal_align(Align::Center)
            .vertical_align(Align::Center);
        let mut field = ui.put(l.field, field);
        crate::android_text::edit(&mut field, &mut self.name, usize::MAX);
        if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            action = self.save();
        } else if ui.memory(|mem| mem.focused().is_none()) {
            field.request_focus();
        }

        // The format tabs: the selected one underlined, the one under the mouse lit; the unavailable ones dimmed and
        // inert. The divider along their bottom.
        let shown = if title.is_none() { FORMATS.len() } else { 0 };
        let row = ExportFormat::ALL.into_iter().zip(l.formats).take(shown);
        for (i, (format, cell)) in row.enumerate() {
            let mut ink = text;
            if !format.available() {
                ink = text.gamma_multiply(DISABLED_ALPHA);
            } else {
                let tab = ui.interact(cell, id.with(("format", i)), Sense::click());
                if tab.hovered() {
                    painter.rect_filled(cell, 0.0, text.gamma_multiply(HOVER_ALPHA));
                }
                if tab.clicked() {
                    self.format = format;
                }
            }
            if format == self.format {
                let line = cell.with_min_y(cell.bottom() - 2.0 * m.t());
                painter.rect_filled(line, 0.0, text);
            }
            let font = m.font_id(FontSize::Pt10);
            painter.text(
                cell.center(),
                Align2::CENTER_CENTER,
                format.button(),
                font,
                ink,
            );
        }
        if shown > 0 {
            let divider = l.divider.with_min_y(l.divider.bottom() - 1.0 / m.ppp());
            painter.rect_filled(divider, 0.0, text.gamma_multiply(DIVIDER_ALPHA));
        }

        if title.is_none() && self.format == ExportFormat::Flp {
            let label = "Include rendered audio";
            toggle(ui, cx, id, l.flp_audio, label, &mut self.flp_audio);
        }
        // The song's dialog and `Save as MIDI` alike.
        if self.format == ExportFormat::Midi {
            let label = "Apply note effects";
            toggle(ui, cx, id, l.flp_audio, label, &mut self.midi_effects);
        }
        let save = crate::ui::dialog::button(id.with("save"), l.save, "Save", c, m);
        if button(ui, cx, &save, true).clicked() {
            action = self.save();
        }
        let send = Button {
            fill_on: text.gamma_multiply(DISABLED_ALPHA),
            text_on: fill.gamma_multiply(DISABLED_ALPHA),
            disabled: true,
            ..crate::ui::dialog::button(id.with("send"), l.send, "Send", c, m)
        };
        button(ui, cx, &send, true);
        action
    }

    fn render_page(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        id: Id,
        l: &DialogLayout,
    ) -> Option<DialogAction> {
        let (c, m) = (&cx.theme.colors, cx.m);
        let text = c.text.to_color32();
        let painter = ui.painter().clone();
        // With Split on, the stems' folder.
        let ext = self.format.ext();
        let tail = match self.render.split {
            true => format!(" - Multitrack {ext}"),
            false => format!(".{ext}"),
        };
        let heading = format!("{}{tail}", self.name);
        let font = m.font_id(FontSize::Pt12);
        truncated(
            &painter,
            l.heading,
            Align2::CENTER_CENTER,
            &heading,
            font,
            text,
            // A long name is cut in the middle: the extension (or the folder's ending) stays.
            tail.chars().count(),
        );
        let secs = match self.render.range {
            RenderRange::Selection => self.selection_secs.unwrap_or(self.song_secs),
            _ => self.song_secs,
        };
        painter.text(
            l.estimate.center(),
            Align2::CENTER_CENTER,
            // The PROJECT tab's `Project Length:` form.
            format!("Estimated length: {}", duration(secs.max(0.0) as u64)),
            m.font_id(FontSize::Pt8),
            text.gamma_multiply(DIM_ALPHA),
        );

        let r = &mut self.render;
        let [normalize, hq, split, master] = l.toggles;
        toggle(ui, cx, id, normalize, "Normalize level", &mut r.normalize);
        toggle(ui, cx, id, hq, "High quality", &mut r.high_quality);
        toggle(ui, cx, id, split, "Split mixer tracks", &mut r.split);
        if r.split {
            toggle(
                ui,
                cx,
                id,
                master,
                "Enable master FX",
                &mut r.master_effects,
            );
        }

        let no_selection: &[usize] = if self.selection_secs.is_none() {
            &[RenderRange::Selection as usize]
        } else {
            &[]
        };
        let mut range = r.range as usize;
        spinner(
            ui,
            cx,
            id.with("range"),
            l.spinners[0],
            "Range",
            &RANGES,
            &mut range,
            no_selection,
        );
        r.range = [
            RenderRange::Full,
            RenderRange::TrimSilence,
            RenderRange::Selection,
        ][range];
        let mut remainder = r.remainder as usize;
        spinner(
            ui,
            cx,
            id.with("remainder"),
            l.spinners[1],
            "Tail",
            &REMAINDERS,
            &mut remainder,
            &[],
        );
        r.remainder = [Remainder::Leave, Remainder::Cut, Remainder::Wrap][remainder];

        let back = ui.interact(l.back, id.with("back"), Sense::click());
        draw_icon(ui.painter(), l.back, IconId::Previous, text, 0.7, m);
        if back.clicked() {
            self.page = Page::Formats;
        }
        let render = crate::ui::dialog::button(id.with("render"), l.render, "Render", c, m);
        button(ui, cx, &render, true)
            .clicked()
            .then_some(DialogAction::Render(RenderPage {
                // Kept while hidden, but it only acts with Split on.
                master_effects: self.render.split && self.render.master_effects,
                ..self.render
            }))
    }
}

/// A render page toggle in `rect`: the pill at its left, `label` right of it, in `text`. (The MIDI import dialog's too.)
pub(crate) fn toggle(ui: &mut Ui, cx: &UiCtx, id: Id, rect: Rect, label: &str, on: &mut bool) {
    let text = cx.theme.colors.text.to_color32();
    let c = Control {
        label_font: FontSize::Pt10,
        // Options, not parameters: no parameter menu.
        menu: ParamMenu::Unbound,
        ..Control::new(id.with(label), rect, label, text, 0.0)
    };
    let pill = Pill {
        label_right: true,
        ..Pill::default()
    };
    let mut v = if *on { 1.0 } else { 0.0 };
    if pill_toggle(ui, cx, &c, pill, &mut v, |_| String::new()).changed() {
        *on = v >= 0.5;
    }
}

/// A render page spinner in `rect` over `items`, skipping the `disabled` ones, with its `caption` in the controls'
/// label strip above `rect`.
#[allow(clippy::too_many_arguments)]
fn spinner(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    caption: &str,
    items: &[&str],
    index: &mut usize,
    disabled: &[usize],
) {
    let text = cx.theme.colors.text.to_color32();
    let rect = rect.with_min_y(rect.top() - cx.m.u(STRIP_H));
    let c = Control {
        menu: ParamMenu::Off,
        disabled_items: disabled,
        ..Control::new(id, rect, caption, text, 0.0)
    };
    spinner_index(ui, cx, &c, items, index);
}

/// Where everything of the dialog goes on a `screen`. Both pages share the band and the close X.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialogLayout {
    pub band: Rect,
    pub close: Rect,
    /// A preset or one-format dialog's title line: from the band's left inset to the field's left (empty when that is
    /// left of it), level with the close X. A longer title is shortened to it.
    pub title: Rect,
    /// The `File Name` label's line.
    pub name_label: Rect,
    pub field: Rect,
    /// The format description's line.
    pub description: Rect,
    /// The format tabs' cells, and the divider's band: the tabs' bottom row of pixels is its line.
    pub formats: [Rect; 8],
    pub divider: Rect,
    pub save: Rect,
    pub send: Rect,
    /// FLP's `Include rendered audio` (MIDI's `Apply note effects`), in the left column level with `Save`.
    pub flp_audio: Rect,
    /// The render page: the `<name>.<ext>` title line and the `Estimated length:` line under it.
    pub heading: Rect,
    pub estimate: Rect,
    /// `Normalize level`, `High quality` (the left column), `Split mixer tracks`, `Enable master FX` (the right).
    pub toggles: [Rect; 4],
    /// The range and the remainder spinner, side by side, without their caption strip above.
    pub spinners: [Rect; 2],
    pub render: Rect,
    /// `Back`, the arrow icon in a square the close X's size, left of it.
    pub back: Rect,
}

/// Lays the dialog out on `screen`, in whole device pixels.
pub fn layout(screen: Rect, m: &Metrics) -> DialogLayout {
    let band_h = m.round_px(m.u(BAND_H));
    let band = Rect::from_x_y_ranges(
        crate::ui::dialog::floating_x(screen, m),
        m.round_px(screen.center().y - band_h / 2.0)
            ..=m.round_px(screen.center().y - band_h / 2.0) + band_h,
    );
    let mid = screen.center().x;
    let top = |t: f32| band.top() + m.round_px(m.u(t));
    // A row of `n` items `w` wide with gaps `gap`, centred, at `row`.
    let row = |(t, h): (f32, f32), w: f32, gap: f32, n: usize| -> Vec<Rect> {
        let (w, gap, h) = (m.round_px(m.u(w)), m.round_px(m.u(gap)), m.round_px(m.u(h)));
        let left = m.round_px(mid - (n as f32 * w + (n - 1) as f32 * gap) / 2.0);
        (0..n)
            .map(|i| Rect::from_min_size(pos2(left + i as f32 * (w + gap), top(t)), vec2(w, h)))
            .collect()
    };
    let line = |r: (f32, f32)| row(r, FIELD_W, 0.0, 1)[0];
    let formats = row(FORMAT_ROW, FORMAT_W, 0.0, FORMATS.len());
    // The divider: placed between the format tabs and the options, inset from the band's sides.
    let inset = m.round_px(m.u(EDGE_INSET));
    let divider = Rect::from_x_y_ranges(
        band.left() + inset..=band.right() - inset,
        formats[0].y_range(),
    );
    let buttons = row(BUTTON_ROW, BUTTON_W, BUTTON_GAP, 2);
    let inset = m.u(CLOSE_INSET + CLOSE_D / 2.0);
    let close = Rect::from_center_size(
        pos2(
            m.round_px(band.right() - inset),
            m.round_px(band.top() + inset),
        ),
        vec2(m.u(CLOSE_D), m.u(CLOSE_D)),
    );
    let field = line(FIELD_ROW);
    let spinner_w = (band.width() / 2.0 / m.u(1.0)
        - (EDGE_INSET + TOGGLE_W + COLUMN_GAP + SPINNER_GAP / 2.0))
        .min(SPINNER_W);
    let spinners = row(SPINNER_ROW, spinner_w, SPINNER_GAP, 2);
    let (toggle_w, gap) = (m.round_px(m.u(TOGGLE_W)), m.round_px(m.u(COLUMN_GAP)));
    let toggle = |left: f32, (t, h): (f32, f32)| {
        Rect::from_min_size(pos2(left, top(t)), vec2(toggle_w, m.round_px(m.u(h))))
    };
    let (left, right) = (
        spinners[0].left() - gap - toggle_w,
        spinners[1].right() + gap,
    );
    let back = close.translate(vec2(-m.round_px(m.u(CLOSE_D + BACK_GAP)), 0.0));
    // The title lines: centred, and clear of `Back` by the same gap.
    let heading_w = (m.u(HEADING_W) / 2.0).min(back.left() - m.u(BACK_GAP) - mid);
    let heading = |(t, h): (f32, f32)| {
        let half = m.round_px(heading_w);
        Rect::from_x_y_ranges(
            mid - half..=mid + half,
            top(t)..=top(t) + m.round_px(m.u(h)),
        )
    };
    DialogLayout {
        band,
        close,
        title: {
            let left = m.round_px(band.left() + m.u(CLOSE_INSET));
            Rect::from_x_y_ranges(left..=field.left().max(left), close.y_range())
        },
        name_label: line(NAME_LABEL_ROW),
        field,
        description: line(DESCRIPTION_ROW),
        formats: std::array::from_fn(|i| formats[i]),
        divider,
        save: buttons[0],
        send: buttons[1],
        flp_audio: toggle(left, BUTTON_ROW),
        heading: heading(HEADING_ROW),
        estimate: heading(ESTIMATE_ROW),
        toggles: [
            toggle(left, TOGGLE_ROWS[0]),
            toggle(left, TOGGLE_ROWS[1]),
            toggle(right, TOGGLE_ROWS[0]),
            toggle(right, TOGGLE_ROWS[1]),
        ],
        spinners: [spinners[0], spinners[1]],
        render: row(BUTTON_ROW, BUTTON_W, 0.0, 1)[0],
        back,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE;
    use crate::project_files::PresetKind;
    use crate::theme::theme_by_id;
    use ExportFormat::*;
    use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Shape, pos2};

    const SCREEN: Rect = Rect {
        min: pos2(0.0, 0.0),
        max: pos2(1170.0, 540.0),
    };
    const U: f32 = 32.0;

    fn metrics() -> Metrics {
        Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE)
    }

    #[test]
    fn the_formats_are_in_row_order_with_their_stored_ids() {
        let buttons = ExportFormat::ALL.map(ExportFormat::button);
        assert_eq!(
            buttons,
            ["FLM", "FLP", "ZIP", "MIDI", "WAV", "FLAC", "OGG", "MP3"]
        );
        assert_eq!(
            ExportFormat::ALL.map(ExportFormat::id),
            [0, 8, 6, 1, 2, 5, 7, 3]
        );
        for f in ExportFormat::ALL {
            assert_eq!(ExportFormat::from_id(f.id()), Some(f));
        }
        assert_eq!(ExportFormat::from_id(4), None);
        assert_eq!(ExportFormat::default(), Flm);
    }

    #[test]
    fn the_formats_with_a_writer_are_selectable() {
        let available: Vec<_> = (ExportFormat::ALL.into_iter())
            .filter(|f| f.available())
            .collect();
        assert_eq!(available, ExportFormat::ALL);
    }

    #[test]
    fn each_format_has_its_description_folder_and_extension() {
        let table = [
            (Flm, FLM_DESCRIPTION, "My Songs", "flm", false),
            (Flp, "Project (.flp)", "My Songs", "zip", false),
            (Zip, "Project Archive", "My Songs", "zip", false),
            (Midi, "MIDI Sequence", "My MIDI", "mid", false),
            (Wav, "Audio Wave", "My Tracks", "wav", true),
            (Flac, "Lossless FLAC Audio", "My Tracks", "flac", true),
            (Ogg, "Compressed Audio", "My Tracks", "ogg", true),
            (Mp3, "Compressed Audio", "My Tracks", "mp3", true),
        ];
        for (f, description, folder, ext, audio) in table {
            assert_eq!(f.description(), description, "{f:?}");
            assert_eq!(f.folder(), folder, "{f:?}");
            assert_eq!(f.ext(), ext, "{f:?}");
            assert_eq!(f.is_audio(), audio, "{f:?}");
            assert!(
                !f.description().contains("Studio"),
                "{f:?} names the product"
            );
        }
    }

    #[test]
    fn the_render_options_start_off_and_only_split_resets_on_reopening() {
        let fresh = RenderPage::default();
        assert_eq!(
            (fresh.range, fresh.remainder),
            (RenderRange::Full, Remainder::Leave)
        );
        assert!(!fresh.normalize && !fresh.high_quality && !fresh.split && !fresh.master_effects);
        let last = RenderPage {
            range: RenderRange::TrimSilence,
            remainder: Remainder::Wrap,
            normalize: true,
            high_quality: true,
            split: true,
            master_effects: true,
        };
        assert_eq!(
            last.reopened(),
            RenderPage {
                split: false,
                ..last
            }
        );
        // A new dialog starts on the format row with the fresh options.
        let d = SaveDialog::new("x");
        assert_eq!((d.format, d.page, d.render), (Flm, Page::Formats, fresh));
    }

    /// Every rect of `l`, named.
    fn rects(l: &DialogLayout) -> Vec<(&'static str, Rect)> {
        let mut all = vec![
            ("band", l.band),
            ("close", l.close),
            ("title", l.title),
            ("name_label", l.name_label),
            ("field", l.field),
            ("description", l.description),
            ("save", l.save),
            ("send", l.send),
            ("heading", l.heading),
            ("estimate", l.estimate),
            ("render", l.render),
            ("back", l.back),
            ("divider", l.divider),
        ];
        all.extend(l.formats.map(|r| ("format", r)));
        all.extend(l.toggles.map(|r| ("toggle", r)));
        all.extend(l.spinners.map(|r| ("spinner", r)));
        all
    }

    #[test]
    fn the_band_is_a_20_u_floating_panel_across_the_middle_and_holds_everything() {
        let l = layout(SCREEN, &metrics());
        assert_eq!(l.band.x_range(), egui::Rangef::new(265.0, 905.0));
        assert_eq!(l.band.center().y, SCREEN.center().y);
        for (what, r) in rects(&l) {
            assert!(l.band.contains_rect(r), "{what} {r:?} is outside the band");
        }
        // Centred: the field, the format row and the button pair.
        let mid = SCREEN.center().x;
        assert_eq!(l.field.center().x, mid);
        assert_eq!((l.formats[0].left() + l.formats[7].right()) / 2.0, mid);
        assert_eq!((l.save.left() + l.send.right()) / 2.0, mid);
        // Top to bottom: field, formats, buttons; Save left of Send; the close X at the right.
        assert!(l.field.bottom() < l.formats[0].top());
        assert!(l.formats[0].bottom() < l.save.top());
        assert_eq!(l.save.top(), l.send.top());
        assert!(l.save.right() < l.send.left());
        assert!(l.close.left() > l.send.right());
        assert!(l.close.right() <= l.band.right() - 0.25 * U);
        // The formats are equal cells in one row, left to right.
        for pair in l.formats.windows(2) {
            assert_eq!(pair[0].size(), pair[1].size());
            assert_eq!(pair[0].top(), pair[1].top());
            assert!(pair[0].right() <= pair[1].left());
        }
    }

    #[test]
    fn the_render_page_has_its_title_on_top_toggles_at_the_sides_spinners_centred() {
        // The screen is never narrower than 20 U (U ≤ width / 20): the page fits there too.
        let narrow = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(20.0 * U, 360.0));
        for (ppp, screen) in [(1.0, SCREEN), (2.0, SCREEN), (1.0, narrow), (2.0, narrow)] {
            let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE);
            let l = layout(screen, &m);
            let mid = screen.center().x;
            for (what, r) in rects(&l) {
                assert!(l.band.contains_rect(r), "{what} {r:?} is outside the band");
            }
            // Title over the estimate, both centred, above everything else.
            assert_eq!(l.heading.center().x, mid);
            assert_eq!(l.estimate.center().x, mid);
            assert!(l.heading.bottom() <= l.estimate.top());
            for r in l.toggles.into_iter().chain(l.spinners) {
                assert!(
                    l.estimate.bottom() <= r.top(),
                    "{r:?} is above the estimate"
                );
            }
            // The spinners side by side in the middle; `Render` centred below them.
            let [range, remainder] = l.spinners;
            assert_eq!((range.left() + remainder.right()) / 2.0, mid);
            assert!(range.right() < remainder.left());
            assert_eq!(range.size(), remainder.size());
            assert_eq!(l.render.center().x, mid);
            assert!(range.bottom() < l.render.top());
            // Normalize over High quality on the left, Split over Enable master FX on the right.
            let [normalize, hq, split, master] = l.toggles;
            assert!(normalize.right() < range.left() && hq.right() < range.left());
            assert!(split.left() > remainder.right() && master.left() > remainder.right());
            assert_eq!((normalize.left(), split.left()), (hq.left(), master.left()));
            assert!(normalize.bottom() <= hq.top() && split.bottom() <= master.top());
            assert_eq!((normalize.top(), hq.top()), (split.top(), master.top()));
            // Back: an equal circle left of the close X, level with it.
            assert_eq!(l.back.size(), l.close.size());
            assert_eq!(l.back.center().y, l.close.center().y);
            assert!(l.back.right() < l.close.left());
            assert!(l.heading.right() < l.back.left());
            // Whole device pixels.
            for (what, r) in rects(&l) {
                for v in [r.min.x, r.min.y, r.max.x, r.max.y] {
                    assert_eq!((v * ppp).fract(), 0.0, "{what} {r:?} at ppp {ppp}");
                }
            }
        }
    }

    struct Rig {
        ctx: egui::Context,
        dialog: SaveDialog,
        time: f64,
        screen: Rect,
    }

    impl Rig {
        fn new() -> Self {
            Self::with(SaveDialog::new("New Song"))
        }

        fn with(dialog: SaveDialog) -> Self {
            let mut rig = Self {
                ctx: egui::Context::default(),
                dialog,
                time: 0.0,
                screen: SCREEN,
            };
            for _ in 0..2 {
                rig.frame(vec![]);
            }
            rig
        }

        fn run(&mut self, events: Vec<Event>) -> (Option<DialogAction>, Vec<Shape>) {
            self.time += 0.1;
            let raw = RawInput {
                screen_rect: Some(self.screen),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let (theme, m) = (theme_by_id(0), metrics());
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            let mut action = None;
            let output = self.ctx.run(raw, |ctx| action = self.dialog.show(ctx, &cx));
            (action, output.shapes.into_iter().map(|s| s.shape).collect())
        }

        fn frame(&mut self, events: Vec<Event>) -> Option<DialogAction> {
            self.run(events).0
        }

        fn click(&mut self, pos: Pos2) -> Option<DialogAction> {
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![button(true)])
                .or(self.frame(vec![button(false)]))
        }

        fn key(&mut self, key: Key) -> Option<DialogAction> {
            self.frame(crate::ui::offscreen::key_tap(key, Modifiers::NONE))
        }

        /// The strings drawn this frame.
        fn drawn(&mut self) -> Vec<String> {
            let (_, shapes) = self.run(vec![]);
            texts(&shapes).into_iter().map(|(t, ..)| t).collect()
        }

        /// Picks `format` on the row and presses `SAVE`.
        fn save_as(&mut self, format: ExportFormat) -> Option<DialogAction> {
            let l = layout(self.screen, &metrics());
            let i = ExportFormat::ALL.iter().position(|&f| f == format).unwrap();
            assert_eq!(self.click(l.formats[i].center()), None);
            self.click(l.save.center())
        }
    }

    #[test]
    fn save_and_the_close_x_act_and_send_does_nothing() {
        let l = layout(SCREEN, &metrics());
        let mut rig = Rig::new();
        assert_eq!(rig.click(l.send.center()), None);
        assert_eq!(rig.click(l.save.center()), Some(DialogAction::Save(Flm)));
        assert_eq!(rig.click(l.close.center()), Some(DialogAction::Close));
        // The band and the backdrop swallow clicks without closing.
        assert_eq!(rig.click(pos2(5.0, 5.0)), None);
        assert_eq!(rig.click(l.band.left_center() + egui::vec2(5.0, 0.0)), None);
    }

    /// Which format tabs `shapes` underline: a `text` line 2t thick along the cell's bottom.
    fn underlined(shapes: &[Shape], l: &DialogLayout) -> Vec<bool> {
        let (m, text) = (metrics(), theme_by_id(0).colors.text.to_color32());
        (l.formats.iter())
            .map(|cell| {
                let line = cell.with_min_y(cell.bottom() - 2.0 * m.t());
                (shapes.iter())
                    .any(|s| matches!(s, Shape::Rect(r) if r.rect == line && r.fill == text))
            })
            .collect()
    }

    #[test]
    fn a_picked_format_is_underlined_and_described_and_save_saves_a_project_format() {
        let l = layout(SCREEN, &metrics());
        for (i, format) in [(2, Zip), (3, Midi), (0, Flm)] {
            let mut rig = Rig::new();
            assert_eq!(rig.click(l.formats[i].center()), None);
            assert_eq!(rig.dialog.format, format);
            let (_, shapes) = rig.run(vec![egui::Event::PointerMoved(pos2(5.0, 5.0))]);
            let underlined = underlined(&shapes, &l);
            let at = underlined.iter().position(|&u| u);
            assert_eq!(at, Some(i), "{format:?}: {underlined:?}");
            assert_eq!(underlined.iter().filter(|&&u| u).count(), 1);
            assert!(
                texts(&shapes)
                    .iter()
                    .any(|(t, ..)| t == format.description())
            );
            assert_eq!(rig.click(l.save.center()), Some(DialogAction::Save(format)));
            assert_eq!(rig.dialog.page, Page::Formats);
        }
    }

    #[test]
    fn save_with_an_audio_format_opens_the_render_page_and_back_returns() {
        let l = layout(SCREEN, &metrics());
        let mut rig = Rig::new();
        rig.dialog.song_secs = 2.7;
        assert_eq!(rig.save_as(Wav), None);
        assert_eq!(rig.dialog.page, Page::Render);
        let drawn = rig.drawn();
        for s in [
            "New Song.wav",
            "Estimated length: 0m 2s",
            "Normalize level",
            "High quality",
            "Split mixer tracks",
            "Full song",
            "Leave remainder",
            "Range",
            "Tail",
            "Render",
        ] {
            assert!(drawn.iter().any(|t| t == s), "{s} is not drawn: {drawn:?}");
        }
        for gone in [
            "Enable master FX",
            "Save",
            "Send",
            "File Name",
            "WAV",
            "Audio Wave",
        ] {
            assert!(!drawn.iter().any(|t| t == gone), "{gone} is drawn");
        }
        // The format row's places do nothing here.
        assert_eq!(rig.click(l.save.center()), None);
        assert_eq!(rig.click(l.formats[0].center()), None);
        assert_eq!((rig.dialog.format, rig.dialog.page), (Wav, Page::Render));
        assert_eq!(rig.click(l.back.center()), None);
        assert_eq!(rig.dialog.page, Page::Formats);
        assert!(rig.drawn().iter().any(|t| t == "Save"));
        // Enter in the name field does what Save does.
        rig.save_as(Flac);
        rig.click(l.back.center());
        assert_eq!(rig.dialog.page, Page::Formats);
        // The field takes the focus back in the frame after the page switch.
        rig.frame(vec![]);
        assert_eq!(rig.key(Key::Enter), None);
        assert_eq!(rig.dialog.page, Page::Render);
        assert!(rig.drawn().iter().any(|t| t == "New Song.flac"));
    }

    #[test]
    fn render_returns_the_toggled_options_and_the_close_x_and_escape_close() {
        let l = layout(SCREEN, &metrics());
        let mut rig = Rig::new();
        rig.save_as(Wav);
        assert_eq!(rig.click(l.toggles[0].center()), None);
        assert_eq!(rig.click(l.toggles[1].center()), None);
        let expected = RenderPage {
            normalize: true,
            high_quality: true,
            ..RenderPage::default()
        };
        assert_eq!(rig.dialog.render, expected);
        assert_eq!(
            rig.click(l.render.center()),
            Some(DialogAction::Render(expected))
        );
        assert_eq!(rig.click(l.toggles[1].center()), None);
        assert!(!rig.dialog.render.high_quality);
        assert_eq!(rig.click(l.close.center()), Some(DialogAction::Close));
        assert_eq!(rig.key(Key::Escape), Some(DialogAction::Close));
    }

    #[test]
    fn split_titles_the_render_page_with_the_stems_folder() {
        let l = layout(SCREEN, &metrics());
        let mut rig = Rig::new();
        rig.save_as(Flac);
        assert!(rig.drawn().iter().any(|t| t == "New Song.flac"));
        rig.click(l.toggles[2].center());
        assert!(
            rig.drawn()
                .iter()
                .any(|t| t == "New Song - Multitrack flac")
        );
        rig.click(l.toggles[2].center());
        assert!(rig.drawn().iter().any(|t| t == "New Song.flac"));
    }

    #[test]
    fn enable_master_effects_shows_only_while_split_is_on() {
        let l = layout(SCREEN, &metrics());
        let mut rig = Rig::new();
        rig.save_as(Flac);
        // Hidden: its place takes no click.
        assert_eq!(rig.click(l.toggles[3].center()), None);
        assert!(!rig.dialog.render.master_effects);
        rig.click(l.toggles[2].center());
        assert!(rig.dialog.render.split);
        assert!(rig.drawn().iter().any(|t| t == "Enable master FX"));
        rig.click(l.toggles[3].center());
        assert!(rig.dialog.render.master_effects);
        rig.click(l.toggles[2].center());
        assert!(!rig.dialog.render.split);
        assert!(!rig.drawn().iter().any(|t| t == "Enable master FX"));
        // It keeps its state while hidden, but `Render` reports it off: it only acts with Split on.
        assert!(rig.dialog.render.master_effects);
        let Some(DialogAction::Render(page)) = rig.click(l.render.center()) else {
            panic!("no render");
        };
        assert!(!page.split && !page.master_effects);
        assert!(rig.dialog.render.master_effects);
        rig.click(l.toggles[2].center());
        let Some(DialogAction::Render(page)) = rig.click(l.render.center()) else {
            panic!("no render");
        };
        assert!(page.split && page.master_effects);
    }

    #[test]
    fn escape_with_a_spinner_popup_open_closes_only_the_popup() {
        let l = layout(SCREEN, &metrics());
        let mut rig = Rig::new();
        rig.save_as(Wav);
        // A tap on the range spinner's value opens its item popup.
        assert_eq!(rig.click(l.spinners[0].center()), None);
        assert!(rig.drawn().iter().any(|t| t == "Trim initial silence"));
        assert_eq!(rig.key(Key::Escape), None);
        assert!(!rig.drawn().iter().any(|t| t == "Trim initial silence"));
        assert_eq!(rig.dialog.page, Page::Render);
        assert_eq!(rig.key(Key::Escape), Some(DialogAction::Close));
    }

    #[test]
    fn a_long_name_keeps_its_extension_in_the_render_title() {
        // 20 U wide, the narrowest screen: the title line is 13 U.
        let screen = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(20.0 * U, 540.0));
        for format in [Wav, Flac] {
            let mut rig = Rig::with(SaveDialog {
                format,
                page: Page::Render,
                ..SaveDialog::new(
                    "A song with a name far too long for the title line of the band ".repeat(2),
                )
            });
            rig.screen = screen;
            rig.frame(vec![]);
            let drawn = rig.drawn();
            let ext = format!(".{}", format.ext());
            let title = (drawn.iter())
                .find(|t| t.starts_with("A song"))
                .expect("no title");
            assert!(title.contains("...") && title.ends_with(&ext), "{title}");
        }
    }

    /// The tap points of a spinner's `^` and `v` chevrons.
    fn chevrons(spinner: Rect) -> (Pos2, Pos2) {
        let c = spinner.center();
        (pos2(c.x, c.y - 0.85 * U), pos2(c.x, c.y + 0.85 * U))
    }

    #[test]
    fn the_spinners_step_through_their_items_and_skip_a_missing_selection() {
        let l = layout(SCREEN, &metrics());
        let ((range_up, range_down), (rem_up, rem_down)) =
            (chevrons(l.spinners[0]), chevrons(l.spinners[1]));
        let mut rig = Rig::new();
        rig.save_as(Wav);
        let range = |rig: &Rig| rig.dialog.render.range;
        rig.click(range_down);
        assert_eq!(range(&rig), RenderRange::TrimSilence);
        assert!(rig.drawn().iter().any(|t| t == "Trim initial silence"));
        // No selection: `Selection` is skipped, the end stays.
        rig.click(range_down);
        assert_eq!(range(&rig), RenderRange::TrimSilence);
        rig.click(range_up);
        assert_eq!(range(&rig), RenderRange::Full);
        // With one, it is the last item, and the estimate is its length.
        rig.dialog.song_secs = 125.0;
        rig.dialog.selection_secs = Some(3.5);
        assert!(rig.drawn().iter().any(|t| t == "Estimated length: 2m 5s"));
        rig.click(range_down);
        rig.click(range_down);
        assert_eq!(range(&rig), RenderRange::Selection);
        let drawn = rig.drawn();
        assert!(drawn.iter().any(|t| t == "Selection"));
        assert!(drawn.iter().any(|t| t == "Estimated length: 0m 3s"));

        let remainder = |rig: &Rig| rig.dialog.render.remainder;
        for expected in [Remainder::Cut, Remainder::Wrap, Remainder::Wrap] {
            rig.click(rem_down);
            assert_eq!(remainder(&rig), expected);
        }
        assert!(rig.drawn().iter().any(|t| t == "Wrap remainder"));
        rig.click(rem_up);
        assert_eq!(remainder(&rig), Remainder::Cut);
    }

    #[test]
    fn a_kept_selection_range_without_a_selection_reads_as_full_song() {
        let mut rig = Rig::with(SaveDialog {
            format: Wav,
            page: Page::Render,
            render: RenderPage {
                range: RenderRange::Selection,
                ..RenderPage::default()
            },
            ..SaveDialog::new("A")
        });
        let drawn = rig.drawn();
        assert!(drawn.iter().any(|t| t == "Full song"), "{drawn:?}");
        assert_eq!(rig.dialog.render.range, RenderRange::Full);
    }

    #[test]
    fn save_as_midi_has_one_format_its_title_and_saves_midi() {
        let l = layout(SCREEN, &metrics());
        let mut rig = Rig::with(SaveDialog::save_as_midi("Bass"));
        let (_, shapes) = rig.run(vec![]);
        let texts = texts(&shapes);
        let drawn = |s: &str| texts.iter().find(|(t, ..)| t == s);
        let (_, at, _) = drawn("Save as MIDI").expect("no title");
        assert_eq!(at.x, l.title.left());
        assert!(drawn("MIDI Sequence").is_some());
        for gone in FORMATS {
            assert!(drawn(gone).is_none(), "{gone} is drawn");
        }
        assert_eq!(rig.click(l.save.center()), Some(DialogAction::Save(Midi)));
    }

    #[test]
    fn escape_closes() {
        let mut rig = Rig::new();
        assert_eq!(rig.key(Key::Escape), Some(DialogAction::Close));
    }

    #[test]
    fn the_name_field_has_the_focus_takes_typing_and_enter_saves() {
        let mut rig = Rig::new();
        // Select all, then type over it.
        rig.frame(vec![Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }]);
        assert_eq!(rig.frame(vec![Event::Text("Groove".into())]), None);
        assert_eq!(rig.dialog.name, "Groove");
        assert_eq!(rig.key(Key::Enter), Some(DialogAction::Save(Flm)));
    }

    /// Every text the dialog drew: its string, where it starts and its size.
    fn texts(shapes: &[Shape]) -> Vec<(String, Pos2, egui::Vec2)> {
        (shapes.iter())
            .filter_map(|s| match s {
                Shape::Text(t) => Some((t.galley.text().to_owned(), t.pos, t.galley.size())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_preset_dialog_has_its_title_left_of_the_close_x_its_file_type_and_no_formats() {
        let l = layout(SCREEN, &metrics());
        for kind in [PresetKind::Module, PresetKind::Rack] {
            let mut rig = Rig::new();
            rig.dialog.preset = Some(kind);
            let (_, shapes) = rig.run(vec![]);
            let texts = texts(&shapes);
            let drawn = |s: &str| texts.iter().find(|(t, ..)| t == s);
            let (_, at, size) = drawn(kind.title()).expect("no title");
            assert_eq!(at.x, l.title.left());
            assert!((at.y + size.y / 2.0 - l.close.center().y).abs() < 0.5);
            assert!(
                at.x + size.x <= l.field.left(),
                "the title runs into the field"
            );
            assert!(drawn(kind.format()).is_some(), "no file type");
            for gone in FORMATS.into_iter().chain([FLM_DESCRIPTION]) {
                assert!(drawn(gone).is_none(), "{gone} is drawn");
            }
            assert!(drawn("File Name").is_some() && drawn("Save").is_some());
            assert_eq!(rig.click(l.save.center()), Some(DialogAction::Save(Flm)));
        }
        // The song's dialog keeps its look: no title.
        let (_, shapes) = Rig::new().run(vec![]);
        let texts = texts(&shapes);
        assert!(texts.iter().any(|(t, ..)| t == FLM_DESCRIPTION));
        assert!(!texts.iter().any(|(t, ..)| t.starts_with("Save ")));
    }

    #[test]
    fn a_title_wider_than_its_room_is_shortened_before_the_field() {
        // 15 U wide: 2 U between the band's inset and the field.
        let screen = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(15.0 * U, 540.0));
        let l = layout(screen, &metrics());
        let mut rig = Rig::new();
        rig.screen = screen;
        rig.dialog.preset = Some(PresetKind::Rack);
        rig.frame(vec![]);
        let (_, shapes) = rig.run(vec![]);
        let texts = texts(&shapes);
        let (title, at, size) = (texts.iter())
            .find(|(t, ..)| t.starts_with("Sav") && t != "Save")
            .expect("no title");
        // Shrunk to 6 pt, then shortened in the middle.
        assert!(title.contains("...") && title.ends_with("eset"), "{title}");
        assert!(
            at.x + size.x <= l.field.left(),
            "{title} runs into the field"
        );
    }

    #[test]
    fn only_flm_is_underlined_and_send_is_dimmed() {
        let l = layout(SCREEN, &metrics());
        let colors = theme_by_id(0).colors;
        let text = colors.text.to_color32();
        let (_, shapes) = Rig::new().run(vec![]);
        let underlined = underlined(&shapes, &l);
        assert_eq!(
            underlined.iter().position(|&u| u),
            Some(0),
            "{underlined:?}"
        );
        assert_eq!(underlined.iter().filter(|&&u| u).count(), 1);
        let rect_fill = |r: Rect| {
            shapes.iter().find_map(|s| match s {
                Shape::Rect(rs) if rs.rect == r => Some(rs.fill),
                _ => None,
            })
        };
        assert_eq!(rect_fill(l.save), Some(text));
        assert_eq!(rect_fill(l.send), Some(text.gamma_multiply(0.3)));
        assert_eq!(rect_fill(l.band), Some(colors.menu_fill.to_color32()));
        assert_eq!(rect_fill(l.field), Some(colors.background.to_color32()));
    }

    /// The dialog base: screen dim α 0.6, then the floating panel, from the first frame, on both pages and for the
    /// preset, sample-name and `Save as MIDI` forms (one painter).
    #[test]
    fn the_dialog_base_dims_the_screen_then_fills_the_band_with_module_fill() {
        let l = layout(SCREEN, &metrics());
        let colors = theme_by_id(0).colors;
        let base = |rig: &mut Rig| {
            let (_, shapes) = rig.run(vec![]);
            crate::ui::dialog::assert_base(
                &crate::ui::dialog::painted_rects(&shapes),
                &colors,
                SCREEN,
                l.band,
            );
        };
        base(&mut Rig::new());
        let mut render = Rig::new();
        render.dialog.page = Page::Render;
        base(&mut render);
        base(&mut Rig::with(SaveDialog {
            preset: Some(PresetKind::Rack),
            ..SaveDialog::new("Rack")
        }));
        base(&mut Rig::with(SaveDialog::save_as_midi("Song")));
        // In the very first frame.
        base(&mut Rig {
            ctx: egui::Context::default(),
            dialog: SaveDialog::new("New Song"),
            time: 0.0,
            screen: SCREEN,
        });
    }
}

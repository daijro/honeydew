//! The browsers' search bar: one [`SearchBar`] in each browser that offers `Search`, the SONGS page
//! (`main_menu/songs.rs`) and the side browser panel of every picker (`modules_browser.rs`'s `SidePanel`). There is no
//! SHOP page, so no product browser has a bar.
//!
//! The browser keeps the rest: its `Search` button, lit while [`SearchBar::shown`]; select mode, which it leaves on
//! [`SearchBar::start`] and [`SearchBar::end`]; and Escape's order, bar → select mode → close.
//!
//! A non-empty query lists everything below the open folder, recursively: [`Walk`] gathers the folders and files below
//! it once, [`matching`] keeps those whose name holds the query without case, and each shows in a [`FOUND_ROW_H`] row
//! with its folder's path from the browser's root under the name ([`place_line`]). A found folder opens through the
//! folders above it ([`Found::folders`]), and the search ends. Both browsers use these.
//!
//! - A `Search` tap that shows the bar starts the search as Ctrl+F does (the field takes the keyboard, select mode is
//!   left).
//! - The caret's 60-frame cycle counts the frames drawn since the field took the keyboard: on for 30, off for 30. A
//!   hardware keyboard cannot be told from here, so it shows whenever the field has the keyboard.
//! - `Clear`'s label is 10 pt, as the main menu's light buttons; the selection band is `text` at the placeholder's
//!   alpha.
//! - egui's own extra field keys (Home, End, word steps, Ctrl+A/C/X/Z) stay.
//! - The results come depth first in the browser's own order, each folder followed by what is below it. The path's
//!   folders are joined with `/`; a row of the browser's root has an empty path.
//! - The walk skips the library's bin ([`crate::ui::main_menu::SongLibrary::trash`], by its path): deleted files are no results. A folder of
//!   the user's that is only named `Recently Deleted` is searched; searched inside the bin, its files are.
//! - The walk runs on the UI thread, once per open folder (the first non-empty query), bounded as the listings are:
//!   at most [`MAX_LISTED`] rows seen, files and folders, and [`WALK_DEPTH`] folders deep.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use eframe::egui::{
    Align, Align2, Color32, Context, Id, Painter, Rect, RichText, Stroke, TextEdit, Ui, vec2,
};

use crate::keymap;
use crate::metrics::{FontSize, Metrics};
use crate::ui::UiCtx;
use crate::ui::focus::{self, Dir, Kind};
use crate::ui::main_menu::SongEntry;
use crate::ui::main_menu::library::MAX_LISTED;
use crate::ui::widgets::{Button, button, truncated};

/// How far the bar moves the breadcrumbs and the list down while it shows, in U.
pub(crate) const DROP: f32 = 1.75;
/// The bar's inset from the browser's left edge (past the 1.5 U button column at x + 0.25 U) and top, the width it
/// leaves out, and its height, in U.
const LEFT: f32 = 1.75;
const TOP: f32 = 0.25;
const SHORT: f32 = 2.0;
const H: f32 = 1.5;
/// `Clear`, at the bar's right end.
const CLEAR_W: f32 = 2.0;
/// The field's text margins, left and right, top and bottom.
const MARGIN_X: f32 = 0.5;
const MARGIN_Y: f32 = 0.25;
/// The empty field's `Search` placeholder: `text` at this alpha.
const HINT_ALPHA: f32 = 0.25;
/// The caret's blink: a cycle of this many drawn frames, shown for its first half.
const BLINK_FRAMES: u32 = 60;
/// The longest text, in characters (as SONGS' name fields).
const TEXT_LIMIT: usize = 255;
/// A recursive result's row height, in U, and its path line: its top below the row's, its height, its alpha.
pub(crate) const FOUND_ROW_H: f32 = 2.2;
const PLACE_TOP: f32 = 1.4;
const PLACE_H: f32 = 0.6;
const PLACE_ALPHA: f32 = 0.6;
/// The tail a too-long path keeps after its middle ellipsis (the browsers' titles').
const PLACE_TAIL_K: usize = 5;
/// How deep the walk goes below the open folder.
const WALK_DEPTH: usize = 16;

/// A folder or file the search found below the open folder: its folder's path from the browser's root, and the folders
/// from the open one down to it, which a found folder opens through ([`Found::folders`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub entry: SongEntry,
    pub place: String,
    /// The folders between the open folder and `entry`, outermost first; shared by the rows of one folder.
    trail: Rc<Vec<SongEntry>>,
}

impl Found {
    /// The folders a tap on a found folder opens, from the open folder's first child down to the folder itself.
    pub(crate) fn folders(&self) -> Vec<SongEntry> {
        let mut folders = (*self.trail).clone();
        folders.push(self.entry.clone());
        folders
    }
}

/// The folders and files below a folder, gathered for the search ([`Walk::add`]): at most [`MAX_LISTED`] rows seen,
/// [`WALK_DEPTH`] folders deep, the library's bin left out.
#[derive(Default)]
pub(crate) struct Walk {
    pub found: Vec<Found>,
    seen: usize,
    /// The library's bin ([`crate::ui::main_menu::SongLibrary::trash`]): deleted files are no results.
    bin: Option<PathBuf>,
}

impl Walk {
    /// A walk that leaves out `bin`, the library's [`crate::ui::main_menu::SongLibrary::trash`].
    pub(crate) fn skipping(bin: Option<PathBuf>) -> Self {
        Self {
            bin,
            ..Self::default()
        }
    }

    /// Adds `rows`, at `place`, each folder followed by what is below it: `list` gives a folder's rows in the
    /// browser's order.
    pub(crate) fn add(
        &mut self,
        list: &dyn Fn(&Path) -> Vec<SongEntry>,
        rows: Vec<SongEntry>,
        place: &str,
    ) {
        self.add_at(list, rows, place, &Rc::default());
    }

    fn add_at(
        &mut self,
        list: &dyn Fn(&Path) -> Vec<SongEntry>,
        rows: Vec<SongEntry>,
        place: &str,
        trail: &Rc<Vec<SongEntry>>,
    ) {
        for entry in rows {
            if self.seen >= MAX_LISTED {
                return;
            }
            if entry.is_folder && self.bin.as_ref() == Some(&entry.path) {
                continue;
            }
            self.seen += 1;
            let below = (entry.is_folder && trail.len() < WALK_DEPTH).then(|| {
                let place = match place {
                    "" => entry.name.clone(),
                    _ => format!("{place}/{}", entry.name),
                };
                let mut deeper = (**trail).clone();
                deeper.push(entry.clone());
                (place, Rc::new(deeper), list(&entry.path))
            });
            self.found.push(Found {
                entry,
                place: place.to_owned(),
                trail: trail.clone(),
            });
            if let Some((place, trail, rows)) = below {
                self.add_at(list, rows, &place, &trail);
            }
        }
    }
}

/// The rows of `found` whose name holds `query`, ignoring case, in their order.
pub(crate) fn matching<'a>(found: &'a [Found], query: &str) -> Vec<&'a Found> {
    let query = query.to_lowercase();
    (found.iter())
        .filter(|f| f.entry.name.to_lowercase().contains(&query))
        .collect()
}

/// Draws a result's folder path under its name, in FontSize 1 at α 0.6 of the row's `color`: `label` is the row's
/// text rect, which it leaves to the name, the row's top [`PLACE_TOP`] U.
pub(crate) fn place_line(
    painter: &Painter,
    m: &Metrics,
    label: &mut Rect,
    place: &str,
    color: Color32,
) {
    let top = label.top();
    let line = label.with_min_y(top + m.u(PLACE_TOP));
    let line = line.with_max_y(line.top() + m.u(PLACE_H));
    let (font, dim) = (m.font_id(FontSize::Pt8), color.gamma_multiply(PLACE_ALPHA));
    truncated(
        painter,
        line,
        Align2::LEFT_CENTER,
        place,
        font,
        dim,
        PLACE_TAIL_K,
    );
    label.max.y = top + m.u(PLACE_TOP);
}

/// One browser's search: the bar, its field's text and the query the list filters by.
#[derive(Default)]
pub(crate) struct SearchBar {
    shown: bool,
    /// The field's text. It stays while the bar hides, so showing it again brings its filter back.
    text: String,
    /// The list's filter: the text trimmed, from the frame after an edit; empty while the bar hides.
    query: String,
    /// The field takes the keyboard as it draws next.
    focus: bool,
    /// The field was edited: the query follows on the next frame.
    edited: bool,
    /// The keyboard focus was on the field last frame.
    on_field: bool,
    /// Frames drawn since the field took the keyboard: the caret's blink.
    blink: u32,
}

impl SearchBar {
    /// The bar shows, and `Search` is lit.
    pub(crate) fn shown(&self) -> bool {
        self.shown
    }

    /// The list's filter: empty for the normal folder listing.
    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    /// `startSearch`: a hidden bar shows with its field's text as the query; either way the field takes the keyboard.
    /// The browser leaves select mode and refreshes its list.
    pub(crate) fn start(&mut self) {
        if !self.shown {
            self.shown = true;
            self.query = self.text.trim().to_owned();
        }
        self.focus = true;
    }

    /// Escape, or a tap on a lit `Search`: the bar hides and the query clears; the field keeps its text. The browser
    /// leaves select mode.
    pub(crate) fn end(&mut self) {
        self.shown = false;
        self.query.clear();
        self.edited = false;
    }

    /// `startSearch` (Ctrl+F; Cmd+F on a Mac) for the browser in focus container `browser`, whose field is `field`: it
    /// is taken only while the keyboard focus is inside the browser, and then [`Self::start`]s with the focus on the
    /// field. Returns whether it did.
    pub(crate) fn start_key(&mut self, ctx: &Context, browser: Id, field: Id) -> bool {
        let within = focus::within(ctx, browser) || ctx.memory(|m| m.has_focus(field));
        if !(within && keymap::pressed(ctx, "startSearch")) {
            return false;
        }
        self.start();
        focus::focus(ctx, field);
        true
    }

    /// The bar in browser rect `browser` (x, y, w): (x + 1.75 U, y + 0.25 U, w − 2 U, 1.5 U).
    pub(crate) fn rect(browser: Rect, m: &Metrics) -> Rect {
        let size = vec2(browser.width() - m.u(SHORT), m.u(H));
        Rect::from_min_size(browser.min + vec2(m.u(LEFT), m.u(TOP)), size)
    }

    /// Draws the shown bar for browser rect `browser`: the `menuFill` band, the field `field` (a focus target of the
    /// browser) and `Clear` (`field.with("clear")`). Returns whether the query changed this frame (last frame's edit):
    /// the browser refreshes its list.
    pub(crate) fn show(&mut self, ui: &mut Ui, cx: &UiCtx, field: Id, browser: Rect) -> bool {
        let refresh = std::mem::take(&mut self.edited);
        if refresh {
            self.query = self.text.trim().to_owned();
        }
        let (c, m) = (&cx.theme.colors, cx.m);
        let text = c.text.to_color32();
        let bar = Self::rect(browser, m);
        ui.painter().rect_filled(bar, 0.0, c.menu_fill.to_color32());
        let (rect, clear) = bar.split_left_right_at_x(bar.right() - m.u(CLEAR_W));
        let ctx = ui.ctx().clone();
        // A target of the browser's focus, with no `open` of its own (Enter does nothing).
        focus::register(ui, field, rect, Kind::Container);
        // Up and Down are no keys of the field: they move the focus out of it.
        if ctx.memory(|mem| mem.has_focus(field)) {
            const OUT: [(&str, Dir); 2] = [("focusUp", Dir::Up), ("focusDown", Dir::Down)];
            let keys = keymap::for_os(ctx.os());
            let out = ctx.input_mut(|i| {
                (OUT.iter())
                    .find(|(action, _)| keys.consume_press(i, action))
                    .map(|&(_, dir)| dir)
            });
            if let Some(dir) = out {
                ctx.memory_mut(|mem| mem.surrender_focus(field));
                focus::move_dir(&ctx, dir);
            }
        }
        // The keyboard focus arriving on the field (Tab, an arrow) gives it the typing focus; the field taking it moves
        // the keyboard focus there.
        let typing = ctx.memory(|mem| mem.has_focus(field));
        let on = focus::focused(&ctx) == Some(field);
        let arrived = on && !std::mem::replace(&mut self.on_field, on);
        let take = std::mem::take(&mut self.focus) || arrived && !typing;
        if take {
            self.blink = 0;
            focus::move_to(&ctx, field);
        }
        let caret = typing && self.blink % BLINK_FRAMES < BLINK_FRAMES / 2;
        if typing {
            self.blink += 1;
            ctx.request_repaint();
        }
        let hint = RichText::new("Search").color(text.gamma_multiply(HINT_ALPHA));
        let edit = TextEdit::singleline(&mut self.text)
            .id(field)
            .char_limit(TEXT_LIMIT)
            .frame(false)
            .font(m.font_id(FontSize::Pt12))
            .text_color(text)
            .hint_text(hint)
            .margin(vec2(m.u(MARGIN_X), m.u(MARGIN_Y)))
            .desired_width(rect.width())
            .min_size(rect.size())
            .vertical_align(Align::Center)
            // Enter does nothing: there is no commit step.
            .return_key(None);
        let response = ui
            .scope(|ui| {
                let v = ui.visuals_mut();
                let ink = if caret { text } else { Color32::TRANSPARENT };
                v.text_cursor.stroke = Stroke::new(1.0 / m.ppp(), ink);
                v.text_cursor.blink = false;
                v.selection.bg_fill = text.gamma_multiply(HINT_ALPHA);
                v.selection.stroke.color = text;
                ui.put(rect, edit)
            })
            .inner;
        if take {
            response.request_focus();
        }
        let mut edited = response.changed();
        let clear = Button::new(field.with("clear"), clear, "Clear", false, c);
        if button(ui, cx, &clear, true).clicked() {
            self.text.clear();
            edited = true;
            // The field keeps the keyboard: typing goes on into it from the next frame.
            response.request_focus();
            self.blink = 0;
        }
        if edited {
            self.edited = true;
            ctx.request_repaint();
        }
        refresh
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn entry(path: &str, is_folder: bool) -> SongEntry {
        let path = PathBuf::from(path);
        SongEntry {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path,
            is_folder,
            recency: 0,
        }
    }

    #[test]
    fn the_walk_stops_after_max_listed_rows_folders_included() {
        // One folder holding MAX_LISTED files: the folder is the first row seen, so one file is left out.
        let files = |_: &Path| -> Vec<SongEntry> {
            (0..MAX_LISTED)
                .map(|i| entry(&format!("F/{i}.flm"), false))
                .collect()
        };
        let mut walk = Walk::default();
        walk.add(&files, vec![entry("F", true)], "");
        assert_eq!(walk.found.len(), MAX_LISTED);
        assert!(walk.found[0].entry.is_folder);
        assert_eq!(
            walk.found.last().unwrap().entry.name,
            format!("{}.flm", MAX_LISTED - 2)
        );
    }
}

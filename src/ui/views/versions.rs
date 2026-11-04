//! The project versions window (#69): the song's branches and their versions ([`crate::versions`]), on the dialog base.
//! PROJECT `View project versions` opens it, and so does the Save dialog's `Save project version` once it saved the song
//! with a new version.
//!
//! A floating panel min(20 U, the window) wide and min(14 U, the window less 0.5 U above and below) tall, padded 0.5 U:
//! `Project versions` (12 pt) at the top left and the close X at the top right; then the list, each branch followed by its
//! versions, newest first, in rows 1 U tall that scroll; then the `Name` and `Note` fields, which edit the picked row (a
//! branch has no note); then the buttons `Save version`, `Restore`, `New branch` and `Delete`. A branch row shows its
//! name, and dimmed at its right `current` (the branch a save goes into), its count of versions and their size; a
//! version row, 0.75 U in, its name with its note dimmed after it, and its time dimmed at its right. A tap picks a row,
//! which is lit with `text` at α 0.1.
//!
//! - `Save version` saves the song as a new version into the picked row's branch, else the current one (the first one
//!   makes `main`), and picks it.
//! - `Restore` opens the picked version as the song, as one undo step (`SongEdit::Restored`).
//! - `New branch` starts a branch from the picked version, or from the picked branch's newest.
//! - `Delete` asks, then deletes the picked version or branch.
//!
//! The close X and Escape close it. The versions are the file's, not the song's edits: no undo step, and every change but
//! `Restore` (an edit) sets the song-changed flag.

use std::sync::Arc;

use eframe::egui::{self, Align2, Area, Id, Order, Rect, Sense, TextEdit, pos2, vec2};

use super::{AppCtx, Nav, Overlay};
use crate::metrics::{FontSize, Metrics};
use crate::song::Song;
use crate::theme::DISABLED_ALPHA;
use crate::ui::buttons::close_x;
use crate::ui::main_menu::{meta_time, scroll, text_field};
use crate::ui::scroller::Scroller;
use crate::ui::widgets::{button, truncated};
use crate::versions::{self, Versions};

/// `Delete`'s question.
pub const DELETE_TAG: &str = "versions_delete";
/// A save or restore that failed.
pub const ERROR_TAG: &str = "versions_error";

// Sizes in U.
const MAX_H: f32 = 14.0;
const MARGIN: f32 = 0.5;
const PAD: f32 = 0.5;
const TITLE_H: f32 = 1.25;
const ROW_H: f32 = 1.0;
const INDENT: f32 = 0.75;
const LABEL_H: f32 = 0.5;
const FIELD_H: f32 = 1.0;
const BUTTON_H: f32 = 1.25;
const GAP: f32 = 0.25;
const PICKED_ALPHA: f32 = 0.1;
const DIM_ALPHA: f32 = 0.6;
/// The fields' lengths, in characters.
const NAME_LIMIT: usize = 255;
const NOTE_LIMIT: usize = 1023;
const BUTTONS: [&str; 4] = ["Save version", "Restore", "New branch", "Delete"];

/// A row of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pick {
    Branch(usize),
    Version(usize, usize),
}

impl Pick {
    fn branch(self) -> usize {
        match self {
            Pick::Branch(b) | Pick::Version(b, _) => b,
        }
    }

    /// The version it is, or a branch's newest.
    fn version(self, versions: &Versions) -> Option<(usize, usize)> {
        let at = match self {
            Pick::Version(b, v) => (b, v),
            Pick::Branch(b) => (b, versions.branches.get(b)?.versions.len().checked_sub(1)?),
        };
        versions.get(at).map(|_| at)
    }

    fn exists(self, versions: &Versions) -> bool {
        match self {
            Pick::Branch(b) => b < versions.branches.len(),
            Pick::Version(b, v) => versions.get((b, v)).is_some(),
        }
    }
}

/// What the window keeps while the app runs.
#[derive(Default)]
pub struct State {
    pick: Option<Pick>,
    scroll: f32,
    kinetic: Scroller,
    /// What `Delete` asked about, until the answer.
    asked: Option<Pick>,
    /// A version `Restore` opened, for the app to take (`SongEdit::Restored`).
    pub restored: Option<Song>,
}

/// Where the window's parts go.
struct Layout {
    panel: Rect,
    title: Rect,
    close: Rect,
    list: Rect,
    /// `Name` and `Note`: the label lines and the fields under them.
    labels: [Rect; 2],
    fields: [Rect; 2],
    buttons: [Rect; 4],
}

fn layout(screen: Rect, m: &Metrics) -> Layout {
    let u = |v: f32| m.u(v);
    let h = m.round_px(u(MAX_H).min(screen.height() - 2.0 * u(MARGIN)));
    let top = m.round_px(screen.center().y - h / 2.0);
    let panel = Rect::from_x_y_ranges(crate::ui::dialog::floating_x(screen, m), top..=top + h);
    let inner = panel.shrink(u(PAD));
    // `n` cells side by side across the inner width, `gap` apart, from `top`, `h` tall.
    let cells = |top: f32, h: f32, n: usize| -> Vec<Rect> {
        let w = (inner.width() - (n - 1) as f32 * u(GAP)) / n as f32;
        (0..n)
            .map(|i| {
                let left = m.round_px(inner.left() + i as f32 * (w + u(GAP)));
                Rect::from_min_size(pos2(left, top), vec2(m.round_px(w), h))
            })
            .collect()
    };
    let buttons = cells(inner.bottom() - u(BUTTON_H), u(BUTTON_H), 4);
    let fields = cells(buttons[0].top() - u(GAP) - u(FIELD_H), u(FIELD_H), 2);
    let labels = cells(fields[0].top() - u(LABEL_H), u(LABEL_H), 2);
    let title = Rect::from_min_size(inner.min, vec2(inner.width(), u(TITLE_H)));
    Layout {
        panel,
        close: Rect::from_min_size(
            pos2(inner.right() - u(TITLE_H), inner.top()),
            vec2(u(TITLE_H), u(TITLE_H)),
        ),
        list: Rect::from_x_y_ranges(
            inner.x_range(),
            title.bottom() + u(GAP)..=labels[0].top() - u(GAP),
        ),
        title,
        labels: [labels[0], labels[1]],
        fields: [fields[0], fields[1]],
        buttons: [buttons[0], buttons[1], buttons[2], buttons[3]],
    }
}

/// `bytes` as `12 KB` or `1.5 MB`.
fn size(bytes: usize) -> String {
    match bytes < 1 << 20 {
        true => format!("{} KB", bytes.div_ceil(1024)),
        false => format!("{:.1} MB", bytes as f64 / f64::from(1 << 20)),
    }
}

fn message(tag: &'static str, line: String, confirm: Option<(String, String)>) -> Nav {
    Nav::Open(Overlay::Message {
        tag,
        lines: vec![line],
        confirm,
        extra: None,
    })
}

/// Shows the window; call it every frame while it is the topmost overlay.
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx) -> Option<Nav> {
    let ctx = ui.ctx().clone();
    let (c, m) = (&app.cx.theme.colors, app.cx.m);
    let screen = ctx.content_rect();
    let l = layout(screen, m);
    let id = Id::new("versions");
    crate::ui::dialog::dialog_box(&ctx, id, (c, m), screen, l.panel);
    let mut nav = None;

    // `Delete`'s answer.
    if let Some((DELETE_TAG, yes)) = app.answer
        && let Some(pick) = state.asked.take()
        && yes
    {
        let (b, v) = match pick {
            Pick::Branch(b) => (b, None),
            Pick::Version(b, v) => (b, Some(v)),
        };
        Arc::make_mut(&mut app.song.file.versions).delete(b, v);
        state.pick = None;
        app.files.song_changed();
    }
    let versions = Arc::clone(&app.song.file.versions);
    state.pick = state.pick.filter(|p| p.exists(&versions));

    let (text, fill) = (c.text.to_color32(), c.menu_fill.to_color32());
    let dim = text.gamma_multiply(DIM_ALPHA);
    let (font, small) = (m.font_id(FontSize::Pt10), m.font_id(FontSize::Pt8));
    let mut clicked = None;
    // The fields' new texts: (field, text).
    let mut typed = Vec::new();
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(l.panel.min)
        .constrain(false)
        .fade_in(false)
        .show(&ctx, |ui| {
            ui.allocate_exact_size(l.panel.size(), Sense::hover());
            let painter = ui.painter().clone();
            let title_font = m.font_id(FontSize::Pt12);
            painter.text(
                l.title.left_center(),
                Align2::LEFT_CENTER,
                "Project versions",
                title_font,
                text,
            );
            if close_x(ui, &app.cx, id.with("close"), l.close, (fill, 0.375), 0.7).clicked() {
                nav = Some(Nav::Close);
            }

            // The list: each branch, then its versions newest first.
            let rows: Vec<Pick> = (versions.branches.iter().enumerate())
                .flat_map(|(b, branch)| {
                    let newest_first = (0..branch.versions.len()).rev();
                    std::iter::once(Pick::Branch(b))
                        .chain(newest_first.map(move |v| Pick::Version(b, v)))
                })
                .collect();
            let row_h = m.u(ROW_H);
            let content_h = rows.len() as f32 * row_h;
            let offset = scroll(
                ui,
                id.with("scroll"),
                m.u(1.0),
                &mut state.kinetic,
                l.list,
                content_h,
                &mut state.scroll,
            );
            let clipped = painter.with_clip_rect(l.list);
            if rows.is_empty() {
                let empty = "No versions yet. Save version keeps the song as it is now.";
                clipped.text(
                    l.list.center(),
                    Align2::CENTER_CENTER,
                    empty,
                    small.clone(),
                    dim,
                );
            }
            for (i, &row) in rows.iter().enumerate() {
                let r = Rect::from_min_size(
                    pos2(l.list.left(), l.list.top() - offset + i as f32 * row_h),
                    vec2(l.list.width(), row_h),
                );
                if !r.intersects(l.list) {
                    continue;
                }
                if ui
                    .interact(r.intersect(l.list), id.with(("row", i)), Sense::click())
                    .clicked()
                {
                    state.pick = Some(row);
                }
                if state.pick == Some(row) {
                    clipped.rect_filled(r, 0.0, text.gamma_multiply(PICKED_ALPHA));
                }
                let inside = r.shrink2(vec2(m.u(GAP), 0.0));
                let (name, note, right, inside) = match row {
                    Pick::Branch(b) => {
                        let branch = &versions.branches[b];
                        let n = branch.versions.len();
                        let current = if b == versions.current {
                            "current · "
                        } else {
                            ""
                        };
                        let plural = if n == 1 { "" } else { "s" };
                        let right =
                            format!("{current}{n} version{plural} · {}", size(branch.bytes()));
                        (&branch.name, "", right, inside)
                    }
                    Pick::Version(b, v) => {
                        let version = &versions.branches[b].versions[v];
                        let time = meta_time(version.time);
                        let right = time.get(..16).unwrap_or(&time).to_owned();
                        (
                            &version.name,
                            version.note.as_str(),
                            right,
                            inside.with_min_x(inside.left() + m.u(INDENT)),
                        )
                    }
                };
                let right_w = clipped
                    .layout_no_wrap(right.clone(), small.clone(), dim)
                    .size()
                    .x;
                clipped.text(
                    inside.right_center(),
                    Align2::RIGHT_CENTER,
                    right,
                    small.clone(),
                    dim,
                );
                let room = inside.with_max_x(inside.right() - right_w - m.u(GAP));
                let name_w = clipped
                    .layout_no_wrap(name.clone(), font.clone(), text)
                    .size()
                    .x;
                truncated(
                    &clipped.with_clip_rect(room.intersect(l.list)),
                    room,
                    Align2::LEFT_CENTER,
                    name,
                    font.clone(),
                    text,
                    0,
                );
                let note_room = room.with_min_x(room.left() + name_w + m.u(GAP));
                if !note.is_empty() && note_room.width() > 0.0 {
                    let clip = note_room.intersect(l.list);
                    truncated(
                        &clipped.with_clip_rect(clip),
                        note_room,
                        Align2::LEFT_CENTER,
                        note,
                        small.clone(),
                        dim,
                        0,
                    );
                }
            }

            // `Name` and `Note`, of the picked row.
            let picked = state.pick.map(|p| match p {
                Pick::Branch(b) => (&versions.branches[b].name, None),
                Pick::Version(b, v) => {
                    let version = &versions.branches[b].versions[v];
                    (&version.name, Some(&version.note))
                }
            });
            let fields = [
                ("Name", picked.map(|p| p.0), NAME_LIMIT),
                ("Note", picked.and_then(|p| p.1), NOTE_LIMIT),
            ];
            for (i, (label, value, limit)) in fields.into_iter().enumerate() {
                let ink = if value.is_some() {
                    text
                } else {
                    text.gamma_multiply(DISABLED_ALPHA)
                };
                painter.text(
                    l.labels[i].left_center(),
                    Align2::LEFT_CENTER,
                    label,
                    small.clone(),
                    ink,
                );
                let mut edited = value.cloned().unwrap_or_default();
                let hint = match (value, i) {
                    (None, _) => "",
                    (Some(_), 0) => "Name",
                    (Some(_), _) => "Add a note",
                };
                let edit = TextEdit::singleline(&mut edited)
                    .id(id.with(label))
                    .char_limit(limit)
                    .interactive(value.is_some());
                let mut response = text_field(ui, &app.cx, l.fields[i], hint, edit);
                crate::android_text::edit(&mut response, &mut edited, limit);
                if value.is_some_and(|v| *v != edited) {
                    typed.push((i, edited));
                }
            }

            // The buttons; those with nothing to act on dimmed and inert.
            let version = state.pick.and_then(|p| p.version(&versions));
            let can = [
                true,
                matches!(state.pick, Some(Pick::Version(..))),
                version.is_some(),
                state.pick.is_some(),
            ];
            for (i, label) in BUTTONS.into_iter().enumerate() {
                let mut b = crate::ui::dialog::button(id.with(label), l.buttons[i], label, c, m);
                b.font = FontSize::Pt10;
                if !can[i] {
                    (b.fill_on, b.text_on) = (
                        text.gamma_multiply(DISABLED_ALPHA),
                        fill.gamma_multiply(DISABLED_ALPHA),
                    );
                    b.disabled = true;
                }
                if button(ui, &app.cx, &b, true).clicked() {
                    clicked = Some(i);
                }
            }
        });
    drop(versions);

    for (field, edited) in typed {
        let Some(pick) = state.pick else { break };
        let versions = Arc::make_mut(&mut app.song.file.versions);
        match (pick, field) {
            (Pick::Branch(b), 0) => versions.branches[b].name = edited,
            (Pick::Version(b, v), 0) => versions.branches[b].versions[v].name = edited,
            (Pick::Version(b, v), _) => versions.branches[b].versions[v].note = edited,
            _ => continue,
        }
        app.files.song_changed();
    }

    let pick = state.pick;
    let at = pick.and_then(|p| p.version(&app.song.file.versions));
    match clicked {
        Some(0) => {
            let (time, limit) = (
                crate::project_files::now_secs(),
                crate::history::size_limit(),
            );
            match versions::commit(app.song, pick.map(Pick::branch), time, limit) {
                Ok((b, v)) => {
                    state.pick = Some(Pick::Version(b, v));
                    app.files.song_changed();
                }
                Err(e) => nav = Some(message(ERROR_TAG, e.to_string(), None)),
            }
        }
        Some(1) => match at.map(|at| versions::restored(app.song, at)) {
            Some(Ok(song)) => {
                state.restored = Some(song);
                app.version_restored();
            }
            Some(Err(e)) => nav = Some(message(ERROR_TAG, e, None)),
            None => {}
        },
        Some(2) => {
            if let Some(b) =
                at.and_then(|at| Arc::make_mut(&mut app.song.file.versions).branch_from(at))
            {
                state.pick = Some(Pick::Branch(b));
                app.files.song_changed();
            }
        }
        Some(3) => {
            let versions = &app.song.file.versions;
            let question = match pick {
                Some(Pick::Branch(b)) => {
                    let branch = &versions.branches[b];
                    format!(
                        "Delete the branch '{}' and its versions? This cannot be undone.",
                        branch.name
                    )
                }
                Some(Pick::Version(b, v)) => {
                    let version = &versions.branches[b].versions[v];
                    format!("Delete '{}'? This cannot be undone.", version.name)
                }
                None => return nav,
            };
            state.asked = pick;
            let confirm = Some(("Delete".to_owned(), "Cancel".to_owned()));
            nav = Some(message(DELETE_TAG, question, confirm));
        }
        _ => {}
    }
    if crate::ui::close_pressed(&ctx) {
        nav = Some(Nav::Close);
    }
    nav
}

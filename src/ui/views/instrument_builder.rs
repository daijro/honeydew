//! A DW Sampler's `New from samples…` (its card's ≡ menu; issue #39, §3.1 and §3.3): the instrument builder, a side
//! browser panel that maps audio files into an instrument by their names ([`crate::token_map`]).
//!
//! - Down the left edge: `Files` and `Folder` pick the audio files (several files, or a folder scanned through its
//!   folders, by the desktop's dialogs); `Split` picks the separator (`_ - . @ +` or a custom one); `Save` asks for the
//!   name, then saves and loads the instrument.
//! - The list: the first file's tokens, each with its role (a tap picks another from the roles menu), then each file
//!   with the keys and velocities it maps to, the flagged ones first: a file whose token does not parse is dimmed and
//!   left out; one that overlaps another says which.
//! - New files guess the separator and the roles again ([`crate::token_map::guess`]).
//! - `Save` writes `My Instruments/<name>.sfz` and the samples into `My Instruments/<name>/`
//!   ([`ProjectFiles::save_instrument`]), then loads it as the module's program, as its picker's pick does, and
//!   closes. A failure shows its message instead.
//!
//! The X, a click outside, Escape or the module being gone close it.

use std::path::PathBuf;

use eframe::egui;

use super::{AppCtx, Nav, Overlay};
use crate::panels::cstm::pick_dw_program;
use crate::project_files::SAVE_FILE_FAILED;
use crate::token_map::{Flag, Region, Role, SEPARATORS, guess, map, separator, tokens};
use crate::ui::keyboard::key_name;
use crate::ui::modules_browser::{Chrome, PanelClick, PanelRow, RowKind, SideButton, SidePanel};
use crate::ui::popup::PopupItem;
use crate::ui::preset_picker::modal;

/// The panel's title.
pub const TITLE: &str = "New instrument";
/// The message's tag when the save fails.
const TAG: &str = "instrument_builder";
/// The left buttons, in order.
const FILES: usize = 0;
const FOLDER: usize = 1;
const SPLIT: usize = 2;
const SAVE: usize = 3;
/// The open builder, afresh on each opening.
#[derive(Default)]
pub struct State {
    panel: SidePanel,
    /// The egui pass it was last drawn in: a gap means it was closed in between.
    pass: u64,
    separator: String,
    roles: Vec<Role>,
    /// The first file the separator and the roles were guessed from.
    guessed: Option<PathBuf>,
    /// The name `Save` proposes: the picked folder's, else the first file's first token.
    folder: Option<String>,
    popup: Option<Popup>,
    /// What the name field asks for.
    asking: Option<Ask>,
}

#[derive(Clone, Copy)]
enum Popup {
    Separator(egui::Pos2),
    /// The roles menu of this token position.
    Role(usize, egui::Pos2),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ask {
    Separator,
    Name,
}

/// A list row's tap.
#[derive(Clone, Copy)]
enum Row {
    Token(usize),
    File,
}

/// A separator menu pick: one of [`SEPARATORS`], or a custom one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Split {
    Given(usize),
    Custom,
}

/// The files can be picked here: the desktop's dialogs.
const PICKS: bool = cfg!(not(any(target_arch = "wasm32", mobile)));

/// Draws the builder for the DW Sampler `module` on `channel`, over `files` (the overlay's, which `Files` and `Folder`
/// replace).
pub fn show(
    state: &mut State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    (channel, module): (usize, i32),
    files: &mut Vec<PathBuf>,
) -> Option<Nav> {
    if app
        .song
        .find_module(channel, module)
        .is_none_or(|m| m.type_id != 1)
    {
        return Some(Nav::Close);
    }
    let ctx = ui.ctx().clone();
    let pass = ctx.cumulative_pass_nr();
    if state.pass + 1 != pass {
        *state = State::default();
    }
    state.pass = pass;
    let c4 = app.middle_c_is_c4;
    let names: Vec<String> = (files.iter())
        .map(|f| {
            f.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    if state.guessed.as_ref() != files.first()
        && let Some(first) = names.first()
    {
        state.separator = separator(first).to_owned();
        state.roles = guess(&tokens(first, &state.separator));
        state.guessed = files.first().cloned();
    }
    let first: Vec<&str> = names
        .first()
        .map_or_else(Vec::new, |n| tokens(n, &state.separator));
    let regions = map(&names, &state.separator, &state.roles);
    let id = egui::Id::new(TAG);
    let cx = &app.cx;

    // Before the panel, which would take Escape.
    match state.popup {
        Some(Popup::Separator(at)) => {
            let items = split_items(&state.separator);
            let result = modal(&ctx, cx, id.with("separator"), at, &items);
            match result.selected {
                Some(Split::Given(i)) => state.set_separator(SEPARATORS[i], &first_of(&names)),
                Some(Split::Custom) => {
                    state.asking = Some(Ask::Separator);
                    state.panel.ask("Enter separator", state.separator.clone());
                }
                None => {}
            }
            if result.close {
                state.popup = None;
            }
        }
        Some(Popup::Role(at, pos)) => {
            let current = state.roles.get(at).copied().unwrap_or(Role::Ignore);
            let result = modal(&ctx, cx, id.with("roles"), pos, &role_items(current));
            if let Some(role) = result.selected {
                state
                    .roles
                    .resize(state.roles.len().max(at + 1), Role::Ignore);
                state.roles[at] = role;
            }
            if result.close {
                state.popup = None;
            }
        }
        None => {}
    }

    let saving = regions.iter().any(|r| !unparsed(r));
    let buttons = [
        SideButton::Plain("Files", PICKS),
        SideButton::Plain("Folder", PICKS),
        SideButton::Plain("Split", !first.is_empty()),
        SideButton::Plain("Save", saving),
    ];
    let chrome = Chrome {
        id,
        title: TITLE,
        back: false,
        buttons: &buttons,
        popup: state.popup.is_some(),
    };
    let sections = [
        format!("Tokens, split at {}", quoted(&state.separator)),
        format!("Mapping: {} files", files.len()),
    ];
    let roles: Vec<String> = (0..first.len())
        .map(|i| {
            let role = state.roles.get(i).copied().unwrap_or(Role::Ignore);
            format!("Token {} · {}", i + 1, role.label())
        })
        .collect();
    let subs: Vec<String> = (regions.iter().zip(&names))
        .map(|(r, name)| {
            describe(
                r,
                &regions,
                &tokens(name, &state.separator),
                &state.roles,
                c4,
            )
        })
        .collect();
    let click = state.panel.show(&ctx, cx, &chrome, |_| {
        if files.is_empty() {
            let hint = "Pick audio files, or a folder";
            return vec![section(hint)];
        }
        let tokens = (first.iter().zip(&roles).enumerate()).map(|(i, (token, role))| PanelRow {
            name: token,
            folder: false,
            sub: Some(role.as_str()),
            kind: RowKind::Enabled(Row::Token(i)),
            picked: false,
            mark: None,
        });
        // The flagged files first, so that a long list shows what to fix.
        let mut mapped: Vec<_> = regions.iter().zip(&subs).collect();
        mapped.sort_by_key(|(r, _)| r.flag.is_none());
        let mapped = mapped.into_iter().map(|(r, sub)| PanelRow {
            name: &r.file,
            folder: false,
            sub: Some(sub.as_str()),
            kind: match unparsed(r) {
                true => RowKind::Disabled,
                false => RowKind::Enabled(Row::File),
            },
            picked: false,
            mark: None,
        });
        std::iter::once(section(&sections[0]))
            .chain(tokens)
            .chain(std::iter::once(section(&sections[1])))
            .chain(mapped)
            .collect()
    });
    let pointer = ctx
        .input(|i| i.pointer.interact_pos())
        .unwrap_or(ctx.content_rect().center());
    match click? {
        PanelClick::Close => return Some(Nav::Close),
        PanelClick::Button(FILES) => {
            if let Some(picked) = pick_files() {
                state.folder = None;
                *files = picked;
            }
        }
        PanelClick::Button(FOLDER) => {
            if let Some((folder, picked)) = pick_folder() {
                state.folder = Some(folder);
                *files = picked;
            }
        }
        PanelClick::Button(SPLIT) => state.popup = Some(Popup::Separator(pointer)),
        PanelClick::Button(SAVE) => {
            let name = (state.folder.clone())
                .or_else(|| first.first().map(|t| t.to_string()))
                .unwrap_or_default();
            state.asking = Some(Ask::Name);
            state.panel.ask("Enter instrument name", name);
        }
        PanelClick::Row(Row::Token(at)) => state.popup = Some(Popup::Role(at, pointer)),
        PanelClick::Enter => {
            let text = state.panel.take_prompt().unwrap_or_default();
            match state.asking.take() {
                Some(Ask::Separator) if !text.is_empty() => {
                    state.set_separator(&text, &first_of(&names))
                }
                Some(Ask::Name) if !text.trim().is_empty() => {
                    return Some(save(app, (channel, module), &text, &regions, files));
                }
                _ => {}
            }
        }
        _ => {}
    }
    None
}

impl State {
    /// A new separator: the roles are guessed again for the first file's new tokens.
    fn set_separator(&mut self, separator: &str, first: &str) {
        self.separator = separator.to_owned();
        self.roles = guess(&tokens(first, separator));
    }
}

fn first_of(names: &[String]) -> String {
    names.first().cloned().unwrap_or_default()
}

fn section(name: &str) -> PanelRow<'_, Row> {
    PanelRow {
        name,
        folder: false,
        sub: None,
        kind: RowKind::Section,
        picked: false,
        mark: None,
    }
}

fn unparsed(region: &Region) -> bool {
    matches!(region.flag, Some(Flag::Unparsed(_)))
}

/// `"_"` as the list shows a separator: in quotes, so that a space shows.
fn quoted(separator: &str) -> String {
    format!("\u{201C}{separator}\u{201D}")
}

/// What a file maps to, under its name: `Root C4 · C3–F#4 · Vel 1–100`, its round robin, group and overlap; or the token
/// that did not parse.
fn describe(r: &Region, all: &[Region], tokens: &[&str], roles: &[Role], c4: bool) -> String {
    if let Some(Flag::Unparsed(at)) = r.flag {
        let token = tokens
            .get(at)
            .map_or_else(|| "(none)".to_owned(), |t| quoted(t));
        let role = roles.get(at).map_or("", |r| r.label());
        return format!("Not parsed: token {} {token} as {role}", at + 1);
    }
    let key = |k: u8| key_name(k, true, c4);
    let mut text = format!(
        "Root {} · {}–{} · Vel {}–{}",
        key(r.root),
        key(r.keys[0]),
        key(r.keys[1]),
        r.vels[0],
        r.vels[1]
    );
    if let Some((position, length)) = r.seq {
        text += &format!(" · RR {position}/{length}");
    }
    if !r.group.is_empty() {
        text += &format!(" · {}", r.group);
    }
    if let Some(Flag::Overlaps(other)) = r.flag {
        text += &format!(" · Overlaps {}", all[other].file);
    }
    text
}

/// The separator menu: [`SEPARATORS`], then `Custom…`; the current one marked.
fn split_items(current: &str) -> Vec<PopupItem<Split>> {
    let given = SEPARATORS.iter().enumerate().map(|(i, s)| PopupItem {
        current: *s == current,
        ..PopupItem::new(Split::Given(i), 0, i, quoted(s))
    });
    let custom = PopupItem {
        current: !SEPARATORS.contains(&current),
        ..PopupItem::new(Split::Custom, 0, SEPARATORS.len(), "Custom…")
    };
    given.chain(std::iter::once(custom)).collect()
}

/// The roles menu, two columns, `current` marked.
fn role_items(current: Role) -> Vec<PopupItem<Role>> {
    let half = Role::ALL.len().div_ceil(2);
    (Role::ALL.iter().enumerate())
        .map(|(i, &role)| PopupItem {
            current: role == current,
            ..PopupItem::new(role, i / half, i % half, role.label())
        })
        .collect()
}

/// Saves the instrument and loads it into the module, as its picker's pick of the file would: its program, its name,
/// the channel's name, a new engine.
fn save(
    app: &mut AppCtx,
    (channel, module): (usize, i32),
    name: &str,
    regions: &[Region],
    files: &[PathBuf],
) -> Nav {
    let saved = app.files.save_instrument(name, regions, files);
    let failed = |err: String| {
        Nav::Swap(Overlay::Message {
            tag: TAG,
            lines: vec![SAVE_FILE_FAILED.into(), err],
            confirm: None,
            extra: None,
        })
    };
    let path = match saved {
        Ok(path) => path,
        Err(err) => return failed(err.to_string()),
    };
    let shorthand = app.files.shorthand(&path);
    let module = (app.song.channels.get_mut(channel)).and_then(|c| c.rack.get_mut(module));
    let (Some(shorthand), Some(module)) = (shorthand, module) else {
        return Nav::Close;
    };
    let mut picked = module.clone();
    super::preset_picker::renew_close_grand(app.files, &mut picked);
    if !pick_dw_program(&mut picked, &shorthand, None) {
        return failed(path.display().to_string());
    }
    *module = picked;
    app.files.name_module(module, &path);
    let name = module.file.preset_name.clone();
    crate::ui::rack::name_song_channel(app.song, channel, name);
    app.audio.samples.forget_missing();
    app.files.song_changed();
    app.pick_loaded();
    Nav::Close
}

/// The desktop's dialogs. Elsewhere nothing is picked: the card's menu offers no builder there.
#[cfg(any(target_arch = "wasm32", mobile))]
fn pick_files() -> Option<Vec<PathBuf>> {
    None
}

#[cfg(any(target_arch = "wasm32", mobile))]
fn pick_folder() -> Option<(String, Vec<PathBuf>)> {
    None
}

#[cfg(not(any(target_arch = "wasm32", mobile)))]
use desktop::{pick_files, pick_folder};

#[cfg(not(any(target_arch = "wasm32", mobile)))]
mod desktop {
    use std::path::{Path, PathBuf};

    use crate::project_files::{SAMPLE_EXTS, is_audio};

    /// The most files a folder scan takes, and the folders it goes down.
    const MAX_FILES: usize = 2000;
    const MAX_DEPTH: usize = 8;

    /// Several audio files, by the desktop's dialog, in name order. `None` when none is picked.
    pub fn pick_files() -> Option<Vec<PathBuf>> {
        let mut files = rfd::FileDialog::new()
            .add_filter("Audio", &SAMPLE_EXTS)
            .pick_files()?;
        files.retain(|f| is_audio(f));
        sort(&mut files);
        (!files.is_empty()).then_some(files)
    }

    /// A folder, by the desktop's dialog: its name and its audio files, its folders' too ([`scan`]). `None` when none
    /// is picked.
    pub fn pick_folder() -> Option<(String, Vec<PathBuf>)> {
        let dir = rfd::FileDialog::new().pick_folder()?;
        let mut files = Vec::new();
        scan(&dir, 0, &mut files);
        sort(&mut files);
        let name = dir.file_name().unwrap_or_default().to_string_lossy();
        Some((name.into_owned(), files))
    }

    /// The audio files in `dir` and its folders, [`MAX_DEPTH`] down, at most [`MAX_FILES`]; hidden entries and links
    /// left out.
    fn scan(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let (path, kind) = (entry.path(), entry.file_type());
            let hidden = entry.file_name().to_string_lossy().starts_with('.');
            match kind {
                _ if hidden || out.len() >= MAX_FILES => {}
                Ok(k) if k.is_dir() && depth < MAX_DEPTH => scan(&path, depth + 1, out),
                Ok(k) if k.is_file() && is_audio(&path) => out.push(path),
                _ => {}
            }
        }
    }

    /// By file name, natural (`C2` before `C10`), then path.
    fn sort(files: &mut [PathBuf]) {
        let name = |p: &PathBuf| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        files.sort_by(|a, b| {
            crate::ui::preset_picker::natural(&name(a), &name(b)).then_with(|| a.cmp(b))
        });
    }
}

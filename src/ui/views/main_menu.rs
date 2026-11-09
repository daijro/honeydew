//! The main menu slot: [`MainMenu`] on the right of the screen, its SETTINGS tab the settings slot's
//! [`SettingsTab`](super::settings_tab::State), and its actions routed:
//! - SONGS `New` (after the Song options dialog's `OK`) replaces the song with an empty one with its options,
//!   `Import` with a file the system's picker gives, and a song or archive row with that file's song, through
//!   [`ProjectFiles::replace`] (which asks first when the song has unsaved changes). `Save` saves it (as `Ctrl+S`).
//!   `Unzip` extracts the selected archive ([`ProjectFiles::unzip_archive`]) and leaves the song as it is.
//! - PROJECT: the tempo steps are song edits; the title, author and link (META@520, @264, @777) are param edits with
//!   no undo step and no rebuild. The title is the display title, not the file name (`Song::title`, HEAD@8), which
//!   names the file a quick save writes. The times and the lock come from `META`.
//!   `Samplerate:` is the running audio output's (`AudioLink::sample_rate`), `-` without one; PR #87: its dropdown
//!   sets the project's own rate (`Song::sample_rate`, a song edit), which the row then shows.
//!   `Reset`, `Hide` and `Lock metadata forever` each ask once, then set the time spent to 0, the time spent to −1
//!   (META@256) or the lock (META@776): no unsaved changes (they set no song-changed flag) and no undo steps
//!   ([`Song::keep_unedited`](crate::song::Song::keep_unedited)).
//!   `Save New` shows the Save dialog; `Quick Render` renders the last audio format with the kept render options;
//!   `Open` opens the online link in the browser. `Detach` shows the floating performance monitor ([`monitor`]) and
//!   closes the menu; `Attach` hides it and leaves the menu open (the monitor is the app's: it keeps its state over a
//!   song load).
//! - The menu has no `Quit`: the desktop window's own close button asks [`quit_question`], which the app answers
//!   (closing the window on `Yes`). `Help` is disabled.
//! - SETTINGS shows the user folder and the Original Content folder in it ([`ProjectFiles::content_dir`]); `Open user
//!   folder` and `Open folder` show them in the system's file manager (the SONGS `Show in file browser...` helper),
//!   and a failure shows its message.
//!   `Restore demo songs` asks the app to write our demo songs into the library's `Demo songs` folder again, behind
//!   its loading screen (`app/demo_restore.rs`).
//! - The theme creator (SETTINGS' `Theme creator`): `Save` writes the theme into the user's themes and flashes where,
//!   `Export` writes it where the user puts it ([`ProjectFiles::export_theme`]), `Import` picks a theme file, which joins
//!   the user's themes and is put in use; a file that is no theme, or a write that fails, shows why. SETTINGS'
//!   `Theme:` and `Load themes` list the user's themes, read again as the menu opens and after each of those.
//! - The SONGS file actions act on the library themselves; a failure shows its message, and `Delete permanently`
//!   asks `Yes` / `No` first. The song's file follows what they move or delete
//!   ([`ProjectFiles::path_moved`](crate::project_files::ProjectFiles::path_moved)).
//! - SONGS' preview (#106): a tapped audio file decodes on the sample pickers' worker
//!   ([`sample_picker::decode_file`]), and the strip's plays go to the preview player, as the pickers' do. It stops
//!   once SONGS is not shown ([`State::leave`]) and when Play starts ([`State::stop_sample`]). `Send to playlist`
//!   hands its files to the playlist as files dropped off it ([`crate::ui::file_drag::send`]) and closes the menu.
//!
//! [`ProjectFiles::replace`]: crate::project_files::ProjectFiles::replace
//! [`ProjectFiles::unzip_archive`]: crate::project_files::ProjectFiles::unzip_archive
//! [`ProjectFiles::content_dir`]: crate::project_files::ProjectFiles::content_dir
//! [`ProjectFiles::export_theme`]: crate::project_files::ProjectFiles::export_theme

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui;

use super::sample_picker::{self, Decoded};
use super::{AppCtx, MenuTab, Nav, Overlay, settings_tab};
use crate::APP_NAME;
use crate::audio_link::AudioLink;
use crate::project_files::Replace;
use crate::project_files::export::quick_render_format;
use crate::ui::clips::song_end_beats;
use crate::ui::main_menu::{MainMenu, MainMenuAction, ProjectInfo, meta_secs};
use crate::ui::performance::Performance;

/// The questions `Lock metadata forever`, `Hide` and `Reset` ask.
const LOCK_TAG: &str = "main_menu_lock";
const HIDE_TAG: &str = "main_menu_hide_time";
const RESET_TAG: &str = "main_menu_reset_time";
/// The window close's question, [`quit_question`].
pub const QUIT_TAG: &str = "main_menu_quit";
/// SONGS `Delete permanently`'s question, and the SONGS messages.
const PURGE_TAG: &str = "main_menu_purge";
const SONGS_TAG: &str = "main_menu_songs";
/// The theme creator's messages.
const THEME_TAG: &str = "main_menu_theme";

/// What the main menu keeps between frames. The app resets it with the song.
#[derive(Default)]
pub struct State {
    menu: MainMenu,
    /// The PROJECT tab's values, from the song every frame.
    info: ProjectInfo,
    /// The frame the menu was last drawn in: a gap means it has just opened.
    shown_frame: u64,
    /// `ProjectFiles::library_changes` when SONGS last read its folder.
    library_changes: u64,
    /// The question asked, until its answer comes.
    asked: Option<&'static str>,
    /// SONGS' previewed file decoding on the worker (#106).
    decoding: Option<(PathBuf, Receiver<Decoded>)>,
}

impl State {
    /// Shows PROJECT with its Performance block scrolled into view, as the menu draws next.
    pub fn show_performance(&mut self) {
        self.menu.show_performance();
    }

    /// The folder open in the SONGS tab, `None` at its root.
    pub fn songs_folder(&self) -> Option<&Path> {
        self.menu.songs_folder()
    }

    /// SONGS is not shown (the menu closed, or on another tab): its preview stops and goes.
    pub(super) fn leave(&mut self, audio: &mut AudioLink) {
        self.decoding = None;
        self.menu.songs_preview().end();
        self.send_plays(audio);
    }

    /// Play started, which stopped the preview player: SONGS' strip stops too.
    pub(super) fn stop_sample(&mut self) {
        self.menu.songs_preview().stop();
    }

    /// A decode that landed goes to SONGS' strip; the strip's plays go to the preview player.
    fn preview(&mut self, ctx: &egui::Context, audio: &mut AudioLink) {
        match self.decoding.as_ref().map(|(_, done)| done.try_recv()) {
            None => {}
            Some(Err(TryRecvError::Empty)) => ctx.request_repaint(),
            Some(done) => {
                let (path, _) = self.decoding.take().expect("decoding");
                let wave = done.ok().and_then(|d| d.wave);
                self.menu.songs_preview().decoded(&path, wave);
            }
        }
        self.send_plays(audio);
    }

    fn send_plays(&mut self, audio: &mut AudioLink) {
        let preview = self.menu.songs_preview();
        sample_picker::send_plays(preview.take_plays(), preview.data(), audio);
    }

    /// [`MainMenu::show_render`].
    pub fn show_render(&mut self, file: &Path, library: &dyn crate::ui::main_menu::SongLibrary) {
        self.menu.show_render(file, library);
    }
}

/// The sound library question's tag ([`sounds_question`]): the app takes its answer (`app/sounds.rs`).
pub const SOUNDS_TAG: &str = "main_menu_sounds";

/// The question at start without a sound library ([`sounds_start_question`]): the app takes its answer
/// (`app/sounds.rs`).
pub const SOUNDS_START_TAG: &str = "main_menu_sounds_start";

/// At start without a sound library (#66): `why`, then `Download` (yes: from its release, `update::download_sounds`),
/// `Later` (no) or `From file…` (extra: [`sounds_question`]), tagged [`SOUNDS_START_TAG`].
pub fn sounds_start_question(why: String) -> Nav {
    Nav::Open(Overlay::Message {
        tag: SOUNDS_START_TAG,
        lines: vec![why, "Download it, or install it from a file?".into()],
        confirm: Some(("Download".into(), "Later".into())),
        extra: Some("From file…".into()),
    })
}

/// Where to install the sound library from, after `why` (none for SETTINGS `Install sound library…`): `ZIP file…`
/// (the extra choice), `Folder…` (yes) or `Cancel`, tagged [`SOUNDS_TAG`].
pub fn sounds_question(why: Option<String>) -> Nav {
    let zip = format!("honeydew-v{}-sounds.zip", env!("CARGO_PKG_VERSION"));
    let how = format!(
        "Install it from the sounds ZIP of a release ({zip}) or the content folder of a full download."
    );
    Nav::Open(Overlay::Message {
        tag: SOUNDS_TAG,
        lines: why.into_iter().chain([how]).collect(),
        confirm: Some(("Folder…".into(), "Cancel".into())),
        extra: Some("ZIP file…".into()),
    })
}

/// `Exit <app>?` with `Yes` / `Cancel`, tagged [`QUIT_TAG`]. It never asks about unsaved changes.
pub fn quit_question() -> Nav {
    message(QUIT_TAG, vec![format!("Exit {APP_NAME}?")], "Yes", "Cancel")
}

/// Draws the menu on `tab` on the right of the screen; its tab row switches `tab`.
pub fn show(
    state: &mut State,
    settings: &mut settings_tab::State,
    perf: &mut Performance,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    tab: &mut MenuTab,
) -> Option<Nav> {
    let ctx = ui.ctx().clone();
    // SONGS reads its folder again when the menu opens and after a save.
    let frame = ctx.cumulative_frame_nr();
    if frame > state.shown_frame + 1 || state.library_changes != app.files.library_changes() {
        state.menu.refresh();
        state.library_changes = app.files.library_changes();
        settings.user_themes = app.files.user_themes();
    }
    // An `.obb` or `.apk` dropped into the content folder imports when SETTINGS, which shows the folder,
    // comes up.
    #[cfg(not(target_arch = "wasm32"))]
    if *tab == MenuTab::Settings && (frame > state.shown_frame + 1 || state.menu.tab != *tab) {
        app.files.import_archives();
    }
    state.shown_frame = frame;
    let mut nav = None;
    let mut answered = None;
    if let Some((tag, yes)) = app.answer
        && state.asked == Some(tag)
    {
        state.asked = None;
        match (tag, yes) {
            // No undo steps and no edits, as `Song::keep_unedited` says, and no song-changed flag.
            (LOCK_TAG, true) => app.song.file.meta.locked = true,
            (HIDE_TAG, true) => app.song.file.meta.editing_seconds = -1.0,
            (RESET_TAG, true) => app.song.file.meta.editing_seconds = 0.0,
            (PURGE_TAG, yes) => answered = state.menu.delete_answered(yes, &mut *app.files.library),
            _ => {}
        }
    }

    // The theme file the theme creator's `Import` picked: it joins the user's themes and is put in use.
    if let Some((path, bytes)) = app.files.take_theme_pick() {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let lines = match crate::theme::import(&stem, &bytes) {
            Ok(theme) => {
                let saved = app.files.save_theme(&theme);
                app.files.flash(format!("Imported theme {}", theme.name));
                state.menu.theme_imported(theme, app.settings);
                settings.user_themes = app.files.user_themes();
                saved
                    .err()
                    .map(|err| vec!["Could not save theme".into(), err])
            }
            Err(err) => Some(vec![format!("Could not load theme \"{file}\""), err]),
        };
        nav = lines.map(theme_message);
    }
    let info = &mut state.info;
    let song = &*app.song;
    let meta = &song.file.meta;
    info.title.clone_from(&meta.display_title);
    info.author.clone_from(&meta.author);
    info.link.clone_from(&meta.info_url);
    info.time_spent_secs = meta.editing_seconds;
    info.metadata_locked = meta.locked;
    info.started_on = meta_secs(&meta.created);
    info.last_saved_on = meta_secs(&meta.modified);
    info.tempo_bpm = song.tempo_bpm;
    // Where the last clip ends, as the playlist's song end, at the song tempo.
    info.length_secs = song_end_beats(song) * 60.0 / song.tempo_bpm.max(1.0);
    // `Samplerate:` is the running output's (`-` without one), or the project's own where it has one (PR #87).
    info.sample_rate = app.audio.sample_rate();
    info.project_rate = app.song.sample_rate;
    // The block reads the shared data, which the app updates; it draws `Detach` or `Attach` by it.
    info.performance.clone_from(perf);
    state.menu.tab = *tab;
    // `Input device:` lists the app's MIDI ports and asks it for a rescan each time it opens.
    settings.set_midi_ports(app.midi_devices);
    settings.content = app.files.content_dir();
    settings.plugins = cfg!(plugin_host);
    settings.orientation = cfg!(target_os = "android");
    state.menu.set_factory(app.files.factory);
    let action = state.menu.show(
        ui,
        &app.cx,
        &mut *app.files.library,
        info,
        (&mut *settings, app.settings),
    );
    app.midi_rescan |= settings.take_midi_rescan();
    if settings.take_credits() {
        nav = Some(Nav::Open(Overlay::Credits));
    }
    if settings.take_install_sounds() {
        nav = Some(sounds_question(None));
    }
    plugin_folders(settings.take_plugin_action(), app.settings);
    let action = action.or(answered);
    *tab = state.menu.tab;

    // The display title, author and link are no undo steps (`Song::keep_unedited`) and the audio never reads them:
    // param edits, with no rebuild, which set the song-changed flag.
    let meta = &mut app.song.file.meta;
    let typed = (&info.title, &info.author, &info.link);
    if typed != (&meta.display_title, &meta.author, &meta.info_url) {
        meta.display_title.clone_from(&info.title);
        meta.author.clone_from(&info.author);
        meta.info_url.clone_from(&info.link);
        app.param_edited();
    }
    state.preview(&ctx, app.audio);
    // A renamed song file renames the song (its file-name title, `Song::title`), which is no edit.
    for (from, to) in state.menu.take_moves() {
        app.files.path_moved(app.song, &from, to.as_deref());
    }

    let Some(action) = action else {
        return nav;
    };
    match action {
        MainMenuAction::Close => return Some(Nav::Close),
        MainMenuAction::NewSong(options) => replace(app, Replace::NewWith(options)),
        MainMenuAction::Save => app.files.save(app.song),
        MainMenuAction::SaveNew => app.files.save_as(app.song),
        MainMenuAction::QuickRender => {
            let format = quick_render_format(app.settings.last_audio_format);
            app.files.quick_render(app.song, format);
        }
        MainMenuAction::Import => replace(app, Replace::Pick),
        MainMenuAction::ImportFolder => app.files.pick_folder(),
        MainMenuAction::Open(path) => replace(app, Replace::Open(path)),
        // The files extract the archive; they flash where it went, or show why not, and SONGS reads again.
        MainMenuAction::Unzip(path) => app.files.unzip_archive(&path),
        MainMenuAction::NewFromTemplate(path) => replace(app, Replace::Template(path)),
        MainMenuAction::SetTempo(bpm) => {
            app.song.set_tempo(bpm);
            app.song_edited(true);
        }
        // PR #87: an edit, whose rebuild reopens the output at the new rate (`AudioLink::replace_song_keeping`).
        MainMenuAction::SetSampleRate(rate) => {
            app.song.sample_rate = rate;
            app.song_edited(true);
        }
        MainMenuAction::LockMetadata => nav = Some(ask(&mut state.asked, LOCK_TAG)),
        MainMenuAction::HideTimeSpent => nav = Some(ask(&mut state.asked, HIDE_TAG)),
        MainMenuAction::ResetTimeSpent => nav = Some(ask(&mut state.asked, RESET_TAG)),
        MainMenuAction::OpenLink(url) => ui.ctx().open_url(egui::OpenUrl::new_tab(url)),
        MainMenuAction::Message(line) => {
            nav = Some(Nav::Open(Overlay::Message {
                tag: SONGS_TAG,
                lines: vec![line],
                confirm: None,
                extra: None,
            }));
        }
        MainMenuAction::ConfirmDelete(question) => {
            state.asked = Some(PURGE_TAG);
            nav = Some(message(PURGE_TAG, vec![question], "Yes", "No"));
        }
        MainMenuAction::WhatsNew => nav = Some(Nav::Open(Overlay::WhatsNew)),
        MainMenuAction::ViewVersions => nav = Some(Nav::Open(Overlay::Versions)),
        // On Linux the file manager opens the folder that holds the file, elsewhere it selects it.
        #[cfg(not(target_arch = "wasm32"))]
        MainMenuAction::Reveal(file) => {
            let shown = crate::ui::main_menu::reveal(&file);
            if let Err(line) = shown {
                nav = Some(Nav::Open(Overlay::Message {
                    tag: SONGS_TAG,
                    lines: vec![line],
                    confirm: None,
                    extra: None,
                }));
            }
        }
        #[cfg(target_arch = "wasm32")]
        MainMenuAction::Reveal(_) => {}
        // Behind its loading screen, in the app (`app/demo_restore.rs`).
        MainMenuAction::RestoreDemoSongs => app.files.restore_demos = true,
        MainMenuAction::SaveTheme(theme) => match app.files.save_theme(&theme) {
            Ok(path) => {
                let file = path.file_name().unwrap_or_default().to_string_lossy();
                let folder = crate::theme::THEMES_FOLDER;
                app.files.flash(format!("Saved theme to {folder}/{file}"));
                settings.user_themes = app.files.user_themes();
            }
            Err(err) => nav = Some(theme_message(vec!["Could not save theme".into(), err])),
        },
        MainMenuAction::ExportTheme(theme) => {
            if let Err(err) = app.files.export_theme(&theme) {
                nav = Some(theme_message(vec!["Could not save theme".into(), err]));
            }
        }
        MainMenuAction::ImportTheme => app.files.pick_theme(),
        MainMenuAction::Preview(path) => {
            let done = sample_picker::decode_file(app.files, &path);
            state.decoding = Some((path, done));
            ctx.request_repaint();
        }
        MainMenuAction::SendToPlaylist(files) => {
            crate::ui::file_drag::send(&ctx, files);
            return Some(Nav::Close);
        }
    }
    nav
}

/// Replaces the song with `what` now, or after the frame (a picked file, a question answered).
fn replace(app: &mut AppCtx, what: Replace) {
    if let Some(song) = app.files.replace(what) {
        *app.song = song;
        app.song_replaced();
    }
}

/// Asks `tag`'s one-step question and remembers it in `asked`. The message rows do not wrap: the lock's long second
/// line is split in two.
fn ask(asked: &mut Option<&'static str>, tag: &'static str) -> Nav {
    *asked = Some(tag);
    let (lines, yes, no): (&[&str], _, _) = match tag {
        LOCK_TAG => (
            &[
                "Proceed with caution!",
                "Lock forever will prevent editing this information in future.",
                "Including copies or new versions of this project!",
            ],
            "Lock forever",
            "Cancel",
        ),
        HIDE_TAG => (
            &[
                "Do you want to hide the following metadata?",
                "'Time spent on project', 'Project started on' and 'Project last saved on'",
                "THIS CAN NOT BE UNDONE!",
            ],
            "Hide forever",
            "Cancel",
        ),
        _ => (
            &[
                "Do you want to reset 'Time spent on project' to zero?",
                "THIS CAN NOT BE UNDONE!",
            ],
            "Yes",
            "No",
        ),
    };
    message(tag, lines.iter().map(|l| l.to_string()).collect(), yes, no)
}

/// The theme creator's message, with `OK`.
fn theme_message(lines: Vec<String>) -> Nav {
    Nav::Open(Overlay::Message {
        tag: THEME_TAG,
        lines,
        confirm: None,
        extra: None,
    })
}

fn message(tag: &'static str, lines: Vec<String>, yes: &str, no: &str) -> Nav {
    Nav::Open(Overlay::Message {
        tag,
        lines,
        confirm: Some((yes.into(), no.into())),
        extra: None,
    })
}

/// A plugin row's tap in SETTINGS: `Add folder` picks a folder (the system's dialog) and scans it; `Rescan` scans every
/// plugin file again; forgotten folders scan again without them.
fn plugin_folders(
    action: Option<crate::ui::settings_tab::PluginAction>,
    settings: &mut crate::settings::Settings,
) {
    use crate::ui::settings_tab::PluginAction;
    let Some(action) = action else { return };
    let all = match action {
        PluginAction::Rescan => true,
        PluginAction::Forgot => false,
        PluginAction::AddFolder => {
            #[cfg(plugin_host)]
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                let dir = dir.display().to_string();
                if !settings.plugin_folders.contains(&dir) {
                    settings.plugin_folders.push(dir);
                }
            }
            false
        }
    };
    crate::plugins::rescan_folders(&settings.plugin_folders, all);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_link::AudioLink;
    use crate::project_files::{ProjectFiles, mem::MemFiles};
    use crate::settings::Settings;
    use crate::song::{ChannelKind, SamplerSlot, Song};
    use crate::ui::UiCtx;
    use crate::ui::main_screen::Regions;
    use crate::ui::offscreen::{Offscreen, SCREEN};
    use crate::ui::views::SongEdit;

    /// The PROJECT tab's `+` on a 140 BPM song with one SYNC-on audio clip (ratio 1.5).
    #[test]
    fn the_tempo_steps_rescale_the_audio_clips_in_a_final_edit() {
        let mut song = Song {
            tempo_bpm: 140.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::AudioClip);
        let mut sample = SamplerSlot::default();
        (sample.file.stretch_mode, sample.file.stretch_ratio) = (1, 1.5);
        song.channels[1].lanes[0].add_clip(0, 4.0).file.sample = Some(Box::new(sample));
        let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        let (mut state, mut settings, mut tab) = (
            State::default(),
            settings_tab::State::default(),
            MenuTab::Project,
        );
        let mut screen = Offscreen::new();
        let mut edits = Vec::new();
        let mut frame = |screen: &mut Offscreen, click: Option<egui::Pos2>| {
            let mut f = |ui: &mut egui::Ui, cx: &UiCtx| {
                let mut app = AppCtx {
                    cx: *cx,
                    regions: Regions::new(SCREEN, cx.m),
                    song: &mut song,
                    audio: &mut AudioLink::default(),
                    selected: &mut 1,
                    snapshot: None,
                    playing: false,
                    armed: false,
                    previewing: false,
                    middle_c_is_c4: false,
                    settings: &mut Settings::default(),
                    ghost_notes: true,
                    files: &mut files,
                    edit: None,
                    answer: None,
                    covered: false,
                    playhead: None,
                    seek: None,
                    midi_devices: &[],
                    midi_rescan: false,
                };
                let mut perf = Performance::default();
                show(&mut state, &mut settings, &mut perf, ui, &mut app, &mut tab);
                edits.push(app.edit);
            };
            match click {
                Some(pos) => screen.click(pos, &mut f),
                None => {
                    screen.frame(vec![], &mut f);
                }
            }
        };
        frame(&mut screen, None);
        let id = egui::Id::new("main_menu_project").with(("tempo", 2_usize));
        let plus = (screen.ctx.read_response(id))
            .expect("`+` was drawn")
            .rect
            .center();
        frame(&mut screen, Some(plus));
        assert_eq!(song.tempo_bpm, 141.0);
        let sample = (song.channels[1].lanes[0].clips[0].file.sample.as_ref()).unwrap();
        assert!((sample.file.stretch_ratio - 1.5 * 140.0 / 141.0).abs() < 1e-12);
        assert_eq!(edits.last(), Some(&Some(SongEdit::Final)));
    }

    /// SETTINGS' `Input device:` lists `AppCtx::midi_devices` and asks the app for one rescan per open; a pick is
    /// the stored `settings.midi_device`.
    #[test]
    fn the_input_device_row_lists_the_apps_ports_and_asks_for_one_rescan_per_open() {
        let mut song = Song::default();
        let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        let ports = vec!["Keys 49 20:0".to_owned(), "Pad Box 24:0".to_owned()];
        let mut stored = Settings::default();
        let (mut state, mut tab_state, mut tab) = (
            State::default(),
            settings_tab::State::default(),
            MenuTab::Settings,
        );
        let mut screen = Offscreen::new();
        let rescans = std::cell::Cell::new(0);
        let mut frame = |screen: &mut Offscreen, click: Option<egui::Pos2>| {
            let mut f = |ui: &mut egui::Ui, cx: &UiCtx| {
                let mut app = AppCtx {
                    cx: *cx,
                    regions: Regions::new(SCREEN, cx.m),
                    song: &mut song,
                    audio: &mut AudioLink::default(),
                    selected: &mut 1,
                    snapshot: None,
                    playing: false,
                    armed: false,
                    previewing: false,
                    middle_c_is_c4: false,
                    settings: &mut stored,
                    ghost_notes: true,
                    files: &mut files,
                    edit: None,
                    answer: None,
                    covered: false,
                    playhead: None,
                    seek: None,
                    midi_devices: &ports,
                    midi_rescan: false,
                };
                let mut perf = Performance::default();
                show(
                    &mut state,
                    &mut tab_state,
                    &mut perf,
                    ui,
                    &mut app,
                    &mut tab,
                );
                rescans.set(rescans.get() + u32::from(app.midi_rescan));
            };
            match click {
                Some(pos) => {
                    screen.click(pos, &mut f);
                    Vec::new()
                }
                None => Offscreen::texts(&screen.frame(vec![], &mut f)),
            }
        };
        frame(&mut screen, None);
        let row = egui::Id::new("settings_tab").with(1_usize);
        let at = screen
            .ctx
            .read_response(row)
            .expect("the row")
            .rect
            .center();
        frame(&mut screen, Some(at));
        let texts = frame(&mut screen, None);
        for label in ["<no midi input>", "Any available", "Keys 49", "Pad Box"] {
            assert!(texts.iter().any(|(t, _)| t == label), "{label}: {texts:?}");
        }
        // Once, in the frame of the tap.
        assert_eq!(rescans.get(), 1);
        // The pick: `Pad Box` is the fourth item.
        let pad = screen
            .ctx
            .read_response(
                egui::Id::new("settings_tab")
                    .with("popup")
                    .with((0_usize, 3_usize)),
            )
            .expect("the item")
            .rect
            .center();
        frame(&mut screen, Some(pad));
        assert_eq!(stored.midi_device, "Pad Box");
    }
}

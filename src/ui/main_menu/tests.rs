//! The main menu driven offscreen: 1170 × 540 points at 1 pixel per point and UI scale 100 % (U = 32).

use std::io;
use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Color32, Context, Event, Key, Modifiers, PointerButton, RawInput, Shape, pos2, vec2,
};

use super::*;
use crate::metrics::DEFAULT_UI_SCALE_INDEX;
use crate::theme::theme_by_id;
use crate::ui::offscreen::key_tap;
use crate::ui::performance::{IDLE_TEXT, Performance, footer_line};
use crate::ui::song_options::{Mode, SongOptions, TITLE};

const SIZE: [f32; 2] = [1170.0, 540.0];

/// A fake factory of the song templates `names`, each file's bytes its name.
fn factory_of(names: &[&str]) -> &'static crate::content::Factory {
    let entries = names.iter().map(|name| {
        let bytes = name.as_bytes().to_vec();
        crate::content::Entry::new(format!("Templates/{name}.songtmpl"), move || bytes.clone())
    });
    Box::leak(Box::new(crate::content::Factory::new(entries.collect())))
}

/// Where the fake factory's template `name` is.
fn template_at(name: &str) -> PathBuf {
    Path::new(crate::content::ROOT).join(format!("Templates/{name}.songtmpl"))
}

/// A library with one folder: `/My Songs` holds `Alpha.flm`, `beta.flm` and the folder `Demo songs`, which holds
/// `Three.flm`.
struct Fake;

impl SongLibrary for Fake {
    fn list(&self, folder: Option<&Path>) -> io::Result<Vec<SongEntry>> {
        let entry = |name: &str, dir: &str, is_folder, recency| SongEntry {
            name: name.to_owned(),
            path: PathBuf::from(dir).join(name),
            is_folder,
            recency,
        };
        Ok(match folder.and_then(Path::to_str) {
            None => vec![
                entry("beta.flm", "/My Songs", false, 2),
                entry("Demo songs", "/My Songs", true, 0),
                entry("Alpha.flm", "/My Songs", false, 1),
            ],
            Some("/My Songs/Demo songs") => {
                vec![entry("Three.flm", "/My Songs/Demo songs", false, 0)]
            }
            Some(other) => panic!("listed {other}"),
        })
    }

    fn read(&self, _: &Path) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn store(&mut self, _: &str, _: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

struct Rig {
    ctx: Context,
    menu: MainMenu,
    /// What SONGS lists: [`Fake`] unless a test changes it.
    library: Box<dyn SongLibrary>,
    info: ProjectInfo,
    settings: (SettingsTab, Settings),
    /// The screen's size, [`SIZE`] unless a test changes it.
    screen: egui::Vec2,
    /// Device pixels per point: 1 unless a test changes it.
    ppp: f32,
    time: f64,
    /// What the last frame painted and returned.
    shapes: Vec<egui::epaint::ClippedShape>,
    action: Option<MainMenuAction>,
}

impl Rig {
    fn new(tab: Tab) -> Self {
        let mut rig = Self {
            ctx: Context::default(),
            menu: MainMenu::new(tab),
            library: Box::new(Fake),
            info: ProjectInfo::default(),
            settings: Default::default(),
            screen: SIZE.into(),
            ppp: 1.0,
            time: 0.0,
            shapes: Vec::new(),
            action: None,
        };
        // No templates of ours, whatever the real factory holds.
        rig.menu.set_factory(factory_of(&[]));
        rig.frame(vec![]);
        rig
    }

    /// The same at `ppp` device pixels per point.
    fn at_ppp(tab: Tab, ppp: f32) -> Self {
        let mut rig = Self::new(tab);
        rig.ppp = ppp;
        rig.frame(vec![]);
        rig
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.1;
        let mut raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        raw.viewports
            .entry(raw.viewport_id)
            .or_default()
            .native_pixels_per_point = Some(self.ppp);
        let theme = theme_by_id(0);
        let (menu, info, library) = (&mut self.menu, &mut self.info, &mut *self.library);
        let (tab, settings) = (&mut self.settings.0, &mut self.settings.1);
        let mut action = None;
        let output = self.ctx.run(raw, |ctx| {
            // No surface-width clamp: these tests fix U = 32 × scale. The app caps U at about width / 20, which the
            // tab-row test's 200 % scale exceeds on both its 360 px and its 1170 px screens.
            let m = Metrics::from_pixels_per_point(ctx.pixels_per_point(), settings.ui_scale_index);
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    action = menu.show(ui, &cx, library, info, (&mut *tab, &mut *settings))
                });
        });
        self.shapes = output.shapes;
        self.action = action;
    }

    /// A press and release at `pos`; returns the action of the release frame.
    fn click(&mut self, pos: Pos2) -> Option<MainMenuAction> {
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        let action = self.action.clone();
        // Let the release settle (popups close, rows re-read).
        self.frame(vec![]);
        action
    }

    /// Texts painted in the last frame, in paint order, with where they were drawn.
    fn texts(&self) -> Vec<(String, Rect)> {
        self.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.galley.rect.translate(t.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect()
    }

    fn has(&self, text: &str) -> bool {
        self.texts().iter().any(|(t, _)| t == text)
    }

    /// Where `text` was drawn (the last time, so a popup item wins over the button it covers).
    fn find(&self, text: &str) -> Rect {
        let texts = self.texts();
        let found = texts.iter().rev().find(|(t, _)| t == text);
        found
            .unwrap_or_else(|| panic!("{text:?} not drawn: {texts:?}"))
            .1
    }

    /// Scrolls the page under the content's centre all the way down.
    fn scroll_down(&mut self) {
        let content = shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m()).content;
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -5000.0),
            modifiers: Default::default(),
        };
        self.frame(vec![Event::PointerMoved(content.center()), wheel]);
        for _ in 0..60 {
            self.frame(vec![]);
        }
    }

    fn click_text(&mut self, text: &str) -> Option<MainMenuAction> {
        let at = self.find(text).center();
        self.click(at)
    }

    /// The song list's rows, top to bottom: the known names below the title (1.75 U), the breadcrumb (0.75 U) inside a
    /// folder, and the search bar's 1.75 U while it shows.
    fn rows(&self) -> Vec<String> {
        let names = [
            "Alpha.flm",
            "beta.flm",
            "Demo songs",
            "Three.flm",
            "Demo Songs",
            "Gen Demo.flm",
        ];
        let content = shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m()).content;
        let bar = if self.has("Clear") { 1.75 } else { 0.0 };
        let crumb = if self.menu.songs_folder().is_some() {
            0.75
        } else {
            0.0
        };
        let list_top = content.top() + (1.75 + crumb + bar) * 32.0;
        let mut rows: Vec<_> = self
            .texts()
            .into_iter()
            .filter(|(t, r)| names.contains(&t.as_str()) && r.top() >= list_top)
            .collect();
        rows.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
        rows.into_iter().map(|(t, _)| t).collect()
    }
}

fn m() -> Metrics {
    Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX)
}

#[test]
fn the_menu_is_at_most_20_u_wide_on_the_right_with_the_tab_row_under_the_header() {
    let screen = Rect::from_min_size(Pos2::ZERO, SIZE.into());
    let l = shell_layout(screen, &m());
    // 20 U of a 1170 point screen (U = 32).
    assert_eq!(
        l.panel,
        Rect::from_min_max(pos2(530.0, 0.0), pos2(1170.0, 540.0))
    );
    // Header 1.75 U with 0.5 U side insets; tabs 1.25 U across the panel; a 0.25 U gap, 0.5 U insets.
    assert_eq!(
        l.header,
        Rect::from_min_max(pos2(546.0, 0.0), pos2(1154.0, 56.0))
    );
    assert_eq!(
        l.tabs,
        Rect::from_min_max(pos2(530.0, 56.0), pos2(1170.0, 96.0))
    );
    assert_eq!(
        l.content,
        Rect::from_min_max(pos2(546.0, 104.0), pos2(1154.0, 524.0))
    );
    // A narrower screen: the whole width.
    let narrow = Rect::from_min_size(Pos2::ZERO, vec2(600.0, 540.0));
    assert_eq!(shell_layout(narrow, &m()).panel, narrow);
}

#[test]
fn the_header_names_the_app_and_its_version_and_the_tabs_run_in_order() {
    let rig = Rig::new(Tab::Songs);
    let panel = shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m()).panel;
    for text in [APP_NAME, "Version 0.1.3", "Help", "Quit"] {
        assert!(panel.contains_rect(rig.find(text)), "{text}");
    }
    let xs: Vec<f32> = Tab::ALL
        .iter()
        .map(|t| rig.find(t.label()).center().x)
        .collect();
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
    assert!(xs[0] > panel.left());
    // No SHOP and no SYNC tab.
    assert!(!rig.has("SHOP") && !rig.has("SYNC"));
}

#[test]
fn clicking_a_tab_shows_it() {
    let mut rig = Rig::new(Tab::Songs);
    assert!(rig.has("My Songs"));
    for (tab, marker) in [
        (Tab::Project, "Project info"),
        (Tab::Settings, "MIDI:"),
        (Tab::Songs, "My Songs"),
    ] {
        assert_eq!(rig.click_text(tab.label()), None);
        assert_eq!(rig.menu.tab, tab);
        assert!(rig.has(marker), "{tab:?}");
    }
}

#[test]
fn no_tab_shows_the_original_app_s_names() {
    let mut rig = Rig::new(Tab::Songs);
    for tab in Tab::ALL {
        rig.click_text(tab.label());
        for (text, _) in rig.texts() {
            let lower = text.to_lowercase();
            assert!(
                !lower.contains("fl studio") && !lower.contains("image-line"),
                "{tab:?}: {text}"
            );
        }
    }
}

#[test]
fn songs_buttons_return_their_actions() {
    let mut rig = Rig::new(Tab::Songs);
    // `New` asks for the song options first: `Cancel` makes no song, `OK` makes one with them.
    assert_eq!(rig.click_text("New"), None);
    assert!(rig.has(TITLE));
    assert_eq!(rig.click_text("Cancel"), None);
    assert!(!rig.has(TITLE));
    rig.click_text("New");
    // Modal: `Import`, under the band, takes no click.
    assert_eq!(rig.click_text("Import"), None);
    rig.click_text("Time division:");
    let division = SongOptions {
        mode: Mode::Division,
        ..SongOptions::default()
    };
    assert_eq!(
        rig.click_text("OK"),
        Some(MainMenuAction::NewSong(division))
    );
    assert!(!rig.has(TITLE));
    assert_eq!(rig.click_text("Save"), Some(MainMenuAction::Save));
    assert_eq!(rig.click_text("Import"), Some(MainMenuAction::Import));
}

#[test]
fn the_wheel_over_the_song_options_dialog_leaves_the_song_list_where_it_was() {
    let mut library = MemoryLibrary::default();
    for i in 0..40 {
        library.store(&format!("Song {i:02}.flm"), b"s").unwrap();
    }
    let mut rig = Rig::new(Tab::Songs);
    rig.library = Box::new(library);
    rig.menu.refresh();
    rig.frame(vec![]);
    let before = rig.find("Song 00.flm");
    // Over the list and the dialog's band.
    let content = shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m()).content;
    let wheel = |rig: &mut Rig| {
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -5000.0),
            modifiers: Default::default(),
        };
        rig.frame(vec![Event::PointerMoved(content.center()), wheel]);
        for _ in 0..60 {
            rig.frame(vec![]);
        }
    };
    rig.click_text("New");
    wheel(&mut rig);
    rig.click_text("Cancel");
    assert_eq!(rig.find("Song 00.flm"), before);
    // Without the dialog the same wheel scrolls it.
    wheel(&mut rig);
    assert!(!rig.has("Song 00.flm"));
}

/// The size of the font `text` was last drawn in.
fn font_size(rig: &Rig, text: &str) -> f32 {
    (rig.shapes.iter().rev())
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == text => {
                Some(t.galley.job.sections[0].format.font_id.size)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("{text:?} not drawn"))
}

/// The folder title is in the folder rows' size, with no gap above it: at the root it is level with `Back`, the left
/// column's first button; inside a folder the breadcrumb is over it.
#[test]
fn the_folder_title_is_in_the_folder_rows_size_level_with_back_at_the_root() {
    let mut rig = Rig::new(Tab::Songs);
    let content = shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m()).content;
    let back = Id::new("main_menu_songs").with(("side", 0_usize));
    let back = rig.ctx.read_response(back).expect("drawn").rect;
    let title = rig.find("My Songs");
    assert_eq!(font_size(&rig, "My Songs"), font_size(&rig, "Demo songs"));
    assert!(
        (title.center().y - back.center().y).abs() < 1.0,
        "{title:?} {back:?}"
    );
    assert!(title.top() < back.center().y && content.top() < title.top());
    rig.click_text("Demo songs");
    // The breadcrumb's line is at the content's top; the title follows it in the same size as at the root.
    let (crumb, title) = (rig.find("My Songs"), rig.find("Demo songs"));
    assert!(content.top() <= crumb.top() && crumb.bottom() <= title.top() + 1.0);
    assert_eq!(
        font_size(&rig, "Demo songs"),
        m().font_id(FontSize::Pt12).size
    );
}

/// One row size. Every row, folder or song, is in the title's 12 pt; a long name keeps its middle ellipsis and stops
/// before the big circles.
#[test]
fn every_row_is_in_12_pt_and_a_long_name_is_cut_before_the_big_circles() {
    let mut rig = Rig::new(Tab::Songs);
    let pt12 = m().font_id(FontSize::Pt12).size;
    for name in ["My Songs", "Demo songs", "Alpha.flm", "beta.flm"] {
        assert_eq!(font_size(&rig, name), pt12, "{name}");
    }
    let mut library = MemoryLibrary::default();
    let long = "A Very Long Song Name That Runs On Far Past The Big Circles Column Tail.flm";
    library.store(long, b"s").unwrap();
    rig.library = Box::new(library);
    rig.menu.refresh();
    rig.frame(vec![]);
    let texts = rig.texts();
    let (cut, at) = (texts.iter())
        .find(|(t, _)| t.starts_with("A Very"))
        .expect("the long name is drawn");
    assert!(cut.contains("...") && cut.ends_with("l.flm"), "{cut}");
    assert_eq!(font_size(&rig, cut), pt12);
    let content = shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m()).content;
    assert!(at.right() <= content.right() - 2.5 * 32.0, "{at:?}");
}

#[test]
fn a_song_opens_and_a_folder_goes_in_and_back_comes_out() {
    let mut rig = Rig::new(Tab::Songs);
    assert_eq!(rig.rows(), ["Demo songs", "Alpha.flm", "beta.flm"]);
    assert_eq!(
        rig.click_text("Alpha.flm"),
        Some(MainMenuAction::Open(PathBuf::from("/My Songs/Alpha.flm")))
    );
    // Back does nothing at the root.
    assert_eq!(rig.click_text("Back"), None);
    assert_eq!(rig.rows(), ["Demo songs", "Alpha.flm", "beta.flm"]);
    assert_eq!(rig.click_text("Demo songs"), None);
    assert_eq!(rig.rows(), ["Three.flm"]);
    // The breadcrumb names the root above the folder title.
    assert!(rig.find("My Songs").bottom() <= rig.find("Demo songs").top() + 1.0);
    assert_eq!(
        rig.click_text("Three.flm"),
        Some(MainMenuAction::Open(PathBuf::from(
            "/My Songs/Demo songs/Three.flm"
        )))
    );
    rig.click_text("Back");
    assert_eq!(rig.rows(), ["Demo songs", "Alpha.flm", "beta.flm"]);
    assert!(rig.has("My Songs"));
}

#[test]
fn ctrl_up_comes_out_of_a_folder_as_back_does() {
    // `leaveFolder`.
    let key = |pressed| Event::Key {
        key: Key::ArrowUp,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    };
    let ctrl_up = || [key(true), key(false)];
    let mut rig = Rig::new(Tab::Songs);
    rig.click_text("Demo songs");
    assert_eq!(rig.rows(), ["Three.flm"]);
    rig.frame(ctrl_up().into());
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["Demo songs", "Alpha.flm", "beta.flm"]);
    // At the root it does nothing.
    rig.frame(ctrl_up().into());
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["Demo songs", "Alpha.flm", "beta.flm"]);
    // While the search field has the keyboard, the key is the field's.
    rig.click_text("Demo songs");
    rig.click_text("Search");
    rig.frame(vec![]);
    assert!(rig.ctx.wants_keyboard_input());
    rig.frame(ctrl_up().into());
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["Three.flm"]);
}

#[test]
fn sort_and_search_change_the_rows() {
    let mut rig = Rig::new(Tab::Songs);
    rig.click_text("Sort");
    assert!(rig.has("Recent"));
    rig.click_text("Z-A");
    assert!(!rig.has("Recent"), "the popup closed");
    assert_eq!(rig.rows(), ["Demo songs", "beta.flm", "Alpha.flm"]);
    rig.click_text("Sort");
    rig.click_text("Recent");
    assert_eq!(rig.rows(), ["Demo songs", "beta.flm", "Alpha.flm"]);
    rig.click_text("Sort");
    rig.click_text("A-Z");
    assert_eq!(rig.rows(), ["Demo songs", "Alpha.flm", "beta.flm"]);

    // The search field takes the focus when it opens.
    rig.click_text("Search");
    rig.frame(vec![Event::Text("ALP".into())]);
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["Alpha.flm"]);
    // Closing the search shows everything again.
    rig.click_text("Search");
    assert_eq!(rig.rows(), ["Demo songs", "Alpha.flm", "beta.flm"]);
}

#[test]
fn in_select_mode_rows_toggle_and_select_all_selects_every_row() {
    let mut rig = Rig::new(Tab::Songs);
    rig.click_text("Select");
    assert_eq!(rig.click_text("Alpha.flm"), None);
    assert_eq!(
        rig.click_text("Demo songs"),
        None,
        "selects, does not go in"
    );
    assert_eq!(rig.rows(), ["Demo songs", "Alpha.flm", "beta.flm"]);
    let selection = |rig: &Rig| rig.menu.songs.selection.clone().unwrap();
    assert_eq!(selection(&rig).len(), 2);
    rig.click_text("Alpha.flm");
    assert_eq!(selection(&rig), [PathBuf::from("/My Songs/Demo songs")]);
    // ≡ is the last circle of the left column, under Select.
    let select = rig.find("Select").center();
    rig.click(select + vec2(0.0, 1.75 * 32.0));
    assert!(rig.has("Rename"), "a selection adds the file actions");
    rig.click_text("Select all");
    assert_eq!(selection(&rig).len(), 3);
    // Leaving select mode drops the selection; a click opens again.
    rig.click_text("Select");
    assert!(rig.menu.songs.selection.is_none());
    assert!(matches!(
        rig.click_text("beta.flm"),
        Some(MainMenuAction::Open(_))
    ));
}

#[test]
fn project_buttons_return_their_actions() {
    let mut rig = Rig::new(Tab::Project);
    assert_eq!(rig.click_text("Save"), Some(MainMenuAction::Save));
    assert_eq!(rig.click_text("Save New"), Some(MainMenuAction::SaveNew));
    assert_eq!(
        rig.click_text("Quick Render"),
        Some(MainMenuAction::QuickRender)
    );
    assert_eq!(rig.click_text("+"), Some(MainMenuAction::SetTempo(141.0)));
    assert_eq!(rig.click_text("140.0 BPM"), None);
    // `-` is also the unknown values' text: find the button left of the tempo value (3.5 U wide, a 0.25 U gap,
    // then the 1.25 U `-`).
    let minus = rig.find("140.0 BPM").center() - vec2(2.625 * 32.0, 0.0);
    assert_eq!(rig.click(minus), Some(MainMenuAction::SetTempo(139.0)));
    rig.info.time_spent_secs = 60.0;
    rig.frame(vec![]);
    assert_eq!(
        rig.click_text("Reset"),
        Some(MainMenuAction::ResetTimeSpent)
    );
    assert_eq!(rig.click_text("Hide"), Some(MainMenuAction::HideTimeSpent));
    rig.scroll_down();
    assert_eq!(
        rig.click_text("Lock metadata forever"),
        Some(MainMenuAction::LockMetadata)
    );
}

#[test]
fn project_texts_fit_their_meta_fields_in_bytes() {
    // char[256] and char[2048], terminated; the fields count characters.
    let mut rig = Rig::new(Tab::Project);
    rig.info.author = "é".repeat(200);
    rig.info.link = "ü".repeat(2000);
    rig.frame(vec![]);
    assert_eq!(rig.info.author, "é".repeat(127));
    assert_eq!(rig.info.link, "ü".repeat(1023));
}

#[test]
fn project_info_shows_the_values_it_is_given() {
    let mut rig = Rig::new(Tab::Project);
    let mut performance = Performance::default();
    (
        performance.channels,
        performance.active_modules,
        performance.smoothed,
    ) = (3, 3, 0.01);
    rig.info = ProjectInfo {
        length_secs: 2.4,
        time_spent_secs: 13.0 * 60.0 + 2.7,
        started_on: Some(1_791_676_503),
        sample_rate: Some(48_000),
        performance,
        ..ProjectInfo::default()
    };
    rig.frame(vec![]);
    for text in [
        "0m 2s",
        "13m 2s",
        "10 October 2026 23:55:03",
        "140.0 BPM",
        "48,000 Hz",
        "Channels: 3 | Active modules: 3 | Utilization:   1 %",
        "Press play to analyze performance",
    ] {
        assert!(rig.has(text), "{text}");
    }
    // The values start at 40 % of the content width.
    let content = shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m()).content;
    let value_x = content.left() + 0.4 * content.width();
    assert!((rig.find("13m 2s").left() - value_x).abs() < 1.0);
}

/// Which of `texts` the last frame drew, in paint order.
fn drawn(rig: &Rig, texts: &[&str]) -> Vec<String> {
    let mut all: Vec<_> = rig.texts().into_iter().map(|(t, _)| t).collect();
    all.retain(|t| texts.contains(&t.as_str()));
    all
}

#[test]
fn hide_and_lock_show_the_time_rows_and_buttons_as_screens_md_says() {
    // `Hide` iff unlocked and time spent ≥ 0, `Reset` iff unlocked and time spent > 0; time spent < 0 shows `Hidden`
    // in the three time rows, which a lock removes.
    let mut rig = Rig::new(Tab::Project);
    let (d0, d1) = ("10 October 2026 23:55:03", "11 October 2026 00:02:20");
    rig.info.started_on = Some(1_791_676_503);
    rig.info.last_saved_on = Some(1_791_676_940);
    rig.info.length_secs = 2.0;
    let rows = [
        "Time spent on project:",
        "Project started on:",
        "Project last saved on:",
    ];
    let mut check = |time: f64, locked: bool, buttons: &[&str], values: &[&str]| {
        (rig.info.time_spent_secs, rig.info.metadata_locked) = (time, locked);
        rig.frame(vec![]);
        let at = format!("{time} {locked}");
        assert_eq!(drawn(&rig, &["Hide", "Reset", "Show"]), buttons, "{at}");
        let all = ["Hidden", "0m 0s", "0m 5s", d0, d1];
        assert_eq!(drawn(&rig, &all), values, "{at}");
        let shown = drawn(&rig, &rows).len();
        assert_eq!(shown, if values.is_empty() { 0 } else { 3 }, "{at}");
    };
    check(0.0, false, &["Hide"], &["0m 0s", d0, d1]);
    check(5.0, false, &["Hide", "Reset"], &["0m 5s", d0, d1]);
    check(-1.0, false, &[], &["Hidden"; 3]);
    check(5.0, true, &[], &["0m 5s", d0, d1]);
    check(-1.0, true, &[], &[]);
}

#[test]
fn a_locked_song_keeps_its_texts_read_only_and_hides_the_empty_link_and_the_lock_button() {
    // Locked, Title / Author / Online link stay visible but read-only, the link row goes while the link is empty, and
    // the lock button goes. `Open` shows iff the link is non-empty, locked or not.
    let mut rig = Rig::new(Tab::Project);
    rig.scroll_down();
    assert!(rig.has("Online link:") && rig.has("Lock metadata forever"));
    assert!(!rig.has("Open"));
    rig.info.link = "example.com".into();
    rig.frame(vec![]);
    assert_eq!(
        rig.click_text("Open"),
        Some(MainMenuAction::OpenLink("https://example.com".into()))
    );
    // A link that is no web address still shows `Open` (it is non-empty), which then opens nothing.
    rig.info.link = "x:y/../../etc/passwd".into();
    rig.frame(vec![]);
    assert_eq!(rig.click_text("Open"), None);
    rig.info.link = "example.com".into();
    rig.info.metadata_locked = true;
    rig.frame(vec![]);
    assert!(rig.has("Online link:") && rig.has("Open"));
    assert!(!rig.has("Lock metadata forever"));
    rig.info.link.clear();
    rig.frame(vec![]);
    assert!(rig.has("Title:") && rig.has("Author:"));
    assert!(!rig.has("Online link:") && !rig.has("Open"));
}

#[test]
fn open_takes_only_a_web_address_with_a_host() {
    let ok = |link: &str, url: &str| assert_eq!(link_url(link).as_deref(), Some(url), "{link:?}");
    // No scheme: an https address.
    ok("example.com/a", "https://example.com/a");
    ok(" HTTP://x.test ", "HTTP://x.test");
    ok("https://x.test:8080/a?b#c", "https://x.test:8080/a?b#c");
    ok("192.168.0.1/x", "https://192.168.0.1/x");
    ok("localhost", "https://localhost");
    // Anything that is no http(s) address with a valid host opens nothing: the browser opener would take a link
    // that fails to parse as a local file path, and other schemes as themselves.
    for bad in [
        "",
        "javascript:alert(1)",
        "file:///etc/passwd",
        "x:y/../../../../../../etc/passwd",
        "http://[bad/../..",
        "http://[::1]/",
        "https://",
        "https:///etc/passwd",
        "http://user:pass@x.test/",
        "http://x.test:99999/",
        "http://x.test:/",
        "http://999.1.1.1/",
        "http://08.1.1.1/",
        "http://x.0x1/",
        "http://-x.test/",
        "http://x..test/",
        "http://x.test./",
        "http://bücher.de/",
        // Punycode labels may be invalid punycode, which `Url::parse` rejects: none are taken.
        "http://xn--a.test/",
        "http://a.XN--b.test/",
        "xn--nxasmq6b.com",
        "http://x.test\\..\\etc",
        "http://x.test/a b",
        "http://x.test/\u{7}",
        "ftp://x.test/",
    ] {
        assert_eq!(link_url(bad), None, "{bad:?}");
    }
}

#[test]
fn the_metadata_fields_edit_the_project_info_until_locked() {
    let mut rig = Rig::new(Tab::Project);
    // The title field is the first.
    let (_, title) = rig
        .texts()
        .into_iter()
        .find(|(t, _)| t == "Click to edit")
        .expect("drawn");
    rig.click(title.center());
    rig.frame(vec![Event::Text("My tune".into())]);
    assert_eq!(rig.info.title, "My tune");
    assert_eq!(rig.info.author, "");

    rig.info.metadata_locked = true;
    rig.frame(vec![]);
    let title = rig.find("My tune");
    rig.click(title.center());
    rig.frame(vec![Event::Text("!".into())]);
    assert_eq!(rig.info.title, "My tune");
}

#[test]
fn the_project_page_scrolls_to_the_performance_footer() {
    let mut rig = Rig::new(Tab::Project);
    let content = shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m()).content;
    let footer = "Physical memory: - | Memory used: - | App memory: -";
    assert!(
        rig.find(footer).top() > content.bottom(),
        "below the fold at first"
    );
    rig.scroll_down();
    let at = rig.find(footer);
    assert!(content.contains_rect(at), "{at:?} in {content:?}");
}

#[test]
fn outside_the_panel_the_logo_and_escape_close_the_menu() {
    let mut rig = Rig::new(Tab::Songs);
    assert_eq!(rig.click(pos2(100.0, 270.0)), Some(MainMenuAction::Close));
    let logo = shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m()).header;
    assert_eq!(
        rig.click(logo.left_center() + vec2(16.0, 0.0)),
        Some(MainMenuAction::Close)
    );
    let escape = || key_tap(Key::Escape, Modifiers::NONE);
    rig.frame(escape());
    assert_eq!(rig.action, Some(MainMenuAction::Close));
    // With a popup open, Escape closes the popup only.
    rig.click_text("Sort");
    rig.frame(escape());
    assert_eq!(rig.action, None);
    rig.frame(vec![]);
    assert!(!rig.has("Recent"));
    // Inside the panel, an empty spot does nothing.
    assert_eq!(rig.click(pos2(1100.0, 500.0)), None);
}

#[test]
fn quit_returns_its_action_and_help_is_disabled() {
    let mut rig = Rig::new(Tab::Project);
    assert_eq!(rig.click_text("Help"), None);
    assert_eq!(rig.click_text("Quit"), Some(MainMenuAction::Quit));
}

#[test]
fn the_settings_tab_edits_the_settings_it_is_given() {
    let mut rig = Rig::new(Tab::Settings);
    // `Multi core processing`, the settings tab's row 5.
    let toggle = rig
        .ctx
        .read_response(Id::new("settings_tab").with(5_usize))
        .expect("drawn")
        .rect;
    assert!(
        shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m())
            .content
            .contains_rect(toggle)
    );
    rig.click(toggle.center());
    assert!(rig.settings.1.multi_core);
}

#[test]
fn song_names_are_clipped_to_the_list_they_scroll_in() {
    // `text_label` is plain text and clips nothing, so the list does.
    let rig = Rig::new(Tab::Songs);
    let content = shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m()).content;
    let list_top = content.top() + 1.75 * 32.0;
    let clips: Vec<_> = rig
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text().ends_with(".flm") => Some(c.clip_rect),
            _ => None,
        })
        .collect();
    assert_eq!(clips.len(), 2);
    for clip in clips {
        assert!(
            clip.top() >= list_top && content.contains_rect(clip),
            "{clip:?}"
        );
    }
}

/// A row scrolled half out of the list keeps its name centred in the whole row (the list's clip cuts it), not in the
/// part that shows: the names keep their spacing as they scroll.
#[test]
fn a_half_scrolled_rows_name_stays_centred_in_its_row() {
    let mut library = MemoryLibrary::default();
    for i in 0..30 {
        library.store(&format!("Song {i:02}.flm"), b"s").unwrap();
    }
    let mut rig = Rig::new(Tab::Songs);
    rig.library = Box::new(library);
    rig.menu.refresh();
    rig.frame(vec![]);
    let content = shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m()).content;
    // 13 points: not a whole row.
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -13.0),
        modifiers: Default::default(),
    };
    rig.frame(vec![Event::PointerMoved(content.center()), wheel]);
    for _ in 0..60 {
        rig.frame(vec![]);
    }
    let mut rows: Vec<Rect> = (rig.texts().into_iter())
        .filter(|(t, _)| t.ends_with(".flm"))
        .map(|(_, r)| r)
        .collect();
    rows.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let pitch = rows[2].center().y - rows[1].center().y;
    for pair in rows.windows(2) {
        let gap = pair[1].center().y - pair[0].center().y;
        assert!((gap - pitch).abs() < 0.01, "{gap} not {pitch}: {rows:?}");
    }
}

#[test]
fn the_tab_row_has_equal_cells_and_labels_centred_in_them_that_overflow_evenly() {
    let mut rig = Rig::new(Tab::Settings);
    let factor = |f: f32| {
        crate::metrics::UI_SCALE_FACTORS
            .iter()
            .position(|&x| x == f)
            .unwrap()
    };
    let mut widths_at_200 = Vec::new();
    for (scale, width) in [(1.0, 1170.0), (1.5, 1170.0), (2.0, 1170.0), (2.0, 360.0)] {
        let case = format!("{scale} on {width}");
        rig.screen = vec2(width, 540.0);
        rig.settings.1.ui_scale_index = factor(scale);
        rig.frame(vec![]);
        rig.frame(vec![]);
        let screen = Rect::from_min_size(Pos2::ZERO, rig.screen);
        let l = shell_layout(screen, &Metrics::from_pixels_per_point(1.0, factor(scale)));
        let u = 32.0 * scale;
        assert_eq!(l.tabs.top(), l.header.bottom(), "{case}");
        assert_eq!(l.header.height(), 1.75 * u, "{case}");
        assert_eq!(l.tabs.height(), 1.25 * u, "{case}");
        assert_eq!(l.tabs.x_range(), l.panel.x_range(), "{case}");
        let cell_w = l.panel.width() / Tab::ALL.len() as f32;
        let mut labels = Vec::new();
        for (i, tab) in Tab::ALL.into_iter().enumerate() {
            let cell = rig
                .ctx
                .read_response(Id::new("main_menu").with(("tab", i)))
                .expect("drawn")
                .rect;
            let left = l.tabs.left() + i as f32 * cell_w;
            let expected = Rect::from_x_y_ranges(left..=left + cell_w, l.tabs.y_range());
            assert!(
                (cell.min - expected.min).length() < 0.01
                    && (cell.max - expected.max).length() < 0.01,
                "{case}: {tab:?} {cell:?} != {expected:?}"
            );
            let (label, clip) = rig
                .shapes
                .iter()
                .find_map(|c| match &c.shape {
                    egui::Shape::Text(t) if t.galley.text() == tab.label() => {
                        Some((t.galley.rect.translate(t.pos.to_vec2()), c.clip_rect))
                    }
                    _ => None,
                })
                .expect("drawn");
            // Centred in its cell, overflowing it evenly; plain text, clipped by no cell (only by the screen).
            assert!(
                (label.center().x - cell.center().x).abs() < 0.5
                    && (label.center().y - cell.center().y).abs() < 1.0,
                "{case}: {tab:?} {label:?} not centred in {cell:?}"
            );
            assert!(
                clip.contains_rect(label.intersect(screen)),
                "{case}: {tab:?} clipped to {clip:?}"
            );
            labels.push(label);
        }
        if width == 1170.0 {
            for pair in labels.windows(2) {
                assert!(pair[0].right() <= pair[1].left(), "{case}: {pair:?}");
            }
        }
        if scale == 2.0 {
            widths_at_200.push(labels.iter().map(|l| l.width()).collect::<Vec<_>>());
        }
        if width == 360.0 {
            // 10 pt at 200 %, never shrunk: `SETTINGS` is wider than its cell and runs into its neighbours.
            assert!(labels[2].width() > cell_w, "{labels:?}");
            assert!(labels[2].left() < labels[1].right(), "{labels:?}");
        }
    }
    assert_eq!(widths_at_200[0], widths_at_200[1], "the labels shrank");
}

/// SONGS on an in-memory library holding `A.flm` and `B.flm`, in select mode.
fn songs_rig() -> Rig {
    let mut library = MemoryLibrary::default();
    library.store("A.flm", b"a").unwrap();
    library.store("B.flm", b"b").unwrap();
    let mut rig = Rig::new(Tab::Songs);
    rig.library = Box::new(library);
    rig.menu.refresh();
    rig.frame(vec![]);
    rig.click_text("Select");
    rig
}

impl Rig {
    /// Opens the ≡ menu, the last circle of the left column, under `Select`.
    fn options(&mut self) {
        let select = self.find("Select").center();
        self.click(select + vec2(0.0, 1.75 * 32.0));
    }

    /// Picks `item` from the ≡ menu; returns its action.
    fn pick(&mut self, item: &str) -> Option<MainMenuAction> {
        self.options();
        self.click_text(item)
    }

    /// The root's names, sorted.
    fn names(&self, folder: Option<&Path>) -> Vec<String> {
        let mut names: Vec<_> = self
            .library
            .list(folder)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        names
    }

    /// Presses and releases `key`.
    fn key(&mut self, key: Key) {
        self.frame(key_tap(key, Modifiers::NONE));
        self.frame(vec![]);
    }
}

#[test]
fn copy_paste_delete_and_restore_from_the_options_menu() {
    let mut rig = songs_rig();
    rig.click_text("A.flm");
    rig.pick("Copy");
    // Paste into the same folder keeps both.
    assert_eq!(rig.pick("Paste"), None);
    assert_eq!(rig.names(None), ["A (1).flm", "A.flm", "B.flm"]);
    assert!(rig.has("A (1).flm"), "the list is read again");
    // Delete moves into Recently Deleted, which the root then lists.
    rig.click_text("A (1).flm");
    assert_eq!(rig.pick("Delete"), None);
    assert_eq!(rig.names(None), ["A.flm", "B.flm", "Recently Deleted"]);
    assert!(!rig.has("A (1).flm"));
    // Recently Deleted is a place, not an item: a click goes in even in select mode, and Select all skips it.
    rig.pick("Select all");
    assert_eq!(rig.menu.songs.selection.as_ref().unwrap().len(), 2);
    rig.pick("Select all");
    rig.click_text("Recently Deleted");
    assert!(rig.has("A (1).flm"));
    assert!(rig.has("Recently Deleted (automatically deleted after 30 days)"));
    rig.options();
    assert!(rig.has("Restore") && !rig.has("Create folder"));
    rig.key(Key::Escape);
    rig.click_text("A (1).flm");
    assert_eq!(rig.pick("Restore"), None);
    assert!(!rig.has("A (1).flm"));
    rig.click_text("Back");
    assert_eq!(
        rig.names(None),
        ["A (1).flm", "A.flm", "B.flm", "Recently Deleted"]
    );
}

#[test]
fn a_long_bin_breadcrumb_is_shortened_in_the_middle_keeping_its_end() {
    const NOTE: &str = "Recently Deleted (automatically deleted after 30 days)";
    const LONG: &str = "Songs from the long summer of the year before last";
    let mut library = MemoryLibrary::default();
    let trash = library.trash().unwrap();
    for dir in [
        trash.clone(),
        trash.join(LONG),
        trash.join(LONG).join("Inner"),
    ] {
        library.make_folder(&dir).unwrap();
    }
    let mut rig = Rig::new(Tab::Songs);
    rig.library = Box::new(library);
    rig.menu.refresh();
    rig.frame(vec![]);
    let crumb = |rig: &Rig| {
        let hit = |t: &str| t.starts_with("Recently Deleted (");
        let found = rig.texts().into_iter().find(|(t, _)| hit(t));
        found.unwrap_or_else(|| panic!("no breadcrumb: {:?}", rig.texts()))
    };
    // The note alone fits whole, above the bin's name.
    rig.click_text("Recently Deleted");
    assert_eq!(crumb(&rig).0, NOTE);
    // With a folder in the trail it is too wide for the line: head, `...` and the end of the trail, within the line.
    // (The folder's own row is shortened too.)
    let row = rig
        .texts()
        .into_iter()
        .find(|(t, _)| t.starts_with("Songs from"));
    rig.click(row.expect("the folder's row").1.center());
    rig.click_text("Inner");
    let (shown, rect) = crumb(&rig);
    let full = format!("{NOTE} / {LONG}");
    let (head, tail) = shown.split_once("...").unwrap_or_else(|| panic!("{shown}"));
    assert!(full.starts_with(head) && full.ends_with(tail), "{shown}");
    // The nearest folder stays readable: a good part of its name is kept, not 5 characters.
    assert!(
        head.len() > NOTE.len() / 2 && tail.chars().count() >= 20,
        "{shown}"
    );
    // The line ends where the big circles' column starts (2.5 U from the content's edge).
    assert!(rect.right() <= 1154.0 - 2.5 * 32.0 + 1.0, "{rect:?}");
}

/// The folder title under the breadcrumb and the list's rows are truncated drawing too (K = 5 as the browsers'
/// titles): a long name keeps its head, `...` and its last characters, and stops before the big circles.
#[test]
fn a_long_folder_name_is_shortened_in_the_middle_in_its_row_and_title_before_the_big_circles() {
    const LONG: &str = "Songs from the long summer of the year before last, the winter after it, and the spring \
                        that came much too late for any of them";
    let mut library = MemoryLibrary::default();
    library.make_folder(Path::new(LONG)).unwrap();
    let mut rig = Rig::new(Tab::Songs);
    rig.library = Box::new(library);
    rig.menu.refresh();
    rig.frame(vec![]);
    // The one shortened text: the row at the root, then the title of the folder it opens (which lists nothing).
    let shortened = |rig: &Rig| {
        let found = rig.texts().into_iter().find(|(t, _)| t.contains("..."));
        let (shown, rect) = found.unwrap_or_else(|| panic!("nothing short: {:?}", rig.texts()));
        let (head, tail) = shown.split_once("...").unwrap();
        assert!(LONG.starts_with(head) && head.len() > 5, "{shown}");
        assert_eq!(tail, &LONG[LONG.len() - 5..], "{shown}");
        assert!(rect.right() <= 1154.0 - 2.5 * 32.0 + 1.0, "{rect:?}");
        rect
    };
    let row = shortened(&rig);
    rig.click(row.center());
    let title = shortened(&rig);
    assert!(title.top() < row.top(), "the title, above the list");
}

#[test]
fn cut_moves_into_a_folder_made_by_create_folder() {
    let mut rig = songs_rig();
    rig.pick("Create folder");
    assert!(rig.has("Enter folder name"));
    rig.frame(vec![Event::Text("Beats".into())]);
    rig.key(Key::Enter);
    assert!(!rig.has("Enter folder name"));
    assert!(rig.has("Beats"));
    rig.click_text("B.flm");
    rig.pick("Cut");
    rig.click_text("Select");
    rig.click_text("Beats");
    rig.click_text("Select");
    assert_eq!(rig.pick("Paste"), None);
    assert!(rig.has("B.flm"));
    assert_eq!(rig.names(Some(Path::new("Beats"))), ["B.flm"]);
    assert_eq!(rig.names(None), ["A.flm", "Beats"]);
    // A cut is pasted once: Paste is disabled now (pasting the moved song again would fail with a message).
    rig.options();
    assert_eq!(rig.click_text("Paste"), None);
    rig.key(Key::Escape);
    assert_eq!(rig.names(Some(Path::new("Beats"))), ["B.flm"]);
}

#[test]
fn rename_asks_for_the_name_and_refuses_a_taken_one() {
    let mut rig = songs_rig();
    rig.click_text("A.flm");
    rig.pick("Rename");
    assert!(rig.has("Enter name"));
    // The field holds the name without its extension.
    assert!(rig.has("A"));
    rig.key(Key::Backspace);
    rig.frame(vec![Event::Text("Song".into())]);
    rig.key(Key::Enter);
    assert_eq!(rig.names(None), ["B.flm", "Song.flm"]);
    // A taken name: the message, nothing renamed.
    rig.click_text("Song.flm");
    rig.pick("Rename");
    for _ in "Song".chars() {
        rig.key(Key::Backspace);
    }
    rig.frame(vec![Event::Text("B".into())]);
    rig.frame(vec![Event::Key {
        key: Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert_eq!(
        rig.action,
        Some(MainMenuAction::Message("The file already exists".into()))
    );
    assert_eq!(rig.names(None), ["B.flm", "Song.flm"]);
    // Escape drops the field and leaves the menu open.
    rig.pick("Create folder");
    rig.frame(vec![Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert_eq!(rig.action, None);
    rig.frame(vec![]);
    assert!(!rig.has("Enter folder name"));
}

#[test]
fn delete_permanently_asks_first_and_removes_on_yes_only() {
    let mut rig = songs_rig();
    rig.click_text("A.flm");
    rig.click_text("B.flm");
    rig.pick("Delete");
    rig.click_text("Recently Deleted");
    rig.pick("Select all");
    assert_eq!(
        rig.pick("Delete permanently"),
        Some(MainMenuAction::ConfirmDelete("Delete 2 files?".into()))
    );
    assert_eq!(rig.menu.delete_answered(false, &mut *rig.library), None);
    assert_eq!(
        rig.names(rig.library.trash().as_deref()),
        ["A.flm", "B.flm"]
    );
    rig.pick("Select all");
    rig.pick("Delete permanently");
    assert_eq!(rig.menu.delete_answered(true, &mut *rig.library), None);
    assert!(rig.names(rig.library.trash().as_deref()).is_empty());
    rig.frame(vec![]);
    assert!(!rig.has("A.flm") && !rig.has("B.flm"));
}

// The main menu panel's animations.

/// The panel's `menuFill` rect and the screen dim's alpha, from the last frame.
fn panel_and_dim(rig: &Rig) -> (Rect, f32) {
    let colors = theme_by_id(0).colors;
    let (fill, bg) = (
        colors.menu_fill.to_color32(),
        colors.background.to_color32(),
    );
    let screen = Rect::from_min_size(Pos2::ZERO, SIZE.into());
    let mut panel = None;
    let mut dim = 0.0;
    for c in &rig.shapes {
        if let egui::Shape::Rect(r) = &c.shape {
            if r.fill == fill && r.rect.height() == screen.height() {
                panel = Some(r.rect);
            }
            if r.rect == screen
                && r.fill.r() == (f32::from(bg.r()) * r.fill.a() as f32 / 255.0).round() as u8
            {
                dim = f32::from(r.fill.a()) / 255.0;
            }
        }
    }
    (panel.expect("the panel"), dim)
}

#[test]
fn the_menu_slides_in_from_the_right_over_10_frames_as_the_screen_dims() {
    let mut rig = Rig::new(Tab::Project);
    crate::ui::anim::enable(&rig.ctx);
    let w = 640.0;
    for n in 1..=12u32 {
        rig.frame(vec![]);
        let u4 = crate::ui::anim::menu_slide(n).powi(4);
        let (panel, dim) = panel_and_dim(&rig);
        assert_eq!(panel.left(), 530.0 + (u4 * w).trunc(), "frame {n}");
        assert!((dim - 0.6 * (1.0 - u4)).abs() < 0.01, "frame {n}: {dim}");
    }
    // Frame 1 is 0.9⁴ = 65.6 % of the way out; frame 10 is home.
    assert_eq!((0.9f32.powi(4) * w).trunc(), 419.0);
}

#[test]
fn a_tab_change_slides_only_the_page_in_again() {
    // Where PROJECT's page and tab row sit at rest.
    let mut rig = Rig::new(Tab::Songs);
    crate::ui::anim::enable(&rig.ctx);
    for _ in 0..12 {
        rig.frame(vec![]);
    }
    let home = {
        let mut r = Rig::new(Tab::Project);
        r.frame(vec![]);
        r.texts()
    };
    let label =
        |texts: &[(String, Rect)], t: &str| texts.iter().find(|(s, _)| s == t).map(|(_, r)| *r);
    let at = rig.find("PROJECT").center();
    rig.frame(vec![Event::PointerMoved(at)]);
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    rig.frame(vec![button(true)]);
    rig.frame(vec![button(false)]);
    // The release frame changed the tab: from the next one the new page slides in by trunc(u⁴·W).
    for n in 1..=11u32 {
        rig.frame(vec![]);
        let dx = (crate::ui::anim::menu_slide(n).powi(4) * 640.0).trunc();
        let texts = rig.texts();
        assert_eq!(
            label(&texts, "PROJECT"),
            label(&home, "PROJECT"),
            "frame {n}: the tab row stays"
        );
        let (page, want) = (label(&texts, "Tempo:"), label(&home, "Tempo:"));
        let (page, want) = (page.expect("Tempo:"), want.unwrap());
        assert_eq!(page.left(), want.left() + dx, "frame {n}");
        assert_eq!(
            panel_and_dim(&rig).0.left(),
            530.0,
            "frame {n}: the panel stays"
        );
    }
}

#[test]
fn the_menu_takes_touches_at_rest_while_it_slides_in() {
    // Touch handlers never check the animation state. A tap on PROJECT's rest position in the first frames picks the
    // tab (it does not land left of the sliding panel and close it).
    let at = Rig::new(Tab::Songs).find("PROJECT").center();
    let mut rig = Rig::new(Tab::Songs);
    rig.ctx = Context::default();
    crate::ui::anim::enable(&rig.ctx);
    rig.frame(vec![]);
    assert_eq!(rig.click(at), None, "no Close");
    assert_eq!(rig.menu.tab, Tab::Project);
    // Meanwhile it is drawn where the slide has it: frame 2's tab row is trunc(0.8⁴·640) = 262 points right.
    let mut rig = Rig::new(Tab::Songs);
    rig.ctx = Context::default();
    crate::ui::anim::enable(&rig.ctx);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(rig.find("PROJECT").center().x, at.x + 262.0);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn original_demo_songs_list_at_the_root_and_new_offers_the_original_templates() {
    use crate::project_files::CONTENT_FOLDER;
    use crate::project_files::content::tests::{Temp, fake_tree};
    let tmp = Temp::new("songs-content");
    let songs = tmp.0.join(SONGS_FOLDER);
    let mut rig = Rig::new(Tab::Songs);
    rig.library = Box::new(NativeLibrary::at(songs.clone()));
    rig.menu.refresh();
    rig.frame(vec![]);
    // Without content: no demo folder, and `New` asks for the empty song's options at once.
    assert_eq!(rig.rows(), Vec::<String>::new());
    assert_eq!(rig.click_text("New"), None);
    assert!(rig.has(TITLE));
    rig.key(Key::Escape);
    assert!(!rig.has(TITLE));
    fake_tree(&tmp.0.join(CONTENT_FOLDER));
    rig.menu.refresh();
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["Demo Songs"]);
    assert_eq!(rig.click_text("Demo Songs"), None);
    assert_eq!(rig.rows(), ["Gen Demo.flm"]);
    let demo = tmp
        .0
        .join(CONTENT_FOLDER)
        .join("Songs/Demo Songs/Gen Demo.flm");
    assert_eq!(
        rig.click_text("Gen Demo.flm"),
        Some(MainMenuAction::Open(demo))
    );
    // `New`: `Empty`, the `Templates` header, then the templates in byte order.
    assert_eq!(rig.click_text("New"), None);
    assert!(rig.has("Empty") && rig.has("Templates"));
    assert!(rig.find("Templates").top() < rig.find("Gen Ambient").top());
    assert!(rig.find("Gen Ambient").top() < rig.find("Gen Beat").top());
    let template = tmp
        .0
        .join(CONTENT_FOLDER)
        .join("New Song Templates/Gen Beat.songtmpl");
    assert_eq!(
        rig.click_text("Gen Beat"),
        Some(MainMenuAction::NewFromTemplate(template))
    );
    assert_eq!(rig.click_text("New"), None);
    assert_eq!(rig.click_text("Empty"), None);
    assert_eq!(
        rig.click_text("OK"),
        Some(MainMenuAction::NewSong(SongOptions::default()))
    );
    // In `Demo Songs` the ≡ items that would change it are dimmed and inert: no failure message, no name field.
    rig.click_text("Select");
    rig.click_text("Gen Demo.flm");
    for item in ["Delete", "Rename", "Cut", "Create folder"] {
        rig.options();
        assert_eq!(rig.click_text(item), None, "{item}");
        assert!(
            !rig.has("Enter name") && !rig.has("Enter folder name"),
            "{item}"
        );
        rig.key(Key::Escape);
    }
    let demo = tmp
        .0
        .join(CONTENT_FOLDER)
        .join("Songs/Demo Songs/Gen Demo.flm");
    assert!(demo.exists());
}

// ---- PROJECT Performance ----

/// The one text `text` the last frame painted: its rect and its colour and font.
fn text_of(rig: &Rig, text: &str) -> Option<(Rect, Color32, egui::FontId)> {
    rig.shapes.iter().find_map(|c| match &c.shape {
        Shape::Text(t) if t.galley.text() == text => {
            let format = &t.galley.job.sections[0].format;
            Some((
                t.galley.rect.translate(t.pos.to_vec2()),
                format.color,
                format.font_id.clone(),
            ))
        }
        _ => None,
    })
}

/// The discs and rings the last frame painted: (centre, radius, fill, stroke width, stroke colour).
fn circles(rig: &Rig) -> Vec<(Pos2, f32, Color32, f32, Color32)> {
    rig.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Circle(s) => Some((s.center, s.radius, s.fill, s.stroke.width, s.stroke.color)),
            _ => None,
        })
        .collect()
}

/// The page's facts at `ppp`: the metrics, the menu panel and the line height of FontSize 2.
fn page(rig: &Rig, ppp: f32) -> (Metrics, Rect, f32) {
    let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
    let panel = shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m).panel;
    let fh = rig
        .ctx
        .fonts_mut(|f| f.row_height(&m.font_id(FontSize::Pt10)));
    (m, panel, fh)
}

#[test]
fn the_performance_block_is_one_5_u_block_2_u_in_from_the_page_and_1_3_u_under_its_header() {
    // x = page left + 2 U, width = page width - 4 U, top = the header row + 1.3 U, 5.0 U tall; the status line is at
    // its top and the footer at its bottom, both FontSize 2.
    for ppp in [1.0, 2.0] {
        let mut rig = Rig::at_ppp(Tab::Project, ppp);
        rig.scroll_down();
        let (m, panel, fh) = page(&rig, ppp);
        let u = m.u(1.0);
        let (header, ..) = text_of(&rig, "Performance").expect("the header");
        let status = "Channels: 0 | Active modules: 0 | Utilization:   0 %";
        let (status, _, status_font) = text_of(&rig, status).expect("the status line");
        let (footer, _, footer_font) = text_of(&rig, &footer_line("-", None)).expect("the footer");
        let at = format!("ppp {ppp}");
        // The header row is 1.3 U: the block starts where it ends, and its status line fills the block's first line (a
        // text is centred in its line, so the block's top is the status text's centre less half a line).
        let top = status.center().y - fh / 2.0;
        assert!(
            (top - (header.center().y + 0.65 * u)).abs() < 0.3,
            "{at}: {top} vs {}",
            header.center().y + 0.65 * u
        );
        assert!((status.height() - fh).abs() < 1.0, "{at}");
        assert!(
            (footer.center().y + fh / 2.0 - (top + 5.0 * u)).abs() < 0.3,
            "{at}: {}",
            footer.center().y + fh / 2.0
        );
        assert_eq!(
            (status_font.clone(), footer_font),
            (m.font_id(FontSize::Pt10), m.font_id(FontSize::Pt10))
        );
        // The block is 2 U in from the panel's sides (the bar, `background`, spans its width).
        let colors = theme_by_id(0).colors;
        let bar = rig
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Rect(r) if r.fill == colors.background.to_color32() => Some(r.rect),
                _ => None,
            })
            .find(|r| (r.height() - 2.0 * m.t()).abs() < 0.01)
            .expect("the load bar");
        assert!(
            (bar.left() - (panel.left() + 2.0 * u)).abs() < 0.01,
            "{at}: {bar:?}"
        );
        assert!(
            (bar.right() - (panel.right() - 2.0 * u)).abs() < 0.01,
            "{at}: {bar:?}"
        );
        // The bar is at y + fh + t.
        assert!(
            (bar.top() - (top + fh + m.t())).abs() < 1.0,
            "{at}: {} vs {}",
            bar.top(),
            top + fh + m.t()
        );
        assert!(
            (status.left() - bar.left()).abs() < 1.0 && (footer.left() - bar.left()).abs() < 1.0,
            "{at}"
        );
        // The graph box starts at y + fh + 8t and runs to h - fh - t.
        let graph = rig
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Rect(r) if r.fill == colors.background.to_color32() => Some(r.rect),
                _ => None,
            })
            .find(|r| r.height() > 10.0 * m.t() && r.width() > 2.0 * u && r.top() > bar.bottom())
            .expect("the graph box");
        let want = Rect::from_min_max(
            pos2(bar.left(), top + fh + 8.0 * m.t()),
            pos2(bar.right(), top + 5.0 * u - fh - m.t()),
        );
        assert!(
            (graph.min - want.min).abs().max_elem() < 1.0,
            "{at}: {graph:?} {want:?}"
        );
        assert!(
            (graph.max - want.max).abs().max_elem() < 1.0,
            "{at}: {graph:?} {want:?}"
        );
    }
}

#[test]
fn detach_hide_and_reset_are_text_discs_of_radius_0_7_u_with_a_module_fill_rim_and_label() {
    // Each is a `text` disc of radius 0.7 U with a 2 px `moduleFill` rim, its label FontSize 1 (8 pt) in `moduleFill`.
    // `Detach` is centred on the block's right edge, 0.5 U under its top.
    let colors = theme_by_id(0).colors;
    let (text, module_fill) = (colors.text.to_color32(), colors.module_fill.to_color32());
    for ppp in [1.0, 2.0] {
        let mut rig = Rig::at_ppp(Tab::Project, ppp);
        rig.info.time_spent_secs = 5.0;
        rig.frame(vec![]);
        let (m, panel, _) = page(&rig, ppp);
        let u = m.u(1.0);
        let at = format!("ppp {ppp}");
        let styled = |rig: &Rig, label: &str| {
            let (rect, ink, font) =
                text_of(rig, label).unwrap_or_else(|| panic!("{at}: no {label}"));
            assert_eq!(
                (ink, font),
                (module_fill, m.font_id(FontSize::Pt8)),
                "{at}: {label}"
            );
            let discs = circles(rig);
            let disc = discs
                .iter()
                .find(|d| d.1 > 0.6 * u && d.2 == text && (d.0 - rect.center()).length() < 2.0)
                .unwrap_or_else(|| panic!("{at}: no `text` disc under {label}: {discs:?}"));
            assert!(
                (disc.1 - 0.7 * u).abs() < 0.01,
                "{at}: {label} radius {}",
                disc.1
            );
            let rim = discs
                .iter()
                .find(|d| d.2 == Color32::TRANSPARENT && (d.0 - disc.0).length() < 0.01);
            let rim = rim.unwrap_or_else(|| panic!("{at}: no rim on {label}"));
            assert_eq!(
                (rim.3, rim.4),
                (2.0 / ppp, module_fill),
                "{at}: {label} rim"
            );
            disc.0
        };
        // `Hide` and `Reset`, on the time row.
        for label in ["Hide", "Reset"] {
            styled(&rig, label);
        }
        // `Detach`, once the page is scrolled to it.
        rig.scroll_down();
        let centre = styled(&rig, "Detach");
        let (header, ..) = text_of(&rig, "Performance").unwrap();
        let want = pos2(
            panel.right() - 2.0 * u,
            header.center().y + 0.65 * u + 0.5 * u,
        );
        assert!(
            (centre - want).length() < 1.0,
            "{at}: {centre:?} vs {want:?}"
        );
    }
}

#[test]
fn detach_takes_a_tap_in_its_2_u_box_and_not_outside_it() {
    // The button's touch area is a 2 U × 2 U box at (column right − 1 U, monitor top − 0.5 U); the disc is 1.4 U.
    for ppp in [1.0, 2.0] {
        let mut rig = Rig::at_ppp(Tab::Project, ppp);
        rig.scroll_down();
        let (m, ..) = page(&rig, ppp);
        let u = m.u(1.0);
        let centre = text_of(&rig, "Detach").expect("the button").0.center();
        // In a corner of the box, well off the disc (1.27 U from its centre).
        let corner = centre + vec2(0.9 * u, 0.9 * u);
        assert_eq!(
            rig.click(corner),
            Some(MainMenuAction::DetachPerformance),
            "ppp {ppp}"
        );
        let corner = centre + vec2(-0.9 * u, -0.9 * u);
        assert_eq!(
            rig.click(corner),
            Some(MainMenuAction::DetachPerformance),
            "ppp {ppp}"
        );
        // Just outside the box.
        for away in [vec2(1.1 * u, 0.0), vec2(0.0, 1.1 * u), vec2(-1.1 * u, 0.0)] {
            assert_eq!(rig.click(centre + away), None, "ppp {ppp}: {away:?}");
        }
    }
}

#[test]
fn the_panel_is_menu_fill_with_a_module_fill_header_and_a_menu_fill_tab_row() {
    // The body is `menuFill`, the full panel height, behind every page; the header (1.75 U) `moduleFill`; the tab row
    // (1.25 U) `menuFill`.
    let colors = theme_by_id(0).colors;
    let (menu_fill, module_fill) = (
        colors.menu_fill.to_color32(),
        colors.module_fill.to_color32(),
    );
    for ppp in [1.0, 2.0] {
        let rig = Rig::at_ppp(Tab::Songs, ppp);
        let (m, panel, _) = page(&rig, ppp);
        let fills: Vec<(Rect, Color32)> = rig
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Rect(r) => Some((r.rect, r.fill)),
                _ => None,
            })
            .collect();
        let header =
            Rect::from_x_y_ranges(panel.x_range(), panel.top()..=panel.top() + 1.75 * m.u(1.0));
        assert!(fills.contains(&(panel, menu_fill)), "ppp {ppp}: the body");
        assert!(
            fills.contains(&(header, module_fill)),
            "ppp {ppp}: the header"
        );
        // The tab row has no fill of its own: across the panel there is only the body's, behind it.
        let tabs = Rect::from_x_y_ranges(
            panel.x_range(),
            header.bottom()..=header.bottom() + 1.25 * m.u(1.0),
        );
        for (r, f) in &fills {
            let across = r.left() >= panel.left() - 0.01 && r.width() > 0.9 * panel.width();
            assert!(
                !(across && r.intersects(tabs) && *r != panel && *r != header),
                "ppp {ppp}: a fill {r:?} {f:?} in the tab row"
            );
        }
    }
}

#[test]
fn detach_becomes_attach_while_the_monitor_shows_and_the_block_stays_in_both_states() {
    let mut rig = Rig::new(Tab::Project);
    rig.scroll_down();
    assert!(rig.has("Detach") && !rig.has("Attach") && rig.has(IDLE_TEXT));
    assert_eq!(
        rig.click_text("Detach"),
        Some(MainMenuAction::DetachPerformance)
    );
    // Shown: the page offers `Attach`, which hides it and leaves the menu to the app (no close).
    rig.info.performance.detached = true;
    rig.frame(vec![]);
    assert!(rig.has("Attach") && !rig.has("Detach") && rig.has(IDLE_TEXT));
    assert_eq!(
        rig.click_text("Attach"),
        Some(MainMenuAction::AttachPerformance)
    );
}

#[test]
fn the_cpu_and_ram_tap_scrolls_the_performance_block_into_view() {
    for ppp in [1.0, 2.0] {
        let mut rig = Rig::at_ppp(Tab::Project, ppp);
        let content = shell_layout(
            Rect::from_min_size(Pos2::ZERO, SIZE.into()),
            &page(&rig, ppp).0,
        )
        .content;
        assert!(
            rig.find(&footer_line("-", None)).top() > content.bottom(),
            "ppp {ppp}: below the fold at first"
        );
        rig.menu.show_performance();
        rig.frame(vec![]);
        rig.frame(vec![]);
        let (footer, status) = (
            rig.find(&footer_line("-", None)),
            rig.find("Channels: 0 | Active modules: 0 | Utilization:   0 %"),
        );
        // (Within a pixel: the page's offset is snapped to device pixels.)
        let view = content.expand(1.0);
        assert!(
            view.contains_rect(footer) && view.contains_rect(status),
            "ppp {ppp}: {footer:?} in {content:?}"
        );
        // Already in view: a second tap leaves the page where it is.
        rig.menu.show_performance();
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert_eq!(rig.find(&footer_line("-", None)), footer, "ppp {ppp}");
    }
}

/// Opens SONGS `New`'s menu in a rig whose factory has the templates `names`.
fn new_menu_rig(names: &[&str], ppp: f32) -> Rig {
    let mut rig = Rig::at_ppp(Tab::Songs, ppp);
    rig.menu.set_factory(factory_of(names));
    assert_eq!(rig.click_text("New"), None);
    rig
}

/// The popup's rows, top to bottom.
fn menu_rows(rig: &Rig, labels: &[&str]) -> Vec<String> {
    let mut rows: Vec<_> = (rig.texts().into_iter())
        .filter(|(t, _)| labels.contains(&t.as_str()))
        .collect();
    rows.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
    rows.into_iter().map(|(t, _)| t).collect()
}

const THREE: [&str; 3] = ["Drill", "D&B", "Drift Phonk"];
const MENU: [&str; 5] = ["Empty", "Templates", "D&B", "Drift Phonk", "Drill"];

#[test]
fn new_lists_empty_the_templates_header_then_the_factory_templates_in_byte_order() {
    // One column: `D&B` < `Drift Phonk` < `Drill` by bytes (`&` is 0x26, `f` < `l`), whatever order the factory holds
    // them in.
    let rig = new_menu_rig(&THREE, 1.0);
    assert_eq!(menu_rows(&rig, &MENU), MENU);
    // One column: the labels share a left edge; the file extension is not shown.
    let lefts: Vec<f32> = MENU.iter().map(|t| rig.find(t).left()).collect();
    assert!(lefts.windows(2).all(|w| w[0] == w[1]), "{lefts:?}");
    assert!(!rig.texts().iter().any(|(t, _)| t.contains("songtmpl")));
}

#[test]
fn the_header_cannot_be_picked_and_empty_asks_for_the_song_options() {
    let mut rig = new_menu_rig(&THREE, 1.0);
    // The header does nothing, and the menu stays.
    assert_eq!(rig.click_text("Templates"), None);
    assert!(MENU.iter().all(|t| rig.has(t)));
    assert!(!rig.has(TITLE));
    // `Empty` is the new empty song, which the Song options dialog makes (`NewSong` on `OK`).
    assert_eq!(rig.click_text("Empty"), None);
    assert!(rig.has(TITLE));
    assert!(!rig.has("Drill"), "the menu closed");
    assert_eq!(
        rig.click_text("OK"),
        Some(MainMenuAction::NewSong(SongOptions::default()))
    );
}

#[test]
fn a_template_is_a_new_song_from_its_factory_file() {
    let mut rig = new_menu_rig(&THREE, 1.0);
    assert_eq!(
        rig.click_text("Drift Phonk"),
        Some(MainMenuAction::NewFromTemplate(template_at("Drift Phonk")))
    );
    assert!(!rig.has("Empty"), "the menu closed");
    assert!(!rig.has(TITLE), "a template skips the options dialog");
}

#[test]
fn escape_or_a_click_outside_closes_the_new_menu_and_does_nothing() {
    let mut rig = new_menu_rig(&THREE, 1.0);
    assert!(rig.has("Empty"));
    rig.key(Key::Escape);
    assert!(!rig.has("Empty"));
    assert_eq!(rig.action, None);
    // Outside the popup and outside the menu panel: neither the menu closes nor a button is pressed.
    let mut rig = new_menu_rig(&THREE, 1.0);
    assert!(rig.has("Empty"));
    assert_eq!(rig.click(pos2(100.0, 300.0)), None);
    assert!(!rig.has("Empty"));
    assert!(rig.has("Save"), "the SONGS tab is still there");
}

#[test]
fn the_new_menu_sits_on_a_whole_pixel_grid_at_ppp_1_and_2() {
    for ppp in [1.0_f32, 2.0] {
        let rig = new_menu_rig(&THREE, ppp);
        let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
        let content = shell_layout(Rect::from_min_size(Pos2::ZERO, SIZE.into()), &m).content;
        let boxes: Vec<Rect> = MENU.iter().map(|t| rig.find(t)).collect();
        // Five rows 1.25 U apart (40 points), a whole pixel each.
        for pair in boxes.windows(2) {
            let step = pair[1].center().y - pair[0].center().y;
            assert!((step - m.u(1.25)).abs() <= 1.0 / ppp, "ppp {ppp}: {step}");
        }
        // The popup is the `text`-coloured fill that holds the five labels, inside the tab's area, 0.5 U padding
        // either side of the widest label.
        let text = theme_by_id(0).colors.text.to_color32();
        let fill = rig
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == text => Some(r.rect),
                _ => None,
            })
            .filter(|r| boxes.iter().all(|b| r.contains_rect(*b)))
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .unwrap_or_else(|| panic!("ppp {ppp}: no popup fill"));
        assert!(
            content.contains_rect(fill),
            "ppp {ppp}: {fill:?} {content:?}"
        );
        assert_eq!(fill.height(), 5.0 * m.u(1.25), "ppp {ppp}");
        let widest = boxes.iter().map(|b| b.width()).fold(0.0, f32::max);
        assert!(
            (fill.width() - widest - m.u(1.0)).abs() <= 2.0 / ppp,
            "ppp {ppp}: {fill:?} {widest}"
        );
        for edge in [fill.left(), fill.right(), fill.top(), fill.bottom()] {
            let px = edge * ppp;
            assert!(
                (px - px.round()).abs() < 1e-3,
                "ppp {ppp}: {edge} is between pixels"
            );
        }
        // At the `New` button: the popup's top-right is the tab's, as the button is in the top-right corner.
        assert_eq!(fill.right(), content.right(), "ppp {ppp}");
        assert_eq!(fill.top(), content.top(), "ppp {ppp}");
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_original_content_s_templates_join_ours_without_repeating_a_name() {
    use crate::project_files::CONTENT_FOLDER;
    use crate::project_files::content::tests::{Temp, fake_tree};
    let tmp = Temp::new("songs-both-templates");
    fake_tree(&tmp.0.join(CONTENT_FOLDER));
    let mut rig = Rig::new(Tab::Songs);
    rig.library = Box::new(NativeLibrary::at(tmp.0.join(SONGS_FOLDER)));
    // The content folder has `Gen Ambient` and `Gen Beat`; ours has `Gen Beat` too, and `Zeta`.
    rig.menu.set_factory(factory_of(&["Zeta", "Gen Beat"]));
    rig.menu.refresh();
    rig.frame(vec![]);
    assert_eq!(rig.click_text("New"), None);
    let all = ["Empty", "Templates", "Gen Ambient", "Gen Beat", "Zeta"];
    assert_eq!(menu_rows(&rig, &all), all);
    // The name that both have is ours.
    assert_eq!(
        rig.click_text("Gen Beat"),
        Some(MainMenuAction::NewFromTemplate(template_at("Gen Beat")))
    );
}

#[test]
fn templates_sort_by_their_stems_not_their_file_names() {
    // `D&B 2.songtmpl` < `D&B-.songtmpl` < `D&B.songtmpl` as files (space 0x20, `-` 0x2d, `.` 0x2e), but the names are
    // `D&B` < `D&B 2` < `D&B-`.
    let names: Vec<String> = songs::all_templates(factory_of(&["D&B 2", "D&B-", "D&B"]), &Fake)
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["D&B", "D&B 2", "D&B-"]);
}

#[test]
fn only_song_templates_of_the_templates_folder_are_listed() {
    use crate::content::{Entry, Factory};
    let entry = |rel: &'static str| Entry::new(rel, Vec::new);
    let factory = Box::leak(Box::new(Factory::new(vec![
        entry("Templates/Trap.songtmpl"),
        entry("Templates/notes.txt"),
        entry("Templates/Sub/Deep.songtmpl"),
        // A folder that is named like a template is no template.
        entry("Templates/Odd.songtmpl/Inner.bin"),
        entry("Drumsets/Kit.songtmpl"),
        entry("Presets/Pad.flmpst"),
    ])));
    let names: Vec<String> = songs::all_templates(factory, &Fake)
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["Trap"]);
}

// SONGS lists project archives and offers `Unzip` for one.

/// SONGS on an in-memory library holding the song `A.flm`, the archives `Pack.zip` and `Other.zip`, the folder `F`
/// and a folder named like an archive, `Dir.zip`.
fn archives_rig() -> Rig {
    let mut library = MemoryLibrary::default();
    library.store("A.flm", b"a").unwrap();
    library.put(PathBuf::from("Pack.zip"), b"p").unwrap();
    library.put(PathBuf::from("Other.zip"), b"o").unwrap();
    library.make_folder(Path::new("F")).unwrap();
    library.make_folder(Path::new("Dir.zip")).unwrap();
    let mut rig = Rig::new(Tab::Songs);
    rig.library = Box::new(library);
    rig.menu.refresh();
    rig.frame(vec![]);
    rig
}

impl Rig {
    /// Whether the ≡ menu offers `Unzip` now; closes the menu again.
    fn offers_unzip(&mut self) -> bool {
        self.options();
        let offered = self.has("Unzip");
        self.key(Key::Escape);
        offered
    }
}

#[test]
fn an_archive_is_a_row_beside_the_songs_and_a_tap_opens_it() {
    let mut rig = archives_rig();
    // Folders first; then songs and archives together, by name.
    let top = |rig: &Rig, name: &str| rig.find(name).top();
    let order = ["Dir.zip", "F", "A.flm", "Other.zip", "Pack.zip"];
    for pair in order.windows(2) {
        assert!(top(&rig, pair[0]) < top(&rig, pair[1]), "{pair:?}");
    }
    // The same row as a song's: one row pitch (1.25 U) apart, a name at the row's left inset.
    let pitch = top(&rig, "Pack.zip") - top(&rig, "Other.zip");
    assert!((pitch - 1.25 * 32.0).abs() < 1.0, "{pitch}");
    assert_eq!(rig.find("Pack.zip").left(), rig.find("A.flm").left());
    // A tap opens it, as it opens a song: the app reads the file through the library.
    assert_eq!(
        rig.click_text("Pack.zip"),
        Some(MainMenuAction::Open(PathBuf::from("Pack.zip")))
    );
    // A folder named like an archive still goes in.
    assert_eq!(rig.click_text("Dir.zip"), None);
    assert!(rig.has("Back"));
    assert!(!rig.has("Pack.zip"));
}

#[test]
fn unzip_is_offered_for_exactly_one_selected_archive_between_delete_and_copy() {
    let mut rig = archives_rig();
    rig.click_text("Select");
    assert!(!rig.offers_unzip(), "nothing selected");
    // A song, a folder, and a folder named like an archive: none is an archive.
    for name in ["A.flm", "F", "Dir.zip"] {
        rig.click_text(name);
        assert!(!rig.offers_unzip(), "{name}");
        rig.click_text(name);
    }
    // One archive: after `Delete`, before `Copy`, enabled; it asks the app to unzip that file.
    rig.click_text("Pack.zip");
    rig.options();
    let (delete, unzip, copy) = (rig.find("Delete"), rig.find("Unzip"), rig.find("Copy"));
    assert!(delete.top() < unzip.top() && unzip.top() < copy.top());
    assert_eq!(
        rig.click_text("Unzip"),
        Some(MainMenuAction::Unzip(PathBuf::from("Pack.zip")))
    );
    // The menu closed; the selection stays.
    assert!(!rig.has("Unzip"));
    assert_eq!(
        rig.menu.songs.selection.clone().unwrap(),
        [PathBuf::from("Pack.zip")]
    );
    // Two archives, an archive with a song, or with a folder: not exactly one archive.
    for other in ["Other.zip", "A.flm", "F"] {
        rig.click_text(other);
        assert!(!rig.offers_unzip(), "Pack.zip and {other}");
        rig.click_text(other);
    }
    assert!(rig.offers_unzip());
    // Out of select mode the menu has no selection, so no `Unzip`.
    rig.click_text("Select");
    assert!(!rig.offers_unzip());
}

/// `Unzipped songs` is a place beside `My Songs`, as `Recently Deleted` is: a tap goes in even in select mode, and
/// `Select all` leaves it out. What is in it is an ordinary folder.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn unzipped_songs_is_a_place_a_tap_goes_into_in_select_mode_and_select_all_skips() {
    let tmp = crate::project_files::content::tests::Temp::new("su-place");
    let user = tmp.0.clone();
    let root = user.join("My Songs");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("A.flm"), b"a").unwrap();
    let pack = user.join("Unzipped songs/Pack");
    std::fs::create_dir_all(pack.join("My Samples")).unwrap();
    std::fs::write(pack.join("Beat.flm"), b"b").unwrap();
    let mut rig = Rig::new(Tab::Songs);
    rig.library = Box::new(NativeLibrary::at(root));
    rig.menu.refresh();
    rig.frame(vec![]);
    let selection = |rig: &Rig| rig.menu.songs.selection.clone().unwrap();
    assert!(rig.has("Unzipped songs") && rig.has("A.flm"));
    // Select mode: the song toggles, the place goes in and is not selected.
    rig.click_text("Select");
    rig.click_text("A.flm");
    assert_eq!(selection(&rig).len(), 1);
    assert_eq!(rig.click_text("Unzipped songs"), None);
    assert!(rig.has("Pack") && rig.has("Back"));
    assert!(
        selection(&rig).is_empty(),
        "a folder change drops the selection"
    );
    // Out of select mode, `Pack` goes in: an ordinary folder, below `My Songs / Unzipped songs`.
    rig.click_text("Select");
    rig.click_text("Pack");
    assert!(rig.has("Beat.flm") && rig.has("My Samples"));
    assert!(rig.has("My Songs / Unzipped songs"));
    assert_eq!(
        rig.click_text("Beat.flm"),
        Some(MainMenuAction::Open(pack.join("Beat.flm")))
    );
    rig.click_text("Back");
    rig.click_text("Back");
    assert!(rig.has("Unzipped songs") && rig.has("A.flm"));
    // `Select all` selects the song, not the place.
    rig.click_text("Select");
    rig.pick("Select all");
    assert_eq!(selection(&rig), [user.join("My Songs/A.flm")]);
}

// ---- the SONGS search bar and the browser actions ----

impl Rig {
    fn tap_key(&mut self, key: Key, modifiers: Modifiers) {
        self.frame(key_tap(key, modifiers));
    }

    /// `startSearch`: Ctrl+F.
    fn ctrl_f(&mut self) {
        self.tap_key(Key::F, Modifiers::COMMAND);
    }

    fn escape(&mut self) {
        self.tap_key(Key::Escape, Modifiers::NONE);
    }

    /// The search field has the keyboard.
    fn searching(&self) -> bool {
        let field = Id::new("main_menu_songs").with("search");
        self.ctx.memory(|mem| mem.has_focus(field))
    }

    fn rect_of(&self, id: Id) -> Rect {
        self.ctx.read_response(id).expect("drawn").rect
    }
}

#[test]
fn ctrl_f_shows_the_songs_bar_over_the_breadcrumb_and_list_which_move_down_1_75_u() {
    // The page moved the focus into its list as it showed, so Ctrl+F is the browser's.
    let mut rig = Rig::new(Tab::Songs);
    let (title, row) = (
        rig.find(SONGS_FOLDER).center(),
        rig.find("Alpha.flm").center(),
    );
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(rig.searching());
    // The browser: the content left of the big circles' 2.5 U column (546..1074 at U = 32), from the content's top.
    let id = Id::new("main_menu_songs");
    let bar = Rect::from_min_size(pos2(546.0 + 56.0, 104.0 + 8.0), vec2(528.0 - 64.0, 48.0));
    assert_eq!(
        rig.rect_of(id.with("search").with("clear")),
        bar.with_min_x(bar.right() - 64.0)
    );
    assert_eq!(
        rig.rect_of(id.with("search")),
        bar.with_max_x(bar.right() - 64.0)
    );
    assert_eq!(rig.find(SONGS_FOLDER).center(), title + vec2(0.0, 56.0));
    assert_eq!(rig.find("Alpha.flm").center(), row + vec2(0.0, 56.0));
    // Typing filters on the next frame, by the trimmed text.
    rig.frame(vec![Event::Text(" alp ".into())]);
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["Alpha.flm"]);
    // Enter does nothing in the field; Clear empties the query and the bar stays open and focused.
    rig.tap_key(Key::Enter, Modifiers::NONE);
    assert_eq!(rig.action, None);
    rig.frame(vec![]);
    assert!(rig.searching() && rig.has(" alp "));
    let clear = rig.rect_of(id.with("search").with("clear")).center();
    assert_eq!(rig.click(clear), None);
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["Demo songs", "Alpha.flm", "beta.flm"]);
    assert!(rig.searching() && rig.has("Clear"));
    // A second Ctrl+F keeps the bar.
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(rig.searching() && rig.has("Clear"));
}

#[test]
fn escape_in_songs_hides_the_bar_then_stops_selecting_then_closes_the_menu() {
    let mut rig = Rig::new(Tab::Songs);
    rig.ctrl_f();
    rig.frame(vec![Event::Text("alp".into())]);
    rig.frame(vec![]);
    rig.click_text("Select");
    assert!(rig.menu.songs.selection.is_some());
    // Bar shown: hidden, its query cleared, select mode left; the menu stays.
    rig.escape();
    assert_eq!(rig.action, None);
    rig.frame(vec![]);
    assert!(!rig.has("Clear") && rig.menu.songs.selection.is_none());
    assert_eq!(rig.rows(), ["Demo songs", "Alpha.flm", "beta.flm"]);
    // Select mode: stopped.
    rig.click_text("Select");
    rig.escape();
    assert_eq!(rig.action, None);
    assert!(rig.menu.songs.selection.is_none());
    // Else the menu closes.
    rig.escape();
    assert_eq!(rig.action, Some(MainMenuAction::Close));
    // The hidden field kept its text: Ctrl+F brings the filter back. (The focus went to `Search`, in the browser.)
    let mut rig = Rig::new(Tab::Songs);
    rig.ctrl_f();
    rig.frame(vec![Event::Text("alp".into())]);
    rig.escape();
    rig.frame(vec![]);
    rig.ctrl_f();
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["Alpha.flm"]);
}

#[test]
fn showing_the_songs_page_again_resets_the_search() {
    let mut rig = Rig::new(Tab::Songs);
    rig.ctrl_f();
    rig.frame(vec![Event::Text("alp".into())]);
    rig.frame(vec![]);
    rig.click_text("PROJECT");
    rig.click_text("SONGS");
    assert!(!rig.has("Clear"));
    assert_eq!(rig.rows(), ["Demo songs", "Alpha.flm", "beta.flm"]);
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(rig.has("Clear") && !rig.has("alp"), "the text cleared too");
    assert_eq!(rig.rows(), ["Demo songs", "Alpha.flm", "beta.flm"]);
}

#[test]
fn ctrl_f_does_nothing_off_the_songs_page() {
    let mut rig = Rig::new(Tab::Project);
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(!rig.has("Clear") && rig.action.is_none());
}

#[test]
fn a_songs_search_lists_the_songs_below_the_open_folder_with_their_folders_path() {
    // A non-empty query lists everything below the open folder, recursively, in 2.2 U rows with the folder's
    // path under the name in FontSize 1 at α 0.6.
    let mut rig = Rig::new(Tab::Songs);
    rig.ctrl_f();
    rig.frame(vec![Event::Text("thr".into())]);
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["Three.flm"]);
    let place = "My Songs/Demo songs";
    let text = theme_by_id(0).colors.text.to_color32();
    let (name, at) = (rig.find("Three.flm"), rig.find(place));
    assert!(at.top() > name.top(), "the path is under the name");
    let painted = |t: &str| {
        rig.shapes.iter().find_map(|c| match &c.shape {
            Shape::Text(s) if s.galley.text() == t => Some(s.galley.job.sections[0].format.clone()),
            _ => None,
        })
    };
    let format = painted(place).unwrap();
    assert_eq!(format.font_id.size, m().font_id(FontSize::Pt8).size);
    assert_eq!(format.color, text.gamma_multiply(0.6));
    // The row is 2.2 U: the file below it lands one such row lower.
    // Depth first in the page's order: the folder, its song, then the root's.
    rig.clear_and_type("e");
    assert_eq!(rig.rows(), ["Demo songs", "Three.flm", "beta.flm"]);
    let (first, second) = (rig.find("Three.flm").top(), rig.find("beta.flm").top());
    assert!(
        (second - first - 2.2 * 32.0).abs() < 0.01,
        "{first} {second}"
    );
    // A pick opens the song where it is.
    let at = rig.find("Three.flm").center();
    let three = PathBuf::from("/My Songs/Demo songs/Three.flm");
    assert_eq!(rig.click(at), Some(MainMenuAction::Open(three)));
}

impl Rig {
    /// Empties the search field with `Clear` and types `query`, then waits for the next frame's filter.
    fn clear_and_type(&mut self, query: &str) {
        let clear = self.rect_of(Id::new("main_menu_songs").with("search").with("clear"));
        self.click(clear.center());
        self.frame(vec![Event::Text(query.into())]);
        self.frame(vec![]);
    }
}

#[test]
fn a_found_songs_folder_opens_with_its_trail_and_ends_the_search() {
    let mut rig = Rig::new(Tab::Songs);
    rig.ctrl_f();
    rig.frame(vec![Event::Text("demo".into())]);
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["Demo songs"]);
    assert!(
        rig.find(SONGS_FOLDER).top() > rig.find("Demo songs").top(),
        "its path under it"
    );
    rig.click_text("Demo songs");
    assert!(!rig.has("Clear"));
    assert_eq!(rig.rows(), ["Three.flm"]);
    rig.click_text("Back");
    assert_eq!(rig.rows(), ["Demo songs", "Alpha.flm", "beta.flm"]);
}

#[test]
fn a_sort_change_reorders_the_search_results() {
    let mut rig = Rig::new(Tab::Songs);
    rig.ctrl_f();
    rig.frame(vec![Event::Text("a".into())]);
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["Alpha.flm", "beta.flm"]);
    rig.click_text("Sort");
    rig.click_text("Z-A");
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["beta.flm", "Alpha.flm"]);
}

#[test]
fn the_search_skips_the_bin_but_not_a_folder_of_its_name() {
    let mut library = MemoryLibrary::default();
    let trash = library.trash().unwrap();
    library.make_folder(&trash).unwrap();
    library.store_in(&trash, "Lost Old.flm", b"x").unwrap();
    let mine = Path::new("F").join(library::RECENTLY_DELETED);
    library.make_folder(Path::new("F")).unwrap();
    library.make_folder(&mine).unwrap();
    library.store_in(&mine, "Old Kept.flm", b"x").unwrap();
    let mut rig = Rig::new(Tab::Songs);
    rig.library = Box::new(library);
    rig.menu.refresh();
    rig.frame(vec![]);
    assert!(
        rig.has(library::RECENTLY_DELETED),
        "the bin lists at the root"
    );
    rig.ctrl_f();
    rig.frame(vec![Event::Text("old".into())]);
    rig.frame(vec![]);
    assert!(rig.has("Old Kept.flm") && rig.has("My Songs/F/Recently Deleted"));
    assert!(!rig.has("Lost Old.flm"));
}

#[test]
fn a_deleted_song_leaves_the_search_results() {
    let mut rig = songs_rig();
    rig.ctrl_f();
    rig.frame(vec![Event::Text("a.".into())]);
    rig.frame(vec![]);
    assert!(rig.has("A.flm") && !rig.has("B.flm"));
    rig.click_text("Select");
    rig.click_text("A.flm");
    assert_eq!(rig.pick("Delete"), None);
    rig.frame(vec![]);
    assert!(rig.has("Clear") && !rig.has("A.flm"));
}

#[test]
fn ctrl_f_does_nothing_while_a_popup_a_name_field_or_the_song_options_show() {
    let mut rig = Rig::new(Tab::Songs);
    rig.click_text("Sort");
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(!rig.has("Clear") && rig.has("Recent"));

    let mut rig = songs_rig();
    rig.pick("Create folder");
    assert!(rig.has("Enter folder name"));
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(!rig.has("Clear") && rig.has("Enter folder name"));

    // No templates: `New` asks for the empty song at once.
    let mut rig = Rig::new(Tab::Songs);
    rig.click_text("New");
    assert!(rig.has(TITLE));
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(!rig.has("Clear") && rig.has(TITLE));
}

#[test]
fn a_found_folder_two_levels_down_opens_with_the_folders_above_it() {
    let mut library = MemoryLibrary::default();
    for dir in ["F", "F/G", "F/G/Hidden"] {
        library.make_folder(Path::new(dir)).unwrap();
    }
    library
        .store_in(Path::new("F/G/Hidden"), "Deep.flm", b"x")
        .unwrap();
    let mut rig = Rig::new(Tab::Songs);
    rig.library = Box::new(library);
    rig.menu.refresh();
    rig.frame(vec![]);
    rig.ctrl_f();
    rig.frame(vec![Event::Text("hidden".into())]);
    rig.frame(vec![]);
    assert!(rig.has("Hidden") && rig.has("My Songs/F/G"));
    rig.click_text("Hidden");
    assert!(
        rig.has("Deep.flm") && rig.has("My Songs / F / G"),
        "the breadcrumb through F and G"
    );
    rig.click_text("Back");
    assert!(rig.has("My Songs / F") && rig.has("Hidden") && !rig.has("Deep.flm"));
}

#[test]
fn the_songs_sort_is_kept_as_song_browser_ordering_and_restored() {
    // The sort orders are persisted (hidden), SONGS' as `SongBrowserOrdering`.
    let mut rig = Rig::new(Tab::Songs);
    assert_eq!(rig.settings.1.song_ordering(), Sort::AZ);
    rig.click_text("Sort");
    rig.click_text("Z-A");
    assert_eq!(rig.settings.1.song_browser_ordering, Sort::ZA.id());
    // A menu opened anew lists in the stored order.
    let mut rig = Rig::new(Tab::Songs);
    rig.settings.1.song_browser_ordering = Sort::ZA.id();
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["Demo songs", "beta.flm", "Alpha.flm"]);
}

#[test]
fn a_sort_changed_outside_the_page_reorders_the_search_results() {
    // `Restore default settings` (or any other writer of `SongBrowserOrdering`) changes the order under an open
    // search: its results are walked again in the new order.
    let mut rig = Rig::new(Tab::Songs);
    rig.ctrl_f();
    rig.frame(vec![Event::Text("a".into())]);
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["Alpha.flm", "beta.flm"]);
    rig.settings.1.song_browser_ordering = Sort::ZA.id();
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(rig.rows(), ["beta.flm", "Alpha.flm"]);
}

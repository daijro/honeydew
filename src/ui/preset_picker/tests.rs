use std::cell::Cell;
use std::io;

use eframe::egui::epaint::ClippedShape;
use eframe::egui::{
    self, Color32, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2,
};

use super::*;
use crate::metrics::{DEFAULT_UI_SCALE_INDEX, FontSize, Metrics};
use crate::project_files::{PRESET_EXT, RACK_EXT};
use crate::theme::{DISABLED_ALPHA, theme_by_id};
use crate::ui::main_menu::MemoryLibrary;
use crate::ui::modules_browser::layout;
use crate::ui::offscreen::key_tap;

/// `My Presets/Reverb`, the user's folder for Reverb in these tests, as a [`MemoryLibrary`] holds it.
const REVERB: &str = "My Presets/Reverb";

/// A library holding `paths`: a path ending in `/` is a folder, any other a file.
fn library(paths: &[&str]) -> MemoryLibrary {
    let mut lib = MemoryLibrary::default();
    for path in paths {
        match path.strip_suffix('/') {
            Some(folder) => lib.make_folder(Path::new(folder)).unwrap(),
            None => {
                // `store` takes song names only: store one, then move it into place.
                lib.store("x.flm", path.as_bytes()).unwrap();
                lib.move_to(Path::new("x.flm"), Path::new(path)).unwrap();
            }
        }
    }
    lib
}

/// Reverb's folder: two folders, presets in any case, and files of other scopes.
fn reverb_lib() -> MemoryLibrary {
    library(&[
        "My Presets/",
        "My Presets/Reverb/",
        "My Presets/Reverb/b Hall.flmpst",
        "My Presets/Reverb/Vocal/",
        "My Presets/Reverb/Vocal/Plate.flmpst",
        "My Presets/Reverb/Archive/",
        "My Presets/Reverb/A Room.flmpst",
        "My Presets/Reverb/Long.FLMPST",
        "My Presets/Reverb/Song.flm",
        "My Presets/Reverb/Chain.flmrack",
        "My Presets/Reverb/notes.txt",
    ])
}

fn reverb() -> PresetPicker {
    PresetPicker::new("Reverb", Some(REVERB.into()), PRESET_EXT, true)
}

fn names(picker: &PresetPicker, query: &str) -> Vec<String> {
    let rows = picker.rows(query);
    rows.into_iter().map(|i| i.name().to_owned()).collect()
}

/// Opens the folder `name` of the rows, as a click on it does.
fn open(picker: &mut PresetPicker, lib: &dyn SongLibrary, name: &str) {
    let folder = match picker.rows("").into_iter().find(|i| i.name() == name) {
        Some(Item::MyFiles(Some(path))) => SongEntry {
            name: MY_FILES.into(),
            path: path.to_owned(),
            is_folder: true,
            recency: 0,
        },
        Some(Item::Extra(Extra {
            path: Some(path), ..
        })) => SongEntry {
            name: name.into(),
            path: path.to_owned(),
            is_folder: true,
            recency: 0,
        },
        Some(Item::Entry(e)) if e.is_folder => e.clone(),
        other => panic!("{name}: {other:?}"),
    };
    picker.folders.push(folder);
    let folder = picker.folders.last().unwrap().path.clone();
    picker.listing = Some(picker.read_open(lib, &folder));
}

#[test]
fn the_root_lists_my_files_then_default_and_a_rack_only_my_files() {
    assert_eq!(names(&reverb(), ""), [MY_FILES, DEFAULT]);
    let rack = PresetPicker::new(RACK_TITLE, Some("My Racks".into()), RACK_EXT, false);
    assert_eq!(names(&rack, ""), [MY_FILES]);
    // Without a user folder `My Files` is still listed (and dimmed: see the offscreen tests).
    let none = PresetPicker::new("Reverb", None, PRESET_EXT, true);
    assert_eq!(names(&none, ""), [MY_FILES, DEFAULT]);
}

#[test]
fn a_folder_lists_its_folders_then_its_presets_a_to_z_by_name() {
    let lib = reverb_lib();
    let mut picker = reverb();
    open(&mut picker, &lib, MY_FILES);
    assert_eq!(
        names(&picker, ""),
        ["Archive", "Vocal", "A Room", "b Hall", "Long"]
    );
    open(&mut picker, &lib, "Vocal");
    assert_eq!(names(&picker, ""), ["Plate"]);
}

#[test]
fn only_the_scopes_files_are_listed() {
    let lib = reverb_lib();
    let mut rack = PresetPicker::new(RACK_TITLE, Some(REVERB.into()), RACK_EXT, false);
    open(&mut rack, &lib, MY_FILES);
    assert_eq!(names(&rack, ""), ["Archive", "Vocal", "Chain"]);
    // A preset picks its path as listed, extension and all.
    let chain = rack.rows("").into_iter().find(|i| i.name() == "Chain");
    let Some(Item::Entry(chain)) = chain else {
        panic!("{chain:?}")
    };
    assert_eq!(chain.path, Path::new("My Presets/Reverb/Chain.flmrack"));
}

#[test]
fn a_picker_of_several_types_lists_the_files_of_each() {
    // The sample picker's audio files, in any case; other types stay hidden.
    let lib = library(&[
        "My Samples/",
        "My Samples/Kicks/",
        "My Samples/Kick.WAV",
        "My Samples/Loop.flac",
        "My Samples/Vox.mp3",
        "My Samples/Pad.ogg",
        "My Samples/Voice.m4a",
        "My Samples/Song.flm",
    ]);
    let exts = ["wav", "flac", "mp3", "ogg"];
    let mut picker =
        PresetPicker::new("Samples", Some("My Samples".into()), exts[0], false).of_exts(&exts);
    open(&mut picker, &lib, MY_FILES);
    assert_eq!(names(&picker, ""), ["Kicks", "Kick", "Loop", "Pad", "Vox"]);
}

#[test]
fn search_keeps_the_rows_whose_name_holds_the_query() {
    let lib = reverb_lib();
    let mut picker = reverb();
    assert_eq!(names(&picker, "DEF"), [DEFAULT]);
    assert_eq!(names(&picker, "files"), [MY_FILES]);
    assert!(names(&picker, "hall").is_empty());
    open(&mut picker, &lib, MY_FILES);
    assert_eq!(names(&picker, "o"), ["Vocal", "A Room", "Long"]);
    // The stem is searched, not the extension.
    assert!(names(&picker, "flmpst").is_empty());
}

#[test]
fn the_header_is_the_title_at_the_root_and_the_breadcrumb_in_a_folder() {
    // The breadcrumb path joined with " | ", after 5 leading spaces, down to the open folder.
    let lib = reverb_lib();
    let mut picker = reverb();
    assert_eq!(picker.header(), "Reverb");
    open(&mut picker, &lib, MY_FILES);
    assert_eq!(picker.header(), "     Reverb | My Files");
    open(&mut picker, &lib, "Vocal");
    assert_eq!(picker.header(), "     Reverb | My Files | Vocal");
}

#[test]
fn a_my_files_folder_that_does_not_exist_yet_lists_as_empty() {
    let lib = library(&[]);
    let mut picker = reverb();
    open(&mut picker, &lib, MY_FILES);
    assert!(names(&picker, "").is_empty());
}

// ---- the panel, offscreen ----

const SCREEN: Rect = Rect {
    min: pos2(0.0, 0.0),
    max: pos2(1000.0, 800.0),
};

/// `U` = 32 points at ppp 1.
fn metrics() -> Metrics {
    Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX)
}

/// A library that counts its listings.
struct Counting {
    lib: MemoryLibrary,
    lists: Cell<usize>,
}

impl SongLibrary for Counting {
    fn list(&self, folder: Option<&Path>) -> io::Result<Vec<SongEntry>> {
        self.lists.set(self.lists.get() + 1);
        self.lib.list(folder)
    }
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.lib.read(path)
    }
    fn store(&mut self, _: &str, _: &[u8]) -> io::Result<()> {
        unreachable!("the picker stores nothing")
    }
    // The file actions' building blocks, as the library has them.
    fn root(&self) -> Option<&Path> {
        self.lib.root()
    }
    fn trash(&self) -> Option<PathBuf> {
        self.lib.trash()
    }
    fn bin_path(&self, path: &Path) -> Option<PathBuf> {
        self.lib.bin_path(path)
    }
    fn restore_path(&self, path: &Path) -> Option<PathBuf> {
        self.lib.restore_path(path)
    }
    fn exists(&self, path: &Path) -> bool {
        self.lib.exists(path)
    }
    fn make_folder(&mut self, path: &Path) -> io::Result<()> {
        self.lib.make_folder(path)
    }
    fn move_to(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        self.lib.move_to(from, to)
    }
    fn copy_to(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        self.lib.copy_to(from, to)
    }
    fn remove(&mut self, path: &Path) -> io::Result<()> {
        self.lib.remove(path)
    }
}

struct Rig {
    ctx: egui::Context,
    picker: PresetPicker,
    lib: Counting,
    time: f64,
    /// What the last frame returned and painted.
    action: Option<PickerAction>,
    shapes: Vec<ClippedShape>,
    /// Shown with [`PresetPicker::show_mut`]: the file actions.
    files: bool,
    /// The keyboard focus runs as in the app: its frame start and end around the picker.
    focus: bool,
}

impl Rig {
    /// `picker` just opened over [`reverb_lib`], run until the slide-in is over.
    fn new(picker: PresetPicker) -> Self {
        Self::with(picker, reverb_lib())
    }

    /// `picker` just opened over `lib`, run until the slide-in is over.
    fn with(picker: PresetPicker, lib: MemoryLibrary) -> Self {
        let lib = Counting {
            lib,
            lists: Cell::new(0),
        };
        let mut rig = Self {
            ctx: egui::Context::default(),
            picker,
            lib,
            time: 0.0,
            action: None,
            shapes: Vec::new(),
            files: false,
            focus: false,
        };
        rig.settle();
        rig
    }

    /// [`Self::with`], shown with the file actions.
    fn files(picker: PresetPicker, lib: MemoryLibrary) -> Self {
        let mut rig = Self::with(picker, lib);
        rig.files = true;
        rig.settle();
        rig
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (theme, m) = (theme_by_id(0), metrics());
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let (picker, lib, files, focus) = (&mut self.picker, &mut self.lib, self.files, self.focus);
        let mut action = None;
        let output = self.ctx.run(raw, |ctx| {
            if focus {
                crate::ui::focus::begin_frame(ctx);
            }
            action = match files {
                true => picker.show_mut(ctx, &cx, lib),
                false => picker.show(ctx, &cx, lib),
            };
            if focus {
                crate::ui::focus::end_frame(ctx, &theme);
            }
        });
        self.action = action;
        self.shapes = output.shapes;
    }

    fn settle(&mut self) {
        for _ in 0..30 {
            self.frame(vec![]);
        }
    }

    /// Returns what the release frame returned, and lets the next frame draw what the click changed.
    fn click(&mut self, pos: Pos2) -> Option<PickerAction> {
        self.frame(vec![Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }]);
        }
        let action = self.action.clone();
        self.frame(vec![]);
        action
    }

    fn click_text(&mut self, text: &str) -> Option<PickerAction> {
        let at = self.text_at(text);
        self.click(at)
    }

    /// A click on the header's Back arrow (where it shows inside a folder).
    fn back(&mut self) -> Option<PickerAction> {
        self.click(layout(SCREEN, 1.0, &metrics()).back.center())
    }

    /// Whether something other than text is drawn inside `rect` alone (an icon there).
    fn draws_in(&self, rect: Rect) -> bool {
        (self.shapes.iter())
            .filter(|c| !matches!(c.shape, egui::Shape::Text(_)))
            .any(|c| rect.contains_rect(c.shape.visual_bounding_rect()))
    }

    /// Texts painted in the last frame, with where they were drawn and their colour.
    fn texts(&self) -> Vec<(String, Rect, Color32)> {
        self.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.galley.rect.translate(t.pos.to_vec2()),
                    t.galley.job.sections[0].format.color,
                )),
                _ => None,
            })
            .collect()
    }

    fn find(&self, text: &str) -> (Rect, Color32) {
        let texts = self.texts();
        let found = texts.iter().find(|(t, ..)| t == text);
        let (_, rect, color) = found.unwrap_or_else(|| panic!("{text} not drawn: {texts:?}"));
        (*rect, *color)
    }

    fn shows(&self, text: &str) -> bool {
        self.texts().iter().any(|(t, ..)| t == text)
    }

    fn text_at(&self, text: &str) -> Pos2 {
        self.find(text).0.center()
    }

    fn color(&self, text: &str) -> Color32 {
        self.find(text).1
    }
}

fn text() -> Color32 {
    theme_by_id(0).colors.text.to_color32()
}

#[test]
fn my_files_then_a_folder_then_a_preset_picks_its_path() {
    let mut rig = Rig::new(reverb());
    assert!(rig.shows("Reverb") && rig.shows(MY_FILES) && rig.shows(DEFAULT));
    assert_eq!(rig.click_text(MY_FILES), None);
    assert!(rig.shows("Archive") && rig.shows("A Room") && !rig.shows(DEFAULT));
    assert_eq!(rig.click_text("Vocal"), None);
    assert!(rig.shows("Plate") && !rig.shows("A Room"));
    let plate = PathBuf::from("My Presets/Reverb/Vocal/Plate.flmpst");
    assert_eq!(rig.click_text("Plate"), Some(PickerAction::Pick(plate)));
}

#[test]
fn default_picks_the_init_preset() {
    let mut rig = Rig::new(reverb());
    assert_eq!(rig.click_text(DEFAULT), Some(PickerAction::Default));
}

#[test]
fn every_row_folder_or_item_is_in_fontsize_4_and_a_long_name_is_cut_to_the_row() {
    // One size for every row of every side browser, the folder rows' FontSize 4.
    let m = metrics();
    let pt18 = m.font_id(FontSize::Pt18).size;
    let mut lib = reverb_lib();
    let long = "My Presets/Reverb/Cathedral Hall With A Very Long Shimmering Tail.flmpst";
    lib.store("x.flm", b"x").unwrap();
    lib.move_to(Path::new("x.flm"), Path::new(long)).unwrap();
    let mut rig = Rig::with(reverb(), lib);
    assert_eq!(
        (font_size(&rig, MY_FILES), font_size(&rig, DEFAULT)),
        (pt18, pt18)
    );
    rig.click_text(MY_FILES);
    assert_eq!(
        (font_size(&rig, "Vocal"), font_size(&rig, "A Room")),
        (pt18, pt18)
    );
    // The row height stays the picker's own: 1.25 U here.
    let id = Id::new("preset_picker").with(("row", 0_usize));
    let row = rig.ctx.read_response(id).unwrap().rect;
    assert_eq!(row.height(), m.round_px(m.u(1.25)));
    // The long name keeps its middle ellipsis inside the row.
    let texts = rig.texts();
    let (cut, at, _) = (texts.iter())
        .find(|(t, ..)| t.starts_with("Cathedral"))
        .expect("the long name is drawn");
    assert!(cut.contains("...") && cut.ends_with("Tail"), "{cut}");
    assert!(
        at.right() <= row.right() - m.u(0.5) + 0.01,
        "{at:?} in {row:?}"
    );
}

#[test]
fn the_close_x_escape_and_a_click_outside_close() {
    let close = layout(SCREEN, 1.0, &metrics()).close;
    let mut rig = Rig::new(reverb());
    assert_eq!(rig.click(close), Some(PickerAction::Close));

    let mut rig = Rig::new(reverb());
    assert_eq!(rig.click(pos2(800.0, 250.0)), Some(PickerAction::Close));

    let mut rig = Rig::new(reverb());
    rig.frame(vec![Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert_eq!(rig.action, Some(PickerAction::Close));

    // In a folder too; a click on the panel outside any control closes nothing.
    let mut rig = Rig::new(reverb());
    rig.click_text(MY_FILES);
    assert_eq!(rig.click(pos2(200.0, 780.0)), None);
    assert_eq!(rig.click(close), Some(PickerAction::Close));
}

#[test]
fn the_back_arrow_is_hidden_at_the_root_and_goes_up_below_it() {
    // Inside a folder the Back arrow (icon 43, 1.5 U wide) shows at the header's left; at the root it is
    // hidden and Back does nothing. No `Back` button is left down the edge.
    let m = metrics();
    let at = layout(SCREEN, 1.0, &m);
    assert_eq!(at.back.width(), m.u(1.5));
    assert_eq!(at.back.y_range(), at.header.y_range());
    let mut rig = Rig::new(reverb());
    assert!(!rig.draws_in(at.back) && !rig.shows("Back"));
    assert_eq!(rig.back(), None);
    assert!(rig.shows(DEFAULT));

    rig.click_text(MY_FILES);
    rig.click_text("Vocal");
    assert!(rig.draws_in(at.back) && !rig.shows("Back"));
    assert_eq!(rig.back(), None);
    assert!(rig.shows("Archive") && rig.shows("     Reverb | My Files") && !rig.shows("Plate"));
    rig.back();
    assert!(rig.shows(DEFAULT) && rig.shows("Reverb"));
    assert!(!rig.draws_in(at.back));
}

#[test]
fn ctrl_up_leaves_the_folder_and_does_nothing_at_the_root() {
    // `leaveFolder` (Ctrl+Up; Cmd+Up on a Mac).
    let mut rig = Rig::new(reverb());
    let up = || key_tap(Key::ArrowUp, Modifiers::CTRL | Modifiers::COMMAND);
    rig.click_text(MY_FILES);
    rig.click_text("Vocal");
    rig.frame(up());
    assert_eq!(rig.action, None);
    rig.frame(vec![]);
    assert!(rig.shows("Archive") && !rig.shows("Plate"));
    rig.frame(up());
    rig.frame(vec![]);
    assert!(rig.shows(DEFAULT));
    rig.frame(up());
    assert_eq!(rig.action, None);
    rig.frame(vec![]);
    assert!(rig.shows(DEFAULT));
}

#[test]
fn the_header_is_2_u_with_a_1_px_text_line_and_the_x_1_5_u_wide_0_25_u_from_the_right() {
    let m = metrics();
    let at = layout(SCREEN, 1.0, &m);
    assert_eq!(at.header.height(), m.u(2.0));
    assert_eq!(at.close.x, at.panel.right() - m.u(0.25) - m.u(1.5) / 2.0);
    assert_eq!(at.close.y, at.header.center().y);
    let rig = Rig::new(reverb());
    let line = Rect::from_x_y_ranges(
        at.header.x_range(),
        at.header.bottom() - 1.0 / m.ppp()..=at.header.bottom(),
    );
    let drawn = rig.shapes.iter().any(|c| match &c.shape {
        egui::Shape::Rect(r) => r.rect == line && r.fill == text(),
        _ => false,
    });
    assert!(drawn, "no 1 px text line under the header");
}

#[test]
fn the_header_shows_the_title_at_the_root_then_the_breadcrumb_in_one_line() {
    let mut rig = Rig::new(reverb());
    let header = layout(SCREEN, 1.0, &metrics()).header;
    assert!(header.contains(rig.text_at("Reverb")));
    rig.click_text(MY_FILES);
    rig.click_text("Vocal");
    let crumb = rig.find("     Reverb | My Files | Vocal").0;
    assert!(header.contains(crumb.center()));
    assert!(!rig.shows("Vocal") && !rig.shows("Reverb"));
}

#[test]
fn a_missing_user_folder_dims_my_files() {
    let mut rig = Rig::new(PresetPicker::new("Reverb", None, PRESET_EXT, true));
    assert_eq!(rig.color(MY_FILES), text().gamma_multiply(DISABLED_ALPHA));
    assert_eq!(rig.click_text(MY_FILES), None);
    assert!(rig.shows(DEFAULT) && !rig.shows("Archive"));
}

#[test]
fn search_filters_the_open_folder() {
    let mut rig = Rig::new(reverb());
    rig.click_text(MY_FILES);
    rig.click_text("Search");
    rig.frame(vec![Event::Text("ro".into())]);
    rig.frame(vec![]);
    assert!(rig.shows("A Room"));
    for gone in ["Archive", "Vocal", "b Hall", "Long"] {
        assert!(!rig.shows(gone), "{gone}");
    }
    let room = PathBuf::from("My Presets/Reverb/A Room.flmpst");
    assert_eq!(rig.click_text("A Room"), Some(PickerAction::Pick(room)));
}

#[test]
fn a_folder_is_read_when_it_opens_not_every_frame() {
    let mut rig = Rig::new(reverb());
    // The root lists nothing from the library.
    assert_eq!(rig.lib.lists.get(), 0);
    rig.click_text(MY_FILES);
    rig.settle();
    assert_eq!(rig.lib.lists.get(), 1);
    rig.click_text("Vocal");
    rig.back();
    rig.settle();
    assert_eq!(rig.lib.lists.get(), 3);
}

/// `lib` with `bytes` at `path`.
fn put(lib: &mut MemoryLibrary, path: &str, bytes: &[u8]) {
    lib.store("x.flm", bytes).unwrap();
    lib.move_to(Path::new("x.flm"), Path::new(path)).unwrap();
}

#[test]
fn a_module_scope_lists_only_the_presets_its_module_loads() {
    let mut lib = library(&[
        "My Presets/",
        "My Presets/Reverb/",
        "My Presets/Reverb/Sub/",
    ]);
    for (name, type_id) in [("Hall", 32), ("Old Hall", 12), ("Crunch", 6)] {
        let bytes = crate::flm::write_preset(&crate::song::ModuleInstance::new(type_id, 0));
        put(
            &mut lib,
            &format!("{REVERB}/{name}.flmpst"),
            &bytes.unwrap(),
        );
    }
    put(
        &mut lib,
        &format!("{REVERB}/Broken.flmpst"),
        b"not a preset",
    );
    let mut picker = reverb().of_type(32);
    open(&mut picker, &lib, MY_FILES);
    // A legacy Reverb (12) loads into a Reverb v2 (32); a Distortion's preset and an unreadable file do not.
    assert_eq!(names(&picker, ""), ["Sub", "Hall", "Old Hall"]);
}

#[test]
fn a_long_preset_name_is_shortened_with_its_tail_kept() {
    let long = "A very long preset name that runs well past the side panel's edge 12345";
    let mut lib = reverb_lib();
    put(&mut lib, &format!("{REVERB}/{long}.flmpst"), b"");
    let mut rig = Rig::with(reverb(), lib);
    rig.click_text(MY_FILES);
    let list = layout(SCREEN, 1.0, &metrics()).list;
    let (shown, rect, _) = (rig.texts().into_iter())
        .find(|(t, ..)| t.starts_with("A very"))
        .expect("drawn");
    assert!(shown.contains("...") && shown.ends_with("12345"), "{shown}");
    assert!(rect.right() <= list.right(), "{rect:?} in {list:?}");
}

#[test]
fn a_folder_opens_scrolled_to_its_top_with_the_search_kept() {
    let mut paths = vec!["My Presets/".to_owned(), format!("{REVERB}/")];
    for i in 0..30 {
        paths.push(format!("{REVERB}/Set {i:02}/"));
        for preset in ["Hall", "Plate"] {
            paths.push(format!("{REVERB}/Set {i:02}/{preset} {i:02}.flmpst"));
        }
    }
    let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
    let mut rig = Rig::with(reverb(), library(&paths));
    rig.click_text(MY_FILES);
    // The bar shown, empty: the folder's own rows.
    rig.click_text("Search");
    let list = layout(SCREEN, 1.0, &metrics()).list;
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -500.0),
        modifiers: Modifiers::NONE,
    };
    rig.frame(vec![Event::PointerMoved(list.center()), wheel]);
    rig.settle();
    assert!(!rig.shows("Set 00") && rig.shows("Set 20"), "scrolled");
    rig.click_text("Set 20");
    assert!(rig.shows("Clear"), "the bar stays");
    // The first row, under the search bar (which moves the list down 1.75 U), at the top of the list.
    let (m, row_h) = (metrics(), metrics().round_px(metrics().u(1.25)));
    let first = rig.text_at("Hall 20").y;
    assert!(
        (first - (list.top() + m.u(1.75) + 0.5 * row_h)).abs() < 1.0,
        "{first}"
    );
    // The tap took the keyboard from the field: Ctrl+F gives it back.
    rig.ctrl_f();
    rig.frame(vec![Event::Text("plate".into())]);
    rig.frame(vec![]);
    assert!(rig.shows("Plate 20") && !rig.shows("Hall 20"));
}

/// Original content for a GMSynth picker: categories and a loose preset, beside other files.
fn content_lib() -> MemoryLibrary {
    library(&[
        "Content/",
        "Content/GMSynth/",
        "Content/GMSynth/Keys/",
        "Content/GMSynth/Bass/",
        "Content/GMSynth/Bass/Gen Bass.flmpst",
        "Content/GMSynth/Loose.flmpst",
        "Content/GMSynth/notes.txt",
        "Content/More/",
        "Content/More/Arps/",
    ])
}

#[test]
fn the_root_lists_the_factory_folders_and_presets_after_my_files_and_default() {
    let lib = content_lib();
    let mut picker = reverb()
        .with_factory(["Content/GMSynth", "Content/More", "Content/Gone"].map(PathBuf::from));
    picker.read_factory(&lib);
    // "Top Level": every factory folder's folders, then their presets; `Content/More`'s after `Content/GMSynth`'s.
    assert_eq!(
        names(&picker, ""),
        [MY_FILES, DEFAULT, "Bass", "Keys", "Arps", "Loose"]
    );
    // Search keeps the factory rows that match, too.
    assert_eq!(
        names(&picker, "s"),
        [MY_FILES, "Bass", "Keys", "Arps", "Loose"]
    );
    // A factory folder opens like any other, under the root's title.
    open(&mut picker, &lib, "Bass");
    assert_eq!(names(&picker, ""), ["Gen Bass"]);
    assert_eq!(picker.header(), "     Reverb | Bass");
    // Without factory folders the root is as before.
    let mut plain = reverb();
    plain.read_factory(&lib);
    assert_eq!(names(&plain, ""), [MY_FILES, DEFAULT]);
}

#[test]
fn a_dw_programs_sample_folder_is_left_out() {
    let lib = library(&[
        "DW/",
        "DW/Guitar/",
        "DW/Guitar/Gen Guitar.dwp",
        "DW/Guitar/Gen Guitar/",
        "DW/Guitar/Gen Guitar/zone 1.wav",
        "DW/Guitar/Other/",
    ]);
    let mut picker = PresetPicker::new("DW Sampler", None, PRESET_EXT, true)
        .of_exts(&[PRESET_EXT, "dwp"])
        .of_type(1)
        .with_factory([PathBuf::from("DW")]);
    picker.read_factory(&lib);
    open(&mut picker, &lib, "Guitar");
    assert_eq!(names(&picker, ""), ["Other", "Gen Guitar"]);
}

/// The DW Sampler's picker (its presets, programs and audio files; any module type, as these files are only names)
/// over the factory bank `Bank`, with `my_files` as the user's folder.
fn dw_bank(my_files: Option<&str>) -> PresetPicker {
    PresetPicker::new("DW Sampler", my_files.map(PathBuf::from), PRESET_EXT, true)
        .of_exts(&[PRESET_EXT, "dwp", "wav"])
        .with_factory([PathBuf::from("Bank")])
}

#[test]
fn a_presets_sample_folder_is_left_out() {
    let lib = library(&[
        "Bank/",
        "Bank/Close Grand.flmpst",
        "Bank/Close Grand/",
        "Bank/Close Grand/Close Grand 1.wav",
        "Bank/Close Grand/Close Grand 10.wav",
        "Bank/Pianos/",
        "Bank/Pianos/Electric Piano Bright.flmpst",
        "Bank/Pianos/Electric Piano Bright/",
        "Bank/Pianos/Electric Piano Bright/Piano Electric (1).wav",
        "Bank/Pianos/Upright.dwp",
        "Bank/Pianos/Upright/",
    ]);
    let mut picker = dw_bank(None);
    picker.read_factory(&lib);
    assert_eq!(
        names(&picker, ""),
        [MY_FILES, DEFAULT, "Pianos", "Close Grand"]
    );
    open(&mut picker, &lib, "Pianos");
    assert_eq!(names(&picker, ""), ["Electric Piano Bright", "Upright"]);
}

#[test]
fn a_folder_of_samples_without_a_program_still_lists_them() {
    let lib = library(&[
        "Bank/",
        "Bank/Kicks/",
        "Bank/Kicks/Kick 1.wav",
        "Bank/Kicks/Kick 2.wav",
        // An audio file is no program: the folder of its name lists.
        "Bank/Snare.WAV",
        "Bank/Snare/",
    ]);
    let mut picker = dw_bank(None);
    picker.read_factory(&lib);
    assert_eq!(
        names(&picker, ""),
        [MY_FILES, DEFAULT, "Kicks", "Snare", "Snare"]
    );
    open(&mut picker, &lib, "Kicks");
    assert_eq!(names(&picker, ""), ["Kick 1", "Kick 2"]);
    // A picker that lists no programs (the sample picker's audio files) lists a program's folder: browsed as samples.
    let lib = library(&["Bank/", "Bank/X.dwp", "Bank/X/", "Bank/X/zone.wav"]);
    let mut samples =
        PresetPicker::new("Samples", None, "wav", false).with_factory([PathBuf::from("Bank")]);
    samples.read_factory(&lib);
    assert_eq!(names(&samples, ""), [MY_FILES, "X"]);
}

#[test]
fn my_files_leave_only_a_dw_programs_sample_folder_out() {
    let lib = library(&[
        "My Presets/",
        "My Presets/DW/",
        "My Presets/DW/My Grand.flmpst",
        "My Presets/DW/My Grand/",
        "My Presets/DW/My Grand/My Grand 1.wav",
        "My Presets/DW/Kit.DWP",
        "My Presets/DW/kit/",
        "My Presets/DW/Loops/",
    ]);
    let mut picker = dw_bank(Some("My Presets/DW"));
    open(&mut picker, &lib, MY_FILES);
    // The user's own folders: a folder beside a preset of its name is theirs, and lists; a DW program's is its
    // samples, as before, matched without case.
    assert_eq!(names(&picker, ""), ["Loops", "My Grand", "Kit", "My Grand"]);
    // A program the picker does not list (the sample picker's audio files) still has its folder left out.
    let lib = library(&["Mine/", "Mine/X.dwp", "Mine/X/", "Mine/X/zone.wav"]);
    let mut samples = PresetPicker::new("Samples", Some("Mine".into()), "wav", false);
    open(&mut samples, &lib, MY_FILES);
    assert_eq!(names(&samples, ""), Vec::<String>::new());
}

/// On screen, a factory bank's preset lists once, without the folder of its samples beside it.
#[test]
fn a_factory_banks_preset_shows_one_row_without_its_sample_folder() {
    let lib = library(&[
        "Bank/",
        "Bank/Pianos/",
        "Bank/Pianos/Close Grand.flmpst",
        "Bank/Pianos/Close Grand/",
        "Bank/Pianos/Close Grand/Close Grand 1.wav",
    ]);
    let mut rig = Rig::with(dw_bank(None), lib);
    rig.click_text("Pianos");
    assert!(rig.shows("Close Grand"), "{:?}", rig.texts());
    let rows = rig.texts().into_iter().filter(|(t, ..)| t == "Close Grand");
    assert_eq!(rows.count(), 1, "{:?}", rig.texts());
}

#[test]
fn a_folder_the_user_makes_beside_a_preset_of_its_name_lists() {
    let mut rig = files_rig();
    rig.pick("Create folder");
    rig.enter("A Room");
    assert!(rig.stored(REVERB).contains(&"A Room/".to_owned()));
    let rows = rig.texts().into_iter().filter(|(t, ..)| t == "A Room");
    assert_eq!(
        rows.count(),
        2,
        "the folder and the preset: {:?}",
        rig.texts()
    );
}

#[test]
fn a_sample_folder_matches_its_program_without_case() {
    let lib = library(&[
        "Bank/",
        "Bank/Close Grand.FLMPST",
        "Bank/close grand/",
        "Bank/Upright.dwp",
        "Bank/UPRIGHT/",
    ]);
    let mut picker = dw_bank(None);
    picker.read_factory(&lib);
    assert_eq!(
        names(&picker, ""),
        [MY_FILES, DEFAULT, "Close Grand", "Upright"]
    );
}

#[test]
fn my_files_sort_numbers_by_value_and_a_factory_folder_keeps_index_order() {
    let files = [
        "Set 10/",
        "Set 9/",
        "Take 10.flmpst",
        "Take 2.flmpst",
        "take 1.flmpst",
    ];
    let mut paths = vec!["My Presets/", "My Presets/DW/", "Bank/", "Bank/Set/"];
    let mine: Vec<String> = files.iter().map(|f| format!("My Presets/DW/{f}")).collect();
    let bank: Vec<String> = files.iter().map(|f| format!("Bank/Set/{f}")).collect();
    paths.extend(mine.iter().chain(&bank).map(String::as_str));
    let lib = library(&paths);
    let mut picker = dw_bank(Some("My Presets/DW"));
    open(&mut picker, &lib, MY_FILES);
    assert_eq!(
        names(&picker, ""),
        ["Set 9", "Set 10", "take 1", "Take 2", "Take 10"]
    );
    // A factory folder: the bytes of the paths.
    let mut picker = dw_bank(None);
    picker.read_factory(&lib);
    open(&mut picker, &lib, "Set");
    assert_eq!(
        names(&picker, ""),
        ["Set 10", "Set 9", "Take 10", "Take 2", "take 1"]
    );
}

#[test]
fn the_natural_compare_reads_digit_runs_as_numbers() {
    use std::cmp::Ordering::{Equal, Greater, Less};
    assert_eq!(natural("Grand 2", "Grand 10"), Less);
    assert_eq!(
        natural("grand 2", "Grand 2"),
        Greater,
        "a tie without case: bytewise"
    );
    assert_eq!(
        natural("Grand 02", "Grand 2"),
        Less,
        "leading zeros: bytewise"
    );
    assert_eq!(natural("a", "a"), Equal);
    assert_eq!(natural("a 9", "a"), Greater);
    // Longer than any integer type.
    assert_eq!(
        natural("x 99999999999999999999999", "x 100000000000000000000000"),
        Less
    );
    assert_eq!(natural("Piano (7)", "Piano (10)"), Less);
}

#[test]
fn the_factory_rows_show_and_pick_on_screen() {
    let lib = content_lib();
    let picker = reverb().with_factory([PathBuf::from("Content/GMSynth")]);
    let mut rig = Rig::with(picker, lib);
    assert!(rig.shows("Bass") && rig.shows("Loose"));
    assert_eq!(
        rig.click_text("Loose"),
        Some(PickerAction::Pick(PathBuf::from(
            "Content/GMSynth/Loose.flmpst"
        )))
    );
}

#[test]
fn minisynth_and_gmsynth_pickers_list_their_legacy_presets_too() {
    use crate::project_files::preset_exts;
    let flms = [&b"\x99FLS\r\n\x1a\nDATA\x04\0\0\x000rev"[..]].concat();
    let flgsynth = [&b"2nhC"[..], &[0; 1024]].concat();
    let mut lib = library(&["My Presets/", "My Presets/Synths/"]);
    let dir = "My Presets/Synths";
    for (name, bytes) in [
        ("Bass.flms", flms.clone()),
        ("Lead.flgsynth", flgsynth.clone()),
        (
            "Pad.flmpst",
            crate::flm::write_preset(&crate::song::ModuleInstance::new(2, 0)).unwrap(),
        ),
        // Named for one type, holding another: listed by what it holds only.
        ("Odd.flgsynth", flms),
        ("Junk.flms", b"not a preset".to_vec()),
    ] {
        put(&mut lib, &format!("{dir}/{name}"), &bytes);
    }
    let picker = |type_id: u16| {
        PresetPicker::new("Synth", Some(dir.into()), PRESET_EXT, true)
            .of_exts(preset_exts(type_id))
            .of_type(type_id)
    };
    let mut mini = picker(0);
    open(&mut mini, &lib, MY_FILES);
    assert_eq!(names(&mini, ""), ["Bass"]);
    let mut gm = picker(2);
    open(&mut gm, &lib, MY_FILES);
    assert_eq!(names(&gm, ""), ["Lead", "Pad"]);
    // The SoundFont Player's presets are its `.sf2` files too.
    assert_eq!(preset_exts(52), [PRESET_EXT, crate::project_files::SF2_EXT]);
    // Any other module lists `.flmpst` only.
    assert_eq!(preset_exts(6), [PRESET_EXT]);
}

/// The SoundFont Player's picker over `My Presets/SoundFont Player`, with `My SoundFonts` beside `My Files`.
fn soundfont_player() -> PresetPicker {
    use crate::project_files::{SF2_EXT, preset_exts};
    PresetPicker::new(
        "SoundFont Player",
        Some("My Presets/SoundFont Player".into()),
        PRESET_EXT,
        true,
    )
    .of_exts(preset_exts(52))
    .of_type(52)
    .with_root("My SoundFonts", Some("My SoundFonts".into()), &[SF2_EXT])
}

fn soundfont_player_lib() -> MemoryLibrary {
    let mut lib = library(&[
        "My Presets/",
        "My Presets/SoundFont Player/",
        "My SoundFonts/",
        "My SoundFonts/Keys/",
    ]);
    let preset = crate::flm::write_preset(&crate::song::ModuleInstance::new(52, 0)).unwrap();
    let files: [(&str, &[u8]); 7] = [
        ("My Presets/SoundFont Player/Warm.flmpst", &preset),
        // A `.sf2` among the presets lists in `My Files` too: the module's loader takes it, as the DW Sampler's takes
        // its audio files there (`project_files::fits`).
        ("My Presets/SoundFont Player/Stray.sf2", b"sf2"),
        ("My SoundFonts/Piano.sf2", b"sf2"),
        ("My SoundFonts/Pad.SF2", b"sf2"),
        ("My SoundFonts/Keys/Organ.sf2", b"sf2"),
        // Not SoundFonts: not listed in `My SoundFonts`, whatever they hold.
        ("My SoundFonts/Warm.flmpst", &preset),
        ("My SoundFonts/notes.txt", b"x"),
    ];
    for (path, bytes) in files {
        put(&mut lib, path, bytes);
    }
    lib
}

#[test]
fn the_soundfont_players_root_lists_my_soundfonts_beside_my_files_and_other_modules_do_not() {
    assert_eq!(
        names(&soundfont_player(), ""),
        [MY_FILES, "My SoundFonts", DEFAULT]
    );
    assert_eq!(names(&soundfont_player(), "sound"), ["My SoundFonts"]);
    // No user folder: the row is there, dimmed (as `My Files` is).
    let none = PresetPicker::new("SoundFont Player", None, PRESET_EXT, true).with_root(
        "My SoundFonts",
        None,
        &["sf2"],
    );
    assert_eq!(names(&none, ""), [MY_FILES, "My SoundFonts", DEFAULT]);
    assert_eq!(names(&reverb(), ""), [MY_FILES, DEFAULT]);
}

#[test]
fn my_files_lists_the_presets_and_my_soundfonts_the_sf2_files_of_any_case_down_its_folders() {
    let lib = soundfont_player_lib();
    let mut picker = soundfont_player();
    open(&mut picker, &lib, MY_FILES);
    assert_eq!(names(&picker, ""), ["Stray", "Warm"]);
    let mut picker = soundfont_player();
    open(&mut picker, &lib, "My SoundFonts");
    assert_eq!(names(&picker, ""), ["Keys", "Pad", "Piano"]);
    // Inside a folder of it: still its files, though the module type (and its `HEAD` check) is the scope's.
    open(&mut picker, &lib, "Keys");
    assert_eq!(names(&picker, ""), ["Organ"]);
}

#[test]
fn the_soundfont_players_root_lists_the_content_folders_soundfonts_as_factory_rows_read_only() {
    // The factory folders, with `My SoundFonts`: `My Files`, `My SoundFonts`, `Default`, then the content folder's
    // `SoundFonts` (its folders, then its `.sf2` files), in which no file action applies.
    let mut lib = soundfont_player_lib();
    for path in [
        "Original Content/",
        "Original Content/SoundFonts/",
        "Original Content/SoundFonts/Bass/",
        "Original Content/SoundFonts/Strings.sf2",
        "Original Content/SoundFonts/Bass/Fretless.sf2",
        "Original Content/SoundFonts/notes.txt",
    ] {
        match path.strip_suffix('/') {
            Some(folder) => lib.make_folder(Path::new(folder)).unwrap(),
            None => put(&mut lib, path, b"sf2"),
        }
    }
    let factory =
        || soundfont_player().with_factory([PathBuf::from("Original Content/SoundFonts")]);
    let mut picker = factory();
    assert_eq!(
        picker.names_in(&lib, &[]),
        [MY_FILES, "My SoundFonts", DEFAULT, "Bass", "Strings"]
    );
    assert_eq!(picker.names_in(&lib, &["Bass"]), ["Fretless"]);
    let mut rig = Rig::files(factory(), lib);
    rig.click_text("Bass");
    assert_eq!(rig.color("Select"), text().gamma_multiply(DISABLED_ALPHA));
    assert_eq!(
        rig.click_text("Fretless"),
        Some(PickerAction::Pick(
            "Original Content/SoundFonts/Bass/Fretless.sf2".into()
        ))
    );
}

#[test]
fn a_soundfont_is_picked_by_its_path_and_my_soundfonts_offers_no_file_action() {
    let mut rig = Rig::files(soundfont_player(), soundfont_player_lib());
    // At the root both are dimmed; in `My Files` they are on; in `My SoundFonts` they are dimmed again, which no
    // file action may touch (the SoundFonts are the SoundFont picker's files: it offers none).
    rig.click_text("My Files");
    assert_eq!(rig.color("Select"), text());
    rig.back();
    rig.click_text("My SoundFonts");
    assert_eq!(rig.color("Select"), text().gamma_multiply(DISABLED_ALPHA));
    assert!(rig.shows("Piano") && !rig.shows("Warm"));
    rig.options();
    assert!(!rig.shows("Select all") && !rig.shows("Create folder"));
    // The ≡ button is inert there: nothing is open, so the click picks.
    assert_eq!(
        rig.click_text("Piano"),
        Some(PickerAction::Pick("My SoundFonts/Piano.sf2".into()))
    );
}

// ---- the file actions, offscreen ----

/// Reverb's picker with the file actions, in `My Files`, in select mode.
fn files_rig() -> Rig {
    let mut rig = Rig::files(reverb(), reverb_lib());
    rig.click_text(MY_FILES);
    rig.click_text("Select");
    rig
}

impl Rig {
    /// Opens the ≡ menu, the left column's fourth button.
    fn options(&mut self) {
        self.click(layout(SCREEN, 1.0, &metrics()).button(OPTIONS));
        // A new popup area sits out its first frame.
        self.frame(vec![]);
    }

    /// Picks `item` from the ≡ menu.
    fn pick(&mut self, item: &str) {
        self.options();
        assert!(
            self.shows(item),
            "{item} not in the menu: {:?}",
            self.texts()
        );
        self.click_text(item);
        // What it opens (a message) sits out its first frame.
        self.frame(vec![]);
    }

    fn key(&mut self, key: Key) {
        self.frame(key_tap(key, Modifiers::NONE));
        self.frame(vec![]);
    }

    /// Types `text` into the focused field, then Enter.
    fn enter(&mut self, text: &str) {
        self.frame(vec![Event::Text(text.into())]);
        self.key(Key::Enter);
        self.frame(vec![]);
    }

    /// The names in `dir` of the library, sorted, folders ending in `/`.
    fn stored(&self, dir: &str) -> Vec<String> {
        let list = self.lib.lib.list(Some(Path::new(dir))).unwrap();
        let mut names: Vec<_> = (list.into_iter())
            .map(|e| e.name + if e.is_folder { "/" } else { "" })
            .collect();
        names.sort();
        names
    }

    /// The labels of the open ≡ menu, in order, each with whether it is enabled.
    fn menu(&mut self) -> Vec<(String, bool)> {
        self.options();
        let labels = [
            "Select all",
            "Restore",
            "Delete permanently",
            "Create folder",
            "Delete",
            "Copy",
            "Cut",
            "Paste",
            "Export",
            "Rename",
            "Show in file browser...",
        ];
        let mut shown: Vec<_> = (self.texts().into_iter())
            .filter(|(t, ..)| labels.contains(&t.as_str()))
            .map(|(t, rect, color)| {
                (
                    rect.top(),
                    t,
                    color == theme_by_id(0).colors.background.to_color32(),
                )
            })
            .collect();
        shown.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.key(Key::Escape);
        shown.into_iter().map(|(_, t, on)| (t, on)).collect()
    }
}

fn on(label: &str, enabled: bool) -> (String, bool) {
    (label.to_owned(), enabled)
}

#[test]
fn select_and_the_options_menu_live_in_the_users_files_only() {
    // Without the file actions (the sample and SoundFont pickers) there are neither.
    let rig = Rig::new(reverb());
    assert!(!rig.shows("Select"));
    // At the root, whose rows are no files of the user's, both are dimmed and inert.
    let mut rig = Rig::files(reverb(), reverb_lib());
    assert_eq!(rig.color("Select"), text().gamma_multiply(DISABLED_ALPHA));
    rig.options();
    assert!(!rig.shows("Select all"));
    rig.click_text(MY_FILES);
    assert_eq!(rig.color("Select"), text());
    rig.options();
    assert!(rig.shows("Select all") && rig.shows("Create folder"));
}

#[test]
fn select_and_the_options_menu_are_dimmed_inside_a_factory_folder() {
    let mut lib = reverb_lib();
    for folder in ["Content/", "Content/Reverb/", "Content/Reverb/Halls/"] {
        lib.make_folder(Path::new(folder.trim_end_matches('/')))
            .unwrap();
    }
    let picker = reverb().with_factory([PathBuf::from("Content/Reverb")]);
    let mut rig = Rig::files(picker, lib);
    rig.click_text("Halls");
    assert!(
        rig.shows("     Reverb | Halls"),
        "the factory folder is open"
    );
    let dimmed = text().gamma_multiply(DISABLED_ALPHA);
    assert_eq!(rig.color("Select"), dimmed);
    rig.click_text("Select");
    rig.options();
    assert!(!rig.shows("Select all"), "the ≡ menu does not open");
    // In `My Files` both work.
    rig.back();
    rig.click_text(MY_FILES);
    assert_eq!(rig.color("Select"), text());
}

#[test]
fn the_options_menu_has_the_file_browsers_items_for_the_selection() {
    let mut rig = files_rig();
    assert_eq!(
        rig.menu(),
        [
            on("Select all", true),
            on("Create folder", true),
            on("Paste", false)
        ]
    );
    rig.click_text("A Room");
    let mut one = vec![
        on("Select all", true),
        on("Create folder", true),
        on("Delete", true),
        on("Copy", true),
        on("Cut", true),
        on("Paste", false),
        on("Export", true),
        on("Rename", true),
    ];
    if crate::ui::main_menu::FILE_MANAGER.is_some() {
        one.push(on("Show in file browser...", true));
    }
    assert_eq!(rig.menu(), one);
    // Two: Rename needs exactly one.
    rig.click_text("b Hall");
    assert!(rig.menu().contains(&on("Rename", false)));
}

#[test]
fn a_selected_row_is_drawn_inverted_and_a_click_in_select_mode_toggles_it() {
    let mut rig = files_rig();
    let ink = theme_by_id(0).colors.menu_fill.to_color32();
    assert_eq!(rig.click_text("A Room"), None, "selected, not picked");
    assert_eq!(rig.color("A Room"), ink);
    assert_eq!(rig.click_text("Archive"), None, "a folder too, not opened");
    assert!(rig.shows("b Hall") && rig.color("Archive") == ink);
    rig.click_text("A Room");
    assert_eq!(rig.color("A Room"), text());
    // Out of select mode a click picks again.
    rig.click_text("Select");
    let room = PathBuf::from("My Presets/Reverb/A Room.flmpst");
    assert_eq!(rig.click_text("A Room"), Some(PickerAction::Pick(room)));
}

#[test]
fn select_all_selects_every_row_the_search_keeps() {
    // The search lists the folders and presets below the folder.
    let mut rig = files_rig();
    rig.click_text("Search");
    rig.frame(vec![Event::Text("o".into())]);
    rig.frame(vec![]);
    rig.pick("Select all");
    let ink = theme_by_id(0).colors.menu_fill.to_color32();
    for name in ["Vocal", "A Room", "Long"] {
        assert_eq!(rig.color(name), ink, "{name}");
    }
    assert_eq!(rig.picker.picked().len(), 3);
}

#[test]
fn create_folder_asks_for_a_name_and_a_taken_one_shows_the_message() {
    let mut rig = files_rig();
    rig.pick("Create folder");
    assert!(rig.shows("Enter folder name"));
    rig.enter("Halls");
    assert!(!rig.shows("Enter folder name") && rig.shows("Halls"));
    assert!(rig.stored(REVERB).contains(&"Halls/".to_owned()));
    // Taken: the message over the panel, which stays open; `OK` closes the message only.
    rig.pick("Create folder");
    rig.enter("Archive");
    assert!(rig.shows("The folder already exists") && rig.shows("OK"));
    assert_eq!(rig.click_text("OK"), None);
    assert!(!rig.shows("The folder already exists") && rig.shows("Halls"));
    // Escape drops the field and keeps the picker.
    rig.pick("Create folder");
    rig.key(Key::Escape);
    assert!(!rig.shows("Enter folder name") && rig.shows("Halls"));
    assert_eq!(rig.action, None);
}

#[test]
fn rename_keeps_the_extension_and_refuses_a_taken_name() {
    let mut rig = files_rig();
    rig.click_text("A Room");
    rig.pick("Rename");
    assert!(rig.shows("Enter name"));
    for _ in "Room".chars() {
        rig.key(Key::Backspace);
    }
    rig.enter("Big Hall");
    assert!(
        rig.shows("A Big Hall") && !rig.shows("A Room"),
        "{:?}",
        rig.texts()
    );
    let stored = rig.stored(REVERB);
    assert!(
        stored.contains(&"A Big Hall.flmpst".to_owned()),
        "{stored:?}"
    );
    rig.click_text("A Big Hall");
    rig.pick("Rename");
    for _ in "A Big Hall".chars() {
        rig.key(Key::Backspace);
    }
    rig.enter("b Hall");
    assert!(rig.shows("The file already exists"));
    assert!(rig.stored(REVERB).contains(&"A Big Hall.flmpst".to_owned()));
}

#[test]
fn copy_paste_keeps_both_and_cut_paste_moves_into_a_folder() {
    let mut rig = files_rig();
    rig.click_text("A Room");
    rig.pick("Copy");
    rig.pick("Paste");
    assert!(rig.shows("A Room (1)") && rig.shows("A Room"));
    // A copy pastes as often as wanted.
    rig.pick("Paste");
    assert!(rig.shows("A Room (2)"));
    rig.click_text("b Hall");
    rig.pick("Cut");
    rig.click_text("Select");
    rig.click_text("Archive");
    rig.pick("Paste");
    assert!(rig.shows("b Hall"));
    assert_eq!(rig.stored(&format!("{REVERB}/Archive")), ["b Hall.flmpst"]);
    assert!(!rig.stored(REVERB).contains(&"b Hall.flmpst".to_owned()));
    // A cut pastes once.
    assert!(rig.menu().contains(&on("Paste", false)));
}

#[test]
fn delete_puts_a_preset_into_recently_deleted_and_restore_brings_it_back() {
    let mut rig = files_rig();
    rig.click_text("Long");
    rig.pick("Delete");
    assert!(!rig.shows("Long"));
    let bin = "/Recently Deleted/My Presets/Reverb";
    assert_eq!(rig.stored(bin), ["Long.FLMPST"]);
    // The root lists the bin's copy of My Files once it exists.
    rig.back();
    assert!(rig.shows(RECENTLY_DELETED) && rig.shows(DEFAULT));
    rig.click_text(RECENTLY_DELETED);
    assert!(rig.shows("Long"));
    // The bin's note, cut in its middle on this small screen: its `30 days` stays.
    let texts = rig.texts();
    let note = texts
        .iter()
        .find(|(t, ..)| t.starts_with("     Recently De"));
    assert!(
        note.is_some_and(|(t, ..)| t.ends_with("after 30 days)")),
        "{texts:?}"
    );
    // In the bin: Restore and Delete permanently, no Create folder or Delete.
    assert_eq!(
        rig.menu(),
        [
            on("Select all", true),
            on("Restore", false),
            on("Delete permanently", false),
            on("Paste", false)
        ]
    );
    rig.click_text("Long");
    rig.pick("Restore");
    assert!(!rig.shows("Long") && rig.stored(bin).is_empty());
    rig.back();
    rig.click_text(MY_FILES);
    assert!(rig.shows("Long"));
    assert!(rig.stored(REVERB).contains(&"Long.FLMPST".to_owned()));
}

#[test]
fn delete_permanently_asks_first_and_removes_on_yes_only() {
    let mut rig = files_rig();
    rig.click_text("Long");
    rig.click_text("b Hall");
    rig.pick("Delete");
    rig.back();
    rig.click_text(RECENTLY_DELETED);
    rig.pick("Select all");
    rig.pick("Delete permanently");
    assert!(rig.shows("Delete 2 files?") && rig.shows("Yes") && rig.shows("No"));
    rig.click_text("No");
    let bin = "/Recently Deleted/My Presets/Reverb";
    assert_eq!(rig.stored(bin), ["Long.FLMPST", "b Hall.flmpst"]);
    // Escape answers no too.
    rig.pick("Select all");
    rig.pick("Delete permanently");
    rig.key(Key::Escape);
    assert_eq!(rig.stored(bin).len(), 2);
    assert_eq!(rig.action, None, "the picker stays open");
    rig.pick("Select all");
    rig.pick("Delete permanently");
    rig.click_text("Yes");
    assert!(rig.stored(bin).is_empty());
    assert!(!rig.shows("Long") && !rig.shows("b Hall"));
}

#[test]
fn a_popup_is_modal_over_the_panel() {
    let mut rig = files_rig();
    rig.click_text("Select");
    rig.options();
    // A click on a row outside the menu (its right end, clear of the menu) closes the menu, and picks nothing.
    let right = pos2(SCREEN.left() + 540.0, rig.text_at("A Room").y);
    assert_eq!(rig.click(right), None);
    assert!(!rig.shows("Select all") && rig.picker.selection.is_none());
    let room = PathBuf::from("My Presets/Reverb/A Room.flmpst");
    assert_eq!(rig.click_text("A Room"), Some(PickerAction::Pick(room)));
    // Escape closes the menu, not the picker.
    let mut rig = files_rig();
    rig.options();
    rig.key(Key::Escape);
    assert!(!rig.shows("Select all") && rig.action.is_none());
    assert!(rig.shows("Long"));
}

// ---- our own factory bank beside imported content ----

/// Our bank for the Reverb picker: a folder, presets whose index and A-Z orders differ, and rows imported content
/// has too (`Arps`, a folder; `Loose`, a preset) or has as another kind (`Bass`, a folder there).
fn built_in() -> crate::content::Factory {
    use crate::content::{Entry, Factory};
    let rels = [
        "Presets/Reverb/Hall/b Room.flmpst",
        "Presets/Reverb/Hall/Big.flmpst",
        "Presets/Reverb/Arps/Up.flmpst",
        "Presets/Reverb/b Soft.flmpst",
        "Presets/Reverb/Bright.flmpst",
        "Presets/Reverb/Loose.flmpst",
        "Presets/Reverb/Bass.flmpst",
    ];
    Factory::new(rels.map(|rel| Entry::new(rel, move || rel.into())).into())
}

fn ours(rel: &str) -> PathBuf {
    Path::new(crate::content::ROOT).join(rel)
}

#[test]
fn our_bank_lists_after_default_in_index_order_and_gives_way_to_the_original_content() {
    let factory = built_in();
    let mut lib = content_lib();
    put(&mut lib, "Content/GMSynth/Bass/b Low.flmpst", b"x");
    let union = crate::content::Union {
        user: &mut lib,
        factory: &factory,
    };
    let folders = [
        ours("Presets/Reverb"),
        "Content/GMSynth".into(),
        "Content/More".into(),
    ];
    let mut picker = reverb().with_factory(folders);
    picker.read_factory(&union);
    // "Top Level": the folders, then the presets; each kind ours first, then imported content's folders in turn,
    // each in index order (bytewise: `Bright` before `b Soft`). Ours named as one of imported content's rows of the
    // same kind give way.
    assert_eq!(
        names(&picker, ""),
        [
            MY_FILES, DEFAULT, "Hall", "Bass", "Keys", "Arps", "Bass", "Bright", "b Soft", "Loose"
        ]
    );
    assert_eq!(names(&picker, "b"), ["Bass", "Bass", "Bright", "b Soft"]);
    // Our folder opens like the others, read-only, in index order.
    open(&mut picker, &union, "Hall");
    assert_eq!(names(&picker, ""), ["Big", "b Room"]);
    assert_eq!(picker.header(), "     Reverb | Hall");
    assert!(picker.in_factory());
    // A pick reads back through the union.
    let big = picker.path_of("Big").unwrap();
    assert_eq!(big, ours("Presets/Reverb/Hall/Big.flmpst"));
    assert_eq!(union.read(&big).unwrap(), b"Presets/Reverb/Hall/Big.flmpst");
    // The imported content's folders keep the index order too; the user's keep A-Z.
    let bass = Path::new("Content/GMSynth/Bass");
    picker.folders = vec![folder("Bass", bass)];
    picker.listing = Some(read(&union, bass, &picker.exts, None, true));
    assert_eq!(names(&picker, ""), ["Gen Bass", "b Low"]);
    let mut mine = library(&[
        "My Presets/",
        "My Presets/Reverb/",
        "My Presets/Reverb/Gen Bass.flmpst",
        "My Presets/Reverb/b Low.flmpst",
    ]);
    let mut picker = reverb();
    open(&mut picker, &mine, MY_FILES);
    assert_eq!(names(&picker, ""), ["b Low", "Gen Bass"]);
    // With no bank of ours (the empty stubs), the root is imported content's alone, as before.
    let empty = crate::content::Factory::new(Vec::new());
    let union = crate::content::Union {
        user: &mut mine,
        factory: &empty,
    };
    let mut picker = reverb().with_factory([ours("Presets/Reverb")]);
    picker.read_factory(&union);
    assert_eq!(names(&picker, ""), [MY_FILES, DEFAULT]);
}

// ---- picks, the audition strip and the sample strip ----

use crate::sample_cache::Waveform;
use crate::ui::picker_preview::{LOADING, Play, SampleStrip, Sound};
use eframe::egui::{TouchDeviceId, TouchId, TouchPhase};

const A_ROOM: &str = "My Presets/Reverb/A Room.flmpst";

impl Rig {
    /// Row `i`'s rect as laid out last frame.
    fn row(&self, i: usize) -> Rect {
        let id = Id::new("preset_picker").with(("row", i));
        self.ctx.read_response(id).expect("the row is drawn").rect
    }

    fn press(&mut self, pos: Pos2) -> Option<PickerAction> {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]);
        self.action.clone()
    }

    fn release(&mut self, pos: Pos2) -> Option<PickerAction> {
        self.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        self.action.clone()
    }

    fn touch(&mut self, finger: u64, phase: TouchPhase, pos: Pos2) -> Option<PickerAction> {
        self.frame(vec![Event::Touch {
            device_id: TouchDeviceId(0),
            id: TouchId(finger),
            phase,
            pos,
            force: None,
        }]);
        self.action.clone()
    }

    fn enter_key(&mut self, pressed: bool) {
        self.frame(vec![Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
    }

    /// `My Files` open, `A Room` (row 2) picked, the audition strip on and laid out over it.
    fn auditioning() -> Self {
        let mut rig = Rig::new(reverb());
        rig.click_text(MY_FILES);
        rig.click_text("A Room");
        rig.picker.audition_on();
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert!(rig.picker.take_sounds().is_empty());
        rig
    }
}

fn pick_a_room() -> Option<PickerAction> {
    Some(PickerAction::Pick(A_ROOM.into()))
}

/// The alpha of the flash over the row showing `name` (the last translucent fill no taller than a row over the name's
/// centre: a selected row's bar is opaque, a focus ring unfilled), if one is painted.
fn flash(rig: &Rig, name: &str) -> Option<u8> {
    let (at, row) = (rig.text_at(name), metrics().u(2.0));
    (rig.shapes.iter().rev())
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.rect.contains(at) && r.rect.height() < row => {
                Some(r.fill.a())
            }
            _ => None,
        })
        .find(|a| (1..255).contains(a))
}

#[test]
fn a_tap_on_an_item_row_flashes_it_and_a_re_tap_restarts_the_flash() {
    // A row flashes on pick.
    let mut rig = Rig::new(reverb());
    rig.click_text(MY_FILES);
    assert_eq!(flash(&rig, "A Room"), None);
    assert_eq!(rig.click_text("A Room"), pick_a_room());
    let first = flash(&rig, "A Room").expect("the picked row flashes");
    for _ in 0..9 {
        rig.frame(vec![]);
    }
    let faded = flash(&rig, "A Room").expect("it fades over 0.3 s");
    assert!(faded < first, "{faded} < {first}");
    assert!(
        rig.ctx.requested_repaint_last_pass(),
        "it repaints while it runs"
    );
    // A re-tap of the row already picked starts it over.
    assert_eq!(rig.click_text("A Room"), pick_a_room());
    assert_eq!(flash(&rig, "A Room"), Some(first));
    for _ in 0..18 {
        rig.frame(vec![]);
    }
    assert_eq!(flash(&rig, "A Room"), None, "over after 0.3 s");
    // egui keeps a requested repaint for the pass after the request.
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(
        !rig.ctx.requested_repaint_last_pass(),
        "no repaint once it is over"
    );
    // The audition strip takes a re-tap (it plays the row): that flashes it all the same.
    let mut rig = Rig::auditioning();
    for _ in 0..20 {
        rig.frame(vec![]);
    }
    assert_eq!(flash(&rig, "A Room"), None);
    let row = rig.row(2);
    assert_eq!(rig.press(row.center()), None);
    assert_eq!(rig.picker.take_sounds(), [Sound::On(54)]);
    rig.frame(vec![]);
    assert_eq!(flash(&rig, "A Room"), Some(first));
    rig.release(row.center());
    // Enter on the strip plays the row: that flashes it too.
    let mut rig = Rig::new(reverb());
    rig.focus = true;
    rig.click_text(MY_FILES);
    rig.click_text("A Room");
    rig.picker.audition_on();
    for _ in 0..20 {
        rig.frame(vec![]);
    }
    assert_eq!(flash(&rig, "A Room"), None);
    rig.enter_key(true);
    assert_eq!(rig.picker.take_sounds(), [Sound::On(60)]);
    rig.frame(vec![]);
    assert_eq!(flash(&rig, "A Room"), Some(first));
    rig.enter_key(false);
}

#[test]
fn a_refresh_drops_the_flash() {
    // Showing the search bar is a list refresh (`SidePanel::refreshed`): the list is the same, and the flash goes.
    let mut rig = Rig::new(reverb());
    rig.click_text(MY_FILES);
    rig.click_text("A Room");
    assert!(flash(&rig, "A Room").is_some());
    rig.click_text("Search");
    assert!(rig.shows("A Room"));
    assert_eq!(flash(&rig, "A Room"), None);
}

#[test]
fn a_tap_in_select_mode_selects_the_row_and_does_not_flash_it() {
    // A tap in select mode picks nothing, so nothing flashes.
    let mut rig = Rig::files(reverb(), reverb_lib());
    rig.click_text(MY_FILES);
    rig.click_text("Select");
    assert_eq!(rig.click_text("A Room"), None);
    let ink = theme_by_id(0).colors.menu_fill.to_color32();
    assert_eq!(rig.color("A Room"), ink, "selected");
    assert_eq!(flash(&rig, "A Room"), None);
}

#[test]
fn the_first_tap_on_a_row_only_picks_it() {
    let mut rig = Rig::new(reverb());
    rig.click_text(MY_FILES);
    assert_eq!(rig.click_text("A Room"), pick_a_room());
    assert_eq!(rig.picker.take_sounds(), []);
    assert_eq!(rig.color("A Room"), text(), "drawn as any row");
    // Until the caller turns the strip on (an instrument's load), a tap on it picks it again.
    assert_eq!(rig.click_text("A Room"), pick_a_room());
    assert_eq!(rig.picker.take_sounds(), []);
}

#[test]
fn a_touch_on_the_picked_row_plays_the_note_of_its_x_while_down_and_never_picks() {
    let mut rig = Rig::auditioning();
    let row = rig.row(2);
    for (x, note) in [(row.left(), 24), (row.center().x, 54), (row.right(), 84)] {
        let at = pos2(x, row.center().y);
        assert_eq!(rig.press(at), None);
        assert_eq!(rig.picker.take_sounds(), [Sound::On(note)], "x {x}");
        // Held while down; moving changes nothing.
        rig.frame(vec![Event::PointerMoved(row.center())]);
        rig.frame(vec![]);
        assert_eq!(rig.picker.take_sounds(), []);
        assert_eq!(rig.release(row.center()), None, "the list never saw it");
        assert_eq!(rig.picker.take_sounds(), [Sound::Off(note)]);
        rig.frame(vec![]);
    }
    // Another row still picks.
    let hall = PathBuf::from("My Presets/Reverb/b Hall.flmpst");
    assert_eq!(rig.click_text("b Hall"), Some(PickerAction::Pick(hall)));
    assert_eq!(rig.picker.take_sounds(), []);
}

#[test]
fn a_second_finger_while_one_holds_the_note_re_picks_the_row() {
    let mut rig = Rig::auditioning();
    let row = rig.row(2);
    let (mid, right) = (row.center(), pos2(row.right(), row.center().y));
    assert_eq!(rig.touch(1, TouchPhase::Start, mid), None);
    assert_eq!(rig.picker.take_sounds(), [Sound::On(54)]);
    assert_eq!(rig.touch(2, TouchPhase::Start, right), pick_a_room());
    assert_eq!(rig.picker.take_sounds(), []);
    rig.touch(2, TouchPhase::End, right);
    assert_eq!(rig.picker.take_sounds(), []);
    rig.touch(1, TouchPhase::End, mid);
    assert_eq!(rig.picker.take_sounds(), [Sound::Off(54)]);
}

#[test]
fn enter_holds_note_60_until_it_is_released() {
    let mut rig = Rig::auditioning();
    rig.enter_key(true);
    assert_eq!(rig.picker.take_sounds(), [Sound::On(60)]);
    rig.frame(vec![]);
    assert_eq!(rig.picker.take_sounds(), []);
    rig.enter_key(false);
    assert_eq!(rig.picker.take_sounds(), [Sound::Off(60)]);
}

#[test]
fn enter_plays_on_the_audition_strip_only_while_the_strip_has_the_focus() {
    // As the chord editor's key-held button: the strip takes the focus after each pick, Enter held there plays note
    // 60, and the focus moving off it lets the note go.
    let mut rig = Rig::new(reverb());
    rig.focus = true;
    rig.click_text(MY_FILES);
    rig.click_text("A Room");
    rig.picker.audition_on();
    rig.frame(vec![]);
    let strip = Some(Id::new("preset_picker").with("audition"));
    let focused = |rig: &Rig| crate::ui::focus::focused(&rig.ctx);
    assert_eq!(focused(&rig), strip);
    // Tab with Enter still down.
    rig.enter_key(true);
    assert_eq!(rig.picker.take_sounds(), [Sound::On(60)]);
    rig.frame(key_tap(Key::Tab, Modifiers::NONE));
    assert_ne!(focused(&rig), strip);
    assert_eq!(rig.picker.take_sounds(), [Sound::Off(60)]);
    rig.enter_key(false);
    assert_eq!(rig.picker.take_sounds(), []);
    // Enter at the root.
    crate::ui::focus::clear(&rig.ctx);
    rig.enter_key(true);
    rig.enter_key(false);
    assert_eq!(rig.picker.take_sounds(), []);
    // A second pick: the focus goes to the strip again.
    let hall = PathBuf::from("My Presets/Reverb/b Hall.flmpst");
    assert_eq!(rig.click_text("b Hall"), Some(PickerAction::Pick(hall)));
    rig.picker.audition_on();
    rig.frame(vec![]);
    assert_eq!(focused(&rig), strip);
    rig.enter_key(true);
    assert_eq!(rig.picker.take_sounds(), [Sound::On(60)]);
}

#[test]
fn every_list_refresh_turns_the_strip_off_and_releases_its_note() {
    // A search.
    let mut rig = Rig::auditioning();
    rig.press(rig.row(2).center());
    rig.click_text("Search");
    assert!(!rig.picker.auditioning());
    assert_eq!(rig.picker.take_sounds(), [Sound::On(54), Sound::Off(54)]);
    rig.release(rig.row(2).center());
    // A folder left and entered again.
    let mut rig = Rig::auditioning();
    rig.back();
    rig.click_text(MY_FILES);
    assert!(!rig.picker.auditioning());
    assert_eq!(
        rig.click_text("A Room"),
        pick_a_room(),
        "the tap falls to the row"
    );
    assert_eq!(rig.picker.take_sounds(), []);
    // Select mode.
    let mut rig = Rig::files(reverb(), reverb_lib());
    rig.click_text(MY_FILES);
    rig.click_text("A Room");
    rig.picker.audition_on();
    rig.click_text("Select");
    assert!(!rig.picker.auditioning());
}

#[test]
fn while_the_module_loads_the_picked_row_pulses_loading() {
    let mut rig = Rig::auditioning();
    assert!(!rig.shows(LOADING));
    rig.picker.set_loading(true);
    rig.frame(vec![]);
    let row = rig.row(2);
    assert!(row.contains(rig.text_at(LOADING)));
    // Touches still play while it loads.
    rig.press(row.center());
    assert_eq!(rig.picker.take_sounds(), [Sound::On(54)]);
    rig.release(row.center());
    rig.picker.set_loading(false);
    rig.frame(vec![]);
    assert!(!rig.shows(LOADING));
}

#[test]
fn a_pick_loading_with_no_audition_strip_pulses_too_and_a_tap_picks_it_again() {
    // A Slicer's or a sample picker's pick decoding on the worker; those never audition.
    let mut rig = Rig::new(reverb());
    rig.click_text(MY_FILES);
    rig.click_text("A Room");
    rig.picker.set_loading(true);
    rig.frame(vec![]);
    let row = rig.row(2);
    assert!(row.contains(rig.text_at(LOADING)));
    assert_eq!(rig.click_text("A Room"), pick_a_room());
    assert_eq!(rig.picker.take_sounds(), []);
    rig.picker.set_loading(false);
    rig.frame(vec![]);
    assert!(!rig.shows(LOADING));
}

fn wave(frames: usize) -> Waveform {
    Waveform::new(std::sync::Arc::new(honeydew_audio::sample::SampleData {
        sample_rate: 1000,
        left: vec![0.5; frames].into(),
        right: None,
        meta: Default::default(),
    }))
}

#[test]
fn the_sample_strip_sits_at_the_bottom_and_its_controls_stop_and_play() {
    let m = metrics();
    let (theme, at) = (theme_by_id(0), layout(SCREEN, 1.0, &m));
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let [strip, play, wave_rect, name, progress] = SampleStrip::layout(at.panel, &cx);
    assert_eq!(strip.height(), m.u(1.75));
    assert_eq!(
        (strip.bottom(), strip.width()),
        (at.panel.bottom(), at.panel.width())
    );
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
    assert!(near(play.width(), m.u(1.2)) && near(play.height(), m.u(1.2)));
    assert!(near(play.center().x, strip.left() + m.u(1.0)));
    assert!(near(play.center().y, strip.center().y));
    assert!(near(wave_rect.left(), strip.left() + m.u(2.0)));
    assert!(near(wave_rect.width(), strip.width() - m.u(2.4)));
    assert!(near(wave_rect.height(), m.u(1.0)));
    assert!((name.width() - 5.0 * progress.width()).abs() < 0.01);

    let mut rig = Rig::new(reverb());
    rig.click_text(MY_FILES);
    rig.picker
        .set_sample(SampleStrip::new("Kick".into(), Some(wave(1000))));
    assert_eq!(rig.picker.take_plays(), [Play::From(0)]);
    rig.frame(vec![]);
    assert!(name.contains(rig.text_at("Kick")));
    assert!(progress.contains(rig.text_at("0:00")));
    // The list shrinks by the strip (egui reads the rects of the frame before the last).
    rig.frame(vec![]);
    let list = rig.ctx.read_response(Id::new("preset_picker").with("list"));
    assert_eq!(list.unwrap().rect.bottom(), strip.top());
    // Play stops it while it plays, else plays from the start.
    rig.click(play.center());
    assert_eq!(rig.picker.take_plays(), [Play::Stop]);
    rig.click(play.center());
    assert_eq!(rig.picker.take_plays(), [Play::From(0)]);
    // A touch on the waveform stops; its release plays from there.
    let mid = wave_rect.center();
    rig.press(mid);
    assert_eq!(rig.picker.take_plays(), [Play::Stop]);
    rig.release(mid);
    assert_eq!(rig.picker.take_plays(), [Play::From(500)]);
    // A new pick stops the one playing; a refresh (a folder left) stops it; so does closing.
    rig.picker
        .set_sample(SampleStrip::new("Snare".into(), Some(wave(1000))));
    assert_eq!(rig.picker.take_plays(), [Play::Stop, Play::From(0)]);
    rig.back();
    assert_eq!(rig.picker.take_plays(), [Play::Stop]);
    rig.click_text(MY_FILES);
    rig.click(play.center());
    rig.picker.take_plays();
    rig.picker.closing();
    assert_eq!(rig.picker.take_plays(), [Play::Stop]);
}

#[test]
fn a_sample_that_ends_stops_by_itself() {
    let mut rig = Rig::new(reverb());
    rig.picker
        .set_sample(SampleStrip::new("Tick".into(), Some(wave(10))));
    rig.picker.take_plays();
    // 10 frames at 1000 Hz: over within a frame of 1/60 s.
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.picker.closing();
    assert_eq!(rig.picker.take_plays(), [], "nothing left to stop");
}

#[test]
fn closing_releases_the_held_note() {
    let mut rig = Rig::auditioning();
    rig.press(rig.row(2).center());
    rig.picker.closing();
    assert_eq!(rig.picker.take_sounds(), [Sound::On(54), Sound::Off(54)]);
}

// ---- the Instruments picker ----

/// Two categories as [`categories::index`] makes them, their presets by their real paths in index order: `Bass`'s
/// first preset's path sorts after its second's, so a sort would show.
fn instrument_categories() -> Vec<categories::Category> {
    let category = |name: &str, paths: &[&str]| categories::Category {
        folder: folder(name, &Path::new(categories::ROOT).join(name)),
        items: (paths.iter())
            .map(|p| SongEntry {
                name: Path::new(p).file_stem().unwrap().to_string_lossy().into(),
                path: p.into(),
                is_folder: false,
                recency: 0,
            })
            .collect(),
    };
    vec![
        category(
            "Bass",
            &[
                "<built-in>/Presets/TransistorBass/Acid Drip.flmpst",
                "<built-in>/Presets/GMSynth/Bass/Round.flmpst",
            ],
        ),
        category(
            "Keyboard",
            &["<built-in>/Presets/MiniSynth/Keys/Felt.flmpst"],
        ),
    ]
}

/// The whole user folder: presets of an instrument, of the Slicer and of an effect; a legacy preset; an audio file, an
/// `.instr`, a song, a text file and an empty folder.
fn user_folder() -> MemoryLibrary {
    let mut lib = library(&[
        "Empty/",
        "My Presets/",
        "My Presets/GMSynth/",
        "My Samples/",
        "My Instruments/",
        "My Songs/",
        "My Presets/GMSynth/Old.flgsynth",
        "My Samples/Hit.wav",
        "My Instruments/Piano.instr",
        "My Songs/Beat.flm",
        "notes.txt",
    ]);
    let preset =
        |t: u16| crate::flm::write_preset(&crate::song::ModuleInstance::new(t, 2)).unwrap();
    put(&mut lib, "My Presets/GMSynth/Mine.flmpst", &preset(2));
    put(&mut lib, "My Presets/GMSynth/Cut.flmpst", &preset(33));
    put(&mut lib, "My Presets/GMSynth/Hall.flmpst", &preset(32));
    lib
}

fn instruments() -> PresetPicker {
    PresetPicker::new(
        INSTRUMENTS_TITLE,
        Some("".into()),
        categories::EXTS[0],
        false,
    )
    .of_exts(&categories::EXTS)
    .instruments(instrument_categories())
}

/// The names and places of the search rows of `picker` for `query`.
fn found(picker: &PresetPicker, lib: &dyn SongLibrary, query: &str) -> Vec<(String, String)> {
    let rows = picker.found_rows(query, picker.found(lib));
    (rows.into_iter())
        .map(|i| match i {
            Item::Found(f) => (f.entry.name.clone(), f.place.clone()),
            other => panic!("{other:?}"),
        })
        .collect()
}

#[test]
fn the_instruments_root_lists_my_files_then_the_categories_and_no_default() {
    let picker = instruments();
    assert_eq!(picker.root_title(), "Instruments");
    assert_eq!(names(&picker, ""), [MY_FILES, "Bass", "Keyboard"]);
    // The root's search, closed, still filters the rows (open, it lists the presets below: see the search tests).
    assert_eq!(names(&picker, "key"), ["Keyboard"]);
}

#[test]
fn a_category_keeps_its_index_order_and_picks_a_presets_real_path() {
    let lib = user_folder();
    let mut picker = instruments();
    assert_eq!(picker.names_in(&lib, &["Bass"]), ["Acid Drip", "Round"]);
    let round = picker.path_of("Round").unwrap();
    assert_eq!(
        round,
        Path::new("<built-in>/Presets/GMSynth/Bass/Round.flmpst")
    );
    // Read-only: no file action.
    assert!(picker.in_factory());
}

#[test]
fn my_files_is_the_whole_user_folder_listing_every_folder_and_only_instrument_files_audio_and_instr()
 {
    let lib = user_folder();
    let names = |path: &[&str]| instruments().names_in(&lib, path);
    assert_eq!(
        names(&[MY_FILES]),
        [
            "Empty",
            "My Instruments",
            "My Presets",
            "My Samples",
            "My Songs"
        ]
    );
    // An instrument's `.flmpst` and a legacy preset; not the Slicer's nor an effect's.
    assert_eq!(names(&[MY_FILES, "My Presets", "GMSynth"]), ["Mine", "Old"]);
    assert_eq!(names(&[MY_FILES, "My Samples"]), ["Hit"]);
    assert_eq!(names(&[MY_FILES, "My Instruments"]), ["Piano"]);
    // A song is no instrument file; an empty folder lists nothing (no placeholder either: see below).
    assert_eq!(names(&[MY_FILES, "My Songs"]), Vec::<String>::new());
    assert_eq!(names(&[MY_FILES, "Empty"]), Vec::<String>::new());
}

#[test]
fn the_instruments_search_lists_every_preset_below_the_open_folder_with_its_folders_path() {
    let lib = user_folder();
    let mut picker = instruments();
    // At the root: `My Files` and what is below it (a walk, each folder followed by its rows, folders first, `A-Z`),
    // then each category and its presets.
    let all = found(&picker, &lib, "");
    let at = |name: &str, place: &str| (name.to_owned(), place.to_owned());
    assert_eq!(
        all,
        [
            at(MY_FILES, ""),
            at("Empty", "My Files"),
            at("My Instruments", "My Files"),
            at("Piano", "My Files/My Instruments"),
            at("My Presets", "My Files"),
            at("GMSynth", "My Files/My Presets"),
            at("Mine", "My Files/My Presets/GMSynth"),
            at("Old", "My Files/My Presets/GMSynth"),
            at("My Samples", "My Files"),
            at("Hit", "My Files/My Samples"),
            at("My Songs", "My Files"),
            at("Bass", ""),
            at("Acid Drip", "Bass"),
            at("Round", "Bass"),
            at("Keyboard", ""),
            at("Felt", "Keyboard"),
        ]
    );
    // The query keeps the rows whose name holds it, without case, folders too.
    assert_eq!(
        found(&picker, &lib, "O"),
        [
            at("Piano", "My Files/My Instruments"),
            at("Old", "My Files/My Presets/GMSynth"),
            at("My Songs", "My Files"),
            at("Round", "Bass"),
            at("Keyboard", ""),
        ]
    );
    // Below a category: its presets only.
    picker.names_in(&lib, &["Bass"]);
    picker.found = OnceCell::new();
    assert_eq!(
        found(&picker, &lib, ""),
        [at("Acid Drip", "Bass"), at("Round", "Bass")]
    );
}

/// The size of the font `text` was drawn in.
fn font_size(rig: &Rig, text: &str) -> f32 {
    let size = rig.shapes.iter().find_map(|c| match &c.shape {
        egui::Shape::Text(t) if t.galley.text() == text => {
            Some(t.galley.job.sections[0].format.font_id.size)
        }
        _ => None,
    });
    size.unwrap_or_else(|| panic!("{text} not drawn"))
}

/// The height of row `i` of the open folder.
fn row_h(rig: &Rig, i: usize) -> f32 {
    let id = Id::new("preset_picker").with(("row", i));
    rig.ctx
        .read_response(id)
        .expect("the row is drawn")
        .rect
        .height()
}

#[test]
fn instrument_rows_are_1_5_u_tall_and_folders_and_presets_alike_in_fontsize_4() {
    let m = metrics();
    let mut rig = Rig::with(instruments(), user_folder());
    // The presets take the folders' FontSize 4 too.
    let (folder_pt, preset_pt) = (
        m.font_id(FontSize::Pt18).size,
        m.font_id(FontSize::Pt18).size,
    );
    assert_eq!(font_size(&rig, MY_FILES), folder_pt);
    assert_eq!(font_size(&rig, "Bass"), folder_pt);
    assert_eq!(row_h(&rig, 1), m.round_px(m.u(1.5)));
    rig.click_text("Bass");
    assert_eq!(font_size(&rig, "Acid Drip"), preset_pt);
    assert_eq!(row_h(&rig, 0), m.round_px(m.u(1.5)));
    // No type or folder icon: nothing but text is drawn in a row.
    let id = Id::new("preset_picker").with(("row", 1_usize));
    assert!(!rig.draws_in(rig.ctx.read_response(id).unwrap().rect));
    rig.back();
    rig.click_text(MY_FILES);
    assert_eq!(font_size(&rig, "My Presets"), folder_pt);
    rig.click_text("My Samples");
    assert_eq!(font_size(&rig, "Hit"), preset_pt);
}

#[test]
fn an_open_search_shows_2_2_u_rows_with_the_folder_path_small_and_dimmed_under_the_name_and_picks_its_path()
 {
    let m = metrics();
    let mut rig = Rig::with(instruments(), user_folder());
    rig.click_text("Search");
    rig.frame(vec![Event::Text("ro".into())]);
    // The edit filters on the next frame, whose rows egui reports a frame later.
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(
        rig.shows("Round")
            && rig.shows("Bass")
            && !rig.shows("Acid Drip")
            && !rig.shows("Keyboard")
    );
    assert_eq!(row_h(&rig, 0), m.round_px(m.u(2.2)));
    // A found preset's name in FontSize 4, as every row's.
    assert_eq!(font_size(&rig, "Round"), m.font_id(FontSize::Pt18).size);
    let (name, place) = (rig.find("Round").0, rig.find("Bass").0);
    assert!(place.top() > name.top(), "the path is under the name");
    assert_eq!(font_size(&rig, "Bass"), m.font_id(FontSize::Pt8).size);
    assert_eq!(rig.color("Bass"), text().gamma_multiply(0.6));
    let round = PathBuf::from("<built-in>/Presets/GMSynth/Bass/Round.flmpst");
    assert_eq!(rig.click_text("Round"), Some(PickerAction::Pick(round)));
}

#[test]
fn the_instruments_left_column_is_search_with_select_and_the_menu_inside_my_files_only() {
    let mut rig = Rig::files(instruments(), user_folder());
    let column = |rig: &Rig| (rig.shows("Search"), rig.shows("Select"));
    assert_eq!(column(&rig), (true, false));
    assert!(!rig.shows("Sort") && !rig.shows("Filter"));
    rig.click_text("Bass");
    assert_eq!(column(&rig), (true, false));
    rig.back();
    rig.click_text(MY_FILES);
    assert_eq!(column(&rig), (true, true));
    assert_eq!(rig.color("Select"), text());
    rig.options();
    assert!(rig.shows("Create folder"));
}

#[test]
fn an_empty_folder_shows_no_rows_and_no_placeholder() {
    let mut rig = Rig::with(instruments(), user_folder());
    rig.click_text(MY_FILES);
    rig.click_text("Empty");
    // The header (its breadcrumb, shortened) and `Search`: no row, no text in place of one.
    let texts: Vec<String> = rig.texts().into_iter().map(|t| t.0).collect();
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert!(texts[0].starts_with("     Instruments") && texts[1] == "Search");
}

#[test]
fn a_held_ctrl_up_leaves_one_folder_and_its_repeats_nothing() {
    // A press of a key already held is ignored.
    let mut rig = Rig::new(reverb());
    let tap = key_tap(Key::ArrowUp, Modifiers::CTRL | Modifiers::COMMAND);
    let up = |pressed: bool| tap[usize::from(!pressed)].clone();
    rig.click_text(MY_FILES);
    rig.click_text("Vocal");
    rig.frame(vec![up(true)]);
    rig.frame(vec![]);
    assert!(rig.shows("Archive") && !rig.shows("Plate"));
    // Held: the auto-repeats stay in `My Files`.
    rig.frame(vec![up(true)]);
    rig.frame(vec![]);
    assert!(rig.shows("Archive"), "a repeat left the folder");
    rig.frame(vec![up(false)]);
    rig.frame(vec![up(true)]);
    rig.frame(vec![]);
    assert!(rig.shows(DEFAULT));
}

// ---- the search bar's Escape order and select mode ----

impl Rig {
    fn escape(&mut self) {
        self.frame(key_tap(Key::Escape, Modifiers::NONE));
    }

    /// `startSearch`.
    fn ctrl_f(&mut self) {
        self.frame(key_tap(Key::F, Modifiers::COMMAND));
    }

    fn selecting(&self) -> bool {
        self.picker.selection.is_some()
    }
}

#[test]
fn escape_stops_select_mode_before_it_closes_the_picker() {
    let mut rig = files_rig();
    assert!(rig.selecting());
    rig.escape();
    assert_eq!(rig.action, None);
    rig.frame(vec![]);
    assert!(!rig.selecting());
    assert_eq!(rig.color("Select"), text(), "unlit");
    rig.escape();
    assert_eq!(rig.action, Some(PickerAction::Close));
}

#[test]
fn escape_with_the_bar_shown_hides_it_and_leaves_select_mode_then_closes() {
    let mut rig = files_rig();
    rig.click_text("Search");
    assert!(rig.shows("Clear"));
    rig.click_text("Select");
    assert!(rig.selecting() && rig.shows("Clear"));
    rig.escape();
    assert_eq!(rig.action, None);
    rig.frame(vec![]);
    assert!(!rig.shows("Clear") && !rig.selecting());
    rig.escape();
    assert_eq!(rig.action, Some(PickerAction::Close));
}

#[test]
fn ctrl_f_leaves_select_mode() {
    let mut rig = files_rig();
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(rig.shows("Clear") && !rig.selecting());
    assert_eq!(rig.color("Select"), text());
}

#[test]
fn the_instruments_bar_with_an_empty_query_lists_the_open_folder_and_a_query_searches_below_it() {
    // An empty query is the normal folder listing (1.5 U rows), even with the bar open.
    let m = metrics();
    let mut rig = Rig::with(instruments(), user_folder());
    rig.click_text("Search");
    assert!(rig.shows(MY_FILES) && rig.shows("Keyboard") && !rig.shows("Round"));
    assert_eq!(row_h(&rig, 0), m.round_px(m.u(1.5)));
    rig.frame(vec![Event::Text("ro".into())]);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.shows("Round") && !rig.shows(MY_FILES));
    assert_eq!(row_h(&rig, 0), m.round_px(m.u(2.2)));
}

// ---- the recursive search of every side picker ----

impl Rig {
    /// Shows the search bar and types `query`, then waits for the next frame's filter and egui's row rects.
    fn search(&mut self, query: &str) {
        self.ctrl_f();
        self.frame(vec![Event::Text(query.into())]);
        self.frame(vec![]);
        self.frame(vec![]);
    }
}

#[test]
fn a_search_lists_the_presets_below_the_open_folder_with_their_folders_path_in_any_picker() {
    let m = metrics();
    // At the root, `My Files`' presets are searched too, into its folders.
    let mut rig = Rig::new(reverb());
    rig.search("plate");
    assert!(rig.shows("Plate") && rig.shows("My Files/Vocal"));
    assert!(!rig.shows(MY_FILES) && !rig.shows(DEFAULT));
    assert_eq!(row_h(&rig, 0), m.round_px(m.u(2.2)));
    let (name, place) = (rig.find("Plate").0, rig.find("My Files/Vocal").0);
    assert!(place.top() > name.top(), "the path is under the name");
    assert_eq!(
        font_size(&rig, "My Files/Vocal"),
        m.font_id(FontSize::Pt8).size
    );
    assert_eq!(rig.color("My Files/Vocal"), text().gamma_multiply(0.6));
    let plate = PathBuf::from("My Presets/Reverb/Vocal/Plate.flmpst");
    assert_eq!(rig.click_text("Plate"), Some(PickerAction::Pick(plate)));
    // Inside a folder: what is below it, the path still from the root.
    let mut rig = Rig::new(reverb());
    rig.click_text(MY_FILES);
    rig.search("plate");
    assert!(rig.shows("Plate") && rig.shows("My Files/Vocal") && !rig.shows("Vocal"));
    // A factory folder's presets, by their folder.
    let picker = reverb().with_factory([PathBuf::from("Content/GMSynth")]);
    let mut rig = Rig::with(picker, content_lib());
    rig.search("gen");
    assert!(rig.shows("Gen Bass") && !rig.shows("Loose"));
    // Its path is its folder's, under the name.
    assert!(rig.find("Bass").0.top() > rig.find("Gen Bass").0.top());
    let gen_bass = PathBuf::from("Content/GMSynth/Bass/Gen Bass.flmpst");
    assert_eq!(
        rig.click_text("Gen Bass"),
        Some(PickerAction::Pick(gen_bass))
    );
}

#[test]
fn after_an_instrument_pick_the_focus_is_on_the_audition_strip_where_ctrl_f_does_nothing() {
    let mut rig = Rig::new(reverb());
    rig.click_text(MY_FILES);
    let room = PathBuf::from("My Presets/Reverb/A Room.flmpst");
    assert_eq!(rig.click_text("A Room"), Some(PickerAction::Pick(room)));
    // The caller loaded it into an instrument: the strip goes on over the row.
    rig.picker.audition_on();
    rig.frame(vec![]);
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(!rig.shows("Clear"), "the focus is outside the browser");
    // Back in the list, Ctrl+F searches.
    crate::ui::focus::focus(&rig.ctx, Id::new("preset_picker").with("rows"));
    rig.frame(vec![]);
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(rig.shows("Clear"));
}

#[test]
fn ctrl_f_does_nothing_while_the_options_menu_is_open() {
    let mut rig = files_rig();
    rig.options();
    assert!(rig.shows("Select all"));
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(!rig.shows("Clear") && rig.shows("Select all"));
}

#[test]
fn a_found_folder_shows_with_its_path_and_a_tap_opens_it_and_ends_the_search() {
    let m = metrics();
    let mut rig = Rig::new(reverb());
    rig.search("voc");
    assert!(
        rig.shows("Vocal") && rig.shows(MY_FILES),
        "the folder, its path `My Files` under it"
    );
    assert!(rig.find(MY_FILES).0.top() > rig.find("Vocal").0.top());
    assert_eq!(row_h(&rig, 0), m.round_px(m.u(2.2)));
    assert_eq!(font_size(&rig, "Vocal"), m.font_id(FontSize::Pt18).size);
    assert_eq!(rig.click_text("Vocal"), None);
    // Inside it, with the trail from the root: the search is over.
    assert!(rig.shows("Plate") && rig.shows("     Reverb | My Files | Vocal"));
    assert!(!rig.shows("Clear"));
    rig.back();
    assert!(rig.shows("Archive") && rig.shows("A Room"));
}

#[test]
fn a_deleted_preset_leaves_the_search_results() {
    let mut rig = files_rig();
    rig.search("room");
    assert!(rig.shows("A Room"));
    rig.click_text("Select");
    rig.click_text("A Room");
    rig.pick("Delete");
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.shows("Clear") && !rig.shows("A Room"));
}

#[test]
fn escape_in_the_name_field_drops_only_the_field() {
    // `Create folder` out of select mode: the picker stays open.
    let mut rig = Rig::files(reverb(), reverb_lib());
    rig.click_text(MY_FILES);
    rig.pick("Create folder");
    assert!(rig.shows("Enter folder name"));
    rig.frame(key_tap(Key::Escape, Modifiers::NONE));
    assert_eq!(rig.action, None, "the picker stays open");
    rig.frame(vec![]);
    assert!(!rig.shows("Enter folder name") && rig.shows("A Room"));
    // `Rename` in select mode: select mode stays, its row still selected.
    let mut rig = files_rig();
    rig.click_text("A Room");
    rig.pick("Rename");
    assert!(rig.shows("Enter name"));
    rig.frame(key_tap(Key::Escape, Modifiers::NONE));
    assert_eq!(rig.action, None);
    rig.frame(vec![]);
    assert!(!rig.shows("Enter name"));
    let ink = theme_by_id(0).colors.menu_fill.to_color32();
    assert_eq!(rig.color("A Room"), ink, "still selected");
}

#[test]
fn ctrl_up_in_the_name_field_leaves_no_folder() {
    // `leaveFolder` waits while a text field has the keyboard.
    let mut rig = Rig::files(reverb(), reverb_lib());
    rig.click_text(MY_FILES);
    rig.pick("Create folder");
    rig.frame(key_tap(Key::ArrowUp, Modifiers::CTRL | Modifiers::COMMAND));
    assert_eq!(rig.action, None);
    rig.frame(vec![]);
    assert!(rig.shows("Enter folder name") && rig.shows("A Room"));
    assert!(!rig.shows(DEFAULT), "still in My Files");
}

#[test]
fn ctrl_f_does_nothing_while_a_name_is_asked() {
    let mut rig = files_rig();
    rig.pick("Create folder");
    assert!(rig.shows("Enter folder name"));
    rig.ctrl_f();
    rig.frame(vec![]);
    assert!(!rig.shows("Clear") && rig.shows("Enter folder name"));
}

// ---- the file actions over a folder and what is in it; found folders; the strip's focus ----

/// Reverb's picker with the file actions in `My Files`, searching for `l`: `Vocal`, its `Plate`, `b Hall` and `Long`,
/// all selected.
fn all_found_l() -> Rig {
    let mut rig = files_rig();
    rig.search("l");
    assert!(rig.shows("Vocal") && rig.shows("Plate") && rig.shows("b Hall") && rig.shows("Long"));
    rig.pick("Select all");
    assert_eq!(rig.picker.picked().len(), 4);
    rig
}

#[test]
fn delete_over_a_found_folder_and_its_preset_deletes_the_folder_once() {
    let mut rig = all_found_l();
    rig.pick("Delete");
    assert!(!rig.shows("OK"), "no failure");
    rig.escape();
    rig.frame(vec![]);
    assert!(rig.shows("Archive") && rig.shows("A Room"));
    for gone in ["Vocal", "b Hall", "Long"] {
        assert!(!rig.shows(gone), "{gone}");
    }
}

#[test]
fn copy_and_paste_over_a_found_folder_and_its_preset_copy_the_folder_once() {
    let mut rig = all_found_l();
    rig.pick("Copy");
    // Out of the search and select mode, into `Archive`.
    rig.escape();
    rig.frame(vec![]);
    rig.click_text("Archive");
    rig.pick("Paste");
    assert!(!rig.shows("OK"), "no failure");
    assert!(rig.shows("Vocal") && rig.shows("b Hall") && rig.shows("Long"));
    assert!(!rig.shows("Plate"), "inside the copy of `Vocal` only");
    rig.click_text("Vocal");
    assert!(rig.shows("Plate"));
}

#[test]
fn in_select_mode_a_found_folder_toggles_and_does_not_open() {
    let mut rig = files_rig();
    rig.search("voc");
    rig.click_text("Select");
    assert_eq!(rig.click_text("Vocal"), None);
    let ink = theme_by_id(0).colors.menu_fill.to_color32();
    assert_eq!(rig.color("Vocal"), ink, "selected");
    assert!(rig.shows("Clear") && !rig.shows("Plate"), "not opened");
}

#[test]
fn the_instruments_search_skips_the_users_bin() {
    use crate::ui::main_menu::NativeLibrary;
    let tmp = crate::project_files::content::tests::Temp::new("sb-instruments-bin");
    for (dir, file) in [
        ("My Instruments", "Piano.instr"),
        (RECENTLY_DELETED, "Gone Piano.instr"),
    ] {
        std::fs::create_dir_all(tmp.0.join(dir)).unwrap();
        std::fs::write(tmp.0.join(dir).join(file), b"x").unwrap();
    }
    let lib = NativeLibrary::user_folder(tmp.0.clone()).with_exts(&categories::EXTS);
    let picker = PresetPicker::new(
        INSTRUMENTS_TITLE,
        Some(tmp.0.clone()),
        categories::EXTS[0],
        false,
    )
    .of_exts(&categories::EXTS)
    .instruments(Vec::new());
    let names: Vec<String> = found(&picker, &lib, "piano")
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(names, ["Piano"]);
}

#[test]
fn the_audition_strip_keeps_the_focus_while_its_row_is_scrolled_out_of_view() {
    let mut paths = vec!["My Presets/".to_owned(), format!("{REVERB}/")];
    paths.extend((0..40).map(|i| format!("{REVERB}/P {i:02}.flmpst")));
    let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
    let mut rig = Rig::with(reverb(), library(&paths));
    rig.focus = true;
    rig.click_text(MY_FILES);
    let p00 = PathBuf::from(format!("{REVERB}/P 00.flmpst"));
    assert_eq!(rig.click_text("P 00"), Some(PickerAction::Pick(p00)));
    rig.picker.audition_on();
    rig.frame(vec![]);
    let strip = Id::new("preset_picker").with("audition");
    assert_eq!(crate::ui::focus::focused(&rig.ctx), Some(strip));
    let list = layout(SCREEN, 1.0, &metrics()).list;
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -2000.0),
        modifiers: Modifiers::NONE,
    };
    rig.frame(vec![Event::PointerMoved(list.center()), wheel]);
    rig.settle();
    assert!(!rig.shows("P 00"), "scrolled out of view");
    assert_eq!(crate::ui::focus::focused(&rig.ctx), Some(strip));
}

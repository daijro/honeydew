//! The SoundFont picker, which the SoundFont Player's `Patch` spinner opens, as the side browser panel.
//!
//! - **The patch picker.** Opened on the module's SoundFont, it starts in its patches: titled with the SoundFont's file
//!   name, the patch names in file order (`Sort` `Default`), or by name (`A-Z`, [`PatchOrdering`]), which `Search`
//!   filters. The current patch is the current row, scrolled into view as it opens. A patch is the pick, and the
//!   picker stays in the patches, the pick now current. Back goes to the files.
//! - **The files**, where it opens without a SoundFont: a [`PresetPicker`]'s root and folders over `My SoundFonts`,
//!   titled [`TITLE`]: `My Files`, then the folders and the `.sf2` and `.sfz` files by name. No `Default`. No factory
//!   SoundFonts are shipped; with imported content in the content folder the root then lists its `SoundFonts`, read-only
//!   ([`SoundFontPicker::with_factory`]). A SoundFont opens in its patches. An SFZ is one patch, named after its file:
//!   it opens in it, and is the pick at once. One that cannot be read is the pick, with its error: the caller loads it,
//!   and the panel shows the error.
//!
//! The caller keeps a [`SoundFontPicker`] while it is open and loads what [`SoundFontPicker::show`] returns.

use std::path::PathBuf;

use eframe::egui::{Context, Id, Pos2};

use crate::project_files::{SF2_EXT, SOUNDFONT_EXTS, is_sfz};
use crate::sf2::{Preset, Sf2Error};
use crate::ui::UiCtx;
use crate::ui::main_menu::SongLibrary;
use crate::ui::modules_browser::{
    Chrome, PanelClick, PanelRow, RowKind, SideButton, SidePanel, crumb_text, layout, sort_menu,
};
use crate::ui::preset_picker::{PickerAction, PresetPicker, natural};

/// The root's title: the factory folder's name.
pub const TITLE: &str = "SoundFonts";

/// The patch picker's orderings, each with its `SoundFontPatchOrdering` id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PatchOrdering {
    /// File order.
    #[default]
    Default = 0,
    /// By name, natural alphanumeric ([`natural`]).
    AZ = 1,
}

impl PatchOrdering {
    const MENU: [(Self, &'static str); 2] = [(Self::Default, "Default"), (Self::AZ, "A-Z")];

    /// Its stored `SoundFontPatchOrdering`.
    pub fn id(self) -> i32 {
        self as i32
    }

    /// The ordering stored as `id`: `A-Z`'s, else `Default` (−1, unset, too).
    pub fn from_id(id: i32) -> Self {
        match id == Self::AZ.id() {
            true => Self::AZ,
            false => Self::Default,
        }
    }
}

/// What the user did in the picker this frame. A close or a failed SoundFont is the end of it: drop it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoundFontAction {
    /// The close X, a click outside the panel, or Escape.
    Close,
    /// Patch `index` of `patches`, the patches of the SoundFont at `path` (as its library lists it). The picker stays
    /// open.
    Patch {
        path: PathBuf,
        patches: Vec<Preset>,
        index: usize,
    },
    /// The SoundFont at `path` could not be read.
    Failed { path: PathBuf, error: Sf2Error },
}

/// An open SoundFont: its path, its patches, the current one, and the panel listing them.
struct Open {
    path: PathBuf,
    patches: Vec<Preset>,
    current: Option<usize>,
    panel: SidePanel,
}

impl Open {
    fn new(path: PathBuf, patches: Vec<Preset>, current: Option<usize>, panel: SidePanel) -> Self {
        let mut open = Self {
            path,
            patches,
            current,
            panel,
        };
        open.panel.reveal = true;
        open
    }
}

/// The open picker's state. Make a fresh one each time it opens.
pub struct SoundFontPicker {
    files: PresetPicker,
    open: Option<Open>,
    /// The patches' ordering: set it from `SoundFontPatchOrdering` as the picker opens.
    pub sort: PatchOrdering,
    /// A pick from `Sort`'s menu, the active ordering included: take it and store it as `SoundFontPatchOrdering`.
    pub sort_picked: Option<PatchOrdering>,
    /// `Sort`'s menu, open, centred on its button.
    sort_menu: Option<Pos2>,
}

impl SoundFontPicker {
    /// A picker over `my_files` (`My SoundFonts`; `None` dims `My Files`), at its root, or in the patches of
    /// `current`: the module's SoundFont, its path as the library lists it, its patches and the current patch.
    pub fn new(my_files: Option<PathBuf>, current: Option<(PathBuf, Vec<Preset>, usize)>) -> Self {
        Self {
            files: PresetPicker::new(TITLE, my_files, SF2_EXT, false).of_exts(&SOUNDFONT_EXTS),
            open: current.map(|(path, patches, patch)| {
                Open::new(path, patches, Some(patch), SidePanel::default())
            }),
            sort: PatchOrdering::Default,
            sort_picked: None,
            sort_menu: None,
        }
    }

    /// Lists the SoundFonts in `folder` (the content folder's `SoundFonts`) at the root, after `My Files`.
    pub fn with_factory(mut self, folder: Option<PathBuf>) -> Self {
        self.files = self.files.with_factory(folder);
        self
    }

    /// Shows the picker over everything else, listing from `library`; call it every frame while it is open. Modal,
    /// and it takes Escape, as the preset picker does.
    pub fn show(
        &mut self,
        ctx: &Context,
        cx: &UiCtx,
        library: &dyn SongLibrary,
    ) -> Option<SoundFontAction> {
        let Some(open) = &mut self.open else {
            let path = match self.files.show(ctx, cx, library)? {
                PickerAction::Pick(path) => path,
                // `Default` is not listed, nor `Import` offered, and the files are no nested list.
                PickerAction::Close
                | PickerAction::Default
                | PickerAction::Import
                | PickerAction::Back => {
                    return Some(SoundFontAction::Close);
                }
            };
            let read = library.read(&path).map_err(|_| Sf2Error::NotFound);
            match read.and_then(|bytes| crate::sf2::patches(&path, &bytes)) {
                Ok(patches) if is_sfz(&path) => {
                    let open = Open::new(
                        path.clone(),
                        patches.clone(),
                        Some(0),
                        SidePanel::in_place(),
                    );
                    self.open = Some(open);
                    return Some(SoundFontAction::Patch {
                        path,
                        patches,
                        index: 0,
                    });
                }
                Ok(patches) => {
                    self.open = Some(Open::new(path, patches, None, SidePanel::in_place()));
                    return None;
                }
                Err(error) => return Some(SoundFontAction::Failed { path, error }),
            }
        };
        // The files' own id: one panel area for both, as egui hides a new area for its first frame.
        let id = Id::new("preset_picker");
        // Before the panel, which would take Escape: the menu takes it first.
        if let Some(at) = self.sort_menu {
            let result = sort_menu(ctx, cx, id, at, &PatchOrdering::MENU, self.sort);
            self.sort_picked = result.selected.or(self.sort_picked);
            if let Some(sort) = result.selected.filter(|&s| s != self.sort) {
                // A sort is a list refresh, from the top.
                self.sort = sort;
                open.panel.refreshed();
                open.panel.scroll_y = 0.0;
            }
            if result.close {
                self.sort_menu = None;
            }
        }
        let title = open.path.file_stem().unwrap_or_default().to_string_lossy();
        let header = crumb_text([title.as_ref()]);
        let chrome = Chrome {
            id,
            title: &header,
            back: true,
            buttons: &[SideButton::Search, SideButton::Plain("Sort", true)],
            popup: self.sort_menu.is_some(),
        };
        let (patches, current) = (&open.patches, open.current);
        let click = open.panel.show(ctx, cx, &chrome, |query| {
            let rows = matching(patches, query, self.sort).into_iter();
            rows.map(|i| PanelRow {
                name: &patches[i].name,
                folder: false,
                sub: None,
                kind: RowKind::Enabled(i),
                picked: current == Some(i),
            })
            .collect()
        });
        match click? {
            PanelClick::Close => Some(SoundFontAction::Close),
            // `Sort`, the one button it hands back.
            PanelClick::Button(i) => {
                self.sort_menu = Some(layout(ctx.content_rect(), 1.0, cx.m).button(i));
                None
            }
            PanelClick::Back => {
                self.open = None;
                None
            }
            // It asks for no name and has no select mode.
            PanelClick::Enter | PanelClick::Context(..) => None,
            PanelClick::Drag(_) => None,
            PanelClick::Row(index) => {
                open.current = Some(index);
                Some(SoundFontAction::Patch {
                    path: open.path.clone(),
                    patches: open.patches.clone(),
                    index,
                })
            }
        }
    }
}

/// The indices of the patches whose name holds `query`, ignoring case, in file order or by name (`sort`).
fn matching(patches: &[Preset], query: &str, sort: PatchOrdering) -> Vec<usize> {
    let query = query.to_lowercase();
    let holds = |(_, p): &(usize, &Preset)| p.name.to_lowercase().contains(&query);
    let mut found: Vec<usize> = (patches.iter().enumerate())
        .filter(holds)
        .map(|(i, _)| i)
        .collect();
    if sort == PatchOrdering::AZ {
        found.sort_by(|&a, &b| natural(&patches[a].name, &patches[b].name));
    }
    found
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use eframe::egui::epaint::ClippedShape;
    use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2};

    use super::*;
    use crate::metrics::{DEFAULT_UI_SCALE, FontSize, Metrics};
    use crate::sf2::tests::soundfont;
    use crate::theme::theme_by_id;
    use crate::ui::main_menu::MemoryLibrary;
    use crate::ui::preset_picker::MY_FILES;

    const DIR: &str = "My SoundFonts";

    fn piano() -> Vec<u8> {
        soundfont(&[("Upright", 0, 0), ("Grand", 1, 0), ("Bright", 2, 0)])
    }

    /// `My SoundFonts` with a folder holding `Piano.sf2`, a SoundFont in upper case, a broken one, one cut off in its
    /// samples and a text file.
    fn library() -> MemoryLibrary {
        let mut lib = MemoryLibrary::default();
        lib.make_folder(Path::new("My SoundFonts/Keys")).unwrap();
        let cut = crate::sf2::tests::cut_off_samples(&[("Warm", 0, 0)]);
        let files: [(&str, Vec<u8>); 5] = [
            ("My SoundFonts/Keys/Piano.sf2", piano()),
            ("My SoundFonts/Pad.SF2", soundfont(&[("Warm", 0, 0)])),
            ("My SoundFonts/Broken.sf2", b"junk".to_vec()),
            ("My SoundFonts/Cut.sf2", cut),
            ("My SoundFonts/notes.txt", b"x".to_vec()),
        ];
        for (path, bytes) in files {
            // `store` takes song names only: store one, then move it into place.
            lib.store("x.flm", &bytes).unwrap();
            lib.move_to(Path::new("x.flm"), Path::new(path)).unwrap();
        }
        lib
    }

    struct Rig {
        ctx: egui::Context,
        picker: SoundFontPicker,
        lib: MemoryLibrary,
        time: f64,
        action: Option<SoundFontAction>,
        shapes: Vec<ClippedShape>,
    }

    impl Rig {
        /// `picker` just opened over [`library`], run until the slide-in is over.
        fn new(picker: SoundFontPicker) -> Self {
            let mut rig = Self {
                ctx: egui::Context::default(),
                picker,
                lib: library(),
                time: 0.0,
                action: None,
                shapes: Vec::new(),
            };
            for _ in 0..30 {
                rig.frame(vec![]);
            }
            rig
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 1.0 / 60.0;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 800.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let (theme, m) = (
                theme_by_id(0),
                Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE),
            );
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            let (picker, lib) = (&mut self.picker, &self.lib);
            let mut action = None;
            let output = self.ctx.run(raw, |ctx| action = picker.show(ctx, &cx, lib));
            self.action = action;
            self.shapes = output.shapes;
        }

        /// Returns what the release frame returned, and lets the next frame draw what the click changed.
        fn click(&mut self, pos: Pos2) -> Option<SoundFontAction> {
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

        fn click_text(&mut self, text: &str) -> Option<SoundFontAction> {
            let at = self.find(text).center();
            self.click(at)
        }

        /// The header's breadcrumb (the text after its 5 spaces) and where it is drawn.
        fn header(&self) -> (String, Rect) {
            let texts = self.texts();
            let crumb = texts.into_iter().find(|(t, _)| t.starts_with("     "));
            crumb.unwrap_or_else(|| panic!("no breadcrumb: {:?}", self.texts()))
        }

        /// A click on the header's Back arrow.
        fn back(&mut self) -> Option<SoundFontAction> {
            let screen = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 800.0));
            let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
            let back = crate::ui::modules_browser::layout(screen, 1.0, &m).back;
            self.click(back.center())
        }

        fn texts(&self) -> Vec<(String, Rect)> {
            (self.shapes.iter())
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => Some((
                        t.galley.text().to_owned(),
                        t.galley.rect.translate(t.pos.to_vec2()),
                    )),
                    _ => None,
                })
                .collect()
        }

        fn find(&self, text: &str) -> Rect {
            let texts = self.texts();
            let found = texts.iter().find(|(t, _)| t == text);
            found
                .unwrap_or_else(|| panic!("{text} not drawn: {texts:?}"))
                .1
        }

        fn shows(&self, text: &str) -> bool {
            self.texts().iter().any(|(t, _)| t == text)
        }
    }

    fn picker() -> SoundFontPicker {
        SoundFontPicker::new(Some(DIR.into()), None)
    }

    #[test]
    fn the_root_lists_the_original_soundfonts_after_my_files_and_they_open_under_the_title() {
        let mut picker = picker().with_factory(Some("Content/SoundFonts".into()));
        // The rig's library, with the content folder's SoundFonts in it too.
        let mut rig = Rig::new(SoundFontPicker::new(None, None));
        rig.lib
            .make_folder(Path::new("Content/SoundFonts"))
            .unwrap();
        rig.lib.store("x.flm", &piano()).unwrap();
        rig.lib
            .move_to(
                Path::new("x.flm"),
                Path::new("Content/SoundFonts/BAS Gen.sf2"),
            )
            .unwrap();
        std::mem::swap(&mut rig.picker, &mut picker);
        for _ in 0..30 {
            rig.frame(vec![]);
        }
        assert!(rig.shows(MY_FILES) && rig.shows("BAS Gen"));
        assert_eq!(rig.click_text("BAS Gen"), None);
        // Titled with the SoundFont's name.
        assert!(rig.shows("Upright") && rig.header().0 == "     BAS Gen");
        let pick = rig.click_text("Grand");
        assert!(
            matches!(&pick, Some(SoundFontAction::Patch { path, index: 1, .. }) if path == Path::new("Content/SoundFonts/BAS Gen.sf2")),
            "{pick:?}"
        );
    }

    #[test]
    fn my_files_then_a_soundfont_lists_its_patches_in_file_order_and_a_patch_is_the_pick() {
        let mut rig = Rig::new(picker());
        assert!(rig.shows(TITLE) && rig.shows(MY_FILES) && !rig.shows("Default"));
        assert_eq!(rig.click_text(MY_FILES), None);
        assert!(rig.shows("Keys") && rig.shows("Pad") && rig.shows("Broken"));
        assert!(!rig.shows("notes"));
        assert_eq!(rig.click_text("Keys"), None);
        // Drilled in place: the header does not slide in again. The patches are titled with the SoundFont's name.
        let title = rig.header().1.left();
        assert_eq!(rig.click_text("Piano"), None);
        let (crumb, at) = rig.header();
        assert_eq!(at.left(), title);
        assert_eq!(crumb, "     Piano");
        let ys: Vec<f32> = ["Upright", "Grand", "Bright"]
            .map(|p| rig.find(p).top())
            .to_vec();
        assert!(ys.is_sorted() && ys[0] < ys[2], "file order: {ys:?}");
        // A patch row is in FontSize 4, as every side browser row.
        let size = rig.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "Grand" => {
                Some(t.galley.job.sections[0].format.font_id.size)
            }
            _ => None,
        });
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
        assert_eq!(size, Some(m.font_id(FontSize::Pt18).size));
        let pick = rig.click_text("Grand");
        let want = SoundFontAction::Patch {
            path: "My SoundFonts/Keys/Piano.sf2".into(),
            patches: crate::sf2::presets(&piano()).unwrap(),
            index: 1,
        };
        assert_eq!(pick, Some(want));
        // It stays in the patches.
        assert!(rig.shows("Upright") && rig.shows("Bright"));
    }

    #[test]
    fn a_soundfont_that_does_not_read_is_the_pick_with_its_error() {
        let mut rig = Rig::new(picker());
        rig.click_text(MY_FILES);
        let want = SoundFontAction::Failed {
            path: "My SoundFonts/Broken.sf2".into(),
            error: Sf2Error::NotSoundFont,
        };
        assert_eq!(rig.click_text("Broken"), Some(want));
    }

    #[test]
    fn a_soundfont_cut_off_in_its_samples_is_the_pick_with_a_corrupted_error() {
        // Its patch headers are whole, but the engine cannot play it: no patch list, the error.
        let mut rig = Rig::new(picker());
        rig.click_text(MY_FILES);
        let want = SoundFontAction::Failed {
            path: "My SoundFonts/Cut.sf2".into(),
            error: Sf2Error::Corrupted,
        };
        assert_eq!(rig.click_text("Cut"), Some(want));
    }

    #[test]
    fn back_leaves_the_soundfont_for_the_files() {
        let mut rig = Rig::new(picker());
        rig.click_text(MY_FILES);
        rig.click_text("Pad");
        assert!(rig.shows("Warm"));
        assert_eq!(rig.back(), None);
        assert!(rig.shows("Pad") && rig.shows("Keys") && !rig.shows("Warm"));
    }

    #[test]
    fn opened_on_the_modules_soundfont_it_starts_in_its_patches() {
        let path = PathBuf::from("My SoundFonts/Keys/Piano.sf2");
        let patches = crate::sf2::presets(&piano()).unwrap();
        let mut rig = Rig::new(SoundFontPicker::new(
            Some(DIR.into()),
            Some((path, patches, 1)),
        ));
        assert!(rig.shows("Upright") && rig.shows("Bright"));
        assert_eq!(rig.header().0, "     Piano");
        // Back goes to the files, at their root.
        rig.back();
        assert!(rig.shows(MY_FILES) && !rig.shows("Upright"));
        // Escape closes it, wherever it is.
        rig.frame(vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        assert_eq!(rig.action, Some(SoundFontAction::Close));
    }

    #[test]
    fn search_keeps_the_patches_whose_name_holds_the_query_in_file_order_or_a_to_z() {
        let patches = crate::sf2::presets(&piano()).unwrap();
        let matching = |query| matching(&patches, query, PatchOrdering::Default);
        assert_eq!(matching(""), [0, 1, 2]);
        assert_eq!(matching("R"), [0, 1, 2]);
        assert_eq!(matching("ght"), [0, 2]);
        assert_eq!(matching("GRAND"), [1]);
        assert!(matching("x").is_empty());
        // `A-Z`: Bright, Grand, Upright.
        assert_eq!(super::matching(&patches, "", PatchOrdering::AZ), [2, 1, 0]);
        assert_eq!(super::matching(&patches, "ght", PatchOrdering::AZ), [2, 0]);
    }
}

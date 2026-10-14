//! The SoundFont picker, which the SoundFont Player's `Patch` spinner opens, as the side browser panel.
//!
//! - Its root and folders are a [`PresetPicker`]'s over `My SoundFonts`, titled [`TITLE`]: `My Files`, then the
//!   folders and the `.sf2` files by name. No `Default`. No factory SoundFonts are shipped; with imported content in
//!   the content folder the root then lists its `SoundFonts`, read-only ([`SoundFontPicker::with_factory`]).
//! - A SoundFont opens like a folder, its name ending the header's breadcrumb, and lists its patches in file order,
//!   which `Search` filters; Back returns to the files.
//! - A patch is the pick. A SoundFont that cannot be read is the pick too, with its error: the caller loads it, and
//!   the panel shows the error.
//!
//! The caller keeps a [`SoundFontPicker`] while it is open and loads what [`SoundFontPicker::show`] returns.

use std::path::{Path, PathBuf};

use eframe::egui::{Context, Id};

use crate::project_files::SF2_EXT;
use crate::sf2::{Preset, Sf2Error};
use crate::ui::UiCtx;
use crate::ui::main_menu::SongLibrary;
use crate::ui::modules_browser::{
    Chrome, PanelClick, PanelRow, ROW_H, RowKind, SideButton, SidePanel, crumb_text,
};
use crate::ui::preset_picker::{MY_FILES, PickerAction, PresetPicker};

/// The root's title: the factory folder's name.
pub const TITLE: &str = "SoundFonts";

/// What the user did in the picker this frame. Either way the picker is done: drop it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoundFontAction {
    /// The close X, a click outside the panel, or Escape.
    Close,
    /// Patch `index` of `patches`, the patches of the SoundFont at `path` (as its library lists it).
    Patch {
        path: PathBuf,
        patches: Vec<Preset>,
        index: usize,
    },
    /// The SoundFont at `path` could not be read.
    Failed { path: PathBuf, error: Sf2Error },
}

/// An open SoundFont: its path, its patches, and the panel listing them.
struct Open {
    path: PathBuf,
    patches: Vec<Preset>,
    panel: SidePanel,
}

/// The open picker's state. Make a fresh one each time it opens.
pub struct SoundFontPicker {
    my_files: Option<PathBuf>,
    /// The factory folder the root lists, if any.
    factory: Option<PathBuf>,
    files: PresetPicker,
    open: Option<Open>,
}

impl SoundFontPicker {
    /// A picker over `my_files` (`My SoundFonts`; `None` dims `My Files`), at its root, or in the patches of
    /// `current`: the module's SoundFont, its path as the library lists it, and its patches.
    pub fn new(my_files: Option<PathBuf>, current: Option<(PathBuf, Vec<Preset>)>) -> Self {
        Self {
            files: PresetPicker::new(TITLE, my_files.clone(), SF2_EXT, false),
            my_files,
            factory: None,
            open: current.map(|(path, patches)| Open {
                path,
                patches,
                panel: SidePanel::default(),
            }),
        }
    }

    /// Lists the SoundFonts in `folder` (the content folder's `SoundFonts`) at the root, after `My Files`.
    pub fn with_factory(mut self, folder: Option<PathBuf>) -> Self {
        self.files = self.files.with_factory(folder.clone());
        self.factory = folder;
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
                // `Default` is not listed, nor `Import` offered.
                PickerAction::Close | PickerAction::Default | PickerAction::Import => {
                    return Some(SoundFontAction::Close);
                }
            };
            let read = library.read(&path).map_err(|_| Sf2Error::NotFound);
            match read.and_then(|bytes| crate::sf2::presets(&bytes)) {
                Ok(patches) => {
                    let panel = SidePanel::in_place();
                    self.open = Some(Open {
                        path,
                        patches,
                        panel,
                    });
                    return None;
                }
                Err(error) => return Some(SoundFontAction::Failed { path, error }),
            }
        };
        // The folders from `My Files` (or the factory folder, which the root lists itself) down to the SoundFont's.
        let below =
            |root: Option<&Path>| Some(open.path.parent()?.strip_prefix(root?).ok()?.to_owned());
        let (top, folders) = match below(self.factory.as_deref()) {
            Some(folders) => (None, Some(folders)),
            None => (Some(MY_FILES), below(self.my_files.as_deref())),
        };
        let folders = folders
            .iter()
            .flat_map(|f| f.iter())
            .map(|f| f.to_string_lossy().into_owned());
        let title = open.path.file_stem().unwrap_or_default().to_string_lossy();
        let crumb: Vec<String> = std::iter::once(TITLE.to_owned())
            .chain(top.map(String::from))
            .chain(folders)
            .chain([title.into_owned()])
            .collect();
        let header = crumb_text(crumb.iter().map(String::as_str));
        let chrome = Chrome {
            // The files' own id: one panel area for both, as egui hides a new area for its first frame.
            id: Id::new("preset_picker"),
            title: &header,
            back: true,
            buttons: &[SideButton::Search],
            popup: false,
        };
        let patches = &open.patches;
        let click = open.panel.show(ctx, cx, &chrome, |query| {
            let rows = matching(patches, query).into_iter();
            rows.map(|i| PanelRow {
                name: &patches[i].name,
                folder: false,
                h: ROW_H,
                sub: None,
                kind: RowKind::Enabled(i),
                picked: false,
            })
            .collect()
        });
        match click? {
            PanelClick::Close => Some(SoundFontAction::Close),
            // Back; it has no other button and asks for no name.
            PanelClick::Back | PanelClick::Button(_) | PanelClick::Enter => {
                self.open = None;
                None
            }
            PanelClick::Row(index) => {
                let open = self.open.take()?;
                Some(SoundFontAction::Patch {
                    path: open.path,
                    patches: open.patches,
                    index,
                })
            }
        }
    }
}

/// The indices of the patches whose name holds `query`, ignoring case, in file order.
fn matching(patches: &[Preset], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    let holds = |(_, p): &(usize, &Preset)| p.name.to_lowercase().contains(&query);
    patches
        .iter()
        .enumerate()
        .filter(holds)
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use eframe::egui::epaint::ClippedShape;
    use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2};

    use super::*;
    use crate::metrics::{DEFAULT_UI_SCALE_INDEX, FontSize, Metrics};
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
                Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX),
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
            let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
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
        // The folder the root lists is no `My Files`.
        assert!(rig.shows("Upright") && rig.header().0 == "     SoundFonts | BAS Gen");
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
        // Drilled in place: the header does not slide in again.
        let title = rig.header().1.left();
        assert_eq!(rig.click_text("Piano"), None);
        let (crumb, at) = rig.header();
        assert_eq!(at.left(), title);
        // The trail keeps its tail when it is too long for the header.
        assert!(crumb.ends_with("Keys | Piano"), "{crumb}");
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
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
        assert_eq!(size, Some(m.font_id(FontSize::Pt18).size));
        let pick = rig.click_text("Grand");
        let want = SoundFontAction::Patch {
            path: "My SoundFonts/Keys/Piano.sf2".into(),
            patches: crate::sf2::presets(&piano()).unwrap(),
            index: 1,
        };
        assert_eq!(pick, Some(want));
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
            Some((path, patches)),
        ));
        assert!(rig.shows("Upright") && rig.shows("Bright"));
        assert!(rig.header().0.ends_with("Keys | Piano"));
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
    fn search_keeps_the_patches_whose_name_holds_the_query_in_file_order() {
        let patches = crate::sf2::presets(&piano()).unwrap();
        assert_eq!(matching(&patches, ""), [0, 1, 2]);
        assert_eq!(matching(&patches, "R"), [0, 1, 2]);
        assert_eq!(matching(&patches, "ght"), [0, 2]);
        assert_eq!(matching(&patches, "GRAND"), [1]);
        assert!(matching(&patches, "x").is_empty());
    }
}

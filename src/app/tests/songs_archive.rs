//! SONGS archives through the whole app: listed, opened by a tap, and unzipped from the ≡ menu.

use super::*;
use crate::project_files::{ARCHIVE_FAILED, UNZIPPED_FOLDER};
use crate::ui::main_menu::MemoryLibrary;
#[cfg(not(target_arch = "wasm32"))]
use crate::ui::main_menu::NativeLibrary;

/// The ≡ menu row of `Unzip` with one row selected, outside the bin: `Select all`, `Create folder`, `Delete`, `Unzip`.
const UNZIP_ROW: usize = 3;

/// A rig with SONGS open on a library that holds `Pack.zip`, an archive of the song `Beat` and a sample.
fn rig_with_archive(archive: Option<Vec<u8>>) -> Rig {
    let (mut rig, _) = rig_with_file("/songs/X.flm", b"");
    let archive = archive.unwrap_or_else(|| {
        crate::zip::write(&[
            ("Beat.flm".to_owned(), synthetic_file(1, FORMAT_VERSION, "")),
            ("My Samples/kick.wav".to_owned(), b"kick".to_vec()),
        ])
        .unwrap()
    });
    let mut library = MemoryLibrary::default();
    library.put(PathBuf::from("Pack.zip"), &archive).unwrap();
    rig.app.files.library = Box::new(library);
    rig.open_menu(MenuTab::Songs);
    rig
}

/// Select mode, the archive's row `archive` selected, and `Unzip` picked.
fn unzip(rig: &mut Rig, archive: egui::Id) {
    rig.click_id(songs_id(("side", 3_usize)));
    rig.click_id(archive);
    songs_option(rig, UNZIP_ROW);
}

#[test]
fn a_tap_on_an_archive_opens_its_song_as_a_picked_archive_does() {
    let mut rig = rig_with_archive(None);
    assert!(rig.drawn(row("Pack.zip")));
    rig.click_id(row("Pack.zip"));
    // The song of the archive, titled after the zip, which is its path.
    assert_eq!(rig.app.song.title, "Pack");
    assert!(!rig.app.files.modified());
    let beat = synthetic_file(1, FORMAT_VERSION, "");
    let extracted = Path::new(UNZIPPED_FOLDER).join("Pack/Beat.flm");
    assert_eq!(rig.app.files.read_user_file(&extracted).unwrap(), beat);
}

#[test]
fn unzip_extracts_the_archive_and_flashes_where_without_opening_it() {
    let mut rig = rig_with_archive(None);
    let (song, changes) = (rig.app.song.clone(), rig.app.files.library_changes());
    unzip(&mut rig, row("Pack.zip"));
    // The flash says where; no message, the menu stays, the song is the one it was.
    assert!(
        rig.texts()
            .contains(&format!("Unzipped to \"{UNZIPPED_FOLDER}/Pack\"")),
        "{:?}",
        rig.texts()
    );
    assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Songs)]);
    assert_eq!(rig.app.song, song);
    let dir = Path::new(UNZIPPED_FOLDER).join("Pack");
    let kick = rig
        .app
        .files
        .read_user_file(&dir.join("My Samples/kick.wav"));
    assert_eq!(kick.unwrap(), b"kick");
    // SONGS reads its folder again: the library changed.
    assert!(rig.app.files.library_changes() > changes);
    assert!(rig.drawn(row("Pack.zip")));
}

#[test]
fn unzip_of_a_file_that_is_no_archive_shows_the_message() {
    let mut rig = rig_with_archive(Some(b"not an archive".to_vec()));
    unzip(&mut rig, row("Pack.zip"));
    rig.frame(vec![]);
    let overlays = rig.overlays();
    let [Overlay::MainMenu(_), Overlay::Message { lines, .. }] = &overlays[..] else {
        panic!("{overlays:?}");
    };
    assert_eq!(lines[0], ARCHIVE_FAILED);
    assert!(!rig.texts().iter().any(|t| t.starts_with("Unzipped")));
}

/// A folder in the temp folder, removed when dropped (and before, if a run that died left it).
#[cfg(not(target_arch = "wasm32"))]
struct Home(PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl Home {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("honeydew-su-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The SONGS row of the library's entry at `path` (the desktop's rows are by full path).
#[cfg(not(target_arch = "wasm32"))]
fn row_at(path: &Path) -> egui::Id {
    songs_id(("row", path.to_owned()))
}

/// `Unzipped songs` is a root of SONGS: once an archive is unzipped, its folder, and the songs
/// and folders in it, can be gone into and are listed.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn after_unzip_the_extracted_folder_and_its_files_are_browsable_from_songs() {
    let home = Home::new("browse");
    let (user, songs) = (home.0.join("user"), home.0.join("user/My Songs"));
    std::fs::create_dir_all(&songs).unwrap();
    let archive = crate::zip::write(&[
        ("Beat.flm".to_owned(), synthetic_file(1, FORMAT_VERSION, "")),
        ("My Samples/kick.wav".to_owned(), b"kick".to_vec()),
        (
            "Extra/Loop.flm".to_owned(),
            synthetic_file(2, FORMAT_VERSION, ""),
        ),
    ])
    .unwrap();
    std::fs::write(songs.join("Pack.zip"), archive).unwrap();
    let mut rig = Rig::new();
    rig.app.files = without_factory(ProjectFiles::on_disk(&user));
    rig.app.files.library = Box::new(NativeLibrary::at(songs.clone()));
    rig.open_menu(MenuTab::Songs);
    // Before: the archive, and no `Unzipped songs` yet.
    let unzipped = user.join(UNZIPPED_FOLDER);
    assert!(rig.drawn(row_at(&songs.join("Pack.zip"))));
    assert!(!rig.drawn(row_at(&unzipped)));
    unzip(&mut rig, row_at(&songs.join("Pack.zip")));
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Songs)]);
    // SONGS read its folder again: the root has the new place. In select mode it is a place, so a tap goes in.
    assert!(rig.drawn(row_at(&unzipped)));
    rig.click_id(row_at(&unzipped));
    rig.frame(vec![]);
    assert!(
        rig.texts().contains(&UNZIPPED_FOLDER.to_owned()),
        "its title"
    );
    // Out of select mode: the extraction's folder, then what is in it (songs and folders; the samples are no songs).
    rig.click_id(songs_id(("side", 3_usize)));
    let pack = unzipped.join("Pack");
    assert!(rig.drawn(row_at(&pack)));
    rig.click_id(row_at(&pack));
    rig.frame(vec![]);
    assert!(rig.drawn(row_at(&pack.join("Beat.flm"))));
    assert!(rig.drawn(row_at(&pack.join("Extra"))));
    assert!(rig.drawn(row_at(&pack.join("My Samples"))));
    assert!(!rig.drawn(row_at(&pack.join("My Samples/kick.wav"))));
    rig.click_id(row_at(&pack.join("Extra")));
    rig.frame(vec![]);
    assert!(rig.drawn(row_at(&pack.join("Extra/Loop.flm"))));
    // The breadcrumb names the folders above the open one, from the root.
    assert!(
        rig.texts()
            .contains(&format!("My Songs / {UNZIPPED_FOLDER} / Pack"))
    );
}

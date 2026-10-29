//! Our own factory content: drum kits, presets, Slicer loops, 3x Osc shapes, chord sets, song templates and the demo
//! song. Every item is made in code here and is ours; nothing is read from disk and no binary file is in the repo.
//!
//! - **The library.** [`Factory`] is a read-only [`SongLibrary`] under the virtual root [`ROOT`], `<built-in>`: `<` and
//!   `>` are in the file browser's forbidden characters, so no user file or folder can be named that. Its folders come
//!   from its entries' paths, its files list in index order (the bytes of their paths), and a read runs the entry's
//!   generator. [`factory`] holds every kind's entries, one sub-module each ([`PRESETS`], [`DRUMSETS`], [`LOOPS`],
//!   [`SHAPES`], [`CHORD_SETS`], [`TEMPLATES`]).
//! - **The union.** [`Union`] is a user library with the factory beside it: a path under [`ROOT`] goes to the factory,
//!   any other to the user's. The preset pickers list and read through it
//!   ([`crate::project_files::ProjectFiles::preset_library`]); the factory refuses every write.
//! - **Content paths** (shared with the Original Content folder, [`crate::project_files::CONTENT_FOLDER`]): a factory
//!   file `<built-in>/<rel>` is stored as the shorthand `@R@Built-in/<rel>` ([`content_path`]). The sample cache plays
//!   such a path from code, before any disk or Original Content lookup ([`crate::sample_cache::SampleKey::of_path`]).

use std::io;
use std::ops::DerefMut;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::project_files::BUILT_IN;
use crate::ui::main_menu::{SongEntry, SongLibrary};

pub mod bundle;
mod chord_sets;
mod demo;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub mod install;
pub mod kits;
mod presets;
mod samples;
mod templates;
pub mod web;

pub use bundle::Bundle;
pub use demo::{DEMO_FOLDER, demo_songs, restore_demo_songs};
/// Our code-made preset `name` of module type `type_id`, as a pick of it loads it.
pub use presets::find as preset;

/// The factory's virtual root.
pub const ROOT: &str = "<built-in>";
/// The factory's top folders.
pub const PRESETS: &str = "Presets";
pub const DRUMSETS: &str = "Drumsets";
pub const LOOPS: &str = "Loops";
pub const SHAPES: &str = "Shapes";
/// Bundled one-shots, by category ([`bundle`]).
pub const SAMPLES: &str = "Samples";
pub const CHORD_SETS: &str = "Chord Sets";
pub const TEMPLATES: &str = "Templates";
/// Our bundled grand: what a new DW Sampler loads where the content has no `Close Grand`, and what one whose files are
/// all missing plays ([`crate::audio_engine::samples::stand_in`]).
pub const GRAND_PIANO: &str = "Presets/DW Sampler/Piano/Grand Piano.dwp";

/// One factory file: its path below [`ROOT`], `/`-separated (`Drumsets/Dust Kit.tmpl`), and what makes its bytes, run
/// on each read: `None` (not found) for a bundled member that cannot be read.
pub struct Entry {
    pub rel: String,
    pub bytes: Box<dyn Fn() -> Option<Vec<u8>> + Send + Sync>,
}

impl Entry {
    pub fn new(
        rel: impl Into<String>,
        bytes: impl Fn() -> Vec<u8> + Send + Sync + 'static,
    ) -> Self {
        Self {
            rel: rel.into(),
            bytes: Box::new(move || Some(bytes())),
        }
    }
}

/// The factory: its entries, in index order, and the bundled content they include ([`bundle`]).
pub struct Factory {
    entries: Vec<Entry>,
    pub bundle: Option<Arc<Bundle>>,
}

impl Factory {
    /// A factory of `entries` (a test's fake one; the app's is [`factory`]), kept in index order.
    pub fn new(mut entries: Vec<Entry>) -> Self {
        entries.sort_by(|a, b| a.rel.cmp(&b.rel));
        Self {
            entries,
            bundle: None,
        }
    }

    /// This factory with `bundle`'s listed members as entries ([`Bundle::listed`]), read from it. The sample reader
    /// reads the others (zone files, lite programs) through [`Self::bundle`].
    pub fn with_bundle(mut self, bundle: Bundle) -> Self {
        let bundle = Arc::new(bundle);
        for rel in bundle.listed() {
            let (from, at) = (bundle.clone(), rel.clone());
            let bytes = Box::new(move || from.read(&at));
            self.entries.push(Entry { rel, bytes });
        }
        self.entries.sort_by(|a, b| a.rel.cmp(&b.rel));
        self.bundle = Some(bundle);
        self
    }
}

/// Every kind's entries, made once.
pub fn factory() -> &'static Factory {
    static FACTORY: OnceLock<Factory> = OnceLock::new();
    FACTORY.get_or_init(|| {
        let entries = (presets::entries().into_iter())
            .chain(kits::entries())
            .chain(samples::entries())
            .chain(chord_sets::entries())
            .chain(templates::entries());
        let factory = Factory::new(entries.collect());
        match Bundle::find() {
            Some(bundle) => factory.with_bundle(bundle),
            None => factory,
        }
    })
}

/// `path` is in the factory: below [`ROOT`].
pub fn is_factory(path: &Path) -> bool {
    path.starts_with(ROOT)
}

/// The shorthand a factory file `<built-in>/<rel>` is stored as: `@R@Built-in/<rel>`. `None` for any other path.
pub fn content_path(path: &Path) -> Option<ContentPath> {
    let rel = below_root(path).filter(|rel| !rel.is_empty())?;
    Some(ContentPath::Shorthand {
        root: ContentRoot::App,
        relative: format!("{BUILT_IN}/{rel}"),
    })
}

/// `path`'s place below [`ROOT`], `/`-separated: `""` for the root itself. `None` for a path elsewhere, or with
/// anything but plain names below the root.
fn below_root(path: &Path) -> Option<String> {
    let parts: Option<Vec<&str>> = (path.strip_prefix(ROOT).ok()?.components())
        .map(|c| match c {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    Some(parts?.join("/"))
}

impl SongLibrary for Factory {
    /// The folders and files right in `folder` (the root for `None`), in index order. A folder with nothing in it is
    /// not there.
    fn list(&self, folder: Option<&Path>) -> io::Result<Vec<SongEntry>> {
        let folder = folder.unwrap_or(Path::new(ROOT));
        let dir = below_root(folder).ok_or(io::ErrorKind::NotFound)?;
        let mut rows: Vec<SongEntry> = Vec::new();
        for entry in &self.entries {
            let rest = match dir.as_str() {
                "" => Some(entry.rel.as_str()),
                dir => (entry.rel.strip_prefix(dir)).and_then(|r| r.strip_prefix('/')),
            };
            let Some(rest) = rest else { continue };
            let (name, is_folder) = rest
                .split_once('/')
                .map_or((rest, false), |(f, _)| (f, true));
            let path = folder.join(name);
            if !rows
                .iter()
                .any(|r| r.is_folder == is_folder && r.path == path)
            {
                let display = self
                    .bundle
                    .as_ref()
                    .filter(|_| !is_folder)
                    .and_then(|bundle| bundle.display_name(&entry.rel));
                rows.push(SongEntry {
                    name: display.unwrap_or_else(|| name.into()),
                    path,
                    is_folder,
                    recency: 0,
                });
            }
        }
        if rows.is_empty() && !dir.is_empty() {
            return Err(io::ErrorKind::NotFound.into());
        }
        Ok(rows)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let rel = below_root(path).ok_or(io::ErrorKind::NotFound)?;
        let entry = self.entries.iter().find(|e| e.rel == rel);
        let entry = entry.ok_or(io::ErrorKind::NotFound)?;
        // On the web a bundled file that does not read failed to download.
        let lost = || match web::lost(self, path) {
            true => io::Error::new(io::ErrorKind::NotFound, web::DOWNLOAD_FAILED),
            false => io::ErrorKind::NotFound.into(),
        };
        (entry.bytes)().ok_or_else(lost)
    }

    fn store(&mut self, _: &str, _: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn root(&self) -> Option<&Path> {
        Some(Path::new(ROOT))
    }

    fn exists(&self, path: &Path) -> bool {
        below_root(path).is_some_and(|rel| {
            let within = |e: &Entry| e.rel.strip_prefix(&rel).is_some_and(|r| r.starts_with('/'));
            rel.is_empty() || self.entries.iter().any(|e| e.rel == rel || within(e))
        })
    }

    fn read_only(&self, _: &Path) -> bool {
        true
    }
}

/// A user library `user` with the factory beside it: a path under [`ROOT`] lists and reads from `factory`, which is
/// read-only; any other path is the user's.
pub struct Union<'a, U> {
    pub user: U,
    pub factory: &'a Factory,
}

impl<U: DerefMut<Target: SongLibrary + Sized>> Union<'_, U> {
    /// The library `path` is in.
    fn of(&self, path: &Path) -> &dyn SongLibrary {
        match is_factory(path) {
            true => self.factory,
            false => &*self.user,
        }
    }

    /// The user's library, to change: refused when any of `paths` is in the factory.
    fn user_for(&mut self, paths: &[&Path]) -> io::Result<&mut U::Target> {
        match paths.iter().any(|p| is_factory(p)) {
            true => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "built-in content is read-only",
            )),
            false => Ok(&mut *self.user),
        }
    }
}

impl<U: DerefMut<Target: SongLibrary + Sized>> SongLibrary for Union<'_, U> {
    fn list(&self, folder: Option<&Path>) -> io::Result<Vec<SongEntry>> {
        match folder {
            Some(path) => self.of(path).list(folder),
            None => self.user.list(None),
        }
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.of(path).read(path)
    }

    fn store(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        self.user.store(name, bytes)
    }

    fn store_in(&mut self, folder: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
        self.user_for(&[folder])?.store_in(folder, name, bytes)
    }

    fn root(&self) -> Option<&Path> {
        self.user.root()
    }

    fn trash(&self) -> Option<PathBuf> {
        self.user.trash()
    }

    fn places(&self) -> Vec<PathBuf> {
        self.user.places()
    }

    fn bin_path(&self, path: &Path) -> Option<PathBuf> {
        self.of(path).bin_path(path)
    }

    fn restore_path(&self, path: &Path) -> Option<PathBuf> {
        self.of(path).restore_path(path)
    }

    fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
        self.of(path).canonical(path)
    }

    fn is_link(&self, path: &Path) -> bool {
        self.of(path).is_link(path)
    }

    fn shadowed(&self, path: &Path) -> bool {
        self.of(path).shadowed(path)
    }

    fn list_all(&self, folder: &Path) -> io::Result<Vec<SongEntry>> {
        self.of(folder).list_all(folder)
    }

    fn list_raw(&self, folder: &Path) -> io::Result<Vec<SongEntry>> {
        self.of(folder).list_raw(folder)
    }

    fn move_link(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        self.user_for(&[from, to])?.move_link(from, to)
    }

    fn is_empty_folder(&self, path: &Path) -> bool {
        self.of(path).is_empty_folder(path)
    }

    fn exists(&self, path: &Path) -> bool {
        self.of(path).exists(path)
    }

    fn make_folder(&mut self, path: &Path) -> io::Result<()> {
        self.user_for(&[path])?.make_folder(path)
    }

    fn move_to(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        self.user_for(&[from, to])?.move_to(from, to)
    }

    fn copy_to(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        self.user_for(&[from, to])?.copy_to(from, to)
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        self.user_for(&[path])?.remove(path)
    }

    fn write_new(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.user_for(&[path])?.write_new(path, bytes)
    }

    fn read_only(&self, path: &Path) -> bool {
        self.of(path).read_only(path)
    }

    fn templates(&self) -> Vec<SongEntry> {
        self.user.templates()
    }
}

#[cfg(test)]
mod tests;

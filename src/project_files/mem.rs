//! An in-memory [`FileProvider`] for tests: a map of files, and scripted answers for the dialogs.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::{FileEvent, FileProvider, OPEN_FAILED, SAVE_FAILED, UNKNOWN_TYPE, accepts};

/// Clones share the state, so a test keeps a handle while the app owns the provider.
#[derive(Clone, Default)]
pub(crate) struct MemFiles(pub Rc<RefCell<MemState>>);

#[derive(Default)]
pub(crate) struct MemState {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    /// The file the open dialog picks; `None` cancels it.
    pub pick: Option<PathBuf>,
    /// The folder the save dialog saves into (it keeps the suggested file name); `None` cancels it.
    pub save_dir: Option<PathBuf>,
    /// Writing fails with this message.
    pub write_error: Option<String>,
    /// Every save request, as (path, ask).
    pub saves: Vec<(PathBuf, bool)>,
    /// Every open request's accepted extensions.
    pub accepts: Vec<&'static [&'static str]>,
    /// Keeps and reports a save under its file name only, as the web's download does.
    pub names_only: bool,
    events: VecDeque<FileEvent>,
}

impl MemFiles {
    pub fn with_file(path: &str, bytes: &[u8]) -> Self {
        let mem = Self::default();
        mem.0
            .borrow_mut()
            .files
            .insert(PathBuf::from(path), bytes.to_vec());
        mem.0.borrow_mut().pick = Some(PathBuf::from(path));
        mem
    }
}

impl FileProvider for MemFiles {
    fn open(&mut self, accept: &'static [&'static str]) {
        let mut s = self.0.borrow_mut();
        s.accepts.push(accept);
        let Some(path) = s.pick.clone() else { return };
        let event = match s.files.get(&path) {
            _ if !accepts(&path, accept) => {
                FileEvent::Failed(UNKNOWN_TYPE, path.display().to_string())
            }
            Some(bytes) => FileEvent::Opened(path, bytes.clone()),
            None => FileEvent::Failed(OPEN_FAILED, format!("{}: not found", path.display())),
        };
        s.events.push_back(event);
    }

    fn save(&mut self, path: &Path, ask: bool, bytes: Vec<u8>) {
        let mut s = self.0.borrow_mut();
        s.saves.push((path.to_owned(), ask));
        let target = if ask {
            let Some(dir) = &s.save_dir else { return };
            dir.join(path.file_name().unwrap_or_default())
        } else {
            path.to_owned()
        };
        let target = match s.names_only {
            true => PathBuf::from(target.file_name().unwrap_or_default()),
            false => target,
        };
        let event = match s.write_error.clone() {
            Some(err) => FileEvent::Failed(SAVE_FAILED, err),
            None => {
                s.files.insert(target.clone(), bytes);
                FileEvent::Saved(target)
            }
        };
        s.events.push_back(event);
    }

    fn poll(&mut self) -> Option<FileEvent> {
        self.0.borrow_mut().events.pop_front()
    }

    /// A file, or a folder holding one.
    fn exists(&self, path: &Path) -> bool {
        self.0
            .borrow()
            .files
            .keys()
            .any(|file| file.starts_with(path))
    }

    fn folders(&self) -> bool {
        !self.0.borrow().names_only
    }
}

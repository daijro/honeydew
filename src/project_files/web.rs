//! The web: a file input to open (`<input type="file">`), a download to save. The
//! "path" of a file is its name, so `Ctrl+S` downloads it again under the same name.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use eframe::egui::Context;
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::js_sys::{Array, Reflect, Uint8Array};
use web_sys::{Blob, HtmlAnchorElement, HtmlInputElement, Url};

use super::{
    FileEvent, FileProvider, MAX_FILE_BYTES, OPEN_FAILED, READ_FAILED, SAVE_FAILED, UNKNOWN_TYPE,
    accept_attr, accepts, too_large,
};

#[derive(Default)]
pub struct WebFiles {
    /// Repaints when a file has been read; attached by the first frame, shared with the read callbacks.
    ctx: Rc<RefCell<Option<Context>>>,
    /// Shared with the read callbacks.
    events: Rc<RefCell<VecDeque<FileEvent>>>,
}

thread_local! {
    /// The last download's blob URL, released at the next one (the browser may still be reading it right after the
    /// click).
    static LAST_URL: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Downloads `bytes` as the file `name` (a save, and SONGS `Export`).
pub(crate) fn download(name: &str, bytes: &[u8]) -> Result<(), String> {
    let blob = Blob::new_with_u8_array_sequence(&Array::of1(&Uint8Array::from(bytes)));
    let url = blob.and_then(|blob| Url::create_object_url_with_blob(&blob));
    let url = url.map_err(|err| text(&err))?;
    let link: HtmlAnchorElement = document()
        .and_then(|d| d.create_element("a"))
        .map_err(|err| text(&err))?
        .unchecked_into();
    link.set_href(&url);
    link.set_download(name);
    link.click();
    if let Some(old) = LAST_URL.with_borrow_mut(|last| last.replace(url)) {
        Url::revoke_object_url(&old).map_err(|err| text(&err))?;
    }
    Ok(())
}

impl WebFiles {
    fn pick(&self, accept: &'static [&'static str]) -> Result<(), JsValue> {
        let input: HtmlInputElement = document()?.create_element("input")?.unchecked_into();
        input.set_type("file");
        input.set_accept(&accept_attr(accept));
        let (events, ctx, picked) = (self.events.clone(), self.ctx.clone(), input.clone());
        // Freed after its one call; a cancelled pick leaks this one small closure.
        let on_change = wasm_bindgen::closure::Closure::once_into_js(move || {
            let Some(file) = picked.files().and_then(|files| files.get(0)) else {
                return;
            };
            wasm_bindgen_futures::spawn_local(async move {
                // The type and the size are checked before anything is read.
                let event = if !accepts(Path::new(&file.name()), accept) {
                    FileEvent::Failed(UNKNOWN_TYPE, file.name())
                } else if file.size() > MAX_FILE_BYTES as f64 {
                    FileEvent::Failed(READ_FAILED, format!("{}: {}", file.name(), too_large()))
                } else {
                    match JsFuture::from(file.array_buffer()).await {
                        Ok(buffer) => {
                            FileEvent::Opened(file.name().into(), Uint8Array::new(&buffer).to_vec())
                        }
                        Err(err) => FileEvent::Failed(
                            READ_FAILED,
                            format!("{}: {}", file.name(), text(&err)),
                        ),
                    }
                };
                events.borrow_mut().push_back(event);
                if let Some(ctx) = ctx.borrow().as_ref() {
                    ctx.request_repaint();
                }
            });
        });
        input.set_onchange(Some(on_change.unchecked_ref()));
        input.click();
        Ok(())
    }
}

impl FileProvider for WebFiles {
    fn open(&mut self, accept: &'static [&'static str]) {
        if let Err(err) = self.pick(accept) {
            let event = FileEvent::Failed(OPEN_FAILED, text(&err));
            self.events.borrow_mut().push_back(event);
        }
    }

    fn save(&mut self, path: &Path, _ask: bool, bytes: Vec<u8>) {
        let name = path
            .file_name()
            .map_or_else(|| path.to_string_lossy(), |name| name.to_string_lossy());
        let event = match download(&name, &bytes) {
            Ok(()) => FileEvent::Saved(PathBuf::from(name.as_ref())),
            Err(err) => FileEvent::Failed(SAVE_FAILED, format!("{name}: {err}")),
        };
        self.events.borrow_mut().push_back(event);
    }

    fn poll(&mut self) -> Option<FileEvent> {
        self.events.borrow_mut().pop_front()
    }

    fn attach(&mut self, ctx: &Context) {
        self.ctx.borrow_mut().get_or_insert_with(|| ctx.clone());
    }

    /// The browser's downloads folder cannot be listed.
    fn exists(&self, _path: &Path) -> bool {
        false
    }

    /// A download keeps only its file name.
    fn folders(&self) -> bool {
        false
    }
}

fn document() -> Result<web_sys::Document, JsValue> {
    web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("no document"))
}

/// A JS error as text: its `message` (an `Error` or a `DOMException`), else the value itself.
fn text(value: &JsValue) -> String {
    Reflect::get(value, &JsValue::from_str("message"))
        .ok()
        .and_then(|message| message.as_string())
        .or_else(|| value.as_string())
        .unwrap_or_else(|| format!("{value:?}"))
}

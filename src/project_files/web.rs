//! The web: a file input to open (`<input type="file">`), a download to save (inside Playground, its Save As, which
//! keeps the file in Playground's Files or downloads it). The "path" of a file is its name, so `Ctrl+S` saves it again
//! under the same name. Playground hands a file opened from its Files to the page as a drop, which opens it.

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

/// `playgroundFiles.<method>(arg)`, awaited.
async fn call(files: &JsValue, method: &str, arg: &JsValue) -> Result<JsValue, JsValue> {
    let f: web_sys::js_sys::Function =
        Reflect::get(files, &JsValue::from_str(method))?.dyn_into()?;
    JsFuture::from(web_sys::js_sys::Promise::resolve(&f.call1(files, arg)?)).await
}

/// [`WebFiles::locate_in_playground`]'s search: the files named one of `names` (any case) in the folder of the sample
/// the user picks; none when the pick is cancelled.
async fn playground_locate(
    files: &JsValue,
    names: &[String],
) -> Result<Vec<(String, Vec<u8>)>, JsValue> {
    let wanted = |name: &str| names.iter().any(|n| n.eq_ignore_ascii_case(name));
    let base = |path: &str| path.rsplit('/').next().unwrap_or(path).to_owned();
    let request = web_sys::js_sys::Object::new();
    let types: Array = ["wav", "flac", "ogg", "mp3", "aif", "aiff"]
        .iter()
        .map(|t| JsValue::from_str(t))
        .collect();
    Reflect::set(&request, &JsValue::from_str("types"), &types)?;
    let picked = call(files, "open", &request).await?;
    let Some(first) = picked
        .dyn_ref::<Array>()
        .map(|a| a.get(0))
        .filter(|f| f.is_object())
    else {
        return Ok(Vec::new());
    };
    let path = Reflect::get(&first, &JsValue::from_str("path"))?.as_string();
    let Some(path) = path else {
        // From the computer: that one file.
        let name = Reflect::get(&first, &JsValue::from_str("name"))?
            .as_string()
            .unwrap_or_default();
        let bytes: Uint8Array = Reflect::get(&first, &JsValue::from_str("bytes"))?.dyn_into()?;
        return Ok(wanted(&name)
            .then(|| (name, bytes.to_vec()))
            .into_iter()
            .collect());
    };
    let folder = path
        .rsplit_once('/')
        .map_or(path.as_str(), |(dir, _)| dir)
        .to_owned();
    let listed = call(files, "files", &JsValue::from_str(&folder)).await?;
    let mut found: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in listed
        .dyn_ref::<Array>()
        .map(Array::to_vec)
        .unwrap_or_default()
    {
        let Some(at) = entry.as_string() else {
            continue;
        };
        let name = base(&at);
        if !wanted(&name) || found.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let bytes: Uint8Array = call(files, "read", &entry).await?.dyn_into()?;
        found.push((name, bytes.to_vec()));
    }
    Ok(found)
}

/// Playground's file store (`globalThis.playgroundFiles`), when the page runs inside Playground.
fn playground_files() -> Option<JsValue> {
    let files = Reflect::get(
        &web_sys::js_sys::global(),
        &JsValue::from_str("playgroundFiles"),
    )
    .ok()?;
    (files.is_object()).then_some(files)
}

impl WebFiles {
    /// A save inside Playground: its Save As (`playgroundFiles.saveAs({ name, types, bytes })`), which keeps the file
    /// in Playground's Files or downloads it. Saved once it answers; a cancelled one finishes with no event, as a
    /// cancelled desktop dialog does.
    fn save_in_playground(&self, files: &JsValue, name: &str, bytes: &[u8]) {
        let ext = Path::new(name)
            .extension()
            .map(|e| e.to_string_lossy().into_owned());
        let request = web_sys::js_sys::Object::new();
        let types = Array::new();
        if let Some(ext) = ext {
            types.push(&JsValue::from_str(&ext));
        }
        let set =
            |key: &str, value: &JsValue| Reflect::set(&request, &JsValue::from_str(key), value);
        let call = set("name", &JsValue::from_str(name))
            .and_then(|_| set("types", &types))
            .and_then(|_| set("bytes", &Uint8Array::from(bytes)))
            .and_then(|_| Reflect::get(files, &JsValue::from_str("saveAs")))
            .and_then(|save_as| {
                save_as
                    .unchecked_into::<web_sys::js_sys::Function>()
                    .call1(files, &request)
            });
        let (events, ctx, name) = (self.events.clone(), self.ctx.clone(), name.to_owned());
        let promise = match call {
            Ok(promise) => promise,
            Err(err) => {
                let event = FileEvent::Failed(SAVE_FAILED, format!("{name}: {}", text(&err)));
                return self.events.borrow_mut().push_back(event);
            }
        };
        wasm_bindgen_futures::spawn_local(async move {
            let event = match JsFuture::from(web_sys::js_sys::Promise::resolve(&promise)).await {
                Ok(answer) if answer.is_null() || answer.is_undefined() => None,
                Ok(_) => Some(FileEvent::Saved(PathBuf::from(&name))),
                Err(err) => Some(FileEvent::Failed(
                    SAVE_FAILED,
                    format!("{name}: {}", text(&err)),
                )),
            };
            events.borrow_mut().extend(event);
            if let Some(ctx) = ctx.borrow().as_ref() {
                ctx.request_repaint();
            }
        });
    }

    /// `Locate samples…` inside Playground: its Open picks a sample in the folder to search, and every file of that
    /// folder (and below) named one of `names` is read from Playground's Files. A file from the computer is only itself.
    fn locate_in_playground(&self, files: JsValue, names: Vec<String>) {
        let (events, ctx) = (self.events.clone(), self.ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let found = playground_locate(&files, &names).await;
            events.borrow_mut().push_back(match found {
                Ok(found) => FileEvent::Located(found),
                Err(err) => FileEvent::Failed(OPEN_FAILED, text(&err)),
            });
            if let Some(ctx) = ctx.borrow().as_ref() {
                ctx.request_repaint();
            }
        });
    }

    /// `Locate samples…`: a folder input (`webkitdirectory`); the files in it named one of `names` are read.
    fn pick_folder(&self, names: Vec<String>) -> Result<(), JsValue> {
        let input: HtmlInputElement = document()?.create_element("input")?.unchecked_into();
        input.set_type("file");
        input.set_multiple(true);
        input.set_attribute("webkitdirectory", "")?;
        let (events, ctx, picked) = (self.events.clone(), self.ctx.clone(), input.clone());
        let on_change = wasm_bindgen::closure::Closure::once_into_js(move || {
            let Some(files) = picked.files() else {
                return;
            };
            wasm_bindgen_futures::spawn_local(async move {
                let mut found: Vec<(String, Vec<u8>)> = Vec::new();
                for i in 0..files.length() {
                    let Some(file) = files.get(i) else { continue };
                    let name = file.name();
                    let wanted = names.iter().any(|n| n.eq_ignore_ascii_case(&name));
                    let have = found.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name));
                    if !wanted || have || file.size() > MAX_FILE_BYTES as f64 {
                        continue;
                    }
                    if let Ok(buffer) = JsFuture::from(file.array_buffer()).await {
                        found.push((name, Uint8Array::new(&buffer).to_vec()));
                    }
                }
                events.borrow_mut().push_back(FileEvent::Located(found));
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
    fn locate(&mut self, names: Vec<String>) {
        if let Some(files) = playground_files() {
            return self.locate_in_playground(files, names);
        }
        if let Err(err) = self.pick_folder(names) {
            let event = FileEvent::Failed(OPEN_FAILED, text(&err));
            self.events.borrow_mut().push_back(event);
        }
    }

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
        if let Some(files) = playground_files() {
            return self.save_in_playground(&files, &name, &bytes);
        }
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

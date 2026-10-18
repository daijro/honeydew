// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio
// Modified for Honeydew: Lemon Studio's plugin search and scanner (os-app `plugin_db.rs`, `plugin_scan.rs` and
// `plugins::load_file`), without its preferences, native plugins and built-in plugin list; the cache is kept in
// Honeydew's settings store, and the yabridge / vstbridge folders join the standard ones.

//! Finding plugin files and scanning them. Each file is loaded in a scanner process (this program run with
//! [`ARG`]), so that a plugin that crashes, hangs or opens a window while it loads cannot harm the app; the results
//! are cached by file size and modification time, so a file is scanned again only when it changes (or on a Rescan).

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use os_host::{Descriptor, Format, Kind, Library};
use serde::{Deserialize, Serialize};

/// The scanner's command-line switch: `honeydew-native --scan-plugin <format> <path>`.
pub const ARG: &str = "--scan-plugin";
/// The scanner's one line on standard output starts with this.
const TAG: &str = "HDSCAN ";
/// A scanner still running after this is ended (status `timed out`).
const TIMEOUT: Duration = Duration::from_secs(30);
/// Scanners running at the same time.
const WORKERS: usize = 4;
/// How deep below a search folder plugin files are looked for.
const MAX_DEPTH: u32 = 8;

/// The folders searched by default: the standard VST3, CLAP and VST2 folders of the platform, and on Linux the
/// folders yabridge (and its fork vstbridge) put their bridged Windows plugins in, `~/.vst3/yabridge`,
/// `~/.vst/yabridge` and `~/.clap/yabridge`. Each is searched for every format.
pub fn default_folders() -> Vec<PathBuf> {
    let mut dirs = os_vst3::standard_dirs();
    dirs.extend(os_clap::standard_dirs());
    dirs.extend(os_vst2::standard_dirs());
    #[cfg(target_os = "linux")]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for format in [".vst3", ".vst", ".clap"] {
            dirs.push(home.join(format).join("yabridge"));
        }
    }
    let mut seen = HashSet::new();
    dirs.retain(|d| seen.insert(path_key(d)));
    dirs
}

/// What scanning a file gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Error,
    Crashed,
    Timeout,
    /// Loaded, but no instrument or effect in it.
    Empty,
    /// A plugin this program cannot load, as its file tells without loading it (a 32-bit plugin).
    Unsupported,
}

/// A plugin a scanned file offers (a `Descriptor` without its format and path).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub kind: String,
}

impl Record {
    pub fn of(d: &Descriptor) -> Record {
        Record {
            id: d.id.clone(),
            name: d.name.clone(),
            vendor: d.vendor.clone(),
            version: d.version.clone(),
            features: d.features.clone(),
            kind: match d.kind {
                Kind::Instrument => "instrument",
                Kind::Effect => "effect",
                Kind::Other => "other",
            }
            .into(),
        }
    }

    pub fn kind(&self) -> Kind {
        match self.kind.as_str() {
            "instrument" => Kind::Instrument,
            "effect" => Kind::Effect,
            _ => Kind::Other,
        }
    }

    pub fn descriptor(&self, format: Format, path: &Path) -> Descriptor {
        Descriptor {
            format,
            id: self.id.clone(),
            name: self.name.clone(),
            vendor: self.vendor.clone(),
            version: self.version.clone(),
            features: self.features.clone(),
            kind: self.kind(),
            path: path.to_path_buf(),
        }
    }
}

/// One scanned file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileScan {
    pub path: String,
    pub format: String,
    #[serde(default)]
    pub size: u64,
    /// Seconds since 1970.
    #[serde(default)]
    pub modified: u64,
    pub status: Status,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub plugins: Vec<Record>,
}

/// The scan results, as the settings store keeps them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Cache {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub files: Vec<FileScan>,
}

/// A path compared without case and trailing separators on Windows.
fn path_key(p: &Path) -> String {
    let mut s = p.to_string_lossy().into_owned();
    if cfg!(windows) {
        s = s.replace('/', "\\").to_lowercase();
    }
    while s.len() > 1 && (s.ends_with('\\') || s.ends_with('/')) && !s.ends_with(":\\") {
        s.pop();
    }
    s
}

/// A plugin file the search found.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub path: PathBuf,
    pub format: Format,
}

/// The plugin files in `folders`: `.clap` files (bundles on macOS), `.vst3` bundles or files and VST2 plugins
/// (libraries that export a VST2 entry point, `.vst` bundles on macOS); a bundle is not searched inside. Each file once.
pub fn discover(folders: &[PathBuf]) -> Vec<Found> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut walked = HashSet::new();
    for dir in folders {
        walk(dir, 0, &mut walked, &mut seen, &mut out);
    }
    out
}

fn walk(
    dir: &Path,
    depth: u32,
    walked: &mut HashSet<PathBuf>,
    seen: &mut HashSet<String>,
    out: &mut Vec<Found>,
) {
    // A folder reached twice (links, nested search folders) is searched once.
    let canon = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if depth > MAX_DEPTH || !walked.insert(canon) {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort_by_key(|p| path_key(p));
    for p in entries {
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase());
        let format = match ext.as_deref() {
            Some("clap") if os_clap::is_plugin(&p) => Some(Format::Clap),
            Some("vst3") if p.is_dir() || p.is_file() => Some(Format::Vst3),
            Some(x) if x == os_vst2::EXTENSION && os_vst2::is_plugin(&p) => Some(Format::Vst2),
            _ => None,
        };
        match format {
            Some(format) => {
                if seen.insert(path_key(&p)) {
                    out.push(Found { path: p, format });
                }
            }
            None if p.is_dir() => walk(&p, depth + 1, walked, seen, out),
            None => {}
        }
    }
}

/// A file's size and modification time: a bundle's module inside it (VST3; CLAP and VST2 on macOS), else the file.
fn stamp(path: &Path, format: Format) -> (u64, u64) {
    let module = match format {
        Format::Vst3 => os_vst3::module_file(path),
        Format::Clap => os_clap::module_file(path),
        Format::Vst2 => os_vst2::module_file(path),
        Format::Native => path.to_path_buf(),
    };
    let file = if module.exists() {
        module
    } else {
        path.to_path_buf()
    };
    let Ok(md) = std::fs::metadata(&file) else {
        return (0, 0);
    };
    let modified = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    (if md.is_dir() { 0 } else { md.len() }, modified)
}

/// What scanning one file gave.
#[derive(Clone, Debug, PartialEq)]
pub struct Scanned {
    pub status: Status,
    pub error: String,
    pub plugins: Vec<Record>,
}

impl Scanned {
    fn ok(plugins: Vec<Record>) -> Scanned {
        Scanned {
            status: Status::Ok,
            error: String::new(),
            plugins,
        }
    }

    fn failed(status: Status, error: impl Into<String>) -> Scanned {
        Scanned {
            status,
            error: error.into(),
            plugins: Vec::new(),
        }
    }
}

/// Scans the files of `found` with up to [`WORKERS`] scanner processes of `scanner` at a time: new and changed files,
/// or all of them with `all` (Rescan); the others keep their result from `old`. Files no longer found drop out.
pub fn sweep(found: &[Found], old: &Cache, all: bool, scanner: &Path) -> Cache {
    let old_by: HashMap<(String, &str), &FileScan> = old
        .files
        .iter()
        .map(|f| ((path_key(Path::new(&f.path)), f.format.as_str()), f))
        .collect();
    let mut files: Vec<Option<FileScan>> = vec![None; found.len()];
    let mut todo = Vec::new();
    for (i, f) in found.iter().enumerate() {
        let (size, modified) = stamp(&f.path, f.format);
        match old_by.get(&(path_key(&f.path), f.format.as_str())) {
            Some(p) if !all && p.size == size && p.modified == modified => {
                files[i] = Some((*p).clone())
            }
            _ => todo.push((i, size, modified)),
        }
    }
    let next = AtomicUsize::new(0);
    let results = Mutex::new(files);
    std::thread::scope(|sc| {
        for _ in 0..WORKERS.min(todo.len()) {
            sc.spawn(|| {
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(i, size, modified)) = todo.get(k) else {
                        break;
                    };
                    let f = &found[i];
                    let r = scan_file(scanner, &f.path, f.format);
                    let usable = r.plugins.iter().any(|p| p.kind() != Kind::Other);
                    let status = if r.status == Status::Ok && !usable {
                        Status::Empty
                    } else {
                        r.status
                    };
                    let scan = FileScan {
                        path: f.path.to_string_lossy().into_owned(),
                        format: f.format.as_str().into(),
                        size,
                        modified,
                        status,
                        error: r.error,
                        plugins: r.plugins,
                    };
                    if let Ok(mut v) = results.lock() {
                        v[i] = Some(scan);
                    }
                }
            });
        }
    });
    Cache {
        version: 1,
        files: results
            .into_inner()
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .collect(),
    }
}

/// Scans one file: a plugin this program cannot load (its file tells) is not tried at all; the others are loaded in a
/// scanner process.
fn scan_file(scanner: &Path, path: &Path, format: Format) -> Scanned {
    if let Some(why) = (format == Format::Vst2)
        .then(|| os_vst2::unsupported(path))
        .flatten()
    {
        return Scanned::failed(Status::Unsupported, why);
    }
    scan_in_child(scanner, path, format)
}

/// Scans file `path` in a scanner process, the program `scanner`.
fn scan_in_child(scanner: &Path, path: &Path, format: Format) -> Scanned {
    let mut cmd = std::process::Command::new(scanner);
    cmd.arg(ARG)
        .arg(format.as_str())
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Scanned::failed(Status::Error, format!("the scanner did not start: {e}")),
    };
    let (tx, rx) = mpsc::channel();
    if let Some(out) = child.stdout.take() {
        // Plugins may print anything while they load: only the tagged line counts.
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Some(rest) = line.strip_prefix(TAG) {
                    let _ = tx.send(rest.to_string());
                    break;
                }
            }
        });
    }
    let line = match rx.recv_timeout(TIMEOUT) {
        Ok(l) => Some(l),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = child.kill();
            let _ = child.wait();
            return Scanned::failed(
                Status::Timeout,
                format!("still loading after {} s", TIMEOUT.as_secs()),
            );
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => None,
    };
    // The scanner ends itself after its line; one that does not is ended.
    let end = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                break child.wait().ok();
            }
        }
    };
    match line {
        Some(l) => parse(&l),
        None => {
            let code = status.and_then(|s| s.code()).map_or(0, |c| c as u32);
            Scanned::failed(
                Status::Crashed,
                format!(
                    "the scanner ended with {} while loading it",
                    os_host::guard::describe(code)
                ),
            )
        }
    }
}

/// The outcome of a scanner's line.
fn parse(line: &str) -> Scanned {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return Scanned::failed(Status::Error, "the scanner's answer was unreadable");
    };
    if v["ok"].as_bool() == Some(true) {
        return Scanned::ok(
            serde_json::from_value::<Vec<Record>>(v["plugins"].clone()).unwrap_or_default(),
        );
    }
    let error = v["error"]
        .as_str()
        .unwrap_or("the plugin did not load")
        .to_string();
    let status = if v["crashed"].as_bool() == Some(true) {
        Status::Crashed
    } else {
        Status::Error
    };
    Scanned::failed(status, error)
}

/// Loads plugin file `path` of `format` under the crash guard.
pub fn load_file(path: &Path, format: Format) -> Result<Arc<dyn Library>, String> {
    let load = || -> Result<Arc<dyn Library>, String> {
        match format {
            Format::Clap => os_clap::Library::load(path).map(|l| l as Arc<dyn Library>),
            Format::Vst3 => os_vst3::Module::load(path).map(|l| l as Arc<dyn Library>),
            Format::Vst2 => os_vst2::Module::load(path).map(|l| l as Arc<dyn Library>),
            Format::Native => Err(format!("{}: not a supported plugin format", path.display())),
        }
    };
    os_host::guard::catch(load).unwrap_or_else(|c| {
        Err(format!(
            "{}: crashed while loading ({})",
            path.display(),
            os_host::guard::describe(c)
        ))
    })
}

/// The scanner process (`args` are the ones after [`ARG`]): loads one plugin file, prints its plugin list as one
/// tagged JSON line and ends at once, without unloading the plugin (its clean-up code never runs).
pub fn child_main(args: &[String]) -> ! {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
        use windows_sys::Win32::System::Diagnostics::Debug::{
            SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX, SetErrorMode,
        };
        // No error boxes (a missing DLL, a crash): the app reads the outcome from the exit code.
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);
        // The same thread model as the app's UI thread, where plugins are loaded for use.
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
    }
    let format = args.first().and_then(|f| Format::parse(f));
    let path = args.get(1).map(PathBuf::from);
    let out = match (format, path) {
        (Some(format), Some(path)) => match load_file(&path, format) {
            Ok(lib) => {
                let plugins: Vec<Record> = lib.plugins().iter().map(Record::of).collect();
                let out = serde_json::json!({ "ok": true, "plugins": plugins });
                // Dropping it would unload the plugin (on Windows), running the clean-up code this process skips.
                std::mem::forget(lib);
                out
            }
            Err(e) => {
                serde_json::json!({ "ok": false, "error": e, "crashed": os_host::guard::tripped() })
            }
        },
        _ => {
            serde_json::json!({ "ok": false, "error": format!("usage: {ARG} clap|vst3|vst2 <path>") })
        }
    };
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{TAG}{out}");
    let _ = stdout.flush();
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        TerminateProcess(GetCurrentProcess(), 0);
    }
    std::process::exit(0)
}

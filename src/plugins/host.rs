//! The hosted plugins, on the app thread: the built-in ones ([`super::builtin`], every platform) and the plugins the
//! scan found (the desktop), and an instance per plugin module of the song, keyed by channel id and instance id. An
//! instance outlives engine builds (every edit builds one): the engines get it shared ([`super::engine`]). A module
//! whose plugin is not installed keeps its state and plays nothing.
//!
//! Threading follows Lemon Studio's hosts: everything here runs on the app thread, the audio thread only `try_lock`s an
//! instance, and an instance is destroyed on the app thread once no engine holds it.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
#[cfg(plugin_host)]
use std::sync::mpsc;
use std::sync::{Arc, OnceLock, TryLockError};

use os_host::{Descriptor, Format, Kind, Library, ParamEvent, SharedPlugin};

use super::builtin;
use super::engine::{ParamShared, PluginEffect, PluginInstrument};
#[cfg(plugin_host)]
use super::scan::{self, Cache, Status};
use super::window::{self, EditorWindow};
use super::{Available, Info, PluginKind, PluginMove, State};
use crate::song::plugin::{PLUGIN_EFFECT, PLUGIN_INSTRUMENT, PluginParam, PluginRef, is_plugin};
use crate::song::{Channel, ModuleInstance, Rack, Song};

/// A rate to start an instance at before the audio output says its own; the audio thread asks for the stream's, and
/// the next [`idle`] reactivates.
const START_RATE: u32 = 48_000;

/// A plugin file of the scan, loaded the first time one of its plugins is created.
struct FileLib {
    path: PathBuf,
    #[cfg_attr(not(plugin_host), allow(dead_code))]
    format: Format,
    lib: OnceLock<Result<Arc<dyn Library>, String>>,
}

impl FileLib {
    fn get(&self) -> Result<Arc<dyn Library>, String> {
        #[cfg(plugin_host)]
        let load = || scan::load_file(&self.path, self.format);
        #[cfg(not(plugin_host))]
        let load = || {
            Err(format!(
                "{}: plugins run in the desktop app only",
                self.path.display()
            ))
        };
        self.lib.get_or_init(load).clone()
    }
}

/// A plugin the scan found, or a built-in one.
struct Entry {
    desc: Descriptor,
    file: Arc<FileLib>,
    builtin: bool,
}

/// The built-in plugins' entries ([`builtin::libraries`]), listed before the scan's.
fn builtin_entries() -> Vec<Entry> {
    (builtin::libraries().iter())
        .map(|(desc, lib)| Entry {
            desc: desc.clone(),
            file: Arc::new(FileLib {
                path: desc.path.clone(),
                format: desc.format,
                lib: OnceLock::from(Ok(lib.clone())),
            }),
            builtin: true,
        })
        .collect()
}

/// A plugin module's instance.
struct Hosted {
    plugin: SharedPlugin,
    /// The plugin's param ids, in its list order: module param `1 + i` is `ids[i]`.
    ids: Vec<u32>,
    params: Arc<ParamShared>,
    /// The module's params `1..` as the last sync saw them: a change there is the song's (an undo, a remote edit) and
    /// goes to the plugin.
    song_values: Vec<f32>,
    window: Option<EditorWindow>,
    /// A built-in plugin's editor, drawn into its rack card: its id and scale ([`lemon_editor`]).
    embedded: Option<(u32, f32)>,
    name: String,
    /// Its crash was reported.
    stopped: bool,
    /// Its editor failed to open (an X error, no X display, the plugin refused): why; it is not offered again.
    editor_failed: Option<String>,
    /// The card keeps its last successful reads while the audio thread holds the plugin lock.
    card_info: Option<Info>,
    card_text: HashMap<usize, (f32, String)>,
    /// The module's state it was started from: a copy of the module (a channel's clone, a paste) shares it, and starts
    /// from this instance's state now instead.
    origin: Arc<[u8]>,
}

impl Drop for Hosted {
    /// The editor closes before its window goes.
    fn drop(&mut self) {
        close_editor_of(self);
    }
}

#[derive(Default)]
struct Host {
    entries: Vec<Entry>,
    #[cfg(plugin_host)]
    cache: Cache,
    live: HashMap<(i32, i32), Hosted>,
    /// Instances whose module left the song (a delete, an undo): an undo or redo that brings it back finds it here.
    parked: HashMap<(i32, i32), Hosted>,
    /// Instances to destroy once no engine holds them.
    retired: Vec<SharedPlugin>,
    /// Modules whose plugin is missing or did not start, and why; tried again after a song load or a rescan.
    failed: HashMap<(i32, i32), String>,
    notices: BTreeSet<String>,
    rate: u32,
    rendering: bool,
    #[cfg(plugin_host)]
    scan: Option<mpsc::Receiver<Cache>>,
    /// A scan asked for while one ran, started when it ends: its folders, `all`, the scanner.
    #[cfg(plugin_host)]
    queued: Option<(Vec<PathBuf>, bool, PathBuf)>,
}

thread_local! {
    static HOST: RefCell<Host> = RefCell::new(Host {
        entries: builtin_entries(),
        ..Host::default()
    });
}

fn with<R>(f: impl FnOnce(&mut Host) -> R) -> R {
    HOST.with_borrow_mut(f)
}

/// At the app's start: a plugin that wants the app thread wakes `ctx`'s loop, the cached scan (`cache`, the settings
/// store's [`super::CACHE_KEY`]) is listed, and new or changed files are scanned in the background ([`rescan_folders`]).
pub fn start(ctx: &eframe::egui::Context, cache: Option<String>, folders: &[String]) {
    static CTX: OnceLock<eframe::egui::Context> = OnceLock::new();
    let _ = CTX.set(ctx.clone());
    fn wake() {
        if let Some(ctx) = CTX.get() {
            ctx.request_repaint();
        }
    }
    os_host::set_waker(wake);
    os_plugkit::embed::set_repaint(wake);
    #[cfg(plugin_host)]
    if let Some(json) = cache {
        load_cache(&json);
    }
    #[cfg(not(plugin_host))]
    let _ = cache;
    rescan_folders(folders, false);
}

/// The web and Android scan nothing: only the built-in plugins are there.
#[cfg(not(plugin_host))]
mod no_scan {
    pub fn rescan_folders(_: &[String], _: bool) {}

    pub fn poll_scan() -> bool {
        false
    }

    pub fn scanning() -> bool {
        false
    }

    pub fn scan_counts() -> (usize, usize) {
        (0, 0)
    }

    pub fn cache_json() -> String {
        String::new()
    }
}
#[cfg(not(plugin_host))]
pub use no_scan::*;

#[cfg(plugin_host)]
/// Scans the default folders and the user's `folders` ([`rescan`]) with this program as the scanner.
pub fn rescan_folders(folders: &[String], all: bool) {
    let mut all_folders = scan::default_folders();
    all_folders.extend(folders.iter().map(PathBuf::from));
    if let Ok(scanner) = std::env::current_exe() {
        rescan(all_folders, all, scanner);
    }
}

#[cfg(plugin_host)]
/// Lists the plugins of scan results `cache` (the settings store's JSON); unreadable JSON is an empty list.
pub fn load_cache(json: &str) {
    let cache = serde_json::from_str(json).unwrap_or_default();
    with(|host| host.set_cache(cache));
}

#[cfg(plugin_host)]
/// The scan results as JSON, for the settings store.
pub fn cache_json() -> String {
    with(|host| serde_json::to_string(&host.cache).unwrap_or_default())
}

#[cfg(plugin_host)]
/// Starts a scan of `folders` in the background with the scanner program `scanner`: new and changed files, or every
/// file with `all`. [`poll_scan`] takes the result. A scan already running goes on.
pub fn rescan(folders: Vec<PathBuf>, all: bool, scanner: PathBuf) {
    with(|host| {
        if host.scan.is_some() {
            let all = all || host.queued.as_ref().is_some_and(|q| q.1);
            host.queued = Some((folders, all, scanner));
            return;
        }
        start_scan(host, folders, all, scanner);
    });
}

#[cfg(plugin_host)]
fn start_scan(host: &mut Host, folders: Vec<PathBuf>, all: bool, scanner: PathBuf) {
    let old = host.cache.clone();
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("plugin scan".into())
        .spawn(move || {
            let found = scan::discover(&folders);
            let _ = tx.send(scan::sweep(&found, &old, all, &scanner));
            os_host::wake();
        });
    if spawned.is_ok() {
        host.scan = Some(rx);
    }
}

#[cfg(plugin_host)]
/// A scan is running or waiting.
pub fn scanning() -> bool {
    with(|host| host.scan.is_some())
}

#[cfg(plugin_host)]
/// The finished scan's result, once: true when it came (the app stores [`cache_json`]). Modules that missed their plugin
/// try again.
pub fn poll_scan() -> bool {
    with(|host| {
        let Some(rx) = &host.scan else { return false };
        match rx.try_recv() {
            Ok(cache) => {
                host.scan = None;
                host.set_cache(cache);
                host.failed.clear();
                if let Some((folders, all, scanner)) = host.queued.take() {
                    // Not through `rescan`, which takes the host again.
                    start_scan(host, folders, all, scanner);
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                host.scan = None;
                false
            }
        }
    })
}

/// The instruments and effects found, by name.
pub fn available() -> Vec<Available> {
    with(|host| {
        let mut list: Vec<Available> = (host.entries.iter())
            .filter(|e| !e.builtin)
            .filter_map(|e| {
                let kind = match e.desc.kind {
                    Kind::Instrument => PluginKind::Instrument,
                    Kind::Effect => PluginKind::Effect,
                    Kind::Other => return None,
                };
                Some(Available {
                    uid: e.desc.uid(),
                    name: e.desc.name.clone(),
                    vendor: e.desc.vendor.clone(),
                    format: format_label(e.desc.format),
                    kind,
                })
            })
            .collect();
        list.sort_by_key(|a| (a.name.to_lowercase(), a.format));
        list
    })
}

/// Whether the plugin `plugin` names is installed (by format and id, else by name, as a song's module finds it).
pub fn installed(plugin: &PluginRef) -> bool {
    with(|host| host.find(plugin).is_some())
}

#[cfg(plugin_host)]
/// The scan's counts: files scanned and files that failed (crashed, timed out, did not load).
pub fn scan_counts() -> (usize, usize) {
    with(|host| {
        let failed = |f: &&scan::FileScan| {
            matches!(f.status, Status::Error | Status::Crashed | Status::Timeout)
        };
        (
            host.cache.files.len(),
            host.cache.files.iter().filter(failed).count(),
        )
    })
}

fn format_label(format: Format) -> &'static str {
    match format {
        Format::Clap => "CLAP",
        Format::Vst3 => "VST3",
        Format::Vst2 => "VST2",
        Format::Native => "Native",
    }
}

/// The audio output's rate, for instances started from now on.
pub fn set_rate(rate: u32) {
    with(|host| host.rate = rate);
}

/// A new rack module hosting plugin `uid` ([`available`]) on the channel with id `channel_id`, with instance id
/// `instance_id`: its instance is started now, and the module holds the plugin's identity, state and params.
pub fn instantiate(channel_id: i32, instance_id: i32, uid: &str) -> Result<ModuleInstance, String> {
    with(|host| {
        let entry = (host.entries.iter())
            .find(|e| e.desc.uid() == uid)
            .ok_or_else(|| format!("{uid} is not installed"))?;
        let (desc, file) = (entry.desc.clone(), entry.file.clone());
        let plugin = start_instance(&file, &desc, None, host.rate_or_start())?;
        let (ids, infos, values) = param_list(&plugin);
        let state = plugin
            .lock()
            .map_err(|_| "the plugin stopped".to_string())?
            .save_state();
        let type_id = if desc.kind == Kind::Instrument {
            PLUGIN_INSTRUMENT
        } else {
            PLUGIN_EFFECT
        };
        let mut module = ModuleInstance::new(type_id, instance_id);
        module.params.extend(&values);
        let state: Arc<[u8]> = state.unwrap_or_default().into();
        module.file.plugin = Some(Box::new(PluginRef {
            format: desc.format.as_str().into(),
            id: desc.id.clone(),
            name: desc.name.clone(),
            vendor: desc.vendor.clone(),
            path: desc.path.to_string_lossy().into_owned(),
            state: state.clone(),
            params: infos,
        }));
        host.live.insert(
            (channel_id, instance_id),
            Hosted {
                params: Arc::new(ParamShared::new(&values)),
                plugin,
                ids,
                song_values: values,
                window: None,
                embedded: None,
                name: desc.name,
                stopped: false,
                editor_failed: None,
                card_info: None,
                card_text: HashMap::new(),
                origin: state,
            },
        );
        Ok(module)
    })
}

/// Adds a module hosting plugin `uid` ([`available`]) to `rack` at chain position `index` (clamped), on the channel
/// with id `channel_id`: [`instantiate`] with the rack's next instance id, which it returns.
pub fn add(rack: &mut Rack, channel_id: i32, uid: &str, index: usize) -> Result<i32, String> {
    let module = instantiate(channel_id, rack.last_instance_id + 1, uid)?;
    let id = rack.insert(index, module.type_id);
    let index = rack.index_of(id).expect("just added");
    rack.modules[index] = module;
    Ok(id)
}

/// Creates plugin `desc` of `file`, restores `state`, activates it at `rate`.
fn start_instance(
    file: &FileLib,
    desc: &Descriptor,
    state: Option<&[u8]>,
    rate: u32,
) -> Result<SharedPlugin, String> {
    window::catch_x_errors();
    let lib = file.get()?;
    let plugin = lib.create(&desc.id)?;
    {
        let mut p = plugin
            .lock()
            .map_err(|_| format!("{} stopped", desc.name))?;
        if let Some(state) = state.filter(|s| !s.is_empty())
            && !p.load_state(state)
        {
            eprintln!("plugins: {} did not take its saved state", desc.name);
        }
        if !p.activate(rate) {
            return Err(format!("{} could not start", desc.name));
        }
    }
    Ok(plugin)
}

/// The plugin's params: their ids, their ids, names and defaults, and their values now.
fn param_list(plugin: &SharedPlugin) -> (Vec<u32>, Vec<PluginParam>, Vec<f32>) {
    let Ok(p) = plugin.lock() else {
        return Default::default();
    };
    let infos: Vec<_> = (0..p.param_count())
        .filter_map(|i| p.param_info(i))
        .collect();
    let ids = infos.iter().map(|i| i.id).collect();
    let values = (infos.iter())
        .map(|i| p.param_value(i.id).unwrap_or(i.default) as f32)
        .collect();
    let params = (infos.into_iter())
        .map(|i| PluginParam {
            id: i.id,
            name: i.name,
            default: i.default as f32,
        })
        .collect();
    (ids, params, values)
}

impl Host {
    #[cfg(plugin_host)]
    fn set_cache(&mut self, cache: Cache) {
        let mut files: HashMap<(String, String), Arc<FileLib>> = HashMap::new();
        // A file loaded already stays loaded: its instances run on.
        for e in self.entries.drain(..) {
            files.insert(
                (
                    e.file.path.to_string_lossy().into_owned(),
                    e.desc.format.as_str().into(),
                ),
                e.file,
            );
        }
        let mut entries = builtin_entries();
        for f in cache.files.iter().filter(|f| f.status == Status::Ok) {
            let Some(format) = Format::parse(&f.format) else {
                continue;
            };
            let path = PathBuf::from(&f.path);
            let file =
                (files.get(&(f.path.clone(), f.format.clone())).cloned()).unwrap_or_else(|| {
                    Arc::new(FileLib {
                        path: path.clone(),
                        format,
                        lib: OnceLock::new(),
                    })
                });
            for record in &f.plugins {
                let desc = record.descriptor(format, &path);
                // The first file wins for a plugin found twice; a built-in plugin's own copy is left out.
                let builtin = builtin::find(desc.format.as_str(), &desc.id).is_some();
                if !builtin && !entries.iter().any(|e| e.desc.uid() == desc.uid()) {
                    entries.push(Entry {
                        desc,
                        file: file.clone(),
                        builtin: false,
                    });
                }
            }
        }
        self.entries = entries;
        self.cache = cache;
    }

    fn rate_or_start(&self) -> u32 {
        if self.rate > 0 { self.rate } else { START_RATE }
    }

    /// The installed plugin a module's `plugin` names: the same format and id, else the same format, file name and name,
    /// else the same format and name. Another format's copy would not read the state.
    // ponytail: no cross-format stand-in (Lemon plays another format's copy from defaults); add it if users ask.
    fn find(&self, plugin: &PluginRef) -> Option<&Entry> {
        // A built-in plugin (or its own copy installed): the built-in one.
        if let Some(b) = builtin::of(plugin) {
            return self
                .entries
                .iter()
                .find(|e| e.builtin && e.desc.id == b.clap_id());
        }
        let format = Format::parse(&plugin.format)?;
        let same = |e: &&Entry| e.desc.format == format;
        let file = |p: &str| p.rsplit(['/', '\\']).next().unwrap_or(p).to_lowercase();
        let entries = || self.entries.iter().filter(same);
        (entries().find(|e| !plugin.id.is_empty() && e.desc.id.eq_ignore_ascii_case(&plugin.id)))
            .or_else(|| {
                entries().find(|e| {
                    !plugin.path.is_empty()
                        && file(&e.desc.path.to_string_lossy()) == file(&plugin.path)
                        && e.desc.name == plugin.name
                })
            })
            .or_else(|| entries().find(|e| e.desc.name == plugin.name))
    }

    /// Starts the instance of a module read from a song (or another person's): its state restored, and its params set
    /// as the module has them, matched by param id.
    fn revive(&self, module: &ModuleInstance) -> Result<Hosted, String> {
        let plugin_ref = module.file.plugin.as_deref().ok_or("no plugin")?;
        let missing = || {
            let (name, format) = (&plugin_ref.name, plugin_ref.format_label());
            if cfg!(plugin_host) {
                format!(
                    "{name} ({format}) is not installed: it plays nothing, and its settings are kept."
                )
            } else {
                format!("{name} ({format}) runs in the desktop app only; its settings are kept.")
            }
        };
        let entry = self.find(plugin_ref).ok_or_else(missing)?;
        let (desc, file) = (entry.desc.clone(), entry.file.clone());
        // A copy of a running module starts from that instance's state now.
        let copied = |h: &&Hosted| {
            !h.origin.is_empty() && Arc::ptr_eq(&h.origin, &plugin_ref.state) && h.name == desc.name
        };
        let source = self.live.values().find(copied);
        let now = source.and_then(|h| h.plugin.lock().ok()?.save_state());
        let state = now.as_deref().unwrap_or(&plugin_ref.state);
        let plugin = start_instance(&file, &desc, Some(state), self.rate_or_start())?;
        let (ids, _, values) = param_list(&plugin);
        let saved = saved_params(module);
        let shared = Arc::new(ParamShared::new(&values));
        if let Ok(mut p) = plugin.lock() {
            for (i, id) in ids.iter().enumerate() {
                if let Some(&v) = saved.get(id).filter(|&&v| v != values[i]) {
                    p.set_param(*id, f64::from(v));
                    shared.known(i, v);
                }
            }
        }
        let song_values = song_values(module, &ids);
        Ok(Hosted {
            plugin,
            ids,
            params: shared,
            song_values,
            window: None,
            embedded: None,
            origin: plugin_ref.state.clone(),
            name: desc.name,
            stopped: false,
            editor_failed: None,
            card_info: None,
            card_text: HashMap::new(),
        })
    }
}

/// A module's saved plugin params by param id.
fn saved_params(module: &ModuleInstance) -> HashMap<u32, f32> {
    let Some(plugin) = &module.file.plugin else {
        return HashMap::new();
    };
    (plugin.params.iter().zip(module.params.iter().skip(1)))
        .map(|(p, &v)| (p.id, v))
        .collect()
}

/// The module's params `1..` when they are the plugin's `ids` in its order, else none (the module is brought in line
/// by [`adopt_params`] first).
fn song_values(module: &ModuleInstance, ids: &[u32]) -> Vec<f32> {
    let lists = module
        .file
        .plugin
        .as_ref()
        .is_some_and(|p| p.params.iter().map(|p| p.id).eq(ids.iter().copied()));
    if lists {
        module.params.iter().skip(1).copied().collect()
    } else {
        Vec::new()
    }
}

/// Matches the instances to `song`'s plugin modules: a module without one gets it (its state restored), a module
/// whose params changed in the song (an undo, another person's edit) sets them on its plugin, and an instance whose
/// module left the song is parked for an undo. A module whose plugin is missing is reported once ([`take_notices`]).
/// The audio link runs it before every engine build; the app every frame.
pub fn sync(song: &Song) {
    with(|host| {
        let mut seen = Vec::new();
        for channel in &song.channels {
            let channel_id = channel.id;
            for module in channel.rack.modules.iter().filter(|m| is_plugin(m.type_id)) {
                let key = (channel_id, module.instance_id);
                seen.push(key);
                if let Some(hosted) = host.parked.remove(&key) {
                    host.live.insert(key, hosted);
                }
                if let Some(hosted) = host.live.get_mut(&key) {
                    push_song_params(hosted, module);
                    continue;
                }
                if host.failed.contains_key(&key) || module.file.plugin.is_none() {
                    continue;
                }
                match host.revive(module) {
                    Ok(hosted) => {
                        host.live.insert(key, hosted);
                    }
                    Err(why) => {
                        host.notices.insert(why.clone());
                        host.failed.insert(key, why);
                    }
                }
            }
        }
        let gone: Vec<_> = host
            .live
            .keys()
            .filter(|k| !seen.contains(k))
            .copied()
            .collect();
        for key in gone {
            let mut hosted = host.live.remove(&key).expect("listed");
            close_editor_of(&mut hosted);
            host.parked.insert(key, hosted);
        }
    });
}

/// Brings plugin modules whose params are not their running plugin's list in line with it: a module an import made
/// (no params) takes the plugin's values, one whose plugin now lists other params keeps its values by param id.
/// Whether the song changed (a repair of what was read, no undo step).
pub fn adopt_params(song: &mut Song) -> bool {
    with(|host| {
        let mut changed = false;
        for channel in &mut song.channels {
            let channel_id = channel.id;
            for module in channel
                .rack
                .modules
                .iter_mut()
                .filter(|m| is_plugin(m.type_id))
            {
                let Some(hosted) = host.live.get_mut(&(channel_id, module.instance_id)) else {
                    continue;
                };
                if !song_values(module, &hosted.ids).is_empty() || hosted.ids.is_empty() {
                    continue;
                }
                let saved = saved_params(module);
                let (_, infos, values) = param_list(&hosted.plugin);
                let Some(plugin_ref) = module.file.plugin.as_deref_mut() else {
                    continue;
                };
                plugin_ref.params = infos;
                module.params.truncate(1);
                module.params.extend(
                    hosted
                        .ids
                        .iter()
                        .zip(&values)
                        .map(|(id, v)| saved.get(id).copied().unwrap_or(*v)),
                );
                hosted.song_values = module.params.iter().skip(1).copied().collect();
                changed = true;
            }
        }
        changed
    })
}

/// The module's params that changed in the song since the last sync, to the plugin (app thread).
fn push_song_params(hosted: &mut Hosted, module: &ModuleInstance) {
    if hosted.song_values.is_empty() {
        return;
    }
    let mut plugin = None;
    for (i, &v) in module.params.iter().skip(1).enumerate() {
        let Some(old) = hosted.song_values.get_mut(i) else {
            break;
        };
        if *old == v {
            continue;
        }
        *old = v;
        let Some(&id) = hosted.ids.get(i) else {
            continue;
        };
        if plugin.is_none() {
            plugin = hosted.plugin.lock().ok();
        }
        if let Some(p) = plugin.as_mut() {
            p.set_param(id, f64::from(v));
            hosted.params.known(i, v);
        }
    }
}

/// A new song (New, Open, Import, a template): every instance goes, and missing plugins are looked for again.
pub fn song_replaced() {
    with(|host| {
        let live = std::mem::take(&mut host.live).into_values();
        let parked = std::mem::take(&mut host.parked).into_values();
        for mut hosted in live.chain(parked) {
            close_editor_of(&mut hosted);
            host.retired.push(hosted.plugin.clone());
        }
        host.failed.clear();
        host.notices.clear();
    });
}

/// Closes the plugin's editor, then its window (an editor whose window went first crashes); a built-in plugin's editor
/// in its card.
fn close_editor_of(hosted: &mut Hosted) {
    if hosted.embedded.take().is_some()
        && let Ok(mut p) = hosted.plugin.lock()
    {
        p.close_editor();
    }
    let Some(window) = hosted.window.take() else {
        return;
    };
    window::editor_call(|| {
        if let Ok(mut p) = hosted.plugin.lock() {
            p.close_editor();
        }
    });
    drop(window);
}

/// The missing or failed plugins met since the last call, one line each.
pub fn take_notices() -> Vec<String> {
    with(|host| std::mem::take(&mut host.notices).into_iter().collect())
}

/// The app thread's housekeeping, every frame: the editors' windows, the plugins' callbacks and run loop, their editors'
/// sizes, and the param changes the plugins made themselves (their editors), which the app applies to the song.
pub fn idle() -> Vec<PluginMove> {
    let closed = window::pump();
    // Editor code runs in the run loop's callbacks and the plugins' idle calls.
    if with(|host| host.live.values().any(|h| h.window.is_some())) {
        window::editor_call(|| idle_plugins(&closed)).0
    } else {
        idle_plugins(&closed)
    }
}

fn idle_plugins(closed: &[usize]) -> Vec<PluginMove> {
    #[cfg(unix)]
    os_host::runloop::dispatch();
    with(|host| {
        let mut moves = Vec::new();
        for (&(channel_id, instance_id), hosted) in &mut host.live {
            if (hosted.window.as_ref()).is_some_and(|w| closed.iter().any(|&c| w.is(c))) {
                close_editor_of(hosted);
            }
            let Ok(mut p) = hosted.plugin.try_lock() else {
                continue;
            };
            if let Some(code) = p.stopped().filter(|_| !hosted.stopped) {
                hosted.stopped = true;
                if let Some(window) = hosted.window.take() {
                    p.close_editor();
                    drop(window);
                }
                host.notices
                    .insert(os_host::guard::stopped_text(&hosted.name, code));
            }
            p.idle();
            if let (Some(w), Some(size)) = (&hosted.window, p.editor_size()) {
                w.resize(size);
            }
            for event in p.take_param_events() {
                let (id, kind) = match event {
                    ParamEvent::Begin(id) => (id, 0),
                    ParamEvent::Value(id, _) => (id, 1),
                    ParamEvent::End(id) => (id, 2),
                };
                let Some(index) = hosted.ids.iter().position(|&i| i == id) else {
                    continue;
                };
                let value = match event {
                    ParamEvent::Value(_, v) => v as f32,
                    _ => p.param_value(id).map_or(f32::NAN, |v| v as f32),
                };
                if kind == 1 {
                    hosted.params.known(index, value);
                    if let Some(old) = hosted.song_values.get_mut(index) {
                        *old = value;
                    }
                }
                moves.push(PluginMove {
                    channel_id,
                    instance_id,
                    param: index as i32 + 1,
                    value,
                    gesture: kind,
                });
            }
        }
        // Destroyed on this thread, once the engines let go.
        host.retired.retain(|p| Arc::strong_count(p) > 1);
        moves
    })
}

/// Issue #33: when the app thread should run [`idle`] again without input (the editors draw from it): in about 16 ms
/// while a plugin's editor is open (its window, VST2's editor idle) or the run loop watches descriptors (an editor's X11
/// connection), else when the run loop's first timer is due; `None` while nothing waits, so no frames are spent on
/// plugins. A finished scan wakes the app itself.
pub fn idle_after() -> Option<std::time::Duration> {
    const EDITOR: std::time::Duration = std::time::Duration::from_millis(16);
    #[cfg(unix)]
    let (polling, timer) = (
        os_host::runloop::polling(),
        os_host::runloop::next_timer()
            .map(|due| due.saturating_duration_since(std::time::Instant::now())),
    );
    #[cfg(not(unix))]
    let (polling, timer) = (false, None);
    let editor = polling || with(|host| host.live.values().any(|h| h.window.is_some()));
    editor.then_some(EDITOR).into_iter().chain(timer).min()
}

/// Saves each instance's state into its module (a project save, a share): the plugins keep their state themselves, so
/// the song's copy is brought up to date only when it is written out. No undo step.
pub fn store_states(song: &mut Song) {
    with(|host| {
        for channel in &mut song.channels {
            let channel_id = channel.id;
            for module in channel
                .rack
                .modules
                .iter_mut()
                .filter(|m| is_plugin(m.type_id))
            {
                let Some(hosted) = host.live.get(&(channel_id, module.instance_id)) else {
                    continue;
                };
                let Some(plugin) = module.file.plugin.as_deref_mut() else {
                    continue;
                };
                // Edits waiting for a block (none plays while stopped) are applied first.
                let saved = hosted.plugin.lock().ok().and_then(|mut p| {
                    p.idle();
                    p.save_state()
                });
                let Some(state) = saved else { continue };
                if *plugin.state != state[..] {
                    plugin.state = state.into();
                }
            }
        }
    });
}

/// The module's plugin as the card shows it; a busy audio callback keeps the last successful read.
/// `None` for a module that is not a plugin module of the song as last synced.
pub fn state(channel_id: i32, instance_id: i32) -> Option<State> {
    with(|host| {
        let key = (channel_id, instance_id);
        if let Some(why) = host.failed.get(&key) {
            return Some(State::Missing(why.clone()));
        }
        let hosted = host.live.get_mut(&key)?;
        let read = match hosted.plugin.try_lock() {
            Ok(p) => Some({
                // A built-in plugin's editor is in its card.
                let builtin = builtin::find(p.desc().format.as_str(), &p.desc().id).is_some();
                Info {
                    has_editor: p.has_editor() && window::AVAILABLE && !builtin,
                    editor_open: hosted.window.is_some(),
                    editor_failed: hosted.editor_failed.clone(),
                    latency: p.latency(),
                    stopped: p
                        .stopped()
                        .map(|c| os_host::guard::stopped_text(&hosted.name, c)),
                }
            }),
            Err(TryLockError::WouldBlock) => None,
            Err(TryLockError::Poisoned(_)) => {
                return Some(State::Missing("plugin lock poisoned".into()));
            }
        };
        if let Some(info) = read {
            hosted.card_info = Some(info);
        }
        let mut info = hosted.card_info.clone()?;
        info.editor_open = hosted.window.is_some();
        info.editor_failed.clone_from(&hosted.editor_failed);
        Some(State::Live(info))
    })
}

/// The plugin's text for param `param` (module index, ≥ 1) at `value`. A busy callback keeps
/// its last text only if the normalized parameter value is unchanged.
pub fn param_text(channel_id: i32, instance_id: i32, param: usize, value: f32) -> Option<String> {
    with(|host| {
        let hosted = host.live.get_mut(&(channel_id, instance_id))?;
        let id = *hosted.ids.get(param.checked_sub(1)?)?;
        match hosted.plugin.try_lock() {
            Ok(p) => {
                let text = p.param_text(id, f64::from(value));
                if let Some(ref text) = text {
                    hosted.card_text.insert(param, (value, text.clone()));
                }
                text
            }
            Err(TryLockError::WouldBlock) => {
                hosted.card_text.get(&param).and_then(|(cached, text)| {
                    (cached.to_bits() == value.to_bits()).then(|| text.clone())
                })
            }
            Err(TryLockError::Poisoned(_)) => None,
        }
    })
}

/// Opens the plugin's own editor in a window, or brings it to the front; closes it with `open` false.
pub fn show_editor(channel_id: i32, instance_id: i32, open: bool) -> Result<(), String> {
    with(|host| {
        let hosted = host
            .live
            .get_mut(&(channel_id, instance_id))
            .ok_or("the plugin is not running")?;
        if !open {
            close_editor_of(hosted);
            return Ok(());
        }
        if let Some(w) = hosted.window.as_mut() {
            w.show();
            return Ok(());
        }
        if let Some(why) = &hosted.editor_failed {
            return Err(why.clone());
        }
        let opened = open_window(hosted);
        if let Err(why) = &opened {
            hosted.editor_failed = Some(why.clone());
        }
        opened
    })
}

/// Opens the plugin's editor in a new window, or says why it could not.
fn open_window(hosted: &mut Hosted) -> Result<(), String> {
    let failed = |e: String| format!("The editor could not be opened: {e}");
    let mut w = EditorWindow::new(&hosted.name).map_err(failed)?;
    let plugin = &hosted.plugin;
    let (opened, x_error) = window::editor_call(|| {
        let mut p = plugin
            .lock()
            .map_err(|_| "the plugin stopped".to_string())?;
        p.open_editor(w.handle(), w.scale())
    });
    if let Some(error) = x_error {
        // Not trusted to draw: closed before its window goes.
        if opened.is_ok() {
            let _ = window::editor_call(|| plugin.lock().map(|mut p| p.close_editor()));
        }
        return Err(format!("The editor could not be opened (X error: {error})"));
    }
    let size = opened.map_err(failed)?;
    w.resize(size);
    w.show();
    hosted.window = Some(w);
    Ok(())
}

/// The editor of built-in plugin module `instance_id` on the channel with id `channel_id`, drawn into its rack card
/// (`os_plugkit::embed`) at `scale` device pixels per editor pixel: its id, opened now if it is not open, or opened
/// again when it was opened at a scale more than 2 % off (a resized window; the card stretches it meanwhile). `None`
/// while the plugin is not running.
pub fn lemon_editor(channel_id: i32, instance_id: i32, scale: f32) -> Option<u32> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    with(|host| {
        let hosted = host.live.get_mut(&(channel_id, instance_id))?;
        if let Some((id, at)) = hosted.embedded
            && (at / scale - 1.0).abs() <= 0.02
            && os_plugkit::embed::is_open(id as _)
        {
            return Some(id);
        }
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        os_plugkit::embed::set_scale(id as _, scale);
        let mut p = hosted.plugin.lock().ok()?;
        p.close_editor();
        // The handle is the editor's id ([`os_plugkit::embed::Parent`]).
        p.open_editor(id as usize as *mut std::ffi::c_void, scale)
            .ok()?;
        hosted.embedded = Some((id, scale));
        Some(id)
    })
}

/// While `f` runs, the engines built get fresh instances of their plugins (an export renders on its own, twice): each
/// with the live instance's state now, else the module's, at the build's rate, rendering offline.
pub fn rendering<T>(f: impl FnOnce() -> T) -> T {
    let was = with(|host| std::mem::replace(&mut host.rendering, true));
    let out = f();
    with(|host| host.rendering = was);
    out
}

/// The instance an engine build of `module` on `channel` plays: the live one, or a fresh one while [`rendering`].
fn engine_instance(
    channel: &Channel,
    module: &ModuleInstance,
    rate: u32,
) -> Option<(SharedPlugin, Arc<ParamShared>)> {
    with(|host| {
        let key = (channel.id, module.instance_id);
        if !host.rendering {
            let hosted = host.live.get(&key)?;
            return Some((hosted.plugin.clone(), hosted.params.clone()));
        }
        let plugin_ref = module.file.plugin.as_deref()?;
        let live_state = (host.live.get(&key)).and_then(|h| h.plugin.lock().ok()?.save_state());
        let state = live_state.as_deref().unwrap_or(&plugin_ref.state);
        let entry = host.find(plugin_ref)?;
        let plugin = start_instance(&entry.file, &entry.desc, Some(state), rate).ok()?;
        let (ids, _, values) = param_list(&plugin);
        let shared = Arc::new(ParamShared::new(&values));
        if let Ok(mut p) = plugin.lock() {
            p.set_offline(true);
            for (i, (&id, &v)) in ids.iter().zip(module.params.iter().skip(1)).enumerate() {
                if values.get(i) != Some(&v) {
                    p.set_param(id, f64::from(v));
                    shared.known(i, v);
                }
            }
        }
        Some((plugin, shared))
    })
}

/// The engine instrument for plugin module `module` on `channel`; `None` when its plugin is not running.
pub fn instrument(
    channel: &Channel,
    module: &ModuleInstance,
    rate: f32,
    bar_beats: u32,
    max_block: usize,
) -> Option<Box<dyn honeydew_audio::instrument::Instrument>> {
    let rate = rate as u32;
    let (plugin, params) = engine_instance(channel, module, rate)?;
    // Issue #33: a plugin plays the sustain pedal itself, but the built-in ones, which have none of their own (they
    // ignore CC #64): the engine holds their note-offs, as for its own instruments.
    let pedal = (module.file.plugin.as_deref()).is_none_or(|p| builtin::of(p).is_none());
    Some(Box::new(PluginInstrument::new(
        plugin,
        params,
        rate,
        bar_beats as u16,
        max_block,
        pedal,
    )))
}

/// The engine effect for plugin module `module` on `channel`; `None` when its plugin is not running.
pub fn effect(
    channel: &Channel,
    module: &ModuleInstance,
    rate: f32,
    bar_beats: u32,
) -> Option<Box<dyn honeydew_audio::ExternalEffect>> {
    let rate = rate as u32;
    let (plugin, params) = engine_instance(channel, module, rate)?;
    Some(Box::new(PluginEffect::new(
        plugin,
        params,
        rate,
        bar_beats as u16,
    )))
}

#[cfg(test)]
mod card_tests {
    use super::*;

    /// Issue #33: without input, the app runs the plugins' idle about 60 times a second while an editor waits for it
    /// (its window, or the descriptors the run loop watches for it), at the timer's own time while only a timer waits,
    /// and never while nothing does.
    #[cfg(unix)]
    #[test]
    fn plugin_idle_runs_only_as_often_as_something_waits_for_it() {
        use os_host::runloop;
        use std::os::fd::AsRawFd;
        use std::time::Duration;

        assert_eq!(idle_after(), None);
        let timer = runloop::add_timer(Duration::from_millis(500), || {});
        let after = idle_after().expect("a timer waits");
        let (low, high) = (Duration::from_millis(400), Duration::from_millis(500));
        assert!(after > low && after <= high, "{after:?}");
        let null = std::fs::File::open("/dev/null").unwrap();
        let watch = runloop::add_fd(null.as_raw_fd(), runloop::READ, |_, _| {});
        let after = idle_after().expect("an editor's descriptor waits");
        assert!(after <= Duration::from_millis(20), "{after:?}");
        for token in [timer, watch] {
            runloop::remove(token);
        }
        assert_eq!(idle_after(), None);
    }

    #[test]
    fn busy_plugin_lock_keeps_the_last_card_status_and_value_text() {
        let channel_id = 3901;
        let instance_id = 7;
        let module = instantiate(channel_id, instance_id, builtin::LEMON_TRIO.uid).unwrap();
        let status = state(channel_id, instance_id).expect("plugin is running");
        let (param, value, label) = (1..module.params.len())
            .find_map(|index| {
                let value = module.params[index];
                param_text(channel_id, instance_id, index, value).map(|label| (index, value, label))
            })
            .expect("plugin provides a parameter label");
        let plugin = with(|host| host.live[&(channel_id, instance_id)].plugin.clone());
        let guard = plugin.lock().unwrap();
        assert_eq!(state(channel_id, instance_id), Some(status));
        assert_eq!(
            param_text(channel_id, instance_id, param, value),
            Some(label)
        );
        assert_eq!(
            param_text(channel_id, instance_id, param, (value + 0.1).min(1.0)),
            None
        );
        drop(guard);
        with(|host| {
            host.live.remove(&(channel_id, instance_id));
        });
    }
}

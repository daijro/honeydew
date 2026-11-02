//! The web's bundled content: the packs' parts download as picks and songs need them,
//! stay in Cache Storage, and [`super::Bundle::read`] slices members out of those in memory.
//!
//! - **What.** [`ensure`] claims the parts of the entries a pick or a song needs and downloads the missing ones,
//!   [`PARALLEL`] at a time: first the parts each one waits for (part 0 of an entry with a lite program, which holds it
//!   and its zones; every part of any other), then the rest. Its [`Ticket`] holds the claim and tells the progress.
//!   Dropping it stops the downloads only it needed and frees what they brought; [`Ticket::keep`] hands the claim to
//!   The song instead, until the next song load ([`song`]).
//! - **From where** (`web.js`). Cache Storage (`honeydew-packs-v1`), else the `.gz` beside the part, gunzipped unless
//!   the host sent it with `Content-Encoding` (index.html's rule for the wasm), its tar's SHA-256 checked before it is
//!   cached; without a `.gz` the plain part, which the browser checks against `integrity`. A body stays a JS
//!   `Uint8Array`, outside the wasm's memory, and a read copies one member out ([`slice`]).
//! - **Failures**. A part that fails is tried again from the network after 1 s, 3 s and 9 s (±20%), then given up: its
//!   members read as missing. Offline and not cached, it gives up at once. The next claim tries again.
//! - **LRU**. The book (`localStorage` `honeydew.packs`) holds each cached part's last landing and size. After each
//!   landing the least recent parts no claim holds leave Cache Storage, until the rest fit the budget: half the
//!   origin's quota, at most 1 GiB.
//! - **Upgrades**. Until all of an entry's parts are in, its program reads as its lite one; [`poll`] names it once they
//!   are, and the app loads it in full. A small device (4 GB or less, or unknown) keeps a program over 128 MB decoded
//!   at its lite one.
//!
//! The I/O is [`Io`]'s fn pointers: the browser's on the web ([`start`]), a fake in tests. The desktop has none: its
//! packs are files, and nothing here runs.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui;

use super::bundle::{Bundle, IndexEntry, Part as IndexPart};
use crate::project_files::BUILT_IN;
use crate::project_files::content::built_in;
use crate::sample_cache::SampleKey;

/// Parts downloading at once.
const PARALLEL: usize = 4;
/// The waits before the second, third and fourth try, in ms (±20%); then it gives up.
const RETRIES_MS: [f64; 3] = [1000.0, 3000.0, 9000.0];
/// A program over this many bytes decoded stays at its lite one on a small device.
const LITE_ONLY_DECODED: u64 = 128 << 20;
/// Parts in memory past this many bytes: the song's claim shrinks to what the song plays ([`trim`]).
const MEMORY_CAP: u64 = 256 << 20;

/// A pick or song whose bundled file is not in memory: its download failed (or gave up).
pub const DOWNLOAD_FAILED: &str = "Download failed, please check your internet connection and amount of free memory space and try again.";

/// A part's body: a JS `Uint8Array` on the web.
#[cfg(target_arch = "wasm32")]
pub type Body = web_sys::js_sys::Uint8Array;
#[cfg(not(target_arch = "wasm32"))]
pub type Body = Vec<u8>;

/// A download to start.
pub struct Request<'a> {
    /// The part, relative to `content/`.
    pub file: &'a str,
    /// Its tar's SHA-256, base64.
    pub sha256: &'a str,
    /// A retry: from the network, past the HTTP cache.
    pub reload: bool,
    /// Its number, which its outcome comes back with ([`landed`]).
    pub n: u32,
}

/// How a download ended.
pub enum Outcome {
    /// In, and in Cache Storage.
    Body(Body),
    /// In memory only: Cache Storage refused it (over the quota) or there is none. The LRU book leaves it out.
    Uncached(Body),
    Failed,
    /// The browser is offline, and the part is not in Cache Storage.
    Offline,
}

/// The loader's I/O.
#[derive(Clone, Copy)]
pub struct Io {
    /// Starts a download; its outcome comes to [`landed`].
    pub fetch: fn(&Request),
    /// Stops a part's download.
    pub abort: fn(&str),
    /// Drops a part from Cache Storage.
    pub evict: fn(&str),
    /// Writes the LRU book.
    pub save: fn(&str),
    /// Starts downloading a text file beside the index; it comes to [`noticed`].
    pub text: fn(&str),
    /// The time in ms since the epoch.
    pub now: fn() -> f64,
}

/// What a ticket waits for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// The parts it waits for, those in memory and those given up.
    pub total: usize,
    pub landed: usize,
    pub failed: usize,
    /// One failed and is to be tried again.
    pub retrying: bool,
}

impl Progress {
    /// Nothing more comes: each part it waits for is in, or given up.
    pub fn done(&self) -> bool {
        self.landed + self.failed >= self.total
    }
}

/// A claim [`ensure`] made: its parts stay in memory and download while it lives.
pub struct Ticket(u64);

impl Ticket {
    pub fn progress(&self) -> Progress {
        LOADER.with_borrow(|l| l.progress(self.0))
    }

    /// Hands the claim to the song: what it brought stays and what is left goes on downloading, until the next song
    /// load.
    pub fn keep(self) {
        let id = std::mem::ManuallyDrop::new(self).0;
        LOADER.with_borrow_mut(|l| l.keep(id));
    }
}

impl Drop for Ticket {
    /// The downloads only it needed stop, and what they brought leaves memory (Cache Storage keeps it).
    fn drop(&mut self) {
        let _ = LOADER.try_with(|l| l.borrow_mut().release(self.0));
    }
}

#[derive(Default)]
struct Loader {
    io: Option<Io>,
    /// The cache's budget in bytes.
    budget: u64,
    /// Programs over [`LITE_ONLY_DECODED`] stop at their lite one.
    small: bool,
    /// The parts some claim holds, by file.
    parts: HashMap<String, Part>,
    /// Each live ticket's needs, by its id; 0 holds the song's.
    claims: BTreeMap<u64, Vec<Need>>,
    tickets: u64,
    downloads: u32,
    /// Each cached part's last landing (ms since the epoch) and size, by file.
    book: HashMap<String, (f64, u64)>,
    /// The entries whose lite program has its full one now, by id, for [`poll`].
    upgraded: Vec<String>,
    /// Repaints when a download lands.
    ctx: Option<egui::Context>,
    /// `CREDITS.txt`: asked for, and once in, its text (`None` inside: it failed).
    notices: Option<Option<Option<String>>>,
    /// The bytes in memory after the last [`trim`].
    trimmed: u64,
}

#[derive(Default)]
struct Part {
    body: Option<Body>,
    bytes: u64,
    /// The number of the download under way.
    download: Option<u32>,
    fails: u32,
    /// When the next try may start.
    retry_at: Option<f64>,
    gave_up: bool,
}

/// An entry as a claim needs it.
struct Need {
    id: String,
    /// Its path below `Built-in/`.
    path: String,
    parts: Vec<IndexPart>,
    /// How many of the first parts it waits for.
    wait: usize,
    /// It has a lite program, in part 0.
    lite: bool,
}

impl Need {
    fn of(entry: &IndexEntry, small: bool) -> Self {
        let lite = entry.lite.is_some() && entry.parts.len() > 1;
        let mut parts = entry.parts.clone();
        if lite && small && entry.decoded > LITE_ONLY_DECODED {
            parts.truncate(1);
        }
        Self {
            id: entry.id.clone(),
            path: entry.path.clone(),
            wait: if lite { 1 } else { parts.len() },
            lite,
            parts,
        }
    }
}

thread_local! {
    static LOADER: RefCell<Loader> = RefCell::default();
}

impl Loader {
    /// Claims `needs` for a new ticket, its id: what they need starts downloading. A part given up is tried again.
    fn claim(&mut self, needs: Vec<Need>) -> u64 {
        for part in needs.iter().flat_map(|n| &n.parts) {
            let state = self.parts.entry(part.file.clone()).or_default();
            if state.gave_up {
                *state = Part::default();
            }
            state.bytes = part.bytes;
        }
        self.tickets += 1;
        self.claims.insert(self.tickets, needs);
        self.pump();
        self.tickets
    }

    fn progress(&self, id: u64) -> Progress {
        let waits = self.claims.get(&id).into_iter().flatten();
        let files: HashSet<&str> = waits
            .flat_map(|n| &n.parts[..n.wait])
            .map(|p| p.file.as_str())
            .collect();
        let mut progress = Progress {
            total: files.len(),
            ..Progress::default()
        };
        for part in files.iter().filter_map(|file| self.parts.get(*file)) {
            progress.landed += usize::from(part.body.is_some());
            progress.failed += usize::from(part.gave_up);
            progress.retrying |= part.fails > 0 && !part.gave_up && part.body.is_none();
        }
        progress
    }

    /// The claim `id` becomes the song's, but for the entries the song holds already.
    fn keep(&mut self, id: u64) {
        let mut needs = self.claims.remove(&id).unwrap_or_default();
        let kept = self.claims.entry(0).or_default();
        needs.retain(|n| !kept.iter().any(|k| k.path == n.path));
        kept.extend(needs);
    }

    /// Drops the claim `id`: the parts no other claim holds stop downloading and leave memory.
    fn release(&mut self, id: u64) {
        self.claims.remove(&id);
        self.prune();
    }

    /// The parts no claim holds stop downloading and leave memory.
    fn prune(&mut self) {
        let claimed: HashSet<&str> = (self.claims.values().flatten())
            .flat_map(|n| &n.parts)
            .map(|p| p.file.as_str())
            .collect();
        let io = self.io;
        self.parts.retain(|file, part| {
            let keep = claimed.contains(file.as_str());
            if let (false, Some(_), Some(io)) = (keep, part.download, io) {
                (io.abort)(file);
            }
            keep
        });
        self.pump();
    }

    /// Starts the downloads due while fewer than [`PARALLEL`] run: the parts the claims wait for first, then the rest,
    /// each in claim and part order.
    fn pump(&mut self) {
        let Some(io) = self.io else { return };
        let now = (io.now)();
        let mut running = self.parts.values().filter(|p| p.download.is_some()).count();
        let needs: Vec<&Need> = self.claims.values().flatten().collect();
        let first = needs.iter().flat_map(|n| &n.parts[..n.wait]);
        for part in first.chain(needs.iter().flat_map(|n| &n.parts[n.wait..])) {
            if running >= PARALLEL {
                break;
            }
            let Some(state) = self.parts.get_mut(&part.file) else {
                continue;
            };
            let due = state.retry_at.is_none_or(|at| at <= now);
            if state.body.is_some() || state.download.is_some() || state.gave_up || !due {
                continue;
            }
            self.downloads += 1;
            (state.download, state.retry_at) = (Some(self.downloads), None);
            running += 1;
            (io.fetch)(&Request {
                file: &part.file,
                sha256: &part.sha256,
                reload: state.fails > 0,
                n: self.downloads,
            });
        }
    }

    fn landed(&mut self, file: &str, n: u32, outcome: Outcome) {
        let Some(io) = self.io else { return };
        // ponytail: a part released just as it landed may be in Cache Storage unbooked, where the LRU never evicts it;
        // book it here if such strays ever add up.
        let Some(part) = self.parts.get_mut(file).filter(|p| p.download == Some(n)) else {
            return;
        };
        part.download = None;
        let now = (io.now)();
        let cached = matches!(outcome, Outcome::Body(_));
        match outcome {
            Outcome::Body(body) | Outcome::Uncached(body) => {
                part.body = Some(body);
                if cached {
                    self.book.insert(file.to_owned(), (now, part.bytes));
                }
                self.upgrades(file);
                self.evict();
            }
            Outcome::Offline => part.gave_up = true,
            Outcome::Failed => {
                part.fails += 1;
                match retry_after(part.fails, jitter(file)) {
                    Some(wait) => part.retry_at = Some(now + wait),
                    None => part.gave_up = true,
                }
            }
        }
        self.pump();
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
    }

    /// Notes the lite programs whose last part `file` is.
    fn upgrades(&mut self, file: &str) {
        for need in self.claims.values().flatten() {
            let landed = |p: &IndexPart| self.parts.get(&p.file).is_some_and(|s| s.body.is_some());
            let last = need.parts.iter().any(|p| p.file == file) && need.parts.iter().all(landed);
            if need.lite && need.parts.len() > 1 && last && !self.upgraded.contains(&need.id) {
                self.upgraded.push(need.id.clone());
            }
        }
    }

    /// The bytes of the parts in memory.
    fn resident(&self) -> u64 {
        (self.parts.values())
            .filter(|p| p.body.is_some())
            .map(|p| p.bytes)
            .sum()
    }

    /// The least recent parts no claim holds leave Cache Storage until the book fits the budget, then it is saved.
    fn evict(&mut self) {
        let Some(io) = self.io else { return };
        let mut total: u64 = self.book.values().map(|&(_, bytes)| bytes).sum();
        let mut old: Vec<(f64, String)> = (self.book.iter())
            .filter(|(file, _)| !self.parts.contains_key(*file))
            .map(|(file, &(used, _))| (used, file.clone()))
            .collect();
        old.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, file) in old {
            if total <= self.budget {
                break;
            }
            total -= self.book.remove(&file).map_or(0, |(_, bytes)| bytes);
            (io.evict)(&file);
        }
        (io.save)(&serde_json::to_string(&self.book).unwrap_or_default());
    }
}

/// The wait before the next try of a part that failed `fails` times, in ms, ±20% by `jitter` (0 to 1); `None` once it
/// failed after the last retry: it gives up.
fn retry_after(fails: u32, jitter: f64) -> Option<f64> {
    let wait = RETRIES_MS.get(fails.checked_sub(1)? as usize)?;
    Some(wait * (0.8 + 0.4 * jitter))
}

/// `file`'s retry jitter, 0 to 1: parts failing together try again apart.
// ponytail: from the name (FNV-1a), the same in every session; draw it at random if clients ever retry in step.
fn jitter(file: &str) -> f64 {
    let hash = (file.bytes()).fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    (hash % 1001) as f64 / 1000.0
}

/// Installs the loader: its I/O, the cache's budget in bytes, whether the device is small, and the LRU book as saved.
pub fn install(io: Io, budget: u64, small: bool, book: &str) {
    LOADER.set(Loader {
        io: Some(io),
        budget,
        small,
        book: serde_json::from_str(book).unwrap_or_default(),
        ..Loader::default()
    });
}

/// Claims the parts `entries` of `bundle` need, downloading what is not in memory: a ticket while what they wait for
/// is not all in; else `None`, and the claim is the song's. Always `None` with packs on disk, or with no loader.
pub fn ensure<'a>(
    bundle: &Bundle,
    entries: impl IntoIterator<Item = &'a IndexEntry>,
) -> Option<Ticket> {
    claim(bundle, entries, false)
}

/// [`ensure`]; for a new `song`, the old song's claim goes, but for what this one needs too.
fn claim<'a>(
    bundle: &Bundle,
    entries: impl IntoIterator<Item = &'a IndexEntry>,
    song: bool,
) -> Option<Ticket> {
    if bundle.dir.is_some() {
        return None;
    }
    LOADER.with_borrow_mut(|l| {
        l.io?;
        let needs = entries.into_iter().map(|e| Need::of(e, l.small)).collect();
        let id = l.claim(needs);
        if song {
            l.release(0);
        }
        let progress = l.progress(id);
        if progress.landed < progress.total {
            return Some(Ticket(id));
        }
        l.keep(id);
        None
    })
}

/// [`ensure`] for a pick of `path`, below [`super::ROOT`], in the app's factory.
pub fn pick(path: &Path) -> Option<Ticket> {
    LOADER.with_borrow(|l| l.io)?;
    let bundle = super::factory().bundle.as_deref()?;
    ensure(bundle, [bundle.entry_of(&super::below_root(path)?)?])
}

/// `path`, below [`super::ROOT`], is a bundled file of `factory`'s on the web: when it does not read, its download
/// failed ([`DOWNLOAD_FAILED`]).
pub fn lost(factory: &super::Factory, path: &Path) -> bool {
    let bundle = factory.bundle.as_deref().filter(|b| b.dir.is_none());
    let rel = super::below_root(path);
    bundle
        .zip(rel)
        .is_some_and(|(bundle, rel)| bundle.entry_of(&rel).is_some())
}

/// A pick in `slot` that waits for its download ([`pick`]): the path to pick now. That is `new` when it reads at once,
/// or the one waiting once its download is done (given up too: its read then fails with [`DOWNLOAD_FAILED`]). `new`
/// replaces the one waiting, whose download stops.
pub fn picked(slot: &mut Option<(PathBuf, Ticket)>, new: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(new) = new {
        let ticket = pick(&new);
        *slot = None;
        let Some(ticket) = ticket else {
            return Some(new);
        };
        *slot = Some((new, ticket));
        return None;
    }
    if !slot.as_ref()?.1.progress().done() {
        return None;
    }
    let (path, ticket) = slot.take()?;
    ticket.keep();
    Some(path)
}

/// [`ensure`] for a new song that plays `keys` ([`crate::sample_cache::SampleCache::referenced`]); the old song's
/// claim goes.
pub fn song(bundle: &Bundle, keys: &[SampleKey]) -> Option<Ticket> {
    claim(bundle, needed(bundle, keys), true)
}

/// The entries of `bundle` that `keys` read from, each once: our own `@R@Built-in/…` file's (a DW program's too), and a
/// built-in one-shot's `.flac`, aliases applied. Code-made sounds and other files have none.
pub fn needed<'a>(bundle: &'a Bundle, keys: &[SampleKey]) -> Vec<&'a IndexEntry> {
    let mut entries: Vec<&IndexEntry> = Vec::new();
    for key in keys {
        let rel = match key {
            SampleKey::File(path) => built_in(path).map(|rel| rel.replace('\\', "/")),
            // ponytail: bundled audio is always `.flac`, as `Refs::add` reads it.
            SampleKey::Builtin(stem) => Some(format!("{stem}.flac")),
            SampleKey::Embedded(..) => None,
        };
        let rel = rel
            .as_deref()
            .and_then(|rel| rel.strip_prefix(BUILT_IN)?.strip_prefix('/'));
        if let Some(entry) = rel.and_then(|rel| bundle.entry_of(rel))
            && !entries.iter().any(|e| e.id == entry.id)
        {
            entries.push(entry);
        }
    }
    entries
}

/// Each frame: the retries due start, and egui repaints when one is due (a landing repaints too). The entries whose
/// lite program has its full one now, by id.
pub fn poll(ctx: &egui::Context) -> Vec<String> {
    LOADER.with_borrow_mut(|l| {
        let Some(io) = l.io else { return Vec::new() };
        l.ctx.get_or_insert_with(|| ctx.clone());
        l.pump();
        let next = l
            .parts
            .values()
            .filter_map(|p| p.retry_at)
            .min_by(f64::total_cmp);
        if let Some(at) = next {
            ctx.request_repaint_after_secs(((at - (io.now)()) / 1000.0).max(0.0) as f32);
        }
        std::mem::take(&mut l.upgraded)
    })
}

/// Once the parts in memory pass [`MEMORY_CAP`] (since the last trim), the song's claim shrinks to the entries `song`
/// plays now: those picked and replaced since the song loaded leave memory.
pub fn trim(bundle: &Bundle, song: &crate::song::Song) {
    if !LOADER.with_borrow(|l| l.resident() > l.trimmed.max(MEMORY_CAP)) {
        return;
    }
    let keys = crate::sample_cache::SampleCache::referenced(song);
    let plays: HashSet<&str> = needed(bundle, &keys)
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    LOADER.with_borrow_mut(|l| {
        if let Some(kept) = l.claims.get_mut(&0) {
            kept.retain(|n| plays.contains(n.id.as_str()));
        }
        l.prune();
        l.trimmed = l.resident();
    });
}

/// The bundled content's full notices, `content/CREDITS.txt`, on the web: downloaded once, when first asked for.
/// `None` while it is on its way; then its text, `None` inside when it failed or there is no loader.
pub fn notices() -> Option<Option<String>> {
    LOADER.with_borrow_mut(|l| {
        let Some(io) = l.io else { return Some(None) };
        if l.notices.is_none() {
            l.notices = Some(None);
            (io.text)("CREDITS.txt");
        }
        l.notices.clone()?
    })
}

/// `CREDITS.txt` came in (`None`: it failed).
pub fn noticed(text: Option<String>) {
    LOADER.with_borrow_mut(|l| {
        l.notices = Some(Some(text));
        if let Some(ctx) = &l.ctx {
            ctx.request_repaint();
        }
    });
}

/// The outcome of `file`'s download numbered `n` ([`Request::n`]); one stopped or replaced since is dropped.
pub fn landed(file: &str, n: u32, outcome: Outcome) {
    LOADER.with_borrow_mut(|l| l.landed(file, n, outcome));
}

/// `len` bytes from `at` in the part `file`, when it is in memory: one member.
pub fn slice(file: &str, at: u64, len: u64) -> Option<Vec<u8>> {
    LOADER.with_borrow(|l| cut(l.parts.get(file)?.body.as_ref()?, at, at.checked_add(len)?))
}

/// Every part of `entry` is in memory.
pub fn all_in(entry: &IndexEntry) -> bool {
    LOADER.with_borrow(|l| {
        (entry.parts.iter()).all(|p| l.parts.get(&p.file).is_some_and(|s| s.body.is_some()))
    })
}

#[cfg(target_arch = "wasm32")]
fn cut(body: &Body, at: u64, end: u64) -> Option<Vec<u8>> {
    (end <= u64::from(body.length())).then(|| body.subarray(at as u32, end as u32).to_vec())
}

#[cfg(not(target_arch = "wasm32"))]
fn cut(body: &Body, at: u64, end: u64) -> Option<Vec<u8>> {
    let range = usize::try_from(at).ok()?..usize::try_from(end).ok()?;
    body.get(range).map(<[u8]>::to_vec)
}

#[cfg(target_arch = "wasm32")]
pub use browser::start;

/// The browser's side: `web.js`.
#[cfg(target_arch = "wasm32")]
mod browser {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::prelude::wasm_bindgen;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::js_sys::{Array, Date, Promise, Uint8Array};

    use super::{Io, Outcome, Request};
    use crate::content::bundle::{Index, install};

    #[wasm_bindgen(module = "/src/content/web.js")]
    extern "C" {
        fn index(url: &str) -> Promise;
        fn quota() -> Promise;
        fn small() -> bool;
        fn book() -> String;
        fn save(json: &str);
        fn evict(url: &str);
        fn abort(url: &str);
        fn part(url: &str, sha256: &str, reload: bool) -> Promise;
        fn notices(url: &str) -> Promise;
    }

    /// The content folder, beside the page (`index.html` preloads its index).
    const CONTENT: &str = "content/";
    /// The cache's budget's cap.
    const MAX_BUDGET: f64 = (1u64 << 30) as f64;

    fn url(file: &str) -> String {
        format!("{CONTENT}{file}")
    }

    /// Before the app starts (`lib.rs` `run_web`): the content index (from the network for 5 s at most, else Cache
    /// Storage's copy), installed for [`crate::content::factory`], and the loader with the browser's I/O. Without an
    /// index the app runs without our bundled content.
    pub async fn start() {
        let bytes = JsFuture::from(index(&url("index.json"))).await.ok();
        let bytes = bytes
            .filter(|b| !b.is_null())
            .map(|b| Uint8Array::new(&b).to_vec());
        match bytes.as_deref().and_then(Index::parse) {
            Some(parsed) => install(parsed),
            None => web_sys::console::warn_1(&"No content index: no bundled sounds.".into()),
        }
        let quota = JsFuture::from(quota()).await.ok().and_then(|q| q.as_f64());
        let budget = quota
            .filter(|&q| q > 0.0)
            .map_or(MAX_BUDGET, |q| (q / 2.0).min(MAX_BUDGET));
        let io = Io {
            fetch,
            abort: |file| abort(&url(file)),
            evict: |file| evict(&url(file)),
            save,
            now: Date::now,
            text,
        };
        super::install(io, budget as u64, small(), &book());
    }

    fn fetch(request: &Request) {
        let (file, n) = (request.file.to_owned(), request.n);
        let promise = part(&url(&file), request.sha256, request.reload);
        wasm_bindgen_futures::spawn_local(async move {
            let outcome = match JsFuture::from(promise).await {
                // `[body, cached]`.
                Ok(landed) => {
                    let landed: Array = landed.unchecked_into();
                    let body = landed.get(0).unchecked_into();
                    match landed.get(1).is_truthy() {
                        true => Outcome::Body(body),
                        false => Outcome::Uncached(body),
                    }
                }
                Err(err) => match err.as_string().as_deref() {
                    Some("offline") => Outcome::Offline,
                    Some("aborted") => Outcome::Failed,
                    _ => {
                        web_sys::console::warn_2(&format!("{file}:").into(), &err);
                        Outcome::Failed
                    }
                },
            };
            super::landed(&file, n, outcome);
        });
    }

    fn text(file: &str) {
        let promise = notices(&url(file));
        wasm_bindgen_futures::spawn_local(async move {
            super::noticed(
                JsFuture::from(promise)
                    .await
                    .ok()
                    .and_then(|t| t.as_string()),
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::Cell;
    use std::collections::BTreeMap;

    use super::*;
    use crate::content::bundle::{Index, Kind, Member, SCHEMA};
    use crate::flm::chunk::{ContentPath, ContentRoot};
    use crate::sample_cache::SampleCache;
    use crate::sample_cache::tests::dw_cstm;
    use crate::song::{ChannelKind, Song, SynthKind};

    thread_local! {
        static FETCHED: RefCell<Vec<(String, u32, bool)>> = const { RefCell::new(Vec::new()) };
        static ABORTED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        static EVICTED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        static NOW: Cell<f64> = const { Cell::new(0.0) };
    }

    /// Installs this thread's loader with a fake I/O that logs, at time 0, with `budget` and the LRU `book`.
    pub(crate) fn fake(budget: u64, book: &str) {
        FETCHED.set(Vec::new());
        ABORTED.set(Vec::new());
        EVICTED.set(Vec::new());
        NOW.set(0.0);
        let io = Io {
            fetch: |r| FETCHED.with_borrow_mut(|f| f.push((r.file.to_owned(), r.n, r.reload))),
            abort: |file| ABORTED.with_borrow_mut(|a| a.push(file.to_owned())),
            evict: |file| EVICTED.with_borrow_mut(|e| e.push(file.to_owned())),
            save: |_| {},
            now: || NOW.get(),
            text: |_| {},
        };
        install(io, budget, false, book);
    }

    /// The downloads started since the last call: part, number, and whether from the network.
    pub(crate) fn fetched() -> Vec<(String, u32, bool)> {
        FETCHED.take()
    }

    pub(crate) fn advance(ms: f64) {
        NOW.set(NOW.get() + ms);
    }

    /// An entry of `kind` at `path`, with the lite program `lite`, whose `members` (name, part, bytes) lie back to back
    /// in their parts, `p/<id><part>.tar` (its id is its name's, `test-piano`); and its parts' bytes.
    pub(crate) fn pack(
        kind: Kind,
        path: &str,
        lite: Option<&str>,
        members: &[(&str, usize, Vec<u8>)],
    ) -> (IndexEntry, Vec<Vec<u8>>) {
        let name = path.trim_end_matches('/').rsplit('/').next().unwrap();
        let id = name
            .split('.')
            .next()
            .unwrap()
            .to_lowercase()
            .replace(' ', "-");
        let mut bodies = vec![Vec::new(); members.iter().map(|m| m.1 + 1).max().unwrap_or(1)];
        let members = (members.iter())
            .map(|(name, part, bytes)| {
                let at = bodies[*part].len() as u64;
                bodies[*part].extend_from_slice(bytes);
                Member(name.to_string(), *part, at, bytes.len() as u64)
            })
            .collect();
        let parts = (bodies.iter().enumerate())
            .map(|(n, body)| IndexPart {
                file: format!("p/{id}{n}.tar"),
                bytes: body.len() as u64,
                sha256: String::new(),
            })
            .collect();
        let entry = IndexEntry {
            id,
            kind,
            path: path.into(),
            license: "CC0-1.0".into(),
            credits: Vec::new(),
            decoded: 0,
            lite: lite.map(Into::into),
            parts,
            members,
        };
        (entry, bodies)
    }

    /// The web's bundle of `entries`: no content dir.
    pub(crate) fn bundle(entries: Vec<IndexEntry>) -> Bundle {
        let index = Index {
            schema: SCHEMA,
            release: "test".into(),
            aliases: BTreeMap::new(),
            credits: BTreeMap::new(),
            entries,
        };
        Bundle::new(index, None)
    }

    #[test]
    fn a_song_needs_the_entries_it_reads_from_aliases_included() {
        let program = "Presets/DW Sampler/Keys/Test Piano.dwp";
        let (piano, _) = pack(
            Kind::Program,
            program,
            None,
            &[("Test Piano.dwp", 0, vec![])],
        );
        let (kicks, _) = pack(
            Kind::Samples,
            "Samples/Kick/",
            None,
            &[("K1.flac", 0, vec![])],
        );
        let (kit, _) = pack(
            Kind::Kit,
            "Drumsets/Rock.tmpl",
            None,
            &[("Rock.tmpl", 0, vec![])],
        );
        let mut bundle = bundle(vec![piano, kicks, kit]);
        let old = "Presets/DW Sampler/Keys/Old Piano.dwp";
        bundle.index.aliases.insert(old.into(), program.into());
        // A DW Sampler on the piano's old path; pads on a bundled one-shot (by either separator), a user's file and
        // the code-made one-shots.
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::DwSampler));
        let cstm = dw_cstm(format!("@R@\0Built-in/{old}").as_bytes());
        song.channels[1].rack.modules[0].file.custom_data = Some(cstm);
        song.add_channel(ChannelKind::Drums);
        let slots = &mut song.channels[2].drum_kit.as_mut().unwrap().slots;
        let path = |root, relative: &str| {
            let relative = relative.into();
            Some(ContentPath::Shorthand { root, relative })
        };
        slots[0].file.path = path(ContentRoot::App, "Built-in/Samples/Kick/K1.flac");
        slots[1].file.path = path(ContentRoot::App, "Built-in\\Samples\\Kick\\K1.flac");
        slots[2].file.path = path(ContentRoot::User, "My Samples/Mine.wav");
        let keys = SampleCache::referenced(&song);
        let mut ids: Vec<&str> = needed(&bundle, &keys)
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, ["kick", "test-piano"]);
    }

    #[test]
    fn the_cache_keeps_the_claimed_parts_and_evicts_the_least_recent_others_to_the_budget() {
        // From earlier sessions: the piano's part 1, claimed now, the oldest; then a, b and c.
        let book = r#"{"p/piano1.tar": [1, 40], "p/a.tar": [2, 40], "p/b.tar": [3, 40], "p/c.tar": [4, 40]}"#;
        fake(120, book);
        let members = [
            ("Piano.dwp", 0, vec![0; 40]),
            ("Piano/C4.flac", 1, vec![0; 40]),
        ];
        let (piano, bodies) = pack(
            Kind::Program,
            "Presets/DW Sampler/Keys/Piano.dwp",
            None,
            &members,
        );
        let bundle = bundle(vec![piano]);
        let _ticket = ensure(&bundle, &bundle.index.entries);
        advance(10.0);
        let (file, n, _) = fetched().remove(0);
        assert_eq!(file, "p/piano0.tar");
        landed(&file, n, Outcome::Body(bodies[0].clone()));
        // 200 bytes for 120: a and b go, the least recent first; then the rest fits.
        assert_eq!(EVICTED.take(), ["p/a.tar", "p/b.tar"]);
    }

    #[test]
    fn a_replaced_pick_stops_its_download_and_drops_what_it_brings() {
        fake(u64::MAX, "");
        let (a, bodies) = pack(
            Kind::Kit,
            "Drumsets/A.tmpl",
            None,
            &[("A.tmpl", 0, b"A".to_vec())],
        );
        let (b, _) = pack(
            Kind::Kit,
            "Drumsets/B.tmpl",
            None,
            &[("B.tmpl", 0, b"B".to_vec())],
        );
        let bundle = bundle(vec![a, b]);
        let [first, second] = &bundle.index.entries[..] else {
            unreachable!()
        };
        let mut pick = ensure(&bundle, [first]);
        assert!(pick.is_some());
        let log = fetched();
        let [(file, n, _)] = &log[..] else {
            panic!("one download: {log:?}")
        };
        pick = ensure(&bundle, [second]);
        assert_eq!(ABORTED.take(), [file.as_str()]);
        // A's download ends anyway: dropped.
        landed(file, *n, Outcome::Body(bodies[0].clone()));
        assert_eq!(bundle.read("Drumsets/A.tmpl"), None);
        assert_eq!(fetched().len(), 1, "B's download");
        assert!(pick.is_some_and(|t| !t.progress().done()));
    }
}

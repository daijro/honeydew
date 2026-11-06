//! In-app updates (#66), natively: the web is always current. Main menu UPDATES (`ui::main_menu::updates`) shows
//! [`status`] and starts what follows.
//!
//! - **The check** ([`check`]) asks GitHub's API for [`REPO`]'s latest release, unauthenticated: at start while
//!   `Check for updates automatically` is on, at most once per [`CHECK_EVERY`], and on `Check now`, at most once a
//!   minute. The answer is kept in the app data folder ([`CACHE`]), so the starts in between show it without asking
//!   GitHub, which allows 60 requests an hour from an address.
//! - **The update** ([`install`]) is this build's download of that release ([`pick`]): Android's base APK, an AppImage,
//!   or the desktop archive without sounds (the sound library is in the app data folder and stays). Android hands the
//!   APK to the system's installer (`HoneydewActivity.installApk`, which first opens the setting that allows it when
//!   the user has not yet). An AppImage replaces itself: renaming over a running file is safe on Linux, and the next
//!   start is the new version. Any other download goes to the Downloads folder, shown in the file manager: replacing a
//!   running app's own files is not safe everywhere. Before the base APK replaces a full one, the full one's sounds are
//!   copied into the sound library's folder ([`keep_sounds`]), so that they are not downloaded again.
//! - **The sound library** ([`download_sounds`]) has its own versions and releases (`sounds-<version>`, [`sounds_url`]).
//!   A check also finds the newest one this build reads ([`newest_sounds`]): at least the one it was made with
//!   (`sounds/release.txt`), which a start fetches by itself where needed; a newer one UPDATES offers. Only the parts
//!   the installed one lacks are fetched
//!   (`content::install::download`), each checked against its SHA-256. On Android it waits for Wi-Fi (an unmetered network) unless `Download sounds over
//!   mobile data` is on ([`allow_mobile_data`]).
//! - **Transfers** ([`get`]) run on worker threads, never the UI's: `curl` on the desktop (part of Windows 10 and later,
//!   macOS and Linux distributions), Android's own `HttpURLConnection` there. No HTTP or TLS crate.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::content::install::{self, Progress, SHIPPED_RELEASE};

/// This build's GitHub repository, whose releases are its updates: the fork's own (upstream's APKs are another app, with
/// another signature, and lack the fork's changes).
pub const REPO: &str = "RememberedA/DqAqW";
/// Its latest release, in GitHub's API.
const LATEST: &str = "https://api.github.com/repos/RememberedA/DqAqW/releases/latest";
/// The repository whose releases hold the sound library (`sounds-<version>`): upstream's, which the fork shares.
pub const SOUNDS_REPO: &str = "daijro/honeydew";
/// Its sound library releases' tags (`sounds-<version>`), each made when its release is published.
const SOUNDS_TAGS: &str =
    "https://api.github.com/repos/daijro/honeydew/git/matching-refs/tags/sounds-";
/// An automatic check asks GitHub at most this often, in seconds; `Check now` at most once a [`MANUAL_EVERY`].
const CHECK_EVERY: u64 = 6 * 3600;
const MANUAL_EVERY: u64 = 60;
/// The last answer, in the app data folder.
pub const CACHE: &str = "update.json";
/// What a transfer without a network fails with.
pub const OFFLINE: &str = "No internet connection.";
/// The waits before the second, third and fourth try of a transfer, in seconds.
const RETRIES: [u64; 3] = [1, 3, 9];

/// A release, as GitHub's API gives it: the fields used.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub tag_name: String,
    /// Its notes (Markdown).
    #[serde(default)]
    pub body: String,
    /// Its page.
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// One of a release's downloads.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

impl Release {
    /// Its version: the tag without its `v`.
    pub fn version(&self) -> &str {
        self.tag_name.trim_start_matches('v')
    }

    /// It is newer than this build.
    pub fn newer(&self) -> bool {
        install::older(env!("CARGO_PKG_VERSION"), self.version())
    }

    /// This build's download of it ([`pick`]).
    pub fn asset(&self) -> Option<&Asset> {
        let appimage = std::env::var_os("APPIMAGE").is_some();
        pick(&self.assets, &self.tag_name, platform()?, appimage)
    }
}

/// This build's name in a release's downloads (`.github/workflows/release.yml`'s matrix); `None` for one no release
/// has.
fn platform() -> Option<&'static str> {
    use std::env::consts::{ARCH, OS};
    let (musl, avx2) = (cfg!(target_env = "musl"), cfg!(target_feature = "avx2"));
    Some(match (OS, ARCH, musl, avx2) {
        ("android", ..) => "android",
        ("linux", "x86_64", true, _) => "linux-x86_64-musl",
        ("linux", "aarch64", true, _) => "linux-aarch64-musl",
        ("linux", "x86_64", _, true) => "linux-x86_64-avx2",
        ("linux", "x86_64", ..) => "linux-x86_64",
        ("linux", "aarch64", ..) => "linux-aarch64",
        ("windows", "x86_64", _, true) => "windows-x86_64-avx2",
        ("windows", "x86_64", ..) => "windows-x86_64",
        ("windows", "aarch64", ..) => "windows-arm64",
        ("macos", "aarch64", ..) => "macos-arm64",
        ("freebsd", "x86_64", ..) => "freebsd-x86_64",
        _ => return None,
    })
}

/// The download for `platform` among `assets` of the release tagged `tag`: Android's base APK (else the one with the
/// sounds); the AppImage when this is one (`appimage`); else the archive without sounds, `.tar.gz` or `.zip`.
fn pick<'a>(assets: &'a [Asset], tag: &str, platform: &str, appimage: bool) -> Option<&'a Asset> {
    let version = tag.trim_start_matches('v');
    let names = match platform {
        "android" => vec![
            format!("honeydew-android-{version}-arm-base.apk"),
            format!("honeydew-android-{version}-arm.apk"),
        ],
        _ if appimage => vec![format!("honeydew-{tag}-{platform}.AppImage")],
        _ => ["tar.gz", "zip"]
            .map(|ext| format!("honeydew-{tag}-{platform}-no-sounds.{ext}"))
            .into(),
    };
    (names.iter()).find_map(|name| assets.iter().find(|a| a.name == *name))
}

/// A download's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Job {
    #[default]
    Idle,
    /// Bytes so far, and in all (0 while unknown).
    Running(u64, u64),
    /// Done: what to tell.
    Done(String),
    /// Why it failed.
    Failed(String),
}

impl Job {
    pub fn running(&self) -> bool {
        matches!(self, Job::Running(..))
    }

    /// `42%`, or `-` while the size is unknown.
    pub fn percent(done: u64, total: u64) -> String {
        match (done * 100).checked_div(total) {
            Some(p) => format!("{}%", p.min(100)),
            None => "-".into(),
        }
    }
}

/// What the UPDATES tab shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// The latest release, once known (now or from the cache).
    pub latest: Option<Release>,
    pub checking: bool,
    /// Why the last check failed.
    pub check_failed: Option<String>,
    /// The update's download.
    pub update: Job,
    /// The sound library's download.
    pub sounds: Job,
    /// The sound library's download waits for Wi-Fi.
    pub waiting_for_wifi: bool,
    /// The sound library's download is paused (it goes on from what it has when continued).
    pub paused: bool,
    /// The progress shown ([`Self::update`], [`Self::sounds`]) is the sound library's copy out of the full APK
    /// ([`keep_sounds`]).
    pub copying_sounds: bool,
    /// The newest sound library this build can read, of those published ([`newest_sounds`]); `None` until a check.
    pub sounds_latest: Option<String>,
    /// The sound library's download ended since [`take_sounds_end`].
    sounds_ended: bool,
    /// The version [`announce`] last told about or found told.
    announced: String,
}

static STATUS: Mutex<Status> = Mutex::new(Status {
    latest: None,
    checking: false,
    check_failed: None,
    update: Job::Idle,
    sounds: Job::Idle,
    waiting_for_wifi: false,
    paused: false,
    copying_sounds: false,
    sounds_latest: None,
    sounds_ended: false,
    announced: String::new(),
});
/// `Download sounds over mobile data` (the app keeps it in step with its setting).
static MOBILE_DATA: AtomicBool = AtomicBool::new(false);

fn with<R>(f: impl FnOnce(&mut Status) -> R) -> R {
    f(&mut STATUS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// What the UPDATES tab shows now.
pub fn status() -> Status {
    with(|s| s.clone())
}

/// Shows `status` as if it were so (the showcase renders).
#[doc(hidden)]
pub fn show(status: Status) {
    with(|s| *s = status);
}

/// `Download sounds over mobile data`: the sound library's download goes on over a metered network.
pub fn allow_mobile_data(on: bool) {
    MOBILE_DATA.store(on, Relaxed);
}

/// The kept answer: when GitHub was asked, in seconds since 1970, its answer, the newest sound library this build can
/// read ("" before any), and the newest version the app told about ([`announce`]).
#[derive(Default, Serialize, Deserialize)]
struct Cache {
    checked: u64,
    release: Release,
    #[serde(default)]
    sounds: String,
    #[serde(default)]
    announced: String,
}

/// The newest sound library among `tags` (`sounds-<version>`) that a build shipping `shipped` reads: one of the same
/// major version, whose index has the schema it reads (a library whose format changes takes a new major version, and
/// `Index::parse` refuses a schema it does not know besides). Never older than `shipped`, which is the floor.
fn newest_sounds<'a>(tags: impl IntoIterator<Item = &'a str>, shipped: &str) -> String {
    let major = |v: &str| v.trim().split('.').next().map(str::to_owned);
    let readable = |v: &&str| major(v) == major(shipped) && !install::older(v, shipped);
    let versions = tags.into_iter().filter_map(|t| t.strip_prefix("sounds-"));
    let newest =
        versions.filter(readable).fold(
            shipped.trim(),
            |a, b| if install::older(a, b) { b } else { a },
        );
    newest.to_owned()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The app data folder: never in tests, which do not touch the user's folders.
fn data_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::ui::main_menu::app_data_dir()
}

fn cache() -> Option<Cache> {
    serde_json::from_slice(&fs::read(data_dir()?.join(CACHE)).ok()?).ok()
}

fn store(cache: &Cache) {
    if let (Some(dir), Ok(bytes)) = (data_dir(), serde_json::to_vec(cache)) {
        let _ = fs::create_dir_all(&dir);
        let _ = crate::project_files::write_atomic(&dir.join(CACHE), &bytes);
    }
}

/// Learns the latest release on a worker, unless a check runs: from the kept answer while it is younger than
/// [`CHECK_EVERY`] (`auto`, at start) or [`MANUAL_EVERY`] (`Check now`), else from GitHub.
pub fn check(auto: bool) {
    let every = if auto { CHECK_EVERY } else { MANUAL_EVERY };
    let kept = cache();
    if let Some(kept) = kept.as_ref().filter(|c| now() < c.checked + every) {
        let tag = format!("sounds-{}", kept.sounds);
        let sounds = Some(newest_sounds([tag.as_str()], SHIPPED_RELEASE));
        with(|s| {
            (s.latest, s.check_failed) = (Some(kept.release.clone()), None);
            s.sounds_latest = sounds;
        });
        return;
    }
    if with(|s| std::mem::replace(&mut s.checking, true)) {
        return;
    }
    thread::spawn(move || {
        let found = api::<Release>(LATEST);
        // The sound library's own releases: a failure keeps the one this build ships with.
        #[derive(Deserialize)]
        struct Tag {
            #[serde(rename = "ref")]
            name: String,
        }
        let tags = api::<Vec<Tag>>(SOUNDS_TAGS).unwrap_or_default();
        let tags = tags.iter().map(|t| t.name.trim_start_matches("refs/tags/"));
        let sounds = newest_sounds(tags, SHIPPED_RELEASE);
        if let Ok(release) = &found {
            let announced = kept.map(|c| c.announced).unwrap_or_default();
            let (checked, release, sounds) = (now(), release.clone(), sounds.clone());
            store(&Cache {
                checked,
                release,
                sounds,
                announced,
            });
        }
        with(|s| {
            s.checking = false;
            s.sounds_latest = Some(sounds);
            match found {
                Ok(release) => (s.latest, s.check_failed) = (Some(release), None),
                // GitHub's answer for a repository without a release yet.
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    s.check_failed = Some(format!("{REPO} has no release yet."));
                }
                Err(err) => s.check_failed = Some(err.to_string()),
            }
        });
    });
}

/// GitHub's API's answer at `url`.
fn api<T: serde::de::DeserializeOwned>(url: &str) -> io::Result<T> {
    let dir = data_dir().ok_or(io::ErrorKind::NotFound)?;
    fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{CACHE}.part"));
    let _ = fs::remove_file(&file);
    get(url, &file, &|_| ())?;
    let bytes = fs::read(&file);
    let _ = fs::remove_file(&file);
    serde_json::from_slice(&bytes?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// The latest release's version, once, when it is newer than this build and than the last one told about: the app
/// tells the user about it.
pub fn announce() -> Option<String> {
    let version = with(|s| {
        let latest = s.latest.as_ref().filter(|r| r.newer())?;
        let version = latest.version().to_owned();
        (s.announced != version).then(|| s.announced.clone_from(&version))?;
        Some(version)
    })?;
    let mut kept = cache().unwrap_or_default();
    if !kept.announced.is_empty() && !install::older(&kept.announced, &version) {
        return None;
    }
    kept.announced.clone_from(&version);
    if kept.release.tag_name.is_empty() {
        kept.release = with(|s| s.latest.clone()).unwrap_or_default();
    }
    store(&kept);
    Some(version)
}

/// Downloads this build's download of the latest release on a worker, then installs it as the module docs say.
pub fn install() {
    let asset = with(|s| s.latest.as_ref().and_then(Release::asset).cloned());
    let Some(asset) = asset else { return };
    let size = asset.size;
    if with(|s| std::mem::replace(&mut s.update, Job::Running(0, size)).running()) {
        return;
    }
    thread::spawn(move || {
        let result = fetch_update(&asset);
        with(|s| match result {
            Ok(done) => s.update = Job::Done(done),
            Err(err) => s.update = Job::Failed(err.to_string()),
        });
    });
}

/// The update's download, then its install: what to tell.
fn fetch_update(asset: &Asset) -> io::Result<String> {
    let to = update_path(&asset.name)?;
    if fs::metadata(&to).map_or(0, |m| m.len()) != asset.size || asset.size == 0 {
        let _ = fs::remove_file(&to);
        let size = asset.size;
        get(&asset.browser_download_url, &to, &|n| {
            with(|s| s.update = Job::Running(n, size));
        })?;
    }
    if asset.size != 0 && fs::metadata(&to)?.len() != asset.size {
        let _ = fs::remove_file(&to);
        return Err(io::Error::other("The download came damaged. Try again."));
    }
    #[cfg(target_os = "android")]
    if let Some(why) = keep_sounds(asset.size, |done, total| {
        with(|s| s.update = Job::Running(done, total))
    }) {
        return Ok(format!("{why} {}", installed(&to)?));
    }
    installed(&to)
}

/// Before the base APK replaces a full one: its sounds copied into the sound library's folder, which the base APK
/// reads, so that it downloads only the parts that changed (`install::keep_apk_library`), on this worker, the copy's
/// percent in the update's button. `Some` what to tell when they could not be kept: the base APK downloads them.
#[cfg(target_os = "android")]
fn keep_sounds(apk_bytes: u64, tell: impl Fn(u64, u64)) -> Option<String> {
    /// Room kept free beside the update and the sounds, for the installer.
    const MARGIN: u64 = 64 << 20;
    let target = install::dir()?;
    let free = crate::android::usable_space()?;
    let progress = Progress::default();
    let kept = thread::scope(|scope| {
        let copy =
            scope.spawn(|| install::keep_apk_library(&target, free, apk_bytes + MARGIN, &progress));
        while !copy.is_finished() {
            with(|s| s.copying_sounds = true);
            tell(progress.done.load(Relaxed), progress.total.load(Relaxed));
            thread::sleep(Duration::from_millis(100));
        }
        copy.join()
    });
    with(|s| s.copying_sounds = false);
    let later = "They download after the update.";
    match kept {
        Ok(Ok(_)) => None,
        Ok(Err(err)) if err.kind() == io::ErrorKind::StorageFull => Some(format!(
            "There is not enough free space to keep the sounds. {later}"
        )),
        Ok(Err(err)) => Some(format!("The sounds could not be kept ({err}). {later}")),
        Err(_) => Some(format!("The sounds could not be kept. {later}")),
    }
}

/// Where the update downloads to: Android's update folder (`HoneydewUpdate` serves it to the installer, which keeps
/// nothing else there), beside the AppImage, or the Downloads folder.
#[cfg(target_os = "android")]
fn update_path(name: &str) -> io::Result<PathBuf> {
    let dir = crate::android::update_dir().ok_or(io::ErrorKind::NotFound)?;
    fs::create_dir_all(&dir)?;
    for old in fs::read_dir(&dir)?.flatten() {
        if old.file_name() != name {
            let _ = fs::remove_file(old.path());
        }
    }
    Ok(dir.join(name))
}

#[cfg(not(target_os = "android"))]
fn update_path(name: &str) -> io::Result<PathBuf> {
    if let Some(appimage) = appimage(name) {
        return Ok(appimage.with_extension("AppImage.new"));
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    let downloads = home.map(|h| PathBuf::from(h).join("Downloads"));
    let dir = (downloads.filter(|d| d.is_dir())).or_else(data_dir);
    Ok(dir.ok_or(io::ErrorKind::NotFound)?.join(name))
}

/// The running AppImage (`$APPIMAGE`), when `name` is an AppImage to replace it.
#[cfg(not(target_os = "android"))]
fn appimage(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("APPIMAGE")?);
    (name.ends_with(".AppImage") && path.is_absolute()).then_some(path)
}

/// Installs the update downloaded `to` there: what to tell.
#[cfg(target_os = "android")]
fn installed(to: &Path) -> io::Result<String> {
    match crate::android::install_apk(to).map_err(io::Error::other)? {
        true => Ok("The installer is open.".into()),
        false => Ok(format!(
            "Allow {} to install apps, then tap Install again.",
            crate::APP_NAME
        )),
    }
}

#[cfg(not(target_os = "android"))]
fn installed(to: &Path) -> io::Result<String> {
    let name = to.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let name = name.trim_end_matches(".new");
    if let Some(appimage) = appimage(name) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(to, fs::Permissions::from_mode(0o755))?;
        }
        fs::rename(to, &appimage)?;
        return Ok(format!("Updated. Restart {} to use it.", crate::APP_NAME));
    }
    let _ = crate::ui::main_menu::reveal(to);
    Ok(format!(
        "Saved to {}. Quit {} and use the new download.",
        to.display(),
        crate::APP_NAME
    ))
}

/// The sound library release `version`, published as its own release (`sounds-<version>`, from
/// `.github/workflows/release.yml`): `index.json`, `CREDITS.txt` and the parts.
pub fn sounds_url(version: &str) -> String {
    format!("https://github.com/{SOUNDS_REPO}/releases/download/sounds-{version}")
}

/// The sound library to download: the newest this build reads, of those a check found ([`newest_sounds`]), else the
/// one it ships with.
pub fn sounds_wanted() -> String {
    let found = with(|s| s.sounds_latest.clone());
    found.unwrap_or_else(|| SHIPPED_RELEASE.trim().to_owned())
}

/// The running sound library download's progress, while one runs, so [`pause_sounds`] can reach it.
static SOUNDS_PROGRESS: Mutex<Option<Arc<Progress>>> = Mutex::new(None);

/// Pauses (`on`) or continues the sound library's download. Paused, it waits before its next part and goes on from
/// the `.part` it left, so continuing downloads only what is missing.
pub fn pause_sounds(on: bool) {
    with(|s| s.paused = on);
    let held = SOUNDS_PROGRESS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = held.as_ref() {
        p.paused.store(on, Relaxed);
    }
}

/// Downloads the sound library ([`sounds_wanted`]) into its folder on a worker, unless it is running: only the parts
/// the folder lacks. On Android's full APK the folder first gets the APK's own sounds ([`keep_sounds`]), so that only
/// what changed downloads.
pub fn download_sounds() {
    let Some(target) = install::dir() else { return };
    if with(|s| std::mem::replace(&mut s.sounds, Job::Running(0, 0)).running()) {
        return;
    }
    let version = sounds_wanted();
    thread::spawn(move || {
        #[cfg(target_os = "android")]
        let _ = keep_sounds(0, |done, total| {
            with(|s| s.sounds = Job::Running(done, total))
        });
        let progress = Arc::new(Progress::default());
        *SOUNDS_PROGRESS.lock().unwrap_or_else(|e| e.into_inner()) = Some(progress.clone());
        let tell = || {
            let (done, total) = (progress.done.load(Relaxed), progress.total.load(Relaxed));
            with(|s| s.sounds = Job::Running(done, total));
        };
        let fetch = |url: &str, to: &Path, grown: &dyn Fn(u64)| {
            network_allowed()?;
            get(url, to, &|n| {
                grown(n);
                tell();
            })
        };
        let result = install::download(&sounds_url(&version), &target, &progress, &fetch);
        *SOUNDS_PROGRESS.lock().unwrap_or_else(|e| e.into_inner()) = None;
        with(|s| {
            s.waiting_for_wifi = false;
            s.paused = false;
            s.sounds_ended = true;
            s.sounds = match result {
                Ok(_) => Job::Done(format!(
                    "Downloaded. Restart {} to use it.",
                    crate::APP_NAME
                )),
                Err(err) => Job::Failed(err.to_string()),
            };
        });
    });
}

/// How the sound library's download ended, once: `Err` with why it failed.
pub fn take_sounds_end() -> Option<Result<(), String>> {
    with(|s| {
        if !std::mem::take(&mut s.sounds_ended) {
            return None;
        }
        match &s.sounds {
            Job::Failed(why) => Some(Err(why.clone())),
            _ => Some(Ok(())),
        }
    })
}

/// Waits while the only network is metered and mobile data is not allowed (Android); fails without a network.
fn network_allowed() -> io::Result<()> {
    #[cfg(target_os = "android")]
    loop {
        use crate::android::Network;
        let wait = match crate::android::network() {
            Network::None => return Err(io::Error::other(OFFLINE)),
            Network::Metered => !MOBILE_DATA.load(Relaxed),
            Network::Unmetered => false,
        };
        with(|s| s.waiting_for_wifi = wait);
        if !wait {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(5));
    }
    #[cfg(not(target_os = "android"))]
    Ok(())
}

/// Downloads `url` into `to`, going on from the bytes `to` holds, telling `grown` its size as it grows. A failure is
/// tried again after [`RETRIES`], but without a network or a file there. Blocks: call it on a worker.
pub fn get(url: &str, to: &Path, grown: &dyn Fn(u64)) -> io::Result<()> {
    let mut waits = RETRIES.into_iter();
    loop {
        let err = match transfer(url, to, grown) {
            Ok(()) => return Ok(()),
            Err(err) => err,
        };
        let hopeless = err.kind() == io::ErrorKind::NotFound || err.to_string() == OFFLINE;
        match waits.next().filter(|_| !hopeless) {
            Some(secs) => thread::sleep(Duration::from_secs(secs)),
            None => return Err(err),
        }
    }
}

/// The size of the file at `path`, 0 without one.
fn size_of(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |m| m.len())
}

/// One try of [`get`]: `curl`, its exit code read as the error.
#[cfg(not(target_os = "android"))]
fn transfer(url: &str, to: &Path, grown: &dyn Fn(u64)) -> io::Result<()> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut curl = Command::new("curl");
    // Only HTTPS, redirects too; a stalled transfer (under 1 KB/s for a minute) fails and is tried again.
    curl.args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--proto", "=https", "--proto-redir", "=https"])
        .args([
            "--connect-timeout",
            "30",
            "--speed-limit",
            "1024",
            "--speed-time",
            "60",
        ])
        .args(["--continue-at", "-", "--output"])
        .arg(to)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // No console window flashing up for it.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut curl, 0x0800_0000);
    let mut child = curl.spawn().map_err(|err| match err.kind() {
        io::ErrorKind::NotFound => io::Error::other("Downloads need curl, which is not installed."),
        _ => err,
    })?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        grown(size_of(to));
        thread::sleep(Duration::from_millis(100));
    };
    grown(size_of(to));
    let mut why = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut why);
    }
    let why = why.trim().trim_start_matches("curl: ").to_owned();
    match status.code() {
        Some(0) => Ok(()),
        // Could not resolve the host, or connect to it.
        Some(6 | 7) => Err(io::Error::other(OFFLINE)),
        Some(22) if why.contains("404") => Err(io::Error::new(io::ErrorKind::NotFound, why)),
        _ => Err(io::Error::other(why)),
    }
}

/// One try of [`get`]: `HoneydewActivity.download`, on a thread of its own while this one tells the size.
#[cfg(target_os = "android")]
fn transfer(url: &str, to: &Path, grown: &dyn Fn(u64)) -> io::Result<()> {
    let (url, path) = (url.to_owned(), to.to_string_lossy().into_owned());
    let worker = thread::spawn(move || crate::android::download(&url, &path));
    while !worker.is_finished() {
        grown(size_of(to));
        thread::sleep(Duration::from_millis(100));
    }
    grown(size_of(to));
    let failed = worker
        .join()
        .map_err(|_| io::Error::other("The download stopped."))?;
    match failed.as_deref() {
        None => Ok(()),
        Some("offline") => Err(io::Error::other(OFFLINE)),
        Some(why) if why.contains("404") => Err(io::Error::new(io::ErrorKind::NotFound, why)),
        Some(why) => Err(io::Error::other(why.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sound_library_offered_is_the_newest_this_build_reads_and_never_older_than_its_own() {
        let tags = [
            "sounds-0.1.3",
            "sounds-0.1.10",
            "sounds-0.2.0",
            "sounds-1.0.0",
            "v0.4.10",
            "sounds-x",
        ];
        assert_eq!(newest_sounds(tags, "0.1.4"), "0.2.0");
        // Another major version is a format this build may not read; an older one is below its own.
        assert_eq!(
            newest_sounds(["sounds-1.0.0", "sounds-0.1.3"], "0.1.4\n"),
            "0.1.4"
        );
        assert_eq!(newest_sounds(["sounds-0.1.10"], "0.1.9"), "0.1.10");
    }

    #[test]
    fn a_newer_tag_is_an_update_and_each_platform_picks_its_own_download() {
        let release = |tag: &str| Release {
            tag_name: tag.into(),
            ..Release::default()
        };
        let this = env!("CARGO_PKG_VERSION");
        assert!(!release(&format!("v{this}")).newer());
        assert!(release("v999.0.0").newer() && !release("v0.0.1").newer());
        // 0.4.10 is newer than 0.4.9: numbers, not text.
        assert!(install::older("0.4.9", release("v0.4.10").version()));

        let names = [
            "honeydew-v0.4.10-linux-x86_64.tar.gz",
            "honeydew-v0.4.10-linux-x86_64-no-sounds.tar.gz",
            "honeydew-v0.4.10-linux-x86_64-avx2-no-sounds.tar.gz",
            "honeydew-v0.4.10-linux-x86_64.AppImage",
            "honeydew-v0.4.10-linux-x86_64-musl-no-sounds.tar.gz",
            "honeydew-v0.4.10-windows-x86_64-no-sounds.zip",
            "honeydew-v0.4.10-windows-arm64.zip",
            "honeydew-android-0.4.10-arm.apk",
            "honeydew-android-0.4.10-arm-base.apk",
            "honeydew-v0.4.10-sounds.zip",
        ];
        let assets: Vec<Asset> = (names.iter())
            .map(|name| Asset {
                name: name.to_string(),
                ..Asset::default()
            })
            .collect();
        let picked = |platform, appimage| {
            pick(&assets, "v0.4.10", platform, appimage).map(|a| a.name.as_str())
        };
        assert_eq!(picked("linux-x86_64", false), Some(names[1]));
        assert_eq!(picked("linux-x86_64-avx2", false), Some(names[2]));
        assert_eq!(picked("linux-x86_64", true), Some(names[3]));
        assert_eq!(picked("linux-x86_64-musl", false), Some(names[4]));
        assert_eq!(picked("windows-x86_64", false), Some(names[5]));
        assert_eq!(picked("android", false), Some(names[8]));
        // Not in this release: none rather than another platform's.
        assert_eq!(picked("windows-arm64", false), None);
        assert_eq!(picked("linux-aarch64", true), None);
        // A release before the base APK: the one with the sounds.
        let full = &assets[..8];
        let picked = pick(full, "v0.4.10", "android", false).map(|a| a.name.as_str());
        assert_eq!(picked, Some(names[7]));
    }
}

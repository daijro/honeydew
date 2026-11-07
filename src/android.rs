//! Android's NativeActivity entry point and access to app-private storage and APK assets.

use std::ffi::CString;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use android_activity::{AndroidApp, ndk::asset::Asset};

use crate::audio_engine::RecordTap;
use crate::recording::input::{InputRing, Mic};

static ACTIVITY: Mutex<Option<AndroidApp>> = Mutex::new(None);
static DISPLAY_DPI: AtomicU32 = AtomicU32::new(0);

/// The Activity refreshes physical density at creation and configuration changes; zero means unavailable.
pub fn display_dpi() -> f32 {
    f32::from_bits(DISPLAY_DPI.load(Ordering::Relaxed))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daijro_honeydew_HoneydewActivity_onDisplayDpi(
    _env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    dpi: jni::sys::jfloat,
) {
    DISPLAY_DPI.store(dpi.to_bits(), Ordering::Relaxed);
}

/// The microphone permission (`RECORD_AUDIO`): not asked (or the last answer was no, already told), asking, allowed,
/// denied.
static MIC: AtomicU8 = AtomicU8::new(UNASKED);
const UNASKED: u8 = 0;
const ASKING: u8 = 1;
const GRANTED: u8 = 2;
const DENIED: u8 = 3;

/// Whether the app may record. The first call asks (`HoneydewActivity.askMic`): allowed already, else the system's
/// question shows and its answer comes back (`onMic`). A denial is told once; the next call asks again (after two
/// denials Android answers at once, without the question).
pub fn mic() -> Mic {
    match MIC.load(Ordering::Acquire) {
        GRANTED => Mic::Granted,
        ASKING => Mic::Asking,
        DENIED => {
            let _ = MIC.compare_exchange(DENIED, UNASKED, Ordering::AcqRel, Ordering::Acquire);
            Mic::Denied
        }
        _ => {
            MIC.store(ASKING, Ordering::Release);
            let mut granted = false;
            let asked = with_activity(|env, activity| {
                granted = env.call_method(activity, "askMic", "()Z", &[])?.z()?;
                Ok(())
            });
            match asked {
                Ok(()) if granted => {
                    MIC.store(GRANTED, Ordering::Release);
                    Mic::Granted
                }
                Ok(()) => Mic::Asking,
                Err(err) => {
                    eprintln!("microphone permission not asked: {err}");
                    MIC.store(UNASKED, Ordering::Release);
                    Mic::Denied
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daijro_honeydew_HoneydewActivity_onMic(
    _env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    granted: jni::sys::jboolean,
) {
    MIC.store(
        if granted != 0 { GRANTED } else { DENIED },
        Ordering::Release,
    );
}

/// Headphones are among the outputs (wired, USB or Bluetooth), as the Activity's device callback last said.
static HEADPHONES: AtomicBool = AtomicBool::new(false);

/// Whether headphones play the sound, not the phone's speaker: monitoring may sound then
/// (`recording::input::may_monitor`).
pub fn headphones() -> bool {
    HEADPHONES.load(Ordering::Acquire)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daijro_honeydew_HoneydewActivity_onHeadphones(
    _env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    on: jni::sys::jboolean,
) {
    HEADPHONES.store(on != 0, Ordering::Release);
}

/// The inputs `Audio input:` lists (`HoneydewActivity.inputs`), as (device id, label): the phone's mics, a wired
/// headset's and USB audio. None when the Activity cannot say.
pub fn inputs() -> Vec<(i32, String)> {
    let mut list = String::new();
    let listed = with_activity(|env, activity| {
        let got = env.call_method(activity, "inputs", "()Ljava/lang/String;", &[])?;
        list = text(env, &got.l()?.into()).unwrap_or_default();
        Ok(())
    });
    if let Err(err) = listed {
        eprintln!("audio inputs not listed: {err}");
    }
    let parts: Vec<&str> = list.split_terminator('\0').collect();
    let pairs = parts.chunks_exact(2);
    pairs
        .filter_map(|p| Some((p[0].parse().ok()?, p[1].to_owned())))
        .collect()
}

/// Opens the input labelled `label` (one of [`inputs`]) at `rate` into a new ring: stereo float, else mono, at the
/// default input preset as cpal's default input. Through AAudio by its device id, as cpal opens a device by its name
/// only, which a headset's mic shares with its earphones. (ndk's `audio` feature is cpal's, for its AAudio host.)
pub fn open_input(
    label: &str,
    rate: u32,
) -> Result<(Arc<InputRing>, android_activity::ndk::audio::AudioStream), String> {
    use android_activity::ndk::audio::{
        AudioCallbackResult, AudioDirection, AudioFormat, AudioStreamBuilder,
    };
    let (id, _) = (inputs().into_iter())
        .find(|(_, l)| l == label)
        .ok_or_else(|| format!("{label} is not connected."))?;
    let mut error = String::new();
    for channels in [2_usize, 1] {
        // Two seconds, as cpal's: the engine drains it once per render call.
        let ring = Arc::new(InputRing::new(channels, rate, 2 * rate as usize));
        let (fill, failed) = (ring.clone(), ring.clone());
        let builder = AudioStreamBuilder::new().map_err(|err| format!("{err:?}"))?;
        let opened = builder
            .direction(AudioDirection::Input)
            .device_id(id)
            .channel_count(channels as i32)
            .format(AudioFormat::PCM_Float)
            .sample_rate(rate as i32)
            .data_callback(Box::new(move |_, data, frames| {
                let len = frames.max(0) as usize * channels;
                // SAFETY: AAudio hands `frames` frames of `channels` floats, the format asked for.
                let samples = unsafe { std::slice::from_raw_parts(data.cast::<f32>(), len) };
                fill.push(samples.iter().copied());
                AudioCallbackResult::Continue
            }))
            .error_callback(Box::new(move |_, _| failed.set_failed()))
            .open_stream();
        match opened {
            Ok(stream) => {
                stream.request_start().map_err(|err| format!("{err:?}"))?;
                return Ok((ring, stream));
            }
            Err(err) => error = format!("{err:?}"),
        }
    }
    Err(error)
}

/// The tap of the recording under way. No frame runs while the app is in the background, so an interruption ends the
/// take from Java's thread by disarming it (`onInterrupted`), and the app's next frame stops the transport
/// ([`interrupted`]).
static RECORDING: Mutex<Option<Arc<RecordTap>>> = Mutex::new(None);
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// The recording under way, `tap` (each frame): while one runs, the app holds the audio focus
/// (`HoneydewActivity.recording`), and a call, another app taking the focus, or the app leaving the screen ends its
/// take there and then.
pub fn recording(tap: Option<&Arc<RecordTap>>) {
    let mut held = RECORDING.lock().expect("recording lock");
    if held.as_ref().map(Arc::as_ptr) == tap.map(Arc::as_ptr) {
        return;
    }
    *held = tap.cloned();
    drop(held);
    INTERRUPTED.store(false, Ordering::Release);
    let on = tap.is_some();
    let focus = with_activity(|env, activity| {
        env.call_method(activity, "recording", "(Z)V", &[on.into()])
            .map(drop)
    });
    if let Err(err) = focus {
        eprintln!("audio focus not changed: {err}");
    }
}

/// An interruption ended the recording's take since the last call.
pub fn interrupted() -> bool {
    INTERRUPTED.swap(false, Ordering::AcqRel)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daijro_honeydew_HoneydewActivity_onInterrupted(
    _env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
) {
    if let Some(tap) = RECORDING.lock().ok().as_deref().and_then(Option::as_ref) {
        tap.disarm();
        INTERRUPTED.store(true, Ordering::Release);
    }
}

/// `onPause`, before Android pauses the app thread, which then runs no frame until the app is back: the frame that
/// writes the song's unsaved changes ([`crate::project_files::leave`]) runs first, waited for up to half a second.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daijro_honeydew_HoneydewActivity_onLeaving(
    _env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
) {
    crate::project_files::leave();
    // ponytail: polled every 5 ms; a condvar if the wait ever shows.
    for _ in 0..100 {
        if !crate::project_files::leaving() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Back was pressed (Android 13 and up hand it to `HoneydewActivity`'s back callback, not as a key) and waits for
/// the next frame ([`take_back`]), which [`BACK_WAKE`] asks for.
static BACK: AtomicBool = AtomicBool::new(false);
static BACK_WAKE: std::sync::OnceLock<eframe::egui::Context> = std::sync::OnceLock::new();

/// Back was pressed since the last call. `ctx` is the context a later press wakes.
pub fn take_back(ctx: &eframe::egui::Context) -> bool {
    BACK_WAKE.get_or_init(|| ctx.clone());
    BACK.swap(false, Ordering::AcqRel)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daijro_honeydew_HoneydewActivity_onBack(
    _env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
) {
    BACK.store(true, Ordering::Release);
    if let Some(ctx) = BACK_WAKE.get() {
        ctx.request_repaint();
    }
}

/// Android sends Back to the activity's back callback (Android 13 and up, `HoneydewActivity.onCreate`): its key event,
/// which comes too, is then not a second press.
pub fn back_callback() -> bool {
    AndroidApp::sdk_version() >= 33
}

/// Back's default with nothing open: the app goes to the background, as Android's own Back does for it.
pub fn move_to_back() {
    let moved = with_activity(|env, activity| {
        env.call_method(activity, "moveTaskToBack", "(Z)Z", &[true.into()])
            .map(drop)
    });
    if let Err(err) = moved {
        eprintln!("could not move to the background: {err}");
    }
}

/// What a document pick finishes with: the copy of the picked file, an error, or `None` when it was cancelled.
pub type Picked = Option<Result<PathBuf, String>>;
/// What a folder pick finishes with: each file in the picked folder or below, as its path from the folder's parent
/// (`Drums/Kit/Kick.wav`) and its document URI; an error; or `None` when it was cancelled.
pub type PickedFolder = Option<Result<Vec<(String, String)>, String>>;
/// Where a Java call's result goes once it comes, on a Java thread.
type Waiting<T> = Mutex<Option<Box<dyn FnOnce(T) + Send>>>;
static PICKING: Waiting<Picked> = Mutex::new(None);
static PICKING_FOLDER: Waiting<PickedFolder> = Mutex::new(None);
static COPYING: Waiting<Option<String>> = Mutex::new(None);

fn finish<T>(waiting: &Waiting<T>, result: T) {
    let done = waiting.lock().ok().and_then(|mut done| done.take());
    if let Some(done) = done {
        done(result);
    }
}

/// A Java string, `None` for `null`.
fn text(env: &mut jni::JNIEnv<'_>, s: &jni::objects::JString<'_>) -> Option<String> {
    (!s.is_null()).then(|| env.get_string(s).map(String::from).unwrap_or_default())
}

/// Opens the system document picker (`HoneydewActivity.openDocument`); `done` gets the pick, copied into the app's
/// cache (at most `max_bytes` + 1 bytes), on a Java thread.
pub fn pick_document(max_bytes: u64, done: impl FnOnce(Picked) + Send + 'static) {
    *PICKING.lock().expect("picker lock") = Some(Box::new(done));
    let max = jni::sys::jlong::try_from(max_bytes).unwrap_or(jni::sys::jlong::MAX - 1);
    let opened = with_activity(|env, activity| {
        env.call_method(activity, "openDocument", "(J)V", &[max.into()])
            .map(drop)
    });
    if let Err(err) = opened {
        finish(&PICKING, Some(Err(err)));
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daijro_honeydew_HoneydewActivity_onDocumentPicked(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    path: jni::objects::JString<'_>,
    error: jni::objects::JString<'_>,
) {
    let picked = match (text(&mut env, &path), text(&mut env, &error)) {
        (Some(path), _) => Some(Ok(path.into())),
        (None, Some(error)) => Some(Err(error)),
        (None, None) => None,
    };
    finish(&PICKING, picked);
}

/// Opens the system folder picker (`HoneydewActivity.openFolder`); `done` gets the files of the folder picked, listed
/// on a Java thread.
pub fn pick_folder(done: impl FnOnce(PickedFolder) + Send + 'static) {
    *PICKING_FOLDER.lock().expect("picker lock") = Some(Box::new(done));
    let opened = with_activity(|env, activity| {
        env.call_method(activity, "openFolder", "()V", &[])
            .map(drop)
    });
    if let Err(err) = opened {
        finish(&PICKING_FOLDER, Some(Err(err)));
    }
}

/// `files` alternates a path and its URI, each ending in `\0` (no name holds one).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daijro_honeydew_HoneydewActivity_onFolderPicked(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    files: jni::objects::JString<'_>,
    error: jni::objects::JString<'_>,
) {
    let picked = match (text(&mut env, &files), text(&mut env, &error)) {
        (Some(files), _) => {
            let parts: Vec<&str> = files.split_terminator('\0').collect();
            let pairs = parts
                .chunks_exact(2)
                .map(|p| (p[0].to_owned(), p[1].to_owned()));
            Some(Ok(pairs.collect()))
        }
        (None, Some(error)) => Some(Err(error)),
        (None, None) => None,
    };
    finish(&PICKING_FOLDER, picked);
}

/// Copies each file `(document URI, user file)` on a Java thread (`HoneydewActivity.copyFiles`); `done` gets `None`
/// once all are in, else why the copy stopped.
pub fn copy_files(files: &[(String, PathBuf)], done: impl FnOnce(Option<String>) + Send + 'static) {
    *COPYING.lock().expect("copy lock") = Some(Box::new(done));
    let list: String = (files.iter())
        .map(|(from, to)| format!("{from}\0{}\0", to.display()))
        .collect();
    let started = with_activity(|env, activity| {
        let list = env.auto_local(env.new_string(list)?);
        env.call_method(
            activity,
            "copyFiles",
            "(Ljava/lang/String;)V",
            &[jni::objects::JValue::Object(list.as_ref())],
        )
        .map(drop)
    });
    if let Err(err) = started {
        finish(&COPYING, Some(err));
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daijro_honeydew_HoneydewActivity_onFilesCopied(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    error: jni::objects::JString<'_>,
) {
    finish(&COPYING, text(&mut env, &error));
}

/// Asks the Activity for a screen orientation (`ActivityInfo.SCREEN_ORIENTATION_*`, from
/// `Settings::android_orientation`). The window then turns and resizes like a rotation (the manifest's `configChanges`
/// keep the Activity), and the next frame lays out at the new size. A failure is only logged.
///
/// PR #87: the Activity posts the request to its UI thread itself (`HoneydewActivity.requestOrientation`): changing a
/// window's orientation from the app's own thread is not the UI thread's to do, and on some devices it wedges the
/// window until the app is killed.
pub fn request_orientation(orientation: i32) {
    let requested = with_activity(|env, activity| {
        env.call_method(
            activity,
            "requestOrientation",
            "(I)V",
            &[orientation.into()],
        )
        .map(drop)
    });
    if let Err(err) = requested {
        eprintln!("screen orientation not set: {err}");
    }
}

/// Opens `folder`, in the user folder, in the system Files app (`HoneydewActivity.showFolder`, through the app's
/// documents provider, `HoneydewDocuments`), which itself tells the user when no app can.
pub fn show_folder(folder: &std::path::Path) -> Result<(), String> {
    with_activity(|env, activity| {
        let path = env.auto_local(env.new_string(folder.to_string_lossy())?);
        env.call_method(
            activity,
            "showFolder",
            "(Ljava/lang/String;)V",
            &[jni::objects::JValue::Object(path.as_ref())],
        )
        .map(drop)
    })
}

/// `Send` (#72): `files`, in the user folder, to the system's chooser as `mime` (`HoneydewActivity.share`), each by
/// its document in `HoneydewDocuments` with a read grant only. Why not, when one is not in the user folder.
pub fn share(files: &[PathBuf], mime: &str) -> Result<(), String> {
    let list: String = files.iter().map(|f| format!("{}\0", f.display())).collect();
    let mut failed = None;
    with_activity(|env, activity| {
        let list = env.auto_local(env.new_string(list)?);
        let mime = env.auto_local(env.new_string(mime)?);
        let why = env.call_method(
            activity,
            "share",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
            &[
                jni::objects::JValue::Object(list.as_ref()),
                jni::objects::JValue::Object(mime.as_ref()),
            ],
        )?;
        let why = env.auto_local(jni::objects::JString::from(why.l()?));
        failed = text(env, &why);
        Ok(())
    })?;
    failed.map_or(Ok(()), Err)
}

/// The network the phone is on: the sound library's download waits for an unmetered one (`update`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    /// None, or none that reaches the internet.
    None,
    /// Wi-Fi or Ethernet.
    Unmetered,
    /// Mobile data, or a hotspot.
    Metered,
}

/// The network the phone is on (`HoneydewActivity.network`).
pub fn network() -> Network {
    let mut kind = 0;
    let asked = with_activity(|env, activity| {
        kind = env.call_method(activity, "network", "()I", &[])?.i()?;
        Ok(())
    });
    match (asked, kind) {
        (Ok(()), 1) => Network::Unmetered,
        (Ok(()), 2) => Network::Metered,
        _ => Network::None,
    }
}

/// Downloads `url` into the file at `path`, going on from the bytes it holds (`HoneydewActivity.download`), on this
/// thread, which it blocks for as long: `None` once it is in, else why not (`offline` without a network).
pub fn download(url: &str, path: &str) -> Option<String> {
    let mut failed = None;
    let called = with_activity_unlocked(|env, activity| {
        let url = env.auto_local(env.new_string(url)?);
        let path = env.auto_local(env.new_string(path)?);
        let why = env.call_method(
            activity,
            "download",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
            &[
                jni::objects::JValue::Object(url.as_ref()),
                jni::objects::JValue::Object(path.as_ref()),
            ],
        )?;
        let why = env.auto_local(jni::objects::JString::from(why.l()?));
        failed = text(env, &why);
        Ok(())
    });
    called.err().or(failed)
}

/// The bytes free for the app's files (`HoneydewActivity.usableSpace`): what the sounds' copy out of the full APK
/// checks first (`update`).
pub fn usable_space() -> Option<u64> {
    let mut free = -1;
    with_activity(|env, activity| {
        free = env.call_method(activity, "usableSpace", "()J", &[])?.j()?;
        Ok(())
    })
    .ok()?;
    u64::try_from(free).ok()
}

/// Where an update's APK downloads to (`update`): `update` in the app's files, which `HoneydewUpdate` serves to the
/// system's installer.
pub fn update_dir() -> Option<PathBuf> {
    Some(data_dir()?.join("update"))
}

/// Hands the APK at `path`, in [`update_dir`], to the system's installer (`HoneydewActivity.installApk`): `false` when
/// the app may not install apps yet, and the setting that allows it opened instead.
pub fn install_apk(path: &std::path::Path) -> Result<bool, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let name = name.ok_or("no APK")?;
    let mut opened = false;
    with_activity(|env, activity| {
        let name = env.auto_local(env.new_string(&name)?);
        let args = [jni::objects::JValue::Object(name.as_ref())];
        opened = (env.call_method(activity, "installApk", "(Ljava/lang/String;)Z", &args)?).z()?;
        Ok(())
    })?;
    Ok(opened)
}

/// Android gives each install an absolute, private data directory; no storage permission is needed.
pub fn data_dir() -> Option<PathBuf> {
    ACTIVITY
        .lock()
        .ok()?
        .as_ref()?
        .internal_data_path()
        .filter(|path| path.is_absolute())
}

/// The app's external files folder (`Android/data/<package>/files`), which a computer sees over USB: the user folder
/// (`ui::main_menu::user_dir`). No storage permission is needed either.
pub fn external_dir() -> Option<PathBuf> {
    ACTIVITY
        .lock()
        .ok()?
        .as_ref()?
        .external_data_path()
        .filter(|path| path.is_absolute())
}

/// Assets are opened separately on each reader thread, so no non-thread-safe asset handle is shared.
pub fn asset(path: &str) -> Option<Asset> {
    let name = CString::new(path).ok()?;
    ACTIVITY.lock().ok()?.as_ref()?.asset_manager().open(&name)
}

pub fn asset_bytes(path: &str, limit: usize) -> Option<Vec<u8>> {
    let mut asset = asset(path)?;
    if asset.length() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    asset.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// Called by android-activity's NativeActivity glue, on its app thread.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    crate::android_text::reset();
    *ACTIVITY.lock().expect("activity lock") = Some(app.clone());
    if let Some(index) = asset_bytes(
        "content/index.json",
        crate::content::bundle::MAX_INDEX_BYTES,
    )
    .and_then(|bytes| crate::content::bundle::Index::parse(&bytes))
    {
        crate::content::bundle::install(index);
    }
    let options = eframe::NativeOptions {
        android_app: Some(app),
        viewport: eframe::egui::ViewportBuilder::default().with_app_id("dev.daijro.honeydew"),
        ..Default::default()
    };
    if let Err(err) = eframe::run_native(
        crate::APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(crate::app::HoneydewApp::new(cc)))),
    ) {
        eprintln!("Honeydew could not start: {err}");
    }
    crate::android_text::reset();
    *ACTIVITY.lock().expect("activity lock") = None;
}

/// [`with_activity`] for a call that blocks (a download): the activity lock is not held through it, and a global
/// reference keeps the activity object alive.
fn with_activity_unlocked(
    f: impl FnOnce(&mut jni::JNIEnv<'_>, &jni::objects::JObject<'_>) -> jni::errors::Result<()>,
) -> Result<(), String> {
    let (vm, activity) = {
        let held = ACTIVITY.lock().map_err(|err| err.to_string())?;
        let app = held.as_ref().ok_or("No Android Activity")?;
        // SAFETY: as in `with_activity`, while the lock is held.
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }
            .map_err(|err| err.to_string())?;
        let object = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
        let env = vm.attach_current_thread().map_err(|err| err.to_string())?;
        let global = env.new_global_ref(&object).map_err(|err| err.to_string())?;
        drop(env);
        (vm, global)
    };
    let mut env = vm.attach_current_thread().map_err(|err| err.to_string())?;
    let result = f(&mut env, activity.as_obj()).map_err(|err| err.to_string());
    if result.is_err() {
        let _ = env.exception_clear();
    }
    result
}

/// Runs a JNI call while the matching Activity remains alive.
pub fn with_activity(
    f: impl FnOnce(&mut jni::JNIEnv<'_>, &jni::objects::JObject<'_>) -> jni::errors::Result<()>,
) -> Result<(), String> {
    let activity = ACTIVITY.lock().map_err(|err| err.to_string())?;
    let app = activity.as_ref().ok_or("No Android Activity")?;
    // SAFETY: AndroidApp keeps the VM and this unowned global reference alive for the call.
    let vm =
        unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.map_err(|err| err.to_string())?;
    let object = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
    let mut env = vm.attach_current_thread().map_err(|err| err.to_string())?;
    let result = f(&mut env, &object).map_err(|err| err.to_string());
    if result.is_err() {
        let _ = env.exception_clear();
    }
    result
}

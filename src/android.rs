//! Android's NativeActivity entry point and access to app-private storage and APK assets.

use std::ffi::CString;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use android_activity::{AndroidApp, ndk::asset::Asset};

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

/// Android gives each install an absolute, private data directory; no storage permission is needed.
pub fn data_dir() -> Option<PathBuf> {
    ACTIVITY
        .lock()
        .ok()?
        .as_ref()?
        .internal_data_path()
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

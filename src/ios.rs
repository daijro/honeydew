//! iOS: the app's folders, the audio session, the microphone permission, the system document picker and share sheet,
//! through UIKit and AVFAudio (objc2). The app starts from `main.rs` as on the desktop: winit runs UIKit's application
//! there.

use std::cell::RefCell;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use block2::{Block, RcBlock};
use eframe::egui::SafeAreaInsets;
use eframe::egui::epaint::MarginF32;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, NSObjectProtocol};
use objc2::{ClassType, DeclaredClass, class, declare_class, msg_send, msg_send_id, mutability};
use objc2_foundation::{
    CGPoint, CGRect, CGSize, MainThreadMarker, NSArray, NSDictionary, NSNotification,
    NSNotificationCenter, NSString, NSURL,
};
use objc2_ui_kit::{UIDocumentPickerDelegate, UIDocumentPickerViewController};

use crate::audio_engine::RecordTap;
use crate::recording::input::{Mic, Opened};

// AVAudioSession's names. AVFoundation re-exports AVFAudio, which defines them.
#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVAudioSessionCategoryPlayback: &'static NSString;
    static AVAudioSessionCategoryPlayAndRecord: &'static NSString;
    static AVAudioSessionModeDefault: &'static NSString;
    static AVAudioSessionModeMeasurement: &'static NSString;
    static AVAudioSessionRouteChangeNotification: &'static NSString;
    static AVAudioSessionRouteChangeReasonKey: &'static NSString;
    static AVAudioSessionInterruptionNotification: &'static NSString;
    static AVAudioSessionInterruptionTypeKey: &'static NSString;
    static AVAudioSessionPortHeadphones: &'static NSString;
    static AVAudioSessionPortBluetoothA2DP: &'static NSString;
    static AVAudioSessionPortBluetoothHFP: &'static NSString;
    static AVAudioSessionPortBluetoothLE: &'static NSString;
    static AVAudioSessionPortUSBAudio: &'static NSString;
}

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    static UIApplicationOpenSettingsURLString: &'static NSString;
    static UIApplicationWillResignActiveNotification: &'static NSString;
    static UIApplicationDidBecomeActiveNotification: &'static NSString;
}

/// `AVAudioSessionCategoryOptions`: the speaker, not the earpiece, while recording; Bluetooth headphones play.
const DEFAULT_TO_SPEAKER: usize = 0x8;
const ALLOW_BLUETOOTH_A2DP: usize = 0x20;
/// `AVAudioSessionInterruptionTypeBegan`.
const INTERRUPTION_BEGAN: usize = 1;
/// `AVAudioSessionRouteChangeReason`: a device came or went (headphones, a USB interface).
const NEW_DEVICE: usize = 1;
const OLD_DEVICE_GONE: usize = 2;
/// The output buffer asked for: about 256 frames at 48 kHz.
const IO_BUFFER_SECS: f64 = 0.005;
/// The `NSError **` the session's setters take: their `BOOL` says whether they worked.
const NO_ERROR: *mut *mut AnyObject = std::ptr::null_mut();

/// The app's private folder, `Library/Application Support` in its container: the settings and the app data folder.
pub fn data_dir() -> Option<PathBuf> {
    Some(home()?.join("Library").join("Application Support"))
}

/// The app's `Documents` folder, which the Files app shows as the app's (`UIFileSharingEnabled`): the user folder
/// (`ui::main_menu::user_dir`).
pub fn documents_dir() -> Option<PathBuf> {
    Some(home()?.join("Documents")).filter(|dir| std::fs::create_dir_all(dir).is_ok())
}

/// The app's container, which iOS gives as `HOME`.
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn session() -> Retained<AnyObject> {
    unsafe { msg_send_id![class!(AVAudioSession), sharedInstance] }
}

/// Before the app's first frame: the playback session, and the route, interruption and app state observers.
pub fn start() {
    set_category(false);
    let session = session();
    let ok: Bool = unsafe {
        msg_send![&session, setPreferredIOBufferDuration: IO_BUFFER_SECS, error: NO_ERROR]
    };
    if !ok.as_bool() {
        eprintln!("the audio buffer size was not set");
    }
    HEADPHONES.store(route_has_headphones(), Ordering::Release);
    // Headphones (wired, USB or Bluetooth) in or out: the streams reopen. Not on the session's own category changes,
    // which the input's open and close make.
    observe(unsafe { AVAudioSessionRouteChangeNotification }, |note| {
        let reason = user_info(note, unsafe { AVAudioSessionRouteChangeReasonKey });
        let headphones = route_has_headphones();
        let changed = HEADPHONES.swap(headphones, Ordering::AcqRel) != headphones;
        if changed && matches!(reason, Some(NEW_DEVICE | OLD_DEVICE_GONE)) {
            REOPEN.store(true, Ordering::Release);
        }
    });
    observe(
        unsafe { AVAudioSessionInterruptionNotification },
        |note| match user_info(note, unsafe { AVAudioSessionInterruptionTypeKey })
            == Some(INTERRUPTION_BEGAN)
        {
            true => interrupt(),
            false => resume(),
        },
    );
    // As on Android, leaving the app ends a take, and the next frame writes the song's unsaved changes; the streams
    // start again on the way back.
    observe(unsafe { UIApplicationWillResignActiveNotification }, |_| {
        interrupt();
        crate::project_files::leave();
    });
    observe(unsafe { UIApplicationDidBecomeActiveNotification }, |_| {
        resume()
    });
}

/// As Android's `onInterrupted`: the take ends there and then, what it recorded stays ([`interrupted`]).
fn interrupt() {
    if let Some(tap) = RECORDING.lock().ok().as_deref().and_then(Option::as_ref) {
        tap.disarm();
        INTERRUPTED.store(true, Ordering::Release);
    }
}

/// The session active again, and the streams reopened ([`take_reopen`]).
fn resume() {
    activate();
    REOPEN.store(true, Ordering::Release);
}

/// The session's category: `.playback`, or while the input is open `.playAndRecord` on the speaker, with Bluetooth
/// headphones, in `.measurement` mode (no gain control or voice processing). Then the session is made active.
fn set_category(record: bool) {
    let (category, mode, options) = unsafe {
        match record {
            true => (
                AVAudioSessionCategoryPlayAndRecord,
                AVAudioSessionModeMeasurement,
                DEFAULT_TO_SPEAKER | ALLOW_BLUETOOTH_A2DP,
            ),
            false => (AVAudioSessionCategoryPlayback, AVAudioSessionModeDefault, 0),
        }
    };
    let session = session();
    let ok: Bool = unsafe {
        msg_send![&session, setCategory: category, mode: mode, options: options, error: NO_ERROR]
    };
    if !ok.as_bool() {
        eprintln!("the audio session category was not set (record {record})");
    }
    activate();
}

fn activate() {
    let ok: Bool = unsafe { msg_send![&session(), setActive: Bool::YES, error: NO_ERROR] };
    if !ok.as_bool() {
        eprintln!("the audio session did not start");
    }
}

/// `note`'s `userInfo` number at `key`.
fn user_info(note: &NSNotification, key: &NSString) -> Option<usize> {
    let info: Option<Retained<AnyObject>> = unsafe { msg_send_id![note, userInfo] };
    let number: Option<Retained<AnyObject>> = unsafe { msg_send_id![&*info?, objectForKey: key] };
    Some(unsafe { msg_send![&*number?, unsignedIntegerValue] })
}

/// Calls `handler` on each `name` notification, on the thread that posts it, for the app's life.
fn observe(name: &NSString, handler: impl Fn(&NSNotification) + 'static) {
    let block =
        RcBlock::new(move |note: NonNull<NSNotification>| handler(unsafe { note.as_ref() }));
    let center = unsafe { NSNotificationCenter::defaultCenter() };
    let observer = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
    };
    std::mem::forget(observer);
}

/// The output route's headphones (wired, USB, Bluetooth), as the last route change left it.
static HEADPHONES: AtomicBool = AtomicBool::new(false);

/// Whether headphones play the sound, not the speaker: monitoring may sound then (`recording::input::may_monitor`).
pub fn headphones() -> bool {
    HEADPHONES.load(Ordering::Acquire)
}

fn route_has_headphones() -> bool {
    let kinds = unsafe {
        [
            AVAudioSessionPortHeadphones,
            AVAudioSessionPortBluetoothA2DP,
            AVAudioSessionPortBluetoothHFP,
            AVAudioSessionPortBluetoothLE,
            AVAudioSessionPortUSBAudio,
        ]
    };
    let route: Retained<AnyObject> = unsafe { msg_send_id![&session(), currentRoute] };
    let outputs: Retained<NSArray<AnyObject>> = unsafe { msg_send_id![&route, outputs] };
    outputs.iter().any(|port| {
        let kind: Retained<NSString> = unsafe { msg_send_id![port, portType] };
        kinds.iter().any(|k| **k == *kind)
    })
}

/// A device came or went, or an interruption ended: the app reopens its output and input (taken once).
static REOPEN: AtomicBool = AtomicBool::new(false);

pub fn take_reopen() -> bool {
    REOPEN.swap(false, Ordering::AcqRel)
}

/// The tap of the recording under way, which an interruption ends from the notification's thread ([`interrupted`]).
static RECORDING: Mutex<Option<Arc<RecordTap>>> = Mutex::new(None);
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// The recording under way, `tap` (each frame): a call, Siri or an alarm ends its take there and then.
pub fn recording(tap: Option<&Arc<RecordTap>>) {
    let mut held = RECORDING.lock().expect("recording lock");
    if held.as_ref().map(Arc::as_ptr) != tap.map(Arc::as_ptr) {
        *held = tap.cloned();
        INTERRUPTED.store(false, Ordering::Release);
    }
}

/// An interruption ended the recording's take since the last call.
pub fn interrupted() -> bool {
    INTERRUPTED.swap(false, Ordering::AcqRel)
}

/// The microphone permission, as on Android: not asked (or a denial already told), asking, allowed, denied.
static MIC: AtomicU8 = AtomicU8::new(UNASKED);
const UNASKED: u8 = 0;
const ASKING: u8 = 1;
const GRANTED: u8 = 2;
const DENIED: u8 = 3;

/// `AVAudioApplicationRecordPermission` (iOS 17) and `AVAudioSessionRecordPermission`: the same four-character codes.
const PERMISSION_GRANTED: u32 = u32::from_be_bytes(*b"grnt");
const PERMISSION_DENIED: u32 = u32::from_be_bytes(*b"deny");

/// Whether the app may record. The first call asks iOS (`requestRecordPermission`), which shows its question once;
/// its answer comes back on another thread. A denial is told once; later calls read the stored answer, which only the
/// Settings app changes ([`open_settings`]).
pub fn mic() -> Mic {
    match MIC.load(Ordering::Acquire) {
        GRANTED => Mic::Granted,
        ASKING => Mic::Asking,
        DENIED => {
            let _ = MIC.compare_exchange(DENIED, UNASKED, Ordering::AcqRel, Ordering::Acquire);
            Mic::Denied
        }
        _ => match record_permission() {
            PERMISSION_GRANTED => {
                MIC.store(GRANTED, Ordering::Release);
                Mic::Granted
            }
            PERMISSION_DENIED => Mic::Denied,
            _ => {
                MIC.store(ASKING, Ordering::Release);
                request_record_permission();
                Mic::Asking
            }
        },
    }
}

/// The stored answer: `AVAudioApplication` from iOS 17, else `AVAudioSession`.
fn record_permission() -> u32 {
    match AnyClass::get("AVAudioApplication") {
        Some(class) => {
            let app: Retained<AnyObject> = unsafe { msg_send_id![class, sharedInstance] };
            let permission: isize = unsafe { msg_send![&app, recordPermission] };
            permission as u32
        }
        None => {
            let permission: usize = unsafe { msg_send![&session(), recordPermission] };
            permission as u32
        }
    }
}

fn request_record_permission() {
    let answered = RcBlock::new(|granted: Bool| {
        let answer = if granted.as_bool() { GRANTED } else { DENIED };
        MIC.store(answer, Ordering::Release);
    });
    match AnyClass::get("AVAudioApplication") {
        Some(class) => unsafe {
            let _: () = msg_send![class, requestRecordPermissionWithCompletionHandler: &*answered];
        },
        None => unsafe {
            let _: () = msg_send![&session(), requestRecordPermission: &*answered];
        },
    }
}

/// Opens the app's page in the Settings app, where the microphone is allowed.
pub fn open_settings() {
    open_url(unsafe { UIApplicationOpenSettingsURLString });
}

/// Opens `folder`, in the user folder, in the Files app.
pub fn show_folder(folder: &std::path::Path) -> Result<(), String> {
    let path = NSString::from_str(&folder.to_string_lossy());
    let url = unsafe { NSURL::fileURLWithPath(&path) };
    let text = unsafe { url.absoluteString() }
        .ok_or("no folder URL")?
        .to_string();
    let files = text.replacen("file://", "shareddocuments://", 1);
    open_url(&NSString::from_str(&files));
    Ok(())
}

fn open_url(text: &NSString) {
    let Some(url) = (unsafe { NSURL::URLWithString(text) }) else {
        return;
    };
    let options = NSDictionary::<NSString, AnyObject>::new();
    unsafe {
        let app: Retained<AnyObject> = msg_send_id![class!(UIApplication), sharedApplication];
        let _: () = msg_send![&app, openURL: &*url, options: &*options, completionHandler: None::<&Block<dyn Fn(Bool)>>];
    }
}

/// The input open on iOS: the session records while it is ([`set_category`]), and plays only again once it closes.
pub fn open_input(device: Option<&str>, rate: u32) -> Result<Opened, String> {
    set_category(true);
    let recording = Recording;
    let (ring, stream) = crate::recording::input::open_device(device, rate)?;
    // Dropped in order: the stream stops before the session plays only again.
    Ok((ring, Box::new((stream, recording))))
}

/// The session in `.playAndRecord` while it lives.
struct Recording;

impl Drop for Recording {
    fn drop(&mut self) {
        set_category(false);
    }
}

/// What a pick finishes with: the picked file or folder, or `None` when it was cancelled.
pub type Picked = Option<PathBuf>;
type Waiting = Option<Box<dyn FnOnce(Picked) + Send>>;

thread_local! {
    /// The picker's delegate (the picker holds it weakly) and where its pick goes.
    static PICKING: RefCell<(Option<Retained<PickerDelegate>>, Waiting)> = const { RefCell::new((None, None)) };
    /// The folder picked last, which the app reads while this holds it: its files are listed, then copied in.
    static FOLDER: RefCell<Option<Retained<NSURL>>> = const { RefCell::new(None) };
}

declare_class!(
    struct PickerDelegate;

    unsafe impl ClassType for PickerDelegate {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "HoneydewPickerDelegate";
    }

    // Ivars: a folder pick.
    impl DeclaredClass for PickerDelegate {
        type Ivars = bool;
    }

    unsafe impl NSObjectProtocol for PickerDelegate {}

    unsafe impl UIDocumentPickerDelegate for PickerDelegate {
        #[method(documentPicker:didPickDocumentsAtURLs:)]
        fn picked(&self, _picker: &UIDocumentPickerViewController, urls: &NSArray<NSURL>) {
            let url = unsafe { urls.firstObject() };
            if *self.ivars() {
                if let Some(url) = &url {
                    unsafe { url.startAccessingSecurityScopedResource() };
                }
                let old = FOLDER.with(|folder| folder.replace(url.clone()));
                if let Some(old) = old {
                    unsafe { old.stopAccessingSecurityScopedResource() };
                }
            }
            let path = url.and_then(|url| unsafe { url.path() });
            finish(path.map(|path| PathBuf::from(path.to_string())));
        }

        #[method(documentPickerWasCancelled:)]
        fn cancelled(&self, _picker: &UIDocumentPickerViewController) {
            finish(None);
        }
    }
);

/// Hands the pick on. The delegate stays until the next pick replaces it: it is the one running.
fn finish(picked: Picked) {
    let done = PICKING.with(|p| p.borrow_mut().1.take());
    if let Some(done) = done {
        done(picked);
    }
}

/// Opens the system document picker for a file (copied into the app's `tmp`, which the caller removes once read) or,
/// with `folder`, a folder (read in place while [`FOLDER`] holds it). `done` gets the pick on the main thread.
pub fn pick(folder: bool, done: impl FnOnce(Picked) + Send + 'static) {
    let Some(mtm) = MainThreadMarker::new() else {
        return done(None);
    };
    let kind = NSString::from_str(if folder {
        "public.folder"
    } else {
        "public.item"
    });
    let kinds = NSArray::from_vec(vec![kind]);
    // `initWithDocumentTypes:inMode:`: Import copies the file in, Open lends a folder.
    #[allow(deprecated)]
    let picker = unsafe {
        let mode = match folder {
            true => objc2_ui_kit::UIDocumentPickerMode::Open,
            false => objc2_ui_kit::UIDocumentPickerMode::Import,
        };
        UIDocumentPickerViewController::initWithDocumentTypes_inMode(mtm.alloc(), &kinds, mode)
    };
    let delegate: Retained<PickerDelegate> = {
        let this = mtm.alloc::<PickerDelegate>().set_ivars(folder);
        unsafe { msg_send_id![super(this), init] }
    };
    unsafe { picker.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*delegate))) };
    let root = key_window().and_then(|window| unsafe { msg_send_id![&window, rootViewController] });
    let Some(root): Option<Retained<AnyObject>> = root else {
        return done(None);
    };
    // A pick still open answers nothing.
    let earlier = PICKING.with(|p| p.replace((Some(delegate), Some(Box::new(done)))));
    if let (_, Some(earlier)) = earlier {
        earlier(None);
    }
    unsafe {
        let _: () = msg_send![&root, presentViewController: &*picker, animated: Bool::YES, completion: None::<&Block<dyn Fn()>>];
    }
}

/// `Send` (#72): `files` in the system's share sheet (`UIActivityViewController`, which reads their types from their
/// names), over the key window's root view controller; an iPad shows it as a popover from the middle of the screen.
pub fn share(files: &[PathBuf], _mime: &str) -> Result<(), String> {
    MainThreadMarker::new().ok_or("Send runs on the main thread")?;
    let urls = (files.iter())
        .map(|file| unsafe { NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy())) })
        .collect();
    let items = NSArray::from_vec(urls);
    let root = key_window().and_then(|window| unsafe { msg_send_id![&window, rootViewController] });
    let root: Retained<AnyObject> = root.ok_or("There is no window to share from")?;
    unsafe {
        let sheet: Allocated<AnyObject> = msg_send_id![class!(UIActivityViewController), alloc];
        let none = None::<&NSArray<AnyObject>>;
        let sheet: Retained<AnyObject> =
            msg_send_id![sheet, initWithActivityItems: &*items, applicationActivities: none];
        let popover: Option<Retained<AnyObject>> =
            msg_send_id![&sheet, popoverPresentationController];
        if let Some(popover) = popover {
            let view: Retained<AnyObject> = msg_send_id![&root, view];
            let bounds: CGRect = msg_send![&view, bounds];
            let middle = CGPoint::new(bounds.size.width / 2.0, bounds.size.height / 2.0);
            let _: () = msg_send![&popover, setSourceView: &*view];
            let _: () =
                msg_send![&popover, setSourceRect: CGRect::new(middle, CGSize::new(0.0, 0.0))];
            // `UIPopoverArrowDirection`: none.
            let _: () = msg_send![&popover, setPermittedArrowDirections: 0usize];
        }
        let _: () = msg_send![&root, presentViewController: &*sheet, animated: Bool::YES, completion: None::<&Block<dyn Fn()>>];
    }
    Ok(())
}

/// The app's window (winit's one).
fn key_window() -> Option<Retained<AnyObject>> {
    unsafe {
        let app: Retained<AnyObject> = msg_send_id![class!(UIApplication), sharedApplication];
        msg_send_id![&app, keyWindow]
    }
}

/// `UIInterfaceOrientationMaskLandscape`.
const LANDSCAPE: usize = 24;

/// Turns the app to landscape as turning the phone would (iOS 16 or later): the self-test's.
pub fn turn_landscape() {
    let Some(class) = AnyClass::get("UIWindowSceneGeometryPreferencesIOS") else {
        return;
    };
    let scene: Option<Retained<AnyObject>> =
        key_window().and_then(|window| unsafe { msg_send_id![&window, windowScene] });
    let Some(scene) = scene else { return };
    unsafe {
        let preferences: Allocated<AnyObject> = msg_send_id![class, alloc];
        let preferences: Retained<AnyObject> =
            msg_send_id![preferences, initWithInterfaceOrientations: LANDSCAPE];
        let none = None::<&Block<dyn Fn(NonNull<AnyObject>)>>;
        let _: () = msg_send![&scene, requestGeometryUpdateWithPreferences: &*preferences, errorHandler: none];
    }
}

/// UIKit's safe area (`insets`) kept on the notch's side only: elsewhere the app reaches the screen's edges, under the
/// home indicator too, as on Android. In landscape UIKit insets both sides alike; the orientation tells which one has
/// the notch.
pub fn notch_only(insets: SafeAreaInsets) -> SafeAreaInsets {
    let (inset, none) = (insets.0, MarginF32::ZERO);
    SafeAreaInsets(match interface_orientation() {
        // `UIInterfaceOrientationLandscapeRight`: the home button's side right, the notch left.
        LANDSCAPE_RIGHT => MarginF32 {
            left: inset.left,
            ..none
        },
        LANDSCAPE_LEFT => MarginF32 {
            right: inset.right,
            ..none
        },
        UPSIDE_DOWN => MarginF32 {
            bottom: inset.bottom,
            ..none
        },
        _ => MarginF32 {
            top: inset.top,
            ..none
        },
    })
}

/// `UIInterfaceOrientation`'s.
const UPSIDE_DOWN: isize = 2;
const LANDSCAPE_RIGHT: isize = 3;
const LANDSCAPE_LEFT: isize = 4;

/// The window scene's `interfaceOrientation`, portrait (1) when unknown.
fn interface_orientation() -> isize {
    let scene: Option<Retained<AnyObject>> =
        key_window().and_then(|window| unsafe { msg_send_id![&window, windowScene] });
    scene.map_or(1, |scene| unsafe {
        msg_send![&scene, interfaceOrientation]
    })
}

/// The screen's size in points, as it is turned: the self-test's check that the app covers it.
pub fn screen_size() -> eframe::egui::Vec2 {
    let bounds: CGRect = unsafe {
        let screen: Retained<AnyObject> = msg_send_id![class!(UIScreen), mainScreen];
        msg_send![&screen, bounds]
    };
    eframe::egui::vec2(bounds.size.width as f32, bounds.size.height as f32)
}

// System Safari UI stays inside the foreground app, allowing its loopback OAuth listener to complete on iPad.
#[link(name = "SafariServices", kind = "framework")]
unsafe extern "C" {}
thread_local! {
    static AUTH_BROWSER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}
pub fn present_auth_browser(url: &str) -> Result<(), String> {
    let _main = MainThreadMarker::new().ok_or("Open sign-in on the main thread.")?;
    let root = key_window().and_then(|window| unsafe { msg_send_id![&window, rootViewController] });
    let mut root: Retained<AnyObject> = root.ok_or("No window is available for sign-in.")?;
    loop {
        let next: Option<Retained<AnyObject>> =
            unsafe { msg_send_id![&root, presentedViewController] };
        if let Some(next) = next {
            root = next;
        } else {
            break;
        }
    }
    let url =
        unsafe { NSURL::URLWithString(&NSString::from_str(url)) }.ok_or("Invalid sign-in URL.")?;
    let allocated: Allocated<AnyObject> =
        unsafe { msg_send_id![class!(SFSafariViewController), alloc] };
    let browser: Retained<AnyObject> = unsafe { msg_send_id![allocated, initWithURL: &*url] };
    unsafe {
        let _: () = msg_send![&root, presentViewController: &*browser, animated: Bool::YES, completion: None::<&Block<dyn Fn()>>];
    }
    AUTH_BROWSER.with(|slot| *slot.borrow_mut() = Some(browser));
    Ok(())
}
pub fn dismiss_auth_browser() {
    if MainThreadMarker::new().is_none() {
        return;
    }
    AUTH_BROWSER.with(|slot| if let Some(browser)=slot.borrow_mut().take() {
        unsafe { let _: () = msg_send![&browser, dismissViewControllerAnimated: Bool::YES, completion: None::<&Block<dyn Fn()>>]; }
    });
}

//! Optional single-scene lifecycle for apps whose Info.plist selects WinitSceneDelegate.
//! Legacy application-delegate apps retain the existing startup path.
use super::window::WinitUIWindow;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{class, declare_class, msg_send_id, mutability, ClassType, DeclaredClass};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSString};
use objc2_ui_kit::{
    UIScene, UISceneConnectionOptions, UISceneDelegate, UISceneSession, UIWindowScene,
    UIWindowSceneDelegate,
};
use std::cell::RefCell;

thread_local! {
    static SCENE: RefCell<Option<Retained<UIWindowScene>>> = const { RefCell::new(None) };
    static PENDING: RefCell<Vec<(Retained<WinitUIWindow>, bool)>> = const { RefCell::new(Vec::new()) };
}

declare_class!(
    pub(crate) struct WinitSceneDelegate;
    unsafe impl ClassType for WinitSceneDelegate {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "WinitSceneDelegate";
    }
    impl DeclaredClass for WinitSceneDelegate {}
    unsafe impl NSObjectProtocol for WinitSceneDelegate {}
    unsafe impl UISceneDelegate for WinitSceneDelegate {
        #[method(scene:willConnectToSession:options:)]
        fn connect(&self, scene: &UIScene, _session: &UISceneSession, _options: &UISceneConnectionOptions) {
            if !scene.isKindOfClass(UIWindowScene::class()) { return; }
            // SAFETY: the runtime class was checked above; UIKit owns the scene.
            let scene = unsafe { &*(scene as *const UIScene as *const UIWindowScene) };
            let retained = unsafe { Retained::retain(scene as *const UIWindowScene as *mut UIWindowScene) }.unwrap();
            SCENE.with(|slot| *slot.borrow_mut() = Some(retained));
            let pending = PENDING.with(|windows| std::mem::take(&mut *windows.borrow_mut()));
            for (window, visible) in pending {
                // SAFETY: UIKit supplied this window scene on the main thread.
                unsafe { window.setWindowScene(Some(scene)); }
                if visible { window.makeKeyAndVisible(); }
            }
        }
        #[method(sceneDidDisconnect:)]
        fn disconnect(&self, _scene: &UIScene) {
            SCENE.with(|slot| *slot.borrow_mut() = None);
        }
    }
    unsafe impl UIWindowSceneDelegate for WinitSceneDelegate {}
);

pub(crate) fn register_class() {
    let _ = WinitSceneDelegate::class();
}

pub(crate) fn attach(window: &Retained<WinitUIWindow>, visible: bool, _mtm: MainThreadMarker) {
    // Only opt into the scene path for an app that supplied a scene manifest.
    let bundle: Retained<NSObject> = unsafe { msg_send_id![class!(NSBundle), mainBundle] };
    let manifest: Option<Retained<NSObject>> = unsafe {
        msg_send_id![&bundle, objectForInfoDictionaryKey: &*NSString::from_str("UIApplicationSceneManifest")]
    };
    if manifest.is_none() {
        return;
    }
    let scene = SCENE.with(|slot| slot.borrow().clone());
    if let Some(scene) = scene {
        // SAFETY: the scene was retained from UIKit on this same main thread.
        unsafe {
            window.setWindowScene(Some(&scene));
        }
    } else {
        PENDING.with(|windows| windows.borrow_mut().push((window.clone(), visible)));
    }
}

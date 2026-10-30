// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio
// Modified for Honeydew: the X11 window of Lemon Studio's plugin windows (os-app `plugin_window/x11.rs`) without its
// frame: a plain top-level window the window manager decorates, which the plugin's editor is embedded in.

//! A plugin editor's own window. Plugin editors on Linux embed only in X11 windows (CLAP `x11`, VST3
//! `X11EmbedWindowID`), so the window is an X11 one (XWayland under Wayland), top-level and decorated by the window
//! manager, sized to the editor. One connection serves every window (each plugin opens its own for its editor); the
//! app reads its events every frame ([`pump`]) and closes an editor whose window was closed.
//!
//! Windows uses a small Win32 host window and macOS uses an AppKit window whose content view is passed to the plugin.

use std::ffi::c_void;

#[cfg(windows)]
mod win {
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    // Handles as integers: `HWND` is a raw pointer, which a static cannot hold.
    static WINDOWS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    static CLOSED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    const CLASS: &[u16] = &[
        b'H' as u16,
        b'o' as u16,
        b'n' as u16,
        b'e' as u16,
        b'y' as u16,
        b'd' as u16,
        b'e' as u16,
        b'w' as u16,
        b'P' as u16,
        b'l' as u16,
        b'u' as u16,
        b'g' as u16,
        b'i' as u16,
        b'n' as u16,
        b'E' as u16,
        b'd' as u16,
        b'i' as u16,
        b't' as u16,
        b'o' as u16,
        b'r' as u16,
        0,
    ];

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if msg == WM_CLOSE {
            if let Ok(mut closed) = CLOSED.lock() {
                closed.push(hwnd as usize);
            }
            unsafe { DestroyWindow(hwnd) };
            return 0;
        }
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }

    pub fn create(title: &str) -> Result<HWND, String> {
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance,
                lpszClassName: CLASS.as_ptr(),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                ..std::mem::zeroed()
            };
            RegisterClassW(&class);
            let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
            let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
            let hwnd = CreateWindowExW(
                0,
                CLASS.as_ptr(),
                title.as_ptr(),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                400,
                300,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return Err(format!(
                    "could not create the plugin window (Win32 error {})",
                    GetLastError()
                ));
            }
            WINDOWS.lock().unwrap().push(hwnd as usize);
            Ok(hwnd)
        }
    }

    pub fn resize(hwnd: HWND, (w, h): (u32, u32)) {
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: w.min(16384) as i32,
                bottom: h.min(16384) as i32,
            };
            AdjustWindowRectEx(&mut rect, style, 0, ex);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    pub fn show(hwnd: HWND) {
        unsafe {
            ShowWindow(hwnd, SW_SHOWNORMAL);
            SetForegroundWindow(hwnd);
        }
    }
    pub fn destroy(hwnd: HWND) {
        unsafe {
            DestroyWindow(hwnd);
        }
        if let Ok(mut windows) = WINDOWS.lock() {
            windows.retain(|&w| w != hwnd as usize);
        }
    }
    pub fn pump() -> Vec<usize> {
        let windows = WINDOWS.lock().map(|w| w.clone()).unwrap_or_default();
        unsafe {
            for hwnd in windows {
                let mut msg = std::mem::zeroed();
                while PeekMessageW(&mut msg, hwnd as HWND, 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            // Issue #33: the thread's timers too, which most editors redraw from (their own windows' and window-less
            // ones). Windows makes a WM_TIMER only while no paint is pending, and the app's window has one pending
            // whenever it redraws every frame (as it does while an editor is open): without this, an editor drew only
            // when the mouse moved over it.
            let mut msg = std::mem::zeroed();
            while PeekMessageW(
                &mut msg,
                std::ptr::null_mut(),
                WM_TIMER,
                WM_TIMER,
                PM_REMOVE,
            ) != 0
            {
                DispatchMessageW(&msg);
            }
        }
        CLOSED
            .lock()
            .map(|mut c| std::mem::take(&mut *c))
            .unwrap_or_default()
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::sync::Mutex;

    pub struct Native {
        pub window: Retained<NSWindow>,
        pub view: Retained<NSView>,
        pub shown: bool,
    }
    static WINDOWS: Mutex<Vec<(usize, usize, bool)>> = Mutex::new(Vec::new());

    pub fn create(title: &str) -> Result<Native, String> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Err("plugin editor windows must open on the macOS main thread".into());
        };
        let rect = NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(400.0, 300.0));
        // SAFETY: a fresh allocation on the main thread; the window is not released when closed (below), so the
        // `Retained` stays the only owner.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&NSString::from_str(title));
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        let view = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), rect.size),
        );
        window.setContentView(Some(&view));
        let native = Native {
            window,
            view,
            shown: false,
        };
        WINDOWS.lock().unwrap().push((
            Retained::as_ptr(&native.window) as usize,
            Retained::as_ptr(&native.view) as usize,
            false,
        ));
        Ok(native)
    }
    pub fn resize(native: &Native, (w, h): (u32, u32)) {
        native
            .window
            .setContentSize(NSSize::new(f64::from(w.max(1)), f64::from(h.max(1))));
    }
    pub fn show(native: &mut Native) {
        native.shown = true;
        if let Ok(mut windows) = WINDOWS.lock() {
            if let Some(w) = windows
                .iter_mut()
                .find(|w| w.0 == Retained::as_ptr(&native.window) as usize)
            {
                w.2 = true;
            }
        }
        native.window.makeKeyAndOrderFront(None);
    }
    pub fn destroy(native: &Native) {
        native.window.orderOut(None);
        native.window.close();
        if let Ok(mut windows) = WINDOWS.lock() {
            windows.retain(|w| w.0 != Retained::as_ptr(&native.window) as usize);
        }
    }
    pub fn scale(native: &Native) -> f32 {
        native.window.backingScaleFactor() as f32
    }
    pub fn pump() -> Vec<usize> {
        let Some(_) = MainThreadMarker::new() else {
            return Vec::new();
        };
        WINDOWS
            .lock()
            .map(|windows| {
                windows
                    .iter()
                    .filter(|w| w.2 && !unsafe { (&*(w.0 as *mut NSWindow)).isVisible() })
                    .map(|w| w.1)
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(x11)]
mod x11 {
    use std::cell::RefCell;
    use std::rc::Rc;

    use x11rb::connection::Connection;
    use x11rb::properties::WmSizeHints;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{
        self, AtomEnum, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, EventMask,
        PropMode, StackMode, WindowClass,
    };
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    x11rb::atom_manager! {
        pub Atoms: AtomsCookie {
            WM_PROTOCOLS,
            WM_DELETE_WINDOW,
            _NET_WM_PING,
            _NET_WM_NAME,
            _NET_WM_PID,
            UTF8_STRING,
        }
    }

    pub struct Conn {
        pub c: RustConnection,
        pub root: xproto::Window,
        pub atoms: Atoms,
        /// Device pixels per logical one: the X resources' `Xft.dpi` / 96.
        pub scale: f32,
    }

    thread_local! {
        static CONN: RefCell<Option<Rc<Conn>>> = const { RefCell::new(None) };
    }

    /// The connection, opened on first use (`DISPLAY`: XWayland under Wayland).
    pub fn conn() -> Result<Rc<Conn>, String> {
        if let Some(c) = CONN.with(|c| c.borrow().clone()) {
            return Ok(c);
        }
        let err = |e: &dyn std::fmt::Display| {
            format!("plugin windows need an X11 display (XWayland under Wayland): {e}")
        };
        let (c, screen) = RustConnection::connect(None).map_err(|e| err(&e))?;
        let root = c.setup().roots[screen].root;
        let atoms = Atoms::new(&c)
            .map_err(|e| err(&e))?
            .reply()
            .map_err(|e| err(&e))?;
        let dpi = x11rb::resource_manager::new_from_default(&c)
            .ok()
            .and_then(|db| db.get_value::<u32>("Xft.dpi", "").ok().flatten());
        let scale = dpi.filter(|&d| d >= 96).map_or(1.0, |d| d as f32 / 96.0);
        let conn = Rc::new(Conn {
            c,
            root,
            atoms,
            scale,
        });
        CONN.with(|c| *c.borrow_mut() = Some(conn.clone()));
        Ok(conn)
    }

    /// The window manager keeps the window at this size: the window follows its editor.
    pub fn fixed_size(c: &RustConnection, win: xproto::Window, w: u16, h: u16) {
        let mut hints = WmSizeHints::new();
        hints.min_size = Some((i32::from(w), i32::from(h)));
        hints.max_size = Some((i32::from(w), i32::from(h)));
        let _ = hints.set_normal_hints(c, win);
    }

    pub fn create(conn: &Conn, title: &str) -> Result<xproto::Window, String> {
        let fail = |e: &dyn std::fmt::Display| format!("could not create the plugin window: {e}");
        let (c, a) = (&conn.c, &conn.atoms);
        let win = c.generate_id().map_err(|e| fail(&e))?;
        let made = (|| -> Result<(), Box<dyn std::error::Error>> {
            let events = EventMask::STRUCTURE_NOTIFY;
            let aux = CreateWindowAux::new()
                .event_mask(events)
                .background_pixel(0);
            c.create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win,
                conn.root,
                0,
                0,
                400,
                300,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &aux,
            )?;
            c.change_property8(
                PropMode::REPLACE,
                win,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                title.as_bytes(),
            )?;
            c.change_property8(
                PropMode::REPLACE,
                win,
                a._NET_WM_NAME,
                a.UTF8_STRING,
                title.as_bytes(),
            )?;
            // The desktop entry's id (`main.rs` `with_app_id`): the window is grouped with the app.
            c.change_property8(
                PropMode::REPLACE,
                win,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                b"honeydew\0honeydew\0",
            )?;
            c.change_property32(
                PropMode::REPLACE,
                win,
                a.WM_PROTOCOLS,
                AtomEnum::ATOM,
                &[a.WM_DELETE_WINDOW, a._NET_WM_PING],
            )?;
            c.change_property32(
                PropMode::REPLACE,
                win,
                a._NET_WM_PID,
                AtomEnum::CARDINAL,
                &[std::process::id()],
            )?;
            // The plugin parents its editor into the window through its own connection: it must exist on the server.
            c.flush()?;
            Ok(())
        })();
        if let Err(e) = made {
            let _ = c.destroy_window(win);
            let _ = c.flush();
            return Err(fail(&e));
        }
        Ok(win)
    }

    pub fn resize(conn: &Conn, win: xproto::Window, (w, h): (u32, u32)) {
        let (w, h) = (w.clamp(1, 16384) as u16, h.clamp(1, 16384) as u16);
        fixed_size(&conn.c, win, w, h);
        let aux = ConfigureWindowAux::new()
            .width(u32::from(w))
            .height(u32::from(h));
        let _ = conn.c.configure_window(win, &aux);
        let _ = conn.c.flush();
    }

    pub fn show(conn: &Conn, win: xproto::Window) {
        let _ = conn.c.map_window(win);
        let _ = conn
            .c
            .configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        let _ = conn.c.flush();
    }

    pub fn destroy(conn: &Conn, win: xproto::Window) {
        let _ = conn.c.destroy_window(win);
        let _ = conn.c.flush();
    }

    /// Reads the connection's events: the windows the window manager asked to close; pings are answered.
    pub fn pump() -> Vec<xproto::Window> {
        let Some(conn) = CONN.with(|c| c.borrow().clone()) else {
            return Vec::new();
        };
        let (c, a) = (&conn.c, &conn.atoms);
        let mut closed = Vec::new();
        while let Ok(Some(event)) = c.poll_for_event() {
            if let Event::ClientMessage(m) = event
                && m.type_ == a.WM_PROTOCOLS
            {
                let [atom, ..] = m.data.as_data32();
                if atom == a.WM_DELETE_WINDOW {
                    closed.push(m.window);
                } else if atom == a._NET_WM_PING {
                    let mut pong = m;
                    pong.window = conn.root;
                    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
                    let _ = c.send_event(false, conn.root, mask, pong);
                }
            }
        }
        let _ = c.flush();
        closed
    }
}

/// What plugin editor code needs on the app thread. The app draws with OpenGL, and its context stays current on this
/// thread; an editor that draws with GLX is refused its own `glXMakeCurrent` while another API's (EGL's, the app's on
/// Wayland) context is current here, with an X error. Xlib's default error handler then exits the whole app, so the
/// app's context is released around editor calls and plugins' X errors go to a handler that only reports them.
#[cfg(x11)]
mod editor_env {
    use std::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};

    type Ptr = *mut c_void;

    /// The first of `names` that opens; with `load` false, only a library the process has loaded already.
    fn open(names: &[&CStr], load: bool) -> Option<Ptr> {
        let flags = libc::RTLD_NOW | if load { 0 } else { libc::RTLD_NOLOAD };
        // SAFETY: dlopen of a system library by name; these run no constructors of ours.
        (names.iter())
            .find_map(|n| Some(unsafe { libc::dlopen(n.as_ptr(), flags) }).filter(|h| !h.is_null()))
    }

    /// Symbol `name` of `lib` as the function pointer type `F`.
    ///
    /// # Safety
    /// `F` must be the symbol's C signature.
    unsafe fn sym<F: Copy>(lib: Ptr, name: &CStr) -> Option<F> {
        // SAFETY: the caller names the symbol's own signature; a found symbol is a function of the library.
        let p = unsafe { libc::dlsym(lib, name.as_ptr()) };
        (!p.is_null()).then(|| unsafe { std::mem::transmute_copy::<Ptr, F>(&p) })
    }

    /// One API's calls about the context current on this thread.
    enum Gl {
        Egl {
            context: unsafe extern "C" fn() -> Ptr,
            display: unsafe extern "C" fn() -> Ptr,
            surface: unsafe extern "C" fn(c_int) -> Ptr,
            make: unsafe extern "C" fn(Ptr, Ptr, Ptr, Ptr) -> c_uint,
        },
        Glx {
            context: unsafe extern "C" fn() -> Ptr,
            display: unsafe extern "C" fn() -> Ptr,
            draw: unsafe extern "C" fn() -> c_ulong,
            read: unsafe extern "C" fn() -> c_ulong,
            make: unsafe extern "C" fn(Ptr, c_ulong, c_ulong, Ptr) -> c_int,
        },
    }

    impl Gl {
        fn egl() -> Option<Self> {
            let lib = open(&[c"libEGL.so.1"], false)?;
            // SAFETY: the EGL 1.4 signatures.
            unsafe {
                Some(Self::Egl {
                    context: sym(lib, c"eglGetCurrentContext")?,
                    display: sym(lib, c"eglGetCurrentDisplay")?,
                    surface: sym(lib, c"eglGetCurrentSurface")?,
                    make: sym(lib, c"eglMakeCurrent")?,
                })
            }
        }

        fn glx() -> Option<Self> {
            let lib = open(&[c"libGLX.so.0", c"libGL.so.1"], false)?;
            // SAFETY: the GLX 1.3 signatures.
            unsafe {
                Some(Self::Glx {
                    context: sym(lib, c"glXGetCurrentContext")?,
                    display: sym(lib, c"glXGetCurrentDisplay")?,
                    draw: sym(lib, c"glXGetCurrentDrawable")?,
                    read: sym(lib, c"glXGetCurrentReadDrawable")?,
                    make: sym(lib, c"glXMakeContextCurrent")?,
                })
            }
        }

        /// The (display, draw surface, read surface, context) current on this thread, if a context is.
        fn current(&self) -> Option<[usize; 4]> {
            // SAFETY: queries about the calling thread's current context.
            let now = unsafe {
                match self {
                    // EGL_DRAW, EGL_READ.
                    Self::Egl {
                        context,
                        display,
                        surface,
                        ..
                    } => {
                        [display(), surface(0x3059), surface(0x305A), context()].map(|p| p as usize)
                    }
                    Self::Glx {
                        context,
                        display,
                        draw,
                        read,
                        ..
                    } => [
                        display() as usize,
                        draw() as usize,
                        read() as usize,
                        context() as usize,
                    ],
                }
            };
            (now[0] != 0 && now[3] != 0).then_some(now)
        }

        /// Makes `[display, draw, read, context]` current on this thread; zeros but the display release.
        fn make_current(&self, [d, w, r, c]: [usize; 4]) -> bool {
            // SAFETY: values `current` read (or zeros, which release) for the API's own call.
            unsafe {
                match self {
                    Self::Egl { make, .. } => make(d as Ptr, w as Ptr, r as Ptr, c as Ptr) != 0,
                    Self::Glx { make, .. } => {
                        make(d as Ptr, w as c_ulong, r as c_ulong, c as Ptr) != 0
                    }
                }
            }
        }

        fn release(&self) {
            if let Some([display, ..]) = self.current() {
                self.make_current([display, 0, 0, 0]);
            }
        }
    }

    // SAFETY: function pointers into thread-safe libraries; each call is about the calling thread's own context.
    unsafe impl Send for Gl {}
    unsafe impl Sync for Gl {}

    /// EGL's and GLX's calls, once something has loaded the library (until then no context of it can be current).
    fn apis() -> [Option<&'static Gl>; 2] {
        static EGL: OnceLock<Gl> = OnceLock::new();
        static GLX: OnceLock<Gl> = OnceLock::new();
        let get = |cell: &'static OnceLock<Gl>, load: fn() -> Option<Gl>| match cell.get() {
            Some(gl) => Some(gl),
            None => load().map(|gl| cell.get_or_init(|| gl)),
        };
        [get(&EGL, Gl::egl), get(&GLX, Gl::glx)]
    }

    /// Runs `f` with no GL context current on this thread, then makes the one that was (the app's) current again.
    /// Whatever `f` left current (a plugin's) is released first: the app's EGL context could not come back over it.
    pub fn without_app_gl<R>(f: impl FnOnce() -> R) -> R {
        let saved = apis().map(|api| api.and_then(|gl| Some((gl, gl.current()?))));
        for (gl, _) in saved.iter().flatten() {
            gl.release();
        }
        let out = f();
        // `f` may have loaded GLX first.
        for gl in apis().iter().flatten() {
            gl.release();
        }
        for (gl, context) in saved.iter().flatten() {
            if !gl.make_current(*context) {
                let _ = writeln!(
                    std::io::stderr(),
                    "plugins: the app's GL context could not be made current again"
                );
            }
        }
        out
    }

    /// Xlib's `XErrorEvent`.
    #[repr(C)]
    struct XErrorEvent {
        kind: c_int,
        display: Ptr,
        resource: c_ulong,
        serial: c_ulong,
        error_code: u8,
        request_code: u8,
        minor_code: u8,
    }

    type ErrorHandler = unsafe extern "C" fn(Ptr, *mut XErrorEvent) -> c_int;

    struct Xlib {
        error_text: unsafe extern "C" fn(Ptr, c_int, *mut c_char, c_int) -> c_int,
        /// The handler in place before [`on_x_error`] unless it was Xlib's own (winit's under X11): it is told too.
        before: Option<ErrorHandler>,
    }

    static XLIB: OnceLock<Option<Xlib>> = OnceLock::new();
    /// The first X error since [`take_x_error`], as Xlib words it.
    static X_ERROR: Mutex<Option<String>> = Mutex::new(None);

    /// Reports an X error and lets the app go on (Xlib's own handler exits).
    unsafe extern "C" fn on_x_error(display: Ptr, event: *mut XErrorEvent) -> c_int {
        // SAFETY: Xlib passes a valid event for the call.
        let e = unsafe { &*event };
        let mut buf = [0 as c_char; 160];
        if let Some(Some(x)) = XLIB.get() {
            // SAFETY: a buffer of its length; local text lookup, no request.
            unsafe {
                (x.error_text)(
                    display,
                    c_int::from(e.error_code),
                    buf.as_mut_ptr(),
                    buf.len() as c_int,
                )
            };
        }
        // SAFETY: XGetErrorText terminates the text within the buffer (it was zeroed if it did not run).
        let text = unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let text = if text.is_empty() {
            format!("error {}", e.error_code)
        } else {
            text
        };
        let _ = writeln!(
            std::io::stderr(),
            "X error {text} (request {}.{}, resource {:#x}, serial {}): ignored",
            e.request_code,
            e.minor_code,
            e.resource,
            e.serial
        );
        if let Ok(mut slot) = X_ERROR.lock() {
            slot.get_or_insert(text);
        }
        if let Some(Some(Xlib {
            before: Some(before),
            ..
        })) = XLIB.get()
        {
            // SAFETY: the handler Xlib had, with Xlib's own arguments.
            unsafe { before(display, event) };
        }
        0
    }

    /// Puts [`on_x_error`] in as Xlib's error handler, once, before any plugin is loaded: a plugin that puts its own in
    /// later handles its own errors (and puts this one back when it goes).
    pub fn catch_x_errors() {
        XLIB.get_or_init(|| {
            let lib = open(&[c"libX11.so.6"], true)?;
            // SAFETY: the Xlib signatures; a handler of Xlib's signature that lives as long as the program.
            unsafe {
                let set: unsafe extern "C" fn(Option<ErrorHandler>) -> Option<ErrorHandler> =
                    sym(lib, c"XSetErrorHandler")?;
                let error_text = sym(lib, c"XGetErrorText")?;
                let xlib_own: Option<ErrorHandler> = sym(lib, c"_XDefaultError");
                let before = set(Some(on_x_error))
                    .filter(|&h| !xlib_own.is_some_and(|own| std::ptr::fn_addr_eq(h, own)));
                Some(Xlib { error_text, before })
            }
        });
    }

    /// The first X error since the last call.
    pub fn take_x_error() -> Option<String> {
        X_ERROR.lock().ok()?.take()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The app's EGL context (as on Wayland) is not current while editor code runs, and is current again after,
        /// over a context the editor code left current. Skipped without Mesa's surfaceless EGL.
        #[test]
        fn editor_code_runs_without_the_apps_gl_context() {
            let Some(lib) = open(&[c"libEGL.so.1"], true) else {
                return;
            };
            // SAFETY: the EGL 1.5 signatures; a surfaceless display and contexts without a config (Mesa's extensions).
            unsafe {
                let get_display: unsafe extern "C" fn(c_uint, Ptr, *const isize) -> Ptr =
                    sym(lib, c"eglGetPlatformDisplay").unwrap();
                let initialize: unsafe extern "C" fn(Ptr, *mut c_int, *mut c_int) -> c_uint =
                    sym(lib, c"eglInitialize").unwrap();
                let create: unsafe extern "C" fn(Ptr, Ptr, Ptr, *const c_int) -> Ptr =
                    sym(lib, c"eglCreateContext").unwrap();
                const SURFACELESS_MESA: c_uint = 0x31DD;
                let display = get_display(SURFACELESS_MESA, std::ptr::null_mut(), std::ptr::null());
                if display.is_null()
                    || initialize(display, std::ptr::null_mut(), std::ptr::null_mut()) == 0
                {
                    return;
                }
                // EGL_CONTEXT_MAJOR_VERSION 2 (the default API is OpenGL ES).
                let attribs = [0x3098, 2, 0x3038];
                let app = create(
                    display,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    attribs.as_ptr(),
                );
                let plugin = create(
                    display,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    attribs.as_ptr(),
                );
                let egl = Gl::egl().unwrap();
                let app_bound = [display as usize, 0, 0, app as usize];
                if app.is_null() || plugin.is_null() || !egl.make_current(app_bound) {
                    return;
                }
                let inside = without_app_gl(|| {
                    let inside = egl.current();
                    egl.make_current([display as usize, 0, 0, plugin as usize]);
                    inside
                });
                assert_eq!(inside, None, "no context is current for editor code");
                assert_eq!(
                    egl.current(),
                    Some(app_bound),
                    "the app's context is current again"
                );
                egl.release();
            }
        }
    }
}

/// Plugins' X errors are reported from now on instead of exiting the app (Xlib's own handler exits): called before
/// plugins are loaded.
pub fn catch_x_errors() {
    #[cfg(x11)]
    editor_env::catch_x_errors();
}

/// Runs `f`, a call into plugin editor code, in the conditions it needs on the app thread (on Linux: the app's GL
/// context released, plugins' X errors reported instead of exiting the app); returns the first X error it caused.
pub fn editor_call<R>(f: impl FnOnce() -> R) -> (R, Option<String>) {
    #[cfg(x11)]
    {
        catch_x_errors();
        let _ = editor_env::take_x_error();
        let out = editor_env::without_app_gl(f);
        (out, editor_env::take_x_error())
    }
    #[cfg(not(x11))]
    (f(), None)
}

/// Whether this platform has plugin editor windows.
pub const AVAILABLE: bool = cfg!(any(x11, target_os = "macos", windows));

/// A plugin editor's window. Dropping it destroys the window: close the plugin's editor first.
pub struct EditorWindow {
    #[cfg(x11)]
    conn: std::rc::Rc<x11::Conn>,
    #[cfg(x11)]
    win: u32,
    #[cfg(windows)]
    native: windows_sys::Win32::Foundation::HWND,
    #[cfg(target_os = "macos")]
    native: mac::Native,
}

impl EditorWindow {
    /// A hidden window titled `title`; [`Self::show`] maps it once the editor is in it.
    pub fn new(title: &str) -> Result<Self, String> {
        #[cfg(x11)]
        {
            let conn = x11::conn()?;
            let win = x11::create(&conn, title)?;
            Ok(Self { conn, win })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                native: win::create(title)?,
            })
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                native: mac::create(title)?,
            })
        }
        #[cfg(not(any(x11, target_os = "macos", windows)))]
        {
            let _ = title;
            Err("plugin editor windows are not available on this platform".into())
        }
    }

    /// The native window handle a plugin embeds its editor in (an X11 window id).
    pub fn handle(&self) -> *mut c_void {
        #[cfg(x11)]
        {
            self.win as usize as *mut c_void
        }
        #[cfg(windows)]
        {
            self.native as *mut c_void
        }
        #[cfg(target_os = "macos")]
        {
            objc2::rc::Retained::as_ptr(&self.native.view) as *mut c_void
        }
        #[cfg(not(any(x11, target_os = "macos", windows)))]
        std::ptr::null_mut()
    }

    /// The display's scale, which the editor is told to draw at.
    pub fn scale(&self) -> f32 {
        #[cfg(x11)]
        {
            self.conn.scale
        }
        #[cfg(target_os = "macos")]
        {
            mac::scale(&self.native)
        }
        #[cfg(not(any(x11, target_os = "macos")))]
        1.0
    }

    /// Fits the window to an editor of `size` device pixels.
    pub fn resize(&self, size: (u32, u32)) {
        #[cfg(x11)]
        x11::resize(&self.conn, self.win, size);
        #[cfg(windows)]
        win::resize(self.native, size);
        #[cfg(target_os = "macos")]
        mac::resize(&self.native, size);
        #[cfg(not(any(x11, target_os = "macos", windows)))]
        let _ = size;
    }

    /// Shows the window in front.
    pub fn show(&mut self) {
        #[cfg(x11)]
        x11::show(&self.conn, self.win);
        #[cfg(windows)]
        win::show(self.native);
        #[cfg(target_os = "macos")]
        mac::show(&mut self.native);
    }

    /// Whether `window` (from [`pump`]) is this one.
    pub fn is(&self, window: usize) -> bool {
        #[cfg(x11)]
        {
            self.win as usize == window
        }
        #[cfg(not(x11))]
        {
            #[cfg(windows)]
            {
                self.native as usize == window
            }
            #[cfg(target_os = "macos")]
            {
                let _ = window;
                false
            }
            #[cfg(not(any(target_os = "macos", windows)))]
            {
                let _ = window;
                false
            }
        }
    }
}

impl Drop for EditorWindow {
    fn drop(&mut self) {
        #[cfg(x11)]
        x11::destroy(&self.conn, self.win);
        #[cfg(windows)]
        win::destroy(self.native);
        #[cfg(target_os = "macos")]
        mac::destroy(&self.native);
    }
}

/// Reads the windows' events; returns the windows the user closed.
pub fn pump() -> Vec<usize> {
    #[cfg(x11)]
    {
        x11::pump().into_iter().map(|w| w as usize).collect()
    }
    #[cfg(windows)]
    {
        win::pump()
    }
    #[cfg(target_os = "macos")]
    {
        mac::pump()
    }
    #[cfg(not(any(x11, target_os = "macos", windows)))]
    Vec::new()
}

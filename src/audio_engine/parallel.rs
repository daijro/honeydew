//! Rack workers, on the desktop and (a Honeydew change, #145, #156) Android. Constructed on the UI thread, never in an
//! audio callback.

use std::sync::OnceLock;

use rayon::{ThreadPool, ThreadPoolBuilder};

static POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();

/// The most workers: on Android (a Honeydew change) 4, as the audio callback waits for them.
const MAX_THREADS: usize = if cfg!(target_os = "android") { 4 } else { 8 };

/// Start the reusable workers before asking the audio callback to use them.
/// A failed build leaves the sequential renderer in place.
pub(super) fn prepare() -> bool {
    POOL.get_or_init(|| {
        let cores = std::thread::available_parallelism().ok()?.get();
        if cores < 2 {
            return None;
        }
        let builder = ThreadPoolBuilder::new()
            .num_threads(cores.min(MAX_THREADS))
            .thread_name(|index| format!("honeydew-rack-{index}"));
        #[cfg(target_os = "android")]
        let builder = builder.start_handler(|_| raise_priority());
        builder.build().ok()
    })
    .is_some()
}

pub(super) fn pool() -> Option<&'static ThreadPool> {
    POOL.get().and_then(Option::as_ref)
}

/// Honeydew change (#145, #156): Android's low-latency audio callback runs at real-time priority and waits while the
/// workers process its racks, so they run at the highest priority an app may set, nice −19 (Android's
/// `THREAD_PRIORITY_URGENT_AUDIO`), not behind the UI. A refusal keeps the worker's priority and is logged once.
#[cfg(target_os = "android")]
fn raise_priority() {
    const URGENT_AUDIO: libc::c_int = -19;
    // SAFETY: plain system calls on the calling thread's own id.
    let refused = unsafe {
        let tid = libc::gettid() as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, URGENT_AUDIO) != 0
    };
    if refused {
        let error = std::io::Error::last_os_error();
        static LOGGED: std::sync::Once = std::sync::Once::new();
        LOGGED.call_once(|| eprintln!("audio: the rack workers keep their priority: {error}"));
    }
}

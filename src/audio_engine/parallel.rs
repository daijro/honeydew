//! Desktop rack workers. Constructed on the UI thread, never in an audio callback.

use std::sync::OnceLock;

use rayon::{ThreadPool, ThreadPoolBuilder};

static POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();

/// Start the reusable workers before asking the audio callback to use them.
/// A failed build leaves the sequential renderer in place.
pub(super) fn prepare() -> bool {
    POOL.get_or_init(|| {
        let cores = std::thread::available_parallelism().ok()?.get();
        if cores < 2 {
            return None;
        }
        ThreadPoolBuilder::new()
            .num_threads(cores.min(8))
            .thread_name(|index| format!("honeydew-rack-{index}"))
            .build()
            .ok()
    })
    .is_some()
}

pub(super) fn pool() -> Option<&'static ThreadPool> {
    POOL.get().and_then(Option::as_ref)
}

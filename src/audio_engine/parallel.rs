use std::sync::OnceLock;

use rayon::{ThreadPool, ThreadPoolBuilder};

static POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();

/// Start the reusable workers before asking the audio callback to use them.
/// On Android, cap the pool to 4 workers so big.LITTLE performance cores stay free
/// for UI rendering and AudioFlinger. A failed build leaves the sequential renderer in place.
pub(super) fn prepare() -> bool {
    POOL.get_or_init(|| {
        let cores = std::thread::available_parallelism().ok()?.get();
        if cores < 2 {
            return None;
        }
        let cap = if cfg!(target_os = "android") { 4 } else { 8 };
        ThreadPoolBuilder::new()
            .num_threads(cores.min(cap))
            .thread_name(|index| format!("banaforge-rack-{index}"))
            .build()
            .ok()
    })
    .is_some()
}

pub(super) fn pool() -> Option<&'static ThreadPool> {
    POOL.get().and_then(Option::as_ref)
}

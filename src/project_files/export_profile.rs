//! Wall-time accounting for offline jobs. No audio callback instrumentation or PCM copies.
#[derive(Clone, Debug, Default)]
pub struct ExportProfile {
    pub prepare: f64,
    pub analysis: f64,
    pub processing: f64,
    pub render: f64,
    pub encode: f64,
    pub flush: f64,
    pub archive: f64,
    pub slices: u64,
}

pub(crate) fn clock() -> f64 {
    #[cfg(target_arch = "wasm32")]
    { web_sys::js_sys::Date::now() / 1000.0 }
    #[cfg(not(target_arch = "wasm32"))]
    {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
    }
}

impl ExportProfile {
    pub fn active_seconds(&self) -> f64 {
        self.prepare + self.analysis + self.processing + self.render + self.encode + self.flush + self.archive
    }
    pub fn report(&self, wall: f64) -> String {
        format!("export-profile wall_s={wall:.6} active_s={:.6} outside_timed_work_s={:.6} prepare_s={:.6} analysis_s={:.6} processing_s={:.6} render_s={:.6} encode_s={:.6} flush_s={:.6} archive_s={:.6} slices={}",
            self.active_seconds(), (wall - self.active_seconds()).max(0.0), self.prepare,
            self.analysis, self.processing, self.render, self.encode, self.flush, self.archive, self.slices)
    }
}

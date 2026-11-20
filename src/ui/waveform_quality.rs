//! Display-only waveform policy. Source PCM, summaries, transport and project data are never edited.
use eframe::egui::{self, Id};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WaveformQuality {
    #[default]
    High,
    Balanced,
    Performance,
}

impl WaveformQuality {
    pub const ALL: [Self; 3] = [Self::High, Self::Balanced, Self::Performance];
    pub fn label(self) -> &'static str {
        match self {
            Self::High => "High Quality",
            Self::Balanced => "Balanced",
            Self::Performance => "Performance",
        }
    }
    pub fn group(self) -> usize {
        match self {
            Self::High => 1,
            Self::Balanced => 3,
            Self::Performance => 8,
        }
    }
    /// Build on existing automatic zoom LOD. A mode change only queries visible groups of cached peaks.
    pub fn stride(self, automatic: usize, width: usize) -> usize {
        automatic
            .saturating_mul(self.group())
            .max(1)
            .min(width.max(1))
    }
    /// Performance uses one device-pixel-wide disconnected bar at each group's time centre.
    pub fn column(self, x: f32, width: f32, px: f32) -> (f32, f32) {
        if self == Self::Performance {
            let line = px.min(width);
            (x + (width - line) * 0.5, line)
        } else {
            (x, width)
        }
    }
    /// Per-egui-context (not a process global), published before any screen draws each frame.
    pub fn publish(self, ctx: &egui::Context) {
        ctx.data_mut(|d| d.insert_temp(Id::new("waveform-quality"), self));
    }
    pub fn current(ctx: &egui::Context) -> Self {
        ctx.data(|d| d.get_temp(Id::new("waveform-quality")))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn waveform_quality_contexts_are_independent_and_default_to_high() {
        let a = egui::Context::default();
        let b = egui::Context::default();
        assert_eq!(WaveformQuality::current(&a), WaveformQuality::High);
        WaveformQuality::Performance.publish(&a);
        assert_eq!(WaveformQuality::current(&a), WaveformQuality::Performance);
        assert_eq!(WaveformQuality::current(&b), WaveformQuality::High);
    }
}

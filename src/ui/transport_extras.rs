//! The transport bar's CPU/RAM readout ([`meters`]: `NN %` over `NNN MB` in one slot) and the memory it reads. Memory is
//! the resident size in whole mebibytes (`MB`, as PROJECT's `App memory`), read once a second, and `-- MB` where the
//! platform does not report it (anything but Linux and Android).

use eframe::egui::{Align2, Painter, Rect, vec2};

use crate::theme::{Rgba, hsl_to_rgb};
use crate::ui::UiCtx;
use crate::ui::widgets::shrink;

/// Seconds between memory reads.
const POLL_S: f64 = 1.0;

/// The readout's value: `bytes` in whole mebibytes, `NNN MB`, or `-- MB` when unknown.
pub fn ram_text(bytes: Option<u64>) -> String {
    const MIB: u64 = 1024 * 1024;
    match bytes {
        Some(b) => format!("{} MB", b.saturating_add(MIB / 2) / MIB),
        None => "-- MB".to_owned(),
    }
}

/// The resident size in a `/proc/self/status` text: its `VmRSS:` line, in kB.
fn rss_bytes(status: &str) -> Option<u64> {
    kb_field(status, "VmRSS:")
}

/// The bytes of a `/proc` text's `key` line, given in kB.
fn kb_field(text: &str, key: &str) -> Option<u64> {
    let line = text.lines().find_map(|l| l.strip_prefix(key))?;
    let kb: u64 = line.trim().strip_suffix("kB")?.trim().parse().ok()?;
    Some(kb.saturating_mul(1024))
}

/// The app's resident memory where the platform reports it in `/proc` (Linux, Android); `None` elsewhere.
pub fn app_memory() -> Option<u64> {
    if cfg!(any(target_os = "linux", target_os = "android")) {
        rss_bytes(&std::fs::read_to_string("/proc/self/status").ok()?)
    } else {
        None
    }
}

/// The machine's memory (`Physical memory:`, `Memory used:`), in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemMemory {
    pub total: u64,
    pub used: u64,
}

impl SystemMemory {
    /// The total in whole gibibytes, nearest (`16 GB`: GB as the readout's MB, binary).
    pub fn physical_text(self) -> String {
        const GIB: u64 = 1 << 30;
        format!("{} GB", self.total.saturating_add(GIB / 2) / GIB)
    }

    /// The share in use, 0 to 100.
    pub fn used_percent(self) -> f64 {
        self.used as f64 / self.total as f64 * 100.0
    }
}

/// The machine's memory in a `/proc/meminfo` text: `MemTotal`, with `MemTotal − MemAvailable` in use. `None` without
/// both, or with no memory.
fn meminfo_bytes(meminfo: &str) -> Option<SystemMemory> {
    let total = kb_field(meminfo, "MemTotal:")?;
    let available = kb_field(meminfo, "MemAvailable:")?;
    (total > 0).then(|| SystemMemory {
        total,
        used: total.saturating_sub(available),
    })
}

/// The machine's memory where the platform reports it in `/proc` (Linux, Android); `None` elsewhere, which shows `-`.
/// Windows and macOS readers would need `windows-sys` or `libc` as new direct dependencies (only transitive in the
/// lock).
pub fn system_memory() -> Option<SystemMemory> {
    if cfg!(any(target_os = "linux", target_os = "android")) {
        meminfo_bytes(&std::fs::read_to_string("/proc/meminfo").ok()?)
    } else {
        None
    }
}

/// `Physical memory:`'s value, `-` when unknown.
pub fn physical_memory(mem: Option<SystemMemory>) -> String {
    mem.map_or_else(|| "-".to_owned(), SystemMemory::physical_text)
}

/// `Memory used:`'s value: `%2.0f %%` in the Performance footer (`padded`), `%.0f %%` in the tooltip; `-` when unknown.
pub fn memory_used(mem: Option<SystemMemory>, padded: bool) -> String {
    match mem.map(SystemMemory::used_percent) {
        Some(p) if padded => format!("{p:2.0} %"),
        Some(p) => format!("{p:.0} %"),
        None => "-".to_owned(),
    }
}

/// The `RAM` readout, with its last value.
#[derive(Debug, Default)]
pub struct RamReadout {
    text: String,
    /// The machine's memory at the last read.
    system: Option<SystemMemory>,
    /// When to read the memory next, in egui time.
    next_poll: f64,
}

impl RamReadout {
    /// The memory text of the last read, `NNN MB` (empty before the first read).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The machine's memory at the last read, where the platform reports it.
    pub fn system(&self) -> Option<SystemMemory> {
        self.system
    }

    /// Reads the memory when it is due. It only reads on frames: while the meters are shown the host must keep frames
    /// coming, e.g. with `ctx.request_repaint_after(Duration::from_secs(1))`.
    pub fn poll(&mut self, now: f64) {
        if now >= self.next_poll {
            self.text = ram_text(app_memory());
            self.system = system_memory();
            self.next_poll = now + POLL_S;
        }
    }
}

/// The performance monitor's load as the transport reads it: the peak, and the smoothed load `u`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Load {
    pub peak: f32,
    pub smoothed: f32,
}

/// The smoothed load from which the CPU row is filled red.
const ALARM: f32 = 0.95;
/// The readout's text alpha.
const TEXT_ALPHA: f32 = 0.6;

/// HSL(`h`°, 0.9, 0.5).
fn hue(h: f32) -> Rgba {
    let (r, g, b) = hsl_to_rgb(h / 360.0, 0.9, 0.5);
    Rgba::rgb(r, g, b)
}

/// The CPU text's colour for the smoothed load `u` (before its α 0.6): `text` up to 0.5, then a ramp through
/// HSL(60°, 0.9, 0.5) at 0.75 and HSL(30°) at 0.875 to HSL(0°) at 1.
pub fn cpu_color(u: f32, text: Rgba) -> Rgba {
    let stops = [
        (0.5, text),
        (0.75, hue(60.0)),
        (0.875, hue(30.0)),
        (1.0, hue(0.0)),
    ];
    let u = u.clamp(0.5, 1.0);
    let k = stops.windows(2).position(|w| u <= w[1].0).unwrap_or(2);
    let ((u0, a), (u1, b)) = (stops[k], stops[k + 1]);
    let t = (u - u0) / (u1 - u0);
    let mix = |x: f32, y: f32| x * (1.0 - t) + y * t;
    Rgba::rgb(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b))
}

/// The CPU/RAM readout in the transport `slot`: inset 2t at the sides and 4t at top and bottom, split 1:5:5:1; CPU as
/// `{:3.0} %` of the smoothed load `u` in the second part, the app's memory `ram` in the third, both 8 pt shrunk to fit
/// and centred, at α 0.6 (CPU in [`cpu_color`]). From u ≥ 0.95 the CPU part is filled HSL(0°, 0.9, 0.5) under full
/// `text`.
pub fn meters(painter: &Painter, cx: &UiCtx, slot: Rect, u: f32, ram: &str) {
    let (m, c) = (cx.m, &cx.theme.colors);
    let t = m.t();
    let inner = slot.shrink2(vec2(2.0 * t, 4.0 * t));
    let part = inner.height() / 12.0;
    let row = |from: f32| {
        inner
            .with_min_y(inner.top() + from * part)
            .with_max_y(inner.top() + (from + 5.0) * part)
    };
    let (cpu, mem) = (row(1.0), row(6.0));
    let ink = if u >= ALARM {
        painter.rect_filled(cpu, 0.0, hue(0.0).to_color32());
        c.text.to_color32()
    } else {
        cpu_color(u, c.text).to_color32().gamma_multiply(TEXT_ALPHA)
    };
    let text = format!("{:3.0} %", u.clamp(0.0, 1.0) * 100.0);
    shrink(painter, m, cpu, Align2::CENTER_CENTER, &text, 8, ink);
    let ink = c.text.to_color32().gamma_multiply(TEXT_ALPHA);
    shrink(painter, m, mem, Align2::CENTER_CENTER, ram, 8, ink);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::theme_by_id;

    #[test]
    fn memory_reads_in_whole_mebibytes() {
        assert_eq!(ram_text(Some(241 * 1024 * 1024)), "241 MB");
        assert_eq!(ram_text(Some(0)), "0 MB");
        assert_eq!(ram_text(Some(3 * 1024 * 1024 / 2)), "2 MB");
        assert_eq!(ram_text(None), "-- MB");
        // No overflow at the top.
        assert_eq!(
            ram_text(Some(u64::MAX)),
            format!("{} MB", u64::MAX / (1024 * 1024))
        );
    }

    #[test]
    fn the_machines_memory_comes_from_meminfo_total_less_available() {
        let meminfo = "MemTotal:       16303112 kB\nMemFree:         1234567 kB\nMemAvailable:    2282435 kB\n";
        let mem = meminfo_bytes(meminfo).expect("read");
        assert_eq!(mem.total, 16303112 * 1024);
        assert_eq!(mem.used, (16303112 - 2282435) * 1024);
        // 86 % in use, of 15.5 GiB: `16 GB` (nearest whole GB).
        assert_eq!(mem.physical_text(), "16 GB");
        assert!(
            (mem.used_percent() - 86.0).abs() < 0.01,
            "{}",
            mem.used_percent()
        );
        assert_eq!(meminfo_bytes("MemTotal: 100 kB\n"), None, "no MemAvailable");
        assert_eq!(
            meminfo_bytes("MemTotal: 0 kB\nMemAvailable: 0 kB\n"),
            None,
            "no memory"
        );
        assert_eq!(
            meminfo_bytes("MemTotal: lots kB\nMemAvailable: 1 kB\n"),
            None
        );
    }

    #[test]
    fn memory_used_is_padded_to_2_in_the_footer_and_unpadded_in_the_tooltip() {
        // `Memory used: %2.0f %%`; the tooltip's values are `%.0f %%`.
        let at = |percent: u64| SystemMemory {
            total: 100 << 30,
            used: percent << 30,
        };
        assert_eq!(memory_used(Some(at(7)), true), " 7 %");
        assert_eq!(memory_used(Some(at(7)), false), "7 %");
        assert_eq!(memory_used(Some(at(86)), true), "86 %");
        assert_eq!(memory_used(Some(at(100)), true), "100 %");
        assert_eq!(memory_used(None, true), "-");
        assert_eq!(physical_memory(Some(at(1))), "100 GB");
        assert_eq!(physical_memory(None), "-");
    }

    #[test]
    fn the_resident_size_comes_from_the_vmrss_line() {
        let status = "Name:\tstudio\nVmPeak:\t  900000 kB\nVmRSS:\t  246784 kB\nThreads:\t9\n";
        assert_eq!(rss_bytes(status), Some(246784 * 1024));
        assert_eq!(rss_bytes("Name:\tstudio\n"), None);
        assert_eq!(rss_bytes("VmRSS:\tlots kB\n"), None);
        assert_eq!(
            rss_bytes(&format!("VmRSS: {} kB\n", u64::MAX)),
            Some(u64::MAX)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_reports_the_app_memory() {
        assert!(app_memory().is_some_and(|b| b > 0));
    }

    #[test]
    fn the_cpu_text_stays_text_to_half_load_then_ramps_through_yellow_and_orange_to_red() {
        let text = theme_by_id(0).colors.text;
        assert_eq!(cpu_color(0.3, text), text);
        assert_eq!(cpu_color(0.5, text), text);
        assert_eq!(cpu_color(0.75, text), hue(60.0));
        assert_eq!(cpu_color(0.875, text), hue(30.0));
        assert_eq!(cpu_color(1.0, text), hue(0.0));
        // Halfway between the orange and the red stop.
        let mid = cpu_color(0.9375, text);
        assert!(
            (mid.g - (hue(30.0).g + hue(0.0).g) / 2.0).abs() < 1e-6,
            "{mid:?}"
        );
    }
}

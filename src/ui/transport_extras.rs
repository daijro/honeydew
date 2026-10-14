//! Parts of the transport bar's right side: the `RAM` readout (`RAM` over `NNN MB`) and the `CPU` and `RAM` labels'
//! press ([`MeterLabels`]). (`Undo` and `Redo` are light bubbles, and `Preview` a toggle: `main_screen.rs`.)
//!
//! The readout's lines are the playlist screen's [`READOUT_LINES`] (10 pt `RAM` over the 8 pt value), memory is the
//! resident size in whole mebibytes (`MB`, as PROJECT's `App memory`), read once a second, and `-- MB` where the
//! platform does not report it (anything but Linux and Android).

use eframe::egui::{Align2, Color32, Id, LayerId, Order, Rect, Sense, Ui, Vec2, pos2, vec2};

use crate::metrics::FontSize;
use crate::theme::{Rgba, hsl_to_rgb};
use crate::ui::main_screen::READOUT_LINES;
use crate::ui::{UiCtx, gesture};

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

    /// Paints `RAM` over the value, centred in `rect` (a transport slot), reading the memory when it is due. It only
    /// reads on frames: while the meters are shown the host must keep frames coming, e.g. with
    /// `ctx.request_repaint_after(Duration::from_secs(1))`.
    pub fn show(&mut self, ui: &Ui, cx: &UiCtx, rect: Rect) {
        let now = ui.input(|i| i.time);
        if now >= self.next_poll {
            self.text = ram_text(app_memory());
            self.system = system_memory();
            self.next_poll = now + POLL_S;
        }
        let (m, ink) = (cx.m, cx.theme.colors.text.to_color32());
        let painter = ui.painter();
        for (line, text, size) in [(0, "RAM", FontSize::Pt10), (1, &self.text, FontSize::Pt8)] {
            let at = pos2(rect.center().x, rect.top() + m.u(READOUT_LINES[line]));
            painter.text(at, Align2::CENTER_CENTER, text, m.font_id(size), ink);
        }
    }
}

/// The performance monitor's load as the transport reads it: the peak, and the smoothed load `u`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Load {
    pub peak: f32,
    pub smoothed: f32,
}

/// The smoothed load from which the `CPU` meter's background blinks.
pub const BLINK_FROM: f32 = 0.95;
/// Where the background's ramp starts (`background`) and ends (X) in the smoothed load.
const RAMP: [f32; 2] = [0.9, 1.0];

/// The `CPU` meter rect's fill: a two-stop ramp at `u`, the smoothed load, 0.9 = `background`, 1.0 = X, where X =
/// HSL(0, 0.9, 0.5) while `u` >= 0.95 and floor(2 t) is even (`t`: the clock in seconds), else `background`. At
/// `u` >= 0.95 it blinks at 1 Hz (half a second tinted, half plain), from 50 % red at 0.95 to full red at 1.
pub fn cpu_blink(u: f32, t: f64, background: Color32) -> Color32 {
    let tinted = u >= BLINK_FROM && (2.0 * t).floor().rem_euclid(2.0) == 0.0;
    let x = if tinted {
        let (r, g, b) = hsl_to_rgb(0.0, 0.9, 0.5);
        Rgba::rgb(r, g, b).to_color32()
    } else {
        background
    };
    let k = ((u - RAMP[0]) / (RAMP[1] - RAMP[0])).clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (f32::from(a) * (1.0 - k) + f32::from(b) * k).round() as u8;
    Color32::from_rgb(
        mix(background.r(), x.r()),
        mix(background.g(), x.g()),
        mix(background.b(), x.b()),
    )
}

/// A transport meter with a label to press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Meter {
    Cpu,
    Ram,
}

impl Meter {
    /// The tooltip's width in U: 5 U for `CPU`, 7 U for `RAM`.
    pub fn tooltip_w(self) -> f32 {
        match self {
            Meter::Cpu => 5.0,
            Meter::Ram => 7.0,
        }
    }
}

/// A tooltip row's height, and the room kept round its rows, in U.
const TIP_ROW_H: f32 = 1.0;
const TIP_PAD: f32 = 0.25;
/// How far above the transport bar a tooltip's bottom is, in U.
const TIP_GAP: f32 = 0.25;

/// What pressing the `CPU` and `RAM` labels does: a press released inside the label within 0.3 s asks for the
/// Performance block ([`Self::show`] returns true); a press held past 0.3 s toggles a pinned tooltip for that meter,
/// which unpins the other; a hovering mouse shows it too.
///
/// The label's rect is its transport slot; the tooltip is a `menuFill` box with `text` rows, centred over the slot, its
/// bottom a little above the bar, on the tooltip layer. `Underruns` is `-`: the app does not count them. The machine's
/// memory is `-` where the platform does not report it ([`system_memory`]).
#[derive(Debug, Default)]
pub struct MeterLabels {
    pinned: Option<Meter>,
}

impl MeterLabels {
    /// The meter whose tooltip is pinned.
    pub fn pinned(&self) -> Option<Meter> {
        self.pinned
    }

    /// Handles the presses on the `slots` (each meter's label rect) of the transport `bar` and draws the tooltips that
    /// show. `meter` is the shared data's smoothed load and peak, `memory` the `RAM` readout's text and the machine's
    /// memory. Returns whether a tap asked for the Performance block.
    pub fn show(
        &mut self,
        ui: &Ui,
        cx: &UiCtx,
        bar: Rect,
        slots: [(Meter, Rect); 2],
        meter: (f32, f32),
        memory: (&str, Option<SystemMemory>),
    ) -> bool {
        let mut tapped = false;
        let mut shown = Vec::new();
        for (which, slot) in slots {
            let id = Id::new(("transport", "meter label", which as u8));
            let mut response = ui.interact(slot, id, Sense::click());
            let press = gesture::press(ui, &mut response, cx.m, Vec2::length);
            let inside = ui
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|p| slot.contains(p));
            tapped |= press.tap && inside;
            if press.long {
                self.pinned = (self.pinned != Some(which)).then_some(which);
            }
            if self.pinned == Some(which) || response.hovered() {
                shown.push((which, slot));
            }
        }
        for (which, slot) in shown {
            // Unpadded `%.0f %%`.
            let percent = |v: f32| format!("{:.0} %", v.clamp(0.0, 1.0) * 100.0);
            let rows = match which {
                Meter::Cpu => [
                    ("Utilization:", percent(meter.0)),
                    ("Peak:", percent(meter.1)),
                    ("Underruns:", "-".to_owned()),
                ],
                Meter::Ram => [
                    ("Physical memory:", physical_memory(memory.1)),
                    ("Memory used:", memory_used(memory.1, false)),
                    ("App memory:", memory.0.to_owned()),
                ],
            };
            tooltip(ui, cx, bar, slot, which, &rows);
        }
        tapped
    }
}

/// The tooltip of `which`, with its label / value `rows`, over the transport `bar` at `slot`.
fn tooltip(ui: &Ui, cx: &UiCtx, bar: Rect, slot: Rect, which: Meter, rows: &[(&str, String); 3]) {
    let (m, c) = (cx.m, &cx.theme.colors);
    let size = vec2(
        m.u(which.tooltip_w()),
        m.u(TIP_ROW_H * rows.len() as f32 + 2.0 * TIP_PAD),
    );
    let screen = ui.ctx().content_rect();
    let left = (slot.center().x - size.x / 2.0)
        .clamp(screen.left(), (screen.right() - size.x).max(screen.left()));
    let top = (bar.top() - m.u(TIP_GAP) - size.y).max(screen.top());
    let rect = Rect::from_min_size(pos2(left, top), size);
    let layer = LayerId::new(
        Order::Tooltip,
        Id::new(("transport", "tooltip", which as u8)),
    );
    let painter = ui.ctx().layer_painter(layer);
    painter.rect_filled(rect, 0.0, c.menu_fill.to_color32());
    let (font, ink) = (m.font_id(FontSize::Pt10), c.text.to_color32());
    for (i, (name, value)) in rows.iter().enumerate() {
        let y = rect.top() + m.u(TIP_PAD) + (i as f32 + 0.5) * m.u(TIP_ROW_H);
        let inset = m.u(TIP_PAD);
        painter.text(
            pos2(rect.left() + inset, y),
            Align2::LEFT_CENTER,
            name,
            font.clone(),
            ink,
        );
        painter.text(
            pos2(rect.right() - inset, y),
            Align2::RIGHT_CENTER,
            value,
            font.clone(),
            ink,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::theme_by_id;
    use crate::ui::offscreen::Offscreen;
    use eframe::egui::{Event, PointerButton, Pos2, Shape, epaint::ClippedShape, pos2};

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

    /// A 1.75 U slot of a transport bar at the bottom of the screen.
    const SLOT: Rect = Rect {
        min: pos2(900.0, 484.0),
        max: pos2(956.0, 540.0),
    };

    #[test]
    fn the_readout_shows_ram_over_the_value_and_polls_once_a_second() {
        let mut screen = Offscreen::new();
        let mut ram = RamReadout::default();
        let mut polls = 0;
        let mut texts = Vec::new();
        for _ in 0..90 {
            let shapes = screen.frame(vec![], &mut |ui, cx| {
                let before = ram.next_poll;
                ram.show(ui, cx, SLOT);
                polls += usize::from(ram.next_poll != before);
            });
            texts = Offscreen::texts(&shapes);
        }
        // 1.5 s of frames: the first frame and one second later.
        assert_eq!(polls, 2);
        let ink = theme_by_id(0).colors.text.to_color32();
        assert_eq!(texts[0], ("RAM".to_owned(), ink));
        assert!(texts[1].0.ends_with(" MB"), "{texts:?}");
    }

    // ---- the CPU / RAM labels ----

    /// The transport bar at the bottom of the screen, and the CPU and RAM slots (1.75 U) in it.
    const BAR: Rect = Rect {
        min: pos2(0.0, 484.0),
        max: pos2(1170.0, 540.0),
    };
    const CPU: Rect = Rect {
        min: pos2(840.0, 484.0),
        max: pos2(896.0, 540.0),
    };
    /// 8 GiB with 1 GiB in use.
    const MACHINE: Option<SystemMemory> = Some(SystemMemory {
        total: 8 << 30,
        used: 1 << 30,
    });

    const RAM: Rect = Rect {
        min: pos2(896.0, 484.0),
        max: pos2(952.0, 540.0),
    };
    const AWAY: Pos2 = pos2(200.0, 100.0);

    /// One frame of the labels: whether a tap asked for the Performance block, and what was painted.
    fn labels(
        screen: &mut Offscreen,
        state: &mut MeterLabels,
        events: Vec<Event>,
    ) -> (bool, Vec<ClippedShape>) {
        let mut tapped = false;
        let shapes = screen.frame(events, &mut |ui, cx| {
            let slots = [(Meter::Cpu, CPU), (Meter::Ram, RAM)];
            tapped |= state.show(ui, cx, BAR, slots, (0.234, 0.567), ("37 MB", MACHINE));
        });
        (tapped, shapes)
    }

    fn button(at: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    /// A press at `at`, held `frames` frames (1/60 s each), then released: whether any frame was a tap.
    fn press(screen: &mut Offscreen, state: &mut MeterLabels, at: Pos2, frames: usize) -> bool {
        labels(screen, state, vec![Event::PointerMoved(at)]);
        let mut tapped = labels(screen, state, vec![button(at, true)]).0;
        for _ in 0..frames {
            tapped |= labels(screen, state, vec![]).0;
        }
        tapped | labels(screen, state, vec![button(at, false)]).0
    }

    fn painted(shapes: &[ClippedShape]) -> Vec<String> {
        Offscreen::texts(shapes).into_iter().map(|t| t.0).collect()
    }

    #[test]
    fn a_press_released_inside_a_label_within_0_3_s_is_a_tap() {
        for at in [CPU.center(), RAM.center()] {
            let (mut screen, mut state) = (Offscreen::new(), MeterLabels::default());
            assert!(press(&mut screen, &mut state, at, 3), "{at:?}");
            assert_eq!(state.pinned(), None);
        }
        // Not outside a label, and not one that left it before the release.
        let (mut screen, mut state) = (Offscreen::new(), MeterLabels::default());
        assert!(!press(&mut screen, &mut state, AWAY, 3));
        labels(
            &mut screen,
            &mut state,
            vec![Event::PointerMoved(CPU.center())],
        );
        labels(&mut screen, &mut state, vec![button(CPU.center(), true)]);
        labels(&mut screen, &mut state, vec![Event::PointerMoved(AWAY)]);
        assert!(!labels(&mut screen, &mut state, vec![button(AWAY, false)]).0);
    }

    #[test]
    fn a_press_held_past_0_3_s_pins_its_tooltip_and_is_no_tap() {
        let (mut screen, mut state) = (Offscreen::new(), MeterLabels::default());
        // 0.4 s: past the 0.3 s of a tap.
        assert!(!press(&mut screen, &mut state, CPU.center(), 24));
        assert_eq!(state.pinned(), Some(Meter::Cpu));
        let (_, shapes) = labels(&mut screen, &mut state, vec![Event::PointerMoved(AWAY)]);
        assert_eq!(
            painted(&shapes),
            ["Utilization:", "23 %", "Peak:", "57 %", "Underruns:", "-"]
        );
        // Pinning RAM's unpins CPU's; a second press on RAM unpins it.
        assert!(!press(&mut screen, &mut state, RAM.center(), 24));
        assert_eq!(state.pinned(), Some(Meter::Ram));
        let (_, shapes) = labels(&mut screen, &mut state, vec![Event::PointerMoved(AWAY)]);
        assert_eq!(
            painted(&shapes),
            [
                "Physical memory:",
                "8 GB",
                "Memory used:",
                // 12.5 rounds to even, as C's `%.0f` does.
                "12 %",
                "App memory:",
                "37 MB"
            ]
        );
        assert!(!press(&mut screen, &mut state, RAM.center(), 24));
        assert_eq!(state.pinned(), None);
    }

    #[test]
    fn a_hovering_mouse_shows_the_tooltip_and_leaving_hides_it() {
        let (mut screen, mut state) = (Offscreen::new(), MeterLabels::default());
        labels(&mut screen, &mut state, vec![Event::PointerMoved(AWAY)]);
        assert!(painted(&labels(&mut screen, &mut state, vec![]).1).is_empty());
        labels(
            &mut screen,
            &mut state,
            vec![Event::PointerMoved(CPU.center())],
        );
        let shown = painted(&labels(&mut screen, &mut state, vec![]).1);
        assert!(shown.contains(&"Utilization:".to_owned()), "{shown:?}");
        labels(&mut screen, &mut state, vec![Event::PointerMoved(AWAY)]);
        assert!(painted(&labels(&mut screen, &mut state, vec![]).1).is_empty());
        assert_eq!(state.pinned(), None);
    }

    #[test]
    fn the_tooltips_are_5_and_7_u_wide_boxes_over_the_bar_with_three_rows() {
        let menu_fill = theme_by_id(0).colors.menu_fill.to_color32();
        for (slot, width) in [(CPU, 5.0 * 32.0), (RAM, 7.0 * 32.0)] {
            let (mut screen, mut state) = (Offscreen::new(), MeterLabels::default());
            labels(
                &mut screen,
                &mut state,
                vec![Event::PointerMoved(slot.center())],
            );
            let (_, shapes) = labels(&mut screen, &mut state, vec![]);
            let boxes: Vec<Rect> = shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    Shape::Rect(r) if r.fill == menu_fill => Some(r.rect),
                    _ => None,
                })
                .collect();
            let [tip] = boxes[..] else {
                panic!("{boxes:?}")
            };
            assert_eq!(tip.width(), width);
            // Three 1 U rows with a quarter U round them, centred over the slot, above the bar.
            assert_eq!(tip.height(), 3.5 * 32.0);
            assert!((tip.center().x - slot.center().x).abs() < 0.01 || tip.right() <= 1170.0);
            assert!(tip.bottom() <= BAR.top());
        }
    }

    #[test]
    fn a_release_just_outside_a_label_is_no_tap_and_just_inside_is() {
        // The press and the release are both inside the 0.5 U calm zone; only where the release lands decides.
        let y = CPU.center().y;
        for (release_x, tap) in [(CPU.right() - 1.0, true), (CPU.right() + 1.0, false)] {
            let (mut screen, mut state) = (Offscreen::new(), MeterLabels::default());
            let press = pos2(CPU.right() - 3.0, y);
            labels(&mut screen, &mut state, vec![Event::PointerMoved(press)]);
            labels(&mut screen, &mut state, vec![button(press, true)]);
            let away = pos2(release_x, y);
            labels(&mut screen, &mut state, vec![Event::PointerMoved(away)]);
            let released = labels(&mut screen, &mut state, vec![button(away, false)]).0;
            assert_eq!(released, tap, "released at {release_x}");
        }
    }

    #[test]
    fn the_cpu_meters_background_ramps_from_0_9_and_blinks_at_1_hz_from_0_95() {
        // A two-stop ramp at u, 0.9 = `background`, 1.0 = X; X = HSL(0, 0.9, 0.5) while u >= 0.95 and floor(2 t) is
        // even, else `background`.
        let bg = theme_by_id(0).colors.background.to_color32();
        let red = Color32::from_rgb(242, 13, 13);
        // (A channel may round either way: within one.)
        let near = |a: Color32, b: Color32| {
            (a.r().abs_diff(b.r()) | a.g().abs_diff(b.g()) | a.b().abs_diff(b.b())) <= 1
        };
        let toward = |k: f32| {
            let mix = |a: u8, b: u8| (f32::from(a) * (1.0 - k) + f32::from(b) * k).round() as u8;
            Color32::from_rgb(
                mix(bg.r(), red.r()),
                mix(bg.g(), red.g()),
                mix(bg.b(), red.b()),
            )
        };
        for u in [0.0, 0.5, 0.9] {
            assert_eq!(cpu_blink(u, 0.0, bg), bg, "u {u}");
        }
        // Below 0.95 the ramp's far end is the background too, whatever the clock.
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert_eq!(cpu_blink(0.94, t, bg), bg, "t {t}");
        }
        // At 0.95: 50 % red for half a second, then plain for half a second, again every second.
        for (t, tinted) in [
            (0.0, true),
            (0.49, true),
            (0.5, false),
            (0.99, false),
            (1.0, true),
            (2.25, true),
            (2.5, false),
        ] {
            let want = if tinted { toward(0.5) } else { bg };
            assert!(near(cpu_blink(0.95, t, bg), want), "t {t}");
        }
        // Up the ramp: 75 % at 0.975, full red at 1.
        assert!(near(cpu_blink(0.975, 0.1, bg), toward(0.75)));
        assert_eq!(cpu_blink(1.0, 0.1, bg), red);
        assert_eq!(cpu_blink(1.0, 0.6, bg), bg);
        // Past the end of the ramp it stays at X.
        assert_eq!(cpu_blink(1.5, 0.1, bg), red);
    }
}

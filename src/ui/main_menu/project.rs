//! The PROJECT tab: `Save`, `Save New` and `Quick Render`, then the `Project info`, `Metadata` and `Performance`
//! sections, scrolling as one page.
//!
//! The tab shows a [`ProjectInfo`] the app fills in. It edits the metadata texts in place and reports everything else
//! as a [`MainMenuAction`].
//!
//! The Performance section: the header row, then one block (status line, load bar, graph, footer;
//! [`crate::ui::performance`]) 2 U in from each side of the page, 5 U tall, with the `Detach` / `Attach` button centred
//! on its right edge's top. `Hide` and `Reset` are the same button. The page is the menu panel; the tabs' content is
//! inset 0.5 U from it. A request to show the block ([`ProjectTab::show_performance`], the CPU / RAM labels' tap)
//! scrolls it into view with a 1.5 U margin ([`scroll_to`]).
//!
//! Hide and Lock, from the song's META@256 time spent and META@776 lock:
//! - While the time spent is negative, the Time spent, Project started on and Project last saved on values show
//!   `Hidden`; if the song is also locked, those three rows go.
//! - `Hide` shows when unlocked with a time spent ≥ 0, `Reset` when unlocked with a time spent > 0.
//! - When locked, Title, Author and Online link stay but are read-only, the Online link row (label and field) hides
//!   while the link is empty, and the `Lock metadata forever` button hides.
//! - `Open` shows whenever the link is non-empty, locked or not.

use eframe::egui::{Align2, Id, Rect, TextEdit, Ui, pos2, vec2};

use super::{
    Circle, CircleStyle, MainMenuAction, PAD, circle_button, light_button, scroll, text_field,
};
use crate::metrics::FontSize;
use crate::ui::UiCtx;
use crate::ui::performance::{self, Performance};
use crate::ui::widgets::text_label as label;

// Sizes in U.
const BUTTON_H: f32 = 1.25;
const GAP: f32 = 0.25;
const HEADER_H: f32 = 1.25;
const INFO_ROW_H: f32 = 1.0;
/// Where the value column starts, as a share of the width.
const VALUE_X: f32 = 0.4;
/// `Open`'s circle; `-` / value / `+` tempo buttons (widths) and their height.
const SMALL_D: f32 = 1.25;
const TEMPO_W: [f32; 3] = [1.25, 3.5, 1.25];
const TEMPO_H: f32 = 0.8;
/// Metadata: the label line over each field, and the field.
const FIELD_LABEL_H: f32 = 0.75;
const FIELD_H: f32 = 1.0;
/// Performance: the header row, the block under it and how far it is in from the page's sides, the `Detach` / `Attach`,
/// `Hide` and `Reset` discs (diameter 2 × 0.7 U), the disc's centre below the block's top, and the margin the block is
/// scrolled into view with.
const PERFORMANCE_HEADER_H: f32 = 1.3;
const BLOCK_H: f32 = 5.0;
const BLOCK_INSET: f32 = 2.0;
const BUTTON_D: f32 = 1.4;
const DETACH_DY: f32 = 0.5;
/// `Detach`'s touch box.
const DETACH_BOX: f32 = 2.0;
const SCROLL_MARGIN: f32 = 1.5;

/// The lowest and highest tempo the `-` / `+` buttons reach, in BPM: the tempo dial's range.
pub const TEMPO_RANGE: (f64, f64) = (10.0, 999.0);
/// `META` text lengths in bytes, less the terminating zero.
const TEXT_LIMIT: usize = 255;
const LINK_LIMIT: usize = 2047;

/// What the PROJECT tab shows. The app fills it in; the tab edits `title`, `author` and `link` in place.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectInfo {
    /// `Project Length:`, in seconds.
    pub length_secs: f64,
    /// `Time spent on project:`, in seconds: META@256, negative once hidden (`Hide`).
    pub time_spent_secs: f64,
    /// `Project started on:` and `Project last saved on:`, in seconds since 1970 in local time (they are shown as
    /// given, with no time zone applied).
    pub started_on: Option<i64>,
    pub last_saved_on: Option<i64>,
    pub tempo_bpm: f64,
    /// The audio output's sample rate, while audio runs.
    pub sample_rate: Option<u32>,
    /// `Title:`, `Author:` and `Online link:`: META@520 (the display title, not the file name), @264 and @777.
    pub title: String,
    pub author: String,
    pub link: String,
    /// The song's META@776 lock (`Lock metadata forever` was confirmed): the texts are read-only, and `Hide`, `Reset`
    /// and the lock button are gone.
    pub metadata_locked: bool,
    /// The performance monitor's data: the block's status line, bar and graph, and which button it offers
    /// ([`Performance::detached`]).
    pub performance: Performance,
}

impl Default for ProjectInfo {
    fn default() -> Self {
        Self {
            length_secs: 0.0,
            time_spent_secs: 0.0,
            started_on: None,
            last_saved_on: None,
            tempo_bpm: 140.0,
            sample_rate: None,
            title: String::new(),
            author: String::new(),
            link: String::new(),
            metadata_locked: false,
            performance: Performance::default(),
        }
    }
}

/// A duration as `13m 2s`, and from one hour on as `1h 2m 3s`.
pub fn duration(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else {
        format!("{m}m {s}s")
    }
}

/// A time in seconds since 1970 as `10 October 2026 23:55:03` (`d MMMM yyyy HH:mm:ss`).
pub fn date(secs: i64) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let ((year, month, day), time) = civil(secs);
    format!(
        "{day} {} {year} {:02}:{:02}:{:02}",
        MONTHS[month as usize - 1],
        time / 3600,
        time / 60 % 60,
        time % 60
    )
}

/// A time in seconds since 1970 as `META` writes it: `2026-10-10 23:55:03`.
pub fn meta_time(secs: i64) -> String {
    let ((year, month, day), time) = civil(secs);
    let (h, m, s) = (time / 3600, time / 60 % 60, time % 60);
    format!("{year:04}-{month:02}-{day:02} {h:02}:{m:02}:{s:02}")
}

/// A `META` time (`YYYY-MM-DD HH:MM:SS`, or with dashes in the time as `created` has them) in seconds since 1970;
/// `None` for anything else ("?", "Never / demo project").
pub fn meta_secs(text: &str) -> Option<i64> {
    let (date, time) = text.split_once(' ')?;
    let parts = |s: &str, sep: &[char]| -> Option<[i64; 3]> {
        let v: Vec<i64> = s
            .split(sep)
            .map(|p| p.parse::<u32>().ok().map(i64::from))
            .collect::<Option<_>>()?;
        v.try_into().ok()
    };
    let ([y, mo, d], [h, mi, s]) = (parts(date, &['-'])?, parts(time, &[':', '-'])?);
    let valid = (1..=12).contains(&mo) && (1..=31).contains(&d) && h < 24 && mi < 60 && s < 60;
    // Days since 1970-01-01 from a civil date (H. Hinnant's `days_from_civil`), with years starting in March.
    let y = y - i64::from(mo <= 2);
    let (era, yoe) = (y.div_euclid(400), y.rem_euclid(400));
    let doy = (153 * ((mo + 9) % 12) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    valid.then_some(days * 86_400 + h * 3600 + mi * 60 + s)
}

/// Seconds since 1970 as ((year, month, day), seconds into the day).
fn civil(secs: i64) -> ((i64, i64, i64), i64) {
    let (days, time) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since 1970-01-01 to a civil date (H. Hinnant's `civil_from_days`), with years starting in March.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    ((year, month, day), time)
}

/// `n` with a comma between thousands: `48,000`.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(d);
    }
    out
}

/// The address `Open` opens for the online `link`, which comes from the song file: an `http://` / `https://` address
/// (any case; with neither, `https://` goes in front) whose host is a plain domain or a dotted IPv4 address, with an
/// optional port, no user info, and no whitespace, control character or `\` anywhere. Anything else is `None` and
/// opens nothing: the browser opener takes text that is no valid URL as a local file path.
// ponytail: a hand check, as the `url` crate is no direct dependency: ASCII hosts only, no punycode or IPv6.
// Native builds also turn on `webbrowser`'s `hardened` feature, which refuses whatever is no http(s) URL.
pub fn link_url(link: &str) -> Option<String> {
    let link = link.trim();
    let web = ["http://", "https://"].iter().any(|s| {
        link.get(..s.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(s))
    });
    let url = match web {
        true => link.to_owned(),
        false => format!("https://{link}"),
    };
    if url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return None;
    }
    let rest = &url[url.find("://")? + 3..];
    let authority = rest.split(['/', '?', '#']).next()?;
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let port_ok = port.is_none_or(|p| digits(p) && p.parse::<u16>().is_ok());
    let labels: Vec<&str> = host.split('.').collect();
    // Four decimal numbers up to 255, without leading zeros (which would read as octal).
    let ipv4 = labels.len() == 4
        && (labels.iter())
            .all(|l| digits(l) && l.parse::<u8>().is_ok() && (*l == "0" || !l.starts_with('0')));
    // Letters, digits and inner hyphens, and no punycode (`xn--`, which may be invalid); the last label starts with a
    // letter, or the host would read as a number.
    let label = |l: &&str| {
        !l.is_empty()
            && !l.starts_with('-')
            && !l.ends_with('-')
            && !l.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("xn--"))
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    let domain = labels.iter().all(label)
        && labels
            .last()
            .is_some_and(|l| l.starts_with(|c: char| c.is_ascii_alphabetic()));
    (port_ok && (ipv4 || domain)).then_some(url)
}

/// The tempo one BPM down (`-1.0`) or up (`1.0`), kept in [`TEMPO_RANGE`].
pub fn tempo_step(bpm: f64, step: f64) -> f64 {
    (bpm + step).clamp(TEMPO_RANGE.0, TEMPO_RANGE.1)
}

/// The PROJECT tab's state.
#[derive(Default)]
pub(super) struct ProjectTab {
    scroll: f32,
    /// The page's height in the last frame, for the scroll range.
    content_h: f32,
    /// The Performance block is to be scrolled into view ([`Self::show_performance`]).
    scroll_to_performance: bool,
}

impl ProjectTab {
    /// Scrolls the Performance block into view as the tab draws next (the CPU / RAM labels' tap).
    pub(super) fn show_performance(&mut self) {
        self.scroll_to_performance = true;
    }
}

/// The scroll offset that brings the block spanning `top..bottom` of the page's content into a view `view_h` tall that
/// is at `offset`, with `margin` kept on both sides: none when block and margin are in view; the block's top - margin
/// at the view's top when it is above; otherwise its bottom + margin at the view's bottom. Kept in `0..=max`.
pub(super) fn scroll_to(
    offset: f32,
    view_h: f32,
    (top, bottom): (f32, f32),
    margin: f32,
    max: f32,
) -> f32 {
    let (lo, hi) = (top - margin, bottom + margin);
    let target = if lo >= offset && hi <= offset + view_h {
        offset
    } else if lo < offset {
        lo
    } else {
        hi - view_h
    };
    target.clamp(0.0, max)
}

impl ProjectTab {
    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        area: Rect,
        info: &mut ProjectInfo,
    ) -> Option<MainMenuAction> {
        let m = cx.m;
        let c = &cx.theme.colors;
        let text = c.text.to_color32();
        let id = Id::new("main_menu_project");
        let mut action = None;
        let offset = scroll(
            ui,
            id.with("scroll"),
            area,
            self.content_h,
            &mut self.scroll,
        );
        let outer_clip = ui.clip_rect();
        ui.set_clip_rect(outer_clip.intersect(area));

        let top = area.top() - offset;
        let mut y = top;
        let mut band = |h: f32| {
            let r = Rect::from_x_y_ranges(area.x_range(), y..=y + m.u(h));
            y += m.u(h);
            r
        };

        // Save, Save New, Quick Render.
        let row = band(BUTTON_H);
        let w = (row.width() - 2.0 * m.u(GAP)) / 3.0;
        for (i, (name, act)) in [
            ("Save", Some(MainMenuAction::Save)),
            ("Save New", Some(MainMenuAction::SaveNew)),
            ("Quick Render", Some(MainMenuAction::QuickRender)),
        ]
        .into_iter()
        .enumerate()
        {
            let left = row.left() + i as f32 * (w + m.u(GAP));
            let rect = Rect::from_x_y_ranges(left..=left + w, row.y_range());
            if light_button(ui, cx, id.with(name), rect, name, act.is_some()) {
                action = act;
            }
        }
        band(GAP);

        // Project info.
        let header = band(HEADER_H);
        label(
            ui,
            cx,
            header,
            "Project info",
            FontSize::Pt12,
            Align2::LEFT_CENTER,
            text,
        );
        let value_x = area.left() + VALUE_X * area.width();
        let locked = info.metadata_locked;
        let (time, hidden) = (info.time_spent_secs, info.time_spent_secs < 0.0);
        let shown = |v: Option<String>| match hidden {
            true => "Hidden".into(),
            false => v.unwrap_or_else(|| "-".into()),
        };
        let rows = [
            (
                "Project Length:",
                duration(info.length_secs.max(0.0) as u64),
            ),
            ("Time spent on project:", shown(Some(duration(time as u64)))),
            ("Project started on:", shown(info.started_on.map(date))),
            (
                "Project last saved on:",
                shown(info.last_saved_on.map(date)),
            ),
            ("Tempo:", String::new()),
            (
                "Samplerate:",
                info.sample_rate
                    .map_or_else(|| "-".into(), |r| format!("{} Hz", thousands(r.into()))),
            ),
        ];
        // Hidden and locked: the three time rows go.
        let time_row = |name: &str| {
            [
                "Time spent on project:",
                "Project started on:",
                "Project last saved on:",
            ]
            .contains(&name)
        };
        let rows = rows
            .into_iter()
            .filter(|(name, _)| !(locked && hidden && time_row(name)));
        for (name, value) in rows {
            let row = band(INFO_ROW_H);
            let (left, right) = row.split_left_right_at_x(value_x);
            label(
                ui,
                cx,
                left,
                name,
                FontSize::Pt10,
                Align2::LEFT_CENTER,
                text,
            );
            label(
                ui,
                cx,
                right,
                &value,
                FontSize::Pt10,
                Align2::LEFT_CENTER,
                text,
            );
            match name {
                "Time spent on project:" => {
                    let d = m.u(BUTTON_D);
                    // `Hide` rightmost, `Reset` left of it; `Reset` shows only while `Hide` does.
                    for (i, (button, act, visible)) in [
                        ("Hide", MainMenuAction::HideTimeSpent, time >= 0.0),
                        ("Reset", MainMenuAction::ResetTimeSpent, time > 0.0),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        if locked || !visible {
                            continue;
                        }
                        let at = pos2(
                            row.right() - d / 2.0 - i as f32 * (d + m.u(GAP)),
                            row.center().y,
                        );
                        let circle = Circle {
                            style: CircleStyle::Rimmed,
                            ..Circle::new(id.with(button), at, d, button)
                        };
                        if circle_button(ui, cx, &circle) {
                            action = Some(act);
                        }
                    }
                }
                "Tempo:" => {
                    let mut x = value_x;
                    let h = m.u(TEMPO_H);
                    for (i, w) in TEMPO_W.into_iter().enumerate() {
                        let rect =
                            Rect::from_min_size(pos2(x, row.center().y - h / 2.0), vec2(m.u(w), h));
                        x += m.u(w + GAP);
                        let caption = match i {
                            0 => "-".to_owned(),
                            1 => format!("{:.1} BPM", info.tempo_bpm),
                            _ => "+".to_owned(),
                        };
                        // The value button opens the tempo dialog, which is not part of this tab: it only shows,
                        // at full strength.
                        let clicked =
                            light_button(ui, cx, id.with(("tempo", i)), rect, &caption, true);
                        if clicked && i != 1 {
                            let step = if i == 0 { -1.0 } else { 1.0 };
                            action =
                                Some(MainMenuAction::SetTempo(tempo_step(info.tempo_bpm, step)));
                        }
                    }
                }
                _ => {}
            }
        }
        band(GAP);

        // Metadata.
        let header = band(HEADER_H);
        label(
            ui,
            cx,
            header,
            "Metadata",
            FontSize::Pt12,
            Align2::LEFT_CENTER,
            text,
        );
        for (name, value, limit) in [
            ("Title:", &mut info.title, TEXT_LIMIT),
            ("Author:", &mut info.author, TEXT_LIMIT),
            ("Online link:", &mut info.link, LINK_LIMIT),
        ] {
            let link = limit == LINK_LIMIT;
            if link && locked && value.is_empty() {
                continue;
            }
            let line = band(FIELD_LABEL_H);
            label(
                ui,
                cx,
                line,
                name,
                FontSize::Pt10,
                Align2::LEFT_CENTER,
                text,
            );
            let mut field = band(FIELD_H);
            if link && !value.is_empty() {
                // `Open` at the field's right end, as `Reset` / `Hide` end the time row.
                let d = m.u(SMALL_D);
                let at = pos2(field.right() - d / 2.0, field.center().y);
                let circle = Circle {
                    font: FontSize::Pt6,
                    ..Circle::new(id.with("open"), at, d, "Open")
                };
                // A link that is no web address shows `Open` all the same, which then opens nothing.
                if circle_button(ui, cx, &circle)
                    && let Some(url) = link_url(value)
                {
                    action = Some(MainMenuAction::OpenLink(url));
                }
                field.set_right(field.right() - d - m.u(GAP));
            }
            let edit = TextEdit::singleline(value)
                .id(id.with(name))
                .char_limit(limit)
                .interactive(!locked);
            text_field(ui, cx, field, "Click to edit", edit);
            // The field counts characters; the file counts bytes.
            value.truncate(value.floor_char_boundary(limit));
            band(GAP);
        }
        if !locked {
            let lock = band(BUTTON_H);
            let name = "Lock metadata forever";
            if light_button(ui, cx, id.with("lock"), lock, name, true) {
                action = Some(MainMenuAction::LockMetadata);
            }
            band(GAP);
        }

        // Performance: the header row, then the block 2 U in from the page's sides.
        let header = band(PERFORMANCE_HEADER_H);
        label(
            ui,
            cx,
            header,
            "Performance",
            FontSize::Pt12,
            Align2::LEFT_CENTER,
            text,
        );
        let inset = m.u(BLOCK_INSET - PAD);
        let (left, right) = (area.left() + inset, area.right() - inset);
        let row = band(BLOCK_H);
        let block = Rect::from_min_max(pos2(left, row.top()), pos2(right, row.bottom()));
        performance::show(ui, cx, block, &info.performance);
        // `Detach` / `Attach`, centred on the block's right edge, 0.5 U below its top.
        let detached = info.performance.detached;
        let button = Circle {
            style: CircleStyle::Rimmed,
            // The 2 U x 2 U box is the touch area, not the 1.4 U disc.
            hit: Some(m.u(DETACH_BOX)),
            ..Circle::new(
                id.with("detach"),
                pos2(block.right(), block.top() + m.u(DETACH_DY)),
                m.u(BUTTON_D),
                if detached { "Attach" } else { "Detach" },
            )
        };
        if circle_button(ui, cx, &button) {
            action = Some(if detached {
                MainMenuAction::AttachPerformance
            } else {
                MainMenuAction::DetachPerformance
            });
        }

        self.content_h = y - top;
        if std::mem::take(&mut self.scroll_to_performance) {
            let span = (block.top() - top, block.bottom() - top);
            let max = (self.content_h - area.height()).max(0.0);
            let view = area.height();
            self.scroll = scroll_to(self.scroll, view, span, m.u(SCROLL_MARGIN), max);
            ui.ctx().request_repaint();
        }
        ui.set_clip_rect(outer_clip);
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_count_minutes_and_seconds_then_hours() {
        assert_eq!(duration(0), "0m 0s");
        assert_eq!(duration(2), "0m 2s");
        assert_eq!(duration(13 * 60 + 2), "13m 2s");
        assert_eq!(duration(3599), "59m 59s");
        assert_eq!(duration(3600), "1h 0m 0s");
        assert_eq!(duration(3600 + 2 * 60 + 3), "1h 2m 3s");
    }

    #[test]
    fn dates_are_day_month_year_and_time() {
        assert_eq!(date(1_791_676_503), "10 October 2026 23:55:03");
        assert_eq!(date(1_791_676_940), "11 October 2026 00:02:20");
        assert_eq!(date(0), "1 January 1970 00:00:00");
        // A leap day, and a time before 1970.
        assert_eq!(date(951_825_600), "29 February 2000 12:00:00");
        assert_eq!(date(-1), "31 December 1969 23:59:59");
    }

    #[test]
    fn meta_times_are_written_with_colons_and_read_with_either() {
        assert_eq!(meta_time(1_791_676_503), "2026-10-10 23:55:03");
        assert_eq!(meta_time(951_825_600), "2000-02-29 12:00:00");
        assert_eq!(meta_time(-1), "1969-12-31 23:59:59");
        // `modified` has colons, `created` dashes in the time.
        assert_eq!(meta_secs("2026-10-10 23:55:03"), Some(1_791_676_503));
        assert_eq!(meta_secs("2026-10-10 23-55-03"), Some(1_791_676_503));
        assert_eq!(meta_secs("2000-02-29 12:00:00"), Some(951_825_600));
        assert_eq!(meta_secs("1969-12-31 23:59:59"), Some(-1));
        for bad in [
            "?",
            "Never / demo project",
            "",
            "2026-10-10",
            "2026-13-01 00:00:00",
            "2026-10-10 24:00:00",
            "2026-10-10 23:55:03:01",
            "2026-10-10 -1:00:00",
        ] {
            assert_eq!(meta_secs(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn thousands_are_separated_by_commas() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(48_000), "48,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn scroll_to_brings_the_block_into_view_with_a_margin_as_screens_md_says() {
        // (offset, view height, block span, margin, max) -> offset. In view with its margin: none. Above the view: its
        // top - margin goes to the view's top. Otherwise its bottom + margin goes to the view's bottom. Clamped.
        let v = |offset, span, max| scroll_to(offset, 300.0, span, 48.0, max);
        assert_eq!(v(100.0, (150.0, 250.0), 900.0), 100.0, "in view");
        assert_eq!(
            v(100.0, (110.0, 250.0), 900.0),
            62.0,
            "the margin above is out of view: top - margin to the top"
        );
        assert_eq!(v(100.0, (20.0, 80.0), 900.0), 0.0, "above, clamped at 0");
        assert_eq!(
            v(0.0, (500.0, 600.0), 900.0),
            348.0,
            "below: bottom + margin to the bottom"
        );
        assert_eq!(
            v(0.0, (500.0, 600.0), 300.0),
            300.0,
            "below, clamped to the range"
        );
        assert_eq!(
            v(100.0, (150.0, 380.0), 900.0),
            128.0,
            "its bottom's margin is out of view"
        );
        assert_eq!(
            v(100.0, (150.0, 350.0), 900.0),
            100.0,
            "the margin just fits"
        );
    }

    #[test]
    fn the_scroll_margin_is_1_5_u() {
        // The CPU / RAM tap scrolls the block into view with a 1.5 U margin on all sides.
        assert_eq!(SCROLL_MARGIN, 1.5);
    }

    #[test]
    fn tempo_steps_stay_in_range() {
        assert_eq!(tempo_step(140.0, 1.0), 141.0);
        assert_eq!(tempo_step(140.5, -1.0), 139.5);
        assert_eq!(tempo_step(TEMPO_RANGE.0, -1.0), TEMPO_RANGE.0);
        assert_eq!(tempo_step(TEMPO_RANGE.1, 1.0), TEMPO_RANGE.1);
    }
}

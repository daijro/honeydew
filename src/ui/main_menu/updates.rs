//! The UPDATES tab (#66), not on the web, which is always current. One page that scrolls, as PROJECT does:
//! - `Version <this>` with `Check now` at its right, and what the check found: the latest version or why it failed;
//! - for a newer release, `Install <version>` (Android) or `Download <version>` (desktop), its progress in the
//!   button, and `Release page`; then what the download did;
//! - `Check for updates automatically`;
//! - `Sound library:`, its own version installed (or `Not installed`) and the newest one this build reads, with
//!   `Download sound library` without one or `Update sound library to <version>` while a newer one is out, and on
//!   Android `Download sounds over mobile data`;
//! - the latest release's notes.
//!
//! The work runs in [`crate::update`], on workers: the tab only shows [`update::status`] and starts it.

use std::time::Duration;

use eframe::egui::{Align2, Id, Rect, Ui, pos2, vec2};

use super::{MainMenuAction, light_button, scroll};
use crate::content::install::{self, SHIPPED_RELEASE};
use crate::metrics::FontSize;
use crate::settings::Settings;
use crate::ui::UiCtx;
use crate::ui::scroller::Scroller;
use crate::ui::widgets::{Control, ParamMenu, Pill, pill_toggle, text_label as label};
use crate::update::{self, Job, Status};

// Sizes in U.
const LINE_H: f32 = 1.0;
const HEADER_H: f32 = 1.25;
const BUTTON_H: f32 = 1.25;
const BUTTON_W: f32 = 6.0;
const TOGGLE_H: f32 = 1.5;
const GAP: f32 = 0.25;
/// The dividers' alpha (`text`), as PROJECT's.
const DIVIDER_ALPHA: f32 = 0.18;

/// The tab's state: how far it is scrolled.
#[derive(Default)]
pub(super) struct UpdatesTab {
    scroll: f32,
    /// The page's touch and fling ([`scroll`]).
    kinetic: Scroller,
    /// The page's height in the last frame, for the scroll range.
    content_h: f32,
}

/// The update's button label: `Install` on Android, which installs it, `Download` elsewhere.
const INSTALL: &str = if cfg!(target_os = "android") {
    "Install"
} else {
    "Download"
};

/// What the check found, in a line.
fn found(status: &Status) -> String {
    match (&status.latest, &status.check_failed) {
        _ if status.checking => "Checking for updates...".into(),
        (_, Some(why)) => format!("Could not check for updates. {why}"),
        (Some(latest), _) if latest.newer() => {
            format!("Version {} is available.", latest.version())
        }
        (Some(_), _) => "This is the latest version.".into(),
        (None, _) => "Not checked yet.".into(),
    }
}

/// The sound library's state, in a line, and the label of the button that fetches it, where one shows: `Download
/// sound library` without one (or with one cut short), `Update sound library to <version>` while a newer one this
/// build reads is out ([`update::sounds_wanted`]); none while the installed one is the newest, or one is downloading.
fn sounds(status: &Status) -> (String, Option<String>) {
    let bundle = crate::content::factory().bundle.as_deref();
    let installed = bundle.map(|b| b.index.release.as_str());
    let newest = (status.sounds_latest.clone()).unwrap_or_else(|| SHIPPED_RELEASE.trim().into());
    let download = "Download sound library".to_owned();
    let button = match installed {
        None => Some(download),
        Some(v) if install::older(v, &newest) => Some(format!("Update sound library to {newest}")),
        Some(_) if install::needed(bundle) => Some(download),
        Some(_) => None,
    };
    let mb = |bytes: u64| bytes.div_ceil(1_000_000);
    let line = match &status.sounds {
        Job::Running(..) if status.waiting_for_wifi => "Waiting for Wi-Fi to download it.".into(),
        Job::Running(done, total) if status.copying_sounds => {
            format!(
                "Copying the app's own sounds {}.",
                Job::percent(*done, *total)
            )
        }
        Job::Running(done, total) if status.paused => {
            let percent = Job::percent(*done, *total);
            format!(
                "Paused at {percent} ({} of {} MB).",
                mb(*done),
                mb(*total)
            )
        }
        Job::Running(done, total) => {
            let percent = Job::percent(*done, *total);
            format!(
                "Downloading {percent} ({} of {} MB).",
                mb(*done),
                mb(*total)
            )
        }
        Job::Done(done) => done.clone(),
        Job::Failed(why) => format!("Could not download it. {why}"),
        Job::Idle => match installed {
            None => format!("Not installed. Version {newest} is available."),
            Some(v) if install::older(v, &newest) => {
                format!("Version {v} is installed. Version {newest} is available.")
            }
            Some(v) if button.is_some() => format!("Version {v} is incomplete."),
            Some(v) if status.sounds_latest.is_some() => format!("Version {v}, the newest."),
            Some(v) => format!("Version {v}."),
        },
    };
    let idle = matches!(status.sounds, Job::Idle | Job::Failed(_));
    (line, button.filter(|_| idle && install::dir().is_some()))
}

/// Release notes as text: Markdown's heading marks and carriage returns gone.
fn notes(body: &str) -> String {
    let lines = body
        .lines()
        .map(|line| line.trim_start_matches('#').trim_start());
    lines.collect::<Vec<_>>().join("\n").trim().to_owned()
}

impl UpdatesTab {
    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        area: Rect,
        settings: &mut Settings,
    ) -> Option<MainMenuAction> {
        let m = cx.m;
        let text = cx.theme.colors.text.to_color32();
        let id = Id::new("main_menu_updates");
        let status = update::status();
        if status.checking || status.update.running() || status.sounds.running() {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        let mut action = None;
        let offset = scroll(
            ui,
            id.with("scroll"),
            m.u(1.0),
            &mut self.kinetic,
            area,
            self.content_h,
            &mut self.scroll,
        );
        let outer_clip = ui.clip_rect();
        ui.set_clip_rect(outer_clip.intersect(area));
        let top = area.top() - offset;
        let mut y = top;
        let mut band = |h: f32| {
            let r = Rect::from_x_y_ranges(area.x_range(), y..=y + h);
            y += h;
            r
        };
        let line = |ui: &Ui, rect: Rect, s: &str, font| {
            label(ui, cx, rect, s, font, Align2::LEFT_CENTER, text);
        };
        let divider = |ui: &Ui, rect: Rect| {
            let y = m.floor_px(rect.center().y);
            let rule = Rect::from_x_y_ranges(rect.x_range(), y..=y + 1.0 / m.ppp());
            (ui.painter()).rect_filled(rule, 0.0, text.gamma_multiply(DIVIDER_ALPHA));
        };
        // A toggle of `on`, its label right of its pill.
        let toggle = |ui: &mut Ui, rect: Rect, name: &str, on: &mut bool| {
            let c = Control {
                menu: ParamMenu::Unbound,
                label_font: FontSize::Pt10,
                ..Control::new(id.with(name), rect, name, text, 0.0)
            };
            let pill = Pill {
                label_right: true,
                ..Pill::default()
            };
            let mut v = if *on { 1.0 } else { 0.0 };
            let changed = pill_toggle(ui, cx, &c, pill, &mut v, |_| String::new()).changed();
            if changed {
                *on = v >= 0.5;
            }
        };
        let button_w = m.u(BUTTON_W).min((area.width() - m.u(GAP)) / 2.0);

        // This version and `Check now`.
        let row = band(m.u(HEADER_H));
        let check = Rect::from_x_y_ranges(row.right() - button_w..=row.right(), row.y_range());
        let this = format!("Version {}", super::version());
        line(ui, row.with_max_x(check.left()), &this, FontSize::Pt12);
        let checking = status.checking;
        if light_button(ui, cx, id.with("check"), check, "Check now", !checking) {
            update::check(false);
        }
        band(m.u(GAP));
        line(ui, band(m.u(LINE_H)), &found(&status), FontSize::Pt10);

        // The newer release: its download and its page.
        if let Some(latest) = &status.latest {
            band(m.u(GAP));
            let row = band(m.u(BUTTON_H));
            let left = Rect::from_x_y_ranges(row.left()..=row.left() + button_w, row.y_range());
            let right = left.translate(vec2(button_w + m.u(GAP), 0.0));
            if latest.newer() {
                let asset = latest.asset().is_some();
                let name = match &status.update {
                    Job::Running(done, total) if status.copying_sounds => {
                        format!("Keeping sounds {}", Job::percent(*done, *total))
                    }
                    Job::Running(done, total) => {
                        format!("Downloading {}", Job::percent(*done, *total))
                    }
                    _ => format!("{INSTALL} {}", latest.version()),
                };
                let ready = asset && !status.update.running();
                if light_button(ui, cx, id.with("install"), left, &name, ready) {
                    update::install();
                }
                let told = match &status.update {
                    Job::Done(done) => Some(done.clone()),
                    Job::Failed(why) => Some(format!("Could not download it. {why}")),
                    _ if !asset => {
                        Some("No download for this system: see the release page.".into())
                    }
                    _ => None,
                };
                if light_button(ui, cx, id.with("page"), right, "Release page", true) {
                    action = Some(MainMenuAction::OpenLink(latest.html_url.clone()));
                }
                if let Some(told) = told {
                    band(m.u(GAP));
                    line(ui, band(m.u(LINE_H)), &told, FontSize::Pt10);
                }
            } else if light_button(ui, cx, id.with("page"), left, "Release page", true) {
                action = Some(MainMenuAction::OpenLink(latest.html_url.clone()));
            }
        }
        band(m.u(GAP));
        let auto = band(m.u(TOGGLE_H));
        toggle(
            ui,
            auto,
            "Check for updates automatically",
            &mut settings.check_updates,
        );

        // The sound library.
        divider(ui, band(m.u(GAP * 2.0)));
        line(ui, band(m.u(HEADER_H)), "Sound library:", FontSize::Pt12);
        let (state, button) = sounds(&status);
        line(ui, band(m.u(LINE_H)), &state, FontSize::Pt10);
        if let Some(label) = button {
            band(m.u(GAP));
            let row = band(m.u(BUTTON_H));
            let at = Rect::from_x_y_ranges(row.left()..=row.left() + button_w, row.y_range());
            if light_button(ui, cx, id.with("sounds"), at, &label, true) {
                update::download_sounds();
            }
        }
        // While the download runs: pause it, or continue a paused one (it goes on from what it has).
        if matches!(status.sounds, Job::Running(..)) {
            band(m.u(GAP));
            let row = band(m.u(BUTTON_H));
            let at = Rect::from_x_y_ranges(row.left()..=row.left() + button_w, row.y_range());
            let label = if status.paused { "Continue download" } else { "Pause download" };
            if light_button(ui, cx, id.with("sounds_pause"), at, label, true) {
                update::pause_sounds(!status.paused);
            }
        }
        if cfg!(target_os = "android") {
            band(m.u(GAP));
            let row = band(m.u(TOGGLE_H));
            let name = "Download sounds over mobile data";
            toggle(ui, row, name, &mut settings.sounds_mobile_data);
        }

        // The latest release's notes.
        if let Some(latest) = status.latest.as_ref().filter(|l| !l.body.trim().is_empty()) {
            divider(ui, band(m.u(GAP * 2.0)));
            let header = format!("What's new in {}:", latest.version());
            line(ui, band(m.u(HEADER_H)), &header, FontSize::Pt12);
            let font = m.font_id(FontSize::Pt10);
            let galley = (ui.painter()).layout(notes(&latest.body), font, text, area.width());
            let at = band(galley.size().y);
            ui.painter().galley(pos2(at.left(), at.top()), galley, text);
            band(m.u(GAP * 2.0));
        }
        self.content_h = y - top;
        ui.set_clip_rect(outer_clip);
        action
    }
}

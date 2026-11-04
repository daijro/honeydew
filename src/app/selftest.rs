//! The iOS self-test. Launched with [`ARG`] (`xcrun simctl launch … --selftest`: a tap on the app's icon passes no
//! arguments), the app opens the demo song from the user folder, plays it and checks that the master output sounds,
//! opens the main menu and a rack, then records a take from the microphone onto a new Audio Record channel (which arms
//! REC). With [`DENIED_ARG`] (the microphone denied) it adds the channel and checks for the denial message instead. Each step logs
//! `SELFTEST STEP <name>` ([`say`]) and waits for the workflow's screenshot of it (`tools/ios-selftest.sh`, [`shot`]);
//! the run ends with `SELFTEST PASS` or `SELFTEST FAIL: <reason>` and exits. The taps go through egui's input
//! ([`input`]), as a finger's do.

use std::path::PathBuf;
use std::sync::{Mutex, Once, PoisonError};
use std::time::{Duration, Instant};

use super::*;

const ARG: &str = "--selftest";
const DENIED_ARG: &str = "--selftest-mic-denied";
/// How long a step shows once its screenshot is taken.
const HOLD: Duration = Duration::from_secs(1);
/// How long the demo plays, and the take records.
const PLAY: Duration = Duration::from_secs(3);
/// The longest a step's screenshot is waited for: a run without the workflow goes on without.
const SHOT_WAIT: Duration = Duration::from_secs(90);
/// The longest a step may wait for what it waits on.
const TIMEOUT: Duration = Duration::from_secs(30);
/// The quietest master peak that counts as sound.
const AUDIBLE: f32 = 1e-3;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Start,
    Demo,
    Playing,
    Stopped,
    Menu,
    Rack,
    Armed,
    Recording,
    Recorded,
    DeniedPlaying,
    Denied,
}

struct Run {
    denied: bool,
    step: Step,
    since: Instant,
    /// The steps logged, and of those the ones the workflow took its screenshot of.
    logged: u32,
    shown: u32,
    /// The loudest master peak while the demo played.
    loudest: f32,
    /// A tap under way on the control of this id: pressed in one frame, released in the next.
    tap: Option<(egui::Id, bool)>,
}

static RUN: Mutex<Option<Run>> = Mutex::new(None);

fn play() -> egui::Id {
    egui::Id::new(("transport", "play"))
}

/// `name` in the app's `Library/Caches`, where the workflow reads the log ([`say`]) and writes how many steps it took
/// a screenshot of ([`shot`]): the Simulator hands the app's own output on late.
fn cache(name: &str) -> Option<PathBuf> {
    let dir = std::path::Path::new(&std::env::var_os("HOME")?).join("Library/Caches");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(name))
}

/// Prints `line` and adds it to the log.
fn say(line: &str) {
    use std::io::Write as _;
    println!("{line}");
    let log = cache("selftest.log");
    let file = log.and_then(|log| {
        (std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log))
        .ok()
    });
    if let Some(mut file) = file {
        let _ = writeln!(file, "{line}");
    }
}

/// How many steps the workflow took a screenshot of.
fn shot() -> u32 {
    let text = cache("selftest.shot").and_then(|path| std::fs::read_to_string(path).ok());
    text.and_then(|t| t.trim().parse().ok()).unwrap_or(0)
}

/// Ends the run on a failure.
fn finish_on(check: Result<(), String>) {
    if check.is_err() {
        finish(check);
    }
}

/// The verdict. A failure stays on screen a moment, for its screenshot.
fn finish(verdict: Result<(), String>) -> ! {
    match verdict {
        Ok(()) => say("SELFTEST PASS"),
        Err(why) => {
            say(&format!("SELFTEST FAIL: {why}"));
            std::thread::sleep(Duration::from_secs(5));
        }
    }
    std::process::exit(0);
}

/// The app covers the whole screen: its viewport is the screen's size, and its content reaches every edge but the
/// notch's side (`ios::notch_only`). Logs the sizes.
fn covers_screen(ctx: &egui::Context) -> Result<(), String> {
    let (screen, viewport, content) = (
        crate::ios::screen_size(),
        ctx.viewport_rect(),
        ctx.content_rect(),
    );
    say(&format!(
        "SELFTEST screen {screen:?} viewport {viewport:?} content {content:?}"
    ));
    if (viewport.size() - screen).length() > 1.0 {
        return Err(format!(
            "the app is {:?} on a {screen:?} screen",
            viewport.size()
        ));
    }
    let insets = [
        content.left() - viewport.left(),
        viewport.right() - content.right(),
        content.top() - viewport.top(),
        viewport.bottom() - content.bottom(),
    ];
    match insets.iter().filter(|inset| **inset > 0.5).count() {
        0 | 1 => Ok(()),
        _ => Err(format!(
            "the app leaves more than the notch's side empty: {insets:?}"
        )),
    }
}

/// Logs step `name` and makes it the run's.
fn next(run: &mut Run, step: Step, name: &str) {
    say(&format!("SELFTEST STEP {name}"));
    run.logged += 1;
    (run.step, run.since) = (step, Instant::now());
}

/// The frame's taps, before egui reads its input ([`eframe::App::raw_input_hook`]).
pub(super) fn input(ctx: &egui::Context, raw: &mut egui::RawInput) {
    let mut run = RUN.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(run) = run.as_mut() else { return };
    let Some((id, pressed)) = run.tap else { return };
    let Some(pos) = ctx.read_response(id).map(|r| r.rect.center()) else {
        return;
    };
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    if pressed {
        raw.events.extend([button(false), egui::Event::PointerGone]);
        run.tap = None;
    } else {
        raw.events
            .extend([egui::Event::PointerMoved(pos), button(true)]);
        run.tap = Some((id, true));
    }
}

impl HoneydewApp {
    /// One frame of the self-test, after the app's.
    pub(super) fn selftest(&mut self, ctx: &egui::Context) {
        static ARGS: Once = Once::new();
        let mut guard = RUN.lock().unwrap_or_else(PoisonError::into_inner);
        ARGS.call_once(|| {
            let arg = std::env::args().nth(1);
            *guard = matches!(arg.as_deref(), Some(ARG | DENIED_ARG)).then(|| {
                let mut run = Run {
                    denied: arg.as_deref() == Some(DENIED_ARG),
                    step: Step::Start,
                    since: Instant::now(),
                    logged: 0,
                    shown: 0,
                    loudest: 0.0,
                    tap: None,
                };
                next(&mut run, Step::Start, "launch");
                run
            });
        });
        let Some(run) = guard.as_mut() else { return };
        ctx.request_repaint_after(Duration::from_millis(50));
        if run.step == Step::Playing
            && let Some(snapshot) = self.audio.poll()
        {
            let [left, right] =
                crate::ui::mini_mixer::meter_peaks(Some(&snapshot.module_meters), 0);
            run.loudest = run.loudest.max(left).max(right);
        }
        if run.tap.is_some() {
            return;
        }
        // The step's screenshot first; its time starts then.
        if run.shown < run.logged {
            if shot() < run.logged && run.since.elapsed() < SHOT_WAIT {
                return;
            }
            (run.shown, run.since) = (run.logged, Instant::now());
        }
        let waited = run.since.elapsed();
        if waited > TIMEOUT {
            finish(Err(format!("{:?} timed out", run.step)));
        }
        match run.step {
            Step::Start if waited < HOLD => {}
            // The audio starts with a tap, as a user's first one starts it; the input opens on a running output.
            Step::Start if run.denied => {
                run.tap = Some((play(), false));
                run.step = Step::DeniedPlaying;
            }
            Step::DeniedPlaying => {
                self.add_channel(ChannelKind::AudioRecord, 0);
                next(run, Step::Denied, "arm");
            }
            Step::Start => {
                finish_on(covers_screen(ctx));
                // The phone held as the app is mostly used; the denial's run stays upright.
                crate::ios::turn_landscape();
                let name = crate::content::demo_songs().remove(0).0;
                let root = self.files.library.root().map(std::path::Path::to_owned);
                let path = root.map(|root| root.join(crate::content::DEMO_FOLDER).join(name));
                let song = path.and_then(|path| self.files.replace(Replace::Open(path)));
                let Some(song) = song else {
                    finish(Err("the demo song did not open from the user folder".into()));
                };
                self.song = song;
                self.song_replaced(ctx);
                next(run, Step::Demo, "demo");
            }
            Step::Demo if waited >= HOLD => {
                finish_on(covers_screen(ctx));
                run.tap = Some((play(), false));
                next(run, Step::Playing, "play");
            }
            Step::Playing if waited >= PLAY => {
                if !self.audio.running() {
                    finish(Err("the audio output did not start".into()));
                }
                if run.loudest < AUDIBLE {
                    let (peak, playing) = (run.loudest, self.main_screen.playing);
                    finish(Err(format!(
                        "the master output is silent (peak {peak}, playing {playing})"
                    )));
                }
                say(&format!("SELFTEST master peak {:.3}", run.loudest));
                run.tap = Some((play(), false));
                run.step = Step::Stopped;
            }
            // Once the stop's tap is in: the menu would take it.
            Step::Stopped => {
                self.navigate(Nav::Open(Overlay::MainMenu(views::MenuTab::Songs)));
                next(run, Step::Menu, "menu");
            }
            Step::Menu if waited >= HOLD => {
                self.navigate(Nav::Close);
                self.navigate(Nav::Open(Overlay::Rack(1)));
                next(run, Step::Rack, "rack");
            }
            Step::Rack if waited >= HOLD => {
                self.navigate(Nav::Close);
                self.settings.count_in = false;
                self.add_channel(ChannelKind::AudioRecord, 0);
                next(run, Step::Armed, "arm");
            }
            Step::Armed if self.input.ring().is_some() => {
                run.tap = Some((play(), false));
                next(run, Step::Recording, "record");
            }
            Step::Armed if waited >= Duration::from_secs(10) => {
                let state = format!(
                    "armed {}, output rate {:?}, input failed {}, mic {:?}, view {:?}, overlays {}",
                    self.main_screen.armed,
                    self.audio.sample_rate(),
                    self.input_failed,
                    self.input.mic(),
                    self.nav.view,
                    self.nav.overlays.len(),
                );
                let lines = self.message_lines();
                finish(Err(format!(
                    "the microphone did not open: {lines:?} ({state})"
                )));
            }
            Step::Recording if waited >= PLAY => {
                run.tap = Some((play(), false));
                next(run, Step::Recorded, "recorded");
            }
            Step::Recorded if waited >= HOLD => {
                let channel = self.song.channels.last();
                let clips = channel.map_or(0, |c| c.lanes.iter().map(|l| l.clips.len()).sum());
                finish(match clips {
                    0 => Err("the take left no clip".into()),
                    _ => Ok(()),
                });
            }
            Step::Denied if waited >= HOLD => {
                let lines = self.message_lines();
                say(&format!("SELFTEST message {lines:?}"));
                let denied = lines.iter().any(|l| l.contains("needs the microphone"));
                finish(
                    denied
                        .then_some(())
                        .ok_or("no microphone denial message".into()),
                );
            }
            _ => {}
        }
    }

    /// The lines of the input's message, when it is up.
    fn message_lines(&self) -> Vec<String> {
        (self.nav.overlays.iter())
            .find_map(|o| match o {
                Overlay::Message { tag, lines, .. } if *tag == INPUT_TAG => Some(lines.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }
}

//! The rack's input row in the app: the input opens for what needs it, the row's meter shows its peaks, its monitor
//! button rebuilds the engine, an `INPUT` pick is kept per song and channel and feeds the monitor and the recorder, and
//! `Calibrate` measures the engine's round trip for the next recording's latency drop. No device: a fake opener and
//! pushed frames.

use std::any::Any;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use honeydew_audio::device::AudioRenderer;

use super::*;
use crate::recording::calibrate;
use crate::recording::input::{InputDevice, InputLink, InputRing};
use crate::recording::routing::{self, Route, Routing};
use crate::song::PPQ;
use crate::ui::rack::{METER_RINGS, meter_alpha, meter_radius};

/// REC, in the transport bar's slot 0 (two slots left of play).
const REC: Pos2 = pos2(585.0 - 2.0 * 1.75 * U, BAR_Y);
/// 150 BPM at the Recorder's 48 kHz.
const FRAMES_PER_BEAT: usize = 150 * PPQ as usize;
/// A 25 ms peak window at 48 kHz.
const WINDOW: usize = 1_200;

/// The fake devices: a stereo interface and a mono mic.
fn devices() -> Vec<InputDevice> {
    let device = |name: &str, channels| InputDevice {
        name: name.into(),
        channels,
    };
    vec![device("USB", 2), device("Mic", 1)]
}

/// A widget of the rack of channel `channel`.
fn rack_id(channel: usize, what: &str) -> egui::Id {
    egui::Id::new(("rack", channel)).with(what)
}

/// The undo steps there are.
fn undo_steps(rig: &mut Rig) -> usize {
    let history = &mut rig.app.history;
    let n = std::iter::from_fn(|| history.undo()).count();
    for _ in 0..n {
        history.redo();
    }
    n
}

/// Where the last frame painted `text`.
fn text_at(rig: &Rig, text: &str) -> Pos2 {
    let found = rig.out.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect().center()),
        _ => None,
    });
    found.unwrap_or_else(|| panic!("{text:?} not painted"))
}

/// The open message's lines and tag.
fn message(rig: &Rig) -> Option<(&'static str, Vec<String>)> {
    rig.app.nav.overlays.iter().find_map(|o| match o {
        Overlay::Message { tag, lines, .. } => Some((*tag, lines.clone())),
        _ => None,
    })
}

/// Picks choice `choice` (0 the first) of the open message, after the frame that sizes it.
fn pick(rig: &mut Rig, choice: usize) {
    rig.frame(vec![]);
    // The box wraps the lines to its width, so the choices start at the first row that is a button (lines draw none).
    let first = (0..64)
        .find(|&row| rig.drawn(message_item(row)))
        .expect("a message button");
    rig.click_id(message_item(first + choice));
}

/// A rig that records: its audio runs (the Recorder holds an engine) with a 150 BPM song, its files are on disk in its
/// own user folder, its input devices are fakes (the `USB` one stereo, the others mono), and an `Audio Record` channel
/// (1) was just added, which armed REC.
struct InputRig {
    rig: Rig,
    user: PathBuf,
    /// The device of each open.
    opens: Rc<RefCell<Vec<Option<String>>>>,
}

impl InputRig {
    fn new(name: &str) -> Self {
        let mut rig = Rig::new();
        rig.click(OUTSIDE);
        let user = std::env::temp_dir().join(format!("honeydew-h14-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user);
        rig.app.files = ProjectFiles::on_disk(&user);
        let opens = Rc::<RefCell<Vec<_>>>::default();
        let log = opens.clone();
        rig.app.input = InputLink::new(move |device, rate| {
            assert_eq!(rate, 48_000);
            log.borrow_mut().push(device.map(str::to_owned));
            let ports = if device == Some("USB") { 2 } else { 1 };
            let ring = Arc::new(InputRing::new(ports, rate, 2 * rate as usize));
            Ok((ring, Box::new(()) as Box<dyn Any>))
        })
        .listing(devices);
        rig.app.song.tempo_bpm = 150.0;
        rig.app.add_channel(ChannelKind::AudioRecord, 0);
        rig.frame(vec![]);
        rig.take();
        Self { rig, user, opens }
    }

    fn ring(&self) -> Arc<InputRing> {
        self.rig.app.input.ring().expect("an open input").clone()
    }

    /// Renders the Recorder's engine for `frames` frames, given the output's buffer size first as the device does.
    fn render(&mut self, frames: usize) {
        let mut output = vec![0.0; 2 * frames];
        let mut rec = self.rig.rec.0.borrow_mut();
        let buffer = rec.buffer_frames;
        let engine = rec.engine.as_mut().expect("an engine");
        engine.set_buffer_frames(buffer);
        engine.render(&mut output, 2);
    }

    /// One frame with the playhead at `tick`.
    fn at(&mut self, tick: u64) {
        self.rig.rec.0.borrow_mut().tick = tick;
        self.rig.frame(vec![]);
    }
}

impl Drop for InputRig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.user);
    }
}

/// The level meter's rings around `mic` in the last frame, as (radius, fill), outermost first: the circles centred
/// there but the last, the button's disc (drawn over the newest ring, as wide).
fn rings(rig: &Rig, mic: Pos2) -> Vec<(f32, egui::Color32)> {
    let mut circles: Vec<_> = (rig.out.shapes.iter())
        .filter_map(|s| match &s.shape {
            egui::Shape::Circle(c) if c.center == mic => Some((c.radius, c.fill)),
            _ => None,
        })
        .collect();
    assert_eq!(
        circles.pop().map(|c| c.0),
        Some(0.7 * U),
        "the button's disc last"
    );
    circles
}

#[test]
fn an_audio_record_racks_input_row_shows_a_live_meter_from_pushed_frames() {
    // The acceptance.
    let mut a = InputRig::new("meter");
    a.rig.click(REC);
    assert!(
        a.rig.app.input.ring().is_none(),
        "disarmed, nothing needs it"
    );
    // The rack of the Audio Record channel opens it.
    a.rig.app.navigate(Nav::Open(Overlay::Rack(1)));
    a.rig.frame(vec![]);
    let ring = a.ring();
    assert_eq!(*a.opens.borrow(), [None, None], "the default device");
    // Ten 25 ms windows, each louder than the one before.
    let peak = |k: usize| 0.05 * (10 - k) as f32;
    for k in (0..10).rev() {
        ring.push(std::iter::repeat_n(peak(k), WINDOW));
    }
    // The row fades in over 12 frames.
    for _ in 0..12 {
        a.rig.frame(vec![]);
    }
    let mic = a
        .rig
        .ctx
        .read_response(rack_id(1, "record"))
        .unwrap()
        .rect
        .center();
    let rack_width = a
        .rig
        .ctx
        .read_response(rack_id(1, "rack"))
        .unwrap()
        .rect
        .width();
    let ink =
        crate::theme::channel_color(a.rig.app.song.channels[1].hue, &a.rig.app.theme).to_color32();
    let check = |rig: &Rig, base: f32| {
        let drawn = rings(rig, mic);
        assert_eq!(drawn.len(), 10, "a ring per loud window: {drawn:?}");
        for (&(radius, fill), k) in drawn.iter().zip((0..10).rev()) {
            assert!(
                (radius - meter_radius(0.7 * U, k, rack_width)).abs() < 1e-3,
                "ring {k}"
            );
            let want = ink.gamma_multiply(base * meter_alpha(peak(k), k));
            let near =
                (fill.to_array().iter().zip(want.to_array())).all(|(a, b)| a.abs_diff(b) <= 1);
            assert!(near, "ring {k}: {fill:?}, want {want:?}");
        }
    };
    // Faint while not monitoring.
    check(&a.rig, 0.15);
    // Live: a newer silent window moves every ring out by one.
    ring.push(std::iter::repeat_n(0.0, WINDOW));
    a.rig.frame(vec![]);
    let drawn = rings(&a.rig, mic);
    assert_eq!(drawn.len(), 10);
    assert!((drawn[9].0 - meter_radius(0.7 * U, 1, rack_width)).abs() < 1e-3);

    // The mic button turns monitoring on: one undo step, a new engine that monitors the input.
    ring.push(std::iter::repeat_n(0.0, (METER_RINGS + 16) * WINDOW));
    for k in (0..10).rev() {
        ring.push(std::iter::repeat_n(peak(k), WINDOW));
    }
    let undos = undo_steps(&mut a.rig);
    a.rig.click_id(rack_id(1, "record"));
    assert!(a.rig.app.song.channels[1].file.input_monitoring);
    assert_eq!(undo_steps(&mut a.rig), undos + 1);
    assert_eq!(
        a.rig.take(),
        ["Metronome false 0.500", "ReplaceSong adopting no-resync"]
    );
    a.rig.frame(vec![]);
    check(&a.rig, 0.5);
    // Closed, the rack needs it no more; the monitoring does.
    a.rig.app.nav.overlays.clear();
    a.rig.frame(vec![]);
    assert!(
        a.rig
            .app
            .input
            .ring()
            .is_some_and(|r| Arc::ptr_eq(r, &ring))
    );
    // Monitoring off and the rack closed: nothing needs it.
    a.rig.app.song.channels[1].file.input_monitoring = false;
    a.rig.frame(vec![]);
    assert!(a.rig.app.input.ring().is_none());
    assert!(a.rig.app.audio.input.is_none());
}

#[test]
fn an_input_pick_is_kept_for_the_channel_opens_its_device_and_feeds_the_monitor_and_the_recorder() {
    let mut a = InputRig::new("pick");
    let spy = Spy::default();
    a.rig.app.storage = Some(Box::new(spy.clone()));
    a.rig.app.navigate(Nav::Open(Overlay::Rack(1)));
    a.rig.slide_rack();
    a.rig.click_id(rack_id(1, "input"));
    // The app lists the devices after that frame; the popup's first frame sizes it.
    a.rig.frame(vec![]);
    a.rig.frame(vec![]);
    let (text, bg) = (
        a.rig.app.theme.colors.text,
        a.rig.app.theme.colors.background,
    );
    assert_eq!(
        a.rig.color_of("Inputs 1 + 2"),
        Some(bg.to_color32()),
        "nothing marked yet"
    );
    assert!(a.rig.painted("USB") && a.rig.painted("Mic"));
    let undos = undo_steps(&mut a.rig);
    let at = text_at(&a.rig, "Inputs 1 + 2");
    a.rig.click(at);
    a.rig.frame(vec![]);
    assert_eq!(undo_steps(&mut a.rig), undos, "no song edit");
    // Kept for this song's channel, and stored at once.
    let (song, channel) = (&a.rig.app.song, &a.rig.app.song.channels[1]);
    let pair = Route {
        device: "USB".into(),
        first: 0,
        second: Some(1),
    };
    assert_eq!(a.rig.app.routing.get(song, channel), Some(&pair));
    let stored = Routing::load(&spy);
    assert_eq!(stored.get(song, channel), Some(&pair));
    assert!(spy.0.borrow().map.contains_key(routing::STORAGE_KEY));
    assert!(spy.counts().1 >= 1, "flushed");
    // The input reopened on that device; the engines get the pair for the channel.
    assert_eq!(*a.opens.borrow(), [None, Some("USB".to_owned())]);
    let id = channel.id;
    let ports = a.rig.app.audio.input.as_ref().map(|i| i.ports.clone());
    assert_eq!(ports, Some(vec![(id, 0, Some(1))]));
    // The meter takes the louder port of the pair: three windows on the right one alone.
    let silent_left = |i: usize| if i.is_multiple_of(2) { 0.0 } else { 0.5 };
    a.ring().push((0..2 * 3 * WINDOW).map(silent_left));
    a.rig.frame(vec![]);
    let mic = a
        .rig
        .ctx
        .read_response(rack_id(1, "record"))
        .unwrap()
        .rect
        .center();
    assert_eq!(rings(&a.rig, mic).len(), 3);
    // Opened again, the menu marks it.
    a.rig.click_id(rack_id(1, "input"));
    a.rig.frame(vec![]);
    a.rig.frame(vec![]);
    assert_eq!(a.rig.color_of("Inputs 1 + 2"), Some(text.to_color32()));
    assert_eq!(a.rig.color_of("Input 2"), Some(bg.to_color32()));
    // The rack slides out of the transport's way.
    a.rig.app.nav.overlays.clear();
    a.rig.slide_rack();

    // A recording takes the pair: a stereo file.
    a.rig.app.settings.count_in = false;
    a.rig.click(PLAY);
    let ring = a.ring();
    assert_eq!(ring.channels(), 2);
    ring.push(std::iter::repeat_n(0.5, 2 * FRAMES_PER_BEAT));
    a.render(FRAMES_PER_BEAT);
    a.at(PPQ.into());
    a.at(PPQ.into());
    a.rig.click(PLAY);
    let file = a.user.join("My Recordings").join("Rec.wav");
    let bytes = std::fs::read(&file).unwrap();
    assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2, "stereo");
    assert_eq!(bytes.len(), 46 + 2 * 2 * FRAMES_PER_BEAT);

    // Another app on the same storage has it.
    let app = HoneydewApp::with_storage(Some(Box::new(spy.clone())));
    assert_eq!(app.routing, a.rig.app.routing);
}

/// Answers the calibration's question with its first choice (`Calibrate`, `Test again`): the next frame shows the
/// progress message, and the one after opens the input for it and starts the engine's measurement.
fn start_calibration(a: &mut InputRig) {
    pick(&mut a.rig, 0);
    a.rig.frame(vec![]);
    assert_eq!(
        message(&a.rig).map(|m| m.1),
        Some(vec![calibrate::SENDING.to_owned()])
    );
    a.rig.frame(vec![]);
}

/// Runs the Recorder's engine in blocks of 256 frames until its measurement has a result (4 s at most), its output
/// heard back on the first port of the open input `delay` frames late at half level (`None`: never heard), the other
/// ports loud.
fn measure(a: &mut InputRig, delay: Option<usize>) {
    let ring = a.ring();
    let ports = ring.channels();
    let mut sent: Vec<f32> = Vec::new();
    for _ in 0..4 * 48_000 / 256 {
        let t0 = sent.len();
        let heard = |t: usize| delay.filter(|&d| t >= d).map_or(0.0, |d| 0.5 * sent[t - d]);
        let block: Vec<f32> = (t0..t0 + 256)
            .flat_map(|t| (0..ports).map(move |p| (t, p)))
            .map(|(t, p)| if p == 0 { heard(t) } else { 0.9 })
            .collect();
        ring.push(block.into_iter());
        let mut output = vec![0.0; 2 * 256];
        let mut rec = a.rig.rec.0.borrow_mut();
        let engine = rec.engine.as_mut().expect("an engine");
        engine.render(&mut output, 2);
        sent.extend(output.as_chunks::<2>().0.iter().map(|f| f[0]));
        if engine
            .ping_report()
            .is_some_and(|(_, result)| result.is_some())
        {
            return;
        }
    }
}

#[test]
fn calibrate_measures_the_round_trip_and_the_next_recording_drops_that_much() {
    let mut a = InputRig::new("calibrate");
    a.rig.click(REC);
    a.rig.app.settings.latency_compensation = Some(false);
    // The channel's input is the USB interface's second port; the calibration hears the first device's first port.
    let song = a.rig.app.song.clone();
    let port_2 = Route {
        device: "USB".into(),
        first: 1,
        second: None,
    };
    a.rig.app.routing.set(&song, &song.channels[1], port_2);
    // The settings tab's `Calibrate` (its own test taps it): the question first.
    a.rig.app.slots.settings_tab.calibrate = true;
    a.rig.frame(vec![]);
    assert_eq!(
        message(&a.rig),
        Some(("calibrate", vec![calibrate::INTRO.to_string()]))
    );
    assert!(a.rig.app.input.ring().is_none(), "not yet");
    a.rig.frame(vec![]);
    a.rig.frame(vec![]);
    assert!(a.rig.painted("Calibrate") && a.rig.painted("Cancel"));
    // `Calibrate`: the default input opens, an engine that hears it is built and measures.
    a.rig.take();
    start_calibration(&mut a);
    assert_eq!(*a.opens.borrow(), [None, None]);
    let log = a.rig.take();
    assert_eq!(
        log.first().map(String::as_str),
        Some("ReplaceSong"),
        "{log:?}"
    );
    assert_eq!(log.last().map(String::as_str), Some("Ping 1"), "{log:?}");
    // Its echo 480 frames after the tone goes out: 10 ms at 48 kHz.
    measure(&mut a, Some(480));
    a.rig.frame(vec![]);
    let done = "Succeeded, Latency is: 10.0 ms (480 samples)".to_owned();
    assert_eq!(message(&a.rig), Some(("calibrate", vec![done])));
    assert_eq!(a.rig.app.latency, Some(480));
    a.rig.frame(vec![]);
    assert!(a.rig.app.input.ring().is_none(), "done: nothing needs it");

    // The next recording drops that much from its take's start (compensation is on: the toggle is OFF), not the
    // buffer size.
    a.rig.rec.0.borrow_mut().buffer_frames = 256;
    a.rig.app.nav.overlays.clear();
    a.rig.app.settings.count_in = false;
    a.rig.click(REC);
    a.rig.click(PLAY);
    let ring = a.ring();
    ring.push(std::iter::repeat_n(0.25, ring.channels() * FRAMES_PER_BEAT));
    a.render(FRAMES_PER_BEAT);
    a.at(PPQ.into());
    let frames = a.rig.app.recorder.as_ref().unwrap().takes()[0].frames;
    assert_eq!(frames, (FRAMES_PER_BEAT - 480) as u64);
    a.rig.click(PLAY);

    // A new `Sound mode` forgets it.
    let current = a.rig.app.settings.sound_mode;
    let other = crate::settings::SOUND_MODES.iter().find(|m| m.0 != current);
    a.rig.app.settings.sound_mode = other.expect("another sound mode").0;
    a.rig.frame(vec![]);
    assert_eq!(a.rig.app.latency, None);
}

#[test]
fn calibrating_stops_playback() {
    let mut a = InputRig::new("calibrate-stop");
    a.rig.app.settings.count_in = false;
    a.rig.click(PLAY);
    assert!(a.rig.app.main_screen.playing);
    a.rig.app.slots.settings_tab.calibrate = true;
    a.rig.frame(vec![]);
    a.rig.take();
    start_calibration(&mut a);
    assert!(!a.rig.app.main_screen.playing);
    let log = a.rig.take();
    assert_eq!(log.first().map(String::as_str), Some("Stop"), "{log:?}");
}

#[test]
fn a_calibration_that_hears_no_echo_asks_to_test_again_and_its_progress_message_cancels() {
    let mut a = InputRig::new("calibrate-again");
    a.rig.click(REC);
    a.rig.app.slots.settings_tab.calibrate = true;
    a.rig.frame(vec![]);
    start_calibration(&mut a);
    measure(&mut a, None);
    a.rig.frame(vec![]);
    let timeout = vec![calibrate::failed(calibrate::Failure::Timeout).to_string()];
    assert_eq!(message(&a.rig), Some(("calibrate", timeout)));
    a.rig.frame(vec![]);
    a.rig.frame(vec![]);
    assert!(a.rig.painted("Test again") && a.rig.painted("Cancel"));
    assert_eq!(a.rig.app.latency, None);
    // `Test again` runs it again, as measurement 2: the first one's result is not its; the progress message's `OK`
    // cancels it.
    a.rig.take();
    start_calibration(&mut a);
    assert_eq!(a.rig.take().last().map(String::as_str), Some("Ping 2"));
    a.rig.frame(vec![]);
    assert_eq!(
        message(&a.rig).map(|m| m.1),
        Some(vec![calibrate::SENDING.to_owned()])
    );
    assert!(a.rig.app.input.ring().is_some());
    pick(&mut a.rig, 0);
    a.rig.frame(vec![]);
    assert!(a.rig.app.calibration.is_none());
    assert_eq!(message(&a.rig), None);
    a.rig.frame(vec![]);
    assert!(a.rig.app.input.ring().is_none());
    // The question's `Cancel` ends it too.
    a.rig.app.slots.settings_tab.calibrate = true;
    a.rig.frame(vec![]);
    pick(&mut a.rig, 1);
    a.rig.frame(vec![]);
    assert!(a.rig.app.calibration.is_none());
}

#[test]
fn without_an_input_a_calibration_fails_with_the_unknown_error() {
    let mut a = InputRig::new("calibrate-none");
    a.rig.click(REC);
    a.rig.app.input = InputLink::new(|_, _| Err("no device".into()));
    a.rig.app.slots.settings_tab.calibrate = true;
    a.rig.frame(vec![]);
    start_calibration(&mut a);
    let unknown = vec![calibrate::failed(calibrate::Failure::Unknown).to_string()];
    assert_eq!(message(&a.rig), Some(("calibrate", unknown)));
}

/// Channel 1's input set to `device`'s ports `first` and `second`, as an `INPUT` pick sets it.
fn route(a: &mut InputRig, device: &str, first: usize, second: Option<usize>) {
    let app = &mut a.rig.app;
    let route = Route {
        device: device.into(),
        first,
        second,
    };
    app.routing.set(&app.song, &app.song.channels[1], route);
}

/// `frames` frames of 0.5 on every port of the open input, which the Recorder's engine renders.
fn feed(a: &mut InputRig, frames: usize) {
    let ring = a.ring();
    ring.push(std::iter::repeat_n(0.5, frames * ring.channels()));
    a.render(frames);
}

/// The takes of the recording under way, as (lane, start tick, frames).
fn takes(a: &InputRig) -> Vec<(usize, u64, u64)> {
    let recorder = a.rig.app.recorder.as_ref().expect("a recording");
    (recorder.takes().iter())
        .map(|t| (t.lane, t.start_tick, t.frames))
        .collect()
}

#[test]
fn a_port_pick_mid_take_with_monitoring_on_hands_the_engine_over_and_the_take_goes_on() {
    // An edit's engine takes the new ports, the transport and the take over. A fresh one would jump
    // back to the frame's tick, behind the engine here, and split the take.
    let mut a = InputRig::new("port-pick");
    route(&mut a, "USB", 0, Some(1));
    a.rig.app.song.channels[1].file.input_monitoring = true;
    a.rig.app.song_edited(SongEdit::Final, 0);
    a.rig.app.settings.count_in = false;
    a.rig.frame(vec![]);
    a.rig.click(PLAY);
    feed(&mut a, FRAMES_PER_BEAT);
    a.at(PPQ.into());
    feed(&mut a, FRAMES_PER_BEAT / 2);
    a.rig.take();
    route(&mut a, "USB", 1, None);
    a.at(PPQ.into());
    let log = a.rig.take();
    assert!(
        log.iter().any(|c| c.starts_with("ReplaceSong adopting")),
        "{log:?}"
    );
    assert!(
        !log.iter()
            .any(|c| c == "ReplaceSong" || c.starts_with("Seek")),
        "{log:?}"
    );
    let id = a.rig.app.song.channels[1].id;
    let ports = a.rig.app.audio.input.as_ref().map(|i| i.ports.clone());
    assert_eq!(ports, Some(vec![(id, 1, None)]));
    feed(&mut a, FRAMES_PER_BEAT / 2);
    a.at(2 * u64::from(PPQ));
    assert_eq!(takes(&a), [(0, 0, 2 * FRAMES_PER_BEAT as u64)]);
}

#[test]
fn an_input_of_another_width_mid_take_ends_the_recording_with_its_clip_and_records_on_from_it() {
    // A tap is made for a width and rate, so another device of another width ends the
    // recording, its take so far a clip. REC stays armed: the next recording takes the new input at once.
    let mut a = InputRig::new("width");
    a.rig.app.settings.count_in = false;
    a.rig.frame(vec![]);
    a.rig.click(PLAY);
    assert_eq!(a.ring().channels(), 1);
    feed(&mut a, FRAMES_PER_BEAT);
    a.at(PPQ.into());
    route(&mut a, "USB", 0, Some(1));
    a.at(PPQ.into());
    assert_eq!(a.ring().channels(), 2);
    let clips = &a.rig.app.song.channels[1].lanes[0].clips;
    assert_eq!(clips.len(), 1, "the take so far");
    assert_eq!(
        (clips[0].start_tick, clips[0].file.source_len_beats),
        (0, 1.0)
    );
    let bytes = std::fs::read(a.user.join("My Recordings").join("Rec.wav")).unwrap();
    assert_eq!(bytes.len(), 46 + 2 * FRAMES_PER_BEAT);
    assert!(a.rig.app.main_screen.armed);
    feed(&mut a, FRAMES_PER_BEAT);
    a.at(2 * u64::from(PPQ));
    assert_eq!(takes(&a), [(0, PPQ.into(), FRAMES_PER_BEAT as u64)]);
    a.rig.click(PLAY);
    let clips = &a.rig.app.song.channels[1].lanes[0].clips;
    assert_eq!(clips.len(), 2);
}

#[test]
fn an_output_at_another_rate_mid_take_ends_the_recording_and_the_next_one_takes_the_new_rate() {
    // A tap is made for a rate too, so the input reopened at the output's new rate gets a new one. A kept
    // tap would write the new rate's frames under the old rate's header.
    let mut a = InputRig::new("rate");
    a.rig.app.input = InputLink::new(|_, rate| {
        let ring = Arc::new(InputRing::new(1, rate, 2 * rate as usize));
        Ok((ring, Box::new(()) as Box<dyn Any>))
    });
    a.rig.app.settings.count_in = false;
    a.rig.frame(vec![]);
    a.rig.click(PLAY);
    feed(&mut a, FRAMES_PER_BEAT);
    a.at(PPQ.into());
    a.rig.rec.0.borrow_mut().sample_rate = Some(44_100);
    a.at(PPQ.into());
    assert_eq!(a.ring().rate(), 44_100);
    let clips = &a.rig.app.song.channels[1].lanes[0].clips;
    assert_eq!(clips.len(), 1, "the take so far");
    let rate = (a.rig.app.recorder.as_ref()).map(|r| r.tap().frames().rate());
    assert_eq!(rate, Some(44_100));
}

#[test]
fn an_input_reopened_during_the_calibration_is_heard_by_the_measurement() {
    // With no monitoring and REC off, the calibration alone needs the input. Its stream fails mid-measurement
    // and the next frame reopens it: an engine built with the new ring takes the measurement over, or it times out.
    let mut a = InputRig::new("calibrate-reopen");
    a.rig.click(REC);
    a.rig.app.slots.settings_tab.calibrate = true;
    a.rig.frame(vec![]);
    a.rig.frame(vec![]);
    a.rig.frame(vec![]);
    start_calibration(&mut a);
    let failed = a.ring();
    failed.push(std::iter::repeat_n(0.0, 4 * 256));
    a.render(4 * 256);
    failed.set_failed();
    a.rig.frame(vec![]);
    assert!(!Arc::ptr_eq(&a.ring(), &failed), "reopened");
    measure(&mut a, Some(480));
    a.rig.frame(vec![]);
    let done = "Succeeded, Latency is: 10.0 ms (480 samples)".to_owned();
    assert_eq!(message(&a.rig), Some(("calibrate", vec![done])));
}

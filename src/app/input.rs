//! The audio input in the app (`HoneydewApp::open_input` opens it): the rack's `INPUT` menu and the inputs kept per
//! song and channel ([`HoneydewApp::rack_input`]), and the latency calibration the settings tab's `Calibrate` starts
//! ([`HoneydewApp::calibrate`]).

use eframe::egui;
use honeydew_audio::device::{AudioCommand, PingReport};

use super::HoneydewApp;
use crate::recording::calibrate::{self, Failure};
use crate::song::ChannelKind;
use crate::ui::views::{Answer, Nav, Overlay};

/// The tags of the calibration's question (and of its failure, which asks again) and of its progress message.
const CALIBRATE_TAG: &str = "calibrate";
const SENDING_TAG: &str = "calibrating";

/// The latency calibration under way.
pub(super) enum Calibrating {
    /// Its question is open: `Calibrate` (or `Test again`) starts the test, `Cancel` ends it.
    Asked,
    /// The test runs: the number of its measurement once the engine has it, `None` until the input is open.
    Running(Option<u32>),
}

impl HoneydewApp {
    /// The song's channel the open rack shows, if one is open.
    pub(super) fn rack_channel(&self) -> Option<usize> {
        (self.nav.overlays.iter()).find_map(|o| match o {
            Overlay::Rack(channel) => Some(*channel),
            _ => None,
        })
    }

    /// Each Audio Record channel's stored input ports, by channel id, as (first, second): what the monitor and the
    /// recorder take. A channel without a stored input takes `Input 1`.
    pub(super) fn input_ports(&self) -> Vec<(i32, usize, Option<usize>)> {
        let records = self.song.channels.iter();
        let records = records.filter(|c| c.kind == ChannelKind::AudioRecord);
        records
            .filter_map(|c| {
                let route = self.routing.get(&self.song, c)?;
                Some((c.id, route.first, route.second))
            })
            .collect()
    }

    /// The rack's `INPUT` menu: when it opens, the input devices, with the channel's input marked among them; a pick is
    /// kept for the channel of this song, stored at once. Not a song edit.
    pub(super) fn rack_input(&mut self) {
        let rack = self.rack_channel().and_then(|c| self.song.channels.get(c));
        if std::mem::take(&mut self.slots.rack.list_inputs) {
            let devices = self.input.devices();
            let route = rack.and_then(|channel| self.routing.get(&self.song, channel));
            let current = route.and_then(|route| route.choice(&devices));
            self.slots.rack.inputs(devices, current);
        }
        if let Some((channel, route)) = self.slots.rack.route.take()
            && let Some(channel) = self.song.channels.get(channel)
        {
            self.routing.set(&self.song, channel, route);
            if let Some(storage) = self.storage.as_deref_mut() {
                self.routing.save(storage);
                // Flushed with the settings, at once once no button is down ([`Self::keep_settings`]).
                self.settings_unflushed = true;
            }
        }
    }

    /// The latency calibration: the settings tab's `Calibrate` asks first; its `Calibrate` stops playback (held stopped
    /// while it runs), opens the first input device (`HoneydewApp::open_input`), hands the engine that input and starts
    /// its measurement, and shows `Sending test signal...` until the engine's `report` has the result: the latency,
    /// kept for this session ([`Self::latency`]), or the failure, which asks `Test again` / `Cancel`. No input or no
    /// audio output is the unknown error. Closing the progress message (its `OK`) cancels. A new `Sound mode` forgets
    /// The latency.
    pub(super) fn calibrate(&mut self, ctx: &egui::Context, report: Option<PingReport>, tick: u64) {
        if self.settings.sound_mode != self.settings_saved.sound_mode {
            self.latency = None;
        }
        if std::mem::take(&mut self.slots.settings_tab.calibrate) {
            self.calibration = Some(Calibrating::Asked);
            self.ask_calibration(calibrate::INTRO, "Calibrate");
        }
        match &mut self.calibration {
            None => {}
            Some(Calibrating::Asked) => match self.nav.take_choice(CALIBRATE_TAG) {
                None => {}
                Some(Answer::Yes) => {
                    self.calibration = Some(Calibrating::Running(None));
                    self.navigate(Nav::Open(Overlay::Message {
                        tag: SENDING_TAG,
                        lines: vec![calibrate::SENDING.to_string()],
                        confirm: None,
                        extra: None,
                    }));
                    // The input opens for it next frame.
                    ctx.request_repaint();
                }
                Some(_) => self.calibration = None,
            },
            Some(Calibrating::Running(_)) if self.nav.take_choice(SENDING_TAG).is_some() => {
                self.calibration = None;
            }
            Some(Calibrating::Running(None)) => {
                if self.input.ring().is_none() || !self.audio.running() {
                    return self.calibrated(Err(Failure::Unknown));
                }
                // An engine that hears this input, then the measurement, numbered past the last one it reports.
                self.rebuild(tick);
                let id = report.map_or(0, |(id, _)| id).wrapping_add(1);
                self.audio.send(AudioCommand::Ping(id));
                self.calibration = Some(Calibrating::Running(Some(id)));
                ctx.request_repaint();
            }
            Some(Calibrating::Running(Some(id))) => {
                ctx.request_repaint();
                let rate = self.audio.sample_rate();
                match (report, rate) {
                    (Some((at, Some(result))), Some(rate)) if at == *id => {
                        self.calibrated(result.map(|samples| (samples, rate)));
                    }
                    (_, None) => self.calibrated(Err(Failure::Unknown)),
                    _ => {}
                }
            }
        }
    }

    /// The calibration's message with `Calibrate` (or `Test again`) and `Cancel`, tagged [`CALIBRATE_TAG`].
    fn ask_calibration(&mut self, line: &str, yes: &str) {
        self.navigate(Nav::Open(Overlay::Message {
            tag: CALIBRATE_TAG,
            lines: vec![line.to_string()],
            confirm: Some((yes.into(), "Cancel".into())),
            extra: None,
        }));
    }

    /// The calibration ended with `result`, the latency in frames at a rate: kept and shown, or the failure shown with
    /// `Test again`.
    fn calibrated(&mut self, result: Result<(u32, u32), Failure>) {
        match result {
            Ok((samples, rate)) => {
                self.latency = Some(samples);
                self.calibration = None;
                self.navigate(Nav::Open(Overlay::Message {
                    tag: CALIBRATE_TAG,
                    lines: vec![calibrate::succeeded(samples, rate)],
                    confirm: None,
                    extra: None,
                }));
            }
            Err(failure) => {
                self.calibration = Some(Calibrating::Asked);
                self.ask_calibration(calibrate::failed(failure), "Test again");
            }
        }
    }
}

//! The rack side panel: [`RackPanel`] in `app.regions.rack`, 20 U at the content area's right, so the keyboard stays
//! playable; the view is laid out left of it. It slides in and out ([`State::slide`]). It draws in the background
//! layer, below the Modules browser's `Order::Middle` backdrop.

use eframe::egui::{self, UiBuilder};

use super::{AppCtx, Nav, Overlay};
use crate::audio_engine::slicer::write_auto_clip;
use crate::flm::chunk::ContentPath;
use crate::flm::sf2d::Sf2d;
use crate::panels::inst::soundfont_player;
use crate::sf2::{Preset, Sf2Error};
use crate::song::ModuleInstance;
use crate::ui::ctrl_menu::Touch;
use crate::ui::rack::{Held, RackPanel, create_send, modulation_names, send_targets, slide};

/// The rack of the channel it was last drawn for; see [`RackPanel`] for what another channel's rack keeps.
#[derive(Default)]
pub struct State {
    panel: RackPanel,
    /// How far the rack has slid in: 0 closed … 1 open.
    open: f32,
    /// The channel the rack last showed: it slides out showing that one.
    last: Option<usize>,
    /// The egui frame the slide last stepped in.
    slid: Option<u64>,
    /// A bound control's touch, with the channel: the app takes it after the frame for its recent control.
    pub(crate) touch: Option<(usize, Touch)>,
    /// A card's `Delete` on a module (channel, instance id) that automation lanes target: the app asks first.
    pub(crate) ask_delete: Option<(usize, i32)>,
    /// `Clear Rack` removing a module automation lanes target, held back: the app asks `Delete automation?` first.
    pub(crate) ask_remove: Option<Held>,
    /// The `INPUT` menu opened: the app lists the input devices for it ([`Self::inputs`]).
    pub(crate) list_inputs: bool,
    /// An `INPUT` menu pick: the channel and its input, for the app to keep.
    pub(crate) route: Option<(usize, crate::recording::routing::Route)>,
    /// The recent control, by channel id, for a Modulator's `Modulate recent control`: the app sets it each frame.
    pub(crate) recent: Option<(i32, crate::song::AutomationTarget)>,
    /// The rack was re-pointed from a gone channel to the one now at its index, whose widgets take the same ids: a
    /// drag of the old rack's control ends before it moves the new one's ([`show`]).
    pub(crate) stop_drag: bool,
}

impl State {
    /// One frame of the slide: in while a rack is `shown`, else out, in an area `width_px` wide, a step per frame of
    /// `ctx` ([`anim::PANEL_KEEP`](crate::ui::anim::PANEL_KEEP)), however many passes the frame runs. Returns how far in the
    /// rack is now, 0 closed … 1 open.
    pub(crate) fn slide(&mut self, shown: bool, width_px: f32, ctx: &egui::Context) -> f32 {
        let frame = Some(ctx.cumulative_frame_nr());
        if self.slid != frame {
            self.slid = frame;
            let step = crate::ui::anim::step(ctx, crate::ui::anim::PANEL_KEEP);
            self.open = slide(self.open, if shown { 1.0 } else { 0.0 }, width_px, step);
        }
        self.open
    }

    /// After channels moved: see [`RackPanel::rekey`].
    pub(crate) fn rekey(&mut self, channel: &dyn Fn(usize) -> Option<usize>) {
        self.panel.rekey(channel);
        self.last = self.last.and_then(channel);
    }

    /// See [`RackPanel::end_learn`].
    pub(crate) fn end_learn(&mut self) -> Option<crate::ui::rack::SettingChange> {
        self.panel.end_learn()
    }

    /// A live key played on channel `channel`: see [`RackPanel::key_played`] (`Set Range`, `Set First Note`).
    pub(crate) fn key_played(&mut self, channel: usize, key: u8) {
        self.panel.key_played(channel, key);
    }

    /// A clone's rack: see [`RackPanel::copy_panels`].
    pub(crate) fn copy_panels(&mut self, from: usize, to: usize) {
        self.panel.copy_panels(from, to);
    }

    /// The module meters the panels got in the last frame: `None` without a running stream.
    #[cfg(test)]
    pub(crate) fn meters(&self) -> Option<&[honeydew_audio::device::ModuleMeter]> {
        self.panel.meters.as_deref()
    }

    /// The rack panel.
    #[cfg(test)]
    pub(crate) fn panel(&self) -> &RackPanel {
        &self.panel
    }

    /// What the rack read of the SoundFont `path` (an `SF2D` path) names; `None` before its first read.
    pub(crate) fn soundfont(&self, path: &ContentPath) -> Option<&Result<Vec<Preset>, Sf2Error>> {
        self.panel.soundfonts.get(path)
    }

    /// The SoundFont `path` (an `SF2D` path) was just read as `read` (the SoundFont picker's pick): its panels show
    /// that, not an older read.
    pub(crate) fn soundfont_read(
        &mut self,
        path: ContentPath,
        read: Result<Vec<Preset>, Sf2Error>,
    ) {
        self.panel.soundfonts.insert(path, read);
    }

    /// The input devices for the open `INPUT` menu, and the channel's input among them.
    pub(crate) fn inputs(
        &mut self,
        devices: Vec<crate::recording::input::InputDevice>,
        current: Option<crate::recording::input::InputChoice>,
    ) {
        self.panel.inputs = Some((devices, current));
    }
}

// The input row's imports, kept next to its code.
use crate::audio_engine::LiveInput;
use crate::ui::rack::METER_RINGS;

/// The 25 ms peaks of the open input's ports that channel `id` takes, newest first: its stored port, or the louder of
/// its pair (`Input 1` without a stored input, as the monitor takes it).
fn input_peaks(input: &LiveInput, id: i32) -> [f32; METER_RINGS] {
    let ports = input.ports.iter().find(|p| p.0 == id);
    let (first, second) = ports.map_or((0, None), |&(_, first, second)| (first, second));
    let peak = |k, port| input.ring.peak(k, port);
    std::array::from_fn(|k| second.map_or(0.0, |s| peak(k, s)).max(peak(k, first)))
}

/// Draws `channel`'s rack, which no undo step covers. Param edits (the panels' and the level popup's or settings
/// dialog's common params) are already in the song and go to the audio as `SetParam`, a channel param edit (the
/// song-changed flag), as is a card minimized or maximized and a module setting (its `CSTM`; the audio gets each
/// setting edit as `SetSetting`); a changed chain, a finished note-effect edit or a finished edit of a setting the
/// engine reads at build is a final song edit, which rebuilds the engine. `Add Module` and `Insert above/below` /
/// `Replace` open the Modules browser; a card's `Copy` puts its module on `clipboard`, the rack's `Copy Rack` all of
/// them, in place of what it held; `Save preset` and `Save Rack preset` open the preset dialog (after the frame,
/// through [`AppCtx::files`]); a card's preset button and `Load Rack preset` open the preset picker, a SoundFont
/// Player's `Patch` spinner the SoundFont picker (its `Prev` / `Next` are final song edits; the SoundFonts the players
/// name are read first), and `Custom` picked in a 3x Osc Shape menu or a Slicer's `LOAD SAMPLE` or waveform the
/// module's sample picker (the Slicers' loops are handed to their waveforms first); the close X, the collapse tab, the
/// left margin, or the channel being gone, closes the rack. A Slicer's AUTO writes into `held` (the channel's piano
/// roll clip) first ([`write_auto_clip`]). A bound control's touch and a `Delete` the app must ask about first go to
/// [`State::touch`] and [`State::ask_delete`], a held `Clear Rack` to [`State::ask_remove`].
pub fn show(
    state: &mut State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    channel: usize,
    clipboard: &mut Vec<ModuleInstance>,
    held: Option<(usize, u32)>,
) -> Option<Nav> {
    state.panel.targets = send_targets(app.song, channel);
    (state.panel.recent, state.panel.modulations) =
        (state.recent, modulation_names(app.song, channel));
    let Some(ch) = app.song.channels.get_mut(channel) else {
        return Some(Nav::Close);
    };
    state.last = Some(channel);
    if std::mem::take(&mut state.stop_drag) {
        ui.ctx().stop_dragging();
    }
    let rect = app.regions.rack;
    // A fixed id scope per channel: the rack's widget ids do not depend on what the frame drew before it, and the
    // temp data egui keys by them (drags, menu positions) is not shared with another channel's same instance id.
    let mut ui = ui.new_child(UiBuilder::new().id(("rack", channel)).max_rect(rect));
    state.panel.covered = app.covered;
    state.panel.middle_c_is_c4 = app.middle_c_is_c4;
    // The input row's (an Audio Record channel's): the arm button with multitrack on, the meter while the input is open.
    state.panel.multitrack = app.settings.multitrack_recording;
    let records = ch.kind == crate::song::ChannelKind::AudioRecord;
    let input = app.audio.input.as_ref().filter(|_| records);
    state.panel.input_peaks = input.map(|input| input_peaks(input, ch.id));
    // A failed stream's meters are stale: as without a stream, the panels get none.
    match (
        app.snapshot.filter(|s| !s.stream_failed),
        &mut state.panel.meters,
    ) {
        (Some(s), Some(meters)) => meters.clone_from(&s.module_meters),
        (live, meters) => *meters = live.map(|s| s.module_meters.clone()),
    }
    // The SoundFonts the channel's players name, each read the first time.
    // ponytail: never read again, so a file changed on disk shows once the app restarts or the picker picks it.
    let players = ch.rack.modules.iter();
    let players = players.filter(|m| m.type_id == soundfont_player::TYPE_ID);
    for sf2d in players.filter_map(|m| m.file.custom_data.as_deref().and_then(Sf2d::read)) {
        let files = &*app.files;
        let entry = state.panel.soundfonts.entry(sf2d.path);
        entry.or_insert_with_key(|path| files.soundfont(path));
    }
    for module in &ch.rack.modules {
        crate::panels::post_loop(ui.ctx(), module, &app.audio.samples);
    }
    let rate = app.audio.sample_rate().map_or(0.0, |r| r as f32);
    crate::panels::set_text_env(ui.ctx(), app.song.tempo_bpm, rate);
    let actions = state.panel.show(&mut ui, &app.cx, rect, channel, ch);
    state.touch = actions.touch.map(|t| (channel, t)).or(state.touch);
    state.ask_delete = actions
        .ask_delete
        .map(|id| (channel, id))
        .or(state.ask_delete);
    state.list_inputs |= actions.list_inputs;
    if let Some(route) = actions.input {
        state.route = Some((channel, route));
    }
    if let Some(after) = actions.ask_remove {
        state.ask_remove = Some(Held {
            channel,
            after,
            skipped: 0,
        });
    }
    let channel_id = ch.id;
    if !actions.params.is_empty() || actions.minimized || actions.settings {
        app.channel_param_edited();
    }
    for change in actions.params {
        // A DW Sampler's kept Attack or Release init value follows its control.
        if let Some(module) = app.song.channels[channel].rack.get_mut(change.instance_id) {
            module.init_follows(change.index);
        }
        app.audio.send(change.command());
    }
    for change in actions.setting_changes {
        // A GMSynth's new OSC shape is built here, on the app thread, before the audio thread takes it.
        if let Some(module) = app.song.channels[channel].rack.get(change.instance_id) {
            crate::audio_engine::settings::prepare(module.type_id, change.key, change.value);
        }
        app.audio.send(change.command());
    }
    // A Slicer's `Set First Note` stores the key slice 0 plays, which its slice count may clamp.
    if actions.replace_song {
        crate::audio_engine::settle_first_notes(
            &mut app.song.channels[channel],
            &app.audio.samples,
        );
    }
    // AUTO's clip, in this edit's undo step: a Sensitivity re-detect while the AUTO flag is on; the `AUTO` button
    // whatever the flag says.
    let auto = (actions.redetect.iter().map(|&id| (id, false)))
        .chain(actions.auto_clip.map(|id| (id, true)));
    let mut rewrote = false;
    for (id, always) in auto {
        let read = crate::sample_cache::reader(app.files);
        let samples = &mut app.audio.samples;
        rewrote |= write_auto_clip(app.song, channel, id, samples, &read, always, held);
    }
    // `Create FX Channel`: the new send channel goes last, so no channel index moves.
    let created = actions
        .create_send
        .and_then(|at| create_send(app.song, channel, at));
    if actions.replace_song || created.is_some() || rewrote {
        app.song_edited(true);
    }
    if let Some(copy) = actions.copy {
        *clipboard = copy;
    }
    if let Some(preset) = actions.preset {
        app.files.ask_preset(preset, channel_id);
    }
    if let Some(target) = actions.picker {
        return Some(Nav::Open(Overlay::PresetPicker { channel, target }));
    }
    if let Some(module) = actions.soundfont {
        return Some(Nav::Open(Overlay::SoundFontPicker { channel, module }));
    }
    if let Some(module) = actions.module_sample {
        return Some(Nav::Open(Overlay::ModuleSamplePicker { channel, module }));
    }
    if let Some(at) = actions.open_browser {
        return Some(Nav::Open(Overlay::ModulesBrowser { channel, at }));
    }
    actions.close.then_some(Nav::Close)
}

/// The rack sliding out after it closed: the channel it last showed, drawn as it was and taking no input.
pub fn show_closing(
    state: &mut State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    clipboard: &mut Vec<ModuleInstance>,
) {
    let Some(channel) = state
        .last
        .filter(|_| app.regions.rack.left() < app.regions.area.right())
    else {
        return;
    };
    let (mut ui, opacity) = (ui.new_child(UiBuilder::new()), ui.opacity());
    ui.disable();
    // Without the disabled look.
    ui.set_opacity(opacity);
    show(state, &mut ui, app, channel, clipboard, None);
}

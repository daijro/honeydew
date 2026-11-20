//! Contextual UI-only help. Keep the copy alongside the feature contracts and regression tests.
use eframe::egui::{self, Align2, Color32, FontId, Id, Rect, Sense, Stroke, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    Playlist,
    PianoRoll,
    Mixer,
    AudioEditor,
    WaveformQuality,
    SoundMode,
    Envelopes,
    Colors,
    Follow,
    KeyNames,
}
impl Topic {
    pub fn title(self) -> &'static str {
        match self {
            Self::Playlist => "Playlist controls",
            Self::PianoRoll => "Piano Roll tools",
            Self::Mixer => "Mixer controls",
            Self::AudioEditor => "Audio Editor",
            Self::WaveformQuality => "Waveform Quality",
            Self::SoundMode => "Android sound mode",
            Self::Envelopes => "Playlist envelopes",
            Self::Colors => "Envelope colors",
            Self::Follow => "Continuous scrolling",
            Self::KeyNames => "Key names",
        }
    }
    pub fn text(self) -> &'static str {
        match self {
            Self::Playlist => {
                "Clips are arranged in song time. Select a clip before editing it. Double-tap a selected audio clip's envelope area to arm point creation; drag a point, or hold/right-click it for curve shapes. Volume and pan belong to each clip, including overlaps.\n\nEnvelope lines show only controls differing from their base value. Settings → Edit can hide the lines without disabling audio automation. When PAN controls exist, the waveform shows predicted left output above right, including balance attenuation. It is before track/master effects, not an output meter. Use stereo headphones to judge panning; a mono device cannot reproduce left/right position."
            }
            Self::PianoRoll => {
                "The grid edits notes in the selected clip. Snap sets the time grid used by note edits; turning snap off allows free timing. Selecting notes lets editing tools act on that selection. The graph lane edits note properties such as velocity and pan, not the track fader.\n\nVocal pitch drawing is available only in the Vocal Synth workspace; it is not an audio pitch detector for ordinary clips. Undo reverses completed note edits."
            }
            Self::Mixer => {
                "Level changes the entire channel's output. Pan balances its left and right output; it is separate from each audio clip's PAN envelope. Mute silences that channel. Solo restricts the audible channel set; routing and other solos still matter.\n\nMeters show measured channel signal, not predicted waveform peaks. Stereo effects, sends, and master processing can change the final left/right levels. Mixer controls affect playback, not just the display."
            }
            Self::AudioEditor => {
                "VOL controls sample gain; PAN controls stereo balance; SPEED changes playback rate. Envelope points are shared with the Playlist. PAN 0 is left, halfway is center, and full scale is right. At center both source channels retain their level; hard panning attenuates the opposite channel rather than moving its content across.\n\nEdits audition during playback. A sharp gain/pan jump can click by design. The waveform shows left above right when PAN points exist, with VOL and PAN applied before channel effects. Source samples are not rewritten by envelope edits. Stretch/region operations are different: they can alter timing or create edited sample content."
            }
            Self::WaveformQuality => {
                "High keeps the fullest current waveform detail. Balanced groups adjacent peaks; Performance draws sparse bars. All modes retain conservative peak ranges at overview zoom. This changes drawing geometry only, not sample quality, playback, recording, export, or clip timing. Phone performance depends on project size and device; no fixed FPS is guaranteed."
            }
            Self::SoundMode => {
                "Native requests Oboe's recommended low-latency backend. Fast uses OpenSL ES with a 256-frame callback target. Safe and Ultra Safe request 1024/2048 frames without the low-latency performance request. The device may choose different actual sizes.\n\nChanging mode restarts real-time output and can briefly interrupt sound/reset active voices or effect tails. It does not change offline export quality. Smaller targets can reduce latency but may underrun on a busy device. These profiles apply to Android."
            }
            Self::Envelopes => {
                "ON shows meaningful, edited Playlist VOL/PAN curves. OFF hides curves and editing outlines and prevents starting invisible Playlist point edits. It does not erase points, disable volume/pan processing, change the waveform's predicted audio, or prevent editing in the Audio Editor. Untouched clips do not draw curves. Defaults: volume yellow, pan cyan."
            }
            Self::Colors => {
                "Choose volume and pan display colors independently. Swatches include white, with no black preset. HEX accepts a six-digit #RRGGBB value, including colors not in the swatches. The preview updates as you choose. Reset restores yellow/cyan. These preferences survive restart; they never edit points, shapes, or sound."
            }
            Self::Follow => {
                "ON lets the Playlist follow playback continuously. Drag the yellow ruler handle to set the playhead's preferred position (5–95% across the timeline). OFF restores ordinary scrolling and hides the handle. The position is saved separately. This changes the viewport, not playback timing, clips, or automation; project edges still limit scrolling."
            }
            Self::KeyNames => {
                "ON shows note-name labels on keyboard and Piano Roll keys/notes. OFF removes those text labels, including octave/root names. Colored root markers remain. This is a display preference only: MIDI pitches and the sound do not change."
            }
        }
    }
}
pub fn setting(label: &str) -> Option<Topic> {
    match label {
        "Waveform Quality:" => Some(Topic::WaveformQuality),
        "Sound mode:" => Some(Topic::SoundMode),
        "Show Playlist envelopes" => Some(Topic::Envelopes),
        "VOL / PAN colors" => Some(Topic::Colors),
        "Continuous scrolling" | "Continuous scrolling during playback" => Some(Topic::Follow),
        "Show key names on keyboard and piano roll" => Some(Topic::KeyNames),
        _ => None,
    }
}
/// 30-point touch target around a smaller circular icon. Separate foreground area prevents parent drag handlers taking it.
pub fn button(ctx: &egui::Context, at: egui::Pos2, topic: Topic) {
    let key = Id::new("contextual-help-open");
    let response = egui::Area::new(Id::new(("help-icon", topic.title())))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .show(ctx, |ui| {
            let (r, response) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
            let ink = Color32::from_rgb(185, 205, 220);
            ui.painter().circle(
                r.center(),
                8.0,
                Color32::from_rgb(35, 43, 54),
                Stroke::new(1.0, ink),
            );
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                "i",
                FontId::proportional(13.0),
                ink,
            );
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("About {}", topic.title())));
            response.on_hover_text(format!("About {}", topic.title()))
        })
        .inner;
    if response.clicked() {
        ctx.data_mut(|d| d.insert_temp(key, topic));
    }
    let active = ctx.data(|d| d.get_temp::<Topic>(key));
    if active != Some(topic) {
        return;
    }
    let mut open = !ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let size = ctx.content_rect().size();
    let shown = egui::Window::new(topic.title())
        .id(key.with("panel"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_pos(pos2((at.x - 270.0).max(4.0), at.y + 32.0))
        .default_width(280.0_f32.min(size.x - 24.0))
        .max_width(300.0_f32.min(size.x - 24.0))
        .max_height((size.y - 80.0).max(100.0))
        .vscroll(true)
        .show(ctx, |ui| {
            ui.label(topic.text());
            ui.small("Tap outside, press Escape, or use × to close.");
        });
    if let Some(panel) = shown {
        if ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| !panel.response.rect.contains(p) && !response.rect.contains(p))
        }) {
            open = false;
        }
    }
    if !open {
        ctx.data_mut(|d| d.remove::<Topic>(key));
    }
}
pub fn corner(ctx: &egui::Context, area: Rect, topic: Topic) -> bool {
    let key = Id::new("contextual-help-open");
    let previous = ctx.data(|d| d.get_temp::<Topic>(key));
    // Navigation dismisses the previous workspace's help instead of leaving an invisible modal.
    if previous.is_some_and(|p| p != topic) {
        ctx.data_mut(|d| d.remove::<Topic>(key));
    }
    let was_open = previous == Some(topic);
    let at = pos2(area.right() - 32.0, area.top() + 2.0);
    button(ctx, at, topic);
    was_open
        || ctx.data(|d| d.get_temp::<Topic>(key)).is_some()
        || ctx.input(|i| {
            i.pointer
                .interact_pos()
                .is_some_and(|p| Rect::from_min_size(at, vec2(30.0, 30.0)).contains(p))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contextual_help_opens_on_tap_and_escape_dismisses() {
        let mut off = crate::ui::offscreen::Offscreen::new();
        let mut draw = |ui: &mut egui::Ui, _: &crate::ui::UiCtx| {
            button(ui.ctx(), pos2(10.0, 10.0), Topic::Envelopes)
        };
        off.frame(vec![], &mut draw);
        off.click(pos2(25.0, 25.0), &mut draw);
        assert_eq!(
            off.ctx
                .data(|d| d.get_temp::<Topic>(Id::new("contextual-help-open"))),
            Some(Topic::Envelopes)
        );
        off.frame(
            crate::ui::offscreen::key_tap(egui::Key::Escape, Default::default()),
            &mut draw,
        );
        assert_eq!(
            off.ctx
                .data(|d| d.get_temp::<Topic>(Id::new("contextual-help-open"))),
            None
        );
    }
    #[test]
    fn contextual_help_copy_describes_stereo_limit_and_visual_only_switch() {
        assert!(Topic::AudioEditor.text().contains("attenuates"));
        assert!(Topic::Envelopes.text().contains("does not erase"));
        assert!(setting("Waveform Quality:").is_some());
    }
}

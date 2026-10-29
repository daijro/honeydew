//! The playlist screen's chrome: the keyboard panel's close X and the show-keyboard button, and the add-channel `+`.

use eframe::egui::{Color32, Shape, epaint::CircleShape};

use super::*;

/// In the `↖`'s touch area on the bottom-right corner.
const SHOW_KEYS: Pos2 = pos2(1170.0 - 0.5 * U, 540.0 - 0.5 * U);
/// The `↖` disc: 1.5 U, 0.6 U in from the right edge and 0.2 U up from the bottom.
const SHOW_DISC: egui::Rect = egui::Rect::from_min_max(
    pos2(1170.0 - 2.1 * U, 540.0 - 1.7 * U),
    pos2(1170.0 - 0.6 * U, 540.0 - 0.2 * U),
);
/// The bottom-right corner.
const CORNER: Pos2 = pos2(1170.0, 540.0);

impl Rig {
    /// The last frame's filled circles centred on `center`, with their index in the shapes.
    fn discs_at(&self, center: Pos2) -> Vec<(usize, CircleShape)> {
        (self.out.shapes.iter().enumerate())
            .filter_map(|(i, s)| match &s.shape {
                Shape::Circle(c) if c.center == center && c.fill != Color32::TRANSPARENT => {
                    Some((i, *c))
                }
                _ => None,
            })
            .collect()
    }

    fn hidden(&self) -> bool {
        self.app.main_screen.panel_hidden
    }

    /// The `↖`'s `moduleFill` disc in the last frame's shapes: its index.
    fn show_button(&self) -> Option<usize> {
        let fill = self.app.theme.colors.module_fill.to_color32();
        (self.out.shapes.iter()).position(
            |s| matches!(&s.shape, Shape::Rect(r) if r.rect == SHOW_DISC && r.fill == fill),
        )
    }

    /// Lets an opened clip editor's split settle (`ui::split`): f halves toward 0 each frame and snaps on the ninth in
    /// the rig's 380-point view.
    pub(super) fn slide_split(&mut self) {
        for _ in 0..9 {
            self.frame(vec![]);
        }
    }
}

#[test]
fn the_close_x_on_the_playlist_hides_the_panel_and_the_corner_button_brings_it_back() {
    let mut rig = Rig::with_synth();
    rig.frame(vec![]);
    assert!(rig.painted("BPM"));
    assert!(rig.show_button().is_none());
    rig.click(CLOSE);
    assert!(rig.hidden());
    rig.frame(vec![]);
    // No transport, no keyboard; the track rows reach the bottom, and the rack tab is centred on them again.
    assert!(!rig.painted("BPM"));
    rig.take();
    rig.click(E4);
    assert_eq!(rig.take(), Vec::<String>::new());
    let tab = rig.ctx.read_response(egui::Id::new("edge_tab")).unwrap();
    assert_eq!(tab.rect.center().y, (540.0 + 2.0 * U) / 2.0);
    // The `↖`: a floating 1.5 U `moduleFill` disc with icon 11 in `text` over it.
    let disc = rig.show_button().expect("the ↖");
    let icon = (rig.out.shapes[disc + 1..].iter()).any(|s| match &s.shape {
        Shape::Circle(c) => c.fill == ink(),
        Shape::Path(p) => {
            p.fill == ink() || p.stroke.color == egui::epaint::ColorMode::Solid(ink())
        }
        Shape::LineSegment { stroke, .. } => stroke.color == ink(),
        _ => false,
    });
    assert!(icon, "no icon in text over the disc");
    rig.click(SHOW_KEYS);
    assert!(!rig.hidden());
    rig.frame(vec![]);
    assert!(rig.painted("BPM"));
    assert!(rig.show_button().is_none());
    rig.click(E4);
    assert_eq!(rig.take(), ["NoteOn 1 52 108", "NoteOff 1 52"]);
}

/// The transport's close X hides the keyboard panel in every view; the clip editors keep their view, and the `↖`
/// brings the panel back.
#[test]
fn in_the_editors_and_the_mixer_the_close_x_hides_the_panel_and_keeps_the_view() {
    let mut rig = Rig::with_drums_and_synth();
    for (channel, view) in [
        (2, View::PianoRoll(2)),
        (1, View::DrumEditor(1)),
        (0, View::Mixer),
    ] {
        match view {
            View::Mixer => rig.app.navigate(Nav::Show(view)),
            _ => rig.edit(channel),
        }
        rig.slide_split();
        rig.click(CLOSE);
        assert_eq!(rig.app.nav.view, view);
        assert!(rig.hidden(), "{view:?}");
        rig.frame(vec![]);
        assert!(!rig.painted("BPM"), "{view:?}");
        rig.click(SHOW_KEYS);
        assert!(!rig.hidden(), "{view:?}");
        assert_eq!(rig.app.nav.view, view);
        rig.frame(vec![]);
        assert!(rig.painted("BPM"), "{view:?}");
    }
}

#[test]
fn the_corner_button_lies_over_the_open_rack_and_takes_its_taps() {
    let mut rig = Rig::with_synth();
    rig.click(CLOSE);
    rig.app.navigate(Nav::Open(Overlay::Rack(1)));
    rig.slide_rack();
    rig.frame(vec![]);
    // The rack is 20 U at the right of the content area, which now reaches the bottom.
    let rack = egui::Rect::from_min_max(pos2(530.0, 0.0), CORNER);
    let rack_fill = (rig.out.shapes.iter())
        .rposition(|s| matches!(&s.shape, Shape::Rect(r) if r.rect == rack))
        .expect("the rack's fill");
    let disc = rig.show_button().expect("the ↖");
    assert!(disc > rack_fill, "{disc} <= {rack_fill}");
    rig.click(SHOW_KEYS);
    assert!(!rig.hidden());
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
}

#[test]
fn the_drum_editor_runs_with_the_panel_hidden() {
    let mut rig = Rig::with_drums_and_synth();
    rig.click(CLOSE);
    rig.edit(1);
    rig.slide_split();
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::DrumEditor(1));
    assert!(rig.show_button().is_some());
    rig.click(SHOW_KEYS);
    rig.frame(vec![]);
    assert!(!rig.hidden());
    assert_eq!(rig.app.nav.view, View::DrumEditor(1));
}

#[test]
fn the_add_channel_plus_sits_under_the_last_row_on_the_screen_centre_and_the_rack_covers_it() {
    let mut rig = Rig::with_drums_and_synth();
    let plus = egui::Id::new("add_channel_button");
    // Three 2 U rows under the ruler: 0.75 U below the last, 2 U across, centred on the screen.
    rig.frame(vec![]);
    let rect = rig.ctx.read_response(plus).unwrap().rect;
    let expected = egui::Rect::from_min_size(
        pos2(585.0 - U, RULER + 6.0 * U + 0.75 * U),
        vec2(2.0 * U, 2.0 * U),
    );
    assert_eq!(rect, expected);
    let disc = rig.discs_at(expected.center());
    assert_eq!(disc.len(), 1);
    // The open rack narrows the playlist but does not move the `+`; drawn after it, the rack covers it, whole here: its
    // left edge (530) is left of the screen centre − 1 U (553).
    rig.app.navigate(Nav::Open(Overlay::Rack(1)));
    rig.slide_rack();
    rig.frame(vec![]);
    assert_eq!(rig.ctx.read_response(plus).unwrap().rect, expected);
    let rack = egui::Rect::from_min_max(pos2(530.0, 0.0), pos2(1170.0, PANEL_TOP));
    let rack_fill = (rig.out.shapes.iter())
        .position(|s| matches!(&s.shape, Shape::Rect(r) if r.rect == rack))
        .expect("the rack's fill");
    let disc = rig.discs_at(expected.center());
    assert!(disc[0].0 < rack_fill);
}

/// The rack tab hides while the rack is open; the logo lies over the rack and takes its tap, and the main menu it
/// opens covers it.
#[test]
fn the_logo_lies_over_the_open_rack_and_takes_its_tap_under_the_main_menu() {
    let mut rig = Rig::with_synth();
    rig.app.navigate(Nav::Open(Overlay::Rack(1)));
    rig.slide_rack();
    rig.frame(vec![]);
    assert!(rig.ctx.read_response(egui::Id::new("edge_tab")).is_none());
    let rack = egui::Rect::from_min_max(pos2(530.0, 0.0), pos2(1170.0, PANEL_TOP));
    let rack_fill = (rig.out.shapes.iter())
        .rposition(|s| matches!(&s.shape, Shape::Rect(r) if r.rect == rack))
        .expect("the rack's fill");
    let logo = |rig: &Rig| {
        let at = rig
            .ctx
            .read_response(egui::Id::new("app_logo"))
            .unwrap()
            .rect
            .expand(4.0);
        (rig.out.shapes.iter())
            .position(|s| matches!(&s.shape, Shape::Mesh(m) if at.contains_rect(m.calc_bounds())))
            .expect("the logo's image")
    };
    assert!(rack_fill < logo(&rig));
    rig.click(LOGO);
    assert_eq!(
        rig.overlays(),
        [Overlay::Rack(1), Overlay::MainMenu(MenuTab::Songs)]
    );
    rig.frame(vec![]);
    let menu_fill = rig.app.theme.colors.menu_fill.to_color32();
    let corner = pos2(1169.0, 1.0);
    let menu = (rig.out.shapes.iter()).rposition(
        |s| matches!(&s.shape, Shape::Rect(r) if r.fill == menu_fill && r.rect.contains(corner)),
    );
    assert!(menu > Some(logo(&rig)), "{menu:?}");
}

impl Rig {
    /// The tint of the last frame's first image (a mesh off the font atlas) inside `rect`, if its vertices share one.
    fn image_tint_in(&self, rect: egui::Rect) -> Option<Color32> {
        self.out.shapes.iter().find_map(|s| match &s.shape {
            Shape::Mesh(m)
                if m.texture_id != egui::TextureId::default()
                    && rect.contains_rect(m.calc_bounds()) =>
            {
                let tint = m.vertices[0].color;
                m.vertices.iter().all(|v| v.color == tint).then_some(tint)
            }
            _ => None,
        })
    }
}

/// A clip editor opens under the playlist, sliding up fully expanded, the playlist going to 0. The close ribbon drags
/// the split down: a 4 px gap under the playlist, the t colour band, the editor the rest, and the playlist above is
/// live.
#[test]
fn a_clip_editor_opens_under_the_playlist_and_the_ribbon_moves_it() {
    let mut rig = Rig::with_synth();
    rig.app.song.channels[1].lanes[0].add_clip(512, 8.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.edit(1);
    rig.frame(vec![]);
    // The first frame: the playlist has half the area (to the pixel), the roll its settled size, moved down to the gap.
    assert_eq!(
        roll_grid(&rig).top(),
        (PANEL_TOP / 2.0).round() + 4.0 + ROLL_TOP
    );
    assert_eq!(roll_grid(&rig).height(), PANEL_TOP - ROLL_TOP);
    rig.slide_split();
    assert_eq!(
        roll_grid(&rig).y_range(),
        egui::Rangef::new(ROLL_TOP, PANEL_TOP)
    );
    // The ribbon dragged down moves the split with the finger.
    drag_split(&mut rig, 120.0);
    assert_eq!(roll_grid(&rig).top(), 120.0 + 4.0 + ROLL_TOP);
    // The playlist above is live: its ruler (a beat 38.4 points from x 48) seeks.
    rig.take();
    rig.click(pos2(48.0 + 3.1 * 38.4, 24.0));
    assert_eq!(rig.take(), ["Seek 384"]);
}

/// The piano roll's grid as last laid out.
fn roll_grid(rig: &Rig) -> egui::Rect {
    let id = egui::Id::new("piano_roll").with("grid");
    rig.ctx.read_response(id).expect("the roll's grid").rect
}

/// Some of the playlist's widgets were drawn last frame.
fn playlist_drawn(rig: &Rig) -> bool {
    [
        "playlist_snap",
        "playhead_knob",
        "track_scroll",
        "add_channel_button",
    ]
    .map(|id| rig.drawn(egui::Id::new(id)))
    .contains(&true)
}

/// The roll's grid top with the editor at the area's top: the t colour band (2 points at U = 32), then the ruler.
const ROLL_TOP: f32 = 2.0 + 52.0;

/// The close ribbon dragged `dy` points down (up when negative) in 4-point steps, so no flick, and two frames after it.
fn drag_split(rig: &mut Rig, dy: f32) {
    let at = rig
        .ctx
        .read_response(egui::Id::new("editor_close"))
        .expect("the ribbon")
        .rect
        .center();
    rig.press(at);
    let steps = (dy.abs() / 4.0).ceil() as usize;
    for i in 1..=steps {
        rig.drag_to(at + vec2(0.0, dy * i as f32 / steps as f32));
    }
    rig.release(at + vec2(0.0, dy));
    // The release sets the split after this frame's layout; the next lays out by it, and a read comes a frame late.
    rig.frame(vec![]);
    rig.frame(vec![]);
}

/// A Tab press and its release.
fn tab(rig: &mut Rig) {
    let release = egui::Event::Key {
        key: egui::Key::Tab,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    rig.frame(vec![
        key_event(egui::Key::Tab, egui::Modifiers::NONE),
        release,
    ]);
}

/// The synth's piano roll open, the ribbon dragged down to 100 points of playlist (the gap at 100 … 104).
fn roll_rig() -> Rig {
    let mut rig = Rig::with_synth();
    rig.app.song.channels[1].lanes[0].add_clip(512, 8.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.edit(1);
    rig.slide_split();
    drag_split(&mut rig, 100.0);
    rig
}

/// Editors open fully expanded. The piano roll and the drum editor open with the playlist 0 high and not drawn, the
/// editor from the area's top (the band at the window top, the ribbon under it); the ribbon still drags it down, a
/// switch to another editor keeps that split, and the next opening from the playlist is fully expanded again.
#[test]
fn the_piano_roll_and_the_drum_editor_open_fully_expanded_and_a_switch_keeps_the_split() {
    let mut rig = Rig::with_drums_and_synth();
    for channel in [1, 2] {
        rig.app.song.channels[channel].lanes[0].add_clip(0, 4.0);
    }
    rig.app.song_edited(SongEdit::Final, 0);
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let area = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1170.0, PANEL_TOP));
    let x = egui::Id::new("editor_close");
    let expanded = |rig: &Rig| {
        let (playlist, editor, _) = rig.app.main_screen.split.layout(area, &m);
        assert_eq!((playlist.height(), editor.top()), (0.0, area.top() + 2.0));
        assert!(!playlist_drawn(rig));
        assert_eq!(rig.ctx.read_response(x).map(|r| r.rect.top()), Some(2.0));
    };
    // The synth's piano roll: its ruler under the band at the area top.
    rig.edit(2);
    rig.slide_split();
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::PianoRoll(2));
    expanded(&rig);
    assert_eq!(roll_grid(&rig).top(), ROLL_TOP);
    // The ribbon dragged down: the playlist shows its first rows.
    drag_split(&mut rig, 240.0);
    assert_eq!(roll_grid(&rig).top(), 240.0 + 4.0 + ROLL_TOP);
    assert!(playlist_drawn(&rig));
    let split = rig.app.main_screen.split;
    // A tap on the drums' clip (row 1, beat 2) switches to the drum editor: the split stays.
    rig.click(pos2(48.0 + 2.0 * 38.4, header(1).y));
    rig.slide_split();
    assert_eq!(rig.app.nav.view, View::DrumEditor(1));
    assert_eq!(rig.app.main_screen.split, split);
    // Closed and opened again: fully expanded.
    rig.click_id(x);
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::Playlist);
    rig.edit(1);
    rig.slide_split();
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::DrumEditor(1));
    expanded(&rig);
}

/// The ribbon drags a clip editor all the way up. Let go under the playlist's 2.5 U floor, the editor fills the area
/// from its top with no gap, the playlist is 0 high and not drawn (no ruler controls over the editor, nothing to
/// focus), the ribbon's tap still shows the playlist alone, and a slow drag down brings the playlist back.
#[test]
fn the_ribbon_drags_the_editor_up_to_the_area_top_and_back_down() {
    let mut rig = roll_rig();
    assert!(playlist_drawn(&rig));
    drag_split(&mut rig, -60.0);
    assert_eq!(rig.app.main_screen.split.f, 0.0);
    // The roll's ruler starts under the band at the area top.
    assert_eq!(roll_grid(&rig).top(), ROLL_TOP);
    assert!(!playlist_drawn(&rig));
    for _ in 0..40 {
        tab(&mut rig);
        assert!(!focus::within(&rig.ctx, egui::Id::new(focus::PLAYLIST)));
    }
    // The ribbon hangs from the band at the window top, and its tap still jumps the split to 1.
    let x = egui::Id::new("editor_close");
    assert_eq!(rig.ctx.read_response(x).map(|r| r.rect.top()), Some(2.0));
    rig.click_id(x);
    rig.frame(vec![]);
    assert_eq!(
        (rig.app.nav.view, rig.app.main_screen.split.f),
        (View::Playlist, 1.0)
    );
    assert!(playlist_drawn(&rig));
    // Reopened, the editor opens fully expanded.
    rig.edit(1);
    rig.slide_split();
    // The ninth frame's snap to 0 is the last move (read back a frame late).
    rig.frame(vec![]);
    assert_eq!(roll_grid(&rig).top(), ROLL_TOP);
    // 4 points a frame: the drag counts from the press, so the playlist, 0 while under its ruler, comes back.
    drag_split(&mut rig, 100.0);
    assert_eq!(roll_grid(&rig).top(), 100.0 + 4.0 + ROLL_TOP);
    assert!(playlist_drawn(&rig));
}

/// A playlist dragged under its floor goes to 0, so nothing of it acts over the editor: a pinch on the editor's ruler
/// zooms the editor only, and Tab never reaches the playlist's hidden `+`.
#[test]
fn a_playlist_dragged_under_its_ruler_is_none_and_takes_no_pinch_or_focus() {
    let view = |rig: &Rig| {
        let s = &rig.app.main_screen;
        (s.zoom, s.scroll_x, s.scroll_y)
    };
    let mut rig = roll_rig();
    drag_split(&mut rig, -80.0);
    assert_eq!(roll_grid(&rig).top(), ROLL_TOP);
    assert!(!playlist_drawn(&rig));
    // Both fingers on the editor's ruler, where the playlist's ruler would reach; the second spreads them.
    use egui::TouchPhase::{End, Move, Start};
    let touch = |id, phase, x| egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(id),
        phase,
        pos: pos2(x, 40.0),
        force: None,
    };
    let before = view(&rig);
    rig.frame(vec![egui::Event::PointerMoved(pos2(400.0, 40.0))]);
    rig.frame(vec![
        touch(1, Start, 400.0),
        button_event(pos2(400.0, 40.0), true),
    ]);
    rig.frame(vec![touch(2, Start, 600.0)]);
    rig.frame(vec![touch(2, Move, 800.0)]);
    rig.frame(vec![
        touch(2, End, 800.0),
        touch(1, End, 400.0),
        button_event(pos2(400.0, 40.0), false),
        egui::Event::PointerGone,
    ]);
    rig.frame(vec![]);
    assert_eq!(view(&rig), before);

    let mut rig = roll_rig();
    drag_split(&mut rig, -96.0);
    assert_eq!(roll_grid(&rig).top(), ROLL_TOP);
    tab(&mut rig);
    assert!(!focus::within(&rig.ctx, egui::Id::new(focus::PLAYLIST)));
    assert_ne!(
        focus::focused(&rig.ctx),
        Some(egui::Id::new("add_channel_button"))
    );
}

/// One close ribbon serves the four bottom editors: 1.5 U × 1.3 U in the channel colour, hanging from the t colour band
/// at the editor's top, its left edge at min(screen right − 4.4 U, editor right − 1.8 U), its touch area 0.3 U wider
/// left, right and below. A tap shows the playlist alone at once, the clip kept, with no undo step.
#[test]
fn one_close_ribbon_closes_every_bottom_editor_and_keeps_its_clip() {
    let mut rig = Rig::with_drums_and_synth();
    rig.app.add_channel(ChannelKind::AudioClip, 0);
    for channel in [2, 3, 0, 1] {
        rig.app.song.channels[channel].lanes[0].add_clip(0, 4.0);
    }
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let master = crate::ui::preview::ClipKey {
        channel: 0,
        lane: 0,
        id: 1,
    };
    let x = egui::Id::new("editor_close");
    // The touch area of a ribbon whose left edge is at `left`, under the band at the area top.
    let touch = |left: f32| {
        egui::Rect::from_min_size(pos2(left, 2.0), vec2(1.5 * U, 1.3 * U))
            .expand2(vec2(0.3 * U, 0.0))
            .with_max_y(2.0 + 1.6 * U)
    };
    // Nothing to undo: a tap that made a step would show.
    rig.app.history = History::new(&rig.app.song);
    for (nav, view) in [
        (Nav::Edit(2), View::PianoRoll(2)),
        (Nav::Edit(1), View::DrumEditor(1)),
        (Nav::EditClip(master), View::AutomationEditor(0)),
        (Nav::Edit(3), View::AudioEditor(3)),
    ] {
        rig.app.navigate(nav);
        rig.frame(vec![egui::Event::PointerGone]);
        rig.slide_split();
        assert_eq!(rig.app.nav.view, view);
        let current = crate::ui::preview::current(view, &rig.app.slots, &rig.app.song);
        // Just left of the logo: 1170 − 4.4 U, to the pixel.
        let at = rig.ctx.read_response(x).expect("the ribbon").rect;
        let want = touch((1170.0 - 4.4 * U).floor());
        assert!(
            (at.min - want.min).length() + (at.max - want.max).length() < 1e-3,
            "{view:?} {at:?}"
        );
        rig.click_id(x);
        assert_eq!(rig.app.nav.view, View::Playlist, "{view:?}");
        assert!(!rig.hidden());
        // The split is 1 at once: from the next frame the playlist has the whole area.
        rig.frame(vec![]);
        assert_eq!(rig.app.main_screen.split.f, 1.0);
        assert_eq!(
            crate::ui::preview::current(view, &rig.app.slots, &rig.app.song),
            current
        );
        assert!(!rig.app.history.can_undo(), "{view:?}");
    }
    // With the rack open, the editor ends at its left edge and the ribbon sits 1.8 U in from it.
    rig.edit(2);
    rig.app.navigate(Nav::Open(Overlay::Rack(2)));
    rig.slide_rack();
    let at = rig.ctx.read_response(x).expect("the ribbon").rect;
    assert!(
        (at.left() + 0.3 * U - (530.0 - 1.8 * U)).abs() <= 1.0,
        "{at:?}"
    );
    // Its tap collapses the editor and keeps the rack (and every other overlay).
    rig.click_id(x);
    assert_eq!(
        (rig.app.nav.view, rig.overlays()),
        (View::Playlist, vec![Overlay::Rack(2)])
    );
}

/// The editor area collapses whenever the current track has no current clip: an editor whose clip goes (deleted, or
/// undone, in the playlist above) shows the playlist alone, and its next tap makes no clip.
#[test]
fn an_editor_whose_clip_goes_collapses_to_the_playlist() {
    let mut rig = Rig::with_drums_and_synth();
    for channel in [1, 2] {
        rig.app.song.channels[channel].lanes[0].add_clip(0, 4.0);
    }
    rig.app.song_edited(SongEdit::Final, 0);
    for channel in [1, 2] {
        let id = rig.app.song.channels[channel].lanes[0].clips[0].id;
        let key = crate::ui::preview::ClipKey {
            channel,
            lane: 0,
            id,
        };
        rig.app.navigate(Nav::EditClip(key));
        rig.slide_split();
        let view = rig.app.nav.view;
        assert_ne!(view, View::Playlist);
        rig.app.song.channels[channel].lanes[0].clips.clear();
        rig.app.song_edited(SongEdit::Final, 0);
        rig.frame(vec![]);
        assert_eq!(rig.app.nav.view, View::Playlist, "{view:?}");
        rig.slide_split();
        assert!(rig.app.song.channels[channel].lanes[0].clips.is_empty());
    }
}

/// A new note's sound ends with the roll, however the view leaves it: the shared X within the note's gate, or
/// the mixer button.
#[test]
fn the_rolls_new_note_stops_when_the_x_or_the_mixer_button_takes_the_roll_away() {
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    for away in [None, Some(MIXER)] {
        let mut rig = Rig::with_drums_and_synth();
        rig.app.song.channels[2].lanes[0].add_clip(0, 4.0);
        rig.app.song_edited(SongEdit::Final, 0);
        rig.edit(2);
        rig.frame(vec![egui::Event::PointerGone]);
        rig.slide_split();
        let grid = rig
            .ctx
            .read_response(egui::Id::new("piano_roll").with("grid"))
            .unwrap();
        let x = rig
            .ctx
            .read_response(egui::Id::new("editor_close"))
            .unwrap();
        // A tap on the empty grid: a new note, sounding for its gate.
        rig.click(pos2(grid.rect.left() + 4.0, grid.rect.center().y));
        let [crate::song::Event::Note { key, .. }] =
            rig.app.song.channels[2].lanes[0].clips[0].events[..]
        else {
            panic!("no note");
        };
        rig.take();
        // Within the next 0.1 s frame, the X (pressed and released) or the mixer button.
        let at = away.unwrap_or(x.rect.center());
        rig.frame(vec![
            egui::Event::PointerMoved(at),
            button(at, true),
            button(at, false),
        ]);
        assert_ne!(rig.app.nav.view, View::PianoRoll(2), "{away:?}");
        assert_eq!(rig.take(), [format!("NoteOff 2 {key}")], "{away:?}");
    }
}

/// The bottom editors with their Snap buttons, each with a clip from beat 0: the piano roll (channel 2), the drum
/// editor's free grid (a notes lane of channel 1), the automation editor (the master) and the audio editor (channel 3):
/// how to open each, its id (its Snap button's is `.with("snap")`, its play box's `.with("knob")`), and its snap state.
#[allow(clippy::type_complexity)]
fn snap_editors(rig: &mut Rig) -> Vec<(Nav, egui::Id, fn(&Rig) -> bool)> {
    rig.app.add_channel(ChannelKind::AudioClip, 0);
    for channel in [3, 0, 2] {
        rig.app.song.channels[channel].lanes[0].add_clip(0, 4.0);
    }
    // The drum editor's free grid has the button (the step grid has `SWING` there).
    let drums = &mut rig.app.song.channels[1];
    let notes = drums.add_lane(LaneKind::Drums);
    drums.lanes[notes].grid = crate::song::DrumGrid::Notes;
    drums.lanes[notes].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    let key = |channel, lane| crate::ui::preview::ClipKey {
        channel,
        lane,
        id: 1,
    };
    vec![
        (Nav::Edit(2), egui::Id::new("piano_roll"), |rig| {
            rig.app.slots.piano_roll.rolls[&2].snap.on
        }),
        (
            Nav::EditClip(key(1, notes)),
            egui::Id::new("drum_grid"),
            |rig| rig.app.slots.drum_editor.grid.snap.on,
        ),
        (
            Nav::EditClip(key(0, 0)),
            egui::Id::new("automation_editor"),
            |rig| rig.app.slots.automation_editor.editor.snap.on,
        ),
        (Nav::Edit(3), egui::Id::new("audio_editor"), |rig| {
            rig.app.slots.audio_editor.editor.snap.on
        }),
    ]
}

/// Every clip editor has the shared Snap button in its ruler row's left cell; the editors start with Snap on, and a tap
/// turns it off.
#[test]
fn every_clip_editor_has_the_shared_snap_button_and_a_tap_turns_snap_off() {
    let mut rig = Rig::with_drums_and_synth();
    // The play box's touch area, which covers part of the button at beat 0, at beat 2 (the output, which the first
    // click starts, reports it).
    rig.click(OUTSIDE);
    rig.rec.0.borrow_mut().tick = 256;
    rig.frame(vec![]);
    for (nav, id, on) in snap_editors(&mut rig) {
        rig.app.navigate(nav);
        rig.slide_split();
        let view = rig.app.nav.view;
        assert!(on(&rig), "{view:?}");
        rig.click_id(id.with("snap"));
        assert!(!on(&rig), "{view:?}");
    }
}

/// At beat 0, stopped, the play box's touch area (from 0.56 U left of the playhead) reaches over the ruler row's left
/// cell, the Snap button, in the playlist, the piano roll and the automation and audio editors; there it takes the
/// touches (a tap plays, Snap stays). In the drum editor's free grid the 3 U header column keeps it clear of the button.
#[test]
fn at_beat_0_the_play_box_takes_the_taps_over_the_snap_button_in_the_playlist_and_every_editor() {
    let mut rig = Rig::with_drums_and_synth();
    rig.app.settings.count_in = false;
    let editors = snap_editors(&mut rig);
    let rect = |rig: &Rig, id: egui::Id| rig.ctx.read_response(id).expect("drawn").rect;
    let check =
        |rig: &mut Rig, knob: egui::Id, snap: egui::Id, on: fn(&Rig) -> bool, overlap: bool| {
            // The box fades in.
            for _ in 0..40 {
                rig.frame(vec![egui::Event::PointerGone]);
            }
            let view = rig.app.nav.view;
            let both = rect(rig, knob).intersect(rect(rig, snap));
            assert_eq!(both.is_positive(), overlap, "{view:?}");
            if overlap {
                let was = on(rig);
                rig.click(both.center());
                assert!(rig.app.main_screen.playing, "{view:?}");
                assert_eq!(on(rig), was, "{view:?}");
                rig.click(PLAY);
                assert!(!rig.app.main_screen.playing, "{view:?}");
            }
        };
    let playlist = (
        egui::Id::new("playhead_knob"),
        egui::Id::new("playlist_snap"),
    );
    check(
        &mut rig,
        playlist.0,
        playlist.1,
        |rig| rig.app.main_screen.snap.on,
        true,
    );
    for (nav, id, on) in editors {
        rig.app.navigate(nav);
        rig.frame(vec![egui::Event::PointerGone]);
        rig.slide_split();
        let overlap = !matches!(rig.app.nav.view, View::DrumEditor(_));
        check(&mut rig, id.with("knob"), id.with("snap"), on, overlap);
    }
}

/// In the playlist the disc is the playhead knob; where it covers the Snap button it takes the touches (a tap plays).
/// Faded out while playing it takes none (`timeline::knob_shown`): the button has them.
#[test]
fn over_the_snap_button_the_playlists_playhead_knob_takes_the_tap_while_it_shows() {
    let mut rig = Rig::with_synth();
    rig.app.settings.count_in = false;
    for _ in 0..40 {
        rig.frame(vec![egui::Event::PointerGone]);
    }
    let rect = |rig: &Rig, id| rig.ctx.read_response(egui::Id::new(id)).unwrap().rect;
    let both = rect(&rig, "playhead_knob").intersect(rect(&rig, "playlist_snap"));
    assert!(both.is_positive());
    let snap = rig.app.main_screen.snap.on;
    rig.click(both.center());
    assert!(rig.app.main_screen.playing);
    assert_eq!(rig.app.main_screen.snap.on, snap);
    for _ in 0..40 {
        rig.frame(vec![egui::Event::PointerGone]);
    }
    rig.click(both.center());
    assert!(rig.app.main_screen.playing);
    assert_eq!(rig.app.main_screen.snap.on, !snap);
}

// ---- TK: two-keyboard mode and the panel height ----

/// The strip's icon 13 (two rows) on the default panel: the last of six 5 U cells centred on the 1170-point strip, its
/// button 1.74 U under the strip's top.
const TWO_ROWS: Pos2 = pos2(985.0, KEYS_TOP + 1.74 * U);

impl Rig {
    /// The keyboard panel's height as the last frame laid it out, in points.
    fn panel(&self) -> f32 {
        self.app.main_screen.panel.expect("a frame was drawn")
    }

    /// The keys' rows on a panel `panel_u` U tall, top to bottom (one, or `two`), from the layout.
    pub(super) fn rows_at(panel_u: f32, two: bool) -> Vec<egui::Rect> {
        let keys = regions_at(panel_u).keyboard;
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
        crate::ui::keyboard::rows(keys, two, &m)
            .into_iter()
            .map(|r| r.0)
            .collect()
    }
}

/// The rig's regions under a keyboard panel `panel_u` U tall.
fn regions_at(panel_u: f32) -> Regions {
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
    Regions::sized(screen, panel_u * U, 1, &m)
}

/// The keyboard settings button and the strip's icon 13 on a panel `panel_u` U tall.
fn settings_buttons(panel_u: f32) -> (Pos2, Pos2) {
    let r = regions_at(panel_u);
    let bar = r.transport.center().y;
    (
        pos2(KEYBOARD_SETTINGS.x, bar),
        pos2(TWO_ROWS.x, r.transport.bottom() + 1.74 * U),
    )
}

#[test]
fn two_rows_raise_the_panel_to_its_room_a_half_step_a_frame_and_the_second_row_plays_an_octave_up()
{
    // The default panel: 5 U, at once from the first frame. Two rows need a keys area over 3.25 U.
    let rig = Rig::new();
    assert_eq!(rig.panel(), 5.0 * U);
    let spy = Spy::default();
    let mut rig = Rig::with_synth();
    rig.app.storage = Some(Box::new(spy.clone()));
    rig.frame(vec![]);
    assert_eq!(rig.panel(), 5.0 * U);
    rig.click(KEYBOARD_SETTINGS);
    rig.click(TWO_ROWS);
    assert!(rig.app.settings.show_two_rows);
    // The bar, 4.25 U of keys and the open strip: 1.75 + 4.25 + 3 = 9 U.
    assert_eq!(rig.app.settings.preferred_kbd_size, 9.0);
    // h ← h + 0.5·(9 U − h) a frame (at 60 fps, as tests run), snapping within 2 px: 64, 32, 16, 8, 4, 2 px short,
    // then there. Each frame on the way asks for the next; once it is there (and egui has settled) no more frames are
    // asked for.
    let (mut short, mut asks) = (vec![], vec![]);
    for _ in 0..8 {
        rig.frame(vec![]);
        let delay = rig.out.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        short.push(9.0 * U - rig.panel());
        asks.push(delay.is_zero());
    }
    assert_eq!(short, [64.0, 32.0, 16.0, 8.0, 4.0, 2.0, 0.0, 0.0]);
    assert_eq!(asks[..6], [true; 6]);
    assert!(!asks[7]);
    // Kept: the settings were written with both values.
    let stored = Settings::load(&spy);
    assert!(stored.show_two_rows);
    assert_eq!(stored.preferred_kbd_size, 9.0);

    // Closed strip: the second row over the main one, each key an octave over the key under it.
    let (settings, two_rows) = settings_buttons(9.0);
    rig.click(settings);
    rig.take();
    let [top, bottom] = Rig::rows_at(9.0, true)[..] else {
        panic!()
    };
    let low = |row: egui::Rect| pos2(5.0 * U, row.top() + 0.9 * row.height());
    rig.click(low(top));
    rig.click(low(bottom));
    let played = rig.take();
    assert!(
        played[0].starts_with("NoteOn 1 64 ") && played[1] == "NoteOff 1 64",
        "{played:?}"
    );
    assert!(
        played[2].starts_with("NoteOn 1 52 ") && played[3] == "NoteOff 1 52",
        "{played:?}"
    );

    // Off: one row again from the next frame, and the panel keeps its 9 U.
    rig.click(settings);
    rig.click(two_rows);
    rig.click(settings);
    rig.frame(vec![]);
    assert!(!rig.app.settings.show_two_rows);
    assert_eq!(rig.panel(), 9.0 * U);
    rig.take();
    rig.click(low(top));
    assert!(
        rig.take()[0].starts_with("NoteOn 1 52 "),
        "the top half of one row: E4"
    );
}

#[test]
fn the_chord_kinds_keep_one_row_and_a_5_u_panel_shows_none() {
    // Two rows on, but the kind is Chords: the chord keyboard shows, with no piano row. The place is strip 1's
    // second cell (32-point cells from the keys' top + 2): D min, inversion +2.
    let mut rig = Rig::with_synth();
    rig.app.settings.show_two_rows = true;
    rig.app.settings.preferred_kbd_size = 6.0;
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    rig.frame(vec![]);
    rig.take();
    let [top, _] = Rig::rows_at(6.0, true)[..] else {
        panic!()
    };
    let at = pos2(5.0 * U, top.top() + 0.9 * top.height());
    rig.click(at);
    let played = rig.take();
    assert_eq!(
        played[..3],
        ["NoteOn 1 45 64", "NoteOn 1 50 64", "NoteOn 1 53 64"]
    );
    // Classic: two rows, so the same place is E5.
    rig.app.settings.keyboard_kind = KeyboardMode::Classic as u8;
    rig.frame(vec![]);
    rig.click(at);
    assert!(rig.take()[0].starts_with("NoteOn 1 64 "));
    // A panel of exactly 5 U has a keys area of exactly 3.25 U, not over it: one row.
    rig.app.settings.preferred_kbd_size = 5.0;
    for _ in 0..8 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.panel(), 5.0 * U);
    let [top, _] = Rig::rows_at(5.0, true)[..] else {
        panic!()
    };
    rig.click(pos2(5.0 * U, top.top() + 0.9 * top.height()));
    assert!(rig.take()[0].starts_with("NoteOn 1 52 "));
}

#[test]
fn the_scale_kind_with_key_lock_lays_out_only_the_scales_tones_where_classic_keeps_the_piano() {
    // D major, where C# is a tone: key-lock is on (`HoneydewApp::key_input`).
    let mut rig = Rig::with_synth();
    rig.app.song.key_root = 2;
    rig.app.song.key_scale = crate::ui::keyboard_settings::scale_to_stored(0);
    rig.app.settings.keyboard_kind = KeyboardMode::Scale as u8;
    rig.frame(vec![]);
    let black = rig.app.theme.colors.black_key.to_color32();
    // Black keys: 0.6 of the keys' height (the keys' own background, the full height, may share the colour).
    let black_keys = |rig: &Rig| {
        let is_key = |r: &egui::epaint::RectShape| {
            r.fill == black
                && r.rect.top() >= KEYS_TOP
                && r.rect.height() < 0.9 * (540.0 - KEYS_TOP)
        };
        (rig.out.shapes.iter())
            .filter(|s| matches!(&s.shape, Shape::Rect(r) if is_key(r)))
            .count()
    };
    assert_eq!(black_keys(&rig), 0);
    // High on the keys, half a key into the second slot: D. (2 U itself is the edge C# and D share.)
    let high = pos2(2.5 * U, KEYS_TOP + 4.0);
    rig.take();
    rig.click(high);
    let played = rig.take();
    assert!(played[0].starts_with("NoteOn 1 50 "), "{played:?}");
    // Classic: the piano with its black keys; the same place is C#, a tone, which key-lock keeps.
    rig.app.settings.keyboard_kind = KeyboardMode::Classic as u8;
    rig.frame(vec![]);
    assert!(black_keys(&rig) > 0);
    rig.click(high);
    let played = rig.take();
    assert!(played[0].starts_with("NoteOn 1 49 "), "{played:?}");
}

// ---- CK: the panel's hand resize, tap toggle and scroll zone ----

/// An empty part of the transport bar on a panel `h` points tall.
fn bar_gap(h: f32) -> Pos2 {
    pos2(OUTSIDE.x, 540.0 - h + TRANSPORT_H * U / 2.0)
}

impl Rig {
    /// A quick tap (pressed and let go 0.1 s later, without moving) at `at`.
    fn quick_tap(&mut self, at: Pos2) {
        self.press(at);
        self.release(at);
    }

    /// Frames until the panel stops moving.
    fn settle_panel(&mut self) {
        for _ in 0..12 {
            self.frame(vec![]);
        }
    }
}

#[test]
fn a_bar_drag_resizes_past_1_u_without_a_jump_and_its_release_settles_and_is_kept() {
    let spy = Spy::default();
    let mut rig = Rig::with_synth();
    rig.app.storage = Some(Box::new(spy.clone()));
    rig.frame(vec![]);
    assert_eq!(rig.panel(), 5.0 * U);
    let at = bar_gap(5.0 * U);
    rig.press(at);
    // Exactly 1 U up is still calm; the move past it starts the resize without moving the panel.
    rig.drag_to(at - vec2(0.0, U));
    assert_eq!(rig.panel(), 5.0 * U);
    rig.drag_to(at - vec2(0.0, 40.0));
    assert_eq!(rig.panel(), 5.0 * U);
    // From then on h ← h − Δy per move: 60 points up, then 2 down, laid out in the same frame (the bar's REC is there).
    rig.drag_to(at - vec2(0.0, 100.0));
    assert_eq!(rig.panel(), 5.0 * U + 60.0);
    // REC's caption, about 0.5 U over the bottom of a bar whose top is the panel's (the bottom third's centre, lifted
    // 0.2 U).
    let caption = rig.text_rect("REC").center().y;
    let bar_bottom = 540.0 - (5.0 * U + 60.0) + 1.75 * U;
    assert!(
        (caption - (bar_bottom - 0.5 * U)).abs() < 2.0,
        "no frame of lag: {caption}"
    );
    rig.drag_to(at - vec2(0.0, 98.0));
    assert_eq!(rig.panel(), 5.0 * U + 58.0);
    // The panel follows the finger, not its preferred height, while it is held.
    rig.frame(vec![]);
    assert_eq!(rig.panel(), 5.0 * U + 58.0);
    assert_eq!(rig.app.settings.preferred_kbd_size, 5.0);
    // Let go: h' = h − 10·(last Δy = 2) = 198, kept as both sizes, and the panel settles there half the way a frame.
    rig.release(at - vec2(0.0, 98.0));
    let h = 5.0 * U + 58.0 - 20.0;
    assert_eq!(rig.app.settings.preferred_kbd_size, h / U);
    assert_eq!(rig.app.settings.recent_kbd_size, h / U);
    assert_eq!(
        rig.panel(),
        (5.0 * U + 58.0 + h) / 2.0,
        "on its way from the release"
    );
    rig.settle_panel();
    assert_eq!(rig.panel(), h);
    let stored = Settings::load(&spy);
    assert_eq!(
        (stored.preferred_kbd_size, stored.recent_kbd_size),
        (h / U, h / U)
    );
    // The keys follow: E4 is still where the layout puts it on a 198 pt panel.
    rig.take();
    let keys = regions_at(h / U).keyboard;
    rig.click(pos2(5.0 * U, keys.bottom() - 4.0));
    assert!(rig.take()[0].starts_with("NoteOn 1 52 "));
}

#[test]
fn a_throw_up_fills_the_screen_and_one_down_leaves_the_bar_alone_without_touching_recent() {
    let mut rig = Rig::with_synth();
    let at = bar_gap(5.0 * U);
    // A last move of 40 points up throws it 400 points taller: past H − min, the whole screen.
    rig.press(at);
    rig.drag_to(at - vec2(0.0, 40.0));
    rig.drag_to(at - vec2(0.0, 80.0));
    rig.release(at - vec2(0.0, 80.0));
    assert_eq!(rig.app.settings.preferred_kbd_size, 540.0 / U);
    assert_eq!(rig.app.settings.recent_kbd_size, 540.0 / U);
    rig.settle_panel();
    assert_eq!(rig.panel(), 540.0);
    // From the top, a throw down: under 2.5 U, the bar alone (56 pt); recent stays.
    let at = bar_gap(540.0);
    rig.press(at);
    rig.drag_to(at + vec2(0.0, 40.0));
    rig.drag_to(at + vec2(0.0, 120.0));
    rig.release(at + vec2(0.0, 120.0));
    assert_eq!(rig.app.settings.preferred_kbd_size, 1.75);
    assert_eq!(rig.app.settings.recent_kbd_size, 540.0 / U);
    rig.settle_panel();
    assert_eq!(rig.panel(), 56.0);
}

#[test]
fn a_tap_on_the_bars_gap_collapses_the_panel_and_another_opens_it_to_the_recent_size() {
    let mut rig = Rig::with_synth();
    rig.app.settings.recent_kbd_size = 7.0;
    rig.frame(vec![]);
    rig.quick_tap(bar_gap(5.0 * U));
    assert_eq!(rig.app.settings.preferred_kbd_size, 1.75);
    rig.settle_panel();
    assert_eq!(rig.panel(), 56.0);
    // At the bar alone: the recent size. A tap on a button is the button's, not the toggle's.
    rig.quick_tap(bar_gap(56.0));
    assert_eq!(rig.app.settings.preferred_kbd_size, 7.0);
    assert_eq!(rig.app.settings.recent_kbd_size, 7.0);
    rig.settle_panel();
    assert_eq!(rig.panel(), 7.0 * U);
    let settings = pos2(KEYBOARD_SETTINGS.x, bar_gap(7.0 * U).y);
    rig.quick_tap(settings);
    assert_eq!(rig.overlays(), [Overlay::KeyboardSettings]);
    assert_eq!(rig.app.settings.preferred_kbd_size, 7.0);
    // A press held past 0.3 s is no tap.
    let at = bar_gap(7.0 * U);
    rig.press(at);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.release(at);
    assert_eq!(rig.app.settings.preferred_kbd_size, 7.0);
    // The open strip's gaps toggle too.
    let strip_gap = pos2(1100.0, bar_gap(7.0 * U).y + 1.75 * U);
    rig.quick_tap(strip_gap);
    assert_eq!(rig.app.settings.preferred_kbd_size, 1.75);
}

#[test]
fn the_shortcuts_toggle_snaps_like_a_release() {
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let at = |preferred: f32, recent: f32| {
        let s = Settings {
            preferred_kbd_size: preferred,
            recent_kbd_size: recent,
            ..Settings::default()
        };
        let c = crate::ui::panel_resize::collapsed(1, &m);
        let sizes = toggle_snapped(&s, c, 540.0, &m);
        (sizes.preferred / U, sizes.recent.map(|r| r / U))
    };
    // Over the bar: collapse, recent kept.
    assert_eq!(at(5.0, 7.0), (1.75, None));
    // At the bar: the recent size, which stays recent; one within min of the screen fills it.
    assert_eq!(at(1.75, 7.0), (7.0, Some(7.0)));
    assert_eq!(at(1.75, 16.0), (540.0 / U, Some(540.0 / U)));
    // A recent size under c + 0.75 U opens to c + 0.75 U.
    assert_eq!(at(1.75, 2.0), (2.5, Some(2.5)));
}

#[test]
fn a_sideways_drag_on_the_bar_scrolls_the_keys_flings_and_a_key_stops_it_and_the_wheel_zooms() {
    let mut rig = Rig::with_synth();
    let at = bar_gap(5.0 * U);
    let left = |rig: &Rig| rig.app.main_screen.keys_view.left;
    assert_eq!(left(&rig), 28.0);
    rig.press(at);
    // Past the 0.5 U calm zone it scrolls, then follows the finger 1:1 (a white key is 2 U = 64 points).
    rig.drag_to(at - vec2(20.0, 0.0));
    assert_eq!(left(&rig), 28.0);
    rig.drag_to(at - vec2(84.0, 0.0));
    assert_eq!(left(&rig), 29.0);
    assert_eq!(rig.app.settings.preferred_kbd_size, 5.0);
    // Let go after a 64-point move: it flings at 0.9 of that a frame.
    rig.release(at - vec2(84.0, 0.0));
    rig.frame(vec![]);
    assert!((left(&rig) - (29.0 + 0.9)).abs() < 1e-4, "{}", left(&rig));
    rig.frame(vec![]);
    assert!((left(&rig) - (29.0 + 0.9 + 0.81)).abs() < 1e-4);
    // A key's touch stops it.
    rig.click(E4);
    let stopped = left(&rig);
    rig.frame(vec![]);
    assert_eq!(left(&rig), stopped);
    // The wheel over the bar zooms the keys around the pointer: dy 100 makes them 1/0.9 as wide.
    rig.frame(vec![egui::Event::PointerMoved(at)]);
    rig.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, 100.0),
        modifiers: Default::default(),
    }]);
    let w = rig.app.main_screen.keys_view.white_w;
    assert!((w - 2.0 / 0.9).abs() < 1e-4, "{w}");
}

// ---- CK: the transport bar's placement ----

#[test]
fn a_tap_on_the_cpu_or_ram_readout_opens_project_scrolled_to_its_performance_graph() {
    // Opened from the logo: where the `Performance` header sits on an unscrolled page.
    let mut rig = Rig::with_synth();
    rig.open_menu(MenuTab::Project);
    let unscrolled = rig.text_rect("Performance:").top();
    // The readout's slot: one b = 1.75 U left of ↘'s, at the bar's right end.
    let b = 1.75 * U;
    let mut rig = Rig::with_synth();
    rig.click(pos2(1170.0 - 1.5 * b, BAR_Y));
    assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Project)]);
    for _ in 0..12 {
        rig.frame(vec![]);
    }
    let top = rig.text_rect("Performance:").top();
    assert!(top < unscrolled - U, "{top} vs {unscrolled}");
}

// ---- CK: the pitch and mod wheels ----

const PITCH_WHEEL: crate::song::AutomationTarget = crate::song::AutomationTarget {
    instance_id: 0,
    param: 4,
};
const MOD_WHEEL: crate::song::AutomationTarget = crate::song::AutomationTarget {
    instance_id: 0,
    param: 8,
};

#[test]
fn the_wheels_show_over_5_u_narrow_the_piano_and_drive_pitch_and_cc_1_as_the_recent_control() {
    let mut rig = Rig::with_synth();
    rig.app.settings.show_wheels = true;
    rig.frame(vec![]);
    // A 5 U panel is not over 5 U: no wheels, and the piano starts at the left edge (E4 at 5 U).
    rig.take();
    let low = |keys: egui::Rect| keys.bottom() - 4.0;
    rig.click(pos2(5.0 * U, low(regions_at(5.0).keyboard)));
    assert!(rig.take()[0].starts_with("NoteOn 1 52 "));
    // 6 U: the wheels show, and the piano starts 4 U in.
    rig.app.settings.preferred_kbd_size = 6.0;
    rig.settle_panel();
    let keys = regions_at(6.0).keyboard;
    rig.take();
    rig.click(pos2(9.0 * U, low(keys)));
    assert!(rig.take()[0].starts_with("NoteOn 1 52 "));
    assert!(rig.painted("MOD") && rig.painted("PITCH"));
    // The wheels: under the bar, 6 − 2.25 = 3.75 U tall, a 0.75 U label line over each slider's body; the track runs
    // 0.5 U in from both ends of the body: 64 points long here.
    let body = (keys.top() + 0.75 * U, keys.top() + 3.75 * U);
    let track = (body.0 + 0.5 * U, body.1 - 0.5 * U);
    let id = rig.app.song.channels[1].id;
    // PITCH, pressed on its thumb in the middle and dragged up a quarter of the track: 0.75, live and in the song. A
    // channel param: the song-changed flag, and no undo step.
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    let thumb = pos2(3.0 * U, (track.0 + track.1) / 2.0);
    rig.press(thumb);
    assert_eq!(rig.app.recent, Some((id, PITCH_WHEEL)));
    rig.drag_to(thumb - vec2(0.0, 16.0));
    assert_eq!(rig.app.song.channels[1].params.0[4], 0.75);
    assert_eq!(rig.take(), ["SetParam 1 0 4 0.750"]);
    assert!(rig.app.files.modified());
    assert!(
        rig.app.params_moved && !rig.app.param_gesture,
        "no gesture opened"
    );
    // Let go: its value again, then 0.5 at once.
    rig.release(thumb - vec2(0.0, 16.0));
    assert_eq!(rig.take(), ["SetParam 1 0 4 0.750", "SetParam 1 0 4 0.500"]);
    assert_eq!(rig.app.song.channels[1].params.0[4], 0.5);
    assert_eq!(rig.app.history.steps(), steps);
    // MOD, linked to the synth's param 3 on CC 1: from 0 up half the track, live on param 8, through the link
    // unquantised, and it stays when let go. It is the recent control, which the song does not keep a value for.
    crate::midi::links::link(&mut rig.app.song.channels[1].file.cc_links, 1, 2, 3);
    let bottom = pos2(1.0 * U, track.1);
    rig.press(bottom);
    assert_eq!(rig.app.recent, Some((id, MOD_WHEEL)));
    rig.drag_to(bottom - vec2(0.0, 32.0));
    rig.release(bottom - vec2(0.0, 32.0));
    let log = rig.take();
    assert!(log.contains(&"SetParam 1 0 8 0.500".to_owned()), "{log:?}");
    assert!(log.contains(&"SetParam 1 2 3 0.500".to_owned()), "{log:?}");
    let synth = crate::song::AutomationTarget {
        instance_id: 2,
        param: 3,
    };
    assert_eq!(rig.app.song.channels[1].param(synth), Some(0.5));
    assert_eq!(rig.app.param_value(1, MOD_WHEEL), Some(0.5));
    assert_eq!(rig.app.wheel_values(), [0.5, 0.5]);
    assert!(rig.painted("MOD"));
    // The CTRL menu's `Add automation track` makes it a lane, named for the CC.
    rig.app.slots.ctrl_menu.picked = Some(crate::ui::ctrl_menu::CtrlAction::AddAutomationTrack);
    rig.frame(vec![]);
    let lane = rig.app.song.channels[1].lanes.last().unwrap();
    assert_eq!(
        (lane.target, lane.label.as_str()),
        (Some(MOD_WHEEL), "CC 1 Modulation")
    );
}

#[test]
fn a_mod_wheel_move_records_while_armed_and_rolling() {
    let mut rig = Rig::with_synth();
    rig.app.settings.show_wheels = true;
    rig.app.settings.preferred_kbd_size = 6.0;
    rig.app.settings.count_in = false;
    rig.settle_panel();
    let keys = regions_at(6.0).keyboard;
    let bottom = pos2(1.0 * U, keys.top() + 3.25 * U);
    let rec = transport_rec(6.0);
    rig.click(rec);
    assert!(rig.app.main_screen.armed);
    rig.press(bottom);
    rig.drag_to(bottom - vec2(0.0, 32.0));
    rig.drag_to(bottom - vec2(0.0, 48.0));
    rig.release(bottom - vec2(0.0, 48.0));
    // The armed move started the transport and recorded CC 1 into a lane of its own.
    assert!(rig.app.main_screen.playing);
    let lanes = &rig.app.song.channels[1].lanes;
    assert!(
        lanes
            .iter()
            .any(|l| l.target == Some(MOD_WHEEL) && !l.clips.is_empty()),
        "{:?}",
        lanes.iter().map(|l| l.target).collect::<Vec<_>>()
    );
}

/// REC's centre on a panel `panel_u` U tall.
fn transport_rec(panel_u: f32) -> Pos2 {
    let bar = regions_at(panel_u).transport;
    pos2(bar.center().x - 2.0 * 1.75 * U, bar.center().y)
}

// ---- CK: the chord keyboard ----

#[test]
fn the_chord_kinds_play_the_set_on_the_live_input_and_the_strips_hold_latches() {
    let mut rig = Rig::with_synth();
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    rig.frame(vec![]);
    rig.take();
    // The default 5 U panel: 32-point cells from the keys' top + 2; the top one is inversion +3 (C3 major: 48).
    let keys = regions_at(5.0).keyboard;
    let top_cell = |strip: f32, left: f32| {
        pos2(
            left + (strip + 0.5) * (1170.0 - left) / 8.0,
            keys.top() + 18.0,
        )
    };
    rig.click(top_cell(0.0, 0.0));
    assert_eq!(
        rig.take(),
        [
            "NoteOn 1 48 64",
            "NoteOn 1 52 64",
            "NoteOn 1 55 64",
            "NoteOff 1 48",
            "NoteOff 1 52",
            "NoteOff 1 55"
        ]
    );
    // The strip's Hold: KEYB's hold, an undo step; a tap then latches its cell.
    rig.click(KEYBOARD_SETTINGS);
    rig.frame(vec![]);
    let hold = rig.text_rect("Hold").center();
    rig.click(hold);
    assert!(rig.app.song.file.keyboard.hold);
    assert!(rig.app.history.can_undo());
    rig.click(KEYBOARD_SETTINGS);
    rig.take();
    rig.click(top_cell(0.0, 0.0));
    assert_eq!(
        rig.take(),
        ["NoteOn 1 48 64", "NoteOn 1 52 64", "NoteOn 1 55 64"]
    );
    // Another strip's tap stops every note first (E min +3: 52 55 59; 52 and 55 go off, then on again).
    rig.click(top_cell(2.0, 0.0));
    assert_eq!(
        rig.take(),
        [
            "NoteOff 1 48",
            "NoteOff 1 52",
            "NoteOff 1 55",
            "NoteOn 1 52 64",
            "NoteOn 1 55 64",
            "NoteOn 1 59 64"
        ]
    );
    // The piano back: the latched notes stop.
    rig.app.settings.keyboard_kind = KeyboardMode::Classic as u8;
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOff 1 52", "NoteOff 1 55", "NoteOff 1 59"]);
    // The wheels setting moves the strips 4 U in, even where the wheels do not show (5 U).
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    rig.app.song.file.keyboard.hold = false;
    rig.app.settings.show_wheels = true;
    rig.frame(vec![]);
    assert!(!rig.painted("MOD"));
    rig.click(top_cell(0.0, 4.0 * U));
    assert_eq!(rig.take()[0], "NoteOn 1 48 64");
}

#[test]
fn a_drag_on_the_strips_velocity_knob_turns_it_and_never_resizes_the_panel() {
    let mut rig = Rig::with_synth();
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    rig.click(KEYBOARD_SETTINGS);
    rig.frame(vec![]);
    // The knob's body, under its caption.
    let at = rig.text_rect("Velocity").center() + vec2(0.0, 1.0 * U);
    rig.press(at);
    rig.drag_to(at - vec2(0.0, 40.0));
    rig.drag_to(at - vec2(0.0, 80.0));
    rig.release(at - vec2(0.0, 80.0));
    assert_eq!(rig.app.settings.preferred_kbd_size, 5.0);
    assert_eq!(rig.panel(), 5.0 * U);
    assert!(rig.app.song.file.keyboard.velocity > 0.5);
    // The strip's own gap resizes.
    let gap = pos2(1100.0, at.y);
    rig.press(gap);
    rig.drag_to(gap - vec2(0.0, 40.0));
    rig.drag_to(gap - vec2(0.0, 50.0));
    assert_eq!(rig.panel(), 5.0 * U + 10.0);
}

// ---- CK: the chord editor and the chord set's Save ----

impl Rig {
    /// The keyboard settings strip's `ChordSet` ≡ (the strip open, its `Chords` controls shown): picks `item`.
    fn chord_set_item(&mut self, item: &str) {
        self.frame(vec![]);
        // The box is under the 0.75 U caption, 2 U tall.
        let at = self.text_rect("ChordSet").center() + vec2(0.0, 1.375 * U);
        self.click(at);
        for _ in 0..24 {
            self.frame(vec![]);
        }
        let item = self.text_rect(item).center();
        self.click(item);
        self.frame(vec![]);
    }

    /// Opens or closes the keyboard settings strip on a panel `panel_u` U tall.
    fn toggle_strip(&mut self, panel_u: f32) {
        self.click(settings_buttons(panel_u).0);
        self.frame(vec![]);
    }
}

#[test]
fn the_chord_editor_opens_from_chordset_shrinks_the_strips_and_its_edits_play_and_mark_the_set_edited()
 {
    let mut rig = Rig::with_synth();
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    rig.app.settings.preferred_kbd_size = 12.0;
    rig.settle_panel();
    rig.toggle_strip(12.0);
    rig.chord_set_item("Open Editor");
    assert!(rig.app.slots.keyboard_settings.editor);
    rig.toggle_strip(12.0);
    rig.take();
    // K = 10.25 U: the strips are the bottom 2.5 U, one cell each above its 1 U label; the editor fills the rest.
    let keys = regions_at(12.0).keyboard;
    let strips_top = keys.bottom() - 2.5 * U;
    let strip = |s: f32| pos2((s + 0.5) * 1170.0 / 8.0, strips_top + 0.75 * U);
    // A strip's tap plays its root position and retargets the editor.
    rig.click(strip(2.0));
    assert_eq!(
        rig.take(),
        [
            "NoteOn 1 40 64",
            "NoteOn 1 43 64",
            "NoteOn 1 47 64",
            "NoteOff 1 40",
            "NoteOff 1 43",
            "NoteOff 1 47"
        ]
    );
    assert_eq!(rig.app.main_screen.chords.target, 2);
    assert!(rig.painted("Maj7") && rig.painted("Sus4") && rig.painted("+2"));
    // `Maj` on E min: E major plays while held, and the set is edited.
    let row_h = (strips_top - keys.top()) / 4.0;
    let maj = pos2(1170.0 / 12.0, keys.top() + 1.5 * row_h);
    let before = rig.app.song.file.keyboard.clone();
    rig.press(maj);
    assert_eq!(
        rig.take(),
        ["NoteOn 1 40 64", "NoteOn 1 44 64", "NoteOn 1 47 64"]
    );
    let k = &rig.app.song.file.keyboard;
    assert_eq!(k.set_state, crate::song::SET_EDITED);
    let set = crate::song::chords::ChordSet::parse(&k.chord_json).unwrap();
    assert_eq!((set.chords[2].base_note, set.chords[2].quality), (4, 0));
    assert_eq!(
        set.chords[0],
        crate::song::chords::ChordSet::default_set(false)
            .transposed(0)
            .chords[0]
    );
    rig.release(maj);
    assert_eq!(rig.take(), ["NoteOff 1 40", "NoteOff 1 44", "NoteOff 1 47"]);
    // The strip names the set edited; an undo takes it back.
    rig.toggle_strip(12.0);
    assert!(rig.painted("* Major"));
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.app.song.file.keyboard, before);
    // `Close Editor`: the six cells again.
    rig.chord_set_item("Close Editor");
    assert!(!rig.app.slots.keyboard_settings.editor);
}

#[test]
fn chordset_save_marks_the_set_saved_at_once_and_writes_custom_chord_set_into_my_chord_sets() {
    let mem = MemFiles::default();
    mem.0.borrow_mut().names_only = true;
    let mut rig = Rig::with_synth();
    rig.app.files = ProjectFiles::with_provider(Box::new(mem.clone()));
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    // D, Minor Natural (id 10): the untouched default is `Minor`, while KEYB still holds the new song's Major JSON.
    (rig.app.song.key_root, rig.app.song.key_scale) = (2, 10);
    rig.toggle_strip(5.0);
    assert!(rig.painted("Default: Minor"));
    rig.chord_set_item("Save");
    // Saved at once, before the dialog's answer.
    assert_eq!(
        rig.app.song.file.keyboard.set_state,
        crate::song::SET_LOADED
    );
    let dialog = Overlay::PresetDialog {
        name: "Custom Chord Set".into(),
        kind: crate::project_files::PresetKind::ChordSet,
    };
    assert_eq!(rig.overlays(), [Overlay::KeyboardSettings, dialog]);
    rig.frame(vec![]);
    assert!(rig.painted("New Chord Set"), "{:?}", rig.texts());
    super::rack_wiring::dialog_save(&mut rig);
    let path = std::path::Path::new("My Chord Sets/Custom Chord Set.chords");
    assert_eq!(mem.0.borrow().saves, [(path.to_owned(), false)]);
    // The file is the set in use (in the key, D); the set takes the file's name.
    let bytes = rig.app.files.read_user_file(path).unwrap();
    let set = crate::song::chords::ChordSet::parse(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(
        set,
        crate::song::chords::ChordSet::default_set(true).transposed(2)
    );
    // KEYB keeps that set, so the keyboard plays on what it played.
    assert_eq!(rig.app.song.file.keyboard.chord_json, set.to_json());
    rig.frame(vec![]);
    assert_eq!(
        rig.app.song.file.keyboard.chord_set_name,
        "Custom Chord Set"
    );
    rig.frame(vec![]);
    // The ChordSet box shows it, shrunk (here truncated) to its cell.
    let label = &rig.app.slots.keyboard_settings.chords.set_label;
    assert_eq!(label, "Custom Chord Set");
    assert!(rig.painted("Custom ...d Set"), "{:?}", rig.texts());
}

// ---- CK: the keyboard kind is a setting (`keyboardKind`) ----

#[test]
fn the_keyboard_kind_is_the_persisted_keyboardkind_setting() {
    let spy = Spy::default();
    let mut rig = Rig::with_synth();
    rig.app.storage = Some(Box::new(spy.clone()));
    rig.click(KEYBOARD_SETTINGS);
    rig.frame(vec![]);
    // The `Keyboard` spinner's `v`, twice: Scale, then Chords.
    for want in [KeyboardMode::Scale, KeyboardMode::Chords] {
        let label = format!("{:?}", rig.app.settings.keyboard_kind());
        let down = rig.text_rect(&label).center() + vec2(0.0, 0.85 * U);
        rig.click(down);
        rig.frame(vec![]);
        assert_eq!(rig.app.settings.keyboard_kind(), want);
    }
    assert_eq!(Settings::load(&spy).keyboard_kind, 2);
    // A new start reads it.
    let app = HoneydewApp::with_storage(Some(Box::new(spy.clone())));
    assert_eq!(app.settings.keyboard_kind(), KeyboardMode::Chords);
    assert!(
        !rig.app.piano_keys(),
        "the synth's notes lane: the chord keyboard"
    );
    // Reset puts Classic back.
    let reset = rig.text_rect("Reset").center();
    rig.click(reset);
    assert_eq!(rig.app.settings.keyboard_kind(), KeyboardMode::Classic);
}

#[test]
fn a_wobbly_upward_drag_at_45_or_58_degrees_resizes_and_never_scrolls_the_keys() {
    for degrees in [45.0_f32, 58.0] {
        let mut rig = Rig::with_synth();
        let at = bar_gap(5.0 * U);
        let dir = vec2(-degrees.to_radians().cos(), -degrees.to_radians().sin());
        rig.press(at);
        // 30 points along it: past the 0.5 U sideways calm at 45°, short of 1 U up.
        for d in [30.0, 50.0, 90.0] {
            rig.drag_to(at + dir * d);
        }
        assert_eq!(
            rig.app.main_screen.keys_view.left, 28.0,
            "{degrees}°: no scroll"
        );
        assert!(rig.panel() > 5.0 * U, "{degrees}°: {}", rig.panel());
        rig.release(at + dir * 90.0);
    }
}

#[test]
fn the_shortcut_reopens_a_collapsed_panel_at_every_density_and_ui_scale() {
    for ppp in [1.0, 1.25, 1.5, 2.0, 2.75, 3.0] {
        for factor in [60, 75, 90, 100, 110, 125, 150, 175, 200] {
            let m = Metrics::from_pixels_per_point(ppp, factor);
            let u = m.u(1.0);
            // Collapsed by the shortcut itself, then the shortcut again: the recent 7 U (on a tall screen).
            let open = Settings {
                preferred_kbd_size: 5.0,
                recent_kbd_size: 7.0,
                ..Settings::default()
            };
            let c = crate::ui::panel_resize::collapsed(1, &m);
            let shut = toggle_snapped(&open, c, 2000.0, &m);
            let collapsed = Settings {
                preferred_kbd_size: shut.preferred / u,
                ..open.clone()
            };
            let back = toggle_snapped(&collapsed, c, 2000.0, &m);
            assert!(
                (back.preferred - 7.0 * u).abs() < 1e-3,
                "ppp {ppp}, scale {factor}: {back:?}"
            );
        }
    }
}

#[test]
fn a_held_chord_ends_on_its_channel_after_a_channel_change_and_when_the_close_x_hides_the_panel() {
    let mut rig = Rig::with_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    rig.app.selected_channel = 1;
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    rig.frame(vec![]);
    rig.take();
    let keys = regions_at(5.0).keyboard;
    let cell = pos2(1170.0 / 16.0, keys.top() + 18.0);
    rig.press(cell);
    assert_eq!(rig.take()[0], "NoteOn 1 48 64");
    // Another channel becomes current while the chord is held: its notes still end on channel 1.
    rig.app.selected_channel = 2;
    rig.release(cell);
    assert_eq!(rig.take(), ["NoteOff 1 48", "NoteOff 1 52", "NoteOff 1 55"]);
    // Held while the close X hides the panel: the notes stop.
    rig.press(cell);
    rig.take();
    rig.app.main_screen.panel_hidden = true;
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOff 2 48", "NoteOff 2 52", "NoteOff 2 55"]);
}

// ---- CK: the chord keyboard records as live keys do ----

/// The undo steps there are (undone, then redone back).
fn undo_count(rig: &mut Rig) -> usize {
    let history = &mut rig.app.history;
    let n = std::iter::from_fn(|| history.undo()).count();
    for _ in 0..n {
        history.redo();
    }
    n
}

/// Channel 1's recorded notes, as (song tick, key, length in beats), in order.
fn recorded_notes(rig: &Rig) -> Vec<(u32, u8, f64)> {
    let clips = rig.app.song.channels[1].lanes[0].clips.iter();
    let mut notes: Vec<_> = clips
        .flat_map(|c| {
            c.events.iter().filter_map(move |e| match *e {
                crate::song::Event::Note {
                    pos,
                    key,
                    len_beats,
                    ..
                } => Some((c.start_tick + pos, key, len_beats)),
                _ => None,
            })
        })
        .collect();
    notes.sort_by_key(|n| (n.0, n.1));
    notes
}

#[test]
fn an_armed_chord_held_a_beat_and_a_half_records_one_take_of_its_three_notes() {
    let ppq = u64::from(crate::song::PPQ);
    let mut rig = Rig::with_synth();
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    rig.app.settings.count_in = false;
    rig.app.main_screen.armed = true;
    rig.frame(vec![]);
    let undos = undo_count(&mut rig);
    // The top cell: inversion +3 of C3 major, 48 52 55.
    let cell = pos2(1170.0 / 16.0, regions_at(5.0).keyboard.top() + 18.0);
    rig.rec.0.borrow_mut().tick = 2 * ppq;
    rig.press(cell);
    assert!(
        rig.app.main_screen.playing,
        "an armed press starts playback"
    );
    rig.rec.0.borrow_mut().tick = 2 * ppq + 3 * ppq / 2;
    rig.release(cell);
    rig.click(PLAY);
    let at = 2 * crate::song::PPQ;
    assert_eq!(
        recorded_notes(&rig),
        [(at, 48, 1.5), (at, 52, 1.5), (at, 55, 1.5)]
    );
    assert_eq!(undo_count(&mut rig), undos + 1, "one take, one undo step");
}

#[test]
fn an_armed_strum_slide_records_two_separate_notes() {
    let ppq = u64::from(crate::song::PPQ);
    let mut rig = Rig::with_synth();
    rig.app.settings.keyboard_kind = KeyboardMode::Strum as u8;
    rig.app.settings.count_in = false;
    rig.app.main_screen.armed = true;
    rig.frame(vec![]);
    // Strum's 16-point cells from the keys' top + 2: the top two are indices 11 (G5, 79) and 10 (E5, 76).
    let top = regions_at(5.0).keyboard.top();
    let (first, second) = (
        pos2(1170.0 / 16.0, top + 10.0),
        pos2(1170.0 / 16.0, top + 26.0),
    );
    rig.rec.0.borrow_mut().tick = 2 * ppq;
    rig.press(first);
    rig.rec.0.borrow_mut().tick = 2 * ppq + ppq / 2;
    rig.drag_to(second);
    rig.rec.0.borrow_mut().tick = 3 * ppq;
    rig.release(second);
    rig.click(PLAY);
    let p = crate::song::PPQ;
    assert_eq!(
        recorded_notes(&rig),
        [(2 * p, 79, 0.5), (2 * p + p / 2, 76, 0.5)]
    );
}

#[test]
fn a_chord_held_on_the_editor_stops_when_the_editor_closes_under_it() {
    let mut rig = Rig::with_synth();
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    rig.app.settings.preferred_kbd_size = 12.0;
    rig.settle_panel();
    rig.toggle_strip(12.0);
    rig.chord_set_item("Open Editor");
    rig.toggle_strip(12.0);
    rig.take();
    // `Maj`, focused and held by Enter: the target strip's chord sounds.
    let maj = egui::Id::new("chord_editor").with((1_usize, 0_usize));
    crate::ui::focus::focus(&rig.ctx, maj);
    rig.frame(vec![]);
    rig.frame(vec![key_event(egui::Key::Enter, egui::Modifiers::NONE)]);
    let on = rig.take();
    assert!(
        on.len() == 3 && on.iter().all(|n| n.starts_with("NoteOn 1 ")),
        "{on:?}"
    );
    // The editor closes with Enter still down (as another finger's `Close Editor` would close it): the chord stops.
    rig.app.slots.keyboard_settings.editor = false;
    rig.frame(vec![]);
    let off: Vec<String> = on
        .iter()
        .map(|n| n.rsplit_once(' ').unwrap().0.replace("NoteOn", "NoteOff"))
        .collect();
    assert_eq!(rig.take(), off);
}

#[test]
fn with_the_chord_editor_open_the_bar_works_and_the_strips_take_their_taps_over_it() {
    // At the default 5 U panel (K = 3.25 U) the editor runs from the keys' top down under the strips and off the
    // screen; it never reaches the bar, whose controls all work. The strips are drawn over it and take the touches
    // there first.
    let mut rig = Rig::with_synth();
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    rig.app.slots.keyboard_settings.editor = true;
    rig.frame(vec![]);
    rig.frame(vec![]);
    let keys = regions_at(5.0).keyboard;
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let (editor, strips) = crate::ui::chord_editor::editor_rects(keys, &m);
    assert_eq!(editor.top(), keys.top());
    assert!(editor.bottom() > 540.0 && strips.top() > editor.top());
    let keyb = rig.app.song.file.keyboard.clone();
    // Strip 2's cell, over the editor's `Maj7` (row 0): E min's root position plays, and the editor retargets.
    let cell = pos2(2.5 * 1170.0 / 8.0, keys.top() + 1.25 * U);
    let maj7 = crate::ui::chord_editor::button_rect(editor, 0, 2, 7, &m);
    assert!(strips.contains(cell) && maj7.contains(cell));
    rig.take();
    rig.click(cell);
    assert_eq!(notes(&rig.take(), true), [(1, 40), (1, 43), (1, 47)]);
    assert_eq!(rig.app.main_screen.chords.target, 2);
    assert_eq!(rig.app.song.file.keyboard, keyb, "no edit under the strips");
    // The band above the strips is the editor's: `7`.
    let seventh = crate::ui::chord_editor::button_rect(editor, 0, 1, 7, &m);
    let at = pos2(seventh.center().x, (seventh.top() + strips.top()) / 2.0);
    rig.click(at);
    assert_ne!(rig.app.song.file.keyboard, keyb, "`7` edits the chord");
    // The bar: Play and icon 48.
    rig.click(PLAY);
    assert!(rig.app.main_screen.playing);
    rig.click(KEYBOARD_SETTINGS);
    assert_eq!(rig.overlays(), [Overlay::KeyboardSettings]);
    assert!(rig.app.slots.keyboard_settings.editor);
}

/// The keys the commands `log` turn on or off (`on`), as (channel, key), in order.
fn notes(log: &[String], on: bool) -> Vec<(usize, u8)> {
    let what = if on { "NoteOn " } else { "NoteOff " };
    (log.iter())
        .filter_map(|c| c.strip_prefix(what))
        .map(|c| {
            let mut words = c.split(' ').map(|w| w.parse::<usize>().unwrap());
            (words.next().unwrap(), words.next().unwrap() as u8)
        })
        .collect()
}

#[test]
fn the_chord_editor_stays_open_through_escape_a_kind_change_a_hidden_panel_and_a_song_load() {
    // The chord editor reappears still open when the chord keyboard shows again.
    let mut rig = Rig::with_synth();
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    rig.app.slots.keyboard_settings.editor = true;
    rig.frame(vec![]);
    let open = |rig: &Rig| rig.app.slots.keyboard_settings.editor;
    // Escape, with the settings strip open (it closes the strip only) and closed.
    rig.app.navigate(Nav::Open(Overlay::KeyboardSettings));
    rig.frame(vec![]);
    crate::ui::focus::focus(&rig.ctx, egui::Id::new(crate::ui::focus::TRANSPORT));
    rig.frame(vec![]);
    rig.frame(escape());
    assert_eq!(rig.overlays(), []);
    rig.frame(escape());
    assert!(open(&rig), "Escape");
    // Another kind and back.
    rig.app.settings.keyboard_kind = KeyboardMode::Classic as u8;
    rig.frame(vec![]);
    rig.app.settings.keyboard_kind = KeyboardMode::Strum as u8;
    rig.frame(vec![]);
    assert!(open(&rig) && rig.painted("Maj7"), "a kind change");
    // The transport X, then the panel back.
    rig.click(CLOSE);
    assert!(rig.app.main_screen.panel_hidden);
    rig.app.main_screen.panel_hidden = false;
    rig.frame(vec![]);
    assert!(open(&rig) && rig.painted("Maj7"), "a hidden panel");
    // A song load.
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.frame(vec![]);
    assert!(open(&rig) && rig.painted("Maj7"), "a song load");
}

#[test]
fn the_settings_strips_reset_closes_the_chord_editor_and_resets_the_chord_controls() {
    // Reset closes the editor with the rest of its chord reset: the default set, `Octave` 0, `Timing` 0, `Velocity`
    // 0.5, `Hold` off.
    use crate::song::chords::default_keyb;
    use crate::song::{SET_DEFAULT, SET_EDITED};
    use crate::ui::keyboard_settings::{CHROMATIC, scale_to_stored};
    let mut rig = Rig::with_synth();
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    rig.app.slots.keyboard_settings.editor = true;
    let k = &mut rig.app.song.file.keyboard;
    (k.chord_json, k.chord_set_name) = (default_keyb(7).0, "Mine".into());
    (k.set_state, k.octave_shift, k.timing, k.velocity, k.hold) = (SET_EDITED, -2, 20.0, 0.8, true);
    rig.app.navigate(Nav::Open(Overlay::KeyboardSettings));
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.painted("* Mine"));
    let reset = rig.text_rect("Reset").center();
    rig.click(reset);
    assert!(!rig.app.slots.keyboard_settings.editor);
    let k = &rig.app.song.file.keyboard;
    assert_eq!(
        (k.set_state, k.chord_json.clone(), k.chord_set_name.as_str()),
        (
            SET_DEFAULT,
            default_keyb(scale_to_stored(CHROMATIC)).0,
            "Major"
        )
    );
    assert_eq!(
        (k.octave_shift, k.timing, k.velocity, k.hold),
        (0, 0.0, 0.5, false)
    );
    rig.frame(vec![]);
    assert!(!rig.painted("Maj7"));
    // Back to `Chords`: the strip shows the defaults.
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    rig.frame(vec![]);
    assert!(rig.painted("Default: Major"));
}

impl Rig {
    /// The chord editor open on a 12 U panel, an empty log: the editor fills the keys area above the strips.
    fn chord_editor_12u(&mut self) {
        self.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
        self.app.settings.preferred_kbd_size = 12.0;
        self.app.slots.keyboard_settings.editor = true;
        self.settle_panel();
        self.take();
    }
}

/// `Maj` on the 12 U panel's editor.
fn editor_maj() -> Pos2 {
    let keys = regions_at(12.0).keyboard;
    let row_h = (keys.bottom() - 2.5 * U - keys.top()) / 4.0;
    pos2(1170.0 / 12.0, keys.top() + 1.5 * row_h)
}

#[test]
fn a_chord_held_on_the_editor_stops_on_a_focus_loss_and_stays_stopped_until_a_new_press() {
    // The editor tracks its finger as the keyboard does, so a focus loss lets it go (egui keeps the button down).
    let mut rig = Rig::with_synth();
    rig.chord_editor_12u();
    rig.press(editor_maj());
    let on = rig.take();
    assert!(
        on.len() == 3 && on.iter().all(|n| n.starts_with("NoteOn 1 ")),
        "{on:?}"
    );
    let off: Vec<String> = on
        .iter()
        .map(|n| n.rsplit_once(' ').unwrap().0.replace("NoteOn", "NoteOff"))
        .collect();
    rig.frame(vec![egui::Event::WindowFocused(false)]);
    assert_eq!(rig.take(), off);
    rig.frame(vec![]);
    rig.release(editor_maj());
    assert_eq!(rig.take(), Vec::<String>::new());
    // A new press plays again.
    rig.press(editor_maj());
    assert_eq!(rig.take(), on);
}

#[test]
fn a_chord_held_on_the_editor_stays_stopped_after_the_web_page_hides() {
    // The hide stops every note at once (`on_hidden`); the next frame must not start the editor's chord again.
    let mut rig = Rig::with_synth();
    rig.chord_editor_12u();
    rig.press(editor_maj());
    let on = rig.take();
    assert_eq!(on.len(), 3, "{on:?}");
    rig.app.on_hidden();
    assert_eq!(rig.take().len(), 3, "the hide stops the chord");
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.release(editor_maj());
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn a_pitch_bend_springs_back_when_the_wheels_go_away_mid_drag() {
    // The wheels never see a drag end once they are gone, by the close X (the panel hidden), Ctrl+K (the panel toggled
    // down to 5 U or less) or the wheels setting turned off. PITCH springs back as its release would: the song keeps
    // 0.5, and the release that comes later sends nothing.
    for how in ["close", "ctrl+k", "setting"] {
        let mut rig = Rig::with_synth();
        rig.app.settings.show_wheels = true;
        rig.app.settings.preferred_kbd_size = 6.0;
        rig.settle_panel();
        let keys = regions_at(6.0).keyboard;
        let thumb = pos2(3.0 * U, keys.top() + 2.25 * U);
        rig.press(thumb);
        rig.drag_to(thumb - vec2(0.0, 16.0));
        assert_eq!(rig.app.song.channels[1].params.0[4], 0.75, "{how}");
        rig.take();
        match how {
            "close" => rig.app.main_screen.panel_hidden = true,
            "ctrl+k" => rig.frame(key_tap(egui::Key::K, egui::Modifiers::COMMAND)),
            _ => rig.app.settings.show_wheels = false,
        }
        rig.settle_panel();
        assert!(!rig.painted("PITCH"), "{how}");
        let pitch: Vec<String> = (rig.take().into_iter())
            .filter(|c| c.starts_with("SetParam 1 0 4 "))
            .collect();
        assert_eq!(
            pitch.last().map(String::as_str),
            Some("SetParam 1 0 4 0.500"),
            "{how}"
        );
        assert_eq!(rig.app.song.channels[1].params.0[4], 0.5, "{how}");
        rig.release(thumb - vec2(0.0, 16.0));
        assert_eq!(rig.take(), Vec::<String>::new(), "{how}");
        assert_eq!(rig.app.song.channels[1].params.0[4], 0.5, "{how}");
    }
}

#[test]
fn a_chord_cell_tapped_within_one_frame_sounds_for_the_tap_gate() {
    // Its note-offs came with its note-ons, which the engine drains together: a silent chord. They wait for the tap
    // gate, as a tapped piano key's does.
    let mut rig = Rig::with_synth();
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    let cell = pos2(1170.0 / 16.0, KEYS_TOP + 18.0);
    rig.frame(vec![egui::Event::PointerMoved(cell)]);
    rig.take();
    let tapped = rig.time + 0.1;
    rig.frame(vec![button_event(cell, true), button_event(cell, false)]);
    assert_eq!(
        rig.take(),
        ["NoteOn 1 48 64", "NoteOn 1 52 64", "NoteOn 1 55 64"]
    );
    rig.next_frame_at(tapped + TAP_GATE / 2.0);
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.next_frame_at(tapped + TAP_GATE + 1e-3);
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOff 1 48", "NoteOff 1 52", "NoteOff 1 55"]);
}

#[test]
fn a_held_chord_stops_on_a_focus_loss_on_the_web_pages_hide_and_on_a_song_load() {
    // The chord keyboard's own focus-loss release, `stop_notes`' (the hide's, at once, with no frame to come), and a
    // load's.
    let mut rig = Rig::with_synth();
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    let cell = pos2(1170.0 / 16.0, KEYS_TOP + 18.0);
    rig.frame(vec![egui::Event::PointerMoved(cell)]);
    rig.take();
    let on = ["NoteOn 1 48 64", "NoteOn 1 52 64", "NoteOn 1 55 64"];
    let off = ["NoteOff 1 48", "NoteOff 1 52", "NoteOff 1 55"];
    rig.button(cell, true);
    assert_eq!(rig.take(), on);
    rig.frame(vec![egui::Event::WindowFocused(false)]);
    assert_eq!(rig.take(), off, "the focus loss");
    rig.release(cell);
    rig.button(cell, true);
    assert_eq!(rig.take(), on);
    rig.app.on_hidden();
    assert_eq!(rig.take(), off, "the hide, at once");
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.release(cell);
    rig.button(cell, true);
    assert_eq!(rig.take(), on);
    rig.app.song_edited(SongEdit::Replaced, 0);
    let log = rig.take();
    assert_eq!(log[..3], off, "the load, before its engine: {log:?}");
}

#[test]
fn opening_the_chord_editor_moves_the_focus_to_its_first_button_and_closing_it_gives_the_focus_back()
 {
    // Opening moves the keyboard focus into the editor (its first target in reading order, `6`). The editor
    // is no modal: no way back is kept for it, so a control that later goes never sends the focus to `ChordSet`;
    // Closing it from the popup goes back to the box, as any pick there does.
    use crate::ui::focus;
    let mut rig = Rig::with_synth();
    rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    rig.app.settings.preferred_kbd_size = 12.0;
    rig.settle_panel();
    rig.toggle_strip(12.0);
    let chord_set = egui::Id::new("keyboard_settings").with("chord_set");
    focus::focus(&rig.ctx, chord_set);
    rig.frame(vec![]);
    assert_eq!(focus::focused(&rig.ctx), Some(chord_set));
    let tap = |k| key_tap(k, egui::Modifiers::NONE);
    rig.frame(tap(egui::Key::Enter));
    for _ in 0..24 {
        rig.frame(vec![]);
    }
    for _ in 0..3 {
        rig.frame(tap(egui::Key::ArrowDown));
    }
    rig.frame(tap(egui::Key::Enter));
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.app.slots.keyboard_settings.editor);
    assert_eq!(
        focus::focused(&rig.ctx),
        Some(crate::ui::chord_editor::first_button())
    );
    assert_eq!(focus::ways_back(&rig.ctx), 0);
    // Tab on, out of the editor's first button: still none.
    rig.frame(tap(egui::Key::Tab));
    assert_ne!(
        focus::focused(&rig.ctx),
        Some(crate::ui::chord_editor::first_button())
    );
    assert_eq!(focus::ways_back(&rig.ctx), 0);
    // `Close Editor` from the box by keyboard: back on the box.
    focus::focus(&rig.ctx, chord_set);
    rig.frame(vec![]);
    rig.frame(tap(egui::Key::Enter));
    for _ in 0..24 {
        rig.frame(vec![]);
    }
    for _ in 0..3 {
        rig.frame(tap(egui::Key::ArrowDown));
    }
    rig.frame(tap(egui::Key::Enter));
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(!rig.app.slots.keyboard_settings.editor);
    assert_eq!(focus::focused(&rig.ctx), Some(chord_set));
    assert_eq!(focus::ways_back(&rig.ctx), 0);
}

#[test]
fn the_floating_logo_and_the_menu_header_logo_are_text() {
    // The silhouette is white (`text`) on the floating logo's `moduleFill` disc and beside the app name on the menu's
    // gray header.
    let mut rig = Rig::with_synth();
    rig.frame(vec![]);
    let colors = &rig.app.theme.colors;
    let text = colors.text.to_color32();
    let rect = |rig: &Rig, id: egui::Id| rig.ctx.read_response(id).unwrap().rect;
    // Its icon sits 0.125 U left of the disc's centre, a little past the touch area's left edge.
    let corner = rig.image_tint_in(rect(&rig, egui::Id::new("app_logo")).expand(4.0));
    rig.app
        .navigate(Nav::Open(Overlay::MainMenu(MenuTab::Songs)));
    rig.frame(vec![]);
    rig.frame(vec![]);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1170.0, 540.0));
    let m = crate::metrics::Metrics::from_pixels_per_point(1.0, crate::metrics::DEFAULT_UI_SCALE);
    let header = rig.image_tint_in(crate::ui::main_menu::shell_layout(screen, &m).header);
    assert_eq!(corner, Some(text));
    assert_eq!(header, Some(text));
}

//! SONGS `New` through the Song options dialog, and what it writes.

use eframe::egui::Shape;

use super::*;
use crate::song::TimeSig;
use crate::ui::song_options::{ID, TITLE};

/// Clicks the dialog's widget `part`, then lets a popup it opens show.
fn click_part(rig: &mut Rig, part: &str) {
    rig.click_id(egui::Id::new(ID).with(part));
    rig.frame(vec![]);
}

/// Clicks where the last frame painted `text` last (a popup item over what it covers).
fn click_painted(rig: &mut Rig, text: &str) {
    let at = (rig.out.shapes.iter().rev())
        .find_map(|s| match &s.shape {
            Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect().center()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{text:?} not painted"));
    rig.click(at);
    rig.frame(vec![]);
}

#[test]
fn ok_makes_one_new_song_with_the_options_and_no_undo_history() {
    // An edited song: New asks about it after the options, and Discard lets the new one in.
    let mut rig = Rig::with_synth();
    assert!(rig.app.history.can_undo());
    rig.open_menu(MenuTab::Songs);
    rig.click_id(songs_id(("big", 0_usize)));
    assert!(rig.texts().contains(&TITLE.to_owned()));
    click_part(&mut rig, "tempo");
    click_painted(&mut rig, "100 BPM");
    click_painted(&mut rig, "Time division:");
    click_part(&mut rig, "second");
    click_painted(&mut rig, "8");
    click_part(&mut rig, "beats");
    click_painted(&mut rig, "3");
    assert_eq!(rig.take(), Vec::<String>::new(), "no song yet");
    click_part(&mut rig, "ok");
    choose(&mut rig, 2);
    rig.frame(vec![]);
    let song = &rig.app.song;
    let made = TimeSig {
        subdivision: 4,
        beats_per_bar: 3,
    };
    // 100 BPM × 8/4.
    assert_eq!((song.time_sig, song.tempo_bpm), (made, 200.0));
    assert_eq!(song.channels.len(), 1);
    assert!(!rig.app.history.can_undo());
    assert_eq!(rig.overlays(), []);
    let replaced = rig.take().iter().filter(|c| *c == "ReplaceSong").count();
    assert_eq!(replaced, 1);
    assert!(!rig.app.files.modified());
}

#[test]
fn cancel_makes_no_song() {
    let mut rig = Rig::with_synth();
    let song = rig.app.song.clone();
    rig.open_menu(MenuTab::Songs);
    rig.click_id(songs_id(("big", 0_usize)));
    click_part(&mut rig, "tempo");
    click_painted(&mut rig, "200 BPM");
    click_part(&mut rig, "cancel");
    assert_eq!(rig.app.song, song);
    assert!(rig.app.history.can_undo());
    assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Songs)]);
    assert!(!rig.texts().contains(&TITLE.to_owned()));
    assert_eq!(rig.take(), Vec::<String>::new());
}

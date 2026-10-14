//! SONGS `New`'s template menu through the whole app.

use super::*;
use crate::content::{Entry, Factory};
use crate::song::TimeSig;

/// The templates' song: two drum channels after the master, 87 BPM, 3/4.
fn beat() -> Song {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::Drums);
    song.tempo_bpm = 87.0;
    song.time_sig = TimeSig {
        subdivision: 4,
        beats_per_bar: 3,
    };
    song
}

/// A rig whose factory has `Drill`, `D&B` and `Drift Phonk` templates of [`beat`], with SONGS open.
fn rig_with_templates(mut rig: Rig) -> Rig {
    let bytes = crate::flm::write(&beat()).unwrap();
    let entries = ["Drill", "D&B", "Drift Phonk"].map(|name| {
        let bytes = bytes.clone();
        Entry::new(format!("Templates/{name}.songtmpl"), move || bytes.clone())
    });
    rig.app.files.factory = Box::leak(Box::new(Factory::new(entries.into())));
    rig.open_menu(MenuTab::Songs);
    rig
}

/// Opens `New`'s menu and picks the template at menu row `row` (`Empty`, the header, then the templates).
fn pick(rig: &mut Rig, row: usize) {
    rig.click_id(songs_id(("big", 0_usize)));
    rig.frame(vec![]);
    rig.click_id(songs_id("new").with((0_usize, row)));
}

#[test]
fn a_template_makes_an_untitled_song_with_its_channels_and_tempo_and_no_history() {
    let mut rig = rig_with_templates(Rig::new());
    assert!(!rig.app.files.modified());
    // The menu: `Empty`, `Templates`, then `D&B`, `Drift Phonk`, `Drill`.
    rig.click_id(songs_id(("big", 0_usize)));
    rig.frame(vec![]);
    let texts = rig.texts();
    let menu = ["Empty", "Templates", "D&B", "Drift Phonk", "Drill"];
    let at = |t: &str| texts.iter().position(|x| x == t).unwrap_or(usize::MAX);
    assert!(menu.windows(2).all(|w| at(w[0]) < at(w[1])), "{texts:?}");
    // `Drift Phonk`.
    rig.click_id(songs_id("new").with((0_usize, 3_usize)));
    rig.frame(vec![]);
    let song = &rig.app.song;
    assert_eq!((song.channels.len(), song.tempo_bpm), (3, 87.0));
    assert_eq!(song.time_sig, beat().time_sig);
    assert_eq!(song.title, "New Song");
    assert!(!rig.app.history.can_undo());
    assert!(!rig.app.files.modified());
    assert_eq!(rig.take(), ["Stop", "ReplaceSong"]);
    // Untitled: the first save asks for the name.
    rig.app.files.save(&rig.app.song);
    assert!(matches!(
        rig.app.files.take_overlay(),
        Some(Overlay::SaveDialog { .. })
    ));
}

#[test]
fn a_template_asks_about_unsaved_changes_first() {
    let mut rig = rig_with_templates(Rig::with_synth());
    let edited = rig.app.song.clone();
    assert!(rig.app.files.modified());
    pick(&mut rig, 4);
    rig.frame(vec![]);
    // The question, and the song as it was.
    assert!(
        rig.overlays().iter().any(|o| matches!(
            o,
            Overlay::Message {
                tag: DISCARD_TAG,
                ..
            }
        )),
        "{:?}",
        rig.overlays()
    );
    assert_eq!(rig.app.song, edited);
    // `Cancel` keeps it.
    choose(&mut rig, 3);
    rig.frame(vec![]);
    assert_eq!(rig.app.song, edited);
    assert_eq!(rig.take(), Vec::<String>::new());
    // `Discard changes` lets the template in.
    rig.open_menu(MenuTab::Songs);
    pick(&mut rig, 4);
    choose(&mut rig, 2);
    rig.frame(vec![]);
    assert_eq!(rig.app.song.channels.len(), 3);
    assert_eq!(rig.app.song.tempo_bpm, 87.0);
    assert!(!rig.app.history.can_undo());
    assert_eq!(rig.take(), ["Stop", "ReplaceSong"]);
}

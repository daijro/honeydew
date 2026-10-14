//! The Save dialog's exports and PROJECT `Quick Render` in the app: the dialog kept from frame to frame, the
//! render in slices behind the busy screen, the overwrite question, the flashes and the errors.

use eframe::egui::Shape;
use honeydew_audio::sample::decode;

use super::*;
use crate::project_files::export::{NO_SOUND, RENDER_FAILED};
use crate::project_files::{CANCEL, OVERWRITE_TAG};
use crate::render::Remainder;
use crate::song::Event;
use crate::ui::save_dialog::{DialogLayout, ExportFormat, layout};

/// The Save dialog's places on the rig's screen.
fn dialog() -> DialogLayout {
    let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
    layout(
        screen,
        &Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX),
    )
}

const WAV: usize = 4;
const FLAC: usize = 5;
const ZIP: usize = 2;

impl Rig {
    /// A rig with an in-memory provider (and a handle on it) and a one-note song: 120 BPM, a MiniSynth playing A4 for
    /// a beat in a 2-beat clip. No audio output runs.
    fn exporting() -> (Self, MemFiles) {
        let mut rig = Self::new();
        let mem = MemFiles::default();
        rig.app.files = ProjectFiles::with_provider(Box::new(mem.clone()));
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let note = Event::Note {
            pos: 0,
            len_beats: 1.0,
            key: 69,
            velocity: 204,
            pan: 127,
            slide: false,
        };
        song.channels[1].lanes[0].add_clip(0, 2.0).events.push(note);
        rig.app.song = song;
        rig.app.song_edited(SongEdit::Replaced, 0);
        rig.app.files.saved();
        rig.frame(vec![]);
        (rig, mem)
    }

    /// `Save New`: the Save dialog over the song, drawn.
    fn open_dialog(&mut self) {
        self.app.files.save_as(&self.app.song);
        self.frame(vec![]);
        self.frame(vec![]);
    }

    /// The render page of the format button `format`.
    fn render_page(&mut self, format: usize) {
        self.open_dialog();
        self.click(dialog().formats[format].center());
        self.click(dialog().save.center());
        self.frame(vec![]);
    }

    /// Frames until the export under way is done: the busy screen's titles they showed, in order without repeats.
    fn finish_export(&mut self) -> Vec<String> {
        let mut titles: Vec<String> = Vec::new();
        for _ in 0..100_000 {
            if self.app.export.is_none() {
                break;
            }
            self.frame(vec![]);
            for o in &self.app.nav.overlays {
                if let Overlay::Busy { title } = o
                    && titles.last() != Some(title)
                {
                    titles.push(title.clone());
                }
            }
        }
        assert!(self.app.export.is_none(), "the export ends");
        titles
    }

    /// The last frame painted `text`.
    fn shows(&self, text: &str) -> bool {
        (self.out.shapes.iter())
            .any(|s| matches!(&s.shape, Shape::Text(t) if t.galley.text() == text))
    }

    fn flash_text(&self) -> Option<&str> {
        self.app.flash.as_ref().map(|(text, _)| text.as_str())
    }
}

fn written(mem: &MemFiles, path: &str) -> Option<Vec<u8>> {
    mem.0.borrow().files.get(Path::new(path)).cloned()
}

#[test]
fn save_wav_render_writes_a_file_that_decodes_back() {
    let (mut rig, mem) = Rig::exporting();
    // No output opens: the render runs at 44 100 Hz.
    rig.no_device = true;
    rig.render_page(WAV);
    assert!(rig.shows("New Song.wav"), "the render page");
    rig.click(dialog().render.center());
    // The dialog closes and the busy screen takes its place.
    assert!(matches!(rig.overlays()[..], [Overlay::Busy { .. }]));
    let titles = rig.finish_export();
    assert_eq!(
        titles,
        [
            "Rendering \"New Song.wav\"",
            "Processing \"New Song.wav\"",
            "Encoding \"New Song.wav\""
        ]
    );
    assert_eq!(rig.overlays(), []);
    let file = written(&mem, "My Tracks/New Song.wav").expect("written");
    let decoded = decode(&file).unwrap();
    assert_eq!(decoded.sample_rate, 44_100);
    assert!(decoded.right.is_some());
    assert!(decoded.left.iter().any(|x| x.abs() > 0.01), "it has sound");
    assert_eq!(
        rig.flash_text(),
        Some("Render complete\nSaved to: My Tracks/New Song.wav")
    );
    // `Render` stores the format for `Quick Render`; an export is no song edit.
    assert_eq!(rig.app.settings.last_audio_format, ExportFormat::Wav.id());
    assert!(!rig.app.files.modified());
    assert!(!rig.app.history.can_undo());
}

#[test]
fn render_stores_its_format_for_quick_render() {
    let (mut rig, mem) = Rig::exporting();
    rig.app.slots.export.page.remainder = Remainder::Cut;
    rig.render_page(FLAC);
    rig.click(dialog().render.center());
    rig.finish_export();
    assert!(written(&mem, "My Tracks/New Song.flac").is_some());
    assert_eq!(rig.app.settings.last_audio_format, ExportFormat::Flac.id());
}

#[test]
fn the_busy_screen_shows_the_title_and_takes_the_clicks() {
    let (mut rig, _) = Rig::exporting();
    rig.app.slots.export.page.remainder = Remainder::Leave;
    rig.render_page(WAV);
    rig.click(dialog().render.center());
    rig.frame(vec![]);
    assert!(rig.app.export.is_some());
    assert!(rig.shows("Rendering \"New Song.wav\""));
    // The logo lies under it.
    rig.click(LOGO);
    assert!(matches!(rig.overlays()[..], [Overlay::Busy { .. }]));
    rig.finish_export();
}

#[test]
fn a_render_stops_the_transport_and_runs_at_the_engine_rate() {
    let (mut rig, mem) = Rig::exporting();
    rig.click(OUTSIDE);
    rig.click(PLAY);
    assert!(rig.app.main_screen.playing);
    rig.take();
    rig.app.slots.export.page.remainder = Remainder::Cut;
    // As the dialog's `Render` asks.
    let song = rig.app.song.clone();
    rig.app
        .files
        .export(&song, ExportFormat::Flac, "Take", false);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(!rig.app.main_screen.playing);
    assert!(rig.take().contains(&"Stop".to_owned()));
    rig.finish_export();
    let decoded = decode(&written(&mem, "My Tracks/Take.flac").unwrap()).unwrap();
    // The recorder's output runs at 48 kHz; the render has the song's length exactly: its 2-beat clip rounded up to a
    // bar, 4 beats.
    assert_eq!(decoded.sample_rate, 48_000);
    assert_eq!(decoded.left.len(), 96_000);
}

#[test]
fn split_resets_when_the_dialog_opens_again_and_the_other_options_stay() {
    let (mut rig, _) = Rig::exporting();
    rig.render_page(WAV);
    let l = dialog();
    rig.click(l.toggles[0].center());
    rig.click(l.toggles[1].center());
    rig.click(l.toggles[2].center());
    rig.click(l.close.center());
    assert_eq!(rig.overlays(), []);
    rig.render_page(WAV);
    let render = rig.app.slots.export.dialog.as_ref().expect("open").render;
    assert!(render.normalize && render.high_quality && !render.split);
    // It opened anew: on the format row, the name the song proposes.
    rig.click(l.close.center());
    rig.open_dialog();
    let open = rig.app.slots.export.dialog.as_ref().expect("open");
    assert_eq!(open.format, ExportFormat::Flm);
    assert_eq!(open.name, "New Song");
}

#[test]
fn a_zip_is_saved_from_the_dialog_which_closes_and_flashes_saved() {
    let (mut rig, mem) = Rig::exporting();
    rig.open_dialog();
    rig.click(dialog().formats[ZIP].center());
    rig.click(dialog().save.center());
    assert_eq!(rig.overlays(), []);
    assert!(written(&mem, "My Songs/New Song.zip").is_some());
    rig.frame(vec![]);
    assert_eq!(rig.flash_text(), Some("Saved!"));
}

#[test]
fn an_existing_file_asks_cancel_keeps_the_dialog_and_replace_renders_over_it() {
    let (mut rig, mem) = Rig::exporting();
    rig.app.slots.export.page.remainder = Remainder::Cut;
    let old = b"old".to_vec();
    (mem.0.borrow_mut().files).insert("My Tracks/New Song.wav".into(), old.clone());
    rig.render_page(WAV);
    rig.click(dialog().render.center());
    rig.frame(vec![]);
    assert!(matches!(
        &rig.overlays()[..],
        [
            Overlay::SaveDialog { .. },
            Overlay::Message {
                tag: OVERWRITE_TAG,
                ..
            }
        ]
    ));
    // Cancel (the third choice, after the two lines): back to the dialog, nothing rendered.
    rig.frame(vec![]);
    assert!(rig.shows(CANCEL));
    rig.click_id(message_item(4));
    rig.frame(vec![]);
    assert!(matches!(&rig.overlays()[..], [Overlay::SaveDialog { .. }]));
    assert!(rig.app.export.is_none());
    // Replace files (the first).
    rig.click(dialog().render.center());
    rig.frame(vec![]);
    rig.click_id(message_item(2));
    rig.frame(vec![]);
    assert!(matches!(rig.overlays()[..], [Overlay::Busy { .. }]));
    rig.finish_export();
    let file = written(&mem, "My Tracks/New Song.wav").unwrap();
    assert!(decode(&file).is_ok());
}

#[test]
fn quick_render_renders_the_last_format_with_the_kept_options_and_no_dialog() {
    let (mut rig, mem) = Rig::exporting();
    rig.app.settings.last_audio_format = ExportFormat::Flac.id();
    rig.app.slots.export.page.remainder = Remainder::Cut;
    // Split is no kept option (it resets whenever the dialog opens): Quick Render renders the mix.
    rig.app.slots.export.page.split = true;
    rig.app.song.title = "Beat".into();
    rig.open_menu(MenuTab::Project);
    rig.click_id(project_id("Quick Render"));
    assert!(
        !(rig.overlays().iter()).any(|o| matches!(o, Overlay::SaveDialog { .. })),
        "no dialog"
    );
    rig.finish_export();
    let decoded = decode(&written(&mem, "My Tracks/Beat.flac").unwrap()).unwrap();
    // Cut: exactly the song's bar (its 2-beat clip rounded up), at the rate of the output the first click opened.
    assert_eq!(decoded.left.len(), 96_000);
    assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Project)]);
}

#[test]
fn a_silent_song_shows_the_song_has_no_sound() {
    let (mut rig, mem) = Rig::exporting();
    rig.app.song.channels[1].lanes[0].clips[0].events.clear();
    rig.app.slots.export.page.remainder = Remainder::Cut;
    rig.render_page(WAV);
    rig.click(dialog().render.center());
    rig.finish_export();
    rig.frame(vec![]);
    assert!(matches!(
        &rig.overlays()[..],
        [Overlay::Message { lines, .. }] if lines == &[NO_SOUND]
    ));
    assert!(mem.0.borrow().files.is_empty());
}

#[test]
fn split_writes_each_stem_into_its_folder_with_its_progress_and_flashes_the_folder() {
    let (mut rig, mem) = Rig::exporting();
    rig.app.slots.export.page.remainder = Remainder::Cut;
    // A second synth playing the same note, under the same name.
    let song = &mut rig.app.song;
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let at = song.channels.len() - 1;
    let clip = song.channels[1].lanes[0].clips[0].clone();
    song.channels[at].lanes[0].clips.push(clip);
    song.channels[at].name = song.channels[1].name.clone();
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.app.files.saved();
    let name = rig.app.song.channels[1].name.clone();
    rig.render_page(WAV);
    rig.click(dialog().toggles[2].center());
    assert!(rig.shows("Enable master effects"), "shown with Split on");
    rig.click(dialog().render.center());
    rig.frame(vec![]);
    // The two lines of the busy title are each centred.
    let rows = (rig.out.shapes.iter())
        .find_map(|s| match &s.shape {
            Shape::Text(t) if t.galley.text().starts_with("Exporting channel") => Some(
                t.galley
                    .rows
                    .iter()
                    .map(|r| t.pos.x + r.rect().center().x)
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .expect("the busy title");
    assert_eq!(rows.len(), 2);
    assert!((rows[0] - rows[1]).abs() < 1.0, "{rows:?}");
    assert!((rows[0] - 585.0).abs() < 1.0, "{rows:?}");
    let titles = rig.finish_export();
    for title in [
        format!("Exporting channel 1: {name}\nEncoding \"{name}.wav\""),
        format!("Exporting channel 2: {name}\nRendering \"{name} 1.wav\""),
        format!("Exporting channel 2: {name}\nEncoding \"{name} 1.wav\""),
    ] {
        assert!(titles.contains(&title), "{title:?} in {titles:?}");
    }
    let folder = "My Tracks/New Song - Multitrack wav";
    for file in [format!("{name}.wav"), format!("{name} 1.wav")] {
        let bytes = written(&mem, &format!("{folder}/{file}")).expect("written");
        let decoded = decode(&bytes).unwrap();
        assert!(
            decoded.left.iter().any(|x| x.abs() > 0.01),
            "{file} has sound"
        );
    }
    assert!(
        written(&mem, "My Tracks/New Song.wav").is_none(),
        "stems only"
    );
    rig.frame(vec![]);
    assert_eq!(
        rig.flash_text(),
        Some(format!("Render complete\nSaved to: {folder}").as_str())
    );
    assert_eq!(rig.overlays(), []);
    assert!(!rig.app.files.modified());
}

#[test]
fn a_stem_that_cannot_be_written_ends_the_export_with_one_message() {
    let (mut rig, mem) = Rig::exporting();
    rig.app.slots.export.page.remainder = Remainder::Cut;
    let song = &mut rig.app.song;
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let at = song.channels.len() - 1;
    let clip = song.channels[1].lanes[0].clips[0].clone();
    song.channels[at].lanes[0].clips.push(clip);
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.app.files.saved();
    mem.0.borrow_mut().write_error = Some("disk full".into());
    rig.app.slots.export.page.split = true;
    let song = rig.app.song.clone();
    rig.app.files.export(&song, ExportFormat::Wav, "Take", true);
    rig.frame(vec![]);
    assert!(rig.app.export.is_some());
    rig.finish_export();
    rig.frame(vec![]);
    assert_eq!(
        mem.0.borrow().saves.len(),
        1,
        "no stem after the failed one"
    );
    assert!(matches!(
        &rig.overlays()[..],
        [Overlay::Message { lines, .. }] if lines == &[RENDER_FAILED, "disk full"]
    ));
    assert_eq!(rig.flash_text(), None);
}

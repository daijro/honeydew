//! Exports: the Save dialog's ZIP, MIDI and audio formats, and PROJECT `Quick Render`.
//!
//! - **Where.** `<user>/<folder>/<name>.<ext>`, the folder per format ([`ExportFormat::folder`]): `My Songs` for ZIP,
//!   `My MIDI` for MIDI, `My Tracks` for audio. A file that exists asks `The file already exists` first: `Replace
//!   files` writes over it, `Keep both` writes `name (n)` beside it, `Cancel` writes nothing and leaves the dialog open.
//!   The web downloads instead; its provider cannot see the downloads folder, so it never asks.
//! - **ZIP** holds `<name>.flm`, the song as a save writes it, and every user file (`@L@`) the song plays, at its path
//!   in the user folder; bundled files never. Nor, ever, a file that really is in the Original Content folder
//!   (imported content never leaves the user folder): `@R@`, an `@L@` path into it, or the
//!   `@L@Online Content` fallback into it. The song keeps their paths, for the recipient's own copy, and the flash
//!   says how many were left out. **MIDI** is [`smf_with`] the whole song, with its note effects applied per `Apply note
//!   effects` ([`ProjectFiles::midi_note_effects`]). Both flash `Saved!`.
//! - **Audio.** [`ProjectFiles::take_export`] hands the export to the app, which runs it as a [`Job`] in slices behind
//!   the busy screen: `Rendering "<file>"` (pass 1, [`Render::step`], 1024-frame blocks), `Processing "<file>"`
//!   ([`Render::finish`]), then `Encoding "<file>"` (pass 2, [`Stream::step`]: the same audio rendered again and
//!   processed), 1024 frames per push into a streaming encoder: 16-bit
//!   [`WavWriter`] or 32-bit float WAV, 16- or 24-bit [`FlacEncoder`], 192 or 320 kbps [`Mp3Encoder`], Vorbis quality
//!   0.4 or 1.0 [`OggEncoder`], per `High quality`. Once written, `Render complete\nSaved to: <path>` flashes.
//! - **Errors.** No audible frame: `The song has no sound`. A render that cannot be built, is too long or cannot be
//!   written: `An error occurred while rendering`. MIDI with no lane to write: `Error: no data to save`. ZIP: the
//!   export messages (`Failed to save project` for the song, `Failed to compress files`, `Failed to export
//!   archive` for the write). A failed MIDI write: `Failed to save file`.
//!
//! - **Stems** (`Split mixer tracks`): one file per channel with content ([`stems_of`]), each rendered with its
//!   channel soloed (the engine's own routing) into `<user>/My Tracks/<name> - Multitrack <ext>/`, stems only.
//!   A silent stem is skipped; with none left the song has no sound. The busy screen shows `Exporting channel %d: %s`
//!   (1-based among the stems) over the stem's stage title. `Enable master effects` off leaves the master's rack
//!   out of each stem ([`RenderSpec::master_effects`]); the mix always plays it. The overwrite question covers the
//!   folder. The web downloads one `<name> - Multitrack <ext>.zip` of the stems instead.
//!
//! the loading titles name the file with its extension (`Rendering "Song.wav"`). `Cancel` on the overwrite
//! question returns to the dialog, as the preset dialog's does. A user file the song names that cannot be read is
//! left out of the archive (the song reports it missing anyway). An archive larger than [`MAX_FILE_BYTES`] fails, as a
//! song save does: it could never be opened again. Stems: `Keep both` numbers the folder (`… - Multitrack wav (1)`);
//! `Replace files` writes over the files of the same names and leaves the folder's others; each stem goes through the
//! whole processing pass (remainder, trim, normalize) on its own; the web archive holds the stems at its root;
//! `Quick Render` renders the mix, since Split is no kept option (it resets whenever the dialog opens).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use honeydew_audio::flac_writer::FlacEncoder;
use honeydew_audio::mp3_writer::Mp3Encoder;
use honeydew_audio::ogg_writer::OggEncoder;
use honeydew_audio::wav_writer::{HEADER_LEN, WavWriter, float32_wav};

use super::{
    FileEvent, MAX_FILE_BYTES, OVERWRITE, ProjectFiles, SAVE_FAILED, SAVE_FILE_FAILED, SAVED,
    in_user_folder, stamped, stem, too_large,
};
use crate::flm::chunk::ContentPath;
use crate::render::{Render, RenderError, RenderSpec, Stream};
use crate::sample_cache::{self, SampleCache};
use crate::smf::smf_with;
use crate::song::{ChannelKind, LaneKind, Song};
use crate::ui::main_menu::library::{FORBIDDEN, check_folder_name};
use crate::ui::save_dialog::{ExportFormat, RenderPage};
use crate::ui::views::Answer;

/// The export messages.
pub const NO_SOUND: &str = "The song has no sound";
pub const RENDER_FAILED: &str = "An error occurred while rendering";
pub const NO_DATA: &str = "Error: no data to save";
pub const ARCHIVE_FAILED: &str = "Failed to export archive";
pub const COMPRESS_FAILED: &str = "Failed to compress files";
/// The message after an FLP export that did not carry everything over exactly; a line for each follows.
pub const FLP_NOTICES: &str = "Saved. These parts differ in the .flp:";
/// The render rate when no audio output runs.
pub const NO_AUDIO_RATE: u32 = 44_100;
/// Frames a slice renders at least: the render's block.
const BLOCK: usize = 1024;
/// The export work one egui frame does.
#[cfg(not(test))]
const SLICE_MS: u64 = 25;
/// The blocks a slice runs in tests, which never read the clock.
#[cfg(test)]
const TEST_SLICE_BLOCKS: usize = 64;

/// An export that went ahead ([`ProjectFiles::take_export`]): the app closes the Save dialog, and renders an audio one
/// into `path`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    pub format: ExportFormat,
    /// The file, or with [`Split::Folder`] the stems' folder.
    pub path: PathBuf,
    pub split: Split,
}

/// `Split mixer tracks`: where the stems go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Split {
    /// Off: the mix, into the file.
    #[default]
    Off,
    /// One file per stem, into the folder `<name> - Multitrack <ext>`.
    Folder,
    /// The web: one archive of the stems, `<name> - Multitrack <ext>.zip`, downloaded.
    Zip,
    /// An FLP's `Include rendered audio`: a WAV stem per channel, kept for the project's archive ([`Done::Rendered`]).
    Flp,
}

/// A split export's stems being written ([`ProjectFiles::write_stem`]): their folder, the files whose writes have not
/// finished, whether every stem is rendered, and whether a write failed. `Render complete` flashes once all are done.
#[derive(Default)]
pub(super) struct StemWrites {
    folder: PathBuf,
    writing: Vec<PathBuf>,
    rendered: bool,
    failed: bool,
}

/// An export waiting for the overwrite question, with the bytes of a ZIP or MIDI one.
pub(super) struct Asked {
    export: Export,
    bytes: Option<Vec<u8>>,
}

/// The format `Quick Render` renders: the one stored as `lastAudioFileFormat` (`id`), else WAV (also for a format with
/// no encoder yet).
pub fn quick_render_format(id: u8) -> ExportFormat {
    ExportFormat::from_id(id)
        .filter(|f| f.is_audio() && f.available())
        .unwrap_or(ExportFormat::Wav)
}

/// `Saved!`, and a line for the imported content files a project archive left out (`left_out`), in the import's
/// `Imported %d file%s` pattern.
fn saved(left_out: usize) -> String {
    let s = if left_out == 1 { "" } else { "s" };
    match left_out {
        0 => SAVED.into(),
        n => format!("{SAVED}\n{n} original content file{s} not included"),
    }
}

/// The message a failed write of `format` shows.
fn failure(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Zip | ExportFormat::Flp => ARCHIVE_FAILED,
        ExportFormat::Midi => SAVE_FILE_FAILED,
        _ => RENDER_FAILED,
    }
}

impl ProjectFiles {
    /// The Save dialog's `SAVE` with ZIP or MIDI, or `Render` with an audio format, with the file name `name`. Checks
    /// the name as `SAVE` does (a bad one shows its message), then writes a ZIP or MIDI file, or hands an audio one to
    /// the app ([`Self::take_export`]), split into stems with `split` ([`Split`]); an FLP with `split` (`Include
    /// rendered audio`) has the app render its stems first ([`Split::Flp`]). A file or stem folder that exists asks
    /// first ([`super::OVERWRITE_TAG`]). FLM ([`Self::dialog_save`]) and the formats that are not
    /// [`ExportFormat::available`] do nothing.
    pub fn export(&mut self, song: &Song, format: ExportFormat, name: &str, split: bool) {
        self.export_only(song, format, name, split, None);
    }

    /// The track menu's `Save as MIDI`: [`Self::export`] of MIDI with the bytes
    /// of channel `channel` alone, on MIDI channel 1 (its drum lanes on 10).
    pub fn save_as_midi(&mut self, song: &Song, channel: usize, name: &str) {
        self.export_only(song, ExportFormat::Midi, name, false, Some(channel));
    }

    /// [`Self::export`], a MIDI file of channel `only` alone when it is `Some`.
    fn export_only(
        &mut self,
        song: &Song,
        format: ExportFormat,
        name: &str,
        split: bool,
        only: Option<usize>,
    ) {
        if format == ExportFormat::Flm || !format.available() {
            return;
        }
        let Some(name) = self.typed_name(name, format.ext()) else {
            return;
        };
        let Some(user) = self.user.clone() else {
            return self.fail(failure(format), "There is no user folder.".into());
        };
        self.export_left_out = 0;
        let bytes = match format {
            ExportFormat::Zip => match self.archive(song, &name) {
                Ok((bytes, left_out)) => {
                    self.export_left_out = left_out;
                    Some(bytes)
                }
                Err((title, detail)) => return self.fail(title, detail),
            },
            // Rendered first ([`Split::Flp`]), then archived.
            ExportFormat::Flp if split => None,
            ExportFormat::Flp => match self.flp_archive(song, &name, &[]) {
                Ok(bytes) => Some(bytes),
                Err(err) => return self.fail(COMPRESS_FAILED, err),
            },
            ExportFormat::Midi => match smf_with(song, only, self.midi_note_effects) {
                Ok(bytes) => Some(bytes),
                Err(_) => {
                    self.message = Some(vec![NO_DATA.into()]);
                    return;
                }
            },
            _ => None,
        };
        let ext = format.ext();
        let (file, split) = match (split, format.is_audio(), self.provider.folders()) {
            (true, false, _) if format == ExportFormat::Flp => {
                (format!("{name}.{ext}"), Split::Flp)
            }
            (true, true, true) => (format!("{name} - Multitrack {ext}"), Split::Folder),
            (true, true, false) => (format!("{name} - Multitrack {ext}.zip"), Split::Zip),
            _ => (format!("{name}.{ext}"), Split::Off),
        };
        let path = user.join(format.folder()).join(&file);
        let export = Export {
            format,
            path,
            split,
        };
        if self.provider.exists(&export.path) {
            self.message = Some(vec![OVERWRITE.into(), file]);
            self.overwrite = Some(name);
            self.export_asked = Some(Asked { export, bytes });
            return;
        }
        self.go(export, bytes);
    }

    /// PROJECT `Quick Render`: [`Self::export`] of `format`, named as the Save dialog would propose, with no dialog: the
    /// mix, never split.
    pub fn quick_render(&mut self, song: &Song, format: ExportFormat) {
        let name = self.proposed_name(song);
        self.export(song, format, &name, false);
    }

    /// The overwrite question for `asked` was answered: `Replace files` writes over the file, `Keep both` writes the
    /// first free `name (n)` beside it (a stem folder's whole name, which may hold dots, takes the ` (n)`), `Cancel`
    /// nothing.
    pub(super) fn export_answered(&mut self, mut asked: Asked, answer: Answer) {
        match answer {
            Answer::Extra => {}
            Answer::Yes if asked.export.split == Split::Folder => {
                let path = &asked.export.path;
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let free = (1..)
                    .map(|n| path.with_file_name(format!("{name} ({n})")))
                    .find(|p| !self.taken(p))
                    .expect("an unbounded range has a free number");
                asked.export.path = free;
            }
            Answer::Yes => {
                let (path, ext) = (&asked.export.path, asked.export.format.ext());
                let name = self.free_name(path.parent(), &stem(path), ext, None);
                asked.export.path = path.with_file_name(format!("{name}.{ext}"));
            }
            Answer::No => return,
        }
        self.go(asked.export, asked.bytes);
    }

    /// The export goes ahead: a ZIP or MIDI one is written now.
    fn go(&mut self, export: Export, bytes: Option<Vec<u8>>) {
        if let Some(bytes) = bytes {
            self.write_export(&export, bytes);
        }
        self.stem_writes = StemWrites::default();
        self.exported = Some(export);
    }

    /// The export that went ahead since the last call, for the app: it closes the Save dialog and renders an audio one.
    pub fn take_export(&mut self) -> Option<Export> {
        self.exported.take()
    }

    /// Writes `export`'s file (the web downloads it). Once written, `Saved!` flashes, or `Render complete\nSaved to:
    /// <path>` for audio; a failed write shows its format's message.
    pub fn write_export(&mut self, export: &Export, bytes: Vec<u8>) {
        self.export_saving = Some(export.clone());
        self.provider.save(&export.path, false, bytes);
    }

    /// Writes the stem file `name` of the split export `export` into its folder. Once every stem is rendered
    /// ([`Self::stems_rendered`]) and written, `Render complete\nSaved to: <folder>` flashes; a failed write shows its
    /// message instead.
    pub fn write_stem(&mut self, export: &Export, name: &str, bytes: Vec<u8>) {
        let path = export.path.join(name);
        self.stem_writes.folder.clone_from(&export.path);
        self.stem_writes.writing.push(path.clone());
        self.provider.save(&path, false, bytes);
    }

    /// A stem of the split export under way could not be written: the app ends the export (its message shows).
    pub fn stem_write_failed(&self) -> bool {
        self.stem_writes.failed
    }

    /// Every stem of the split export under way is rendered ([`Done::Stems`]).
    pub fn stems_rendered(&mut self) {
        self.stem_writes.rendered = true;
        self.stems_flash();
    }

    /// `Render complete` for the stems' folder, once they are all rendered and written, unless a write failed.
    fn stems_flash(&mut self) {
        let writes = &mut self.stem_writes;
        if writes.rendered && writes.writing.is_empty() {
            writes.rendered = false;
            if !std::mem::take(&mut writes.failed) {
                let folder = writes.folder.display();
                self.flash = Some(format!("Render complete\nSaved to: {folder}"));
            }
        }
    }

    /// A render failed: `The song has no sound` with no audible frame, else `An error occurred while rendering`.
    pub fn render_failed(&mut self, error: &RenderError) {
        let line = match error {
            RenderError::Silent => NO_SOUND,
            _ => RENDER_FAILED,
        };
        self.message = Some(vec![line.into()]);
    }

    /// `event` finishes the export write under way, or a stem's.
    pub(super) fn is_export_event(&self, event: &FileEvent) -> bool {
        (self.export_saving.as_ref()).is_some_and(|e| self.finishes(&e.path, event))
            || (self.stem_writes.writing.iter()).any(|path| self.finishes(path, event))
    }

    /// The export write under way finished with `event`: the flash, or the message.
    pub(super) fn export_done(&mut self, event: FileEvent) {
        let writing = &self.stem_writes.writing;
        if let Some(at) = writing.iter().position(|path| self.finishes(path, &event)) {
            self.stem_writes.writing.remove(at);
            if let FileEvent::Failed(_, detail) = event
                && !std::mem::replace(&mut self.stem_writes.failed, true)
            {
                self.fail(RENDER_FAILED, detail);
            }
            return self.stems_flash();
        }
        let Some(export) = self.export_saving.take() else {
            return;
        };
        match event {
            FileEvent::Saved(path) if export.format.is_audio() => {
                self.flash = Some(format!("Render complete\nSaved to: {}", path.display()));
            }
            FileEvent::Saved(_) => {
                self.flash = Some(saved(std::mem::take(&mut self.export_left_out)));
                // What the FLP had no counterpart for.
                let notices = std::mem::take(&mut self.export_notices);
                if !notices.is_empty() {
                    let mut notices = notices;
                    super::cap_notices(&mut notices);
                    self.message = Some(
                        std::iter::once(FLP_NOTICES.to_owned())
                            .chain(notices)
                            .collect(),
                    );
                }
            }
            FileEvent::Failed(_, detail) => self.fail(failure(export.format), detail),
            FileEvent::Opened(..) | FileEvent::Located(_) => {}
        }
    }

    /// The project archive of `song` as `<name>.flm`: the song as a save writes it, then every user file it plays;
    /// and the count of original content files left out. An error carries its message's title.
    fn archive(&self, song: &Song, name: &str) -> Result<(Vec<u8>, usize), (&'static str, String)> {
        let flm =
            (self.codec.write)(&stamped(song, name.to_owned())).map_err(|e| (SAVE_FAILED, e))?;
        let mut entries = vec![(format!("{name}.flm"), flm)];
        let (user_files, left_out) = self.user_files_of(song);
        entries.extend(user_files);
        let bytes = crate::zip::write(&entries).map_err(|e| (COMPRESS_FAILED, e.to_string()))?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err((ARCHIVE_FAILED, too_large()));
        }
        Ok((bytes, left_out))
    }

    /// The user files `song` plays, by their `/` path in the user folder: what the sample cache asks to read for it.
    /// Only a DW program (`.dwp`) is handed on, for its zones' files; nothing else is decoded, parsed or stretched.
    /// A file it plays that really is in the Original Content folder, every link followed, is only counted.
    fn user_files_of(&self, song: &Song) -> (BTreeMap<String, Vec<u8>>, usize) {
        let (read, found) = (sample_cache::reader(self), RefCell::new(BTreeMap::new()));
        let left_out = RefCell::new(std::collections::BTreeSet::new());
        let record = |path: &ContentPath| {
            let bytes = read(path)?;
            let file = self.resolve(path);
            if let Some(file) = file.filter(|file| self.in_content(file)) {
                left_out.borrow_mut().insert(file);
            } else if let Some(relative) = in_user_folder(path) {
                let parts: Vec<_> = relative.iter().map(|p| p.to_string_lossy()).collect();
                found.borrow_mut().insert(parts.join("/"), bytes.clone());
            }
            let (ContentPath::Shorthand { relative, .. } | ContentPath::Plain(relative)) = path;
            let ext = Path::new(relative).extension();
            ext.is_some_and(|e| e.eq_ignore_ascii_case("dwp"))
                .then_some(bytes)
        };
        SampleCache::default().load(song, &record);
        (found.into_inner(), left_out.into_inner().len())
    }
}

/// An audio export under way: the render's pass 1, the start of pass 2, then pass 2 into the encoder, in slices
/// ([`Job::step`]); with [`Split`] on, so for each stem in turn.
pub struct Job {
    export: Export,
    rate: u32,
    high_quality: bool,
    stage: Stage,
    /// A split export's stems.
    split: Option<Box<Stems>>,
}

/// What a [`Job::step`] ended with.
#[derive(Debug)]
pub enum Done {
    /// The file: the mix, or the web's archive of the stems. The job is over.
    File(Vec<u8>),
    /// A stem for the folder ([`Split::Folder`]): its file name and bytes. More may follow.
    Stem(String, Vec<u8>),
    /// Every stem is rendered, at least one handed over. The job is over.
    Stems,
    /// An FLP's stems ([`Split::Flp`]): the song they are of, and each a WAV file by its channel there, for
    /// [`ProjectFiles::flp_rendered`]. The job is over.
    Rendered(Box<Song>, Vec<(usize, Vec<u8>)>),
    /// The job is over: `Silent` also when no stem has sound.
    Failed(RenderError),
}

/// A split export's stems: what each renders from, each one's channel and file name ([`stems_of`]), the one under way,
/// how many have sound, and on the web those, for the archive.
struct Stems {
    song: Song,
    samples: SampleCache,
    spec: RenderSpec,
    stems: Vec<(usize, String)>,
    at: usize,
    written: usize,
    zip: Vec<(String, Vec<u8>)>,
}

enum Stage {
    Rendering(Box<Render>),
    Processing(Box<Render>),
    Encoding {
        stream: Box<Stream>,
        encoder: Encoder,
    },
    Done,
}

impl Job {
    /// Starts rendering `export` of `song`, with `samples` loaded for it, at `rate` Hz with the options of `page`: the
    /// mix, or with [`Split`] the first stem (`Silent` when no channel has content). An FLP's stems are the whole song
    /// and its remainder in 32-bit float, without the master's effects (the project's master has them).
    pub fn new(
        song: &Song,
        samples: &SampleCache,
        export: Export,
        page: RenderPage,
        rate: u32,
    ) -> Result<Self, RenderError> {
        let page = match export.split {
            Split::Flp => RenderPage {
                high_quality: true,
                ..RenderPage::default()
            },
            _ => page,
        };
        let mut spec = RenderSpec {
            range: page.range,
            remainder: page.remainder,
            normalize: page.normalize,
            high_quality: page.high_quality,
            rate,
            solo: None,
            master_effects: export.split == Split::Off || page.master_effects,
        };
        let mut split = None;
        if export.split != Split::Off {
            let ext = match export.split {
                Split::Flp => "wav",
                _ => export.format.ext(),
            };
            let stems = stems_of(song, ext);
            spec.solo = Some(stems.first().ok_or(RenderError::Silent)?.0);
            split = Some(Box::new(Stems {
                song: song.clone(),
                samples: samples.clone(),
                spec,
                stems,
                at: 0,
                written: 0,
                zip: Vec::new(),
            }));
        }
        let render = Render::new(song, samples, spec)?;
        Ok(Self {
            export,
            rate,
            high_quality: page.high_quality,
            stage: Stage::Rendering(Box::new(render)),
            split,
        })
    }

    pub fn export(&self) -> &Export {
        &self.export
    }

    /// The busy screen's title for the stage the next [`Self::step`] runs: `Rendering "<file>"`, `Processing
    /// "<file>"` or `Encoding "<file>"`, under `Exporting channel <n>: <channel name>` for a stem.
    pub fn title(&self) -> String {
        let next = (self.split.as_ref()).is_some_and(|split| split.at < split.stems.len());
        let stage = match self.stage {
            // Between stems: the next one renders next.
            Stage::Done if next => "Rendering",
            Stage::Rendering(_) => "Rendering",
            Stage::Processing(_) => "Processing",
            Stage::Encoding { .. } | Stage::Done => "Encoding",
        };
        let Some(split) = &self.split else {
            let file = self.export.path.file_name().unwrap_or_default();
            return format!("{stage} \"{}\"", file.to_string_lossy());
        };
        let at = split.at.min(split.stems.len() - 1);
        let (channel, file) = &split.stems[at];
        let name = &split.song.channels[*channel].name;
        format!("Exporting channel {}: {name}\n{stage} \"{file}\"", at + 1)
    }

    /// One slice of the stage under way, a block at a time until `over` says the slice is spent (at least one block).
    /// The end of a stage ends the slice, so the busy screen shows the next stage's title before it runs. Returns what
    /// the job ended with, or a stem to write.
    pub fn step(&mut self, over: &mut dyn FnMut() -> bool) -> Option<Done> {
        loop {
            match std::mem::replace(&mut self.stage, Stage::Done) {
                Stage::Rendering(mut render) => {
                    if render.step(BLOCK) {
                        self.stage = Stage::Processing(render);
                        return None;
                    }
                    self.stage = Stage::Rendering(render);
                }
                Stage::Processing(render) => {
                    let stream = match render.finish() {
                        Ok(stream) => stream,
                        Err(RenderError::Silent) if self.split.is_some() => {
                            return self.next_stem(None);
                        }
                        Err(error) => return Some(Done::Failed(error)),
                    };
                    let frames = stream.frames();
                    let encoder =
                        Encoder::new(self.export.format, self.rate, self.high_quality, frames);
                    self.stage = Stage::Encoding {
                        stream: Box::new(stream),
                        encoder,
                    };
                    return None;
                }
                Stage::Encoding {
                    mut stream,
                    mut encoder,
                } => {
                    // Pass 2: a block rendered, and pushed once a whole one is there.
                    if let Some([left, right]) = stream.step() {
                        encoder.push(left, right);
                    }
                    if stream.done() {
                        let bytes = encoder.finish();
                        return match self.split {
                            None => Some(Done::File(bytes)),
                            Some(_) => self.next_stem(Some(bytes)),
                        };
                    }
                    self.stage = Stage::Encoding { stream, encoder };
                }
                Stage::Done => {
                    let split = self.split.as_mut()?;
                    // The next stem starts rendering.
                    if let Some(&(channel, _)) = split.stems.get(split.at) {
                        let spec = RenderSpec {
                            solo: Some(channel),
                            ..split.spec
                        };
                        let render = match Render::new(&split.song, &split.samples, spec) {
                            Ok(render) => render,
                            Err(error) => return Some(Done::Failed(error)),
                        };
                        self.stage = Stage::Rendering(Box::new(render));
                        continue;
                    }
                    return Some(match (split.written, self.export.split) {
                        (0, _) => Done::Failed(RenderError::Silent),
                        // ponytail: deflated in one go, a stall for long stems; in slices if the web's UI stalls.
                        (_, Split::Zip) => match crate::zip::write(&std::mem::take(&mut split.zip))
                        {
                            Ok(zip) => Done::File(zip),
                            // Past zip's 4 GiB, which wasm32 memory never holds anyway.
                            Err(_) => Done::Failed(RenderError::Archive),
                        },
                        (_, Split::Flp) => {
                            let channel = |file: &str| split.stems.iter().find(|s| s.1 == file);
                            let zip = std::mem::take(&mut split.zip).into_iter();
                            let stems =
                                zip.filter_map(|(file, wav)| Some((channel(&file)?.0, wav)));
                            let stems = stems.collect();
                            Done::Rendered(Box::new(std::mem::take(&mut split.song)), stems)
                        }
                        _ => Done::Stems,
                    });
                }
            }
            if over() {
                return None;
            }
        }
    }

    /// The stem under way is done with `bytes` (`None`: it had no sound, and is skipped). A stem for the folder is
    /// handed over (the web keeps it for the archive) before the next one is built, at the next step, which also ends
    /// the job after the last.
    fn next_stem(&mut self, bytes: Option<Vec<u8>>) -> Option<Done> {
        let split = self.split.as_mut()?;
        let file = split.stems[split.at].1.clone();
        split.at += 1;
        let bytes = bytes?;
        split.written += 1;
        if matches!(self.export.split, Split::Zip | Split::Flp) {
            split.zip.push((file, bytes));
            return None;
        }
        Some(Done::Stem(file, bytes))
    }
}

/// The stems of `song`: each channel but the master and the sends that has content (a clip with an event, or an
/// audio clip), with its file `<channel name>.<ext>`. In the name, `/\:*?"<>|`
/// and control characters are `_`, a name no file can have (empty, hidden, a device name) takes a `_` first, and a
/// name taken (in any case: some desktops ignore it) takes ` 1`, ` 2`, ….
fn stems_of(song: &Song, ext: &str) -> Vec<(usize, String)> {
    let mut taken: Vec<String> = Vec::new();
    let channels = song.channels.iter().enumerate();
    channels
        .filter(|(_, c)| !matches!(c.kind, ChannelKind::Master | ChannelKind::EffectTrack))
        .filter(|(_, c)| {
            (c.lanes.iter()).any(|l| {
                (l.clips.iter()).any(|clip| l.kind == LaneKind::Audio || !clip.events.is_empty())
            })
        })
        .map(|(channel, c)| {
            let bad = |ch: char| FORBIDDEN.contains(ch) || ch.is_control();
            let mut base: String = c
                .name
                .chars()
                .map(|ch| if bad(ch) { '_' } else { ch })
                .collect();
            if check_folder_name(&format!("{base}.{ext}")).is_err() {
                base.insert(0, '_');
            }
            let name = std::iter::once(base.clone())
                .chain((1..).map(|n| format!("{base} {n}")))
                .find(|name| !taken.contains(&name.to_lowercase()))
                .expect("an unbounded range has a free number");
            taken.push(name.to_lowercase());
            (channel, format!("{name}.{ext}"))
        })
        .collect()
}

/// Whether the slice of export work begun now is spent: [`SLICE_MS`] of wall time.
#[cfg(not(any(test, target_arch = "wasm32")))]
pub fn slice() -> impl FnMut() -> bool {
    let end = std::time::Instant::now() + std::time::Duration::from_millis(SLICE_MS);
    move || std::time::Instant::now() >= end
}

/// [`slice`] on the web, where std's clock panics: `Date.now()`.
#[cfg(all(not(test), target_arch = "wasm32"))]
pub fn slice() -> impl FnMut() -> bool {
    let end = web_sys::js_sys::Date::now() + SLICE_MS as f64;
    move || web_sys::js_sys::Date::now() >= end
}

/// [`slice`] in tests: spent after [`TEST_SLICE_BLOCKS`] blocks, whatever the clock says.
#[cfg(test)]
pub fn slice() -> impl FnMut() -> bool {
    let mut blocks = 0;
    move || {
        blocks += 1;
        blocks >= TEST_SLICE_BLOCKS
    }
}

/// A streaming stereo encoder (`High quality` picks its settings).
// ponytail: the encoded file is held whole (float WAV 8 B/frame ≈ 690 MB for 30 min at 48 kHz; wasm memory never
// shrinks); a sink (native temp file + rename, web Blob parts) if that bites.
enum Encoder {
    /// 16-bit PCM WAV.
    Pcm16(WavWriter<Cursor<Vec<u8>>>),
    /// 32-bit float WAV: the whole file's header, then the frames as they come.
    Float(Vec<u8>),
    Flac(FlacEncoder),
    /// Boxed: rusty_mp3's state is kilobytes.
    Mp3(Box<Mp3Encoder>),
    /// Boxed: libvorbis's analysis state is kilobytes too.
    Ogg(Box<OggEncoder>),
}

impl Encoder {
    /// For `frames` frames of `format` at `rate` Hz. WAV is also the encoder of a format that has none yet (no such
    /// format is available to export).
    fn new(format: ExportFormat, rate: u32, high_quality: bool, frames: usize) -> Self {
        match (format, high_quality) {
            (ExportFormat::Flac, hq) => {
                Self::Flac(FlacEncoder::new(rate, if hq { 24 } else { 16 }, 2))
            }
            (ExportFormat::Mp3, hq) => Self::Mp3(Box::new(Mp3Encoder::new(rate, hq))),
            (ExportFormat::Ogg, hq) => Self::Ogg(Box::new(OggEncoder::new(rate, hq))),
            (_, false) => {
                let out = Cursor::new(Vec::with_capacity(HEADER_LEN + frames * 4));
                Self::Pcm16(WavWriter::new(out, rate, 2).expect("a Vec takes every write"))
            }
            (_, true) => {
                // `float32_wav`'s header, its sizes those of `frames` frames (capped as its own are).
                let mut out = float32_wav(rate, &[[0f32; 0]; 2]);
                let data = u32::try_from(frames * 8)
                    .unwrap_or(u32::MAX)
                    .min(u32::MAX - 38);
                out[4..8].copy_from_slice(&(38 + data).to_le_bytes());
                out[HEADER_LEN - 4..].copy_from_slice(&data.to_le_bytes());
                out.reserve(frames * 8);
                Self::Float(out)
            }
        }
    }

    fn push(&mut self, left: &[f32], right: &[f32]) {
        match self {
            Self::Pcm16(wav) => {
                let frames: Vec<f32> = (left.iter().zip(right))
                    .flat_map(|(&l, &r)| [l, r])
                    .collect();
                wav.write(&frames).expect("a Vec takes every write");
            }
            Self::Float(out) => {
                out.extend_from_slice(&float32_wav(0, &[left, right])[HEADER_LEN..])
            }
            Self::Flac(flac) => flac.push(&[left, right]),
            Self::Mp3(mp3) => mp3.push(&[left, right]),
            Self::Ogg(ogg) => ogg.push(&[left, right]),
        }
    }

    fn finish(self) -> Vec<u8> {
        match self {
            Self::Pcm16(wav) => wav.finish().expect("a Vec takes every write").into_inner(),
            Self::Float(out) => out,
            Self::Flac(flac) => flac.finish(),
            Self::Mp3(mp3) => mp3.finish(),
            Self::Ogg(ogg) => ogg.finish(),
        }
    }
}

#[cfg(test)]
mod tests;

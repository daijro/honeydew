//! FL Studio projects: opening an `.flp` (or the `.zip` of one and its samples, FL's "Zipped loop package" and our
//! own export), its samples found and kept, the missing ones' stand-ins, `Locate samples…`, and the Save dialog's FLP
//! export, saved as a `.zip` of the `.flp` and its samples.
//!
//! **Finding a sample**, in order: the exact path; the path relative to the project; the archive's file of that path or
//! name; a file of that name in the project's folder and below; one in the user's `My Samples` and `My Recordings`
//! (the web: any file this session holds). FL's path variables (`%FLStudioFactoryData%\…`) never resolve, so their
//! samples are found by name. A found sample is copied to `Unzipped songs/<project name>/`, where a song's files can be
//! named, and loads into its channel (a DW Sampler playing the file, its root key at FL's).
//!
//! **A missing sample** keeps its channel, notes, volume and pan, and plays the closest sound of ours, guessed from
//! the channel's and the sample's names. One summary lists every stand-in, replaced plugin and dropped effect, with
//! `Locate samples…` when samples are missing: the user picks a folder, and each missing sample found there by name
//! loads into its channel (a drum stand-in becomes a one-pad kit of it).

use std::path::{Path, PathBuf};

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
use super::FileEvent;
use super::{LOAD_FAILED, ProjectFiles, UNZIPPED_FOLDER, created_now, stand_ins, stem};
#[cfg(not(target_arch = "wasm32"))]
use super::{RECORDINGS_FOLDER, SAMPLES_FOLDER};
use crate::flm::chunk::ContentPath;
use crate::flp::import::{SampleRef, file_name, file_stem};
use crate::panels::cstm::{load_dw_program, load_dw_programs, pick_dw_program};
use crate::sample_cache::{SampleCache, reader};
use crate::smf::gm;
use crate::song::{ChannelKind, DrumKit, Event, LaneKind, SamplerSlot, Song, SynthKind};
use crate::zip::Entry;

/// The summary's tag while samples are missing, and its choice.
pub const LOCATE_TAG: &str = "project_files_locate";
pub const LOCATE: &str = "Locate samples...";
/// A project's samples' folder is searched this deep, and at most this many entries.
#[cfg(not(target_arch = "wasm32"))]
const SEARCH_DEPTH: usize = 6;
#[cfg(not(target_arch = "wasm32"))]
const SEARCH_ENTRIES: usize = 20_000;
/// A one-shot's Release in a DW Sampler: FL's Sampler plays a sample out after a short note.
const ONE_SHOT_RELEASE: f32 = 0.75;

/// A sample the last FLP open could not find.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Missing {
    /// The channel it belongs to, by id.
    channel: i32,
    /// Its file name, which `Locate samples…` looks for.
    name: String,
    root: u8,
    /// Where the project's samples are kept.
    dir: PathBuf,
}

impl ProjectFiles {
    /// Opens the FLP `bytes` of `path` (from the archive `entries`, which hold its samples) as a new untitled song
    /// titled after the file: its samples found and kept, a stand-in for each one that is missing, and the summary of
    /// what was replaced.
    pub(super) fn open_flp(
        &mut self,
        path: &Path,
        bytes: &[u8],
        entries: Option<&[Entry]>,
    ) -> Option<Song> {
        let imported = match crate::flp::import::import(bytes, self.factory) {
            Ok(imported) => imported,
            Err(err) => {
                self.fail(LOAD_FAILED, err);
                return None;
            }
        };
        let (mut song, notices) = (imported.song, imported.notices);
        let dir = self
            .user
            .clone()
            .map(|u| u.join(UNZIPPED_FOLDER).join(stem(path)));
        self.missing.clear();
        let mut missing = Vec::new();
        for sample in &imported.samples {
            let found = (self.find_sample(path, &sample.path, entries))
                .and_then(|data| self.keep_sample(dir.as_deref()?, file_name(&sample.path), &data));
            match found {
                Some((shorthand, kept)) => load_sample(
                    &mut song,
                    sample.channel,
                    &shorthand,
                    &kept,
                    sample.root,
                    self,
                ),
                None => missing.push(self.stand_in(&mut song, sample, dir.clone())),
            }
        }
        load_dw_programs(&mut song, &reader(self));
        song.title = stem(path);
        song.file.meta.created = created_now();
        self.path = None;
        self.saved();
        self.autosave(&song);
        // The missing samples first: they are what `Locate samples…` is for.
        missing.extend(notices);
        self.message = stand_ins(missing);
        self.locate_asked = self.message.is_some() && !self.missing.is_empty();
        Some(song)
    }

    /// A stand-in for the missing `sample` ([`gm::stand_in`]), remembered for `Locate samples…`; its summary line.
    fn stand_in(&mut self, song: &mut Song, sample: &SampleRef, dir: Option<PathBuf>) -> String {
        let channel = &song.channels[sample.channel];
        let (id, name) = (channel.id, channel.name.clone());
        let loaded = match sample.audio {
            true => "its audio clips stay silent".to_owned(),
            false => {
                let names = [name.as_str(), &file_stem(&sample.path)];
                format!(
                    "plays {}",
                    gm::stand_in(song, sample.channel, &names, self.factory)
                )
            }
        };
        if let Some(dir) = dir {
            self.missing.push(Missing {
                channel: id,
                name: file_name(&sample.path).to_owned(),
                root: sample.root,
                dir,
            });
        }
        format!(
            "{name}: missing sample {}, {loaded}",
            short_path(&sample.path)
        )
    }

    /// The bytes of the sample `stored` names, for the project at `project` (from the archive `entries`), searched as
    /// the module docs say; `None` when it is nowhere.
    fn find_sample(
        &self,
        project: &Path,
        stored: &str,
        entries: Option<&[Entry]>,
    ) -> Option<Vec<u8>> {
        let name = file_name(stored);
        if name.is_empty() {
            return None;
        }
        let plain = stored.replace('\\', "/");
        // FL's `%…%` variables name folders on the machine that saved it.
        let variable = plain.starts_with('%');
        let folder = project.parent().filter(|d| !d.as_os_str().is_empty());
        #[cfg(not(target_arch = "wasm32"))]
        if !self.copies && !variable {
            let exact = Path::new(&plain);
            let relative = folder
                .filter(|_| exact.is_relative())
                .map(|d| d.join(exact));
            for path in [Some(exact.to_owned()), relative].into_iter().flatten() {
                if path.is_absolute() && path.is_file() {
                    return read(&path);
                }
            }
        }
        let same = |e: &&Entry| {
            e.path.eq_ignore_ascii_case(&plain) || file_name(&e.path).eq_ignore_ascii_case(name)
        };
        if let Some(entry) = entries.and_then(|all| all.iter().find(same)) {
            return Some(entry.data.clone());
        }
        if self.copies {
            let path = self.user_copies.find_name(name)?;
            return self.read_user_file(&path).ok();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let user = self.user.as_deref();
            let places = [folder.map(Path::to_owned)]
                .into_iter()
                .chain([SAMPLES_FOLDER, RECORDINGS_FOLDER].map(|f| user.map(|u| u.join(f))));
            for dir in places.flatten() {
                if let Some(path) = search(&dir, name) {
                    return read(&path);
                }
            }
        }
        let _ = (folder, variable);
        None
    }

    /// Keeps `data` as `<dir>/<name>` in the user folder: its shorthand and path, `None` when it cannot be written.
    fn keep_sample(
        &mut self,
        dir: &Path,
        name: &str,
        data: &[u8],
    ) -> Option<(ContentPath, PathBuf)> {
        let path = dir.join(name);
        self.write_user_file(&path, data.to_vec()).ok()?;
        Some((self.user_shorthand(&path)?, path))
    }

    /// The names of the samples the last FLP open could not find.
    fn missing_names(&self) -> Vec<String> {
        self.missing.iter().map(|m| m.name.clone()).collect()
    }

    /// `Locate samples…` answered: on `yes` a folder to look in is asked for.
    pub(super) fn locate_answered(&mut self, yes: bool) {
        self.locate_asked = false;
        if yes && !self.missing.is_empty() {
            self.provider.locate(self.missing_names());
        }
    }

    /// The files `Locate samples…` found, by name: each missing sample among them is kept and loads into its channel
    /// in `song` (one the user deleted takes nothing); the app then rebuilds ([`Self::take_relinked`]). A flash counts
    /// them.
    pub(super) fn located(&mut self, song: &mut Song, files: Vec<(String, Vec<u8>)>) {
        let mut loaded = 0;
        for missing in std::mem::take(&mut self.missing) {
            let file = files
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(&missing.name));
            let index = song.channels.iter().position(|c| c.id == missing.channel);
            let kept =
                file.and_then(|(_, data)| self.keep_sample(&missing.dir, &missing.name, data));
            match (index, kept) {
                (Some(i), Some((shorthand, path))) => {
                    load_sample(song, i, &shorthand, &path, missing.root, self);
                    loaded += 1;
                }
                (Some(_), None) => self.missing.push(missing),
                (None, _) => {}
            }
        }
        let left = self.missing.len();
        self.flash = Some(match left {
            0 => format!("Located {loaded} sample(s)"),
            _ => format!("Located {loaded} sample(s), {left} still missing"),
        });
        self.relinked |= loaded > 0;
    }

    /// Samples were located into the song since the last call: the app makes the edit and rebuilds its audio.
    pub fn take_relinked(&mut self) -> bool {
        std::mem::take(&mut self.relinked)
    }

    /// The Save dialog's FLP export: `<name>.flp` and the samples it plays, as a `.zip`; the notices of what had no FL
    /// counterpart show once it is written.
    pub(super) fn flp_archive(&mut self, song: &Song, name: &str) -> Result<Vec<u8>, String> {
        let mut cache = SampleCache::default();
        cache.load(song, &reader(self));
        let out = crate::flp::export::export(song, &cache);
        let mut files = vec![(format!("{name}.flp"), out.flp)];
        files.extend(out.samples);
        self.export_notices = out.notices;
        crate::zip::write(&files).map_err(|e| e.to_string())
    }
}

/// `path` as a summary shows it: a long one as `...` and its last two parts, so that the line wraps on screen.
fn short_path(path: &str) -> String {
    let separator = if path.contains('\\') { '\\' } else { '/' };
    let parts: Vec<&str> = path.split(['/', '\\']).collect();
    match parts.len() > 3 && path.chars().count() > 40 {
        true => format!(
            "...{separator}{}",
            parts[parts.len() - 2..].join(&separator.to_string())
        ),
        false => path.to_owned(),
    }
}

/// Channel `index` plays the kept sample `shorthand` (its file `path`): a Drums channel (a missing sample's drum
/// stand-in) as a kit of that one pad, its hits on it; any other as a DW Sampler of the file, at FL's root key `root`
/// (ours plays an audio file at 48) and a one-shot's release.
pub(super) fn load_sample(
    song: &mut Song,
    index: usize,
    shorthand: &ContentPath,
    path: &Path,
    root: u8,
    files: &ProjectFiles,
) {
    let channel = &mut song.channels[index];
    let slot = || {
        let mut slot = SamplerSlot {
            name: path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            // The pad shows its folder's last part: the sample's own name.
            folder: path.with_extension("").to_string_lossy().replace('\\', "/"),
            ..SamplerSlot::default()
        };
        slot.file.path = Some(shorthand.clone());
        slot
    };
    if channel.kind == ChannelKind::AudioClip {
        for clip in channel
            .lanes
            .iter_mut()
            .filter(|l| l.kind == LaneKind::Audio)
            .flat_map(|l| &mut l.clips)
        {
            clip.file.sample = Some(Box::new(slot()));
        }
        return;
    }
    if channel.kind == ChannelKind::Drums {
        let mut kit = DrumKit::default();
        kit.insert_slot(0, slot());
        channel.drum_kit = Some(kit);
        for event in channel
            .lanes
            .iter_mut()
            .flat_map(|l| l.clips.iter_mut().flat_map(|c| &mut c.events))
        {
            if let Event::Drum { pad, .. } = event {
                *pad = 0;
            }
        }
        return;
    }
    channel.kind = ChannelKind::Synth(SynthKind::DwSampler);
    let module = gm::instrument(&mut channel.rack, SynthKind::DwSampler);
    pick_dw_program(module, shorthand, None);
    files.name_module(module, path);
    // trunc(48v − 24) semitones: 48 − root.
    let shift = (48 - i32::from(root)).clamp(-24, 24);
    module.params[3] = ((shift + 24) as f32 + 0.5) / 48.0;
    module.params[2] = ONE_SHOT_RELEASE;
    load_dw_program(module, &reader(files));
}

/// A file named `name` (any case) in `dir` or below, at most [`SEARCH_DEPTH`] deep and [`SEARCH_ENTRIES`] entries;
/// links are not followed.
#[cfg(not(target_arch = "wasm32"))]
fn search(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![(dir.to_owned(), 0)];
    let mut seen = 0;
    while let Some((dir, depth)) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            seen += 1;
            if seen > SEARCH_ENTRIES {
                return None;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_file()
                && entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(name)
            {
                return Some(entry.path());
            }
            if kind.is_dir() && depth < SEARCH_DEPTH {
                stack.push((entry.path(), depth + 1));
            }
        }
    }
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn read(path: &Path) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    super::read_limited(file, super::MAX_FILE_BYTES).ok()
}

/// The desktop's `Locate samples…`: a folder dialog, then each file named one of `names` in it or below.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub(super) fn locate_native(names: &[String]) -> Option<FileEvent> {
    let dir = rfd::FileDialog::new().pick_folder()?;
    let found = (names.iter())
        .filter_map(|name| Some((name.clone(), read(&search(&dir, name)?)?)))
        .collect();
    Some(FileEvent::Located(found))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flp::tests::{ev, flp, note, utf16};
    use crate::flp::{Value, id};
    use crate::project_files::mem::MemFiles;
    use crate::ui::views::Overlay;

    /// A Fruity Wrapper state naming the plugin `name` (record 54).
    fn wrapper(name: &str) -> Value {
        let mut state = 10u32.to_le_bytes().to_vec();
        state.extend(54u32.to_le_bytes());
        state.extend((name.len() as u64).to_le_bytes());
        state.extend(name.as_bytes());
        Value::var(state)
    }

    #[test]
    fn a_zipped_project_finds_its_samples_and_sums_up_every_stand_in_once() {
        let project = flp(
            96,
            vec![
                ev(id::VERSION, Value::var(b"24.2.2.4597\0".to_vec())),
                ev(id::PAT_NEW, Value::U16(1)),
                ev(
                    id::PAT_NOTES,
                    Value::var(
                        [
                            note(0, 0, 24, 60, 100),
                            note(0, 1, 96, 60, 100),
                            note(0, 2, 96, 64, 100),
                        ]
                        .concat(),
                    ),
                ),
                ev(id::CH_NEW, Value::U16(0)),
                ev(id::CH_TYPE, Value::U8(0)),
                ev(id::DISPLAY_NAME, utf16("Kick")),
                ev(
                    id::SAMPLE_PATH,
                    utf16(r"%FLStudioFactoryData%\Data\Patches\Packs\Drums\Kicks\FPC_Kick_01.wav"),
                ),
                ev(id::CH_NEW, Value::U16(1)),
                ev(id::CH_TYPE, Value::U8(0)),
                ev(id::DISPLAY_NAME, utf16("Vox")),
                ev(id::SAMPLE_PATH, utf16(r"C:\Users\me\Samples\vox.wav")),
                ev(id::CH_NEW, Value::U16(2)),
                ev(id::CH_TYPE, Value::U8(2)),
                ev(id::PLUGIN_NAME, utf16("Fruity Wrapper")),
                ev(id::PLUGIN_STATE, wrapper("Serum")),
                ev(id::CH_TARGET, Value::U16(1)),
                ev(id::MX_FLAGS, Value::var(vec![0; 12])),
                ev(id::MX_OUTPUT, Value::U32(0)),
                ev(id::MX_FLAGS, Value::var(vec![0; 12])),
                ev(id::PLUGIN_NAME, utf16("Fruity Limiter")),
                ev(id::MX_SLOT, Value::U16(0)),
                ev(id::PLUGIN_NAME, utf16("Gross Beat")),
                ev(id::MX_SLOT, Value::U16(1)),
                ev(id::MX_OUTPUT, Value::U32(0)),
            ],
        );
        let wav = honeydew_audio::wav_writer::float32_wav(44_100, &[[0.5f32; 64]]);
        let zip = crate::zip::write(&[
            ("Beat.flp".into(), project),
            ("samples/vox.wav".into(), wav.clone()),
        ])
        .unwrap();
        let mut files =
            ProjectFiles::with_provider(Box::new(MemFiles::with_file("Beat.zip", &zip)));
        files.open();
        let song = files.poll(&mut Song::default()).expect("the project opens");
        // Untitled, titled after the archive.
        assert_eq!((song.title.as_str(), files.path.as_deref()), ("Beat", None));
        // The archive's vox.wav is kept with the project's files and plays in its channel.
        let kept = Path::new(UNZIPPED_FOLDER).join("Beat/vox.wav");
        assert_eq!(files.read_user_file(&kept).unwrap(), wav);
        assert_eq!(
            song.channels[2].kind,
            ChannelKind::Synth(SynthKind::DwSampler)
        );
        // The kick, whose FL path cannot resolve, plays our kit's; Serum, our closest instrument.
        assert_eq!(song.channels[1].kind, ChannelKind::Drums);
        assert_eq!(
            song.channels[3].kind,
            ChannelKind::Synth(SynthKind::MiniSynth)
        );
        let Some(Overlay::Message {
            tag,
            lines,
            confirm,
            ..
        }) = files.take_overlay()
        else {
            panic!("one summary");
        };
        assert_eq!(
            (tag, confirm),
            (LOCATE_TAG, Some((LOCATE.to_owned(), "OK".to_owned())))
        );
        assert_eq!(
            lines,
            [
                crate::project_files::STAND_INS,
                r"Kick: missing sample ...\Kicks\FPC_Kick_01.wav, plays Relay Kit Kick",
                "Serum: Serum replaced by Neon Ribbon",
                "Serum: Fruity Limiter as Limiter",
                "Serum: Gross Beat dropped",
            ]
        );
        assert_eq!(files.missing_names(), ["FPC_Kick_01.wav"]);
    }
}

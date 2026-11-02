//! The files recordings write: one per take, in the recordings folder
//! ([`crate::project_files::ProjectFiles::recordings_dir`]), named `<song title> Rec.wav`, then
//! `<song title> Rec 1.wav`, ` Rec 2` …, 16-bit PCM at the input's rate streamed through [`WavWriter`]. Native only: on
//! The web every create fails.

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};

use honeydew_audio::wav_writer::WavWriter;

use crate::ui::main_menu::library::FORBIDDEN;

/// A take's file while it is written. Until [`WavWriter::finish`] its header's sizes say "to the end of the file", so
/// a recording cut short by a crash still loads.
pub type TakeWriter = WavWriter<BufWriter<File>>;

/// Creates the next free take file of the song titled `title` in the recordings folder `dir`, `rate` Hz with
/// `channels` (1, or 2 for an `Inputs N + M` pair), and returns its path and its writer, the header written. The
/// file is created new, so no file is ever overwritten.
pub fn create_take(
    dir: &Path,
    title: &str,
    rate: u32,
    channels: u16,
) -> io::Result<(PathBuf, TakeWriter)> {
    let base = base_name(title);
    let mut n = 0;
    loop {
        let name = match n {
            0 => format!("{base}.wav"),
            n => format!("{base} {n}.wav"),
        };
        let path = dir.join(name);
        match File::create_new(&path) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
            file => return Ok((path, WavWriter::new(BufWriter::new(file?), rate, channels)?)),
        }
    }
}

/// `<title> Rec`, the title made a file name: `/\:*?"<>|` and control characters become `_`, leading dots and spaces
/// go (no hidden file), and at most 200 bytes of it stay, as a 255-byte title plus ` Rec 12.wav` would pass the usual
/// 255-byte name limit. An untitled song's is `Rec`.
fn base_name(title: &str) -> String {
    let named = |c: char| match FORBIDDEN.contains(c) || c.is_control() {
        true => '_',
        false => c,
    };
    let named: String = title.chars().map(named).collect();
    let mut named = named.trim_start_matches(['.', ' ']).to_owned();
    let ends = named.char_indices().map(|(at, c)| at + c.len_utf8());
    named.truncate(ends.take_while(|&end| end <= 200).last().unwrap_or(0));
    format!("{named} Rec").trim_start().to_owned()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// An empty folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("honeydew-h11-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn name(path: &Path) -> &str {
        path.file_name().unwrap().to_str().unwrap()
    }

    #[test]
    fn names_count_up_past_the_files_already_there() {
        let dir = folder("names");
        std::fs::write(dir.join("Tune Rec.wav"), b"old").unwrap();
        std::fs::write(dir.join("Tune Rec 1.wav"), b"old").unwrap();
        std::fs::write(dir.join("Other Rec.wav"), b"old").unwrap();
        let made: Vec<_> = (0..3)
            .map(|_| create_take(&dir, "Tune", 44_100, 1).unwrap())
            .collect();
        let names: Vec<_> = made.iter().map(|(path, _)| name(path)).collect();
        assert_eq!(
            names,
            ["Tune Rec 2.wav", "Tune Rec 3.wav", "Tune Rec 4.wav"]
        );
        // The files already there are untouched; a new song's first take takes the plain name.
        assert_eq!(std::fs::read(dir.join("Tune Rec 1.wav")).unwrap(), b"old");
        let (path, _) = create_take(&dir, "Fresh", 44_100, 2).unwrap();
        assert_eq!(name(&path), "Fresh Rec.wav");
        drop(made);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_take_file_holds_the_header_at_once_and_its_sizes_at_finish() {
        let dir = folder("header");
        let (path, mut writer) = create_take(&dir, "Song", 48_000, 2).unwrap();
        writer.write(&[0.5, -0.5, 0.25, 0.0]).unwrap();
        writer.finish().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 46 + 8);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 38 + 8);
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            48_000
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn titles_become_file_names() {
        assert_eq!(base_name("My Song"), "My Song Rec");
        assert_eq!(
            base_name("a/b\\c:d*e?f\"g<h>i|j\tk"),
            "a_b_c_d_e_f_g_h_i_j_k Rec"
        );
        assert_eq!(base_name(""), "Rec");
        assert_eq!(base_name(" ..hidden"), "hidden Rec");
        // At most 200 bytes of the title, cut on a character boundary.
        let long = "é".repeat(150);
        assert_eq!(base_name(&long), format!("{} Rec", "é".repeat(100)));
    }
}

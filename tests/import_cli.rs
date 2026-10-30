//! `honeydew-import` on generated fake archives, into a temp `--to`: no real archive is read.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A content container of `entries`, all stored: as the importer's own tests generate.
fn container(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let dir: usize = entries.iter().map(|(name, _)| 14 + name.len() + 1).sum();
    let count = (entries.len() as u32).to_be_bytes();
    let mut obb = [&3u32.to_be_bytes()[..], &[0; 20], &count].concat();
    let mut at = 28 + dir;
    for (name, data) in entries {
        obb.extend((name.len() as u32 + 1).to_be_bytes());
        obb.extend(name.as_bytes());
        obb.push(0);
        obb.extend((at as u32).to_be_bytes());
        obb.extend((data.len() as u32).to_be_bytes());
        obb.extend([0, 0]);
        at += data.len();
    }
    for (_, data) in entries {
        obb.extend(*data);
    }
    obb
}

/// This run's home for the tool, a folder that is not there: its user folder is `Documents/Honeydew` in it.
fn nowhere() -> PathBuf {
    std::env::temp_dir().join(format!(
        "honeydew-import-cli-nowhere-{}",
        std::process::id()
    ))
}

/// Runs the tool with `args`, every user-folder variable pointing at [`nowhere`]: it never sees the developer's own.
fn import(args: &[&Path]) -> (Output, String) {
    let nowhere = nowhere();
    let out = Command::new(env!("CARGO_BIN_EXE_honeydew-import"))
        .args(args)
        .env("HOME", &nowhere)
        .env("XDG_DATA_HOME", &nowhere)
        .env("XDG_CONFIG_HOME", &nowhere)
        .env("APPDATA", &nowhere)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    (out, stdout)
}

#[test]
fn the_tool_imports_an_obb_and_an_apk_into_the_folder_it_is_given() {
    let tmp = std::env::temp_dir().join(format!("honeydew-import-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let obb = tmp.join("Gen.obb");
    std::fs::write(&obb, container(&[("Drum Samples/Gen Kick.wav", b"kick")])).unwrap();
    let inner = container(&[
        ("Gen Root.data", b"root"),
        ("Drum Samples/Gen Kick.wav", b"x"),
    ]);
    let apk = honeydew::zip::write(&[("assets/obb/assets.obb".into(), inner)]).unwrap();
    let apk_path = tmp.join("Gen.apk");
    std::fs::write(&apk_path, apk).unwrap();
    // A folder that is not there yet.
    let to = tmp.join("User").join("Original Content");
    let to_flag = Path::new("--to");

    let (out, stdout) = import(&[&obb, &apk_path, to_flag, &to]);
    assert!(out.status.success(), "{stdout}");
    assert_eq!(stdout.matches("not affiliated").count(), 1, "{stdout}");
    assert!(stdout.contains("Gen.obb: 1 written, 0 skipped"), "{stdout}");
    assert!(stdout.contains("Gen.apk: 1 written, 1 skipped"), "{stdout}");
    let read = |name: &str| std::fs::read(to.join(name)).unwrap();
    assert_eq!(read("Drum Samples/Gen Kick.wav"), b"kick");
    assert_eq!(read("Gen Root.data"), b"root");
    // Each archive's marker is the app's.
    let names = std::fs::read_dir(&to).unwrap().flatten();
    let names: Vec<_> = names
        .map(|e| e.file_name().into_string().unwrap())
        .collect();
    for archive in ["Gen.obb", "Gen.apk"] {
        let marker = format!(".imported-{archive}-");
        assert!(names.iter().any(|n| n.starts_with(&marker)), "{names:?}");
    }

    // Again: nothing is written over.
    let (out, stdout) = import(&[to_flag, &to, &obb]);
    assert!(out.status.success());
    assert!(stdout.contains("Gen.obb: 0 written, 1 skipped"), "{stdout}");
    // What is no archive fails, with its reason; no archive at all prints the usage.
    let (out, _) = import(&[&tmp.join("Gen.txt"), to_flag, &to]);
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stderr).is_empty());
    // A `--to` in the user folder (`~/Documents/Honeydew` here, with no `user-dirs.dirs`) but outside its Original
    // Content folder is refused, nothing written.
    if cfg!(all(unix, not(target_os = "macos"))) {
        let samples = nowhere().join("Documents/Honeydew/My Samples");
        let (out, _) = import(&[&obb, to_flag, &samples]);
        assert!(!out.status.success());
        assert!(!samples.exists());
    }
    let (out, _) = import(&[]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Usage"));
    std::fs::remove_dir_all(&tmp).unwrap();
    let _ = std::fs::remove_dir_all(nowhere());
}

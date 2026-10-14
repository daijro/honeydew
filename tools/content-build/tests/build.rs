use std::fs;
use std::path::{Path, PathBuf};

use content_build::build::{Inputs, Opts, build};
use content_build::verify::verify_capped;
use honeydew::content::bundle::Index;

/// A one-source content dir and source tree under the test's own temp dir: two FLAC one-shots, one `samples` recipe.
fn fixture(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("content/recipes/samples")).unwrap();
    fs::create_dir_all(root.join("sources/fx")).unwrap();
    fs::write(
        root.join("content/sources.json"),
        r#"[{"folder": "fx", "title": "FX", "page": "https://example.org/fx", "author": "A", "spdx": "CC0-1.0"}]"#,
    )
    .unwrap();
    fs::write(
        root.join("content/recipes/samples/kick.toml"),
        "id = \"samples-kick\"\nkind = \"samples\"\npath = \"Samples/Kick/\"\nsource = \"fx\"\n\
         [[sample]]\nname = \"Tight Kick\"\nfile = \"b.flac\"\n[[sample]]\nname = \"Deep Kick\"\nfile = \"a.flac\"\n",
    )
    .unwrap();
    for (file, hz) in [("a.flac", 55.0), ("b.flac", 80.0)] {
        let s: Vec<f32> = (0..4410)
            .map(|i| (i as f32 * hz * std::f32::consts::TAU / 44_100.0).sin() * 0.5)
            .collect();
        fs::write(
            root.join("sources/fx").join(file),
            honeydew_audio::flac_writer::flac(44_100, 16, &[s]),
        )
        .unwrap();
    }
    root
}

fn opts(root: &Path, out: &str, update_lock: bool) -> Opts {
    Opts {
        content: root.join("content"),
        sources: root.join("sources"),
        out: root.join(out),
        only: Vec::new(),
        update_lock,
        release: "test".into(),
    }
}

fn files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let path = e.unwrap().path();
        if path.is_dir() {
            out.extend(
                files(&path)
                    .into_iter()
                    .map(|(p, b)| (Path::new(path.file_name().unwrap()).join(p), b)),
            );
        } else {
            out.push((path.file_name().unwrap().into(), fs::read(&path).unwrap()));
        }
    }
    out.sort();
    out
}

#[test]
fn a_double_build_is_byte_identical_and_its_index_round_trips_through_the_bundle_types() {
    let root = fixture("double-build");
    // `--update-lock` on a full build drops a line no recipe reads any more.
    let lock = root.join("content/inputs.lock");
    fs::write(&lock, format!("{}  fx/gone.flac\n", "0".repeat(64))).unwrap();
    build(&opts(&root, "out1", true)).unwrap();
    let text = fs::read_to_string(&lock).unwrap();
    assert!(
        !text.contains("gone.flac") && text.lines().count() == 2,
        "{text}"
    );
    // The second build reads the lock the first one wrote: every input must match it.
    build(&opts(&root, "out2", false)).unwrap();
    let (one, two) = (files(&root.join("out1")), files(&root.join("out2")));
    assert_eq!(
        one.iter().map(|f| &f.0).collect::<Vec<_>>(),
        two.iter().map(|f| &f.0).collect::<Vec<_>>()
    );
    assert!(one == two, "the two builds differ");

    let raw = fs::read(root.join("out1/index.json")).unwrap();
    let index: Index = serde_json::from_slice(&raw).unwrap();
    assert_eq!(serde_json::to_vec(&index).unwrap(), raw);
    let e = &index.entries[0];
    let names: Vec<&str> = e.members.iter().map(|m| m.0.as_str()).collect();
    assert_eq!(names, ["Deep Kick.flac", "Tight Kick.flac"]);
    assert_eq!(
        (e.credits.as_slice(), e.decoded),
        (&["fx".to_owned()][..], 2 * 4410 * 4)
    );
    let part = fs::read(root.join("out1").join(&e.parts[0].file)).unwrap();
    let m = &e.members[0];
    assert_eq!(
        &part[m.2 as usize..(m.2 + m.3) as usize],
        fs::read(root.join("sources/fx/a.flac")).unwrap()
    );

    // An input that no longer matches the lock fails the build.
    fs::write(root.join("sources/fx/a.flac"), &part[..100]).unwrap();
    let err = build(&opts(&root, "out3", false)).unwrap_err();
    assert!(
        err.contains("fx/a.flac: changed since inputs.lock"),
        "{err}"
    );
}

#[test]
fn verify_catches_an_oversize_part_and_an_unaliased_removed_path() {
    let root = fixture("verify-oversize");
    build(&opts(&root, "out", true)).unwrap();
    let err = verify_capped(&root.join("out"), None, 1024).unwrap_err();
    assert!(err.contains("over the 1024-byte cap"), "{err}");

    // A previous release listed a sample this one neither keeps nor aliases.
    let mut old: Index =
        serde_json::from_slice(&fs::read(root.join("out/index.json")).unwrap()).unwrap();
    old.entries[0].members[0].0 = "Old Kick.flac".into();
    let prev = root.join("previous.json");
    fs::write(&prev, serde_json::to_vec(&old).unwrap()).unwrap();
    let err = verify_capped(&root.join("out"), Some(&prev), 24_000_000).unwrap_err();
    assert_eq!(
        err,
        "Samples/Kick/Old Kick.flac: in the previous index but neither kept nor aliased"
    );
}

#[cfg(unix)]
#[test]
fn an_input_symlinked_out_of_its_source_folder_is_refused() {
    let root = fixture("symlink-escape");
    fs::copy(root.join("sources/fx/a.flac"), root.join("outside.flac")).unwrap();
    std::os::unix::fs::symlink(root.join("outside.flac"), root.join("sources/fx/link.flac"))
        .unwrap();
    let mut inputs = Inputs::new(&root.join("sources"), "", true);
    assert!(inputs.read("fx", "a.flac").is_ok());
    let err = inputs.read("fx", "link.flac").unwrap_err();
    assert!(err.contains("outside its source folder"), "{err}");
}

/// The program stage on a tiny SFZ, two keys × two layers with the sources' loops, read back through `dwp::read`.
#[test]
fn a_program_maps_keys_and_layers_and_keeps_its_sources_loops_at_44_1_khz() {
    use content_build::{build::Inputs, recipe::Recipe, stages};
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("program-stage");
    let _ = fs::remove_dir_all(&root);
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    // C4 at 44.1 kHz, period 100; C5 at 48 kHz, period 96 (88.2 at 44.1). Each loop's inclusive `loop_end` repeats
    // its `loop_start` frame, a zero crossing, as the organ's do: the exclusive end is `loop_end`, scaled.
    for (name, rate, period, amp) in [
        ("c4p", 44_100, 100.0, 0.25),
        ("c4f", 44_100, 100.0, 0.5),
        ("c5p", 48_000, 96.0, 0.125),
        ("c5f", 48_000, 96.0, 0.25),
    ] {
        let x: Vec<f32> = (0..rate / 10)
            .map(|i| amp * (std::f64::consts::TAU * f64::from(i) / period).sin() as f32)
            .collect();
        let flac = honeydew_audio::flac_writer::flac(rate, 16, &[x]);
        fs::write(src.join(format!("{name}.flac")), flac).unwrap();
    }
    fs::write(
        src.join("t.sfz"),
        "<global> loop_mode=loop_continuous\n\
         <group> lokey=55 hikey=65 pitch_keycenter=60 loop_start=1000 loop_end=1100\n\
         <region> sample=c4p.flac hivel=63 <region> sample=c4f.flac lovel=64\n\
         <group> lokey=66 hikey=80 pitch_keycenter=72 loop_start=960 loop_end=1056\n\
         <region> sample=c5p.flac hivel=63 <region> sample=c5f.flac lovel=64\n",
    )
    .unwrap();
    let text = "id = \"t\"\nkind = \"program\"\npath = \"Presets/DW Sampler/Test/T.dwp\"\nsource = \"src\"\n\
                sfz = \"t.sfz\"\ngain = \"normalize\"\n";
    let recipe: Recipe = toml::from_str(text).unwrap();
    let (members, lite) = stages::program(&recipe, &mut Inputs::new(&root, "", true)).unwrap();
    let names: Vec<&str> = members.iter().map(|m| m.0.as_str()).collect();
    // The program, then the layer at velocity 90 (the loud one), then the soft one.
    let order = [
        "T.dwp",
        "T/C4 v2.flac",
        "T/C5 v2.flac",
        "T/C4 v1.flac",
        "T/C5 v1.flac",
    ];
    assert_eq!((names, lite), (order.to_vec(), 0));
    let zones: Vec<_> = honeydew_audio::dwp::read(&members[0].1)
        .unwrap()
        .into_iter()
        .map(|d| {
            let z = d.zone;
            let at = (z.root, z.key_low, z.key_high, z.vel_low, z.vel_high);
            (at, d.file.unwrap(), (z.loop_mode, z.loop_start, z.loop_end))
        })
        .collect();
    let zone = |at, file: &str, looped| (at, file.to_owned(), looped);
    assert_eq!(
        zones,
        [
            zone((60, 55, 66, 0, 63), "C4 v1.flac", (2, 1000, 1100)),
            zone((60, 55, 66, 64, 127), "C4 v2.flac", (2, 1000, 1100)),
            zone((72, 67, 80, 0, 63), "C5 v1.flac", (2, 882, 970)),
            zone((72, 67, 80, 64, 127), "C5 v2.flac", (2, 882, 970)),
        ]
    );
    // The loudest sample peaks at −6 dBFS, and is cut at its loop's end.
    let loud = honeydew_audio::sample::decode(&members[1].1).unwrap();
    let peak = loud.left.iter().fold(0f32, |p, x| p.max(x.abs()));
    assert!((peak - 10f32.powf(-6.0 / 20.0)).abs() < 2e-3, "{peak}");
    assert_eq!(loud.left.len(), 1100);

    // `even`: C5, 6 dB under C4 in the source, comes out level with it, and each key's layers keep their ratio.
    let even: Recipe = toml::from_str(&text.replace("normalize", "even")).unwrap();
    let (members, _) = stages::program(&even, &mut Inputs::new(&root, "", true)).unwrap();
    let peak = |name: &str| {
        let m = members.iter().find(|m| m.0 == name).unwrap();
        let d = honeydew_audio::sample::decode(&m.1).unwrap();
        d.left.iter().fold(0f32, |p, x| p.max(x.abs()))
    };
    let (c4, c5) = (peak("T/C4 v2.flac"), peak("T/C5 v2.flac"));
    assert!(
        (c5 - c4).abs() < 2e-3 && (peak("T/C5 v1.flac") - c5 / 2.0).abs() < 2e-3,
        "{c4} {c5}"
    );
}

/// A tree `inst/` of short sines, `bad` files garbage (reading one fails the build), and the `.dwp` zones
/// (root, lokey, hikey, lovel, hivel) and files of a program recipe ending in `tail` over it.
fn tree_program(name: &str, good: &[&str], bad: &[&str], tail: &str) -> Vec<([u8; 5], String)> {
    use content_build::{build::Inputs, recipe::Recipe, stages};
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&root);
    let inst = root.join("src/inst");
    fs::create_dir_all(&inst).unwrap();
    let sine: Vec<f32> = (0..2205).map(|i| 0.5 * (i as f32 * 0.06).sin()).collect();
    for f in good {
        let flac = honeydew_audio::flac_writer::flac(44_100, 16, std::slice::from_ref(&sine));
        fs::write(inst.join(format!("{f}.flac")), flac).unwrap();
    }
    for f in bad {
        fs::write(inst.join(format!("{f}.flac")), b"not audio").unwrap();
    }
    let recipe: Recipe = toml::from_str(&format!(
        "id = \"t\"\nkind = \"program\"\npath = \"Presets/DW Sampler/Test/T.dwp\"\nsource = \"src\"\n{tail}"
    ))
    .unwrap();
    let (members, _) = stages::program(&recipe, &mut Inputs::new(&root, "", true)).unwrap();
    (honeydew_audio::dwp::read(&members[0].1)
        .unwrap()
        .into_iter())
    .map(|d| {
        let z = d.zone;
        let at = [z.root, z.key_low, z.key_high, z.vel_low, z.vel_high];
        (at, d.file.unwrap())
    })
    .collect()
}

/// A `files` pattern: notes (transposed) and layers from the names (in `layers` order, others skipped), round robin 1
/// else the lowest one, and the keys between and beyond the sampled notes filled.
#[test]
fn a_files_pattern_maps_notes_layers_and_round_robins_and_fills_the_key_gaps() {
    let good = [
        "X_C4_p_rr1",
        "X_C4_f_rr1",
        "X_E4_f_rr2",
        "X_G4_p_rr1",
        "X_G4_f_rr1",
    ];
    let bad = ["X_C4_f_rr2", "X_E4_f_rr3", "X_G4_mf_rr1", "Y_C4_p_rr1"];
    let zones = tree_program(
        "files-pattern",
        &good,
        &bad,
        "files = \"inst/X_{note}_{layer}_rr{rr}.flac\"\nlayers = [\"p\", \"f\"]\ntranspose = 12\n",
    );
    let zone = |at, file: &str| (at, file.to_owned());
    assert_eq!(
        zones,
        [
            zone([72, 0, 74, 0, 63], "C5 v1.flac"),
            zone([72, 0, 74, 64, 127], "C5 v2.flac"),
            zone([76, 75, 77, 0, 127], "E5 v1.flac"),
            zone([79, 78, 127, 0, 63], "G5 v1.flac"),
            zone([79, 78, 127, 64, 127], "G5 v2.flac"),
        ]
    );
}

/// `[[zone]]`s play where they are written: no gap between them is filled.
#[test]
fn explicit_zones_keep_their_keys_and_velocities() {
    let zones = tree_program(
        "explicit-zones",
        &["a", "b", "c"],
        &[],
        "[[zone]]\nfile = \"inst/a.flac\"\nroot = 41\nlokey = 36\nhikey = 43\nhivel = 63\n\
         [[zone]]\nfile = \"inst/b.flac\"\nroot = 41\nlokey = 36\nhikey = 43\nlovel = 64\n\
         [[zone]]\nfile = \"inst/c.flac\"\nroot = 50\n",
    );
    let zone = |at, file: &str| (at, file.to_owned());
    assert_eq!(
        zones,
        [
            zone([41, 36, 43, 0, 63], "F2 v1.flac"),
            zone([41, 36, 43, 64, 127], "F2 v2.flac"),
            zone([50, 50, 50, 0, 127], "D3 v1.flac"),
        ]
    );
}

/// The samples stage on a WAV one-shot: a stereo pair with L = R folds to mono, its silent tail is cut and faded,
/// and it peaks at −1 dBFS; a FLAC in a recipe without `tail` or `channels` still passes through as it is.
#[test]
fn a_samples_recipe_with_a_tail_decodes_folds_trims_and_peaks_its_one_shots() {
    use content_build::{build::Inputs, recipe::Recipe, stages};
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("samples-stage");
    let _ = fs::remove_dir_all(&root);
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    // 0.1 s of a decaying 440 Hz tone at 0.25, then 0.9 s of silence.
    let x: Vec<f32> = (0..44_100)
        .map(|i| {
            let t = i as f32 / 44_100.0;
            if t < 0.1 {
                0.25 * (-30.0 * t).exp() * (std::f32::consts::TAU * 440.0 * t).sin()
            } else {
                0.0
            }
        })
        .collect();
    let wav = honeydew_audio::wav_writer::float32_wav(44_100, &[x.clone(), x]);
    fs::write(src.join("hit.wav"), wav).unwrap();
    let recipe: Recipe = toml::from_str(
        "id = \"t\"\nkind = \"samples\"\npath = \"Samples/T/\"\nsource = \"src\"\n\
         tail = { floor_db = -60, fade_ms = 5 }\n[[sample]]\nname = \"Hit\"\nfile = \"hit.wav\"\n",
    )
    .unwrap();
    let (members, _) = stages::samples(&recipe, &mut Inputs::new(&root, "", true)).unwrap();
    assert_eq!(members[0].0, "Hit.flac");
    let d = honeydew_audio::sample::decode(&members[0].1).unwrap();
    assert!(d.right.is_none(), "L = R folds to mono");
    let peak = d.left.iter().fold(0f32, |p, x| p.max(x.abs()));
    assert!((peak - 10f32.powf(-1.0 / 20.0)).abs() < 1e-3, "{peak}");
    assert!(d.left.len() < 4_410 + 300, "{}", d.left.len());
    assert!(d.left.last().unwrap().abs() < 1e-4);
}

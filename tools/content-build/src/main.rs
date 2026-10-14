use std::path::PathBuf;
use std::process::ExitCode;

use content_build::{build, gates, recipe, verify};

const USAGE: &str = "usage:
  content-build check  [--content content]
  content-build build  --sources <content-sources> --out <dir> [--content content] [--only <id>]... [--update-lock] [--release <ver>]
  content-build verify --out <dir> [--previous <old index.json>]";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let (cmd, rest) = args.split_first().ok_or(USAGE)?;
    let mut flags: Vec<(String, Option<String>)> = Vec::new();
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--update-lock" => flags.push((a.clone(), None)),
            f if f.starts_with("--") => {
                flags.push((a.clone(), Some(it.next().ok_or(USAGE)?.clone())))
            }
            _ => return Err(USAGE.into()),
        }
    }
    let get = |name: &str| {
        flags
            .iter()
            .rev()
            .find(|(f, _)| f == name)
            .and_then(|(_, v)| v.clone())
    };
    let path = |name: &str| {
        get(name)
            .map(PathBuf::from)
            .ok_or_else(|| format!("{name} is required\n{USAGE}"))
    };
    let content = get("--content").map_or_else(|| PathBuf::from("content"), PathBuf::from);
    match cmd.as_str() {
        "check" => gates::check(&recipe::recipes(&content)?, &recipe::sources(&content)?),
        "build" => build::build(&build::Opts {
            content,
            sources: path("--sources")?,
            out: path("--out")?,
            only: flags
                .iter()
                .filter(|(f, _)| f == "--only")
                .filter_map(|(_, v)| v.clone())
                .collect(),
            update_lock: flags.iter().any(|(f, _)| f == "--update-lock"),
            release: get("--release").unwrap_or_else(|| "dev".into()),
        }),
        "verify" => verify::verify(
            &path("--out")?,
            get("--previous").map(PathBuf::from).as_deref(),
        ),
        _ => Err(USAGE.into()),
    }
}

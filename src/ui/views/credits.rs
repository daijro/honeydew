//! The `Sound credits` screen (SETTINGS' `Sound credits`): the app's own credits, then the sources
//! of the bundled sounds (`index.json`'s `credits`) grouped by licence, each with its title,
//! author, link and note, then the full notices of the content dir's `CREDITS.txt` (on the web, downloaded when the
//! screen first shows, `Loading...` until it is in: [`web::notices`]). It is a message box
//! ([`message::show`]): its lines word-wrap, scroll past the screen's height, and `OK`, Escape or a click outside
//! close it.

use std::collections::BTreeMap;

use eframe::egui;

use super::{AppCtx, Nav, message};
use crate::APP_NAME;
use crate::content::bundle::Credit;
use crate::content::{Bundle, web};

/// The logo's credit, as the README gives it.
pub const LOGO: &str = "The icon is a derivative of \"Melon\" by afitrose. It is licensed as Free for commercial use \
                        WITH ATTRIBUTION.";
/// The repo's notices for the third-party code the app includes.
const THIRD_PARTY: &str = "Third-party software notices: THIRD_PARTY_NOTICES.md";
/// The bundled sounds' section, and its only line without a bundle.
const SOUNDS: &str = "Bundled sounds:";
pub const NO_BUNDLE: &str = "No bundled sounds installed";
const NOTICES: &str = "Full notices:";
const NO_SOURCES: &str = "No sources listed";
const NO_NOTICES: &str = "The full notices (CREDITS.txt) are unavailable.";
/// The notices' line while the web downloads them.
const LOADING: &str = "Loading...";
const TAG: &str = "credits";

/// What the screen keeps: its lines, made when it shows and dropped when it closes, and whether they wait for the web's
/// notices.
#[derive(Default)]
pub struct State {
    lines: Option<Vec<String>>,
    loading: bool,
}

/// Shows the screen over everything; returns `Nav::Close` once it closes.
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx) -> Option<Nav> {
    let bundle = app.files.factory.bundle.as_deref();
    // The notices came in: the lines are made again.
    if state.loading && web::notices().is_some() {
        state.lines = None;
    }
    let loading = &mut state.loading;
    let lines = state.lines.get_or_insert_with(|| lines(bundle, loading));
    let nav = message::show(ui, app, TAG, lines, None, None);
    if nav.is_some() {
        state.lines = None;
    }
    nav
}

/// The screen's lines: a section of the app's own credits, then the bundled sounds' ([`NO_BUNDLE`] without a bundle).
/// `loading`: the notices are on their way.
fn lines(bundle: Option<&Bundle>, loading: &mut bool) -> Vec<String> {
    let mut lines: Vec<String> = [
        "Sound credits",
        "",
        &format!("{APP_NAME}:"),
        LOGO,
        THIRD_PARTY,
        "",
        SOUNDS,
    ]
    .map(str::to_owned)
    .into();
    let Some(bundle) = bundle else {
        lines.push(NO_BUNDLE.into());
        return lines;
    };
    let mut by_license: BTreeMap<&str, Vec<&Credit>> = BTreeMap::new();
    for credit in bundle.index.credits.values() {
        by_license.entry(&credit.license).or_default().push(credit);
    }
    if by_license.is_empty() {
        lines.extend([NO_SOURCES.into(), String::new()]);
    }
    for (license, credits) in by_license {
        lines.push(format!("License {license}:"));
        for c in credits {
            lines.push(format!("{} by {}", c.title, c.author));
            lines.extend(c.url.iter().chain(&c.note).cloned());
        }
        lines.push(String::new());
    }
    lines.push(NOTICES.into());
    // The web has no content dir: it downloads `content/CREDITS.txt`.
    let notices = bundle.notices().map(Some).or_else(web::notices);
    *loading = notices.is_none();
    lines.push(notices.map_or(LOADING.into(), |n| n.unwrap_or_else(|| NO_NOTICES.into())));
    lines
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use eframe::egui::{Context, RawInput, Rect, pos2};

    use super::*;
    use crate::audio_link::AudioLink;
    use crate::content::Factory;
    use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
    use crate::project_files::ProjectFiles;
    use crate::project_files::content::tests::Temp;
    use crate::project_files::mem::MemFiles;
    use crate::settings::Settings;
    use crate::song::Song;
    use crate::theme::theme_by_id;
    use crate::ui::UiCtx;
    use crate::ui::main_screen::Regions;
    use crate::ui::offscreen::Offscreen;

    const INDEX: &str = r#"{"schema": 1, "release": "test", "entries": [], "credits": {
        "salamander": {"title": "Salamander Grand Piano V3", "author": "Alexander Holm",
            "license": "LicenseRef-PD-declared", "url": "https://archive.org/details/SalamanderGrandPianoV3"},
        "musescore": {"title": "MuseScore_General", "author": "S. Christian Collins", "license": "MIT",
            "note": "Full notice in CREDITS.txt"},
        "honeydew": {"title": "Honeydew procedural sounds", "author": "The Honeydew contributors",
            "license": "CC0-1.0"}}}"#;
    const MIT: &str =
        "Permission is hereby granted, free of charge, to any person obtaining a copy";

    /// The texts of the screen over a factory with a fixture bundle in temp folder `name` (`None`: no bundle), on a
    /// screen tall enough to show every line.
    fn rendered(bundle: Option<&str>, name: &str) -> String {
        let tmp = Temp::new(name);
        let mut factory = Factory::new(Vec::new());
        if let Some(index) = bundle {
            std::fs::write(tmp.0.join("index.json"), index).unwrap();
            std::fs::write(
                tmp.0.join("CREDITS.txt"),
                format!("MuseScore_General\n\n{MIT}"),
            )
            .unwrap();
            factory = factory.with_bundle(Bundle::open(&tmp.0).unwrap());
        }
        let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        files.factory = Box::leak(Box::new(factory));
        let (theme, m) = (
            theme_by_id(0),
            Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX),
        );
        let screen = Rect::from_min_max(pos2(0.0, 0.0), pos2(1170.0, 3000.0));
        let (ctx, mut state, mut song, mut audio) = (
            Context::default(),
            State::default(),
            Song::default(),
            AudioLink::default(),
        );
        let (mut settings, mut selected) = (Settings::default(), 0);
        let mut shapes = Vec::new();
        // An area's first frame is egui's sizing pass.
        for time in [0.1, 0.2] {
            let raw = RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                ..Default::default()
            };
            shapes = ctx
                .run(raw, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut app = AppCtx {
                            cx: UiCtx {
                                theme: &theme,
                                m: &m,
                            },
                            regions: Regions::new(screen, &m),
                            song: &mut song,
                            audio: &mut audio,
                            selected: &mut selected,
                            snapshot: None,
                            playing: false,
                            armed: false,
                            previewing: false,
                            middle_c_is_c4: false,
                            settings: &mut settings,
                            ghost_notes: true,
                            files: &mut files,
                            edit: None,
                            answer: None,
                            covered: false,
                            playhead: None,
                            seek: None,
                            midi_devices: &[],
                            midi_rescan: false,
                        };
                        show(&mut state, ui, &mut app);
                    });
                })
                .shapes;
        }
        let texts: Vec<String> = Offscreen::texts(&shapes)
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        texts.join(" ")
    }

    #[test]
    fn the_screen_lists_each_source_with_its_licence_and_link_then_the_full_notices() {
        let text = rendered(Some(INDEX), "credits-listed");
        for want in [
            LOGO,
            "License MIT:",
            "MuseScore_General by S. Christian Collins",
            "Full notice in CREDITS.txt",
            "License LicenseRef-PD-declared:",
            "Salamander Grand Piano V3 by Alexander Holm",
            "https://archive.org/details/SalamanderGrandPianoV3",
            "Honeydew procedural sounds by The Honeydew contributors",
            MIT,
            "OK",
        ] {
            assert!(text.contains(want), "{want}: {text}");
        }
        assert!(!text.contains(NO_BUNDLE));
    }

    #[test]
    fn the_screen_never_names_the_original_and_without_a_bundle_shows_the_app_credits_alone() {
        let none = rendered(None, "credits-none");
        assert!(none.contains(LOGO) && none.contains(NO_BUNDLE), "{none}");
        for text in [none, rendered(Some(INDEX), "credits-names")] {
            for banned in ["Image-Line", "FL Studio", "FL "] {
                assert!(!text.contains(banned), "{banned}: {text}");
            }
        }
    }
}

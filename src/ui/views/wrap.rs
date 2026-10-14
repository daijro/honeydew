//! Word-wrap for the message box. The box measures its text with the popup's font and wraps to the screen less its
//! margins.

use std::mem::take;

const NBSP: char = '\u{a0}';

/// `text` as lines no wider than `max_w`, as `width` measures a string. Breaks fall between words, at whitespace
/// (a no-break space joins its neighbours); a word wider than `max_w` on its own breaks anywhere. `\n` starts a new
/// line (an empty one stays). Text that fits as it stands, with no `\n` in it, is returned untouched; otherwise each
/// line is rebuilt, so a run of whitespace becomes one space and a line's leading indentation is lost.
pub fn wrap(text: &str, max_w: f32, width: impl Fn(&str) -> f32) -> Vec<String> {
    if !text.contains('\n') && width(text) <= max_w {
        return vec![text.to_owned()];
    }
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph
            .split(|c: char| c.is_whitespace() && c != NBSP)
            .filter(|word| !word.is_empty())
        {
            let joined = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if width(&joined) <= max_w {
                line = joined;
                continue;
            }
            out.extend((!line.is_empty()).then(|| take(&mut line)));
            // The word alone; too wide even so, it breaks at the last character that fits (one at least), and the
            // tail goes on as the current line.
            for c in word.chars() {
                line.push(c);
                if width(&line) > max_w && line.chars().count() > 1 {
                    let c = line.pop().expect("a character was just pushed");
                    out.push(take(&mut line));
                    line.push(c);
                }
            }
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10 per character.
    fn w(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    #[test]
    fn text_that_fits_is_returned_as_it_is() {
        assert_eq!(wrap("a  b ", 100.0, w), ["a  b "]);
        assert_eq!(wrap("", 100.0, w), [""]);
    }

    #[test]
    fn lines_break_between_words() {
        assert_eq!(
            wrap("hello brave new world", 100.0, w),
            ["hello", "brave new", "world"]
        );
    }

    #[test]
    fn a_word_wider_than_the_line_breaks_anywhere_and_its_tail_takes_the_next_words() {
        assert_eq!(
            wrap("ab abcdefghijklmnopq cd", 50.0, w),
            ["ab", "abcde", "fghij", "klmno", "pq cd"]
        );
    }

    #[test]
    fn a_newline_starts_a_line_and_a_blank_one_stays() {
        assert_eq!(wrap("one\n\ntwo three", 100.0, w), ["one", "", "two three"]);
        assert_eq!(wrap("one\r\ntwo", 100.0, w), ["one", "two"]);
    }

    #[test]
    fn a_no_break_space_keeps_its_words_together() {
        assert_eq!(wrap("a\u{a0}b cdefgh", 70.0, w), ["a\u{a0}b", "cdefgh"]);
        assert_eq!(wrap("ab\tcd  ef", 20.0, w), ["ab", "cd", "ef"]);
    }

    #[test]
    fn a_line_narrower_than_a_character_still_ends() {
        assert_eq!(wrap("abc d", 5.0, w), ["a", "b", "c", "d"]);
    }

    #[test]
    fn every_line_fits_and_no_word_is_split_unless_it_is_too_wide() {
        let text = "Failed to save file: the folder is read-only or the disk is full, and /home/someone/Documents/My Songs/a-rather-long-song-name.flm could not be written. Check the free space and try again, or choose another folder in the Save dialog.";
        for chars in [3, 8, 12, 20, 33, 80, 400] {
            let max_w = chars as f32 * 10.0;
            let lines = wrap(text, max_w, w);
            assert!(lines.iter().all(|l| w(l) <= max_w), "{chars}: {lines:?}");
            let tokens: Vec<&str> = lines.iter().flat_map(|l| l.split(' ')).collect();
            // Nothing lost or reordered, and each word that fits a line alone came through whole.
            assert_eq!(tokens.concat(), text.split_whitespace().collect::<String>());
            for word in text.split_whitespace().filter(|word| w(word) <= max_w) {
                assert!(tokens.contains(&word), "{chars}: {word} in {lines:?}");
            }
        }
    }
}

/// The message box wraps (`message::show`): the text stays inside the box and the box inside the screen.
#[cfg(test)]
mod message_tests {
    use eframe::egui::{
        self, Color32, Context, Id, RawInput, Rect, Shape, Vec2, epaint::ClippedShape, pos2, vec2,
    };

    use crate::audio_link::AudioLink;
    use crate::metrics::{DEFAULT_UI_SCALE_INDEX, FontSize, Metrics};
    use crate::project_files::ProjectFiles;
    use crate::project_files::mem::MemFiles;
    use crate::settings::Settings;
    use crate::song::Song;
    use crate::theme::theme_by_id;
    use crate::ui::UiCtx;
    use crate::ui::anim;
    use crate::ui::main_screen::Regions;
    use crate::ui::popup::PAD_X;
    use crate::ui::views::{AppCtx, message};

    /// 300 characters of words, the longest 12.
    const LONG: &str = "Failed to save the song: the folder is read-only, or the disk is full, or the file is open in another application. Free some space or choose another folder in the Save dialog and try again; if the problem stays, export the song to a different folder. The system said: permission denied (os error 13).";

    /// The gap in U between the text column and the screen edges: `message::MARGIN`.
    const MARGIN: f32 = 1.0;

    /// One frame of `lines` with `OK` on a `size` window at `ppp`; `time` in frames.
    fn frame(ctx: &Context, lines: &[&str], size: Vec2, ppp: f32, time: u32) -> Vec<ClippedShape> {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), size);
        let theme = theme_by_id(0);
        let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
        let lines: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
        let mut raw = RawInput {
            screen_rect: Some(screen),
            time: Some(f64::from(time) / 60.0),
            ..Default::default()
        };
        raw.viewports
            .entry(raw.viewport_id)
            .or_default()
            .native_pixels_per_point = Some(ppp);
        let (mut song, mut audio, mut settings) =
            (Song::default(), AudioLink::default(), Settings::default());
        let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut app = AppCtx {
                    cx: UiCtx {
                        theme: &theme,
                        m: &m,
                    },
                    regions: Regions::new(screen, &m),
                    song: &mut song,
                    audio: &mut audio,
                    selected: &mut 1,
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
                message::show(ui, &mut app, "test", &lines, None, None);
            });
        })
        .shapes
    }

    fn new_ctx() -> Context {
        let ctx = Context::default();
        crate::fonts::install(&ctx);
        ctx
    }

    fn texts(shapes: &[ClippedShape]) -> Vec<(String, Rect)> {
        (shapes.iter())
            .filter_map(|c| match &c.shape {
                Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.galley.rect.translate(t.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect()
    }

    /// What one message left on the screen, a few frames in (the animation off: the box is open at once).
    struct Shown {
        ctx: Context,
        m: Metrics,
        screen: Rect,
        /// The text column: the popup area the items sit in.
        area: Rect,
        /// The painted `background` box, full width.
        boxed: Rect,
        texts: Vec<(String, Rect)>,
    }

    impl Shown {
        /// The texts above the `OK` choice, in order.
        fn lines(&self) -> Vec<&str> {
            let n = self.texts.len() - 1;
            self.texts[..n].iter().map(|(t, _)| t.as_str()).collect()
        }

        /// The room a line has: the screen less the margins and the popup's padding.
        fn max_w(&self) -> f32 {
            let m = &self.m;
            m.floor_px(self.screen.width() - 2.0 * m.u(MARGIN) - 2.0 * m.u(PAD_X))
        }

        /// `text` in the header font.
        fn width(&self, text: &str) -> f32 {
            let font = self.m.font_id(FontSize::Pt12);
            let galley = self
                .ctx
                .fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER));
            galley.size().x
        }
    }

    fn show(lines: &[&str], size: Vec2, ppp: f32) -> Shown {
        let ctx = new_ctx();
        let mut shapes = Vec::new();
        for n in 0..4 {
            shapes = frame(&ctx, lines, size, ppp, n);
        }
        let screen = Rect::from_min_size(pos2(0.0, 0.0), size);
        let bg = theme_by_id(0).colors.background.to_color32();
        let boxed = (shapes.iter())
            .find_map(|c| match &c.shape {
                Shape::Rect(r) if r.fill == bg && r.rect.width() == size.x && r.rect != screen => {
                    Some(r.rect)
                }
                _ => None,
            })
            .expect("the box is painted");
        let area = ctx
            .memory(|mem| mem.area_rect(Id::new("message")))
            .expect("the message is shown");
        Shown {
            m: Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX),
            screen,
            area,
            boxed,
            texts: texts(&shapes),
            ctx,
        }
    }

    const WINDOWS: [(f32, Vec2); 4] = [
        (1.0, vec2(1170.0, 540.0)),
        (2.0, vec2(1170.0, 540.0)),
        (1.0, vec2(360.0, 640.0)),
        (2.0, vec2(360.0, 640.0)),
    ];

    #[test]
    fn a_long_message_wraps_inside_the_box_and_the_box_inside_the_screen() {
        assert_eq!(LONG.len(), 300);
        for (ppp, size) in WINDOWS {
            let shown = show(&[LONG], size, ppp);
            let bounds = shown.screen.shrink(shown.m.u(MARGIN));
            let case = format!("ppp {ppp}, {size:?}: {:?}", shown.texts);
            // The painted box is as wide as the screen and inside the margins vertically; the text column and the
            // texts inside the margins.
            assert_eq!(shown.boxed.x_range(), shown.screen.x_range(), "{case}");
            assert!(
                bounds.y_range().contains(shown.boxed.top())
                    && bounds.y_range().contains(shown.boxed.bottom()),
                "{case}: {:?}",
                shown.boxed
            );
            assert!(bounds.contains_rect(shown.area), "{case}: {:?}", shown.area);
            assert!(shown.boxed.contains_rect(shown.area), "{case}");
            for (text, rect) in &shown.texts {
                assert!(shown.boxed.contains_rect(*rect), "{case}: {text}");
                assert!(shown.area.contains_rect(*rect), "{case}: {text}");
            }
            // Wrapped, in order, no word broken.
            let words: Vec<&str> = shown.lines().iter().flat_map(|l| l.split(' ')).collect();
            assert!(
                words.iter().copied().eq(LONG.split_whitespace()),
                "{case}: a word broke or went missing"
            );
            assert!(shown.lines().len() >= 2, "{case}: it did not wrap");
            // Each line is as full as it can be: it fits, and the next line's first word would not have.
            let max_w = shown.max_w();
            let lines = shown.lines();
            for (i, line) in lines.iter().enumerate() {
                assert!(shown.width(line) <= max_w, "{case}: {line}");
                if let Some(next) = lines.get(i + 1) {
                    let word = next.split(' ').next().unwrap();
                    assert!(
                        shown.width(&format!("{line} {word}")) > max_w,
                        "{case}: `{word}` fits on `{line}`"
                    );
                }
            }
            // The text column is the widest line's: within one word of the room.
            assert!(shown.area.width() <= bounds.width(), "{case}");
            assert!(
                shown.area.width() > bounds.width() - shown.width("application. "),
                "{case}: column {}",
                shown.area.width()
            );
        }
    }

    #[test]
    fn one_word_wider_than_the_screen_breaks_inside_it() {
        let word = "x".repeat(300);
        for (ppp, size) in WINDOWS {
            let shown = show(&[&word], size, ppp);
            let case = format!("ppp {ppp}, {size:?}");
            assert!(shown.lines().len() >= 2, "{case}");
            assert_eq!(shown.lines().concat(), word, "{case}");
            for (text, rect) in &shown.texts {
                assert!(shown.area.contains_rect(*rect), "{case}: {text}");
                assert!(shown.boxed.contains_rect(*rect), "{case}: {text}");
            }
        }
    }

    #[test]
    fn a_header_and_a_body_wrap_each_on_their_own() {
        let shown = show(&["Warning", LONG], vec2(360.0, 640.0), 1.0);
        assert_eq!(shown.lines()[0], "Warning");
        assert!(shown.lines().len() > 2);
    }

    #[test]
    fn a_short_message_is_one_line_per_text_as_before() {
        for (ppp, size) in WINDOWS {
            let shown = show(&["Delete?", "Really."], size, ppp);
            let case = format!("ppp {ppp}, {size:?}");
            let texts: Vec<&str> = shown.texts.iter().map(|(t, _)| t.as_str()).collect();
            assert_eq!(texts, ["Delete?", "Really.", "OK"], "{case}");
            // Centred on the screen; the box is full width and three 1.25 U rows tall, the column as wide as its
            // widest text and padding.
            assert!(
                (shown.area.center() - shown.screen.center())
                    .abs()
                    .max_elem()
                    <= 1.0
            );
            assert!(shown.area.width() < size.x / 2.0, "{case}");
            assert_eq!(shown.boxed.width(), size.x, "{case}");
            assert_eq!(shown.boxed.height(), shown.area.height(), "{case}");
            assert!((shown.boxed.center() - shown.screen.center()).abs().y <= 1.0);
        }
    }

    #[test]
    fn a_message_taller_than_the_screen_fills_the_margins_and_no_more() {
        let long = LONG.repeat(12);
        let size = vec2(360.0, 300.0);
        for ppp in [1.0, 2.0] {
            let shown = show(&[&long], size, ppp);
            let bounds = shown.screen.shrink(shown.m.u(MARGIN));
            let case = format!("ppp {ppp}: box {:?}, column {:?}", shown.boxed, shown.area);
            // The painted box is the scroll viewport's height (screen less margins) and full width.
            assert_eq!(shown.boxed.y_range(), bounds.y_range(), "{case}");
            assert_eq!(shown.boxed.x_range(), shown.screen.x_range(), "{case}");
            assert!(shown.screen.contains_rect(shown.boxed), "{case}");
            assert_eq!(shown.area.y_range(), bounds.y_range(), "{case}");
        }
    }

    #[test]
    fn a_resize_that_wraps_again_does_not_replay_the_opening() {
        let ctx = new_ctx();
        anim::enable(&ctx);
        let wide = vec2(1170.0, 540.0);
        let mut time = 0;
        let mut shapes = Vec::new();
        for _ in 0..14 {
            time += 1;
            shapes = frame(&ctx, &[LONG], wide, 1.0, time);
        }
        assert!(!texts(&shapes).is_empty(), "open after 14 frames");
        // Narrower: the same message wraps into more lines, and is still open.
        shapes = frame(&ctx, &[LONG], vec2(360.0, 640.0), 1.0, time + 1);
        let lines = texts(&shapes);
        assert!(lines.len() > 4, "{lines:?}");
        // Another message opens from the start.
        shapes = frame(&ctx, &["Another"], vec2(360.0, 640.0), 1.0, time + 2);
        assert!(texts(&shapes).is_empty());
    }
}

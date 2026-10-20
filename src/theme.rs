//! Theme system: colour roles, options, built-in and user themes, derived colours.
//!
//! **Colour type.** [`Rgba`] keeps straight-alpha `f32` channels in `0..=1`, because every colour formula (HSL, the
//! contrast ratio, the `(int)(v*255)` JSON truncation) is defined on floats. Painting code uses `egui::Color32`;
//! convert with [`Rgba::to_color32`].
//!
//! **Defaults.** Built-in themes and user JSON themes are both parsed into a `RawTheme` (every key optional) and
//! turned into a [`Theme`] by `RawTheme::resolve`, the only place defaults are applied.

use eframe::egui::Color32;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Fixed (not themeable) record colour, `#FF0000`.
pub const RECORD_COLOR: Rgba = Rgba::hex(0xFF0000);
/// Alpha of disabled items and controls: shown in place, dimmed and inert.
pub const DISABLED_ALPHA: f32 = 0.3;
/// Fixed fully transparent colour, `#00000000`.
pub const TRANSPARENT: Rgba = Rgba {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

/// Straight-alpha colour with float channels in `0..=1`. In JSON it is a `RRGGBB` string (alpha is dropped).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// Opaque colour from `0xRRGGBB`.
    pub const fn hex(rgb: u32) -> Self {
        Self::rgb(
            ((rgb >> 16) & 0xFF) as f32 / 255.0,
            ((rgb >> 8) & 0xFF) as f32 / 255.0,
            (rgb & 0xFF) as f32 / 255.0,
        )
    }

    /// Parses `RRGGBB`, with an optional leading `#`, in either case. Anything else (length, alpha) is `None`.
    pub fn from_hex_str(s: &str) -> Option<Self> {
        let digits = s.strip_prefix('#').unwrap_or(s);
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(digits, 16).ok().map(Self::hex)
    }

    /// Uppercase `RRGGBB`; each channel is `(int)(v*255)`, truncated as theme files store it.
    pub fn to_hex(self) -> String {
        let ch = |v: f32| (v * 255.0) as u8;
        format!("{:02X}{:02X}{:02X}", ch(self.r), ch(self.g), ch(self.b))
    }

    /// For painting with egui (channels are rounded to the nearest 1/255).
    pub fn to_color32(self) -> Color32 {
        let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgba_unmultiplied(ch(self.r), ch(self.g), ch(self.b), ch(self.a))
    }
}

impl Serialize for Rgba {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::from_hex_str(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid colour {s:?}")))
    }
}

/// The 11 colour roles of a theme.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeColors {
    pub background: Rgba,
    pub control_fill: Rgba,
    pub module_fill: Rgba,
    pub menu_fill: Rgba,
    pub text: Rgba,
    /// Text and icons on an `accent` fill.
    pub text_on_accent: Rgba,
    pub accent: Rgba,
    pub white_key: Rgba,
    pub black_key: Rgba,
    pub white_key_disabled: Rgba,
    pub black_key_disabled: Rgba,
}

impl ThemeColors {
    /// A dark theme: `background`'s brightest channel is no brighter than `text`'s. Paper is the one light built-in
    /// theme.
    pub fn is_dark(&self) -> bool {
        let max = |c: Rgba| c.r.max(c.g).max(c.b);
        max(self.background) <= max(self.text)
    }

    /// Default candidates for [`Self::best_contrasting`]: background, controlFill, moduleFill, menuFill, text.
    pub fn contrast_candidates(&self) -> [Rgba; 5] {
        [
            self.background,
            self.control_fill,
            self.module_fill,
            self.menu_fill,
            self.text,
        ]
    }

    /// The candidate with the largest lightness contrast ratio against `color`; ties keep the earlier candidate.
    /// An empty list gives `background`.
    pub fn best_contrasting(&self, color: Rgba, candidates: &[Rgba]) -> Rgba {
        let lightness = |c: Rgba| (c.r.max(c.g).max(c.b) + c.r.min(c.g).min(c.b)) / 2.0 + 0.01;
        let ratio = |c: Rgba| {
            let (l1, l2) = (lightness(color), lightness(c));
            l1.max(l2) / l1.min(l2)
        };
        let mut best: Option<(Rgba, f32)> = None;
        for &c in candidates {
            let r = ratio(c);
            if best.is_none_or(|(_, best_ratio)| r > best_ratio) {
                best = Some((c, r));
            }
        }
        best.map_or(self.background, |(c, _)| c)
    }
}

/// The least `playlistTrackNoteBrightness` a theme loads with: the muted-clip tint divides by it on a theme with a
/// dark text colour, so 0 (or less) would blow up.
pub const MIN_NOTE_BRIGHTNESS: f32 = 0.01;

/// The 4 float options of a theme.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeOptions {
    pub accent_saturation: f32,
    pub accent_lightness: f32,
    pub playlist_track_note_brightness: f32,
    pub playlist_track_background_transparency: f32,
}

/// A fully resolved theme.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Theme {
    pub id: u32,
    pub name: String,
    pub colors: ThemeColors,
    pub options: ThemeOptions,
}

impl Theme {
    /// Theme file contents.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a theme always serializes")
    }

    /// Parses a theme file; missing colours and options fall back to the defaults.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str::<RawTheme>(json).map(RawTheme::resolve)
    }
}

/// Theme as authored (built-in table or JSON file): every key may be missing.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RawColors {
    background: Option<Rgba>,
    control_fill: Option<Rgba>,
    module_fill: Option<Rgba>,
    menu_fill: Option<Rgba>,
    text: Option<Rgba>,
    text_on_accent: Option<Rgba>,
    accent: Option<Rgba>,
    white_key: Option<Rgba>,
    black_key: Option<Rgba>,
    white_key_disabled: Option<Rgba>,
    black_key_disabled: Option<Rgba>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RawOptions {
    accent_saturation: Option<f32>,
    accent_lightness: Option<f32>,
    playlist_track_note_brightness: Option<f32>,
    playlist_track_background_transparency: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
struct RawTheme {
    id: u32,
    name: String,
    #[serde(default)]
    colors: RawColors,
    #[serde(default)]
    options: RawOptions,
}

impl RawTheme {
    /// Applies the defaults table.
    fn resolve(self) -> Theme {
        let (c, o) = (self.colors, self.options);
        let text = c.text.unwrap_or(Rgba::hex(0xC7CEDA));
        Theme {
            id: self.id,
            name: self.name,
            colors: ThemeColors {
                background: c.background.unwrap_or(Rgba::hex(0x21272B)),
                control_fill: c.control_fill.unwrap_or(Rgba::hex(0x31373B)),
                module_fill: c.module_fill.unwrap_or(Rgba::hex(0x394046)),
                menu_fill: c.menu_fill.unwrap_or(Rgba::hex(0x434C51)),
                text,
                // A fixed default, not the theme's background: no built-in theme sets it.
                text_on_accent: c.text_on_accent.unwrap_or(Rgba::hex(0x21272B)),
                accent: c.accent.unwrap_or(text),
                white_key: c.white_key.unwrap_or(Rgba::hex(0xC7CEDA)),
                black_key: c.black_key.unwrap_or(Rgba::hex(0x21272B)),
                white_key_disabled: c.white_key_disabled.unwrap_or(Rgba::hex(0x9FA6B0)),
                black_key_disabled: c.black_key_disabled.unwrap_or(Rgba::hex(0x717981)),
            },
            options: ThemeOptions {
                accent_saturation: o.accent_saturation.unwrap_or(0.9),
                accent_lightness: o.accent_lightness.unwrap_or(0.63),
                playlist_track_note_brightness: o
                    .playlist_track_note_brightness
                    .unwrap_or(0.65)
                    .max(MIN_NOTE_BRIGHTNESS),
                playlist_track_background_transparency: o
                    .playlist_track_background_transparency
                    .unwrap_or(1.0),
            },
        }
    }
}

/// The 9 built-in themes, in ID order 0..=8.
pub fn builtin_themes() -> Vec<Theme> {
    // [background, controlFill, moduleFill, menuFill, text, accent]
    type Main = [u32; 6];
    // [whiteKey, blackKey, whiteKeyDisabled, blackKeyDisabled]
    type Keys = [u32; 4];
    // [accentSaturation, accentLightness, playlistTrackNoteBrightness, playlistTrackBackgroundTransparency]
    type Options = [f32; 4];

    let row = |id: u32, name: &str, main: Main, keys: Option<Keys>, options: Option<Options>| {
        let c = |hex: u32| Some(Rgba::hex(hex));
        let key = |i: usize| keys.and_then(|k| c(k[i]));
        let opt = |i: usize| options.map(|o| o[i]);
        RawTheme {
            id,
            name: name.to_owned(),
            colors: RawColors {
                background: c(main[0]),
                control_fill: c(main[1]),
                module_fill: c(main[2]),
                menu_fill: c(main[3]),
                text: c(main[4]),
                text_on_accent: None,
                accent: c(main[5]),
                white_key: key(0),
                black_key: key(1),
                white_key_disabled: key(2),
                black_key_disabled: key(3),
            },
            options: RawOptions {
                accent_saturation: opt(0),
                accent_lightness: opt(1),
                playlist_track_note_brightness: opt(2),
                playlist_track_background_transparency: opt(3),
            },
        }
    };
    const ACCENT: u32 = 0xFB8B3F;
    let clay_keys = Some([0xC5CAD7, 0x211D20, 0x9A98A4, 0x807981]);
    let clay_options = Some([0.90, 0.63, 1.00, 0.153]);

    [
        row(
            0,
            "Classic Gray",
            [0x21272B, 0x31373B, 0x394046, 0x434C51, 0xC7CEDA, ACCENT],
            None,
            None,
        ),
        row(
            1,
            "Deep Purple",
            [0x0C0C16, 0x17182D, 0x1D2033, 0x23283C, 0xFFFFFF, ACCENT],
            Some([0xFFFFFF, 0x000000, 0xBFBFBF, 0x404040]),
            Some([0.80, 0.55, 0.90, 0.15]),
        ),
        // Theme 2 has a neutral name of our own.
        row(
            2,
            "Slate",
            [0x1D2930, 0x233038, 0x34444E, 0x43525B, 0xD3D8DC, ACCENT],
            None,
            None,
        ),
        row(
            3,
            "Paper",
            [0x838383, 0xD4D4D4, 0xECEBE9, 0xF5F5F4, 0x584C46, ACCENT],
            Some([0xFFFFFF, 0x76665D, 0xE0E0E0, 0xCFC8C3]),
            Some([0.7961, 0.347, 0.60, 0.7453]),
        ),
        row(
            4,
            "Clay",
            [0x1C1B1C, 0x2B292B, 0x3D3637, 0x3D3637, 0xACA6A6, ACCENT],
            clay_keys,
            clay_options,
        ),
        row(
            5,
            "Ground",
            [0x131112, 0x1B191B, 0x262223, 0x262223, 0xACA6A6, ACCENT],
            clay_keys,
            clay_options,
        ),
        row(
            6,
            "Mint",
            [0x101413, 0x1A211F, 0x1D2727, 0x1D2727, 0xABDDD2, ACCENT],
            Some([0xD5E8DF, 0x192020, 0xADC2B8, 0x829998]),
            Some([0.90, 0.63, 1.00, 0.2007]),
        ),
        row(
            7,
            "High Contrast A",
            [0x090911, 0x17182D, 0x1D2033, 0x303753, 0xFFE577, ACCENT],
            Some([0xFFFFFF, 0x000000, 0xB6B6B6, 0x959595]),
            Some([1.00, 0.7852, 0.90, 0.15]),
        ),
        row(
            8,
            "High Contrast B",
            [0x111111, 0x222222, 0x1E1E1E, 0x5E5E5E, 0xFFFFFF, ACCENT],
            Some([0xFFFFFF, 0x000000, 0xCCCCCC, 0x7C7C7C]),
            Some([1.00, 0.6439, 1.00, 0.1303]),
        ),
    ]
    .into_iter()
    .map(RawTheme::resolve)
    .collect()
}

/// Theme by ID; an unknown ID gives theme 0 (Classic Gray).
pub fn theme_by_id(id: u32) -> Theme {
    let mut themes = builtin_themes();
    let index = themes.iter().position(|t| t.id == id).unwrap_or(0);
    themes.swap_remove(index)
}

/// Standard HSL to RGB with `h` in `[0,1)`, using rounded constants; outputs clamped to `[0,1]`.
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 0.16667 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 0.66667 {
            p + (q - p) * (0.66667 - t) * 6.0
        } else {
            p
        };
        v.clamp(0.0, 1.0)
    };
    (channel(h + 0.33333), channel(h), channel(h - 0.33333))
}

/// Colour of a channel with hue value `h` (clamped to `[0,1]`) under `theme`.
pub fn channel_color(h: f32, theme: &Theme) -> Rgba {
    let hue = (0.83 + 0.73 * h.clamp(0.0, 1.0)).rem_euclid(1.0);
    let (r, g, b) = hsl_to_rgb(
        hue,
        theme.options.accent_saturation,
        theme.options.accent_lightness,
    );
    Rgba::rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    fn assert_color_near(c: Rgba, hex: u32, tol: f32) {
        let want = Rgba::hex(hex);
        assert!(
            near(c.r, want.r, tol) && near(c.g, want.g, tol) && near(c.b, want.b, tol),
            "{} is not within {tol} of {hex:06X}",
            c.to_hex()
        );
    }

    #[test]
    fn classic_gray_resolves_with_defaults() {
        let c = theme_by_id(0).colors;
        assert_eq!(c.background.to_hex(), "21272B");
        assert_eq!(c.text.to_hex(), "C7CEDA");
        assert_eq!(c.accent.to_hex(), "FB8B3F");
        assert_eq!(c.white_key.to_hex(), "C7CEDA");
        assert_eq!(c.black_key_disabled.to_hex(), "717981");
    }

    #[test]
    fn paper_options() {
        let o = theme_by_id(3).options;
        assert!(near(o.accent_saturation, 0.7961, 1e-4));
        assert!(near(o.playlist_track_background_transparency, 0.7453, 1e-4));
    }

    #[test]
    fn builtins_are_numbered_in_order_and_unknown_id_is_classic_gray() {
        let themes = builtin_themes();
        assert_eq!(themes.len(), 9);
        assert!(themes.iter().enumerate().all(|(i, t)| t.id == i as u32));
        assert_eq!(themes[7].name, "High Contrast A");
        assert_eq!(theme_by_id(9).id, 0);
        assert_eq!(theme_by_id(u32::MAX).name, "Classic Gray");
        // A theme that sets its own keys and options: Deep Purple.
        let deep = &themes[1];
        assert_eq!(deep.colors.black_key.to_hex(), "000000");
        assert_eq!(deep.colors.white_key_disabled.to_hex(), "BFBFBF");
        assert!(near(deep.options.accent_lightness, 0.55, 1e-6));
        assert!(near(
            deep.options.playlist_track_note_brightness,
            0.90,
            1e-6
        ));
    }

    #[test]
    fn missing_keys_resolve_to_the_defaults_table() {
        let t = Theme::from_json(r#"{"id": 9, "name": "x"}"#).unwrap();
        let hex: Vec<String> = [
            t.colors.background,
            t.colors.control_fill,
            t.colors.module_fill,
            t.colors.menu_fill,
            t.colors.text,
            t.colors.accent,
            t.colors.white_key,
            t.colors.black_key,
            t.colors.white_key_disabled,
            t.colors.black_key_disabled,
        ]
        .map(Rgba::to_hex)
        .into();
        assert_eq!(
            hex.join(" "),
            "21272B 31373B 394046 434C51 C7CEDA C7CEDA C7CEDA 21272B 9FA6B0 717981"
        );
        let o = t.options;
        assert_eq!(
            [
                o.accent_saturation,
                o.accent_lightness,
                o.playlist_track_note_brightness,
                o.playlist_track_background_transparency
            ],
            [0.9, 0.63, 0.65, 1.0]
        );
    }

    #[test]
    fn the_note_brightness_never_loads_as_zero() {
        // The muted-clip tint divides by it on a theme with a dark text colour.
        let load = |v: &str| {
            let json = format!(
                r#"{{"id": 9, "name": "x", "options": {{"playlistTrackNoteBrightness": {v}}}}}"#
            );
            Theme::from_json(&json)
                .unwrap()
                .options
                .playlist_track_note_brightness
        };
        assert_eq!(load("0"), MIN_NOTE_BRIGHTNESS);
        assert_eq!(load("-0.5"), MIN_NOTE_BRIGHTNESS);
        assert!(load("0") > 0.0);
        assert_eq!(load("0.4"), 0.4);
    }

    #[test]
    fn accent_defaults_to_the_themes_own_text() {
        let t =
            Theme::from_json(r#"{"id": 9, "name": "x", "colors": {"text": "112233"}}"#).unwrap();
        assert_eq!(t.colors.accent.to_hex(), "112233");
        assert_eq!(t.colors.background.to_hex(), "21272B");
    }

    #[test]
    fn hsl_to_rgb_primaries_and_gray() {
        let (r, g, b) = hsl_to_rgb(0.0, 1.0, 0.5);
        assert!(near(r, 1.0, 1e-3) && near(g, 0.0, 1e-3) && near(b, 0.0, 1e-3));
        for h in [0.0, 0.3, 0.77] {
            assert_eq!(hsl_to_rgb(h, 0.0, 0.4), (0.4, 0.4, 0.4));
        }
    }

    #[test]
    fn channel_color_cyan() {
        let c = channel_color(0.9178, &theme_by_id(0));
        assert_color_near(c, 0x4CF5F5, 3.0 / 255.0);
        assert_eq!(c.a, 1.0);
        // h is clamped: 5.0 behaves like 1.0.
        assert_eq!(
            channel_color(5.0, &theme_by_id(0)),
            channel_color(1.0, &theme_by_id(0))
        );
    }

    #[test]
    fn best_contrasting_picks_the_highest_ratio() {
        let c = theme_by_id(0).colors;
        let list = c.contrast_candidates();
        assert_eq!(c.best_contrasting(Rgba::hex(0xF8F8F8), &list), c.background);
        assert_eq!(c.best_contrasting(Rgba::hex(0x101010), &list), c.text);
        assert_eq!(c.best_contrasting(Rgba::hex(0xF8F8F8), &[]), c.background);
        // Equal lightness ties: the earlier candidate wins, whatever the order.
        let (a, b) = (Rgba::rgb(0.25, 0.25, 0.25), Rgba::rgb(0.5, 0.0, 0.0));
        assert_eq!(c.best_contrasting(Rgba::hex(0xFFFFFF), &[a, b]), a);
        assert_eq!(c.best_contrasting(Rgba::hex(0xFFFFFF), &[b, a]), b);
    }

    /// All 11 colour roles, in struct order.
    fn all_colors(c: &ThemeColors) -> [Rgba; 11] {
        [
            c.background,
            c.control_fill,
            c.module_fill,
            c.menu_fill,
            c.text,
            c.text_on_accent,
            c.accent,
            c.white_key,
            c.black_key,
            c.white_key_disabled,
            c.black_key_disabled,
        ]
    }

    /// All 4 options, in struct order.
    fn all_options(o: &ThemeOptions) -> [f32; 4] {
        [
            o.accent_saturation,
            o.accent_lightness,
            o.playlist_track_note_brightness,
            o.playlist_track_background_transparency,
        ]
    }

    #[test]
    fn json_round_trip_and_hex_forms() {
        for t in builtin_themes() {
            let back = Theme::from_json(&t.to_json()).unwrap();
            assert_eq!((back.id, &back.name), (t.id, &t.name));
            let colors = all_colors(&t.colors)
                .into_iter()
                .zip(all_colors(&back.colors));
            for (i, (x, y)) in colors.enumerate() {
                let tol = 1.0 / 255.0;
                assert!(
                    near(x.r, y.r, tol) && near(x.g, y.g, tol) && near(x.b, y.b, tol),
                    "theme {} colour #{i}: {} became {}",
                    t.id,
                    x.to_hex(),
                    y.to_hex()
                );
            }
            let options = all_options(&t.options)
                .into_iter()
                .zip(all_options(&back.options));
            for (i, (x, y)) in options.enumerate() {
                assert!(
                    near(x, y, 1e-6),
                    "theme {} option #{i}: {x} became {y}",
                    t.id
                );
            }
        }
        // Wire key names; a renamed key would round-trip but not interoperate.
        let paper = theme_by_id(3).to_json();
        for key in [
            "whiteKeyDisabled",
            "textOnAccent",
            "playlistTrackBackgroundTransparency",
        ] {
            assert!(
                paper.contains(&format!("\"{key}\"")),
                "missing key {key} in {paper}"
            );
        }
        for s in ["21272b", "#21272B", "#21272b"] {
            assert_eq!(Rgba::from_hex_str(s), Some(Rgba::hex(0x21272B)), "{s}");
        }
        for s in ["", "#", "21272", "21272BFF", "+12345", "gggggg"] {
            assert_eq!(Rgba::from_hex_str(s), None, "{s}");
        }
    }

    #[test]
    fn hex_output_truncates_and_is_exact_for_n_over_255() {
        assert_eq!(Rgba::rgb(0.999, 0.5, 0.0).to_hex(), "FE7F00");
        for n in 0..=255u32 {
            assert_eq!(
                Rgba::hex(n * 0x01_01_01).to_hex(),
                format!("{n:02X}{n:02X}{n:02X}")
            );
        }
    }

    #[test]
    fn color32_conversion_and_fixed_globals() {
        assert_eq!(
            Rgba::hex(0x21272B).to_color32(),
            Color32::from_rgb(0x21, 0x27, 0x2B)
        );
        assert_eq!(RECORD_COLOR.to_color32(), Color32::from_rgb(255, 0, 0));
        assert_eq!(TRANSPARENT.to_color32(), Color32::TRANSPARENT);
    }
}

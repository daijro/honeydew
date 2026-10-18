//! Vector icons on a 48-unit design grid, drawn in one tint. An imported icon sheet replaces the glyphs
//! except for trademark IDs ([`sheet`]). The app mark uses our logo as a tinted image ([`logo`]).

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

#[cfg(any(debug_assertions, test))]
use crate::metrics::FontSize;
use crate::metrics::Metrics;

mod logo;
mod sheet;
#[cfg(test)]
pub(crate) use sheet::tests as sheet_tests;
pub use sheet::{SHEET_FILE, decode_sheet, install_sheet};

/// Icon IDs, numbered as the icon sheet's cells ([`sheet`]). Names follow each icon's role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum IconId {
    Browser = 0,
    StepSeq = 1,
    Piano = 2,
    Play = 3,
    Rewind = 4,
    Record = 5,
    AppLogo = 6,
    More = 7,
    Mixer = 8,
    Tempo = 9,
    Back = 10,
    Detach = 11,
    ChannelVolume = 12,
    Tracks = 13,
    Manual = 14,
    Snap = 15,
    PlatformA = 16,
    PlatformM = 17,
    PlatformW = 18,
    Close = 19,
    WirelessMidi = 20,
    AudioClip = 21,
    AudioRecord = 22,
    Position = 23,
    Volume = 24,
    Warning = 25,
    Loop = 26,
    Menu = 27,
    Freeze = 28,
    Search = 29,
    Import = 30,
    PatternClip = 31,
    Up = 32,
    Down = 33,
    Sharp = 34,
    Flat = 35,
    Natural = 36,
    Reorder = 37,
    Shop = 38,
    Locked = 39,
    KeyboardOff = 40,
    Help = 41,
    Free = 42,
    Previous = 43,
    Routing = 44,
    AddLarge = 45,
    Swap = 46,
    TempoTap = 47,
    KeyboardSettings = 48,
    PadA = 49,
    PadB = 50,
    PadX = 51,
    PadY = 52,
    PadLb = 53,
    PadRb = 54,
    PadLt = 55,
    PadRt = 56,
    DpadUp = 57,
    DpadRight = 58,
    DpadDown = 59,
    DpadLeft = 60,
    LeftStickVertical = 61,
    LeftStickHorizontal = 62,
    RightStickVertical = 63,
    RightStickHorizontal = 64,
    LeftStickClick = 65,
    RightStickClick = 66,
    PadMenu = 67,
    PadView = 68,
    FavoriteOff = 69,
    FavoriteOn = 70,
    AddSmall = 71,
    Confirm = 72,
    Done = 73,
    Filter = 74,
    Music = 75,
    Download = 76,
    Randomize = 77,
    CloudSync = 78,
}

impl IconId {
    /// Every icon, in ID order (`ALL[n] as u8 == n`).
    #[rustfmt::skip]
    pub const ALL: [IconId; 79] = {
        use IconId::*;
        [
            Browser, StepSeq, Piano, Play, Rewind, Record, AppLogo, More, Mixer, Tempo, Back,
            Detach, ChannelVolume, Tracks, Manual, Snap, PlatformA, PlatformM, PlatformW, Close,
            WirelessMidi, AudioClip, AudioRecord, Position, Volume, Warning, Loop, Menu, Freeze,
            Search, Import, PatternClip, Up, Down, Sharp, Flat, Natural, Reorder, Shop, Locked,
            KeyboardOff, Help, Free, Previous, Routing, AddLarge, Swap, TempoTap, KeyboardSettings,
            PadA, PadB, PadX, PadY, PadLb, PadRb, PadLt, PadRt, DpadUp, DpadRight, DpadDown,
            DpadLeft, LeftStickVertical, LeftStickHorizontal, RightStickVertical,
            RightStickHorizontal, LeftStickClick, RightStickClick, PadMenu, PadView, FavoriteOff,
            FavoriteOn, AddSmall, Confirm, Done, Filter, Music, Download, Randomize, CloudSync,
        ]
    };
}

/// Icons are designed on a 48 × 48 unit grid.
const GRID: f32 = 48.0;

/// Icon edge length in points: `U × size_factor × 4/3` (64-px sheet cells on a 48-unit grid).
pub fn icon_edge(size_factor: f32, m: &Metrics) -> f32 {
    m.u(size_factor * 4.0 / 3.0)
}

/// The square an icon occupies: centred in `rect`, then snapped to whole pixels by truncation.
pub fn icon_square(rect: Rect, size_factor: f32, m: &Metrics) -> Rect {
    let edge = icon_edge(size_factor, m);
    let min = rect.center() - Vec2::splat(edge / 2.0);
    let min = pos2(m.floor_px(min.x), m.floor_px(min.y));
    Rect::from_min_size(min, Vec2::splat(m.floor_px(edge)))
}

/// Draws icon `id` centred in `rect`, tinted with `color` ([`painted_shapes`]).
pub fn draw_icon(
    painter: &egui::Painter,
    rect: Rect,
    id: IconId,
    color: Color32,
    size_factor: f32,
    m: &Metrics,
) {
    let square = icon_square(rect, size_factor, m);
    painter.extend(painted_shapes(painter.ctx(), id, square, color));
}

/// What icon `id` paints on `square` in `ctx`, tinted `color`: for [`IconId::AppLogo`] our logo glyph's image
/// ([`logo`]); else its cell of the imported sheet when the user's imported content holds one ([`install_sheet`]), else
/// our own glyph ([`icon_shapes`]).
pub fn painted_shapes(ctx: &egui::Context, id: IconId, square: Rect, color: Color32) -> Vec<Shape> {
    if id == IconId::AppLogo {
        return vec![logo::logo_shape(ctx, square, color)];
    }
    match sheet::sheet_shape(ctx, id, square, color) {
        Some(shape) => vec![shape],
        None => icon_shapes(id, square, color),
    }
}

/// The shapes of icon `id`, with its 48 × 48 design box mapped onto `square`.
///
/// Shapes barely overlap, so a translucent tint stays (nearly) even.
pub fn icon_shapes(id: IconId, square: Rect, color: Color32) -> Vec<Shape> {
    use IconId::*;
    let mut p = Pen {
        origin: square.min,
        scale: square.width() / GRID,
        color,
        shapes: Vec::new(),
    };
    match id {
        Browser => {
            // Five bars of different heights standing on a shelf.
            for (i, top) in [22.0, 12.0, 26.0, 16.0, 30.0].into_iter().enumerate() {
                let x = 8.0 + 7.0 * i as f32;
                p.rect(x, top, x + 4.0, 35.0, 1.0);
            }
            p.rect(6.0, 37.0, 42.0, 40.0, 1.5);
        }
        StepSeq => {
            // Two rows of four pads.
            for row in 0..2 {
                for col in 0..4 {
                    let (x, y) = (6.0 + 9.5 * col as f32, 11.5 + 14.0 * row as f32);
                    p.rect(x, y, x + 7.5, y + 11.0, 2.0);
                }
            }
        }
        Piano => piano(&mut p, 8.0, 10.0, 40.0, 38.0, 4),
        Play => p.poly(&[(16.0, 11.0), (38.0, 24.0), (16.0, 37.0)]),
        Rewind => {
            p.rect(11.0, 12.0, 15.0, 36.0, 1.0);
            p.poly(&[(17.0, 24.0), (37.0, 12.0), (37.0, 36.0)]);
        }
        Record => p.disc(24.0, 24.0, 12.0),
        AppLogo => {
            // A one-colour stand-in for our logo, which `painted_shapes` draws instead ([`logo`]): one thick stroke
            // curled into an "S".
            let mut path = vec![(36.0, 12.0)];
            path.extend(arc_points(18.0, 18.0, 6.0, 270.0, 90.0));
            path.extend(arc_points(30.0, 30.0, 6.0, 270.0, 450.0));
            path.push((12.0, 36.0));
            p.line(&path, 6.0);
        }
        More => {
            for x in [12.0, 24.0, 36.0] {
                p.disc(x, 24.0, 3.5);
            }
        }
        Mixer => faders(&mut p, &[(11.5, 30.0), (24.0, 16.0), (36.5, 24.0)]),
        Back => p.line(&[(32.0, 7.0), (17.0, 24.0), (32.0, 41.0)], 2.5),
        Detach => {
            // Arrow to the upper left: a right-triangle head, the shaft starting on its long side.
            p.poly(&[(11.0, 11.0), (25.0, 11.0), (11.0, 25.0)]);
            p.line(&[(18.0, 18.0), (37.0, 37.0)], 3.5);
        }
        Close => {
            // One full diagonal; the other is split so the two never overlap.
            const W: f32 = 3.5;
            let gap = W / 2.0 / std::f32::consts::SQRT_2;
            p.line(&[(13.0, 13.0), (35.0, 35.0)], W);
            p.line(&[(35.0, 13.0), (24.0 + gap, 24.0 - gap)], W);
            p.line(&[(13.0, 35.0), (24.0 - gap, 24.0 + gap)], W);
        }
        AudioClip => {
            // Waveform bars around the centre line, then two trailing dots.
            for (i, half) in [5.0, 11.0, 16.0, 9.0, 13.0, 7.0, 4.0]
                .into_iter()
                .enumerate()
            {
                let x = 6.0 + 4.0 * i as f32;
                p.rect(x, 24.0 - half, x + 2.5, 24.0 + half, 1.0);
            }
            p.disc(36.5, 24.0, 1.5);
            p.disc(41.5, 24.0, 1.5);
        }
        AudioRecord => {
            // Microphone: capsule, cradle, stem and base.
            p.rect(18.0, 6.0, 30.0, 28.0, 6.0);
            p.arc(24.0, 22.0, 11.0, 0.0, 180.0, 2.5);
            p.rect(23.0, 34.25, 25.0, 40.0, 0.0);
            p.rect(17.0, 40.0, 31.0, 42.5, 1.0);
        }
        Menu => {
            for y in [14.0, 24.0, 34.0] {
                p.rect(10.0, y - 1.75, 38.0, y + 1.75, 1.75);
            }
        }
        Routing => {
            // A filled source box branching to three outlined targets.
            p.rect(6.5, 19.5, 15.5, 28.5, 1.5);
            p.rect(15.5, 22.75, 22.5, 25.25, 0.0);
            p.rect(22.5, 8.75, 25.0, 39.25, 0.0);
            for y in [10.0, 24.0, 38.0] {
                p.rect(25.0, y - 1.25, 32.5, y + 1.25, 0.0);
                p.frame(32.5, y - 4.5, 41.5, y + 4.5, 1.5, 2.5);
            }
        }
        AddLarge => {
            p.ring(24.0, 24.0, 18.0, 2.0);
            plus(&mut p, 10.0, 2.0);
        }
        KeyboardSettings => {
            // Four keys across the bottom, a gear clear above their right end.
            piano(&mut p, 4.0, 24.0, 44.0, 43.0, 4);
            gear(&mut p, 34.5, 11.5);
        }
        AddSmall => {
            p.ring(24.0, 24.0, 10.0, 2.5);
            plus(&mut p, 5.0, 2.5);
        }
        // Neutral stand-ins for trademarked glyphs: an "OS" badge with a letter, and broadcast waves.
        PlatformA => {
            badge(&mut p);
            p.line(&[(16.0, 33.0), (24.0, 14.0), (32.0, 33.0)], 3.0);
            p.line(&[(20.4, 26.5), (27.6, 26.5)], 3.0);
        }
        PlatformM => {
            badge(&mut p);
            p.line(
                &[
                    (15.0, 33.0),
                    (15.0, 15.0),
                    (24.0, 26.0),
                    (33.0, 15.0),
                    (33.0, 33.0),
                ],
                3.0,
            );
        }
        PlatformW => {
            badge(&mut p);
            p.line(
                &[
                    (14.0, 15.0),
                    (19.0, 33.0),
                    (24.0, 21.0),
                    (29.0, 33.0),
                    (34.0, 15.0),
                ],
                3.0,
            );
        }
        WirelessMidi => {
            p.disc(24.0, 24.0, 3.5);
            for r in [9.5, 16.0] {
                p.arc(24.0, 24.0, r, -45.0, 45.0, 2.5);
                p.arc(24.0, 24.0, r, 135.0, 225.0, 2.5);
            }
        }
        Snap => {
            // Horseshoe magnet opening upward; the pole tips stand apart from the arms.
            let mut horseshoe = vec![(13.0, 17.0)];
            horseshoe.extend(arc_points(24.0, 25.0, 11.0, 180.0, 0.0));
            horseshoe.push((35.0, 17.0));
            p.line(&horseshoe, 8.0);
            p.rect(9.0, 8.0, 17.0, 15.0, 0.0);
            p.rect(31.0, 8.0, 39.0, 15.0, 0.0);
        }
        Swap => {
            // Right along the top, left along the bottom.
            p.rect(8.0, 15.5, 30.0, 18.5, 0.0);
            p.poly(&[(30.0, 10.0), (40.0, 17.0), (30.0, 24.0)]);
            p.rect(18.0, 29.5, 40.0, 32.5, 0.0);
            p.poly(&[(18.0, 24.0), (18.0, 38.0), (8.0, 31.0)]);
        }
        Music => {
            // Two tilted note heads rising to the right, their stems and the beam.
            let beam = |x: f32| 12.0 - (x - 18.25) * 4.0 / 19.5;
            p.poly(&[
                (18.25, beam(18.25)),
                (37.75, beam(37.75)),
                (37.75, beam(37.75) + 5.0),
                (18.25, beam(18.25) + 5.0),
            ]);
            for (x, y) in [(15.5, 36.0), (32.5, 32.0)] {
                p.poly(&ellipse_points(x, y, 5.5, 4.0, -25.0));
                p.rect(x + 2.75, beam(x + 5.25) + 5.0, x + 5.25, y - 1.0, 0.0);
            }
        }
        Tempo => {
            // A clock: rim and two hands (a tint-only mask cannot cut the hands out of a filled disc).
            p.ring(24.0, 24.0, 16.0, 3.0);
            p.line(&[(24.0, 13.0), (24.0, 24.0), (32.0, 28.0)], 3.0);
        }
        ChannelVolume => faders(&mut p, &[(17.0, 28.0), (31.0, 17.0)]),
        Tracks => {
            // Two clips on two tracks.
            p.rect(6.0, 13.0, 32.0, 21.0, 2.0);
            p.rect(16.0, 27.0, 42.0, 35.0, 2.0);
        }
        Manual => {
            // An open book: two pages rising from the spine.
            p.outline(&[(22.0, 14.0), (7.0, 10.0), (7.0, 35.0), (22.0, 39.0)], 2.5);
            p.outline(
                &[(26.0, 14.0), (41.0, 10.0), (41.0, 35.0), (26.0, 39.0)],
                2.5,
            );
        }
        Position => plus(&mut p, 16.0, 2.0),
        Volume => {
            // Speaker box and cone, then two sound waves.
            p.rect(7.5, 18.0, 14.5, 30.0, 0.0);
            p.poly(&[(14.5, 18.0), (24.5, 10.0), (24.5, 38.0), (14.5, 30.0)]);
            for r in [7.0, 13.5] {
                p.arc(25.5, 24.0, r, -50.0, 50.0, 2.5);
            }
        }
        Warning => {
            p.outline(&[(24.0, 8.0), (42.0, 39.0), (6.0, 39.0)], 2.5);
            p.rect(22.75, 18.0, 25.25, 30.0, 1.0);
            p.disc(24.0, 34.0, 1.75);
        }
        Loop => {
            // Two arcs chasing each other clockwise.
            for from in [200.0f32, 20.0] {
                let to = from + 125.0;
                p.arc(24.0, 24.0, 13.0, from, to, 2.5);
                let (s, c) = to.to_radians().sin_cos();
                arrowhead(
                    &mut p,
                    (24.0 + 13.0 * c, 24.0 + 13.0 * s),
                    (-s, c),
                    6.0,
                    5.0,
                );
            }
        }
        Freeze => {
            // Snowflake: a hub, six arms, a V of twigs on each.
            p.disc(24.0, 24.0, 3.0);
            for i in 0..6 {
                let (s, c) = (std::f32::consts::FRAC_PI_3 * i as f32).sin_cos();
                // `along` the arm (the first one points up) and `across` it.
                let at = |along: f32, across: f32| {
                    (24.0 + along * s + across * c, 24.0 - along * c + across * s)
                };
                p.line(&[at(3.0, 0.0), at(17.0, 0.0)], 2.5);
                p.line(&[at(14.0, -4.5), at(10.0, 0.0), at(14.0, 4.5)], 2.5);
            }
        }
        Search => {
            p.ring(20.0, 20.0, 11.0, 3.0);
            p.line(&[(29.0, 29.0), (39.0, 39.0)], 4.5);
        }
        Import => {
            // An arrow down into an open tray.
            p.rect(22.75, 7.0, 25.25, 22.0, 0.0);
            p.poly(&[(16.0, 22.0), (32.0, 22.0), (24.0, 31.0)]);
            p.line(&[(8.0, 27.0), (8.0, 39.0), (40.0, 39.0), (40.0, 27.0)], 2.5);
        }
        PatternClip => {
            // Short notes staggered up and down.
            for (x, y) in [(6.0, 33.0), (14.0, 21.0), (22.0, 27.0), (30.0, 15.0)] {
                p.rect(x, y - 2.5, x + 12.0, y + 2.5, 1.5);
            }
        }
        Up => p.line(&[(12.0, 30.0), (24.0, 18.0), (36.0, 30.0)], 3.0),
        Down => p.line(&[(12.0, 18.0), (24.0, 30.0), (36.0, 18.0)], 3.0),
        Sharp => {
            // Two upright strokes; two rising bars, cut where they cross the strokes so nothing overlaps.
            p.rect(17.75, 7.0, 20.25, 41.0, 0.0);
            p.rect(27.75, 7.0, 30.25, 41.0, 0.0);
            for mid in [18.5, 29.5] {
                for (x0, x1) in [(11.0, 17.75), (20.25, 27.75), (30.25, 37.0)] {
                    rising_bar(&mut p, x0, x1, mid);
                }
            }
        }
        Flat => {
            // Stem, and a bowl swelling out to the right and closing to a point on the stem.
            p.rect(17.75, 6.0, 20.25, 42.0, 0.0);
            let mut bowl: Vec<_> = arc_points(22.0, 30.0, 7.0, 268.0, 400.0).collect();
            bowl.push((21.75, 41.0));
            p.line(&bowl, 3.0);
        }
        Natural => {
            // Left stroke from the top, right stroke to the bottom, two rising bars between them.
            p.rect(15.75, 6.0, 18.25, 32.4, 0.0);
            p.rect(29.75, 15.6, 32.25, 42.0, 0.0);
            for mid in [19.0, 29.0] {
                rising_bar(&mut p, 18.25, 29.75, mid);
            }
        }
        Reorder => {
            p.poly(&[(24.0, 6.0), (35.0, 17.0), (13.0, 17.0)]);
            p.rect(21.75, 17.0, 26.25, 31.0, 0.0);
            p.poly(&[(13.0, 31.0), (35.0, 31.0), (24.0, 42.0)]);
        }
        Shop => {
            // Cart: handle and frame, basket, two wheels.
            p.line(&[(5.0, 9.0), (10.5, 9.0), (14.5, 31.0), (37.0, 31.0)], 2.5);
            p.poly(&[(13.2, 14.0), (41.0, 14.0), (38.0, 26.0), (15.4, 26.0)]);
            p.disc(17.0, 37.5, 3.0);
            p.disc(34.0, 37.5, 3.0);
        }
        Locked => {
            // Padlock: a shackle over a solid body.
            let mut shackle = vec![(16.0, 22.0)];
            shackle.extend(arc_points(24.0, 17.0, 8.0, 180.0, 360.0));
            shackle.push((32.0, 22.0));
            p.line(&shackle, 3.0);
            p.rect(12.0, 22.0, 36.0, 41.0, 3.0);
        }
        KeyboardOff => {
            piano(&mut p, 8.0, 10.0, 40.0, 38.0, 4);
            p.line(&[(7.0, 41.0), (41.0, 7.0)], 3.0);
        }
        Help => {
            // A page with a folded corner, and a question mark.
            p.outline(
                &[
                    (10.0, 6.0),
                    (30.0, 6.0),
                    (38.0, 14.0),
                    (38.0, 42.0),
                    (10.0, 42.0),
                ],
                2.5,
            );
            p.line(&[(30.0, 6.0), (30.0, 14.0), (38.0, 14.0)], 2.5);
            let mut hook: Vec<_> = arc_points(24.0, 20.0, 5.0, 180.0, 405.0).collect();
            hook.extend([(24.0, 26.5), (24.0, 29.0)]);
            p.line(&hook, 2.5);
            p.disc(24.0, 34.0, 1.75);
        }
        Free => {
            // A tag reading FREE.
            p.frame(4.0, 13.0, 44.0, 35.0, 4.0, 2.0);
            text(&mut p, "FREE", 24.0, 24.0, 10.0, 2.0);
        }
        Previous => {
            p.poly(&[(8.0, 24.0), (22.0, 11.0), (22.0, 37.0)]);
            p.rect(22.0, 22.25, 40.0, 25.75, 0.0);
        }
        TempoTap => {
            // A half-round dial, its hub and the needle.
            p.arc(24.0, 31.0, 16.0, 180.0, 360.0, 2.5);
            p.disc(24.0, 31.0, 3.0);
            let (s, c) = (-55.0f32).to_radians().sin_cos();
            p.line(
                &[
                    (24.0 + 3.0 * c, 31.0 + 3.0 * s),
                    (24.0 + 12.5 * c, 31.0 + 12.5 * s),
                ],
                2.5,
            );
        }
        // Gamepad glyphs: outlined shapes with stroked letters (no cut-outs in a tint-only mask).
        PadA | PadB | PadX | PadY => {
            p.ring(24.0, 24.0, 16.0, 2.5);
            let name = ["A", "B", "X", "Y"][id as usize - PadA as usize];
            text(&mut p, name, 24.0, 24.0, 14.0, 3.0);
        }
        PadLb | PadRb => {
            // A bumper: a wide pill.
            p.frame(5.0, 14.0, 43.0, 34.0, 10.0, 2.5);
            text(
                &mut p,
                if id == PadLb { "LB" } else { "RB" },
                24.0,
                24.0,
                10.0,
                2.5,
            );
        }
        PadLt | PadRt => {
            // A trigger: a tall rounded block.
            p.frame(11.0, 6.0, 37.0, 42.0, 8.0, 2.5);
            text(
                &mut p,
                if id == PadLt { "LT" } else { "RT" },
                24.0,
                26.0,
                10.0,
                2.5,
            );
        }
        DpadUp | DpadRight | DpadDown | DpadLeft => dpad(&mut p, id as u8 - DpadUp as u8),
        LeftStickVertical => stick_axis(&mut p, "L", true),
        LeftStickHorizontal => stick_axis(&mut p, "L", false),
        RightStickVertical => stick_axis(&mut p, "R", true),
        RightStickHorizontal => stick_axis(&mut p, "R", false),
        LeftStickClick => stick_click(&mut p, "L"),
        RightStickClick => stick_click(&mut p, "R"),
        PadMenu => {
            p.ring(24.0, 24.0, 16.0, 2.5);
            for y in [18.5, 24.0, 29.5] {
                p.rect(16.0, y - 1.25, 32.0, y + 1.25, 1.25);
            }
        }
        PadView => {
            // Two overlapping squares; the back one stops at the front one's edge.
            p.ring(24.0, 24.0, 16.0, 2.5);
            p.line(
                &[
                    (20.75, 26.0),
                    (16.0, 26.0),
                    (16.0, 16.0),
                    (26.0, 16.0),
                    (26.0, 20.75),
                ],
                2.5,
            );
            p.outline(
                &[(22.0, 22.0), (32.0, 22.0), (32.0, 32.0), (22.0, 32.0)],
                2.5,
            );
        }
        FavoriteOff => p.outline(&star(), 2.5),
        FavoriteOn => {
            // One polygon: egui fills a path as a fan from its first point, and a star is star-shaped from
            // its notches, so starting at one fills it without the seams of convex pieces.
            let mut s = star();
            s.rotate_left(1);
            p.poly(&s);
        }
        Confirm => p.line(&[(11.0, 24.0), (20.0, 33.0), (37.0, 14.0)], 3.5),
        Done => {
            p.ring(24.0, 24.0, 17.0, 2.5);
            p.line(&[(16.0, 24.5), (21.5, 30.0), (32.0, 18.5)], 3.0);
        }
        Filter => {
            // Three centred lines, each shorter than the last.
            for (y, half) in [(14.0, 15.0), (24.0, 10.0), (34.0, 5.0)] {
                p.rect(24.0 - half, y - 1.75, 24.0 + half, y + 1.75, 1.75);
            }
        }
        Download => {
            p.ring(24.0, 24.0, 17.0, 2.5);
            p.rect(22.75, 13.0, 25.25, 25.0, 0.0);
            p.poly(&[(17.0, 25.0), (31.0, 25.0), (24.0, 33.0)]);
        }
        Randomize => {
            // A die showing three pips.
            badge(&mut p);
            for xy in [16.5, 24.0, 31.5] {
                p.disc(xy, xy, 3.25);
            }
        }
        CloudSync => {
            // Cloud outline: two small bumps flanking a big one, over a flat base.
            let mut cloud: Vec<_> = arc_points(14.0, 29.0, 7.0, 90.0, 270.0).collect();
            cloud.extend(arc_points(24.0, 22.0, 10.0, 180.0, 360.0).skip(1));
            cloud.extend(arc_points(34.0, 29.0, 7.0, 270.0, 450.0).skip(1));
            p.outline(&cloud, 2.5);
        }
    }
    p.shapes
}

/// Piano keys seen from the front: an outline, `keys` white keys and a black key on every inner boundary.
fn piano(p: &mut Pen, x0: f32, y0: f32, x1: f32, y1: f32, keys: u8) {
    const W: f32 = 2.5;
    p.frame(x0, y0, x1, y1, 2.0, W);
    let black_bottom = y0 + 0.6 * (y1 - y0);
    for i in 1..keys {
        let x = x0 + (x1 - x0) * f32::from(i) / f32::from(keys);
        p.rect(x - 2.5, y0 + W, x + 2.5, black_bottom, 0.0);
        p.rect(x - 1.0, black_bottom, x + 1.0, y1 - W, 0.0);
    }
}

/// A six-tooth gear with a small hole, centred on (x, y), 21 units across: a solid hub out to 6.5 and teeth 4 units
/// deep and 3.4 wide at the tip, so the teeth still stand apart at 0.6 px a unit.
fn gear(p: &mut Pen, x: f32, y: f32) {
    p.ring(x, y, 4.5, 4.0);
    for i in 0..6 {
        let (s, c) = (std::f32::consts::FRAC_PI_3 * i as f32).sin_cos();
        let at = |r: f32, w: f32| (x + r * c - w * s, y + r * s + w * c);
        p.poly(&[at(6.0, -2.0), at(10.5, -1.7), at(10.5, 1.7), at(6.0, 2.0)]);
    }
}

/// A centred plus with arms `arm` long; the vertical bar is split so the bars never overlap.
fn plus(p: &mut Pen, arm: f32, w: f32) {
    let (lo, hi) = (24.0 - w / 2.0, 24.0 + w / 2.0);
    p.rect(24.0 - arm, lo, 24.0 + arm, hi, 0.0);
    p.rect(lo, 24.0 - arm, hi, lo, 0.0);
    p.rect(lo, hi, hi, 24.0 + arm, 0.0);
}

/// Fader tracks 3 units wide at `(x, knob y)`, each split around its 11 × 8 knob.
fn faders(p: &mut Pen, faders: &[(f32, f32)]) {
    for &(x, knob) in faders {
        p.rect(x - 1.5, 7.0, x + 1.5, knob - 4.0, 0.0);
        p.rect(x - 5.5, knob - 4.0, x + 5.5, knob + 4.0, 2.0);
        p.rect(x - 1.5, knob + 4.0, x + 1.5, 41.0, 0.0);
    }
}

/// A filled arrowhead with its base centred on `base`, pointing along the unit vector `dir`.
fn arrowhead(p: &mut Pen, (x, y): (f32, f32), (dx, dy): (f32, f32), len: f32, half: f32) {
    p.poly(&[
        (x - dy * half, y + dx * half),
        (x + dx * len, y + dy * len),
        (x + dy * half, y - dx * half),
    ]);
}

/// A 4.5-unit bar of a sharp or natural from `x0` to `x1`, rising one unit per five, centred on `mid` at x = 24.
fn rising_bar(p: &mut Pen, x0: f32, x1: f32, mid: f32) {
    let y = |x: f32| mid - 0.2 * (x - 24.0);
    p.poly(&[
        (x0, y(x0) - 2.25),
        (x1, y(x1) - 2.25),
        (x1, y(x1) + 2.25),
        (x0, y(x0) + 2.25),
    ]);
}

/// Points around an ellipse with radii `rx` and `ry`, its `rx` axis turned `tilt` degrees (positive = clockwise).
fn ellipse_points(x: f32, y: f32, rx: f32, ry: f32, tilt: f32) -> Vec<(f32, f32)> {
    let (s, c) = tilt.to_radians().sin_cos();
    (0..24)
        .map(|i| {
            let (v, u) = (std::f32::consts::TAU * i as f32 / 24.0).sin_cos();
            let (u, v) = (rx * u, ry * v);
            (x + u * c - v * s, y + u * s + v * c)
        })
        .collect()
}

/// A five-pointed star centred on (24, 25.5): tips (even indices) 17 units out, notches 7.5, the first tip up.
fn star() -> [(f32, f32); 10] {
    std::array::from_fn(|i| {
        let r = if i % 2 == 0 { 17.0 } else { 7.5 };
        let (s, c) = (36.0 * i as f32).to_radians().sin_cos();
        (24.0 + r * s, 25.5 - r * c)
    })
}

/// A d-pad outline with arm `arm` (0 up, 1 right, 2 down, 3 left) filled up to the stroke's inner edge.
fn dpad(p: &mut Pen, arm: u8) {
    p.outline(
        &[
            (18.0, 7.0),
            (30.0, 7.0),
            (30.0, 18.0),
            (41.0, 18.0),
            (41.0, 30.0),
            (30.0, 30.0),
            (30.0, 41.0),
            (18.0, 41.0),
            (18.0, 30.0),
            (7.0, 30.0),
            (7.0, 18.0),
            (18.0, 18.0),
        ],
        2.5,
    );
    match arm {
        0 => p.rect(19.25, 8.25, 28.75, 19.25, 0.0),
        1 => p.rect(28.75, 19.25, 39.75, 28.75, 0.0),
        2 => p.rect(19.25, 28.75, 28.75, 39.75, 0.0),
        _ => p.rect(8.25, 19.25, 19.25, 28.75, 0.0),
    }
}

/// A thumbstick: a ring around its letter, centred on (24, y).
fn stick(p: &mut Pen, name: &str, y: f32) {
    p.ring(24.0, y, 11.0, 2.5);
    text(p, name, 24.0, y, 10.0, 2.5);
}

/// A thumbstick with arrowheads at both ends of one axis.
fn stick_axis(p: &mut Pen, name: &str, vertical: bool) {
    stick(p, name, 24.0);
    for sign in [-1.0, 1.0] {
        let (dx, dy) = if vertical { (0.0, sign) } else { (sign, 0.0) };
        arrowhead(p, (24.0 + dx * 13.5, 24.0 + dy * 13.5), (dx, dy), 5.5, 5.0);
    }
}

/// A thumbstick pressed down: an arrowhead pointing at its top.
fn stick_click(p: &mut Pen, name: &str) {
    stick(p, name, 28.0);
    arrowhead(p, (24.0, 8.0), (0.0, 1.0), 6.0, 6.0);
}

/// Stroked capitals, `h` units tall and centred on (x, y); see [`letter`].
fn text(p: &mut Pen, text: &str, x: f32, y: f32, h: f32, w: f32) {
    let k = h / 10.0;
    let (advance, n) = (8.5 * k, text.chars().count() as f32);
    let left = x - (advance * n - 2.5 * k) / 2.0;
    for (i, c) in text.chars().enumerate() {
        let x0 = left + advance * i as f32;
        for stroke in letter(c) {
            let points: Vec<_> = stroke
                .into_iter()
                .map(|(u, v)| (x0 + u * k, y - h / 2.0 + v * k))
                .collect();
            p.line(&points, w);
        }
    }
}

/// The strokes of capital `c` in a 6 × 10 box; only the letters the icons use.
fn letter(c: char) -> Vec<Vec<(f32, f32)>> {
    // Stem, then the upper bowl of B and R back to the stem's middle.
    let upper = || {
        let mut s = vec![(0.0, 10.0), (0.0, 0.0)];
        s.extend(arc_points(3.5, 2.5, 2.5, 270.0, 450.0));
        s.push((0.0, 5.0));
        s
    };
    match c {
        'A' => vec![
            vec![(0.0, 10.0), (3.0, 0.0), (6.0, 10.0)],
            vec![(1.05, 6.5), (4.95, 6.5)],
        ],
        'B' => vec![
            upper(),
            arc_points(3.5, 7.5, 2.5, 270.0, 450.0)
                .chain([(0.0, 10.0)])
                .collect(),
        ],
        'E' => vec![
            vec![(6.0, 0.0), (0.0, 0.0), (0.0, 10.0), (6.0, 10.0)],
            vec![(0.0, 5.0), (5.0, 5.0)],
        ],
        'F' => vec![
            vec![(6.0, 0.0), (0.0, 0.0), (0.0, 10.0)],
            vec![(0.0, 5.0), (5.0, 5.0)],
        ],
        'L' => vec![vec![(0.0, 0.0), (0.0, 10.0), (6.0, 10.0)]],
        'R' => vec![upper(), vec![(3.0, 5.0), (6.0, 10.0)]],
        'T' => vec![vec![(0.0, 0.0), (6.0, 0.0)], vec![(3.0, 0.0), (3.0, 10.0)]],
        'X' => vec![vec![(0.0, 0.0), (6.0, 10.0)], vec![(6.0, 0.0), (0.0, 10.0)]],
        'Y' => vec![
            vec![(0.0, 0.0), (3.0, 5.0), (6.0, 0.0)],
            vec![(3.0, 5.0), (3.0, 10.0)],
        ],
        _ => unreachable!("no stroked letter {c}"),
    }
}

/// Rounded-square outline of the generic "OS" badges, and of the die.
fn badge(p: &mut Pen) {
    p.frame(8.0, 8.0, 40.0, 40.0, 6.0, 2.5);
}

/// Emits tinted shapes, taking coordinates in design units (0..48, y down).
struct Pen {
    origin: Pos2,
    /// Points per design unit.
    scale: f32,
    color: Color32,
    shapes: Vec<Shape>,
}

impl Pen {
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.scale
    }

    fn pts(&self, points: &[(f32, f32)]) -> Vec<Pos2> {
        points.iter().map(|&(x, y)| self.pt(x, y)).collect()
    }

    fn stroke(&self, width: f32) -> Stroke {
        Stroke::new(width * self.scale, self.color)
    }

    /// Filled rectangle with corner radius `r`.
    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) {
        let rect = Rect::from_min_max(self.pt(x0, y0), self.pt(x1, y1));
        self.shapes
            .push(Shape::rect_filled(rect, r * self.scale, self.color));
    }

    /// Rectangle outline `w` wide, drawn inside the edges.
    fn frame(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, w: f32) {
        let rect = Rect::from_min_max(self.pt(x0, y0), self.pt(x1, y1));
        let stroke = self.stroke(w);
        self.shapes.push(Shape::rect_stroke(
            rect,
            r * self.scale,
            stroke,
            StrokeKind::Inside,
        ));
    }

    fn disc(&mut self, x: f32, y: f32, r: f32) {
        self.shapes.push(Shape::circle_filled(
            self.pt(x, y),
            r * self.scale,
            self.color,
        ));
    }

    /// Circle outline `w` wide, centred on radius `r`.
    fn ring(&mut self, x: f32, y: f32, r: f32, w: f32) {
        let stroke = self.stroke(w);
        self.shapes
            .push(Shape::circle_stroke(self.pt(x, y), r * self.scale, stroke));
    }

    /// Closed outline `w` wide, centred on the path through `points`.
    fn outline(&mut self, points: &[(f32, f32)], w: f32) {
        let (points, stroke) = (self.pts(points), self.stroke(w));
        self.shapes.push(Shape::closed_line(points, stroke));
    }

    /// Filled polygon: convex, or at least star-shaped from its first point (egui fills a fan from it).
    fn poly(&mut self, points: &[(f32, f32)]) {
        let points = self.pts(points);
        self.shapes
            .push(Shape::convex_polygon(points, self.color, Stroke::NONE));
    }

    /// Open polyline `w` wide.
    fn line(&mut self, points: &[(f32, f32)], w: f32) {
        let (points, stroke) = (self.pts(points), self.stroke(w));
        self.shapes.push(Shape::line(points, stroke));
    }

    /// Arc stroke `w` wide; see [`arc_points`].
    fn arc(&mut self, x: f32, y: f32, r: f32, from: f32, to: f32, w: f32) {
        let points: Vec<_> = arc_points(x, y, r, from, to).collect();
        self.line(&points, w);
    }
}

/// Points on an arc of radius `r` around (x, y) from `from` to `to` degrees (0° = right, 90° = down).
fn arc_points(x: f32, y: f32, r: f32, from: f32, to: f32) -> impl Iterator<Item = (f32, f32)> {
    const STEPS: u8 = 16;
    (0..=STEPS).map(move |i| {
        let a = (from + (to - from) * f32::from(i) / f32::from(STEPS)).to_radians();
        (x + r * a.cos(), y + r * a.sin())
    })
}

/// Debug view: every icon in an 8-column grid with its ID, tinted `color` on `background`. Debug builds (the F2
/// sheet) and tests (the `render_icon_sheets` harness) only.
#[cfg(any(debug_assertions, test))]
pub fn draw_debug_sheet(
    painter: &egui::Painter,
    area: Rect,
    color: Color32,
    background: Color32,
    m: &Metrics,
) {
    painter.rect_filled(area, 0.0, background);
    // Each cell: the ID on the left, the icon in a 1.5 U square on the right.
    let cell = vec2(m.u(2.0), m.u(1.5));
    let faint = color.gamma_multiply(0.3);
    for id in IconId::ALL {
        let n = id as usize;
        let min = area.min + vec2((n % 8) as f32 * cell.x, (n / 8) as f32 * cell.y);
        let icon_rect = Rect::from_min_size(min + vec2(cell.x - cell.y, 0.0), Vec2::splat(cell.y));
        let square = icon_square(icon_rect, 1.0, m);
        painter.rect_stroke(
            square,
            0.0,
            Stroke::new(1.0 / m.ppp(), faint),
            StrokeKind::Outside,
        );
        draw_icon(painter, icon_rect, id, color, 1.0, m);
        painter.text(
            min,
            egui::Align2::LEFT_TOP,
            n.to_string(),
            m.font_id(FontSize::Pt8),
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE_INDEX;

    #[test]
    fn the_catalog_lists_every_icon_in_id_order() {
        for (n, id) in IconId::ALL.into_iter().enumerate() {
            assert_eq!(id as usize, n);
        }
    }

    #[test]
    fn edge_is_four_thirds_of_u() {
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
        assert_eq!(icon_edge(1.0, &m), m.u(1.0) * 4.0 / 3.0);
        assert_eq!(icon_edge(0.5, &m), m.u(1.0) * 2.0 / 3.0);
    }

    #[test]
    fn icon_square_is_centred_and_pixel_snapped() {
        // ppp 1.5 -> U = 32 pt, edge 42.67 pt = 64 px.
        let m = Metrics::from_pixels_per_point(1.5, DEFAULT_UI_SCALE_INDEX);
        let rect = Rect::from_min_size(pos2(10.1, 20.3), vec2(100.0, 50.0));
        let sq = icon_square(rect, 1.0, &m);
        assert!((sq.width() - sq.height()).abs() < 1e-3);
        assert!(
            (sq.width() * m.ppp() - 64.0).abs() < 1e-3,
            "edge is not 64 px"
        );
        for v in [sq.min.x, sq.min.y, sq.width()] {
            let px = v * m.ppp();
            assert!(
                (px - px.round()).abs() < 1e-3,
                "{v} pt is not a whole pixel"
            );
        }
        assert!((sq.center() - rect.center()).length() <= 1.0 / m.ppp());
    }

    #[test]
    fn every_icon_is_its_own_glyph() {
        let square = Rect::from_min_size(pos2(0.0, 0.0), vec2(48.0, 48.0));
        let glyphs: Vec<_> = IconId::ALL
            .iter()
            .map(|&id| (id, icon_shapes(id, square, Color32::WHITE)))
            .collect();
        for (i, (a, a_shapes)) in glyphs.iter().enumerate() {
            assert!(!a_shapes.is_empty(), "{a:?} draws nothing");
            for (b, b_shapes) in &glyphs[i + 1..] {
                assert!(a_shapes != b_shapes, "{a:?} and {b:?} draw the same shapes");
            }
        }
    }

    #[test]
    fn every_glyph_stays_inside_its_square() {
        // Odd offsets, and sizes from a half-size icon at ppp 1 to a double-size one at ppp 2.
        for square in [
            Rect::from_min_size(pos2(13.0, 7.0), vec2(42.5, 42.5)),
            Rect::from_min_size(pos2(0.5, 3.25), vec2(21.0, 21.0)),
            Rect::from_min_size(pos2(101.0, 0.0), vec2(170.0, 170.0)),
        ] {
            for id in IconId::ALL {
                for shape in icon_shapes(id, square, Color32::WHITE) {
                    let bounds = shape.visual_bounding_rect();
                    assert!(
                        square.expand(1e-3).contains_rect(bounds),
                        "{id:?}: {bounds:?} leaves {square:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn glyphs_use_only_the_tint() {
        let tint = Color32::from_rgb(1, 2, 3);
        let square = Rect::from_min_size(pos2(0.0, 0.0), vec2(48.0, 48.0));
        for id in IconId::ALL {
            for shape in icon_shapes(id, square, tint) {
                let colors = match &shape {
                    Shape::Rect(r) => vec![r.fill, r.stroke.color],
                    Shape::Circle(c) => vec![c.fill, c.stroke.color],
                    Shape::Path(p) => match p.stroke.color {
                        egui::epaint::ColorMode::Solid(c) => vec![p.fill, c],
                        _ => panic!("{id:?}: gradient stroke"),
                    },
                    other => panic!("{id:?}: unexpected shape {other:?}"),
                };
                for c in colors {
                    assert!(c == tint || c == Color32::TRANSPARENT, "{id:?} uses {c:?}");
                }
            }
        }
    }

    #[test]
    fn the_transport_icons_read_at_their_actual_size() {
        // The mixer and keyboard-settings buttons draw at 0.7 × 4/3 U: 0.6 px a unit at U = 32 px. The faders keep
        // every part at least 3 units (about 2 px) across.
        let square = Rect::from_min_size(Pos2::ZERO, Vec2::splat(GRID));
        let bounds = |id| -> Vec<Rect> {
            (icon_shapes(id, square, Color32::WHITE).iter())
                .map(Shape::visual_bounding_rect)
                .collect()
        };
        for b in bounds(IconId::Mixer) {
            assert!(b.width().min(b.height()) >= 3.0, "{b:?}");
        }
        // Piano keys under a gear: the keys span most of the width, and the gear sits clear above them.
        let (gear, keys): (Vec<_>, Vec<_>) = (bounds(IconId::KeyboardSettings).into_iter())
            .partition(|b| b.center().y < GRID / 2.0 - 2.0);
        let union = |rs: Vec<Rect>| rs.into_iter().fold(Rect::NOTHING, |a, b| a.union(b));
        let (gear, keys) = (union(gear), union(keys));
        assert!(keys.width() >= 36.0, "{keys:?}");
        assert!(gear.width() >= 18.0, "{gear:?}");
        assert!(keys.top() - gear.bottom() >= 2.0, "{gear:?} over {keys:?}");
    }

    /// The render harness, not a check: rasterises the F2 debug sheet in software at ppp 1, 2 and 4 into
    /// `$ICON_RENDER_DIR/icons-ppp<n>.ppm`. Run with
    /// `ICON_RENDER_DIR=<dir> cargo test --lib render_icon_sheets -- --ignored`.
    #[test]
    #[ignore]
    fn render_icon_sheets() {
        use egui::epaint::{Primitive, TextureId};
        use std::collections::HashMap;

        let dir = std::env::var("ICON_RENDER_DIR").expect("set ICON_RENDER_DIR");
        let colors = crate::theme::theme_by_id(0).colors;
        for ppp in [1.0f32, 2.0, 4.0] {
            let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
            let area = Rect::from_min_size(Pos2::ZERO, vec2(m.u(16.0), m.u(15.0)));
            let ctx = egui::Context::default();
            crate::fonts::install(&ctx);
            let mut raw = egui::RawInput {
                screen_rect: Some(area),
                ..Default::default()
            };
            raw.viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .native_pixels_per_point = Some(ppp);

            // Two frames: the first one loads the fonts.
            let mut textures: HashMap<TextureId, ([usize; 2], Vec<Color32>)> = HashMap::new();
            let mut shapes = Vec::new();
            for _ in 0..2 {
                let out = ctx.run(raw.clone(), |ctx| {
                    let (fg, bg) = (colors.text.to_color32(), colors.background.to_color32());
                    draw_debug_sheet(&ctx.debug_painter(), area, fg, bg, &m);
                });
                for (id, delta) in out.textures_delta.set {
                    let egui::ImageData::Color(image) = &delta.image;
                    let [w, h] = image.size;
                    match delta.pos {
                        None => {
                            textures.insert(id, (image.size, image.pixels.clone()));
                        }
                        Some([x0, y0]) => {
                            let (size, pixels) =
                                textures.get_mut(&id).expect("patch of a new texture");
                            for y in 0..h {
                                let row = (y0 + y) * size[0] + x0;
                                pixels[row..row + w]
                                    .copy_from_slice(&image.pixels[y * w..(y + 1) * w]);
                            }
                        }
                    }
                }
                shapes = out.shapes;
            }

            // Premultiplied "over" blending of every triangle, sampled at pixel centres.
            let (w, h) = (
                (area.width() * ppp) as usize,
                (area.height() * ppp) as usize,
            );
            let mut image = vec![[0.0f32; 3]; w * h];
            let rgba = |c: Color32| c.to_array().map(|v| f32::from(v) / 255.0);
            let cross = |a: Pos2, b: Pos2, c: Pos2| (b - a).x * (c - a).y - (b - a).y * (c - a).x;
            for prim in ctx.tessellate(shapes, ppp) {
                let Primitive::Mesh(mesh) = prim.primitive else {
                    continue;
                };
                let clip = (prim.clip_rect * ppp)
                    .intersect(Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32)));
                let (tex_size, tex) = &textures[&mesh.texture_id];
                for tri in mesh.indices.as_chunks::<3>().0 {
                    let v = [0, 1, 2].map(|k| mesh.vertices[tri[k] as usize]);
                    let p = v.map(|v| (v.pos.to_vec2() * ppp).to_pos2());
                    let area2 = cross(p[0], p[1], p[2]);
                    if area2 == 0.0 {
                        continue;
                    }
                    let bounds = Rect::from_points(&p).intersect(clip);
                    if !bounds.is_positive() {
                        continue;
                    }
                    for y in bounds.min.y.floor() as usize..bounds.max.y.ceil() as usize {
                        for x in bounds.min.x.floor() as usize..bounds.max.x.ceil() as usize {
                            let c = pos2(x as f32 + 0.5, y as f32 + 0.5);
                            let b = [
                                cross(p[1], p[2], c),
                                cross(p[2], p[0], c),
                                cross(p[0], p[1], c),
                            ]
                            .map(|e| e / area2);
                            if b.iter().any(|&e| e < 0.0) {
                                continue;
                            }
                            let uv = v[0].uv.to_vec2() * b[0]
                                + v[1].uv.to_vec2() * b[1]
                                + v[2].uv.to_vec2() * b[2];
                            let texel = |i: usize, n: f32| {
                                ((n * tex_size[i] as f32) as usize).min(tex_size[i] - 1)
                            };
                            let t = rgba(tex[texel(1, uv.y) * tex_size[0] + texel(0, uv.x)]);
                            let vc = v.map(|v| rgba(v.color));
                            let src: [f32; 4] = std::array::from_fn(|i| {
                                (vc[0][i] * b[0] + vc[1][i] * b[1] + vc[2][i] * b[2]) * t[i]
                            });
                            let dst = &mut image[y * w + x];
                            for i in 0..3 {
                                dst[i] = src[i] + dst[i] * (1.0 - src[3]);
                            }
                        }
                    }
                }
            }
            let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
            ppm.extend(
                image
                    .iter()
                    .flatten()
                    .map(|&v| (v.clamp(0.0, 1.0) * 255.0).round() as u8),
            );
            std::fs::write(format!("{dir}/icons-ppp{ppp}.ppm"), ppm).expect("write the sheet");
        }
    }
}

//! Issue #68: a built-in Lemon plugin's editor full screen, from its rack card's full screen button. It shows as large
//! as the screen holds it, its aspect kept and centred, over the dimmed screen, painted at that size and driven as in
//! its card ([`lemon_card::show`]): taps, drags, a second finger, double taps, the wheel and its file requests (the
//! sample picker opens over it). The close X or `close` (Escape, Back) closes it, once the editor's own menu or type-in
//! box is gone.

use eframe::egui::{self, Area, Id, LayerId, Order, Rect, Sense, Vec2, pos2, vec2};

use super::{AppCtx, Nav, Overlay};
use crate::icons::{IconId, draw_icon};
use crate::ui::anim;
use crate::ui::buttons::filled_circle_button;
use crate::ui::rack::lemon_card::{self, LemonCard};

/// The close X's disc (U): the corner logo's (`main_screen`), 2.1 U in from the screen's right and 0.2 U down, which it
/// covers.
const CLOSE: f32 = 1.5;
const CLOSE_AT: (f32, f32) = (2.1, 0.2);

/// The editor shown: its texture and the pointer's.
#[derive(Default)]
pub struct State {
    card: LemonCard,
}

/// Shows the editor of module `module` on `channel` full screen. `Nav::Close` once it closes, or once the module or
/// its running plugin is gone.
pub fn show(
    state: &mut State,
    ui: &egui::Ui,
    app: &mut AppCtx,
    channel: usize,
    module: i32,
) -> Option<Nav> {
    let ch = app.song.channels.get(channel);
    let plugin = (ch.and_then(|c| c.rack.get(module))).and_then(crate::ui::rack::lemon_plugin);
    let (Some(ch), Some(plugin)) = (ch, plugin) else {
        return Some(Nav::Close);
    };
    let ctx = ui.ctx().clone();
    if crate::ui::close_pressed(&ctx) && !lemon_card::dismiss(&state.card) {
        return close(state);
    }
    let (m, c) = (app.cx.m, &app.cx.theme.colors);
    let (id, screen, bg) = (
        Id::new("lemon full screen"),
        app.regions.screen,
        c.background.to_color32(),
    );
    crate::ui::backdrop(&ctx, id);
    let dim = ctx.layer_painter(LayerId::new(Order::Middle, id.with("backdrop")));
    dim.rect_filled(screen, 0.0, anim::dim(bg, anim::DIM_ALPHA));
    // The editor between the close X's column and its mirror on the left.
    let disc = Rect::from_min_size(
        pos2(
            screen.right() - m.u(CLOSE_AT.0),
            screen.top() + m.u(CLOSE_AT.1),
        ),
        Vec2::splat(m.u(CLOSE)),
    );
    let area = screen.shrink2(vec2(m.u(CLOSE_AT.0), 0.0));
    let rect = lemon_card::fit(area, lemon_card::shown_size(Some(&state.card), plugin));
    let (mut running, mut closed) = (true, false);
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .fade_in(false)
        .show(&ctx, |ui| {
            ui.allocate_exact_size(screen.size(), Sense::hover());
            running = lemon_card::show(ui, rect, ch.id, module, plugin, &mut state.card, m);
            // The busy screen's abort: a `background` disc, the X in `text`.
            let x = filled_circle_button(ui, id.with("close"), disc.center(), disc.width(), bg);
            draw_icon(
                ui.painter(),
                disc,
                IconId::Close,
                c.text.to_color32(),
                0.7,
                m,
            );
            closed = x.clicked();
        });
    if lemon_card::wants_file(&mut state.card) {
        return Some(Nav::Open(Overlay::ModuleSamplePicker { channel, module }));
    }
    if closed || !running {
        return close(state);
    }
    None
}

/// Closes it, letting go of its editor's texture.
fn close(state: &mut State) -> Option<Nav> {
    *state = State::default();
    Some(Nav::Close)
}

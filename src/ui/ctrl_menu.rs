//! The `•••` CTRL menu of the transport bar: an inverted popup of seven items acting on the most recently touched
//! parameter, the "recent control", but for `Add MIDI CC track…`, which acts on the selected channel. Disabled items
//! stay in place, dimmed.
//!
//! An item is enabled when its condition holds **and** the caller lists it as implemented: [`IMPLEMENTED`], every
//! item now: the three MIDI CC link items, the three automation items and `Fine tune control`
//! ([`crate::ui::fine_tune`]). The popup sits right above the CTRL button, centred on it and kept on screen.
//!
//! The recent control comes from the views as [`Touch`]es: a bound control's press, or its parameter menu's pick of
//! one of these items, which the app acts on as on this menu's.

use eframe::egui::{Context, Id, Rect};

use crate::song::{AutomationTarget, LaneKind, Song};
use crate::ui::UiCtx;
use crate::ui::popup::{self, PopupItem};
use crate::ui::widgets::Signal;

/// The menu's items, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtrlAction {
    LinkMidiCc,
    LinkModWheel,
    UnlinkMidiCc,
    AddAutomationTrack,
    /// `Add MIDI CC track…`: a picker of the CCs, for a lane on the selected channel. Not about the recent control.
    AddCcTrack,
    LinkRecentControl,
    FineTune,
}

/// Its first four rows are also the parameter menu's rows 1…4, its last the parameter menu's `Fine tune control`.
pub(crate) const ITEMS: [(CtrlAction, &str); 7] = [
    (CtrlAction::LinkMidiCc, "Link to MIDI CC"),
    (CtrlAction::LinkModWheel, "Link to Mod Wheel"),
    (CtrlAction::UnlinkMidiCc, "Unlink from MIDI CC"),
    (CtrlAction::AddAutomationTrack, "Add automation track"),
    (CtrlAction::AddCcTrack, "Add MIDI CC track…"),
    (CtrlAction::LinkRecentControl, "Link to recent control"),
    (CtrlAction::FineTune, "Fine tune control"),
];

/// The actions the app carries out; the others stay disabled.
pub const IMPLEMENTED: [CtrlAction; 7] = [
    CtrlAction::LinkMidiCc,
    CtrlAction::LinkModWheel,
    CtrlAction::UnlinkMidiCc,
    CtrlAction::AddAutomationTrack,
    CtrlAction::AddCcTrack,
    CtrlAction::LinkRecentControl,
    CtrlAction::FineTune,
];

/// A press on a control bound to a parameter, or its parameter menu's pick of a CTRL item (which makes the control
/// recent first): what the app's recent control follows. Its channel is the reporter's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Touch {
    pub target: AutomationTarget,
    /// The parameter menu's pick, for the app to act on as on the CTRL menu's; `None`: the control was pressed.
    pub pick: Option<CtrlAction>,
}

impl Touch {
    /// What `signal`, the control's for `target`, reports, if anything.
    pub fn new(target: AutomationTarget, signal: Signal) -> Option<Self> {
        (signal.pressed || signal.pick.is_some()).then_some(Self {
            target,
            pick: signal.pick,
        })
    }
}

/// What the menu needs to know about the recent control to enable its items.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecentControl {
    /// It is a channel MIDI-CC pseudo-parameter (channel parameters 4…133): it cannot be linked.
    pub channel_cc: bool,
    /// It has a MIDI CC link.
    pub cc_linked: bool,
    /// It belongs to the current channel.
    pub on_current_channel: bool,
    /// The current track is an automation track that automates a different parameter.
    pub track_automates_other: bool,
    /// The fine tune dialog supports it ([`crate::ui::fine_tune::param`]).
    pub fine_tune: bool,
}

/// The enable condition for `action`, with `recent` the recent control (`None`: there is none) and `cc_channel` whether
/// the selected channel takes a MIDI CC track ([`cc_channel`]).
fn allowed(action: CtrlAction, recent: Option<&RecentControl>, cc_channel: bool) -> bool {
    let Some(r) = recent else {
        return action == CtrlAction::AddCcTrack && cc_channel;
    };
    let linkable = !r.channel_cc;
    match action {
        CtrlAction::LinkMidiCc | CtrlAction::LinkModWheel => linkable,
        CtrlAction::UnlinkMidiCc => linkable && r.cc_linked,
        CtrlAction::AddAutomationTrack => true,
        CtrlAction::AddCcTrack => cc_channel,
        CtrlAction::LinkRecentControl => {
            linkable && r.on_current_channel && r.track_automates_other
        }
        CtrlAction::FineTune => r.fine_tune,
    }
}

/// The menu's items for the recent control and the selected channel (`cc_channel`, [`allowed`]): one column, each enabled
/// when its condition holds and it is in `implemented`, the actions whose feature exists.
pub fn menu_items(
    recent: Option<&RecentControl>,
    cc_channel: bool,
    implemented: &[CtrlAction],
) -> Vec<PopupItem<CtrlAction>> {
    ITEMS
        .iter()
        .enumerate()
        .map(|(row, &(action, label))| PopupItem {
            disabled: !(allowed(action, recent, cc_channel) && implemented.contains(&action)),
            ..PopupItem::new(action, 0, row, label)
        })
        .collect()
}

/// Whether channel `channel` takes `Add MIDI CC track…`: an instrument or drum channel (its main lane notes or drums),
/// whose instruments the CCs play to.
pub fn cc_channel(song: &Song, channel: usize) -> bool {
    let main = song.channels.get(channel).and_then(|c| c.lanes.first());
    main.is_some_and(|l| matches!(l.kind, LaneKind::Notes | LaneKind::Drums))
}

/// Whether the CTRL menu is open (the transport opens it when CTRL is tapped), and what it tells the app.
#[derive(Debug, Default)]
pub struct CtrlMenu {
    pub open: bool,
    /// The recent control, `None` when there is none: the app sets it before the frame.
    pub recent: Option<RecentControl>,
    /// The action picked: the app takes it after the frame.
    pub picked: Option<CtrlAction>,
}

impl CtrlMenu {
    /// Shows the menu while it is open, its bottom on the top of `button` (the CTRL button's rect) and centred
    /// on it. Returns the action picked this frame; a pick, a tap outside or Escape closes it.
    pub fn show(
        &mut self,
        ctx: &Context,
        button: Rect,
        items: &[PopupItem<CtrlAction>],
        cx: &UiCtx,
    ) -> Option<CtrlAction> {
        if !self.open {
            return None;
        }
        // Placement keeps the popup inside its bounds: ending them at the button's top puts it on the button.
        let bounds = ctx.content_rect().with_max_y(button.top());
        let result = popup::show(
            ctx,
            Id::new("ctrl_menu"),
            button.center(),
            bounds,
            items,
            cx,
        );
        if result.close {
            self.open = false;
            ctx.request_repaint();
        }
        result.selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::offscreen::{Offscreen, SCREEN};
    use eframe::egui::{Pos2, pos2, vec2};

    use CtrlAction::*;

    const ALL: [CtrlAction; 7] = [
        LinkMidiCc,
        LinkModWheel,
        UnlinkMidiCc,
        AddAutomationTrack,
        AddCcTrack,
        LinkRecentControl,
        FineTune,
    ];

    fn enabled(recent: Option<&RecentControl>) -> Vec<CtrlAction> {
        menu_items(recent, false, &ALL)
            .into_iter()
            .filter(|i| !i.disabled)
            .filter_map(|i| i.id)
            .collect()
    }

    #[test]
    fn seven_items_in_the_spec_order_in_one_column() {
        let items = menu_items(None, false, &ALL);
        let cells: Vec<_> = items
            .iter()
            .map(|i| (i.col, i.row, &*i.label, i.id))
            .collect();
        assert_eq!(
            cells,
            [
                (0, 0, "Link to MIDI CC", Some(LinkMidiCc)),
                (0, 1, "Link to Mod Wheel", Some(LinkModWheel)),
                (0, 2, "Unlink from MIDI CC", Some(UnlinkMidiCc)),
                (0, 3, "Add automation track", Some(AddAutomationTrack)),
                (0, 4, "Add MIDI CC track…", Some(AddCcTrack)),
                (0, 5, "Link to recent control", Some(LinkRecentControl)),
                (0, 6, "Fine tune control", Some(FineTune)),
            ]
        );
        assert!(items.iter().all(|i| !i.dot && !i.keep_open && !i.current));
    }

    #[test]
    fn without_a_recent_control_everything_is_disabled() {
        assert_eq!(enabled(None), []);
        // But `Add MIDI CC track…`, on an instrument or drum channel: it acts on the selected channel.
        let cc = menu_items(None, true, &ALL);
        let on: Vec<_> = (cc.iter().filter(|i| !i.disabled))
            .filter_map(|i| i.id)
            .collect();
        assert_eq!(on, [AddCcTrack]);
    }

    #[test]
    fn the_enable_rules_follow_the_spec() {
        let plain = RecentControl::default();
        assert_eq!(
            enabled(Some(&plain)),
            [LinkMidiCc, LinkModWheel, AddAutomationTrack]
        );
        let linked = RecentControl {
            cc_linked: true,
            ..plain
        };
        assert_eq!(
            enabled(Some(&linked)),
            [LinkMidiCc, LinkModWheel, UnlinkMidiCc, AddAutomationTrack]
        );
        // A channel MIDI-CC parameter cannot be linked, unlinked or retargeted to.
        let cc = RecentControl {
            channel_cc: true,
            cc_linked: true,
            on_current_channel: true,
            track_automates_other: true,
            fine_tune: true,
        };
        assert_eq!(enabled(Some(&cc)), [AddAutomationTrack, FineTune]);
        // Retargeting needs both: the current channel, and an automation track on another parameter.
        for (on_current_channel, track_automates_other, want) in [
            (true, false, false),
            (false, true, false),
            (true, true, true),
        ] {
            let r = RecentControl {
                on_current_channel,
                track_automates_other,
                ..plain
            };
            assert_eq!(enabled(Some(&r)).contains(&LinkRecentControl), want);
        }
    }

    #[test]
    fn items_without_their_feature_are_disabled() {
        let everything = RecentControl {
            cc_linked: true,
            on_current_channel: true,
            track_automates_other: true,
            fine_tune: true,
            ..Default::default()
        };
        let some = menu_items(Some(&everything), true, &[FineTune, LinkModWheel]);
        let on: Vec<_> = some
            .iter()
            .filter(|i| !i.disabled)
            .filter_map(|i| i.id)
            .collect();
        assert_eq!(on, [LinkModWheel, FineTune]);
        // Nothing implemented: everything disabled.
        assert!(
            menu_items(Some(&everything), true, &[])
                .iter()
                .all(|i| i.disabled)
        );
    }

    #[test]
    fn the_app_implements_every_item() {
        let on = |recent: &RecentControl| -> Vec<_> {
            menu_items(Some(recent), false, &IMPLEMENTED)
                .into_iter()
                .filter(|i| !i.disabled)
                .filter_map(|i| i.id)
                .collect()
        };
        let plain = RecentControl::default();
        assert_eq!(on(&plain), [LinkMidiCc, LinkModWheel, AddAutomationTrack]);
        let linked = RecentControl {
            cc_linked: true,
            ..plain
        };
        assert_eq!(
            on(&linked),
            [LinkMidiCc, LinkModWheel, UnlinkMidiCc, AddAutomationTrack]
        );
        // A channel MIDI-CC pseudo-parameter cannot be linked (the fine tune dialog never supports one; the flag is
        // independent here).
        let cc = RecentControl {
            channel_cc: true,
            fine_tune: true,
            ..linked
        };
        assert_eq!(on(&cc), [AddAutomationTrack, FineTune]);
        // `Fine tune control` follows the dialog's support, and so does every item.
        let tunable = RecentControl {
            fine_tune: true,
            on_current_channel: true,
            track_automates_other: true,
            ..linked
        };
        let all = menu_items(Some(&tunable), true, &IMPLEMENTED);
        assert!(all.iter().all(|i| !i.disabled));
        assert!(IMPLEMENTED.iter().all(|a| ALL.contains(a)));
    }

    #[test]
    fn a_touch_is_a_press_or_a_parameter_menu_pick() {
        let target = AutomationTarget {
            instance_id: 2,
            param: 1,
        };
        let signal = |pressed, pick| Signal { pressed, pick };
        assert_eq!(Touch::new(target, signal(false, None)), None);
        let touch = |pick| Some(Touch { target, pick });
        assert_eq!(Touch::new(target, signal(true, None)), touch(None));
        for pick in [AddAutomationTrack, LinkMidiCc, LinkModWheel, UnlinkMidiCc] {
            assert_eq!(
                Touch::new(target, signal(false, Some(pick))),
                touch(Some(pick))
            );
            assert_eq!(
                Touch::new(target, signal(true, Some(pick))),
                touch(Some(pick))
            );
        }
    }

    /// The CTRL button: a 1.75 U slot at the bottom of the screen.
    const BUTTON: Rect = Rect {
        min: pos2(700.0, 440.0),
        max: pos2(756.0, 496.0),
    };

    struct Rig {
        screen: Offscreen,
        menu: CtrlMenu,
        items: Vec<PopupItem<CtrlAction>>,
        picked: Vec<CtrlAction>,
    }

    impl Rig {
        fn new(items: Vec<PopupItem<CtrlAction>>) -> Self {
            let mut rig = Self {
                screen: Offscreen::new(),
                menu: CtrlMenu {
                    open: true,
                    ..CtrlMenu::default()
                },
                items,
                picked: Vec::new(),
            };
            for _ in 0..3 {
                rig.frame_at(None);
            }
            rig
        }

        fn frame_at(&mut self, click: Option<Pos2>) {
            let (menu, items, picked) = (&mut self.menu, &self.items, &mut self.picked);
            let mut f = |ui: &mut eframe::egui::Ui, cx: &UiCtx| {
                picked.extend(menu.show(ui.ctx(), BUTTON, items, cx));
            };
            match click {
                Some(pos) => self.screen.click(pos, &mut f),
                None => {
                    self.screen.frame(vec![], &mut f);
                }
            }
        }

        /// The popup's rect, from the painted fill; `None` when nothing is painted.
        fn popup_rect(&mut self) -> Option<Rect> {
            let (menu, items) = (&mut self.menu, &self.items);
            let shapes = self.screen.frame(vec![], &mut |ui, cx| {
                menu.show(ui.ctx(), BUTTON, items, cx);
            });
            let fill = crate::theme::theme_by_id(0).colors.text.to_color32();
            shapes.iter().find_map(|s| match &s.shape {
                eframe::egui::Shape::Rect(r) if r.fill == fill => Some(r.rect),
                _ => None,
            })
        }
    }

    #[test]
    fn the_popup_stands_on_the_button_centred_on_it() {
        let mut rig = Rig::new(menu_items(None, false, &[]));
        let r = rig.popup_rect().unwrap();
        assert_eq!(r.bottom(), BUTTON.top());
        assert!((r.center().x - BUTTON.center().x).abs() <= 1.0, "{r:?}");
        // Seven rows of 1.25 U.
        assert_eq!(r.height(), 7.0 * 40.0);
        assert!(SCREEN.contains_rect(r));
    }

    #[test]
    fn picking_an_enabled_item_returns_its_action_and_closes() {
        let mut items = menu_items(None, false, &[]);
        items[3].disabled = false;
        let mut rig = Rig::new(items);
        let r = rig.popup_rect().unwrap();
        // Row 3 of seven: Add automation track.
        rig.frame_at(Some(pos2(r.center().x, r.top() + 3.5 * 40.0)));
        assert_eq!(rig.picked, [AddAutomationTrack]);
        assert!(!rig.menu.open);
    }

    #[test]
    fn disabled_items_do_nothing_and_a_tap_outside_closes() {
        let mut rig = Rig::new(menu_items(None, false, &[]));
        let r = rig.popup_rect().unwrap();
        rig.frame_at(Some(pos2(r.center().x, r.top() + 0.5 * 40.0)));
        assert!(rig.picked.is_empty() && rig.menu.open);
        rig.frame_at(Some(r.left_top() - vec2(20.0, 20.0)));
        assert!(rig.picked.is_empty() && !rig.menu.open);
        // Closed: nothing is drawn.
        assert_eq!(rig.popup_rect(), None);
    }
}

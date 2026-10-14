//! MIDI input devices: the input device menu and the [`MidiLink`] seam. The link holds an opener and a lister: `midir`
//! natively, Web MIDI on the web, fakes in tests (no test opens a port). Devices push raw messages into a channel that
//! The app drains each frame.

use std::any::Any;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use eframe::egui::Context;

#[cfg(any(not(target_os = "android"), test))]
use super::MidiMsg;

/// The menu item that opens no device.
pub const NO_INPUT: &str = "<no midi input>";
/// The menu item that connects every device.
pub const ANY: &str = "Any available";
/// how often the ports are listed again while a choice can gain or lose ports.
pub const RESCAN_SECS: f64 = 2.0;
/// Messages held for the app; more are dropped until it drains.
const QUEUE: usize = 4096;

/// An `Input device:` pick. Stored by its [label](Self::label) (`MidiDevice`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MidiChoice {
    None,
    #[default]
    Any,
    /// A device by its [`device_name`].
    Named(String),
}

impl MidiChoice {
    pub fn label(&self) -> &str {
        match self {
            Self::None => NO_INPUT,
            Self::Any => ANY,
            Self::Named(name) => name,
        }
    }

    pub fn from_label(label: &str) -> Self {
        match label {
            NO_INPUT => Self::None,
            ANY => Self::Any,
            name => Self::Named(name.to_owned()),
        }
    }
}

/// A port's device name: the port name without the ` <client>:<port>` numbers ALSA appends, which change when a
/// device is plugged in again.
pub fn device_name(port: &str) -> &str {
    let numbers = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match port.rsplit_once(' ') {
        Some((name, ids))
            if ids
                .split_once(':')
                .is_some_and(|(c, p)| numbers(c) && numbers(p)) =>
        {
            name
        }
        _ => port,
    }
}

/// The `Input device:` menu: `<no midi input>`, `Any available`, then each device once, by name.
pub fn menu(ports: &[String]) -> Vec<MidiChoice> {
    let mut items = vec![MidiChoice::None, MidiChoice::Any];
    for port in ports {
        let named = MidiChoice::Named(device_name(port).to_owned());
        if !items.contains(&named) {
            items.push(named);
        }
    }
    items
}

/// The indices of the ports a choice connects: every port for `Any`, the first with the name for `Named`.
fn chosen(choice: &MidiChoice, ports: &[String]) -> Vec<usize> {
    let mut indices = (0..ports.len()).filter(|&i| match choice {
        MidiChoice::None => false,
        MidiChoice::Any => true,
        MidiChoice::Named(name) => device_name(&ports[i]) == name,
    });
    match choice {
        MidiChoice::Named(_) => indices.next().into_iter().collect(),
        _ => indices.collect(),
    }
}

/// Connects each of a choice's ports ([`chosen`]), skipping the ones that fail. An error only when a named device
/// is missing, or when ports were tried and none connected (no ports for `Any` is no error).
#[cfg(any(not(target_os = "android"), test))]
fn connect_each<C>(
    choice: &MidiChoice,
    ports: &[String],
    mut connect: impl FnMut(usize) -> Result<C, String>,
) -> Result<Vec<C>, String> {
    let wanted = chosen(choice, ports);
    if let MidiChoice::Named(name) = choice
        && wanted.is_empty()
    {
        return Err(format!("no MIDI input named {name}"));
    }
    let mut errors = Vec::new();
    let connections: Vec<C> = wanted
        .into_iter()
        .filter_map(|i| connect(i).map_err(|err| errors.push(err)).ok())
        .collect();
    if connections.is_empty() && !errors.is_empty() {
        return Err(errors.join("; "));
    }
    Ok(connections)
}

/// Opens the ports of a choice. Each message goes to the sender; the context, if any, repaints. The box keeps the
/// connections open until it is dropped.
type Opener = Box<
    dyn FnMut(&MidiChoice, SyncSender<Vec<u8>>, Option<Context>) -> Result<Box<dyn Any>, String>,
>;
/// Lists the input ports by name.
type Lister = Box<dyn FnMut() -> Vec<String>>;

/// The app's MIDI input: the chosen device's open ports and the messages they sent.
pub struct MidiLink {
    open: Opener,
    list: Lister,
    ctx: Option<Context>,
    tx: SyncSender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    choice: MidiChoice,
    /// The ports at the last scan.
    ports: Vec<String>,
    /// The open connections; `None` when the last open failed or nothing is chosen.
    conn: Option<Box<dyn Any>>,
    /// When [`Self::tick`] last listed the ports; `None` until the first tick after an open (which just listed).
    scanned: Option<f64>,
    /// See [`Self::reopened`].
    reopened: u32,
}

impl MidiLink {
    pub fn new(
        open: impl FnMut(
            &MidiChoice,
            SyncSender<Vec<u8>>,
            Option<Context>,
        ) -> Result<Box<dyn Any>, String>
        + 'static,
        list: impl FnMut() -> Vec<String> + 'static,
        ctx: Option<Context>,
    ) -> Self {
        let (tx, rx) = sync_channel(QUEUE);
        Self {
            open: Box::new(open),
            list: Box::new(list),
            ctx,
            tx,
            rx,
            choice: MidiChoice::None,
            ports: Vec::new(),
            conn: None,
            scanned: None,
            reopened: 0,
        }
    }

    /// The system's link: `midir` natively, Web MIDI on the web. Nothing opens until [`Self::open`].
    pub fn platform(ctx: Context) -> Self {
        Self::new(backend::open, backend::lister(), Some(ctx))
    }

    /// Counts every close-and-reopen: each [`Self::open`], and each rescan that found the chosen ports changed
    /// (a device unplugged, plugged in, or plugged in again) or retried a failed open. When it changes, keys held
    /// on the old connection may never get their note-offs, so the app calls `Router::release_all` then.
    pub fn reopened(&self) -> u32 {
        self.reopened
    }

    pub fn choice(&self) -> &MidiChoice {
        &self.choice
    }

    /// The ports at the last scan, for [`menu`].
    pub fn ports(&self) -> &[String] {
        &self.ports
    }

    /// Closes the open ports, lists the ports and opens the choice's (none for `<no midi input>`). On failure
    /// nothing is open, and [`Self::tick`] retries. The next scan is [`RESCAN_SECS`] after the next tick.
    ///
    /// Natively the app opens at start. On the web the first open asks for MIDI access, so it must run from
    /// a user gesture (a click or key press handled that frame), never at startup.
    pub fn open(&mut self, choice: MidiChoice) -> Result<(), String> {
        self.choice = choice;
        self.ports = (self.list)();
        self.scanned = None;
        self.connect()
    }

    /// Lists the ports again. The app calls it once each time the `Input device:` dropdown opens (not every frame
    /// it stays open); [`Self::tick`] calls it every [`RESCAN_SECS`]. Reopens the choice when its ports came or
    /// went (by raw name, so an ALSA device plugged in again reconnects), or when its last open failed.
    pub fn rescan(&mut self) {
        let ports = (self.list)();
        let names = |ports: &[String]| -> Vec<String> {
            chosen(&self.choice, ports)
                .into_iter()
                .map(|i| ports[i].clone())
                .collect()
        };
        let changed = names(&ports) != names(&self.ports);
        self.ports = ports;
        if changed || (self.conn.is_none() && self.choice != MidiChoice::None) {
            // A failed reopen is retried at the next rescan; there is nobody to tell.
            let _ = self.connect();
        }
    }

    /// Call every frame with the egui time: rescans every [`RESCAN_SECS`] while a device is chosen (
    /// a named device too, since unplugging one is noticed only by listing), and schedules the frame that does.
    pub fn tick(&mut self, now: f64) {
        if self.choice == MidiChoice::None {
            return;
        }
        let scanned = *self.scanned.get_or_insert(now);
        if now - scanned >= RESCAN_SECS {
            self.scanned = Some(now);
            self.rescan();
        }
        if let (Some(ctx), Some(scanned)) = (&self.ctx, self.scanned) {
            ctx.request_repaint_after_secs((scanned + RESCAN_SECS - now) as f32);
        }
    }

    /// The messages received since the last call, oldest first: one complete message each.
    pub fn drain(&mut self) -> Vec<Vec<u8>> {
        self.rx.try_iter().collect()
    }

    fn connect(&mut self) -> Result<(), String> {
        // The old ports close before any reopen: a backend may open a port only once.
        self.conn = None;
        self.reopened = self.reopened.wrapping_add(1);
        if self.choice != MidiChoice::None {
            self.conn = Some((self.open)(
                &self.choice,
                self.tx.clone(),
                self.ctx.clone(),
            )?);
        }
        Ok(())
    }
}

/// Hands one message from a device callback to the link, and wakes the UI. Only what [`MidiMsg::parse`] reads is
/// kept: system messages (SysEx, clock, active sensing), poly pressure and malformed bytes are skipped.
#[cfg(any(not(target_os = "android"), test))]
fn forward(bytes: &[u8], tx: &SyncSender<Vec<u8>>, ctx: Option<&Context>) {
    // ponytail: a full queue (an app that stopped drawing frames) drops new messages; a ring that keeps the newest
    // if that ever matters.
    if MidiMsg::parse(bytes).is_some() && tx.try_send(bytes.to_vec()).is_ok() {
        // ponytail: the message waits for the next egui frame; a direct path to the engine if latency matters.
        if let Some(ctx) = ctx {
            ctx.request_repaint();
        }
    }
}

/// `midir`: `Any` connects every port, `Named` the first port of that device.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
mod backend {
    use std::any::Any;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::mpsc::SyncSender;

    use eframe::egui::Context;
    use midir::{Ignore, MidiInput};

    use super::{MidiChoice, connect_each, forward};

    /// Lists through one cached client (`ports()` asks the system each time); a failed listing drops it.
    pub fn lister() -> impl FnMut() -> Vec<String> {
        let mut cached: Option<MidiInput> = None;
        move || {
            let names = guard(|| {
                if cached.is_none() {
                    cached = Some(client()?);
                }
                let Some(input) = &cached else {
                    return Ok(Vec::new());
                };
                Ok(input
                    .ports()
                    .iter()
                    .filter_map(|port| input.port_name(port).ok())
                    .collect())
            });
            names.unwrap_or_else(|_| {
                cached = None;
                Vec::new()
            })
        }
    }

    pub fn open(
        choice: &MidiChoice,
        tx: SyncSender<Vec<u8>>,
        ctx: Option<Context>,
    ) -> Result<Box<dyn Any>, String> {
        guard(|| {
            let input = client()?;
            let ports = input.ports();
            let names: Vec<String> = ports
                .iter()
                .map(|p| input.port_name(p).unwrap_or_default())
                .collect();
            let connections = connect_each(choice, &names, |i| {
                // `connect` consumes its client, so each port takes one.
                let mut input = client()?;
                input.ignore(Ignore::All);
                let (tx, ctx) = (tx.clone(), ctx.clone());
                let on_message =
                    move |_: u64, bytes: &[u8], _: &mut ()| forward(bytes, &tx, ctx.as_ref());
                let connection = input.connect(&ports[i], crate::APP_NAME, on_message, ());
                connection.map_err(|err| format!("{}: {err}", names[i]))
            })?;
            Ok(Box::new(connections) as Box<dyn Any>)
        })
    }

    fn client() -> Result<MidiInput, String> {
        MidiInput::new(crate::APP_NAME).map_err(|err| err.to_string())
    }

    /// The backend `unwrap`s some system calls; a failure there must not take the app down.
    fn guard<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        catch_unwind(AssertUnwindSafe(f))
            .unwrap_or_else(|_| Err("the MIDI backend panicked".to_owned()))
    }
}

/// Web MIDI: access is asked at the first open, then each open sets `onmidimessage` on its inputs. A refusal (or a
/// browser without Web MIDI) leaves the list empty and warns once.
#[cfg(target_arch = "wasm32")]
mod backend {
    use std::any::Any;
    use std::cell::RefCell;
    use std::rc::{Rc, Weak};
    use std::sync::mpsc::SyncSender;

    use eframe::egui::Context;
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{MidiAccess, MidiInput, MidiMessageEvent, MidiPortDeviceState};

    use super::{MidiChoice, connect_each, forward};

    enum Access {
        NotAsked,
        Asking,
        Granted(MidiAccess),
        Refused,
    }

    /// One open: the inputs it listens to, detached when the app drops it.
    struct Conn {
        choice: MidiChoice,
        ctx: Option<Context>,
        on_message: Closure<dyn FnMut(MidiMessageEvent)>,
        inputs: RefCell<Vec<MidiInput>>,
    }

    thread_local! {
        static ACCESS: RefCell<Access> = const { RefCell::new(Access::NotAsked) };
        /// The opens, re-attached when access arrives and when an input comes or goes.
        static CONNS: RefCell<Vec<Weak<Conn>>> = const { RefCell::new(Vec::new()) };
    }

    pub fn lister() -> impl FnMut() -> Vec<String> {
        || inputs().into_iter().map(|(_, name)| name).collect()
    }

    pub fn open(
        choice: &MidiChoice,
        tx: SyncSender<Vec<u8>>,
        ctx: Option<Context>,
    ) -> Result<Box<dyn Any>, String> {
        if ACCESS.with_borrow(|access| matches!(access, Access::NotAsked)) {
            ask();
        }
        let repaint = ctx.clone();
        let on_message =
            Closure::<dyn FnMut(MidiMessageEvent)>::new(move |event: MidiMessageEvent| {
                if let Ok(bytes) = event.data() {
                    forward(&bytes, &tx, repaint.as_ref());
                }
            });
        let conn = Rc::new(Conn {
            choice: choice.clone(),
            ctx,
            on_message,
            inputs: RefCell::default(),
        });
        conn.attach();
        CONNS.with_borrow_mut(|conns| conns.push(Rc::downgrade(&conn)));
        Ok(Box::new(conn))
    }

    /// The connected inputs and their names, once access is granted.
    fn inputs() -> Vec<(MidiInput, String)> {
        let Some(access) = ACCESS.with_borrow(|access| match access {
            Access::Granted(access) => Some(access.clone()),
            _ => None,
        }) else {
            return Vec::new();
        };
        let inputs = access.inputs().values().into_iter().filter_map(Result::ok);
        inputs
            .map(|value| value.unchecked_into::<MidiInput>())
            .filter(|input| input.state() == MidiPortDeviceState::Connected)
            .map(|input| {
                let name = input.name().unwrap_or_else(|| input.id());
                (input, name)
            })
            .collect()
    }

    fn ask() {
        let request = web_sys::window().map(|window| window.navigator().request_midi_access());
        let Some(Ok(promise)) = request else {
            return refuse("this browser has no Web MIDI");
        };
        ACCESS.set(Access::Asking);
        wasm_bindgen_futures::spawn_local(async move {
            match JsFuture::from(promise).await {
                Ok(access) => {
                    let access: MidiAccess = access.unchecked_into();
                    // "onstatechange re-lists the inputs": lives as long as the access, which is the page's.
                    let on_change = Closure::<dyn FnMut()>::new(refresh);
                    access.set_onstatechange(Some(on_change.as_ref().unchecked_ref()));
                    on_change.forget();
                    ACCESS.set(Access::Granted(access));
                    refresh();
                }
                Err(err) => refuse(&format!("{err:?}")),
            }
        });
    }

    fn refuse(why: &str) {
        ACCESS.set(Access::Refused);
        web_sys::console::warn_1(&format!("MIDI input unavailable: {why}").into());
    }

    /// Re-attaches every live open to the inputs there are now, and repaints so the app lists them.
    fn refresh() {
        let conns: Vec<Rc<Conn>> = CONNS.with_borrow_mut(|conns| {
            conns.retain(|conn| conn.strong_count() > 0);
            conns.iter().filter_map(Weak::upgrade).collect()
        });
        for conn in conns {
            conn.attach();
            if let Some(ctx) = &conn.ctx {
                ctx.request_repaint();
            }
        }
    }

    impl Conn {
        fn attach(&self) {
            self.detach();
            let inputs = inputs();
            let names: Vec<String> = inputs.iter().map(|(_, name)| name.clone()).collect();
            let attached = connect_each(&self.choice, &names, |i| {
                let input = &inputs[i].0;
                input.set_onmidimessage(Some(self.on_message.as_ref().unchecked_ref()));
                Ok(input.clone())
            });
            *self.inputs.borrow_mut() = attached.unwrap_or_default();
        }

        fn detach(&self) {
            for input in self.inputs.borrow_mut().drain(..) {
                input.set_onmidimessage(None);
            }
        }
    }

    impl Drop for Conn {
        /// Detached before the closure goes, so the browser never calls a dropped closure.
        fn drop(&mut self) {
            self.detach();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    fn labels(items: &[MidiChoice]) -> Vec<&str> {
        items.iter().map(MidiChoice::label).collect()
    }

    fn ports(names: &[&str]) -> Vec<String> {
        names.iter().map(|&n| n.to_owned()).collect()
    }

    #[test]
    fn the_menu_is_no_input_any_then_the_devices() {
        assert_eq!(labels(&menu(&[])), [NO_INPUT, ANY]);
        assert_eq!(menu(&[]), [MidiChoice::None, MidiChoice::Any]);
        assert_eq!(
            labels(&menu(&ports(&["Keys:Keys MIDI 1 20:0"]))),
            [NO_INPUT, ANY, "Keys:Keys MIDI 1"]
        );
        let two = menu(&ports(&["Pads", "Keys 24:0", "Keys 28:0"]));
        assert_eq!(labels(&two), [NO_INPUT, ANY, "Pads", "Keys"]);
        assert_eq!(two[3], MidiChoice::Named("Keys".to_owned()));
    }

    #[test]
    fn device_names_drop_the_alsa_port_numbers_only() {
        assert_eq!(device_name("Keys:Keys MIDI 1 20:0"), "Keys:Keys MIDI 1");
        assert_eq!(
            device_name("Midi Through:Midi Through Port-0 14:0"),
            "Midi Through:Midi Through Port-0"
        );
        for kept in ["Keys", "Port 2", "Keys 1:", "Keys :2", "Keys 1:2x", "12:0"] {
            assert_eq!(device_name(kept), kept);
        }
    }

    #[test]
    fn a_choice_round_trips_through_its_label() {
        let named = MidiChoice::Named("Keys".to_owned());
        for choice in [MidiChoice::None, MidiChoice::Any, named] {
            assert_eq!(MidiChoice::from_label(choice.label()), choice);
        }
        assert_eq!(MidiChoice::default(), MidiChoice::Any);
    }

    #[test]
    fn only_messages_the_router_reads_are_forwarded() {
        let (tx, rx) = sync_channel(QUEUE);
        for bytes in [
            &[0x90, 60, 100][..],
            &[0xF0, 0x7E, 0xF7],
            &[0xF8],
            &[0xFE],
            &[],
            &[0x40],
            &[0xA0, 60, 10],
            &[0x90, 60],
        ] {
            forward(bytes, &tx, None);
        }
        forward(&[0xB0, 7, 100], &tx, None);
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            [vec![0x90, 60, 100], vec![0xB0, 7, 100]]
        );
    }

    #[test]
    fn a_full_queue_drops_new_messages() {
        let (tx, rx) = sync_channel(2);
        for key in [60, 62, 64] {
            forward(&[0x90, key, 100], &tx, None);
        }
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            [vec![0x90, 60, 100], vec![0x90, 62, 100]]
        );
    }

    /// A fake system: the ports it lists and those that fail to connect, every opener call, every successful open
    /// (the ports it connected, and its sender), and how many connections were dropped.
    #[derive(Default)]
    struct Fake {
        ports: RefCell<Vec<String>>,
        broken: RefCell<Vec<String>>,
        lists: Cell<usize>,
        calls: Cell<usize>,
        opens: RefCell<Vec<Open>>,
        /// Drops seen by each open, when it was made.
        drops_at_open: RefCell<Vec<usize>>,
        drops: Rc<Cell<usize>>,
    }

    /// One successful open: the ports it connected, and its sender.
    type Open = (Vec<String>, SyncSender<Vec<u8>>);

    struct Conn(Rc<Cell<usize>>);

    impl Drop for Conn {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    /// A link over `fake`, whose opener connects ports as the native one does.
    fn link(fake: &Rc<Fake>) -> MidiLink {
        let (opener, lister) = (fake.clone(), fake.clone());
        MidiLink::new(
            move |choice, tx, _ctx| {
                let f = &opener;
                f.calls.set(f.calls.get() + 1);
                let ports = f.ports.borrow().clone();
                let connected = connect_each(choice, &ports, |i| {
                    if f.broken.borrow().contains(&ports[i]) {
                        Err(format!("{} is busy", ports[i]))
                    } else {
                        Ok(ports[i].clone())
                    }
                })?;
                f.drops_at_open.borrow_mut().push(f.drops.get());
                f.opens.borrow_mut().push((connected, tx));
                Ok(Box::new(Conn(f.drops.clone())) as Box<dyn Any>)
            },
            move || {
                lister.lists.set(lister.lists.get() + 1);
                lister.ports.borrow().clone()
            },
            None,
        )
    }

    fn send(fake: &Fake, open: usize, bytes: &[u8]) {
        forward(bytes, &fake.opens.borrow()[open].1, None);
    }

    fn connected(fake: &Fake, open: usize) -> Vec<String> {
        fake.opens.borrow()[open].0.clone()
    }

    #[test]
    fn drain_returns_the_messages_in_order_once() {
        let fake = Rc::new(Fake::default());
        *fake.ports.borrow_mut() = ports(&["Keys"]);
        let mut link = link(&fake);
        assert_eq!(link.drain(), Vec::<Vec<u8>>::new());
        link.open(MidiChoice::Any).unwrap();
        assert_eq!(link.ports(), ["Keys"]);
        send(&fake, 0, &[0x90, 60, 100]);
        send(&fake, 0, &[0x90, 64, 90]);
        send(&fake, 0, &[0x80, 60, 0]);
        let expected = [vec![0x90, 60, 100], vec![0x90, 64, 90], vec![0x80, 60, 0]];
        assert_eq!(link.drain(), expected);
        assert!(link.drain().is_empty());
    }

    #[test]
    fn a_reopen_drops_the_old_connection_first() {
        let fake = Rc::new(Fake::default());
        *fake.ports.borrow_mut() = ports(&["Keys", "Pads"]);
        let mut link = link(&fake);
        link.open(MidiChoice::Any).unwrap();
        link.open(MidiChoice::Named("Pads".to_owned())).unwrap();
        assert_eq!(*fake.drops_at_open.borrow(), [0, 1]);
        assert_eq!(link.choice(), &MidiChoice::Named("Pads".to_owned()));
        link.open(MidiChoice::None).unwrap();
        assert_eq!(fake.drops.get(), 2);
        assert_eq!(fake.calls.get(), 2, "no input opens nothing");
    }

    #[test]
    fn any_skips_a_port_that_fails_and_errs_only_when_none_connect() {
        let fake = Rc::new(Fake::default());
        *fake.ports.borrow_mut() = ports(&["Keys", "Pads", "Drums"]);
        *fake.broken.borrow_mut() = ports(&["Pads"]);
        let mut link = link(&fake);
        link.open(MidiChoice::Any).unwrap();
        assert_eq!(connected(&fake, 0), ["Keys", "Drums"]);
        link.tick(0.0);
        link.tick(2.0);
        link.tick(4.0);
        assert_eq!(fake.calls.get(), 1, "the same ports: no reopen every scan");
        *fake.broken.borrow_mut() = ports(&["Keys", "Pads", "Drums"]);
        assert!(link.open(MidiChoice::Any).is_err());
        fake.ports.borrow_mut().clear();
        link.open(MidiChoice::Any).unwrap();
        assert!(connected(&fake, 1).is_empty(), "no ports is no error");
    }

    #[test]
    fn a_named_device_connects_its_first_port_only() {
        let fake = Rc::new(Fake::default());
        *fake.ports.borrow_mut() = ports(&["Keys 20:0", "Pads", "Keys 24:0"]);
        let mut link = link(&fake);
        link.open(MidiChoice::Named("Keys".to_owned())).unwrap();
        assert_eq!(connected(&fake, 0), ["Keys 20:0"]);
        link.open(MidiChoice::Any).unwrap();
        assert_eq!(connected(&fake, 1), ["Keys 20:0", "Pads", "Keys 24:0"]);
    }

    #[test]
    fn reopened_counts_every_reconnect() {
        let fake = Rc::new(Fake::default());
        *fake.ports.borrow_mut() = ports(&["Keys"]);
        let mut link = link(&fake);
        assert_eq!(link.reopened(), 0);
        link.open(MidiChoice::Any).unwrap();
        assert_eq!(link.reopened(), 1);
        link.rescan();
        assert_eq!(link.reopened(), 1, "nothing changed");
        fake.ports.borrow_mut().clear();
        link.rescan();
        assert_eq!(link.reopened(), 2, "unplugged");
        link.open(MidiChoice::None).unwrap();
        assert_eq!(link.reopened(), 3);
    }

    #[test]
    fn a_missing_named_device_connects_when_a_rescan_finds_it() {
        let fake = Rc::new(Fake::default());
        let mut link = link(&fake);
        assert!(link.open(MidiChoice::Named("Keys".to_owned())).is_err());
        *fake.ports.borrow_mut() = ports(&["Keys 20:0"]);
        link.tick(0.0);
        link.tick(1.9);
        assert!(
            fake.opens.borrow().is_empty(),
            "no rescan before {RESCAN_SECS} s"
        );
        link.tick(2.0);
        assert_eq!(fake.opens.borrow().len(), 1);
        send(&fake, 0, &[0x90, 60, 100]);
        assert_eq!(link.drain(), [vec![0x90, 60, 100]]);
        // Unplugged: the connection closes and the open fails, without a panic. Plugged in again: it reconnects.
        fake.ports.borrow_mut().clear();
        link.tick(4.0);
        assert_eq!(fake.drops.get(), 1);
        *fake.ports.borrow_mut() = ports(&["Keys 24:0"]);
        link.tick(6.0);
        assert_eq!(fake.opens.borrow().len(), 2);
    }

    #[test]
    fn tick_scans_every_rescan_secs_counted_from_the_open() {
        let fake = Rc::new(Fake::default());
        *fake.ports.borrow_mut() = ports(&["Keys"]);
        let mut link = link(&fake);
        link.open(MidiChoice::Any).unwrap();
        let lists = fake.lists.get();
        link.tick(100.0);
        assert_eq!(fake.lists.get(), lists, "the open just listed");
        link.tick(101.9);
        assert_eq!(fake.lists.get(), lists);
        link.tick(102.0);
        assert_eq!(fake.lists.get(), lists + 1);
        link.tick(102.1);
        link.tick(103.9);
        assert_eq!(fake.lists.get(), lists + 1);
        link.tick(104.0);
        assert_eq!(fake.lists.get(), lists + 2);
        // A new open starts the count again.
        link.open(MidiChoice::Any).unwrap();
        link.tick(105.0);
        link.tick(106.5);
        assert_eq!(fake.lists.get(), lists + 3, "only the open listed");
        link.tick(107.0);
        assert_eq!(fake.lists.get(), lists + 4);
    }

    #[test]
    fn a_failed_open_retries_at_a_rescan_with_the_same_ports() {
        let fake = Rc::new(Fake::default());
        *fake.ports.borrow_mut() = ports(&["Keys"]);
        *fake.broken.borrow_mut() = ports(&["Keys"]);
        let mut link = link(&fake);
        assert!(link.open(MidiChoice::Any).is_err());
        link.rescan();
        assert_eq!(fake.calls.get(), 2);
        fake.broken.borrow_mut().clear();
        link.rescan();
        assert_eq!(connected(&fake, 0), ["Keys"]);
    }

    #[test]
    fn a_new_alsa_number_reopens_a_named_device() {
        let fake = Rc::new(Fake::default());
        *fake.ports.borrow_mut() = ports(&["Keys 20:0"]);
        let mut link = link(&fake);
        link.open(MidiChoice::Named("Keys".to_owned())).unwrap();
        *fake.ports.borrow_mut() = ports(&["Keys 24:0"]);
        link.rescan();
        assert_eq!(fake.calls.get(), 2);
        assert_eq!(connected(&fake, 1), ["Keys 24:0"]);
    }

    #[test]
    fn a_rescan_reconnects_only_when_the_chosen_ports_change() {
        let fake = Rc::new(Fake::default());
        *fake.ports.borrow_mut() = ports(&["Keys"]);
        let mut link = link(&fake);
        link.open(MidiChoice::Named("Keys".to_owned())).unwrap();
        fake.ports.borrow_mut().push("Pads".to_owned());
        link.rescan();
        assert_eq!(
            fake.calls.get(),
            1,
            "another device leaves a named one alone"
        );
        assert_eq!(link.ports(), ["Keys", "Pads"]);
        link.open(MidiChoice::Any).unwrap();
        link.rescan();
        assert_eq!(fake.calls.get(), 2, "same ports, no reconnect");
        fake.ports.borrow_mut().push("Drums".to_owned());
        link.rescan();
        assert_eq!(fake.calls.get(), 3, "a new port joins Any");
        assert_eq!(fake.drops.get(), 2);
    }

    #[test]
    fn no_input_never_rescans() {
        let fake = Rc::new(Fake::default());
        let mut link = link(&fake);
        link.open(MidiChoice::None).unwrap();
        let lists = fake.lists.get();
        link.tick(0.0);
        link.tick(10.0);
        assert_eq!(fake.lists.get(), lists);
        assert_eq!(fake.calls.get(), 0);
    }
}

// shortcut: this preview has no Android MIDI service bridge; enable the menu when one is available.
#[cfg(target_os = "android")]
mod backend {
    use super::*;

    pub fn lister() -> impl FnMut() -> Vec<String> {
        Vec::new
    }

    pub fn open(
        _: &MidiChoice,
        _: SyncSender<Vec<u8>>,
        _: Option<Context>,
    ) -> Result<Box<dyn Any>, String> {
        Ok(Box::new(()))
    }
}

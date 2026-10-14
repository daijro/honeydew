//! The app's audio input: the `INPUT` menu and the [`InputLink`] seam that opens
//! one input device into an [`InputRing`]. Tests hand the link a fake opener and push frames into its ring; no test
//! opens a device. The web has no input: its menu is `<no input>` and every open fails.

use std::any::Any;
use std::sync::Arc;

pub use honeydew_audio::input::{InputDevice, InputReader, InputRing, input_devices};

use crate::ui::popup::PopupItem;

/// An `INPUT` menu pick: a device of the listed devices, and its port (`Input N`) or port pair (`Inputs N + M`),
/// all 0-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputChoice {
    /// Index into the device list the menu was built from.
    pub device: usize,
    pub first: usize,
    /// The right port of a stereo pair.
    pub second: Option<usize>,
}

/// The `INPUT` menu: per device its name as a disabled row, then `Input N` per port, then `Inputs N + M` per pair.
/// Without devices, one `<no input>` item. An item's id is its pick; `None` (`<no input>`, or a device row) picks
/// no input.
// The pairs are 1 + 2, 3 + 4 … (an odd last port has none), listed after the single ports.
pub fn input_menu(devices: &[InputDevice]) -> Vec<PopupItem<Option<InputChoice>>> {
    if devices.is_empty() {
        return vec![PopupItem::new(None, 0, 0, "<no input>")];
    }
    let mut items = Vec::new();
    for (index, device) in devices.iter().enumerate() {
        let ports = usize::from(device.channels);
        let mut push = |id, label: String, disabled| {
            let row = items.len();
            items.push(PopupItem {
                disabled,
                ..PopupItem::new(id, 0, row, label)
            });
        };
        push(None, device.name.clone(), true);
        let pick = |first, second| {
            Some(InputChoice {
                device: index,
                first,
                second,
            })
        };
        for port in 0..ports {
            push(pick(port, None), format!("Input {}", port + 1), false);
        }
        for left in (0..ports.saturating_sub(1)).step_by(2) {
            let label = format!("Inputs {} + {}", left + 1, left + 2);
            push(pick(left, Some(left + 1)), label, false);
        }
    }
    items
}

/// What an opener hands back: the ring the device fills, and the stream that fills it (dropped to stop it).
pub type Opened = (Arc<InputRing>, Box<dyn Any>);

/// Opens an input device (`None` = the default one) at a rate.
type Opener = Box<dyn FnMut(Option<&str>, u32) -> Result<Opened, String>>;

/// At most one open input device, and the opener that opens it: the device (cpal) natively, a fake in tests.
pub struct InputLink {
    open: Opener,
    /// Lists the input devices ([`Self::devices`]).
    list: fn() -> Vec<InputDevice>,
    /// The open device and rate, its ring and its stream.
    current: Option<(Option<String>, u32, Opened)>,
}

impl Default for InputLink {
    fn default() -> Self {
        Self::new(open_device).listing(input_devices)
    }
}

impl InputLink {
    pub fn new(open: impl FnMut(Option<&str>, u32) -> Result<Opened, String> + 'static) -> Self {
        Self {
            open: Box::new(open),
            list: Vec::new,
            current: None,
        }
    }

    /// This link with `list` listing its devices: cpal's by default, none for a link made with [`Self::new`].
    pub fn listing(self, list: fn() -> Vec<InputDevice>) -> Self {
        Self { list, ..self }
    }

    /// The input devices there are now, for the `INPUT` menu.
    pub fn devices(&self) -> Vec<InputDevice> {
        (self.list)()
    }

    /// The open device's ring.
    pub fn ring(&self) -> Option<&Arc<InputRing>> {
        self.current.as_ref().map(|(_, _, (ring, _))| ring)
    }

    /// Makes `device` at `rate` the open input. Already open and its stream working: nothing happens. Otherwise
    /// the open device closes first; on failure none is open and the error comes back (the app's permission
    /// message).
    pub fn open(&mut self, device: Option<&str>, rate: u32) -> Result<(), String> {
        if let Some((open, at, (ring, _))) = &self.current
            && open.as_deref() == device
            && *at == rate
            && !ring.failed()
        {
            return Ok(());
        }
        self.close();
        let opened = (self.open)(device, rate)?;
        self.current = Some((device.map(str::to_owned), rate, opened));
        Ok(())
    }

    /// Stops the open device, if any.
    pub fn close(&mut self) {
        self.current = None;
    }
}

/// The real opener: cpal natively, none on the web.
pub fn open_device(device: Option<&str>, rate: u32) -> Result<Opened, String> {
    #[cfg(not(target_arch = "wasm32"))]
    return honeydew_audio::input::open_input(device, rate)
        .map(|(ring, stream)| (ring, Box::new(stream) as Box<dyn Any>))
        .map_err(|err| format!("{err:?}"));
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (device, rate);
        Err("the web has no audio input".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    fn device(name: &str, channels: u16) -> InputDevice {
        InputDevice {
            name: name.to_owned(),
            channels,
        }
    }

    /// Each item as (label, id, disabled), checking the one-column layout.
    fn rows(items: &[PopupItem<Option<InputChoice>>]) -> Vec<(&str, Option<InputChoice>, bool)> {
        for (row, item) in items.iter().enumerate() {
            assert_eq!((item.col, item.row), (0, row));
            assert!(!item.is_header());
        }
        let rows = items
            .iter()
            .map(|i| (&*i.label, i.id.flatten(), i.disabled));
        rows.collect()
    }

    fn pick(device: usize, first: usize, second: Option<usize>) -> Option<InputChoice> {
        Some(InputChoice {
            device,
            first,
            second,
        })
    }

    #[test]
    fn without_devices_the_menu_is_no_input() {
        assert_eq!(rows(&input_menu(&[])), [("<no input>", None, false)]);
    }

    #[test]
    fn one_device_lists_its_name_disabled_then_its_ports() {
        assert_eq!(
            rows(&input_menu(&[device("Mic", 1)])),
            [("Mic", None, true), ("Input 1", pick(0, 0, None), false)]
        );
        assert_eq!(
            rows(&input_menu(&[device("Headset", 2)])),
            [
                ("Headset", None, true),
                ("Input 1", pick(0, 0, None), false),
                ("Input 2", pick(0, 1, None), false),
                ("Inputs 1 + 2", pick(0, 0, Some(1)), false),
            ]
        );
    }

    #[test]
    fn two_devices_list_one_after_the_other_with_their_pairs() {
        assert_eq!(
            rows(&input_menu(&[device("Interface", 5), device("Mic", 1)])),
            [
                ("Interface", None, true),
                ("Input 1", pick(0, 0, None), false),
                ("Input 2", pick(0, 1, None), false),
                ("Input 3", pick(0, 2, None), false),
                ("Input 4", pick(0, 3, None), false),
                ("Input 5", pick(0, 4, None), false),
                ("Inputs 1 + 2", pick(0, 0, Some(1)), false),
                ("Inputs 3 + 4", pick(0, 2, Some(3)), false),
                ("Mic", None, true),
                ("Input 1", pick(1, 0, None), false),
            ]
        );
    }

    /// The device and rate of each open.
    type Log = Rc<RefCell<Vec<(Option<String>, u32)>>>;

    /// A fake opener: logs each open, makes a stereo ring, and counts its streams' drops.
    fn fake(log: Log, dropped: Rc<Cell<usize>>) -> InputLink {
        struct Stream(Rc<Cell<usize>>);
        impl Drop for Stream {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }
        InputLink::new(move |device, rate| {
            if device == Some("broken") {
                return Err("denied".to_owned());
            }
            log.borrow_mut().push((device.map(str::to_owned), rate));
            let ring = Arc::new(InputRing::new(2, rate, 4096));
            Ok((ring, Box::new(Stream(dropped.clone()))))
        })
    }

    #[test]
    fn the_link_opens_through_its_opener_and_the_pushed_frames_reach_a_reader() {
        let (log, dropped) = (Rc::default(), Rc::new(Cell::new(0)));
        let mut link = fake(Rc::clone(&log), dropped.clone());
        assert!(link.ring().is_none());
        assert_eq!(link.open(None, 48_000), Ok(()));
        let ring = link.ring().expect("open").clone();
        assert_eq!((ring.channels(), ring.rate()), (2, 48_000));
        let mut reader = InputReader::new(ring.clone());
        ring.push([0.5, -0.5, 0.25, -0.25].into_iter());
        let mut out = [0.0; 4];
        assert_eq!(reader.read(&[1], &mut out), 2);
        assert_eq!(out[..2], [-0.5, -0.25]);

        // The same device at the same rate stays open; another device replaces it.
        assert_eq!(link.open(None, 48_000), Ok(()));
        assert_eq!(log.borrow().len(), 1);
        assert_eq!(link.open(Some("USB"), 48_000), Ok(()));
        assert_eq!(dropped.get(), 1);
        assert_eq!(link.open(Some("USB"), 44_100), Ok(()));
        assert_eq!(dropped.get(), 2);
        assert_eq!(
            *log.borrow(),
            [
                (None, 48_000),
                (Some("USB".to_owned()), 48_000),
                (Some("USB".to_owned()), 44_100),
            ]
        );
        assert_eq!(link.ring().map(|r| r.rate()), Some(44_100));

        link.close();
        assert!(link.ring().is_none());
        assert_eq!(dropped.get(), 3);
    }

    #[test]
    fn a_failed_open_leaves_no_input_open_and_says_why() {
        let dropped = Rc::new(Cell::new(0));
        let mut link = fake(Rc::default(), dropped.clone());
        assert_eq!(link.open(None, 48_000), Ok(()));
        assert_eq!(link.open(Some("broken"), 48_000), Err("denied".to_owned()));
        assert!(link.ring().is_none());
        assert_eq!(dropped.get(), 1);
    }

    #[test]
    fn a_failed_stream_on_the_same_device_is_reopened() {
        let (log, dropped): (Log, _) = (Rc::default(), Rc::new(Cell::new(0)));
        let mut link = fake(Rc::clone(&log), dropped.clone());
        assert_eq!(link.open(Some("USB"), 48_000), Ok(()));
        link.ring().expect("open").set_failed();
        assert_eq!(link.open(Some("USB"), 48_000), Ok(()));
        assert_eq!(log.borrow().len(), 2);
        assert_eq!(dropped.get(), 1);
        assert!(!link.ring().expect("reopened").failed());
    }
}

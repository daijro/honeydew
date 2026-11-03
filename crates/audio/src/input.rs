//! Audio input capture: the [`InputRing`] an input callback fills, its [`InputReader`]s, and, natively
//! only, the cpal input devices. Tests never open a device: they push frames into a ring.
//!
//! The ring never blocks its producer. One producer writes interleaved frames and publishes how many it has written;
//! any number of readers (the engine; the recorder, from the engine's tap) copy from their own positions. A reader a
//! whole ring behind skips ahead and counts an overrun. Nothing on the producer's or a reader's path allocates or locks.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering, fence};

/// Slots in the 25 ms peak history. The one the producer writes next is never read, so [`InputRing::peak`] holds
/// `PEAK_SLOTS − 1` windows: more than the 48 the rack meter draws.
pub const PEAK_SLOTS: usize = 64;

/// Preallocated interleaved frames from one input device, with a 25 ms peak history per port.
pub struct InputRing {
    channels: usize,
    rate: u32,
    /// Frames held.
    capacity: usize,
    /// Frames per peak slot: 25 ms at `rate`, rounded.
    window: usize,
    /// `capacity × channels` f32 bit patterns; frame `n` is at slot `n % capacity`.
    samples: Box<[AtomicU32]>,
    /// Frames the producer has begun writing. A reader drops what it copied from a slot being overwritten.
    claimed: AtomicU64,
    /// Frames fully written.
    written: AtomicU64,
    /// The producer's open peak window: frames in it, and each port's peak so far (atomics so `push` takes `&self`).
    fill: AtomicU64,
    open_peaks: Box<[AtomicU32]>,
    /// `PEAK_SLOTS × channels` closed peaks; window `n` is at slot `n % PEAK_SLOTS`.
    peaks: Box<[AtomicU32]>,
    /// Peak windows closed.
    peaks_done: AtomicU64,
    /// The stream filling the ring has failed: it fills no more.
    failed: AtomicBool,
}

fn atomics(len: usize) -> Box<[AtomicU32]> {
    (0..len).map(|_| AtomicU32::new(0)).collect()
}

impl InputRing {
    /// A ring of `capacity` frames of `channels` ports at `rate` Hz.
    pub fn new(channels: usize, rate: u32, capacity: usize) -> Self {
        assert!(channels > 0 && capacity > 0, "an input ring holds frames");
        Self {
            channels,
            rate,
            capacity,
            // 25 ms rounded half up (1102.5 frames at 44.1 kHz → 1103).
            window: ((rate as usize + 20) / 40).max(1),
            samples: atomics(capacity * channels),
            claimed: AtomicU64::new(0),
            written: AtomicU64::new(0),
            fill: AtomicU64::new(0),
            open_peaks: atomics(channels),
            peaks: atomics(PEAK_SLOTS * channels),
            peaks_done: AtomicU64::new(0),
            failed: AtomicBool::new(false),
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Frames per peak-history slot.
    pub fn window_frames(&self) -> usize {
        self.window
    }

    /// Frames written since the ring was made.
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Acquire)
    }

    /// Appends interleaved samples; a trailing partial frame is dropped. The single producer's call (the input
    /// callback, or a test): two pushes at once would interleave their frames. Never allocates or blocks.
    pub fn push(&self, mut samples: impl ExactSizeIterator<Item = f32>) {
        let channels = self.channels;
        let frames = samples.len() / channels;
        if frames == 0 {
            return;
        }
        let start = self.written.load(Ordering::Relaxed);
        let end = start + frames as u64;
        // Seqlock order: the claim is visible before any slot it overwrites (see `InputReader::read`).
        self.claimed.store(end, Ordering::Relaxed);
        fence(Ordering::Release);
        let mut slot = (start % self.capacity as u64) as usize;
        let mut fill = self.fill.load(Ordering::Relaxed) as usize;
        for _ in 0..frames {
            for port in 0..channels {
                let x = samples.next().unwrap_or(0.0);
                self.samples[slot * channels + port].store(x.to_bits(), Ordering::Relaxed);
                let open = &self.open_peaks[port];
                let peak = f32::from_bits(open.load(Ordering::Relaxed)).max(x.abs());
                open.store(peak.to_bits(), Ordering::Relaxed);
            }
            slot = (slot + 1) % self.capacity;
            fill += 1;
            if fill == self.window {
                fill = 0;
                self.close_window();
            }
        }
        self.fill.store(fill as u64, Ordering::Relaxed);
        self.written.store(end, Ordering::Release);
    }

    fn close_window(&self) {
        let done = self.peaks_done.load(Ordering::Relaxed);
        let base = (done % PEAK_SLOTS as u64) as usize * self.channels;
        for (closed, open) in self.peaks[base..].iter().zip(&self.open_peaks) {
            closed.store(open.swap(0, Ordering::Relaxed), Ordering::Relaxed);
        }
        self.peaks_done.store(done + 1, Ordering::Release);
    }

    /// Peak windows closed since the ring was made.
    pub fn peaks_done(&self) -> u64 {
        self.peaks_done.load(Ordering::Acquire)
    }

    /// The absolute peak of `port` over the 25 ms window closed `age` windows ago (0 = the newest); 0 for a window
    /// not yet closed or no longer held (`age ≥ PEAK_SLOTS − 1`: that slot is the one being written), or a port the
    /// device lacks.
    pub fn peak(&self, age: usize, port: usize) -> f32 {
        let done = self.peaks_done();
        if age >= PEAK_SLOTS - 1 || age as u64 >= done || port >= self.channels {
            return 0.0;
        }
        let slot = ((done - 1 - age as u64) % PEAK_SLOTS as u64) as usize;
        f32::from_bits(self.peaks[slot * self.channels + port].load(Ordering::Relaxed))
    }

    /// The stream filling this ring has failed (its error callback ran); it needs reopening.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    /// The stream's report of [`Self::failed`].
    pub fn set_failed(&self) {
        self.failed.store(true, Ordering::Release);
    }
}

/// One reader's position in an [`InputRing`]. It starts at the newest frame. Made on the app thread; its reads may
/// run on the audio thread.
pub struct InputReader {
    ring: Arc<InputRing>,
    /// The next frame to read.
    pos: u64,
    overruns: u32,
}

impl InputReader {
    pub fn new(ring: Arc<InputRing>) -> Self {
        let pos = ring.written();
        Self {
            ring,
            pos,
            overruns: 0,
        }
    }

    pub fn ring(&self) -> &Arc<InputRing> {
        &self.ring
    }

    /// The frames the next read would take with room enough: the backlog, at most what the ring holds.
    pub fn available(&self) -> u64 {
        (self.ring.written() - self.pos).min(self.ring.capacity as u64)
    }

    /// The ring frame the next read starts at.
    pub fn position(&self) -> u64 {
        self.pos
    }

    /// How many times this reader fell a whole ring behind and skipped ahead.
    pub fn overruns(&self) -> u32 {
        self.overruns
    }

    /// Copies the waiting frames, as many as fit, into `out` as frames of `ports` (indices into the device's ports;
    /// one the device lacks reads 0), and returns how many. A reader more than a ring behind first skips to the
    /// oldest frame held. Never allocates or blocks.
    pub fn read(&mut self, ports: &[usize], out: &mut [f32]) -> usize {
        let ring = &*self.ring;
        let width = ports.len();
        if width == 0 {
            return 0;
        }
        let capacity = ring.capacity as u64;
        let written = ring.written.load(Ordering::Acquire);
        if written - self.pos > capacity {
            self.pos = written - capacity;
            self.overruns += 1;
        }
        let frames = ((written - self.pos) as usize).min(out.len() / width);
        let mut slot = (self.pos % capacity) as usize;
        for frame in out.chunks_exact_mut(width).take(frames) {
            let base = slot * ring.channels;
            for (sample, &port) in frame.iter_mut().zip(ports) {
                *sample = if port < ring.channels {
                    f32::from_bits(ring.samples[base + port].load(Ordering::Relaxed))
                } else {
                    0.0
                };
            }
            slot = (slot + 1) % ring.capacity;
        }
        // The frames the producer began overwriting while they were copied may be torn: they are dropped. A slot
        // copied after the producer's claim fence sees that claim here (the fences pair).
        fence(Ordering::Acquire);
        let oldest = ring
            .claimed
            .load(Ordering::Relaxed)
            .saturating_sub(capacity);
        let torn = (oldest.saturating_sub(self.pos) as usize).min(frames);
        self.pos += frames as u64;
        if torn > 0 {
            out.copy_within(torn * width..frames * width, 0);
            self.overruns += 1;
        }
        frames - torn
    }
}

/// An input device: its name and how many ports it captures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDevice {
    pub name: String,
    pub channels: u16,
}

/// The web has no audio input: its `INPUT` menu lists `<no input>`.
#[cfg(target_arch = "wasm32")]
pub fn input_devices() -> Vec<InputDevice> {
    Vec::new()
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{InputError, input_devices, open_input};

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::Arc;

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, Sample, SampleFormat, SampleRate, SizedSample};

    use super::{InputDevice, InputRing};

    /// Seconds the ring holds: the engine drains it once per render call.
    const RING_SECONDS: usize = 2;

    #[derive(Debug)]
    pub enum InputError {
        NoDevice,
        /// The device cannot capture at the output's rate (no resampling, the open fails).
        Rate(u32),
        Configs(cpal::SupportedStreamConfigsError),
        Build(cpal::BuildStreamError),
        Play(cpal::PlayStreamError),
        UnsupportedSampleFormat,
    }

    /// The host's input devices with the ports of their default config. Not the most any config offers: ALSA's
    /// plug PCMs (default, pulse, pipewire) list every count up to 32.
    pub fn input_devices() -> Vec<InputDevice> {
        let Ok(devices) = cpal::default_host().input_devices() else {
            return Vec::new();
        };
        devices
            .filter_map(|device| {
                let name = device.name().ok()?;
                let channels = device.default_input_config().ok()?.channels();
                Some(InputDevice { name, channels })
            })
            .collect()
    }

    /// The config at `rate` with the default config's port count, else with the most ports; f32 first among equals.
    fn pick_config(
        configs: impl Iterator<Item = cpal::SupportedStreamConfigRange>,
        rate: u32,
        default_channels: Option<u16>,
    ) -> Option<cpal::SupportedStreamConfig> {
        configs
            .filter_map(|c| c.try_with_sample_rate(SampleRate(rate)))
            .max_by_key(|c| {
                let channels = c.channels();
                let f32 = c.sample_format() == SampleFormat::F32;
                (Some(channels) == default_channels, channels, f32)
            })
    }

    /// Starts capturing from the input device named `device` (the default one for `None`) at `rate`, the output's
    /// rate, with the ports [`input_devices`] lists, into a new ring. Dropping the stream stops the capture.
    pub fn open_input(
        device: Option<&str>,
        rate: u32,
    ) -> Result<(Arc<InputRing>, cpal::Stream), InputError> {
        let host = cpal::default_host();
        let device = match device {
            None => host.default_input_device(),
            Some(name) => host
                .input_devices()
                .ok()
                .and_then(|mut devices| devices.find(|d| d.name().is_ok_and(|n| n == name))),
        }
        .ok_or(InputError::NoDevice)?;
        let default_channels = device.default_input_config().ok().map(|c| c.channels());
        let configs = device
            .supported_input_configs()
            .map_err(InputError::Configs)?;
        let supported =
            pick_config(configs, rate, default_channels).ok_or(InputError::Rate(rate))?;
        let ring = Arc::new(InputRing::new(
            usize::from(supported.channels()),
            rate,
            rate as usize * RING_SECONDS,
        ));
        let (config, r) = (supported.config(), ring.clone());
        let stream = match supported.sample_format() {
            SampleFormat::I8 => build::<i8>(&device, &config, r),
            SampleFormat::I16 => build::<i16>(&device, &config, r),
            SampleFormat::I24 => build::<cpal::I24>(&device, &config, r),
            SampleFormat::I32 => build::<i32>(&device, &config, r),
            SampleFormat::I64 => build::<i64>(&device, &config, r),
            SampleFormat::U8 => build::<u8>(&device, &config, r),
            SampleFormat::U16 => build::<u16>(&device, &config, r),
            SampleFormat::U32 => build::<u32>(&device, &config, r),
            SampleFormat::U64 => build::<u64>(&device, &config, r),
            SampleFormat::F32 => build::<f32>(&device, &config, r),
            SampleFormat::F64 => build::<f64>(&device, &config, r),
            _ => return Err(InputError::UnsupportedSampleFormat),
        }
        .map_err(InputError::Build)?;
        stream.play().map_err(InputError::Play)?;
        Ok((ring, stream))
    }

    fn build<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        ring: Arc<InputRing>,
    ) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        let failed = ring.clone();
        device.build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                ring.push(data.iter().map(|&s| f32::from_sample(s)));
            },
            move |_| failed.set_failed(),
            None,
        )
    }

    #[cfg(test)]
    mod tests {
        use cpal::{SampleFormat, SampleRate, SupportedBufferSize, SupportedStreamConfigRange};

        use super::pick_config;

        fn range(channels: u16, format: SampleFormat) -> SupportedStreamConfigRange {
            let (min, max) = (SampleRate(8_000), SampleRate(96_000));
            SupportedStreamConfigRange::new(
                channels,
                min,
                max,
                SupportedBufferSize::Unknown,
                format,
            )
        }

        #[test]
        fn the_default_port_count_wins_over_a_plug_pcm_s_32_channels() {
            // ALSA's default/pulse/pipewire plugs list every count from 1 to 32.
            let plug = || {
                (1..=32).flat_map(|c| [range(c, SampleFormat::I16), range(c, SampleFormat::F32)])
            };
            let picked = pick_config(plug(), 48_000, Some(2)).expect("a config");
            assert_eq!(
                (
                    picked.channels(),
                    picked.sample_format(),
                    picked.sample_rate()
                ),
                (2, SampleFormat::F32, SampleRate(48_000))
            );
            // No default, or none at its count: the most ports, f32 first.
            let most = pick_config(plug(), 48_000, None).expect("a config");
            assert_eq!(
                (most.channels(), most.sample_format()),
                (32, SampleFormat::F32)
            );
            assert_eq!(
                pick_config(plug(), 48_000, Some(40)).map(|c| c.channels()),
                Some(32)
            );
            // Nothing at the rate.
            assert!(pick_config(plug(), 192_000, Some(2)).is_none());
        }
    }
}

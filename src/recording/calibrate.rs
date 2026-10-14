//! The audio recording latency calibration. The engine runs a [`Ping`] on the audio thread, one sample at a time
//! against the input heard in the same block, so the result is the round trip in the engine's own sample clock: out of
//! The speaker and back into the microphone, with no effect latency in it.

pub use honeydew_audio::device::PingFailure as Failure;

/// The question `Calibrate` asks first; its `Calibrate` starts the test.
pub const INTRO: &str = "To accurately align recordings with the Playlist please complete the audio recording delay \
                         calibration. Disconnect any headphones, turn up the volume and use a quiet location.";
/// Shown while the test runs.
pub const SENDING: &str = "Sending test signal...";

/// A failure's text.
pub fn failed(failure: Failure) -> &'static str {
    match failure {
        Failure::Noise => {
            "Failed, too much environment noise. Please do this test in a quiet environment!"
        }
        Failure::Timeout => {
            "Failed, reached timeout. Please remove headphones and use maximum volume!"
        }
        Failure::Unknown => "Failed, unknown error! Please report!",
    }
}

/// The success text for a latency of `samples` frames at `rate` Hz: L·1000/Fs ms with one decimal.
pub fn succeeded(samples: u32, rate: u32) -> String {
    let ms = f64::from(samples) * 1000.0 / f64::from(rate);
    format!("Succeeded, Latency is: {ms:.1} ms ({samples} samples)")
}

/// The tone's frequency: φ advances by this over Fs a sample.
const TONE_HZ: f64 = 4000.0;
/// Input louder than this in the second second of silence is noise.
const NOISE: f32 = 0.1;
/// Input louder than this during the tone is its echo.
const ECHO: f32 = 0.2;

/// One latency measurement at Fs = `rate`: Fs samples of silence; Fs more, failing with noise on any input sample above
/// 0.1; then cos(2πφ) at amplitude 1, φ from 0 advancing 4000/Fs a sample and wrapped to 0–1, with a counter from 0 on
/// its first sample. The first input sample above 0.2 succeeds with the counter before that sample counts; the counter
/// reaching Fs times out. Allocates nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ping {
    rate: u32,
    /// The phase: 0 and 1 silence, 2 the tone.
    phase: u8,
    /// Samples into a silent phase, or the tone's counter.
    n: u32,
    phi: f64,
    result: Option<Result<u32, Failure>>,
}

impl Ping {
    pub fn new(rate: u32) -> Self {
        Self {
            rate,
            phase: 0,
            n: 0,
            phi: 0.0,
            result: None,
        }
    }

    /// The round trip in frames, or why there is none; `None` while it runs.
    pub fn result(&self) -> Option<Result<u32, Failure>> {
        self.result
    }

    /// There is no input to listen to: the unknown error.
    pub fn fail(&mut self) {
        self.result.get_or_insert(Err(Failure::Unknown));
    }

    /// The output sample that goes out as input sample `x` comes in (the first port of the input block); silence once
    /// it has a result.
    pub fn sample(&mut self, x: f32) -> f32 {
        if self.result.is_some() {
            return 0.0;
        }
        let fs = self.rate.max(1);
        match self.phase {
            0 | 1 => {
                if self.phase == 1 && x.abs() > NOISE {
                    self.result = Some(Err(Failure::Noise));
                    return 0.0;
                }
                self.n += 1;
                if self.n == fs {
                    (self.phase, self.n) = (self.phase + 1, 0);
                }
                0.0
            }
            _ => {
                let y = (std::f64::consts::TAU * self.phi).cos() as f32;
                self.phi += TONE_HZ / f64::from(fs);
                self.phi -= self.phi.floor();
                if x.abs() > ECHO {
                    self.result = Some(Ok(self.n));
                } else {
                    self.n += 1;
                    if self.n == fs {
                        self.result = Some(Err(Failure::Timeout));
                    }
                }
                y
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 kHz, so the tone's φ steps by 4 and wraps to 0 every sample: cos(0) = 1 throughout.
    const RATE: u32 = 1_000;

    /// Runs `ping` over `input` until it has a result: the outputs and the result.
    fn run(
        ping: &mut Ping,
        input: impl IntoIterator<Item = f32>,
    ) -> (Vec<f32>, Option<Result<u32, Failure>>) {
        let mut out = Vec::new();
        for x in input {
            out.push(ping.sample(x));
            if ping.result().is_some() {
                break;
            }
        }
        (out, ping.result())
    }

    /// Silence, then the echo of the tone `delay` samples after it starts, louder than 0.2 by a hair.
    fn echoed(delay: usize) -> impl Iterator<Item = f32> {
        let silence = std::iter::repeat_n(0.05, 2 * RATE as usize);
        let tone = (0..).map(move |i| if i < delay { 0.2 } else { -0.2001 });
        silence.chain(tone)
    }

    #[test]
    fn two_seconds_of_silence_then_the_tone_until_its_echo_gives_the_counter() {
        let mut ping = Ping::new(RATE);
        let (out, result) = run(&mut ping, echoed(37));
        assert_eq!(result, Some(Ok(37)));
        // Fs + Fs silent samples, then 37 counted tone samples and the one heard with the echo.
        assert_eq!(out.len(), 2 * RATE as usize + 38);
        assert!(out[..2 * RATE as usize].iter().all(|&y| y == 0.0));
        assert!(
            out[2 * RATE as usize..].iter().all(|&y| y == 1.0),
            "cos(2π·0)"
        );
        // An echo on the tone's first sample is no delay at all; done, it is silent.
        let mut ping = Ping::new(RATE);
        assert_eq!(run(&mut ping, echoed(0)).1, Some(Ok(0)));
        assert_eq!(ping.sample(0.9), 0.0);
        assert_eq!(ping.result(), Some(Ok(0)));
    }

    #[test]
    fn the_tone_is_a_4_khz_cosine_on_the_sample_clock() {
        // At 48 kHz φ steps 1/12: the tone repeats every 12 samples, starting at 1.
        let mut ping = Ping::new(48_000);
        let silence = std::iter::repeat_n(0.0, 96_000);
        let (out, result) = run(&mut ping, silence.chain(std::iter::repeat_n(0.0, 24)));
        assert_eq!(result, None);
        let tone = &out[96_000..];
        for (i, &y) in tone.iter().enumerate() {
            let want = (std::f64::consts::TAU * (i % 12) as f64 / 12.0).cos() as f32;
            assert!((y - want).abs() < 1e-5, "{i}: {y} vs {want}");
        }
    }

    #[test]
    fn noise_in_the_second_second_fails_and_the_first_is_not_heard() {
        // Loud input in phase 0 is ignored; the same in phase 1 fails at once with noise.
        let mut input = vec![0.0; 2 * RATE as usize + 10];
        input[RATE as usize - 1] = 0.9;
        input[RATE as usize + 400] = -0.10001;
        let mut ping = Ping::new(RATE);
        let (out, result) = run(&mut ping, input.iter().copied());
        assert_eq!(result, Some(Err(Failure::Noise)));
        assert_eq!(out.len(), RATE as usize + 401);
        // Exactly 0.1 is no noise.
        input[RATE as usize + 400] = 0.1;
        let mut ping = Ping::new(RATE);
        assert_eq!(run(&mut ping, input.iter().copied()).1, None);
    }

    #[test]
    fn no_echo_within_fs_tone_samples_times_out_and_no_input_is_the_unknown_error() {
        // Exactly 0.2 is no echo: the counter reaches Fs on the tone's Fs-th sample.
        let mut ping = Ping::new(RATE);
        let silence = std::iter::repeat_n(0.0, 2 * RATE as usize);
        let (out, result) = run(&mut ping, silence.chain(std::iter::repeat(0.2)));
        assert_eq!(result, Some(Err(Failure::Timeout)));
        assert_eq!(out.len(), 3 * RATE as usize);
        // An echo on the last sample before that still counts.
        let mut ping = Ping::new(RATE);
        assert_eq!(
            run(&mut ping, echoed(RATE as usize - 1)).1,
            Some(Ok(RATE - 1))
        );
        // No input: the unknown error, which a later result does not replace.
        let mut ping = Ping::new(RATE);
        ping.fail();
        assert_eq!(ping.result(), Some(Err(Failure::Unknown)));
        let mut done = Ping::new(RATE);
        run(&mut done, echoed(5));
        done.fail();
        assert_eq!(done.result(), Some(Ok(5)));
    }

    #[test]
    fn the_texts_give_milliseconds_to_a_tenth_and_samples_or_the_failure() {
        assert_eq!(
            succeeded(1234, 48_000),
            "Succeeded, Latency is: 25.7 ms (1234 samples)"
        );
        assert_eq!(
            succeeded(0, 44_100),
            "Succeeded, Latency is: 0.0 ms (0 samples)"
        );
        assert_eq!(
            [Failure::Noise, Failure::Timeout, Failure::Unknown].map(failed),
            [
                "Failed, too much environment noise. Please do this test in a quiet environment!",
                "Failed, reached timeout. Please remove headphones and use maximum volume!",
                "Failed, unknown error! Please report!",
            ]
        );
    }
}

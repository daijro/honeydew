use honeydew_audio::builtin_drums::{KITS, PADS, RATE, builtin};
use std::f32::consts::TAU;

const MINUS_1_DBFS: f32 = 0.891_250_9;

fn mono(pad: &str) -> Vec<f32> {
    let sample = builtin(&format!("Built-in/{pad}")).unwrap_or_else(|| panic!("no built-in {pad}"));
    assert_eq!(sample.sample_rate, RATE);
    assert!(sample.right.is_none(), "{pad} is mono");
    sample.left.to_vec()
}

fn seconds(samples: &[f32]) -> f32 {
    samples.len() as f32 / RATE as f32
}

/// Frequency from the rising zero crossings in `from..to` seconds.
fn zero_crossing_hz(samples: &[f32], from: f32, to: f32) -> f32 {
    let window = &samples[(from * RATE as f32) as usize..(to * RATE as f32) as usize];
    let rising: Vec<usize> = (1..window.len())
        .filter(|&i| window[i - 1] < 0.0 && window[i] >= 0.0)
        .collect();
    let (first, last) = (rising[0], rising[rising.len() - 1]);
    (rising.len() - 1) as f32 * RATE as f32 / (last - first) as f32
}

/// The power-weighted mean frequency of the whole sound: Hann frames of 1024, power spectra summed (Welch).
fn spectral_centroid(samples: &[f32]) -> f32 {
    const N: usize = 1024;
    let hann: Vec<f32> = (0..N)
        .map(|i| 0.5 - 0.5 * (TAU * i as f32 / N as f32).cos())
        .collect();
    let (cos, sin): (Vec<f32>, Vec<f32>) = (0..N)
        .map(|i| (TAU * i as f32 / N as f32).sin_cos())
        .map(|(s, c)| (c, s))
        .unzip();
    let mut power = vec![0.0_f64; N / 2];
    for frame in samples.as_chunks::<N>().0 {
        let x: Vec<f32> = frame.iter().zip(&hann).map(|(s, w)| s * w).collect();
        for (k, p) in power.iter_mut().enumerate() {
            let (mut re, mut im) = (0.0_f32, 0.0_f32);
            for (i, s) in x.iter().enumerate() {
                re += s * cos[(k * i) % N];
                im -= s * sin[(k * i) % N];
            }
            *p += f64::from(re * re + im * im);
        }
    }
    let bin_hz = f64::from(RATE) / N as f64;
    let total: f64 = power.iter().sum();
    (power
        .iter()
        .enumerate()
        .map(|(k, p)| k as f64 * bin_hz * p)
        .sum::<f64>()
        / total) as f32
}

#[test]
fn the_eight_default_pads_have_built_ins() {
    assert_eq!(
        PADS,
        [
            "Kick",
            "Snare",
            "Clap",
            "Closed Hat",
            "Open Hat",
            "Tom",
            "Perc",
            "Crash"
        ]
    );
}

#[test]
fn folders_take_either_separator_and_nothing_else() {
    for pad in PADS {
        let slash = builtin(&format!("Built-in/{pad}")).unwrap();
        let backslash = builtin(&format!("Built-in\\{pad}")).unwrap();
        assert_eq!(slash.left, backslash.left, "{pad}");
    }
    for folder in [
        "Kick",
        "Built-in",
        "Built-in/",
        "Built-in/Cowbell",
        "Built-in/kick",
        "Samples/Kick",
        "Built-in/Kick/x",
    ] {
        assert!(builtin(folder).is_none(), "{folder:?}");
    }
}

#[test]
fn rendering_is_deterministic() {
    for pad in PADS {
        assert_eq!(mono(pad), mono(pad), "{pad}");
    }
}

#[test]
fn every_one_shot_peaks_at_minus_1_dbfs() {
    for pad in PADS {
        let samples = mono(pad);
        assert!(samples.iter().all(|s| s.is_finite()), "{pad}");
        let peak = samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!((peak - MINUS_1_DBFS).abs() < 1e-4, "{pad} peaks at {peak}");
    }
}

#[test]
fn every_one_shot_ends_in_silence() {
    for pad in PADS {
        let samples = mono(pad);
        assert_eq!(*samples.last().unwrap(), 0.0, "{pad}");
        let tail = &samples[samples.len() - RATE as usize / 50..];
        let tail_peak = tail.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(
            tail_peak < 0.001,
            "{pad}: the last 20 ms reach {tail_peak} (above −60 dBFS)"
        );
    }
}

#[test]
fn every_one_shot_has_a_body() {
    for pad in PADS {
        let head = &mono(pad)[..RATE as usize / 20];
        let rms = (head.iter().map(|s| s * s).sum::<f32>() / head.len() as f32).sqrt();
        assert!(rms > 0.05, "{pad}: the first 50 ms have RMS {rms}");
    }
}

/// The clap strikes again 11 ms in; the perc only decays.
#[test]
fn the_clap_restrikes_and_the_perc_does_not() {
    let peak = |pad, from_ms: usize, to_ms: usize| {
        let ms = RATE as usize / 1000;
        mono(pad)[from_ms * ms..to_ms * ms]
            .iter()
            .fold(0.0_f32, |m, s| m.max(s.abs()))
    };
    let (gap, restrike) = (peak("Clap", 6, 10), peak("Clap", 11, 15));
    assert!(restrike > 1.5 * gap, "clap: {gap} then {restrike}");
    let (early, later) = (peak("Perc", 6, 10), peak("Perc", 11, 15));
    assert!(later < early, "perc: {early} then {later}");
}

#[test]
fn lengths_fit_each_sound() {
    let length = |pad| seconds(&mono(pad));
    for pad in PADS {
        assert!(
            (0.1..=3.0).contains(&length(pad)),
            "{pad}: {} s",
            length(pad)
        );
    }
    assert!(length("Closed Hat") < 0.25);
    assert!(length("Open Hat") > 4.0 * length("Closed Hat"));
    assert!((0.5..=1.0).contains(&length("Kick")));
    assert!(length("Crash") >= 2.0);
    assert!(length("Crash") > length("Open Hat"));
    assert!(length("Clap") > length("Perc"));
}

#[test]
fn kick_and_tom_settle_on_their_pitches() {
    let kick = zero_crossing_hz(&mono("Kick"), 0.15, 0.45);
    let tom = zero_crossing_hz(&mono("Tom"), 0.15, 0.4);
    assert!((45.0..60.0).contains(&kick), "kick body at {kick} Hz");
    assert!((130.0..170.0).contains(&tom), "tom body at {tom} Hz");
}

#[test]
fn spectral_centroids_rise_from_kick_to_crash() {
    let centroid = |pad| spectral_centroid(&mono(pad));
    let order = ["Kick", "Tom", "Snare", "Closed Hat", "Crash"];
    let centroids: Vec<f32> = order.iter().map(|pad| centroid(pad)).collect();
    for (pair, hz) in order.windows(2).zip(centroids.windows(2)) {
        assert!(
            hz[0] < hz[1],
            "{} ({} Hz) is not below {} ({} Hz)",
            pair[0],
            hz[0],
            pair[1],
            hz[1]
        );
    }
    let open_hat = centroid("Open Hat");
    assert!(
        centroids[2] < open_hat && open_hat < centroids[4],
        "open hat at {open_hat} Hz"
    );
}

/// FNV-1a 64 over the samples' bits.
fn hash(samples: &[f32]) -> u64 {
    samples
        .iter()
        .flat_map(|s| s.to_bits().to_le_bytes())
        .fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

/// Pins the default voices from before they became recipes. Kick accepts the verified glibc 2.39 and 2.44 `tanhf`
/// outputs; native math can differ even on the same target. The portable check is [`the_default_kit_keeps_its_samples`].
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn the_default_kit_is_bit_identical() {
    let hashes: Vec<u64> = PADS.iter().map(|pad| hash(&mono(pad))).collect();
    assert!(
        [0x54e2_f46e_d851_6685, 0xb12b_13b4_0b48_3b88].contains(&hashes[0]),
        "unexpected Kick hash: {:#018x}",
        hashes[0]
    );
    assert_eq!(
        hashes[1..],
        [
            0xe43d_a116_2539_b337, // Snare
            0x516e_0c02_98f4_bd4d, // Clap
            0x6641_ac02_3a62_b036, // Closed Hat
            0x94c8_97af_63db_f35d, // Open Hat
            0xae23_04bf_037e_f49f, // Tom
            0xe88e_1559_0131_5fe9, // Perc
            0x088d_8e2e_f006_8c49, // Crash
        ]
    );
}

/// Each default pad's length and every (length / 40)-th sample, to 6 decimals.
#[rustfmt::skip]
const DEFAULT_KIT_SAMPLES: [(usize, [f32; 40]); 8] = [
    // Kick
    (39690, [
        -0.007423, 0.301707, 0.513298, -0.637208, 0.192228, 0.491118, -0.060380, -0.357366, -0.198881, 0.101682,
        0.206542, 0.094378, -0.065369, -0.113632, -0.046296, 0.039370, 0.061714, 0.022516, -0.023312, -0.033317,
        -0.010771, 0.013656, 0.017913, 0.005043, -0.007928, -0.009597, -0.002294, 0.004568, 0.005123, 0.001002,
        -0.002614, -0.002725, -0.000410, 0.001487, 0.001444, 0.000150, -0.000841, -0.000762, -0.000042, 0.000473,
    ]),
    // Snare
    (22050, [
        0.123355, 0.158171, -0.306634, 0.065812, 0.043587, -0.036921, -0.099604, -0.037008, 0.118287, 0.006645,
        -0.053121, -0.029528, 0.007026, -0.007748, -0.008084, 0.011967, -0.002225, 0.015563, -0.013774, 0.005029,
        -0.004531, -0.003899, 0.000194, 0.001252, 0.003972, -0.002986, 0.000590, -0.000193, -0.000694, -0.001175,
        0.000655, -0.000772, -0.000048, 0.000687, 0.000047, 0.000293, -0.000056, -0.000019, -0.000021, 0.000082,
    ]),
    // Clap
    (24255, [
        0.043769, -0.002283, -0.092770, -0.544748, -0.102341, 0.039203, -0.051272, 0.210150, 0.011489, -0.161253,
        0.059039, 0.043133, -0.039555, -0.025992, 0.002366, -0.031793, -0.019982, -0.007217, 0.011869, -0.012229,
        -0.001169, -0.003069, 0.002286, 0.002634, 0.004490, -0.002020, -0.004262, 0.000102, 0.000426, 0.000922,
        -0.000917, -0.000254, -0.001289, -0.000388, -0.000458, 0.000482, 0.000395, 0.000021, -0.000026, 0.000109,
    ]),
    // Closed Hat
    (7938, [
        0.558572, -0.300019, 0.197325, 0.016201, 0.098226, 0.013144, 0.151715, 0.031919, -0.069564, -0.019540,
        -0.007196, -0.027166, 0.027550, 0.041592, 0.009220, -0.034937, -0.000043, 0.010991, -0.002557, 0.000028,
        -0.006448, 0.000715, 0.003221, 0.003385, 0.001550, 0.004507, 0.000436, 0.000252, -0.001079, -0.000233,
        0.001051, 0.000621, 0.000679, 0.000003, -0.000615, -0.000229, -0.000009, 0.000122, 0.000164, 0.000053,
    ]),
    // Open Hat
    (52920, [
        0.515121, 0.030161, -0.186030, 0.353840, 0.042201, 0.002942, 0.108934, -0.063448, -0.064024, 0.010603,
        -0.061568, 0.087744, -0.043721, 0.008238, -0.053109, -0.006755, 0.017975, 0.019595, 0.011042, 0.005180,
        -0.006935, 0.007818, 0.005053, -0.004166, -0.002043, 0.007772, 0.003666, -0.004347, 0.000648, -0.000391,
        -0.000729, -0.001041, 0.000154, -0.000387, -0.000834, 0.000808, -0.000630, -0.000174, 0.000013, -0.000340,
    ]),
    // Tom
    (28664, [
        -0.014793, -0.537141, -0.126592, 0.195029, -0.030430, -0.329644, 0.066696, 0.094280, -0.124279, 0.103519,
        -0.063450, 0.014559, 0.031609, -0.059865, 0.059695, -0.034018, -0.000773, 0.024892, -0.027769, 0.013441,
        0.004639, -0.014198, 0.011729, -0.002459, -0.005498, 0.007325, -0.003719, -0.001223, 0.003783, -0.002995,
        0.000445, 0.001596, -0.001914, 0.000841, 0.000459, -0.001037, 0.000730, -0.000028, -0.000471, 0.000493,
    ]),
    // Perc
    (19845, [
        0.163456, -0.132949, -0.169597, -0.012824, 0.110208, 0.034347, -0.100098, -0.008661, 0.041433, 0.002192,
        0.002948, -0.006604, 0.009759, 0.000358, -0.005360, 0.013036, 0.000003, -0.000613, -0.000127, 0.000036,
        0.005191, 0.000859, 0.000127, -0.002244, -0.000211, 0.004229, 0.000117, -0.001412, -0.000085, 0.000808,
        0.000809, -0.000300, -0.000026, 0.000149, 0.000005, 0.000201, 0.000018, 0.000014, -0.000002, 0.000006,
    ]),
    // Crash
    (114659, [
        -0.016302, 0.279056, -0.214335, -0.151567, 0.091288, 0.079038, -0.001558, -0.036377, -0.039301, 0.001760,
        -0.031104, -0.047580, 0.006012, -0.018461, -0.024653, 0.004083, -0.007907, -0.020188, 0.001434, 0.006758,
        0.001133, -0.000199, 0.007247, -0.005965, -0.002977, -0.006119, 0.002665, -0.002401, -0.000236, 0.000423,
        -0.001512, -0.001034, -0.000535, 0.000961, -0.000330, -0.000127, 0.000419, 0.000089, -0.000142, 0.000323,
    ]),
];

/// The portable twin of the hash pin: libm drift between platforms is about 6e-8, and any edit to a recipe moves
/// these samples far more than 1e-5.
#[test]
fn the_default_kit_keeps_its_samples() {
    for (pad, (len, expected)) in PADS.iter().zip(DEFAULT_KIT_SAMPLES) {
        let samples = mono(pad);
        assert_eq!(samples.len(), len, "{pad}");
        let step = len / 40;
        for (k, want) in expected.iter().enumerate() {
            let got = samples[k * step];
            assert!(
                (got - want).abs() < 1e-5,
                "{pad} frame {}: {got}, not {want}",
                k * step
            );
        }
    }
}

fn kit_pad(kit: &str, pad: &str) -> Vec<f32> {
    let sample = builtin(&format!("Built-in/{kit}/{pad}"))
        .unwrap_or_else(|| panic!("no built-in {kit} {pad}"));
    assert_eq!(sample.sample_rate, RATE);
    assert!(sample.right.is_none(), "{kit} {pad} is mono");
    sample.left.to_vec()
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
}

/// In-place radix-2 FFT of `re` + i·`im` (equal power-of-two lengths).
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let (sin, cos) = (-std::f64::consts::TAU / len as f64).sin_cos();
        for start in (0..n).step_by(len) {
            let (mut wr, mut wi) = (1.0, 0.0);
            for k in start..start + len / 2 {
                let m = k + len / 2;
                let (tr, ti) = (re[m] * wr - im[m] * wi, re[m] * wi + im[m] * wr);
                (re[m], im[m]) = (re[k] - tr, im[k] - ti);
                re[k] += tr;
                im[k] += ti;
                (wr, wi) = (wr * cos - wi * sin, wr * sin + wi * cos);
            }
        }
        len <<= 1;
    }
}

const FFT: usize = 2048;

/// Welch power spectrum: Hann frames of [`FFT`], hop ½, zero-padded to at least one frame. Bin k is k·RATE/FFT Hz.
fn power_spectrum(samples: &[f32]) -> Vec<f64> {
    let mut padded = samples.to_vec();
    padded.resize(samples.len().max(FFT), 0.0);
    let mut power = vec![0.0; FFT / 2];
    for start in (0..=padded.len() - FFT).step_by(FFT / 2) {
        let mut re: Vec<f64> = (0..FFT)
            .map(|i| {
                let hann = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / FFT as f64).cos();
                f64::from(padded[start + i]) * hann
            })
            .collect();
        let mut im = vec![0.0; FFT];
        fft(&mut re, &mut im);
        for (k, p) in power.iter_mut().enumerate() {
            *p += re[k] * re[k] + im[k] * im[k];
        }
    }
    power
}

fn bin_hz(k: usize) -> f64 {
    k as f64 * f64::from(RATE) / FFT as f64
}

/// The share of the sound's power at or above `hz`.
fn share_above(samples: &[f32], hz: f64) -> f64 {
    let power = power_spectrum(samples);
    let above: f64 = (0..power.len())
        .filter(|&k| bin_hz(k) >= hz)
        .map(|k| power[k])
        .sum();
    above / power.iter().sum::<f64>()
}

fn centroid(samples: &[f32]) -> f64 {
    let power = power_spectrum(samples);
    (0..power.len()).map(|k| bin_hz(k) * power[k]).sum::<f64>() / power.iter().sum::<f64>()
}

/// The fundamental of `from..to` s: the lag (2–25 ms) of the autocorrelation's highest peak.
fn autocorrelation_hz(samples: &[f32], from: f32, to: f32) -> f32 {
    let x = &samples[(from * RATE as f32) as usize..(to * RATE as f32) as usize];
    let r = |lag: usize| -> f32 { x.iter().zip(&x[lag..]).map(|(a, b)| a * b).sum() };
    let lag = (RATE as usize / 500..RATE as usize / 40)
        .max_by(|&a, &b| r(a).total_cmp(&r(b)))
        .unwrap();
    RATE as f32 / lag as f32
}

#[test]
fn four_kits_of_4_to_14_distinct_pads() {
    let shape: Vec<(&str, usize)> = KITS.iter().map(|(key, pads)| (*key, pads.len())).collect();
    assert_eq!(
        shape,
        [("Pocket", 4), ("Dust", 8), ("Relay", 11), ("Foundry", 14)]
    );
    for (key, pads) in KITS {
        let mut sorted = pads.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), pads.len(), "{key} repeats a pad");
        assert_eq!(pads[0], "Kick", "{key} starts on its kick");
    }
}

#[test]
fn every_kit_pad_renders_the_same_each_time_at_minus_1_dbfs_into_silence() {
    for (kit, pads) in KITS {
        for &pad in pads {
            let samples = kit_pad(kit, pad);
            assert!(
                samples.iter().all(|s| s.is_finite()),
                "{kit} {pad} is not finite"
            );
            let length = seconds(&samples);
            assert!((0.1..=3.0).contains(&length), "{kit} {pad}: {length} s");
            let top = peak(&samples);
            assert!(
                (top - MINUS_1_DBFS).abs() < 1e-4,
                "{kit} {pad} peaks at {top}"
            );
            assert_eq!(*samples.last().unwrap(), 0.0, "{kit} {pad}");
            let tail = peak(&samples[samples.len() - RATE as usize / 50..]);
            assert!(tail < 0.001, "{kit} {pad}: the last 20 ms reach {tail}");
            assert_eq!(
                samples,
                kit_pad(kit, pad),
                "{kit} {pad} is not deterministic"
            );
            let backslash = builtin(&format!("Built-in\\{kit}\\{pad}")).unwrap();
            assert_eq!(backslash.left.to_vec(), samples, "{kit} {pad} by backslash");
        }
    }
}

#[test]
fn every_kit_pad_has_its_own_sound() {
    let mut seen: Vec<(String, u64)> = Vec::new();
    for (kit, pads) in KITS {
        for &pad in pads {
            let hash = hash(&kit_pad(kit, pad));
            if let Some((other, _)) = seen.iter().find(|(_, h)| *h == hash) {
                panic!("{kit} {pad} sounds exactly like {other}");
            }
            seen.push((format!("{kit} {pad}"), hash));
        }
    }
}

#[test]
fn unknown_kits_and_pads_are_none() {
    for folder in [
        "Built-in/Pocket",
        "Built-in/Pocket/",
        "Built-in/Pocket/Open Hat",
        "Built-in/Pocket/kick",
        "Built-in/pocket/Kick",
        "Built-in/Nowhere/Kick",
        "Built-in/Dust/Kick/x",
        "Built-in/Relay/Tom",
        "Built-in/Foundry/Perc",
        "Built-in//Kick",
        "Pocket/Kick",
    ] {
        assert!(builtin(folder).is_none(), "{folder:?}");
    }
}

#[test]
fn centroids_rise_kick_snare_closed_hat_in_every_kit() {
    for (kit, _) in KITS {
        let hz: Vec<f64> = ["Kick", "Snare", "Closed Hat"]
            .iter()
            .map(|pad| centroid(&kit_pad(kit, pad)))
            .collect();
        assert!(hz[0] < hz[1] && hz[1] < hz[2], "{kit}: {hz:?}");
    }
}

#[test]
fn kicks_sit_below_150_hz_and_hats_above_5_khz() {
    let default_kit = PADS.map(|pad| (pad.to_string(), mono(pad)));
    let factory = KITS.iter().flat_map(|(kit, pads)| {
        pads.iter()
            .map(move |pad| (format!("{kit} {pad}"), kit_pad(kit, pad)))
    });
    for (name, samples) in default_kit.into_iter().chain(factory) {
        if name.ends_with("Kick") {
            let low = 1.0 - share_above(&samples, 150.0);
            assert!(low > 0.6, "{name}: {low:.2} of the power below 150 Hz");
        }
        if name.ends_with("Hat") {
            let high = share_above(&samples, 5000.0);
            assert!(high > 0.6, "{name}: {high:.2} of the power above 5 kHz");
        }
    }
}

#[test]
fn relay_toms_are_tuned_low_mid_high() {
    let hz: Vec<f32> = ["Low Tom", "Mid Tom", "High Tom"]
        .iter()
        .map(|pad| autocorrelation_hz(&kit_pad("Relay", pad), 0.15, 0.45))
        .collect();
    assert!(hz[0] < hz[1] && hz[1] < hz[2], "{hz:?}");
    for (hz, note) in hz.iter().zip([98.0, 130.8, 174.6]) {
        assert!((hz / note - 1.0).abs() < 0.03, "{hz} Hz is not {note} Hz");
    }
}

#[test]
fn relay_has_the_long_sub_kick() {
    let kick = kit_pad("Relay", "Kick");
    assert!(
        (1.1..=1.3).contains(&seconds(&kick)),
        "{} s",
        seconds(&kick)
    );
    let body = autocorrelation_hz(&kick, 0.3, 0.8);
    assert!(body < 55.0, "the sub settles at {body} Hz");
    for (kit, _) in KITS.iter().filter(|(kit, _)| *kit != "Relay") {
        assert!(seconds(&kit_pad(kit, "Kick")) < seconds(&kick), "{kit}");
    }
}

#[test]
fn dust_has_less_air_above_8_khz_than_pocket() {
    for pad in ["Snare", "Closed Hat"] {
        let (dust, pocket) = (
            share_above(&kit_pad("Dust", pad), 8000.0),
            share_above(&kit_pad("Pocket", pad), 8000.0),
        );
        assert!(dust < pocket, "{pad}: Dust {dust:.4}, Pocket {pocket:.4}");
    }
    let hat = share_above(&kit_pad("Dust", "Closed Hat"), 8000.0);
    assert!(
        hat < 0.2,
        "Dust Closed Hat: {hat:.3} of the power above 8 kHz"
    );
}

/// `BUILTIN_DRUMS_WAV_DIR=<dir> cargo test -p honeydew-audio --test builtin_drums` writes every pad, and each kit's
/// pads in a row, as float WAVs for listening.
#[test]
fn writes_wavs_when_asked() {
    let Some(dir) = std::env::var_os("BUILTIN_DRUMS_WAV_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let default_kit = ("Default", &PADS[..]);
    for (kit, pads) in std::iter::once(default_kit).chain(KITS) {
        let mut row = Vec::new();
        for &pad in pads {
            let folder = match kit {
                "Default" => format!("Built-in/{pad}"),
                _ => format!("Built-in/{kit}/{pad}"),
            };
            let samples = builtin(&folder).unwrap().left.to_vec();
            let wav = honeydew_audio::wav_writer::float32_wav(RATE, &[&samples]);
            std::fs::write(dir.join(format!("{kit} {pad}.wav")), wav).unwrap();
            row.extend(&samples);
            row.extend(std::iter::repeat_n(0.0, RATE as usize / 5));
        }
        let wav = honeydew_audio::wav_writer::float32_wav(RATE, &[&row]);
        std::fs::write(dir.join(format!("{kit} kit.wav")), wav).unwrap();
    }
}

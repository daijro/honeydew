use std::f64::consts::TAU;

use honeydew_audio::mp3_writer::Mp3Encoder;
use honeydew_audio::sample::{SampleData, decode};

/// Samples per channel in an MPEG-1 frame.
const FRAME: usize = 1152;

/// `secs` of a sine at `freq` Hz and `amp` at `rate` Hz.
fn sine(rate: u32, freq: f64, amp: f64, secs: f64) -> Vec<f32> {
    let n = (f64::from(rate) * secs).round() as usize;
    (0..n)
        .map(|i| (amp * (TAU * freq * i as f64 / f64::from(rate)).sin()) as f32)
        .collect()
}

/// `channels` encoded in 1024-frame slices, as an export pushes them.
fn encode(rate: u32, high_quality: bool, channels: &[Vec<f32>]) -> Vec<u8> {
    let mut encoder = Mp3Encoder::new(rate, high_quality);
    let len = channels[0].len();
    for at in (0..len).step_by(1024) {
        let end = (at + 1024).min(len);
        let slices: Vec<&[f32]> = channels.iter().map(|c| &c[at..end]).collect();
        encoder.push(&slices);
    }
    encoder.finish()
}

/// Both channels of a decoded stereo file.
fn stereo(decoded: &SampleData) -> [&[f32]; 2] {
    let right = decoded.right.as_deref().expect("a stereo stream");
    [&decoded.left[..], right]
}

/// The lag in 0…4096 at which `want`, after that much silence, best matches the start of `got`. The silence pins the
/// alignment, which a periodic signal alone would leave open.
fn lag(got: &[f32], want: &[f32]) -> usize {
    const SPAN: usize = 4096 + 512;
    let error = |lag: usize| {
        let mut sum = 0.0;
        for i in 0..SPAN {
            let w = if i < lag { 0.0 } else { want[i - lag] };
            sum += f64::from(got[i] - w).powi(2);
        }
        sum
    };
    (0..=4096)
        .min_by(|&a, &b| error(a).total_cmp(&error(b)))
        .unwrap()
}

/// Signal-to-noise ratio in dB of `got` (from `lag` on) against all of `want`.
fn snr(got: &[f32], want: &[f32], lag: usize) -> f64 {
    let (signal, noise) = (got[lag..].iter().zip(want)).fold((0.0, 0.0), |(s, n), (&g, &w)| {
        (s + f64::from(w).powi(2), n + f64::from(g - w).powi(2))
    });
    10.0 * (signal / noise).log10()
}

/// CRC-16/ARC (x¹⁶+x¹⁵+x²+1 reflected, from 0), bit at a time: the LAME tag's.
fn crc16_arc(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in bytes {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                crc >> 1 ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// The 4-byte headers of an MPEG-1 Layer III stream's frames, walked by frame size to the end of the file.
fn headers(bytes: &[u8]) -> Vec<[u8; 4]> {
    const KBPS: [usize; 15] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    const RATES: [usize; 3] = [44_100, 48_000, 32_000];
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let h: [u8; 4] = bytes[at..at + 4].try_into().unwrap();
        assert_eq!(h[..2], [0xFF, 0xFB], "MPEG-1 Layer III without CRC at {at}");
        let kbps = KBPS[usize::from(h[2] >> 4)];
        let rate = RATES[usize::from(h[2] >> 2 & 3)];
        at += 144 * kbps * 1000 / rate + usize::from(h[2] >> 1 & 1);
        out.push(h);
    }
    assert_eq!(at, bytes.len(), "the last frame ends the file");
    out
}

#[test]
fn the_stream_is_cbr_192_kbps_or_320_with_high_quality_behind_an_info_frame() {
    let n: usize = 2 * 44_100;
    let audio = [sine(44_100, 440.0, 0.5, 2.0), sine(44_100, 660.0, 0.5, 2.0)];
    for (high_quality, index) in [(false, 0b1011), (true, 0b1110)] {
        let bytes = encode(44_100, high_quality, &audio);
        let headers = headers(&bytes);
        for h in &headers {
            assert_eq!(h[2] >> 4, index, "{high_quality}: bitrate index");
            assert_eq!(h[2] >> 2 & 3, 0, "{high_quality}: 44.1 kHz");
            assert!(h[3] >> 6 <= 1, "{high_quality}: stereo or joint stereo");
        }
        // The Info frame (the CBR Xing tag) after the header and MPEG-1 stereo's 32 bytes of side info: its frame
        // field counts the audio frames, its byte field the whole file. Then the audio, with the codec's delay of
        // silence after the input.
        assert_eq!(&bytes[36..40], b"Info");
        assert_eq!(bytes[40..44], 3u32.to_be_bytes());
        let frames = headers.len() - 1;
        assert_eq!(bytes[44..48], (frames as u32).to_be_bytes());
        assert_eq!(bytes[48..52], (bytes.len() as u32).to_be_bytes());
        assert_eq!(frames, (n + 1057).div_ceil(FRAME));
        // The LAME extension after it: the encoder string, the encoder's delay (528; decoders add their 529) and the
        // padding after the input, and the CRC of the frame up to it.
        assert_eq!(&bytes[52..56], b"LAME");
        let trim = u32::from_be_bytes([0, bytes[73], bytes[74], bytes[75]]) as usize;
        assert_eq!((trim >> 12, trim & 0xFFF), (528, frames * FRAME - 528 - n));
        assert_eq!(bytes[86..88], crc16_arc(&bytes[..86]).to_be_bytes());
    }
}

#[test]
fn a_ten_second_stereo_sine_decodes_whole_at_25_db_or_better() {
    let n = 10 * 44_100;
    let audio = [
        sine(44_100, 440.0, 0.5, 10.0),
        sine(44_100, 1000.0, 0.3, 10.0),
    ];
    let want = [&audio[0][..], &audio[1][..]];
    for high_quality in [false, true] {
        let decoded = decode(&encode(44_100, high_quality, &audio)).unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        let got = stereo(&decoded);
        // The LAME tag has the decoder trim the codec's delay and the padding: the input comes back whole, in place.
        assert_eq!(decoded.frames(), n, "{high_quality}");
        assert_eq!(lag(got[0], want[0]), 0, "{high_quality}");
        for c in 0..2 {
            let snr = snr(got[c], want[c], 0);
            assert!(snr >= 25.0, "{high_quality} channel {c}: {snr:.1} dB");
        }
    }
}

#[test]
fn every_length_decodes_exactly_whole_frames_or_not() {
    // Frame-filling and ragged lengths, MPEG-1 and MPEG-2 frames, and through halving (rounding a half sample up) and
    // linear resampling.
    for (rate, n, stream, out) in [
        (44_100, 40 * FRAME, 44_100, 40 * FRAME),
        (44_100, 40 * FRAME - 1, 44_100, 40 * FRAME - 1),
        (44_100, 40 * FRAME + 1, 44_100, 40 * FRAME + 1),
        (44_100, 1, 44_100, 1),
        (22_050, 40 * 576, 22_050, 40 * 576),
        (22_050, 12_345, 22_050, 12_345),
        (96_000, 12_345, 48_000, 6_173),
        (40_000, 12_345, 44_100, 13_610),
    ] {
        let tone: Vec<f32> = sine(rate, 1000.0, 0.5, 2.0).into_iter().take(n).collect();
        assert_eq!(tone.len(), n);
        let decoded = decode(&encode(rate, false, &[tone.clone(), tone])).unwrap();
        assert_eq!(decoded.sample_rate, stream, "{rate} {n}");
        assert_eq!(decoded.frames(), out, "{rate} {n}");
    }
}

#[test]
fn silence_encodes_to_silence() {
    let silence = vec![0.0; 44_100];
    for high_quality in [false, true] {
        let decoded = decode(&encode(
            44_100,
            high_quality,
            &[silence.clone(), silence.clone()],
        ))
        .unwrap();
        for channel in stereo(&decoded) {
            assert_eq!(channel.len(), 44_100);
            assert!(channel.iter().all(|&x| x == 0.0), "{high_quality}");
        }
    }
}

#[test]
fn mono_input_is_duplicated_into_a_stereo_stream() {
    let mono = sine(44_100, 440.0, 0.5, 1.0);
    let bytes = encode(44_100, false, std::slice::from_ref(&mono));
    assert_eq!(bytes, encode(44_100, false, &[mono.clone(), mono]));
    let decoded = decode(&bytes).unwrap();
    let [left, right] = stereo(&decoded);
    assert_eq!(left, right);
}

#[test]
fn rates_above_48_khz_are_halved_and_others_go_to_the_nearest_layer_iii_rate() {
    // Halved above 48 kHz, while that leaves 32 kHz or more. A rate that is then no Layer III rate is resampled
    // to the nearest one; 8 kHz, which rusty_mp3 codes wrongly, to 11.025 kHz.
    for (rate, stream) in [
        (8_000, 11_025),
        (22_050, 22_050),
        (32_000, 32_000),
        (44_100, 44_100),
        (48_000, 48_000),
        (64_000, 32_000),
        (88_200, 44_100),
        (96_000, 48_000),
        (176_400, 44_100),
        (192_000, 48_000),
        (40_000, 44_100),
        (100_000, 48_000),
        (50_000, 48_000),
    ] {
        // 500 Hz: 8 kHz's linear resampling is the weakest step, and a 1 kHz tone through it scores 25.0 dB.
        let tone = sine(rate, 500.0, 0.5, 0.5);
        let decoded = decode(&encode(rate, false, &[tone.clone(), tone])).unwrap();
        assert_eq!(decoded.sample_rate, stream, "{rate}");
        let want = sine(stream, 500.0, 0.5, 0.5);
        let got = stereo(&decoded);
        // Exactly the input's duration at the stream rate, in place (the filters add no delay), and whole to its end:
        // the conversion lets out what it holds back.
        let len = decoded.frames();
        assert_eq!(len, want.len(), "{rate}");
        assert_eq!(lag(got[0], &want), 0, "{rate}");
        let snr_all = snr(got[0], &want, 0);
        let snr_end = snr(&got[0][len - 256..], &want[len - 256..], 0);
        assert!(
            snr_all >= 25.0 && snr_end >= 25.0,
            "{rate}: {snr_all:.1} dB, the last 256 {snr_end:.1} dB"
        );
    }
}

#[test]
fn halving_filters_out_what_would_alias() {
    // 30 kHz at 96 kHz would fold to 18 kHz at 48 kHz: the low-pass removes it first.
    let tone = sine(96_000, 30_000.0, 0.5, 1.0);
    let decoded = decode(&encode(96_000, true, &[tone.clone(), tone])).unwrap();
    let rms =
        |x: &[f32]| (x.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
    let level = 20.0 * (rms(&decoded.left) / (0.5 / 2.0_f64.sqrt())).log10();
    assert!(level < -50.0, "{level:.1} dB");
}

#[test]
fn the_vendored_encoder_reads_no_environment_outside_its_tests() {
    // rusty_mp3's `MP3_*` debug knobs changed its output ; the vendored copy must not read them again.
    fn walk(dir: &std::path::Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|e| e == "rs") {
                // A `#[cfg(test)]` item runs to its closing `}` (or `;`) at the left margin: rustfmt's layout.
                let mut test = false;
                for (i, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
                    if line == "#[cfg(test)]" {
                        test = true;
                    } else if test {
                        let top = !line.starts_with([' ', '#', '/']);
                        test = !(line == "}" || top && line.ends_with(';'));
                    } else if line.contains("env::") || line.contains("env!") {
                        found.push(format!("{}:{}", path.display(), i + 1));
                    }
                }
            }
        }
    }
    let mut found = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/rusty_mp3/src"),
        &mut found,
    );
    assert!(found.is_empty(), "{found:?}");
}

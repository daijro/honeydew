use std::f64::consts::TAU;
use std::io::Cursor;

use honeydew_audio::flac_writer::{FlacEncoder, flac};
use honeydew_audio::sample::decode;
use honeydew_audio::wav_writer::{HEADER_LEN, WavWriter};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::default::{codecs::FlacDecoder, formats::FlacReader};

const STREAMINFO_END: usize = 4 + 4 + 34;

/// trunc(x·(2^(bits−1)−1)), clamped, NaN → 0: the `WavWriter` rule.
fn quantize(x: f32, bits: u8) -> i32 {
    let scale = f64::from((1u32 << (bits - 1)) - 1);
    (f64::from(x) * scale).clamp(-scale, scale) as i32
}

/// A float that quantizes to exactly `q`, even for 24-bit values near full scale where an `f32` is coarser than 1/2.
fn float_for(q: i32, bits: u8) -> f32 {
    let scale = f64::from((1u32 << (bits - 1)) - 1);
    ((f64::from(q) + 0.5 * f64::from(q.signum())) / scale) as f32
}

/// Every frame of every channel, as integers, straight from symphonia (no block is skipped, unlike `sample::decode`,
/// which turns blocks under 32 frames into silence).
fn decode_ints(bytes: &[u8], bits: u8) -> (u32, Vec<Vec<i32>>) {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut reader = FlacReader::try_new(source, &FormatOptions::default()).unwrap();
    let params = reader.default_track().unwrap().codec_params.clone();
    let mut decoder = FlacDecoder::try_new(&params, &DecoderOptions::default()).unwrap();
    let mut out: Vec<Vec<i32>> = vec![Vec::new(); params.channels.unwrap().count()];
    while let Ok(packet) = reader.next_packet() {
        let AudioBufferRef::S32(buf) = decoder.decode(&packet).unwrap() else {
            panic!("FLAC decodes to S32")
        };
        for (c, channel) in out.iter_mut().enumerate() {
            // symphonia left-aligns the samples in 32 bits.
            channel.extend(buf.chan(c).iter().map(|&s| s >> (32 - bits)));
        }
    }
    (params.sample_rate.unwrap(), out)
}

/// Encodes integer channels, decodes them again and asserts they are identical. Returns the file.
fn check(rate: u32, bits: u8, channels: &[Vec<i32>]) -> Vec<u8> {
    let floats: Vec<Vec<f32>> = channels
        .iter()
        .map(|c| c.iter().map(|&q| float_for(q, bits)).collect())
        .collect();
    let bytes = flac(rate, bits, &floats);
    let (got_rate, got) = decode_ints(&bytes, bits);
    assert_eq!(got_rate, rate);
    assert_eq!(got.len(), channels.len());
    for (c, (got, want)) in got.iter().zip(channels).enumerate() {
        assert_eq!(got.len(), want.len(), "channel {c} length");
        let at = got.iter().zip(want).position(|(g, w)| g != w);
        assert_eq!(at, None, "channel {c} first differing frame");
    }
    bytes
}

/// Bit-at-a-time CRCs, independent of the encoder's table.
fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in bytes {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                crc << 1 ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in bytes {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                crc << 1 ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// A FLAC UTF-8 style number at the start of `b`.
fn utf8_number(b: &[u8]) -> u64 {
    let len = b[0].leading_ones() as usize;
    if len == 0 {
        return u64::from(b[0]);
    }
    let mut n = u64::from(b[0]) & (0x7F >> len);
    for &c in &b[1..len] {
        assert_eq!(c >> 6, 0b10, "continuation byte");
        n = n << 6 | u64::from(c & 0x3F);
    }
    n
}

/// The frame number in every frame header, with the frame length symphonia found (`packet.dur`) and the
/// time stamp it derived from the number (a fixed-size stream's `number × block size`).
fn frame_numbers(bytes: &[u8]) -> Vec<(u64, u64, u64)> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut reader = FlacReader::try_new(source, &FormatOptions::default()).unwrap();
    let mut out = Vec::new();
    while let Ok(packet) = reader.next_packet() {
        let buf = packet.buf();
        assert_eq!(buf[..2], [0xFF, 0xF8], "frame starts at its sync code");
        out.push((utf8_number(&buf[4..]), packet.ts, packet.dur));
    }
    out
}

/// A deterministic xorshift noise source in [−1, 1).
fn noise(seed: u32, n: usize) -> Vec<f32> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f64 / 2_147_483_648.0 - 1.0) as f32
        })
        .collect()
}

/// Music-like: two partials, noise, a silent stretch and a click.
fn signal(seed: u32, rate: u32, n: usize) -> Vec<f32> {
    let hiss = noise(seed, n);
    (0..n)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            let tone = 0.4 * (TAU * 220.0 * t).sin() + 0.2 * (TAU * 1_330.0 * t).sin();
            let x = match i {
                3_000..4_500 => 0.0,
                7_777 => 1.0,
                _ => tone as f32 + 0.01 * hiss[i],
            };
            x.clamp(-1.0, 1.0)
        })
        .collect()
}

#[test]
fn round_trips_bit_exact_through_sample_decode() {
    for bits in [16u8, 24] {
        for channels in [1usize, 2] {
            for rate in [44_100u32, 48_000] {
                // Two full blocks and a 1808-frame tail.
                let pcm: Vec<Vec<f32>> = (0..channels)
                    .map(|c| signal(7 + c as u32, rate, 10_000))
                    .collect();
                let decoded = decode(&flac(rate, bits, &pcm)).unwrap();
                let label = format!("{bits}-bit, {channels} ch, {rate} Hz");
                assert_eq!(decoded.sample_rate, rate, "{label}");
                assert_eq!(decoded.right.is_some(), channels == 2, "{label}");
                let want_scale = (1u32 << (bits - 1)) as f32;
                for (got, pcm) in [Some(&decoded.left), decoded.right.as_ref()]
                    .into_iter()
                    .flatten()
                    .zip(&pcm)
                {
                    let want: Vec<f32> = pcm
                        .iter()
                        .map(|&x| quantize(x, bits) as f32 / want_scale)
                        .collect();
                    assert_eq!(&got[..], &want[..], "{label}");
                }
            }
        }
    }
}

#[test]
fn quantizes_exactly_as_wav_writer_does() {
    let mut x = noise(99, 6_000);
    // Out of range, non-finite, and values whose product with 32767 sits next to an integer.
    x.extend([
        2.0,
        -2.0,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        1.0,
        -1.0,
        0.0,
        -0.0,
    ]);
    x.extend((0..300).map(|i| i as f32 / 32_767.0));
    x.extend((0..300).map(|i| -(i as f32) / 32_767.0));
    let mut wav = WavWriter::new(Cursor::new(Vec::new()), 44_100, 1).unwrap();
    wav.write(&x).unwrap();
    let wav = wav.finish().unwrap().into_inner();
    let want: Vec<i32> = wav[HEADER_LEN..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i32::from(i16::from_le_bytes([b[0], b[1]])))
        .collect();
    let (_, got) = decode_ints(&flac(44_100, 16, &[x]), 16);
    assert_eq!(got, [want]);
}

#[test]
fn silence_is_tiny() {
    let bytes = flac(44_100, 16, &[vec![0.0; 3 * 4096]]);
    assert!(bytes.len() < 100, "{} bytes", bytes.len());
    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.frames(), 3 * 4096);
    assert!(decoded.left.iter().all(|&s| s == 0.0));
    let stereo = flac(48_000, 24, &[vec![0.0; 5000], vec![0.0; 5000]]);
    let decoded = decode(&stereo).unwrap();
    assert_eq!(decoded.frames(), 5000);
    assert!(decoded.right.unwrap().iter().all(|&s| s == 0.0));
}

#[test]
fn full_scale_survives() {
    for bits in [16u8, 24] {
        let max = (1i32 << (bits - 1)) - 1;
        // Alternating full scale needs the widest residuals; a square wave is mostly flat with full-scale edges.
        let alternating: Vec<i32> = (0..5000)
            .map(|i| if i % 2 == 0 { max } else { -max })
            .collect();
        let square: Vec<i32> = (0..5000)
            .map(|i| if i / 37 % 2 == 0 { max } else { -max })
            .collect();
        check(44_100, bits, &[alternating, square]);
        // Floats at and beyond ±1 clamp to ±max, never to −max−1.
        let beyond = [
            1.0,
            -1.0,
            1.5,
            -1.5,
            1e9,
            -1e9,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ];
        let (_, got) = decode_ints(&flac(44_100, bits, &[beyond]), bits);
        assert_eq!(
            got,
            [vec![max, -max, max, -max, max, -max, 0, max, -max]],
            "{bits}-bit"
        );
    }
}

#[test]
fn full_scale_noise_is_stored_verbatim() {
    for bits in [16u8, 24] {
        let max = (1i32 << (bits - 1)) - 1;
        let x: Vec<i32> = noise(5, 4096)
            .iter()
            .map(|&v| (v * max as f32) as i32)
            .collect();
        let bytes = check(44_100, bits, &[x]);
        // Header, no-compression subframe and 4096 raw samples (plus CRC-16 and the frame header).
        let frame = &bytes[STREAMINFO_END..];
        assert_eq!(
            frame[6], 0b0000_0010,
            "VERBATIM subframe header, {bits}-bit"
        );
        assert_eq!(frame.len(), 6 + 1 + 4096 * usize::from(bits) / 8 + 2);
    }
}

#[test]
fn tail_blocks_of_every_awkward_size() {
    // 1 and 2 frames (under every predictor order), the 8-bit and 16-bit block size fields' edges, one short of a block,
    // an exact block, and a block plus a tail.
    for bits in [16u8, 24] {
        for n in [
            1usize, 2, 3, 5, 31, 32, 255, 256, 257, 1000, 4095, 4096, 4097, 8192, 8193,
        ] {
            let wave = signal(n as u32, 44_100, n);
            let ints = |s: &[f32]| s.iter().map(|&x| quantize(x, bits)).collect::<Vec<_>>();
            let channels = [
                ints(&wave),
                ints(&wave.iter().rev().copied().collect::<Vec<_>>()),
            ];
            check(44_100, bits, &channels);
            check(44_100, bits, &channels[..1]);
        }
    }
}

#[test]
fn a_one_frame_tail_after_a_full_block_is_exact() {
    let ints: Vec<i32> = (0..4097).map(|i| (i * 7) % 2000 - 1000).collect();
    let bytes = check(
        48_000,
        16,
        &[ints.clone(), ints.iter().map(|v| -v).collect()],
    );
    // sample::decode keeps the length even though it silences blocks under 32 frames.
    assert_eq!(decode(&bytes).unwrap().frames(), 4097);
}

#[test]
fn many_frames_need_multi_byte_frame_numbers() {
    // 130 full blocks and a tail: frame numbers past 127 take two bytes.
    let n = 130 * 4096 + 123;
    let ints: Vec<i32> = (0..n).map(|i| (i * 37) % 4001 - 2000).collect();
    let bytes = check(44_100, 16, &[ints]);
    let frames = frame_numbers(&bytes);
    assert_eq!(frames.len(), 131);
    for (index, &(number, ts, dur)) in frames.iter().enumerate() {
        assert_eq!(number, index as u64, "number in frame {index}'s header");
        assert_eq!(ts, index as u64 * 4096, "time stamp of frame {index}");
        assert_eq!(dur, if index == 130 { 123 } else { 4096 }, "frame {index}");
    }
}

#[test]
fn fixed_predictor_order_follows_the_signal() {
    // A polynomial of degree d is constant under d differences and zero under d + 1, so the order that wins is d + 1
    // (4 for a quartic). The subframe header follows the 7-byte frame header of a short mono frame.
    let poly = |n: i32, f: fn(i32) -> i32| (0..n).map(f).collect::<Vec<i32>>();
    let cases: [(&str, Vec<i32>, u8); 6] = [
        ("silence", vec![0; 200], 0),
        ("DC", vec![1234; 200], 1),
        ("ramp", poly(200, |t| 100 + 7 * t), 2),
        ("parabola", poly(200, |t| 3 * t * t - 50 * t + 9), 3),
        ("cubic", poly(200, |t| t * t * t - 300 * t + 5), 4),
        ("quartic", poly(40, |t| t * t * t * t + 11 * t), 4),
    ];
    for (name, ints, order) in cases {
        let bytes = check(44_100, 24, &[ints]);
        assert_eq!(bytes[STREAMINFO_END + 7], (8 + order) << 1, "{name}");
    }
}

#[test]
fn a_sine_compresses_below_70_percent_of_the_wav() {
    let sine: Vec<f32> = (0..44_100)
        .map(|i| (0.5 * (TAU * 440.0 * f64::from(i) / 44_100.0).sin()) as f32)
        .collect();
    let mut wav = WavWriter::new(Cursor::new(Vec::new()), 44_100, 1).unwrap();
    wav.write(&sine).unwrap();
    let wav = wav.finish().unwrap().into_inner();
    let bytes = flac(44_100, 16, &[sine]);
    assert_eq!(decode(&bytes).unwrap().frames(), 44_100);
    assert!(
        bytes.len() * 10 < wav.len() * 7,
        "{} bytes against {} for the WAV",
        bytes.len(),
        wav.len()
    );
}

#[test]
fn headers_say_what_is_inside() {
    let ints: Vec<i32> = (0..100).map(|i| i * 3 - 150).collect();
    let bytes = check(48_000, 16, &[ints.clone(), ints]);
    assert_eq!(&bytes[..8], b"fLaC\x80\0\0\x22");
    let info = &bytes[8..STREAMINFO_END];
    assert_eq!(&info[..4], [0x10, 0x00, 0x10, 0x00], "block sizes 4096");
    let frame_len = bytes.len() - STREAMINFO_END;
    assert_eq!(
        &info[4..7],
        &(frame_len as u32).to_be_bytes()[1..],
        "min frame size"
    );
    assert_eq!(
        &info[7..10],
        &(frame_len as u32).to_be_bytes()[1..],
        "max frame size"
    );
    // 20-bit rate, 3-bit channels − 1, 5-bit depth − 1, 36-bit frame count.
    let packed = u64::from_be_bytes(info[10..18].try_into().unwrap());
    assert_eq!(packed >> 44, 48_000);
    assert_eq!((packed >> 41) & 7, 1);
    assert_eq!((packed >> 36) & 31, 15);
    assert_eq!(packed & ((1 << 36) - 1), 100);
    assert_eq!(&info[18..], [0; 16], "MD5 unknown");
}

#[test]
fn a_short_noisy_block_does_not_get_a_predictor_it_cannot_afford() {
    // 5 samples at 24 bits: order 4 would store 4 raw warm-up samples and code one residual, which costs more than
    // storing all five. Counting the warm-up keeps the order under 4 (verbatim here).
    for seed in 1..=20 {
        let x: Vec<i32> = noise(seed, 5)
            .iter()
            .map(|&v| (v * 8_388_607.0) as i32)
            .collect();
        let bytes = check(44_100, 24, &[x]);
        let subframe_header = bytes[STREAMINFO_END + 7];
        assert!(
            subframe_header < (8 + 4) << 1,
            "seed {seed}: header {subframe_header:#04x}"
        );
    }
}

#[test]
fn the_frame_header_codes_the_rate() {
    // (rate, 4-bit code, bytes that follow the block size field). Code 0 (rate from STREAMINFO) is the fallback for
    // the rates no other code can say.
    #[rustfmt::skip]
    let cases: [(u32, u8, &[u8]); 21] = [
        (88_200, 1, &[]), (176_400, 2, &[]), (192_000, 3, &[]), (8_000, 4, &[]), (16_000, 5, &[]),
        (22_050, 6, &[]), (24_000, 7, &[]), (32_000, 8, &[]), (44_100, 9, &[]), (48_000, 10, &[]), (96_000, 11, &[]),
        // 8-bit kHz, 16-bit Hz, 16-bit tens of Hz.
        (1_000, 12, &[1]), (64_000, 12, &[64]), (255_000, 12, &[255]),
        (1, 13, &[0, 1]), (11_025, 13, &[0x2B, 0x11]), (65_535, 13, &[0xFF, 0xFF]),
        (100_010, 14, &[0x27, 0x11]), (256_000, 14, &[0x64, 0x00]), (655_350, 14, &[0xFF, 0xFF]),
        (100_001, 0, &[]),
    ];
    for (rate, code, extra) in cases {
        let ints: Vec<i32> = (0..100).map(|i| i * 5 - 250).collect();
        let bytes = check(rate, 16, &[ints]);
        let frame = &bytes[STREAMINFO_END..];
        // 100 frames: 8-bit size field (code 6).
        assert_eq!(frame[2], 0x60 | code, "{rate} Hz: code");
        assert_eq!(&frame[6..6 + extra.len()], extra, "{rate} Hz: extra bytes");
        let end = 6 + extra.len();
        assert_eq!(frame[end], crc8(&frame[..end]), "{rate} Hz: header CRC-8");
        // The subframe header (pad bit, type, wasted-bits flag) follows the CRC.
        assert_eq!(frame[end + 1] & 0x81, 0, "{rate} Hz: subframe header");
    }
    // 65 536 Hz is not a whole kHz, over 16 bits, and not a multiple of 10: only STREAMINFO can say it.
    let bytes = check(65_536, 16, &[vec![0, 1, 2]]);
    assert_eq!(bytes[STREAMINFO_END + 2] & 15, 0);
}

/// The frame's own CRCs, with bit-at-a-time implementations independent of the encoder's.
#[test]
fn the_single_frame_checks_out_against_both_crcs() {
    let bytes = flac(
        44_100,
        24,
        &[signal(1, 44_100, 200), signal(2, 44_100, 200)],
    );
    let frame = &bytes[STREAMINFO_END..];
    // Sync, fixed blocking; 200 frames = 8-bit size field; 44.1 kHz; mid/side (the two share their tones); 24 bits.
    assert_eq!(frame[..5], [0xFF, 0xF8, 0x69, 0xAC, 0x00]);
    assert_eq!(frame[5], 199);
    assert_eq!(frame[6], crc8(&frame[..6]));
    assert_eq!(
        frame[frame.len() - 2..],
        crc16(&frame[..frame.len() - 2]).to_be_bytes()
    );
}

#[test]
fn the_streaming_encoder_writes_the_same_bytes_as_flac() {
    const FRAMES: usize = 20_000;
    for (bits, channels) in [(16u8, 1usize), (24, 2)] {
        let pcm: Vec<Vec<f32>> = (0..channels)
            .map(|c| signal(c as u32 + 1, 44_100, FRAMES))
            .collect();
        let mut encoder = FlacEncoder::new(44_100, bits, channels);
        // Slices that stop short of a block, end on one, straddle one and span several; one is empty.
        let mut at = 0usize;
        for len in [1usize, 0, 1_000, 3_095, 4_096, 4_097, 1, 5_000, usize::MAX] {
            let end = at.saturating_add(len).min(FRAMES);
            let slice: Vec<&[f32]> = pcm.iter().map(|c| &c[at..end]).collect();
            encoder.push(&slice);
            at = end;
        }
        assert_eq!(at, FRAMES);
        assert_eq!(
            encoder.finish(),
            flac(44_100, bits, &pcm),
            "{bits}-bit, {channels} ch"
        );
    }
    // Nothing pushed is the empty file.
    assert_eq!(
        FlacEncoder::new(48_000, 24, 2).finish(),
        flac(48_000, 24, &[Vec::<f32>::new(), Vec::new()])
    );
}

#[test]
fn a_pushed_slice_of_uneven_channels_uses_the_shortest() {
    let mut encoder = FlacEncoder::new(44_100, 16, 2);
    encoder.push(&[vec![0.25; 700], vec![-0.25; 500]]);
    encoder.push(&[vec![0.5; 10], vec![-0.5; 10]]);
    let (_, got) = decode_ints(&encoder.finish(), 16);
    assert_eq!((got[0].len(), got[1].len()), (510, 510));
    assert_eq!((got[0][500], got[1][500]), (16_383, -16_383));
}

#[test]
#[should_panic(expected = "one slice per channel")]
fn pushing_the_wrong_number_of_channels_is_refused() {
    FlacEncoder::new(44_100, 16, 2).push(&[vec![0.0; 10]]);
}

#[test]
fn frame_numbers_past_2047_take_three_bytes() {
    // 2 050 silent blocks, streamed so no 8.4 M-sample buffer exists.
    let mut encoder = FlacEncoder::new(8_000, 16, 1);
    let silence = vec![0.0f32; 4096];
    for _ in 0..2_050 {
        encoder.push(&[&silence]);
    }
    let frames = frame_numbers(&encoder.finish());
    assert_eq!(frames.len(), 2_050);
    for (index, &(number, ts, dur)) in frames.iter().enumerate() {
        assert_eq!((number, ts, dur), (index as u64, index as u64 * 4096, 4096));
    }
}

#[test]
fn uneven_channels_use_the_shortest() {
    let bytes = flac(44_100, 16, &[vec![0.25; 700], vec![-0.25; 500]]);
    let (_, got) = decode_ints(&bytes, 16);
    assert_eq!(got[0].len(), 500);
    assert_eq!(got[1].len(), 500);
}

#[test]
fn no_audio_is_just_the_header() {
    let bytes = flac(44_100, 16, &[Vec::<f32>::new()]);
    assert_eq!(bytes.len(), STREAMINFO_END);
    assert_eq!(
        &bytes[8 + 10..8 + 18],
        &[0x0A, 0xC4, 0x40, 0xF0, 0, 0, 0, 0]
    );
}

#[test]
#[should_panic(expected = "16 or 24")]
fn eight_bit_is_refused() {
    flac(44_100, 8, &[vec![0.0; 10]]);
}

#[test]
#[should_panic(expected = "channels")]
fn no_channels_is_refused() {
    flac::<Vec<f32>>(44_100, 16, &[]);
}

#[test]
#[should_panic(expected = "rate")]
fn a_zero_rate_is_refused() {
    flac(0, 16, &[vec![0.0; 10]]);
}

#[test]
fn i16_samples_round_trip_exactly_and_flsm_reads_back() {
    use honeydew_audio::sample::SampleMeta;
    // Full-range integers: two full blocks and a 1808-frame tail.
    let ints = |seed: u32| -> Vec<i16> {
        let mut v: Vec<i16> = (0..10_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) ^ seed) as i16)
            .collect();
        (v[0], v[1]) = (i16::MIN, i16::MAX);
        v
    };
    let pcm = [ints(0), ints(0xBEEF)];
    let meta = SampleMeta {
        tempo: Some(128.0),
        beats: Some(8),
        loop_frames: Some((0, 9_000)),
        cues: vec![0, 2_500, 5_000, 7_500],
        ..SampleMeta::default()
    };
    for bits in [16u8, 24] {
        let mut encoder = FlacEncoder::new(44_100, bits, 2);
        encoder.push_i16(&[&pcm[0][..3_000], &pcm[1][..3_000]]);
        encoder.push_i16(&[&pcm[0][3_000..], &pcm[1][3_000..]]);
        let bytes = encoder.finish_with_meta(&meta);
        let (_, got) = decode_ints(&bytes, bits);
        for (got, want) in got.iter().zip(&pcm) {
            let want: Vec<i32> = want.iter().map(|&s| i32::from(s) << (bits - 16)).collect();
            assert_eq!(got, &want, "{bits}-bit");
        }
        assert_eq!(decode(&bytes).unwrap().meta, meta, "{bits}-bit");
    }
    // No metadata, no block: `finish` is unchanged.
    let mut plain = FlacEncoder::new(44_100, 16, 1);
    plain.push_i16(&[&pcm[0]]);
    let mut same = FlacEncoder::new(44_100, 16, 1);
    same.push_i16(&[&pcm[0]]);
    assert_eq!(
        plain.finish(),
        same.finish_with_meta(&SampleMeta::default())
    );
}

#[test]
fn correlated_stereo_and_noise_round_trip_and_shrink() {
    // A decaying inharmonic chord, the right channel a quieter copy plus hiss: LPC and a side channel pay off. White
    // noise gets nothing to predict. Both decode bit-exactly, 24-bit side channels (25 bits) included.
    let n = 20_000;
    let hiss = noise(3, n);
    let chord: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 / 44_100.0;
            (-3.0 * t).exp()
                * (0.5 * (TAU * 196.0 * t).sin()
                    + 0.3 * (TAU * 311.1 * t).sin()
                    + 0.2 * (TAU * 1_047.3 * t).sin())
        })
        .collect();
    for bits in [16u8, 24] {
        let max = f64::from((1 << (bits - 1)) - 1);
        let left = chord.iter().map(|&x| (0.9 * x * max) as i32).collect();
        let right = chord
            .iter()
            .zip(&hiss)
            .map(|(&x, &h)| ((0.7 * x + 0.001 * f64::from(h)) * max) as i32)
            .collect();
        let bytes = check(44_100, bits, &[left, right]);
        let white: Vec<Vec<i32>> = (1..=2)
            .map(|seed| {
                noise(seed, 9_000)
                    .iter()
                    .map(|&v| (f64::from(v) * 0.5 * max) as i32)
                    .collect()
            })
            .collect();
        check(44_100, bits, &white);
        if bits == 16 {
            // FIXED-only, independent channels took 29 659 bytes.
            assert!(bytes.len() < 26_000, "{} bytes", bytes.len());
        }
    }
}

#[test]
fn loud_24_bit_material_takes_wide_rice_parameters_and_round_trips() {
    // Low-passed Gaussian-ish noise near full scale: the LPC residual is ~2^19, past the 4-bit parameter field's 14, so
    // the residual coding method with 5-bit parameters must code it (an escape would spend ~23 bits a sample).
    let n = 10_000;
    let pcm: Vec<Vec<f32>> = (0..2u32)
        .map(|c| {
            let hiss: Vec<Vec<f32>> = (1..=4).map(|s| noise(10 * c + s, n)).collect();
            let mut y = 0.0f64;
            (0..n)
                .map(|i| {
                    let e: f64 = hiss.iter().map(|h| f64::from(h[i])).sum::<f64>() / 4.0;
                    y = 0.95 * y + 0.27 * e;
                    y.clamp(-1.0, 1.0) as f32
                })
                .collect()
        })
        .collect();
    let bytes = flac(44_100, 24, &pcm);
    let decoded = decode(&bytes).unwrap();
    for (got, pcm) in [&decoded.left, decoded.right.as_ref().unwrap()]
        .into_iter()
        .zip(&pcm)
    {
        let want: Vec<f32> = pcm
            .iter()
            .map(|&x| quantize(x, 24) as f32 / 8_388_608.0)
            .collect();
        assert_eq!(&got[..], &want[..]);
    }
    // Method 0 alone, escaping, took 55 603 bytes (22.2 bits a sample).
    assert!(bytes.len() < 54_500, "{} bytes", bytes.len());
}

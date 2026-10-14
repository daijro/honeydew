use std::f64::consts::TAU;
use std::io::{Cursor, Seek};

use honeydew_audio::sample::decode;
use honeydew_audio::wav_writer::{HEADER_LEN, WavWriter, float32_wav};

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn pcm16(bytes: &[u8]) -> Vec<i16> {
    bytes[HEADER_LEN..]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect()
}

fn record(rate: u32, channels: u16, blocks: &[&[f32]]) -> Vec<u8> {
    let mut w = WavWriter::new(Cursor::new(Vec::new()), rate, channels).unwrap();
    for block in blocks {
        w.write(block).unwrap();
    }
    w.finish().unwrap().into_inner()
}

#[test]
fn pcm16_header_is_the_46_byte_layout_with_cb_size_0() {
    let bytes = record(44_100, 2, &[&[0.5, -0.5, 1.0, -1.0]]);
    let mut expected = Vec::new();
    expected.extend_from_slice(b"RIFF");
    expected.extend_from_slice(&(38u32 + 8).to_le_bytes());
    expected.extend_from_slice(b"WAVEfmt ");
    expected.extend_from_slice(&18u32.to_le_bytes());
    expected.extend_from_slice(&1u16.to_le_bytes()); // PCM
    expected.extend_from_slice(&2u16.to_le_bytes()); // channels
    expected.extend_from_slice(&44_100u32.to_le_bytes());
    expected.extend_from_slice(&(44_100u32 * 4).to_le_bytes()); // byte rate
    expected.extend_from_slice(&4u16.to_le_bytes()); // block align
    expected.extend_from_slice(&16u16.to_le_bytes()); // bits
    expected.extend_from_slice(&0u16.to_le_bytes()); // cbSize
    expected.extend_from_slice(b"data");
    expected.extend_from_slice(&8u32.to_le_bytes());
    assert_eq!(expected.len(), HEADER_LEN);
    assert_eq!(bytes[..HEADER_LEN], expected[..]);
    assert_eq!(pcm16(&bytes), [16383, -16383, 32767, -32767]);
}

#[test]
fn samples_truncate_toward_zero_and_clamp_to_32767() {
    let bytes = record(
        48_000,
        1,
        &[&[
            2.0,
            -2.0,
            0.999_99,
            -0.999_99,
            0.25,
            -0.75,
            1e-6,
            f32::NAN,
            f32::INFINITY,
        ]],
    );
    let x = |v: f32| (f64::from(v) * 32767.0).trunc() as i16;
    assert_eq!(
        pcm16(&bytes),
        [
            32767,
            -32767,
            x(0.999_99),
            x(-0.999_99),
            8191,
            -24575,
            0,
            0,
            32767
        ]
    );
    assert_eq!(le32(&bytes, 42), 18);
}

#[test]
fn the_product_is_taken_in_f64_before_the_truncation() {
    let below_one = 0.999_999_94f32; // the largest f32 below 1
    // 32767·x is 0.999999999 exactly, but 1.0 once rounded to f32.
    let rounds_up = f32::from_bits(0x3800_0100);
    assert_eq!((rounds_up * 32767.0).trunc(), 1.0);
    let bytes = record(
        44_100,
        1,
        &[&[below_one, -below_one, rounds_up, -rounds_up]],
    );
    assert_eq!(pcm16(&bytes), [32766, -32766, 0, 0]);
}

#[test]
fn a_one_second_sine_round_trips_within_one_lsb() {
    let rate = 44_100;
    let sine: Vec<f32> = (0..rate)
        .flat_map(|i| {
            let t = f64::from(i) / f64::from(rate);
            [
                (0.9 * (TAU * 441.0 * t).sin()) as f32,
                (-0.5 * (TAU * 1000.0 * t).sin()) as f32,
            ]
        })
        .collect();
    // Streamed in uneven blocks, as a recorder drains them.
    let (a, rest) = sine.split_at(2 * 1000);
    let (b, c) = rest.split_at(2 * 33_333);
    let bytes = record(rate, 2, &[a, b, &[], c]);
    assert_eq!(bytes.len(), HEADER_LEN + sine.len() * 2);

    let d = decode(&bytes).unwrap();
    assert_eq!(d.sample_rate, rate);
    assert_eq!(d.frames(), rate as usize);
    let right = d.right.as_ref().unwrap();
    for (i, frame) in sine.chunks_exact(2).enumerate() {
        for (x, y) in [(frame[0], d.left[i]), (frame[1], right[i])] {
            // The decoder reads 16-bit as code/32768, the writer stores trunc(x·32767).
            let code = f64::from(y) * 32768.0;
            assert_eq!(code, (f64::from(x) * 32767.0).trunc(), "frame {i}");
            assert!((code - f64::from(x) * 32767.0).abs() < 1.0, "frame {i}");
        }
    }
}

#[test]
fn an_unfinished_recording_still_decodes() {
    let mut out = Cursor::new(Vec::new());
    let mut w = WavWriter::new(&mut out, 22_050, 1).unwrap();
    w.write(&[0.5; 100]).unwrap();
    drop(w); // no `finish`: a crash mid-recording
    let bytes = out.into_inner();
    assert_eq!(le32(&bytes, 4), u32::MAX);
    let d = decode(&bytes).unwrap();
    assert_eq!(d.frames(), 100);
    assert!(d.right.is_none());
}

#[test]
fn a_partial_frame_is_an_error_and_writes_nothing() {
    let mut w = WavWriter::new(Cursor::new(Vec::new()), 44_100, 2).unwrap();
    w.write(&[0.5, -0.5]).unwrap();
    let error = w.write(&[0.25, 0.25, 0.25]).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    w.write(&[0.0, 0.0]).unwrap(); // the stream stays frame-aligned
    let bytes = w.finish().unwrap().into_inner();
    assert_eq!(pcm16(&bytes), [16383, -16383, 0, 0]);
}

#[test]
fn finish_patches_sizes_relative_to_where_the_writer_started() {
    let mut out = Cursor::new(b"prefix".to_vec());
    out.set_position(6);
    let mut w = WavWriter::new(out, 8_000, 1).unwrap();
    w.write(&[0.0, 0.5]).unwrap();
    let mut out = w.finish().unwrap();
    assert_eq!(out.stream_position().unwrap(), (6 + HEADER_LEN + 4) as u64);
    let bytes = out.into_inner();
    assert_eq!(&bytes[..6], b"prefix");
    assert_eq!(le32(&bytes, 6 + 4), 38 + 4);
    assert_eq!(le32(&bytes, 6 + 42), 4);
    assert_eq!(bytes.len(), 6 + HEADER_LEN + 4);
}

#[test]
fn float32_wav_is_the_same_layout_with_format_3() {
    let left = [0.25f32, -1.5, 0.0];
    let right = [1.0f32, 0.125, -0.0];
    let bytes = float32_wav(48_000, &[&left[..], &right[..]]);
    assert_eq!(bytes.len(), HEADER_LEN + 3 * 2 * 4);
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(le32(&bytes, 4), 38 + 24);
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    assert_eq!(le32(&bytes, 16), 18);
    assert_eq!(le16(&bytes, 20), 3);
    assert_eq!(le16(&bytes, 22), 2);
    assert_eq!(le32(&bytes, 24), 48_000);
    assert_eq!(le32(&bytes, 28), 48_000 * 8);
    assert_eq!(le16(&bytes, 32), 8);
    assert_eq!(le16(&bytes, 34), 32);
    assert_eq!(le16(&bytes, 36), 0);
    assert_eq!(&bytes[38..42], b"data");
    assert_eq!(le32(&bytes, 42), 24);
    let data: Vec<f32> = bytes[HEADER_LEN..]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(data, [0.25, 1.0, -1.5, 0.125, 0.0, -0.0]);

    let d = decode(&bytes).unwrap();
    assert_eq!(d.sample_rate, 48_000);
    assert_eq!(&d.left[..], &left[..]);
    assert_eq!(&d.right.unwrap()[..], &right[..]);
}

#[test]
fn float32_wav_mono_round_trips_bit_exact() {
    let mono: Vec<f32> = (0..1000).map(|i| ((i as f32) * 0.37).sin() * 0.8).collect();
    let d = decode(&float32_wav(44_100, &[&mono])).unwrap();
    assert_eq!(&d.left[..], &mono[..]);
    assert!(d.right.is_none());
}

use std::f64::consts::TAU;

use honeydew_audio::ogg_writer::{OggEncoder, VENDOR};
use honeydew_audio::sample::{SampleData, decode};

/// `secs` of a sine at `freq` Hz and `amp` at `rate` Hz.
fn sine(rate: u32, freq: f64, amp: f64, secs: f64) -> Vec<f32> {
    let n = (f64::from(rate) * secs).round() as usize;
    (0..n)
        .map(|i| (amp * (TAU * freq * i as f64 / f64::from(rate)).sin()) as f32)
        .collect()
}

/// `channels` encoded in 1024-frame slices, as an export pushes them.
fn encode(rate: u32, high_quality: bool, channels: &[Vec<f32>]) -> Vec<u8> {
    let mut encoder = OggEncoder::new(rate, high_quality);
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

/// Signal-to-noise ratio in dB of `got` against `want`, sample for sample.
fn snr(got: &[f32], want: &[f32]) -> f64 {
    let (signal, noise) = (got.iter().zip(want)).fold((0.0, 0.0), |(s, n), (&g, &w)| {
        (s + f64::from(w).powi(2), n + f64::from(g - w).powi(2))
    });
    10.0 * (signal / noise).log10()
}

/// One Ogg page (RFC 3533 §6).
struct Page<'a> {
    flags: u8,
    granule: u64,
    serial: u32,
    sequence: u32,
    crc: u32,
    /// The whole page, its CRC field as written.
    bytes: &'a [u8],
    lacing: &'a [u8],
    body: &'a [u8],
}

/// The pages of `bytes`, which must be nothing but pages, back to back.
fn pages(bytes: &[u8]) -> Vec<Page<'_>> {
    let le = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap());
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let header = &bytes[at..at + 27];
        assert_eq!(&header[..5], b"OggS\0", "a version 0 page at {at}");
        let lacing = &bytes[at + 27..at + 27 + usize::from(header[26])];
        let len = 27 + lacing.len() + lacing.iter().map(|&l| usize::from(l)).sum::<usize>();
        let page = &bytes[at..at + len];
        out.push(Page {
            flags: header[5],
            granule: u64::from_le_bytes(header[6..14].try_into().unwrap()),
            serial: le(&header[14..18]),
            sequence: le(&header[18..22]),
            crc: le(&header[22..26]),
            bytes: page,
            lacing,
            body: &page[27 + lacing.len()..],
        });
        at += len;
    }
    out
}

/// The Ogg CRC-32 (polynomial 0x04C11DB7, not reflected, initial 0) of `page` with its CRC field as zero.
fn crc(page: &[u8]) -> u32 {
    let mut crc = 0u32;
    for (i, &byte) in page.iter().enumerate() {
        crc ^= u32::from(if (22..26).contains(&i) { 0 } else { byte }) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[test]
fn the_three_headers_open_the_stream_each_on_its_own_page() {
    let audio = [sine(44_100, 440.0, 0.5, 1.0), sine(44_100, 660.0, 0.5, 1.0)];
    // libvorbis's nominal bitrates at 44.1 kHz stereo: q4 (0.4) is 128 kb/s; q10 (1.0) interpolates its rate mapping
    // at setting 10.999, between 160 000 and 250 001 per channel.
    for (high_quality, nominal) in [(false, 128_000i32), (true, 499_821)] {
        let bytes = encode(44_100, high_quality, &audio);
        let pages = pages(&bytes);
        // Each header page holds one whole packet: its lacing ends in a value below 255 and has no other.
        let packet = |i: usize| {
            let page = &pages[i];
            let (last, full) = page.lacing.split_last().unwrap();
            assert!(*last < 255 && full.iter().all(|&l| l == 255), "{i}");
            assert_eq!(page.flags & 1, 0, "{i} continues nothing");
            assert_eq!(page.granule, 0, "{i}");
            page.body
        };
        // Identification (Vorbis I §4.2.2): version 0, 2 channels, 44 100 Hz, no upper or lower bitrate, block sizes
        // 256 and 2048, framing bit.
        let ident = packet(0);
        assert_eq!(pages[0].flags, 2, "BOS");
        assert_eq!(&ident[..7], b"\x01vorbis");
        assert_eq!(ident[7..11], 0u32.to_le_bytes());
        assert_eq!(ident[11], 2);
        assert_eq!(ident[12..16], 44_100u32.to_le_bytes());
        assert_eq!(ident[16..20], 0i32.to_le_bytes());
        assert_eq!(ident[20..24], nominal.to_le_bytes(), "{high_quality}");
        assert_eq!(ident[24..28], 0i32.to_le_bytes());
        assert_eq!(ident[28..], [0xB8, 1]);
        // Comment (§5.2.1): the vendor string, no user comments, framing bit.
        let comment = packet(1);
        assert_eq!(&comment[..7], b"\x03vorbis");
        assert_eq!(comment[7..11], (VENDOR.len() as u32).to_le_bytes());
        assert_eq!(&comment[11..11 + VENDOR.len()], VENDOR.as_bytes());
        assert_eq!(comment[11 + VENDOR.len()..], [0, 0, 0, 0, 1]);
        // Setup (§4.2.4).
        assert_eq!(&packet(2)[..7], b"\x05vorbis");
        // Then audio packets (bit 0 clear) on fresh pages.
        assert_eq!(pages[3].flags & 1, 0);
        assert_eq!(pages[3].body[0] & 1, 0);
    }
    assert_eq!(
        VENDOR,
        "zvidlib 0.5.0 (port of Xiph.Org libVorbis I 20200704)"
    );
}

#[test]
fn a_ten_second_stereo_sine_decodes_to_its_exact_length_at_25_db_or_better() {
    let n = 10 * 44_100;
    let audio = [
        sine(44_100, 440.0, 0.5, 10.0),
        sine(44_100, 1000.0, 0.3, 10.0),
    ];
    for high_quality in [false, true] {
        let bytes = encode(44_100, high_quality, &audio);
        assert_eq!(pages(&bytes).last().unwrap().granule, n as u64);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        // Vorbis has no codec delay: the last granule position trims the stream to the input, sample for sample.
        assert_eq!(decoded.frames(), n, "{high_quality}");
        for (c, got) in stereo(&decoded).into_iter().enumerate() {
            let snr = snr(got, &audio[c]);
            assert!(snr >= 25.0, "{high_quality} channel {c}: {snr:.1} dB");
        }
    }
}

#[test]
fn every_page_has_its_crc_and_the_stream_has_one_serial_bos_first_and_eos_last() {
    // Long enough for several audio pages: the writer fills a page's 255 lacing values (about 64 KiB) before it
    // starts the next.
    let audio = [sine(44_100, 440.0, 0.5, 6.0), sine(44_100, 660.0, 0.5, 6.0)];
    for high_quality in [false, true] {
        let bytes = encode(44_100, high_quality, &audio);
        let pages = pages(&bytes);
        let last = pages.len() - 1;
        assert!(last >= 4, "{high_quality}: {} pages", pages.len());
        let mut granule = 0;
        for (i, page) in pages.iter().enumerate() {
            assert_eq!(crc(page.bytes), page.crc, "{high_quality} page {i}");
            assert_eq!(page.serial, pages[0].serial, "{high_quality} page {i}");
            assert_eq!(page.sequence, i as u32, "{high_quality} page {i}");
            assert_eq!(page.flags & 2 != 0, i == 0, "{high_quality} page {i}: BOS");
            assert_eq!(
                page.flags & 4 != 0,
                i == last,
                "{high_quality} page {i}: EOS"
            );
            if page.granule != u64::MAX {
                assert!(page.granule >= granule, "{high_quality} page {i}");
                granule = page.granule;
            }
        }
        assert_eq!(pages[last].granule, audio[0].len() as u64);
        // The last page ends its packet.
        assert!(*pages[last].lacing.last().unwrap() < 255);
    }
}

#[test]
fn the_bytes_are_the_same_on_every_run() {
    let audio = [sine(44_100, 440.0, 0.5, 2.0), sine(44_100, 660.0, 0.3, 2.0)];
    for high_quality in [false, true] {
        assert_eq!(
            encode(44_100, high_quality, &audio),
            encode(44_100, high_quality, &audio),
            "{high_quality}"
        );
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
fn every_rate_up_to_200_khz_is_kept_and_higher_ones_are_averaged_down_under_it() {
    // No resampling ("Render options"), except where libvorbis has no setup: above 200 kHz, whole groups of frames
    // are averaged into one, and a last partial group into one more.
    for (rate, stream) in [
        (8_000, 8_000),
        (11_025, 11_025),
        (22_050, 22_050),
        (44_100, 44_100),
        (48_000, 48_000),
        (96_000, 96_000),
        (192_000, 192_000),
        (384_000, 192_000),
        (400_000, 200_000),
        (600_000, 200_000),
    ] {
        // Half a second and one frame: above 200 kHz the last group holds that one frame alone.
        let mut tone = sine(rate, 1000.0, 0.5, 0.5);
        tone.push(tone[tone.len() - 1]);
        let group = rate.div_ceil(200_000) as usize;
        let decoded = decode(&encode(rate, false, &[tone.clone(), tone.clone()])).unwrap();
        assert_eq!(decoded.sample_rate, stream, "{rate}");
        assert_eq!(decoded.frames(), tone.len().div_ceil(group), "{rate}");
        let want = sine(stream, 1000.0, 0.5, 0.5);
        let snr = snr(&decoded.left, &want);
        assert!(snr >= 25.0, "{rate}: {snr:.1} dB");
    }
}

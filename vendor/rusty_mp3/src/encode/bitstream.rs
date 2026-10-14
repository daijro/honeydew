//! Frame assembly + the encoder-side bit reservoir, and the side-info /
//! scalefactor serializers (bricks **B5**, **B6**, **B7**, **B8**).
//!
//! Writes the header (+ optional CRC), the side-information block, and the main
//! data. The reservoir lets a granule borrow unused bits donated by earlier
//! frames: `main_data_begin` records how far back this frame's main data starts,
//! so a complex granule can spend more than its nominal budget.

use crate::bitio::BitWriter;
use crate::decode::scalefactors::ScaleFactors;
use crate::frame::{BlockType, SideInfo};
use crate::header::{FrameHeader, MpegVersion};
use crate::tables;

/// scfsi band groups (long blocks) — the encode twin of decode's `SCFSI_GROUPS`.
const SCFSI_GROUPS: [(usize, usize); 4] = [(0, 6), (6, 11), (11, 16), (16, 21)];

// ── B5: side-information serializer ───────────────────────────────────────────

/// **B5** — serialize the side-information block, the exact inverse of
/// `decode/sideinfo.rs`. Produces exactly `header.side_info_len()` bytes (every
/// bit of the block is a defined field, so there is no padding). MPEG-1 only.
#[must_use]
pub fn serialize_side_info(header: &FrameHeader, si: &SideInfo) -> Vec<u8> {
    // Exactly `side_info_len()` bytes, every time -- see the doc comment above.
    let mut w = BitWriter::with_capacity(header.side_info_len());
    let nch = header.channel_mode.channels();
    let mpeg1 = matches!(header.version, MpegVersion::V1);

    if mpeg1 {
        w.write(u32::from(si.main_data_begin), 9);
        w.write(0, if nch == 1 { 5 } else { 3 }); // private bits
        for ch in 0..nch {
            for band in 0..4 {
                w.write(u32::from(si.scfsi[ch][band]), 1);
            }
        }
    } else {
        w.write(u32::from(si.main_data_begin), 8);
        w.write(0, if nch == 1 { 1 } else { 2 });
    }

    for gr in 0..header.version.granules() {
        for ch in 0..nch {
            let g = &si.granules[gr][ch];
            w.write(u32::from(g.part2_3_length), 12);
            w.write(u32::from(g.big_values), 9);
            w.write(u32::from(g.global_gain), 8);
            w.write(u32::from(g.scalefac_compress), if mpeg1 { 4 } else { 9 });
            w.write(u32::from(g.window_switching), 1);
            if g.window_switching {
                let bt = match g.block_type {
                    BlockType::Start => 1,
                    BlockType::Short => 2,
                    BlockType::Stop => 3,
                    // Long with window switching is invalid; the encoder never emits it.
                    BlockType::Long => 0,
                };
                w.write(bt, 2);
                w.write(u32::from(g.mixed_block), 1);
                for t in g.table_select.iter().take(2) {
                    w.write(u32::from(*t), 5);
                }
                for sg in &g.subblock_gain {
                    w.write(u32::from(*sg), 3);
                }
            } else {
                for t in &g.table_select {
                    w.write(u32::from(*t), 5);
                }
                w.write(u32::from(g.region0_count), 4);
                w.write(u32::from(g.region1_count), 3);
            }
            if mpeg1 {
                w.write(u32::from(g.preflag), 1);
            }
            w.write(u32::from(g.scalefac_scale), 1);
            w.write(u32::from(g.count1table_select), 1);
        }
    }
    w.finish()
}

// ── B6: scalefactor serializer ────────────────────────────────────────────────

/// **B6** — write one granule/channel's scalefactors into the main-data bitstream,
/// the inverse of `decode/scalefactors.rs`. Mirrors the band-major short-block
/// layout and the granule-1 `scfsi` reuse (skipped bands are not written). MPEG-1.
pub fn serialize_scalefactors(
    w: &mut BitWriter,
    header: &FrameHeader,
    si: &SideInfo,
    gr: usize,
    ch: usize,
    sf: &ScaleFactors,
) {
    if !matches!(header.version, MpegVersion::V1) {
        return; // brick: MPEG-2/2.5 scalefactor scheme
    }
    let gi = &si.granules[gr][ch];
    let (slen1, slen2) = tables::SCALEFAC_COMPRESS_V1[gi.scalefac_compress as usize & 0xF];
    let (s1, s2) = (u32::from(slen1), u32::from(slen2));

    if gi.window_switching && gi.block_type == BlockType::Short {
        // Band-major: for each sfb, its three windows (the decode-side gotcha).
        let start = if gi.mixed_block {
            for b in 0..8 {
                w.write(u32::from(sf.long[b]), s1);
            }
            3
        } else {
            0
        };
        for sfb in start..12 {
            let slen = if sfb < 6 { s1 } else { s2 };
            for window in 0..3 {
                w.write(u32::from(sf.short[window][sfb]), slen);
            }
        }
    } else {
        for (g, &(lo, hi)) in SCFSI_GROUPS.iter().enumerate() {
            for b in lo..hi {
                let slen = if b < 11 { s1 } else { s2 };
                // Granule 1 reuses granule 0's bands per scfsi — those aren't coded.
                if gr == 1 && si.scfsi[ch][g] {
                    continue;
                }
                w.write(u32::from(sf.long[b]), slen);
            }
        }
    }
}

// ── R3: Xing/Info tag frame ───────────────────────────────────────────────────

/// **R3** — build the Xing/Info header frame to prepend to a CBR stream so players
/// read an exact frame count (accurate duration + seeking) instead of estimating
/// from the bitrate. It is a valid, silent MPEG frame carrying the ASCII tag
/// (`Info` for CBR) right after the side info, at the canonical Xing offset.
///
/// `frame_count` and `byte_count` describe the whole file *including* this frame.
/// `vbr` selects the `Xing` tag (variable bitrate) over `Info` (constant).
#[must_use]
pub fn info_frame(header: &FrameHeader, frame_count: u32, byte_count: u32, vbr: bool) -> Vec<u8> {
    let mut out = header.to_bytes().to_vec();
    if header.crc_protected {
        out.extend_from_slice(&[0, 0]);
    }
    // Silent side info (all zero: main_data_begin 0, part2_3_length 0 → no audio).
    out.resize(out.len() + header.side_info_len(), 0);

    // The tag, at offset 4 + side_info_len (the canonical Xing position).
    out.extend_from_slice(if vbr { b"Xing" } else { b"Info" });
    out.extend_from_slice(&0x0000_0003u32.to_be_bytes()); // flags: frames | bytes
    out.extend_from_slice(&frame_count.to_be_bytes());
    out.extend_from_slice(&byte_count.to_be_bytes());

    out.resize(header.frame_size(), 0); // pad to a full frame
    out
}

/// Smallest MPEG-1 bitrate (kbps) whose frame can hold `main_data_bytes`, capped
/// at 320. Lets VBR size each frame to its content.
#[must_use]
pub fn smallest_bitrate_for(header: &FrameHeader, main_data_bytes: usize) -> u32 {
    for &br in &tables::BITRATE_V1_L3[1..15] {
        let mut h = header.clone();
        h.bitrate_kbps = br;
        if region_capacity(&h) >= main_data_bytes {
            return br;
        }
    }
    320
}

// ── B7: single-frame assembly (reservoir-free) ────────────────────────────────

/// Encoder-side reservoir: how many spare main-data bytes are banked.
#[derive(Debug, Clone, Default)]
pub struct EncReservoir {
    /// Spare bytes carried forward for future frames to borrow.
    pub spare_bytes: usize,
}

/// Physical main-data capacity of a frame (bytes after header/CRC/side-info).
#[must_use]
pub fn region_capacity(header: &FrameHeader) -> usize {
    let crc = if header.crc_protected { 2 } else { 0 };
    header
        .frame_size()
        .saturating_sub(4 + crc + header.side_info_len())
}

/// **B7** — assemble one complete MP3 frame: header (+ optional CRC) + side info +
/// main data, padded to the frame size.
///
/// This runs **reservoir-free** (`main_data_begin = 0`): each frame's main data
/// must fit within its own region, and the unused tail is zero stuffing. That is a
/// fully valid, decodable CBR frame. [`assemble_stream`] (**B8**) layers the
/// bit-reservoir *borrowing* on top when the whole frame sequence is known.
/// MP3 CRC-16 (ISO 11172-3 §2.4.3.1): computed over the last two header bytes
/// (bytes 2–3) followed by the whole side-information block, MSB-first, polynomial
/// `0x8005`, initial value `0xFFFF`. Written big-endian between header and side info
/// when `protection_bit` (our `crc_protected`) is set.
fn crc16(header_tail: [u8; 2], side_info: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &byte in header_tail.iter().chain(side_info) {
        for i in (0..8).rev() {
            let bit = u16::from((byte >> i) & 1);
            let msb = crc >> 15;
            crc <<= 1;
            if (msb ^ bit) & 1 == 1 {
                crc ^= 0x8005;
            }
        }
    }
    crc
}

pub fn format(
    header: &FrameHeader,
    side_info: &SideInfo,
    main_data: &[u8],
    reservoir: &mut EncReservoir,
) -> Vec<u8> {
    let mut si = side_info.clone();
    si.main_data_begin = 0;

    let hdr = header.to_bytes();
    let si_bytes = serialize_side_info(header, &si);
    // The frame's final size is fixed by the header; growing into it from four
    // bytes costs several reallocs per frame for nothing.
    let mut out = Vec::with_capacity(header.frame_size());
    out.extend_from_slice(&hdr);
    if header.crc_protected {
        out.extend_from_slice(&crc16([hdr[2], hdr[3]], &si_bytes).to_be_bytes());
    }
    out.extend_from_slice(&si_bytes);

    let region_cap = region_capacity(header);
    let used = main_data.len().min(region_cap);
    out.extend_from_slice(&main_data[..used]);
    out.resize(header.frame_size(), 0); // zero stuffing fills the rest

    reservoir.spare_bytes = region_cap - used;
    out
}

// ── B8: reservoir-aware stream assembly ───────────────────────────────────────

/// **B8** — assemble a whole sequence of frames using the bit reservoir.
///
/// The main data of all frames forms one continuous stream `MD`; each frame's
/// fixed-size physical region holds the next `C_n` bytes of `MD`, so a frame whose
/// data is shorter than its region leaves room that the *following* frame's data
/// flows into. `main_data_begin_n = P_n − S_n` (physical-region start minus the
/// frame's start in `MD`) tells the decoder how far back to reach. This is the
/// exact inverse of the decoder's rolling-buffer `Reservoir::assemble`, so the
/// stream round-trips; it also lets a complex frame spend more than its own
/// region by borrowing the slack earlier frames banked.
///
/// Each item is `(header, side_info, main_data)`; `side_info.main_data_begin` is
/// overwritten. Requires the cumulative data never to outrun cumulative capacity
/// (the rate loop's job) and `main_data_begin ≤ 511`.
#[must_use]
pub fn assemble_stream(frames: &[(FrameHeader, SideInfo, Vec<u8>)]) -> Vec<u8> {
    let (md, begins, caps) = reservoir_layout(frames);
    let mut out = Vec::with_capacity(frames.iter().map(|(h, _, _)| h.frame_size()).sum());
    let mut p = 0usize;
    for (n, (header, side_info, _)) in frames.iter().enumerate() {
        write_reservoir_frame(&mut out, header, side_info, begins[n], &md[p..p + caps[n]]);
        p += caps[n];
    }
    out
}

/// [`assemble_stream`], one `Vec` per frame -- the same bytes, cut at the frame
/// boundaries during assembly rather than by copying them out afterwards.
pub(crate) fn assemble_frames(frames: &[(FrameHeader, SideInfo, Vec<u8>)]) -> Vec<Vec<u8>> {
    let (md, begins, caps) = reservoir_layout(frames);
    let mut out = Vec::with_capacity(frames.len());
    let mut p = 0usize;
    for (n, (header, side_info, _)) in frames.iter().enumerate() {
        let mut frame = Vec::with_capacity(header.frame_size());
        write_reservoir_frame(
            &mut frame,
            header,
            side_info,
            begins[n],
            &md[p..p + caps[n]],
        );
        out.push(frame);
        p += caps[n];
    }
    out
}

/// Append one assembled frame: header, CRC, side info with its back-reference,
/// and the frame's physical main-data region.
fn write_reservoir_frame(
    out: &mut Vec<u8>,
    header: &FrameHeader,
    side_info: &SideInfo,
    begin: usize,
    region: &[u8],
) {
    let mut si = side_info.clone();
    si.main_data_begin = begin as u16;
    let hdr = header.to_bytes();
    let si_bytes = serialize_side_info(header, &si);
    out.extend_from_slice(&hdr);
    if header.crc_protected {
        out.extend_from_slice(&crc16([hdr[2], hdr[3]], &si_bytes).to_be_bytes());
    }
    out.extend_from_slice(&si_bytes);
    out.extend_from_slice(region);
}

/// The continuous main-data stream `MD`, each frame's `main_data_begin`, and
/// each frame's region capacity -- the layout half of [`assemble_stream`].
fn reservoir_layout(
    frames: &[(FrameHeader, SideInfo, Vec<u8>)],
) -> (Vec<u8>, Vec<usize>, Vec<usize>) {
    const MAX_BEGIN: usize = 511; // the 9-bit main_data_begin field

    let caps: Vec<usize> = frames.iter().map(|(h, _, _)| region_capacity(h)).collect();

    // Build the continuous main-data stream, recording each frame's begin. When
    // banked slack would exceed 511, insert stuffing so the back-reference fits
    // (the standard reservoir cap — the wasted bytes are unreferenced ancillary).
    // Reserve both whole-stream buffers up front. A `Vec` grown from empty by
    // appending pays for its contents TWICE -- once for the append and again in
    // realloc traffic, since doubling to N copies ~N bytes on the way -- and that
    // second cost is invisible to a copy census because it happens inside `Vec`,
    // in the allocator, not in this symbol. Both totals are known exactly here:
    // the main-data stream is the sum of the region capacities already computed
    // above, and the output is the sum of the frame sizes.
    let mut md = Vec::with_capacity(caps.iter().sum::<usize>());
    let mut begins = Vec::with_capacity(frames.len());
    let mut p = 0usize; // P_n
    for (n, (_, _, data)) in frames.iter().enumerate() {
        if p > md.len() + MAX_BEGIN {
            md.resize(p - MAX_BEGIN, 0); // stuffing → begin == MAX_BEGIN
        }
        let begin = if let Some(begin) = p.checked_sub(md.len()) {
            begin
        } else {
            // The banked data outran the capacity behind it. The rate loop does
            // not let that happen for any input the push boundary admits
            // (finite, |x| <= MAX_INPUT_AMPLITUDE: measured clean up to 4096x
            // full scale, i.e. a 512x margin). This used to `expect` -- a panic
            // reachable from hostile PCM. If it ever happens, drop the overflow
            // instead: the preceding frame is damaged, the stream stays valid.
            md.truncate(p);
            0
        };
        begins.push(begin);
        md.extend_from_slice(data);
        p += caps[n];
    }
    if md.len() < p {
        md.resize(p, 0); // pad the final region
    }
    (md, begins, caps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{ChannelMode, GranuleSideInfo};
    use crate::header::MpegVersion;

    fn hdr(channel_mode: ChannelMode) -> FrameHeader {
        FrameHeader {
            version: MpegVersion::V1,
            crc_protected: false,
            bitrate_kbps: 128,
            sample_rate: 44100,
            padding: false,
            channel_mode,
            copyright: false,
            original: true,
            emphasis: 0,
        }
    }

    #[test]
    fn crc16_matches_known_vector() {
        // MP3 CRC-16 == CRC-16/CMS (poly 0x8005, init 0xFFFF, MSB-first, no reflect/xorout).
        // Its canonical check value for the ASCII bytes "123456789" is 0xAEE7.
        assert_eq!(crc16([b'1', b'2'], b"3456789"), 0xAEE7);
        // A protected frame writes 2 CRC bytes between header and side info.
        let mut h = hdr(ChannelMode::Mono);
        h.crc_protected = true;
        let out = format(&h, &SideInfo::default(), &[], &mut EncReservoir::default());
        let want = crc16(
            [h.to_bytes()[2], h.to_bytes()[3]],
            &serialize_side_info(&h, &SideInfo::default()),
        );
        assert_eq!(
            &out[4..6],
            &want.to_be_bytes(),
            "CRC bytes follow the 4-byte header"
        );
    }

    #[test]
    fn side_info_round_trips_stereo_long() {
        let header = hdr(ChannelMode::Stereo);
        let mut si = SideInfo {
            main_data_begin: 42,
            scfsi: [[true, false, true, false], [false, true, false, true]],
            ..Default::default()
        };
        for gr in 0..2 {
            for ch in 0..2 {
                si.granules[gr][ch] = GranuleSideInfo {
                    part2_3_length: (100 + gr * 10 + ch) as u16,
                    big_values: (200 + ch) as u16,
                    global_gain: (120 + gr * 4) as u8,
                    scalefac_compress: 9,
                    table_select: [3, 7, 11],
                    region0_count: 7,
                    region1_count: 2,
                    preflag: gr == 0,
                    scalefac_scale: ch == 1,
                    count1table_select: gr == 1,
                    ..Default::default()
                };
            }
        }

        let bytes = serialize_side_info(&header, &si);
        assert_eq!(bytes.len(), header.side_info_len());
        let parsed = crate::decode::sideinfo::parse(&header, &bytes).unwrap();
        assert_eq!(parsed, si);
    }

    #[test]
    fn side_info_round_trips_mono_short() {
        let header = hdr(ChannelMode::Mono);
        let mut si = SideInfo {
            main_data_begin: 7,
            ..Default::default()
        };
        si.granules[0][0] = GranuleSideInfo {
            part2_3_length: 333,
            big_values: 50,
            global_gain: 150,
            scalefac_compress: 5,
            window_switching: true,
            block_type: BlockType::Short,
            mixed_block: false,
            table_select: [5, 9, 0],
            subblock_gain: [1, 2, 3],
            ..Default::default()
        };
        si.granules[1][0] = GranuleSideInfo {
            part2_3_length: 120,
            big_values: 10,
            global_gain: 130,
            window_switching: true,
            block_type: BlockType::Start,
            table_select: [2, 4, 0],
            subblock_gain: [0, 0, 0],
            ..Default::default()
        };

        let bytes = serialize_side_info(&header, &si);
        assert_eq!(bytes.len(), header.side_info_len());
        let parsed = crate::decode::sideinfo::parse(&header, &bytes).unwrap();
        assert_eq!(parsed, si);
    }

    /// Serialize scalefactors then read them back with the decoder.
    fn sf_round_trip(
        si: &SideInfo,
        gr: usize,
        ch: usize,
        sf: &ScaleFactors,
        prev: Option<&ScaleFactors>,
    ) {
        let header = hdr(ChannelMode::Mono);
        let mut w = BitWriter::new();
        serialize_scalefactors(&mut w, &header, si, gr, ch, sf);
        let bits = w.finish();
        let mut pos = 0;
        let got = crate::decode::scalefactors::decode(&bits, &mut pos, &header, si, gr, ch, prev);
        assert_eq!(&got, sf);
    }

    #[test]
    fn scalefactors_round_trip_long() {
        let mut si = SideInfo::default();
        si.granules[0][0] = GranuleSideInfo {
            scalefac_compress: 15, // (slen1,slen2)=(4,3)
            ..Default::default()
        };
        let mut sf = ScaleFactors::default();
        for b in 0..21 {
            sf.long[b] = (b as u8) % (if b < 11 { 16 } else { 8 });
        }
        sf_round_trip(&si, 0, 0, &sf, None);
    }

    #[test]
    fn scalefactors_round_trip_short_band_major() {
        let mut si = SideInfo::default();
        si.granules[0][0] = GranuleSideInfo {
            scalefac_compress: 12, // (3,2)
            window_switching: true,
            block_type: BlockType::Short,
            ..Default::default()
        };
        let mut sf = ScaleFactors::default();
        for sfb in 0..12 {
            let cap = if sfb < 6 { 8 } else { 4 };
            for window in 0..3 {
                sf.short[window][sfb] = ((sfb * 3 + window) as u8) % cap;
            }
        }
        sf_round_trip(&si, 0, 0, &sf, None);
    }

    #[test]
    fn info_frame_is_well_formed() {
        let header = hdr(ChannelMode::Stereo);
        let frame = info_frame(&header, 1234, 567_890, false);
        // A valid MPEG frame of the right size, with the tag at the Xing offset.
        assert_eq!(frame.len(), header.frame_size());
        assert_eq!(frame[0], 0xFF);
        assert_eq!(frame[1] & 0xE0, 0xE0);
        let tag = 4 + header.side_info_len(); // no CRC
        assert_eq!(&frame[tag..tag + 4], b"Info");
        assert_eq!(
            u32::from_be_bytes(frame[tag + 4..tag + 8].try_into().unwrap()),
            3
        ); // flags
        assert_eq!(
            u32::from_be_bytes(frame[tag + 8..tag + 12].try_into().unwrap()),
            1234
        );
        assert_eq!(
            u32::from_be_bytes(frame[tag + 12..tag + 16].try_into().unwrap()),
            567_890
        );

        // Our own decoder accepts it as a (silent) frame.
        let mut dec = crate::Mp3Decoder::default();
        dec.push(&frame);
        dec.flush();
        assert!(dec.next_frame().is_ok());
    }

    #[test]
    fn frame_assembly_is_decodable() {
        use crate::frame::GranuleSideInfo;

        let header = hdr(ChannelMode::Mono);
        // A trivial-but-valid granule: no scalefactors, no big_values, no count1 —
        // decodes to silence. Both granules (MPEG-1) identical.
        let mut si = SideInfo::default();
        for gr in 0..2 {
            si.granules[gr][0] = GranuleSideInfo {
                part2_3_length: 0,
                ..Default::default()
            };
        }
        let main_data = [0u8; 10]; // a few bytes; the rest is stuffing

        let mut res = EncReservoir::default();
        let frame = format(&header, &si, &main_data, &mut res);

        // Structural checks.
        assert_eq!(frame.len(), header.frame_size());
        assert_eq!(&frame[0..4], &header.to_bytes());
        let si_bytes = &frame[4..4 + header.side_info_len()];
        assert_eq!(
            crate::decode::sideinfo::parse(&header, si_bytes).unwrap(),
            si
        );

        // The real decoder must accept two concatenated frames and yield two
        // audio frames of one frame's samples each.
        let mut stream = frame.clone();
        stream.extend_from_slice(&format(&header, &si, &main_data, &mut res));

        let mut dec = crate::Mp3Decoder::default();
        dec.push(&stream);
        dec.flush();
        let mut frames = 0;
        while let Ok(af) = dec.next_frame() {
            assert_eq!(
                af.samples.len() / af.channels as usize,
                header.version.samples_per_frame()
            );
            frames += 1;
        }
        assert_eq!(frames, 2, "both assembled frames must decode");
    }

    /// Data that outruns the cumulative capacity (which the rate loop never
    /// produces for admitted input) must degrade, not panic: every frame is still
    /// emitted at its fixed size and the stream still decodes frame for frame.
    #[test]
    fn reservoir_overflow_degrades_instead_of_panicking() {
        use crate::frame::GranuleSideInfo;
        let header = hdr(ChannelMode::Mono);
        let cap = region_capacity(&header);
        let mut si = SideInfo::default();
        for gr in 0..2 {
            si.granules[gr][0] = GranuleSideInfo::default();
        }
        // Three frames each carrying far more than any reservoir could bank.
        let frames: Vec<_> = (0..3u8)
            .map(|i| (header.clone(), si.clone(), vec![0x50 + i; 3 * cap + 600]))
            .collect();
        let stream = assemble_stream(&frames);
        assert_eq!(stream.len(), frames.len() * header.frame_size());
        let split = assemble_frames(&frames);
        assert_eq!(
            split.concat(),
            stream,
            "both assemblers agree on overflow too"
        );
        let mut dec = crate::Mp3Decoder::default();
        dec.push(&stream);
        dec.flush();
        let mut n = 0;
        while dec.next_frame().is_ok() {
            n += 1;
        }
        assert_eq!(n, 3);
    }

    #[test]
    fn reservoir_stream_round_trips_and_borrows() {
        use crate::decode::reservoir::Reservoir;
        use crate::frame::GranuleSideInfo;

        let header = hdr(ChannelMode::Mono);
        let cap = region_capacity(&header);
        // Frame 0 banks slack (tiny data); frame 1 borrows (data > its region);
        // frame 2 settles. part2_3_length=0 → the bytes are pure reservoir payload.
        let datas = [
            vec![0xAAu8; 40],        // small → banks ~cap-40
            vec![0xBBu8; cap + 120], // larger than one region → must borrow
            vec![0xCCu8; 60],
            vec![0xDDu8; 30],
        ];
        let si_silent = {
            let mut si = SideInfo::default();
            for gr in 0..2 {
                si.granules[gr][0] = GranuleSideInfo::default();
            }
            si
        };
        let frames: Vec<_> = datas
            .iter()
            .map(|d| (header.clone(), si_silent.clone(), d.clone()))
            .collect();

        let stream = assemble_stream(&frames);
        assert_eq!(stream.len(), frames.len() * header.frame_size());

        // Walk the assembled stream the way a demuxer would: read each frame's
        // physical region + its coded main_data_begin, run the *decoder's*
        // reservoir, and confirm each frame's data is recovered as the prefix of
        // the assembled main data.
        let fsz = header.frame_size();
        let si_len = header.side_info_len();
        let mut res = Reservoir::default();
        let mut saw_borrow = false;
        for (n, d) in datas.iter().enumerate() {
            let frame = &stream[n * fsz..(n + 1) * fsz];
            let si = crate::decode::sideinfo::parse(&header, &frame[4..4 + si_len]).unwrap();
            let begin = si.main_data_begin;
            if begin > 0 {
                saw_borrow = true;
            }
            let region = &frame[4 + si_len..];
            let assembled = res.assemble(begin, region);
            let want = d.len().min(assembled.len());
            assert_eq!(
                &assembled[..want],
                &d[..want],
                "frame {n} data not recovered"
            );
        }
        assert!(
            saw_borrow,
            "the scenario must exercise a non-zero back-reference"
        );

        // And the whole stream must decode frame-for-frame.
        let mut dec = crate::Mp3Decoder::default();
        dec.push(&stream);
        dec.flush();
        let mut frames_out = 0;
        while dec.next_frame().is_ok() {
            frames_out += 1;
        }
        assert_eq!(frames_out, datas.len(), "every frame must decode");
    }

    #[test]
    fn scalefactors_round_trip_scfsi_reuse() {
        // Granule 1 reuses granule 0's group-0 bands (0..6): they aren't coded.
        let mut si = SideInfo::default();
        si.scfsi[0][0] = true;
        si.granules[1][0] = GranuleSideInfo {
            scalefac_compress: 15,
            ..Default::default()
        };
        let mut prev = ScaleFactors::default();
        for b in 0..6 {
            prev.long[b] = (b as u8) + 1;
        }
        // The reused bands must come back from `prev`; the rest from the stream.
        let mut sf = prev.clone();
        for b in 6..21 {
            sf.long[b] = (b as u8) % (if b < 11 { 16 } else { 8 });
        }
        sf_round_trip(&si, 1, 0, &sf, Some(&prev));
    }
}

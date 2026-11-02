//! Huffman encoding of the quantized spectrum — the inverse of decode's spectrum
//! decode (`decode/huffman.rs`). Bricks **B1–B4** (+ **N8** linbits and **N9**
//! region maps, both reused from the decode side).
//!
//! The quantized lines are partitioned into the `big_values` region (coded as
//! pairs by up to three sub-regions, each picking one of the 32 ISO pair-tables)
//! and the `count1` region (quads of `{0,±1}`). Everything past the last non-zero
//! line is implicit `rzero`. We reuse the decoder's canonical codebooks
//! (`PAIR_TABLES`, `QUAD_A/B`) — encoding is the same `(code, len)` data indexed
//! by symbol instead of matched bit-by-bit — so a round-trip through
//! `decode::huffman::decode` reproduces the coefficients exactly. That is B's
//! verification gate.

use crate::bitio::BitWriter;
use crate::decode::codebooks::{PAIR_TABLES, QUAD_A, QUAD_B};
use crate::decode::huffman::{PairTable, QuadTable};
use crate::frame::{BlockType, GranuleSideInfo, GRANULE_LINES};
use crate::header::FrameHeader;
use crate::tables;

use super::quantize::QuantizedGranule;

// ── B1/B2: per-symbol cost + emit (pairs and quads) ───────────────────────────

/// Bits to code one coordinate's escape+sign under a pair table, or `None` if the
/// value is out of the table's range (no escape big enough).
fn coord_bits(t: &PairTable, v: i32) -> Option<usize> {
    let maxc = i32::from(t.dim) - 1;
    let a = v.abs();
    let sign = usize::from(v != 0);
    if t.linbits == 0 {
        if a > maxc {
            None
        } else {
            Some(sign)
        }
    } else if a >= maxc {
        if (a - maxc) as u32 >= (1u32 << t.linbits) {
            None
        } else {
            Some(t.linbits as usize + sign)
        }
    } else {
        Some(sign)
    }
}

/// Bit cost of coding pair `(x, y)` with pair table `t` (codeword + escapes +
/// signs), or `None` if either value is out of range.
fn pair_bits(t: &PairTable, x: i32, y: i32) -> Option<usize> {
    if t.dim == 0 {
        return if x == 0 && y == 0 { Some(0) } else { None };
    }
    let maxc = i32::from(t.dim) - 1;
    let cx = x.abs().min(maxc) as usize;
    let cy = y.abs().min(maxc) as usize;
    let idx = cx * t.dim as usize + cy;
    let (_, len) = t.book.code_len(idx)?;
    Some(len as usize + coord_bits(t, x)? + coord_bits(t, y)?)
}

/// Emit pair `(x, y)` with table `t`. Read order in the decoder is codeword, then
/// x-escape, x-sign, y-escape, y-sign — mirrored here.
fn pair_emit(t: &PairTable, x: i32, y: i32, w: &mut BitWriter) {
    if t.dim == 0 {
        return;
    }
    let maxc = i32::from(t.dim) - 1;
    let cx = x.abs().min(maxc) as usize;
    let cy = y.abs().min(maxc) as usize;
    let idx = cx * t.dim as usize + cy;
    let (code, len) = t.book.code_len(idx).expect("pair index in book");
    w.write(u32::from(code), u32::from(len));
    if t.linbits > 0 && x.abs() >= maxc {
        w.write((x.abs() - maxc) as u32, u32::from(t.linbits));
    }
    if x != 0 {
        w.write(u32::from(x < 0), 1);
    }
    if t.linbits > 0 && y.abs() >= maxc {
        w.write((y.abs() - maxc) as u32, u32::from(t.linbits));
    }
    if y != 0 {
        w.write(u32::from(y < 0), 1);
    }
}

fn quad_index(v: i32, w: i32, x: i32, y: i32) -> usize {
    (((v.abs() & 1) << 3) | ((w.abs() & 1) << 2) | ((x.abs() & 1) << 1) | (y.abs() & 1)) as usize
}

fn quad_bits(q: &QuadTable, v: i32, w: i32, x: i32, y: i32) -> usize {
    let (_, len) = q
        .book
        .code_len(quad_index(v, w, x, y))
        .expect("quad index 0..15");
    len as usize + [v, w, x, y].iter().filter(|&&c| c != 0).count()
}

fn quad_emit(q: &QuadTable, v: i32, w: i32, x: i32, y: i32, wr: &mut BitWriter) {
    let (code, len) = q
        .book
        .code_len(quad_index(v, w, x, y))
        .expect("quad index 0..15");
    wr.write(u32::from(code), u32::from(len));
    for &c in &[v, w, x, y] {
        if c != 0 {
            wr.write(u32::from(c < 0), 1);
        }
    }
}

/// **B2** — estimate the bit cost of coding `coeffs` (an even-length pair region)
/// with pair `table`, without emitting. Non-covering tables cost "infinity" so the
/// selector skips them.
pub fn estimate_bits(coeffs: &[i32], table: u8) -> usize {
    let t = PAIR_TABLES[(table as usize).min(PAIR_TABLES.len() - 1)];
    let mut total = 0usize;
    let mut i = 0;
    while i + 1 < coeffs.len() {
        total =
            total.saturating_add(pair_bits(t, coeffs[i], coeffs[i + 1]).unwrap_or(usize::MAX / 4));
        i += 2;
    }
    total
}

// ── N9/B4: region boundaries + table/partition selection ──────────────────────

/// **N9** — long-block region boundaries from `(region0_count, region1_count)`,
/// the exact inverse of the decoder's `region_bounds` (long branch).
fn region_bounds_long(sfb: &[u16; 23], r0c: u8, r1c: u8, bv2: usize) -> (usize, usize) {
    let i1 = (r0c as usize + 1).min(22);
    let i2 = (r0c as usize + r1c as usize + 2).min(22);
    let r1 = (sfb[i1] as usize).min(bv2);
    let r2 = (sfb[i2] as usize).min(bv2).max(r1);
    (r1, r2)
}

/// Region boundaries for either block type, mirroring the decoder: window-switched
/// (short/start/stop) blocks use a fixed `(36, bv2)` split; long blocks derive
/// them from the region counts.
fn region_bounds(gi: &GranuleSideInfo, sfb: &[u16; 23], bv2: usize) -> (usize, usize) {
    if gi.window_switching && gi.block_type != BlockType::Long {
        (36.min(bv2), bv2)
    } else {
        region_bounds_long(sfb, gi.region0_count, gi.region1_count, bv2)
    }
}

/// Largest magnitude a pair table can represent: `dim−1` plus the linbits escape
/// range. A table can't code a region whose peak exceeds this.
fn pair_table_max(t: &PairTable) -> i32 {
    if t.dim == 0 {
        return 0; // the empty table codes only (0, 0)
    }
    let maxc = i32::from(t.dim) - 1;
    if t.linbits == 0 {
        maxc
    } else {
        maxc + (1i32 << t.linbits) - 1
    }
}

/// Cheapest pair table covering `coeffs[lo..hi]` (an even-aligned region), and its
/// bit cost.
///
/// **A3** — only tables whose range covers the region's peak magnitude are costed;
/// the rest can't represent it (`estimate_bits` would score them "infinity"
/// anyway), so skipping them in O(1) avoids walking the region per dead table.
/// Output-identical to costing all 32.
///
/// **C (redundancy):** returns the winning cost alongside the table, so callers
/// that need the region's bit count don't have to re-walk it (the cost was already
/// computed to choose the table).
fn best_pair_table(coeffs: &[i32; GRANULE_LINES], lo: usize, hi: usize) -> (u8, usize) {
    if lo >= hi {
        return (0, 0);
    }
    let slice = &coeffs[lo..hi];

    // ── One region walk → a compact histogram (C: kill the per-table re-walk) ──
    // `pair_bits` decomposes exactly as: codeword(clamp(x),clamp(y)) + sign(x≠0) +
    // sign(y≠0) + linbits·[|x|≥maxc] + linbits·[|y|≥maxc]. So per table the cost is
    // a sum over the (few) distinct clamped pairs + a sign count (table-independent)
    // + an escape count derived from a coordinate-magnitude histogram. Magnitudes
    // clamp at 15 because no table's `maxc` exceeds 15 (`min(|v|,15)≥maxc ⇔ |v|≥maxc`).
    // Peak is taken over the whole slice (including any trailing unpaired element),
    // matching the reference: that element is uncoded, yet it still gates the
    // coverage prune — replicated here so the selection stays byte-identical.
    let peak = slice.iter().map(|c| c.unsigned_abs()).max().unwrap_or(0) as i32;
    // FLAT histogram, indexed `ax * 16 + ay`, and cells holding that flat index.
    // As a `[[u32; 16]; 16]` indexed by `a as usize` from a `u8`, the scoring loop
    // below could not prove `a <= 15` -- the value is 0..=255 to the compiler -- so
    // every cell it scored, for every candidate table, paid two bounds checks. A
    // `u8` index into 256 entries is in range by construction.
    let mut hist = [0u32; 256]; // counts of clamped (|x|,|y|) pairs
    let mut cells: [u8; 256] = [0; 256]; // the populated pair cells, flat index
    let mut ncells = 0usize;
    let mut cmag = [0u32; 16]; // counts of clamped coordinate magnitudes
    let mut i = 0;
    while i + 1 < slice.len() {
        let (x, y) = (slice[i], slice[i + 1]);
        let ax = x.unsigned_abs().min(15) as usize;
        let ay = y.unsigned_abs().min(15) as usize;
        let cell = ax * 16 + ay;
        if hist[cell] == 0 {
            cells[ncells & 255] = cell as u8;
            ncells += 1;
        }
        hist[cell] += 1;
        cmag[ax] += 1;
        cmag[ay] += 1;
        i += 2;
    }
    // sign bits: one per nonzero coordinate (the same for every table).
    let total_signs = cmag[1..].iter().sum::<u32>() as usize;
    // cum[m] = #coords with magnitude ≥ m (for the linbits escape count).
    let mut cum = [0u32; 17];
    for m in (0..16).rev() {
        cum[m] = cum[m + 1] + cmag[m];
    }

    let mut best = (usize::MAX, 0u8);
    for table in 0u8..PAIR_TABLES.len() as u8 {
        let t = PAIR_TABLES[table as usize];
        if pair_table_max(t) < peak {
            continue; // can't code this region's peak — would cost "infinity"
        }
        if t.dim == 0 {
            // The empty book codes only all-zero regions (peak == 0), at zero cost.
            if best.0 > 0 {
                best = (0, table);
            }
            continue;
        }
        let dim = t.dim as usize;
        let maxc = dim - 1;
        let mut codeword = 0usize;
        for &cell in &cells[..ncells] {
            let (a, b) = ((cell >> 4) as usize, (cell & 15) as usize);
            let (cx, cy) = (a.min(maxc), b.min(maxc));
            let len = t
                .book
                .code_len(cx * dim + cy)
                .map_or(usize::MAX / 4, |(_, l)| l as usize);
            codeword += hist[cell as usize] as usize * len;
        }
        let escape = if t.linbits > 0 {
            t.linbits as usize * cum[maxc] as usize
        } else {
            0
        };
        let cost = codeword.saturating_add(escape).saturating_add(total_signs);
        if cost < best.0 {
            best = (cost, table);
        }
    }
    (best.1, best.0)
}

/// Cheaper of the two count1 quad tables for `coeffs[lo..hi]` (`false`=A, `true`=B),
/// and that table's bit cost.
fn best_quad_table(coeffs: &[i32; GRANULE_LINES], lo: usize, hi: usize) -> (bool, usize) {
    let (mut a, mut b) = (0usize, 0usize);
    let mut i = lo;
    while i + 4 <= hi {
        a += quad_bits(
            &QUAD_A,
            coeffs[i],
            coeffs[i + 1],
            coeffs[i + 2],
            coeffs[i + 3],
        );
        b += quad_bits(
            &QUAD_B,
            coeffs[i],
            coeffs[i + 1],
            coeffs[i + 2],
            coeffs[i + 3],
        );
        i += 4;
    }
    if b < a {
        (true, b)
    } else {
        (false, a)
    }
}

/// Roughly split the big-values region into thirds at scalefactor-band boundaries.
fn choose_regions(sfb: &[u16; 23], big_end: usize) -> (u8, u8) {
    if big_end == 0 {
        return (0, 0);
    }
    let third = big_end / 3;
    let two_third = 2 * big_end / 3;
    let i1 = (1..=22).rfind(|&i| sfb[i] as usize <= third).unwrap_or(1);
    let i2 = (i1..=22)
        .rfind(|&i| sfb[i] as usize <= two_third)
        .unwrap_or(i1);
    ((i1 - 1).min(15) as u8, (i2 - i1).min(7) as u8)
}

/// **B4 / Q5** — partition a quantized spectrum and pick its codebooks, for any
/// `block_type`. Long blocks split into three region-count-derived regions; window-
/// switched blocks (short/start/stop) use the decoder's fixed `(36, bv2)` split.
/// Fills the Huffman side-info so [`encode`] and the decoder agree on the layout.
#[must_use]
pub fn select(
    header: &FrameHeader,
    coeffs: &[i32; GRANULE_LINES],
    block_type: BlockType,
) -> (GranuleSideInfo, usize) {
    // big_values must cover every line with magnitude > 1 (count1 only codes ±1).
    let big_end = match coeffs.iter().rposition(|&c| c.abs() > 1) {
        Some(i) => (i / 2 + 1) * 2, // even, includes the pair holding line i
        None => 0,
    };
    let last_nz = coeffs.iter().rposition(|&c| c != 0).map_or(0, |i| i + 1);
    let region_end = last_nz.max(big_end);
    let count1_quads = (region_end - big_end).div_ceil(4);
    let count1_end = (big_end + count1_quads * 4).min(GRANULE_LINES);
    debug_assert!(
        coeffs[count1_end..].iter().all(|&c| c == 0),
        "rzero invariant: no non-zero line past the coded region"
    );

    let (count1_select, count1_bits) = best_quad_table(coeffs, big_end, count1_end);
    let common = GranuleSideInfo {
        big_values: (big_end / 2) as u16,
        count1table_select: count1_select,
        ..Default::default()
    };

    let (side, big_bits) = if block_type == BlockType::Long {
        let sfb = tables::sfb_long_offsets(header.sample_rate);
        let (r0c, r1c) = choose_regions(sfb, big_end);
        let (r1, r2) = region_bounds_long(sfb, r0c, r1c, big_end);
        let (t0, c0) = best_pair_table(coeffs, 0, r1);
        let (t1, c1) = best_pair_table(coeffs, r1, r2);
        let (t2, c2) = best_pair_table(coeffs, r2, big_end);
        (
            GranuleSideInfo {
                region0_count: r0c,
                region1_count: r1c,
                table_select: [t0, t1, t2],
                ..common
            },
            c0 + c1 + c2,
        )
    } else {
        let (tables, big_bits) = windowed_table_select(coeffs, big_end);
        (
            GranuleSideInfo {
                window_switching: true,
                block_type,
                table_select: tables,
                ..common
            },
            big_bits,
        )
    };
    // Total part-3 bits = big_values + count1 — already summed here, so callers
    // never re-walk the spectrum (the `cost`-equals-this invariant is tested).
    (side, big_bits + count1_bits)
}

/// Pair-table selection for the window-switched fixed `(36, bv2)` region split
/// (used by short, start, and stop blocks), with the two regions' total bits.
/// Region 2 is empty.
#[must_use]
pub fn windowed_table_select(coeffs: &[i32; GRANULE_LINES], bv2: usize) -> ([u8; 3], usize) {
    let r1 = 36.min(bv2);
    let (t0, c0) = best_pair_table(coeffs, 0, r1);
    let (t1, c1) = best_pair_table(coeffs, r1, bv2);
    ([t0, t1, 0], c0 + c1)
}

/// Huffman bit cost of a short-block coefficient set (counted, not emitted).
#[must_use]
pub fn cost_short(header: &FrameHeader, coeffs: &[i32; GRANULE_LINES]) -> usize {
    select(header, coeffs, BlockType::Short).1
}

// ── B3: emit the whole spectrum ───────────────────────────────────────────────

/// **B3** — encode one quantized granule's spectrum into `writer`, per the layout
/// in `quant.side`. Returns the number of Huffman bits written (the part3 length);
/// the caller adds the scalefactor (part2) bits to get `part2_3_length`.
pub fn encode(quant: &QuantizedGranule, header: &FrameHeader, writer: &mut BitWriter) -> usize {
    let start = writer.bit_len();
    let gi = &quant.side;
    let coeffs = &quant.coeffs;
    let sfb = tables::sfb_long_offsets(header.sample_rate);
    let bv2 = (gi.big_values as usize * 2).min(GRANULE_LINES);
    let (r1, r2) = region_bounds(gi, sfb, bv2);

    // big_values: pairs, table chosen per region.
    let mut i = 0;
    while i + 1 < bv2 {
        let t = if i < r1 {
            gi.table_select[0]
        } else if i < r2 {
            gi.table_select[1]
        } else {
            gi.table_select[2]
        } as usize;
        pair_emit(
            PAIR_TABLES[t.min(PAIR_TABLES.len() - 1)],
            coeffs[i],
            coeffs[i + 1],
            writer,
        );
        i += 2;
    }

    // count1: quads up to rzero (the decoder stops at the part2_3 budget, which the
    // caller sets from the returned length, so it reads exactly these).
    let quad = if gi.count1table_select {
        &QUAD_B
    } else {
        &QUAD_A
    };
    let rzero = coeffs
        .iter()
        .rposition(|&c| c != 0)
        .map_or(0, |x| x + 1)
        .max(bv2);
    while i + 4 <= GRANULE_LINES && i < rzero {
        quad_emit(
            quad,
            coeffs[i],
            coeffs[i + 1],
            coeffs[i + 2],
            coeffs[i + 3],
            writer,
        );
        i += 4;
    }

    writer.bit_len() - start
}

/// **A2** — bit cost of encoding `coeffs` under layout `gi`, counted without
/// emitting. Walks the exact regions [`encode`] does and sums `pair_bits` /
/// `quad_bits` (which equal the bits those coordinates emit), so the rate loop
/// can probe a gain without allocating a `BitWriter` or writing any codewords.
/// Returns the same value [`encode`] would return — pinned by a test.
pub fn cost(gi: &GranuleSideInfo, coeffs: &[i32; GRANULE_LINES], header: &FrameHeader) -> usize {
    let sfb = tables::sfb_long_offsets(header.sample_rate);
    let bv2 = (gi.big_values as usize * 2).min(GRANULE_LINES);
    let (r1, r2) = region_bounds(gi, sfb, bv2);
    let mut bits = 0usize;

    let mut i = 0;
    while i + 1 < bv2 {
        let t = if i < r1 {
            gi.table_select[0]
        } else if i < r2 {
            gi.table_select[1]
        } else {
            gi.table_select[2]
        } as usize;
        let pt = PAIR_TABLES[t.min(PAIR_TABLES.len() - 1)];
        bits =
            bits.saturating_add(pair_bits(pt, coeffs[i], coeffs[i + 1]).unwrap_or(usize::MAX / 4));
        i += 2;
    }

    let quad = if gi.count1table_select {
        &QUAD_B
    } else {
        &QUAD_A
    };
    let rzero = coeffs
        .iter()
        .rposition(|&c| c != 0)
        .map_or(0, |x| x + 1)
        .max(bv2);
    while i + 4 <= GRANULE_LINES && i < rzero {
        bits += quad_bits(quad, coeffs[i], coeffs[i + 1], coeffs[i + 2], coeffs[i + 3]);
        i += 4;
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::ChannelMode;
    use crate::header::MpegVersion;

    fn hdr() -> FrameHeader {
        FrameHeader {
            version: MpegVersion::V1,
            crc_protected: false,
            bitrate_kbps: 128,
            sample_rate: 44100,
            padding: false,
            channel_mode: ChannelMode::Mono,
            copyright: false,
            original: true,
            emphasis: 0,
        }
    }

    /// Select tables, encode, decode back — coefficients must survive exactly.
    fn round_trip(coeffs: &[i32; GRANULE_LINES]) {
        let header = hdr();
        let (side, _) = select(&header, coeffs, BlockType::Long);
        let quant = QuantizedGranule {
            coeffs: *coeffs,
            side: side.clone(),
            scalefactors: [0; 39],
        };
        let mut w = BitWriter::new();
        let hlen = encode(&quant, &header, &mut w);
        let bits = w.finish();

        let mut pos = 0;
        let (out, _nz) = crate::decode::huffman::decode(&bits, &mut pos, hlen, &header, &side);
        assert_eq!(pos, hlen, "decoder must consume exactly the emitted bits");
        assert_eq!(out, *coeffs, "spectrum must round-trip exactly");
    }

    #[test]
    fn round_trip_count1_only() {
        // Small values + trailing zeros: count1-dominated.
        let mut c = [0i32; GRANULE_LINES];
        for (i, v) in c.iter_mut().take(40).enumerate() {
            *v = [(-1), 0, 1, 1, 0, -1, 0, 1][i % 8];
        }
        round_trip(&c);
    }

    #[test]
    fn round_trip_big_values_and_signs() {
        let mut c = [0i32; GRANULE_LINES];
        // Low-frequency big values (need pair tables), then small, then zeros.
        let seed = [12, -7, 3, -2, 5, 9, -4, 2, 1, -1, 1, 0, 2, -3];
        c[..seed.len()].copy_from_slice(&seed);
        for (i, v) in c.iter_mut().take(80).skip(seed.len()).enumerate() {
            *v = [0, 1, -1, 0, 1, 0, -1, 0][i % 8];
        }
        round_trip(&c);
    }

    #[test]
    fn round_trip_linbits_escape() {
        // Large magnitudes force the ESC (linbits) tables.
        let mut c = [0i32; GRANULE_LINES];
        c[0] = 600;
        c[1] = -432;
        c[2] = 87;
        c[3] = -15;
        c[4] = 1;
        c[5] = -1;
        round_trip(&c);
    }

    #[test]
    fn round_trip_all_zero() {
        round_trip(&[0i32; GRANULE_LINES]);
    }

    #[test]
    fn round_trip_dense_spectrum() {
        // A broad, deterministic spectrum with rzero in the top lines.
        let mut c = [0i32; GRANULE_LINES];
        let mut s = 0x9E3779B9u32;
        for v in c.iter_mut().take(500) {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let r = (s >> 24) as i32; // 0..255
            *v = (r % 11) - 5; // -5..5
        }
        round_trip(&c);
    }

    /// A2 + C invariant: `select`'s returned bit cost and the standalone `cost`
    /// must both equal what `encode` actually writes, for every layout the rate
    /// loop might probe — so the redundancy-eliminated path stays bit-exact.
    fn assert_cost_matches(coeffs: &[i32; GRANULE_LINES]) {
        let header = hdr();
        for bt in [BlockType::Long, BlockType::Short] {
            let (side, sel_cost) = select(&header, coeffs, bt);
            let q = QuantizedGranule {
                coeffs: *coeffs,
                side: side.clone(),
                scalefactors: [0; 39],
            };
            let mut w = BitWriter::new();
            let emitted = encode(&q, &header, &mut w);
            assert_eq!(
                cost(&side, coeffs, &header),
                emitted,
                "cost vs encode ({bt:?})"
            );
            assert_eq!(sel_cost, emitted, "select cost vs encode ({bt:?})");
        }
    }

    #[test]
    fn cost_matches_encoded_bits() {
        let mut dense = [0i32; GRANULE_LINES];
        let mut s = 0x1234_5678u32;
        for v in dense.iter_mut().take(530) {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *v = ((s >> 23) as i32 % 21) - 10; // -10..10, exercises pairs+escapes+count1
        }
        assert_cost_matches(&dense);

        let mut esc = [0i32; GRANULE_LINES];
        esc[0] = 700;
        esc[1] = -512;
        esc[2] = 40;
        esc[3] = -1;
        esc[4] = 1;
        assert_cost_matches(&esc);

        assert_cost_matches(&[0i32; GRANULE_LINES]);
    }

    /// Reference: the pre-C per-table `estimate_bits` search the histogram replaces.
    fn best_pair_table_ref(coeffs: &[i32; GRANULE_LINES], lo: usize, hi: usize) -> (u8, usize) {
        if lo >= hi {
            return (0, 0);
        }
        let slice = &coeffs[lo..hi];
        let peak = slice.iter().map(|c| c.unsigned_abs()).max().unwrap_or(0) as i32;
        let mut best = (usize::MAX, 0u8);
        for table in 0u8..PAIR_TABLES.len() as u8 {
            if pair_table_max(PAIR_TABLES[table as usize]) < peak {
                continue;
            }
            let cost = estimate_bits(slice, table);
            if cost < best.0 {
                best = (cost, table);
            }
        }
        (best.1, best.0)
    }

    /// **C gate.** The histogram table search must return the identical
    /// `(table, cost)` as the per-table reference — across small values (the common
    /// case), escape-range values, all-zero regions, and odd region lengths.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn best_pair_table_hist_matches_ref() {
        let mut st = 0x517C_C1EFu32;
        let mut rng = || {
            st = st.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            st >> 8
        };
        for trial in 0..4000 {
            let mut c = [0i32; GRANULE_LINES];
            // Vary the value range per trial: mostly small, sometimes large (escape).
            let span = [2i32, 4, 8, 16, 64, 600][trial % 6];
            let fill = (rng() as usize % 400) + 1;
            for v in c.iter_mut().take(fill) {
                let m = (rng() as i32) % (span + 1);
                *v = if rng() & 1 == 0 { m } else { -m };
            }
            // Random even-aligned region, plus an occasional odd hi.
            let lo = (rng() as usize % 100) & !1;
            let hi = (lo + 2 + (rng() as usize % 300)).min(GRANULE_LINES);
            assert_eq!(
                best_pair_table(&c, lo, hi),
                best_pair_table_ref(&c, lo, hi),
                "trial {trial}: span {span}, region [{lo},{hi})"
            );
        }
    }

    #[test]
    fn estimate_matches_emitted_bits() {
        // B2's estimate must equal what B3 actually writes for a pure pair region.
        let mut c = [0i32; GRANULE_LINES];
        let seed = [4, -3, 2, -1, 5, 6, -2, 1];
        c[..seed.len()].copy_from_slice(&seed);
        let (table, _) = best_pair_table(&c, 0, seed.len());
        let est = estimate_bits(&c[0..seed.len()], table);

        let t = PAIR_TABLES[table as usize];
        let mut w = BitWriter::new();
        let mut i = 0;
        while i + 1 < seed.len() {
            pair_emit(t, c[i], c[i + 1], &mut w);
            i += 2;
        }
        assert_eq!(est, w.bit_len(), "estimate must match emitted bits");
    }
}

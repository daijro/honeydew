//! Huffman decoding of the quantized spectrum.
//!
//! The `big_values` region is `2 * big_values` lines decoded as **pairs** (x, y)
//! by up to three sub-regions, each using one of the 32 ISO pair-tables (square,
//! side `dim`, with a `linbits` escape on the max value). After it, the `count1`
//! region decodes **quads** (v, w, x, y ∈ {0, ±1}) with one of two tables. The
//! rest of the 576 lines are implicit zeros.
//!
//! Following the AAC decoder's approach, the *logic* here is proven with
//! synthetic tables; the real ISO codebooks are transcribed into [`tables`] and
//! gated by Kraft/prefix-free validation. A table is two parallel arrays —
//! `codes[i]`/`lens[i]` in (x·dim + y) raster order — matched MSB-first.

// Parses untrusted bytes: narrowing casts are lint-enforced here (see the crate
// lint policy in Cargo.toml) -- every one is masked, typed, or states its bound.
#![warn(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use std::sync::OnceLock;

use crate::bitio::BitReader;
use crate::frame::{BlockType, GranuleSideInfo, GRANULE_LINES};
use crate::header::FrameHeader;
use crate::tables;

use super::codebooks::{PAIR_TABLES, QUAD_A, QUAD_B};

/// Root LUT width cap. Codewords up to this length resolve in one table lookup;
/// the (rare, large-coefficient) longer codes fall back to a linear scan. Bounds
/// each book's table at `2^12 = 4096` entries.
const LUT_BITS_CAP: u8 = 12;

/// `DIV_TAB[d][i] == i / d` for `d <= 16`, `i < 256` — the pair tables' whole
/// index range. Built at compile time so the hot path never divides.
#[allow(clippy::cast_possible_truncation)] // `i / d` with i < 256, d >= 1: fits u8
const DIV_TAB: [[u8; 256]; 17] = {
    let mut t = [[0u8; 256]; 17];
    let mut d = 1;
    while d < 17 {
        let mut i = 0;
        while i < 256 {
            t[d][i] = (i / d) as u8;
            i += 1;
        }
        d += 1;
    }
    t
};

/// One decode-LUT slot: `(symbol_index, codeword_length)`. `len == 0` marks an
/// "escape" — the peeked prefix belongs to a codeword longer than `lut_bits`
/// (or no codeword), to be resolved by the linear fallback.
#[derive(Clone, Copy)]
struct LutSlot {
    sym: u16,
    len: u8,
}

/// A prefix-free codeword book: parallel codeword / bit-length arrays, plus a
/// lazily-built lookup table for O(1) decode.
pub struct HuffBook {
    codes: &'static [u16],
    lens: &'static [u8],
    max_len: u8,
    /// Peek width for the LUT = `min(max_len, LUT_BITS_CAP)`.
    lut_bits: u8,
    lut: OnceLock<Vec<LutSlot>>,
}

impl HuffBook {
    #[must_use]
    pub const fn new(codes: &'static [u16], lens: &'static [u8]) -> Self {
        let mut max = 0u8;
        let mut i = 0;
        while i < lens.len() {
            if lens[i] > max {
                max = lens[i];
            }
            i += 1;
        }
        let lut_bits = if max < LUT_BITS_CAP {
            max
        } else {
            LUT_BITS_CAP
        };
        Self {
            codes,
            lens,
            max_len: max,
            lut_bits,
            lut: OnceLock::new(),
        }
    }

    /// Build (once) the peek table: every `lut_bits`-bit prefix maps to its
    /// codeword's `(symbol, length)`, or stays an escape if it heads a longer
    /// codeword. Prefix-freeness guarantees no short code overwrites a long one's
    /// prefix region, so escapes are exactly the long-code / invalid prefixes.
    // `i` indexes a codebook (at most 256 codes), so `i as u16` is exact.
    #[allow(clippy::cast_possible_truncation)]
    fn lut(&self) -> &[LutSlot] {
        self.lut.get_or_init(|| {
            let bits = u32::from(self.lut_bits);
            let mut t = vec![LutSlot { sym: 0, len: 0 }; 1usize << bits];
            for (i, (&code, &len)) in self.codes.iter().zip(self.lens.iter()).enumerate() {
                if len == 0 || u32::from(len) > bits {
                    continue; // empty slot / resolved by the linear fallback
                }
                let shift = bits - u32::from(len);
                let start = (code as usize) << shift;
                for slot in &mut t[start..start + (1usize << shift)] {
                    *slot = LutSlot { sym: i as u16, len };
                }
            }
            t
        })
    }

    /// Decode the next codeword MSB-first, returning its symbol index, or `None`
    /// if the bits match no codeword (corrupt stream). One table lookup for the
    /// common short codes; a linear scan only for codewords past `lut_bits`.
    pub fn decode_index(&self, r: &mut BitReader) -> Option<usize> {
        if self.codes.is_empty() {
            return Some(0); // the empty book (table 0) codes a constant 0 pair.
        }
        let slot = self.lut()[r.peek(u32::from(self.lut_bits)) as usize];
        if slot.len != 0 {
            r.skip(u32::from(slot.len));
            return Some(slot.sym as usize);
        }
        self.decode_linear(r)
    }

    /// Bit-by-bit fallback: the reference decoder, used only when the peeked
    /// prefix escapes the LUT (a codeword longer than `lut_bits`).
    fn decode_linear(&self, r: &mut BitReader) -> Option<usize> {
        let mut code = 0u32;
        for len in 1..=self.max_len {
            code = (code << 1) | r.read(1);
            // NOT zipped, deliberately: zipping `lens` with `codes` here removes a
            // bounds check in this COLD fallback and costs `decode_index` -- the hot
            // caller it is inlined into -- six instructions. The cold path is not
            // worth paying for in the stage that is ~42% of decode.
            for i in 0..self.codes.len() {
                if self.lens[i] == len && u32::from(self.codes[i]) == code {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Encode side: the `(codeword, bit-length)` for symbol index `idx` (the
    /// inverse of [`decode_index`](Self::decode_index)). The empty book (table 0) codes index 0 as a
    /// zero-length word. Returns `None` if `idx` is outside the book.
    pub fn code_len(&self, idx: usize) -> Option<(u16, u8)> {
        if self.codes.is_empty() {
            return if idx == 0 { Some((0, 0)) } else { None };
        }
        // Fetch BOTH through `get`. Indexing `lens[idx]` after a successful
        // `codes.get(idx)` still carries its own bounds check -- the compiler
        // cannot know the two vectors are the same length -- and this runs per
        // cell per candidate table in the encoder's table search.
        match (self.codes.get(idx), self.lens.get(idx)) {
            (Some(&c), Some(&l)) => Some((c, l)),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn kraft_sum(&self) -> f64 {
        self.lens.iter().map(|&l| 2f64.powi(-i32::from(l))).sum()
    }

    #[cfg(test)]
    pub fn is_prefix_free(&self) -> bool {
        for a in 0..self.codes.len() {
            for b in (a + 1)..self.codes.len() {
                let (la, lb) = (self.lens[a], self.lens[b]);
                let (short, long, ls, ll) = if la <= lb {
                    (self.codes[a], self.codes[b], la, lb)
                } else {
                    (self.codes[b], self.codes[a], lb, la)
                };
                if (u32::from(long) >> (ll - ls)) == u32::from(short) {
                    return false; // `short` is a prefix of `long`
                }
            }
        }
        true
    }
}

/// A `big_values` pair table: a book plus its square dimension and linbits.
pub struct PairTable {
    pub book: HuffBook,
    pub dim: u8,
    pub linbits: u8,
}

impl PairTable {
    /// Decode one (x, y) pair: Huffman index → coordinates, linbits escape on the
    /// max coordinate, then a sign bit for each non-zero coordinate.
    // `idx - x * d` is the remainder `y < d <= 16`, and a linbits escape reads at
    // most 13 bits (the static codebooks' maximum; the stream only selects among
    // them), so every narrowing below is exact.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap
    )]
    fn read(&self, r: &mut BitReader) -> (i32, i32) {
        if self.dim == 0 {
            return (0, 0);
        }
        let Some(idx) = self.book.decode_index(r) else {
            return (0, 0);
        };
        // **D7** — `idx / dim` and `idx % dim` used to run PER CODEWORD. Integer
        // division is 20-40 cycles on x86 and a few minutes of audio carries tens
        // of millions of pair codewords, on a stage that is now ~40% of decode.
        // `dim <= 16` and `idx < dim*dim <= 256`, so the quotient is a small
        // compile-time table; the remainder is then one multiply-subtract.
        // A const table, not a OnceLock+Vec: that would trade the division for an
        // atomic load and a pointer chase, which is not obviously cheaper.
        let d = self.dim as usize;
        // The inner index is already masked; the OUTER one was not. `d` is
        // `self.dim as usize` -- a `u8`, so 0..=255 to the compiler -- against a
        // 17-row table, which is a bounds check on every decoded pair in the stage
        // that is ~42% of decode. A mask will not do (16 is a legal dim, so `& 15`
        // would fold it onto 0); `.min(16)` is a cmov and exact for every real dim.
        let mut x = i32::from(DIV_TAB[d.min(16)][idx & 0xFF]);
        let mut y = (idx - x as usize * d) as i32;
        let maxc = i32::from(self.dim) - 1;
        if self.linbits > 0 && x == maxc {
            x += r.read(u32::from(self.linbits)) as i32;
        }
        if x != 0 && r.read(1) == 1 {
            x = -x;
        }
        if self.linbits > 0 && y == maxc {
            y += r.read(u32::from(self.linbits)) as i32;
        }
        if y != 0 && r.read(1) == 1 {
            y = -y;
        }
        (x, y)
    }
}

/// A `count1` quad table: a 16-entry book whose index is the 4 value bits.
pub struct QuadTable {
    pub book: HuffBook,
}

impl QuadTable {
    /// Decode one (v, w, x, y) quad: index bits give magnitudes (0/1), each
    /// non-zero followed by a sign bit.
    fn read(&self, r: &mut BitReader) -> (i32, i32, i32, i32) {
        let idx = self.book.decode_index(r).unwrap_or(0);
        let mut q = [
            i32::from(idx & 8 != 0),
            i32::from(idx & 4 != 0),
            i32::from(idx & 2 != 0),
            i32::from(idx & 1 != 0),
        ];
        for v in &mut q {
            if *v != 0 && r.read(1) == 1 {
                *v = -*v;
            }
        }
        q.into()
    }
}

/// Big-value region boundaries (line indices) for one granule/channel.
fn region_bounds(
    gi: &GranuleSideInfo,
    sfb_long: &[u16; 23],
    bv2: usize,
    sample_rate: u32,
) -> (usize, usize) {
    if gi.window_switching && gi.block_type != BlockType::Long {
        // Window-switching blocks have an IMPLIED region0 (region counts aren't coded),
        // and it differs by block type (matched to minimp3 + FFmpeg, ALL bit-exact):
        //   • PURE short: region0 = 36 lines (fixed, all rates).
        //   • START/STOP/mixed (long windows): region0 = the 8th long-band offset
        //     `sfb_long[8]`. At 44.1 kHz that IS 36 — which is why a single hardcoded 36
        //     was bit-exact for MPEG-1 but WRONG for LSF, where `sfb_long[8] ≠ 36`. This
        //     was the whole LSF short-block bug: Start/Stop got 36 instead of sfb_long[8].
        // region1 spans the rest; region2 is empty.
        let r0 = if gi.block_type == BlockType::Short && !gi.mixed_block {
            // pure short: region0 = 3 · the 4th short-band offset. That's 36 at every
            // rate where the low short bands are 4 wide (sfb_short[3]=12), but 72 at
            // 8 kHz where they're 8 wide (sfb_short[3]=24) — another 44.1k-masked const.
            3 * tables::sfb_short_offsets(sample_rate)[3] as usize
        } else {
            sfb_long[8] as usize // START/STOP/mixed: first 8 long bands (= 36 at 44.1 kHz)
        };
        (r0.min(bv2), bv2)
    } else {
        let i1 = (gi.region0_count as usize + 1).min(22);
        let i2 = (gi.region0_count as usize + gi.region1_count as usize + 2).min(22);
        let r1 = (sfb_long[i1] as usize).min(bv2);
        let r2 = (sfb_long[i2] as usize).min(bv2).max(r1);
        (r1, r2)
    }
}

/// Decode one granule/channel's quantized coefficients from `main` starting at
/// `*bit_pos`, stopping at `part2_3_end`. Returns the 576 integer coefficients
/// and the count of non-zero (decoded) lines — the rzero boundary.
pub fn decode(
    main: &[u8],
    bit_pos: &mut usize,
    part2_3_end: usize,
    header: &FrameHeader,
    gi: &GranuleSideInfo,
) -> ([i32; GRANULE_LINES], usize) {
    let mut r = BitReader::new(main);
    r.seek_bits(*bit_pos);
    let mut out = [0i32; GRANULE_LINES];

    let sfb_long = tables::sfb_long_offsets(header.sample_rate);
    let bv2 = (gi.big_values as usize * 2).min(GRANULE_LINES);
    let (r1, r2) = region_bounds(gi, sfb_long, bv2, header.sample_rate);

    // big_values: pairs, choosing the table by region.
    let mut i = 0;
    while i + 1 < bv2 && r.bit_pos() < part2_3_end {
        let t = if i < r1 {
            gi.table_select[0]
        } else if i < r2 {
            gi.table_select[1]
        } else {
            gi.table_select[2]
        } as usize;
        let (x, y) = PAIR_TABLES[t.min(PAIR_TABLES.len() - 1)].read(&mut r);
        out[i] = x;
        out[i + 1] = y;
        i += 2;
    }

    // count1: quads until the part2_3 budget is spent.
    let quad = if gi.count1table_select {
        &QUAD_B
    } else {
        &QUAD_A
    };
    let count1_start = i;
    while i + 3 < GRANULE_LINES && r.bit_pos() < part2_3_end {
        let (v, w, x, y) = quad.read(&mut r);
        out[i] = v;
        out[i + 1] = w;
        out[i + 2] = x;
        out[i + 3] = y;
        i += 4;
    }
    // A quad whose codeword ENDS past part2_3_end was not coded by the encoder:
    // some encoders (shine, measured) size part2_3_length so that the last quad
    // straddles the budget, and the bits it reads belong to the next granule.
    // FFmpeg and minimp3 both drop it ("overread" in mpegaudiodec); keeping it put
    // up to four spurious +-1 lines at the top of the count1 region and disagreed
    // with FFmpeg by up to 861 LSB on 119 of 162 shine streams.
    //
    // Only when the loop stopped on the BIT budget: a quad that fills the spectrum
    // to line 576 is kept even if it overran, because FFmpeg's loop exits on the
    // line limit before it ever re-tests the position (`while s_index <= 572`).
    // Dropping that one too left 10 shine streams still disagreeing.
    if i > count1_start && i + 3 < GRANULE_LINES && r.bit_pos() > part2_3_end {
        i -= 4;
        out[i..i + 4].fill(0);
    }

    let nonzero = i.min(GRANULE_LINES);
    // The granule's main-data ends at part2_3_end regardless of overrun/stuffing.
    *bit_pos = part2_3_end;
    (out, nonzero)
}

// The 32 pair-tables + 2 count1 quad-tables are the canonical ISO codebooks,
// generated into `codebooks.rs` and imported above.

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a main-data buffer from a list of (value, bit-length) tokens.
    fn pack(tokens: &[(u32, u32)]) -> Vec<u8> {
        let mut w = crate::bitio::BitWriter::new();
        for &(v, n) in tokens {
            w.write(v, n);
        }
        w.finish()
    }

    #[test]
    fn all_codebooks_complete_and_prefix_free() {
        // Every canonical ISO codebook must be a complete (Kraft = 1) prefix code.
        for (n, t) in PAIR_TABLES.iter().enumerate() {
            if t.dim == 0 {
                continue; // tables 0, 4, 14 are unused/empty
            }
            let k = t.book.kraft_sum();
            assert!((k - 1.0).abs() < 1e-9, "pair table {n}: Kraft = {k}");
            assert!(t.book.is_prefix_free(), "pair table {n} not prefix-free");
        }
        for (name, q) in [("A", &QUAD_A), ("B", &QUAD_B)] {
            let k = q.book.kraft_sum();
            assert!((k - 1.0).abs() < 1e-9, "count1 table {name}: Kraft = {k}");
            assert!(
                q.book.is_prefix_free(),
                "count1 table {name} not prefix-free"
            );
        }
    }

    #[test]
    fn pair_decode_with_signs() {
        // Real table 1: codeword "000" → (1,1); each non-zero gets a sign bit.
        // sign x = 1 (negative), sign y = 0 (positive) → (-1, 1).
        let bits = pack(&[(0b000, 3), (1, 1), (0, 1)]);
        let mut r = BitReader::new(&bits);
        assert_eq!(PAIR_TABLES[1].read(&mut r), (-1, 1));

        // Codeword "1" → (0,0), no sign bits.
        let bits = pack(&[(0b1, 1)]);
        let mut r = BitReader::new(&bits);
        assert_eq!(PAIR_TABLES[1].read(&mut r), (0, 0));
    }

    #[test]
    fn pair_decode_with_linbits() {
        // A synthetic 2×2 table with linbits=4: (1,1) is "000"; x==max(1) so read
        // 4 linbits then sign; same for y.
        static TL: PairTable = PairTable {
            book: HuffBook::new(&[0b1, 0b001, 0b01, 0b000], &[1, 3, 2, 3]),
            dim: 2,
            linbits: 4,
        };
        // (1,1) code "000"; x: +5 via linbits 0b0100=4 → x=1+4=5, sign 0 → +5;
        // y: linbits 0b0001=1 → y=1+1=2, sign 1 → -2.
        let bits = pack(&[(0b000, 3), (0b0100, 4), (0, 1), (0b0001, 4), (1, 1)]);
        let mut r = BitReader::new(&bits);
        assert_eq!(TL.read(&mut r), (5, -2));
    }

    #[test]
    fn quad_decode_bits_and_signs() {
        // count1 table B codes index i as the 4 bits of 15-i. Index 10 = (1,0,1,0)
        // → codeword 0b0101; then v=1 sign1→-1, x=1 sign0→+1.
        let bits = pack(&[(0b0101, 4), (1, 1), (0, 1)]);
        let mut r = BitReader::new(&bits);
        assert_eq!(QUAD_B.read(&mut r), (-1, 0, 1, 0));
    }

    fn mono_44k() -> FrameHeader {
        FrameHeader {
            version: crate::header::MpegVersion::V1,
            crc_protected: false,
            bitrate_kbps: 128,
            sample_rate: 44100,
            padding: false,
            channel_mode: crate::frame::ChannelMode::Mono,
            copyright: false,
            original: false,
            emphasis: 0,
        }
    }

    /// count1 table B codes quad index i as the 4 bits of 15-i, so the zero quad
    /// is `1111` and (1,0,0,0) is `0111` plus one sign bit.
    const ZERO_QUAD: (u32, u32) = (0b1111, 4);

    #[test]
    fn count1_quad_straddling_the_budget_is_dropped() {
        // Budget 6 bits: the zero quad ends at bit 4, then a (-1,0,0,0) quad starts
        // inside the budget and ENDS at bit 9 -- past it. FFmpeg and minimp3 drop it.
        let bits = pack(&[ZERO_QUAD, (0b0111, 4), (1, 1), ZERO_QUAD, ZERO_QUAD]);
        let gi = GranuleSideInfo {
            count1table_select: true,
            ..Default::default()
        };
        let mut pos = 0;
        let (out, nz) = decode(&bits, &mut pos, 6, &mono_44k(), &gi);
        assert_eq!(out[4], 0, "the straddling quad must not be decoded");
        assert_eq!(nz, 4);
        assert_eq!(pos, 6, "the cursor still ends at the granule's budget");
    }

    #[test]
    fn count1_quad_reaching_line_576_is_kept_even_if_it_overruns() {
        // 143 zero quads fill lines 0..572; the 144th starts at bit 572, inside a
        // 574-bit budget, and ends at 577. The loop stops on the LINE limit, which
        // FFmpeg tests before the bit position, so this quad is kept.
        let mut toks = vec![ZERO_QUAD; 143];
        toks.extend([(0b0111, 4), (0, 1)]);
        let bits = pack(&toks);
        let gi = GranuleSideInfo {
            count1table_select: true,
            ..Default::default()
        };
        let mut pos = 0;
        let (out, nz) = decode(&bits, &mut pos, 574, &mono_44k(), &gi);
        assert_eq!(out[572], 1, "a quad that completes the spectrum is kept");
        assert_eq!(nz, 576);
    }

    /// The LUT decode must be bit-identical to the reference linear scan: for
    /// every codeword in every book, both return the same symbol and consume the
    /// same number of bits — under several trailing-bit contexts (so escapes and
    /// the buffer boundary are exercised). This is the bit-exact gate for the LUT.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn lut_matches_linear_for_every_codeword() {
        fn check(book: &HuffBook) {
            for (i, &len) in book.lens.iter().enumerate() {
                if len == 0 {
                    continue;
                }
                let code = book.codes[i];
                for &filler in &[0u32, 0xFFFF_FFFF, 0xAAAA_AAAA, 0x5555_5555] {
                    let mut w = crate::bitio::BitWriter::new();
                    w.write(u32::from(code), u32::from(len));
                    w.write(filler, 24); // plenty of trailing bits to peek into
                    let bits = w.finish();

                    let mut rl = BitReader::new(&bits);
                    let sym_lin = book.decode_linear(&mut rl);
                    let mut ru = BitReader::new(&bits);
                    let sym_lut = book.decode_index(&mut ru);

                    assert_eq!(sym_lin, Some(i), "linear must decode its own codeword");
                    assert_eq!(sym_lut, sym_lin, "LUT vs linear symbol (book idx {i})");
                    assert_eq!(
                        ru.bit_pos(),
                        rl.bit_pos(),
                        "LUT vs linear bits consumed (book idx {i})"
                    );
                    assert_eq!(rl.bit_pos(), len as usize, "consumed == codeword length");
                }
            }
        }
        for t in &PAIR_TABLES {
            if t.dim != 0 {
                check(&t.book);
            }
        }
        check(&QUAD_A.book);
        check(&QUAD_B.book);
    }

    #[test]
    fn region_bounds_long_and_short() {
        use crate::frame::GranuleSideInfo;
        let sfb = tables::sfb_long_offsets(44100);
        // Long: region0_count=7, region1_count=2 → i1=8 (sfb[8]=36), i2=11 (sfb[11]=62).
        let mut gi = GranuleSideInfo {
            big_values: 100,
            region0_count: 7,
            region1_count: 2,
            ..Default::default()
        };
        assert_eq!(region_bounds(&gi, sfb, 200, 44100), (36, 62));
        // Pure short window-switched: region0 fixed at 36.
        gi.window_switching = true;
        gi.block_type = BlockType::Short;
        assert_eq!(region_bounds(&gi, sfb, 200, 44100), (36, 200));
        // Start/Stop: region0 = sfb_long[8] (36 at 44.1 kHz, but rate-specific for LSF).
        gi.block_type = BlockType::Start;
        assert_eq!(region_bounds(&gi, sfb, 200, 44100), (36, 200));
        assert_eq!(
            region_bounds(&gi, tables::sfb_long_offsets(22050), 200, 22050).0,
            tables::sfb_long_offsets(22050)[8] as usize
        );
    }
}

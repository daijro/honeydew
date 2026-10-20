//! FLAC writing. `flac` encodes finished PCM (the offline renderer's output) as a `.flac` file,
//! and `FlacEncoder` takes the audio in slices as it is rendered. The stream is `fLaC`, one STREAMINFO block
//! (MD5 left zero = unknown), then frames of 4096
//! samples, the last one shorter, inside the format's streamable subset (each frame header carries its sample rate and
//! bit depth; LPC order ≤ 12, Rice partition order ≤ 8). A stereo frame takes the cheapest of independent, left/side,
//! right/side and mid/side. Each subframe is the cheapest of VERBATIM, the best FIXED order 0…4 and the best LPC
//! order 1…12, its residual in the cheapest Rice partitioning (each partition's parameter or escape code; parameters
//! past 14 switch the subframe to the 5-bit parameter method).
//!
//! LPC is Levinson–Durbin on the f64 autocorrelation under a fixed Tukey(0.5)-shaped window (a cubic taper, not a
//! cosine), the coefficients quantized with the rounding error carried forward. Floats see only exactly rounded IEEE-754
//! operations (+, −, ×, ÷, `round`, comparisons, exact conversions) in a fixed order (Rust neither fuses nor reorders
//! f64 operations) and no transcendental, so the output is byte-identical on every IEEE-754 target (x86-64, aarch64,
//! wasm32; not x87). The residual is computed from the integer coefficients, so float results only steer the choice and
//! never affect losslessness.
//!
//! ponytail: no CONSTANT subframe and no wasted-bits detection (silence already costs ~2 bytes per subframe through the
//! zero-width escape); one window where `flac -8` tries several (the bundled content still comes out ~2% under it).

use crate::sample::SampleMeta;

/// Samples per frame and channel.
const BLOCK: usize = 4096;
/// Rice parameters: residual coding method 0's 4-bit field holds 0…14 (15 is its escape code), method 1's 5-bit
/// field 0…30 (31).
const MAX_RICE: u32 = 30;
/// The streamable subset's limits on LPC order and Rice partition order.
const MAX_LPC: usize = 12;
const MAX_PARTITION: u32 = 8;
/// The LPC coefficient precision, the format's most: on the bundled content it beat libFLAC's 12 for 16-bit audio, and a
/// search over 5…15 bits gained under 0.1% for over a third of the encoding time.
const PRECISION: u32 = 15;

/// MSB-first bit sink.
#[derive(Default)]
struct Bits {
    out: Vec<u8>,
    acc: u64,
    pending: u32,
}

impl Bits {
    /// The low `bits` (≤ 56) bits of `value`.
    fn put(&mut self, value: u64, bits: u32) {
        debug_assert!(bits <= 56);
        self.acc = (self.acc << bits) | (value & ((1 << bits) - 1));
        self.pending += bits;
        while self.pending >= 8 {
            self.pending -= 8;
            self.out.push((self.acc >> self.pending) as u8);
        }
    }

    /// `q` zeros then a one.
    fn unary(&mut self, mut q: u64) {
        while q >= 32 {
            self.put(0, 32);
            q -= 32;
        }
        self.put(1, q as u32 + 1);
    }

    /// FLAC's UTF-8 style number: the standard 1…4-byte forms, extended to 7 bytes (< 2^36).
    fn utf8(&mut self, n: u64) {
        if n < 0x80 {
            return self.put(n, 8);
        }
        let len = (2..=7).find(|&l| n >> (5 * l + 1) == 0).expect("< 2^36");
        self.put(
            u64::from((0xFF00u16 >> len) as u8) | n >> (6 * (len - 1)),
            8,
        );
        for i in (0..len - 1).rev() {
            self.put(0x80 | (n >> (6 * i) & 0x3F), 8);
        }
    }

    /// Bits written so far.
    fn len(&self) -> u64 {
        self.out.len() as u64 * 8 + u64::from(self.pending)
    }

    fn append(&mut self, other: &Bits) {
        other.out.iter().for_each(|&b| self.put(b.into(), 8));
        self.put(other.acc, other.pending);
    }

    fn pad(&mut self) {
        if self.pending > 0 {
            self.put(0, 8 - self.pending);
        }
    }
}

/// CRC-8 (x⁸+x²+x+1) of a frame header.
fn crc8(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |mut crc, &b| {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                crc << 1 ^ 0x07
            } else {
                crc << 1
            };
        }
        crc
    })
}

const CRC16_TABLE: [u16; 256] = {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u16) << 8;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 0x8000 != 0 {
                crc << 1 ^ 0x8005
            } else {
                crc << 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// CRC-16 (x¹⁶+x¹⁵+x²+1) of a whole frame.
fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0, |crc, &b| {
        crc << 8 ^ CRC16_TABLE[usize::from((crc >> 8) as u8 ^ b)]
    })
}

fn zigzag(r: i32) -> u64 {
    u64::from((r << 1 ^ r >> 31) as u32)
}

/// How a residual block is entropy coded.
#[derive(Clone, Copy)]
enum Rice {
    Param(u32),
    /// Every residual as a signed integer of this many bits.
    Escape(u32),
}

/// The cheapest coding of `res` (never empty) in one partition and its size in bits, parameter field included.
fn rice_plan(res: &[i32]) -> (u64, Rice) {
    let n = res.len() as u64;
    let mean = res.iter().map(|&r| zigzag(r)).sum::<u64>() / n;
    let cost = |k: u32| {
        let quotients: u64 = res.iter().map(|&r| zigzag(r) >> k).sum();
        (4 + n * u64::from(k + 1) + quotients, Rice::Param(k))
    };
    // The cost is convex in k (each residual's quotient halves per step), so a walk downhill from log2(mean) finds the
    // best of the 31 parameters in a few passes.
    let mut k = mean.checked_ilog2().unwrap_or(0).min(MAX_RICE);
    let mut best = cost(k);
    for step in [-1, 1] {
        while let Some(next) = k.checked_add_signed(step).filter(|&k| k <= MAX_RICE) {
            let c = cost(next);
            if c.0 >= best.0 {
                break;
            }
            (k, best) = (next, c);
        }
    }
    let width = res
        .iter()
        .map(|&r| {
            if r == 0 {
                0
            } else {
                33 - (r ^ r >> 31).leading_zeros()
            }
        })
        .max()
        .unwrap_or(0);
    let escape = 4 + 5 + n * u64::from(width);
    if escape < best.0 {
        (escape, Rice::Escape(width))
    } else {
        best
    }
}

/// One step of FIXED prediction: the order-k residual of `x` is the order-(k−1) residual differenced.
fn difference(res: &mut Vec<i32>) {
    for i in 1..res.len() {
        res[i - 1] = res[i] - res[i - 1];
    }
    res.pop();
}

/// Rice-coding cost of `res` in one partition, as if each u >> k were u / 2^k: the best of the 31 parameters, from one
/// sum. Close enough to rank predictors and partitionings; `rice_plan` then picks the coding exactly.
fn rice_estimate(res: &[i32]) -> u64 {
    let n = res.len() as u64;
    let sum: u64 = res.iter().map(|&r| zigzag(r)).sum();
    (0..=MAX_RICE)
        .map(|k| 4 + n * u64::from(k + 1) + (sum >> k))
        .min()
        .expect("31 parameters")
}

/// The Rice partitioning (order 0…8) of the residual of an `order`-sample warm-up, by estimate, and each partition's
/// exact plan.
fn partitioned(res: &[i32], order: usize) -> (u32, Vec<Rice>) {
    let n = res.len() + order;
    let mut best = (u64::MAX, 0);
    for p in 0..=MAX_PARTITION {
        // Partitions split the block evenly, and the first one, short by the warm-up, must keep a residual.
        let len = n >> p;
        if !n.is_multiple_of(1 << p) || len <= order {
            break;
        }
        let cost = partitions(res, len - order, len).map(rice_estimate).sum();
        if cost < best.0 {
            best = (cost, p);
        }
    }
    let len = n >> best.1;
    let plans = partitions(res, len - order, len)
        .map(|part| rice_plan(part).1)
        .collect();
    (best.1, plans)
}

fn partitions(res: &[i32], first: usize, len: usize) -> impl Iterator<Item = &[i32]> {
    std::iter::once(&res[..first]).chain(res[first..].chunks(len))
}

/// A subframe's predictor: FIXED of an order, or LPC with `precision`-bit coefficients (the most recent sample's first)
/// and a right shift.
enum Predictor {
    Fixed(usize),
    Lpc {
        q: Vec<i32>,
        precision: u32,
        shift: u32,
    },
}

/// Predictor weights of orders 1, 2 … from the autocorrelation `r` (Levinson–Durbin), the most recent sample's first.
/// It stops early once the prediction error is gone.
fn levinson(r: &[f64]) -> Vec<Vec<f64>> {
    let (mut a, mut err, mut out) = (Vec::<f64>::new(), r[0], Vec::new());
    for i in 0..r.len() - 1 {
        if err <= 0.0 {
            break;
        }
        let k = (r[i + 1]
            - a.iter()
                .zip(r[1..=i].iter().rev())
                .map(|(a, r)| a * r)
                .sum::<f64>())
            / err;
        a = a
            .iter()
            .zip(a.iter().rev())
            .map(|(a, b)| a - k * b)
            .chain([k])
            .collect();
        err *= 1.0 - k * k;
        out.push(a.clone());
    }
    out
}

/// `a` as `precision`-bit integers and a right shift (0…15), the largest using the full precision, each rounding error
/// carried into the next coefficient (as libFLAC does). `None` when `a` is all zero or needs a negative shift.
fn quantize(a: &[f64], precision: u32) -> Option<(Vec<i32>, u32)> {
    let cmax = a.iter().fold(0.0f64, |m, c| m.max(c.abs()));
    if !cmax.is_normal() {
        return None;
    }
    // floor(log2 cmax), read from the exponent so no libm call is involved.
    let log2 = (cmax.to_bits() >> 52) as i32 - 1023;
    let shift = u32::try_from(precision as i32 - 2 - log2).ok()?.min(15);
    let (scale, max) = (f64::from(1u32 << shift), (1 << (precision - 1)) - 1);
    let mut carry = 0.0;
    let q = a
        .iter()
        .map(|&c| {
            carry += c * scale;
            let q = (carry.round() as i32).clamp(-max - 1, max);
            carry -= f64::from(q);
            q
        })
        .collect();
    Some((q, shift))
}

/// `x` after the warm-up minus its LPC prediction, as the decoder computes it (i64 sum, arithmetic shift). `None` if a
/// residual reaches ±2^30, past what the 5-bit escape width and a decoder's i32 hold.
fn lpc_residual(x: &[i32], q: &[i32], shift: u32) -> Option<Vec<i32>> {
    // Coefficient by coefficient over the whole block, so the loop vectorizes.
    let order = q.len();
    let mut p = vec![0i64; x.len() - order];
    for (j, &c) in q.iter().enumerate() {
        for (p, &s) in p.iter_mut().zip(&x[order - 1 - j..]) {
            *p += i64::from(c) * i64::from(s);
        }
    }
    p.iter()
        .zip(&x[order..])
        .map(|(&p, &s)| {
            let r = i64::from(s) - (p >> shift);
            (r.abs() < 1 << 30).then_some(r as i32)
        })
        .collect()
}

/// The best LPC predictor of order 1…12 for `x` by estimated cost, and its residual, every order quantized at
/// `PRECISION` bits.
fn lpc(bits: u32, x: &[i32]) -> Option<(Predictor, Vec<i32>)> {
    let n = x.len();
    // Tukey(0.5): flat in the middle half, a smoothstep taper over each outer quarter.
    let taper = (n / 4).max(1) as f64;
    let xw: Vec<f64> = (0..n)
        .map(|i| {
            let t = (i.min(n - 1 - i) as f64 + 0.5) / taper;
            f64::from(x[i])
                * if t < 1.0 {
                    t * t * (3.0 - 2.0 * t)
                } else {
                    1.0
                }
        })
        .collect();
    let r: Vec<f64> = (0..=MAX_LPC.min(n - 1))
        .map(|lag| xw[lag..].iter().zip(&xw).map(|(a, b)| a * b).sum())
        .collect();
    let orders = levinson(&r);
    let coded = |a: &[f64], precision: u32| {
        let (q, shift) = quantize(a, precision)?;
        let res = lpc_residual(x, &q, shift)?;
        let cost = a.len() as u64 * u64::from(bits + precision) + 9 + rice_estimate(&res);
        Some((
            cost,
            Predictor::Lpc {
                q,
                precision,
                shift,
            },
            res,
        ))
    };
    let (_, predictor, res) = orders
        .iter()
        .filter_map(|a| coded(a, PRECISION))
        .min_by_key(|c| c.0)?;
    Some((predictor, res))
}

/// The subframe coding `x` (1…4096 samples of `bits` each) with `predictor`, whose residual is `res`.
fn coded_subframe(bits: u32, x: &[i32], predictor: &Predictor, res: &[i32]) -> Bits {
    let mut w = Bits::default();
    let order = match predictor {
        Predictor::Fixed(order) => *order,
        Predictor::Lpc { q, .. } => q.len(),
    };
    let kind = match predictor {
        Predictor::Fixed(order) => 0b001000 | *order as u64,
        Predictor::Lpc { q, .. } => 0b100000 | (q.len() as u64 - 1),
    };
    w.put(kind << 1, 8); // pad, type, no wasted bits
    x[..order]
        .iter()
        .for_each(|&s| w.put(s as u32 as u64, bits));
    if let Predictor::Lpc {
        q,
        precision,
        shift,
    } = predictor
    {
        w.put(u64::from(precision - 1), 4);
        w.put(u64::from(*shift), 5);
        q.iter().for_each(|&c| w.put(c as u32 as u64, *precision));
    }
    let (partition_order, plans) = partitioned(res, order);
    // Method 1 (5-bit parameters) only when a partition needs a parameter past method 0's 14.
    let field = if plans.iter().any(|r| matches!(r, Rice::Param(k) if *k > 14)) {
        5
    } else {
        4
    };
    w.put(u64::from(field - 4), 2); // residual coding method 0 or 1
    w.put(u64::from(partition_order), 4);
    let len = x.len() >> partition_order;
    for (part, rice) in partitions(res, len - order, len).zip(plans) {
        match rice {
            Rice::Param(k) => {
                w.put(u64::from(k), field);
                for &r in part {
                    let u = zigzag(r);
                    w.unary(u >> k);
                    w.put(u, k);
                }
            }
            Rice::Escape(width) => {
                w.put((1 << field) - 1, field);
                w.put(u64::from(width), 5);
                part.iter().for_each(|&r| w.put(r as u32 as u64, width));
            }
        }
    }
    w
}

/// The shortest subframe for `x`: the best FIXED order and the best LPC predictor (each chosen by estimate, then coded
/// in full), or VERBATIM. Ties go to the simpler coding.
fn subframe(bits: u32, x: &[i32]) -> Bits {
    let mut res = x.to_vec();
    let mut fixed = (u64::MAX, Predictor::Fixed(0), Vec::new());
    for order in 0..=4.min(x.len() - 1) {
        if order > 0 {
            difference(&mut res);
        }
        let cost = order as u64 * u64::from(bits) + rice_estimate(&res);
        if cost < fixed.0 {
            fixed = (cost, Predictor::Fixed(order), res.clone());
        }
    }
    let mut verbatim = Bits::default();
    verbatim.put(0b0000_0010, 8); // pad, VERBATIM, no wasted bits
    x.iter().for_each(|&s| verbatim.put(s as u32 as u64, bits));
    [Some((fixed.1, fixed.2)), lpc(bits, x)]
        .into_iter()
        .flatten()
        .map(|(predictor, res)| coded_subframe(bits, x, &predictor, &res))
        .chain([verbatim])
        .min_by_key(Bits::len)
        .expect("FIXED always codes")
}

/// The frame header's 4-bit sample-rate code, then the value (and its width in bits) that follows the block size for
/// the three escape codes. Code 0 (the rate is in STREAMINFO) is only the fallback for rates nothing else can say.
fn rate_code(rate: u32) -> (u64, u64, u32) {
    match rate {
        88_200 => (1, 0, 0),
        176_400 => (2, 0, 0),
        192_000 => (3, 0, 0),
        8_000 => (4, 0, 0),
        16_000 => (5, 0, 0),
        22_050 => (6, 0, 0),
        24_000 => (7, 0, 0),
        32_000 => (8, 0, 0),
        44_100 => (9, 0, 0),
        48_000 => (10, 0, 0),
        96_000 => (11, 0, 0),
        r if r % 1000 == 0 && r / 1000 <= 255 => (12, u64::from(r / 1000), 8),
        r if r <= 65_535 => (13, u64::from(r), 16),
        // `flac` caps the rate at 655 350 Hz, so the tens fit 16 bits.
        r if r % 10 == 0 => (14, u64::from(r / 10), 16),
        _ => (0, 0, 0),
    }
}

/// One frame: `block` holds the quantized samples of every channel, all of one length (1…4096).
fn frame(number: u64, rate: u32, bits: u32, block: &[Vec<i32>]) -> Vec<u8> {
    let n = block[0].len();
    let size_code = match n {
        BLOCK => 12,
        ..=256 => 6,
        _ => 7,
    };
    let mut w = Bits::default();
    w.put(0xFFF8, 16); // sync, reserved, fixed block size
    w.put(size_code, 4);
    let (rate_code, rate_value, rate_bits) = rate_code(rate);
    w.put(rate_code, 4);
    // Independent channels, or for stereo the cheapest of left/side (8), right/side (9) and mid/side (10).
    let mut subframes: Vec<Bits> = block.iter().map(|x| subframe(bits, x)).collect();
    let mut assignment = block.len() as u64 - 1;
    if let [l, r] = block {
        let side = subframe(
            bits + 1,
            &l.iter().zip(r).map(|(l, r)| l - r).collect::<Vec<_>>(),
        );
        let mid = subframe(
            bits,
            &l.iter()
                .zip(r)
                .map(|(l, r)| (l + r) >> 1)
                .collect::<Vec<_>>(),
        );
        let (left, right) = (subframes[0].len(), subframes[1].len());
        let costs = [
            left + right,
            left + side.len(),
            side.len() + right,
            mid.len() + side.len(),
        ];
        match (0..4).min_by_key(|&i| costs[i]) {
            Some(1) => (assignment, subframes[1]) = (8, side),
            Some(2) => (assignment, subframes[0]) = (9, side),
            Some(3) => (assignment, subframes) = (10, vec![mid, side]),
            _ => {}
        }
    }
    w.put(assignment, 4);
    w.put(if bits == 16 { 0b100 } else { 0b110 }, 3);
    w.put(0, 1);
    w.utf8(number);
    match size_code {
        6 => w.put(n as u64 - 1, 8),
        7 => w.put(n as u64 - 1, 16),
        _ => {}
    }
    w.put(rate_value, rate_bits);
    let header_crc = crc8(&w.out);
    w.put(u64::from(header_crc), 8);
    subframes.iter().for_each(|s| w.append(s));
    w.pad();
    let crc = crc16(&w.out);
    w.put(u64::from(crc), 16);
    w.out
}

/// A streaming encoder: `push` the audio as it is rendered, `finish` for the file. Every completed 4096-frame block is
/// encoded inside the `push` that completes it, so the work spreads over the pushes and one call never holds more than
/// a block of quantized samples.
pub struct FlacEncoder {
    rate: u32,
    bits: u8,
    scale: f64,
    /// Quantized samples per channel, not yet a full block.
    pending: Vec<Vec<i32>>,
    body: Vec<u8>,
    /// Frames (samples per channel) in `body`, and the encoded frame sizes seen.
    frames: u64,
    smallest: usize,
    largest: usize,
    blocks: u64,
}

impl FlacEncoder {
    /// An encoder of `channels` (1…8) at `rate` Hz (1…655350), `bits` 16 or 24 per sample. Samples are stored as
    /// trunc(x·(2^(bits−1)−1)), clamped (NaN → 0), as `WavWriter` does.
    ///
    /// # Panics
    /// On a `rate`, `bits` or channel count outside those ranges.
    pub fn new(rate: u32, bits: u8, channels: usize) -> Self {
        assert!(matches!(bits, 16 | 24), "FLAC depth is 16 or 24 bits");
        assert!((1..=8).contains(&channels), "FLAC has 1…8 channels");
        assert!((1..=655_350).contains(&rate), "FLAC rate is 1…655350 Hz");
        Self {
            rate,
            bits,
            scale: f64::from((1u32 << (bits - 1)) - 1),
            pending: vec![Vec::with_capacity(BLOCK); channels],
            body: Vec::new(),
            frames: 0,
            smallest: usize::MAX,
            largest: 0,
            blocks: 0,
        }
    }

    /// Appends one slice of samples per channel. The slices may be any length, but of equal length: if they differ,
    /// the shortest counts and the rest of the longer ones is dropped.
    ///
    /// # Panics
    /// If there is not exactly one slice per channel.
    pub fn push<C: AsRef<[f32]>>(&mut self, channels: &[C]) {
        let scale = self.scale;
        self.extend(channels, |&x| {
            (f64::from(x) * scale).clamp(-scale, scale) as i32
        });
    }

    /// [`Self::push`] for samples already quantized to 16 bits (dithered, say): stored exactly, shifted up 8 bits in a
    /// 24-bit stream.
    ///
    /// # Panics
    /// If there is not exactly one slice per channel.
    pub fn push_i16<C: AsRef<[i16]>>(&mut self, channels: &[C]) {
        let shift = self.bits - 16;
        self.extend(channels, |&s| i32::from(s) << shift);
    }

    /// [`Self::push`] for samples already at the stream's depth (within ±2^(bits−1)): stored exactly.
    ///
    /// # Panics
    /// If there is not exactly one slice per channel.
    pub fn push_i32<C: AsRef<[i32]>>(&mut self, channels: &[C]) {
        self.extend(channels, |&s| s);
    }

    fn extend<T, C: AsRef<[T]>>(&mut self, channels: &[C], quantize: impl Fn(&T) -> i32) {
        assert_eq!(channels.len(), self.pending.len(), "one slice per channel");
        let len = channels.iter().map(|c| c.as_ref().len()).min().unwrap_or(0);
        let mut at = 0;
        while at < len {
            let take = (BLOCK - self.pending[0].len()).min(len - at);
            for (pending, c) in self.pending.iter_mut().zip(channels) {
                pending.extend(c.as_ref()[at..at + take].iter().map(&quantize));
            }
            at += take;
            if self.pending[0].len() == BLOCK {
                self.flush();
            }
        }
    }

    /// Encodes whatever is pending (a block of 1…4096 frames) as the next frame.
    fn flush(&mut self) {
        let len = self.pending[0].len();
        if len == 0 {
            return;
        }
        let bytes = frame(self.blocks, self.rate, u32::from(self.bits), &self.pending);
        (self.smallest, self.largest) = (
            self.smallest.min(bytes.len()),
            self.largest.max(bytes.len()),
        );
        self.body.extend(bytes);
        self.pending.iter_mut().for_each(Vec::clear);
        self.frames += len as u64;
        self.blocks += 1;
    }

    /// The finished file: the last, shorter block is encoded and the header written in front.
    pub fn finish(self) -> Vec<u8> {
        self.finish_with_meta(&SampleMeta::default())
    }

    /// [`Self::finish`], plus an APPLICATION `FLSM` block after STREAMINFO that `sample::decode`
    /// reads back as `meta`: `fLmL` (tempo, beats, loop; root note `unity_note` or 0xFF) when any of tempo, beats and
    /// loop is set, then `fLsF` holding a `cue ` chunk when there are cues. An empty `meta` adds no block. The Slicer's
    /// tempo, beats and slice markers travel this way.
    pub fn finish_with_meta(mut self, meta: &SampleMeta) -> Vec<u8> {
        self.flush();
        let (smallest, largest) = if self.frames == 0 {
            (0, 0)
        } else {
            (self.smallest, self.largest)
        };
        let mut info = Bits::default();
        info.put(BLOCK as u64, 16); // minimum and maximum block size: the configured one, even for a short stream
        info.put(BLOCK as u64, 16);
        info.put(smallest as u64, 24);
        info.put(largest as u64, 24);
        info.put(u64::from(self.rate), 20);
        info.put(self.pending.len() as u64 - 1, 3);
        info.put(u64::from(self.bits) - 1, 5);
        info.put(self.frames, 36);
        info.out.extend([0; 16]); // MD5: unknown
        let flsm = flsm(meta);
        // STREAMINFO, 34 bytes, the last metadata block unless FLSM follows.
        let mut out = [&b"fLaC"[..], &[u8::from(flsm.is_empty()) << 7, 0, 0, 34]].concat();
        out.extend(info.out);
        if !flsm.is_empty() {
            assert!(flsm.len() < 1 << 24, "an FLSM block holds under 16 MiB");
            out.extend(&(0x8200_0000 | flsm.len() as u32).to_be_bytes()); // APPLICATION, last, 24-bit length
            out.extend(flsm);
        }
        out.extend(self.body);
        out
    }
}

/// The `FLSM` APPLICATION block's data for `meta` (its ID, then the chunks), empty when there is nothing to carry.
fn flsm(meta: &SampleMeta) -> Vec<u8> {
    let chunk = |id: &[u8; 4], payload: &[u8]| {
        [&id[..], &(payload.len() as u32).to_le_bytes(), payload].concat()
    };
    let mut chunks = Vec::new();
    if meta.tempo.is_some() || meta.beats.is_some() || meta.loop_frames.is_some() {
        let (start, end) = meta.loop_frames.unwrap_or_default();
        let ml = [
            &meta.tempo.unwrap_or(0.0).to_le_bytes()[..],
            &(meta.beats.unwrap_or(0).min(u32::from(u16::MAX)) as u16).to_le_bytes(),
            &start.to_le_bytes(),
            &end.to_le_bytes(),
            &[meta.unity_note.map_or(0xFF, |n| n.min(127) as u8), 0],
        ]
        .concat();
        chunks.extend(chunk(b"fLmL", &ml));
    }
    if !meta.cues.is_empty() {
        // RIFF `cue `: a count, then 24-byte points (ID, position, `data`, chunk and block start 0, sample offset).
        let mut cue = (meta.cues.len() as u32).to_le_bytes().to_vec();
        for (id, &at) in (1u32..).zip(&meta.cues) {
            cue.extend(
                [
                    id.to_le_bytes(),
                    at.to_le_bytes(),
                    *b"data",
                    [0; 4],
                    [0; 4],
                    at.to_le_bytes(),
                ]
                .concat(),
            );
        }
        chunks.extend(chunk(b"fLsF", &chunk(b"cue ", &cue)));
    }
    if chunks.is_empty() {
        return chunks;
    }
    [&b"FLSM"[..], &chunks].concat()
}

/// A FLAC file of `channels` (1…8, equal-length frames; extra frames in a longer channel are dropped) at `rate` Hz
/// (1…655350), `bits` 16 or 24 per sample: `FlacEncoder` fed in one push.
///
/// # Panics
/// On a `rate`, `bits` or channel count outside those ranges.
pub fn flac<C: AsRef<[f32]>>(rate: u32, bits: u8, channels: &[C]) -> Vec<u8> {
    let mut encoder = FlacEncoder::new(rate, bits, channels.len());
    encoder.push(channels);
    encoder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_numbers_match_std_for_unicode_scalars_and_the_long_forms() {
        for n in (0..0x10_FFFF).filter(|n| !(0xD800..0xE000).contains(n)) {
            let mut w = Bits::default();
            w.utf8(u64::from(n));
            let mut buf = [0; 4];
            let expected = char::from_u32(n).unwrap().encode_utf8(&mut buf).as_bytes();
            assert_eq!(w.out, expected, "{n:#x}");
        }
        let mut w = Bits::default();
        w.utf8(0x7FFF_FFFF);
        assert_eq!(w.out, [0xFD, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF]);
        let mut w = Bits::default();
        w.utf8(0xF_FFFF_FFFF);
        assert_eq!(w.out, [0xFE, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF, 0xBF]);
    }

    #[test]
    fn crcs_match_the_published_check_values() {
        // CRC-8/SMBUS and CRC-16/UMTS "123456789" check values.
        assert_eq!(crc8(b"123456789"), 0xF4);
        assert_eq!(crc16(b"123456789"), 0xFEE8);
    }

    /// Asserts `rice_plan` returns the cheapest of the 31 parameters and the escape, and that its coding costs what it says.
    fn assert_optimal(res: &[i32], label: &str) {
        let (cost, rice) = rice_plan(res);
        let rice_cost = |k: u32| {
            4 + res
                .iter()
                .map(|&r| (zigzag(r) >> k) + 1 + u64::from(k))
                .sum::<u64>()
        };
        let width = res
            .iter()
            .map(|&r| {
                if r == 0 {
                    0
                } else {
                    33 - (r ^ r >> 31).leading_zeros()
                }
            })
            .max()
            .unwrap();
        let escape = 9 + res.len() as u64 * u64::from(width);
        let brute = (0..=MAX_RICE).map(rice_cost).min().unwrap();
        assert_eq!(cost, brute.min(escape), "{label}: cost");
        match rice {
            Rice::Param(k) => assert_eq!(cost, rice_cost(k), "{label}: parameter {k}"),
            Rice::Escape(w) => assert_eq!((cost, w), (escape, width), "{label}: escape"),
        }
    }

    fn xorshift(mut seed: u32) -> impl FnMut() -> u32 {
        move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        }
    }

    /// Laplacian-ish residuals with the given mean magnitude.
    fn laplacian(rand: &mut impl FnMut() -> u32, n: usize, scale: f64) -> Vec<i32> {
        (0..n)
            .map(|_| {
                let u = f64::from(rand() >> 8) / f64::from(1u32 << 24);
                let mag = (-(1.0 - u).ln() * scale) as i32;
                if rand() & 1 == 0 { mag } else { -mag }
            })
            .collect()
    }

    #[test]
    fn the_rice_parameter_is_the_best_of_all() {
        let mut rand = xorshift(0x1234_5678);
        for scale_bits in 0..=24 {
            // Every magnitude, plus an outlier that drags the mean off the best parameter.
            let mut res = laplacian(&mut rand, BLOCK, f64::from(1u32 << scale_bits));
            res[7] = 1 << 26;
            assert_optimal(&res, &format!("scale 2^{scale_bits}"));
        }
    }

    #[test]
    fn the_rice_parameter_is_the_best_in_quarter_octave_steps_of_a_short_block() {
        // The best parameter is rarely the one log2(mean) names, so this fails if the downhill walk is dropped.
        let mut rand = xorshift(0x9E37_79B9);
        for step in 0..=96 {
            let scale = (f64::from(step) / 4.0).exp2();
            for _ in 0..4 {
                assert_optimal(
                    &laplacian(&mut rand, 256, scale),
                    &format!("scale 2^({step}/4)"),
                );
            }
        }
    }
}

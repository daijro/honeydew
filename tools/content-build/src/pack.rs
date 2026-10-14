//! Pack parts: an entry's members, in priority order, split into uncompressed ustar files of at most
//! [`CAP`] bytes raw and gzipped, with each member's data offset recorded so no client ever parses a tar header.

use sha2::{Digest, Sha256};

/// The part cap, raw and as served gzipped: under both 25 MB and Cloudflare Pages' 25 MiB.
pub const CAP: usize = 24_000_000;
const BLOCK: usize = 512;
/// The two zero blocks that end a tar.
const END: usize = 2 * BLOCK;

/// One finished part.
pub struct Part {
    pub tar: Vec<u8>,
    pub gz: Vec<u8>,
    /// Index into the input members, and the data offset in `tar`, for each member this part holds.
    pub members: Vec<(usize, u64)>,
}

/// Splits `members` (name relative to the entry base, bytes) into parts in order: greedy fill up to `cap` with headers,
/// padding and end blocks counted, dropping trailing members while the gzip is over `cap`. A member never spans parts.
/// When `lite > 0`, part 0 closes right after the first `lite` members (the lite set), so the first fetch is
/// small.
pub fn split(members: &[(String, Vec<u8>)], lite: usize, cap: usize) -> Result<Vec<Part>, String> {
    let lite = lite.min(members.len());
    let mut parts: Vec<Part> = Vec::new();
    let mut start = 0;
    while start < members.len() {
        let stop = if parts.is_empty() && lite > 0 {
            lite
        } else {
            members.len()
        };
        let (mut end, mut size) = (start, END);
        while end < stop && size + BLOCK + padded(members[end].1.len()) <= cap {
            size += BLOCK + padded(members[end].1.len());
            end += 1;
        }
        loop {
            if end == start {
                return Err(format!(
                    "{}: {} bytes do not fit in one part",
                    members[start].0,
                    members[start].1.len()
                ));
            }
            let (tar, offsets) = tar(&members[start..end])?;
            let gz = gzip(&tar);
            if gz.len() <= cap {
                parts.push(Part {
                    tar,
                    gz,
                    members: (start..end).zip(offsets).collect(),
                });
                break;
            }
            end -= 1;
        }
        if parts.len() == 1 && end < lite {
            return Err(format!(
                "{}: the lite set does not fit in part 0",
                members[0].0
            ));
        }
        start = end;
    }
    Ok(parts)
}

fn padded(len: usize) -> usize {
    len.div_ceil(BLOCK) * BLOCK
}

/// A deterministic ustar file: headers with mode 0644, uid/gid 0, mtime 0 and no owner names, then the end blocks.
/// Returns the bytes and each member's data offset.
fn tar(members: &[(String, Vec<u8>)]) -> Result<(Vec<u8>, Vec<u64>), String> {
    let mut out = Vec::new();
    let mut offsets = Vec::new();
    for (name, data) in members {
        out.extend_from_slice(&header(name, data.len())?);
        offsets.push(out.len() as u64);
        out.extend_from_slice(data);
        out.resize(padded(out.len()), 0);
    }
    out.resize(out.len() + END, 0);
    Ok((out, offsets))
}

fn header(name: &str, size: usize) -> Result<[u8; BLOCK], String> {
    let b = name.as_bytes();
    // ustar: names over 100 bytes split at a `/` into a ≤ 155-byte prefix and a ≤ 100-byte name.
    let (prefix, base) = if b.len() <= 100 {
        (&b[..0], b)
    } else {
        let cut = (1..b.len().min(156))
            .rev()
            .find(|&i| b[i] == b'/' && b.len() - i - 1 <= 100 && i + 1 < b.len())
            .ok_or_else(|| format!("{name}: too long for a ustar header"))?;
        (&b[..cut], &b[cut + 1..])
    };
    let mut h = [0u8; BLOCK];
    h[..base.len()].copy_from_slice(base);
    h[100..108].copy_from_slice(b"0000644\0");
    h[108..116].copy_from_slice(b"0000000\0");
    h[116..124].copy_from_slice(b"0000000\0");
    h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    h[136..148].copy_from_slice(b"00000000000\0");
    h[148..156].copy_from_slice(b"        ");
    h[156] = b'0';
    h[257..265].copy_from_slice(b"ustar\x0000");
    h[345..345 + prefix.len()].copy_from_slice(prefix);
    let sum: u32 = h.iter().map(|&x| u32::from(x)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    Ok(h)
}

/// gzip -9 -n: no name and mtime 0, so the output is deterministic.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 2, 255];
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 9));
    out.extend(crc32fast::hash(data).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out
}

/// The body of a gzip member without optional header fields (as [`gzip`] and `gzip -n` write).
pub fn gunzip(gz: &[u8]) -> Option<Vec<u8>> {
    if gz.len() < 18 || gz[..4] != [0x1f, 0x8b, 8, 0] {
        return None;
    }
    miniz_oxide::inflate::decompress_to_vec(&gz[10..gz.len() - 8]).ok()
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Standard base64 with padding, as SRI's `integrity` wants.
pub fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = c
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            s.push(if i <= c.len() {
                A[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(len: usize, seed: u32) -> Vec<u8> {
        let mut x = seed;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    #[test]
    fn split_keeps_members_whole_and_parts_under_the_cap_raw_and_gzipped() {
        let cap = 16_384;
        // Incompressible noise, so some greedy fills land within gzip's overhead of the cap and must drop a member.
        let members: Vec<(String, Vec<u8>)> = (0..40)
            .map(|i| {
                (
                    format!("m{i:02}.flac"),
                    noise(300 + i * 211 % 5_000, i as u32 + 1),
                )
            })
            .collect();
        let parts = split(&members, 3, cap).unwrap();
        assert_eq!(
            parts[0].members.len(),
            3,
            "part 0 closes after the lite set"
        );
        let mut next = 0;
        for p in &parts {
            assert!(
                p.tar.len() <= cap && p.gz.len() <= cap,
                "{} / {}",
                p.tar.len(),
                p.gz.len()
            );
            assert_eq!(gunzip(&p.gz).unwrap(), p.tar);
            for &(i, off) in &p.members {
                assert_eq!(i, next, "order kept, nothing skipped or repeated");
                next += 1;
                let data = &members[i].1;
                assert_eq!(
                    &p.tar[off as usize..off as usize + data.len()],
                    &data[..],
                    "member whole in its part"
                );
            }
        }
        assert_eq!(next, members.len());
        assert!(
            split(&[("big".into(), vec![0; cap])], 0, cap).is_err(),
            "a member over the cap is an error"
        );

        // At the real cap, noise gzips past it (stored-block overhead beats the zero headers and padding): two 12 MB
        // members fit raw together but must go into two parts.
        let noise = noise(2 * (CAP / 2 - 1536), 7);
        let (a, b) = noise.split_at(noise.len() / 2);
        let parts = split(
            &[("a".into(), a.to_vec()), ("b".into(), b.to_vec())],
            0,
            CAP,
        )
        .unwrap();
        assert_eq!(
            parts.iter().map(|p| p.members.len()).collect::<Vec<_>>(),
            [1, 1]
        );
        assert!(
            parts
                .iter()
                .all(|p| p.tar.len() <= CAP && p.gz.len() <= CAP)
        );
    }
}

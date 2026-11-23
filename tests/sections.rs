//! Compressed sections with one or more chunks, built from the documented
//! layout: a minimal container holding one section of free blocks.

use pla_open::{Error, Pla};

const MIB: usize = 1 << 20;
const MARKER: [u8; 16] = [
    0x79, 0xa8, 0xa9, 0x96, 0x45, 0xb4, 0x4a, 0xf3, 0x98, 0x38, 0x33, 0xc3, 0x3a, 0x06, 0x1b, 0x06,
];

/// A free block (type -36) of `len` bytes: tag, unit count, type, 0x01, zeros.
fn free_block(len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    v[0..2].copy_from_slice(b"BL");
    v[2..6].copy_from_slice(&((len / 16) as u32).to_le_bytes());
    v[6..8].copy_from_slice(&(-36i16).to_le_bytes());
    v[8] = 1;
    v
}

fn stored9(data: &[u8]) -> Vec<u8> {
    let mut v = vec![0x46];
    v.extend(((data.len() + 9) as u32).to_le_bytes());
    v.extend((data.len() as u32).to_le_bytes());
    v.extend(data);
    v
}

fn stored3(data: &[u8]) -> Vec<u8> {
    let mut v = vec![0x44, (data.len() + 3) as u8, data.len() as u8];
    v.extend(data);
    v
}

/// Compressed chunk of literals only: a zero control word before every 31 literals.
fn literal(data: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    for group in data.chunks(31) {
        body.extend([0; 4]);
        body.extend(group);
    }
    let mut v = vec![0x47];
    v.extend(((body.len() + 9) as u32).to_le_bytes());
    v.extend((data.len() as u32).to_le_bytes());
    v.extend(body);
    v
}

/// Signature, zeros to 0x20, one section (`ds`, `n`, chunk records), padding to 16.
fn container(ds: u64, n: u64, chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut v = b"ROF FDB ".to_vec();
    v.resize(0x20, 0);
    v.extend(MARKER);
    v.extend(ds.to_le_bytes());
    v.extend(n.to_le_bytes());
    for c in chunks {
        v.extend((c.len() as u32).to_le_bytes());
        v.extend(c);
    }
    v.resize(v.len().div_ceil(16) * 16, 0);
    v
}

fn two_small() -> Vec<u8> {
    [free_block(16), free_block(16)].concat()
}

fn spanning() -> Vec<u8> {
    [free_block(MIB + 16), free_block(32)].concat()
}

fn k4() -> Vec<u8> {
    let b = spanning();
    container(b.len() as u64, 2, &[stored9(&b[..MIB]), literal(&b[MIB..])])
}

/// Name, bytes, blocks expected (none: reject), size, head hex, SHA-256 hex.
type Vector = (
    &'static str,
    Vec<u8>,
    Option<usize>,
    usize,
    &'static str,
    &'static str,
);

fn vectors() -> Vec<Vector> {
    let b = two_small();
    let mut k5 = k4();
    k5[0x38..0x40].copy_from_slice(&1u64.to_le_bytes());
    let mut k7 = container(32, 1, &[stored9(&b)]);
    k7[0x30..0x38].copy_from_slice(&64u64.to_le_bytes());
    vec![
        (
            "k1-one-stored9",
            container(32, 1, &[stored9(&b)]),
            Some(2),
            112,
            "200000000000000001000000000000002900000046290000",
            "940fb42a2d588aee4689787acd822979691d60381bb188af68b761cb7a74f42f",
        ),
        (
            "k2-one-stored3",
            container(32, 1, &[stored3(&b)]),
            Some(2),
            112,
            "200000000000000001000000000000002300000044232042",
            "d42020db5946de861493ca73830a01408d7360112f1e82dd22f7e8f6b0c39750",
        ),
        (
            "k3-one-literal",
            container(48, 1, &[literal(&free_block(48))]),
            Some(1),
            144,
            "300000000000000001000000000000004100000047410000",
            "6141c4f61cfc019d04091f6c5c16ae554800c40df77a8ff432dff556f947268e",
        ),
        (
            "k4-two-chunks-span",
            k4(),
            Some(2),
            1_048_736,
            "300010000000000002000000000000000900100046090010",
            "15da1fd81b28c250f5bac0501ef52b3dc706200ae59af5d29353fd038a244b43",
        ),
        (
            "k5-bad-count",
            k5,
            None,
            1_048_736,
            "300010000000000001000000000000000900100046090010",
            "b79c97bc6f1439943e02517ea8c32d2a40c93cca94f38a3cc9739f3e381dd3ad",
        ),
        (
            "k6-chunk-over-1mib",
            container((MIB + 48) as u64, 1, &[stored9(&spanning())]),
            None,
            1_048_704,
            "300010000000000001000000000000003900100046390010",
            "6c7dbd37a7e8f4251bf27f2dfe33eaa591bf26b48739de5b179c4d60eb9b12c4",
        ),
        (
            "k7-total-mismatch",
            k7,
            None,
            112,
            "400000000000000001000000000000002900000046290000",
            "c710587999322173eb6af0028d72a35bd24ac1b52ea5dccddd7a3a03c7b52a46",
        ),
    ]
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// SHA-256 (FIPS 180-4), enough to check the published vector hashes.
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut m = data.to_vec();
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend(((data.len() as u64) * 8).to_be_bytes());
    for block in m.chunks(64) {
        let mut w = [0u32; 64];
        for (x, b) in w.iter_mut().zip(block.chunks(4)) {
            *x = u32::from_be_bytes(b.try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for (k, wi) in K.iter().zip(w) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(*k)
                .wrapping_add(wi);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            (hh, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

#[test]
fn sha256_known_answer() {
    assert_eq!(
        hex(&sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn test_vectors() {
    for (name, bytes, blocks, len, head, digest) in vectors() {
        assert_eq!(bytes.len(), len, "{name}: size");
        assert_eq!(hex(&bytes[0x30..0x48]), head, "{name}: head");
        assert_eq!(hex(&sha256(&bytes)), digest, "{name}: SHA-256");

        let p = std::env::temp_dir().join(format!("pla-open-{}-{name}.pln", std::process::id()));
        std::fs::write(&p, &bytes).unwrap();
        let result = Pla::open(&p).unwrap().walk(|_| Ok(()));
        std::fs::remove_file(&p).unwrap();
        match blocks {
            Some(n) => {
                let st = result.unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!((st.blocks, st.sections, st.gap_units), (n, 1, 0), "{name}");
            }
            None => assert!(
                matches!(result, Err(Error::Corrupt(_))),
                "{name}: {result:?}"
            ),
        }
    }
}

#[test]
fn rejects_other_section_faults() {
    let b = spanning();
    let cases = [
        // Chunk record size disagrees with its QuickLZ header.
        ("size-mismatch", {
            let mut f = container(32, 1, &[stored9(&two_small())]);
            f[0x40] = 0x2a;
            f
        }),
        // A chunk record that runs past the end of the file.
        ("truncated", k4()[..0x50].to_vec()),
        // First chunk short of 1 MiB although another follows.
        (
            "short-first",
            container(
                b.len() as u64,
                2,
                &[stored9(&b[..MIB - 16]), stored9(&b[MIB - 16..])],
            ),
        ),
        // A section of zero bytes.
        ("empty", container(0, 0, &[])),
        // Blocks that do not end exactly at the decoded length.
        ("walk-overrun", {
            let mut d = two_small();
            d[2..6].copy_from_slice(&3u32.to_le_bytes());
            container(32, 1, &[stored9(&d)])
        }),
    ];
    for (name, bytes) in cases {
        let p = std::env::temp_dir().join(format!("pla-open-{}-{name}.pln", std::process::id()));
        std::fs::write(&p, &bytes).unwrap();
        let result = Pla::open(&p).unwrap().walk(|_| Ok(()));
        std::fs::remove_file(&p).unwrap();
        assert!(result.is_err(), "{name}");
    }
}

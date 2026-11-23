//! QuickLZ 1.5 level 1 decompression (the public chunk format).

use crate::{Error, Result};

fn get(input: &[u8], i: usize) -> u32 {
    input.get(i).copied().unwrap_or(0) as u32
}

fn read_u24(input: &[u8], i: usize) -> u32 {
    get(input, i) | get(input, i + 1) << 8 | get(input, i + 2) << 16
}

fn read_u32(input: &[u8], i: usize) -> u32 {
    read_u24(input, i) | get(input, i + 3) << 24
}

/// Header of one chunk: (header length, chunk size including header, decompressed size, compressed?).
fn chunk_header(input: &[u8]) -> Result<(usize, usize, usize, bool)> {
    let h = *input.first().ok_or(Error::Corrupt("empty QuickLZ chunk"))?;
    if (h >> 2) & 3 != 1 {
        return Err(Error::Unsupported("QuickLZ level other than 1"));
    }
    let (hl, size, dsize) = if h & 2 != 0 {
        if input.len() < 9 {
            return Err(Error::Corrupt("truncated QuickLZ header"));
        }
        (9, read_u32(input, 1) as usize, read_u32(input, 5) as usize)
    } else {
        if input.len() < 3 {
            return Err(Error::Corrupt("truncated QuickLZ header"));
        }
        (3, input[1] as usize, input[2] as usize)
    };
    if size < hl || size > input.len() {
        return Err(Error::Corrupt("QuickLZ chunk size out of range"));
    }
    Ok((hl, size, dsize, h & 1 != 0))
}

/// Decode chunks back to back from `input` until `out_len` bytes are produced.
/// The chunk sizes must add up to the whole input.
pub fn decompress(input: &[u8], out_len: usize, limit: usize) -> Result<Vec<u8>> {
    if out_len > limit {
        return Err(Error::TooLarge(out_len as u64));
    }
    let mut out = Vec::with_capacity(out_len);
    let mut pos = 0;
    while out.len() < out_len {
        let (hl, size, dsize, compressed) = chunk_header(&input[pos..])?;
        if out.len() + dsize > out_len {
            return Err(Error::Corrupt("QuickLZ chunk overruns the stated size"));
        }
        let chunk = &input[pos..pos + size];
        if compressed {
            out.extend_from_slice(&chunk_body(chunk, hl, dsize)?);
        } else {
            if size != hl + dsize {
                return Err(Error::Corrupt("stored QuickLZ chunk size mismatch"));
            }
            out.extend_from_slice(&chunk[hl..]);
        }
        pos += size;
    }
    if pos != input.len() {
        return Err(Error::Corrupt("QuickLZ chunks do not fill the input"));
    }
    Ok(out)
}

fn chunk_body(chunk: &[u8], hl: usize, size: usize) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(size);
    if size == 0 {
        return Ok(out);
    }
    let last = size as isize - 1;
    let mut table = [0usize; 4096];
    let mut hashed: isize = -1;
    let mut cword: u32 = 1;
    let mut i = hl;
    let end = chunk.len();
    let update = |out: &[u8], table: &mut [usize; 4096], hashed: &mut isize, x: isize| {
        while *hashed < x {
            *hashed += 1;
            let h = *hashed as usize;
            let v = out[h] as u32 | (out[h + 1] as u32) << 8 | (out[h + 2] as u32) << 16;
            table[(((v >> 12) ^ v) & 0xFFF) as usize] = h;
        }
    };
    loop {
        if i > end {
            return Err(Error::Corrupt("QuickLZ input overrun"));
        }
        if cword == 1 {
            cword = read_u32(chunk, i) | 0x8000_0000;
            i += 4;
        }
        if cword & 1 != 0 {
            cword >>= 1;
            let f = read_u24(chunk, i);
            let src = table[((f >> 4) & 0xFFF) as usize];
            let len = if f & 0xF != 0 {
                i += 2;
                (f & 0xF) as usize + 2
            } else {
                i += 3;
                (f >> 16) as usize
            };
            let start = out.len();
            if len < 3 || src >= start || start + len > size || i > end {
                return Err(Error::Corrupt("bad QuickLZ match"));
            }
            for k in 0..len {
                let b = out[src + k];
                out.push(b);
            }
            update(&out, &mut table, &mut hashed, start as isize);
            hashed = out.len() as isize - 1;
        } else if (out.len() as isize) < last - 10 {
            out.push(
                *chunk
                    .get(i)
                    .ok_or(Error::Corrupt("QuickLZ input overrun"))?,
            );
            i += 1;
            cword >>= 1;
            update(&out, &mut table, &mut hashed, out.len() as isize - 3);
        } else {
            while (out.len() as isize) <= last {
                if cword == 1 {
                    i += 4;
                    cword = 0x8000_0000;
                }
                out.push(
                    *chunk
                        .get(i)
                        .ok_or(Error::Corrupt("QuickLZ input overrun"))?,
                );
                i += 1;
                cword >>= 1;
            }
            break;
        }
    }
    if i != end {
        return Err(Error::Corrupt("QuickLZ chunk length mismatch"));
    }
    Ok(out)
}

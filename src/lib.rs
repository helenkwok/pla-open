//! Reader for Archicad project files (`.pln`, `.pla`).
//!
//! The file is a header followed by 16-byte units holding blocks, compressed
//! sections and gap units. [`Pla::walk`] visits every live block without
//! loading the whole file; [`Pla::walls`] decodes walls.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

mod quicklz;
mod wall;

pub use wall::Wall;

/// Largest block, section or decoded payload we accept, in bytes.
pub const MAX_ITEM_BYTES: usize = 256 << 20;

const SIGNATURE: &[u8; 8] = b"ROF FDB ";
const SECTION_MARKER: [u8; 16] = [
    0x79, 0xa8, 0xa9, 0x96, 0x45, 0xb4, 0x4a, 0xf3, 0x98, 0x38, 0x33, 0xc3, 0x3a, 0x06, 0x1b, 0x06,
];
/// Block type of a free block: leftover bytes, never parsed.
pub const FREE_BLOCK: i16 = -36;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    NotArchicad,
    Corrupt(&'static str),
    Unsupported(&'static str),
    TooLarge(u64),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::NotArchicad => write!(f, "not an Archicad project file (bad signature)"),
            Error::Corrupt(m) => write!(f, "corrupt file: {m}"),
            Error::Unsupported(m) => write!(f, "not supported yet: {m}"),
            Error::TooLarge(n) => write!(
                f,
                "item of {n} bytes is over the {MAX_ITEM_BYTES}-byte limit"
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A GUID, kept in its stored byte order (first three fields little-endian).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Guid(pub [u8; 16]);

impl Guid {
    pub fn from_slice(b: &[u8]) -> Guid {
        let mut g = [0; 16];
        g.copy_from_slice(&b[..16]);
        Guid(g)
    }

    /// Parse the usual `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` text form.
    pub fn parse(s: &str) -> Option<Guid> {
        let hex: String = s.chars().filter(|c| *c != '-').collect();
        if hex.len() != 32 {
            return None;
        }
        let mut be = [0u8; 16];
        for (i, b) in be.iter_mut().enumerate() {
            *b = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
        }
        let mut g = be;
        g[0..4].reverse();
        g[4..6].reverse();
        g[6..8].reverse();
        Some(Guid(g))
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut be = self.0;
        be[0..4].reverse();
        be[4..6].reverse();
        be[6..8].reverse();
        for (i, b) in be.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Where a block's bytes live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    /// Directly in the file.
    File { offset: u64 },
    /// Inside the compressed section that starts at `section`, at `offset` in its decoded bytes.
    Section { section: u64, offset: usize },
}

/// Header fields of one block.
#[derive(Clone, Copy, Debug)]
pub struct BlockInfo {
    pub block_type: i16,
    pub guid: Guid,
    pub f1: u8,
    pub f2: u8,
    /// Block length in bytes, header included.
    pub len: usize,
    pub location: Location,
}

/// A block and its bytes, handed to the [`Pla::walk`] visitor.
pub struct Block<'a> {
    pub info: BlockInfo,
    /// The whole block, header included (empty for free blocks, which are not read).
    pub bytes: &'a [u8],
}

impl Block<'_> {
    /// Decode the payload (raw, zero mask or QuickLZ).
    pub fn payload(&self) -> Result<Vec<u8>> {
        decode_payload(&self.info, self.bytes)
    }
}

/// Counts from a full walk.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub blocks: usize,
    pub sections: usize,
    pub gap_units: usize,
    pub by_type: BTreeMap<i16, usize>,
}

/// An open project file.
pub struct Pla {
    file: BufReader<File>,
    size: u64,
    /// Bytes 8..16 of the header.
    pub header_words: [u8; 8],
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

fn f64_at(b: &[u8], o: usize) -> Option<f64> {
    u64_at(b, o).map(f64::from_bits)
}

fn is_block_tag(b: &[u8]) -> bool {
    b.len() >= 9 && (&b[0..2] == b"BL" || &b[0..2] == b"DY") && b[8] == 0x01
}

fn info_from_header(h: &[u8], len: usize, location: Location) -> Result<BlockInfo> {
    if h.len() < 27 {
        return Err(Error::Corrupt("block shorter than its header"));
    }
    Ok(BlockInfo {
        block_type: u16_at(h, 6).unwrap() as i16,
        guid: Guid::from_slice(&h[11..27]),
        f1: h[9],
        f2: h[10],
        len,
        location,
    })
}

fn too_large(n: u64) -> Result<usize> {
    if n > MAX_ITEM_BYTES as u64 {
        Err(Error::TooLarge(n))
    } else {
        Ok(n as usize)
    }
}

fn short() -> Error {
    Error::Corrupt("block payload runs past the block")
}

/// Decode the payload of a block whose full bytes are `b`.
pub fn decode_payload(info: &BlockInfo, b: &[u8]) -> Result<Vec<u8>> {
    if info.block_type == FREE_BLOCK {
        return Err(Error::Unsupported("free blocks have no payload"));
    }
    let mut p = 27;
    match info.f1 >> 4 {
        0xB => {
            let n = u32_at(b, p).ok_or_else(short)? as usize;
            Ok(b.get(p + 4..p + 4 + n).ok_or_else(short)?.to_vec())
        }
        0xA => {
            let a = if info.f1 & 0x0F <= 6 {
                p += 2;
                u16_at(b, p - 2).ok_or_else(short)? as usize
            } else {
                p += 4;
                u32_at(b, p - 4).ok_or_else(short)? as usize
            };
            match info.f2 & 0x0F {
                4 => Ok(b.get(p + 4..p + 4 + a).ok_or_else(short)?.to_vec()),
                5 | 6 => {
                    let s = too_large(u32_at(b, p).ok_or_else(short)? as u64)?;
                    let data = b.get(p + 8..p + 8 + a).ok_or_else(short)?;
                    if info.f2 & 0x0F == 5 {
                        zero_mask(data, s)
                    } else {
                        quicklz::decompress(data, s, MAX_ITEM_BYTES)
                    }
                }
                _ => Err(Error::Unsupported("block payload method")),
            }
        }
        _ => Err(Error::Unsupported("block family")),
    }
}

/// Zero-mask decode `input` into exactly `size` bytes, consuming all of `input`.
pub fn zero_mask(input: &[u8], size: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(size);
    let mut it = input.iter();
    while out.len() < size {
        let mask = *it
            .next()
            .ok_or(Error::Corrupt("zero-mask data ends early"))?;
        for bit in 0..8 {
            if out.len() == size {
                break;
            }
            if mask >> bit & 1 != 0 {
                out.push(
                    *it.next()
                        .ok_or(Error::Corrupt("zero-mask data ends early"))?,
                );
            } else {
                out.push(0);
            }
        }
    }
    if it.next().is_some() {
        return Err(Error::Corrupt("zero-mask data longer than needed"));
    }
    Ok(out)
}

impl Pla {
    /// Open a file and check its signature.
    pub fn open(path: impl AsRef<Path>) -> Result<Pla> {
        let f = File::open(path)?;
        let size = f.metadata()?.len();
        let mut file = BufReader::new(f);
        let mut h = [0u8; 32];
        file.read_exact(&mut h).map_err(|_| Error::NotArchicad)?;
        if &h[0..8] != SIGNATURE {
            return Err(Error::NotArchicad);
        }
        let mut header_words = [0; 8];
        header_words.copy_from_slice(&h[8..16]);
        Ok(Pla {
            file,
            size,
            header_words,
        })
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut v = vec![0; len];
        self.file.read_exact(&mut v)?;
        Ok(v)
    }

    /// Read and decompress the section at `offset`. Returns (decoded bytes, stored length).
    fn read_section(&mut self, offset: u64) -> Result<(Vec<u8>, u64)> {
        let h = self.read_at(offset, 36)?;
        let ds = too_large(u64_at(&h, 16).unwrap())?;
        let cs = u32_at(&h, 32).unwrap() as u64;
        if offset + 36 + cs > self.size {
            return Err(Error::Corrupt(
                "compressed section runs past the end of the file",
            ));
        }
        let cs = too_large(cs)?;
        let mut data = vec![0; cs];
        self.file.read_exact(&mut data)?;
        Ok((
            quicklz::decompress(&data, ds, MAX_ITEM_BYTES)?,
            36 + cs as u64,
        ))
    }

    /// Visit every live block in file order, including blocks inside
    /// compressed sections. Free blocks are visited with empty bytes.
    pub fn walk(&mut self, mut visit: impl FnMut(&Block) -> Result<()>) -> Result<Stats> {
        let mut st = Stats::default();
        let mut o: u64 = 0x20;
        self.file.seek(SeekFrom::Start(o))?;
        let mut unit = [0u8; 16];
        while o < self.size {
            if self.size - o < 16 {
                return Err(Error::Corrupt("file does not end on a 16-byte unit"));
            }
            self.file.seek(SeekFrom::Start(o))?;
            self.file.read_exact(&mut unit)?;
            let u = u32_at(&unit, 2).unwrap() as u64;
            if is_block_tag(&unit) && u > 0 && o + 16 * u <= self.size {
                let len = too_large(16 * u)?;
                let ty = u16_at(&unit, 6).unwrap() as i16;
                if ty == FREE_BLOCK {
                    let mut h = unit.to_vec();
                    h.resize(27, 0);
                    let info = info_from_header(&h, len, Location::File { offset: o })?;
                    count(&mut st, ty);
                    visit(&Block { info, bytes: &[] })?;
                } else {
                    let bytes = self.read_at(o, len)?;
                    let info = info_from_header(&bytes, len, Location::File { offset: o })?;
                    count(&mut st, ty);
                    visit(&Block {
                        info,
                        bytes: &bytes,
                    })?;
                }
                o += 16 * u;
            } else if unit == SECTION_MARKER {
                let (data, stored) = self.read_section(o)?;
                st.sections += 1;
                walk_section(&data, o, &mut st, &mut visit)?;
                o = (o + stored).div_ceil(16) * 16;
            } else {
                st.gap_units += 1;
                o += 16;
            }
        }
        Ok(st)
    }

    /// List every live block (type, GUID, place) in one pass.
    pub fn index(&mut self) -> Result<Vec<BlockInfo>> {
        let mut v = Vec::new();
        self.walk(|b| {
            v.push(b.info);
            Ok(())
        })?;
        Ok(v)
    }

    /// Read and decode the payload of a block found by [`Pla::index`].
    pub fn payload(&mut self, info: &BlockInfo) -> Result<Vec<u8>> {
        match info.location {
            Location::File { offset } => {
                let b = self.read_at(offset, info.len)?;
                decode_payload(info, &b)
            }
            Location::Section { section, offset } => {
                let (data, _) = self.read_section(section)?;
                let b = data
                    .get(offset..offset + info.len)
                    .ok_or(Error::Corrupt("block outside its section"))?;
                decode_payload(info, b)
            }
        }
    }

    /// Decode every wall in the file.
    pub fn walls(&mut self) -> Result<Vec<Wall>> {
        wall::read_walls(self)
    }
}

fn count(st: &mut Stats, ty: i16) {
    st.blocks += 1;
    *st.by_type.entry(ty).or_default() += 1;
}

fn walk_section(
    data: &[u8],
    section: u64,
    st: &mut Stats,
    visit: &mut impl FnMut(&Block) -> Result<()>,
) -> Result<()> {
    let mut o = 0;
    while o < data.len() {
        let h = &data[o..];
        let u = u32_at(h, 2).unwrap_or(0) as usize;
        if !is_block_tag(h) || u == 0 || u > (data.len() - o) / 16 {
            return Err(Error::Corrupt("bad block inside a compressed section"));
        }
        let len = 16 * u;
        let info = info_from_header(&h[..len], len, Location::Section { section, offset: o })?;
        count(st, info.block_type);
        let bytes = if info.block_type == FREE_BLOCK {
            &[][..]
        } else {
            &h[..len]
        };
        visit(&Block { info, bytes })?;
        o += len;
    }
    Ok(())
}

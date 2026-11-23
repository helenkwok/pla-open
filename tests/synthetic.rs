//! Builds small files from the documented byte layouts and reads them back.

use pla_open::{Error, Guid, Location, Pla};

const MARKER: [u8; 16] = [
    0x79, 0xa8, 0xa9, 0x96, 0x45, 0xb4, 0x4a, 0xf3, 0x98, 0x38, 0x33, 0xc3, 0x3a, 0x06, 0x1b, 0x06,
];

fn g(n: u8) -> [u8; 16] {
    let mut v = [0u8; 16];
    for (i, b) in v.iter_mut().enumerate() {
        *b = n.wrapping_mul(17).wrapping_add(i as u8);
    }
    v
}

fn pad16(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(16) {
        v.push(0);
    }
}

fn block(ty: i16, f1: u8, f2: u8, guid: [u8; 16], body: &[u8]) -> Vec<u8> {
    let mut v = b"BL".to_vec();
    v.extend([0; 4]);
    v.extend(ty.to_le_bytes());
    v.extend([1, f1, f2]);
    v.extend(guid);
    v.extend(body);
    pad16(&mut v);
    let u = (v.len() / 16) as u32;
    v[2..6].copy_from_slice(&u.to_le_bytes());
    v
}

fn raw_object(ty: i16, guid: [u8; 16], payload: &[u8]) -> Vec<u8> {
    let mut body = (payload.len() as u16).to_le_bytes().to_vec();
    body.extend(0xDEADBEEFu32.to_le_bytes());
    body.extend(payload);
    block(ty, 0xA1, 0x04, guid, &body)
}

fn zero_mask_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for group in data.chunks(8) {
        let mask = group
            .iter()
            .enumerate()
            .fold(0u8, |m, (i, b)| if *b != 0 { m | 1 << i } else { m });
        out.push(mask);
        out.extend(group.iter().filter(|b| **b != 0));
    }
    out
}

fn zero_mask_object(ty: i16, guid: [u8; 16], payload: &[u8]) -> Vec<u8> {
    let enc = zero_mask_encode(payload);
    let mut body = (enc.len() as u16).to_le_bytes().to_vec();
    body.extend((payload.len() as u32).to_le_bytes());
    body.extend(7u32.to_le_bytes());
    body.extend(&enc);
    block(ty, 0xA2, 0x05, guid, &body)
}

fn quicklz_object(ty: i16, guid: [u8; 16], chunk: &[u8], size: u32) -> Vec<u8> {
    let mut body = (chunk.len() as u32).to_le_bytes().to_vec();
    body.extend(size.to_le_bytes());
    body.extend(9u32.to_le_bytes());
    body.extend(chunk);
    block(ty, 0xA7, 0x06, guid, &body)
}

fn stored_chunk(data: &[u8]) -> Vec<u8> {
    let mut v = vec![0x46];
    v.extend(((data.len() + 9) as u32).to_le_bytes());
    v.extend((data.len() as u32).to_le_bytes());
    v.extend(data);
    v
}

/// A section holding `inner` as two stored chunks.
fn section(inner: &[u8]) -> Vec<u8> {
    let half = inner.len() / 2;
    let mut chunks = stored_chunk(&inner[..half]);
    chunks.extend(stored_chunk(&inner[half..]));
    let mut v = MARKER.to_vec();
    v.extend((inner.len() as u64).to_le_bytes());
    v.extend(1u64.to_le_bytes());
    v.extend((chunks.len() as u32).to_le_bytes());
    v.extend(chunks);
    pad16(&mut v);
    v
}

/// QuickLZ level 1 chunk: "abcdef", a match of 18 bytes, then "XYZ123" (30 bytes).
fn qlz_long_match() -> (Vec<u8>, Vec<u8>) {
    let mut c = vec![0x45, 22, 30];
    c.extend(0x40u32.to_le_bytes());
    c.extend(b"abcdef");
    c.extend([0x70, 0x45, 18]);
    c.extend(b"XYZ123");
    (c, b"abcdefabcdefabcdefabcdefXYZ123".to_vec())
}

/// Same with a short match (12 bytes, two-byte form): 24 bytes.
fn qlz_short_match() -> (Vec<u8>, Vec<u8>) {
    let mut c = vec![0x45, 21, 24];
    c.extend(0x40u32.to_le_bytes());
    c.extend(b"abcdef");
    c.extend([0x7A, 0x45]);
    c.extend(b"XYZ123");
    (c, b"abcdefabcdefabcdefXYZ123".to_vec())
}

fn links(items: &[(u8, [u8; 16], [u8; 16])]) -> Vec<u8> {
    let mut v = vec![1, 0];
    for (tag, role, target) in items {
        v.push(*tag);
        v.extend(role);
        v.extend(target);
    }
    v.push(0);
    v
}

fn object_payload(data_end: usize, fill: impl FnOnce(&mut Vec<u8>), list: &[u8]) -> Vec<u8> {
    let mut p = vec![0u8; data_end];
    p[0..16].copy_from_slice(&g(200));
    p[16] = 2;
    fill(&mut p);
    let n1 = (data_end - 39) as u32;
    p[35..39].copy_from_slice(&n1.to_le_bytes());
    p.extend((list.len() as u32).to_le_bytes());
    p.extend(list);
    p
}

struct WallIn<'a> {
    id: &'a str,
    begin: [f64; 2],
    end: [f64; 2],
    thick: [f64; 2],
    height: f64,
    bottom: f64,
    offset: f64,
}

fn wall_payload(w: &WallIn, list: &[u8]) -> Vec<u8> {
    let units: Vec<u16> = w.id.encode_utf16().collect();
    let e = 0xF4 + 2 * units.len();
    object_payload(
        e + 0x110,
        |p| {
            p[0xF0..0xF4].copy_from_slice(&(units.len() as u32).to_le_bytes());
            for (i, u) in units.iter().enumerate() {
                p[0xF4 + 2 * i..0xF6 + 2 * i].copy_from_slice(&u.to_le_bytes());
            }
            let mut put = |o: usize, v: f64| p[e + o..e + o + 8].copy_from_slice(&v.to_le_bytes());
            put(0x00, w.begin[0]);
            put(0x08, w.begin[1]);
            put(0x10, w.end[0]);
            put(0x18, w.end[1]);
            put(0x4D, w.thick[0]);
            put(0x55, w.thick[1]);
            put(0x99, w.height);
            put(0xA1, w.bottom);
            put(0xF3, w.offset);
        },
        list,
    )
}

fn story_payload(name: &str) -> Vec<u8> {
    let units: Vec<u16> = name.encode_utf16().collect();
    object_payload(
        39 + 8 + 4 + 2 * units.len() + 8,
        |p| {
            let o = 39 + 8;
            p[o..o + 4].copy_from_slice(&(units.len() as u32).to_le_bytes());
            for (i, u) in units.iter().enumerate() {
                p[o + 4 + 2 * i..o + 6 + 2 * i].copy_from_slice(&u.to_le_bytes());
            }
        },
        &links(&[]),
    )
}

const HOME_ROLE: [u8; 16] = [3; 16];
const TOP_ROLE: [u8; 16] = [4; 16];
const STORY: u8 = 1;
const TOP: u8 = 2;
const WALL_A: u8 = 3;
const WALL_B: u8 = 4;

fn build() -> Vec<u8> {
    let mut f = b"ROF FDB ".to_vec();
    f.extend([0x10, 0, 2, 0, 0, 0, 1, 1]);
    f.extend([0; 16]);

    f.extend(block(-536, 0xB1, 0x80, g(0), &{
        let mut b = 20u32.to_le_bytes().to_vec();
        b.extend(4u32.to_le_bytes());
        b.extend(g(9));
        b
    }));
    f.extend(raw_object(210, g(STORY), &story_payload("Ground Floor")));
    // Gap unit with stale bytes that almost look like a block header.
    f.extend(b"BL\x01\0\0\0\x09\x01\0stale!!");
    f.extend(raw_object(
        204,
        g(TOP),
        &object_payload(60, |_| {}, &links(&[])),
    ));
    // Free block whose leftovers contain a valid-looking header.
    let mut free = block(-36, 0x37, 0x99, g(50), &[0; 21]);
    free[32..48].copy_from_slice(&block(265, 0xA1, 4, g(51), &[])[..16]);
    f.extend(free);

    let wall_a = WallIn {
        id: "SW - 001",
        begin: [0.0, 0.0],
        end: [1.0, 0.0],
        thick: [0.3, 0.3],
        height: 0.0,
        bottom: 0.0,
        offset: 0.05,
    };
    f.extend(raw_object(
        265,
        g(WALL_A),
        &wall_payload(
            &wall_a,
            &links(&[
                (0x11, [7; 16], g(60)),
                (0x12, HOME_ROLE, g(STORY)),
                (0x12, TOP_ROLE, g(TOP)),
            ]),
        ),
    ));
    f.extend([0; 16]);

    let wall_b = WallIn {
        id: "Wall-0002x",
        begin: [1.25, 2.5],
        end: [4.75, 6.125],
        thick: [0.2, 0.25],
        height: 2.5,
        bottom: 0.5,
        offset: 0.0,
    };
    let mut inner = zero_mask_object(
        265,
        g(WALL_B),
        &wall_payload(&wall_b, &links(&[(0x12, HOME_ROLE, g(STORY))])),
    );
    inner.extend(block(-36, 0x12, 0x34, g(70), &[0xAB; 30]));
    inner.extend(quicklz_object(999, g(80), &qlz_long_match().0, 30));
    inner.extend(quicklz_object(998, g(81), &qlz_short_match().0, 24));
    f.extend(section(&inner));
    f
}

fn write(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("pla-open-{}-{name}.pln", std::process::id()));
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn walks_every_item_to_the_end() {
    let path = write("walk", &build());
    let mut pla = Pla::open(&path).unwrap();
    assert_eq!(pla.header_words, [0x10, 0, 2, 0, 0, 0, 1, 1]);
    let st = pla.walk(|_| Ok(())).unwrap();
    assert_eq!(st.sections, 1);
    assert_eq!(st.gap_units, 2);
    assert_eq!(st.blocks, 9);
    assert_eq!(st.by_type[&-36], 2);
    assert_eq!(st.by_type[&265], 2);
    assert_eq!(st.by_type.get(&-534), None);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn decodes_payloads_and_finds_blocks_by_guid() {
    let path = write("payload", &build());
    let mut pla = Pla::open(&path).unwrap();
    let index = pla.index().unwrap();
    let find = |n: u8| *index.iter().find(|b| b.guid == Guid(g(n))).unwrap();

    let long = find(80);
    assert!(matches!(long.location, Location::Section { .. }));
    assert_eq!(pla.payload(&long).unwrap(), qlz_long_match().1);
    assert_eq!(pla.payload(&find(81)).unwrap(), qlz_short_match().1);

    let root = find(0);
    assert_eq!(root.block_type, -536);
    let rp = pla.payload(&root).unwrap();
    assert_eq!(rp.len(), 20);
    assert_eq!(&rp[4..], &g(9));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn reads_walls() {
    let path = write("walls", &build());
    let walls = Pla::open(&path).unwrap().walls().unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(walls.len(), 2);

    let a = &walls[0];
    assert_eq!(a.guid, Guid(g(WALL_A)));
    assert_eq!(a.id, "SW - 001");
    assert_eq!((a.begin, a.end), ([0.0, 0.0], [1.0, 0.0]));
    assert_eq!((a.begin_thickness, a.end_thickness), (0.3, 0.3));
    assert!(a.top_linked);
    assert_eq!(a.height, None);
    assert_eq!(a.offset_from_outside, 0.05);
    assert_eq!(a.home_story, Some(Guid(g(STORY))));
    assert_eq!(a.home_story_name.as_deref(), Some("Ground Floor"));

    let b = &walls[1];
    assert_eq!(b.id, "Wall-0002x");
    assert_eq!((b.begin, b.end), ([1.25, 2.5], [4.75, 6.125]));
    assert_eq!((b.begin_thickness, b.end_thickness), (0.2, 0.25));
    assert!(!b.top_linked);
    assert_eq!(b.height, Some(2.5));
    assert_eq!(b.bottom_offset, 0.5);
    assert_eq!(b.home_story_name.as_deref(), Some("Ground Floor"));
}

#[test]
fn rejects_bad_files() {
    let p = write("sig", b"NOT A PLN FILE AT ALL, JUST TEXT");
    assert!(matches!(Pla::open(&p), Err(Error::NotArchicad)));

    // A section claiming more stored bytes than the file holds.
    let mut f = build();
    let at = f.windows(16).position(|w| w == MARKER).unwrap();
    f[at + 32..at + 36].copy_from_slice(&u32::MAX.to_le_bytes());
    std::fs::write(&p, &f).unwrap();
    assert!(Pla::open(&p).unwrap().walk(|_| Ok(())).is_err());

    // A file that stops mid-unit.
    let mut f = build();
    f.extend([0; 5]);
    std::fs::write(&p, &f).unwrap();
    assert!(Pla::open(&p).unwrap().walk(|_| Ok(())).is_err());
    std::fs::remove_file(p).unwrap();
}

#[test]
fn guid_text_round_trip() {
    let s = "4f654582-33af-4e52-abfc-c35d46765ec4";
    let guid = Guid::parse(s).unwrap();
    assert_eq!(&guid.0[..4], &[0x82, 0x45, 0x65, 0x4f]);
    assert_eq!(guid.to_string(), s);
    assert_eq!(Guid::parse(&s.to_uppercase()), Some(guid));
}

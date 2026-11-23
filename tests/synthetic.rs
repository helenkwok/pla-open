//! Builds small files from the documented byte layouts and reads them back.

use pla_open::{Error, Geometry, Guid, Location, Pla, Profile, ReferenceLine, Structure};

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
    let chunks = stored_chunk(inner);
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
    top_offset: f64,
    ref_offset: f64,
    arc: f64,
    geometry: u8,
    profile: u16,
    slant: f64,
    structure: u16,
    composite: i32,
    flipped: u8,
    ref_line: u32,
}

const DEFAULT_WALL: WallIn = WallIn {
    id: "SW - 001",
    begin: [0.0, 0.0],
    end: [1.0, 0.0],
    thick: [0.3, 0.3],
    height: 0.0,
    bottom: 0.0,
    offset: 0.05,
    top_offset: 0.0,
    ref_offset: 0.0,
    arc: 0.0,
    geometry: 0,
    profile: 0,
    // Value seen in the slant field of a wall with a normal profile; must be ignored.
    slant: 0.8727,
    structure: 2,
    composite: 23,
    flipped: 1,
    ref_line: 3,
};

fn put_text(p: &mut [u8], o: usize, s: &str) -> usize {
    let units: Vec<u16> = s.encode_utf16().collect();
    p[o..o + 4].copy_from_slice(&(units.len() as u32).to_le_bytes());
    for (i, u) in units.iter().enumerate() {
        p[o + 4 + 2 * i..o + 6 + 2 * i].copy_from_slice(&u.to_le_bytes());
    }
    o + 4 + 2 * units.len()
}

fn text_len(s: &str) -> usize {
    4 + 2 * s.encode_utf16().count()
}

fn wall_payload(w: &WallIn, list: &[u8]) -> Vec<u8> {
    let e = 0xF0 + text_len(w.id);
    object_payload(
        e + 0x110,
        |p| {
            put_text(p, 0xF0, w.id);
            let mut put = |o: usize, v: f64| p[e + o..e + o + 8].copy_from_slice(&v.to_le_bytes());
            put(0x00, w.begin[0]);
            put(0x08, w.begin[1]);
            put(0x10, w.end[0]);
            put(0x18, w.end[1]);
            put(0x4D, w.thick[0]);
            put(0x55, w.thick[1]);
            put(0x5D, w.ref_offset);
            put(0x75, w.arc);
            put(0x91, w.top_offset);
            put(0x99, w.height);
            put(0xA1, w.bottom);
            put(0xBA, w.slant);
            put(0xF3, w.offset);
            p[e + 0x7E] = w.geometry;
            p[e + 0xB8..e + 0xBA].copy_from_slice(&w.profile.to_le_bytes());
            p[e + 0xE1..e + 0xE3].copy_from_slice(&w.structure.to_le_bytes());
            p[e + 0xE3] = 1;
            p[e + 0xE5..e + 0xE9].copy_from_slice(&w.composite.to_le_bytes());
            p[e + 0xEE] = w.flipped;
            p[e + 0xEF..e + 0xF3].copy_from_slice(&w.ref_line.to_le_bytes());
        },
        list,
    )
}

/// Story object (type 210): constant prefix, name at payload offset 49, then the index.
fn story_payload(index: i16, name: &str, list: &[u8]) -> Vec<u8> {
    object_payload(
        49 + text_len(name) + 5,
        |p| {
            p[39..49].copy_from_slice(&[5, 0, 0x83, 0, 3, 0, 0, 0, 1, 0]);
            let o = put_text(p, 49, name);
            p[o..o + 2].copy_from_slice(&index.to_le_bytes());
            p[o + 2] = 1;
        },
        list,
    )
}

/// Story table (type 224) with records (index, level, name).
fn story_table_payload(records: &[(i16, f64, &str)]) -> Vec<u8> {
    let mut t = vec![1, 0, 3, 0, 0, 1, 0, records.len() as u8];
    for (i, (index, level, name)) in records.iter().enumerate() {
        t.extend(index.to_le_bytes());
        t.push(u8::from(i > 0));
        t.extend(2u16.to_le_bytes());
        t.extend(level.to_le_bytes());
        t.extend(1u16.to_le_bytes());
        let mut text = vec![0; text_len(name)];
        put_text(&mut text, 0, name);
        t.extend(text);
    }
    t.extend([0xe8, 0x86, 1, 0, 0, 2, 0, 0, 0]);
    let len = 39 + t.len();
    object_payload(len, |p| p[39..].copy_from_slice(&t), &links(&[]))
}

/// Composite attribute (type 137): name at payload offset 55, u16 1, then the index.
fn composite_payload(index: i32, name: &str) -> Vec<u8> {
    object_payload(
        55 + text_len(name) + 6,
        |p| {
            let o = put_text(p, 55, name);
            p[o] = 1;
            p[o + 2..o + 6].copy_from_slice(&index.to_le_bytes());
        },
        &links(&[]),
    )
}

fn role(s: &str) -> [u8; 16] {
    Guid::parse(s).unwrap().0
}

fn home_role() -> [u8; 16] {
    role("79b61203-ae84-4d88-a9c6-20ead705e0bc")
}
fn top_role() -> [u8; 16] {
    role("7b48cc47-7267-4cfd-9133-9214ea18fb99")
}
fn anchor_role() -> [u8; 16] {
    role("baf95b80-f256-4932-b891-03ff66848591")
}

/// Top anchor (type 204) owning `walls` (one `0x01` item) and pointing up to `story`.
fn anchor_payload(story: u8, walls: &[u8]) -> Vec<u8> {
    let mut list = vec![1, 0, 0x01];
    list.extend(top_role());
    list.extend((walls.len() as u32).to_le_bytes());
    for w in walls {
        list.extend(g(*w));
    }
    list.push(0x12);
    list.extend(anchor_role());
    list.extend(g(story));
    list.push(0);
    object_payload(60, |_| {}, &list)
}

const STORY: u8 = 1;
const TOP: u8 = 2;
const WALL_A: u8 = 3;
const WALL_B: u8 = 4;
const STORY_1: u8 = 5;
const STORY_2: u8 = 6;
const TOP_2: u8 = 7;
const WALL_C: u8 = 8;
const TABLE: u8 = 10;
const COMPOSITE: u8 = 11;

fn header() -> Vec<u8> {
    let mut f = b"ROF FDB ".to_vec();
    f.extend([0x10, 0, 2, 0, 0, 0, 1, 1]);
    f.extend([0; 16]);
    f.extend(block(-536, 0xB1, 0x80, g(0), &{
        let mut b = 20u32.to_le_bytes().to_vec();
        b.extend(4u32.to_le_bytes());
        b.extend(g(9));
        b
    }));
    f
}

fn build() -> Vec<u8> {
    let mut f = header();
    f.extend(raw_object(
        210,
        g(STORY),
        &story_payload(0, "Ground Floor", &links(&[(0x11, home_role(), g(WALL_A))])),
    ));
    // Gap unit with stale bytes that almost look like a block header.
    f.extend(b"BL\x01\0\0\0\x09\x01\0stale!!");
    // A one-unit block, too short for a full header.
    f.extend(b"BL\x01\0\0\0\x05\0\x01\xA1\x04short");
    f.extend(raw_object(204, g(TOP), &anchor_payload(STORY_1, &[WALL_A])));
    // Free block whose leftovers contain a valid-looking header.
    let mut free = block(-36, 0x37, 0x99, g(50), &[0; 21]);
    free[32..48].copy_from_slice(&block(265, 0xA1, 4, g(51), &[])[..16]);
    f.extend(free);

    let wall_a = WallIn {
        bottom: 0.1,
        top_offset: 0.4,
        ref_offset: 0.1,
        ..DEFAULT_WALL
    };
    f.extend(raw_object(
        265,
        g(WALL_A),
        &wall_payload(
            &wall_a,
            &links(&[
                (0x11, [7; 16], g(60)),
                (0x12, home_role(), g(STORY)),
                (0x12, top_role(), g(TOP)),
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
        // Stored but meaningless: the top is not linked.
        top_offset: 0.7,
        offset: 0.1,
        arc: 0.5,
        geometry: 1,
        profile: 1,
        slant: 1.3,
        structure: 1,
        flipped: 0,
        ref_line: 1,
        ..DEFAULT_WALL
    };
    let mut inner = zero_mask_object(
        265,
        g(WALL_B),
        &wall_payload(&wall_b, &links(&[(0x12, home_role(), g(STORY))])),
    );
    inner.extend(block(-36, 0x12, 0x34, g(70), &[0xAB; 30]));
    inner.extend(quicklz_object(999, g(80), &qlz_long_match().0, 30));
    inner.extend(quicklz_object(998, g(81), &qlz_short_match().0, 24));
    f.extend(section(&inner));

    // Stories 1 and 2 (the virtual one), the table, a composite, and a wall on story 1
    // whose top is linked to story 2.
    f.extend(raw_object(
        210,
        g(STORY_1),
        &story_payload(1, "", &links(&[])),
    ));
    f.extend(raw_object(
        210,
        g(STORY_2),
        &story_payload(2, "Top", &links(&[])),
    ));
    f.extend(raw_object(
        204,
        g(TOP_2),
        &anchor_payload(STORY_2, &[WALL_C]),
    ));
    f.extend(raw_object(
        224,
        g(TABLE),
        &story_table_payload(&[(0, 0.0, "Ground Floor"), (1, 3.5, ""), (2, 6.5, "Top")]),
    ));
    f.extend(raw_object(
        137,
        g(COMPOSITE),
        &composite_payload(23, "Generic Wall/Shell"),
    ));
    let wall_c = WallIn {
        id: "",
        top_offset: -0.25,
        ref_line: 5,
        ..DEFAULT_WALL
    };
    f.extend(raw_object(
        265,
        g(WALL_C),
        &wall_payload(
            &wall_c,
            &links(&[
                (0x12, home_role(), g(STORY_1)),
                (0x12, top_role(), g(TOP_2)),
            ]),
        ),
    ));
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
    assert_eq!(st.blocks, 16);
    assert_eq!(st.short_blocks, 1);
    assert_eq!(st.by_type[&-36], 2);
    assert_eq!(st.by_type[&265], 3);
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
fn reads_stories() {
    let path = write("stories", &build());
    let stories = Pla::open(&path).unwrap().stories().unwrap();
    std::fs::remove_file(path).unwrap();
    let got: Vec<_> = stories
        .iter()
        .map(|s| (s.index, s.level, s.height, s.name.as_str(), s.is_virtual))
        .collect();
    assert_eq!(
        got,
        [
            (0, 0.0, Some(3.5), "Ground Floor", false),
            (1, 3.5, Some(3.0), "", false),
            (2, 6.5, None, "Top", true),
        ]
    );
}

#[test]
fn reads_walls() {
    let path = write("walls", &build());
    let walls = Pla::open(&path).unwrap().walls().unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(walls.len(), 3);

    // Composite, top linked to story +1 with a top offset.
    let a = &walls[0];
    assert_eq!(a.guid, Guid(g(WALL_A)));
    assert_eq!(a.id, "SW - 001");
    assert_eq!((a.begin, a.end), ([0.0, 0.0], [1.0, 0.0]));
    assert_eq!((a.begin_thickness, a.end_thickness), (0.3, 0.3));
    assert!(a.top_linked);
    assert_eq!(a.home_story, Some(Guid(g(STORY))));
    assert_eq!(a.home_story_index, Some(0));
    assert_eq!(a.home_story_name.as_deref(), Some("Ground Floor"));
    assert_eq!(a.top_story_index, Some(1));
    assert_eq!(a.relative_top_story, Some(1));
    assert_eq!(a.top_offset, Some(0.4));
    assert!(a.notes.is_empty());
    assert_eq!(a.bottom_offset, 0.1);
    // 3.5 + 0.4 - 0 - 0.1
    assert!((a.height.unwrap() - 3.8).abs() < 1e-12);
    assert_eq!(a.reference_line, ReferenceLine::CoreOutside);
    assert_eq!(a.reference_line_offset, 0.1);
    assert_eq!(a.offset_from_outside, 0.05);
    assert!(a.flipped);
    assert_eq!(a.structure, Structure::Composite);
    assert_eq!(a.composite_index, Some(23));
    assert_eq!(a.composite_name.as_deref(), Some("Generic Wall/Shell"));
    assert_eq!(a.composite_guid, Some(Guid(g(COMPOSITE))));
    assert_eq!(a.geometry, Geometry::Straight);
    assert_eq!(a.arc_angle, 0.0);
    assert_eq!(a.profile, Profile::Normal);
    assert_eq!(a.slant_alpha, None);

    // Basic, top not linked, curved, trapezoid flag, slanted, not flipped.
    let b = &walls[1];
    assert_eq!(b.id, "Wall-0002x");
    assert_eq!((b.begin, b.end), ([1.25, 2.5], [4.75, 6.125]));
    assert_eq!((b.begin_thickness, b.end_thickness), (0.2, 0.25));
    assert!(!b.top_linked);
    assert_eq!(b.height, Some(2.5));
    assert_eq!(b.bottom_offset, 0.5);
    assert_eq!(b.relative_top_story, Some(0));
    assert_eq!(b.top_offset, None);
    assert!(b.notes.is_empty());
    assert_eq!(b.top_story_index, None);
    assert_eq!(b.home_story_name.as_deref(), Some("Ground Floor"));
    assert_eq!(b.reference_line, ReferenceLine::Center);
    assert_eq!(b.offset_from_outside, 0.1);
    assert!(!b.flipped);
    assert_eq!(b.structure, Structure::Basic);
    // The composite index stays in the record of a basic wall; it is not reported.
    assert_eq!(b.composite_index, None);
    assert_eq!(b.composite_name, None);
    assert_eq!(b.geometry, Geometry::Trapezoid);
    assert_eq!(b.arc_angle, 0.5);
    assert_eq!(b.profile, Profile::Slanted);
    assert_eq!(b.slant_alpha, Some(1.3));

    // Empty element ID, home story 1, top linked to story 2 (owned through a 0x01 link item).
    let c = &walls[2];
    assert_eq!(c.id, "");
    assert_eq!(c.home_story_index, Some(1));
    assert_eq!(c.home_story_name.as_deref(), Some(""));
    assert_eq!(c.top_story_index, Some(2));
    assert_eq!(c.relative_top_story, Some(1));
    // 6.5 - 0.25 - 3.5 - 0
    assert!((c.height.unwrap() - 2.75).abs() < 1e-12);
    assert_eq!(c.reference_line, ReferenceLine::CoreInside);
    assert!(c.notes.is_empty());
    assert_eq!((c.begin, c.end), ([0.0, 0.0], [1.0, 0.0]));
}

#[test]
fn element_id_shifts_every_field() {
    for id in [
        "",
        "ABCDEFGHI",
        "LONG-ELEMENT-ID-0123456789",
        "W\u{e4}nd-\u{3a9}",
    ] {
        let mut f = header();
        let w = WallIn {
            id,
            begin: [0.5, 0.25],
            end: [1.5, 0.25],
            bottom: 0.2,
            height: 3.0,
            ref_line: 4,
            ..DEFAULT_WALL
        };
        f.extend(raw_object(265, g(WALL_A), &wall_payload(&w, &links(&[]))));
        let path = write("ids", &f);
        let walls = Pla::open(&path).unwrap().walls().unwrap();
        std::fs::remove_file(path).unwrap();
        let w = &walls[0];
        assert_eq!(w.id, id);
        assert_eq!((w.begin, w.end), ([0.5, 0.25], [1.5, 0.25]));
        assert_eq!((w.bottom_offset, w.height), (0.2, Some(3.0)));
        assert_eq!(w.reference_line, ReferenceLine::CoreCenter);
        assert_eq!(w.composite_index, Some(23));
        // No story links and no table: the home story is unknown.
        assert_eq!(w.home_story_index, None);
    }
}

#[test]
fn deleted_wall_guid_left_behind_is_not_a_wall() {
    const GONE: u8 = 90;
    const GONE_STORY: u8 = 91;
    let gone = WallIn {
        id: "GONE",
        ..DEFAULT_WALL
    };
    let gone_block = raw_object(265, g(GONE), &wall_payload(&gone, &links(&[])));

    let mut f = header();
    // An index table whose slot for the deleted wall is now empty (type -34).
    f.extend(block(-534, 0xB1, 0x80, g(20), &{
        let mut slot = (-34i16).to_le_bytes().to_vec();
        slot.extend([0; 12]);
        slot.extend(g(GONE));
        let mut b = ((24 + slot.len()) as u32).to_le_bytes().to_vec();
        b.extend([0; 24]);
        b.extend(slot);
        b
    }));
    // A live type-347 block with the deleted wall's bytes after the end of its payload.
    let mut stale = raw_object(347, g(21), &object_payload(60, |_| {}, &links(&[])));
    stale.extend(&gone_block);
    let u = (stale.len() / 16) as u32;
    stale[2..6].copy_from_slice(&u.to_le_bytes());
    f.extend(stale);
    // A free block holding the old wall block and a story object, at top level and in a section.
    let mut leftovers = gone_block.clone();
    leftovers.extend(raw_object(
        210,
        g(GONE_STORY),
        &story_payload(7, "Old", &links(&[])),
    ));
    f.extend(block(-36, 0xA1, 0x04, g(GONE), &leftovers));
    let mut inner = block(-36, 0xA1, 0x04, g(GONE), &leftovers);
    // The remaining wall still points at the story that only exists in a free block.
    inner.extend(raw_object(
        265,
        g(WALL_B),
        &wall_payload(&DEFAULT_WALL, &links(&[(0x12, home_role(), g(GONE_STORY))])),
    ));
    f.extend(section(&inner));

    let path = write("deleted", &f);
    let mut pla = Pla::open(&path).unwrap();
    let index = pla.index().unwrap();
    let walls = pla.walls().unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(!index
        .iter()
        .any(|b| b.guid == Guid(g(GONE)) && b.block_type != -36));
    assert!(!index.iter().any(|b| b.guid == Guid(g(GONE_STORY))));
    assert_eq!(walls.len(), 1);
    assert_eq!(walls[0].guid, Guid(g(WALL_B)));
    assert_eq!(walls[0].home_story, None);
    assert_eq!(walls[0].home_story_name, None);
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

/// Stories -1 (basement), 0 and 1 (virtual), with walls that break documented rules.
fn odd_file() -> Vec<u8> {
    const BASEMENT: u8 = 20;
    const GROUND: u8 = 21;
    const VIRTUAL: u8 = 22;
    const TOP_GROUND: u8 = 23;
    const TOP_BASEMENT: u8 = 24;
    const DUP_A: u8 = 25;
    const DUP_B: u8 = 26;
    let mut f = header();
    for (guid, index, name) in [
        (BASEMENT, -1, "Basement"),
        (GROUND, 0, "Ground"),
        (VIRTUAL, 1, ""),
    ] {
        f.extend(raw_object(
            210,
            g(guid),
            &story_payload(index, name, &links(&[])),
        ));
    }
    f.extend(raw_object(
        204,
        g(TOP_GROUND),
        &anchor_payload(GROUND, &[30]),
    ));
    f.extend(raw_object(
        204,
        g(TOP_BASEMENT),
        &anchor_payload(BASEMENT, &[31]),
    ));
    f.extend(raw_object(
        224,
        g(TABLE),
        &story_table_payload(&[(-1, -3.0, "Basement"), (0, 0.0, "Ground"), (1, 3.0, "")]),
    ));
    f.extend(raw_object(137, g(DUP_A), &composite_payload(23, "A")));
    f.extend(raw_object(137, g(DUP_B), &composite_payload(23, "B")));
    // 30: on the basement, top linked to the ground story; second home link.
    f.extend(raw_object(
        265,
        g(30),
        &wall_payload(
            &WallIn {
                top_offset: 0.5,
                ..DEFAULT_WALL
            },
            &links(&[
                (0x12, home_role(), g(BASEMENT)),
                (0x12, home_role(), g(GROUND)),
                (0x12, top_role(), g(TOP_GROUND)),
            ]),
        ),
    ));
    // 31: on the ground story, top linked to the basement (negative relative top story),
    // invalid reference line code, flipped byte 2.
    f.extend(raw_object(
        265,
        g(31),
        &wall_payload(
            &WallIn {
                ref_line: 7,
                flipped: 2,
                ..DEFAULT_WALL
            },
            &links(&[
                (0x12, home_role(), g(GROUND)),
                (0x12, top_role(), g(TOP_BASEMENT)),
            ]),
        ),
    ));
    f
}

#[test]
fn negative_story_indices() {
    let path = write("negative-stories", &odd_file());
    let mut pla = Pla::open(&path).unwrap();
    let stories = pla.stories().unwrap();
    let walls = pla.walls().unwrap();
    std::fs::remove_file(path).unwrap();
    let got: Vec<_> = stories
        .iter()
        .map(|s| (s.index, s.level, s.height, s.is_virtual))
        .collect();
    assert_eq!(
        got,
        [
            (-1, -3.0, Some(3.0), false),
            (0, 0.0, Some(3.0), false),
            (1, 3.0, None, true),
        ]
    );
    let w = &walls[0];
    assert_eq!(w.home_story_index, Some(-1));
    assert_eq!(w.home_story_name.as_deref(), Some("Basement"));
    assert_eq!(w.top_story_index, Some(0));
    assert_eq!(w.relative_top_story, Some(1));
    // 0 + 0.5 - (-3) - 0
    assert!((w.height.unwrap() - 3.5).abs() < 1e-12);
    // The second home link is noted; the first one wins. (The file's two
    // composites share an index, which every composite wall notes too.)
    assert_eq!(
        w.notes,
        [
            "more than one home story link; the first is used",
            "2 composites share index 23; none is used"
        ]
    );
}

#[test]
fn rule_breaks_are_noted_not_fatal() {
    let path = write("rule-breaks", &odd_file());
    let walls = Pla::open(&path).unwrap().walls().unwrap();
    std::fs::remove_file(path).unwrap();
    let w = &walls[1];
    assert_eq!(w.relative_top_story, Some(-1));
    assert_eq!(w.reference_line, ReferenceLine::Other(7));
    assert_eq!(w.reference_line.to_string(), "7");
    assert!(w.flipped);
    // Two composites with index 23: neither is picked.
    assert_eq!(w.composite_index, Some(23));
    assert_eq!((w.composite_name.as_ref(), w.composite_guid), (None, None));
    assert_eq!(
        w.notes,
        [
            "relative top story -1 is invalid (the top is below the home story)",
            "2 composites share index 23; none is used",
            "reference line code 7 is invalid (0 to 5)",
            "flipped byte is 2, read as flipped",
        ]
    );
}

#[test]
fn second_story_table_is_not_supported() {
    let mut f = odd_file();
    f.extend(raw_object(
        224,
        g(99),
        &story_table_payload(&[(0, 0.0, "")]),
    ));
    let path = write("two-tables", &f);
    let e = Pla::open(&path).unwrap().stories().unwrap_err();
    std::fs::remove_file(path).unwrap();
    assert!(e.to_string().contains("hotlink modules"), "{e}");
}

#[test]
fn geometry_codes() {
    assert_eq!(Geometry::from(2), Geometry::Polygonal);
    assert_eq!(Geometry::from(9).to_string(), "9");
}

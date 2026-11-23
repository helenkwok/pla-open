//! Walls (object type 265).

use std::collections::HashMap;

use crate::{f64_at, u32_at, Error, Guid, Pla, Result};

pub const WALL_TYPE: i16 = 265;
const STORY_TYPE: i16 = 210;
const TOP_LINK_TYPE: i16 = 204;
const TAG_OWNED_BY: u8 = 0x12;

/// One wall. Lengths in metres.
#[derive(Clone, Debug, PartialEq)]
pub struct Wall {
    pub guid: Guid,
    /// Element ID as shown in Archicad (for example `SW - 001`).
    pub id: String,
    pub begin: [f64; 2],
    pub end: [f64; 2],
    pub begin_thickness: f64,
    pub end_thickness: f64,
    /// Height, known only when the top is not linked to a story.
    pub height: Option<f64>,
    pub bottom_offset: f64,
    /// Offset of the reference line from the outside face (composite walls), else 0.
    pub offset_from_outside: f64,
    /// True when the wall top is linked to a story.
    pub top_linked: bool,
    pub home_story: Option<Guid>,
    pub home_story_name: Option<String>,
}

/// Parts of an object payload: (type-specific data, link list bytes).
fn split_object(p: &[u8]) -> Result<(&[u8], &[u8])> {
    let bad = || Error::Corrupt("object payload shorter than its stated parts");
    let n1 = u32_at(p, 35).ok_or_else(bad)? as usize;
    let data = p.get(39..39 + n1).ok_or_else(bad)?;
    let n2 = u32_at(p, 39 + n1).ok_or_else(bad)? as usize;
    let links = p.get(43 + n1..43 + n1 + n2).ok_or_else(bad)?;
    Ok((data, links))
}

/// Link list items `(tag, role, target)`, stopping at the end mark or an unknown tag.
pub fn links(list: &[u8]) -> Vec<(u8, Guid, Guid)> {
    let mut v = Vec::new();
    if list.get(0..2) != Some(&[1, 0][..]) {
        return v;
    }
    let mut i = 2;
    while let Some(item) = list.get(i..i + 33) {
        if !matches!(item[0], 0x11 | 0x12) {
            break;
        }
        v.push((
            item[0],
            Guid::from_slice(&item[1..17]),
            Guid::from_slice(&item[17..33]),
        ));
        i += 33;
    }
    v
}

fn utf16(b: &[u8]) -> Option<String> {
    let units: Vec<u16> = b
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

/// Story name: the first length-prefixed UTF-16LE text in the story's data.
// ponytail: scans for the first plausible string; replace with a fixed offset once one is confirmed.
fn story_name(data: &[u8]) -> Option<String> {
    (0..data.len().saturating_sub(4)).find_map(|o| {
        let n = u32_at(data, o)? as usize;
        if !(2..=256).contains(&n) {
            return None;
        }
        let s = utf16(data.get(o + 4..o + 4 + 2 * n)?)?;
        let plausible = s
            .chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || c.is_ascii_punctuation());
        (plausible && s.chars().any(char::is_alphabetic)).then_some(s)
    })
}

fn decode_wall(
    guid: Guid,
    p: &[u8],
    types: &HashMap<Guid, i16>,
    stories: &HashMap<Guid, Vec<u8>>,
) -> Result<Wall> {
    let (data, list) = split_object(p)?;
    let bad = || Error::Corrupt("wall record shorter than expected");
    let n = u32_at(p, 0xF0).ok_or_else(bad)? as usize;
    if n > 1024 {
        return Err(Error::Corrupt("wall element ID length out of range"));
    }
    let e = 0xF4 + 2 * n;
    // Every field must sit inside the type-specific data.
    if e + 0xFB > 39 + data.len() {
        return Err(bad());
    }
    let f = |o: usize| f64_at(p, e + o).unwrap();
    let id = utf16(&p[0xF4..e]).ok_or(Error::Corrupt("wall element ID is not UTF-16"))?;

    let mut home_story = None;
    let mut top_linked = false;
    for (tag, _role, target) in links(list) {
        if tag != TAG_OWNED_BY {
            continue;
        }
        match types.get(&target) {
            Some(&STORY_TYPE) => home_story = home_story.or(Some(target)),
            Some(&TOP_LINK_TYPE) => top_linked = true,
            _ => {}
        }
    }
    let home_story_name = home_story
        .and_then(|g| stories.get(&g))
        .and_then(|sp| split_object(sp).ok())
        .and_then(|(d, _)| story_name(d));

    Ok(Wall {
        guid,
        id,
        begin: [f(0x00), f(0x08)],
        end: [f(0x10), f(0x18)],
        begin_thickness: f(0x4D),
        end_thickness: f(0x55),
        height: (!top_linked).then(|| f(0x99)),
        bottom_offset: f(0xA1),
        offset_from_outside: f(0xF3),
        top_linked,
        home_story,
        home_story_name,
    })
}

pub(crate) fn read_walls(pla: &mut Pla) -> Result<Vec<Wall>> {
    let mut types = HashMap::new();
    let mut walls = Vec::new();
    let mut stories = HashMap::new();
    pla.walk(|b| {
        if b.info.block_type != crate::FREE_BLOCK {
            types.insert(b.info.guid, b.info.block_type);
        }
        match b.info.block_type {
            WALL_TYPE => walls.push((b.info.guid, b.payload()?)),
            STORY_TYPE => {
                stories.insert(b.info.guid, b.payload()?);
            }
            _ => {}
        }
        Ok(())
    })?;
    walls
        .iter()
        .map(|(g, p)| decode_wall(*g, p, &types, &stories))
        .collect()
}

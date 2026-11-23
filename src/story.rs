//! Stories: the story table (object type 224) and story objects (type 210).

use crate::{counted_utf16, f64_at, split_object, u16_at, Error, Result};

/// Story indices are signed: stories below ground have negative numbers.
fn i16_at(b: &[u8], o: usize) -> Option<i16> {
    u16_at(b, o).map(|v| v as i16)
}

pub const STORY_TABLE_TYPE: i16 = 224;
pub const STORY_TYPE: i16 = 210;

/// One story. Levels and heights in metres.
#[derive(Clone, Debug, PartialEq)]
pub struct Story {
    /// Signed: stories below ground have negative indices, and the first need not be 0.
    pub index: i16,
    pub level: f64,
    /// Level of the next story minus this level; unknown for the last entry.
    pub height: Option<f64>,
    /// Empty when the story has no name.
    pub name: String,
    /// True for the last entry (by position, not index): the story above the top one, which only gives
    /// the top story its height.
    pub is_virtual: bool,
}

/// Decode a story table payload.
pub(crate) fn parse_table(p: &[u8]) -> Result<Vec<Story>> {
    let (t, _) = split_object(p)?;
    let bad = || Error::Corrupt("story table shorter than its records");
    let count = *t.get(7).ok_or_else(bad)?;
    let mut v: Vec<Story> = Vec::with_capacity(count.into());
    let mut o = 8;
    for _ in 0..count {
        let index = i16_at(t, o).ok_or_else(bad)?;
        let level = f64_at(t, o + 5).ok_or_else(bad)?;
        let (name, end) = counted_utf16(t, o + 15).ok_or_else(bad)?;
        v.push(Story {
            index,
            level,
            height: None,
            name,
            is_virtual: false,
        });
        o = end;
    }
    for i in 1..v.len() {
        v[i - 1].height = Some(v[i].level - v[i - 1].level);
    }
    if let Some(last) = v.last_mut() {
        last.is_virtual = true;
    }
    Ok(v)
}

/// Decode a story object payload: (story index, name).
pub(crate) fn parse_story_object(p: &[u8]) -> Result<(i16, String)> {
    let bad = || Error::Corrupt("story object shorter than expected");
    let (data, _) = split_object(p)?;
    let (name, end) = counted_utf16(p, 49).ok_or_else(bad)?;
    if end + 2 > 39 + data.len() {
        return Err(bad());
    }
    Ok((i16_at(p, end).ok_or_else(bad)?, name))
}

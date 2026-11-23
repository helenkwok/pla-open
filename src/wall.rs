//! Walls (object type 265), with the objects they point to: story objects
//! (210), top anchors (204), composites (137) and the story table (224).

use std::collections::HashMap;
use std::fmt;

use crate::story::{self, Story, STORY_TABLE_TYPE, STORY_TYPE};
use crate::{
    counted_utf16, f64_at, i32_at, links, split_object, u16_at, u32_at, utf16, Error, Guid, Pla,
    Result, TAG_OWNED_BY,
};

pub const WALL_TYPE: i16 = 265;
const TOP_ANCHOR_TYPE: i16 = 204;
const COMPOSITE_TYPE: i16 = 137;

/// Role of a wall's link to its home story.
const HOME_STORY_ROLE: &str = "79b61203-ae84-4d88-a9c6-20ead705e0bc";
/// Role of a wall's link to the top anchor of the story its top is linked to.
const TOP_LINK_ROLE: &str = "7b48cc47-7267-4cfd-9133-9214ea18fb99";
/// Role of a top anchor's link to its story.
const ANCHOR_STORY_ROLE: &str = "baf95b80-f256-4932-b891-03ff66848591";

/// Position of the reference line across the wall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceLine {
    Outside,
    Center,
    Inside,
    CoreOutside,
    CoreCenter,
    CoreInside,
    Other(u32),
}

impl From<u32> for ReferenceLine {
    fn from(n: u32) -> Self {
        use ReferenceLine::*;
        match n {
            0 => Outside,
            1 => Center,
            2 => Inside,
            3 => CoreOutside,
            4 => CoreCenter,
            5 => CoreInside,
            n => Other(n),
        }
    }
}

/// Basic (one material) or composite (layered) wall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Structure {
    Basic,
    Composite,
    Other(u16),
}

impl From<u16> for Structure {
    fn from(n: u16) -> Self {
        match n {
            1 => Structure::Basic,
            2 => Structure::Composite,
            n => Structure::Other(n),
        }
    }
}

/// Plan shape. A curved wall is `Straight` with a non-zero arc angle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Geometry {
    Straight,
    Trapezoid,
    Other(u8),
}

impl From<u8> for Geometry {
    fn from(n: u8) -> Self {
        match n {
            0 => Geometry::Straight,
            1 => Geometry::Trapezoid,
            n => Geometry::Other(n),
        }
    }
}

/// Section profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Normal,
    Slanted,
    Other(u16),
}

impl From<u16> for Profile {
    fn from(n: u16) -> Self {
        match n {
            0 => Profile::Normal,
            1 => Profile::Slanted,
            n => Profile::Other(n),
        }
    }
}

macro_rules! display_variants {
    ($($t:ty),*) => {$(
        /// Variant name, or the stored number for an unknown value.
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    Self::Other(n) => write!(f, "{n}"),
                    v => write!(f, "{v:?}"),
                }
            }
        }
    )*};
}
display_variants!(ReferenceLine, Structure, Geometry, Profile);

/// One wall. Lengths in metres, angles in radians.
#[derive(Clone, Debug, PartialEq)]
pub struct Wall {
    pub guid: Guid,
    /// Element ID as shown in Archicad (for example `SW - 001`).
    pub id: String,
    pub begin: [f64; 2],
    pub end: [f64; 2],
    pub begin_thickness: f64,
    pub end_thickness: f64,
    /// Height. For a wall whose top is linked to a story it is worked out from
    /// the story levels: top level + top offset - home level - bottom offset.
    pub height: Option<f64>,
    pub bottom_offset: f64,
    /// Offset of the top from the story the top is linked to (0 when not linked).
    pub top_offset: f64,
    /// True when the wall top is linked to a story.
    pub top_linked: bool,
    pub home_story: Option<Guid>,
    pub home_story_index: Option<u16>,
    pub home_story_name: Option<String>,
    /// Index of the story the top is linked to.
    pub top_story_index: Option<u16>,
    /// Top story index minus home story index; 0 when the top is not linked.
    pub relative_top_story: Option<i32>,
    pub reference_line: ReferenceLine,
    /// Offset of the wall from its reference line.
    pub reference_line_offset: f64,
    /// Distance of the reference line from the outside face.
    pub offset_from_outside: f64,
    pub flipped: bool,
    pub structure: Structure,
    /// Composite attribute index, name and GUID (composite walls only).
    pub composite_index: Option<i32>,
    pub composite_name: Option<String>,
    pub composite_guid: Option<Guid>,
    pub geometry: Geometry,
    /// Arc angle of a curved wall (0 for straight walls); begin and end are the chord ends.
    pub arc_angle: f64,
    pub profile: Profile,
    /// Slant angle, only for a slanted profile.
    pub slant_alpha: Option<f64>,
}

/// The objects walls and stories are read from, gathered in one walk.
pub(crate) struct Objects {
    types: HashMap<Guid, i16>,
    walls: Vec<(Guid, Vec<u8>)>,
    /// Story object GUID -> (story index, name).
    story_objects: HashMap<Guid, (u16, String)>,
    /// Top anchor GUID -> GUID of its story object.
    anchors: HashMap<Guid, Guid>,
    /// Composite attribute index -> (GUID, name).
    composites: HashMap<i32, (Guid, String)>,
    pub stories: Vec<Story>,
}

/// Decode a composite attribute payload: (index, name).
fn parse_composite(p: &[u8]) -> Option<(i32, String)> {
    let (name, end) = counted_utf16(p, 55)?;
    Some((i32_at(p, end + 2)?, name))
}

fn owned_by(p: &[u8], role: Guid) -> Option<Guid> {
    let (_, list) = split_object(p).ok()?;
    links(list)
        .into_iter()
        .find(|&(tag, r, _)| tag == TAG_OWNED_BY && r == role)
        .map(|(_, _, target)| target)
}

impl Objects {
    /// Only live blocks (the block walk, free blocks skipped) are used, so a
    /// GUID left behind by a deleted object never resolves.
    pub(crate) fn read(pla: &mut Pla) -> Result<Objects> {
        let anchor_role = Guid::parse(ANCHOR_STORY_ROLE).unwrap();
        let mut o = Objects {
            types: HashMap::new(),
            walls: Vec::new(),
            story_objects: HashMap::new(),
            anchors: HashMap::new(),
            composites: HashMap::new(),
            stories: Vec::new(),
        };
        let mut tables = 0;
        pla.walk(|b| {
            let ty = b.info.block_type;
            if ty == crate::FREE_BLOCK {
                return Ok(());
            }
            let g = b.info.guid;
            o.types.insert(g, ty);
            match ty {
                WALL_TYPE => o.walls.push((g, b.payload()?)),
                STORY_TYPE => {
                    o.story_objects
                        .insert(g, story::parse_story_object(&b.payload()?)?);
                }
                TOP_ANCHOR_TYPE => {
                    if let Some(s) = owned_by(&b.payload()?, anchor_role) {
                        o.anchors.insert(g, s);
                    }
                }
                COMPOSITE_TYPE => {
                    if let Some((i, name)) = parse_composite(&b.payload()?) {
                        o.composites.entry(i).or_insert((g, name));
                    }
                }
                STORY_TABLE_TYPE => {
                    tables += 1;
                    if tables > 1 {
                        return Err(Error::Unsupported("more than one story table"));
                    }
                    o.stories = story::parse_table(&b.payload()?)?;
                }
                _ => {}
            }
            Ok(())
        })?;
        Ok(o)
    }

    pub(crate) fn walls(&self) -> Result<Vec<Wall>> {
        self.walls
            .iter()
            .map(|(g, p)| self.decode_wall(*g, p))
            .collect()
    }

    fn level(&self, index: u16) -> Option<f64> {
        self.stories
            .iter()
            .find(|s| s.index == index)
            .map(|s| s.level)
    }

    fn decode_wall(&self, guid: Guid, p: &[u8]) -> Result<Wall> {
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

        let home_role = Guid::parse(HOME_STORY_ROLE).unwrap();
        let top_role = Guid::parse(TOP_LINK_ROLE).unwrap();
        let mut home_story = None;
        let mut top_anchor = None;
        for (tag, role, target) in links(list) {
            if tag != TAG_OWNED_BY {
                continue;
            }
            let ty = self.types.get(&target);
            if role == home_role && ty == Some(&STORY_TYPE) {
                home_story = home_story.or(Some(target));
            } else if role == top_role && ty == Some(&TOP_ANCHOR_TYPE) {
                top_anchor = top_anchor.or(Some(target));
            }
        }
        let home = home_story.and_then(|g| self.story_objects.get(&g));
        let home_story_index = home.map(|s| s.0);
        let top_story_index = top_anchor
            .and_then(|a| self.anchors.get(&a))
            .and_then(|s| self.story_objects.get(s))
            .map(|s| s.0);
        let top_linked = top_anchor.is_some();

        let bottom_offset = f(0xA1);
        let top_offset = f(0x91);
        let (height, relative_top_story) = if top_linked {
            match (home_story_index, top_story_index) {
                (Some(h), Some(t)) => (
                    self.level(t)
                        .zip(self.level(h))
                        .map(|(lt, lh)| lt + top_offset - lh - bottom_offset),
                    Some(i32::from(t) - i32::from(h)),
                ),
                _ => (None, None),
            }
        } else {
            (Some(f(0x99)), Some(0))
        };

        let structure = Structure::from(u16_at(p, e + 0xE1).unwrap());
        let composite_index =
            (structure == Structure::Composite).then(|| i32_at(p, e + 0xE5).unwrap());
        let composite = composite_index.and_then(|i| self.composites.get(&i));
        let profile = Profile::from(u16_at(p, e + 0xB8).unwrap());

        Ok(Wall {
            guid,
            id,
            begin: [f(0x00), f(0x08)],
            end: [f(0x10), f(0x18)],
            begin_thickness: f(0x4D),
            end_thickness: f(0x55),
            height,
            bottom_offset,
            top_offset,
            top_linked,
            home_story,
            home_story_index,
            home_story_name: home.map(|s| s.1.clone()),
            top_story_index,
            relative_top_story,
            reference_line: ReferenceLine::from(u32_at(p, e + 0xEF).unwrap()),
            reference_line_offset: f(0x5D),
            offset_from_outside: f(0xF3),
            flipped: p[e + 0xEE] != 0,
            structure,
            composite_index,
            composite_name: composite.map(|c| c.1.clone()),
            composite_guid: composite.map(|c| c.0),
            geometry: Geometry::from(p[e + 0x7E]),
            arc_angle: f(0x75),
            profile,
            slant_alpha: (profile == Profile::Slanted).then(|| f(0xBA)),
        })
    }
}

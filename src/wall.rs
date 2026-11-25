//! Walls, with the objects they point to: story objects, top anchors,
//! composites and the story table. Every kind is found by its class GUID; the
//! block type number each kind has had so far (265, 210, 204, 137, 224) is only
//! a hint. Doors and windows, layers and building materials are gathered here too.

use std::collections::HashMap;
use std::fmt;

use crate::opening::{self, Opening, OpeningKind};
use crate::story::{self, Story};
use crate::{
    counted_utf16, f64_at, i32_at, links, owned, owns, split_object, u16_at, u32_at, utf16, Error,
    Guid, Pla, Result,
};

pub(crate) const WALL_CLASS: &str = "4f654582-33af-4e52-abfc-c35d46765ec4";
pub(crate) const STORY_CLASS: &str = "bc046b25-27d6-461a-a6e5-6fa0e8594a95";
pub(crate) const STORY_TABLE_CLASS: &str = "2fa7b8cb-ee33-48d6-8daa-8a3fe02bddea";
pub(crate) const TOP_ANCHOR_CLASS: &str = "64d6044d-03e8-42ec-bd56-8813cc6481cb";
pub(crate) const COMPOSITE_CLASS: &str = "c7ecb629-3cf3-4c9c-a222-11082672e04b";
pub(crate) const LAYER_CLASS: &str = "6fda3fe2-8e75-4e3f-b8c0-d0c4777e5d4d";
pub(crate) const BUILDING_MATERIAL_CLASS: &str = "2b0c5e9b-605e-4b8f-aaae-cc166f03bee5";
/// Block types the kinds above have had in every file so far. A payload of
/// such a block that does not decode stops the read, as before; any other
/// block's payload may fail without stopping it.
const HINT_TYPES: [i16; 7] = [265, 210, 204, 137, 224, 140, 151];

/// Role of a wall's link to its home story.
const HOME_STORY_ROLE: &str = "79b61203-ae84-4d88-a9c6-20ead705e0bc";
/// Role of a wall's link to the top anchor of the story its top is linked to.
const TOP_LINK_ROLE: &str = "7b48cc47-7267-4cfd-9133-9214ea18fb99";
/// Role of a top anchor's link to its story.
const ANCHOR_STORY_ROLE: &str = "baf95b80-f256-4932-b891-03ff66848591";

/// Position of the reference line across the wall. Codes 0 to 5 are the
/// complete set; anything else is kept as `Other` and noted as invalid.
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
    /// Code 2, inferred from the documented order (not yet seen in a file).
    Polygonal,
    Other(u8),
}

impl From<u8> for Geometry {
    fn from(n: u8) -> Self {
        match n {
            0 => Geometry::Straight,
            1 => Geometry::Trapezoid,
            2 => Geometry::Polygonal,
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
    /// Offset of the top from the story the top is linked to; none when not linked.
    pub top_offset: Option<f64>,
    /// True when the wall top is linked to a story.
    pub top_linked: bool,
    pub home_story: Option<Guid>,
    pub home_story_index: Option<i16>,
    pub home_story_name: Option<String>,
    /// Index of the story the top is linked to.
    pub top_story_index: Option<i16>,
    /// Top story index minus home story index; 0 when the top is not linked.
    /// A negative value is invalid and is noted in `notes`.
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
    /// Doors and windows hosted by this wall (their GUIDs), in link-list order.
    pub doors: Vec<Guid>,
    pub windows: Vec<Guid>,
    /// Things in the record that a valid file should not contain. The wall is
    /// still read; each note says which reading was taken.
    pub notes: Vec<String>,
}

/// Attribute GUIDs (the block GUIDs, which are Archicad's attribute GUIDs), in file order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attributes {
    pub composites: Vec<Guid>,
    pub layers: Vec<Guid>,
    pub building_materials: Vec<Guid>,
}

/// The objects walls and stories are read from, gathered in one walk.
pub(crate) struct Objects {
    /// Block GUID -> class GUID (first 16 bytes of the payload).
    classes: HashMap<Guid, Guid>,
    walls: Vec<(Guid, Vec<u8>)>,
    openings: Vec<(Guid, OpeningKind, Vec<u8>)>,
    /// Library-part reference object -> the library part ID it holds.
    lib_parts: HashMap<Guid, Option<Guid>>,
    /// Story object GUID -> (story index, name).
    story_objects: HashMap<Guid, (i16, String)>,
    /// Top anchor GUID -> GUID of its story object.
    anchors: HashMap<Guid, Guid>,
    /// Composite attribute index -> every (GUID, name) with that index
    /// (more than one should not happen: indices are unique).
    composites: HashMap<i32, Vec<(Guid, String)>>,
    pub stories: Vec<Story>,
    pub attributes: Attributes,
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
        .find(|&(tag, r, _)| owned(tag) && r == role)
        .map(|(_, _, target)| target)
}

impl Objects {
    /// Only live blocks (the block walk, free blocks skipped) are used, so a
    /// GUID left behind by a deleted object never resolves.
    pub(crate) fn read(pla: &mut Pla) -> Result<Objects> {
        let anchor_role = Guid::parse(ANCHOR_STORY_ROLE).unwrap();
        let wall_class = Guid::parse(WALL_CLASS).unwrap();
        let door_class = Guid::parse(opening::DOOR_CLASS).unwrap();
        let window_class = Guid::parse(opening::WINDOW_CLASS).unwrap();
        let lib_class = Guid::parse(opening::LIB_PART_REF_CLASS).unwrap();
        let [story_class, table_class, anchor_class, composite_class, layer_class, material_class] =
            [
                STORY_CLASS,
                STORY_TABLE_CLASS,
                TOP_ANCHOR_CLASS,
                COMPOSITE_CLASS,
                LAYER_CLASS,
                BUILDING_MATERIAL_CLASS,
            ]
            .map(|c| Guid::parse(c).unwrap());
        let mut o = Objects {
            classes: HashMap::new(),
            walls: Vec::new(),
            openings: Vec::new(),
            lib_parts: HashMap::new(),
            story_objects: HashMap::new(),
            anchors: HashMap::new(),
            composites: HashMap::new(),
            stories: Vec::new(),
            attributes: Attributes::default(),
        };
        let mut tables = 0;
        pla.walk(|b| {
            let ty = b.info.block_type;
            if ty == crate::FREE_BLOCK {
                return Ok(());
            }
            let g = b.info.guid;
            // Every payload is decoded to learn its class.
            let p = match b.payload() {
                Ok(p) => p,
                Err(e) if HINT_TYPES.contains(&ty) => return Err(e),
                Err(_) => return Ok(()),
            };
            if p.len() < 16 {
                return Ok(());
            }
            let class = Guid::from_slice(&p);
            o.classes.insert(g, class);
            if class == wall_class {
                o.walls.push((g, p));
            } else if class == door_class {
                o.openings.push((g, OpeningKind::Door, p));
            } else if class == window_class {
                o.openings.push((g, OpeningKind::Window, p));
            } else if class == lib_class {
                o.lib_parts.insert(g, opening::parse_lib_part_ref(&p));
            } else if class == story_class {
                o.story_objects.insert(g, story::parse_story_object(&p)?);
            } else if class == anchor_class {
                if let Some(s) = owned_by(&p, anchor_role) {
                    o.anchors.insert(g, s);
                }
            } else if class == composite_class {
                o.attributes.composites.push(g);
                if let Some((i, name)) = parse_composite(&p) {
                    o.composites.entry(i).or_default().push((g, name));
                }
            } else if class == layer_class {
                o.attributes.layers.push(g);
            } else if class == material_class {
                o.attributes.building_materials.push(g);
            } else if class == table_class {
                tables += 1;
                if tables > 1 {
                    return Err(Error::Unsupported(
                        "more than one story table (probably hotlink modules)",
                    ));
                }
                o.stories = story::parse_table(&p)?;
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

    pub(crate) fn openings(&self) -> Result<Vec<Opening>> {
        self.openings
            .iter()
            .map(|(g, k, p)| opening::decode(*g, *k, p, &self.classes, &self.lib_parts))
            .collect()
    }

    fn level(&self, index: i16) -> Option<f64> {
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
        let mut notes = Vec::new();
        let mut home_story = None;
        let mut top_anchor = None;
        let (door_role, window_role) = (
            OpeningKind::Door.host_role(),
            OpeningKind::Window.host_role(),
        );
        let door_class = Guid::parse(opening::DOOR_CLASS).unwrap();
        let window_class = Guid::parse(opening::WINDOW_CLASS).unwrap();
        let story_class = Guid::parse(STORY_CLASS).unwrap();
        let anchor_class = Guid::parse(TOP_ANCHOR_CLASS).unwrap();
        let (mut doors, mut windows) = (Vec::new(), Vec::new());
        for (tag, role, target) in links(list) {
            let class = self.classes.get(&target);
            // One opening of a kind is a 0x11 item, two or more one 0x01 item.
            if owns(tag) {
                if role == door_role && class == Some(&door_class) {
                    doors.push(target);
                } else if role == window_role && class == Some(&window_class) {
                    windows.push(target);
                }
                continue;
            }
            if !owned(tag) {
                continue;
            }
            let (slot, what) = if role == home_role && class == Some(&story_class) {
                (&mut home_story, "home story")
            } else if role == top_role && class == Some(&anchor_class) {
                (&mut top_anchor, "top")
            } else {
                continue;
            };
            if slot.is_some() {
                notes.push(format!("more than one {what} link; the first is used"));
            } else {
                *slot = Some(target);
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
        // The stored top offset means nothing unless the top is linked.
        let top_offset = top_linked.then(|| f(0x91));
        let (height, relative_top_story) = match (top_offset, home_story_index, top_story_index) {
            (None, ..) => (Some(f(0x99)), Some(0)),
            (Some(off), Some(h), Some(t)) => (
                self.level(t)
                    .zip(self.level(h))
                    .map(|(lt, lh)| lt + off - lh - bottom_offset),
                Some(i32::from(t) - i32::from(h)),
            ),
            _ => (None, None),
        };
        if let Some(r) = relative_top_story.filter(|&r| r < 0) {
            notes.push(format!(
                "relative top story {r} is invalid (the top is below the home story)"
            ));
        }

        let structure = Structure::from(u16_at(p, e + 0xE1).unwrap());
        let composite_index =
            (structure == Structure::Composite).then(|| i32_at(p, e + 0xE5).unwrap());
        let composite = match composite_index.and_then(|i| self.composites.get(&i)) {
            Some(c) if c.len() > 1 => {
                notes.push(format!(
                    "{} composites share index {}; none is used",
                    c.len(),
                    composite_index.unwrap()
                ));
                None
            }
            c => c.and_then(|c| c.first()),
        };
        let profile = Profile::from(u16_at(p, e + 0xB8).unwrap());
        let reference_line = ReferenceLine::from(u32_at(p, e + 0xEF).unwrap());
        if let ReferenceLine::Other(n) = reference_line {
            notes.push(format!("reference line code {n} is invalid (0 to 5)"));
        }
        let flipped = p[e + 0xEE];
        if flipped > 1 {
            notes.push(format!("flipped byte is {flipped}, read as flipped"));
        }

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
            reference_line,
            reference_line_offset: f(0x5D),
            offset_from_outside: f(0xF3),
            flipped: flipped != 0,
            structure,
            composite_index,
            composite_name: composite.map(|c| c.1.clone()),
            composite_guid: composite.map(|c| c.0),
            geometry: Geometry::from(p[e + 0x7E]),
            arc_angle: f(0x75),
            profile,
            slant_alpha: (profile == Profile::Slanted).then(|| f(0xBA)),
            doors,
            windows,
            notes,
        })
    }
}

//! Doors and windows: one record layout, told apart by class GUID.

use std::collections::HashMap;
use std::fmt;

use crate::{f64_at, links, split_object, u32_at, utf16, Error, Guid, Result};
use crate::{owned, owns};

pub(crate) const DOOR_CLASS: &str = "d8f07689-9cfa-4fbe-aeb4-0a60b8e667ee";
pub(crate) const WINDOW_CLASS: &str = "93f161a0-4c4e-4df7-a100-4fd0e8c4f1e1";
/// Library-part reference object, owned by a door or window.
pub(crate) const LIB_PART_REF_CLASS: &str = "2bfca33d-bd38-49e3-bf6d-52ac8b850b9f";
/// Role of the host-wall link (wall `0x11`/`0x01` to the door, door `0x12` to the wall).
pub(crate) const DOOR_HOST_ROLE: &str = "1f6bfe02-d60c-40ae-9ac8-c3953ce12814";
pub(crate) const WINDOW_HOST_ROLE: &str = "c54379e6-afd0-4b9a-9887-af5baeeccf84";
/// Role of a door's or window's link to its library-part reference.
pub(crate) const LIB_PART_ROLE: &str = "672f1c46-64f3-41f5-8a4a-ffdfd86bcbfd";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpeningKind {
    Door,
    Window,
}

impl fmt::Display for OpeningKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl OpeningKind {
    pub(crate) fn host_role(self) -> Guid {
        Guid::parse(match self {
            OpeningKind::Door => DOOR_HOST_ROLE,
            OpeningKind::Window => WINDOW_HOST_ROLE,
        })
        .unwrap()
    }
}

/// One door or window. Lengths in metres.
#[derive(Clone, Debug, PartialEq)]
pub struct Opening {
    pub guid: Guid,
    pub kind: OpeningKind,
    /// Element ID as shown in Archicad (for example `D - 001`).
    pub id: String,
    pub width: f64,
    pub height: f64,
    pub sill_height: f64,
    /// Distance along the host wall's reference line from its begin point to the centre.
    pub center_offset: f64,
    /// Mirrored along the wall.
    pub reflected: bool,
    /// Opening side flipped.
    pub o_side: bool,
    /// Placed from the wall's other side.
    pub ref_side: bool,
    /// GUID of the host wall.
    pub host_wall: Option<Guid>,
    /// The library part's own unique ID.
    pub lib_part: Option<Guid>,
    /// Things in the record that a valid file should not contain (see [`crate::Wall::notes`]).
    pub notes: Vec<String>,
}

/// Library-part reference payload: the library part's own unique ID at offset 51.
pub(crate) fn parse_lib_part_ref(p: &[u8]) -> Option<Guid> {
    let (data, _) = split_object(p).ok()?;
    (51 + 16 <= 39 + data.len()).then(|| Guid::from_slice(&p[51..67]))
}

/// Decode a door or window payload. `classes` maps live block GUIDs to class
/// GUIDs; `lib_parts` maps library-part reference objects to the ID they hold.
pub(crate) fn decode(
    guid: Guid,
    kind: OpeningKind,
    p: &[u8],
    classes: &HashMap<Guid, Guid>,
    lib_parts: &HashMap<Guid, Option<Guid>>,
) -> Result<Opening> {
    let (data, list) = split_object(p)?;
    let bad = || Error::Corrupt("door or window record shorter than expected");
    let n = u32_at(p, 0x151).ok_or_else(bad)? as usize;
    if n > 1024 {
        return Err(Error::Corrupt(
            "door or window element ID length out of range",
        ));
    }
    let e = 0x155 + 2 * n;
    if e + 0x48 > 39 + data.len() {
        return Err(bad());
    }
    let id =
        utf16(&p[0x155..e]).ok_or(Error::Corrupt("door or window element ID is not UTF-16"))?;
    let f = |o: usize| f64_at(p, o).unwrap();
    let mut notes = Vec::new();
    let mut flag = |o: usize, name: &str| {
        let b = p[o];
        if b > 1 {
            notes.push(format!("{name} byte is {b}, read as set"));
        }
        b != 0
    };
    let reflected = flag(0xF8, "reflected");
    let o_side = flag(0xF9, "oSide");
    let ref_side = flag(0xFA, "refSide");

    let wall_class = Guid::parse(crate::wall::WALL_CLASS).unwrap();
    let host_role = kind.host_role();
    let lib_role = Guid::parse(LIB_PART_ROLE).unwrap();
    let mut host_wall = None;
    let mut lib_part = None;
    for (tag, role, target) in links(list) {
        // The opening's own owned-side link is authoritative for the host.
        let (slot, what, value) =
            if owned(tag) && role == host_role && classes.get(&target) == Some(&wall_class) {
                (&mut host_wall, "host wall", Some(target))
            } else if owns(tag) && role == lib_role {
                let Some(v) = lib_parts.get(&target) else {
                    continue;
                };
                (&mut lib_part, "library part", *v)
            } else {
                continue;
            };
        if slot.is_some() {
            notes.push(format!("more than one {what} link; the first is used"));
        } else {
            *slot = Some(value);
        }
    }
    if lib_part == Some(None) {
        notes.push("library part reference too short to hold its ID".into());
    }

    Ok(Opening {
        guid,
        kind,
        id,
        width: f(0xFD),
        height: f(0x105),
        sill_height: f(e + 0x40),
        center_offset: f(e + 0x18),
        reflected,
        o_side,
        ref_side,
        host_wall: host_wall.flatten(),
        lib_part: lib_part.flatten(),
        notes,
    })
}

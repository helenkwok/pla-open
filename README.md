# pla-open

An open-source reader for Archicad project files (`.pln` and `.pla`), written in Rust from our own study of files we made ourselves.

## What it reads

- **The container.** It checks the file signature, walks the whole file (blocks, compressed sections and the empty units between them) and decodes block payloads stored raw, with a zero mask, or with QuickLZ level 1. Large compressed sections are stored in 1 MiB chunks; the reader joins them before walking the blocks inside, since blocks cross chunk boundaries. Every block can be listed by type and GUID.
- **Stories.** The story table: index (negative for stories below ground), level, height and name of each story, plus the extra story above the top one that the file keeps to give the top story its height.
- **Walls.** For each wall: element ID, start and end points, thickness at each end, height, bottom offset, home story, and for a wall whose top is linked to a story the top story, the top offset and the height worked out from the story levels. Also the reference line (outside, center, inside, or the same three for the core) with its offset and its distance from the outside face, whether the wall is flipped, basic or composite structure with the composite's name, straight, trapezoid or polygonal plan, the arc angle of a curved wall, and normal or slanted profile with the slant angle.
- **Doors and windows.** For each: door or window, element ID, width, height, sill height, the centre's distance along the host wall's reference line from its begin point, the three flip flags (mirrored, opening side flipped, placed from the wall's other side), the host wall, and the library part's own unique ID. Each wall lists the doors and windows it hosts.
- **Attributes.** The GUIDs of the composite, layer and building material attributes (Archicad's own attribute GUIDs).

Objects are recognised by the class GUID at the start of their payload, since block type numbers from about 470 up are given out per file. Every kind the reader uses is found that way: walls, doors and windows, stories, the story table, top anchors, composites, layers and building materials. The type numbers the lower kinds have had in every file seen (265, 210, 204, 137, 224, 140, 151) are only a hint.

Values a valid project should not hold (a reference line code above 5, a top linked below the home story, two composites with one index, a second home or top link, a flipped flag other than 0 or 1) do not stop the reader: the wall is still read and each problem is listed in its `notes`. The top offset is reported only for a wall whose top is linked. A file with more than one story table (probably from hotlinked modules) is reported as not supported yet.

It reads the file as a stream and caps every block, section and decoded payload at 256 MiB, so a large or damaged file gives a clear error instead of filling memory.

## What it does not read yet

- Other elements (slabs, roofs, objects and so on).
- Door and window parameters held in their library-part objects, and an unidentified length in each record.
- Double-slanted, polygonal and profiled (complex) walls: only the fields above are read.
- Writing files.

## Use

```
pla-open info FILE           # header, counts of blocks by type
pla-open stories FILE        # one line per story
pla-open stories FILE --json # stories as JSON (levels in metres)
pla-open walls FILE          # one line per wall
pla-open walls FILE --json   # walls as JSON (lengths in metres, angles in radians)
pla-open openings FILE       # one line per door or window
pla-open openings FILE --json
pla-open attributes FILE     # attribute GUIDs as JSON
```

As a library:

```rust
let mut pla = pla_open::Pla::open("project.pln")?;
for wall in pla.walls()? {
    println!("{} {:?} -> {:?}", wall.id, wall.begin, wall.end);
}
```

## Tests

`cargo test` builds small files from the documented byte layouts and reads them back. An extra test checks walls against JSON files of expected values placed next to project files:

```
PLA_OPEN_TEST_FILES=/path/to/folder[:/another/folder] cargo test -- --ignored
```

Each `name.pln` is compared with `name.details.json`: every wall, door and window field listed above that the JSON holds, and the story list.

## Licence

Apache-2.0. Archicad is a trademark of its owner; this project is not affiliated with it.

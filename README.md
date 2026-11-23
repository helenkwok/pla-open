# pla-open

An open-source reader for Archicad project files (`.pln` and `.pla`), written in Rust from our own study of files we made ourselves.

## What it reads

- **The container.** It checks the file signature, walks the whole file (blocks, compressed sections and the empty units between them) and decodes block payloads stored raw, with a zero mask, or with QuickLZ level 1. Every block can be listed by type and GUID.
- **Stories.** The story table: index, level, height and name of each story, plus the extra story above the top one that the file keeps to give the top story its height.
- **Walls.** For each wall: element ID, start and end points, thickness at each end, height, bottom offset, home story, and for a wall whose top is linked to a story the top story, the top offset and the height worked out from the story levels. Also the reference line (outside, center, inside, or the same three for the core) with its offset and its distance from the outside face, whether the wall is flipped, basic or composite structure with the composite's name, straight or trapezoid plan, the arc angle of a curved wall, and normal or slanted profile with the slant angle.

It reads the file as a stream and caps every block, section and decoded payload at 256 MiB, so a large or damaged file gives a clear error instead of filling memory.

## What it does not read yet

- Other elements (slabs, roofs, doors, windows, objects and so on).
- Double-slanted, polygonal and profiled (complex) walls: only the fields above are read.
- Writing files.

## Use

```
pla-open info FILE           # header, counts of blocks by type
pla-open stories FILE        # one line per story
pla-open stories FILE --json # stories as JSON (levels in metres)
pla-open walls FILE          # one line per wall
pla-open walls FILE --json   # walls as JSON (lengths in metres, angles in radians)
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

Each `name.pln` is compared with `name.details.json`: every wall field listed above that the JSON holds, and the story list.

## Licence

Apache-2.0. Archicad is a trademark of its owner; this project is not affiliated with it.

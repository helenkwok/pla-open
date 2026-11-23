# pla-open

An open-source reader for Archicad project files (`.pln` and `.pla`), written in Rust from our own study of files we made ourselves.

## What it reads

- **The container.** It checks the file signature, walks the whole file (blocks, compressed sections and the empty units between them) and decodes block payloads stored raw, with a zero mask, or with QuickLZ level 1. Every block can be listed by type and GUID.
- **Walls.** For each wall: element ID, start and end points, thickness at each end, height (when the top is not linked to a story), bottom offset, offset of the reference line from the outside face, and the home story with its name.

It reads the file as a stream and caps every block, section and decoded payload at 256 MiB, so a large or damaged file gives a clear error instead of filling memory.

## What it does not read yet

- Other elements (slabs, roofs, doors, windows, objects and so on).
- Story levels, so the height of a wall whose top is linked to a story is not worked out yet.
- Curved, slanted or profiled walls: only the straight wall fields above are read.
- Writing files.

The story name is found by looking for the first text in the story record; treat it as a best effort for now.

## Use

```
pla-open info FILE           # header, counts of blocks by type
pla-open walls FILE          # one line per wall
pla-open walls FILE --json   # walls as JSON (lengths in metres)
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
PLA_OPEN_TEST_FILES=/path/to/folder cargo test -- --ignored
```

Each `name.pln` is compared with `name.details.json` (wall GUID, element ID, coordinates, thickness, height and bottom offset).

## Licence

Apache-2.0. Archicad is a trademark of its owner; this project is not affiliated with it.

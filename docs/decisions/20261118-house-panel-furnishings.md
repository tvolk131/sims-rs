# The house panel's Furnishings rating

*Decided 2026-11-18.*

The house panel's Furnishings bar now shows a rating (0x416851,
`sims_ui::house::furnishings`). It is read from the exe. The two values
it weighs match, to the unit, what the original's simulator held after
loading the Goths' house: track 2's capture of update 1 has the
simulator's +0xa0 at 14,660 and its +0xa4 at 15,277, where the house
file saved 0 for both. Their Furnishings is 58. No screenshot of the
panel was compared.

- **The rating** (0x416851): the architecture's value (+0xa4) over the
  objects' (+0xa0), through string 1 of the global file's `STR#` 505,
  cut toward zero. The objects' value is rounded to single precision
  first; the architecture's is loaded whole and divided by it, and the
  quotient is rounded to single. With the objects' value 0 the ratio is
  100 (0x66674c), not a division.
- **When the values are worked out:** the original's lot statistics
  (0x416085) store them at the load (0x41621f), at the day's turn
  (0x41647e), on leaving build or buy mode (0x4cea9e) and from 0x50206e,
  and the panel reads what was stored last. The panel here works them
  out at each read (`Values::read`), without changing the world. Where
  an object breaks, is mended or changes value between those passes,
  ours follows at once and the original's later. This is a known
  difference, not the original's behaviour. The simulation's
  `Budget::objects_value` and `architecture_value` are left as they
  are.
- **The objects** (0x444cb9), in the lot's order, in 32-bit sums that
  wrap:
  - A tile of a multi-tile group that is not its lead, and a type with
    definition word 9 set to 7, are passed over.
  - An object that stands on the lot (0x432595, 0x422632, read as
    `sims-vm` reads it: its tile, as a signed byte, at least 1 and below
    the lot's size less 1 each way) and is valued adds its value to the
    objects'.
  - Valued (0x4354d5) means: data word 43 lacks flag 0x10, and then
    either the object is a person (word 9 is 2) or its catalog master's
    word 69 (the build mode type, 0x45728d) is 0.
  - Any other object adds its value to the architecture's when that
    master's word is neither 0 nor 4.
  - The value (0x43219a) is data word 41, or the lead's for a tile of a
    group. It is halved toward zero when data word 15 is not 0 on it, or
    for a group on any of its tiles.
  - The definition words are read as `sims-vm` holds them, and the
    master is found as `sims-vm` finds it (`Kinds::types`). A person
    without a master is still valued, since the person test comes first.
    Any other type without a master, where the original reads through a
    null pointer, is neither valued nor counted to the architecture
    here.
- **The architecture** (0x47b006) goes over the tiles of levels 1 and 2
  inside the lot's size. A tile with flag 0x20 is skipped.
  - **Walls:** the tile's wall cell as the wall service gives it
    (0x4795c5, `ExpandedWall`). The lowest segment is taken first, then
    each higher one below 0x20, so the ascending diagonal counts only
    when it is the lowest.
  - Each segment adds its style's price to a sum that is halved at the
    end, since a wall between two tiles is counted from both.
  - A style 1, 7, 8 or 9 wall also adds its pattern's price, whole: the
    side facing the tile, the descending diagonal's second pattern, or
    the ascending diagonal's first.
  - **Floors:** the floor pattern's price. A tile split by a diagonal
    (floor 0xff) with a diagonal wall adds half the price of each half's
    floor. Those floors are kept in the cell's west style and west
    pattern words.
  - **Pool:** on level 1, a pool byte below 0x10 adds the pool's price.
  - A level whose tile flags, floors or walls are missing adds nothing.
    A missing pool array leaves out only the pool's price.
- **The prices** (`Prices::load`) come from `GameData/Build.iff`. Its
  tables, and a pattern file's, are read as the original's string
  loader reads them (0x46feac, `StringTable::parse_chunk`). A resource
  without flag 0x10 is big-endian: 0x470351 swaps its leading word, and
  the count when that is -1. The flag is the resource map's when the
  file has one (0x41d3a7, `Iff::resource_flags`). A leading word other
  than -1, -2 or -3 is the count of length-prefixed strings after it. Each
  string-table entry is three strings, and the first is the price, read
  as C's `atoi` reads its bytes (0x636029, `sims_content::atoi`), so a
  byte order mark before the digits reads 0:
  - Wall styles (0x6e5838): `STR#` 129, entries 1 to 5, give styles 1,
    2, 13, 12 and 14 (0x4cd149), and entry 1 gives style 255 too
    (0x4cd48d). Every other style costs 0.
  - The pool (0x6c5ff8): `STR#` 133, entry 1 (75 here, over the exe's
    100).
  - The built-in floors, keys 1 to 29: `STR#` 130 (0x4a01cb).
  - The built-in wall patterns, keys 0 to 30 and 248 to 255: `STR#` 131
    (0x4c4ee5). Key 0 and the high keys have no entry and cost 0.
  - A plug-in pattern costs string 1, from 0, of its file's `STR#` with
    ID 0 (0x4a0a84, 0x4c59a0), now kept as `SurfaceFile::price`. Without
    that table or string it costs string 0 of `STR#` 134 (floors) or 135
    (walls).
  - **Not ported:** when a `STR#` is missing, the lookup (0x47028b)
    tries a resource of type `'CST'` (0x00435354) with the same ID,
    which is the chunk tag `\0CST` on disk (the reader reverses the
    type's bytes, 0x41a5ab), and reads it by another path (0x470178).
    The install's files have no such chunk.
    Pattern files ask for `STR#` alone.
  - Build mode's catalog, which fills the style table and the pool's
    price, is made on first request (0x4ce31a). The match with the
    capture needs the five style prices, so it was made by update 1.
- **The viewer:** when the house cannot be loaded as the game loads
  it, the viewer shows the house file as saved, whose floors and walls
  keep the house's own pattern numbers, not the catalogs' keys. Its
  panel then leaves Furnishings out rather than price the wrong
  patterns. It does the same for a snapshot, which does not record the
  catalogs its patterns were keyed by: a plug-in added or removed since
  would shift the keys.
- **Unverified:**
  - A wall side whose neighbour does not own it costs 0. The original
    reads whatever its stack held there.
  - A segment above 0x20 in a cell's first byte reads the pattern words
    as styles, as the original's record lays them out. No lot is known
    to have one.
- **Tests:**
  - `atoi`, a byte order mark included;
  - a pattern file's price from `STR#` 0, with another table first,
    without table 0 or its string, a big-endian count without flag
    0x10, and a little-endian length-prefixed table;
  - the string loader's byte orders and layouts, and the map's flags
    over the header's;
  - the prices from stand-in tables and pattern files;
  - the architecture of a hand-made lot (straight walls, a fence, a
    split floor, a diagonal that is not the lowest segment, a skipped
    tile, the pool), an ascending diagonal alone, the walls' sum
    wrapping before it is halved, and a level missing an array;
  - the scan over hand-made objects (broken, not valued, persons, off
    the lot, the lot's edge at its size less 2 and less 1, type 7, a
    group, an unread type, a person and another type without a
    master);
  - the rating, including no valued objects;
  - the Goths' values (14,660 and 15,277), Furnishings 58 and the
    panel's digest, gated on `SIMS_INSTALL`.

Reopen with a screenshot of the original's house panel, or if the
simulation takes over the lot statistics' values.

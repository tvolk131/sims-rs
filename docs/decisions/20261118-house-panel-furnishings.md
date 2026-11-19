# The house panel's Furnishings rating

*Decided 2026-11-18.*

The house panel's Furnishings bar now shows a rating (0x416851,
`sims_ui::house::furnishings`). It weighs the lot's two values as the
simulation keeps them (`Budget::objects_value` and
`architecture_value`), which the original's panel reads from its
simulator (0x41685f, 0x41687e). The lot statistics' sums that work them
out (`sims_ui::house::Values::read`) are ported too, from the exe, for
the simulation to store; the panel does not run them. For the Goths'
house the two agree to the unit: track 2's capture of update 1 has the
simulator's +0xa0 at 14,660 and its +0xa4 at 15,277, the house file in
the player's folder saved the same two, and the sums worked out from
the lot with those cleared come to them too. Their Furnishings is 58.
No screenshot of the panel was compared.

- **The rating** (0x416851): the architecture's value (+0xa4) over the
  objects' (+0xa0), through string 1 of the global file's `STR#` 505,
  cut toward zero. The curve's text is its bytes up to the first zero
  byte, as the original copies them as a C string (0x4703ad), each read
  as it is by its scanner (0x461435): a byte order mark is two bytes
  that are not a point, not a change of encoding. **Unverified:** its
  points are read by the motives' curve parser
  (`sims_sim::motives::Curve::parse`), which drops a point that is not
  well formed (`(100;90 coins)`) where the original's scanner (0x4614a2)
  reads it, and takes the reciprocal of the points' span whole where
  the original rounds the span to single precision first (0x4eedf5,
  0x46140a to 0x461416), which can differ for a wide span. That parser
  is shared with the motives, so a fix belongs to the simulation; the
  install's curves are well formed and narrow. The objects'
  value is rounded to single precision first; the architecture's is loaded whole and divided by it, and the
  quotient is rounded to single. With the objects' value 0 the ratio is
  100 (0x66674c), not a division.
- **When the values are worked out:** the original keeps them in its
  simulator (+0xa0 and +0xa4), saved and loaded in `SIMI` (0x46d1ad,
  0x46d1bd). Its lot statistics (0x416085) store them (0x416118,
  0x416126) on going to live mode (0x4cea9e through 0x416534), so after
  a load, at the day's turn (0x41647e, signal 0xdc), from the budget
  dialog (0x50206e through 0x416558, which stores them only when the
  simulator's word +0x36 is not 0, 0x41655b) and before the house is
  written
  (0x41621f, ahead of the `SIMI` write at 0x416253). The panel reads
  what was stored last, so an object that breaks or is mended shows at
  the next of those. Here the panel reads the world's budget, which a
  checkpoint or snapshot keeps with the rest of the world. The
  simulation stores the objects' value at the day's turn (`sims-vm`'s
  `day_statistics`) and neither value elsewhere: the architecture's
  stays what the house was saved with, past any day's turn, and going
  to live mode stores neither (`World::update_statistics`). So a house
  saved with values out of date shows them, and after a day turns the
  new objects' value is weighed against the saved architecture's. This is a known gap in
  the simulation, not the original's behaviour; it is the simulation's
  to close, with these sums.
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
- **The prices** (`Prices::load`) come from `GameData/Build.iff`. Each
  string-table entry is three strings, and the first is the price, read
  as C's `atoi` reads its bytes (0x636029, `sims_content::atoi`), so a
  byte order mark before the digits reads 0. The strings are US
  English's (language 1), as everything here reads; the original takes
  the game's language (0x47028b, 0x470256), and some tables price a
  style differently in another language.
- **The string tables:** `Build.iff`'s, a pattern file's and the
  curves' (`STR#` 505) are read as the original's string loader reads
  them (0x46feac, `StringTable::parse_chunk`):
  - A resource without flag 0x10 is big-endian: 0x470351 swaps its
    leading word, and the count when that is -1. The flag is the
    resource map's when the file has one (0x41d3a7,
    `Iff::resource_flags`).
  - A leading word other than -1, -2 or -3 is the count of
    length-prefixed strings after it. A count of 0 or below, in any
    layout, is an empty table.
  - The linker (`sims_vm::program`), which also reads `STR#` 505, falls
    back to this reading for a table its own reader refuses, so such a
    table does not stop the game before the panel reads it.
  - **Not ported:** the original files a version -3 table's strings by
    language, its place starting again at each change of language
    (0x46ffcc). Here they are counted within the language, which is the
    same when each language's strings are together. They are in all 488
    such tables of the install's `Build.iff` and pattern files: 31 loose
    and 457 in FAR archives.
  - **Unverified:** a resource map entry whose ID or flags' high byte
    differs from its chunk header's. The IFF reader refuses the map then
    and looks resources up as if there were none (`sims_formats::iff`,
    from the frozen prototype; `resource_order` reports the error),
    where the original finds the resource by the map's ID (0x41bbfb)
    and compares only the chunk's type (0x41d33e). A price table under
    such an entry is then found by its chunk header's ID, as the lookup
    by ID alone ignores the flags' high byte, so it is missed only when
    that ID differs from the map's. No file of the install has one: its 3,583 mapped
    files, FAR members and family files included, list 163,797 entries,
    each with its chunk's type, ID and flags' high byte.
- **The prices by table:**
  - Wall styles (0x6e5838): `STR#` 129, entries 1 to 5, give styles 1,
    2, 13, 12 and 14 (0x4cd149), and entry 1 gives style 255 too
    (0x4cd48d). Every other style costs 0.
  - The pool (0x6c5ff8): `STR#` 133, entry 1 (75 here, over the exe's
    100).
  - A style's or the pool's string, or its whole table, missing costs
    0 here. The original reads it without a check (0x4cd4d3; 0x470531
    returns null; a missing table's failed lookup is passed over at
    0x4cd45f and 0x4cd727) and crashes in `atoi` (0x4cdace, 0x635fbe).
    The install's tables have them all. So too a Furnishings curve
    string missing from its table: it weighs everything 0 here, where
    the original's scanner crashes on the null string (0x415df9,
    0x415e02, 0x461435). A missing curve table weighs everything 0 in
    both (the original skips both curves, 0x415ddb).
  - The global file's and `Build.iff`'s tables are looked up by ID
    alone, whatever their flags' high byte, as the original's string
    loader does (0x415dd6, 0x4cd45f, through 0x46ff01 and 0x41bbfb):
    of tables sharing an ID, the last in resource order, as its table
    hashes the ID sign-extended (0x41de42 to 0x41de4e), so they share a
    bucket, puts each at the head of its bucket (0x41de8f, 0x41de94)
    and its lookup walks the whole table (0x5807c2, 0x41ddef;
    `Iff::get_by_id`). **Not ported:** past 192 resources the table
    grows to 389 buckets and its rehash reverses each chain (0x41df8d
    to 0x41dfb9), so there the first of tables sharing an ID can win.
    No `STR#` of the install shares an ID with another of its file
    (3,782 files, FAR members included; 151 have more than 192
    resources).
  - The built-in floors, keys 1 to 29: `STR#` 130 (0x4a01cb).
  - The built-in wall patterns, keys 0 to 30 and 248 to 255: `STR#` 131
    (0x4c4ee5). Key 0 and the high keys have no entry and cost 0.
  - A plug-in pattern costs string 1, from 0, of its file's `STR#` with
    ID 0, found by ID alone as above (0x4a0a84, 0x4c59a0), now kept as
    `SurfaceFile::price`. Without
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
- **The catalogs:** the sums price the patterns by the catalogs the
  load built and keyed the lot by (`sims_vm::boot::Lot::surfaces`), not
  by catalogs built again, so a pattern file added in between cannot
  shift the keys. The test prices the Goths' lot so.
- **The viewer:** its panel reads the world's budget as the game's
  does, so it shows Furnishings for the house file as saved and for a
  snapshot too, with the values they hold.
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
  - the rating, including no valued objects, and the panel weighing
    the budget's values;
  - the curve's text cut at a zero byte, and read past a byte order
    mark;
  - the Goths' values (14,660 and 15,277, as saved, and the sums worked
    out with the saved ones cleared), Furnishings 58 and the panel's
    digest, gated on `SIMS_INSTALL`.

Reopen with a screenshot of the original's house panel, or if the
simulation takes over the lot statistics' values.

# The floor and wall pattern catalogs, and a house's maps renumbered through them

*Decided 2026-11-15.*

A house saves its floor and wall patterns as IDs and maps each ID to a
file and a number in that file. The load turns them into the keys of the
game's pattern catalogs (0x4a0e3b, 0x4c5d35). Both catalogs are now built
the way the original builds them at start-up (floors 0x4a01cb, walls
0x4c512d; `sims_content::Surfaces`):

- File 0 is the built-in `floors.iff` or `walls.iff`.
- Then come the `.FLR` or `.WLL` files of each installed pack's
  `WallsAndFloors/` (in edition-bit order), then
  `ExpansionShared/WallsAndFloors/` (edition above 3), then
  `GameData/Floors/` or `GameData/Walls/`. Each directory is listed in
  its file-table order, going into each subdirectory where its name
  falls (0x412699; the catalog scan asks for every directory, 0x412863).
  A file is known by its file name alone, read in Windows-1252 as a
  house's map names it, byte for byte: a leading byte-order mark is part
  of the name, as the original's byte comparison sees it. The same holds
  for the Windows-1252 text `sims-formats` reads through its shared
  reader (a house's surface map and roofs, FAR member names, chunk
  labels, field strings, tuning and career values): none is sniffed for
  a byte-order mark. The language-aware string tables (`STR#`,
  `strings::decode`) still go through the encoding's own decode, which
  sniffs one; nothing on the catalogs' path reads them, and the
  original's handling there is not checked.
- A file's pattern count is a third of its `SPR2` (floors) or `SPR#`
  (walls) chunks, and a file with none is left out.
- Each pattern takes the next key from a free list seeded with
  [30, 254] and [256, 65534] for floors, or [31, 247] and [256, 65534]
  for walls. A key taken for a missing sprite is lost.

**Verified** against the catalogs in the guest's memory on the Complete
Collection: 209 floor and 273 wall files, the file at every place whose
name the guest still held (150 of 208 floor files, 186 of 272 wall
files), and every key (`crates/sims-content/tests/surfaces.rs`). The
directory listing order on disk would only matter for two names sharing
a file-table bucket, and none do in this install.

The renumbering (`World::renumber_surfaces`):

- **Lookup.** Each map group finds the first catalog file of its name
  (any case), and each saved ID listed in the group takes that file's key
  for its number. A missing pattern gives 0xffff, and a later group
  overwrites an earlier one's ID unconditionally, as in the original.
- **IDs left 0xffff.** An ID still at 0xffff would be matched to a
  pattern by colour (0x4a0bbd). That is not ported and fails loudly
  (`LoadError::SurfaceColour`).
- **Unlisted IDs.** An ID no group lists becomes 1.
- **Floors** are renumbered first: each floor cell, then the half floors
  of a diagonal wall.
- **Walls** come next. Each wall cell is read through the wall service
  (0x4795c5) and written back (0x4797e2) with its neighbours, as
  `legacy/crates/sims-vm/src/native_wall_io.rs` did. A write-back the
  port does not know fails loudly (`LoadError::SurfaceRewrite`). The
  wall vertex map that the put also refreshes (world +0x4c/+0x50,
  0x479c58) is drawing state and is left out.

**Unverified:** no capture holds a house whose maps have entries, so
the renumbered cells are not yet compared with the original's. The Goth
and Newbie houses' maps are empty and their pins are unchanged.

What the houses do now (`sims boot`, 1000 ticks, no answers):

- UserData2's houses 1 and 9 now get past the renumbering.
  - House 1 stops at the household selection (no family).
  - House 9 stops at the first frame: person 195's ROOT bone was last
    turned by a layer that no longer plays (0x5992b3).
- Of the 474 houses surveyed (UserData to UserData5 and 69 of
  UserData6), none stops at the renumbering. 306 stop at an object with
  more saved attributes than the 8 its constructor allocates.

Reopen if a capture of a house with map entries renumbers a cell
differently, or another install lists two pattern files in one bucket.

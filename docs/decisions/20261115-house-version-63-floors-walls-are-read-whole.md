# From house version 63 the floors and walls are read whole from Arry 11, 12, 111 and 112

*Decided 2026-11-15.*

The lot array loader (0x478599) tests the house's version once per level
(0x478617): from 63 on, it reads the lot's floor array from Arry chunk
level base + 11 and its wall array from base + 12 (0x47861d to
0x478652), and skips the widening of chunks 1 and 2 that older versions
need. The read (0x431005) replaces the whole array, taking the chunk's
size and cell size, as for every other array. So a version-63 house's
floors and walls are 2-byte and 14-byte cells as the lot holds them,
every cell of them, not just those inside the lot's size. `World::read_arrays`
now does that (`load::WIDE_SURFACES`). The loader's other version tests
are all below 53, so this covers every later version too (the expansion
lots are 69 to 73).

Every shipped house of version 63 or later has these arrays at 64 by 64,
with 2-byte floor and 14-byte wall cells, and the port requires that: any
other size or cell size fails here. The original's read takes any size
(its resize, 0x4312ff to 0x431388, has no 64-by-64 requirement), and what
follows with another shape is not known. Arrays 1, 2, 101 and 102 left in such a file are not read, as in
the original.

The floor and wall maps already followed the version (chunk 1 and word IDs
from 63, `House::load`).

**Verified:** the Goth house (version 62) is byte-identical through tick
2881, hash included (`eed688a0...`). Booted together with the records
change, UserData3's house 2 (version 63) gets through the arrays and the
renumbering and stops at the household selection (it has no family).
**Unverified:** a version-63 lot's arrays against a capture.

Next for the template houses: UserData2's houses 1 and 9 have floor and
wall maps with entries, and resolving those to the game's loaded patterns
is not ported (`World::renumber_surfaces`).

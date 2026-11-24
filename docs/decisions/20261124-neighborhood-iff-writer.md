# Neighborhood.iff writer

*Decided 2026-11-24.*

This follows
[the neighborhood open](20261124-neighborhood-open-and-pools-parity.md),
which left `Neighborhood.iff` unwritten. `cTSNeighborhood::Save`
(`Sims.exe` 1.91 0x4252cc, called with stream version 0x49) is now
ported. `sims_formats::neighborhood_file` writes the file.
`sims_sim::neighborhood_save` builds the file's contents from a
`StoredNeighborhood`. Saving the three game-written captures we have
(UD2 after-run2, after-with-lot-load, after-partial-0607utc) gives the
recorded files byte for byte. This holds both directly from the parsed
file and through import → REDB → save. All addresses below are
`Sims.exe` 1.91.

## What 0x4252cc does

1. It opens a new file at `NTemp.iff` (0x6c2608) in the neighborhood
   folder (0x46f4ce, 0x465949 mode 1).
2. If a lot is loaded (`[ebx+0x44]`), data word 2 (+0x12c) becomes the
   current house (+0x164).
3. **NGBH #1:** stream header 0x464dbd(key 0x4301f7 'NGBH',
   version 0x49, id 1), serialized by 0x4254f6.
4. **NBRS #1:** 0x43071e(+0x148 vector, 'NBRS', 1, 0x49) → 0x464dbd,
   serialized by 0x430a24 → 0x430ad6.
5. **Families:** a loop over the family vector (+0x154..+0x158) in
   vector order. For each family:
   - if its house (+0x110) is the current house, its architecture value
     (+0x11c) becomes round(lot +0xa4 × 0x6e4a88) + lot +0x9c + lot +0xa0;
   - if byte +0x124 is set, 0x427bd7 runs;
   - then 0x410696 writes the family's resources.
6. **TATT #1:** 0x43e0be → 0x43f114('TATT', id 1, version 0),
   serialized by 0x43e0f0.
7. 0x46b6d2 closes the file and writes the resource map. On success
   (0x46b42a, then 0x5b8f64 / 0x5b9988), `NTemp.iff` is moved onto
   `Neighborhood.iff`.

The family vector is kept sorted by comparator 0x425209:

- the family number (+0x114), descending;
- then the ID (+0x10c, compared signed), descending.

This decides the FAMI/FAMs/FAMh order.

## Stream header (0x464dbd / 0x464cdf)

A counting pass sizes the payload. Then a buffer of size + 12 is
allocated and not zeroed (0x41ff87). The header is:

- 4 zero bytes;
- the version, as a little-endian u32;
- the key's tag (key vtable +8), as a little-endian u32. This is why
  'NGBH' appears as `HBGN`.

The payload follows.

## Chunks

- **NGBH:** the 16 data words, then the inventory vector. Each slot is
  either 0, or 1 + neighbor i16 + item count + items (i32 kind,
  u32 GUID, u16 count). Then the option bytes, by minimum version:
  66 → 0,1; 68 → 2,3; 69 → 10; 70 → 4..7; 73 → 8,9.
- **NBRS (0x430ad6):** the count, then per slot a presence int (0/1).
  For each present slot, 0x423e08 writes:
  - int 10, then int 9 (the record's own version);
  - the name, via 0x464c3e;
  - int +0x1c, then int +0x18 (the person mode);
  - if the mode is nonzero (0x423e8e, `je`: negative modes too),
    0x100 shorts from +0x74. 0x423e08 is the read and write in one, so
    the reader now reads them for any nonzero mode, not only positive
    ones, and so does the store's world writeback. sims-vm's CAF export
    (`caf.rs`) still writes them only for a positive mode; that file is
    outside this change and is a follow-up;
  - short id, int GUID (+4);
  - the relationship map (0x4650ee): −1, the count, then per entry 1,
    the target, the count, and the values.
- **FAMI (0x410696 → 0x4104ce, key 0x66665c, version 9):**
  - house, number, budget, architecture value, friends, flags;
  - the member list, only when the ID is not 0 (0x4105e7);
  - the secondary house;
  - the two magic words (0x410a87);
  - 6 shorts from +0x14e.

  A family with ID −1 is skipped. The save always writes version 9, so
  older records are upgraded first.
- **FAMs:** a version −3 `STR#` (0x4705f8 / 0x470256 / 0x471234,
  writer 0x470696). It holds one entry: language 1, the name, and an
  empty comment.
- **FAMh:** if the family has a source `.FAM` file (+0x140), its FAMh is
  copied raw. Otherwise, if it has a record (+0x144), the record is
  written with key 0x66664c, version 1. Otherwise there is no FAMh.
- **TATT (0x43e0f0):** a packed byte (1), then packed fields (0x4644f5)
  for the count and, per entry, the GUID, the word count, and the
  shorts (0x464885).
  - The widths are i16 {5, 8, 13, 16}, i32 {6, 11, 21, 32} and
    u8 {2, 4, 6, 8}.
  - 0x46465b picks the first width whose sign mask fits.
  - Zero is one clear bit. Any other value is a set bit, a 2-bit
    selector, then the value MSB first.
  - The last partial byte is zero.

## Uninitialized bytes

The original leaves two kinds of bytes unwritten, so they hold whatever
was in memory:

- the even-padding byte after each odd-length neighbor name. The string
  writer 0x464c3e only advances past it (0x464cd7);
- one byte after each FAMs entry. 0x470696 sizes each string with two
  extra bytes but advances only one past the language word.

In the captures these bytes are not constant. `Uninitialized` carries
them, read from an earlier file or zero by default. The resource map's
unused bytes are carried the same way.

## Resource map type order: recorded, not traced

The chunks are written in the order above. But all three captures list
the map's types as FAMh, FAMs, NGBH, TATT, NBRS, FAMI. Our reading of
the map writer, 0x41ac25, is that it writes the type vector in order.
Our reading of 0x418d30 is that it appends each new type (0x41a793).
Neither reading explains the recorded order: a move-to-front list and a
hash order don't fit either. The writer therefore takes the type order
as an input, and the save uses the recorded order as the constant
`TYPE_ORDER`. Within a type, entries are in insertion order.

## What the store doesn't keep

`StoredNeighborhood` is postcard-encoded with migrations, so this change
adds no fields to it. A save needs things the store doesn't keep:

- the FAMh payloads;
- the uninitialized bytes and the map's unused bytes;
- the NGBH option bytes as read. The original saves each option byte as
  it holds it, and a read keeps any nonzero byte; the store keeps only
  each option's truth. A read byte is written while its truth matches the
  stored option;
- the family names as read. The store keeps them trimmed; a read name is
  written while it trims to the stored one.

They go in a separate `SaveInputs`, which `SaveInputs::from_saved` reads
from an earlier save. The store now keeps them in their own table, with
no record migration
([save inputs in the store](20261124-neighborhood-save-inputs-and-active-trait.md)).

## Family 0

The original reads and writes family 0 without a member list (0x4105e7).
`sims_content::Neighborhood::open` reads every FAMI with one, so family
0's secondary house is taken for a member count. The store's import now
reads family 0 again as the original does (`Family::for_chunk(0)`), and
the save writes the stored record as it is. `NEIGHBORHOOD_FORMAT` is now
6: the v5 → v6 migration reads family 0 again the same way. That is exact
even after a world writeback, because `Family::upgrade_to_v9` only
appends zeros to a version 9 tail, and the first 16 bytes after the
fixed fields are what the original read. The layout is format 5's, so a
format-5 build refuses a format-6 store rather than misreading it.

Two kinds of format-5 record can't be recovered exactly. The migration
still reads them, but the result isn't what the original read:

- **Rewritten members.** A world writeback can replace family 0's
  misread "members" with the live member list. The live list drops GUIDs
  that aren't in NBRS (`neighbors.rs`), and those GUIDs were really the
  secondary house and magic words. Once the writeback has saved the
  rewritten list, the bytes are gone.
- **Non-canonical v8.** A version 8 family 0 followed by bytes the
  original never writes gains magic words from them.

The project hasn't shipped, so only development stores can hold these
records. Importing the folder again restores them.

`Neighborhood::open` now reads FAMI by chunk ID itself, and the import's
re-read is gone
([save inputs in the store](20261124-neighborhood-save-inputs-and-active-trait.md)).

The lot-dependent updates in steps 2 and 5, and 0x427bd7, are
writeback, not serialization. `StoredNeighborhood::write_back_world`
does them before the save.

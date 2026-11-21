# An opened neighborhood boots from memory

*Decided 2026-11-20.*

Step 6b wires the `Patch` import into boot.
`sims_vm::neighborhood_open::open_neighborhood` opens a neighborhood as the
original does before its view first shows: house 0's reload, then the
import (`sims_sim::patch_import::open`). It writes the files the import
makes. `Lot::load_in` then loads a lot with that neighborhood already open.

**Files go to an output copy.** The original writes into the
neighborhood's own folder:
- each made person type as `Characters/User%05d.iff`;
- each housed family's `*.FAM`, with its GUID table rewritten in place, as
  `HouseNN.iff`.

The open writes them into a caller's `output` folder instead, a copy of the
neighborhood, and does not touch the install.

**The neighborhood stays in memory.** There is no `Neighborhood.iff`
writer yet (save 0x4252cc). The opened neighborhood is returned and handed
to `Lot::load_in`, not saved and read back. Neither of these is ported,
and neither changes the state a lot loads:
- removing the imported `*.FAM` files (0x429a75 to 0x429a82);
- the portraits (BMP_ 2002 to 2007, 0x4e8c5d).

The pools (`20261120-neighborhood-open-fills-five-pools.md`, 6b E) are
not ported either, and they do change it: their families, records and
person files enter the loaded world. They also set the neighborhood's
option bytes, which the lot load does not read (`World::read_neighbors`
ignores them), so the boot's hash does not check them.

**The converted flag.** The resource writer sets a resource's flags
word to 0x10 when it marks it converted (0x419f80), and to 0 otherwise.
The rewritten Gtab #1 gets 0x10. The original's five house files after
its UserData2 import (House 56, 63, 69, 76 and 77) all have it. The GUID
pairs are appended at 0x429754 and the write starts at 0x4297f7.

**Residue bytes.** The in-place rewrite leaves some bytes as the original's
heap held them (`sims_formats::iff_write`). A caller may give the
original's file for a house, whose bytes are used there. Otherwise they
are 0. The lot load reads none of them. With the original's bytes, all
five house files match it byte for byte.

**Inventories.** Reading lot and family information gets or creates an
empty inventory for each member of the family on that lot (0x4e1d8e, then
0x410282, then 0x42a8a1). It does so only when the lot's house file opens
(0x4e1e04). The open does this at two points, in this order:
1. **The view's set-up.** With Unleashed (0x4f5b42), its view (0x55693b)
   takes the lots in `LotZoning.iff` row order, up to the first above 80
   as a signed double word (0x557125). In UserData2 this gives 48 and 47.
   Without it, the other view takes lots 1 to 10 (0x55ecf8, 0x55ed9f).
2. **After each housed import's save** (0x429a97, 0x429ac9), for its
   destination lot. House 0 is skipped. In UserData2 this gives 294 to 313.

Details:
- The family is the first one whose house (+0x110) or second house
  (+0x13c) is the lot (0x425d95). Two matches fail loudly, since the order
  of our families is not the original's vector order.
- A member whose GUID is 0 (0x425723) or has no record is skipped.
- A family of version 8 or later too short for its second house fails,
  as the reader (0x4104ce) rejects it.
- The lot information's read (0x4e261f), whose failure would skip the
  family, is taken to succeed.
- Before each import, the scan shows the file's family (0x42bfca to
  0x4e29f5). That asks after its member GUIDs among the records
  (0x4e2a99), which can make inventories too. It is not ported: a member
  GUID that has a record fails. None of the six UserData2 files has one.

The scan reads the `Patch` files only when an import is pending
(0x42c1d5), so a stale file is not parsed otherwise.

`an_opened_neighborhood_matches_the_originals_import` (in
`crates/sims-vm/tests/boot.rs`) opens the install's UserData2 with the
GUIDs the original drew in track 2's second run (`SIMS_UD2_AFTER`,
`ud2-import-20261119/after-run2`), read from its `User00002.iff` to
`User00029.iff`.
It checks:
- the six imports;
- the five house files, byte for byte;
- the inventories, against the original's `Neighborhood.iff`.

It then takes the pools from the original:
- families 2000, 3000 to 3009, 5000, 5500 and 6000;
- records 322 to 444;
- files from `User00030.iff` on;
- the option bytes.

It loads House 69 with the C runtime where the House 69 test starts it
(`OPEN_DRAWS`, `20261120-ud2-house-69-boots-from-the-originals-import.md`)
and checks the hash against `IMPORTED_69`, the hash the original's own
files boot to. It skips without `SIMS_INSTALL` or `SIMS_UD2_AFTER`.

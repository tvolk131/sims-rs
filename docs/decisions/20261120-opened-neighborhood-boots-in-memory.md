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
to `Lot::load_in`, not saved and read back. These are not ported either,
and none of them changes the state a lot loads:
- removing the imported `*.FAM` files (0x429a75 to 0x429a82);
- the portraits (BMP_ 2002 to 2007, 0x4e8c5d);
- the pools (`20261120-neighborhood-open-fills-five-pools.md`, 6b E).

**The converted bit.** The rewritten Gtab #1 carries flag 0x10. The
original's five house files after its UserData2 import (House 56, 63, 69,
76 and 77) all have it. The address that sets it is not found.

**Residue bytes.** The in-place rewrite leaves some bytes as the original's
heap held them (`sims_formats::iff_write`). A caller may give the
original's file for a house, whose bytes are used there. Otherwise they
are 0. The lot load reads none of them. With the original's bytes, all
five house files match it byte for byte.

**Inventories.** Reading lot and family information gets or creates an
empty inventory for each member of the family on that lot (0x4e1d8e, then
0x410282, then 0x42a8a1). The open does this at two points, in this order:
1. **The view's set-up** (0x55693b), over the lots in `LotZoning.iff`
   row order, up to the first above 80 (0x557125). In UserData2 this gives
   48 and 47.
2. **After each housed import's save** (0x429a97, 0x429ac9), for its
   destination lot. House 0 is skipped. In UserData2 this gives 294 to 313.

Details:
- The family is the first one whose house (+0x110) or second house
  (+0x13c) is the lot (0x425d95). Two matches fail loudly, since the order
  of our families is not the original's vector order.
- A member whose GUID has no record is skipped.
- 0x4e1d8e's failure branches are taken not to happen.

`an_opened_neighborhood_matches_the_originals_import` (in
`crates/sims-vm/tests/boot.rs`) opens the install's UserData2 with the
GUIDs the original drew, read from its `User00002.iff` to `User00029.iff`.
It checks:
- the six imports;
- the five house files, byte for byte;
- the inventories, against the original's `Neighborhood.iff`.

It then takes the pools from the original:
- families 2000, 3000 to 3009, 5000, 5500 and 6000;
- records 322 to 446;
- files from `User00030.iff` on;
- the option bytes.

It loads House 69 and checks the hash against `IMPORTED_69`, the hash the
original's own files boot to. It skips without `SIMS_INSTALL` or
`SIMS_UD2_AFTER`.

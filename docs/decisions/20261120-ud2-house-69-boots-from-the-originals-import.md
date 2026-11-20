# UserData2's House 69 boots from the original's own import

*Decided 2026-11-20.*

Step 6a's lot is House 69, the Charmings, in the second neighborhood
(UserData2). The original enters it only after its first open of that
neighborhood. That open imports the `Patch` families and fills the pools
(`20261120-neighborhood-open-fills-five-pools.md`). The pools draw
from the wall clock, so no port can make the same files today.

`an_imported_home_loads_from_the_originals_files_and_steps` (in
`crates/sims-vm/tests/boot.rs`) therefore boots from the original's own
files.

- **The state.** `SIMS_UD2_AFTER` names the folder that holds
  `Characters`, `Houses` and `Neighborhood.iff` as the original's open left
  them. The test copies the install's `UserData2` into its temporary
  directory. It replaces `Characters` and `Houses` with that folder's, takes
  its `Neighborhood.iff`, and empties `Patch`, as the import does. It keeps
  the install's other files (for example `LotZoning.iff`). Without
  `LotZoning.iff`, lot 69 is not residential, and the load stops at
  0x4285e2. With either variable unset, the test is skipped. No game data
  is committed.
- **The folder used.** Track 2's second copy
  (`ud2-import-20261119/after-run2`). It was taken in the session that
  recorded House 69 (`house69-ud2-ticks-20261120`), after the import's
  last write (`Neighborhood.iff`) and before the lot load wrote anything.
  Its `User00000.iff` and `User00001.iff` match the `before` copy. That
  run's import made 151 people, so the folder holds 153 person files.
  The first run's copy (`after-with-lot-load`, repaired as
  `cache/ud2-import-state-20261120`) is no longer used: its GUIDs and pool
  rolls are another session's, which no recording covers.
- **The C runtime.** Nothing seeds the C runtime's generator between
  the launch and this load. The recording's entry state is state 1 after
  405,850 draws, and the UD1 House 69 recording's, from a launch straight
  into the lot, is state 1 after the load's own draws. The open's import
  and pools draw from it before the lot loads, and that is not ported
  (6b E). So the test starts it where the open left it, through
  `Lot::load_with`: state 1 after 401,120 draws (`OPEN_DRAWS`). The load
  makes the last 4,730 itself. The test checks the entry state.
- **What it checks.**
  - The load finishes.
  - The C runtime's state at entry is the recording's.
  - The records number 444.
  - Family 13 is in House 69 with the GUIDs the import drew.
  - The pools hold the original's counts, including the ten tourist
    families' sizes.
  - The world's hash at entry (`IMPORTED_69`) and after 1,000 ticks
    (`IMPORTED_69_1000`) matches its pin.
- **Dialogs.** Every dialog answers 2, as in track 2's recordings.

**Compared.** Track 2 recorded House 69 from this snapshot for 1,000
updates (`house69-ud2-ticks-20261120`, ticks 227245 to 228244). Every
dialog closed itself with result 2. Our run matches the recording at entry
and at every update through 1,000: the tick, the C runtime, the lot and
global streams, the globals and every object. There are two exceptions,
both of a kind the UD1 House 69 check also has:
- Object 2's last result.
- One word of a person's frame block. From update 112 to 157, the
  Charmings' first member (object 274) runs tree 8292 in its seventh
  frame. Its one local is 5623 in the original and 0 here. That local is
  word 77 of the person's frame block. Our block reached only 68 words
  before this frame, so nothing had written word 77. The original's holds
  whatever its heap left there. The UD1 recording shows the same: the same
  person (object 274 there too) in the same frame from update 112, with
  3244.

So `IMPORTED_69` and `IMPORTED_69_1000` are the recording's states, apart
from those two.

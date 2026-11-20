# UserData2's House 69 boots from the original's own import

*Decided 2026-11-20.*

Step 6a's lot is House 69, the Charmings, in the second neighborhood
(UserData2). The original enters it only after its first open of that
neighborhood. That open imports the `Patch` families and fills the pools
(`20261120-neighborhood-open-fills-three-pools.md`). The pools draw
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
- **The folder used.** Track 2's snapshot (`ud2-import-20261119`,
  `after-with-lot-load`), with `User00000.iff` and `User00001.iff` from its
  `before` copy. An unrecorded House 69 load after the import rewrote those
  two. On this machine the folder is `cache/ud2-import-state-20261120`.
  Its README says how it was made, and a digest list sits beside it. The
  earlier copy, `after-partial-0607utc`, was taken before the fifth box
  (Vacation) and is not used.
- **What it checks.**
  - The load finishes.
  - The records number 446.
  - Family 13 is in House 69 with the GUIDs the import drew.
  - The pools hold the original's counts.
  - The world's hash at entry (`IMPORTED_69`) and after 1,000 ticks
    (`IMPORTED_69_1000`) matches its pin.
- **Dialogs.** Every dialog answers 2, as in track 2's recordings.

**Unverified:** the two pins. They hold only this port's own state.
Track 2 is redoing the UD2 House 69 recording, which has to come from a
fresh import, so its GUIDs will differ from this snapshot's. When it
lands, the test moves to that recording's after-import files. It then
checks the entry C runtime state and the per-update lot state against
the recording, as the UD1 House 69 check does.

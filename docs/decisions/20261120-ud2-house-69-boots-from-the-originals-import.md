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
- **The C runtime.** The open's import and pools draw from the C
  runtime's generator before the lot loads, and that is not ported (6b E).
  So the test starts the generator, through `Lot::load_with`, at state 1
  after 401,120 draws (`OPEN_DRAWS`). That number is fitted: the
  recording's entry state is state 1 after 405,850 draws, and our load
  makes the last 4,730. It is the effective offset before our load's
  steps, which include re-creating the indicators. That nothing reseeded
  the generator in between is inferred from the fit, not observed; the
  executable's `rand` can seed a thread from the time stamp counter
  (0x619e5a). The test checks the entry state.
- **Frames.** The test draws the recording's frames (`RECORDED_FRAMES`):
  after updates 0 to 2, 29, 59, 89, 101 and 102, then every 30th from 123.
  With the default frames, an object is made one update later (321, not
  320), and the lot stream falls one draw behind there.
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
dialog closed itself with result 2. With the recording's frames, our run
matches it at entry and at every update through 1,000 in what the
comparison (track 3's `cmp_house.py`) reads: the tick, the C runtime, the
lot and global streams, the globals, and every object's data, temps,
attributes, frames and person data. The comparison skips any frame local
that our port marks as starting on half of a pointer. Object 283's tree
4099 local 7 is one, from update 14 (2609 in the original).

The differences:
- Object 2's last result, which the UD1 House 69 check shows too.
- One word of a person's frame block. From update 112 to 157, the
  Charmings' first member (object 274) runs tree 8292 in its seventh
  frame. Its one local is 5623 in the original and 0 here. That local is
  word 77 of the person's frame block. Our block reached only 68 words
  before this frame, so nothing had written word 77. The original
  allocates the block without clearing it past its first four bytes
  (0x4750f0, 0x4750fc), and a new frame keeps the locals it finds. The UD1
  recording shows the same: the same person (object 274 there too) in the
  same frame from update 112, with 3244.

**Relationships**, which the comparison does not read, match too; we
compared them on their own. At update 599 objects 160 and 161 (two of
GUID 0xF3AAA62F) delete themselves, in both runs. The original's
registry then erases, from every remaining object, the first
relationship row naming the deleted ID (0x441223 calls 0x464fa2), so
objects 7 and 63 lose their rows toward them. Our deletion now does the
same (`crates/sims-vm/src/removal.rs`), and our relationships equal the
recording's at entry and at every update through 1,000. The original
reads each row through its pointer with no null check, so an empty row
before the match fails loudly here. This erasure moves the Goth and
Newbie tick pins, whose runs delete objects too.

So `IMPORTED_69` is the recording's entry, and `IMPORTED_69_1000` is our
run with the recording's frames at update 1000, which differs from the
recording only in object 2's last result and object 274's word.

# Neighborhood open: imported files, preview, pool rules and lazy skeletons

*Decided 2026-11-24.*

This continues
[the five-pool replay](20261120-neighborhood-open-replays-five-pools.md).
`sims_vm::neighborhood_open::open_neighborhood` now leaves the caller's
copy of the neighborhood as the original leaves its own folder, except
for `Neighborhood.iff`. It also checks two more pool rules against the
captures and loads `.ndx` skeletons on first use. Addresses are
`Sims.exe` 1.91.

## Imported family files are used up

After each Patch import (0x429947–0x429a82), the original does one of
two things:

- **Housed family** (house number at `[ebp-0x18]` nonzero): its
  `HouseNN.iff` is moved to `HouseNN.iff.tmp` (0x5b9988, suffix 0x6c281c),
  the family file is moved onto `HouseNN.iff`, and the `.tmp` is deleted
  (0x5b98d5). If the second move fails, the first is undone and the
  import returns −50.
- **Houseless family:** its file is deleted (0x429a75 → 0x5b98d5), and
  the result is not checked (0x429a87).

The port has already written each housed import's house file into the
output. So for every import it deletes the output copy's family file,
which leaves the same end state. Before the open writes any file, each
copy must be a plain file inside the output (after resolving links), not
the source file, and must hold the bytes that were imported; otherwise
the open is refused. A copy that is missing is accepted, as 0x429a87
accepts a failed deletion. The deletions (`std::fs::remove_file`) run
last, after every character file is written. So a refused open leaves
the copy's `Patch` folder as it was, and only an I/O error during the
deletions can leave some used up; the error names them. The UD2 open
test checks that the folder ends empty.

## The import preview gives members an inventory

The preview of a pending import (0x42bfca → 0x4e29f5) parses the
family (0x410305). It then counts the family's MagiCoins: 0x4e2a99 →
0x4e1b2b → 0x4e1b93 → 0x410282. That count looks up each member's first
record by GUID (0x42571f) and finds or makes that record's inventory in
the open neighborhood (0x42a8a1). This is the same member loop the import
runs afterwards. The port used to reject a member that already had a
record. It now runs the loop (`patch_import::count_magicoins`): an empty
inventory slot fails as at 0x42a8cb, and a missing one is appended. The
presentation (0x4e2ada–0x4e2b7c) is not ported.

## Celebrity sex and Townie specialties

- **Celebrities.** The person maker (0x42c649) takes the member's ordinal,
  which is the family's member count before it (the 0x42c4bc loop).
  Ordinals 0–6 get name table 0xa6 (default "Nora") and are female;
  later ones get table 0xa5 ("Nomad") and are male (0x42c6d7 `setle`).
  The flag reaches the editor's sex (0x42cc96 → 0x42cf5d → 0x4eaaba). The
  replay rejects a recorded celebrity whose sex disagrees with its place.
- **Townies.** The candidates are every registered definition of kind
  0x400, in registry order, with duplicates (0x42a9e8–0x42aa2f). In a
  new family, while it has fewer than ten members and candidates remain,
  each person is a candidate picked by a bounded draw (0x42ac96, on the
  game stream). A made candidate is erased (0x42acc1); a failed one is
  drawn again. Then, up to the target, a draw of five that gives 3 picks
  a remaining candidate (0x42ae75, 0x42ae8f, erased at 0x42aebe), and
  anything else uses the generic kind-0x200 template (0x42aecf). An
  existing family runs only the second phase. The game stream is not
  ported, so the replay checks only that each recorded template could
  have been the pick.

Both captured UD2 opens satisfy both rules.

## Skeletons load on first use

The pools' first actor takes the template's skeleton, and a second actor
takes `adult` or `child`. Each is looked up by name (0x5a0384). An entry
that is not loaded yet (flag `+0x30`) is loaded at 0x5a03cd → 0x5a3810.
That load opens the `.ndx` catalog (0x5b94ae) and reads it (0x59e60e →
0x59f087), which builds every skeleton in the file, each drawing
(0x59f107 → 0x598af7). It then registers the skeletons not already
registered (0x59f18c) and marks the entry loaded (`+0x2c`). The pools
now do the same, using the World's model of a lazy load
(`people::load_indexed_bcf_catalog_into`). The load's draws come just
before the actor's copy, and the copy uses the scales of the entry the
registry then holds: the first catalog to register a name keeps it. A
failed lookup is marked (0x5a389b).
`OpenedPools` reports the loads in the World's shape, for a caller to
carry on.

The startup state comes from the avatar registry: the eager catalogs
are loaded, and `.ndx` entries are not. In the Complete Collection,
`adult` and `child` come from `GameData/Animation/Animation.ndx`. Both
UD2 captures need them loaded before the pools. With them loaded
lazily, the first celebrity's load draws before its copy, and the next
pool GUID (`User00031`) differs (the test checks this).
`PoolInput::loaded_skeletons` therefore stays as a fitted input, now
meaning "loaded by earlier work", as a list in load order. Which earlier step loads them is
untraced. No capture loads a skeleton during the pools, so the lazy
load's draws there are **unverified** beyond sharing the World's model.

## Dialogs belong to the window

The release latch (0x5e45e1–0x5e45ed, the button window: on state 0
or 2 with `+0x169` set, it publishes the button at 0x6fe9a4 and fires)
and the dialog window's handling at 0x436c76–0x436c91 (which arms the
window through 0x523a76 with 40,000) are interface code. They decide
when the window hands the VM an answer. The VM already offers both
answer models: `Answers::replay` (every dialog, one-button notices
included, waits for an answer) and `Answers::choices` (one-button
dialogs answer themselves). So sims-vm needs no change. Matching the
window's timing belongs to the window track (sims-game and sims-ui).

## Not done here: saving `Neighborhood.iff`

The open still returns the neighborhood in memory. What the save
(0x4252cc) does is traced, for a follow-up:

- It creates a fresh IFF in a temporary file (0x465949) and writes:
  - NGBH #1 (0x464dbd with 0x4254f6, version 0x49);
  - NBRS #1 (0x43071e);
  - for each family in the family vector, FAMI, FAMs and FAMh
    (0x410696);
  - TATT #1 (0x43e0be);
  - the resource map (0x46b6d2).

  It then moves the temporary file onto `Neighborhood.iff`.
- The family vector stays sorted by number (`+0x114`) descending, then
  ID (`+0x10c`) descending (comparator 0x425209). It is re-sorted
  (0x42fe6f) on reload (0x424d31), when a family is made (0x425ee1),
  and in every pool wrapper. After-run2's order (4000, 6, 10, 12, 13,
  11, 1, 6000, 5500, 5000, 3009…3000, 2000, 0) is this sort.
- FAMI is always written at version 9 by the two-way serializer 0x4104ce.
  A family with ID −1 is skipped. The member list is written only for a
  nonzero ID (0x4105e7), so family 0 is four bytes shorter. Before the
  magic words, 0x410a87 raises them from the current lot's magic object;
  at open no lot is loaded.
- FAMs is a one-string table of the name (`+4`, 0x471234). After-run2's
  FAMs each end in one stray byte. FAMh is copied from the family's
  source (`+0x140`, the family file) or written from its record buffer
  (`+0x144`).
- The open changes TATT #1 (25 bytes become 107), which is a packed
  field stream; neither its new entries nor a packed encoder are ported.
  The pool family names ("Townie", "Sartre", …) come from name tables
  the replay does not model.

So a byte-for-byte save needs the TATT update, a packed encoder, and the
pool names and stray bytes carried as recorded inputs. That is a
separate change.

## What would reopen this

- A capture whose pools load a skeleton, which would test the lazy
  load's draw placement.
- Tracing the step that loads `adult` and `child` before the pools,
  which would replace the fitted `loaded_skeletons`.
- A recorded Townie or celebrity that breaks either rule, which would
  mean the reading of 0x42c6d7 or 0x42ac96–0x42aecf is wrong.

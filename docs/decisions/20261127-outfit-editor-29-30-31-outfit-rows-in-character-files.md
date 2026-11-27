# Outfit-editor routines 29, 30 and 31; edited outfit rows go into the character files

*Decided 2026-11-27.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-vm/src/primitives/generic.rs`, `crates/sims-vm/src/primitives/generic/outfit_editor.rs`
  - `crates/sims-vm/src/lifecycle.rs`, `crates/sims-vm/src/lifecycle/outfit_rows.rs`, `crates/sims-vm/src/travel.rs`, `crates/sims-vm/src/boot/save.rs`
  - `crates/sims-sim/src/neighborhood_store.rs`, `crates/sims-content/src/people.rs`
  - `crates/sims-game/src/neighborhood/mod.rs` (saves through a staging store)
  - `crates/sims-vm/tests/interpreter/outfit_editor.rs`
- **Track:** Wave 1 (travel and destination completeness), PR 2

## Context

#430 ported Generic Sims Call 24, the outfit editor on the running person, and listed two follow-ups:
- routines 29, 30 and 31, which run the same editor on another person;
- the rows the editor writes, which the port kept in the lot (`World::made_outfits`) while the original writes them into the person's character file.

Until now a lot saved the rows with itself and a trip carried them in its traveler snapshots. Any other lot read the person's file, so it never saw the edit.

## Decision 1: routines 29, 30 and 31 run the editor on the stack object

All three routines start the same way:
- They find the stack object by its place plus one in the lot's object table.
- They cast it to a person (0x635abe). Anything missing, off the lot, or not a person throws native error 0x1c.
- They load the editor (0x408b3b, 0x409315).

Temps 0 and 1 are still the running object's (+0x3a, +0x3c). Each routine's switch knows only categories 0 to 4: body, formal, swimwear, pajamas and winter. None has high fashion.

| routine | reads | commit | temp 1 |
| --- | --- | --- | --- |
| 29 (0x4501bd) | as routine 24: when temp 1 ≥ 0 and below the count, keeps the current place and writes entry temp 1 | 0x409492(1) at 0x450391 | the kept place (0x4503e3); left as it was when nothing was written |
| 30 (0x450528) | the current accessor (0x40d0a4, 0x40d52f, 0x40d847, 0x40db5f, 0x40de77) before the commit; -1 for another category | 0x409492(1) at 0x45060c | that value (0x45064c) |
| 31 (0x4503f2) | the count accessor (0x40d077, 0x40d4ff, 0x40d817, 0x40db2f, 0x40de47), which is 0 for a null list; -1 for another category | 0x409492(0) at 0x4504e7 | that value |

A commit with 1 runs the clothing check (0x40a588) and writes the table if it changed. A commit with 0 copies the backup the load took (+0xc, 0x47053e) back over the live table. For routine 31 that restores exactly what the load left, so it changes nothing.

The portrait refresh (0x45810c, or 0x4572af/0x4e90c9/0x4eb0cc for 29) and the control-panel mark (0x4ceceb(0x20000000)) are presentation.

**Correction.** The earlier module note said 30 and 31 both commit with 0. Routine 30 commits with 1 (0x45060c `push 1`), so its clothing check and file write do run.

**Unsupported, as in routine 24:**
- 29 and 30 for a person of a gender code the editor does not know (0x40d0a4 and the commit read a null body list);
- a pet's clothing categories in 29, 30 and 31 (lists 0x408d8e and kin are not traced for pets);
- in 31, the clothing categories of any non-human.

Routine 31's body count for an unknown gender is 0, since the native list is null.

## Decision 2: the rows go into the person's character file at the next store commit

The original keeps one body-string table for each person type: the definition's word 19, `STR#` 200 for a character, loaded once (0x457f60). The editor writes that table into the character file in two places:
- the load (0x409315) writes after each group of rows it fills (0x587bbd);
- a commit with 1 writes when the table differs from the backup (0x587aec, 0x587bbd = 0x470256 + 0x46fd8b).

So every lot after an edit reads the edited rows from the file.

**What the port does.** It writes the rows into the store's `NEIGHBORHOOD_CHARACTERS` file at each commit that saves a lot (`lifecycle::outfit_rows`):

| commit | rows written | character files read from |
| --- | --- | --- |
| lot save (`save_and_leave_lot`) | the lot's | the store |
| departure | the frozen home lot's | the destination snapshot (the same store revision) |
| abort | the reconciled home lot's | the home snapshot |
| return | first the visited lot's (everyone on it), then the reconciled home lot's for the people the first left out | the home snapshot |
| trip on | the lot being left (travelers and those left behind); the frozen home lot in the envelope drops their rows | the next destination's snapshot |

The writes go in the same checked REDB transaction as the lot and neighborhood, through the new `NeighborhoodStore::save_lot_neighborhood_and_characters_checked`.

**Saves through a staging store.** A lot booted from a restored save slot, and a lot a cheat changed through `Lot::restore`, save into an ephemeral staging store first, and the shell then commits the staged lot and neighborhood to the real store. The character files the staged save changed (its files before and after the save, compared) now go into that same checked commit. Without them the real store would get a lot stripped of rows its files never received.

**A lot that stays live after a save.** `save_and_leave_lot` leaves the lot live. After the commit, the rows written become the rows the lot loaded (`OutfitWrites::rebase` into `Program::people`), as the original's table is the file's. So a later edit back to an earlier row is still an edit and is written.

**A trip stages its writes before it boots the next lot.** Departure, return and trip on all compute their writes before loading the next lot's runtime, and boot it from the character files as they will be committed (`OutfitWrites::staged`). So everyone on that lot reads the edited rows from the file, including people the arrival makes later, such as a date's companion.

**Only edits are written.** A person's rows are compared with the rows the lot loaded for that person (`Program::people`, as `read_character_row` reads a profile). Only the rows that differ are set into the file's table. A person whose rows all equal the loaded ones counts as written without a write. So a lot that only holds rows it read cannot put them back over a newer edit another commit wrote to the file.

**Which people.** A person is written only when:
- the lot's winning definition for it came from the store's `Characters` mount (`Program::characters`), so a later mount (Downloads, 0x43c609) that shadows the GUID is never written behind;
- it is not a type made on a lot (`characters.made`), whose own strings come before `made_outfits`.

**Travelers carry an empty set.** Each traveler whose file now holds its rows carries an empty set of rows in its snapshot (`OutfitWrites::carried`). The empty set replaces whatever rows the lot it arrives on held from before, so:
- the person reads the file;
- an abort cannot write a departure's rows back over a newer edit made elsewhere during the trip.

**The visited lot's rows win at a return.** The return writes the visited lot's rows first. It then drops, from the live home lot as well as its save, the rows of every person those writes covered, before writing the home lot's own rows. So rows a home lot saved before the trip never undo an edit made while away.

**Snapshots overlay early.** The destination boot puts each traveler snapshot's rows (an empty set included) into the lot's `made_outfits` before anything on the lot runs, so people the arrival makes later, such as a date's companion, already read them.

**How a file is written:**
- The person's file is the one file whose default-variant `OBJD` of type 2 has the person's GUID.
- Its table (`OBJD` byte 0x26) is decoded as the person's string set loads it: US-English rows, padded to 35 (`sims_content::decode_body_strings`).
- The rows are set into the decoded table.
- It is written as the native writer writes a string set: format -3, each row in language 1 with an empty comment, one spare zero byte per row, the resource flagged converted (0x10). This is the same layout as the family import's `localized_strings`.
- The table keeps its id and label.
- Bytes the native writer leaves uninitialized (the map's alignment bytes) are written as zeros, which nothing reads.
- A table that already holds the rows is not rewritten.
- The rewrite is declined, and the person's rows stay in the lot, when:
  - the file's resource map cannot be used, or the written file's cannot;
  - the written file would read back any other resource differently (every type's resources, in the original's order, compared by id, flags, resource flags, label and data before and after);
  - another person definition in the file names the same table, which can hold only one set of rows;
  - the person is a type made on a lot (`characters.made`), whose own strings come before `made_outfits`.

**After the write.** The people whose files now hold their rows have them dropped from the saved lot: both `Saved::made_outfits` and its world. Otherwise an older lot save could put back rows older than the file when it reloads, since `World::made_outfits` is read before the profile.

The in-memory lot keeps its rows; they equal the file's.

## Deviations and follow-ups

- **Timing.** The original writes at the edit. The port writes at the next store commit. A lot abandoned without saving loses the edit in both, since the port's file is the store's. A crash between the edit and the next save loses it only in the port.
- **Characters made on a lot** (`characters.made`, `made_characters`) keep their rows in their own strings, which shadow `made_outfits`. They are not written here.
- **Types without a store file of their own keep their rows in the lot, as before:** types the install defines, files inside FAR archives, a GUID that two files define, and a table two people share.
- **Which store file.** The person's file is found by scanning the store's character files for the one default-variant person definition with its GUID. `Program::characters` says the winning definition is a store file; it does not say which one. So a GUID two store files define is left out, and so is one a member of a FAR archive in the store also defines, since the archive's definition may be the one that wins.
- **Legacy rows per person, not per row.** A return and a trip on drop all of the home lot's rows for a person the visited lot wrote, not just the rows it wrote. Only rows saved before this change can be lost this way: a non-traveler's unsaved row in an old frozen home lot, for a person the visited lot also edited. This is part of the one-time migration below.
- **Concurrent edits of the same row.** Two lots that both changed the same row of the same person before either commits: the later commit wins, as the original's later write does. A trip's edit and a home edit made during the trip resolve the same way at the return (the visited lot's).
- **Legacy home rows at a return** for people who did not travel are written after the home runtime is booted from the files; the home lot's own `made_outfits` still holds them, so its reads match the file.
- **Rows the deletion epoch should purge.** A deleted visitor's rows are not purged from the lot. They are written only when a store file still defines that GUID.
- **Rows the load fills.** 0x409315 also writes the groups it fills (rows 30 to 34). Those land in `made_outfits` and are written with everything else at the next commit.
- **Old lot saves** made before this change still carry rows. The first save after loading one writes them into the file and drops them.
  - When another lot already wrote a newer edit for the same person, the older rows win. Nothing tells them apart; this is a one-time migration, accepted.
  - An old trip envelope's snapshots also carry rows, which its abort or return writes as the trip's.
- **Routine 31 for a pet's clothing** stays unsupported, as routine 24's does. The native list builder (0x4098f1) skips the sex filter for a non-human. Its pattern for pets is untraced.

## Verification

- Interpreter tests (`tests/interpreter/outfit_editor.rs`), all against a synthetic wardrobe:
  - 29 dresses the stack object from the running object's temps and knows no category 5;
  - all three throw 0x1c without a person on the stack;
  - 30 answers places, -1 and runs the commit's check;
  - 31 counts, answers -1 past 4, skips the check, and counts a null body list as 0.
- Unit tests (`lifecycle::outfit_rows`):
  - the write and its layout;
  - growing the table;
  - an equal table counts but is not written;
  - rows equal to the loaded profile count but are not written, and only the changed rows go in;
  - ambiguous (including an archive member), non-person, missing and unparsable files are skipped;
  - staging, carrying and chaining writes;
  - a table another definition reads, a person whose winning definition is not a store file, and a made type are left out.
- `sims-game`: a staged save carries only the character files it changed.
- Install-backed tests:
  - a Goth House 5 member's edited row survives a lot save in the store file, is gone from the saved lot, and is read back on re-entry and on another lot;
  - saving the live lot again after an edit back to the first row writes that row;
  - the date round trip's edited row reaches Mortimer's file at return;
  - the saved home lot keeps none of the row;
  - the home lot booted at the return reads the row from the file.
- `generic_routines`' `RESIDUALS` drops 29 and 30. The install calls them; it does not call 31.
- The Goth `TICK_43200` and all strict pins are unchanged. They never pass a store commit.

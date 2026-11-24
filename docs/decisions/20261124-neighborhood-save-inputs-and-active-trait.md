# Neighborhood save inputs in the store, FAMI #0 by chunk ID, and the Active trait in motive decay

*Decided 2026-11-24.*

This follows [the Neighborhood.iff writer](20261124-neighborhood-iff-writer.md).
It makes a stored neighborhood saveable to a folder from the store alone.
Addresses are `Sims.exe` 1.91.

## FAMI #0 is read by chunk ID

`sims_content::Neighborhood::open` now reads each FAMI with
`Family::parse_chunk(bytes, chunk.id)`. As the original does (0x4105e7),
it reads no member list for family 0. Before, the reader took family 0's
secondary house for a member count. That misread the record, and it
failed the open when the count ran past the chunk: for example, a valid
v9 family 0 with secondary house 5.

The store's import used to re-read family 0 (`Family::for_chunk(0)`).
That is now removed: on a record that was read correctly, the re-read
would shift its fields. The format 5 → 6 migration still repairs records
imported before
[the writer](20261124-neighborhood-iff-writer.md#family-0).

## Save inputs live in their own table

A save needs things the `StoredNeighborhood` record doesn't hold:

- each family's FAMh;
- the bytes the save leaves uninitialized;
- the NGBH option bytes as read;
- the family names as read.

These are kept as `SaveInputs` in a new REDB table,
`neighborhood_save_inputs`, keyed by neighborhood ID. The value is a
`SAVE_INPUTS_FORMAT` (1) header followed by postcard. A record field
would have changed `StoredNeighborhood`'s layout and every struct literal
of it, including those in sims-vm. A separate table changes neither, so
`NEIGHBORHOOD_FORMAT` stays 6 and `SNAPSHOT_FORMAT` is untouched.

- **Import** fills the table from the folder's `Neighborhood.iff`
  (`SaveInputs::from_file`). It reads the file once
  (`Neighborhood::open_with_file`), so the record, the inputs and the
  surnames come from the same file even if a save replaces it during the
  import. Only the imported families' entries are kept. A file the save wrote gives every input
  (`from_saved`). Any other file, such as a shipped one at an older NGBH
  version, gives what reads: FAMh, one-entry FAMs names with their last
  byte, and the option bytes.
- **Re-import** replaces the entry, and **delete** removes it.
- A neighborhood imported before this change has no entry. Its save then
  writes no FAMh and zeros for the uninitialized bytes.
- `NeighborhoodStore::save_save_inputs` replaces the entry. Callers use
  it, for example, when a family moves in from a `.FAM` file: when the
  family has one (+0x140), 0x4107a8 copies that file's FAMh. Only
  otherwise does 0x4107cf test the stored history (+0x144).
- When a commit drops a family, its entries go too: its FAMh, its name
  and its name's stray byte. A family that later takes the same ID then
  gets none of the old family's. A commit that adds a family leaves its
  entries alone, so a caller may store a moved-in family's FAMh before
  or after committing the family.

## Saving to a folder

`StoredNeighborhood::save_neighborhood_iff(directory, inputs)` writes the
file. `NeighborhoodStore::save_neighborhood_iff(id, directory)` does the
same with the stored record and inputs. Both follow the end of 0x4252cc:

1. The chunks are written into `NTemp.iff` (0x6c2608) and the file is
   closed (0x46b6d2).
2. 0x46b6d2's result is not checked. 0x425474 tests the status the save
   kept, `[ebp-0x10]`. If it is 0:
   - 0x46b42a deletes `Neighborhood.iff` (0x425481). A result of 0 or
     −0x28 ("not found") continues; any other result is returned.
   - 0x5b9988 moves `NTemp.iff` onto the name (0x4254a2). If the move
     fails, the save returns −1.
3. Otherwise 0x46b42a deletes `NTemp.iff` (0x4254bd).

The port renames `NTemp.iff` over `Neighborhood.iff` in one step. That
leaves the same files, and the folder is never without a
`Neighborhood.iff`. An existing file whose name differs only in case is
replaced under its own name. A `NTemp.iff` left by an earlier save is
deleted first and the new one is created exclusively, so a link under
that name is never written through. A failed write or rename deletes
`NTemp.iff`. `NeighborhoodStore::save_neighborhood_iff` reads the record
and the inputs in one read transaction. Like the original's fixed
`NTemp.iff`, the save assumes one writer per folder.

Saving each of the three UD2 captures from the store gives the recorded
`Neighborhood.iff` byte for byte. Each was imported, and then saved from
the stored record and stored inputs alone.

## Motive decay reads the Active trait (word 3), not Neat

`sims-sim`'s motive decay named person data word 3 `NEAT`. The
character-file writer and the `.antics` lifter put Neat at word 7 and
Active at word 3. In the decay (0x420fb6), the person's data is at +0x58c
(word 65, the kind, is +0x60e at 0x420fc4):

- 0x421054: `movsx eax, word [esi+0x592]`, which is word 3. It is
  multiplied by `[edx+0x6e49b4]`, has `[edx+0x6e49c0]` added, and the sum
  is subtracted from comfort (+0x18).
- 0x421093: `movsx eax, word [esi+0x598]`, which is word 6 (Outgoing). It
  is used the same way for social, but only while awake.

The two comfort rates are the tuning constants "comfort decrement lazy"
(Active 0) and "comfort decrement active" (Active 1000). So the code
reads the right word. Only the constant's name was wrong: it is now
`ACTIVE`. Behavior is unchanged, and the Goth TICK_43200 hash `df6eb544…`
stays.

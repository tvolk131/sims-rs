# Neighborhood REDB Persistence & Lot Lifecycle

*Decided 2026-11-21.*

Multi-neighborhood REDB persistence and lot lifecycle operations (import,
multi-lot save & leave round-trip, Move-In, Evict, and Bulldoze) now live in
`sims-sim::neighborhood_store` and `sims-vm::lifecycle`, verified against
`Sims.exe` 1.91 disassembly and Complete Collection assets without modifying
any install files.

## REDB Store Layout (`sims-sim::neighborhood_store`)

`NeighborhoodStore` stores multiple imported neighborhoods in one REDB file
using five tables:

- `neighborhoods` (`&str -> &[u8]`): versioned `StoredNeighborhood`
  (`NEIGHBORHOOD_FORMAT = 1` little-endian `u32` + postcard payload) holding
  `FAMI`, `NBRS`, `NGBH` data/inventories/options, `TATT`, resolved
  `LotZoning`, lot row order, runtime-created `World::characters`, and a
  monotonic `revision`.
- `neighborhood_characters` (`(&str, &str) -> &[u8]`): raw immutable
  `Characters/*.iff` blobs imported from `UserData*/Characters`.
- `neighborhood_houses` (`(&str, u32) -> &[u8]`): raw immutable base
  `Houses/HouseXX.iff` blobs imported from `UserData*/Houses`.
- `neighborhood_lots` (`(&str, u32) -> &[u8]`): latest saved lot blobs
  (`sims_vm::boot::save::Saved::encode`) written on Save & Leave, Evict, or
  Bulldoze.
- `neighborhood_meta` (`&str -> &[u8]`): store metadata including the active
  neighborhood key (`"active"`).

`NeighborhoodStore::import_directory` reads `UserData*` strictly read-only and
commits all tables in a single REDB write transaction. To keep character and
house loading independent of `UserData*` on disk, `sims-content` adds
`Index::add_character_files` (reproducing the exact `0x5ba9bd` file-table hash
order of `Index::add_characters`) and `House::parse_bytes`.

## Native `Sims.exe` 1.91 Rules Verified

### Lot Valuation & Net Worth (`0x4253ff`, `0x4272c5`, `0x427f78`, `0x4282bc`)

- Total lot value (used for `FAMI.architecture_value` on save, lot purchase
  price on Move-In, and net worth during play):
  `lot_value + objects_value + trunc(architecture_value as f32 * mult)`,
  where `mult` is `Global.far` `FCNS #4` `"architecture value multiplier"`
  (`0x6e4a88` = `0.7f`, `0x3f333333`) using single-precision `f32` arithmetic
  matching the original's x87 24-bit precision mode.
- Verified with zero discrepancy (`diff == 0`) against shipped Complete
  Collection `UserData/Neighborhood.iff` `FAMI.architecture_value` vs
  `HouseXX.iff` `SIMI` on Houses 7, 56, 63, 69, 76, and 77.
- Family net worth outside live play (`0x4272c5` / `0x410271`):
  `family.budget + family.architecture_value`.

### Save & Leave Writeback (`0x428687` → `0x41615d` → `0x43fb1b` → `0x4252cc`)

1. Flush every live person on the lot into its neighborhood record via
   `people::save_to_record` (`0x459d8b` → `0x425a69`, revision 9 persistent
   fields).
2. Recompute lot statistics (`0x416085` → `0x444cb9` + `0x47b006`, reading
   `Build.iff` `STR#` 129–131, 133–135 and surface catalogs): updates
   `world.budget.objects_value`, `world.budget.architecture_value`, globals 22,
   23, and 37, and family flags bits 0 (`category 1` on lot) and 1 (`category
   9` on lot).
3. Write back `NGBH` (16 data words + global 26 → `data[2]`, inventories),
   `NBRS` (slots, person data, relationships), `FAMI` (budget,
   `architecture_value`, house, recomputed `friends` via `0x427bd7`, flags with
   bits 2 and 3 cleared, members, `magic_history` at `tail[4..16]`), `TATT`
   (`0x43e0be`), and runtime-made characters together with the encoded `Saved`
   lot blob in one atomic REDB write transaction.

### Re-Entry Reconciliation (`0x424806`, `0x428041`)

When `enter_lot` opens a house with a saved lot in `neighborhood_lots`, it
restores lot-local state (`Saved`) and reconciles neighborhood-wide state from
the latest `StoredNeighborhood`: reloads `NBRS`/`FAMI`/`NGBH`/`TATT`/made
characters, re-selects the current household, re-attaches every live person's
persistent fields (`0x4258a3`), and runs household selection (`0x4283cf..0x4285f9`)
so edits from other lots (e.g. Goth House 5 → Newbie House 7 → Goth House 5)
propagate deterministically.

### Move-In (`0x42731e`, `0x427f78`, `0x4282bc..0x4284f9`)

- Validates `family_id != 0`, non-empty `members`, `family.house == 0` (Family
  Bin), residential zoning (`zoning == 1`), unoccupied lot, and
  `family.budget >= lot_price`.
- Deducts `lot_price` into `family.architecture_value` (`budget = budget -
  lot_price + architecture_value; architecture_value = lot_price`), assigns
  `family.house`, clears flag bit 3 (`0x8`), and sets `FAMILY_FLAG_NEW_MOVE_IN`
  (bit 2, `0x4`).
- On first entry while `FAMILY_FLAG_NEW_MOVE_IN` is set: sets clock to
  `FCNS #4` `"new family start hour"` (`0x6e4a84` = `7:00`, `0x46d86f`), resets
  budget history & global 28 (`0x46d97a`), unpauses (`0x46d83a`), resets house
  usage history to `"fill value for layout history"` (`0x6e4a78` = `200`,
  `0x4165f3`), broadcasts `"reset lot"` (`0x444741`), refreshes visitors
  (`0x42a2b4`), and goes live.
- Ported `0x445501` (`Interpreter::select_next_resident`) in `end_of_update` so
  newly moved-in lots tick cleanly before and as household members arrive.

### Evict & Bulldoze (`0x42737e`, `0x4354d5`, `0x461a34`)

- Clears `family.house = 0` before loading the lot (`0x4273d1`), flushes live
  persons to `NBRS`, sets simulator funds to the evicted family's budget, and
  broadcasts `"evict lot"` (`0x444741`).
- Removes top-level removable objects matching `0x4354d5` (persons + valued
  furniture); when `bulldoze = true`, also removes non-locked build objects
  (skipping off-lot global controllers `words[42] != 0`, objects with flag
  `0x10`, indicators, and objects on `tile_flags & 0x20` road/sidewalk tiles).
- On Bulldoze, zeroes non-`0x20` floor cells and clears wall cells via
  `ExpandedWall::put` (`0x4797e2`) across `1..(size - 1)` on levels 1–2, fills
  pool array `Arry #9` with `0xff` (`0x461a34`), rebuilds rooms, and removes
  home-bound un-familied tombstone character records (`word 67 == house`,
  `0x427697..0x4276ce`).
- Refunds `family.architecture_value` into `family.budget` (`0x427674`), zeroes
  `family.architecture_value` and `family.house`, resets lot funds/global
  26/budget history/usage, recomputes lot values (`0x416085`), and commits both
  the cleared lot and updated neighborhood atomically to REDB.

## Known Boundaries

- On newly moved-in empty lots (`House04`) where load/reset creates a person
  whose skin still awaits its first update (`0x590d26`), drawing a tick-0 frame
  hits `frame.rs`'s existing unported `0x590d26` guard; stepping with
  `Frames::parse("none")` or `Lot::step_play` steps cleanly.
- `Bulldoze` resets lots in place per `0x42737e` (preserving terrain elevation
  and locked road/sidewalk tiles) rather than copying `TemplateUserData` files.
- `cTSNeighborhood::Save` stray un-homed character deletion (`0x4287b9`) and
  `0x4276fb` `StreetNames` house-description clearing are omitted from REDB
  writeback.

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
  (`NEIGHBORHOOD_FORMAT = 2` little-endian `u32` + postcard payload) holding
  `FAMI`, `NBRS`, `NGBH` data/inventories/options, `TATT`, resolved
  `LotZoning`, lot row order, runtime-created `World::characters`, permanently
  deleted character GUIDs (`deleted_characters`), unique `instance_id`, and a
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
commits all tables in a single REDB write transaction, walking `Characters/`
recursively via `Neighborhood::character_tree` (including nested subdirectories
and `.far` archives). To keep character and house loading independent of
`UserData*` on disk, `sims-content` adds `Index::add_character_files`
(reproducing the exact `MountedFiles::scan` + `0x5ba9bd` file-table hash,
recursive directory / inline `.far` mounting, Windows-1252 key encoding, and
`sorted_entries` order of `Index::add_characters`) and `House::parse_bytes`.

## Native `Sims.exe` 1.91 Rules Verified

### Lot Valuation & Net Worth (`0x4253ff`, `0x4272c5`, `0x427f78`, `0x4282bc`)

- Total lot value (used for `FAMI.architecture_value` on save, lot purchase
  price on Move-In, and net worth during play):
  `lot_value + objects_value + trunc((Ext::from_i32(architecture_value) * F32(mult).ext()).single())`,
  where `mult` is `Global.far` `FCNS #4` `"architecture value multiplier"`
  (`0x6e4a88` = `0.7f`, `0x3f333333`) using exact 64-bit-significand x87
  extended multiplication (`31 + 24 = 55 <= 64` bits, zero double-rounding)
  followed by a single round-to-nearest-even to 24-bit `f32` and `__ftol`
  truncation toward zero.
- Pool pricing (`0x47b1cc`): `pool[cell] < 0x10` adds `prices.pool` and sets
  built (`globals[22] = 1`); `pool[cell] == 0x10` marks the lot built at zero
  cost.
- Verified with zero discrepancy (`diff == 0`) against shipped Complete
  Collection `UserData/Neighborhood.iff` `FAMI.architecture_value` vs
  `HouseXX.iff` `SIMI` on Houses 7, 56, 63, 69, 76, and 77.
- Family net worth outside live play (`0x4272c5` / `0x410271`):
  `family.budget + family.architecture_value`.

### Save & Leave Writeback (`0x428687` → `0x41615d` → `0x43fb1b` → `0x4252cc`)

1. Verify `lot.neighborhood_origin` matches the store's file-identity-bound
   128-bit `store_id` (combining persisted metadata with Unix `(dev, ino)` so
   copied `.redb` files never share a `store_id` while same-volume renames and
   reopens preserve it), `neighborhood_id`, and `instance_id`
   (`LifecycleError::NeighborhoodMismatch`), and matches the store's current
   `revision` both before serialization and inside the REDB write transaction
   (`LifecycleError::StaleNeighborhood`), preventing cross-store or copied-DB
   writes, same-ID re-import overwrites, or silent lost updates from stale open
   lots. `Lot::restore` clears `neighborhood_origin` so restoring an older
   snapshot cannot bypass the revision guard, every mutating store writer
   (`save_neighborhood`, `save_neighborhood_checked`,
   `save_lot_and_neighborhood`, `save_lot_and_neighborhood_checked`,
   `save_base_house`, `save_character_file`) strictly advances `revision` when
   mutating an existing neighborhood record, and `enter_lot`/`evict`/
   `bulldoze`/`lot_price` all read from a single MVCC REDB read transaction via
   `NeighborhoodStore::load_lot_snapshot`. `move_in` likewise commits via an
   in-transaction `(instance_id, revision)` check.
2. Flush every live person on the lot into its neighborhood record via
   `people::save_to_record` (`0x459d8b` → `0x425a69`, revision 9 persistent
   fields).
3. Recompute lot statistics (`0x416085` → `0x444cb9` + `0x47b006`, reading
   `Build.iff` `STR#` 129–131, 133–135 and surface catalogs): updates
   `world.budget.objects_value`, `world.budget.architecture_value`, globals 22,
   23, and 37, and family flags bits 0 (`category 1` on lot) and 1 (`category
   9` on lot).
4. Write back `NGBH` (16 data words + global 26 → `data[2]`, inventories),
   `NBRS` (slots, person data, relationships), `FAMI` (upgraded to version 9
   per `0x410696`, budget, `architecture_value`, house, recomputed `friends`
   via `0x427bd7`, flags with bits 2 and 3 cleared, members, and `magic_history`
   at `tail[4..16]` merged with signed max of live Magic Controller
   `0xb6c90029` attributes `0..6` per `0x410a87`), `TATT` (`0x43e0be`), and
   runtime-made characters together with the encoded `Saved` lot blob in one
   atomic REDB write transaction, advancing `lot.neighborhood_origin` to the
   committed revision.

### Re-Entry Reconciliation (`0x424806`, `0x428041`)

When `enter_lot` opens a house with a saved lot in `neighborhood_lots`, it
restores lot-local state (`Saved`) and reconciles neighborhood-wide state from
the latest `StoredNeighborhood`: includes persisted `made_characters` in the
definition registry and `Program::characters`/`character_files`, reloads
`NBRS`/`FAMI`/`NGBH`/`TATT`/made characters, re-selects the current household,
re-attaches every live person's persistent fields (`0x4258a3`), runs controller
reconciliation (`0x440328`) and object load visits (`0x43fc94`) when
`FAMILY_FLAG_NEW_MOVE_IN` is set so global controllers removed during
`evict`/`bulldoze` (including `VisitGenerator`, whose `"reset lot"` tree spawns
the incoming household) are recreated before household selection
(`0x4283cf..0x4285f9`), and propagates cross-lot edits (e.g. Goth House 5 →
Newbie House 7 → Goth House 5) deterministically. Base-house loads return an
error if `visit_objects` stops before finishing (`load.stopped`).

### Move-In (`0x42731e`, `0x427f78`, `0x4282bc..0x4284f9`)

- Validates `family_id != 0`, non-empty `members`, `family.house == 0` and
  `family.secondary_house() == 0` (`0x425d95`, Family Bin), residential zoning
  (`zoning == 1`), unoccupied lot (checking both primary `house` and
  `secondary_house`), and `family.budget >= lot_price`.
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
  newly moved-in lots tick cleanly before and as household members arrive, and
  ported the first-draw skin update (`0x5893a9` → `0x589361` at `0x58939e` →
  `0x590d26` → `0x5992b3`) in `frame.rs` so newly arrived residents draw and
  step cleanly with `Frames::default()`.

### Evict & Bulldoze (`0x42737e`, `0x4354d5`, `0x461a34`, `0x426fcd`)

- Clears `family.house = 0` (or `secondary_house = 0` if occupied as a
  secondary residence) before loading the lot (`0x4273d1`), flushes live
  persons to `NBRS`, sets simulator funds to the evicted family's budget, and
  broadcasts `"evict lot"` (`0x444741`).
- Evaluates every top-level registered object (including non-lead tiles of a
  multi-tile group, matching `0x42749a..0x427528`) against `0x4354d5` (persons
  + valued furniture); when `bulldoze = true`, off-lot non-controllers
  (`words[42] == 0`) are removed directly (`0x4274e2..0x4274e7`) and on-lot
  non-locked build objects without flag `0x10` on non-`0x20` road/sidewalk
  tiles are removed.
- On Bulldoze, zeroes non-`0x20` floor cells and clears wall cells via
  `ExpandedWall::put` (`0x4797e2`) across `1..(size - 1)` on levels 1–2, fills
  pool array `Arry #9` with `0xff` (`0x461a34`), rebuilds rooms, and removes
  home-bound un-familied tombstone character records (`word 67 == house`,
  `0x427697..0x4276ce`) along with their loose `NEIGHBORHOOD_CHARACTERS` files
  (`0x426fcd` → `0x46b42a`) and `made_characters` entries so reloads never
  recreate them.
- Refunds `family.architecture_value` into `family.budget` (`0x427674`), zeroes
  `family.architecture_value` and `family.house`, resets lot funds/global
  26/budget history/usage, recomputes lot values (`0x416085`), and commits the
  cleared lot, updated neighborhood, and character-file deletions atomically
  to REDB with an in-transaction revision check.

## Known Boundaries

- `Bulldoze` resets lots in place per `0x42737e` (preserving terrain elevation
  and locked road/sidewalk tiles) rather than copying `TemplateUserData` files.
- `cTSNeighborhood::Save` stray un-homed character deletion (`0x4287b9`) and
  `0x4276fb` `StreetNames` house-description clearing are omitted from REDB
  writeback.

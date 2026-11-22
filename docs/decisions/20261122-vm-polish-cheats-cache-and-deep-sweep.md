# Complete Collection VM Polish, Warm Lot-Entry Cache & 600-Tick Deep Expansion Sweep

*Decided 2026-11-22.*

## Context

Following PR #379 (Performance Part 4: VM hot loop, routing, paged CoW, and REDB V2 storage) and PR #374 (200-tick Complete Collection sweep burn-down to 0 unsupported), four high-value backend polish items remained in `sims-vm` and `sims-sim`:

1. **Native Live Cheats on `Lot`:** Host shells (`sims-game`) previously had to mutate a `World` snapshot and call `Lot::restore(...)` to adjust funds or clock hours (wiping Buy/Build undo history and `neighborhood_origin`), and had no native Buy Mode `move_objects` override.
2. **Warm Lot-Entry Latency (`enter_lot`):** Every `sims_vm::lifecycle::enter_lot` call rebuilt the immutable install-wide `Install`, `Index`, `Catalog`, `Program` libraries, `Animations`, `AvatarRegistry`, `People`, `Surfaces`, `Wardrobe`, sprite frame counts, and default character name from disk — and `boot_from_base_house` ran `People::load` a second time.
3. **Deeper 600-Tick Expansion Interaction Horizon:** The existing Complete Collection sweep ran 200 ticks per interaction. Extending representative expansion lots (`House05`, `House21`, `House40`, `House58`, `House81`, `House93`) to **600 strict ticks** exposes deeper multi-stage interactions (routing target deletions, neighborhood family-flag queries on phone/NPC scripts, and animation/viewport edge cases).
4. **Native Household Surname Persistence & Autosave SHA-256 Fast Path:** Create-a-Family (`CAF`) and neighborhood UI needed family surnames persisted inside `StoredNeighborhood` (eliminating external `.tsv` sidecar files), and `AutosaveStore` validators benefited from receiving the already-computed `payload_sha256: &[u8; 32]` alongside payload bytes without re-hashing.

## Decisions

### 1. Native Live Cheats on `Lot` (`crates/sims-vm/src/{boot.rs, buy.rs, interpreter.rs}`)

Added strictly additive live-lot APIs on `Lot` (all defaulting to strict, unmodified behavior):

- **`Lot::set_move_objects(&mut self, enabled: bool)` / `Lot::move_objects(&self) -> bool`** (default `false`):
  - Propagated only into Buy Mode `Interpreter` instances (`buy_start`, `buy_pick_up`, `buy_move`, `buy_turn`, `buy_rotate`, `buy_put_if_fits`, `buy_recheck`, `buy_sell`, `buy_commit`, `buy_undo`, `buy_redo`, `buy_snap_back`, `buy_cancel`), never into `advance_with_coop` / normal simulation ticks.
  - When `true`, `can_pick_up` and `can_delete` bypass `data[43]` buy/delete flags, in-use guards (`data[62] > 0`, `data[8] & 0x20`), and locked-tile (`tile_flags & 0x20`) checks; `buy_move` / `buy_put_if_fits` / `buy_turn` / `buy_recheck` try strict placement (and wall/pool auto-rotation) first, then allow a tile-centred ground placement override (`(x & 15) == 8 && (y & 15) == 8`).
  - **Invariants always preserved** even when `move_objects` is enabled:
    - All parts of single-tile and multi-tile objects must remain strictly inside lot interior bounds (`1..size - 1`) on `level in 1..=2` (validated both on preview/floor changes and again on commit, never mutating cursor coordinates on refusal).
    - Persons (`obj.person.is_some()`), ID `0`, missing objects, `world.indicators`, `TILE_CURSOR_GUID`, `WALL_CURSOR_GUID`, `COMEANDSEE_GUID`, system objects (`OBJD` word `TYPE == 7`), and global controllers (`OBJD` word `42 != 0`) are never pickable or deletable.
    - Overlapping two doors/portals onto the same tile is refused so `PORTAL_TILE` flags stay exact (and `take_off_one` preserves `PORTAL_TILE` if another portal remains on the tile).
    - `can_delete` still refuses surfaces carrying child objects (`has_child`).
    - Disabling `move_objects` mid-drag revalidates any override-placed preview before commit, while `buy_cancel` / `buy_snap_back` still restores captured overlapped origins cleanly so held objects are never stranded.
- **`Lot::add_funds(&mut self, delta: i32) -> Result<i32, OrderError>`:**
  - Applies `saturating_add(delta)` clamped to the exact lossless `globals[13..14]` script range `[-327_680_000, 327_679_999]` via `World::set_funds` directly on the live `World` (respecting budget mode `global[32] != 0`) without invoking `Lot::restore`, preserving Buy/Build undo history and `neighborhood_origin`. Refused while a co-op session is attached so peers cannot desynchronize.
- **`Lot::set_hour(&mut self, hour: u8) -> Result<(), OrderError>`:**
  - Rejects `hour >= 24`, updates `globals[0] = hour`, clears `globals[5]` and `globals[6]` (minute/second), updates `globals[4]` (time-of-day period matching `Sims.exe` `0x46d525`: `0..=5 => 2`, `6 => 3`, `7..=17 => 0`, `18 => 1`, `19..=23 => 2`), calls `world.mark_all_rooms()` when the period changes, preserves Buy/Build undo history and `neighborhood_origin`, and refuses mutation while a co-op session is attached.

### 2. Reusable `VmInstallCache` Across Lot Entries (`crates/sims-vm/src/{lifecycle.rs, program.rs, lib.rs}`)

Introduced an additive, thread-safe (`Clone + Send + Sync`) `VmInstallCache` (`Arc`-backed) plus `enter_lot_with_cache` and `enter_lot_with_cache_and_options` (keeping existing `enter_lot` and `enter_lot_with` untouched for 100% backwards compatibility):

- **`VmInstallCache::load(install_root: &Path)`** scans and links immutable install-wide assets once in parallel via `std::thread::scope`: `Install`, `ArchivePool`, base `Index`, base `Catalog`, base `Program` (Global, semi-globals, base objects, careers, text flags), `Animations`, `AvatarRegistry`, base `People` (+ `base_unresolved_people`), `Surfaces`, `Wardrobe`, sprite `icon_frames`, `NeighborhoodTuning`, and `default_character_name`.
- **Incremental `StoreRuntime::from_snapshot_with_cache`:**
  - Splices neighborhood `Characters` into a cloned base `Index` (`inserted_range`).
  - When no inserted character filename collides with any base file name, base qualified path, unresolved base semi-global (qualified or unqualified), `Global.iff`, `work.iff`, or `Careers.txt` (the common case for `User%05d.iff`), `splice_character_catalog` scans **only** the inserted `.iff` files (`Iff::scan_types` + `resource_order(b"OBJD")` + `GLOB`), splices them in exact `Index::resources` order, records duplicate-GUID replacement diagnostics, and shifts later base resource indices by `+delta`. Otherwise falls back cleanly to `Catalog::build_with_pool` and full `Program::link_with_pool`.
  - `VmInstallCache::load` defers any base `Program` or base `People` error to per-neighborhood full linking so a malformed base resource shadowed by a valid neighborhood `Characters` definition still opens identically to uncached `enter_lot`.
  - `Program::link_with_base_cache` clones shifted base `Library` entries, parses only `inserted_range` `.iff` files, and recomputes `careers` / `career_texts_end` if `work.iff` or `Careers.txt` is inserted.
  - `Animations` are reused directly unless an inserted character file is `.cfp` or `.bcf`.
  - `People` clones `base_people` + `base_unresolved_people` and calls `People::load_each_with_registry` only for `kind == 2` GUIDs whose winning catalog entry lives in `inserted_range`.
  - Also removed the redundant second `People::load` inside `boot_from_base_house` for both uncached and cached entry paths by reusing `&program.people` after verifying `unresolved_people` against saved object GUIDs (matching `boot.rs:1333-1344`).
- **Bit-exactness & measured `--release` latency (`House05.iff`, Complete Collection, Apple Silicon, 5-run median):**
  - `VmInstallCache::load` (one-time startup): **179.5 ms**
  - **Base `House05` `enter_lot`:** uncached **277.4 ms** → cached **60.4 ms** (**4.59× faster**, −217.0 ms / −78.2%)
  - **Saved `House05` `enter_lot`:** uncached **260.0 ms** → cached **56.1 ms** (**4.63× faster**, −203.9 ms / −78.4%)
  - Verified bit-identical `World::hash()` (initial and post-tick) between uncached `enter_lot` and `enter_lot_with_cache` across base-house entry, saved-lot re-entry, and CAF-created character households (`vm_install_cache_matches_uncached_enter_lot_for_base_saved_and_caf_households`).

### 3. Deeper 600-Tick Expansion Sweep & `Sims.exe` 1.91 Ports

Added `deep_expansion_600_tick_sweep` (`crates/sims-vm/tests/interaction_sweep.rs`) exercising **600 strict ticks** per interaction across six representative Complete Collection lots: Goth (`House05`), Downtown (`House21`), Vacation (`House40`), Old Town (`House58`), Studio Town (`House81`), and Magic Town Comm (`House93`).

The initial 600-tick baseline surfaced **54 `Trap::Unsupported` hits** across 4 distinct causes (`0` pie-menu unsupported). Disassembling unpacked `Sims.exe` 1.91 (`capture-00000020-00400000.bin`) resolved both non-presentation VM root causes (44 / 54 hits):

1. **Deleting an object a Sim is routing to (43 hits in Studio Town `House81` — `crates/sims-vm/src/{removal.rs, primitives/go_to_slot.rs}`):**
   - `Sims.exe` `0x440fff..0x44123c` (`delete_one`) never rejects deleting an object because another Sim has a `RouteContext` targeting it. `drop_references` (`0x441096` → `cXPerson::Reset(obj)` `0x45e791..0x45ecd9`) resets persons actively using or showing the deleted object's icon (clearing `route.contexts` at `0x45e8aa`), and any surviving route context whose target vanished transitions cleanly through `LOST` / `LOST_UNDER_WAY` (`0x45ae8d`, already ported in `crates/sims-vm/src/primitives/go_to_slot.rs:489`) on its next tick.
   - Removed the premature `Trap::Unsupported` guard in `crates/sims-vm/src/removal.rs` and stamped surviving route contexts targeting the deleted ID with `DELETED_ROUTE_TARGET` (`u16::MAX`, reserved/unallocatable) so immediate object-ID reuse cannot redirect a surviving route onto a newly-created object.
2. **Scope 24 fields 7 & 8: family flags bits 0 & 1 (1 hit in Goth `House05`, SCTC Cordless Wall Phone — `crates/sims-sim/src/{world.rs, neighbors.rs}`, `crates/sims-vm/src/{memory.rs, lifecycle.rs, boot/save.rs, coop.rs}`):**
   - `Sims.exe` `0x425c5e` (field 7) and `0x425cb4` (field 8) look up the neighbor record's family (`record.data[61]`), return `0` if missing or unhoused (`family.house == 0`), and otherwise return bit 0 (`0x1`, field 7) or bit 1 (`0x2`, field 8) of `family.flags` (`+0x138`, updated by `update_lot_values` at `0x4160e5`/`0x4160f9`).
   - Added `#[serde(skip)] pub family_flags: BTreeMap<i32, u32>` to `World` (preserving `World::encode()` and `World::hash()` bit-for-bit, keeping Goth 43,200-tick hash unchanged), populated in `World::read_neighbors`, refreshed in `lifecycle.rs::update_lot_values` and published back to the live lot after checked save commit, carried in `Saved` format `v10` (with explicit `v9 -> v10` migration so pre-v10 saves inherit loaded flags), included in `Lot::coop_world_hash()`, and read in `crates/sims-vm/src/memory.rs`.
3. **Remaining strict-mode presentation traps (10 hits: 8 Studio Town `House81`, 2 Old Town `House58`):**
   - All 10 remaining hits are intentional strict-mode viewport-culling guardrails (`animator byte +0x109`, `0x590b8b`, whether the camera framed the Sim on the last draw frame). Strict mode traps when drawn vs. undrawn animation branches diverge; Play Mode (`Lot::step_play`) handles them cleanly without faulting.

*Post-port 600-tick deep expansion sweep results (`deep_expansion_600_tick_sweep`, `--release`, 235.0 s, report written to `cache/memory/interaction-sweep-report.md`):*

| Lot | Objects | Entries | Clean | Unsupported (before → after) | Pie Unsup |
| --- | ---: | ---: | ---: | ---: | ---: |
| Goth (`House05`) | 254 | 294 | 294 | 1 → **0** | 0 |
| Downtown (`House21`) | 582 | 268 | 268 | 0 → **0** | 0 |
| Vacation (`House40`) | 354 | 214 | 214 | 0 → **0** | 0 |
| Old Town (`House58`) | 394 | 156 | 154 | 2 → **2** (`+0x109` only) | 0 |
| Studio Town (`House81`) | 347 | 153 | 145 | 51 → **8** (`+0x109` only) | 0 |
| Magic Town Comm (`House93`) | 660 | 154 | 150 | 0 → **0** | 0 |
| **Total** | **2,591** | **1,239** | **1,225 (98.9%)** | **54 → 10 (0 non-presentation)** | **0** |

### 4. Native CAF Surname Persistence & Autosave SHA-256 Validator Fast Path (`crates/sims-sim/src/{neighborhood_store.rs, migration.rs, autosave.rs}`, `crates/sims-vm/src/caf.rs`)

- **`StoredNeighborhood` v4 (`family_surnames: BTreeMap<u16, String>`):**
  - Bumped `NEIGHBORHOOD_FORMAT` from `3` to `4` with an explicit forward migration `v3 -> v4` (`StoredNeighborhoodV3` → `StoredNeighborhood` with empty `family_surnames`, `#[serde(default)]` for backwards-compatible serde).
  - Added `family_surname`, `set_family_surname`, and `remove_family_surname` helpers, automatic orphan pruning in `write_back_world`, native `FAMs` (`STR#` parsed via `StringTable::parse_chunk` respecting IFF byte-order flags, US English index `0`) import from `Neighborhood.iff` in `NeighborhoodStore::import_directory`, and automatic surname persistence/cleanup in `caf.rs` (`commit_family_with_context_hook`, `import_family_fam`, `delete_bin_family`). Note: adding the public `family_surnames` field on `StoredNeighborhood` requires struct-literal constructors to specify `family_surnames` (none exist in downstream workspace crates; all serde/REDB callers migrate transparently via `v3 -> v4`).
- **`AutosaveStore` SHA-256 Validator Fast Path:**
  - Added additive `commit_job_validated_with_sha256`, `list_saves_validated_with_sha256`, `load_save_validated_with_sha256`, `load_latest_intact_autosave_validated_with_sha256`, and `load_with_fallback_validated_with_sha256` passing `(&SavePayloadKind, &[u8; 32], &[u8])` with the already-computed `payload_sha256` digest across both V2 and legacy V1 records. All existing `*_validated` methods delegate directly to these helpers with zero breaking changes.

## Invariants Verified

- Exact Goth 43,200-tick hash unchanged: `df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a`.
- `unsafe_code = "forbid"`, zero new external crates, `clippy.toml` determinism compliant.
- Strictly additive public APIs; zero breaking changes to `sims-game`, `sims-antics`, or any other crate.
- Strict disjoint scope: only `crates/sims-vm/`, `crates/sims-sim/`, and this ADR touched.

## What would reopen this

- A `Sims.exe` 1.91 trace showing `move_objects` should additionally override a specific surface-child or wall-placement rule not covered by the bounds-checked ground fallback.
- If `sims-game` needs `VmInstallCache` to also pre-populate `Program::ordered_wardrobe` (matching `boot::load_ordered_wardrobe`) so `Make New Character` runs with native-ordered suit/texture lists on `enter_lot_with_cache` as well as `boot::load`.
- Porting camera-independent viewport-cull presentation mode for `+0x109` if strict-mode deep sweeps ever need to eliminate the final 10 presentation guardrails.

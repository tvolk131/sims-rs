# Performance Part 4 — VM Hot Loop, Routing, Paged CoW, and REDB V2 Storage

*Decided 2026-11-22.*

Following Performance Parts 1–3 (`20261121-frame-time-and-blit-perf.md`,
`20261121-vm-tick-and-two-tier-lot-cache-perf.md`, and
`20261121-startup-dedup-and-parallel-load.md`), a deep workspace audit identified seven
high-leverage, strictly **100% correctness-neutral** optimization groups in `crates/sims-vm`
and `crates/sims-sim` (Audit Items **1.1–1.6** and **4.4 `sims-sim` REDB storage**):

1. **Interpreter hot-loop churn (Item 1.1):** `Machine::step` unconditionally called
   `frame_mut()` on every BHAV instruction to clear `Frame::TREE_FLAG` (triggering CoW
   detaches on `asks = true` ticks), resolved `(code_owner, tree_id, &Behavior)` through
   multiple `BTreeMap` lookups on every instruction, performed redundant `me_mut()` writes,
   and re-resolved the current instruction inside `Machine::branch`.
2. **BHAV call & frame-storage overhead (Item 1.2):** `push_frame`/`pop_frame`/`write_mark`
   ran full `sync_frames()` over the entire call stack on every push/pop, `Machine::call`
   heap-allocated `instruction.operands.to_vec()` on every call, and `enter_as`/`enter_on`
   linearly scanned `world.types` to map `GUID -> lowest type ID`.
3. **Uncached `Program` tuning & curve re-parsing (Item 1.3):** `room_tuning`,
   `autonomy_constants`, and `score_curves` repeatedly scanned `FCNS` tables with
   case-insensitive string comparisons and re-parsed 9 piecewise-linear curve strings on
   every `Find Best Interaction` call.
4. **Routing A\* and spatial obstacle scans (Item 1.4):** `routing/search.rs` scanned
   `open`/`closed` `Vec`s linearly (`O(N^2)`), cloned `neighbors: Vec<usize>` on every
   expansion, and linearly scanned `nodes` to find `(goal, region)`; `routing/regions.rs`
   linearly scanned all dynamic/fixed obstacles and existing regions up to 200+ times per
   `expand()`; `routing/planner.rs` eagerly cloned `world.order` and performed repeated
   `world.objects.get(&id)` lookups per object.
5. **Catalog duplicate scans, 4× `BTreeSet` scratch pointers, `Set To Next`, and scheduler holes
   (Item 1.5):** `memory.rs` and `set_to_next.rs` scanned all ~3,000+ `catalog.objects`
   entries to detect duplicate GUIDs; every `Expression` and `Set To Next` mutated four
   separate `BTreeSet<u16>` fields on `World`; `set_to_next::next_in_order` re-validated all
   of `world.order` on every iteration (`O(N^2 log N)`); and `tick.rs` probed empty ID holes
   in `start..=end` and looked up ready objects twice.
6. **Paged dense `CowMap<u16, V>`, zero-alloc streaming SHA-256, CoW-shortcut `same_lot`,
   and single-buffer `Saved::encode` (Item 1.6):** `World::objects`/`frame_storage`/`arrays`
   paid `BTreeMap` traversal and full map clone on CoW detach; `World::hash()` and
   `coop_world_hash()` allocated temporary ~200–400 KB `Vec<u8>` buffers;
   `both_ways::same_lot` serialized both worlds to `Vec<u8>` instead of using `Arc::ptr_eq`;
   and `Saved::encode` copied its postcard payload into a second buffer.
7. **`sims-sim` REDB persistence bloat & double SHA-256 (Item 4.4):** `AutosaveStore`,
   `AlbumStore`, and `NeighborhoodStore` lot thumbnails stored uncompressed payloads and
   varint-encoded `Vec<u16>` RGB565 pixels (3 bytes per high-bit pixel, including magenta
   `0xf81f`), verified SHA-256 twice on autosaves, and lacked a fast header-only metadata
   listing path for Save Manager UIs.

## What was decided

All changes remain **100% safe Rust** (`unsafe_code = "forbid"`), **100% `clippy.toml`
determinism compliant** (zero `HashMap`/`HashSet`, zero platform float transcendentals),
introduce **zero new external crates** (`flate2.workspace = true` in `crates/sims-sim/Cargo.toml`
reuses workspace `flate2 = "1.1"`), preserve **100% public API compatibility** for downstream
crates (`sims-game`, `sims-render`, `sims-antics`), and keep every simulation state hash,
`coop_world_hash`, `Saved` V9 wire format, and the 43,200-tick Goth day hash
(`df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a`) **bit-for-bit identical**:

### 1. Interpreter Hot Loop & Public-API-Compatible `Program` Fast Paths (Item 1.1)
- Guarded `frame.tree = frame.tree_id()` in `Machine::step` (`crates/sims-vm/src/interpreter.rs`)
  behind `if self.frame()?.tree & Frame::TREE_FLAG != 0`, avoiding CoW detaches on read-only
  instructions during `asks = true` ticks.
- **Public API compatibility note:** Downstream tests in `sims-antics` construct `Program`
  via exhaustive struct literals and `sims-game` indexes `program.libraries` directly as
  `BTreeMap<usize, Library>`. Rather than breaking `Program`'s public layout, `Interpreter`
  caches `global_lib: Option<&'a Library>`, `Machine` caches `ActiveFrame<'a>`
  `(depth, code_owner_type, tree_id, owner_res, &'a Behavior)` + `CachedOwner<'a>`
  `(code_owner_type, owner_res, owner_lib, semi_lib)` (invalidated on every `push_frame` /
  `pop_frame` / `clear_frames` / re-entry / tree mutation), and `Lot` owns a private
  `ProgramFast` (`dense semi_globals: Vec<Option<usize>>` + `duplicate_guids: BTreeSet<u32>`)
  borrowed into `Interpreter::fast`. This eliminates `Program::libraries` `BTreeMap` lookups
  on BHAV step/branch and BCON scope-26 reads without breaking any downstream caller.
- Combined `me_mut()` writes in `Machine::step` and passed already-fetched `(on_true, on_false)`
  into a local-node fast path in `Machine::step` / `Machine::branch`.

### 2. Incremental `frame_storage` Sync, Stack Call Buffer, & GUID Reverse Index (Item 1.2)
- Added `Machine::sync_top_frame` alongside a `#[doc(hidden)] #[serde(skip)]`
  `World::dirty_frames: CowBox<BTreeMap<u16, u64>>` bitmask. Lower-frame cross-frame
  mutation sites (`idle.rs`, `go_to_slot.rs`, `notify.rs`, `memory.rs::wake`, `create.rs`)
  call `mark_frame_dirty(world, id, depth)` without touching `frame_storage` early; the
  object's next `sync_top_frame` flushes only those dirty depths plus the top frame at the
  exact tick boundary full `sync_frames` would have. `World::decode` and
  `Saved::hydrate_world_aux` call `World::mark_all_frame_storage_dirty()` so restored
  worlds perform one full sync on each object's first post-restore push/pop (preserving
  bit-exact parity if a lower frame was dirty before save). Verified against full
  `sync_frames` and across `World::encode`/`World::decode` in `interpreter::tests`.
- Eliminated `instruction.operands.to_vec()` in `Machine::call` by reading `&[u8]` directly
  for standard calls and copying into a stack `[u8; 20]` buffer for extended operand layouts.
- Added a `#[doc(hidden)] #[serde(skip)]` `World::types_by_guid: TypesByGuidCache` (`Arc`-backed
  for `O(1)` `World::clone()`, self-validating against direct external `world.types`
  mutations) with `World::insert_type` and `World::type_id_for_guid` returning the lowest type
  ID for a GUID (`enter_as`, `enter_on`).

### 3. Precomputed `Program` Tuning & Curve Cache (Item 1.3)
- Added `#[doc(hidden)] pub cached_tuning: Option<LibraryTuningCache>` on `Library`
  (`crates/sims-vm/src/program.rs`), populated once in `load_library_with_pool` (leaving
  synthetic `Library::default()` unit tests that mutate `.tuning`/`.strings` at runtime 100%
  live). Precomputes `room_tuning`, `autonomy_constants`, and `score_curves` for global tables
  501/503 and custom `words[51]` tables, and adds `Program::score_curves_ref -> Result<Cow<'_, [Curve]>, String>`
  used in `autonomy.rs` to eliminate per-scan curve re-parsing and `Vec` cloning.

### 4. Routing Spatial Grid, O(N log N) Exact-Tie-Break A\*, & Zero-Clone `planner.rs` (Item 1.4)
- Added `SpatialGrid` (`crates/sims-vm/src/routing/regions.rs`): 16×16 coarse grid of 64×64
  fine-unit (4×4-tile) buckets storing strictly ascending rectangle indices, with clamped edge
  cells and exact handling for negative, zero-width, and inverted rectangles. Preserves exact
  first-`HARD`-else-last-soft priority in `Space::obstacle` and minimum-index in
  `Graph::region_at`, plus `O(1)` `Space::nth`. Verified against brute-force linear scans in
  `routing::regions::tests::spatial_grid_matches_linear_scan_for_priority_and_edge_rects`.
- In `crates/sims-vm/src/routing/search.rs`:
  - Indexed `(goal, region)` in deterministic `BTreeMap<(Option<usize>, usize), usize>` via
    `.or_insert` (preserving first-match semantics when a goal starts in region 0).
  - Replaced `neighbors(&mut self, id) -> Result<Vec<usize>, Trap>` with zero-clone
    `ensure_neighbors(&mut self, id) -> Result<(), Trap>` and `Node::{is_goal, is_start}` flags.
  - Replaced linear `open`/`closed` `Vec` scans with dense `in_open`/`in_closed` `Vec<bool>`,
    monotonic `open_seq: Vec<u64>` (preserved across decrease-key, bumped only on fresh insert
    or re-open from closed), IEEE-754 total-order `float_sort_key` (normalizing `-0.0` to `+0.0`
    with exact fallback if any `NaN` enters `open`), and lazy-deletion
    `BinaryHeap<Reverse<(u32, u64, usize)>>`.
- In `crates/sims-vm/src/routing/planner.rs`: skipped dynamic obstacle loop when `through` is
  set, combined repeated `world.objects.get(&id)`/`datum()` lookups per object, and borrowed
  `&world.order` by index until the first object actually requiring an `ALLOW_OVERLAP`
  callback (then snapshotting only the remaining suffix).

### 5. Duplicate GUID Set, Compact `ScratchPtrs`, O(N) `Set To Next`, & Scheduler Cursor (Item 1.5)
- Precomputed `ProgramFast::duplicate_guids: BTreeSet<u32>` once from `catalog.objects`,
  turning O(|Catalog|) duplicate scans in `memory.rs` and `set_to_next.rs` into O(1) lookups.
- Replaced the four `#[serde(skip)]` `scratch_ptr_*` `BTreeSet<u16>` fields on `World` with a
  single compact CoW `ScratchPtrs` (`CowBox` with dense `Vec<u8>` up to 4096 + overflow
  `BTreeMap<u16, u8>`, per-bit counts, and read-before-write no-op guard) plus zero-alloc
  `ScratchPtrBitSerialize` adapter preserving bit-exact `coop_world_hash` bytes (verified by
  `scratch_ptrs_serialize_bit_matches_btreeset_postcard_bytes`).
- Cached `order_validated` and `last_order_cursor` on `Machine` (invalidated on any primitive
  outside the pure-query whitelist), turning full `Set To Next` scans from `O(N^2 log N)` to
  `O(N)` while preserving upfront validation semantics.
- Switched `tick.rs` scheduler loop to `world.objects.next_key_from(cursor)` and eliminated
  the duplicate `world.objects.get(&id)` lookup.

### 6. Paged `CowMap<u16, V>`, Streaming SHA-256 Flavor, CoW `same_lot`, & Single-Buffer `Saved::encode` (Item 1.6)
- Added `crates/sims-sim/src/hash_flavor.rs`: `Sha256Flavor<'a>` (256-byte stack staging
  buffer implementing `postcard::ser_flavors::Flavor`) + `hash_postcard`, used by
  `World::hash()` and `coop_world_hash()` for zero-allocation hashing (verified by
  `streaming_sha256_matches_to_allocvec_across_boundaries` and
  `world_streaming_hash_matches_encode_sha256`).
- Upgraded `CowMap<K, V>` (`crates/sims-sim/src/cow.rs`) with paged dense storage for `u16`
  keys via `CowKey`: 64-entry `Arc<Page<V>>` pages (`0..4096`) with `u64` occupancy bitmask +
  `overflow: BTreeMap<K, Arc<V>>` for `>= 4096`, `CowMap::ptr_eq_or`, `next_key_from`, and
  `range`, preserving exact ascending `serialize_map` postcard wire output and `NaN`
  `PartialEq` semantics.
- Rewrote `both_ways::same_lot` (`crates/sims-vm/src/both_ways.rs`) as a structural
  comparison using `CowBox::ptr_eq_or` / `CowMap::ptr_eq_or` `Arc::ptr_eq` short-circuits and
  bit-exact `f32::to_bits()` on `Object::{portal, person}` (verified by
  `same_lot_cow_shortcut_matches_encode_on_floats_and_scratch_ptrs`).
- Switched `Saved::encode` (`crates/sims-vm/src/boot/save.rs`) to single-buffer serialization
  via `postcard::ser_flavors::ExtendFlavor`.

### 7. Versioned V2 REDB Persistence with V1 Backwards Compatibility (Item 4.4 `sims-sim`)
- **`AutosaveStore` (`crates/sims-sim/src/autosave.rs`):** Bumped `AUTOSAVE_FORMAT = 2`
  (`AUTOSAVE_MIN_FORMAT = 1`). V2 splits `AutosaveHeaderV2` + raw deflate
  (`flate2::Compression::fast()`) payload in a single buffer, enforces a 128 MiB payload cap
  on both write (before transaction/pruning) and read, rejects trailing compressed bytes,
  verifies `payload_sha256` on full reads, preserves full V1 read compatibility, and adds
  `AutosaveStore::list_saves_metadata` (envelope SHA-256 + V2 header only — zero decompression).
- **`AlbumStore` (`crates/sims-sim/src/album_store.rs`):** Bumped `ALBUM_FORMAT = 2`
  (`ALBUM_MIN_FORMAT = 1`). V2 packs RGB565 pixels as raw little-endian bytes compressed with
  `Compression::fast()` + bounded deserialization/decompression and trailing-byte rejection;
  `list_summaries` validates V2 pixel streams via a 4 KiB stack discard buffer without
  retaining pixel buffers; V1 varint records remain fully readable.
- **`NeighborhoodStore` lot thumbnails (`crates/sims-sim/src/neighborhood_store.rs`):** Added
  `LOT_THUMBNAIL_MIN_FORMAT = 1`, bumped `LOT_THUMBNAIL_FORMAT = 2`, packing raw LE RGB565
  bytes with `Compression::fast()` + bounded decompression, trailing-byte rejection, and full
  V1 backwards compatibility.

## Before vs. After Benchmark (`House05.iff`, `--release`, Apple Silicon)

Measured on the same machine (`origin/main` `e6295101` baseline vs. `perf/vm-routing-and-storage`):

| Benchmark | Before (`e6295101`) | After (`perf/vm-routing-and-storage`) | Result |
| --- | ---: | ---: | ---: |
| 43,200-tick Goth full-day test (`strict_mode_runs_the_goth_houses_whole_day`, 3 runs) | `2.84s`, `2.80s`, `2.85s` (**median `2.84s`**) | `1.50s`, `1.52s`, `1.54s` (**median `1.52s`**) | **1.87× faster** (−46.5% wall time; ~2.1× pure tick throughput excluding ~0.30s load) |
| 43,200-tick Goth day SHA-256 hash | `df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a` | `df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a` | **Bit-for-bit identical** |
| `World::hash()` / `coop_world_hash()` temporary heap buffer | ~200–400 KB `Vec<u8>` per hash | `0 B` heap (256 B stack buffer via `Sha256Flavor`) | **Zero heap allocations** |
| `AutosaveStore` V2 vs V1 stored record size (64 KiB repetitive lot payload) | V1 uncompressed postcard | V2 `Compression::fast()` deflate | **>4× smaller** + zero-decompress `list_saves_metadata` |
| `AlbumStore` V2 vs V1 stored photo size (`128×96` RGB565 with `0xf81f`) | V1 varint `Vec<u16>` (up to 3 B/px) | V2 raw LE + `Compression::fast()` | **>3× smaller** + stack-validated `list_summaries` (zero pixel buffer allocation) |
| `NeighborhoodStore` lot thumbnail V2 vs V1 (`128×96` normal + roofs_off) | V1 varint `Vec<u16>` | V2 raw LE + `Compression::fast()` | **>4× smaller** |

## What would reopen this

- Adding any new `World` or `Object` field (especially raw `f32`/`f64`) must also update
  `both_ways::same_lot` (`crates/sims-vm/src/both_ways.rs`) to keep structural CoW-shortcut
  equality bit-exact with `World::encode()`.
- Adding any new primitive or VM routine that mutates a lower stack frame (`stack[depth]`
  with `depth < stack.len() - 1`) or another object's stack frame must call
  `crate::interpreter::mark_frame_dirty(world, target_id, depth)` so incremental
  `sync_top_frame` stays bit-exact with full `sync_frames`.
- Once Wave B (`sims-game`) updates Save Manager listing and `autosave_worker` to call
  `AutosaveStore::list_saves_metadata`, Save Manager UI opening will skip payload
  decompression and `Saved::decode` completely.

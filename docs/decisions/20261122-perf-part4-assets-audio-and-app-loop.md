# Performance Part 4 (Track 1): Asset Loading, Real-Time Audio, App Event Loop, Startup Caching & Autosave Worker

*Decided 2026-11-22.*

## Context

The November 2026 performance audit (`performance-audit-20261122.md`) identified eight correctness-neutral optimization opportunities across `sims-formats`, `sims-content`, `sims-audio`, and `sims-game` (Audit Items **3.1, 3.2, 3.3, 3.4, 4.1, 4.2, 4.3, and 4.4 `autosave_worker`**):

- **IFF / Catalog scanning (4.1):** `Iff::parse` allocated a `String` for every chunk label and built a temporary `ResourceMap` `BTreeMap` whose entry label `String`s were immediately discarded; `Catalog::build_with_pool` fully read and parsed entire multi-megabyte object IFFs (including all `SPR2` sprite chunks) just to extract `OBJD` and `GLOB` chunks.
- **FAR `ArchivePool` syscalls (4.2):** `ArchivePool::read_slice` issued an `fstat` (`file.metadata()`) syscall before every `pread` even though archive file handles are cached read-only.
- **Real-time audio hot loop & startup (4.3):** `Player::render_stereo` and `Mixer::render` allocated three heap `Vec`s per 64-frame audio block (~700–1,000 heap allocations/second on the real-time audio callback thread), `Note::chain` used `Vec::remove(0)`, and `Mixer::open` cloned the entire `SoundFiles` index five times across subsystems.
- **Event loop polling (3.1):** `App::about_to_wait` unconditionally scheduled `ControlFlow::WaitUntil(now + 5ms)` (200 wakeups/second even when idle or paused).
- **Audio `Feed` rebuilds (3.2):** `Feed::update` rebuilt `Game::sound_sources()` and cloned `Snapshot` on every wakeup, including non-tick UI/hover wakeups and pure camera pans.
- **Per-frame clones in `Game::draw` and `StartupLoading` (3.3):** Co-op HUD rendering deep-cloned the entire `World` every frame solely to override which Sim was highlighted as selected; `StartupLoading::render` allocated a fresh RGB framebuffer every frame.
- **Repeated startup/transition scans (3.4):** `NeighborhoodShell::open` ran sequentially and re-read `*Desc.iff` files 40+ times during `build_lot_meta`; lot transitions and Save Manager restores rebuilt install-wide assets (`Index`, `Catalog`, surface plug-ins, `HudStatic`, `HudShared`, `ContentAssets`, `PeopleLibrary`, `AvatarRegistry`, `PreloadedCast`) from scratch on every lot entry.
- **Redundant save payload re-decoding (4.4):** `AutosaveWorker` and `build_save_manager_snapshot` ran a full `Saved::decode` (bincode deserialization of every ~500 KB lot snapshot) when listing slots or committing bytes that the worker itself had just encoded via `Saved::encode`.

## Correctness-Neutrality Mandate

Every change in this lane is strictly **100% correctness-neutral**:

- Decoded IFF/Catalog structures, diagnostics, and ordering are bit-for-bit identical (`objects = 5289, diagnostics = 0`).
- Rendered PCM audio samples from `Player::render_stereo_into` / `Mixer::render_into` are bit-for-bit identical to `render_stereo` / `render`.
- Rendered pixels — including co-op HUD frames with `selected_override` and all 8 `ratchet` baselines (`clock_us == 0`) — are 100% pixel-identical.
- Saved REDB payloads and integrity checks (`AutosaveStore` record & payload SHA-256 checksums, plus full `Saved::decode` on every actual slot load) remain intact.
- Zero new external crates (`Cargo.toml` / `Cargo.lock` untouched), zero `unsafe` (`unsafe_code = "forbid"`), and zero breaking public API changes for untouched crates (`sims-vm`, `sims-sim`, `sims-render`, `sims-people`, `sims-ui`, `sims-antics`).

## Decisions

### 1. Item 4.1 — Zero-Copy/Streaming IFF Type Scanner & `rsmp` Fast Path (`crates/sims-formats/src/iff.rs`, `crates/sims-content/src/{pool,catalog}.rs`)

- Kept `Chunk::label: String` intact for backward compatibility with downstream crates (`sims-vm`, `sims-ui`, `sims-antics`) while adding zero-allocation `Iff::label_bytes(&self, chunk)` and `Iff::chunk_label(&self, chunk) -> Cow<'_, str>` helpers and optimizing `Iff::parse` via `decode_label_fast` (zero heap allocation on empty labels, direct ASCII UTF-8 fast path).
- Replaced `ResourceMap::parse` inside `Resources::mapped` with `parse_rsmp_raw`, eliminating the intermediate `BTreeMap<[u8; 4], Vec<ResourceMapEntry>>` and all discarded entry label `String` allocations while preserving exact error offsets, duplicate-type diagnostics, and tag ordering.
- Added additive `ScannedChunk`, `ScannedIff`, `ScanError<E>`, zero-copy `Iff::scan_types(bytes, tags)`, and 4 KiB-windowed streaming `Iff::scan_types_from_reader` that walk chunk headers identically to `Iff::parse` while skipping payloads for non-target tags (`SPR2`, etc.).
- Wired `Catalog::build_with_pool` (`ArchivePool::scan_iff_member` + `scan_loose_iff`) to use a per-worker reusable `Vec<u8>` scratch buffer for IFFs `<= 256 KiB` (single `pread` + zero-copy `Iff::scan_types`) and 4 KiB-windowed streaming `Iff::scan_types_from_reader` for larger IFFs (`> 256 KiB`), avoiding small-file syscall amplification while still skipping megabytes of `SPR2` data in large archives.

### 2. Item 4.2 — Cached Archive Length in `ArchivePool` (`crates/sims-content/src/pool.rs`)

- Cached `file_len: u64` inside `OpenArchive::open` on open, removing the per-`pread` `fstat` syscall in `ArchivePool::read_slice` and `ArchivePool::scan_iff_member`.
- Bounds checks use `archive.file_len`, and post-open file truncation is still caught and reported as `"ends early"` via `read_exact_at`'s `UnexpectedEof` error (verified by `read_member_rejects_member_truncated_after_handle_cached`).

### 3. Item 4.3 — Zero-Allocation Real-Time Audio Mixing & Shared `SoundFiles` (`crates/sims-audio/src/{player,music,voices}.rs`, `crates/sims-game/src/{sound,output}.rs`)

- Added `Player::render_stereo_into(&mut self, frames, sources, out)` using a stack `[0i64; 2 * BLOCK]` mix accumulator when `frames <= BLOCK` (64 frames), avoiding all per-block heap allocations on the audio callback thread.
- Changed `Note::chain` from `Vec<u32>` to `VecDeque<u32>` (`push_back`/`pop_front`), eliminating `Vec::remove(0)` element shifts.
- Implemented `Samples` and `TrackSource` for `Arc<SoundFiles>` and added `VoiceCatalog::from_shared_sound_files(Arc<SoundFiles>)`, allowing `Mixer::open` to share a single `Arc<SoundFiles>` across all five audio subsystems (`Player`, `MusicPlayer`, `RadioPlayer`, `AmbienceMixer`, `VoiceCatalog`) instead of cloning the full index five times.
- Added reusable `voice_scratch: Vec<i16>` + `Mixer::render_into` and preallocated the stereo output buffer once in `output::mix`.
- Verified bit-for-bit PCM sample identity in both `sims-audio` (`render_stereo_into_is_bit_identical_to_render_stereo`) and `sims-game` (`perf_part4` 500-block dual render comparison with zero output buffer reallocations).

### 4. Item 3.1 — Exact Event-Loop Deadlines (`crates/sims-game/src/{pace,hud,game,caf_screen,neighborhood,main}.rs`)

- Added `Pace::remaining_us`, `Hud::next_blink_us`, `Game::is_edge_scrolling`, `Game::next_wake_us`, `CafSession::next_wake_us`, and `NeighborhoodShell::next_wake_us`.
- Replaced the unconditional `WAKE = 5ms` poll in `App::about_to_wait` with `App::next_deadline(now) -> Option<Instant>`:
  - Returns `ControlFlow::Wait` (0 idle wakeups/sec) when paused/idle with no active animations, timers, co-op sessions, or pending autosaves.
  - Returns `ControlFlow::WaitUntil(deadline)` for the exact next owed sim tick (`42 ms` Normal, `14 ms` Fast, `2 ms` Ultra), plumb-bob spin, clock blink, UI animation/fade, edge scroll, hover-fade, hover-card delay, water-wave frame (`200 ms`), co-op pump, or autosave poll (`16 ms` startup poll only during `Session::Starting`).

### 5. Item 3.2 — Dirty-Gated Audio `Feed` & `Arc<Snapshot>` (`crates/sims-game/src/{sound,game}.rs`)

- Wrapped `Input::Sources(Arc<Snapshot>)` and `Mixer::sources: Arc<Snapshot>` for zero-clone channel handoff to the audio thread.
- Added `SoundWorldKey` (`ticks`, `restore_gen`, `sound_rev`, `error_count`, `object_count`, `hour`, `lot_id`, `buy_held_pos`) bumped on every non-tick world mutation (`refresh_world`, `cancel_active_buy_build_tools`, `apply_world_mutation`, `restore`, undo/redo, co-op tick).
- Dirty-gated `Feed::sync_sources`: skips `sound_sources()` completely on unchanged non-tick UI/hover wakeups, and reuses `objects`/`positions` from the previous `Arc<Snapshot>` during pure camera pans (updating only `screen` via `Game::sound_screen()`), verified bit-for-bit identical to `Game::sound_sources()`.

### 6. Item 3.3 — Zero-Clone Co-op HUD Override & Startup Canvas Reuse (`crates/sims-game/src/{hud,game,main}.rs`)

- Added `selected_override: Option<Option<u16>>` to `Hud::household` and additive `Hud::draw_for_lot_at_with_selection`, eliminating the per-frame `World` deep clone in `Game::draw` during co-op sessions while preserving 100% pixel-exact output (verified in `perf_part4` against a cloned `World`).
- Reused `self.canvas.color` via `Frame::from_rgb` + `frame.fill` in `StartupLoading::render` instead of allocating a fresh RGB buffer every loading frame.

### 7. Item 3.4 — Parallel `NeighborhoodShell::open` & Cross-Lot `InstallAssets` Cache (`crates/sims-game/src/{neighborhood,game}.rs`)

- Cached all `*Desc.iff` `STR#` tables once in `NeighborhoodDescriptions::load` using `Iff::scan_types(&bytes, &[*b"STR#"])` and switched `build_lot_meta`'s `HouseXX.iff` `SIMI` extraction to `Iff::scan_types(&base_bytes, &[*b"SIMI"])`, eliminating 40+ redundant disk reads.
- Parallelized `NeighborhoodShell::open` via `std::thread::scope` sharing a single `Arc<ArchivePool>` across UI art packs, strings/tuning, neighborhood map load, street/description tables, and `base_index`/faces.
- Introduced `InstallAssets` (caching `Install`, `Arc<ArchivePool>`, `Arc<Index>`, `Arc<Catalog>`, surface plug-ins, `HudStatic`, `HudShared`, `ContentAssets`, `PeopleLibrary`, `Arc<AvatarRegistry>`, and `PreloadedCast`) on `NeighborhoodShell`, wired into `Game::open_from_store_cached` and `Game::from_loaded_lot_cached` across all lot-entry, Save Manager, travel, and co-op restage paths, and removed a duplicate `people.refresh(lot.world())` call during lot assembly.

### 8. Item 4.4 — Header-Only Save Validation for Listing & Self-Encoded Commits (`crates/sims-game/src/{autosave_worker,neighborhood}.rs`)

- Added `validate_save_payload_header` verifying the 8-byte `Saved` format (`MIN_FORMAT..=FORMAT`) and `SNAPSHOT_FORMAT` header while `AutosaveStore` continues verifying full record and payload SHA-256 checksums.
- Used `validate_save_payload_header` in `build_save_manager_snapshot`, `neighborhood.rs`'s corrupt-save presence check, and self-encoded `AutosaveWorker` commits (`SavedLot`, `enqueue_world`, `enqueue_saved_lot_bytes`), while keeping full `validate_save_payload` (`Saved::decode`) on actual slot loads (`load_save_validated`) and arbitrary external `enqueue_job` payloads.

## Measured Results (`--release`, Apple Silicon, Complete Collection)

Captured with `crates/sims-game/tests/perf_part4.rs` before and after on identical hardware and install data:

| Metric | Baseline (`origin/main`) | After (`perf/assets-audio-and-app-loop`) | Improvement |
| --- | ---: | ---: | ---: |
| `Catalog::build_with_pool` (warm, 5,289 objs, 0 diags) | `26.35–27.66 ms` | `17.07–18.83 ms` | **1.55× faster (−36%)** |
| `NeighborhoodShell::open` (cold w/ REDB import) | `399.77 ms` | `238.62 ms` | **1.68× faster (−40%)** |
| `NeighborhoodShell::open` (warm, pre-imported) | `185.74 ms` | `101.41 ms` | **1.83× faster (−45%)** |
| `Game::from_loaded_lot` (`sims-game` lot wrap) | `218.57 ms` (uncached) | `108.70 ms` (`InstallAssets` cached) | **2.01× faster (−50%)** |
| Full `shell.enter_lot` (incl. untouched `sims-vm::enter_lot`) | `570.74 ms` (lot 5) / `596.69 ms` (lot 7) | `453.77 ms` (lot 5) / `455.50 ms` (lot 7) | **−21% to −24% (−117 to −141 ms)** |
| `Mixer::open` (warm) | `82.62 ms` | `50.57–50.97 ms` | **1.63× faster (−38%)** |
| Audio callback `render_into(64)` heap allocs/block | `3 allocs/block` (~700–1,000/s) | **`0 allocs/block`** (0 output buffer reallocs) | **100% eliminated** |
| Idle/paused main-loop wakeups (`about_to_wait`) | `200 wakeups/sec` (`5 ms` poll) | **`0 wakeups/sec`** (`ControlFlow::Wait`) | **100% eliminated** |
| `SaveManager` commit 6 Goth slots | `76.05 ms` | `62.63 ms` | **−18%** |
| `SaveManager` list 6 Goth slots | `14.34–14.73 ms` | `11.75–12.25 ms` | **−18% to −22%** |

## Follow-Ups Outside Track 1's Disjoint Scope

- **`sims-vm::lifecycle::enter_lot` (`StoreRuntime::from_snapshot`):** The remaining ~345 ms of `shell.enter_lot` lives inside `crates/sims-vm/` (`StoreRuntime::from_snapshot` rebuilds `Index`, `Catalog`, `Program`, and `Animations` sequentially on every lot entry). Caching immutable base `Program`/`Animations` in a future `sims-vm` lane will eliminate the remaining warm lot-entry latency.
- **`sims-sim::autosave::decode_record_validated`:** ~9 ms of the 11.75 ms `list_6_slots` latency is double SHA-256 hashing of six ~500 KB REDB blobs inside `crates/sims-sim/`; a metadata-only slot header table in a future `sims-sim` lane can reduce `list_slots` to `< 1 ms`.
- **`sims-render::BaseAssets`:** Deriving `Clone` (or `Arc`-wrapping immutable floor/wall/lightmap tables) in `crates/sims-render/` would allow `InstallAssets` to share `BaseAssets` across lot entries as well.

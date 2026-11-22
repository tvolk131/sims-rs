# Startup deduplication, pooled FAR reads, and parallel asset loading

*Decided 2026-11-21.*

After the frame-time/blit optimizations (`20261121-frame-time-and-blit-perf.md`) and VM tick /
two-tier `LotCache` speedups (`20261121-vm-tick-and-two-tier-lot-cache-perf.md`), steady interactive
play ran smoothly, but cold and warm lot startup on `House05.iff` still took **2.5–3.8 s** (plus
`~83–195 ms` for `sound::Mixer::open`), and `cargo test --release -p sims-game --test goth` spent
**77.10 s** of test execution time (including **219.3 s** of kernel `sys` time across parallel
tests).

Profiling `House05.iff` in `--release` uncovered three exact, correctness-neutral bottlenecks:

1. **Duplicate startup construction across crates:**
   - `Game::open` called `sims_render::open_house`, `Lot::load`, `Hud::open`, and `People::open`
     independently, constructing `Install`, `Neighborhood`, `Index`, and `Catalog` **3×** (plus a
     4th `Install` + `Neighborhood` in `Hud::open`), scanning `sims_content::Assets` **2×** (in
     `Lot::boot` for `Wardrobe` and in `sims_people::People::open`), and scanning shared
     `WallsAndFloors` directories **2×** (once for floors, once for walls).
   - `AvatarRegistry::load` ran **5×** during a single `Game::open`: twice inside `Lot::boot` (once
     for `People::load_each` over all kind-2 catalog definitions and again at load step 6e for
     `People::load` over the lot's saved objects) and **3×** inside `GamePeople::refresh` (once per
     Goth resident because `sims_people::People::update` called `Cast::residents` per person and
     `Cast::residents` called `People::load_each` -> `AvatarRegistry::load` on every invocation).
   - `Catalog::build` resolved every `GLOB` semi-global with an `O(N)` linear `index.named(...)`
     scan across all resources, and `People::load_each` re-read `PersonGlobals.iff` (`STR# 8192`)
     once per person type rather than once per unique semi-global resource.
2. **Uncached per-member FAR syscalls and eager animation-only `.bcf` reads:**
   - Every `Resource::read` on a FAR archive member executed `File::open` + `fstat` + `seek` +
     `read_exact` + `close`, producing tens of thousands of kernel syscalls per launch.
   - `Animations::load`, `Wardrobe::load`, `AvatarRegistry::load`, and `sims_people::Library::scan`
     read entire `.bcf` payloads into memory before checking whether the 8-byte header was
     `[0; 8]` (animation-only BCF vs. skeleton/appearance BCF).
   - `UiGraphics::read` walked loose directories via `resolve` on every UI bitmap even when
     `UIGraphics/` held only ~61 loose files and `UIGraphics.far` held the rest.
3. **Serial loading of independent startup stages:**
   - Install mounts (`Index::build`), IFF scanning (`Catalog::build`), SimAntics library linking
     (`Program::link`), animation clips/samples (`Animations::load`), avatar catalogs
     (`AvatarRegistry::load`), wardrobe/surface/HUD art/face/audio scans, and `sound::Mixer::open`
     all executed strictly sequentially on a single thread.

## What was decided

All changes are **100% bit-for-bit correctness-neutral** (zero changes to simulation determinism,
43,200-tick Goth state hash `df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a` on
`origin/main` after `#340` — `48440355df84c6963b8e634c29c6a379e85e33d86a14d08a9aebb1d51bf85586`
pre-`#340`, save wire layout, rendered `Canvas` pixels/depth/IDs, or any of the 8 SHA-256 hashes in
`crates/sims-game/tests/ratchet.txt`), use only Rust `std` (`std::thread::scope`, `Arc`,
`RwLock`/`Mutex`, `BTreeMap`/`BTreeSet`) with **no new dependencies** and **zero `unsafe`**, and
keep every existing public API signature backward-compatible:

1. **Shared bounded `ArchivePool` (`crates/sims-content/src/pool.rs`):**
   - Retains at most `MAX_OPEN_ARCHIVES = 64` cached FAR archive entries (`File`) in an
     `RwLock<BTreeMap<PathBuf, Arc<OpenArchive>>>` with LRU eviction (preferring idle
     `Arc::strong_count == 1` entries) so download-heavy installs stay bounded in retained handles
     (concurrent opens and evicted in-flight handles may briefly exceed 64 until their reads
     complete).
   - Uses safe positional `pread` (`std::os::unix::fs::FileExt::read_exact_at`) on Unix (with a
     `Mutex<File>` `seek`+`read_exact` fallback on non-Unix) so concurrent workers share one open
     descriptor per FAR without lock contention on Unix.
   - Validates bounds against the live open file length (`metadata().len()`) and `MAX_MEMBER` on
     every full read (`Resource::read_with`) and 8-byte header peek (`Resource::read_head_with`)
     so post-open archive truncation is never silently skipped.
2. **Deterministic parallel content, UI, people, VM, and audio loaders:**
   - Parallelized with `std::thread::scope` (every chunked worker pool bounded to at most
     `min(available_parallelism, 8)` workers, alongside small fixed-size sibling coordinators such
     as `Program::link_with_pool`'s 3 metadata helpers and `Game::open`'s stage coordinators) while
     collecting per-item results into pre-ordered vectors and merging sequentially in exact
     original mount/index/sorted order and error priority:
     - `sims-content`: `Index::build` (bounded mount chunks preserving mount-by-mount error
       priority), `Catalog::build_with_pool` (parallel IFF reads + `O(log N)`
       `Index::first_by_file_name` `GLOB` lookup), `Assets::scan` (per `SOURCES` mount),
       `Animations::load_with_pool` (parallel `.cfp`/`.bcf` reads + 8-byte header peek),
       `Wardrobe::load_with_pool` (parallel `.cmx`/`.bcf` reads + 8-byte `[0; 8]` skip),
       `ImportWardrobe::load_lenient_with_pool` (pooled parallel `.cmx`/`.bcf` scan + 8-byte
       `[0; 8]` skip), `AvatarRegistry::load_with_pool` (parallel `.cmx`, `.bcf` with 8-byte
       `[0; 8]` skip, indexed `.bcf`, and HEAD mesh validation applied in exact startup precedence
       order), `People::load_each_with_registry` / `load_each_with_registry_result` (caches
       semi-global `STR# 8192` skin tones per semi-global while preserving exact `body_strings` ->
       `skin_tones` -> `AvatarRegistry` error order and immutable `export_skeletons` resolution),
       `Surfaces::build_with_pool` (scans shared `WallsAndFloors` directories once in bounded
       batches and parses `.flr`/`.wll` in parallel while checking directory and parse errors in
       exact original traversal order), and `UiGraphics::open_with_pool` (caches visited directory
       listings on demand for exact component-by-component case resolution without walking
       unreferenced subdirectories, and reuses `ArchivePool` for `UIGraphics.far`).
     - `sims-vm`: `Program::link_with_pool` (parallel library reads/parses + parallel career/label
       checks with exact sequential error priority: `careers` -> `career_texts_end` ->
       `labels_hold_no_token` -> libraries -> `MissingDefinition`) and `Lot::boot` /
       `Lot::load_preloaded` (two-stage parallel DAG deferring stage errors to their exact original
       points, forwarding boot-time `view_centre`, and reusing immutable `program.people` directly
       at step 6e after checking `unresolved_people` over `saved_guids`).
     - `sims-render`: `BaseAssets::load` + `Assets::from_base` / `Assets::new_shared` sharing
       `Arc<Index>`, `Arc<Catalog>`, `Arc<ArchivePool>`, and pre-scanned `floor_plugins` /
       `wall_plugins` from `Surfaces::build_with_plugins_and_pool` / `Surfaces::plugins_with_pool`
       (and reading plug-in floor/wall sprites through `ArchivePool`).
     - `sims-people`: `Library::scan_with_pool` (8-byte `[0; 8]` peek + parallel `.bcf` parse) and
       `Cast::from_shared` with optional `PreloadedCast`, cached immutable `AvatarRegistry`,
       and cached `body_strings`.
     - `sims-ui`: `Art::load_with_optional` (bounded `<= 8` workers decoding required/optional
       bitmaps + fonts in parallel while preserving exact `missing` order), `Careers::load_with_pool`,
       `Faces::load_with_pool`, and `house::Tuning::load_with_pool`.
     - `sims-audio`: `SoundFiles::scan` (`walk_parallel` across top-level directory chunks bounded
       to `<= 8` workers, merged in exact sorted order) and parallel `Module::load_all`.
3. **Unified deduplicated startup DAG in `sims-game` (`game.rs`, `hud.rs`, `people.rs`, `main.rs`):**
   - `Game::open` constructs `Install`, `Neighborhood`, `Index`, `Catalog`, `ArchivePool`,
     `AvatarRegistry`, `sims_content::Assets`, `Wardrobe`, `Surfaces`, `BaseAssets`, and
     `HudStatic` **once** in Stage A; links `Program`, loads `Animations`, resolves all kind-2
     `People`, and loads `HudShared` (`Careers` + `house::Tuning`) in parallel in Stage B; then
     runs `Lot::load_preloaded` followed by parallel `Hud::from_preloaded` (`Faces`) and
     `Cast::from_shared` + `People::refresh` in Stage C.
   - `crates/sims-game/src/main.rs` overlaps `sound::Mixer::open(&options.install)` in parallel
     with `Game::open(&options)` when launching interactively without `--mute`, then starts
     `Output::start(mixer)` on the main thread.

## Before vs. After Benchmark (`House05.iff`, `--release`, Apple Silicon)

Measured back-to-back on the same machine (`origin/main` `9e845766` quiet baseline vs. this branch):

| Stage / Metric | Before (`9e845766`) | After (`perf/startup-dedup-and-parallel`) | Speedup |
| --- | ---: | ---: | ---: |
| `Game::open` (initial) | `3,769.31 ms` | `341.23 ms` | **11.0×** |
| `Game::open` (warm best) | `2,496.51 ms` | `311.43 ms` | **8.0×** |
| `TTFF` (`Game::open` + first 800×600 render, warm) | `2,508.00 ms` | `323.83 ms` | **7.7×** |
| `Game::open` + `Mixer::open` (warm, sequential) | `2,579.15 ms` | `366.42 ms` | **7.0×** |
| `Game::open` + `Mixer::open` (interactive overlapped) | `2,579.15 ms` | `~311 ms` | **~8.3×** |
| `Index::build` | `41.80 ms` | `14.59 ms` | **2.9×** |
| `Catalog::build` | `132.83 ms` | `40.35 ms` | **3.3×** |
| `Program::link` (all catalog) | `320.78 ms` | `67.35 ms` | **4.8×** |
| `Animations::load` | `336.55 ms` | `26.10 ms` | **12.9×** |
| `People::load_each` (all kind=2) | `237.75 ms` | `90.19 ms` | **2.6×** |
| `Wardrobe::load` | `102.04 ms` | `17.79 ms` | **5.7×** |
| `Surfaces::build` | `77.34 ms` | `14.15 ms` | **5.5×** |
| `sims_render::open_house` | `177.93 ms` | `60.98 ms` | **2.9×** |
| `Lot::load` (standalone) | `1,475.31 ms` | `207.73 ms` | **7.1×** |
| `Hud::open` (standalone) | `70.00 ms` | `17.85 ms` | **3.9×** |
| `GamePeople::open` (standalone) | `363.01 ms` | `76.70 ms` | **4.7×** |
| `GamePeople::refresh` (standalone / shared `Cast`) | `617.19 ms` | `180.20 ms` / `<5 ms` | **3.4×** / **>100×** |
| `sound::Mixer::open` | `82.64 ms` | `46.58 ms` | **1.8×** |
| `sims-game --test goth` (60 tests wall / `sys` time) | `77.10 s` / `219.29 s` | `27.02 s` / `46.12 s` | **2.85×** / **4.75×** |

## What would reopen this

- Any new install-wide loader added to `Lot::boot`, `Hud::open`, or `People::open` should wire
  through `ArchivePool` and `PreloadedBoot` / `Game::open`'s shared stages rather than re-creating
  `Install`/`Index`/`Catalog`/`AvatarRegistry`.
- Any change to binary `.bcf` parsing that allows non-zero skeleton/appearance counts in
  animation-only containers (or inspects bytes past offset 8 when both counts are zero) must update
  the 8-byte `[0; 8]` header peeks in `Animations::load_with_pool`, `Wardrobe::load_with_pool`,
  `AvatarRegistry::load_with_pool`, and `sims_people::Library::scan_with_pool`.
- If future work adds neighborhood-switching or lot-reloading inside one process, the shared
  `ArchivePool` and install-scoped caches (`Index` install mounts, `AvatarRegistry`, `Surfaces`,
  `HudStatic`, `sims_content::Assets`) could be kept across lot transitions instead of rebuilt per
  lot.

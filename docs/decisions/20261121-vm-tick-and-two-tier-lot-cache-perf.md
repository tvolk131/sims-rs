# VM tick throughput, two-tier `LotCache`, and Ultra-speed pacing

*Decided 2026-11-21.*

After the static lot cache and people-pass optimizations (`20261121-frame-time-and-blit-perf.md`),
steady normal-speed frames dropped to `1.3–2.1 ms`, but **Fast/Ultra speed (`14 ms` / `2 ms` tick
period)** still stuttered and the 43,200-tick Goth full-day test spent ~10.5 s wall-clock time.
Profiling `House05.iff` in `--release` uncovered three exact, correctness-neutral bottlenecks:

1. **`sims-vm` per-tick `World::clone()` + hot interpreter/animation loops:**
   - Interactive stepping (`Lot::step_in` with `asks = true`) deep-cloned the entire `World` before
     every tick (`~97 µs` per clone+drop on `House05.iff`: `54.4 µs` in `objects`, `18.5 µs` in
     `neighbors`, `12.0 µs` in `frame_storage`, `4.0 µs` in `arrays` totaling 188 KB) just in case
     the tick paused on an unanswered dialog.
   - Inside `Lot::advance`, macOS `sample` profiling over 20,000 ticks showed:
     - `Animations::clip` (~23% of VM CPU): O(N) linear `eq_ignore_ascii_case` scan across the
       clip map on case-insensitive lookups (plus per-lookup `to_ascii_lowercase()` heap allocation
       in `Animations::samples` sample-name normalization).
     - `Interpreter::strides` (~17% of VM CPU): repeatedly decompressing `.cfp` animations and
       posing skeletons to recompute walk/run stride distances for the same `(clip, skeleton)` pair.
     - `set_to_next::next_in_order` (~8% of VM CPU): rebuilding a full `Vec<(i16, &Object)>`
       allocation on every `Set To Next` instruction.
     - Cross-crate non-inlined `sims_core::float::{F32, Ext}` arithmetic and `Catalog::object`, plus
       redundant `world.objects.keys().eq(ids)` checks and `person.data.clone()` in
       `autonomy::scan`.
2. **Monolithic `LotCache` invalidation on animated objects:**
   - Whenever a single door, lamp glow word, TV/appliance, or Buy Mode preview changed, `LotCache`
     missed and repainted all terrain, grass, floors, floor shadows, walls, trims, and caps
     (`2.7–3.9 ms` at `1024×768 2×`).
3. **Uncapped Ultra-speed catch-up bursts and per-frame title IPC (`crates/sims-game/src/main.rs`):**
   - `App::tick` stamped `self.last = now` before `Game::advance(micros)` with no wall-clock budget,
     letting Ultra/Idle speeds run up to `CATCH_UP = 20` synchronous ticks before yielding to draw a
     frame and risking catch-up spirals on heavy ticks.
   - `App::redraw` called `window.set_title(...)` on every rendered frame whose tick count changed,
     generating continuous AppKit/window-manager IPC at Fast/Ultra speed.

## What was decided

All changes are **100% bit-for-bit correctness-neutral** (zero changes to simulation determinism,
43,200-tick Goth state hash `48440355df84c6963b8e634c29c6a379e85e33d86a14d08a9aebb1d51bf85586`,
`SNAPSHOT_FORMAT`, `postcard` save wire layout, rendered `Canvas` pixels/depth/IDs/rects/`Stats`,
or any of the 8 SHA-256 hashes in `crates/sims-game/tests/ratchet.txt`):

1. **Copy-on-write `World` storage + `sims-vm` hot-loop speedups:**
   - Added `sims_sim::{CowBox, CowMap}` (`crates/sims-sim/src/cow.rs`) backed by `Arc` with
     `Arc::strong_count` unshared fast paths, exact `PartialEq` (matching `NaN` inequality of the
     wrapped types), and wire-identical `Serialize`/`Deserialize` (using `collect_map`) matching
     plain `T` / `BTreeMap<K, V>`. Wrapped `World::objects`, `World::arrays`, `World::frame_storage`
     in `CowMap` and `World::neighbors` in `CowBox`.
   - Guarded read-before-write sites in `sims-vm` (`tick::pass`, `Machine::wrote_local`,
     `Machine::end_turn`) so non-mutating objects and untouched `frame_storage` entries avoid
     triggering `Arc::make_mut` during `asks = true` ticks.
   - Indexed `Animations` (`crates/sims-content/src/animations.rs`) by ASCII-lowercased clip name
     (`by_lower: BTreeMap<String, Vec<String>>`) with stack-buffered ASCII lowercasing and memoized
     `Interpreter::strides` (`Animations::cached_strides`, keyed by clip name, skeleton name, and
     bone list, invalidated on animation library mutation via an internal revision counter).
   - Eliminated per-call `Vec` allocation in `set_to_next::next_in_order`, skipped redundant
     object-list scans and `person.data` clones in `autonomy::scan`, and marked hot
     `sims_core::float::{F32, Ext}` operations and `Catalog::object` `#[inline]`.
2. **Two-tier `LotCache` (`crates/sims-render/src/{cache.rs, scene.rs}`):**
   - Split `LotCache` into **Tier 2 (`CacheHit::Full`)** — the complete `Pass::Lot` canvas — and
     **Tier 1 (`CacheHit::Environment`)** — the level-1 static environment right after `trims(1)`
     and before `objects(1)` (background, terrain/grass, Buy/Build grid, level-1 floors and
     resolved floor shadows, level-1 walls with resolved wall-shaping object apertures, and
     trims/caps).
   - Validated Tier 1 with `EnvKey` covering every environment input (`assets_id`, canvas size,
     `clip_bottom`, zoom/rotation/origin, effective level-1 wall-cut mode & cutaway `Mask`,
     `time_of_day`, `house`, Buy/Build grid flag + array `8`, exact `Lot` equality, resolved
     level-1 floor shadows `shadows0`, and resolved level-1 wall apertures `wall_openings0`).
   - On a Tier-1 hit (animated non-wall objects, doors swinging open/closed without moving their
     wall aperture, lamp glow word `data[51]`, TV/appliance `dynamic_sprites`, non-shadowing Buy
     preview movement, or level-2 object/wall-mode changes), `LotCache` restores Tier 1 via fast
     slice copies (`Canvas::restore_pixels_from`) and paints only `objects(1)` (plus level 2 when
     `camera.level == 2`) without rebuilding `Grass` or repainting level-1 terrain/floors/walls.
   - Extended `crates/sims-render/tests/lot_cache_parity.rs` to assert both `CacheHit` tier
     transitions and bit-for-bit parity against uncached `draw_lot_with_preview` across all of
     those cases.
3. **Interactive pacing budget and title throttling (`crates/sims-game/src/{game.rs, main.rs}`):**
   - Added `Game::advance_while(clock_elapsed_us, step_elapsed_us, keep_going)` (`Game::advance`
     delegates to it with `(elapsed_us, elapsed_us, |_| true)`, leaving headless `Game::advance`
     and `Pace` semantics 100% unchanged).
   - Wired `SIM_BUDGET = 8 ms` (`within_step_budget`: always allows at least 1 owed tick, then
     yields to render once spent time reaches 8 ms) and `WakeClock` into `App::tick`, keeping
     continuous wall-clock presentation time (`clock_us`) independent of capped scheduler pacing so
     plumb-bob rotation and pause-button blinking never slow down when Ultra/Idle speed hits the
     budget cap.
   - Added `TitleThrottle` (`TITLE_INTERVAL = 250 ms`) in `App::redraw` so steady running updates
     the window title at most 4×/sec while immediately flushing first draw and paused/stopped state
     transitions, with unit tests for both helpers in `crates/sims-game/src/main.rs`.

## Before vs. After Benchmark (`House05.iff`, `--release`, Apple Silicon)

Measured on the same machine (`origin/main` `d0db4136` baseline vs. this branch):

| Benchmark | Before (`d0db4136`) | After (`perf/vm-tick-and-two-tier-cache`) | Speedup |
| --- | ---: | ---: | ---: |
| `World::clone()` + drop (avg 500 iters, 346 objects) | `97.21 µs` | `4.25 µs` | **22.9×** |
| `sims-vm` 500 ticks (`House05.iff`, `asks = false`) | `0.140 ms/tick` (`69.96 ms`) | `0.105 ms/tick` (`52.54 ms`) | **1.33×** |
| `sims-vm` 500 ticks (`House05.iff`, `asks = true`) | `0.249 ms/tick` (`124.29 ms`) | `0.178 ms/tick` (`88.98 ms`) | **1.40×** |
| 43,200-tick Goth full-day test (`strict_mode_runs_the_goth_houses_whole_day`) | `10.47 s` (`6.67 s` user) | `3.77 s` (`3.61 s` user) | **2.78×** wall (**1.85×** user) |
| `LotCache` animated object (`1024×768 2× Far Cutaway`) | `2.80 ms` | `0.45 ms` | **6.2×** |
| `LotCache` door animation (`1024×768 2× Far Cutaway`) | `2.73 ms` | `0.45 ms` | **6.1×** |
| `LotCache` animated object (`1024×768 2× Med Cutaway`) | `3.89 ms` | `0.56 ms` | **6.9×** |
| `LotCache` door animation (`1024×768 2× Med Cutaway`) | `3.84 ms` | `0.84 ms` | **4.6×** |
| `LotCache` animated object (`1024×768 2× Near Cutaway`) | `3.67 ms` | `0.62 ms` | **5.9×** |
| `LotCache` door animation (`1024×768 2× Near Cutaway`) | `3.56 ms` | `0.60 ms` | **5.9×** |

Steady Tier-2 `LotCache` hits remain `0.05–0.06 ms` across all three zoom levels.

## What would reopen this

- Adding any new world, lot, or object field read by level-1 environment passes (`floors(1)`,
  `corners(1)`, `walls(1)`, `trims(1)`) must also add that field to `EnvKey` in
  `crates/sims-render/src/cache.rs` and cover it in `crates/sims-render/tests/lot_cache_parity.rs`.
- If level-2 object-heavy lots ever show animation bottlenecks on floor 2, Tier 1 could be extended
  with an optional level-2 environment tier after `trims(2)` when level-1 objects are unchanged.

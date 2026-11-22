# Performance Part 4 — Wave B: Compact `Canvas` Layout, Zero-Copy Picking, `memcpy` Blit & Safe `softbuffer` Partial Damage

*Decided 2026-11-22.*

This decision records the Wave B (`perf/canvas-compaction-and-partial-present`)
memory-layout, picking, and window-presentation optimizations from the
2026-11-22 performance audit (**Audit Items 2.1 and 3.5 Tier A**), building
directly on the merged Wave A tracks (`#378`, `#379`, `#380`).

Every optimization is **100% correctness-neutral** (bit-for-bit identical
rendered color, depth, pick IDs, `Game::under` picking results, and 43,200-tick
Goth simulation hash), uses **zero new external crates**, and stays **100% safe
Rust (`unsafe_code = "forbid"`)**.

## Summary of Changes

### 1. Compact `Canvas` Layout Across All Crates (Audit Item 2.1)
- `crates/sims-render/src/{canvas,lib,cache,png,scene,build_preview,neighborhood}.rs`
- `crates/sims-ui/src/frame.rs`, `crates/sims-people/src/lot.rs`,
  `crates/sims-viewer/src/{hud,main}.rs`,
  `crates/sims-game/src/{game,hud,people,pick,neighborhood,main}.rs`
- Replaced `Canvas`'s unaligned 11-byte/pixel layout (`Vec<[u8; 3]>` color +
  `Vec<i32>` depth + `Vec<Option<u16>>` ids = `8.65 MB` at `1024×768`) with a
  compact, 4-byte-aligned 8-byte/pixel layout (`6.29 MB` at `1024×768`, **−27.3%**):
  - `Canvas::color: Vec<u32>` storing packed `0x00RRGGBB` natively matching
    `softbuffer`'s framebuffer pixel format (100% lossless 24-bit RGB), with
    ergonomic helpers (`pack_rgb`, `unpack_rgb`, `pixel_rgb`, `set_pixel_rgb`,
    `color_rgb`).
  - `Canvas::depth: Vec<u16>` (`EMPTY_DEPTH = u16::MAX`, `clamp_depth(d: i32) -> u16`
    clamping negative / `> 65535` depths identically to `0..=65535` before storing),
    halving depth buffer bandwidth (`3.15 MB -> 1.57 MB` at `1024×768`) and
    doubling L2/L3 cache residency across both `sims-render` and `sims-people`.
  - `Canvas::ids: Vec<u16>` using `ID_NONE = u16::MAX` sentinel instead of
    `Option<u16>` (`4 bytes -> 2 bytes`, halving ID buffer size from
    `3.15 MB -> 1.57 MB` at `1024×768`), keeping ID `0` valid (`cache.rs:262`)
    and asserting valid IDs `!= u16::MAX` in `encode_id`.
- Compacted `sims_ui::frame::Frame` to store `Vec<u32>` (`0x00RRGGBB`) with
  zero-copy `from_packed` / `into_packed` / `packed` (preserving `from_rgb` /
  `into_rgb` / `rgb` compatibility helpers), keeping `std::mem::take(&mut canvas.color)`
  zero-copy across HUD, dialog, neighborhood chrome, loading screens, and viewer
  overlays.

### 2. Zero-Copy `lot_depth` & Sparse Bounding-Rect `people_ids` (Audit Item 2.1)
- `crates/sims-render/src/{cache,canvas}.rs`, `crates/sims-people/src/lot.rs`,
  `crates/sims-game/src/{game,people,pick}.rs`
- **Zero-copy `lot_depth`:** Added `LotCache::last_lot_depth(&self) -> Option<&[u16]>`
  (tracked via `entry_is_current`). On cached draws, `Game::draw` sets
  `lot_depth_from_cache = true` and `Game::under` borrows `&[u16]` directly
  from `LotCache` with **0 bytes copied** (eliminating a `3.15 MB/frame`
  unconditional `lot_depth.clone_from` copy). On uncached/bypassed draws or
  inside `Game::clear_lot_cache()`, `Game` snapshots the depth into its fallback
  `self.lot_depth: Vec<u16>` so `Game::under` remains exact at all times.
- **Sparse `people_ids`:** Added `TrackedLotCanvas<'a>` implementing
  `sims_people::raster::Target` and `People::draw_on_lot_in_viewport_tracked`
  to record the tight per-Sim and per-plumb-bob bounding rectangles actually
  written during 3D rendering, plus `Ids::take_rects(&Canvas, &[Rect])` storing
  only those clipped rects in a flat reusable pool.
  - Because `pick::noted` only ever accepts `object.person.is_some() || object.guid == sims_people::bob::STANDARD`
    (and `Game::draw` also appends any `canvas.drawn(id)` rect for those objects),
    sparse `people_ids` returns bit-for-bit identical results to full-screen
    `Ids::take` at every pixel while shrinking per-frame picking capture from
    **`3.15 MB/frame` to `< 40 KB/frame`** (over **80× smaller**).

### 3. `memcpy` 1× Blit, Pure-`u32` 2× Blit & Safe `softbuffer` Partial Damage (Audit Item 3.5 Tier A)
- `crates/sims-game/src/{main,game,neighborhood}.rs`,
  `crates/sims-render/src/neighborhood.rs`
- Updated `blit_canvas` to read `Canvas::color: &[u32]` directly:
  - `scale == 1` with matching dimensions executes a single contiguous
    `buffer[..total].copy_from_slice(&canvas.color[..total])` (`memcpy`);
    mismatched widths use row-level `copy_from_slice` + edge clamping.
  - `scale == 2` doubles aligned `u32` pixels into `[px, px]` pairs with zero
    bit-packing shifts and duplicates rows via `copy_within`.
  - `scale >= 3` indexes `u32` pixels directly without shift/OR packing.
- Added `blit_canvas_rect` (handling arbitrary scales `1..=4`, odd window sizes,
  and trailing edge-clamped columns/rows bit-for-bit identically to `blit_canvas`)
  and safe partial-update presentation in `App::redraw`:
  - Partial rectangular CPU blitting (`blit_canvas_rect`) is used **only** when
    `buffer.age() == 1` (backbuffer retains the exact previous frame; macOS
    CoreGraphics returns `age() == 0` and falls back to full `blit_canvas` +
    `buffer.present()`), the `(bw, bh, scale, cw, ch, screen_kind)` signature
    matches the last successfully presented frame (invalidated on resize, error,
    or loading-screen direct presents), and the active screen proves a complete,
    bounded list of canvas-space damage rects.
  - `buffer.present_with_damage(&damage)` is invoked **only** on composited
    exposure-safe display backends (`RawDisplayHandle::Wayland` / `Web`); on
    X11 and Win32, where winit coalesces OS exposure events into
    `RedrawRequested`, `blit_canvas_rect` still skips rewriting unchanged
    backbuffer pixels and then calls full `buffer.present()` so any exposed
    window region outside `damage` is always repainted.
  - Proven steady-state damage invariants:
    - **Lot screen (`Game::SteadyFrameKey`):** `LotCache::last_hit() == CacheHit::Full`,
      Live mode with no dialogs/menus/highlights/overlays/animations/blink change
      or active/starting co-op overlay, unchanged HUD items (`hud_items`) and
      world inputs (`prev_dynamic_rects ∪ cur_dynamic_rects` covering all 3D
      Sims, plumb bob, and `draw_held` objects across every unpresented `draw()`),
      invalidated whenever a pie-menu check tree runs (`Game::list`) or a shell
      modal overlay (`save_manager` / `album` / `travel_dialog`) is composited.
    - **Neighborhood screen (`NeighborhoodSteadyKey` + `NeighborhoodMap::render_into_with_damage`):**
      cached static base-layer hit, unchanged chrome hover/press/water/pointer/state
      with no dialogs or filters (`prev_nb_dynamic_rects ∪ cur_nb_dynamic_rects`
      covering all fading/hovered and transitively overlapping dynamic lots).
  - `on_presented()` is called **only after** `buffer.present()` /
    `buffer.present_with_damage()` returns `Ok(())`; any intermediate non-steady
    frame or presentation error forces a full blit on the next present.

## Memory Traffic Impact (`1024×768` = 786,432 pixels)

| Metric | Before (Wave A `f7d52c07`) | After (Wave B) | Delta |
|---|---:|---:|---:|
| Single `Canvas` (`color + depth + ids`) | 8.65 MB (11 B/px) | **6.29 MB (8 B/px)** | **−27.3%** |
| Resident `Game` buffers (`3× Canvas + lot_depth + people_ids`) | 32.25 MB | **18.91 MB** | **−41.4% (−13.34 MB)** |
| `Game::render` steady-state Tier-2 copies / frame | 14.95 MB/frame | **6.33 MB/frame** | **−57.7% (−8.62 MB/frame)** |
| `lot_depth` copy / frame (cached draws) | 3.15 MB/frame | **0 B/frame** | **−100%** |
| `people_ids` copy / frame | 3.15 MB/frame | **< 0.04 MB/frame** | **−98.7%+** |

## `--release` Benchmark Results (Apple Silicon, `1024×768`)

Measured against untouched `origin/main` (`f7d52c07`, with Wave A `#378`–`#380`
merged) on the exact same `#[ignore]` harnesses:

| Benchmark (`1024×768`, `--release`) | Before (`f7d52c07`) | After (Wave B) | Speedup |
|---|---:|---:|---:|
| `Game::render` steady (Tier-2 hit, 60 frames) | 1.369 ms/frame | **0.683 ms/frame** | **2.00×** |
| `Game::render` uncached pan (20 frames) | 6.569 ms/frame | **2.184 ms/frame** | **3.01×** |
| `blit_canvas` 1× (`1024×768`, checksum `00e2d638`) | 0.1069 ms/frame | **0.0430 ms/frame** | **2.49×** |
| `blit_canvas` 2× (`2048×1536`, checksum `00e2d638`) | 0.2970 ms/frame | 0.3188 ms/frame | 0.93× *(~40 GB/s write-bandwidth bound)* |
| `LotCache` pan — Far L1 Cutaway | 1.966 ms/frame | **1.617 ms/frame** | **1.22×** |
| `LotCache` pan — Med L1 Cutaway | 2.080 ms/frame | **1.661 ms/frame** | **1.25×** |
| `LotCache` pan — Near L1 Cutaway | 1.907 ms/frame | **1.463 ms/frame** | **1.30×** |
| `LotCache` pan — Med L2 Up | 2.264 ms/frame | **1.992 ms/frame** | **1.14×** |
| Direct `draw_lot_with_preview` — Med L1 Cutaway | 1.819 ms/frame | **1.489 ms/frame** | **1.22×** |
| Neighborhood Map — HoverFade + Highlight | 0.285 ms/frame | 0.291 ms/frame | 0.98× |
| Neighborhood Map — HoverFade | 0.279 ms/frame | 0.289 ms/frame | 0.97× |

## Correctness Verification

- `sims-render::canvas::tests::compact_canvas_helpers_depth_clamping_and_sparse_ids_parity`:
  verifies `pack_rgb`/`unpack_rgb`, `clamp_depth` boundary clamping (`i32::MIN`, `-1`, `0`, `65535`, `65536`, `i32::MAX`), `ID_NONE` sentinel + ID `0` validity, and sparse `Ids::take_rects` parity.
- `sims-render --test band_parity`: **0 differing pixels** across 1 vs 4 vs 8 bands on `House05.iff`, `House02.iff`, and `House08.iff`.
- `sims-game --bin sims-game blit_canvas`: verifies `blit_canvas` and `blit_canvas_rect` bit-for-bit parity across scales `1..=4`, odd/clamped sizes, interior/edge/out-of-bounds rects.
- `sims-game --test perf_part4 wave_b_picking_parity_and_steady_present_damage_completeness`:
  verifies `lot_depth_from_cache() == true`, `sparse_bytes < 40 KB`, bit-exact `lot_depth_slice()`, **0 `Game::under` mismatches** across all Sim/plumb-bob pixels and dense `1024×768` grid vs full-screen `lot_depth` + `Ids::take`, and **0 changed pixels outside `present_damage`** across all 786,432 pixels on steady plumb-bob frames.
- All 5 workspace release gates pass with **0 lowered ratchet floors** and unchanged 43,200-tick Goth simulation hash `df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a`.

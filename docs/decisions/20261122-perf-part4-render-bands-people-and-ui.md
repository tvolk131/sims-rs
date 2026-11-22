# Performance Part 4 — Wave A (Track 5): Precompiled Sprite Spans, Multi-Threaded Band Rasterization, 3D People, Neighborhood Map & UI Blitter

*Decided 2026-11-22.*

This decision records six strictly **100% correctness-neutral** rendering,
3D people, neighborhood map, and UI blitter optimizations (Audit Items 2.2,
2.3, 2.4, 2.5, 2.6, 2.7 plus cached `Palette` decoding / LRU-bounded raw IFF
cache from Item 4.1) landed in Wave A Track 5
(`perf/render-spans-bands-people-nb-hud`).

All public `Canvas` fields (`color`, `depth`, `width`, `height`), `Image`
fields, `Assets` signatures, `NeighborhoodMap` public fields, and `People` /
`Target` public APIs remain 100% backwards-compatible (strictly additive
helpers only), with **zero new external crates** and **100% safe Rust
(`unsafe_code = "forbid"`)**.

## Summary of Changes

1. **Precompiled Active Row Spans, Hoisted Light LUTs, `Light::FULL` Fast Path & Palette Caching (Audit Items 2.2 & 4.1)**
   - `crates/sims-render/src/canvas.rs`, `light.rs`, `assets.rs`:
     - Added `RowSpan` / `RowSpans` (`start`, `end`, `all_opaque`) computed once
       when sprites are decoded and stored in `Assets`'s internal cache wrappers
       (`CachedImage`, `CachedQuad`, `CachedTrimSet`, `CachedGroup`,
       `PoolCorner`), leaving `sims_render::Image` public fields unchanged.
     - Hoisted `light::tables()` once outside row loops and specialized
       `Light::FULL` + `all_opaque` spans (and `mask_span.all_opaque` on masked
       walls) into tight auto-vectorizable loops that skip transparent margins
       (50–80% of isometric diamonds & object sprites).
     - Added `Palette` caching per `(resource, palt_id)` in `Assets` and
       LRU-bounded `Assets::files` (`MAX_CACHED_FILES = 96`) so multi-lot /
       catalog browsing avoids unbounded raw IFF retention while keeping active
       lot decode hit rates warm.

2. **Multi-Threaded Horizontal-Band Software Rasterization (Audit Item 2.3)**
   - `crates/sims-render/src/scene.rs`, `canvas.rs`, `grass.rs`:
     - Collected sorted draw commands (with `[y0, y1)` screen bounds and warmed
       `&Image` / `&RowSpans` references) on the main thread, recording
       `Canvas::record_rect_for` deterministically before rasterization.
     - Split `Canvas` into disjoint horizontal row bands (`Canvas::split_bands`
       via safe `chunks_mut`) and rasterized bands in parallel with
       `std::thread::scope` (`1..=8` bands), preserving exact draw order,
       depth comparisons, 5-6-5 alpha blends, and `Canvas::drawn` rects.
     - Added `grass::Column::rows_rgb_in_band(band_y0, band_y1)` to preserve
       the exact unbanded dither parity `(self.y + self.height + row) & 1`
       across arbitrary band splits.
     - Added `crates/sims-render/tests/band_parity.rs::single_thread_and_multi_band_render_bit_for_bit_identically`
       proving **0 differing pixels** across `color`, `depth`, `ids`, every
       object's `drawn(id)` rect, and `Stats` comparing 1-thread vs 4-band vs
       8-band rendering on `House05.iff` (Far/Med/Near × rotations 0–3 × levels
       1–2 × Up/Cutaway/Down × day/night × shadows), `House02.iff`, and
       `House08.iff`.

3. **3D People Zero-Alloc Update, Scratch Buffers, Pose/Skin Cache & Exact Scanline Clipping (Audit Item 2.4)**
   - `crates/sims-people/src/{skin.rs, raster.rs, lot.rs}`:
     - Added `Key::matches(&self, guid, person)` comparing borrowed `&str`
       fields in `People::update` (zero `[String; 10]` + `Vec<(String, String)>`
       clones when unchanged).
     - Added `skin::skin_into` and per-Sim `CachedPose` (keyed on
       `animator.position`, `ground_height.to_bits()`, `head.turn`,
       `animator.base`, `base_layers`, `foreground`, `carry`, and fallback
       clock/clip) so steady-state frames between 24 Hz simulation ticks skip
       re-posing and re-skinning, and dynamic frames reuse `Skinned` buffers in
       place.
     - Added `PeopleScratch` (reusable `shades`, `screen`, `bob_skinned`,
       `lamps`, flat `LitLamps` without per-frame `BTreeMap<_, Vec<_>>` churn,
       `ready`, and `draw_order`).
     - Added dyn-compatible row-indexed fast-path methods (`row_index`,
       `passes_at`, `write_passed_at`, `write_coloured_passed_at`) on
       `raster::Target` and implemented them for `raster::Canvas`,
       `sims_render::Canvas` (via additive `Canvas::depth_passes_at` /
       `Canvas::put_passed_at`), and `Packed565<T>`.
     - Added monotonicity-verified integer `[x_start..=x_end]` scanline span
       clipping (`clip_edge_span`) for wide triangles in `raster::triangle`
       while **strictly preserving exact `v0 / area`, `v1 / area`, `v2 / area`
       division** so zero boundary texels ever flip.

4. **Neighborhood Map Brighten LUT, Highlight & Static Base-Layer Cache & Row-Sliced Blitters (Audit Item 2.5)**
   - `crates/sims-render/src/neighborhood.rs`:
     - Added a 65,536-entry `OnceLock<Box<[u16; 65536]>>` LUT for
       `brighten_565(p, 1.0, HIGHLIGHT_Y_OFFSET)` (verified bit-exact across all
       65,536 inputs) and hoisted it outside the pixel loop in
       `highlight_row_in_place`.
     - Added an interior-mutable `RenderCacheSlot` on `NeighborhoodMap` (keeping
       `render_into(&self, ...)` unchanged and `NeighborhoodMap: Debug + Clone + Send + Sync`)
       caching both highlighted lot surfaces and the static base layer
       (`large_frame + backdrop + pre-lot wave + non-dynamic static lots`),
       validated by fast 4-way unrolled content fingerprints so any external
       field mutation invalidates immediately.
     - Excluded hovered, fading, and badged lots — plus any later Z-order lots
       transitively overlapping an excluded lot's clipped screen rect — from the
       static base layer so both Z-order and overhanging-roof backdrop
       cross-fading remain 100% bit-exact.
     - Row-sliced `blit_surface_to_canvas`, `blit_crossfade_to_canvas`,
       `mark_surface_ids`, `blit_surface_stretched_to_canvas`, and
       `blit_subrect_to_canvas_mode`, and hoisted `blend_table()` + direct 5-6-5
       channel extraction in `blit_crossfade_to_canvas` without `widen_565`
       round-trips.

5. **UI `Frame::draw` Row-Sliced Blitter (Audit Item 2.6)**
   - `crates/sims-ui/src/frame.rs`:
     - Sliced `src_row` and `dst_row` once per scanline, hoisted
       `blend_table()` outside loops, and specialized `alpha <= 7` /
       `alpha >= 248` directly on row slices (preserving fallback behavior for
       truncated source buffers).

6. **Floor/Object Micro-Optimizations & `LotCache::cached_depth` (Audit Item 2.7)**
   - `crates/sims-render/src/scene.rs`, `crates/sims-render/src/cache.rs`:
     - Replaced 4-corner `min_by_key` in `Painter::floors` with constant
       rotation-indexed `top_offset`, added conservative slope-padded viewport
       culling before floor/pool/water/shadow lookups (preserving `stats.floors`
       via `Assets::has_floor`), and borrowed `&'a Object` directly in reusable
       `Painter::object_scratch` without per-frame `Vec` allocations.
     - Added additive `pub fn LotCache::cached_depth(&self) -> Option<&[i32]>`
       for future zero-copy depth access.

## Correctness Verification

- **Bit-exactness proofs:**
  - `sims-render --test band_parity`: **0 differing pixels** (`color`, `depth`,
    `ids`, every object's `drawn(id)` rect, and `Stats`) across 1 vs 4 vs 8
    bands on `House05.iff`, `House02.iff`, and `House08.iff`.
  - `sims-render::neighborhood::tests`: **0 differing pixels** across all
    65,536 `brighten_565` LUT entries and cached vs uncached rendering
    (including overhanging roof cross-fades, overlapping Z-order lots,
    highlights, and in-place surface mutations).
  - `sims-people::raster::tests::clip_edge_span_never_skips_any_inside_pixel`
    and `sims-people --test lot cached_pose_and_skinning_match_fresh_people_bit_for_bit`:
    **0 differing pixels** (`color`, `depth`, `ids`).
- **Ratchets:** Zero lowered floors across `sims-game --test ratchet` (8/8),
  `sims-ui --test ui_ratchet` (4/4), and `sims-render --test neighborhood_map`
  (6/6, including 1.91 live oracles).

## `--release` Benchmark Results (Apple Silicon, 1024×768)

| Benchmark | Baseline (`origin/main`) | Optimized (Track 5) | Speedup |
|---|---:|---:|---:|
| Lot Pan (`LotCache`) — Far L1 Cutaway | 7.472 ms/frame | 2.143 ms/frame | **3.49×** |
| Lot Pan (`LotCache`) — Med L1 Cutaway | 10.101 ms/frame | 2.348 ms/frame | **4.30×** |
| Lot Pan (`LotCache`) — Near L1 Cutaway | 11.992 ms/frame | 2.040 ms/frame | **5.88×** |
| Lot Pan (`LotCache`) — Med L2 Up | 16.313 ms/frame | 2.434 ms/frame | **6.70×** |
| Direct `draw_lot_with_preview` — Med L1 Cutaway | 15.027 ms/frame | 1.847 ms/frame | **8.14×** |
| Neighborhood Map — HoverFade + Highlight | 5.995 ms/frame | 0.400 ms/frame | **14.99×** |
| Neighborhood Map — HoverFade | 5.623 ms/frame | 0.372 ms/frame | **15.12×** |
| 3D People Steady (`update + draw`, 4 Sims Near) | 0.243 ms/frame | 0.195 ms/frame | **1.25×** |
| UI `Frame::draw` (800×120 mixed-alpha) | 0.1645 ms/call | 0.0428 ms/call | **3.84×** |

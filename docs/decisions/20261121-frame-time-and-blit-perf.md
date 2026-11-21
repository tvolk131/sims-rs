# Frame-time, people lamp lookup, and Retina blit optimizations

*Decided 2026-11-21.*

Over 95% of CPU frame time in `sims-game` on `House05.iff` (`--release`) came
from three bottlenecks while simulation (`game.run` at ~2.5 ms/tick) and 2D HUD
drawing (~1–2 ms) were already fast:

1. **Redrawing the entire static lot (`Pass::Lot`) on every frame** even when
   only Sims, the spinning plumb bob, pointer hover, or a pie menu updated —
   combined with heavy per-frame allocations inside `sims_render::paint`
   (cloning floor, wall pattern, wall shape, wall pool, trim, and `Definition`
   buffers; recomputing grass blade scatter; per-pixel `f64` lighting math; and
   rebuilding `Grass` during `draw_held`).
2. **Quadratic room-lamp scan in `sims_people::lot::lamps`**, which iterated
   all `world.objects` for every tile in `room.tiles` (thousands of tiles for
   large/outdoor rooms) per Sim per frame (~16–30 ms/frame).
3. **Un-specialized Retina presentation in `App::redraw`**, which ran per-pixel
   integer division and `.get()` bounds checks across `2048×1536` physical
   pixels and unconditionally called `surface.resize` and `window.set_title`
   every frame.

## What was decided

All changes are **100% bit-for-bit correctness-neutral** (zero changes to
simulation state, pixels, depth, pick IDs, `Canvas::drawn` rects, or `Stats`;
all 8 SHA-256 hashes in `crates/sims-game/tests/ratchet.txt` remain unchanged):

1. **`sims_render::LotCache` + `sims-render` hot-loop speedups:**
   - Added `sims_render::LotCache` (`crates/sims-render/src/cache.rs`), wired
     into `Game::draw` with minimal changes in `crates/sims-game/src/game.rs`.
     It caches the completed `Pass::Lot` `Canvas` (`color`, `depth`, `ids`,
     `pen`, `rects`) and `Stats`, validating every input `Pass::Lot` reads:
     `Assets` instance identity, canvas size, `clip_bottom`, camera (`zoom`, `rotation`, `level`, `origin`,
     `walls`, and resolved 512-byte cutaway `Mask`), `preview`, `time_of_day`,
     `house`, Buy/Build grid flag + array `8`, exact `Lot` equality (with
     `f32::to_bits`), `holds_person(world)`, resolved tile shadows per active
     level (so walking Sims that mutate array `3` without casting shadows do
     not bust the cache, while any real shadow-chain `slots[0]` change
     invalidates), and all non-person object render fields (plus wall-shaping
     or preview-referenced objects).
   - Cached `Grass` tables inside `LotCache` across camera pans (since `Grass`
     is independent of `camera.origin`), and cached the zoom-only `scatter`
     tables in a `OnceLock` with zero-allocation column/blade visitors and a
     pre-widened RGB grass palette.
   - Replaced per-pixel `f64` math in `Light::apply` with 4 KB `OnceLock`
     5-6-5 lookup tables matching the original's `0x49eea8` startup tables.
   - Eliminated per-tile/per-wall `Image::clone()`, `WallPool::to_vec()`,
     `Definition::clone()`, `within()` allocations, offscreen floor/wall/shadow
     pixel work, `Grass::new` in `draw_held` (plus early return when no object
     holds a person), and `BTreeSet` allocation for uncontained objects in
     `held()`.
   - Hoisted floor row depth (`Depth::floor_base` / `Depth::floor_row`) once
     per visible row in `Canvas::draw_floor_shaded` and `Canvas::shade_mask`.
2. **`sims_people::draw_in_viewport` (`LitLamps` index + rasterizer hoisting):**
   - Replaced the `O(|room.tiles| × |world.objects|)` nested scan in
     `sims_people::lot::lamps` with `LitLamps` (single `O(|world.objects|)`
     pass once per frame in `draw_in_viewport` with a 64×64×2 presence bitmask
     and `BTreeMap<[i32; 3], Vec<&Object>>`), preserving exact `room.tiles`
     order, duplicate tile entries, ascending object ID order, and the
     `ROOM_LAMPS` (6) cap before `lamp_strength` distance filtering.
   - Hoisted canonical edge setup and per-row terms out of the inner pixel loop
     in `sims_people::raster::triangle` (preserving exact IEEE-754 operations),
     avoided interpolating unused RGB channels for grey-shaded vertices,
     normalized `lighting.towards` once per part, and eliminated the duplicate
     `Canvas::index` call in `Canvas::put`.
3. **`App::redraw` Retina presentation (`crates/sims-game/src/main.rs`):**
   - Added `blit_canvas` with zero-division specialized paths for `scale == 1`
     and `scale == 2` (horizontal pixel doubling + `copy_within` row doubling)
     and row-duplicating fallback for `scale >= 3`, matching the exact edge
     clamp on odd and non-multiple physical buffer sizes.
   - Cached `surface_size` (skipping redundant `surface.resize(w, h)` calls,
     resetting on `ScaleFactorChanged` or `buffer_mut` failure) and
     `last_title` (skipping redundant `window.set_title` calls).

## Before vs. After Benchmark (`House05.iff`, tick 250, `--release`, Apple Silicon)

Measured on the same machine (`cc99bfab` baseline vs. this branch):

| Scenario | Before steady (`render`+`blit`) | After steady (`render`+`blit`) | Steady speedup | Before pan (`render`+`blit`) | After pan (`render`+`blit`) | Pan speedup | `draw_lot` (cached / uncached) | `people` (before → after) | `blit` (before → after) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `1024×768 2× Far Cutaway` | `25.60 ms` | `1.31 ms` | **19.5×** | `25.95 ms` | `8.55 ms` | **3.0×** | `9.33 → 0.25 / 8.82 ms` | `13.97 → 0.24 ms` | `2.77 → 0.29 ms` |
| `1024×768 2× Med Cutaway` | `36.82 ms` | `1.42 ms` | **25.9×** | `39.01 ms` | `13.44 ms` | **2.9×** | `16.95 → 0.34 / 10.72 ms` | `13.14 → 0.41 ms` | `2.83 → 0.30 ms` |
| `1024×768 2× Near Cutaway` | `35.14 ms` | `2.12 ms` | **16.6×** | `34.72 ms` | `10.15 ms` | **3.4×** | `18.23 → 0.18 / 8.72 ms` | `15.48 → 0.13 ms` | `2.99 → 0.47 ms` |
| `1024×768 2× Med BuyMode` | `33.08 ms` | `1.45 ms` | **22.8×** | `33.03 ms` | `11.22 ms` | **2.9×** | `17.09 → 0.21 / 10.48 ms` | `15.67 → 0.33 ms` | `2.45 → 0.42 ms` |
| `1024×768 2× Med PieMenu` | `33.74 ms` | `2.03 ms` | **16.6×** | `38.72 ms` | `13.40 ms` | **2.9×** | `24.63 → 0.19 / 13.19 ms` | `35.88 → 0.45 ms` | `3.75 → 0.36 ms` |
| `800×600 1× Med Cutaway` | `34.64 ms` | `0.69 ms` | **50.2×** | `35.91 ms` | `8.08 ms` | **4.4×** | `19.21 → 0.12 / 7.29 ms` | `13.66 → 0.11 ms` | `0.35 → 0.07 ms` |

`sims_people::lot::lamps` across 4 posed Sims on `House05`: `16.549 ms → 0.021 ms` (**788× faster**).

## What would reopen this

- Adding any new world/object/lot field read by `sims_render::paint(..., Pass::Lot)`
  must also add that field to `LotCache`'s `LotKey` (and to
  `crates/sims-render/tests/lot_cache_parity.rs`).
- Future GPU or dirty-rectangle tile compositing could reduce pan frame times
  further if needed.

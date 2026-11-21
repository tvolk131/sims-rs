# Grass column clipping, sloped-ground object placement, and diagonal trim depth offsets

*Decided 2026-11-21.*

Three pure `sims-render` mismatches against Complete Collection 1.91 (`Sims.exe`) showed up across all eight Goth-house ratchet scenes: flipped dither parity on screen-edge grass columns, sloped-terrain shrubs drawn two pixels low at far zoom, and diagonal cut-wall trims clipping objects in front of them.

## What `Sims.exe` does

1. **Grass column vertical clipping and dither parity (`0x49dacd`, `0x49da21`).**
   For each 4-pixel column, `0x49dacd` generates blades (`0x49d79d`) on the unclipped column and then clips the column fill vertically to the active draw viewport `[clip_top, clip_bottom)` (`word ptr [0x6f0c10]`, `[0x6f0c14]`, set to `800×500` when the live/buy/build/options strip is expanded at `0x49aed6`–`0x49aee2`, `0x4b1392`–`0x4b13ad`, `0x4c44dd`–`0x4c4547`) on both halves (`0x49dcd3`–`0x49dd1f` left, `0x49dee0`–`0x49df2e` right), and clips blades (`0x49d956`–`0x49d96e`) to the same bounds:
   - When `y < clip_top`, `skip = clip_top - y`, `height -= skip`, `shade[0] += shade[1] * skip`, `color[0] += color[1] * skip`, and `y = clip_top`.
   - When `y + height > clip_bottom`, `height = clip_bottom - y`.
   - `0x49da21` initializes the dither row parity from the clipped `(y + height) & 1` (`0x49da59`) and flips it on each drawn row. Because `sims-render` previously iterated the unclipped column and let `Canvas::plot` drop off-screen pixels, an odd top clip or a bottom clip whose clipped and unclipped end rows have opposite parity inverted the dither parity for the entire visible column.

2. **Object terrain altitude, held-slot pose, and sloped-ground screen projection (`0x4b9613`, `0x4b97ea`, `0x4b9991`, `0x4a9dc7`, `0x478dfe`, `0x437fdd`).**
   - Unheld objects enter `0x4aa430` -> `0x4b9613`, which reads terrain altitude from `0x478dfe` (`0x478e2a`–`0x478e4e`, indexing the raw 64×64 altitude array via `sims_sim::head::altitude_at` with `(y & 15, x & 15)`, without the active-size check) rather than the four-corner mean. `0x4b97ea` (`0x4b984f`–`0x4b988e`) rounds that altitude to a whole step `steps = (altitude + 0.5).trunc() as u8` and calls `0x4a9dc7` (`0x4a9e9b`) with `alt_ptr = &steps`, lifting the sprite by `steps << (zoom + 1)`.
   - Held objects (`0x4378fa` -> `0x437c56` -> `0x437fdd` -> `0x438068`) traverse slots even when a holder is hidden, clear both horizontal offsets for odd holder facings (`0x438044` -> `0x439109`), and accumulate each slot sequentially onto the running `f32` pose in root-to-child order (`0x438049`–`0x438090`) before calling `0x4b9991`.
   - Draw-group sprites with nonzero `[x, y, z]` world offset (`0x437af2`–`0x437bbb`, where x87 `fcomp 0.0; jne` keeps both `0.0` and `NaN` on the base path) also call `0x4b9991`. `0x4b9991` projects the rotated rounded fine position through `0x4a9dc7` with `alt_ptr = NULL` (`0x4a9e22`–`0x4a9f90`, interpolating the view tile's rotated terrain heights across its two triangles inside `1..size - 1`, or `[0; 4]` outside), subtracts `((total_z - altitude_at(fine)) * (2 << zoom) + 0.5).trunc()` (`0x4b9a09`–`0x4b9a3f`), and anchors depth from that screen row and `total_z` (`0x4b9a47`–`0x4b9a8e` -> `0x4c9ceb`).
   - Previously, `Scene::objects` averaged all four tile corners and divided by `near_scale()`, putting shrubs on half-step slopes two pixels too low at far zoom.

3. **Flat wall-trim depth offsets (`0x53c478`–`0x53c4af`, `0x53c65f`–`0x53c675`, `0x4c8849`).**
   The flat-depth trim drawer `0x4c8849` (`0x4c8a74`–`0x4c8aa8`) adds its caller's depth offset argument `[ebp + 0x28]` to `ground(corner, height) + (LEVEL_DEPTH + 65) * level + correction` before clamping to `0xffff` (`EMPTY`):
   - Cut diagonal corner records (`is_record && bits & 0x50 != 0`, `0x53c41d`–`0x53c4fa`): `+900` (`0x384`) for `0x10`, overridden by `+770` (`0x302`) for `0x40` on level != 2 and `+700` (`0x2bc`) on level 2.
   - Standing corner records (`0x53c65f`–`0x53c675`): `-100` (`-0x64`) when `bits & 0x40 != 0` on level 2, `0` otherwise.
   - Wall-run trims (`0x540473` -> `0x53db67`): cut runs always use the depth-sprite drawer `0x4c8b1f` (`0x53e2fa`–`0x53e362`), with runs along view `(1, 1)` (`bits & 0x50 != 0`) raised by `2 >> (3 - zoom)` (`[0, 1, 2]` px at far/medium/near, `0x53e300`–`0x53e313`); standing runs pass offset `0` to `0x4c8849` (`0x53e379`–`0x53e3b4`).
   - Previously, `Depth::flat` used offset `0` for all flat trims, placing cut diagonal trims 700–900 units too near so they occluded objects in front of them (such as the patio cactus outside the Goth bathroom in `near-r3-down-0816`).

## What changed

- `crates/sims-render/src/grass.rs` (`Column::clipped`), `crates/sims-render/src/scene.rs` (`draw_lot_in_viewport`, `Painter::terrain`), `crates/sims-render/src/lib.rs`, and `crates/sims-game/src/game.rs` (`Game::draw`): clip grass columns and blades vertically to the active house viewport `[0, clip_bottom)` before filling rows while keeping blade generation on the unclipped column.
- `crates/sims-render/src/lot.rs` (`Lot::altitude_at`) and `crates/sims-render/src/scene.rs` (`held`, `project_fine_terrain`, `place_base`, `place_moved`, `Painter::objects`): match `0x478dfe` (64×64 raw array), `0x4378fa`/`0x437fdd`, `0x4a9dc7`, `0x4b97ea`, and `0x4b9991` for base, held, and offset object sprites.
- `crates/sims-render/src/depth.rs` (`Depth::flat`) and `crates/sims-render/src/scene.rs` (`flat_trim_offset`, `Painter::trims`): apply `0x4c8849`'s exact per-trim depth offset for corner records and route non-record cut wall runs through the depth-sprite path.
- Updated pinned render digests in `crates/sims-render/tests/goth_house.rs` (far-zoom renders plus medium/near rotation-0 cutaway) and `crates/sims-game/tests/ratchet.txt`.

## Ratchet results (against `origin/main` `1e7636f6`)

All eight scenes improved with zero regressions:

| Scene | `frame` | `surface` | `objects` | `people` | `panel` |
|---|---|---|---|---|---|
| `near-r2-0712-bench` | 99.26% → **99.37%** (+0.11) | 99.67% → **99.81%** (+0.14) | 99.67% → **99.72%** (+0.05) | 46.16% | 99.93% |
| `near-r2-0805` | 99.32% → **99.48%** (+0.16) | 99.59% → **99.81%** (+0.22) | 99.89% → **99.94%** (+0.05) | 73.66% | 99.99% |
| `far-r0-cut-0816` | 99.52% → **99.79%** (+0.27) | 99.60% → **99.84%** (+0.24) | 99.13% → **99.89%** (+0.76) | 0.77% | 99.88% |
| `far-r1-cut-0816` | 99.46% → **99.86%** (+0.40) | 99.51% → **99.91%** (+0.40) | 99.20% → **99.88%** (+0.68) | 9.36% | 99.88% |
| `medium-r1-cut-0816` | 99.50% → **99.55%** (+0.05) | 99.66% → **99.72%** (+0.06) | 99.99% | 9.28% | 99.87% → **99.88%** (+0.01) |
| `near-r3-down-0816` | 98.32% → **98.40%** (+0.08) | 99.58% → **99.69%** (+0.11) | 94.46% → **94.50%** (+0.04) | — | 99.88% |
| `near-r0-up-0816` | 99.51% → **99.94%** (+0.43) | 99.34% → **99.95%** (+0.61) | 99.93% | — | 99.88% |
| `night-far-r0-cut-2153` | 99.54% → **99.90%** (+0.36) | 99.78% → **99.98%** (+0.20) | 96.61% → **100.00%** (+3.39) | — | 99.19% |

## Remaining unverified / out of scope

- Floor shadows on sloped tiles (`0x4b00b6`–`0x4b010c`) still sit at the tile's top corner height; a ~30 px oval remains beneath the far-zoom sloped backyard bush in `far-r0-cut-0816`.
- `0x49dacd`'s horizontal 4-pixel-aligned column clipping (`0x49db19`–`0x49db47`) is not added here because our camera origin is shifted by 1 px relative to the original's grid; `Canvas::plot` continues to clip horizontally per pixel.

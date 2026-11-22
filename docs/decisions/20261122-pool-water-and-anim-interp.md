# Swimming pool (`Arry 9`), pond water (`Arry 10`), `cAnimLayer::Update` interpolation, and split-diagonal roof triangle clipping

*Decided 2026-11-22.*

Ports Complete Collection swimming pool (`Arry 9`) and pond/natural water (`Arry 10`) rendering from
`GameData/floors.iff`, implements `cAnimLayer::Update`'s (`0x597f1b`–`0x597f38`) per-layer
`interpolate` threshold without changing `World` serialization or the 43,200-tick Goth day hash,
and refines split-diagonal roof clipping in `build_preview.rs` from `0.5`-tile sub-cells to exact
diagonal triangles.

## Disassembly findings (`Sims.exe`)

### 1. Swimming Pool (`Arry 9`, `0x4ae63b`–`0x4ae83e`)

- **Array identity (`0x478556`–`0x478581`, `0x478f42`):** `[lot + 0x60]` is `Arry 9` (swimming
  pool); `[lot + 0x64]` (`0x478f46` / `0x478f4a`) is `Arry 10` (pond/natural water).
- **Encoding & rotation (`0x4614d6`, table `[0x6f4b70]` built at `0x53d310`–`0x53d39c`):**
  Values `0..=15` encode 4-bit cardinal pool connectivity (`1 = N (-y)`, `2 = W (-x)`,
  `4 = S (+y)`, `8 = E (+x)`), value `0x10` encodes a pool deck tile (observed in `House28.iff`),
  and values `> 0x10` (`0xff`) are absent. Rotation right-shifts the low nibble by `rotation & 3`
  (`1 → 8 → 4 → 2 → 1`) while preserving the high nibble.
- **Sprites & placement (`0x4ae63b`–`0x4ae70f`):** Drawn on level 1 after floors and before object
  shadows (`0x4ae844`). For `rot_val < 0x10`, `Sims.exe` draws `GameData/floors.iff`
  `SPR2 0x400 + zoom * 16 + rot_val` (frame 0; far `31×16`, medium `63×32`, near `127×64`); for
  `rot_val == 0x10`, it draws built-in floor `12` (`12 + [0, 256, 512][zoom]`). Position uses
  `shadow_origin`, Z-tested via `0x4c811f` (`d <= dst_d`, room light + lamp pools).
- **Inner-corner coping patches (`0x4ae710`–`0x4ae83e` → `0x461e39`, table `[0x6e52a0..0x6e52c8]`
  at `0x461d2f`–`0x461e38`):** Four rules inspect `self_val` and two orthogonal neighbors to detect
  L-shaped inner corners, rotate the 4-bit mask right by `rotation & 3`, and draw
  `SPR2 0x430 + zoom * 4 + corner` at `at + [frame.x, frame.y]` with depth bias `-1`
  (`0x4ae744: push -1`) and `first_row` anchored at `at[1]` prior to adding `frame.y`
  (`0x4c8345`–`0x4c8351`).
- **Verification against `House02.iff` (`h02-g.png`):** Across the `135×160` (`21,600 px`) pool
  bounding box (`x = 665..800, y = 75..235`), the swimming pool water, straight coping edges,
  L-shaped inner-corner coping patches, diving board, and ladder match `h02-g.png` pixel-for-pixel
  (excluding Buy Mode's dotted grid and the ladder's `ZERO_EXTENT` water-tile floor shadow, which is
  an unrelated pre-existing shadow-caster filter).

### 2. Pond / Natural Water (`Arry 10`, `0x4af9d3`–`0x4b010c` → `0x4c1eea`–`0x4c2059` → `0x4c6594`)

- **Sprites & placement (`0x4afa09`–`0x4afad8`):** Drawn on inner level-1 tiles
  (`1 <= x, y < size - 1`) when `Arry 10 != 0xff`. Uses the same 4-bit cardinal right-rotation
  (`0x4614d6`) to select `GameData/floors.iff` `SPR2 0x800 + rot_val`, frame `3 - zoom1 = 2 - zoom`
  (`2` far `31×16`, `1` medium `63×32`, `0` near `127×64`). Placed at
  `[top_screen[0] - half_w - 1, top_screen[1] - (4 if Near else 1)]`.
- **Alpha-blended ground blitter (`0x4c7866` → `0x4c1a0c` with `[ebp + 0x20] = 1` → `0x4c1eea` →
  `[0x6f0bfc] = 0x4c6594`):** Crucially, `SPR2 0x800..=0x80f` stores every pixel as an `op = 1`
  pair `(byte0, color_idx)` where `byte0` (normally the SPR2 depth channel) is an **8-bit alpha
  mask**, quantized to 5 bits (`a5 = byte0 >> 3`):
  - `a5 == 0` (`byte0 < 8`): skipped completely (hiding the rectangular chroma-key corners outside
    the isometric diamond).
  - `1 <= a5 <= 30`: alpha-blended over the terrain canvas via `0x49a60b` (`blend565_with`) for
    smooth shoreline transitions, unconditionally writing row depth `ground + 0x50` (`floor_base + 15`,
    stepping along `R[y - origin_y]` anchored at `at[1]`, `0x4c1f9d`–`0x4c1fb1`).
  - `a5 == 31` (`byte0 >= 248`, including the `254` water interior): opaque copy with the same
    unconditional row depth write.
- **Verification:** Visually verified on `House08.iff` (river/pond) and `House28.iff` (beach pond
  + pool deck `0x10`), and integration-tested across all 4 rotations × 3 zooms on House02, House08,
  House28, and Build Mode Part 2 dug pools & painted ponds (including `LotCache` invalidation via
  `Lot::exact_eq`).

### 3. `cAnimLayer::Update` (`0x597f1b`–`0x597f38`) Per-Layer `interpolate` Flag

- **Formula (`0x597f1b`–`0x597f38`, `0x59844d`–`0x5985f8`):** Under x87 24-bit significand
  precision, `cAnimLayer::Update` computes `frame_ms = duration / frames`, `threshold = 2 * frame_ms`,
  and `elapsed = float(current - previous)`. It sets `interpolate = true` iff `elapsed < threshold`
  (or unordered), and clears `interpolate = false` when `elapsed >= threshold`, snapping directly to
  keyframe `floor(phase * frames)` while preserving `weight < 1.0` pose blending.
- **Hash- & serialization-preserving design:** Implemented as `Layer::interpolate(&self) -> bool` in
  `crates/sims-sim/src/animator.rs` (zero new struct fields, keeping `World` serialization and the
  43,200-tick Goth day hash `df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a`
  untouched), paired with `Clip::apply_with_interpolation` in `crates/sims-people/src/animation.rs`
  and `Player::pose_with_layer_interpolation` in `crates/sims-people/src/person.rs`.
- **Why `Player::pose` keeps interpolating by default (forensic finding on `near-r2-0712-bench`):**
  Wiring `layer.interpolate()` unconditionally into `Player::pose` was tested against all eight
  frozen Goth ratchet screenshots. On `near-r2-0712-bench.ppm` (tick `1255`, the only ratchet scene
  where a `speed = 1.4` Sim aligns at the exact tick and position of the reference capture),
  Cassandra's `C2O-sit-front` (`dur = 2166.43 ms`, `frames = 91`, `2 * frame_ms = 47.614 ms < 50 ms`,
  `phase * 91 = 31.503458`) matches the frozen `Sims.exe` screenshot at **1,481 / 1,485 (99.73%)**
  when interpolated, but drops to **545 / 1,485 (36.7%)** (`people`: `99.67% → 71.63%`, `frame`:
  `99.99% → 99.77%`, `surface`: `100.00% → 99.98%`, `objects`: `100.00% → 99.94%`) when snapped to
  frame `31`. Disassembly of `cSimAnimator::Draw` (`0x590ed1`–`0x590f2b`) and `DrawThumbnail`
  (`0x5934a8`) explains why: changing walls/detail (Cutaway → Down before capturing
  `near-r2-0712-bench.ppm`) invalidates the Sim's portrait (`0x4eb0eb`), whose redraw invokes
  `cAnimSkin::Update(clock, 1, force=1)` at the unchanged clock (`elapsed = 0 < 47.614`), re-enabling
  interpolation at the exact same phase (`31.503458`). Because `sims-vm` keeps the simulation
  independent of UI/camera detail changes (and the remaining 4 `people` scenes carry 1-to-22 tick
  replay drift where snapping lowers `people` by `0.01%`–`0.46%`), `Player::pose` preserves its
  interpolated default (zero ratchet regressions, zero lowered floors) while exposing the exact
  `0x597f1b`–`0x597f38` path through `Layer::interpolate`, `Clip::apply_with_interpolation`, and
  `Player::pose_with_layer_interpolation`.

### 4. Exact Split-Diagonal Roof Triangle Clipping (`crates/sims-render/src/build_preview.rs`)

- Replaced the `0.5`-tile sub-cell boolean approximation in `draw_roof_layer` with exact per-triangle
  `SubCell::{Empty, Full, Split0011(u8), Split1001(u8)}` clipping against `Lot::halves` /
  `Lot::room_at` centroids, setting eave distance `0` on non-full sub-cells and eliminating roof
  overhang onto the outdoor triangle of split-diagonal tiles.

## Changes

- `crates/sims-render/src/lot.rs`: Added `Lot::pool` (`Arry 9`) and `Lot::water` (`Arry 10`),
  loaded in `Lot::from_world` and compared in `Lot::exact_eq`.
- `crates/sims-render/src/assets.rs`: Added cached `pool_tile` / `pool_tile_with_pools`,
  `pool_corner` / `pool_corner_with_pools`, and `water_tile` loaders from `GameData/floors.iff`.
- `crates/sims-render/src/canvas.rs`: Added `Canvas::draw_water_overlay` implementing `0x4c6594`'s
  5-bit alpha-blended ground overlay blitter and unit test coverage.
- `crates/sims-render/src/scene.rs`: Added `rotate_cardinal_mask`, `pool_inner_corners`,
  `Painter::pool`, `Painter::water`, and unit tests.
- `crates/sims-render/src/build_preview.rs`: Upgraded split-diagonal roof clipping to exact diagonal
  triangles (`SubCell`) and added unit test coverage.
- `crates/sims-render/tests/pool_water.rs`: Added live-install integration tests for House02
  swimming pool, House08/House28 pond water & pool deck, and Build Mode Part 2 dug pools & painted
  ponds across all 4 rotations × 3 zooms with `LotCache` invalidation.
- `crates/sims-sim/src/animator.rs`: Added `Layer::interpolate` (`0x597f1b`–`0x597f38`) and unit
  tests verifying exact boundary agreement with `pose::Selection::at`.
- `crates/sims-people/src/{animation.rs,person.rs}`: Added `Clip::apply_with_interpolation` and
  `Player::pose_with_layer_interpolation` with unit tests.

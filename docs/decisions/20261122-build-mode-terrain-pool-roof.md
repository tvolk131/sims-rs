# Build Mode Part 2: Terrain, Water/Pool Digging & Roof Style/Pitch Engine

*Decided 2026-11-22.*

## Context

Build Mode Part 1 (`20261121-build-mode-core.md`) ported Wall & Fence, Floor,
Wallpaper, Doors/Windows/Stairs/Fireplaces/Plants, Hand Tool, and unified
Build/Buy Undo/Redo. Three remaining Build Mode tools (`BuildTool::Terrain`,
`BuildTool::Water`, `BuildTool::Roof`) plus a narrow-viewport (`< 620 px`)
Build tool-strip overlap nit remained to complete the `sims-vm` and `sims-ui`
Build Mode engine.

## Decisions

### 1. Native Complete Collection 1.91 Data Representations
Verified directly against Complete Collection 1.91 game assets (`GameData/Build.iff`,
`UIGraphics/Res_CPanel.h`, `GameData/Roofs/*.bmp`, and `UserData/Houses/House*.iff`):

- **Terrain heights (`Arry 0`)**: 64×64 array with stride `4` (`[u8; 4]` per
  tile). Tile `(tx, ty)` stores its four vertex corners in order
  `[SW(tx, ty+1), NW(tx, ty), NE(tx+1, ty), SE(tx+1, ty+1)]` (matching
  `sims-render/src/grass.rs`). Editing a grid vertex `(vx, vy)` updates all
  four incident tile corners simultaneously.
- **Grass & soil (`Arry 6`, `Arry 7`)**: Signed `i8` per tile. Raising,
  lowering, or leveling terrain drops disturbed tiles in `Arry 6` to
  `TERRAIN_DIRT_GRASS = -100` (`< SPARSE = -73` in `sims-render/src/grass.rs`)
  while leaving `Arry 7` (`TARGET_GRASS_ARRAY`) intact so `World::grow_grass`
  naturally regrows the lawn over time. The Grass tool (`STR# 128[4]`, `$1`)
  paints `Arry 6` to `GRASS_RESTORED_VALUE = 100` (`> DENSE = 85`, or higher
  if `Arry 7` exceeds `100`); Ctrl-click removes grass back to `-100` for `$0`.
- **Swimming pool (`Arry 9`) & Pond/Water (`Arry 10`)**: 64×64 stride-1 byte
  arrays where `0xff` denotes dry land and values `0x00..=0x0f` encode a 4-bit
  cardinal neighbor connectivity mask (`0x01` North `y-1`, `0x02` West `x-1`,
  `0x04` South `y+1`, `0x08` East `x+1`), verified against `House02.iff`,
  `House08.iff`, `House10.iff`, and `House22.iff`.
- **Pricing (`GameData/Build.iff`)**:
  - `STR# 128` (`Terrain`): Raise Terrain `$1`, Lower Terrain `$1`, Level
    Terrain `$1`, Grass `$1`.
  - `STR# 133` (`Pool`): Pool Tool `$75` (80% demolish refund `$60`), Water
    Tool `$15` (80% remove refund `$12`).
  - Roof pattern & pitch changes: `$0`.
- **Roof style & pitch (`HOUS 0` & `GameData/Roofs/*.bmp`)**:
  - `HOUS 0` stores a 4-byte little-endian IEEE-754 `f32` slope pitch
    (`0.0` Flat, `0.4` Shallow, `0.6` Medium, `1.0` Steep) immediately followed
    by the null-terminated texture stem (e.g. `r1_`..`r4_`, `gravel1`, `tar1`,
    `Superstar*`, `MM*`; 18 `32×32` 8-bit BMPs in `GameData/Roofs/`).
  - `RoofState::from_house_view` decodes `HouseView::unknown_word` (`f32::from_bits`)
    as the roof pitch and `HouseView::roof` as the texture stem.

### 2. Simulation Planning, Guards, Persistence & Unified Undo/Redo (`sims-vm::build`)
- **Terrain (`plan_terrain_edit`, `build_commit_terrain`)**:
  - Supports `Raise`, `Lower`, `Level` (anchor vertex height per `STR# 128[3]`),
    `LevelAverage` (rounded mean), `RestoreGrass`, and `RemoveGrass`.
  - Enforces buildability guards on all four incident tiles of every edited
    vertex: inside `2..=active-2`, off-road (`FLOORS[0] not in 9..=13`),
    unlocked (`Arry 8` bit `0x20` clear), no walls or floors on level 1 or 2,
    no pool (`Arry 9 < 16`) or water (`Arry 10 < 16`), and no non-cursor on-lot
    objects.
  - Enforces `MAX_TERRAIN_SLOPE = 5` (matching `sims-render/src/grass.rs`'s
    `[-5, 5]` lighting table) on single-vertex edits and via feasible-interval
    relaxation on `Level`/`LevelAverage` drags so boundary edges never exceed
    the slope limit.
  - Rebuilds rooms and `FLAG_BUILDABLE` flags (`rebuild_architecture_rooms`)
    whenever terrain heights change, and charges Architecture budget category `7`.
- **Water/Pond (`plan_water_edit`, `build_commit_water`)**:
  - Paints or Ctrl-removes `Arry 10` tiles with automatic 4-bit connectivity
    recomputation across modified tiles and neighbors, depresses mutable basin
    vertices on elevated terrain (without disturbing locked/wall/floor/pool/object
    banks), protects locked river tiles (`FLAG_LOCKED = 0x20`), charges `$15` /
    refunds `$12` (80%) to Architecture category `7`, and handles optional
    `Arry 10` creation/removal across Undo.
- **Swimming Pool (`plan_pool_edit`, `build_commit_pool`)**:
  - Validates flat ground, uniform pool elevation across drag and adjacent
    existing pool tiles, off-road, unlocked, dry, wall-free, floor-free, and
    object-free tiles on Dig; refuses demolishing tiles occupied by a Sim or
    object on Demolish; charges `$75` / refunds `$60` (80%) to Architecture
    category `7`; and runs `rebuild_architecture_rooms` so `World::rooms`
    tracks `room.pool` and tile flags immediately.
- **Roof (`plan_roof_edit`, `plan_roof_pattern`, `plan_roof_pitch`, `build_commit_roof`)**:
  - Stores edited roof state inside `World::arrays[ROOF_META_ARRAY]` (`0x7266`,
    `[pitch_f32_le][utf8_stem]`) only upon explicit roof edits so load hashes
    and existing replay/ratchet outputs remain bit-identical, while `World::clone`,
    `Lot::save()`/`Lot::restore()`, and `BuildHistory::Roof` Undo/Redo
    (removing `ROOF_META_ARRAY` when undoing back to initial state) work
    seamlessly without altering `World`'s snapshot schema.
- **Unified Undo/Redo (`BuildHistory::{Terrain, Water, Pool, Roof}` & `Lot::build_commit_*`)**:
  - Implemented `Lot::build_commit_{terrain, water, pool, roof}` and
    `Lot::roof_state` directly in `crates/sims-vm/src/build.rs`, integrating
    chronologically with `Lot::undo()` / `Lot::redo()` and re-validating live
    guards, funds, and cell pre-states before every commit/undo/redo.

### 3. UI Subtools, Roof Catalog & Narrow-Viewport Overlap Fix (`sims-ui::build`)
- Added native `Res_CPanel.h` subtool and roof resource IDs (`3516..=3520`,
  `3800`, `3900..=3904`) to `build::id` and `build::resource_ids()`.
- Extended `BuildCatalog` to load `STR# 128` (Terrain subtools), `STR# 133`
  (Pool & Water subtools), and all `GameData/Roofs/*.bmp` textures (ordered
  `r1_..r4_` first, then case-insensitively by stem, with decoded `32×32`
  swatches).
- Exposed non-breaking `BuildSubtoolAction`, `BuildSubtoolWidget`,
  `BuildSubtoolPopup`, `BuildLayout::press_subtool`, `BuildState::apply_subtool`,
  and `build::draw_with_catalog` so `sims-game` compiles untouched until the
  follow-up wiring pass lands.
- Fixed the sub-620 px narrow-viewport Build tool-strip overlap nit by
  computing right-hand grid geometry first and paginating the 6×2 Build tool
  columns strictly left of `prev_arrow_x` with a `ToolPage(1)` cycle control
  when `width < 620`, while preserving exact native `cWinArch` coordinates for
  all `width >= 620` viewports.

## What Would Reopen This
- Wiring the thin `sims-game` pointer/drag dispatch and `sims-render` roof/pool
  overlay previews in the follow-up integration lane once PR #354, PR #349, and
  `build_preview.rs` merge.
- Recording native Complete Collection 1.91 traces for Terrain, Pond, Pool, and
  Roof interactions to verify any finer edge-case differences (such as
  drag-brush falloff radii or pool rim object auto-detachment).

# Build Mode Part 2 Previews (Terrain, Water/Pool, Roof) & Pie-Menu 400 ms Transitions

*Decided 2026-11-21.*

## Context

Track 1's Build Mode core (`#342`, `docs/decisions/20261121-build-mode-core.md`)
implemented Walls/Fences, Floors, Wallpapers, Doors/Windows, and unified
Undo/Redo. The next Build Mode milestone (Terrain sculpting/grass, Water/Pool
excavation, and Roof style/pitch customization) needs dedicated CPU rendering
primitives in `sims-render` that project onto sloped lot terrain, respect the
house depth buffer, and load Complete Collection's `GameData/Roofs/*.bmp`
catalog.

Separately, `docs/decisions/20261121-pie-menu-3d-head-parity.md` deferred
`Sims.exe`'s three 400 ms pie-menu transition animations (backdrop pop-in /
submenu glide, head pop-in scale & `Y_OFFSET` ramp, and direction-change
ease-out).

## Verified native behavior (`Sims.exe` Complete Collection 1.91)

### 1. Terrain, Pool & Water tools (`Build.iff`, `cDirtTool`, `cPoolTool`, `cWaterTool`)
- **`Build.iff STR# 128` (`Terrain`):** defines the four tools at `$1` each:
  `Raise Terrain`, `Lower Terrain`, `Level Terrain` (rectangular drag flatten
  to anchor elevation), and `Grass` (click/drag to restore grass, Ctrl-click to
  remove grass to soil). `cSimpleAltTool` caps candidate terrain elevation at
  `64` (`0x40`, `0x490f54`). `cSimpleAltTool`, `cLevelDirtTool`, and
  `cGrassTool` operate on interior lot tiles (`1..size-1`) away from roads
  (`floors 9..=13`), walls, floors, objects, and water/pool arrays (`9`/`10`).
- **`Build.iff STR# 133` (`Pool`):** defines `Pool Tool` (`$75`/tile,
  `cPoolTool` vtable `0x667fa4`) and `Water Tool` (`$15`/tile, `cWaterTool`
  vtable `0x66822c`).
- **`cPoolTool::Validate` (`0x48fa7b`–`0x48fb90`) & `cWaterTool` (`0x49838e`–`0x4984cf`):**
  both enforce interior bounds `1..size-1` (`0x48fa95`–`0x48faee`) and require
  all four corners of every excavated tile to match the anchor's flat elevation
  (`0x48faf9`–`0x48fb11`, `0x49838e`–`0x4984cf`). Pool billing counts only
  newly excavated tiles (`0x48fa06`–`0x48fa0c`).

### 2. Roof catalog, pitch presets & projection (`0x4153c4`, `cWinRoofPanel`, `cRoofMeshFactory`)
- **Pattern catalog (`0x4153c4`):** `GameData/Roofs/` holds 18 `32×32` 8-bit
  paletted BMPs (uncompressed and RLE8) in Complete Collection (`r1_.bmp`
  through `r4_.bmp`, `gravel1.bmp`, `tar1.bmp`, plus Superstar and Makin' Magic
  roofs). Missing or unspecified house roofs fall back to `"r1_"` (`0x6ceabc`,
  `0x56dfc1`, `0x56f657`).
- **Pitch presets (`cWinRoofPanel` `0x529d31`–`0x529d9d`, `0x52a11b`–`0x52a1f2`):**
  - `Flat`: `0.0` (`0x529d4b` `fldz`, matched by `< 0.1` at `0x52a18a`)
  - `Shallow`: `0.4` (`0x529d93` `[0x666a44]`)
  - `Medium`: `0.6` (`0x529d8b` `[0x666a40]`, default in `0x56dfc6`)
  - `Steep`: `1.0` (`0x529d87` `fld1`)
  - `cRoofLayer::SetPitch` (`0x56f0e9`–`0x56f120`) clamps custom slopes to
    `[0.0, 2.0]` (`[0x66657c] = 0.0`, `[0x66740c] = 2.0`).
- **World geometry scale (`cRoofMeshFactory` `0x56f7fa`–`0x56f81a`):** uses
  `TILE_WORLD_UNITS = 3.0` (`[0x6714e8]`) and half-tile sub-cells (`0.5`,
  `[0x666578]`). Each roof piece anchors to its supporting terrain height
  (`0x56ed12`–`0x56ed41`) before adding the pitch increment
  (`0x56ee4b`–`0x56ee73`), and automatic footprints include indoor halves of
  diagonally split tiles (`Lot::halves`). Combined with `Sims.exe`'s house
  camera terrain-step ratio (`16` near-zoom pixels per `3 / (2√6)` world
  units), vertical roof elevation projects at `32√6 / 3 ≈ 26.12789` near-zoom
  pixels per world unit through `Camera::project_fine_f32` and `Depth::point`.

### 3. Pie-menu 400 ms transitions (`0x462078`–`0x462822`)
- **Constants in unpacked `Sims.exe` image:**
  - `[0x6c4700] = 400` ms (`POP_IN_MS`, backdrop and head pop-in duration)
  - `[0x6c4704] = 400` ms (`DIRECTION_EASE_MS`, head turn ease-out duration)
  - `[0x6c4708] = 100` px (`BACKDROP_RADIUS`)
  - `[0x66744c] = 14.0` px (`Y_OFFSET`)
  - `[0x666a38] = 0.1`, `[0x667444] = 0.9` (`sqrt(0.1 + 0.9 * t)` ease-out)
- **Backdrop pop-in & submenu glide (`0x462106`, `0x46213e`, `0x46216e`–`0x46228d`):**
  on open, linearly interpolates `(x, y, radius)` from `(main, 0)` to
  `(main, 100)` over 400 ms using `_ftol` truncation (`(1 - t) * from + t * to`).
  Opening or closing a submenu snapshots the current interpolated state, clears
  `is_pop_in`, and glides over 400 ms to the target menu center.
- **Head pop-in ramp (`0x4625c1`–`0x46260d`):** while `is_pop_in` is true and
  `elapsed < 400 ms`, scales the head by `t = elapsed / 400.0` (both screen
  scale and depth scale) and ramps `y_offset` via `_ftol(t * 14.0)`.
- **Direction ease-out (`0x46269b`–`0x462822`):** on highlight direction change,
  snapshots the raw pointer-tracked `[pitch, yaw]` angles (`0x4626cf`–`0x4626d8`)
  and blends toward the compass-offset target via `u = sqrt(0.1 + 0.9 * t)` over
  400 ms (`0x462799`–`0x4627d2`).

## Decisions

1. **`crates/sims-render/src/build_preview.rs` (exported via `sims_render::build_preview`):**
   - Added pure validation + CPU overlay rasterization for Terrain
     (`TerrainTool`, `TerrainTarget`, `TerrainOverlay`, `TerrainPreviewPlan`,
     `validate_terrain_overlay`, `draw_terrain_overlay`, `draw_terrain_preview`),
     Water/Pool (`WaterTool`, `WaterAction`, `WaterOverlay`, `WaterPreviewPlan`,
     `CopingTile`, `validate_water_overlay`, `draw_water_overlay`,
     `draw_water_preview`), and Roof catalog + isometric preview (`RoofPitch`,
     `RoofStyle`, `RoofPattern`, `RoofCatalog`, `RoofPreviewConfig`,
     `detect_roof_footprints`, `draw_roof_preview`,
     `draw_roof_preview_from_catalog`).
   - Ground overlays test against `Canvas::depth` with `+32` bias so they
     appear cleanly above grass (`+80`) and floors (`+65`) while standing walls
     and foreground objects (`<= 0`) naturally occlude them, without modifying
     `Canvas::depth` or static lot pixels.
   - Roof previews rasterize affine-textured, sun-shaded triangles writing both
     5-6-5 color and 16-bit depth (`Depth::point`) across all 4 styles (`Hip`,
     `Gable`, `Mansard`, `Flat`), 4 pitches, 4 camera rotations, and 3 zooms.
2. **`crates/sims-people/src/pie_head.rs`:**
   - Added `POP_IN_MS`, `DIRECTION_EASE_MS`, `BACKDROP_RADIUS`,
     `DIRECTION_EASE_BASE`, `DIRECTION_EASE_SPAN`, `BackdropState`, `HeadPose`,
     `PieTransition`, `PieAnimState`, `project_scaled`, `draw_body_pose`, and
     `draw_body_in_viewport_pose`.
   - Preserved `draw_body`, `draw_body_in_viewport`, and `project` signatures
     and steady-state pixels bit-identically.
3. **Disjointness & static parity:**
   - Left `crates/sims-render/src/assets.rs`, `crates/sims-game/tests/ratchet.txt`,
     and `crates/sims-people/src/{lot,raster,animation,house}.rs` untouched.

## What Would Reopen This
- Wiring Build Mode Part 2's UI tool handlers and undoable simulation commands
  in `sims-ui` / `sims-vm` / `sims-game` to drive `build_preview` overlays and
  `PieAnimState` redraw ticks.
- Capturing native `Sims.exe` Build Mode roof/pool/terrain screenshots to tune
  sub-pixel roof eave overhang or pond shoreline dithering against pixel oracles.

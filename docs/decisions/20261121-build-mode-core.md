# Build Mode Core Tools (Walls, Floors, Wallpaper, Doors/Windows, Undo/Redo)

*Decided 2026-11-21.*

## Context

`sims-rs` already had Buy Mode placement, pickup, move, rotation, sale, and
Undo/Redo wired through `sims-vm`, `sims-ui`, and `sims-game`, along with full
house wall/floor/room rendering in `sims-render`. To make Build Mode (`F3`,
`PanelMode::Build`, `globals[19] == 1`) playable end-to-end with native-exact
simulation behavior, we implemented the core Build Mode tools—Wall & Fence,
Floor, Wallpaper, Doors, Windows, Columns, Stairs, Fireplaces, Plants, Hand
Tool, and unified Build/Buy Undo & Redo—and verified them against Track 2's
1,200-update Complete Collection 1.91 recording `goth-build-wall-floor-20261121`.

## Decisions

### 1. Build Mode UI (`cWinArch`, `sims-ui::build`)
- Modeled the 12-button tool grid using the native `cWinArch` coordinates from
  `0x6f2bf0`..`0x6f2c58` (Terrain, Water, Wall/Fence, Wallpaper, Stairs,
  Fireplace, Plants, Floor, Doors, Windows, Roof, Hand Tool), Undo/Redo at
  `(231, 21)` / `(231, 53)` relative to the bottom strip, and the 2×5 catalog
  grid at `x = 556, y = 505` with 45×45 swatch cells (`TEMPLATE = 4918`).
- Built `BuildCatalog` directly from `GameData/Build.iff` `STR# 129` (walls and
  fences), `STR# 130` (built-in floors), `STR# 131` (built-in wallpapers),
  `STR# 134`/`135` (plug-in fallback names/prices), `UIText.iff STR# 139`
  (tooltips), plug-in `.flr`/`.wll` files (`SurfaceCatalog::plugins`), and OBJD
  `build_mode_type` (`+0x8a`: 3 Doors, 4 Windows, 5 Stairs, 6 Plants,
  7 Fireplaces, 8 Columns, 9 Pool).
- Sorted floors, wallpapers, and objects by native `(price ASC, raw_name ASC)`
  (`0x4cd9a5`), keeping the five built-in Wall/Fence styles in native
  `cWinArch` order (`0x4cd464`..`0x4cd608`, `0x501c80`): Standard Wall (`1`,
  `$70`), White Picket (`2`, `$10`), Privacy Fence (`12`, `$35`), Wrought Iron
  Balustrade (`13`, `$45`), and Monticello Balustrade (`14`, `$45`). Road and
  sidewalk floor IDs `9..=13` and fence pattern IDs `248..=255` are excluded
  from the player catalog.

### 2. Plug-in Floor & Wallpaper Rendering (`sims-content`, `sims-render`)
- Extended `sims-content::surfaces` with `SurfaceCatalog::plugins` returning
  parsed `SurfacePlugin` metadata (`key`, `local`, `resource`, `name`,
  `raw_name`, `price`, `description`) alongside the existing `SurfaceFile` path.
- Extended `sims-render::Assets::floor_with_pools` and `Assets::pattern` to
  lazily decode plug-in `.flr` (`SPR2` at `local + [0, 256, 512][zoom]`) and
  `.wll` (`SPR#` at `local + [1536, 1792, 2048]` with fallback offsets) so
  expansion and custom surfaces render both on the lot and as catalog swatches.

### 3. `"Wall Cursor"` (`0x00000439`) Lifecycle & Architecture Simulation (`sims-vm::build`)
- Spawned `"Wall Cursor"` (`0x00000439`) off-lot (`[-16, -16, 1]`) when
  activating the Wall (`data[0] = 0/1/3`) or Wallpaper (`data[0] = 2`) tool,
  and deleted it when switching to non-wall tools (such as Floor) or leaving
  Build Mode (`0x48b68b`, `0x48a66a`, `0x48a6e5`).
- Positioned `"Wall Cursor"` at `(corner_x * 16 + 1, corner_y * 16 + 1, level)`
  for Wall drag (`0x48b7cd`, corner snapping via `0x48b717`) and at
  `(tile_x * 16 + 8, tile_y * 16 + 8, level)` (or `+15` for descending
  diagonal) with camera-rotated facing `data[1]` for Wallpaper (`0x48a80e`,
  `0x48a813`..`0x48a8a7`).
- Implemented pure planning + preview + atomic commit for:
  - **Walls & Fences (`0x48af6f`..`0x496931`):** 8-way straight/diagonal drag
    (`0x49474a`), Shift 4-sided room-box (`0x49642c`), Ctrl demolition with
    80% refund (`ftol(0.8 * price + 0.5)` at `0x4938c9`), wall-mounted object
    detachment/refund (restorable via Undo), split-diagonal floor preservation,
    and post-commit room/lighting rebuild (`World::rebuild_rooms`,
    `assign_rooms`, `update_rooms(0)`).
  - **Floors (`0x4881e6`..`0x4899d5`):** single tile, drag rectangle, Shift
    indoor room-fill (refusing outdoor room `0` per `0x4893d8`), Ctrl/Shift+Ctrl
    80% refund removal, and split-diagonal half-tile floors (`SPLIT_FLOOR_ID =
    255`).
  - **Wallpapers (`0x48a08d`..`0x4939c0`):** screen-space front-facing wall
    quad picking across all 3 zooms and 4 camera rotations, drag application,
    Shift room-fill, Ctrl/Shift+Ctrl 80% refund stripping, and fence/balustrade
    refusal (`styles 2, 12, 13, 14` per `0x4937da`).
- Charged/refunded all Build Mode mutations to Architecture budget category `7`
  (`0x46d91c` -> `world.budget.days[0][7]`, simulator `+0x1a0`).
- Validated every committed plan against live household funds and current cell
  pre-states (`cell_diffs_match`) so stale or unaffordable plans never mutate
  the lot.
- Unified Build and Buy Mode Undo/Redo chronologically inside `Lot` (`undo()`,
  `redo()`, `can_undo()`, `can_redo()`), keeping `Lot::buy_undo()`/`buy_redo()`
  synchronized with the unified stack, snapping back any held `BuyCursor`
  before Undo/Redo, and stripping `"Wall Cursor"` plus detached history objects
  on `Lot::save()` and mode switches (`0x4cea5d`, running `update_comeandsee`
  before teardown).

### 4. Verification
- Verified strict 1,200-update parity against `goth-build-wall-floor-20261121`
  on drawn-frame schedule `0,1,2,3+30`: exact `"Wall Cursor"` ID (`260`), fine
  positions (`[577, 465, 1]` -> `[577, 417, 1]`, `[584, 440, 1]`), `data[0..2]`
  (`[0, 0]`, `[2, 0]`, `[2, 6]`), funds (`2542 -> 2332 -> 2329 -> 2326`),
  Architecture spend (`33654 -> 33864 -> 33867 -> 33870`), object count
  (`345 -> 346 -> 345 -> 346 -> 345 -> 346`), and post-Build Live Mode RNG
  (`tick = 88894`, `lot_rng = 2_327_799_612`, `global_rng = 270_289_136`) at
  `u = 1200`.

## What Would Reopen This
- Implementing Terrain elevation/grass tools (`BuildTool::Terrain`), Water/Pool
  digging (`BuildTool::Water`), Roof style/pitch (`BuildTool::Roof`), or
  exterior connected-wall Shift wallpaper flood-fill beyond the clicked face.
- Recording additional native Build Mode oracles (e.g. diagonal walls, upper
  floor support edge cases, or door/window placement with wall cutouts) that
  reveal finer native differences.

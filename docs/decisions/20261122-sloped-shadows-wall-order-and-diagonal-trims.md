# Sloped floor shadows, back-to-front wall order, and mid-run diagonal trims

*Decided 2026-11-22.*

Three Complete Collection 1.91 (`Sims.exe`) rendering parity fixes close
remaining surface, shadow, and House02 diagonal-trim mismatches:

## What the original does

1. **Floor shadows project the tile centre at its rounded terrain altitude**
   (`0x4adac0` → `0x4b9613` → `0x4b97ea`, `0x4ae8cc`–`0x4ae9d3`, `0x4c9e38`).
   Before drawing a tile's shadow, `0x4adac0` projects tile centre
   `[x * 16 + 8, y * 16 + 8]` via `0x4b9613`, which reads `0x478dfe`
   (`altitude_at`), rounds to a whole terrain step `(altitude + 0.5).trunc()`,
   and projects onto that flat plane via `0x4b97ea`. `0x4ae8d4`–`0x4ae8e3`
   places the shadow mask at `screen_x - tile_half_width` and
   `screen_y - tile_half_height`, while `0x4ae93e`–`0x4ae9ab` and `0x4c9e38`
   anchor floor-shadow depth from the tile's top corner and step down from the
   shadow's top row. Previously `Painter::shadow` used the top corner's screen
   `y`, leaving ~60 px shadow ovals misaligned beneath shrubs on sloped tiles
   in `far-r0-cut-0816` and `far-r1-cut-0816`. (Note: the prior decision doc
   cited `0x4b00b6`–`0x4b010c` for sloped floor shadows, which is actually the
   swimming-pool water overlay `SPR2 0x800+val` in `0x4aee4c`; the floor shadow
   drawer is `0x4ae844`–`0x4ae9d3`.)

2. **Walls draw back-to-front in view order**
   (`0x54045b` → `0x540809` → `0x54088d` → `0x44054c`).
   `Sims.exe` sorts wall runs by `(vx + vy, vx - vy)` ascending so that on
   depth ties (`d <= dst + 1` in `0x4c49f2` / `Canvas::draw_wall_masked`),
   nearer walls overwrite farther walls at shared corner columns. Previously
   `Painter::walls_with_openings` iterated unrotated `(x, y)` order, which
   drew walls front-to-back at rotation 2 (and reversed along one axis at
   rotations 1 and 3), leaving 1-pixel vertical seam stripes at corners on
   `near-r2-0712-bench`, `near-r2-0805`, and `near-r3-down-0816`.

3. **Mid-run diagonal wall trims follow `0x53df39`–`0x53df90` and
   `0x53e184`–`0x53e1a8`.**
   At a non-record corner where two segments of a diagonal run meet, `0x53de05`
   takes `is_cut` from `curr` if solid (else `prev` if solid, else standing),
   and:
   - `Line::Rising` (`0x53df39`–`0x53df5c`): starts `mask = 0x50`, reduces to
     `0x10` only when `curr` is marked (`flags & 6 != 0`, ignoring `prev`'s
     mark), clears `0x40` / `0x10` for fence `curr` / `prev`, and leaves
     `trim_cut = is_cut` unchanged at `0x53e184`–`0x53e1a8` (`run_kind == 5`).
   - `Line::Falling` (`0x53df61`–`0x53df90`): starts `mask = 0xa0`, reduces to
     `0x20` when `curr` is marked, clears `0x20` when `prev` is marked, clears
     `0x80` / `0x20` for fence `curr` / `prev`, and clears `trim_cut` when
     `curr` has mark bit 2 (`run_kind == 6` at `0x53e19a`).
   This resolves the previously unexplained far rows 101–111 colour difference
   on House02's marked edge-on diagonal (`docs/decisions/20261120-edge-on-strip-ported.md`):
   at corner `(27, 24)` where cut marked `(26, 23)` meets standing `(27, 24)`,
   `Sims.exe` emits standing trim `0x50` (frame 29, rows `96..111`), which
   wins the depth test over the lifted edge-on strip starting at row 101
   (`44764 < 44774`, vs row 100 where `44807 > 44760`).

## Ported

- `crates/sims-render/src/scene.rs`: added `shadow_origin` for sloped floor
  shadow projection in `Painter::shadow`, and `wall_draw_key` to sort walls in
  `Painter::walls_with_openings` by `(vx + vy, vx - vy)` ascending, with unit
  tests covering all zooms and rotations.
- `crates/sims-render/src/caps.rs`: extended non-record mid-run trim handling
  in `Corners::new` to `Line::Rising` and `Line::Falling` (`0x53df39`–`0x53df90`,
  `0x53e184`–`0x53e1a8`), with unit tests for cut/standing joins and fences.
- `crates/sims-render/tests/edge_on_strip.rs`: expanded the pinned far-zoom
  House02 colour check from rows `80..=100` to all 32 rows `80..=111`.
- `crates/sims-render/tests/goth_house.rs`: updated pinned digests for rotated
  views affected by wall draw order and sloped floor shadows.
- `crates/sims-game/tests/ratchet.txt`: ratcheted floors strictly upward (0
  lowered floors).
- `docs/decisions/20261121-switch-order-replayed-and-centre-view-terrain.md`:
  fixed line 105 to reflect `0x4f4b43`'s shared 150-pixel container (`height - 100`).

## Audited and left unchanged (out of scope or non-renderer causes)

- **House02 flower beds vs swimming pool:** audited all 93 `FlowersOutdoor.iff`
  objects in `UserData/Houses/House02.iff` against `h02-g.png`. All 93 already
  render (`0` missing parts; front walkway flowers match `h02-g.png` on 99.6%
  of non-grid pixels, pool perimeter flowers on 98.6%). What currently draws as
  plain grass on `House02.iff` is the **swimming pool** (`Arry 9`,
  `0x4afa0d`–`0x4b010c`, `GameData/floors.iff` `SPR2 0x800..=0x80f` with frame
  `3 - zoom`), which needs a sprite loader in `crates/sims-render/src/assets.rs`
  (deferred to avoid conflicting with open PRs touching `assets.rs`).
- **Remaining grass bands (`far-r0-cut-0816` rows `320..326`,
  `near-r3-down-0816` rows `340..364` and `404..460`):** verified exact
  flipped dither parity (`(+1,+1,+1)` vs `(-1,-1,-1)`) ending cleanly on odd
  screen rows, caused by `Sims.exe`'s live partial-redraw dirty-rectangle
  clipping (`(y + height) & 1` in `0x49da59`) during capture rather than a
  full-frame grass bug.
- **Panel diffs (`49 px` in 08:16 scenes, `322 px` in `night-far-r0-cut-2153`):**
  verified `sims-ui`'s clock (`0x5fe51e`) renders the exact `World` values; the
  differences are 1-minute clock (`AM 8:17` vs `8:16`, `PM 9:54` vs `9:53`) and
  `$30` funds (`$3,192` vs `$3,222` from the kitchen fire in the simulation
  replay), not `sims-ui` rendering defects.

## Verified

Screenshot ratchet across all 8 frozen Complete Collection 1.91 scenes (0
lowered floors):

| Scene | `frame` | `surface` | `objects` | `people` | `panel` |
|---|---|---|---|---|---|
| `near-r2-0712-bench` | 99.97% → **99.99%** | 99.98% → **100.00%** | 100.00% | 99.67% | 99.93% → **99.99%** |
| `near-r2-0805` | 99.48% → **99.49%** | 99.81% → **99.84%** | 99.94% | 73.70% | 99.99% |
| `far-r0-cut-0816` | 99.79% → **99.81%** | 99.84% → **99.85%** | 99.89% → **99.91%** | 0.77% | 99.88% |
| `far-r1-cut-0816` | 99.86% → **99.87%** | 99.91% → **99.93%** | 99.88% → **99.91%** | 9.36% | 99.88% |
| `medium-r1-cut-0816` | 99.55% | 99.72% | 99.99% | 9.28% | 99.88% |
| `near-r3-down-0816` | 99.77% → **99.78%** | 99.69% → **99.70%** | 99.93% | - | 99.88% |
| `near-r0-up-0816` | 99.98% | 99.99% | 100.00% | - | 99.88% |
| `night-far-r0-cut-2153` | 99.90% | 99.98% | 100.00% | - | 99.19% |

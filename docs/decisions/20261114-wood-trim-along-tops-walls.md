# Wood trim along the tops of walls

*Decided 2026-11-14.*

The original lays a strip of wood trim along the top of every standing
wall and every cut-down wall. It is now drawn, read from its code
(0x53c227, the corner drawing) and the old prototype's corner pass
(`legacy/crates/sims-sim/src/lot_presenter/walls/construction.rs`).

- **One sprite per corner.** Each corner a wall touches gets a mask of
  the directions its walls leave it in: 1, 2, 4 and 8 for view -y, -x,
  +y and +x, 0x10 to 0x80 for the diagonals. A table at 0x6cd468 turns
  the mask into a frame; masks it has no frame for draw nothing.
  Standing walls' trim comes from SPR# 4096 to 4098 (by zoom) at the
  storey's height. Cut walls' trim comes from SPR# 5120 to 5122 at the cut
  height (6, 12 or 23 pixels), except along a diagonal through the
  corner (bits 0x10, 0x40), which takes the standing set.
- **Which walls.** Fences take none. A cut wall in a door-like style (3,
  5, 6, 15, 23) takes none. A cut wall meeting a standing wall gets a cap
  instead, which is not drawn yet.
- **Depth.** Each trim pixel takes the depth of the wall top under it.
  The original's own depth sprites for the trim (SPR# 6144 to 6146) are
  not read.
- **Light.** The trim is drawn at full light. The original's trim
  pixels are the sprite's own colours, even in the kitchen, which sits
  at half light.
- **Position: fitted.** The code puts the sprite half a tile left of
  the corner and one more pixel left. Here that extra pixel puts every
  trim one pixel left of the screenshot, so it is dropped; our corner
  point sits one pixel left of the original's.

**Result at the compared camera** (far zoom, rotation 0, floor 1, the
world after one tick of `sims boot`). Exact matches:

- Walls up, box around the house (x 120 to 500, y 100 to 320): 72.3%
  before, 75.7% now. Whole screen: 83.8% to 84.4%.
- Cutaway (the settled screenshot): box 67.1% to 69.0%; whole screen
  82.4% to 82.8%.

**What still differs:** the caps where a cut wall meets a standing one.
The original draws there a narrow upright slice of the wall's own
pattern with trim down its edge (0x53c6b7 onward, using the descriptors
at corner +0x28 and +0x3c). The extra cut-trim pieces the original adds
at a diagonal corner with bits 8 or 4 are also missing.

**Unverified:** other zooms and rotations, the cut trim on diagonal
walls, and the trim on terrain that is not flat.

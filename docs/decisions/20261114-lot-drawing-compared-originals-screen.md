# The lot drawing compared with the original's screen

*Decided 2026-11-14.*

One camera was compared: the Goth house (House05) as the game opens it,
far zoom, rotation 0, floor 1, walls cut away, with the window at 800 by
600. The original ran in the comparison harness (its own guest and
container); screenshots were taken with `xwd` and nothing was written to
the guest's memory. The screenshots stay outside the repository.

How: the render is drawn on a larger canvas, the screenshot's offset in
it is found by search, and both are reduced to the original's 16-bit
colour (5, 6 and 5 bits) before pixels are compared. Grass-green pixels
of the render are left out of the search, because the grass is not the
original's yet.

What matches:

- **The projection.** The original's camera is a pure translation of
  ours: the same offset aligns the yard, the house and the road.
- **Object sprites outdoors** (hedges, gravestones, trees, the fountain,
  the mailbox) match pixel for pixel at that offset.
- **The road and sidewalk**: 82% of pixels are exactly equal, most of
  the rest one step off.
- **Wall sprites**, once two bugs were fixed (below). With walls up,
  27% of pixels in the compared area match exactly, up from 19%. Most
  of the remaining difference is grass, lighting and the Sims.

Fixed by the comparison:

- Walls were drawn one pixel too low. The wall sprite's bottom row sits
  one pixel above the corner's projected point, not on it.
- A cut-down wall with a door or window in it showed the opening's cut
  sprite, a small outline shared by every door and window. The original
  draws the same short stub there as beside the opening, so it now takes
  the stub of plain wall style 1. Which style the original takes is
  **unverified**; the house only uses style 1 around its openings.

What does not match yet:

- **Lighting.** The original darkens floors, walls and objects indoors
  by the room's light. Everything here is drawn at full brightness, so
  most indoor pixels differ, though their shapes line up.
- **Grass.** The original's grass is a smooth, shaded texture. Ours is a
  made-up speckle, and lots whose grass value is low draw brown where
  the original is green.
- **The cap on a cut wall.** The original draws a three-row wood trim on
  top of a cut-down wall's stub. Its source is not found yet (it is not
  the wall pattern's top rows nor the 4096, 5120 and 6144 sprites of
  `walls.iff`).
- **Which walls cut away.** The original cuts walls room by room; its
  view keeps a 64 by 64 tile mask per visible room (the old prototype's
  reading in `legacy/crates/sims-sim/src/lot_presenter/room_masks.rs`).
  Right after the house opened, every ground-floor wall was still up.
  About a minute later most rooms were cut away, but the bathroom and
  the front of the round living room were not. Our rule (cut a wall
  unless it is a room's back wall) cuts them all.
- **Sims** are not drawn here; the people track draws them.

What would reopen it: a second camera (another zoom or rotation) that
disagrees, or a reading of the original's lighting or cutaway room
choice.

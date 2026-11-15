# Drawing the lot starts in its own crate, on the CPU

*Decided 2026-11-14.*

The lot is drawn by a new crate, `sims-render`. It reads the world
(`sims_sim::World`) and never changes it, and nothing in the simulation
depends on it. It draws on the CPU, so tests need no window or GPU.

Terms used here. A *sprite* is a pre-drawn picture from the game's files.
An object's sprites (SPR2) carry a *depth byte* per pixel that says how
far that pixel is from the camera. A *draw group* (DGRP) lists the sprites
that make up one view of an object. A wall's *shape* is a mask that says
which pixels of a wall exist (full wall, cut-down wall, fence); its
*pattern* is the wallpaper. *Cutaway* is the original's middle wall mode,
in which walls that would hide a room's inside are cut down.

What was decided:

- **The original's projection.** A tile's top corner is at
  `((x - y) * (8 << zoom), (x + y) * (4 << zoom))` plus the camera's
  origin, raised 58, 116 or 232 pixels per storey at zooms 1 to 3 and
  `2 << zoom` pixels per terrain step. The four rotations use the
  original's 64-tile rotation table. This is the old prototype's reading
  of the original's code (`legacy/crates/sims-sim/src/lot_presenter/`).
- **Depth is a number per pixel, smaller nearer, and a tie goes to the
  later draw.** Its scale was fitted from the game's sprites, not read
  from the original's code. For the lowest pixel of each sprite column,
  which mostly touches the floor, the depth byte falls 1.5 per screen
  pixel below the object's anchor at the near zoom, 3 at medium and 6
  at far, and is about 200 level with the anchor. So one byte is a fixed
  distance in the world. The renderer gives floors, walls and the
  anchors of objects a depth on the same scale. Whether this matches the
  original's depth tables is **unverified**.
- **Where a sprite goes.** An object's anchor is the screen point of its
  position on the floor. Each sprite of its draw group is drawn with its
  left edge and bottom at the offsets the draw group gives (checked on
  every near-zoom draw group of the Goth house's objects: the anchor sits
  at the same point of each sprite's frame). A draw group sprite with
  flag 1 is mirrored. Dynamic sprites hide when their word is zero
  (0x437f99, from the old prototype).
- **Floors** are `GameData/floors.iff` sprites `id`, `id + 256` and
  `id + 512` for the three zooms, drawn with their middle column on the
  tile's top corner. A tile a diagonal wall splits shows two floors from
  its wall cell; which half takes which is **unverified**. Grass is a
  brown-to-green mix by each tile's grass value, scattered in two
  shades. The original draws grass blades from a seeded pattern, so
  grass will not match pixel for pixel.
- **Walls** are a pattern from `GameData/walls.iff` (`id` plus 1536,
  1792 or 2048) masked by a shape (`style * 2 + cut` plus 0, 512 or
  1024), one frame per wall direction. Each wall cell owns its low-x and
  low-y walls and its diagonals; which side's wallpaper shows follows
  the old prototype's `faces`, which was compared with the original's
  wall records. A wall of style 22 or 23 takes its shape from the door
  or window standing in it (the object's OBJD byte 0xa2), as the old
  prototype did. Where a wall sprite sits on its edge was worked out
  from the shapes' outlines and is **unverified** until compared with a
  screenshot.
- **Cutaway is simplified.** A wall stays up only if the room on the
  camera's side is indoors and the other side is outdoors: the back
  walls of the house. Doors and windows in a cut wall are hidden. The
  original's cutaway (0x54059a and 0x4aa446) also depends on a mask it
  builds per view; that is not ported, so this is **unverified**.
- **Not drawn yet:** the Sims (the people track draws them), roofs,
  shadows, lighting by time of day and room, objects in other objects'
  slots raised to the slot's height, and terrain shading.
- **A small loader** (`open_house`) builds a world from a house file,
  widens its floor and wall arrays as the load does and works out the
  rooms, so a house can be drawn without a full `sims boot`.

How it was checked:

- The Goth house renders at all three zooms and four rotations, with
  every wall shape found. The renders were looked at: floors, walls,
  fences, doors, windows and furniture sit where they should, and the
  back walls stay up in cutaway. They are pinned by digest of the
  16-bit-color image (`tests/goth_house.rs`, gated on `SIMS_INSTALL`).
- Nothing has been compared with a screenshot of the original yet.

What would reopen it: a screenshot comparison that disagrees, or a
reading of the original's depth tables or cutaway mask.

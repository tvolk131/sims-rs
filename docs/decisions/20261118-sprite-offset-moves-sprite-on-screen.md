# A draw group sprite's x and y offset move it on the screen

*Decided 2026-11-18.*

A dining chair scooted in to the table stood a quarter tile out from
the person eating on it. The original draws it right behind them. The
chair (guid 0x25b82d98) takes its graphic from data word 0. Graphics 0
to 6 share one sprite (4128) and one pixel offset, and differ only in
the sprite's world offset y: 0, −0.8, −1.6, −2.4, −3.2, −4.0 and −8.0.
Eating scoots it to graphic 5. So the y offset is the whole scoot, and
we used it only for depth.

The entry "Hidden objects, sprite world offsets and soft edges"
(2026-11-14) left x and y out of the screen position, as the catalog
preview does, and marked that **unverified** for the lot view. This
entry settles it: the lot view moves the sprite.

**The original's code.** The lot view draws a draw group sprite at
0x437b7e. The entry holds z at +0x10, x at +0x14 and y at +0x18
(floats), the pixel offset at +8 and +0xc, and flags at +0x1c.

- x and y turn with the object's facing (0x439109): (x, y) becomes
  (−y, x) for each two steps. An odd facing (a diagonal) sets both to 0.
- They are added to the object's fine position, a float.
- 0x4b9991 projects that point. It adds a half and truncates to a fine
  position, turns it with the camera (0x47bc1f), takes the ground
  there (0x478dfe), and writes the screen position.

Ours does the same (`part_fine`), for the screen position and the
depth. Depth used to round x and y to the nearest, and turned them for
an odd facing too. Both now follow the code above.

**What the screenshot shows.** The original at 7:47 (medium zoom,
rotation 0, cutaway) has Mortimer eating at the patio table's west end.
The chair's back is right behind him and its seat is under him. Ours,
before, drew the same chair a quarter tile out, with his legs in the
gap. With the change, a scooted chair at the table on the current main
(tick 15420, chair 223) sits under its Sim the same way. The pinned
renders did not change: nothing in them has a nonzero x or y offset.

**Unverified.**
- The ground. The original takes it at the moved fine position. Ours
  keeps the mean of the object's own tile's corners. They differ only
  where an offset crosses into a tile on a slope.
- z. The 2026-11-14 entry's rise of `z * (2 << zoom)` pixels is
  unchanged and still read from the preview only.
- The pixel-exact position. The original's frame and ours show
  different meals (the simulation runs a different day), so only the
  chair's place against its Sim is compared, by eye.

What would reopen it: an object with a nonzero x or y offset that the
original draws where ours drew it before.

# A split floor tile's grass keeps the blades of the tile drawn before it

*Decided 2026-11-15.*

The grass's blade count is now worked out tile by tile in the order the
original draws the tiles, and carried from one to the next, as the
original keeps it in the grass object between tiles (0x49d0da to
0x49d215). The tiles are drawn in runs along the view's diagonals, x + y
rising, and x rising along each run (0x4ad4bd calling 0x4ad8ac). A tile:

- with no floor counts its blades by its grass value, as before;
- with a floor and no diagonal wall counts none (0x49d155) and keeps the
  previous tile's short-blade flag;
- with a floor split by a diagonal wall (wall byte 0 & 0x30) keeps the
  count and the flag of the tile drawn before it, whatever its own grass
  (0x49d14b);
- off the lot's inner tiles (1 to size − 2 in x and y) returns before
  any of this and leaves the count as it was (0x49cecc); it draws no
  blades here, as before.

So the floorless half of a split tile beside a porch shows flat grass,
and beside a lawn a full lawn. Before, it counted by its own grass.

**Checked.** Against the original at the tick-570 world: near zoom (the
porch view, origin 619, −1189) rose from 86.5% to 87.0% exact; of the
2,402 pixels the change touches, 2,115 now match exactly (3 did before),
and the mean channel error there fell from 21.1 to 4.6. Medium zoom
(cutaway): 82.7% to 82.8%; of 771 pixels changed, 715 match (none did),
error 16.6 to 0.3. At far zoom only a few pixels change: 8 at rotation
1, all now exact (none were), and 9 at rotation 2, which match neither
way. Tile (27, 28) on the Goth
porch is one: the tile drawn before it, (26, 29), has a floor, so its
empty half is flat in both.

**Not ported.** The original clips each run to the screen before drawing
it, so a tile off screen is not drawn and does not pass its count on;
here every tile is in the order. A split tile at the screen's edge could
take a count from a different tile. The pool test (0x47c16d), which
gives a tile in a pool none, is still not looked at.

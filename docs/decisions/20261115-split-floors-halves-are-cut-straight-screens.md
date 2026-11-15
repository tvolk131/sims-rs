# A split floor's halves are cut straight, by the screen's clip rectangle

*Decided 2026-11-15.*

A floor tile a diagonal wall splits now draws each half as the whole
floor sprite with everything past the cut left out, the cut a straight
row or column: the original draws each half's sprite with the screen's
clip rectangle (the words at 0x6f0c10, top, left, bottom, right)
narrowed to its side of the cut, and restores it after (0x4af380 on).
On screen a tile's diagonal runs level, through its left and right
corners, or upright, through its top and bottom ones. For a level cut
one half is drawn with the top raised to the corners' row (0x4af70a) and
the other with the bottom lowered to it (0x4af924); for an upright cut
the right edge is lowered to the tile's x (0x4afcf5) and the left raised
to it (0x4afef6). The corners' row goes to the lower half and the cut's
column to the right half. The sprite is drawn at that x less half a tile
(the view's field +0x5c, `8 << zoom`, set at 0x4a6cfa), so the cut is
16, 32 or 64 pixels into the image. The sprites are a pixel narrower
than the tile (31, 63 and 127 wide) with their top and bottom corners on
the middle column, so the cut falls one column right of the corners,
not on them.

Before, each pixel was placed in the tile and tested against the
diagonal, with the position rounded toward zero. On the level cut's row
that put alternate 4-pixel runs at near zoom (2 at medium) in the upper
half, so a porch floor ended in a dashed row over the grass below it.

**Checked.** Against the original at the tick-570 world, the pixels the
change touches fall in mean channel error from 98.7 to 9.8 at near zoom
(192 pixels, the porch view), from 98.5 to 13.7 at medium (49), and
from 80 to 2 and 105 to 1.5 at far zoom, rotations 2 and 1 (48 and 10).
Few match exactly yet because the grass under them still differs (see
the split tile's blade count). Which half takes which floor is
unchanged.

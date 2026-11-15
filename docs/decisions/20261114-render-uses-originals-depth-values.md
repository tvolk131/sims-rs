# The render uses the original's depth values

*Decided 2026-11-14.*

The lot is now depth-tested in the original's own depth space: 16-bit,
cleared to 65535, smaller is nearer. Every surface computes the number
the original would store. Checked pixel by pixel against depth buffers
read from the running game at three cameras (memory dumps, never
written):

| Camera | Depth exact | Screen exact (before → after) |
|---|---|---|
| Medium zoom, rotation 0 | 99.1% | 88.0% → 88.9% |
| Far zoom, rotation 0 | 99.7% | 89.9% → 90.6% |
| Medium zoom, rotation 3 (night) | 97.2% | (lighting differs) |

House box at the screenshot cameras, exact in 565 space, before → after:
medium 90.4% → 91.9%, far 90.2% → 92.4%, far rotation 3 90.0% → 92.5%,
cutaway 87.1% → 89.0%, rotation 1 85.5% → 86.8%, rotation 2 86.1% →
87.6%, near zoom 90.4% → 93.8%, 80.5% → 81.8% and 56.2% → 56.9%.

How each surface gets its depth (all exact against the dumps unless
noted):

- **Tables.** Five per zoom, generated from formulas and checked entry
  by entry against the game's own (digests in `depth.rs` tests): a row
  table R (the ground's depth by screen row from the origin), an
  upright table L (height above an anchor), a column table C (across a
  wall), a terrain height curve H, and the 255 sprite depth bytes T.
- **Grass and floors**: R at the tile's row plus H for its height, plus
  80 for grass (the caller's own offset); floors follow the ground row
  by row, plus 65.
- **Walls**: an anchor at the wall's lower end on screen, then L by row
  and C by column (0x4c4907); a wall pixel wins at up to one more than
  what is there. Diagonal walls use no column table.
- **Objects**: the object's anchor (0x4c9ceb) plus the sprite's depth
  byte. The original reads L at the sprite context's own row, so a
  sprite's height above its anchor adds nothing (0x49a5d3).
- **Faint sprite pixels**: a sprite pixel with alpha under 8 of 31 is
  not drawn at all, colour or depth. This is fitted, not read: it is
  the threshold that matches best (alpha under 6 or 9 matches less).

The door's left jamb now matches: the column the wall covered was a
faint door pixel in front of the wall, which the original never draws.
At medium zoom the door boxes rose from 86.4% to 88.5% exact.

**Unverified:** upper-level floors and anchors (the dumps show level 1
only), and the wall depth the original takes off for its mask 32. What
is left at rotation 3 is mostly the fountain, a bench and stairs on the
upper level, and sprite edges.

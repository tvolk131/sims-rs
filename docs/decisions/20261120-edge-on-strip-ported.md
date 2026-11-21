# The strip on a marked edge-on diagonal is ported

*Decided 2026-11-20.*

This reopens "The original's strip on a marked edge-on diagonal is read,
not ported" (2026-11-14), which waited for a screenshot of the case.
House02's cutaway has one at every zoom: a cut diagonal by the front
door, tile (26,23), meeting a standing wall. The screenshots are the
far, medium and near shots of House02 in the render corpus.

What the original does, from its code and those screenshots:

- The strip is pattern 252 in shape 9 on an edge-on diagonal, whichever
  end is marked (0x53eb6d to 0x53ebf5). Other walls keep 252 in shape 9
  or 253 in shape 10, by the marked end.
- Shape 9's edge-on frame is taller than the pattern's: 4x69, 8x136 and
  16x273 against 4x58, 8x116 and 16x232. The blit takes the shape's
  rectangle as its base and moves the pattern's bounds onto it
  (0x4c76d1, 0x4c7fdb), and starts both images at their first row. The
  screenshots settle the alignment: the shape's transparent first rows
  (one at far and medium, two at near) hide the pattern's first rows, so
  the two line up at their top-left corners. Lined up at the bottom
  instead, the strip shows one or two rows too many at its top. Only the
  pattern's rectangle is drawn. `Image::masked_top_left` does this, and
  only the edge-on strip uses it; every other wall still needs a shape
  the size of its pattern (`Image::masked`), because other pairs are not
  read.
- The strip's rectangle is moved up by `(92 >> (3 - zoom)) + 1`, or `+ 2`
  at near zoom (24, 47 and 94 pixels), before the draw and restored after.
  Depth is still looked up at the row drawn, from the same anchor
  (0x49a5d3, read in review); on the original's tile, reading it at the
  unmoved row scores the same, so the screenshots do not decide this.
  Lighting is unchanged (room 0's light, capped by the time of day).

Measured against the three House02 shots (focus on room 2 at far zoom,
rooms 2, 1 and 3 at medium and near; doors and windows given their
hide-with-walls flag, as in the running game):

- On the original's tile, the strip's top row is the original's at every
  zoom (rows 80, 102 and 146), and every pixel of the rows just above
  and below its top matches (far 16 of 16, medium 40 of 40, near 96 of
  96), with the lift exactly as read.
- Its colours match the original's row by row to within two levels,
  except for a few rows just above the stub (far rows 101 to 111),
  which are not explained.
- Exact pixels (16-bit) in the compared box, before and after: far
  50815 to 50899 of 55998; medium 317445 to 317380 of 336000; near
  270345 to 270155 of 336000. Medium and near get worse because of the
  open question below: there the strip is drawn on another tile.

Open, recorded as evidence rather than ruled on: at medium and near our
cutaway mask holds 18 more tiles than at far, and with it the strip is
drawn on (27,24) and (24,31) instead of (26,23), where the original
draws it at every zoom; at (27,24) most of the strip is behind the floor
beyond it, and what shows is out of place. Building the medium and near
masks as at far zoom puts it on (26,23) and gives medium 317864 and near
271711, the strip then matching the original's from its top row.
Whether the original's mask is the same at every zoom, or what else
differs, is not read, so the mask is unchanged here.

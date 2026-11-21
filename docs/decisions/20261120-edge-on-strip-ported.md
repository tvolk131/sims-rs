# The strip on a marked edge-on diagonal is ported

*Decided 2026-11-20.*

This reopens "The original's strip on a marked edge-on diagonal is read,
not ported" (2026-11-14), which waited for a screenshot of the case.
House02's cutaway has one at every zoom: a cut diagonal by the front
door, tile (26,23), meeting a standing wall. The screenshots are the
far, medium and near shots of House02 in the render corpus.

What the original does, from its code:

- The strip is pattern 252 in shape 9 on an edge-on diagonal, whichever
  end is marked (0x53eb6d to 0x53ebf5). Other walls keep 252 in shape 9
  or 253 in shape 10, by the marked end.
- Shape 9's edge-on frame is taller than the pattern's: 4x69, 8x136 and
  16x273 against 4x58, 8x116 and 16x232. The blit lines the shape up
  with the pattern's bottom-left corner and draws only the pattern's
  rectangle (0x4c7fdb moves the shape's bounds by the difference of the
  two rectangles' bottoms and lefts; 0x4c76d1 keeps the pattern's
  rectangle as the one drawn). So the strip is the pattern masked by
  the shape's bottom rows. `Image::masked` now does this for any mask at
  least as large as the image; a smaller mask still gives nothing.
- The strip's rectangle is moved up by `(92 >> (3 - zoom)) + 1`, or `+ 2`
  at near zoom (24, 47 and 94 pixels), before the draw and restored after.
  The sprite carries its own depth, so its depth moves up with it; ours
  draws the strip that much higher and reads the wall's depth that much
  lower. Lighting is unchanged (room 0's light, capped by the time of
  day).

Measured against the three House02 shots (focus on room 2 at far zoom,
rooms 2, 1 and 3 at medium and near):

- With the lift as read, the strip's top row is the original's: row 80
  at far zoom, and 102 and 146 at medium and near once the strip is on
  the original's tile (see below). A lift one pixel smaller scores worse
  at every zoom on that tile; a larger one was not tried.
- The strip's colours match the original's row by row to within two
  levels, except for a few rows just above the stub (far rows 102 to
  110, medium rows 144 to 152), which are not explained. At near its
  mean is 156,120,80 against the original's 155,119,80. The old strip
  (253 in shape 10, not moved) was darker at near and brighter at
  medium.
- Exact pixels (16-bit) in the compared box: far 50815 to 50895 of
  55998; medium 317445 to 317452 of 336000; near 270345 to 270424.

Open, recorded as evidence rather than ruled on: at medium and near our
cutaway mask holds 18 more tiles than at far, and with it the strip is
drawn on (27,24) and (24,31) instead of (26,23), where the original
draws it at every zoom; at (27,24) most of the strip is behind the
floor beyond it. Building the medium and near masks as at far zoom puts
it on (26,23) and raises medium to 317856 and near to 271679, the strip
then matching the original's from its top row. Whether the original's
mask is the same at every zoom, or what else differs, is not read, so
the mask is unchanged here.

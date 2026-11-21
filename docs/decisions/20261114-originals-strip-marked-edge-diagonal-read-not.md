# The original's strip on a marked edge-on diagonal is read, not ported

*Decided 2026-11-14.*

The last unported piece of the wall caps is the strip the original lays
on a marked cut wall that runs edge-on (a "rising" diagonal, frame 3).
From its code (0x53eb6d to 0x53ebf5, 0x53e87b, 0x4ac14b):

- The strip is always pattern 252 in shape 9, whichever end is marked.
- Before drawing, the strip's rectangle is moved up by 24, 47 or 94
  pixels at the far, medium and near zoom (`(92 >> (3 - zoom)) + 1`,
  or `+ 2` at near). The call is to OffsetRect; other call sites pass
  (rect, -left, -top).
- The rectangle is the wall's: 4 pixels wide at far zoom, from 42 above
  its anchor to 16 below, and an edge-on wall is anchored 1 pixel left
  and 17 pixels lower (at far zoom) than the other kinds.

Put together, the strip would end 8 pixels above the stub it belongs
to (the marked wall's shape 7 or 8 is only the stub for this frame), so
either part of the placement is still unread or the case does not come
up. None of our screenshots has one, and the Goth house has five
diagonal walls, none marked in the compared cutaway at any rotation.
So the original's rule is not ported: if such a wall does come up, we
draw it like any marked cut wall (pattern 252 in shape 9 or 253 in
shape 10, by which end is marked, not moved). Reopen with a screenshot of a cut edge-on diagonal meeting a standing
wall.

Caps on diagonal corners (the 2-pixel slices) are drawn as the corner
records give them; they too are unverified for the same reason.

Reopened and ported on 2026-11-20 with House02 screenshots: see
`20261120-edge-on-strip-ported.md`.

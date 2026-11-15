# People are drawn over the finished lot, against its depth

*Decided 2026-11-15.*

The original keeps its people out of the lot's depth buffer. Two dumps
of the running game (far zoom with a Sim standing on the grass, medium
zoom with two Sims on the porch bench) show the Sims in the colour
buffer and only the lot in the depth buffer: plain grass under the
standing Sim, the wall and the porch floor where the seated ones are.
The bench they sit on is missing from the depth buffer too. So the
original draws the lot first, then draws what moves, and what is in
use, over it, tested against the lot's depth.

`sims-render` takes the same order. After `draw`, a caller draws people
a pixel at a time with `Canvas::put` (the sprite rule: fainter than the
cutoff or farther than what is there is left out, a tie wins, the depth
is written so a nearer part of the same mesh covers a farther one) at
the depth `Depth::point` gives a point: the ground rows at the point's
row, the level's offset, and the height curve for its elevation. It is
an object anchor's depth without the rounding, in the same units as
everything else. `Camera::project_fine_f32` is the projection without
rounding, so the people's code does not keep its own copy.

The interleaved draw planned with the people track (people sorted in
with the lot's objects) is left out: the original does not interleave,
and the depth test against the finished lot already hides a person
behind a wall, a counter or a cut wall's stub. The original draws a bench
in use with the people rather than with the lot; here it stays with the
lot and people test against it, which matters only where a person is
drawn in front of part of the bench and behind another part.

How the original gives a person's mesh its depth is **unverified**:
the dumps show where people are not, not their own depth. Reopen it
with a frame in which a person is partly hidden, comparing the hidden
pixels with `Depth::point`.

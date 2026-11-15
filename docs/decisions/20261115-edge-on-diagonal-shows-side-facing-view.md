# An edge-on diagonal shows the side facing view (−1, 1), a little higher

*Decided 2026-11-15.*

A diagonal wall seen edge-on (frame 3: its normal turned into the view
is (1, −1) or (−1, 1)) now shows the side facing view direction
(−1, 1), and its sprite stands 1, 2 or 4 pixels higher at far, medium
and near zoom than before (`sims_render::scene`, `shown_side` and
`Scene::place_wall`). Before, it showed the side its normal points to,
so at rotation 0 the Goths' bay ends showed the inside wallpaper where
the original shows the shingles.

**The side** is the rule read in
`20261115-edge-diagonal-wall-stands-300-nearer-anchored.md`: the
original's runs store each entry's two patterns in a fixed order
(0x53f6c1) and turn them with the view (0x53d93a), so the side that
shows faces view direction (−1, 1) whatever the lot side. That entry
could not check it because the texel offset was unread.

**The offset** is fitted, not read. The column the original draws is the
pattern's first texel column, and the sprite's left edge is where ours
was (half the sprite's width left of the column); only its rows were
off. Fitting each column the original shows against every texel column
of the wall's two patterns, at every row shift, gives a perfect match
(correlation 1.000 of brightness down the column) in three places, all
with the side above and the pattern's column 0:

| Camera | Wall | Column | Sprite top, original vs ours |
|---|---|---|---|
| Medium zoom, rotation 0, [335, −560] | (23, 31) ascending, normal (1, −1) | x = 75 | 2 pixels higher |
| Near zoom, rotation 0, [619, −1189] | the same wall | x = 99 | 4 pixels higher |
| Far zoom, rotation 1, [283, −203] | (26, 31) descending, normal (−1, 1) | x = 377 | 1 pixel higher |

So the two sides are told apart at two rotations, once with the side the
normal points to and once with the other.

**Checked** against the original at the tick-570 world. On the pixels the
change touches, the mean channel error falls from 59.6 to 0.9 at medium
zoom (120 pixels), from 40.6 to 1.0 at far zoom rotation 1 (58), from
16.4 to 0 at far zoom rotation 2 (15, all now exact) and, at near zoom,
from about 75 to under 2 on the rows the game's control panel leaves
uncovered. The rest are a lighting step off, as on the walls beside them.
Whole-screen exact rates move by at most 0.1 points.

**Still open.**

- The strip on a marked edge-on diagonal
  (`20261114-originals-strip-marked-edge-diagonal-read-not.md`) is
  unchanged: no screenshot has one.
- At near zoom, below the bay's left end (x 90 to 98, rows 434 to 480 at
  origin [619, −1189]), the original shows shingles where we draw the
  flower bed in front of the bay. That is not this column, which matches
  from x = 99. The rest of that area is under the control panel.
- Where the lift comes from is not read: it is 1 << (zoom − 1) pixels in
  all three fits.

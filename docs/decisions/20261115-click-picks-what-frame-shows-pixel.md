# A click picks what the frame shows at the pixel

*Decided 2026-11-15.*

The game window picks from the per-pixel IDs the lot and its people
were drawn with (`Canvas::id_at`, [20261115-canvas-notes-which-object-or-person-drew-each](20261115-canvas-notes-which-object-or-person-drew-each.md)), replacing its guess
from each object's tile and its own copy of the people's drawing loop
(`sims-game` now draws people with `draw_on_lot`, pixel for pixel as
before; the Goth test checks it against `sims-people`'s own call).
`sims_game::pick::at` takes, in the frame last drawn:

1. the person shown at the pixel, or the one shown nearest within two
   pixels (`REACH`, as before: people are thin);
2. else the object shown at the pixel itself, with no search around it.

So a person or object behind a wall or a bookcase takes no click meant
for what hides it, and a short object in front of a tall one no longer
takes clicks on the tall one's upper part. A click near a person still
picks the person over the object under the pointer, as before.

**A tile of a multi-tile object is picked as itself, not mapped to its
group's lead.** Each tile is its own object in the world, and the pie
menu is listed for the tile (`Lot::pie_menu`); the simulation then
sends a tile's interactions on to the tile that takes them, as the
original's `interaction_target` (0x435286) does (`menu::target`): the
first tile of the group whose definition word 8 is the clicked tile's
negated, when that is positive. Mapping every tile to the lead here
would skip that rule, which picks a tile other than the lead for some
groups. This is what the tile guess did too. **Unverified:** how the
original turns a click into an object (an ID buffer as here, or
otherwise), and whether it passes the tile or its lead to the pie
menu, are not read; compare the object the original lists at the same
pixel once the harness can click. Reopen if it maps to the lead.

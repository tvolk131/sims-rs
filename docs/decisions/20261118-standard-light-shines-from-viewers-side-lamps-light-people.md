# The standard light shines from the viewer's side, and lamps light people

*Decided 2026-11-18.*

A person on a lot was lit by the standard light from the wrong side, and
the room's lamps did not light them. Both are fixed (`View::light`,
`lot::lamps`, `Lighting::shade_with`). The entry "A person is lit by the
original's standard light, at their room's level" (2026-11-15) left the
light's direction **unverified**. This entry settles it and ports the
lamps that entry listed as not ported.

**Our drawing frame is the original's turned half a turn.** The person's
placement (`crate::placement`) puts fine position (x, y) at (−3y/16,
height, −3x/16). The original puts it at (3y/16, height, 3x/16). Its
running memory shows this. The room table (the map at the house's +0x24,
read by 0x466fcc, keyed by room word at node +0x10) holds one light for
room 4 at (88.5, 8.674, 79.5). That is the lamp at fine (424, 472): 3 ×
472 / 16 = 88.5 and 3 × 424 / 16 = 79.5. The person draw object at the
same moment has its root at (77.06, 3.674, 63.56), the same signs.

`stdlight.lgt`'s direction is in the original's frame. We used it
unchanged, so our light pointed towards smaller fine positions. The lot
renderer draws those higher on the screen, farther from the viewer, so
the light came from behind the scene. It now turns half a turn into our
frame before it turns with the view. It comes from above and behind the
viewer, as the file's own position (6.79, 9.55, 6.79) and the house
camera's eye (0.61, 0.5, 0.61, 0x49b0de) both say.

**What the screenshot shows.** The original at 8:00 at medium zoom
(rotation 0, cutaway) has Mortimer at the bookcase in room 4, his back
to the viewer. Room 4's light byte is 162, so his level is 0.635 and the
light's side shows. The floor around him matches ours within 1% in four
boxes. Over his 1011 pixels (ours at tick 2675; the original's pose is a
few dozen ticks behind ours, so pixels are compared as distributions):

| | mean | quartiles | legs mean |
|---|---|---|---|
| original | 32.8 | 10.3 / 28.7 / 44.6 | 47.6 |
| light from behind (before) | 25.9 | 8.6 / 24.5 / 30.8 | 35.6 |
| light turned | 34.3 | 12.5 / 31.3 / 45.1 | 48.9 |
| light turned, with lamps | 36.5 | 13.1 / 34.4 / 47.5 | 52.2 |

With the light behind him, his back was at the ambient grey only. With
the light turned, it is lit as the original's is. Forcing his level,
with the old light, matched only at about 0.85, which nothing in the
original's code gives.

**The lamps** (0x4b4b04 to 0x4b4c95, inside the light setup 0x4b49c7):

- **Which lamps.** The room count gives a room a light for each lamp it
  counts that is lit (`sims_render::lot::gives_light`, the test its lamp
  records already used, now shared). There are at most six (0x467eff).
- **Where the light is.** At the lamp's fine position, 5 units above
  the ground there (0x467f56; 5.0 at 0x667678). The running memory has
  the lamp above at 8.674 while the person stands at 3.674.
- **Which room.** The person's lighting record holds the root of their
  model (+0x48 of the transform at their model's +0xc, set at 0x590e18
  through 0x4b4987) and their level (+0x110). The room is the word the
  level's room map holds at that tile. A word of 0xfffb or more (a split
  tile) takes no lamps.
- **Strength and direction.** For each light, d² is the squared
  distance from the light to the root. Past 400 (0x6687b8) the light is
  off. Below 225 (0x6687b4) its strength is 1. Between the two it is
  `1 / ((d² − 225) × 0.5 + 1)` (0.5 at 0x666578). The direction is root
  minus light (0x49b6df), normalized: from the lamp to the person.
- **Colour.** (1, 251/255, 209/255) (0x4b4c49 to 0x4b4c5d). The strength
  is not scaled by the level.

Direct3D adds the lamps to the ambient and the standard light per
vertex, and holds each channel to 1, so a person lit by a lamp takes a
shade per channel. With no lamp the shade is the grey one, unchanged to
the bit.

The original's recording of the Goth day (track 2's, update 2675) has
both of room 4's lamps on: data word 16 is 1 on objects 71 and 194, and
word 51 is 25 and 50. So the lamps belong in the 8:00 picture. The
nearer lamp, four tiles from Mortimer, lights him at full strength. The
farther, five tiles away, adds about 6%.

**What is left.** With the lamps Mortimer is about 11% brighter than the
original's (mean 36.5 to 32.8), against 5% without. The poses differ, so
part of that is the crouch. The materials' colours are still not read,
and a material darker than white would close it.

**Unverified.**
- The materials' ambient and diffuse colours: taken to be white.
- That the lamps' lights are directional, not point lights. The
  original sets a position and a direction for each (0x5d0b7c); the
  direction alone is used here.
- The order of the lamps when a room has more than six. Ours are in
  object order, the original's in its room count's order.
- A lamp upstairs. The light's height comes from the ground under the
  lamp (0x478dfe takes no level). The person's root is on the ground
  too, so the two match, but no upstairs lamp has been compared.

**Still not ported:** the level's fade over 250 ms, the cap of 0.8 after
dark, and the person's flag that draws them at full light.

What would reopen this: a screenshot of a person in a dim room facing
the viewer that is brighter on the side away from the viewer, or a lit
person with no lamp in their room who is brighter than ours.

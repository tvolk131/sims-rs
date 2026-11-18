# A person is lit by the original's standard light, at their room's level

*Decided 2026-11-15.*

People on a lot were drawn with our own light: a shade from 0.45 in
shadow to 1 facing a light from the viewer's right. They are now lit as
the original lights them, as far as it is read (`raster::Lighting`,
`lot::level`, `lot::STANDARD_LIGHT`). The entry "How the original
lights a person" (2026-11-14) has what was read before. This entry says
what changed and what settled it.

- **The light keeps its place relative to the viewer.** Before setting
  the standard light, the original turns it by the view object's matrix
  at +0x130 (0x4b49f5). That matrix is a turn about the vertical by
  `((−rotation) & 3) × π/2`, built with the house camera (0x49b60e to
  0x49b642). The camera turns by the same quarter turns, so the light
  stays where it is relative to the viewer. Ours already did that.
- **Its direction.** `stdlight.lgt` gives the light a position (6.79,
  9.55, 6.79) and a direction (−0.644, −0.418, −0.641). The direction
  points from that position towards the middle, so it is the way the
  light shines, as Direct3D takes a directional light's direction. The
  direction towards the light is therefore (0.644, 0.418, 0.641). The
  drawing frame is the files' frame, so this needs no change of axes.
  At rotation 0 the house camera's eye is along (0.61, 0.5, 0.61), so
  the light is above and behind the viewer, a little lower than the eye
  (about 25° above the ground to the eye's 30°).
- **The level.** The person's level is their room's light, as the lot
  renderer stores it for the room's tiles (`sims_render::lot::room_light`,
  the room's shown light), over 255, held between 0.5 and 1. The
  original reads the level from the drawing's lighting record
  (0x4b4499). That record's only setter (0x4b4525) holds it between 0.5
  (0x6c7420) and the brightest a lit tile may be (0x6c741c, 1 by day).
  The lot's objects set it from the tile light map, the light byte over
  255 (0x467482, 0x4b0e24). That a person's record is set the same way,
  from the tile they stand on, is **unverified**.
- **The shade.** The original sets the ambient light to the grey
  `round(255 × level)` and the standard light's strength to the level,
  and Direct3D adds them up per vertex, held to 1. So the shade is
  `min(1, round(255 × level) / 255 + level × max(0, n · l))`. The
  material is taken to be white: a person at full light matches the
  screenshot below only if the ambient part is near 1.

**What a screenshot shows.** The original's screenshot of Mortimer on
the porch at medium zoom (outdoors, by day, so level 1) was compared
with ours drawn at full light everywhere. Over his 768 pixels the
original is 0.93 of ours at the median. The lot around him is 0.97,
from colour depth and filtering. The ratio does not follow any light
direction tried (eight signs of the standard light and five others; the
largest correlation is 0.15). So the original draws him with no visible
light side, as level 1 gives. Our old light drew his shadow side at
0.45 and was visibly darker.

**Unverified.**

*Later (2026-11-18):* the light's direction was taken in the wrong frame
and the lamps are now ported; see "The standard light shines from the
viewer's side, and lamps light people".

- That the vector is the way the light shines. A person at full light
  shows no side, so only a person in a dim room in a screenshot can
  settle it. A screenshot of Bella indoors (2026-11-14) is brighter on
  the viewer's left, but she was walking at an unknown phase and was
  not drawn to match.
- The person's level coming from their tile.
- The material's colours.

**Not ported.**
- The room's lamps: a light from each lamp of the room within 20
  units, at full strength to 15.
- The level's fade over 250 ms.
- The cap of 0.8 after dark (the lot renderer does not have it either).
- The person's flag (+0x114 bit 0) that draws them at full light.

`Canvas::draw`, the offscreen renderer and the pie menu's head, keeps
our own light, unchanged to the bit.

# How the original lights a person, and how far ours is from it

*Decided 2026-11-14.*

People are still drawn with our own light. This entry records what the
original does, read from its code, and what matching it would need.

**What the original does.** Before it draws a person (0x590c05), it
sets the lights (0x4b49c7), and puts them back afterwards (0x4b4cf5).
Direct3D's fixed-function lighting then adds the lights up per vertex.

- **A level from 0 to 1.** The draw reads a level from a lighting
  object it is given (0x4b4499): 1 when that object's byte +0x34 is set,
  otherwise its float +0x48, which a setter clamps (0x4b4525). The
  level is 1 when the person's object has bit 0 of its flags +0x114
  set. The person keeps its own copy (+0xe0). A new level starts a fade
  of 250 ms from the old one (0x5929ff, through the person's own
  method at vtable +0xe8). Reading it during the fade interpolates and
  clamps it to 0..1 (0x586dce, vtable +0xe4). The result goes to the
  lighting record (+0x20).
- **Ambient.** A grey of `round(255 × (low + (high − low) × level))`.
  `low` and `high` live in a static lighting object at 0x6f0150 and
  start at 0 and 1 (its constructor, 0x4b457c). With these defaults the
  grey is `round(255 × level)`. Whether anything changes them while the
  game runs was not checked (**unverified**).
- **The standard light.** One directional light from
  `GameData/Lights/stdlight.lgt`: white, intensity 1, direction
  (−0.644, −0.418, −0.641) in the files' frame. Each light of the record
  is turned by a matrix the game's view object gives (0x415711, +0x130)
  before it is set. Its strength is the same level.
- **Room lamps.** For each light of the room the person stands in, within
  a squared distance of 400 (20 units; which units these positions use is
  not pinned down), a directional light goes from the lamp towards the
  person. It is at full strength up to a squared distance of 225, then
  `1 / ((d² − 225) / 2 + 1)`. Its colour is (1.0, 0.984, 0.819).

**What a screenshot shows.** In two screenshots of the original, a
quarter turn of the view apart, a person facing the viewer is brighter
on the viewer's left. They are different people facing different ways,
so this only suggests that the light turns with the view.

**What we do.** One light fixed in the lot, from above and the viewer's
right (`raster::LIGHT`), with shade `0.45 + 0.55 × max(0, n · l)`,
where `n` is the vertex's normal and `l` the unit vector towards the
light. There is no level, no lamps and no fade.

**What matching needs.**

- The level. The simulation keeps each room's light from 0 to 1
  (`Room::light`, and which room a tile is in, on `World.rooms`), and the
  drawing can read it directly. That the original's level is the room's
  light is **unverified**: the lighting object passed to the draw, and
  what writes its +0x48, are not read yet.
- Which frame the view matrix (+0x130) is in. If it is the camera's, the
  light turns with the view, as the screenshots suggest.
- The materials' ambient and diffuse colours, which are not read.
- The room's lamps: their positions, from the simulation's objects.

**Stand-ins kept.** The base layer's clip and phase are not simulation
state yet. The animator only records whether the base slot holds a
layer (`Animator::base`). Which clip the original puts there needs more
reverse engineering on the simulation side. Until then the clip comes
from the saved person record and the phase from the animator's clock.

# People are no longer drawn mirrored, and the lot lies the right way round

*Decided 2026-11-14.*

People were drawn as the mirror image of the original's. Now they are
not, and they stand on the lot the way the lot drawing has it.

**How the mirror was found.** Cassandra Goth (person 16) wears a shirt
with a pocket protector on one side. Two screenshots of the original
(House05, sitting on the porch bench and walking on the lawn, both seen
from the front) show it on her own left. Our drawing put it on her
right. So the image was mirrored.

**Where it came from.** The file conversion mirrored the X axis
(`[-x, y, z]`), to get a right-handed frame, as the old prototype's
renderer did. But our camera reads its frame as the original's renderer
does: x right, y up, z away from the viewer. So the mirror was never
undone. A scratch check showed our bones were exactly the X-mirror of
the original's own composition (0x59615b, ported as
`sims_sim::pose::absolute`), at rest and mid-clip.

**The fix.** Positions are kept as the files have them. A rotation
`[x, y, z, w]` becomes `[-x, -y, -z, w]`: the original keeps a turn with
x, y and z negated (its bone matrix, 0x595f32, negates them again). Each
bone now lands exactly where the original's composition puts it. A test
checks this on a chain of bones whose turns do not commute.

**The lot's layout.** Our placement put sim x on model X and sim y on
model Z. With the house view camera that drew sim +x up the screen. The
lot drawing's projection (0x4b3c88, `sims_render::camera`), which was
checked against the original's screen, draws sim +x right and down.
The original's model matrix (0x592132) puts sim y on X and sim x on Z,
and turns the skeleton by (direction + 6) eighths. Here the lot is that,
turned half a turn about the vertical, so that our camera draws it as
the lot drawing does:

- sim x becomes model −Z and sim y model −X (3 units per tile, as
  before);
- a person is turned by θ + π/2 about the vertical, for θ its direction
  times 45 degrees.

A person still faces (sin θ, −cos θ) in sim x and y, which is the way
the simulation walks them (the check of person 259 still holds).
Direction 3 now faces the viewer, not direction 7. A test checks the
screen directions of sim +x and +y and that direction 3 faces the
camera.

**What is verified and what is not.**

- Verified against the original: which side is which (the pocket), and
  that bones compose as the original's code does.
- Checked only against the lot drawing's projection, not against a
  person in a screenshot: that sim +x goes right and down. The lot
  drawing was itself compared with the original.
- **Unverified:** the pixel scale and offset of a person against the
  lot, and the camera's exact angle for the 3D people.

The pinned renders of the gated tests change, and so do the viewer's
demonstration and the tests' facing (direction 3).

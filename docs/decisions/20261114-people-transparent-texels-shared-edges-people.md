# People: transparent texels, shared edges, and people made after the load

*Decided 2026-11-14.*

A review of `sims-people` found five defects. They are fixed as follows.

- **Transparent texels.** A texel's alpha was ignored, so a texel a
  run-length bitmap skips was drawn opaque black and hid what was behind
  it. Now a texel with alpha 0 draws nothing and leaves the depth buffer
  (the nearest depth drawn at each pixel) alone. A texel with alpha 255
  is drawn and sets the depth. A texel in between is blended over the
  pixel and leaves the depth alone. The canvas holds straight colour (not
  multiplied by alpha), as a PNG does, so the blend weighs what is below
  by its own alpha. The game's skins are either opaque or
  fully transparent, so the in-between case only keeps other textures
  sane. What the original does with partial alpha is **unverified**.
- **Shared edges.** Testing the alpha fix showed a second fault. A pixel
  exactly on the edge two triangles share was drawn by both. Now each
  triangle owns only its top and left edges (the usual "top-left" fill
  rule). Which side of an edge a pixel is on is always worked out from
  the same end of the edge, so the two triangles get exactly opposite
  answers even with rounding. A pixel near a shared edge is then drawn
  by exactly one of them. This changed the pinned renders of the Goths:
  7 to 43 edge pixels each, out of 8,000 to 16,000. No
  pixel became empty or newly drawn, so none of the change comes from
  transparency. The new digests were checked that way and then pinned.
- **Bit-field bitmaps.** A bitmap of type `BI_BITFIELDS` was read as if
  its colour masks were the usual ones. Now its masks are applied, for 16
  and 32 bits per pixel. A bitmap with an empty colour mask is refused,
  and so is one whose header does not fit in the file or whose pixels
  start inside its header or masks.
- **People made after the load.** With a snapshot, the viewer only drew
  people who were in the saved house. Person 259, the travelling salesman
  a script makes during the boot, was never drawn. Now the viewer draws
  every person object the snapshot holds. A person whose record names no
  clothes is drawn in its type's default outfit: the first descriptor
  (before any `;`) of the body, hands and head rows of the type's body
  strings (the string table its object
  definition names). These are the rows the old prototype's port of the
  original's outfit choice (0x58a178) reads. Whether the original draws a
  newly made person in them is **unverified**; the simulation does not
  keep the outfit it chose.
- **Walk clip.** Pressing W took the first three letters of the base clip
  as the walk clip's prefix, and panicked on a shorter name. Now the
  prefix comes from the skeleton: `c2o` for a child, `a2o` otherwise.
- **Weight test.** The test of a clip's weight started and ended at the
  same pose, so it proved nothing. Now it blends towards a different pose
  and checks weights 0, 0.5 and 1.

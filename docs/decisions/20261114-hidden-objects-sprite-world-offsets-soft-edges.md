# Hidden objects, sprite world offsets and soft edges in the lot drawing

*Decided 2026-11-14.*

A review of `crates/sims-render` found four defects. All four are fixed,
with tests.

- **Hidden objects are not drawn.** An object whose data word 34 is not
  zero is hidden; the simulation (`HIDDEN` in `sims-vm`) and the old
  prototype skip it, and now the renderer does too. Checked against the
  original: the Goth house file hides 18 objects, and one of them, a
  table on the road, showed in our far-zoom render but is not in the
  original's screenshot. At that camera (far zoom, rotation 0, floor 1),
  exact matches among non-grass pixels rose from 33.6% to 33.9% with
  walls cut away and from 39.5% to 39.7% with walls up. Objects inside
  other objects are not drawn at their slots yet, so a hidden container
  does not hide its contents here.
- **A draw group sprite's world offset is kept.** Each sprite of a draw
  group (DGRP) carries a position relative to the object. It was
  dropped. Its units: x and y are sixteenths of a tile in the object's
  own frame, and z is height, where one unit is `2 << zoom` screen
  pixels (16 at the near zoom). The old prototype divided x and y by 16
  to get tiles (`legacy/crates/sims-render/src/lib.rs`).
  - z raises the sprite on screen by `z * (2 << zoom)` pixels, rounded
    by adding a half and truncating, as the original's catalog preview
    does (0x438ae4 to 0x438b58, read by the old prototype in
    `legacy/crates/sims-render/src/object_preview/software.rs`). That
    the lot view places sprites the same way is **unverified**.
  - x, y and z all move the depth the sprite is drawn at. x and y turn
    with the object's facing, a quarter turn per two steps (the old
    prototype's rotation). The preview leaves x and y out of the
    sprite's screen position, and so does this; that is **unverified**
    for the lot view too.
  - None of the groups the Goth house draws now has a nonzero offset,
    so no render changed, and nothing here has been compared with the
    original.
- **Soft edges write depth.** A sprite pixel that is partly transparent
  now writes its depth as well as blending its colour, as the old
  prototype's blitter did. Before, a farther pixel drawn later could
  show through a nearer soft edge. This changed several pinned renders
  slightly. The compared camera did not change at all.
- **Sprite positions cannot overflow.** Draw group offsets are any
  32-bit number. Placing and clipping a sprite now use wider or
  saturating arithmetic, so a sprite far off the screen draws nothing
  instead of panicking. An image whose pixel count does not match its
  size, or overflows, draws nothing.

What would reopen it: a camera where an object with a nonzero world
offset can be compared with the original, or a reading of the
original's blitter that treats soft edges differently.

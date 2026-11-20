# The pointer pick is the original's: noted 3D graphics, then the sprite pick from the depth under the pointer

*Decided 2026-11-19.*

This replaces two rules: "a person within two pixels first"
(`pick::REACH`, `20261115-click-picks-what-frame-shows-pixel.md`) and
"the object whose pixel the frame shows". The plumb bob note
(`20261118-plumb-bob-picks-as-itself.md`) read the first half of the
original's pointer pick (0x48c9be). This note reads the rest and ports
it into `sims_game::pick` and `Lot::sprite_pick` (sims-vm).

## What the original does

1. **A noted 3D graphic.** A person's draw (0x590ebd to 0x590f84) and
   the bob's (0x5863eb to 0x586455) read the depth buffer at the pointer
   before and after drawing the mesh. If the value changed, that graphic
   is noted as under the pointer (0x58462f, the global 0x6f6bc4), and
   the last one drawn wins. The pick takes it when it stands on the
   floor shown. People and the bob are kept out of the depth the lot
   leaves: a dump with Sims on screen shows them in the colour buffer
   only.
2. **Otherwise the sprite pick** (0x4aa713, called in mode 1):
   - **The start tile.** The view's point-to-tile (0x4a97c3, mode 0)
     multiplies (x, y, depth, 1) by the view's inverse matrix. It gets
     the level from the height above the terrain (0x4b07b3: one floor's
     height or more is level 2, two is level 3) and converts the result
     to lot units (0x454fb9, rounded half away from zero). It falls back
     to the ground plane of the level shown (mode 1) when the depth is
     empty, when the level is not the one shown, or when the place is
     off the lot (0x422632). The view's rotation turns the place
     (0x47bc1f, which reflects fine positions about 1023). Outside the
     view's tile bounds (the scene's +0x70 to +0x7c, min + 1 to max − 1,
     tested on the view's tiles) the pick finds nothing.
   - **The walk.** On each tile, turned back to the lot's (0x434f58),
     the objects on the level are walked in link order (0x43f331,
     0x43f379). An object is found when its drawn rectangle holds the
     point:
     - the rectangle is the union of +0x160 and +0x170 (0x438f1b);
     - its left edge is rounded down and its right edge up to a
       multiple of four (0x438f71);
     - x is tested in [left, right) and y in [top, bottom).

     The last object found on a tile wins, and the walk ends after that
     tile.
   - **The fallback.** On the first tile only, an object that is not
     found is the fallback when:
     - its data word 40 has neither bit 0xc000; and
     - either the mode (global 19) is not live, or its definition word
       91 has bit 0.
   - **The steps.** The walk then steps toward the viewer: one column
     on, then one column back and one row on, then one column on, and
     so on. It takes 20 steps at most, so it looks at 21 tiles.
   - **The answer** is the found object, else the fallback.
3. The pick lists the selected Sim's pie on the id when it changes
   (0x48c834, the house tool's +0x2e written at 0x48c8d9).

## The port

- `pick::noted`: the person or bob whose pixel the frame shows at the
  pointer, if it stands on the level shown. This is the pixel itself,
  with no reach around it. `sims_people` already names each person's
  pixels and the bob's, and draws the bob after its holder.
- `Lot::sprite_pick` follows step 2 as written, turning by the rotation
  the frame was drawn with. The tile bounds are taken as the lot's inner
  tiles turned into the view (**unverified**).
- The drawn rectangles come from the canvas (`Canvas::drawn`): it keeps
  the union of the whole images drawn under each object's id since it
  was cleared. Our people are drawn as meshes and have no rectangles, so
  the sprite pick never finds a person. Whether the original's people
  have rectangles the sprite pick tests is **unverified**.
- The start tile comes from our depth, not the original's matrix:
  - Our depth is affine in the screen row and the height, with a fixed
    depth offset for each level above the first. So `Depth::unproject`
    inverts it exactly, for a given level, to the view position and the
    height above that level's floor (a unit test round-trips points at
    every zoom and level).
  - On drawn objects this lands within about one sixteenth of a tile of
    their positions: the fountain, fridge and counter.
  - 0x4b07b3 classifies a point by its height above the terrain under
    it (the mean of its tile's corners): one floor's height or more is
    level 2, two is level 3, anything lower level 1. Our depth gives a
    position only for a level taken, so the pick tries levels 1, 2 and 3
    and takes the highest whose position and height classify as that
    level. With none, it takes the level that level 1's position
    classifies as, at that level's own position.
  - Where more than one level fits, which the original's single 3D
    position would choose is **unverified**. On steep terrain, either
    choice misses in some made-up lots. Taking the lowest missed an
    upstairs bookcase beside a plateau's edge. Taking the highest misses
    a ground-floor bookcase whose level-2 reading lands on a 32-step rise
    next to it. The Goth replays do not reach such a pixel.
  - The ground-plane fallback is flat: it leaves out the terrain, which
    the original's may not (**unverified** on a sloped lot).
  - Whether our unprojection agrees with the original's inverse matrix
    is **unverified** beyond these objects.
- The game keeps the lot's depth from before the people are drawn and
  the camera the frame was drawn through. `Game::under` picks from them,
  so a view moved or turned since the last frame does not move the pick.
  Before the first frame, and off the canvas, the sprite pick finds
  nothing.
- The objects that hold a person (an in-use bench) draw after the
  people, as the original's moving pass does
  (`sims_render::draw_held`,
  `20261120-objects-holding-a-person-draw-after-the-people.md`; 0x4a9458,
  then 0x4aa1d7). So:
  - The noted person is read from the frame's IDs as they were once the
    people were drawn, before those objects draw over them. In the
    original the Sim's draw changed the depth at the pointer before the
    bench drew, so a Sim behind the backrest of the bench they sit on is
    noted. A Goth-house test pins this: at 7:12 AM, 1134 of the in-use
    bench's pixels cover Mortimer and pick him.
  - The depth the sprite pick unprojects is the one `draw_lot` leaves,
    without those objects, as the people are left out of it. That they
    are left out of the original's too is **unverified**: the dump that
    showed the people missing did not look at them.
  - Their drawn rectangles are kept as the lot's are, so the sprite pick
    finds them: the bench's other 2039 pixels in that test all pick it.

## What the replays show

The snack, Sit and cancel replays (`goth.rs`) now draw the house view
only after the updates the recording's frame schedule draws a frame
after. The window draws more often than that, so before this change the
pointer picked from a newer frame than the original's. With this pick,
every replay's differences from the recording are the ones it had
before, or fewer:

- **Through a wall, cancel 90: settled.** With the view drawn on the
  schedule, the replay's state equals the recording's at every update
  around 90. The pointer reaches the counter's spot at 90, but the last
  frame (after 63) shows the kitchen wall there. The wall's pixels
  unproject to the wall's tile (19, 31), in front of the counter's
  (18, 30), so the walk ends on the wall's tile with nothing found. The
  counter (119) is listed with the frame after 93, which cuts the wall
  away (the cancel test pins this). Before, the window drew after every
  update, and its state differed from the recording's at 91 to 93.
- **Under the still pointer, cancel 257–259 and Sit 261, 291–560:
  open.** At the original's pointer, cancel's (200, 123) in the
  recording's pixels:
  - our lot depth unprojects to (188.4, 355.4), tile (11, 22) on level 1;
  - the 20 tiles from there cross the kitchen objects 29, 199, 99, 98,
    114 and 118, but none of their rectangles holds the point;
  - so ours lists nothing, while the original lists something whose
    listing leaves Mortimer's last result 1 and his word 50 at 0. Many
    objects would.

  In Sit, our pick finds object 101 under the pointer once the pie
  closes, but the listing's effect that the original shows at 261 does
  not show in ours; why is not known. A recording has to say which
  object the original listed (below).
- **The salesman or the bookcase, Sit 590: open.** The old pick found
  the salesman (259) only through its reach. The pointer's pixel is a
  1-pixel gap between the front door (51) and the salesman. It
  unprojects to (416.0, 399.0), tile (26, 24). The walk finds the door
  on its third tile. The bookcase (132), on tile (25, 24), is behind the
  start, where the walk never goes. The ground plane's tile would find
  the bookcase, so the original most likely had a different depth at
  that pixel, for example with the door drawn open. The differences in
  the Sit replay (data from 591 to 620) are the same as before.

## What would settle the open cases

- **Cancel order, updates 250–262 and 85–95:**
  - a write watch on the house tool's current-object word (+0x2e of
    0x48c834's `this`, written at 0x48c8d9), or a trace of 0x48c834's
    id argument;
  - a write watch on 0x6f6bc4 (the noted graphic);
  - screenshots at 257–262, after the pie closes.
- **Sit order:** screenshots around updates 261, 291 and 590, to show
  what the original draws at the recording's (384, 250), and whether the
  door is open.

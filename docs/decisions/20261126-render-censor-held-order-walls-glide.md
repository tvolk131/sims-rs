# 2026-11-26: Censor mosaic, held objects before their Sim, wall and object order, linear glide

## Context

The render track still had these gaps:

- The censor mosaic (`0x4ac707` → `0x4ac301`) was only in the legacy
  prototype.
- What a Sim holds was drawn after all the people. The original draws it
  just before the Sim (#442 follow-up).
- Walls were drawn segment by segment and objects by depth, not in the
  original's order.
- The camera glide to a Sim was a 350 ms cubic ease that we chose.
- The legacy audit listed two small rules: house 99's zoom floor and the
  ground under floored tiles.

## Censor mosaic

`sims_render::censor` ports the mosaic. The frame calls it between the
people and the objects that hold someone, as `0x4b3510` does (`0x4b017b`,
`0x4ac707`, then `0x4aa1d7`). Steps:

- **Who.** A person the people pass drew this frame whose censor word
  (person data word 30, `+0x5c8`) is not 0. A person on a level above the
  camera's is skipped (`+0xfe`). People go in ID order.
- **Rectangle.** The rectangle is the screen bounds of the person's
  censor boxes, as `0x591248..0x5912b4` keeps them in `+0x230`:
  - The boxes are the bindings of the skeleton's `-needs` appearance whose
    censor bits meet the word. A word with bit 7 chooses all seven boxes.
  - Each box is posed on the person's bones and projected like the
    person's own mesh.
  - The least x and y are cut down, the greatest are cut up with `+ 0.999`,
    and each is offset by `[0x6f6d74]`/`[0x6d0904]` (0 and -3, shifted by
    the zoom less one).
  - `0x4ac707` then pads the rectangle by 1, 2 or 4 pixels and moves it
    down 3, 6 or 12. That cancels the -3 offset.
- **Threshold.** The person's position (`+0x240`, the skeleton root) is
  placed as a moved object (`0x4b966b` → `0x4b9991`). A pixel takes part
  when its depth is greater (farther) than `Depth::stand` of that anchor at
  the row.
- **Which depth.** The test reads the lot's depth from before the people,
  plus the depth the Sims' held objects wrote, not the canvas's. The
  original draws a person's mesh through Direct3D, and held objects as
  sprites. We assume (unverified) that the mesh does not write the software
  depth buffer `0x4ac301` reads. If it did, a person's front would hide
  their own mosaic, which the original does not show. A plate held in
  front of a censored Sim keeps its pixels out of the mosaic. As in the original, the row asked about is the clipped top plus
  the row within the block, for every block row (`0x4ac4c8`, `0x4ac649`).
- **Blocks.** Blocks are 2, 4 or 8 pixels by zoom. Pixels are read in the
  original's 5-6-5 unpacking: channels zero-filled, not widened. Each
  channel's sum is divided by the count, truncating. Three draws add
  `(draw & 15) - 8` in red, green, blue order. The result is clamped,
  packed and written to the pixels that took part. A block with none draws
  nothing.

The surface is 5-6-5, so only that format is ported. The legacy's 5-5-6
unpack lost a green bit.

**Random draws.** The original draws from the global stream (`0x46f457`),
which the simulation also uses. The presentation must never change the
world. So `sims_game::people::People` keeps a copy of
`World::global_rng`:

- The copy is taken again after every tick, whenever `World::ticks`, the
  stream's state or the game's restore generation changes. A restored save
  draws as it did when saved.
- Between ticks, only the mosaic advances the copy, so a paused frame
  still varies.
- The first censored frame after a tick draws as the original's would.
  The original's draws would also shift the simulation's later draws; ours
  do not.

Headless pins (`TICK_43200` and the others) are untouched.

The censor's rectangles join the frame's dynamic rectangles, so the next
frame repaints them. Open-pack Sims have no skeleton and get no censor.

## Held objects before their Sim

`People::draw_on_lot_in_viewport_tracked_holding` calls back just before
each person is drawn, with that person's attach points. The frame then
calls `sims_render::draw_held_by`:

- It draws what the person holds (`held_by`): the person's slots, minus
  the first entry (a tile link), and what those hold, at every depth.
- The walk stops at people.
- What an earlier person's draw drew is left out. Slot lists that
  disagree could otherwise draw an object twice.
- `holds_person` also takes in each person's own slot tree. When a
  holder's link words reach the person but its slot list does not, the
  lot pass would otherwise draw the person's plate as well.
- The person's items keep slot order ([Object order](#object-order)).

Then `draw_held_except` draws what holds a person (a bench), minus what
the people already drew. Effects:

- What a Sim without body strings holds stays at its own position after
  the people, as before.
- Open-pack mode keeps the old pass.
- What a person holds is now in the canvas before `people_ids` captures
  the people's rectangles. So a held object's pixels inside them count
  for a click, as they would in the original's frame.

Still not ported:

- **Slot 1** animated objects (`0x58f8ff..0x58fa91`).
- **Pets.** A pet's held objects go with the other held objects, because
  the pet's draw is not split out.

## Wall order

`wall_order` builds the original's wall graph (`0x540619`):

- **Corners.** Each corner gets an 8-bit mask of the segments that meet
  it. Each corner is classified as a junction, an end or plain. A corner
  where a wall and a diagonal leave on the same side is invalid.
- **Runs.** From each junction or end, in key order, a run starts down the
  south slot (`0x44`) and the east slot (`0x88`). It follows straight on
  through plain corners (`0x53cfa7`). Each run is inserted before runs
  with an equal key (lower bound, `0x5409ab`).
- **View.** Each run is turned to the camera's rotation. Its ends are
  swapped, and its segments reversed, when the far end has the smaller
  key (`0x5404b5`). Then the runs are sorted by start key
  `(x + y, x - y)` with the original's sort (`sims_core::sort`,
  `0x540809`/`0x541389`).

Walls are drawn run by run, each run from its start.

Boundaries:

- **Rotation history.** The graph is built with the view's rotation at
  that time. Each later view turn turns the graph by the difference and
  sorts again (`0x479360`). So the order of runs with equal keys depends on
  the rotations the view went through since the walls last changed. We
  build at rotation 0 and turn once, which matches a graph built at the
  current rotation only when the unturned order's ties survive the single
  sort.
- **Leftover segments.** Segments that no run covers (an invalid corner's)
  go last, by the old segment key.
- **Sort routine.** `0x541389` is assumed to be the same `std::sort` as
  `0x449cbc`. It is the same template, but that instantiation was not
  diffed.

## Object order

`object_order` keeps the depth order and adds the holder walk:

- **Order of objects in no slot.** They stay far to near by depth anchor,
  then by ID, as before.
- **Slot contents.** Each object is followed by what its slots hold, slot
  by slot, at every depth, as the holder's walk draws them (`0x437c56`).
- **Holder outside the pass.** A holder not in the pass (hidden, or a
  person) keeps its place in the walk: under its own holder at its slot,
  or, held by nothing in the pass, as a root at its first object's key.
  What it holds stays together in slot order.
- **Loops.** Holders that hold each other, which no root reaches, are
  broken at their lowest-keyed object, which joins the roots.

The presenter's own order was tried and is **not** used. It sorts by each
object's view tile key `(x + y, x - y)` with the original's sort (legacy
`rebuild_order`), from object ID order. Against the original's
screenshots in the ratchet:

| Order tried | Result |
| --- | --- |
| Tile key, from ID order | 6 pixels change in two 0816 scenes. All 6 matched the original before and none after. `far-r0-cut-0816` objects fall to 99.90%, under its 99.91% floor. |
| Tile key, from depth order | The same 6 pixels. |
| Tile key with the legacy's swapped axes | Far worse: 98.73% to 99.29% objects in three scenes. |
| Depth order (kept) | Every pin is unchanged. |

So the presenter's tile, or its key, is not understood yet. The order
stays an open item: find which objects those pixels belong to, and what
tile the presenter reads for them. The original also leaves out objects
on the lot's outermost tiles; those are still drawn here.

## Linear glide

`CameraPan::sample` now follows `0x4c9a27`:

- It takes 250 ms (`[0x6c7788]`, never written).
- Time is the millisecond clock.
- Each axis keeps a single-precision slope `(target - start) / 250`
  (`0x4733b7`).
- The value is `target - slope * (end - now)` (`0x473390`), rounded half
  away from zero (`0x4c9b4e`, `0x4c9b87`).
- x glides in the original's column (`pick::ORIGIN_COLUMN`).

## Small rules

- **House 99.** House 99 zooms out no farther than zoom 2
  (`0x4cedc0`/`0x4ce9f8`): `Zoom::zoom_out_on(house)`, with
  `Lot::house` from global 10. The shell's zoom-out path uses it. The
  control panel's zoom-out button still enables on any zoom above 1. That
  is in `hud.rs`, outside this change: follow-up.
- **Ground under floors.** The ground under a level-1 tile is drawn unless
  the tile and its four neighbours all have floors and all four neighbours
  are water (`0x4af029`; `Arry 10` not `0xff`, `0x478f4a`). The legacy
  audit's wording, "a neighbour is water", is inverted. The legacy
  `ground.rs` and the disassembly agree with this rule. The global enable
  byte `0x6c724a` is not ported, so the rule is always on.

## Verification

Unit tests:

- **Censor:** pack/unpack, the padded rectangle at every zoom, block
  averaging behind the threshold, three draws per non-empty block, the
  threshold row quirk, noise and clamping.
- **Walls:** corner classification, run order at rotations 0 and 2, every
  segment exactly once at each rotation, leftovers last.
- **Ground:** the rule.
- **Objects:** holders before what they hold, a holder outside the list
  (or hidden) keeps slot order, and loops join the roots.
- **`held_by`:** on the bench, person and drink tree.
- **Glide:** the glide's samples and its equal steps.
- **Zoom:** house 99's zoom floor.

The install-gated suites of `sims-render`, `sims-people` and `sims-game`
pass, and so does the screenshot ratchet (see the PR).

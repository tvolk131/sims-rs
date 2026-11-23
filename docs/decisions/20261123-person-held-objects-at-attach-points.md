# A Sim draws what it holds at its hand

*Decided 2026-11-23.*

This replaces the last point of "Held objects stand on their holder's
slot" (`20261114-held-objects-stand-their-holders-slot.md`): "objects a
Sim holds stay where they were". The rest of that entry still applies to
objects held by other objects.

A Sim's snack plate was drawn on the floor at the Sim's feet. The plate's
own position is its holder's, and we drew person-held objects there with
no offset.

## What the original does

- The generic slot walk that draws a holder's contents (`0x437c56`) skips
  holders that are people (OBJD `+0x12 == 2`).
- A person's draw (`0x459dc0`..`0x45a131`) draws its own slot objects first,
  then its `+0x148` list, then the Sim (`0x4378bc`), all depth-tested.
  Each slot object goes through `0x59185d(ctx, k, object)`. That call takes
  the animator's attach point `k` (`0x59293d`), puts its position and the
  person's level (`+0x110`) into the draw context (`0x4b966b`), and calls
  the object's draw (vtbl `+0x30`).
- Slots map to attach points like this:
  - slot 0 (the carry slot) and slot 4 use point 0;
  - slot 2 uses point 3;
  - slot 3 uses point 1;
  - slot 1 is not drawn here.
- The animator binds its attach points when it makes the body
  (`0x5924a4`, from `0x5884cc`). They come from the person type's body
  strings (`STR# 200`, counting from 1, read at `0x5924db`..`0x592585`):
  - point 0 (`+0x140`) is the bone of entry 6 at the named point of
    entry 7;
  - points 1 and 2 (`+0x144`) are the bone of entry 4 at the named point
    of entry 5;
  - point 3 (`+0x148`) is `PELVIS` itself.

  A bone the skeleton lacks (bone names match in their exact case), or a
  point name the original does not know, leaves the point unbound, and the
  slot draw skips an unbound point (`0x459e2c`, `0x591874`): what the slot
  holds is not drawn, nor anything on it. Nearly every character file has `HEAD`, `Top`, `R_HAND`,
  `Palm` as entries 4 to 7. The Side Show Assistant
  (`NPC_SideShowAssistTop.iff`) leaves entries 6 and 7 empty, so it has no
  point 0.
- A named point is a fixed matrix on its bone (`0x5a1631`), a move and
  then a turn:
  - "Palm" moves (0.5, 0, 0);
  - "Top" moves (0.8, 0, 0);
  - "HeadFake" moves (0.8, 0, 100);
  - "Shoulder" moves (0, 0, 1);
  - no name is the bone itself (`0x5a199b`).

  The attach position is the translation row of point · bone · skeleton
  (`0x59640d`, `0x59638f`; row vectors, `0x49b87c(out, a, b) = a · b`). The
  turn comes after the move, so it does not change the position: it is the
  bone-local move put through the posed bone.

## What we do

- `sims_people::attach` binds a person's points from its type's body
  strings as the original does, and places them on its posed bones, which
  already match the original's composition.
- `People::held_attach` turns them into lot coordinates: unrotated fine
  x and y, and height in height units. Like the original, these leave out
  the level offset the person's model is moved by. The original adds it at
  `0x5922d2`..`0x592394` and takes it off again before it projects a sprite
  (`0x4b9716`..`0x4b9750`). The projection (`0x4b9991`, our
  `place_moved`) then raises the sprite to the person's level itself.
- `sims_render::draw_held_with_attach` takes them. A person in the holder
  chain starts the held object's position at the slot's attach point
  (`PersonAttach::for_slot`), and object holders above it add their slot
  offsets as before. The held objects are drawn in the existing pass after
  the people, depth-tested, as before.
- What sits in a slot whose point is unbound is not drawn, nor anything
  stacked on it, as in the original.
- If there is no point (slot 1, a person not drawn this frame), or the point
  is not finite or is more than a lot's width from the object's own
  position, the object stays where it was and is drawn as an unheld
  object. The range check keeps a malformed pose from overflowing the
  integer projection.

This is presentation only. The simulation, strict parity and the Goth
43,200-tick hash are unchanged.

## Result

Mortimer's snack plate (object 263, his slot 0) is drawn in his hands at
chest height while he carries it from the fridge. Before, it was drawn at
the bottom of his figure. `a_carried_snack_is_drawn_in_the_carriers_hand`
checks this on the Goth house.

A capture of the original on the same order (the Goth house, default
camera, "Have A Snack" from the fridge) shows the same thing: the blue
snack is in Mortimer's hands, about a third of his figure's height below
the top of his head. Ours draws it at about the same height. The two poses
are not the same frame (in the capture he stands by the fridge, in ours he
walks), so this is a visual check, not a pixel match.

## Unverified and not ported

- Slot 1 (the `HEAD` record, `+0x48`). The tail of `0x58dc72`
  (`0x58f8ff`..`0x58fa91`) puts an object there at its bone's position,
  but only an object that has its own animator, and ticks that animator.
  The standard plumb bob is held there; `sims_people::bob` already draws it
  from the same record (`0x58db05`..`0x58db19`). We have not ported the
  tail for any other animated object. We have not checked what the
  original does with a slot 1 object that has no animator (the person's
  draw skips slot 1). We keep drawing such an object at its own position.
- Draw order. The original draws a person's held objects before the Sim
  (slot 0 at `0x459e45`, the Sim at `0x45a131`). We draw them in the held
  pass after all people. Both are depth-tested, but the order still
  matters:
  - a nearer translucent edge drawn after the Sim blends over the Sim,
    where the original's blends over what was behind;
  - the original's held sprite writes depth first, so it can reject the
    Sim's later pixels.
- The points come from the people drawn earlier in the same frame
  (`People::draw_in_viewport`). A Sim that frame did not draw (outside
  the viewport) gives none, so what it holds is drawn at its feet. That
  spot is next to the Sim, so it is outside the viewport too unless the
  Sim is right at the edge.
- The other callers of `draw_held` (`sims-people`'s lot tests,
  `perf_part4`) still pass no points.
- Pets: their skeletons have no `R_HAND`. If a pet's body strings name it,
  its point 0 is unbound and what it holds in slots 0 and 4 is not drawn,
  which is what the binding above implies. We did not check this against a
  capture.
- Body strings we cannot read give a person no points (what it holds stays
  where it was), not unbound ones, so a loader failure does not hide it.

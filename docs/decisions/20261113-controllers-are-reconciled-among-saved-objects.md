# The controllers are reconciled among the saved objects, and the grid repair clears only what is inside the lot

*Decided 2026-11-13.*

Load step 8 makes the lot hold one of every lot-wide controller, now that
the saved objects are on it. Load step 9 clears cells of the ground
level's object grid that name an object which is not there. `sims boot`
runs both and stops before step 10.

What was decided:

- **Step 8 is the reconciliation of step 3, called on the full lot**
  (`Interpreter::reconcile_controllers`). Creating the missing controllers
  needed no new code: the factory, the primitives and the services the
  141 controllers reached on a bare lot were enough for the 137 created
  here.
- **Later duplicates are removed.** The original lists the objects whose
  definition has a nonzero global word once, in registration order. For
  each controller definition it keeps the first listed instance and
  removes each later one at once (`Interpreter::remove_now`), scanning the
  list again from the start after each removal. The function now returns
  what it created and what it removed (`Reconciled`), and so does
  `Interpreter::resize`.
- **An object can be deleted on a lot with people, if no person refers to
  it.** The original tells every person about a deleted object. A person
  that does not refer to it changes nothing. A person refers to an object
  when it is using it, or when its running, last or a queued action
  targets it or shows its icon. Deleting an object a person refers to
  fails with an error that names the person, before anything changes.
- **Step 9 is a step of `World`** (`World::repair_grid`), because it runs
  no script. It looks only at the ground level's grid, and only at cells
  inside the lot's size. A cell is cleared if its ID is negative, names no
  object, or names an object whose tile and level are not that cell on
  level 1. The tile is the position divided by 16, rounding down
  (`World::tile`).
- **"Restore placement links" is not a step.** The roadmap listed it with
  step 9, after the old prototype, which built its own link tables at
  this point. The original's function for steps 8 and 9 (0x43fc02) ends
  after the grid repair. The links are state the object read already
  filled: the grid, and each object's slots. Nothing is rebuilt.
- **Step 11 has its state change** (`World::discard_removals`). It is not
  called, because step 10 is not ported.
- **The snapshot format stays 5.** `World` has no new field.

How it was checked. The old prototype kept, privately, a trace of the
original loading the Goth house, with the game stopped at several moments.
One is the return of 0x43fc02, after steps 8 and 9. Our state after both
steps was compared with it outside the repository.

Equal for all 382 objects: the ID and GUID, the 72 data words, the temps,
the attributes and their number, the scheduler word, the last result,
each frame's node, stack object, arguments, locals and saved primitive
state, the relationships, the number of slots, the position, the tile and
level bytes the grid repair reads, and the mark of an object that asked to
be removed without resets. Equal for the three people: the 256 words of
person data. Also equal: the registration order, the tick, the lot size,
the 64 globals, both game random streams, the four IDs queued for removal
(42, 44, 278, 280), the scheduler cursor, the funds, the indicator
references, the one instance list, and inside the lot's 52 tiles the
terrain heights, both levels' floors, walls, object grid and flags, the
pool and the water.

The comparison is not empty. Between the stop after the object read and
this one, the trace gains 137 objects and two queued IDs and its global
random stream moves. Nothing else it holds changes: no saved object, no
global, no type attribute, no grid cell. The same is true here.

*Slots.* The trace also saved pages of the game's memory at each stop.
They hold the slots of 198 objects at this stop and of 149 saved objects
at the stop before, 216 objects in all. For each, the object ID in every
slot equals ours. Those objects hold 30 of the 32 links between objects
that our state has.

*The grid.* In the trace every occupied cell of both grids names an
object whose tile bytes are that cell, before and after step 9, so the
original cleared nothing. We clear nothing either. The saved grid does
name eight objects that are elsewhere, all in cells outside the lot's 52
tiles; scanning the whole 64 would clear them. That the scan stops at the
lot's size is from the original's code, which was read for this port, and
from the old prototype, which ran that code alone on made-up grids with a
size smaller than the array.

What did not match. Object 19's second frame still has tree ID 0x8118 in
the trace and 0x118 here, as after the object read. Nothing in steps 8 or
9 touches it. In the trace the frame is gone after step 13, when the
object's thread holds only its Main frame. It is the only frame with that
bit at any stop looked at. It stays unexplained.

The C runtime's random stream and six type attribute words belong to the
game that was running, as before. With those copied from the trace's
state at the start of the load, they are equal at this stop too. Step 8
draws nothing from the C runtime's stream.

What is **unverified**:

- **Removing a duplicate controller.** The Goth house has none, so no
  capture shows it. The selection follows the old prototype, which
  compared its loop with the original's 0x442b9f run alone on 97 made-up
  catalogs, with the factory and the removal replaced by stubs.
- **That a person who does not refer to a deleted object changes
  nothing.** From the old prototype, which compared its port of the
  person's notification with the original's code run alone. No capture
  holds a deletion on a lot with people. Which of a person's fields count
  as a reference follows that port too; where it tests the frames of the
  person's thread that use an object, we test the saved list of uses.
- **Clearing a grid cell.** No capture shows one being cleared. The rule
  is from the original's code and the old prototype's isolated runs of it.
- **Cells outside the lot's size.** The trace does not hold them. We keep
  what the house saved there.
- **The slots of the other 166 objects**, and every slot field but the
  object ID.
- **`World::discard_removals` in its place.** The trace shows the queue
  going from four IDs to none with no object changing. Our state at that
  moment was not compared, because the load does not reach it.

What is not modeled and fails loudly instead: removing a duplicate that
is on the lot, holds or sits in something, is multi-tile, or has a
callback for leaving the lot; removing one a person refers to; a
controller that a script removed while the controllers were being
reconciled, where the original would use freed memory; and a ground grid
that is missing or not 64 by 64 two-byte cells.

What is not modeled and not compared: the animation tester's ID, which
step 8 notes for the interface; the log the second pass of step 9 writes;
and two flag bytes the object registry sets around these steps.

What the trace shows about the next steps, noted here because it changes
the plan. Step 10 needs the lot's rooms, which the original builds
between step 9 and step 10 (the trace has 2 rooms before and 11 after).
Working rooms out from the walls is the next piece to port. And the two
IDs step 3 left in the removal queue, 42 and 44, name saved objects after
step 6e. Step 11 empties the queue before anything acts on it, so it must
not be skipped.

What would reopen this: a house that saved a controller twice; a house
whose grid names a missing object inside the lot's size; a capture of
either; or finding what sets bit 15 of a frame's tree ID.

# Clearing the lot deletes objects one at a time, and the type table is read before the objects

*Decided 2026-11-13.*

Load step 5 removes every object the lot holds before the house's own
objects are read. Removing is not dropping: each object leaves the lot,
resets one last time, and is then unlinked. The reset runs the object's
Reset tree, a script, so the clear can change globals, draw random numbers
and queue removals. This entry also corrects the order of steps 6 and 7 in
`docs/roadmap.md` and says where the load now stops.

What was decided:

- **Deletion is ported for the simple case** (`crates/sims-vm/src/removal.rs`).
  `Interpreter::remove_now` takes an object off the lot and deletes it.
  `Interpreter::delete` releases the ownership of interface events, runs
  the last reset, and unlinks the object from the registration list and
  from its type's instance list. `Interpreter::reset` runs the Reset tree
  on top of the object's thread and then restarts the thread at Main; an
  object with no Reset tree only leaves the lot. `Interpreter::clear` is
  step 5: the two indicators first, then the head of the registration list
  until it is empty.
- **The removal queue keeps IDs whose objects are gone.** The original's
  queue holds bare IDs. The clear does not touch it, and the load erases
  it later (step 11). So after the clear `World::removals` still names 42
  and 44.
- **An instance list outlives its instances.** A type's list in
  `World::instances` stays, empty, when its last object is deleted. An
  object read or created later is appended to it.
- **`World` holds the indicators' IDs** (`World::indicators`, 0 for none).
  The clear needs to know which objects they are, and it sets both to 0.
- **The changed flag is state of the running game.**
  `World::changed_types` holds the GUIDs of the types whose scripts
  changed since a house that listed them was saved. The original keeps
  the flag on each definition. Reading a type table sets or clears it for
  the types the table lists and leaves the others alone.
  `Interpreter::read_types` does that and rebuilds `World::types`.
- **Steps 6 and 7 run in another order than the roadmap said.** After the
  clear the original reads the lot arrays, renumbers the saved floors and
  walls, reads the type table and marks the changed types in the same
  pass, clears the lot a second time, and only then reads the objects. The
  roadmap had the changed types after the objects. The step table now
  lists 6a to 6e.
- **`World::from_house` is split.** `World::read_arrays` and
  `World::read_objects` read onto the lot the load holds and keep what the
  earlier steps left. `from_house` stays as a way to look at a house file.
- **`sims boot` stops after the lot arrays.** The next thing the original
  does, renumbering floors and walls, is not ported.
- **The snapshot format is 4.** The two new fields are at the end of the
  encoding.

How it was checked. The old prototype's harness captured the original at
the moment the clear returns, loading the Goth house. Our state was
compared with that capture outside the repository. Equal: no objects, an
empty registration list, the tick, the lot size, the 64 globals, both game
random streams, the queued removals (42 and 44), the scheduler cursor, the
funds, both indicator references (0), the one instance list (empty), and
the type attribute records' GUIDs, order and lengths. Eighteen controllers
ran a Reset tree on the way. The test
`the_goth_house_clear_removes_every_object` pins a digest of our state.

The state when the resize returns was not compared again. Its encoding is
the old one plus the three bytes of the new fields, and without those
bytes it hashes to the digest the earlier comparison pinned.

For the type table, a capture taken after the original had read it shows
the three Goths' own types: an Init signature of 20 for each, the changed
flag set, and a Main signature never worked out. We get 20 and the flag
for all three. That also confirms the rule that Main is only looked at
when Init matches.

What did not match, and why. The same things as when the resize returns,
and for the same reason: the C runtime's random stream and six type
attribute words belong to the game that was running. With those copied
from the capture into the lot before the resize, they match after the
clear too, so no Reset tree changes either one here.

What is **unverified**:

- **The lot arrays** (`World::read_arrays`). No capture covers the
  arrays after they are read. Which arrays are read, and in what order,
  comes from the old prototype's reading of the code. A missing array is
  an error here; the original stops reading arrays and carries on.
- **The other 110 changed flags and the rebuilt type table.** Ported from
  the old prototype, which compared its version with the original's code
  run alone on made-up records.
- **`World::read_objects`.** No capture covers the object read. It copies
  saved fields and does not construct anything, so it is not a load step
  yet and `sims boot` does not call it.
- **Deletion paths the 143 objects do not take.** A Reset entry whose
  tree is missing, a Reset tree that removes or creates objects, and an
  object that owns interface events are ported from the old prototype and
  covered by synthetic tests only.
- **The reset outside a deletion.** `Interpreter::reset` with a Reset
  tree is the same code path, but nothing calls it that way yet.
- **The leftover type table.** After the clear `World::types` still lists
  the controllers' types. That table is our bookkeeping, and the type
  table read replaces it.

What is not modeled and fails loudly instead: removing an object that is
on the lot, in a container, holding something, multi-tile, a person, or
has a callback for leaving the lot; deleting anything while a person is on
the lot, because people must drop their references to it; resetting an
object with no Reset tree outside a deletion, which rebuilds and replaces
it; reading a type table onto a lot that holds objects; a lot array that
is missing or not 64 by 64; and a house with a GUID translation table.

What is not modeled and not compared: the interface being switched off
during the clear, the objects' graphics, the pools of spare indicators,
the redraw of the room a removed light was in, the deletion mark on the
object (only a person's reset reads it), type ID 0 in the rebuilt table,
and type tables older than version 2.

What would reopen this: a capture taken after the arrays or the objects
are read; a lot whose objects are on tiles when it is cleared; or a house
with custom floors or walls, which makes the renumbering matter.

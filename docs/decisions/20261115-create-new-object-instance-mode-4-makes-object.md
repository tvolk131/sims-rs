# Create New Object Instance mode 4 makes the object in the stack object's first slot; flag 0x1 skips a tile with its type

*Decided 2026-11-15.*

Mode 4 of opcode 42 (0x44e3a2) takes the stack object as the container,
and its position, level and room (data word 29). The new object goes
into the container's first container slot (slot 0 as scripts count it).
Before it is made, the common tail (0x44e4c5) runs three tests, any of
which takes the false branch:

- with flag 0x1 (0x44e4e1), an object of the same type stands on the
  container's tile, or is held there however deep (0x43f331, 0x43f379);
- the position is off the lot (0x422632);
- the container's room is not -5, and the lot's room map gives its tile
  another room (0x44e582).

Then come the slot's place test (`fits_in`), the removal of an object
the slot refuses, and the put. Flag 0x1 is the same tail code for every
mode, so it is ported for modes 6 and 8 too. Mode 6's tile is off the
arrays, where it finds nothing. Mode 8 keeps the room filter at -5, so
its room test never runs. Making a person, a type that belongs to a
group, a person as the container, and a container that the new object's
Init removes still fail loudly, as do the other modes and flags.

The fridge's snack tree (0x1019 node 1) runs mode 4 with flag 0x1 on
the fridge. In play mode, after a player's Have A Snack order at tick
51, Mortimer now puts the snack in the fridge at tick 634. He stops at
tick 748, on Grab (primitive 4, tree 0x100d node 9). Strict mode on the
Goth house is unchanged. **Unverified**: no capture covers a mode 4
creation. Reopen when a recording of the snack order disagrees.

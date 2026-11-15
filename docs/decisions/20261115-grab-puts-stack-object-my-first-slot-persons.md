# Grab puts the stack object in my first slot; a person's slots include its own five

*Decided 2026-11-15.*

Grab (opcode 4, 0x451dda) ignores its operands. It looks the stack
object up in the registry (0x40743b) and takes the false branch when it
names none. Then it asks the stack object's place test (virtual function
0x54) for my slot 0 as scripts count it, at my position and level, and
on a refusal takes the false branch with nothing changed. Otherwise it
puts the object there with the lot told (virtual function 0x58) and
takes the true branch. Only three classes of object exist, found from
the three callers of the base constructor 0x43189d. An ordinary object
(vtable 0x666d34) and a person (0x667378) share 0x433311 and 0x433340,
so `fits_in` and `put` serve both. A tile of a group (0x666c20) has
0x422abb, which refuses any container at once (0x422ac8, code 0x10), so
Grab takes the false branch for it; its test on the ground still fails
loudly. A put that fails partway puts the lot back as it was before the
Grab.

A person's container slots, which the place test reads, are its
definition's (0x431aaf) followed by five records its post-constructor
adds (0x45962b). Each is made by 0x46de93, which gives an offset of 0
and a height of 0, and only the height or the flags are then changed.
None goes through the SLOT converter that turns a height of 0 into 1 or
5. In order: height 6, the slot a person carries in; height 0 with flag
2 and the name "HEAD" (0x6c4528); then three at height 1. The place
test's step for a person as the container (0x432e57) is ported with
them. A container whose definition type is 2 takes nothing unless the
simulator's mode (global 19) is 0 (error 0xd). With the slots ported,
Create New Object Instance mode 4 no longer refuses a person as the
container.

The snack order's fridge tree 0x100d (node 9) runs Grab for Mortimer on
the snack (object 263) the fridge made. In play mode, after the Have A
Snack order at tick 51, he takes it into his carry slot (saved slot 1)
at tick 748. The snack's data word 4 is 189, which has the bit for height 6. He
now stops at tick 839, on Snap mode 4 (0x4549c7, tree 0x0127 node 2).
Strict mode on the Goth house is unchanged. **Unverified**: no capture
covers a Grab, and nothing has compared a person's slot records with the
original's memory. Reopen when a recording of the snack order
disagrees.

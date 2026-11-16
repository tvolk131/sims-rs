# The person vector, and K = 416

*Decided 2026-11-14.*

Tick 409 of the Goth house differed silently: person 18's waiting tree
(semi-global 8276, calling 8291) looked at person 16 where the original
looked at person 259. Tree 8291 walks the people with Set To Next mode
1, and for each one in its room but itself draws one of two, looking at
the first that draws 1. Ours walked the object registration list, where
person 259, made at run time, comes first: `[259, 21, 18, 16]`. The
random streams agreed after the tick, so both sides drew three times;
with the draws ours made (1 to look round, then 0, then 1), the
original's result needs person 16 before person 259.

The original's mode 1 (`0x44efb4`, found through the jump table at
`0x44f25f`) walks a vector of person pointers at
`[[person+0xdc]+0x84]`, not the object list; mode 12 (`0x44f0d4`) walks
the same one. So the world now keeps that vector.

What was decided:

- **`World::people` is the person vector** (snapshot format 20,
  `format_19_bytes` proves the older pins): the people a load reads, in
  registration order (as before, from the old prototype's reading of
  the load), each person the factory makes appended after it is
  constructed, and a removed object taken out. A name in it with no
  object, or an object that is not a person, is `Trap::Invalid`, not
  skipped.
- **Unverified**: the code that appends to the vector was not found (the
  person code passes it to `0x4258a3`, which does not use it), so the
  end is read from tick 409's draws, and when in the making the person
  joins is not known. No capture page holds the vector. Any order with
  person 16 before person 259 fits tick 409. Because of that, a walk of
  the vector while the factory is running a new person's scripts traps
  `Unsupported` rather than showing the person or hiding it. Two people
  made at run time are taken to join in the order they are made, as a
  vector's append gives.

What would reopen it: a capture or trace of the vector, or a tick where
two people made at run time are walked.

K goes from 408 to 416. Tick 417 stops loudly on Find Best Object for
Function (opcode 14), the next step.

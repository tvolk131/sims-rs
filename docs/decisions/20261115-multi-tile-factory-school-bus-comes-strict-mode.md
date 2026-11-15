# The multi-tile factory; the school bus comes; strict mode to tick 2881

*Decided 2026-11-15.*

**Unverified.** No capture covers any of it: the Goth captures end at
update 1000, and all of this is from tick 2703 on. Strict mode's output
for ticks 1 to 2702 is unchanged line for line, hashes included:
3b539cc1… at tick 1102 and c5803e84… at tick 2702. (The entry below
gives older hashes for those ticks; `boot.rs` pins the current ones.)
Strict mode now runs through tick 2881, hash eed688a0…, and stops at
tick 2882.

**Time.** Every update is two game seconds: 30 an in-game minute, 1,800
an hour, 43,200 a day. This is measured, not assumed: the load starts at
06:30:44 (globals 0, 5, 6) and tick 2881 ends at 08:06:46, which
`boot.rs` checks. Strict mode covers 96 minutes of the Goths' day. A
full day is about 15 times that.

**The multi-tile factory** (the group branch of 0x440b2d, from
0x440b63) is `Interpreter::create_group`. It is ported from the old
prototype (which compared it with the original's code run alone) and
read again from the original's code:

1. The definition asked for must be its group's catalog master (word
   11 is -1). Anything else makes nothing in the original and returns 0
   (0x440b90), which fails loudly.
2. The plan: the lead tile first (the first definition of the group
   with word 63 set, 0x43acf5, else the one at the group's origin with
   level 0, 0x43ac5c), then every other tile in the file's resource
   order. A group with no lead fails loudly (0x440c9b).
3. Each tile takes the lowest unused ID. It is built as an ordinary
   object and then joined to the end of its lead's list (0x4227b3). Its
   place in the group comes from its definition (0x422872, 0x43183c).
4. The lead takes the master's price in data word 41 (0x440cd9).
5. The group's reset (0x423175 with 1): every tile is reset and its
   thread restarts at Main, the offsets from the lead are worked out
   (0x422a3b), and then, tile by tile, Init runs, the tile gets its first
   turn, and its scheduler flags are refreshed.
6. Each tile still in the group gets the ready flag (0x440d07).

Not ported: the paths for an allocation or registration failure, and
object flag 0x80, which nothing in the simulation reads.

**What the bus reaches next:**

- Create New Object Instance turns a new group to the stack object's
  facing by the group's own turn (vf 0x4c, 0x422937, at most four
  steps). The group comes off, each tile turns by the tile's data word
  23, the offsets are worked out again, and the group goes back if it
  fits. If it does not fit and the place is on the lot, it turns back
  first. A one-tile turn (0x433f75) still fails.
- The place test of a group (vf 0x54, 0x422abb). The origin is the
  place less the tile's offset, and an origin off the inner tiles
  refuses with code 1 (0x422632). Then every tile is tested at its own
  place. The code kept is the first tile's, replaced by a later 0x1e or
  0x1f. A tile that leaves no code fails loudly, and so does a later
  tile's unknown code after a known one. In a collision test, every
  tile of my own group counts as me (vf 0x5c, 0x42314e).
- A level outside 1 to 3 is now its own refusal, code 2 (0x433058).
  Before, a level of 0 was taken as on the lot. The hashes did not
  change.
- Find Location For modes 3 and 4 (0x44b676) search along a line through
  the local object, by its facing (mode 4 turns it by 2). The line steps
  0, 1, -1, 2… along one coordinate (0x434be9). Without flag bit 0 they
  give native error 0x3d. Modes 2 and 5 still fail.
- An entry that runs at once, pushed onto another person, runs on that
  person's stack (0x45d240). 0x45d20b is the person's own method and
  reads nothing of who pushed it.

**After the bus:**

- **Tick 2747: Special Effect** (opcode 35, 0x44aaa6) is a machine on
  the primitive state. State 0 sets global 11 to 1 and yields. State 1
  needs a stack object (native 0x17) and reads the duration. It then
  yields if the effect is shown, and otherwise takes the true branch;
  any later state is true. The effect is shown with flag 8, or when the
  options' check box (options +4, 0x4cfe18) is on and the zoning or
  family gate passes. That check box is written only by the Options
  panel (0x4e62fa) and by the constructor (0x4e5f63, with 0), and the
  frozen image holds 0, so it is taken as off. The listeners' message
  0x10c, the text and the effect itself are presentation. That they
  write nothing the sim reads is **unverified**.
- **Tick 2791: Snap's false branch** (0x454e09 to 0x454e9b). Person 16
  stands up from the sofa (objects 173 and 174) onto a place that
  collides with tile 174. Word 52 becomes 0, or 2 when the placement
  error is 1 (off the lot). Word 54 becomes the ID of the object
  collided with, or 0. Snap clears the error and the object first
  (0x454d64), and only a collision (0x433b42) writes the object, so a
  room or wall refusal leaves 0. Two cases fail loudly:
  - A person blocked by another person with flag bit 1, which asks
    0x45bd5e (not ported).
  - A multi-tile object that does not fit. Its group test keeps the
    first tile's code, but a later tile's collision still writes the
    object.
- **Tick 2836: animator states 1 and 2** (0x58f0eb, 0x58f12e). A clip
  still in the foreground is cleared, the noted posture is forgotten and
  the base layer is updated, unless the state is 1 and person word 78
  is set. Either way the animator stops walking. The route fields
  cleared with it (+0x24, +0x28, +0x290) are not kept.
- **Tick 2837: a head looking at an object that is not a person**
  (0x59058f). The target is the object's position in world units (3/16,
  0x454f5c) at height +0.0. A person looking at itself still fails.

**Where it stops.** At tick 2882 person 16, standing after the sofa,
has a pose without a foreground layer (0x5992b3). That needs the base
layer's playback, which is new animator state, and the skin's pose
from a base layer. It is big, so strict mode stops there.

**The bus itself.** At tick 2703 the carpool (object 8, tree 0x1017
node 4) makes the School Bus (`SchoolBus.iff`, master 0x50AC15BF). Its
lead is object 12 (GUID 0x50AE1B48) at (728, 488), and it has 30 tiles,
3 across and 10 long. In the same tick a second group is made,
`CarJunk.iff` (lead 334, GUID 0x4B748E3B, 18 tiles). Deleting a group
(0x440f37) is still not ported and is not reached by tick 2881.

**The snack order** now runs through step 3400. At step 3401 person 16
(tree 0x1013 node 1) routes for an action that wants to be interrupted
(0x45afd2), which is not ported.
